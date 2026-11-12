//! ranges.rs - value-range analysis on SSA form.
//!
//! Every value gets an unsigned interval [lo, hi] that is true on every run. Branch
//! conditions refine the interval inside the blocks that the branch guards. The result is
//! used to prove that a load / store / div / mod cannot trap, that a value is not negative,
//! and to fold comparisons.
use crate::ir::*;
use crate::kir::*;
use std::collections::HashMap;

pub const SMAX: i64 = 0x7FFF_FFFF;
pub type Iv = (i64, i64);
pub const FULL: Iv = (0, MASK);
const TWO32: i64 = 1 << 32;

fn bitcap(x: i64) -> i64 {
    if x <= 0 {
        0
    } else {
        (1i64 << (64 - x.leading_zeros())) - 1
    }
}

type Cons = Vec<(i64, Iv)>;

fn put(c: &mut Cons, k: i64, v: Iv) {
    for e in c.iter_mut() {
        if e.0 == k {
            e.1 = v;
            return;
        }
    }
    c.push((k, v));
}

fn cget(c: &Cons, k: i64) -> Option<Iv> {
    c.iter().find(|e| e.0 == k).map(|e| e.1)
}

pub struct Ranges {
    pub base: Vec<Option<Iv>>,
    cons: Vec<Cons>,
    cup: Vec<Option<usize>>,
    pub defs: Vec<Option<DefInfo>>,
    defblk: Vec<Option<usize>>,
    idom: Vec<usize>,
    pub ok: bool,
    edge_cache: HashMap<(usize, usize), Cons>,
    order: Vec<usize>,
    entry: usize,
    // CFG snapshot used by the walks
    preds: Vec<Vec<usize>>,
    term: Vec<Option<(Op, i64, Vec<usize>)>>,
    // widening thresholds: m - 1 for every `mod` by a constant m (sorted)
    thresholds: Vec<i64>,
}

impl Ranges {
    pub fn new(f: &mut Func, max_passes: usize) -> Ranges {
        analyze(f);
        let nb = f.arena.len();
        let nv = f.nval.max(0) as usize + 1;
        let mut r = Ranges {
            base: vec![None; nv],
            cons: vec![Vec::new(); nb],
            cup: vec![None; nb],
            defs: vec![None; nv],
            defblk: vec![None; nv],
            idom: vec![0; nb],
            ok: false,
            edge_cache: HashMap::new(),
            order: Vec::new(),
            entry: f.blocks[0],
            preds: vec![Vec::new(); nb],
            term: vec![None; nb],
            thresholds: Vec::new(),
        };
        for &b in &f.blocks {
            r.idom[b] = f.arena[b].idom.unwrap_or(b);
            r.preds[b] = f.arena[b].preds.clone();
            let t = f.arena[b].term();
            r.term[b] = Some((t.op, t.a.first().copied().unwrap_or(L0), t.t.clone()));
        }
        r.compute(f, max_passes);
        r
    }

    // ------------------------------------------------------------------
    /// Interval of operand x when it is used in block b (None = not computed yet).
    pub fn at(&self, x: i64, b: usize) -> Option<Iv> {
        if x < 0 {
            return Some((!x, !x));
        }
        let r = self.base.get(x as usize).copied().flatten()?;
        let mut blk = match self.cup.get(b).copied().flatten() {
            None => return Some(r),
            Some(k) => k,
        };
        let db = self.defblk.get(x as usize).copied().flatten();
        let (mut lo, mut hi) = r;
        let mut n = 0;
        loop {
            if Some(blk) == db || n >= 24 {
                break;
            }
            if let Some(c) = cget(&self.cons[blk], x) {
                let l2 = if c.0 > lo { c.0 } else { lo };
                let h2 = if c.1 < hi { c.1 } else { hi };
                if l2 <= h2 {
                    lo = l2;
                    hi = h2;
                }
            }
            let p = self.idom[blk];
            if p == blk {
                break;
            }
            match self.cup.get(p).copied().flatten() {
                Some(k) => blk = k,
                None => break,
            }
            n += 1;
        }
        Some((lo, hi))
    }

    pub fn get(&self, x: i64, b: usize) -> Iv {
        self.at(x, b).unwrap_or(FULL)
    }

