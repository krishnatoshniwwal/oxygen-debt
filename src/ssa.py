"""ssa.py - into SSA form and back out (with register colouring)."""
import heapq

from ir import Instr, L, L0, analyze, compute_preds, MASK


# --------------------------------------------------------------------------
# SSA construction (pruned, Cytron et al.), with copy and constant propagation
# done by the renaming itself.
# --------------------------------------------------------------------------
def build_ssa(f):
    entry = f.blocks[0]
    if entry.preds:
        ne = f.new_block('entry')
        ne.ins.append(Instr('jmp', None, [], [entry]))
        f.blocks.insert(0, ne)
        entry.preds.append(ne)
        entry = ne
    rpo = analyze(f)

    # ---- liveness of the original registers (per block) ----
    use = {}
    defs = {}
    defblocks = {}
    for b in rpo:
        u = set()
        d = set()
        for ins in b.ins:
            for x in ins.a:
                if x >= 0 and x not in d:
                    u.add(x)
            dd = ins.d
            if dd is not None:
                d.add(dd)
        use[b.id] = u
        defs[b.id] = d
        for v in d:
            lst = defblocks.get(v)
            if lst is None:
                defblocks[v] = [b]
            else:
                lst.append(b)
    livein = dict((b.id, set(use[b.id])) for b in rpo)
    liveout = dict((b.id, set()) for b in rpo)
    work = list(rpo)
    inwork = set(b.id for b in rpo)
    while work:
        b = work.pop()
        inwork.discard(b.id)
        lo = liveout[b.id]
        n0 = len(lo)
        for s in b.succs():
            lo |= livein[s.id]
        if len(lo) != n0 or n0 == 0:
            li = livein[b.id]
            n1 = len(li)
            li |= (lo - defs[b.id])
            if len(li) != n1:
                for p in b.preds:
                    if p.id not in inwork:
                        inwork.add(p.id)
                        work.append(p)

    # ---- dominance frontiers ----
    df = dict((b.id, []) for b in rpo)
    for b in rpo:
        if len(b.preds) >= 2:
            for p in b.preds:
                r = p
                while r is not b.idom:
                    lst = df[r.id]
                    if not lst or lst[-1] is not b:
                        if b not in lst:
                            lst.append(b)
                    r = r.idom

    # ---- phi placement ----
    for v in sorted(defblocks):
        dbs = defblocks[v]
        has = set()
        inw = set(b.id for b in dbs)
        work = list(dbs)
        while work:
            x = work.pop()
            for y in df[x.id]:
                if y.id not in has and v in livein[y.id]:
                    has.add(y.id)
                    y.phis.append(Instr('phi', None, [L0] * len(y.preds), v))
                    if y.id not in inw:
                        inw.add(y.id)
                        work.append(y)

    # ---- renaming ----
    f.nval = 0
    stacks = {}
    newparams = []
    for p in f.params:
        nv = f.new_val()
        newparams.append(nv)
        stacks[p] = [nv]
    f.params = newparams
    stack = [(entry, None)]
    while stack:
        b, pushed = stack.pop()
        if pushed is not None:
            for v in pushed:
                stacks[v].pop()
            continue
        pushed = []
        for phi in b.phis:
            nv = f.new_val()
            v = phi.t
            s = stacks.get(v)
            if s is None:
                stacks[v] = [nv]
            else:
                s.append(nv)
            pushed.append(v)
            phi.d = nv
        out = []
        for ins in b.ins:
            a = ins.a
            for i in range(len(a)):
                x = a[i]
                if x >= 0:
                    s = stacks.get(x)
                    a[i] = s[-1] if s else L0
            op = ins.op
            d = ins.d
            if d is None:
                out.append(ins)
                continue
            if op == 'const' or op == 'mov':
                nv = a[0]
            elif op == 'mac':
                t = f.new_val()
                out.append(Instr('mul', t, [a[0], a[1]]))
                nv = f.new_val()
                out.append(Instr('add', nv, [t, a[2]]))
            elif op == 'addsh':
                t = f.new_val()
                out.append(Instr('mul', t, [a[1], L(1 << (~a[2]))]))
                nv = f.new_val()
                out.append(Instr('add', nv, [a[0], t]))
            else:
                nv = f.new_val()
                ins.d = nv
                out.append(ins)
            s = stacks.get(d)
            if s is None:
                stacks[d] = [nv]
            else:
                s.append(nv)
            pushed.append(d)
        b.ins = out
        for sblk in b.succs():
            if sblk.phis:
                idx = sblk.preds.index(b)
                for phi in sblk.phis:
                    s = stacks.get(phi.t)
                    phi.a[idx] = s[-1] if s else L0
        stack.append((b, pushed))
        for c in reversed(b.dkids):
            stack.append((c, None))
    for b in rpo:
        for phi in b.phis:
            phi.t = None


