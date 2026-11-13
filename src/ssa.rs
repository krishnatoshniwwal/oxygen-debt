//! ssa.rs - into SSA form and back out (with register colouring).
use crate::ir::*;
use crate::kir::*;
use std::cmp::Reverse;
use std::collections::BTreeMap;
use std::collections::BinaryHeap;

/// Fast deterministic hasher (Fx-style) for the integer-keyed sets and maps of liveness and the
/// interference graph; the std SipHash dominated the cost of `allocate` on large functions.
#[derive(Default, Clone, Copy)]
pub struct FxHasher(u64);
impl std::hash::Hasher for FxHasher {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.write_u64(b as u64);
        }
    }
    fn write_u32(&mut self, i: u32) {
        self.write_u64(i as u64);
    }
    fn write_i64(&mut self, i: i64) {
        self.write_u64(i as u64);
    }
    fn write_usize(&mut self, i: usize) {
        self.write_u64(i as u64);
    }
    fn write_u64(&mut self, i: u64) {
        self.0 = (self.0.rotate_left(5) ^ i).wrapping_mul(0x517c_c1b7_2722_0a95);
    }
}
pub type FxBuild = std::hash::BuildHasherDefault<FxHasher>;
type HashSet<T> = std::collections::HashSet<T, FxBuild>;
type HashMap<K, V> = std::collections::HashMap<K, V, FxBuild>;

