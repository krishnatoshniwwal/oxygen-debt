//! driver.rs - the kopt pipeline and the search over its knobs.
use crate::algebra;
use crate::backend;
use crate::evalsim::Reference;
use crate::ir::{self, Func};
use crate::kir::*;
use crate::loopopt;
use crate::passes;
use crate::passes2;
use crate::passes3;
use crate::ranges::Ranges;
use crate::sched2;
use crate::domain;
use crate::reassoc;
use crate::sccp;
use crate::ssa;
use std::collections::HashMap;
use std::panic::{catch_unwind, AssertUnwindSafe};

/// One setting of the optimizer's knobs.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Variant {
    pub peel: u8,     // 0 = off, 1 = within the instruction cache, 2 = loose
    pub unroll: usize, // loop unrolling factor applied before the second SSA round
    pub ifc: usize,   // if-conversion: max instructions per side (0 = off)
    pub sched: bool,
    pub mac: bool,
    pub spill: bool, // keep values in scratch memory when registers run out
    pub fwd: bool,   // reuse a loaded value for a later load of the same word (else reload)
    pub spec: u8,    // speculation inside loops: 0 = off, else the number of sweeps (blocks moved up)
    pub hot: bool,   // with more names than registers: hottest values get the lowest names
    pub csub: bool,  // x % m with 0 <= x < 2m proven -> conditional subtract
    pub spill1: u8,  // >0: keep the n-th least-touched value live through the hottest blocks in scratch
    pub iv: bool,    // induction-variable rewrite of i + k in loops (and of the exit test)
    pub lazy: bool,  // lazy modular reduction of running sums in counted loops
    pub rot: u8,     // latency rotation of loops (loopopt): 0 = off, 1 = best set by the model, 2 = all long-latency work
    pub gu: u8,      // guarded unrolling (loopopt): 0 = off, else u bodies per test
    pub acc: u8,     // accumulator splitting (loopopt): 0 = off, else at most this many partial chains
    pub sr: bool,    // scalar replacement of loads across iterations (loopopt)
    pub alg: u8,     // loop algebra (algebra.rs), bit 1 = closed forms of polynomial loops
    pub thr: bool,   // tree-height reduction of add/mul/xor/and/or chains
    pub sccp: bool,  // optimistic conditional constant propagation combined with the ranges
}

impl Default for Variant {
    fn default() -> Variant {
        Variant { peel: 1, unroll: 1, ifc: 4, sched: true, mac: true, spill: false, fwd: true, spec: 0, hot: false, csub: true, spill1: 0, iv: false, lazy: false, rot: 0, gu: 0, acc: 0, sr: false }
        Variant { peel: 1, unroll: 1, ifc: 4, sched: true, mac: true, spill: false, fwd: true, spec: 0, hot: false, csub: true, spill1: 0, iv: false, lazy: false, alg: 0 }
        Variant { peel: 1, unroll: 1, ifc: 4, sched: true, mac: true, spill: false, fwd: true, spec: 0, hot: false, csub: true, spill1: 0, iv: false, lazy: false, thr: false, sccp: true }
    }
}

/// Developer timing: with the environment variable KOPT_PROF set, print how long the step
/// since the last call took. Without it the clock is never read, and the output file never
/// depends on it either way.
fn prof_start() -> Option<std::time::Instant> {
    if std::env::var_os("KOPT_PROF").is_some() {
        Some(std::time::Instant::now())
    } else {
        None
    }
}

fn prof(label: &str, t0: &mut Option<std::time::Instant>) {
    if let Some(t) = t0 {
        eprintln!("kopt-prof {:>8.3}s {}", t.elapsed().as_secs_f64(), label);
        *t0 = Some(std::time::Instant::now());
    }
}

fn chk(f: &mut Func, what: &str) {
    if std::env::var_os("KOPT_CHECK").is_some() {
        if let Err(e) = ssa::check_ssa(f) {
            panic!("invalid SSA after {}: {}", what, e);
        }
    }
}

