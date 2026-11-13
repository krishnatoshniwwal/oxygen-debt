# Machine sweep report (tests/machine_sweep.py, kopt @ 9c390eb = main 70e94cc + docs)

Setup: 29 generated machines (registers 4/5/6, spill penalty 0..1000, issue width 1/8, mem_ports 1/4, icache
8/16/4096, icache penalty up to 1000, lat_div 40, lat_load 12, fused 0/1, taken-branch penalty 0/10/100, all-latency-1,
a wide-fast and a tiny-slow combination) x 100 kit/bench programs = 2900 cases. Each output checked against the
input with kirlib: sample .in + 3 perturbed variants + 30 generated inputs. Cycles counted on sample + 3 variants.
Run: `python3 -I tests/machine_sweep.py ./kopt out.json` (~10 min on 4 cores); `--report out.json` reprints the summary.
Raw results: `tests/machine_sweep_results.json`.

## Failures
**None.** 2900/2900 cases: valid KIR, no feature violations (mac/addsh respected when fused_*=0), no mismatches,
no crashes, no timeouts. Slowest kopt run 1.9 s (vending_fsm, wide_fast).

## Slower than the original
On the **sample input: none** (worst ratio 1.000 exactly, i.e. equal; div_trap on most machines, sem_bit_mix and
sem_shifts on w1_m1/w1_m4).
On the perturbed **variants: 5 of 11,600 measurements**, all insertion_sort / is_prime with a degenerate input
(array length 1, or a prime that exits at once), where the optimized prologue costs more than the tiny original run:

| case | orig | opt |
|---|---|---|
| insertion_sort, ic8_pen1000, variant1 (arg 17 -> n=1) | 1004 | 6001 |
| insertion_sort, tiny_slow, variant1 | 56 | 301 |
| is_prime, br100_ic16, variant1 | 27 | 118 |
| insertion_sort, ic16_pen20, variant1 | 24 | 85 |
| insertion_sort, ic8, variant1 | 7 | 16 |

Not a bug (output correct) but a real risk if hidden inputs include n=0/1 on a machine with a tiny icache: the
optimized code has a larger/worse-laid-out entry path. Reproducer (in this branch's scratch, 3 files): kit/bench/insertion_sort.kir,
a machine file with the defaults but `icache_size = 8`, `icache_penalty = 1000`, input `args 17` + the sample mem line:
original 1004 cycles, optimized 6001 (both execute 7 instructions; the difference is icache penalty from the
33-line optimized layout). Suggestion for whoever owns it: when icache_size is tiny, compare the optimized
program against the original on the entry-only path (n<=1) or keep the original as a fast path; low priority.

## Worst optimized/original ratios (sample)
Mostly 1.000 (no gain possible): div_trap everywhere, sem_bit_mix and sem_shifts on issue_width 1. Next worst:
poly_mac on wide_fast (1.000), dead_div_trap on div40 (0.977), store_then_div_trap on div40 (0.955),
factorial_mod (0.884, load12_div40_mul8), is_prime (0.877, br100_ic16), lcm_pair (0.850), wide_expression (0.831).
Per-machine total ratios (sample): from 0.0064 (k5_spill200, spill-bound originals) to 0.2698 (k8_spill0);
typical 0.10-0.15. Highest: k8_spill0 0.270, wide_fast 0.216, br100_ic16 0.212, ic8_pen1000 0.205, ic8 0.155.
Full table in the JSON / `--report`.

## Programs whose ratio differs most between machines
sem_bit_mix and sem_shifts (0.002 on k4_spill1000_w1 .. 1.000 on w1_m1), poly_mac (0.017 k5_spill200 .. 1.000
wide_fast), dead_div_trap (0.014 .. 0.977 div40), store_then_div_trap (0.014 .. 0.955 div40), factorial_mod
(0.002 .. 0.884), is_prime (0.003 .. 0.877 br100_ic16), lcm_pair, wide_expression, multi_exit, gcd (0.001 ic8_pen1000
.. 0.817 div40). Pattern: huge original-side penalties (spill, icache) are removed (ratio ~0), while a 40-cycle
divide or an issue-width-1 machine leaves the work bound by instructions the optimizer cannot remove. Notably
sem_bit_mix/sem_shifts reach 1.000 on issue_width=1 but are far lower elsewhere: on width 1 the gain came from
packing, so there may be room for strength reduction there (not a correctness issue).

## src/ bugs
None found.
