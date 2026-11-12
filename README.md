# kopt - an optimizer for the Kiln (Oxygen Debt, compilers track)

`kopt` reads a KIR program and a machine file and writes a KIR program that does exactly
the same thing in fewer cycles.

## Build

`kopt` is a Rust program (standard library only, no external crates, no network needed).
It needs a stable Rust toolchain (`cargo`). Build it once from the top folder:

```
./build.sh
```

This runs `cargo build --release` (about half a minute) and puts the program in
`target/release/`. The file `kopt` is a two-line launcher for it.

## Run

```
./kopt IN.kir --machine MACHINE.cfg [--profile PROFILE.prof] -o OUT.kir
```

It works from any folder.

* The exit status is 0 when `OUT.kir` was written.
* If `kopt` cannot improve a program, or anything goes wrong inside it, it writes the
  input program unchanged.
* The same command always writes the same file.

## Check a result (with the starter kit)

```
PATH-TO-KIT/kvalidate OUT.kir --against IN.kir --machine MACHINE.cfg
PATH-TO-KIT/ksim OUT.kir --machine MACHINE.cfg --input IN.in
```

## Files

| File | What it is |
|---|---|
| `kopt` | The program the judges run. A small launcher for the compiled binary. |
| `build.sh` | Builds the Rust program with `cargo build --release`. |
| `Cargo.toml`, `Cargo.lock` | The Rust project (no dependencies). |
| `DESIGN.md` | How the optimizer works. |
| `src/main.rs` | Command line; writes the input unchanged if anything goes wrong. |
| `src/driver.rs` | The pipeline, and the search over the optimizer's settings. |
| `src/kir.rs` | KIR parser, machine file reader, interpreter, timing model (a port of the kit's `kirlib.py`). |
| `src/ir.rs` | The in-memory form of a program, control-flow graph, dominators, loops. |
| `src/ssa.rs` | Into SSA form and back; register colouring; spilling to scratch memory. |
| `src/passes.rs` | Simplification, constant folding, value numbering, dead code, CFG clean-up. |
| `src/ranges.rs` | Value-range analysis (proves "cannot trap" and "not negative"). |
| `src/passes2.rs` | Range-based rewrites, memory optimizations, loop-invariant code motion, fused instructions. |
| `src/passes3.rs` | If-conversion, loop peeling, speculation inside loops, instruction scheduling. |
| `src/backend.rs` | Final clean-up, block order, code copying (loop rotation and unrolling). |
| `src/evalsim.rs` | kopt's own measurement and self-check of candidate outputs. |
| `tests/run_cli.py` | Runs `./kopt` on every practice program and machine with the kit's `kvalidate`, twice. |
| `tests/compare.py` | Compares the Rust output with the Python reference on every practice program (they no longer match exactly: the Rust version has moved on). |
| `tests/fuzz_cli.py` | Random-program stress test through the command line. |
| `tests/gen_large.py` | Generates ~100,000-instruction programs to check kopt's time limit on very large inputs. |
| `tests/measure.py`, `tests/compare_runs.py` | Cycle counts of a kopt binary on every practice program and machine (sample input plus 3 perturbed variants), and an A/B comparison of two such runs. |
| `pyref/` | The original Python implementation, kept as a reference for the tests. |

## Tests

The tests need the starter kit in a folder called `kit/` next to this file (it is not
part of this repository), or pass `--kit PATH`. They need Python 3.

```
python3 tests/run_cli.py              # command line + kvalidate, run twice (determinism)
python3 tests/compare.py              # Rust against the Python reference, all programs
python3 tests/fuzz_cli.py --n 500     # random programs on random machines
python3 tests/gen_large.py /tmp/large # then run ./kopt on /tmp/large/*.kir and time it
python3 tests/measure.py target/release/kopt_bin /tmp/a.json   # cycles; then after a change:
python3 tests/compare_runs.py /tmp/a.json /tmp/b.json         # A/B per machine and per case
```

`KOPT_PROF=1 ./kopt ...` prints how long each step took (the output file is the same).