// ---------------------------------------------------------------------------
// SSA construction (pruned, Cytron et al.), with copy and constant propagation
// done by the renaming itself.
// ---------------------------------------------------------------------------
pub fn build_ssa(f: &mut Func) {
    let mut entry = f.blocks[0];
    if !f.arena[entry].preds.is_empty() {
        let ne = f.new_block("entry");
        f.arena[ne].ins.push(Instr::no_d(Op::Jmp, vec![], vec![entry]));
        f.blocks.insert(0, ne);
        f.arena[entry].preds.push(ne);
        entry = ne;
    }
    let rpo = analyze(f);
    let nb = f.arena.len();

    // ---- liveness of the original registers (per block) ----
    let mut defs: Vec<HashSet<i64>> = vec![HashSet::default(); nb];
    let mut livein: Vec<HashSet<i64>> = vec![HashSet::default(); nb];
    let mut liveout: Vec<HashSet<i64>> = vec![HashSet::default(); nb];
    let mut defblocks: BTreeMap<i64, Vec<usize>> = BTreeMap::new();
    for &b in &rpo {
        let mut u: HashSet<i64> = HashSet::default();
        let mut d: HashSet<i64> = HashSet::default();
        let mut dorder: Vec<i64> = Vec::new();
        for ins in &f.arena[b].ins {
            for &x in &ins.a {
                if x >= 0 && !d.contains(&x) {
                    u.insert(x);
                }
            }
            if ins.has_d() && d.insert(ins.d) {
                dorder.push(ins.d);
            }
        }
        for v in dorder {
            defblocks.entry(v).or_default().push(b);
        }
        livein[b] = u;
        defs[b] = d;
    }
    let mut work: Vec<usize> = rpo.clone();
    let mut inwork = vec![false; nb];
    for &b in &rpo {
        inwork[b] = true;
    }
    while let Some(b) = work.pop() {
        inwork[b] = false;
        let n0 = liveout[b].len();
        let succs = f.arena[b].succs().to_vec();
        let mut add: Vec<i64> = Vec::new();
        for s in succs {
            add.extend(livein[s].iter().copied());
        }
        for x in add {
            liveout[b].insert(x);
        }
        if liveout[b].len() != n0 || n0 == 0 {
            let n1 = livein[b].len();
            let lo: Vec<i64> = liveout[b].iter().copied().filter(|x| !defs[b].contains(x)).collect();
            for x in lo {
                livein[b].insert(x);
            }
            if livein[b].len() != n1 {
                for k in 0..f.arena[b].preds.len() {
                    let p = f.arena[b].preds[k];
                    if !inwork[p] {
                        inwork[p] = true;
                        work.push(p);
                    }
                }
            }
        }
    }

    // ---- dominance frontiers ----
    let mut df: Vec<Vec<usize>> = vec![Vec::new(); nb];
    for &b in &rpo {
        if f.arena[b].preds.len() >= 2 {
            let bidom = f.arena[b].idom;
            for k in 0..f.arena[b].preds.len() {
                let mut r = f.arena[b].preds[k];
                while Some(r) != bidom {
                    let lst = &mut df[r];
                    if lst.last() == Some(&b) {
                        break; // an earlier predecessor of b already climbed through r and above
                    }
                    if !lst.contains(&b) {
                        lst.push(b);
                    }
                    r = f.arena[r].idom.unwrap();
                }
            }
        }
    }

    // ---- phi placement ----
    for (&v, dbs) in &defblocks {
        let mut has: HashSet<usize> = HashSet::default();
        let mut inw: HashSet<usize> = dbs.iter().copied().collect();
        let mut work: Vec<usize> = dbs.clone();
        while let Some(x) = work.pop() {
            for k in 0..df[x].len() {
                let y = df[x][k];
                if !has.contains(&y) && livein[y].contains(&v) {
                    has.insert(y);
                    let npreds = f.arena[y].preds.len();
                    let mut phi = Instr::new(Op::Phi, NOD, vec![L0; npreds], vec![]);
                    phi.pv = v;
                    f.arena[y].phis.push(phi);
                    if inw.insert(y) {
                        work.push(y);
                    }
                }
            }
        }
    }

    // ---- renaming ----
    let oldn = (f.nval.max(0) as usize) + 1;
    f.nval = 0;
    let mut stacks: Vec<Vec<i64>> = vec![Vec::new(); oldn];
    let mut newparams = Vec::new();
    for pi in 0..f.params.len() {
        let p = f.params[pi];
        let nv = f.new_val();
        newparams.push(nv);
        stacks[p as usize].push(nv);
    }
    f.params = newparams;
    // position of a predecessor in a block with many predecessors (a linear search per edge
    // would be quadratic for a join with tens of thousands of incoming edges)
    let mut pred_pos: HashMap<usize, HashMap<usize, usize>> = HashMap::default();
    let mut stack: Vec<(usize, Option<Vec<i64>>)> = vec![(entry, None)];
    while let Some((b, pushed)) = stack.pop() {
        if let Some(pv) = pushed {
            for v in pv {
                stacks[v as usize].pop();
            }
            continue;
        }
        let mut pushed: Vec<i64> = Vec::new();
        for pi in 0..f.arena[b].phis.len() {
            let nv = f.new_val();
            let v = f.arena[b].phis[pi].pv;
            stacks[v as usize].push(nv);
            pushed.push(v);
            f.arena[b].phis[pi].d = nv;
        }
        let old_ins = std::mem::take(&mut f.arena[b].ins);
        let mut out: Vec<Instr> = Vec::with_capacity(old_ins.len());
        for mut ins in old_ins {
            for x in ins.a.iter_mut() {
                if *x >= 0 {
                    *x = match stacks[*x as usize].last() {
                        Some(&t) => t,
                        None => L0,
                    };
                }
            }
            if !ins.has_d() {
                out.push(ins);
                continue;
            }
            let d = ins.d;
            let nv;
            match ins.op {
                Op::Const | Op::Mov => nv = ins.a[0],
                Op::Mac => {
                    let t = f.new_val();
                    out.push(Instr::new(Op::Mul, t, vec![ins.a[0], ins.a[1]], vec![]));
                    nv = f.new_val();
                    out.push(Instr::new(Op::Add, nv, vec![t, ins.a[2]], vec![]));
                }
                Op::Addsh => {
                    let t = f.new_val();
                    out.push(Instr::new(Op::Mul, t, vec![ins.a[1], lit(1i64 << (!ins.a[2]))], vec![]));
                    nv = f.new_val();
                    out.push(Instr::new(Op::Add, nv, vec![ins.a[0], t], vec![]));
                }
                _ => {
                    nv = f.new_val();
                    ins.d = nv;
                    out.push(ins);
                }
            }
            stacks[d as usize].push(nv);
            pushed.push(d);
        }
        f.arena[b].ins = out;
        let succs = f.arena[b].succs().to_vec();
        for sb in succs {
            if !f.arena[sb].phis.is_empty() {
                let idx = if f.arena[sb].preds.len() > 16 {
                    let m = pred_pos.entry(sb).or_insert_with(|| f.arena[sb].preds.iter().enumerate().map(|(i, &p)| (p, i)).collect());
                    m[&b]
                } else {
                    f.arena[sb].preds.iter().position(|&p| p == b).unwrap()
                };
                for pi in 0..f.arena[sb].phis.len() {
                    let v = f.arena[sb].phis[pi].pv;
                    f.arena[sb].phis[pi].a[idx] = match stacks[v as usize].last() {
                        Some(&t) => t,
                        None => L0,
                    };
                }
            }
        }
        stack.push((b, Some(pushed)));
        for k in (0..f.arena[b].dkids.len()).rev() {
            stack.push((f.arena[b].dkids[k], None));
        }
    }
}

