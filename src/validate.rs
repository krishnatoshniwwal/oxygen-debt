//! validate.rs - the stronger self-check that runs after the quick one in `evalsim`.
//!
//! The input program and a candidate are compared on inputs that are chosen to be hard:
//!
//! 1. **Literals.** Every literal of the program, plus and minus one, is tried as an argument
//!    (the judges' inputs hit `x == 1000` style guards; random numbers never do).
//! 2. **Exhaustion of small domains.** If every parameter is only ever read through
//!    `and p, CONST` (the samples mask their arguments, e.g. `n & 63`), the program can only
//!    see `p & U` where U is the union of the masks, so all 2^popcount(U) values of each
//!    parameter are run (up to 65536 combinations). If the program also reads no memory and
//!    no `vload`, that is a complete proof of equality (up to step-limit timeouts).
//!    Otherwise it is only a much denser test (a few memory contents per combination).
//! 3. **Inverse execution of targets.** The input is run with a trace that records, for every
//!    instruction, the values read and where they were defined. For every `div`/`mod`
//!    (divisor), `load`/`store` (address) and `br` (condition) we ask: which input value would
//!    make the divisor 0, the address 4096, the branch go the other way? The answer is found
//!    by walking the definition chain backwards through invertible operations (`add`, `sub`,
//!    `xor`, `mul` by any constant, shifts by constants, `and`/`or` with a mask, `lt`, `eq`,
//!    `select`, `mov`, loads of never-written words), down to an argument or a memory word.
//!    The changed input is added and run, and the new traces are inverted again (two rounds).
//!    This is a small concolic search with no solver; it finds inputs that reach traps hidden
//!    behind guards like `d == 1234567`.
//!
//! Everything has a step budget, so the time is bounded and does not depend on the machine.
use crate::kir::*;
use std::collections::HashSet;

const TOTAL_STEPS: u64 = 6_000_000;
const RUN_CAP: u64 = 100_000;

struct Budget {
    left: u64,
}

fn same_on(co: &Compiled, cn: &Compiled, inp: &Inputs, b: &mut Budget) -> bool {
    if b.left == 0 {
        return true;
    }
    let ro = execute(co, inp, RUN_CAP.min(b.left), false);
    b.left = b.left.saturating_sub(ro.steps);
    if ro.outcome == Outcome::Timeout {
        return true;
    }
    let rn = execute(cn, inp, 4 * ro.steps + 1000, false);
    b.left = b.left.saturating_sub(rn.steps);
    !(rn.outcome == Outcome::Timeout || differs(&ro, &rn))
}

// ---------------------------------------------------------------------------
// 2. Argument domains
// ---------------------------------------------------------------------------
pub struct Domain {
    /// per parameter: the union of the masks it is read through (0 = never read unmasked)
    pub masks: Vec<u32>,
    /// the program reads no memory and no vload: exhaustion is a proof
    pub pure: bool,
}

