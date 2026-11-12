//! driver.rs - the kopt pipeline and the search over its knobs.
use crate::backend;
use crate::evalsim::Reference;
use crate::ir::{self, Func};
use crate::kir::*;
use crate::passes;
use crate::passes2;
use crate::passes3;
use crate::ranges::Ranges;
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
}

impl Default for Variant {
    fn default() -> Variant {
        Variant { peel: 1, unroll: 1, ifc: 4, sched: true, mac: true, spill: false }
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
    scalar(f);
    if light {
        let rng = Ranges::new(f, 6);
        passes2::range_simplify(f, &rng);
        passes2::mem_pass(f, &rng);
        passes2::licm(f, &rng);
        passes::scalar_pass(f);
        passes::cleanup_cfg(f);
        return;
    }
    for _ in 0..4 {
        let mut rng = Ranges::new(f, 14);
        let c1 = passes2::range_simplify(f, &rng);
        chk(f, "range_simplify");
        if c1 {
            scalar(f);
            rng = Ranges::new(f, 14);
        }
        let c2 = passes2::mem_pass(f, &rng);
        chk(f, "mem_pass");
        if c2 {
            scalar(f);
            rng = Ranges::new(f, 14);
        }
        let c3 = passes2::licm(f, &rng);
        chk(f, "licm");
        if c3 {
            scalar(f);
        }
        let mut c4 = false;
        if v.ifc > 0 {
            let rng = Ranges::new(f, 14);
            c4 = passes3::ifconv(f, &rng, v.ifc);
            chk(f, "ifconv");
            if c4 {
                scalar(f);
            }
        }
        if !(c1 || c2 || c3 || c4) {
            break;
        }
    }
}

/// Everything up to (not including) the choice of block order and final code copying.
fn midend(prog: &Program, mc: &Machine, v: &Variant, light: bool) -> Result<Func, String> {
    let mut f = ir::from_program(prog);
    let ic = mc.icache_size;
    ssa::build_ssa(&mut f);
    chk(&mut f, "build_ssa");
    phase_a(&mut f, v, light);
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
    passes2::lower(&mut f, mc, v.mac);
    if v.sched {
        if !light {
            rng = Ranges::new(&mut f, 14);
        }
        passes3::schedule(&mut f, mc, &rng);
    }
    ir::analyze(&mut f);
    for bi in 0..f.blocks.len() {
        let b = f.blocks[bi];
        let d = f.arena[b].depth as i32;
        f.arena[b].freq = 8f64.powi(d).min(1e9);
    }
    ssa::destruct_ssa(&mut f);
    let k = mc.registers as usize;
    f.nreg = ssa::allocate(&mut f, Some(k), v.spill && scratch_ok)?;
    backend::final_cleanup(&mut f);
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

const BIG: usize = 6000; // programs with more instructions than this get the light pipeline

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
            let out = finish(&mut f, mc, profile, be.0, be.1, be.2);
            if let Some(&s) = self.seen.get(&out) {
                return Ok((out, None, s));
            }
            let newprog = parse_program(&out)?;
            check_machine_features(&newprog, mc)?;
            if newprog.params.len() != prog.params.len() {
                return Err("parameter count changed".into());
            }
            let cp = compile_program(&newprog);
            let sc = rf.score(&cp).ok_or_else(|| "behaves differently on a timing input".to_string())?;
            Ok((out, Some((cp, newprog.instrs.len(), sc)), sc))
        }));
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
    let rf = match catch_unwind(AssertUnwindSafe(|| Reference::new(&prog, mc, size))) {
        Ok(r) => r,
        Err(_) => return Ok(text.to_string()),
    };
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
    let v0 = if size > BIG { Variant { peel: 0, ifc: 0, ..Variant::default() } } else { Variant::default() };
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

    if size <= BIG {
        let over_k = |s: &Search, v: &Variant| -> bool {
            match s.mids.get(v) {
                Some(Some(f0)) => f0.nreg > mc.registers,
                _ => false,
            }
        };
        if over_k(&s, &state.0) {
            let v = Variant { spill: true, ..state.0 };
            consider(&mut s, &mut state, v);
        }
        let v = Variant { peel: 0, unroll: 1, ifc: 0, sched: false, spill: state.0.spill, mac: true };
        consider(&mut s, &mut state, v);
        for u in [2, 4, 8] {
            let v = Variant { unroll: u, ..state.0 };
            consider(&mut s, &mut state, v);
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
        for be in BACKENDS[1..].iter() {
            if s.spent > WORK_BUDGET {
                break;
            }
            let v = state.0;
            s.run(&v, *be);
        }
    }
    let mut order: Vec<usize> = (0..s.results.len()).collect();
    order.sort_by(|&a, &b| {
        let (x, y) = (&s.results[a], &s.results[b]);
        x.score.partial_cmp(&y.score).unwrap().then(x.size.cmp(&y.size)).then(x.order.cmp(&y.order))
    });
    for i in order {
        let c = &s.results[i];
        let ok = catch_unwind(AssertUnwindSafe(|| rf.same(&c.cp))).unwrap_or(false);
        if ok {
            return Ok(c.text.clone());
        }
        eprintln!("kopt: candidate {} rejected by the self-check", c.desc);
    }
    eprintln!("kopt: writing the input unchanged");
    Ok(text.to_string())
}
