//! finite.rs - exhaustive specialization of functions with a proven finite input domain.
//!
//! Precondition (checked structurally on the ORIGINAL program):
//!   * no memory, volatile or explicit-trap instructions, and the entry block is not a branch target;
//!   * every parameter is either never read, or every read of it before its first redefinition
//!     in the entry block is `and _, p, LIT` (either operand order), and if it is never redefined
//!     in the entry block it is not read outside the entry block.
//! Then the whole execution depends only on the bits `p & mask_p` (all other registers start at
//! zero), so running the original on all 2^k representatives (k = total mask bits <= 12) gives
//! the complete function. Every run must return normally within the step limits.
//!
//! The candidate is a decision tree over the input bits (identical subtrees shared), whose leaves
//! either return a literal or extract a value from a 32-bit packed literal using the low key bits.
//! It is checked on every representative and accepted only if its summed cycle count over the
//! whole domain is strictly lower than that of the current best output.
use crate::kir::*;
use std::collections::HashMap;
use std::fmt::Write;

const MAX_BITS: u32 = 12;
const RUN_STEPS: u64 = 20_000;
const TOTAL_STEPS: u64 = 4_000_000;
const MAX_INSTRS: usize = 4096;

fn reads(ins: &PInstr) -> Vec<i64> {
    let sig = ins.op.sig().as_bytes();
    let mut v = Vec::new();
    for (j, a) in ins.args.iter().enumerate() {
        if let Arg::R(r) = a {
            if sig.get(j) != Some(&b'd') {
                v.push(*r);
            }
        }
    }
    v
}

fn dest(ins: &PInstr) -> Option<i64> {
    match (ins.op.sig().as_bytes().first(), ins.args.first()) {
        (Some(b'd'), Some(Arg::R(r))) => Some(*r),
        _ => None,
    }
}

/// Mask of relevant bits per parameter, or None if the precondition fails.
fn domain(prog: &Program) -> Option<Vec<u32>> {
    if prog.params.is_empty() || prog.instrs.is_empty() {
        return None;
    }
    for ins in &prog.instrs {
        if ins.op.is_mem() || ins.op == Op::Trap || ins.op == Op::Phi {
            return None;
        }
        for a in &ins.args {
            if let Arg::L(l) = a {
                if prog.labels.get(l) == Some(&0) {
                    return None; // entry is re-entered
                }
            }
        }
    }
    let entry_end = prog.block_of.iter().position(|&b| b != prog.block_of[0]).unwrap_or(prog.instrs.len());
    let mut masks = Vec::new();
    for &p in &prog.params {
        let mut mask = 0u32;
        let mut redefined = false;
        let mut known: HashMap<i64, u32> = HashMap::new(); // entry-block constants
        for ins in &prog.instrs[..entry_end] {
            if reads(ins).contains(&p) {
                let val = |a: &Arg| match a {
                    Arg::I(m) => Some(*m as u32),
                    Arg::R(r) if *r != p => known.get(r).copied(),
                    _ => None,
                };
                let lit = match (ins.op, &ins.args[..]) {
                    (Op::And, [_, Arg::R(x), m]) | (Op::And, [_, m, Arg::R(x)]) if *x == p && val(m).is_some() => val(m).unwrap(),
                    _ => return None,
                };
                mask |= lit;
            }
            if let Some(d) = dest(ins) {
                match (ins.op, &ins.args[..]) {
                    (Op::Const, [_, Arg::I(v)]) => { known.insert(d, *v as u32); }
                    _ => { known.remove(&d); }
                }
            }
            if dest(ins) == Some(p) {
                redefined = true;
                break;
            }
        }
        if !redefined && prog.instrs[entry_end..].iter().any(|i| reads(i).contains(&p)) {
            return None;
        }
        masks.push(mask);
    }
    if masks.iter().map(|m| m.count_ones()).sum::<u32>() > MAX_BITS {
        return None;
    }
    Some(masks)
}

/// key bit i -> (param index, bit position); low key bits = low bits of the first params.
fn key_bits(masks: &[u32]) -> Vec<(usize, u32)> {
    let mut v = Vec::new();
    for (pi, m) in masks.iter().enumerate() {
        for b in 0..32 {
            if m >> b & 1 == 1 {
                v.push((pi, b));
            }
        }
    }
    v
}

