//! Mapping logic onto K-input LUTs for depth, the way an FPGA's synthesis sees how deep a circuit is.
//!
//! An [`Xag`] holds the logic as two-input AND and XOR nodes over inputs, each edge plain or complemented, built with
//! structural hashing and constant folding. [`map`] rebuilds it with every wide associative gate balanced for LUTs,
//! then covers it with LUTs of at most `k` inputs:
//! - a supergate is a maximal tree of AND nodes joined by plain edges, or of XOR nodes, whose inner nodes feed
//!   nothing else and lie in one group (an instance of the design). It is rebuilt from its leaves `k` at a time,
//!   the earliest-arriving first, so an AND or a parity of `n` inputs is `⌈log_k n⌉` LUT levels deep, where the
//!   balanced binary tree a netlist hands over allows no better than `⌈log₂ n / ⌊log₂ k⌋⌉`;
//! - every node keeps its best [`CUTS`] cuts of at most `k` leaves, ranked by arrival and then size (priority cuts:
//!   Mishchenko, Cho, Chatterjee & Brayton, *Combinational and sequential mapping with priority cuts*, 2007), and
//!   its level is its best cut's arrival, one more than its latest leaf's;
//! - the cover takes each required node's best cut as a LUT, from the outputs back.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};

/// How many cuts each node keeps.
pub const CUTS: usize = 8;

/// The largest LUT [`map`] maps to.
pub const MAX_K: usize = 8;

/// A node and whether the edge to it is complemented: `node << 1 | complemented`.
pub type Lit = u32;

pub const FALSE: Lit = 0;
pub const TRUE: Lit = 1;

pub fn node(l: Lit) -> usize {
    (l >> 1) as usize
}

pub fn complemented(l: Lit) -> bool {
    l & 1 == 1
}

/// A node of an [`Xag`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Gate {
    /// Node 0, the constant false.
    Constant,
    Input,
    And(Lit, Lit),
    Xor(Lit, Lit),
}

/// An AND-XOR graph: node 0 is the constant, and every node's fanins come before it.
#[derive(Clone, Debug)]
pub struct Xag {
    gates: Vec<Gate>,
    hash: HashMap<Gate, u32>,
}

impl Default for Xag {
    fn default() -> Xag {
        Xag::new()
    }
}

impl Xag {
    pub fn new() -> Xag {
        Xag { gates: vec![Gate::Constant], hash: HashMap::new() }
    }

    pub fn len(&self) -> usize {
        self.gates.len()
    }

    pub fn is_empty(&self) -> bool {
        self.gates.len() == 1
    }

    pub fn gate(&self, n: usize) -> Gate {
        self.gates[n]
    }

    pub fn input(&mut self) -> Lit {
        self.gates.push(Gate::Input);
        ((self.gates.len() - 1) as u32) << 1
    }

    fn hashed(&mut self, g: Gate) -> Lit {
        if let Some(&n) = self.hash.get(&g) {
            return n << 1;
        }
        let n = self.gates.len() as u32;
        self.gates.push(g);
        self.hash.insert(g, n);
        n << 1
    }

    pub fn and(&mut self, a: Lit, b: Lit) -> Lit {
        let (a, b) = if a <= b { (a, b) } else { (b, a) };
        match (a, b) {
            (FALSE, _) => FALSE,
            (TRUE, _) => b,
            _ if a == b => a,
            _ if a ^ 1 == b => FALSE,
            _ => self.hashed(Gate::And(a, b)),
        }
    }

    pub fn xor(&mut self, a: Lit, b: Lit) -> Lit {
        let parity = (a ^ b) & 1;
        let (a, b) = (a & !1, b & !1);
        let (a, b) = if a <= b { (a, b) } else { (b, a) };
        if a == FALSE {
            return b ^ parity;
        }
        if a == b {
            return FALSE ^ parity;
        }
        self.hashed(Gate::Xor(a, b)) ^ parity
    }

    pub fn or(&mut self, a: Lit, b: Lit) -> Lit {
        self.and(a ^ 1, b ^ 1) ^ 1
    }

    /// `s ? t : e`.
    pub fn mux(&mut self, s: Lit, t: Lit, e: Lit) -> Lit {
        let on = self.and(s, t);
        let off = self.and(s ^ 1, e);
        self.or(on, off)
    }