/// Domain analysis. None if some parameter value can be read other than through `and p, CONST`.
pub fn arg_domains(prog: &Program) -> Option<Domain> {
    let n = prog.instrs.len();
    if n > 3000 || prog.params.len() > 4 {
        return None;
    }
    // registers by dense index
    let mut regs: Vec<i64> = Vec::new();
    let idx = |r: i64, regs: &mut Vec<i64>| -> usize {
        if let Some(p) = regs.iter().position(|&x| x == r) {
            p
        } else {
            regs.push(r);
            regs.len() - 1
        }
    };
    for &p in &prog.params {
        idx(p, &mut regs);
    }
    for ins in &prog.instrs {
        for a in &ins.args {
            if let Arg::R(r) = a {
                idx(*r, &mut regs);
            }
        }
    }
    if regs.len() > 256 {
        return None;
    }
    let nr = regs.len();
    let ri = |r: i64| regs.iter().position(|&x| x == r).unwrap();
    // single-definition constant registers
    let mut defs = vec![0usize; nr];
    let mut cval: Vec<Option<u32>> = vec![None; nr];
    for ins in &prog.instrs {
        if ins.op.sig().starts_with('d') {
            if let Arg::R(d) = ins.args[0] {
                let d = ri(d);
                defs[d] += 1;
                if ins.op == Op::Const {
                    if let Arg::I(v) = ins.args[1] {
                        cval[d] = Some(v as u32);
                    }
                }
            }
        }
    }
    for &p in &prog.params {
        defs[ri(p)] += 1; // a parameter counts as a definition
    }
    let constval = |a: &Arg| -> Option<u32> {
        match a {
            Arg::I(v) => Some(*v as u32),
            Arg::R(r) => {
                let r = ri(*r);
                if defs[r] == 1 {
                    cval[r]
                } else {
                    None
                }
            }
            _ => None,
        }
    };
    // blocks
    let nb = prog.block_labels.len();
    let mut bstart = vec![0usize; nb + 1];
    for i in 0..n {
        bstart[prog.block_of[i] + 1] = i + 1;
    }
    for b in 1..=nb {
        if bstart[b] < bstart[b - 1] {
            bstart[b] = bstart[b - 1];
        }
    }
    let mut succ: Vec<Vec<usize>> = vec![Vec::new(); nb];
    for b in 0..nb {
        let last = &prog.instrs[bstart[b + 1] - 1];
        for a in &last.args {
            if let Arg::L(l) = a {
                succ[b].push(prog.block_of[prog.labels[l]]);
            }
        }
    }
    // forward "may still hold its initial parameter value" sets
    let mut init: Vec<Vec<bool>> = vec![vec![false; nr]; nb];
    for &p in &prog.params {
        init[0][ri(p)] = true;
    }
    let mut masks = vec![0u32; nr];
    let step = |cur: &mut Vec<bool>, ins: &PInstr, masks: &mut Vec<u32>, check: bool| -> bool {
        // reads
        let sig = ins.op.sig().as_bytes();
        let mut reads: Vec<(usize, &Arg)> = Vec::new();
        for (k, a) in ins.args.iter().enumerate() {
            if sig[k] != b'd' && sig[k] != b'L' {
                reads.push((k, a));
            }
        }
        if check {
            for (_, a) in &reads {
                if let Arg::R(r) = a {
                    let r = ri(*r);
                    if cur[r] {
                        // allowed only: and r, CONST
                        if ins.op == Op::And {
                            let other = if let Arg::R(x) = &ins.args[1] { if ri(*x) == r { &ins.args[2] } else { &ins.args[1] } } else { &ins.args[1] };
                            // both operands could be the same register
                            if let Some(c) = constval(other) {
                                masks[r] |= c;
                                continue;
                            }
                        }
                        return false;
                    }
                }
            }
        }
        if ins.op.sig().starts_with('d') {
            if let Arg::R(d) = ins.args[0] {
                cur[ri(d)] = false;
            }
        }
        true
    };
    let mut changed = true;
    let mut rounds = 0;
    while changed && rounds < 200 {
        changed = false;
        rounds += 1;
        for b in 0..nb {
            let mut cur = init[b].clone();
            for i in bstart[b]..bstart[b + 1] {
                step(&mut cur, &prog.instrs[i], &mut masks, false);
            }
            for &s in &succ[b] {
                for r in 0..nr {
                    if cur[r] && !init[s][r] {
                        init[s][r] = true;
                        changed = true;
                    }
                }
            }
        }
    }
    for b in 0..nb {
        let mut cur = init[b].clone();
        for i in bstart[b]..bstart[b + 1] {
            if !step(&mut cur, &prog.instrs[i], &mut masks, true) {
                return None;
            }
        }
    }
    let pm: Vec<u32> = prog.params.iter().map(|&p| masks[ri(p)]).collect();
    let pure = !prog.instrs.iter().any(|i| matches!(i.op, Op::Load | Op::Vload));
    Some(Domain { masks: pm, pure })
}

