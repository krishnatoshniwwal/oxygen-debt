//! kern.rs - dataflow idioms in SSA form that algebra.rs does not cover:
//!
//! * the Kernighan loop `while (y != 0) { y = y & (y - 1); cnt = cnt + 1 }`
//!   -> `cnt0 + popcount(y0)`, `y = 0` afterwards (SWAR, 12 ops, or a variant without multiply);
//! * a bit-parity loop `for i in i0..N { acc ^= (x >> i) & 1 }` (x loop-invariant, literal bounds
//!   0 <= i0 < N <= 32) -> `acc0 ^ parity(bits i0..N-1 of x)`, `i = N` afterwards.
//!
//! Recognition is on the dataflow of the loop only (no names, no program hashes). The loop must
//! be a header plus a straight-line body with pure instructions, leave only through the header
//! test and have no live values other than the matched phis. The result is emitted into the
//! preheader, the header is cut to jump to the exit.
use crate::algebra::Em;
use crate::ir::*;
use crate::kir::*;
use crate::passes::{apply_rep, NOREP};
use std::collections::{HashMap, HashSet};

struct Shape {
    h: usize,
    p0: usize,
    po: usize,
    pi: usize,
    exit: usize,
    cont_true: bool,
    cond: i64,
    ins: Vec<Instr>,
}

fn pure_op(ins: &Instr) -> bool {
    matches!(ins.op, Op::Add | Op::Sub | Op::And | Op::Or | Op::Xor | Op::Shl | Op::Shr | Op::Lt | Op::Eq)
}

fn shape(f: &Func, li: usize) -> Option<Shape> {
    let h = f.loops[li].header;
    let ids = &f.loops[li].ids;
    let nblocks = f.loops[li].blocks.len();
    let preds = &f.arena[h].preds;
    if preds.len() != 2 {
        return None;
    }
    let po = preds.iter().position(|p| !ids.contains(p))?;
    let pi = 1 - po;
    if !ids.contains(&preds[pi]) {
        return None;
    }
    let p0 = preds[po];
    {
        let t = f.arena[p0].term();
        if t.op != Op::Jmp || t.t.len() != 1 || t.t[0] != h {
            return None;
        }
    }
    let ht = f.arena[h].term().clone();
    if ht.op != Op::Br || ht.a[0] < 0 || ht.t[0] == ht.t[1] {
        return None;
    }
    let (in_idx, cont_true) = if ids.contains(&ht.t[0]) && !ids.contains(&ht.t[1]) {
        (0, true)
    } else if ids.contains(&ht.t[1]) && !ids.contains(&ht.t[0]) {
        (1, false)
    } else {
        return None;
    };
    let body_in = ht.t[in_idx];
    let exit = ht.t[1 - in_idx];
    if body_in == h {
        return None;
    }
    let mut chain: Vec<usize> = Vec::new();
    let mut b = body_in;
    while b != h {
        if !ids.contains(&b) || chain.len() > nblocks || chain.contains(&b) {
            return None;
        }
        let blk = &f.arena[b];
        if !blk.phis.is_empty() || blk.term().op != Op::Jmp || blk.preds.len() != 1 {
            return None;
        }
        chain.push(b);
        b = blk.term().t[0];
    }
    if chain.len() + 1 != nblocks || *chain.last().unwrap() != preds[pi] {
        return None;
    }
    let mut ins: Vec<Instr> = Vec::new();
    for &blk in std::iter::once(&h).chain(chain.iter()) {
        let all = &f.arena[blk].ins;
        for i in &all[..all.len() - 1] {
            if !pure_op(i) {
                return None;
            }
            ins.push(i.clone());
        }
    }
    if ins.len() > 24 || f.arena[h].phis.len() > 4 {
        return None;
    }
    Some(Shape { h, p0, po, pi, exit, cont_true, cond: ht.a[0], ins })
}

/// Is `x` the literal `v` (as a 32-bit value)?
fn is_lit(x: i64, v: i64) -> bool {
    x < 0 && !x == v
}

/// the value `x` is `op(a, lit v)` or `op(lit v, a)` -> a
fn bin_lit(defs: &HashMap<i64, Instr>, x: i64, op: Op, lv: i64) -> Option<i64> {
    let i = defs.get(&x)?;
    if i.op != op || i.a.len() != 2 {
        return None;
    }
    if is_lit(i.a[1], lv) && i.a[0] >= 0 {
        Some(i.a[0])
    } else if op != Op::Sub && is_lit(i.a[0], lv) && i.a[1] >= 0 {
        Some(i.a[1])
    } else {
        None
    }
}

