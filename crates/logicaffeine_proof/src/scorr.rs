//! **Signal correspondence**: the signals of a sequential circuit that equal one another, or a constant, at every clock
//! of every run, proven by induction (van Eijk, *Sequential equivalence checking based on structural similarities*,
//! TCAD 2000; Mishchenko, Case, Brayton & Jang, *Scalable and scalably-verifiable sequential synthesis*, ICCAD 2008,
//! the scheme of ABC's `scorr`), and the circuit with each merged into its representative.
//!
//! The signals are the latches and the And nodes no input reaches, so each one is a function of the state alone and an
//! equality between two of them is a set of states. Random runs from the initial states put them into classes of
//! signals alike, or complementary, at every clock so far, a run's clocks counted while its constraints held at every
//! clock before; each class's representative is its lowest node. The classes are then refined until two questions,
//! each put to [`crate::fraig`] as pairs of one combinational circuit, find no difference: whether every initial state
//! satisfies every equality (the base), and whether every successor of a state under a step whose constraints hold
//! does (the step). The step is speculatively reduced: the step's clock 0 is the circuit with each signal replaced by its
//! representative, so a state of a reachable run, where the equalities hold, steps as the circuit steps. A difference
//! splits the classes it tells apart; a pair the sweeper leaves open is taken apart. The classes left hold at every
//! clock of every run whose constraints held at the clocks before it: at clock 0 by the base, and from each clock to
//! the next by the step.
//!
//! [`reduce`] replaces every merged signal by its representative, keeping the inputs and the latches in place, so the
//! reduced circuit computes what the circuit does at those clocks, and a run of one is a run of the other. An invariant
//! of the reduced circuit and the equalities together are an invariant of the circuit ([`invariant`]), which the
//! caller holds to [`crate::itp::check_invariant`] on the circuit itself.

use std::collections::HashMap;

use crate::aig::{Aig, AigLit, Init, Node};
use crate::fraig;
use crate::itp::Invariant;
use crate::sweep::Verdict as Answer;

/// How [`find`] searches.
#[derive(Clone, Copy, Debug)]
pub struct Options {
    /// The random runs the first classes are drawn from: `words` words of 64 runs, each `clocks` clocks long.
    pub words: usize,
    pub clocks: usize,
    pub seed: u64,
    /// The most rounds of base and step questions, and the most nodes their circuits may have together, before the
    /// search gives up and merges nothing: a question whose circuit would carry the nodes swept past the bound is not
    /// asked.
    pub rounds: usize,
    pub nodes: usize,
    /// The most nodes a circuit may have for its correspondence to be sought.
    pub max_nodes: usize,
    /// How the base and the step are swept.
    pub fraig: fraig::Options,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            words: 4,
            clocks: 32,
            seed: 0x5c0_77e5_90de_c0de,
            rounds: 64,
            nodes: 100_000,
            max_nodes: 200_000,
            fraig: fraig::Options { budget: 30, final_budget: 1000, rounds: 2, ..fraig::Options::default() },
        }
    }
}

/// Signals proven to correspond: per node of the circuit, the literal of a lower node it equals at every clock of every
/// run whose constraints held at the clocks before, or the node itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Correspondence {
    pub repr: Vec<AigLit>,
}

impl Correspondence {
    /// The nodes merged into a representative.
    pub fn merged(&self) -> usize {
        self.members().count()
    }

    /// Each merged node and the literal it equals.
    fn members(&self) -> impl Iterator<Item = (u32, AigLit)> + '_ {
        self.repr.iter().enumerate().filter(|(n, r)| r.node() as usize != *n).map(|(n, &r)| (n as u32, r))
    }
}

/// The nodes no input reaches: the constant, the latches, and every And node over them.
fn state_signals(aig: &Aig) -> Vec<bool> {
    let mut state = vec![false; aig.num_nodes()];
    for n in 0..aig.num_nodes() {
        state[n] = match aig.node(n as u32) {
            Node::False | Node::Latch(_) => true,
            Node::Input(_) => false,
            Node::And(a, b) => state[a.node() as usize] && state[b.node() as usize],
        };
    }
    state
}

/// The classes as they are refined: per signal, its class's representative and whether it is that representative's
/// complement; a node that is no signal is its own.
struct Classes {
    rep: Vec<u32>,
    flip: Vec<bool>,
}

