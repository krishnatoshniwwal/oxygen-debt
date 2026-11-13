//! idiom.rs - bit-idiom recognition on SSA dataflow (no names, no program shapes).
//!
//! A counted or data-driven loop whose whole effect is one of
//!  1. `for i in i0..N: acc += (x >> i) & 1`           x loop-invariant, N literal <= 32
//!       -> acc0 + popcount(x & mask[i0..N))            (acc ^= ... : parity, acc0 ^ parity)
//!  2. `for i in i0..N: acc |= ((x >> i) & 1) << (c - i)`   (| or + or ^) -> acc0 op rev(x)
//!  3. `while y != 0 { y &= y - 1; cnt += k }`          -> cnt0 + k * popcount(y0), y = 0
//! is replaced by straight-line code in the preheader (SWAR popcount / 5-stage bit reversal).
//! Loop shape and preheader handling follow `algebra::try_loop`: one entry edge from a block
//! that ends in `jmp header`, one back edge, header ends in `br`, rest is a straight chain, and
//! only header values are live after the loop. Every loop instruction must be part of the idiom
//! (no stores, loads, traps, other live computations), so removing the loop is exact.
//! `mode` 1 sums popcount bytes with a multiply, 2 with shifts (the search decides).
use crate::algebra::Em;
use crate::ir::*;
use crate::kir::*;
use crate::passes::{apply_rep, NOREP};
use std::collections::{HashMap, HashSet};

pub fn run(f: &mut Func, mode: u8) -> bool {
    let _ = analyze(f);
    let nv0 = f.nval.max(0) as usize + 1;
    let mut rep = vec![NOREP; nv0];
    let mut changed = false;
    for li in 0..f.loops.len() {
        if try_loop(f, li, &mut rep, mode) {
            changed = true;
        }
    }
    if changed {
        rep.resize(f.nval.max(0) as usize + 1, NOREP);
        apply_rep(f, &rep);
        remove_unreachable(f);
        let _ = analyze(f);
    }
    changed
}

// ---------------------------------------------------------------------------
// emitted bit tricks (values are 32-bit words)
// ---------------------------------------------------------------------------
fn mask_lit(m: u32) -> i64 {
    lit(m as i64)
}

/// popcount of the 32-bit value v (an operand)
pub fn emit_popcount(em: &mut Em, v: i64, mode: u8) -> i64 {
    let t = em.bin(Op::Shr, v, L1);
    let t = em.bin(Op::And, t, mask_lit(0x5555_5555));
    let v = em.sub(v, t);
    let a = em.bin(Op::And, v, mask_lit(0x3333_3333));
    let b = em.bin(Op::Shr, v, lit(2));
    let b = em.bin(Op::And, b, mask_lit(0x3333_3333));
    let v = em.add(a, b);
    let t = em.bin(Op::Shr, v, lit(4));
    let v = em.add(v, t);
    let v = em.bin(Op::And, v, mask_lit(0x0F0F_0F0F));
    if mode == 1 {
        let v = em.mul(v, lit(0x0101_0101));
        em.bin(Op::Shr, v, lit(24))
    } else {
        let t = em.bin(Op::Shr, v, lit(8));
        let v = em.add(v, t);
        let t = em.bin(Op::Shr, v, lit(16));
        let v = em.add(v, t);
        em.bin(Op::And, v, lit(0x3F))
    }
}

/// parity (xor of all bits) of v
pub fn emit_parity(em: &mut Em, v: i64) -> i64 {
    let t = em.bin(Op::Shr, v, lit(16));
    let v = em.bin(Op::Xor, v, t);
    let t = em.bin(Op::Shr, v, lit(8));
    let v = em.bin(Op::Xor, v, t);
    let t = em.bin(Op::Shr, v, lit(4));
    let v = em.bin(Op::Xor, v, t);
    let v = em.bin(Op::And, v, lit(15));
    let r = em.bin(Op::Shr, lit(0x6996), v);
    em.bin(Op::And, r, L1)
}

/// reversal of all 32 bits
pub fn emit_rev32(em: &mut Em, v: i64) -> i64 {
    let mut v = v;
    for (s, m) in [(1u32, 0x5555_5555u32), (2, 0x3333_3333), (4, 0x0F0F_0F0F), (8, 0x00FF_00FF)] {
        let a = em.bin(Op::Shr, v, lit(s as i64));
        let a = em.bin(Op::And, a, mask_lit(m));
        let b = em.bin(Op::And, v, mask_lit(m));
        let b = em.bin(Op::Shl, b, lit(s as i64));
        v = em.add(a, b);
    }
    let a = em.bin(Op::Shr, v, lit(16));
    let b = em.bin(Op::Shl, v, lit(16));
    em.add(a, b)
}