/// All subsets of a mask, in increasing order.
fn subsets(mask: u32) -> Vec<u32> {
    let mut v = vec![0u32];
    let mut sub = mask;
    // enumerate by the carry trick
    let mut all = Vec::new();
    loop {
        all.push(sub);
        if sub == 0 {
            break;
        }
        sub = (sub - 1) & mask;
    }
    all.reverse();
    v.clear();
    v.extend(all);
    v
}

/// Returns Some(true) = proof by exhaustion (and no difference), Some(false) = a difference
/// was found, None = not applicable (the domain is too big or parameters are read unmasked).
fn exhaust(orig: &Program, co: &Compiled, cn: &Compiled, b: &mut Budget) -> Option<bool> {
    let dom = arg_domains(orig)?;
    let mut total: u64 = 1;
    for &m in &dom.masks {
        total = total.saturating_mul(1u64 << m.count_ones().min(40));
        if total > 65536 {
            return None;
        }
    }
    let lists: Vec<Vec<u32>> = dom.masks.iter().map(|&m| subsets(m)).collect();
    let mems: Vec<Inputs> = if dom.pure {
        vec![Inputs::default()]
    } else if total <= 1024 {
        generate_inputs(0, 3, 99)
    } else {
        generate_inputs(0, 1, 99)
    };
    let np = lists.len();
    let mut counter = vec![0usize; np];
    let mut complete = true;
    loop {
        let args: Vec<u32> = (0..np).map(|i| lists[i][counter[i]]).collect();
        for m in &mems {
            let mut inp = m.clone();
            inp.args = args.clone();
            if b.left == 0 {
                complete = false;
            } else if !same_on(co, cn, &inp, b) {
                return Some(false);
            }
        }
        // next combination
        let mut i = 0;
        loop {
            if i == np {
                return if dom.pure && complete { Some(true) } else { None };
            }
            counter[i] += 1;
            if counter[i] < lists[i].len() {
                break;
            }
            counter[i] = 0;
            i += 1;
        }
    }
}

// ---------------------------------------------------------------------------
// 3. Inverse execution
// ---------------------------------------------------------------------------
#[derive(Clone, Copy)]
struct Step {
    pc: u32,
    /// values of the three source slots
    v: [u32; 3],
    /// defining step of each source: >= 0 a step, -1 initial register (parameter if slot < nparams),
    /// -2 literal / constant
    d: [i32; 3],
    taint: bool,
    addr: u32,
}

