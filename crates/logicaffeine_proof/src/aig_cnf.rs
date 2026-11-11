//! **CNF of an and-inverter graph by a cover of its cones with cuts** (Eén, Mishchenko & Sörensson, *Applying logic
//! synthesis for speeding up SAT*, SAT 2007). Tseitin's encoding gives every And node a variable and three clauses;
//! here the cones of the roots are covered by cuts of at most `k` leaves, and only a cut's root gets a variable,
//! defined over its leaves' by the clauses of the irredundant sums of products (Minato–Morreale) of its function and of
//! the function's complement. The cover is chosen by area flow (Manohararajah, Brown & Vranesic, 2006), a cut's area
//! the clauses its function takes: the variables a solver then searches over are the few roots the cover keeps, each
//! a function of several gates.
//!
//! A function is a truth table over at most [`MAX_LEAVES`] inputs, bit `x` its value where input `i` takes bit `i`
//! of `x`, every bit of the word kept: a function of fewer inputs repeats over the inputs it ignores, so cofactors,
//! swaps and conjunctions are word operations whatever the width.
//!
//! An instance of a cut ([`Instances::instance`]) is its function over signals, each a constant or a solver literal:
//! a constant leaf is cofactored away, a leaf another one equals or complements is merged into it, an input the
//! function then ignores is dropped, and what is left (a constant, a leaf's literal, or a function of the leaves'
//! variables in ascending order, made false on the all-false input by complementing it) is one variable for every
//! instance of one function over the same variables.

use std::collections::HashMap;

use crate::aig::{Aig, AigLit, Node};
use crate::cdcl::{Lit, Solver, Var};

/// The most leaves a cut has.
pub(crate) const MAX_LEAVES: usize = 6;

/// A function of at most [`MAX_LEAVES`] inputs.
pub(crate) type Truth = u64;

/// Input `i`'s truth table.
const INPUT: [Truth; MAX_LEAVES] = [
    0xAAAA_AAAA_AAAA_AAAA,
    0xCCCC_CCCC_CCCC_CCCC,
    0xF0F0_F0F0_F0F0_F0F0,
    0xFF00_FF00_FF00_FF00,
    0xFFFF_0000_FFFF_0000,
    0xFFFF_FFFF_0000_0000,
];

/// The cuts a node keeps while the cover is chosen.
const CUTS: usize = 8;

/// `f` with input `i` fixed to `value`, repeated over both of its values.
fn cofactor(f: Truth, i: usize, value: bool) -> Truth {
    let s = 1 << i;
    if value {
        let hi = f & INPUT[i];
        hi | (hi >> s)
    } else {
        let lo = f & !INPUT[i];
        lo | (lo << s)
    }
}

/// Whether `f` reads input `i`.
fn depends(f: Truth, i: usize) -> bool {
    cofactor(f, i, false) != cofactor(f, i, true)
}

/// `f` with inputs `i` and `i + 1` exchanged.
fn swap_adjacent(f: Truth, i: usize) -> Truth {
    let s = 1 << i;
    let up = INPUT[i] & !INPUT[i + 1];
    let down = !INPUT[i] & INPUT[i + 1];
    (f & !(up | down)) | ((f & up) << s) | ((f & down) >> s)
}

/// `f` with input `from` moved to place `to` (`from <= to`), the inputs between moved down one place.
fn move_up(mut f: Truth, from: usize, to: usize) -> Truth {
    for i in from..to {
        f = swap_adjacent(f, i);
    }
    f
}

/// `f` with input `i` complemented.
fn flip(f: Truth, i: usize) -> Truth {
    let s = 1 << i;
    ((f & INPUT[i]) >> s) | ((f & !INPUT[i]) << s)
}

/// A function over leaves `from` (ascending) as one over `to` (ascending, holding each of `from`).
fn stretch(f: Truth, from: &[u32], to: &[u32]) -> Truth {
    let mut f = f;
    for (i, leaf) in from.iter().enumerate().rev() {
        let at = to.iter().position(|t| t == leaf).expect("the leaves stretched to hold every leaf");
        f = move_up(f, i, at);
    }
    f
}

/// A product of literals over a function's inputs: the inputs of `pos` true, those of `neg` false.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Cube {
    pub(crate) pos: u8,
    pub(crate) neg: u8,
}

