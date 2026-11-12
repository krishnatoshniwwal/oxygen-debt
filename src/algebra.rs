//! algebra.rs - loop algebra for the numbers / loops kinds (SSA form).
//!
//! * closed forms of polynomial recurrences (chains of recurrences, degree <= 4 in the trip
//!   count, i.e. summands of degree <= 3) with the trip count computed from the loop's own
//!   exit test: see `closed_forms`;
//! * (further sections are appended below: division by constants, divisibility tests,
//!   div/mod of induction variables, strength reduction.)
//!
//! Every rewrite here is an identity of 32-bit wrap-around arithmetic or is guarded by a proof
//! from the value ranges. Nothing here looks at program names or shapes of whole programs.
use crate::ir::*;
use crate::kir::*;
use crate::passes::{apply_rep, evalop, NOREP};
use crate::ranges::Ranges;
use std::collections::{HashMap, HashSet};

pub const SMAX: i64 = 0x7FFF_FFFF;

/// Inverse of an odd number modulo 2^32 (Newton iteration; each step doubles the correct bits).
pub fn inv_odd(c: u32) -> u32 {
    debug_assert!(c & 1 == 1);
    let mut x = c;
    for _ in 0..5 {
        x = x.wrapping_mul(2u32.wrapping_sub(c.wrapping_mul(x)));
    }
    x
}

// ---------------------------------------------------------------------------
// Emitter: straight-line code with constant folding, collected in a list that is only
// inserted into the program when the whole rewrite succeeds.
// ---------------------------------------------------------------------------
pub struct Em {
    pub code: Vec<Instr>,
    pub nval: i64,
}

impl Em {
    pub fn new(nval: i64) -> Em {
        Em { code: Vec::new(), nval }
    }
    fn val(&mut self) -> i64 {
        let v = self.nval;
        self.nval += 1;
        v
    }
    /// A binary operation with folding of literal operands and the obvious identities.
    pub fn bin(&mut self, op: Op, a: i64, b: i64) -> i64 {
        if a < 0 && b < 0 {
            if let Some(r) = evalop(op, !a, !b) {
                return lit(r);
            }
        }
        match op {
            Op::Add | Op::Or | Op::Xor => {
                if a == L0 {
                    return b;
                }
                if b == L0 {
                    return a;
                }
            }
            Op::Sub | Op::Shl | Op::Shr => {
                if b == L0 {
                    return a;
                }
            }
            Op::Mul => {
                if a == L0 || b == L0 {
                    return L0;
                }
                if a == L1 {
                    return b;
                }
                if b == L1 {
                    return a;
                }
            }
            _ => {}
        }
        let d = self.val();
        // literals second for commutative ops (the simplifier would do the same)
        let (a, b) = if is_comm(op) && a < 0 && b >= 0 { (b, a) } else { (a, b) };
        self.code.push(Instr::new(op, d, vec![a, b], vec![]));
        d
    }
    pub fn add(&mut self, a: i64, b: i64) -> i64 {
        self.bin(Op::Add, a, b)
    }
    pub fn sub(&mut self, a: i64, b: i64) -> i64 {
        self.bin(Op::Sub, a, b)
    }
    pub fn mul(&mut self, a: i64, b: i64) -> i64 {
        self.bin(Op::Mul, a, b)
    }
    pub fn select(&mut self, c: i64, a: i64, b: i64) -> i64 {
        if c < 0 {
            return if !c != 0 { a } else { b };
        }
        if a == b {
            return a;
        }
        let d = self.val();
        self.code.push(Instr::new(Op::Select, d, vec![c, a, b], vec![]));
        d
    }
}

/// Run an emitted list (used by the unit tests): env maps value numbers to values.
pub fn run_code(code: &[Instr], env: &mut HashMap<i64, u32>) {
    let g = |env: &HashMap<i64, u32>, x: i64| -> i64 {
        if x < 0 {
            !x
        } else {
            *env.get(&x).expect("run_code: undefined value") as i64
        }
    };
    for ins in code {
        let r = match ins.op {
            Op::Select => {
                if g(env, ins.a[0]) != 0 {
                    g(env, ins.a[1])
                } else {
                    g(env, ins.a[2])
                }
            }
            Op::Mac => (g(env, ins.a[0]).wrapping_mul(g(env, ins.a[1])) + g(env, ins.a[2])) & MASK,
            Op::Addsh => (g(env, ins.a[0]) + (g(env, ins.a[1]) << (!ins.a[2] & 31))) & MASK,
            op => evalop(op, g(env, ins.a[0]), g(env, ins.a[1])).expect("run_code: trap"),
        };
        env.insert(ins.d, r as u32);
    }
}

