"""ir.py - the in-memory form of a KIR function used by kopt.

Operands are plain ints:
  x >= 0  : a register (or, in SSA form, a value number)
  x <  0  : the literal ~x (0 .. 2^32-1)
"""
import kirlib as K

MASK = 0xFFFFFFFF
SIGN = 0x80000000


def L(v):
    """Make a literal operand."""
    return ~(v & MASK)


L0 = L(0)
L1 = L(1)

TERMS = ('jmp', 'br', 'ret', 'trap')
BINOPS = ('add', 'sub', 'mul', 'and', 'or', 'xor', 'shl', 'shr', 'lt', 'eq', 'div', 'mod')
COMM = ('add', 'mul', 'and', 'or', 'xor', 'eq')
# ops with a destination and no side effect other than (for div/mod/load) a possible trap
PURE = ('add', 'sub', 'mul', 'and', 'or', 'xor', 'shl', 'shr', 'lt', 'eq', 'select', 'mac', 'addsh')


class Instr(object):
    __slots__ = ('op', 'd', 'a', 't')

    def __init__(self, op, d=None, a=None, t=None):
        self.op = op
        self.d = d          # destination register / value, or None
        self.a = a if a is not None else []   # operands
        self.t = t          # jump targets (list of Block) for jmp / br

    def copy(self):
        return Instr(self.op, self.d, list(self.a), list(self.t) if self.t else self.t)


class Block(object):
    __slots__ = ('id', 'name', 'phis', 'ins', 'preds', 'idom', 'dkids', 'rpo', 'pre', 'post',
                 'depth', 'freq', 'loop')

    def __init__(self, bid, name):
        self.id = bid
        self.name = name
        self.phis = []
        self.ins = []
        self.preds = []
        self.idom = None
        self.dkids = []
        self.rpo = -1
        self.pre = 0
        self.post = 0
        self.depth = 0
        self.freq = 1.0
        self.loop = None

    def succs(self):
        t = self.ins[-1].t
        return t if t else []


class Func(object):
    def __init__(self):
        self.name = ''
        self.params = []
        self.blocks = []
        self.nval = 0
        self.nbid = 0
        self.names = set()

    def new_val(self):
        v = self.nval
        self.nval += 1
        return v

    def new_block(self, hint='k'):
        i = self.nbid
        name = '%s_%d' % (hint, i)
        while name in self.names:
            i += 1
            name = '%s_%d' % (hint, i)
        b = Block(self.nbid, name)
        self.nbid += 1
        self.names.add(name)
        return b


def from_program(prog):
    """Convert a parsed kirlib Program into a Func (not SSA)."""
    f = Func()
    f.name = prog.name
    f.params = list(prog.params)
    byname = {}
    for name in prog.block_labels:
        b = Block(f.nbid, name)
        f.nbid += 1
        f.names.add(name)
        f.blocks.append(b)
        byname[name] = b
    maxreg = -1
    for p in prog.params:
        if p > maxreg:
            maxreg = p
    block_of = prog.block_of
    for i, pi in enumerate(prog.instrs):
        b = byname[block_of[i]]
        op = pi.op
        args = pi.args
        d = None
        a = []
        t = None
        sig = K.SIGS[op]
        for kind, (ak, av) in zip(sig, args):
            if kind == 'd':
                d = av
                if av > maxreg:
                    maxreg = av
            elif kind == 'L':
                if t is None:
                    t = []
                t.append(byname[av])
            elif ak == 'r':
                a.append(av)
                if av > maxreg:
                    maxreg = av
            else:
                a.append(~av)
        if op == 'br' and t[0] is t[1]:
            op = 'jmp'
            a = []
            t = [t[0]]
        b.ins.append(Instr(op, d, a, t))
    f.nval = maxreg + 1
    compute_preds(f)
    return f


def fmt_operand(x):
    return ('v%d' % x) if x >= 0 else str(~x)


def to_text(f):
    out = ['func %s(%s)' % (f.name, ', '.join('v%d' % p for p in f.params))]
    for b in f.blocks:
        out.append('%s:' % b.name)
        for ins in b.ins:
            op = ins.op
            parts = []
            if ins.d is not None:
                parts.append('v%d' % ins.d)
            for x in ins.a:
                parts.append(('v%d' % x) if x >= 0 else str(~x))
            if ins.t:
                for tb in ins.t:
                    parts.append(tb.name)
            out.append('  %s %s' % (op, ', '.join(parts)))
    return '\n'.join(out) + '\n'


def count_instrs(f):
    n = 0
    for b in f.blocks:
        n += len(b.ins)
    return n


# --------------------------------------------------------------------------
# CFG utilities
# --------------------------------------------------------------------------
def compute_preds(f):
    """Recompute preds from terminators (only valid when there are no phis)."""
    for b in f.blocks:
        b.preds = []
    for b in f.blocks:
        for s in b.succs():
            if b not in s.preds:
                s.preds.append(b)


def remove_edge(p, s):
    """Remove p from s.preds and drop the matching phi operands (the terminator is the caller's job)."""
    i = s.preds.index(p)
    del s.preds[i]
    for phi in s.phis:
        del phi.a[i]


