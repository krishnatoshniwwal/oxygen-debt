//! sched2.rs - exact-cost scheduling and renaming on the final, named program text.
//!
//! The Kiln's cost of a program depends on the instruction order (in-order issue), on the
//! register *names* (hardware LRU, write-after-write stalls) and on the layout (fences). The
//! earlier passes choose an order on SSA values and names afterwards; here both are chosen
//! together and scored with an exact copy of the timing model of `kir::time_trace`.
//!
//! What is in this file (see DESIGN.md, section "sched2"):
//!  * `Net`      the program as blocks of named instructions (parse / print).
//!  * `Sim`      the exact timing model, step by step (slots, memory ports, latency, fences,
//!               write-after-write waits, instruction-cache cost, hardware-LRU register misses).
//!  * `Region`   dependence DAG of one block + the naming rules (which names are free where).
//!  * windows    short stretches of the real execution (blocks before and after) that give the
//!               state a block is entered in; a candidate order is scored by simulating them.
//!  * search     list scheduling seeds, exhaustive search for small blocks, local search.
//!
//! Safety: only the order of instructions inside a block and the names of block-local values
//! change. Every use keeps reading the value it read (data edges), instructions that may trap,
//! write memory or are visible keep the order of `Region::conflict`, and a name is reused only
//! where no other value that is still needed lives in it (`Region::name_block`). The driver
//! still measures the result and compares it with the input on the self-check inputs.
use crate::kir::*;

// ---------------------------------------------------------------------------
// The program as named instructions
// ---------------------------------------------------------------------------
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Opd {
    R(i32), // register (dense id)
    L(i64), // literal
}

#[derive(Clone, Debug)]
pub struct Ins {
    pub op: Op,
    pub d: i32, // destination (dense id) or -1
    pub a: [Opd; 3],
    pub na: u8,
    pub t: [u32; 2], // branch targets (block indices)
    pub nt: u8,
}

impl Ins {
    /// Register sources in operand order (may repeat).
    #[inline]
    pub fn regs(&self) -> ([i32; 3], usize) {
        let mut r = [0i32; 3];
        let mut n = 0;
        for k in 0..self.na as usize {
            if let Opd::R(x) = self.a[k] {
                r[n] = x;
                n += 1;
            }
        }
        (r, n)
    }

    /// Names touched by the instruction in the order the timing model touches them:
    /// sources (without repeats), then the destination (if not already listed).
    #[inline]
    pub fn touch(&self) -> ([i32; 4], usize) {
        let (r, n) = self.regs();
        let mut t = [0i32; 4];
        let mut m = 0;
        for k in 0..n {
            if !t[..m].contains(&r[k]) {
                t[m] = r[k];
                m += 1;
            }
        }
        if self.d >= 0 && !t[..m].contains(&self.d) {
            t[m] = self.d;
            m += 1;
        }
        (t, m)
    }
}

pub struct Net {
    pub name: String,
    pub params: Vec<i32>,
    pub orig: Vec<i64>, // dense id -> register number in the text
    pub nnames: usize,
    pub labels: Vec<String>,
    pub blocks: Vec<Vec<Ins>>,
    pub start: Vec<usize>,    // index of the first instruction of every block
    pub block_of: Vec<u32>,   // instruction index -> block
}

impl Net {
    pub fn from_program(p: &Program) -> Net {
        let mut map: std::collections::HashMap<i64, i32> = std::collections::HashMap::new();
        let mut orig: Vec<i64> = Vec::new();
        let mut id = |r: i64, map: &mut std::collections::HashMap<i64, i32>, orig: &mut Vec<i64>| -> i32 {
            if let Some(&x) = map.get(&r) {
                return x;
            }
            let x = orig.len() as i32;
            map.insert(r, x);
            orig.push(r);
            x
        };
        let params: Vec<i32> = p.params.iter().map(|&r| id(r, &mut map, &mut orig)).collect();
        // same numbering as compile_program: parameters first, then order of appearance
        for ins in &p.instrs {
            for a in &ins.args {
                if let Arg::R(r) = a {
                    id(*r, &mut map, &mut orig);
                }
            }
        }
        let nb = p.block_labels.len();
        let mut blocks: Vec<Vec<Ins>> = vec![Vec::new(); nb];
        for (i, pi) in p.instrs.iter().enumerate() {
            let sig = pi.op.sig().as_bytes();
            let mut ins = Ins { op: pi.op, d: -1, a: [Opd::L(0); 3], na: 0, t: [0; 2], nt: 0 };
            for (k, arg) in sig.iter().zip(pi.args.iter()) {
                match (*k, arg) {
                    (b'd', Arg::R(r)) => ins.d = map[r],
                    (_, Arg::R(r)) => {
                        ins.a[ins.na as usize] = Opd::R(map[r]);
                        ins.na += 1;
                    }
                    (_, Arg::I(v)) | (_, Arg::K(v)) => {
                        ins.a[ins.na as usize] = Opd::L(*v);
                        ins.na += 1;
                    }
                    (_, Arg::L(l)) => {
                        ins.t[ins.nt as usize] = p.block_labels.iter().position(|x| x == l).unwrap() as u32;
                        ins.nt += 1;
                    }
                }
            }
            blocks[p.block_of[i]].push(ins);
        }
        let mut start = Vec::with_capacity(nb);
        let mut block_of = Vec::with_capacity(p.instrs.len());
        let mut s = 0;
        for (b, bl) in blocks.iter().enumerate() {
            start.push(s);
            s += bl.len();
            for _ in 0..bl.len() {
                block_of.push(b as u32);
            }
        }
        Net { name: p.name.clone(), params, nnames: orig.len(), orig, labels: p.block_labels.clone(), blocks, start, block_of }
    }

    pub fn count(&self) -> usize {
        self.blocks.iter().map(|b| b.len()).sum()
    }

