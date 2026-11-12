//! reroll.rs - partial loop rerolling (RoLAG-style) on SSA form, inside one basic block.
//!
//! A long straight-line block often is `n` copies of the same `p` instructions that differ only in
//! literal operands (a load address 0,1,2,..., a stride constant 7,10,13,...). Beyond the
//! instruction cache every executed instruction costs the fetch penalty, so such a run is turned
//! back into a loop that fits in the cache. The pass works like this:
//!
//!  1. every instruction gets a signature: its op and, for every operand, "literal", "defined d
//!     instructions earlier in this block" or "defined elsewhere (this value)". A repeated block of
//!     p instructions is a stretch where signature[i] == signature[i + p].
//!  2. candidate periods p come from repeated windows of signatures; each candidate is checked
//!     exactly (not by hash): every operand slot must behave the same way in every copy
//!     (same-copy value, previous-copy value = loop-carried, outside value, or a literal sequence
//!     that is constant, affine, affine modulo 2^k, or periodic).
//!  3. g copies are put in one loop body (so periodic literals become plain literals and the body
//!     fits the cache); affine literals are produced from an induction variable; the copies that
//!     do not fill a whole iteration stay straight-line after the loop.
//!
//! The transformation executes exactly the same instructions in the same order as the original
//! copies, plus the loop control, so it cannot change traps, memory or volatile effects. It is
//! only applied when everything is proven consistent; otherwise the block is left alone.
//! No recursion; every loop is bounded by the block size times small constants.
use crate::ir::*;
use crate::kir::*;
use std::collections::{BTreeMap, HashMap, HashSet};

const PMAX: usize = 2048; // longest period looked for
const DMAX: usize = 3 * PMAX + 8;

fn mix(h: u64, x: u64) -> u64 {
    (h ^ x).wrapping_mul(0x9E37_79B9_7F4A_7C15).rotate_left(23)
}

#[derive(Clone, Debug)]
enum Lits {
    Const(u32),
    /// c0 + j*delta (wrapping), optionally masked with `mask`
    Affine { c0: u32, delta: u32, mask: Option<u32>, period: Option<usize>, bias: u32 },
    Periodic(Vec<u32>),
}

#[derive(Clone, Debug)]
enum Slot {
    Lit(Lits),
    Ext(i64),
    Same(usize),
    Prev(usize),
}

struct Plan {
    s: usize,
    p: usize,
    n: usize,
    slots: Vec<Vec<Slot>>,       // [offset][operand]
    inits: BTreeMap<usize, i64>, // offset -> value entering the loop (for Prev slots)
}

fn lit_val(x: i64) -> u32 {
    (!x) as u32
}

fn classify(c: &[u32]) -> Option<Lits> {
    let n = c.len();
    if c.iter().all(|&v| v == c[0]) {
        return Some(Lits::Const(c[0]));
    }
    let delta = c[1].wrapping_sub(c[0]);
    let at = |j: usize| c[0].wrapping_add((j as u32).wrapping_mul(delta));
    if (0..n).all(|j| c[j] == at(j)) {
        return Some(Lits::Affine { c0: c[0], delta, mask: None, period: None, bias: 0 });
    }
    let maxp = (n / 2).min(4096);
    let period = (2..=maxp).find(|&pi| (pi..n).all(|j| c[j] == c[j - pi]));
    // c_j = bias + ((c0' + j*delta) & mask): bias = the smallest value (an array base address)
    let lo = *c.iter().min().unwrap();
    for bias in if lo == 0 { vec![0] } else { vec![0, lo] } {
        let d0 = c[0] - bias;
        for k in 1..=31u32 {
            let m = (1u32 << k) - 1;
            if d0 <= m && (0..n).all(|j| c[j].wrapping_sub(bias) == d0.wrapping_add((j as u32).wrapping_mul(delta)) & m) {
                return Some(Lits::Affine { c0: d0, delta, mask: Some(m), period, bias });
            }
        }
    }
    period.map(|pi| Lits::Periodic(c[..pi].to_vec()))
}