// ---------------------------------------------------------------------------
// Polynomials in the binomial basis: P(k) = sum_j c_j * C(k, j), coefficients in Z/2^32
// (as operands: literals or outside values). `ev` is a syntactic proof that the coefficient
// is even (needed only for the coefficient of C(k,4)).
// ---------------------------------------------------------------------------
pub const MAXDEG: usize = 4;

#[derive(Clone, Copy, Debug)]
pub struct Cf {
    pub v: i64,
    pub ev: bool,
}

pub const ZERO: Cf = Cf { v: L0, ev: true };

fn mk(v: i64, ev: bool) -> Cf {
    if v < 0 {
        Cf { v, ev: (!v) & 1 == 0 }
    } else {
        Cf { v, ev }
    }
}
pub fn cf_lit(x: u32) -> Cf {
    mk(lit(x as i64), false)
}
pub fn cf_val(v: i64) -> Cf {
    mk(v, false)
}

pub type Poly = Vec<Cf>;

fn trim(p: &mut Poly) {
    while p.len() > 1 && p.last().unwrap().v == L0 {
        p.pop();
    }
}

fn bn(n: u64, k: u64) -> u64 {
    let mut r: u64 = 1;
    for i in 0..k {
        r = r * (n - i) / (i + 1);
    }
    r
}

fn cf_add(em: &mut Em, x: Cf, y: Cf) -> Cf {
    mk(em.add(x.v, y.v), x.ev && y.ev)
}

pub fn padd(em: &mut Em, a: &Poly, b: &Poly, negate_b: bool) -> Poly {
    let n = a.len().max(b.len());
    let mut r: Poly = Vec::with_capacity(n);
    for i in 0..n {
        let x = a.get(i).copied().unwrap_or(ZERO);
        let y = b.get(i).copied().unwrap_or(ZERO);
        r.push(if negate_b { mk(em.sub(x.v, y.v), x.ev && y.ev) } else { cf_add(em, x, y) });
    }
    trim(&mut r);
    r
}

/// P * literal c.
pub fn pscale(em: &mut Em, a: &Poly, c: u32) -> Poly {
    let mut r: Poly = a.iter().map(|x| mk(em.mul(x.v, lit(c as i64)), x.ev || c & 1 == 0)).collect();
    trim(&mut r);
    r
}

/// C(k,a) * C(k,b) = sum_t C(t,a) C(a,t-b) C(k,t), t = max(a,b)..a+b
pub fn pmul(em: &mut Em, a: &Poly, b: &Poly) -> Option<Poly> {
    let (da, db) = (a.len() - 1, b.len() - 1);
    if da + db > MAXDEG {
        return None;
    }
    let mut r: Poly = vec![ZERO; da + db + 1];
    for i in 0..=da {
        if a[i].v == L0 {
            continue;
        }
        for j in 0..=db {
            if b[j].v == L0 {
                continue;
            }
            let prod = em.mul(a[i].v, b[j].v);
            let pev = a[i].ev || b[j].ev;
            for t in i.max(j)..=(i + j) {
                let m = bn(t as u64, i as u64) * bn(i as u64, (t - j) as u64);
                let term = em.mul(prod, lit(m as i64));
                let c = mk(term, pev || m % 2 == 0);
                r[t] = cf_add(em, r[t], c);
            }
        }
    }
    trim(&mut r);
    Some(r)
}

/// Q(k) with Q(0) = init and Q(k+1) - Q(k) = f(k):  Q = init + sum_m f_m C(k, m+1).
pub fn psum(init: Cf, f: &Poly) -> Option<Poly> {
    if f.len() + 1 > MAXDEG + 1 {
        return None;
    }
    let mut r: Poly = vec![init];
    r.extend(f.iter().copied());
    trim(&mut r);
    Some(r)
}