    pub fn to_text(&self) -> String {
        let mut out = String::new();
        out.push_str("func ");
        out.push_str(&self.name);
        out.push('(');
        for (i, &p) in self.params.iter().enumerate() {
            if i > 0 {
                out.push_str(", ");
            }
            out.push_str(&format!("v{}", self.orig[p as usize]));
        }
        out.push_str(")\n");
        for (b, bl) in self.blocks.iter().enumerate() {
            out.push_str(&self.labels[b]);
            out.push_str(":\n");
            for ins in bl {
                out.push_str("  ");
                out.push_str(ins.op.name());
                let mut first = true;
                let mut sep = |out: &mut String| {
                    out.push_str(if first { " " } else { ", " });
                    first = false;
                };
                if ins.d >= 0 {
                    sep(&mut out);
                    out.push_str(&format!("v{}", self.orig[ins.d as usize]));
                }
                for k in 0..ins.na as usize {
                    sep(&mut out);
                    match ins.a[k] {
                        Opd::R(x) => out.push_str(&format!("v{}", self.orig[x as usize])),
                        Opd::L(v) => out.push_str(&format!("{}", v)),
                    }
                }
                for k in 0..ins.nt as usize {
                    sep(&mut out);
                    out.push_str(&self.labels[ins.t[k] as usize]);
                }
                out.push('\n');
            }
        }
        out
    }

    /// Execution trace (instruction indices) -> sequence of executed blocks. A block that
    /// the run did not finish (trap in the middle, step limit) is dropped.
    pub fn block_seq(&self, trace: &[u32]) -> Vec<u32> {
        let mut seq = Vec::new();
        for &pc in trace {
            let b = self.block_of[pc as usize] as usize;
            if self.start[b] == pc as usize {
                seq.push(b as u32);
            }
        }
        if let Some(&pc) = trace.last() {
            let b = self.block_of[pc as usize] as usize;
            if pc as usize != self.start[b] + self.blocks[b].len() - 1 {
                seq.pop();
            }
        }
        seq
    }
}

// ---------------------------------------------------------------------------
// The exact timing model, one instruction at a time
// ---------------------------------------------------------------------------
#[derive(Clone)]
pub struct Sim {
    w: i64,
    mp: i64,
    k: usize,
    sp: i64,
    tb: i64,
    ic: usize,
    icp: i64,
    lat: [i64; 32],
    pub track: bool,
    ready: Vec<i64>,
    stamp: Vec<u64>,
    spilled: Vec<bool>,
    res: Vec<i32>,
    counter: u64,
    pub last: i64,
    used: i64,
    memused: i64,
    pub fence: i64,
    pub steps: u64, // work counter: instructions simulated
}

impl Sim {
    pub fn new(mc: &Machine, nnames: usize) -> Sim {
        let mut lat = [1i64; 32];
        for op in [
            Op::Const, Op::Mov, Op::Add, Op::Sub, Op::Mul, Op::And, Op::Or, Op::Xor, Op::Shl, Op::Shr, Op::Lt,
            Op::Eq, Op::Div, Op::Mod, Op::Select, Op::Mac, Op::Addsh, Op::Load, Op::Store, Op::Vload, Op::Vstore,
            Op::Trap, Op::Jmp, Op::Br, Op::Ret, Op::Phi,
        ] {
            lat[op as usize] = mc.lat(op);
        }
        let k = mc.registers as usize;
        Sim {
            w: mc.issue_width,
            mp: mc.mem_ports,
            k,
            sp: mc.reg_spill_penalty,
            tb: mc.taken_branch_penalty,
            ic: mc.icache_size as usize,
            icp: mc.icache_penalty,
            lat,
            track: nnames > k,
            ready: vec![0; nnames],
            stamp: vec![0; nnames],
            spilled: vec![false; nnames],
            res: Vec::new(),
            counter: 0,
            last: 0,
            used: 0,
            memused: 0,
            fence: 0,
            steps: 0,
        }
    }

    #[inline]
    pub fn lat(&self, op: Op) -> i64 {
        self.lat[op as usize]
    }

    /// Fresh state. `from_start`: the program's first instruction comes next (parameters are
    /// loaded in order; those beyond K start spilled). Otherwise nothing is resident or spilled.
    pub fn reset(&mut self, nparams: usize, from_start: bool) {
        for x in self.ready.iter_mut() {
            *x = 0;
        }
        for x in self.stamp.iter_mut() {
            *x = 0;
        }
        for x in self.spilled.iter_mut() {
            *x = false;
        }
        self.res.clear();
        self.counter = 0;
        self.last = 0;
        self.used = 0;
        self.memused = 0;
        self.fence = 0;
        if self.track && from_start {
            for r in 0..nparams {
                if self.res.len() >= self.k {
                    self.evict();
                }
                self.counter += 1;
                self.stamp[r] = self.counter;
                self.res.push(r as i32);
            }
        }
    }

    fn evict(&mut self) {
        let mut bi = 0;
        let mut bs = u64::MAX;
        for (i, &r) in self.res.iter().enumerate() {
            let s = self.stamp[r as usize];
            if s < bs {
                bs = s;
                bi = i;
            }
        }
        let old = self.res.swap_remove(bi) as usize;
        self.stamp[old] = 0;
        self.spilled[old] = true;
    }

