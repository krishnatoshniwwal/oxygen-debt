"""passes.py - machine-independent optimizations on SSA form."""
from ir import (Instr, L, L0, L1, MASK, SIGN, COMM, PURE, BINOPS, analyze, remove_edge,
                remove_unreachable, dominates)


def sgn(x):
    return x - 0x100000000 if x & SIGN else x


def evalop(op, x, y):
    """Exact KIR semantics for a binary op on two 32-bit values. None if it would trap."""
    if op == 'add':
        return (x + y) & MASK
    if op == 'sub':
        return (x - y) & MASK
    if op == 'mul':
        return (x * y) & MASK
    if op == 'and':
        return x & y
    if op == 'or':
        return x | y
    if op == 'xor':
        return x ^ y
    if op == 'shl':
        return (x << (y & 31)) & MASK
    if op == 'shr':
        return x >> (y & 31)
    if op == 'lt':
        return 1 if (x ^ SIGN) < (y ^ SIGN) else 0
    if op == 'eq':
        return 1 if x == y else 0
    if op == 'div' or op == 'mod':
        if y == 0:
            return None
        sx, sy = sgn(x), sgn(y)
        if op == 'div':
            q = abs(sx) // abs(sy)
            if (sx < 0) != (sy < 0):
                q = -q
            return q & MASK
        r = abs(sx) % abs(sy)
        if sx < 0:
            r = -r
        return r & MASK
    raise AssertionError(op)


def simplify(ins, defs):
    """Simplify one SSA instruction (operands already resolved).
    Returns an operand that replaces the result, or None (the instruction may have been
    rewritten in place)."""
    while True:
        op = ins.op
        a = ins.a
        if op in BINOPS:
            x, y = a
            if x < 0 and y < 0:
                r = evalop(op, ~x, ~y)
                if r is None:
                    return None
                return L(r)
            if x < 0 and op in COMM:
                x, y = y, x
                a[0], a[1] = x, y
            dx = defs.get(x) if x >= 0 else None
            dy = defs.get(y) if y >= 0 else None
            if op == 'add':
                if y < 0:
                    c = ~y
                    if c == 0:
                        return x
                    if dx is not None and dx.op == 'add' and dx.a[1] < 0:
                        ins.a = [dx.a[0], L(c + ~dx.a[1])]
                        continue
                    return None
                if x == y:
                    ins.op = 'mul'
                    ins.a = [x, L(2)]
                    continue
                if dx is not None and dx.op == 'sub' and dx.a[1] == y:
                    return dx.a[0]
                if dy is not None and dy.op == 'sub' and dy.a[1] == x:
                    return dy.a[0]
                mx = dx is not None and dx.op == 'mul' and dx.a[1] < 0
                my = dy is not None and dy.op == 'mul' and dy.a[1] < 0
                if mx and my and dx.a[0] == dy.a[0]:
                    ins.op = 'mul'
                    ins.a = [dx.a[0], L(~dx.a[1] + ~dy.a[1])]
                    continue
                if mx and dx.a[0] == y:
                    ins.op = 'mul'
                    ins.a = [y, L(~dx.a[1] + 1)]
                    continue
                if my and dy.a[0] == x:
                    ins.op = 'mul'
                    ins.a = [x, L(~dy.a[1] + 1)]
                    continue
                return None
            if op == 'sub':
                if y < 0:
                    ins.op = 'add'
                    ins.a = [x, L(-(~y))]
                    continue
                if x == y:
                    return L0
                if dx is not None and dx.op == 'add':
                    if dx.a[1] == y:
                        return dx.a[0]
                    if dx.a[0] == y:
                        return dx.a[1]
                mx = dx is not None and dx.op == 'mul' and dx.a[1] < 0
                my = dy is not None and dy.op == 'mul' and dy.a[1] < 0
                if mx and my and dx.a[0] == dy.a[0]:
                    ins.op = 'mul'
                    ins.a = [dx.a[0], L(~dx.a[1] - ~dy.a[1])]
                    continue
                if mx and dx.a[0] == y:
                    ins.op = 'mul'
                    ins.a = [y, L(~dx.a[1] - 1)]
                    continue
                return None
            if op == 'mul':
                if y < 0:
                    c = ~y
                    if c == 0:
                        return L0
                    if c == 1:
                        return x
                    if dx is not None and dx.op == 'mul' and dx.a[1] < 0:
                        ins.a = [dx.a[0], L(c * ~dx.a[1])]
                        continue
                return None
            if op == 'and':
                if y < 0:
                    c = ~y
                    if c == 0:
                        return L0
                    if c == MASK:
                        return x
                    if dx is not None and dx.op == 'and' and dx.a[1] < 0:
                        ins.a = [dx.a[0], L(c & ~dx.a[1])]
                        continue
                    return None
                if x == y:
                    return x
                return None
            if op == 'or':
                if y < 0:
                    c = ~y
                    if c == 0:
                        return x
                    if c == MASK:
                        return L(MASK)
                    if dx is not None and dx.op == 'or' and dx.a[1] < 0:
                        ins.a = [dx.a[0], L(c | ~dx.a[1])]
                        continue
                    return None
                if x == y:
                    return x
                return None
            if op == 'xor':
                if y < 0:
                    c = ~y
                    if c == 0:
                        return x
                    if dx is not None and dx.op == 'xor' and dx.a[1] < 0:
                        ins.a = [dx.a[0], L(c ^ ~dx.a[1])]
                        continue
                    return None
                if x == y:
                    return L0
                return None
            if op == 'shl':
                if x == L0:
                    return L0
                if y < 0:
                    k = ~y & 31
                    if k == 0:
                        return x
                    ins.op = 'mul'
                    ins.a = [x, L(1 << k)]
                    continue
                return None
            if op == 'shr':
                if x == L0:
                    return L0
                if y < 0:
                    k = ~y & 31
                    if k == 0:
                        return x
                    if dx is not None and dx.op == 'shr' and dx.a[1] < 0:
                        k2 = k + (~dx.a[1] & 31)
                        if k2 >= 32:
                            return L0
                        ins.a = [dx.a[0], L(k2)]
                        continue
                    if ~y != k:
                        a[1] = L(k)
                return None
            if op == 'lt':
                if x == y:
                    return L0
                return None
            if op == 'eq':
                if x == y:
                    return L1
                if y < 0 and dx is not None:
                    c = ~y
                    if dx.op == 'add' and dx.a[1] < 0:
                        ins.a = [dx.a[0], L(c - ~dx.a[1])]
                        continue
                    if dx.op == 'xor' and dx.a[1] < 0:
                        ins.a = [dx.a[0], L(c ^ ~dx.a[1])]
                        continue
                    if c == 0 and dx.op in ('sub', 'xor'):
                        ins.a = [dx.a[0], dx.a[1]]
                        continue
                return None
            if op == 'div':
                if y == L1:
                    return x
                return None
            if op == 'mod':
                if y == L1:
                    return L0
                return None
            return None
        if op == 'select':
            c, p, q = a
            if c < 0:
                return p if ~c != 0 else q
            if p == q:
                return p
            dc = defs.get(c)
            if dc is not None:
                if dc.op == 'eq' and dc.a[1] == L0:
                    ins.a = [dc.a[0], q, p]
                    continue
                if p == L1 and q == L0 and dc.op in ('lt', 'eq'):
                    return c
            return None
        return None


