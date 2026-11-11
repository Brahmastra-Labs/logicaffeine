//! **An and-inverter graph edited in place**, the ground the rewriting passes stand on. Each node keeps what reads it,
//! the nodes and the ports, so a node is replaced by a literal equal to it at once ([`Network::replace`]): every reader
//! then reads the literal, a reader left reading a pair another node reads, or a pair that folds, is replaced in turn
//! by that node or that fold, and what nothing reads any more is freed down its cone. Structural hashing holds
//! throughout, so no two live nodes read the same pair and none reads a pair that folds. [`Network::to_aig`] reads the
//! network back as a graph with the ports of the one it was made from.

use std::collections::HashMap;

use crate::aig::{Aig, AigLit, Node};

/// What a node of a [`Network`] is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    False,
    /// An input or a latch: the node of the same number in the graph the network was made from.
    Leaf,
    /// The conjunction of two literals, the larger code first.
    And(AigLit, AigLit),
    /// A node nothing reads any more.
    Freed,
}

/// What reads a node: an And node, or a port by its place in the network's ports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Reader {
    Node(u32),
    Port(u32),
}

/// An and-inverter graph whose nodes know their readers. The ports are the graph's outputs, bad states, constraints
/// and next states, in that order.
#[derive(Clone, Debug)]
pub(crate) struct Network {
    kinds: Vec<Kind>,
    readers: Vec<Vec<Reader>>,
    strash: HashMap<(AigLit, AigLit), u32>,
    ports: Vec<AigLit>,
    ands: usize,
    /// How many times each node's fan-ins have changed.
    versions: Vec<u32>,
    /// The replacements under way that keep each node, a node a [`Network::replace`] is yet to move readers to: not
    /// freed while it has one, though nothing may read it yet.
    pins: Vec<u32>,
    /// The nodes [`Network::freed`] found last: those whose stamp is the epoch.
    freed_stamp: Vec<u32>,
    /// Readers left to each node during [`Network::freed`], where its stamp is the epoch.
    left: Vec<u32>,
    left_stamp: Vec<u32>,
    epoch: u32,
}

/// The pair in the order a node holds it, the larger code first.
fn ordered(a: AigLit, b: AigLit) -> (AigLit, AigLit) {
    if a >= b {
        (a, b)
    } else {
        (b, a)
    }
}

/// What `hi ∧ lo` (ordered) folds to, if it folds: false for a false operand or a literal and its complement, the
/// other operand for a true one, the literal for a literal twice.
fn fold(hi: AigLit, lo: AigLit) -> Option<AigLit> {
    if lo == AigLit::FALSE || hi == !lo {
        Some(AigLit::FALSE)
    } else if lo == AigLit::TRUE || hi == lo {
        Some(hi)
    } else {
        None
    }
}

impl Network {
    /// The network of `aig`, node for node, but for the And nodes no port reads, which are freed.
    pub(crate) fn new(aig: &Aig) -> Network {
        let n = aig.num_nodes();
        let mut net = Network {
            kinds: Vec::with_capacity(n),
            readers: vec![Vec::new(); n],
            strash: HashMap::with_capacity(aig.num_ands()),
            ports: Vec::new(),
            ands: 0,
            versions: vec![0; n],
            pins: vec![0; n],
            freed_stamp: vec![0; n],
            left: vec![0; n],
            left_stamp: vec![0; n],
            epoch: 0,
        };
        for m in 0..n as u32 {
            let kind = match aig.node(m) {
                Node::False => Kind::False,
                Node::Input(_) | Node::Latch(_) => Kind::Leaf,
                Node::And(a, b) => {
                    net.readers[a.node() as usize].push(Reader::Node(m));
                    net.readers[b.node() as usize].push(Reader::Node(m));
                    net.strash.insert((a, b), m);
                    net.ands += 1;
                    Kind::And(a, b)
                }
            };
            net.kinds.push(kind);
        }
        let ports = aig.outputs().iter().chain(aig.bad()).chain(aig.constraints()).map(|p| p.lit);
        for lit in ports.chain(aig.latches().iter().map(|l| l.next)).collect::<Vec<_>>() {
            net.readers[lit.node() as usize].push(Reader::Port(net.ports.len() as u32));
            net.ports.push(lit);
        }
        for m in (0..n as u32).rev() {
            net.free_unread(m);
        }
        net
    }

