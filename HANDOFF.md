# HANDOFF — kopt optimizer (state: 2026-11-13, branch `main`; all first-round branches merged)

kopt reads a KIR program + a machine file and writes a faster KIR program (Rust, std only).
Read this file first, then `DESIGN.md` (how kopt works, one section per pass) and `FINAL_FINAL.md`
(the original master plan: Part 1 §1 machine laws, §4 technique verdicts, §7 build order).

## 0. Next steps (2026-11-13, after the pre-submission check)

**State:** the code is ready to submit. All items in section 5 are DONE or EVALUATED. Re-measured on `main` at `c0e021a` (WSL):
sample 70,525 cycles (original 580,799), 0 failures, slowest bench program 0.87 s. Biggest remaining shares of the optimized total:
`sieve` 6,987, `spill_storm` 4,652, `bubble_sort` 3,468, `collatz` 3,279, `bounds_trap` 2,628, `count_divisors` 2,562, `insertion_sort` 2,370.

Not built yet from `FINAL_FINAL.md` Part 1 §4, ranked by ROI. Each one is a new transformation worth about 0.5 % or less of the bench
total, so do them only if there is time before the deadline:

| # | Item (FINAL_FINAL §) | Target | Expected | Effort |
|---|---|---|---|---|
| 1 | Judge-speed safety margin (§4.9 robustness): run `tests/measure_large.py` on a throttled CPU (`taskset` + a busy loop on the same core) and lower the `large::soft_expired` budgets if any large program goes above ~12 s | hidden ~100k programs (each worth ~200 bench programs) | avoids losing whole cases | low |
| 2 | Loop versioning on trip count (§4.9 multiversioning): drop the `i < 64` bounds check when `n <= 64` | `bounds_trap` | ~0.5 % total (hand-measured -14 % on that program) | medium |
| 3 | GVN-PRE / lazy code motion for pure ops (§4.2) | `partial_redundancy` and hidden programs of that kind | unknown on the bench; generic | medium-high |
| 4 | Loop unswitching (§4.5, bounded) | loop-invariant branches in hidden programs | unknown | medium |
| 5 | Divisibility test `x % c == 0` with a low multiply (§4.4) | constant-divisor tests in hidden programs | 0 on the bench (its divisors are variables) | low |
| 6 | Latch duplication with rotated placement (§4.8) | `spill_storm` | ~4 % of that one program | medium |

Organiser-gated: GF(2) CRC tables and idiom recognition are NOT BUILT (no code in `src/`; DESIGN.md lists them under "Not done"). Build them only if the organisers confirm
they are allowed (`crc_bitwise` is 1,607 cycles; estimated gain 0.5-0.9 % of the bench total, most of a day of work). Before every push, run the checks in section 3; for the final commit, check again
that `kit/` is not tracked.

## 1. Where the work is