/// The SSA optimizations, repeated until nothing changes. light=true (very large programs):
/// one round only.
fn phase_a(f: &mut Func, v: &Variant, light: bool) {
    let scalar = |f: &mut Func| {
        for _ in 0..(if light { 2 } else { 6 }) {
            let c1 = passes::scalar_pass(f);
            chk(f, "scalar_pass");
            let c2 = passes::cleanup_cfg(f);
            chk(f, "cleanup_cfg");
            let c3 = passes::dce(f, None);
            chk(f, "dce");
            if !(c1 || c2 || c3) {
                break;
            }
        }
    };
    let mut t = prof_start();
    scalar(f);
    prof("  phase_a scalar", &mut t);
    if light {
        let rng = Ranges::new(f, 6);
        passes2::range_simplify(f, &rng, v.csub);
        passes2::mem_pass(f, &rng, v.fwd);
        passes2::licm(f, &rng);
        passes::scalar_pass(f);
        passes::cleanup_cfg(f);
        return;
    }
    // Large programs: at most 2 rounds (the value-range analysis is the costly part of a round).
    let rounds = if f.count_instrs() > MID { 2 } else { 4 };
    for _ in 0..rounds {
        let mut lz = false;
        let mut sc = false;
        if v.sccp {
            let rng = Ranges::new(f, 14);
            if sccp::sccp(f, Some(&rng)) {
                chk(f, "sccp");
                scalar(f);
                sc = true;
            }
            if domain::bits_simplify(f) {
                chk(f, "bits_simplify");
                scalar(f);
                sc = true;
            }
        }
        if v.lazy {
            let rng = Ranges::new(f, 14);
            if passes3::lazy_mod(f, &rng) {
                chk(f, "lazy_mod");
                scalar(f);
                lz = true;
            }
        }
        if v.alg & 1 != 0 {
            let rng = Ranges::new(f, 14);
            if algebra::closed_forms(f, &rng) {
                chk(f, "closed_forms");
                scalar(f);
                lz = true;
            }
        }
        if v.alg & 2 != 0 {
            let rng = Ranges::new(f, 14);
            if algebra::div_const(f, &rng) {
                chk(f, "div_const");
                scalar(f);
                lz = true;
            }
        }
        let mut rng = Ranges::new(f, 14);
        prof("  ranges", &mut t);
        let c1 = passes2::range_simplify(f, &rng, v.csub);
        chk(f, "range_simplify");
        if c1 {
            scalar(f);
            rng = Ranges::new(f, 14);
        }
        prof("  range_simplify (+scalar, ranges)", &mut t);
        let c2 = passes2::mem_pass(f, &rng, v.fwd);
        chk(f, "mem_pass");
        if c2 {
            scalar(f);
            rng = Ranges::new(f, 14);
        }
        prof("  mem_pass (+scalar, ranges)", &mut t);
        let mut c3 = passes2::licm(f, &rng);
        chk(f, "licm");
        if c3 {
            scalar(f);
        }
        if v.iv {
            let rng = Ranges::new(f, 14);
            if passes3::iv_rewrite(f, &rng) {
                chk(f, "iv_rewrite");
                scalar(f);
                c3 = true;
            }
        }
        prof("  licm (+scalar)", &mut t);
        let mut c4 = false;
        if v.ifc > 0 {
            let rng = Ranges::new(f, 14);
            c4 = passes3::ifconv(f, &rng, v.ifc);
            chk(f, "ifconv");
            if c4 {
                scalar(f);
            }
        }
        prof("  ifconv (+ranges, scalar)", &mut t);
        if !(sc || lz || c1 || c2 || c3 || c4) {
            break;
        }
    }
}

