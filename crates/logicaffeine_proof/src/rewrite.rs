//! **Cut rewriting** of and-inverter graphs by exact synthesis, aware of the logic the graph already holds, each
//! result proven equal to its input.
//!
//! The graph is edited in place ([`crate::aig_edit`]) and its nodes visited in topological order. A node's cuts of at
//! most four leaves are enumerated, the smallest few kept, and each cut's function (a truth table over its leaves)
//! taken to its NPN class (the least truth table over every permutation and complement of its inputs and complement of
//! its output). Every class's circuits were found by SAT-based exact synthesis ahead of time and ship with the crate:
//! each circuit of the fewest And gates the search found, up to a cap, and a few of one gate more, each size refuted,
//! satisfied, or given up past a conflict budget and passed over. A circuit in a node's place gains the nodes the
//! replacement frees (the node's maximum fanout-free cone above the cut) less what it costs: each gate the graph does
//! not hold, each it holds among those freed, and the freed nodes below those that it keeps by reading them; a gate
//! the graph holds elsewhere is free. A node is replaced at once by its circuit of most gain, if one gains, so the
//! nodes after it meet the graph as it now is. The result is proven equal to the input before it is returned: the two
//! side by side over shared inputs and latch values, every output, bad state, constraint and next state paired with
//! its counterpart and decided by the certified sweeper ([`crate::fraig::decide`]), its derivation replayed; a result
//! that does not check is not returned.

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use crate::aig::{Aig, AigLit, Leaf};
use crate::aig_edit::{Kind, Network};
use crate::cdcl::Lit;
use crate::fraig;
use crate::lrat::{self, Lrat};
use crate::sweep::Verdict;

/// A function of at most four inputs: bit `x` its value where input `i` takes bit `i` of `x`.
type Truth = u16;

/// The four inputs' truth tables.
const INPUT: [Truth; 4] = [0xAAAA, 0xCCCC, 0xF0F0, 0xFF00];

/// How [`rewrite`] searches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Options {
    /// Cuts kept per node.
    pub cuts: usize,
    /// The most passes, each over the graph the last one left; a pass that saves nothing is the last.
    pub rounds: usize,
}

impl Default for Options {
    fn default() -> Options {
        Options { cuts: 8, rounds: 4 }
    }
}

/// A rewritten graph and its proof.
#[derive(Clone, Debug)]
pub struct Report {
    pub aig: Aig,
    /// And nodes before and after, nodes rewritten, the passes run, the NPN classes the cuts met, and those whose
    /// least circuit found is proven their least (no smaller size given up on).
    pub before: usize,
    pub after: usize,
    pub rewritten: usize,
    pub rounds: usize,
    pub classes: usize,
    pub minimal: usize,
    /// The derivation, from [`side_by_side`]'s formula, that every port of the result equals the input's.
    pub proof: Lrat,
}

impl Report {
    /// Whether the proof shows the result equal to `original`, every port to its counterpart.
    pub fn check(&self, original: &Aig) -> bool {
        let Some((g, pairs)) = side_by_side(original, &self.aig) else { return false };
        let differing = pairs.iter().filter(|(l, r)| l != r);
        let claims: Vec<Vec<Lit>> = differing.flat_map(|&(l, r)| fraig::claim(l, r)).collect();
        lrat::check_derivation(g.num_nodes(), &fraig::formula(&g), &self.proof, &claims)
    }
}

