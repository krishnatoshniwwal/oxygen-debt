#!/usr/bin/env python3
"""Generate very large KIR programs (about 100,000 instructions), in the naive style of the
kit's `large` kind (fresh register names everywhere, const + op + mov chains), to test kopt's
time limit (30 s) and what it gains on such programs.

Usage: python tests/gen_large.py OUTDIR [--size N] [--seed S]
Writes OUTDIR/large_{poly,crc,mix,loops,affine,crcstep,chain,ladder,nest,cold}.kir with a matching .in file each.
"""
import argparse
import os
import random


class Gen:
    def __init__(self, nparams):
        self.n = nparams
        self.lines = []
        self.blk = 0

    def v(self):
        self.n += 1
        return 'v%d' % (self.n - 1)

    def emit(self, s):
        self.lines.append('  ' + s)

    def label(self, name):
        self.lines.append(name + ':')

    def lab(self, base):
        self.blk += 1
        return '%s%d' % (base, self.blk)

    def count(self):
        return sum(1 for l in self.lines if l.startswith('  '))


def gen_poly(size, rnd):
    """acc = acc * x + mem[i % 64], written out term by term (unrolled_poly, much longer)."""
    g = Gen(1)
    acc = g.v()
    g.emit('const %s, 0' % acc)
    i = 0
    while g.count() < size:
        t, c, l, s = g.v(), g.v(), g.v(), g.v()
        g.emit('mul %s, %s, v0' % (t, acc))
        g.emit('const %s, %d' % (c, i % 64))
        g.emit('load %s, %s' % (l, c))
        g.emit('add %s, %s, %s' % (s, t, l))
        g.emit('mov %s, %s' % (acc, s))
        i += 1
    g.emit('ret %s' % acc)
    return 'func large_poly(v0)\nentry:\n' + '\n'.join(g.lines) + '\n', 'args 3\nmem 0 ' + ' '.join(
        str(rnd.randrange(11)) for _ in range(64)) + '\n'


def gen_crc(size, rnd):
    """Bitwise CRC steps over the bytes of mem[0..255], every step an if/else (unrolled_crc12)."""
    g = Gen(1)
    crc = g.v()
    g.emit('const %s, 0xFFFFFFFF' % crc)
    byte = 0
    while g.count() < size:
        a, b, x = g.v(), g.v(), g.v()
        g.emit('const %s, %d' % (a, byte % 256))
        g.emit('load %s, %s' % (b, a))
        g.emit('xor %s, %s, %s' % (x, crc, b))
        g.emit('mov %s, %s' % (crc, x))
        for _ in range(8):
            m, e = g.v(), g.v()
            th, el, en = g.lab('then'), g.lab('else'), g.lab('endif')
            g.emit('and %s, %s, 1' % (m, crc))
            g.emit('eq %s, %s, 1' % (e, m))
            g.emit('br %s, %s, %s' % (e, th, el))
            g.label(th)
            s1, s2 = g.v(), g.v()
            g.emit('shr %s, %s, 1' % (s1, crc))
            g.emit('xor %s, %s, 0xEDB88320' % (s2, s1))
            g.emit('mov %s, %s' % (crc, s2))
            g.emit('jmp %s' % en)
            g.label(el)
            s3 = g.v()
            g.emit('shr %s, %s, 1' % (s3, crc))
            g.emit('mov %s, %s' % (crc, s3))
            g.emit('jmp %s' % en)
            g.label(en)
        byte += 1
    r = g.v()
    g.emit('xor %s, %s, 0xFFFFFFFF' % (r, crc))
    g.emit('ret %s' % r)
    return 'func large_crc(v0)\nentry:\n' + '\n'.join(g.lines) + '\n', 'args 7\nmem 0 ' + ' '.join(
        str(rnd.randrange(256)) for _ in range(256)) + '\n'