// ---------------------------------------------------------------------------
// loop recognition
// ---------------------------------------------------------------------------
struct Shape {
    h: usize,
    p0: usize,
    exit: usize,
    cont_true: bool,
    cond: i64,                 // the branch condition operand
    phis: Vec<(i64, i64, i64)>, // (d, init, next)
    ins: Vec<Instr>,           // header + chain instructions, terminators excluded
    live: Vec<i64>,
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
    let mut defined: HashSet<i64> = HashSet::new();
    let phis: Vec<(i64, i64, i64)> = f.arena[h].phis.iter().map(|p| (p.d, p.a[po], p.a[pi])).collect();
    for p in &phis {
        defined.insert(p.0);
    }
    for &blk in std::iter::once(&h).chain(chain.iter()) {
        let all = &f.arena[blk].ins;
        for i in &all[..all.len() - 1] {
            if !is_pure(i.op) {
                return None;
            }
            defined.insert(i.d);
            ins.push(i.clone());
        }
    }
    if ins.len() > 40 || phis.len() > 4 {
        return None;
    }
    let mut live: Vec<i64> = Vec::new();
    for &blk in &f.blocks {
        if ids.contains(&blk) {
            continue;
        }
        let bl = &f.arena[blk];
        for i in bl.phis.iter().chain(bl.ins.iter()) {
            for &x in &i.a {
                if x >= 0 && defined.contains(&x) && !live.contains(&x) {
                    live.push(x);
                }
            }
        }
    }
    // only the loop's phis may be live afterwards
    if live.iter().any(|x| !phis.iter().any(|p| p.0 == *x)) {
        return None;
    }
    Some(Shape { h, p0, exit, cont_true, cond: ht.a[0], phis, ins, live })
}

fn strip_val(x: i64, defs: &HashMap<i64, Instr>) -> Option<&Instr> {
    if x < 0 {
        None
    } else {
        defs.get(&x)
    }
}

/// `(x >> i) & 1` for the phi `i`: returns x (loop-invariant operand) and the instrs used.
fn bit_of(e: i64, i: i64, defs: &HashMap<i64, Instr>, used: &mut Vec<i64>) -> Option<i64> {
    let a = strip_val(e, defs)?;
    if a.op != Op::And {
        return None;
    }
    let s = if a.a[1] == L1 { a.a[0] } else if a.a[0] == L1 { a.a[1] } else { return None };
    let sh = strip_val(s, defs)?;
    if sh.op != Op::Shr || sh.a[1] != i || sh.a[0] < 0 || defs.contains_key(&sh.a[0]) || sh.a[0] == i {
        return None;
    }
    used.push(e);
    used.push(s);
    Some(sh.a[0])
}

/// literal value of a loop-invariant operand
fn lit_of(x: i64) -> Option<u32> {
    if x < 0 {
        Some(!x as u32)
    } else {
        None
    }
}

fn range_mask(i0: u32, n: u32) -> u32 {
    let mut m = 0u32;
    for b in i0..n.min(32) {
        m |= 1 << b;
    }
    m
}

fn try_loop(f: &mut Func, li: usize, rep: &mut Vec<i64>, mode: u8) -> bool {
    let sh = match shape(f, li) {
        Some(s) => s,
        None => return false,
    };
    let defs: HashMap<i64, Instr> = sh.ins.iter().map(|i| (i.d, i.clone())).collect();
    let phi_next: HashMap<i64, (i64, i64)> = sh.phis.iter().map(|p| (p.0, (p.1, p.2))).collect();
    let mut em = Em::new(f.nval);
    let mut outv: Vec<(i64, i64)> = Vec::new();
    let ok = match counted(&sh, &defs, &phi_next, &mut em, &mut outv, mode) {
        Some(true) => true,
        _ => {
            em = Em::new(f.nval);
            outv.clear();
            kernighan(&sh, &defs, &phi_next, &mut em, &mut outv, mode) == Some(true)
        }
    };
    if !ok {
        return false;
    }
    // every live value must have a replacement
    if sh.live.iter().any(|x| !outv.iter().any(|o| o.0 == *x)) {
        return false;
    }
    let (p0, h, exit) = (sh.p0, sh.h, sh.exit);
    let term = f.arena[p0].ins.pop().unwrap();
    f.arena[p0].ins.extend(em.code.drain(..));
    f.arena[p0].ins.push(term);
    f.nval = em.nval;
    rep.resize(f.nval.max(0) as usize + 1, NOREP);
    for (x, v) in outv {
        rep[x as usize] = v;
    }
    let hb = f.arena[h].ins.last_mut().unwrap();
    hb.op = Op::Jmp;
    hb.a.clear();
    hb.t = vec![exit];
    true
}

