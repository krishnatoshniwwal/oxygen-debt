# DESIGN.md - how kopt works

`kopt` is written in Rust (stable, standard library only). No compiler framework and no
solver is used. It was first written in Python; that version is kept in `pyref/` as a
reference. The first Rust version produced byte-identical output on all practice programs;
the Rust version has since moved on (candidate choice, section 2 step 9), so the two now differ.

**Code ported from the kit.** `src/kir.rs` is a Rust port of the kit's `kirlib.py`: the
parser, the machine file reader, the interpreter and the timing model, plus the generator
of made-up inputs. kopt uses it to read the input and to measure and check its own outputs
(see "Pass order", step 9).

**Reading used.** Cytron et al. (SSA construction with dominance frontiers); Cooper,
Harvey, Kennedy (a simple dominator algorithm); Sreedhar et al. (leaving SSA through
fresh names, "method I"); Chaitin (register colouring, coalescing, spill choice);
Pettis and Hansen (ordering blocks by chaining hot edges); the textbook descriptions of
value numbering, loop-invariant code motion, if-conversion and interval analysis with
widening.

---

## 1. What is implemented

Analyses

* Control-flow graph, reverse post-order, dominators, dominator tree, natural loops and
  loop depth. Loops with two entrances are simply not treated as loops.
* SSA form (pruned: phis only where the register is live).
* Value ranges: an unsigned interval for every value, refined by branch conditions.
* A simple memory model: an address is "base value + constant"; two addresses are known
  to be different if they have the same base and different constants, or if their value
  ranges do not overlap.

Optimizations

* Constant folding, copy propagation, algebraic simplification (`x+0`, `x*1`, `x^x`,
  `(x+5)-5`, `i*8 + i*10` -> `i*18`, shifts by constants, double negation of branch
  conditions, and so on).
* Value numbering over the dominator tree (the same computation is done once).
* Branch folding, unreachable-block removal, jump threading, block merging.
* Dead-code removal that keeps everything that can trap or is visible.
* Range-based rewrites: `div`/`mod` by a power of two becomes `shr`/`and` **only when
  the value is proven non-negative**; `mod` by a constant m of a value proven in [0, 2m)
  becomes a conditional subtract; masks that do nothing are removed; comparisons and
  branches that the ranges decide are folded.
* Memory: a load of an address whose content is already known is removed (redundant
  loads, store-to-load forwarding); a store that writes what is already there is removed;
  a store that is overwritten in the same block before anyone can see it is removed.
* Loop-invariant code motion (only for instructions that cannot trap).
* If-conversion: small `if` / `if-else` shapes become `select`.
* Full peeling of loops with a small constant trip count; loop unrolling (every copy
  keeps its exit test); loop rotation by copying the loop test to the end of the body.
* Fused instructions `mac` and `addsh`, and `shl` instead of `mul`, chosen from the
  machine file.
* Speculation inside loops: long-latency work (`div`, `mod`, `mul`, `mac`, `load`) of a block is
  moved above the branch of its only predecessor, so the scheduler can overlap the
  latencies of several unrolled iterations.
* Instruction scheduling inside a block for the in-order machine.
* Register colouring with copy coalescing, so that few register names are used.
* Spilling to the scratch region when there are still more names than hardware registers.
* Block ordering so that hot jumps fall through and hot code sits early in the file.
* A search over the settings of the above, measured with the exact timing model.

Missing or only half done

* Strength reduction of induction variables only for `i + k` (a new induction variable,
  and the exit test moved onto it); `i*c` is not kept up to date by adding.
* No reassociation of long sums (for example to shorten the chain through an accumulator).
* No partial redundancy elimination: a value computed on only one path into a join is
  not reused after the join.
* No closed forms for loops (the `guarded_div` example in the kit is not reproduced).
* Unrolling never removes the exit tests of the copies, even when the trip count could
  be computed at run time.
* Signed `div`/`mod` by a constant stays a division when the value may be negative.
* Load/store facts are not carried through a join as a new phi.
* Loops with two entrances get no loop optimizations.
* Scheduling works one block at a time. Only the speculation pass moves work across
  branches, and only inside loops, only upwards into a block's single predecessor.

## 2. Pass order

1. **Parse** with the kit's parser. Build the control-flow graph.
2. **Into SSA.** Copies and constants disappear here: a `mov` or `const` just renames.
   Registers that are read before any write become the literal 0 (every register starts
   at 0). `mac` and `addsh` in the input are split into `mul` + `add` so that later
   passes see one simple form.
3. **SSA optimization round**, repeated until nothing changes (at most a few times):
   simplify + value numbering + branch folding; CFG clean-up; dead code;
   lazy modular reduction (optional); value ranges -> range-based rewrites; memory pass;
   loop-invariant code motion; induction-variable rewrite (optional); if-conversion. The cheap clean-ups run first because every later pass sees less code,
   and they run again after each bigger pass because that pass exposes new chances.
4. **Peeling.** Loops whose exit test depends only on constants are run "on paper" to
   get the trip count. The function leaves SSA, the loop body is copied that many times
   in front of the loop, and step 2 and 3 run again. Constant folding then removes the
   loop tests and the left-over loop. This repeats for the next outer loop.
5. **Unrolling** (optional, chosen by the search): leave SSA, copy the loop body in a
   chain, go back into SSA, run step 3 again. Doing it here instead of at the very end
   lets the simplifier merge `i+1+1` into `i+2`, so the copies do not wait for each other.
6. **Lowering**: last dead-code pass with range information; pick `mac`, `addsh`, `shl`.
   This is late because the simple `mul`/`add` form is easier for all other passes.
7. **Speculation** (optional, chosen by the search), then **scheduling** inside each
   block. After lowering, because they need the real latencies.
