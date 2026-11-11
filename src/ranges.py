"""ranges.py - value-range analysis on SSA form.

Every value gets an unsigned interval [lo, hi] that is true on every run. Branch
conditions refine the interval inside the blocks that the branch guards. The result is
used to prove that a load / store / div / mod cannot trap, that a value is not negative,
and to fold comparisons.
"""
from ir import MASK, analyze

SMAX = 0x7FFFFFFF
FULL = (0, MASK)
TWO32 = 1 << 32


def _bitcap(x):
    return (1 << x.bit_length()) - 1


class Ranges(object):
    def __init__(self, f, max_passes=14):
        self.f = f
        self.base = {}
        self.cons = {}
        self.cup = {}
        self.defs = {}
        self.defblk = {}
        self.ok = False
        self._edge_cache = {}
        self._compute(max_passes)

    # ------------------------------------------------------------------
    def at(self, x, b):
        """Interval of operand x when it is used in block b (None = not computed yet)."""
        if x < 0:
            return (~x, ~x)
        r = self.base.get(x)
        if r is None:
            return None
        blk = self.cup.get(b.id)
        if blk is None:
            return r
        db = self.defblk.get(x)
        lo, hi = r
        n = 0
        cons = self.cons
        cup = self.cup
        while blk is not None and blk is not db and n < 24:
            c = cons[blk.id].get(x)
            if c is not None:
                l2 = c[0] if c[0] > lo else lo
                h2 = c[1] if c[1] < hi else hi
                if l2 <= h2:
                    lo, hi = l2, h2
            p = blk.idom
            if p is blk:
                break
            blk = cup.get(p.id)
            n += 1
        return (lo, hi)

    def get(self, x, b):
        r = self.at(x, b)
        return r if r is not None else FULL

    def may_trap(self, ins, b):
        op = ins.op
        if op == 'load' or op == 'store':
            if not self.ok:
                return True
            return self.get(ins.a[0], b)[1] >= 4096
        if op == 'div' or op == 'mod':
            y = ins.a[1]
            if y < 0:
                return ~y == 0
            if not self.ok:
                return True
            return self.get(y, b)[0] < 1
        return op == 'trap'

    # ------------------------------------------------------------------
    def _derive(self, p, c, pol):
        """Constraints that hold when the branch on c in block p goes the `pol` way."""
        out = {}
        if c < 0:
            return out
        rc = self.at(c, p)
        if rc is not None:
            if pol:
                if rc[0] == 0 and rc[1] >= 1:
                    out[c] = (1, rc[1])
            elif rc[0] == 0:
                out[c] = (0, 0)
        dc = self.defs.get(c)
        if dc is None:
            return out
        op = dc.op
        if op != 'lt' and op != 'eq':
            return out
        a, b = dc.a
        ra = self.at(a, p)
        rb = self.at(b, p)
        if ra is None or rb is None:
            return out
        na = nb = None
        if op == 'lt':
            if pol:
                if ra[1] <= SMAX:
                    # 0 <= a < b (signed), so b is positive
                    nb = (max(rb[0], ra[0] + 1), min(rb[1], SMAX))
                    na = (ra[0], min(ra[1], nb[1] - 1))
            else:
                if rb[1] <= SMAX:
                    # a >= b >= 0 (signed)
                    na = (max(ra[0], rb[0]), min(ra[1], SMAX))
                    if ra[1] <= SMAX:
                        nb = (rb[0], min(rb[1], ra[1]))
        else:
            if pol:
                lo = max(ra[0], rb[0])
                hi = min(ra[1], rb[1])
                na = nb = (lo, hi)
            else:
                if rb[0] == rb[1]:
                    if ra[0] == rb[0]:
                        na = (ra[0] + 1, ra[1])
                    elif ra[1] == rb[0]:
                        na = (ra[0], ra[1] - 1)
                if ra[0] == ra[1]:
                    if rb[0] == ra[0]:
                        nb = (rb[0] + 1, rb[1])
                    elif rb[1] == ra[0]:
                        nb = (rb[0], rb[1] - 1)
        if na is not None and a >= 0 and na[0] <= na[1]:
            out[a] = na
        if nb is not None and b >= 0 and nb[0] <= nb[1]:
            out[b] = nb
        return out

    def _xfer(self, ins, b):
        op = ins.op
        a = ins.a
        if op == 'load' or op == 'vload':
            return FULL
        if op == 'select':
            rc = self.at(a[0], b)
            rp = self.at(a[1], b)
            rq = self.at(a[2], b)
            if rc is None:
                return None
            if rc[0] >= 1:
                return rp
            if rc[1] == 0:
                return rq
            if rp is None:
                return rq
            if rq is None:
                return rp
            return (min(rp[0], rq[0]), max(rp[1], rq[1]))
        ra = self.at(a[0], b)
        rb = self.at(a[1], b)
        if ra is None or rb is None:
            return None
        la, ha = ra
        lb, hb = rb
        if op == 'add':
            lo, hi = la + lb, ha + hb
            if hi <= MASK:
                return (lo, hi)
            if lo > MASK:
                return (lo - TWO32, hi - TWO32)
            return FULL
        if op == 'sub':
            lo, hi = la - hb, ha - lb
            if lo >= 0:
                return (lo, hi)
            if hi < 0:
                return (lo + TWO32, hi + TWO32)
            return FULL
        if op == 'mul':
            if ha * hb <= MASK:
                return (la * lb, ha * hb)
            return FULL
        if op == 'and':
            return (0, ha if ha < hb else hb)
        if op == 'or':
            return (la if la > lb else lb, _bitcap(ha if ha > hb else hb))
        if op == 'xor':
            return (0, _bitcap(ha if ha > hb else hb))
        if op == 'shl':
            if lb == hb:
                k = lb & 31
                if (ha << k) <= MASK:
                    return (la << k, ha << k)
            return FULL
        if op == 'shr':
            if lb == hb:
                k = lb & 31
                return (la >> k, ha >> k)
            return (0, ha)
        if op == 'lt':
            if ha <= SMAX and hb <= SMAX:
                if ha < lb:
                    return (1, 1)
                if la >= hb:
                    return (0, 0)
            return (0, 1)
        if op == 'eq':
            if ha < lb or hb < la:
                return (0, 0)
            if la == ha and lb == hb and la == lb:
                return (1, 1)
            return (0, 1)
        if op == 'div':
            if ha <= SMAX and hb <= SMAX and lb >= 1:
                return (la // hb, ha // lb)
            return FULL
        if op == 'mod':
            if ha <= SMAX:
                if hb <= SMAX and hb >= 1:
                    return (0, min(ha, hb - 1))
                return (0, ha)
            return FULL
        return FULL

    def _edge(self, p, s):
        """Constraints on the CFG edge p -> s (cached per pass)."""
        term = p.ins[-1]
        if term.op != 'br' or term.t[0] is term.t[1]:
            return None
        key = (p.id, s.id)
        r = self._edge_cache.get(key)
        if r is None:
            r = self._derive(p, term.a[0], term.t[0] is s)
            self._edge_cache[key] = r
        return r

    def _phi(self, phi, b):
        lo = hi = None
        for x, p in zip(phi.a, b.preds):
            r = self.at(x, p)
            if r is None:
                continue
            if x >= 0:
                ec = self._edge(p, b)
                if ec:
                    c = ec.get(x)
                    if c is not None:
                        l2 = max(r[0], c[0])
                        h2 = min(r[1], c[1])
                        if l2 <= h2:
                            r = (l2, h2)
            if lo is None:
                lo, hi = r
            else:
                if r[0] < lo:
                    lo = r[0]
                if r[1] > hi:
                    hi = r[1]
        if lo is None:
            return None
        return (lo, hi)

    def _walk(self, narrowing, counts):
        f = self.f
        base = self.base
        changed = False
        self._edge_cache = {}
        for b in self.order:
            # constraints on entry to b
            cs = {}
            if len(b.preds) == 1 and b is not f.blocks[0]:
                ec = self._edge(b.preds[0], b)
                if ec:
                    cs = ec
            self.cons[b.id] = cs
            if cs:
                self.cup[b.id] = b
            elif b.idom is not b:
                self.cup[b.id] = self.cup.get(b.idom.id)
            else:
                self.cup[b.id] = None
            for phi in b.phis:
                new = self._phi(phi, b)
                if self._update(phi.d, new, narrowing, counts):
                    changed = True
            for ins in b.ins:
                d = ins.d
                if d is None:
                    continue
                new = self._xfer(ins, b)
                if self._update(d, new, narrowing, counts):
                    changed = True
        return changed

    def _update(self, d, new, narrowing, counts):
        base = self.base
        old = base.get(d)
        if new is None:
            return False
        if narrowing:
            if old is None:
                return False
            lo = new[0] if new[0] > old[0] else old[0]
            hi = new[1] if new[1] < old[1] else old[1]
            if lo > hi or (lo, hi) == old:
                return False
            base[d] = (lo, hi)
            return True
        if old is None:
            base[d] = new
            return True
        if new[0] >= old[0] and new[1] <= old[1]:
            return False
        lo = new[0] if new[0] < old[0] else old[0]
        hi = new[1] if new[1] > old[1] else old[1]
        n = counts.get(d, 0) + 1
        counts[d] = n
        if n >= 3:
            if lo < old[0]:
                lo = 0
            if hi > old[1]:
                hi = SMAX if hi <= SMAX else MASK
        base[d] = (lo, hi)
        return True

    def _compute(self, max_passes):
        f = self.f
        analyze(f)
        # dominator-tree preorder
        order = []
        stack = [f.blocks[0]]
        while stack:
            b = stack.pop()
            order.append(b)
            for c in reversed(b.dkids):
                stack.append(c)
        self.order = order
        for p in f.params:
            self.base[p] = FULL
            self.defblk[p] = None
        for b in order:
            for phi in b.phis:
                self.defs[phi.d] = phi
                self.defblk[phi.d] = b
            for ins in b.ins:
                if ins.d is not None:
                    self.defs[ins.d] = ins
                    self.defblk[ins.d] = b
        counts = {}
        stable = False
        for _ in range(max_passes):
            if not self._walk(False, counts):
                stable = True
                break
        if not stable:
            # give up: no information
            self.base = {}
            self.cons = dict((b.id, {}) for b in order)
            self.cup = {}
            self.ok = False
            return
        for b in order:
            for phi in b.phis:
                if phi.d not in self.base:
                    self.base[phi.d] = FULL
            for ins in b.ins:
                if ins.d is not None and ins.d not in self.base:
                    self.base[ins.d] = FULL
        for _ in range(2):
            if not self._walk(True, counts):
                break
        self.ok = True
