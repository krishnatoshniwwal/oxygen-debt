//! sccp.rs - optimistic sparse conditional constant propagation on SSA form.
//!
//! Every value is Top (not computed yet: its block has not been reached), a constant, or
//! Bottom (not a known constant). Blocks and CFG edges start unreachable; a branch whose
//! condition is a constant (or that the range analysis decides) makes only one edge
//! reachable. A phi only looks at reachable incoming edges, so a loop-carried value that
//! is the same constant on entry and on every reachable back edge stays a constant, which
//! the plain value-numbering pass cannot see.
//!
//! The result is applied by replacing constants by literals, turning decided branches into
//! jumps and deleting the blocks that became unreachable. Values are only treated as
//! constants when the instruction cannot trap in a way that matters: `div`/`mod` give a
//! constant only when both operands are constants and the divisor is not 0.
use crate::ir::*;
use crate::kir::*;
use crate::passes::{apply_rep, evalop, NOREP};
use crate::ranges::Ranges;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum L {
    Top,
    C(i64),
    Bot,
}

fn meet(a: L, b: L) -> L {
    match (a, b) {
        (L::Top, x) | (x, L::Top) => x,
        (L::C(x), L::C(y)) if x == y => L::C(x),
        _ => L::Bot,
    }
}

/// Abstract transfer of a pure binary operation (exposed for the soundness tests).
pub fn bin(op: Op, x: L, y: L, same: bool) -> L {
    if let (L::C(a), L::C(b)) = (x, y) {
        return match evalop(op, a, b) {
            Some(v) => L::C(v),
            None => L::Bot,
        };
    }
    // absorbing elements (also when the other side is still Top)
    let z = |v: L, k: i64| v == L::C(k);
    match op {
        Op::Mul | Op::And if z(x, 0) || z(y, 0) => return L::C(0),
        Op::Or if z(x, MASK) || z(y, MASK) => return L::C(MASK),
        Op::Shl | Op::Shr if z(x, 0) => return L::C(0),
        _ => {}
    }
    if x == L::Top || y == L::Top {
        return L::Top;
    }
    if same {
        match op {
            Op::Sub | Op::Xor | Op::Lt => return L::C(0),
            Op::Eq => return L::C(1),
            _ => {}
        }
    }
    if op == Op::Mod && z(y, 1) {
        return L::C(0);
    }
    L::Bot
}

pub struct Sccp {
    pub val: Vec<L>,
    pub exec: Vec<bool>,
    /// per block: which of the terminator's successors can be taken (index into `t`)
    pub taken: Vec<[bool; 2]>,
}

fn opv(val: &[L], x: i64) -> L {
    if x < 0 {
        L::C(!x)
    } else {
        val[x as usize]
    }
}

/// The values of a select: condition c, then p, else q.
pub fn sel(c: L, p: L, q: L) -> L {
    match c {
        L::Top => L::Top,
        L::C(k) => {
            if k != 0 {
                p
            } else {
                q
            }
        }
        L::Bot => meet(p, q),
    }
}

