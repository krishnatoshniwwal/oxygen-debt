//! crc.rs - GF(2) nibble tables for bitwise (LSB-first) CRC steps.
//!
//! One CRC step is x -> (x >> 1) ^ (P if x & 1 else 0). It is linear over GF(2), so k steps are
//! x -> (x >> k) ^ L_k(x & (2^k - 1)). For k = 4, L_4 is a table of 16 words: four steps cost
//! `and, add, load, shr, xor` instead of four diamonds. The table is written once into the
//! scratch region (words 3072.. : not observable) at function entry.
//!
//! The pass works on the program before SSA, as a source-to-source candidate:
//!  1. a diamond  `and t,r,1 [; eq c,t,1]; br ..` whose arms are `r = (r>>1)^P` / `r = r>>1`
//!     collapses into a marker (op Phi, d = r, a = [P]);
//!  2. a counted loop (`i` from a constant, `lt i, N`, body = one step + `i += 1`) becomes
//!     straight-line markers;
//!  3. every run of n markers with the same r and P becomes n/4 table steps plus n%4
//!     branch-free steps (`select`). Only when some run has n >= 4 (else ifconv already does it).
//! The result is checked like every other candidate (same behaviour on the timing inputs,
//! validator), and is only offered when interval analysis shows the program never touches
//! words >= 3072.
use crate::ir::*;
use crate::kir::*;
use std::collections::{HashMap, HashSet};

pub const TABLE_BASE: i64 = 3072;
const MAX_TABLES: usize = 4;
const MAX_TRIPS: i64 = 64;

/// One bitwise CRC step.
pub fn step(x: u32, p: u32) -> u32 {
    (x >> 1) ^ if x & 1 != 0 { p } else { 0 }
}

/// The 16-entry table of four steps: (x >> 4) ^ nibble_table(p)[x & 15].
pub fn nibble_table(p: u32) -> [u32; 16] {
    let mut t = [0u32; 16];
    for (n, e) in t.iter_mut().enumerate() {
        let mut x = n as u32;
        for _ in 0..4 {
            x = step(x, p);
        }
        *e = x;
    }
    t
}

fn occurrences(f: &Func) -> Vec<u32> {
    let mut cnt = vec![0u32; f.nval.max(0) as usize + 1];
    for &p in &f.params {
        cnt[p as usize] += 2; // a parameter is never private
    }
    for &b in &f.blocks {
        for ins in &f.arena[b].ins {
            if ins.has_d() {
                cnt[ins.d as usize] += 1;
            }
            for &x in &ins.a {
                if x >= 0 {
                    cnt[x as usize] += 1;
                }
            }
        }
    }
    cnt
}

fn is_marker(ins: &Instr) -> bool {
    ins.op == Op::Phi
}

/// An arm of the step diamond: pure shr/xor/mov over `r` and private temporaries; returns the
/// final form (shift, xor constant) of `r` = (r >> shift) ^ constant, and the jump target.
fn arm_form(f: &Func, arm: usize, x: usize, r: i64, cnt: &[u32]) -> Option<((u32, u32), usize)> {
    let blk = &f.arena[arm];
    if blk.preds.as_slice() != [x] || blk.ins.is_empty() {
        return None;
    }
    let term = blk.ins.last().unwrap();
    if term.op != Op::Jmp {
        return None;
    }
    let mut form: HashMap<i64, (u32, u32)> = HashMap::new();
    form.insert(r, (0, 0));
    let mut local: HashMap<i64, u32> = HashMap::new(); // occurrences inside the arm
    for ins in &blk.ins[..blk.ins.len() - 1] {
        if !ins.has_d() {
            return None;
        }
        let get = |o: i64| -> Option<(u32, u32)> { if o >= 0 { form.get(&o).copied() } else { None } };
        let val = match ins.op {
            Op::Mov => get(ins.a[0])?,
            Op::Shr => {
                let (s, c) = get(ins.a[0])?;
                if ins.a[1] >= 0 {
                    return None;
                }
                let k = (!ins.a[1] & 31) as u32;
                if k == 0 || s + k >= 32 {
                    return None;
                }
                (s + k, c >> k)
            }
            Op::Xor => {
                let (fa, lb) = match (get(ins.a[0]), get(ins.a[1])) {
                    (Some(fa), None) if ins.a[1] < 0 => (fa, !ins.a[1]),
                    (None, Some(fb)) if ins.a[0] < 0 => (fb, !ins.a[0]),
                    _ => return None,
                };
                (fa.0, fa.1 ^ (lb as u32))
            }
            _ => return None,
        };
        for &o in &ins.a {
            if o >= 0 {
                *local.entry(o).or_insert(0) += 1;
            }
        }
        *local.entry(ins.d).or_insert(0) += 1;
        form.insert(ins.d, val);
    }
    for (&reg, &n) in &local {
        if reg != r && cnt[reg as usize] != n {
            return None; // a temporary that lives outside the arm
        }
    }
    let fr = *form.get(&r)?;
    Some((fr, term.t[0]))
}