/// An irredundant sum of products covering at least `lower` and at most `upper` over the first `n` inputs, its cubes
/// pushed to `cubes`; returns the function it is.
fn isop_within(lower: Truth, upper: Truth, n: usize, cubes: &mut Vec<Cube>) -> Truth {
    if lower == 0 {
        return 0;
    }
    if upper == !0 {
        cubes.push(Cube { pos: 0, neg: 0 });
        return !0;
    }
    let reads = |i: &usize| depends(lower, *i) || depends(upper, *i);
    let i = (0..n).rev().find(reads).expect("a bound neither constant reads an input");
    let (l0, l1) = (cofactor(lower, i, false), cofactor(lower, i, true));
    let (u0, u1) = (cofactor(upper, i, false), cofactor(upper, i, true));
    let start = cubes.len();
    let r0 = isop_within(l0 & !u1, u0, i, cubes);
    for c in &mut cubes[start..] {
        c.neg |= 1 << i;
    }
    let mid = cubes.len();
    let r1 = isop_within(l1 & !u0, u1, i, cubes);
    for c in &mut cubes[mid..] {
        c.pos |= 1 << i;
    }
    let rest = isop_within((l0 & !r0) | (l1 & !r1), u0 & u1, i, cubes);
    (r0 & !INPUT[i]) | (r1 & INPUT[i]) | rest
}

/// The irredundant sum of products of `f` over its first `n` inputs (Minato–Morreale): prime cubes whose disjunction
/// is `f`, none of them covered by the others.
pub(crate) fn isop(f: Truth, n: usize) -> Vec<Cube> {
    let mut cubes = Vec::new();
    let made = isop_within(f, f, n, &mut cubes);
    debug_assert_eq!(made, f, "an exact cover is the function");
    cubes
}

/// A cut: its leaves, nodes in ascending order, and its root's function over them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Cut {
    leaves: [u32; MAX_LEAVES],
    len: u8,
    pub(crate) truth: Truth,
    flow: f32,
}

impl Cut {
    pub(crate) fn leaves(&self) -> &[u32] {
        &self.leaves[..self.len as usize]
    }

    fn trivial(n: u32) -> Cut {
        let mut leaves = [0; MAX_LEAVES];
        leaves[0] = n;
        Cut { leaves, len: 1, truth: INPUT[0], flow: 0.0 }
    }

    /// The union of two cuts' leaves, ascending, when it has at most `k`.
    fn union(&self, other: &Cut, k: usize) -> Option<([u32; MAX_LEAVES], u8)> {
        let (a, b) = (self.leaves(), other.leaves());
        let mut out = [0; MAX_LEAVES];
        let (mut i, mut j, mut len) = (0, 0, 0);
        while i < a.len() || j < b.len() {
            let next = if j == b.len() || (i < a.len() && a[i] < b[j]) {
                i += 1;
                a[i - 1]
            } else if i == a.len() || b[j] < a[i] {
                j += 1;
                b[j - 1]
            } else {
                i += 1;
                j += 1;
                a[i - 1]
            };
            if len == k {
                return None;
            }
            out[len] = next;
            len += 1;
        }
        Some((out, len as u8))
    }

    /// Whether every leaf of `self` is one of `other`'s.
    fn within(&self, other: &Cut) -> bool {
        self.leaves().iter().all(|l| other.leaves().contains(l))
    }
}

/// The clauses a function takes: those of its irredundant sum of products and of its complement's.
fn area(f: Truth, n: usize, memo: &mut HashMap<(Truth, u8), u32>) -> u32 {
    *memo.entry((f, n as u8)).or_insert_with(|| (isop(f, n).len() + isop(!f, n).len()) as u32)
}

/// `cut` with the leaves its function ignores dropped.
fn trimmed(mut cut: Cut) -> Cut {
    let mut i = 0;
    while i < cut.len as usize {
        if depends(cut.truth, i) {
            i += 1;
            continue;
        }
        let last = cut.len as usize - 1;
        cut.truth = move_up(cut.truth, i, last);
        cut.leaves.copy_within(i + 1..=last, i);
        cut.len -= 1;
    }
    cut
}

/// The cover of the cones of some roots: each node of it an And node's cut, chosen by area flow among the priority
/// cuts each node keeps.
pub(crate) struct Mapping {
    cuts: Vec<Option<Cut>>,
}