GVN_OPS = set(PURE) | set(['div', 'mod'])


def scalar_pass(f):
    """Simplification, constant folding, copy propagation, value numbering and branch
    folding in one walk over the dominator tree. Returns True if anything changed."""
    analyze(f)
    rep = {}
    defs = {}
    changed = False

    def res(x):
        while x >= 0:
            y = rep.get(x)
            if y is None:
                return x
            x = y
        return x

    table = {}
    stack = [(f.blocks[0], None)]
    while stack:
        b, undo = stack.pop()
        if undo is not None:
            for key in undo:
                del table[key]
            continue
        undo = []
        # phis
        if b.phis:
            keep = []
            for phi in b.phis:
                a = phi.a
                same = None
                triv = True
                for i in range(len(a)):
                    x = a[i]
                    if x >= 0 and x in rep:
                        x = res(x)
                        a[i] = x
                    if x == phi.d:
                        continue
                    if same is None:
                        same = x
                    elif same != x:
                        triv = False
                if triv:
                    rep[phi.d] = same if same is not None else L0
                    changed = True
                    continue
                key = ('phi', b.id) + tuple(a)
                o = table.get(key)
                if o is not None:
                    rep[phi.d] = o
                    changed = True
                    continue
                table[key] = phi.d
                undo.append(key)
                defs[phi.d] = phi
                keep.append(phi)
            b.phis = keep
        out = []
        for ins in b.ins:
            a = ins.a
            for i in range(len(a)):
                x = a[i]
                if x >= 0 and x in rep:
                    a[i] = res(x)
            d = ins.d
            op = ins.op
            if d is None:
                if op == 'br':
                    c = ins.a[0]
                    while c >= 0:
                        dc = defs.get(c)
                        if dc is not None and dc.op == 'eq' and dc.a[1] == L0:
                            c = dc.a[0]
                            ins.a[0] = c
                            ins.t.reverse()
                            changed = True
                        else:
                            break
                    if c < 0:
                        tk = ins.t[0] if ~c != 0 else ins.t[1]
                        dr = ins.t[1] if ~c != 0 else ins.t[0]
                        if dr is not tk:
                            remove_edge(b, dr)
                        ins.op = 'jmp'
                        ins.a = []
                        ins.t = [tk]
                        changed = True
                out.append(ins)
                continue
            if op in GVN_OPS:
                before = (op, a[0], a[1])
                r = simplify(ins, defs)
                if r is not None:
                    rep[d] = r
                    changed = True
                    continue
                op = ins.op
                a = ins.a
                if (op, a[0], a[1]) != before:
                    changed = True
                if op in COMM and a[0] > a[1] and a[1] >= 0:
                    a[0], a[1] = a[1], a[0]
                key = (op,) + tuple(a)
                o = table.get(key)
                if o is not None:
                    rep[d] = o
                    changed = True
                    continue
                table[key] = d
                undo.append(key)
            defs[d] = ins
            out.append(ins)
        b.ins = out
        stack.append((b, undo))
        for c in reversed(b.dkids):
            stack.append((c, None))
    if rep:
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


