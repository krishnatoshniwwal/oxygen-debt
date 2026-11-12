//! domain.rs - known-bits ("tnum") analysis on SSA form.
//!
//! Every value gets a pair (v, m): the bits set in `m` are unknown, every other bit equals
//! the same bit of `v` (so v & m == 0). This is a bit-level domain; it also contains the
//! power-of-two congruences ("x is a multiple of 8" = low three bits known zero) and gives
//! an unsigned interval [v, v | m] and a sign. It is combined with the interval analysis
//! of ranges.rs: both are sound facts, so each can narrow the other.
//!
//! All transfer functions take the bit width `w` (32 in the program; 4 and 8 in the
//! exhaustive soundness tests) and agree with the concrete operations of the machine:
//! add/sub/mul modulo 2^w, shifts by (y mod w) (logical right shift), `lt` signed, `div`
//! and `mod` signed with truncation towards zero.
use crate::ir::*;
use crate::kir::*;
use crate::passes::{apply_rep, NOREP};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct T {
    pub v: u64,
    pub m: u64,
}

pub fn mk(w: u32) -> u64 {
    (1u64 << w) - 1
}

impl T {
    pub fn c(x: u64, w: u32) -> T {
        T { v: x & mk(w), m: 0 }
    }
    pub fn top(w: u32) -> T {
        T { v: 0, m: mk(w) }
    }
    pub fn is_const(&self) -> bool {
        self.m == 0
    }
    pub fn umin(&self) -> u64 {
        self.v
    }
    pub fn umax(&self) -> u64 {
        self.v | self.m
    }
    pub fn contains(&self, x: u64) -> bool {
        (x & !self.m) == self.v
    }
    /// least upper bound
    pub fn join(a: T, b: T) -> T {
        let mu = a.m | b.m | (a.v ^ b.v);
        T { v: a.v & !mu, m: mu }
    }
    /// bits that may be 1
    pub fn may1(&self) -> u64 {
        self.v | self.m
    }
    pub fn sign_known(&self, w: u32) -> Option<bool> {
        let s = 1u64 << (w - 1);
        if self.m & s != 0 {
            None
        } else {
            Some(self.v & s != 0)
        }
    }
}

fn sx(x: u64, w: u32) -> i64 {
    let s = 1u64 << (w - 1);
    ((x & mk(w)) ^ s) as i64 - s as i64
}

/// Concrete operation on w-bit values (None = traps). Used by the tests as the reference.
pub fn conc(op: Op, x: u64, y: u64, w: u32) -> Option<u64> {
    let m = mk(w);
    let (x, y) = (x & m, y & m);
    Some(match op {
        Op::Add => x.wrapping_add(y) & m,
        Op::Sub => x.wrapping_sub(y) & m,
        Op::Mul => x.wrapping_mul(y) & m,
        Op::And => x & y,
        Op::Or => x | y,
        Op::Xor => x ^ y,
        Op::Shl => (x << (y % w as u64)) & m,
        Op::Shr => x >> (y % w as u64),
        Op::Lt => (sx(x, w) < sx(y, w)) as u64,
        Op::Eq => (x == y) as u64,
        Op::Div | Op::Mod => {
            if y == 0 {
                return None;
            }
            let (a, b) = (sx(x, w), sx(y, w));
            let r = if op == Op::Div { a / b } else { a % b };
            (r as u64) & m
        }
        _ => panic!("conc"),
    })
}

fn add(a: T, b: T, w: u32) -> T {
    let m = mk(w);
    let sm = (a.m + b.m) & m;
    let sv = (a.v + b.v) & m;
    let sigma = (sm + sv) & m;
    let chi = sigma ^ sv;
    let mu = (chi | a.m | b.m) & m;
    T { v: sv & !mu, m: mu }
}

fn sub(a: T, b: T, w: u32) -> T {
    let m = mk(w);
    let dv = a.v.wrapping_sub(b.v) & m;
    let alpha = (dv + a.m) & m;
    let beta = dv.wrapping_sub(b.m) & m;
    let chi = alpha ^ beta;
    let mu = (chi | a.m | b.m) & m;
    T { v: dv & !mu, m: mu }
}

fn mul(a: T, b: T, w: u32) -> T {
    let m = mk(w);
    let (mut a, mut b) = (a, b);
    let accv = a.v.wrapping_mul(b.v) & m;
    let mut accm = T::c(0, w);
    while a.v != 0 || a.m != 0 {
        if a.v & 1 != 0 {
            accm = add(accm, T { v: 0, m: b.m }, w);
        } else if a.m & 1 != 0 {
            accm = add(accm, T { v: 0, m: (b.v | b.m) & m }, w);
        }
        a = T { v: a.v >> 1, m: a.m >> 1 };
        b = T { v: (b.v << 1) & m, m: (b.m << 1) & m };
    }
    add(T { v: accv, m: 0 }, accm, w)
}

