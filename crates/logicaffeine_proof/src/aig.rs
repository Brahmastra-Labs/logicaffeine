//! And-inverter graphs: a circuit as two-input AND gates over complemented edges, read and written in the AIGER
//! format (Biere, *The AIGER And-Inverter Graph Format, Version 20071012*, and its 1.9 extension).
//!
//! An [`Aig`] is built bottom-up and kept in topological order: every gate is created after its operands, so a
//! gate's node index exceeds both of theirs. [`Aig::and`] hashes structurally (Kuehlmann & Krohm, DAC 1997): an
//! operand pair it has seen returns the existing gate, and a constant, repeated or complementary operand folds away,
//! so two circuits built into one graph share every gate they compute the same way. Inputs and latches are leaves;
//! a latch's next-state literal may name any node, which is how a sequential circuit closes its loops without
//! breaking the order.
//!
//! [`Aig::parse`] reads the ASCII (`aag`) and binary (`aig`) forms, including latch initial values, bad-state and
//! invariant-constraint sections and the symbol table, and refuses justice and fairness properties by name. ASCII
//! gates may be defined in any order; they are rebuilt in dependency order without recursion, so a chain of any
//! depth reads. [`Aig::to_ascii`] and [`Aig::to_binary`] write the canonical numbering: inputs, then latches, then
//! gates in node order.
//!
//! [`Aig::eval`] evaluates one assignment and [`Aig::simulate`] sixty-four assignments a word over any number of
//! words: the signatures SAT sweeping ([`crate::sweep`]) groups candidate equivalences by.

use std::collections::HashMap;

/// A literal of an and-inverter graph: node `n` as `2n`, its complement as `2n + 1`, the AIGER encoding. Node 0 is
/// the constant false.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct AigLit(u32);

impl AigLit {
    pub const FALSE: AigLit = AigLit(0);
    pub const TRUE: AigLit = AigLit(1);

    pub fn new(node: u32, complemented: bool) -> AigLit {
        AigLit(node << 1 | complemented as u32)
    }

    /// The literal whose AIGER code is `code`.
    pub fn from_code(code: u32) -> AigLit {
        AigLit(code)
    }

    pub fn node(self) -> u32 {
        self.0 >> 1
    }

    pub fn is_complemented(self) -> bool {
        self.0 & 1 == 1
    }

    pub fn code(self) -> u32 {
        self.0
    }

    /// This literal, complemented when `flip` is set.
    pub fn with_phase(self, flip: bool) -> AigLit {
        AigLit(self.0 ^ flip as u32)
    }
}

impl std::ops::Not for AigLit {
    type Output = AigLit;
    fn not(self) -> AigLit {
        AigLit(self.0 ^ 1)
    }
}

/// What a node is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Node {
    False,
    /// The input at this index of [`Aig::inputs`].
    Input(usize),
    /// The latch at this index of [`Aig::latches`].
    Latch(usize),
    /// The conjunction of two literals of lower-numbered nodes, the larger code first.
    And(AigLit, AigLit),
}

/// A latch's value before the first clock.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Init {
    Zero,
    One,
    /// Any value: the AIGER file writes the latch's own literal.
    Free,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Latch {
    pub node: u32,
    pub next: AigLit,
    pub init: Init,
    pub name: Option<String>,
}

/// An input (its node, uncomplemented), output, bad-state property or invariant constraint, with its symbol.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Port {
    pub lit: AigLit,
    pub name: Option<String>,
}

/// A leaf of a graph being imported into another ([`Aig::import`]).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Leaf {
    Input(usize),
    Latch(usize),
}

/// Why an AIGER file was refused. Lines count from 1.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AigerError {
    Header(String),
    /// A literal outside the range the header allows, or of the wrong parity for its place.
    Literal { line: usize, literal: u64 },
    /// A literal whose variable nothing defines.
    Undefined { line: usize, var: u64 },
    /// A variable defined twice.
    Redefined { line: usize, var: u64 },
    /// A gate that depends on itself.
    Cycle { var: u64 },
    /// A line that is not what its place in the file calls for.
    Malformed { line: usize },
    Unsupported(String),
    Truncated,
}