def check_ssa(f):
    """Debug helper: every value has one definition that dominates its uses."""
    from ir import dominates
    analyze(f)
    defblk = {}
    pos = {}
    for p in f.params:
        defblk[p] = f.blocks[0]
        pos[p] = -1
    for b in f.blocks:
        for phi in b.phis:
            assert phi.d not in defblk, 'double def v%d' % phi.d
            defblk[phi.d] = b
            pos[phi.d] = -1
            assert len(phi.a) == len(b.preds), 'phi arity in %s' % b.name
        for i, ins in enumerate(b.ins):
            if ins.d is not None:
                assert ins.d not in defblk, 'double def v%d' % ins.d
                defblk[ins.d] = b
                pos[ins.d] = i
    for b in f.blocks:
        for s in b.succs():
            assert b in s.preds, 'pred missing %s->%s' % (b.name, s.name)
        for p in b.preds:
            assert b in p.succs(), 'stale pred %s->%s' % (p.name, b.name)
        assert len(set(p.id for p in b.preds)) == len(b.preds), 'dup pred in %s' % b.name
        for phi in b.phis:
            for x, p in zip(phi.a, b.preds):
                if x >= 0:
                    assert x in defblk, 'undefined v%d in phi' % x
                    assert dominates(defblk[x], p), 'phi arg v%d does not dominate pred' % x
        for i, ins in enumerate(b.ins):
            for x in ins.a:
                if x >= 0:
                    assert x in defblk, 'undefined v%d in %s' % (x, b.name)
                    db = defblk[x]
                    if db is b:
                        assert pos[x] < i, 'use before def v%d in %s' % (x, b.name)
                    else:
                        assert dominates(db, b), 'def of v%d does not dominate use in %s' % (x, b.name)
        assert b.ins and b.ins[-1].op in ('jmp', 'br', 'ret', 'trap')
        for ins in b.ins[:-1]:
            assert ins.op not in ('jmp', 'br', 'ret', 'trap')


# --------------------------------------------------------------------------
# Out of SSA
# --------------------------------------------------------------------------
def destruct_ssa(f):
    """Replace every phi by copies through a fresh name (Sreedhar's method I).
    The copies are removed later by coalescing in the register allocator."""
    for b in f.blocks:
        if not b.phis:
            continue
        heads = []
        tails = [[] for _ in b.preds]
        for phi in b.phis:
            x = f.new_val()
            heads.append(Instr('mov', phi.d, [x]))
            for i, a in enumerate(phi.a):
                tails[i].append(Instr('mov', x, [a]))
        b.phis = []
        for i, p in enumerate(b.preds):
            p.ins[-1:-1] = tails[i]
        b.ins[0:0] = heads


def liveness(f):
    """Per-block live-out sets for a non-SSA function. Returns (liveout dict by block id)."""
    blocks = f.blocks
    use = {}
    dfn = {}
    for b in blocks:
        u = set()
        d = set()
        for ins in b.ins:
            for x in ins.a:
                if x >= 0 and x not in d:
                    u.add(x)
            if ins.d is not None:
                d.add(ins.d)
        use[b.id] = u
        dfn[b.id] = d
    livein = dict((b.id, set(use[b.id])) for b in blocks)
    liveout = dict((b.id, set()) for b in blocks)
    work = list(blocks)
    inwork = set(b.id for b in blocks)
    first = set(b.id for b in blocks)
    while work:
        b = work.pop()
        inwork.discard(b.id)
        lo = liveout[b.id]
        n0 = len(lo)
        for s in b.succs():
            lo |= livein[s.id]
        if len(lo) != n0 or b.id in first:
            first.discard(b.id)
            li = livein[b.id]
            n1 = len(li)
            li |= (lo - dfn[b.id])
            if len(li) != n1:
                for p in b.preds:
                    if p.id not in inwork:
                        inwork.add(p.id)
                        work.append(p)
    return liveout, livein


class TooBig(Exception):
    pass


MAX_EDGES = 40000000