impl Classes {
    /// Every signal in one class with the constant, each signal's phase its value in the first pattern of `values`
    /// (node `n`'s words at `values[n * words..]`), then split by the patterns `mask` keeps.
    fn first(signals: &[bool], values: &[u64], words: usize, mask: &[u64]) -> Classes {
        let n = signals.len();
        let first = (0..words).find_map(|w| (mask[w] != 0).then(|| (w, mask[w].trailing_zeros())));
        let mut flip = vec![false; n];
        if let Some((w, bit)) = first {
            for (k, f) in flip.iter_mut().enumerate() {
                *f = signals[k] && values[k * words + w] >> bit & 1 == 1;
            }
        }
        let mut classes = Classes { rep: (0..n as u32).map(|k| if signals[k as usize] { 0 } else { k }).collect(), flip };
        classes.refine(values, words, mask);
        classes
    }

    fn lit(&self, n: u32) -> AigLit {
        AigLit::new(self.rep[n as usize], self.flip[n as usize])
    }

    fn is_member(&self, n: u32) -> bool {
        self.rep[n as usize] != n
    }

    /// Split every class by the patterns `mask` keeps of `values`: the members whose values, in their
    /// representative's phase, are not the representative's on those patterns leave it, each group of them alike there
    /// a class of its own under its lowest node. Whether any class split.
    fn refine(&mut self, values: &[u64], words: usize, mask: &[u64]) -> bool {
        let phase = |v: u64, flip: bool| if flip { !v } else { v };
        let key = |n: usize, flip: bool| -> Vec<u64> {
            (0..words).map(|w| phase(values[n * words + w], flip) & mask[w]).collect()
        };
        let mut movers: HashMap<(u32, Vec<u64>), (u32, bool)> = HashMap::new();
        let mut split = false;
        for n in 0..self.rep.len() {
            let r = self.rep[n] as usize;
            if r == n {
                continue;
            }
            let k = key(n, self.flip[n]);
            if k == key(r, false) {
                continue;
            }
            split = true;
            match movers.get(&(r as u32, k.clone())) {
                Some(&(first, first_flip)) => {
                    self.rep[n] = first;
                    self.flip[n] ^= first_flip;
                }
                None => {
                    movers.insert((r as u32, k), (n as u32, self.flip[n]));
                    self.rep[n] = n as u32;
                    self.flip[n] = false;
                }
            }
        }
        split
    }

    /// The signal `n` taken apart from its class: a class of its own, and the class's other members kept.
    fn separate(&mut self, n: u32) {
        self.rep[n as usize] = n;
        self.flip[n as usize] = false;
    }
}

/// A word generator for the random runs (xorshift).
struct Words(u64);