/// Value of P at the integer k in [0, 2^32) (an operand). The binomials are computed exactly
/// modulo 2^32 without any division by an even number of a wrapped value:
///   C(k,2) = (k>>1) * ((k-1)|1)            (exact for every k)
///   C(k,3) = C(k,2) * (k-2) * inv(3)
///   2*C(k,4) = C(k,2) * C(k-2,2) * inv(3)  and a4*C(k,4) = (a4>>1) * 2*C(k,4)  (a4 even)
/// where "C(k-2,2)" uses the same half trick. Returns None for an odd, unknown a4.
pub fn peval(em: &mut Em, p: &Poly, t: i64) -> Option<i64> {
    let mut acc = p[0].v;
    if p.len() > 1 {
        let x = em.mul(p[1].v, t);
        acc = em.add(acc, x);
    }
    let mut a2 = L0;
    if p.len() > 2 {
        let half = em.bin(Op::Shr, t, L1);
        let tm1 = em.sub(t, L1);
        let odd = em.bin(Op::Or, tm1, L1);
        a2 = em.mul(half, odd);
        let x = em.mul(p[2].v, a2);
        acc = em.add(acc, x);
    }
    if p.len() > 3 {
        let tm2 = em.sub(t, lit(2));
        let x = em.mul(a2, tm2);
        let c3 = em.mul(x, lit(inv_odd(3) as i64));
        let y = em.mul(p[3].v, c3);
        acc = em.add(acc, y);
    }
    if p.len() > 4 {
        if !p[4].ev {
            return None;
        }
        let tm2 = em.sub(t, lit(2));
        let h2 = em.bin(Op::Shr, tm2, L1);
        let tm3 = em.sub(t, lit(3));
        let o3 = em.bin(Op::Or, tm3, L1);
        let b2 = em.mul(h2, o3);
        let ab = em.mul(a2, b2);
        let c4x2 = em.mul(ab, lit(inv_odd(3) as i64));
        let bq = em.bin(Op::Shr, p[4].v, L1);
        let y = em.mul(bq, c4x2);
        acc = em.add(acc, y);
    }
    Some(acc)
}

// ---------------------------------------------------------------------------
// Values of a loop as "polynomial in the iteration number k plus a linear combination of the
// header phis that are not solved yet".
// ---------------------------------------------------------------------------
#[derive(Clone, Debug)]
struct Cp {
    p: Poly,
    lin: Vec<(i64, u32)>,
}

fn cp_const(x: i64) -> Cp {
    Cp { p: vec![mk(x, false)], lin: vec![] }
}

fn cp_lit(c: &Cp) -> Option<u32> {
    if c.lin.is_empty() && c.p.len() == 1 && c.p[0].v < 0 {
        Some(!c.p[0].v as u32)
    } else {
        None
    }
}

fn lin_merge(a: &[(i64, u32)], b: &[(i64, u32)], neg: bool) -> Vec<(i64, u32)> {
    let mut m: Vec<(i64, u32)> = a.to_vec();
    for &(d, c) in b {
        let c = if neg { 0u32.wrapping_sub(c) } else { c };
        if let Some(e) = m.iter_mut().find(|e| e.0 == d) {
            e.1 = e.1.wrapping_add(c);
        } else {
            m.push((d, c));
        }
    }
    m.retain(|e| e.1 != 0);
    m.sort();
    m
}

fn cp_add(em: &mut Em, a: &Cp, b: &Cp, neg: bool) -> Cp {
    Cp { p: padd(em, &a.p, &b.p, neg), lin: lin_merge(&a.lin, &b.lin, neg) }
}

fn cp_mul(em: &mut Em, a: &Cp, b: &Cp) -> Option<Cp> {
    if a.lin.is_empty() && b.lin.is_empty() {
        return Some(Cp { p: pmul(em, &a.p, &b.p)?, lin: vec![] });
    }
    // an unsolved phi may only be scaled by a literal
    let (x, c) = if let Some(c) = cp_lit(b) { (a, c) } else if let Some(c) = cp_lit(a) { (b, c) } else { return None };
    Some(Cp { p: pscale(em, &x.p, c), lin: x.lin.iter().map(|&(d, k)| (d, k.wrapping_mul(c))).filter(|e| e.1 != 0).collect() })
}

fn is_inv(c: &Cp) -> bool {
    c.lin.is_empty() && c.p.len() == 1
}

/// Allowed (effect-free, trap-free) instructions inside a loop that is to disappear.
fn harmless(ins: &Instr) -> bool {
    match ins.op {
        Op::Add | Op::Sub | Op::Mul | Op::And | Op::Or | Op::Xor | Op::Shl | Op::Shr | Op::Lt | Op::Eq | Op::Select | Op::Mac | Op::Addsh => true,
        Op::Div | Op::Mod => ins.a[1] < 0 && ins.a[1] != L0,
        _ => false,
    }
}

