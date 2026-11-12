"""passes2.py - passes that need value ranges or memory reasoning, plus machine lowering."""
from ir import Instr, L, L0, L1, MASK, PURE, analyze, remove_edge
from ranges import Ranges, SMAX, FULL
from passes import use_counts


def _pow2(c):
    return c > 0 and (c & (c - 1)) == 0


def range_simplify(f, rng):
    """Use value ranges: fold results that are known constants, drop masks that do nothing,
    turn div / mod of non-negative values by powers of two into shr / and, fold branches."""
    if not rng.ok:
        return False
    rep = {}
    changed = False
    base = rng.base
    for b in f.blocks:
        out = []
        for ins in b.ins:
            d = ins.d
            op = ins.op
            a = ins.a
            if d is None:
                if op == 'br' and a[0] >= 0:
                    r = rng.get(a[0], b)
                    tk = None
                    if r[0] >= 1:
                        tk, dr = ins.t[0], ins.t[1]
                    elif r[1] == 0:
                        tk, dr = ins.t[1], ins.t[0]
                    if tk is not None:
                        if dr is not tk:
                            remove_edge(b, dr)
                        ins.op = 'jmp'
                        ins.a = []
                        ins.t = [tk]
                        changed = True
                out.append(ins)
                continue
            if op in PURE:
                r = base.get(d)
                if r is not None and r[0] == r[1]:
                    rep[d] = L(r[0])
                    changed = True
                    continue
            if op == 'and' and a[1] < 0:
                m = ~a[1]
                if _pow2(m + 1) and rng.get(a[0], b)[1] <= m:
                    rep[d] = a[0]
                    changed = True
                    continue
            elif op == 'div' or op == 'mod':
                rx = rng.get(a[0], b)
                ry = rng.get(a[1], b)
                if rx[1] <= SMAX and ry[1] <= SMAX and ry[0] >= 1:
                    if rx[1] < ry[0]:
                        rep[d] = a[0] if op == 'mod' else L0
                        changed = True
                        continue
                    if a[1] < 0 and _pow2(~a[1]):
                        c = ~a[1]
                        if op == 'div':
                            ins.op = 'shr'
                            ins.a = [a[0], L(c.bit_length() - 1)]
                        else:
                            ins.op = 'and'
                            ins.a = [a[0], L(c - 1)]
                        changed = True
            elif op == 'select':
                r = rng.get(a[0], b)
                if r[0] >= 1:
                    rep[d] = a[1]
                    changed = True
                    continue
                if r[1] == 0:
                    rep[d] = a[2]
                    changed = True
                    continue
            elif op == 'lt' or op == 'eq':
                # context-sensitive decision (the base range was not enough)
                ra = rng.get(a[0], b)
                rb = rng.get(a[1], b)
                v = None
                if op == 'lt':
                    if ra[1] <= SMAX and rb[1] <= SMAX:
                        if ra[1] < rb[0]:
                            v = 1
                        elif ra[0] >= rb[1]:
                            v = 0
                else:
                    if ra[1] < rb[0] or rb[1] < ra[0]:
                        v = 0
                if v is not None:
                    rep[d] = L(v)
                    changed = True
                    continue
            out.append(ins)
        b.ins = out
    if rep:
        def res(x):
            while x >= 0 and x in rep:
                x = rep[x]
            return x
        for b in f.blocks:
            for phi in b.phis:
                a = phi.a
                for i in range(len(a)):
                    if a[i] >= 0 and a[i] in rep:
                        a[i] = res(a[i])
            for ins in b.ins:
                a = ins.a
                for i in range(len(a)):
                    if a[i] >= 0 and a[i] in rep:
                        a[i] = res(a[i])
    return changed


# --------------------------------------------------------------------------
# Memory: redundant loads, store-to-load forwarding, redundant and dead stores
# --------------------------------------------------------------------------
class MemInfo(object):
    def __init__(self, f, rng):
        self.rng = rng
        self.defs = rng.defs

    def key(self, x):
        if x < 0:
            return (-1, ~x)
        d = self.defs.get(x)
        if d is not None and d.op == 'add' and d.a[1] < 0:
            return (d.a[0], ~d.a[1])
        return (x, 0)


