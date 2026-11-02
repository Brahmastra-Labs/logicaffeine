//! Hierarchical names, interned: `Nat.succ` is `Str(Str(Anon, "Nat"), "succ")`, and each distinct
//! name is one [`NameId`], so name equality is an integer compare.

use logicaffeine_base::{Interner, Symbol};
use rustc_hash::FxHashMap as FxMap;

#[derive(Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct NameId(pub u32);

impl NameId {
    pub const ANON: NameId = NameId(0);
}

#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub enum NameNode {
    Anon,
    Str(NameId, Symbol),
    Num(NameId, u64),
}

#[derive(Clone)]
pub struct Names {
    nodes: Vec<NameNode>,
    index: FxMap<NameNode, NameId>,
    strings: Interner,
}

impl Default for Names {
    fn default() -> Self {
        Self::new()
    }
}

impl Names {
    pub fn new() -> Names {
        let mut index = FxMap::default();
        index.insert(NameNode::Anon, NameId::ANON);
        Names {
            nodes: vec![NameNode::Anon],
            index,
            strings: Interner::new(),
        }
    }

    pub fn intern(&mut self, node: NameNode) -> NameId {
        if let Some(&id) = self.index.get(&node) {
            return id;
        }
        let id = NameId(self.nodes.len() as u32);
        self.nodes.push(node);
        self.index.insert(node, id);
        id
    }

    pub fn str(&mut self, prefix: NameId, s: &str) -> NameId {
        let sym = self.strings.intern(s);
        self.intern(NameNode::Str(prefix, sym))
    }

    pub fn component(&self, sym: Symbol) -> &str {
        self.strings.resolve(sym)
    }

    pub fn num(&mut self, prefix: NameId, n: u64) -> NameId {
        self.intern(NameNode::Num(prefix, n))
    }

    /// Interns a dotted name such as `Nat.succ`. Only for well-known names written in source.
    pub fn dotted(&mut self, s: &str) -> NameId {
        s.split('.').fold(NameId::ANON, |p, part| self.str(p, part))
    }

    pub fn node(&self, id: NameId) -> &NameNode {
        &self.nodes[id.0 as usize]
    }

    pub fn prefix(&self, id: NameId) -> NameId {
        match self.node(id) {
            NameNode::Anon => NameId::ANON,
            NameNode::Str(p, _) | NameNode::Num(p, _) => *p,
        }
    }

    /// `true` when `p` is a (non-strict) prefix of `n`.
    pub fn is_prefix_of(&self, p: NameId, mut n: NameId) -> bool {
        loop {
            if n == p {
                return true;
            }
            if n == NameId::ANON {
                return false;
            }
            n = self.prefix(n);
        }
    }

    /// Replaces prefix `old` of `n` by `new` (`n` unchanged when `old` is not a prefix).
    pub fn replace_prefix(&mut self, n: NameId, old: NameId, new: NameId) -> NameId {
        if n == old {
            return new;
        }
        match *self.node(n) {
            NameNode::Anon => n,
            NameNode::Str(p, s) => {
                let p2 = self.replace_prefix(p, old, new);
                if p2 == p {
                    n
                } else {
                    self.intern(NameNode::Str(p2, s))
                }
            }
            NameNode::Num(p, k) => {
                let p2 = self.replace_prefix(p, old, new);
                if p2 == p {
                    n
                } else {
                    self.intern(NameNode::Num(p2, k))
                }
            }
        }
    }

    /// Lean's `Name.appendIndexAfter`: `u` ↦ `u_1`.
    pub fn append_index_after(&mut self, n: NameId, i: u64) -> NameId {
        match *self.node(n) {
            NameNode::Str(p, s) => {
                let text = format!("{}_{i}", self.component(s));
                self.str(p, &text)
            }
            _ => self.str(n, &format!("_{i}")),
        }
    }

    /// Lean's `Name.append`.
    pub fn append(&mut self, a: NameId, b: NameId) -> NameId {
        match *self.node(b) {
            NameNode::Anon => a,
            NameNode::Str(p, s) => {
                let p2 = self.append(a, p);
                self.intern(NameNode::Str(p2, s))
            }
            NameNode::Num(p, k) => {
                let p2 = self.append(a, p);
                self.intern(NameNode::Num(p2, k))
            }
        }
    }

    pub fn display(&self, id: NameId) -> String {
        match self.node(id) {
            NameNode::Anon => "[anonymous]".to_string(),
            NameNode::Str(p, s) => {
                let s = self.component(*s);
                if *p == NameId::ANON {
                    s.to_string()
                } else {
                    format!("{}.{}", self.display(*p), s)
                }
            }
            NameNode::Num(p, k) => {
                if *p == NameId::ANON {
                    k.to_string()
                } else {
                    format!("{}.{}", self.display(*p), k)
                }
            }
        }
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.len() <= 1
    }
}

/// The kernel spelling of a Lean name given as its hierarchical components: the name the Lean
/// importer gives a declaration in a kernel context, each component written after `/s:` as a JSON
/// string, with the kernel's primitive identities applied (`Nat`, `Zero`, `Succ`, the quotient's
/// four, and every other `Nat` member under `Nat/`).
///
/// Every component is a string component; a numeric Lean component cannot be told apart from a
/// digit string by its dotted spelling, so callers holding an export resolve such names through
/// the export's own name table (`interface::lean::reference::reference_declaration`).
pub fn kernel_name_of_components(components: &[&str]) -> String {
    let mut raw = String::new();
    for component in components {
        raw.push_str("/s:");
        push_json_string(&mut raw, component);
    }
    primitive_identity(&raw)
}

/// `s` as a JSON string literal, byte for byte as `serde_json` writes it: `"` and `\` escaped,
/// the control characters below U+0020 as `\b`, `\t`, `\n`, `\f`, `\r` or `\u00xx`, and every other
/// character as itself.
fn push_json_string(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{08}' => out.push_str("\\b"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\u{0c}' => out.push_str("\\f"),
            '\r' => out.push_str("\\r"),
            c if (c as u32) < 0x20 => {
                const HEX: &[u8; 16] = b"0123456789abcdef";
                out.push_str("\\u00");
                out.push(HEX[(c as usize) >> 4] as char);
                out.push(HEX[(c as usize) & 0xf] as char);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// The kernel's primitive identities, applied to a raw hierarchical spelling.
fn primitive_identity(raw: &str) -> String {
    const NAT: &str = "/s:\"Nat\"";
    match raw {
        "/s:\"Quot\"" => "kernel.quot.type".into(),
        "/s:\"Quot\"/s:\"mk\"" => "kernel.quot.mk".into(),
        "/s:\"Quot\"/s:\"lift\"" => "kernel.quot.lift".into(),
        "/s:\"Quot\"/s:\"ind\"" => "kernel.quot.ind".into(),
        NAT => "Nat".into(),
        "/s:\"Nat\"/s:\"zero\"" => "Zero".into(),
        "/s:\"Nat\"/s:\"succ\"" => "Succ".into(),
        _ => match raw.strip_prefix("/s:\"Nat\"/") {
            Some(rest) => format!("Nat/{rest}"),
            None => raw.to_string(),
        },
    }
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_kernel/dag/name/tests.rs"]
mod tests;