/// Emit popcount of x (32 bit). mode 1: SWAR with multiply (12 ops); mode 2: no multiply (15 ops).
pub fn emit_popcount(em: &mut Em, x: i64, mode: u8) -> i64 {
    let t = em.bin(Op::Shr, x, lit(1));
    let t = em.bin(Op::And, t, lit(0x5555_5555));
    let x = em.bin(Op::Sub, x, t);
    let a = em.bin(Op::And, x, lit(0x3333_3333));
    let b = em.bin(Op::Shr, x, lit(2));
    let b = em.bin(Op::And, b, lit(0x3333_3333));
    let x = em.bin(Op::Add, a, b);
    let t = em.bin(Op::Shr, x, lit(4));
    let x = em.bin(Op::Add, x, t);
    let x = em.bin(Op::And, x, lit(0x0F0F_0F0F));
    if mode == 1 {
        let x = em.bin(Op::Mul, x, lit(0x0101_0101));
        em.bin(Op::Shr, x, lit(24))
    } else {
        let t = em.bin(Op::Shr, x, lit(8));
        let x = em.bin(Op::Add, x, t);
        let t = em.bin(Op::Shr, x, lit(16));
        let x = em.bin(Op::Add, x, t);
        em.bin(Op::And, x, lit(0x3F))
    }
}

/// Emit the parity (0/1) of bits i0..n-1 of x; requires 0 <= i0 < n <= 32.
pub fn emit_parity(em: &mut Em, x: i64, i0: i64, n: i64) -> i64 {
    let w = n - i0;
    let mut y = if i0 > 0 { em.bin(Op::Shr, x, lit(i0)) } else { x };
    if w < 32 {
        y = em.bin(Op::And, y, lit((1i64 << w) - 1));
    }
    for s in [16i64, 8, 4, 2, 1] {
        if s < w {
            let t = em.bin(Op::Shr, y, lit(s));
            y = em.bin(Op::Xor, y, t);
        }
    }
    em.bin(Op::And, y, L1)
}

/// continue iff (v != 0) == nz; peel `eq v, 0` wrappers
fn resolve_nz(defs: &HashMap<i64, Instr>, mut v: i64, mut nz: bool) -> (i64, bool) {
    for _ in 0..4 {
        match defs.get(&v) {
            Some(i) if i.op == Op::Eq && i.a.len() == 2 => {
                let a = if i.a[1] == L0 {
                    i.a[0]
                } else if i.a[0] == L0 {
                    i.a[1]
                } else {
                    return (v, nz);
                };
                if a < 0 {
                    return (v, nz);
                }
                v = a;
                nz = !nz;
            }
            _ => return (v, nz),
        }
    }
    (v, nz)
}

enum Found {
    Kern { y: i64, c: i64, y0: i64, c0: i64 },
    Parity { i: i64, acc: i64, x: i64, i0: i64, n: i64, acc0: i64 },
}

fn recognize(f: &Func, s: &Shape) -> Option<Found> {
    let phis = &f.arena[s.h].phis;
    if phis.len() != 2 {
        return None;
    }
    let defs: HashMap<i64, Instr> = s.ins.iter().map(|i| (i.d, i.clone())).collect();
    let (cv, cnz) = resolve_nz(&defs, s.cond, s.cont_true);
    let phi_of = |d: i64| phis.iter().find(|p| p.d == d);
    if cnz {
        if let Some(yp) = phi_of(cv) {
            // y' = y & (y - 1)
            let i = defs.get(&yp.a[s.pi])?;
            if i.op != Op::And || i.a.len() != 2 {
                return None;
            }
            let dec = |x: i64| -> bool {
                x >= 0 && (bin_lit(&defs, x, Op::Sub, 1) == Some(cv) || bin_lit(&defs, x, Op::Add, 0xFFFF_FFFF) == Some(cv))
            };
            if (i.a[0] == cv && dec(i.a[1])) || (i.a[1] == cv && dec(i.a[0])) {
                let cp = phis.iter().find(|p| p.d != yp.d)?;
                if bin_lit(&defs, cp.a[s.pi], Op::Add, 1) == Some(cp.d) {
                    return Some(Found::Kern { y: yp.d, c: cp.d, y0: yp.a[s.po], c0: cp.a[s.po] });
                }
            }
        }
    }
    parity(f, s, &defs)
}

fn parity(f: &Func, s: &Shape, defs: &HashMap<i64, Instr>) -> Option<Found> {
    let phis = &f.arena[s.h].phis;
    let phi_of = |d: i64| phis.iter().find(|p| p.d == d);
    if !s.cont_true {
        return None;
    }
    let c = defs.get(&s.cond)?;
    if c.op != Op::Lt || c.a[0] < 0 || c.a[1] >= 0 {
        return None;
    }
    let n = !c.a[1];
    let ip = phi_of(c.a[0])?;
    let i0 = ip.a[s.po];
    if i0 >= 0 || bin_lit(defs, ip.a[s.pi], Op::Add, 1) != Some(ip.d) {
        return None;
    }
    let i0 = !i0;
    if !(0 <= i0 && i0 < n && n <= 32) {
        return None;
    }
    let ap = phis.iter().find(|p| p.d != ip.d)?;
    let an = defs.get(&ap.a[s.pi])?;
    if an.op != Op::Xor || an.a.len() != 2 {
        return None;
    }
    let other = if an.a[0] == ap.d {
        an.a[1]
    } else if an.a[1] == ap.d {
        an.a[0]
    } else {
        return None;
    };
    let t = bin_lit(defs, other, Op::And, 1)?;
    let sh = defs.get(&t)?;
    if sh.op != Op::Shr || sh.a[1] != ip.d || sh.a[0] < 0 || defs.contains_key(&sh.a[0]) || phi_of(sh.a[0]).is_some() {
        return None;
    }
    Some(Found::Parity { i: ip.d, acc: ap.d, x: sh.a[0], i0, n, acc0: ap.a[s.po] })
}

