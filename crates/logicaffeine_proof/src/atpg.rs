//! **Automatic test-pattern generation** for single stuck-at faults on and-inverter graphs, every verdict certified.
//!
//! A fault holds one node at a constant. The circuit is read full-scan: its inputs and its latches' current values
//! are what a test sets, its outputs and its latches' next values what a test observes. A test detects a fault when
//! some observed value differs between the circuit and the circuit with the fault.
//!
//! Random patterns go first, 64 to a word: the circuit is simulated once per word, and each fault not yet detected is
//! simulated from its node forward, only where a value differs from the circuit's, so a word costs the faults' effects
//! and not the circuit once per fault. What is left goes to the solver, a fault at a time: its formula is the circuit
//! over the cone the fault can reach an observed value through, a copy of the nodes the fault reaches with the fault's
//! node at its constant, and the clause that some observed value differs ([`miter`]), solved by justification. A model
//! is a test, the inputs and latches the search left open filled at random, simulated again before it counts and then
//! simulated against every fault still left; a refutation proves the fault
//! untestable (the node may be replaced by its constant), its LRAT proof checked against that formula. A solve that
//! runs past its budget leaves the fault aborted, never untestable.

use crate::aig::{Aig, AigLit, Node};
use crate::cdcl::{BudgetedResult, Lit, Solver, Var};
use crate::lrat::{self, Lrat};

/// A node held at a constant.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Fault {
    pub node: u32,
    pub stuck: bool,
}

/// A test: a value for each input and for each latch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Test {
    pub inputs: Vec<bool>,
    pub latches: Vec<bool>,
}

/// What became of a fault.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    /// The test of this index detects it.
    Detected(usize),
    /// No test detects it: a refutation of [`miter`]'s formula for it.
    Untestable(Lrat),
    /// Its solve ran out of budget.
    Aborted,
}

/// How [`generate`] searches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Options {
    /// Words of 64 random patterns simulated before any solve.
    pub random_words: usize,
    pub seed: u64,
    /// Conflicts a fault's solve may spend; 0 is unlimited.
    pub budget: u64,
}

impl Default for Options {
    fn default() -> Options {
        Options { random_words: 16, seed: 0x5eed_a7f9_c0de_0001, budget: 0 }
    }
}

/// The tests found and what became of every fault, with their cost.
#[derive(Clone, Debug)]
pub struct Report {
    /// Each fault on a node other than the constant, both constants, in node order.
    pub faults: Vec<(Fault, Status)>,
    pub tests: Vec<Test>,
    /// Faults detected by random patterns, and solves made.
    pub by_random: usize,
    pub solves: usize,
}

impl Report {
    /// Whether every verdict stands: each detected fault's test detects it, and each untestable fault's refutation
    /// checks against its formula, rebuilt from the circuit.
    pub fn check(&self, aig: &Aig) -> bool {
        self.faults.iter().all(|(fault, status)| match status {
            Status::Detected(t) => self.tests.get(*t).is_some_and(|test| detects(aig, test, *fault)),
            Status::Untestable(proof) => {
                let (num_vars, clauses) = miter(aig, *fault);
                lrat::check(num_vars, &clauses, proof)
            }
            Status::Aborted => true,
        })
    }

    /// The faults detected, untestable and aborted.
    pub fn counts(&self) -> (usize, usize, usize) {
        let mut c = (0, 0, 0);
        for (_, s) in &self.faults {
            match s {
                Status::Detected(_) => c.0 += 1,
                Status::Untestable(_) => c.1 += 1,
                Status::Aborted => c.2 += 1,
            }
        }
        c
    }
}

/// The literals a test observes: the outputs, then each latch's next value.
fn observed(aig: &Aig) -> Vec<AigLit> {
    aig.outputs().iter().map(|p| p.lit).chain(aig.latches().iter().map(|l| l.next)).collect()
}

/// Whether `test` detects `fault`: an observed value differs between the circuit and the circuit with the fault.
pub fn detects(aig: &Aig, test: &Test, fault: Fault) -> bool {
    if test.inputs.len() != aig.inputs().len() || test.latches.len() != aig.latches().len() {
        return false;
    }
    let good = aig.eval(&test.inputs, &test.latches);
    let mut bad = good.clone();
    bad[fault.node as usize] = fault.stuck;
    for n in fault.node + 1..aig.num_nodes() as u32 {
        if let Node::And(a, b) = aig.node(n) {
            bad[n as usize] = Aig::value(&bad, a) && Aig::value(&bad, b);
        }
    }
    observed(aig).iter().any(|&o| Aig::value(&good, o) != Aig::value(&bad, o))
}

/// The formula whose models are the tests of `fault`, over its variables: node `k` of the circuit the variable `k`
/// (each input and latch among them), then a variable for each node the fault reaches holding its value under the
/// fault, then one per observed value the fault reaches, true when it differs; the clause that one of those does.
pub fn miter(aig: &Aig, fault: Fault) -> (usize, Vec<Vec<Lit>>) {
    let m = Miter::new(aig, fault);
    (m.num_vars, m.clauses)
}

