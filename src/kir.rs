//! kir.rs - the KIR parser, the machine file reader, the interpreter (exact semantics) and
//! the timing model. A port of the starter kit's kirlib.py.
use std::collections::{BTreeMap, HashMap, VecDeque};

pub const MASK: i64 = 0xFFFF_FFFF;
pub const SIGN: i64 = 0x8000_0000;
pub const MEM_WORDS: usize = 4096;
pub const SCRATCH_BASE: usize = 3072;
pub const MAX_INSTRS: usize = 500_000;
pub const MAX_PARAMS: usize = 8;
pub const GEN_MEM_WORDS: usize = 256;

pub type KResult<T> = Result<T, String>;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub enum Op {
    Const, Mov, Add, Sub, Mul, And, Or, Xor, Shl, Shr, Lt, Eq, Div, Mod, Select, Mac, Addsh,
    Load, Store, Vload, Vstore, Trap, Jmp, Br, Ret, Phi,
}

impl Op {
    pub fn from_name(s: &str) -> Option<Op> {
        Some(match s {
            "const" => Op::Const, "mov" => Op::Mov, "add" => Op::Add, "sub" => Op::Sub,
            "mul" => Op::Mul, "and" => Op::And, "or" => Op::Or, "xor" => Op::Xor,
            "shl" => Op::Shl, "shr" => Op::Shr, "lt" => Op::Lt, "eq" => Op::Eq, "div" => Op::Div,
            "mod" => Op::Mod, "select" => Op::Select, "mac" => Op::Mac, "addsh" => Op::Addsh,
            "load" => Op::Load, "store" => Op::Store, "vload" => Op::Vload, "vstore" => Op::Vstore,
            "trap" => Op::Trap, "jmp" => Op::Jmp, "br" => Op::Br, "ret" => Op::Ret,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Op::Const => "const", Op::Mov => "mov", Op::Add => "add", Op::Sub => "sub",
            Op::Mul => "mul", Op::And => "and", Op::Or => "or", Op::Xor => "xor",
            Op::Shl => "shl", Op::Shr => "shr", Op::Lt => "lt", Op::Eq => "eq", Op::Div => "div",
            Op::Mod => "mod", Op::Select => "select", Op::Mac => "mac", Op::Addsh => "addsh",
            Op::Load => "load", Op::Store => "store", Op::Vload => "vload", Op::Vstore => "vstore",
            Op::Trap => "trap", Op::Jmp => "jmp", Op::Br => "br", Op::Ret => "ret",
            Op::Phi => "phi",
        }
    }

    /// Operand signature: d = destination register, s = register or literal, i = literal,
    /// k = shift literal 0..31, L = label.
    pub fn sig(self) -> &'static str {
        match self {
            Op::Const => "di", Op::Mov => "ds",
            Op::Add | Op::Sub | Op::Mul | Op::And | Op::Or | Op::Xor | Op::Shl | Op::Shr
            | Op::Lt | Op::Eq | Op::Div | Op::Mod => "dss",
            Op::Select | Op::Mac => "dsss", Op::Addsh => "dssk",
            Op::Load | Op::Vload => "ds", Op::Store | Op::Vstore => "ss",
            Op::Trap | Op::Ret => "s", Op::Jmp => "L", Op::Br => "sLL", Op::Phi => "",
        }
    }

    pub fn is_term(self) -> bool {
        matches!(self, Op::Jmp | Op::Br | Op::Ret | Op::Trap)
    }

