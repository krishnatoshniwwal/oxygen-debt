# RULES_AND_SUBMISSION.md - rules, GitHub files and testing

Read this file before you push your code. It has three parts: the rules, what must be on GitHub, and how your code is tested.

---

## 1. The rules

**Team size.** 1 to 3 people. Each team submits once.

**Language.** Any programming language you like. The two tools in this kit are written in Python, but your optimizer can be written in anything that runs on Linux.

**You may:**

* use the standard library of your language, and normal libraries for lists, maps, graphs and so on;
* read papers, books and lecture notes (write down what you used in `DESIGN.md`);
* copy code from this kit (`kirlib.py` has a parser, an interpreter and the timing model) into your own project, or port it to your language. If you do, say so in `DESIGN.md`;
* write your own test programs, test scripts and machine files.

**You may not:**

* use a compiler or optimizer framework or library: LLVM, GCC or Clang internals, Cranelift, MLIR, Binaryen, or anything like them;
* use an SMT or SAT solver (Z3 and similar);
* copy optimizer code from other projects or from other teams;
* recognize a program by its name, a hash or any other fingerprint and then write a stored answer. `kopt` must work from the program it is given. A team that does this is disqualified;
* read any file except the ones named on the command line and your own source files, or use the network.

**Your optimizer must be deterministic.** The same command and the same files must always give exactly the same output file. Do not use the clock or random numbers without a fixed seed.

**Time limit for `kopt`.** 5 seconds per program for normal programs, 30 seconds for the very large ones (about 100,000 instructions). This is real (wall-clock) time on the judges' computer, including the start-up of your program.

**The output must be valid.** The file that `kopt` writes must pass

```
kvalidate OUT.kir --against IN.kir --machine MACHINE.cfg
```

and must also behave like the input on the much larger set of inputs that the judges use.

**Fused instructions.** If the machine file says `fused_mac = 0` or `fused_addsh = 0`, the output must not contain `mac` or `addsh`.

**The judges' computer.** Linux x86-64 (Ubuntu 22.04), 8 GB of memory, one core for each run, no network. Installed: Python 3.10 or newer, gcc and g++ 11 or newer, OpenJDK 17, Rust (stable), Go 1.21. If you need anything else, your `build.sh` must get it or build it from files inside your repository.

---

## 2. What must be on GitHub

You submit **the link to one GitHub repository**. Send it to the organizers before the deadline. The repository must be public, or private with read access given to the organizers (the organizers will tell you the account name). The judges use the **last commit made before the deadline**. Anything you push later is ignored.

### 2.1 Files that MUST be in the repository

All four must be in the top folder of the repository (not inside a sub-folder).

| File | What it must be |
|---|---|
| `build.sh` | A script the judges run with `./build.sh` from the top folder. It must finish with exit status 0 in under 10 minutes. If your language needs no build step (for example Python), the file can contain just two lines: `#!/bin/sh` and `exit 0`. |
| `kopt` | The program the judges run. A file named exactly `kopt` (no `.py`, no `.exe`) in the top folder. After `./build.sh`, the command `./kopt IN.kir --machine M.cfg [--profile P.prof] -o OUT.kir` must work from any current folder. It can be a compiled program or a script. A script must start with a line like `#!/usr/bin/env python3`. |
| `README.md` | Your own README: how to build and run `kopt`, in a few lines. Someone who has never seen your code must be able to do it in under 5 minutes. (This is **your** README, not the one from the starter kit.) |
| `DESIGN.md` | Your write-up. It must contain the 7 headings listed in section 4 below. |

### 2.2 Files that are welcome

* Your source code, in any folders you like (for example `src/`).
* `tests/`: your own small test programs, test scripts, and your own machine files.
* `.gitattributes`: see the tip below.

### 2.3 Files that must NOT be in the repository

* Copies of the starter kit's `bench/`, `examples/` or `machines/` folders. They are big and the judges already have them. (Put your own small tests in `tests/` instead.)
* Files that `build.sh` creates: `.o`, `.class`, `target/`, `node_modules/`, `__pycache__/`, compiled `kopt` binaries that you build yourself, and so on. Use a `.gitignore` file.
* Passwords, keys, and anything private.