def _build_graph(f):
    """Interference graph of a non-SSA function, with copies coalesced.
    Returns (nodes, adj, find, cost)."""
    compute_preds(f)
    liveout, livein = liveness(f)
    adj = {}
    nodes = []
    cost = {}

    def node(v):
        if v not in adj:
            adj[v] = set()
            nodes.append(v)
            cost[v] = 0.0

    for p in f.params:
        node(p)
    for i, p in enumerate(f.params):
        for q in f.params[:i]:
            adj[p].add(q)
            adj[q].add(p)
    copies = []
    entry = f.blocks[0]
    ninstr = 0
    for b in f.blocks:
        w = b.freq
        ninstr += len(b.ins)
        for ins in b.ins:
            if ins.d is not None:
                node(ins.d)
                cost[ins.d] += w
            for x in ins.a:
                if x >= 0:
                    node(x)
                    cost[x] += w
    if ninstr > 20000:
        # safety valve for huge functions: estimate the size of the interference graph
        # first and refuse to build one that would take too long
        est = 0
        for b in f.blocks:
            live = set(liveout[b.id])
            for ins in reversed(b.ins):
                if ins.d is not None:
                    est += len(live)
                    live.discard(ins.d)
                for x in ins.a:
                    if x >= 0:
                        live.add(x)
        if est > MAX_EDGES:
            raise TooBig('interference graph too large (%d)' % est)
    for b in f.blocks:
        live = set(liveout[b.id])
        w = b.freq
        for ins in reversed(b.ins):
            d = ins.d
            if d is not None:
                if ins.op == 'mov' and ins.a[0] >= 0:
                    s = ins.a[0]
                    copies.append((w, d, s))
                    ad = adj[d]
                    for l in live:
                        if l != d and l != s:
                            ad.add(l)
                            adj[l].add(d)
                else:
                    ad = adj[d]
                    for l in live:
                        if l != d:
                            ad.add(l)
                            adj[l].add(d)
                live.discard(d)
            for x in ins.a:
                if x >= 0:
                    live.add(x)
        if b is entry:
            # parameters are all defined together at the start
            for p in f.params:
                ap = adj[p]
                for l in live:
                    if l != p:
                        ap.add(l)
                        adj[l].add(p)

    # ---- coalescing (aggressive, hottest copies first) ----
    parent = {}

    def find(x):
        r = x
        while r in parent:
            r = parent[r]
        while x in parent:
            nx = parent[x]
            parent[x] = r
            x = nx
        return r

    order = sorted(range(len(copies)), key=lambda i: (-copies[i][0], i))
    for i in order:
        w, d, s = copies[i]
        rd, rs = find(d), find(s)
        if rd == rs or rs in adj[rd]:
            continue
        if len(adj[rd]) < len(adj[rs]):
            rd, rs = rs, rd
        # merge rs into rd
        ard = adj[rd]
        for n in adj[rs]:
            an = adj[n]
            an.discard(rs)
            an.add(rd)
            ard.add(n)
        del adj[rs]
        parent[rs] = rd
        cost[rd] += cost[rs] - 2 * w
    return nodes, adj, find, cost


def _colour(nodes, adj):
    """Greedy colouring, smallest-degree-last order. Returns (colour dict, number of colours)."""
    live_nodes = [v for v in nodes if v in adj]
    if len(live_nodes) > 4000:
        # very large function: colour in order of first appearance (cheap, and good for
        # long straight-line code)
        stack = live_nodes[::-1]
    else:
        deg = dict((v, len(adj[v])) for v in live_nodes)
        heap = [(deg[v], v) for v in live_nodes]
        heapq.heapify(heap)
        removed = set()
        stack = []
        while heap:
            dg, v = heapq.heappop(heap)
            if v in removed or dg != deg[v]:
                continue
            removed.add(v)
            stack.append(v)
            for n in adj[v]:
                if n not in removed:
                    deg[n] -= 1
                    heapq.heappush(heap, (deg[n], n))
    colour = {}
    ncol = 0
    for v in reversed(stack):
        used = set()
        for n in adj[v]:
            c = colour.get(n)
            if c is not None:
                used.add(c)
        c = 0
        while c in used:
            c += 1
        colour[v] = c
        if c + 1 > ncol:
            ncol = c + 1
    return colour, ncol