/// Two circuits with the same ports side by side: one graph over the inputs and then one input per latch (its value),
/// shared, each circuit imported over them, and each output, bad state, constraint and next state of one paired with
/// the same of the other. `None` when the two have different ports.
pub fn side_by_side(left: &Aig, right: &Aig) -> Option<(Aig, Vec<(AigLit, AigLit)>)> {
    let same = left.inputs().len() == right.inputs().len()
        && left.latches().len() == right.latches().len()
        && left.outputs().len() == right.outputs().len()
        && left.bad().len() == right.bad().len()
        && left.constraints().len() == right.constraints().len();
    if !same {
        return None;
    }
    let mut g = Aig::new();
    let inputs: Vec<AigLit> = left.inputs().iter().map(|p| g.add_input(p.name.clone())).collect();
    let latches: Vec<AigLit> = (0..left.latches().len()).map(|j| g.add_input(Some(format!("latch {j}")))).collect();
    let mut leaf = |l: Leaf| match l {
        Leaf::Input(i) => inputs[i],
        Leaf::Latch(j) => latches[j],
    };
    let (lmap, rmap) = (g.import(left, &mut leaf), g.import(right, &mut leaf));
    let at = |map: &[AigLit], l: AigLit| map[l.node() as usize].with_phase(l.is_complemented());
    let ports = |a: &Aig| -> Vec<AigLit> {
        let mut p: Vec<AigLit> = a.outputs().iter().chain(a.bad()).chain(a.constraints()).map(|p| p.lit).collect();
        p.extend(a.latches().iter().map(|l| l.next));
        p
    };
    let pairs = ports(left).into_iter().zip(ports(right)).map(|(l, r)| (at(&lmap, l), at(&rmap, r))).collect();
    Some((g, pairs))
}

/// An input permutation, input complements and an output complement: `apply(f, t)(y) = out ⊕ f(z)` where
/// `z_i = y_{perm[i]} ⊕ neg_i`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Transform {
    perm: [u8; 4],
    neg: u8,
    out: bool,
}

fn apply(f: Truth, t: Transform) -> Truth {
    let mut g = 0;
    for y in 0..16u16 {
        let mut z = 0u16;
        for i in 0..4 {
            z |= ((y >> t.perm[i] & 1) ^ (t.neg >> i & 1) as u16) << i;
        }
        g |= ((f >> z & 1) ^ t.out as u16) << y;
    }
    g
}

/// Every NPN transform of four inputs.
fn transforms() -> Vec<Transform> {
    let mut perms = Vec::new();
    for a in 0..4u8 {
        for b in 0..4u8 {
            for c in 0..4u8 {
                for d in 0..4u8 {
                    let p = [a, b, c, d];
                    if (0..4).all(|i| p.iter().filter(|&&x| x == i).count() == 1) {
                        perms.push(p);
                    }
                }
            }
        }
    }
    let mut all = Vec::new();
    for perm in perms {
        for neg in 0..16u8 {
            for out in [false, true] {
                all.push(Transform { perm, neg, out });
            }
        }
    }
    all
}

/// An And-inverter circuit over four inputs: nodes `0..4` the inputs, then a gate each, two fan-ins and their
/// complements; the output a node, complemented or not, or a constant.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Circuit {
    gates: Vec<[(u8, bool); 2]>,
    out: Option<u8>,
    negated: bool,
}

impl Circuit {
    fn truth(&self) -> Truth {
        let mut v: Vec<Truth> = INPUT.to_vec();
        for &[(a, ca), (b, cb)] in &self.gates {
            let read = |n: u8, c: bool| if c { !v[n as usize] } else { v[n as usize] };
            let g = read(a, ca) & read(b, cb);
            v.push(g);
        }
        let out = self.out.map_or(0, |o| v[o as usize]);
        if self.negated {
            !out
        } else {
            out
        }
    }
}

