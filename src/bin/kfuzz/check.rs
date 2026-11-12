//! check.rs - run kopt as a subprocess and compare the original with the optimized program
//! on many inputs, the way the judges' kvalidate does (and more).
use crate::kir::*;
use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub struct KoptRun {
    pub out: Option<String>,
    pub stderr: String,
    pub secs: f64,
    pub timed_out: bool,
    pub exit_ok: bool,
}

/// Run `kopt IN --machine CFG [--profile P] -o OUT` with a wall-clock limit.
pub fn run_kopt(kopt: &Path, dir: &Path, tag: &str, text: &str, mc: &Machine, prof: Option<&str>, limit: f64) -> KoptRun {
    let inp = dir.join(format!("{}.kir", tag));
    let cfg = dir.join(format!("{}.cfg", tag));
    let outp = dir.join(format!("{}.out", tag));
    let _ = std::fs::remove_file(&outp);
    std::fs::write(&inp, text).unwrap();
    std::fs::write(&cfg, crate::gen::to_cfg(mc)).unwrap();
    let mut cmd = Command::new(kopt);
    cmd.arg(&inp).arg("--machine").arg(&cfg).arg("-o").arg(&outp);
    if let Some(p) = prof {
        let pp = dir.join(format!("{}.prof", tag));
        std::fs::write(&pp, p).unwrap();
        cmd.arg("--profile").arg(&pp);
    }
    cmd.env("KOPT_DEBUG", "1").stdout(Stdio::null()).stderr(Stdio::piped());
    let t0 = Instant::now();
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return KoptRun { out: None, stderr: format!("spawn failed: {}", e), secs: 0.0, timed_out: false, exit_ok: false },
    };
    let mut err = child.stderr.take().unwrap();
    let h = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = err.read_to_string(&mut s);
        s
    });
    let mut timed_out = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break Some(st),
            Ok(None) => {
                if t0.elapsed().as_secs_f64() > limit {
                    let _ = child.kill();
                    let _ = child.wait();
                    timed_out = true;
                    break None;
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            Err(_) => break None,
        }
    };
    let secs = t0.elapsed().as_secs_f64();
    let stderr = h.join().unwrap_or_default();
    let exit_ok = status.map_or(false, |s| s.success());
    let out = std::fs::read_to_string(&outp).ok();
    let _ = std::fs::remove_file(&inp);
    let _ = std::fs::remove_file(&cfg);
    let _ = std::fs::remove_file(&outp);
    KoptRun { out, stderr, secs, timed_out, exit_ok }
}

// ---------------------------------------------------------------------------
// Inputs: a mirror of the kit generator plus program-specific ones
// ---------------------------------------------------------------------------
fn harvest_literals(prog: &Program) -> Vec<u32> {
    let mut set = std::collections::BTreeSet::new();
    for ins in &prog.instrs {
        for a in &ins.args {
            if let Arg::I(v) = a {
                let v = *v as u32;
                set.insert(v);
                set.insert(v.wrapping_add(1));
                set.insert(v.wrapping_sub(1));
                set.insert(v.wrapping_neg());
            }
        }
    }
    set.into_iter().collect()
}