    /// Issue one instruction; `gidx` is its index in the file (instruction cache).
    /// Returns the cycle it issues in.
    #[inline]
    pub fn step(&mut self, ins: &Ins, gidx: usize) -> i64 {
        self.steps += 1;
        let mut f = if gidx >= self.ic { self.icp } else { 0 };
        let (t4, nt) = ins.touch();
        if self.track {
            for &r in &t4[..nt] {
                let ru = r as usize;
                if self.stamp[ru] != 0 {
                    self.counter += 1;
                    self.stamp[ru] = self.counter;
                } else {
                    if self.spilled[ru] {
                        f += self.sp;
                        self.spilled[ru] = false;
                    }
                    if self.res.len() >= self.k {
                        self.evict();
                    }
                    self.counter += 1;
                    self.stamp[ru] = self.counter;
                    self.res.push(r);
                }
            }
        }
        let mut t = self.last + f;
        if self.fence > t {
            t = self.fence;
        }
        let (rs, n) = ins.regs();
        for &r in &rs[..n] {
            let x = self.ready[r as usize];
            if x > t {
                t = x;
            }
        }
        if ins.d >= 0 {
            let x = self.ready[ins.d as usize];
            if x > t {
                t = x;
            }
        }
        let m = ins.op.is_mem();
        if t == self.last && (self.used >= self.w || (m && self.memused >= self.mp)) {
            t += 1;
        }
        if t > self.last {
            self.used = 0;
            self.memused = 0;
        }
        self.used += 1;
        if m {
            self.memused += 1;
        }
        self.last = t;
        if ins.d >= 0 {
            self.ready[ins.d as usize] = t + self.lat[ins.op as usize];
        }
        t
    }

    /// The instruction just issued was a taken jump.
    #[inline]
    pub fn taken(&mut self) {
        self.fence = self.last + 1 + self.tb;
    }

    /// Run a sequence of whole blocks (indices into `blocks`). A block whose successor in the
    /// sequence is not the next block of the file ends with a taken jump. Returns the cycle
    /// count (`last + 1`).
    pub fn run_seq(&mut self, blocks: &[Vec<Ins>], start: &[usize], seq: &[u32], nparams: usize, from_start: bool) -> i64 {
        self.reset(nparams, from_start);
        for (i, &b) in seq.iter().enumerate() {
            let b = b as usize;
            let bl = &blocks[b];
            let s0 = start[b];
            for (j, ins) in bl.iter().enumerate() {
                self.step(ins, s0 + j);
            }
            if i + 1 < seq.len() && seq[i + 1] as usize != b + 1 && bl.last().map_or(false, |x| x.op.is_term()) {
                self.taken();
            }
        }
        self.last + 1
    }
}

// ---------------------------------------------------------------------------
// Liveness of names
// ---------------------------------------------------------------------------
pub struct Live {
    pub words: usize,
    pub live_in: Vec<Vec<u64>>,
    pub live_out: Vec<Vec<u64>>,
}

#[inline]
fn bit(v: &[u64], i: usize) -> bool {
    v[i >> 6] >> (i & 63) & 1 != 0
}

pub fn liveness(net: &Net) -> Live {
    let nb = net.blocks.len();
    let words = (net.nnames + 63) / 64;
    let mut usev = vec![vec![0u64; words]; nb];
    let mut defv = vec![vec![0u64; words]; nb];
    let mut succ: Vec<Vec<usize>> = vec![Vec::new(); nb];
    for b in 0..nb {
        for ins in &net.blocks[b] {
            let (rs, n) = ins.regs();
            for &r in &rs[..n] {
                let r = r as usize;
                if !bit(&defv[b], r) {
                    usev[b][r >> 6] |= 1 << (r & 63);
                }
            }
            if ins.d >= 0 {
                let r = ins.d as usize;
                defv[b][r >> 6] |= 1 << (r & 63);
            }
        }
        if let Some(t) = net.blocks[b].last() {
            for k in 0..t.nt as usize {
                if !succ[b].contains(&(t.t[k] as usize)) {
                    succ[b].push(t.t[k] as usize);
                }
            }
        }
    }
    let mut preds: Vec<Vec<usize>> = vec![Vec::new(); nb];
    for b in 0..nb {
        for &s in &succ[b] {
            preds[s].push(b);
        }
    }
    let mut live_in = usev.clone();
    let mut live_out = vec![vec![0u64; words]; nb];
    let mut work: Vec<usize> = (0..nb).rev().collect();
    let mut inwork = vec![true; nb];
    while let Some(b) = work.pop() {
        inwork[b] = false;
        let mut out = vec![0u64; words];
        for &s in &succ[b] {
            for w in 0..words {
                out[w] |= live_in[s][w];
            }
        }
        let mut inn = vec![0u64; words];
        for w in 0..words {
            inn[w] = usev[b][w] | (out[w] & !defv[b][w]);
        }
        live_out[b] = out;
        if inn != live_in[b] {
            live_in[b] = inn;
            for &p in &preds[b] {
                if !inwork[p] {
                    inwork[p] = true;
                    work.push(p);
                }
            }
        }
    }
    Live { words, live_in, live_out }
}

// ---------------------------------------------------------------------------
// One block: dependences and naming
// ---------------------------------------------------------------------------
const END: i32 = i32::MAX / 2;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Trap {
    None,
    Bad, // address out of range
    Div, // division by zero
}

#[derive(Clone, Copy)]
struct Eff {
    rd: i64,  // memory read: -2 none, -1 unknown address, >= 0 a literal address
    wr: i64,  // memory write: same coding
    trap: Trap,
    vis: bool,
}

fn effect(ins: &Ins) -> Eff {
    let mut e = Eff { rd: -2, wr: -2, trap: Trap::None, vis: false };
    let addr = |o: Opd| -> (i64, bool) {
        match o {
            Opd::L(v) if (v as u64) < MEM_WORDS as u64 => (v, true),
            _ => (-1, false),
        }
    };
    match ins.op {
        Op::Load => {
            let (a, safe) = addr(ins.a[0]);
            e.rd = a;
            if !safe {
                e.trap = Trap::Bad;
            }
        }
        Op::Store => {
            let (a, safe) = addr(ins.a[0]);
            e.wr = a;
            if !safe {
                e.trap = Trap::Bad;
            }
        }
        Op::Vload | Op::Vstore => e.vis = true,
        Op::Div | Op::Mod => {
            let ok = matches!(ins.a[1], Opd::L(v) if v & MASK != 0);
            if !ok {
                e.trap = Trap::Div;
            }
        }
        _ => {}
    }
    e
}

