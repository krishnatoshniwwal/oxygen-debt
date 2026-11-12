//! passes.rs - machine-independent optimizations on SSA form.
use crate::ir::*;
use crate::kir::*;
use crate::ranges::Ranges;
use std::collections::HashMap;

/// Exact KIR semantics for a binary op on two 32-bit values. None if it would trap.
pub fn evalop(op: Op, x: i64, y: i64) -> Option<i64> {
    Some(match op {
        Op::Add => (x + y) & MASK,
        Op::Sub => (x - y) & MASK,
        Op::Mul => x.wrapping_mul(y) & MASK,
        Op::And => x & y,
        Op::Or => x | y,
        Op::Xor => x ^ y,
        Op::Shl => (x << (y & 31)) & MASK,
        Op::Shr => x >> (y & 31),
        Op::Lt => ((x as i32) < (y as i32)) as i64,
        Op::Eq => (x == y) as i64,
        Op::Div | Op::Mod => {
            if y == 0 {
                return None;
            }
            let sx = x as i32 as i64;
            let sy = y as i32 as i64;
            if op == Op::Div {
                let mut q = sx.abs() / sy.abs();
                if (sx < 0) != (sy < 0) {
                    q = -q;
                }
                q & MASK
            } else {
                let mut r = sx.abs() % sy.abs();
                if sx < 0 {
                    r = -r;
                }
                r & MASK
            }
        }
        _ => panic!("evalop: {:?}", op),
    })
}

type Defs = [Option<DefInfo>];

fn def(defs: &Defs, x: i64) -> Option<DefInfo> {
    if x >= 0 {
        defs.get(x as usize).copied().flatten()
    } else {
        None
    }
}