impl Circuit {
    /// A circuit of the library's text: its gates in order, each two fan-ins, each a node (`a` to `d` the inputs, `e`
    /// on the gates in order) and `'` for a complement, then `=` and the output, a node, complemented or not, or `0`
    /// or `1`. `None` for a text that is not one, or a gate that reads a node not before it.
    fn parse(text: &str) -> Option<Circuit> {
        let (gates, out) = text.split_once('=')?;
        let node = |c: char| c.is_ascii_lowercase().then(|| c as u8 - b'a');
        let mut fanins = Vec::new();
        let mut chars = gates.chars().peekable();
        while let Some(c) = chars.next() {
            let complemented = chars.next_if_eq(&'\'').is_some();
            fanins.push((node(c)?, complemented));
        }
        if fanins.len() % 2 == 1 {
            return None;
        }
        let gates: Vec<[(u8, bool); 2]> = fanins.chunks(2).map(|p| [p[0], p[1]]).collect();
        if gates.iter().enumerate().any(|(i, g)| g.iter().any(|&(n, _)| n as usize >= 4 + i)) {
            return None;
        }
        let (out, negated) = match out {
            "0" => (None, false),
            "1" => (None, true),
            _ => {
                let mut chars = out.chars();
                let n = node(chars.next()?)?;
                let negated = match chars.as_str() {
                    "" => false,
                    "'" => true,
                    _ => return None,
                };
                if n as usize >= 4 + gates.len() {
                    return None;
                }
                (Some(n), negated)
            }
        };
        Some(Circuit { gates, out, negated })
    }
}

/// The circuits of every NPN class of four inputs, found by SAT-based exact synthesis ahead of time (the rewrite tests
/// regenerate them and hold this text to what they find): a line a class, its canonical truth table in hex, the first
/// size the search gave up at (`-` for none, so that the first circuit is proven least), then its circuits in the text
/// [`Circuit::parse`] reads, every one of the least size found up to a cap and a few of one gate more.
const LIBRARY: &str = include_str!("rewrite_npn4.txt");

/// A class of the library: its circuits, fewest gates first, and the first size the search gave up at, below which
/// the least of them is proven least.
#[derive(Debug, Default)]
struct Class {
    circuits: Vec<Circuit>,
    gave_up: Option<usize>,
}

impl Class {
    /// Whether the least circuit is proven the least: no smaller size given up on.
    fn least(&self) -> bool {
        self.circuits.first().is_some_and(|b| self.gave_up.is_none_or(|r| r > b.gates.len()))
    }
}

/// The classes of a library text by canonical truth table. A malformed line, or a circuit that does not compute its
/// class, is refused loudly: the text ships with the crate.
fn parse_library(text: &str) -> HashMap<Truth, Class> {
    let mut classes = HashMap::new();
    for line in text.lines().filter(|l| !l.is_empty() && !l.starts_with('#')) {
        let mut words = line.split_whitespace();
        let c = words.next().and_then(|w| Truth::from_str_radix(w, 16).ok()).expect("a class in hex");
        let gave_up = match words.next().expect("the size given up at") {
            "-" => None,
            r => Some(r.parse().expect("a size")),
        };
        let circuits: Vec<Circuit> = words.map(|w| Circuit::parse(w).expect("a circuit")).collect();
        assert!(circuits.iter().all(|k| k.truth() == c), "class {c:04x}: a circuit that does not compute it");
        classes.insert(c, Class { circuits, gave_up });
    }
    classes
}

/// The shipped library's classes, read once a process.
fn shipped() -> &'static HashMap<Truth, Class> {
    static CLASSES: OnceLock<HashMap<Truth, Class>> = OnceLock::new();
    CLASSES.get_or_init(|| parse_library(LIBRARY))
}

/// What a rewrite reads of the library: each function's canonical form and transform to it, and the classes met.
struct Library {
    transforms: Vec<Transform>,
    canon: HashMap<Truth, (Truth, Transform)>,
    met: HashSet<Truth>,
}

impl Library {
    fn new() -> Library {
        Library { transforms: transforms(), canon: HashMap::new(), met: HashSet::new() }
    }

    /// `f`'s canonical form and a transform taking `f` to it.
    fn canonical(&mut self, f: Truth) -> (Truth, Transform) {
        if let Some(&c) = self.canon.get(&f) {
            return c;
        }
        let best = self.transforms.iter().map(|&t| (apply(f, t), t)).min_by_key(|&(g, _)| g).expect("a transform");
        self.canon.insert(f, best);
        best
    }

    /// `f`'s class and the transform taking `f` to it.
    fn class(&mut self, f: Truth) -> (&'static Class, Transform) {
        let (c, t) = self.canonical(f);
        self.met.insert(c);
        (shipped().get(&c).expect("every class of four inputs is in the library"), t)
    }

}