8. **Out of SSA and registers**: phis become copies through fresh names; colouring with
   coalescing removes almost all of them; optional spilling to scratch memory.
9. **Back end and choice**: final jump clean-up; copy small blocks into their
   predecessors (this rotates loops); optional late unrolling; block order.
   kopt builds several candidates (different settings of steps 3 to 9: a greedy walk that
   changes one setting at a time and keeps a change when it is faster, then a refinement
   round that tries every single change of the final setting not measured yet, because
   the greedy walk depends on the order of its steps), runs each one
   on up to 8 made-up *timing inputs* with the kit's timing model, and keeps the one with
   the fewest total cycles. The timing inputs are shaped like the kit's sample `.in` files:
   arguments mostly small (2..20, 3..12, 8..47, 16..128), some larger (50..300,
   100..2000), now and then a full 32-bit value or a small negative number; memory filled
   with small values. Cycles are added up as absolute numbers, not ratios, because the
   judges rank by Average Cycles (total cycles / programs). A candidate is only written if
   its total is **strictly below the input program's** on the same inputs; otherwise the
   input is written unchanged (do no harm). The chosen candidate is then compared with the
   input program on more made-up inputs (edge values, values that trap). If it differs
   anywhere it is thrown away and the next best is tried. If none is left, the input is
   written unchanged.

## 3. Each pass

`n` = number of instructions, `b` = number of blocks.

**SSA construction** (`ssa.build_ssa`). Liveness by a worklist, dominance frontiers,
phi placement, renaming along the dominator tree (no recursion). Time: about linear;
the liveness sets can make it n times (number of live registers). Safe because it only
renames: each use reads the same value as before.

**Simplify + value numbering + branch folding** (`passes.scalar_pass`). One walk over the
dominator tree with a hash table of "operation + operands -> value" that is undone when
the walk leaves a subtree. Time: linear per walk. Safe because:
* every algebra rule is an identity of 32-bit wrap-around arithmetic for **all** values;
* a `div`/`mod` with constant operands is folded only when the divisor is not 0;
  nothing else about `div`/`mod` is assumed here;
* a computation is replaced only by an identical one in a block that dominates it, so
  the earlier one has always run already. This also holds for `div`/`mod`: if the first
  one did not trap, the second one cannot trap either;
* a branch is folded only when its condition is a constant.

**CFG clean-up** (`passes.cleanup_cfg`). Removes unreachable blocks, skips blocks that
only jump, merges a block with its only successor. Linear. Code that can never run
cannot trap.

**Dead code** (`passes.dce`). Mark and sweep from the roots: `store`, `vstore`, `vload`,
`trap`, `ret`, `br`, and every `load`/`div`/`mod` that is not proven trap-free. Linear.
An unused `vload` stays (only its result is unused). An unused `div` stays unless the
range analysis proves that the divisor is never 0; an unused `load` stays unless its
address is proven below 4096.

**Value ranges** (`ranges.Ranges`). Each value gets an interval `[lo, hi]` of unsigned
numbers. The pass walks the dominator tree again and again until nothing changes;
intervals that keep growing are widened (to 2^31-1, then to the full range) so it stops
after a few walks; two more walks shrink the result again. In a block that can only be
reached through one side of a branch, the branch condition (`lt`, `eq`, or the value
itself) narrows the operands. The first widening of an upper bound stops at the next
*threshold* (m - 1 for every `mod` by a constant m in the program) before jumping to
2^31-1, so a running value that is reduced `% m` every step is proven to stay in
[0, m - 1]. Signed compares only narrow a value that is already known
to be non-negative. Time: a small number of linear walks; if it has not settled after 14
walks kopt uses no range information at all. Safe because the final state is checked by
one more full walk that changes nothing: every interval then contains everything its
defining instruction can produce.

**Range-based rewrites** (`passes2.range_simplify`). Linear. `x / 2^k -> x >> k` and
`x % 2^k -> x & (2^k-1)` need `x <= 2^31-1` from the ranges; `x % y -> x` needs
`0 <= x < y`. Without that proof nothing is changed, so negative values are never
shifted. With a constant `m` and `0 <= x < 2m` proven (for example a running sum that is
reduced every step, as in Adler-32), `x % m` becomes `t = x - m; c = x < m; select c, x, t`:
three ALU instructions instead of a division-latency instruction on the loop-carried chain.
This is exact because `x - m` is then the remainder when `x >= m`, and every value involved
is non-negative and below 2^31 (signed `lt` and signed `mod` agree with unsigned ones). It
needs one more temporary, so the search can switch it off.

**Memory pass** (`passes2.mem_pass`). A forward data-flow of facts "the word at address A
holds value V", created by loads and stores (in the candidate with load reuse off, only by
stores: a second load of the same word is kept). A store removes every fact whose address it
may write. At a join only facts that agree on all incoming paths survive. Time: linear
per round, at most 10 rounds; if it does not settle, only facts inside one block are
used. Safe because a fact for address A exists only if a load or store of exactly that
address has already run on every path, so the address is valid (no trap is lost) and no
store that may write A lies in between. Dead-store removal works backwards in one block
and gives up at any instruction that may trap, and at volatile operations when the store
itself may trap.

**Loop-invariant code motion** (`passes2.licm`). Innermost loops first. An instruction
moves to a block in front of the loop when all its operands are defined outside the
loop. Time: linear in the loop sizes. Safe because only instructions that cannot trap
move: plain arithmetic; `div`/`mod` whose divisor is proven non-zero **at the new
place**; `load` whose address is proven valid at the new place and that no store in the
loop may write. A guarded division therefore stays behind its guard.