impl Mapping {
    /// The cut covering node `n`, for an And node the cover keeps.
    pub(crate) fn cut(&self, n: u32) -> Option<&Cut> {
        self.cuts.get(n as usize).and_then(Option::as_ref)
    }
}

/// Cover the cones of `roots` with cuts of at most `k` leaves (`2 ≤ k ≤ MAX_LEAVES`): each And node keeps [`CUTS`]
/// cuts of least area flow, enumerated from its fan-ins' (Pan & Lin; Mishchenko, Cho, Chatterjee & Brayton, *Priority
/// cuts*, ICCAD 2007), and the cover takes each node's cut of least flow from the roots down.
pub(crate) fn map(aig: &Aig, roots: &[AigLit], k: usize) -> Mapping {
    assert!((2..=MAX_LEAVES).contains(&k), "a cut has two to {MAX_LEAVES} leaves");
    let nodes = aig.num_nodes();
    let cone = aig.cone(roots);
    let mut fanout = vec![0u32; nodes];
    for &n in &cone {
        if let Node::And(a, b) = aig.node(n) {
            fanout[a.node() as usize] += 1;
            fanout[b.node() as usize] += 1;
        }
    }
    for r in roots {
        fanout[r.node() as usize] += 1;
    }
    let mut remaining = fanout.clone();
    let mut memo = HashMap::new();
    let mut cuts: Vec<Vec<Cut>> = vec![Vec::new(); nodes];
    let mut best: Vec<Option<Cut>> = vec![None; nodes];
    let flow_of = |best: &[Option<Cut>], fanout: &[u32], n: u32| {
        best[n as usize].map_or(0.0, |c| c.flow / fanout[n as usize].max(1) as f32)
    };
    for &n in &cone {
        let Node::And(a, b) = aig.node(n) else { continue };
        let fanin = |x: AigLit, cuts: &[Vec<Cut>]| -> Vec<Cut> {
            let mut all = vec![Cut::trivial(x.node())];
            all.extend(cuts[x.node() as usize].iter().copied());
            all
        };
        let (left, right) = (fanin(a, &cuts), fanin(b, &cuts));
        let mut found: Vec<Cut> = Vec::new();
        for x in &left {
            for y in &right {
                let Some((leaves, len)) = x.union(y, k) else { continue };
                let to = &leaves[..len as usize];
                let tx = stretch(x.truth, x.leaves(), to) ^ if a.is_complemented() { !0 } else { 0 };
                let ty = stretch(y.truth, y.leaves(), to) ^ if b.is_complemented() { !0 } else { 0 };
                let cut = trimmed(Cut { leaves, len, truth: tx & ty, flow: 0.0 });
                if found.iter().any(|f| f.leaves() == cut.leaves()) {
                    continue;
                }
                let leaves_flow: f32 = cut.leaves().iter().map(|&l| flow_of(&best, &fanout, l)).sum();
                let flow = area(cut.truth, cut.len as usize, &mut memo) as f32 + leaves_flow;
                found.push(Cut { flow, ..cut });
            }
        }
        let dominated = |c: &Cut, all: &[Cut]| all.iter().any(|d| d.len < c.len && d.within(c));
        let mut kept: Vec<Cut> = found.iter().filter(|c| !dominated(c, &found)).copied().collect();
        kept.sort_by(|x, y| x.flow.total_cmp(&y.flow).then(x.len.cmp(&y.len)).then(x.leaves().cmp(y.leaves())));
        kept.retain(|c| !(c.len == 1 && c.leaves[0] == n));
        kept.truncate(CUTS);
        best[n as usize] = kept.first().copied();
        cuts[n as usize] = kept;
        for f in [a.node(), b.node()] {
            remaining[f as usize] -= 1;
            if remaining[f as usize] == 0 {
                cuts[f as usize] = Vec::new();
            }
        }
    }
    let mut chosen: Vec<Option<Cut>> = vec![None; nodes];
    let mut stack: Vec<u32> = roots.iter().map(|r| r.node()).collect();
    while let Some(n) = stack.pop() {
        if chosen[n as usize].is_some() {
            continue;
        }
        if let Some(cut) = best[n as usize] {
            stack.extend(cut.leaves());
            chosen[n as usize] = Some(cut);
        }
    }
    Mapping { cuts: chosen }
}

/// A signal of an encoded circuit, packed in a word: false, true, or a solver literal; [`Signal::UNMADE`] marks one
/// not made yet.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub(crate) struct Signal(u32);

