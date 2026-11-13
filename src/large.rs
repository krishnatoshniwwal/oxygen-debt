//! large.rs - everything that exists only because of very large programs (about 100,000
//! instructions): the safety deadline, the best-so-far candidate, structural guards, and (below)
//! hot/cold layout and partial loop rerolling.
//!
//! Safety rules for this file: no recursion over the IR, no clocks in any decision that changes
//! the output except the final deadline (which can only replace the result by an earlier,
//! already verified candidate or by the unchanged input).
use crate::evalsim::Reference;
use std::collections::HashMap;
use crate::kir::*;
use std::sync::mpsc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

static START: OnceLock<Instant> = OnceLock::new();
static BEST: Mutex<Option<(f64, String)>> = Mutex::new(None);

/// Call first thing in main: the deadline counts from process start.
pub fn start() {
    let _ = START.set(Instant::now());
}

pub fn elapsed_ms() -> u64 {
    START.get().map_or(0, |s| s.elapsed().as_millis() as u64)
}

/// Hard deadline in milliseconds for an input of this many instructions. Normal programs get
/// 5 s from the judges, the very large ones 30 s; both include start-up.
pub fn hard_limit_ms(ninstr: usize) -> u64 {
    // developer aid: KOPT_HARD_MS overrides the limit (to test the fallback)
    if let Some(v) = std::env::var("KOPT_HARD_MS").ok().and_then(|v| v.parse::<u64>().ok()) {
        return v;
    }
    if ninstr < 40_000 {
        3_800
    } else {
        25_000
    }
}

/// A verified candidate that is faster than the input: remember it for the deadline fallback.
/// `same` must already have been checked by the caller's reference (done here).
pub fn publish(rf: &Reference, cp: &Compiled, score: f64, base: Option<f64>, text: &str) {
    if let Some(b) = base {
        if score >= b - 1e-9 {
            return;
        }
    }
    {
        let g = BEST.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((s, _)) = &*g {
            if *s <= score {
                return;
            }
        }
    }
    if !rf.same(cp) {
        return;
    }
    let mut g = BEST.lock().unwrap_or_else(|e| e.into_inner());
    if g.as_ref().map_or(true, |(s, _)| score < *s) {
        *g = Some((score, text.to_string()));
    }
}

/// Run `work` on a worker thread with a large stack. If it has not finished at the hard
/// deadline, return the best published candidate, or the unchanged input.
pub fn guarded<F>(input: &str, work: F) -> Result<String, String>
where
    F: FnOnce() -> Result<String, String> + Send + 'static,
{
    let n = input.bytes().filter(|&c| c == b'\n').count();
    let hard = hard_limit_ms(n);
    let (tx, rx) = mpsc::channel::<Result<String, String>>();
    let spawned = std::thread::Builder::new().stack_size(256 << 20).spawn(move || {
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(work));
        let _ = tx.send(match r {
            Ok(x) => x,
            Err(_) => Err("internal error".to_string()),
        });
    });
    if spawned.is_err() {
        return Err("cannot start the worker thread".to_string());
    }
    let left = hard.saturating_sub(elapsed_ms());
    match rx.recv_timeout(Duration::from_millis(left.max(50))) {
        Ok(r) => r,
        Err(_) => {
            eprintln!("kopt: deadline reached");
            let g = BEST.lock().unwrap_or_else(|e| e.into_inner());
            match &*g {
                Some((_, t)) => Ok(t.clone()),
                None => Ok(input.to_string()),
            }
        }
    }
}

/// Programs where some pass of the pipeline is known to be quadratic (very high fan-in of a
/// join block): they get the input back instead of a run that cannot finish in time.
pub fn too_hostile(prog: &Program) -> bool {
    let mut cnt: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    let mut worst = 0usize;
    for pi in &prog.instrs {
        let targets: &[Arg] = &pi.args;
        for a in targets {
            if let Arg::L(l) = a {
                let c = cnt.entry(l.as_str()).or_insert(0);
                *c += 1;
                worst = worst.max(*c);
            }
        }
    }
    let lim = std::env::var("KOPT_HOSTILE").ok().and_then(|v| v.parse().ok()).unwrap_or(40000usize);
    worst > lim
}

// ---------------------------------------------------------------------------
// Rerolling mode (chosen by the driver: it runs the pipeline once per mode and keeps the
// fastest candidate by exact simulation)
// ---------------------------------------------------------------------------
static ROLL: AtomicU8 = AtomicU8::new(0);

pub fn set_roll(mode: u8) {
    ROLL.store(mode, Ordering::Relaxed);
}

pub fn roll_mode() -> u8 {
    ROLL.load(Ordering::Relaxed)
}

