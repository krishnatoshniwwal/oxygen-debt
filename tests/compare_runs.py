#!/usr/bin/env python3
"""Compare two tests/measure.py results: python3 tests/compare_runs.py A.json B.json"""
import json, sys, collections
a = json.load(open(sys.argv[1])); b = json.load(open(sys.argv[2]))
def tot(r, idx):  # idx 0 = sample .in, 'var' = 3 variants
    s0 = s1 = 0
    for k, v in r.items():
        if 'cyc' not in v: continue
        cs = v['cyc'][:1] if idx == 0 else v['cyc'][1:]
        for c in cs:
            if c: s0 += c[0]; s1 += c[1]
    return s0, s1
for nm, r in (('base', a), ('new', b)):
    bad = [k for k, v in r.items() if v['status'] != 'ok']
    print('%-5s fails=%d  tmax=%.2fs  sample: %d -> %d   variants: %d -> %d' % ((nm, len(bad), max(v['t'] for v in r.values())) + tot(r, 0) + tot(r, 'v')), bad[:5])
permach = collections.defaultdict(lambda: [0, 0, 0])
diffs = []
for k in a:
    if 'cyc' not in a[k] or 'cyc' not in b.get(k, {}): continue
    ca = sum(c[1] for c in a[k]['cyc'] if c); cb = sum(c[1] for c in b[k]['cyc'] if c); c0 = sum(c[0] for c in a[k]['cyc'] if c)
    m = k.split('|')[1]; permach[m][0] += c0; permach[m][1] += ca; permach[m][2] += cb
    if ca != cb: diffs.append((cb - ca, k, ca, cb, c0))
for m, (c0, ca, cb) in sorted(permach.items()): print('  %-14s orig %7d  base %7d  new %7d  (%+.2f%%)' % (m, c0, ca, cb, 100.0 * (cb - ca) / ca))
diffs.sort()
print('changed cases: %d; better %d, worse %d' % (len(diffs), sum(d[0] < 0 for d in diffs), sum(d[0] > 0 for d in diffs)))
print('biggest gains:'); [print('   %-34s %6d -> %6d  (orig %d)' % (d[1], d[2], d[3], d[4])) for d in diffs[:12]]
print('biggest losses:'); [print('   %-34s %6d -> %6d  (orig %d)' % (d[1], d[2], d[3], d[4])) for d in diffs[::-1][:12] if d[0] > 0]
print('slower than original (new):', [(k, sum(c[0] for c in v['cyc'] if c), sum(c[1] for c in v['cyc'] if c)) for k, v in b.items() if 'cyc' in v and sum(c[1] for c in v['cyc'] if c) > sum(c[0] for c in v['cyc'] if c)])
print('slower than original (base):', [(k, sum(c[0] for c in v['cyc'] if c), sum(c[1] for c in v['cyc'] if c)) for k, v in a.items() if 'cyc' in v and sum(c[1] for c in v['cyc'] if c) > sum(c[0] for c in v['cyc'] if c)])
