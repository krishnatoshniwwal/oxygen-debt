//! kfuzz - offline test tool for kopt (not needed at run time).
//!
//!   kfuzz gen      random programs x random machines
//!   kfuzz bench    the kit's bench programs x machines
//!   kfuzz machines the kit's bench programs x random machines
//!   kfuzz mutate   judge simulator: mutated bench programs
//!   kfuzz min      minimise a failing case
//!   kfuzz show     print the program / machine of a seed
//!
//! Build: cargo build --release --offline --features tools --bin kfuzz
#![allow(dead_code)]
#[path = "../../kir.rs"]
mod kir;
#[path = "../../vgraph.rs"]
mod vgraph;
mod check;
mod gen;
mod minimize;
mod mutate;
mod tp;

use kir::*;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

pub struct Ctx {
    pub kopt: PathBuf,
    pub ninputs: usize,
    pub minimize: bool,
    pub slow: f64,
    pub fail_dir: PathBuf,
    pub det_every: usize,
    pub validator: bool,
}

#[derive(Clone, Debug)]
pub struct Problem {
    pub kind: String,
    pub msg: String,
}

#[derive(Default, Clone)]
pub struct Stats {
    pub secs: f64,
    pub orig_cycles: f64,
    pub opt_cycles: f64,
    pub changed: bool,
    pub ran: usize,
    pub outcomes: [usize; 5],
    /// 0 = not applicable, 1 = unproved, 2 = proved by the value-graph validator
    pub proof: u8,
}

fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or("").chars().take(160).collect()
}

/// Run kopt on (text, mc) and check the result. `reference` = a second program the output must
/// also match (for mutations that keep the semantics). `extra` = additional inputs.
pub fn eval(ctx: &Ctx, tag: &str, text: &str, mc: &Machine, prof: Option<&str>, seed: u64, reference: Option<&str>, extra: &[Inputs], det: bool) -> (Vec<Problem>, Stats) {
    let mut probs: Vec<Problem> = Vec::new();
    let mut st = Stats::default();
    let dir = check::tmp_dir(&format!("t_{:?}", std::thread::current().id()).replace(['(', ')'], ""));
    let run = check::run_kopt(&ctx.kopt, &dir, tag, text, mc, prof, 20.0);
    st.secs = run.secs;
    for line in run.stderr.lines() {
        if line.contains("rejected by the self-check") {
            probs.push(Problem { kind: "caught-selfcheck".into(), msg: first_line(line) });
        } else if line.contains("behaves differently on a timing input") {
            probs.push(Problem { kind: "caught-timing".into(), msg: first_line(line) });
        } else if line.contains("failed (") && line.starts_with("kopt:") {
            if !line.contains("behaves differently") {
                probs.push(Problem { kind: "pass-failed".into(), msg: first_line(line) });
            }
        } else if line.contains("internal error") {
            probs.push(Problem { kind: "internal".into(), msg: first_line(line) });
        }
    }
    if run.timed_out {
        probs.push(Problem { kind: "timeout".into(), msg: format!("kopt exceeded 20 s") });
        return (probs, st);
    }
    if !run.exit_ok || run.out.is_none() {
        probs.push(Problem { kind: "crash".into(), msg: format!("exit not ok / no output: {}", first_line(&run.stderr)) });
        return (probs, st);
    }
    if run.secs > ctx.slow {
        probs.push(Problem { kind: "slow".into(), msg: format!("{:.2} s", run.secs) });
    }
    let out = run.out.unwrap();
    st.changed = out != text;
    let orig = match parse_program(text) {
        Ok(p) => p,
        Err(e) => {
            probs.push(Problem { kind: "badinput".into(), msg: e });
            return (probs, st);
        }
    };
    let newp = match parse_program(&out) {
        Ok(p) => p,
        Err(e) => {
            probs.push(Problem { kind: "invalid".into(), msg: e });
            return (probs, st);
        }
    };
    if let Err(e) = check_machine_features(&newp, mc) {
        probs.push(Problem { kind: "invalid".into(), msg: e });
    }
    if newp.params.len() != orig.params.len() {
        probs.push(Problem { kind: "invalid".into(), msg: "parameter count changed".into() });
        return (probs, st);
    }
    let mut inputs = check::make_inputs(&orig, seed, ctx.ninputs);
    inputs.extend(extra.iter().cloned());
    let c = check::compare(&orig, &newp, mc, &inputs, 200_000);
    st.ran = c.ran;
    st.outcomes = c.outcomes;
    st.orig_cycles = c.orig_cycles;
    st.opt_cycles = c.opt_cycles;
    let verdict = if ctx.validator { Some(std::panic::catch_unwind(|| vgraph::prove(&orig, &newp))) } else { None };
    if let Some(v) = &verdict {
        match v {
            Ok(vgraph::Verdict::Proved) => st.proof = 2,
            Ok(vgraph::Verdict::Unproved(_)) => st.proof = 1,
            Ok(vgraph::Verdict::NotApplicable(_)) => st.proof = 0,
            Err(_) => probs.push(Problem { kind: "validator-panic".into(), msg: "vgraph::prove panicked".into() }),
        }
    }
    if let Some(m) = c.mismatch {
        if st.proof == 2 {
            probs.push(Problem { kind: "VALIDATOR-UNSOUND".into(), msg: "proved equal but runs differ".into() });
        }
        probs.push(Problem { kind: "mismatch".into(), msg: m });
    } else if c.opt_cycles > c.orig_cycles * 1.0005 {
        probs.push(Problem { kind: "regress".into(), msg: format!("{:.0} -> {:.0} cycles ({} of {} inputs worse)", c.orig_cycles, c.opt_cycles, c.worse_inputs, c.ran) });
    }
    if let Some(rt) = reference {
        if let Ok(rp) = parse_program(rt) {
            if rp.params.len() == newp.params.len() {
                let ri = check::make_inputs(&rp, seed ^ 77, ctx.ninputs);
                let c2 = check::compare(&rp, &newp, mc, &ri, 200_000);
                if let Some(m) = c2.mismatch {
                    probs.push(Problem { kind: "mismatch-vs-original".into(), msg: m });
                }
            }
        }
    }
    if det {
        let run2 = check::run_kopt(&ctx.kopt, &dir, &format!("{}d", tag), text, mc, prof, 20.0);
        if run2.out.as_deref() != Some(out.as_str()) {
            probs.push(Problem { kind: "nondeterministic".into(), msg: "two runs differ".into() });
        }
    }
    (probs, st)
}

