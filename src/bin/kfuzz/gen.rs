//! gen.rs - random KIR program generator (Csmith style) and random machine generator.
//!
//! Programs are built from structured pieces so that they are valid (every block ends in a
//! terminator, labels exist) and mostly terminate: every loop has a counter that the body
//! never writes, and a bounded trip count. On top of that the generator deliberately builds
//! the things that make optimizers fail: traps (div by 0, wild addresses, explicit trap),
//! volatile operations, loops with two entrances (irreducible), aliasing stores, high
//! register pressure, deep control flow, unreachable blocks, masked loop bounds, idioms of
//! the bench programs (checksums with `mod`, `i + k` array walks, guarded division, ...).
use crate::kir::*;

#[derive(Clone)]
pub struct GenCfg {
    pub fused_mac: bool,
    pub fused_addsh: bool,
    /// target number of statements (rough)
    pub size: usize,
    /// many registers alive at once
    pub pressure: bool,
    /// allow wild addresses / zero divisors / explicit traps
    pub traps: bool,
    /// allow touching the scratch region (3072..4095)
    pub scratch: bool,
}

struct Block {
    label: String,
    lines: Vec<String>,
}

pub struct Gen {
    rng: Rng,
    cfg: GenCfg,
    blocks: Vec<Block>,
    open: bool,
    nl: usize,
    ng: usize,
    np: usize,
    next_reg: usize,
    recent: Vec<usize>,
    loops: Vec<usize>,
    pool: Vec<i64>,
    budget: i64,
    max_depth: usize,
}

const POOL: [i64; 36] = [
    0, 1, 2, 3, 4, 5, 7, 8, 10, 15, 16, 31, 32, 33, 63, 64, 100, 255, 256, 1000, 1023, 1024, 4095, 4096, 3071, 3072,
    65521, 65535, 0x7FFF_FFFF, 0x8000_0000, 0xFFFF_FFFF, -1, -2, -8, 1000003, 0x9E37_79B1,
];

impl Gen {
    pub fn new(seed: u64, cfg: GenCfg) -> Gen {
        let mut rng = Rng::new(seed ^ 0xA5A5_1234_9876_5432);
        let np = rng.below(5) as usize; // 0..4 (sometimes more below)
        let np = if rng.below(12) == 0 { 5 + rng.below(4) as usize } else { np };
        let ng = if cfg.pressure { 14 + rng.below(30) as usize } else { 4 + rng.below(9) as usize };
        let ng = ng.max(np + 1);
        let mut pool: Vec<i64> = POOL.to_vec();
        for _ in 0..4 {
            pool.push(rng.below(5000) as i64);
        }
        for _ in 0..2 {
            pool.push((rng.next() & MASK as u64) as i64);
        }
        let budget = cfg.size as i64;
        Gen { rng, cfg, blocks: Vec::new(), open: false, nl: 0, ng, np, next_reg: ng, recent: Vec::new(), loops: Vec::new(), pool, budget, max_depth: 3 + 0 }
    }

    // ---- low level ----
    fn pct(&mut self, p: u64) -> bool {
        self.rng.below(100) < p
    }
    fn label(&mut self, hint: &str) -> String {
        self.nl += 1;
        format!("{}{}", hint, self.nl)
    }
    fn begin(&mut self, l: &str) {
        self.blocks.push(Block { label: l.to_string(), lines: Vec::new() });
        self.open = true;
    }
    fn emit(&mut self, s: String) {
        if !self.open {
            let l = self.label("dead");
            self.begin(&l);
        }
        self.blocks.last_mut().unwrap().lines.push(s);
    }
    fn term(&mut self, s: String) {
        self.emit(s);
        self.open = false;
    }
    fn jmp(&mut self, l: &str) {
        self.term(format!("jmp {}", l));
    }
    fn fresh_reg(&mut self) -> usize {
        let r = self.next_reg;
        self.next_reg += 1;
        r
    }
    fn lit(&mut self) -> String {
        let v = self.pool[self.rng.below(self.pool.len() as u64) as usize];
        self.fmt_lit(v)
    }
    fn fmt_lit(&mut self, v: i64) -> String {
        let u = (v as u64 & MASK as u64) as u32;
        match self.rng.below(5) {
            0 => format!("0x{:x}", u),
            1 if u >= 0x8000_0000 => format!("-{}", (u as i32).unsigned_abs()),
            _ => {
                if v < 0 && v >= -(1 << 31) {
                    format!("{}", v)
                } else {
                    format!("{}", u)
                }
            }
        }
    }
    fn greg(&mut self) -> usize {
        if !self.recent.is_empty() && self.pct(35) {
            let k = self.recent.len();
            return self.recent[self.rng.below(k as u64) as usize];
        }
        self.rng.below(self.ng as u64) as usize
    }
    fn dst(&mut self) -> usize {
        // params are overwritten rarely so that they stay interesting
        let r = self.rng.below(self.ng as u64) as usize;
        self.recent.push(r);
        if self.recent.len() > 6 {
            self.recent.remove(0);
        }
        r
    }
    fn src(&mut self) -> String {
        if self.pct(30) {
            self.lit()
        } else if !self.loops.is_empty() && self.pct(15) {
            let k = self.loops.len();
            format!("v{}", self.loops[self.rng.below(k as u64) as usize])
        } else {
            format!("v{}", self.greg())
        }
    }
    fn reg_src(&mut self) -> String {
        format!("v{}", self.greg())
    }

