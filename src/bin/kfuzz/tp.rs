//! tp.rs - a light text-level program structure (blocks of instructions) used by the
//! mutators and the minimiser. It does not interpret anything.
use crate::kir::*;

#[derive(Clone, Debug)]
pub struct TIns {
    pub op: String,
    pub args: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct TBlock {
    pub label: String,
    pub ins: Vec<TIns>,
}

#[derive(Clone, Debug)]
pub struct TProg {
    pub name: String,
    pub params: Vec<String>,
    pub blocks: Vec<TBlock>,
}

impl TIns {
    pub fn new(op: &str, args: &[&str]) -> TIns {
        TIns { op: op.to_string(), args: args.iter().map(|s| s.to_string()).collect() }
    }
    pub fn opc(&self) -> Option<Op> {
        Op::from_name(&self.op)
    }
    pub fn sig(&self) -> &'static str {
        self.opc().map(|o| o.sig()).unwrap_or("")
    }
    pub fn is_term(&self) -> bool {
        self.opc().map_or(false, |o| o.is_term())
    }
    /// index of the destination argument, if any
    pub fn dest(&self) -> Option<usize> {
        if self.sig().starts_with('d') {
            Some(0)
        } else {
            None
        }
    }
    pub fn text(&self) -> String {
        if self.args.is_empty() {
            self.op.clone()
        } else {
            format!("{} {}", self.op, self.args.join(", "))
        }
    }
}

pub fn is_reg(s: &str) -> bool {
    parse_reg(s).is_some()
}

impl TProg {
    pub fn parse(text: &str) -> Result<TProg, String> {
        // validate with the real parser first
        parse_program(text)?;
        let mut tp = TProg { name: String::new(), params: Vec::new(), blocks: Vec::new() };
        for raw in splitlines(text) {
            let line = raw.split(';').next().unwrap().trim();
            if line.is_empty() {
                continue;
            }
            if let Some(rest) = line.strip_prefix("func") {
                if rest.starts_with(char::is_whitespace) {
                    let rest = rest.trim();
                    let lp = rest.find('(').ok_or("bad func")?;
                    tp.name = rest[..lp].trim().to_string();
                    let inner = rest[lp + 1..].trim_end_matches(')').trim();
                    tp.params = if inner.is_empty() { vec![] } else { inner.split(',').map(|s| s.trim().to_string()).collect() };
                    continue;
                }
            }
            if let Some(l) = line.strip_suffix(':') {
                tp.blocks.push(TBlock { label: l.to_string(), ins: Vec::new() });
                continue;
            }
            let (op, rest) = match line.find(char::is_whitespace) {
                Some(i) => (&line[..i], line[i..].trim()),
                None => (line, ""),
            };
            let args = if rest.is_empty() { vec![] } else { rest.split(',').map(|t| t.trim().to_string()).collect() };
            tp.blocks.last_mut().ok_or("ins outside block")?.ins.push(TIns { op: op.to_string(), args });
        }
        Ok(tp)
    }

    pub fn to_text(&self) -> String {
        let mut s = String::new();
        s.push_str(&format!("func {}({})\n", self.name, self.params.join(", ")));
        for b in &self.blocks {
            s.push_str(&format!("{}:\n", b.label));
            for i in &b.ins {
                s.push_str(&format!("  {}\n", i.text()));
            }
        }
        s
    }

    pub fn count(&self) -> usize {
        self.blocks.iter().map(|b| b.ins.len()).sum()
    }

    /// Labels that are the target of some jump/branch.
    pub fn referenced(&self) -> std::collections::HashSet<String> {
        let mut s = std::collections::HashSet::new();
        for b in &self.blocks {
            for i in &b.ins {
                if let Some(o) = i.opc() {
                    let sig = o.sig().as_bytes();
                    for (k, c) in sig.iter().enumerate() {
                        if *c == b'L' {
                            s.insert(i.args[k].clone());
                        }
                    }
                }
            }
        }
        s
    }
}