struct BlockInfo {
    sig: Vec<u64>,
    def_idx: HashMap<i64, usize>,
}

fn block_info(ins: &[Instr]) -> BlockInfo {
    let l = ins.len();
    let mut def_idx: HashMap<i64, usize> = HashMap::with_capacity(l);
    for (i, x) in ins.iter().enumerate() {
        if x.has_d() {
            def_idx.insert(x.d, i);
        }
    }
    // values used more than once far from their definition are treated as outside values
    let mut far: Vec<u8> = vec![0; l];
    for (i, x) in ins.iter().enumerate() {
        for &o in &x.a {
            if o >= 0 {
                if let Some(&j) = def_idx.get(&o) {
                    if i - j > DMAX && far[j] < 2 {
                        far[j] += 1;
                    }
                }
            }
        }
    }
    let mut sig = Vec::with_capacity(l);
    for (i, x) in ins.iter().enumerate() {
        let mut h = mix(0x1234, x.op as u64);
        h = mix(h, x.a.len() as u64);
        for &o in &x.a {
            if o < 0 {
                h = mix(h, 1);
            } else if let Some(&j) = def_idx.get(&o) {
                if far[j] < 2 && i - j <= DMAX {
                    h = mix(mix(h, 2), (i - j) as u64);
                } else {
                    h = mix(mix(h, 3), o as u64);
                }
            } else {
                h = mix(mix(h, 3), o as u64);
            }
        }
        sig.push(h);
    }
    BlockInfo { sig, def_idx }
}

/// Candidate regions (start, period, copies), best first.
fn find_candidates(bi: &BlockInfo, l: usize) -> Vec<(usize, usize, usize)> {
    const W: usize = 4;
    if l < 8 {
        return Vec::new();
    }
    let sig = &bi.sig;
    let mut last: HashMap<u64, usize> = HashMap::new();
    let mut votes: BTreeMap<usize, u32> = BTreeMap::new();
    for i in (W - 1)..l {
        let mut h = 0u64;
        for k in 0..W {
            h = mix(h, sig[i - k]);
        }
        if let Some(&i0) = last.get(&h) {
            let p = i - i0;
            if p <= PMAX {
                *votes.entry(p).or_insert(0) += 1;
            }
        }
        last.insert(h, i);
    }
    let mut cand: Vec<(u32, usize)> = votes.into_iter().map(|(p, v)| (v, p)).collect();
    cand.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    cand.truncate(8);
    let mut out: Vec<(usize, usize, usize, usize)> = Vec::new(); // (value, start, p, n)
    for &(_, p) in &cand {
        if p == 0 || l < 3 * p {
            continue;
        }
        let mut i = 0;
        while i + p < l {
            if sig[i] == sig[i + p] {
                let a = i;
                while i + p < l && sig[i] == sig[i + p] {
                    i += 1;
                }
                let n = (i - a + p) / p;
                if n >= 3 {
                    out.push(((n - 1) * p, a, p, n));
                }
            } else {
                i += 1;
            }
        }
    }
    out.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
    out.truncate(6);
    out.into_iter().map(|(_, s, p, n)| (s, p, n)).collect()
}

/// Exact check of a region; shrinks it to the longest consistent run of copies.
fn verify(ins: &[Instr], bi: &BlockInfo, mut s: usize, p: usize, mut n: usize) -> Option<Plan> {
    for _try in 0..3 {
        if n < 3 || s + n * p > ins.len() {
            return None;
        }
        match verify_once(ins, bi, s, p, n) {
            Ok(plan) => return Some(plan),
            Err(Some(good)) => {
                if good >= 3 && good < n {
                    n = good; // copies 0..good are consistent
                } else {
                    return None;
                }
            }
            Err(None) => {
                // copy 0 does not fit the loop (its inputs differ): peel it
                s += p;
                n -= 1;
            }
        }
    }
    None
}

