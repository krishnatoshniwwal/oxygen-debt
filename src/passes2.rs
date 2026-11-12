//! passes2.rs - passes that need value ranges or memory reasoning, plus machine lowering.
use crate::ir::*;
use crate::kir::*;
use crate::passes::{apply_rep, use_counts, NOREP};
use crate::ranges::{Iv, Ranges, SMAX};
use std::collections::{HashMap, HashSet};

fn pow2(c: i64) -> bool {
    c > 0 && (c & (c - 1)) == 0
}

fn bit_length(c: i64) -> i64 {
    64 - c.leading_zeros() as i64
}

/// Use value ranges: fold results that are known constants, drop masks that do nothing,
/// turn div / mod of non-negative values by powers of two into shr / and, fold branches.
/// csub=true: `x % m` (m constant) with 0 <= x < 2m proven becomes a conditional subtract.
pub fn range_simplify(f: &mut Func, rng: &Ranges, csub: bool) -> bool {
    if !rng.ok {
        return false;
    }
    let nv = f.nval.max(0) as usize + 1;
    let mut rep = vec![NOREP; nv];
    let mut changed = false;
    let mut any_rep = false;
    for bi in 0..f.blocks.len() {
        let b = f.blocks[bi];
        let old = std::mem::take(&mut f.arena[b].ins);
        let mut out = Vec::with_capacity(old.len());
        for mut ins in old {
            if !ins.has_d() {
                if ins.op == Op::Br && ins.a[0] >= 0 {
                    let r = rng.get(ins.a[0], b);
                    let mut pick: Option<(usize, usize)> = None;
                    if r.0 >= 1 {
                        pick = Some((ins.t[0], ins.t[1]));
                    } else if r.1 == 0 {
                        pick = Some((ins.t[1], ins.t[0]));
                    }
                    if let Some((tk, dr)) = pick {
                        if dr != tk {
                            remove_edge(f, b, dr);
                        }
                        ins.op = Op::Jmp;
                        ins.a = vec![];
                        ins.t = vec![tk];
                        changed = true;
                    }
                }
                out.push(ins);
                continue;
            }
            let d = ins.d;
            let op = ins.op;
            if is_pure(op) {
                if let Some(r) = rng.base.get(d as usize).copied().flatten() {
                    if r.0 == r.1 {
                        rep[d as usize] = lit(r.0);
                        any_rep = true;
                        changed = true;
                        continue;
                    }
                }
            }
            if op == Op::And && ins.a[1] < 0 {
                let m = !ins.a[1];
                if pow2(m + 1) && rng.get(ins.a[0], b).1 <= m {
                    rep[d as usize] = ins.a[0];
                    any_rep = true;
                    changed = true;
                    continue;
                }
            } else if op == Op::Div || op == Op::Mod {
                let rx = rng.get(ins.a[0], b);
                let ry = rng.get(ins.a[1], b);
                if rx.1 <= SMAX && ry.1 <= SMAX && ry.0 >= 1 {
                    if rx.1 < ry.0 {
                        rep[d as usize] = if op == Op::Mod { ins.a[0] } else { L0 };
                        any_rep = true;
                        changed = true;
                        continue;
                    }
                    if ins.a[1] < 0 && pow2(!ins.a[1]) {
                        let c = !ins.a[1];
                        if op == Op::Div {
                            ins.op = Op::Shr;
                            ins.a = vec![ins.a[0], lit(bit_length(c) - 1)];
                        } else {
                            ins.op = Op::And;
                            ins.a = vec![ins.a[0], lit(c - 1)];
                        }
                        changed = true;
                    } else if csub && op == Op::Mod && ins.a[1] < 0 {
                        // 0 <= x < 2m (proven): x % m == (x < m ? x : x - m).
                        // Three ALU ops instead of a div-latency op on the chain.
                        let m = !ins.a[1];
                        if m >= 1 && 2 * m - 1 <= SMAX && rx.1 <= 2 * m - 1 {
                            let x = ins.a[0];
                            let t1 = f.new_val();
                            let t2 = f.new_val();
                            out.push(Instr::new(Op::Sub, t1, vec![x, lit(m)], vec![]));
                            out.push(Instr::new(Op::Lt, t2, vec![x, lit(m)], vec![]));
                            out.push(Instr::new(Op::Select, d, vec![t2, x, t1], vec![]));
                            changed = true;
                            continue;
                        }
                    }
                }
            } else if op == Op::Select {
                let r = rng.get(ins.a[0], b);
                if r.0 >= 1 {
                    rep[d as usize] = ins.a[1];
                    any_rep = true;
                    changed = true;
                    continue;
                }
                if r.1 == 0 {
                    rep[d as usize] = ins.a[2];
                    any_rep = true;
                    changed = true;
                    continue;
                }
            } else if op == Op::Lt || op == Op::Eq {
                let ra = rng.get(ins.a[0], b);
                let rb = rng.get(ins.a[1], b);
                let mut v: Option<i64> = None;
                if op == Op::Lt {
                    if ra.1 <= SMAX && rb.1 <= SMAX {
                        if ra.1 < rb.0 {
                            v = Some(1);
                        } else if ra.0 >= rb.1 {
                            v = Some(0);
                        }
                    }
                } else if ra.1 < rb.0 || rb.1 < ra.0 {
                    v = Some(0);
                }
                if let Some(v) = v {
                    rep[d as usize] = lit(v);
                    any_rep = true;
                    changed = true;
                    continue;
                }
            }
            out.push(ins);
        }
        f.arena[b].ins = out;
    }
    if any_rep {
        rep.resize(f.nval.max(0) as usize + 1, NOREP); // values made above have no replacement
        apply_rep(f, &rep);
    }
    changed
}