    /// Every node's value under `inputs`, one bit lane per input pattern: `inputs[i]` is the `i`-th input's word.
    pub fn simulate(&self, inputs: &[u64]) -> Vec<u64> {
        let mut v = vec![0u64; self.gates.len()];
        let lit = |v: &[u64], l: Lit| if complemented(l) { !v[node(l)] } else { v[node(l)] };
        let mut next = 0;
        for n in 0..self.gates.len() {
            v[n] = match self.gates[n] {
                Gate::Constant => 0,
                Gate::Input => {
                    next += 1;
                    inputs.get(next - 1).copied().unwrap_or(0)
                }
                Gate::And(a, b) => lit(&v, a) & lit(&v, b),
                Gate::Xor(a, b) => lit(&v, a) ^ lit(&v, b),
            };
        }
        v
    }
}

/// A cut: at most [`MAX_K`] leaves, sorted, and the level the node is at when it is a LUT over them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Cut {
    len: u8,
    leaves: [u32; MAX_K],
    arrival: u32,
}

impl Cut {
    fn leaves(&self) -> &[u32] {
        &self.leaves[..self.len as usize]
    }

    fn unit(n: u32) -> Cut {
        let mut leaves = [0; MAX_K];
        leaves[0] = n;
        Cut { len: 1, leaves, arrival: 0 }
    }

    /// The union of two cuts' leaves, when it has at most `k`.
    fn merge(a: &Cut, b: &Cut, k: usize) -> Option<Cut> {
        let (x, y) = (a.leaves(), b.leaves());
        let mut out = [0u32; MAX_K];
        let (mut i, mut j, mut n) = (0, 0, 0);
        while i < x.len() || j < y.len() {
            let take = match (x.get(i), y.get(j)) {
                (Some(&p), Some(&q)) if p == q => {
                    i += 1;
                    j += 1;
                    p
                }
                (Some(&p), Some(&q)) if p < q => {
                    i += 1;
                    p
                }
                (Some(&p), None) => {
                    i += 1;
                    p
                }
                (_, Some(&q)) => {
                    j += 1;
                    q
                }
                (None, None) => unreachable!(),
            };
            if n == k {
                return None;
            }
            out[n] = take;
            n += 1;
        }
        Some(Cut { len: n as u8, leaves: out, arrival: 0 })
    }
}

/// A LUT of the cover: its root node and its leaves, nodes of the balanced network.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lut {
    pub root: u32,
    pub leaves: Vec<u32>,
}

/// A network mapped onto LUTs.
#[derive(Clone, Debug)]
pub struct Mapping {
    /// The network rebuilt with its supergates balanced.
    pub xag: Xag,
    /// The literal of the balanced network each original node is; `None` for a supergate's inner node, which
    /// nothing outside its supergate reads.
    pub image: Vec<Option<Lit>>,
    /// The original node each balanced node stands for: the node it computes when `exact`, else the root of the
    /// supergate whose balancing made it.
    pub origin: Vec<u32>,
    pub exact: Vec<bool>,
    /// Each balanced node's LUT level: 0 for an input.
    pub level: Vec<u32>,
    /// The cover: a LUT for each node an output needs, in reverse topological order.
    pub luts: Vec<Lut>,
}

struct Builder<'a> {
    k: usize,
    old: &'a Xag,
    xag: Xag,
    cuts: Vec<Vec<Cut>>,
    level: Vec<u32>,
    origin: Vec<u32>,
    exact: Vec<bool>,
}