/// [`miter`]'s formula and its shape: the gates it defines (a variable and its two fan-ins, the circuit's and the
/// faulty copy's), and which of its clauses are no gate's, those a justifying search makes true itself (that an
/// observed value differs, and what differing means).
struct Miter {
    num_vars: usize,
    clauses: Vec<Vec<Lit>>,
    gates: Vec<(Var, Lit, Lit)>,
    sides: Vec<usize>,
}

impl Miter {
    /// A solver holding the formula, clause by clause in its order, deciding by justification.
    fn solver(&self) -> Solver {
        let mut s = Solver::new(self.num_vars);
        let mut sides = self.sides.iter().peekable();
        for (k, c) in self.clauses.iter().enumerate() {
            if sides.next_if_eq(&&k).is_some() {
                s.add_side_clause(c.clone());
            } else {
                s.add_clause(c.clone());
            }
        }
        for &(g, a, b) in &self.gates {
            s.set_gate(g as usize, a, b);
        }
        s.set_justifying(true);
        s
    }

    /// The formula for `fault`, built as [`miter`] describes.
    fn new(aig: &Aig, fault: Fault) -> Miter {
        let n = aig.num_nodes();
        let mut reached = vec![false; n];
        reached[fault.node as usize] = true;
        for m in fault.node as usize + 1..n {
            if let Node::And(a, b) = aig.node(m as u32) {
                reached[m] = reached[a.node() as usize] || reached[b.node() as usize];
            }
        }
        let outs: Vec<AigLit> = observed(aig).into_iter().filter(|o| reached[o.node() as usize]).collect();
        let cone = aig.cone(&outs);
        let mut clauses: Vec<Vec<Lit>> = vec![vec![Lit::neg(0)]];
        let (mut gates, mut sides) = (Vec::new(), Vec::new());
        let good = |l: AigLit| Lit::new(l.node() as Var, !l.is_complemented());
        for &m in &cone {
            if let Node::And(a, b) = aig.node(m) {
                let g = Lit::pos(m as Var);
                let (x, y) = (good(a), good(b));
                clauses.extend([vec![g.negated(), x], vec![g.negated(), y], vec![g, x.negated(), y.negated()]]);
                gates.push((m as Var, x, y));
            }
        }
        let mut faulty: Vec<Option<Var>> = vec![None; n];
        let mut num_vars = n;
        for &m in &cone {
            if reached[m as usize] {
                faulty[m as usize] = Some(num_vars as Var);
                num_vars += 1;
            }
        }
        let bad = |l: AigLit| match faulty[l.node() as usize] {
            Some(v) => Lit::new(v, !l.is_complemented()),
            None => good(l),
        };
        for &m in &cone {
            let Some(v) = faulty[m as usize] else { continue };
            let f = Lit::pos(v);
            if m == fault.node {
                clauses.push(vec![if fault.stuck { f } else { f.negated() }]);
            } else if let Node::And(a, b) = aig.node(m) {
                let (x, y) = (bad(a), bad(b));
                clauses.extend([vec![f.negated(), x], vec![f.negated(), y], vec![f, x.negated(), y.negated()]]);
                gates.push((v, x, y));
            }
        }
        let mut differs = Vec::new();
        for &o in &outs {
            let d = Lit::pos(num_vars as Var);
            num_vars += 1;
            let (x, y) = (good(o), bad(o));
            sides.extend([clauses.len(), clauses.len() + 1]);
            clauses.extend([vec![d.negated(), x, y], vec![d.negated(), x.negated(), y.negated()]]);
            differs.push(d);
        }
        sides.push(clauses.len());
        clauses.push(differs);
        Miter { num_vars, clauses, gates, sides }
    }
}