// ---------------------------------------------------------------------------
// Memory: redundant loads, store-to-load forwarding, redundant and dead stores
// ---------------------------------------------------------------------------
pub type MKey = (i64, i64);

pub struct MemInfo<'a> {
    defs: &'a [Option<DefInfo>],
}

impl<'a> MemInfo<'a> {
    pub fn new(rng: &'a Ranges) -> MemInfo<'a> {
        MemInfo { defs: &rng.defs }
    }
    pub fn key(&self, x: i64) -> MKey {
        if x < 0 {
            return (-1, !x);
        }
        if let Some(Some(d)) = self.defs.get(x as usize) {
            if d.op == Op::Add && d.a1 < 0 {
                return (d.a0, !d.a1);
            }
        }
        (x, 0)
    }
}

pub fn may_alias(k1: MKey, r1: Iv, k2: MKey, r2: Iv) -> bool {
    if k1 == k2 {
        return true;
    }
    if k1.0 == k2.0 {
        return false;
    }
    if r1.1 < r2.0 || r2.1 < r1.0 {
        return false;
    }
    true
}

const MAXFACTS: usize = 64;

/// facts: insertion-ordered list of (address key, (value, range))
type Facts = Vec<(MKey, (i64, Iv))>;

fn fpos(s: &Facts, k: MKey) -> Option<usize> {
    s.iter().position(|e| e.0 == k)
}

fn facts_eq(a: &Facts, b: &Facts) -> bool {
    a.len() == b.len() && a.iter().all(|e| b.iter().any(|g| g.0 == e.0 && g.1 == e.1))
}

/// Walk block `ins` with `state`; with apply=true also rewrite it (returns the new list).
/// fwd_loads=false: a load does not create a fact, so a later load of the same word is kept
/// (only values written by stores are forwarded). Reloading can be cheaper than keeping many
/// loaded values alive when there are few registers.
fn mem_transfer(ins_list: &[Instr], b: usize, state: &mut Facts, mi: &MemInfo, rng: &Ranges, rep: &mut Vec<i64>, apply: bool, fwd_loads: bool) -> (bool, Option<Vec<Instr>>) {
    let mut out: Vec<Instr> = Vec::new();
    let mut changed = false;
    for ins in ins_list {
        match ins.op {
            Op::Load => {
                let k = mi.key(ins.a[0]);
                match fpos(state, k) {
                    Some(p) => {
                        if apply {
                            rep[ins.d as usize] = state[p].1 .0;
                            changed = true;
                            continue;
                        }
                    }
                    None => {
                        if fwd_loads {
                            if state.len() >= MAXFACTS {
                                state.remove(0);
                            }
                            state.push((k, (ins.d, rng.get(ins.a[0], b))));
                        }
                    }
                }
            }
            Op::Store => {
                let k = mi.key(ins.a[0]);
                let v = ins.a[1];
                let e = fpos(state, k);
                if e.map_or(false, |p| state[p].1 .0 == v) {
                    if apply {
                        changed = true;
                        continue;
                    }
                } else {
                    let r = rng.get(ins.a[0], b);
                    state.retain(|e2| e2.0 == k || !may_alias(k, r, e2.0, e2.1 .1));
                    if let Some(p) = fpos(state, k) {
                        state.remove(p);
                    } else if state.len() >= MAXFACTS {
                        state.remove(0);
                    }
                    state.push((k, (v, r)));
                }
            }
            _ => {}
        }
        if apply {
            out.push(ins.clone());
        }
    }
    (changed, if apply { Some(out) } else { None })
}

/// Forward data-flow of 'memory word at address A holds value V' facts.
pub fn mem_pass(f: &mut Func, rng: &Ranges, fwd_loads: bool) -> bool {
    let rpo = analyze(f);
    let has_mem = rpo.iter().any(|&b| f.arena[b].ins.iter().any(|i| i.op == Op::Load || i.op == Op::Store));
    if !has_mem {
        return false;
    }
    let mi = MemInfo::new(rng);
    let nb = f.arena.len();
    let mut outs: Vec<Option<Facts>> = vec![None; nb];
    let mut ins_st: Vec<Facts> = vec![Vec::new(); nb];
    let entry = f.blocks[0];
    let mut converged = false;
    let mut dummy = Vec::new();
    for _ in 0..10 {
        let mut changed = false;
        for &b in &rpo {
            let mut st: Facts;
            if b == entry {
                st = Vec::new();
            } else {
                let mut cur: Option<Facts> = None;
                for &p in &f.arena[b].preds {
                    let o = match &outs[p] {
                        Some(o) => o,
                        None => continue,
                    };
                    match cur.as_mut() {
                        None => cur = Some(o.clone()),
                        Some(s) => {
                            let mut keep: Facts = Vec::new();
                            for e1 in s.iter() {
                                match o.iter().find(|e2| e2.0 == e1.0) {
                                    None => {}
                                    Some(e2) => {
                                        if e2.1 .0 != e1.1 .0 {
                                        } else if e2.1 .1 != e1.1 .1 {
                                            let r1 = e1.1 .1;
                                            let r2 = e2.1 .1;
                                            keep.push((e1.0, (e1.1 .0, (r1.0.min(r2.0), r1.1.max(r2.1)))));
                                        } else {
                                            keep.push(e1.clone());
                                        }
                                    }
                                }
                            }
                            *s = keep;
                        }
                    }
                }
                st = cur.unwrap_or_default();
            }
            ins_st[b] = st.clone();
            mem_transfer(&f.arena[b].ins, b, &mut st, &mi, rng, &mut dummy, false, fwd_loads);
            let same = match &outs[b] {
                Some(o) => facts_eq(o, &st),
                None => false,
            };
            if !same {
                outs[b] = Some(st);
                changed = true;
            }
        }
        if !changed {
            converged = true;
            break;
        }
    }
    let nv = f.nval.max(0) as usize + 1;
    let mut rep = vec![NOREP; nv];
    let mut changed = false;
    for &b in &rpo {
        let mut st: Facts = if converged { ins_st[b].clone() } else { Vec::new() };
        let (ch, out) = mem_transfer(&f.arena[b].ins, b, &mut st, &mi, rng, &mut rep, true, fwd_loads);
        if ch {
            changed = true;
        }
        f.arena[b].ins = out.unwrap();
    }
    if changed {
        apply_rep(f, &rep);
    }
    if dse(f, rng, &mi) {
        changed = true;
    }
    changed
}

/// Remove a store that is overwritten later in the same block before anyone can see it.
pub fn dse(f: &mut Func, rng: &Ranges, mi: &MemInfo) -> bool {
    let mut changed = false;
    for bi in 0..f.blocks.len() {
        let b = f.blocks[bi];
        let mut pending: HashMap<MKey, (Iv, bool)> = HashMap::new();
        let old = std::mem::take(&mut f.arena[b].ins);
        let mut out: Vec<Instr> = Vec::with_capacity(old.len());
        for ins in old.into_iter().rev() {
            match ins.op {
                Op::Store => {
                    let k = mi.key(ins.a[0]);
                    let trap = rng.may_trap(&ins, b);
                    if let Some(e) = pending.get(&k) {
                        if e.1 || !trap {
                            changed = true;
                            continue;
                        }
                    }
                    if trap {
                        pending.clear();
                    } else {
                        for e2 in pending.values_mut() {
                            e2.1 = false;
                        }
                    }
                    pending.insert(k, (rng.get(ins.a[0], b), true));
                }
                Op::Load => {
                    if rng.may_trap(&ins, b) {
                        pending.clear();
                    } else {
                        let k = mi.key(ins.a[0]);
                        let r = rng.get(ins.a[0], b);
                        pending.retain(|k2, e2| !may_alias(k, r, *k2, e2.0));
                    }
                }
                Op::Vload | Op::Vstore => {
                    for e2 in pending.values_mut() {
                        e2.1 = false;
                    }
                }
                Op::Div | Op::Mod => {
                    if rng.may_trap(&ins, b) {
                        pending.clear();
                    }
                }
                Op::Trap => pending.clear(),
                _ => {}
            }
            out.push(ins);
        }
        out.reverse();
        f.arena[b].ins = out;
    }
    changed
}

// ---------------------------------------------------------------------------
// Loop-invariant code motion
// ---------------------------------------------------------------------------
/// Return a block that is the only way into the loop from outside and that only jumps to the
/// header. Creates one if needed. Returns None if the loop cannot be entered.
pub fn get_preheader(f: &mut Func, li: usize) -> Option<usize> {
    let h = f.loops[li].header;
    let outside: Vec<usize> = f.arena[h].preds.iter().copied().filter(|p| !f.loops[li].ids.contains(p)).collect();
    if outside.is_empty() {
        return None;
    }
    if outside.len() == 1 && f.arena[outside[0]].term().op == Op::Jmp {
        return Some(outside[0]);
    }
    let n = f.new_block("pre");
    let idxs: Vec<usize> = f.arena[h].preds.iter().enumerate().filter(|(_, p)| !f.loops[li].ids.contains(p)).map(|(i, _)| i).collect();
    let mut newargs = Vec::new();
    let nphis = f.arena[h].phis.len();
    for pi in 0..nphis {
        let args: Vec<i64> = idxs.iter().map(|&i| f.arena[h].phis[pi].a[i]).collect();
        let same = args.iter().all(|&x| x == args[0]);
        if same {
            newargs.push(args[0]);
        } else {
            let v = f.new_val();
            f.arena[n].phis.push(Instr::new(Op::Phi, v, args, vec![]));
            newargs.push(v);
        }
    }
    f.arena[n].preds = outside.clone();
    f.arena[n].ins = vec![Instr::no_d(Op::Jmp, vec![], vec![h])];
    for &p in &outside {
        for t in f.arena[p].ins.last_mut().unwrap().t.iter_mut() {
            if *t == h {
                *t = n;
            }
        }
    }
    for &i in idxs.iter().rev() {
        f.arena[h].preds.remove(i);
        for phi in f.arena[h].phis.iter_mut() {
            phi.a.remove(i);
        }
    }
    f.arena[h].preds.push(n);
    for (pi, v) in newargs.into_iter().enumerate() {
        f.arena[h].phis[pi].a.push(v);
    }
    let pos = f.blocks.iter().position(|&x| x == h).unwrap();
    f.blocks.insert(pos, n);
    f.arena[n].idom = None;
    f.arena[n].depth = f.arena[h].depth.saturating_sub(1);
    Some(n)
}

/// Move computations that give the same result in every round to the loop preheader. Only
/// instructions that cannot trap are moved (a load also needs: no store in the loop may
/// write its address).
pub fn licm(f: &mut Func, rng: &Ranges) -> bool {
    analyze(f);
    let mut order: Vec<usize> = (0..f.loops.len()).collect();
    order.sort_by_key(|&i| (std::cmp::Reverse(f.loops[i].depth), f.arena[f.loops[i].header].rpo));
    if order.is_empty() {
        return false;
    }
    let mi = MemInfo::new(rng);
    let nv = f.nval.max(0) as usize + 1;
    let mut defblk: Vec<Option<usize>> = vec![None; nv + 1];
    for &b in &f.blocks {
        for phi in &f.arena[b].phis {
            defblk[phi.d as usize] = Some(b);
        }
        for ins in &f.arena[b].ins {
            if ins.has_d() {
                defblk[ins.d as usize] = Some(b);
            }
        }
    }
    let mut changed = false;
    for li in order {
        let lp_ids: HashSet<usize> = f.loops[li].ids.clone();
        let lp_blocks: Vec<usize> = f.loops[li].blocks.clone();
        let header = f.loops[li].header;
        let invariant = |ins: &Instr, defblk: &Vec<Option<usize>>| -> bool {
            ins.a.iter().all(|&x| x < 0 || defblk.get(x as usize).copied().flatten().map_or(true, |db| !lp_ids.contains(&db)))
        };
        let mut cand = false;
        'outer: for &b in &lp_blocks {
            for ins in &f.arena[b].ins {
                if !ins.has_d() {
                    continue;
                }
                if invariant(ins, &defblk) && ins.op != Op::Vload {
                    cand = true;
                    break 'outer;
                }
            }
        }
        if !cand {
            continue;
        }
        let pre = match get_preheader(f, li) {
            Some(p) => p,
            None => continue,
        };
        let mut par = f.loops[li].parent;
        while let Some(pi) = par {
            if !f.loops[pi].ids.contains(&pre) {
                f.loops[pi].ids.insert(pre);
                let pos = f.loops[pi].blocks.iter().position(|&x| x == header).unwrap();
                f.loops[pi].blocks.insert(pos, pre);
            }
            par = f.loops[pi].parent;
        }
        let mut stores: Vec<(MKey, Iv)> = Vec::new();
        if lp_blocks.iter().any(|&b| f.arena[b].ins.iter().any(|i| i.op == Op::Load)) {
            for &bb in &lp_blocks {
                for i2 in &f.arena[bb].ins {
                    if i2.op == Op::Store {
                        stores.push((mi.key(i2.a[0]), rng.get(i2.a[0], bb)));
                    }
                }
            }
        }
        let mut moved: Vec<Instr> = Vec::new();
        for &b in &lp_blocks {
            let old = std::mem::take(&mut f.arena[b].ins);
            let mut out = Vec::with_capacity(old.len());
            for ins in old {
                if !ins.has_d() || ins.op == Op::Vload {
                    out.push(ins);
                    continue;
                }
                if !invariant(&ins, &defblk) {
                    out.push(ins);
                    continue;
                }
                let op = ins.op;
                if is_pure(op) {
                } else if op == Op::Div || op == Op::Mod {
                    if rng.may_trap(&ins, pre) {
                        out.push(ins);
                        continue;
                    }
                } else if op == Op::Load {
                    if rng.may_trap(&ins, pre) {
                        out.push(ins);
                        continue;
                    }
                    let k = mi.key(ins.a[0]);
                    let r = rng.get(ins.a[0], pre);
                    let hit = stores.iter().any(|(k2, r2)| may_alias(k, r, *k2, *r2));
                    if hit {
                        out.push(ins);
                        continue;
                    }
                } else {
                    out.push(ins);
                    continue;
                }
                defblk[ins.d as usize] = Some(pre);
                moved.push(ins);
            }
            f.arena[b].ins = out;
        }
        if !moved.is_empty() {
            let at = f.arena[pre].ins.len() - 1;
            f.arena[pre].ins.splice(at..at, moved);
            changed = true;
        }
    }
    changed
}