fn traced(cp: &Compiled, inp: &Inputs, maxsteps: u64) -> (Vec<Step>, Outcome) {
    let mut r = cp.r0.clone();
    for (i, v) in inp.args.iter().enumerate() {
        r[i] = *v;
    }
    let mut mem = vec![0u32; MEM_WORDS];
    for (addr, words) in &inp.segments {
        mem[*addr..*addr + words.len()].copy_from_slice(words);
    }
    let mut written = vec![false; MEM_WORDS];
    let mut lastdef = vec![-1i32; cp.nregs];
    let mut taint_reg: Vec<bool> = (0..cp.nregs).map(|i| i < cp.nparams).collect();
    let mut steps: Vec<Step> = Vec::new();
    let mut queues: std::collections::HashMap<u32, std::collections::VecDeque<u32>> = Default::default();
    for (p, w) in &inp.vin {
        queues.insert(*p, w.iter().copied().collect());
    }
    let nregs = cp.nregs;
    let mut pc = 0usize;
    let ts = |s: usize, lastdef: &Vec<i32>, taint_reg: &Vec<bool>| -> (i32, bool) {
        if s < nregs {
            (lastdef[s], taint_reg[s])
        } else {
            (-2, false)
        }
    };
    loop {
        if steps.len() as u64 >= maxsteps {
            return (steps, Outcome::Timeout);
        }
        let c = cp.code[pc];
        let op = c.op;
        // source slots by op
        let srcs: [Option<usize>; 3] = match op {
            Op::Mov | Op::Load | Op::Vload | Op::Trap | Op::Ret | Op::Br => [Some(c.x), None, None],
            Op::Store | Op::Vstore => [Some(c.x), Some(c.y), None],
            Op::Jmp => [None, None, None],
            Op::Select | Op::Mac => [Some(c.x), Some(c.y), Some(c.z)],
            Op::Addsh => [Some(c.x), Some(c.y), None],
            _ => [Some(c.x), Some(c.y), None],
        };
        let mut st = Step { pc: pc as u32, v: [0; 3], d: [-2; 3], taint: false, addr: 0 };
        for k in 0..3 {
            if let Some(s) = srcs[k] {
                st.v[k] = r[s];
                let (d, t) = ts(s, &lastdef, &taint_reg);
                st.d[k] = d;
                st.taint |= t;
            }
        }
        let idx = steps.len() as i32;
        let mut dest: Option<usize> = None;
        let mut next = pc + 1;
        let mut outcome: Option<Outcome> = None;
        match op {
            Op::Add => { r[c.d] = r[c.x].wrapping_add(r[c.y]); dest = Some(c.d); }
            Op::Mov => { r[c.d] = r[c.x]; dest = Some(c.d); }
            Op::Lt => { r[c.d] = ((r[c.x] as i32) < (r[c.y] as i32)) as u32; dest = Some(c.d); }
            Op::Sub => { r[c.d] = r[c.x].wrapping_sub(r[c.y]); dest = Some(c.d); }
            Op::Mul => { r[c.d] = r[c.x].wrapping_mul(r[c.y]); dest = Some(c.d); }
            Op::And => { r[c.d] = r[c.x] & r[c.y]; dest = Some(c.d); }
            Op::Or => { r[c.d] = r[c.x] | r[c.y]; dest = Some(c.d); }
            Op::Xor => { r[c.d] = r[c.x] ^ r[c.y]; dest = Some(c.d); }
            Op::Shl => { r[c.d] = r[c.x] << (r[c.y] & 31); dest = Some(c.d); }
            Op::Shr => { r[c.d] = r[c.x] >> (r[c.y] & 31); dest = Some(c.d); }
            Op::Eq => { r[c.d] = (r[c.x] == r[c.y]) as u32; dest = Some(c.d); }
            Op::Select => { r[c.d] = if r[c.x] != 0 { r[c.y] } else { r[c.z] }; dest = Some(c.d); }
            Op::Mac => { r[c.d] = r[c.x].wrapping_mul(r[c.y]).wrapping_add(r[c.z]); dest = Some(c.d); }
            Op::Addsh => { r[c.d] = r[c.x].wrapping_add(r[c.y] << c.z); dest = Some(c.d); }
            Op::Br => { next = if r[c.x] != 0 { c.t1 } else { c.t2 }; }
            Op::Jmp => { next = c.t1; }
            Op::Load => {
                let a = r[c.x] as usize;
                st.addr = a as u32;
                if a >= MEM_WORDS {
                    outcome = Some(Outcome::BadAddr);
                } else {
                    r[c.d] = mem[a];
                    st.taint = !written[a];
                    dest = Some(c.d);
                }
            }
            Op::Store => {
                let a = r[c.x] as usize;
                st.addr = a as u32;
                if a >= MEM_WORDS {
                    outcome = Some(Outcome::BadAddr);
                } else {
                    mem[a] = r[c.y];
                    written[a] = true;
                }
            }
            Op::Div | Op::Mod => {
                let y = r[c.y];
                if y == 0 {
                    outcome = Some(Outcome::DivZero);
                } else {
                    let sx = r[c.x] as i32 as i64;
                    let sy = y as i32 as i64;
                    r[c.d] = if op == Op::Div {
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
                    };
                    dest = Some(c.d);
                }
            }
            Op::Vload => {
                let port = r[c.x];
                r[c.d] = queues.get_mut(&port).and_then(|q| q.pop_front()).unwrap_or(0);
                st.taint = false;
                dest = Some(c.d);
            }
            Op::Vstore => {}
            Op::Trap => outcome = Some(Outcome::Explicit(r[c.x])),
            _ => outcome = Some(Outcome::Ret(r[c.x])),
        }
        steps.push(st);
        if let Some(d) = dest {
            lastdef[d] = idx;
            taint_reg[d] = st.taint;
        }
        if let Some(o) = outcome {
            return (steps, o);
        }
        pc = next;
    }
}