def may_alias(k1, r1, k2, r2):
    if k1 == k2:
        return True
    if k1[0] == k2[0]:
        return False
    if r1[1] < r2[0] or r2[1] < r1[0]:
        return False
    return True


MAXFACTS = 64


def _mem_transfer(b, state, mi, rng, rep, apply):
    """state: dict key -> (value, range). Walk block b; with apply=True also rewrite it."""
    out = [] if apply else None
    changed = False
    for ins in b.ins:
        op = ins.op
        if op == 'load':
            k = mi.key(ins.a[0])
            e = state.get(k)
            if e is not None:
                if apply:
                    rep[ins.d] = e[0]
                    changed = True
                    continue
            else:
                if len(state) >= MAXFACTS:
                    del state[next(iter(state))]
                state[k] = (ins.d, rng.get(ins.a[0], b))
        elif op == 'store':
            k = mi.key(ins.a[0])
            e = state.get(k)
            v = ins.a[1]
            if e is not None and e[0] == v:
                if apply:
                    changed = True
                    continue
            else:
                r = rng.get(ins.a[0], b)
                dead = [k2 for k2, e2 in state.items() if k2 != k and may_alias(k, r, k2, e2[1])]
                for k2 in dead:
                    del state[k2]
                if k in state:
                    del state[k]
                elif len(state) >= MAXFACTS:
                    del state[next(iter(state))]
                state[k] = (v, r)
        if apply:
            out.append(ins)
    if apply:
        b.ins = out
    return changed


def mem_pass(f, rng):
    """Forward data-flow of 'memory word at address A holds value V' facts."""
    rpo = analyze(f)
    has_mem = False
    for b in rpo:
        for ins in b.ins:
            if ins.op == 'load' or ins.op == 'store':
                has_mem = True
                break
        if has_mem:
            break
    if not has_mem:
        return False
    mi = MemInfo(f, rng)
    outs = {}
    ins_ = {}
    entry = f.blocks[0]
    converged = False
    for _ in range(10):
        changed = False
        for b in rpo:
            if b is entry:
                st = {}
            else:
                st = None
                for p in b.preds:
                    o = outs.get(p.id)
                    if o is None:
                        continue
                    if st is None:
                        st = dict(o)
                    else:
                        for k in list(st):
                            e1 = st[k]
                            e2 = o.get(k)
                            if e2 is None or e2[0] != e1[0]:
                                del st[k]
                            elif e2[1] != e1[1]:
                                st[k] = (e1[0], (min(e1[1][0], e2[1][0]), max(e1[1][1], e2[1][1])))
                if st is None:
                    st = {}
            ins_[b.id] = dict(st)
            _mem_transfer(b, st, mi, rng, None, False)
            if outs.get(b.id) != st:
                outs[b.id] = st
                changed = True
        if not changed:
            converged = True
            break
    rep = {}
    changed = False
    for b in rpo:
        st = dict(ins_[b.id]) if converged else {}
        if _mem_transfer(b, st, mi, rng, rep, True):
            changed = True
    if rep:
        def res(x):
            while x >= 0 and x in rep:
                x = rep[x]
            return x
        for b in f.blocks:
            for phi in b.phis:
                a = phi.a
                for i in range(len(a)):
                    if a[i] >= 0 and a[i] in rep:
                        a[i] = res(a[i])
            for ins in b.ins:
                a = ins.a
                for i in range(len(a)):
                    if a[i] >= 0 and a[i] in rep:
                        a[i] = res(a[i])
    if dse(f, rng, mi):
        changed = True
    return changed


