#!/usr/bin/env python3
"""Ruler: how close is kopt's output to the machine's limits?

Usage: python3 tests/ruler.py [KOPT_BINARY] [--only SUBSTR] [--large FILE.kir]
For every kit/bench/*.kir (with .in) x kit/machines/*.cfg: run kopt, execute the optimized program on
the .in file and report cycles, dynamic instructions n, IPC = n/cycles vs issue width W, and lower
bounds: critical path (dataflow, infinite width, no penalties), issue bound ceil(n/W), memory bound
ceil(mem/ports). 'gap' = cycles / max(bounds). Dominant limit is a heuristic label.
"""
import glob, os, subprocess, sys, tempfile, math

ROOT = os.path.dirname(os.path.dirname(os.path.realpath(__file__)))
sys.path.insert(0, os.path.join(ROOT, 'kit'))
sys.dont_write_bytecode = True
import kirlib as K  # noqa


def critical_path(cp, trace, mc):
    lat = [mc[K.LATKEY[n]] if n in K.LATKEY else 1 for n in cp.opnames]
    ready = [0] * cp.nregs
    best = 0
    for pc in trace:
        t = 0
        for r in cp.srcs[pc]:
            t = max(t, ready[r])
        d = cp.dst[pc]
        if d >= 0:
            t = max(t, ready[d])
            ready[d] = t + lat[pc]
        best = max(best, t + 1)
    return best


def analyse(cp, trace, mc):
    n = len(trace)
    cyc = K.time_trace(cp, trace, mc)
    mem = sum(1 for pc in trace if cp.ismem[pc])
    cpath = critical_path(cp, trace, mc)
    ib = -(-n // mc['issue_width'])
    mb = -(-mem // mc['mem_ports'])
    lb = max(cpath, ib, mb)
    # same trace, no icache penalty / no taken-branch penalty: isolates those costs
    m2 = dict(mc); m2['icache_penalty'] = 0
    cyc_noq = K.time_trace(cp, trace, m2)
    m3 = dict(m2); m3['taken_branch_penalty'] = 0
    cyc_nob = K.time_trace(cp, trace, m3)
    return dict(n=n, cyc=cyc, cpath=cpath, ib=ib, mb=mb, lb=lb, noq=cyc_noq, nob=cyc_nob)


def label(a, mc):
    if a['cyc'] <= 1.1 * a['lb']:
        return 'at-bound(' + ('issue' if a['ib'] >= a['cpath'] else 'latency') + ')'
    parts = []
    if a['cyc'] - a['noq'] > 0.3 * a['cyc']:
        parts.append('Q')
    if a['noq'] - a['nob'] > 0.2 * a['cyc']:
        parts.append('branch')
    return '+'.join(parts) or 'stall/spill'


def run_kopt(kopt, p, mf, out):
    r = subprocess.run([kopt, p, '--machine', mf, '-o', out], capture_output=True, text=True)
    if r.returncode != 0:
        raise RuntimeError('kopt exit %d' % r.returncode)
    return K.parse_program(K.read_text(out))


def main():
    a = sys.argv[1:]
    kopt = os.path.join(ROOT, 'target/release/kopt_bin')
    only = None
    large = None
    pos = []
    i = 0
    while i < len(a):
        if a[i] == '--only':
            only = a[i + 1]; i += 2
        elif a[i] == '--large':
            large = a[i + 1]; i += 2
        else:
            pos.append(a[i]); i += 1
    if pos:
        kopt = pos[0]
    tmp = tempfile.mkdtemp()
    out = os.path.join(tmp, 'o.kir')
    progs = [large] if large else sorted(glob.glob(os.path.join(ROOT, 'kit/bench/*.kir')))
    machines = sorted(glob.glob(os.path.join(ROOT, 'kit/machines/*.cfg')))
    print('%-18s %-12s %3s %8s %8s %5s %8s %8s %7s %5s  %s' % (
        'prog', 'machine', 'W', 'n', 'cycles', 'IPC', 'critpath', 'issueLB', 'LB', 'gap', 'limit'))
    for p in progs:
        name = os.path.basename(p)[:-4]
        if only and only not in name:
            continue
        orig = K.parse_program(K.read_text(p))
        ip = p[:-4] + '.in'
        if not os.path.exists(ip):
            continue
        inp = K.load_input(ip, len(orig.params))
        for mf in machines:
            mc = K.load_machine(mf)
            mname = os.path.basename(mf)[8:-4]
            try:
                new = run_kopt(kopt, p, mf, out)
                cp = K.compile_program(new)
                r = K.execute(cp, inp.args, inp.segments, inp.vin, 40000000, want_trace=True)
                if r.status == 'TIMEOUT':
                    print('%-18s %-12s TIMEOUT' % (name, mname)); continue
                an = analyse(cp, r.trace, mc)
            except Exception as e:
                print('%-18s %-12s ERROR %s' % (name, mname, e)); continue
            print('%-18s %-12s %3d %8d %8d %5.2f %8d %8d %7d %5.2f  %s' % (
                name, mname, mc['issue_width'], an['n'], an['cyc'], an['n'] / an['cyc'],
                an['cpath'], an['ib'], an['lb'], an['cyc'] / max(1, an['lb']), label(an, mc)), flush=True)


if __name__ == '__main__':
    main()