impl std::fmt::Display for AigerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AigerError::Header(why) => write!(f, "AIGER header: {why}"),
            AigerError::Literal { line, literal } => write!(f, "line {line}: literal {literal} is out of place"),
            AigerError::Undefined { line, var } => write!(f, "line {line}: variable {var} is never defined"),
            AigerError::Redefined { line, var } => write!(f, "line {line}: variable {var} is defined twice"),
            AigerError::Cycle { var } => write!(f, "gate {var} depends on itself"),
            AigerError::Malformed { line } => write!(f, "line {line} is malformed"),
            AigerError::Unsupported(what) => write!(f, "unsupported: {what}"),
            AigerError::Truncated => write!(f, "the file ends early"),
        }
    }
}

impl std::error::Error for AigerError {}

/// An and-inverter graph with its inputs, latches, outputs, bad-state properties, invariant constraints and
/// comments.
#[derive(Clone, Debug)]
pub struct Aig {
    nodes: Vec<Node>,
    inputs: Vec<Port>,
    latches: Vec<Latch>,
    outputs: Vec<Port>,
    bad: Vec<Port>,
    constraints: Vec<Port>,
    comments: Vec<String>,
    strash: HashMap<(AigLit, AigLit), u32>,
}

impl Default for Aig {
    fn default() -> Aig {
        Aig::new()
    }
}

impl Aig {
    /// A graph holding only the constant node.
    pub fn new() -> Aig {
        Aig {
            nodes: vec![Node::False],
            inputs: Vec::new(),
            latches: Vec::new(),
            outputs: Vec::new(),
            bad: Vec::new(),
            constraints: Vec::new(),
            comments: Vec::new(),
            strash: HashMap::new(),
        }
    }

    fn push(&mut self, node: Node) -> u32 {
        let n = self.nodes.len() as u32;
        self.nodes.push(node);
        n
    }

    pub fn add_input(&mut self, name: Option<String>) -> AigLit {
        let n = self.push(Node::Input(self.inputs.len()));
        let lit = AigLit::new(n, false);
        self.inputs.push(Port { lit, name });
        lit
    }

    /// A latch whose next state is false until [`Aig::set_next`]; returns its index.
    pub fn add_latch(&mut self, name: Option<String>, init: Init) -> usize {
        let k = self.latches.len();
        let node = self.push(Node::Latch(k));
        self.latches.push(Latch { node, next: AigLit::FALSE, init, name });
        k
    }

    /// The latch's current-state literal.
    pub fn latch_lit(&self, latch: usize) -> AigLit {
        AigLit::new(self.latches[latch].node, false)
    }

    pub fn set_next(&mut self, latch: usize, next: AigLit) {
        self.latches[latch].next = next;
    }

    /// `a ∧ b`, folded and structurally hashed.
    pub fn and(&mut self, a: AigLit, b: AigLit) -> AigLit {
        let (hi, lo) = if a >= b { (a, b) } else { (b, a) };
        if lo == AigLit::FALSE || hi == !lo {
            return AigLit::FALSE;
        }
        if lo == AigLit::TRUE || hi == lo {
            return hi;
        }
        if let Some(&n) = self.strash.get(&(hi, lo)) {
            return AigLit::new(n, false);
        }
        let n = self.push(Node::And(hi, lo));
        self.strash.insert((hi, lo), n);
        AigLit::new(n, false)
    }

    pub fn or(&mut self, a: AigLit, b: AigLit) -> AigLit {
        !self.and(!a, !b)
    }

    pub fn xor(&mut self, a: AigLit, b: AigLit) -> AigLit {
        let l = self.and(a, !b);
        let r = self.and(!a, b);
        self.or(l, r)
    }

    pub fn xnor(&mut self, a: AigLit, b: AigLit) -> AigLit {
        !self.xor(a, b)
    }

    /// `s ? t : e`.
    pub fn mux(&mut self, s: AigLit, t: AigLit, e: AigLit) -> AigLit {
        let l = self.and(s, t);
        let r = self.and(!s, e);
        self.or(l, r)
    }

    pub fn add_output(&mut self, lit: AigLit, name: Option<String>) {
        self.outputs.push(Port { lit, name });
    }

    pub fn add_bad(&mut self, lit: AigLit, name: Option<String>) {
        self.bad.push(Port { lit, name });
    }

    pub fn add_constraint(&mut self, lit: AigLit, name: Option<String>) {
        self.constraints.push(Port { lit, name });
    }

