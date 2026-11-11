#!/usr/bin/env python3
"""Random-program stress test for kopt.

Builds random KIR programs (loops, branches, memory, traps, volatile I/O, odd control
flow), optimizes each one for a random machine and compares the behaviour of input and
output on many inputs. Any difference is printed with the program so it can be replayed.

Usage: python tests/fuzz.py [--n 500] [--seed 1] [--keep DIR]
"""
import argparse
import os
import random
import sys

HERE = os.path.dirname(os.path.realpath(__file__))
ROOT = os.path.dirname(HERE)
sys.dont_write_bytecode = True
sys.path.insert(0, os.path.join(ROOT, 'src'))
import kirlib as K  # noqa: E402
import driver  # noqa: E402

CONSTS = [0, 1, 2, 3, 4, 5, 7, 8, 15, 16, 31, 32, 33, 63, 64, 100, 255, 256, 1000, 4095, 4096,
          3071, 3072, 65535, 0x7FFFFFFF, 0x80000000, 0xFFFFFFFF, 0xFFFFFFFE, -1, -2, -8]


class Gen(object):
    def __init__(self, rnd):
        self.r = rnd
        self.nparams = rnd.randint(0, 3)
        self.nregs = rnd.randint(self.nparams + 2, 14)
        self.lines = []
        self.nlabel = 0
        self.budget = rnd.randint(8, 70)
        self.loopdepth = 0

    def reg(self):
        return 'v%d' % self.r.randrange(self.nregs)

    def src(self):
        if self.r.random() < 0.3:
            return str(self.r.choice(CONSTS))
        return self.reg()

    def label(self, hint='b'):
        self.nlabel += 1
        return '%s%d' % (hint, self.nlabel)

    def emit(self, s):
        self.lines.append('  ' + s)

    def simple(self):
        r = self.r
        k = r.random()
        if k < 0.55:
            op = r.choice(['add', 'sub', 'mul', 'and', 'or', 'xor', 'shl', 'shr', 'lt', 'eq',
                           'add', 'sub', 'add', 'mul', 'and'])
            self.emit('%s %s, %s, %s' % (op, self.reg(), self.src(), self.src()))
        elif k < 0.62:
            self.emit('%s %s, %s' % (r.choice(['mov', 'mov', 'const']), self.reg(),
                                     str(r.choice(CONSTS)) if r.random() < 0.4 else self.reg()))
            if self.lines[-1].strip().startswith('const') and 'v' in self.lines[-1].split(',')[1]:
                self.lines[-1] = '  const %s, %d' % (self.reg(), r.choice(CONSTS))
        elif k < 0.67:
            self.emit('select %s, %s, %s, %s' % (self.reg(), self.src(), self.src(), self.src()))
        elif k < 0.72:
            self.emit('%s %s, %s, %s' % (r.choice(['div', 'mod']), self.reg(), self.src(),
                                         r.choice([self.src(), '3', '4', '8', '-1', '7', '16'])))
        elif k < 0.84:
            # memory access; most of the time through a masked address
            a = self.reg()
            if r.random() < 0.75:
                t = self.reg()
                self.emit('and %s, %s, %d' % (t, a, r.choice([7, 15, 63, 255, 4095])))
                a = t
                if r.random() < 0.3:
                    self.emit('add %s, %s, %d' % (t, t, r.choice([1, 8, 64, 3000, 4090])))
            elif r.random() < 0.5:
                a = str(r.choice([0, 1, 5, 64, 3071, 3072, 4095, 4096]))
            if r.random() < 0.5:
                self.emit('load %s, %s' % (self.reg(), a))
            else:
                self.emit('store %s, %s' % (a, self.src()))
        elif k < 0.90:
            p = str(r.randrange(4)) if r.random() < 0.8 else self.reg()
            if r.random() < 0.5:
                self.emit('vload %s, %s' % (self.reg(), p))
            else:
                self.emit('vstore %s, %s' % (p, self.src()))
        elif k < 0.94:
            self.emit('mac %s, %s, %s, %s' % (self.reg(), self.src(), self.src(), self.src())) \
                if self.fused else self.emit('add %s, %s, %s' % (self.reg(), self.src(), self.src()))
        else:
            self.emit('addsh %s, %s, %s, %d' % (self.reg(), self.src(), self.src(), r.randrange(32))) \
                if self.fused else self.emit('xor %s, %s, %s' % (self.reg(), self.src(), self.src()))

    def stmts(self, n):
        r = self.r
        for _ in range(n):
            if self.budget <= 0:
                return
            self.budget -= 1
            k = r.random()
            if k < 0.72:
                self.simple()
            elif k < 0.84:
                # if / else
                c = self.reg()
                if r.random() < 0.6:
                    self.emit('%s %s, %s, %s' % (r.choice(['lt', 'eq']), c, self.src(), self.src()))
                lt, lf, le = self.label('then'), self.label('else'), self.label('endif')
                if r.random() < 0.3:
                    self.emit('br %s, %s, %s' % (c, lt, le))
                    self.lines.append(lt + ':')
                    self.stmts(r.randint(1, 4))
                    self.emit('jmp ' + le)
                else:
                    self.emit('br %s, %s, %s' % (c, lt, lf))
                    self.lines.append(lt + ':')
                    self.stmts(r.randint(1, 4))
                    self.emit('jmp ' + le)
                    self.lines.append(lf + ':')
                    self.stmts(r.randint(1, 4))
                    self.emit('jmp ' + le)
                self.lines.append(le + ':')
            elif k < 0.94 and self.loopdepth < 2:
                # counted loop with a small bound; the counter and bound get reserved registers
                self.loopdepth += 1
                i = 'v%d' % (self.nregs + 2 * self.loopdepth)
                n_ = 'v%d' % (self.nregs + 2 * self.loopdepth + 1)
                lh, lb, lx = self.label('head'), self.label('body'), self.label('exit')
                style = r.random()
                if r.random() < 0.6:
                    self.emit('and %s, %s, %d' % (n_, self.reg(), r.choice([3, 7, 15])))
                else:
                    self.emit('const %s, %d' % (n_, r.choice([0, 1, 2, 4, 5, 8])))
                self.emit('const %s, 0' % i)
                if style < 0.7:
                    self.emit('jmp ' + lh)
                    self.lines.append(lh + ':')
                    c = self.reg()
                    self.emit('lt %s, %s, %s' % (c, i, n_))
                    self.emit('br %s, %s, %s' % (c, lb, lx))
                    self.lines.append(lb + ':')
                    self.stmts(r.randint(1, 6))
                    self.emit('add %s, %s, 1' % (i, i))
                    self.emit('jmp ' + lh)
                else:
                    # bottom-tested loop
                    self.emit('jmp ' + lb)
                    self.lines.append(lb + ':')
                    self.stmts(r.randint(1, 6))
                    self.emit('add %s, %s, 1' % (i, i))
                    c = self.reg()
                    self.emit('lt %s, %s, %s' % (c, i, n_))
                    self.emit('br %s, %s, %s' % (c, lb, lx))
                self.lines.append(lx + ':')
                self.loopdepth -= 1
            elif k < 0.97:
                # early exit
                c = self.reg()
                lt, le = self.label('out'), self.label('cont')
                self.emit('br %s, %s, %s' % (c, lt, le))
                self.lines.append(lt + ':')
                if r.random() < 0.5:
                    self.emit('ret %s' % self.src())
                else:
                    self.emit('trap %s' % self.src())
                self.lines.append(le + ':')
            else:
                self.simple()

    def build(self, fused):
        r = self.r
        self.fused = fused
        self.lines.append('func fz(%s)' % ', '.join('v%d' % i for i in range(self.nparams)))
        self.lines.append('entry:')
        self.stmts(self.budget)
        self.emit('ret %s' % self.src())
        text = '\n'.join(self.lines) + '\n'
        if r.random() < 0.25:
            text = self.mangle(text)
        return text

    def mangle(self, text):
        """Odd control flow: shuffle the blocks (entry stays first), add a fuel-limited
        jump into the middle of somewhere, add an unreachable block."""
        r = self.r
        lines = text.split('\n')
        head = lines[0]
        blocks = []
        for ln in lines[1:]:
            if ln.endswith(':'):
                blocks.append([ln])
            elif ln.strip():
                blocks[-1].append(ln)
        labels = [b[0][:-1] for b in blocks]
        fuel = 'v%d' % (self.nregs + 20)
        if len(blocks) > 2 and r.random() < 0.7:
            # an extra edge to a random block, taken at most 3 times
            bi = r.randrange(len(blocks))
            tgt = r.choice(labels[1:])
            term = blocks[bi].pop()
            la, lb_ = 'odd_a', 'odd_b'
            cnd = 'v%d' % (self.nregs + 21)
            blocks[bi].append('  add %s, %s, 1' % (fuel, fuel))
            blocks[bi].append('  lt %s, %s, 4' % (cnd, fuel))
            blocks[bi].append('  br %s, %s, %s' % (cnd, la, lb_))
            blocks.append([la + ':', '  and %s, %s, 1' % (cnd, self.reg()), '  br %s, %s, %s' % (cnd, tgt, lb_)])
            blocks.append([lb_ + ':', term])
        blocks.append(['dead_x:', '  div v0, v0, 0', '  jmp %s' % r.choice(labels)])
        rest = blocks[1:]
        r.shuffle(rest)
        out = [head]
        for b in [blocks[0]] + rest:
            out.extend(b)
        return '\n'.join(out) + '\n'


