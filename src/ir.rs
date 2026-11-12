//! ir.rs - the in-memory form of a KIR function used by kopt.
//!
//! Operands are plain i64:  x >= 0 is a register (or, in SSA form, a value number);
//! x < 0 is the literal !x (0 .. 2^32-1).
//! Blocks live in an arena and are addressed by their id; `Func::blocks` is the layout order.
use crate::kir::*;
use std::collections::{HashMap, HashSet};

pub const NOD: i64 = i64::MIN; // "no destination"

pub fn lit(v: i64) -> i64 {
    !(v & MASK)
}
pub const L0: i64 = !0;
pub const L1: i64 = !1;

pub fn is_binop(op: Op) -> bool {
    matches!(op, Op::Add | Op::Sub | Op::Mul | Op::And | Op::Or | Op::Xor | Op::Shl | Op::Shr | Op::Lt | Op::Eq | Op::Div | Op::Mod)
}
pub fn is_comm(op: Op) -> bool {
    matches!(op, Op::Add | Op::Mul | Op::And | Op::Or | Op::Xor | Op::Eq)
}
/// Ops with a destination and no effect other than (for div/mod/load) a possible trap.
pub fn is_pure(op: Op) -> bool {
    matches!(op, Op::Add | Op::Sub | Op::Mul | Op::And | Op::Or | Op::Xor | Op::Shl | Op::Shr | Op::Lt | Op::Eq | Op::Select | Op::Mac | Op::Addsh)
}

/// Snapshot of a defining instruction (op and its first two operands).
#[derive(Clone, Copy, Debug)]
pub struct DefInfo {
    pub op: Op,
    pub a0: i64,
    pub a1: i64,
}

impl DefInfo {
    pub fn of(ins: &Instr) -> DefInfo {
        DefInfo { op: ins.op, a0: ins.a.first().copied().unwrap_or(L0), a1: ins.a.get(1).copied().unwrap_or(L0) }
    }
}

#[derive(Clone, Debug)]
pub struct Instr {
    pub op: Op,
    pub d: i64,
    pub a: Vec<i64>,
    pub t: Vec<usize>,
    pub pv: i64, // phi construction: the original register
}

impl Instr {
    pub fn new(op: Op, d: i64, a: Vec<i64>, t: Vec<usize>) -> Instr {
        Instr { op, d, a, t, pv: 0 }
    }
    pub fn no_d(op: Op, a: Vec<i64>, t: Vec<usize>) -> Instr {
        Instr { op, d: NOD, a, t, pv: 0 }
    }
    pub fn has_d(&self) -> bool {
        self.d != NOD
    }
}

#[derive(Clone)]
pub struct Block {
    pub id: usize,
    pub name: String,
    pub phis: Vec<Instr>,
    pub ins: Vec<Instr>,
    pub preds: Vec<usize>,
    pub idom: Option<usize>,
    pub dkids: Vec<usize>,
    pub rpo: i64,
    pub pre: usize,
    pub post: usize,
    pub depth: usize,
    pub freq: f64,
    pub lp: Option<usize>,
}

impl Block {
    pub fn new(id: usize, name: String) -> Block {
        Block { id, name, phis: Vec::new(), ins: Vec::new(), preds: Vec::new(), idom: None, dkids: Vec::new(), rpo: -1, pre: 0, post: 0, depth: 0, freq: 1.0, lp: None }
    }
    pub fn term(&self) -> &Instr {
        self.ins.last().unwrap()
    }
    pub fn succs(&self) -> &[usize] {
        match self.ins.last() {
            Some(i) => &i.t,
            None => &[],
        }
    }
}

#[derive(Clone)]
pub struct Loop {
    pub header: usize,
    pub blocks: Vec<usize>,
    pub ids: HashSet<usize>,
    pub latches: Vec<usize>,
    pub parent: Option<usize>,
    pub depth: usize,
}

