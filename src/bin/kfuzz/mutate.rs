//! mutate.rs - the "judge simulator": mutate a program the way the rules say the judges do
//! (rename registers, shuffle blocks keeping the entry first, change numbers) plus EMI-style
//! mutations (Le, Afshari, Su, PLDI 2014): put code on paths that never run, delete code on
//! paths that never ran.
use crate::kir::*;
use crate::tp::*;

fn regs_used(tp: &TProg) -> Vec<i64> {
    let mut set = std::collections::BTreeSet::new();
    for p in &tp.params {
        if let Some(r) = parse_reg(p) {
            set.insert(r);
        }
    }
    for b in &tp.blocks {
        for i in &b.ins {
            for a in &i.args {
                if let Some(r) = parse_reg(a) {
                    set.insert(r);
                }
            }
        }
    }
    set.into_iter().collect()
}

pub fn rename_regs(tp: &mut TProg, rng: &mut Rng) {
    let used = regs_used(tp);
    let n = used.len() as u64;
    let mut targets: Vec<i64> = match rng.below(3) {
        0 => (0..n as i64).collect(),                                        // dense
        1 => (0..n as i64 * 3 + 5).collect(),                                // a bit sparse
        _ => (0..n as i64 + 20).map(|k| k * 997 % 999_000 + (k % 7)).collect(), // spread out, may repeat
    };
    targets.sort();
    targets.dedup();
    let mut extra = 1_000_000 - 1;
    while (targets.len() as u64) < n {
        targets.push(extra);
        extra -= 1;
    }
    // shuffle and take
    for i in (1..targets.len()).rev() {
        let j = rng.below(i as u64 + 1) as usize;
        targets.swap(i, j);
    }
    let map: std::collections::HashMap<i64, i64> = used.iter().cloned().zip(targets.into_iter()).collect();
    let conv = |s: &String| -> String {
        match parse_reg(s) {
            Some(r) => format!("v{}", map[&r]),
            None => s.clone(),
        }
    };
    tp.params = tp.params.iter().map(conv).collect();
    for b in tp.blocks.iter_mut() {
        for i in b.ins.iter_mut() {
            i.args = i.args.iter().map(conv).collect();
        }
    }
}

pub fn shuffle_blocks(tp: &mut TProg, rng: &mut Rng) {
    let n = tp.blocks.len();
    for i in (2..n).rev() {
        let j = 1 + rng.below(i as u64) as usize;
        tp.blocks.swap(i, j);
    }
}

pub fn rename_labels(tp: &mut TProg, rng: &mut Rng) {
    let mut map = std::collections::HashMap::new();
    for (k, b) in tp.blocks.iter().enumerate() {
        let new = match rng.below(3) {
            0 => format!("L{}", k * 7 + rng.below(5) as usize),
            1 => format!("blk_{}_{}", k, rng.below(1000)),
            _ => format!("b{}x", k),
        };
        map.insert(b.label.clone(), new);
    }
    // make unique
    let mut seen = std::collections::HashSet::new();
    let keys: Vec<String> = map.keys().cloned().collect();
    for k in keys {
        let mut v = map[&k].clone();
        while !seen.insert(v.clone()) {
            v.push('_');
        }
        map.insert(k, v);
    }
    for b in tp.blocks.iter_mut() {
        b.label = map[&b.label].clone();
        for i in b.ins.iter_mut() {
            let sig = i.sig().as_bytes();
            for (k, c) in sig.iter().enumerate() {
                if *c == b'L' {
                    i.args[k] = map[&i.args[k]].clone();
                }
            }
        }
    }
}

fn new_const(v: u32, rng: &mut Rng) -> u32 {
    match rng.below(8) {
        0 => v.wrapping_add(1 + rng.below(3) as u32),
        1 => v.wrapping_sub(1 + rng.below(3) as u32),
        2 => v.wrapping_mul(2),
        3 => v / 2,
        4 => {
            if v == 0 {
                rng.below(10) as u32
            } else {
                let bits = 32 - v.leading_zeros();
                (rng.next() as u32) >> (32 - bits.min(32)).min(31)
            }
        }
        5 => rng.below(70) as u32,
        6 => v ^ (1 << rng.below(32)),
        _ => v.wrapping_add(rng.below(100) as u32),
    }
}

