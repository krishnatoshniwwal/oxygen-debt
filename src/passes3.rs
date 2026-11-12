//! passes3.rs - if-conversion, loop peeling / trip counts, instruction scheduling.
use crate::ir::*;
use crate::kir::*;
use crate::passes::{cleanup_cfg, evalop};
use crate::passes2::{may_alias, MKey, MemInfo};
use crate::ranges::{Iv, Ranges};
use std::collections::{BTreeMap, HashMap};

// ---------------------------------------------------------------------------
// If-conversion: branches over a few cheap, safe instructions become selects
// ---------------------------------------------------------------------------
fn side_ok(f: &Func, s: usize, b: usize, j: usize, limit: usize, rng: &Ranges) -> bool {
    let sb = &f.arena[s];
    if sb.preds.len() != 1 || sb.preds[0] != b || !sb.phis.is_empty() || s == j || s == b {
        return false;
    }
    let term = sb.term();
    if term.op != Op::Jmp || term.t[0] != j {
        return false;
    }
    if sb.ins.len() - 1 > limit {
        return false;
    }
    for ins in &sb.ins[..sb.ins.len() - 1] {
        if is_pure(ins.op) {
            continue;
        }
        if matches!(ins.op, Op::Div | Op::Mod | Op::Load) && !rng.may_trap(ins, b) {
            continue;
        }
        return false;
    }
    true
}

pub fn ifconv(f: &mut Func, rng: &Ranges, limit: usize) -> bool {
    if limit == 0 {
        return false;
    }
    let mut changed = false;
    for _ in 0..8 {
        let mut any = false;
        let blocks = f.blocks.clone();
        for b in blocks {
            let term = f.arena[b].term();
            if term.op != Op::Br {
                continue;
            }
            let (t, e) = (term.t[0], term.t[1]);
            if t == e {
                continue;
            }
            let c = term.a[0];
            let tj = if f.arena[t].term().op == Op::Jmp { Some(f.arena[t].term().t[0]) } else { None };
            let ej = if f.arena[e].term().op == Op::Jmp { Some(f.arena[e].term().t[0]) } else { None };
            let sides: Option<(Option<usize>, Option<usize>)>;
            let j;
            if tj.is_some() && tj == ej && side_ok(f, t, b, tj.unwrap(), limit, rng) && side_ok(f, e, b, tj.unwrap(), limit, rng) {
                j = tj.unwrap();
                sides = Some((Some(t), Some(e)));
            } else if tj == Some(e) && side_ok(f, t, b, e, limit, rng) {
                j = e;
                sides = Some((Some(t), None));
            } else if ej == Some(t) && side_ok(f, e, b, t, limit, rng) {
                j = t;
                sides = Some((None, Some(e)));
            } else {
                continue;
            }
            if j == b {
                continue;
            }
            let (st, se) = sides.unwrap();
            let pt = st.unwrap_or(b);
            let pe = se.unwrap_or(b);
            let it = f.arena[j].preds.iter().position(|&x| x == pt).unwrap();
            let ie = f.arena[j].preds.iter().position(|&x| x == pe).unwrap();
            let nsel = f.arena[j].phis.iter().filter(|phi| phi.a[it] != phi.a[ie]).count();
            if nsel > limit.max(1) {
                continue;
            }
            f.arena[b].ins.pop();
            for s in [st, se].iter().flatten() {
                let l = f.arena[*s].ins.len() - 1;
                let moved: Vec<Instr> = f.arena[*s].ins[..l].to_vec();
                f.arena[b].ins.extend(moved);
            }
            let nphi = f.arena[j].phis.len();
            for pi in 0..nphi {
                let (vt, ve) = (f.arena[j].phis[pi].a[it], f.arena[j].phis[pi].a[ie]);
                let v;
                if vt == ve {
                    v = vt;
                } else {
                    v = f.new_val();
                    f.arena[b].ins.push(Instr::new(Op::Select, v, vec![c, vt, ve], vec![]));
                }
                f.arena[j].phis[pi].a[it] = v;
            }
            f.arena[b].ins.push(Instr::no_d(Op::Jmp, vec![], vec![j]));
            f.arena[j].preds[it] = b;
            f.arena[j].preds.remove(ie);
            for phi in f.arena[j].phis.iter_mut() {
                phi.a.remove(ie);
            }
            for s in [st, se].iter().flatten() {
                f.arena[*s].preds.clear();
                f.arena[*s].ins = vec![Instr::no_d(Op::Jmp, vec![], vec![*s])];
            }
            any = true;
        }
        if !any {
            break;
        }
        changed = true;
        cleanup_cfg(f);
    }
    changed
}