// ---------------------------------------------------------------------------
// Out of SSA
// ---------------------------------------------------------------------------
/// Replace every phi by copies through a fresh name (Sreedhar's method I).
pub fn destruct_ssa(f: &mut Func) {
    for bi in 0..f.blocks.len() {
        let b = f.blocks[bi];
        if f.arena[b].phis.is_empty() {
            continue;
        }
        let phis = std::mem::take(&mut f.arena[b].phis);
        let npreds = f.arena[b].preds.len();
        let mut heads: Vec<Instr> = Vec::new();
        let mut tails: Vec<Vec<Instr>> = vec![Vec::new(); npreds];
        for phi in &phis {
            let x = f.new_val();
            heads.push(Instr::new(Op::Mov, phi.d, vec![x], vec![]));
            for (i, &a) in phi.a.iter().enumerate() {
                tails[i].push(Instr::new(Op::Mov, x, vec![a], vec![]));
            }
        }
        for i in 0..npreds {
            let p = f.arena[b].preds[i];
            let ins = &mut f.arena[p].ins;
            let at = ins.len() - 1;
            let tl = std::mem::take(&mut tails[i]);
            ins.splice(at..at, tl);
        }
        f.arena[b].ins.splice(0..0, heads);
    }
}

/// Per-block live-in / live-out sets for a non-SSA function (indexed by block id).
fn liveness(f: &Func) -> (Vec<HashSet<i64>>, Vec<HashSet<i64>>) {
    let nb = f.arena.len();
    let mut dfn: Vec<HashSet<i64>> = vec![HashSet::default(); nb];
    let mut livein: Vec<HashSet<i64>> = vec![HashSet::default(); nb];
    let mut liveout: Vec<HashSet<i64>> = vec![HashSet::default(); nb];
    for &b in &f.blocks {
        let mut u: HashSet<i64> = HashSet::default();
        let mut d: HashSet<i64> = HashSet::default();
        for ins in &f.arena[b].ins {
            for &x in &ins.a {
                if x >= 0 && !d.contains(&x) {
                    u.insert(x);
                }
            }
            if ins.has_d() {
                d.insert(ins.d);
            }
        }
        livein[b] = u;
        dfn[b] = d;
    }
    let mut lwork: u64 = 0;
    let mut work: Vec<usize> = f.blocks.clone();
    let mut inwork = vec![false; nb];
    let mut first = vec![false; nb];
    for &b in &f.blocks {
        inwork[b] = true;
        first[b] = true;
    }
    while let Some(b) = work.pop() {
        inwork[b] = false;
        let n0 = liveout[b].len();
        lwork += 4 + livein[b].len() as u64 + liveout[b].len() as u64;
        let mut add: Vec<i64> = Vec::new();
        for &s in f.arena[b].succs() {
            add.extend(livein[s].iter().copied());
        }
        for x in add {
            liveout[b].insert(x);
        }
        if liveout[b].len() != n0 || first[b] {
            first[b] = false;
            let n1 = livein[b].len();
            let lo: Vec<i64> = liveout[b].iter().copied().filter(|x| !dfn[b].contains(x)).collect();
            for x in lo {
                livein[b].insert(x);
            }
            if livein[b].len() != n1 {
                for &p in &f.arena[b].preds {
                    if !inwork[p] {
                        inwork[p] = true;
                        work.push(p);
                    }
                }
            }
        }
    }
    crate::ranges::add_work(lwork / 8);
    (liveout, livein)
}

const MAX_EDGES: usize = 40_000_000;

struct Graph {
    nodes: Vec<i64>,
    adj: Vec<HashSet<u32>>,
    present: Vec<bool>,
    parent: Vec<i64>,
    cost: Vec<f64>,
}

