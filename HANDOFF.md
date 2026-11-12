# HANDOFF — kopt optimizer, state on 2026-11-13

Read this first, then `DESIGN.md` (how kopt works) and `FINAL_FINAL.md` (the full plan:
Part 1 §1 machine laws, §4 technique verdicts, §7 build order, §11 experiments).

## Setup facts

- Rust, std only. **cargo exists only in WSL Ubuntu**:
  `wsl -e bash -c '. ~/.cargo/env; cd /mnt/c/Users/Krishna/Desktop/oxygen-debt && cargo build --release --offline'`.
  From Git Bash prefix `MSYS_NO_PATHCONV=1`; use `~/kw` in WSL for scratch (WSL `/tmp` is wiped).
- The starter kit is in `kit/` (gitignored; never commit it): `KIR_SPEC.md`, `kirlib.py` (the exact
  timing model = ground truth), `bench/*.kir` + `.in`, `machines/*.cfg`.
- **Regression check** (always before merging into main):
  `python3 tests/measure.py <kopt_bin> out.json` then `python3 tests/compare_runs.py base.json out.json`
  (100 bench programs × 5 machines, sample inputs + 3 variants each). Required: `fails=0`, nothing
  slower than the original.
- **Fuzzing**: `cargo build --release --offline --features tools --bin kfuzz`, then
  `kfuzz gen|mutate|machines --n 200 --seed S --kopt <bin> --kit kit` (random programs, judge-style
  mutations, random machines). Unit tests: `cargo test --release --offline` (28 pass).
- New passes are added as optional `Variant` settings in `src/driver.rs`; the driver measures every
  candidate with the exact model and keeps one only if strictly faster than the input and equal on
  the self-check inputs. Note the names: `Variant.tree` = tree-height reduction, `Variant.thr` = DFA
  threading.

## Where main stands (pushed, `origin/main`)

All 7 first-round session branches are merged (PR #8), plus two passes and two bug fixes:

| Commit | What |
|---|---|
| PR #8 | loops-arrays, regs-sched, algebra, midend, layout-control, large-progs, verify-infra merged |
| `d097f03` | `iv_rewrite` accepts loop-invariant steps (sieve) |
| `b8428df` | `src/memfwd.rs`: memory forwarding through joins and across iterations (bubble_sort) |
| `a068883` | verify-infra's 2 correctness fixes (speculation, lazy mod) |

Bench, old main → now: sample cycles **88,988 → ~76,900 (−13.6%)**, 0 failures, no case slower than
its original. ~14,000 fuzzed programs gave no wrong output.

PRs #1–#7 are obsolete (all contained in #8); close them.

## Unmerged work on pushed branches (all WIP checkpoints, verify before merging)

Merge each into main only after: it builds, `cargo test` passes, `measure.py` shows 0 failures and
no total regression, and kfuzz finds no mismatch. All are based on the merged main, so conflicts
should be small (`src/driver.rs`, `DESIGN.md`).

| Branch | Ahead of main | What it has | Session's own result |
|---|---|---|---|
| `claude/regs-sched-66b143` | 3 | **scratch peephole** (spill load/store cleanup), second naming mode | peephole: 29 cases better, 0 worse, 0 failures. **Merge first.** |
| `claude/algebra-b60799` | 2 | `algebra::iv_divmod` (i/c, i%c as wrap counters, `alg` bit 4); partial magic division / divisibility test | iv_divmod checked; items 3–4 partial |
| `claude/layout-control-3092e9` | 2 | node splitting for two-entrance (irreducible) loops | measured at P=0/4/8; check numbers before merging |
| `claude/midend-8682d5` | 3 | code sinking pass | correct, nearly no gain (1 case better) |
| `claude/large-progs-2-4b8b5d` | 1 | just started (timing / budgets for 100k programs) | nothing measured |

## Remaining cycle sinks (merged main, sum over 5 machines × 4 inputs)

sieve 8.8%, spill_storm 8.4%, bubble_sort 6.5%, matmul4 4.2%, count_divisors 3.6%, transpose8 3.6%,
bounds_trap 3.2%, crc_bitwise 2.7%, strength_ladder 2.5%, vending_fsm 2.5%, conv_mac 2.4%.

## Next steps, in ROI order

1. **Merge `claude/regs-sched-66b143`** (scratch peephole), then `algebra` (iv_divmod), after the checks above.
2. **Large programs (~100k instructions)**: hidden tests include them, and each is worth ~200 normal
   programs under Average Cycles. Linear-time passes, work budget + safety deadline (~25 s huge,
   ~2.5 s normal), hot/cold layout so hot code sits in the first `icache_size` instructions, partial
   rerolling only of code beyond the cache. Generators: `tests/gen_large.py`, `tests/measure_large.py`.
3. **Register pressure** (spill_storm, few_registers machine): full MIN-then-steer LRU naming with an
   exact-LRU oracle test, WAW-aware colours, loop-aware spilling (`src/sched2.rs`, `src/ssa.rs`).
4. **Unroll-and-jam / full inner unroll** for matmul4, transpose8, matvec8, row_sums (new pass; affine
   dependence test for legality).
5. **Algebra**: magic division with peeling (digit_sum), divisibility test, signed division by
   constant, i*c strength reduction (`src/algebra.rs`).
6. **Layout / control**: latch duplication with rotated placement, cost-based if-conversion,
   multiversioning on trip count, hot-code budget (MCKP + exact local search) (`src/layout.rs`, `src/control.rs`).
7. **Mid-end**: wrapped intervals + congruence domain, relational facts (i < n), affine dependence
   tests, GVN-PRE (`src/domain.rs`, `src/ranges.rs`).
8. Update `FINAL_FINAL.md` §0.1 (it still says "Rust not started"; that is wrong).

## Known caveats

- The 11 cases that got slower than the old main (worst `sieve|few_registers` 5,343 → 6,162) are
  still well below their originals; worth a look when tuning the candidate search.
- The verify-infra fixes cost ~0.2% cycles; keep them (they fix wrong outputs).
- GF(2) CRC tables and idiom recognition are implemented off by default: the organisers have not
  confirmed they are allowed.
- Local sessions here could not `git commit` (the auto-mode classifier blocked it); the manager
  committed for them. Commits named "WIP checkpoint by manager" may not build on their own.