def random_machine(rnd):
    mc = {
        'issue_width': rnd.choice([1, 2, 2, 3, 4]),
        'mem_ports': rnd.choice([1, 1, 2]),
        'registers': rnd.choice([4, 4, 5, 6, 8, 8, 12, 16]),
        'reg_spill_penalty': rnd.choice([0, 3, 5, 8]),
        'taken_branch_penalty': rnd.choice([0, 1, 2, 4]),
        'icache_size': rnd.choice([8, 16, 32, 64, 64, 256]),
        'icache_penalty': rnd.choice([0, 1, 2, 4]),
        'lat_alu': rnd.choice([1, 1, 1, 2]),
        'lat_mul': rnd.choice([1, 2, 3, 5]),
        'lat_div': rnd.choice([4, 12, 20]),
        'lat_load': rnd.choice([1, 2, 4, 12]),
        'lat_mac': rnd.choice([1, 3, 5]),
        'lat_addsh': rnd.choice([1, 1, 2]),
        'fused_mac': rnd.choice([0, 1]),
        'fused_addsh': rnd.choice([0, 1]),
    }
    return mc


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--n', type=int, default=300)
    ap.add_argument('--seed', type=int, default=1)
    ap.add_argument('--keep', default=None)
    ap.add_argument('--inputs', type=int, default=70)
    ap.add_argument('--nodebug', action='store_true')
    o = ap.parse_args()
    bad = 0
    fallback = 0
    for it in range(o.n):
        rnd = random.Random(o.seed * 1000003 + it)
        mc = random_machine(rnd)
        g = Gen(rnd)
        text = g.build(bool(mc['fused_mac'] and mc['fused_addsh']))
        try:
            orig = K.parse_program(text)
        except K.KirError as e:
            print('generator bug: %s' % e)
            print(text)
            return 2
        opts = driver.Options(debug=not o.nodebug, check=not o.nodebug)
        problem = None
        try:
            out = driver.optimize(text, mc, None, opts)
            new = K.parse_program(out)
            K.check_machine_features(new, mc)
        except Exception as e:  # noqa
            import traceback
            problem = 'EXCEPTION %s: %s\n%s' % (type(e).__name__, e, traceback.format_exc())
            out = ''
        if problem is None:
            if opts.stats.get('candidate') == -1:
                fallback += 1
            cpo, cpn = K.compile_program(orig), K.compile_program(new)
            inputs = K.generate_inputs(len(orig.params), o.inputs, 777 + it)
            # extra inputs: big memory values everywhere / zero memory
            for ii in K.generate_inputs(len(orig.params), 6, 31337 + it):
                ii.segments = [(0, [(w * 2654435761) & 0xFFFFFFFF for w in ii.segments[0][1]])]
                inputs.append(ii)
            for k, ii in enumerate(inputs):
                ro = K.execute(cpo, ii.args, ii.segments, ii.vin, 30000)
                if ro.status == 'TIMEOUT':
                    continue
                rn = K.execute(cpn, ii.args, ii.segments, ii.vin, 20 * ro.steps + 1000)
                diff = K.first_difference(ro, rn)
                if diff:
                    problem = 'MISMATCH on input #%d: %s\n%s' % (k, diff, K.format_input(ii))
                    break
        if problem:
            bad += 1
            print('==== iteration %d (seed %d): %s' % (it, o.seed, problem))
            print('machine: %r' % (mc,))
            if o.keep:
                os.makedirs(o.keep, exist_ok=True)
                with open(os.path.join(o.keep, 'fail_%d_%d.kir' % (o.seed, it)), 'w') as fh:
                    fh.write(text)
                with open(os.path.join(o.keep, 'fail_%d_%d.cfg' % (o.seed, it)), 'w') as fh:
                    for kk, vv in mc.items():
                        fh.write('%s = %d\n' % (kk, vv))
                with open(os.path.join(o.keep, 'fail_%d_%d.out' % (o.seed, it)), 'w') as fh:
                    fh.write(out)
            else:
                print(text)
            if bad >= 5:
                break
    print('fuzz: %d programs, %d problems, %d fell back to the input' % (o.n, bad, fallback))
    return 1 if bad else 0


if __name__ == '__main__':
    sys.exit(main())