/// Ok(plan) | Err(Some(k)): copies 0..k are consistent | Err(None): copy 0 is the problem.
fn verify_once(ins: &[Instr], bi: &BlockInfo, s: usize, p: usize, n: usize) -> Result<Plan, Option<usize>> {
    let at = |j: usize, k: usize| &ins[s + j * p + k];
    let mut slots: Vec<Vec<Slot>> = Vec::with_capacity(p);
    let mut inits: BTreeMap<usize, i64> = BTreeMap::new();
    let mut good = n;
    let mut copy0_bad = false;
    for k in 0..p {
        let t = at(1, k);
        let mut row: Vec<Slot> = Vec::with_capacity(t.a.len());
        for sg in 0..t.a.len() {
            let o1 = t.a[sg];
            let kind = if o1 < 0 {
                0
            } else {
                match bi.def_idx.get(&o1) {
                    Some(&d) if d >= s + p && d < s + p + k => 1, // same copy
                    Some(&d) if d >= s && d < s + p => 2,         // previous copy
                    _ => 3,                                        // outside
                }
            };
            let same_shape = |ij: &Instr| ij.op == t.op && ij.a.len() == t.a.len() && ij.has_d() == t.has_d();
            let slot: Slot;
            match kind {
                0 => {
                    let mut seq: Vec<u32> = Vec::with_capacity(n);
                    for j in 0..n {
                        let ij = at(j, k);
                        if !same_shape(ij) || ij.a[sg] >= 0 {
                            if j == 0 {
                                copy0_bad = true;
                            }
                            good = good.min(j);
                            break;
                        }
                        seq.push(lit_val(ij.a[sg]));
                    }
                    if seq.len() < n {
                        row.push(Slot::Lit(Lits::Const(0)));
                        continue;
                    }
                    match classify(&seq) {
                        Some(c) => slot = Slot::Lit(c),
                        None => return Err(Some(0)), // arbitrary constants: not handled
                    }
                }
                1 | 2 => {
                    let base1 = if kind == 1 { s + p } else { s };
                    let o = bi.def_idx[&o1] - base1;
                    for j in 0..n {
                        let ij = at(j, k);
                        let jb = s + j * p;
                        let x = ij.a.get(sg).copied().unwrap_or(-1);
                        let ok = same_shape(ij)
                            && x >= 0
                            && match bi.def_idx.get(&x) {
                                Some(&d) => {
                                    if kind == 1 {
                                        d >= jb && d < jb + k && d - jb == o
                                    } else if j >= 1 {
                                        d >= jb - p && d < jb && d - (jb - p) == o
                                    } else {
                                        d < jb
                                    }
                                }
                                None => kind == 2 && j == 0,
                            };
                        if !ok {
                            if j == 0 {
                                copy0_bad = true;
                            }
                            good = good.min(j);
                            break;
                        }
                        if kind == 2 && j == 0 {
                            match inits.get(&o) {
                                Some(&v) if v != x => copy0_bad = true,
                                _ => {
                                    inits.insert(o, x);
                                }
                            }
                        }
                    }
                    slot = if kind == 1 { Slot::Same(o) } else { Slot::Prev(o) };
                }
                _ => {
                    for j in 0..n {
                        let ij = at(j, k);
                        if !same_shape(ij) || ij.a[sg] != o1 {
                            if j == 0 {
                                copy0_bad = true;
                            }
                            good = good.min(j);
                            break;
                        }
                    }
                    slot = Slot::Ext(o1);
                }
            }
            row.push(slot);
        }
        if t.op == Op::Addsh {
            if let Some(Slot::Lit(l)) = row.get(2) {
                if !matches!(l, Lits::Const(_)) {
                    return Err(Some(0));
                }
            }
        }
        slots.push(row);
    }
    if copy0_bad {
        return Err(None);
    }
    if good < n {
        return Err(Some(good));
    }
    Ok(Plan { s, p, n, slots, inits })
}

fn gcd(a: usize, b: usize) -> usize {
    if b == 0 {
        a
    } else {
        gcd(b, a % b)
    }
}