/// Everything up to (not including) the choice of block order and final code copying.
fn midend(prog: &Program, mc: &Machine, v: &Variant, light: bool) -> Result<Func, String> {
    let mut t = prof_start();
    let mut f = ir::from_program(prog);
    let ic = mc.icache_size;
    ssa::build_ssa(&mut f);
    chk(&mut f, "build_ssa");
    prof("build_ssa", &mut t);
    phase_a(&mut f, v, light);
    prof("phase_a total", &mut t);
    if v.peel != 0 {
        let budget = if v.peel == 1 { ic } else { (4 * ic).max(256) } as usize;
        for _ in 0..4 {
            let trips = passes3::find_const_trips(&mut f, 64);
            if trips.is_empty() {
                break;
            }
            ssa::destruct_ssa(&mut f);
            let ch = passes3::peel_const_loops(&mut f, &trips, budget, 16);
            ssa::build_ssa(&mut f);
            phase_a(&mut f, v, false);
            if !ch {
                break;
            }
        }
        prof("peeling (+phase_a)", &mut t);
    }
    if v.unroll > 1 {
        ssa::destruct_ssa(&mut f);
        let size = f.count_instrs() as i64;
        let budget = if v.peel != 2 { (ic - size).max(0) } else { ic.max(2 * size) };
        if backend::unroll(&mut f, v.unroll, budget) {
            backend::final_cleanup(&mut f);
        }
        ssa::build_ssa(&mut f);
        phase_a(&mut f, v, false);
    }
    let guv = std::env::var("KOPT_FORCE_GU").ok().and_then(|s| s.parse::<u8>().ok()).unwrap_or(v.gu);
    if guv > 1 && !light {
        if loopopt::unroll_guarded(&mut f, guv as usize) {
            chk(&mut f, "unroll_guarded");
            phase_a(&mut f, v, false);
        }
    }
    let mut rng = Ranges::new(&mut f, if light { 6 } else { 14 });
    passes::dce(&mut f, Some(&rng));
    passes::cleanup_cfg(&mut f);
    // may we use the scratch region? only if the program provably never touches it
    let mut scratch_ok = rng.ok;
    if scratch_ok {
        'o: for &b in &f.blocks {
            for ins in &f.b(b).ins {
                if (ins.op == Op::Load || ins.op == Op::Store) && rng.get(ins.a[0], b).1 >= 3072 {
                    scratch_ok = false;
                    break 'o;
                }
            }
        }
    }
    prof("unroll, ranges, dce", &mut t);
    if v.thr {
        reassoc::reassoc(&mut f);
        chk(&mut f, "reassoc");
    }
    passes2::lower(&mut f, mc, v.mac);
    let srv = std::env::var_os("KOPT_FORCE_SR").is_some() || v.sr;
    if srv && !light {
        loopopt::reuse_loads(&mut f);
        chk(&mut f, "reuse_loads");
    }
    let rotv = std::env::var("KOPT_FORCE_ROT").ok().and_then(|s| s.parse::<u8>().ok()).unwrap_or(v.rot);
    if rotv > 0 && !light {
        loopopt::rotate_loops(&mut f, mc, rotv);
        chk(&mut f, "rotate_loops");
    }
    let accv = std::env::var("KOPT_FORCE_ACC").ok().and_then(|s| s.parse::<u8>().ok()).unwrap_or(v.acc);
    if accv > 1 {
        loopopt::split_accumulators(&mut f, accv as usize);
        chk(&mut f, "split_accumulators");
    }
    if v.sched {
        if !light {
            rng = Ranges::new(&mut f, 14);
        }
        if v.spec > 0 {
            passes3::speculate(&mut f, &rng, 64, v.spec as usize);
            chk(&mut f, "speculate");
        }
        passes3::schedule(&mut f, mc, &rng);
    }
    prof("lower, schedule", &mut t);
    ir::analyze(&mut f);
    for bi in 0..f.blocks.len() {
        let b = f.blocks[bi];
        let d = f.arena[b].depth as i32;
        f.arena[b].freq = 8f64.powi(d).min(1e9);
    }
    ssa::destruct_ssa(&mut f);
    prof("destruct_ssa", &mut t);
    let k = mc.registers as usize;
    let force = if scratch_ok { v.spill1 as usize } else { 0 };
    f.nreg = ssa::allocate(&mut f, Some(k), v.spill && scratch_ok, v.hot, force)?;
    prof("allocate", &mut t);
    backend::final_cleanup(&mut f);
    prof("final_cleanup", &mut t);
    Ok(f)
}