pub fn is_serious(kind: &str) -> bool {
    !matches!(kind, "regress" | "slow")
}

fn save_failure(ctx: &Ctx, tag: &str, text: &str, mc: &Machine, prof: Option<&str>, probs: &[Problem], seed: u64) {
    let _ = std::fs::create_dir_all(&ctx.fail_dir);
    let base = ctx.fail_dir.join(tag);
    let _ = std::fs::write(base.with_extension("kir"), text);
    let _ = std::fs::write(base.with_extension("cfg"), gen::to_cfg(mc));
    if let Some(p) = prof {
        let _ = std::fs::write(base.with_extension("prof"), p);
    }
    let desc: Vec<String> = probs.iter().map(|p| format!("{}: {}", p.kind, p.msg)).collect();
    let _ = std::fs::write(base.with_extension("txt"), format!("seed {}\n{}\n", seed, desc.join("\n")));
    // minimise on the first serious kind
    if ctx.minimize {
        if let Some(p) = probs.iter().find(|p| is_serious(&p.kind)) {
            let kind = p.kind.clone();
            let mc2 = mc.clone();
            let pred = |t: &str| -> bool {
                let (pr, _) = eval(ctx, "min", t, &mc2, None, seed, None, &[], false);
                pr.iter().any(|q| q.kind == kind)
            };
            let small = minimize::minimize(text, &pred, 120.0);
            let _ = std::fs::write(base.with_extension("min.kir"), small);
        }
    }
}

struct Summary {
    counts: BTreeMap<String, usize>,
    examples: Vec<String>,
    cases: usize,
    secs_max: f64,
    changed: usize,
    orig_cycles: f64,
    opt_cycles: f64,
    outcomes: [usize; 5],
    proof: [usize; 3],
}

fn record(sum: &Mutex<Summary>, tag: &str, probs: &[Problem], st: &Stats) {
    let mut s = sum.lock().unwrap();
    s.cases += 1;
    if st.secs > s.secs_max {
        s.secs_max = st.secs;
    }
    if st.changed {
        s.changed += 1;
    }
    s.orig_cycles += st.orig_cycles;
    for k in 0..5 {
        s.outcomes[k] += st.outcomes[k];
    }
    s.proof[st.proof as usize] += 1;
    s.opt_cycles += st.opt_cycles;
    for p in probs {
        *s.counts.entry(p.kind.clone()).or_insert(0) += 1;
    }
    if probs.iter().any(|p| is_serious(&p.kind)) {
        let line = format!("{}: {}", tag, probs.iter().filter(|p| is_serious(&p.kind)).map(|p| format!("{} ({})", p.kind, p.msg)).collect::<Vec<_>>().join("; "));
        println!("FAIL {}", line);
        s.examples.push(line);
    } else if let Some(p) = probs.first() {
        if p.kind == "slow" {
            println!("SLOW {}: {}", tag, p.msg);
        }
    }
}