/// Collapse one diamond at the end of block `x`. Returns true if it did.
fn collapse(f: &mut Func, x: usize, cnt: &[u32], gone: &mut HashSet<usize>) -> bool {
    let n = f.arena[x].ins.len();
    if n < 2 {
        return false;
    }
    let br = f.arena[x].ins[n - 1].clone();
    if br.op != Op::Br || br.a[0] < 0 || br.t[0] == br.t[1] {
        return false;
    }
    let cond = br.a[0];
    let one = |a: &[i64]| a.len() == 2 && ((a[1] == L1 && a[0] >= 0) || (a[0] == L1 && a[1] >= 0));
    let reg_of = |a: &[i64]| if a[0] >= 0 { a[0] } else { a[1] };
    // pattern A: and t,r,1 ; eq c,t,1 ; br c     pattern B: and c,r,1 ; br c
    let (r, tail) = {
        let i1 = &f.arena[x].ins[n - 2];
        if i1.op == Op::And && i1.d == cond && one(&i1.a) && cnt[cond as usize] == 2 {
            (reg_of(&i1.a), 2)
        } else if i1.op == Op::Eq && i1.d == cond && one(&i1.a) && cnt[cond as usize] == 2 && n >= 3 {
            let t = reg_of(&i1.a);
            let i0 = &f.arena[x].ins[n - 3];
            if i0.op == Op::And && i0.d == t && one(&i0.a) && cnt[t as usize] == 2 && t != reg_of(&i0.a) {
                (reg_of(&i0.a), 3)
            } else {
                return false;
            }
        } else {
            return false;
        }
    };
    let (tt, ee) = (br.t[0], br.t[1]);
    if tt == x || ee == x || tt == ee {
        return false;
    }
    let (ft, jt) = match arm_form(f, tt, x, r, cnt) {
        Some(v) => v,
        None => return false,
    };
    let (fe, je) = match arm_form(f, ee, x, r, cnt) {
        Some(v) => v,
        None => return false,
    };
    if jt != je || ft.0 != 1 || fe != (1, 0) || jt == x || jt == tt || jt == ee {
        return false;
    }
    let j = jt;
    let mut jp = f.arena[j].preds.clone();
    jp.sort();
    let mut want = vec![tt, ee];
    want.sort();
    if jp != want {
        return false;
    }
    // collapse: x = x[..n-tail] + marker + j's instructions; t, e, j leave the layout
    let jins = std::mem::take(&mut f.arena[j].ins);
    let blk = &mut f.arena[x];
    blk.ins.truncate(n - tail);
    blk.ins.push(Instr::new(Op::Phi, r, vec![lit((ft.1 & 0xFFFF_FFFF) as i64)], vec![]));
    blk.ins.extend(jins);
    for &s in f.arena[x].succs().to_vec().iter() {
        for p in f.arena[s].preds.iter_mut() {
            if *p == j {
                *p = x;
            }
        }
    }
    gone.extend([tt, ee, j]);
    true
}

/// Append the single-predecessor successor of `x` (reached by a plain jmp) to `x`.
fn merge_next(f: &mut Func, x: usize, gone: &mut HashSet<usize>) -> bool {
    let mut any = false;
    loop {
        let t = match f.arena[x].ins.last() {
            Some(t) if t.op == Op::Jmp => t.t[0],
            _ => return any,
        };
        if t == x || t == f.blocks[0] || f.arena[t].preds.as_slice() != [x] {
            return any;
        }
        let tins = std::mem::take(&mut f.arena[t].ins);
        f.arena[x].ins.pop();
        f.arena[x].ins.extend(tins);
        for &s in f.arena[x].succs().to_vec().iter() {
            for p in f.arena[s].preds.iter_mut() {
                if *p == t {
                    *p = x;
                }
            }
        }
        gone.insert(t);
        any = true;
    }
}