pub fn make_inputs(prog: &Program, seed: u64, extra: usize) -> Vec<Inputs> {
    let np = prog.params.len();
    let mut v = generate_inputs(np, 20 + 16 + 24, seed);
    let mut rng = Rng::new(seed.wrapping_mul(0x9E37_79B9).wrapping_add(17));
    let lits = harvest_literals(prog);
    // literals +-1 as arguments (one position at a time)
    if np > 0 && !lits.is_empty() {
        for _ in 0..extra {
            let mut base = v[rng.below(v.len() as u64) as usize].clone();
            let k = rng.below(np as u64) as usize;
            base.args[k] = lits[rng.below(lits.len() as u64) as usize];
            if np > 1 && rng.below(3) == 0 {
                let k2 = rng.below(np as u64) as usize;
                base.args[k2] = lits[rng.below(lits.len() as u64) as usize];
            }
            v.push(base);
        }
    }
    // large memory contents of several shapes
    for m in 0..(extra / 2).max(4) {
        let mut inp = v[rng.below(v.len() as u64) as usize].clone();
        let n = [3072usize, 4096, 1024, 600][m % 4];
        let words: Vec<u32> = (0..n)
            .map(|_| match rng.below(6) {
                0 => rng.next() as u32,
                1 => rng.below(8) as u32,
                2 => rng.below(64) as u32,
                3 => lits.get(rng.below(lits.len().max(1) as u64) as usize).copied().unwrap_or(0),
                4 => rng.below(4096) as u32,
                _ => rng.below(300) as u32,
            })
            .collect();
        inp.segments = vec![(0, words)];
        if rng.below(3) == 0 {
            inp.vin = BTreeMap::new();
        } else {
            for p in 0..6u32 {
                let len = rng.below(40) as usize;
                inp.vin.insert(p, (0..len).map(|_| if rng.below(4) == 0 { rng.next() as u32 } else { rng.below(1000) as u32 }).collect());
            }
        }
        v.push(inp);
    }
    // zero memory, all-ones memory
    let mut z = v[0].clone();
    z.segments = vec![];
    z.vin = BTreeMap::new();
    v.push(z);
    let mut o = v[1].clone();
    o.segments = vec![(0, vec![0xFFFF_FFFF; 4096])];
    v.push(o);
    v
}

pub struct Cmp {
    pub mismatch: Option<String>,
    pub ran: usize,
    pub orig_cycles: f64,
    pub opt_cycles: f64,
    pub worse_inputs: usize,
    /// orig outcomes: ret, div0, badaddr, explicit trap, timeout
    pub outcomes: [usize; 5],
}

pub fn outcome_str(o: &ExecResult) -> String {
    format!("{:?} vlog={} entries", o.outcome, o.vlog.len())
}

/// Compare two program texts on the given inputs. The original's timeouts are skipped.
pub fn compare(orig: &Program, opt: &Program, mc: &Machine, inputs: &[Inputs], cap: u64) -> Cmp {
    let (co, cn) = (compile_program(orig), compile_program(opt));
    let mut r = Cmp { mismatch: None, ran: 0, orig_cycles: 0.0, opt_cycles: 0.0, worse_inputs: 0, outcomes: [0; 5] };
    for (k, inp) in inputs.iter().enumerate() {
        let ro = execute(&co, inp, cap, true);
        let oi = match ro.outcome { Outcome::Ret(_) => 0, Outcome::DivZero => 1, Outcome::BadAddr => 2, Outcome::Explicit(_) => 3, Outcome::Timeout => 4 };
        r.outcomes[oi] += 1;
        if ro.outcome == Outcome::Timeout {
            continue;
        }
        let rn = execute(&cn, inp, 40 * ro.steps + 10_000, true);
        r.ran += 1;
        if rn.outcome == Outcome::Timeout || differs(&ro, &rn) {
            let mut why = format!("input #{} args {:?}: orig {} / opt {}", k, inp.args, outcome_str(&ro), outcome_str(&rn));
            if ro.vlog != rn.vlog {
                why.push_str(" (volatile log differs)");
            }
            if ro.mem[..SCRATCH_BASE] != rn.mem[..SCRATCH_BASE] {
                let a = (0..SCRATCH_BASE).find(|&i| ro.mem[i] != rn.mem[i]).unwrap();
                why.push_str(&format!(" (memory differs at {}: {} vs {})", a, ro.mem[a], rn.mem[a]));
            }
            r.mismatch = Some(why);
            return r;
        }
        let (c0, c1) = (time_trace(&co, &ro.trace, mc), time_trace(&cn, &rn.trace, mc));
        r.orig_cycles += c0 as f64;
        r.opt_cycles += c1 as f64;
        if c1 > c0 {
            r.worse_inputs += 1;
        }
    }
    r
}

pub fn tmp_dir(name: &str) -> PathBuf {
    let d = PathBuf::from("out").join("kfuzz").join(name);
    std::fs::create_dir_all(&d).unwrap();
    d
}