/// Number of instructions a loop body with g copies has.
fn body_size(plan: &Plan, g: usize) -> usize {
    let mut size = g * plan.p;
    let mut ivs: HashSet<u32> = HashSet::new();
    let mut ref_c0: HashMap<u32, u32> = HashMap::new();
    for row in &plan.slots {
        for sl in row {
            if let Slot::Lit(Lits::Affine { c0, delta, mask, bias, .. }) = sl {
                ivs.insert(*delta);
                let r = *ref_c0.entry(*delta).or_insert(*c0);
                for m in 0..g {
                    let kk = c0.wrapping_sub(r).wrapping_add((m as u32).wrapping_mul(*delta));
                    if kk != 0 {
                        size += 1;
                    }
                    if mask.is_some() {
                        size += 1 + (*bias != 0) as usize;
                    }
                }
            }
        }
    }
    size + ivs.len() + 3
}

/// The same plan with every literal sequence that has a period written as a periodic table
/// (plain literals in the copies, no induction-variable arithmetic).
fn as_periodic(plan: &Plan) -> Plan {
    let slots = plan
        .slots
        .iter()
        .map(|row| {
            row.iter()
                .map(|sl| match sl {
                    Slot::Lit(Lits::Affine { c0, delta, mask: Some(m), period: Some(pi), bias }) => {
                        Slot::Lit(Lits::Periodic((0..*pi).map(|j| bias.wrapping_add(c0.wrapping_add((j as u32).wrapping_mul(*delta)) & m)).collect()))
                    }
                    other => other.clone(),
                })
                .collect()
        })
        .collect();
    Plan { s: plan.s, p: plan.p, n: plan.n, slots, inits: plan.inits.clone() }
}

fn choose_g(plan: &Plan, budget: usize) -> Option<usize> {
    let mut lc = 1usize;
    for row in &plan.slots {
        for sl in row {
            if let Slot::Lit(Lits::Periodic(v)) = sl {
                lc = lc / gcd(lc, v.len()) * v.len();
                if lc > plan.n / 2 {
                    return None;
                }
            }
        }
    }
    let qmax = plan.n / 2 / lc;
    if qmax == 0 {
        return None;
    }
    let mut q_fit = 1;
    for q in 1..=qmax {
        if body_size(plan, q * lc) <= budget {
            q_fit = q;
        } else {
            break;
        }
    }
    let lo = (q_fit / 2).max(1);
    let mut best: Option<(usize, usize)> = None; // (tail copies, g)
    for q in lo..=q_fit {
        let g = q * lc;
        let r = plan.n % g;
        match best {
            Some((br, _)) if br <= r => {}
            _ => best = Some((r, g)),
        }
    }
    best.map(|(_, g)| g)
}