/// Simplify one SSA instruction (operands already resolved). Returns an operand that
/// replaces the result, or None (the instruction may have been rewritten in place).
pub fn simplify(ins: &mut Instr, defs: &Defs) -> Option<i64> {
    let mut rounds = 0;
    loop {
        rounds += 1;
        if rounds > 40 {
            return None; // safety net: the rewrite rules must never loop forever
        }
        let op = ins.op;
        if is_binop(op) {
            let (mut x, mut y) = (ins.a[0], ins.a[1]);
            if x < 0 && y < 0 {
                return evalop(op, !x, !y).map(lit);
            }
            if x < 0 && is_comm(op) {
                std::mem::swap(&mut x, &mut y);
                ins.a[0] = x;
                ins.a[1] = y;
            }
            let dx = def(defs, x);
            let dy = def(defs, y);
            // dx / dy as "is a multiply by a literal"
            let mulc = |d: Option<DefInfo>| d.map_or(false, |d| d.op == Op::Mul && d.a1 < 0);
            match op {
                Op::Add => {
                    if y < 0 {
                        let c = !y;
                        if c == 0 {
                            return Some(x);
                        }
                        if let Some(d) = dx {
                            if d.op == Op::Add && d.a1 < 0 {
                                ins.a = vec![d.a0, lit(c + !d.a1)];
                                continue;
                            }
                        }
                        return None;
                    }
                    if x == y {
                        ins.op = Op::Mul;
                        ins.a = vec![x, lit(2)];
                        continue;
                    }
                    if let Some(d) = dx {
                        if d.op == Op::Sub && d.a1 == y {
                            return Some(d.a0);
                        }
                    }
                    if let Some(d) = dy {
                        if d.op == Op::Sub && d.a1 == x {
                            return Some(d.a0);
                        }
                    }
                    let mx = mulc(dx);
                    let my = mulc(dy);
                    if mx && my && dx.unwrap().a0 == dy.unwrap().a0 {
                        ins.op = Op::Mul;
                        ins.a = vec![dx.unwrap().a0, lit(!dx.unwrap().a1 + !dy.unwrap().a1)];
                        continue;
                    }
                    if mx && dx.unwrap().a0 == y {
                        ins.op = Op::Mul;
                        ins.a = vec![y, lit(!dx.unwrap().a1 + 1)];
                        continue;
                    }
                    if my && dy.unwrap().a0 == x {
                        ins.op = Op::Mul;
                        ins.a = vec![x, lit(!dy.unwrap().a1 + 1)];
                        continue;
                    }
                    return None;
                }
                Op::Sub => {
                    if y < 0 {
                        ins.op = Op::Add;
                        ins.a = vec![x, lit(-(!y))];
                        continue;
                    }
                    if x == y {
                        return Some(L0);
                    }
                    if let Some(d) = dx {
                        if d.op == Op::Add {
                            if d.a1 == y {
                                return Some(d.a0);
                            }
                            if d.a0 == y {
                                return Some(d.a1);
                            }
                        }
                    }
                    let mx = mulc(dx);
                    let my = mulc(dy);
                    if mx && my && dx.unwrap().a0 == dy.unwrap().a0 {
                        ins.op = Op::Mul;
                        ins.a = vec![dx.unwrap().a0, lit(!dx.unwrap().a1 - !dy.unwrap().a1)];
                        continue;
                    }
                    if mx && dx.unwrap().a0 == y {
                        ins.op = Op::Mul;
                        ins.a = vec![y, lit(!dx.unwrap().a1 - 1)];
                        continue;
                    }
                    return None;
                }
                Op::Mul => {
                    if y < 0 {
                        let c = !y;
                        if c == 0 {
                            return Some(L0);
                        }
                        if c == 1 {
                            return Some(x);
                        }
                        if let Some(d) = dx {
                            if d.op == Op::Mul && d.a1 < 0 {
                                ins.a = vec![d.a0, lit(c.wrapping_mul(!d.a1))];
                                continue;
                            }
                        }
                    }
                    return None;
                }
                Op::And => {
                    if y < 0 {
                        let c = !y;
                        if c == 0 {
                            return Some(L0);
                        }
                        if c == MASK {
                            return Some(x);
                        }
                        if let Some(d) = dx {
                            if d.op == Op::And && d.a1 < 0 {
                                ins.a = vec![d.a0, lit(c & !d.a1)];
                                continue;
                            }
                        }
                        return None;
                    }
                    if x == y {
                        return Some(x);
                    }
                    return None;
                }
                Op::Or => {
                    if y < 0 {
                        let c = !y;
                        if c == 0 {
                            return Some(x);
                        }
                        if c == MASK {
                            return Some(lit(MASK));
                        }
                        if let Some(d) = dx {
                            if d.op == Op::Or && d.a1 < 0 {
                                ins.a = vec![d.a0, lit(c | !d.a1)];
                                continue;
                            }
                        }
                        return None;
                    }
                    if x == y {
                        return Some(x);
                    }
                    return None;
                }
                Op::Xor => {
                    if y < 0 {
                        let c = !y;
                        if c == 0 {
                            return Some(x);
                        }
                        if let Some(d) = dx {
                            if d.op == Op::Xor && d.a1 < 0 {
                                ins.a = vec![d.a0, lit(c ^ !d.a1)];
                                continue;
                            }
                        }
                        return None;
                    }
                    if x == y {
                        return Some(L0);
                    }
                    return None;
                }
                Op::Shl => {
                    if x == L0 {
                        return Some(L0);
                    }
                    if y < 0 {
                        let k = !y & 31;
                        if k == 0 {
                            return Some(x);
                        }
                        ins.op = Op::Mul;
                        ins.a = vec![x, lit(1i64 << k)];
                        continue;
                    }
                    return None;
                }
                Op::Shr => {
                    if x == L0 {
                        return Some(L0);
                    }
                    if y < 0 {
                        let k = !y & 31;
                        if k == 0 {
                            return Some(x);
                        }
                        if let Some(d) = dx {
                            if d.op == Op::Shr && d.a1 < 0 {
                                let k2 = k + (!d.a1 & 31);
                                if k2 >= 32 {
                                    return Some(L0);
                                }
                                ins.a = vec![d.a0, lit(k2)];
                                continue;
                            }
                        }
                        if !y != k {
                            ins.a[1] = lit(k);
                        }
                    }
                    return None;
                }
                Op::Lt => {
                    if x == y {
                        return Some(L0);
                    }
                    return None;
                }
                Op::Eq => {
                    if x == y {
                        return Some(L1);
                    }
                    if y < 0 {
                        if let Some(d) = dx {
                            let c = !y;
                            if d.op == Op::Add && d.a1 < 0 {
                                ins.a = vec![d.a0, lit(c - !d.a1)];
                                continue;
                            }
                            if d.op == Op::Xor && d.a1 < 0 {
                                ins.a = vec![d.a0, lit(c ^ !d.a1)];
                                continue;
                            }
                            if c == 0 && (d.op == Op::Sub || d.op == Op::Xor) {
                                ins.a = vec![d.a0, d.a1];
                                continue;
                            }
                        }
                    }
                    return None;
                }
                Op::Div => {
                    if y == L1 {
                        return Some(x);
                    }
                    return None;
                }
                Op::Mod => {
                    if y == L1 {
                        return Some(L0);
                    }
                    return None;
                }
                _ => return None,
            }
        }
        if op == Op::Select {
            let (c, p, q) = (ins.a[0], ins.a[1], ins.a[2]);
            if c < 0 {
                return Some(if !c != 0 { p } else { q });
            }
            if p == q {
                return Some(p);
            }
            if let Some(dc) = def(defs, c) {
                if dc.op == Op::Eq && dc.a1 == L0 {
                    ins.a = vec![dc.a0, q, p];
                    continue;
                }
                if p == L1 && q == L0 && (dc.op == Op::Lt || dc.op == Op::Eq) {
                    return Some(c);
                }
            }
            return None;
        }
        return None;
    }
}