fn mod_inv(a: u32) -> u32 {
    // a odd: Newton iteration
    let mut x = a;
    for _ in 0..5 {
        x = x.wrapping_mul(2u32.wrapping_sub(a.wrapping_mul(x)));
    }
    x
}

enum Change {
    Arg(usize, u32),
    Mem(usize, u32),
}

/// Which input change makes step `s` produce the value `want`? Walks the definition chain
/// backwards through invertible operations.
fn invert(cp: &Compiled, steps: &[Step], mut s: usize, mut want: u32) -> Option<Change> {
    for _ in 0..64 {
        let st = &steps[s];
        let op = cp.code[st.pc as usize].op;
        let c = cp.code[st.pc as usize];
        // which source to follow, and what it must become
        let tainted = |k: usize| -> bool {
            let d = st.d[k];
            if d >= 0 {
                steps[d as usize].taint
            } else if d == -1 {
                // initial register: parameter?
                let slot = [c.x, c.y, c.z][k];
                slot < cp.nparams
            } else {
                false
            }
        };
        let (k, nw): (usize, u32) = match op {
            Op::Mov => (0, want),
            Op::Add => {
                if tainted(0) { (0, want.wrapping_sub(st.v[1])) } else if tainted(1) { (1, want.wrapping_sub(st.v[0])) } else { return None }
            }
            Op::Sub => {
                if tainted(0) { (0, want.wrapping_add(st.v[1])) } else if tainted(1) { (1, st.v[0].wrapping_sub(want)) } else { return None }
            }
            Op::Xor => {
                if tainted(0) { (0, want ^ st.v[1]) } else if tainted(1) { (1, want ^ st.v[0]) } else { return None }
            }
            Op::Mul => {
                let (k, o) = if tainted(0) { (0, st.v[1]) } else if tainted(1) { (1, st.v[0]) } else { return None };
                if o == 0 {
                    return None;
                }
                let tz = o.trailing_zeros();
                if tz > 0 && want & ((1u32 << tz) - 1) != 0 {
                    return None;
                }
                let odd = o >> tz;
                (k, (want >> tz).wrapping_mul(mod_inv(odd)))
            }
            Op::Shl => {
                if !tainted(0) {
                    return None;
                }
                let sh = st.v[1] & 31;
                if sh > 0 && want & ((1u32 << sh) - 1) != 0 {
                    return None;
                }
                (0, (want >> sh) | (st.v[0] & !(u32::MAX >> sh)))
            }
            Op::Shr => {
                if !tainted(0) {
                    return None;
                }
                let sh = st.v[1] & 31;
                if (want << sh) >> sh != want {
                    return None;
                }
                (0, (want << sh) | (st.v[0] & ((1u32 << sh).wrapping_sub(1))))
            }
            Op::And => {
                let (k, o, old) = if tainted(0) { (0, st.v[1], st.v[0]) } else if tainted(1) { (1, st.v[0], st.v[1]) } else { return None };
                if want & !o != 0 {
                    return None;
                }
                (k, want | (old & !o))
            }
            Op::Or => {
                let (k, o, old) = if tainted(0) { (0, st.v[1], st.v[0]) } else if tainted(1) { (1, st.v[0], st.v[1]) } else { return None };
                if o & !want != 0 {
                    return None;
                }
                let _ = old;
                (k, want & !o)
            }
            Op::Lt => {
                if tainted(0) {
                    (0, if want != 0 { st.v[1].wrapping_sub(1) } else { st.v[1] })
                } else if tainted(1) {
                    (1, if want != 0 { st.v[0].wrapping_add(1) } else { st.v[0] })
                } else {
                    return None;
                }
            }
            Op::Eq => {
                let (k, o) = if tainted(0) { (0, st.v[1]) } else if tainted(1) { (1, st.v[0]) } else { return None };
                (k, if want != 0 { o } else { o ^ 1 })
            }
            Op::Select => {
                let k = if st.v[0] != 0 { 1 } else { 2 };
                if !tainted(k) {
                    return None;
                }
                (k, want)
            }
            Op::Mac => {
                if !tainted(2) {
                    return None;
                }
                (2, want.wrapping_sub(st.v[0].wrapping_mul(st.v[1])))
            }
            Op::Addsh => {
                if !tainted(0) {
                    return None;
                }
                (0, want.wrapping_sub(st.v[1] << c.z))
            }
            Op::Load => {
                if !st.taint || st.addr as usize >= MEM_WORDS {
                    return None;
                }
                return Some(Change::Mem(st.addr as usize, want));
            }
            _ => return None,
        };
        want = nw;
        match st.d[k] {
            d if d >= 0 => s = d as usize,
            -1 => {
                let slot = [c.x, c.y, c.z][k];
                return if slot < cp.nparams { Some(Change::Arg(slot, want)) } else { None };
            }
            _ => return None,
        }
    }
    None
}

