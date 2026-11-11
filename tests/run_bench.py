#!/usr/bin/env python3
"""Run kopt (in-process) on every practice program and check and measure the result.

Usage: python tests/run_bench.py [--kit DIR] [--machine NAME ...] [--only SUBSTR] [--random N]
                                 [--verbose] [--debug] [--save DIR]

For every program and machine it checks the output with the kit's own code (same checks
as `kvalidate --against`) and compares the cycle counts on the sample input.
"""
import argparse
import glob
import math
import os
import sys
import time

HERE = os.path.dirname(os.path.realpath(__file__))
ROOT = os.path.dirname(HERE)
sys.dont_write_bytecode = True
sys.path.insert(0, os.path.join(ROOT, 'src'))
import kirlib as K  # noqa: E402
import driver  # noqa: E402


def cycles_of(prog, mc, inp, max_steps=5000000):
    cp = K.compile_program(prog)
    res = K.execute(cp, inp.args, inp.segments, inp.vin, max_steps, want_trace=True)
    if res.status == 'TIMEOUT':
        return None, res
    return K.time_trace(cp, res.trace, mc), res


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--kit', default=os.path.join(ROOT, 'kit'))
    ap.add_argument('--machine', action='append', default=[])
    ap.add_argument('--only', default=None)
    ap.add_argument('--random', type=int, default=100)
    ap.add_argument('--seed', type=int, default=12345)
    ap.add_argument('--verbose', action='store_true')
    ap.add_argument('--debug', action='store_true')
    ap.add_argument('--check', action='store_true')
    ap.add_argument('--save', default=None)
    o = ap.parse_args()
    machines = sorted(glob.glob(os.path.join(o.kit, 'machines', '*.cfg')))
    if o.machine:
        machines = [m for m in machines if any(s in os.path.basename(m) for s in o.machine)]
    progs = sorted(glob.glob(os.path.join(o.kit, 'bench', '*.kir')))
    if o.only:
        progs = [p for p in progs if o.only in os.path.basename(p)]
    bad = 0
    for mpath in machines:
        mc = K.load_machine(mpath)
        mname = os.path.basename(mpath)[:-4]
        logsum = 0.0
        n = 0
        tmax = 0.0
        tot0 = tot1 = 0
        for p in progs:
            name = os.path.basename(p)[:-4]
            text = K.read_text(p)
            orig = K.parse_program(text)
            prof = p[:-4] + '.prof'
            ptext = K.read_text(prof) if os.path.exists(prof) else None
            t0 = time.time()
            out = driver.optimize(text, mc, ptext, driver.Options(debug=o.debug, check=o.check))
            dt = time.time() - t0
            tmax = max(tmax, dt)
            out2 = driver.optimize(text, mc, ptext, driver.Options(debug=o.debug, check=o.check))
            status = 'ok'
            if out2 != out:
                status = 'NONDETERMINISTIC'
            try:
                new = K.parse_program(out)
                K.check_machine_features(new, mc)
            except K.KirError as e:
                print('%-26s %-22s INVALID OUTPUT: %s' % (name, mname, e))
                bad += 1
                continue
            if o.save:
                os.makedirs(os.path.join(o.save, mname), exist_ok=True)
                with open(os.path.join(o.save, mname, name + '.kir'), 'w', newline='\n') as fh:
                    fh.write(out)
            inp = K.load_input(p[:-4] + '.in', len(orig.params))
            inputs = [inp] + K.generate_inputs(len(orig.params), o.random, o.seed)
            cpo, cpn = K.compile_program(orig), K.compile_program(new)
            for k, ii in enumerate(inputs):
                ro = K.execute(cpo, ii.args, ii.segments, ii.vin, 200000)
                if ro.status == 'TIMEOUT':
                    continue
                rn = K.execute(cpn, ii.args, ii.segments, ii.vin, 20 * ro.steps + 1000)
                diff = K.first_difference(ro, rn)
                if diff:
                    status = 'MISMATCH input #%d: %s' % (k, diff)
                    if o.verbose:
                        print(K.format_input(ii))
                    break
            c0, _ = cycles_of(orig, mc, inp)
            c1, _ = cycles_of(new, mc, inp)
            if status != 'ok':
                bad += 1
            if c0 and c1:
                logsum += math.log(float(c1) / c0)
                n += 1
                tot0 += c0
                tot1 += c1
            if o.verbose or status != 'ok':
                print('%-26s %-22s %7s -> %7s  %5.2fx  %4d ins  %.2fs  %s' % (
                    name, mname, c0, c1, (float(c0) / c1 if c0 and c1 else 0),
                    len(new.instrs), dt, status))
        if n:
            print('== %-22s programs %d  geomean speedup %.3fx  total cycles %d -> %d  max kopt time %.2fs' % (
                mname, n, math.exp(-logsum / n), tot0, tot1, tmax))
    print('FAILURES: %d' % bad)
    return 1 if bad else 0


if __name__ == '__main__':
    sys.exit(main())
