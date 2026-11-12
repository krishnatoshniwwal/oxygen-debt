//! minimize.rs - shrink a failing program while a predicate keeps holding.
use crate::kir::*;
use crate::tp::*;
use std::time::Instant;

fn reachable(tp: &TProg) -> Vec<bool> {
    let idx: std::collections::HashMap<&str, usize> = tp.blocks.iter().enumerate().map(|(i, b)| (b.label.as_str(), i)).collect();
    let mut seen = vec![false; tp.blocks.len()];
    let mut st = vec![0usize];
    seen[0] = true;
    while let Some(b) = st.pop() {
        for i in &tp.blocks[b].ins {
            if let Some(o) = i.opc() {
                for (k, c) in o.sig().bytes().enumerate() {
                    if c == b'L' {
                        if let Some(&t) = idx.get(i.args[k].as_str()) {
                            if !seen[t] {
                                seen[t] = true;
                                st.push(t);
                            }
                        }
                    }
                }
            }
        }
    }
    seen
}

pub fn drop_unreachable(tp: &mut TProg) {
    let seen = reachable(tp);
    let mut k = 0;
    tp.blocks.retain(|_| {
        k += 1;
        seen[k - 1]
    });
}

pub fn minimize(text: &str, pred: &dyn Fn(&str) -> bool, secs: f64) -> String {
    let t0 = Instant::now();
    let mut cur = match TProg::parse(text) {
        Ok(t) => t,
        Err(_) => return text.to_string(),
    };
    let try_ = |cur: &mut TProg, mut cand: TProg| -> bool {
        drop_unreachable(&mut cand);
        let t = cand.to_text();
        if t == cur.to_text() || parse_program(&t).is_err() {
            return false;
        }
        if pred(&t) {
            *cur = cand;
            true
        } else {
            false
        }
    };
    loop {
        let mut progress = false;
        // 1. branches -> jumps
        let mut bi = 0;
        while bi < cur.blocks.len() {
            if t0.elapsed().as_secs_f64() > secs {
                return cur.to_text();
            }
            let last = cur.blocks[bi].ins.len() - 1;
            if cur.blocks[bi].ins[last].op == "br" {
                for side in [1, 2] {
                    let mut c = cur.clone();
                    let tgt = c.blocks[bi].ins[last].args[side].clone();
                    c.blocks[bi].ins[last] = TIns { op: "jmp".into(), args: vec![tgt] };
                    if try_(&mut cur, c) {
                        progress = true;
                        break;
                    }
                }
            }
            bi += 1;
        }
        // 2. delete instructions, chunks first
        for chunk in [8usize, 4, 2, 1] {
            let mut bi = 0;
            while bi < cur.blocks.len() {
                let mut k = 0;
                while k + 1 < cur.blocks[bi].ins.len() {
                    if t0.elapsed().as_secs_f64() > secs {
                        return cur.to_text();
                    }
                    let end = (k + chunk).min(cur.blocks[bi].ins.len() - 1);
                    if end > k {
                        let mut c = cur.clone();
                        c.blocks[bi].ins.drain(k..end);
                        if try_(&mut cur, c) {
                            progress = true;
                            continue;
                        }
                    }
                    k += chunk;
                }
                bi += 1;
            }
        }
        // 3. simplify operands
        for bi in 0..cur.blocks.len() {
            for ii in 0..cur.blocks[bi].ins.len() {
                if t0.elapsed().as_secs_f64() > secs {
                    return cur.to_text();
                }
                let sig = cur.blocks[bi].ins[ii].sig().as_bytes().to_vec();
                for (k, ch) in sig.iter().enumerate() {
                    if *ch == b's' || *ch == b'i' {
                        let old = cur.blocks[bi].ins[ii].args[k].clone();
                        for rep in ["0", "1"] {
                            if old == rep {
                                continue;
                            }
                            let mut c = cur.clone();
                            c.blocks[bi].ins[ii].args[k] = rep.to_string();
                            if try_(&mut cur, c) {
                                progress = true;
                                break;
                            }
                        }
                    }
                }
                // turn a computation into a constant
                let ins = &cur.blocks[bi].ins[ii];
                if let Some(o) = ins.opc() {
                    if ins.dest().is_some() && !matches!(o, Op::Const | Op::Mov) && !o.is_mem() {
                        let mut c = cur.clone();
                        let d = ins.args[0].clone();
                        c.blocks[bi].ins[ii] = TIns { op: "const".into(), args: vec![d, "0".into()] };
                        if try_(&mut cur, c) {
                            progress = true;
                        }
                    }
                }
            }
        }
        if !progress {
            break;
        }
    }
    cur.to_text()
}