fn apply(inp: &Inputs, ch: &Change) -> Inputs {
    let mut n = inp.clone();
    match ch {
        Change::Arg(i, v) => {
            if *i < n.args.len() {
                n.args[*i] = *v;
            }
        }
        Change::Mem(a, v) => n.segments.push((*a, vec![*v])),
    }
    n
}

/// Inputs found by inverse execution from `base`.
fn explore(cp: &Compiled, base: &[Inputs], seen: &mut HashSet<u64>, limit: usize) -> Vec<Inputs> {
    let mut out: Vec<Inputs> = Vec::new();
    for inp in base {
        let (steps, _) = traced(cp, inp, 20_000);
        // newest occurrence of each pc is the most informative; a few per pc
        let mut cnt: std::collections::HashMap<u32, u32> = Default::default();
        for s in (0..steps.len()).rev() {
            let st = &steps[s];
            let c = cp.code[st.pc as usize];
            let n = cnt.entry(st.pc).or_insert(0);
            if *n >= 2 {
                continue;
            }
            let target: Option<(usize, u32)> = match c.op {
                Op::Div | Op::Mod => (st.d[1] >= 0 || st.d[1] == -1).then(|| (1usize, 0u32)),
                Op::Load | Op::Store => Some((0, 4096)),
                Op::Br => Some((0, if st.v[0] != 0 { 0 } else { 1 })),
                _ => None,
            };
            if let Some((k, w)) = target {
                *n += 1;
                let ch = if st.d[k] >= 0 {
                    invert(cp, &steps, st.d[k] as usize, w)
                } else if st.d[k] == -1 {
                    let slot = [c.x, c.y, c.z][k];
                    if slot < cp.nparams { Some(Change::Arg(slot, w)) } else { None }
                } else {
                    None
                };
                if let Some(ch) = ch {
                    let ni = apply(inp, &ch);
                    let h = hash_input(&ni);
                    if seen.insert(h) {
                        out.push(ni);
                        if out.len() >= limit {
                            return out;
                        }
                    }
                }
            }
        }
    }
    out
}

fn hash_input(i: &Inputs) -> u64 {
    let mut h: u64 = 1469598103934665603;
    let mut mix = |v: u64| {
        h ^= v;
        h = h.wrapping_mul(1099511628211);
    };
    for a in &i.args {
        mix(*a as u64);
    }
    for (a, w) in &i.segments {
        mix(*a as u64 + 7);
        for x in w.iter().take(300) {
            mix(*x as u64);
        }
    }
    for (p, w) in &i.vin {
        mix(*p as u64 + 99);
        for x in w {
            mix(*x as u64);
        }
    }
    h
}