/// Loops 1 and 2: an induction variable i = i0, i0+1, ... while i < N.
fn counted(sh: &Shape, defs: &HashMap<i64, Instr>, pn: &HashMap<i64, (i64, i64)>, em: &mut Em, outv: &mut Vec<(i64, i64)>, mode: u8) -> Option<bool> {
    if sh.phis.len() != 2 || !sh.cont_true {
        return Some(false);
    }
    // exit test lt i, N
    let c = strip_val(sh.cond, defs)?;
    if c.op != Op::Lt {
        return Some(false);
    }
    let i = c.a[0];
    let n = lit_of(c.a[1])?;
    let (i_init, i_next) = *pn.get(&i)?;
    if n > 32 {
        return Some(false);
    }
    let i0 = lit_of(i_init)?;
    if i0 > 32 {
        return Some(false);
    }
    let inc = strip_val(i_next, defs)?;
    if inc.op != Op::Add || !((inc.a[0] == i && inc.a[1] == L1) || (inc.a[1] == i && inc.a[0] == L1)) {
        return Some(false);
    }
    let acc = sh.phis.iter().map(|p| p.0).find(|&d| d != i)?;
    let (a0, a_next) = pn[&acc];
    let upd = strip_val(a_next, defs)?;
    let (u0, u1) = (upd.a[0], upd.a[1]);
    let other = if u0 == acc { u1 } else if u1 == acc { u0 } else { return Some(false) };
    let mut used = vec![c.d, inc.d, upd.d];
    let mask = range_mask(i0, n);
    let res;
    if let Some(x) = bit_of(other, i, defs, &mut used) {
        // popcount / parity
        let v = if mask == u32::MAX { x } else { em.bin(Op::And, x, mask_lit(mask)) };
        match upd.op {
            Op::Add => {
                let p = emit_popcount(em, v, mode);
                res = em.add(a0, p);
            }
            Op::Xor => {
                let p = emit_parity(em, v);
                res = em.bin(Op::Xor, a0, p);
            }
            _ => return Some(false),
        }
    } else {
        // bit reversal: other = shl(bit, s - i) with s literal
        let s = strip_val(other, defs)?;
        if s.op != Op::Shl {
            return Some(false);
        }
        let amt = strip_val(s.a[1], defs)?;
        if amt.op != Op::Sub || amt.a[1] != i {
            return Some(false);
        }
        let cpos = lit_of(amt.a[0])?;
        if cpos > 31 {
            return Some(false);
        }
        used.push(s.d);
        used.push(amt.d);
        let x = bit_of(s.a[0], i, defs, &mut used)?;
        // positions c - i for i in [i0, n) must stay >= 0
        if n > 0 && i0 < n && cpos + 1 < n {
            return Some(false);
        }
        if !matches!(upd.op, Op::Or | Op::Xor | Op::Add) {
            return Some(false);
        }
        let v = if mask == u32::MAX { x } else { em.bin(Op::And, x, mask_lit(mask)) };
        let r = emit_rev32(em, v);
        let r = em.bin(Op::Shr, r, lit(31 - cpos as i64));
        res = em.bin(upd.op, a0, r);
    }
    // every instruction of the loop must belong to the idiom
    if sh.ins.iter().any(|x| !used.contains(&x.d)) {
        return Some(false);
    }
    for &x in &sh.live {
        if x == acc {
            outv.push((x, res));
        } else if x == i {
            // final value of i: max(i0, N) for the signed test with literals in 0..=32
            outv.push((x, lit(i0.max(n) as i64)));
        }
    }
    Some(true)
}