    /// The nodes ever made, freed ones among them.
    pub(crate) fn len(&self) -> usize {
        self.kinds.len()
    }

    pub(crate) fn kind(&self, n: u32) -> Kind {
        self.kinds[n as usize]
    }

    /// The live And nodes.
    pub(crate) fn num_ands(&self) -> usize {
        self.ands
    }

    /// How many times node `n`'s fan-ins have changed, by a node below it replaced.
    pub(crate) fn version(&self, n: u32) -> u32 {
        self.versions[n as usize]
    }

    /// `a ∧ b` if the network holds it without a new node: its fold, or the node reading the pair.
    pub(crate) fn lookup(&self, a: AigLit, b: AigLit) -> Option<AigLit> {
        let (hi, lo) = ordered(a, b);
        fold(hi, lo).or_else(|| self.strash.get(&(hi, lo)).map(|&n| AigLit::new(n, false)))
    }

    /// `a ∧ b`, folded and structurally hashed: a node made when the network holds none. A node made reads its fan-ins
    /// at once, and stays until it is read and then freed, or [`Network::free_unread`] frees it.
    pub(crate) fn and(&mut self, a: AigLit, b: AigLit) -> AigLit {
        if let Some(l) = self.lookup(a, b) {
            return l;
        }
        let (hi, lo) = ordered(a, b);
        let n = self.kinds.len() as u32;
        self.kinds.push(Kind::And(hi, lo));
        self.readers.push(Vec::new());
        self.versions.push(0);
        self.pins.push(0);
        self.freed_stamp.push(0);
        self.left.push(0);
        self.left_stamp.push(0);
        self.readers[hi.node() as usize].push(Reader::Node(n));
        self.readers[lo.node() as usize].push(Reader::Node(n));
        self.strash.insert((hi, lo), n);
        self.ands += 1;
        AigLit::new(n, false)
    }

    /// Replace node `old` by `new`, a literal equal to it whose cone does not hold it: every reader of `old` reads
    /// `new` instead, a reader that then reads a pair the network holds, or one that folds, is replaced in turn by what
    /// it holds, and `old` and whatever nothing reads any more are freed.
    pub(crate) fn replace(&mut self, old: u32, new: AigLit) {
        assert_ne!(old, new.node(), "a node is not replaced by itself");
        let mut work = vec![(old, new)];
        let mut targets = vec![new.node()];
        self.pins[new.node() as usize] += 1;
        while let Some((old, new)) = work.pop() {
            for reader in std::mem::take(&mut self.readers[old as usize]) {
                match reader {
                    Reader::Port(p) => {
                        self.ports[p as usize] = new.with_phase(self.ports[p as usize].is_complemented());
                        self.readers[new.node() as usize].push(reader);
                    }
                    Reader::Node(m) => {
                        let Kind::And(a, b) = self.kinds[m as usize] else { unreachable!("a reader is an And node") };
                        let swap = |l: AigLit| if l.node() == old { new.with_phase(l.is_complemented()) } else { l };
                        let (hi, lo) = ordered(swap(a), swap(b));
                        if let Some(held) = self.lookup(hi, lo) {
                            self.readers[old as usize].push(reader);
                            self.pins[held.node() as usize] += 1;
                            targets.push(held.node());
                            work.push((m, held));
                        } else {
                            self.strash.remove(&(a, b));
                            self.kinds[m as usize] = Kind::And(hi, lo);
                            self.strash.insert((hi, lo), m);
                            self.versions[m as usize] += 1;
                            self.readers[new.node() as usize].push(reader);
                        }
                    }
                }
            }
            self.free_unread(old);
        }
        for &n in &targets {
            self.pins[n as usize] -= 1;
        }
        for n in targets {
            self.free_unread(n);
        }
    }

