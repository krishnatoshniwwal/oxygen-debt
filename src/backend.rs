//! backend.rs - work on the final (non-SSA) code: clean-up, block order, duplication.
use crate::ir::*;
use crate::kir::*;
use std::collections::HashMap;

pub type Profile = HashMap<(String, String), i128>;

/// Jump threading, block merging and unreachable-block removal on non-SSA code.
pub fn final_cleanup(f: &mut Func) {
    let mut changed = true;
    while changed {
        changed = false;
        // normalise branches
        for bi in 0..f.blocks.len() {
            let b = f.blocks[bi];
            let term = f.arena[b].ins.last_mut().unwrap();
            if term.op == Op::Br {
                if term.t[0] == term.t[1] {
                    term.op = Op::Jmp;
                    term.a = vec![];
                    term.t = vec![term.t[0]];
                    changed = true;
                } else if term.a[0] < 0 {
                    let tk = if !term.a[0] != 0 { term.t[0] } else { term.t[1] };
                    term.op = Op::Jmp;
                    term.a = vec![];
                    term.t = vec![tk];
                    changed = true;
                }
            }
        }
        // forward jumps to blocks that only contain a jump
        let mut fwd: HashMap<usize, usize> = HashMap::new();
        for &b in &f.blocks {
            let blk = &f.arena[b];
            if blk.ins.len() == 1 && blk.ins[0].op == Op::Jmp && blk.ins[0].t[0] != b {
                fwd.insert(b, blk.ins[0].t[0]);
            }
        }
        let fin = |mut t: usize| -> usize {
            let mut n = 0;
            while let Some(&nt) = fwd.get(&t) {
                if n >= 1000 {
                    break;
                }
                t = nt;
                n += 1;
            }
            t
        };
        if !fwd.is_empty() {
            for bi in 0..f.blocks.len() {
                let b = f.blocks[bi];
                let tt = &mut f.arena[b].ins.last_mut().unwrap().t;
                for t in tt.iter_mut() {
                    let nt = fin(*t);
                    if nt != *t {
                        *t = nt;
                        changed = true;
                    }
                }
            }
            let e = f.blocks[0];
            if fwd.contains_key(&e) {
                let ne = fin(e);
                if ne != e && !fwd.contains_key(&ne) {
                    let pos = f.blocks.iter().position(|&x| x == ne).unwrap();
                    f.blocks.remove(pos);
                    f.blocks.insert(0, ne);
                    changed = true;
                }
            }
        }
        // unreachable blocks
        let entry = f.blocks[0];
        let mut seen = vec![false; f.arena.len()];
        seen[entry] = true;
        let mut count = 1;
        let mut stack = vec![entry];
        while let Some(b) = stack.pop() {
            for k in 0..f.arena[b].succs().len() {
                let s = f.arena[b].succs()[k];
                if !seen[s] {
                    seen[s] = true;
                    count += 1;
                    stack.push(s);
                }
            }
        }
        if count != f.blocks.len() {
            f.blocks.retain(|&b| seen[b]);
            changed = true;
        }
        compute_preds(f);
        // merge straight-line pairs
        let mut dead = vec![false; f.arena.len()];
        let mut any_dead = false;
        for bi in 0..f.blocks.len() {
            let b = f.blocks[bi];
            if dead[b] {
                continue;
            }
            loop {
                let term = f.arena[b].ins.last().unwrap();
                if term.op != Op::Jmp {
                    break;
                }
                let s = term.t[0];
                if s == b || s == entry || f.arena[s].preds.len() != 1 || dead[s] {
                    break;
                }
                let sins = f.arena[s].ins.clone();
                f.arena[b].ins.pop();
                f.arena[b].ins.extend(sins.iter().cloned());
                dead[s] = true;
                any_dead = true;
                for &ss in sins.last().unwrap().t.iter() {
                    for i in 0..f.arena[ss].preds.len() {
                        if f.arena[ss].preds[i] == s {
                            f.arena[ss].preds[i] = b;
                        }
                    }
                }
                changed = true;
            }
        }
        if any_dead {
            f.blocks.retain(|&b| !dead[b]);
            compute_preds(f);
        }
    }
}

