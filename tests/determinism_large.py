#!/usr/bin/env python3
"""Determinism of kopt on large programs under CPU throttling.

Usage: python3 tests/determinism_large.py KOPT PROGDIR [--only SUBSTR] [--machine CFG] [--loads 1,2]
For every PROGDIR/*.kir: run kopt twice unloaded (reference), then once per load level N with the
process pinned (taskset -c 0) to a core shared with N busy loops (`sh -c 'while :; do :; done'`), twice
per level. Output bytes are compared with the unloaded run. Prints identical / DIFFERENT per run, the
wall times and slowdown. Exit code 1 if anything differs, crashes or exceeds 30 s.
"""
import glob, os, subprocess, sys, tempfile, time

CORE = '0'
LIMIT = 30.0


def run(kopt, prog, mach, out, pin):
    cmd = [kopt, prog, '-o', out]
    if mach:
        cmd[2:2] = ['--machine', mach]
    if pin:
        cmd = ['taskset', '-c', CORE] + cmd
    t0 = time.time()
    try:
        r = subprocess.run(cmd, capture_output=True, text=True, timeout=120)
        rc = r.returncode
    except subprocess.TimeoutExpired:
        rc = -999
    dt = time.time() - t0
    data = open(out, 'rb').read() if rc == 0 and os.path.exists(out) else None
    return rc, dt, data


def main():
    a = sys.argv
    kopt, d = os.path.abspath(a[1]), a[2]
    only, loads = None, [1, 2]
    mach = os.path.join(os.path.dirname(os.path.dirname(os.path.realpath(__file__))), 'kit/machines/machine_default.cfg')
    i = 3
    while i < len(a):
        if a[i] == '--only':
            only = a[i + 1]
        elif a[i] == '--machine':
            mach = a[i + 1]
        elif a[i] == '--loads':
            loads = [int(x) for x in a[i + 1].split(',')]
        i += 2
    tmp = tempfile.mkdtemp()
    out = os.path.join(tmp, 'o.kir')
    bad = 0
    print('%-18s %-22s %7s %8s  %s' % ('prog', 'run', 'secs', 'slowdown', 'result'))
    for p in sorted(glob.glob(os.path.join(d, '*.kir'))):
        name = os.path.basename(p)[:-4]
        if only and only not in name:
            continue
        rc, t0, ref = run(kopt, p, mach, out, False)
        rc2, t1, ref2 = run(kopt, p, mach, out, False)
        if rc or rc2:
            print('%-18s %-22s EXIT %s/%s' % (name, 'unloaded', rc, rc2)); bad += 1; continue
        base = min(t0, t1)
        print('%-18s %-22s %7.2f %8s  %s' % (name, 'unloaded x2', t1, '1.0x',
              'identical' if ref == ref2 else 'DIFFERENT'))
        bad += ref != ref2
        if base > LIMIT:
            print('  TIMEOUT-RISK: unloaded %.1f s > %.0f s' % (base, LIMIT)); bad += 1
        for n in loads:
            hogs = [subprocess.Popen(['taskset', '-c', CORE, 'sh', '-c', 'while :; do :; done']) for _ in range(n)]
            try:
                time.sleep(0.2)
                for rep in range(2):
                    rc, dt, data = run(kopt, p, mach, out, True)
                    tag = 'throttled %d hog%s #%d' % (n, '' if n == 1 else 's', rep + 1)
                    if rc != 0:
                        res = 'EXIT %s' % rc; bad += 1
                    else:
                        res = 'identical' if data == ref else 'DIFFERENT'
                        if data != ref: bad += 1
                        if dt > LIMIT:
                            res += ' OVER-30s'; bad += 1
                    print('%-18s %-22s %7.2f %7.1fx  %s' % (name, tag, dt, dt / base, res), flush=True)
            finally:
                for h in hogs:
                    h.kill(); h.wait()
    print('problems: %d' % bad)
    sys.exit(1 if bad else 0)


if __name__ == '__main__':
    main()
