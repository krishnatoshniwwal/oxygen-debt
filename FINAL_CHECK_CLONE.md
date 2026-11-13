# FINAL_CHECK_CLONE - judge simulation on a fresh clone

Commit under test: `a0257fbb1d4d4b8a17d53d8685e0fdc42df16f9d` (a0257fb, tip of `integration`), fresh `git clone` of GitHub.
No `src/` changes were made.

| # | Item | Result |
|---|------|--------|
| 1a | `git ls-files \| grep '^kit/'` empty | PASS |
| 1b | no target/, __pycache__, *.o, out/ tracked | PASS |
| 1c | `kopt` and `build.sh` mode 100755 | PASS |
| 1d | .gitattributes: `*.sh`, `*.py`, `kopt` = `eol=lf` | PASS |
| 2 | `./build.sh` from fresh clone, no kit/, offline | PASS, exit 0, 22 s wall (5 compiler warnings only) |
| 3a | Determinism: 10 programs x 3 machines (default, few_registers, slow_memory), run from two different cwds, `cmp` | PASS, 30/30 identical |
| 3b | `kvalidate OUT --against PROG --machine M` | PASS, 30/30 OK (101 inputs each) |
| 4a | 6001+ instr (gen_large --size 6100; mix, irreducible, diamonds, nestloops) | PASS, rc 0, 0-1 s, optimized; mix validates OK |
| 4b | 40001+ instr (gen_large --size 40100; mix, loops, stld) | PASS, rc 0, 1-2 s, optimized; mix and loops validate OK |
| 4c | empty file | PASS, rc 0, output = input unchanged |
| 4d | valid-UTF8 non-KIR text | PASS, rc 0, output = input unchanged |
| 4e | missing machine file / garbage machine file | PASS, rc 0, output = input unchanged |
| 4f | missing or garbage profile | PASS, rc 0, optimized without profile |
| 5 | DESIGN.md has the 7 required headings, in order (sections 1-7) | PASS |

## Problems / notes (none blocking)

1. **Binary (non-UTF-8) input**: rc 2, no output file written, message "cannot read ...: invalid utf-8". This is the only case where the "write the input unchanged on failure" rule is not met. It is unlikely in judging, but `src/main.rs` could use a lossy or raw-byte copy. (I did not change src/.)
2. **Missing input file**: rc 2, no output, which is reasonable (nothing to copy).
3. **Unwritable output path**: rc 1, which is reasonable.
4. No args or bad usage: rc 2 plus usage message.
5. DESIGN.md has extra sections after 7, and several are numbered `## 8`, which is cosmetic. The 7 required headings come first and in order.
6. `git archive` lost the exec bit on kit `kvalidate`, so I ran it with `python3`. This is only a simulation artifact and not a repo issue.
7. Cargo warnings (unused `mut` etc.) are harmless.