**Lazy modular reduction** (`passes3.lazy_mod`, optional, chosen by the search; runs at
the start of each SSA round). In a loop entered from one block, with a counter
`i = phi(i0, i + 1)` (i0 a constant >= 0) tested at the header by `br (lt i, n), body,
exit`, a running value `s = phi(s0, (s + t) % m)` (m a constant) stops being reduced inside
the loop when: `s` is used in the loop only by that add, the add only by the `%`, the `%`
only by the phi, and after the loop `s` is used only by ordinary instructions; the add
runs at most once per iteration (not in an inner loop); and with T = the largest t,
S0 = the largest s0 and N = (largest n) - i0 (the most iterations), S0 + N*T < 2^31 is
proven by the value ranges. The loop then carries the plain sum, and each use after the
loop reads `s % m` (one `mod` inserted before it). Example: the Adler-32 and Fletcher
checksums, and `sum_cubes_mod`. Time: linear. Safe because every unreduced value is then
in [0, 2^31), where KIR's signed `%` is the ordinary remainder, and
((s0 + t1) % m + t2) % m ... equals (s0 + t1 + t2 + ...) % m for non-negative numbers.

**Induction-variable rewrite** (`passes3.iv_rewrite`, optional, chosen by the search; runs
in the SSA round after loop-invariant code motion). For a loop entered from one block `p0`
with an induction variable `i = phi(i0, i + c)` (the same `i + c` on every back edge, c a
constant), each `a = i + k` in the loop (k a constant, at most 4 different k) is replaced
by a new induction variable `ai = phi(i0 + k, ai + c)`, with `i0 + k` computed at the end
of `p0`. Then an exit test `lt i, n` (or `lt n, i`) with `n` defined outside the loop
becomes `lt ai, n + k`, with `n + k` computed in `p0`. When `i` has no use left, dead-code
removal deletes it. Typical effect: an array loop steps the address directly and no
longer computes `base + i` every iteration. Time: linear in the loop sizes. Safe because
`add` is arithmetic modulo 2^32, so `ai == i + k` holds in every iteration for any values;
the comparison is rewritten only when the ranges prove `i + k` and `n + k` both stay in
[0, 2^31), where adding k to both sides of a signed `<` does not change its result.

**If-conversion** (`passes3.ifconv`). Looks for a branch around one or two small blocks
that rejoin. The blocks' instructions are moved up and each phi at the join becomes a
`select`. Only instructions that cannot trap at the branch are allowed, and no stores or
volatile operations, so running both sides has no visible effect.

**Trip counts and peeling** (`passes3.find_const_trips`, `peel`). The exit test of a loop
is evaluated on constants until it says "leave" (at most 64 rounds). Peeling is plain
copying of blocks on non-SSA code: copy 0 runs first, its back edge enters copy 1, the
last copy enters the original loop, which stays in the program. So peeling is correct
**even if the trip count were wrong**; the count only decides how many copies to make.
The left-over loop disappears only when constant folding and the ranges prove its test
false.

**Unrolling** (`backend.unroll`). Also plain copying: k copies of the loop body in a
ring, every copy keeps all its exit tests. Correct for any loop.

**Lowering** (`passes2.lower`). `add(mul(a,b), c)` -> `mac`, `add(a, mul(b, 2^k))` ->
`addsh`, when the multiplication has no other use and sits in the same block. Linear.
These are the definitions of the fused instructions.

**Speculation** (`passes3.speculate`). In SSA form, for a block `s` inside a loop whose only
predecessor `p` (in the same loop) ends in a two-way branch: an instruction of `s` moves to
the end of `p` (before the branch) when it is a `div`, `mod`, `mul`, `mac` or `load`, or
computes an operand of one; when it is pure, or is a `div`/`mod` whose divisor the value
ranges prove non-zero *at p*, or is a `load` whose address the ranges prove below 4096 *at
p* and that no store or volatile operation of `s` comes before (it then still runs after
exactly the memory operations it ran after before); when every operand is defined outside
`s`; and when its value does not
feed a phi (moving such a value above an exit would make the exit need the old value too).
One sweep moves work up by one block; the search tries 1 and 8 sweeps. Time: linear per
sweep. Safe because the moved instruction has no effect and cannot trap, so running it on
the path that leaves the loop changes nothing that can be seen; every use of its value is
in `s` or below, and `p` dominates all of them (it is the only way into `s`).

**Scheduling** (`passes3.schedule`). For every block a dependency graph is built and the
instructions are re-ordered greedily: among the instructions whose inputs are done, take
the one that can start earliest, and among those the one with the longest chain of work
behind it. It only looks 32 instructions ahead, so that values do not stay alive much
longer. Time: n times a small window. Safe because these orders are kept: (a) a value is
computed before it is used; (b) all stores, volatile operations and instructions that
may trap keep their order among themselves; (c) a trap-free load stays between the
stores that may write its address.

**Out of SSA and colouring** (`ssa.destruct_ssa`, `ssa.allocate`). Each phi gets a fresh
name X: every predecessor ends with `X = operand`, the block starts with `result = X`.
Because X is new, no value is overwritten too early. Then an interference graph is
built (two names interfere if one is written while the other is live), copies whose two
sides do not interfere are merged, and the graph is coloured greedily in
smallest-degree-last order. All parameters interfere with each other. Time: n times the
number of live names.

**Hot colouring** (`ssa.colour_hot`, optional, chosen by the search). If the normal
colouring still needs more names than the machine has registers, the hardware register
file works like an LRU cache: what costs is touching many different names inside a hot
loop. In this mode the values are coloured in order of their frequency-weighted number of
touches (loop depth counts 8x per level), each taking the lowest colour its neighbours
leave free, so the values of the hottest loops share the lowest names and stay resident.
Time: one sort and one greedy pass. Safe for the same reason as any colouring: two values
that are alive at the same time never get the same name.