/// Roll the region. Returns the (prefix, tail) blocks, which may still contain regions.
fn apply(f: &mut Func, b: usize, plan: &Plan, g: usize) -> Option<(usize, usize)> {
    let p = plan.p;
    let t_iters = plan.n / g;
    let gt = g * t_iters;
    let s = plan.s;
    let region_end = s + gt * p;
    let old_ins = f.arena[b].ins.clone();
    let mut covered: HashSet<i64> = HashSet::new();
    for i in s..region_end {
        if old_ins[i].has_d() {
            covered.insert(old_ins[i].d);
        }
    }
    // exit map: the values of the last iteration's copies
    let first_last = g * (t_iters - 1);
    // ---- check the uses of covered values outside the region before changing anything ----
    let mut last_vals: HashMap<i64, (usize, usize)> = HashMap::new(); // old value -> (copy in last iteration, offset)
    for j in first_last..gt {
        for k in 0..p {
            let od = &old_ins[s + j * p + k];
            if od.has_d() {
                last_vals.insert(od.d, (j - first_last, k));
            }
        }
    }
    let check = |x: i64| -> bool { x < 0 || !covered.contains(&x) || last_vals.contains_key(&x) };
    for ins in &old_ins[region_end..] {
        if !ins.a.iter().all(|&x| check(x)) {
            return None;
        }
    }
    for k in 0..f.blocks.len() {
        let ob = f.blocks[k];
        for ph in &f.arena[ob].phis {
            if !ph.a.iter().all(|&x| check(x)) {
                return None;
            }
        }
        if ob != b {
            for ins in &f.arena[ob].ins {
                if !ins.a.iter().all(|&x| check(x)) {
                    return None;
                }
            }
        }
    }
    // ---- new values ----
    let mut vals: Vec<Vec<i64>> = vec![vec![NOD; p]; g];
    for m in 0..g {
        for k in 0..p {
            if old_ins[s + k].has_d() {
                vals[m][k] = f.new_val();
            }
        }
    }
    let mut phi_of: BTreeMap<usize, i64> = BTreeMap::new();
    for &o in plan.inits.keys() {
        let v = f.new_val();
        phi_of.insert(o, v);
    }
    let mut iv_of: BTreeMap<u32, (i64, u32)> = BTreeMap::new(); // delta -> (phi, c0 of the reference slot)
    for row in &plan.slots {
        for sl in row {
            if let Slot::Lit(Lits::Affine { c0, delta, .. }) = sl {
                if !iv_of.contains_key(delta) {
                    let v = f.new_val();
                    iv_of.insert(*delta, (v, *c0));
                }
            }
        }
    }
    // ---- loop body ----
    let mut body: Vec<Instr> = Vec::new();
    for m in 0..g {
        for k in 0..p {
            let tmpl = &old_ins[s + k];
            let mut ops: Vec<i64> = Vec::with_capacity(tmpl.a.len());
            for sl in plan.slots[k].iter() {
                let x = match sl {
                    Slot::Lit(Lits::Const(c)) => lit(*c as i64),
                    Slot::Lit(Lits::Periodic(v)) => lit(v[m % v.len()] as i64),
                    Slot::Lit(Lits::Affine { c0, delta, mask, bias, .. }) => {
                        let (iv, r) = iv_of[delta];
                        let kk = c0.wrapping_sub(r).wrapping_add((m as u32).wrapping_mul(*delta));
                        let mut x = iv;
                        if kk != 0 {
                            let t = f.new_val();
                            body.push(Instr::new(Op::Add, t, vec![x, lit(kk as i64)], vec![]));
                            x = t;
                        }
                        if let Some(mk) = mask {
                            let t = f.new_val();
                            body.push(Instr::new(Op::And, t, vec![x, lit(*mk as i64)], vec![]));
                            x = t;
                            if *bias != 0 {
                                let t = f.new_val();
                                body.push(Instr::new(Op::Add, t, vec![x, lit(*bias as i64)], vec![]));
                                x = t;
                            }
                        }
                        x
                    }
                    Slot::Ext(x) => *x,
                    Slot::Same(o) => vals[m][*o],
                    Slot::Prev(o) => {
                        if m > 0 {
                            vals[m - 1][*o]
                        } else {
                            phi_of[o]
                        }
                    }
                };
                ops.push(x);
            }
            body.push(Instr { op: tmpl.op, d: vals[m][k], a: ops, t: vec![], pv: 0 });
        }
    }
    // ---- loop control ----
    let lb = f.new_block("roll");
    let eb = f.new_block("rolx");
    let mut phis: Vec<Instr> = Vec::new();
    for (&o, &pv) in &phi_of {
        phis.push(Instr::new(Op::Phi, pv, vec![plan.inits[&o], vals[g - 1][o]], vec![]));
    }
    let mut test_iv: Option<(i64, u32)> = None; // (next value, end value)
    for (&delta, &(iv, c0)) in &iv_of {
        let step = (g as u32).wrapping_mul(delta);
        let nx = f.new_val();
        body.push(Instr::new(Op::Add, nx, vec![iv, lit(step as i64)], vec![]));
        phis.push(Instr::new(Op::Phi, iv, vec![lit(c0 as i64), nx], vec![]));
        let sd = step as i32 as i64;
        // the induction variable reaches `end` exactly once, at the last iteration
        if test_iv.is_none() && sd != 0 && (sd.unsigned_abs() as u128) * (t_iters as u128) < (1u128 << 31) {
            test_iv = Some((nx, c0.wrapping_add((t_iters as u32).wrapping_mul(step))));
        }
    }
    if let Some((nx, end)) = test_iv {
        let c = f.new_val();
        body.push(Instr::new(Op::Eq, c, vec![nx, lit(end as i64)], vec![]));
        body.push(Instr::no_d(Op::Br, vec![c], vec![eb, lb]));
    } else {
        let cnt = f.new_val();
        let nx = f.new_val();
        phis.push(Instr::new(Op::Phi, cnt, vec![lit(t_iters as i64), nx], vec![]));
        body.push(Instr::new(Op::Sub, nx, vec![cnt, lit(1)], vec![]));
        body.push(Instr::no_d(Op::Br, vec![nx], vec![lb, eb]));
    }
    // ---- commit ----
    let exit_map: HashMap<i64, i64> = last_vals.iter().map(|(&old, &(m, k))| (old, vals[m][k])).collect();
    let remap = |x: &mut i64| {
        if *x >= 0 {
            if let Some(&n) = exit_map.get(x) {
                *x = n;
            }
        }
    };
    for k in 0..f.blocks.len() {
        let ob = f.blocks[k];
        for ph in f.arena[ob].phis.iter_mut() {
            ph.a.iter_mut().for_each(&remap);
        }
        if ob != b {
            for ins in f.arena[ob].ins.iter_mut() {
                ins.a.iter_mut().for_each(&remap);
            }
        }
    }
    let mut tail: Vec<Instr> = old_ins[region_end..].to_vec();
    for ins in tail.iter_mut() {
        ins.a.iter_mut().for_each(&remap);
    }
    let succs: Vec<usize> = tail.last().map(|t| t.t.clone()).unwrap_or_default();
    f.arena[b].ins.truncate(s);
    f.arena[b].ins.push(Instr::no_d(Op::Jmp, vec![], vec![lb]));
    f.arena[lb].phis = phis;
    f.arena[lb].ins = body;
    f.arena[lb].preds = vec![b, lb];
    f.arena[eb].ins = tail;
    f.arena[eb].preds = vec![lb];
    for sb in succs {
        for pr in f.arena[sb].preds.iter_mut() {
            if *pr == b {
                *pr = eb;
            }
        }
    }
    let pos = f.blocks.iter().position(|&x| x == b).unwrap();
    f.blocks.insert(pos + 1, lb);
    f.blocks.insert(pos + 2, eb);
    Some((b, eb))
}