def remove_unreachable(f):
    """Delete blocks that cannot be reached from the entry. Keeps phis consistent. Returns True if changed."""
    entry = f.blocks[0]
    seen = set([entry.id])
    stack = [entry]
    while stack:
        b = stack.pop()
        for s in b.succs():
            if s.id not in seen:
                seen.add(s.id)
                stack.append(s)
    if len(seen) == len(f.blocks):
        return False
    for b in f.blocks:
        if b.id in seen and b.preds:
            dead = [i for i, p in enumerate(b.preds) if p.id not in seen]
            if dead:
                for i in reversed(dead):
                    del b.preds[i]
                    for phi in b.phis:
                        del phi.a[i]
    f.blocks = [b for b in f.blocks if b.id in seen]
    return True


def analyze(f):
    """Remove unreachable blocks, then compute RPO, dominators, dominator tree, loops, depth."""
    remove_unreachable(f)
    entry = f.blocks[0]
    # iterative DFS post-order
    post = []
    seen = set([entry.id])
    stack = [(entry, 0)]
    while stack:
        b, i = stack.pop()
        ss = b.succs()
        if i < len(ss):
            stack.append((b, i + 1))
            s = ss[i]
            if s.id not in seen:
                seen.add(s.id)
                stack.append((s, 0))
        else:
            post.append(b)
    rpo = post[::-1]
    for i, b in enumerate(rpo):
        b.rpo = i
        b.idom = None
        b.dkids = []
        b.depth = 0
        b.loop = None
    entry.idom = entry
    changed = True
    while changed:
        changed = False
        for b in rpo[1:]:
            new = None
            for p in b.preds:
                if p.idom is None:
                    continue
                if new is None:
                    new = p
                else:
                    x, y = p, new
                    while x is not y:
                        while x.rpo > y.rpo:
                            x = x.idom
                        while y.rpo > x.rpo:
                            y = y.idom
                    new = x
            if b.idom is not new:
                b.idom = new
                changed = True
    for b in rpo[1:]:
        b.idom.dkids.append(b)
    # pre/post numbering of the dominator tree
    n = 0
    stack = [(entry, 0)]
    entry.pre = 0
    while stack:
        b, i = stack.pop()
        if i == 0:
            b.pre = n
            n += 1
        if i < len(b.dkids):
            stack.append((b, i + 1))
            stack.append((b.dkids[i], 0))
        else:
            b.post = n
            n += 1
    f.rpo = rpo
    find_loops(f)
    return rpo


def dominates(a, b):
    return a.pre <= b.pre and b.post <= a.post


class Loop(object):
    __slots__ = ('header', 'blocks', 'ids', 'latches', 'parent', 'depth')

    def __init__(self, header):
        self.header = header
        self.blocks = []
        self.ids = set()
        self.latches = []
        self.parent = None
        self.depth = 1


def find_loops(f):
    """Natural loops (back edge = edge to a dominator). Loops sharing a header are merged."""
    loops = {}
    order = []
    for b in f.rpo:
        for s in b.succs():
            if dominates(s, b):
                lp = loops.get(s.id)
                if lp is None:
                    lp = Loop(s)
                    loops[s.id] = lp
                    order.append(lp)
                lp.latches.append(b)
    for lp in order:
        h = lp.header
        ids = set([h.id])
        body = [h]
        stack = []
        for l in lp.latches:
            if l.id not in ids:
                ids.add(l.id)
                body.append(l)
                stack.append(l)
        while stack:
            x = stack.pop()
            for p in x.preds:
                if p.id not in ids:
                    ids.add(p.id)
                    body.append(p)
                    stack.append(p)
        body.sort(key=lambda bb: bb.rpo)
        lp.blocks = body
        lp.ids = ids
    # nesting: process loops from the largest to the smallest so that the innermost wins
    order.sort(key=lambda lp: (-len(lp.blocks), lp.header.rpo))
    for lp in order:
        for b in lp.blocks:
            if b.loop is not None and lp.parent is None and b is lp.header:
                lp.parent = b.loop
            b.loop = lp
    for lp in order:
        d = 1
        p = lp.parent
        while p is not None:
            d += 1
            p = p.parent
        lp.depth = d
    for b in f.rpo:
        b.depth = b.loop.depth if b.loop is not None else 0
    f.loops = order
    return order


def clone_func(f):
    """Deep copy of a function without phis (non-SSA form)."""
    g = Func()
    g.name = f.name
    g.params = list(f.params)
    g.nval = f.nval
    g.nbid = f.nbid
    g.names = set(f.names)
    m = {}
    for b in f.blocks:
        nb = Block(b.id, b.name)
        nb.freq = b.freq
        m[b.id] = nb
        g.blocks.append(nb)
    for b in f.blocks:
        nb = m[b.id]
        for ins in b.ins:
            t = [m[x.id] for x in ins.t] if ins.t else ins.t
            nb.ins.append(Instr(ins.op, ins.d, list(ins.a), t))
    compute_preds(g)
    return g