/// Give every block a frequency estimate: from the profile when it names the block,
/// otherwise from the loop depth.
pub fn estimate_freq(f: &mut Func, profile: Option<&Profile>) {
    analyze(f);
    for &b in &f.blocks.clone() {
        let d = f.arena[b].depth as u32;
        f.arena[b].freq = (8f64.powi(d as i32)).min(1e9);
    }
    if let Some(prof) = profile {
        let byname: HashMap<String, usize> = f.blocks.iter().map(|&b| (f.arena[b].name.clone(), b)).collect();
        let mut cnt: HashMap<&str, i128> = HashMap::new();
        for ((_, dst), c) in prof.iter() {
            if byname.contains_key(dst) {
                *cnt.entry(dst.as_str()).or_insert(0) += *c;
            }
        }
        if !cnt.is_empty() {
            let entry = f.blocks[0];
            for (name, c) in cnt.iter() {
                let b = byname[*name];
                // mix with the static estimate so that a wrong profile cannot zero a hot block
                f.arena[b].freq = 0.5 * f.arena[b].freq + *c as f64;
            }
            let ename = f.arena[entry].name.clone();
            if !cnt.contains_key(ename.as_str()) {
                f.arena[entry].freq = f.arena[entry].freq.max(1.0);
            }
        }
    }
}

/// List of (weight, order, src, dst) for all CFG edges.
pub fn edge_weights(f: &Func, profile: Option<&Profile>) -> Vec<(f64, usize, usize, usize)> {
    let mut edges = Vec::new();
    let mut n = 0;
    for &b in &f.blocks {
        let ss = f.arena[b].succs();
        if ss.is_empty() {
            continue;
        }
        let probs: Vec<f64> = if ss.len() == 1 {
            vec![1.0]
        } else {
            // an edge that stays in the loop is likelier than one that leaves it
            let lp = f.arena[b].lp;
            let in0 = lp.map_or(false, |l| f.loops[l].ids.contains(&ss[0]));
            let in1 = lp.map_or(false, |l| f.loops[l].ids.contains(&ss[1]));
            if in0 && !in1 {
                vec![0.9, 0.1]
            } else if in1 && !in0 {
                vec![0.1, 0.9]
            } else {
                vec![0.5, 0.5]
            }
        };
        for (&s, p) in ss.iter().zip(probs) {
            let mut w = f.arena[b].freq * p;
            if let Some(prof) = profile {
                if let Some(&c) = prof.get(&(f.arena[b].name.clone(), f.arena[s].name.clone())) {
                    w = 0.5 * w + c as f64;
                }
            }
            edges.push((w, n, b, s));
            n += 1;
        }
    }
    edges
}

/// Order the blocks so that hot edges fall through (greedy chaining).
pub fn layout(f: &mut Func, profile: Option<&Profile>) {
    estimate_freq(f, profile);
    let entry = f.blocks[0];
    let mut edges = edge_weights(f, profile);
    edges.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap().then(a.1.cmp(&b.1)));
    let nb = f.arena.len();
    let mut chain_of: Vec<usize> = (0..nb).collect();
    let mut chains: Vec<Option<Vec<usize>>> = vec![None; nb];
    for &b in &f.blocks {
        chains[b] = Some(vec![b]);
    }
    for (_, _, s, d) in edges {
        if d == entry || s == d {
            continue;
        }
        let (cs, cd) = (chain_of[s], chain_of[d]);
        if cs == cd {
            continue;
        }
        if *chains[cs].as_ref().unwrap().last().unwrap() != s || chains[cd].as_ref().unwrap()[0] != d {
            continue;
        }
        let moved = chains[cd].take().unwrap();
        for &x in &moved {
            chain_of[x] = cs;
        }
        chains[cs].as_mut().unwrap().extend(moved);
    }
    let ec = chain_of[entry];
    let order_index: HashMap<usize, usize> = f.blocks.iter().enumerate().map(|(i, &b)| (b, i)).collect();
    let mut rest: Vec<usize> = (0..nb).filter(|&c| c != ec && chains[c].is_some()).collect();
    let key = |c: usize| -> (f64, usize) {
        let ch = chains[c].as_ref().unwrap();
        let mx = ch.iter().map(|&x| f.arena[x].freq).fold(f64::NEG_INFINITY, f64::max);
        (-mx, order_index[&ch[0]])
    };
    rest.sort_by(|&a, &b| {
        let (ka, kb) = (key(a), key(b));
        ka.0.partial_cmp(&kb.0).unwrap().then(ka.1.cmp(&kb.1))
    });
    let mut out: Vec<usize> = chains[ec].clone().unwrap();
    for c in rest {
        out.extend(chains[c].as_ref().unwrap().iter().copied());
    }
    f.blocks = out;
}