/// Code copying and block order for one candidate. Returns the program text.
fn finish(f: &mut Func, mc: &Machine, profile: Option<&backend::Profile>, maxdup: usize, factor: usize, loose: bool) -> String {
    let size = f.count_instrs() as i64;
    let ic = mc.icache_size;
    let mut budget = if loose { ic.max(2 * size) } else { (ic - size).max(0) };
    if maxdup > 0 {
        backend::tail_dup(f, maxdup, budget);
        budget -= f.count_instrs() as i64 - size;
    }
    if factor > 1 {
        backend::unroll(f, factor, budget.max(0));
        backend::final_cleanup(f);
    }
    backend::layout(f, profile);
    ir::to_text(f)
}

// Programs with more than MID instructions skip the general search and get only a few
// whole-program candidates (each costs about one pipeline run: ~0.6 s at 100k instructions).
// Programs with more than BIG instructions get the light pipeline (one round, no search).
const MID: usize = 6000;
const BIG: usize = 300_000;

// (maxdup, unroll factor, loose code-size budget)
const BACKENDS: [(usize, usize, bool); 9] = [
    (4, 1, false),
    (0, 1, false),
    (12, 1, false),
    (4, 2, false),
    (4, 4, false),
    (4, 8, false),
    (12, 2, false),
    (4, 2, true),
    (12, 1, true),
];

type Backend = (usize, usize, bool);

struct Cand {
    score: f64,
    size: usize,
    order: usize,
    text: String,
    cp: Compiled,
    desc: String,
}

struct Search<'a> {
    prog: &'a Program,
    mc: &'a Machine,
    profile: Option<&'a backend::Profile>,
    rf: &'a Reference,
    mids: HashMap<Variant, Option<Func>>,
    results: Vec<Cand>,
    seen: HashMap<String, f64>,
    n: usize,
    size: usize,
    light: bool,
    spent: usize,
}

fn panic_msg(e: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = e.downcast_ref::<&str>() {
        s.to_string()
    } else if let Some(s) = e.downcast_ref::<String>() {
        s.clone()
    } else {
        "panic".to_string()
    }
}

impl<'a> Search<'a> {
    fn mid(&mut self, v: &Variant) {
        if self.mids.contains_key(v) {
            return;
        }
        let r = catch_unwind(AssertUnwindSafe(|| midend(self.prog, self.mc, v, self.light)));
        match r {
            Ok(Ok(f)) => {
                self.spent += self.size + f.count_instrs();
                self.mids.insert(*v, Some(f));
            }
            other => {
                let msg = match other {
                    Ok(Err(e)) => e,
                    Err(p) => panic_msg(&p),
                    _ => String::new(),
                };
                eprintln!("kopt: variant {:?} failed ({})", v, msg);
                self.mids.insert(*v, None);
                self.spent += 2 * self.size;
            }
        }
    }

