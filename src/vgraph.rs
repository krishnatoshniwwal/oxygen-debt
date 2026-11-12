//! vgraph.rs - a solver-free translation validator on a value graph.
//!
//! `prove(orig, cand)` symbolically executes both programs on the same hash-consed term graph
//! and tries to show that on every input they give the same result, the same volatile log,
//! the same data memory and the same trap. It works on programs without loops (after peeling
//! and unrolling many samples are loop-free; for the others it answers "not applicable").
//!
//! Terms. Registers hold terms; every term has ONE canonical form so that equal values get the
//! same id:
//! * ring terms (`add`, `sub`, `mul`, `shl` by a constant, `mac`, `addsh`) are polynomials
//!   modulo 2^32 in "atoms" (sum of coefficient * product of atoms): this is a complete
//!   decision procedure for the ring fragment;
//! * bitwise terms (`and`, `or`, `xor`) are algebraic normal forms over GF(2) with 32-bit
//!   coefficient masks (a canonical form: complete for pure bitwise identities, which is the
//!   same as the truth-table test);
//! * `lt`, `eq`, `shr`, `div`, `mod`, `select`, loads and `vload` results are atoms built from
//!   canonical operands, with constant folding and a few identities: sound but incomplete;
//! * a difference of two terms that is a *linear mixed boolean-arithmetic* expression
//!   (sum of constant * bitwise term) is decided by the 0/1-input theorem: f(x) = sum_j 2^j g(bits
//!   of x at position j), so f == 0 for all x iff g(b) == 0 (mod 2^32) for every b in {0,1}^n.
//!
//! Paths. Each program is explored path by path (branches and possible traps fork). A path is
//! described by the set of conditions it assumed. An original path is matched by a candidate path
//! whose conditions are a subset of its own (equal, or coarser as after if-conversion); the terms of
//! the candidate are simplified under the original path's conditions (`select` on a known
//! condition) and then compared: result or trap, volatile log, and the final memory. Any
//! unmatched path makes the answer `Unproved` (never "wrong"): the caller then falls back to
//! differential testing. A proof is sound: the candidate is deterministic, so an input that
//! satisfies an original path's conditions satisfies the matching candidate path's conditions
//! and follows exactly that path.
use crate::kir::*;
use std::collections::{BTreeMap, HashMap, HashSet};

pub type T = u32;

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
enum Node {
    Const(u32),
    Param(u32),
    Poly(Vec<(Vec<T>, u32)>),
    Bit(Vec<(Vec<T>, u32)>),
    Lt(T, T),
    Ltu(T, T),
    Eq(T, T),
    Shr(T, T),
    Shl(T, T),
    Select(T, T, T),
    Div(T, T),
    Mod(T, T),
    Load(Vec<(T, T)>, T),
    VLoad(u32, T),
    Raw(u8, T, T),
}

pub struct Arena {
    nodes: Vec<Node>,
    map: HashMap<Node, T>,
}

type Anf = BTreeMap<Vec<T>, u32>;

const BLOWUP: usize = 200;