/// True if the instruction at the earlier position may not be exchanged with the later one
/// because of memory, traps or visible effects (data dependences are separate).
fn conflict(x: &Eff, y: &Eff) -> bool {
    let alias = |p: i64, q: i64| p == -1 || q == -1 || p == q;
    if x.wr != -2 && (y.rd != -2 || y.wr != -2) {
        if (y.rd != -2 && alias(x.wr, y.rd)) || (y.wr != -2 && alias(x.wr, y.wr)) {
            return true;
        }
    }
    if x.rd != -2 && y.wr != -2 && alias(x.rd, y.wr) {
        return true;
    }
    let sx = x.trap != Trap::None || x.wr != -2 || x.vis;
    let sy = y.trap != Trap::None || y.wr != -2 || y.vis;
    if x.vis && y.vis {
        return true;
    }
    if (x.trap != Trap::None && sy) || (y.trap != Trap::None && sx) {
        // two trapping reads of the same kind may be exchanged: nothing observable happens
        // between them
        let same_pure = x.trap == y.trap && x.wr == -2 && y.wr == -2 && !x.vis && !y.vis;
        return !same_pure;
    }
    false
}

pub struct Region {
    pub b: usize,
    pub n: usize,           // movable instructions; the terminator is `ins[n]`
    pub ins: Vec<Ins>,
    prod: Vec<[i32; 3]>,    // producer of each register operand: node, -1 live-in, -2 none
    pin: Vec<i32>,          // pinned destination name (value live after the block) or -1
    pub preds: Vec<Vec<usize>>,
    pub succs: Vec<Vec<usize>>,
    cands: Vec<i32>,        // names a block-local value may take
    cidx: Vec<i32>,         // name -> index into cands or -1
    boundary: Vec<bool>,    // per cand: live in or out of the block
    through: Vec<i32>,      // names live through the block without being touched or written
    livein: Vec<i32>,       // names read from the block's input state
    pub lat: Vec<i64>,
    pub cp: Vec<i64>,       // critical path to the end of the block
}

impl Region {
    pub fn new(net: &Net, lv: &Live, b: usize, sim: &Sim) -> Region {
        let ins = net.blocks[b].clone();
        let n = ins.len() - 1;
        let nn = net.nnames;
        let mut prod = vec![[-2i32; 3]; n + 1];
        let mut lastdef: Vec<i32> = vec![-1; nn];
        let mut defd = vec![false; nn];
        for i in 0..=n {
            let mut k2 = 0;
            for k in 0..ins[i].na as usize {
                if let Opd::R(x) = ins[i].a[k] {
                    prod[i][k2] = if defd[x as usize] { lastdef[x as usize] } else { -1 };
                    k2 += 1;
                }
            }
            if ins[i].d >= 0 {
                lastdef[ins[i].d as usize] = i as i32;
                defd[ins[i].d as usize] = true;
            }
        }
        let mut pin = vec![-1i32; n];
        for x in 0..nn {
            if defd[x] && bit(&lv.live_out[b], x) && lastdef[x] >= 0 && (lastdef[x] as usize) < n {
                pin[lastdef[x] as usize] = x as i32;
            }
        }
        // names read from the block's input state
        let mut livein: Vec<i32> = Vec::new();
        let mut through: Vec<i32> = Vec::new();
        for x in 0..nn {
            let li = bit(&lv.live_in[b], x);
            if li {
                livein.push(x as i32);
                if !defd[x] && bit(&lv.live_out[b], x) {
                    through.push(x as i32);
                }
            }
        }
        // candidate names: everything the block mentions, the live names, and one name the
        // block does not mention at all
        let mut mention = vec![false; nn];
        let mut boundary_n = vec![false; nn];
        for i in 0..=n {
            if ins[i].d >= 0 {
                mention[ins[i].d as usize] = true;
            }
            let (rs, c) = ins[i].regs();
            for &r in &rs[..c] {
                mention[r as usize] = true;
            }
        }
        for x in 0..nn {
            if bit(&lv.live_in[b], x) || bit(&lv.live_out[b], x) {
                mention[x] = true;
                boundary_n[x] = true;
            }
        }
        let mut cands: Vec<i32> = (0..nn as i32).filter(|&x| mention[x as usize]).collect();
        if let Some(x) = (0..nn as i32).find(|&x| !mention[x as usize]) {
            cands.push(x);
        }
        let mut cidx = vec![-1i32; nn];
        for (c, &x) in cands.iter().enumerate() {
            cidx[x as usize] = c as i32;
        }
        let boundary: Vec<bool> = cands.iter().map(|&x| boundary_n[x as usize]).collect();
        // dependences
        let mut preds: Vec<Vec<usize>> = vec![Vec::new(); n];
        let eff: Vec<Eff> = (0..n).map(|i| effect(&ins[i])).collect();
        for i in 0..n {
            for k in 0..3 {
                let p = prod[i][k];
                if p >= 0 {
                    preds[i].push(p as usize);
                }
            }
            for j in 0..i {
                if conflict(&eff[j], &eff[i]) {
                    preds[i].push(j);
                }
            }
        }
        // a pinned definition of a name must come after every read of that name's input value
        for j in 0..n {
            if pin[j] >= 0 {
                let x = pin[j];
                for i in 0..=n {
                    if i == j || i > n {
                        continue;
                    }
                    let mut k2 = 0;
                    for k in 0..ins[i].na as usize {
                        if let Opd::R(r) = ins[i].a[k] {
                            if r == x && prod[i][k2] == -1 && i < n {
                                preds[j].push(i);
                            }
                            k2 += 1;
                        }
                    }
                }
            }
        }
        for p in preds.iter_mut() {
            p.sort_unstable();
            p.dedup();
        }
        let mut succs: Vec<Vec<usize>> = vec![Vec::new(); n];
        for i in 0..n {
            for &p in &preds[i] {
                succs[p].push(i);
            }
        }
        let mut lat = vec![0i64; n];
        for i in 0..n {
            lat[i] = if ins[i].d >= 0 { sim.lat(ins[i].op) } else { 1 };
        }
        let mut cp = vec![0i64; n];
        let mut tail = vec![0i64; n];
        for k in 0..3 {
            let p = prod[n][k];
            if p >= 0 {
                tail[p as usize] = 1;
            }
        }
        for i in (0..n).rev() {
            let mut m = tail[i];
            for &s in &succs[i] {
                m = m.max(cp[s]);
            }
            cp[i] = lat[i] + m;
        }
        Region { b, n, ins, prod, pin, preds, succs, cands, cidx, boundary, through, livein, lat, cp }
    }