fn eval_ins(em: &mut Em, ins: &Instr, get: &dyn Fn(i64) -> Option<Cp>) -> Option<Cp> {
    match ins.op {
        Op::Add | Op::Sub => {
            let (a, b) = (get(ins.a[0])?, get(ins.a[1])?);
            Some(cp_add(em, &a, &b, ins.op == Op::Sub))
        }
        Op::Mul => {
            let (a, b) = (get(ins.a[0])?, get(ins.a[1])?);
            cp_mul(em, &a, &b)
        }
        Op::Mac => {
            let (a, b, c) = (get(ins.a[0])?, get(ins.a[1])?, get(ins.a[2])?);
            let m = cp_mul(em, &a, &b)?;
            Some(cp_add(em, &m, &c, false))
        }
        Op::Addsh => {
            let (a, b) = (get(ins.a[0])?, get(ins.a[1])?);
            let s = (!ins.a[2]) & 31;
            let m = cp_mul(em, &b, &cp_const(lit(1i64 << s)))?;
            Some(cp_add(em, &a, &m, false))
        }
        Op::Shl => {
            let (a, b) = (get(ins.a[0])?, get(ins.a[1])?);
            if let Some(s) = cp_lit(&b) {
                return cp_mul(em, &a, &cp_const(lit(1i64 << (s & 31))));
            }
            if is_inv(&a) && is_inv(&b) {
                return Some(cp_const(em.bin(Op::Shl, a.p[0].v, b.p[0].v)));
            }
            None
        }
        Op::And | Op::Or | Op::Xor | Op::Shr | Op::Lt | Op::Eq | Op::Div | Op::Mod => {
            let (a, b) = (get(ins.a[0])?, get(ins.a[1])?);
            if !(is_inv(&a) && is_inv(&b)) {
                return None;
            }
            if ins.op == Op::Div || ins.op == Op::Mod {
                // only with literals (folded, never emitted: the divisor must be a known non-zero)
                if a.p[0].v < 0 && b.p[0].v < 0 && b.p[0].v != L0 {
                    return Some(cp_const(lit(evalop(ins.op, !a.p[0].v, !b.p[0].v)?)));
                }
                return None;
            }
            Some(cp_const(em.bin(ins.op, a.p[0].v, b.p[0].v)))
        }
        Op::Select => {
            let (c, a, b) = (get(ins.a[0])?, get(ins.a[1])?, get(ins.a[2])?);
            if !(is_inv(&a) && is_inv(&b) && is_inv(&c)) {
                return None;
            }
            Some(cp_const(em.select(c.p[0].v, a.p[0].v, b.p[0].v)))
        }
        _ => None,
    }
}

fn slope(c: &Cp) -> Option<(i64, u32)> {
    if !c.lin.is_empty() {
        return None;
    }
    match c.p.len() {
        1 => Some((c.p[0].v, 0)),
        2 if c.p[1].v < 0 => Some((c.p[0].v, !c.p[1].v as u32)),
        _ => None,
    }
}

fn nonneg(iv: (i64, i64)) -> bool {
    iv.1 <= SMAX
}

/// Some(true/false) if the ranges decide the signed comparison a < b.
fn lt_known(rng: &Ranges, blk: usize, a: i64, b: i64) -> Option<bool> {
    let (ia, ib) = (rng.get(a, blk), rng.get(b, blk));
    if nonneg(ia) && nonneg(ib) {
        if ia.1 < ib.0 {
            return Some(true);
        }
        if ia.0 >= ib.1 {
            return Some(false);
        }
    }
    None
}

/// Number of body iterations T of a loop whose header exits on `br c, .., ..` where c is
/// lt/eq of two linear sequences. `cont_true`: the body is entered when c is true.
fn trip_count(em: &mut Em, rng: &Ranges, p0: usize, cond: &Instr, ca: &Cp, cb: &Cp, cont_true: bool) -> Option<i64> {
    let (a0, a1) = slope(ca)?;
    let (b0, b1) = slope(cb)?;
    let d1 = a1.wrapping_sub(b1);
    match (cond.op, cont_true) {
        (Op::Lt, true) if d1 == 1 => {
            // continue while A0 + ..  < B0: k < B0 - A0 (signed), no wrap: A rises to B0 at most
            Some(match lt_known(rng, p0, a0, b0) {
                Some(true) => em.sub(b0, a0),
                Some(false) => L0,
                None => {
                    let g = em.bin(Op::Lt, a0, b0);
                    let d = em.sub(b0, a0);
                    em.select(g, d, L0)
                }
            })
        }
        (Op::Lt, false) if d1 == 0xFFFF_FFFF => {
            // continue while A >= B, A falls by one per step relative to B: k <= A0 - B0
            Some(match lt_known(rng, p0, a0, b0) {
                Some(true) => L0,
                Some(false) => {
                    let d = em.sub(a0, b0);
                    em.add(d, L1)
                }
                None => {
                    let g = em.bin(Op::Lt, a0, b0);
                    let d = em.sub(a0, b0);
                    let d1 = em.add(d, L1);
                    em.select(g, L0, d1)
                }
            })
        }
        (Op::Eq, false) if d1 & 1 == 1 => {
            // leave when A(k) == B(k): d1 * k == B0 - A0 (mod 2^32), unique solution
            let d = em.sub(b0, a0);
            Some(em.mul(d, lit(inv_odd(d1) as i64)))
        }
        _ => None,
    }
}