// ---------------------------------------------------------------------------
// Trip counts of loops with a constant number of rounds (found by running the loop's own
// exit test on constants)
// ---------------------------------------------------------------------------
fn ev(f: &Func, locs: &[Option<(usize, usize)>], x: i64, env: &HashMap<i64, i64>, depth: i32) -> Option<i64> {
    if x < 0 {
        return Some(!x);
    }
    if let Some(&v) = env.get(&x) {
        return Some(v);
    }
    if depth > 12 {
        return None;
    }
    let (b, i) = locs.get(x as usize).copied().flatten()?;
    let d = &f.arena[b].ins[i];
    if d.a.len() != 2 || d.op == Op::Load || d.op == Op::Vload || !is_binop(d.op) {
        return None;
    }
    let p = ev(f, locs, d.a[0], env, depth + 1)?;
    let q = ev(f, locs, d.a[1], env, depth + 1)?;
    evalop(d.op, p, q)
}

/// {header block id: number of header executions} for loops whose exit test only depends on
/// constants. The result is only used to decide how much to peel; peeling is correct
/// whatever this function says.
pub fn find_const_trips(f: &mut Func, maxtrip: usize) -> BTreeMap<usize, usize> {
    analyze(f);
    let nv = f.nval.max(0) as usize + 1;
    let mut locs: Vec<Option<(usize, usize)>> = vec![None; nv + 1];
    for &b in &f.blocks {
        for (i, ins) in f.arena[b].ins.iter().enumerate() {
            if ins.has_d() {
                locs[ins.d as usize] = Some((b, i));
            }
        }
    }
    let mut out: BTreeMap<usize, usize> = BTreeMap::new();
    for lp in &f.loops {
        let h = lp.header;
        if f.arena[h].phis.is_empty() {
            continue;
        }
        let preds = &f.arena[h].preds;
        let out_idx: Vec<usize> = preds.iter().enumerate().filter(|(_, p)| !lp.ids.contains(p)).map(|(i, _)| i).collect();
        let in_idx: Vec<usize> = preds.iter().enumerate().filter(|(_, p)| lp.ids.contains(p)).map(|(i, _)| i).collect();
        if out_idx.is_empty() || in_idx.is_empty() {
            continue;
        }
        let mut env0: HashMap<i64, i64> = HashMap::new();
        let mut nxt: Vec<(i64, i64)> = Vec::new();
        for phi in &f.arena[h].phis {
            let init = phi.a[out_idx[0]];
            if init >= 0 || out_idx.iter().any(|&i| phi.a[i] != init) {
                continue;
            }
            let n = phi.a[in_idx[0]];
            if in_idx.iter().any(|&i| phi.a[i] != n) {
                continue;
            }
            env0.insert(phi.d, !init);
            nxt.push((phi.d, n));
        }
        if env0.is_empty() {
            continue;
        }
        let mut tests: Vec<(i64, bool)> = Vec::new();
        for &x in &lp.blocks {
            let term = f.arena[x].term();
            if term.op != Op::Br {
                continue;
            }
            let o0 = !lp.ids.contains(&term.t[0]);
            let o1 = !lp.ids.contains(&term.t[1]);
            if o0 == o1 {
                continue;
            }
            if in_idx.iter().all(|&i| f.dominates(x, preds[i])) {
                tests.push((term.a[0], o0));
            }
        }
        if tests.is_empty() {
            continue;
        }
        for (c, exit_when_true) in tests {
            let mut env = env0.clone();
            let mut stays = 0usize;
            let mut ok = false;
            while stays <= maxtrip {
                let v = match ev(f, &locs, c, &env, 0) {
                    Some(v) => v,
                    None => break,
                };
                if (v != 0) == exit_when_true {
                    ok = true;
                    break;
                }
                stays += 1;
                let mut new: HashMap<i64, i64> = HashMap::new();
                for &(p, n) in &nxt {
                    if env.contains_key(&p) {
                        if let Some(nv) = ev(f, &locs, n, &env, 0) {
                            new.insert(p, nv);
                        }
                    }
                }
                env = new;
            }
            if ok {
                let cur = out.get(&h).copied();
                if cur.map_or(true, |c| stays + 1 < c) {
                    out.insert(h, stays + 1);
                }
            }
        }
    }
    out
}