/// Where a circuit is built: And gates folded and structurally hashed.
trait Gates {
    fn and(&mut self, a: AigLit, b: AigLit) -> AigLit;
}

impl Gates for Aig {
    fn and(&mut self, a: AigLit, b: AigLit) -> AigLit {
        Aig::and(self, a, b)
    }
}

impl Gates for Network {
    fn and(&mut self, a: AigLit, b: AigLit) -> AigLit {
        Network::and(self, a, b)
    }
}

/// The literals a class's circuit reads as its inputs to compute, through the transform `t` from the function to its
/// class, the function over `leaves` (up to four; a missing leaf false).
fn inputs(t: Transform, leaves: &[AigLit]) -> [AigLit; 4] {
    let mut at = [AigLit::FALSE; 4];
    for i in 0..4 {
        let leaf = leaves.get(i).copied().unwrap_or(AigLit::FALSE);
        at[t.perm[i] as usize] = leaf.with_phase(t.neg >> i & 1 == 1);
    }
    at
}

/// `circuit`, of the class `t` takes a function to, built into `g` to compute that function over `leaves`.
fn implement<G: Gates>(circuit: &Circuit, t: Transform, leaves: &[AigLit], g: &mut G) -> AigLit {
    let mut node: Vec<AigLit> = inputs(t, leaves).to_vec();
    for &[(a, ca), (b, cb)] in &circuit.gates {
        let gate = g.and(node[a as usize].with_phase(ca), node[b as usize].with_phase(cb));
        node.push(gate);
    }
    let out = circuit.out.map_or(AigLit::FALSE, |o| node[o as usize]);
    out.with_phase(circuit.negated ^ t.out)
}

/// What [`implement`] of `circuit` in `root`'s place costs, the nodes [`Network::freed`] found last being those the
/// replacement frees: each gate the network does not hold, and each it holds among those freed, which the replacement
/// then keeps (a gate held reads the cut's leaves or gates the circuit holds before it, so nothing below it is kept but
/// what is counted). `None` when it would read `root` itself, or cost more than `bound`.
fn cost(net: &Network, root: u32, circuit: &Circuit, t: Transform, leaves: &[AigLit], bound: usize) -> Option<usize> {
    let mut node: Vec<Option<AigLit>> = inputs(t, leaves).iter().map(|&l| Some(l)).collect();
    let (mut added, mut kept) = (0, Vec::new());
    for &[(a, ca), (b, cb)] in &circuit.gates {
        let pair = node[a as usize].zip(node[b as usize]);
        let held = pair.and_then(|(x, y)| net.lookup(x.with_phase(ca), y.with_phase(cb)));
        match held {
            Some(l) if l.node() == root => return None,
            Some(l) if net.frees(l.node()) && !kept.contains(&l.node()) => kept.push(l.node()),
            Some(_) => {}
            None => added += 1,
        }
        if added + kept.len() > bound {
            return None;
        }
        node.push(held);
    }
    Some(added + kept.len())
}

/// A cut's leaves, sorted: up to four nodes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Leaves {
    len: u8,
    at: [u32; 4],
}

impl Leaves {
    fn one(n: u32) -> Leaves {
        Leaves { len: 1, at: [n, 0, 0, 0] }
    }

    fn nodes(&self) -> &[u32] {
        &self.at[..self.len as usize]
    }

    /// The leaves of both, if at most four.
    fn union(&self, other: &Leaves) -> Option<Leaves> {
        let (mut out, mut len) = ([0u32; 4], 0);
        let (x, y) = (self.nodes(), other.nodes());
        let (mut i, mut j) = (0, 0);
        while i < x.len() || j < y.len() {
            let next = match (x.get(i), y.get(j)) {
                (Some(&a), Some(&b)) if a == b => {
                    i += 1;
                    j += 1;
                    a
                }
                (Some(&a), Some(&b)) if a < b => {
                    i += 1;
                    a
                }
                (Some(_), Some(&b)) | (None, Some(&b)) => {
                    j += 1;
                    b
                }
                (Some(&a), None) => {
                    i += 1;
                    a
                }
                (None, None) => unreachable!(),
            };
            if len == 4 {
                return None;
            }
            out[len] = next;
            len += 1;
        }
        Some(Leaves { len: len as u8, at: out })
    }