def gen_mix(size, rnd):
    """Independent groups: load a few words, compute with repeated and dead work, store the
    result to 1024+k, fold it into a checksum. Many values alive, much junk."""
    g = Gen(2)
    ck = g.v()
    g.emit('const %s, 0' % ck)
    k = 0
    ops = ['add', 'sub', 'xor', 'and', 'or', 'mul', 'shl', 'shr']
    while g.count() < size:
        vals = []
        for _ in range(rnd.randrange(2, 5)):
            c, l = g.v(), g.v()
            g.emit('const %s, %d' % (c, rnd.randrange(64)))
            g.emit('load %s, %s' % (l, c))
            vals.append(l)
        vals += ['v0', 'v1']
        for _ in range(rnd.randrange(4, 10)):
            op = rnd.choice(ops)
            d = g.v()
            a = rnd.choice(vals)
            if op in ('shl', 'shr'):
                g.emit('%s %s, %s, %d' % (op, d, a, rnd.randrange(1, 8)))
            else:
                b = rnd.choice(vals)
                g.emit('%s %s, %s, %s' % (op, d, a, b))
                if rnd.random() < 0.3:  # the same work again
                    d2 = g.v()
                    g.emit('%s %s, %s, %s' % (op, d2, a, b))
                    d = d2
            if rnd.random() < 0.2:  # dead code
                g.emit('mul %s, %s, %d' % (g.v(), d, rnd.randrange(3, 99)))
            vals.append(d)
        res = vals[-1]
        a = g.v()
        g.emit('const %s, %d' % (a, 1024 + k % 1024))
        g.emit('store %s, %s' % (a, res))
        n = g.v()
        g.emit('add %s, %s, %s' % (n, ck, res))
        g.emit('mov %s, %s' % (ck, n))
        k += 1
    g.emit('ret %s' % ck)
    return 'func large_mix(v0, v1)\nentry:\n' + '\n'.join(g.lines) + '\n', 'args 5 9\nmem 0 ' + ' '.join(
        str(rnd.randrange(100)) for _ in range(64)) + '\n'


def gen_loops(size, rnd):
    """Many small loops one after another: sums, constant-trip loops (peeling) and loops over
    memory, each feeding a checksum. Tests the loop passes on a very large program."""
    g = Gen(1)
    ck = g.v()
    g.emit('const %s, 0' % ck)
    while g.count() < size:
        kind = rnd.randrange(3)
        i, s, lim = g.v(), g.v(), g.v()
        head, body, out = g.lab('head'), g.lab('body'), g.lab('exit')
        g.emit('const %s, 0' % i)
        g.emit('const %s, 0' % s)
        # short trips: the whole program must finish within kvalidate's 200,000 steps
        if kind == 0:
            g.emit('and %s, v0, %d' % (lim, rnd.choice([1, 3])))
        elif kind == 1:
            g.emit('const %s, %d' % (lim, rnd.randrange(1, 4)))
        else:
            g.emit('const %s, %d' % (lim, rnd.randrange(1, 4)))
        g.emit('jmp %s' % head)
        g.label(head)
        c = g.v()
        g.emit('lt %s, %s, %s' % (c, i, lim))
        g.emit('br %s, %s, %s' % (c, body, out))
        g.label(body)
        t, u, n = g.v(), g.v(), g.v()
        if kind == 2:
            a = g.v()
            g.emit('add %s, %s, %d' % (a, i, rnd.randrange(32)))
            g.emit('load %s, %s' % (t, a))
        else:
            g.emit('mul %s, %s, %d' % (t, i, rnd.randrange(2, 9)))
        g.emit('add %s, %s, %s' % (u, s, t))
        g.emit('mov %s, %s' % (s, u))
        g.emit('add %s, %s, 1' % (n, i))
        g.emit('mov %s, %s' % (i, n))
        g.emit('jmp %s' % head)
        g.label(out)
        x = g.v()
        g.emit('xor %s, %s, %s' % (x, ck, s))
        g.emit('mov %s, %s' % (ck, x))
    g.emit('ret %s' % ck)
    return 'func large_loops(v0)\nentry:\n' + '\n'.join(g.lines) + '\n', 'args 20\nmem 0 ' + ' '.join(
        str(rnd.randrange(100)) for _ in range(64)) + '\n'


def wrap(name, params, g):
    return 'func large_%s(%s)\nentry:\n' % (name, params) + '\n'.join(g.lines) + '\n'