impl Signal {
    pub(crate) const UNMADE: Signal = Signal(u32::MAX);
    pub(crate) const FALSE: Signal = Signal(0);
    pub(crate) const TRUE: Signal = Signal(1);

    pub(crate) fn lit(l: Lit) -> Signal {
        Signal(l.index() as u32 + 2)
    }

    pub(crate) fn constant(b: bool) -> Signal {
        if b {
            Signal::TRUE
        } else {
            Signal::FALSE
        }
    }

    /// The literal, for a signal that is one.
    pub(crate) fn as_lit(self) -> Option<Lit> {
        (self.0 >= 2 && self != Signal::UNMADE).then(|| {
            let code = self.0 - 2;
            Lit::new(code >> 1, code & 1 == 0)
        })
    }

    /// This signal complemented when `flip` says so.
    pub(crate) fn with_phase(self, flip: bool) -> Signal {
        if flip && self != Signal::UNMADE {
            Signal(self.0 ^ 1)
        } else {
            self
        }
    }
}

/// The cut instances made into one solver ([`Instances::instance`]), and the sums of products of the functions they
/// took.
#[derive(Default)]
pub(crate) struct Instances {
    made: HashMap<(Truth, [u32; MAX_LEAVES]), Signal>,
    covers: HashMap<(Truth, u8), (Vec<Cube>, Vec<Cube>)>,
}

impl Instances {
    /// The signal of function `truth` over the signals `leaves`, made in `solver` when no instance of that function
    /// over those variables was: a fresh variable and its defining clauses. See the module documentation.
    pub(crate) fn instance(&mut self, truth: Truth, leaves: &[Signal], solver: &mut Solver) -> Signal {
        let mut f = truth;
        let mut vars: Vec<(Var, usize)> = Vec::with_capacity(leaves.len());
        for (i, &s) in leaves.iter().enumerate() {
            match s.as_lit() {
                None => f = cofactor(f, i, s == Signal::TRUE),
                Some(l) => {
                    if !l.is_positive() {
                        f = flip(f, i);
                    }
                    match vars.iter().find(|&&(v, _)| v == l.var()) {
                        Some(&(_, j)) => {
                            f = (cofactor(f, i, true) & INPUT[j]) | (cofactor(f, i, false) & !INPUT[j]);
                        }
                        None => vars.push((l.var(), i)),
                    }
                }
            }
        }
        vars.retain(|&(_, i)| depends(f, i));
        vars.sort_unstable();
        let mut g: Truth = 0;
        for x in 0..(1u32 << vars.len()) {
            let mut at = 0usize;
            for (bit, &(_, i)) in vars.iter().enumerate() {
                if x >> bit & 1 == 1 {
                    at |= 1 << i;
                }
            }
            if f >> at & 1 == 1 {
                for rep in (x as usize..64).step_by(1 << vars.len()) {
                    g |= 1 << rep;
                }
            }
        }
        let n = vars.len();
        if n == 0 {
            return Signal::constant(g & 1 == 1);
        }
        if n == 1 && (g == INPUT[0] || g == !INPUT[0]) {
            return Signal::lit(Lit::new(vars[0].0, g == INPUT[0]));
        }
        let complement = g & 1 == 1;
        if complement {
            g = !g;
        }
        let mut key = [u32::MAX; MAX_LEAVES];
        for (slot, &(v, _)) in key.iter_mut().zip(&vars) {
            *slot = v;
        }
        if let Some(&s) = self.made.get(&(g, key)) {
            return s.with_phase(complement);
        }
        let root = Lit::pos(solver.add_vars(1) as Var);
        let (on, off) = self.covers.entry((g, n as u8)).or_insert_with(|| (isop(g, n), isop(!g, n)));
        for (cubes, out) in [(&*on, root), (&*off, root.negated())] {
            for c in cubes {
                let mut clause = vec![out];
                for (i, &(v, _)) in vars.iter().enumerate() {
                    if c.pos >> i & 1 == 1 {
                        clause.push(Lit::neg(v));
                    } else if c.neg >> i & 1 == 1 {
                        clause.push(Lit::pos(v));
                    }
                }
                solver.add_clause(clause);
            }
        }
        let s = Signal::lit(root);
        self.made.insert((g, key), s);
        s.with_phase(complement)
    }
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_proof/aig_cnf/tests.rs"]
mod tests;