fn shl_c(a: T, k: u64, w: u32) -> T {
    let m = mk(w);
    T { v: (a.v << k) & m, m: (a.m << k) & m }
}

fn shr_c(a: T, k: u64) -> T {
    T { v: a.v >> k, m: a.m >> k }
}

/// Smallest number of leading zero bits (in w bits) of any value in a.
fn lead_zeros_known(a: T, w: u32) -> u32 {
    let may = a.may1();
    if may == 0 {
        w
    } else {
        w - (64 - may.leading_zeros())
    }
}

fn high_zero_mask(lz: u32, w: u32) -> u64 {
    // mask of the top `lz` bits of a w-bit word
    if lz == 0 {
        0
    } else {
        mk(w) & !mk(w - lz)
    }
}

/// Transfer function of a pure binary op. Div/mod operands are such that a zero divisor
/// traps (no result), so the result only has to cover the non-trapping cases.
pub fn bin(op: Op, a: T, b: T, w: u32) -> T {
    let m = mk(w);
    if a.is_const() && b.is_const() {
        if let Some(r) = conc(op, a.v, b.v, w) {
            return T::c(r, w);
        }
        return T::top(w);
    }
    match op {
        Op::Add => add(a, b, w),
        Op::Sub => sub(a, b, w),
        Op::Mul => mul(a, b, w),
        Op::And => {
            let v = a.v & b.v;
            let may = a.may1() & b.may1();
            T { v, m: may & !v }
        }
        Op::Or => {
            let v = a.v | b.v;
            let may = a.may1() | b.may1();
            T { v, m: may & !v }
        }
        Op::Xor => {
            let m_ = a.m | b.m;
            T { v: (a.v ^ b.v) & !m_, m: m_ }
        }
        Op::Shl | Op::Shr => {
            if b.is_const() {
                let k = b.v % w as u64;
                if op == Op::Shl {
                    shl_c(a, k, w)
                } else {
                    shr_c(a, k)
                }
            } else {
                // amount unknown: it is (y mod w) only when w is a power of two, which it is.
                // every possible amount is < w; join over those the amount bits allow
                let amt = T { v: b.v & (w as u64 - 1), m: b.m & (w as u64 - 1) };
                let mut r: Option<T> = None;
                for k in 0..w as u64 {
                    if amt.contains(k) {
                        let t = if op == Op::Shl { shl_c(a, k, w) } else { shr_c(a, k) };
                        r = Some(match r {
                            None => t,
                            Some(x) => T::join(x, t),
                        });
                    }
                }
                r.unwrap_or(T::top(w))
            }
        }
        Op::Lt => {
            let s = 1u64 << (w - 1);
            let smin = |t: T| sx(t.v | (t.m & s), w);
            let smax = |t: T| sx(t.v | (t.m & !s), w);
            if smax(a) < smin(b) {
                T::c(1, w)
            } else if smin(a) >= smax(b) {
                T::c(0, w)
            } else {
                T { v: 0, m: 1 }
            }
        }
        Op::Eq => {
            if (a.v ^ b.v) & !(a.m | b.m) & m != 0 {
                T::c(0, w)
            } else {
                T { v: 0, m: 1 }
            }
        }
        Op::Div | Op::Mod => {
            let mut r = T::top(w);
            // y = 2^k constant
            if b.is_const() && b.v.is_power_of_two() {
                let k = b.v.trailing_zeros();
                if op == Op::Mod && a.m & mk(k) == 0 && a.v & mk(k) == 0 && k < w {
                    return T::c(0, w); // low k bits known zero: remainder 0 for any sign
                }
                if a.sign_known(w) == Some(false) && k < w {
                    return if op == Op::Mod { bin(Op::And, a, T::c(b.v - 1, w), w) } else { shr_c(a, k as u64) };
                }
            }
            // non-negative dividend and divisor: result in [0, dividend]
            if a.sign_known(w) == Some(false) && b.sign_known(w) == Some(false) {
                let lz = lead_zeros_known(a, w);
                r.m &= !high_zero_mask(lz, w);
                r.v &= !high_zero_mask(lz, w);
            }
            if op == Op::Mod && a.sign_known(w) == Some(false) {
                let lz = lead_zeros_known(a, w);
                r.m &= !high_zero_mask(lz, w);
            }
            r
        }
        _ => T::top(w),
    }
}