pub fn change_consts(tp: &mut TProg, rng: &mut Rng, prob: u64) {
    for b in tp.blocks.iter_mut() {
        for i in b.ins.iter_mut() {
            let sig = i.sig().as_bytes().to_vec();
            for (k, c) in sig.iter().enumerate() {
                if (*c == b's' || *c == b'i') && parse_reg(&i.args[k]).is_none() && rng.below(100) < prob {
                    if let Ok(v) = parse_literal_raw(&i.args[k]) {
                        let nv = new_const((v & MASK) as u32, rng);
                        i.args[k] = format!("{}", nv);
                    }
                }
            }
        }
    }
}

fn max_reg(tp: &TProg) -> i64 {
    regs_used(tp).into_iter().max().unwrap_or(0)
}

fn junk(tp: &TProg, rng: &mut Rng, n: usize, regs: &[i64]) -> Vec<TIns> {
    let _ = tp;
    let r = |rng: &mut Rng| -> String { format!("v{}", regs[rng.below(regs.len() as u64) as usize]) };
    let s = |rng: &mut Rng| -> String {
        if rng.below(3) == 0 {
            format!("{}", rng.below(300))
        } else {
            format!("v{}", regs[rng.below(regs.len() as u64) as usize])
        }
    };
    let mut out = Vec::new();
    for _ in 0..n {
        let k = rng.below(12);
        let ops = ["add", "sub", "mul", "and", "or", "xor", "shl", "shr", "lt", "eq"];
        let ins = match k {
            0..=4 => TIns { op: ops[rng.below(10) as usize].into(), args: vec![r(rng), s(rng), s(rng)] },
            5 => TIns { op: "div".into(), args: vec![r(rng), s(rng), s(rng)] },
            6 => TIns { op: "mod".into(), args: vec![r(rng), s(rng), s(rng)] },
            7 => TIns { op: "load".into(), args: vec![r(rng), s(rng)] },
            8 => TIns { op: "store".into(), args: vec![s(rng), s(rng)] },
            9 => TIns { op: "vstore".into(), args: vec![format!("{}", rng.below(4)), s(rng)] },
            10 => TIns { op: "vload".into(), args: vec![r(rng), format!("{}", rng.below(4))] },
            _ => TIns { op: "mov".into(), args: vec![r(rng), s(rng)] },
        };
        out.push(ins);
    }
    out
}

/// Opaque predicate guard: x*(x+1) is always even, so `br (x*(x+1))&1` never jumps. The junk
/// block never runs, but a simple analysis cannot tell. Semantics are preserved exactly.
/// Returns the number of guards inserted.
pub fn emi_opaque(tp: &mut TProg, rng: &mut Rng, count: usize) -> usize {
    let regs = regs_used(tp);
    if regs.is_empty() {
        return 0;
    }
    let mut next = max_reg(tp) + 1;
    let mut done = 0;
    for _ in 0..count {
        let bi = rng.below(tp.blocks.len() as u64) as usize;
        let len = tp.blocks[bi].ins.len();
        if len < 2 {
            continue;
        }
        let at = rng.below(len as u64 - 1) as usize; // split before ins[at], not before the terminator
        let x = format!("v{}", regs[rng.below(regs.len() as u64) as usize]);
        let (t, u, w) = (format!("v{}", next), format!("v{}", next + 1), format!("v{}", next + 2));
        next += 3;
        let junk_l = format!("emi_j{}", rng.below(1 << 30));
        let cont_l = format!("emi_c{}", rng.below(1 << 30));
        if tp.blocks.iter().any(|b| b.label == junk_l || b.label == cont_l) {
            continue;
        }
        let rest: Vec<TIns> = tp.blocks[bi].ins.split_off(at);
        let mut jr = regs.clone();
        jr.push(next - 1);
        let jn = 1 + rng.below(6) as usize;
        let mut jins = junk(tp, rng, jn, &jr);
        match rng.below(3) {
            0 => jins.push(TIns::new("jmp", &[&cont_l])),
            1 => jins.push(TIns { op: "trap".into(), args: vec![format!("{}", rng.below(50))] }),
            _ => jins.push(TIns { op: "ret".into(), args: vec![x.clone()] }),
        }
        {
            let b = &mut tp.blocks[bi];
            b.ins.push(TIns { op: "add".into(), args: vec![t.clone(), x.clone(), "1".into()] });
            b.ins.push(TIns { op: "mul".into(), args: vec![u.clone(), x.clone(), t.clone()] });
            b.ins.push(TIns { op: "and".into(), args: vec![w.clone(), u.clone(), "1".into()] });
            b.ins.push(TIns { op: "br".into(), args: vec![w.clone(), junk_l.clone(), cont_l.clone()] });
        }
        tp.blocks.insert(bi + 1, TBlock { label: junk_l, ins: jins });
        tp.blocks.insert(bi + 2, TBlock { label: cont_l, ins: rest });
        done += 1;
    }
    done
}