    /// Score one (variant, backend) pair. Returns the score or None.
    fn run(&mut self, v: &Variant, be: Backend) -> Option<f64> {
        self.mid(v);
        let mut f = match self.mids.get(v).unwrap() {
            Some(f0) => f0.clone(),
            None => return None,
        };
        self.n += 1;
        self.spent += f.count_instrs() / 3;
        let (prog, mc, profile, rf) = (self.prog, self.mc, self.profile, self.rf);
        let r = catch_unwind(AssertUnwindSafe(|| -> Result<(String, Option<(Compiled, usize, f64)>, f64), String> {
            let mut t = prof_start();
            let out = finish(&mut f, mc, profile, be.0, be.1, be.2);
            prof("finish (tail copy, unroll, layout, text)", &mut t);
            if let Some(&s) = self.seen.get(&out) {
                return Ok((out, None, s));
            }
            let newprog = parse_program(&out)?;
            check_machine_features(&newprog, mc)?;
            if newprog.params.len() != prog.params.len() {
                return Err("parameter count changed".into());
            }
            let cp = compile_program(&newprog);
            let sc = match rf.score(&cp) {
                Some(sc) => sc,
                None => {
                    // developer aid: KOPT_DUMP_BAD=FILE writes the rejected candidate there
                    if let Some(p) = std::env::var_os("KOPT_DUMP_BAD") {
                        let _ = std::fs::write(p, &out);
                    }
                    return Err("behaves differently on a timing input".to_string());
                }
            };
            prof("parse + score", &mut t);
            Ok((out, Some((cp, newprog.instrs.len(), sc)), sc))
        }));
        if std::env::var_os("KOPT_PROF").is_some() {
            if let Ok(Ok((_, _, sc))) = &r {
                eprintln!("kopt-cand #{} score {:>10.0} spent {:>6} {:?} {:?}", self.n, sc, self.spent, v, be);
            }
        }
        // developer aid: KOPT_DUMP_ALL=DIR writes every candidate as DIR/cand_N.kir
        if let (Some(d), Ok(Ok((out, _, _)))) = (std::env::var_os("KOPT_DUMP_ALL"), &r) {
            let _ = std::fs::write(std::path::Path::new(&d).join(format!("cand_{}.kir", self.n)), out);
        }
        match r {
            Ok(Ok((out, Some((cp, size, sc)), _))) => {
                self.seen.insert(out.clone(), sc);
                self.results.push(Cand { score: sc, size, order: self.n, text: out, cp, desc: format!("{:?} {:?}", v, be) });
                Some(sc)
            }
            Ok(Ok((_, None, sc))) => Some(sc),
            other => {
                let msg = match other {
                    Ok(Err(e)) => e,
                    Err(p) => panic_msg(&p),
                    _ => String::new(),
                };
                eprintln!("kopt: candidate {:?} {:?} failed ({})", v, be, msg);
                None
            }
        }
    }
}

const WORK_BUDGET: usize = 16000; // the search stops when this much work has been spent