pub fn select(c: T, p: T, q: T) -> T {
    if c.v != 0 {
        p
    } else if c.m == 0 {
        q
    } else {
        T::join(p, q)
    }
}

// ---------------------------------------------------------------------------
// The analysis on a function
// ---------------------------------------------------------------------------
pub struct Bits {
    pub t: Vec<Option<T>>,
    pub ok: bool,
}

impl Bits {
    pub fn of(&self, x: i64) -> T {
        if x < 0 {
            return T::c((!x) as u64, 32);
        }
        self.t.get(x as usize).copied().flatten().unwrap_or(T::top(32))
    }

    pub fn run(f: &mut Func) -> Bits {
        analyze(f);
        let nv = f.nval.max(0) as usize + 1;
        let mut r = Bits { t: vec![None; nv], ok: false };
        for &p in &f.params {
            r.t[p as usize] = Some(T::top(32));
        }
        let get = |t: &Vec<Option<T>>, x: i64| -> Option<T> {
            if x < 0 {
                Some(T::c((!x) as u64, 32))
            } else {
                t[x as usize]
            }
        };
        let rpo = f.rpo.clone();
        for _round in 0..100 {
            let mut changed = false;
            for &b in &rpo {
                let blk = &f.arena[b];
                for phi in &blk.phis {
                    let mut acc: Option<T> = None;
                    for &x in &phi.a {
                        if let Some(t) = get(&r.t, x) {
                            acc = Some(match acc {
                                None => t,
                                Some(a) => T::join(a, t),
                            });
                        }
                    }
                    if let Some(a) = acc {
                        let n = match r.t[phi.d as usize] {
                            None => a,
                            Some(o) => T::join(o, a),
                        };
                        if r.t[phi.d as usize] != Some(n) {
                            r.t[phi.d as usize] = Some(n);
                            changed = true;
                        }
                    }
                }
                for ins in &blk.ins {
                    if !ins.has_d() {
                        continue;
                    }
                    let op = ins.op;
                    let new: Option<T> = match op {
                        Op::Select => match (get(&r.t, ins.a[0]), get(&r.t, ins.a[1]), get(&r.t, ins.a[2])) {
                            (Some(c), Some(p), Some(q)) => Some(select(c, p, q)),
                            _ => None,
                        },
                        _ if is_binop(op) => match (get(&r.t, ins.a[0]), get(&r.t, ins.a[1])) {
                            (Some(x), Some(y)) => Some(bin(op, x, y, 32)),
                            _ => None,
                        },
                        _ => Some(T::top(32)),
                    };
                    if let Some(a) = new {
                        let n = match r.t[ins.d as usize] {
                            None => a,
                            Some(o) => T::join(o, a),
                        };
                        if r.t[ins.d as usize] != Some(n) {
                            r.t[ins.d as usize] = Some(n);
                            changed = true;
                        }
                    }
                }
            }
            if !changed {
                r.ok = true;
                break;
            }
        }
        if !r.ok {
            for x in r.t.iter_mut() {
                *x = None;
            }
        }
        r
    }
}

