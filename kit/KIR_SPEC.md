# KIR_SPEC.md - the KIR language and the Kiln machine

This file is the exact rulebook for two things: the language your optimizer reads and writes (**KIR**), and the machine that counts cycles (the **Kiln**).
If anything else you read disagrees with this file, **this file is right**. The tools `ksim` and `kvalidate` follow it exactly (their code is in `kirlib.py`).

Contents: 1 Overview - 2 How a program is written - 3 Values and registers - 4 Memory - 5 Instructions - 6 Traps and volatile I/O - 7 Same behaviour - 8 Input files and profile files - 9 The machine file - 10 How cycles are counted - 11 Limits

---

## 1. Overview

A KIR program is **one function**. The function is made of **blocks**. A block is a list of instructions that ends with a jump, a branch, a return or a trap.

A block never "falls through" into the next block. The program moves from block to block only with `jmp`, `br`, `ret` or `trap`.

The function takes up to 8 numbers as input (the parameters) and gives back one number.

## 2. How a program is written

A program is plain text with one statement per line. A `;` starts a comment that goes to the end of the line. Empty lines are ignored. Instruction names are written in lower case.

```
; sum of 0..n-1 (n is the parameter)
func sum_to(v0)
entry:
  const v1, 0          ; running sum
  const v2, 0          ; counter
  jmp check
check:
  lt v3, v2, v0
  br v3, body, done
body:
  add v1, v1, v2
  add v2, v2, 1
  jmp check
done:
  ret v1
```

The rules:

* The first statement is the header `func NAME(v0, v1, ...)`. The list of parameters can be empty. Parameters are registers, and each one may be listed only once.
* A **label** is `NAME:` alone on a line. A label starts a new block. The first label is the **entry block**: the program starts at its first instruction. A label name uses letters, digits and `_`, does not start with a digit, and does not look like a register (`v7` is not allowed). Every label is different.
* An **instruction** is its name followed by operands separated by commas. It must be inside a block.
* Every block has at least one instruction, ends with **exactly one** terminator (`jmp`, `br`, `ret` or `trap`), and has no terminator anywhere before its last instruction.
* Some blocks may be impossible to reach. A block may be the target of jumps from many places, including loops that you can enter at more than one place.
* A **register** is written `v` and a number without leading zeros: `v0`, `v17`, `v999999`. The biggest allowed number is 999999.
* An **integer literal** is written in decimal (`42`, `-1`) or in hexadecimal with a small `x` (`0xFF`, `-0x10`). It must be between -2147483648 and 4294967295. It is stored modulo 2^32.
* A **source operand** (written `s` below) can be a register or an integer literal.

## 3. Values and registers

* Every value is a 32-bit whole number. Arithmetic wraps around: results are taken modulo 2^32.
* Values are stored as numbers from 0 to 4294967295. A few instructions (`lt`, `div`, `mod`) treat a value `x` that is 2147483648 or more as the negative number `x - 2^32`.
* There are as many **virtual registers** as you want. At the start, every register is 0, except the parameters, which hold the arguments.
* You can write to a register as many times as you like. A KIR program is **not** in SSA form.
* The number of real (hardware) registers is a property of the machine (`registers` in the machine file). It changes how many cycles a program needs (section 10). It never changes what a program computes.

## 4. Memory

* Memory is **4096 words** (32 bits each). The address of a word is its number, 0 to 4095.
* All words start at 0, except the ones set by the input file (section 8).
* Words **0 to 3071** are the **data region**. Words **3072 to 4095** are the **scratch region**. Both work like normal memory, but the scratch region is **not observable** (section 7): your optimizer may use it freely, for example to park values.
* An address is read as an unsigned number. An address of 4096 or more causes the trap `bad_address`. (A "negative" address like -1 is 4294967295, so it also traps.)
* The practice programs never touch the scratch region.

## 5. Instructions

`d` is a destination register. `s` is a source operand (register or literal). `L` is a label. Every result wraps around modulo 2^32.

