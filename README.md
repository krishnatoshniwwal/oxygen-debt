# kopt - an optimizer for the Kiln (Oxygen Debt, compilers track)

`kopt` reads a KIR program and a machine file and writes a KIR program that does exactly
the same thing in fewer cycles.

## Build

Nothing to build. `kopt` is a Python 3 program (Python 3.8 or newer, standard library only).

```
./build.sh
```

## Run

```
./kopt IN.kir --machine MACHINE.cfg [--profile PROFILE.prof] -o OUT.kir
```

It works from any folder. On Windows: `python kopt IN.kir --machine MACHINE.cfg -o OUT.kir`.

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
| `kopt` | The program the judges run. A small launcher for the code in `src/`. |
| `build.sh` | Does nothing (no build step). |
| `DESIGN.md` | How the optimizer works. |
| `src/driver.py` | The pipeline, and the search over the optimizer's settings. |
| `src/ir.py` | The in-memory form of a program, control-flow graph, dominators, loops. |
| `src/ssa.py` | Into SSA form and back; register colouring; spilling to scratch memory. |
| `src/passes.py` | Simplification, constant folding, value numbering, dead code, CFG clean-up. |
| `src/ranges.py` | Value-range analysis (proves "cannot trap" and "not negative"). |
| `src/passes2.py` | Range-based rewrites, memory optimizations, loop-invariant code motion, fused instructions. |
| `src/passes3.py` | If-conversion, loop peeling, instruction scheduling. |
| `src/backend.py` | Final clean-up, block order, code copying (loop rotation and unrolling). |
| `src/evalsim.py` | kopt's own measurement and self-check of candidate outputs. |
| `src/kirlib.py` | A copy of the starter kit's `kirlib.py` (parser, interpreter, timing model). |
| `tests/run_bench.py` | Runs kopt on every practice program and machine, checks and measures. |
| `tests/run_cli.py` | The same through the command line, with the kit's `kvalidate`, run twice. |
| `tests/fuzz.py` | Random-program stress test. |

## Tests

The tests need the starter kit in a folder called `kit/` next to this file (it is not
part of this repository), or pass `--kit PATH`.

```
python3 tests/run_bench.py            # all practice programs, all machines
python3 tests/run_cli.py              # through the command line, with kvalidate
python3 tests/fuzz.py --n 300         # random programs on random machines
```