**Forced spill of one hot-loop value** (`ssa.allocate` with `force = n`, optional, chosen
by the search). If more names than registers are needed, the values that are alive on
entry to the hottest blocks are ranked by how often they are touched (fewest first), and
the n-th one is kept in a scratch word (store after each write, load before each read)
before the normal colouring. Inside a hot loop one load can be cheaper than the register
misses caused by one name too many (for example a loop limit read once per iteration).
The search tries n = 1, 2, 3 and keeps whichever measures fastest. Same safety condition
and rewriting as the spilling below.

**Spilling** (`ssa._pick_spills`, `_rewrite_spills`). If there are more names than
hardware registers, the cheapest names (few uses, outside loops) are kept in scratch
words 3072 and up: a `store` after each write, a `load` into a short-lived name before
each read. Then colouring runs again. Only done when the range analysis proves that
every `load` and `store` of the program uses an address below 3072, so the program
cannot see or damage these words. The scratch region is not observable.

**Final clean-up, tail copying, block order** (`backend`). A `jmp` to a small block is
replaced by a copy of that block (this turns `while` loops into loops tested at the
bottom). Blocks are chained along the hottest edges; the entry chain comes first, then
the hottest chains. All of this is copying and re-ordering whole blocks. Linear, plus a
sort of the edges.

## 4. The machine

The machine file is read with the kit's `load_machine`. It is used in these places:

* `fused_mac`, `fused_addsh`: `mac` / `addsh` are written only when the flag is 1.
  The final output is also checked against the machine before it is accepted.
* `lat_alu`, `lat_mul`, `lat_mac`, `lat_addsh`: decide `shl` versus `mul`, and whether
  a fused instruction is worth it.
* All latencies, `issue_width`, `mem_ports`: used by the scheduler.
* `registers`: the target of the colouring; spilling starts only above it.
* `icache_size`: the budget for peeling, unrolling and tail copying.
* **Everything**, including `reg_spill_penalty`, `taken_branch_penalty`,
  `icache_penalty`: the search measures every candidate with the exact timing model for
  this machine and keeps the fastest. The candidates differ in: unroll factor (1, 2, 4,
  8), if-conversion size (off, 2, 4, 8, 24), scheduling on/off, `mac` on/off, peeling
  (off, within the cache, beyond the cache), spilling on/off, reuse of loaded values on/off
  (reloading a word can beat keeping many loaded values alive when registers are few),
  speculation (off, 1 or 8 sweeps), conditional subtract for `mod` on/off, hot colouring
  on/off and forced spill of one hot-loop value (only when there are more names than
  registers), the induction-variable rewrite on/off, lazy modular reduction on/off,
  tail-copy size, late unrolling, and a loose or tight code-size budget. So a setting that is only good for
  the default numbers is not chosen on a machine where it loses.

## 5. Keeping the behaviour

* `vload` and `vstore` are never removed, copied past each other, or re-ordered. They
  are roots for dead-code removal and fixed points for the scheduler. Their operands are
  only replaced by equal values.
* An instruction that may trap (`load`, `store`, `div`, `mod`, `trap`) is never removed,
  never moved across a branch or out of a loop, and never moved across a store, a
  volatile operation or another such instruction. The only way out of this rule is a
  proof from the range analysis that it cannot trap (address below 4096, divisor not 0).
* Loads are only reused when no store that may write the same address lies in between.
* Stores are only removed when the same word is certainly written again before any
  load, trap, or return can see it.
* Unknown registers are treated as 0, exactly as the specification says.
* Code copying (peeling, unrolling, tail copying) never depends on an analysis being right.
* The scratch region is only used after proving that the program never touches it.
* **Self-check.** Before writing the output, kopt runs the input and the output on up
  to 60 made-up inputs (the kit's edge values, awkward pairs, random values, random and
  very large memory contents) and compares result, volatile log and data memory. On any
  difference the candidate is dropped. This is a second line of defence, not the proof.
* If anything fails or panics inside kopt, that candidate is dropped (and if nothing is
  left, or an error escapes, the input is written unchanged).

## 6. Profiles

The profile is only used for the block order. An edge count from the profile is mixed
with kopt's own estimate (loop depth, "staying in a loop is likely"), so a profile that
says "never" cannot push a loop body to the cold end by itself. Block names that no
longer exist are ignored. A wrong profile can therefore only cost some taken jumps; it
cannot change what the program does, and the search still measures every candidate on
kopt's own inputs. Without a profile only the static estimate is used.

## 7. Known problems

* The search measures candidates on made-up inputs shaped like the sample `.in` files
  (the organisers say hidden inputs follow the samples). If the judges' inputs are very
  different, the chosen setting may not be the best one for them. Inputs that run very
  long are skipped for timing (step budget), so very long runs are under-represented.
* Large programs (more than 6000 instructions) get the full pipeline without peeling,
  at most 2 rounds of the SSA optimizations, and only a few whole-program candidates
  (load reuse off, spilling, if-conversion off) instead of the general search; above
  150,000 instructions only the first candidate. Programs above 300,000 instructions get
  one light pass with no search. On generated programs of
  about 100,000 instructions (`tests/gen_large.py`: a long polynomial, unrolled CRC
  steps, mixed straight-line work, thousands of small loops) kopt needs 0.8 to 5 seconds
  on a laptop; the loop-heavy one is the slowest, because the value-range analysis is run
  several times per candidate. If the interference graph would be extremely large, kopt
  writes the input unchanged instead of running out of time.
* With more live values than registers the result is still far from the best possible:
  spilled values are loaded right before each use, and loops are never split.
* The optimizations listed as missing in section 1.
* The work budget of the search is counted in instructions, not in seconds, so the output
  never depends on the speed of the computer.