fn is_gvn(op: Op) -> bool {
    is_pure(op) || op == Op::Div || op == Op::Mod
}

#[derive(Hash, PartialEq, Eq, Clone)]
enum Key {
    N(Op, i64, i64, i64),
    Phi(usize, Vec<i64>),
}

pub const NOREP: i64 = i64::MIN;

pub fn res(rep: &[i64], mut x: i64) -> i64 {
    let mut n = 0usize;
    while x >= 0 {
        n += 1;
        // a replacement cycle would loop forever; fail loudly instead (the caller's variant is dropped)
        if n > rep.len() + 1 {
            panic!("replacement cycle at v{}", x);
        }
        let y = rep[x as usize];
        if y == NOREP {
            return x;
        }
        x = y;
    }
    x
}

/// Replace operands by their representatives in every instruction and phi.
pub fn apply_rep(f: &mut Func, rep: &[i64]) {
    for bi in 0..f.blocks.len() {
        let b = f.blocks[bi];
        let blk = &mut f.arena[b];
        for phi in blk.phis.iter_mut() {
            for x in phi.a.iter_mut() {
                if *x >= 0 && rep[*x as usize] != NOREP {
                    *x = res(rep, *x);
                }
            }
        }
        for ins in blk.ins.iter_mut() {
            for x in ins.a.iter_mut() {
                if *x >= 0 && rep[*x as usize] != NOREP {
                    *x = res(rep, *x);
                }
            }
        }
    }
}

/// Simplification, constant folding, copy propagation, value numbering and branch folding
/// in one walk over the dominator tree. Returns true if anything changed.
pub fn scalar_pass(f: &mut Func) -> bool {
    analyze(f);
    let nv = f.nval.max(0) as usize + 1;
    let mut rep = vec![NOREP; nv];
    let mut defs: Vec<Option<DefInfo>> = vec![None; nv];
    let mut changed = false;
    let mut table: HashMap<Key, i64> = HashMap::new();
    let mut stack: Vec<(usize, Option<Vec<Key>>)> = vec![(f.blocks[0], None)];
    while let Some((b, undo)) = stack.pop() {
        if let Some(u) = undo {
            for k in u {
                table.remove(&k);
            }
            continue;
        }
        let mut undo: Vec<Key> = Vec::new();
        if !f.arena[b].phis.is_empty() {
            let phis = std::mem::take(&mut f.arena[b].phis);
            let mut keep = Vec::with_capacity(phis.len());
            for mut phi in phis {
                let mut same: Option<i64> = None;
                let mut triv = true;
                for i in 0..phi.a.len() {
                    let mut x = phi.a[i];
                    if x >= 0 && rep[x as usize] != NOREP {
                        x = res(&rep, x);
                        phi.a[i] = x;
                    }
                    if x == phi.d {
                        continue;
                    }
                    match same {
                        None => same = Some(x),
                        Some(s) if s != x => triv = false,
                        _ => {}
                    }
                }
                if triv {
                    let tgt = same.unwrap_or(L0);
                    // never map a value onto itself (can happen with irreducible control flow)
                    if res(&rep, tgt) != phi.d {
                        rep[phi.d as usize] = tgt;
                        changed = true;
                        continue;
                    }
                }
                let key = Key::Phi(b, phi.a.clone());
                if let Some(&o) = table.get(&key) {
                    if res(&rep, o) != phi.d {
                        rep[phi.d as usize] = o;
                        changed = true;
                        continue;
                    }
                }
                table.insert(key.clone(), phi.d);
                undo.push(key);
                defs[phi.d as usize] = Some(DefInfo::of(&phi));
                keep.push(phi);
            }
            f.arena[b].phis = keep;
        }
        let old = std::mem::take(&mut f.arena[b].ins);
        let mut out: Vec<Instr> = Vec::with_capacity(old.len());
        for mut ins in old {
            for x in ins.a.iter_mut() {
                if *x >= 0 && rep[*x as usize] != NOREP {
                    *x = res(&rep, *x);
                }
            }
            if !ins.has_d() {
                if ins.op == Op::Br {
                    let mut c = ins.a[0];
                    while c >= 0 {
                        match def(&defs, c) {
                            Some(dc) if dc.op == Op::Eq && dc.a1 == L0 => {
                                c = dc.a0;
                                ins.a[0] = c;
                                ins.t.reverse();
                                changed = true;
                            }
                            _ => break,
                        }
                    }
                    if c < 0 {
                        let (tk, dr) = if !c != 0 { (ins.t[0], ins.t[1]) } else { (ins.t[1], ins.t[0]) };
                        if dr != tk {
                            remove_edge(f, b, dr);
                        }
                        ins.op = Op::Jmp;
                        ins.a = vec![];
                        ins.t = vec![tk];
                        changed = true;
                    }
                }
                out.push(ins);
                continue;
            }
            let d = ins.d;
            if ins.a.contains(&d) {
                // a value defined in terms of itself (odd control flow): leave it alone
                out.push(ins);
                continue;
            }
            if is_gvn(ins.op) {
                let before = (ins.op, ins.a[0], ins.a[1]);
                if let Some(r) = simplify(&mut ins, &defs) {
                    if res(&rep, r) != d {
                        rep[d as usize] = r;
                        changed = true;
                        continue;
                    }
                }
                if (ins.op, ins.a[0], ins.a[1]) != before {
                    changed = true;
                }
                if is_comm(ins.op) && ins.a[0] > ins.a[1] && ins.a[1] >= 0 {
                    ins.a.swap(0, 1);
                }
                let key = Key::N(ins.op, ins.a[0], ins.a[1], *ins.a.get(2).unwrap_or(&0));
                if let Some(&o) = table.get(&key) {
                    if res(&rep, o) != d {
                        rep[d as usize] = o;
                        changed = true;
                        continue;
                    }
                }
                table.insert(key.clone(), d);
                undo.push(key);
            }
            defs[d as usize] = Some(DefInfo::of(&ins));
            out.push(ins);
        }
        f.arena[b].ins = out;
        stack.push((b, Some(undo)));
        for k in (0..f.arena[b].dkids.len()).rev() {
            stack.push((f.arena[b].dkids[k], None));
        }
    }
    if rep.iter().any(|&r| r != NOREP) {
        apply_rep(f, &rep);
    }
    changed
}