pub fn parse_profile(text: &str) -> Profile {
    let mut prof: Profile = HashMap::new();
    for raw in splitlines(text) {
        let line = raw.split('#').next().unwrap().trim();
        if line.is_empty() {
            continue;
        }
        let toks: Vec<&str> = line.split_whitespace().collect();
        if toks.len() != 4 || toks[0] != "edge" {
            continue;
        }
        let c: i128 = match toks[3].parse() {
            Ok(c) => c,
            Err(_) => continue,
        };
        if c < 0 {
            continue;
        }
        *prof.entry((toks[1].to_string(), toks[2].to_string())).or_insert(0) += c;
    }
    prof
}

/// Replace 'jmp S' by a copy of S when S is small (this also rotates loops: the loop test is
/// copied to the end of the loop body). Pure code copying, always legal. `budget` is the
/// number of extra instructions we are allowed to add.
pub fn tail_dup(f: &mut Func, maxdup: usize, mut budget: i64) -> bool {
    if maxdup == 0 {
        return false;
    }
    let mut changed = false;
    for _ in 0..3 {
        compute_preds(f);
        let mut any = false;
        for bi in 0..f.blocks.len() {
            let b = f.blocks[bi];
            let term = f.arena[b].term();
            if term.op != Op::Jmp {
                continue;
            }
            let s = term.t[0];
            if s == b {
                continue;
            }
            let n = f.arena[s].ins.len();
            if n > maxdup && n > 1 {
                continue;
            }
            let sl = f.arena[s].term();
            if sl.op == Op::Jmp && sl.t[0] == s {
                continue;
            }
            let extra = n as i64 - 1;
            if extra > budget {
                continue;
            }
            budget -= extra;
            let copy = f.arena[s].ins.clone();
            f.arena[b].ins.pop();
            f.arena[b].ins.extend(copy);
            any = true;
        }
        if !any {
            break;
        }
        changed = true;
        final_cleanup(f);
    }
    changed
}

/// Copy the body of every innermost loop `factor` times in a row. Every copy keeps its own
/// exit tests, so this is pure code copying: the back edge of copy i goes to copy i+1. It
/// saves taken-jump penalties. `budget` = extra instructions allowed.
pub fn unroll(f: &mut Func, factor: usize, mut budget: i64) -> bool {
    if factor <= 1 {
        return false;
    }
    compute_preds(f);
    analyze(f);
    let mut inner: Vec<usize> = Vec::new();
    for (li, _) in f.loops.iter().enumerate() {
        if f.loops.iter().any(|o| o.parent == Some(li)) {
            continue;
        }
        inner.push(li);
    }
    inner.sort_by_key(|&li| f.arena[f.loops[li].header].rpo);
    let mut changed = false;
    for li in inner {
        let lp_blocks = f.loops[li].blocks.clone();
        let lp_ids = f.loops[li].ids.clone();
        let h = f.loops[li].header;
        let size: i64 = lp_blocks.iter().map(|&b| f.arena[b].ins.len() as i64).sum();
        let mut k = factor;
        while k > 1 && size * (k as i64 - 1) > budget {
            k -= 1;
        }
        if k <= 1 {
            continue;
        }
        budget -= size * (k as i64 - 1);
        let mut copies: Vec<HashMap<usize, usize>> = vec![lp_blocks.iter().map(|&b| (b, b)).collect()];
        let mut newblocks: Vec<usize> = Vec::new();
        for j in 1..k {
            let mut m = HashMap::new();
            for &b in &lp_blocks {
                let name = format!("{}_u{}", f.arena[b].name, j);
                let nb = f.new_block(&name);
                f.arena[nb].ins = f.arena[b].ins.clone();
                m.insert(b, nb);
                newblocks.push(nb);
            }
            copies.push(m);
        }
        for j in 0..k {
            for &b in &lp_blocks {
                let cb = copies[j][&b];
                let ntt = f.arena[cb].ins.last().unwrap().t.len();
                for i in 0..ntt {
                    let t = f.arena[cb].ins.last().unwrap().t[i];
                    let nt = if t == h {
                        copies[(j + 1) % k][&h]
                    } else if lp_ids.contains(&t) {
                        copies[j][&t]
                    } else {
                        t
                    };
                    f.arena[cb].ins.last_mut().unwrap().t[i] = nt;
                }
            }
        }
        f.blocks.extend(newblocks);
        changed = true;
    }
    if changed {
        compute_preds(f);
    }
    changed
}