/// Non-SSA form. Put n copies of the loop in front of it: entering the loop runs copy 0, its
/// back edge goes to copy 1, ..., the last copy's back edge enters the original loop. Pure
/// code copying. The caller must have run analyze().
pub fn peel(f: &mut Func, header: usize, n: usize) -> bool {
    let li = match f.loops.iter().position(|l| l.header == header) {
        Some(l) => l,
        None => return false,
    };
    if n == 0 {
        return false;
    }
    let lp_blocks = f.loops[li].blocks.clone();
    let lp_ids = f.loops[li].ids.clone();
    let h = header;
    let mut copies: Vec<HashMap<usize, usize>> = Vec::new();
    let mut newblocks: Vec<usize> = Vec::new();
    for j in 0..n {
        let mut m = HashMap::new();
        for &b in &lp_blocks {
            let name = format!("{}_p{}", f.arena[b].name, j);
            let nb = f.new_block(&name);
            f.arena[nb].ins = f.arena[b].ins.clone();
            m.insert(b, nb);
            newblocks.push(nb);
        }
        copies.push(m);
    }
    for j in 0..n {
        for &b in &lp_blocks {
            let cb = copies[j][&b];
            let ntt = f.arena[cb].ins.last().unwrap().t.len();
            for i in 0..ntt {
                let t = f.arena[cb].ins.last().unwrap().t[i];
                let nt = if t == h {
                    if j + 1 < n { copies[j + 1][&h] } else { h }
                } else if lp_ids.contains(&t) {
                    copies[j][&t]
                } else {
                    t
                };
                f.arena[cb].ins.last_mut().unwrap().t[i] = nt;
            }
        }
    }
    let first = copies[0][&h];
    for bi in 0..f.blocks.len() {
        let b = f.blocks[bi];
        if lp_ids.contains(&b) {
            continue;
        }
        for t in f.arena[b].ins.last_mut().unwrap().t.iter_mut() {
            if *t == h {
                *t = first;
            }
        }
    }
    if f.blocks[0] == h {
        let mut nb = vec![first];
        nb.extend(f.blocks.iter().copied());
        nb.extend(newblocks.iter().copied().filter(|&x| x != first));
        f.blocks = nb;
    } else {
        f.blocks.extend(newblocks);
    }
    true
}

/// Non-SSA form: fully peel loops with a small constant trip count (innermost first) while
/// the program stays within `budget` instructions. Returns true if anything changed.
pub fn peel_const_loops(f: &mut Func, trips: &BTreeMap<usize, usize>, budget: usize, maxtrip: usize) -> bool {
    if trips.is_empty() {
        return false;
    }
    compute_preds(f);
    analyze(f);
    let mut size = f.count_instrs();
    let mut order: Vec<usize> = (0..f.loops.len()).collect();
    order.sort_by_key(|&i| (std::cmp::Reverse(f.loops[i].depth), f.arena[f.loops[i].header].rpo));
    let mut changed = false;
    let mut done_ids: std::collections::HashSet<usize> = Default::default();
    for li in order {
        let header = f.loops[li].header;
        let n = match trips.get(&header) {
            Some(&n) => n,
            None => continue,
        };
        if n > maxtrip || n < 1 {
            continue;
        }
        if f.loops[li].ids.iter().any(|b| done_ids.contains(b)) {
            continue;
        }
        let lsize: usize = f.loops[li].blocks.iter().map(|&b| f.arena[b].ins.len()).sum();
        if size + n * lsize > budget {
            continue;
        }
        if peel(f, header, n) {
            size += n * lsize;
            changed = true;
            let add: Vec<usize> = f.loops.iter().filter(|l2| l2.ids.contains(&header)).flat_map(|l2| l2.ids.iter().copied()).collect();
            done_ids.extend(add);
        }
    }
    if changed {
        compute_preds(f);
    }
    changed
}

// ---------------------------------------------------------------------------
// Scheduling inside a block
// ---------------------------------------------------------------------------
const WINDOW: usize = 40;
const AHEAD: usize = 32;