| Instruction | What it does |
|---|---|
| `const d, lit` | d = lit (the operand must be a literal) |
| `mov d, s` | d = s |
| `add d, a, b` | d = a + b |
| `sub d, a, b` | d = a - b |
| `mul d, a, b` | d = a * b (the low 32 bits) |
| `and d, a, b` / `or d, a, b` / `xor d, a, b` | bit by bit and / or / xor |
| `shl d, a, b` | d = a shifted left by (b & 31) places |
| `shr d, a, b` | d = a shifted right by (b & 31) places, filling with zeros (**logical** shift) |
| `lt d, a, b` | d = 1 if a < b when both are read as **signed** numbers, otherwise 0 |
| `eq d, a, b` | d = 1 if a equals b, otherwise 0 |
| `select d, c, a, b` | d = a if c is not 0, otherwise d = b (it reads all three, and nothing else happens) |
| `div d, a, b` | signed division, rounded toward zero. Traps with `div_by_zero` if b is 0. -2147483648 / -1 gives -2147483648 |
| `mod d, a, b` | the remainder, with the sign of a, so that `a == (a div b) * b + (a mod b)`. Traps with `div_by_zero` if b is 0. -2147483648 mod -1 is 0 |
| `mac d, a, b, c` | d = a * b + c in one step (fused multiply-add). Only if `fused_mac = 1` |
| `addsh d, a, b, k` | d = a + (b shifted left by k) in one step. `k` must be a **literal** from 0 to 31. Only if `fused_addsh = 1` |
| `load d, a` | d = the memory word at address a. Traps with `bad_address` if a is 4096 or more |
| `store a, v` | the memory word at address a becomes v. Traps with `bad_address` if a is 4096 or more |
| `vload d, p` | volatile read from port p (section 6) |
| `vstore p, v` | volatile write of v to port p (section 6) |
| `trap c` | stop with the trap `explicit(c)`. c is read as an unsigned number |
| `jmp L` | go to the first instruction of block L |
| `br c, Lt, Lf` | go to block Lt if c is not 0, otherwise go to block Lf |
| `ret v` | stop and give back v |

A terminator (`jmp`, `br`, `ret`, `trap`) must be the last instruction of its block.

All source operands are read **before** the destination is written. So `add v1, v1, v2` is fine: it uses the old value of v1.

Examples (checked with `ksim`):

| Instruction | Result |
|---|---|
| `shl d, 1, 33` | 2 (the shift amount is 33 & 31 = 1) |
| `shr d, 0x80000000, 31` | 1 |
| `lt d, -1, 1` | 1 (signed compare) |
| `div d, -7, 2` | 4294967293, which is -3 |
| `mod d, -7, 2` | 4294967295, which is -1 |
| `mod d, 7, -2` | 1 |
| `div d, 0x80000000, -1` | 2147483648 (that is -2147483648) |
| `mac d, 6, 7, 100` | 142 |
| `addsh d, 1, 3, 4` | 49 (1 + 3 * 16) |

## 6. Traps and volatile I/O

**Traps.** A trap stops the program at once. There are three kinds:

* `div_by_zero` - `div` or `mod` with a divisor of 0
* `bad_address` - `load` or `store` with an address of 4096 or more
* `explicit(c)` - the `trap c` instruction, where c is the unsigned value of its operand

Everything before the trapping instruction has already happened: its stores are in memory and its volatile operations are in the log. The trapping instruction itself does nothing else.

**Volatile I/O.** A port is a number. Volatile instructions pretend to talk to devices:

* `vstore p, v` adds the entry *(store, p, v)* to the **volatile log**.
* `vload d, p` takes the next value from the input queue of port p and puts it in d. (The queues come from `vin` lines in the input file. If the queue is empty, the value is 0.) It also adds the entry *(load, p, value)* to the volatile log.

The volatile log can be seen from outside. So you can never remove, add, repeat or reorder `vload` and `vstore`, and you can never change their operands. What counts is the log that the finished run produces.

## 7. Same behaviour

Two runs of a program on the same input are **the same** when all three of these are identical:

1. **The ending:** either `RET` with the same returned value, or `TRAP` with the same kind of trap.
2. **The volatile log:** the same entries in the same order (up to the point where the program stopped).
3. **The data region** (words 0 to 3071) at the moment the program stopped.

These things are **not** looked at: the registers, the scratch region, how many instructions ran, how many cycles it took.

An optimized program must be the same as the original on **every** input. What this means:

* A `div`, `mod`, `load` or `store` that might trap on some input must not be removed, moved in front of the check that guards it, or moved past another visible effect (a store, a volatile operation or another trap). The one exception: you can *prove* it never traps. Removing an unused `load` is only allowed if you can prove that its address is always below 4096.
* If the original traps, the new program must trap in the **same way**, after the **same earlier stores and volatile operations**.
* If the original runs forever (or for longer than the step limit), nothing is asked of the new program on that input.

