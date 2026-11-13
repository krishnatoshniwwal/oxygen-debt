#!/usr/bin/env python3
"""Generate very large KIR programs (about 100,000 instructions), in the naive style of the
kit's `large` kind (fresh register names everywhere, const + op + mov chains), to test kopt's
time limit (30 s) and what it gains on such programs.

Usage: python tests/gen_large.py OUTDIR [--size N] [--seed S]
Writes OUTDIR/large_{poly,crc,mix,loops,affine,crcstep,chain,ladder,nest,cold,diamonds,irreducible,pressure,smallloops,deepnest,nestloops,stld}.kir with a matching .in file each.
"""
import argparse
import os
import random


class Gen:
    def __init__(self, nparams):
        self.n = nparams
        self.lines = []
        self.ni = 0
        self.blk = 0

    def v(self):
        self.n += 1
        return 'v%d' % (self.n - 1)

    def emit(self, s):
        self.ni += 1
        self.lines.append('  ' + s)

    def label(self, name):
        self.lines.append(name + ':')

    def lab(self, base):
        self.blk += 1
        return '%s%d' % (base, self.blk)

    def count(self):
        return self.ni


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
    (i % 256), a stride constant (3*i+7) and a store address (1024 + i % 1024) change per copy
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
        g.emit('const %s, %d' % (o, 1024 + i % 1024))
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


def gen_diamonds(size, rnd):
    """Nested if/else: each unit is a diamond whose arms contain a second diamond (depth 3), all
    updating an accumulator; units chained one after another."""
    g = Gen(2)
    acc = g.v()
    g.emit('const %s, 0' % acc)
    k = [0]

    def diamond(depth):
        k[0] += 1
        t, c = g.v(), g.v()
        th, el, en = g.lab('dt'), g.lab('de'), g.lab('dj')
        g.emit('and %s, v0, %d' % (t, 1 << (k[0] % 24)))
        g.emit('lt %s, 0, %s' % (c, t))
        g.emit('br %s, %s, %s' % (c, th, el))
        for lbl, op in ((th, 'add'), (el, 'xor')):
            g.label(lbl)
            n = g.v()
            g.emit('%s %s, %s, %d' % (op, n, acc, rnd.randrange(1, 1000)))
            g.emit('mov %s, %s' % (acc, n))
            if depth > 0:
                diamond(depth - 1)
            m = g.v()
            g.emit('add %s, %s, v1' % (m, acc))
            g.emit('mov %s, %s' % (acc, m))
            g.emit('jmp %s' % en)
        g.label(en)

    while g.count() < size:
        diamond(3)
    g.emit('ret %s' % acc)
    return wrap('diamonds', 'v0, v1', g), 'args 123457 9\nmem 0 1 2 3\n'


def gen_irreducible(size, rnd):
    """Chained irreducible regions: entry branches into either of two blocks A and B that jump to
    each other (a cycle with two entries); a counter bounds the trips."""
    g = Gen(2)
    acc = g.v()
    g.emit('const %s, 0' % acc)
    k = 0
    while g.count() < size:
        n, c, t = g.v(), g.v(), g.v()
        A, B, out = g.lab('irA'), g.lab('irB'), g.lab('irX')
        g.emit('const %s, 0' % n)
        g.emit('and %s, v0, %d' % (t, 1 << (k % 20)))
        g.emit('lt %s, 0, %s' % (c, t))
        g.emit('br %s, %s, %s' % (c, A, B))
        for me, other in ((A, B), (B, A)):
            g.label(me)
            x, n2, y, cc = g.v(), g.v(), g.v(), g.v()
            g.emit('add %s, %s, %d' % (x, acc, rnd.randrange(1, 99)))
            g.emit('xor %s, %s, v1' % (y, x))
            g.emit('mov %s, %s' % (acc, y))
            g.emit('add %s, %s, 1' % (n2, n))
            g.emit('mov %s, %s' % (n, n2))
            g.emit('lt %s, %s, 3' % (cc, n))
            g.emit('br %s, %s, %s' % (cc, other, out))
        g.label(out)
        k += 1
    g.emit('ret %s' % acc)
    return wrap('irreducible', 'v0, v1', g), 'args 123457 9\nmem 0 1 2 3\n'