#[derive(Clone)]
pub struct Func {
    pub name: String,
    pub params: Vec<i64>,
    pub arena: Vec<Block>,
    pub blocks: Vec<usize>,
    pub nval: i64,
    pub names: HashSet<String>,
    pub rpo: Vec<usize>,
    pub loops: Vec<Loop>,
    pub nreg: i64,
}

impl Func {
    pub fn new() -> Func {
        Func { name: String::new(), params: Vec::new(), arena: Vec::new(), blocks: Vec::new(), nval: 0, names: HashSet::new(), rpo: Vec::new(), loops: Vec::new(), nreg: 0 }
    }
    pub fn new_val(&mut self) -> i64 {
        let v = self.nval;
        self.nval += 1;
        v
    }
    /// Creates a block in the arena (it is NOT added to the layout order).
    pub fn new_block(&mut self, hint: &str) -> usize {
        let mut i = self.arena.len();
        let mut name = format!("{}_{}", hint, i);
        while self.names.contains(&name) {
            i += 1;
            name = format!("{}_{}", hint, i);
        }
        let id = self.arena.len();
        self.arena.push(Block::new(id, name.clone()));
        self.names.insert(name);
        id
    }
    pub fn b(&self, id: usize) -> &Block {
        &self.arena[id]
    }
    pub fn bm(&mut self, id: usize) -> &mut Block {
        &mut self.arena[id]
    }
    pub fn entry(&self) -> usize {
        self.blocks[0]
    }
    pub fn succs(&self, id: usize) -> Vec<usize> {
        self.arena[id].succs().to_vec()
    }
    pub fn dominates(&self, a: usize, b: usize) -> bool {
        let (x, y) = (&self.arena[a], &self.arena[b]);
        x.pre <= y.pre && y.post <= x.post
    }
    pub fn count_instrs(&self) -> usize {
        self.blocks.iter().map(|&b| self.arena[b].ins.len()).sum()
    }
}

pub fn from_program(prog: &Program) -> Func {
    let mut f = Func::new();
    f.name = prog.name.clone();
    f.params = prog.params.clone();
    let mut byname: HashMap<&str, usize> = HashMap::new();
    for name in &prog.block_labels {
        let id = f.arena.len();
        f.arena.push(Block::new(id, name.clone()));
        f.names.insert(name.clone());
        f.blocks.push(id);
        byname.insert(name.as_str(), id);
    }
    let mut maxreg: i64 = -1;
    for &p in &prog.params {
        maxreg = maxreg.max(p);
    }
    for (i, pi) in prog.instrs.iter().enumerate() {
        let bid = prog.block_of[i];
        let mut op = pi.op;
        let mut d = NOD;
        let mut a = Vec::new();
        let mut t: Vec<usize> = Vec::new();
        for (k, arg) in op.sig().as_bytes().iter().zip(pi.args.iter()) {
            match (*k, arg) {
                (b'd', Arg::R(r)) => {
                    d = *r;
                    maxreg = maxreg.max(*r);
                }
                (_, Arg::R(r)) => {
                    a.push(*r);
                    maxreg = maxreg.max(*r);
                }
                (_, Arg::I(v)) | (_, Arg::K(v)) => a.push(!*v),
                (_, Arg::L(l)) => t.push(byname[l.as_str()]),
            }
        }
        if op == Op::Br && t[0] == t[1] {
            op = Op::Jmp;
            a.clear();
            t = vec![t[0]];
        }
        f.arena[bid].ins.push(Instr::new(op, d, a, t));
    }
    f.nval = maxreg + 1;
    compute_preds(&mut f);
    f
}

