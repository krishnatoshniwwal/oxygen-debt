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
///
/// `nest`: the budget applies to the outermost loop that contains the loop being peeled (the code
/// that runs repeatedly and has to stay in the instruction cache), not to the whole program.
pub fn peel_const_loops(f: &mut Func, trips: &BTreeMap<usize, usize>, budget: usize, maxtrip: usize, nest: bool) -> bool {
    if trips.is_empty() {
        return false;
    }
    compute_preds(f);
    analyze(f);
    let mut size = f.count_instrs();
    let mut root_added: HashMap<usize, usize> = HashMap::new();
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
        let mut root = li;
        while let Some(p) = f.loops[root].parent {
            root = p;
        }
        if nest {
            let rsize: usize = f.loops[root].blocks.iter().map(|&b| f.arena[b].ins.len()).sum::<usize>() + root_added.get(&root).copied().unwrap_or(0);
            if rsize + n * lsize > budget {
                continue;
            }
        } else if size + n * lsize > budget {
            continue;
        }
        if peel(f, header, n) {
            size += n * lsize;
            *root_added.entry(root).or_insert(0) += n * lsize;
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

/// Induction-variable rewrite (SSA form). For a loop with one entry edge (from `p0`) and an
/// induction variable `i = phi(i0, i + c)` (c a constant, the same `i + c` on every back
/// edge), every `a = i + k` inside the loop (k a constant) is replaced by a new induction
/// variable `ai = phi(i0 + k, ai + c)`: one add per iteration either way, but `a` no longer
/// depends on `i` (shorter chains). Exact for all values: add is arithmetic modulo 2^32, so
/// `ai` equals `i + k` in every iteration.
///
/// Then the exit test: `lt i, n` (or `lt n, i`) with `n` loop-invariant becomes
/// `lt ai, n + k` (`n + k` computed once in `p0`) when the ranges prove that `i + k` and
/// `n + k` both stay in [0, 2^31) (so the signed comparison is unchanged). If `i` has no
/// other use left, dead-code removal deletes it and its add.
pub fn iv_rewrite(f: &mut Func, rng: &Ranges) -> bool {
    if !rng.ok {
        return false;
    }
    let _ = analyze(f);
    let nv0 = f.nval.max(0) as usize + 1;
    // defining block of every value (instructions and phis)
    let mut defb = vec![usize::MAX; nv0];
    for &b in &f.blocks {
        for ins in f.arena[b].phis.iter().chain(f.arena[b].ins.iter()) {
            if ins.has_d() && ins.d >= 0 && (ins.d as usize) < nv0 {
                defb[ins.d as usize] = b;
            }
        }
    }
    let mut rep = vec![crate::passes::NOREP; nv0];
    let mut changed = false;
    let nloops = f.loops.len();
    for li in 0..nloops {
        let h = f.loops[li].header;
        let ids = f.loops[li].ids.clone();
        let blocks = f.loops[li].blocks.clone();
        let preds = f.arena[h].preds.clone();
        let outside: Vec<usize> = (0..preds.len()).filter(|&j| !ids.contains(&preds[j])).collect();
        if outside.len() != 1 || preds.len() < 2 {
            continue;
        }
        let po = outside[0];
        let p0 = preds[po];
        if p0 == h || f.arena[p0].ins.is_empty() {
            continue;
        }
        let nphis = f.arena[h].phis.len();
        for pi in 0..nphis {
            let phi = f.arena[h].phis[pi].clone();
            let i = phi.d;
            if i < 0 || (i as usize) >= nv0 {
                continue;
            }
            let i0 = phi.a[po];
            // the same i + c on every back edge
            let mut inext: Option<i64> = None;
            let mut ok = true;
            for (j, &x) in phi.a.iter().enumerate() {
                if j == po {
                    continue;
                }
                if x < 0 || inext.map_or(false, |y| y != x) {
                    ok = false;
                    break;
                }
                inext = Some(x);
            }
            let inext = match (ok, inext) {
                (true, Some(x)) if (x as usize) < nv0 => x,
                _ => continue,
            };
            let bn = defb[inext as usize];
            if bn == usize::MAX || !ids.contains(&bn) {
                continue;
            }
            let pos = match f.arena[bn].ins.iter().position(|ins| ins.d == inext) {
                Some(p) => p,
                None => continue,
            };
            // the step: a constant, or a value defined outside the loop (loop-invariant, and its
            // definition dominates the loop because SSA uses are dominated by their definitions)
            let invariant = |x: i64| x < 0 || ((x as usize) < nv0 && x != i && defb[x as usize] != usize::MAX && !ids.contains(&defb[x as usize]));
            let c = {
                let ins = &f.arena[bn].ins[pos];
                if ins.op != Op::Add {
                    continue;
                }
                if ins.a[0] == i && invariant(ins.a[1]) {
                    ins.a[1]
                } else if ins.a[1] == i && invariant(ins.a[0]) {
                    ins.a[0]
                } else {
                    continue;
                }
            };
            // the adds of a constant to i inside the loop (not the step itself)
            let mut ks: Vec<i64> = Vec::new();
            for &b in &blocks {
                for ins in &f.arena[b].ins {
                    if ins.op == Op::Add && ins.d != inext && ins.d >= 0 && (ins.d as usize) < nv0 && rep[ins.d as usize] == crate::passes::NOREP {
                        let k = if ins.a[0] == i && ins.a[1] < 0 {
                            ins.a[1]
                        } else if ins.a[1] == i && ins.a[0] < 0 {
                            ins.a[0]
                        } else {
                            continue;
                        };
                        if k != L0 && !ks.contains(&k) {
                            ks.push(k);
                        }
                    }
                }
            }
            if ks.is_empty() || ks.len() > 4 {
                continue;
            }
            let mut made: Vec<(i64, i64)> = Vec::new(); // (k literal, ai)
            for &k in &ks {
                let pre = f.new_val();
                let ai = f.new_val();
                let ainext = f.new_val();
                // i0 + k at the end of p0 (before its branch)
                let t = f.arena[p0].ins.pop().unwrap();
                f.arena[p0].ins.push(Instr::new(Op::Add, pre, vec![i0, k], vec![]));
                f.arena[p0].ins.push(t);
                // ai + c right after i + c
                f.arena[bn].ins.insert(pos + 1, Instr::new(Op::Add, ainext, vec![ai, c], vec![]));
                let args: Vec<i64> = (0..preds.len()).map(|j| if j == po { pre } else { ainext }).collect();
                f.arena[h].phis.push(Instr::new(Op::Phi, ai, args, vec![]));
                made.push((k, ai));
                changed = true;
            }
            // replace a = i + k by ai
            let made2 = made.clone();
            for &b in &blocks {
                for ins in &f.arena[b].ins {
                    if ins.op != Op::Add || ins.d == inext || ins.d < 0 || (ins.d as usize) >= nv0 {
                        continue;
                    }
                    let k = if ins.a[0] == i && ins.a[1] < 0 {
                        ins.a[1]
                    } else if ins.a[1] == i && ins.a[0] < 0 {
                        ins.a[0]
                    } else {
                        continue;
                    };
                    if let Some(&(_, ai)) = made2.iter().find(|m| m.0 == k) {
                        rep[ins.d as usize] = ai;
                    }
                }
            }
            // exit tests: lt i, n  /  lt n, i  with n invariant, i + k and n + k in [0, 2^31)
            let (kl, ai) = match made.iter().find(|m| !m.0 >= 0) {
                Some(&m) => m,
                None => continue,
            };
            let kv = !kl; // the constant value (literals are stored as !value)
            let ri = rng.get(i, h);
            if ri.1 > SMAX_I - kv {
                continue;
            }
            for bi in 0..blocks.len() {
                let b = blocks[bi];
                for ii in 0..f.arena[b].ins.len() {
                    let ins = f.arena[b].ins[ii].clone();
                    if ins.op != Op::Lt {
                        continue;
                    }
                    let side = if ins.a[0] == i { 1 } else if ins.a[1] == i { 0 } else { continue };
                    let n = ins.a[side];
                    let inv = n < 0 || (n as usize) >= nv0 || (defb[n as usize] != usize::MAX && !ids.contains(&defb[n as usize]));
                    if !inv || (n >= 0 && (n as usize) >= nv0) {
                        continue;
                    }
                    let rn = rng.get(n, b);
                    if rn.1 > SMAX_I - kv {
                        continue;
                    }
                    let nk = if n < 0 {
                        lit(!n + kv)
                    } else {
                        let v = f.new_val();
                        let t = f.arena[p0].ins.pop().unwrap();
                        f.arena[p0].ins.push(Instr::new(Op::Add, v, vec![n, kl], vec![]));
                        f.arena[p0].ins.push(t);
                        v
                    };
                    let e = &mut f.arena[b].ins[ii];
                    e.a[1 - side] = ai;
                    e.a[side] = nk;
                }
            }
        }
    }
    if changed {
        rep.resize(f.nval.max(0) as usize + 1, crate::passes::NOREP);
        crate::passes::apply_rep(f, &rep);
    }
    changed
}

const SMAX_I: i64 = 0x7FFF_FFFF;

/// Lazy modular reduction (SSA form). In a loop with one entry edge, a counter
/// `i = phi(i0, i + 1)` (i0 a constant >= 0) and the header test `br (lt i, n), body, exit`,
/// a running value `s = phi(s0, (s + t) % m)` (m a constant >= 1) is no longer reduced in
/// the loop when
/// * `s` is used in the loop only by that add, `s + t` only by the `%`, and the `%` only by
///   the phi; after the loop `s` is used only by ordinary instructions (not phis);
/// * the add runs at most once per iteration (its block is not in an inner loop);
/// * with T = the largest t, S0 = the largest s0, N = (largest n) - i0 (the most iterations):
///   S0 + N * T < 2^31 (proven by the value ranges).
///
/// Then every unreduced value is in [0, 2^31), where `%` is the ordinary remainder, and
/// ((s0 + t1) % m + t2) % m ... == (s0 + t1 + t2 + ...) % m. Each use after the loop gets
/// `r = s % m` right before it. The division leaves the loop-carried chain.
pub fn lazy_mod(f: &mut Func, rng: &Ranges) -> bool {
    if !rng.ok {
        return false;
    }
    let _ = analyze(f);
    let nv = f.nval.max(0) as usize + 1;
    let mut defloc: Vec<Option<(usize, usize)>> = vec![None; nv]; // (block, index) of instructions
    for &b in &f.blocks {
        for (k, ins) in f.arena[b].ins.iter().enumerate() {
            if ins.has_d() && ins.d >= 0 && (ins.d as usize) < nv {
                defloc[ins.d as usize] = Some((b, k));
            }
        }
    }
    // all uses: (block, is_phi)
    let mut uses: Vec<Vec<(usize, bool)>> = vec![Vec::new(); nv];
    for &b in &f.blocks {
        for phi in &f.arena[b].phis {
            for &x in &phi.a {
                if x >= 0 && (x as usize) < nv {
                    uses[x as usize].push((b, true));
                }
            }
        }
        for ins in &f.arena[b].ins {
            for &x in &ins.a {
                if x >= 0 && (x as usize) < nv {
                    uses[x as usize].push((b, false));
                }
            }
        }
    }
    let mut rep = vec![crate::passes::NOREP; nv];
    let mut changed = false;
    for li in 0..f.loops.len() {
        let h = f.loops[li].header;
        let ids = f.loops[li].ids.clone();
        let preds = f.arena[h].preds.clone();
        let outside: Vec<usize> = (0..preds.len()).filter(|&j| !ids.contains(&preds[j])).collect();
        if outside.len() != 1 || preds.len() < 2 {
            continue;
        }
        let po = outside[0];
        // the header test: br c, inside, outside with c = lt i, n
        let term = f.arena[h].ins.last().unwrap().clone();
        if term.op != Op::Br || term.a[0] < 0 || !ids.contains(&term.t[0]) || ids.contains(&term.t[1]) {
            continue;
        }
        let (cb, ck) = match defloc.get(term.a[0] as usize).copied().flatten() {
            Some(x) => x,
            None => continue,
        };
        let cond = f.arena[cb].ins[ck].clone();
        if cb != h || cond.op != Op::Lt || cond.a[0] < 0 {
            continue;
        }
        let (iv, n) = (cond.a[0], cond.a[1]);
        // i = phi(i0 const >= 0, i + 1 on every back edge)
        let iphi = match f.arena[h].phis.iter().find(|p| p.d == iv) {
            Some(p) => p.clone(),
            None => continue,
        };
        let i0 = iphi.a[po];
        if i0 >= 0 || !i0 > SMAX_I {
            continue;
        }
        let i0v = !i0;
        let mut step_ok = true;
        for (j, &x) in iphi.a.iter().enumerate() {
            if j == po {
                continue;
            }
            let okx = x >= 0
                && match defloc.get(x as usize).copied().flatten() {
                    Some((b, k)) => {
                        let ins = &f.arena[b].ins[k];
                        ins.op == Op::Add && ((ins.a[0] == iv && ins.a[1] == lit(1)) || (ins.a[1] == iv && ins.a[0] == lit(1)))
                    }
                    None => false,
                };
            if !okx {
                step_ok = false;
                break;
            }
        }
        if !step_ok {
            continue;
        }
        let rn = if n < 0 { (!n, !n) } else { rng.get(n, h) };
        if rn.1 > SMAX_I {
            continue;
        }
        let ntrip = (rn.1 - i0v).max(0) as i128;
        let phis = f.arena[h].phis.clone();
        for sphi in phis {
            let s = sphi.d;
            if s == iv || s < 0 || (s as usize) >= nv {
                continue;
            }
            let s0 = sphi.a[po];
            let mut snext: Option<i64> = None;
            let mut ok = true;
            for (j, &x) in sphi.a.iter().enumerate() {
                if j == po {
                    continue;
                }
                if x < 0 || snext.map_or(false, |y| y != x) {
                    ok = false;
                    break;
                }
                snext = Some(x);
            }
            let snext = match (ok, snext) {
                (true, Some(x)) if (x as usize) < nv => x,
                _ => continue,
            };
            let (bm, km) = match defloc[snext as usize] {
                Some(x) => x,
                None => continue,
            };
            let modi = f.arena[bm].ins[km].clone();
            if modi.op != Op::Mod || modi.a[1] >= 0 || modi.a[0] < 0 {
                continue;
            }
            let m = !modi.a[1];
            if m < 1 || m > SMAX_I {
                continue;
            }
            let x = modi.a[0];
            let (bx, kx) = match defloc.get(x as usize).copied().flatten() {
                Some(v) => v,
                None => continue,
            };
            let addi = f.arena[bx].ins[kx].clone();
            if addi.op != Op::Add {
                continue;
            }
            let t = if addi.a[0] == s { addi.a[1] } else if addi.a[1] == s { addi.a[0] } else { continue };
            if t == s {
                continue;
            }
            // the add and the mod run at most once per iteration of this loop
            if f.arena[bx].lp != Some(li) || f.arena[bm].lp != Some(li) {
                continue;
            }
            // uses: s only by the add inside the loop, otherwise outside and not in phis;
            // x only by the mod; snext only by the phi
            let su = &uses[s as usize];
            let inside: Vec<&(usize, bool)> = su.iter().filter(|u| ids.contains(&u.0)).collect();
            if inside.len() != 1 || inside[0].1 || su.iter().any(|u| !ids.contains(&u.0) && u.1) {
                continue;
            }
            if uses[x as usize].len() != 1 || uses[snext as usize].iter().any(|u| !(u.1 && u.0 == h)) {
                continue;
            }
            // the bound
            let rt = if t < 0 { (!t, !t) } else { rng.get(t, bx) };
            let rs0 = if s0 < 0 { (!s0, !s0) } else { rng.get(s0, h) };
            if rt.1 > SMAX_I || rs0.1 > SMAX_I {
                continue;
            }
            // with no iteration at all the original keeps s0 unreduced, but a use after the loop
            // reads s % m: only equal when s0 < m
            if rs0.1 >= m {
                continue;
            }
            if (rs0.1 as i128) + ntrip * (rt.1 as i128) > SMAX_I as i128 {
                continue;
            }
            // rewrite: the loop carries x; uses after the loop read s % m
            rep[snext as usize] = x;
            for bi in 0..f.blocks.len() {
                let b = f.blocks[bi];
                if ids.contains(&b) || !f.arena[b].ins.iter().any(|ins| ins.a.contains(&s)) {
                    continue;
                }
                let old = std::mem::take(&mut f.arena[b].ins);
                let mut out = Vec::with_capacity(old.len() + 1);
                for mut ins in old {
                    if ins.a.contains(&s) {
                        let r = f.new_val();
                        out.push(Instr::new(Op::Mod, r, vec![s, lit(m)], vec![]));
                        for a in ins.a.iter_mut() {
                            if *a == s {
                                *a = r;
                            }
                        }
                    }
                    out.push(ins);
                }
                f.arena[b].ins = out;
            }
            changed = true;
        }
    }
    if changed {
        rep.resize(f.nval.max(0) as usize + 1, crate::passes::NOREP);
        crate::passes::apply_rep(f, &rep);
    }
    changed
}

/// May `ins` trap when run at the end of block `p`? Like `Ranges::may_trap`, but a value defined
/// in the block being emptied is only trusted through `mrange`.
fn spec_may_trap(rng: &Ranges, ins: &Instr, p: usize, in_s: &std::collections::HashSet<i64>, mrange: &HashMap<i64, (i64, i64)>) -> bool {
    let rg = |x: i64| -> (i64, i64) {
        if x < 0 {
            (!x, !x)
        } else if let Some(&r) = mrange.get(&x) {
            r
        } else if in_s.contains(&x) {
            (0, 0xFFFF_FFFF)
        } else {
            rng.get(x, p)
        }
    };
    match ins.op {
        Op::Load | Op::Store => !rng.ok || rg(ins.a[0]).1 >= 4096,
        Op::Div | Op::Mod => {
            let y = ins.a[1];
            if y < 0 {
                return !y == 0;
            }
            !rng.ok || rg(y).0 < 1
        }
        op => op == Op::Trap,
    }
}

/// Speculation inside loops (SSA form, before scheduling). A block `s` whose only
/// predecessor `p` ends in a two-way branch, both in the same innermost loop: move an
/// instruction of `s` into `p`, just before the branch, when
/// * it is pure (`add`, `mul`, `select`, `mac`, ...), or a `div`/`mod` that the value ranges
///   prove cannot trap *at p* (divisor never 0 on any path through p), and
/// * each operand is a literal or is defined outside `s` (then its definition dominates p,
///   because p is the only way into s).
///
/// The moved instruction now also runs when the branch leaves the loop; that is harmless
/// because it has no effect and cannot trap, and its value is only used where it was used
/// before (all those places are dominated by p). Repeated sweeps move work up a whole chain
/// of unrolled loop copies, so the scheduler can overlap long latencies (`mod` 12, `mul` 3)
/// of several iterations. At most `limit` instructions leave one block per sweep, and at most
/// `sweeps` sweeps run: 1 sweep moves work up by one block only (few extra live values),
/// more sweeps move it further (more overlap, but more names alive at once, which costs
/// spills on machines with few registers). Whether this pays is decided by the search with
/// the exact timing model.
pub fn speculate(f: &mut Func, rng: &Ranges, limit: usize, sweeps: usize) -> bool {
    if !rng.ok {
        return false;
    }
    // NOT compute_preds: in SSA the order of `preds` matches the phi operands, and
    // compute_preds would rebuild it in block order (wrong phi values).
    let rpo = analyze(f);
    let nv = f.nval.max(0) as usize + 1;
    // block that defines each value (usize::MAX = parameter / not defined in a block)
    let mut defb = vec![usize::MAX; nv];
    for &b in &rpo {
        for ins in f.arena[b].phis.iter().chain(f.arena[b].ins.iter()) {
            if ins.has_d() && ins.d >= 0 && (ins.d as usize) < nv {
                defb[ins.d as usize] = b;
            }
        }
    }
    // Values that feed a phi (loop-carried values, values leaving the loop) are not moved:
    // moving one above an exit test makes the exit need the old value too (extra copies and
    // an extra live name). The long-latency work that feeds them still moves.
    let mut phi_use = vec![false; nv];
    for &b in &rpo {
        for phi in &f.arena[b].phis {
            for &x in &phi.a {
                if x >= 0 && (x as usize) < nv {
                    phi_use[x as usize] = true;
                }
            }
        }
    }
    let mut changed = false;
    // values moved by this call (in any sweep): their ranges, computed where they used to be,
    // are stale; `mrange` holds a recomputed range where one could be proven
    let mut moved_vals: std::collections::HashSet<i64> = std::collections::HashSet::new();
    let mut mrange: HashMap<i64, (i64, i64)> = HashMap::new();
    for _ in 0..sweeps {
        let mut moved_any = false;
        // blocks that received work in this sweep give nothing away in the same sweep, so
        // one sweep moves each instruction by at most one block
        let mut got = vec![false; f.arena.len()];
        for &s in &rpo {
            if got[s] {
                continue;
            }
            let sb = &f.arena[s];
            if sb.preds.len() != 1 || !sb.phis.is_empty() || sb.lp.is_none() {
                continue;
            }
            let p = sb.preds[0];
            if p == s || f.arena[p].lp != sb.lp {
                continue;
            }
            match f.arena[p].ins.last() {
                Some(t) if t.op == Op::Br && t.t.len() == 2 && t.t[0] != t.t[1] => {}
                _ => continue,
            }
            let n = f.arena[s].ins.len();
            if n < 2 {
                continue;
            }
            // Only long-latency work is worth moving (div, mod, mul, mac), together with the
            // instructions of `s` that compute its operands. Cheap ops would only add live
            // names. Marked backwards: operands come before their uses.
            let mut want = vec![false; n];
            {
                let body = &f.arena[s].ins;
                let mut need: std::collections::HashSet<i64> = std::collections::HashSet::new();
                for i in (0..n.saturating_sub(1)).rev() {
                    let ins = &body[i];
                    let long = matches!(ins.op, Op::Div | Op::Mod | Op::Mul | Op::Mac | Op::Load);
                    if long || (ins.has_d() && need.contains(&ins.d)) {
                        want[i] = true;
                        for &x in &ins.a {
                            if x >= 0 {
                                need.insert(x);
                            }
                        }
                    }
                }
            }
            let mut keep: Vec<Instr> = Vec::with_capacity(n);
            let mut moved: Vec<Instr> = Vec::new();
            let body = std::mem::take(&mut f.arena[s].ins);
            let last = n - 1;
            // Ranges of values defined in `s` were computed under the conditions that hold in `s`
            // (for example the loop test); they do not hold at `p`. A trap-freedom proof at `p`
            // may therefore use only values defined outside `s`, or the range of a moved
            // instruction whose operands are all defined outside `s`, recomputed at `p`.
            let mut in_s: std::collections::HashSet<i64> = body.iter().filter(|i| i.has_d()).map(|i| i.d).collect();
            in_s.extend(moved_vals.iter().copied());
            // a load may only move if no store / volatile op comes before it in s (then it
            // still runs after every memory operation it ran after before: those of p)
            let mut mem_before = false;
            for (i, ins) in body.into_iter().enumerate() {
                let is_load = ins.op == Op::Load;
                let ok = i < last
                    && want[i]
                    && moved.len() < limit
                    && ins.has_d()
                    && !(ins.d >= 0 && (ins.d as usize) < nv && phi_use[ins.d as usize])
                    && (is_pure(ins.op)
                        || (matches!(ins.op, Op::Div | Op::Mod) && !spec_may_trap(rng, &ins, p, &in_s, &mrange))
                        || (is_load && !mem_before && !spec_may_trap(rng, &ins, p, &in_s, &mrange)))
                    && ins.a.iter().all(|&x| x < 0 || (x as usize) >= nv || defb[x as usize] != s);
                if !ok && matches!(ins.op, Op::Store | Op::Vstore | Op::Vload) {
                    mem_before = true;
                }
                if ok {
                    if ins.has_d() {
                        mrange.remove(&ins.d);
                        if ins.a.iter().all(|&x| x < 0 || !in_s.contains(&x)) {
                            if let Some(r) = rng.xfer(&ins, p) {
                                mrange.insert(ins.d, r);
                            }
                        }
                        moved_vals.insert(ins.d);
                    }
                    if ins.d >= 0 && (ins.d as usize) < nv {
                        defb[ins.d as usize] = p;
                    }
                    moved.push(ins);
                } else {
                    keep.push(ins);
                }
            }
            f.arena[s].ins = keep;
            if !moved.is_empty() {
                let pb = &mut f.arena[p].ins;
                let term = pb.pop().unwrap();
                pb.extend(moved);
                pb.push(term);
                got[p] = true;
                moved_any = true;
                changed = true;
            }
        }
        if !moved_any {
            break;
        }
    }
    changed
}

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