impl Graph {
    fn find(&mut self, x: i64) -> i64 {
        let mut r = x;
        while self.parent[r as usize] >= 0 {
            r = self.parent[r as usize];
        }
        let mut x = x;
        while self.parent[x as usize] >= 0 {
            let nx = self.parent[x as usize];
            self.parent[x as usize] = r;
            x = nx;
        }
        r
    }
}

/// Interference graph of a non-SSA function, with copies coalesced.
fn build_graph(f: &mut Func) -> Result<Graph, String> {
    compute_preds(f);
    let (liveout, _) = liveness(f);
    let n = f.nval.max(0) as usize + 1;
    let mut g = Graph { nodes: Vec::new(), adj: vec![HashSet::default(); n], present: vec![false; n], parent: vec![-1; n], cost: vec![0.0; n] };
    fn node(g: &mut Graph, v: i64) {
        if !g.present[v as usize] {
            g.present[v as usize] = true;
            g.nodes.push(v);
        }
    }
    for &p in &f.params {
        node(&mut g, p);
    }
    for i in 0..f.params.len() {
        for j in 0..i {
            let (p, q) = (f.params[i], f.params[j]);
            g.adj[p as usize].insert(q as u32);
            g.adj[q as usize].insert(p as u32);
        }
    }
    let mut copies: Vec<(f64, i64, i64)> = Vec::new();
    let entry = f.blocks[0];
    let mut ninstr = 0usize;
    for &b in &f.blocks {
        let w = f.arena[b].freq;
        ninstr += f.arena[b].ins.len();
        for ins in &f.arena[b].ins {
            if ins.has_d() {
                node(&mut g, ins.d);
                g.cost[ins.d as usize] += w;
            }
            for &x in &ins.a {
                if x >= 0 {
                    node(&mut g, x);
                    g.cost[x as usize] += w;
                }
            }
        }
    }
    if ninstr > 20000 {
        let mut est = 0usize;
        for &b in &f.blocks {
            let mut live: HashSet<i64> = liveout[b].clone();
            for ins in f.arena[b].ins.iter().rev() {
                if ins.has_d() {
                    est += live.len();
                    live.remove(&ins.d);
                }
                for &x in &ins.a {
                    if x >= 0 {
                        live.insert(x);
                    }
                }
            }
        }
        if est > MAX_EDGES {
            return Err(format!("interference graph too large ({})", est));
        }
    }
    let mut gwork: u64 = 0;
    for &b in &f.blocks {
        let mut live: HashSet<i64> = liveout[b].clone();
        gwork += liveout[b].len() as u64;
        let w = f.arena[b].freq;
        for ins in f.arena[b].ins.iter().rev() {
            if ins.has_d() {
                let d = ins.d;
                gwork += 2 * live.len() as u64 + 2;
                let skip = if ins.op == Op::Mov && ins.a[0] >= 0 {
                    copies.push((w, d, ins.a[0]));
                    ins.a[0]
                } else {
                    -1
                };
                for &l in &live {
                    if l != d && l != skip {
                        g.adj[d as usize].insert(l as u32);
                        g.adj[l as usize].insert(d as u32);
                    }
                }
                live.remove(&d);
            }
            for &x in &ins.a {
                if x >= 0 {
                    live.insert(x);
                }
            }
        }
        if b == entry {
            for &p in &f.params {
                for &l in &live {
                    if l != p {
                        g.adj[p as usize].insert(l as u32);
                        g.adj[l as usize].insert(p as u32);
                    }
                }
            }
        }
    }

    crate::ranges::add_work(gwork / 8);
    // ---- coalescing (aggressive, hottest copies first) ----
    let mut order: Vec<usize> = (0..copies.len()).collect();
    order.sort_by(|&i, &j| copies[j].0.partial_cmp(&copies[i].0).unwrap().then(i.cmp(&j)));
    for i in order {
        let (w, d, s) = copies[i];
        let mut rd = g.find(d);
        let mut rs = g.find(s);
        if rd == rs || g.adj[rd as usize].contains(&(rs as u32)) {
            continue;
        }
        if g.adj[rd as usize].len() < g.adj[rs as usize].len() {
            std::mem::swap(&mut rd, &mut rs);
        }
        let nbrs = std::mem::take(&mut g.adj[rs as usize]);
        for &nn in &nbrs {
            let an = &mut g.adj[nn as usize];
            an.remove(&(rs as u32));
            an.insert(rd as u32);
            g.adj[rd as usize].insert(nn);
        }
        g.present[rs as usize] = false;
        g.parent[rs as usize] = rd;
        g.cost[rd as usize] += g.cost[rs as usize] - 2.0 * w;
    }
    Ok(g)
}

