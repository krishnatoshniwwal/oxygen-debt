# FINAL_FINAL.md — kopt v2: master plan, results and handoff

This is the single document to continue from. It contains everything produced so far:

* **Part 0 — Handoff**: where things stand, what to do next, how to resume, what is unfinished.
* **Part 1 — The final plan**: the design, the evidence ledger, the experiments still to run,
  the open risks (the full text of `FINAL_PLAN.md`).
* **Part 2 — Results**: an index of every experiment, the command that reproduces it, and the raw
  output of the two newest experiments.
* **Part 3 — The review record**: both review passes in full (the full text of `REVIEW.md`):
  per-topic literature, derivations, measurements, the citation audits and the coverage report.

If this file and the separate files ever disagree, this file is the newer one. The separate files
(`PLAN.md`, `REVIEW.md`, `FINAL_PLAN.md`) are kept as history.

---

# Part 0 — Handoff

## 0.1 Where things stand

| Item | State |
|---|---|
| kopt v2 (Rust) | **Built and working** (std-only Rust in `src/`; the Python v1 is in `pyref/`). See `HANDOFF.md` for the current state, branches and measurements. |
| Plan | **Complete** (Part 1). Every key decision has a verdict, evidence, a confidence level and the condition that would flip it. |
| Review | **Complete for topics 1–12**, two passes, with a second citation audit (Part 3). |
| Measurements | 13 experiment scripts against the exact cost model `kit/kirlib.py` (Part 2). Every variant's output was checked equal to the original program. |
| Open experiments | 11, listed in Part 1 §11 in priority order, each with when to run it, how much, and the adoption threshold. |
| Organiser questions | Scoring (Q1) and hidden inputs (Q4) **answered** and applied (Part 1 §1, §4.9, §5, §7, §8). Q9 (time-out) and half of Q10 (all inputs must match) answered by the rules; Q8 very likely answered. Still open: Q2, Q5, Q6, Q7 and the rest of Q10. |
| Branch | `claude/zealous-pasteur-xgw10q` (all work is pushed there). |

## 0.2 Repository map

| Path | What it is |
|---|---|
| `kit/` | The competition kit: `KIR_SPEC.md`, `RULES_AND_SUBMISSION.md`, `kirlib.py` (parser, interpreter, **exact timing model** — the ground truth), 100 bench programs in `kit/bench/`, 5 machine files in `kit/machines/`. |
| `src/`, `kopt`, `build.sh`, `tests/` | kopt v1 (Python). |
| `PLAN.md` | The research plan, revision 3 (history). |
| `REVIEW.md` | The review, both passes (history; also Part 3 here). |
| `FINAL_PLAN.md` | The final plan (history; also Part 1 here). |
| `review_experiments/` | `kh.py` harness + one script per measured claim + raw results of t13/t14. |
| `MASTER_PLAN_FINAL.md` | This file. |

## 0.3 What to do next (in this order)

1. **Send the open organiser questions** (Part 1 §8): Q2, Q5, Q6, Q7 and the open half of Q10. Q7 (how Success Rate and Average Cycles combine) and Q6 (GF(2)) change decisions;
   nothing else blocks on them.
2. **Build order step 1: infrastructure** (Part 1 §7) — Rust IR and parser, an exact port of the
   cost model checked cycle-identical to `kirlib` on all 100 bench programs × 5 machines, the
   validator, the fuzzer, the judge simulator, the machine generator. Nothing else is trusted
   until this exists.
3. **Run experiments 1–3 of Part 1 §11** as soon as the components they need exist (naming,
   scheduling, selection): they fix the parameters of the core design.
4. Continue the build order (§7), running each remaining §11 experiment at the point its text
   names, and record the result in the evidence ledger (§10).
5. Before each component lands: unit tests, fuzz runs on all machines, an updated `DESIGN.md`
   section (the rules require it).

## 0.4 Decision log (what changed from the first plan, and why)

| Decision | First plan said | Now | Why (evidence in Part 1 §10 / Part 3) |
|---|---|---|---|
| Rerolling | DO, biggest lever on `large` | DO (bounded), after clean-up and layout | beyond-cache cost is `max(Q, wait)`; in-cache rolling always loses; bench `large` programs fit `C` after clean-up |
| Register naming | ≤ K names per hot region, `mov`s at boundaries | MIN-then-steer with free refresh touches and scratch spills | R1–R7 read off the LRU model |
| Pipelining | multi-stage modulo scheduling | single-stage latency rotation, early-op set chosen exactly | fence keeps latency running; exact choice 3–29 % better on 4 of 8 loops; fixed rule can lose to no pipelining |
| Code-size budget | per-pass caps | one budget of *hot* instructions; knapsack seed + exact local search | position matters; gains interact by ±60 % at the `C` boundary |
| Block layout | Ext-TSP | directed TSP, exact ≤ 14 blocks, latch duplication with rotated placement | Kiln has no jump-distance term; rotated duplication 674 vs 703 (if-conversion) vs 895 |
| Magic division | DO (bounded) | DO with peeling | exact only below ≈ 2^16; `digit_sum` needs 3 peeled iterations |
| Lazy reduction | LATER | DO | `adler32` 1525 → 708 cycles |
| Closed forms | break-even n ≈ 7–9, guarded | unguarded | measured break-even n ≈ 3 |
| Matrix power | DO (bounded) | LATER | fast doubling ties an unroll-8 loop for n ≤ 63 |
| GF(2) summarisation | not in plan / LATER | DO (bounded), nibble table only, pending organisers | 1.7× over best bitwise CRC; mask-xor tree loses at K = 8 |
| B&B scheduling | DO (bounded) | DO (bounded), high priority | list is 2.7 % off optimal without LRU, 18 % with K = 8 |
| Local search over order | absent (STOKE was NO) | DO above the B&B limit | +13–48 % over list at K = 8, never worse; reordering is fully validator-proven |
| Candidate selection | never slower on any input | bounded loss + minimax regret | strict rule vetoed a 10.6× win for 1 cycle |
| Offline SMT rule mining | ask organisers | **not adopted** | the rules ban SMT/SAT solvers outright; solver-free proof classes only |
| Exact stage choice in rotation | fixed rule | exact enumeration of early-op sets | 3–29 % better on 4 of 8 loops (t14) |
| Selection objective | minimax regret with a loss bound | **mean cycles on `.in`-like inputs** | organiser answer: Average Cycles = total / programs; hidden inputs follow the samples |
| Rerolling (again) | DO (bounded) | **DO (bounded), high priority** | bench `large` is 0–1 % of cycles (t15), but rules §3 adds hidden ~100k-instruction programs worth ≈ 200 bench programs each under Average Cycles (briefly set to LATER, then reverted) |
| Priorities | by technique | **arrays, junk, loops, bits first** | ≈ 78 % of bench cycles (t15) |
| Crash / time-out handling | fallback to input | catch-all + internal deadline ≈ 2.5 s | organiser answer: a crash falls back to unoptimised cycles and is penalised |

## 0.5 Unfinished and known caveats

* **Review Tasks 2 and 4** of the original brief were never written; the brief is lost. Nothing in
  the plan depends on them.
* **Latency rotation** is measured on 8 loop shapes (t14); its combination with unrolling and
  multi-stage pipelining are not (experiment 6).
* **t14 on `few_registers`** covered 2 of 8 loop shapes (the run was stopped early); rerun with
  `python3 t14_modulo.py few_registers`.
* **t13's baseline** used simple R1 naming, not the plan's full MIN-then-steer naming; the gain of
  local search may be smaller after full naming (it cannot become negative).
* **Superseded in Part 3:** the review's topic 10 recommends minimax regret with a bounded loss;
  the organisers' scoring answer replaced it with mean cycles on `.in`-like inputs (Part 1 §4.9).
  Part 3 is kept as the historical record.
* **All measurements are on small hand-written loops and random blocks**, not on the hidden test
  set, and no measurement ran kopt itself.
* **Literature** was checked through search results and abstracts, 2–4 searches per topic; papers
  were not read in full. Tags in Part 3: **[verified]** confirmed by a search or page; **[unread]**
  from memory; **[derived]** / **[model]** computed with `kirlib.py`.

## 0.6 The review method (use it for any new topic)

1. **Pin the question.** Write the decision to settle, what would change your mind (skip research
   if nothing could), and the ground truth that outranks any paper (`time_trace`).
2. **Search in three passes**: primary sources; critics and successors; the last 3–4 years.
   Several searches per pass, different wording.
3. **Read for**: what was measured and against what; the objective they optimised (often not
   ours); stated limits; the numbers.
4. **Tag every claim**: verified / from memory / derived. Never upgrade a tag silently.
5. **Re-derive against the model**: translate the claim into the cost function, work an example,
   compute a break-even, check every assumption, look for what the model allows that no paper
   needed. **Run it** with `review_experiments/kh.py` and check outputs equal to the original.
6. **Decide**: verdict, confidence, the one reason; when it flips; if unsettled, the exact
   experiment (A vs B, inputs, parameter ranges) and its adoption threshold.
7. **Write it down at once**, one section per topic; commit every 2–3 topics; keep a running
   "still unsure" list.
8. **Audit at the end**: re-check citations, re-read verdicts that rest on memory only, report
   plainly which topics got the full treatment.

## 0.7 How to resume the experiments

All scripts run from `review_experiments/` with Python 3.8+ and no extra packages; they import
`kit/kirlib.py` through `kh.py`. To measure a new KIR snippet:

```python
import kh
cycles = kh.run(kir_text, kh.MACH['default'], args=[n], mem=[...])   # exact cycles
status, value = kh.ret(kir_text, args=[n], mem=[...])                # result, for equality checks
mc = kh.machine('default', icache_size=32)                           # a modified machine
```

The index of scripts, what each shows and its run time is in Part 2.

---

# Part 1 — The final plan

This is the final plan for the Rust rewrite. It merges the research plan (PLAN.md) with both
review passes (REVIEW.md) and their measurements, so it stands on its own: the design is in
sections 0–9, the evidence and confidence behind each key decision in section 10, the
experiments that settle the remaining open parameters in section 11, and the open risks in
section 12. Lines marked **Test** are open questions to settle by experiment. It ignores
deadlines on purpose. Every technique is weighed only on how many cycles it saves, how risky it
is for Law 1 (same behaviour), and whether it fits the 5 s / 30 s limit.

Verdicts used below:

* **DO**: core. It is in the target design.
* **DO (bounded)**: in the design, with a firm size or work cap.
* **LATER**: worth it, but only after the core is solid. Ranked by value.
* **NO**: we do not build it, and the reason is given.

---

## 0. The one idea that beats "just read the papers"

Papers optimise for real CPUs, which have out-of-order cores, branch predictors and
caches nobody can model exactly. **The Kiln is fully known.** `KIR_SPEC.md` section 10
is a closed-form cost function, and `kirlib.py` computes it exactly. So we can do what
production compilers can't:

1. **Use the papers for the transformations** (what is legal, how to do it fast).
2. **Use the exact Kiln cost model for the decisions** (whether, where, how much). We
   don't use the papers' heuristics for those.

Every "should we unroll / if-convert / use `mac` / spill / version this loop" question is
answered by measuring with the exact model, not by a guessed threshold. That is the
"optimisation-space exploration" idea (Triantafyllis et al., CGO 2003). Our version is
stronger because our cost oracle is exact, and theirs was a model of an Itanium.

The second idea: **derive the laws of this machine first**. Then pick only techniques
that attack the cycle sources those laws expose.

---

## 1. The laws of the Kiln (derived from KIR_SPEC section 10)

These are facts, not guesses. Every later decision follows from them. `W` = issue_width,
`P` = taken_branch_penalty, `C` = icache_size, `Q` = icache_penalty, `K` = registers,
`S` = reg_spill_penalty.

| # | Law | Consequence for the optimiser |
|---|---|---|
| L1 | Issue is **in order**. The run order fixes the schedule: an instruction never issues before the one before it. | The instruction order in the file *is* the schedule. Scheduling is a pure permutation problem, and the exact cost of any order is cheap to compute. |
| L2 | A **taken jump** blocks everything until `jump_cycle + 1 + P` (`fence = t + 1 + TB`). | A taken jump costs **`P` plus the unused issue slots of its own cycle** (not a flat `1+P`). Latency already in flight **keeps running during the fence**; `Q` and `S` stalls are also swallowed by it. |
| L3 | Every loop has **at least one taken jump per iteration**. | The back edge is a fixed tax per iteration. Only unrolling (÷u) or deleting the loop cuts it. A body with a two-way branch pays 2 on one arm unless the **loop latch is duplicated after each arm *and* the copy on the taken arm is placed right before the header** so its back edge falls through (then every path pays exactly 1; duplication without that placement still leaves one arm paying 2 — measured, section 10). |
| L4 | Instructions at file index `>= C` add `Q` to `prev` each time they run — **then the result is `max`-ed with the fence and operand readiness**. | Beyond-cache cost is `max(Q, wait)` per instruction; `W` is irrelevant there. Latency-bound instructions and the first instruction after a taken jump (when `Q <= 1+P`) are **free** beyond `C`. Hot, slot-bound code must sit below `C`; the entry block is fixed at index 0. |
| L5 | Register cost is **zero if the program text uses ≤ K distinct names** (dead/unreachable code and unused parameters count). Otherwise hardware **LRU**: touching an evicted name costs `S`, **also when it is only overwritten**; a never-used name is free; `S` is added to `prev` and overlaps operand waits. | Misses = touches with LRU stack distance > K (Mattson 1970). Write new values into a *resident dead* name, else a *fresh* name, never an evicted one. Any touch refreshes: `select d, X, s, s` is a free refresh of X. Parameters beyond K start spilled. See 4.3. |
| L6 | A write waits for the **old value of its destination** to be ready (in-order writes). | Reusing a name whose last write was a `div` (latency 12) stalls. Name choice affects the schedule, so allocation and scheduling must talk to each other. |
| L7 | A `br` whose taken target is the next block costs nothing extra. A `jmp` to the next block costs one issue slot and no fence. | Block merging removes `jmp` slots. Branch direction (which side falls through) is a free choice: we can invert the condition or swap targets. |
| L8 | Total = issue cycle of the last instruction + 1. Result latency after `ret` is free. | The tail of the program only needs `ret`'s operand ready. |
| L9 | Arithmetic is the ring Z/2^32: `add`, `sub`, `mul` are associative, commutative and distributive **with no exceptions**. | Reassociation, accumulator splitting, tree-height reduction and closed forms are always legal for these ops (unlike floats or C signed overflow). Only `div`, `mod`, `lt` and `shr` need care. |
| L10 | `div`, `mod`, `load`, `store` trap; `vload`, `vstore`, `trap` are visible; final data memory 0–3071 is visible; the scratch region 3072–4095 is invisible. | Every motion of those instructions needs a proof or a runtime guard (versioning). The scratch region is free storage, but its initial contents may not be zero on judge inputs, so never assume that. |
| L11 | **No functional-unit limits**: only `W` and `MP` bound issue. | Any number of `div`/`mul`/`load` latencies overlap. A long-latency op costs only on a critical chain or recurrence. |
| L12 | **Literals are free operands** everywhere an `s` is allowed (`store 3072, 5`, `ret 0`): always ready, touch no register. | Delete nearly every `const`: saves a slot and a name. |
| L13 | A memory op that finds `MP` full moves to the next cycle **and drags every later instruction with it**. | Never put memory ops back to back when `MP < W`; interleave with ALU ops. `vload`/`vstore` count. |
| L14 | **Every latency, including `lat_alu`, is a machine parameter ≥ 1.** | No hard-coded "shift beats mul" or "`mac` beats `mul+add`". Extraction is by the machine's numbers only. |
| L15 | Memory has **no timing dependence**: `store` has no result time; a later `load` pays only its own `lat_load`. | Scratch spill = 1 store slot + 1 load slot + `lat_load` on the chain, literal addresses, no name touched. |
| L16 | A non-terminating original constrains nothing; the output may run ≤ 20× the original's steps + 1000; only the **final** data memory is compared when no trap intervenes. | One-off setup code is allowed but bounded; stores in a trap-free, volatile-free region may be reordered, merged, delayed. |

**Where cycles come from** in any Kiln run is exactly:
issue slots (`#instr / W`, plus `MP` drag, L13) + latency stalls + `P`(+unused slots) × taken jumps + `max(Q, wait)` for
executed beyond-cache instructions + un-absorbed `S` × LRU misses + WAW stalls. Every item in this plan
attacks one or more of these six terms. That is how each one is judged.

**How we are scored** (organiser answer). Two metrics: **Success Rate** = correct programs /
total programs × 100, and **Average Cycles** = total cycles / number of programs. If kopt crashes
or errors on a case, that case gets the unoptimised cycle count; a crash, **a time-out** or a
missing output fails the case (`RULES_AND_SUBMISSION.md` §3 step 1), and so does any difference
from the original on **any** judge input (§3 step 2; `KIR_SPEC.md` §7). Hidden tests follow the sample `.in`
patterns and workloads with different parameters, not duplicates. Consequences:

1. **Correctness first.** A wrong output loses a whole program in Success Rate; a missed
   optimisation only costs cycles. On any doubt, emit the safer candidate (section 5).
2. **Never crash, never time out.** A top-level catch-all writes the input; an internal deadline
   stops optimising well inside the limit (section 5).
3. **Average Cycles is an arithmetic mean of absolute cycles, so long-running programs dominate.**
   Measured on the bench (`review_experiments/t15_cycle_shares.py`, unoptimised, sample inputs,
   all five machines): arrays 26–27 %, junk 21–23 %, loops 15–18 %, bits 7–10 % (≈ 78 % together);
   numbers 5–8 %, state 6–7 %, effects 4–5 %, pressure 3–6 %, fused 2–3 %, control 1 %, large 0–1 %,
   semantics 0 %. Ten programs are ≈ half the total (`matmul4`, `collatz`, `nested_invariant`,
   `transpose8`, `strength_ladder`, `sieve`, `spill_storm`, `bubble_sort`, `count_divisors`,
   `bounds_trap`/`junk_dead`). Work is prioritised by these shares — **except** that the hidden set
   also contains "some very large programs (about 100,000 instructions)" (rules §3), which the
   bench lacks: one such program executed straight through costs ≥ 100 000 × `Q` ≈ 200 000 cycles
   beyond the cache, as much as ≈ 200 typical bench programs. Under an average of absolute cycles a
   handful of them can outweigh everything else, so the large-program path is high priority even
   though the bench's `large` kind is 0–1 %.
4. **Inputs look like the samples**, so trip counts stay in the bench's masked ranges, and
   candidates are chosen by mean cycles on `.in`-like inputs, not by worst-case regret (4.9).

**Where the cycles go, per kind** (input code; from the law check against `time_trace` and the
measurements in section 10):

| Kind | Dominant cycle terms | Main attack in this plan |
|---|---|---|
| loops | LRU misses (11–22 names, K = 8), then fence + recurrence | LRU-aware naming (4.3), closed forms (4.5), unroll with remainder |
| arrays | `lat_load` chain per iteration, fence, `MP = 1` | latency rotation (4.6), interleave memory ops (L13), scalar replacement under range proofs |
| numbers | `lat_div` on the recurrence | lazy reduction / range-removed `mod`, peeled magic division, divisibility test (4.4) |
| bits | fences from per-bit branches, recurrence depth | if-conversion, GF(2) nibble tables (4.5) |
| state | taken jumps of dispatch chains, load latency | DFA threading, DTSP layout + latch duplication (4.7, 4.8), budget allocator |
| junk | issue slots, LRU misses | clean-up, DCE (dead names count, L5) |
| effects | little to win; `vload` latency, `MP` | L13 (vload/vstore are memory ops); never move a visible op |
| semantics | straight-line slots/latency; signed-division traps | sign-exact rules (L9, L10) |
| control | taken jumps | layout DP, node splitting |
| fused | `mul`→`mac` recurrence, loads | extraction by the machine's latencies (L14) |
| pressure | LRU misses almost entirely | LRU-aware naming + exact scheduling with LRU scored (4.3, 4.6) |
| large | `Q` per instruction beyond `C`, LRU (68–98 names) | clean-up + layout first, partial rerolling last (4.8) |


---

## 2. Where we stand out (the headline)

1. **Exact-model decisions everywhere** (section 0). Thresholds are not hard-coded. Each decision
   is a local exact measurement or a global candidate search.
