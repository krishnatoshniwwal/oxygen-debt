#!/usr/bin/env python3
"""Extreme-machine sweep for kopt.

Usage: python3 tests/machine_sweep.py [KOPT] [OUT_JSON] [--only SUBSTR] [--machines a,b] [--jobs N] [--ngen N]
Generates extreme machine files (registers 4-6, huge spill penalty, issue width 1/8, mem_ports 1/4,
icache 8/16/4096, huge icache penalty, div 40, load 12, fused 0/1, branch penalty 0/10), runs kopt on every
kit/bench program for each, checks output vs input with kit/kirlib.py (sample .in + 3 variants + ngen generated
inputs) and counts cycles on sample + 3 variants.  Needs kit/ restored:  git archive a32c393~1 kit | tar -x
Writes OUT_JSON (default machine_sweep.json); the report is made from it by machine_sweep.py --report JSON.
"""
import glob, json, os, random, subprocess, sys, tempfile, time, zlib
from multiprocessing import Pool

ROOT = os.path.dirname(os.path.dirname(os.path.realpath(__file__)))
sys.path.insert(0, os.path.join(ROOT, 'kit'))
sys.dont_write_bytecode = True
import kirlib as K  # noqa

BASE = dict(issue_width=2, mem_ports=1, registers=8, reg_spill_penalty=5, taken_branch_penalty=2,
            icache_size=64, icache_penalty=2, lat_alu=1, lat_mul=3, lat_div=12, lat_load=4, lat_mac=3,
            lat_addsh=1, fused_mac=1, fused_addsh=1)

MACHINES = {
    'base': {},
    'k4': dict(registers=4),
    'k5_spill200': dict(registers=5, reg_spill_penalty=200),
    'k6_spill50': dict(registers=6, reg_spill_penalty=50),
    'k4_spill1000_w1': dict(registers=4, reg_spill_penalty=1000, issue_width=1),
    'k8_spill0': dict(reg_spill_penalty=0),
    'w1_m1': dict(issue_width=1, mem_ports=1),
    'w8_m4': dict(issue_width=8, mem_ports=4),
    'w8_m1': dict(issue_width=8, mem_ports=1),
    'w1_m4': dict(issue_width=1, mem_ports=4),
    'ic8': dict(icache_size=8),
    'ic16_pen20': dict(icache_size=16, icache_penalty=20),
    'ic8_pen1000': dict(icache_size=8, icache_penalty=1000),
    'ic4096': dict(icache_size=4096),
    'ic4096_pen1000': dict(icache_size=4096, icache_penalty=1000),
    'ic64_pen1000': dict(icache_penalty=1000),
    'div40': dict(lat_div=40),
    'load12': dict(lat_load=12),
    'load12_div40_mul8': dict(lat_load=12, lat_div=40, lat_mul=8),
    'nofused': dict(fused_mac=0, fused_addsh=0),
    'nomac': dict(fused_mac=0),
    'noaddsh': dict(fused_addsh=0),
    'br0': dict(taken_branch_penalty=0),
    'br10': dict(taken_branch_penalty=10),
    'br100_ic16': dict(taken_branch_penalty=100, icache_size=16),
    'lat_all1': dict(lat_alu=1, lat_mul=1, lat_div=1, lat_load=1, lat_mac=1, lat_addsh=1),
    'lat_alu4': dict(lat_alu=4, lat_addsh=4),
    'wide_fast': dict(issue_width=8, mem_ports=4, lat_load=1, lat_mul=1, lat_div=2, registers=32, icache_size=4096),
    'tiny_slow': dict(issue_width=1, mem_ports=1, registers=4, reg_spill_penalty=100, icache_size=8,
                      icache_penalty=50, taken_branch_penalty=10, lat_div=40, lat_load=12, fused_mac=0, fused_addsh=0),
}


def write_machine(path, ov):
    d = dict(BASE); d.update(ov)
    with open(path, 'w') as f:
        for k in BASE:
            f.write('%s = %d\n' % (k, d[k]))


def variants(inp, n, seed):
    rnd = random.Random(seed)
    out = []
    for _ in range(n):
        def pert(v):
            s = v - (1 << 32) if v >= (1 << 31) else v
            if abs(s) <= 1:
                return v
            if abs(s) > 100000:
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