fn print_summary(sum: &Mutex<Summary>) -> i32 {
    let s = sum.lock().unwrap();
    println!("---");
    println!("cases {}  changed by kopt {}  slowest kopt run {:.2}s", s.cases, s.changed, s.secs_max);
    println!("cycles on test inputs: original {:.0}  optimized {:.0}  ratio {:.4}", s.orig_cycles, s.opt_cycles, if s.orig_cycles > 0.0 { s.opt_cycles / s.orig_cycles } else { 1.0 });
    println!("original outcomes over all inputs [ret, div0, badaddr, trap, timeout]: {:?}", s.outcomes);
    println!("value-graph validator: not applicable {}, unproved {}, proved {}", s.proof[0], s.proof[1], s.proof[2]);
    println!("problem counts: {:?}", s.counts);
    let serious = s.counts.iter().filter(|(k, _)| is_serious(k)).map(|(_, v)| *v).sum::<usize>();
    if serious > 0 {
        1
    } else {
        0
    }
}

fn new_summary() -> Mutex<Summary> {
    Mutex::new(Summary { counts: BTreeMap::new(), examples: vec![], cases: 0, secs_max: 0.0, changed: 0, orig_cycles: 0.0, opt_cycles: 0.0, outcomes: [0; 5], proof: [0; 3] })
}

fn parallel<F: Fn(usize) + Sync>(n: usize, jobs: usize, f: F) {
    let next = AtomicUsize::new(0);
    std::thread::scope(|sc| {
        for _ in 0..jobs {
            sc.spawn(|| {
                let b = std::thread::Builder::new();
                let _ = b;
                loop {
                    let i = next.fetch_add(1, Ordering::SeqCst);
                    if i >= n {
                        break;
                    }
                    f(i);
                }
            });
        }
    });
}

struct Args {
    cmd: String,
    kv: BTreeMap<String, String>,
    pos: Vec<String>,
}

fn parse_args() -> Args {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let cmd = argv.get(0).cloned().unwrap_or_default();
    let mut kv = BTreeMap::new();
    let mut pos = Vec::new();
    let mut i = 1;
    while i < argv.len() {
        if let Some(k) = argv[i].strip_prefix("--") {
            if i + 1 < argv.len() && !argv[i + 1].starts_with("--") {
                kv.insert(k.to_string(), argv[i + 1].clone());
                i += 2;
            } else {
                kv.insert(k.to_string(), "1".to_string());
                i += 1;
            }
        } else {
            pos.push(argv[i].clone());
            i += 1;
        }
    }
    Args { cmd, kv, pos }
}

impl Args {
    fn num<T: std::str::FromStr>(&self, k: &str, d: T) -> T {
        self.kv.get(k).and_then(|v| v.parse().ok()).unwrap_or(d)
    }
}

pub fn parse_in(text: &str, nparams: usize) -> Inputs {
    let mut inp = Inputs::default();
    for raw in text.lines() {
        let line = raw.split('#').next().unwrap().trim();
        let toks: Vec<&str> = line.split_whitespace().collect();
        if toks.is_empty() {
            continue;
        }
        let nums: Vec<u32> = toks[1..].iter().filter_map(|t| parse_literal_raw(t).ok()).map(|v| (v & MASK) as u32).collect();
        match toks[0] {
            "args" => inp.args = nums,
            "mem" if !nums.is_empty() => inp.segments.push((nums[0] as usize, nums[1..].to_vec())),
            "vin" if !nums.is_empty() => inp.vin.entry(nums[0]).or_default().extend(nums[1..].iter().copied()),
            _ => {}
        }
    }
    inp.args.resize(nparams, 0);
    inp
}

fn default_kopt() -> PathBuf {
    let me = std::env::current_exe().unwrap();
    me.with_file_name("kopt_bin")
}