def dse(f, rng, mi):
    """Remove a store that is overwritten later in the same block before anyone can see it."""
    changed = False
    for b in f.blocks:
        pending = {}     # key -> [range, clean]
        out = []
        for ins in reversed(b.ins):
            op = ins.op
            if op == 'store':
                k = mi.key(ins.a[0])
                e = pending.get(k)
                trap = rng.may_trap(ins, b)
                if e is not None and (e[1] or not trap):
                    changed = True
                    continue
                if trap:
                    pending.clear()
                else:
                    for e2 in pending.values():
                        e2[1] = False
                pending[k] = [rng.get(ins.a[0], b), True]
            elif op == 'load':
                if rng.may_trap(ins, b):
                    pending.clear()
                else:
                    k = mi.key(ins.a[0])
                    r = rng.get(ins.a[0], b)
                    for k2 in [k2 for k2, e2 in pending.items() if may_alias(k, r, k2, e2[0])]:
                        del pending[k2]
            elif op == 'vload' or op == 'vstore':
                for e2 in pending.values():
                    e2[1] = False
            elif op == 'div' or op == 'mod':
                if rng.may_trap(ins, b):
                    pending.clear()
            elif op == 'trap':
                pending.clear()
            out.append(ins)
        out.reverse()
        b.ins = out
    return changed


# --------------------------------------------------------------------------
# Loop-invariant code motion
# --------------------------------------------------------------------------
def get_preheader(f, lp):
    """Return a block that is the only way into the loop from outside and that only jumps
    to the header. Creates one if needed. Returns None if the loop cannot be entered."""
    h = lp.header
    outside = [p for p in h.preds if p.id not in lp.ids]
    if not outside:
        return None
    if len(outside) == 1 and outside[0].ins[-1].op == 'jmp':
        return outside[0]
    n = f.new_block('pre')
    idxs = [i for i, p in enumerate(h.preds) if p.id not in lp.ids]
    newargs = []
    for phi in h.phis:
        args = [phi.a[i] for i in idxs]
        same = True
        for x in args:
            if x != args[0]:
                same = False
        if same:
            newargs.append(args[0])
        else:
            v = f.new_val()
            n.phis.append(Instr('phi', v, args))
            newargs.append(v)
    n.preds = list(outside)
    n.ins = [Instr('jmp', None, [], [h])]
    for p in outside:
        tt = p.ins[-1].t
        for i in range(len(tt)):
            if tt[i] is h:
                tt[i] = n
    for i in reversed(idxs):
        del h.preds[i]
        for phi in h.phis:
            del phi.a[i]
    h.preds.append(n)
    for phi, v in zip(h.phis, newargs):
        phi.a.append(v)
    f.blocks.insert(f.blocks.index(h), n)
    n.idom = None
    n.depth = max(0, h.depth - 1)
    return n