2. **LRU-aware register naming** (L5, section 4.3). The literature has nothing for this model.
   If the whole text fits ≤ K names, do that; otherwise decide residency by Belady/MIN and make
   the hardware LRU agree with it (resident-dead or fresh names, schedule order, free `select`
   refresh touches), spilling to scratch or to a hardware miss on a latency-bound instruction,
   whichever is cheaper. This is original work, and it pays a lot on the `pressure`, `arrays` and
   `state` kinds and on machines with few registers.
3. **Fence-aware latency rotation** (L2+L3, section 4.6): single-stage pipelining (Rau 1994) where
   long-latency work (`load` 4, `mul` 3, `div` 12) from the next iteration is placed
   just before the back-edge jump, so its latency runs out during the fence.
4. **Loop algebra that deletes loops**: closed forms with chains of recurrences (matrix-power
   acceleration is LATER: trip counts in the bench are ≤ 63), lazy modular reduction, strength
   reduction of `div`/`mod` of induction variables (Amarasinghe, Lee, Greenwald, MIT TM-600;
   Sheldon 2001), and range-proven modular reduction (on `adler32` range facts remove the
   reduction entirely: 1525 → 708 cycles on the default machine).
5. **Layout and code budget as one search** (L3, L4, section 4.8). Block order by the Kiln's own
   taken-jump objective (directed TSP, exact for small CFGs), with latch duplication placed so
   every loop path pays one taken jump, and a global budget of *hot* instructions below `C`.
   Large straight-line code is cleaned up and laid out first; only the tail that still lands
   beyond `C` is **rolled back into loops** (RoLAG, CGO 2022).
6. **Versioning when a runtime fact unlocks speed**: trip count ≥ u, all addresses in
   range, divisor ≠ 0, no aliasing. A cheap check up front picks a fast copy; otherwise the
   original copy runs, so behaviour is identical by construction. The code-size cost is
   charged by the exact icache model.
7. **A verification stack beyond testing**: our own solver-free value-graph translation
   validator (Tristan, Govereau & Morrisett, PLDI 2011 style), plus differential fuzzing
   that copies the judges' own test recipe (rename, shuffle, change constants).
8. **Exact scheduling that scores register cost** (section 4.6). On real CPUs exact scheduling
   buys ≈ 3 % over list scheduling; here the LRU register file makes the order decide the
   misses, and the measured gap is 18 % at K = 8. Small regions are searched exactly with the
   full cost model.

---

## 3. Target architecture

```
parse ──► CFG + loop forest (irreducible → controlled node splitting)
      ──► SSA (pruned) with an explicit EFFECT CHAIN (side-effect skeleton)
      ──► analyses: SCCP × wrapped intervals × known bits × congruence,
                     chains of recurrences (IVs, trip counts), affine address model
      ──► mid-end on an ACYCLIC E-GRAPH of pure ops (aegraph, Cranelift-style):
             GVN, algebra, rule set (hand + mined), LICM/GCM, remat for free
      ──► loop layer: closed forms, acceleration, IV strength reduction, scalar
             replacement / store promotion, versioning, unroll(+remainder), reroll
      ──► control layer: cost-based if-conversion, superblocks / tail dup,
             DFA jump threading, unswitching
      ──► extraction (cost = Kiln latency + slots) ─► lowering (mac / addsh / shl)
      ──► global schedule: modulo scheduling for loops, B&B-optimal for small
             superblocks, list scheduling with exact lookahead otherwise
      ──► out of SSA (Boissinot parallel copies) + LRU-aware naming + WAW-aware colours
      ──► layout + budget (one loop): DTSP order (exact ≤ 14 blocks) + latch duplication
             + icache knapsack seed + exact local search + branch-sense choice
      ──► exact evaluation of candidates on an input ensemble ─► robust choice
      ──► self-check (translation validator + differential run) ─► write
```

Two design rules hold throughout:

* **Everything is deterministic and budgeted in work units**, never in wall-clock time
  (you already do this; keep it).
* **Every transformation is either proven by an analysis or protected by a runtime
  guard.** "Probably fine" doesn't exist.

---

## 4. The items, one by one

Each item gives the technique and its source, how it fits the Kiln, the verdict, which
cycle terms it attacks, which of the 12 kinds benefit, the risk, and the tests.

### 4.1 Front end and analyses

| Item | Source | Kiln fit | Verdict |
|---|---|---|---|
| Pruned SSA + dominators | Cytron et al.; Cooper-Harvey-Kennedy | Already done. Port it. | **DO** |
| **Explicit effect chain** (side-effect skeleton) | Cranelift aegraph (Fallin 2023–2026) | `load`/`store`/`div`/`mod`/`vload`/`vstore`/`trap` stay ordered on the skeleton; pure ops float freely. This is the clean way to make L10 impossible to break by accident. | **DO** |
| **SCCP** (optimistic, with unreachable code) | Wegman & Zadeck, TOPLAS 1991; Click & Cooper "Combining analyses" | Finds constants that alternating passes never find (loop-carried constants). | **DO** |
| **Wrapped intervals** | Navas, Schachte, Søndergaard, Stuckey, APLAS 2012; Gange et al. TOPLAS 2015 | KIR values are bit patterns that `lt`/`div` read as signed and addresses read as unsigned. Wrapped intervals are exact for this "signedness-agnostic" setting. Your current unsigned-only intervals lose precision when values cross 2^31. | **DO**, replacing the current domain |
| Known bits + congruence (stride) | LLVM KnownBits; Granger 1989 congruences | Proves `x & m == x`, alignment, "even", `i % 4 == 0` facts. Needed for `addsh` and mask removal. | **DO** |
| Relational facts (`i < n` with `n` symbolic) | ABCD (Bodík, Gupta, Sarkar, PLDI 2000); pentagons (Logozzo) | Proves `load base+i` is in range inside `for i < n` when `base + n <= 4096` is checked once. Feeds versioning and LICM of loads. | **DO (bounded)**: difference constraints only |
| Chains of recurrences / scalar evolution | Bachmann-Wang-Zima 1994; van Engelen 2001; LLVM SCEV | Gives every IV a closed form, gives trip counts, and is the base of 4.5. | **DO** |
| Affine address model + dependence tests | GCD / Banerjee tests (Allen & Kennedy textbook) | Addresses are `base + a*i + c`. Exact alias and dependence answers for array loops. | **DO** |
| Full polyhedral / Omega test | Feautrier; Pugh 1991 | Programs are small and loops simple. The affine tests above cover them. Too heavy. | **NO** |

Tests: an exhaustive soundness check for each abstract transfer function on 8-bit
analogues plus random 32-bit vectors (every interval must contain every concrete result),
and analysis-only regression tests.

### 4.2 Mid-end: redundancy, algebra and code motion

| Item | Source | Kiln fit | Verdict |
|---|---|---|---|
| **Acyclic e-graph for pure ops** (aegraph) | Fallin, EGRAPHS 2023; Tate et al. 2009; egg (Willsey 2021) | Removes the phase-ordering problem for algebra: `mul` vs `shl`, `mac` vs `mul+add`, Horner vs Estrin, reassociated sums. All stay as alternatives until extraction picks by Kiln cost. GVN, LICM and rematerialisation come out of the same structure for free. | **DO**: the core of the rewrite |
| Full equality saturation over the whole program | egg | Blows up, and extraction with schedule-dependent cost is NP-hard. | **NO**: bounded rounds only |
| **Extraction by Kiln cost** | E-graph extraction literature | Cost = critical-path latency under the machine's latencies + issue slots / W + register pressure. Greedy plus local search, then the exact schedule decides. Target: treewidth-exact extraction (Goharshady, Lam, Parreaux, OOPSLA 2024), since small programs give sparse e-graphs. | **DO** |
| Rewrite rules: hand-written ring identities | Hacker's Delight (Warren) | L9 makes most ring identities unconditional. Bitwise identities, shifts by `b & 31`, and signed div/mod rules need side conditions. | **DO** |
| **Mined rewrite rules (offline)** | Ruler (Nandi et al., OOPSLA 2021); Bansal & Aiken, ASPLOS 2006; Hydra (Mukherjee & Regehr, OOPSLA 2024) | Offline, enumerate all 1–3 instruction KIR terms, cluster them by behaviour on random vectors, and keep the shortest equal ones as candidate rules. **Each rule is then proven by our own ring normaliser or by hand** and baked into kopt as a table. No solver runs in kopt. | **LATER** (high value); ask the organisers about offline solver use (section 8) |
| GVN-PRE / lazy code motion | Knoop, Rüthing, Steffen PLDI 1992; VanDrunen & Hosking 2004 | Covers `partial_redundancy`. Only pure ops can be inserted on new paths; a trapping op only with a proof. | **DO** |
| Complete PRE by restructuring | Bodík, Gupta, Soffa PLDI 1998 | Covered by superblocks / tail duplication (4.7) under the icache budget. | Folded into 4.7 |
| Global code motion (schedule early / late) | Click PLDI 1995 | Sink pure ops into the branch that uses them (fewer executed instructions) and hoist out of loops. Placement is chosen by exact cost, not just "lowest loop depth". | **DO** |
| **Tree-height reduction / reassociation** | Baer & Bovet; GCC reassoc width (2011) | With W ≥ 2, a linear chain of n adds costs n cycles of latency, while a balanced tree costs log n. Legal by L9. The width comes from the machine file. | **DO** |
| **Memory**: store-to-load forwarding, redundant load removal, dead stores across blocks | Memory SSA (Chow et al. 1996) | You do this inside blocks and joins. Extend it across joins with memory phis. | **DO** |

### 4.3 Registers: the LRU model (original work)

Facts: the cost is zero if the program text has ≤ K names. Otherwise the hardware runs LRU, each
re-touch of an evicted name costs S serial cycles, and the first touch is free (L5).
Out-of-SSA copies cost issue slots, and reusing a name can cause WAW stalls (L6).

| Item | Source | Kiln fit | Verdict |
|---|---|---|---|
| Out of SSA with parallel copies + optimal sequentialisation | Boissinot et al., CGO 2009 | Fewer copies than Sreedhar method I, and provably correct. Each copy removed is a slot saved in a loop. | **DO** |
| SSA chordal colouring, MaxLive = colours needed | Hack & Goos 2006; Bouchez; Brisk | Tells exactly when ≤ K names is possible: if MaxLive ≤ K, register cost is zero with no spills. | **DO** |
| Aggressive coalescing | Grund & Hack (optimal coalescing); Bouchez | Removes `mov`s in loops. **Subordinate to L5**: never coalesce into an evicted name. | **DO** |
| **LRU-aware naming** | Paging theory, not colouring: Belady 1966 (MIN); Mattson et al. 1970 (stack distance); Braun & Hack CC 2009 (MIN on CFGs). Butts & Sohi ISCA 2004 is hardware background only | (1) If the whole text fits ≤ K names, do that. (2) Else decide residency per hot region by MIN on values. (3) Each evicted value takes the cheapest of rematerialise / scratch store+load (L15) / a hardware miss **placed on a latency-bound instruction**. (4) Name so the hardware LRU agrees with MIN: resident-dead reuse, fresh names, schedule order, and **refresh touches** (`select d, X, s, s` in place of `mov`/`const`, or `mov X, X`). Verify by exact LRU simulation. | **DO**: our edge. **Test:** miss vs scratch vs refresh |
| Belady-style spilling with loop-aware splitting | Braun & Hack, CC 2009 | When MaxLive > K inside one hot loop, choose what to evict by furthest next use. | **DO** |
| Rematerialisation | Briggs, Cooper, Torczon 1992; free from the aegraph | Recomputing a `const` or a cheap op beats holding a register. | **DO** |
| WAW-aware colour choice | Lam's modulo variable expansion idea | Among legal colours, pick the one whose last write finishes earliest under the current schedule (L6). | **DO** |
| Full combinatorial RA + scheduling | Unison (Castañeda Lozano et al.); survey CSUR 2019 | Optimal, but needs a CP solver and seconds per function. The rules ban solver libraries, and writing our own CP solver is out of scope. | **NO** (our own B&B for small regions in 4.6 covers the important part) |

Tests: an LRU oracle (our Rust port of the `kirlib` register model) to check that the
predicted miss count equals the simulated one; random programs with MaxLive in [K−2, K+6];
all five kit machines plus random K in 4..32.

### 4.4 Division, modulo and multiplication

| Item | Source | Kiln fit | Verdict |
|---|---|---|---|
| Division by constant, **range-restricted** | Granlund & Montgomery, PLDI 1994; Lemire, Bartlett, Kaser, Heliyon 2021 | No high-half multiply, so `x·m` must fit 32 bits: with the exactness condition `(m·c − 2^s)·x_max < 2^s` this works **only for `x_max ≲ 2^16`**. Verify each (m, s) by the inequality (measured max exact x: 2^15.6–2^16.5 for every c tried). Remainder as `x − q·c`. **When x only falls into range after a few iterations, peel them** (`digit_sum`: x < 2^24, three true `div`s bring it below 16 778, then magic). | **DO** |
| Signed division by constant | Granlund & Montgomery section on signed | Bias negatives (`x + ((x>>31 arith) & (c-1))`). KIR has only logical `shr`, so build the sign mask with `lt` + `sub`. Use it only when the cost model says it beats `lat_div`. | **DO** |
| Full 32-bit `x / c` with an emulated high multiply | Hacker's Delight `mulhu` from 16-bit halves | 4 `mul` + ~6 ALU, ~11 slots, latency ≈ `lat_mul + 4`. Pays only when `lat_div ≳ 2·lat_mul + 10` **and** the div is on a recurrence (L11). | **LATER**) |
| **Divisibility test `x % c == 0` with a low multiply** | Granlund & Montgomery §9; Hacker's Delight 10-17 | `x·c⁻¹ mod 2^32 ≤ ⌊(2^32−1)/c⌋` (unsigned compare built from `lt` by xor 0x80000000). `lat_mul + 2` instead of `lat_div + 1`. Non-negative x only. | **DO** |
| Exact division by an odd constant | Hacker's Delight 10-16 | One `mul` by the inverse mod 2^32. Used by closed forms. | **DO** |
| Remainder by direct computation | Lemire, Kaser, Kurz, SP&E 2019 | Needs a 2N-bit product. Not available. | **NO** |
| `x % m` → conditional subtract | Standard modular-arithmetic trick | If ranges prove `0 ≤ x < 2m`, then `x % m = select(x < m, x, x − m)`. That is 2–3 ALU ops instead of 12. It hits `sum_cubes_mod`, `adler32`, `fletcher16`, `powmod` after their adds. It needs a temporary: at K = 4 it measured *slower* than `mod` (6864 vs 4226), so choose it by exact cost **after** naming. | **DO** (model-chosen) |
| **Lazy modular reduction** | zlib Adler-32 NMAX trick | Legal iff: constant `m > 0`; every value the *original* reduces is proven non-negative, and every deferred value proven in **[0, 2^31)** (signed `mod`!); a `sub` is fine only when ranges prove its result non-negative; no deferred value observed (store, vstore, ret, address, compare, shift, div) before reduction; bound holds for the whole span (proven trip bound, chunking, or versioning). Turns a `lat_div+1` recurrence into `lat_alu`; ranges may remove the reduction entirely (`adler32`: `a` never reaches m). Measured on `adler32`: lazy 708 vs conditional subtract 948 vs `mod` 1525 cycles (default). Combine with rotation/unrolling: the floor becomes `lat_load` + fence. | **DO**) |
| `div`/`mod` of an induction variable | Amarasinghe, Lee, Greenwald, MIT LCS TM-600, 1999; Sheldon, M.Eng. thesis, MIT 2001 | `i / c` and `i % c` for `i += 1` become counters that step and wrap (`select`). Removes a `lat_div` op per iteration. | **DO** |
| `mul` by constant → shifts and adds | Classic | Use it only when it lowers latency or slots under this machine. The e-graph holds both forms. | **DO** |

### 4.5 Loop algebra: make loops cheaper or delete them

| Item | Source | Kiln fit | Verdict |
|---|---|---|---|
| Operator strength reduction of IVs | Cooper, Simpson, Vick, TOPLAS 2001 (OSR) | `i*c` → running add. `mul` latency 3 becomes add latency 1 on the loop-carried path. | **DO** |
| Linear-function test replacement + IV elimination | Classic (Dragon book; OSR paper) | Removes the counter when an address IV can drive the exit test. One fewer op per iteration. | **DO** |
| **Closed forms of polynomial recurrences** | Chains of recurrences; LLVM SCEV binomial evaluation | Trip count from the **signed** test: T = n if n >ₛ 0 else 0. Odd part of k! by inverse multiply; the factor 2 removed before multiplying: `C(n,2) = (n >> 1)·((n−1) \| 1)` is exact for all n. **Degree ≤ 3 only.** Measured break-even against the best unrolled loop: **n ≈ 3** (2–4 across machines), so with masked n emit the closed form without a *performance* guard (the semantic zero-trip handling, T = n if n >ₛ 0 else 0, is always part of the formula; checked for n = 0..69, 2^31, 2^32−1). Keep the loop if it has any trap, store or volatile. | **DO (bounded)** |
| **Matrix-power acceleration of affine recurrences** | Loop acceleration (Boigelot; Finkel & Leroux); Kincaid et al., POPL 2018 | Measured on fib (n = 0..63): fast doubling (64 instrs) 62 cycles flat vs an unroll-8 loop's 59.8 average — a wash; an ideal guard gains ≤ 18 % and needs both copies (114 instrs > C). It beats the *naive* loop 3×, so always compare against the best unrolled loop. Wins clearly only for large n. | **LATER**); build only if organisers confirm large trip counts |
| GF(2) matrix power at run time (xorshift) | Same idea over GF(2) | One 32×32 bit-matrix apply is ≥ 100 ALU ops; the loop body is 6. | **NO** |
| **Compile-time GF(2)-affine summarisation** of constant-trip bit loops | Linear algebra over GF(2); CRC theory (Sarwate 1988) | `shr/shl/xor/and`-const and "xor POLY if bit 0" are GF(2)-affine. Compose the 8 CRC bit steps at compile time into `(x >> 8) ^ L(x & 255)`; emit L as **one 16-entry nibble table in scratch** (16 `store`s once, applied twice per byte). Measured on `crc_bitwise`: 25 vs 42 cycles/byte for the best branch-free bitwise code (377 vs 2743 for the input, default); ties on `slow_memory`. The mask-xor tree loses on K ≤ 8 (≈ 18 names) — dropped. A dataflow analysis, not a fingerprint. | **DO (bounded)**, nibble table only, model-chosen; pending the organisers (section 8) |
| Constant-trip loops: evaluate at compile time | Partial evaluation | Already done for ≤ 64 rounds. Extend with a work budget, emit the final state as stores and constants, and let the cost model choose against the code size (L4). | **DO** |
| Loop-invariant code motion, including **trapping ops under proof or versioning** | Click GCM; Midkiff, Moreira, Snir (IBM Sys. J. 1998) | A `load`/`div` hoists if proven safe; otherwise version the loop on a cheap preheader check (address < 4096, divisor ≠ 0). The original loop stays as the slow path, so trap order is untouched. | **DO** |
| **Scalar replacement / register promotion** | Carr & Kennedy, SPE 1994; Lo, Chow et al., PLDI 1998 (SSAPRE) | `stencil3`, `conv_mac`, `prefix_sum`, `matvec8`: values loaded in iteration i are reused in i+1 from registers; a store to a fixed address is kept in a register and written once at exit. Memory must be exact when a trap fires, so promote stores only in loops proven trap-free (or in the versioned fast copy). | **DO** |
| Accumulator splitting for reductions | Chang, Mahlke et al. (ILP transforms, 1991) | Legal by L9 for `add`/`mul`/`xor`/`and`/`or`, and for min/max via `select`. It cuts the loop-carried chain when the chain latency (`mac` 3, `mul` 3) exceeds the per-iteration issue time. | **DO** |
| Unrolling **with a remainder loop** (one exit test per u iterations) | Classic; trip count from SCEV | Your current unroll keeps every exit test. With a known or computable trip count, test once per u iterations: saves u−1 compares and branches plus the fence (L3). u is chosen by exact cost under the icache limit `C`. | **DO** |
| Unroll-and-jam | Callahan, Carr, Kennedy | `matmul4`, `matvec8`: reuse loads across the outer loop. | **LATER** |
| Loop rotation / do-while form | Classic | Already present. Keep it. | **DO** |
| Loop unswitching | Classic | A loop-invariant branch inside a loop becomes two loops. It saves a branch per iteration, at an icache cost decided by the model. | **DO (bounded)** |
| Idiom recognition (popcount loop → SWAR popcount, bit reverse, parity fold, min/max/abs) | LLVM LoopIdiomRecognize (ctpop/ctlz); Hacker's Delight | Matched on the **dataflow**, so renamed registers and shuffled blocks don't matter. This is not program fingerprinting; it is what LLVM does. A 32-step loop becomes about 12 ALU ops. | **DO**, after confirming with the organisers (section 8) |
| Loop fusion / fission | Classic | Rare in these kinds. | **NO** |
| Vectorisation / SWAR packing of data | — | No SIMD in KIR, and packing lanes into 32 bits is unsafe with wrap-around. | **NO** |
| 256-entry CRC table built at run time | Sarwate 1988 | Measured 730 vs 377 (nibble table) at n = 15, 492 vs 41 at n = 1, even with setup after the hot loop. | **NO** |