fn make_ctx(a: &Args, name: &str) -> Ctx {
    Ctx {
        kopt: a.kv.get("kopt").map(PathBuf::from).unwrap_or_else(default_kopt),
        ninputs: a.num("inputs", 60usize),
        minimize: !a.kv.contains_key("nomin"),
        slow: a.num("slow", 4.0f64),
        fail_dir: PathBuf::from("out/kfuzz/fail").join(name),
        det_every: a.num("det", 10usize),
        validator: !a.kv.contains_key("novalid"),
    }
}

fn cfg_for(seed: u64, mc: &Machine, size: Option<usize>) -> gen::GenCfg {
    let mut r = Rng::new(seed ^ 0xDEAD_BEEF);
    let size = size.unwrap_or_else(|| match r.below(20) {
        0 => 300 + r.below(300) as usize,
        1..=4 => 5 + r.below(15) as usize,
        _ => 20 + r.below(100) as usize,
    });
    gen::GenCfg { fused_mac: mc.fused_mac == 1, fused_addsh: mc.fused_addsh == 1, size, pressure: r.below(4) == 0, traps: r.below(10) < 8, scratch: r.below(4) == 0 }
}

fn cmd_gen(a: &Args) -> i32 {
    let n = a.num("n", 200usize);
    let seed0 = a.num("seed", 1u64);
    let jobs = a.num("jobs", 8usize);
    let ctx = make_ctx(a, "gen");
    let size = a.kv.get("size").and_then(|v| v.parse().ok());
    let sum = new_summary();
    parallel(n, jobs, |i| {
        let seed = seed0 * 1_000_003 + i as u64;
        let mc = gen::gen_machine(seed);
        let cfg = cfg_for(seed, &mc, size);
        let text = gen::gen_program(seed, &cfg);
        if parse_program(&text).is_err() {
            println!("GENERATOR BUG seed {}: {}", seed, parse_program(&text).err().unwrap());
            return;
        }
        let tag = format!("g{}", seed);
        let (probs, st) = eval(&ctx, &tag, &text, &mc, None, seed, None, &[], ctx.det_every > 0 && i % ctx.det_every == 0);
        if probs.iter().any(|p| is_serious(&p.kind)) {
            save_failure(&ctx, &tag, &text, &mc, None, &probs, seed);
        }
        record(&sum, &tag, &probs, &st);
    });
    print_summary(&sum)
}

fn read_kit(a: &Args) -> (Vec<(String, String, Option<String>, Option<String>)>, Vec<(String, Machine)>) {
    let kit = PathBuf::from(a.kv.get("kit").cloned().unwrap_or("kit".into()));
    let mut progs = Vec::new();
    let mut files: Vec<_> = std::fs::read_dir(kit.join("bench")).unwrap().filter_map(|e| e.ok()).map(|e| e.path()).collect();
    files.sort();
    for p in files {
        if p.extension().map_or(false, |e| e == "kir") {
            let name = p.file_stem().unwrap().to_string_lossy().to_string();
            let text = read_text(p.to_str().unwrap()).unwrap();
            let inp = std::fs::read_to_string(p.with_extension("in")).ok();
            let prof = std::fs::read_to_string(p.with_extension("prof")).ok();
            progs.push((name, text, inp, prof));
        }
    }
    let mut ms = Vec::new();
    let mut mf: Vec<_> = std::fs::read_dir(kit.join("machines")).unwrap().filter_map(|e| e.ok()).map(|e| e.path()).collect();
    mf.sort();
    for p in mf {
        if p.extension().map_or(false, |e| e == "cfg") {
            let name = p.file_stem().unwrap().to_string_lossy().to_string();
            ms.push((name, load_machine(p.to_str().unwrap()).unwrap()));
        }
    }
    (progs, ms)
}