def licm(f, rng):
    """Move computations that give the same result in every round to the loop preheader.
    Only instructions that cannot trap are moved (a load also needs: no store in the loop
    may write its address)."""
    analyze(f)
    loops = sorted(f.loops, key=lambda lp: (-lp.depth, lp.header.rpo))
    if not loops:
        return False
    mi = MemInfo(f, rng)
    defblk = {}
    for b in f.blocks:
        for phi in b.phis:
            defblk[phi.d] = b
        for ins in b.ins:
            if ins.d is not None:
                defblk[ins.d] = b
    changed = False
    for lp in loops:
        cand = False
        for b in lp.blocks:
            for ins in b.ins:
                if ins.d is None:
                    continue
                ok = True
                for x in ins.a:
                    if x >= 0:
                        db = defblk.get(x)
                        if db is not None and db.id in lp.ids:
                            ok = False
                            break
                if ok and ins.op != 'vload':
                    cand = True
                    break
            if cand:
                break
        if not cand:
            continue
        pre = get_preheader(f, lp)
        if pre is None:
            continue
        par = lp.parent
        while par is not None:
            if pre.id not in par.ids:
                par.ids.add(pre.id)
                par.blocks.insert(par.blocks.index(lp.header), pre)
            par = par.parent
        stores = None
        moved = []
        for b in lp.blocks:
            out = []
            for ins in b.ins:
                d = ins.d
                op = ins.op
                if d is None or op == 'vload':
                    out.append(ins)
                    continue
                inv = True
                for x in ins.a:
                    if x >= 0:
                        db = defblk.get(x)
                        if db is not None and db.id in lp.ids:
                            inv = False
                            break
                if not inv:
                    out.append(ins)
                    continue
                if op in PURE:
                    pass
                elif op == 'div' or op == 'mod':
                    if rng.may_trap(ins, pre):
                        out.append(ins)
                        continue
                elif op == 'load':
                    if rng.may_trap(ins, pre):
                        out.append(ins)
                        continue
                    if stores is None:
                        stores = []
                        for bb in lp.blocks:
                            for i2 in bb.ins:
                                if i2.op == 'store':
                                    stores.append((mi.key(i2.a[0]), rng.get(i2.a[0], bb)))
                    k = mi.key(ins.a[0])
                    r = rng.get(ins.a[0], pre)
                    hit = False
                    for k2, r2 in stores:
                        if may_alias(k, r, k2, r2):
                            hit = True
                            break
                    if hit:
                        out.append(ins)
                        continue
                else:
                    out.append(ins)
                    continue
                moved.append(ins)
                defblk[d] = pre
            b.ins = out
        if moved:
            pre.ins[-1:-1] = moved
            changed = True
    return changed


# --------------------------------------------------------------------------
# Lowering: choose machine instructions (fused ops, shifts instead of multiplies)
# --------------------------------------------------------------------------
def lower(f, mc, use_mac=True):
    """Run last on SSA form. Picks mac / addsh when the machine has them and turns
    multiplications by powers of two into shifts when that is not slower."""
    alu, lmul = mc['lat_alu'], mc['lat_mul']
    has_mac = mc['fused_mac'] and use_mac and mc['lat_mac'] <= lmul + alu
    has_addsh = bool(mc['fused_addsh'])
    lash = mc['lat_addsh']
    shift_lat = alu if alu <= lmul else lmul
    uc = use_counts(f)
    defs = {}
    defblk = {}
    for b in f.blocks:
        for ins in b.ins:
            if ins.d is not None:
                defs[ins.d] = ins
                defblk[ins.d] = b
    dead = set()
    for b in f.blocks:
        for ins in b.ins:
            if ins.op != 'add':
                continue
            x, y = ins.a
            done = False
            for (p, q) in ((x, y), (y, x)):
                # p = the multiply, q = the other operand
                if p < 0 or uc.get(p, 0) != 1:
                    continue
                dp = defs.get(p)
                if dp is None or dp.op != 'mul' or dp.d in dead or defblk[p] is not b:
                    continue
                c = ~dp.a[1] if dp.a[1] < 0 else None
                if c is not None and _pow2(c) and has_addsh and lash <= shift_lat + alu:
                    ins.op = 'addsh'
                    ins.a = [q, dp.a[0], L(c.bit_length() - 1)]
                    dead.add(dp.d)
                    done = True
                    break
                if has_mac and not (c is not None and _pow2(c) and alu < lmul):
                    ins.op = 'mac'
                    ins.a = [dp.a[0], dp.a[1], q]
                    dead.add(dp.d)
                    done = True
                    break
            if done:
                continue
    for b in f.blocks:
        out = []
        for ins in b.ins:
            if ins.d is not None and ins.d in dead:
                continue
            if ins.op == 'mul' and ins.a[1] < 0:
                c = ~ins.a[1]
                if _pow2(c) and alu <= lmul:
                    ins.op = 'shl'
                    ins.a = [ins.a[0], L(c.bit_length() - 1)]
                elif c > 2 and _pow2(c - 1) and has_addsh and lash < lmul:
                    ins.op = 'addsh'
                    ins.a = [ins.a[0], ins.a[0], L((c - 1).bit_length() - 1)]
            out.append(ins)
        b.ins = out
