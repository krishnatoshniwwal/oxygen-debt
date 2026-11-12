#!/usr/bin/env python3
"""Compare the Rust kopt with the Python reference (pyref/kopt) on the practice programs.

For every program x machine: run both, validate the Rust output with the kit's kvalidate,
and compare the cycle counts on the sample input.  Usage:
  python3 tests/compare.py [--kit DIR] [--machine SUBSTR] [--only SUBSTR] [--jobs N] [--nopy]
"""
import argparse, glob, os, subprocess, sys, time
from concurrent.futures import ProcessPoolExecutor

HERE = os.path.dirname(os.path.realpath(__file__))
ROOT = os.path.dirname(HERE)
sys.dont_write_bytecode = True


def cycles(K, kir, inp, mc):
    prog = K.load_program(kir)
    cp = K.compile_program(prog)
    i = K.load_input(inp, len(prog.params))
    res = K.execute(cp, i.args, i.segments, i.vin, 5000000, want_trace=True)
    if res.status == 'TIMEOUT':
        return None
    return K.time_trace(cp, res.trace, mc)


def task(args):
    kit, prog, mach, nopy = args
    sys.path.insert(0, kit)
    import kirlib as K
    name = os.path.basename(prog)[:-4]
    mname = os.path.basename(mach)[:-4]
    d = os.path.join(ROOT, 'out', 'cmp', mname)
    os.makedirs(d, exist_ok=True)
    prof = prog[:-4] + '.prof'
    extra = ['--profile', prof] if os.path.exists(prof) else []
    r = {'name': name, 'machine': mname}
    rust = os.path.join(d, name + '.rust.kir')
    t0 = time.time()
    p = subprocess.run([os.path.join(ROOT, 'kopt'), prog, '--machine', mach] + extra + ['-o', rust], capture_output=True, text=True)
    r['rust_time'] = time.time() - t0
    r['rust_err'] = p.stderr.strip()
    if p.returncode != 0:
        r['fail'] = 'rust exit %d' % p.returncode
        return r
    v = subprocess.run([sys.executable, os.path.join(kit, 'kvalidate'), rust, '--against', prog, '--machine', mach], capture_output=True, text=True)
    if v.returncode != 0:
        r['fail'] = 'kvalidate: ' + (v.stdout.strip().splitlines() or ['?'])[0]
        return r
    mc = K.load_machine(mach)
    inp = prog[:-4] + '.in'
    r['orig'] = cycles(K, prog, inp, mc)
    r['rust'] = cycles(K, rust, inp, mc)
    if not nopy:
        py = os.path.join(d, name + '.py.kir')
        if not os.path.exists(py):
            subprocess.run([sys.executable, os.path.join(ROOT, 'pyref', 'kopt'), prog, '--machine', mach] + extra + ['-o', py], capture_output=True, text=True)
        r['py'] = cycles(K, py, inp, mc)
    return r


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--kit', default=os.path.join(ROOT, 'kit'))
    ap.add_argument('--machine', default=None)
    ap.add_argument('--only', default=None)
    ap.add_argument('--jobs', type=int, default=10)
    ap.add_argument('--nopy', action='store_true')
    ap.add_argument('-v', action='store_true')
    o = ap.parse_args()
    machines = sorted(glob.glob(os.path.join(o.kit, 'machines', '*.cfg')))
    if o.machine:
        machines = [m for m in machines if o.machine in os.path.basename(m)]
    progs = sorted(glob.glob(os.path.join(o.kit, 'bench', '*.kir')))
    if o.only:
        progs = [p for p in progs if o.only in os.path.basename(p)]
    tasks = [(o.kit, p, m, o.nopy) for m in machines for p in progs]
    with ProcessPoolExecutor(o.jobs) as ex:
        results = list(ex.map(task, tasks, chunksize=1))
    fails = [r for r in results if 'fail' in r]
    for r in fails:
        print('FAIL %-22s %-22s %s' % (r['name'], r['machine'], r['fail']))
    noisy = [r for r in results if r.get('rust_err')]
    for r in noisy[:10]:
        print('STDERR %-20s %-20s %s' % (r['name'], r['machine'], r['rust_err'].splitlines()[0]))
    ok = [r for r in results if 'fail' not in r and r.get('orig') and r.get('rust')]
    import math
    gm = lambda xs: math.exp(sum(math.log(x) for x in xs) / len(xs)) if xs else float('nan')
    print('programs x machines: %d, failures: %d, stderr output: %d' % (len(results), len(fails), len(noisy)))
    print('slowest rust run: %.2fs' % max(r['rust_time'] for r in results))
    print('geomean rust/orig cycles: %.4f' % gm([r['rust'] / r['orig'] for r in ok]))
    if not o.nopy:
        both = [r for r in ok if r.get('py')]
        print('geomean py/orig cycles:   %.4f' % gm([r['py'] / r['orig'] for r in both]))
        worse = [r for r in both if r['rust'] > r['py']]
        better = [r for r in both if r['rust'] < r['py']]
        print('rust worse than py: %d, better: %d, equal: %d' % (len(worse), len(better), len(both) - len(worse) - len(better)))
        for r in sorted(worse, key=lambda r: -r['rust'] / r['py'])[:15]:
            print('  worse %-22s %-22s rust %d py %d orig %d' % (r['name'], r['machine'], r['rust'], r['py'], r['orig']))
    return 1 if fails else 0


if __name__ == '__main__':
    sys.exit(main())