/// Greedy colouring, smallest-degree-last order. Returns (colour per node, number of colours).
fn colour(g: &Graph) -> (Vec<i32>, i32) {
    let live_nodes: Vec<i64> = g.nodes.iter().copied().filter(|&v| g.present[v as usize]).collect();
    let stack: Vec<i64>;
    if live_nodes.len() > 4000 {
        stack = live_nodes.iter().rev().copied().collect();
    } else {
        let mut deg: Vec<usize> = vec![0; g.adj.len()];
        let mut heap: BinaryHeap<Reverse<(usize, i64)>> = BinaryHeap::new();
        for &v in &live_nodes {
            deg[v as usize] = g.adj[v as usize].len();
            heap.push(Reverse((deg[v as usize], v)));
        }
        let mut removed = vec![false; g.adj.len()];
        let mut st = Vec::new();
        while let Some(Reverse((dg, v))) = heap.pop() {
            if removed[v as usize] || dg != deg[v as usize] {
                continue;
            }
            removed[v as usize] = true;
            st.push(v);
            for &nn in &g.adj[v as usize] {
                if !removed[nn as usize] {
                    deg[nn as usize] -= 1;
                    heap.push(Reverse((deg[nn as usize], nn as i64)));
                }
            }
        }
        stack = st;
    }
    let mut col: Vec<i32> = vec![-1; g.adj.len()];
    let mut ncol = 0;
    let mut used: Vec<bool> = Vec::new();
    for &v in stack.iter().rev() {
        used.clear();
        used.resize(g.adj[v as usize].len() + 2, false);
        for &nn in &g.adj[v as usize] {
            let c = col[nn as usize];
            if c >= 0 && (c as usize) < used.len() {
                used[c as usize] = true;
            }
        }
        let mut c = 0;
        while used[c] {
            c += 1;
        }
        col[v as usize] = c as i32;
        if c as i32 + 1 > ncol {
            ncol = c as i32 + 1;
        }
    }
    (col, ncol)
}

/// Greedy colouring in order of decreasing cost (frequency-weighted touches): the hottest
/// values get the lowest names. Used when more than K names are needed anyway: the
/// hardware register file is LRU (KIR_SPEC section 10), so what costs is touching many
/// different names inside a hot loop, not the total number of names. Giving the hot values
/// a small set of names keeps them resident.
fn colour_hot(g: &Graph) -> (Vec<i32>, i32) {
    let mut nodes: Vec<i64> = g.nodes.iter().copied().filter(|&v| g.present[v as usize]).collect();
    nodes.sort_by(|&a, &b| g.cost[b as usize].partial_cmp(&g.cost[a as usize]).unwrap().then(a.cmp(&b)));
    let mut col: Vec<i32> = vec![-1; g.adj.len()];
    let mut ncol = 0;
    let mut used: Vec<bool> = Vec::new();
    for &v in &nodes {
        used.clear();
        used.resize(g.adj[v as usize].len() + 2, false);
        for &nn in &g.adj[v as usize] {
            let c = col[nn as usize];
            if c >= 0 && (c as usize) < used.len() {
                used[c as usize] = true;
            }
        }
        let mut c = 0;
        while used[c] {
            c += 1;
        }
        col[v as usize] = c as i32;
        if c as i32 + 1 > ncol {
            ncol = c as i32 + 1;
        }
    }
    (col, ncol)
}

