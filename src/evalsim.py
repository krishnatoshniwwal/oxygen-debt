"""evalsim.py - kopt's own measuring stick.

kopt runs the input program and each candidate output on a fixed set of made-up inputs
(with the simulator code copied from the kit). It uses this for two things:
  * to pick the fastest of several candidate outputs (the timing model is exact);
  * as a safety net: a candidate that behaves differently from the input is thrown away.
Everything here is deterministic (fixed seeds, step budgets instead of clocks).
"""
import kirlib as K


def _timing_inputs(nparams, count):
    rng = K.Rng(987654321)
    out = []
    for i in range(count):
        args = [8 + rng.below(40) for _ in range(nparams)]
        bound = (256, 100, 16, 1000)[i % 4]
        words = [rng.below(bound) for _ in range(K.GEN_MEM_WORDS)]
        vin = {}
        for port in range(4):
            vin[port] = [rng.below(1000) for _ in range(24)]
        out.append(K.Inputs(args, [(0, words)], vin))
    return out


class Reference(object):
    """The input program, compiled, with its behaviour on the test inputs."""

    def __init__(self, prog, mc, size):
        self.mc = mc
        self.cp = K.compile_program(prog)
        self.checks = []     # (inputs, result)
        self.timing = []     # (inputs, result, cycles)
        nparams = len(prog.params)
        if size > 20000:
            ncheck, ntime, cap, total = 6, 1, 4 * size + 1000, 6 * size + 100000
        elif size > 2000:
            ncheck, ntime, cap, total = 24, 2, 40000, 300000
        else:
            ncheck, ntime, cap, total = 56, 4, 20000, 400000
        self.cap = cap
        used = 0
        tsteps = 0
        for inp in _timing_inputs(nparams, ntime):
            if self.timing and tsteps > 12000:
                break
            res = K.execute(self.cp, inp.args, inp.segments, inp.vin, cap, want_trace=True)
            used += res.steps
            if res.status == 'TIMEOUT':
                continue
            if self.timing and tsteps + res.steps > 16000:
                continue
            tsteps += res.steps
            cyc = K.time_trace(self.cp, res.trace, mc)
            res.trace = None
            self.timing.append((inp, res, cyc))
        for inp in K.generate_inputs(nparams, ncheck, 4242):
            if used > total:
                break
            res = K.execute(self.cp, inp.args, inp.segments, inp.vin, cap)
            used += res.steps
            if res.status == 'TIMEOUT':
                continue
            self.checks.append((inp, res))

    def score(self, cp):
        """Sum over the timing inputs of (candidate cycles / original cycles). None = wrong."""
        if not self.timing:
            return 0.0
        tot = 0.0
        for inp, ref, cyc in self.timing:
            res = K.execute(cp, inp.args, inp.segments, inp.vin, 4 * ref.steps + 1000, want_trace=True)
            if res.status == 'TIMEOUT' or K.first_difference(ref, res):
                return None
            tot += float(K.time_trace(cp, res.trace, self.mc)) / cyc
        return tot

    def same(self, cp):
        """True if the candidate behaves like the original on all check inputs."""
        for inp, ref in self.checks:
            res = K.execute(cp, inp.args, inp.segments, inp.vin, 4 * ref.steps + 1000)
            if res.status == 'TIMEOUT' or K.first_difference(ref, res):
                return False
        return True