/// Reroll straight-line repetition in the SSA function. Mode 0 = off; 1 = loop bodies up to the
/// instruction cache size; 2 = half of it; 3 = four times it. Programs that fit in the cache are
/// left alone: rolling code that is cheap anyway only adds loop overhead.
pub fn reroll_pass(f: &mut crate::ir::Func, mc: &Machine) -> usize {
    let mode = roll_mode();
    if mode == 0 {
        return 0;
    }
    let c = mc.icache_size.max(8) as usize;
    if f.count_instrs() <= c + 8 {
        return 0;
    }
    let budget = match mode {
        1 => c,
        2 => c / 2 + 8,
        _ => 4 * c,
    };
    crate::reroll::reroll(f, budget)
}

/// Clean-up, if-conversion and rerolling, in this order, on the freshly built SSA function (before
/// the memory pass merges loads of equal addresses, which would hide the repetition). Returns true
/// when a loop was made.
pub fn pre_roll(f: &mut crate::ir::Func, mc: &Machine, ifc: usize) -> bool {
    use crate::{passes, passes3, ranges::Ranges};
    let c = mc.icache_size.max(8) as usize;
    if f.count_instrs() <= c + 8 {
        return false;
    }
    let scalar = |f: &mut crate::ir::Func| {
        for _ in 0..3 {
            let c1 = passes::scalar_pass(f);
            let c2 = passes::cleanup_cfg(f);
            let c3 = passes::dce(f, None);
            if !(c1 || c2 || c3) {
                break;
            }
        }
    };
    scalar(f);
    if ifc > 0 {
        let rng = Ranges::new(f, 6);
        if passes3::ifconv(f, &rng, ifc) {
            scalar(f);
        }
    }
    reroll_pass(f, mc) > 0
}

// ---------------------------------------------------------------------------
// Hot/cold layout from exact dynamic counts
// ---------------------------------------------------------------------------
static LAYOUT: AtomicU8 = AtomicU8::new(0);
static COUNT_INPUTS: Mutex<Vec<Inputs>> = Mutex::new(Vec::new());

/// 0 = the classic chaining layout; 1 = exact counts; 2 = exact counts and a tiny entry block.
pub fn set_layout(mode: u8) {
    LAYOUT.store(mode, Ordering::Relaxed);
}

/// Inputs on which the program is simulated to count how often each block and edge runs.
pub fn set_count_inputs(v: Vec<Inputs>) {
    *COUNT_INPUTS.lock().unwrap_or_else(|e| e.into_inner()) = v;
}

struct Counts {
    block: HashMap<usize, u64>,
    edge: HashMap<(usize, usize), u64>,
}

/// Run the function (in its current block order) and count executions per block and per edge.
fn exact_counts(f: &crate::ir::Func) -> Option<Counts> {
    let inputs = COUNT_INPUTS.lock().unwrap_or_else(|e| e.into_inner()).clone();
    if inputs.is_empty() {
        return None;
    }
    let text = crate::ir::to_text(f);
    let prog = parse_program(&text).ok()?;
    let cp = compile_program(&prog);
    let n = prog.instrs.len();
    let mut start = Vec::with_capacity(f.blocks.len());
    let mut owner: Vec<usize> = Vec::with_capacity(n);
    let mut last_pc: Vec<bool> = Vec::with_capacity(n);
    for &b in &f.blocks {
        let len = f.arena[b].ins.len();
        start.push(owner.len());
        for k in 0..len {
            owner.push(b);
            last_pc.push(k + 1 == len);
        }
    }
    if owner.len() != n {
        return None;
    }
    let mut block: HashMap<usize, u64> = HashMap::new();
    let mut edge: HashMap<(usize, usize), u64> = HashMap::new();
    let mut is_start = vec![false; n];
    for &s in &start {
        if s < n {
            is_start[s] = true;
        }
    }
    let cap = (6 * n as u64 + 1000).max(20_000);
    for inp in &inputs {
        let res = execute(&cp, inp, cap, true);
        let tr = &res.trace;
        for (i, &pc) in tr.iter().enumerate() {
            let pc = pc as usize;
            if is_start[pc] {
                *block.entry(owner[pc]).or_insert(0) += 1;
            }
            if last_pc[pc] && i + 1 < tr.len() {
                let nx = tr[i + 1] as usize;
                *edge.entry((owner[pc], owner[nx])).or_insert(0) += 1;
            }
        }
    }
    Some(Counts { block, edge })
}

