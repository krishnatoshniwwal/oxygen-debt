#!/usr/bin/env python3
"""Fuzz the loop rerolling pass: random programs made of repeated copies (with affine / masked /
periodic literal differences, loop-carried values, loads, stores, volatile ops and trapping ops),
optimized with a tiny instruction cache so that rolling is attractive, then compared with the
original on many inputs (ret, volatile log, memory, traps).

Usage: python3 tests/fuzz_roll.py KOPT_BINARY [--n 200] [--seed 1]
"""
import os, random, subprocess, sys, tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.realpath(__file__)))
sys.path.insert(0, os.path.join(ROOT, 'kit'))
sys.dont_write_bytecode = True
import kirlib as K  # noqa

OPS2 = ['add', 'sub', 'xor', 'and', 'or', 'mul', 'shl', 'shr', 'lt', 'eq']


def gen(rnd):
    nparams = rnd.randrange(1, 4)
    params = ['v%d' % i for i in range(nparams)]
    lines = []
    nv = [nparams]

    def new():
        nv[0] += 1
        return 'v%d' % (nv[0] - 1)

    carried = []
    for _ in range(rnd.randrange(1, 3)):
        c = new()
        lines.append('  const %s, %d' % (c, rnd.randrange(0, 100)))
        carried.append(c)
    ncopies = rnd.randrange(6, 60)
    bodylen = rnd.randrange(2, 9)
    # per-slot literal behaviours, chosen once per body position
    body = []
    for k in range(bodylen):
        kind = rnd.choice(['alu', 'alu', 'load', 'store', 'vst', 'vld', 'div', 'select'])
        beh = rnd.choice(['const', 'affine', 'mask', 'period', 'affine2'])
        body.append((kind, beh, rnd.randrange(1, 40), rnd.choice(OPS2), rnd.randrange(1, 5), rnd.randrange(2, 9)))
    for j in range(ncopies):
        local = []
        for k, (kind, beh, base, op, dl, per) in enumerate(body):
            if beh == 'const':
                lv = base
            elif beh == 'affine':
                lv = base + j * dl
            elif beh == 'affine2':
                lv = base * 3 + j * (dl + 1)
            elif beh == 'mask':
                lv = (base + j * dl) & 31
            else:
                lv = (base + (j % per) * dl)
            srcs = local + carried + params
            a = rnd.choice(srcs) if False else srcs[(k * 7 + 3) % len(srcs)]
            b = srcs[(k * 5 + 1) % len(srcs)]
            d = new()
            if kind == 'alu':
                lines.append('  %s %s, %s, %d' % (op, d, a, lv) if k % 2 == 0 else '  %s %s, %s, %s' % (op, d, a, b))
                local.append(d)
            elif kind == 'load':
                lines.append('  load %s, %d' % (d, lv % 200))
                local.append(d)
            elif kind == 'store':
                lines.append('  store %d, %s' % (300 + lv % 200, a))
            elif kind == 'vst':
                lines.append('  vstore %d, %s' % (k % 3, a))
            elif kind == 'vld':
                lines.append('  vload %s, %d' % (d, k % 3))
                local.append(d)
            elif kind == 'div':
                lines.append('  div %s, %s, %d' % (d, a, 1 + lv % 7))
                local.append(d)
            else:
                lines.append('  select %s, %s, %s, %d' % (d, b, a, lv))
                local.append(d)
        # loop-carried: rotate one value into the carried set
        if local and carried:
            idx = j % len(carried)
            nc = new()
            lines.append('  add %s, %s, %s' % (nc, carried[idx], local[-1]))
            carried[idx] = nc
    res = carried[0]
    for c in carried[1:]:
        t = new()
        lines.append('  xor %s, %s, %s' % (t, res, c))
        res = t
    lines.append('  ret %s' % res)
    return 'func fz(%s)\nentry:\n%s\n' % (', '.join(params), '\n'.join(lines)), nparams


def machine(c):
    base = open(os.path.join(ROOT, 'kit/machines/machine_default.cfg')).read().splitlines()
    out = []
    for ln in base:
        if ln.startswith('icache_size'):
            ln = 'icache_size = %d' % c
        out.append(ln)
    return '\n'.join(out) + '\n'


def main():
    kopt = sys.argv[1]
    n = 200
    seed = 1
    for i, a in enumerate(sys.argv):
        if a == '--n':
            n = int(sys.argv[i + 1])
        if a == '--seed':
            seed = int(sys.argv[i + 1])
    rnd = random.Random(seed)
    tmp = tempfile.mkdtemp()
    bad = 0
    rolled = 0
    for it in range(n):
        text, np_ = gen(rnd)
        c = rnd.choice([8, 12, 16, 24, 32])
        src = os.path.join(tmp, 'p.kir')
        out = os.path.join(tmp, 'o.kir')
        mf = os.path.join(tmp, 'm.cfg')
        open(src, 'w').write(text)
        open(mf, 'w').write(machine(c))
        r = subprocess.run([kopt, src, '--machine', mf, '-o', out], capture_output=True, text=True)
        if r.returncode != 0:
            print('EXIT', r.returncode, it)
            bad += 1
            continue
        mc = K.load_machine(mf)
        orig = K.parse_program(K.read_text(src))
        new = K.parse_program(K.read_text(out))
        K.check_machine_features(new, mc)
        if len(new.instrs) < len(orig.instrs) // 2:
            rolled += 1
        cpo, cpn = K.compile_program(orig), K.compile_program(new)
        for ii in K.generate_inputs(np_, 40, 1000 + it):
            ro = K.execute(cpo, ii.args, ii.segments, ii.vin, 300000)
            if ro.status == 'TIMEOUT':
                continue
            rn = K.execute(cpn, ii.args, ii.segments, ii.vin, 20 * ro.steps + 1000)
            d = K.first_difference(ro, rn)
            if d:
                print('MISMATCH iteration', it, 'C', c, d)
                open('/tmp/fuzz_roll_bad_%d.kir' % it, 'w').write(text)
                bad += 1
                break
    print('done: %d programs, %d mismatches, %d shrank to less than half' % (n, bad, rolled))
    sys.exit(1 if bad else 0)


if __name__ == '__main__':
    main()