/// Blocks never executed on the given inputs.
pub fn dead_blocks(tp: &TProg, inputs: &[Inputs]) -> Vec<usize> {
    let text = tp.to_text();
    let prog = match parse_program(&text) {
        Ok(p) => p,
        Err(_) => return vec![],
    };
    let cp = compile_program(&prog);
    let mut seen = vec![false; tp.blocks.len()];
    for inp in inputs {
        let r = execute(&cp, inp, 50_000, true);
        for &pc in &r.trace {
            seen[prog.block_of[pc as usize]] = true;
        }
    }
    (0..seen.len()).filter(|&i| !seen[i]).collect()
}

/// EMI "insert": random code into blocks that did not run on the sample inputs.
pub fn emi_insert_dead(tp: &mut TProg, rng: &mut Rng, inputs: &[Inputs]) -> usize {
    let dead = dead_blocks(tp, inputs);
    let regs = regs_used(tp);
    if regs.is_empty() {
        return 0;
    }
    let mut n = 0;
    for bi in dead {
        if rng.below(3) == 0 {
            continue;
        }
        let k = 1 + rng.below(5) as usize;
        let j = junk(tp, rng, k, &regs);
        let b = &mut tp.blocks[bi];
        let at = b.ins.len() - 1;
        for (o, ins) in j.into_iter().enumerate() {
            b.ins.insert(at + o, ins);
        }
        n += 1;
    }
    n
}

/// EMI "delete": remove instructions in dead blocks, fold a branch that never went one way.
pub fn emi_delete_dead(tp: &mut TProg, rng: &mut Rng, inputs: &[Inputs]) -> usize {
    let dead = dead_blocks(tp, inputs);
    let mut n = 0;
    for &bi in &dead {
        let b = &mut tp.blocks[bi];
        let mut k = 0;
        while k + 1 < b.ins.len() {
            if rng.below(2) == 0 {
                b.ins.remove(k);
                n += 1;
            } else {
                k += 1;
            }
        }
    }
    // branches with a dead side become jumps
    let labels: Vec<String> = tp.blocks.iter().map(|b| b.label.clone()).collect();
    let deadset: std::collections::HashSet<String> = dead.iter().map(|&i| labels[i].clone()).collect();
    for b in tp.blocks.iter_mut() {
        let last = b.ins.len() - 1;
        if b.ins[last].op == "br" {
            let (t, f) = (b.ins[last].args[1].clone(), b.ins[last].args[2].clone());
            if deadset.contains(&f) && !deadset.contains(&t) && rng.below(2) == 0 {
                b.ins[last] = TIns { op: "jmp".into(), args: vec![t] };
                n += 1;
            } else if deadset.contains(&t) && !deadset.contains(&f) && rng.below(2) == 0 {
                b.ins[last] = TIns { op: "jmp".into(), args: vec![f] };
                n += 1;
            }
        }
    }
    n
}

pub const MUT_NAMES: [&str; 6] = ["rename", "shuffle", "consts", "emi-opaque", "emi-insert", "emi-delete"];

/// Apply one random mutation; returns (name, preserves_semantics_exactly).
pub fn mutate_once(tp: &mut TProg, rng: &mut Rng, inputs: &[Inputs]) -> (&'static str, bool) {
    match rng.below(7) {
        0 => {
            rename_regs(tp, rng);
            ("rename", true)
        }
        1 => {
            shuffle_blocks(tp, rng);
            rename_labels(tp, rng);
            ("shuffle", true)
        }
        2 => {
            change_consts(tp, rng, 25);
            ("consts", false)
        }
        3 | 4 => {
            let c = 1 + rng.below(3) as usize;
            emi_opaque(tp, rng, c);
            ("emi-opaque", true)
        }
        5 => {
            emi_insert_dead(tp, rng, inputs);
            ("emi-insert", false)
        }
        _ => {
            emi_delete_dead(tp, rng, inputs);
            ("emi-delete", false)
        }
    }
}