/// Block order for the final program (see DESIGN.md, "Hot/cold layout").
///
/// Mode 0: the classic order of backend::layout. Modes 1 and 2: blocks are chained along their
/// *measured* hottest edges (a cold block is never chained to a hot one), and the chains are then
/// ordered by how often their instructions run, so the code that runs most sits in the first
/// `icache_size` slots and run-once or never-run code goes behind it. Mode 2 also puts a tiny
/// block `jmp OLD_ENTRY` at index 0 (the entry block must be first), which lets the old entry
/// code move behind the hot loops at the price of one taken jump.
pub fn layout(f: &mut crate::ir::Func, mc: &Machine, profile: Option<&crate::backend::Profile>) {
    use crate::ir::*;
    let _ = mc;
    crate::backend::layout(f, profile);
    let mode = LAYOUT.load(Ordering::Relaxed);
    if mode == 0 {
        return;
    }
    let counts = match exact_counts(f) {
        Some(c) => c,
        None => return,
    };
    let entry = f.blocks[0];
    let order_idx: HashMap<usize, usize> = f.blocks.iter().enumerate().map(|(i, &b)| (b, i)).collect();
    let cnt = |b: usize| -> u64 { counts.block.get(&b).copied().unwrap_or(0) };
    let nb = f.arena.len();
    // ---- chains along measured edges ----
    let mut edges: Vec<(u64, usize, usize, usize)> = Vec::new(); // (count, order, src, dst)
    let mut k = 0;
    for &b in &f.blocks {
        for &s in f.arena[b].succs() {
            let w = counts.edge.get(&(b, s)).copied().unwrap_or(0);
            edges.push((w, k, b, s));
            k += 1;
        }
    }
    edges.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    let mut chain_of: Vec<usize> = (0..nb).collect();
    let mut chains: Vec<Option<Vec<usize>>> = vec![None; nb];
    for &b in &f.blocks {
        chains[b] = Some(vec![b]);
    }
    for (_, _, s, d) in edges {
        if d == entry || s == d {
            continue;
        }
        if (cnt(s) == 0) != (cnt(d) == 0) {
            continue; // never join cold code to hot code
        }
        let (cs, cd) = (chain_of[s], chain_of[d]);
        if cs == cd {
            continue;
        }
        if *chains[cs].as_ref().unwrap().last().unwrap() != s || chains[cd].as_ref().unwrap()[0] != d {
            continue;
        }
        let moved = chains[cd].take().unwrap();
        for &x in &moved {
            chain_of[x] = cs;
        }
        chains[cs].as_mut().unwrap().extend(moved);
    }
    // ---- order the chains by instruction heat ----
    let ec = chain_of[entry];
    let heat = |c: usize| -> f64 {
        let ch = chains[c].as_ref().unwrap();
        let (mut run, mut size) = (0f64, 0f64);
        for &x in ch {
            let l = f.arena[x].ins.len() as f64;
            run += cnt(x) as f64 * l;
            size += l;
        }
        run / size.max(1.0)
    };
    let mut rest: Vec<usize> = (0..nb).filter(|&c| chains[c].is_some() && (mode == 2 || c != ec)).collect();
    rest.sort_by(|&a, &b| {
        heat(b)
            .partial_cmp(&heat(a))
            .unwrap()
            .then(order_idx[&chains[a].as_ref().unwrap()[0]].cmp(&order_idx[&chains[b].as_ref().unwrap()[0]]))
    });
    let mut out: Vec<usize> = Vec::new();
    if mode == 2 {
        let ne = f.new_block("entry");
        f.arena[ne].ins.push(Instr::no_d(Op::Jmp, vec![], vec![entry]));
        out.push(ne);
    } else {
        out.extend(chains[ec].as_ref().unwrap().iter().copied());
    }
    for c in rest {
        out.extend(chains[c].as_ref().unwrap().iter().copied());
    }
    f.blocks = out;
    compute_preds(f);
}

// ---------------------------------------------------------------------------
// The SSA function the pipeline starts from, cached per rerolling mode (the search runs the
// pipeline many times on the same input; building and rerolling a 100,000-instruction function
// is a large part of each run).
// ---------------------------------------------------------------------------
type StartKey = (u8, usize, i64);
static START_CACHE: Mutex<Vec<(StartKey, crate::ir::Func, bool)>> = Mutex::new(Vec::new());

/// The SSA function to start from, and whether rerolling changed it (always false in mode 0).
pub fn start_func_ex(prog: &Program, mc: &Machine, ifc: usize) -> (crate::ir::Func, bool) {
    let mode = roll_mode();
    let key: StartKey = (mode, if mode > 0 { ifc } else { 0 }, mc.icache_size);
    {
        let g = START_CACHE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((_, f, r)) = g.iter().find(|(k, _, _)| *k == key) {
            return (f.clone(), *r);
        }
    }
    let mut f = crate::ir::from_program(prog);
    crate::ssa::build_ssa(&mut f);
    let rolled = mode > 0 && pre_roll(&mut f, mc, ifc);
    let mut g = START_CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if g.len() < 12 && f.count_instrs() < 400_000 {
        g.push((key, f.clone(), rolled));
    }
    (f, rolled)
}

pub fn start_func(prog: &Program, mc: &Machine, ifc: usize) -> crate::ir::Func {
    start_func_ex(prog, mc, ifc).0
}