def gen_affine(size, rnd):
    """Repetitive straight-line code whose copies differ by affine constants: a load address that wraps
    (i % 256), a stride constant (3*i+7) and a store address (1024 + i % 1500) change per copy
    (a rerolling target)."""
    g = Gen(2)
    ck = g.v()
    g.emit('const %s, 0' % ck)
    i = 0
    while g.count() < size:
        a, l, m, s, t, o, n = [g.v() for _ in range(7)]
        g.emit('const %s, %d' % (a, i % 256))
        g.emit('load %s, %s' % (l, a))
        g.emit('mul %s, %s, v1' % (m, l))
        g.emit('add %s, %s, %d' % (s, m, 3 * i + 7))
        g.emit('xor %s, %s, v0' % (t, s))
        g.emit('const %s, %d' % (o, 1024 + i % 1500))
        g.emit('store %s, %s' % (o, t))
        g.emit('add %s, %s, %s' % (n, ck, t))
        g.emit('mov %s, %s' % (ck, n))
        i += 1
    g.emit('ret %s' % ck)
    return wrap('affine', 'v0, v1', g), 'args 5 9\nmem 0 ' + ' '.join(
        str(rnd.randrange(100)) for _ in range(256)) + '\n'


def gen_crcstep(size, rnd):
    """Branch-free CRC steps: crc = (crc >> 1) ^ (POLY & -(crc & 1)); eight per input byte, the byte
    address changing per copy."""
    g = Gen(1)
    crc = g.v()
    g.emit('const %s, 0xFFFFFFFF' % crc)
    byte = 0
    while g.count() < size:
        a, b, x = g.v(), g.v(), g.v()
        g.emit('const %s, %d' % (a, byte % 200))
        g.emit('load %s, %s' % (b, a))
        g.emit('xor %s, %s, %s' % (x, crc, b))
        g.emit('mov %s, %s' % (crc, x))
        for _ in range(8):
            m, n, p, s, r = [g.v() for _ in range(5)]
            g.emit('and %s, %s, 1' % (m, crc))
            g.emit('sub %s, 0, %s' % (n, m))
            g.emit('and %s, %s, 0xEDB88320' % (p, n))
            g.emit('shr %s, %s, 1' % (s, crc))
            g.emit('xor %s, %s, %s' % (r, s, p))
            g.emit('mov %s, %s' % (crc, r))
        byte += 1
    g.emit('ret %s' % crc)
    return wrap('crcstep', 'v0', g), 'args 7\nmem 0 ' + ' '.join(
        str(rnd.randrange(256)) for _ in range(256)) + '\n'


def gen_chain(size, rnd):
    """One long dependent chain with mixed operations and random per-step constants (no repetition)."""
    g = Gen(2)
    acc = g.v()
    g.emit('add %s, v0, v1' % acc)
    ops = ['add', 'xor', 'mul', 'sub', 'or']
    while g.count() < size:
        n = g.v()
        g.emit('%s %s, %s, %d' % (rnd.choice(ops), n, acc, rnd.randrange(1, 1 << 20)))
        m = g.v()
        g.emit('shr %s, %s, %d' % (m, n, rnd.randrange(1, 9)))
        k = g.v()
        g.emit('xor %s, %s, %s' % (k, n, m))
        acc = k
    g.emit('ret %s' % acc)
    return wrap('chain', 'v0, v1', g), 'args 5 9\nmem 0 1 2 3\n'


def gen_ladder(size, rnd):
    """Deep CFG: a long sequence of data-dependent if/else diamonds that each update one accumulator
    (deep dominator tree, many joins)."""
    g = Gen(2)
    acc = g.v()
    g.emit('const %s, 0' % acc)
    k = 0
    while g.count() < size:
        c, t = g.v(), g.v()
        cont, side, join = g.lab('cont'), g.lab('side'), g.lab('join')
        g.emit('and %s, v0, %d' % (t, 1 << (k % 20)))
        g.emit('lt %s, 0, %s' % (c, t))
        g.emit('br %s, %s, %s' % (c, side, cont))
        g.label(side)
        s1 = g.v()
        g.emit('add %s, %s, %d' % (s1, acc, k + 1))
        g.emit('mov %s, %s' % (acc, s1))
        g.emit('jmp %s' % join)
        g.label(cont)
        s2 = g.v()
        g.emit('xor %s, %s, v1' % (s2, acc))
        g.emit('mov %s, %s' % (acc, s2))
        g.emit('jmp %s' % join)
        g.label(join)
        k += 1
    g.emit('ret %s' % acc)
    return wrap('ladder', 'v0, v1', g), 'args 123457 9\nmem 0 1 2 3\n'


