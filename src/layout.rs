//! layout.rs - block order as a directed travelling-salesman problem on the Kiln's own objective.
//!
//! The Kiln charges a taken jump (a `jmp`/`br` whose chosen target is not the next block in the
//! file) about P + 1/2 cycles, and every executed instruction at file index >= C about Q
//! cycles. Both costs depend only on the order, so for a block order `o`
//!
//!   cost(o) = pt * (W - sum over adjacent pairs (a, b) of w(a, b))
//!           + q  * sum over blocks b of cnt(b) * (number of instructions of b at index >= C)
//!
//! where w(a, b) is the number of times the edge a -> b is executed and W the total edge weight.
//! The position of a block depends only on the SET of blocks before it, so a Held-Karp dynamic
//! programme over subsets is exact for up to MAX_EXACT blocks. Above that we start from the
//! chain-merging order of `backend::layout` and improve it with exact-scored relocation moves.
//! Branch sense needs no extra work: `br c, A, B` may have either target next in the file.
//! Everything is pure re-ordering of whole blocks, so behaviour cannot change.
use crate::backend;
use crate::ir::*;
use crate::kir::*;
use std::collections::HashMap;

/// Largest block count (entry included) solved exactly.
pub const MAX_EXACT: usize = 16;
/// Largest block count for the local search (dense edge matrix).
const MAX_LOCAL: usize = 700;

/// Edge and block execution counts, by block id.
pub struct Counts {
    pub edge: HashMap<(usize, usize), f64>,
    pub block: HashMap<usize, f64>,
}

/// Count block transitions in traces of the program text produced from `f` in its current order.
pub fn dyn_counts(f: &Func, cp: &Compiled, traces: &[Vec<u32>]) -> Counts {
    let mut blk_of: Vec<usize> = Vec::new();
    let mut first: Vec<bool> = Vec::new();
    for &b in &f.blocks {
        for i in 0..f.arena[b].ins.len() {
            blk_of.push(b);
            first.push(i == 0);
        }
    }
    let mut c = Counts { edge: HashMap::new(), block: HashMap::new() };
    for tr in traces {
        for (i, &p) in tr.iter().enumerate() {
            let p = p as usize;
            if p >= blk_of.len() {
                continue;
            }
            if first[p] {
                *c.block.entry(blk_of[p]).or_insert(0.0) += 1.0;
            }
            if i + 1 < tr.len() && cp.opnames[p].is_term() {
                let q = tr[i + 1] as usize;
                if q < blk_of.len() {
                    *c.edge.entry((blk_of[p], blk_of[q])).or_insert(0.0) += 1.0;
                }
            }
        }
    }
    c
}

/// The cost model over local block indices 0..n (0 = entry).
pub struct Model {
    pub n: usize,
    pub size: Vec<usize>,
    pub cnt: Vec<f64>,
    pub w: Vec<Vec<f64>>, // dense: w[a][b] weight of edges a -> b
    pub wtot: f64,
    pub pt: f64,
    pub q: f64,
    pub c: usize,
}

impl Model {
    /// icache cost of block `b` placed with `off` instructions before it.
    #[inline]
    fn icost(&self, b: usize, off: usize) -> f64 {
        let end = off + self.size[b];
        if end <= self.c {
            return 0.0;
        }
        let over = (end - self.c).min(self.size[b]);
        self.cnt[b] * self.q * over as f64
    }

    pub fn eval(&self, order: &[usize]) -> f64 {
        let mut saved = 0.0;
        let mut off = 0usize;
        let mut ic = 0.0;
        for (i, &b) in order.iter().enumerate() {
            if i + 1 < order.len() {
                saved += self.w[b][order[i + 1]];
            }
            ic += self.icost(b, off);
            off += self.size[b];
        }
        self.pt * (self.wtot - saved) + ic
    }
}