    /// Choose names for the block-local values of `order`, writing them to `names` (one per
    /// node, -1 for nodes without destination). Pinned values keep their name.
    pub fn name_block(&self, order: &[usize], sim: &Sim, names: &mut Vec<i32>, mode: u8) {
        let n = self.n;
        let nc = self.cands.len();
        let mut pos = vec![0i32; n];
        for (p, &i) in order.iter().enumerate() {
            pos[i] = p as i32;
        }
        // last use of every value and of every input name
        let mut lastuse = vec![-1i32; n + 1];
        let mut lastin: Vec<i32> = vec![-1; nc];
        for i in 0..=n {
            let p = if i == n { n as i32 } else { pos[i] };
            let mut k2 = 0;
            for k in 0..self.ins[i].na as usize {
                if let Opd::R(x) = self.ins[i].a[k] {
                    let pr = self.prod[i][k2];
                    k2 += 1;
                    if pr >= 0 {
                        let pr = pr as usize;
                        if p > lastuse[pr] {
                            lastuse[pr] = p;
                        }
                    } else if pr == -1 {
                        let c = self.cidx[x as usize] as usize;
                        if p > lastin[c] {
                            lastin[c] = p;
                        }
                    }
                }
            }
        }
        let mut busy: Vec<Vec<(i32, i32)>> = vec![Vec::new(); nc];
        for &x in &self.livein {
            let c = self.cidx[x as usize] as usize;
            let end = if self.through.contains(&x) { END } else { lastin[c] };
            busy[c].push((-1, end));
        }
        names.clear();
        names.resize(n, -1);
        for i in 0..n {
            if self.pin[i] >= 0 {
                let c = self.cidx[self.pin[i] as usize] as usize;
                busy[c].push((pos[i], END));
                names[i] = self.pin[i];
            }
        }
        let mut touched: Vec<bool> = self.boundary.clone();
        let mut wr = vec![0i64; nc]; // estimated completion time of the last write
        // a light timing model (slots and latencies) to see which names are still waited for
        let (mut last, mut used, mut memused) = (0i64, 0i64, 0i64);
        let mut vready = vec![0i64; n];
        for &i in order {
            let ins = &self.ins[i];
            let mut t = last;
            let mut k2 = 0;
            for k in 0..ins.na as usize {
                if let Opd::R(_) = ins.a[k] {
                    let pr = self.prod[i][k2];
                    k2 += 1;
                    if pr >= 0 {
                        t = t.max(vready[pr as usize]);
                    }
                }
            }
            if ins.d >= 0 && self.pin[i] < 0 {
                let p = pos[i];
                let q = if lastuse[i] >= 0 { lastuse[i] } else { p };
                let mut best: Option<((i64, u8, u8, i32), usize)> = None;
                for c in 0..nc {
                    let x = self.cands[c];
                    let mut ok = true;
                    for &(a, b) in &busy[c] {
                        if p < b && a < q {
                            ok = false;
                            break;
                        }
                    }
                    if !ok {
                        continue;
                    }
                    let delay = (wr[c] - t).max(0);
                    let key = if mode == 0 { (delay, if touched[c] { 0u8 } else { 1u8 }, if x == ins.d { 0u8 } else { 1u8 }, x) } else { (delay, if x == ins.d { 0u8 } else { 1u8 }, if touched[c] { 0u8 } else { 1u8 }, x) };
                    if best.map_or(true, |(bk, _)| key < bk) {
                        best = Some((key, c));
                    }
                }
                // a legal name always exists: the value's original name is free in the
                // original order; for other orders fall back to any free candidate, and
                // if there is none, to a failure marker
                match best {
                    Some((_, c)) => {
                        names[i] = self.cands[c];
                        busy[c].push((p, q));
                        touched[c] = true;
                    }
                    None => {
                        names.clear();
                        return;
                    }
                }
            }
            // light timing
            let m = ins.op.is_mem();
            if t == last && (used >= sim_w(sim) || (m && memused >= sim_mp(sim))) {
                t += 1;
            }
            if t > last {
                used = 0;
                memused = 0;
            }
            used += 1;
            if m {
                memused += 1;
            }
            last = t;
            vready[i] = t + self.lat[i];
            if ins.d >= 0 {
                let c = self.cidx[names[i] as usize] as usize;
                wr[c] = vready[i];
            }
        }
    }

    /// The block's instructions in `order` with the destination and operand names of `names`.
    pub fn apply(&self, order: &[usize], names: &[i32]) -> Vec<Ins> {
        let n = self.n;
        let mut out: Vec<Ins> = Vec::with_capacity(n + 1);
        for idx in 0..=n {
            let i = if idx < n { order[idx] } else { n };
            let mut ins = self.ins[i].clone();
            let mut k2 = 0;
            for k in 0..ins.na as usize {
                if let Opd::R(_) = ins.a[k] {
                    let pr = self.prod[i][k2];
                    k2 += 1;
                    if pr >= 0 {
                        ins.a[k] = Opd::R(names[pr as usize]);
                    }
                }
            }
            if i < n && ins.d >= 0 {
                ins.d = names[i];
            }
            out.push(ins);
        }
        out
    }