    pub fn may_trap(&self, ins: &Instr, b: usize) -> bool {
        match ins.op {
            Op::Load | Op::Store => {
                if !self.ok {
                    return true;
                }
                self.get(ins.a[0], b).1 >= 4096
            }
            Op::Div | Op::Mod => {
                let y = ins.a[1];
                if y < 0 {
                    return !y == 0;
                }
                if !self.ok {
                    return true;
                }
                self.get(y, b).0 < 1
            }
            op => op == Op::Trap,
        }
    }

    // ------------------------------------------------------------------
    /// Constraints that hold when the branch on c in block p goes the `pol` way.
    fn derive(&self, p: usize, c: i64, pol: bool) -> Cons {
        let mut out: Cons = Vec::new();
        if c < 0 {
            return out;
        }
        if let Some(rc) = self.at(c, p) {
            if pol {
                if rc.0 == 0 && rc.1 >= 1 {
                    put(&mut out, c, (1, rc.1));
                }
            } else if rc.0 == 0 {
                put(&mut out, c, (0, 0));
            }
        }
        let dc = match self.defs.get(c as usize).copied().flatten() {
            Some(d) => d,
            None => return out,
        };
        let op = dc.op;
        if op != Op::Lt && op != Op::Eq {
            return out;
        }
        let (a, b) = (dc.a0, dc.a1);
        let (ra, rb) = match (self.at(a, p), self.at(b, p)) {
            (Some(x), Some(y)) => (x, y),
            _ => return out,
        };
        let mut na: Option<Iv> = None;
        let mut nb: Option<Iv> = None;
        if op == Op::Lt {
            if pol {
                if ra.1 <= SMAX {
                    let nbv = (rb.0.max(ra.0 + 1), rb.1.min(SMAX));
                    na = Some((ra.0, ra.1.min(nbv.1 - 1)));
                    nb = Some(nbv);
                }
            } else if rb.1 <= SMAX {
                na = Some((ra.0.max(rb.0), ra.1.min(SMAX)));
                if ra.1 <= SMAX {
                    nb = Some((rb.0, rb.1.min(ra.1)));
                }
            }
        } else if pol {
            let lo = ra.0.max(rb.0);
            let hi = ra.1.min(rb.1);
            na = Some((lo, hi));
            nb = Some((lo, hi));
        } else {
            if rb.0 == rb.1 {
                if ra.0 == rb.0 {
                    na = Some((ra.0 + 1, ra.1));
                } else if ra.1 == rb.0 {
                    na = Some((ra.0, ra.1 - 1));
                }
            }
            if ra.0 == ra.1 {
                if rb.0 == ra.0 {
                    nb = Some((rb.0 + 1, rb.1));
                } else if rb.1 == ra.0 {
                    nb = Some((rb.0, rb.1 - 1));
                }
            }
        }
        if let Some(v) = na {
            if a >= 0 && v.0 <= v.1 {
                put(&mut out, a, v);
            }
        }
        if let Some(v) = nb {
            if b >= 0 && v.0 <= v.1 {
                put(&mut out, b, v);
            }
        }
        out
    }