def _pick_spills(nodes, adj, cost, k, nospill):
    """Chaitin-style: remove nodes of degree < k; when stuck, spill the cheapest node."""
    live_nodes = [v for v in nodes if v in adj]
    deg = dict((v, len(adj[v])) for v in live_nodes)
    removed = set()
    spills = []
    low = [v for v in live_nodes if deg[v] < k]
    remaining = len(live_nodes)
    while remaining:
        while low:
            v = low.pop()
            if v in removed:
                continue
            removed.add(v)
            remaining -= 1
            for n in adj[v]:
                if n not in removed:
                    deg[n] -= 1
                    if deg[n] == k - 1:
                        low.append(n)
        if not remaining:
            break
        best = None
        bestkey = None
        for v in live_nodes:
            if v in removed or v in nospill:
                continue
            key = (cost[v] / float(deg[v] + 1), v)
            if bestkey is None or key < bestkey:
                bestkey = key
                best = v
        if best is None:
            # only short-lived helper registers are left; nothing more to spill
            for v in live_nodes:
                if v not in removed:
                    removed.add(v)
                    remaining -= 1
            break
        spills.append(best)
        removed.add(best)
        remaining -= 1
        for n in adj[best]:
            if n not in removed:
                deg[n] -= 1
                if deg[n] == k - 1:
                    low.append(n)
    return spills


def _rewrite_spills(f, find, slot_of, nospill):
    """Keep every spilled register in a scratch-memory word: store after each definition,
    load before each use (through a fresh short-lived register)."""
    for b in f.blocks:
        out = []
        for ins in b.ins:
            op = ins.op
            a = ins.a
            d = ins.d
            dslot = slot_of.get(find(d)) if d is not None else None
            if op == 'mov' and dslot is not None:
                s = a[0]
                sslot = slot_of.get(find(s)) if s >= 0 else None
                if sslot is None:
                    out.append(Instr('store', None, [L(dslot), s]))
                    continue
                if sslot == dslot:
                    continue
            if op == 'const' and dslot is not None:
                out.append(Instr('store', None, [L(dslot), a[0]]))
                continue
            loaded = {}
            for i in range(len(a)):
                x = a[i]
                if x >= 0:
                    sl = slot_of.get(find(x))
                    if sl is not None:
                        t = loaded.get(sl)
                        if t is None:
                            t = f.new_val()
                            nospill.add(t)
                            loaded[sl] = t
                            out.append(Instr('load', t, [L(sl)]))
                        a[i] = t
            if dslot is not None:
                t = f.new_val()
                nospill.add(t)
                ins.d = t
                out.append(ins)
                out.append(Instr('store', None, [L(dslot), t]))
            else:
                out.append(ins)
        b.ins = out
    # spilled parameters: save them at the very start
    pre = []
    for i, p in enumerate(f.params):
        sl = slot_of.get(find(p))
        if sl is not None:
            t = f.new_val()
            nospill.add(t)
            f.params[i] = t
            pre.append(Instr('store', None, [L(sl), t]))
    if pre:
        entry = f.blocks[0]
        if entry.preds:
            ne = f.new_block('entry')
            ne.ins = pre + [Instr('jmp', None, [], [entry])]
            ne.freq = 1.0
            f.blocks.insert(0, ne)
        else:
            entry.ins[0:0] = pre


SCRATCH_BASE = 3072
SCRATCH_WORDS = 1024


def allocate(f, k=None, spill=False):
    """Colour the registers of a non-SSA function with as few names as possible.
    Copies are coalesced first. With spill=True and more names than k, values are moved to
    scratch memory until k names are enough (the caller has checked that the program
    itself never touches the scratch region). Returns the number of names."""
    nospill = set()
    nslots = 0
    rounds = 0
    while True:
        nodes, adj, find, cost = _build_graph(f)
        colour, ncol = _colour(nodes, adj)
        if not spill or k is None or ncol <= k or rounds >= 8:
            break
        rounds += 1
        spills = _pick_spills(nodes, adj, cost, k, nospill)
        if not spills or nslots + len(spills) > SCRATCH_WORDS:
            break
        slot_of = {}
        for v in spills:
            slot_of[v] = SCRATCH_BASE + nslots
            nslots += 1
        _rewrite_spills(f, find, slot_of, nospill)

    def col(x):
        return colour[find(x)]

    f.params = [col(p) for p in f.params]
    for b in f.blocks:
        out = []
        for ins in b.ins:
            a = ins.a
            for i in range(len(a)):
                if a[i] >= 0:
                    a[i] = col(a[i])
            if ins.d is not None:
                ins.d = col(ins.d)
                if ins.op == 'mov':
                    if a[0] == ins.d:
                        continue
                    if a[0] < 0:
                        ins.op = 'const'
            out.append(ins)
        b.ins = out
    f.nval = ncol
    return ncol