    /// Whether every leaf of `self` is one of `other`'s.
    fn within(&self, other: &Leaves) -> bool {
        self.nodes().iter().all(|l| other.nodes().contains(l))
    }
}

/// Each node's cuts as last enumerated, and the version of the node they were enumerated at: the trivial cut first,
/// then at most `keep` more, fewest leaves first.
struct Cuts {
    keep: usize,
    of: Vec<Option<(u32, Vec<Leaves>)>>,
}

impl Cuts {
    fn new(keep: usize) -> Cuts {
        Cuts { keep, of: Vec::new() }
    }

    fn fresh(&self, net: &Network, n: u32) -> bool {
        self.of.get(n as usize).and_then(Option::as_ref).is_some_and(|&(v, _)| v == net.version(n))
    }

    /// Node `n`'s cuts, enumerated now if the node has changed since, or never had them, and so for its fan-ins.
    fn of(&mut self, net: &Network, n: u32) -> &[Leaves] {
        if self.of.len() < net.len() {
            self.of.resize(net.len(), None);
        }
        let mut stack = vec![n];
        while let Some(&m) = stack.last() {
            if self.fresh(net, m) {
                stack.pop();
                continue;
            }
            let Kind::And(a, b) = net.kind(m) else {
                self.of[m as usize] = Some((net.version(m), vec![Leaves::one(m)]));
                stack.pop();
                continue;
            };
            let open: Vec<u32> = [a.node(), b.node()].into_iter().filter(|&f| !self.fresh(net, f)).collect();
            if !open.is_empty() {
                stack.extend(open);
                continue;
            }
            stack.pop();
            let (left, right) = (&self.of[a.node() as usize], &self.of[b.node() as usize]);
            let (left, right) = (&left.as_ref().expect("fresh").1, &right.as_ref().expect("fresh").1);
            let mut found: Vec<Leaves> = Vec::new();
            for x in left {
                for y in right {
                    if let Some(u) = x.union(y) {
                        if !found.contains(&u) {
                            found.push(u);
                        }
                    }
                }
            }
            let dominated = |c: &Leaves, all: &[Leaves]| all.iter().any(|d| d.len < c.len && d.within(c));
            let mut kept: Vec<Leaves> = found.iter().filter(|c| !dominated(c, &found)).copied().collect();
            kept.sort_unstable();
            kept.truncate(self.keep);
            kept.insert(0, Leaves::one(m));
            self.of[m as usize] = Some((net.version(m), kept));
        }
        &self.of[n as usize].as_ref().expect("enumerated").1
    }
}

/// The most nodes [`truth`] reads between a node and its cut's leaves.
const CONE: usize = 24;

/// The function of `root` over `leaves` read off the network, `None` unless its cone ends at them, live, within
/// [`CONE`] nodes (a cut enumerated before the network changed below it may not).
fn truth(net: &Network, root: u32, leaves: &[u32]) -> Option<Truth> {
    if leaves.iter().any(|&l| net.kind(l) == Kind::Freed) {
        return None;
    }
    let mut known: Vec<(u32, Truth)> = leaves.iter().zip(INPUT).map(|(&l, t)| (l, t)).collect();
    let get = |known: &[(u32, Truth)], l: AigLit| {
        known.iter().find(|&&(m, _)| m == l.node()).map(|&(_, t)| if l.is_complemented() { !t } else { t })
    };
    let mut stack = vec![root];
    while let Some(&n) = stack.last() {
        if known.iter().any(|&(m, _)| m == n) {
            stack.pop();
            continue;
        }
        let Kind::And(a, b) = net.kind(n) else { return None };
        if known.len() > CONE + leaves.len() || stack.len() > 2 * CONE {
            return None;
        }
        match (get(&known, a), get(&known, b)) {
            (Some(x), Some(y)) => {
                known.push((n, x & y));
                stack.pop();
            }
            (x, y) => {
                if x.is_none() {
                    stack.push(a.node());
                }
                if y.is_none() {
                    stack.push(b.node());
                }
            }
        }
    }
    get(&known, AigLit::new(root, false))
}