impl Sccp {
    pub fn run(f: &mut Func, rng: Option<&Ranges>) -> Option<Sccp> {
        analyze(f);
        let nv = f.nval.max(0) as usize + 1;
        let nb = f.arena.len();
        let mut s = Sccp { val: vec![L::Top; nv], exec: vec![false; nb], taken: vec![[false; 2]; nb] };
        for &p in &f.params {
            s.val[p as usize] = L::Bot;
        }
        let rng = rng.filter(|r| r.ok);
        s.exec[f.blocks[0]] = true;
        let rpo = f.rpo.clone();
        let mut settled = false;
        for _ in 0..64 {
            let mut changed = false;
            for &b in &rpo {
                if !s.exec[b] {
                    continue;
                }
                let blk = &f.arena[b];
                // phis: only reachable incoming edges
                for phi in &blk.phis {
                    let mut v = L::Top;
                    for (i, &p) in blk.preds.iter().enumerate() {
                        if i >= phi.a.len() || !s.exec[p] {
                            continue;
                        }
                        let pt = f.arena[p].term();
                        let live = if pt.op == Op::Br && pt.t[0] != pt.t[1] {
                            (pt.t[0] == b && s.taken[p][0]) || (pt.t[1] == b && s.taken[p][1])
                        } else {
                            true
                        };
                        if live {
                            v = meet(v, opv(&s.val, phi.a[i]));
                        }
                    }
                    if let Some(r) = rng {
                        if let Some(iv) = r.base[phi.d as usize] {
                            if iv.0 == iv.1 && v != L::Top {
                                v = L::C(iv.0);
                            }
                        }
                    }
                    if v != s.val[phi.d as usize] {
                        // lattice only moves down
                        let nv = meet(s.val[phi.d as usize], v);
                        if nv != s.val[phi.d as usize] {
                            s.val[phi.d as usize] = nv;
                            changed = true;
                        }
                    }
                }
                for ins in &blk.ins {
                    let op = ins.op;
                    if op == Op::Br {
                        let cv = opv(&s.val, ins.a[0]);
                        let mut c = cv;
                        if let (Some(r), true) = (rng, ins.a[0] >= 0) {
                            let iv = r.get(ins.a[0], b);
                            if iv.0 >= 1 {
                                c = L::C(1);
                            } else if iv.1 == 0 {
                                c = L::C(0);
                            }
                        }
                        let nt = match c {
                            L::Top => [false, false],
                            L::C(k) => {
                                if k != 0 {
                                    [true, false]
                                } else {
                                    [false, true]
                                }
                            }
                            L::Bot => [true, true],
                        };
                        for k in 0..2 {
                            if nt[k] && !s.taken[b][k] {
                                s.taken[b][k] = true;
                                let t = ins.t[k];
                                if !s.exec[t] {
                                    s.exec[t] = true;
                                }
                                changed = true;
                            }
                        }
                        continue;
                    }
                    if op == Op::Jmp {
                        if !s.taken[b][0] {
                            s.taken[b][0] = true;
                            s.exec[ins.t[0]] = true;
                            changed = true;
                        }
                        continue;
                    }
                    if !ins.has_d() {
                        continue;
                    }
                    let mut v = match op {
                        Op::Select => sel(opv(&s.val, ins.a[0]), opv(&s.val, ins.a[1]), opv(&s.val, ins.a[2])),
                        Op::Load | Op::Vload | Op::Mac | Op::Addsh => L::Bot,
                        _ if is_binop(op) => {
                            let x = ins.a[0];
                            let y = ins.a[1];
                            bin(op, opv(&s.val, x), opv(&s.val, y), x == y && x >= 0)
                        }
                        _ => L::Bot,
                    };
                    if let Some(r) = rng {
                        if v == L::Bot && !matches!(op, Op::Load | Op::Vload) {
                            if let Some(iv) = r.base[ins.d as usize] {
                                if iv.0 == iv.1 && !((op == Op::Div || op == Op::Mod) && r.may_trap(ins, b)) {
                                    v = L::C(iv.0);
                                }
                            }
                        }
                    }
                    let nvv = meet(s.val[ins.d as usize], v);
                    if nvv != s.val[ins.d as usize] {
                        s.val[ins.d as usize] = nvv;
                        changed = true;
                    }
                }
            }
            if !changed {
                settled = true;
                break;
            }
        }
        if settled {
            Some(s)
        } else {
            None
        }
    }
}