    pub fn xfer(&self, ins: &Instr, b: usize) -> Option<Iv> {
        let op = ins.op;
        let a = &ins.a;
        if op == Op::Load || op == Op::Vload {
            return Some(FULL);
        }
        if op == Op::Select {
            let rc = self.at(a[0], b);
            let rp = self.at(a[1], b);
            let rq = self.at(a[2], b);
            let rc = rc?;
            if rc.0 >= 1 {
                return rp;
            }
            if rc.1 == 0 {
                return rq;
            }
            return match (rp, rq) {
                (None, _) => rq,
                (_, None) => rp,
                (Some(p), Some(q)) => Some((p.0.min(q.0), p.1.max(q.1))),
            };
        }
        let ra = self.at(a[0], b);
        let rb = self.at(*a.get(1).unwrap_or(&L0), b);
        let (ra, rb) = match (ra, rb) {
            (Some(x), Some(y)) => (x, y),
            _ => return None,
        };
        let (la, ha) = ra;
        let (lb, hb) = rb;
        Some(match op {
            Op::Add => {
                let (lo, hi) = (la + lb, ha + hb);
                if hi <= MASK {
                    (lo, hi)
                } else if lo > MASK {
                    (lo - TWO32, hi - TWO32)
                } else {
                    FULL
                }
            }
            Op::Sub => {
                let (lo, hi) = (la - hb, ha - lb);
                if lo >= 0 {
                    (lo, hi)
                } else if hi < 0 {
                    (lo + TWO32, hi + TWO32)
                } else {
                    FULL
                }
            }
            Op::Mul => {
                if (ha as i128) * (hb as i128) <= MASK as i128 {
                    (la * lb, ha * hb)
                } else {
                    FULL
                }
            }
            Op::And => (0, ha.min(hb)),
            Op::Or => (la.max(lb), bitcap(ha.max(hb))),
            Op::Xor => (0, bitcap(ha.max(hb))),
            Op::Shl => {
                if lb == hb {
                    let k = lb & 31;
                    if (ha << k) <= MASK {
                        return Some((la << k, ha << k));
                    }
                }
                FULL
            }
            Op::Shr => {
                if lb == hb {
                    let k = lb & 31;
                    (la >> k, ha >> k)
                } else {
                    (0, ha)
                }
            }
            Op::Lt => {
                if ha <= SMAX && hb <= SMAX {
                    if ha < lb {
                        return Some((1, 1));
                    }
                    if la >= hb {
                        return Some((0, 0));
                    }
                }
                (0, 1)
            }
            Op::Eq => {
                if ha < lb || hb < la {
                    (0, 0)
                } else if la == ha && lb == hb && la == lb {
                    (1, 1)
                } else {
                    (0, 1)
                }
            }
            Op::Div => {
                if ha <= SMAX && hb <= SMAX && lb >= 1 {
                    (la / hb, ha / lb)
                } else {
                    FULL
                }
            }
            Op::Mod => {
                if ha <= SMAX {
                    if hb <= SMAX && hb >= 1 {
                        (0, ha.min(hb - 1))
                    } else {
                        (0, ha)
                    }
                } else {
                    FULL
                }
            }
            _ => FULL,
        })
    }

    /// Constraints on the CFG edge p -> s (cached per pass).
    fn edge(&mut self, p: usize, s: usize) -> Cons {
        let (op, c, t) = match &self.term[p] {
            Some(x) => (x.0, x.1, x.2.clone()),
            None => return Vec::new(),
        };
        if op != Op::Br || t[0] == t[1] {
            return Vec::new();
        }
        if let Some(r) = self.edge_cache.get(&(p, s)) {
            return r.clone();
        }
        let r = self.derive(p, c, t[0] == s);
        self.edge_cache.insert((p, s), r.clone());
        r
    }

    fn phi_range(&mut self, phi: &Instr, b: usize) -> Option<Iv> {
        let mut lo = 0;
        let mut hi = 0;
        let mut have = false;
        let npreds = self.preds[b].len();
        for i in 0..npreds.min(phi.a.len()) {
            let x = phi.a[i];
            let p = self.preds[b][i];
            let mut r = match self.at(x, p) {
                Some(r) => r,
                None => continue,
            };
            if x >= 0 {
                let ec = self.edge(p, b);
                if let Some(c) = cget(&ec, x) {
                    let l2 = r.0.max(c.0);
                    let h2 = r.1.min(c.1);
                    if l2 <= h2 {
                        r = (l2, h2);
                    }
                }
            }
            if !have {
                lo = r.0;
                hi = r.1;
                have = true;
            } else {
                if r.0 < lo {
                    lo = r.0;
                }
                if r.1 > hi {
                    hi = r.1;
                }
            }
        }
        if have {
            Some((lo, hi))
        } else {
            None
        }
    }