    /// True if `order` respects every dependence.
    pub fn legal(&self, order: &[usize]) -> bool {
        let mut pos = vec![usize::MAX; self.n];
        if order.len() != self.n {
            return false;
        }
        for (p, &i) in order.iter().enumerate() {
            if i >= self.n || pos[i] != usize::MAX {
                return false;
            }
            pos[i] = p;
        }
        for i in 0..self.n {
            for &p in &self.preds[i] {
                if pos[p] >= pos[i] {
                    return false;
                }
            }
        }
        true
    }
}

#[inline]
fn sim_w(s: &Sim) -> i64 {
    s.w
}
#[inline]
fn sim_mp(s: &Sim) -> i64 {
    s.mp
}

// ---------------------------------------------------------------------------
// Windows: the real execution around a block
// ---------------------------------------------------------------------------
const PRE_INSTRS: usize = 48;
const POST_INSTRS: usize = 32;

#[derive(Clone)]
struct Window {
    seq: Vec<u32>,
    from_start: bool,
}

pub struct Opt<'a> {
    pub net: Net,
    mc: &'a Machine,
    sim: Sim,
    lv: Live,
    seqs: Vec<Vec<u32>>,
    occ: Vec<Vec<Vec<u32>>>, // per input, per block: positions in the sequence
    rng: Rng,
    pub max_steps: u64,
    pub evals: u64,
}