### 4.6 Scheduling

| Item | Source | Kiln fit | Verdict |
|---|---|---|---|
| **Exact incremental cost evaluator** | Our port of section 10 | Every scheduling decision can be scored exactly, including the cross-block latency carry and the fence (L1, L2). This is the scheduler's inner loop. | **DO**: build first |
| List scheduling with exact lookahead | Classic critical-path list scheduling | The fallback for big blocks. **Cycle-driven**: pick the ready op that issues earliest under the exact evaluator, ties by critical path. A priority-only order measured 5× further from optimal (12.9 % vs 2.7 % mean gap). | **DO** |
| **Exact-scored local search over instruction order** (STOKE-style moves, restricted to reordering) | Schkufza, Sharma, Aiken, ASPLOS 2013 (search method only) | Start from the list schedule; move one instruction to a random legal position; keep the move if the exact cost (with naming re-applied) does not rise; fixed seed, work budget. Reordering is fully proven by the validator, so no solver is needed. Measured on random blocks: +13 % (15–23 instrs) and +48 % (25–34) over list scheduling at K = 8, +32–46 % at K = 4, 0 % at K = 16; on enumerable 8–13-instr blocks at K = 4 it reaches 2.3 % from optimal vs list's 76 %. Never worse (only accepts non-increasing cost). Gains saturate by 300–1000 evaluations. | **DO**, for every region above the B&B limit |
| **Optimal scheduling of small regions by branch and bound** | Shobaki & Wilken, MICRO 2004; Shobaki, Wilken, Heffernan, TACO 2009; Malik, McInnes, van Beek, IJAIT 2008 | The published machine model (issue width + latencies, in order) is the Kiln minus `MP`, WAW and LRU; 20–40 instructions is far inside the published envelope. Bounds: critical path, slots / W, memory ports (L13). WAW and LRU are scored exactly at the leaves. For ≤ ~12 instructions this *is* exact integrated allocation + scheduling. Measured on random 8-op blocks: list scheduling is 2.7 % off optimal without LRU cost (as in the literature) but **18 % off with K = 8** — the LRU term makes exact search worth much more here than on real CPUs. Node budget, list-schedule fallback. | **DO (bounded)**, high priority |
| **Fence-aware pipelining: latency rotation** | Rau, MICRO 1994; Rau, Schlansker, Tirumalai, MICRO 1992 (code schemas); Lam PLDI 1988; Llosa et al. PACT 1996; TI prolog/epilog collapsing | Move the next iteration's `load`/`mul` to just before the back-edge `br`; `min(lat, 1+P)` of its latency hides in the fence. This is **one stage**: code cost = one copy of the moved ops in the preheader (1–3 instructions), fine even at `C = 32`. The extra trailing op must be proven trap-free (address < 4096, divisor ≠ 0); never a `vload`. The same holds for the **prologue copy**, which runs even when the loop runs zero times: prove it trap-free for n = 0 too, or place it after the zero-trip guard. Use a fresh/resident-dead name for the early value. Full multi-stage IMS only when `lat > 1 + P + body`. **Which ops run early is chosen exactly**: enumerate every dependence-closed set of early ops (≤ ~31 sets for bench-sized bodies), score each with the exact evaluator, keep the best. Measured: 3–29 % better than the best of the fixed rule variants on 4 of 8 loop shapes (`div_sum` −20 %, `cube_sum` −11 %), never worse; the fixed "loads and muls early" rule alone can be *slower than no pipelining* (`dot`, K = 4: 5138 vs 4562), so it is only the fallback for bodies too large to enumerate. | **DO** (rotation, exact stage choice); **DO (bounded)** (multi-stage). **Test** vs unrolling |
| Superblock / trace scheduling | Fisher 1981; Hwu et al. 1993 | Schedule across fall-through branches. Side exits get compensation, and trapping ops never cross a branch without proof. | **DO** |
| Hyperblocks / if-conversion | Mahlke et al. MICRO 1992 | KIR has `select`, and if-conversion lets us schedule both sides together. See 4.7. | **DO** |

### 4.7 Control flow

| Item | Source | Kiln fit | Verdict |
|---|---|---|---|
| Jump threading, block merging, branch folding | Classic | Already done. Port it. | **DO** |
| **Cost-based if-conversion** | Mahlke hyperblocks | `select` replaces a branch when `(both sides' slots) < (taken-branch probability × (1+P) + one side's slots)`. The probability comes from profile + static estimate + exact measurement. | **DO** |
| **DFA jump threading** | LLVM DFAJumpThreading (2021, for CoreMark state machines) | The `state` kind: `dfa_dispatch`, `vending_fsm`, `mini_interp`. When the next state is a known constant on a path, jump straight to that state's code and skip the dispatch compares. Code growth is capped by the icache model. | **DO** |
| Tail duplication / superblock formation | Hwu et al. 1993 | Removes join points so values become constants per path (path-sensitive PRE à la Bodík). | **DO (bounded)** |
| Irreducible loops → reducible by controlled node splitting | Janssen & Corporaal, TOPLAS 1997; Unger & Mueller 2002 | The `control` kind has two-entry loops. Split the smallest node set so every loop optimisation applies. | **DO** |
| Static branch probabilities | Ball & Larus 1993; Wu & Larus MICRO 1994 (Dempster-Shafer combination) | Use when no profile is given. Combine with the profile as evidence, never as fact. | **DO** |

### 4.8 Layout and code size

| Item | Source | Kiln fit | Verdict |
|---|---|---|---|
| **Block ordering by the Kiln's own objective** | Pettis & Hansen, PLDI 1990; Young, Johnson, Karger, Smith, PLDI 1997 (layout as directed TSP); search machinery from Newell & Pupyrev, IEEE TC 2020 | The Kiln has **no** reward for short jumps, so Ext-TSP's objective does not apply: only fall-through vs taken, plus position vs `C`. Exact per input by simulation. **Held–Karp DP is exact for ≤ ~14 blocks** (97 of 100 bench programs; the exceptions are `dfa_dispatch`, `vending_fsm`, `unrolled_crc12`); above that, chain merging + exact-scored local search. **Latch duplication and its placement are part of the search** (L3): measured 674 vs 895 cycles (naive) and vs 703 (if-converted) on a 50/50 loop; if-conversion wins at P = 4. Sources add Calder & Grunwald, ASPLOS 1994. | **DO** |
| **Global code-size budget (multiple-choice knapsack)** | Scheifler 1977 and Arnold et al. 2000 (inlining as knapsack); Kellerer, Pferschy, Pisinger 2004 | One budget: **hot** instructions below `C` (cold code is moved past `C` by layout first — measured: relocating a 40-instruction prelude recovered 151 of 237 lost cycles). Every unroll / version / thread / peel / pipeline option is an item (hot size, estimated gain); one option per site. Gains are additive only while the hot set fits `C`; at the boundary they interact in both directions (measured ±60 %), so the MCKP DP is a **seed**, followed by exact-simulator local search to a fixed point (as arXiv 2608.20137 does for inlining). Layout and budget run as one loop. Replaces per-pass caps. | **DO** |
| **Icache knapsack** | Hot/cold splitting (BOLT, Panchenko et al. CGO 2019) | Fill the first `C` slots with the instructions that run most. Move setup and cold paths past `C`. The entry block must be index 0, so make it a tiny jump when that pays. | **DO** |
| Branch-sense choice | — | For each `br`, pick which successor falls through (invert or swap) to fit the layout. | **DO** |
| **Loop rerolling (partial)** | RoLAG (Rocha et al., CGO 2022); RollBin (Ge et al., LCTES 2022) — both report it as a size win that *costs* speed | Beyond-cache straight-line code costs `m × Q` (not `m × (Q + 1/W)`). Rolling wins only for code that would sit beyond `C`, with copies of ≥ ~3 instructions and affine per-copy differences; one table constant per copy needs ≥ ~5–6; two or more lose. Per-copy constants: IV-affine > packed literal (`shr`+`and`) > existing data memory > scratch table (one `store` *instruction* each). Order: clean-up to get under `C` (enough for both bench `large` programs at `C = 64`), layout, then roll only the tail beyond `C` and re-unroll to fit. Run after if-conversion. | **DO (bounded), high priority**: the bench's `large` kind is only 0–1 % of cycles (t15), but the hidden set includes ~100 000-instruction programs, each worth ≈ 200 typical programs under Average Cycles. **Test** none / partial / full on generated 100k programs (section 11, experiment 8) |

### 4.9 Candidate search and robustness

| Item | Source | Kiln fit | Verdict |
|---|---|---|---|
| Optimisation-space exploration over a small set of pipeline settings | Triantafyllis et al., CGO 2003 | You already do it. Keep it and widen it: more candidates fit the budget in Rust. | **DO** |
| **Input ensemble for measurement** | Organiser scoring (Average Cycles); Fisher & Freudenberger, ASPLOS 1992 | Two roles. *Cost:* the sample `.in` plus `.in`-like variants (same structure, parameters redrawn in the same ranges) — pick the candidate with the lowest **mean cycles** over them, which is exactly the organisers' metric, and reject any candidate whose mean is not strictly below the input's. *Correctness only:* trip counts 0, 1, 2 and the masked maximum, both extremes of each data-dependent branch, profile-contradicting paths. Minimax regret and per-member loss bounds are dropped: the score is an arithmetic mean of absolute cycles, so a 1-cycle loss at n = 0 does not matter, and strict per-member dominance measured unusable (it vetoed a 10.6× win on fib). | **DO**. **Test** ensemble width (section 11) |
| **Multiversioning on trip count** | — | When the best code depends on n (unroll depth, closed form vs loop, matrix power), emit `if n ≥ T: fast else: simple`. Measured cost: +1 cycle on the fall-through side, +4 (P = 2) / +6 (P = 4) on the taken side — put the common case on the fall-through. | **DO** |
| Robust profile use | Wu & Larus evidence combination | Profiles can be wrong on purpose. Blend them with the static estimate, check them against our own simulated counts, and drop a profile that contradicts the program's structure. | **DO** |
| Machine-learned phase ordering / LLM agents | Ashouri et al. survey (CSUR 2018); AutoPass (2026); Meta LLM Compiler | They need training data and a fuzzy cost model. We have an exact cost model, and search with an exact oracle beats prediction. They also add nondeterminism risk. | **NO** |
| Stochastic superoptimisation at run time | STOKE (Schkufza, Sharma, Aiken, ASPLOS 2013) | Without an SMT solver its outputs can't be proven, and test-only validation overfits (the STOKE authors and follow-ups note this). Law 1 forbids it. | **NO** (the offline, proven rule mining in 4.2 takes its place) |

---

## 5. Correctness: how we never lose a test case

A wrong output costs the whole test case, so correctness is a design pillar, not a
final check.

1. **By construction**: the effect skeleton (4.1) makes reordering a trapping or visible
   op structurally impossible without an explicit, proof-carrying move. Each proof
   (range, nonzero, no-alias) is a value attached to the move and logged in debug mode.
2. **Runtime guards instead of risky proofs**: versioning keeps the original code as the
   fallback, so whenever the check fails, behaviour is the original's.
3. **Our own translation validator** (Tristan, Govereau, Morrisett PLDI 2011; Necula PLDI
   2000; Pnueli et al. 1998). Honest coverage: *complete* for renaming, scheduling, layout,
   copy removal, the ring fragment (polynomial normal form), pure bitwise terms (truth table)
   and linear mixed boolean-arithmetic (0/1-input theorem); *sound but incomplete* for terms
   with `lt`/`shr`/`div`/`select`; *no coverage* of loop-restructuring passes (closed forms,
   rerolling, remainder loops, lazy reduction, threading). Those carry a **certificate** (the
   invariant used) re-checked by a separate small checker, then go to step 4.
4. **Differential execution**: mirror the kit generator exactly; harvest every program
   literal ±1 as inputs; **exhaust small masked domains** (`n & 63` → all 64 values; 63/100 bench programs have ≤ 2^16 argument values — a proof only if the program reads no memory and no `vin`);
   inverse execution of trap targets (`divisor = 0`, `address ≥ 4096`) through invertible ops;
   Korel/Angora-style descent on the distance to the trap for the rest. No solver.
5. **Fallback**: any doubt, error or budget overrun → write the input unchanged. Under the
   organisers' scoring a crash costs the case its unoptimised cycles and (we assume) a failure in
   Success Rate, so kopt wraps all work in a catch-all that writes the input, and stops
   optimising at an internal deadline of ≈ 2.5 s (≈ 25 s for 100k-instruction programs),
   start-up included.

Offline only:

* **A KIR random program generator** (Csmith-style: Yang et al., PLDI 2011) with traps,
  volatiles, irreducible loops, aliasing stores and high register pressure.
* **A "judge simulator"**: mutate every bench program exactly as the rules describe
  (renamed registers, shuffled blocks, changed numbers) plus EMI-style mutations (Le,
  Afshari, Su, PLDI 2014: insert code into paths that don't execute, delete dead code)
  and check kopt on all of them.
* **A random machine generator** across every key's legal range (W 1–8, K 4–64, P 0–10,
  C 8–4096, latencies 1–40, fused flags 0/1).
* **A 100k-instruction generator** for the 30 s limit, timed on a slow single core.

---

## 6. Knowing how close to optimal we are

"As close to optimal as possible" needs a ruler. Three of them:

1. **Lower bounds per program and machine**, computed offline: dynamic instruction count after ideal
   clean-up / W, plus the loop-carried critical path × trip count, plus `(1+P)` per
   unavoidable taken jump. The gap between kopt and the bound shows where the cycles are.
2. **Headroom runs**: the same kopt with 100× budget offline. If that is much better, the
   time limit costs us, and we spend effort on speed. If not, we spend it on new techniques.
3. **Reference points**: the kit's 15 hand-optimised examples. We should beat every one on
   every machine.

The dashboard (`tests/`) tracks cycles per program × machine, geomean per kind, max kopt
time, and the determinism check, on every commit.

---

## 7. Build order (by dependency, not by date)

Within each step, order the work by cycle share (section 1): arrays, junk, loops and bits first
(≈ 78 % of bench cycles), and the 100k-instruction path (clean-up, layout, partial rerolling in
linear time) right after them; `semantics` and `control` last.

1. **Infrastructure**: Rust IR and parser, an exact port of the cost model checked to be
   cycle-identical to `kirlib` on every bench program and machine, the validator, the
   fuzzer, the judge simulator and the machine generator. Nothing else is trusted until
   this exists.
2. **Core mid-end**: SSA + effect skeleton, SCCP × wrapped intervals × known bits, aegraph
   with GVN/LICM/remat, DCE, memory forwarding, GVN-PRE.
3. **Back end** (the cycles are measured here, so it comes before the fancy loops):
   Boissinot out-of-SSA, chordal colouring + coalescing, LRU-aware naming, exact
   evaluator, list + B&B scheduling + exact-scored local search over order, DTSP layout +
   latch duplication + budget.
4. **Loops**: SCEV, OSR + LFTR, unroll with remainder, accumulator splitting, scalar
   replacement / store promotion, versioning, latency rotation with exact stage choice.
5. **Control**: cost-based if-conversion, superblocks, DFA threading, node splitting,
   unswitching.
6. **Algebra**: division and modulo by constants (with peeling), IV div/mod, lazy modular
   reduction, closed forms, GF(2) nibble tables, idioms. (Matrix acceleration stays LATER.)
7. **Large programs**: linear-time-only mode, hot/cold splitting and partial rerolling (the
   ~100k-instruction hidden programs can dominate Average Cycles); build a 100k-program
   generator first.
8. **Selection**: `.in`-like variant ensemble, choice by mean cycles, do-no-harm on the mean,
   multiversioning.
9. **Offline / later**: rule mining (4.2), matrix acceleration, unroll-and-jam.

Each step lands with its unit tests, fuzz runs on all machines, and an updated `DESIGN.md`
section (the seven required headings).

---

## 8. Questions to send the organisers before building the grey-area items

Answered by the organisers:

1. ~~**Scoring**~~ — **answered**: Success Rate and Average Cycles (= total cycles / programs); a
   crash or error falls back to the unoptimised cycle count; incorrect code is penalised through
   Success Rate. Applied in sections 1, 4.9, 5, 7.
4. ~~**Cycle inputs**~~ — **answered**: hidden tests follow the sample `.in` patterns and workloads
   with different parameters, not duplicates. Applied in sections 1, 4.9.

Still open:

2. **Idiom recognition**: is replacing a computation recognised from its dataflow (for
   example a 32-step shift-and-count loop → SWAR popcount) allowed? It works on any
   program, not by name or hash, like LLVM's LoopIdiomRecognize.
3. **Offline solvers**: not asked — the rules ban SMT/SAT solvers, and mined rules are accepted
   only if provable by ring normal form, bitwise truth table or the linear-MBA theorem.
5. **Scratch region**: can judge inputs set words 3072–4095? (We won't rely on them being
   zero either way.)
6. **GF(2) summarisation**: is composing a constant-trip bit loop (CRC steps) into a linear
   form allowed?

Follow-ups on the scoring answer:

7. **Combining the two metrics**: how are Success Rate and Average Cycles combined into the
   ranking — Success Rate first and cycles as tie-break, a weighted sum, or something else? This
   sets how much cycle gain justifies any correctness risk.
8. **Incorrect output**: does an incorrect program also get the unoptimised cycle count? *Very
   likely yes, not proven:* the judges' pipeline validates before counting (rules §3 steps 2–3),
   and their unchanged `ksim` refuses programs without a valid `kvalidate` stamp. Low priority:
   the plan treats a wrong output as a lost case either way.
9. ~~**Time-outs**~~ — **answered by the rules**: a time-out fails the case (§3 step 1).
10. **Aggregation**: *correct* means matching on every judge input (answered: §3 step 2,
    `KIR_SPEC.md` §7). Still open: are cycles averaged over several hidden inputs per program or
    measured on one, and are all machine files weighted equally?

---

## 9. Practical notes for the Rust port

* The judge machine has **no network**, so any crate must be vendored in the repo (or use
  none). `cargo build --offline --release` inside `build.sh`, under 10 minutes.
* Keep the work-unit budget, not the clock (determinism, rule section 1).
* No `HashMap` iteration order in output paths (Rust's default hasher is randomly seeded,
  which breaks determinism). Use `BTreeMap`, or `HashMap` with a fixed hasher, sorted
  before any output-affecting loop.
* One core: no threads.
* **Nothing may abort the process.** A stack overflow or an out-of-memory abort in Rust cannot be
  caught by the catch-all, and a crash fails the case. So: no recursion over the IR (iterative
  worklists for DFS, dominators, SCCs, extraction), memory caps per pass proportional to program
  size, `panic = "unwind"` with `catch_unwind` around all optimisation, and on any caught panic
  write the input unchanged and exit 0. Test with generated 100 000-instruction programs, including
  deep CFGs and one long chain of dependent instructions.

---

## 10. Evidence ledger: why each key decision is what it is

Every row was checked against the exact cost model (`kit/kirlib.py`, `time_trace`). **Measured**
rows were run, with every variant's output checked equal to the original program; the scripts are
in `review_experiments/` and reproduce each number. **Derived** rows follow from reading the cost
model's code but were not run. Cycle counts are on `machine_default.cfg` unless stated.

