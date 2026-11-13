# FINAL_CHECK_FUZZ

Commit under test: a0257fb (tip of `integration`). Runs were sequential, 300 cases each, --jobs 3.

| mode | seed 31 | seed 32 | seed 33 |
|---|---|---|---|
| gen | regress 2 | regress 3 | nondeterministic 1 (g33000199) |
| mutate | clean | clean | regress 1 |
| machines | clean | clean | clean |

No MISMATCH, VALIDATOR-UNSOUND, crash or timeout in any of the 9 runs. `regress` (6 total) is the known benign kind.

## Finding: g33000199 (gen seed 33) "nondeterministic"
Reproducer: `fuzz_failures/g33000199.kir` + `.cfg` (also `.min.kir`, `.txt`):
`kopt_bin g33000199.kir --machine g33000199.cfg -o out.kir`
Rerun alone 5 times: it still reproduces. Runs 1-2 (2.09 s, 2.27 s) gave one output hash; runs 3-5 (2.6 s each) gave another. So it is real, not load-induced: this case sits near kopt's 2.6 s hard-deadline fallback, and which result you get depends on whether the deadline hits. Both outputs passed the fuzzer's equivalence checks in other runs (no wrong-output finding), but the output is timing-dependent, and the case is one of the slowest (slowest kopt run 2.61 s in that run). I did not determine which pass is slow. Not a miscompile as far as observed.