def gen_nest(size, rnd):
    """Very deeply nested ifs: if (c1) { ...; if (c2) { ...; if (c3) ... } } with a common exit.
    Dominator-tree depth equals the number of levels."""
    g = Gen(2)
    acc = g.v()
    g.emit('const %s, 0' % acc)
    k = 0
    while g.count() < size:
        c, t = g.v(), g.v()
        nxt = g.lab('lvl')
        g.emit('and %s, v0, %d' % (t, 1 << (k % 24)))
        g.emit('br %s, %s, out' % (t, nxt))
        g.label(nxt)
        s1 = g.v()
        g.emit('add %s, %s, %d' % (s1, acc, k + 1))
        g.emit('mov %s, %s' % (acc, s1))
        k += 1
    g.emit('jmp out')
    g.label('out')
    g.emit('ret %s' % acc)
    return wrap('nest', 'v0, v1', g), 'args -1 9\nmem 0 1 2 3\n'


def gen_cold(size, rnd):
    """A small hot loop, then a huge block that is skipped at run time (v0 != 12345), then a second
    small hot loop: mostly cold code."""
    g = Gen(1)
    i, s, c = g.v(), g.v(), g.v()
    g.emit('const %s, 0' % i)
    g.emit('const %s, 0' % s)
    g.emit('jmp h1')
    g.label('h1')
    g.emit('lt %s, %s, 200' % (c, i))
    g.emit('br %s, b1, mid' % c)
    g.label('b1')
    t = g.v()
    g.emit('mul %s, %s, 3' % (t, i))
    g.emit('add %s, %s, %s' % (s, s, t))
    g.emit('add %s, %s, 1' % (i, i))
    g.emit('jmp h1')
    g.label('mid')
    e = g.v()
    g.emit('eq %s, v0, 12345' % e)
    g.emit('br %s, cold, h2pre' % e)
    g.label('cold')
    acc = g.v()
    g.emit('mov %s, %s' % (acc, s))
    while g.count() < size:
        x, y = g.v(), g.v()
        g.emit('mul %s, %s, %d' % (x, acc, rnd.randrange(3, 99)))
        g.emit('xor %s, %s, %d' % (y, x, rnd.randrange(1 << 16)))
        acc = y
    g.emit('mov %s, %s' % (s, acc))
    g.emit('jmp h2pre')
    g.label('h2pre')
    g.emit('const %s, 0' % i)
    g.emit('jmp h2')
    g.label('h2')
    g.emit('lt %s, %s, 300' % (c, i))
    g.emit('br %s, b2, done' % c)
    g.label('b2')
    g.emit('add %s, %s, %s' % (s, s, i))
    g.emit('add %s, %s, 1' % (i, i))
    g.emit('jmp h2')
    g.label('done')
    g.emit('ret %s' % s)
    return wrap('cold', 'v0', g), 'args 7\nmem 0 1 2 3\n'


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('outdir')
    ap.add_argument('--size', type=int, default=100000)
    ap.add_argument('--seed', type=int, default=1)
    ap.add_argument('--only', default=None)
    o = ap.parse_args()
    os.makedirs(o.outdir, exist_ok=True)
    shapes = (('poly', gen_poly), ('crc', gen_crc), ('mix', gen_mix), ('loops', gen_loops),
              ('affine', gen_affine), ('crcstep', gen_crcstep), ('chain', gen_chain),
              ('ladder', gen_ladder), ('nest', gen_nest), ('cold', gen_cold))
    for name, fn in shapes:
        if o.only and o.only != name:
            continue
        rnd = random.Random(o.seed)
        text, inp = fn(o.size, rnd)
        with open(os.path.join(o.outdir, 'large_%s.kir' % name), 'w', newline='\n') as fh:
            fh.write(text)
        with open(os.path.join(o.outdir, 'large_%s.in' % name), 'w', newline='\n') as fh:
            fh.write(inp)


if __name__ == '__main__':
    main()