    pub fn node(&self, n: u32) -> Node {
        self.nodes[n as usize]
    }

    pub fn num_nodes(&self) -> usize {
        self.nodes.len()
    }

    pub fn num_ands(&self) -> usize {
        self.nodes.iter().filter(|n| matches!(n, Node::And(..))).count()
    }

    pub fn inputs(&self) -> &[Port] {
        &self.inputs
    }

    pub fn latches(&self) -> &[Latch] {
        &self.latches
    }

    pub fn outputs(&self) -> &[Port] {
        &self.outputs
    }

    pub fn bad(&self) -> &[Port] {
        &self.bad
    }

    pub fn constraints(&self) -> &[Port] {
        &self.constraints
    }

    pub fn comments(&self) -> &[String] {
        &self.comments
    }

    pub fn input_index(&self, name: &str) -> Option<usize> {
        self.inputs.iter().position(|p| p.name.as_deref() == Some(name))
    }

    pub fn output_index(&self, name: &str) -> Option<usize> {
        self.outputs.iter().position(|p| p.name.as_deref() == Some(name))
    }

    /// The value of every node under one assignment of the inputs and the latches.
    pub fn eval(&self, inputs: &[bool], latches: &[bool]) -> Vec<bool> {
        let mut v = vec![false; self.nodes.len()];
        for (n, node) in self.nodes.iter().enumerate() {
            v[n] = match *node {
                Node::False => false,
                Node::Input(i) => inputs[i],
                Node::Latch(k) => latches[k],
                Node::And(a, b) => Aig::value(&v, a) && Aig::value(&v, b),
            };
        }
        v
    }

    /// A literal's value among [`Aig::eval`]'s node values.
    pub fn value(values: &[bool], lit: AigLit) -> bool {
        values[lit.node() as usize] ^ lit.is_complemented()
    }

    /// Every node's value under `64 · words` assignments at once: input `i`'s words are
    /// `inputs[i·words .. (i+1)·words]`, likewise each latch's. The result holds node `n`'s words at
    /// `n·words .. (n+1)·words`.
    pub fn simulate(&self, words: usize, inputs: &[u64], latches: &[u64]) -> Vec<u64> {
        let mut v = vec![0u64; self.nodes.len() * words];
        for (n, node) in self.nodes.iter().enumerate() {
            match *node {
                Node::False => {}
                Node::Input(i) => v[n * words..(n + 1) * words].copy_from_slice(&inputs[i * words..(i + 1) * words]),
                Node::Latch(k) => {
                    v[n * words..(n + 1) * words].copy_from_slice(&latches[k * words..(k + 1) * words])
                }
                Node::And(a, b) => {
                    for w in 0..words {
                        v[n * words + w] = Aig::word(&v, words, a, w) & Aig::word(&v, words, b, w);
                    }
                }
            }
        }
        v
    }

    /// Word `w` of a literal's value in a [`Aig::simulate`] result.
    pub fn word(sim: &[u64], words: usize, lit: AigLit, w: usize) -> u64 {
        let x = sim[lit.node() as usize * words + w];
        if lit.is_complemented() {
            !x
        } else {
            x
        }
    }

    /// The nodes the roots depend on, themselves included, in increasing order.
    pub fn cone(&self, roots: &[AigLit]) -> Vec<u32> {
        let mut seen = vec![false; self.nodes.len()];
        let mut stack: Vec<u32> = roots.iter().map(|l| l.node()).collect();
        let mut out = Vec::new();
        while let Some(n) = stack.pop() {
            if std::mem::replace(&mut seen[n as usize], true) {
                continue;
            }
            out.push(n);
            if let Node::And(a, b) = self.nodes[n as usize] {
                stack.push(a.node());
                stack.push(b.node());
            }
        }
        out.sort_unstable();
        out
    }

    /// The graph with only the nodes something reads: every input and latch in order, the outputs, bad and
    /// constraint properties and latches' next states, and the nodes in their cones; with each node's literal in the
    /// result, `None` for one nothing reads.
    pub fn compact(&self) -> (Aig, Vec<Option<AigLit>>) {
        self.compact_keeping(&[])
    }

