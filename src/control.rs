//! control.rs - control-flow transformations that trade code size for fewer executed
//! compares and taken jumps. All of them are pure code copying on non-SSA code: every new
//! block is a faithful copy of an old one and every new edge corresponds to an old edge, so
//! behaviour is the same whatever the analysis believes. The analysis only decides what to
//! copy; the usual SSA passes then fold what the copies make constant.
use crate::ir::*;
use crate::kir::*;
use std::collections::HashMap;

const U: i64 = -1; // "value of the state register unknown"

fn defs_of(ins: &Instr, r: i64) -> bool {
    ins.has_d() && ins.d == r
}

fn uses_of(ins: &Instr, r: i64) -> bool {
    ins.a.iter().any(|&x| x == r)
}

/// Pick up to `n` candidate state registers: written with at least two different constants and
/// compared with constants (`eq r, K`) or branched on, at least twice. Best first.
pub fn state_registers(f: &Func, n: usize) -> Vec<i64> {
    let mut consts: HashMap<i64, Vec<i64>> = HashMap::new();
    let mut tests: HashMap<i64, usize> = HashMap::new();
    for &b in &f.blocks {
        for ins in &f.arena[b].ins {
            if ins.op == Op::Const && ins.has_d() {
                let v = !ins.a[0];
                let e = consts.entry(ins.d).or_default();
                if !e.contains(&v) {
                    e.push(v);
                }
            }
            if ins.op == Op::Eq && ins.a.len() == 2 {
                for k in 0..2 {
                    if ins.a[k] >= 0 && ins.a[1 - k] < 0 {
                        *tests.entry(ins.a[k]).or_insert(0) += 1;
                    }
                }
            }
        }
    }
    let mut v: Vec<(usize, i64)> = Vec::new();
    for (r, cs) in &consts {
        let t = tests.get(r).copied().unwrap_or(0);
        if cs.len() >= 2 && t >= 2 {
            v.push((cs.len() * t, *r));
        }
    }
    v.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    v.into_iter().take(n).map(|x| x.1).collect()
}

/// Split the code by the known constant value of register `r` ("DFA jump threading"): a copy of
/// each block for every constant value `r` can be known to have when control arrives. Where a
/// state is set to a constant and the dispatch on it is reached on every path, the copy for that
/// constant no longer needs the dispatch (the SSA passes fold it) and jumps straight to the
/// right code. Value tracking stops (back to "unknown") where `r` is dead. Returns false and
/// changes nothing if more than `cap` instructions would be added.
pub fn split_by_state(f: &mut Func, r: i64, cap: usize) -> bool {
    compute_preds(f);
    let nb = f.arena.len();
    // liveness of r (backward, one register)
    let mut live_in = vec![false; nb];
    let mut changed = true;
    let first_use = |f: &Func, b: usize| -> (bool, bool) {
        // (used before any def, defined)
        for ins in &f.arena[b].ins {
            if uses_of(ins, r) {
                return (true, true);
            }
            if defs_of(ins, r) {
                return (false, true);
            }
        }
        (false, false)
    };
    while changed {
        changed = false;
        for &b in f.blocks.iter().rev() {
            let (u, d) = first_use(f, b);
            let mut l = u;
            if !d {
                for &s in f.arena[b].succs() {
                    if live_in[s] {
                        l = true;
                    }
                }
            }
            if l && !live_in[b] {
                live_in[b] = true;
                changed = true;
            }
        }
    }
    // exit context of block b entered with context c
    let exit_ctx = |f: &Func, b: usize, c: i64| -> i64 {
        let mut cur = c;
        for ins in &f.arena[b].ins {
            if defs_of(ins, r) {
                cur = if ins.op == Op::Const { !ins.a[0] } else { U };
            }
        }
        cur
    };
    let entry = f.blocks[0];
    let mut ids: HashMap<(usize, i64), usize> = HashMap::new(); // (block, ctx) -> index in `pairs`
    let mut pairs: Vec<(usize, i64)> = Vec::new();
    ids.insert((entry, U), 0);
    pairs.push((entry, U));
    let mut per_block: HashMap<usize, usize> = HashMap::new();
    per_block.insert(entry, 1);
    let mut size = 0usize;
    let mut i = 0;
    while i < pairs.len() {
        let (b, c) = pairs[i];
        i += 1;
        let ec = exit_ctx(f, b, c);
        for &s in f.arena[b].succs() {
            let sc = if live_in[s] { ec } else { U };
            if !ids.contains_key(&(s, sc)) {
                ids.insert((s, sc), pairs.len());
                pairs.push((s, sc));
                // the first context of a block reuses the block itself: only later ones cost
                let cn = per_block.entry(s).or_insert(0);
                *cn += 1;
                if *cn > 1 {
                    size += f.arena[s].ins.len();
                    if size > cap {
                        return false;
                    }
                }
            }
        }
    }
    if size == 0 {
        return false;
    }
    // build: first context of each block keeps the block id
    let mut owner: HashMap<usize, bool> = HashMap::new();
    let mut target: Vec<usize> = Vec::with_capacity(pairs.len());
    let mut fresh: Vec<usize> = Vec::new();
    for &(b, c) in &pairs {
        if owner.insert(b, true).is_none() {
            target.push(b);
        } else {
            let nbid = f.new_block(&format!("{}_s{}", f.arena[b].name, if c == U { 0 } else { c + 1 }));
            f.arena[nbid].ins = f.arena[b].ins.clone();
            target.push(nbid);
            fresh.push(nbid);
        }
    }
    let orig_t: HashMap<usize, Vec<usize>> = owner.keys().map(|&b| (b, f.arena[b].ins.last().unwrap().t.clone())).collect();
    for (pi, &(b, c)) in pairs.iter().enumerate() {
        let ec = exit_ctx(f, b, c);
        let nid = target[pi];
        for k in 0..orig_t[&b].len() {
            let s = orig_t[&b][k]; // original successor
            let sc = if live_in[s] { ec } else { U };
            let nt = target[ids[&(s, sc)]];
            f.arena[nid].ins.last_mut().unwrap().t[k] = nt;
        }
    }
    f.blocks.extend(fresh);
    compute_preds(f);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_a_two_state_machine() {
        let src = "func f(v0)\nentry:\n  const v1, 0\n  const v2, 0\n  jmp h\nh:\n  lt v3, v2, v0\n  br v3, b, x\nb:\n  eq v4, v1, 0\n  br v4, s0, s1\ns0:\n  const v1, 1\n  jmp n\ns1:\n  eq v5, v1, 1\n  br v5, t1, t0\nt1:\n  const v1, 0\n  jmp n\nt0:\n  const v1, 1\n  jmp n\nn:\n  add v2, v2, 1\n  jmp h\nx:\n  ret v1\n";
        let prog = parse_program(src).unwrap();
        let mut f = from_program(&prog);
        assert_eq!(state_registers(&f, 2), vec![1]);
        let before = f.blocks.len();
        assert!(split_by_state(&mut f, 1, 1000));
        assert!(f.blocks.len() > before);
        let text = to_text(&f);
        let p2 = parse_program(&text).unwrap();
        let (c1, c2) = (compile_program(&prog), compile_program(&p2));
        for n in 0..20u32 {
            let inp = Inputs { args: vec![n], segments: vec![], vin: Default::default() };
            let a = execute(&c1, &inp, 10000, false);
            let b = execute(&c2, &inp, 10000, false);
            assert!(!differs(&a, &b));
        }
    }
}
