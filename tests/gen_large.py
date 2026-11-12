#!/usr/bin/env python3
"""Generate very large KIR programs (about 100,000 instructions), in the naive style of the
kit's `large` kind (fresh register names everywhere, const + op + mov chains), to test kopt's
time limit (30 s) and what it gains on such programs.

Usage: python tests/gen_large.py OUTDIR [--size N] [--seed S]
Writes OUTDIR/large_{poly,crc,mix,loops}.kir with a matching .in file each.
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


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('outdir')
    ap.add_argument('--size', type=int, default=100000)
    ap.add_argument('--seed', type=int, default=1)
    o = ap.parse_args()
    os.makedirs(o.outdir, exist_ok=True)
    for name, fn in (('poly', gen_poly), ('crc', gen_crc), ('mix', gen_mix), ('loops', gen_loops)):
        rnd = random.Random(o.seed)
        text, inp = fn(o.size, rnd)
        with open(os.path.join(o.outdir, 'large_%s.kir' % name), 'w', newline='\n') as fh:
            fh.write(text)
        with open(os.path.join(o.outdir, 'large_%s.in' % name), 'w', newline='\n') as fh:
            fh.write(inp)


if __name__ == '__main__':
    main()