    /// [`Aig::compact`], the cones of `keep` kept as well.
    pub fn compact_keeping(&self, keep: &[AigLit]) -> (Aig, Vec<Option<AigLit>>) {
        let mut live = vec![false; self.nodes.len()];
        let mut stack: Vec<u32> = self
            .outputs
            .iter()
            .chain(&self.bad)
            .chain(&self.constraints)
            .map(|p| p.lit.node())
            .chain(self.latches.iter().map(|l| l.next.node()))
            .chain(keep.iter().map(|l| l.node()))
            .collect();
        while let Some(n) = stack.pop() {
            if std::mem::replace(&mut live[n as usize], true) {
                continue;
            }
            if let Node::And(a, b) = self.nodes[n as usize] {
                stack.push(a.node());
                stack.push(b.node());
            }
        }
        let mut out = Aig::new();
        let inputs: Vec<AigLit> = self.inputs.iter().map(|p| out.add_input(p.name.clone())).collect();
        let latches: Vec<usize> = self.latches.iter().map(|l| out.add_latch(l.name.clone(), l.init)).collect();
        let mut map: Vec<Option<AigLit>> = vec![None; self.nodes.len()];
        map[0] = Some(AigLit::FALSE);
        for (n, node) in self.nodes.iter().enumerate() {
            map[n] = match *node {
                Node::False => Some(AigLit::FALSE),
                Node::Input(i) => Some(inputs[i]),
                Node::Latch(k) => Some(out.latch_lit(latches[k])),
                Node::And(a, b) if live[n] => {
                    let at = |l: AigLit, map: &[Option<AigLit>]| map[l.node() as usize].expect("a live node's fan-ins are live").with_phase(l.is_complemented());
                    let (x, y) = (at(a, &map), at(b, &map));
                    Some(out.and(x, y))
                }
                Node::And(..) => None,
            };
        }
        let at = |l: AigLit| map[l.node() as usize].expect("read, so live").with_phase(l.is_complemented());
        for (k, l) in self.latches.iter().enumerate() {
            out.set_next(latches[k], at(l.next));
        }
        for p in &self.outputs {
            out.add_output(at(p.lit), p.name.clone());
        }
        for p in &self.bad {
            out.add_bad(at(p.lit), p.name.clone());
        }
        for p in &self.constraints {
            out.add_constraint(at(p.lit), p.name.clone());
        }
        out.comments = self.comments.clone();
        (out, map)
    }

    /// The graph the outputs `outputs` (by index) read through any number of clocks: those outputs, in the order given;
    /// every latch whose value reaches one of them (a latch a kept latch's next state reads, to a fixpoint), in order,
    /// with its name and initial value; every input, in order; and the nodes their cones need.
    pub fn sequential_cone(&self, outputs: &[usize]) -> Aig {
        let mut live = vec![false; self.nodes.len()];
        let mut kept = vec![false; self.latches.len()];
        let mut stack: Vec<u32> = outputs.iter().map(|&o| self.outputs[o].lit.node()).collect();
        while let Some(n) = stack.pop() {
            if std::mem::replace(&mut live[n as usize], true) {
                continue;
            }
            match self.nodes[n as usize] {
                Node::And(a, b) => {
                    stack.push(a.node());
                    stack.push(b.node());
                }
                Node::Latch(k) => {
                    kept[k] = true;
                    stack.push(self.latches[k].next.node());
                }
                Node::False | Node::Input(_) => {}
            }
        }
        let mut out = Aig::new();
        let inputs: Vec<AigLit> = self.inputs.iter().map(|p| out.add_input(p.name.clone())).collect();
        let latches: Vec<Option<usize>> =
            self.latches.iter().zip(&kept).map(|(l, &k)| k.then(|| out.add_latch(l.name.clone(), l.init))).collect();
        let mut map: Vec<Option<AigLit>> = vec![None; self.nodes.len()];
        for (n, node) in self.nodes.iter().enumerate() {
            map[n] = match *node {
                Node::False => Some(AigLit::FALSE),
                Node::Input(i) => Some(inputs[i]),
                Node::Latch(k) => latches[k].map(|j| out.latch_lit(j)),
                Node::And(a, b) if live[n] => {
                    let at = |l: AigLit, map: &[Option<AigLit>]| map[l.node() as usize].expect("a live node's fan-ins are live").with_phase(l.is_complemented());
                    let (x, y) = (at(a, &map), at(b, &map));
                    Some(out.and(x, y))
                }
                Node::And(..) => None,
            };
        }
        let at = |l: AigLit| map[l.node() as usize].expect("in the cone, so live").with_phase(l.is_complemented());
        for (k, l) in self.latches.iter().enumerate() {
            if let Some(j) = latches[k] {
                out.set_next(j, at(l.next));
            }
        }
        for &o in outputs {
            out.add_output(at(self.outputs[o].lit), self.outputs[o].name.clone());
        }
        out
    }