`kvalidate OPT.kir --against ORIG.kir` checks this by running both programs on many inputs (see `README.md`).

## 8. Input files and profile files

An **input file** (`.in`) gives the arguments and the starting memory and device values. A `#` starts a comment.

```
args 40 7            # the parameters, in order (needed if the function has parameters)
mem 0 5 3 8 1        # from address 0 on, set the words to 5 3 8 1 (can be repeated)
vin 1 10 20 30       # the input queue of port 1: 10, then 20, then 30 (can be repeated)
```

The number of `args` values must be the same as the number of parameters. Values are written like literals in `.kir` files. Memory that is not mentioned is 0.

A **profile file** (`.prof`) says how often each jump between blocks happened in a typical run:

```
# comment
edge for_head1 for_body2 40      # the jump from block for_head1 to block for_body2 happened 40 times
```

Each `edge FROM TO COUNT` line names two blocks of the program. A profile can be missing, incomplete, or **wrong for the input that is measured**. Use it as a hint, never as a fact.

## 9. The machine file

A machine is described by a text file of `key = value` lines (a `#` starts a comment). **Every key below must be present exactly once.** Values are whole numbers that are not negative. `machines/machine_default.cfg` is the public machine. The other four files in `machines/` are practice machines. **The judges also use machines you have never seen: the same keys with other values.** So your optimizer must read the file.

| Key | Allowed | What it means (default value) |
|---|---|---|
| `issue_width` | >= 1 | How many instructions can start in one cycle (2) |
| `mem_ports` | >= 1 | How many `load`, `store`, `vload`, `vstore` instructions can start in one cycle (1) |
| `registers` | >= 4 | K, the number of hardware registers (8) |
| `reg_spill_penalty` | >= 0 | Extra cycles when a spilled register is used again (5) |
| `taken_branch_penalty` | >= 0 | Extra cycles after a jump or branch that does not go to the next block in the file (2) |
| `icache_size` | >= 1 | How many instructions fit in the instruction cache (64) |
| `icache_penalty` | >= 0 | Extra fetch cycles for each executed instruction that lies beyond the cache (2) |
| `lat_alu` | >= 1 | Latency of `const mov add sub and or xor shl shr lt eq select` (1) |
| `lat_mul` | >= 1 | Latency of `mul` (3) |
| `lat_div` | >= 1 | Latency of `div` and `mod` (12) |
| `lat_load` | >= 1 | Latency of `load` and `vload` (4) |
| `lat_mac` | >= 1 | Latency of `mac` (3) |
| `lat_addsh` | >= 1 | Latency of `addsh` (1) |
| `fused_mac` | 0 or 1 | 1 if the `mac` instruction exists (1) |
| `fused_addsh` | 0 or 1 | 1 if the `addsh` instruction exists (1) |

"Latency" means: how many cycles pass until the result can be used.

If `fused_mac` is 0, a program that contains `mac` is **invalid** on that machine (`ksim` and `kvalidate --machine` refuse it). The same is true for `fused_addsh` and `addsh`.

The memory size (4096 words) and the split into data and scratch are **not** machine settings. They are the same on every machine.

## 10. How cycles are counted

`ksim` first runs the program to find out which instructions run, and in which order. (We call this the **run order**.) Then it gives every instruction in the run order an **issue cycle**: the cycle in which it starts. Cycles are numbered from 0.

The machine is **in order**: an instruction never starts before the instruction that ran just before it.

For each instruction *i* in the run order, let *prev* be the issue cycle of the instruction before it (0 for the first one).

1. **Delay before the instruction can start (F).** Start with F = 0.
   * *Instruction cache.* The **index** of an instruction is its place in the file, counted from 0 over all blocks in the order of the file (labels and comments are not counted). If the index is `icache_size` or more, add `icache_penalty` to F.
   * *Register file.* See "Registers" below. Each use of a register that was spilled adds `reg_spill_penalty` to F.
2. **Earliest cycle E** = the biggest of these:
   * *prev* + F
   * the **fence** (set by a taken jump, see step 5)
   * for every **register** that the instruction reads: the cycle in which that register's value is ready
   * for the **destination register**: the cycle in which its old value is ready (so that writes finish in order)
   
   Literals are always ready.