| Decision (section) | Verdict | Evidence | Confidence | Flips if |
|---|---|---|---|---|
| Rerolling (4.8) | DO (bounded), high priority | Derived: beyond-`C` cost is `max(Q, wait)` per instruction, not `Q + 1/W`; rolling in-cache code always loses; both bench `large` programs fit `C = 64` after clean-up; bench `large` is 0–1 % of cycles (t15), but hidden ~100k-instruction programs cost ≈ 200 000 cycles each beyond `C` | high on the cost rule, medium on break-evens | hidden 100k programs are not repetitive → rerolling is dead code (clean-up and layout still apply) |
| LRU-aware naming (4.3) | DO | Derived (R1–R7): overwriting an evicted name pays `S`; fresh names are free; `S` overlaps waits; any touch refreshes; parameters beyond K start spilled; misses = stack distance > K | high on the rules, medium that MIN-then-steer is near-optimal | — |
| Latency rotation (4.6) | DO; multi-stage DO (bounded) | Derived from the fence rule: the fence delays issue, not results, so `min(lat, 1+P)` of a moved op hides. Measured (t14, 8 loop shapes × 5 machines): rotation with the early-op set chosen by the model beats no pipelining by 16–46 % on most shapes (`array_sum` 513 → 324, `dot` 1276 → 959, `div_sum` 1281 → 776, default) | high for single-stage; multi-stage not measured | `lat > 1 + P + body` on most hidden machines → multi-stage needed |
| Code-size budget (4.8) | DO | Measured: best unroll factor depends on *position* (beyond `C`, u = 16 wins; straddling `C` is worst); moving a 40-instruction cold prelude past the loop recovers 151 of 237 lost cycles; gains add up exactly when the hot set fits `C`, and differ by up to ±60 % at the boundary | high | hidden machines always have `C` ≥ program size → per-pass caps suffice |
| Block layout (4.8) | DO | Measured: taken jump costs `P` + unused slots (3, 2, 3, 2); latch duplication + rotated placement: 674 vs 895 (naive) vs 703 (if-converted); unrotated duplication only 770; if-conversion wins at P = 4. 97/100 bench programs ≤ 14 blocks | high | `state` programs much larger than 14 blocks → heuristic carries the kind that matters most |
| Magic division (4.4) | DO, with peeling | Measured: low-multiply magic is exact only up to x ≈ 2^15.6–2^16.5 for every divisor tried; `digit_sum` needs 3 peeled iterations | high | — |
| Divisibility test (4.4) | DO | Measured: exact on 200k random values + all small multiples for odd c | high | — |
| Lazy reduction (4.4) | DO | Measured `adler32`: lazy 708, conditional subtract 948, `mod` each step 1525 (2860 / 6864 / 4226 at K = 4) | high | hidden inputs unmasked → range proofs fail; version on n |
| Closed forms (4.5) | DO, unguarded | Measured `sum_sq`: closed form 13 cycles, beats the best unrolled loop from n ≈ 3 (2–4 across machines) | high | — |
| Matrix power (4.5) | LATER | Measured `fib`: fast doubling 62 flat vs unroll-8 loop 59.8 average (n = 0..63); ideal guard −18 % but 114 instructions | high for n ≤ 63 | hidden trip counts ≫ 63 → DO (bounded) |
| GF(2) nibble table (4.5) | DO (bounded), pending organisers | Measured `crc_bitwise`: 25 vs 42 cycles/byte (best bitwise), 377 vs 2743 for the whole input; ties on `slow_memory`; mask-xor tree 3× slower at K = 8; 256-entry table 730 | high | `lat_load ≳ 3·lat_alu + P` → bitwise ties or wins |
| B&B scheduling (4.6) | DO (bounded), high priority | Measured, random 8-op blocks: exact-lookahead list is 2.7 % off optimal with no LRU cost, **18 % off at K = 8**; priority-only order 12.9 % / 44.6 % | high that the gap exists | LRU-aware naming alone brings blocks under K → gap back to ≈ 3 % |
| Local search over order (4.6) | DO | Measured (`review_experiments/t13_local_search.py`, 30 blocks × 3 sizes × 3 machines): +13–48 % over list at K = 8, +32–46 % at K = 4, 0 % at K = 16; never worse; saturates by ~1000 evaluations | high that it never hurts; medium on magnitude (baseline naming was R1 only, not full MIN-then-steer) | full LRU-aware naming already removes the misses → gain falls toward 0 % (it still cannot hurt) |
| Exact stage choice in latency rotation (4.6) | DO | Measured (`t14_modulo.py`, 8 loop shapes × 5 machines; `few_registers` covers only 2 shapes): ties the model-chosen rule on most shapes, 3–29 % better on `div_sum`, `cube_sum`, `stencil3`, `array_sum`@K=4; ≤ 31 candidate sets | high | bodies far larger than the bench → enumeration too big; rule is the fallback |
| Validation (5) | DO | Derived + tested: linear-MBA zero test exact at width 32 (20 000/20 000); 63/100 bench programs have ≤ 2^16 argument values | high on the MBA claim; medium on trap search | — |
| Candidate selection (4.9) | DO, by mean cycles | Organiser answer: Average Cycles is an arithmetic mean of absolute cycles, hidden inputs follow the samples. Measured: strict per-input dominance vetoed a 10.6× faster `fib` for 1 cycle at n = 0; guard costs +1 fall-through, +4 / +6 taken | high | the organisers' ranking weights Success Rate so heavily that any measurable risk outweighs cycles (follow-up 7) |
| Priorities (7) | arrays, junk, loops, bits first | Measured (t15): those four kinds are ≈ 78 % of bench cycles on every machine; ten programs ≈ 50 %; `large` 0–1 %; but hidden ~100k-instruction programs (absent from the bench) can each outweigh ≈ 200 bench programs | high for the bench; medium for hidden tests | the count and shape of hidden 100k programs |
| Mid-end (4.2) | DO | Searches 2023–2026 found nothing that supersedes aegraphs + exact-model search; treewidth-exact extraction (OOPSLA 2024) added | medium-high | — |

## 11. Experiments to run first (they decide open parameters)

Ordered by how many decisions each one settles:

1. **Scheduling after naming.** B&B vs exact-lookahead list on the same blocks *after*
   LRU-aware naming, K ∈ {4, 8, 16}, block sizes 8 / 12 / 16 — sets the B&B region size and
   budget.
2. **Hardware miss vs scratch spill vs refresh**, per evicted value, over `S ∈ {1, 5, 8, 20}`,
   `lat_load ∈ {2, 4, 12}`, `MP ∈ {1, 2}` — decides the spill strategy in 4.3.
3. **Ensemble width.** *When:* with build step 8. *How much:* for every bench program, 20 variants
   of its `.in` (same structure, parameters redrawn within the ranges the sample and the program's
   masks imply); choose candidates on 10, score on the other 10, all five machines. *Adopt*
   variant-ensemble selection over sample-only selection if mean held-out cycles improve ≥ 1 %;
   otherwise select on the sample `.in` alone.
4. **Budget allocator**: MCKP seed + exact local search vs greedy gain/size vs per-pass caps, on
   programs whose hot set is within ±25 % of `C`.
5. **Layout**: Held–Karp vs chain-merge + 2-opt on `dfa_dispatch` / `vending_fsm`; rotated latch
   duplication vs if-conversion over P ∈ {0, 2, 4, 8}.
6. **Latency rotation** vs unroll-by-u vs both — rotation alone is measured (t14); its interaction with unrolling is not.
7. **Reductions**: lazy vs conditional subtract after naming, K ∈ {4, 8, 16}; peeled magic vs
   `div` on `digit_sum` over `lat_div ∈ {12, 20, 40}`; nibble table vs bitwise over
   `lat_load ∈ {2, 4, 8, 12}`.
8. **Rerolling**: none / partial / full + re-unroll on generated 100k programs, `C ∈ {32, 64, 256}`,
   `Q ∈ {1, 2, 8}`; per-copy constants by scratch table vs packed literal vs leave unrolled.
9. **Trap search**: inverse execution + descent vs the kit generator alone, on the `effects` kind
   with injected wrong optimisations.
10. **Instruction-replacement search (the other half of STOKE).** *When:* after the e-graph and its
    extraction work (build step 2) and the validator's ring / bitwise / linear-MBA deciders exist.
    *How much:* every pure straight-line region of ≤ 10 instructions in the 100 bench programs plus
    1000 generated ones, on all five kit machines; 20 000 proposals per region (opcode, operand and
    literal mutations), fixed seed; a candidate counts only if the validator *proves* it equal.
    *Adopt if:* it beats e-graph extraction by ≥ 2 cycles on ≥ 5 % of regions. Then feed the found
    sequences in as new rewrite rules (offline, proven) rather than searching at kopt time; at kopt
    time only if the rule form cannot capture them. *Otherwise:* drop it.
11. **Exact spill choice under LRU.** *When:* together with experiment 2, once LRU-aware naming and
    scratch spilling exist (build step 3). *How much:* every loop in the `pressure`, `arrays` and
    `loops` kinds plus 200 generated loops with K + 1 … K + 6 loop-carried values; machines with
    K ∈ {4, 8}, `S ∈ {5, 8, 20}`, `lat_load ∈ {4, 12}`, `MP ∈ {1, 2}`. For each loop, try every
    assignment of the excess values to {register, scratch} (≤ 2^6 = 64 combinations), scored
    exactly, against the plan's MIN-then-steer choice. *Adopt* exhaustive choice for loops with
    ≤ 6 excess values if it beats MIN-then-steer by ≥ 3 % on ≥ 10 % of loops; *otherwise* keep
    MIN-then-steer.

## 12. Open risks

1. How many ~100k-instruction programs the hidden set has and how repetitive they are — they can dominate Average Cycles, and decide how much rerolling is worth.
2. Hidden trip counts: the organisers say inputs follow the samples, so the masked ranges (≤ 63) are expected; fast doubling stays LATER.
3. How Success Rate and Average Cycles are combined (follow-up 7), and how cycles are aggregated over inputs and machines (follow-up 10).
4. Whether organisers accept GF(2) summarisation (measured 1.7×) and idiom recognition.
5. Scratch spill vs hardware miss on slow-memory machines (experiment 2).
6. Whether naming closes the 18 % scheduling gap or only part of it (experiment 1).
7. Whether `state`-kind hidden programs exceed ~14 blocks.
8. Rotation combined with unrolling, and multi-stage pipelining, are not measured (experiment 6).

## 13. Provenance

Built from PLAN.md (research-backed plan), REVIEW.md (two review passes: topics 1–3 and the law
check by the first reviewer; topics 4–12 redone with measurements, plus a second citation audit,
by the second reviewer), and the experiments in `review_experiments/`. Two review sub-tasks of
the original brief (Tasks 2 and 4) were never written and their brief is lost; nothing in this plan
depends on them. Papers were checked through search results and abstracts, not read in full;
references below that were not re-checked are well-known sources, and no verdict rests on them
alone.

---

## 14. References

Scheduling and registers
* Rau, "Iterative Modulo Scheduling", MICRO-27, 1994 — https://dx.doi.org/10.1145/192724.192731
* Lam, "Software Pipelining", PLDI 1988 — https://fg-kastens.cs.uni-paderborn.de/lehre/pgrekonf/seminar/Lam1988.pdf
* Swing Modulo Scheduling in GCC — https://ftp.icm.edu.pl/packages/gcc/summit/2004/Swing%20Modulo%20Scheduling.pdf
* Shobaki & Wilken, optimal superblock scheduling by enumeration, MICRO 2004; Shobaki, Wilken, Heffernan, optimal trace scheduling, ACM TACO 2009 — https://scholars.csus.edu/esploro/outputs/journalArticle/Optimal-trace-scheduling-using-enumeration/99257888392801671
* Shobaki, Bassett, Heffernan, Kerbow, CC 2022, pp. 41–53 — https://par.nsf.gov/servlets/purl/10379551 (not the ACO paper; title unconfirmed). ACO: Shobaki, Gordon, McHugh, Dubois, Kerbow, TACO 19(2), 2022
* Wilken, Liu, Heffernan, optimal instruction scheduling using integer programming, PLDI 2000
* Malik, McInnes, van Beek, optimal basic-block scheduling by CP, IJAIT 2008
* Belady, IBM Systems Journal 1966; Mattson, Gecsei, Slutz, Traiger, IBM Systems Journal 9(2), 1970 — https://researchr.org/publication/MattsonGST70
* Rau, Schlansker, Tirumalai, code generation schemas for modulo scheduled loops, MICRO 1992
* Castañeda Lozano & Schulte, survey on combinatorial RA + scheduling, CSUR 2019 — https://arxiv.org/pdf/1409.7628
* Hack & Goos, SSA register allocation — https://compilers.cs.uni-saarland.de/projects/ssara/
* Braun & Hack, spilling and live-range splitting for SSA, CC 2009 — https://i44vm2.informatik.kit.edu/uploads/publikationen/braun09cc.pdf
* Boissinot et al., out-of-SSA, CGO 2009 — https://hal-univ-tlse3.archives-ouvertes.fr/INRIA/inria-00349925v3
* Butts & Sohi, use-based register caching, ISCA 2004 — https://research.cs.wisc.edu/mscalar/papers/2004/isca04-use-based.pdf
* Hwu et al., the superblock, 1993 — https://web.eecs.umich.edu/~mahlke/courses/583w23/reading/hwu_jsuper93.pdf
* Mahlke, Chen, Gyllenhaal, Hwu, "Compiler code transformations for superscalar-based high-performance systems", Supercomputing 1992, pp. 808–817 (accumulator expansion, tree-height reduction) — https://web.eecs.umich.edu/~mahlke/courses/583f21/reading/mahlke_super92.pdf

Mid-end
* Fallin, ægraphs (EGRAPHS 2023) — https://cfallin.org/pubs/egraphs2023_aegraphs_slides.pdf ; 2026 blog — https://www.cfallin.org/blog/2026/04/09/aegraph
* Goharshady, Lam, Parreaux, fast and optimal extraction for sparse e-graphs, OOPSLA 2024 — https://cse.hkust.edu.hk/~parreaux/publication/oopsla24a/
* Phase ordering and equality saturation (UW PLSE) — https://uwplse.org/2025/04/14/Phase-ordering.html
* Nandi et al., Ruler, OOPSLA 2021 — https://www.cl.cam.ac.uk/~ey204/teaching/ACS/R244_2022_2023/papers/NANDI_OOPSLA_2021.pdf
* Bansal & Aiken, automatic peephole superoptimizers, ASPLOS 2006 — https://www.cse.iitm.ac.in/~krishna/courses/2021/odd-cs6013/BansalAikenASPLOS06.pdf
* Mukherjee & Regehr, Hydra, OOPSLA 2024 — https://users.cs.utah.edu/~regehr/generalization-oopsla24.pdf
* Click, GCM/GVN, PLDI 1995 — https://courses.cs.washington.edu/courses/cse501/04wi/papers/click-pldi95.pdf
* Wegman & Zadeck, SCCP — https://www.cs.utexas.edu/~pingali/CS380C/2007fa/papers/p291-wegman.pdf
* Knoop, Rüthing, Steffen, lazy code motion — https://cgi.cse.unsw.edu.au/~cs4133/Papers/LCM.toplas94.pdf
* Bodík, Gupta, Soffa, complete removal of redundant computations — https://www.cs.ucr.edu/~gupta/research/Publications/Comp/cpre.pdf
* Navas et al., wrapped intervals — https://people.eng.unimelb.edu.au/schachte/papers/aplas12.pdf ; Gange et al. TOPLAS — https://jorgenavas.github.io/papers/ACM-TOPLAS-wrapped.pdf
* GCC reassociation width patch (Enkovich, PR 44382, 2011) — https://gcc.gnu.org/ml/gcc-patches/2011-07/msg01493.html

Loops and arithmetic
* Cooper, Simpson, Vick, operator strength reduction — https://cgi.cse.unsw.edu.au/~cs4133/Papers/OSR.toplas01.pdf
* Amarasinghe, Lee, Greenwald, strength reduction of integer division and modulo operations, MIT LCS TM-600, 1999 — https://commit.csail.mit.edu/papers/1999/mdopt-TM.pdf ; Sheldon thesis — https://dspace.mit.edu/handle/1721.1/86849
* Zima, chains of recurrences — https://bohr.wlu.ca/ezima/papers/ISSAC95_p42-zima.pdf
* How LLVM optimises power sums — http://kristerw.blogspot.com/2019/04/how-llvm-optimizes-geometric-sums.html
* Scalar evolution intro — https://sbaziotis.com/compilers/introduction-to-scalar-evolution.html
* Carr & Kennedy, scalar replacement — https://digitalcommons.mtu.edu/michigantech-p/3919
* Lo, Chow et al., register promotion by sparse PRE (patent text) — https://patents.google.com/patent/US6128775
* Granlund & Montgomery, PLDI 1994 — https://gmplib.org/%7Etege/divcnst-pldi94.pdf ; Lemire, Bartlett, Kaser, optimal bounds, Heliyon 2021 — https://arxiv.org/pdf/2012.12369v1
* Lemire et al., faster remainder — https://arxiv.org/abs/1902.01961v3
* LLVM LoopIdiomRecognize (popcount/ctlz) — https://reviews.llvm.org/D45173
* Loop versioning (LLVM) — https://code.woboq.org/llvm/llvm/lib/Transforms/Utils/LoopVersioning.cpp.html

Control and layout
* LLVM DFA jump threading — https://llvm.org/doxygen/DFAJumpThreading_8cpp_source.html ; commit — https://lists.llvm.org/pipermail/llvm-commits/Week-of-Mon-20210726/942312.html
* Janssen & Corporaal, controlled node splitting — https://www.cs.tufts.edu/~nr/cs257/archive/johan-jansson/node-splitting.pdf
* Newell & Pupyrev, IEEE TC 2020 (search machinery only) — https://arxiv.org/pdf/1809.04676v2.pdf ; Pettis & Hansen, PLDI 1990; Calder & Grunwald, ASPLOS 1994; Young, Johnson, Karger, Smith, PLDI 1997; Mestre, Pupyrev, Umboh, ISAAC 2021
* Rocha, Petoumenos, Franke, Bhatotia, O'Boyle, RoLAG, CGO 2022 — https://dse.in.tum.de/wp-content/uploads/2025/02/Loop-Rolling-CGO-2022.pdf ; Ge et al., RollBin, LCTES 2022 — https://dl.acm.org/doi/abs/10.1145/3519941.3535072
* Ball & Larus, branch prediction for free — https://klasses.cs.uchicago.edu/archive/2017/fall/32001-1/papers/ball-larus-branch-predict.pdf ; Wu & Larus — https://klasses.cs.uchicago.edu/archive/2017/fall/32001-1/papers/wu-larus-profiling.pdf

Search, validation, testing
* Triantafyllis et al., optimisation-space exploration, CGO 2003 — https://webdocs.cs.ualberta.ca/~amaral/cdol/papers/triantafyllis03.pdf
* Ashouri et al., ML compiler autotuning survey — https://arxiv.org/pdf/1801.04405
* Schkufza, Sharma, Aiken, STOKE — https://arxiv.org/abs/1211.0557
* Tristan, Govereau, Morrisett, value-graph translation validation, PLDI 2011 — https://dash.harvard.edu/bitstream/1/4762396/1/pldi84-tristan.pdf
* Le, Afshari, Su, EMI, PLDI 2014 — https://www.cs.ucdavis.edu/~su/publications/emi.pdf
* AutoPass (Li, Ren, Tang, Zheng, Wang, 2026; LLM agents) — https://www.alphaxiv.org/abs/2606.20373
* Ding et al., autotuning algorithmic choice for input sensitivity, PLDI 2015 — https://dspace.mit.edu/handle/1721.1/99720
* Code-size budget: Scheifler, CACM 1977; Arnold, Fink, Sarkar, Sweeney, DYNAMO 2000; Lokuciejewski & Marwedel, ECRTS 2009; arXiv 2608.20137 (2026)

---

# Part 2 — Results

## 2.1 Experiment index

Run from `review_experiments/`. Times are rough, on one core in Python.