/// Replace polynomial loops by their closed form (SSA form). A loop qualifies when
/// * it has one entry edge from a block that ends in `jmp header`, one back edge, and a header
///   that ends in `br c, body, exit` (or with the branches swapped); the rest of the loop is a
///   straight chain of blocks;
/// * every instruction is arithmetic that cannot trap (no load, store, volatile, trap, `div` or
///   `mod` by a non-literal);
/// * every value defined in the loop and used after it is a polynomial of degree <= 4 in the
///   iteration number (header phis that add a polynomial to themselves, and anything built
///   from them with add, sub, mul, shl by a literal);
/// * the exit test compares two values that are linear in the iteration number with step
///   +-1 (or an odd step for `eq`), which gives the number of iterations T exactly, by the
///   signed `lt`: `T = n - i0` if `i0 <s n`, else 0.
/// The closed form is computed before the loop; the loop is cut off (header jumps to the exit).
/// Exact for all inputs: the polynomial identities hold in Z/2^32, the binomial coefficients
/// are computed without dividing a wrapped value by an even number (see `peval`).
pub fn closed_forms(f: &mut Func, rng: &Ranges) -> bool {
    if !rng.ok {
        return false;
    }
    let _ = analyze(f);
    let nv0 = f.nval.max(0) as usize + 1;
    let mut rep = vec![NOREP; nv0];
    let mut changed = false;
    for li in 0..f.loops.len() {
        if try_loop(f, rng, li, &mut rep) {
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

const MAXLOOP: usize = 120;

fn try_loop(f: &mut Func, rng: &Ranges, li: usize, rep: &mut Vec<i64>) -> bool {
    let h = f.loops[li].header;
    let ids = f.loops[li].ids.clone();
    let nblocks = f.loops[li].blocks.len();
    let preds = f.arena[h].preds.clone();
    if preds.len() != 2 {
        return false;
    }
    let po = match preds.iter().position(|p| !ids.contains(p)) {
        Some(i) => i,
        None => return false,
    };
    let pi = 1 - po;
    if !ids.contains(&preds[pi]) {
        return false;
    }
    let p0 = preds[po];
    {
        let t = f.arena[p0].term();
        if t.op != Op::Jmp || t.t.len() != 1 || t.t[0] != h {
            return false;
        }
    }
    let ht = f.arena[h].term().clone();
    if ht.op != Op::Br || ht.a[0] < 0 || ht.t[0] == ht.t[1] {
        return false;
    }
    let (in_idx, cont_true) = if ids.contains(&ht.t[0]) && !ids.contains(&ht.t[1]) {
        (0, true)
    } else if ids.contains(&ht.t[1]) && !ids.contains(&ht.t[0]) {
        (1, false)
    } else {
        return false;
    };
    let body_in = ht.t[in_idx];
    let exit = ht.t[1 - in_idx];
    if body_in == h {
        return false;
    }
    // the chain of blocks h -> body_in -> ... -> h
    let mut chain: Vec<usize> = Vec::new();
    let mut b = body_in;
    while b != h {
        if !ids.contains(&b) || chain.len() > nblocks || chain.contains(&b) {
            return false;
        }
        let blk = &f.arena[b];
        if !blk.phis.is_empty() || blk.term().op != Op::Jmp || blk.preds.len() != 1 {
            return false;
        }
        chain.push(b);
        b = blk.term().t[0];
    }
    if chain.len() + 1 != nblocks || *chain.last().unwrap() != preds[pi] {
        return false;
    }
    // instructions
    let mut order: Vec<Instr> = Vec::new();
    let mut defined: HashSet<i64> = HashSet::new();
    for phi in &f.arena[h].phis {
        defined.insert(phi.d);
    }
    for &blk in std::iter::once(&h).chain(chain.iter()) {
        let ins_all = &f.arena[blk].ins;
        for ins in &ins_all[..ins_all.len() - 1] {
            if !harmless(ins) {
                return false;
            }
            defined.insert(ins.d);
            order.push(ins.clone());
        }
    }
    if order.len() > MAXLOOP || f.arena[h].phis.len() > 16 {
        return false;
    }
    // values defined in the loop and used outside
    let mut live: Vec<i64> = Vec::new();
    for &blk in &f.blocks {
        if ids.contains(&blk) {
            continue;
        }
        let bl = &f.arena[blk];
        for ins in bl.phis.iter().chain(bl.ins.iter()) {
            for &x in &ins.a {
                if x >= 0 && defined.contains(&x) && !live.contains(&x) {
                    live.push(x);
                }
            }
        }
    }
    // only header values can be live after the loop (the exit is reached from the header)
    let hdef: HashSet<i64> = f.arena[h].phis.iter().map(|p| p.d).chain(f.arena[h].ins.iter().filter(|i| i.has_d()).map(|i| i.d)).collect();
    if live.iter().any(|x| !hdef.contains(x)) {
        return false;
    }
    let mut em = Em::new(f.nval);
    let mut cps: HashMap<i64, Option<Cp>> = HashMap::new();
    let mut solved: HashSet<i64> = HashSet::new();
    let hp: Vec<(i64, i64, i64)> = f.arena[h].phis.iter().map(|p| (p.d, p.a[po], p.a[pi])).collect();
    if !live.is_empty() {
        for &(d, _, _) in &hp {
            cps.insert(d, Some(Cp { p: vec![ZERO], lin: vec![(d, 1)] }));
        }
        for _round in 0..=hp.len() {
            for ins in &order {
                if let Some(Some(c)) = cps.get(&ins.d) {
                    if c.lin.is_empty() {
                        continue;
                    }
                }
                let r = {
                    let get = |x: i64| -> Option<Cp> {
                        if x < 0 {
                            return Some(cp_const(x));
                        }
                        match cps.get(&x) {
                            Some(c) => c.clone(),
                            None => Some(cp_const(x)),
                        }
                    };
                    eval_ins(&mut em, ins, &get)
                };
                cps.insert(ins.d, r);
                if em.code.len() > 4 * MAXLOOP {
                    return false;
                }
            }
            let mut prog = false;
            for &(d, init, next) in &hp {
                if solved.contains(&d) {
                    continue;
                }
                let nc = if next < 0 {
                    Some(cp_const(next))
                } else {
                    match cps.get(&next) {
                        Some(c) => c.clone(),
                        None => Some(cp_const(next)),
                    }
                };
                if let Some(c) = nc {
                    if c.lin == vec![(d, 1)] {
                        if let Some(pl) = psum(mk(init, false), &c.p) {
                            cps.insert(d, Some(Cp { p: pl, lin: vec![] }));
                            solved.insert(d);
                            prog = true;
                        }
                    }
                }
            }
            if !prog {
                break;
            }
        }
    }
    // the trip count
    let mut outv: Vec<(i64, i64)> = Vec::new();
    if !live.is_empty() {
        let cond = match ht.a[0] {
            c if c >= 0 => f.arena[h].ins.iter().find(|i| i.d == c).cloned(),
            _ => None,
        };
        let cond = match cond {
            Some(c) if c.op == Op::Lt || c.op == Op::Eq => c,
            _ => return false,
        };
        let get = |x: i64, cps: &HashMap<i64, Option<Cp>>| -> Option<Cp> {
            if x < 0 {
                return Some(cp_const(x));
            }
            match cps.get(&x) {
                Some(c) => c.clone(),
                None => Some(cp_const(x)),
            }
        };
        let (ca, cb) = match (get(cond.a[0], &cps), get(cond.a[1], &cps)) {
            (Some(a), Some(b)) => (a, b),
            _ => return false,
        };
        let t = match trip_count(&mut em, rng, p0, &cond, &ca, &cb, cont_true) {
            Some(t) => t,
            None => return false,
        };
        for &x in &live {
            let c = match get(x, &cps) {
                Some(c) if c.lin.is_empty() => c,
                _ => return false,
            };
            let v = match peval(&mut em, &c.p, t) {
                Some(v) => v,
                None => return false,
            };
            outv.push((x, v));
        }
    }
    if em.code.len() > 6 * MAXLOOP {
        return false;
    }
    // commit
    let n = f.arena[p0].ins.len();
    let term = f.arena[p0].ins.pop().unwrap();
    debug_assert!(n >= 1);
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

// ---------------------------------------------------------------------------
// Division and remainder by a constant with a LOW multiply (no high half in KIR):
// floor(x / c) = (x * m) >> s for 0 <= x <= xmax when
//   m = ceil(2^s / c),  x*m < 2^32 (no wrap)  and  (m*c - 2^s) * xmax < 2^s (exact).
// ---------------------------------------------------------------------------
pub fn find_magic(c: u64, xmax: u64) -> Option<(u64, u32)> {
    if c < 2 || c.is_power_of_two() {
        return None;
    }
    for s in 1..=31u32 {
        let p = 1u128 << s;
        let m = (p + c as u128 - 1) / c as u128;
        if m >= 1u128 << 32 || (xmax as u128) * m >= 1u128 << 32 {
            return None;
        }
        let e = m * c as u128 - p;
        if e * (xmax as u128) < p {
            return Some((m as u64, s));
        }
    }
    None
}

/// Rewrite `div x, c` / `mod x, c` (c a literal, not a power of two) when the ranges prove
/// 0 <= x <= xmax (< 2^31) and a magic pair exists: q = (x*m)>>s, remainder x - q*c.
/// Replaces a `lat_div` operation by mul + shr (+ mul + sub); the search decides whether that wins.
pub fn div_const(f: &mut Func, rng: &Ranges) -> bool {
    if !rng.ok {
        return false;
    }
    let _ = analyze(f);
    let mut changed = false;
    for bi in 0..f.blocks.len() {
        let b = f.blocks[bi];
        let old = std::mem::take(&mut f.arena[b].ins);
        let mut out: Vec<Instr> = Vec::with_capacity(old.len() + 4);
        for ins in old {
            if (ins.op == Op::Div || ins.op == Op::Mod) && ins.a[1] < 0 && ins.a[0] >= 0 {
                let c = !ins.a[1] as u64;
                let iv = rng.get(ins.a[0], b);
                if c >= 2 && c <= SMAX as u64 && iv.1 <= SMAX {
                    if let Some((m, s)) = find_magic(c, iv.1 as u64) {
                        let t = f.new_val();
                        out.push(Instr::new(Op::Mul, t, vec![ins.a[0], lit(m as i64)], vec![]));
                        let q = if ins.op == Op::Div { ins.d } else { f.new_val() };
                        out.push(Instr::new(Op::Shr, q, vec![t, lit(s as i64)], vec![]));
                        if ins.op == Op::Mod {
                            let u = f.new_val();
                            out.push(Instr::new(Op::Mul, u, vec![q, lit(c as i64)], vec![]));
                            out.push(Instr::new(Op::Sub, ins.d, vec![ins.a[0], u], vec![]));
                        }
                        changed = true;
                        continue;
                    }
                }
            }
            out.push(ins);
        }
        f.arena[b].ins = out;
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn magic_division_exact() {
        for c in 2u64..400 {
            for &xmax in &[15u64, 255, 1000, 4095, 65535, 100000] {
                if let Some((m, s)) = find_magic(c, xmax) {
                    for x in 0..=xmax.min(70000) {
                        assert_eq!((x * m) >> s, x / c, "c={} xmax={} x={}", c, xmax, x);
                        assert!(x * m < 1 << 32);
                    }
                    if xmax > 70000 {
                        for x in (xmax - 5000)..=xmax {
                            assert_eq!((x * m) >> s, x / c);
                        }
                    }
                }
            }
        }
        assert!(find_magic(10, 65535).is_some());
        assert!(find_magic(10, 1 << 30).is_none());
    }

    fn binom_ref(k: u64, j: u32) -> u128 {
        let mut r: u128 = 1;
        for i in 0..j as u64 {
            r = r * (k as u128 - i as u128 * ((k >= i) as u128)) / (i as u128 + 1);
            if k < i + 1 {
                return 0;
            }
        }
        r
    }

    /// reference: sum_j c_j C(k,j) mod 2^32
    fn poly_ref(c: &[u32], k: u64) -> u32 {
        let mut s: u128 = 0;
        for (j, &cj) in c.iter().enumerate() {
            s = s.wrapping_add(cj as u128 * binom_ref(k, j as u32));
        }
        s as u32
    }

    fn eval_emitted(c: &[u32], k: u32) -> Option<u32> {
        let mut em = Em::new(2);
        let p: Poly = c.iter().map(|&x| cf_val_lit(x)).collect();
        let r = peval(&mut em, &p, 0)?; // value 0 is the parameter k
        let mut env = HashMap::new();
        env.insert(0i64, k);
        run_code(&em.code, &mut env);
        Some(if r < 0 { !r as u32 } else { env[&r] })
    }

    fn cf_val_lit(x: u32) -> Cf {
        // the parameter-free polynomials of the tests use literal coefficients
        cf_lit(x)
    }

    #[test]
    fn binomials_exact_8bit_analogue() {
        // the same formulas at 8 bits, for every k
        let m: u32 = 0xFF;
        let inv3: u32 = {
            let mut x = 3u32;
            for _ in 0..5 {
                x = x.wrapping_mul(2u32.wrapping_sub(3u32.wrapping_mul(x))) & m;
            }
            x & m
        };
        assert_eq!(inv3 * 3 & m, 1);
        for k in 0u32..256 {
            let c2 = ((k >> 1) * ((k.wrapping_sub(1) & m) | 1)) & m;
            assert_eq!(c2 as u128, binom_ref(k as u64, 2) % 256, "C2 k={}", k);
            let k2 = k.wrapping_sub(2) & m;
            let c3 = (c2 * k2 & m) * inv3 & m;
            assert_eq!(c3 as u128, binom_ref(k as u64, 3) % 256, "C3 k={}", k);
            let k3 = k.wrapping_sub(3) & m;
            let b = ((k2 >> 1) * ((k3) | 1)) & m;
            let c4x2 = ((c2 * b) & m) * inv3 & m;
            assert_eq!(c4x2 as u128, (2 * binom_ref(k as u64, 4)) % 256, "2*C4 k={}", k);
        }
    }

    #[test]
    fn peval_matches_reference() {
        let ks: Vec<u32> = (0..70u32).chain([0x7FFF_FFFF, 0x8000_0000, 0xFFFF_FFFF, 0xFFFF_FFFE, 1 << 20, 123456789, 0xDEAD_BEEF]).collect();
        let mut rnd = Rng::new(7);
        for _ in 0..300 {
            let deg = rnd.below(4) as usize + 1; // up to 5 coefficients
            let mut c: Vec<u32> = (0..=deg).map(|_| rnd.next() as u32).collect();
            if deg == 4 {
                c[4] &= !1; // even a4 only
            }
            for &k in &ks {
                let want = poly_ref(&c, k as u64);
                let got = eval_emitted(&c, k).unwrap();
                assert_eq!(got, want, "c={:?} k={}", c, k);
            }
        }
    }

    #[test]
    fn sums_of_powers() {
        // sum_{i<k} i^d  via the same machinery (d = 1, 2, 3)
        let i1 = vec![0u32, 1];
        let i2 = vec![0u32, 1, 2];
        let i3 = vec![0u32, 1, 6, 6];
        for (c, d) in [(i1, 1u32), (i2, 2), (i3, 3)] {
            let mut em = Em::new(0);
            let f: Poly = c.iter().map(|&x| cf_lit(x)).collect();
            let q = psum(cf_lit(0), &f).unwrap();
            let r = peval(&mut em, &q, lit(5)).unwrap();
            let want: u128 = (0..5u128).map(|i| i.pow(d)).sum();
            assert_eq!(!r as u128, want);
            for k in (0u32..80).chain([0x7FFF_FFFF, 0xFFFF_FFFF, 0x8000_0000]) {
                let mut em = Em::new(2);
                let r = peval(&mut em, &q, 0).unwrap();
                let mut env = HashMap::new();
                env.insert(0i64, k);
                run_code(&em.code, &mut env);
                let got = if r < 0 { !r as u32 } else { env[&r] };
                let mut want: u32 = 0;
                if k < 80 {
                    for i in 0..k {
                        want = want.wrapping_add(i.wrapping_pow(d));
                    }
                } else {
                    // closed forms in Z/2^32 with exact halving
                    let kk = k as u128;
                    let s = match d {
                        1 => kk * (kk - 1) / 2,
                        2 => kk * (kk - 1) * (2 * kk - 1) / 6,
                        _ => (kk * (kk - 1) / 2) * (kk * (kk - 1) / 2),
                    };
                    want = s as u32;
                }
                assert_eq!(got, want, "d={} k={}", d, k);
            }
        }
    }

    #[test]
    fn pmul_matches_reference() {
        let mut rnd = Rng::new(11);
        for _ in 0..200 {
            let da = rnd.below(3) as usize;
            let db = rnd.below(3) as usize;
            let a: Vec<u32> = (0..=da).map(|_| rnd.next() as u32).collect();
            let b: Vec<u32> = (0..=db).map(|_| rnd.next() as u32).collect();
            let mut em = Em::new(0);
            let pa: Poly = a.iter().map(|&x| cf_lit(x)).collect();
            let pb: Poly = b.iter().map(|&x| cf_lit(x)).collect();
            let pc = pmul(&mut em, &pa, &pb).unwrap();
            assert!(em.code.is_empty());
            let c: Vec<u32> = pc.iter().map(|x| !x.v as u32).collect();
            for k in 0u64..40 {
                let want = poly_ref(&a, k).wrapping_mul(poly_ref(&b, k));
                assert_eq!(poly_ref(&c, k), want, "a={:?} b={:?} k={}", a, b, k);
            }
        }
    }
}