/// Chaitin-style: remove nodes of degree < k; when stuck, spill the cheapest node.
fn pick_spills(g: &Graph, k: usize, nospill: &HashSet<i64>) -> Vec<i64> {
    let live_nodes: Vec<i64> = g.nodes.iter().copied().filter(|&v| g.present[v as usize]).collect();
    let mut deg: Vec<usize> = vec![0; g.adj.len()];
    for &v in &live_nodes {
        deg[v as usize] = g.adj[v as usize].len();
    }
    let mut removed = vec![false; g.adj.len()];
    let mut spills = Vec::new();
    let mut low: Vec<i64> = live_nodes.iter().copied().filter(|&v| deg[v as usize] < k).collect();
    let mut remaining = live_nodes.len();
    while remaining > 0 {
        while let Some(v) = low.pop() {
            if removed[v as usize] {
                continue;
            }
            removed[v as usize] = true;
            remaining -= 1;
            for &nn in &g.adj[v as usize] {
                if !removed[nn as usize] {
                    deg[nn as usize] -= 1;
                    if deg[nn as usize] + 1 == k {
                        low.push(nn as i64);
                    }
                }
            }
        }
        if remaining == 0 {
            break;
        }
        let mut best: Option<i64> = None;
        let mut bestkey = (0.0f64, 0i64);
        for &v in &live_nodes {
            if removed[v as usize] || nospill.contains(&v) {
                continue;
            }
            let key = (g.cost[v as usize] / (deg[v as usize] + 1) as f64, v);
            if best.is_none() || key.0 < bestkey.0 || (key.0 == bestkey.0 && key.1 < bestkey.1) {
                bestkey = key;
                best = Some(v);
            }
        }
        match best {
            None => {
                for &v in &live_nodes {
                    if !removed[v as usize] {
                        removed[v as usize] = true;
                        remaining -= 1;
                    }
                }
                break;
            }
            Some(b) => {
                spills.push(b);
                removed[b as usize] = true;
                remaining -= 1;
                for &nn in &g.adj[b as usize] {
                    if !removed[nn as usize] {
                        deg[nn as usize] -= 1;
                        if deg[nn as usize] + 1 == k {
                            low.push(nn as i64);
                        }
                    }
                }
            }
        }
    }
    spills
}

/// Keep every spilled register in a scratch-memory word: store after each definition,
/// load before each use (through a fresh short-lived register).
fn rewrite_spills(f: &mut Func, g: &mut Graph, slot_of: &HashMap<i64, usize>, nospill: &mut HashSet<i64>, reuse: usize) {
    for bi in 0..f.blocks.len() {
        let b = f.blocks[bi];
        let old = std::mem::take(&mut f.arena[b].ins);
        let mut out: Vec<Instr> = Vec::with_capacity(old.len());
        // reuse > 0: the last `reuse` values loaded from (or stored to) a scratch word in this block
        // stay available for later uses of that word until it is written again
        let mut cache: Vec<(usize, i64)> = Vec::new();
        for mut ins in old {
            let dslot = if ins.has_d() { slot_of.get(&g.find(ins.d)).copied() } else { None };
            if let Some(ds) = dslot {
                if ins.op == Op::Mov || ins.op == Op::Const {
                    cache.retain(|e| e.0 != ds); // written below without a temp of its own
                }
            }
            if ins.op == Op::Mov && dslot.is_some() {
                let s = ins.a[0];
                let sslot = if s >= 0 { slot_of.get(&g.find(s)).copied() } else { None };
                match sslot {
                    None => {
                        out.push(Instr::no_d(Op::Store, vec![lit(dslot.unwrap() as i64), s], vec![]));
                        continue;
                    }
                    Some(ss) if ss == dslot.unwrap() => continue,
                    _ => {}
                }
            }
            if ins.op == Op::Const && dslot.is_some() {
                out.push(Instr::no_d(Op::Store, vec![lit(dslot.unwrap() as i64), ins.a[0]], vec![]));
                continue;
            }
            let mut loaded: HashMap<usize, i64> = HashMap::default();
            for i in 0..ins.a.len() {
                let x = ins.a[i];
                if x >= 0 {
                    if let Some(sl) = slot_of.get(&g.find(x)).copied() {
                        let cached = if reuse > 0 { cache.iter().find(|e| e.0 == sl).map(|e| e.1) } else { None };
                        let t = match loaded.get(&sl).copied().or(cached) {
                            Some(t) => t,
                            None => {
                                let t = f.new_val();
                                nospill.insert(t);
                                loaded.insert(sl, t);
                                out.push(Instr::new(Op::Load, t, vec![lit(sl as i64)], vec![]));
                                if reuse > 0 {
                                    cache.push((sl, t));
                                    if cache.len() > reuse {
                                        cache.remove(0);
                                    }
                                }
                                t
                            }
                        };
                        ins.a[i] = t;
                    }
                }
            }
            if let Some(ds) = dslot {
                let t = f.new_val();
                nospill.insert(t);
                ins.d = t;
                out.push(ins);
                out.push(Instr::no_d(Op::Store, vec![lit(ds as i64), t], vec![]));
                if reuse > 0 {
                    cache.retain(|e| e.0 != ds);
                    cache.push((ds, t));
                    if cache.len() > reuse {
                        cache.remove(0);
                    }
                }
            } else {
                out.push(ins);
            }
        }
        f.arena[b].ins = out;
    }
    // spilled parameters: save them at the very start
    let mut pre: Vec<Instr> = Vec::new();
    for i in 0..f.params.len() {
        let p = f.params[i];
        if let Some(sl) = slot_of.get(&g.find(p)).copied() {
            let t = f.new_val();
            nospill.insert(t);
            f.params[i] = t;
            pre.push(Instr::no_d(Op::Store, vec![lit(sl as i64), t], vec![]));
        }
    }
    if !pre.is_empty() {
        let entry = f.blocks[0];
        if !f.arena[entry].preds.is_empty() {
            let ne = f.new_block("entry");
            let mut ins = pre;
            ins.push(Instr::no_d(Op::Jmp, vec![], vec![entry]));
            f.arena[ne].ins = ins;
            f.arena[ne].freq = 1.0;
            f.blocks.insert(0, ne);
        } else {
            f.arena[entry].ins.splice(0..0, pre);
        }
    }
}