* **`main`** contains everything: `merge/scratch-peephole` (PR #9), PR #3, `midend` (sink) and `large-progs-2` are merged. Work from `main`.
* Everything below is committed and pushed to `origin/main`.
* `notes/experiment_prefetch_seeds.patch` is an unfinished experiment (see section 5.1), not applied.

Results on the 100 bench programs x 5 machines (sample input + 3 perturbed variants each), against `main` as
it was at the start of this session:

| | main before | this branch |
|---|---|---|
| sample cycles | 76,875 | **74,369** (-3.3 %) |
| variant cycles | 228,710 | 220,711 (-3.5 %) |
| failures | 0 | 0 |
| cases slower than the original | 0 | 0 |

Of the 500 cases, 48 are better and 1 is slightly worse (`spill_storm|default` 3,656 -> 3,784). The unoptimized
sample total is 580,799. Large (~100k instruction) programs: all ten generated ones finish in at most 6.2 s of
the 30 s limit and are verified; `large_nest` went from "returned unchanged" to 100,003 -> 71 instructions.

## 2. Setup

* Rust toolchain needed (std only, no crates, builds offline). On the Mac used here: `brew install rust`
  (cargo 1.99). Build: `./build.sh` (= `cargo build --release --offline`), binary `target/release/kopt_bin`,
  launcher `./kopt IN.kir --machine M.cfg [--profile P.prof] -o OUT.kir`.
* The original authors used WSL Ubuntu on Windows; same commands there.
* The starter kit lives in `kit/` (spec, `kirlib.py` = exact timing model = ground truth, bench programs, machine files). It was **removed from version control** in the final commit (rules: the kit must not be in the submitted repo) and is git-ignored: keep a local copy in `kit/` (copy it from the organisers or from the commit before "Remove the starter kit") because the tests need it. Never commit it again.
* macOS has no `timeout` command; use Python for timing (`tests/measure_large.py` already does).

## 3. How to check a change (always, before merging)

```
cargo build --release --offline && cargo test --release --offline        # 28 unit tests
python3 tests/measure.py target/release/kopt_bin new.json                 # ~30 s; needs kit/
python3 tests/compare_runs.py base.json new.json                          # need fails=0, nothing slower than original
cargo build --release --offline --features tools --bin kfuzz
./target/release/kfuzz gen|mutate|machines --n 400 --seed S --kopt target/release/kopt_bin --kit kit
python3 tests/gen_large.py OUTDIR && python3 tests/measure_large.py target/release/kopt_bin OUTDIR --c 64 --q 2
```

Keep a `base.json` from `main` to compare against. Reading kfuzz output:

* `mismatch` / `VALIDATOR-UNSOUND` = a real bug. None seen.
* `regress` = optimized program slower than the original on kfuzz's *own* random inputs. Not serious (kopt picks by
  its own timing inputs); 1-5 per 400 is normal on every commit tested.
* `caught-timing` = a candidate was rejected by kopt's step cap (4x the original's instructions + 1000). It is a
  *slow* spill-heavy candidate, **not** a miscompile (verified on 540 inputs for seed 63, case `g63000338`).
* `nondeterministic` appears rarely and does not reproduce: the search has wall-clock limits
  (`large::soft_expired`), so a loaded machine can stop it at a different point. Same on the old baseline.

Dev aids (environment variables): `KOPT_PROF=1` (timing of every step and one line per candidate with its score and
variant), `KOPT_DUMP_ALL=DIR` (every candidate as `cand_N.kir`), `KOPT_DUMP_BAD=FILE`, `KOPT_CHECK=1` (SSA validity after
each pass), `KOPT_NOPP=1` / `KOPT_NOPP2=1` (scratch peephole off), `KOPT_NOREUSE=1`, `KOPT_NOPEEL3=1`,
`KOPT_HOSTILE=n` (fan-in guard), `KOPT_FORCE_GU/ROT/ACC/SR`, `KOPT_LOOPOPT=mask`.

## 4. What was done on the branch (each has a section in `DESIGN.md`)

1. Merged the unmerged first-round branches that passed the checks: scratch peephole (`regs-sched`), `iv_divmod`
   (`algebra`; fires on no bench program but is correct and removes `div`/`mod` in its own test), node splitting
   (`layout-control`; fixes `irreducible_loop` on all machines).
2. **Dominator computation made linear for huge-fan-in joins** (`src/ir.rs`, `analyze`): it marks climbed nodes per
   join. `large_nest` 25 s -> 1.2 s, so the `too_hostile` guard (`src/large.rs`) is now 40,000 (was 4,000).
3. **Scratch peephole** runs again after sched2 reorders/renames, and sched2's search scores each candidate *after*
   the block-local peephole (`sched2::peep_block`).
4. **Reload cache** `Variant.reuse` (`ssa::rewrite_spills`): the last n loaded/stored scratch words stay available for
   later uses in the block. `spill_storm|default` 5,116 -> 3,656.
5. **`peel: 3`** (`passes3::peel_const_loops(.., nest=true)`): fully peel inner constant-trip loops while the whole
   enclosing loop nest stays within 2x the icache (sizes are counted before clean-up). Unroll-and-jam in effect:
   `matmul4|wide_fast` 1,352 -> 552. Tried together with the `spill`/`hot`/`sched` settings.
