#!/usr/bin/env python3
"""Fuzz the Rust kopt through its command line.

Uses the random-program generator of the Python reference (pyref/tests/fuzz.py). For each
program: run ./kopt and check that the output behaves like the input on many inputs. With
--ref, also compare the output text with the Python reference's output.
Usage: python3 tests/fuzz_cli.py [--n 300] [--seed 1] [--jobs 10] [--ref]
"""
import argparse, os, random, subprocess, sys
from concurrent.futures import ProcessPoolExecutor

HERE = os.path.dirname(os.path.realpath(__file__))
ROOT = os.path.dirname(HERE)
sys.dont_write_bytecode = True
sys.path.insert(0, os.path.join(ROOT, 'pyref', 'src'))
sys.path.insert(0, os.path.join(ROOT, 'pyref', 'tests'))
import kirlib as K  # noqa: E402
import fuzz  # noqa: E402


def one(args):
    seed, it, inputs, ref = args
    rnd = random.Random(seed * 1000003 + it)
    mc = fuzz.random_machine(rnd)
    g = fuzz.Gen(rnd)
    text = g.build(bool(mc['fused_mac'] and mc['fused_addsh']))
    d = os.path.join(ROOT, 'out', 'fz')
    os.makedirs(d, exist_ok=True)
    base = os.path.join(d, 'f_%d_%d' % (seed, it))
    with open(base + '.kir', 'w') as fh:
        fh.write(text)
    with open(base + '.cfg', 'w') as fh:
        for k, v in mc.items():
            fh.write('%s = %d\n' % (k, v))
    p = subprocess.run([os.path.join(ROOT, 'kopt'), base + '.kir', '--machine', base + '.cfg', '-o', base + '.out'],
                       capture_output=True, text=True)
    res = {'it': it, 'base': base, 'problems': []}
    if p.returncode != 0:
        res['problems'].append('exit %d' % p.returncode)
        return res
    if p.stderr.strip():
        res['problems'].append('stderr: ' + p.stderr.strip().splitlines()[0])
    out = open(base + '.out').read()
    orig = K.parse_program(text)
    try:
        new = K.parse_program(out)
        K.check_machine_features(new, mc)
    except K.KirError as e:
        res['problems'].append('invalid output: %s' % e)
        return res
    cpo, cpn = K.compile_program(orig), K.compile_program(new)
    ins = K.generate_inputs(len(orig.params), inputs, 777 + it)
    for ii in K.generate_inputs(len(orig.params), 6, 31337 + it):
        ii.segments = [(0, [(w * 2654435761) & 0xFFFFFFFF for w in ii.segments[0][1]])]
        ins.append(ii)
    for k, ii in enumerate(ins):
        ro = K.execute(cpo, ii.args, ii.segments, ii.vin, 30000)
        if ro.status == 'TIMEOUT':
            continue
        rn = K.execute(cpn, ii.args, ii.segments, ii.vin, 20 * ro.steps + 1000)
        diff = K.first_difference(ro, rn)
        if diff:
            res['problems'].append('MISMATCH input #%d: %s' % (k, diff))
            break
    if not ref:
        return res
    try:
        q = subprocess.run([sys.executable, os.path.join(ROOT, 'pyref', 'kopt'), base + '.kir', '--machine', base + '.cfg',
                            '-o', base + '.py'], capture_output=True, text=True, timeout=30)
        if q.returncode == 0 and open(base + '.py').read() != out:
            res['problems'].append('text differs from python reference')
    except subprocess.TimeoutExpired:
        res['note'] = 'python reference timed out'
    return res


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--n', type=int, default=300)
    ap.add_argument('--seed', type=int, default=1)
    ap.add_argument('--jobs', type=int, default=10)
    ap.add_argument('--inputs', type=int, default=70)
    ap.add_argument('--ref', action='store_true',
                    help='also compare the output text with the Python reference (they differ since the Rust version moved on)')
    o = ap.parse_args()
    with ProcessPoolExecutor(o.jobs) as ex:
        results = list(ex.map(one, [(o.seed, i, o.inputs, o.ref) for i in range(o.n)], chunksize=2))
    bad = [r for r in results if r['problems']]
    for r in bad[:25]:
        print('iteration %d (%s): %s' % (r['it'], r['base'], '; '.join(r['problems'])))
    kinds = {}
    for r in bad:
        for p in r['problems']:
            k = p.split(':')[0].split(' input')[0]
            kinds[k] = kinds.get(k, 0) + 1
    print('python reference timed out on %d programs' % len([r for r in results if r.get('note')]))
    print('fuzz_cli: %d programs, %d with problems %s' % (o.n, len(bad), kinds))
    return 1 if bad else 0


if __name__ == '__main__':
    sys.exit(main())