fn cmd_bench(a: &Args) -> i32 {
    let (progs, ms) = read_kit(a);
    let jobs = a.num("jobs", 8usize);
    let ctx = make_ctx(a, "bench");
    let only = a.kv.get("only").cloned();
    let tasks: Vec<(usize, usize)> = (0..progs.len()).flat_map(|p| (0..ms.len()).map(move |m| (p, m))).filter(|&(p, _)| only.as_ref().map_or(true, |o| progs[p].0.contains(o.as_str()))).collect();
    let sum = new_summary();
    let rows = Mutex::new(Vec::new());
    parallel(tasks.len(), jobs, |t| {
        let (p, m) = tasks[t];
        let (name, text, inp, prof) = &progs[p];
        let (mname, mc) = &ms[m];
        let np = parse_program(text).unwrap().params.len();
        let extra: Vec<Inputs> = inp.iter().map(|i| parse_in(i, np)).collect();
        let tag = format!("{}@{}", name, mname);
        let (probs, st) = eval(&ctx, &tag, text, mc, prof.as_deref(), 100 + t as u64, None, &extra, ctx.det_every > 0 && t % ctx.det_every == 0);
        if probs.iter().any(|p| is_serious(&p.kind)) {
            save_failure(&ctx, &format!("{}_{}", name, mname), text, mc, prof.as_deref(), &probs, 0);
        }
        rows.lock().unwrap().push(format!("{},{},{:.0},{:.0},{:.2}", name, mname, st.orig_cycles, st.opt_cycles, st.secs));
        record(&sum, &tag, &probs, &st);
    });
    let mut r = rows.into_inner().unwrap();
    r.sort();
    let _ = std::fs::create_dir_all("out/kfuzz");
    let _ = std::fs::write("out/kfuzz/bench_report.csv", format!("program,machine,orig_cycles,opt_cycles,kopt_secs\n{}\n", r.join("\n")));
    print_summary(&sum)
}

fn cmd_machines(a: &Args) -> i32 {
    let (progs, _) = read_kit(a);
    let n = a.num("n", 300usize);
    let seed0 = a.num("seed", 1u64);
    let jobs = a.num("jobs", 8usize);
    let ctx = make_ctx(a, "machines");
    let sum = new_summary();
    parallel(n, jobs, |i| {
        let seed = seed0 * 7_000_003 + i as u64;
        let mc = gen::gen_machine(seed);
        let (name, text, inp, prof) = &progs[(seed as usize) % progs.len()];
        let np = parse_program(text).unwrap().params.len();
        let extra: Vec<Inputs> = inp.iter().map(|i| parse_in(i, np)).collect();
        let tag = format!("{}@m{}", name, seed);
        let (probs, st) = eval(&ctx, &tag, text, &mc, prof.as_deref(), seed, None, &extra, false);
        if probs.iter().any(|p| is_serious(&p.kind)) {
            save_failure(&ctx, &tag.replace('@', "_"), text, &mc, prof.as_deref(), &probs, seed);
        }
        record(&sum, &tag, &probs, &st);
    });
    print_summary(&sum)
}

fn cmd_mutate(a: &Args) -> i32 {
    let (progs, ms) = read_kit(a);
    let per = a.num("per", 3usize);
    let seed0 = a.num("seed", 1u64);
    let jobs = a.num("jobs", 8usize);
    let ctx = make_ctx(a, "mutate");
    let sum = new_summary();
    let total = progs.len() * per;
    parallel(total, jobs, |t| {
        let (p, j) = (t / per, t % per);
        let (name, text, inp, prof) = &progs[p];
        let seed = seed0 * 31_000_003 + t as u64;
        let mut rng = Rng::new(seed);
        let mc = if rng.below(2) == 0 { ms[rng.below(ms.len() as u64) as usize].1.clone() } else { gen::gen_machine(seed) };
        let mut tpg = match tp::TProg::parse(text) {
            Ok(t) => t,
            Err(_) => return,
        };
        let np = tpg.params.len();
        let sample: Vec<Inputs> = {
            let mut v: Vec<Inputs> = inp.iter().map(|i| parse_in(i, np)).collect();
            v.extend(generate_inputs(np, 30, seed));
            v
        };
        let mut preserved = true;
        let mut names = Vec::new();
        for _ in 0..(1 + rng.below(3)) {
            let (nm, pres) = mutate::mutate_once(&mut tpg, &mut rng, &sample);
            preserved &= pres;
            names.push(nm);
        }
        let mtext = tpg.to_text();
        if parse_program(&mtext).is_err() {
            println!("MUTATOR BUG {} {:?}: {}", name, names, parse_program(&mtext).err().unwrap());
            return;
        }
        let tag = format!("{}_{}_{}", name, j, seed);
        let extra: Vec<Inputs> = if names.contains(&"consts") || names.contains(&"emi-insert") { vec![] } else { sample.clone() };
        let (probs, st) = eval(&ctx, &tag, &mtext, &mc, if preserved || names == ["shuffle"] { None } else { None }, seed, if preserved { Some(text.as_str()) } else { None }, &extra, false);
        let _ = prof;
        if probs.iter().any(|p| is_serious(&p.kind)) {
            save_failure(&ctx, &tag, &mtext, &mc, None, &probs, seed);
        }
        record(&sum, &format!("{} {:?}", tag, names), &probs, &st);
    });
    print_summary(&sum)
}

