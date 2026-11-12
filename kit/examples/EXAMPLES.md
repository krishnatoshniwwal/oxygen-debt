# examples/EXAMPLES.md - 15 solved examples

This folder shows **what an optimized program can look like**. For 15 of the 100 programs in `bench/`, there is an optimized version `NAME_opt.kir` next to a sample input `NAME_opt.in`.

Every optimized version was checked with `kvalidate --against` on 300 inputs and gives exactly the same results as the original.

## Please read this first

* These are **examples to learn from**, not the best possible answers. Some were written by hand, the others by a small prototype optimizer. A good optimizer can often do better.
* The hidden test programs are **different programs**. Copying these files will not help. What helps is understanding **why** each change is allowed.
* Four examples (`dead_div_trap`, `store_then_div_trap`, `sensor_sum`, `alias_hazard`) show what you must **not** remove or move. They save very few cycles on purpose.

## The table (public machine, `machines/machine_default.cfg`, sample input)

| # | Program | Idea | Instructions before -> after | Cycles before -> after |
|---|---|---|---|---|
| 1 | `const_chain` | Constant folding and simple algebra | 35 -> 1 | 39 -> 1 |
| 2 | `sum_sq` | Dead code, copy removal, a fused instruction, fewer registers | 22 -> 10 | 2690 -> 207 |
| 3 | `redundant_loads` | Using a loaded value again | 22 -> 13 | 780 -> 163 |
| 4 | `loop_invariant` | Moving work out of a loop | 22 -> 13 | 2448 -> 191 |
| 5 | `strength_ladder` | Cheaper operations (strength reduction) | 34 -> 16 | 5258 -> 327 |
| 6 | `branchy_select` | Replacing branches with `select` | 30 -> 17 | 1767 -> 1323 |
| 7 | `guarded_div` | Moving work out of a loop together with its guard | 18 -> 6 | 433 -> 17 |
| 8 | `dead_div_trap` | What not to delete | 5 -> 3 | 15 -> 14 |
| 9 | `store_then_div_trap` | Keeping the order of visible effects | 9 -> 5 | 16 -> 14 |
| 10 | `sensor_sum` | Volatile operations stay | 20 -> 14 | 353 -> 73 |
| 11 | `alias_hazard` | A store can change a later load | 12 -> 8 | 22 -> 15 |
| 12 | `addsh_index` | A fused instruction (`addsh`) | 17 -> 13 | 746 -> 182 |
| 13 | `popcount` | Loop unrolling | 13 -> 20 | 262 -> 102 |
| 14 | `unreachable_blocks` | Removing code that never runs | 11 -> 3 | 8 -> 5 |
| 15 | `spill_storm` | Too many values for the registers | 60 -> 46 | 3576 -> 1575 |

Programs written by hand: `const_chain`, `strength_ladder`, `branchy_select`, `guarded_div`, `popcount`. The others came from the prototype optimizer.

## How to look at an example (Linux / macOS)

```
./kvalidate bench/sum_sq.kir examples/sum_sq_opt.kir
./ksim bench/sum_sq.kir
./ksim examples/sum_sq_opt.kir
./kvalidate examples/sum_sq_opt.kir --against bench/sum_sq.kir
```

On Windows use `kvalidate.cmd` and `ksim.cmd`, and write paths with backslashes: `bench\sum_sq.kir`. In PowerShell put `.\` in front.

Open the two `.kir` files side by side and compare them. Use `./ksim FILE --trace 30` to see how the cycles are spent.

## What each example shows

### 1. `const_chain` - Constant folding and simple algebra

Everything cancels: `(x + 5) - 5` is `x`, `* 1` and `+ 0` change nothing, `x * 0` and `x ^ x` are 0. This is true for every x because 32-bit arithmetic wraps around in the same way each time. The whole function becomes `ret v0`.

Result: 35 -> 1 instructions, 39 -> 1 cycles.

### 2. `sum_sq` - Dead code, copy removal, a fused instruction, fewer registers

Two products were never used (deleted). The `mov` copies after each calculation were removed by writing the result straight into its final register. `i*i` plus the running sum became one `mac`. The program now uses only a few register names, so the machine never has to spill. Most of the saved cycles come from this last point.

Result: 22 -> 10 instructions, 2690 -> 207 cycles.

### 3. `redundant_loads` - Using a loaded value again

`m[i]` was loaded four times in every round. Nothing stores to memory in between, so one load is enough. The multiply-add became a `mac`.

Result: 22 -> 13 instructions, 780 -> 163 cycles.

### 4. `loop_invariant` - Moving work out of a loop

`a*b + 7` and its square do not change while the loop runs. They are now computed once, before the loop. This is legal because those instructions are plain arithmetic (they cannot trap) and nothing needs their old values.

Result: 22 -> 13 instructions, 2448 -> 191 cycles.

### 5. `strength_ladder` - Cheaper operations (strength reduction)

`i*8 + i*10 + i*3 + i*16` is just `i*37`, and it is kept up to date by adding 37 each round (no multiplication). `i // 4` became `i >> 2` and `i % 8` became `i & 7`. **This is only allowed because `i` is never negative here** (it counts up from 0). For a value that can be negative, `x // 4` is not `x >> 2`. The program `sem_div_mod` is there to catch this mistake.