def gen_pressure(size, rnd):
    """Groups of 48 values loaded and kept alive together, then combined in a scrambled order
    (far more live values than any machine has registers)."""
    g = Gen(2)
    ck = g.v()
    g.emit('const %s, 0' % ck)
    k = 0
    while g.count() < size:
        vals = []
        for j in range(48):
            c, l = g.v(), g.v()
            g.emit('const %s, %d' % (c, (k * 7 + j) % 64))
            g.emit('load %s, %s' % (l, c))
            vals.append(l)
        order = list(range(48))
        rnd.shuffle(order)
        acc = ck
        for j in order:
            n = g.v()
            g.emit('%s %s, %s, %s' % (rnd.choice(['add', 'xor', 'mul']), n, acc, vals[j]))
            acc = n
        for j in range(0, 48, 3):  # second use of everything, after the first sweep
            n = g.v()
            g.emit('add %s, %s, %s' % (n, acc, vals[j]))
            acc = n
        g.emit('mov %s, %s' % (ck, acc))
        k += 1
    g.emit('ret %s' % ck)
    return wrap('pressure', 'v0, v1', g), 'args 5 9\nmem 0 ' + ' '.join(
        str(rnd.randrange(100)) for _ in range(64)) + '\n'


def gen_smallloops(size, rnd):
    """Thousands of tiny loops (trip count 1..3) that load, store and update an accumulator;
    every loop writes a different scratch word."""
    g = Gen(2)
    ck = g.v()
    g.emit('const %s, 0' % ck)
    k = 0
    while g.count() < size:
        i, s, lim = g.v(), g.v(), g.v()
        head, body, out = g.lab('sh'), g.lab('sb'), g.lab('sx')
        g.emit('const %s, 0' % i)
        g.emit('const %s, %d' % (s, rnd.randrange(9)))
        g.emit('const %s, %d' % (lim, rnd.randrange(1, 4)))
        g.emit('jmp %s' % head)
        g.label(head)
        c = g.v()
        g.emit('lt %s, %s, %s' % (c, i, lim))
        g.emit('br %s, %s, %s' % (c, body, out))
        g.label(body)
        a, l, m, u, n, o = [g.v() for _ in range(6)]
        g.emit('add %s, %s, %d' % (a, i, k % 60))
        g.emit('load %s, %s' % (l, a))
        g.emit('mul %s, %s, v1' % (m, l))
        g.emit('add %s, %s, %s' % (u, s, m))
        g.emit('mov %s, %s' % (s, u))
        g.emit('const %s, %d' % (o, 1024 + k % 2048))
        g.emit('store %s, %s' % (o, s))
        g.emit('add %s, %s, 1' % (n, i))
        g.emit('mov %s, %s' % (i, n))
        g.emit('jmp %s' % head)
        g.label(out)
        x = g.v()
        g.emit('add %s, %s, %s' % (x, ck, s))
        g.emit('mov %s, %s' % (ck, x))
        k += 1
    g.emit('ret %s' % ck)
    return wrap('smallloops', 'v0, v1', g), 'args 5 9\nmem 0 ' + ' '.join(
        str(rnd.randrange(100)) for _ in range(64)) + '\n'