fn cmd_min(a: &Args) -> i32 {
    let file = &a.pos[0];
    let mcp = &a.pos[1];
    let text = read_text(file).unwrap();
    let mc = load_machine(mcp).unwrap();
    let ctx = make_ctx(a, "min");
    let (probs, _) = eval(&ctx, "min0", &text, &mc, None, 1, None, &[], false);
    let kind = match a.kv.get("kind") {
        Some(k) => k.clone(),
        None => match probs.iter().find(|p| is_serious(&p.kind)) {
            Some(p) => p.kind.clone(),
            None => {
                println!("no serious problem found: {:?}", probs);
                return 0;
            }
        },
    };
    let pred = |t: &str| -> bool {
        let (pr, _) = eval(&ctx, "min", t, &mc, None, 1, None, &[], false);
        pr.iter().any(|q| q.kind == kind)
    };
    let small = minimize::minimize(&text, &pred, a.num("secs", 300.0f64));
    println!("{}", small);
    0
}

fn cmd_diff(a: &Args) -> i32 {
    let (ta, tb) = (read_text(&a.pos[0]).unwrap(), read_text(&a.pos[1]).unwrap());
    let (pa, pb) = (parse_program(&ta).unwrap(), parse_program(&tb).unwrap());
    let mc = Machine { issue_width: 2, mem_ports: 1, registers: 8, reg_spill_penalty: 5, taken_branch_penalty: 2, icache_size: 64, icache_penalty: 2, lat_alu: 1, lat_mul: 3, lat_div: 12, lat_load: 4, lat_mac: 3, lat_addsh: 1, fused_mac: 1, fused_addsh: 1 };
    let mut ins = check::make_inputs(&pa, 5, 100);
    let mut rng = Rng::new(3);
    for _ in 0..400 {
        let mut i = ins[0].clone();
        for x in i.args.iter_mut() {
            *x = [rng.below(20) as u32, 2 + rng.below(2000) as u32, (rng.below(10) as u32 + 1).wrapping_neg(), rng.next() as u32][rng.below(4) as usize];
        }
        let bound = [256u64, 100, 16, 1000][rng.below(4) as usize];
        let words: Vec<u32> = (0..256).map(|_| rng.below(bound) as u32).collect();
        i.segments = vec![(0, words)];
        ins.push(i);
    }
    let (ca, cb) = (compile_program(&pa), compile_program(&pb));
    for (k, i) in ins.iter().enumerate() {
        let ra = execute(&ca, i, 200_000, false);
        let rb = execute(&cb, i, 4 * ra.steps + 1000, false);
        if ra.outcome != Outcome::Timeout && differs(&ra, &rb) {
            println!("input #{} args {:?}
  A: {:?} vlog {}
  B: {:?} vlog {}", k, i.args, ra.outcome, ra.vlog.len(), rb.outcome, rb.vlog.len());
            if let Some(x) = (0..SCRATCH_BASE).find(|&j| ra.mem[j] != rb.mem[j]) {
                println!("  mem[{}] A={} B={}", x, ra.mem[x], rb.mem[x]);
            }
            let _ = &mc;
            return 1;
        }
    }
    println!("no difference");
    0
}

fn cmd_show(a: &Args) -> i32 {
    let seed: u64 = a.num("seed", 1u64);
    let mc = gen::gen_machine(seed);
    let cfg = cfg_for(seed, &mc, a.kv.get("size").and_then(|v| v.parse().ok()));
    println!("{}", gen::gen_program(seed, &cfg));
    eprintln!("{}", gen::to_cfg(&mc));
    0
}

fn main() {
    let a = parse_args();
    let code = match a.cmd.as_str() {
        "gen" => cmd_gen(&a),
        "bench" => cmd_bench(&a),
        "machines" => cmd_machines(&a),
        "mutate" => cmd_mutate(&a),
        "min" => cmd_min(&a),
        "show" => cmd_show(&a),
        "diff" => cmd_diff(&a),
        _ => {
            eprintln!("usage: kfuzz gen|bench|machines|mutate|min|show [--n N] [--seed S] [--jobs J] [--kit DIR] [--kopt PATH] [--nomin] [--inputs N]");
            2
        }
    };
    std::process::exit(code);
}
