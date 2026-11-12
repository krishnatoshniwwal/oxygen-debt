# bench/INDEX.md - what the 100 practice programs are

Every program `NAME.kir` has a sample input `NAME.in`. Some also have a branch profile `NAME.prof` (column "Profile").
This table tells you **what each program is good for practising** and **what to watch out for**.
It does not give any optimized result. The hidden test programs are different programs of the same kinds.

## How to read the table

* **Kind** - the group of the program (see the list below).
* **Good to practise** - kinds of optimization that make this program faster. We found them by running a small prototype optimizer; a program with an empty cell is already tight, or only advanced work helps.
* **Watch out** - a mistake that makes the output wrong on this program. `kvalidate --against` catches it.
* **Profile** - yes if `NAME.prof` exists. Some profiles do not match the sample input on purpose.

## The 12 kinds

* `loops` - Loops and arithmetic (12 programs)
* `arrays` - Arrays and matrices (21 programs)
* `numbers` - Number theory (4 programs)
* `bits` - Bit tricks and hashes (10 programs)
* `state` - State machines (7 programs)
* `junk` - Wasteful code (repeated work, dead code) (12 programs)
* `effects` - Volatile operations, traps and memory effects (15 programs)
* `semantics` - Exact number rules (negative values, wrap-around, shifts) (6 programs)
* `control` - Unusual control flow (4 programs)
* `fused` - Fused-instruction patterns (4 programs)
* `pressure` - Many values alive at once (3 programs)
* `large` - Long straight-line code (2 programs)

## What "Good to practise" means

* **clean-up** - constant folding, dead code, repeated work, simple algebra.
* **copy removal** - the naive code copies results with `mov`; many copies can be removed.
* **fewer registers** - the programs use many register names but the machine has few. Reusing names saves a lot of cycles (see `KIR_SPEC.md`, section 10).
* **fused ops** - `mac` and `addsh` can replace two instructions. Only on machines that have them.
* **moving work out of loops** - a value that does not change inside a loop can be computed once.
* **jump cost and code layout** - many cycles are lost to jumps that leave the block order. Block order, merging blocks and loop shape matter.
* **instruction scheduling** - many cycles are lost waiting for results; reordering instructions can fill the gaps.

## A good order to learn in

1. Start with `const_chain`, `junk_dead`, `common_subexpr` (clean-up).
2. Then `fib`, `array_sum`, `sum_sq` (copies, registers, and the loop itself).
3. Then `loop_invariant`, `nested_invariant`, `strength_ladder` (work inside loops).
4. Then `poly_mac`, `dot_product`, `addsh_index` (fused instructions; also try `machines/machine_no_fused.cfg`).
5. Then `spill_storm`, `matmul4` (more values than registers).
6. Then the `effects` programs: they show what you must **not** do.

## The table