/// Rewrite the matching loops; `mode` selects the popcount flavour (1 multiply, 2 shifts).
pub fn kern(f: &mut Func, mode: u8) -> bool {
    let _ = analyze(f);
    let nv0 = f.nval.max(0) as usize + 1;
    let mut rep = vec![NOREP; nv0];
    let mut changed = false;
    for li in 0..f.loops.len() {
        let s = match shape(f, li) {
            Some(s) => s,
            None => continue,
        };
        let found = match recognize(f, &s) {
            Some(x) => x,
            None => continue,
        };
        // live values: only the matched phis may be used outside the loop
        let ids = f.loops[li].ids.clone();
        let mut defined: HashSet<i64> = f.arena[s.h].phis.iter().map(|p| p.d).collect();
        for i in &s.ins {
            defined.insert(i.d);
        }
        let ok_live: Vec<i64> = match &found {
            Found::Kern { y, c, .. } => vec![*y, *c],
            Found::Parity { i, acc, .. } => vec![*i, *acc],
        };
        let mut bad = false;
        for &blk in &f.blocks {
            if ids.contains(&blk) {
                continue;
            }
            let bl = &f.arena[blk];
            for ins in bl.phis.iter().chain(bl.ins.iter()) {
                for &x in &ins.a {
                    if x >= 0 && defined.contains(&x) && !ok_live.contains(&x) {
                        bad = true;
                    }
                }
            }
        }
        if bad {
            continue;
        }
        let mut em = Em::new(f.nval);
        let outs: Vec<(i64, i64)> = match found {
            Found::Kern { y, c, y0, c0 } => {
                let p = emit_popcount(&mut em, y0, mode);
                let t = em.bin(Op::Add, c0, p);
                vec![(y, L0), (c, t)]
            }
            Found::Parity { i, acc, x, i0, n, acc0 } => {
                let p = emit_parity(&mut em, x, i0, n);
                let t = em.bin(Op::Xor, acc0, p);
                vec![(i, lit(n)), (acc, t)]
            }
        };
        let p0 = s.p0;
        let term = f.arena[p0].ins.pop().unwrap();
        f.arena[p0].ins.extend(em.code.drain(..));
        f.arena[p0].ins.push(term);
        f.nval = em.nval;
        rep.resize(f.nval.max(0) as usize + 1, NOREP);
        for (x, v) in outs {
            rep[x as usize] = v;
        }
        let hb = f.arena[s.h].ins.last_mut().unwrap();
        hb.op = Op::Jmp;
        hb.a.clear();
        hb.t = vec![s.exit];
        changed = true;
    }
    if changed {
        rep.resize(f.nval.max(0) as usize + 1, NOREP);
        apply_rep(f, &rep);
        remove_unreachable(f);
        let _ = analyze(f);
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::algebra::run_code;

    fn rnd(seed: &mut u64) -> u32 {
        *seed ^= *seed << 13;
        *seed ^= *seed >> 7;
        *seed ^= *seed << 17;
        (*seed >> 16) as u32
    }

    fn vals() -> Vec<u32> {
        let mut v = vec![0u32, 1, 2, 3, 0x7FFF_FFFF, 0x8000_0000, 0xFFFF_FFFF, 0xFFFF_FFFE, 0x5555_5555, 0xAAAA_AAAA, 0x0F0F_0F0F];
        for k in 0..32 {
            v.push(1 << k);
            v.push(!(1u32 << k));
        }
        let mut s = 0x1234_5678_9abc_def1u64;
        for _ in 0..3000 {
            v.push(rnd(&mut s));
        }
        v
    }

    #[test]
    fn popcount_matches() {
        for mode in [1u8, 2] {
            let mut em = Em::new(1);
            let r = emit_popcount(&mut em, 0, mode);
            if mode == 1 {
                assert_eq!(em.code.len(), 12);
            }
            for x in vals() {
                let mut env = HashMap::new();
                env.insert(0i64, x);
                run_code(&em.code, &mut env);
                assert_eq!(env[&r], x.count_ones(), "mode {} x {:#x}", mode, x);
            }
        }
    }

    #[test]
    fn parity_matches() {
        for n in 1..=32i64 {
            for i0 in 0..n {
                let mut em = Em::new(1);
                let r = emit_parity(&mut em, 0, i0, n);
                for x in vals() {
                    let mut want = 0u32;
                    for i in i0..n {
                        want ^= (x >> i) & 1;
                    }
                    let mut env = HashMap::new();
                    env.insert(0i64, x);
                    run_code(&em.code, &mut env);
                    assert_eq!(env[&r], want, "i0 {} n {} x {:#x}", i0, n, x);
                }
            }
        }
    }
}