def job(a):
    kopt, mpath, mname, prog, ngen, tmpd = a
    mc = K.load_machine(mpath)
    name = os.path.basename(prog)[:-4]
    key = '%s|%s' % (name, mname)
    out = os.path.join(tmpd, '%s.%s.kir' % (name, mname))
    cmd = [kopt, prog, '--machine', mpath, '-o', out]
    prof = prog[:-4] + '.prof'
    if os.path.exists(prof):
        cmd[4:4] = ['--profile', prof]
    t0 = time.time()
    r = subprocess.run(cmd, capture_output=True, text=True)
    dt = time.time() - t0
    if r.returncode != 0:
        return key, {'status': 'EXIT %d %s' % (r.returncode, r.stderr[-300:]), 't': dt}
    orig = K.parse_program(K.read_text(prog))
    try:
        new = K.parse_program(K.read_text(out))
        K.check_machine_features(new, mc)
    except K.KirError as e:
        return key, {'status': 'INVALID %s' % e, 't': dt}
    inp = K.load_input(prog[:-4] + '.in', len(orig.params))
    vs = variants(inp, 3, zlib.crc32(name.encode()))
    cpo, cpn = K.compile_program(orig), K.compile_program(new)
    status = 'ok'
    for ii in [inp] + vs + K.generate_inputs(len(orig.params), ngen, 777):
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
    try:
        os.remove(out)
    except OSError:
        pass
    return key, {'status': status, 't': dt, 'cyc': cyc}


def report(res):
    """Print summary; returns text."""
    L = []
    fails = {k: v['status'] for k, v in res.items() if v['status'] != 'ok'}
    L.append('cases: %d, failures: %d' % (len(res), len(fails)))
    for k, s in sorted(fails.items()):
        L.append('FAIL %s: %s' % (k, s[:200]))
    ratios = {}
    slower = []
    for k, v in res.items():
        if v['status'] != 'ok':
            continue
        for i, c in enumerate(v['cyc']):
            if c is None:
                continue
            o, n = c
            if n > o:
                slower.append((n / o, k, 'sample' if i == 0 else 'variant%d' % i, o, n))
            if i == 0:
                ratios[k] = n / o if o else 1.0
    L.append('slower-than-original (sample or variants): %d' % len(slower))
    for r, k, w, o, n in sorted(slower, reverse=True)[:40]:
        L.append('  %.3f %s %s orig=%d opt=%d' % (r, k, w, o, n))
    L.append('worst sample ratios:')
    for k, r in sorted(ratios.items(), key=lambda x: -x[1])[:25]:
        L.append('  %.3f %s' % (r, k))
    per = {}
    for k, r in ratios.items():
        p, m = k.split('|')
        per.setdefault(p, {})[m] = r
    L.append('programs with largest ratio spread across machines:')
    spread = sorted(per.items(), key=lambda x: -(max(x[1].values()) - min(x[1].values())))[:20]
    for p, d in spread:
        lo = min(d, key=d.get); hi = max(d, key=d.get)
        L.append('  %s: %.3f (%s) .. %.3f (%s)' % (p, d[lo], lo, d[hi], hi))
    L.append('per-machine totals (sample, opt/orig):')
    ms = {}
    for k, v in res.items():
        if v['status'] == 'ok' and v['cyc'][0]:
            m = k.split('|')[1]
            a = ms.setdefault(m, [0, 0]); a[0] += v['cyc'][0][0]; a[1] += v['cyc'][0][1]
    for m, (o, n) in sorted(ms.items()):
        L.append('  %-20s orig=%9d opt=%9d ratio=%.4f' % (m, o, n, n / o))
    mt = max((v['t'], k) for k, v in res.items())
    L.append('slowest kopt run: %.1fs %s' % mt)
    return '\n'.join(L)


def main():
    a = sys.argv[1:]
    if '--report' in a:
        print(report(json.load(open(a[a.index('--report') + 1]))))
        return
    def opt(flag, default):
        if flag in a:
            i = a.index(flag); v = a[i + 1]; del a[i:i + 2]; return v
        return default
    only = opt('--only', None)
    msel = opt('--machines', None)
    jobs = int(opt('--jobs', os.cpu_count() or 2))
    ngen = int(opt('--ngen', 30))
    kopt = os.path.realpath(a[0]) if a else os.path.join(ROOT, 'kopt')
    outp = a[1] if len(a) > 1 else 'machine_sweep.json'
    progs = sorted(glob.glob(os.path.join(ROOT, 'kit/bench/*.kir')))
    if only:
        progs = [p for p in progs if only in p]
    names = msel.split(',') if msel else list(MACHINES)
    tmpd = tempfile.mkdtemp(prefix='msweep')
    work = []
    for n in names:
        mp = os.path.join(tmpd, 'machine_%s.cfg' % n)
        write_machine(mp, MACHINES[n])
        K.load_machine(mp)   # validate
        work += [(kopt, mp, n, p, ngen, tmpd) for p in progs]
    res = {}
    with Pool(jobs) as pool:
        for i, (k, v) in enumerate(pool.imap_unordered(job, work, chunksize=1)):
            res[k] = v
            if v['status'] != 'ok':
                print('FAIL', k, v['status'][:200], flush=True)
            if i % 200 == 199:
                print('%d/%d' % (i + 1, len(work)), flush=True)
    json.dump(res, open(outp, 'w'), indent=0, sort_keys=True)
    print(report(res))


if __name__ == '__main__':
    main()