| Script | Question it settles | Headline result | Command | Time |
|---|---|---|---|---|
| `kh.py` | harness: exact cycles and results via `kirlib` | — | `import kh` | — |
| `t4_unroll_position.py` | does the best unroll factor depend on code position? | yes: beyond `C`, u = 16 wins; straddling `C` is worst | `python3 t4_unroll_position.py` | < 10 s |
| `t4_cold_last_and_additivity.py` | is cold code free to move? are gains additive? | relocation recovers 151/237 cycles; additive only when the hot set fits `C` (±60 % at the boundary) | `python3 t4_cold_last_and_additivity.py` | < 30 s |
| `t5_layout.py` | taken-jump cost; latch duplication; vs if-conversion | `P` + unused slots; rotated duplication 674 vs 703 (if-conv) vs 895 | `python3 t5_layout.py` | < 10 s |
| `t6_adler32_reduction.py` | `mod` vs conditional subtract vs lazy | 1525 / 948 / 708 (default); cond. subtract loses at K = 4 | `python3 t6_adler32_reduction.py` | < 10 s |
| `t6_magic_and_divisibility.py` | range of low-multiply magic; divisibility test | exact up to 2^15.6–2^16.5; test exact for odd c | `python3 t6_magic_and_divisibility.py` | ~1 min |
| `t7_closed_forms.py` | closed form vs best loop for Σi² | closed form wins from n ≈ 3 | `python3 t7_closed_forms.py` | < 10 s |
| `t7_fib.py` | fast doubling vs unrolled loops | 62 flat vs 59.8 mean (unroll 8): a wash for n ≤ 63 | `python3 t7_fib.py` | < 30 s |
| `t8_bnb_vs_list.py` | list scheduling vs optimal order | 2.7 % gap (K = 64), 18.1 % (K = 8) | `python3 t8_bnb_vs_list.py`; `K=8 python3 t8_bnb_vs_list.py` | ~5 min each |
| `t9_validation.py` | linear-MBA 0/1 test at width 32; small domains | 20 000/20 000 agree; 63/100 programs ≤ 2^16 args | `python3 t9_validation.py` | < 1 min |
| `t10_selection.py` | selection rules; guard cost | strict do-no-harm vetoes a 10.6× win; guard +1 / +4 / +6 | `python3 t10_selection.py` | < 1 min |
| `t11_crc_gf2.py` | CRC: bitwise vs GF(2) tree vs nibble vs 256 table | nibble 25 vs bitwise 42 cycles/byte; 256-table 730 | `python3 t11_crc_gf2.py` | < 10 s |
| `t15_cycle_shares.py` | which programs and kinds dominate Average Cycles? | arrays, junk, loops, bits ≈ 78 %; ten programs ≈ 50 %; `large` 0–1 % | `python3 t15_cycle_shares.py` | < 1 min |
| `t13_local_search.py` | exact-scored local search over order vs list | +13–48 % at K = 8, +32–46 % at K = 4, 0 % at K = 16 | `python3 t13_local_search.py [machine]` | ~10–20 min per machine |
| `t14_modulo.py` | exact early-op choice vs the rotation rule | 3–29 % better on 4 of 8 loops; fixed rule can lose to none | `python3 t14_modulo.py [machine]` | ~2–10 min per machine |

The per-topic tables (topics 4–11) are in Part 3 next to their analysis.

## 2.2 Raw output: t13 — local search over instruction order

Random SSA blocks (add/xor/sub/or/mul/div/load), 30 per size, R1 naming on both sides; "gain" is
the mean cycle reduction of local search over cycle-driven list scheduling after 300 / 1000 / 3000
evaluations; "optimal" counts blocks at the exact optimum (enumerable blocks only).

```
default       K=8  blocks of 8-13 instrs: list+R1 -> local search gain: 300 evals 0.0%, 1000 evals 0.0%, 3000 evals 0.0%; improved 0/30
                     on the 27 blocks enumerable exactly: gap to optimal order: list 0.0% (optimal 27), local search 0.0% (optimal 27)
                     largest single-block gain 0.0%
default       K=8  blocks of 15-23 instrs: list+R1 -> local search gain: 300 evals 13.3%, 1000 evals 13.4%, 3000 evals 13.4%; improved 10/30
                     largest single-block gain 61.7%
default       K=8  blocks of 25-34 instrs: list+R1 -> local search gain: 300 evals 41.5%, 1000 evals 46.6%, 3000 evals 47.7%; improved 29/30
                     largest single-block gain 72.9%
few_registers K=4  blocks of 8-13 instrs: list+R1 -> local search gain: 300 evals 31.5%, 1000 evals 31.9%, 3000 evals 31.9%; improved 19/30
                     on the 27 blocks enumerable exactly: gap to optimal order: list 76.2% (optimal 11), local search 2.3% (optimal 22)
                     largest single-block gain 73.7%
few_registers K=4  blocks of 15-23 instrs: list+R1 -> local search gain: 300 evals 38.0%, 1000 evals 41.1%, 3000 evals 43.0%; improved 29/30
                     largest single-block gain 69.4%
few_registers K=4  blocks of 25-34 instrs: list+R1 -> local search gain: 300 evals 38.5%, 1000 evals 43.9%, 3000 evals 46.1%; improved 30/30
                     largest single-block gain 64.1%
wide_fast     K=16 blocks of 8-13 instrs: list+R1 -> local search gain: 300 evals 0.0%, 1000 evals 0.0%, 3000 evals 0.0%; improved 0/30
                     on the 27 blocks enumerable exactly: gap to optimal order: list 0.0% (optimal 27), local search 0.0% (optimal 27)
                     largest single-block gain 0.0%
wide_fast     K=16 blocks of 15-23 instrs: list+R1 -> local search gain: 300 evals 0.0%, 1000 evals 0.0%, 3000 evals 0.0%; improved 0/30
                     largest single-block gain 0.0%
wide_fast     K=16 blocks of 25-34 instrs: list+R1 -> local search gain: 300 evals 0.0%, 1000 evals 0.0%, 3000 evals 0.0%; improved 0/30
                     largest single-block gain 0.0%
```

## 2.3 Raw output: t14 — exact early-op choice in latency rotation

Cycles at n = 64. "none" = no pipelining; "fixed rule" = loads and muls early; "model-chosen rule"
= best of {none, loads only, fixed rule}; "exact" = best over every dependence-closed set of
early ops. Every variant gets the same ordering search; outputs and memory checked equal to the
un-pipelined loop for n = 0, 1, 2, 5, 64. The `few_registers` run was stopped after 2 shapes.

```
default      array_sum    none   513 | fixed rule   324 | model-chosen rule   324 (early: A0,L0) | exact   324 (early: A0,L0) | exact vs model-chosen rule +0.0% | 3 sets, orders exhaustive
default      dot          none  1276 | fixed rule  1301 | model-chosen rule   960 (early: A0,A1,L0,L1) | exact   959 (early: A0,A1,L1) | exact vs model-chosen rule -0.1% | 10 sets, orders exhaustive
default      stencil3     none  2979 | fixed rule  2950 | model-chosen rule  2950 (early: A0,A1,A2,L0,L1,L2) | exact  2860 (early: A0,A1,A2) | exact vs model-chosen rule -3.1% | 31 sets, orders sampled 2500
default      cube_sum     none   897 | fixed rule   713 | model-chosen rule   581 (early: A0,L0) | exact   519 (early: A0,L0,M1) | exact vs model-chosen rule -10.7% | 5 sets, orders exhaustive
default      count_above  none   577 | fixed rule   386 | model-chosen rule   386 (early: A0,L0) | exact   386 (early: A0,L0) | exact vs model-chosen rule +0.0% | 4 sets, orders exhaustive
default      prefix_store none   577 | fixed rule   388 | model-chosen rule   388 (early: A0,L0) | exact   388 (early: A0,L0) | exact vs model-chosen rule +0.0% | 3 sets, orders exhaustive
default      div_sum      none  1281 | fixed rule   965 | model-chosen rule   965 (early: A0,L0) | exact   776 (early: A0,D,L0) | exact vs model-chosen rule -19.6% | 4 sets, orders exhaustive
few_registers array_sum    none  2534 | fixed rule  2566 | model-chosen rule  2534 (early: -) | exact  2313 (early: A0) | exact vs model-chosen rule -8.7% | 3 sets, orders exhaustive
few_registers dot          none  4562 | fixed rule  5138 | model-chosen rule  4562 (early: -) | exact  4562 (early: -) | exact vs model-chosen rule +0.0% | 10 sets, orders exhaustive
no_fused     array_sum    none   513 | fixed rule   324 | model-chosen rule   324 (early: A0,L0) | exact   324 (early: A0,L0) | exact vs model-chosen rule +0.0% | 3 sets, orders exhaustive
no_fused     dot          none  1276 | fixed rule  1301 | model-chosen rule   960 (early: A0,A1,L0,L1) | exact   959 (early: A0,A1,L1) | exact vs model-chosen rule -0.1% | 10 sets, orders exhaustive
no_fused     stencil3     none  2979 | fixed rule  2950 | model-chosen rule  2950 (early: A0,A1,A2,L0,L1,L2) | exact  2860 (early: A0,A1,A2) | exact vs model-chosen rule -3.1% | 31 sets, orders sampled 2500
no_fused     cube_sum     none   897 | fixed rule   713 | model-chosen rule   581 (early: A0,L0) | exact   519 (early: A0,L0,M1) | exact vs model-chosen rule -10.7% | 5 sets, orders exhaustive
no_fused     count_above  none   577 | fixed rule   386 | model-chosen rule   386 (early: A0,L0) | exact   386 (early: A0,L0) | exact vs model-chosen rule +0.0% | 4 sets, orders exhaustive
no_fused     prefix_store none   577 | fixed rule   388 | model-chosen rule   388 (early: A0,L0) | exact   388 (early: A0,L0) | exact vs model-chosen rule +0.0% | 3 sets, orders exhaustive
no_fused     div_sum      none  1793 | fixed rule  1477 | model-chosen rule  1477 (early: A0,L0) | exact  1288 (early: A0,D,L0) | exact vs model-chosen rule -12.8% | 4 sets, orders exhaustive
slow_memory  array_sum    none  1025 | fixed rule   772 | model-chosen rule   772 (early: A0,L0) | exact   772 (early: A0,L0) | exact vs model-chosen rule +0.0% | 3 sets, orders exhaustive
slow_memory  dot          none  1725 | fixed rule  1373 | model-chosen rule  1157 (early: A0,A1,L0,L1) | exact  1157 (early: A0,A1,L0,L1) | exact vs model-chosen rule +0.0% | 10 sets, orders exhaustive
slow_memory  stencil3     none  3301 | fixed rule  3020 | model-chosen rule  3020 (early: A0,A1,A2,L0,L1,L2) | exact  2981 (early: A0,A1,A2,L0,L1,L2,S1,S2) | exact vs model-chosen rule -1.3% | 31 sets, orders sampled 2500
slow_memory  cube_sum     none  1665 | fixed rule  1363 | model-chosen rule  1097 (early: A0,L0) | exact  1097 (early: A0,L0) | exact vs model-chosen rule +0.0% | 5 sets, orders exhaustive
slow_memory  count_above  none  1089 | fixed rule   773 | model-chosen rule   773 (early: A0,L0) | exact   773 (early: A0,L0) | exact vs model-chosen rule +0.0% | 4 sets, orders exhaustive
slow_memory  prefix_store none  1089 | fixed rule   900 | model-chosen rule   900 (early: A0,L0) | exact   900 (early: A0,L0) | exact vs model-chosen rule +0.0% | 3 sets, orders exhaustive
slow_memory  div_sum      none  1793 | fixed rule   973 | model-chosen rule   973 (early: A0,L0) | exact   973 (early: A0,L0) | exact vs model-chosen rule +0.0% | 4 sets, orders exhaustive
wide_fast    array_sum    none   511 | fixed rule   447 | model-chosen rule   447 (early: A0,L0) | exact   447 (early: A0) | exact vs model-chosen rule +0.0% | 3 sets, orders exhaustive
wide_fast    dot          none   639 | fixed rule   514 | model-chosen rule   450 (early: A0,A1,L0,L1) | exact   450 (early: A0,A1,L0,L1) | exact vs model-chosen rule +0.0% | 10 sets, orders exhaustive
wide_fast    stencil3     none   639 | fixed rule   451 | model-chosen rule   451 (early: A0,A1,A2,L0,L1,L2) | exact   451 (early: A0,A1,A2,L0,L1,L2) | exact vs model-chosen rule +0.0% | 31 sets, orders sampled 2500
wide_fast    cube_sum     none   767 | fixed rule   643 | model-chosen rule   577 (early: A0,L0) | exact   513 (early: A0,L0,M1) | exact vs model-chosen rule -11.1% | 5 sets, orders exhaustive
wide_fast    count_above  none   575 | fixed rule   447 | model-chosen rule   447 (early: A0,L0) | exact   447 (early: A0,L0) | exact vs model-chosen rule +0.0% | 4 sets, orders exhaustive
wide_fast    prefix_store none   574 | fixed rule   448 | model-chosen rule   448 (early: A0,L0) | exact   448 (early: A0,L0) | exact vs model-chosen rule +0.0% | 3 sets, orders exhaustive
wide_fast    div_sum      none  1279 | fixed rule  1089 | model-chosen rule  1089 (early: A0,L0) | exact   774 (early: A0,D,L0) | exact vs model-chosen rule -28.9% | 4 sets, orders exhaustive
```

---

# Part 3 — The review record (REVIEW.md, both passes)

*Two passes: topics 1–3 and Tasks 1/3/5 by the first reviewer; topics 4–12 redone in full, a second
citation audit (Task 5b) and the coverage report by the second reviewer. See the coverage table at the end.*

Method: literature search topic by topic, then each result is re-derived against the exact
cycle model in `kit/kirlib.py` (`time_trace`). Nothing was run (no kopt, ksim or benchmarks).

**Access caveat.** This session could search the web but could not open paper PDFs (the sandbox
blocks most hosts). Where a claim rests on a paper I could only see through search snippets or
remember from before, it is tagged **[unread]**. Claims derived from `kirlib.py` are tagged
**[model]** and are the ones to trust most.

Confidence scale: **high** = follows from the code of the cost model or from a result I am sure
of; **medium** = sound reasoning, but magnitudes need measuring; **low** = a guess that must be
tested. Every "PIT A AGAINST B" line is an experiment I could not settle on paper.

Symbols: `W` issue_width, `MP` mem_ports, `K` registers, `S` reg_spill_penalty,
`P` taken_branch_penalty, `C` icache_size, `Q` icache_penalty.

---

## Topic 1 — Loop rerolling of straight-line code

### What the papers say