fn inputs_for(key: u32, bits: &[(usize, u32)], nparams: usize) -> Inputs {
    let mut args = vec![0u32; nparams];
    for (i, &(pi, b)) in bits.iter().enumerate() {
        if key >> i & 1 == 1 {
            args[pi] |= 1 << b;
        }
    }
    Inputs { args, ..Inputs::default() }
}

struct Gen<'a> {
    bits: &'a [(usize, u32)],
    pregs: Vec<String>,
    s: u32,     // packed low key bits per leaf
    lw: u32,    // log2 of the field width
    shreg: String,
    tmp: String,
    blocks: Vec<String>,
    memo: HashMap<Vec<u32>, String>,
    ninstr: usize,
}

impl<'a> Gen<'a> {
    /// Label of a block computing the function on `vals` (indexed by the low `level` key bits).
    fn node(&mut self, vals: &[u32], level: u32) -> Option<String> {
        if let Some(l) = self.memo.get(vals) {
            return Some(l.clone());
        }
        if self.ninstr > MAX_INSTRS {
            return None;
        }
        let mut label = String::from("@L@"); // named when pushed, after the children
        let mut body = String::new();
        if vals.iter().all(|&v| v == vals[0]) {
            let _ = writeln!(body, "{}:\n  ret {}", label, vals[0]);
            self.ninstr += 1;
        } else if level == self.s {
            let w = 1u32 << self.lw;
            let lo = *vals.iter().min().unwrap();
            let fm = if w == 32 { u32::MAX } else { (1u32 << w) - 1 };
            if vals.iter().any(|&v| v - lo > fm) {
                return None;
            }
            let mut packed = 0u32;
            for (i, &v) in vals.iter().enumerate() {
                packed |= (v - lo) << (i as u32 * w);
            }
            let t = &self.tmp;
            let _ = writeln!(body, "{}:\n  shr {}, {}, {}", label, t, packed, self.shreg);
            self.ninstr += 2;
            if w < 32 {
                let _ = writeln!(body, "  and {}, {}, {}", t, t, fm);
                self.ninstr += 1;
            }
            if lo != 0 {
                let _ = writeln!(body, "  add {}, {}, {}", t, t, lo);
                self.ninstr += 1;
            }
            let _ = writeln!(body, "  ret {}", t);
        } else {
            let half = vals.len() / 2;
            let (pi, b) = self.bits[level as usize - 1];
            // vals is indexed by key bits 0..level; the top one (level-1) splits it in halves
            let zero = self.node(&vals[..half], level - 1)?;
            let one = self.node(&vals[half..], level - 1)?;
            if zero == one {
                return Some(zero);
            }
            let _ = writeln!(body, "{}:\n  and {}, {}, {}\n  br {}, {}, {}", label, self.tmp, self.pregs[pi], 1u32 << b, self.tmp, one, zero);
            self.ninstr += 2;
        }
        label = format!("f{}", self.blocks.len());
        self.blocks.push(body.replacen("@L@", &label, 1));
        self.memo.insert(vals.to_vec(), label.clone());
        Some(label)
    }
}

