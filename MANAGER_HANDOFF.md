# MANAGER HANDOFF (2026-11-13) - read this first, then HANDOFF.md, DESIGN.md

kopt = Rust optimizer for KIR programs (std only). Judged by Success Rate and Average Cycles, combined as a
weighted mean. A failed case (wrong output, crash, timeout) is scored as the UNOPTIMIZED cycle count AND lowers
Success Rate, so correctness beats cycles. Output must be byte-identical on a second run (no clock decisions).
Organisers confirmed: idiom recognition from dataflow is allowed; CRC/bit-loop summarisation is allowed.
Still unanswered: may judge inputs set words 3072-4095 (scratch region)? how many hidden ~100k-instruction
programs? are cycles averaged over several hidden inputs / machines weighted equally?

## Where everything is (all public)
* Integrated code: https://github.com/krishnatoshniwwal/oxygen-debt  main @ 70e94cc  (= everything below).
* The same tree plus this file: https://github.com/ariv-IIITB/oxygen-debt  branch `manager-handoff`.
* Pieces, if you want them separately (ariv-IIITB/oxygen-debt): `item7-split-checks` (39adab4),
  `large-determinism` (768eb1e, contains item7). main there is still a67f0ef (the old baseline).
* kit/ is NOT in the repo (rules). Restore locally: `git archive a32c393~1 kit | tar -x` (it is git-ignored; never commit).

## State and numbers (sample cycles, 100 programs x 5 machines; original 580,799)
| Step | Total | Verified by |
|---|---|---|
| Start of this session (HANDOFF.md) | 70,525 | re-measured |
| + split_checks (bounds-check index-set splitting, loopopt.rs) | 69,618 (-1.29%) | measure.py, 29 tests, kfuzz gen/mutate/machines 1,100 cases clean |
| + deterministic large-program search (driver.rs/ranges.rs/large.rs) | 69,618 (unchanged) | 500 cases identical; large programs identical except large_loops +72 cycles |
| + F1 CRC nibble table (src/crc.rs), F5 wrap fix (main @ 70e94cc) | 69,066 (-0.8%) | manager session run: 34 tests pass, fails=0, 8 better / 0 worse / 0 slower than original |
Not yet fuzzed: main @ 70e94cc has had NO kfuzz run (task "soak" below).

## What was done
1. split_checks: front loop without a trap-guarded bounds check runs while a1 < min(b, K-delta); original loop
   finishes. bounds_trap -40% on 4 machines. The F5 proof found a real wrap bug (delta<0, a1 wraps while a2 does not);
   fixed in 70e94cc (guard now includes the d1/d2 offset). Read DESIGN.md for the argument + exhaustive test.
2. Large programs (>6000 instrs) no longer use the wall clock: a candidate starts only if spent + previous
   candidate's cost <= LARGE_BUDGET (2.4M work units; interval-analysis visits counted via ranges::take_work).
   Only the 25 s hard deadline remains, as last-resort fallback. Output byte-identical to unloaded at 2x CPU slowdown
   for all ten generated large programs, and at 3x for nine; large_ladder takes 25.1 s at 3x and falls back (output differs).
3. F1 crc.rs (GF(2) nibble table), F4 tests/ruler.py + tests/ruler_report.md, F5 proof+exhaustive test, F9 tests/scan_unswitch.py
   (says loop unswitching is not worth building), F8 fresh-clone check (reported pass, re-run it on the final commit).
4. Tried and REJECTED (do not repeat): pruning rerolling modes whose first candidate is >15% worse - it made
   large_affine 155,542->196,868, large_ladder 85,644->104,442, large_poly 105,444->115,623 cycles. 10x WORK_BUDGET: 0 cases change.

## Remaining, ranked (the seven task prompts are in the pasted chat message; names below match them)
1. soak: kfuzz gen/mutate/machines, seeds 1-5 x 400, on main @ 70e94cc. Do first; it gates everything.
2. idioms (new src/idiom.rs): popcount, bit_reverse, Kernighan popcount (hamming_dist), parity. ~1.1-1.6% of total.
3. large-tests: tests/determinism_large.py + new large families in tests/gen_large.py (diamonds, irreducible, pressure, many small loops).
4. ladder: cut large_ladder's cost (6.7 s) so it is deterministic at 3x. Pure speedups only.
5. chain: large_chain (100,001 instrs, 206,527 cycles unchanged): read tests/ruler_report.md verdict first; if Q-bound do linear-time
   clean-up / ring folding. Nobody has read the ruler verdict yet.
6. fewreg: bounds_trap on machine_few_registers (549 cycles, split gives nothing there), up to +0.35%.
7. machines: tests/machine_sweep.py extreme-machine sweep (K=4, high spill penalty, tiny/huge icache).
Low value, only if idle: divisibility test x%c==0 (algebra.rs), latch duplication (backend.rs tail_dup, <=0.27%).
Needs a human: send the three unanswered organiser questions above.

## Merge protocol
Each task pushes a branch `task-<name>`. Merge one at a time into an integration branch; after EACH merge run
cargo test, tests/measure.py vs the previous json (compare_runs.py: fails=0, nothing worse, nothing slower than original),
a kfuzz gen/mutate/machines run, and tests/measure_large.py on the ten large programs. idioms and ladder both edit driver.rs:
resolve by hand. Final commit: `kit/` untracked, `./build.sh` works from a fresh clone, run kopt twice and diff.
Never push to main without the full check list; the judges use the last commit before the deadline.

## Gotchas
* kfuzz "nondeterministic" under parallel load is usually the 2.6 s hard-deadline fallback; rerun the single case quietly before counting it.
* Variants are searched greedily; new settings go in as extra candidates AFTER the walk (HANDOFF.md section 4 lesson), not as walk neighbours.
* This repo is a single-branch clone in some sessions; a "has unpushed commits" hook warning there is a missing local tracking ref, not lost work.