/// Loop 3: while (y != 0) { y &= y - 1; cnt += k }
fn kernighan(sh: &Shape, defs: &HashMap<i64, Instr>, pn: &HashMap<i64, (i64, i64)>, em: &mut Em, outv: &mut Vec<(i64, i64)>, mode: u8) -> Option<bool> {
    if sh.phis.len() != 2 {
        return Some(false);
    }
    // peel `eq _, 0` layers off the condition: each flips the sense
    let mut x = sh.cond;
    let mut nonzero = sh.cont_true; // loop continues iff (x != 0) == nonzero... see below
    let mut used: Vec<i64> = Vec::new();
    let mut flips = 0;
    while let Some(d) = strip_val(x, defs) {
        if d.op == Op::Eq && d.a[1] == L0 {
            used.push(d.d);
            x = d.a[0];
            flips += 1;
        } else {
            break;
        }
    }
    if flips % 2 == 1 {
        nonzero = !nonzero;
    }
    if !nonzero || x < 0 {
        return Some(false); // the loop must continue exactly while x != 0
    }
    let y = x;
    let (y0, y_next) = *pn.get(&y)?;
    let cnt = sh.phis.iter().map(|p| p.0).find(|&d| d != y)?;
    let (c0, c_next) = pn[&cnt];
    // y_next = y & (y - 1)
    let an = strip_val(y_next, defs)?;
    if an.op != Op::And {
        return Some(false);
    }
    let dec = if an.a[0] == y { an.a[1] } else if an.a[1] == y { an.a[0] } else { return Some(false) };
    let dd = strip_val(dec, defs)?;
    let is_dec = (dd.op == Op::Sub && dd.a[0] == y && dd.a[1] == L1) || (dd.op == Op::Add && ((dd.a[0] == y && dd.a[1] == lit(MASK)) || (dd.a[1] == y && dd.a[0] == lit(MASK))));
    if !is_dec {
        return Some(false);
    }
    used.extend([an.d, dd.d]);
    // cnt_next = cnt + k
    let cn = strip_val(c_next, defs)?;
    if cn.op != Op::Add {
        return Some(false);
    }
    let k = if cn.a[0] == cnt { cn.a[1] } else if cn.a[1] == cnt { cn.a[0] } else { return Some(false) };
    let k = lit_of(k)?;
    used.push(cn.d);
    if sh.ins.iter().any(|i| !used.contains(&i.d)) {
        return Some(false);
    }
    let p = emit_popcount(em, y0, mode);
    let kp = em.mul(p, lit(k as i64));
    let res = em.add(c0, kp);
    for &v in &sh.live {
        if v == cnt {
            outv.push((v, res));
        } else if v == y {
            outv.push((v, L0));
        }
    }
    Some(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::algebra::run_code;

    fn run1(code: &[Instr], r: i64, x: u32) -> u32 {
        let mut env = HashMap::new();
        env.insert(0i64, x);
        run_code(code, &mut env);
        if r < 0 {
            !r as u32
        } else {
            env[&r]
        }
    }

    fn samples() -> Vec<u32> {
        let mut v = vec![0, 1, 2, 3, 0x8000_0000, 0xFFFF_FFFF, 0x7FFF_FFFF, 0x5555_5555, 0xAAAA_AAAA, 0x0001_0000, 0xDEAD_BEEF, 0xFF];
        let mut s = 0x1234_5678u32;
        for _ in 0..300 {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            v.push(s);
        }
        for b in 0..32 {
            v.push(1 << b);
            v.push(!(1u32 << b));
        }
        v
    }

    #[test]
    fn popcount_parity_rev_emitters() {
        for mode in [1u8, 2] {
            let mut em = Em::new(1);
            let r = emit_popcount(&mut em, 0, mode);
            for x in samples() {
                assert_eq!(run1(&em.code, r, x), x.count_ones(), "pop mode {} x={:#x}", mode, x);
            }
        }
        let mut em = Em::new(1);
        let r = emit_parity(&mut em, 0);
        for x in samples() {
            assert_eq!(run1(&em.code, r, x), x.count_ones() & 1, "parity x={:#x}", x);
        }
        let mut em = Em::new(1);
        let r = emit_rev32(&mut em, 0);
        for x in samples() {
            assert_eq!(run1(&em.code, r, x), x.reverse_bits(), "rev x={:#x}", x);
        }
    }

    /// reference interpretation of the three loops, compared with the formulas used in try_loop
    #[test]
    fn counted_formulas_match_loops() {
        for i0 in 0..=33u32 {
            for n in 1..=32u32 {
                let mask = range_mask(i0, n);
                for x in samples() {
                    let (mut pc, mut par, mut rv) = (5u32, 5u32, 5u32);
                    let c = 31u32;
                    let mut i = i0;
                    while (i as i32) < n as i32 {
                        let b = (x >> (i & 31)) & 1;
                        pc = pc.wrapping_add(b);
                        par ^= b;
                        rv |= b << ((c - i) & 31);
                        i += 1;
                    }
                    assert_eq!(pc, 5u32.wrapping_add((x & mask).count_ones()));
                    assert_eq!(par, 5 ^ ((x & mask).count_ones() & 1));
                    assert_eq!(rv, 5 | ((x & mask).reverse_bits() >> (31 - c)), "i0={} n={} x={:#x}", i0, n, x);
                }
            }
        }
        // reversal into positions c - i with c = n - 1
        for n in 1..=32u32 {
            let mask = range_mask(0, n);
            for x in samples() {
                let mut rv = 0u32;
                for i in 0..n {
                    rv |= ((x >> i) & 1) << (n - 1 - i);
                }
                assert_eq!(rv, (x & mask).reverse_bits() >> (32 - n));
            }
        }
    }

    #[test]
    fn kernighan_formula() {
        for y in samples() {
            let (mut v, mut c) = (y, 7u32);
            while v != 0 {
                v &= v.wrapping_sub(1);
                c += 3;
            }
            assert_eq!(c, 7 + 3 * y.count_ones());
        }
    }

    #[test]
    fn end_to_end_shapes() {
        let counted = |body: &str, n: u32, init: &str| {
            format!(
                "func t(v0)\nentry:\n  const v1, {init}\n  const v2, 0\n  jmp h\nh:\n  lt v3, v2, {n}\n  br v3, b, x\nb:\n{body}  add v7, v2, 1\n  mov v2, v7\n  jmp h\nx:\n  ret v1\n"
            )
        };
        let pop = "  shr v4, v0, v2\n  and v5, v4, 1\n  add v6, v1, v5\n  mov v1, v6\n";
        let par = "  shr v4, v0, v2\n  and v5, v4, 1\n  xor v6, v1, v5\n  mov v1, v6\n";
        for n in 1..=32u32 {
            let rev = format!("  shr v4, v0, v2\n  and v5, v4, 1\n  sub v8, {}, v2\n  shl v9, v5, v8\n  or v6, v1, v9\n  mov v1, v6\n", n - 1);
            for (body, init) in [(pop.to_string(), "3"), (par.to_string(), "1"), (rev, "0")] {
                let src = counted(&body, n, init);
                check(&src);
            }
        }
        let kern = "func t(v0)\nentry:\n  mov v5, v0\n  const v2, 4\n  jmp h\nh:\n  eq v11, v5, 0\n  eq v10, v11, 0\n  br v10, b, x\nb:\n  sub v12, v5, 1\n  and v13, v5, v12\n  mov v5, v13\n  add v14, v2, 1\n  mov v2, v14\n  jmp h\nx:\n  ret v2\n";
        check(kern);
    }

    fn check(src: &str) {
        use crate::passes;
        use crate::ssa;
        use std::collections::BTreeMap;
        let orig = parse_program(src).unwrap();
        let co = compile_program(&orig);
        for mode in [1u8, 2] {
            let mut f = from_program(&orig);
            ssa::build_ssa(&mut f);
            for _ in 0..6 {
                let c1 = passes::scalar_pass(&mut f);
                let c2 = passes::cleanup_cfg(&mut f);
                let c3 = passes::dce(&mut f, None);
                if !(c1 || c2 || c3) {
                    break;
                }
            }
            assert!(run(&mut f, mode), "idiom not recognised in:\n{}", src);
            for _ in 0..6 {
                let c1 = passes::scalar_pass(&mut f);
                let c2 = passes::cleanup_cfg(&mut f);
                let c3 = passes::dce(&mut f, None);
                if !(c1 || c2 || c3) {
                    break;
                }
            }
            ssa::check_ssa(&mut f).unwrap();
            assert!(f.loops.is_empty(), "loop left in:\n{}", to_text(&f));
            ssa::destruct_ssa(&mut f);
            ssa::allocate(&mut f, None, false, false, 0, 0, 0).unwrap();
            let text = to_text(&f);
            let cn = compile_program(&parse_program(&text).unwrap());
            for x in samples() {
                let inp = Inputs { args: vec![x], segments: vec![], vin: BTreeMap::new() };
                let a = execute(&co, &inp, 200000, false);
                let b = execute(&cn, &inp, 200000, false);
                assert!(!differs(&a, &b), "x={:#x} mode {} src\n{}\nnew\n{}", x, mode, src, text);
            }
        }
    }
}
