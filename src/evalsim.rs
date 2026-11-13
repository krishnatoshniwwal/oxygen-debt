//! evalsim.rs - kopt's own measuring stick.
//!
//! kopt runs the input program and each candidate output on a fixed set of made-up inputs.
//! It uses this for two things: to pick the fastest of several candidate outputs (the timing
//! model is exact), and as a safety net: a candidate that behaves differently from the input
//! is thrown away. Everything here is deterministic (fixed seeds, step budgets, no clocks).
use crate::kir::*;
use std::collections::BTreeMap;

/// One argument for a timing input, drawn like the arguments of the kit's sample `.in` files:
/// mostly small counts (2..100), sometimes a few hundred or thousand, now and then a full 32-bit
/// value (seeds, bit patterns) or a small negative number. `shape` picks the size class, so the
/// set of timing inputs covers short and long runs of every loop.
fn timing_arg(rng: &mut Rng, shape: usize) -> u32 {
    let r = rng.below(16);
    if r == 0 {
        return rng.next() as u32; // full 32-bit value
    }
    if r == 1 {
        return (rng.below(10) as u32 + 1).wrapping_neg(); // -1 .. -10
    }
    let v = match shape {
        0 => 2 + rng.below(19),     // 2..20
        1 => 8 + rng.below(40),     // 8..47
        2 => 16 + rng.below(113),   // 16..128
        3 => 3 + rng.below(10),     // 3..12
        4 => 50 + rng.below(251),   // 50..300
        _ => 100 + rng.below(1901), // 100..2000
    };
    v as u32
}

fn timing_inputs(nparams: usize, count: usize) -> Vec<Inputs> {
    let mut rng = Rng::new(987654321);
    let mut out = Vec::new();
    for i in 0..count {
        let args: Vec<u32> = (0..nparams).map(|_| timing_arg(&mut rng, i % 6)).collect();
        let bound = [256u64, 100, 16, 1000][i % 4];
        let words: Vec<u32> = (0..GEN_MEM_WORDS).map(|_| rng.below(bound) as u32).collect();
        let mut vin = BTreeMap::new();
        for port in 0..4u32 {
            vin.insert(port, (0..24).map(|_| rng.below(1000) as u32).collect::<Vec<u32>>());
        }
        out.push(Inputs { args, segments: vec![(0, words)], vin });
    }
    out
}

/// The input program, compiled, with its behaviour on the test inputs.
pub struct Reference {
    pub mc: Machine,
    checks: Vec<(Inputs, ExecResult)>,
    timing: Vec<(Inputs, ExecResult, i64)>,
    nparams: usize,
    size: usize,
}

impl Reference {
    pub fn new(prog: &Program, mc: &Machine, size: usize) -> Reference {
        let cp = compile_program(prog);
        let nparams = prog.params.len();
        let (ncheck, ntime, cap, total): (usize, usize, u64, u64) = if size > 20000 {
            (6, 1, 4 * size as u64 + 1000, 6 * size as u64 + 100000)
        } else if size > 2000 {
            (24, 2, 40000, 300000)
        } else {
            (56, 8, 20000, 400000)
        };
        let mut r = Reference { mc: mc.clone(), checks: Vec::new(), timing: Vec::new(), nparams, size };
        let mut used = 0u64;
        let mut tsteps = 0u64;
        for inp in timing_inputs(nparams, ntime) {
            if !r.timing.is_empty() && tsteps > 40000 {
                break;
            }
            let mut res = execute(&cp, &inp, cap, true);
            used += res.steps;
            if res.outcome == Outcome::Timeout {
                continue;
            }
            if !r.timing.is_empty() && tsteps + res.steps > 60000 {
                continue;
            }
            tsteps += res.steps;
            let cyc = time_trace(&cp, &res.trace, mc);
            res.trace = Vec::new();
            r.timing.push((inp, res, cyc));
        }
        for inp in generate_inputs(nparams, ncheck, 4242) {
            if used > total {
                break;
            }
            let res = execute(&cp, &inp, cap, false);
            used += res.steps;
            if res.outcome == Outcome::Timeout {
                continue;
            }
            r.checks.push((inp, res));
        }
        r
    }

    /// Total cycles of the candidate over the timing inputs. None = wrong.
    /// Absolute cycles, not ratios: the judges rank by Average Cycles = total cycles / programs,
    /// so a long-running input matters exactly as much as its cycles.
    pub fn score(&self, cp: &Compiled) -> Option<f64> {
        if self.timing.is_empty() {
            return Some(0.0);
        }
        self.score_each(cp).map(|v| v.iter().sum())
    }

    /// Cycles of the candidate on each timing input (same order for every candidate). None = wrong.
    pub fn score_each(&self, cp: &Compiled) -> Option<Vec<f64>> {
        let mut out = Vec::with_capacity(self.timing.len());
        for (inp, rf, _) in &self.timing {
            let res = execute(cp, inp, 4 * rf.steps + 1000, true);
            if res.outcome == Outcome::Timeout || differs(rf, &res) {
                return None;
            }
            out.push(time_trace(cp, &res.trace, &self.mc) as f64);
        }
        Some(out)
    }

    /// The timing inputs (clones), for passes that tune against the real execution.
    pub fn timing_inputs(&self) -> Vec<Inputs> {
        self.timing.iter().map(|t| t.0.clone()).collect()
    }

    /// Execution traces of a candidate on the timing inputs (for edge counts in layout.rs).
    pub fn traces(&self, cp: &Compiled) -> Vec<Vec<u32>> {
        let mut out = Vec::new();
        for (inp, rf, _) in &self.timing {
            let res = execute(cp, inp, 4 * rf.steps + 1000, true);
            if res.outcome != Outcome::Timeout {
                out.push(res.trace);
            }
        }
        // more made-up inputs of the same shape (small programs only), so that block orders
        // are not fitted to just the 8 scored inputs
        if self.size <= 2000 {
            let mut steps = 0u64;
            for inp in timing_inputs(self.nparams, 32).into_iter().skip(8) {
                let res = execute(cp, &inp, 20000, true);
                if res.outcome == Outcome::Timeout {
                    continue;
                }
                steps += res.steps;
                out.push(res.trace);
                if steps > 300_000 {
                    break;
                }
            }
        }
        out
    }

    /// Total cycles of the input program over the timing inputs, or None if there are no
    /// timing inputs (then candidates cannot be compared with the input).
    pub fn base(&self) -> Option<f64> {
        if self.timing.is_empty() {
            return None;
        }
        Some(self.timing.iter().map(|t| t.2 as f64).sum())
    }

    /// True if the candidate behaves like the original on all check inputs.
    pub fn same(&self, cp: &Compiled) -> bool {
        for (inp, rf) in &self.checks {
            let res = execute(cp, inp, 4 * rf.steps + 1000, false);
            if res.outcome == Outcome::Timeout || differs(rf, &res) {
                return false;
            }
        }
        true
    }
}

impl Reference {
    /// The first `k` timing inputs (used by large.rs to count how often each block runs).
    pub fn first_inputs(&self, k: usize) -> Vec<Inputs> {
        self.timing.iter().take(k).map(|t| t.0.clone()).collect()
    }
}
