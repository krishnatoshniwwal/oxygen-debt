//! reassoc.rs - tree-height reduction of chains of add / mul / xor / and / or.
//!
//! These operations are associative and commutative on 32-bit values (add and mul modulo
//! 2^32), so a chain `((a + b) + c) + d` may be evaluated in any grouping. A chain is the
//! tree of same-operation instructions in one block whose inner results have no other use.
//! Its leaves are regrouped so that the values that become available last (phis, i.e.
//! loop-carried accumulators, and late definitions) are combined last: the two earliest
//! values are always combined first. Literals are folded into one. The chain root keeps its
//! value number; the old chain instructions are deleted and the new ones are inserted at the
//! root's place (every leaf is defined before the root), so no ordering is lost.
use crate::ir::*;
use crate::kir::*;
use crate::passes::{evalop, use_counts};
use std::collections::HashMap;

fn chain_op(op: Op) -> bool {
    matches!(op, Op::Add | Op::Mul | Op::Xor | Op::And | Op::Or)
}

pub fn reassoc(f: &mut Func) -> bool {
    let uc = use_counts(f);
    let mut changed = false;
    let blocks = f.blocks.clone();
    for &b in &blocks {
        let mut pos: HashMap<i64, usize> = HashMap::new();
        for (i, ins) in f.arena[b].ins.iter().enumerate() {
            if ins.has_d() {
                pos.insert(ins.d, i);
            }
        }
        let phis: Vec<i64> = f.arena[b].phis.iter().map(|p| p.d).collect();
        let n = f.arena[b].ins.len();
        // interior chain instructions: single use, used by a same-op instruction of this block
        let mut parent_same = vec![false; n];
        for i in 0..n {
            let ins = &f.arena[b].ins[i];
            if chain_op(ins.op) && ins.has_d() {
                for &x in &ins.a {
                    if x >= 0 && uc[x as usize] == 1 {
                        if let Some(&p) = pos.get(&x) {
                            if p < i && f.arena[b].ins[p].op == ins.op {
                                parent_same[p] = true;
                            }
                        }
                    }
                }
            }
        }
        let mut remove = vec![false; n];
        let mut insert_at: Vec<(usize, Vec<Instr>)> = Vec::new();
        for r in 0..n {
            let root = f.arena[b].ins[r].clone();
            if !chain_op(root.op) || !root.has_d() || parent_same[r] {
                continue;
            }
            let mut leaves: Vec<i64> = Vec::new();
            let mut inner: Vec<usize> = Vec::new();
            let mut stack = vec![r];
            while let Some(i) = stack.pop() {
                let ins = &f.arena[b].ins[i];
                for &x in &ins.a {
                    let mut descend = false;
                    if x >= 0 && uc[x as usize] == 1 {
                        if let Some(&p) = pos.get(&x) {
                            if p < i && f.arena[b].ins[p].op == root.op {
                                descend = true;
                                inner.push(p);
                                stack.push(p);
                            }
                        }
                    }
                    if !descend {
                        leaves.push(x);
                    }
                }
            }
            if leaves.len() < 4 {
                continue;
            }
            let mut acc: Option<i64> = None;
            let mut vars: Vec<i64> = Vec::new();
            for &x in &leaves {
                if x < 0 {
                    acc = Some(match acc {
                        None => !x,
                        Some(a) => evalop(root.op, a, !x).unwrap(),
                    });
                } else {
                    vars.push(x);
                }
            }
            let mut items: Vec<(i64, i64)> = vars
                .iter()
                .map(|&x| {
                    let ready = if phis.contains(&x) { 1 << 20 } else { pos.get(&x).map_or(0, |&p| p as i64 + 1) };
                    (ready, x)
                })
                .collect();
            if let Some(a) = acc {
                items.push((0, lit(a)));
            }
            if items.len() < 3 {
                continue;
            }
            let mut new_ins: Vec<Instr> = Vec::new();
            while items.len() > 1 {
                items.sort_by_key(|&(rd, x)| (rd, x));
                let (r1, x1) = items.remove(0);
                let (r2, x2) = items.remove(0);
                let d = if items.is_empty() { root.d } else { f.new_val() };
                new_ins.push(Instr::new(root.op, d, vec![x1, x2], vec![]));
                items.push((r1.max(r2) + 1, d));
            }
            for &p in &inner {
                remove[p] = true;
            }
            remove[r] = true;
            insert_at.push((r, new_ins));
            changed = true;
        }
        if !insert_at.is_empty() {
            insert_at.sort_by_key(|e| e.0);
            let old = std::mem::take(&mut f.arena[b].ins);
            let mut out = Vec::with_capacity(old.len() + 8);
            let mut it = insert_at.into_iter().peekable();
            for (i, ins) in old.into_iter().enumerate() {
                if it.peek().map_or(false, |e| e.0 == i) {
                    out.extend(it.next().unwrap().1);
                    continue;
                }
                if !remove[i] {
                    out.push(ins);
                }
            }
            f.arena[b].ins = out;
        }
    }
    changed
}