pub fn to_text(f: &Func) -> String {
    let mut out = String::new();
    out.push_str("func ");
    out.push_str(&f.name);
    out.push('(');
    for (i, p) in f.params.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        out.push_str(&format!("v{}", p));
    }
    out.push_str(")\n");
    for &bid in &f.blocks {
        let b = &f.arena[bid];
        out.push_str(&b.name);
        out.push_str(":\n");
        for ins in &b.ins {
            out.push_str("  ");
            out.push_str(ins.op.name());
            out.push(' ');
            let mut first = true;
            let mut sep = |out: &mut String| {
                if !first {
                    out.push_str(", ");
                }
                first = false;
            };
            if ins.has_d() {
                sep(&mut out);
                out.push_str(&format!("v{}", ins.d));
            }
            for &x in &ins.a {
                sep(&mut out);
                if x >= 0 {
                    out.push_str(&format!("v{}", x));
                } else {
                    out.push_str(&format!("{}", !x));
                }
            }
            for &t in &ins.t {
                sep(&mut out);
                out.push_str(&f.arena[t].name);
            }
            out.push('\n');
        }
    }
    out
}

// ---------------------------------------------------------------------------
// CFG utilities
// ---------------------------------------------------------------------------
/// Recompute preds from terminators (only valid when there are no phis).
pub fn compute_preds(f: &mut Func) {
    for i in 0..f.blocks.len() {
        let b = f.blocks[i];
        f.arena[b].preds.clear();
    }
    for i in 0..f.blocks.len() {
        let b = f.blocks[i];
        let succs = f.arena[b].succs().to_vec();
        for s in succs {
            if !f.arena[s].preds.contains(&b) {
                f.arena[s].preds.push(b);
            }
        }
    }
}

/// Remove p from s.preds and drop the matching phi operands (the terminator is the caller's job).
pub fn remove_edge(f: &mut Func, p: usize, s: usize) {
    let blk = &mut f.arena[s];
    let i = blk.preds.iter().position(|&x| x == p).expect("remove_edge: not a predecessor");
    blk.preds.remove(i);
    for phi in blk.phis.iter_mut() {
        phi.a.remove(i);
    }
}

/// Delete blocks that cannot be reached from the entry. Keeps phis consistent.
pub fn remove_unreachable(f: &mut Func) -> bool {
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
    if count == f.blocks.len() {
        return false;
    }
    for i in 0..f.blocks.len() {
        let bid = f.blocks[i];
        if seen[bid] && !f.arena[bid].preds.is_empty() {
            let dead: Vec<usize> = f.arena[bid].preds.iter().enumerate().filter(|(_, p)| !seen[**p]).map(|(i, _)| i).collect();
            for &i in dead.iter().rev() {
                f.arena[bid].preds.remove(i);
                for phi in f.arena[bid].phis.iter_mut() {
                    phi.a.remove(i);
                }
            }
        }
    }
    f.blocks.retain(|&b| seen[b]);
    true
}

