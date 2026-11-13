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

7. **`peel: 4` / `peel: 5`** (DONE, loop iteration 1): same nest peeling as `peel: 3` but the enclosing nest may be 3x / 4x
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

### 5.1 Hoist spill loads (`spill_storm` on few-register machines): experiment half done
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

### 5.2 Other cycle sinks (sum over machines and inputs, before this session's work)
`sieve` ~8.8 %, `bubble_sort` 6.5 %, `matmul4` 4.2 % (now less), `count_divisors` 3.6 %, `transpose8` 3.6 %,
`bounds_trap` 3.2 %, `crc_bitwise` 2.7 %, `strength_ladder` 2.5 %, `vending_fsm` 2.5 %, `conv_mac` 2.4 %.
`transpose8` and `matvec8`: fixed by peel 4/5 (section 4, item 7).

### 5.3 Large programs (~100k instructions, each worth ~200 normal programs under Average Cycles)
Done: all generated families finish with a large margin. `large_chain` (a long dependent chain with random
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
