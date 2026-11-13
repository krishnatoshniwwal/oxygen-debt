#!/usr/bin/env python3
"""iv div/mod check: python3 tests/alg_div.py KOPT"""
import os, subprocess, sys, tempfile
ROOT = os.path.dirname(os.path.dirname(os.path.realpath(__file__)))
sys.path.insert(0, os.path.join(ROOT, 'kit')); sys.dont_write_bytecode = True
import kirlib as K
T = """func f(v0)
entry:
  and v0, v0, 255
  const v2, 0
  const v3, 0
  jmp h
h:
  lt v4, v3, v0
  br v4, b, x
b:
  div v5, v3, 7
  mod v6, v3, 7
  mul v7, v5, v6
  xor v8, v2, v7
  add v2, v8, v3
  add v3, v3, 1
  jmp h
x:
  ret v2
"""
d = tempfile.mkdtemp(); a, b = d + '/a.kir', d + '/b.kir'
open(a, 'w').write(T)
subprocess.run([sys.argv[1], a, '--machine', ROOT + '/kit/machines/machine_default.cfg', '-o', b], check=True)
co, cn = K.compile_program(K.parse_program(T)), K.compile_program(K.parse_program(open(b).read()))
bad = 0
for x in list(range(0, 300)) + [0xFFFFFFFF, 0x80000000]:
    ro = K.execute(co, [x], [], {}, 300000); rn = K.execute(cn, [x], [], {}, 300000)
    if K.first_difference(ro, rn): bad += 1; print('DIFF', x)
print('bad', bad, 'divs in output:', open(b).read().count('div') + open(b).read().count('mod'))