/// Reorder the instructions of each block (SSA form) so that the in-order machine waits
/// less. Order is kept between: all visible effects and all instructions that may trap; a
/// load and any store that may write its address.
pub fn schedule(f: &mut Func, mc: &Machine, rng: &Ranges) {
    let w = mc.issue_width;
    let mp = mc.mem_ports;
    let mi = MemInfo::new(rng);
    for bi in 0..f.blocks.len() {
        let b = f.blocks[bi];
        if f.arena[b].ins.len() < 4 {
            continue;
        }
        let n = f.arena[b].ins.len() - 1;
        let ins_ = std::mem::take(&mut f.arena[b].ins);
        let mut preds: Vec<Vec<usize>> = vec![Vec::new(); n];
        let mut defpos: HashMap<i64, usize> = HashMap::new();
        let mut last_chain: i64 = -1;
        let mut stores: Vec<(usize, MKey, Iv)> = Vec::new();
        let mut safe_loads: Vec<(usize, MKey, Iv)> = Vec::new();
        let mut lat = vec![0i64; n];
        for i in 0..n {
            let ins = &ins_[i];
            let op = ins.op;
            for &x in &ins.a {
                if x >= 0 {
                    if let Some(&j) = defpos.get(&x) {
                        preds[i].push(j);
                    }
                }
            }
            let mut chain = false;
            match op {
                Op::Store => {
                    chain = true;
                    let k = mi.key(ins.a[0]);
                    let r = rng.get(ins.a[0], b);
                    for &(j, k2, r2) in &safe_loads {
                        if may_alias(k, r, k2, r2) {
                            preds[i].push(j);
                        }
                    }
                    stores.push((i, k, r));
                }
                Op::Vload | Op::Vstore => chain = true,
                Op::Load => {
                    if rng.may_trap(ins, b) {
                        chain = true;
                    } else {
                        let k = mi.key(ins.a[0]);
                        let r = rng.get(ins.a[0], b);
                        for &(j, k2, r2) in stores.iter().rev() {
                            if may_alias(k, r, k2, r2) {
                                preds[i].push(j);
                                break;
                            }
                        }
                        safe_loads.push((i, k, r));
                    }
                }
                Op::Div | Op::Mod => {
                    if rng.may_trap(ins, b) {
                        chain = true;
                    }
                }
                _ => {}
            }
            if chain {
                if last_chain >= 0 {
                    preds[i].push(last_chain as usize);
                }
                last_chain = i as i64;
            }
            if ins.has_d() {
                defpos.insert(ins.d, i);
                lat[i] = mc.lat(op);
            }
        }
        let mut succs: Vec<Vec<usize>> = vec![Vec::new(); n];
        let mut npred = vec![0usize; n];
        for i in 0..n {
            if !preds[i].is_empty() {
                preds[i].sort_unstable();
                preds[i].dedup();
                npred[i] = preds[i].len();
                for k in 0..preds[i].len() {
                    let j = preds[i][k];
                    succs[j].push(i);
                }
            }
        }
        // critical path to the end of the block
        let mut cp = vec![0i64; n];
        let term = &ins_[n];
        let mut tail: HashMap<usize, i64> = HashMap::new();
        for &x in &term.a {
            if x >= 0 {
                if let Some(&p) = defpos.get(&x) {
                    tail.insert(p, 1);
                }
            }
        }
        for i in (0..n).rev() {
            let mut m = tail.get(&i).copied().unwrap_or(0);
            for &s in &succs[i] {
                if cp[s] > m {
                    m = cp[s];
                }
            }
            cp[i] = lat[i] + m;
        }
        let mut ready: HashMap<i64, i64> = HashMap::new();
        let mut avail: Vec<usize> = (0..n).filter(|&i| npred[i] == 0).collect();
        let mut order: Vec<usize> = Vec::with_capacity(n);
        let mut t_last = 0i64;
        let mut used = 0i64;
        let mut memused = 0i64;
        while !avail.is_empty() {
            let mut best = 0usize;
            let mut bestkey: Option<(i64, i64, usize)> = None;
            let lim = WINDOW.min(avail.len());
            let far = avail[0] + AHEAD;
            for ai in 0..lim {
                let i = avail[ai];
                if i >= far {
                    break;
                }
                let ins = &ins_[i];
                let mut e = t_last;
                for &x in &ins.a {
                    if x >= 0 {
                        if let Some(&r) = ready.get(&x) {
                            if r > e {
                                e = r;
                            }
                        }
                    }
                }
                let ismem = ins.op.is_mem();
                if e == t_last && (used >= w || (ismem && memused >= mp)) {
                    e += 1;
                }
                let key = (e, -cp[i], i);
                if bestkey.map_or(true, |bk| key < bk) {
                    bestkey = Some(key);
                    best = ai;
                }
            }
            let i = avail.remove(best);
            let ins = &ins_[i];
            let e = bestkey.unwrap().0;
            if e > t_last {
                used = 0;
                memused = 0;
                t_last = e;
            }
            used += 1;
            if ins.op.is_mem() {
                memused += 1;
            }
            if ins.has_d() {
                ready.insert(ins.d, e + lat[i]);
            }
            order.push(i);
            for k in 0..succs[i].len() {
                let s = succs[i][k];
                npred[s] -= 1;
                if npred[s] == 0 {
                    let pos = avail.partition_point(|&x| x < s);
                    avail.insert(pos, s);
                }
            }
        }
        if order.len() == n {
            let mut new_ins: Vec<Instr> = order.iter().map(|&i| ins_[i].clone()).collect();
            new_ins.push(ins_[n].clone());
            f.arena[b].ins = new_ins;
        } else {
            f.arena[b].ins = ins_;
        }
    }
}
