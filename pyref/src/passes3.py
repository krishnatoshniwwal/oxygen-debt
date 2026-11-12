"""passes3.py - if-conversion, loop peeling / trip counts, instruction scheduling."""
from ir import Instr, L, MASK, PURE, analyze, compute_preds, dominates
from passes import evalop, cleanup_cfg
from passes2 import MemInfo, may_alias


# --------------------------------------------------------------------------
# If-conversion: branches over a few cheap, safe instructions become selects
# --------------------------------------------------------------------------
def _side_ok(s, b, j, limit, rng):
    if len(s.preds) != 1 or s.preds[0] is not b or s.phis or s is j or s is b:
        return False
    term = s.ins[-1]
    if term.op != 'jmp' or term.t[0] is not j:
        return False
    if len(s.ins) - 1 > limit:
        return False
    for ins in s.ins[:-1]:
        op = ins.op
        if op in PURE:
            continue
        if op in ('div', 'mod', 'load') and not rng.may_trap(ins, b):
            continue
        return False
    return True


def ifconv(f, rng, limit):
    if limit <= 0:
        return False
    changed = False
    for _ in range(8):
        any_ = False
        for b in f.blocks:
            term = b.ins[-1]
            if term.op != 'br':
                continue
            t, e = term.t
            if t is e:
                continue
            c = term.a[0]
            tj = t.ins[-1].t[0] if t.ins[-1].op == 'jmp' else None
            ej = e.ins[-1].t[0] if e.ins[-1].op == 'jmp' else None
            sides = None
            if tj is not None and tj is ej and _side_ok(t, b, tj, limit, rng) and _side_ok(e, b, tj, limit, rng):
                j = tj
                sides = (t, e)
            elif tj is e and _side_ok(t, b, e, limit, rng):
                j = e
                sides = (t, None)
            elif ej is t and _side_ok(e, b, t, limit, rng):
                j = t
                sides = (None, e)
            if sides is None or j is b:
                continue
            st, se = sides
            pt = st if st is not None else b
            pe = se if se is not None else b
            it = j.preds.index(pt)
            ie = j.preds.index(pe)
            nsel = 0
            for phi in j.phis:
                if phi.a[it] != phi.a[ie]:
                    nsel += 1
            if nsel > max(1, limit):
                continue
            b.ins.pop()
            if st is not None:
                b.ins.extend(st.ins[:-1])
            if se is not None:
                b.ins.extend(se.ins[:-1])
            for phi in j.phis:
                vt, ve = phi.a[it], phi.a[ie]
                if vt == ve:
                    v = vt
                else:
                    v = f.new_val()
                    b.ins.append(Instr('select', v, [c, vt, ve]))
                phi.a[it] = v
            b.ins.append(Instr('jmp', None, [], [j]))
            # predecessor slot `it` now stands for b; drop slot `ie`
            j.preds[it] = b
            del j.preds[ie]
            for phi in j.phis:
                del phi.a[ie]
            for s in (st, se):
                if s is not None:
                    s.preds = []
                    s.ins = [Instr('jmp', None, [], [s])]
            any_ = True
        if not any_:
            break
        changed = True
        cleanup_cfg(f)
    return changed