// ---------------------------------------------------------------------------
// Lowering: choose machine instructions (fused ops, shifts instead of multiplies)
// ---------------------------------------------------------------------------
/// Run last on SSA form. Picks mac / addsh when the machine has them and turns
/// multiplications by powers of two into shifts when that is not slower.
pub fn lower(f: &mut Func, mc: &Machine, use_mac: bool) {
    let alu = mc.lat_alu;
    let lmul = mc.lat_mul;
    let has_mac = mc.fused_mac != 0 && use_mac && mc.lat_mac <= lmul + alu;
    let has_addsh = mc.fused_addsh != 0;
    let lash = mc.lat_addsh;
    let shift_lat = alu.min(lmul);
    let uc = use_counts(f);
    let nv = f.nval.max(0) as usize + 1;
    let mut defloc: Vec<Option<(usize, usize)>> = vec![None; nv + 1];
    for &b in &f.blocks {
        for (i, ins) in f.arena[b].ins.iter().enumerate() {
            if ins.has_d() {
                defloc[ins.d as usize] = Some((b, i));
            }
        }
    }
    let mut dead = vec![false; nv + 1];
    for bi in 0..f.blocks.len() {
        let b = f.blocks[bi];
        for i in 0..f.arena[b].ins.len() {
            if f.arena[b].ins[i].op != Op::Add {
                continue;
            }
            let (x, y) = (f.arena[b].ins[i].a[0], f.arena[b].ins[i].a[1]);
            for (p, q) in [(x, y), (y, x)] {
                if p < 0 || uc[p as usize] != 1 {
                    continue;
                }
                let (db, di) = match defloc[p as usize] {
                    Some(l) => l,
                    None => continue,
                };
                let dp = &f.arena[db].ins[di];
                if dp.op != Op::Mul || dead[dp.d as usize] || db != b {
                    continue;
                }
                let c = if dp.a[1] < 0 { Some(!dp.a[1]) } else { None };
                let (dpa0, dpa1, dpd) = (dp.a[0], dp.a[1], dp.d);
                let cp2 = c.map_or(false, pow2);
                if cp2 && has_addsh && lash <= shift_lat + alu {
                    let ins = &mut f.arena[b].ins[i];
                    ins.op = Op::Addsh;
                    ins.a = vec![q, dpa0, lit(bit_length(c.unwrap()) - 1)];
                    dead[dpd as usize] = true;
                    break;
                }
                if has_mac && !(cp2 && alu < lmul) {
                    let ins = &mut f.arena[b].ins[i];
                    ins.op = Op::Mac;
                    ins.a = vec![dpa0, dpa1, q];
                    dead[dpd as usize] = true;
                    break;
                }
            }
        }
    }
    for bi in 0..f.blocks.len() {
        let b = f.blocks[bi];
        let old = std::mem::take(&mut f.arena[b].ins);
        let mut out = Vec::with_capacity(old.len());
        for mut ins in old {
            if ins.has_d() && dead[ins.d as usize] {
                continue;
            }
            if ins.op == Op::Mul && ins.a[1] < 0 {
                let c = !ins.a[1];
                if pow2(c) && alu <= lmul {
                    ins.op = Op::Shl;
                    ins.a = vec![ins.a[0], lit(bit_length(c) - 1)];
                } else if c > 2 && pow2(c - 1) && has_addsh && lash < lmul {
                    ins.op = Op::Addsh;
                    ins.a = vec![ins.a[0], ins.a[0], lit(bit_length(c - 1) - 1)];
                }
            }
            out.push(ins);
        }
        f.arena[b].ins = out;
    }
}