impl Builder<'_> {
    /// Record a node the balanced network just made or found, its cuts and level, standing for `origin`.
    fn note(&mut self, l: Lit, origin: u32, exact: bool) -> Lit {
        let n = node(l);
        if n < self.cuts.len() {
            return l;
        }
        self.origin.push(origin);
        self.exact.push(exact);
        let (cuts, level) = match self.xag.gate(n) {
            Gate::Constant | Gate::Input => (Vec::new(), 0),
            Gate::And(a, b) | Gate::Xor(a, b) => {
                let (a, b) = (node(a), node(b));
                let mut found: Vec<Cut> = Vec::with_capacity(CUTS * 4);
                let side = |m: usize, cuts: &[Vec<Cut>], level: &[u32]| {
                    let mut unit = Cut::unit(m as u32);
                    unit.arrival = level[m];
                    std::iter::once(unit).chain(cuts[m].iter().copied()).collect::<Vec<_>>()
                };
                let (left, right) = (side(a, &self.cuts, &self.level), side(b, &self.cuts, &self.level));
                for x in &left {
                    for y in &right {
                        if let Some(mut c) = Cut::merge(x, y, self.k) {
                            c.arrival = c.leaves().iter().map(|&m| self.level[m as usize]).max().unwrap_or(0) + 1;
                            if !found.iter().any(|f| f.leaves() == c.leaves()) {
                                found.push(c);
                            }
                        }
                    }
                }
                found.sort_by_key(|c| (c.arrival, c.len));
                found.truncate(CUTS);
                let level = found.first().map_or(0, |c| c.arrival);
                (found, level)
            }
        };
        self.cuts.push(cuts);
        self.level.push(level);
        l
    }

    fn input(&mut self, origin: u32) -> Lit {
        let l = self.xag.input();
        self.note(l, origin, true)
    }

    fn gate(&mut self, xor: bool, a: Lit, b: Lit, origin: u32) -> Lit {
        let l = if xor { self.xag.xor(a, b) } else { self.xag.and(a, b) };
        self.note(l, origin, false)
    }

    /// `leaves` combined by AND or XOR, `k` at a time from the earliest-arriving, each group a balanced binary tree.
    fn supergate(&mut self, xor: bool, leaves: Vec<Lit>, origin: u32) -> Lit {
        let mut heap: BinaryHeap<Reverse<(u32, Lit)>> = leaves.into_iter().map(|l| Reverse((self.level[node(l)], l))).collect();
        loop {
            let take = heap.len().min(self.k);
            let mut group: Vec<Lit> = (0..take).map(|_| heap.pop().expect("take ≤ len").0 .1).collect();
            while group.len() > 1 {
                group = group.chunks(2).map(|p| if p.len() == 2 { self.gate(xor, p[0], p[1], origin) } else { p[0] }).collect();
            }
            let root = group.first().copied().unwrap_or(if xor { FALSE } else { TRUE });
            if heap.is_empty() {
                return root;
            }
            heap.push(Reverse((self.level[node(root)], root)));
        }
    }
}