/// Return the optimized program text. Falls back to the input text on any problem.
pub fn optimize(text: &str, mc: &Machine, profile_text: Option<&str>) -> Result<String, String> {
    let prog = parse_program(text)?; // an invalid input is an error
    let profile = profile_text.map(backend::parse_profile);
    let size = prog.instrs.len();
    let mut t = prof_start();
    let rf = match catch_unwind(AssertUnwindSafe(|| Reference::new(&prog, mc, size))) {
        Ok(r) => r,
        Err(_) => return Ok(text.to_string()),
    };
    prof("reference runs", &mut t);
    let mut s = Search {
        prog: &prog,
        mc,
        profile: profile.as_ref(),
        rf: &rf,
        mids: HashMap::new(),
        results: Vec::new(),
        seen: HashMap::new(),
        n: 0,
        size,
        light: size > BIG,
        spent: 0,
    };
    let b0 = BACKENDS[0];
    let v0 = if size > BIG {
        Variant { peel: 0, ifc: 0, ..Variant::default() }
    } else if size > MID {
        Variant { peel: 0, ..Variant::default() } // peeling re-runs phase_a: too slow here
    } else {
        Variant::default()
    };
    let mut state: (Variant, Option<f64>) = (v0, None);
    state.1 = s.run(&state.0, b0);

    fn consider(s: &mut Search, state: &mut (Variant, Option<f64>), v: Variant) {
        if s.spent > WORK_BUDGET {
            return;
        }
        let sc = s.run(&v, BACKENDS[0]);
        if let Some(sc) = sc {
            if state.1.map_or(true, |b| sc < b - 1e-9) {
                *state = (v, Some(sc));
            }
        }
    }

    // (above 150k instructions only the first candidate: time grows faster than linearly)
    if size > MID && size <= 150_000 {
        // Large programs: no general search, but the settings that matter most for long
        // straight-line code: reload instead of keeping loaded values alive (register
        // pressure), spilling to scratch, and no if-conversion.
        let over_k = match s.mids.get(&state.0) {
            Some(Some(f0)) => f0.nreg > mc.registers,
            _ => false,
        };
        let b = state.0;
        let mut tries = vec![Variant { fwd: false, ..b }];
        if over_k {
            tries.push(Variant { spill: true, ..b });
            tries.push(Variant { hot: true, ..b });
        }
        tries.push(Variant { ifc: 0, ..b });
        for v in tries {
            let sc = s.run(&v, b0);
            if let Some(sc) = sc {
                if state.1.map_or(true, |x| sc < x - 1e-9) {
                    state = (v, Some(sc));
                }
            }
        }
    }
    if size <= MID {
        let over_k = |s: &Search, v: &Variant| -> bool {
            match s.mids.get(v) {
                Some(Some(f0)) => f0.nreg > mc.registers,
                _ => false,
            }
        };
        // loop algebra (algebra.rs): tried first, because a deleted loop makes most other settings moot
        let b = state.0;
        consider(&mut s, &mut state, Variant { alg: 1, ..b });
        let b = state.0;
        consider(&mut s, &mut state, Variant { alg: b.alg | 2, ..b });
        if over_k(&s, &state.0) {
            // More names than registers. Spilling to scratch is one answer; keeping the
            // program order (scheduling stretches live ranges) and giving hot values the low
            // names (LRU register file) are others. Try them before the search settles.
            let b = state.0;
            for v in [
                Variant { spill: true, ..b },
                Variant { sched: false, ..b },
                Variant { hot: true, ..b },
                Variant { sched: false, hot: true, ..b },
                Variant { spill1: 1, ..b },
                Variant { spill1: 2, ..b },
                Variant { spill1: 3, ..b },
            ] {
                consider(&mut s, &mut state, v);
            }
        }
        let v = Variant { peel: 0, unroll: 1, ifc: 0, sched: false, spill: state.0.spill, mac: true, fwd: true, spec: 0, hot: false, csub: true, spill1: 0, iv: false, lazy: false, rot: 0, gu: 0, acc: 0, sr: false };
        let v = Variant { peel: 0, unroll: 1, ifc: 0, sched: false, spill: state.0.spill, mac: true, fwd: true, spec: 0, hot: false, csub: true, spill1: 0, iv: false, lazy: false, alg: 0 };
        let v = Variant { peel: 0, unroll: 1, ifc: 0, sched: false, spill: state.0.spill, mac: true, fwd: true, spec: 0, hot: false, csub: true, spill1: 0, iv: false, lazy: false, thr: false, sccp: false };
        consider(&mut s, &mut state, v);
        for u in [2, 4, 8] {
            let v = Variant { unroll: u, ..state.0 };
            consider(&mut s, &mut state, v);
        }
        // speculation pays mostly on unrolled loops; if it is adopted, try the unroll
        // factors again with it
        for sp in [1u8, 8] {
            let v = Variant { spec: sp, ..state.0 };
            consider(&mut s, &mut state, v);
        }
        if state.0.spec > 0 {
            for u in [2, 4, 8] {
                if u != state.0.unroll {
                    let v = Variant { unroll: u, ..state.0 };
                    consider(&mut s, &mut state, v);
                }
            }
        }
        for ifc in [0, 2, 8, 24] {
            let v = Variant { ifc, ..state.0 };
            consider(&mut s, &mut state, v);
        }
        let v = Variant { sched: !state.0.sched, ..state.0 };
        consider(&mut s, &mut state, v);
        let v = Variant { mac: false, ..state.0 };
        consider(&mut s, &mut state, v);
        let v = Variant { peel: if state.0.peel != 0 { 0 } else { 1 }, ..state.0 };
        consider(&mut s, &mut state, v);
        let v = Variant { peel: 2, ..state.0 };
        consider(&mut s, &mut state, v);
        let v = Variant { spill: !state.0.spill, ..state.0 };
        consider(&mut s, &mut state, v);
        let v = Variant { fwd: !state.0.fwd, ..state.0 };
        consider(&mut s, &mut state, v);
        if over_k(&s, &state.0) {
            let v = Variant { hot: true, ..state.0 };
            consider(&mut s, &mut state, v);
        }
        let v = Variant { csub: !state.0.csub, ..state.0 };
        consider(&mut s, &mut state, v);
        let v = Variant { iv: !state.0.iv, ..state.0 };
        consider(&mut s, &mut state, v);
        let v = Variant { lazy: !state.0.lazy, ..state.0 };
        consider(&mut s, &mut state, v);
        let v = Variant { sccp: !state.0.sccp, ..state.0 };
        consider(&mut s, &mut state, v);
        let v = Variant { thr: !state.0.thr, ..state.0 };
        consider(&mut s, &mut state, v);
        for be in BACKENDS[1..].iter() {
            if s.spent > WORK_BUDGET {
                break;
            }
            let v = state.0;
            s.run(&v, *be);
        }
        // Refinement: the walk above is greedy, so its result depends on the order of the
        // steps (a setting adopted early can hide a better combination). Try every single
        // change of the final setting that was not measured yet; if one is better, move
        // there, measure the back ends for it, and repeat once.
        for _ in 0..2 {
            let b = state.0;
            let mut nb: Vec<Variant> = Vec::new();
            for sp in [0u8, 1, 8] {
                nb.push(Variant { spec: sp, ..b });
            }
            for u in [1, 2, 4, 8] {
                nb.push(Variant { unroll: u, ..b });
            }
            for ifc in [0, 4] {
                nb.push(Variant { ifc, ..b });
            }
            for pl in [0u8, 1, 2] {
                nb.push(Variant { peel: pl, ..b });
            }
            nb.push(Variant { sched: !b.sched, ..b });
            nb.push(Variant { spill: !b.spill, ..b });
            nb.push(Variant { fwd: !b.fwd, ..b });
            nb.push(Variant { hot: !b.hot, ..b });
            nb.push(Variant { csub: !b.csub, ..b });
            nb.push(Variant { mac: !b.mac, ..b });
            nb.push(Variant { iv: !b.iv, ..b });
            nb.push(Variant { lazy: !b.lazy, ..b });
            nb.push(Variant { alg: b.alg ^ 1, ..b });
            nb.push(Variant { alg: b.alg ^ 2, ..b });
            nb.push(Variant { sccp: !b.sccp, ..b });
            nb.push(Variant { thr: !b.thr, ..b });
            for s1 in [0u8, 1, 2] {
                nb.push(Variant { spill1: s1, ..b });
            }
            for v in nb {
                if v != b && !s.mids.contains_key(&v) {
                    consider(&mut s, &mut state, v);
                }
            }
            if state.0 == b {
                break;
            }
            for be in BACKENDS[1..].iter() {
                if s.spent > WORK_BUDGET {
                    break;
                }
                let v = state.0;
                s.run(&v, *be);
            }
        }
    }
    fn consider2(s: &mut Search, state: &mut (Variant, Option<f64>), v: Variant) {
        // developer aid: KOPT_LOOPOPT=mask limits the loopopt settings (1 rot, 2 gu, 4 acc, 8 sr)
        let mask = std::env::var("KOPT_LOOPOPT").ok().and_then(|x| x.parse::<u32>().ok()).unwrap_or(15);
        if (v.rot > 0 && mask & 1 == 0) || (v.gu > 0 && mask & 2 == 0) || (v.acc > 0 && mask & 4 == 0) || (v.sr && mask & 8 == 0) {
            return;
        }
        if s.spent > WORK_BUDGET * 2 {
            return;
        }
        if let Some(sc) = s.run(&v, BACKENDS[0]) {
            if state.1.map_or(true, |b| sc < b - 1e-9) {
                *state = (v, Some(sc));
            }
        }
    }
    if size <= MID {
        // Loop transformations of loopopt.rs. They run after the general search, on its final
        // setting, so that they can only add candidates (never change the path of the walk).
        let b = state.0;
        let mut tries: Vec<Variant> = Vec::new();
        for g in [2u8, 4, 8] {
            tries.push(Variant { gu: g, ..b });
            tries.push(Variant { gu: g, unroll: 1, ..b });
        }
        for r in [1u8, 2] {
            tries.push(Variant { rot: r, ..b });
            tries.push(Variant { rot: r, unroll: 1, ..b });
        }
        for a in [2u8, 4] {
            tries.push(Variant { acc: a, ..b });
        }
        tries.push(Variant { sr: true, ..b });
        tries.push(Variant { sr: true, unroll: 1, ..b });
        let before = state.0;
        for v in tries {
            consider2(&mut s, &mut state, v);
        }
        // combine: rotation on top of the best guarded unrolling and the other way round
        let b2 = state.0;
        if b2 != before {
            for r in [0u8, 1, 2] {
                consider2(&mut s, &mut state, Variant { rot: r, ..b2 });
            }
            for g in [0u8, 2, 4, 8] {
                let cur = state.0;
                consider2(&mut s, &mut state, Variant { gu: g, ..cur });
            }
            for a in [0u8, 2, 4] {
                let cur = state.0;
                consider2(&mut s, &mut state, Variant { acc: a, ..cur });
            }
            for sr in [false, true] {
                let cur = state.0;
                consider2(&mut s, &mut state, Variant { sr, ..cur });
            }
            for be in BACKENDS[1..].iter() {
                if s.spent > WORK_BUDGET * 2 {
                    break;
                }
                let v = state.0;
                s.run(&v, *be);
            }
        }
    }
    post_sched2(&mut s);
    let mut order: Vec<usize> = (0..s.results.len()).collect();
    order.sort_by(|&a, &b| {
        let (x, y) = (&s.results[a], &s.results[b]);
        x.score.partial_cmp(&y.score).unwrap().then(x.size.cmp(&y.size)).then(x.order.cmp(&y.order))
    });
    // Do no harm: a candidate is written only if it is strictly faster than the input program
    // on the timing inputs. Otherwise the input itself is the best answer (and has no risk).
    let base = rf.base();
    prof("search (all candidates)", &mut t);
    for i in order {
        let c = &s.results[i];
        if let Some(b) = base {
            if c.score >= b - 1e-9 {
                // sorted by score: no later candidate is faster either. Not an error, so quiet.
                return Ok(text.to_string());
            }
        }
        let ok = catch_unwind(AssertUnwindSafe(|| rf.same(&c.cp) && crate::validate::deep_check(&prog, &c.text, mc))).unwrap_or(false);
        prof("self-check", &mut t);
        if ok {
            return Ok(c.text.clone());
        }
        eprintln!("kopt: candidate {} rejected by the self-check", c.desc);
    }
    eprintln!("kopt: writing the input unchanged");
    Ok(text.to_string())
}

