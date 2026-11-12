//! sink.rs - sink pure instructions into the one branch arm that uses them.
//!
//! A pure instruction (no trap, no effect) in a block that ends in a two-way branch moves to
//! the start of a successor `s` when `s` has the block as its only predecessor, every use of
//! the value is in a block dominated by `s` (so every path to a use goes through `s`, and the
//! operands, defined in the block or above, still dominate `s`), no use is a phi, and `s` is
//! not in a deeper loop than the block. The path that does not use the value then no longer
//! pays for it. Repeated a few times so that chains sink together.
use crate::ir::*;

pub fn sink(f: &mut Func) -> bool {
    let mut any = false;
    for _ in 0..6 {
        ir::analyze(f);
        let nv = f.nval.max(0) as usize + 1;
        // blocks using each value; u32::MAX marks a phi use
        let mut uses: Vec<Vec<usize>> = vec![Vec::new(); nv];
        let mut phi_use = vec![false; nv];
        for &b in &f.blocks {
            for phi in &f.arena[b].phis {
                for &x in &phi.a {
                    if x >= 0 {
                        phi_use[x as usize] = true;
                    }
                }
            }
            for ins in &f.arena[b].ins {
                for &x in &ins.a {
                    if x >= 0 && uses[x as usize].last() != Some(&b) {
                        uses[x as usize].push(b);
                    }
                }
            }
        }
        let mut changed = false;
        let blocks = f.blocks.clone();
        for &b in blocks.iter().rev() {
            let term = f.arena[b].term().clone();
            if term.op != Op::Br || term.t[0] == term.t[1] {
                continue;
            }
            let n = f.arena[b].ins.len();
            let mut moved: Vec<(usize, usize)> = Vec::new(); // (index, target)
            for i in (0..n - 1).rev() {
                let ins = &f.arena[b].ins[i];
                if !ins.has_d() || !is_pure(ins.op) {
                    continue;
                }
                let d = ins.d as usize;
                if phi_use[d] || uses[d].is_empty() {
                    continue;
                }
                for &s in &term.t {
                    if f.arena[s].preds.len() == 1
                        && f.arena[s].depth <= f.arena[b].depth
                        && uses[d].iter().all(|&u| f.dominates(s, u))
                    {
                        moved.push((i, s));
                        break;
                    }
                }
            }
            if moved.is_empty() {
                continue;
            }
            // moved is in decreasing index order; push each to the front of its target, so the
            // original relative order is kept for instructions sent to the same block
            let mut taken: Vec<Option<Instr>> = Vec::new();
            for &(i, _) in &moved {
                taken.push(Some(f.arena[b].ins[i].clone()));
            }
            for (k, &(_, s)) in moved.iter().enumerate() {
                let ins = taken[k].take().unwrap();
                // its operands' uses now are in s
                for &x in &ins.a {
                    if x >= 0 && uses[x as usize].last() != Some(&s) {
                        uses[x as usize].push(s);
                    }
                }
                f.arena[s].ins.insert(0, ins);
            }
            let dead: Vec<usize> = moved.iter().map(|m| m.0).collect();
            let mut k = 0;
            f.arena[b].ins.retain(|_| {
                let keep = !dead.contains(&k);
                k += 1;
                keep
            });
            changed = true;
        }
        if !changed {
            break;
        }
        any = true;
    }
    any
}
