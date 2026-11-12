//! evalsim.rs - kopt's own measuring stick.
//!
//! kopt runs the input program and each candidate output on a fixed set of made-up inputs.
//! It uses this for two things: to pick the fastest of several candidate outputs (the timing
//! model is exact), and as a safety net: a candidate that behaves differently from the input
//! is thrown away. Everything here is deterministic (fixed seeds, step budgets, no clocks).
use crate::kir::*;
use std::collections::BTreeMap;

fn timing_inputs(nparams: usize, count: usize) -> Vec<Inputs> {
    let mut rng = Rng::new(987654321);
    let mut out = Vec::new();
    for i in 0..count {
        let args: Vec<u32> = (0..nparams).map(|_| (8 + rng.below(40)) as u32).collect();
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
            (56, 4, 20000, 400000)
        };
        let mut r = Reference { mc: mc.clone(), checks: Vec::new(), timing: Vec::new() };
        let mut used = 0u64;
        let mut tsteps = 0u64;
        for inp in timing_inputs(nparams, ntime) {
            if !r.timing.is_empty() && tsteps > 12000 {
                break;
            }
            let mut res = execute(&cp, &inp, cap, true);
            used += res.steps;
            if res.outcome == Outcome::Timeout {
                continue;
            }
            if !r.timing.is_empty() && tsteps + res.steps > 16000 {
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

    /// Sum over the timing inputs of (candidate cycles / original cycles). None = wrong.
    pub fn score(&self, cp: &Compiled) -> Option<f64> {
        if self.timing.is_empty() {
            return Some(0.0);
        }
        let mut tot = 0.0f64;
        for (inp, rf, cyc) in &self.timing {
            let res = execute(cp, inp, 4 * rf.steps + 1000, true);
            if res.outcome == Outcome::Timeout || differs(rf, &res) {
                return None;
            }
            tot += time_trace(cp, &res.trace, &self.mc) as f64 / *cyc as f64;
        }
        Some(tot)
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