/// Roll repeated stretches of straight-line code into loops. `budget` = the largest loop body
/// wanted (instructions). Returns the number of regions rolled.
pub fn reroll(f: &mut Func, budget: usize) -> usize {
    let mut work: Vec<usize> = f.blocks.clone();
    work.reverse();
    let mut rolled = 0;
    let mut tries = 0;
    while let Some(b) = work.pop() {
        if rolled >= 200 || tries >= 2000 {
            break;
        }
        let l = f.arena[b].ins.len().saturating_sub(1);
        if l < 12 {
            continue;
        }
        tries += 1;
        let bi = block_info(&f.arena[b].ins[..l]);
        for (s, p, n) in find_candidates(&bi, l) {
            let plan = match verify(&f.arena[b].ins[..l], &bi, s, p, n) {
                Some(pl) => pl,
                None => continue,
            };
            // literals with a period become plain literals when the body still fits the budget
            let per = as_periodic(&plan);
            let (plan, g) = match choose_g(&per, budget) {
                Some(g) if body_size(&per, g) <= budget => (per, g),
                _ => match choose_g(&plan, budget) {
                    Some(g) => (plan, g),
                    None => continue,
                },
            };
            if plan.n / g < 2 {
                continue;
            }
            if let Some((pre, post)) = apply(f, b, &plan, g) {
                rolled += 1;
                work.push(post);
                work.push(pre);
                break;
            }
        }
    }
    if rolled > 0 {
        analyze(f);
    }
    rolled
}