* With odd control flow (loops with two entrances) a value can end up defined in terms of
  itself. kopt does not simplify such values, and the rewrite rules and the replacement map
  have loop guards, because an earlier version could loop forever on them.

## 8. Verification tools (offline) and the stronger self-check

**kfuzz** (`src/bin/kfuzz/`, built with `cargo build --release --offline --features tools`; not part
of `build.sh`). It runs the real `kopt` binary on a case and compares the output with the input on
many inputs (the kit generator, every program literal +-1 as an argument, memory images of 600 to
4096 words, empty and all-ones memory, long and empty `vin` queues). Sub-commands: `gen` (random
programs x random machines), `bench` (kit programs x kit machines), `machines` (kit programs x random
machines: every key over its legal range), `mutate` (judge simulator), `min`, `diff`, `show`.
* The generator builds valid programs with traps (div by 0, wild address, explicit trap), `vload` /
  `vstore`, loops with two entrances, aliasing stores, up to ~45 live registers, masked and constant
  trip counts, deep if/switch nests, unreachable blocks, scratch-region accesses and the bench idioms
  (checksums with `mod`, `i + k` array walks, guarded division, CRC steps, checked add).
* The judge simulator renames registers, shuffles blocks (entry first), renames labels and changes
  constants (as the rules say), plus EMI mutations: opaque-predicate guards with junk on the never-taken
  side (semantics kept exactly, so the output is also compared with the *unmutated* program), junk inserted
  into blocks that did not run, dead code deleted.
* Problems reported: `mismatch`, `invalid`, `crash`, `timeout`, `nondeterministic`, `caught-selfcheck` /
  `caught-timing` / `pass-failed` (kopt noticed a bad candidate itself: a pass bug that the safety net
  hid), `slow`, `regress` (cycle total of output above the input on the test inputs: informational).
  Failing cases are minimised automatically (branch -> jump, instruction deletion, operand and
  computation simplification).
* Results so far (all with no wrong output): about 14,000 random programs, 500 bench cases (all five
  machines), 4,000 mutations, 2,600 random machines. The self-check and timing check caught
  four candidates from two real pass bugs, both fixed (see below).

**Bugs found.** (1) `speculate` trusted the range of an address defined in the block it was emptying;
that range was computed under the loop test, so a load with address `i + 1` where `i` steps by 16386
was hoisted above the exit test and trapped spuriously (minimal repro: loop `i += 16385 + 1` with
`load [i+1]`). Now only values defined outside the block, or moved values whose range is recomputed at
the new place, are trusted. (2) `lazy_mod` rewrote uses after the loop to `s % m` without requiring the
initial `s0 < m`; with zero iterations the original returns the unreduced `s0` (repro: Fletcher loop with
`s = 2^30`, `n = 0`). It now requires `s0 < m`. (3, not a bug) the search rejects spill-heavy candidates
that need more than 4x the original steps (`evalsim::score`); they are correct, only lost.

**Stronger self-check** (`src/validate.rs`, called after `Reference::same`). (1) Literals +-1 as
arguments. (2) Exhaustion: a dataflow finds which parameters can only be read through `and p, CONST`
(single-definition constant); then all 2^popcount values (up to 65536 combinations) are run; if the
program also has no `load` and no `vload` this is a complete proof. (3) Inverse execution (a small
solver-free concolic step): the trace records operand values and definition sites; for every divisor,
address and branch condition, the wanted value (0, 4096, other branch side) is pushed back through
`add sub xor mul(any constant) shl shr and or lt eq select mac addsh mov` and loads of never-written
words to an argument or a memory word, which is changed; the new inputs are run and inverted again (two
rounds). It finds traps behind guards such as `x == 1234567`. All with a fixed step budget (6M steps).

**Value-graph translation validator** (`src/vgraph.rs`, `prove(orig, cand)`; used by kfuzz, not needed at
run time). Both programs are executed symbolically path by path on one hash-consed term graph; terms
are canonical: ring terms as polynomials mod 2^32, bitwise terms as GF(2) algebraic normal forms with
mask coefficients (complete for the ring and for pure bitwise identities), `lt/eq/shr/div/mod/select`/loads
as atoms over canonical operands (sound, incomplete), and differences that are linear mixed
boolean-arithmetic are decided by the 0/1-input theorem (f == 0 for all x iff g(b) == 0 mod 2^32 for all
b in {0,1}^n). A path = the conditions it assumed; each original path must be matched by a candidate path
with a subset of its conditions (equal or coarser, e.g. after if-conversion) with equal result/trap,
volatile log and memory; unmatched = `Unproved` (never "wrong"). Only loop-free programs are handled
(`NotApplicable` otherwise: about 80 % of the generated programs, 76 % of bench). In kfuzz runs it proved
~2,700 cases and never contradicted a run (`VALIDATOR-UNSOUND` is reported if it ever does).
---

## 8. Loop transformations for arrays and counted loops (`src/loopopt.rs`)

Both passes work on SSA, are optional settings of the search (`rot`, `gu`), and run after
the general search on its final setting, so they can only add candidates; a candidate is
still written only if the exact timing model says it beats the input.