pub const SCRATCH_WORDS: usize = 1024;

/// Colour the registers of a non-SSA function with as few names as possible. Copies are
/// coalesced first. With spill=true and more names than k, values are moved to scratch
/// memory until k names are enough (the caller has checked that the program itself never
/// touches the scratch region). hot=true: if more than k names are still needed, colour the
/// hottest values first instead (see colour_hot). force=n>0 (only when the caller has checked
/// that scratch memory is free): if more than k names are needed, first keep the n-th value
/// of the hottest blocks' live-in values (least-touched first) in scratch memory. Which one
/// pays is measured by the search. Returns the number of names.
pub fn allocate(f: &mut Func, k: Option<usize>, spill: bool, hot: bool, force: usize, reuse: usize, spillk: usize) -> Result<i64, String> {
    let mut nospill: HashSet<i64> = HashSet::default();
    let mut nslots = 0usize;
    if (force > 0 || spillk > 0) && k.is_some() {
        let mut gg = build_graph(f)?;
        let (_, nc) = colour(&gg);
        if nc as usize > k.unwrap() {
            let (_, livein) = liveness(f);
            let maxf = f.blocks.iter().map(|&b| f.arena[b].freq).fold(0.0f64, f64::max);
            let mut cand: std::collections::BTreeSet<i64> = std::collections::BTreeSet::new();
            for bi in 0..f.blocks.len() {
                let b = f.blocks[bi];
                if f.arena[b].freq >= maxf {
                    for &x in &livein[b] {
                        cand.insert(gg.find(x));
                    }
                }
            }
            let mut cl: Vec<i64> = cand.into_iter().filter(|&v| gg.present[v as usize]).collect();
            cl.sort_by(|&a, &b| gg.cost[a as usize].partial_cmp(&gg.cost[b as usize]).unwrap().then(a.cmp(&b)));
            let picks: Vec<i64> = if spillk > 0 { cl.iter().take(spillk).copied().collect() } else { cl.get(force - 1).copied().into_iter().collect() };
            if !picks.is_empty() {
                let mut slot_of: HashMap<i64, usize> = HashMap::default();
                for v in picks {
                    slot_of.insert(v, SCRATCH_BASE + nslots);
                    nslots += 1;
                }
                rewrite_spills(f, &mut gg, &slot_of, &mut nospill, reuse);
            }
        }
    }
    let mut rounds = 0;
    let (mut g, col, ncol);
    loop {
        let mut gg = build_graph(f)?;
        let (cc, nc) = colour(&gg);
        if !spill || k.is_none() || nc as usize <= k.unwrap() || rounds >= 8 {
            g = gg;
            col = cc;
            ncol = nc;
            break;
        }
        rounds += 1;
        let spills = pick_spills(&gg, k.unwrap(), &nospill);
        if spills.is_empty() || nslots + spills.len() > SCRATCH_WORDS {
            g = gg;
            col = cc;
            ncol = nc;
            break;
        }
        let mut slot_of: HashMap<i64, usize> = HashMap::default();
        for v in spills {
            slot_of.insert(v, SCRATCH_BASE + nslots);
            nslots += 1;
        }
        rewrite_spills(f, &mut gg, &slot_of, &mut nospill, reuse);
    }
    let (col, ncol) = match k {
        Some(kk) if hot && ncol as usize > kk => colour_hot(&g),
        _ => (col, ncol),
    };
    let mut colf = |x: i64| -> i64 {
        let r = g.find(x);
        col[r as usize] as i64
    };
    for i in 0..f.params.len() {
        f.params[i] = colf(f.params[i]);
    }
    for bi in 0..f.blocks.len() {
        let b = f.blocks[bi];
        let old = std::mem::take(&mut f.arena[b].ins);
        let mut out = Vec::with_capacity(old.len());
        for mut ins in old {
            for i in 0..ins.a.len() {
                if ins.a[i] >= 0 {
                    ins.a[i] = colf(ins.a[i]);
                }
            }
            if ins.has_d() {
                ins.d = colf(ins.d);
                if ins.op == Op::Mov {
                    if ins.a[0] == ins.d {
                        continue;
                    }
                    if ins.a[0] < 0 {
                        ins.op = Op::Const;
                    }
                }
            }
            out.push(ins);
        }
        f.arena[b].ins = out;
    }
    f.nval = ncol as i64;
    Ok(ncol as i64)
}