/// Exact: entry (local 0) first, best order of the other blocks. Requires n <= MAX_EXACT.
pub fn held_karp(m: &Model) -> Vec<usize> {
    let n = m.n;
    if n <= 2 {
        return (0..n).collect();
    }
    let k = n - 1;
    let full = (1usize << k) - 1;
    let inf = f64::INFINITY;
    let mut dp = vec![inf; (full + 1) * k];
    let mut par = vec![u8::MAX; (full + 1) * k];
    let mut off = vec![0usize; full + 1];
    for mask in 1..=full {
        let low = mask.trailing_zeros() as usize;
        off[mask] = off[mask & (mask - 1)] + m.size[low + 1];
    }
    let s0 = m.size[0];
    for i in 0..k {
        dp[(1 << i) * k + i] = m.icost(i + 1, s0) - m.pt * m.w[0][i + 1];
    }
    for mask in 1..=full {
        let o = s0 + off[mask];
        for last in 0..k {
            if mask >> last & 1 == 0 {
                continue;
            }
            let cur = dp[mask * k + last];
            if cur == inf {
                continue;
            }
            for d in 0..k {
                if mask >> d & 1 != 0 {
                    continue;
                }
                let nm = mask | 1 << d;
                let c = cur + m.icost(d + 1, o) - m.pt * m.w[last + 1][d + 1];
                let slot = nm * k + d;
                if c < dp[slot] - 1e-12 {
                    dp[slot] = c;
                    par[slot] = last as u8;
                }
            }
        }
    }
    let mut best = 0;
    for l in 1..k {
        if dp[full * k + l] < dp[full * k + best] - 1e-12 {
            best = l;
        }
    }
    let mut rev = Vec::with_capacity(n);
    let (mut mask, mut cur) = (full, best);
    loop {
        rev.push(cur + 1);
        let p = par[mask * k + cur];
        mask &= !(1 << cur);
        if p == u8::MAX {
            break;
        }
        cur = p as usize;
    }
    rev.push(0);
    rev.reverse();
    rev
}

/// Relocation local search (segments of 1..=3 blocks moved anywhere), exact objective, first
/// improvement, deterministic, bounded work.
pub fn local_search(m: &Model, start: Vec<usize>, mut work: u64) -> Vec<usize> {
    let n = start.len();
    let mut ord = start;
    let mut best = m.eval(&ord);
    if n < 3 {
        return ord;
    }
    loop {
        let mut improved = false;
        'outer: for len in 1..=3usize {
            for i in 1..n {
                if i + len > n {
                    break;
                }
                let seg: Vec<usize> = ord[i..i + len].to_vec();
                let mut rest: Vec<usize> = ord[..i].to_vec();
                rest.extend_from_slice(&ord[i + len..]);
                for j in 1..=rest.len() {
                    if j == i {
                        continue;
                    }
                    if work < n as u64 {
                        return ord;
                    }
                    work -= n as u64;
                    let mut cand: Vec<usize> = Vec::with_capacity(n);
                    cand.extend_from_slice(&rest[..j]);
                    cand.extend_from_slice(&seg);
                    cand.extend_from_slice(&rest[j..]);
                    let c = m.eval(&cand);
                    if c < best - 1e-9 {
                        best = c;
                        ord = cand;
                        improved = true;
                        break 'outer;
                    }
                }
            }
        }
        if !improved {
            return ord;
        }
    }
}