    /// Free node `n` if it is an And node nothing reads and no replacement keeps, and down its fan-ins whatever that
    /// leaves unread.
    pub(crate) fn free_unread(&mut self, n: u32) {
        let mut stack = vec![n];
        while let Some(n) = stack.pop() {
            let Kind::And(a, b) = self.kinds[n as usize] else { continue };
            if !self.readers[n as usize].is_empty() || self.pins[n as usize] > 0 {
                continue;
            }
            self.strash.remove(&(a, b));
            self.kinds[n as usize] = Kind::Freed;
            self.ands -= 1;
            for f in [a.node(), b.node()] {
                let readers = &mut self.readers[f as usize];
                let at = readers.iter().position(|&r| r == Reader::Node(n)).expect("a fan-in is read by its node");
                readers.swap_remove(at);
                if readers.is_empty() {
                    stack.push(f);
                }
            }
        }
    }

    /// The And nodes freed by replacing `root` with `leaves` kept, `root` first: its fanout-free cone above them.
    /// Until the next call, [`Network::frees`] tells whether a node is among them.
    pub(crate) fn freed(&mut self, root: u32, leaves: &[u32]) -> Vec<u32> {
        self.epoch += 1;
        let epoch = self.epoch;
        let mut cone = Vec::new();
        let mut stack = vec![root];
        while let Some(n) = stack.pop() {
            let Kind::And(a, b) = self.kinds[n as usize] else { continue };
            self.freed_stamp[n as usize] = epoch;
            cone.push(n);
            for f in [a.node(), b.node()] {
                if leaves.contains(&f) || !matches!(self.kinds[f as usize], Kind::And(..)) {
                    continue;
                }
                let at = f as usize;
                if self.left_stamp[at] != epoch {
                    self.left_stamp[at] = epoch;
                    self.left[at] = self.readers[at].len() as u32;
                }
                self.left[at] -= 1;
                if self.left[at] == 0 {
                    stack.push(f);
                }
            }
        }
        cone
    }

    /// Whether the last [`Network::freed`] found node `n`.
    pub(crate) fn frees(&self, n: u32) -> bool {
        self.freed_stamp[n as usize] == self.epoch
    }

    /// The And nodes the ports read, each after its fan-ins.
    pub(crate) fn topological(&self) -> Vec<u32> {
        self.order().unwrap_or_else(|n| panic!("node {n} lies on a cycle"))
    }

    /// [`Network::topological`], or a node on a cycle.
    fn order(&self) -> Result<Vec<u32>, u32> {
        #[derive(Clone, Copy, PartialEq)]
        enum Seen {
            Not,
            Below,
            Done,
        }
        let mut seen = vec![Seen::Not; self.kinds.len()];
        let mut order = Vec::with_capacity(self.ands);
        let mut stack: Vec<(u32, bool)> = Vec::new();
        for port in &self.ports {
            stack.push((port.node(), false));
            while let Some((n, expanded)) = stack.pop() {
                let at = n as usize;
                let Kind::And(a, b) = self.kinds[at] else { continue };
                match (seen[at], expanded) {
                    (Seen::Done, _) => {}
                    (Seen::Below, false) => return Err(n),
                    (_, true) => {
                        seen[at] = Seen::Done;
                        order.push(n);
                    }
                    (Seen::Not, false) => {
                        seen[at] = Seen::Below;
                        stack.push((n, true));
                        stack.push((b.node(), false));
                        stack.push((a.node(), false));
                    }
                }
            }
        }
        Ok(order)
    }