/// Debug helper: every value has one definition that dominates its uses.
pub fn check_ssa(f: &mut Func) -> Result<(), String> {
    analyze(f);
    let mut defblk: HashMap<i64, usize> = HashMap::default();
    let mut pos: HashMap<i64, i64> = HashMap::default();
    let entry = f.blocks[0];
    for &p in &f.params {
        defblk.insert(p, entry);
        pos.insert(p, -1);
    }
    for &b in &f.blocks {
        let blk = &f.arena[b];
        for phi in &blk.phis {
            if defblk.insert(phi.d, b).is_some() {
                return Err(format!("double def v{}", phi.d));
            }
            pos.insert(phi.d, -1);
            if phi.a.len() != blk.preds.len() {
                return Err(format!("phi arity in {}", blk.name));
            }
        }
        for (i, ins) in blk.ins.iter().enumerate() {
            if ins.has_d() {
                if defblk.insert(ins.d, b).is_some() {
                    return Err(format!("double def v{}", ins.d));
                }
                pos.insert(ins.d, i as i64);
            }
        }
    }
    for &b in &f.blocks {
        let blk = &f.arena[b];
        for &s in blk.succs() {
            if !f.arena[s].preds.contains(&b) {
                return Err(format!("pred missing {}->{}", blk.name, f.arena[s].name));
            }
        }
        for &p in &blk.preds {
            if !f.arena[p].succs().contains(&b) {
                return Err(format!("stale pred {}->{}", f.arena[p].name, blk.name));
            }
        }
        for phi in &blk.phis {
            for (&x, &p) in phi.a.iter().zip(blk.preds.iter()) {
                if x >= 0 {
                    match defblk.get(&x) {
                        None => return Err(format!("undefined v{} in phi", x)),
                        Some(&db) => {
                            if !f.dominates(db, p) {
                                return Err(format!("phi arg v{} does not dominate pred in {}", x, blk.name));
                            }
                        }
                    }
                }
            }
        }
        for (i, ins) in blk.ins.iter().enumerate() {
            for &x in &ins.a {
                if x >= 0 {
                    match defblk.get(&x) {
                        None => return Err(format!("undefined v{} in {}", x, blk.name)),
                        Some(&db) => {
                            if db == b {
                                if pos[&x] >= i as i64 {
                                    return Err(format!("use before def v{} in {}", x, blk.name));
                                }
                            } else if !f.dominates(db, b) {
                                return Err(format!("def of v{} does not dominate use in {}", x, blk.name));
                            }
                        }
                    }
                }
            }
            if i + 1 < blk.ins.len() && ins.op.is_term() {
                return Err("terminator in the middle of a block".into());
            }
        }
        if blk.ins.is_empty() || !blk.ins.last().unwrap().op.is_term() {
            return Err(format!("block {} has no terminator", blk.name));
        }
    }
    Ok(())
}
