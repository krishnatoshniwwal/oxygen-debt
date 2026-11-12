#!/usr/bin/env python3
"""Cycles and wall time of kopt on generated large programs for several (C, Q) machines.

Usage: python3 tests/measure_large.py KOPT_BINARY PROGDIR [--c 32,64,256] [--q 1,2,8]
                                       [--only SUBSTR] [--base MACHINE.cfg]
For every PROGDIR/*.kir (with its .in) and every (C, Q): run kopt (timed), check the output against
the input on the .in file, 3 perturbed variants and some generated inputs, then print
instructions, cycles before/after, wall seconds.
"""
import glob, os, random, subprocess, sys, tempfile, time, zlib

ROOT = os.path.dirname(os.path.dirname(os.path.realpath(__file__)))
sys.path.insert(0, os.path.join(ROOT, 'kit'))
sys.dont_write_bytecode = True
import kirlib as K  # noqa


def machine_text(base, c, q):
    out = []
    for ln in open(base).read().splitlines():
        if ln.startswith('icache_size'):
            ln = 'icache_size = %d' % c
        elif ln.startswith('icache_penalty'):
            ln = 'icache_penalty = %d' % q
        out.append(ln)
    return '\n'.join(out) + '\n'


def perturb(inp, rnd):
    def p(v):
        s = v - (1 << 32) if v >= (1 << 31) else v
        if abs(s) <= 1:
            return v
        if abs(s) > 100000:
            return rnd.getrandbits(32)
        t = int(round(s * rnd.uniform(0.5, 1.5)))
        return (t if t else s) & 0xFFFFFFFF
    return K.Inputs([p(a) for a in inp.args], [(a, [p(w) for w in ws]) for a, ws in inp.segments],
                    {k: [p(w) for w in ws] for k, ws in inp.vin.items()})


def main():
    a = sys.argv
    kopt, d = a[1], a[2]
    cs = [32, 64, 256]
    qs = [1, 2, 8]
    only = None
    base = os.path.join(ROOT, 'kit/machines/machine_default.cfg')
    i = 3
    while i < len(a):
        if a[i] == '--c':
            cs = [int(x) for x in a[i + 1].split(',')]
        elif a[i] == '--q':
            qs = [int(x) for x in a[i + 1].split(',')]
        elif a[i] == '--only':
            only = a[i + 1]
        elif a[i] == '--base':
            base = a[i + 1]
        i += 2
    tmp = tempfile.mkdtemp()
    print('%-14s %3s %2s %7s %7s %10s %10s %7s %s' % ('prog', 'C', 'Q', 'n_in', 'n_out', 'cyc_in', 'cyc_out', 'secs', 'status'))
    for p in sorted(glob.glob(os.path.join(d, '*.kir'))):
        name = os.path.basename(p)[:-4]
        if not os.path.exists(p[:-4] + '.in') or (only and only not in name):
            continue
        orig = K.parse_program(K.read_text(p))
        inp = K.load_input(p[:-4] + '.in', len(orig.params))
        rnd = random.Random(zlib.crc32(name.encode()))
        vs = [perturb(inp, rnd) for _ in range(2)]
        cpo = K.compile_program(orig)
        for c in cs:
            for q in qs:
                mf = os.path.join(tmp, 'm_%d_%d.cfg' % (c, q))
                open(mf, 'w').write(machine_text(base, c, q))
                mc = K.load_machine(mf)
                out = os.path.join(tmp, 'out.kir')
                t0 = time.time()
                r = subprocess.run([kopt, p, '--machine', mf, '-o', out], capture_output=True, text=True)
                dt = time.time() - t0
                if r.returncode != 0:
                    print('%-14s %3d %2d EXIT %d' % (name, c, q, r.returncode))
                    continue
                try:
                    new = K.parse_program(K.read_text(out))
                    K.check_machine_features(new, mc)
                except K.KirError as e:
                    print('%-14s %3d %2d INVALID %s' % (name, c, q, e))
                    continue
                cpn = K.compile_program(new)
                status = 'ok'
                for ii in [inp] + vs + K.generate_inputs(len(orig.params), 6, 99):
                    ro = K.execute(cpo, ii.args, ii.segments, ii.vin, 2000000)
                    if ro.status == 'TIMEOUT':
                        continue
                    rn = K.execute(cpn, ii.args, ii.segments, ii.vin, 20 * ro.steps + 1000)
                    df = K.first_difference(ro, rn)
                    if df:
                        status = 'MISMATCH ' + str(df)
                        break
                ro = K.execute(cpo, inp.args, inp.segments, inp.vin, 4000000, want_trace=True)
                rn = K.execute(cpn, inp.args, inp.segments, inp.vin, 40000000, want_trace=True)
                print('%-14s %3d %2d %7d %7d %10d %10d %7.2f %s' % (
                    name, c, q, len(orig.instrs), len(new.instrs),
                    K.time_trace(cpo, ro.trace, mc), K.time_trace(cpn, rn.trace, mc), dt, status), flush=True)


if __name__ == '__main__':
    main()