6. **`Variant.spillk`** (`ssa::allocate`): spill the k least-touched loop-carried values at once; tried with
   `reuse` and `sched: false`. `spill_storm|no_fused` 5,944 -> 4,764; `poly_mac|few_registers` 144 -> 80.

7. **`peel: 4` / `peel: 5` / `peel: 6`** (DONE; 6 added in loop iteration 3: 6x icache budget, sample 72,916 -> 72,514, 4 cases better (`transpose8|slow_memory` 2,052 -> 1,004, `matmul4|default`), 0 worse, fuzz clean; a bigger budget than 6x was not tried): same nest peeling as `peel: 3` but the enclosing nest may be 3x / 4x
   the icache (`driver.rs` budget match). Tried as extra candidates after the walk, next to `peel: 3`, so a case
   keeps its old result when the bigger budget is not better. Sample cycles 74,368 -> **72,916** (-2.0 %), variants
   220,706 -> 215,366; 12 cases better (`transpose8|few_registers` 2,916 -> 1,380, `matvec8`, `crc_bitwise`,
   `row_sums|no_fused`), 2 slightly worse (`spill_storm|few_registers` +120, `matrix_add4|no_fused` +84: kopt picks by
   its own timing inputs); failures 0, nothing slower than the original, 28 unit tests pass, kfuzz gen/mutate
   300 each clean. Why: the size estimate is taken before clean-up, so 8x8 nests (transpose8, matvec8) were just over
   the 2x budget although folding makes them much smaller.
**Lesson about the search (`src/driver.rs`):** it is a greedy walk over `Variant` settings with a work budget. Adding
a new setting to the walk's neighbour list changed its path and made other programs worse (`row_sums`). New settings
that only pay off in combination (peel 3 + spill, reuse, spillk) are therefore tried as **extra candidates after the
walk, from its final setting** (see the blocks guarded by `KOPT_NOPEEL3` / `KOPT_NOREUSE`). Follow that pattern.

## 5. Open items, best first

**Next steps ranked by ROI (re-sorted in loop iteration 11; all cheap knob-style items are used up, what is left needs real design time).**
Sample total now 72,514 cycles (from 74,369 at the start of the loop), 0 failures, nothing slower than the original.

| # | Item | Expected gain | Effort | Risk | State |
|---|---|---|---|---|---|
| 1 | Make single search candidates cheaper (5.0c) | none in cycles; keeps big programs under the time limit on a slower judge machine | medium | low | DONE for the shared prefix (`phase_a` cache, section 5.0d); incremental `ranges` not done (~15 %, skipped: risk) |
| 2 | Spill-load hoisting for `spill_storm` (5.1) | achieved -0.9 % total (spill_storm few_registers -37 %) | high | medium | DONE; leftovers (latch duplication budget, icache index) are small, see 5.1 |
| 3 | `sieve` / `bubble_sort` | achieved -1.8 % total (5.1d: signed-interval refinement enables rotation); sieve and if-conversion of the swap evaluated, no gain (5.1e) | high | medium | DONE |
| 4 | Plan items of 5.4 (latch duplication, cost-based if-conversion, multiversioning on trip count, relational range facts (first step done in 5.1d: signed intervals for `lt`)) | unknown | high | medium | EVALUATED (5.1f): each < 0.5 % total, deferred |
| 5 | Magic division / signed div by constants (`algebra.rs`) | 0 on the bench (no program needs it), unknown on the judges' hidden programs | medium | low | EVALUATED (5.1g): at the ISA limit, nothing to build |
| - | Final step: remove `kit/` from version control, re-run the fresh-clone check | required | trivial | - | DONE (loop run 6, 2026-11-13) |
Done and not worth repeating: peel 3-6 nest budgets (item 7 in section 4), determinism fix (5.0), WORK_BUDGET 10,000 (5.0b), extra unroll/ifc candidates (5.1c), `factorial_mod` magic mod (5.1b).