/// CFG simplification on SSA form: remove unreachable blocks, forward empty blocks, merge a
/// block with its only successor when that successor has no other predecessor.
pub fn cleanup_cfg(f: &mut Func) -> bool {
    let mut changed = false;
    let mut again = true;
    while again {
        again = false;
        if remove_unreachable(f) {
            changed = true;
        }
        let entry = f.blocks[0];
        // forward blocks that only contain 'jmp S'
        for bi in 0..f.blocks.len() {
            let e = f.blocks[bi];
            {
                let eb = &f.arena[e];
                if e == entry || !eb.phis.is_empty() || eb.ins.len() != 1 || eb.ins[0].op != Op::Jmp {
                    continue;
                }
            }
            let s = f.arena[e].ins[0].t[0];
            if s == e {
                continue;
            }
            let mut ok = true;
            if !f.arena[s].phis.is_empty() {
                for p in &f.arena[e].preds {
                    if f.arena[s].preds.contains(p) {
                        ok = false;
                        break;
                    }
                }
            }
            if !ok || f.arena[e].preds.is_empty() {
                continue;
            }
            let ei = f.arena[s].preds.iter().position(|&x| x == e).unwrap();
            let preds = f.arena[e].preds.clone();
            let mut first = true;
            for &p in &preds {
                for t in f.arena[p].ins.last_mut().unwrap().t.iter_mut() {
                    if *t == e {
                        *t = s;
                    }
                }
                if f.arena[s].preds.contains(&p) {
                    // no phis in s (checked above)
                } else if first {
                    f.arena[s].preds[ei] = p;
                    first = false;
                } else {
                    f.arena[s].preds.push(p);
                    for phi in f.arena[s].phis.iter_mut() {
                        let v = phi.a[ei];
                        phi.a.push(v);
                    }
                }
                let term = f.arena[p].ins.last_mut().unwrap();
                if term.op == Op::Br && term.t[0] == term.t[1] {
                    term.op = Op::Jmp;
                    term.a = vec![];
                    term.t = vec![term.t[0]];
                }
            }
            if first {
                // every predecessor already reached s directly
                f.arena[s].preds.remove(ei);
            }
            f.arena[e].preds.clear();
            again = true;
            changed = true;
        }
        if again {
            continue;
        }
        // merge b -> s when s has only b as predecessor
        let mut dead = vec![false; f.arena.len()];
        let mut any_dead = false;
        for bi in 0..f.blocks.len() {
            let b = f.blocks[bi];
            if dead[b] {
                continue;
            }
            loop {
                let term = f.arena[b].ins.last().unwrap();
                if term.op != Op::Jmp {
                    break;
                }
                let s = term.t[0];
                if s == b || s == entry || f.arena[s].preds.len() != 1 {
                    break;
                }
                if !f.arena[s].phis.is_empty() {
                    break;
                }
                let sins = std::mem::replace(&mut f.arena[s].ins, vec![Instr::no_d(Op::Trap, vec![L0], vec![])]);
                f.arena[b].ins.pop();
                let succs: Vec<usize> = sins.last().unwrap().t.clone();
                f.arena[b].ins.extend(sins);
                for ss in succs {
                    let i = f.arena[ss].preds.iter().position(|&x| x == s).unwrap();
                    f.arena[ss].preds[i] = b;
                }
                dead[s] = true;
                any_dead = true;
                f.arena[s].preds.clear();
                changed = true;
            }
        }
        if any_dead {
            f.blocks.retain(|&b| !dead[b]);
            again = true;
        }
    }
    changed
}

