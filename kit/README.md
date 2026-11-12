# README - start here

This folder is the **starter kit** for the Oxygen Debt hackathon problem. You will build an optimizer (`kopt`) that makes KIR programs run in fewer cycles. This file shows you what is in the kit, and how to run the two tools (`kvalidate` and `ksim`) on **Windows, macOS and Linux**, step by step.

Read `PROBLEM.md` first if you have not yet. It tells you the story and the task.

---

## 1. What is in this folder

```
oxygen-debt-starter/
  README.md                   this file
  PROBLEM.md                  the story and the task
  KIR_SPEC.md                 the exact rules of the language and the machine
  RULES_AND_SUBMISSION.md     rules, GitHub files, how your code is tested
  ksim        ksim.cmd        tool 1: run a program, count its cycles
  kvalidate   kvalidate.cmd   tool 2: check a program, compare it with the original
  kirlib.py                   code used by both tools
  machines/                   5 machine files (.cfg)
  bench/                      100 practice programs (.kir) + inputs (.in) + profiles (.prof) + INDEX.md
  examples/                   15 solved examples + EXAMPLES.md + 1 wrong example
```

### Which files can I change, and what goes on GitHub?

| File or folder | What it is | Can I change it? | Put it on my GitHub? |
|---|---|---|---|
| `README.md`, `PROBLEM.md`, `KIR_SPEC.md`, `RULES_AND_SUBMISSION.md` | The documents | **No.** Read only. (Your repository needs its **own** README.md.) | No |
| `ksim`, `ksim.cmd`, `kvalidate`, `kvalidate.cmd` | The two tools | **No.** The judges use their own copies, so a changed tool changes nothing in the judging. | No |
| `kirlib.py` | Code used by the tools | **No** (do not edit the kit's copy). You **may copy** it, or port it to your language, for your own optimizer. | Only your own copy, if you use it. Say so in `DESIGN.md`. |
| `machines/machine_default.cfg` | The public machine | **No.** | No |
| `machines/` (the other 4 files) | Practice machines | **No.** To try other numbers, **copy** a file and change the copy. | No. Your own copies may go in `tests/`. |
| `bench/` (all files) | The 100 practice programs | **No.** To make your own test programs, copy a program and change the copy. | No. Your own tests may go in `tests/`. |
| `examples/` (all files) | The solved examples | **No** | No |
| `.ok` files (created next to a program by `kvalidate`) | Stamps | They are made by the tool. Delete them whenever you like. | **No** |
| **Your own code, tests, README.md, DESIGN.md, build.sh, kopt** | Your work | **Yes** | **Yes**, see `RULES_AND_SUBMISSION.md`, section 2 |

---

## 2. Before you begin: install Python 3

Both tools are Python programs. You need **Python 3.8 or newer**.

**Windows**
1. Go to https://www.python.org/downloads/ and download Python 3.
2. Run the installer. **Tick the box "Add python.exe to PATH"**, then click "Install Now".
3. Close and re-open your terminal. Check it: type `python --version` (or `py -3 --version`). You should see `Python 3.` followed by a number.

**macOS**
1. Open the Terminal app and type `python3 --version`.
2. If it shows `Python 3.` and a number, you are done. If macOS offers to install the "command line developer tools", accept. Or install Python 3 from https://www.python.org/downloads/.

**Linux (Ubuntu)**
1. Type `python3 --version`.
2. If it is missing: `sudo apt update` and then `sudo apt install python3`.

---

## 3. How to type the commands on your computer

The commands are a little different on each system. Use this table.

| What you want | Linux and macOS | Windows: Command Prompt (cmd) | Windows: PowerShell |
|---|---|---|---|
| Run the checker | `./kvalidate bench/sum_sq.kir` | `kvalidate.cmd bench\sum_sq.kir` | `.\kvalidate.cmd bench\sum_sq.kir` |
| Run the simulator | `./ksim bench/sum_sq.kir` | `ksim.cmd bench\sum_sq.kir` | `.\ksim.cmd bench\sum_sq.kir` |
| Show only some lines | `./ksim bench/sum_sq.kir \| grep cycles` | `ksim.cmd bench\sum_sq.kir \| findstr cycles` | `.\ksim.cmd bench\sum_sq.kir \| Select-String cycles` |
| **Works on every system** (if the line above fails) | `python3 ksim bench/sum_sq.kir` | `python ksim bench\sum_sq.kir` (or `py -3 ksim bench\sum_sq.kir`) | the same as cmd |

The rest of this file writes the commands in the **Linux / macOS** form (`./ksim ...`). On Windows:

* write `ksim.cmd` and `kvalidate.cmd` instead of `./ksim` and `./kvalidate` (in PowerShell: `.\ksim.cmd`);
* write paths with a backslash: `bench\sum_sq.kir` (a slash `bench/sum_sq.kir` also works);
* use `findstr` instead of `grep`.

---

## 4. Step by step: your first run

### Step 1. Open a terminal in the kit folder

* **Windows:** unzip the file (right-click, **Extract All**). Open the folder `oxygen-debt-starter` in File Explorer. Click the address bar at the top, type `cmd`, and press Enter. A black window opens in that folder. (Or right-click an empty place in the folder and choose **Open in Terminal**.)
* **macOS:** unzip the file (double-click it). Open **Terminal**. Type `cd ` (with a space), drag the folder `oxygen-debt-starter` into the Terminal window, and press Enter.
* **Linux:** unzip with `unzip oxygen-debt-starter.zip`, then type `cd oxygen-debt-starter`.

Check that you are in the right folder by listing the files: type `ls` (Linux, macOS, PowerShell) or `dir` (Windows cmd). You must see `ksim`, `kvalidate`, `bench` and `README.md`.

### Step 2. Check a program with `kvalidate`

`kvalidate` checks that a KIR file is written correctly. **You must do this before `ksim` will run the file.**

*Linux / macOS*
```
./kvalidate bench/sum_sq.kir
```
*Windows cmd*
```
kvalidate.cmd bench\sum_sq.kir
```
*Windows PowerShell*
```
.\kvalidate.cmd bench\sum_sq.kir
```

You should see:
```
OK bench/sum_sq.kir (function sum_sq, 22 instructions, 4 blocks)
```
(On Windows the path has a backslash.) The tool has also made a small file `bench/sum_sq.kir.ok`. This "stamp" tells `ksim` that the program was checked.

### Step 3. Run the program with `ksim`

*Linux / macOS*
```
./ksim bench/sum_sq.kir
```
*Windows cmd*
```
ksim.cmd bench\sum_sq.kir
```
*Windows PowerShell*
```
.\ksim.cmd bench\sum_sq.kir
```

You should see:
```
program: sum_sq
machine: machine_default.cfg
input: bench/sum_sq.in
status: RET
ret: 20540
vlog_entries: 0
vlog_hash: e3b0c44298fc1c14
mem_hash: f3cc103136423a57
steps: 609
cycles: 2690
```

What the lines mean:

| Line | Meaning |
|---|---|
| `program` | The name of the function. |
| `machine` | The machine file that was used. (If you do not give `--machine`, it is `machines/machine_default.cfg`.) |
| `input` | The input file that was used. If you do not give `--input`, `ksim` uses the file with the same name and the ending `.in` next to the program. |
| `status` | `RET` (it returned a value), `TRAP ...` (it crashed in a defined way), or `TIMEOUT` (it ran too long). |
| `ret` | The value that was returned (a number from 0 to 4294967295). |
| `vlog_entries`, `vlog_hash` | How many volatile operations happened, and a short fingerprint of all of them. |
| `mem_hash` | A short fingerprint of the data memory at the end. |
| `steps` | How many instructions ran. |
| `cycles` | **The number your optimizer wants to make smaller.** |

The lines from `status` to `mem_hash` are the **behaviour** of the program. If an optimized program prints the same values there as the original, on the same input, it behaved the same. `steps` and `cycles` are allowed to be different. That is the whole point.

### Step 4. See the "validate first" rule

Make a copy of a program and try to run it before it is checked.

*Linux / macOS*
```
cp bench/gcd.kir mine.kir
./ksim mine.kir
```
*Windows (cmd or PowerShell)*
```
copy bench\gcd.kir mine.kir
ksim.cmd mine.kir
```
(In PowerShell: `.\ksim.cmd mine.kir`.)

`ksim` refuses and says: `ERROR: mine.kir has not been validated. Run: kvalidate mine.kir`. Now do as it says:

```
./kvalidate mine.kir
./ksim mine.kir
```
It runs. Now change the file and try again. Open `mine.kir` in a text editor, add a comment line like `; hello` at the end, and save. Run `./ksim mine.kir` again. You get `ERROR: mine.kir changed since it was validated`. Every time you change a file, you must run `kvalidate` again.

### Step 5. See where the cycles go

```
./ksim bench/sum_sq.kir --trace 6
```
(Windows: `ksim.cmd bench\sum_sq.kir --trace 6`.) This prints the first 6 instructions that ran, and the cycle in which each one started. After the normal output you see:

```
trace (step, instruction index, block, instruction, issue cycle):
      0  #0    entry          const v1, 63                       cycle 0
      1  #1    entry          and v2, v0, v1                     cycle 1
      2  #2    entry          mov v0, v2                         cycle 2
      3  #3    entry          const v3, 0                        cycle 2
      4  #4    entry          const v4, 0                        cycle 3
      5  #5    entry          jmp for_head1                      cycle 3
```
Look at step 1: `and` needs the value of `v1`, which `const` makes ready in cycle 1, so `and` cannot start earlier. Use a bigger number (for example `--trace 40`) to see more. Section 10 of `KIR_SPEC.md` explains why each instruction starts when it does.

### Step 6. Check all 100 practice programs at once

*Linux / macOS*
```
./kvalidate bench/*.kir
```
*Windows cmd*
```
kvalidate.cmd bench\*.kir
```
*Windows PowerShell*
```
.\kvalidate.cmd bench\*.kir
```
You should get 100 lines that start with `OK`. (Windows does not expand `*` by itself; `kvalidate` does it for you.)

To see the result of every program on your screen:

*Linux / macOS*
```
for f in bench/*.kir; do ./ksim $f | grep -E "program|status|cycles"; done
```
*Windows cmd* (type this exactly, on one line)
```
for %f in (bench\*.kir) do @ksim.cmd %f | findstr "program status cycles"
```
*Windows PowerShell*
```
Get-ChildItem bench\*.kir | ForEach-Object { .\ksim.cmd $_.FullName | Select-String "program|status|cycles" }
```
All programs end with `status: RET`, except `bounds_trap` and a few others that end with a `TRAP` on purpose (traps are part of the problem).

---

## 5. Step by step: check an optimized program

This is what you will do again and again while you build your optimizer. We use a solved example, `examples/sum_sq_opt.kir`, in the place of your optimizer's output.

### Step 1. Check that it behaves like the original

```
./kvalidate examples/sum_sq_opt.kir --against bench/sum_sq.kir
```
(Windows: `kvalidate.cmd examples\sum_sq_opt.kir --against bench\sum_sq.kir`.)

You should see:
```
OK examples/sum_sq_opt.kir behaves like bench/sum_sq.kir on 101 inputs (0 skipped because the original did not finish within 200000 steps)
Inputs used include: bench/sum_sq.in
Note: passing here does not prove equivalence; the judges test many more inputs.
```
`kvalidate` ran both programs on 101 inputs (the sample input and 100 made-up ones) and compared the result, the volatile log and the memory each time. It also made the stamp file, so that `ksim` can run the optimized program.

### Step 2. Compare the cycles

```
./ksim bench/sum_sq.kir | grep cycles
./ksim examples/sum_sq_opt.kir | grep cycles
```
You should see `cycles: 2690` for the original and `cycles: 207` for the optimized one. (The optimized file has no input file next to it, so for your own files add `--input bench/sum_sq.in`. The solved examples have their own `.in` file.)

### Step 3. See a wrong program being caught

```
./kvalidate examples/sum_sq_wrong.kir --against bench/sum_sq.kir
```
You see `MISMATCH ... original gives RET 20540, optimized gives RET 2470` and the input that shows the problem. A wrong program fails that test case, however fast it is.

### Step 4. Try another machine

```
./ksim bench/sum_sq.kir --machine machines/machine_few_registers.cfg | grep cycles
```
The original now needs 4544 cycles (it has only 4 registers). The optimized example still needs 207 because it uses few register names. Try all five machine files in `machines/`.

### Step 5. See a machine without the fused instruction

```
./kvalidate examples/sum_sq_opt.kir --machine machines/machine_no_fused.cfg
```
You see `ERROR ... 'mac' is not available on this machine (fused_mac = 0)`. This machine has no `mac`, so the example would be invalid there. **Your optimizer must read the machine file** and must not write `mac` or `addsh` for a machine that does not have them.

### Step 6. Your daily loop

Run your own optimizer on every practice program, and check each result. Replace `./kopt` with the way you start **your** optimizer.

*Linux / macOS*
```
mkdir -p out
for f in bench/*.kir; do
  n=$(basename $f .kir)
  ./kopt $f --machine machines/machine_default.cfg -o out/$n.kir
  ./kvalidate out/$n.kir --against $f --machine machines/machine_default.cfg
done
```
*Windows PowerShell* (this example starts a Python optimizer; change the `python path\to\your\kopt` part to match yours)
```
New-Item -ItemType Directory -Force out | Out-Null
Get-ChildItem bench\*.kir | ForEach-Object {
  $n = $_.BaseName
  python path\to\your\kopt $_.FullName --machine machines\machine_default.cfg -o out\$n.kir
  .\kvalidate.cmd out\$n.kir --against $_.FullName --machine machines\machine_default.cfg
}
```
Then compare the cycles of `bench/NAME.kir` and `out/NAME.kir`. Do it for the other machine files too. **Always pass `--machine` to `kvalidate`**, so that it also catches a `mac` on a machine that has none.

---

## 6. Command reference

### `kvalidate`

```
kvalidate FILE.kir [FILE2.kir ...] [--machine MACHINE.cfg]
kvalidate OPT.kir --against ORIG.kir [--input FILE.in] [--random N] [--seed S] [--max-steps S] [--machine MACHINE.cfg]
```

| Form | What it does |
|---|---|
| `kvalidate FILE.kir ...` | Checks that each file is written correctly (header, labels, operands, every block ends in a jump, branch, return or trap, every jump goes to a block that exists, and so on). A good file gets a stamp file `FILE.kir.ok`. A bad file gets an error message such as `ERROR file: line 3: ...` and no stamp. |
| `kvalidate OPT.kir --against ORIG.kir` | Does the check above, then runs `ORIG.kir` and `OPT.kir` on many inputs and compares the result, the volatile log and the data memory. On a difference it prints the first difference and the input that shows it (in `.in` format, so you can save it and run it with `ksim --input`). |

| Option | Meaning |
|---|---|
| `--machine MACHINE.cfg` | Also checks that the fused instructions `mac` and `addsh` exist on that machine. |
| `--input FILE.in` | Test with this input too. Can be used several times. (The `ORIG.in` that sits next to the original is always used.) |
| `--random N` | How many made-up inputs to use. Default 100. |
| `--seed S` | The seed for the made-up inputs. The same seed always gives the same inputs. Default 12345. |
| `--max-steps S` | How long the original may run on one input before that input is skipped. Default 200000 instructions. |

**The made-up inputs** are: first 20 inputs where every parameter has the same awkward value (0, 1, 2, 3, 7, 8, 15, 16, 31, 32, 63, 64, 255, 256, 1000, 65535, 2147483647, 2147483648, 4294967294, 4294967295); then 16 pairs of awkward values that must be tried together (such as -2147483648 and -1); then random values. Memory words 0 to 255 are filled with random numbers, and each volatile port 0 to 3 gets 16 random values.

Passing `--against` on your computer **does not prove** that your program is correct. The judges test many more inputs, including ones built to make the original trap. Write your own tests too.

Exit codes: `0` = all checks passed, `1` = a check failed, `2` = wrong command line or a file that cannot be read.

### `ksim`

```
ksim PROG.kir [--machine MACHINE.cfg] [--input FILE.in] [--max-steps N] [--verbose] [--trace [N]]
```

| Option | Meaning |
|---|---|
| `--machine MACHINE.cfg` | The machine to use. Default: `machines/machine_default.cfg`. |
| `--input FILE.in` | The input to use. Default: the file with the same name and the ending `.in` next to the program, if it exists. If there is none, all parameters are 0 and the memory is empty. |
| `--max-steps N` | Stop after N instructions (default 5,000,000). The status is then `TIMEOUT`. |
| `--verbose` | Also print every volatile log entry and every memory word that is not 0. |
| `--trace [N]` | Print the first N instructions that ran (default 50), with their start cycles. |

`ksim` only runs a file that `kvalidate` has approved (section 4, step 4).

Exit codes: `0` = the program finished (returned or trapped), `2` = an error, `3` = `TIMEOUT`.

---

## 7. The file types

| Ending | What it is | Where it is explained |
|---|---|---|
| `.kir` | A program | `KIR_SPEC.md`, sections 2 to 5 |
| `.in` | An input for a program: `args`, `mem` and `vin` lines | `KIR_SPEC.md`, section 8 |
| `.prof` | A branch profile: `edge FROM TO COUNT` lines. It may not match the input that is measured. | `KIR_SPEC.md`, section 8 |
| `.cfg` | A machine: `key = value` lines | `KIR_SPEC.md`, section 9 |
| `.ok` | A stamp made by `kvalidate`. Not part of your work. | this file |

To learn from the programs, open `bench/INDEX.md` (what each practice program is good for) and `examples/EXAMPLES.md` (15 solved examples).

---

## 8. Something went wrong?

| What you see | What to do |
|---|---|
| `python3: command not found` (Linux, macOS) | Install Python 3 (section 2). |
| `'python' is not recognized` or `'python3' is not recognized` (Windows) | Use `ksim.cmd` and `kvalidate.cmd`. If that also fails, Python is not installed or not in the PATH: install it again and tick "Add python.exe to PATH". |
| `ksim.cmd is not recognized` (PowerShell) | Write `.\ksim.cmd`, with the `.\` in front. |
| `Permission denied` when you type `./ksim` (Linux, macOS) | Type `chmod +x ksim kvalidate`, then try again. Or run it as `python3 ksim ...`. |
| `ERROR: ... has not been validated. Run: kvalidate ...` | Run `kvalidate` on that file first. |
| `ERROR: ... changed since it was validated` | You edited the file. Run `kvalidate` on it again. |
| `ERROR ...: line N: ...` from `kvalidate` | Your `.kir` file has a mistake in line N. Compare with `KIR_SPEC.md`, sections 2 and 5. |
| `'mac' is not available on this machine` | You used `mac` (or `addsh`) on a machine without it. Write the longer form instead (`mul` + `add`, or `shl` + `add`). |
| `MISMATCH ...` | Your new program does not behave like the original. Save the input that is printed, run both programs with `ksim --input`, and compare the lines from `status` to `mem_hash`. |
| `ModuleNotFoundError: kirlib` | Run the tools from the kit folder; `kirlib.py` must be next to `ksim`. |
| `status: TIMEOUT` | The program ran more than `--max-steps` instructions. Use a bigger `--max-steps`, or check for an endless loop. |
| Something else | Ask the organizers. Copy the whole message. |
