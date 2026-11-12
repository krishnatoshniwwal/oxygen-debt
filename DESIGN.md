# DESIGN.md - how kopt works

`kopt` is written in Rust (stable, standard library only). No compiler framework and no
solver is used. It was first written in Python; that version is kept in `pyref/` as a
reference, and the Rust version produces byte-identical output on all practice programs.

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
  the value is proven non-negative**; masks that do nothing are removed; comparisons and
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
* Instruction scheduling inside a block for the in-order machine.
* Register colouring with copy coalescing, so that few register names are used.
* Spilling to the scratch region when there are still more names than hardware registers.
* Block ordering so that hot jumps fall through and hot code sits early in the file.
* A search over the settings of the above, measured with the exact timing model.

Missing or only half done

* No strength reduction of induction variables (`i*c` kept up to date by adding).
* No reassociation of long sums (for example to shorten the chain through an accumulator).
* No partial redundancy elimination: a value computed on only one path into a join is
  not reused after the join.
* No closed forms for loops (the `guarded_div` example in the kit is not reproduced).
* Unrolling never removes the exit tests of the copies, even when the trip count could
  be computed at run time.
* Signed `div`/`mod` by a constant stays a division when the value may be negative.
* Load/store facts are not carried through a join as a new phi.
* Loops with two entrances get no loop optimizations.
* Scheduling works one block at a time and does not move work across branches.

## 2. Pass order

1. **Parse** with the kit's parser. Build the control-flow graph.
2. **Into SSA.** Copies and constants disappear here: a `mov` or `const` just renames.
   Registers that are read before any write become the literal 0 (every register starts
   at 0). `mac` and `addsh` in the input are split into `mul` + `add` so that later
   passes see one simple form.
3. **SSA optimization round**, repeated until nothing changes (at most a few times):
   simplify + value numbering + branch folding; CFG clean-up; dead code;
   value ranges -> range-based rewrites; memory pass; loop-invariant code motion;
   if-conversion. The cheap clean-ups run first because every later pass sees less code,
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
7. **Scheduling** inside each block. After lowering, because it needs the real latencies.
8. **Out of SSA and registers**: phis become copies through fresh names; colouring with
   coalescing removes almost all of them; optional spilling to scratch memory.
9. **Back end and choice**: final jump clean-up; copy small blocks into their
   predecessors (this rotates loops); optional late unrolling; block order.
   kopt builds several candidates (different settings of steps 3 to 9), runs each one
   on a few made-up inputs with the kit's timing model, and keeps the fastest. That
   candidate is then compared with the input program on more made-up inputs (edge
   values, values that trap). If it differs anywhere it is thrown away and the next
   best is tried. If none is left, the input is written unchanged.

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
itself) narrows the operands. Signed compares only narrow a value that is already known
to be non-negative. Time: a small number of linear walks; if it has not settled after 14
walks kopt uses no range information at all. Safe because the final state is checked by
one more full walk that changes nothing: every interval then contains everything its
defining instruction can produce.

**Range-based rewrites** (`passes2.range_simplify`). Linear. `x / 2^k -> x >> k` and
`x % 2^k -> x & (2^k-1)` need `x <= 2^31-1` from the ranges; `x % y -> x` needs
`0 <= x < y`. Without that proof nothing is changed, so negative values are never
shifted.

**Memory pass** (`passes2.mem_pass`). A forward data-flow of facts "the word at address A
holds value V", created by loads and stores. A store removes every fact whose address it
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
  (off, within the cache, beyond the cache), spilling on/off, tail-copy size, late
  unrolling, and a loose or tight code-size budget. So a setting that is only good for
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

* The search measures candidates on made-up inputs (small arguments, small memory
  values). If the judges' inputs are very different, the chosen setting may not be the
  best one for them.
* Very large programs (more than 6000 instructions) get one light pass over the pipeline
  with no search: no peeling, no if-conversion, no unrolling. On our test programs of
  about 100,000 instructions kopt needs 1 to 4 seconds on a laptop. If the interference
  graph would be extremely large, kopt writes the input unchanged instead of running out
  of time.
* With more live values than registers the result is still far from the best possible:
  spilled values are loaded right before each use, and loops are never split.
* The optimizations listed as missing in section 1.
* The work budget of the search is counted in instructions, not in seconds, so the output
  never depends on the speed of the computer.
* With odd control flow (loops with two entrances) a value can end up defined in terms of
  itself. kopt does not simplify such values, and the rewrite rules and the replacement map
  have loop guards, because an earlier version could loop forever on them.