impl<'a> Opt<'a> {
    pub fn new(net: Net, mc: &'a Machine, traces: &[Vec<u32>], max_steps: u64) -> Opt<'a> {
        let sim = Sim::new(mc, net.nnames);
        let lv = liveness(&net);
        let nb = net.blocks.len();
        let mut seqs = Vec::new();
        let mut occ = Vec::new();
        for tr in traces {
            let seq = net.block_seq(tr);
            let mut o: Vec<Vec<u32>> = vec![Vec::new(); nb];
            for (i, &b) in seq.iter().enumerate() {
                o[b as usize].push(i as u32);
            }
            seqs.push(seq);
            occ.push(o);
        }
        Opt { net, mc, sim, lv, seqs, occ, rng: Rng::new(0x5EED_5C4D), max_steps, evals: 0 }
    }

    fn windows(&self, b: usize) -> Vec<Window> {
        let mut out = Vec::new();
        for (si, seq) in self.seqs.iter().enumerate() {
            let o = &self.occ[si][b];
            if o.is_empty() {
                continue;
            }
            let picks: Vec<usize> = if o.len() <= 2 { (0..o.len()).collect() } else { vec![o.len() / 3, 2 * o.len() / 3] };
            for pi in picks {
                let pos = o[pi] as usize;
                let mut lo = pos;
                let mut cnt = 0usize;
                while lo > 0 && cnt < PRE_INSTRS && pos - lo < 24 {
                    lo -= 1;
                    cnt += self.net.blocks[seq[lo] as usize].len();
                }
                let mut hi = pos + 1;
                let mut cnt2 = 0usize;
                while hi < seq.len() && cnt2 < POST_INSTRS && hi - pos <= 12 {
                    cnt2 += self.net.blocks[seq[hi] as usize].len();
                    hi += 1;
                }
                out.push(Window { seq: seq[lo..hi].to_vec(), from_start: lo == 0 });
                if out.len() >= 4 {
                    return out;
                }
            }
        }
        out
    }

    fn eval(&mut self, b: usize, code: Vec<Ins>, wins: &[Window]) -> i64 {
        self.evals += 1;
        let old = std::mem::replace(&mut self.net.blocks[b], code);
        let np = self.net.params.len();
        let mut tot = 0i64;
        for w in wins {
            tot += self.sim.run_seq(&self.net.blocks, &self.net.start, &w.seq, np, w.from_start);
        }
        self.net.blocks[b] = old;
        tot
    }

    fn try_order(&mut self, reg: &Region, order: &[usize], wins: &[Window]) -> Option<(i64, Vec<Ins>)> {
        let mut best: Option<(i64, Vec<Ins>)> = None;
        for mode in 0..2u8 {
            let mut names: Vec<i32> = Vec::new();
            reg.name_block(order, &self.sim, &mut names, mode);
            if names.len() != reg.n {
                continue;
            }
            let code = reg.apply(order, &names);
            let c = self.eval(reg.b, code.clone(), wins);
            if best.as_ref().map_or(true, |x| c < x.0) {
                best = Some((c, code));
            }
        }
        best
    }

    /// Cycle-driven list scheduling on a light model: among the ready instructions take the
    /// one that issues first (slots and operand latencies), ties by critical path.
    fn list_order(&self, reg: &Region, mode: u8) -> Vec<usize> {
        let n = reg.n;
        let mut npred: Vec<usize> = reg.preds.iter().map(|p| p.len()).collect();
        let mut ready: Vec<usize> = (0..n).filter(|&i| npred[i] == 0).collect();
        let mut order = Vec::with_capacity(n);
        let mut vready = vec![0i64; n];
        let (mut last, mut used, mut memused) = (0i64, 0i64, 0i64);
        let (w, mp) = (sim_w(&self.sim), sim_mp(&self.sim));
        let est = |i: usize, vready: &Vec<i64>, last: i64, used: i64, memused: i64| -> i64 {
            let mut t = last;
            for k in 0..3 {
                let p = reg.prod_of(i, k);
                if p >= 0 {
                    t = t.max(vready[p as usize]);
                }
            }
            let m = reg.ins[i].op.is_mem();
            if t == last && (used >= w || (m && memused >= mp)) {
                t += 1;
            }
            t
        };
        while !ready.is_empty() {
            let mut bi = 0;
            let mut bk: Option<(i64, i64, usize)> = None;
            for (ri, &i) in ready.iter().enumerate() {
                let e = est(i, &vready, last, used, memused);
                let key = match mode {
                    0 => (e, -reg.cp[i], i),
                    1 => (-reg.cp[i], e, i),
                    _ => (e, 0, i),
                };
                if bk.map_or(true, |k| key < k) {
                    bk = Some(key);
                    bi = ri;
                }
            }
            let i = ready.swap_remove(bi);
            let t = est(i, &vready, last, used, memused);
            if t > last {
                used = 0;
                memused = 0;
            }
            used += 1;
            if reg.ins[i].op.is_mem() {
                memused += 1;
            }
            last = t;
            vready[i] = t + reg.lat[i];
            order.push(i);
            for &s in &reg.succs[i] {
                npred[s] -= 1;
                if npred[s] == 0 {
                    ready.push(s);
                }
            }
        }
        order
    }

    /// Optimise one block against its windows. Returns true if the block changed.
    pub fn opt_block(&mut self, b: usize, iters: usize) -> bool {
        let n_ins = self.net.blocks[b].len();
        if n_ins < 3 || n_ins > 240 {
            return false;
        }
        let wins = self.windows(b);
        if wins.is_empty() {
            return false;
        }
        let reg = Region::new(&self.net, &self.lv, b, &self.sim);
        let n = reg.n;
        let orig_code = self.net.blocks[b].clone();
        let c0 = self.eval(b, orig_code.clone(), &wins);
        let mut best: (i64, Vec<Ins>) = (c0, orig_code);
        // seeds
        let mut cur: Option<(i64, Vec<usize>)> = None;
        let mut seeds: Vec<Vec<usize>> = vec![(0..n).collect()];
        for mode in 0..3u8 {
            let o = self.list_order(&reg, mode);
            if !seeds.contains(&o) {
                seeds.push(o);
            }
        }
        for o in seeds {
            if !reg.legal(&o) {
                continue;
            }
            if let Some((c, code)) = self.try_order(&reg, &o, &wins) {
                if c < best.0 {
                    best = (c, code);
                }
                if cur.as_ref().map_or(true, |x| c < x.0) {
                    cur = Some((c, o));
                }
            }
        }
        // small blocks: enumerate legal orders (leaf budget), each scored exactly with naming
        if n <= 10 {
            let mut leaves = 0usize;
            let mut npred: Vec<usize> = reg.preds.iter().map(|p| p.len()).collect();
            let mut order: Vec<usize> = Vec::with_capacity(n);
            let mut next: Vec<usize> = vec![0]; // per depth: next candidate node to try
            'dfs: while !next.is_empty() {
                if leaves >= 700 || self.sim.steps > self.max_steps {
                    break;
                }
                let depth = next.len() - 1;
                if order.len() == n {
                    leaves += 1;
                    if let Some((c, code)) = self.try_order(&reg, &order, &wins) {
                        if c < best.0 {
                            best = (c, code);
                            cur = Some((c, order.clone()));
                        }
                    }
                    // backtrack
                    next.pop();
                    if let Some(i) = order.pop() {
                        for &s in &reg.succs[i] {
                            npred[s] += 1;
                        }
                    }
                    continue;
                }
                let mut i = next[depth];
                while i < n && (npred[i] != 0 || order.contains(&i)) {
                    i += 1;
                }
                if i >= n {
                    next.pop();
                    if let Some(j) = order.pop() {
                        for &s in &reg.succs[j] {
                            npred[s] += 1;
                        }
                    }
                    continue 'dfs;
                }
                next[depth] = i + 1;
                order.push(i);
                for &s in &reg.succs[i] {
                    npred[s] -= 1;
                }
                next.push(0);
            }
        }
        // local search: move one instruction to another legal position, keep if not worse
        if let Some((mut cc, mut order)) = cur {
            let mut pos = vec![0usize; n];
            for _ in 0..iters {
                if self.sim.steps > self.max_steps {
                    break;
                }
                for (p, &i) in order.iter().enumerate() {
                    pos[i] = p;
                }
                let i = self.rng.below(n as u64) as usize;
                let pi = pos[i];
                let idx = |x: usize| if pos[x] < pi { pos[x] } else { pos[x] - 1 };
                let mut lo = 0usize;
                for &p in &reg.preds[i] {
                    lo = lo.max(idx(p) + 1);
                }
                let mut hi = n - 1;
                for &s in &reg.succs[i] {
                    hi = hi.min(idx(s));
                }
                if lo > hi {
                    continue;
                }
                let q = lo + self.rng.below((hi - lo + 1) as u64) as usize;
                if q == pi {
                    continue;
                }
                let mut cand = order.clone();
                cand.remove(pi);
                cand.insert(q, i);
                if let Some((c, code)) = self.try_order(&reg, &cand, &wins) {
                    if c <= cc {
                        if c < best.0 {
                            best = (c, code);
                        }
                        cc = c;
                        order = cand;
                    }
                }
            }
        }
        if best.0 < c0 {
            self.net.blocks[b] = best.1;
            true
        } else {
            false
        }
    }
}

impl Region {
    #[inline]
    fn prod_of(&self, i: usize, k: usize) -> i32 {
        self.prod[i][k]
    }
}