/// Emit a candidate, or None if it does not fit.
fn emit(prog: &Program, bits: &[(usize, u32)], table: &[u32], s: u32, lw: u32) -> Option<String> {
    // the low s key bits must be one contiguous run of a single parameter
    if s > 0 {
        let (p0, b0) = bits[0];
        if (0..s as usize).any(|i| bits.get(i) != Some(&(p0, b0 + i as u32))) {
            return None;
        }
    }
    let top = prog.instrs.iter().flat_map(|i| i.args.iter()).filter_map(|a| if let Arg::R(r) = a { Some(*r) } else { None })
        .chain(prog.params.iter().copied()).max().unwrap_or(0);
    let (shreg, tmp) = (format!("v{}", top + 1), format!("v{}", top + 2));
    let pregs: Vec<String> = prog.params.iter().map(|p| format!("v{}", p)).collect();
    let mut g = Gen { bits, pregs, s, lw, shreg, tmp, blocks: Vec::new(), memo: HashMap::new(), ninstr: 0 };
    let root = g.node(table, bits.len() as u32)?;
    let mut out = String::new();
    let _ = writeln!(out, "func {}({})", prog.name, g.pregs.join(", "));
    let _ = writeln!(out, "entry:");
    if s > 0 {
        let (p0, b0) = bits[0];
        let src = &g.pregs[p0];
        if b0 > 0 {
            let _ = writeln!(out, "  shr {}, {}, {}", g.shreg, src, b0);
            let _ = writeln!(out, "  and {}, {}, {}", g.shreg, g.shreg, (1u32 << s) - 1);
        } else {
            let _ = writeln!(out, "  and {}, {}, {}", g.shreg, src, (1u32 << s) - 1);
        }
        if lw > 0 {
            let _ = writeln!(out, "  shl {}, {}, {}", g.shreg, g.shreg, lw);
        }
    }
    // blocks are built children-first and the root last; emit in reverse so parents precede
    // children, with the root's body inlined into the entry block (nothing branches to it)
    for b in g.blocks.iter().rev() {
        if b.starts_with(&format!("{}:\n", root)) {
            out.push_str(&b[root.len() + 2..]);
        } else {
            out.push_str(b);
        }
    }
    Some(out)
}

fn domain_cost(cp: &Compiled, ins: &[Inputs], expect: &[u32], mc: &Machine) -> Option<i64> {
    let mut total = 0i64;
    for (inp, &want) in ins.iter().zip(expect) {
        let r = execute(cp, inp, RUN_STEPS * 4, true);
        if r.outcome != Outcome::Ret(want) || !r.vlog.is_empty() {
            return None;
        }
        total += time_trace(cp, &r.trace, mc);
    }
    Some(total)
}

/// Returns a strictly better (over the whole input domain) replacement for `best`, if any.
pub fn improve(original: &str, best: &str, mc: &Machine) -> Option<String> {
    let prog = parse_program(original).ok()?;
    let dbg = std::env::var_os("KOPT_DEBUG_FINITE").is_some();
    let masks = domain(&prog);
    if dbg { eprintln!("finite: masks {:?}", masks); }
    let masks = masks?;
    let bits = key_bits(&masks);
    let n = 1usize << bits.len();
    let cpo = compile_program(&prog);
    let ins: Vec<Inputs> = (0..n as u32).map(|k| inputs_for(k, &bits, prog.params.len())).collect();
    let mut table = Vec::with_capacity(n);
    let mut spent = 0u64;
    for inp in &ins {
        let r = execute(&cpo, inp, RUN_STEPS, false);
        spent += r.steps;
        match r.outcome {
            Outcome::Ret(v) if r.vlog.is_empty() && spent <= TOTAL_STEPS => table.push(v),
            o => { if dbg { eprintln!("finite: run failed {:?} spent {}", o, spent); } return None }
        }
    }
    let cpb = compile_program(&parse_program(best).ok()?);
    let mut best_cost = domain_cost(&cpb, &ins, &table, mc)?;
    if dbg { eprintln!("finite: best cost {}", best_cost); }
    let mut best_text = None;
    for s in 0..=5u32.min(bits.len() as u32) {
        for lw in 0..=5u32 {
            if (1u32 << lw) << s > 32 || (s == 0 && lw > 0) {
                continue;
            }
            let Some(text) = emit(&prog, &bits, &table, s, lw) else { continue };
            let p = match parse_program(&text) { Ok(p) => p, Err(e) => { if dbg { eprintln!("finite: parse {} s={} lw={}
{}", e, s, lw, &text[..text.len().min(400)]); } continue } };
            if check_machine_features(&p, mc).is_err() {
                continue;
            }
            let cp = compile_program(&p);
            let dc = domain_cost(&cp, &ins, &table, mc);
            if dbg { eprintln!("finite: s={} lw={} instrs={} cost {:?}", s, lw, p.instrs.len(), dc); }
            if let Some(c) = dc {
                if c < best_cost {
                    best_cost = c;
                    best_text = Some(text);
                }
            }
        }
    }
    best_text
}