impl Arena {
    pub fn new() -> Arena {
        Arena { nodes: Vec::new(), map: HashMap::new() }
    }
    fn intern(&mut self, n: Node) -> T {
        if let Some(&t) = self.map.get(&n) {
            return t;
        }
        let id = self.nodes.len() as T;
        self.nodes.push(n.clone());
        self.map.insert(n, id);
        id
    }
    pub fn cst(&mut self, c: u32) -> T {
        self.intern(Node::Const(c))
    }
    fn as_const(&self, t: T) -> Option<u32> {
        if let Node::Const(c) = self.nodes[t as usize] {
            Some(c)
        } else {
            None
        }
    }
    // ---- polynomials ----
    fn poly_of(&self, t: T) -> BTreeMap<Vec<T>, u32> {
        let mut m = BTreeMap::new();
        match &self.nodes[t as usize] {
            Node::Poly(p) => {
                for (k, c) in p {
                    m.insert(k.clone(), *c);
                }
            }
            Node::Const(c) => {
                if *c != 0 {
                    m.insert(vec![], *c);
                }
            }
            _ => {
                m.insert(vec![t], 1);
            }
        }
        m
    }
    fn mk_poly(&mut self, mut m: BTreeMap<Vec<T>, u32>) -> T {
        m.retain(|_, c| *c != 0);
        if m.is_empty() {
            return self.cst(0);
        }
        if m.len() == 1 {
            let (k, c) = m.iter().next().unwrap();
            if k.is_empty() {
                let c = *c;
                return self.cst(c);
            }
            if k.len() == 1 && *c == 1 {
                return k[0];
            }
        }
        let v: Vec<(Vec<T>, u32)> = m.into_iter().collect();
        self.intern(Node::Poly(v))
    }
    pub fn add(&mut self, a: T, b: T) -> T {
        let mut pa = self.poly_of(a);
        for (k, c) in self.poly_of(b) {
            let e = pa.entry(k).or_insert(0);
            *e = e.wrapping_add(c);
        }
        if pa.len() > BLOWUP {
            return self.intern(Node::Raw(0, a, b));
        }
        self.mk_poly(pa)
    }
    pub fn mul(&mut self, a: T, b: T) -> T {
        let (pa, pb) = (self.poly_of(a), self.poly_of(b));
        if pa.len() * pb.len() > BLOWUP * 4 {
            return self.intern(Node::Raw(1, a.min(b), a.max(b)));
        }
        let mut out: BTreeMap<Vec<T>, u32> = BTreeMap::new();
        for (ka, ca) in &pa {
            for (kb, cb) in &pb {
                let mut k = ka.clone();
                k.extend(kb.iter().copied());
                k.sort();
                let e = out.entry(k).or_insert(0);
                *e = e.wrapping_add(ca.wrapping_mul(*cb));
            }
        }
        if out.len() > BLOWUP {
            return self.intern(Node::Raw(1, a.min(b), a.max(b)));
        }
        self.mk_poly(out)
    }
    pub fn neg(&mut self, a: T) -> T {
        let m1 = self.cst(u32::MAX);
        self.mul(a, m1)
    }
    pub fn sub(&mut self, a: T, b: T) -> T {
        let nb = self.neg(b);
        self.add(a, nb)
    }
    // ---- bitwise (ANF) ----
    fn anf_of(&self, t: T) -> Anf {
        let mut m = Anf::new();
        match &self.nodes[t as usize] {
            Node::Bit(p) => {
                for (k, c) in p {
                    m.insert(k.clone(), *c);
                }
            }
            Node::Const(c) => {
                if *c != 0 {
                    m.insert(vec![], *c);
                }
            }
            _ => {
                m.insert(vec![t], u32::MAX);
            }
        }
        m
    }
    fn mk_bit(&mut self, mut m: Anf) -> T {
        m.retain(|_, c| *c != 0);
        if m.is_empty() {
            return self.cst(0);
        }
        if m.len() == 1 {
            let (k, c) = m.iter().next().unwrap();
            if k.is_empty() {
                let c = *c;
                return self.cst(c);
            }
            if k.len() == 1 && *c == u32::MAX {
                return k[0];
            }
        }
        let v: Vec<(Vec<T>, u32)> = m.into_iter().collect();
        self.intern(Node::Bit(v))
    }
    fn anf_xor(a: &Anf, b: &Anf) -> Anf {
        let mut o = a.clone();
        for (k, c) in b {
            let e = o.entry(k.clone()).or_insert(0);
            *e ^= *c;
        }
        o.retain(|_, c| *c != 0);
        o
    }
    fn anf_and(a: &Anf, b: &Anf) -> Anf {
        let mut o = Anf::new();
        for (ka, ca) in a {
            for (kb, cb) in b {
                let c = ca & cb;
                if c == 0 {
                    continue;
                }
                let mut k = ka.clone();
                k.extend(kb.iter().copied());
                k.sort();
                k.dedup();
                *o.entry(k).or_insert(0) ^= c;
            }
        }
        o.retain(|_, c| *c != 0);
        o
    }
    pub fn xor(&mut self, a: T, b: T) -> T {
        let r = Self::anf_xor(&self.anf_of(a), &self.anf_of(b));
        self.mk_bit(r)
    }
    pub fn and(&mut self, a: T, b: T) -> T {
        let (x, y) = (self.anf_of(a), self.anf_of(b));
        if x.len() * y.len() > BLOWUP * 4 {
            return self.intern(Node::Raw(2, a.min(b), a.max(b)));
        }
        let r = Self::anf_and(&x, &y);
        if r.len() > BLOWUP {
            return self.intern(Node::Raw(2, a.min(b), a.max(b)));
        }
        self.mk_bit(r)
    }
    pub fn or(&mut self, a: T, b: T) -> T {
        let (x, y) = (self.anf_of(a), self.anf_of(b));
        if x.len() * y.len() > BLOWUP * 4 {
            return self.intern(Node::Raw(3, a.min(b), a.max(b)));
        }
        let both = Self::anf_and(&x, &y);
        let r = Self::anf_xor(&Self::anf_xor(&x, &y), &both);
        if r.len() > BLOWUP {
            return self.intern(Node::Raw(3, a.min(b), a.max(b)));
        }
        self.mk_bit(r)
    }
    // ---- the rest ----
    pub fn lt(&mut self, a: T, b: T) -> T {
        if let (Some(x), Some(y)) = (self.as_const(a), self.as_const(b)) {
            return self.cst(((x as i32) < (y as i32)) as u32);
        }
        if a == b {
            return self.cst(0);
        }
        self.intern(Node::Lt(a, b))
    }
    pub fn ltu(&mut self, a: T, b: T) -> T {
        if let (Some(x), Some(y)) = (self.as_const(a), self.as_const(b)) {
            return self.cst((x < y) as u32);
        }
        if a == b {
            return self.cst(0);
        }
        if self.umax(a) < self.umin(b) {
            return self.cst(1);
        }
        self.intern(Node::Ltu(a, b))
    }
    pub fn eq(&mut self, a: T, b: T) -> T {
        if a == b {
            return self.cst(1);
        }
        let d = self.sub(a, b);
        if let Some(c) = self.as_const(d) {
            return self.cst((c == 0) as u32);
        }
        let nd = self.neg(d);
        self.intern(Node::Eq(d.min(nd), d.max(nd)))
    }
    pub fn shl(&mut self, a: T, b: T) -> T {
        if let Some(k) = self.as_const(b) {
            let k = k & 31;
            let c = self.cst(1u32 << k);
            return self.mul(a, c);
        }
        self.intern(Node::Shl(a, b))
    }
    pub fn shr(&mut self, a: T, b: T) -> T {
        if let Some(k) = self.as_const(b) {
            let k = k & 31;
            if k == 0 {
                return a;
            }
            if let Some(x) = self.as_const(a) {
                return self.cst(x >> k);
            }
            let kc = self.cst(k);
            return self.intern(Node::Shr(a, kc));
        }
        self.intern(Node::Shr(a, b))
    }
    pub fn select(&mut self, c: T, a: T, b: T) -> T {
        if let Some(x) = self.as_const(c) {
            return if x != 0 { a } else { b };
        }
        if a == b {
            return a;
        }
        self.intern(Node::Select(c, a, b))
    }
    pub fn div(&mut self, a: T, b: T) -> T {
        if let (Some(x), Some(y)) = (self.as_const(a), self.as_const(b)) {
            if y != 0 {
                return self.cst(sdiv(x, y, true));
            }
        }
        if self.as_const(b) == Some(1) {
            return a;
        }
        self.intern(Node::Div(a, b))
    }
    pub fn modulo(&mut self, a: T, b: T) -> T {
        if let (Some(x), Some(y)) = (self.as_const(a), self.as_const(b)) {
            if y != 0 {
                return self.cst(sdiv(x, y, false));
            }
        }
        if self.as_const(b) == Some(1) {
            return self.cst(0);
        }
        self.intern(Node::Mod(a, b))
    }
    // ---- facts about terms ----
    pub fn umax(&self, t: T) -> u32 {
        match &self.nodes[t as usize] {
            Node::Const(c) => *c,
            Node::Bit(p) => p.iter().fold(0u32, |a, (_, c)| a | c),
            Node::Lt(..) | Node::Ltu(..) | Node::Eq(..) => 1,
            Node::Shr(a, k) => match self.nodes[*k as usize] {
                Node::Const(k) => self.umax(*a) >> k,
                _ => u32::MAX,
            },
            Node::Mod(a, b) => match self.nodes[*b as usize] {
                Node::Const(c) if c > 0 && c < 0x8000_0000 && self.umax(*a) < 0x8000_0000 => self.umax(*a).min(c - 1),
                _ => u32::MAX,
            },
            Node::Select(_, a, b) => self.umax(*a).max(self.umax(*b)),
            _ => u32::MAX,
        }
    }
    pub fn umin(&self, t: T) -> u32 {
        if let Node::Const(c) = self.nodes[t as usize] {
            c
        } else {
            0
        }
    }
    pub fn nonzero(&self, t: T) -> bool {
        match &self.nodes[t as usize] {
            Node::Const(c) => *c != 0,
            Node::Bit(p) => {
                // some bit j whose function is the constant 1
                for j in 0..32 {
                    let bit = 1u32 << j;
                    let mut ones = 0;
                    let mut other = false;
                    for (k, c) in p {
                        if c & bit != 0 {
                            if k.is_empty() {
                                ones += 1;
                            } else {
                                other = true;
                            }
                        }
                    }
                    if ones == 1 && !other {
                        return true;
                    }
                }
                false
            }
            _ => false,
        }
    }
    fn distinct(&mut self, a: T, b: T) -> bool {
        if let (Some(x), Some(y)) = (self.as_const(a), self.as_const(b)) {
            return x != y;
        }
        let d = self.sub(a, b);
        if let Some(c) = self.as_const(d) {
            return c != 0;
        }
        if let Some(y) = self.as_const(b) {
            if self.umax(a) < y {
                return true;
            }
        }
        if let Some(x) = self.as_const(a) {
            if self.umax(b) < x {
                return true;
            }
        }
        false
    }
}

