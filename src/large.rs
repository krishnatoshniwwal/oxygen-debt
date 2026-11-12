//! large.rs - everything that exists only because of very large programs (about 100,000
//! instructions): the safety deadline, the best-so-far candidate, structural guards, and (below)
//! hot/cold layout and partial loop rerolling.
//!
//! Safety rules for this file: no recursion over the IR, no clocks in any decision that changes
//! the output except the final deadline (which can only replace the result by an earlier,
//! already verified candidate or by the unchanged input).
use crate::evalsim::Reference;
use crate::kir::*;
use std::sync::mpsc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

static START: OnceLock<Instant> = OnceLock::new();
static SOFT_LIMIT_MS: Mutex<u64> = Mutex::new(u64::MAX);
static BEST: Mutex<Option<(f64, String)>> = Mutex::new(None);

/// Call first thing in main: the deadline counts from process start.
pub fn start() {
    let _ = START.set(Instant::now());
}

pub fn elapsed_ms() -> u64 {
    START.get().map_or(0, |s| s.elapsed().as_millis() as u64)
}

/// True when the search should stop starting new work (the hard deadline is at ~1.5x this).
pub fn soft_expired() -> bool {
    elapsed_ms() >= *SOFT_LIMIT_MS.lock().unwrap_or_else(|e| e.into_inner())
}

/// Hard deadline in milliseconds for an input of this many instructions. Normal programs get
/// 5 s from the judges, the very large ones 30 s; both include start-up.
pub fn hard_limit_ms(ninstr: usize) -> u64 {
    // developer aid: KOPT_HARD_MS overrides the limit (to test the fallback)
    if let Some(v) = std::env::var("KOPT_HARD_MS").ok().and_then(|v| v.parse::<u64>().ok()) {
        return v;
    }
    if ninstr < 40_000 {
        2_600
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
    *SOFT_LIMIT_MS.lock().unwrap_or_else(|e| e.into_inner()) = hard * 2 / 3;
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
    worst > 4000
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
// Hot/cold layout
// ---------------------------------------------------------------------------
static LAYOUT: AtomicU8 = AtomicU8::new(0);

pub fn set_layout(mode: u8) {
    LAYOUT.store(mode, Ordering::Relaxed);
}

/// Block order for the final program. Mode 0 = the classic chaining order. Mode 1 = the same, but
/// when the entry block is a big piece of run-once code in front of the loops, a tiny entry block
/// `jmp OLD` is put at index 0 and all loop blocks follow it, so that the code that runs most sits
/// in the first `icache_size` slots and the run-once code moves behind it. (The entry block must
/// stay at index 0; the extra jump costs one taken jump, once.)
pub fn layout(f: &mut crate::ir::Func, mc: &Machine, profile: Option<&crate::backend::Profile>) {
    use crate::ir::*;
    crate::backend::layout(f, profile);
    if LAYOUT.load(Ordering::Relaxed) == 0 {
        return;
    }
    let c = mc.icache_size.max(1) as usize;
    let entry = f.blocks[0];
    if f.arena[entry].ins.len() < 4 || !f.arena[entry].preds.is_empty() {
        return;
    }
    let hot: Vec<usize> = f.blocks.iter().copied().filter(|&b| f.arena[b].depth >= 1).collect();
    if hot.is_empty() || f.arena[entry].depth >= 1 {
        return;
    }
    let hot_size: usize = hot.iter().map(|&b| f.arena[b].ins.len()).sum();
    // the split only helps when the run-once code in front would push the loops out of the cache
    if hot_size + 1 > c {
        return;
    }
    let hotset: HashSet<usize> = hot.iter().copied().collect();
    let ne = f.new_block("entry");
    f.arena[ne].ins.push(Instr::no_d(Op::Jmp, vec![], vec![entry]));
    let rest: Vec<usize> = f.blocks.iter().copied().filter(|b| !hotset.contains(b)).collect();
    let mut order = vec![ne];
    order.extend(hot);
    order.extend(rest);
    f.blocks = order;
    compute_preds(f);
}