/// Counted loop whose body is one marker plus `i += 1`: becomes straight-line markers.
fn unroll_loop(f: &mut Func, h: usize, cnt: &[u32], gone: &mut HashSet<usize>) -> bool {
    let hb = &f.arena[h];
    if hb.ins.len() != 2 || hb.preds.len() != 2 {
        return false;
    }
    let (lt, br) = (&hb.ins[0], &hb.ins[1]);
    if lt.op != Op::Lt || br.op != Op::Br || br.a[0] != lt.d || cnt[lt.d as usize] != 2 {
        return false;
    }
    if lt.a[0] < 0 || lt.a[1] >= 0 {
        return false;
    }
    let (i, nn) = (lt.a[0], !lt.a[1]);
    let (body, exit) = (br.t[0], br.t[1]);
    if body == h || exit == h || exit == body || nn < 1 || nn > MAX_TRIPS {
        return false;
    }
    let pre = match hb.preds.iter().copied().find(|&p| p != body) {
        Some(p) if hb.preds.contains(&body) => p,
        _ => return false,
    };
    let bb = &f.arena[body];
    if bb.preds.as_slice() != [h] || bb.ins.len() < 3 || !is_marker(&bb.ins[0]) {
        return false;
    }
    let tail = &bb.ins[1..];
    let ok = match tail.len() {
        2 => tail[0].op == Op::Add && tail[0].d == i && tail[0].a == vec![i, L1] && tail[1].op == Op::Jmp && tail[1].t[0] == h,
        3 => {
            let (a, m, j) = (&tail[0], &tail[1], &tail[2]);
            a.op == Op::Add && a.a == vec![i, L1] && a.d >= 0 && a.d != i && cnt[a.d as usize] == 2
                && m.op == Op::Mov && m.d == i && m.a == vec![a.d] && j.op == Op::Jmp && j.t[0] == h
        }
        _ => false,
    };
    if !ok {
        return false;
    }
    let mk = bb.ins[0].clone();
    if mk.d == i {
        return false;
    }
    // the loop is entered from `pre` with a jmp; the counter is a known constant there
    let pb = &f.arena[pre];
    if pb.ins.last().map(|t| t.op != Op::Jmp || t.t[0] != h).unwrap_or(true) {
        return false;
    }
    let c0 = match pb.ins.iter().rev().find(|x| x.has_d() && x.d == i) {
        Some(x) if x.op == Op::Const => x.a[0],
        _ => return false,
    };
    let c0 = !c0;
    if c0 < 0 || c0 >= nn {
        return false;
    }
    let pm = &mut f.arena[pre];
    pm.ins.pop();
    for _ in c0..nn {
        pm.ins.push(mk.clone());
    }
    pm.ins.push(Instr::new(Op::Const, i, vec![lit(nn)], vec![]));
    pm.ins.push(Instr::no_d(Op::Jmp, vec![], vec![exit]));
    for p in f.arena[exit].preds.iter_mut() {
        if *p == h {
            *p = pre;
        }
    }
    gone.extend([h, body]);
    true
}

/// Lower the marker runs; returns the polynomials that need a table.
fn lower(f: &mut Func) -> Vec<u32> {
    let mut polys: Vec<u32> = Vec::new();
    let blocks = f.blocks.clone();
    for b in blocks {
        if !f.arena[b].ins.iter().any(is_marker) {
            continue;
        }
        let old = std::mem::take(&mut f.arena[b].ins);
        let mut out: Vec<Instr> = Vec::with_capacity(old.len());
        let mut k = 0;
        while k < old.len() {
            if !is_marker(&old[k]) {
                out.push(old[k].clone());
                k += 1;
                continue;
            }
            let (r, pl) = (old[k].d, old[k].a[0]);
            let mut e = k;
            while e < old.len() && is_marker(&old[e]) && old[e].d == r && old[e].a[0] == pl {
                e += 1;
            }
            let n = e - k;
            let p = !pl as u32;
            let (qa, qb, qt, qs, qy) = (f.new_val(), f.new_val(), f.new_val(), f.new_val(), f.new_val());
            let table = if n >= 4 {
                if !polys.contains(&p) {
                    polys.push(p);
                }
                Some(TABLE_BASE + 16 * polys.iter().position(|&q| q == p).unwrap() as i64)
            } else {
                None
            };
            if let Some(base) = table {
                for _ in 0..n / 4 {
                    out.push(Instr::new(Op::And, qa, vec![r, lit(15)], vec![]));
                    out.push(Instr::new(Op::Add, qb, vec![qa, lit(base)], vec![]));
                    out.push(Instr::new(Op::Load, qt, vec![qb], vec![]));
                    out.push(Instr::new(Op::Shr, qs, vec![r, lit(4)], vec![]));
                    out.push(Instr::new(Op::Xor, r, vec![qs, qt], vec![]));
                }
            }
            for _ in 0..(if table.is_some() { n % 4 } else { n }) {
                out.push(Instr::new(Op::And, qa, vec![r, L1], vec![]));
                out.push(Instr::new(Op::Shr, qs, vec![r, L1], vec![]));
                out.push(Instr::new(Op::Xor, qy, vec![qs, pl], vec![]));
                out.push(Instr::new(Op::Select, r, vec![qa, qy, qs], vec![]));
            }
            k = e;
        }
        f.arena[b].ins = out;
    }
    polys
}