    /// Copy every node of `other` into this graph, each leaf replaced by the literal `leaves` gives it. Returns the
    /// literal each of `other`'s nodes became, indexed by node.
    pub fn import(&mut self, other: &Aig, leaves: &mut dyn FnMut(Leaf) -> AigLit) -> Vec<AigLit> {
        let mut map: Vec<AigLit> = Vec::with_capacity(other.nodes.len());
        for node in &other.nodes {
            let lit = match *node {
                Node::False => AigLit::FALSE,
                Node::Input(i) => leaves(Leaf::Input(i)),
                Node::Latch(k) => leaves(Leaf::Latch(k)),
                Node::And(a, b) => {
                    let ma = map[a.node() as usize].with_phase(a.is_complemented());
                    let mb = map[b.node() as usize].with_phase(b.is_complemented());
                    self.and(ma, mb)
                }
            };
            map.push(lit);
        }
        map
    }

    /// The AIGER variable each node is written as: inputs, then latches, then gates in node order.
    fn numbering(&self) -> Vec<u32> {
        let mut var = vec![0u32; self.nodes.len()];
        for (i, p) in self.inputs.iter().enumerate() {
            var[p.lit.node() as usize] = i as u32 + 1;
        }
        let base = self.inputs.len() as u32;
        for (k, l) in self.latches.iter().enumerate() {
            var[l.node as usize] = base + k as u32 + 1;
        }
        let mut next = base + self.latches.len() as u32 + 1;
        for (n, node) in self.nodes.iter().enumerate() {
            if let Node::And(..) = node {
                var[n] = next;
                next += 1;
            }
        }
        var
    }

    fn header(&self, magic: &str) -> String {
        let (i, l, a) = (self.inputs.len(), self.latches.len(), self.num_ands());
        let mut h = format!("{magic} {} {i} {l} {} {a}", i + l + a, self.outputs.len());
        if !self.bad.is_empty() || !self.constraints.is_empty() {
            h.push_str(&format!(" {} {}", self.bad.len(), self.constraints.len()));
        }
        h.push('\n');
        h
    }

    fn symbols_and_comments(&self, out: &mut String) {
        let tables: [(char, Vec<&Option<String>>); 5] = [
            ('i', self.inputs.iter().map(|p| &p.name).collect()),
            ('l', self.latches.iter().map(|l| &l.name).collect()),
            ('o', self.outputs.iter().map(|p| &p.name).collect()),
            ('b', self.bad.iter().map(|p| &p.name).collect()),
            ('c', self.constraints.iter().map(|p| &p.name).collect()),
        ];
        for (kind, names) in tables {
            for (k, name) in names.into_iter().enumerate() {
                if let Some(name) = name {
                    out.push_str(&format!("{kind}{k} {name}\n"));
                }
            }
        }
        if !self.comments.is_empty() {
            out.push_str("c\n");
            for c in &self.comments {
                out.push_str(c);
                out.push('\n');
            }
        }
    }

    fn latch_line(&self, var: &[u32], l: &Latch) -> String {
        let code = |lit: AigLit| var[lit.node() as usize] * 2 + lit.is_complemented() as u32;
        match l.init {
            Init::Zero => format!("{}", code(l.next)),
            Init::One => format!("{} 1", code(l.next)),
            Init::Free => format!("{} {}", code(l.next), var[l.node as usize] * 2),
        }
    }

