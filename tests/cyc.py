#!/usr/bin/env python3
"""cyc.py PROG.kir MACHINE.cfg INPUT.in -> cycles of PROG on INPUT (kit timing model)."""
import os, sys
sys.dont_write_bytecode = True
sys.path.insert(0, os.path.join(os.path.dirname(os.path.dirname(os.path.realpath(__file__))), 'kit'))
import kirlib as K
def cycles(prog, mach, inp):
    mc = K.load_machine(mach); p = K.parse_program(K.read_text(prog)); i = K.load_input(inp, len(p.params))
    cp = K.compile_program(p); r = K.execute(cp, i.args, i.segments, i.vin, 5000000, want_trace=True)
    return K.time_trace(cp, r.trace, mc)
if __name__ == '__main__':
    print(cycles(*sys.argv[1:4]))
