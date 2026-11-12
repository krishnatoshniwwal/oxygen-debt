//! memfwd.rs - forward memory words inside and across iterations of innermost loops (SSA form).
//!
//! For a loop with a header phi `p` that steps by a constant `s` (`p' = p + s` on the back edge),
//! every address `p + c` (c a constant) names one word per iteration. A forward data flow over
//! the loop body tracks, for one word, which SSA value it holds:
//!   * a load or a store of exactly that word sets it (the loaded / stored value);
//!   * a store to `p + c'` with c' != c (mod 2^32) leaves it alone (a different word);
//!   * any other store, and every vstore, forgets it;
//!   * at a join inside the loop, equal values pass, different values get a new phi.
//!
//! Two rewrites use it:
//!   1. *Inside an iteration*: a load whose word is already known at that point is replaced by
//!      the known value (the earlier access ran on every path, so the address is valid).
//!   2. *Across iterations*: a load of `p + c` that no store can precede within the iteration
//!      reads what word `p + c + s` held at the end of the previous iteration. That value
//!      becomes a header phi; a copy of the load in the preheader feeds the first iteration
//!      (kept only if the value ranges prove it cannot trap). Typical case: bubble sort, where
//!      a[j] of the next step is a[j+1] of this one, whether or not the two were swapped.
//!
//! Both are exact: the value is the one the load would read, and no trap is lost (iterations
//! after the first re-read a word the previous iteration already loaded or stored).
use crate::ir::*;
use crate::kir::*;
use crate::passes2::get_preheader;
use crate::ranges::Ranges;
use std::collections::HashMap;

const M32: i64 = 0xFFFF_FFFF;

fn litval(x: i64) -> i64 {
    (!x) & M32
}

/// Value -> defining instruction (op, operands) for the loop's blocks.
struct Ctx {
    p: i64,
    defs: HashMap<i64, (Op, Vec<i64>)>,
}

impl Ctx {
    /// Offset c (mod 2^32) when `x == p + c`.
    fn form(&self, x: i64) -> Option<i64> {
        if x == self.p {
            return Some(0);
        }
        if x < 0 {
            return None;
        }
        match self.defs.get(&x) {
            Some((Op::Add, a)) if a.len() == 2 => {
                if a[0] == self.p && a[1] < 0 {
                    Some(litval(a[1]))
                } else if a[1] == self.p && a[0] < 0 {
                    Some(litval(a[0]))
                } else {
                    None
                }
            }
            _ => None,
        }
    }
}

/// Effect of one instruction on the tracked word `w`. Returns the new state.
fn step(cx: &Ctx, ins: &Instr, w: i64, st: Option<i64>) -> Option<i64> {
    match ins.op {
        Op::Load => match cx.form(ins.a[0]) {
            Some(c) if c == w => Some(ins.d),
            _ => st,
        },
        Op::Store => match cx.form(ins.a[0]) {
            Some(c) if c == w => Some(ins.a[1]),
            Some(_) => st,
            None => None,
        },
        Op::Vstore => None,
        _ => st,
    }
}

/// Block-entry states for word `w` over the loop body (header entry = unknown). New phis are
/// created in `f` at joins where the incoming values differ. Returns None if the flow gives up.
fn flow(f: &mut Func, cx: &Ctx, order: &[usize], h: usize, w: i64, inside: &dyn Fn(usize) -> bool) -> Option<HashMap<usize, Option<i64>>> {
    let mut out: HashMap<usize, Option<i64>> = HashMap::new();
    let mut ins_state: HashMap<usize, Option<i64>> = HashMap::new();
    for &b in order {
        let entry = if b == h {
            None
        } else {
            let preds = f.arena[b].preds.clone();
            if preds.iter().any(|q| !inside(*q)) {
                return None;
            }
            let vals: Vec<Option<i64>> = preds.iter().map(|q| out.get(q).copied().flatten()).collect();
            if preds.iter().any(|q| !out.contains_key(q)) {
                return None; // a pred later in the order: not a simple acyclic body
            }
            if vals.iter().any(|v| v.is_none()) {
                None
            } else if vals.iter().all(|v| *v == vals[0]) {
                vals[0]
            } else {
                let d = f.new_val();
                let a: Vec<i64> = vals.iter().map(|v| v.unwrap()).collect();
                f.arena[b].phis.push(Instr::new(Op::Phi, d, a, vec![]));
                Some(d)
            }
        };
        ins_state.insert(b, entry);
        let mut st = entry;
        for ins in &f.arena[b].ins {
            st = step(cx, ins, w, st);
        }
        out.insert(b, st);
    }
    let _ = out;
    Some(ins_state)
}