    /// The canonical ASCII AIGER text.
    pub fn to_ascii(&self) -> String {
        let var = self.numbering();
        let code = |lit: AigLit| var[lit.node() as usize] * 2 + lit.is_complemented() as u32;
        let mut out = self.header("aag");
        for p in &self.inputs {
            out.push_str(&format!("{}\n", code(p.lit)));
        }
        for l in &self.latches {
            out.push_str(&format!("{} {}\n", var[l.node as usize] * 2, self.latch_line(&var, l)));
        }
        for p in self.outputs.iter().chain(&self.bad).chain(&self.constraints) {
            out.push_str(&format!("{}\n", code(p.lit)));
        }
        for (n, node) in self.nodes.iter().enumerate() {
            if let Node::And(a, b) = *node {
                let (x, y) = (code(a), code(b));
                out.push_str(&format!("{} {} {}\n", var[n] * 2, x.max(y), x.min(y)));
            }
        }
        self.symbols_and_comments(&mut out);
        out
    }

    /// The canonical binary AIGER bytes.
    pub fn to_binary(&self) -> Vec<u8> {
        let var = self.numbering();
        let code = |lit: AigLit| var[lit.node() as usize] * 2 + lit.is_complemented() as u32;
        let mut text = self.header("aig");
        for l in &self.latches {
            text.push_str(&self.latch_line(&var, l));
            text.push('\n');
        }
        for p in self.outputs.iter().chain(&self.bad).chain(&self.constraints) {
            text.push_str(&format!("{}\n", code(p.lit)));
        }
        let mut out = text.into_bytes();
        for (n, node) in self.nodes.iter().enumerate() {
            if let Node::And(a, b) = *node {
                let (x, y) = (code(a), code(b));
                let (r0, r1) = (x.max(y), x.min(y));
                let lhs = var[n] * 2;
                push_varint(&mut out, lhs - r0);
                push_varint(&mut out, r0 - r1);
            }
        }
        let mut tail = String::new();
        self.symbols_and_comments(&mut tail);
        out.extend_from_slice(tail.as_bytes());
        out
    }

    /// Read an ASCII (`aag`) or binary (`aig`) AIGER file.
    pub fn parse(bytes: &[u8]) -> Result<Aig, AigerError> {
        Parser { bytes, pos: 0, line: 0 }.parse()
    }
}

fn push_varint(out: &mut Vec<u8>, mut x: u32) {
    while x >= 0x80 {
        out.push((x & 0x7f) as u8 | 0x80);
        x >>= 7;
    }
    out.push(x as u8);
}

/// What an AIGER variable is, as the file defines it.
#[derive(Clone, Copy)]
enum Def {
    None,
    Input,
    Latch,
    And { r0: u64, r1: u64, line: usize },
}

struct Parser<'a> {
    bytes: &'a [u8],
    pos: usize,
    line: usize,
}

