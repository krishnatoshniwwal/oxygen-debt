#!/usr/bin/env python3
"""Test kopt the way the judges run it: through the command line, checked by the kit's
own kvalidate, run twice to make sure the output file is identical.

Usage: python tests/run_cli.py [--kit DIR] [--machine NAME ...] [--only SUBSTR] [--out DIR]
"""
import argparse
import glob
import os
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.realpath(__file__))
ROOT = os.path.dirname(HERE)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--kit', default=os.path.join(ROOT, 'kit'))
    ap.add_argument('--machine', action='append', default=[])
    ap.add_argument('--only', default=None)
    ap.add_argument('--out', default=os.path.join(ROOT, 'out', 'cli'))
    o = ap.parse_args()
    py = sys.executable
    kopt = os.path.join(ROOT, 'kopt')
    kval = os.path.join(o.kit, 'kvalidate')
    machines = sorted(glob.glob(os.path.join(o.kit, 'machines', '*.cfg')))
    if o.machine:
        machines = [m for m in machines if any(s in os.path.basename(m) for s in o.machine)]
    progs = sorted(glob.glob(os.path.join(o.kit, 'bench', '*.kir')))
    if o.only:
        progs = [p for p in progs if o.only in os.path.basename(p)]
    bad = 0
    tmax = 0.0
    for m in machines:
        mname = os.path.basename(m)[:-4]
        d = os.path.join(o.out, mname)
        os.makedirs(d, exist_ok=True)
        for p in progs:
            name = os.path.basename(p)[:-4]
            out1 = os.path.join(d, name + '.kir')
            out2 = os.path.join(d, name + '.second.kir')
            cmd = [kopt, p, '--machine', m]
            prof = p[:-4] + '.prof'
            if os.path.exists(prof):
                cmd += ['--profile', prof]
            t0 = time.time()
            r1 = subprocess.run(cmd + ['-o', out1], capture_output=True, text=True)
            dt = time.time() - t0
            tmax = max(tmax, dt)
            r2 = subprocess.run(cmd + ['-o', out2], capture_output=True, text=True)
            msg = None
            if r1.returncode != 0 or r2.returncode != 0:
                msg = 'kopt exit status %d' % r1.returncode
            elif r1.stderr.strip():
                msg = 'kopt wrote to stderr: %s' % r1.stderr.strip().splitlines()[0]
            else:
                with open(out1, 'rb') as fh:
                    a = fh.read()
                with open(out2, 'rb') as fh:
                    b = fh.read()
                if a != b:
                    msg = 'two runs gave different files'
                else:
                    v = subprocess.run([py, kval, out1, '--against', p, '--machine', m],
                                       capture_output=True, text=True)
                    if v.returncode != 0:
                        msg = 'kvalidate: %s' % (v.stdout.strip().splitlines() or ['?'])[0]
            os.remove(out2)
            if msg:
                bad += 1
                print('FAIL %-24s %-22s %s' % (name, mname, msg))
        print('%s: done' % mname)
    print('cli test: %d failures, slowest kopt run %.2fs' % (bad, tmax))
    return 1 if bad else 0


if __name__ == '__main__':
    sys.exit(main())