/// Map `xag` onto LUTs of at most `k` inputs (`2 ≤ k ≤ MAX_K`), its supergates confined to `group`s (one entry a
/// node), covering the nodes `outputs` read.
pub fn map(xag: &Xag, group: &[u32], outputs: &[Lit], k: usize) -> Mapping {
    assert!((2..=MAX_K).contains(&k), "a LUT has 2 to {MAX_K} inputs");
    assert_eq!(group.len(), xag.len(), "a group for every node");
    let n = xag.len();
    let mut fanout = vec![0u32; n];
    let mut reader = vec![usize::MAX; n];
    for (m, g) in xag.gates.iter().enumerate() {
        if let Gate::And(a, b) | Gate::Xor(a, b) = *g {
            for l in [a, b] {
                fanout[node(l)] += 1;
                reader[node(l)] = m;
            }
        }
    }
    for &l in outputs {
        fanout[node(l)] += 1;
        reader[node(l)] = usize::MAX;
    }
    // A supergate's inner node: one reader, of its own kind and group, by a plain edge for an AND.
    let inner: Vec<bool> = (0..n)
        .map(|m| {
            let r = reader[m];
            if fanout[m] != 1 || r == usize::MAX || group[r] != group[m] {
                return false;
            }
            match (xag.gates[m], xag.gates[r]) {
                (Gate::And(..), Gate::And(a, b)) => [a, b].iter().any(|&l| node(l) == m && !complemented(l)),
                (Gate::Xor(..), Gate::Xor(..)) => true,
                _ => false,
            }
        })
        .collect();
    let mut b = Builder { k, old: xag, xag: Xag::new(), cuts: vec![Vec::new()], level: vec![0], origin: vec![0], exact: vec![true] };
    let mut image: Vec<Option<Lit>> = vec![None; n];
    image[0] = Some(FALSE);
    for m in 1..n {
        if inner[m] {
            continue;
        }
        let gate = b.old.gates[m];
        let img = match gate {
            Gate::Constant => FALSE,
            Gate::Input => b.input(m as u32),
            Gate::And(x, y) | Gate::Xor(x, y) => {
                let xor = matches!(gate, Gate::Xor(..));
                let mut parity = 0;
                let mut leaves = Vec::new();
                let mut stack = vec![x, y];
                while let Some(l) = stack.pop() {
                    let f = node(l);
                    if inner[f] {
                        if xor {
                            parity ^= l & 1;
                        }
                        if let Gate::And(p, q) | Gate::Xor(p, q) = b.old.gates[f] {
                            stack.push(p);
                            stack.push(q);
                        }
                        continue;
                    }
                    let mapped = image[f].expect("a leaf comes before its supergate") ^ (l & 1);
                    if xor {
                        parity ^= mapped & 1;
                        leaves.push(mapped & !1);
                    } else {
                        leaves.push(mapped);
                    }
                }
                leaves.sort_unstable();
                if xor {
                    let mut kept: Vec<Lit> = Vec::with_capacity(leaves.len());
                    for l in leaves {
                        if kept.last() == Some(&l) {
                            kept.pop();
                        } else {
                            kept.push(l);
                        }
                    }
                    leaves = kept;
                } else {
                    leaves.dedup();
                }
                let out = if !xor && leaves.windows(2).any(|w| w[0] ^ 1 == w[1]) || !xor && leaves.contains(&FALSE) {
                    FALSE
                } else {
                    leaves.retain(|&l| l != if xor { FALSE } else { TRUE });
                    b.supergate(xor, leaves, m as u32) ^ parity
                };
                let n_out = node(out);
                if n_out < b.exact.len() && b.origin[n_out] == m as u32 {
                    b.exact[n_out] = true;
                }
                out
            }
        };
        image[m] = Some(img);
    }
    let size = b.xag.len();
    let mut required = vec![false; size];
    for &l in outputs {
        if let Some(img) = image[node(l)] {
            required[node(img)] = true;
        }
    }
    let mut luts = Vec::new();
    for m in (1..size).rev() {
        if !required[m] {
            continue;
        }
        if let Some(best) = b.cuts[m].first() {
            for &leaf in best.leaves() {
                required[leaf as usize] = true;
            }
            luts.push(Lut { root: m as u32, leaves: best.leaves().to_vec() });
        }
    }
    Mapping { xag: b.xag, image, origin: b.origin, exact: b.exact, level: b.level, luts }
}

/// The truth table of `root` over `leaves` (at most six): bit `i` of the table is the root's value when leaf `j`
/// holds bit `j` of `i`. Every path from the root down must end at a leaf, as a cut's do.
pub fn function(xag: &Xag, root: u32, leaves: &[u32]) -> u64 {
    assert!(leaves.len() <= 6, "a table of at most six inputs");
    const VARS: [u64; 6] = [
        0xAAAA_AAAA_AAAA_AAAA,
        0xCCCC_CCCC_CCCC_CCCC,
        0xF0F0_F0F0_F0F0_F0F0,
        0xFF00_FF00_FF00_FF00,
        0xFFFF_0000_FFFF_0000,
        0xFFFF_FFFF_0000_0000,
    ];
    let mut value: HashMap<u32, u64> = leaves.iter().enumerate().map(|(i, &l)| (l, VARS[i])).collect();
    value.entry(0).or_insert(0);
    let mut stack = vec![root];
    while let Some(&n) = stack.last() {
        if value.contains_key(&n) {
            stack.pop();
            continue;
        }
        let (a, b) = match xag.gate(n as usize) {
            Gate::And(a, b) | Gate::Xor(a, b) => (a, b),
            Gate::Input | Gate::Constant => panic!("node {n} is below the cut"),
        };
        let missing: Vec<u32> = [a, b].iter().map(|&l| node(l) as u32).filter(|m| !value.contains_key(m)).collect();
        if !missing.is_empty() {
            stack.extend(missing);
            continue;
        }
        let lit = |l: Lit| if complemented(l) { !value[&(node(l) as u32)] } else { value[&(node(l) as u32)] };
        let v = match xag.gate(n as usize) {
            Gate::And(..) => lit(a) & lit(b),
            _ => lit(a) ^ lit(b),
        };
        value.insert(n, v);
        stack.pop();
    }
    let width = 1u32 << leaves.len();
    let mask = if width == 64 { u64::MAX } else { (1u64 << width) - 1 };
    value[&root] & mask
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/codegen_sva/lutmap/tests.rs"]
mod tests;