/// sched2 (exact-cost reordering and renaming) on the best candidates found so far. Each result
/// is a new candidate: it is kept only if it measures faster, like every other candidate.
fn post_sched2(s: &mut Search) {
    if s.size > MID || s.results.is_empty() {
        return;
    }
    let inputs = s.rf.timing_inputs();
    if inputs.is_empty() {
        return;
    }
    let mut idx: Vec<usize> = (0..s.results.len()).collect();
    idx.sort_by(|&a, &b| {
        let (x, y) = (&s.results[a], &s.results[b]);
        x.score.partial_cmp(&y.score).unwrap().then(x.size.cmp(&y.size)).then(x.order.cmp(&y.order))
    });
    let mc = s.mc;
    let nparams = s.prog.params.len();
    for &i in idx.iter().take(3) {
        let text = s.results[i].text.clone();
        let r = catch_unwind(AssertUnwindSafe(|| sched2::optimize(&text, mc, &inputs, 4_000_000)));
        let t2 = match r {
            Ok(Some(t)) => t,
            _ => continue,
        };
        if s.seen.contains_key(&t2) {
            continue;
        }
        let rf = s.rf;
        let ok = catch_unwind(AssertUnwindSafe(|| -> Option<(Compiled, usize, f64)> {
            let np = parse_program(&t2).ok()?;
            check_machine_features(&np, mc).ok()?;
            if np.params.len() != nparams {
                return None;
            }
            let cp = compile_program(&np);
            let sc = rf.score(&cp)?;
            Some((cp, np.instrs.len(), sc))
        }));
        if let Ok(Some((cp, size, sc))) = ok {
            s.n += 1;
            s.seen.insert(t2.clone(), sc);
            let desc = format!("sched2 of {}", s.results[i].desc);
            s.results.push(Cand { score: sc, size, order: s.n, text: t2, cp, desc });
        }
    }
}