/// Generate tests for every single stuck-at fault of `aig`.
pub fn generate(aig: &Aig, opts: &Options) -> Report {
    let faults: Vec<Fault> =
        (1..aig.num_nodes() as u32).flat_map(|node| [false, true].map(|stuck| Fault { node, stuck })).collect();
    let mut status: Vec<Option<Status>> = vec![None; faults.len()];
    let mut tests: Vec<Test> = Vec::new();
    let mut state = opts.seed;
    let mut next = || {
        state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    };
    let (ni, nl) = (aig.inputs().len(), aig.latches().len());
    let mut faulting = Faulting::new(aig);
    for _ in 0..opts.random_words {
        let inputs: Vec<u64> = (0..ni).map(|_| next()).collect();
        let latches: Vec<u64> = (0..nl).map(|_| next()).collect();
        let good = aig.simulate(1, &inputs, &latches);
        let mut test_of_bit: [Option<usize>; 64] = [None; 64];
        for k in 0..faults.len() {
            if status[k].is_some() {
                continue;
            }
            let bits = faulting.detecting(aig, &good, faults[k]);
            if bits == 0 {
                continue;
            }
            let bit = bits.trailing_zeros() as usize;
            let t = *test_of_bit[bit].get_or_insert_with(|| {
                let pick = |words: &[u64]| words.iter().map(|w| w >> bit & 1 == 1).collect::<Vec<bool>>();
                tests.push(Test { inputs: pick(&inputs), latches: pick(&latches) });
                tests.len() - 1
            });
            status[k] = Some(Status::Detected(t));
        }
    }
    let by_random = status.iter().filter(|s| s.is_some()).count();
    let mut solves = 0;
    for k in 0..faults.len() {
        if status[k].is_some() {
            continue;
        }
        let formula = Miter::new(aig, faults[k]);
        let mut s = formula.solver();
        let (num_vars, clauses) = (formula.num_vars, formula.clauses);
        solves += 1;
        status[k] = Some(match s.solve_budgeted(opts.budget) {
            BudgetedResult::Sat(_) => {
                let mut fill = |node: u32| s.value(node as usize).unwrap_or_else(|| next() & 1 == 1);
                let test = Test {
                    inputs: aig.inputs().iter().map(|p| fill(p.lit.node())).collect(),
                    latches: (0..nl).map(|j| fill(aig.latch_lit(j).node())).collect(),
                };
                if !detects(aig, &test, faults[k]) {
                    Status::Aborted
                } else {
                    let spread =
                        |values: &[bool]| values.iter().map(|&v| if v { !0u64 } else { 0 }).collect::<Vec<u64>>();
                    let good = aig.simulate(1, &spread(&test.inputs), &spread(&test.latches));
                    tests.push(test);
                    let t = tests.len() - 1;
                    for j in k + 1..faults.len() {
                        if status[j].is_none() && faulting.detecting(aig, &good, faults[j]) != 0 {
                            status[j] = Some(Status::Detected(t));
                        }
                    }
                    Status::Detected(t)
                }
            }
            BudgetedResult::Unsat => {
                let learned: Vec<Vec<Lit>> = s.learned().iter().map(|c| c.lits.clone()).collect();
                match lrat::refutation(num_vars, &clauses, &learned) {
                    Some(proof) if lrat::check(num_vars, &clauses, &proof) => Status::Untestable(proof),
                    _ => Status::Aborted,
                }
            }
            BudgetedResult::Budget => Status::Aborted,
        });
    }
    let faults = faults.into_iter().zip(status.into_iter().map(|s| s.expect("every fault decided"))).collect();
    Report { faults, tests, by_random, solves }
}

/// What fault simulation reads of the circuit: each node's fanout, and whether a test observes it.
struct Faulting {
    fanout: Vec<Vec<u32>>,
    observed: Vec<bool>,
    /// Per node, the last simulation that changed its value and that value, and the last that queued it; the current
    /// simulation's number.
    met: Vec<u64>,
    value: Vec<u64>,
    queued: Vec<u64>,
    epoch: u64,
}

impl Faulting {
    fn new(aig: &Aig) -> Faulting {
        let mut fanout: Vec<Vec<u32>> = vec![Vec::new(); aig.num_nodes()];
        for n in 0..aig.num_nodes() as u32 {
            if let Node::And(a, b) = aig.node(n) {
                fanout[a.node() as usize].push(n);
                if b.node() != a.node() {
                    fanout[b.node() as usize].push(n);
                }
            }
        }
        let mut seen = vec![false; aig.num_nodes()];
        for o in observed(aig) {
            seen[o.node() as usize] = true;
        }
        let n = aig.num_nodes();
        Faulting { fanout, observed: seen, met: vec![0; n], value: vec![0; n], queued: vec![0; n], epoch: 0 }
    }

    /// The patterns of a word that detect fault `f`, given the circuit's values `good` on them: the fault simulated
    /// from its node forward, in node order, only where a value differs from the circuit's.
    fn detecting(&mut self, aig: &Aig, good: &[u64], f: Fault) -> u64 {
        let forced = if f.stuck { !0u64 } else { 0 };
        if good[f.node as usize] == forced {
            return 0;
        }
        self.epoch += 1;
        let epoch = self.epoch;
        let read = |l: AigLit, value: &[u64], met: &[u64]| {
            let v = if met[l.node() as usize] == epoch { value[l.node() as usize] } else { good[l.node() as usize] };
            v ^ if l.is_complemented() { !0 } else { 0 }
        };
        let mut queue = std::collections::BinaryHeap::from([std::cmp::Reverse(f.node)]);
        let mut detected = 0u64;
        self.queued[f.node as usize] = epoch;
        while let Some(std::cmp::Reverse(m)) = queue.pop() {
            let v = if m == f.node {
                forced
            } else {
                let Node::And(a, b) = aig.node(m) else { continue };
                read(a, &self.value, &self.met) & read(b, &self.value, &self.met)
            };
            if v == good[m as usize] {
                continue;
            }
            self.met[m as usize] = epoch;
            self.value[m as usize] = v;
            if self.observed[m as usize] {
                detected |= v ^ good[m as usize];
            }
            for &g in &self.fanout[m as usize] {
                if self.queued[g as usize] != epoch {
                    self.queued[g as usize] = epoch;
                    queue.push(std::cmp::Reverse(g));
                }
            }
        }
        detected
    }
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_proof/atpg/tests.rs"]
mod tests;
