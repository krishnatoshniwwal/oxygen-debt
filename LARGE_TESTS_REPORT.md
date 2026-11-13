# Large-program tests report (task large-tests; src/ untouched)

Build: main-equivalent tree (manager-handoff), `cargo build --release --offline`, kit restored from a32c393~1.

## New files
* `tests/determinism_large.py KOPT PROGDIR [--only S] [--machine CFG] [--loads 1,2]`: per program 2 unloaded runs,
  then 2 runs pinned with `taskset -c 0` next to 1 and next to 2 busy loops; byte-compares with the unloaded output,
  prints identical/DIFFERENT, wall time and slowdown. Exit 1 on any difference, crash, or >30 s.
  (kopt needs `--machine`; default is kit/machines/machine_default.cfg.)
* `tests/gen_large.py`: 7 new families (each with .in, ~100k instrs): `diamonds` (depth-3 nested if/else),
  `irreducible` (two-entry cycles), `pressure` (48 simultaneously live values), `smallloops` (thousands of 1-3 trip
  loops with load/store), `deepnest` (loop nest of size/10 = 10,000 levels, each trip 1), `nestloops` (many 6-deep
  nests), `stld` (straight line, aliasing stores/loads). `--only NAME` works.

## Results (17 programs)
* measure_large `--c 64 --q 2` and `--c 32,256 --q 1,8`: **no MISMATCH, no crash, no INVALID**, all outputs valid.
* determinism_large (loads 1,2): **0 DIFFERENT, 0 crashes** for all 17 programs. Slowdowns measured were 2.0-2.6x with one
  hog and 2.9-4.6x with two (CPU share ideal is 2x/3x; more because of the two-hog scheduling).
* Slowest, unloaded: deepnest 25.1 s (see 1), irreducible 8.4 s, diamonds 5.3 s, ladder 5.0 s, crc 3.4 s.
  Under 2 hogs: irreducible 25.05 s (hit the hard fallback, output still identical), diamonds 21.8 s, ladder 19 s.

## Findings
### 1. large_deepnest: 25 s hard deadline fires even when unloaded (output = unchanged input, 0% gain)
Reproducer: `python3 tests/gen_large.py D --only deepnest` (depth 10,000), or smaller `--size 1200` (120 levels):
4.2 s with `KOPT_HARD_MS=100000`; `--size 2400` (240 levels) 11.8 s; sizes 5000-20000 all hit the 2.6 s deadline
(they are below the 40,000-line threshold so the limit is 2.6 s, not 25 s: even 600 lines/60 levels takes 1.5 s).
Time is roughly quadratic in nest depth. Sampling with gdb shows the time in
* `ssa::build_ssa` (per-block HashSet<i64> live sets, filter/collect per block: O(blocks x live values); in a nest every
  induction variable and the accumulator are live through all enclosing loops),
* `ssa::liveness` (called from `ssa::build_graph` <- `ssa::allocate`, same shape, once per candidate), and
* `layout::improve` / `layout::Model::eval`.
All candidate evaluations repeat this, so the LARGE_BUDGET work counter (which counts interval-analysis visits only) does
not see it. Determinism is "fine" only because every run falls back to the input; a depth around 60-120 on a normal
(<40k) program would depend on wall time: result differs between a fast and a slow machine (clock decision at 2.6 s).
Not a correctness failure. Probably irrelevant for judge programs unless they nest loops deeply.

### 2. large_irreducible is a near-miss on timing
7.8-8.4 s unloaded, 19.5 s at 2.5x, 25.05 s at 3.2x (two hogs): the hard fallback triggers there. Output was still identical
to the unloaded run (the last candidates did not improve), so no nondeterminism seen, but a 3.2x-slower judge would be one
step from a differing output. Same family: diamonds (4.2x slowdown -> 21 s) and ladder (3.9x -> 19.5 s) are fine but have <2x margin
at 4x. Responsible path: the per-candidate cost of `driver::Search::run_inner` (SSA allocate/liveness) on many blocks;
the LARGE_BUDGET (2.4M units) does not bound the allocator cost, only analysis visits.

### 3. Not failures, observations
* chain is unchanged (n_out = n_in, 206,527 cycles); deepnest unchanged (see 1).
* `large_affine` C=256 Q=8 gives 366,612 cycles vs 155,542 at C=64 Q=2 (different icache, n_out 154): different choice per
  machine, valid but note it is much worse than the C=32 Q=8 answer (155,542, 30 instrs) relative to its input 964,790.
* All new programs terminate within the 200,000-step validator limit.

Raw logs: measure at C64/Q2 and the C/Q grid showed status `ok` on all 17 + 68 rows; determinism script ended `problems: 0`.
