#!/usr/bin/env python3
"""divisibility test x % c == 0 check: python3 tests/alg_divtest.py KOPT"""
import os, subprocess, sys, tempfile, random
ROOT = os.path.dirname(os.path.dirname(os.path.realpath(__file__)))
sys.path.insert(0, os.path.join(ROOT, 'kit')); sys.dont_write_bytecode = True
import kirlib as K
bad = 0
for c in (3, 5, 7, 9, 15, 641, 65537, 0x7FFFFFFF):
    T = """func f(v0)
entry:
  mod v1, v0, %d
  eq v2, v1, 0
  const v3, 1
  select v4, v2, v3, 0
  ret v4
""" % c
    d = tempfile.mkdtemp(); a, b = d + '/a.kir', d + '/b.kir'
    open(a, 'w').write(T)
    subprocess.run([sys.argv[1], a, '--machine', ROOT + '/kit/machines/machine_default.cfg', '-o', b], check=True)
    out = open(b).read()
    co, cn = K.compile_program(K.parse_program(T)), K.compile_program(K.parse_program(out))
    xs = list(range(-300, 300)) + [0xFFFFFFFF, 0x80000000, 0x7FFFFFFF, 0x80000001] + [random.getrandbits(32) for _ in range(300)]
    xs += [(k * c) & 0xFFFFFFFF for k in (random.randint(-10**6, 10**6) for _ in range(300))]
    for x in xs:
        ro = K.execute(co, [x], [], {}, 1000); rn = K.execute(cn, [x], [], {}, 1000)
        if K.first_difference(ro, rn): bad += 1; print('DIFF', c, x)
    print(c, 'mod in output:', ' mod ' in out)
print('bad', bad)