/// Improve the order and the block-local names of `text`. `inputs` are typical inputs of
/// the program; their executions give the contexts a block is scored in. Returns the new
/// text, or None if nothing was changed or the program is not suited.
pub fn optimize(text: &str, mc: &Machine, inputs: &[Inputs], max_steps: u64) -> Option<String> {
    let prog = parse_program(text).ok()?;
    if prog.instrs.len() > 4000 || prog.instrs.len() < 4 {
        return None;
    }
    let net = Net::from_program(&prog);
    if net.nnames > 400 {
        return None;
    }
    let cp = compile_program(&prog);
    let mut traces = Vec::new();
    for inp in inputs.iter().take(3) {
        let r = execute(&cp, inp, 60_000, true);
        traces.push(r.trace);
    }
    let mut opt = Opt::new(net, mc, &traces, max_steps);
    // hottest blocks first
    let nb = opt.net.blocks.len();
    let mut heat: Vec<(u64, usize)> = Vec::new();
    let mut total = 0u64;
    for b in 0..nb {
        let cnt: u64 = opt.occ.iter().map(|o| o[b].len() as u64).sum();
        let h = cnt * opt.net.blocks[b].len() as u64;
        total += h;
        heat.push((h, b));
    }
    heat.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    let mut changed = false;
    for (h, b) in heat {
        if h * 200 < total || h == 0 {
            break;
        }
        let n = opt.net.blocks[b].len();
        let iters = (60 + 25 * n).min(1200);
        if opt.opt_block(b, iters) {
            changed = true;
        }
        if opt.sim.steps > max_steps {
            break;
        }
    }
    if changed {
        Some(opt.net.to_text())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bench_dir() -> Option<std::path::PathBuf> {
        for p in ["kit/bench", "../kit/bench", "../../kit/bench", "../../../kit/bench"] {
            let d = std::path::PathBuf::from(p);
            if d.is_dir() {
                return Some(d);
            }
        }
        None
    }

    fn machines() -> Vec<Machine> {
        let mut v = Vec::new();
        for (k, sp, w, mp) in [(8, 5, 2, 1), (4, 8, 2, 1), (16, 5, 2, 1), (3, 20, 3, 2), (6, 1, 4, 2)] {
            let mut m = Machine::default();
            m.issue_width = w;
            m.mem_ports = mp;
            m.registers = k;
            m.reg_spill_penalty = sp;
            m.taken_branch_penalty = 2;
            m.icache_size = 24;
            m.icache_penalty = 2;
            m.lat_alu = 1;
            m.lat_mul = 3;
            m.lat_div = 12;
            m.lat_load = 4;
            m.lat_mac = 3;
            m.lat_addsh = 1;
            m.fused_mac = 1;
            m.fused_addsh = 1;
            v.push(m);
        }
        v
    }

    fn kir_files() -> Vec<std::path::PathBuf> {
        let dir = match bench_dir() {
            Some(d) => d,
            None => return Vec::new(),
        };
        let mut files: Vec<_> = std::fs::read_dir(&dir).unwrap().filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.extension().map_or(false, |x| x == "kir")).collect();
        files.sort();
        files
    }

    /// The step-by-step model equals `time_trace` on whole runs.
    #[test]
    fn sim_matches_time_trace() {
        let mut checked = 0;
        for f in kir_files().iter().take(70) {
            let text = std::fs::read_to_string(f).unwrap();
            let prog = match parse_program(&text) {
                Ok(p) => p,
                Err(_) => continue,
            };
            let cp = compile_program(&prog);
            let net = Net::from_program(&prog);
            for mc in machines() {
                for inp in generate_inputs(prog.params.len(), 3, 11) {
                    let r = execute(&cp, &inp, 20_000, true);
                    if r.outcome == Outcome::Timeout {
                        continue;
                    }
                    let last_pc = *r.trace.last().unwrap() as usize;
                    let b = net.block_of[last_pc] as usize;
                    if last_pc != net.start[b] + net.blocks[b].len() - 1 {
                        continue; // run ended inside a block (trap)
                    }
                    let want = time_trace(&cp, &r.trace, &mc);
                    let seq = net.block_seq(&r.trace);
                    let mut sim = Sim::new(&mc, net.nnames);
                    let got = sim.run_seq(&net.blocks, &net.start, &seq, net.params.len(), true);
                    assert_eq!(got, want, "{} machine K={}", f.display(), mc.registers);
                    checked += 1;
                }
            }
        }
        assert!(checked > 20 || kir_files().is_empty());
    }

    /// Printing and re-parsing keeps the program.
    #[test]
    fn print_roundtrip() {
        for p in kir_files().iter().take(80) {
            let text = std::fs::read_to_string(p).unwrap();
            let prog = match parse_program(&text) {
                Ok(p) => p,
                Err(_) => continue,
            };
            let net = Net::from_program(&prog);
            let t2 = net.to_text();
            let p2 = parse_program(&t2).unwrap();
            assert_eq!(p2.instrs.len(), prog.instrs.len());
            let c1 = compile_program(&prog);
            let c2 = compile_program(&p2);
            for inp in generate_inputs(prog.params.len(), 4, 5) {
                let r1 = execute(&c1, &inp, 20_000, false);
                let r2 = execute(&c2, &inp, 20_000, false);
                assert!(!differs(&r1, &r2));
            }
        }
    }

    /// Optimising keeps behaviour.
    #[test]
    fn optimize_keeps_behaviour() {
        let mut changed = 0;
        for f in kir_files().iter().step_by(3).take(30) {
            let text = std::fs::read_to_string(f).unwrap();
            let prog = parse_program(&text).unwrap();
            let cp = compile_program(&prog);
            for mc in machines().into_iter().take(2) {
                let inputs = generate_inputs(prog.params.len(), 3, 77);
                if let Some(t2) = optimize(&text, &mc, &inputs, 3_000_000) {
                    changed += 1;
                    let p2 = parse_program(&t2).unwrap();
                    let c2 = compile_program(&p2);
                    for inp in generate_inputs(prog.params.len(), 30, 99) {
                        let r1 = execute(&cp, &inp, 30_000, false);
                        let r2 = execute(&c2, &inp, 60_000, false);
                        assert!(!differs(&r1, &r2), "{}", f.display());
                    }
                }
            }
        }
        eprintln!("changed {}", changed);
    }
}