/// A node's replacement chosen: its gain, the cut, the cut's class, the transform to it, and which of its circuits.
struct Choice {
    gain: usize,
    leaves: Leaves,
    class: &'static Class,
    t: Transform,
    circuit: usize,
}

/// One pass over the network in topological order: each node live when it is met replaced by the circuit of most
/// gain over its cuts, if one gains. The nodes rewritten, and the gain counted, which the pass saves at least.
fn pass(net: &mut Network, opts: &Options, library: &mut Library) -> (usize, usize) {
    let mut cuts = Cuts::new(opts.cuts);
    let (mut rewritten, mut gained) = (0, 0);
    for n in net.topological() {
        if !matches!(net.kind(n), Kind::And(..)) {
            continue;
        }
        let mut best: Option<Choice> = None;
        for leaves in cuts.of(net, n)[1..].to_vec() {
            let Some(f) = truth(net, n, leaves.nodes()) else { continue };
            let freed = net.freed(n, leaves.nodes()).len();
            if freed <= best.as_ref().map_or(0, |b| b.gain) {
                continue;
            }
            let (class, t) = library.class(f);
            let lits: Vec<AigLit> = leaves.nodes().iter().map(|&l| AigLit::new(l, false)).collect();
            for (k, circuit) in class.circuits.iter().enumerate() {
                let floor = best.as_ref().map_or(0, |b| b.gain);
                if freed <= floor {
                    break;
                }
                if let Some(cost) = cost(net, n, circuit, t, &lits, freed - floor - 1) {
                    best = Some(Choice { gain: freed - cost, leaves, class, t, circuit: k });
                }
            }
        }
        let Some(choice) = best else { continue };
        let lits: Vec<AigLit> = choice.leaves.nodes().iter().map(|&l| AigLit::new(l, false)).collect();
        let before = net.num_ands();
        let out = implement(&choice.class.circuits[choice.circuit], choice.t, &lits, net);
        net.replace(n, out);
        debug_assert!(net.num_ands() + choice.gain <= before, "a rewrite saves at least the gain it counted");
        rewritten += 1;
        gained += choice.gain;
    }
    debug_assert_eq!(net.check(), Ok(()), "a pass leaves the network whole");
    (rewritten, gained)
}

/// Rewrite `aig`'s cuts by their classes' circuits, pass after pass, and prove the result equal to it: `None` when the
/// proof does not check.
pub fn rewrite(aig: &Aig, opts: &Options) -> Option<Report> {
    let mut library = Library::new();
    let before = aig.num_ands();
    let mut net = Network::new(aig);
    let (mut rewritten, mut rounds) = (0, 0);
    while rounds < opts.rounds.max(1) {
        let (count, _) = pass(&mut net, opts, &mut library);
        rounds += 1;
        rewritten += count;
        if count == 0 {
            break;
        }
    }
    let current = net.to_aig(aig);
    let (g, pairs) = side_by_side(aig, &current)?;
    let (verdicts, proof, _) = fraig::decide(&g, &pairs, &fraig::Options::default());
    if !verdicts.iter().all(|v| *v == Verdict::Equal) {
        return None;
    }
    let after = current.num_ands();
    let classes = library.met.len();
    let minimal = library.met.iter().filter(|c| shipped()[c].least()).count();
    let report = Report { aig: current, before, after, rewritten, rounds, classes, minimal, proof };
    report.check(aig).then_some(report)
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_proof/rewrite/tests.rs"]
mod tests;
