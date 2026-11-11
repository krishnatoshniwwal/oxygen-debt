"""driver.py - the kopt pipeline and the search over its knobs."""
import sys

import kirlib as K
import ir
import ssa
import passes
import passes2
import passes3
import backend
import evalsim
from ranges import Ranges


class Options(object):
    def __init__(self, **kw):
        self.debug = False
        self.check = False
        self.stats = {}
        for k, v in kw.items():
            setattr(self, k, v)


class Variant(object):
    """One setting of the optimizer's knobs."""
    __slots__ = ('peel', 'unroll', 'ifc', 'sched', 'mac', 'spill')

    def __init__(self, peel=1, unroll=1, ifc=4, sched=True, mac=True, spill=False):
        self.spill = spill      # keep values in scratch memory when registers run out
        self.peel = peel        # 0 = off, 1 = within the instruction cache, 2 = loose
        self.unroll = unroll    # loop unrolling factor applied before the second SSA round
        self.ifc = ifc          # if-conversion: max instructions per side (0 = off)
        self.sched = sched
        self.mac = mac

    def key(self):
        return (self.peel, self.unroll, self.ifc, self.sched, self.mac, self.spill)

    def but(self, **kw):
        v = Variant(*self.key())
        for k, x in kw.items():
            setattr(v, k, x)
        return v


def phase_a(f, v, chk, light=False):
    """The SSA optimizations, repeated until nothing changes.
    light=True (very large programs): one round only."""
    def scalar():
        for _ in range(2 if light else 6):
            c1 = passes.scalar_pass(f)
            c2 = passes.cleanup_cfg(f)
            c3 = passes.dce(f)
            if chk:
                ssa.check_ssa(f)
            if not (c1 or c2 or c3):
                break

    scalar()
    if light:
        rng = Ranges(f, 6)
        passes2.range_simplify(f, rng)
        passes2.mem_pass(f, rng)
        passes2.licm(f, rng)
        passes.scalar_pass(f)
        passes.cleanup_cfg(f)
        return
    for _ in range(4):
        rng = Ranges(f)
        c1 = passes2.range_simplify(f, rng)
        if c1:
            scalar()
            rng = Ranges(f)
        c2 = passes2.mem_pass(f, rng)
        if c2:
            scalar()
            rng = Ranges(f)
        c3 = passes2.licm(f, rng)
        if chk:
            ssa.check_ssa(f)
        if c3:
            scalar()
        c4 = False
        if v.ifc:
            rng = Ranges(f)
            c4 = passes3.ifconv(f, rng, v.ifc)
            if chk:
                ssa.check_ssa(f)
            if c4:
                scalar()
        if not (c1 or c2 or c3 or c4):
            break


def midend(prog, mc, opts, v, light=False):
    """Everything up to (not including) the choice of block order and final code copying."""
    f = ir.from_program(prog)
    chk = opts.check
    ic = mc['icache_size']
    ssa.build_ssa(f)
    if chk:
        ssa.check_ssa(f)
    phase_a(f, v, chk, light)
    if v.peel:
        budget = ic if v.peel == 1 else max(4 * ic, 256)
        for _ in range(4):
            trips = passes3.find_const_trips(f)
            if not trips:
                break
            ssa.destruct_ssa(f)
            ch = passes3.peel_const_loops(f, trips, budget)
            ssa.build_ssa(f)
            phase_a(f, v, chk)
            if not ch:
                break
    if v.unroll > 1:
        ssa.destruct_ssa(f)
        size = ir.count_instrs(f)
        if backend.unroll(f, v.unroll, max(0, ic - size) if v.peel != 2 else max(ic, 2 * size)):
            backend.final_cleanup(f)
        ssa.build_ssa(f)
        phase_a(f, v, chk)
    rng = Ranges(f, 6 if light else 14)
    passes.dce(f, rng)
    passes.cleanup_cfg(f)
    # may we use the scratch region? only if the program provably never touches it
    scratch_ok = rng.ok
    if scratch_ok:
        for b in f.blocks:
            for ins in b.ins:
                if (ins.op == 'load' or ins.op == 'store') and rng.get(ins.a[0], b)[1] >= 3072:
                    scratch_ok = False
                    break
            if not scratch_ok:
                break
    passes2.lower(f, mc, v.mac)
    if v.sched:
        if not light:
            rng = Ranges(f)
        passes3.schedule(f, mc, rng)
    if chk:
        ssa.check_ssa(f)
    ir.analyze(f)
    for b in f.blocks:
        b.freq = float(min(8 ** b.depth, 10 ** 9))
    ssa.destruct_ssa(f)
    f.nreg = ssa.allocate(f, mc['registers'], v.spill and scratch_ok)
    backend.final_cleanup(f)
    return f