    pub fn is_mem(self) -> bool {
        matches!(self, Op::Load | Op::Store | Op::Vload | Op::Vstore)
    }
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------
#[derive(Clone, Debug)]
pub enum Arg {
    R(i64),
    I(i64),
    K(i64),
    L(String),
}

pub struct PInstr {
    pub op: Op,
    pub args: Vec<Arg>,
}

#[derive(Default)]
pub struct Program {
    pub name: String,
    pub params: Vec<i64>,
    pub instrs: Vec<PInstr>,
    pub labels: HashMap<String, usize>,
    pub block_labels: Vec<String>,
    pub block_of: Vec<usize>,
}

pub fn parse_reg(tok: &str) -> Option<i64> {
    let rest = tok.strip_prefix('v')?;
    if rest.is_empty() || !rest.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    if rest == "0" {
        return Some(0);
    }
    if rest.starts_with('0') || rest.len() > 6 {
        return None;
    }
    rest.parse().ok()
}

fn is_name(tok: &str) -> bool {
    let mut it = tok.bytes();
    match it.next() {
        Some(c) if c.is_ascii_alphabetic() || c == b'_' => {}
        _ => return false,
    }
    it.all(|c| c.is_ascii_alphanumeric() || c == b'_')
}

pub fn parse_literal_raw(tok: &str) -> KResult<i64> {
    let (neg, body) = match tok.strip_prefix('-') {
        Some(b) => (true, b),
        None => (false, tok),
    };
    let (digits, radix) = match body.strip_prefix("0x") {
        Some(h) => (h, 16),
        None => (body, 10),
    };
    let ok = !digits.is_empty()
        && digits.bytes().all(|c| if radix == 16 { c.is_ascii_hexdigit() } else { c.is_ascii_digit() });
    if !ok {
        return Err(format!("bad integer literal '{}'", tok));
    }
    let mut n: i128 = 0;
    for c in digits.bytes() {
        let v = (c as char).to_digit(radix).unwrap() as i128;
        n = n * radix as i128 + v;
        if n > (1i128 << 40) {
            return Err(format!("literal '{}' does not fit in 32 bits", tok));
        }
    }
    if neg {
        n = -n;
    }
    if n < -(1i128 << 31) || n > MASK as i128 {
        return Err(format!("literal '{}' does not fit in 32 bits", tok));
    }
    Ok(n as i64)
}

fn parse_operand(kind: u8, tok: &str) -> KResult<Arg> {
    match kind {
        b'd' => parse_reg(tok).map(Arg::R).ok_or_else(|| format!("expected a register but found '{}'", tok)),
        b's' => {
            if let Some(r) = parse_reg(tok) {
                return Ok(Arg::R(r));
            }
            Ok(Arg::I(parse_literal_raw(tok)? & MASK))
        }
        b'i' => Ok(Arg::I(parse_literal_raw(tok)? & MASK)),
        b'k' => {
            if parse_reg(tok).is_some() {
                return Err("the last operand of addsh must be a literal".into());
            }
            let n = parse_literal_raw(tok)?;
            if !(0..=31).contains(&n) {
                return Err("shift amount of addsh must be a literal in 0..31".into());
            }
            Ok(Arg::K(n))
        }
        _ => {
            if !is_name(tok) || parse_reg(tok).is_some() {
                return Err(format!("expected a label name but found '{}'", tok));
            }
            Ok(Arg::L(tok.to_string()))
        }
    }
}

/// Python's str.splitlines().
pub fn splitlines(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut it = s.char_indices().peekable();
    while let Some((i, c)) = it.next() {
        let brk = matches!(c, '\n' | '\r' | '\x0b' | '\x0c' | '\x1c' | '\x1d' | '\x1e' | '\u{85}' | '\u{2028}' | '\u{2029}');
        if brk {
            out.push(&s[start..i]);
            let mut next = i + c.len_utf8();
            if c == '\r' {
                if let Some(&(_, '\n')) = it.peek() {
                    it.next();
                    next += 1;
                }
            }
            start = next;
        }
    }
    if start < s.len() {
        out.push(&s[start..]);
    }
    out
}

fn parse_func(line: &str) -> Option<(String, String)> {
    let rest = line.strip_prefix("func")?;
    if !rest.starts_with(char::is_whitespace) {
        return None;
    }
    let rest = rest.trim_start();
    let end = rest.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).unwrap_or(rest.len());
    let name = &rest[..end];
    if !is_name(name) {
        return None;
    }
    let rest = rest[end..].trim_start().strip_prefix('(')?;
    let inner = rest.strip_suffix(')')?;
    Some((name.to_string(), inner.to_string()))
}

