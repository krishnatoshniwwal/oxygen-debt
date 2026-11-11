"""backend.py - work on the final (non-SSA) code: clean-up, block order, duplication."""
from ir import Instr, compute_preds, analyze, count_instrs


def final_cleanup(f):
    """Jump threading, block merging and unreachable-block removal on non-SSA code."""
    changed = True
    while changed:
        changed = False
        # normalise branches
        for b in f.blocks:
            term = b.ins[-1]
            if term.op == 'br':
                if term.t[0] is term.t[1]:
                    term.op = 'jmp'
                    term.a = []
                    term.t = [term.t[0]]
                    changed = True
                elif term.a[0] < 0:
                    tk = term.t[0] if ~term.a[0] != 0 else term.t[1]
                    term.op = 'jmp'
                    term.a = []
                    term.t = [tk]
                    changed = True
        # forward jumps to blocks that only contain a jump
        fwd = {}
        for b in f.blocks:
            if len(b.ins) == 1 and b.ins[0].op == 'jmp' and b.ins[0].t[0] is not b:
                fwd[b.id] = b.ins[0].t[0]

        def final(t):
            n = 0
            while t.id in fwd and n < 1000:
                t = fwd[t.id]
                n += 1
            return t

        if fwd:
            for b in f.blocks:
                tt = b.ins[-1].t
                if tt:
                    for i in range(len(tt)):
                        nt = final(tt[i])
                        if nt is not tt[i]:
                            tt[i] = nt
                            changed = True
            e = f.blocks[0]
            if e.id in fwd:
                ne = final(e)
                if ne is not e and ne.id not in fwd:
                    f.blocks.remove(ne)
                    f.blocks.insert(0, ne)
                    changed = True
        # unreachable blocks
        entry = f.blocks[0]
        seen = set([entry.id])
        stack = [entry]
        while stack:
            b = stack.pop()
            for s in b.succs():
                if s.id not in seen:
                    seen.add(s.id)
                    stack.append(s)
        if len(seen) != len(f.blocks):
            f.blocks = [b for b in f.blocks if b.id in seen]
            changed = True
        compute_preds(f)
        # merge straight-line pairs
        dead = set()
        for b in f.blocks:
            if b.id in dead:
                continue
            while True:
                term = b.ins[-1]
                if term.op != 'jmp':
                    break
                s = term.t[0]
                if s is b or s is entry or len(s.preds) != 1 or s.id in dead:
                    break
                b.ins.pop()
                b.ins.extend(s.ins)
                dead.add(s.id)
                for ss in s.succs():
                    for i in range(len(ss.preds)):
                        if ss.preds[i] is s:
                            ss.preds[i] = b
                changed = True
        if dead:
            f.blocks = [b for b in f.blocks if b.id not in dead]
            compute_preds(f)


def estimate_freq(f, profile=None):
    """Give every block a frequency estimate: from the profile when it names the block,
    otherwise from the loop depth."""
    analyze(f)
    for b in f.blocks:
        b.freq = float(min(8 ** b.depth, 10 ** 9))
    if profile:
        byname = dict((b.name, b) for b in f.blocks)
        cnt = {}
        for (src, dst), c in profile.items():
            if dst in byname:
                cnt[dst] = cnt.get(dst, 0) + c
        if cnt:
            entry = f.blocks[0]
            for name, c in cnt.items():
                b = byname[name]
                # mix with the static estimate so that a wrong profile cannot zero a hot block
                b.freq = 0.5 * b.freq + float(c)
            if entry.name not in cnt:
                entry.freq = max(entry.freq, 1.0)


def edge_weights(f, profile=None):
    """List of (weight, order, src, dst) for all CFG edges."""
    edges = []
    n = 0
    for b in f.blocks:
        ss = b.succs()
        if not ss:
            continue
        if len(ss) == 1:
            probs = [1.0]
        else:
            # an edge that stays in the loop is likelier than one that leaves it
            lp = b.loop
            in0 = lp is not None and ss[0].id in lp.ids
            in1 = lp is not None and ss[1].id in lp.ids
            if in0 and not in1:
                probs = [0.9, 0.1]
            elif in1 and not in0:
                probs = [0.1, 0.9]
            else:
                probs = [0.5, 0.5]
        for s, p in zip(ss, probs):
            w = b.freq * p
            if profile:
                c = profile.get((b.name, s.name))
                if c is not None:
                    w = 0.5 * w + float(c)
            edges.append((w, n, b, s))
            n += 1
    return edges