pub fn use_counts(f: &Func) -> Vec<u32> {
    let mut uc = vec![0u32; f.nval.max(0) as usize + 1];
    for &b in &f.blocks {
        for phi in &f.arena[b].phis {
            for &x in &phi.a {
                if x >= 0 {
                    uc[x as usize] += 1;
                }
            }
        }
        for ins in &f.arena[b].ins {
            for &x in &ins.a {
                if x >= 0 {
                    uc[x as usize] += 1;
                }
            }
        }
    }
    uc
}

/// Remove instructions whose result is never used and that have no visible effect. A
/// load / div / mod stays unless `rng` proves that it cannot trap.
pub fn dce(f: &mut Func, rng: Option<&Ranges>) -> bool {
    let nv = f.nval.max(0) as usize + 1;
    // where is each value defined: (block, is_phi, index)
    let mut defs: Vec<Option<(usize, bool, usize)>> = vec![None; nv];
    let mut live = vec![false; nv];
    let mut work: Vec<i64> = Vec::new();
    for &b in &f.blocks {
        for (i, phi) in f.arena[b].phis.iter().enumerate() {
            defs[phi.d as usize] = Some((b, true, i));
        }
        for (i, ins) in f.arena[b].ins.iter().enumerate() {
            if ins.has_d() {
                defs[ins.d as usize] = Some((b, false, i));
            }
            let root = if !ins.has_d() || ins.op == Op::Vload {
                true
            } else if matches!(ins.op, Op::Load | Op::Div | Op::Mod) {
                rng.map_or(true, |r| r.may_trap(ins, b))
            } else {
                false
            };
            if root {
                for &x in &ins.a {
                    if x >= 0 && !live[x as usize] {
                        live[x as usize] = true;
                        work.push(x);
                    }
                }
            }
        }
    }
    while let Some(v) = work.pop() {
        if let Some((b, is_phi, i)) = defs[v as usize] {
            let ins = if is_phi { &f.arena[b].phis[i] } else { &f.arena[b].ins[i] };
            for &x in &ins.a {
                if x >= 0 && !live[x as usize] {
                    live[x as usize] = true;
                    work.push(x);
                }
            }
        }
    }
    let mut changed = false;
    for bi in 0..f.blocks.len() {
        let b = f.blocks[bi];
        if !f.arena[b].phis.is_empty() {
            let n = f.arena[b].phis.len();
            f.arena[b].phis.retain(|p| live[p.d as usize]);
            if f.arena[b].phis.len() != n {
                changed = true;
            }
        }
        let old = std::mem::take(&mut f.arena[b].ins);
        let mut out = Vec::with_capacity(old.len());
        for ins in old {
            if ins.has_d() && !live[ins.d as usize] {
                if ins.op == Op::Vload {
                    out.push(ins);
                    continue;
                }
                if matches!(ins.op, Op::Load | Op::Div | Op::Mod) && rng.map_or(true, |r| r.may_trap(&ins, b)) {
                    out.push(ins);
                    continue;
                }
                changed = true;
                continue;
            }
            out.push(ins);
        }
        f.arena[b].ins = out;
    }
    changed
}