    /// The network as a graph with `original`'s inputs, latches and port names, the network's made from it: the nodes
    /// the ports read, each after its fan-ins.
    pub(crate) fn to_aig(&self, original: &Aig) -> Aig {
        let mut g = Aig::new();
        let mut map: Vec<AigLit> = vec![AigLit::FALSE; self.kinds.len()];
        for p in original.inputs() {
            map[p.lit.node() as usize] = g.add_input(p.name.clone());
        }
        let latches: Vec<usize> = original.latches().iter().map(|l| g.add_latch(l.name.clone(), l.init)).collect();
        for (j, l) in original.latches().iter().enumerate() {
            map[l.node as usize] = g.latch_lit(latches[j]);
        }
        let read = |map: &[AigLit], l: AigLit| map[l.node() as usize].with_phase(l.is_complemented());
        for n in self.topological() {
            let Kind::And(a, b) = self.kinds[n as usize] else { unreachable!("an And node in order") };
            map[n as usize] = g.and(read(&map, a), read(&map, b));
        }
        let mut ports = self.ports.iter().map(|&l| read(&map, l));
        for p in original.outputs() {
            g.add_output(ports.next().expect("an output"), p.name.clone());
        }
        for p in original.bad() {
            g.add_bad(ports.next().expect("a bad state"), p.name.clone());
        }
        for p in original.constraints() {
            g.add_constraint(ports.next().expect("a constraint"), p.name.clone());
        }
        for j in 0..latches.len() {
            g.set_next(latches[j], ports.next().expect("a next state"));
        }
        g
    }

    /// Whether the network holds together, recounted from its nodes: each live And node read by something and reading
    /// live nodes a pair in order that does not fold, hashed under that pair and listed by both fan-ins as a reader,
    /// each port listed by its node, no reader listed but these, the And count right, and no cycle.
    pub(crate) fn check(&self) -> Result<(), String> {
        let mut expected: Vec<Vec<Reader>> = vec![Vec::new(); self.kinds.len()];
        let mut ands = 0;
        for (n, kind) in self.kinds.iter().enumerate() {
            let Kind::And(a, b) = *kind else { continue };
            ands += 1;
            if a < b || fold(a, b).is_some() {
                return Err(format!("node {n} reads {a:?} ∧ {b:?}, out of order or folding"));
            }
            for f in [a, b] {
                if matches!(self.kinds[f.node() as usize], Kind::Freed) {
                    return Err(format!("node {n} reads freed node {}", f.node()));
                }
                expected[f.node() as usize].push(Reader::Node(n as u32));
            }
            if self.strash.get(&(a, b)) != Some(&(n as u32)) {
                return Err(format!("node {n} is not hashed under its pair"));
            }
        }
        for (p, l) in self.ports.iter().enumerate() {
            if matches!(self.kinds[l.node() as usize], Kind::Freed) {
                return Err(format!("port {p} reads freed node {}", l.node()));
            }
            expected[l.node() as usize].push(Reader::Port(p as u32));
        }
        if ands != self.ands || self.strash.len() != ands {
            return Err(format!("{ands} live And nodes, counted {}, hashed {}", self.ands, self.strash.len()));
        }
        let key = |r: &Reader| match *r {
            Reader::Node(m) => (0, m),
            Reader::Port(p) => (1, p),
        };
        for (n, want) in expected.iter_mut().enumerate() {
            let mut have = self.readers[n].clone();
            have.sort_by_key(key);
            want.sort_by_key(key);
            if have != *want {
                return Err(format!("node {n} lists readers {have:?}, read by {want:?}"));
            }
            if matches!(self.kinds[n], Kind::And(..)) && want.is_empty() {
                return Err(format!("node {n} is live and read by nothing"));
            }
        }
        if self.pins.iter().any(|&p| p > 0) {
            return Err("a node pinned outside a replacement".to_string());
        }
        match self.order() {
            Err(n) => Err(format!("node {n} lies on a cycle")),
            Ok(order) if order.len() != ands => Err("an And node the ports do not reach".to_string()),
            Ok(_) => Ok(()),
        }
    }
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_proof/aig_edit/tests.rs"]
mod tests;