def gen_deepnest(size, rnd):
    """One loop nest as deep as the size allows (about size/10 levels), every loop running once;
    the body is in the innermost loop. Stresses recursion depth in loop analysis."""
    g = Gen(1)
    s = g.v()
    g.emit('const %s, 0' % s)
    depth = max(1, size // 10)
    ivs = []
    for d in range(depth):
        i = g.v()
        ivs.append(i)
        g.emit('const %s, 0' % i)
        g.emit('jmp dh%d' % d)
        g.label('dh%d' % d)
        c = g.v()
        g.emit('lt %s, %s, 1' % (c, i))
        g.emit('br %s, db%d, dx%d' % (c, d, d))
        g.label('db%d' % d)
    t = g.v()
    g.emit('add %s, %s, v0' % (t, s))
    g.emit('mov %s, %s' % (s, t))
    for d in reversed(range(depth)):
        n = g.v()
        g.emit('add %s, %s, 1' % (n, ivs[d]))
        g.emit('mov %s, %s' % (ivs[d], n))
        g.emit('jmp dh%d' % d)
        g.label('dx%d' % d)
    g.emit('ret %s' % s)
    return wrap('deepnest', 'v0', g), 'args 7\nmem 0 1 2 3\n'


def gen_nestloops(size, rnd):
    """Many separate 6-deep loop nests; trips are 2 only at one random level (product 2) so the
    dynamic length stays small."""
    g = Gen(1)
    ck = g.v()
    g.emit('const %s, 0' % ck)
    k = 0
    while g.count() < size:
        two = rnd.randrange(6)
        ivs = []
        for d in range(6):
            i = g.v()
            ivs.append(i)
            g.emit('const %s, 0' % i)
            g.emit('jmp nh%d_%d' % (k, d))
            g.label('nh%d_%d' % (k, d))
            c = g.v()
            g.emit('lt %s, %s, %d' % (c, i, 2 if d == two else 1))
            g.emit('br %s, nb%d_%d, nx%d_%d' % (c, k, d, k, d))
            g.label('nb%d_%d' % (k, d))
        t, u = g.v(), g.v()
        g.emit('mul %s, %s, %d' % (t, ivs[-1], rnd.randrange(2, 9)))
        g.emit('add %s, %s, v0' % (u, t))
        g.emit('add %s, %s, %s' % (ck, ck, u))
        for d in reversed(range(6)):
            n = g.v()
            g.emit('add %s, %s, 1' % (n, ivs[d]))
            g.emit('mov %s, %s' % (ivs[d], n))
            g.emit('jmp nh%d_%d' % (k, d))
            g.label('nx%d_%d' % (k, d))
        k += 1
    g.emit('ret %s' % ck)
    return wrap('nestloops', 'v0', g), 'args 7\nmem 0 1 2 3\n'


def gen_stld(size, rnd):
    """Long straight line of stores and loads to a small set of scratch addresses (heavy aliasing:
    store-to-load forwarding, redundant loads, dead stores)."""
    g = Gen(2)
    ck = g.v()
    g.emit('const %s, 0' % ck)
    while g.count() < size:
        a, b = g.v(), g.v()
        ad, bd = rnd.randrange(1024, 1040), rnd.randrange(1024, 1040)
        g.emit('const %s, %d' % (a, ad))
        g.emit('const %s, %d' % (b, bd))
        l = g.v()
        g.emit('load %s, %s' % (l, a))
        x = g.v()
        g.emit('add %s, %s, %d' % (x, l, rnd.randrange(1, 50)))
        g.emit('store %s, %s' % (b, x))
        l2 = g.v()
        g.emit('load %s, %s' % (l2, b))
        y = g.v()
        g.emit('xor %s, %s, v1' % (y, l2))
        g.emit('store %s, %s' % (a, y))
        n = g.v()
        g.emit('add %s, %s, %s' % (n, ck, y))
        g.emit('mov %s, %s' % (ck, n))
    g.emit('ret %s' % ck)
    return wrap('stld', 'v0, v1', g), 'args 5 9\nmem 1024 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16\n'


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
              ('ladder', gen_ladder), ('nest', gen_nest), ('cold', gen_cold),
              ('diamonds', gen_diamonds), ('irreducible', gen_irreducible), ('pressure', gen_pressure),
              ('smallloops', gen_smallloops), ('deepnest', gen_deepnest), ('nestloops', gen_nestloops),
              ('stld', gen_stld))
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