pub fn parse_program(text: &str) -> KResult<Program> {
    let mut prog = Program::default();
    let mut seen_func = false;
    let mut blocks: Vec<(String, Vec<PInstr>)> = Vec::new();
    let mut label_seen: std::collections::HashSet<String> = Default::default();
    for raw in splitlines(text) {
        let line = raw.split(';').next().unwrap().trim();
        if line.is_empty() {
            continue;
        }
        if let Some((name, plist)) = parse_func(line) {
            if seen_func {
                return Err("only one 'func' header is allowed".into());
            }
            if !blocks.is_empty() {
                return Err("'func' header must come before the first label".into());
            }
            seen_func = true;
            prog.name = name;
            let plist = plist.trim();
            let mut params: Vec<i64> = Vec::new();
            if !plist.is_empty() {
                for tok in plist.split(',') {
                    let tok = tok.trim();
                    let idx = parse_reg(tok).ok_or_else(|| format!("parameter '{}' is not a register", tok))?;
                    if params.contains(&idx) {
                        return Err(format!("parameter '{}' is listed twice", tok));
                    }
                    params.push(idx);
                }
            }
            if params.len() > MAX_PARAMS {
                return Err("too many parameters".into());
            }
            prog.params = params;
            continue;
        }
        if !seen_func {
            return Err("expected 'func name(params)' as the first statement".into());
        }
        if let Some(name) = line.strip_suffix(':') {
            if is_name(name) {
                if parse_reg(name).is_some() {
                    return Err("label looks like a register name".into());
                }
                if !label_seen.insert(name.to_string()) {
                    return Err(format!("label '{}' is defined twice", name));
                }
                blocks.push((name.to_string(), Vec::new()));
                continue;
            }
        }
        let (opname, rest) = match line.find(char::is_whitespace) {
            Some(i) => (&line[..i], line[i..].trim()),
            None => (line, ""),
        };
        let op = match Op::from_name(opname) {
            Some(o) => o,
            None => return Err(format!("unknown instruction '{}'", opname)),
        };
        if blocks.is_empty() {
            return Err("instruction outside of a block".into());
        }
        let toks: Vec<&str> = if rest.is_empty() { Vec::new() } else { rest.split(',').map(|t| t.trim()).collect() };
        let sig = op.sig().as_bytes();
        if toks.len() != sig.len() {
            return Err(format!("'{}' takes {} operand(s), found {}", opname, sig.len(), toks.len()));
        }
        let mut args = Vec::with_capacity(toks.len());
        for (k, tok) in sig.iter().zip(toks.iter()) {
            if tok.is_empty() {
                return Err("empty operand".into());
            }
            args.push(parse_operand(*k, tok)?);
        }
        blocks.last_mut().unwrap().1.push(PInstr { op, args });
    }
    if !seen_func {
        return Err("missing 'func name(params)' header".into());
    }
    if blocks.is_empty() {
        return Err("the function has no blocks".into());
    }
    let mut total = 0;
    for (label, instrs) in &blocks {
        if instrs.is_empty() {
            return Err(format!("block '{}' is empty", label));
        }
        for ins in &instrs[..instrs.len() - 1] {
            if ins.op.is_term() {
                return Err(format!("'{}' ends the block, but more instructions follow it", ins.op.name()));
            }
        }
        if !instrs[instrs.len() - 1].op.is_term() {
            return Err(format!("block '{}' must end with jmp, br, ret or trap", label));
        }
        total += instrs.len();
    }
    if total > MAX_INSTRS {
        return Err("program is too large".into());
    }
    for (bi, (label, instrs)) in blocks.into_iter().enumerate() {
        prog.labels.insert(label.clone(), prog.instrs.len());
        prog.block_labels.push(label);
        for ins in instrs {
            prog.instrs.push(ins);
            prog.block_of.push(bi);
        }
    }
    for ins in &prog.instrs {
        for a in &ins.args {
            if let Arg::L(l) = a {
                if !prog.labels.contains_key(l) {
                    return Err(format!("jump to undefined label '{}'", l));
                }
            }
        }
    }
    Ok(prog)
}

