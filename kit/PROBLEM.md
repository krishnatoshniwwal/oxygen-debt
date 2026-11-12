# PROBLEM.md - Oxygen Debt

You have received the starter kit: 4 documents, 2 tools, 5 machine files, 100 practice programs and 15 solved examples.
This file tells you the story, what is in the kit, and exactly what you must build.

---

## 1. The story

Foundry-9 is an orbital station. Ten thousand machines keep it alive, and all of them run on one strange chip: the **Kiln**.

The Kiln has a problem. **Every CPU cycle it spends burns a little of the station's oxygen.** The station's engineers are brilliant at many things, but they write wasteful code: values are copied for no reason, the same sum is calculated four times, loops do work that could be done once. The oxygen reserve now reads **11 days**.

The station hires a guild of **Cycle Smugglers**. That is you. Your job is to take any program the engineers have written and quietly rewrite it so that it burns fewer cycles.

There are two laws:

* **Law 1.** The rewritten program must do *exactly* what the old one did. One wrong answer and a crew member dies.
* **Law 2.** Your tool is a program too. If it is slow, it burns oxygen while it works.

You will not rewrite programs by hand. You will build **one tool, an optimizer**, that rewrites any program for you, on any Kiln machine.

---

## 2. Your starter kit: what is in it

| File or folder | What it does |
|---|---|
| `README.md` | Start here. Explains every file and shows, step by step, how to run the tools on Windows, macOS and Linux. |
| `PROBLEM.md` | This file: the story and the task. |
| `KIR_SPEC.md` | The exact rules of the language (KIR) and of the Kiln machine, including how cycles are counted. |
| `RULES_AND_SUBMISSION.md` | What is allowed, what to put on GitHub, and how your work is tested. |
| `ksim` and `ksim.cmd` | **Tool 1.** Runs a KIR program on a Kiln machine and tells you the result and the exact number of cycles. |
| `kvalidate` and `kvalidate.cmd` | **Tool 2.** Checks that a KIR file is valid, and checks that your optimized program behaves exactly like the original. |
| `kirlib.py` | The code that both tools share. Read it if you want to see exactly how a rule works. |
| `machines/` | 5 machine files. `machine_default.cfg` is the public machine. The other 4 are practice machines with different numbers. |
| `bench/` | **100 practice programs** (`NAME.kir`), a sample input for each (`NAME.in`), a branch profile for 22 of them (`NAME.prof`), and `INDEX.md`, which says what each program is good for. |
| `examples/` | **15 solved examples**: an optimized version of 15 practice programs, with `EXAMPLES.md` explaining each change. Also one wrong example. |

You do **not** write a language, a parser for source code, or a virtual machine. The language, the machine, the simulator and the checker are all given. You build only the optimizer.

---

## 3. The task

### 3.1 What you must build

A command-line tool called **`kopt`** (the Kiln optimizer).

```
kopt IN.kir --machine MACHINE.cfg [--profile PROFILE.prof] -o OUT.kir
```

* `IN.kir` is a KIR program (the slow one).
* `MACHINE.cfg` describes the Kiln machine: how many cycles each instruction takes, how many registers there are, and so on.
* `PROFILE.prof` is optional. It says which jumps were taken most often in a typical run.
* `OUT.kir` is the program that **your tool writes**: a faster program that does exactly the same thing.

### 3.2 The one big rule: same behaviour

For every possible input, the new program must behave exactly like the old one. This means three things are identical:

1. **The result:** the same returned value, or the same kind of trap (a crash such as division by zero).
2. **The volatile log:** every `vload` and `vstore` happens, in the same order, with the same values.
3. **The memory:** the data memory (words 0 to 3071) holds the same values when the program stops.

Anything else may change: register contents, how many instructions run, how long it takes. The exact definition is in `KIR_SPEC.md`, section 7.

What this means in practice:

* You can **never** delete, repeat, or reorder `vload`, `vstore` or `trap`.
* `div`, `mod`, `load` and `store` can **trap** (division by zero, bad address). Do not delete them, and do not move them in front of the check that protects them, unless you can *prove* they never trap. A division that the original only does when `d != 0` must stay behind that check.
* If the original program traps, the new one must trap in the same way, after the same stores and volatile operations.
* If you cannot improve a program, write it out unchanged. That is always legal.
* A wrong output is much worse than no optimization.

### 3.3 A tiny example

The slow program (4 instructions, 4 cycles):

```
func tiny(v0)
B:
  const v1, 5
  add v2, v0, v1
  sub v3, v2, v1
  ret v3
```

An optimizer should write this (1 instruction, 1 cycle). It is correct because `(x + 5) - 5` is `x` for every x:

```
func tiny(v0)
B:
  ret v0
```

Real programs are bigger, and most of the saving comes from other things: copies that can be removed, values that can share a register, work that can leave a loop, and special fused instructions. Look at `examples/EXAMPLES.md` for 15 real cases.

### 3.4 What you should know before you start

1. **The machine changes.** The judges test your tool on machine files you have never seen: different latencies, a different number of registers, a machine without the fused instructions `mac` and `addsh`, a smaller instruction cache. A trick that is only good for the numbers in `machine_default.cfg` will lose. Read the machine file and decide from it. Never write `mac` or `addsh` for a machine that does not have them.
2. **The programs change.** The hidden test programs are new. Some are practice programs with renamed registers, shuffled blocks and changed numbers. Others are new programs of the same 12 kinds. Some are very large (about 100,000 instructions). Some have odd control flow: loops with two entrances, blocks that can never run.
3. **Your tool must be fast.** `kopt` must finish within **5 seconds** on a normal program and within **30 seconds** on a very large one.
4. **Profiles can be wrong.** A `.prof` file may describe a different run than the one that is measured. Use it as a hint, not as a fact.
5. **Code size matters.** If a program is longer than `icache_size` instructions, each extra instruction that runs costs extra cycles. Copying code without limit makes programs slower.
6. **Same input, same output.** `kopt` must give byte-for-byte the same file every time. Do not use the clock or unseeded random numbers.

---

## 4. Where to go next

1. Open **`README.md`** and follow the first steps. Run `kvalidate` and `ksim` on a practice program before you write any code.
2. Read **`KIR_SPEC.md`**, at least sections 1 to 7 and section 10 (how cycles are counted).
3. Read **`bench/INDEX.md`** and **`examples/EXAMPLES.md`** to see what the practice programs and a good result look like.
4. Read **`RULES_AND_SUBMISSION.md`** before you push your code to GitHub. It tells you what is allowed, exactly which files your repository must have, and how your work is tested.