    fn update(&mut self, d: i64, new: Option<Iv>, narrowing: bool, counts: &mut Vec<u32>) -> bool {
        let new = match new {
            Some(n) => n,
            None => return false,
        };
        let du = d as usize;
        let old = self.base[du];
        if narrowing {
            let old = match old {
                Some(o) => o,
                None => return false,
            };
            let lo = new.0.max(old.0);
            let hi = new.1.min(old.1);
            if lo > hi || (lo, hi) == old {
                return false;
            }
            self.base[du] = Some((lo, hi));
            return true;
        }
        let old = match old {
            Some(o) => o,
            None => {
                self.base[du] = Some(new);
                return true;
            }
        };
        if new.0 >= old.0 && new.1 <= old.1 {
            return false;
        }
        let mut lo = new.0.min(old.0);
        let mut hi = new.1.max(old.1);
        counts[du] += 1;
        if counts[du] >= 3 {
            if lo < old.0 {
                lo = 0;
            }
            if hi > old.1 {
                // first widening: stop at the next threshold (m - 1 of a `mod` by m), so a
                // running value reduced by `% m` can be proven to stay in [0, m - 1]; the
                // walks only stop at a fixpoint, so any stop is sound
                let t = if counts[du] == 3 && hi <= SMAX {
                    self.thresholds.iter().copied().find(|&t| t >= hi)
                } else {
                    None
                };
                hi = match t {
                    Some(t) => t,
                    None => if hi <= SMAX { SMAX } else { MASK },
                };
            }
        }
        self.base[du] = Some((lo, hi));
        true
    }

    fn walk(&mut self, f: &Func, narrowing: bool, counts: &mut Vec<u32>) -> bool {
        let mut changed = false;
        self.edge_cache.clear();
        for oi in 0..self.order.len() {
            let b = self.order[oi];
            let mut cs: Cons = Vec::new();
            if self.preds[b].len() == 1 && b != self.entry {
                let p = self.preds[b][0];
                cs = self.edge(p, b);
            }
            let nonempty = !cs.is_empty();
            self.cons[b] = cs;
            if nonempty {
                self.cup[b] = Some(b);
            } else if self.idom[b] != b {
                self.cup[b] = self.cup[self.idom[b]];
            } else {
                self.cup[b] = None;
            }
            let blk = &f.arena[b];
            for phi in &blk.phis {
                let new = self.phi_range(phi, b);
                if self.update(phi.d, new, narrowing, counts) {
                    changed = true;
                }
            }
            for ins in &blk.ins {
                if !ins.has_d() {
                    continue;
                }
                let new = self.xfer(ins, b);
                if self.update(ins.d, new, narrowing, counts) {
                    changed = true;
                }
            }
        }
        changed
    }

    fn compute(&mut self, f: &Func, max_passes: usize) {
        let mut order = Vec::new();
        let mut stack = vec![f.blocks[0]];
        while let Some(b) = stack.pop() {
            order.push(b);
            for &c in f.arena[b].dkids.iter().rev() {
                stack.push(c);
            }
        }
        self.order = order.clone();
        for &p in &f.params {
            self.base[p as usize] = Some(FULL);
            self.defblk[p as usize] = None;
        }
        for &b in &order {
            for phi in &f.arena[b].phis {
                self.defs[phi.d as usize] = Some(DefInfo::of(phi));
                self.defblk[phi.d as usize] = Some(b);
            }
            for ins in &f.arena[b].ins {
                if ins.has_d() {
                    self.defs[ins.d as usize] = Some(DefInfo::of(ins));
                    self.defblk[ins.d as usize] = Some(b);
                }
                if ins.op == Op::Mod && ins.a[1] < 0 {
                    let m = !ins.a[1];
                    if m >= 1 && m - 1 < SMAX {
                        self.thresholds.push(m - 1);
                    }
                }
            }
        }
        self.thresholds.sort_unstable();
        self.thresholds.dedup();
        let mut counts = vec![0u32; self.base.len()];
        let mut stable = false;
        for _ in 0..max_passes {
            if !self.walk(f, false, &mut counts) {
                stable = true;
                break;
            }
        }
        if !stable {
            // give up: no information
            for x in self.base.iter_mut() {
                *x = None;
            }
            for c in self.cons.iter_mut() {
                c.clear();
            }
            for c in self.cup.iter_mut() {
                *c = None;
            }
            self.ok = false;
            return;
        }
        for &b in &order {
            for phi in &f.arena[b].phis {
                if self.base[phi.d as usize].is_none() {
                    self.base[phi.d as usize] = Some(FULL);
                }
            }
            for ins in &f.arena[b].ins {
                if ins.has_d() && self.base[ins.d as usize].is_none() {
                    self.base[ins.d as usize] = Some(FULL);
                }
            }
        }
        for _ in 0..2 {
            if !self.walk(f, true, &mut counts) {
                break;
            }
        }
        self.ok = true;
    }
}
