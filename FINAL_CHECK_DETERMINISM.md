# Final check: determinism of kopt on large programs

Commit under test: `a0257fbb1d4d4b8a17d53d8685e0fdc42df16f9d` (tip of `integration`; src/ untouched). Release build, offline.
Programs: all 17 from `tests/gen_large.py`. Machine: 4 cores.

Commands:
- `python3 tests/determinism_large.py target/release/kopt_bin OUT --loads 1,2` (total 13m39s; exit 1, "problems: 1")
- `python3 tests/measure_large.py target/release/kopt_bin OUT --c 64 --q 2` (all 17 status ok)

## Result: NOT fully deterministic — 1 difference out of 17 programs x 4 loaded runs

`large_nestloops` produced different bytes in one run: pinned to core 0 beside 2 busy loops, run #1
(25.06 s, 4.8x slowdown, **hit the 25 s hard-deadline fallback**). Run #2 under the same load (25.07 s, also at
the deadline) happened to be identical to the reference. Unloaded it takes 5.22 s; with 1 busy loop 13.8 s (2.6x);
with 2 busy loops it needs ~25 s of wall time, i.e. exactly the deadline, so the work-counter stop is not reached
before the clock fallback fires. The fallback is a clock decision, so output depends on load there.
All other 16 programs were identical in every run; their worst loaded time is large_diamonds 24.09 s
(4.1x slowdown), only ~0.9 s below the deadline, so it is also close to the same cliff.

Other expected note: `large_deepnest` hits the deadline (25.1 s) in every run, unloaded too (output = unchanged
input); it was identical in all runs, since the fallback happens at the same point in the pipeline.

## Table (seconds; loaded = taskset -c 0 with 1 hog / 2 hogs, two runs each)

| program | unloaded | 1 hog (#1 / #2) | 2 hogs (#1 / #2) | output | cycles in -> out (C=64,Q=2) | measure_large |
|---|---|---|---|---|---|---|
| large_affine | 1.34 | 3.05 / 2.77 | 4.91 / 4.66 | identical | 377681 -> 155542 | ok |
| large_chain | 1.70 | 3.46 / 3.59 | 5.32 / 5.55 | identical | 206527 -> 206527 | ok |
| large_cold | 1.22 | 2.67 / 2.59 | 4.28 / 4.13 | identical | 4919 -> 2502 | ok |
| large_crc | 3.83 | 9.11 / 9.84 | 15.59 / 16.48 | identical | 145508 -> 32182 | ok |
| large_crcstep | 1.97 | 4.11 / 4.02 | 6.14 / 6.37 | identical | 203875 -> 161373 | ok |
| large_deepnest | 25.12 | 25.18 / 25.11 | 25.17 / 25.10 | identical | 279867 -> 279867 | ok |
| large_diamonds | 6.36 | 14.47 / 14.53 | 23.21 / 24.09 | identical | 39880 -> 25824 | ok |
| large_irreducible | 4.00 | 9.14 / 9.85 | 15.14 / 16.67 | identical | 453533 -> 256473 | ok |
| large_ladder | 5.04 | 11.77 / 12.24 | 18.73 / 19.05 | identical | 153288 -> 85644 | ok |
| large_loops | 3.66 | 7.26 / 7.17 | 12.21 / 12.36 | identical | 321391 -> 220403 | ok |
| large_mix | 1.41 | 2.97 / 3.23 | 4.90 / 4.92 | identical | 312772 -> 102528 | ok |
| large_nest | 2.24 | 5.03 / 4.77 | 8.27 / 7.52 | identical | 199910 -> 48 | ok |
| large_nestloops | 5.22 | 13.78 / 13.84 | 25.06 / 25.07 | DIFFERENT (1 of 4 loaded runs) | 683569 -> 389419 | ok |
| large_poly | 1.07 | 2.27 / 2.32 | 3.19 / 3.40 | identical | 239939 -> 105444 | ok |
| large_pressure | 2.47 | 5.44 / 5.53 | 8.86 / 8.36 | identical | 404251 -> 214390 | ok |
| large_smallloops | 4.14 | 9.77 / 9.91 | 16.41 / 16.54 | identical | 746687 -> 287152 | ok |
| large_stld | 0.73 | 1.70 / 1.60 | 2.53 / 2.58 | identical | 269919 -> 89684 | ok |

measure_large: all 17 status ok. Instruction counts in: ~100k each (deepnest 70k).

## Observation
Observed: loaded times for nestloops (>=25 s) and diamonds (24.1 s) are at or near the 25 s fallback, so the work-counter stop was not reached in time on a ~4x slower effective CPU (my reading; src not inspected). Several others (crc, ladder, irreducible) reach 15-19 s.
