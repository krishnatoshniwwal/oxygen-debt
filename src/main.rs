#![allow(dead_code)]
mod backend;
mod driver;
mod evalsim;
mod ir;
mod large;
mod kir;
mod passes;
mod passes2;
mod passes3;
mod ranges;
mod reroll;
mod ssa;

use std::panic::{catch_unwind, AssertUnwindSafe};

const USAGE: &str = "usage: kopt IN.kir --machine MACHINE.cfg [--profile PROFILE.prof] -o OUT.kir\n";

fn run(argv: &[String]) -> i32 {
    let (mut inp, mut machine, mut profile, mut outp): (Option<&str>, Option<&str>, Option<&str>, Option<&str>) = (None, None, None, None);
    let mut i = 0;
    while i < argv.len() {
        let a = argv[i].as_str();
        match a {
            "--machine" | "--profile" | "-o" => {
                if i + 1 >= argv.len() {
                    eprint!("{}", USAGE);
                    return 2;
                }
                let v = argv[i + 1].as_str();
                match a {
                    "--machine" => machine = Some(v),
                    "--profile" => profile = Some(v),
                    _ => outp = Some(v),
                }
                i += 2;
            }
            _ if inp.is_none() && !a.starts_with('-') => {
                inp = Some(a);
                i += 1;
            }
            _ => {
                eprint!("{}", USAGE);
                return 2;
            }
        }
    }
    let (inp, machine, outp) = match (inp, machine, outp) {
        (Some(a), Some(b), Some(c)) => (a, b, c),
        _ => {
            eprint!("{}", USAGE);
            return 2;
        }
    };
    let text = match kir::read_text(inp) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("kopt: cannot read {}: {}", inp, e);
            return 2;
        }
    };
    // The pipeline runs on a worker thread; at the deadline the best verified candidate so far
    // (or the unchanged input) is written instead (see large.rs).
    let (text2, machine2, profile2) = (text.clone(), machine.to_string(), profile.map(|p| p.to_string()));
    let result = large::guarded(&text, move || -> Result<String, String> {
        let mc = kir::load_machine(&machine2)?;
        let ptext = profile2.and_then(|p| kir::read_text(&p).ok());
        driver::optimize(&text2, &mc, ptext.as_deref())
    });
    let out = match result {
        Ok(o) => o,
        Err(e) => {
            eprintln!("kopt: {}; writing the input unchanged", e);
            text
        }
    };
    if let Err(e) = std::fs::write(outp, out) {
        eprintln!("kopt: cannot write {}: {}", outp, e);
        return 1;
    }
    0
}

fn main() {
    large::start();
    if std::env::var("KOPT_DEBUG").is_err() { std::panic::set_hook(Box::new(|_| {})); }
    let argv: Vec<String> = std::env::args().skip(1).collect();
    std::process::exit(run(&argv));
}