| # | Program | Kind | What it does | Good to practise | Watch out | Profile |
|---|---|---|---|---|---|---|
| 1 | `collatz` | loops | Collatz step count | clean-up, copy removal, fused ops, jump cost and code layout | - | yes |
| 2 | `count_divisors` | loops | number of divisors; the loop bound n+1 is recomputed every time | clean-up, copy removal, moving work out of loops, instruction scheduling | - | yes |
| 3 | `digit_sum` | loops | sum of decimal digits using div and mod | clean-up, copy removal, instruction scheduling | - | - |
| 4 | `factorial_mod` | loops | factorial modulo 1000003 | copy removal, instruction scheduling | - | - |
| 5 | `fib` | loops | iterative Fibonacci (wraps at 32 bits) | clean-up, copy removal, jump cost and code layout | - | - |
| 6 | `gcd` | loops | Euclid gcd with div/mod | clean-up, copy removal, instruction scheduling | - | yes |
| 7 | `horner` | loops | degree-7 polynomial evaluated with Horner, coefficients in memory | clean-up, copy removal, instruction scheduling | - | - |
| 8 | `isqrt` | loops | integer square root by binary search | clean-up, copy removal, fewer registers | - | - |
| 9 | `lcm_pair` | loops | least common multiple using a gcd loop and a division | clean-up, fewer registers, instruction scheduling | - | - |
| 10 | `popcount` | loops | population count by shifting 32 times | copy removal, jump cost and code layout | - | - |
| 11 | `sum_cubes_mod` | loops | sum of cubes with a modulo in every step | clean-up, copy removal, fused ops, instruction scheduling | - | - |
| 12 | `sum_sq` | loops | sum of squares below n, with redundant temporaries | clean-up, fused ops, jump cost and code layout, instruction scheduling | - | yes |
| 13 | `alias_hazard` | arrays | a store and a later load may use the same address (when a and b match) | clean-up, fused ops, instruction scheduling | A store can change what a later load sees. | - |
| 14 | `array_max` | arrays | maximum of an array (signed compare) | copy removal, jump cost and code layout, instruction scheduling | - | - |
| 15 | `array_min` | arrays | minimum of an array (signed compare) | copy removal, jump cost and code layout, instruction scheduling | - | - |
| 16 | `array_sum` | arrays | sum of an array | copy removal, instruction scheduling | - | yes |
| 17 | `binary_search` | arrays | lower-bound binary search in a sorted array | clean-up, copy removal, fewer registers | - | yes |
| 18 | `bubble_sort` | arrays | bubble sort (signed) | clean-up, copy removal, fewer registers, moving work out of loops, instruction scheduling | - | yes |
| 19 | `count_above` | arrays | count the elements greater than a threshold | copy removal, jump cost and code layout, instruction scheduling | - | yes |
| 20 | `dot_product` | arrays | dot product of two vectors at addresses 0 and 32 | copy removal, fused ops, instruction scheduling | - | - |
| 21 | `histogram` | arrays | histogram of low 3 bits into bins at 300..307 | clean-up, copy removal, instruction scheduling | - | - |
| 22 | `insertion_sort` | arrays | insertion sort (signed) | clean-up, copy removal, fewer registers, instruction scheduling | - | yes |
| 23 | `matmul4` | arrays | 4x4 matrix multiply (A at 0, B at 16, C at 32) | clean-up, copy removal, fewer registers, fused ops, instruction scheduling | - | - |
| 24 | `matrix_add4` | arrays | add two 4x4 matrices (at 0 and 16) into addresses 32.. | clean-up, copy removal, fewer registers, instruction scheduling | - | - |
| 25 | `matvec8` | arrays | 8-wide matrix-vector product (A at 0, x at 64, y at 72) | copy removal, fewer registers, fused ops, instruction scheduling | - | - |
| 26 | `merge_sorted` | arrays | merge two sorted arrays (at 0 and 16) into addresses 64.. | clean-up, copy removal, fewer registers, instruction scheduling | - | yes |
| 27 | `prefix_sum` | arrays | in-place prefix sums | instruction scheduling | - | - |
| 28 | `reverse_array` | arrays | reverse an array in place | clean-up, copy removal, instruction scheduling | - | - |
| 29 | `rotate_array` | arrays | rotate an array left by one place | clean-up, copy removal, moving work out of loops, instruction scheduling | - | - |
| 30 | `row_sums` | arrays | row sums of an 8-wide matrix written to addresses 100.. | copy removal, fewer registers, fused ops, instruction scheduling | - | - |
| 31 | `stencil3` | arrays | 3-point stencil written to addresses 101.. | clean-up, fewer registers, fused ops, instruction scheduling | - | - |
| 32 | `transpose8` | arrays | 8x8 transpose (input at 0, output at 128) | clean-up, copy removal, fewer registers, fused ops | - | - |
| 33 | `vec_add` | arrays | vector add into addresses 64.. | copy removal, instruction scheduling | - | - |
| 34 | `is_prime` | numbers | primality test by trial division | clean-up, copy removal, instruction scheduling | - | yes |
| 35 | `pow_fast` | numbers | fast modular exponentiation with a while loop (modulus 46337) | clean-up, copy removal, fewer registers, instruction scheduling | - | - |
| 36 | `powmod` | numbers | modular exponentiation, modulus 46337 | clean-up, copy removal, fewer registers, instruction scheduling | - | - |
| 37 | `sieve` | numbers | sieve of Eratosthenes using flags at 512.. | clean-up, copy removal, fewer registers, jump cost and code layout, instruction scheduling | - | yes |
| 38 | `adler32` | bits | Adler-32 style checksum with two modulo operations per byte | clean-up, copy removal, fewer registers, instruction scheduling | - | - |
| 39 | `bit_reverse` | bits | reverse the bits of a word one at a time | clean-up, copy removal, jump cost and code layout | - | - |
| 40 | `crc_bitwise` | bits | bitwise CRC-32 of the low bytes of an array | copy removal, fewer registers, jump cost and code layout | - | - |
| 41 | `fletcher16` | bits | Fletcher-16 checksum | clean-up, copy removal, fewer registers, instruction scheduling | - | - |
| 42 | `fnv_hash` | bits | FNV-style hash of the low bytes of an array | copy removal, instruction scheduling | - | - |
| 43 | `gray_code_sum` | bits | sum of Gray codes | clean-up, copy removal, jump cost and code layout | - | - |
| 44 | `hamming_dist` | bits | total Hamming distance between two arrays (at 0 and 32) | copy removal, fewer registers, jump cost and code layout | - | - |
| 45 | `parity_fold` | bits | parity of a word by xor folding (straight-line code) | clean-up | - | - |
| 46 | `rotate_mix` | bits | rotate-add-xor mixing rounds | copy removal, fewer registers | - | - |
| 47 | `xorshift` | bits | xorshift32 generator iterated n times | clean-up, copy removal, fewer registers | - | - |
| 48 | `dfa_dispatch` | state | four-state automaton driven by if/elif dispatch | clean-up, copy removal, fewer registers, jump cost and code layout, instruction scheduling | - | yes |
| 49 | `mini_interp` | state | tiny bytecode interpreter: (op, arg) pairs in memory | clean-up, copy removal, fewer registers, jump cost and code layout, instruction scheduling | - | - |
| 50 | `paren_depth` | state | bracket depth: 0 opens, 1 closes; reports max depth and an error flag | clean-up, copy removal, fewer registers, fused ops, jump cost and code layout, instruction scheduling | - | - |
| 51 | `rle_encode` | state | run-length encoding; writes (value, count) pairs from address 100 | copy removal, fewer registers, jump cost and code layout, instruction scheduling | - | - |
| 52 | `run_count` | state | count runs of equal neighbouring values | copy removal, jump cost and code layout, instruction scheduling | - | yes |
| 53 | `vending_fsm` | state | vending machine automaton with nested if/elif on state and event | clean-up, copy removal, fewer registers, fused ops, jump cost and code layout, instruction scheduling | - | yes |
| 54 | `word_count` | state | count words (runs of non-zero low bits) | copy removal, fewer registers, jump cost and code layout, instruction scheduling | - | yes |
| 55 | `alias_stores` | junk | a store may land on an address that is loaded again right after (when i*5 and i agree modulo 64) | clean-up, copy removal, fewer registers, instruction scheduling | A store can change what a later load sees. | - |
| 56 | `branchy_select` | junk | branches that could be selects | copy removal, fewer registers, jump cost and code layout, instruction scheduling | - | yes |
| 57 | `common_subexpr` | junk | one big subexpression written out three times | clean-up, fused ops | - | - |
| 58 | `const_chain` | junk | identity operations and constant arithmetic around x | clean-up | - | - |
| 59 | `constant_branch` | junk | a branch whose condition is a known constant | clean-up, copy removal, moving work out of loops, jump cost and code layout, instruction scheduling | - | - |
| 60 | `dead_overwritten_stores` | junk | stores that are overwritten at once, and a store that must stay | clean-up | - | - |
| 61 | `junk_dead` | junk | dead stores, dead loads and redundant products around a small result | clean-up, copy removal, instruction scheduling | - | - |
| 62 | `loop_invariant` | junk | loop-invariant computation inside a loop | clean-up, copy removal, fewer registers, fused ops, moving work out of loops, jump cost and code layout, instruction scheduling | - | yes |
| 63 | `nested_invariant` | junk | part of the inner loop depends only on the outer variable | clean-up, copy removal, fewer registers, moving work out of loops, fused ops, jump cost and code layout, instruction scheduling | - | - |
| 64 | `partial_redundancy` | junk | an expression that is computed on one path and again after the join | clean-up, copy removal, fused ops | - | yes |
| 65 | `redundant_loads` | junk | the same array element is loaded again and again (no stores in between) | clean-up, copy removal, fused ops, instruction scheduling | - | - |
| 66 | `strength_ladder` | junk | multiplications, divisions and modulos by constants of a loop counter | clean-up, copy removal, fewer registers, fused ops, instruction scheduling | - | - |
| 67 | `bounds_trap` | effects | stores a running sum, then traps when the index reaches 64 | copy removal, instruction scheduling | - | - |
| 68 | `checked_add_trap` | effects | addition with an overflow check that traps | clean-up, jump cost and code layout | - | - |
| 69 | `dead_div_trap` | effects | an unused division sits in the code: it must still trap when d is 0 | clean-up, instruction scheduling | An unused division, remainder or load must stay: it can still trap. | - |
| 70 | `dead_load_trap` | effects | an unused load with a computed address: it must still trap when the address is too large | clean-up, fused ops | An unused division, remainder or load must stay: it can still trap. | - |
| 71 | `dead_mod_in_loop` | effects | an unused remainder inside a loop: it must still trap when d is 0 | clean-up, instruction scheduling | An unused division, remainder or load must stay: it can still trap. | - |
| 72 | `div_trap` | effects | division and remainder on raw arguments: traps when d is 0 | instruction scheduling | - | - |
| 73 | `guarded_div` | effects | division that only happens when the divisor is non-zero | copy removal, instruction scheduling | Do not move a division, load or volatile read out of its loop or its guard. | yes |
| 74 | `index_trap` | effects | index computed from the argument: traps when the address is too large | clean-up | - | - |
| 75 | `log_then_divide` | effects | stores and volatile writes happen before a division that may trap | clean-up, instruction scheduling | Keep the stores and volatile writes that come before a division that may trap. | - |
| 76 | `sensor_drain` | effects | a sensor is read twice per round but only the first reading is used | clean-up, copy removal, instruction scheduling | Unused volatile reads (vload) must stay. Do not move a division, load or volatile read out of its loop or its guard. | - |
| 77 | `sensor_sum` | effects | reads a volatile sensor twice per round (the second reading is dropped) and logs a running total | clean-up, instruction scheduling | Unused volatile reads (vload) must stay. Do not move a division, load or volatile read out of its loop or its guard. | - |
| 78 | `sensor_threshold` | effects | reads a sensor (dropping every second reading) and logs only the large readings | clean-up, copy removal, jump cost and code layout, instruction scheduling | Unused volatile reads (vload) must stay. Do not move a division, load or volatile read out of its loop or its guard. | - |
| 79 | `store_then_div_trap` | effects | three stores, then a division that may trap: the stores must be visible | clean-up, instruction scheduling | Keep the stores and volatile writes that come before a division that may trap. | - |
| 80 | `swap_via_memory` | effects | swap two memory words, then read both again (loads after stores) | clean-up, instruction scheduling | A store can change what a later load sees. | - |
| 81 | `vlog_every_other` | effects | logs every other square to a volatile port | copy removal, jump cost and code layout, instruction scheduling | - | - |
| 82 | `sem_bit_mix` | semantics | bit tricks with not, and, or, xor on raw arguments | - | Uses the raw arguments: negative numbers, wrap-around and big shift amounts all occur. | - |
| 83 | `sem_div_mod` | semantics | signed division and remainder, also by constants, on raw arguments (-2147483648 / -1 included) | clean-up, fused ops, instruction scheduling | Division or remainder by a power of two is not a shift or an and when values can be negative. Keep the stores and volatile writes that come before a division that may trap. Uses the raw arguments: negative numbers, wrap-around and big shift amounts all occur. | - |
| 84 | `sem_mul_wrap` | semantics | multiplication that wraps around 32 bits, then a division by 16 | clean-up, instruction scheduling | Division or remainder by a power of two is not a shift or an and when values can be negative. Uses the raw arguments: negative numbers, wrap-around and big shift amounts all occur. | - |
| 85 | `sem_select_abs` | semantics | absolute value, minimum using select, and division by constants | clean-up, instruction scheduling | Division or remainder by a power of two is not a shift or an and when values can be negative. Uses the raw arguments: negative numbers, wrap-around and big shift amounts all occur. | - |
| 86 | `sem_shifts` | semantics | shifts with raw amounts: only the low 5 bits of the amount count | - | Uses the raw arguments: negative numbers, wrap-around and big shift amounts all occur. | - |
| 87 | `sem_signed_cmp` | semantics | signed comparisons of raw arguments | clean-up, fused ops | Uses the raw arguments: negative numbers, wrap-around and big shift amounts all occur. | - |
| 88 | `irreducible_loop` | control | a loop that can be entered at two different blocks (irreducible control flow) | - | The control-flow graph is unusual: do not assume every loop has one entry or every block can be reached. | yes |
| 89 | `jump_chain` | control | blocks that only contain a jump, and a branch whose two targets are the same block | clean-up, jump cost and code layout, instruction scheduling | The control-flow graph is unusual: do not assume every loop has one entry or every block can be reached. | - |
| 90 | `multi_exit` | control | a loop with three different ways out, each returning something different | - | The control-flow graph is unusual: do not assume every loop has one entry or every block can be reached. | yes |
| 91 | `unreachable_blocks` | control | branch on a constant, dead blocks (one would divide by zero) and a block nobody jumps to | clean-up | The control-flow graph is unusual: do not assume every loop has one entry or every block can be reached. | - |
| 92 | `addsh_index` | fused | a + (i << 3) patterns (shift-and-add) | clean-up, copy removal, fused ops, instruction scheduling | - | - |
| 93 | `conv_mac` | fused | 3-tap convolution with coefficients at 64..66 | clean-up, fewer registers, fused ops, instruction scheduling | - | - |
| 94 | `dot_pairs` | fused | products of neighbouring pairs, accumulated (multiply-add pattern) | clean-up, copy removal, fewer registers, fused ops, instruction scheduling | - | - |
| 95 | `poly_mac` | fused | straight-line Horner polynomial with four coefficients | fewer registers, fused ops, instruction scheduling | - | - |
| 96 | `many_live` | pressure | twelve values are alive at the same time | clean-up | - | - |
| 97 | `spill_storm` | pressure | a loop that carries twelve accumulators | copy removal, fewer registers | - | - |
| 98 | `wide_expression` | pressure | a wide expression tree on four arguments | fewer registers, fused ops | - | - |
| 99 | `unrolled_crc12` | large | twelve unrolled CRC steps over two bytes | copy removal, fewer registers | - | - |
| 100 | `unrolled_poly` | large | a 24-term polynomial written out without a loop (more instructions than the cache holds) | clean-up, instruction scheduling | - | - |