A suggested layout:

```
my-optimizer/
  build.sh
  kopt
  README.md
  DESIGN.md
  .gitattributes
  src/
    ...your code...
  tests/
    ...your own test programs...
```

### 2.4 Tips if you work on Windows (the judges use Linux)

Three things go wrong again and again. Check them before you submit.

1. **The execute bit.** Linux must be allowed to run `kopt` and `build.sh`. On Windows, run this once before you commit:

   ```
   git update-index --add --chmod=+x kopt build.sh
   ```

2. **Line endings.** A script with Windows line endings (CRLF) fails on Linux with the message `bad interpreter` or `cannot execute`. Make a file called `.gitattributes` in the top folder with these three lines:

   ```
   * text=auto
   *.sh text eol=lf
   kopt text eol=lf
   ```

3. **Test on Linux before you submit.** Use WSL (Windows Subsystem for Linux), a Linux virtual machine, or a friend's Linux computer. Do a fresh test like this:

   ```
   git clone YOUR-REPOSITORY-LINK check
   cd check
   ./build.sh
   ./kopt PATH-TO-KIT/bench/sum_sq.kir --machine PATH-TO-KIT/machines/machine_default.cfg -o /tmp/out.kir
   PATH-TO-KIT/kvalidate /tmp/out.kir --against PATH-TO-KIT/bench/sum_sq.kir --machine PATH-TO-KIT/machines/machine_default.cfg
   ```

   If the last command prints `OK`, the basics work.

### 2.5 The `kopt` command, exactly

```
kopt IN.kir --machine MACHINE.cfg [--profile PROFILE.prof] -o OUT.kir
```

* Exit with status 0 when you wrote `OUT.kir`. Messages for people may go to the error output.
* `--profile` is optional. Your program must work without it.
* If you cannot improve a program, write it out unchanged.
* Write only `OUT.kir`. Do not leave other files behind.

---

## 3. How your code is tested

After you send the link to your GitHub repository, the judges test your code on **hidden test cases**. You do not see these cases before the end. The practice programs in `bench/` are there so that you can prepare for them.

**What a test case is.** One program, one machine file, and sometimes a profile.

**What the hidden test cases are made of.**

* Practice programs with renamed registers, shuffled blocks and changed numbers.
* New programs of the same kinds as in `bench/INDEX.md`.
* Some very large programs (about 100,000 instructions).
* Different machines: the public machine, the practice machines in `machines/`, and machines you have never seen (other numbers in the same file format).
* Some test cases come with a profile, and some profiles are wrong on purpose.

**What the judges do for each test case.**

1. Run `./build.sh` once, then run `./kopt` on the test case, with the time limit (5 seconds, or 30 seconds for the very large programs). A crash, a time-out or a missing output file means that test case fails.
2. Run `kvalidate OUT --against IN --machine M` with many more inputs than you have: edge values (0, 1, -1, the biggest and smallest numbers), random values, and inputs that make the original trap. Any difference, or an invalid file, means that test case fails.
3. Count the cycles with `ksim`, on hidden inputs. Fewer cycles is better.
4. Run `kopt` a second time and check that the output file is identical.

The judges also read your `DESIGN.md` and look at your code.

The judges always use their own, unchanged copies of `ksim`, `kvalidate` and `kirlib.py`. They ignore any stamp files or results that you bring.

---

## 4. What `DESIGN.md` must contain

Use these 7 headings, in this order. Write in simple words and be honest.

1. **What is implemented.** List every optimization and analysis you built, and say what is missing or only half working.
2. **Pass order.** The order in which your passes run, and why.
3. **Each pass.** For every pass: how it works, how long it takes for a program of size n, and which fact makes each kind of change safe.
4. **The machine.** How you read the machine file and which decisions change with it (for example the number of registers, whether `mac` is used, how much to unroll).
5. **Keeping the behaviour.** How you keep traps, volatile operations and memory effects exactly as they were.
6. **Profiles.** How you use the profile, and what happens when it is wrong.
7. **Known problems.** Anything you know is wrong or slow.

---

## 5. Questions

If a rule or the specification is not clear, ask the organizers. Answers that change or explain the problem are sent to all teams.
