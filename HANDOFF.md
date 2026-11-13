# HANDOFF — kopt optimizer (state: 2026-11-13, branch `main`; all first-round branches merged)

kopt reads a KIR program + a machine file and writes a faster KIR program (Rust, std only).
Read this file first, then `DESIGN.md` (how kopt works, one section per pass) and `FINAL_FINAL.md`
(the original master plan: Part 1 §1 machine laws, §4 technique verdicts, §7 build order).

## 1. Where the work is

* **`main`** contains everything: `merge/scratch-peephole` (PR #9), PR #3, `midend` (sink) and `large-progs-2` are merged. Work from `main`.
* Everything below is committed (the last loop iteration, item 7, is committed locally and not yet pushed).
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
* The starter kit is in `kit/` and **is tracked** (spec, `kirlib.py` = exact timing model = ground truth, bench
  programs, machine files). The submission rules say the kit must **not** be in the submitted repo: remove it
  (`git rm -r kit`) in the final submission commit. The tests need it, so do that last.
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

### 5.1 Hoist spill loads (`spill_storm` on few-register machines): experiment half done (loop iteration 5: re-applied, no gain)
A hand-written loop for `spill_storm|few_registers` (all 12 accumulators in scratch, loop bound in memory, 3 temp
names, each load issued two statements ahead of its use) costs **3,497 cycles at n=63 vs 4,587** from kopt now,
and is verified equal to the original. It is a straight rewrite of the statements in `kit/bench/spill_storm.kir`.
The generic search does not find it: every non-program-order seed fails in `Region::name_block`
(`src/sched2.rs`). `notes/experiment_prefetch_seeds.patch` adds `prefetch_order` seeds (program order with each load
moved up to d = 1..8 places) plus debug prints (`KOPT_S2DEBUG`) and a `KOPT_S2FILE` hook that runs sched2 on one
candidate file. The debug output showed the failure: only 3 candidate names are free in the loop block, and the
`reuse` cache keeps one loaded value live across two statements, so a value stays live for ~8 positions and three
temporaries are not enough at the prefetch distance. Ways forward: (a) make `rewrite_spills` *not* keep values across
statements when prefetching is wanted and let sched2 hoist; (b) a dedicated pass that software-pipelines spill loads
with an explicit 3-name rotation; (c) let `name_block` rename the loop counter too.
Value: `spill_storm` is ~8 % of all cycles, so the potential is roughly 0.5-1 % of the total.

**Loop iteration 5 on 5.1 (stopped, nothing committed to src/):** `notes/experiment_prefetch_seeds.patch` no longer applies to `sched2.rs`
(`opt_block` changed; CRLF working copies need `patch -p1 -F3 -l`). Re-applying its `prefetch_order` seeds by hand (d = 1,2,3,4,6,8) and
measuring all 500 cases: 3 better by 4 cycles each (`matrix_add4|few_registers`, `sensor_sum`, `wide_expression`), 1 worse by 20
(`matmul4|slow_memory`), `spill_storm` unchanged, so the seeds alone are worthless as predicted: `name_block` still has too few names.
Next step if resumed: option (a) or (b) above, i.e. fix the register pressure first (a `reuse: 0` + spill variant that stores nothing across
statements, or a dedicated 3-name rotation pass); the seeds only matter after that. Expected value stays 0.5-1 %.

### 5.1c Extra unroll (3,6,16) and ifc (8,12) candidates: tried in loop iteration 6, no gain (do not repeat)

Added after the walk from its final setting: 3 cases better (`rotate_array|slow_memory` 368 -> 350, `insertion_sort` -13), 3 worse (`lcm_pair` +12..+20 on three machines), total change about 0. Not kept.

### 5.0 Determinism of the search (DONE, loop iteration 8)

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
analysis; GVN-PRE. GF(2) CRC tables and idiom recognition exist but are **off by default** (the organisers have not
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