**Latency rotation** (`loopopt::rotate_loops`, setting `rot` 1 or 2; runs after lowering,
before speculation and scheduling). *What:* in a loop that is a plain chain of blocks
(header, straight body, one latch), a set S of instructions (long-latency roots: `load`,
`mul`, `mac`, `div`, `mod`, plus everything inside the loop they need) is moved to the end of
the previous iteration. Each value x in S becomes a header phi `q_x = phi(x_prologue,
x_tail)`; `x_tail` is a copy of x placed before the back edge whose operands are the *next*
iteration's values (a header phi p is replaced by its back operand), `x_prologue` is a copy at
the end of the preheader whose operands are the initial values. So `min(lat, 1+P)` of the
latency runs inside the fence of the taken back edge. One stage only: code cost is one
copy of S in the preheader. *Choice of S:* with at most 5 roots, every union of root
closures (at most 31 sets) is scored with a small exact model of the loop (list schedule per
block like `passes3::schedule`, then the Kiln issue rules over 6 and 24 iterations: issue
width, memory ports, latencies, fence, plus a penalty for more live values than registers);
the best set is used only if it beats the empty set. Above 5 roots the fixed rules
"all roots", "loads only", "everything but div/mod" are scored instead (`rot` 2 forces "all
roots"). The search then measures the real candidate. *Time:* per loop at most 31 model runs
plus up to 3 range analyses; loops of more than 90 instructions or 12 blocks are skipped.
*Why safe:* (1) pure instructions may run any number of extra times; (2) a `load`, `div` or
`mod` in S runs one extra time after the last iteration and once in the preheader even when the
loop runs zero times, so every copy is re-proven trap-free on the *rewritten* function
(fresh value-range analysis: load address < 4096, divisor >= 1); otherwise the rewrite is
dropped; (3) a load is only moved when no store of the loop that comes earlier in the body
may write its word (same memory model as the scheduler), because the early load now runs
before those stores; (4) a trapping instruction whose value is unused would vanish, so such a
set is rejected; (5) `vload`, `store`, `vstore` are never moved.

**Guarded unrolling** (`loopopt::unroll_guarded`, setting `gu` = 2, 4, 8; runs after the
`unroll` step of the SSA pipeline, followed by another SSA optimisation round). *What:* a
loop `while (a < b) body` (header with the test and no effect, straight-line body, `a` =
header phi + constant, the phi stepping by a constant c in 1..64, `b` loop-invariant) gets a
front loop that runs u bodies per test: `while (a0 + (u-1)c < b) { body x u }`, and then
falls into the original loop, which finishes the remainder. `b - (u-1)c` is computed once in
the preheader. The u copies are one straight block, so the scheduler overlaps them and the
memory pass reuses loads between them. *Why safe:* only done when the value ranges prove
a + (u-1)c < 2^31 and b < 2^31, so `a0 < b - (u-1)c` is exactly "all u tests would pass" (no
wrap-around); the header has no effect (it runs once more at the seam); the original loop is
kept unchanged as the remainder, so every exit and trap happens as before. *Time:* linear in
the loop size times u; loops with u x size > 400 are skipped.

**Accumulator splitting** (`loopopt::split_accumulators`, setting `acc` = 2 or 4; runs right
after rotation, after lowering). *What:* inside one block, a chain `x1 = x0 (+) t1; x2 = x1 (+)
t2; ...` of at least 4 links of the same operation (add, mul, xor, and, or, or `mac` read as
acc + a*b), whose intermediate values have no other use, is cut into k = min(acc, L/2)
interleaved chains (the second one starts from its first term, a `mac` chain from a `mul`),
combined by a small tree; the last combine keeps the original value's name. The loop-carried
latency goes from L to about L/k + log2 k (matters for `mul`/`mac`, latency 3). *Why safe:*
add, mul, xor, and, or are associative and commutative in Z/2^32 with no exceptions, and every
intermediate value is used only by the next link, so nothing else can see a changed value;
no instruction that can trap or has an effect is touched. *Time:* linear per block.

**Versioning of the guarded unroll** (part of `unroll_guarded`). When the ranges cannot prove
a + (u-1)c and b below 2^31, the preheader computes `x = (a_first | b) >> 31` and branches: x = 0
enters the front loop, x = 1 enters the original loop alone (the original loop now has two
entrances, merged by phis). With both values below 2^31 at entry, a stays below b < 2^31 inside
the front loop, so `a0 < b - (u-1)c` is exact as in the proven case. Costs 2-3 preheader ops.

**Scalar replacement** (`loopopt::reuse_loads`, setting `sr`). In a chain loop with counter p
stepping by s, `load(p + c)` of the next iteration reads the word `load(p + c + s)` read in this
one. If the value ranges show that no store of the loop can write either word (store range
and load range disjoint over all iterations), the earlier load becomes a header phi carrying
the later load's value, with a copy in the preheader for the first iteration (re-proven
trap-free on the rewritten function with a fresh range analysis, else dropped). Fixed-address
store promotion is not implemented. On the bench neither this nor the versioning is chosen
(the guarded unrolling and the memory pass already cover those loops, and the masks make the
ranges provable); they exist for programs with unmasked bounds.

Measured ablation (bench, 5 machines, the 3 perturbed inputs, total cycles; base 265,790):
rotation only 263,658 (-0.8%); guarded unrolling only 259,336 (-2.4%); both 257,401 (-3.2%);
all (with accumulator splitting) 256,352 (-3.6%). `KOPT_LOOPOPT=mask` (1 rot, 2 gu, 4 acc,
8 sr) restricts the settings for such experiments.
## 8. sched2: exact-cost reordering and renaming (src/sched2.rs)

