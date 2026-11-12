#!/usr/bin/env python3
"""kinds.py A.json B.json [names...]: per-program totals (sum over machines) for the state/control kinds."""
import json, sys, collections
a = json.load(open(sys.argv[1])); b = json.load(open(sys.argv[2]))
names = sys.argv[3:] or ['dfa_dispatch','vending_fsm','mini_interp','multi_exit','jump_chain','irreducible_loop','branchy_select']
for n in names:
    row = collections.defaultdict(lambda: [0,0,0])
    for k in a:
        if k.split('|')[0] != n or 'cyc' not in a[k]: continue
        m = k.split('|')[1]
        for ca, cb in zip(a[k]['cyc'], b[k]['cyc']):
            if ca and cb:
                row[m][0] += ca[0]; row[m][1] += ca[1]; row[m][2] += cb[1]
    print(n, ' '.join('%s:%d/%d/%d' % (m[:4], *v) for m, v in sorted(row.items())))