def layout(f, profile=None):
    """Order the blocks so that hot edges fall through (greedy chaining)."""
    estimate_freq(f, profile)
    entry = f.blocks[0]
    edges = edge_weights(f, profile)
    edges.sort(key=lambda e: (-e[0], e[1]))
    chain_of = {}
    chains = {}
    for b in f.blocks:
        chain_of[b.id] = b.id
        chains[b.id] = [b]
    for w, _, s, d in edges:
        if d is entry or s is d:
            continue
        cs, cd = chain_of[s.id], chain_of[d.id]
        if cs == cd:
            continue
        if chains[cs][-1] is not s or chains[cd][0] is not d:
            continue
        chains[cs].extend(chains[cd])
        for x in chains[cd]:
            chain_of[x.id] = cs
        del chains[cd]
    ec = chain_of[entry.id]
    rest = [c for c in chains if c != ec]
    order_index = dict((b.id, i) for i, b in enumerate(f.blocks))
    rest.sort(key=lambda c: (-max(x.freq for x in chains[c]), order_index[chains[c][0].id]))
    out = list(chains[ec])
    for c in rest:
        out.extend(chains[c])
    f.blocks = out


def parse_profile(text):
    prof = {}
    for raw in text.splitlines():
        line = raw.split('#', 1)[0].strip()
        if not line:
            continue
        toks = line.split()
        if len(toks) != 4 or toks[0] != 'edge':
            continue
        try:
            c = int(toks[3])
        except ValueError:
            continue
        if c < 0:
            continue
        key = (toks[1], toks[2])
        prof[key] = prof.get(key, 0) + c
    return prof


def tail_dup(f, maxdup, budget):
    """Replace 'jmp S' by a copy of S when S is small (this also rotates loops:
    the loop test is copied to the end of the loop body). Pure code copying, always legal.
    `budget` is the number of extra instructions we are allowed to add."""
    if maxdup <= 0:
        return False
    changed = False
    for _ in range(3):
        compute_preds(f)
        any_ = False
        for b in f.blocks:
            term = b.ins[-1]
            if term.op != 'jmp':
                continue
            s = term.t[0]
            if s is b:
                continue
            n = len(s.ins)
            if n > maxdup and n > 1:
                continue
            if s.ins[-1].op == 'jmp' and s.ins[-1].t[0] is s:
                continue
            extra = n - 1
            if extra > budget:
                continue
            budget -= extra
            b.ins.pop()
            for ins in s.ins:
                b.ins.append(ins.copy())
            any_ = True
        if not any_:
            break
        changed = True
        final_cleanup(f)
    return changed


def unroll(f, factor, budget):
    """Copy the body of every innermost loop `factor` times in a row. Every copy keeps its
    own exit tests, so this is pure code copying: the back edge of copy i goes to copy i+1.
    It saves taken-jump penalties. `budget` = extra instructions allowed."""
    if factor <= 1:
        return False
    compute_preds(f)
    analyze(f)
    inner = []
    for lp in f.loops:
        if any(o.parent is lp for o in f.loops):
            continue
        inner.append(lp)
    inner.sort(key=lambda lp: lp.header.rpo)
    changed = False
    for lp in inner:
        size = sum(len(b.ins) for b in lp.blocks)
        k = factor
        while k > 1 and size * (k - 1) > budget:
            k -= 1
        if k <= 1:
            continue
        budget -= size * (k - 1)
        h = lp.header
        copies = [dict((b.id, b) for b in lp.blocks)]
        newblocks = []
        for j in range(1, k):
            m = {}
            for b in lp.blocks:
                nb = f.new_block(b.name + '_u%d' % j)
                nb.ins = [ins.copy() for ins in b.ins]
                m[b.id] = nb
                newblocks.append(nb)
            copies.append(m)
        for j in range(k):
            m = copies[j]
            nxt = copies[(j + 1) % k]
            for b in lp.blocks:
                cb = m[b.id]
                tt = cb.ins[-1].t
                if not tt:
                    continue
                for i in range(len(tt)):
                    t = tt[i]
                    if t is h:
                        tt[i] = nxt[h.id]
                    elif t.id in lp.ids:
                        tt[i] = m[t.id]
        f.blocks.extend(newblocks)
        changed = True
    if changed:
        compute_preds(f)
    return changed