# --------------------------------------------------------------------------
# Trip counts of loops with a constant number of rounds (found by running the
# loop's own exit test on constants)
# --------------------------------------------------------------------------
def find_const_trips(f, maxtrip=64):
    """Return {header block id: number of header executions} for loops whose exit test
    only depends on constants. The result is only used to decide how much to peel;
    peeling is correct whatever this function says."""
    analyze(f)
    defs = {}
    for b in f.blocks:
        for ins in b.ins:
            if ins.d is not None:
                defs[ins.d] = ins
    out = {}
    for lp in f.loops:
        h = lp.header
        if not h.phis:
            continue
        out_idx = [i for i, p in enumerate(h.preds) if p.id not in lp.ids]
        in_idx = [i for i, p in enumerate(h.preds) if p.id in lp.ids]
        if not out_idx or not in_idx:
            continue
        env0 = {}
        nxt = {}
        for phi in h.phis:
            init = phi.a[out_idx[0]]
            if init >= 0 or any(phi.a[i] != init for i in out_idx):
                continue
            n = phi.a[in_idx[0]]
            if any(phi.a[i] != n for i in in_idx):
                continue
            env0[phi.d] = ~init
            nxt[phi.d] = n
        if not env0:
            continue
        # exit tests that run in every round
        tests = []
        for x in lp.blocks:
            term = x.ins[-1]
            if term.op != 'br':
                continue
            o0 = term.t[0].id not in lp.ids
            o1 = term.t[1].id not in lp.ids
            if o0 == o1:
                continue
            if all(dominates(x, h.preds[i]) for i in in_idx):
                tests.append((term.a[0], o0))
        if not tests:
            continue

        def ev(x, env, depth=0):
            if x < 0:
                return ~x
            if x in env:
                return env[x]
            if depth > 12:
                return None
            d = defs.get(x)
            if d is None or len(d.a) != 2 or d.op in ('load', 'vload'):
                return None
            p = ev(d.a[0], env, depth + 1)
            if p is None:
                return None
            q = ev(d.a[1], env, depth + 1)
            if q is None:
                return None
            return evalop(d.op, p, q)

        for c, exit_when_true in tests:
            env = dict(env0)
            stays = 0
            ok = False
            while stays <= maxtrip:
                v = ev(c, env)
                if v is None:
                    break
                if (v != 0) == exit_when_true:
                    ok = True
                    break
                stays += 1
                new = {}
                for p, n in nxt.items():
                    if p in env:
                        nv = ev(n, env)
                        if nv is not None:
                            new[p] = nv
                env = new
            if ok:
                cur = out.get(h.id)
                if cur is None or stays + 1 < cur:
                    out[h.id] = stays + 1
    return out


def peel(f, header, n):
    """Non-SSA form. Put n copies of the loop in front of it: entering the loop runs copy 0,
    its back edge goes to copy 1, ..., the last copy's back edge enters the original loop.
    Pure code copying. The caller must have run analyze()."""
    lp = None
    for l in f.loops:
        if l.header is header:
            lp = l
            break
    if lp is None or n <= 0:
        return False
    h = header
    copies = []
    newblocks = []
    for j in range(n):
        m = {}
        for b in lp.blocks:
            nb = f.new_block(b.name + '_p%d' % j)
            nb.ins = [ins.copy() for ins in b.ins]
            m[b.id] = nb
            newblocks.append(nb)
        copies.append(m)
    for j in range(n):
        m = copies[j]
        for b in lp.blocks:
            tt = m[b.id].ins[-1].t
            if not tt:
                continue
            for i in range(len(tt)):
                t = tt[i]
                if t is h:
                    tt[i] = copies[j + 1][h.id] if j + 1 < n else h
                elif t.id in lp.ids:
                    tt[i] = m[t.id]
    first = copies[0][h.id]
    for b in f.blocks:
        if b.id in lp.ids:
            continue
        tt = b.ins[-1].t
        if tt:
            for i in range(len(tt)):
                if tt[i] is h:
                    tt[i] = first
    if f.blocks[0] is h:
        f.blocks = [first] + f.blocks + [nb for nb in newblocks if nb is not first]
    else:
        f.blocks.extend(newblocks)
    return True


def peel_const_loops(f, trips, budget, maxtrip=16):
    """Non-SSA form: fully peel loops with a small constant trip count (innermost first)
    while the program stays within `budget` instructions. Returns True if anything changed."""
    if not trips:
        return False
    compute_preds(f)
    analyze(f)
    size = sum(len(b.ins) for b in f.blocks)
    loops = sorted(f.loops, key=lambda lp: (-lp.depth, lp.header.rpo))
    changed = False
    done_ids = set()
    for lp in loops:
        n = trips.get(lp.header.id)
        if n is None or n > maxtrip or n < 1:
            continue
        # skip a loop whose inner loop was peeled in this round (the loop info is stale)
        if any(bid in done_ids for bid in lp.ids):
            continue
        lsize = sum(len(b.ins) for b in lp.blocks)
        if size + n * lsize > budget:
            continue
        if peel(f, lp.header, n):
            size += n * lsize
            changed = True
            for l2 in f.loops:
                if lp.header.id in l2.ids:
                    done_ids |= l2.ids
    if changed:
        compute_preds(f)
    return changed


# --------------------------------------------------------------------------
# Scheduling inside a block
# --------------------------------------------------------------------------
LATKEY = {
    'const': 'lat_alu', 'mov': 'lat_alu', 'add': 'lat_alu', 'sub': 'lat_alu', 'and': 'lat_alu',
    'or': 'lat_alu', 'xor': 'lat_alu', 'shl': 'lat_alu', 'shr': 'lat_alu', 'lt': 'lat_alu',
    'eq': 'lat_alu', 'select': 'lat_alu', 'mul': 'lat_mul', 'div': 'lat_div', 'mod': 'lat_div',
    'load': 'lat_load', 'vload': 'lat_load', 'mac': 'lat_mac', 'addsh': 'lat_addsh',
}
WINDOW = 40
AHEAD = 32