fn literal_inputs(prog: &Program, base: &[Inputs]) -> Vec<Inputs> {
    let np = prog.params.len();
    let mut lits: Vec<u32> = Vec::new();
    for ins in &prog.instrs {
        for a in &ins.args {
            if let Arg::I(v) = a {
                let v = *v as u32;
                for d in [0u32, 1, u32::MAX] {
                    let x = v.wrapping_add(d);
                    if !lits.contains(&x) {
                        lits.push(x);
                    }
                }
            }
        }
    }
    lits.sort();
    let mut out = Vec::new();
    if np == 0 {
        return out;
    }
    let mut rng = Rng::new(0x51F0);
    for (j, &l) in lits.iter().take(160).enumerate() {
        let mut i = base[(j + rng.below(4) as usize) % base.len()].clone();
        i.args[j % np] = l;
        out.push(i);
    }
    out
}

/// True if no difference between `orig` and the candidate text was found.
pub fn deep_check(orig: &Program, cand_text: &str, _mc: &Machine) -> bool {
    let cand = match parse_program(cand_text) {
        Ok(p) => p,
        Err(_) => return false,
    };
    let (co, cn) = (compile_program(orig), compile_program(&cand));
    let mut b = Budget { left: TOTAL_STEPS };
    let np = orig.params.len();
    // 2. exhaustion
    match exhaust(orig, &co, &cn, &mut b) {
        Some(true) => return true, // proof: complete, memory-free
        Some(false) => return false,
        None => {}
    }
    let base = generate_inputs(np, 40, 0x5EED);
    // 1. literals
    for inp in literal_inputs(orig, &base) {
        if !same_on(&co, &cn, &inp, &mut b) {
            return false;
        }
    }
    // 3. inverse execution, two rounds
    let mut seen: HashSet<u64> = HashSet::new();
    let mut frontier: Vec<Inputs> = base.iter().step_by(3).cloned().collect();
    for _ in 0..2 {
        let found = explore(&co, &frontier, &mut seen, 160);
        for inp in &found {
            if !same_on(&co, &cn, inp, &mut b) {
                return false;
            }
        }
        frontier = found;
        if frontier.is_empty() || b.left == 0 {
            break;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mc() -> Machine {
        Machine::default()
    }

    #[test]
    fn finds_hidden_div_trap() {
        // original traps when x == 1234567 (guard hides the division by zero); the bad
        // candidate drops the guard's effect
        let orig = "func f(v0)\nentry:\n  const v1, 1234567\n  eq v2, v0, v1\n  br v2, bad, ok\nbad:\n  const v3, 0\n  div v4, v0, v3\n  ret v4\nok:\n  ret v0\n";
        let cand = "func f(v0)\nentry:\n  ret v0\n";
        let p = parse_program(orig).unwrap();
        assert!(!deep_check(&p, cand, &mc()));
        assert!(deep_check(&p, orig, &mc()));
    }

    #[test]
    fn exhausts_masked_domain() {
        let orig = "func f(v0)\nentry:\n  and v1, v0, 63\n  mul v2, v1, 3\n  ret v2\n";
        // wrong only for n & 63 == 37
        let cand = "func f(v0)\nentry:\n  and v1, v0, 63\n  mul v2, v1, 3\n  eq v3, v1, 37\n  add v4, v2, v3\n  ret v4\n";
        let p = parse_program(orig).unwrap();
        let d = arg_domains(&p).unwrap();
        assert_eq!(d.masks, vec![63]);
        assert!(d.pure);
        assert!(!deep_check(&p, cand, &mc()));
        assert!(deep_check(&p, orig, &mc()));
    }

    #[test]
    fn unmasked_use_is_not_a_domain() {
        let p = parse_program("func f(v0)\nentry:\n  and v1, v0, 63\n  add v2, v0, v1\n  ret v2\n").unwrap();
        assert!(arg_domains(&p).is_none());
    }

    #[test]
    fn rewritten_parameter_is_fine() {
        let p = parse_program("func f(v0)\nentry:\n  and v1, v0, 15\n  mov v0, v1\n  add v2, v0, v0\n  ret v2\n").unwrap();
        assert_eq!(arg_domains(&p).unwrap().masks, vec![15]);
    }
}
