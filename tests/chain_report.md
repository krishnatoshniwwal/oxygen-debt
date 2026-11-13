# large_chain: finding (task chain)

Verdict (re-checked against kit/kirlib.py time_trace): Q-bound. Each instruction at pc >= C (=64) adds F=Q=2 to
`t = last + F`, i.e. the issue of every instruction is >= 2 cycles after the previous one, whatever the dependences.
100,001 instrs -> ~200k cycles (measured 206,527). Dataflow alone (Q=0) is 113,173 (~1.13/instr), so only
removing instructions helps; width, scheduling and latency changes cannot.

Why no linear clean-up pass was written:
* The kit-style "const + op + mov" chain is already handled: a 24,004-line chain of that shape (add/xor/mul/sub
  with const operands and movs) is reduced by kopt to 4,011 lines in 0.14 s (one instruction per step; ring folding
  and copy removal already exist). So the clean-up the task asks for is already there.
* tests/gen_large.py's large_chain is already minimal in that sense: every step is `op n,acc,c ; shr m,n,s ; xor k,n,m`
  (literal operands, no copies, no two adjacent same-ring constant ops; shr/xor-of-self is a non-ring mix).
* The only algebra left: a pending xor-constant floats through the xorshift (k = n ^ (n>>s) maps x^c to
  (x^(x>>s)) ^ (c^(c>>s))), so a step with `xor` immediately followed by another `xor` step saves 1 instruction.
  Count: ~1/25 of 33,333 steps = ~1.3k instructions = ~1.3% (~2.7k cycles) on this one synthetic program, in a
  form (xorshift chains) that the real kit large kind probably does not have. Not worth the risk of a new pass.
No code changed.