### 5.1 Hoist spill loads (`spill_storm` on few-register machines): stage 1 DONE (loop 3a009676 run 2), stage 2 open
**Result (DONE, loop 3a009676 runs 2-3).** `sched2.rs`: `prefetch_order` seeds (program order with each load moved up 1,2,3,4,6,8 places; `KOPT_NOPREFETCH=1` turns them off) in `opt_block`; dev aids
`KOPT_S2DEBUG=1`, `KOPT_S2FILE=cand.kir` (runs sched2 on one candidate file and prints the exact score of input and result; get candidates with `KOPT_DUMP_ALL=DIR`).
`driver.rs::post_sched2`: besides the 3 best and 2 best `spillk` candidates it re-schedules up to `KOPT_FREEPICKS` (8) candidates that leave a register free *in the largest block*
(`max_reg_in_big_block`; the whole-program maximum was wrong: the exit block mentions spilled registers beyond K, which hid the best candidate). Why it works: the prefetch seeds need a
third temporary name, and the all-spilled candidates have one; they score badly before sched2 (20,812) and well after (12,195 vs 15,624 for the old pick). `KOPT_PROF=1` prints every pick and its
sched2 score (`kopt-s2 ...`). Numbers: sample 72,514 -> 71,845 (-0.9 % over runs 2-3), `spill_storm|few_registers` n=63 4,660 -> 2,911 (original 21,344; hand-written version was 3,497),
10 cases better and 4 worse by <= 25 cycles in the last step, nothing slower than the original, fuzz clean, 28 unit tests, large programs unchanged.
**Left over (small, ~4 % on this one program):** latch duplication on top: copying the loop test into the end of the body gave sched2 score 11,749 vs 12,195 on the same candidate
(`backend::tail_dup` is limited by `budget = icache - size`, ~0 here); and instruction index >= `icache_size` costs +2 per execution (entry 17 stores + body + head ~ 70 > 64).
Tried and not kept: running the best free-register variants through the other `BACKENDS` (restricted to spill variants: no gain; unrestricted: overfits, `row_sums|no_fused` 498 -> 863).