Result: 34 -> 16 instructions, 5258 -> 327 cycles.

### 6. `branchy_select` - Replacing branches with `select`

Each `if` only chose between two values that are cheap to compute, so both values are computed and `select` picks one. No jumps inside the loop body. The gain is smaller here, because every round still waits for its `load`.

Result: 30 -> 17 instructions, 1767 -> 1323 cycles.

### 7. `guarded_div` - Moving work out of a loop together with its guard

The original divides `n // d` eight times, but only when `d != 0`. The new code checks `d` once, divides once, and uses the fact that adding the same value 8 times is `8 * q`, and that `0 + 1 + ... + 7 = 28`. The division is still behind the same check. **Moving the division above the check would be wrong:** it would trap when `d` is 0.

Result: 18 -> 6 instructions, 433 -> 17 cycles.

### 8. `dead_div_trap` - What not to delete

The result of `n // d` is never used, but the division stays. If `d` is 0, the original program traps, so the new one must trap too. Only harmless leftovers were removed. The gain is tiny on purpose.

Result: 5 -> 3 instructions, 15 -> 14 cycles.

### 9. `store_then_div_trap` - Keeping the order of visible effects

Two stores come before a division that may trap. If it traps, the memory must already contain those two values, so they must not move after the division. Only copies were removed.

Result: 9 -> 5 instructions, 16 -> 14 cycles.

### 10. `sensor_sum` - Volatile operations stay

The second `vload` in each round is thrown away by the program, but it is still logged, so it must stay. The `vstore` stays too. Only the unused multiplication and the loop clutter were removed.

Result: 20 -> 14 instructions, 353 -> 73 cycles.

### 11. `alias_hazard` - A store can change a later load

The program loads `m[a & 63]`, stores to `m[b & 63]`, and loads `m[a & 63]` again. If `a` and `b` point to the same place, the second load sees the new value. So the second load must stay. Only copies and constants were cleaned up.

Result: 12 -> 8 instructions, 22 -> 15 cycles.

### 12. `addsh_index` - A fused instruction (`addsh`)

`a + (i << 3)` is one `addsh a, i, 3` instead of a shift and an add. Needs a machine with `fused_addsh = 1`. Try it on `machines/machine_no_fused.cfg`: the program must then use `shl` and `add` instead.

Result: 17 -> 13 instructions, 746 -> 182 cycles.

### 13. `popcount` - Loop unrolling

Each round now tests 4 bits instead of 1, so the loop test and the jump run 8 times instead of 32. The program stays small (20 instructions). If you unrolled all 32 steps, the code would be longer than `icache_size` (64) and every extra instruction would cost extra cycles.

Result: 13 -> 20 instructions, 262 -> 102 cycles.

### 14. `unreachable_blocks` - Removing code that never runs

The branch `br 1, live, dead1` always goes to `live`. The other blocks (one of them divides by 0) can never run, so they are deleted. Nobody can trap in code that never executes.

Result: 11 -> 3 instructions, 8 -> 5 cycles.

### 15. `spill_storm` - Too many values for the registers

The original uses 43 register names. By reusing names the optimizer gets it down to 15, which is still more than the 8 hardware registers, so some spill cost remains. A stronger optimizer can do better, for example by shortening how long values stay alive, or by keeping some values in the scratch memory with `store` and `load`.

Result: 60 -> 46 instructions, 3576 -> 1575 cycles.

## One wrong example

`sum_sq_wrong.kir` looks like a good optimization (it is fast: 107 cycles), but its loop always stops at 20 instead of at `n`. It gives 2470 instead of 20540.
Run `./kvalidate examples/sum_sq_wrong.kir --against bench/sum_sq.kir` and read the message. A wrong program fails that test case, however fast it is.