    // ---- statements ----
    fn binop_name(&mut self) -> &'static str {
        const OPS: [&str; 14] = ["add", "sub", "mul", "and", "or", "xor", "shl", "shr", "lt", "eq", "add", "sub", "add", "xor"];
        OPS[self.rng.below(OPS.len() as u64) as usize]
    }

    fn arith(&mut self) {
        let d = self.dst();
        match self.rng.below(100) {
            0..=44 => {
                let op = self.binop_name();
                let (a, b) = (self.src(), self.src());
                self.emit(format!("{} v{}, {}, {}", op, d, a, b));
            }
            45..=52 => {
                let a = self.src();
                self.emit(format!("mov v{}, {}", d, a));
            }
            53..=58 => {
                let l = self.lit();
                self.emit(format!("const v{}, {}", d, l));
            }
            59..=64 => {
                let (c, a, b) = (self.src(), self.src(), self.src());
                self.emit(format!("select v{}, {}, {}, {}", d, c, a, b));
            }
            65..=70 => {
                // shifts by constants
                let op = if self.pct(50) { "shl" } else { "shr" };
                let a = self.reg_src();
                let k = [0, 1, 2, 3, 4, 8, 16, 24, 31, 32, 33][self.rng.below(11) as usize];
                self.emit(format!("{} v{}, {}, {}", op, d, a, k));
            }
            71..=76 => {
                if self.cfg.fused_mac && self.pct(50) {
                    let (a, b, c) = (self.src(), self.src(), self.src());
                    self.emit(format!("mac v{}, {}, {}, {}", d, a, b, c));
                } else if self.cfg.fused_addsh {
                    let (a, b) = (self.src(), self.src());
                    let k = self.rng.below(32);
                    self.emit(format!("addsh v{}, {}, {}, {}", d, a, b, k));
                } else {
                    let (a, b) = (self.src(), self.src());
                    self.emit(format!("mul v{}, {}, {}", d, a, b));
                }
            }
            77..=84 => {
                // algebra bait
                let a = self.reg_src();
                let (op, c) = match self.rng.below(8) {
                    0 => ("add", "0"),
                    1 => ("mul", "1"),
                    2 => ("mul", "8"),
                    3 => ("xor", "-1"),
                    4 => ("and", "0xffffffff"),
                    5 => ("sub", "1"),
                    6 => ("or", "0"),
                    _ => ("mul", "-1"),
                };
                self.emit(format!("{} v{}, {}, {}", op, d, a, c));
            }
            85..=92 => {
                // common sub-expression pair
                let (a, b) = (self.reg_src(), self.src());
                let op = self.binop_name();
                self.emit(format!("{} v{}, {}, {}", op, d, a, b));
                let d2 = self.dst();
                self.emit(format!("{} v{}, {}, {}", op, d2, a, b));
            }
            _ => {
                let a = self.reg_src();
                self.emit(format!("lt v{}, {}, 0", d, a));
            }
        }
    }

    fn mask(&mut self) -> u32 {
        [3, 7, 7, 15, 15, 31, 63, 63, 127, 255, 1023][self.rng.below(11) as usize]
    }

    /// An address operand (emits code that computes it when needed).
    fn addr(&mut self) -> String {
        match self.rng.below(100) {
            0..=34 => {
                let t = self.greg();
                let m = self.mask();
                let x = self.reg_src();
                self.emit(format!("and v{}, {}, {}", t, x, m));
                self.recent.push(t);
                format!("v{}", t)
            }
            35..=54 if !self.loops.is_empty() => {
                let k = self.loops.len();
                let i = self.loops[self.rng.below(k as u64) as usize];
                let t = self.greg();
                let off = [0, 1, 2, 3, 5, 8, 16, 64][self.rng.below(8) as usize];
                if off == 0 {
                    return format!("v{}", i);
                }
                self.emit(format!("add v{}, v{}, {}", t, i, off));
                format!("v{}", t)
            }
            35..=64 => format!("{}", [0, 1, 2, 3, 4, 5, 8, 16, 17, 63, 64, 100, 200, 1000][self.rng.below(14) as usize]),
            65..=71 if self.cfg.scratch => format!("{}", 3072 + self.rng.below(1024)),
            72..=78 if self.cfg.traps => self.reg_src(),
            79..=84 if self.cfg.scratch => {
                let t = self.greg();
                let x = self.reg_src();
                self.emit(format!("and v{}, {}, 4095", t, x));
                format!("v{}", t)
            }
            _ => {
                let t = self.greg();
                let x = self.reg_src();
                self.emit(format!("and v{}, {}, 15", t, x));
                format!("v{}", t)
            }
        }
    }

    fn mem_stmt(&mut self) {
        match self.rng.below(100) {
            0..=44 => {
                let d = self.dst();
                let a = self.addr();
                self.emit(format!("load v{}, {}", d, a));
            }
            45..=84 => {
                let a = self.addr();
                let v = self.src();
                self.emit(format!("store {}, {}", a, v));
            }
            85..=92 => {
                // aliasing pair: store, then load through an address that may be the same
                let m = [1, 3, 7, 15][self.rng.below(4) as usize];
                let (x, y) = (self.reg_src(), self.reg_src());
                let (t1, t2) = (self.greg(), self.greg());
                if t1 == t2 {
                    return;
                }
                let v = self.src();
                let d = self.dst();
                self.emit(format!("and v{}, {}, {}", t1, x, m));
                self.emit(format!("and v{}, {}, {}", t2, y, m));
                self.emit(format!("store v{}, {}", t1, v));
                self.emit(format!("load v{}, v{}", d, t2));
                let d2 = self.dst();
                self.emit(format!("load v{}, v{}", d2, t1));
            }
            _ => {
                // redundant loads / dead store
                let a = self.addr();
                let d = self.dst();
                self.emit(format!("load v{}, {}", d, a));
                let v = self.src();
                self.emit(format!("store {}, {}", a, v));
                let d2 = self.dst();
                self.emit(format!("load v{}, {}", d2, a));
            }
        }
    }

    fn vol_stmt(&mut self) {
        let port = self.rng.below(5);
        if self.pct(50) {
            let d = self.dst();
            if self.pct(15) {
                let p = self.reg_src();
                self.emit(format!("vload v{}, {}", d, p));
            } else {
                self.emit(format!("vload v{}, {}", d, port));
            }
        } else {
            let v = self.src();
            if self.pct(15) {
                let p = self.reg_src();
                self.emit(format!("vstore {}, {}", p, v));
            } else {
                self.emit(format!("vstore {}, {}", port, v));
            }
        }
    }

    fn divmod_stmt(&mut self) {
        let d = self.dst();
        let op = if self.pct(50) { "div" } else { "mod" };
        let a = self.src();
        match self.rng.below(100) {
            0..=39 => {
                // constant non-zero divisor
                let c = [1, 2, 3, 4, 5, 7, 8, 10, 16, 64, 100, 255, 256, 1000, 65521, 1000003, -1, -2, -7, 0x8000_0000i64][self.rng.below(20) as usize];
                let c = self.fmt_lit(c);
                self.emit(format!("{} v{}, {}, {}", op, d, a, c));
            }
            40..=64 => {
                // divisor made non-zero
                let t = self.greg();
                let x = self.reg_src();
                self.emit(format!("or v{}, {}, 1", t, x));
                self.emit(format!("{} v{}, {}, v{}", op, d, a, t));
            }
            65..=79 => {
                // guarded: if (y == 0) skip
                let y = self.reg_src();
                let c = self.fresh_reg();
                let (ok, join) = (self.label("dg"), self.label("dj"));
                self.emit(format!("eq v{}, {}, 0", c, y));
                self.term(format!("br v{}, {}, {}", c, join, ok));
                self.begin(&ok);
                self.emit(format!("{} v{}, {}, {}", op, d, a, y));
                self.jmp(&join);
                self.begin(&join);
            }
            80..=89 if self.cfg.traps => {
                let y = self.reg_src();
                self.emit(format!("{} v{}, {}, {}", op, d, a, y));
            }
            _ => {
                // division by a masked value (zero is common: traps on some inputs)
                let t = self.greg();
                let x = self.reg_src();
                let m = self.mask() & 7;
                if self.cfg.traps {
                    self.emit(format!("and v{}, {}, {}", t, x, m));
                } else {
                    self.emit(format!("or v{}, {}, 1", t, x));
                }
                self.emit(format!("{} v{}, {}, v{}", op, d, a, t));
            }
        }
    }

    fn cond(&mut self) -> String {
        let c = self.fresh_reg();
        match self.rng.below(100) {
            0..=44 => {
                let (a, b) = (self.src(), self.src());
                self.emit(format!("lt v{}, {}, {}", c, a, b));
            }
            45..=64 => {
                let (a, b) = (self.src(), self.src());
                self.emit(format!("eq v{}, {}, {}", c, a, b));
            }
            65..=79 => {
                let a = self.reg_src();
                self.emit(format!("and v{}, {}, 1", c, a));
            }
            80..=89 => {
                return self.reg_src();
            }
            _ => {
                let a = self.reg_src();
                let k = self.lit();
                self.emit(format!("lt v{}, {}, {}", c, k, a));
            }
        }
        format!("v{}", c)
    }

    fn if_stmt(&mut self, depth: usize) {
        let c = self.cond();
        let (t, e, j) = (self.label("then"), self.label("else"), self.label("join"));
        let pure = self.pct(40);
        let two = self.pct(55);
        if two {
            self.term(format!("br {}, {}, {}", c, t, e));
        } else {
            self.term(format!("br {}, {}, {}", c, t, j));
        }
        self.begin(&t);
        let n = 1 + self.rng.below(3) as usize;
        if pure {
            for _ in 0..n {
                self.arith();
            }
        } else {
            self.stmts(depth + 1, n);
        }
        if self.pct(5) {
            let v = self.src();
            self.term(format!("ret {}", v));
        } else if self.cfg.traps && self.pct(4) {
            let v = self.src();
            self.term(format!("trap {}", v));
        }
        self.jmp(&j);
        if two {
            self.begin(&e);
            let n = 1 + self.rng.below(3) as usize;
            if pure {
                for _ in 0..n {
                    self.arith();
                }
            } else {
                self.stmts(depth + 1, n);
            }
            self.jmp(&j);
        }
        self.begin(&j);
    }

    fn switch_stmt(&mut self, depth: usize) {
        let x = self.reg_src();
        let k = 2 + self.rng.below(4) as usize;
        let t = self.fresh_reg();
        let j = self.label("sw_end");
        let m = [3, 7].iter().copied().nth(self.rng.below(2) as usize).unwrap();
        self.emit(format!("and v{}, {}, {}", t, x, m));
        for i in 0..k {
            let c = self.fresh_reg();
            let (case, next) = (self.label("case"), self.label("sw"));
            self.emit(format!("eq v{}, v{}, {}", c, t, i));
            self.term(format!("br v{}, {}, {}", c, case, next));
            self.begin(&case);
            let n = 1 + self.rng.below(2) as usize;
            self.stmts(depth + 1, n);
            self.jmp(&j);
            self.begin(&next);
        }
        let n = 1 + self.rng.below(2) as usize;
        self.stmts(depth + 1, n);
        self.jmp(&j);
        self.begin(&j);
    }

    fn loop_bound_mask(&self, depth: usize) -> u32 {
        [63, 15, 7, 3][depth.min(3)]
    }

    fn for_loop(&mut self, depth: usize) {
        let kind = self.rng.below(100);
        let i = self.fresh_reg();
        let head = self.label("head");
        let body = self.label("body");
        let exit = self.label("exit");
        let maskv = self.loop_bound_mask(depth);
        let bnd: String;
        if self.pct(35) {
            // constant trip count (peeling bait)
            let hi = [20, 8, 5, 4][depth.min(3)];
            bnd = format!("{}", 1 + self.rng.below(hi));
        } else {
            let b = self.fresh_reg();
            let x = self.reg_src();
            self.emit(format!("and v{}, {}, {}", b, x, maskv));
            bnd = format!("v{}", b);
        }
        match kind {
            0..=54 => {
                // for (i = s; i < n; i += step)
                let start = if self.pct(25) { 1 + self.rng.below(3) } else { 0 };
                let step = if self.pct(80) { 1 } else { 2 + self.rng.below(2) };
                self.emit(format!("const v{}, {}", i, start));
                self.jmp(&head);
                self.begin(&head);
                let c = self.fresh_reg();
                self.emit(format!("lt v{}, v{}, {}", c, i, bnd));
                self.term(format!("br v{}, {}, {}", c, body, exit));
                self.begin(&body);
                self.loops.push(i);
                let n = 2 + self.rng.below(5) as usize;
                self.stmts(depth + 1, n);
                self.loops.pop();
                self.emit(format!("add v{}, v{}, {}", i, i, step));
                self.jmp(&head);
                self.begin(&exit);
            }
            55..=69 => {
                // do { ... } while (++i < n)
                self.emit(format!("const v{}, 0", i));
                self.jmp(&body);
                self.begin(&body);
                self.loops.push(i);
                let n = 2 + self.rng.below(4) as usize;
                self.stmts(depth + 1, n);
                self.loops.pop();
                self.emit(format!("add v{}, v{}, 1", i, i));
                let c = self.fresh_reg();
                self.emit(format!("lt v{}, v{}, {}", c, i, bnd));
                self.term(format!("br v{}, {}, {}", c, body, exit));
                self.begin(&exit);
            }
            70..=84 => {
                // count down: i = n; while (i) { ...; i -= 1 }
                self.emit(format!("mov v{}, {}", i, bnd));
                self.jmp(&head);
                self.begin(&head);
                self.term(format!("br v{}, {}, {}", i, body, exit));
                self.begin(&body);
                self.loops.push(i);
                let n = 2 + self.rng.below(4) as usize;
                self.stmts(depth + 1, n);
                self.loops.pop();
                self.emit(format!("sub v{}, v{}, 1", i, i));
                self.jmp(&head);
                self.begin(&exit);
            }
            _ => {
                // loop with an early exit in the middle
                self.emit(format!("const v{}, 0", i));
                self.jmp(&head);
                self.begin(&head);
                let c = self.fresh_reg();
                self.emit(format!("lt v{}, v{}, {}", c, i, bnd));
                self.term(format!("br v{}, {}, {}", c, body, exit));
                self.begin(&body);
                self.loops.push(i);
                let n = 1 + self.rng.below(3) as usize;
                self.stmts(depth + 1, n);
                let cc = self.cond();
                let cont = self.label("cont");
                self.term(format!("br {}, {}, {}", cc, exit, cont));
                self.begin(&cont);
                let n = 1 + self.rng.below(3) as usize;
                self.stmts(depth + 1, n);
                self.loops.pop();
                self.emit(format!("add v{}, v{}, 1", i, i));
                self.jmp(&head);
                self.begin(&exit);
            }
        }
    }

    /// while (x) { x = f(x); } with a step cap: data dependent trip count.
    fn while_loop(&mut self, depth: usize) {
        let cap = self.fresh_reg();
        let (head, body, exit) = (self.label("whead"), self.label("wbody"), self.label("wexit"));
        let x = self.greg();
        let xs = self.reg_src();
        self.emit(format!("mov v{}, {}", x, xs));
        self.emit(format!("const v{}, 0", cap));
        self.jmp(&head);
        self.begin(&head);
        let c = self.fresh_reg();
        let lim = 8 + self.rng.below(60);
        self.emit(format!("lt v{}, v{}, {}", c, cap, lim));
        let c2 = self.fresh_reg();
        self.emit(format!("select v{}, v{}, v{}, 0", c2, c, x));
        self.term(format!("br v{}, {}, {}", c2, body, exit));
        self.begin(&body);
        match self.rng.below(4) {
            0 => self.emit(format!("shr v{}, v{}, 1", x, x)),
            1 => {
                let t = self.fresh_reg();
                self.emit(format!("and v{}, v{}, 1", t, x));
                let u = self.fresh_reg();
                self.emit(format!("mul v{}, v{}, 3", u, x));
                self.emit(format!("add v{}, v{}, 1", u, u));
                let h = self.fresh_reg();
                self.emit(format!("shr v{}, v{}, 1", h, x));
                self.emit(format!("select v{}, v{}, v{}, v{}", x, t, u, h));
            }
            2 => { let m = 1 + self.rng.below(50); self.emit(format!("mod v{}, v{}, {}", x, x, m)) }
            _ => self.emit(format!("and v{}, v{}, v{}", x, x, cap)),
        }
        self.loops.push(cap);
        let n = 1 + self.rng.below(3) as usize;
        self.stmts(depth + 1, n);
        self.loops.pop();
        self.emit(format!("add v{}, v{}, 1", cap, cap));
        self.jmp(&head);
        self.begin(&exit);
    }

    /// A loop with two entrances: entry -> A or B, A <-> B, bounded by a counter.
    fn irreducible(&mut self, depth: usize) {
        let k = self.fresh_reg();
        let (a, b, x) = (self.label("irrA"), self.label("irrB"), self.label("irrX"));
        let lim = 1 + self.rng.below(10);
        self.emit(format!("const v{}, 0", k));
        let c = self.cond();
        self.term(format!("br {}, {}, {}", c, a, b));
        for (me, other) in [(a.clone(), b.clone()), (b.clone(), a.clone())] {
            self.begin(&me);
            let n = 1 + self.rng.below(3) as usize;
            self.stmts(depth + 1, n);
            self.emit(format!("add v{}, v{}, 1", k, k));
            let t = self.fresh_reg();
            self.emit(format!("lt v{}, v{}, {}", t, k, lim));
            self.term(format!("br v{}, {}, {}", t, other, x));
        }
        self.begin(&x);
    }

    // ---- idioms taken from the bench kinds ----
    fn idiom(&mut self, depth: usize) {
        let _ = depth;
        match self.rng.below(9) {
            0 => {
                // checksum with mod
                let m = [65521, 251, 1000003, 97, 255][self.rng.below(5) as usize];
                let (s1, s2, i, n, t, c) = (self.fresh_reg(), self.fresh_reg(), self.fresh_reg(), self.fresh_reg(), self.fresh_reg(), self.fresh_reg());
                let x = self.reg_src();
                let (head, body, exit) = (self.label("ch"), self.label("cb"), self.label("cx"));
                self.emit(format!("and v{}, {}, 63", n, x));
                self.emit(format!("const v{}, 1", s1));
                self.emit(format!("const v{}, 0", s2));
                self.emit(format!("const v{}, 0", i));
                self.jmp(&head);
                self.begin(&head);
                self.emit(format!("lt v{}, v{}, v{}", c, i, n));
                self.term(format!("br v{}, {}, {}", c, body, exit));
                self.begin(&body);
                self.emit(format!("load v{}, v{}", t, i));
                self.emit(format!("and v{}, v{}, 255", t, t));
                self.emit(format!("add v{}, v{}, v{}", s1, s1, t));
                self.emit(format!("mod v{}, v{}, {}", s1, s1, m));
                self.emit(format!("add v{}, v{}, v{}", s2, s2, s1));
                self.emit(format!("mod v{}, v{}, {}", s2, s2, m));
                self.emit(format!("add v{}, v{}, 1", i, i));
                self.jmp(&head);
                self.begin(&exit);
                let d = self.dst();
                self.emit(format!("shl v{}, v{}, 16", d, s2));
                self.emit(format!("or v{}, v{}, v{}", d, d, s1));
            }
            1 => {
                // array sum with i+k addressing
                let (acc, i, n, t, u, c) = (self.fresh_reg(), self.fresh_reg(), self.fresh_reg(), self.fresh_reg(), self.fresh_reg(), self.fresh_reg());
                let x = self.reg_src();
                let (head, body, exit) = (self.label("ah"), self.label("ab"), self.label("ax"));
                let k = 1 + self.rng.below(6);
                self.emit(format!("and v{}, {}, 31", n, x));
                self.emit(format!("const v{}, 0", acc));
                self.emit(format!("const v{}, 0", i));
                self.jmp(&head);
                self.begin(&head);
                self.emit(format!("lt v{}, v{}, v{}", c, i, n));
                self.term(format!("br v{}, {}, {}", c, body, exit));
                self.begin(&body);
                self.emit(format!("add v{}, v{}, {}", t, i, k));
                self.emit(format!("load v{}, v{}", u, t));
                self.emit(format!("add v{}, v{}, v{}", acc, acc, u));
                if self.pct(50) {
                    self.emit(format!("add v{}, v{}, {}", t, i, k + 2));
                    self.emit(format!("store v{}, v{}", t, acc));
                }
                self.emit(format!("add v{}, v{}, 1", i, i));
                self.jmp(&head);
                self.begin(&exit);
                let d = self.dst();
                self.emit(format!("mov v{}, v{}", d, acc));
            }
            2 => {
                // prefix copy with possible overlap: a[i+1] = a[i] + c
                let (i, n, t, c) = (self.fresh_reg(), self.fresh_reg(), self.fresh_reg(), self.fresh_reg());
                let x = self.reg_src();
                let (head, body, exit) = (self.label("ph"), self.label("pb"), self.label("px"));
                self.emit(format!("and v{}, {}, 15", n, x));
                self.emit(format!("const v{}, 0", i));
                self.jmp(&head);
                self.begin(&head);
                self.emit(format!("lt v{}, v{}, v{}", c, i, n));
                self.term(format!("br v{}, {}, {}", c, body, exit));
                self.begin(&body);
                self.emit(format!("load v{}, v{}", t, i));
                self.emit(format!("add v{}, v{}, 3", t, t));
                let j = self.fresh_reg();
                self.emit(format!("add v{}, v{}, 1", j, i));
                self.emit(format!("store v{}, v{}", j, t));
                self.emit(format!("mov v{}, v{}", i, j));
                self.jmp(&head);
                self.begin(&exit);
            }
            3 => {
                // constant-trip bit loop (crc-like)
                let (i, c, r) = (self.fresh_reg(), self.fresh_reg(), self.fresh_reg());
                let x = self.reg_src();
                let (head, body, exit) = (self.label("bh"), self.label("bb"), self.label("bx"));
                let trips = [8, 16, 32, 5][self.rng.below(4) as usize];
                self.emit(format!("mov v{}, {}", r, x));
                self.emit(format!("const v{}, 0", i));
                self.jmp(&head);
                self.begin(&head);
                self.emit(format!("lt v{}, v{}, {}", c, i, trips));
                self.term(format!("br v{}, {}, {}", c, body, exit));
                self.begin(&body);
                let (b, m, sh) = (self.fresh_reg(), self.fresh_reg(), self.fresh_reg());
                self.emit(format!("and v{}, v{}, 1", b, r));
                self.emit(format!("sub v{}, 0, v{}", m, b));
                self.emit(format!("and v{}, v{}, 0xEDB88320", m, m));
                self.emit(format!("shr v{}, v{}, 1", sh, r));
                self.emit(format!("xor v{}, v{}, v{}", r, sh, m));
                self.emit(format!("add v{}, v{}, 1", i, i));
                self.jmp(&head);
                self.begin(&exit);
                let d = self.dst();
                self.emit(format!("mov v{}, v{}", d, r));
            }
            4 => {
                // guarded division
                let (a, b) = (self.reg_src(), self.reg_src());
                let (c, d) = (self.fresh_reg(), self.dst());
                let (ok, bad, join) = (self.label("gd"), self.label("gz"), self.label("gj"));
                self.emit(format!("eq v{}, {}, 0", c, b));
                self.term(format!("br v{}, {}, {}", c, bad, ok));
                self.begin(&ok);
                self.emit(format!("div v{}, {}, {}", d, a, b));
                self.jmp(&join);
                self.begin(&bad);
                self.emit(format!("const v{}, 0", d));
                self.jmp(&join);
                self.begin(&join);
            }
            5 => {
                // min / max by branches
                let (a, b, d) = (self.reg_src(), self.reg_src(), self.dst());
                let (c, t, e, j) = (self.fresh_reg(), self.label("mt"), self.label("me"), self.label("mj"));
                self.emit(format!("lt v{}, {}, {}", c, a, b));
                self.term(format!("br v{}, {}, {}", c, t, e));
                self.begin(&t);
                self.emit(format!("mov v{}, {}", d, a));
                self.jmp(&j);
                self.begin(&e);
                self.emit(format!("mov v{}, {}", d, b));
                self.jmp(&j);
                self.begin(&j);
            }
            6 => {
                // polynomial in i (strength reduction bait)
                let (i, n, c, t, u, acc) = (self.fresh_reg(), self.fresh_reg(), self.fresh_reg(), self.fresh_reg(), self.fresh_reg(), self.fresh_reg());
                let x = self.reg_src();
                let (head, body, exit) = (self.label("sh"), self.label("sb"), self.label("sx"));
                self.emit(format!("and v{}, {}, 15", n, x));
                self.emit(format!("const v{}, 0", i));
                self.emit(format!("const v{}, 0", acc));
                self.jmp(&head);
                self.begin(&head);
                self.emit(format!("lt v{}, v{}, v{}", c, i, n));
                self.term(format!("br v{}, {}, {}", c, body, exit));
                self.begin(&body);
                let (k1, k2) = (1 + self.rng.below(12), 1 + self.rng.below(12));
                self.emit(format!("mul v{}, v{}, {}", t, i, k1));
                self.emit(format!("mul v{}, v{}, {}", u, i, k2));
                self.emit(format!("add v{}, v{}, v{}", t, t, u));
                self.emit(format!("add v{}, v{}, v{}", acc, acc, t));
                self.emit(format!("add v{}, v{}, 1", i, i));
                self.jmp(&head);
                self.begin(&exit);
                let d = self.dst();
                self.emit(format!("mov v{}, v{}", d, acc));
            }
            7 => {
                // checked add: trap on overflow
                let (a, b) = (self.reg_src(), self.reg_src());
                let (s, c, ok, bad) = (self.dst(), self.fresh_reg(), self.label("ok"), self.label("ovf"));
                self.emit(format!("add v{}, {}, {}", s, a, b));
                self.emit(format!("lt v{}, v{}, {}", c, s, a));
                self.term(format!("br v{}, {}, {}", c, bad, ok));
                self.begin(&bad);
                let tv = 1 + self.rng.below(100);
                self.term(format!("trap {}", tv));
                self.begin(&ok);
            }
            _ => {
                // volatile stream copy
                let (i, n, c, t) = (self.fresh_reg(), self.fresh_reg(), self.fresh_reg(), self.fresh_reg());
                let x = self.reg_src();
                let (head, body, exit) = (self.label("vh"), self.label("vb"), self.label("vx"));
                self.emit(format!("and v{}, {}, 7", n, x));
                self.emit(format!("const v{}, 0", i));
                self.jmp(&head);
                self.begin(&head);
                self.emit(format!("lt v{}, v{}, v{}", c, i, n));
                self.term(format!("br v{}, {}, {}", c, body, exit));
                self.begin(&body);
                let pa = self.rng.below(3);
                self.emit(format!("vload v{}, {}", t, pa));
                self.emit(format!("add v{}, v{}, v{}", t, t, i));
                let pb = 1 + self.rng.below(3);
                self.emit(format!("vstore {}, v{}", pb, t));
                self.emit(format!("add v{}, v{}, 1", i, i));
                self.jmp(&head);
                self.begin(&exit);
            }
        }
    }

    fn stmts(&mut self, depth: usize, n: usize) {
        for _ in 0..n {
            if self.budget <= 0 {
                return;
            }
            self.budget -= 1;
            let deep = depth < self.max_depth;
            let save = self.next_reg;
            match self.rng.below(100) {
                0..=39 => self.arith(),
                40..=53 => self.mem_stmt(),
                54..=58 => self.vol_stmt(),
                59..=65 => self.divmod_stmt(),
                66..=73 if deep => self.if_stmt(depth),
                74..=79 if deep => self.for_loop(depth),
                80..=82 if deep => self.while_loop(depth),
                83..=85 if deep => self.irreducible(depth),
                86..=88 if deep => self.switch_stmt(depth),
                89..=92 if deep => self.idiom(depth),
                93 if self.cfg.traps => {
                    let v = self.src();
                    let c = self.cond();
                    let (t, j) = (self.label("tr"), self.label("trj"));
                    self.term(format!("br {}, {}, {}", c, t, j));
                    self.begin(&t);
                    self.term(format!("trap {}", v));
                    self.begin(&j);
                }
                94 => {
                    // dead block / constant branch
                    let (a, b) = (self.label("cba"), self.label("cbb"));
                    let c = if self.pct(50) { "0" } else { "1" };
                    self.term(format!("br {}, {}, {}", c, a, b));
                    self.begin(&a);
                    self.arith();
                    self.jmp(&b);
                    self.begin(&b);
                }
                _ => self.arith(),
            }
            self.next_reg = save;
        }
    }

    pub fn build(mut self) -> String {
        let entry = "entry".to_string();
        self.begin(&entry);
        if self.cfg.pressure {
            // define lots of values that stay alive until the end
            for r in 0..self.ng {
                if self.np == 0 {
                    let l = self.lit();
                    self.emit(format!("const v{}, {}", r, l));
                } else {
                    let p = self.rng.below(self.np as u64);
                    let c = self.lit();
                    let op = ["add", "xor", "mul", "sub"][self.rng.below(4) as usize];
                    if r >= self.np {
                        self.emit(format!("{} v{}, v{}, {}", op, r, p, c));
                    }
                }
            }
        }
        let n = self.cfg.size;
        // top-level sequence
        while self.budget > 0 {
            let k = 1 + self.rng.below(4) as usize;
            self.stmts(0, k);
            let _ = n;
        }
        // result
        if self.cfg.pressure || self.pct(30) {
            let acc = self.fresh_reg();
            self.emit(format!("const v{}, 0", acc));
            for r in 0..self.ng {
                let op = ["add", "xor"][self.rng.below(2) as usize];
                self.emit(format!("{} v{}, v{}, v{}", op, acc, acc, r));
            }
            self.term(format!("ret v{}", acc));
        } else {
            let r = self.src();
            self.term(format!("ret {}", r));
        }
        // sometimes an unreachable block
        if self.pct(10) {
            let l = self.label("unreach");
            self.begin(&l);
            self.arith();
            self.term("ret 0".to_string());
        }
        let params: Vec<String> = (0..self.np).map(|i| format!("v{}", i)).collect();
        let mut s = format!("; generated by kfuzz\nfunc gen({})\n", params.join(", "));
        for b in &self.blocks {
            s.push_str(&format!("{}:\n", b.label));
            for l in &b.lines {
                s.push_str(&format!("  {}\n", l));
            }
        }
        s
    }
}