3. **Slot.** The instruction starts in cycle *E* if there is room in that cycle: fewer than `issue_width` instructions have already started in cycle *E*, and, if it is a memory instruction (`load`, `store`, `vload`, `vstore`), fewer than `mem_ports` memory instructions have already started in cycle *E*. If there is no room, it starts in cycle *E* + 1, which is always empty.
4. **Result ready.** If the instruction has a destination register, that register is ready at (issue cycle) + (latency of the instruction). An instruction that needs it can start in that very cycle, not before.
5. **Taken jumps.** After a `jmp` or `br` runs, look at the instruction that runs next. If it is **not** the instruction right after it in the file (index + 1), the jump was *taken*, and no later instruction may start before the cycle (issue cycle of the jump) + 1 + `taken_branch_penalty`. A jump to the next block in the file costs nothing, and so does a `br` whose chosen target is the next block in the file. `ret` and `trap` end the program.

**Total cycles** = (issue cycle of the last instruction that ran) + 1.

`store`, `vstore`, `jmp`, `br`, `ret` and `trap` have no destination register, so they have no "result ready" time.

**Registers.** Let N be the number of different register names that appear anywhere in the program text (parameters count, even if they are never used).

* If N is not more than `registers`, registers cost nothing.
* If N is more than `registers`, the machine keeps `registers` values in hardware and spills the rest by itself. It remembers which registers are **resident** (in hardware) and which one was used least recently. Before an instruction starts, it **touches** the registers of that instruction in this order: the registers it reads, from left to right, then its destination register (each register once). Touching a register:
  * If the register is resident: it becomes the most recently used. No cost.
  * If it is not resident: when it was **spilled** earlier, add `reg_spill_penalty` to F and un-spill it. (A register that was never touched before is free.) Then, if `registers` registers are resident already, the least recently used one is thrown out (it becomes *spilled*). Finally the touched register becomes resident.
  * At the start, the parameters are touched in the order of the `func` header, with no cost.

So a program that uses at most `registers` different register names pays nothing. Using the same names again and again (register allocation) removes this cost. You can also spill by hand with `store` and `load` to the scratch region; they cost what every `store` and `load` costs.

### Worked examples (output of `ksim --trace`)

**Waiting for results, and two instructions in one cycle.**

```
func demo(v0)
B0:
  mul v1, v0, v0
  add v2, v0, 1
  add v3, v1, v2
  ret v3
```

| Step | Instruction | Issue cycle | Why |
|---|---|---|---|
| 0 | `mul v1, v0, v0` | 0 | v1 is ready at 0 + 3 = 3 |
| 1 | `add v2, v0, 1` | 0 | second slot of cycle 0; v2 is ready at 1 |
| 2 | `add v3, v1, v2` | 3 | waits for v1 |
| 3 | `ret v3` | 4 | v3 is ready at 3 + 1 = 4 |

Total cycles = 4 + 1 = **5**.

**A jump that is taken, and one that is not.**

```
func demo(v0)
B0:
  br v0, T, F
T:
  ret 1
F:
  ret 2
```

With v0 = 1 the branch goes to T, which is the next block: `br` and `ret` both start in cycle 0, so the total is **1** cycle. With v0 = 0 the branch goes to F, which is not the next block: `ret 2` cannot start before 0 + 1 + 2 = 3, so the total is **4** cycles.

**A loop** (the `sum_to` program of section 2, with argument 5). In every round, the `jmp check` at the end of `body` is a taken jump, because `check` is not the next block. In this part of the trace, `jmp check` starts in cycle 3, and the next `lt` cannot start before 3 + 1 + 2 = 6:

```
  #5    body   add v1, v1, v2        cycle 2
  #6    body   add v2, v2, 1         cycle 3
  #7    body   jmp check             cycle 3
  #3    check  lt v3, v2, v0         cycle 6
```

The whole run takes 31 cycles.

## 11. Limits

* A program has at most 500,000 instructions and at most 8 parameters.
* `ksim` stops a run after `--max-steps` instructions (default 5,000,000) and reports `TIMEOUT`.
* `kvalidate --against` runs the original for at most `--max-steps` instructions per input (default 200,000) and skips inputs where the original does not finish. The new program may run up to 20 times as many instructions as the original plus 1,000.