* **RoLAG** — Rocha, Petoumenos, Franke, Bhatotia, O'Boyle, "Loop Rolling for Code Size
  Reduction", CGO 2022, pp. 217–229, doi 10.1109/CGO53902.2022.9741256
  (https://www.research.ed.ac.uk/en/publications/loop-rolling-for-code-size-reduction/).
  Aligns SSA graphs bottom-up from *seed* instructions (stores, calls, reduction roots), builds an
  alignment graph, and emits a loop when a profitability estimate says the loop is smaller.
  It is not limited to contiguous copies: instructions may be rearranged. Reported: up to 88 KB
  saved on full programs, 22.4 % average on TSVC, and it fires orders of magnitude more often
  than LLVM's pass. **[unread]** From memory of the paper: per-copy values that differ are
  handled by node kind — an *arithmetic sequence* becomes a function of the induction variable,
  *all-constant* mismatches become a constant global array indexed by the IV, and *non-constant*
  mismatches are written to a stack array before the loop. It works inside one basic block.
  RoLAG is a size optimisation; the paper does not claim a speed-up.
* **RollBin** — Ge, Mo, Wu, Zhang, Lu, "RollBin: Reducing Code-Size via Loop Rerolling at Binary
  Level", LCTES 2022, doi 10.1145/3519941.3535072
  (https://dl.acm.org/doi/abs/10.1145/3519941.3535072). Finds unrolled loops in binaries, infers
  the unroll factor from regular memory-address patterns, rebuilds iterations with a dependence
  analysis. 1.7 % / 2.2 % average size reduction (up to 7.8 %). The authors state that rerolling
  "may adversely affect performance" and add a profile threshold (`RerollFraction`) so that hot
  code is *not* rerolled.
* **LLVM LoopReroll** — Finkel, 2013 (https://reviews.llvm.org/D1940). Only undoes partial
  unrolling of an existing loop; never enabled by default; RoLAG measures it as "hardly
  distinguishable from no rerolling". It was later deleted from LLVM (**[unread]**, from memory:
  removed in 2023–2024 as unmaintained).
* 2023–2026: I found no newer rolling technique. Neighbouring work is function merging by
  sequence alignment (Rocha et al., CGO 2019; HyFM, LCTES 2021) and global outlining
  (Lee et al., arXiv 2312.03214). None targets speed.

**The literature's own verdict is that rerolling costs speed.** The plan's claim that it *wins*
cycles is therefore a Kiln-specific claim and must stand on the cost model alone.

### What the Kiln model says [model]

The icache rule in `time_trace` is:

```python
F = ICP if pc >= IC else 0
...
t = last + F
if fence > t: t = fence
for r in srcs[pc]:
    if ready[r] > t: t = ready[r]
```

Three consequences the plan does not state:

1. **A beyond-cache instruction costs `max(Q, wait)` not `Q + 1/W`.** `t = last + Q` already puts
   it in a fresh cycle, so for `Q >= 1` beyond-cache code runs at exactly one instruction per `Q`
   cycles and `W` is irrelevant there. The plan's formula `m × (Q + 1/W)` in 4.8 is wrong; it is
   `m × Q` when nothing else waits.
2. **`Q` overlaps with latency waits and with the fence.** It is a `max`, not a sum. A
   beyond-cache instruction that would wait `>= Q` cycles anyway (on a `div`, a `load`, or as the
   first instruction after a taken jump when `1 + P >= Q`) pays **nothing**. So beyond-cache
   straight-line code whose critical path is already `>= Q` cycles per instruction is free.
3. **Per-copy constants in memory cost code, not only cycles.** `store` takes two source operands
   and both may be literals (`'store': 'ss'`), so a table entry is one instruction
   `store ADDR, VALUE` — but that is still one instruction *in the file* per constant. A table of
   n constants needs n stores (placed beyond `C`, run once, `Q` each, and they also contend for
   `MP`) plus one `load` (`lat_load`) per iteration inside the loop.

Break-even, for a block of n copies of a body of b instructions, all beyond the cache:

* straight-line cost ≈ `n × max(b × Q, L)` where L is the dependence latency of one copy;
* rolled, in cache ≈ `n × max(ceil((b + 2..3) / W), R, …) + n × (1 + P)·[fence not hidden] + setup`,
  where R is the loop-carried recurrence latency, `+2..3` is IV update, compare and branch, and
  the fence can be partly hidden by latency already in flight;
* table setup ≈ `k × n × max(Q, 1/MP)` for k non-affine constants per copy.

On the default machine (`Q=2, W=2, P=2`), b = 4, no table: straight ≈ 8n, rolled ≈ 3n + 3n = 6n
before unrolling the *rolled* loop by u to amortise the fence (≈ 3n + 3n/u). With one table
constant per copy: add 2n once, and a load in the body. So:

* **b ≥ ~3 with affine-only differences: rolling wins** once the code is beyond `C`, and wins
  more as `Q` grows.
* **One non-affine constant per copy: wins only for b ≳ 5–6.**
* **Two or more non-affine constants per small copy: loses.**
* **Code inside the cache: rolling always loses** (it adds the fence, the IV and the compare).
  The right shape is therefore **partial**: keep the first ≈ `C` instructions straight, roll only
  the tail that would land beyond `C`, and re-unroll the rolled loop to the largest u that fits.

Two cheaper ways to carry per-copy constants that no paper needs, because they follow from KIR:

* **Packed literal tables.** If the per-copy constants fit in w bits, `32/w` of them fit in one
  literal: `shr t, LIT, sh; and t, t, mask` (2 ALU ops, no memory, no `MP` contention, no setup).
* **Constants already in data memory.** In `unrolled_poly` the per-copy "constant" is the
  *address* 0..23, which is affine in the IV. No table at all.

And one thing that beats rolling outright on the bench's own `large` programs:

* `unrolled_poly` is 122 instructions only because of junk (`const`, `mov`). Cleaned it is
  24 × (`load` + `mac`) + `ret` = 49 < 64. **It does not need rolling on `C = 64`; it needs
  clean-up, then reassociation.** The Horner chain is the bottleneck (24 × `lat_mac`); splitting
  it into even/odd (Estrin-style) chains is legal in Z/2^32 and halves the critical path.
* `unrolled_crc12` is 12 identical if/else diamonds. If-converted, a step is
  `and; sub/neg; and POLY; shr; xor` ≈ 5 ops with a 3–4-cycle recurrence; 12 steps ≈ 60
  instructions — again inside `C = 64` without rolling. Rolling matters on `C = 32`, and on the
  100k programs.

RoLAG's single-block limit matters here: `unrolled_crc12` copies are *diamonds*, so rolling must
run **after if-conversion**, or must align whole single-entry/single-exit regions.

### Verdict

**DO (bounded)** — not "DO: biggest lever on `large`" as the plan has it. Order of attack on the
`large` kind: (1) clean-up to get under `C`; (2) layout so the hot/critical part is under `C`;
(3) partial rerolling of the beyond-cache tail, decided by exact simulation; (4) table constants
only via affine IV, packed literal, or existing data memory; a scratch table is the last resort.

Confidence: **high** that the plan's cost formula is wrong and that in-cache rolling loses;
**medium** on the break-even numbers; **low** on how repetitive the hidden 100k programs are.

**PIT AGAINST EACH OTHER:**
(a) no rolling vs partial rolling vs full rolling + re-unroll, on generated 100k programs with
`C ∈ {32, 64, 256}` and `Q ∈ {1, 2, 8}`;
(b) scratch table vs packed literal vs "leave those copies unrolled" for non-affine constants;
(c) roll-before-if-conversion (region alignment) vs roll-after-if-conversion.

---

## Topic 2 — Registers under a hardware LRU

### What the papers say

* **Belady, "A study of replacement algorithms for a virtual-storage computer", IBM Systems
  Journal 5(2), 1966.** MIN (evict the item whose next use is furthest) is optimal for demand
  paging when the reference string is fixed.
* **Mattson, Gecsei, Slutz, Traiger, "Evaluation techniques for storage hierarchies", IBM
  Systems Journal 9(2):78–117, 1970** (https://researchr.org/publication/MattsonGST70). LRU is a
  *stack algorithm*: a reference misses in a cache of size K **iff its LRU stack distance (number
  of distinct items touched since its last touch, itself included) exceeds K**. One pass gives
  the miss count for every K.
* **Sleator & Tarjan, CACM 1985** (from memory): LRU is K-competitive against MIN, and no
  deterministic online policy is better. So on a bad reference string LRU can be K times worse
  than MIN — the reference string is what we must control.
* **Braun & Hack, "Register Spilling and Live-Range Splitting for SSA-Form Programs", CC 2009,
  LNCS 5501** (https://rd.springer.com/chapter/10.1007/978-3-642-00722-4_13). Extends MIN from
  straight-line code to CFGs with loop-aware next-use distances.
* **Butts & Sohi, "Use-Based Register Caching with Decoupled Indexing", ISCA 2004, pp. 302–313,
  doi 10.1109/ISCA.2004.1310783.** A *hardware* policy: evict the value with the fewest predicted
  remaining uses, do not cache values consumed from bypass. Confirms that use-count/next-use
  beats LRU for register caches; says nothing about what a compiler should do when the policy
  is fixed LRU. The plan's citation is correct but it is only background.
* **Cascaval & Padua, "Estimating cache misses and locality using stack distances", ICS 2003**
  (https://dl.acm.org/doi/10.1145/782814.782836): compile-time stack-distance prediction for
  fully associative LRU — the same mathematics as our register file.
* **Scheduling for register need is NP-complete on DAGs** (Sethi 1975); Sethi–Ullman is optimal
  only on trees. Govindarajan et al., "Minimum register instruction sequencing…", IEEE TC 2003;
  Touati, "Register saturation in instruction level parallelism", IJPP 2005; and a 2023 CP-SAT
  formulation (arXiv 2303.06855) all need ILP/CP for optimality.
* **Rematerialisation**: Briggs, Cooper, Torczon, PLDI 1992; Colombet, Brandner, Darte,
  "Studying optimal spilling in the light of SSA", CASES 2011 / TACO 2015 — "rematerialization is
  extremely important" in optimal spilling.
* 2022–2026: I found nothing on compiling *for* an LRU-managed register file. The plan's claim
  "no prior work fits" holds; the right theory to import is paging, not graph colouring.

### What the Kiln model says [model]

```python
track = cp.nregs > K
...
for r in touch[pc]:                 # sources left to right, then the destination, each once
    if r in resident: resident.move_to_end(r)
    else:
        if r in spilled: F += SP; spilled.discard(r)
        if len(resident) >= K: spilled.add(resident.popitem(last=False)[0])
        resident[r] = True
t = last + F
```

Laws the plan missed (all **high** confidence, they are read straight off the code):

* **R1. A pure overwrite of an evicted name still pays `S`.** The destination is touched like a
  source. So "reuse names as much as possible" (classic colouring) is *wrong* here. The rule is:
  write a new value into (i) a **resident dead** name if one exists (free, and it wastes no
  slot), else (ii) a **never-used fresh** name (free, but evicts the LRU name), and never
  (iii) an evicted name. Names are unlimited (`v0..v999999`), so (ii) is always available in
  straight-line code.
* **R2. Misses are exactly the touches with stack distance > K** (Mattson). The miss count of
  any candidate naming/order is one linear pass; and one pass yields it for all K.
* **R3. `S` is added to `last`, then `max`-ed with operand readiness.** A miss on an instruction
  that would wait ≥ S cycles for a `load`/`div`/`mul` operand anyway costs nothing. Misses should
  be *steered onto latency-bound instructions*.
* **R4. LRU can be steered for free.** Any touch refreshes a name. `mov d, s` can be written
  `select d, X, s, s` and `const d, c` as `select d, X, c, c` (same latency class `lat_alu`, same
  one slot, same result for every X) — this touches X and keeps it resident. The only cost is a
  dependence on X being ready. So existing `mov`/`const` instructions are free *refresh
  carriers*, and an explicit `mov X, X` costs one issue slot (≤ 1/W cycle, often 0) against `S`
  cycles for the miss it prevents. With refreshes we can make the hardware LRU imitate Belady.
* **R5. Memory has no timing dependence.** A `store` has no destination and a later `load` of the
  same address just pays `lat_load` from its own issue. So a value kept in scratch costs
  one `store` slot + one `load` slot + `lat_load` on its dependence chain, limited by `MP`, and
  both use *literal* addresses, touching no extra name. Against `S` serial cycles this is nearly
  always cheaper when the reload can be issued `lat_load` early — except on a loop-carried
  recurrence, where `lat_load` is added to the recurrence every iteration.
* **R6. Parameters beyond K start spilled.** With `nparams > K` the first `nparams − K`
  parameters are evicted during the free initial touches and cost `S` at first use.
* **R7. N counts names in the text, including unreachable code and unused parameters.** Dead
  code elimination and unreachable-block removal are register optimisations.

### Best formulation

Exact joint optimisation (order × names × refreshes × scratch spills) contains minimum-register
scheduling, so it is NP-hard; do not chase it globally. The tractable decomposition:

1. **If the program can be written with ≤ K names in total, do that** (cost is exactly zero).
   SSA chordal colouring gives this iff MaxLive ≤ K after copy handling. On the whole text, not
   per loop.
2. Otherwise, per hot region, **decide residency by Belady/MIN on values** (Braun–Hack next-use
   distances on the CFG). Each value MIN evicts gets the cheapest of: rematerialise; scratch
   store/load (R5); or a hardware miss placed on a latency-bound instruction (R3).
3. **Name so that the hardware LRU agrees with MIN**: enforce stack distance ≤ K for every touch
   MIN wants to hit, using R1 (resident-dead reuse / fresh names), schedule order, and R4
   refreshes. Check by the exact LRU simulation on the trace ensemble.
4. Loops are the hard case because names recur every iteration (no fresh names). A loop body
   that touches more than K names pays at least one miss per excess name per iteration under
   LRU unless the excess values live in scratch. For `spill_storm` (13 loop-carried values,
   K = 8): ≥ 5 values to scratch = 10 memory ops per iteration ≈ 5 issue cycles at W = 2, versus
   ≥ 5 × S = 25 cycles of misses. Scratch wins on the default machine; on `MP = 1` with
   `lat_load = 12` the answer is not obvious.

### Verdict

**DO**, but re-founded: the plan's "allocate per region, ≤ K names per hot loop, `mov`s at the
boundary" is the right first approximation; R1, R3, R4 and R5 are missing and each changes
decisions. "Aggressive coalescing" and "WAW-aware colour choice" must be subordinate to R1.

Confidence: **high** on R1–R7; **medium** that MIN-then-steer is near-optimal; **low** on
scratch vs miss on slow-memory machines.

**PIT AGAINST EACH OTHER:**
(a) hardware miss vs scratch spill vs refresh-steering, per evicted value, over
`S ∈ {1, 5, 8, 20}`, `lat_load ∈ {2, 4, 12}`, `MP ∈ {1, 2}`;
(b) `select`-carried refresh vs explicit `mov X, X` vs none;
(c) region-local ≤ K naming vs whole-program MIN-then-steer on the `pressure` kind and on
`matmul4`/`vending_fsm` with K = 4.

---

> **Note on topics 3–12.** The first reviewer (Fable) wrote topic 3 in full and topics 4–12 in a
> short form after the session ran low. A second reviewer (Opus) redid **topics 4–12** with the
> same per-topic checklist (pin the question; three search passes; tag every claim; re-derive
> against `time_trace`; decide; write down; audit), and — unlike the first pass — **ran** the cost
> model: every **[derived]** number below is reproducible with `review_experiments/`. Tags:
> **[verified]** a search result or page confirmed it; **[unread]** from memory, not checked;
> **[derived]** computed with `kit/kirlib.py` (= **[model]** in topics 1–3).

## Topic 3 — Fence-aware software pipelining

**Papers.** Rau, "Iterative modulo scheduling: an algorithm for software pipelining loops",
MICRO-27, 1994, pp. 63–74 (verified). Rau, Schlansker, Tirumalai, "Code generation schemas for
modulo scheduled loops", MICRO-25, 1992 (verified) — prologue/epilogue schemas. Lam, PLDI 1988 —
modulo variable expansion [unread]. Llosa, González, Ayguadé, Valero, "Swing modulo scheduling: a
lifetime-sensitive approach", PACT 1996, pp. 80–86 (verified); Hagog & Zaks, GCC Summit 2004.
Code size: Granston et al. (TI), US patent application 2002/0112228, "Method for collapsing the
prolog and epilog of software pipelined loops", and the TI C6000 compiler guide: prolog/epilog are
up to `p−1` stages of length II and can be rolled back into the kernel when the extra iterations
are harmless.

**Kiln [model].** `fence = t + 1 + TB` only delays *issue*; `ready[]` keeps counting. With a
bottom-tested loop the period is about `max(ceil(n/W) + P, recurrence, in-iteration chain + P)`.
The only thing pipelining can remove is the *in-iteration chain* (e.g. `load` → use). Moving the
next iteration's long-latency op to just before the back-edge `br` hides `min(lat, 1+P)` of it.
That is **one stage**, and its code cost is one copy of the moved ops in the preheader — not
`p−1` stages. A trailing extra `load` is harmless **only if its address is proven < 4096**;
a `div` only if the divisor is proven non-zero; `vload` never. A second name is needed for the
early value (R1 in topic 2 says use a fresh/resident-dead one).

**Verdict: DO, but as "latency rotation" (single-stage pipelining). Full multi-stage IMS:
DO (bounded), only when `lat > 1 + P + body` (slow-memory machines).** With `C = 64` the single
stage costs 1–3 instructions, so the plan's worry is unfounded for stage count 2; it is real for
3+ stages. Confidence: **medium-high**.
**PIT:** rotation vs unroll-by-u (which also overlaps loads, but is bound by `MP`) vs both.

## Topic 4 — A shared code-size budget

*Redone in full (second reviewer). Tags: **[verified]** = a search result or opened page confirmed
it; **[unread]** = from memory, not checked; **[derived]** = computed with `kit/kirlib.py`
`time_trace` (the same as **[model]** above). Every [derived] number is reproducible with
`review_experiments/t4_*.py`.*

**Question.** Should code growth (unroll, version, thread, peel, pipeline) be allocated by one
global budget, and is a multiple-choice knapsack (MCKP) DP over `C` exact for it?
*Would change my mind:* gains that add up exactly across sites (then DP is exact), or cold code
that cannot be moved out of the way. *Ground truth:* `time_trace`.

### What the papers say

* **Scheifler, "An analysis of inline substitution for a structured programming language",
  CACM 20(9):647–654, 1977** **[verified]** (https://dblp.org/rec/journals/cacm/Scheifler77.html;
  free copy as MIT CSG Memo 139). Minimise run time subject to a size constraint, solved
  approximately; the reference list includes the 0/1-knapsack approximation literature. On CLU,
  > 90 % of calls removed with little growth. That it is literally "reduced to knapsack" I saw only
  in secondary course notes **[unread]**.
* **Arnold, Fink, Sarkar, Sweeney, "A comparative study of static and profile-based heuristics for
  inlining", DYNAMO 2000, SIGPLAN Notices 35(7):52–64, doi 10.1145/351397.351416** **[verified]**:
  a knapsack approximation used as a common meta-algorithm for all heuristics compared; up to
  1.68× speed-up with ≤ 10 % code growth (Jalapeño JVM). Author list: mirrors disagree; the
  IBM page lists these four.
* **Lokuciejewski & Marwedel, "Combining worst-case timing models, loop unrolling, and static
  loop analysis for WCET minimization", ECRTS 2009** **[verified]**
  (https://ls12-www.cs.tu-dortmund.de/daes/media/documents/publications/downloads/2009-ecrts.pdf):
  unroll factors chosen with exact trip counts and the aiT timing model, explicitly to avoid
  I-cache overflow and spill code. The closest prior work to "decide by the exact model".
* **Critic / successor:** *"Formal performance and compile time guarantees for compiler
  optimization heuristics"*, arXiv 2608.20137 (2026) **[verified, abstract only]**: inlining is at
  least as hard as knapsack *when sites are independent*, and the independence assumption is weak
  because one decision changes the cost of others; they use iterative local search to a fixed
  point instead.
* **Recent:** no paper found that solves unrolling + versioning + inlining jointly as an MCKP
  (two searches). LLVM/Intel use per-transformation thresholds **[verified]**.

**What they optimised:** real caches (capacity misses) or WCET. Not a *positional* cache.

### What the Kiln model says [derived]

`F = ICP if pc >= IC` charges by **file index of executed instructions**, so the budget is
"hot instructions below `C`", not "total size ≤ C". Measured on a 64-word sum loop, default
machine (`review_experiments/t4_*.py`):

| setting | u=1 | u=2 | u=4 | u=8 | u=16 |
|---|---|---|---|---|---|
| C=64, no prefix | 576 | 448 | 384 | 352 | **336** |
| C=64, 40 cold instrs before loop | 616 | 488 | **424** | 435 | 495 |
| C=32, no prefix | 576 | 448 | 384 | **352** | 427 |
| C=32, 40 cold instrs before loop | 889 | 729 | 649 | 609 | **589** |
| C=32, same 40 cold instrs moved *after* the loop (entry `jmp`s to them) | 642 | 517 | 459 | **438** | 513 |

1. **The best option at a site depends on where it lands**, not only on its size. Once a loop is
   wholly beyond `C`, *more* unrolling keeps winning (it amortises the fence and `Q` overlaps
   it); half in/half out is the worst place.
2. **Cold code is nearly free to move out of the way:** relocating the prelude cost two taken
   jumps once and recovered 151 of the 237 cycles lost (589 → 438). So before any knapsack,
   *layout* decides what competes for `C`.
3. **Gains are additive when everything fits, and not otherwise.** Two independent loops, every
   (u1, u2) ∈ {1,2,4,8}²: at C=64 the joint gain equals the sum of single gains for all 9 pairs;
   at C=32, 6 of 9 pairs differ, in **both** directions — (4,8) gains 309 vs 381 predicted,
   (8,8) gains 245 vs 154 predicted. The DP's error is concentrated exactly at the `C` boundary.

### Verdict

**DO (revised)**: a global allocator, but not "MCKP DP is exact". Formulate each option with its
*hot* size only (cold parts are placed beyond `C` by layout), seed with the MCKP DP, then run
exact-simulator local search to a fixed point (the arXiv 2608.20137 remedy). Layout (topic 5)
and the budget must be one loop, not two passes. Confidence: **high** on the positional effect
and non-additivity (measured); **medium** that DP-seed + local search is near-optimal.
*Flips if:* hidden machines always have `C` ≥ program size (then per-pass caps suffice).

**PIT:** MCKP seed + exact local search vs greedy gain/size vs per-pass caps, on programs where
the hot set is within ±25 % of `C` (the only regime where they differ).

## Topic 5 — Block layout objective

*Redone in full. Reproduce with `review_experiments/t5_layout.py`.*

**Question.** What objective and algorithm give the cycle-optimal block order on the Kiln, and is
an exact solver affordable? *Would change my mind:* a Kiln term that rewards jump distance (none
exists), or block counts too large for exact DP. *Ground truth:* `time_trace`.

### What the papers say

* **Pettis & Hansen, "Profile guided code positioning", PLDI 1990** **[unread]** — greedy chain
  merging by edge weight.
* **Calder & Grunwald, "Reducing branch costs via branch alignment", ASPLOS 1994, pp. 242–251,
  doi 10.1145/195473.195553** **[verified]** — reorder blocks to cut taken branches; ≈ 5 % average,
  up to 16 % on Alpha 21064. Objective: taken-branch cost, the Kiln's objective.
* **Young, Johnson, Karger, Smith, "Near-optimal intraprocedural branch alignment", PLDI 1997,
  pp. 183–193, doi 10.1145/258915.258932** **[verified]** — branch alignment as a directed TSP;
  DTSP local search often meets the lower bound; greedy is also close to optimal.
* **Newell & Pupyrev, "Improved basic block reordering", IEEE TC 69(12), 2020, arXiv 1809.04676**
  **[verified]** — Ext-TSP objective (rewards short forward/backward jumps for real i-caches);
  an exact MIP solves small functions, and the heuristic is optimal on > 98 % of real instances.
* **Critic/successor: Mestre, Pupyrev, Umboh, "On the Extended TSP problem", ISAAC 2021, LIPIcs
  42, arXiv 2107.07815** **[verified]** — Ext-TSP is APX-hard; (k+1)-approximation; PTAS on sparse
  graphs.
* **Recent:** LLVM PR 109711 (2024) adds a size-aware Ext-TSP mode **[verified]**; no 2023–2026
  exact-layout paper found (three searches).

**What they optimised:** taken branches (Calder, Young: the Kiln's objective), or i-cache
locality / distance (Ext-TSP: not the Kiln's).

### What the Kiln model says [derived]

* **L2 measured.** A taken `jmp` costs 3, 2, 3, 2 extra cycles as 0–3 instructions precede it
  (P = 2, W = 2): exactly `P` + the unused slot of its cycle. Confirmed.
* **No distance term:** `trace[idx+1] != pc+1` is the only test. So the objective is Young et al.'s
  DTSP (plus `C`), not Ext-TSP. Fable's correction stands.
* **Header duplication — the plan's claim is wrong as stated.** Loop with a 50/50 if/else in the
  body, 64 iterations, default machine (cycles):

  | layout | alternating | all-odd | all-even |
  |---|---|---|---|
  | naive (arm `jmp`s to shared latch) | 895 | 895 | 895 |
  | latch duplicated after each arm | 770 | 706 | 831 |
  | **duplicated + taken arm's copy placed right before the header** | **674** | 706 | **648** |
  | if-converted (`select`) | 703 | 703 | 703 |

  Duplication alone still leaves the arm reached by the *taken* `br` paying 2 taken jumps. Only the
  **rotated** placement — the taken arm and its latch copy sit just before the header, so its back
  edge falls through — gives exactly 1 taken jump on every path. That placement **beats
  if-conversion** on the default machine (674 vs 703); on `wide_fast` (P = 4) if-conversion wins
  (637 vs 802 for unrotated duplication). So "header duplication" is a *layout* choice that must
  be searched jointly with block order, not a separate pass.
* **Exact search is affordable.** Block counts in the 100 bench programs (before merging): median
  4, 97 of 100 have ≤ 14, the outliers are `dfa_dispatch` 29, `vending_fsm` 34, `unrolled_crc12` 37
  (the last collapses after if-conversion). Held–Karp at n = 14 is 2^14·14² ≈ 3.2 M steps.

### Verdict

**DO** — DTSP objective (Calder; Young et al.) + positional `C` term, Held–Karp exact for ≤ 14
blocks, chain-merge + exact-scored 2-opt above, **with latch duplication and its placement inside
the same search**. Use Ext-TSP's chain-merging machinery only. Confidence: **high** on the
objective and the duplication result (measured), **medium** that ≤ 14 holds on hidden programs
(the `state` kind is exactly where it doesn't). *Flips if:* `state`-kind hidden programs are
much bigger, then the heuristic path carries the kind that matters most.

**PIT:** Held–Karp vs chain-merge + 2-opt on `dfa_dispatch`/`vending_fsm`; rotated header
duplication vs if-conversion across P ∈ {0, 2, 4, 8}.

## Topic 6 — Division, modulo, lazy reduction

*Redone in full. Reproduce with `review_experiments/t6_*.py`.*

**Question.** Which `div`/`mod` replacements are exact on the Kiln (32-bit low multiply only,
signed `div`/`mod`), and which save cycles? *Would change my mind:* a 32-bit-only magic method
exact on all inputs (none exists), or `mod` latency hidden anyway. *Ground truth:* `time_trace`
plus exhaustive/random checks of the arithmetic against Python big integers.

### What the papers say

* **Granlund & Montgomery, "Division by invariant integers using multiplication", PLDI 1994,
  pp. 61–72, doi 10.1145/178243.178249** **[verified]** — unsigned and both signed roundings; all
  need the high half of an N×N product. The §9 exact-divisibility test I did not see in a snippet:
  **[unread]**, but checked arithmetically below.
* **Lemire, Bartlett, Kaser, "Integer division by constants: optimal bounds", Heliyon 2021,
  arXiv 2012.12369** **[verified link]**; **Lemire, Kaser, Kurz, "Faster remainder by direct
  computation", SP&E 2019, arXiv 1902.01961** **[verified link]** — needs a 2N-bit product.
* **Successor (2024): "Classic round-up variant of fast unsigned division by constants:
  algorithm and full proof", arXiv 2412.03680** **[verified]** — full proof; a simpler variant for
  dividends < 2^(N−1), but still uses the high word. Confirms there is no published low-word-only
  method for full-range dividends.
* **IV div/mod:** **Amarasinghe, Lee, Greenwald, "Strength reduction of integer division and
  modulo operations", MIT LCS TM-600, Nov 1999** **[verified]**
  (https://commit.csail.mit.edu/papers/1999/mdopt-TM.pdf); **Sheldon, M.Eng. thesis, MIT 2001,
  same title** **[verified]** (https://dspace.mit.edu/handle/1721.1/86849). **Correction to the
  first review:** it changed the plan's "Amarasinghe, Lee, Greenwald" to "Sheldon, Lee, Greenwald,
  Amarasinghe, LCPC 2001". The TM's author order is the *original* one. An LCPC 2001 version with
  Sheldon first may exist (I recall LNCS 2624) but two searches did not confirm it **[unread]**.
  Cite TM-600 + the thesis.
* **Lazy reduction:** zlib `adler32.c` NMAX = 5552, the largest n with
  `255·n(n+1)/2 + (n+1)(BASE−1) ≤ 2^32−1` **[verified]**; zlib-ng notes that tails and vector
  accumulators overflow if the bound is not re-derived per code shape **[verified]**. No compiler
  paper found that *proves* such bounds automatically (two searches).

### What the Kiln model says [derived]

* **Low-multiply magic range, computed exactly.** Best (m, s) with `x·m < 2^32` and
  `(x·m) >> s = ⌊x/c⌋`: maximum exact x is **2^15.6–2^16.5** for every c tried (3, 5, 7, 9, 10, 12,
  25, 100, 255, 1000, 46337, 65521, 1000003), each verified on 20k exhaustive + 20k random
  values. Fable's "x_max ≲ 2^16" holds.
* **Consequence on the bench:** `digit_sum` masks x to 24 bits, so `/10` is out of range for the
  first iterations — but after three true divisions x < 16 778. **Peel 3 iterations with `div`,
  then magic** (or a `lt`-versioned body). The plan does not have this pattern.
* **Divisibility test exact:** for odd c ∈ {3, 5, 7, 25, 65521}, `x·c⁻¹ mod 2^32 ≤ ⌊(2^32−1)/c⌋ ⇔
  x mod c = 0` (unsigned) on 200k random + all small multiples: no mismatch. KIR cost: `mul` +
  `xor` + `lt` (the limit's xor folds). Even c needs a rotate, which KIR lacks: `shr`+`shl`+`or`, 3
  more ALU ops.
* **adler32, measured** (63 bytes; same arithmetic with ≤ 8 names, so only reduction differs):

  | machine | `mod` each step | conditional subtract | lazy |
  |---|---|---|---|
  | default | 1525 | 948 | **708** |
  | few_registers (K = 4) | 4226 | **6864 (worse)** | **2860** |
  | no_fused (`lat_div` 20) | 2037 | 948 | **716** |
  | slow_memory | 2029 | 1452 | **1212** |
  | wide_fast | 1522 | 945 | **705** |

  With 13 names instead of 8 the *same* conditional subtract is slower than `mod` (4256 vs 2517
  on default): register naming swamps the arithmetic choice, as topic 2 predicts.
* **The range facts are stronger than the plan uses.** With `n & 63` and bytes `& 255`,
  `a ≤ 1 + 63·255 = 16066 < 65521`: the `mod` on `a` is **never needed at all** (not even lazily),
  and `b < 63·16066 < 2^31`, so one `mod` at exit is exact. All variants agree with the original
  on edge inputs (n = 0, 1, 2, 63, 2^32−1, unmasked memory, all-0xFFFFFFFF).
* **Fable's "13-cycle recurrence into 1" overstates the gain:** lazy runs ≈ 11 cycles/iteration;
  the floor is now `lat_load` + the back-edge fence, so lazy reduction must be combined with
  rotation/unrolling (topic 3) to reach its value.
* **Legality rule 3 ("never `sub`") is too strict.** Signed `mod` differs only on negative
  arguments. The exact condition is: every value the *original* reduces is proven non-negative (so
  its `mod` is the mathematical one), and every deferred value stays in [0, 2^31). A `sub` whose
  result the range analysis proves non-negative is then fine.

### Verdict

Range-restricted magic: **DO**, with **peeling until the range holds** (new). Divisibility test,
exact odd division: **DO**. Conditional subtract: **DO, but chosen by the exact model after
naming** (it loses at K = 4). Lazy reduction: **DO** (upgrade from DO (bounded)) — range analysis
often removes the reduction entirely; rule 3 relaxed as above. Emulated mulhu: **LATER** stands.
Confidence: **high** on all arithmetic (checked) and on the measured ranking; **medium** on
how often hidden programs give such tight ranges. *Flips if:* hidden inputs are unmasked (then
the range proofs fail and versioning on n is needed).

**PIT:** lazy vs conditional subtract after LRU-aware naming, K ∈ {4, 8, 16}; peeled-magic vs
`div` on `digit_sum` across `lat_div ∈ {12, 20, 40}`.

## Topic 7 — Closed forms and matrix-power acceleration

*Redone in full. Reproduce with `review_experiments/t7_*.py`.*

**Question.** For loops with masked trip counts (n ≤ 63), when does a polynomial closed form or a
matrix-power (fast doubling) form beat the *best* loop, on which machines? *Would change my
mind:* a measured break-even far from Fable's paper estimates. *Ground truth:* `time_trace`;
every variant checked equal to a Python reference for n = 0..69 and 2^31, 2^32−1.

### What the papers say

* **Chains of recurrences:** "Chains of recurrences — a method to expedite the evaluation of
  closed-form functions", ISSAC 1994 — title and 1994 date **[verified]** via later citations; the
  authors (Bachmann, Wang, Zima) **[unread]**. Zima, ISSAC 1995 (rational simplification)
  **[verified link]**.
* **Kincaid, Cyphert, Breck, Reps, "Non-linear reasoning for invariant synthesis", POPL 2018,
  PACMPL 2, art. 54** **[verified]** — extracts recurrences from loop bodies with branches and
  nested loops; solver avoids irrational closed forms. Successor: **Kincaid et al., "Closed forms
  for numerical loops", POPL 2019** **[verified]** (polynomials × exponentials over Q).
* **Loop acceleration: Frohn, "A calculus for modular loop acceleration", TACAS 2020, LNCS,
  pp. 58–76, arXiv 2001.01516** **[verified]** — **sole author**; the first review's "Frohn & Giesl
  et al." is wrong (Frohn & Giesl is FMCAD 2019, non-termination). Successor with Fuhs: arXiv
  2111.13952 **[verified]**.
* **Practice / limits:** LLVM IndVarSimplify exit-value replacement defaults to "only cheap"
  replacement and makes the loop dead when it only computed the exit value **[verified, source
  comments]**. SCEV binomial evaluation in W+T bits **[unread]**.
* **What they optimised:** verification (invariants, termination) — not cycles. None gives a
  cycle break-even; that has to come from our model.

### What the Kiln model says [derived]

* **Sum of squares, n = v0 & 63.** Closed form `(n−1)·n >> 1 · (2n−1) · 3⁻¹ mod 2^32` = 7 ops, 13
  cycles on default (10 on wide_fast, 19 on slow_memory). The best of two loops (plain `mac`
  loop; unroll-2 with split accumulators) runs ≈ 3 cycles per element, not 1.5–2. **The closed
  form wins from n = 3** on default, n = 2 on no_fused/wide_fast, n = 4 on slow_memory. Fable's
  "break-even n ≈ 7–9" is wrong by ≈ 3×; with n uniform in 0..63 no version guard is worth it —
  **always emit the closed form**.
* **Fibonacci.** Fast doubling over 6 bits, fully unrolled, branch-free with `select`: 64
  instructions, **62 cycles flat** (86 on slow_memory, 49 on wide_fast).

  | n | loop, unroll 2 | loop, unroll 8 + guarded tail | fast doubling |
  |---|---|---|---|
  | 10 | 34 | 29 | 62 |
  | 30 | 94 | 53 | 62 |
  | 63 | 190 | 101 | 62 |
  | **mean n = 0..63** | 97.2 | **59.8** | 62.0 |

  Against a naive loop fast doubling looks like 3×; **against a well-unrolled loop it is a wash on
  average** (59.8 vs 62.0). An ideal `n < T` guard choosing per n gives 49.2 (−18 %) but needs both
  copies: 114 instructions, past `C = 64`, so the guard's real gain is smaller still.
* **Lesson for the whole plan:** every break-even must be measured against the best competing
  *loop* after unrolling, never against the input program.

### Verdict

Polynomial closed forms (degree ≤ 3) and geometric sums: **DO**, unguarded when the trip count is
masked small (the closed form costs about as much as 3–4 iterations). Matrix power / fast
doubling: **LATER stands**, now measured: ≤ 18 % at ideal guarding on fib, worse for general
2×2. Confidence: **high** for the measured cases; **medium** for other recurrence shapes.
*Flips if:* hidden trip counts are large (n ≫ 63): then fast doubling's flat cost wins by a wide
margin and moves to DO (bounded).

**PIT:** closed form vs best unrolled loop for degree 2–3 sums on all five machines (done for
`sum_sq`); fast doubling vs unroll-8 only if organisers confirm large trip counts (question 4).

## Topic 8 — Optimal scheduling by branch and bound

*Redone in full. Reproduce with `review_experiments/t8_bnb_vs_list.py` (`K=8` env var for the
LRU run).*

**Question.** Does an exact scheduler earn its code over list scheduling at Kiln block sizes?
*Would change my mind:* list scheduling optimal on nearly all blocks (then B&B is dead code).
*Ground truth:* `time_trace`, exhaustive over all topological orders.

### What the papers say

* **Shobaki & Wilken, "Optimal superblock scheduling using enumeration", MICRO-37, 2004,
  pp. 283–293** **[verified]** — DFS + constraint propagation; with a 1 s limit solves ≈ 99 % of
  hard problems, 9 ms average; **2.7 % average improvement** in weighted makespan over the best
  heuristics (per the CSUR survey, arXiv 1409.7628 **[verified]**).
* **Shobaki, Wilken, Heffernan, "Optimal trace scheduling using enumeration", ACM TACO 2009**
  **[verified link]**.
* **Malik, McInnes, van Beek, IJAIT 17:37–54, 2008** (ICTAI 2006 conference version) **[verified]**
  — CP, blocks up to 2600 instructions, all but a handful of SPEC 2000 blocks optimal.
* **Wilken, Liu, Heffernan, "Optimal instruction scheduling using integer programming", PLDI
  2000, pp. 121–133, doi 10.1145/349299.349318** **[verified]** — all blocks up to 1000
  instructions, +14 % compile time.
* **Critic:** van Beek's group: list scheduling is non-optimal on ≤ 6 % of basic blocks but ≥ 40 %
  of superblocks **[verified, snippet]** — exact search matters on *larger regions*.
* **Recent: Shobaki, Gordon, McHugh, Dubois, Kerbow, "Register-pressure-aware instruction
  scheduling using ant colony optimization", ACM TACO 19(2), art. 23, 2022** **[verified]** — this
  settles the first review's flag: the 2022 TACO paper is **ACO, not B&B**. GPU follow-up: CGO
  2024 / TACO, doi 10.1145/3860328 **[verified]**. The plan's URL `par.nsf.gov/.../10379551` is
  still not confirmed to be this paper (the ACO record is 10379549).
* **What they optimised:** latency + issue width, register pressure as a *side* cost. None models
  an LRU register file.

### What the Kiln model says [derived]

Random 8-instruction SSA blocks (`add/xor/mul/load/div`), 40 blocks, default machine, optimum =
minimum over all topological orders by `time_trace`:

| scheduler | K = 64 (no LRU cost) | K = 8 (12 names → LRU misses) |
|---|---|---|
| priority-only topological order | optimal 13/40, mean gap 12.9 %, max 50 % | optimal 4/40, mean 44.6 %, max 200 % |
| **list scheduling with exact lookahead** (pick the ready op that issues earliest) | **optimal 33/40, mean 2.7 %, max 20 %** | optimal 20/40, mean 18.1 %, max 100 % |

* Without LRU the gap matches the literature (2.7 %). The scheduler must be *cycle-driven by the
  exact evaluator*: a priority-only order is 5× worse because in-order issue stalls everything
  behind a waiting op.
* **With LRU the heuristic falls apart**: half the blocks are non-optimal and the mean gap is
  18 %. The order decides stack distances, so order and naming are one problem (topic 2, R2/R4).
  The literature's "B&B gives ~3 %" does **not** transfer; on this machine it is the main lever on
  `pressure`-like code.

### Verdict

**DO (bounded) — upgraded in priority.** Exact search over orders, scored by the exact evaluator
*including LRU*, for regions ≤ ~10–12 instructions; exact-lookahead list scheduling (never a
priority-only order) above that, with the B&B node budget. Confidence: **high** that the gap is
real (measured), **medium** that it survives LRU-aware naming (better names may close part of
it). *Flips if:* naming alone (topic 2) brings blocks under K, then the gap drops back to ≈ 3 %.

**PIT:** B&B vs exact-lookahead list on the same blocks *after* LRU-aware naming, K ∈ {4, 8, 16};
block sizes 8, 12, 16 to find where B&B runs out of budget.

## Topic 9 — Solver-free validation and trap inputs

*Redone in full. Reproduce with `review_experiments/t9_validation.py`.*

**Question.** What can a solver-free validator prove for kopt's passes, and how close can
solver-free input generation get to covering traps? *Would change my mind:* a decision procedure
claim that fails a test, or bench programs whose input spaces are too big to exhaust.
*Ground truth:* the KIR interpreter in `kirlib.py`.

### What the papers say

* **Tristan, Govereau, Morrisett, "Evaluating value-graph translation validation for LLVM", PLDI
  2011, pp. 295–305, doi 10.1145/1993498.1993533** **[verified]** — normalising validator, no
  optimiser instrumentation, one run per pipeline; evaluated on GCC, perl, SQLite3. The
  false-alarm rate I did **not** find in a snippet **[unread]**.
* **Necula, "Translation validation for an optimizing compiler", PLDI 2000** **[unread]**.
* **Peggy / equality saturation for TV, Stepp, Tate, Lerner, CAV 2011** **[verified link]** — the
  closest solver-free design to an aegraph-based kopt.
* **Linear MBA: Zhou et al. 2007** (theorem) and **Liu et al., "MBA-Blast", USENIX Security
  2021** **[verified via secondary sources]**. **Critic: Reichenwallner & Meerwald-Stadler, SiMBA,
  2022** **[verified]** — the original theorem's statement (solvability of a linear system) is
  insufficient *for simplification* in general; they give a corrected condition.
* **Chen & Chen, "Angora: efficient fuzzing by principled search", IEEE S&P 2018, doi
  10.1109/SP.2018.00046, arXiv 1803.01307** **[verified]** — gradient descent on branch constraints
  beat random and magic-byte strategies on every program tested. **Korel, IEEE TSE 1990**
  **[unread]**.
* **Recent:** SMT remains the default (Alive2, smtgcc); Trivet (2026, Lean + LLMs) reports that
  SMT TV only gives bounded guarantees for loops **[verified]**. Nothing found that supersedes a
  normalising validator for our fragment.

### What the Kiln model says [derived]

* **Linear-MBA zero test is exact at fixed width 32.** If `e = Σ aᵢ·bᵢ(x)` with bitwise `bᵢ`, then
  `e(x) = Σₖ 2ᵏ · e₁(bit k of x)` mod 2^32, so `e ≡ 0` iff `e₁ ≡ 0 (mod 2^32)` on all 0/1 inputs.
  Tested: the 0/1 test agrees with random 32-bit testing on **20 000 / 20 000** random linear
  MBAs. SiMBA's correction concerns simplification, not this zero test; the plan's use is sound.
* **Small domains are common.** **63/100** bench programs have an *argument* space ≤ 2^16 values
  (masked parameters, or none); **59** are ≤ 2^8. Exhausting them is cheap — but many of them also
  read memory, so argument exhaustion is a proof **only** for programs that read no memory and no
  `vload` (otherwise it covers the control path for those arguments, not all memory states).
* Fable's coverage split (complete / sound-incomplete / no coverage → certificates) stands.

### Verdict

**DO** — stands. Add: the exhaustive mode must check that the program reads no memory/`vin` before
claiming a proof. Confidence: **high** on the MBA claim (derived + tested) and the domain count;
**medium** on the trap-search recipe (no measurement of how often it finds hidden traps).

**PIT:** inverse execution + descent vs the kit generator alone: count trap paths found on the
`effects` kind, with deliberately injected wrong optimisations (EMI-style mutants).

## Topic 10 — Choosing a candidate when inputs are unknown

*Redone in full. Reproduce with `review_experiments/t10_selection.py`.*

**Question.** Which selection rule should pick among candidates measured on an input ensemble,
and does a small ensemble choose like the full input distribution? *Would change my mind:* a rule
that rejects obvious wins, or an ensemble that picks differently from the full distribution.
*Ground truth:* `time_trace` on every n = 0..63 for the `fib` candidates of topic 7.

### What the papers say

* **Fisher & Freudenberger, "Predicting conditional branch directions from previous runs of a
  program", ASPLOS V, 1992, pp. 85–95** **[verified]** (DOI differs between indexes:
  10.1145/143365.143493 vs 10.1145/143371.143493 — check before citing) — profile-predicted
  branch directions transfer across runs; dataset choice affects accuracy.
* **Wu & Larus, MICRO 1994** **[unread]**.
* **Savage, "The theory of statistical decision", JASA 46:55–67, 1951** **[verified]** — minimax
  regret ("Savage criterion"); polynomial for discrete scenario sets, which is our case.
* **Critics/successors on input sensitivity:** **Ding et al., "Autotuning algorithmic choice for
  input sensitivity", PLDI 2015** **[verified]** — the best configuration varies with inputs;
  two-level input learning, up to 3× over one configuration for all inputs. **Berube & Amaral,
  SPEC workshop 2007** **[verified]** — two similar inputs may induce different transformations.
* **What they optimised:** average speed over a training set. None addresses "never lose on any
  hidden input", which is our scoring risk.

### What the Kiln model says [derived]

Candidates: the input `fib`, unroll-2, unroll-8, fast doubling (topic 7). Default machine:

| n | input | unroll 2 | unroll 8 | fast doubling |
|---|---|---|---|---|
| 0 | **9** | 10 | 23 | 62 |
| 1 | 24 | 10 | 23 | 62 |
| 63 | 2070 | 190 | 101 | 62 |
| mean 0..63 | 1030.8 | 97.2 | **59.8** | 62.0 |

* **The plan's filter "never slower than the input on any member" rejects every candidate** on all
  three machines tested: unroll-2 is 10.6× faster on average but **1 cycle slower at n = 0**, so it
  is vetoed and kopt would emit the input. A strict per-member dominance test is unusable.
* **Guard cost is not "≈ ½ cycle":** `lt` + `br` at entry costs **+1** cycle on the fall-through
  side and **+4** (default) / **+6** (wide_fast) on the taken side.
* The full distribution (64 values) and the 5-point ensemble {0, 1, 2, 31, 63} pick the same
  candidate under every rule here — weak evidence that the small ensemble is enough, for one
  program.

### Verdict

**DO, with the safety rule repaired.** Replace "never slower on any member" by a *bounded* loss:
allowed loss per member ≤ max(δ_abs, δ_rel · input cycles) (for example 2 cycles or 5 %), and a
strict aggregate gain; then minimax regret, tie-break by geometric mean. δ is set from the
scoring formula (question 1): per-test speed-up ratios punish small-n losses more than total
cycles do. Price guards at their measured cost (+1 / +P+…), not ½. Confidence: **high** that strict
dominance fails (measured); **medium** on the choice of δ until the scoring is known.

**PIT:** δ ∈ {0, 1 cycle, 2 %, 5 %} × {total, geomean, minimax regret}, scored by each candidate
scoring formula, on the bench with the kit generator's inputs as the "hidden" set.

## Topic 11 — Re-check of the NO verdicts

*Redone in full. Reproduce with `review_experiments/t11_crc_gf2.py`.*

**Question.** Does any NO (or LATER) verdict hide a real cycle win? *Would change my mind:* a
measured win for a NO item, or a measured loss for the GF(2) idea. *Ground truth:* `time_trace`;
all CRC variants checked equal to `crc_bitwise.kir`, including on unmasked memory words.

### What the papers say

* **Sasnauskas, Ketema, Chen, Lup, Collingbourne, Taneja, Regehr, "Souper: a synthesizing
  superoptimizer", arXiv 1711.04422, 2017** **[verified]** (seven authors; the first review gave no
  list) — SMT-based; Souper-optimised Clang 4.4 % smaller but ≈ 2 % slower.
* **Mukherjee & Regehr, "Hydra: generalizing peephole optimizations with program synthesis",
  PACMPL 8 OOPSLA1, art. 120, 2024, doi 10.1145/3649837** **[verified]** — generalises 75 % of
  LLVM's reported missed peepholes; every result formally verified (SMT). Successor: LPG (arXiv
  2603.18477, 2026, LLM-based) claims 36/40 vs Hydra's 23/40 **[verified, authors' own numbers]**.
* **Liu, Mada, Regehr, "Minotaur: a SIMD-oriented synthesizing superoptimizer", PACMPL 8 OOPSLA2,
  art. 326, 2024, doi 10.1145/3689766** **[verified]** — SIMD; 7.3 % average on GMP. Not relevant
  to KIR (no SIMD) beyond the method.
* **Schkufza, Sharma, Aiken, STOKE, ASPLOS 2013** **[verified link, arXiv 1211.0557]**.
* **Sarwate 1988** (table CRC) **[unread]**.
* All superoptimisers rely on SMT for correctness. Fable's solver-free route (ring normal form,
  truth table, linear MBA — the last now proven in topic 9) stands.

### What the Kiln model says [derived]

`crc_bitwise`, 15 bytes (n = 15), whole program in cycles; "per byte" in brackets:

| machine | input | branch-free bitwise | GF(2) mask-xor tree | **16-entry nibble table** | 256-entry table |
|---|---|---|---|---|---|
| default | 2743 | 632 (42) | 1885 (126) | **377 (25)** | 730 |
| wide_fast | 2146 | 629 (42) | 584 (39) | **306 (20)** | 680 |
| slow_memory | 2849 | 752 (50) | 2005 (134) | **737 (49)** | — |
| no_fused | 2744 | 980 (65) | 2353 (157) | **440 (29)** | — |

* **GF(2) summarisation pays — but only as the nibble table.** Built at run time with 16 `store`s
  of literal pairs into scratch, applied twice per byte: **1.7× over the best bitwise code** on
  default, 7.3× over the input, and still ahead at n = 1 (41 vs 492 cycles for the 256-table).
  On `slow_memory` (`lat_load` 12) it only ties bitwise — the model must choose.
* **The mask-xor tree is dead on K ≤ 8:** its eight parallel terms need ≈ 18 names; LRU misses make
  it 3× slower than bitwise. It barely wins even at W = 4, K = 16. Drop option (a) from the plan.
* **256-entry table: NO stands, measured** (730 vs 377 at n = 15, with its setup placed after the
  hot loop).
* Fable's estimate (12–14 vs 24–32 cycles/byte) had the ratio right (≈ 2×) but the absolute numbers
  low by ≈ 2×: the back-edge fence and load latency are in the per-byte cost.
* Other NO verdicts (run-time GF(2) matrix power, Unison-style CP, STOKE at run time, ML/LLM phase
  ordering) are not cycle questions; their reasons (code size, solver ban, nondeterminism) are
  correct and unchanged.

### Verdict

**GF(2) summarisation: LATER → DO (bounded), as the 16-entry nibble table only**, pending the
organisers' answer (question 6) — the measured gain is 1.7× on the `bits` kind's main program
shape, and it is a dataflow derivation (compose a constant-trip GF(2)-affine inner loop), not a
fingerprint. All NO verdicts **stand**. Confidence: **high** (measured, outputs checked).
*Flips if:* `lat_load` ≳ 3·`lat_alu` + P on a hidden machine (then bitwise ties or wins).

**PIT:** nibble table vs bitwise across `lat_load ∈ {2, 4, 8, 12}`; table setup placed inside vs
after the hot loop at `C ∈ {32, 64}`.

## Topic 12 — Newer work (2023–2026)

*Redone in full.*

**Question.** Has anything since 2023 superseded the plan's core choices (aegraph mid-end,
exact-model search, B&B scheduling)? *Would change my mind:* a newer technique that beats exact-model
search on a fully known machine.

* **Fallin, "ægraphs: acyclic e-graphs for efficient optimization in a production compiler",
  EGRAPHS 2023 invited talk** **[verified]**; Fallin's own advice is to cite the talk, as no paper
  exists. **The 2026 blog post (cfallin.org/blog/2026/04/09/aegraph) exists** **[verified]** — the
  first review's "UNVERIFIED" is lifted — plus a Dagstuhl talk, 8 Jan 2026 **[verified]**.
* **Goharshady, Lam, Parreaux, "Fast and optimal extraction for sparse equality graphs", PACMPL
  OOPSLA 2024, doi 10.1145/3689801, distinguished paper** **[verified]** — extraction is hard to
  approximate in general, exact and fast for low treewidth. Independent similar result by Sun,
  Zhang, Ni (UW), 2^O(w²)·poly time **[verified]**.
* **2025–2026 extraction work:** e-boost (arXiv 2508.13020, heuristics + exact solving)
  **[verified]**; an April 2026 preprint casting extraction as weighted pattern cover on AND-OR
  DAGs, generalising BURS **[verified, not peer-reviewed]** — the closest fit to "extraction =
  instruction selection by Kiln cost"; worth reading before building 4.2.
* **Scheduling:** Shobaki et al. ACO (TACO 2022) and its GPU-parallel successor (CGO 2024)
  **[verified]** — heuristics for large blocks; dominated by exact search at our sizes (topic 8).
* **AlphaDev (Mankowitz et al., Nature 2023)** **[unread]** and LLM optimisers (LPG 2026 above):
  training-based, nondeterministic: **NO** stands.

**Verdict.** Nothing supersedes the plan's choices. Add the treewidth-exact extraction as the
target for 4.2's "greedy plus local search" (small KIR programs give low-treewidth e-graphs).
Confidence: **medium-high** (several searches, no peer-reviewed 2025–2026 result that changes a
decision).

---

## Task 1 — PLAN §1 laws checked against `time_trace`

| Law | Verdict | Detail |
|---|---|---|
| L1 | correct | `t = last + F`, never `< last`. |
| L2 | **imprecise** | `fence = t + 1 + TB`. Without the jump the next instruction issues at `t` (free slot) or `t+1`. Cost = `P` + the unused slots of the jump's cycle, not a flat `1+P`. |
| L3 | correct, incomplete | ≥ 1 taken jump per iteration; a branchy body pays 2 on one arm unless the header is duplicated per arm. |
| L4 | **incomplete** | `F` is added to `last`, then `max`-ed with fence and operand readiness, so `Q` overlaps other waits. Cost per beyond-cache instruction is `max(Q, wait)`; `W` is irrelevant beyond `C` when `Q ≥ 1`. The first instruction after a taken jump is free if `Q ≤ 1+P`. |
| L5 | **incomplete / misleading** | See R1–R7 (topic 2): overwriting an evicted name costs `S`; fresh names are free; `S` overlaps waits; refresh touches steer the LRU; parameters beyond K start spilled. |
| L6 | correct | `if d >= 0 and ready[d] > t: t = ready[d]`. A fresh name has `ready = 0`. |
| L7 | correct | `trace[idx+1] != pc+1`. |
| L8 | correct | `return last + 1`. |
| L9 | correct | Add: `shl` by a constant is a ring multiply. |
| L10 | correct | `mem` lines may set any address < 4096, so scratch may be non-zero. |

**Laws the plan missed.**
* **M1. No functional-unit limits.** Only `W` and `MP`. Any number of `div`/`mul` overlap.
* **M2. Literals are free operands.** Every `s` operand may be a literal: always ready, touches
  no register (`srcs` keeps only slots `< nregs`). `store 3072, 5`, `ret 0`, `br 1, …` are
  legal. Almost every `const` can be deleted — saves a slot *and* a name.
* **M3. A blocked memory op drags the queue.** `if t == last and (used >= W or (m and memused >= MP)): t += 1`
  — a second memory op in a cycle moves to the next cycle and, in-order, so does everything
  after it. Never place memory ops back to back when `MP < W`; interleave them with ALU ops.
* **M4. All latencies are machine parameters ≥ 1, including `lat_alu`.** An unseen machine may
  have `lat_alu > lat_mul`, `lat_mac > lat_mul + lat_alu`, or `lat_addsh > 2·lat_alu`. No rule
  "shift beats multiply" or "`mac` beats `mul`+`add`" may be hard-coded.
* **M5. Non-termination is unconstrained, and the optimised program may run up to 20× the
  original's steps + 1000** (kvalidate). One-off setup code is allowed but bounded.
* **M6. Only the final data memory counts** (when no trap intervenes): stores to data memory can
  be reordered, merged and delayed inside a trap-free, volatile-free region.

## Task 3 — Dominant cycle terms per kind

| Kind | Dominant terms (input code) | Plan coverage |
|---|---|---|
| loops | LRU misses (11–22 names, K = 8), then fence + recurrence | ok; add M2, R1; closed forms DO, matrix power LATER |
| arrays | `lat_load` chain per iteration, fence, `MP = 1` | ok; add M3 and latency rotation; scalar replacement needs range proofs (masks give them) |
| numbers | `lat_div` on the recurrence | **under-attacked**: lazy reduction was LATER; divisibility test missing |
| bits | fences from per-bit branches, recurrence depth | if-conversion ok; GF(2) summarisation missing |
| state | taken jumps of dispatch chains, load latency | DFA threading ok; add header duplication, budget allocator |
| junk | issue slots, LRU misses | ok |
| effects | little to win; `vload` latency, `MP` | ok; M3 matters (vload/vstore are memory ops) |
| semantics | straight-line slots/latency; signed-division traps | ok; needs the sign-exact rules |
| control | taken jumps | ok; layout DP |
| fused | `mul`→`mac` recurrence, loads | ok; respect M4 |
| pressure | LRU misses almost entirely | **re-found on R1–R5** |
| large | `Q` per instruction beyond `C`, LRU (68–98 names) | plan over-relies on rerolling; clean-up + layout first |

## Task 5 — Citation check of PLAN §10

Verified by search this session: Rau MICRO 1994; Butts & Sohi ISCA 2004; Braun & Hack CC 2009;
Rocha et al. CGO 2022; Llosa et al. PACT 1996; Newell & Pupyrev (arXiv 1809.04676).
Fixes:
* ~~"Amarasinghe, Lee, Greenwald" → Sheldon, Lee, Greenwald, Amarasinghe, LCPC 2001~~ — **reverted in Task 5b**: TM-600 is Amarasinghe, Lee, Greenwald.
* "Shobaki & Wilken … trace scheduling" link is **Shobaki, Wilken, Heffernan, ACM TACO 2009**;
  the superblock paper is **Shobaki & Wilken, MICRO 2004**. "Shobaki et al. 2022, B&B": I
  believe the 2022 TACO paper is ant-colony, not B&B — **flag, verify**.
* "Chang, Mahlke et al., ILP transformations, 1991" → accumulator expansion / tree-height
  reduction are in **Mahlke, Chen, Gyllenhaal, Hwu, Chang, Kiyohara, "Compiler code
  transformations for superscalar-based high-performance systems", Supercomputing 1992**; the
  linked file name suggests a different 1992 Mahlke paper — **flag, verify**.
* Ext-TSP is cited for an objective the Kiln does not have (topic 5); add Pettis & Hansen 1990
  and Young et al. 1997.
* "Integer division by constants: optimal bounds" → Lemire, Bartlett, Kaser, Heliyon 2021.
* Knoop–Rüthing–Steffen: the link is the TOPLAS 1994 paper, the table cites PLDI 1992; both exist.
* Unroll-and-jam → Carr & Kennedy, TOPLAS 1994 (not "Callahan, Carr, Kennedy", which is scalar
  replacement, PLDI 1990).
* **Could not verify at all:** Fallin's 2026 blog URL, "AutoPass (2026)" alphaxiv 2606.20373,
  the GCC 2011 reassoc patch link. Treat as unverified.
* Everything else in §10 matches my memory of author/venue/year, but was not re-fetched.

## Task 5b — Second citation audit (topics 4–12 and PLAN §10)

Checked this session by search (**[verified]** unless stated):

* **Correction to the first review's Task 5:** MIT LCS TM-600 (1999) is by **Amarasinghe, Lee,
  Greenwald** — the plan's *original* order. Sheldon's work is a separate 2001 M.Eng. thesis. An
  LCPC 2001 version with Sheldon first is **not confirmed** (two searches). Cite TM-600 + thesis.
* **`par.nsf.gov/servlets/purl/10379551` is not the ACO paper.** It is a **CC 2022** paper, pp.
  41–53, by Shobaki, Bassett, Heffernan, Kerbow. I believe it is the graph-transformation paper for
  their B&B scheduler, which would make the plan's "B&B" label right after all; title **[unread]**.
  The ACO paper is TACO 19(2), art. 23, 2022 (PAR 10379549).
* **Frohn, TACAS 2020** is single-author (not "Frohn & Giesl et al.").
* **Fallin 2026 blog** (cfallin.org/blog/2026/04/09/aegraph) **exists** — UNVERIFIED lifted.
* **AutoPass** = Li, Ren, Tang, Zheng, Wang, "Evidence-guided LLM agents for compiler performance
  tuning", arXiv 2606.20373, June 2026 — exists; it is an LLM-agent tuner (the plan's NO stands).
* **GCC reassociation width:** Enkovich (Intel), PR middle-end/44382, gcc-patches July–Aug 2011;
  `msg01493` exists. +3.04 % CFP2000 on 32-bit Atom.
* **Mahlke et al., Supercomputing '92, pp. 808–817, doi 10.1109/superc.1992.236682**: the PDF
  lists **four** authors (Mahlke, Chen, Gyllenhaal, Hwu); some bibliographies add Chang and
  Kiyohara. The plan's link is the right paper.
* **Carr & Kennedy, TOPLAS 16(6), Nov 1994** (unroll-and-jam / memory balance); pages 1768 or
  1769–1810 depending on index.
* **Fisher & Freudenberger, ASPLOS 1992**: two DOIs in circulation (…143365.143493 /
  …143371.143493) — check before quoting.
* **Arnold, Fink, Sarkar, Sweeney, DYNAMO 2000**: mirrors disagree on authors; the IBM page lists
  these four.
* New references used in topics 4–12, all verified: Scheifler CACM 1977; Lokuciejewski & Marwedel
  ECRTS 2009; arXiv 2608.20137 (2026); Calder & Grunwald ASPLOS 1994; Young et al. PLDI 1997;
  Mestre, Pupyrev, Umboh ISAAC 2021; arXiv 2412.03680; Kincaid et al. POPL 2018/2019; Wilken et al.
  PLDI 2000; Malik et al. IJAIT 2008; Tristan et al. PLDI 2011; Chen & Chen S&P 2018; Ding et al.
  PLDI 2015; Savage JASA 1951; Souper arXiv 1711.04422; Hydra OOPSLA 2024; Minotaur OOPSLA 2024;
  Goharshady et al. OOPSLA 2024.
* **Still not checked:** Pettis & Hansen 1990, Necula 2000, Korel 1990, Wu & Larus 1994, Sarwate
  1988, AlphaDev 2023, the ISSAC 1994 chains-of-recurrences author list, and the rest of PLAN §10
  that no topic touched (Boissinot, Hack & Goos, Janssen & Corporaal, Cooper OSR, Knoop LCM, EMI,
  Triantafyllis, Ball & Larus, Hwu 1993) — all well known, none load-bearing for a verdict.

## Missing Tasks 2 and 4

The first review numbers its extra tasks 1, 3 and 5. **Tasks 2 and 4 were never written, and
their brief is not in the repository or in any session I can reach.** I did not invent them.
If the brief turns up, they are the only part of the review left undone.

## Verdicts that rest on memory only

Re-reading every verdict for ones supported only by **[unread]** claims:

* **Topic 1 (rerolling)** — RoLAG's mismatch-handling details are from memory, but the verdict
  rests on the [model] break-even, so it stands.
* **Topic 3 (latency rotation)** — Lam 1988 and TI collapsing are [unread]; the verdict rests on
  the fence derivation [model]. Not re-measured here; **the one topic-level verdict left
  un-measured**.
* **Topic 12** — AlphaDev [unread]; irrelevant to the NO.
* No other verdict in topics 4–12 rests on an [unread] claim: each has a [derived] measurement or
  a [verified] source behind it.

## Coverage report (plain)

| Topic | First review | Second review |
|---|---|---|
| 1 Rerolling | full | — |
| 2 LRU registers | full | — |
| 3 Latency rotation | full (paper derivation, not measured) | — |
| 4 Code-size budget | short | **full + measured** |
| 5 Block layout | short | **full + measured** |
| 6 Division / lazy reduction | short | **full + measured** |
| 7 Closed forms / matrix power | short | **full + measured** |
| 8 B&B scheduling | short | **full + measured** |
| 9 Validation / inputs | short | **full + measured** |
| 10 Candidate selection | short | **full + measured** |
| 11 NO verdicts / GF(2) | short | **full + measured** |
| 12 Newer work | short | **full** (search only) |
| Task 1, 3 | done | not re-checked |
| Task 5 citations | done | **re-audited (5b)**; one of its fixes reverted |
| Tasks 2, 4 | missing | **still missing** (brief lost) |

Limits of the second review: the experiments are small hand-written programs or random blocks,
not the hidden test set; nothing ran kopt itself; pages were seen through search results, not
opened in full.

## Where I am still unsure (blunt list)

1. How repetitive the hidden 100k programs are — decides whether rerolling matters at all.
2. Whether hidden trip counts stay ≤ 63 — decides fast doubling (topic 7) and lazy-reduction
   chunking; with n ≤ 63 fast doubling is a wash, with large n it wins.
3. Scratch spill vs hardware miss on slow-memory machines (topic 2; not measured).
4. Whether organisers accept GF(2) summarisation (now measured at 1.7×) and idiom recognition.
5. The scoring formula — it sets δ in the repaired do-no-harm rule (topic 10).
6. Whether LRU-aware naming closes the 18 % B&B-vs-list gap measured at K = 8 (topic 8), or only
   part of it.
7. Whether `state`-kind hidden programs exceed ~14 blocks (exact layout then falls back to the
   heuristic exactly where layout matters most).
8. Topic 3 is the only verdict not checked by measurement.