impl Words {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

/// The signals of `aig` that correspond, or `None` once [`Options::rounds`] or [`Options::nodes`] end first or the
/// circuit has more than [`Options::max_nodes`] nodes.
pub fn find(aig: &Aig, opts: &Options) -> Option<Correspondence> {
    if aig.num_nodes() > opts.max_nodes {
        return None;
    }
    let signals = state_signals(aig);
    let mut classes = simulated(aig, &signals, opts);
    let mut swept = 0;
    let mut afford = |question: &Question| {
        swept += question.aig.num_nodes();
        swept <= opts.nodes
    };
    for _ in 0..opts.rounds {
        let question = base(aig, &signals, &classes);
        if !afford(&question) {
            return None;
        }
        if settle(&mut classes, &signals, &question, &opts.fraig) {
            continue;
        }
        let question = step(aig, &signals, &classes);
        if !afford(&question) {
            return None;
        }
        if !settle(&mut classes, &signals, &question, &opts.fraig) {
            let repr = (0..aig.num_nodes() as u32).map(|n| classes.lit(n)).collect();
            return Some(Correspondence { repr });
        }
    }
    None
}

/// The first classes, from random runs of [`Options::clocks`] clocks from the initial states, a free initial value at
/// random; a run's clock counts while its constraints held at every clock before it.
fn simulated(aig: &Aig, signals: &[bool], opts: &Options) -> Classes {
    let words = opts.words.max(1);
    let mut random = Words(opts.seed | 1);
    let mut state: Vec<u64> = Vec::with_capacity(aig.latches().len() * words);
    for latch in aig.latches() {
        for _ in 0..words {
            state.push(match latch.init {
                Init::Zero => 0,
                Init::One => !0,
                Init::Free => random.next(),
            });
        }
    }
    let mut mask = vec![!0u64; words];
    let mut classes: Option<Classes> = None;
    for _ in 0..opts.clocks.max(1) {
        let inputs: Vec<u64> = (0..aig.inputs().len() * words).map(|_| random.next()).collect();
        let values = aig.simulate(words, &inputs, &state);
        match classes.as_mut() {
            None => classes = Some(Classes::first(signals, &values, words, &mask)),
            Some(c) => {
                c.refine(&values, words, &mask);
            }
        }
        for c in aig.constraints() {
            for (w, m) in mask.iter_mut().enumerate() {
                *m &= Aig::word(&values, words, c.lit, w);
            }
        }
        if mask.iter().all(|&m| m == 0) {
            break;
        }
        state = aig.latches().iter().flat_map(|l| (0..words).map(|w| Aig::word(&values, words, l.next, w))).collect();
    }
    classes.expect("at least one clock is simulated")
}

/// A question about the classes as pairs of one combinational circuit: the circuit, the signal pairs (a member, its
/// representative in the member's phase), and each signal's literal in it, whose values under a difference split the
/// classes.
struct Question {
    aig: Aig,
    pairs: Vec<(AigLit, AigLit)>,
    members: Vec<u32>,
    signal: Vec<AigLit>,
}

/// Put `question` to the sweeper and split the classes by its answers: a difference splits every class it tells apart,
/// up to 64 at a time, and a pair left open takes its member apart. Whether any class changed.
fn settle(classes: &mut Classes, signals: &[bool], question: &Question, opts: &fraig::Options) -> bool {
    if question.pairs.is_empty() {
        return false;
    }
    let (answers, _, _) = fraig::decide(&question.aig, &question.pairs, opts);
    let mut changed = false;
    let mut batch: Vec<Vec<bool>> = Vec::new();
    for (&m, answer) in question.members.iter().zip(&answers) {
        match answer {
            Answer::Equal => {}
            Answer::Differ { inputs, latches } => batch.push(question.aig.eval(inputs, latches)),
            Answer::Unknown(_) => {
                classes.separate(m);
                changed = true;
            }
        }
    }
    for chunk in batch.chunks(64) {
        let mut values = vec![0u64; signals.len()];
        for (bit, eval) in chunk.iter().enumerate() {
            for (n, v) in values.iter_mut().enumerate() {
                if signals[n] && Aig::value(eval, question.signal[n]) {
                    *v |= 1 << bit;
                }
            }
        }
        let mask = [if chunk.len() == 64 { !0 } else { (1u64 << chunk.len()) - 1 }];
        changed |= classes.refine(&values, 1, &mask);
    }
    changed
}

/// The pairs of a question: each member against its representative, both at the literals `at` gives them, each side
/// conjoined with `guard`.
fn pairs_of(classes: &Classes, signals: &[bool], at: &[AigLit], out: &mut Aig, guard: AigLit) -> (Vec<(AigLit, AigLit)>, Vec<u32>) {
    let mut pairs = Vec::new();
    let mut members = Vec::new();
    for n in 0..signals.len() as u32 {
        if !signals[n as usize] || !classes.is_member(n) {
            continue;
        }
        let r = classes.lit(n);
        let member = out.and(at[n as usize], guard);
        let rep = out.and(at[r.node() as usize].with_phase(r.is_complemented()), guard);
        pairs.push((member, rep));
        members.push(n);
    }
    (pairs, members)
}

/// The base: every signal at the initial states, a latch free to start anywhere an input of its own.
fn base(aig: &Aig, signals: &[bool], classes: &Classes) -> Question {
    let mut out = Aig::new();
        let mut at = vec![AigLit::FALSE; aig.num_nodes()];
        for n in 0..aig.num_nodes() {
            if !signals[n] {
                continue;
            }
            at[n] = match aig.node(n as u32) {
                Node::False | Node::Input(_) => AigLit::FALSE,
                Node::Latch(k) => match aig.latches()[k].init {
                    Init::Zero => AigLit::FALSE,
                    Init::One => AigLit::TRUE,
                    Init::Free => out.add_input(None),
                },
                Node::And(a, b) => {
                    let (x, y) = (at[a.node() as usize].with_phase(a.is_complemented()), at[b.node() as usize].with_phase(b.is_complemented()));
                    out.and(x, y)
                }
            };
        }
        let (pairs, members) = pairs_of(classes, signals, &at, &mut out, AigLit::TRUE);
        Question { aig: out, pairs, members, signal: at }
}

/// The step, speculatively reduced: clock 0 from any state and inputs, each signal replaced by its representative;
/// clock 1's signals over the latches clock 0 steps them to; each pair conjoined with clock 0's constraints.
fn step(aig: &Aig, signals: &[bool], classes: &Classes) -> Question {
    let mut out = Aig::new();
        let mut now = vec![AigLit::FALSE; aig.num_nodes()];
        for n in 0..aig.num_nodes() as u32 {
            now[n as usize] = if signals[n as usize] && classes.is_member(n) {
                let r = classes.lit(n);
                now[r.node() as usize].with_phase(r.is_complemented())
            } else {
                match aig.node(n) {
                    Node::False => AigLit::FALSE,
                    Node::Input(_) | Node::Latch(_) => out.add_input(None),
                    Node::And(a, b) => {
                        let (x, y) = (now[a.node() as usize].with_phase(a.is_complemented()), now[b.node() as usize].with_phase(b.is_complemented()));
                        out.and(x, y)
                    }
                }
            };
        }
        let at_now = |l: AigLit| now[l.node() as usize].with_phase(l.is_complemented());
        let mut guard = AigLit::TRUE;
        for c in aig.constraints() {
            guard = out.and(guard, at_now(c.lit));
        }
        let mut next = vec![AigLit::FALSE; aig.num_nodes()];
        for n in 0..aig.num_nodes() {
            if !signals[n] {
                continue;
            }
            next[n] = match aig.node(n as u32) {
                Node::False | Node::Input(_) => AigLit::FALSE,
                Node::Latch(k) => at_now(aig.latches()[k].next),
                Node::And(a, b) => {
                    let (x, y) = (next[a.node() as usize].with_phase(a.is_complemented()), next[b.node() as usize].with_phase(b.is_complemented()));
                    out.and(x, y)
                }
            };
        }
        let (pairs, members) = pairs_of(classes, signals, &next, &mut out, guard);
        Question { aig: out, pairs, members, signal: next }
}

/// `aig` with every merged signal replaced by its representative: the same inputs and latches, in place, with their
/// initial values; each latch's next state, the bad states, the constraints and the outputs over the merged logic.
pub fn reduce(aig: &Aig, corr: &Correspondence) -> Aig {
    let mut out = Aig::new();
    let mut map = vec![AigLit::FALSE; aig.num_nodes()];
    for p in aig.inputs() {
        map[p.lit.node() as usize] = out.add_input(p.name.clone());
    }
    let latches: Vec<usize> = aig.latches().iter().map(|l| out.add_latch(l.name.clone(), l.init)).collect();
    for n in 0..aig.num_nodes() as u32 {
        let r = corr.repr[n as usize];
        map[n as usize] = if r.node() != n {
            map[r.node() as usize].with_phase(r.is_complemented())
        } else {
            match aig.node(n) {
                Node::False => AigLit::FALSE,
                Node::Input(_) => map[n as usize],
                Node::Latch(k) => out.latch_lit(latches[k]),
                Node::And(a, b) => {
                    let (x, y) = (map[a.node() as usize].with_phase(a.is_complemented()), map[b.node() as usize].with_phase(b.is_complemented()));
                    out.and(x, y)
                }
            }
        };
    }
    let at = |l: AigLit| map[l.node() as usize].with_phase(l.is_complemented());
    for (k, latch) in aig.latches().iter().enumerate() {
        out.set_next(latches[k], at(latch.next));
    }
    for p in aig.outputs() {
        out.add_output(at(p.lit), p.name.clone());
    }
    for p in aig.bad() {
        out.add_bad(at(p.lit), p.name.clone());
    }
    for p in aig.constraints() {
        out.add_constraint(at(p.lit), p.name.clone());
    }
    out
}

/// An invariant of the circuit [`reduce`] made, over its latches (those of `aig`, in place), as one of `aig`: it and
/// every equality of the correspondence, each signal's function of the latches copied in.
pub fn invariant(aig: &Aig, corr: &Correspondence, reduced: &Invariant) -> Invariant {
    let mut over = Aig::new();
    let latches: Vec<AigLit> = (0..aig.latches().len()).map(|j| over.add_input(Some(format!("l{j}")))).collect();
    let map = over.import(&reduced.aig, &mut |leaf| match leaf {
        crate::aig::Leaf::Input(j) => latches[j],
        crate::aig::Leaf::Latch(_) => unreachable!("an invariant has no latches"),
    });
    let mut root = map[reduced.root.node() as usize].with_phase(reduced.root.is_complemented());
    let roots: Vec<AigLit> = corr.members().flat_map(|(m, r)| [AigLit::new(m, false), r]).collect();
    let mut at = vec![AigLit::FALSE; aig.num_nodes()];
    for n in aig.cone(&roots) {
        at[n as usize] = match aig.node(n) {
            Node::False => AigLit::FALSE,
            Node::Input(_) => unreachable!("a signal is a function of the latches alone"),
            Node::Latch(k) => latches[k],
            Node::And(a, b) => {
                let (x, y) = (at[a.node() as usize].with_phase(a.is_complemented()), at[b.node() as usize].with_phase(b.is_complemented()));
                over.and(x, y)
            }
        };
    }
    for (m, r) in corr.members() {
        let same = over.xnor(at[m as usize], at[r.node() as usize].with_phase(r.is_complemented()));
        root = over.and(root, same);
    }
    Invariant { aig: over, root }
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_proof/scorr/tests.rs"]
mod tests;