fn touches_scratch(prog: &Program) -> bool {
    let mut g = from_program(prog);
    crate::ssa::build_ssa(&mut g);
    let rng = crate::ranges::Ranges::new(&mut g, 14);
    if !rng.ok {
        return true;
    }
    for &b in &g.blocks {
        for ins in &g.arena[b].ins {
            if (ins.op == Op::Load || ins.op == Op::Store) && rng.get(ins.a[0], b).1 >= TABLE_BASE {
                return true;
            }
        }
    }
    false
}

/// The program with its bitwise CRC steps replaced by nibble-table steps, or None when it has
/// none (at least four in a row) or may use the scratch region.
pub fn transform(prog: &Program) -> Option<Program> {
    let mut f = from_program(prog);
    let cnt = occurrences(&f);
    let mut any = false;
    for _ in 0..4 {
        let mut changed = false;
        // blocks that merged into another one are dropped from the layout at the end of the pass
        let mut gone: HashSet<usize> = HashSet::new();
        for b in f.blocks.clone() {
            if gone.contains(&b) {
                continue;
            }
            let mut c = merge_next(&mut f, b, &mut gone);
            while collapse(&mut f, b, &cnt, &mut gone) {
                c = true;
                merge_next(&mut f, b, &mut gone);
            }
            changed |= c;
        }
        f.blocks.retain(|b| !gone.contains(b));
        gone.clear();
        for b in f.blocks.clone() {
            if !gone.contains(&b) && unroll_loop(&mut f, b, &cnt, &mut gone) {
                changed = true;
            }
        }
        f.blocks.retain(|b| !gone.contains(b));
        any |= changed;
        if !changed {
            break;
        }
    }
    if !any || !f.blocks.iter().any(|&b| f.arena[b].ins.iter().any(is_marker)) {
        return None;
    }
    let polys = lower(&mut f);
    if polys.is_empty() || polys.len() > MAX_TABLES || touches_scratch(prog) {
        return None;
    }
    // table initialisation in a new first block
    let init = f.new_block("crc_init");
    let old_entry = f.blocks[0];
    for (ti, &p) in polys.iter().enumerate() {
        for (n, &v) in nibble_table(p).iter().enumerate() {
            let addr = TABLE_BASE + 16 * ti as i64 + n as i64;
            f.arena[init].ins.push(Instr::no_d(Op::Store, vec![lit(addr), lit(v as i64)], vec![]));
        }
    }
    f.arena[init].ins.push(Instr::no_d(Op::Jmp, vec![], vec![old_entry]));
    f.blocks.insert(0, init);
    let text = to_text(&f);
    parse_program(&text).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const POLY: u32 = 0xEDB88320;

    #[test]
    fn nibble_table_matches_bitwise() {
        let mut rng = Rng::new(7);
        for p in [POLY, 0x8408, 0x82F63B78, 1, 0xFFFF_FFFF] {
            let t = nibble_table(p);
            for _ in 0..2000 {
                let x = rng.next() as u32;
                let mut y = x;
                for _ in 0..4 {
                    y = step(y, p);
                }
                assert_eq!((x >> 4) ^ t[(x & 15) as usize], y);
            }
        }
    }

    const BYTES: &str = "func crc(v0)
entry:
  and v1, v0, 15
  mov v0, v1
  const v2, 0xFFFFFFFF
  const v3, 0
  jmp for_head1
for_head1:
  lt v4, v3, v0
  br v4, for_body2, for_exit3
for_body2:
  load v5, v3
  and v6, v5, 255
  xor v7, v2, v6
  mov v2, v7
  const v8, 0
  jmp for_head4
for_head4:
  lt v9, v8, 8
  br v9, for_body5, for_exit6
for_body5:
  and v10, v2, 1
  eq v11, v10, 1
  br v11, then7, else9
then7:
  shr v12, v2, 1
  xor v13, v12, 0xEDB88320
  mov v2, v13
  jmp endif8
else9:
  shr v14, v2, 1
  mov v2, v14
  jmp endif8
endif8:
  add v15, v8, 1
  mov v8, v15
  jmp for_head4
for_exit6:
  add v16, v3, 1
  mov v3, v16
  jmp for_head1
for_exit3:
  xor v17, v2, 0xFFFFFFFF
  ret v17
";

    fn run_same(a: &str, b: &str) {
        let (pa, pb) = (parse_program(a).unwrap(), parse_program(b).unwrap());
        let (ca, cb) = (compile_program(&pa), compile_program(&pb));
        let mut rng = Rng::new(3);
        for n in 0..40 {
            let mut inp = Inputs::default();
            inp.args = vec![n as u32];
            let words: Vec<u32> = (0..32).map(|_| rng.next() as u32).collect();
            inp.segments = vec![(0, words)];
            let (ra, rb) = (execute(&ca, &inp, 1_000_000, false), execute(&cb, &inp, 1_000_000, false));
            assert!(!differs(&ra, &rb), "n={}", n);
        }
    }

    #[test]
    fn bytewise_loop_is_replaced_and_equal() {
        let p = parse_program(BYTES).unwrap();
        let q = transform(&p).expect("crc loop recognised");
        let text = q.instrs.len();
        assert!(text > 0);
        // rebuild the text and compare behaviour with the original
        let t = {
            let f = from_program(&q);
            to_text(&f)
        };
        assert!(t.contains("load"), "{}", t);
        assert!(!t.contains("br v11"), "{}", t);
        run_same(BYTES, &t);
    }

    #[test]
    fn straight_line_chain_is_replaced_and_equal() {
        // 6 unrolled steps, then another byte xor, then 3 steps
        let mut s = String::from("func c(v0, v1)\nentry:\n  const v2, 0xFFFFFFFF\n  xor v3, v2, v0\n  mov v2, v3\n  jmp s0\n");
        let (mut reg, mut blk) = (10, 0);
        let mut next = "s0".to_string();
        let mut emit = |s: &mut String, next: &mut String, reg: &mut i64, blk: &mut i64, last: bool| {
            let (a, b, c, d, e, g) = (*reg, *reg + 1, *reg + 2, *reg + 3, *reg + 4, *reg + 5);
            let (t, e_, j) = (format!("t{}", blk), format!("e{}", blk), format!("j{}", blk));
            s.push_str(&format!("{}:\n  and v{}, v2, 1\n  eq v{}, v{}, 1\n  br v{}, {}, {}\n", next, a, b, a, b, t, e_));
            s.push_str(&format!("{}:\n  shr v{}, v2, 1\n  xor v{}, v{}, 0xEDB88320\n  mov v2, v{}\n  jmp {}\n", t, c, d, c, d, j));
            s.push_str(&format!("{}:\n  shr v{}, v2, 1\n  mov v2, v{}\n  jmp {}\n", e_, e, e, j));
            let nn = if last { "fin".to_string() } else { format!("s{}", *blk + 1) };
            s.push_str(&format!("{}:\n  jmp {}\n", j, nn));
            let _ = g;
            *next = nn;
            *reg += 6;
            *blk += 1;
        };
        for k in 0..9 {
            emit(&mut s, &mut next, &mut reg, &mut blk, k == 8);
            if k == 5 {
                // break the chain: xor a second byte in (the pending jmp target is the next step)
                let nn = format!("s{}", blk);
                let last = s.rfind("jmp").unwrap();
                s.truncate(last);
                s.push_str(&format!("xor v2, v2, v1\n  jmp {}\n", nn));
            }
        }
        s.push_str("fin:\n  xor v900, v2, 0xFFFFFFFF\n  ret v900\n");
        let p = parse_program(&s).unwrap();
        let q = transform(&p).expect("chain recognised");
        let t = to_text(&from_program(&q));
        assert!(t.contains("load"), "{}", t);
        run_same(&s, &t);
    }
}