fn rename_all(f: &mut Func, from: i64, to: i64) {
    for &b in &f.blocks.clone() {
        let blk = &mut f.arena[b];
        for ins in blk.phis.iter_mut().chain(blk.ins.iter_mut()) {
            for a in ins.a.iter_mut() {
                if *a == from {
                    *a = to;
                }
            }
        }
    }
}

/// One rewrite on one loop; returns the new function if something changed.
fn dbg(m: &str) {
    if std::env::var_os("KOPT_MF_DEBUG").is_some() {
        eprintln!("memfwd: {}", m);
    }
}

fn try_loop(f: &Func, li: usize) -> Option<Func> {
    let mut g = f.clone();
    dbg(&format!("loop {} latches {} preds {}", g.arena[g.loops[li].header].name, g.loops[li].latches.len(), g.arena[g.loops[li].header].preds.len()));
    let h = g.loops[li].header;
    let ids = g.loops[li].ids.clone();
    let pre = get_preheader(&mut g, li)?;
    if g.arena[pre].term().op != Op::Jmp {
        return None;
    }
    let hp = g.arena[h].preds.clone();
    let pre_idx = hp.iter().position(|&q| q == pre)?;
    let backs: Vec<usize> = (0..hp.len()).filter(|&j| j != pre_idx).collect();
    if backs.is_empty() || backs.iter().any(|&j| !ids.contains(&hp[j])) || hp.iter().filter(|&&q| q == pre).count() != 1 {
        return None;
    }
    dbg("preheader ok");
    let mut order: Vec<usize> = g.loops[li].blocks.clone();
    order.sort_by_key(|&b| g.arena[b].rpo);
    if order[0] != h {
        return None;
    }
    let mut defs: HashMap<i64, (Op, Vec<i64>)> = HashMap::new();
    let mut n_ins = 0;
    for &b in &order {
        n_ins += g.arena[b].ins.len();
        for ins in &g.arena[b].ins {
            if ins.has_d() && ins.d >= 0 {
                defs.insert(ins.d, (ins.op, ins.a.clone()));
            }
        }
    }
    if n_ins > 400 {
        return None;
    }
    let inside = |q: usize| ids.contains(&q);
    let phis = g.arena[h].phis.clone();
    for phi in &phis {
        let cx = Ctx { p: phi.d, defs: defs.clone() };
        let s = match cx.form(phi.a[backs[0]]) {
            Some(s) if s != 0 => s,
            _ => continue,
        };
        if backs.iter().any(|&j| cx.form(phi.a[j]) != Some(s)) {
            continue;
        }
        dbg(&format!("phi {} step {}", phi.d, s));
        // loads of the loop with address p + c
        let mut loads: Vec<(usize, usize, i64, i64)> = Vec::new(); // (block, index, dest, c)
        for &b in &order {
            for (k, ins) in g.arena[b].ins.iter().enumerate() {
                if ins.op == Op::Load {
                    if let Some(c) = cx.form(ins.a[0]) {
                        loads.push((b, k, ins.d, c));
                    }
                }
            }
        }
        // 1. inside the iteration
        for &(b, k, d, c) in &loads {
            let mut g2 = g.clone();
            let ent = match flow(&mut g2, &cx, &order, h, c, &inside) {
                Some(e) => e,
                None => continue,
            };
            let mut st = ent[&b];
            for ins in &g2.arena[b].ins[..k] {
                st = step(&cx, ins, c, st);
            }
            if let Some(v) = st {
                if v != d {
                    g2.arena[b].ins.remove(k);
                    rename_all(&mut g2, d, v);
                    return Some(g2);
                }
            }
        }
        // 2. across iterations
        for &(b, k, d, c) in &loads {
            // no store or vstore may run between the header entry and this load
            let mut before: Vec<usize> = vec![b];
            let mut i = 0;
            while i < before.len() {
                let x = before[i];
                i += 1;
                if x == h {
                    continue;
                }
                for &q in &g.arena[x].preds {
                    if inside(q) && !before.contains(&q) {
                        before.push(q);
                    }
                }
            }
            let writes = |ins: &Instr| ins.op == Op::Store || ins.op == Op::Vstore;
            if before.iter().any(|&x| x != b && g.arena[x].ins.iter().any(writes)) || g.arena[b].ins[..k].iter().any(writes) {
                continue;
            }
            let w = (c + s) & M32;
            let mut g2 = g.clone();
            let ent = match flow(&mut g2, &cx, &order, h, w, &inside) {
                Some(e) => e,
                None => { dbg("flow failed"); continue }
            };
            let mut vs: Vec<i64> = Vec::new();
            for &j in &backs {
                let lb = hp[j];
                let mut st = ent[&lb];
                for ins in &g2.arena[lb].ins {
                    st = step(&cx, ins, w, st);
                }
                match st {
                    Some(v) if v != d => vs.push(v),
                    _ => break,
                }
            }
            if vs.len() != backs.len() {
                dbg(&format!("word {} unknown at a latch", w));
                continue;
            }
            // preheader copy: load(init + c)
            let init = phi.a[pre_idx];
            let term = g2.arena[pre].ins.pop().unwrap();
            let addr = if c == 0 {
                init
            } else {
                let t = g2.new_val();
                g2.arena[pre].ins.push(Instr::new(Op::Add, t, vec![init, lit(c)], vec![]));
                t
            };
            let lv = g2.new_val();
            g2.arena[pre].ins.push(Instr::new(Op::Load, lv, vec![addr], vec![]));
            g2.arena[pre].ins.push(term);
            let q = g2.new_val();
            let mut a = vec![0i64; hp.len()];
            a[pre_idx] = lv;
            for (t, &j) in backs.iter().enumerate() {
                a[j] = vs[t];
            }
            g2.arena[h].phis.push(Instr::new(Op::Phi, q, a, vec![]));
            g2.arena[b].ins.remove(k);
            rename_all(&mut g2, d, q);
            // the preheader load runs even if the loop body does not: prove it safe
            let mut g3 = g2.clone();
            let rng = Ranges::new(&mut g3, 14);
            if !rng.ok {
                continue;
            }
            let safe = match g3.arena[pre].ins.iter().find(|x| x.has_d() && x.d == lv) {
                Some(x) => !rng.may_trap(x, pre),
                None => false,
            };
            if !safe {
                continue;
            }
            return Some(g2);
        }
    }
    None
}

/// Run both rewrites on every innermost loop until nothing changes (bounded).
pub fn forward(f: &mut Func) -> bool {
    let mut changed = false;
    for _ in 0..24 {
        analyze(f);
        let mut done = false;
        for li in 0..f.loops.len() {
            if f.loops.iter().any(|o| o.parent == Some(li)) {
                continue;
            }
            if let Some(g) = try_loop(f, li) {
                *f = g;
                changed = true;
                done = true;
                break;
            }
        }
        if !done {
            break;
        }
    }
    if changed {
        analyze(f);
    }
    changed
}