def schedule(f, mc, rng):
    """Reorder the instructions of each block (SSA form) so that the in-order machine waits
    less. Order is kept between: all visible effects and all instructions that may trap;
    a load and any store that may write its address."""
    W, MP = mc['issue_width'], mc['mem_ports']
    mi = MemInfo(f, rng)
    for b in f.blocks:
        n = len(b.ins) - 1
        if n < 3:
            continue
        ins_ = b.ins
        preds = [[] for _ in range(n)]
        defpos = {}
        last_chain = -1
        stores = []          # (pos, key, range)
        safe_loads = []      # (pos, key, range)
        lat = [0] * n
        for i in range(n):
            ins = ins_[i]
            op = ins.op
            pl = preds[i]
            for x in ins.a:
                if x >= 0:
                    j = defpos.get(x)
                    if j is not None:
                        pl.append(j)
            chain = False
            if op == 'store':
                chain = True
                k = mi.key(ins.a[0])
                r = rng.get(ins.a[0], b)
                for (j, k2, r2) in safe_loads:
                    if may_alias(k, r, k2, r2):
                        pl.append(j)
                stores.append((i, k, r))
            elif op == 'vload' or op == 'vstore':
                chain = True
            elif op == 'load':
                if rng.may_trap(ins, b):
                    chain = True
                else:
                    k = mi.key(ins.a[0])
                    r = rng.get(ins.a[0], b)
                    for (j, k2, r2) in reversed(stores):
                        if may_alias(k, r, k2, r2):
                            pl.append(j)
                            break
                    safe_loads.append((i, k, r))
            elif op == 'div' or op == 'mod':
                if rng.may_trap(ins, b):
                    chain = True
            if chain:
                if last_chain >= 0:
                    pl.append(last_chain)
                last_chain = i
            if ins.d is not None:
                defpos[ins.d] = i
                lat[i] = mc[LATKEY[op]]
        # a load that may trap must also stay after earlier stores (it is in the chain: yes)
        succs = [[] for _ in range(n)]
        npred = [0] * n
        for i in range(n):
            pl = preds[i]
            if pl:
                pl = sorted(set(pl))
                preds[i] = pl
                npred[i] = len(pl)
                for j in pl:
                    succs[j].append(i)
        # critical path to the end of the block
        cp = [0] * n
        term = ins_[n]
        tail = {}
        for x in term.a:
            if x >= 0 and x in defpos:
                tail[defpos[x]] = 1
        for i in range(n - 1, -1, -1):
            m = tail.get(i, 0)
            for s in succs[i]:
                if cp[s] > m:
                    m = cp[s]
            cp[i] = lat[i] + m
        ready = {}            # value -> cycle
        avail = [i for i in range(n) if npred[i] == 0]   # kept sorted by original position
        order = []
        t_last = 0
        used = 0
        memused = 0
        import bisect
        while avail:
            best = None
            bestkey = None
            lim = WINDOW if len(avail) > WINDOW else len(avail)
            # only look a limited distance ahead of the first waiting instruction, so that
            # values do not stay alive (and occupy registers) for much longer than before
            far = avail[0] + AHEAD
            for ai in range(lim):
                i = avail[ai]
                if i >= far:
                    break
                ins = ins_[i]
                e = t_last
                for x in ins.a:
                    if x >= 0:
                        r = ready.get(x)
                        if r is not None and r > e:
                            e = r
                ismem = ins.op in ('load', 'store', 'vload', 'vstore')
                if e == t_last and (used >= W or (ismem and memused >= MP)):
                    e += 1
                key = (e, -cp[i], i)
                if bestkey is None or key < bestkey:
                    bestkey = key
                    best = ai
            i = avail.pop(best)
            ins = ins_[i]
            e = bestkey[0]
            if e > t_last:
                used = 0
                memused = 0
                t_last = e
            used += 1
            if ins.op in ('load', 'store', 'vload', 'vstore'):
                memused += 1
            if ins.d is not None:
                ready[ins.d] = e + lat[i]
            order.append(i)
            for s in succs[i]:
                npred[s] -= 1
                if npred[s] == 0:
                    bisect.insort(avail, s)
        if len(order) == n:
            b.ins = [ins_[i] for i in order] + [term]