/// Run SCCP and apply the result. Returns true if the function changed.
pub fn sccp(f: &mut Func, rng: Option<&Ranges>) -> bool {
    let s = match Sccp::run(f, rng) {
        Some(s) => s,
        None => return false,
    };
    let nv = s.val.len();
    let mut rep = vec![NOREP; nv];
    let mut changed = false;
    let blocks = f.blocks.clone();
    for &b in &blocks {
        if !s.exec[b] {
            continue;
        }
        // constants
        let mut drop_phi = Vec::new();
        for (i, phi) in f.arena[b].phis.iter().enumerate() {
            if let L::C(c) = s.val[phi.d as usize] {
                rep[phi.d as usize] = lit(c);
                drop_phi.push(i);
            }
        }
        if !drop_phi.is_empty() {
            let mut k = 0;
            f.arena[b].phis.retain(|_| {
                let keep = !drop_phi.contains(&k);
                k += 1;
                keep
            });
            changed = true;
        }
        let old = std::mem::take(&mut f.arena[b].ins);
        let mut out = Vec::with_capacity(old.len());
        for mut ins in old {
            if ins.has_d() {
                if let L::C(c) = s.val[ins.d as usize] {
                    if !matches!(ins.op, Op::Load | Op::Vload) {
                        rep[ins.d as usize] = lit(c);
                        changed = true;
                        continue;
                    }
                }
            }
            if ins.op == Op::Br {
                let nt = s.taken[b];
                if nt[0] != nt[1] {
                    let (tk, dr) = if nt[0] { (ins.t[0], ins.t[1]) } else { (ins.t[1], ins.t[0]) };
                    ins.op = Op::Jmp;
                    ins.a.clear();
                    ins.t = vec![tk];
                    if dr != tk {
                        remove_edge(f, b, dr);
                    }
                    changed = true;
                }
            }
            out.push(ins);
        }
        f.arena[b].ins = out;
    }
    if changed {
        apply_rep(f, &rep);
        remove_unreachable(f);
        analyze(f);
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    const OPS: [Op; 12] = [Op::Add, Op::Sub, Op::Mul, Op::And, Op::Or, Op::Xor, Op::Shl, Op::Shr, Op::Lt, Op::Eq, Op::Div, Op::Mod];

    fn samples() -> Vec<i64> {
        let mut v: Vec<i64> = (0..300).collect();
        v.extend([MASK, MASK - 1, 0x8000_0000, 0x7FFF_FFFF, 0x8000_0001, 0xFFFF_FF00, 0x1234_5678, 0xDEAD_BEEF]);
        let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
        for _ in 0..300 {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            v.push((x & 0xFFFF_FFFF) as i64);
        }
        v
    }

    /// Every abstract result must contain every concrete result.
    #[test]
    fn bin_is_sound() {
        let vs = samples();
        for &op in &OPS {
            for &a in &vs {
                for &b in &vs {
                    let conc = evalop(op, a, b);
                    for (x, y) in [(L::C(a), L::C(b)), (L::C(a), L::Bot), (L::Bot, L::C(b)), (L::Bot, L::Bot), (L::C(a), L::Top), (L::Top, L::C(b))] {
                        let r = bin(op, x, y, false);
                        // Top operands mean "unreached": result must not be a wrong constant
                        // for the values the other operands could take; check with the
                        // concrete pair taken as the unknown side
                        if let L::C(k) = r {
                            // a constant answer is only allowed when concrete trapping ops
                            // do not matter (div/mod never become constants from non-constants)
                            if !matches!((x, y), (L::C(_), L::C(_))) {
                                assert!(!matches!(op, Op::Div | Op::Mod) || (op == Op::Mod && y == L::C(1)), "{:?} {:?} {:?}", op, x, y);
                            }
                            if let Some(c) = conc {
                                assert_eq!(k, c, "{:?} {:?} {:?} on {} {}", op, x, y, a, b);
                            }
                        }
                    }
                    // same operand
                    let rs = bin(op, L::Bot, L::Bot, true);
                    if let L::C(k) = rs {
                        if let Some(c) = evalop(op, a, a) {
                            assert_eq!(k, c, "same {:?} {}", op, a);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn select_is_sound() {
        let vs = [0i64, 1, 5, MASK];
        for &c in &vs {
            for &p in &vs {
                for &q in &vs {
                    let conc = if c != 0 { p } else { q };
                    for cc in [L::C(c), L::Bot] {
                        let r = sel(cc, L::C(p), L::C(q));
                        match r {
                            L::C(k) => assert_eq!(k, conc),
                            L::Top => panic!("top"),
                            L::Bot => {}
                        }
                    }
                }
            }
        }
    }
}