def cleanup_cfg(f):
    """CFG simplification on SSA form: remove unreachable blocks, forward empty blocks,
    merge a block with its only successor when that successor has no other predecessor."""
    changed = False
    again = True
    while again:
        again = False
        if remove_unreachable(f):
            changed = True
        entry = f.blocks[0]
        # forward blocks that only contain 'jmp S'
        for e in f.blocks:
            if e is entry or e.phis or len(e.ins) != 1 or e.ins[0].op != 'jmp':
                continue
            s = e.ins[0].t[0]
            if s is e:
                continue
            ok = True
            if s.phis:
                for p in e.preds:
                    if p in s.preds:
                        ok = False
                        break
            if not ok or not e.preds:
                continue
            ei = s.preds.index(e)
            preds = list(e.preds)
            first = True
            for p in preds:
                tt = p.ins[-1].t
                for i in range(len(tt)):
                    if tt[i] is e:
                        tt[i] = s
                if p in s.preds:
                    # no phis in s (checked above)
                    pass
                elif first:
                    s.preds[ei] = p
                    first = False
                else:
                    s.preds.append(p)
                    for phi in s.phis:
                        phi.a.append(phi.a[ei])
                term = p.ins[-1]
                if term.op == 'br' and term.t[0] is term.t[1]:
                    term.op = 'jmp'
                    term.a = []
                    term.t = [term.t[0]]
            if first:
                # every predecessor already reached s directly
                del s.preds[ei]
            e.preds = []
            again = True
            changed = True
        if again:
            continue
        # merge b -> s when s has only b as predecessor
        dead = set()
        for b in f.blocks:
            if b.id in dead:
                continue
            while True:
                term = b.ins[-1]
                if term.op != 'jmp':
                    break
                s = term.t[0]
                if s is b or s is entry or len(s.preds) != 1:
                    break
                if s.phis:
                    # single predecessor: each phi is a copy of its only operand
                    break
                b.ins.pop()
                b.ins.extend(s.ins)
                for ss in s.succs():
                    i = ss.preds.index(s)
                    ss.preds[i] = b
                dead.add(s.id)
                s.ins = [Instr('trap', None, [L0])]
                s.preds = []
                changed = True
        if dead:
            f.blocks = [b for b in f.blocks if b.id not in dead]
            again = True
    return changed


def use_counts(f):
    uc = {}
    for b in f.blocks:
        for phi in b.phis:
            for x in phi.a:
                if x >= 0:
                    uc[x] = uc.get(x, 0) + 1
        for ins in b.ins:
            for x in ins.a:
                if x >= 0:
                    uc[x] = uc.get(x, 0) + 1
    return uc


def dce(f, rng=None):
    """Remove instructions whose result is never used and that have no visible effect.
    A load / div / mod stays unless `rng` proves that it cannot trap."""
    defs = {}
    live = set()
    work = []
    for b in f.blocks:
        for phi in b.phis:
            defs[phi.d] = phi
        for ins in b.ins:
            d = ins.d
            op = ins.op
            if d is not None:
                defs[d] = ins
            root = False
            if d is None or op == 'vload':
                root = True
            elif op == 'load' or op == 'div' or op == 'mod':
                root = rng is None or rng.may_trap(ins, b)
            if root:
                for x in ins.a:
                    if x >= 0 and x not in live:
                        live.add(x)
                        work.append(x)
    while work:
        v = work.pop()
        ins = defs.get(v)
        if ins is None:
            continue
        for x in ins.a:
            if x >= 0 and x not in live:
                live.add(x)
                work.append(x)
    changed = False
    for b in f.blocks:
        if b.phis:
            n = len(b.phis)
            b.phis = [p for p in b.phis if p.d in live]
            if len(b.phis) != n:
                changed = True
        out = []
        for ins in b.ins:
            d = ins.d
            op = ins.op
            if d is not None and d not in live:
                if op == 'vload':
                    out.append(ins)
                    continue
                if op in ('load', 'div', 'mod') and (rng is None or rng.may_trap(ins, b)):
                    out.append(ins)
                    continue
                changed = True
                continue
            out.append(ins)
        b.ins = out
    return changed