/// Remove unreachable blocks, then compute RPO, dominators, dominator tree, loops, depth.
pub fn analyze(f: &mut Func) -> Vec<usize> {
    remove_unreachable(f);
    let entry = f.blocks[0];
    let n = f.arena.len();
    let mut post: Vec<usize> = Vec::new();
    let mut seen = vec![false; n];
    seen[entry] = true;
    let mut stack: Vec<(usize, usize)> = vec![(entry, 0)];
    while let Some((b, i)) = stack.pop() {
        let nsucc = f.arena[b].succs().len();
        if i < nsucc {
            stack.push((b, i + 1));
            let s = f.arena[b].succs()[i];
            if !seen[s] {
                seen[s] = true;
                stack.push((s, 0));
            }
        } else {
            post.push(b);
        }
    }
    let rpo: Vec<usize> = post.into_iter().rev().collect();
    for (i, &b) in rpo.iter().enumerate() {
        let blk = &mut f.arena[b];
        blk.rpo = i as i64;
        blk.idom = None;
        blk.dkids.clear();
        blk.depth = 0;
        blk.lp = None;
    }
    f.arena[entry].idom = Some(entry);
    let mut changed = true;
    while changed {
        changed = false;
        for &b in &rpo[1..] {
            let mut new: Option<usize> = None;
            for k in 0..f.arena[b].preds.len() {
                let p = f.arena[b].preds[k];
                if f.arena[p].idom.is_none() {
                    continue;
                }
                match new {
                    None => new = Some(p),
                    Some(nw) => {
                        let (mut x, mut y) = (p, nw);
                        while x != y {
                            while f.arena[x].rpo > f.arena[y].rpo {
                                x = f.arena[x].idom.unwrap();
                            }
                            while f.arena[y].rpo > f.arena[x].rpo {
                                y = f.arena[y].idom.unwrap();
                            }
                        }
                        new = Some(x);
                    }
                }
            }
            if f.arena[b].idom != new {
                f.arena[b].idom = new;
                changed = true;
            }
        }
    }
    for &b in &rpo[1..] {
        let id = f.arena[b].idom.unwrap();
        f.arena[id].dkids.push(b);
    }
    // pre/post numbering of the dominator tree
    let mut num = 0usize;
    let mut stack: Vec<(usize, usize)> = vec![(entry, 0)];
    f.arena[entry].pre = 0;
    while let Some((b, i)) = stack.pop() {
        if i == 0 {
            f.arena[b].pre = num;
            num += 1;
        }
        if i < f.arena[b].dkids.len() {
            stack.push((b, i + 1));
            let k = f.arena[b].dkids[i];
            stack.push((k, 0));
        } else {
            f.arena[b].post = num;
            num += 1;
        }
    }
    f.rpo = rpo.clone();
    find_loops(f);
    rpo
}

/// Natural loops (back edge = edge to a dominator). Loops sharing a header are merged.
pub fn find_loops(f: &mut Func) {
    let mut loops: Vec<Loop> = Vec::new();
    let mut by_header: HashMap<usize, usize> = HashMap::new();
    for k in 0..f.rpo.len() {
        let b = f.rpo[k];
        for j in 0..f.arena[b].succs().len() {
            let s = f.arena[b].succs()[j];
            if f.dominates(s, b) {
                let li = *by_header.entry(s).or_insert_with(|| {
                    loops.push(Loop { header: s, blocks: Vec::new(), ids: HashSet::new(), latches: Vec::new(), parent: None, depth: 1 });
                    loops.len() - 1
                });
                loops[li].latches.push(b);
            }
        }
    }
    for lp in loops.iter_mut() {
        let h = lp.header;
        let mut ids: HashSet<usize> = HashSet::new();
        ids.insert(h);
        let mut body = vec![h];
        let mut stack: Vec<usize> = Vec::new();
        for &l in &lp.latches {
            if ids.insert(l) {
                body.push(l);
                stack.push(l);
            }
        }
        while let Some(x) = stack.pop() {
            for &p in &f.arena[x].preds {
                if ids.insert(p) {
                    body.push(p);
                    stack.push(p);
                }
            }
        }
        body.sort_by_key(|&bb| f.arena[bb].rpo);
        lp.blocks = body;
        lp.ids = ids;
    }
    // nesting: largest loops first so that the innermost wins
    loops.sort_by_key(|lp| (std::cmp::Reverse(lp.blocks.len()), f.arena[lp.header].rpo));
    for li in 0..loops.len() {
        let blocks = loops[li].blocks.clone();
        let h = loops[li].header;
        for b in blocks {
            if let Some(cur) = f.arena[b].lp {
                if loops[li].parent.is_none() && b == h {
                    loops[li].parent = Some(cur);
                }
            }
            f.arena[b].lp = Some(li);
        }
    }
    for li in 0..loops.len() {
        let mut d = 1;
        let mut p = loops[li].parent;
        while let Some(pi) = p {
            d += 1;
            p = loops[pi].parent;
        }
        loops[li].depth = d;
    }
    for k in 0..f.rpo.len() {
        let b = f.rpo[k];
        f.arena[b].depth = match f.arena[b].lp {
            Some(l) => loops[l].depth,
            None => 0,
        };
    }
    f.loops = loops;
}