/// Build the model for `f` (blocks in their current order) from dynamic counts blended with the
/// static estimate (and profile), and return a better block order, or None if the current one is
/// already best / the function is too large.
pub fn improve(f: &Func, mc: &Machine, counts: &Counts, profile: Option<&backend::Profile>) -> Option<Vec<usize>> {
    let n = f.blocks.len();
    if n < 3 || n > MAX_LOCAL {
        return None;
    }
    let idx: HashMap<usize, usize> = f.blocks.iter().enumerate().map(|(i, &b)| (b, i)).collect();
    let mut g = f.clone();
    backend::estimate_freq(&mut g, profile);
    let stat = backend::edge_weights(&g, profile);
    let dyn_tot: f64 = counts.edge.values().sum();
    let stat_tot: f64 = stat.iter().map(|e| e.0).sum::<f64>().max(1e-9);
    // static knowledge counts for a twentieth of the measured weight: it breaks ties and covers
    // edges the timing inputs never ran
    let eps = if dyn_tot > 0.0 { 0.05 * dyn_tot / stat_tot } else { 1.0 };
    let mut w = vec![vec![0.0f64; n]; n];
    let mut wtot = 0.0;
    for (&(a, b), &c) in &counts.edge {
        if let (Some(&x), Some(&y)) = (idx.get(&a), idx.get(&b)) {
            w[x][y] += c;
            wtot += c;
        }
    }
    for &(sw, _, a, b) in &stat {
        let (x, y) = (idx[&a], idx[&b]);
        w[x][y] += eps * sw;
        wtot += eps * sw;
    }
    let size: Vec<usize> = f.blocks.iter().map(|&b| f.arena[b].ins.len()).collect();
    let blk_tot: f64 = counts.block.values().sum();
    let freq_tot: f64 = f.blocks.iter().map(|&b| g.arena[b].freq).sum::<f64>().max(1e-9);
    let beps = if blk_tot > 0.0 { 0.05 * blk_tot / freq_tot } else { 1.0 };
    let cnt: Vec<f64> = f.blocks.iter().map(|&b| counts.block.get(&b).copied().unwrap_or(0.0) + beps * g.arena[b].freq).collect();
    let m = Model {
        n,
        size,
        cnt,
        w,
        wtot,
        pt: mc.taken_branch_penalty as f64 + 0.5,
        q: mc.icache_penalty as f64,
        c: mc.icache_size.max(0) as usize,
    };
    let seed: Vec<usize> = (0..n).collect();
    let seed_cost = m.eval(&seed);
    let ord = if n <= MAX_EXACT { held_karp(&m) } else { local_search(&m, seed, 40_000_000) };
    let c = m.eval(&ord);
    if c < seed_cost - 1e-9 {
        Some(ord.iter().map(|&i| f.blocks[i]).collect())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rnd_model(n: usize, seed: u64) -> Model {
        let mut r = Rng::new(seed);
        let size: Vec<usize> = (0..n).map(|_| 1 + r.below(6) as usize).collect();
        let cnt: Vec<f64> = (0..n).map(|_| r.below(50) as f64).collect();
        let mut w = vec![vec![0.0; n]; n];
        let mut wtot = 0.0;
        for a in 0..n {
            for _ in 0..2 {
                let b = r.below(n as u64) as usize;
                let c = r.below(100) as f64;
                w[a][b] += c;
                wtot += c;
            }
        }
        Model { n, size, cnt, w, wtot, pt: 2.5, q: 2.0, c: 8 }
    }

    fn brute(m: &Model) -> f64 {
        fn rec(m: &Model, ord: &mut Vec<usize>, used: &mut Vec<bool>, best: &mut f64) {
            if ord.len() == m.n {
                *best = best.min(m.eval(ord));
                return;
            }
            for b in 1..m.n {
                if !used[b] {
                    used[b] = true;
                    ord.push(b);
                    rec(m, ord, used, best);
                    ord.pop();
                    used[b] = false;
                }
            }
        }
        let mut best = f64::INFINITY;
        rec(m, &mut vec![0], &mut vec![false; m.n], &mut best);
        best
    }

    #[test]
    fn held_karp_is_optimal() {
        for seed in 0..30 {
            let n = 3 + (seed % 5) as usize;
            let m = rnd_model(n, seed);
            let o = held_karp(&m);
            assert_eq!(o[0], 0);
            let mut s = o.clone();
            s.sort();
            assert_eq!(s, (0..n).collect::<Vec<_>>());
            assert!((m.eval(&o) - brute(&m)).abs() < 1e-6, "seed {}", seed);
        }
    }

    #[test]
    fn local_search_never_worse_and_keeps_entry() {
        for seed in 0..10 {
            let m = rnd_model(30, seed);
            let seed_o: Vec<usize> = (0..30).collect();
            let o = local_search(&m, seed_o.clone(), 5_000_000);
            assert_eq!(o[0], 0);
            let mut s = o.clone();
            s.sort();
            assert_eq!(s, seed_o);
            assert!(m.eval(&o) <= m.eval(&seed_o) + 1e-9);
        }
    }
}