fn sdiv(x: u32, y: u32, div: bool) -> u32 {
    let sx = x as i32 as i64;
    let sy = y as i32 as i64;
    if div {
        let mut q = sx.abs() / sy.abs();
        if (sx < 0) != (sy < 0) {
            q = -q;
        }
        q as u32
    } else {
        let mut m = sx.abs() % sy.abs();
        if sx < 0 {
            m = -m;
        }
        m as u32
    }
}

// ---------------------------------------------------------------------------
// Symbolic execution
// ---------------------------------------------------------------------------
#[derive(Clone, PartialEq, Eq, Debug)]
enum Eff {
    VLoad(T, T),
    VStore(T, T),
}

#[derive(Clone, PartialEq, Eq, Debug)]
enum End {
    Ret(T),
    Explicit(T),
    DivZero,
    BadAddr,
}

#[derive(Clone)]
struct State {
    regs: Vec<T>,
    mem: Vec<(T, T)>,
    effs: Vec<Eff>,
    conds: Vec<(T, bool)>,
    nvl: u32,
    visited: HashSet<usize>,
}

struct PathRes {
    conds: Vec<(T, bool)>,
    effs: Vec<Eff>,
    mem: Vec<(T, T)>,
    end: End,
}

pub enum Verdict {
    Proved,
    Unproved(String),
    NotApplicable(String),
}