impl Parser<'_> {
    /// The next line's text without its newline, or `None` at the end.
    fn next_line(&mut self) -> Option<String> {
        if self.pos >= self.bytes.len() {
            return None;
        }
        let end = self.bytes[self.pos..].iter().position(|&b| b == b'\n').map_or(self.bytes.len(), |k| self.pos + k);
        let text = String::from_utf8_lossy(&self.bytes[self.pos..end]).into_owned();
        self.pos = end + 1;
        self.line += 1;
        Some(text)
    }

    fn numbers(&mut self, want: std::ops::RangeInclusive<usize>) -> Result<(Vec<u64>, usize), AigerError> {
        let text = self.next_line().ok_or(AigerError::Truncated)?;
        let nums: Result<Vec<u64>, _> = text.split_whitespace().map(str::parse::<u64>).collect();
        match nums {
            Ok(n) if want.contains(&n.len()) => Ok((n, self.line)),
            _ => Err(AigerError::Malformed { line: self.line }),
        }
    }

    fn varint(&mut self) -> Result<u64, AigerError> {
        let mut x: u64 = 0;
        let mut shift = 0;
        loop {
            let &b = self.bytes.get(self.pos).ok_or(AigerError::Truncated)?;
            self.pos += 1;
            if shift > 56 {
                return Err(AigerError::Malformed { line: self.line });
            }
            x |= ((b & 0x7f) as u64) << shift;
            if b & 0x80 == 0 {
                return Ok(x);
            }
            shift += 7;
        }
    }

    fn parse(mut self) -> Result<Aig, AigerError> {
        let header = self.next_line().ok_or(AigerError::Truncated)?;
        let toks: Vec<&str> = header.split_whitespace().collect();
        let binary = match toks.first() {
            Some(&"aag") => false,
            Some(&"aig") => true,
            _ => return Err(AigerError::Header(format!("expected `aag` or `aig`, read `{header}`"))),
        };
        let nums: Vec<u64> = toks[1..]
            .iter()
            .map(|t| t.parse::<u64>())
            .collect::<Result<_, _>>()
            .map_err(|_| AigerError::Header(format!("non-numeric field in `{header}`")))?;
        if !(5..=9).contains(&nums.len()) {
            return Err(AigerError::Header(format!("expected M I L O A [B C J F], read `{header}`")));
        }
        let field = |k: usize| nums.get(k).copied().unwrap_or(0);
        let (m, ni, nl, no, na) = (field(0), field(1), field(2), field(3), field(4));
        let (nb, nc, nj, nf) = (field(5), field(6), field(7), field(8));
        if nj > 0 || nf > 0 {
            return Err(AigerError::Unsupported("justice and fairness properties".into()));
        }
        if binary && m != ni + nl + na {
            return Err(AigerError::Header(format!("a binary file has M = I + L + A, read `{header}`")));
        }
        let max_lit = 2 * m + 1;
        let mut def = vec![Def::None; m as usize + 1];
        let define = |var: u64, d: Def, line: usize, def: &mut Vec<Def>| -> Result<(), AigerError> {
            if var == 0 || var > m {
                return Err(AigerError::Literal { line, literal: var * 2 });
            }
            if !matches!(def[var as usize], Def::None) {
                return Err(AigerError::Redefined { line, var });
            }
            def[var as usize] = d;
            Ok(())
        };
        let mut input_vars = Vec::with_capacity(ni as usize);
        for k in 0..ni {
            let var = if binary {
                k + 1
            } else {
                let (n, line) = self.numbers(1..=1)?;
                if n[0] % 2 == 1 || n[0] == 0 || n[0] > 2 * m {
                    return Err(AigerError::Literal { line, literal: n[0] });
                }
                define(n[0] / 2, Def::Input, line, &mut def)?;
                n[0] / 2
            };
            if binary {
                define(var, Def::Input, 0, &mut def)?;
            }
            input_vars.push(var);
        }
        // (latch variable, next code, init, line)
        let mut latch_defs: Vec<(u64, u64, Init, usize)> = Vec::with_capacity(nl as usize);
        for k in 0..nl {
            let (n, line) = if binary { self.numbers(1..=2)? } else { self.numbers(2..=3)? };
            let (lit, rest) = if binary { (2 * (ni + k + 1), &n[..]) } else { (n[0], &n[1..]) };
            if lit % 2 == 1 || lit == 0 || lit > 2 * m {
                return Err(AigerError::Literal { line, literal: lit });
            }
            let next = rest[0];
            if next > max_lit {
                return Err(AigerError::Literal { line, literal: next });
            }
            let init = match rest.get(1).copied() {
                None | Some(0) => Init::Zero,
                Some(1) => Init::One,
                Some(x) if x == lit => Init::Free,
                Some(x) => return Err(AigerError::Literal { line, literal: x }),
            };
            define(lit / 2, Def::Latch, line, &mut def)?;
            latch_defs.push((lit / 2, next, init, line));
        }
        let read_ports = |count: u64, this: &mut Self| -> Result<Vec<(u64, usize)>, AigerError> {
            let mut out = Vec::with_capacity(count as usize);
            for _ in 0..count {
                let (n, line) = this.numbers(1..=1)?;
                if n[0] > max_lit {
                    return Err(AigerError::Literal { line, literal: n[0] });
                }
                out.push((n[0], line));
            }
            Ok(out)
        };
        let outputs = read_ports(no, &mut self)?;
        let bad = read_ports(nb, &mut self)?;
        let constraints = read_ports(nc, &mut self)?;
        let mut and_vars = Vec::with_capacity(na as usize);
        for k in 0..na {
            if binary {
                let lhs = 2 * (ni + nl + k + 1);
                let d0 = self.varint()?;
                let d1 = self.varint()?;
                if d0 == 0 || d0 > lhs || d1 > lhs - d0 {
                    return Err(AigerError::Malformed { line: self.line });
                }
                let r0 = lhs - d0;
                let r1 = r0 - d1;
                define(lhs / 2, Def::And { r0, r1, line: self.line }, self.line, &mut def)?;
                and_vars.push(lhs / 2);
            } else {
                let (n, line) = self.numbers(3..=3)?;
                let (lhs, r0, r1) = (n[0], n[1], n[2]);
                if lhs % 2 == 1 || lhs == 0 || lhs > 2 * m {
                    return Err(AigerError::Literal { line, literal: lhs });
                }
                for r in [r0, r1] {
                    if r > max_lit {
                        return Err(AigerError::Literal { line, literal: r });
                    }
                }
                define(lhs / 2, Def::And { r0, r1, line }, line, &mut def)?;
                and_vars.push(lhs / 2);
            }
        }
        let mut aig = Aig::new();
        let mut map: Vec<Option<AigLit>> = vec![None; m as usize + 1];
        map[0] = Some(AigLit::FALSE);
        for &v in &input_vars {
            map[v as usize] = Some(aig.add_input(None));
        }
        for &(v, _, init, _) in &latch_defs {
            let k = aig.add_latch(None, init);
            map[v as usize] = Some(aig.latch_lit(k));
        }
        let resolve = |map: &[Option<AigLit>], code: u64, line: usize| -> Result<AigLit, AigerError> {
            map[(code / 2) as usize]
                .map(|l| l.with_phase(code % 2 == 1))
                .ok_or(AigerError::Undefined { line, var: code / 2 })
        };
        let mut on_path = vec![false; m as usize + 1];
        for &root in &and_vars {
            let mut stack: Vec<(u64, bool)> = vec![(root, false)];
            while let Some((v, expanded)) = stack.pop() {
                if map[v as usize].is_some() {
                    continue;
                }
                let Def::And { r0, r1, line } = def[v as usize] else {
                    return Err(AigerError::Undefined { line: 0, var: v });
                };
                if expanded {
                    let a = resolve(&map, r0, line)?;
                    let b = resolve(&map, r1, line)?;
                    map[v as usize] = Some(aig.and(a, b));
                    on_path[v as usize] = false;
                    continue;
                }
                if on_path[v as usize] {
                    return Err(AigerError::Cycle { var: v });
                }
                on_path[v as usize] = true;
                stack.push((v, true));
                for r in [r1, r0] {
                    let c = r / 2;
                    if map[c as usize].is_some() {
                        continue;
                    }
                    match def[c as usize] {
                        Def::And { .. } => {
                            if on_path[c as usize] {
                                return Err(AigerError::Cycle { var: c });
                            }
                            stack.push((c, false));
                        }
                        _ => return Err(AigerError::Undefined { line, var: c }),
                    }
                }
            }
        }
        for (k, &(_, next, _, line)) in latch_defs.iter().enumerate() {
            let lit = resolve(&map, next, line)?;
            aig.set_next(k, lit);
        }
        for (code, line) in outputs {
            let lit = resolve(&map, code, line)?;
            aig.add_output(lit, None);
        }
        for (code, line) in bad {
            let lit = resolve(&map, code, line)?;
            aig.add_bad(lit, None);
        }
        for (code, line) in constraints {
            let lit = resolve(&map, code, line)?;
            aig.add_constraint(lit, None);
        }
        while let Some(text) = self.next_line() {
            if text == "c" {
                while let Some(c) = self.next_line() {
                    aig.comments.push(c);
                }
                break;
            }
            if text.is_empty() {
                continue;
            }
            let malformed = AigerError::Malformed { line: self.line };
            let mut chars = text.chars();
            let kind = chars.next().ok_or(malformed.clone())?;
            let rest = chars.as_str();
            let (pos, name) = rest.split_once(' ').ok_or(malformed.clone())?;
            let pos: usize = pos.parse().map_err(|_| malformed.clone())?;
            let slot = match kind {
                'i' => aig.inputs.get_mut(pos).map(|p| &mut p.name),
                'l' => aig.latches.get_mut(pos).map(|l| &mut l.name),
                'o' => aig.outputs.get_mut(pos).map(|p| &mut p.name),
                'b' => aig.bad.get_mut(pos).map(|p| &mut p.name),
                'c' => aig.constraints.get_mut(pos).map(|p| &mut p.name),
                _ => None,
            };
            *slot.ok_or(malformed)? = Some(name.to_string());
        }
        Ok(aig)
    }
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_proof/aig/tests.rs"]
mod tests;