/// Generate a random program for a seed.
pub fn gen_program(seed: u64, cfg: &GenCfg) -> String {
    Gen::new(seed, cfg.clone()).build()
}

// ---------------------------------------------------------------------------
// Machines
// ---------------------------------------------------------------------------
pub fn to_cfg(mc: &Machine) -> String {
    format!(
        "issue_width = {}\nmem_ports = {}\nregisters = {}\nreg_spill_penalty = {}\ntaken_branch_penalty = {}\nicache_size = {}\nicache_penalty = {}\nlat_alu = {}\nlat_mul = {}\nlat_div = {}\nlat_load = {}\nlat_mac = {}\nlat_addsh = {}\nfused_mac = {}\nfused_addsh = {}\n",
        mc.issue_width, mc.mem_ports, mc.registers, mc.reg_spill_penalty, mc.taken_branch_penalty, mc.icache_size, mc.icache_penalty,
        mc.lat_alu, mc.lat_mul, mc.lat_div, mc.lat_load, mc.lat_mac, mc.lat_addsh, mc.fused_mac, mc.fused_addsh
    )
}

/// A random machine over every key's legal range (the judges use machines nobody has seen).
pub fn gen_machine(seed: u64) -> Machine {
    let mut r = Rng::new(seed ^ 0x1357_9BDF_2468_ACE0);
    let mode = r.below(10);
    let rng_in = |r: &mut Rng, lo: u64, hi: u64| lo + r.below(hi - lo + 1);
    // log-uniform pick in [lo, hi]
    let logu = |r: &mut Rng, lo: u64, hi: u64| {
        let (a, b) = ((lo as f64).ln(), (hi as f64).ln());
        let u = (r.below(1_000_000) as f64) / 1_000_000.0;
        ((a + (b - a) * u).exp().round() as u64).clamp(lo, hi)
    };
    let mut mc = Machine::default();
    if mode == 0 {
        // extremes
        let pick = |r: &mut Rng, lo: u64, hi: u64| if r.below(2) == 0 { lo } else { hi };
        mc.issue_width = pick(&mut r, 1, 8) as i64;
        mc.mem_ports = pick(&mut r, 1, 8) as i64;
        mc.registers = pick(&mut r, 4, 64) as i64;
        mc.reg_spill_penalty = pick(&mut r, 0, 40) as i64;
        mc.taken_branch_penalty = pick(&mut r, 0, 10) as i64;
        mc.icache_size = pick(&mut r, 8, 4096) as i64;
        mc.icache_penalty = pick(&mut r, 0, 20) as i64;
        mc.lat_alu = pick(&mut r, 1, 40) as i64;
        mc.lat_mul = pick(&mut r, 1, 40) as i64;
        mc.lat_div = pick(&mut r, 1, 40) as i64;
        mc.lat_load = pick(&mut r, 1, 40) as i64;
        mc.lat_mac = pick(&mut r, 1, 40) as i64;
        mc.lat_addsh = pick(&mut r, 1, 40) as i64;
    } else {
        mc.issue_width = rng_in(&mut r, 1, 8) as i64;
        mc.mem_ports = rng_in(&mut r, 1, (mc.issue_width as u64).max(1).min(4)) as i64;
        mc.registers = logu(&mut r, 4, 64) as i64;
        mc.reg_spill_penalty = rng_in(&mut r, 0, 30) as i64;
        mc.taken_branch_penalty = rng_in(&mut r, 0, 10) as i64;
        mc.icache_size = logu(&mut r, 8, 4096) as i64;
        mc.icache_penalty = rng_in(&mut r, 0, 10) as i64;
        mc.lat_alu = if r.below(3) == 0 { rng_in(&mut r, 1, 5) } else { 1 } as i64;
        mc.lat_mul = rng_in(&mut r, 1, 12) as i64;
        mc.lat_div = rng_in(&mut r, 1, 40) as i64;
        mc.lat_load = rng_in(&mut r, 1, 20) as i64;
        mc.lat_mac = rng_in(&mut r, 1, 14) as i64;
        mc.lat_addsh = rng_in(&mut r, 1, 5) as i64;
    }
    mc.fused_mac = r.below(2) as i64;
    mc.fused_addsh = r.below(2) as i64;
    mc
}
