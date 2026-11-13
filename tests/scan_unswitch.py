#!/usr/bin/env python3
"""Count loops that contain a loop-invariant conditional branch (loop-unswitching candidates).

Usage: python3 tests/scan_unswitch.py [PROGDIR_OR_FILES...]   (default kit/bench)
A natural loop = back edge (tail -> header, header dominates tail). A `br` inside the loop is
invariant when its condition register is not defined in the loop, or is defined only by pure
non-memory instructions whose operands are all invariant (computed to a fixpoint, single def).
Branches that exit the loop (one target outside) are reported separately from in-loop diamonds.
"""
import glob, os, sys

ROOT = os.path.dirname(os.path.dirname(os.path.realpath(__file__)))
sys.path.insert(0, os.path.join(ROOT, 'kit'))
sys.dont_write_bytecode = True
import kirlib as K  # noqa

IMPURE = ('load', 'vload', 'store', 'vstore', 'div', 'mod', 'trap', 'br', 'jmp', 'ret', 'vin', 'vout', 'in', 'out')


def analyse(prog):
    ins = prog.instrs
    labels = prog.block_labels
    blocks = {l: [] for l in labels}
    for i, b in enumerate(prog.block_of):
        blocks[b].append(i)
    succ = {}
    for idx, l in enumerate(labels):
        last = ins[blocks[l][-1]] if blocks[l] else None
        s = []
        if last is None or last.op not in K.TERMINATORS:
            if idx + 1 < len(labels):
                s = [labels[idx + 1]]
        elif last.op == 'jmp':
            s = [last.args[0][1]]
        elif last.op == 'br':
            s = [last.args[1][1], last.args[2][1]]
        succ[l] = s
    # dominators (iterative)
    entry = labels[0]
    allb = set(labels)
    dom = {l: set(allb) for l in labels}
    dom[entry] = {entry}
    pred = {l: [] for l in labels}
    for l in labels:
        for s in succ[l]:
            pred[s].append(l)
    ch = True
    while ch:
        ch = False
        for l in labels[1:]:
            ps = [dom[p] for p in pred[l]]
            nd = ({l} | set.intersection(*ps)) if ps else {l}
            if nd != dom[l]:
                dom[l] = nd
                ch = True
    loops = {}
    for t in labels:
        for h in succ[t]:
            if h in dom[t]:
                body = {h, t}
                st = [t]
                while st:
                    x = st.pop()
                    if x == h:
                        continue
                    for p in pred[x]:
                        if p not in body:
                            body.add(p)
                            st.append(p)
                loops.setdefault(h, set()).update(body)
    res = []
    for h, body in loops.items():
        defs = {}
        for b in body:
            for i in blocks[b]:
                ii = ins[i]
                if ii.op in K.TERMINATORS or not ii.args or ii.args[0][0] != 'r':
                    continue
                if ii.op in ('store', 'vstore'):
                    continue
                defs.setdefault(ii.args[0][1], []).append(ii)
        inv = set()
        ch = True
        while ch:
            ch = False
            for r, dl in defs.items():
                if r in inv or len(dl) != 1:
                    continue
                d = dl[0]
                if d.op in IMPURE:
                    continue
                if all(k != 'r' or v in inv or v not in defs for k, v in d.args[1:]):
                    inv.add(r)
                    ch = True
        for b in sorted(body):
            if not blocks[b]:
                continue
            last = ins[blocks[b][-1]]
            if last.op != 'br' or last.args[0][0] != 'r':
                continue
            c = last.args[0][1]
            if c in defs and c not in inv:
                continue
            tg = [x[1] for x in last.args[1:3]]
            kind = 'exit' if any(s not in body for s in tg) else 'inloop'
            res.append((h, b, kind))
    return res


def main():
    args = sys.argv[1:] or [os.path.join(ROOT, 'kit/bench')]
    files = []
    for a in args:
        files += sorted(glob.glob(os.path.join(a, '*.kir'))) if os.path.isdir(a) else [a]
    n_prog = n_loops = n_in = 0
    names = []
    total = 0
    for f in files:
        name = os.path.basename(f)[:-4]
        try:
            prog = K.parse_program(K.read_text(f))
            r = analyse(prog)
        except Exception as e:
            print('skip %s: %s' % (name, e))
            continue
        total += 1
        inl = [x for x in r if x[2] == 'inloop']
        if inl:
            n_prog += 1
            n_in += len(inl)
            names.append('%s(%d)' % (name, len(inl)))
        if r:
            print('%-24s invariant branches: %s' % (name, ', '.join('%s@%s:%s' % (b, h, k) for h, b, k in r)))
    print()
    print('programs scanned: %d' % total)
    print('programs with a loop-invariant in-loop (both targets inside loop) branch: %d' % n_prog)
    print('total such branches: %d' % n_in)
    print('names: ' + ' '.join(names))
    print('recommendation: ' + ('worthwhile' if n_prog >= 5 else 'not worthwhile (too few candidates: %d programs)' % n_prog))


if __name__ == '__main__':
    main()
