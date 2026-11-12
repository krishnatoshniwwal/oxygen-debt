#!/usr/bin/env python3
"""Edge-input check of loop closed forms: hand-written unmasked loops, kopt output vs input.
Usage: python3 tests/alg_edge.py KOPT_BINARY"""
import os, subprocess, sys, tempfile
ROOT = os.path.dirname(os.path.dirname(os.path.realpath(__file__)))
sys.path.insert(0, os.path.join(ROOT, 'kit')); sys.dont_write_bytecode = True
import kirlib as K
PROGS = {
'up3': """func f(v0, v1)
entry:
  const v2, 0
  const v3, 0
  jmp h
h:
  lt v4, v3, v0
  br v4, b, x
b:
  mul v5, v3, v3
  mul v6, v5, v3
  mul v7, v3, v1
  add v8, v2, v6
  add v2, v8, v7
  add v3, v3, 1
  jmp h
x:
  ret v2
""",
'down': """func f(v0, v1)
entry:
  mov v3, v0
  const v2, 7
  jmp h
h:
  lt v4, v1, v3
  br v4, b, x
b:
  mul v5, v3, v3
  add v2, v2, v5
  sub v3, v3, 1
  jmp h
x:
  ret v2
""",
'eqloop': """func f(v0, v1)
entry:
  mov v3, v0
  const v2, 0
  jmp h
h:
  eq v4, v3, v1
  br v4, x, b
b:
  add v2, v2, v3
  add v3, v3, 1
  jmp h
x:
  ret v2
""",
'deg4': """func f(v0, v1)
entry:
  const v2, 0
  const v3, 0
  const v6, 0
  jmp h
h:
  lt v4, v3, v0
  br v4, b, x
b:
  add v2, v2, v3
  add v6, v6, v2
  add v7, v6, v1
  add v3, v3, 1
  jmp h
x:
  add v8, v6, v2
  ret v8
""",
}
vals = list(range(0, 70)) + [0x7FFFFFFF, 0x80000000, 0xFFFFFFFF, 0xFFFFFFFE, 1000]
mc = os.path.join(ROOT, 'kit/machines/machine_default.cfg')
bad = 0
for name, text in PROGS.items():
    d = tempfile.mkdtemp()
    a, b = os.path.join(d, 'a.kir'), os.path.join(d, 'b.kir')
    open(a, 'w').write(text)
    subprocess.run([sys.argv[1], a, '--machine', mc, '-o', b], check=True)
    po, pn = K.parse_program(text), K.parse_program(open(b).read())
    co, cn = K.compile_program(po), K.compile_program(pn)
    n = 0
    for x in vals:
        for y in [0, 1, 5, 0xFFFFFFFF, 0x80000000, 12345]:
            if name in ('up3', 'deg4', 'down') and x >= 70 and x not in (1000,) and name != 'down':
                if x > 100000:  # loops that long time out in the original
                    continue
            if name == 'eqloop' and (abs(x - y) > 5000 and (x - y) & 0xFFFFFFFF > 5000): continue
            inp = K.Inputs([x, y], [], {})
            ro = K.execute(co, inp.args, inp.segments, inp.vin, 300000)
            if ro.status == 'TIMEOUT': continue
            rn = K.execute(cn, inp.args, inp.segments, inp.vin, 20 * ro.steps + 1000)
            n += 1
            d_ = K.first_difference(ro, rn)
            if d_: bad += 1; print('DIFF', name, x, y, d_)
    print(name, 'checked', n, 'cycles-lines', len(open(b).read().splitlines()))
print('bad', bad)