def finish(f, mc, profile, maxdup, factor, loose):
    """Code copying and block order for one candidate. Returns the program text."""
    size = ir.count_instrs(f)
    ic = mc['icache_size']
    if loose:
        budget = max(ic, 2 * size)
    else:
        budget = max(0, ic - size)
    if maxdup:
        backend.tail_dup(f, maxdup, budget)
        budget -= ir.count_instrs(f) - size
    if factor > 1:
        backend.unroll(f, factor, max(0, budget))
        backend.final_cleanup(f)
    backend.layout(f, profile)
    return ir.to_text(f)


BIG = 6000     # programs with more instructions than this get the light pipeline

BACKENDS = [
    # (maxdup, unroll factor, loose code-size budget)
    (4, 1, False),
    (0, 1, False),
    (12, 1, False),
    (4, 2, False),
    (4, 4, False),
    (4, 8, False),
    (12, 2, False),
    (4, 2, True),
    (12, 1, True),
]


class Search(object):
    def __init__(self, prog, mc, profile, opts, ref):
        self.prog = prog
        self.mc = mc
        self.profile = profile
        self.opts = opts
        self.ref = ref
        self.mids = {}
        self.results = []      # (score, size, order, text, cp, description)
        self.seen = {}
        self.n = 0
        self.size = len(prog.instrs)
        self.light = self.size > BIG
        self.spent = 0         # work done so far, in "instructions processed"

    def mid(self, v):
        k = v.key()
        if k not in self.mids:
            try:
                f = midend(self.prog, self.mc, self.opts, v, self.light)
                self.mids[k] = f
                self.spent += self.size + ir.count_instrs(f)
            except Exception as e:  # noqa
                if self.opts.debug:
                    raise
                sys.stderr.write('kopt: variant %r failed (%s: %s)\n' % (k, type(e).__name__, e))
                self.mids[k] = None
                self.spent += 2 * self.size
        return self.mids[k]

    def run(self, v, be):
        """Score one (variant, backend) pair. Returns the score or None."""
        f0 = self.mid(v)
        if f0 is None:
            return None
        self.n += 1
        self.spent += ir.count_instrs(f0) // 3
        try:
            f = ir.clone_func(f0)
            out = finish(f, self.mc, self.profile, be[0], be[1], be[2])
            if out in self.seen:
                return self.seen[out]
            newprog = K.parse_program(out)
            K.check_machine_features(newprog, self.mc)
            if len(newprog.params) != len(self.prog.params):
                raise K.KirError('parameter count changed')
            cp = K.compile_program(newprog)
            sc = self.ref.score(cp)
            if sc is None:
                raise K.KirError('behaves differently on a timing input')
            self.seen[out] = sc
            self.results.append((sc, len(newprog.instrs), self.n, out, cp, (v.key(), be)))
            return sc
        except Exception as e:  # noqa
            if self.opts.debug:
                raise
            sys.stderr.write('kopt: candidate %r %r failed (%s: %s)\n'
                             % (v.key(), be, type(e).__name__, e))
            return None


WORK_BUDGET = 16000    # the search stops when this much work has been spent