### 5.1d Signed-interval refinement of `lt` in the range analysis (DONE, loop 3a009676 run 4, item 3 stage 1)
Why: the latency rotation (`loopopt::rotate_loops`) hoists the next iteration's load to the end of the loop, which needs the new address proven trap-free. In `bubble_sort` the inner loop is
`j = phi(0, j+1)` with exit test `j < lim`, `lim = (n-1) - i`. The unsigned interval of `lim` is useless (it can wrap), so `j` had range (0, 2^31-1) and `j+2` could not be proven in range.
`ranges.rs`: new `Ranges::sint(x, blk, depth)` = sound signed interval (sums/differences only when the exact result stays inside signed 32 bits; unsigned intervals that lie entirely below or
above 2^31 convert directly); `derive` uses its upper bound to cap `b` (and so `a`) on the true edge of `lt a, b` when `a` is non-negative. Now `j` is (0, 14) and rotation applies. `KOPT_NOSINT=1` turns it
off for A/B tests; `KOPT_RANGES_DUMP=1` prints the range of every value after the mid-end (dev aid).
Numbers (500 cases): sample 71,845 -> 70,525 (-1.8 %): 26 better, 12 worse. `bubble_sort|few_registers` 7,507 -> 4,320, `|default` 2,678 -> 2,268, `conv_mac|slow_memory` -22 %, `addsh_index`, `adler32`, `fletcher16`, `branchy_select`
on `slow_memory` all -15..-25 %. Worse by more than 30 cycles: `fnv_hash|default` 279 -> 361 (the greedy walk now commits to `rot: 1, gu: 4` at the first back end, whose tuned score is worse than the old `rot: 0, gu: 4` with back end (12,2); tried "run
all back ends on the best 2 runner-up variants": no gain overall and `row_sums` overfits again, so not kept), `array_sum|no_fused` +28, `fletcher16|no_fused` +56, `adler32|no_fused` +48. Fuzz: gen/mutate/machines seeds 111/222/333 x 500 clean, 28 unit tests, large programs unchanged.
### 5.1e Item 3 leftovers evaluated in loop run 4 (do not repeat)
* **If-converting the bubble-sort swap** (conditional stores -> `select` + two unconditional stores; hand-written form `bs_sel`): once rotation applies (5.1d) the original form is as good or better on 4 of 5 machines (sample cycles default 536 vs 542, slow_memory 918 vs 970, few_registers 959 vs 1,537, no_fused 648 vs 660) and better only on `wide_fast` (271 vs 421). A pass for it would need the old value of the stored word in a register (memfwd) and a per-machine choice; value ~0.1 %. Not built.
* **Guarded unrolling with a loop-invariant variable step** (`loopopt::unroll_guarded` generalised from a constant step `c` to an invariant `s`, front test `a < b - (u-1)*s`, only when the ranges prove `s` in [1,4096] and no overflow): implemented and verified to apply to the `sieve` inner loop (ranges: a (4,256), b (3,129), s (2,128)), but the sieve got slower (1,686 vs 1,187 cycles: most inner loops have fewer than 8 stores because the step is large, so the guard fails and the plain loop runs) and no other bench case changed (0 of 500). Reverted. If ever needed again: the 40-line change is `phi_var_step` + a `mul`/`sub` in the preheader for `mval`.
* **`sieve`** (9.9 % of all cycles): sample = 1,297 executed instructions in 1,187 cycles (IPC 1.1), the outer loop (load flag, test, count) and many short inner loops dominate; nothing cheap left. **`bubble_sort`** is now 4.9 % (ratio 0.20).
* `fnv_hash|default` regressed 279 -> 361 after 5.1d: the greedy walk takes `rot: 1, gu: 4` at the first back end; the old `rot: 0, gu: 4` with back end (12,2) was better. Running every back end on the runner-up variants did not fix it (and overfits `row_sums`). Open, small.
### 5.1f Item 4 (plan items of 5.4) evaluated in loop run 5, none worth building (do not repeat)
Method: `KOPT_ROT_DEBUG=1 KOPT_FORCE_ROT=1` on all 100 bench programs counted how often a hoisted load is rejected as "may trap" (programs with many rejections and no committed rotation: `insertion_sort`, `merge_sorted`, `reverse_array`, `vending_fsm`, `conv_mac`), then each cause was checked on the real program.
* **Relational / signed range facts (more of 5.1d):** a monotone-counter rule in `ranges::sint` (`phi(init, phi +- c)`, |c| <= 16, sound because runs beyond the 5,000,000-step limit are not constrained by the spec) plus the `a >= b` edge of `lt` tightened `insertion_sort`'s `j` to (0,13) but changed 2 of 500 cases (both slightly worse). Reverted. Reason it cannot help: the countdown loop `while (j >= 0 && a[j] > key) j--` would need the *next* iteration's load at address `j-1` hoisted before the test; on the last iteration that address is -1, so an unguarded hoist can really trap and the rejection is correct. Legal alternatives (guarded hoist = only overlaps the branch; clamped address `select(j-1 >= 0, j-1, 0)` = 2 extra ops and needs a relational proof for `select`) look worth < 1 % in total.
* **Multiversioning / bounds-check removal** (`bounds_trap`, 3.7 % of cycles): a loop `for i < n { if !(i < 64) trap; ... }` is check-free when n <= 64. Hand-measured at n = 60: 391 -> 335 cycles (-14 %) on that one program, only for inputs with n <= 64; ~0.5 % of the total at best, for a loop-versioning pass. Not built.
* **Latch duplication / cost-based if-conversion:** see 5.1 (leftover ~4 % of one program) and 5.1e (swap if-conversion only helps `wide_fast`). Not built.
Conclusion: the remaining plan items are each < 0.5 % of the 500-case total and each needs a new SSA transformation; deferred, not blocked.
### 5.1g Item 5 (magic division / division by constants) evaluated in loop run 6, nothing left to build (do not repeat)
Test programs (12 variants of `div`/`mod` by a literal on signed/unsigned/ranged operands, in `kw/dv/` of the session, not kept) and a scan of all bench programs for `div`/`mod` that survive optimisation:
* `div_const`/`find_magic` cover every case a single 32-bit `mul` + `shr` can do exactly. It is provably limited: the product `x*m` must stay below 2^32, e.g. `x / 7` has no exact pair for `x < 2^16` (needs 35 bits), `x / 1000` none for `x < 2^28`. KIR has no `mulhi` and no arithmetic shift, so signed unknown `x / 2^k` needs 5-6 ALU ops (similar latency, more ops) and `x / c` for wide `x` needs a second multiply (best exact two-step form I found, `x = 4096a + b`, `q = 409a + ((6a+b)*26215 >> 18)`, is ~10 cycles vs 12 for `div`).
* Modular accumulation with a small quotient (`adler32` `% 65521`, `fletcher16` `% 255`, `s = (s + x) % M` chains): already handled by the existing `csub` (compare + subtract) and the lazy-`mod` hoist (one `mod` after the loop); a hand-written chained version of two dependent `mod 255` in a loop that stores each result also ends with 0 `mod`s and a 4x speed-up with no new code.
* Tried: making `Ranges::xfer` for `select` condition-aware (`select(x < K, x, x - K)` gets `x - K` bounded by `x >= K`, with `arm_iv`): ranges for the second `mod` of `fletcher16` became exact (`(0,508)`), but 0 of 500 cases changed and the hand-written chain was already optimal without it. Reverted.
* Remaining `div`/`mod` in the bench: `digit_sum` (`/10`, `%10` of a 24-bit value; the exact forms are not faster), `count_divisors`/`gcd`/`lcm_pair`/`is_prime` (variable divisors), `powmod`/`pow_fast`/`factorial_mod` (`%` of a 25-32 bit product). All are at the ISA's limit.
### 5.1c Extra unroll (3,6,16) and ifc (8,12) candidates: tried in loop iteration 6, no gain (do not repeat)

Added after the walk from its final setting: 3 cases better (`rotate_array|slow_memory` 368 -> 350, `insertion_sort` -13), 3 worse (`lcm_pair` +12..+20 on three machines), total change about 0. Not kept.

### 5.0 Determinism of the search (DONE, loop iteration 8)

### 5.0d `phase_a` prefix cache (DONE, loop 3a009676 run 1)
`driver.rs`: `midend` caches the function right after `build_ssa` + the first `phase_a` in `Search.pa`, keyed by exactly what those read:
`(thr, alg, csub, fwd, ifc, iv, lazy, sccp, large::roll_mode(), light)` (type `PaKey`). Candidates that differ only in later knobs (peel, unroll, sched, spill,
hot, reuse, spillk, rot, gu, acc, sr, tree, sink, spec) reuse a clone instead of recomputing. Exact (a clone of what would be computed), so output is unchanged:
500 bench cases bit-identical (0 changed; slowest bench program 0.85 -> 0.56 s), 28 unit tests, kfuzz gen/mutate seed 505 clean, ten large programs identical
cycles (worst 6.6 s, was 8.8 s; max RSS 0.4 GB). Slow fuzz case `g303001329` 2.09 -> 1.51 s. Switch off for debugging with `KOPT_NOPACACHE=1`.
### 5.0c Profile of a slow 1,162-instruction program (done, loop iteration 10: no cheap saving)

`KOPT_PROF=1` on `out/kfuzz/fail/gen/g101000773.kir` (1.85 s search, 19 candidates): `phase_a` 0.45 s (11 runs), `ranges` 0.33 s (43 calls, ~8 ms each), `post_sched2` 0.36 s (new profile label), peeling 0.11 s, ifconv 0.08 s, mem_pass 0.05 s; the rest is spread thinly (allocate, scheduling, scoring). No single hot spot: the best lever would be caching or incremental `ranges` (up to ~15 % of the time). Not done: the saving is too small for the risk.

### 5.0b Lower WORK_BUDGET (tried in loop iteration 9, not kept)

`WORK_BUDGET` 16,000 -> 10,000: bench sample 72,514 -> 72,676 (+0.2 %; 2 better by 1 cycle, 4 worse: `spill_storm` +60..+424, `vending_fsm|wide_fast` +50), while the slow fuzz programs only went from ~2 s to 1.3-2.0 s. Kept 16,000. The real fix for those programs would be making single candidates cheaper (profile `KOPT_PROF=1` on `out/kfuzz/fail/gen/g101000773.kir`), not a smaller budget.

kfuzz reported `nondeterministic` on 1,000+ instruction generated programs (the rules require the same output every run). Cause: for programs of
normal size the search still obeyed the *wall-clock* soft limit (`large::soft_expired`, 1.73 s) in `large_extras` (hot/cold layout and reroll
candidates, run last and often the winner) and in the main walk. A run that was merely a little slower (load) dropped those candidates, e.g.
`g101000773` gave two different outputs (the second under load). Fix (`driver.rs`): the clock gates now apply only when `size > MID` (6000);
normal programs are limited by the deterministic work counter `spent` only, and the 2.6 s hard-deadline fallback still protects the time limit.
Check: the two cases above give identical output quiet and with 8 copies running at once; the 500 bench cases are bit-identical (0 changed, tmax 0.66 s);
28 unit tests pass; kfuzz gen/mutate seeds 101/202/303/404 x 500: no mismatch. **Still seen** (rare, only under heavy parallel load): generated
programs of ~430-1,160 instructions take 1.8-2.2 s alone (bench max is 0.7 s), so with many kfuzz threads they reach the 2.6 s hard fallback and
differ. If the judges' machine is much slower than this laptop, look at lowering `WORK_BUDGET` (16,000) so that such programs finish well before 1 s.

### 5.1b Evaluated in loop iteration 4, no code change (do not repeat)
* **Worst remaining ratios** (optimized/original, current `main`): `factorial_mod` 0.63-0.79 on every machine, `multi_exit` 0.41-0.66,
  `rle_encode|few_registers` 0.42, `sieve|slow_memory` 0.41. Everything else is below 0.4.
* **`factorial_mod`**: the loop-carried chain is `mul` (3) + `mod 1000003` (12) = 15 cycles per iteration. `mod` by a
  constant cannot use the 32-bit multiply-shift magic (the product is up to 2^25, too large for `x*M < 2^32`). The split
  form `((x>>k)*M)>>s` plus a correction step costs `shr, mul, shr, mul, sub, lt/sel` ~ 12-15 cycles on the same chain, so no gain.
  Only a `mulhi`-style instruction would change that, and KIR has none. Dead end unless the trip count (<= 31) can be used.
* **`multi_exit`**: already 4x unrolled with the exit tests kept; the dependent `addsh/add` chain per step bounds it.
* **`sieve`**: the inner `while` is unrolled 8x already; the remaining cost is the stores (1 per cycle) and the loop-entry tests.
### 5.2 Other cycle sinks (sum over machines and inputs, before this session's work)
`sieve` ~8.8 %, `bubble_sort` 6.5 %, `matmul4` 4.2 % (now less), `count_divisors` 3.6 %, `transpose8` 3.6 %,
`bounds_trap` 3.2 %, `crc_bitwise` 2.7 %, `strength_ladder` 2.5 %, `vending_fsm` 2.5 %, `conv_mac` 2.4 %.
`transpose8` and `matvec8`: fixed by peel 4/5 (section 4, item 7).

### 5.3 Large programs (~100k instructions, each worth ~200 normal programs under Average Cycles)
Done: all generated families finish with a large margin. Re-checked on 2026-11-13 after the merges (sink, peel 4/5; WSL on the Windows laptop): all ten verify, worst `large_loops` 8.8 s and `large_ladder` 8.0 s (was 6.2 s max on the Mac; the cause is unknown: machine load or the extra candidates; still 3.4x under 30 s). If it grows further, look at `large::soft_expired` budgets first. `large_chain` (a long dependent chain with random
constants, nothing reducible) is unchanged and probably at its floor. Still untested: shapes other than what
`tests/gen_large.py` produces, and a judges' machine that is slower than this laptop (worst case here: 6.2 s of 30 s).

### 5.4 From the original plan (`FINAL_FINAL.md` section 4 and 7), not yet built
Magic division with peeling, divisibility test, signed division by constants (`src/algebra.rs`); latch duplication
and cost-based if-conversion; multiversioning on trip count; wrapped intervals and relational facts in the range
analysis; GVN-PRE. GF(2) CRC tables and idiom recognition are **not built** (the organisers have not
confirmed they are allowed).

## 6. Branches and housekeeping

* Unmerged remote branches: `claude/midend-8682d5` (code sinking: correct, almost no gain, skip) and
  `claude/large-progs-2-4b8b5d` (nothing measured). PRs #1-#7 on GitHub are obsolete (contained in #8); close them.
  The old `claude/*` branches can go once the PR for this branch is merged.
* `out/` (kfuzz failure cases) is git-ignored; `kfuzz` writes failing programs to `out/kfuzz/fail/`.
* Commits made by the assistant end with a `Co-Authored-By: Claude` line.

## 7. Rules that matter for the submission (from `kit/RULES_AND_SUBMISSION.md`)

Time limit per program is 5 s (30 s for ~100k-instruction programs), wall-clock including start-up; a crash or
time-out counts as a failed case. kopt must write the input unchanged if anything goes wrong (it does: top-level
catch-all plus an internal deadline in `src/main.rs` / `src/large.rs`). The output must match the input on the
judges' much larger input set (kopt self-checks every candidate before writing it). The judges use the **last
commit before the deadline** of the submitted repository: remove `kit/` from that commit and make sure `./build.sh`
works from a fresh clone.

### Submission readiness check (loop iteration 7, 2026-11-13, `main` at `da30d22`)
Fresh clone of `main` + `git rm -r kit` + `./build.sh` (6 s, exit 0) + `./kopt sieve.kir --machine machine_default.cfg -o out.kir`:
works from the clone without `kit/`; `kvalidate --random 200` says the output behaves like the input (1,297 -> 1,187 cycles on the sample).
Also verified: `kopt` and `build.sh` are tracked with mode 100755; `.gitattributes` forces LF for `*.sh`, `*.py`, `kopt`; no `.o`/`target/`/`__pycache__`
tracked; `DESIGN.md` has the 7 required headings in order (sections 1-7; later `## 8.` sections are extra). The only remaining step before submitting is `git rm -r kit` in the last commit.

### Pre-submission verification (2026-11-13, `main` at `a32c393`; run by hand after loop run 6)
Everything below was run on the final code (WSL on the Windows laptop, binary built from `main`).
* **Official `kvalidate` on every optimised bench program:** all 100 programs x 5 public machines, `--random 300 --seed 7`: 500/500 OK (`$HOME/kw/val1.sh`; ~37 s with `xargs -P 4`).
* **Determinism:** all 10 generated large programs (~100k instructions) give byte-identical output on two consecutive runs (quiet machine). Normal programs: identical (see 5.0 for the only exception, the 2.6 s hard-deadline fallback under heavy parallel load).
* **Unseen machines:** 4 hand-made machine files (issue 1-4, mem_ports 1-3, registers 4-12, icache 16-128, penalties 0-9, lat_div 6-30, all four fused combinations) x 100 programs = 400 runs, `kvalidate --random 60`: 0 failures; `mac` never appears when `fused_mac=0`, `addsh` never when `fused_addsh=0`. (Note: `registers` must be >= 4; the validator rejects 3.)
* **Wrong profiles:** 7 programs x 2 foreign `.prof` files x 2 machines = 28 runs: 0 failures.
* **Rules hygiene:** no bench program names or hashes in `src/` (the only name matches are an integer `gcd` helper and a comment about popcount); no external crates; `kopt`/`build.sh` mode 100755; `.gitattributes` forces LF; `kit/` not tracked; fresh clone builds in ~5 s and runs from any folder.
* **Not verified:** the judges' machine speed (single core, Ubuntu 22.04). Slowest bench program ~0.5 s, large programs <= 6.3 s here (limits 5 s / 30 s).