**What.** After the normal pipeline, `driver::post_sched2` runs `sched2::optimize` on the 3 best candidate
texts. The text is parsed into blocks of named instructions. Hot blocks (>= 0.5% of dynamic instructions,
<= 240 instructions; program <= 4000 instructions and 400 names) are re-ordered and their block-local
values re-named. `Sim` is a step-by-step copy of `kir::time_trace` (slots, memory ports, latency, fences,
write-after-write waits, instruction-cache cost, hardware-LRU misses); a unit test checks it equals
`time_trace` on bench programs. A candidate order is scored by simulating short windows of the real
execution (about 48 instructions before the block, the block, about 32 after) from the timing inputs.
Seeds: original order and three cycle-driven list schedules; then a fixed-seed local search moves one
instruction to a random legal position and keeps it if the exact score does not rise (<= 1200 moves per
block, global step budget, no clocks). Naming is redone per candidate (`Region::name_block`): a local
value takes a name no needed value lives in, preferring no write-after-write wait, then names already in
the block's working set, then its old name. Names live into or out of the block never change.
**Time.** ~100-300 simulated instructions per evaluation; 4M simulated steps per candidate.
**Why safe.** Only order inside a block and local names change. Data edges keep every use reading the same
value. Instructions that may trap, write memory or are visible keep their order (`conflict`): two loads or
two divisions may swap; a store or visible op never crosses a trapping one; literal addresses < 4096 and
division by a non-zero literal cannot trap. A definition of a name live after the block stays last, after
every read of the old value. The result is an ordinary candidate: kept only if strictly faster, then
self-checked.
**Measured** (tests/measure.py, 100 programs x 5 machines): sample cycles 88988 -> 83230 (-6.5%),
held-out variants 265790 -> 247999, no mismatches.
**Not done:** global LRU-aware naming (MIN, rematerialisation, scratch), branch and bound for small blocks,
memory ordering from range proofs, large programs.
## 8. Loop algebra (`src/algebra.rs`, setting `alg`, bit 1)

**Closed forms** (`algebra::closed_forms`, SSA form, optional, chosen by the search; runs in each SSA
round before the range rewrites). A loop qualifies when it has one entry edge from a block that
ends in `jmp header`, one back edge, a header ending in `br c, body, exit` (either orientation) and
a straight chain of blocks as the rest; every instruction is trap-free arithmetic (no load, store,
volatile, trap, `div`/`mod` by a non-literal); every value used after the loop is a polynomial of
degree <= 4 in the iteration number k. Values are kept as polynomials in the binomial basis
`sum c_j C(k,j)` (coefficients are literals or outside values; products use
`C(k,a)C(k,b) = sum_t C(t,a)C(a,t-b)C(k,t)`); a header phi `p = phi(i0, p + f)` gets
`i0 + sum_m f_m C(k,m+1)`, other phis are solved in rounds. The trip count T comes from the exit
test with the **signed** `lt`: `T = n - i0` if `i0 <s n` else 0 (guard dropped when the ranges
prove it); the other supported tests are the falling counter and `eq` with an odd step
(`T = (B0-A0) * inverse(step)`). The values are evaluated at k = T before the loop and the loop
is cut off (header jumps to the exit; the chain becomes unreachable). Zero trips are part of the
formula. Binomials are exact mod 2^32 without dividing a wrapped value by an even number:
`C(k,2) = (k>>1)*((k-1)|1)`, `C(k,3) = C(k,2)*(k-2)*inv3`, and `a4*C(k,4) = (a4>>1) * C(k,2)*C(k-2,2)*inv3`
(only when `a4` is provably even; true for sums of cubes: a4 = 6). Safe because it is the same
function of the inputs, only computed directly; loops with any effect or trap are never touched.
Tests: `cargo test algebra` (8-bit analogue of the binomials for every k, random 32-bit and
edge vectors against a reference, polynomial products) and `tests/alg_edge.py` (unmasked loops
with n = 0..69, 2^31, 2^32-1 against the input program).
Bench (5 machines): total cycles on the sample inputs -3.6 % (default 58043 -> 55961), 0 failures;
sum_sq, nested_invariant, loop_invariant, sum_cubes_mod drop 5-20x.
Not done yet: division by constants, divisibility tests, i/c and i%c counters, strength
reduction, larger compile-time evaluation, the GF(2) / idiom items.

**Division by a constant with a low multiply** (`algebra::div_const`, `alg` bit 2). `div x, c` /
`mod x, c` (c a literal, not a power of two) with `0 <= x <= xmax < 2^31` proven by the ranges
becomes `q = (x*m) >> s` (remainder `x - q*c`) when a pair (m, s) exists with `m = ceil(2^s/c)`,
`xmax*m < 2^32` (no wrap) and `(m*c - 2^s)*xmax < 2^s` (exact). Tested for c < 400 and
xmax up to 10^5 against real division. Chosen by the search only (bench: 4 cases changed,
`digit_sum|wide_fast` 360 -> 330; two few_registers cases lose 2-9 % through the greedy search
order). Not done: peeling until x is in range (`digit_sum`), signed division with bias,
divisibility tests, `i/c` counters, strength reduction, larger constant evaluation, GF(2)/idioms.
## 8. Analysis upgrades (midend session)

**SCCP** (`sccp::sccp`, setting `sccp`). Optimistic sparse conditional constant propagation:
values start at Top, blocks and edges unreachable; a phi meets only its reachable incoming
edges, so loop-carried constants are found; a branch with a constant condition, or one the
value ranges decide (`ranges.get(c, b)` is [1,..] or [0,0]), makes one edge reachable. The result
replaces constants by literals, turns decided branches into jumps and drops unreachable
blocks. `div`/`mod` give a constant only from two constants with a non-zero divisor, or a
range singleton that proves no trap. Gives up (changes nothing) if 64 sweeps do not settle.
Soundness test: `sccp::tests` (abstract result contains concrete result on constants and
absorbing cases).

**Known bits** (`domain.rs`). Each value is (v, m): bits in m unknown, others = v. Contains the
power-of-two congruences (alignment, evenness) and an unsigned interval [v, v|m]. Transfer
functions for add/sub/mul (carry propagation), and/or/xor, shifts, signed `lt`, `eq`,
`div`/`mod` (including mod by 2^k of a value with known-zero low bits = 0), select, phi join.
Tested exhaustively on 4 bits (all pairs of 81 tnums, every member pair), on 8 bits against a
spread of operands, and with random 32-bit vectors: every concrete result is contained.
`bits_simplify` replaces known constants and removes `and`/`or` that change nothing. The same
analysis narrows the intervals of `Ranges::new` (intersection of two true facts).
Not done yet: wrapped intervals, non-power-of-two strides, relational facts, affine
dependence tests, PRE, memory phis, code motion, tree-height reduction, extra rewrite rules.