def optimize(text, mc, profile_text=None, opts=None):
    """Return the optimized program text. Falls back to the input text on any problem."""
    opts = opts or Options()
    opts.stats = {}
    prog = K.parse_program(text)          # raises KirError on an invalid input
    profile = backend.parse_profile(profile_text) if profile_text else None
    size = len(prog.instrs)
    try:
        ref = evalsim.Reference(prog, mc, size)
    except Exception:  # noqa
        if opts.debug:
            raise
        return text
    S = Search(prog, mc, profile, opts, ref)
    b0 = BACKENDS[0]
    if size > BIG:
        state = [Variant(peel=0, ifc=0), None]
    else:
        state = [Variant(), None]
    state[1] = S.run(state[0], b0)

    def consider(v):
        if S.spent > WORK_BUDGET:
            return
        sc = S.run(v, b0)
        if sc is not None and (state[1] is None or sc < state[1] - 1e-9):
            state[0], state[1] = v, sc

    def over_k(v):
        f0 = S.mids.get(v.key())
        return f0 is not None and f0.nreg > mc['registers']

    if size <= BIG:
        if over_k(state[0]):
            consider(state[0].but(spill=True))
        consider(Variant(peel=0, unroll=1, ifc=0, sched=False, spill=state[0].spill))
        for u in (2, 4, 8):
            consider(state[0].but(unroll=u))
        for ifc in (0, 2, 8, 24):
            consider(state[0].but(ifc=ifc))
        consider(state[0].but(sched=not state[0].sched))
        consider(state[0].but(mac=False))
        consider(state[0].but(peel=0 if state[0].peel else 1))
        consider(state[0].but(peel=2))
        consider(state[0].but(spill=not state[0].spill))
        for be in BACKENDS[1:]:
            if S.spent > WORK_BUDGET:
                break
            S.run(state[0], be)
    results = sorted(S.results, key=lambda r: (r[0], r[1], r[2]))
    for r in results:
        ok = False
        try:
            ok = ref.same(r[4])
        except Exception:  # noqa
            ok = False
        if ok:
            opts.stats['candidate'] = r[5]
            return r[3]
        if opts.debug:
            raise K.KirError('candidate %r behaves differently on the check inputs' % (r[5],))
        sys.stderr.write('kopt: candidate %r rejected by the self-check\n' % (r[5],))
    opts.stats['candidate'] = -1
    sys.stderr.write('kopt: writing the input unchanged\n')
    return text


USAGE = 'usage: kopt IN.kir --machine MACHINE.cfg [--profile PROFILE.prof] -o OUT.kir\n'


def main(argv):
    inp = None
    machine = None
    profile = None
    outp = None
    i = 0
    try:
        while i < len(argv):
            a = argv[i]
            if a == '--machine':
                machine = argv[i + 1]
                i += 2
            elif a == '--profile':
                profile = argv[i + 1]
                i += 2
            elif a == '-o':
                outp = argv[i + 1]
                i += 2
            elif inp is None and not a.startswith('-'):
                inp = a
                i += 1
            else:
                raise IndexError
    except IndexError:
        sys.stderr.write(USAGE)
        return 2
    if inp is None or machine is None or outp is None:
        sys.stderr.write(USAGE)
        return 2
    try:
        text = K.read_text(inp)
    except (IOError, OSError, UnicodeDecodeError) as e:
        sys.stderr.write('kopt: cannot read %s: %s\n' % (inp, e))
        return 2
    out = text
    try:
        mc = K.load_machine(machine)
        ptext = None
        if profile:
            try:
                ptext = K.read_text(profile)
            except (IOError, OSError, UnicodeDecodeError):
                ptext = None
        out = optimize(text, mc, ptext)
    except Exception as e:  # noqa
        sys.stderr.write('kopt: %s; writing the input unchanged\n' % e)
        out = text
    with open(outp, 'w', newline='\n') as fh:
        fh.write(out)
    return 0