/// Rewrites justified by the known bits: constants, `and`/`or` that change nothing.
/// Returns true if the function changed.
pub fn bits_simplify(f: &mut Func) -> bool {
    let bits = Bits::run(f);
    if !bits.ok {
        return false;
    }
    let nv = bits.t.len();
    let mut rep = vec![NOREP; nv];
    let mut changed = false;
    let blocks = f.blocks.clone();
    for &b in &blocks {
        let old = std::mem::take(&mut f.arena[b].ins);
        let mut out = Vec::with_capacity(old.len());
        for ins in old {
            if ins.has_d() {
                let d = ins.d as usize;
                let t = bits.of(ins.d);
                if t.is_const() && is_pure(ins.op) && !matches!(ins.op, Op::Mac | Op::Addsh) {
                    rep[d] = lit(t.v as i64);
                    changed = true;
                    continue;
                }
                let mut ident: Option<i64> = None;
                if ins.op == Op::And {
                    let (x, y) = (bits.of(ins.a[0]), bits.of(ins.a[1]));
                    if x.may1() & !y.v == 0 {
                        ident = Some(ins.a[0]);
                    } else if y.may1() & !x.v == 0 {
                        ident = Some(ins.a[1]);
                    }
                } else if ins.op == Op::Or {
                    let (x, y) = (bits.of(ins.a[0]), bits.of(ins.a[1]));
                    if y.may1() & !x.v == 0 {
                        ident = Some(ins.a[0]);
                    } else if x.may1() & !y.v == 0 {
                        ident = Some(ins.a[1]);
                    }
                }
                if let Some(x) = ident {
                    if x != ins.d {
                        rep[d] = x;
                        changed = true;
                        continue;
                    }
                }
            }
            out.push(ins);
        }
        f.arena[b].ins = out;
    }
    if changed {
        apply_rep(f, &rep);
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    const OPS: [Op; 12] = [Op::Add, Op::Sub, Op::Mul, Op::And, Op::Or, Op::Xor, Op::Shl, Op::Shr, Op::Lt, Op::Eq, Op::Div, Op::Mod];

    fn members(t: T, w: u32) -> Vec<u64> {
        (0..=mk(w)).filter(|&x| t.contains(x)).collect()
    }

    fn all_tnums(w: u32) -> Vec<T> {
        let mut out = Vec::new();
        let n = 3u64.pow(w);
        for mut code in 0..n {
            let (mut v, mut m) = (0u64, 0u64);
            for bit in 0..w {
                match code % 3 {
                    1 => v |= 1 << bit,
                    2 => m |= 1 << bit,
                    _ => {}
                }
                code /= 3;
            }
            out.push(T { v, m });
        }
        out
    }

    fn check(op: Op, a: T, b: T, w: u32) {
        let r = bin(op, a, b, w);
        assert_eq!(r.v & r.m, 0, "invariant {:?} {:?} {:?}", op, a, b);
        for x in members(a, w) {
            for y in members(b, w) {
                if let Some(c) = conc(op, x, y, w) {
                    assert!(r.contains(c), "{:?} w={} a={:?} b={:?} x={} y={} conc={} abs={:?}", op, w, a, b, x, y, c, r);
                }
            }
        }
    }

    #[test]
    fn exhaustive_4bit() {
        let ts = all_tnums(4);
        for &op in &OPS {
            for &a in &ts {
                for &b in &ts {
                    check(op, a, b, 4);
                }
            }
        }
    }

    #[test]
    fn exhaustive_8bit_sampled() {
        // every tnum of 8 bits against a fixed spread of the other operand
        let ts = all_tnums(8);
        let mut other: Vec<T> = Vec::new();
        let mut x: u64 = 88172645463325252;
        for _ in 0..40 {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            let m = x & 0xFF;
            let v = (x >> 8) & 0xFF & !m;
            other.push(T { v, m });
        }
        for c in [0u64, 1, 2, 4, 8, 7, 255, 128, 3] {
            other.push(T::c(c, 8));
        }
        for &op in &OPS {
            for &a in &ts {
                for &b in &other {
                    check(op, a, b, 8);
                    check(op, b, a, 8);
                }
            }
        }
    }

    #[test]
    fn random_32bit() {
        let mut x: u64 = 0x2545F4914F6CDD1D;
        let mut next = || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        for _ in 0..4000 {
            let w = 32;
            let mm = |r: u64| -> T {
                let m = match r % 4 {
                    0 => 0,
                    1 => r >> 8,
                    2 => (r >> 8) & (r >> 20),
                    _ => 0xFF << ((r >> 8) % 24),
                } & mk(w);
                T { v: (r >> 32) & !m & mk(w), m }
            };
            let (a, b) = (mm(next()), mm(next()));
            for &op in &OPS {
                let r = bin(op, a, b, w);
                assert_eq!(r.v & r.m, 0);
                for _ in 0..20 {
                    let pick = |t: T, rnd: u64| (t.v | (rnd & t.m)) & mk(w);
                    let (p, q) = (pick(a, next()), pick(b, next()));
                    if let Some(c) = conc(op, p, q, w) {
                        assert!(r.contains(c), "{:?} {:?} {:?} {} {}", op, a, b, p, q);
                    }
                }
            }
        }
    }

    #[test]
    fn join_and_select() {
        let ts = all_tnums(3);
        for &a in &ts {
            for &b in &ts {
                let j = T::join(a, b);
                for x in members(a, 3).into_iter().chain(members(b, 3)) {
                    assert!(j.contains(x));
                }
                for &c in &ts {
                    let s = select(c, a, b);
                    for cv in members(c, 3) {
                        let pick = if cv != 0 { a } else { b };
                        for x in members(pick, 3) {
                            assert!(s.contains(x));
                        }
                    }
                }
            }
        }
    }
}