**Tree-height reduction** (`reassoc::reassoc`, setting `thr`; runs just before lowering). A chain
is the tree of same-op instructions (add, mul, xor, and, or) in one block whose inner results
have exactly one use. Its leaves are regrouped by repeatedly combining the two earliest-ready
values (phis, i.e. loop-carried accumulators, count as latest, so `acc + a + b + c + d` becomes
`acc + ((a + b) + (c + d))`); literals are folded into one. Safe because these ops are
associative and commutative modulo 2^32 and every leaf is defined before the chain root,
where the new instructions are placed. Measured: 12 bench cases faster, none slower
(about -1% on `wide_fast`); fuzz clean.
## 8. Block layout as a TSP, and state splitting (`layout.rs`, `control.rs`)

**Layout objective.** On the Kiln only two things depend on the block order: a taken jump
(about P + 1/2 cycles) and, for every executed instruction at file index >= C, about Q cycles.
`br c, A, B` may have either target next in the file, so branch sense needs no extra work; the
order alone decides. For an order `o`: cost = pt * (W - sum of w(a,b) over adjacent pairs) +
q * sum of cnt(b) * (instructions of b at index >= C). `w` are edge execution counts, measured
by running the candidate on kopt's timing inputs (`Reference::traces`), plus 5 % of the static
estimate / profile weight so unmeasured edges still count.
`layout::held_karp` solves this exactly for <= 16 blocks (the position of a block depends only
on the set before it, so the icache term fits the DP). Above that `local_search` relocates
segments of 1-3 blocks from the chain-merging order with the exact objective and a fixed work
budget (no clocks). Only whole blocks are re-ordered, so behaviour cannot change. The TSP order
is built in `Search::try_tsp` for every (variant, back end) pair as an *extra candidate*, scored
with the exact timing model like any other; it wins only if strictly faster (ties keep the
greedy order). Latch duplication with rotated placement is the existing tail copy (`BACKENDS`
maxdup 4/12) followed by this layout, which puts the copy before the header.

**State splitting** (`control::split_by_state`, knob `thr` = 0/1/2 state registers). A register
written with >= 2 different constants and compared with constants >= 2 times is a state register.
The code is copied once per constant value the register is known to hold on arrival (tracked
along the CFG by the last `const` write; forgotten where the register is dead). Every copy is a
faithful copy and every new edge an old edge, so the transformation is correct whatever the
tracking believes; the SSA passes then fold `eq state, K` in each copy and the dispatch
disappears (vending_fsm -30 % cycles). Growth is capped at 3 * icache_size instructions (nothing
changes if more would be added); the search measures it like every other knob.

**`i / c` and `i % c` of an induction variable** (`algebra::iv_divmod`, `alg` bit 4). For
`i = phi(i0, i + 1)` with a literal `i0` in [0, 2^31) and `i` proven in [0, 2^31) at the use, a
literal `c` in 2..4096: counters `r' = r+1; w = (r' == c); r = select(w, 0, r'); q = q + w`
with `r0 = i0 % c`, `q0 = i0 / c` replace the `mod` / `div`. Exact for non-negative i.
Checked by `tests/alg_div.py` (n = 0..299, 2^31, 2^32-1) and the bench (0 failures). Chosen by the search.
`count_divisors` uses `n % i` (variable divisor), which this does not touch.

**Node splitting** (`control::split_entries`, always on, no effect unless a cycle has two
entrances). A cycle (strongly connected component) entered at several blocks is irreducible, so
no loop pass touches it. The first entrance (lowest block id) stays the header; every other
entrance block E (<= 24 instructions) gets a copy E' that takes over all edges from outside the
cycle. E' is outside the cycle, which now has one entrance. Pure block copying. On
`irreducible_loop`: 416 -> 200 cycles at P=4 and 608 -> 216 at P=8 (variants; the original was
slower than the input there before). Tried and dropped: a larger tail-copy size (24) as an extra
back end changed no result on P in {0,4,8}.

**Reload cache in spill code** (`Variant.reuse`, `ssa::rewrite_spills`). With `reuse = n` the last n
values loaded from (or stored to) a scratch word stay available for later uses in the block until
that word is written again, instead of a fresh load per use. The longer live ranges can raise
register pressure, so it is tried only after the general search, on its final setting (trying it
early changed the walk's path and made `row_sums` worse). `sched2` also scores each candidate order
after the block-local reload peephole (`sched2::peep_block`), and the peephole runs once more on the
result. `spill_storm|default` 5116 -> 3656 cycles.

**Peeling by nest size** (`Variant.peel = 3`, `passes3::peel_const_loops(.., nest = true)`). Fully peels
constant-trip loops while the *outermost enclosing loop* (the code that repeats) stays within twice
the cache size counted before clean-up (clean-up roughly halves it), instead of the whole program
within one cache size. For `matmul4` the inner loop disappears and the outer loop's invariant row
loads move into registers (unroll-and-jam in effect): `matmul4|wide_fast` 1352 -> 552. The result often
needs more names than registers, so the pressure settings (`spill`, `hot`, `sched`, `reuse`) are tried
together with it, again as extra candidates after the main search.

**Dominator computation** (`ir::analyze`). The intersect step marks the nodes it climbs while it
handles one join block, so a join with tens of thousands of predecessors costs linear instead of
quadratic time (`large_nest`: 25 s -> 1.2 s). `large::too_hostile` now triggers at a fan-in of 40000
(`KOPT_HOSTILE=n` overrides it).