const MAX_PATHS: usize = 3000;

fn fact(st: &State, c: T) -> Option<bool> {
    st.conds.iter().find(|(t, _)| *t == c).map(|(_, b)| *b)
}

fn explore(ar: &mut Arena, prog: &Program) -> Result<Vec<PathRes>, String> {
    let mut dense: HashMap<i64, usize> = HashMap::new();
    for &p in &prog.params {
        let n = dense.len();
        dense.entry(p).or_insert(n);
    }
    for ins in &prog.instrs {
        for a in &ins.args {
            if let Arg::R(r) = a {
                let n = dense.len();
                dense.entry(*r).or_insert(n);
            }
        }
    }
    let zero = ar.cst(0);
    let mut regs = vec![zero; dense.len()];
    for (i, p) in prog.params.iter().enumerate() {
        regs[dense[p]] = ar.intern(Node::Param(i as u32));
    }
    let mut out: Vec<PathRes> = Vec::new();
    let mut stack: Vec<(usize, State)> = vec![(0, State { regs, mem: vec![], effs: vec![], conds: vec![], nvl: 0, visited: HashSet::new() })];
    let mut steps = 0usize;
    while let Some((mut pc, mut st)) = stack.pop() {
        if out.len() + stack.len() > MAX_PATHS {
            return Err("too many paths".into());
        }
        'path: loop {
            steps += 1;
            if steps > 3_000_000 {
                return Err("too much work".into());
            }
            if pc >= prog.instrs.len() {
                return Err("ran off the end".into());
            }
            // block entry: loop detection
            if pc == 0 || prog.block_of[pc] != prog.block_of[pc - 1] {
                if !st.visited.insert(pc) {
                    return Err("loop".into());
                }
            }
            let ins = &prog.instrs[pc];
            let val = |ar: &mut Arena, st: &State, a: &Arg| -> T {
                match a {
                    Arg::R(r) => st.regs[dense[r]],
                    Arg::I(v) | Arg::K(v) => ar.cst(*v as u32),
                    Arg::L(_) => 0,
                }
            };
            let dreg = |a: &Arg| -> usize {
                if let Arg::R(r) = a {
                    dense[r]
                } else {
                    0
                }
            };
            // a condition that may end the path: fork `cond` (nonzero = continue / trap)
            macro_rules! guard {
                ($ok_cond:expr, $trap_when_false:expr, $end:expr) => {{
                    let c: T = $ok_cond;
                    let known = if let Node::Const(v) = ar.nodes[c as usize] { Some(v != 0) } else { fact(&st, c) };
                    match known {
                        Some(true) => {}
                        Some(false) => {
                            let mut conds = st.conds.clone();
                            conds.sort();
                            out.push(PathRes { conds, effs: st.effs.clone(), mem: st.mem.clone(), end: $end });
                            break 'path;
                        }
                        None => {
                            let _ = $trap_when_false;
                            let mut t = st.clone();
                            t.conds.push((c, false));
                            let mut conds = t.conds.clone();
                            conds.sort();
                            out.push(PathRes { conds, effs: t.effs.clone(), mem: t.mem.clone(), end: $end });
                            st.conds.push((c, true));
                        }
                    }
                }};
            }
            match ins.op {
                Op::Const | Op::Mov => {
                    let v = val(ar, &st, &ins.args[1]);
                    st.regs[dreg(&ins.args[0])] = v;
                }
                Op::Add | Op::Sub | Op::Mul | Op::And | Op::Or | Op::Xor | Op::Shl | Op::Shr | Op::Lt | Op::Eq => {
                    let (a, b) = (val(ar, &st, &ins.args[1]), val(ar, &st, &ins.args[2]));
                    let r = match ins.op {
                        Op::Add => ar.add(a, b),
                        Op::Sub => ar.sub(a, b),
                        Op::Mul => ar.mul(a, b),
                        Op::And => ar.and(a, b),
                        Op::Or => ar.or(a, b),
                        Op::Xor => ar.xor(a, b),
                        Op::Shl => ar.shl(a, b),
                        Op::Shr => ar.shr(a, b),
                        Op::Lt => ar.lt(a, b),
                        _ => ar.eq(a, b),
                    };
                    st.regs[dreg(&ins.args[0])] = r;
                }
                Op::Select => {
                    let (c, a, b) = (val(ar, &st, &ins.args[1]), val(ar, &st, &ins.args[2]), val(ar, &st, &ins.args[3]));
                    let c = match fact(&st, c) {
                        Some(f) => ar.cst(f as u32),
                        None => c,
                    };
                    let r = ar.select(c, a, b);
                    st.regs[dreg(&ins.args[0])] = r;
                }
                Op::Mac => {
                    let (a, b, c) = (val(ar, &st, &ins.args[1]), val(ar, &st, &ins.args[2]), val(ar, &st, &ins.args[3]));
                    let m = ar.mul(a, b);
                    let r = ar.add(m, c);
                    st.regs[dreg(&ins.args[0])] = r;
                }
                Op::Addsh => {
                    let (a, b) = (val(ar, &st, &ins.args[1]), val(ar, &st, &ins.args[2]));
                    let k = if let Arg::K(k) = ins.args[3] { k as u32 } else { 0 };
                    let kc = ar.cst(k);
                    let s = ar.shl(b, kc);
                    let r = ar.add(a, s);
                    st.regs[dreg(&ins.args[0])] = r;
                }
                Op::Div | Op::Mod => {
                    let (a, b) = (val(ar, &st, &ins.args[1]), val(ar, &st, &ins.args[2]));
                    if !ar.nonzero(b) {
                        // "b != 0" as a condition term: b itself (nonzero test)
                        guard!(b, (), End::DivZero);
                    }
                    let r = if ins.op == Op::Div { ar.div(a, b) } else { ar.modulo(a, b) };
                    st.regs[dreg(&ins.args[0])] = r;
                }
                Op::Load | Op::Store => {
                    let a = if ins.op == Op::Load { val(ar, &st, &ins.args[1]) } else { val(ar, &st, &ins.args[0]) };
                    if ar.umax(a) >= 4096 {
                        let lim2 = ar.cst(4096);
                        let valid = ar.ltu(a, lim2);
                        guard!(valid, (), End::BadAddr);
                    }
                    if ins.op == Op::Load {
                        // value
                        let mut rel: Vec<(T, T)> = Vec::new();
                        let mut result: Option<T> = None;
                        for &(sa, sv) in st.mem.iter().rev() {
                            if sa == a {
                                if rel.is_empty() {
                                    result = Some(sv);
                                } else {
                                    rel.push((sa, sv));
                                }
                                break;
                            }
                            if ar.distinct(sa, a) {
                                continue;
                            }
                            rel.push((sa, sv));
                        }
                        let r = match result {
                            Some(v) => v,
                            None => {
                                rel.reverse();
                                ar.intern(Node::Load(rel, a))
                            }
                        };
                        st.regs[dreg(&ins.args[0])] = r;
                    } else {
                        let v = val(ar, &st, &ins.args[1]);
                        st.mem.push((a, v));
                    }
                }
                Op::Vload => {
                    let p = val(ar, &st, &ins.args[1]);
                    let r = ar.intern(Node::VLoad(st.nvl, p));
                    st.nvl += 1;
                    st.effs.push(Eff::VLoad(p, r));
                    st.regs[dreg(&ins.args[0])] = r;
                }
                Op::Vstore => {
                    let (p, v) = (val(ar, &st, &ins.args[0]), val(ar, &st, &ins.args[1]));
                    st.effs.push(Eff::VStore(p, v));
                }
                Op::Jmp => {
                    if let Arg::L(l) = &ins.args[0] {
                        pc = prog.labels[l];
                        continue 'path;
                    }
                }
                Op::Br => {
                    let c = val(ar, &st, &ins.args[0]);
                    let (l1, l2) = match (&ins.args[1], &ins.args[2]) {
                        (Arg::L(a), Arg::L(b)) => (prog.labels[a], prog.labels[b]),
                        _ => unreachable!(),
                    };
                    let known = if let Node::Const(v) = ar.nodes[c as usize] { Some(v != 0) } else { fact(&st, c) };
                    match known {
                        Some(true) => pc = l1,
                        Some(false) => pc = l2,
                        None => {
                            let mut t = st.clone();
                            t.conds.push((c, false));
                            stack.push((l2, t));
                            st.conds.push((c, true));
                            pc = l1;
                        }
                    }
                    continue 'path;
                }
                Op::Ret | Op::Trap => {
                    let v = val(ar, &st, &ins.args[0]);
                    let mut conds = st.conds.clone();
                    conds.sort();
                    let end = if ins.op == Op::Ret { End::Ret(v) } else { End::Explicit(v) };
                    out.push(PathRes { conds, effs: st.effs.clone(), mem: st.mem.clone(), end });
                    break 'path;
                }
                Op::Phi => {}
            }
            pc += 1;
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Rewriting under path conditions
// ---------------------------------------------------------------------------
impl Arena {
    /// Rebuild `t` with the known facts substituted (a term with a known truth value becomes a
    /// constant, `select` on a known condition is resolved). Children have smaller ids than their
    /// parents, so a pass over the reachable ids in increasing order needs no recursion.
    fn under(&mut self, roots: &[T], facts: &HashMap<T, bool>, memo: &mut HashMap<T, T>) {
        let mut reach: Vec<T> = Vec::new();
        let mut seen: HashSet<T> = HashSet::new();
        let mut st: Vec<T> = roots.iter().copied().filter(|r| !memo.contains_key(r)).collect();
        while let Some(t) = st.pop() {
            if memo.contains_key(&t) || !seen.insert(t) {
                continue;
            }
            reach.push(t);
            match &self.nodes[t as usize] {
                Node::Const(_) | Node::Param(_) => {}
                Node::Poly(p) | Node::Bit(p) => {
                    for (k, _) in p {
                        st.extend(k.iter().copied());
                    }
                }
                Node::Lt(a, b) | Node::Ltu(a, b) | Node::Eq(a, b) | Node::Shr(a, b) | Node::Shl(a, b) | Node::Div(a, b) | Node::Mod(a, b) | Node::Raw(_, a, b) => {
                    st.push(*a);
                    st.push(*b);
                }
                Node::Select(c, a, b) => {
                    st.push(*c);
                    st.push(*a);
                    st.push(*b);
                }
                Node::Load(ch, a) => {
                    for (x, y) in ch {
                        st.push(*x);
                        st.push(*y);
                    }
                    st.push(*a);
                }
                Node::VLoad(_, p) => st.push(*p),
            }
        }
        reach.sort();
        for t in reach {
            let node = self.nodes[t as usize].clone();
            let m = |memo: &HashMap<T, T>, x: T| *memo.get(&x).unwrap_or(&x);
            let new = if let (Node::Select(c, a, b), Some(&f)) = (&node, facts.get(match &node { Node::Select(c, _, _) => c, _ => &t })) {
                let _ = c;
                m(&memo, if f { *a } else { *b })
            } else if let Some(&f) = facts.get(&t) {
                // facts are only recorded for 0/1-valued terms (and plain nonzero tests)
                if matches!(node, Node::Lt(..) | Node::Ltu(..) | Node::Eq(..)) { self.cst(f as u32) } else { self.rebuild(&node, memo) }
            } else {
                self.rebuild(&node, memo)
            };
            let _ = m;
            memo.insert(t, new);
        }
    }

    fn rebuild(&mut self, node: &Node, memo: &HashMap<T, T>) -> T {
        let m = |x: T| *memo.get(&x).unwrap_or(&x);
        match node {
            Node::Const(c) => self.cst(*c),
            Node::Param(_) => self.intern(node.clone()),
            Node::Poly(p) => {
                let mut acc = self.cst(0);
                for (k, c) in p {
                    let mut term = self.cst(*c);
                    for &a in k {
                        let a = m(a);
                        term = self.mul(term, a);
                    }
                    acc = self.add(acc, term);
                }
                acc
            }
            Node::Bit(p) => {
                let mut acc = self.cst(0);
                for (k, c) in p {
                    let mut term = self.cst(*c);
                    for &a in k {
                        let a = m(a);
                        term = self.and(term, a);
                    }
                    acc = self.xor(acc, term);
                }
                acc
            }
            Node::Lt(a, b) => self.lt(m(*a), m(*b)),
            Node::Ltu(a, b) => self.ltu(m(*a), m(*b)),
            Node::Eq(a, b) => {
                let (x, y) = (m(*a), m(*b));
                // Eq stores a normalised difference: Eq(d, -d) means d == 0
                let z = self.cst(0);
                let _ = y;
                self.eq(x, z)
            }
            Node::Shr(a, b) => self.shr(m(*a), m(*b)),
            Node::Shl(a, b) => self.shl(m(*a), m(*b)),
            Node::Select(c, a, b) => self.select(m(*c), m(*a), m(*b)),
            Node::Div(a, b) => self.div(m(*a), m(*b)),
            Node::Mod(a, b) => self.modulo(m(*a), m(*b)),
            Node::Raw(k, a, b) => {
                let (x, y) = (m(*a), m(*b));
                match k {
                    0 => self.add(x, y),
                    1 => self.mul(x, y),
                    2 => self.and(x, y),
                    _ => self.or(x, y),
                }
            }
            Node::Load(ch, a) => {
                let v: Vec<(T, T)> = ch.iter().map(|(x, y)| (m(*x), m(*y))).collect();
                let a2 = m(*a);
                self.intern(Node::Load(v, a2))
            }
            Node::VLoad(n, p) => {
                let p2 = m(*p);
                self.intern(Node::VLoad(*n, p2))
            }
        }
    }

    /// Is `t` identically zero? Decides the linear mixed boolean-arithmetic fragment.
    pub fn mba_zero(&mut self, t: T) -> bool {
        match &self.nodes[t as usize] {
            Node::Const(c) => return *c == 0,
            _ => {}
        }
        let poly = self.poly_of(t);
        let mut vars: Vec<T> = Vec::new();
        // each term: (coeff, kind) ; kind = Var(index) | BitAtom(anf over var indexes)
        enum Term {
            Var(usize),
            Bit(Vec<(Vec<usize>, u32)>),
            One,
        }
        let mut terms: Vec<(u32, Term)> = Vec::new();
        let mut vindex = |vars: &mut Vec<T>, a: T| -> usize {
            if let Some(p) = vars.iter().position(|&x| x == a) {
                p
            } else {
                vars.push(a);
                vars.len() - 1
            }
        };
        for (k, c) in &poly {
            if k.is_empty() {
                terms.push((c.wrapping_neg(), Term::One));
            } else if k.len() == 1 {
                let a = k[0];
                match self.nodes[a as usize].clone() {
                    Node::Bit(anf) => {
                        let mut v = Vec::new();
                        for (mono, mask) in anf {
                            if mask != 0 && mask != u32::MAX {
                                return false;
                            }
                            let idx: Vec<usize> = mono.iter().map(|&x| vindex(&mut vars, x)).collect();
                            v.push((idx, mask));
                        }
                        terms.push((*c, Term::Bit(v)));
                    }
                    _ => {
                        let i = vindex(&mut vars, a);
                        terms.push((*c, Term::Var(i)));
                    }
                }
            } else {
                return false;
            }
        }
        if vars.len() > 14 {
            return false;
        }
        for b in 0u32..(1u32 << vars.len()) {
            let mut g: u32 = 0;
            for (c, term) in &terms {
                let e: u32 = match term {
                    Term::One => 1,
                    Term::Var(i) => (b >> i) & 1,
                    Term::Bit(v) => {
                        let mut x = 0u32;
                        for (idx, mask) in v {
                            let mut a = (*mask != 0) as u32;
                            for &i in idx {
                                a &= (b >> i) & 1;
                            }
                            x ^= a;
                        }
                        x
                    }
                };
                g = g.wrapping_add(c.wrapping_mul(e));
            }
            if g != 0 {
                return false;
            }
        }
        true
    }

    pub fn equal(&mut self, a: T, b: T) -> bool {
        if a == b {
            return true;
        }
        let d = self.sub(a, b);
        self.mba_zero(d)
    }
}

fn canon_mem(ar: &mut Arena, mem: &[(T, T)], memo: &HashMap<T, T>) -> Result<Vec<(T, T)>, ()> {
    let m = |x: T| *memo.get(&x).unwrap_or(&x);
    let mut all_const = true;
    let mut cmap: BTreeMap<u32, T> = BTreeMap::new();
    let mut list: Vec<(T, T)> = Vec::new();
    for &(a, v) in mem {
        let (a, v) = (m(a), m(v));
        match ar.as_const(a) {
            Some(c) => {
                if (c as usize) < SCRATCH_BASE {
                    cmap.insert(c, v);
                }
            }
            None => all_const = false,
        }
        list.push((a, v));
    }
    if all_const {
        let mut out = Vec::new();
        for (c, v) in cmap {
            let ct = ar.cst(c);
            out.push((ct, v));
        }
        Ok(out)
    } else {
        // symbolic addresses: keep the order, drop scratch constants
        let mut out = Vec::new();
        for (a, v) in list {
            if let Some(c) = ar.as_const(a) {
                if (c as usize) >= SCRATCH_BASE {
                    continue;
                }
            }
            out.push((a, v));
        }
        Ok(out)
    }
}

pub fn prove(orig: &Program, cand: &Program) -> Verdict {
    if orig.params.len() != cand.params.len() {
        return Verdict::Unproved("parameter count".into());
    }
    let mut ar = Arena::new();
    let po = match explore(&mut ar, orig) {
        Ok(p) => p,
        Err(e) => return Verdict::NotApplicable(format!("original: {}", e)),
    };
    let pc = match explore(&mut ar, cand) {
        Ok(p) => p,
        Err(e) => return Verdict::NotApplicable(format!("candidate: {}", e)),
    };
    for (pi, p) in po.iter().enumerate() {
        let ko: HashSet<(T, bool)> = p.conds.iter().copied().collect();
        let facts: HashMap<T, bool> = p.conds.iter().map(|&(t, b)| (t, b)).collect();
        // candidate paths whose conditions are implied by the original path's
        let mut matched = false;
        let mut why = String::new();
        for q in pc.iter().filter(|q| q.conds.iter().all(|c| ko.contains(c))) {
            // collect all terms to rewrite
            let mut roots: Vec<T> = Vec::new();
            for e in [&p.effs, &q.effs] {
                for x in e {
                    match x {
                        Eff::VLoad(a, b) | Eff::VStore(a, b) => {
                            roots.push(*a);
                            roots.push(*b);
                        }
                    }
                }
            }
            for (x, y) in p.mem.iter().chain(q.mem.iter()) {
                roots.push(*x);
                roots.push(*y);
            }
            for e in [&p.end, &q.end] {
                if let End::Ret(t) | End::Explicit(t) = e {
                    roots.push(*t);
                }
            }
            let mut memo: HashMap<T, T> = HashMap::new();
            ar.under(&roots, &facts, &mut memo);
            let m = |x: T| *memo.get(&x).unwrap_or(&x);
            // ends
            let end_ok = match (&p.end, &q.end) {
                (End::Ret(a), End::Ret(b)) | (End::Explicit(a), End::Explicit(b)) => {
                    let (a, b) = (m(*a), m(*b));
                    ar.equal(a, b)
                }
                (End::DivZero, End::DivZero) | (End::BadAddr, End::BadAddr) => true,
                _ => false,
            };
            if !end_ok {
                why = "different result or trap".into();
                continue;
            }
            // volatile log
            if p.effs.len() != q.effs.len() {
                why = "different volatile log length".into();
                continue;
            }
            let mut eff_ok = true;
            for (x, y) in p.effs.iter().zip(q.effs.iter()) {
                let ok = match (x, y) {
                    (Eff::VLoad(a, _), Eff::VLoad(b, _)) => {
                        let (a, b) = (m(*a), m(*b));
                        ar.equal(a, b)
                    }
                    (Eff::VStore(a, v), Eff::VStore(b, w)) => {
                        let (a, b, v, w) = (m(*a), m(*b), m(*v), m(*w));
                        ar.equal(a, b) && ar.equal(v, w)
                    }
                    _ => false,
                };
                if !ok {
                    eff_ok = false;
                    break;
                }
            }
            if !eff_ok {
                why = "different volatile effect".into();
                continue;
            }
            // memory
            let (cm, cn) = match (canon_mem(&mut ar, &p.mem, &memo), canon_mem(&mut ar, &q.mem, &memo)) {
                (Ok(a), Ok(b)) => (a, b),
                _ => {
                    why = "memory".into();
                    continue;
                }
            };
            if cm.len() != cn.len() {
                why = "different stores".into();
                continue;
            }
            let mut mem_ok = true;
            for ((a1, v1), (a2, v2)) in cm.iter().zip(cn.iter()) {
                if !(ar.equal(*a1, *a2) && ar.equal(*v1, *v2)) {
                    mem_ok = false;
                    break;
                }
            }
            if !mem_ok {
                why = "different stores".into();
                continue;
            }
            matched = true;
            break;
        }
        if !matched {
            return Verdict::Unproved(format!("original path {} of {} not matched ({})", pi, po.len(), if why.is_empty() { "no candidate path with implied conditions" } else { &why }));
        }
    }
    Verdict::Proved
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(a: &str, b: &str) -> bool {
        let (pa, pb) = (parse_program(a).unwrap(), parse_program(b).unwrap());
        matches!(prove(&pa, &pb), Verdict::Proved)
    }

    #[test]
    fn ring_and_renaming() {
        assert!(v("func f(v0, v1)\ne:\n  mul v2, v0, 3\n  mul v3, v1, 3\n  add v4, v2, v3\n  ret v4\n",
                  "func f(v5, v6)\ne:\n  add v1, v5, v6\n  mul v9, v1, 3\n  ret v9\n"));
        assert!(!v("func f(v0)\ne:\n  add v1, v0, 1\n  ret v1\n", "func f(v0)\ne:\n  add v1, v0, 2\n  ret v1\n"));
    }

    #[test]
    fn bitwise_and_mba() {
        // x ^ y == (x | y) - (x & y)
        assert!(v("func f(v0, v1)\ne:\n  xor v2, v0, v1\n  ret v2\n",
                  "func f(v0, v1)\ne:\n  or v2, v0, v1\n  and v3, v0, v1\n  sub v4, v2, v3\n  ret v4\n"));
        // de Morgan
        assert!(v("func f(v0, v1)\ne:\n  and v2, v0, v1\n  xor v3, v2, -1\n  ret v3\n",
                  "func f(v0, v1)\ne:\n  xor v2, v0, -1\n  xor v3, v1, -1\n  or v4, v2, v3\n  ret v4\n"));
        assert!(!v("func f(v0, v1)\ne:\n  or v2, v0, v1\n  ret v2\n", "func f(v0, v1)\ne:\n  add v2, v0, v1\n  ret v2\n"));
    }

    #[test]
    fn ifconversion_and_traps() {
        let a = "func f(v0, v1)\ne:\n  lt v2, v0, v1\n  br v2, t, f\nt:\n  mov v3, v0\n  jmp j\nf:\n  mov v3, v1\n  jmp j\nj:\n  ret v3\n";
        let b = "func f(v0, v1)\ne:\n  lt v2, v0, v1\n  select v3, v2, v0, v1\n  ret v3\n";
        assert!(v(a, b));
        // removing a division that can trap is not accepted
        let c = "func f(v0, v1)\ne:\n  div v2, v0, v1\n  ret v0\n";
        let d = "func f(v0, v1)\ne:\n  ret v0\n";
        assert!(!v(c, d));
        assert!(v(c, c));
        // but a provably non-zero divisor is fine
        let e = "func f(v0, v1)\ne:\n  or v3, v1, 1\n  div v2, v0, v3\n  ret v0\n";
        assert!(v(e, d));
    }

    #[test]
    fn memory() {
        let a = "func f(v0)\ne:\n  and v1, v0, 63\n  store 5, v1\n  store 7, 9\n  load v2, 5\n  ret v2\n";
        let b = "func f(v0)\ne:\n  and v1, v0, 63\n  store 7, 9\n  store 5, v1\n  ret v1\n";
        assert!(v(a, b));
    }
}
