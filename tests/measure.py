#!/usr/bin/env python3
"""Measure a kopt binary on every bench program x machine.

Usage: python3 tests/measure.py KOPT_BINARY OUT_JSON [--only SUBSTR]
Compare two runs with tests/compare_runs.py A.json B.json.
For each case: run kopt (timed), check against the original with the kit's checks
(sample .in + 60 generated inputs + the variants), and count cycles on the sample .in and on
3 variants of it (args perturbed within +-50% keeping sign, memory values perturbed likewise).
"""
import glob, json, os, random, subprocess, sys, time, zlib

ROOT = os.path.dirname(os.path.dirname(os.path.realpath(__file__)))
sys.path.insert(0, os.path.join(ROOT, 'kit'))
sys.dont_write_bytecode = True
import kirlib as K  # noqa


def variants(inp, n, seed):
    rnd = random.Random(seed)
    out = []
    for _ in range(n):
        def pert(v):
            s = v - (1 << 32) if v >= (1 << 31) else v
            if abs(s) <= 1:
                return v
            if abs(s) > 100000:          # a seed / hash constant: redraw fully
                return rnd.getrandbits(32)
            t = int(round(s * rnd.uniform(0.5, 1.5)))
            if t == 0:
                t = s
            return t & 0xFFFFFFFF
        args = [pert(a) for a in inp.args]
        segs = [(a, [pert(w) for w in ws]) for a, ws in inp.segments]
        vin = {p: [pert(w) for w in ws] for p, ws in inp.vin.items()}
        out.append(K.Inputs(args, segs, vin))
    return out


def main():
    kopt, outp = sys.argv[1], sys.argv[2]
    only = sys.argv[4] if len(sys.argv) > 4 and sys.argv[3] == '--only' else None
    machines = sorted(glob.glob(os.path.join(ROOT, 'kit/machines/*.cfg')))
    progs = sorted(glob.glob(os.path.join(ROOT, 'kit/bench/*.kir')))
    if only:
        progs = [p for p in progs if only in p]
    res = {}
    tmp = outp + '.tmp.kir'   # one per run: runs may go in parallel
    for m in machines:
        mc = K.load_machine(m)
        mn = os.path.basename(m)[8:-4]
        for p in progs:
            name = os.path.basename(p)[:-4]
            cmd = [kopt, p, '--machine', m, '-o', tmp]
            prof = p[:-4] + '.prof'
            if os.path.exists(prof):
                cmd[4:4] = ['--profile', prof]
            t0 = time.time()
            r = subprocess.run(cmd, capture_output=True, text=True)
            dt = time.time() - t0
            key = '%s|%s' % (name, mn)
            if r.returncode != 0:
                res[key] = {'status': 'EXIT %d' % r.returncode, 't': dt}
                continue
            orig = K.parse_program(K.read_text(p))
            try:
                new = K.parse_program(K.read_text(tmp))
                K.check_machine_features(new, mc)
            except K.KirError as e:
                res[key] = {'status': 'INVALID %s' % e, 't': dt}
                continue
            inp = K.load_input(p[:-4] + '.in', len(orig.params))
            vs = variants(inp, 3, zlib.crc32(name.encode()))
            cpo, cpn = K.compile_program(orig), K.compile_program(new)
            status = 'ok'
            for ii in [inp] + vs + K.generate_inputs(len(orig.params), 60, 777):
                ro = K.execute(cpo, ii.args, ii.segments, ii.vin, 300000)
                if ro.status == 'TIMEOUT':
                    continue
                rn = K.execute(cpn, ii.args, ii.segments, ii.vin, 20 * ro.steps + 1000)
                d = K.first_difference(ro, rn)
                if d:
                    status = 'MISMATCH ' + str(d)
                    break
            cyc = []
            for ii in [inp] + vs:
                ro = K.execute(cpo, ii.args, ii.segments, ii.vin, 2000000, want_trace=True)
                if ro.status == 'TIMEOUT':
                    cyc.append(None)
                    continue
                rn = K.execute(cpn, ii.args, ii.segments, ii.vin, 40000000, want_trace=True)
                cyc.append((K.time_trace(cpo, ro.trace, mc), K.time_trace(cpn, rn.trace, mc)))
            res[key] = {'status': status, 't': dt, 'cyc': cyc}
            if status != 'ok':
                print('FAIL', key, status, flush=True)
        print(mn, 'done', flush=True)
    json.dump(res, open(outp, 'w'), indent=0)


if __name__ == '__main__':
    main()