/// Read a text file. Accepts UTF-8, UTF-8 with BOM, and UTF-16 with BOM.
pub fn read_text(path: &str) -> KResult<String> {
    let data = std::fs::read(path).map_err(|e| format!("cannot read {}: {}", path, e))?;
    if data.starts_with(&[0xef, 0xbb, 0xbf]) {
        return String::from_utf8(data[3..].to_vec()).map_err(|e| e.to_string());
    }
    if data.starts_with(&[0xff, 0xfe]) || data.starts_with(&[0xfe, 0xff]) {
        let le = data[0] == 0xff;
        let units: Vec<u16> = data[2..]
            .chunks(2)
            .map(|c| if c.len() == 2 { if le { u16::from_le_bytes([c[0], c[1]]) } else { u16::from_be_bytes([c[0], c[1]]) } } else { 0 })
            .collect();
        return String::from_utf16(&units).map_err(|e| e.to_string());
    }
    String::from_utf8(data).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// machine.cfg
// ---------------------------------------------------------------------------
#[derive(Clone, Debug, Default)]
pub struct Machine {
    pub issue_width: i64,
    pub mem_ports: i64,
    pub registers: i64,
    pub reg_spill_penalty: i64,
    pub taken_branch_penalty: i64,
    pub icache_size: i64,
    pub icache_penalty: i64,
    pub lat_alu: i64,
    pub lat_mul: i64,
    pub lat_div: i64,
    pub lat_load: i64,
    pub lat_mac: i64,
    pub lat_addsh: i64,
    pub fused_mac: i64,
    pub fused_addsh: i64,
}

const MACHINE_KEYS: [(&str, i64); 15] = [
    ("issue_width", 1), ("mem_ports", 1), ("registers", 4), ("reg_spill_penalty", 0),
    ("taken_branch_penalty", 0), ("icache_size", 1), ("icache_penalty", 0), ("lat_alu", 1),
    ("lat_mul", 1), ("lat_div", 1), ("lat_load", 1), ("lat_mac", 1), ("lat_addsh", 1),
    ("fused_mac", 0), ("fused_addsh", 0),
];

impl Machine {
    pub fn lat(&self, op: Op) -> i64 {
        match op {
            Op::Mul => self.lat_mul,
            Op::Div | Op::Mod => self.lat_div,
            Op::Load | Op::Vload => self.lat_load,
            Op::Mac => self.lat_mac,
            Op::Addsh => self.lat_addsh,
            Op::Store | Op::Vstore | Op::Trap | Op::Jmp | Op::Br | Op::Ret | Op::Phi => 1,
            _ => self.lat_alu,
        }
    }

    fn set(&mut self, key: &str, v: i64) {
        match key {
            "issue_width" => self.issue_width = v,
            "mem_ports" => self.mem_ports = v,
            "registers" => self.registers = v,
            "reg_spill_penalty" => self.reg_spill_penalty = v,
            "taken_branch_penalty" => self.taken_branch_penalty = v,
            "icache_size" => self.icache_size = v,
            "icache_penalty" => self.icache_penalty = v,
            "lat_alu" => self.lat_alu = v,
            "lat_mul" => self.lat_mul = v,
            "lat_div" => self.lat_div = v,
            "lat_load" => self.lat_load = v,
            "lat_mac" => self.lat_mac = v,
            "lat_addsh" => self.lat_addsh = v,
            "fused_mac" => self.fused_mac = v,
            _ => self.fused_addsh = v,
        }
    }
}

pub fn load_machine(path: &str) -> KResult<Machine> {
    let text = read_text(path).map_err(|e| format!("cannot read machine config {}: {}", path, e))?;
    let mut mc = Machine::default();
    let mut seen: Vec<&str> = Vec::new();
    for raw in splitlines(&text) {
        let line = raw.split('#').next().unwrap().trim();
        if line.is_empty() {
            continue;
        }
        let (key, val) = match line.split_once('=') {
            Some((k, v)) => (k.trim(), v.trim()),
            None => return Err("expected 'key = value'".into()),
        };
        let min = match MACHINE_KEYS.iter().find(|(k, _)| *k == key) {
            Some((_, m)) => *m,
            None => return Err(format!("unknown key '{}'", key)),
        };
        if seen.contains(&key) {
            return Err(format!("key '{}' given twice", key));
        }
        if val.is_empty() || !val.bytes().all(|c| c.is_ascii_digit()) {
            return Err(format!("value of '{}' must be a non-negative integer", key));
        }
        let n: i64 = val.trim_start_matches('0').parse().unwrap_or(if val.bytes().all(|c| c == b'0') { 0 } else { i64::MAX });
        if n < min || n > 1_000_000 {
            return Err(format!("value of '{}' is out of range", key));
        }
        if key.starts_with("fused_") && n > 1 {
            return Err(format!("'{}' must be 0 or 1", key));
        }
        mc.set(key, n);
        seen.push(key);
    }
    for (k, _) in MACHINE_KEYS.iter() {
        if !seen.contains(k) {
            return Err(format!("missing key '{}'", k));
        }
    }
    Ok(mc)
}

pub fn check_machine_features(prog: &Program, mc: &Machine) -> KResult<()> {
    for ins in &prog.instrs {
        if ins.op == Op::Mac && mc.fused_mac == 0 {
            return Err("'mac' is not available on this machine".into());
        }
        if ins.op == Op::Addsh && mc.fused_addsh == 0 {
            return Err("'addsh' is not available on this machine".into());
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Compilation to an executable form
// ---------------------------------------------------------------------------
#[derive(Clone, Copy)]
pub struct Code {
    pub op: Op,
    pub d: usize,
    pub x: usize,
    pub y: usize,
    pub z: usize,
    pub t1: usize,
    pub t2: usize,
}

pub struct Compiled {
    pub code: Vec<Code>,
    pub srcs: Vec<Vec<usize>>,
    pub dst: Vec<i64>,
    pub opnames: Vec<Op>,
    pub touch: Vec<Vec<usize>>,
    pub nregs: usize,
    pub nparams: usize,
    pub r0: Vec<u32>,
}

pub fn compile_program(prog: &Program) -> Compiled {
    let mut regmap: HashMap<i64, usize> = HashMap::new();
    fn rid(m: &mut HashMap<i64, usize>, idx: i64) -> usize {
        let n = m.len();
        *m.entry(idx).or_insert(n)
    }
    for &p in &prog.params {
        rid(&mut regmap, p);
    }
    for ins in &prog.instrs {
        for a in &ins.args {
            if let Arg::R(r) = a {
                rid(&mut regmap, *r);
            }
        }
    }
    let nregs = regmap.len();
    let mut consts: HashMap<i64, usize> = HashMap::new();
    let mut const_list: Vec<u32> = Vec::new();
    let mut slot = |a: &Arg, regmap: &HashMap<i64, usize>| -> usize {
        match a {
            Arg::R(r) => regmap[r],
            Arg::I(v) | Arg::K(v) => {
                if let Some(&s) = consts.get(v) {
                    s
                } else {
                    let s = nregs + const_list.len();
                    consts.insert(*v, s);
                    const_list.push(*v as u32);
                    s
                }
            }
            Arg::L(_) => 0,
        }
    };
    let mut code = Vec::new();
    let mut srcs = Vec::new();
    let mut dst = Vec::new();
    let mut opnames = Vec::new();
    for ins in &prog.instrs {
        let a = &ins.args;
        let (mut d, mut x, mut y, mut z, mut t1, mut t2) = (0, 0, 0, 0, 0, 0);
        let mut src_slots: Vec<usize> = Vec::new();
        let mut dest: i64 = -1;
        let op = ins.op;
        match op {
            Op::Const => {
                d = slot(&a[0], &regmap);
                x = slot(&a[1], &regmap);
                dest = d as i64;
            }
            Op::Mov => {
                d = slot(&a[0], &regmap);
                x = slot(&a[1], &regmap);
                dest = d as i64;
                src_slots = vec![x];
            }
            Op::Add | Op::Sub | Op::Mul | Op::And | Op::Or | Op::Xor | Op::Shl | Op::Shr | Op::Lt
            | Op::Eq | Op::Div | Op::Mod => {
                d = slot(&a[0], &regmap);
                x = slot(&a[1], &regmap);
                y = slot(&a[2], &regmap);
                dest = d as i64;
                src_slots = vec![x, y];
            }
            Op::Select | Op::Mac => {
                d = slot(&a[0], &regmap);
                x = slot(&a[1], &regmap);
                y = slot(&a[2], &regmap);
                z = slot(&a[3], &regmap);
                dest = d as i64;
                src_slots = vec![x, y, z];
            }
            Op::Addsh => {
                d = slot(&a[0], &regmap);
                x = slot(&a[1], &regmap);
                y = slot(&a[2], &regmap);
                z = if let Arg::K(k) = a[3] { k as usize } else { 0 };
                dest = d as i64;
                src_slots = vec![x, y];
            }
            Op::Load | Op::Vload => {
                d = slot(&a[0], &regmap);
                x = slot(&a[1], &regmap);
                dest = d as i64;
                src_slots = vec![x];
            }
            Op::Store | Op::Vstore => {
                x = slot(&a[0], &regmap);
                y = slot(&a[1], &regmap);
                src_slots = vec![x, y];
            }
            Op::Trap | Op::Ret => {
                x = slot(&a[0], &regmap);
                src_slots = vec![x];
            }
            Op::Jmp => {
                if let Arg::L(l) = &a[0] {
                    t1 = prog.labels[l];
                }
            }
            Op::Br => {
                x = slot(&a[0], &regmap);
                if let (Arg::L(l1), Arg::L(l2)) = (&a[1], &a[2]) {
                    t1 = prog.labels[l1];
                    t2 = prog.labels[l2];
                }
                src_slots = vec![x];
            }
            Op::Phi => {}
        }
        let eop = if op == Op::Const { Op::Mov } else { op };
        code.push(Code { op: eop, d, x, y, z, t1, t2 });
        srcs.push(src_slots.into_iter().filter(|&s| s < nregs).collect::<Vec<_>>());
        dst.push(dest);
        opnames.push(op);
    }
    let mut touch = Vec::with_capacity(code.len());
    for i in 0..code.len() {
        let mut order: Vec<usize> = Vec::new();
        for &r in srcs[i].iter().chain(if dst[i] >= 0 { Some(dst[i] as usize) } else { None }.iter()) {
            if !order.contains(&r) {
                order.push(r);
            }
        }
        touch.push(order);
    }
    let mut r0 = vec![0u32; nregs];
    r0.extend(const_list);
    Compiled { code, srcs, dst, opnames, touch, nregs, nparams: prog.params.len(), r0 }
}

// ---------------------------------------------------------------------------
// Execution (exact semantics)
// ---------------------------------------------------------------------------
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Outcome {
    Ret(u32),
    DivZero,
    BadAddr,
    Explicit(u32),
    Timeout,
}

pub struct ExecResult {
    pub outcome: Outcome,
    pub vlog: Vec<(u8, u32, u32)>,
    pub mem: Vec<u32>,
    pub steps: u64,
    pub trace: Vec<u32>,
}

#[derive(Clone, Default)]
pub struct Inputs {
    pub args: Vec<u32>,
    pub segments: Vec<(usize, Vec<u32>)>,
    pub vin: BTreeMap<u32, Vec<u32>>,
}

pub fn execute(cp: &Compiled, inp: &Inputs, max_steps: u64, want_trace: bool) -> ExecResult {
    let mut r = cp.r0.clone();
    for (i, v) in inp.args.iter().enumerate() {
        r[i] = *v;
    }
    let mut mem = vec![0u32; MEM_WORDS];
    for (addr, words) in &inp.segments {
        mem[*addr..*addr + words.len()].copy_from_slice(words);
    }
    let mut queues: HashMap<u32, VecDeque<u32>> = HashMap::new();
    for (p, w) in &inp.vin {
        queues.insert(*p, w.iter().copied().collect());
    }
    let mut vlog: Vec<(u8, u32, u32)> = Vec::new();
    let mut trace: Vec<u32> = Vec::new();
    let code = &cp.code;
    let mut pc = 0usize;
    let mut steps: u64 = 0;
    macro_rules! done {
        ($o:expr) => {
            return ExecResult { outcome: $o, vlog, mem, steps, trace }
        };
    }
    loop {
        if steps >= max_steps {
            done!(Outcome::Timeout);
        }
        let c = code[pc];
        steps += 1;
        if want_trace {
            trace.push(pc as u32);
        }
        match c.op {
            Op::Add => { r[c.d] = r[c.x].wrapping_add(r[c.y]); pc += 1; }
            Op::Mov => { r[c.d] = r[c.x]; pc += 1; }
            Op::Lt => { r[c.d] = ((r[c.x] as i32) < (r[c.y] as i32)) as u32; pc += 1; }
            Op::Br => { pc = if r[c.x] != 0 { c.t1 } else { c.t2 }; }
            Op::Jmp => { pc = c.t1; }
            Op::Sub => { r[c.d] = r[c.x].wrapping_sub(r[c.y]); pc += 1; }
            Op::Mul => { r[c.d] = r[c.x].wrapping_mul(r[c.y]); pc += 1; }
            Op::And => { r[c.d] = r[c.x] & r[c.y]; pc += 1; }
            Op::Or => { r[c.d] = r[c.x] | r[c.y]; pc += 1; }
            Op::Xor => { r[c.d] = r[c.x] ^ r[c.y]; pc += 1; }
            Op::Shl => { r[c.d] = r[c.x] << (r[c.y] & 31); pc += 1; }
            Op::Shr => { r[c.d] = r[c.x] >> (r[c.y] & 31); pc += 1; }
            Op::Eq => { r[c.d] = (r[c.x] == r[c.y]) as u32; pc += 1; }
            Op::Select => { r[c.d] = if r[c.x] != 0 { r[c.y] } else { r[c.z] }; pc += 1; }
            Op::Load => {
                let addr = r[c.x] as usize;
                if addr >= MEM_WORDS {
                    done!(Outcome::BadAddr);
                }
                r[c.d] = mem[addr];
                pc += 1;
            }
            Op::Store => {
                let addr = r[c.x] as usize;
                if addr >= MEM_WORDS {
                    done!(Outcome::BadAddr);
                }
                mem[addr] = r[c.y];
                pc += 1;
            }
            Op::Mac => { r[c.d] = r[c.x].wrapping_mul(r[c.y]).wrapping_add(r[c.z]); pc += 1; }
            Op::Addsh => { r[c.d] = r[c.x].wrapping_add(r[c.y] << c.z); pc += 1; }
            Op::Div | Op::Mod => {
                let y = r[c.y];
                if y == 0 {
                    done!(Outcome::DivZero);
                }
                let sx = r[c.x] as i32 as i64;
                let sy = y as i32 as i64;
                r[c.d] = if c.op == Op::Div {
                    let mut q = sx.abs() / sy.abs();
                    if (sx < 0) != (sy < 0) {
                        q = -q;
                    }
                    q as u32
                } else {
                    let mut m = sx.abs() % sy.abs();
                    if sx < 0 {
                        m = -m;
                    }
                    m as u32
                };
                pc += 1;
            }
            Op::Vload => {
                let port = r[c.x];
                let val = queues.get_mut(&port).and_then(|q| q.pop_front()).unwrap_or(0);
                vlog.push((0, port, val));
                r[c.d] = val;
                pc += 1;
            }
            Op::Vstore => {
                vlog.push((1, r[c.x], r[c.y]));
                pc += 1;
            }
            Op::Trap => {
                done!(Outcome::Explicit(r[c.x]));
            }
            _ => {
                done!(Outcome::Ret(r[c.x]));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Timing model
// ---------------------------------------------------------------------------
/// Exact cycle count of a finished run (trace = executed instruction indices).
pub fn time_trace(cp: &Compiled, trace: &[u32], mc: &Machine) -> i64 {
    let w = mc.issue_width;
    let mp = mc.mem_ports;
    let k = mc.registers as usize;
    let sp = mc.reg_spill_penalty;
    let tb = mc.taken_branch_penalty;
    let ic = mc.icache_size as usize;
    let icp = mc.icache_penalty;
    let lat: Vec<i64> = cp.opnames.iter().map(|&o| mc.lat(o)).collect();
    let mut ready = vec![0i64; cp.nregs];
    let track = cp.nregs > k;
    // LRU of the registers that are resident: stamp per register, oldest first
    let mut stamp = vec![0u64; cp.nregs];
    let mut lru: BTreeMap<u64, usize> = BTreeMap::new();
    let mut spilled = vec![false; cp.nregs];
    let mut counter = 0u64;
    if track {
        for r in 0..cp.nparams {
            if lru.len() >= k {
                let (&s, &old) = lru.iter().next().unwrap();
                lru.remove(&s);
                stamp[old] = 0;
                spilled[old] = true;
            }
            counter += 1;
            stamp[r] = counter;
            lru.insert(counter, r);
        }
    }
    let mut last = 0i64;
    let mut used = 0i64;
    let mut memused = 0i64;
    let mut fence = 0i64;
    let n = trace.len();
    for idx in 0..n {
        let pc = trace[idx] as usize;
        let mut f = if pc >= ic { icp } else { 0 };
        if track {
            for &r in &cp.touch[pc] {
                if stamp[r] != 0 {
                    lru.remove(&stamp[r]);
                    counter += 1;
                    stamp[r] = counter;
                    lru.insert(counter, r);
                } else {
                    if spilled[r] {
                        f += sp;
                        spilled[r] = false;
                    }
                    if lru.len() >= k {
                        let (&s, &old) = lru.iter().next().unwrap();
                        lru.remove(&s);
                        stamp[old] = 0;
                        spilled[old] = true;
                    }
                    counter += 1;
                    stamp[r] = counter;
                    lru.insert(counter, r);
                }
            }
        }
        let mut t = last + f;
        if fence > t {
            t = fence;
        }
        for &r in &cp.srcs[pc] {
            if ready[r] > t {
                t = ready[r];
            }
        }
        let d = cp.dst[pc];
        if d >= 0 && ready[d as usize] > t {
            t = ready[d as usize];
        }
        let m = cp.opnames[pc].is_mem();
        if t == last && (used >= w || (m && memused >= mp)) {
            t += 1;
        }
        if t > last {
            used = 0;
            memused = 0;
        }
        used += 1;
        if m {
            memused += 1;
        }
        last = t;
        if d >= 0 {
            ready[d as usize] = t + lat[pc];
        }
        if cp.opnames[pc].is_term() && idx + 1 < n && trace[idx + 1] as usize != pc + 1 {
            fence = t + 1 + tb;
        }
    }
    last + 1
}

// ---------------------------------------------------------------------------
// Comparison and input generation
// ---------------------------------------------------------------------------
/// True if the two runs differ in outcome, volatile log or data memory.
pub fn differs(a: &ExecResult, b: &ExecResult) -> bool {
    a.outcome != b.outcome || a.vlog != b.vlog || a.mem[..SCRATCH_BASE] != b.mem[..SCRATCH_BASE]
}

/// splitmix64: deterministic.
pub struct Rng {
    s: u64,
}

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng { s: seed }
    }
    pub fn next(&mut self) -> u64 {
        self.s = self.s.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.s;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

const EDGE_VALUES: [u32; 20] = [
    0, 1, 2, 3, 7, 8, 15, 16, 31, 32, 63, 64, 255, 256, 1000, 65535, 0x7FFF_FFFF, 0x8000_0000,
    0xFFFF_FFFE, 0xFFFF_FFFF,
];
const SPECIAL_PAIRS: [(u32, u32); 16] = [
    (0x8000_0000, 0xFFFF_FFFF), (0xFFFF_FFFF, 0x8000_0000), (0x7FFF_FFFF, 0xFFFF_FFFF), (1, 0),
    (0, 1), (0xFFFF_FFFF, 1), (7, 0xFFFF_FFFE), (0xFFFF_FFF9, 2), (7, 2), (0x8000_0000, 1),
    (0x8000_0000, 2), (0x7FFF_FFFF, 1), (0x8000_0000, 31), (1, 32), (1, 33), (0xFFFF_FFFF, 31),
];

pub fn generate_inputs(nparams: usize, count: usize, seed: u64) -> Vec<Inputs> {
    let mut rng = Rng::new(seed);
    let mut out = Vec::new();
    for i in 0..count {
        let mut args: Vec<u32> = Vec::new();
        if i < EDGE_VALUES.len() {
            args = vec![EDGE_VALUES[i]; nparams];
        } else if i < EDGE_VALUES.len() + SPECIAL_PAIRS.len() {
            let pair = SPECIAL_PAIRS[i - EDGE_VALUES.len()];
            for j in 0..nparams {
                args.push(match j {
                    0 => pair.0,
                    1 => pair.1,
                    _ => EDGE_VALUES[(i + j) % EDGE_VALUES.len()],
                });
            }
        } else {
            for _ in 0..nparams {
                let r = rng.below(10);
                if r < 6 {
                    args.push(rng.below(16) as u32);
                } else if r < 8 {
                    args.push(EDGE_VALUES[rng.below(EDGE_VALUES.len() as u64) as usize]);
                } else {
                    args.push((rng.next() & MASK as u64) as u32);
                }
            }
        }
        let mode = rng.below(4);
        let bound: Option<u64> = [Some(16), Some(1000), None, Some(256)][mode as usize];
        let mut words = Vec::with_capacity(GEN_MEM_WORDS);
        for _ in 0..GEN_MEM_WORDS {
            words.push(match bound {
                None => (rng.next() & MASK as u64) as u32,
                Some(b) => rng.below(b) as u32,
            });
        }
        let mut vin = BTreeMap::new();
        for port in 0..4u32 {
            let v: Vec<u32> = (0..16).map(|_| rng.below(1000) as u32).collect();
            vin.insert(port, v);
        }
        out.push(Inputs { args, segments: vec![(0, words)], vin });
    }
    out
}
