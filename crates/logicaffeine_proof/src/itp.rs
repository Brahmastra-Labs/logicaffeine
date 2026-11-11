//! Unbounded safety by interpolation (McMillan, *Interpolation and SAT-based model checking*, CAV 2003), over
//! and-inverter graphs, with no solver but this crate's own.
//!
//! A circuit ([`crate::aig::Aig`]) is safe when no run from an initial state reaches a state where a bad-state
//! property holds; every state of a run, the bad one included, satisfies the invariant constraints. (A circuit with
//! no bad-state properties has its outputs read as them, the older AIGER convention.) The procedure keeps a set `R`
//! of states, starting at the initial ones, and asks whether `R`, one step, and then a run of `k − 1` more steps can
//! end in a bad state:
//!
//! ```text
//! A = R(s₀) ∧ C(s₀) ∧ T(s₀, s₁)          B = ⋁_{j=1..k} ( T(s₁ … s_j) ∧ C(s₁) ∧ … ∧ C(s_j) ∧ Bad(s_j) )
//! ```
//!
//! When `A ∧ B` is unsatisfiable, the interpolant ([`crate::interpolate`]) is a set of states over `s₁` containing
//! every successor of `R` and none that reaches a bad state within `k − 1` steps. If it lies inside `R`, then `R`
//! contains the initial states, is closed under a step, and holds no bad state: an inductive invariant, and the
//! circuit is safe. Otherwise `R` grows by it and the question is asked again. When `A ∧ B` is satisfiable and `R` is
//! still the initial states, the model is a run to a bad state; when `R` has grown, the run may start outside the
//! reachable states, and `k` increases with `R` reset.
//!
//! Neither verdict is taken on the procedure's word. A counterexample is a [`Trace`] that [`replays`] by simulation of
//! the circuit; an invariant is accepted only once [`check_invariant`] has established its three obligations, each by
//! CDCL solves whose LRAT proof an independent checker ([`crate::lrat::check`], [`crate::lrat::check_derivation`])
//! replays. A step that does not check makes the verdict [`ItpVerdict::Unknown`], never a wrong one.
//!
//! [`bmc`] looks for a shortest run to a bad state within a bound, by bounded model checking in one incremental solver:
//! the circuit unrolled only as far as its constraints and bad states read it, each clock's logic encoded by a cover
//! of its cones with cuts of up to six leaves (Eén, Mishchenko & Sörensson, *Applying logic synthesis for speeding up
//! SAT*, SAT 2007), a variable for each cut, with constants folded and every instance of one function over the same
//! variables one variable across all clocks.

use std::collections::HashMap;

use crate::aig::{Aig, AigLit, Init, Leaf, Node};
use crate::aig_cnf::{self, Instances, Mapping, Signal};
use crate::cdcl::{BudgetedResult, Lit, SolveResult, Solver, Var};
use crate::interpolate::{encode_cone, encode_cones, interpolant};

/// Bounds on the search.
#[derive(Clone, Copy, Debug)]
pub struct ItpOptions {
    /// The largest unrolling depth `k` tried.
    pub max_depth: usize,
    /// The most interpolation rounds at one depth before the next depth is tried.
    pub max_rounds: usize,
}

impl Default for ItpOptions {
    fn default() -> ItpOptions {
        ItpOptions { max_depth: 64, max_rounds: 256 }
    }
}

/// A set of states: a circuit over one input per latch, in latch order.
#[derive(Clone, Debug)]
pub struct Invariant {
    /// The circuit. Its input `i` is latch `i`'s value; it has no latches of its own.
    pub aig: Aig,
    /// The literal of the circuit that holds exactly on the set's states.
    pub root: AigLit,
}

impl Invariant {
    /// Whether the state whose latch values are `latches` lies in the set.
    pub fn holds(&self, latches: &[bool]) -> bool {
        let values = self.aig.eval(latches, &[]);
        Aig::value(&values, self.root)
    }
}

/// A run from an initial state to a bad state: the latches' values at the start, then each clock's inputs, the last
/// clock's state bad.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Trace {
    /// Each latch's value at the first clock, in latch order.
    pub initial: Vec<bool>,
    /// Each clock's input values, in input order; at least one clock.
    pub inputs: Vec<Vec<bool>>,
}

/// What [`prove_safe`] found.
#[derive(Clone, Debug)]
pub enum ItpVerdict {
    /// No reachable state is bad; the invariant passed [`check_invariant`].
    Safe(Invariant),
    /// A bad state is reachable; the trace passed [`replays`].
    Unsafe(Trace),
    /// Neither, within the bounds, or a step did not check.
    Unknown,
}

/// The bad-state literals: the bad-state properties, or the outputs of a circuit that has none.
fn bad_lits(aig: &Aig) -> Vec<AigLit> {
    let ports = if aig.bad().is_empty() { aig.outputs() } else { aig.bad() };
    ports.iter().map(|p| p.lit).collect()
}

/// A literal of the unrolling: a constant, or a variable's literal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Sig {
    Const(bool),
    Lit(Lit),
}

impl Sig {
    fn with_phase(self, flip: bool) -> Sig {
        match self {
            Sig::Const(b) => Sig::Const(b ^ flip),
            Sig::Lit(l) => Sig::Lit(if flip { l.negated() } else { l }),
        }
    }
}

/// The circuit unrolled clock by clock into CNF variables: per clock its latches', inputs' and gates' variables.
struct Unrolling<'a> {
    aig: &'a Aig,
    next_var: usize,
    latch: Vec<Vec<Lit>>,
    input: Vec<Vec<Lit>>,
    gate: Vec<HashMap<u32, Lit>>,
}

impl<'a> Unrolling<'a> {
    fn new(aig: &'a Aig) -> Unrolling<'a> {
        Unrolling { aig, next_var: 0, latch: Vec::new(), input: Vec::new(), gate: Vec::new() }
    }

    fn fresh(&mut self) -> Lit {
        let v = self.next_var as Var;
        self.next_var += 1;
        Lit::pos(v)
    }

    /// The next clock's latch and input variables.
    fn add_frame(&mut self) {
        let latches = (0..self.aig.latches().len()).map(|_| self.fresh()).collect();
        let inputs = (0..self.aig.inputs().len()).map(|_| self.fresh()).collect();
        self.latch.push(latches);
        self.input.push(inputs);
        self.gate.push(HashMap::new());
    }

    /// Clock `t`'s and-gates, each a fresh variable defined by its Tseitin clauses into `out`.
    fn gates(&mut self, t: usize, out: &mut Vec<Vec<Lit>>) {
        for n in 0..self.aig.num_nodes() as u32 {
            self.gate(t, n, out);
        }
    }

    /// Clock `t`'s and-gates in the cones of `roots`, as [`Unrolling::gates`] encodes them all.
    fn gates_of(&mut self, t: usize, roots: &[AigLit], out: &mut Vec<Vec<Lit>>) {
        for n in self.aig.cone(roots) {
            self.gate(t, n, out);
        }
    }

    /// Node `n` at clock `t`, when it is an and-gate whose fan-ins are numbered: a fresh variable defined by its
    /// Tseitin clauses into `out`.
    fn gate(&mut self, t: usize, n: u32, out: &mut Vec<Vec<Lit>>) {
        let Node::And(x, y) = self.aig.node(n) else { return };
        let g = self.fresh();
        self.gate[t].insert(n, g);
        match (self.sig(t, x), self.sig(t, y)) {
            (Sig::Lit(lx), Sig::Lit(ly)) => {
                out.push(vec![g.negated(), lx]);
                out.push(vec![g.negated(), ly]);
                out.push(vec![g, lx.negated(), ly.negated()]);
            }
            (Sig::Const(false), _) | (_, Sig::Const(false)) => out.push(vec![g.negated()]),
            (Sig::Const(true), Sig::Const(true)) => out.push(vec![g]),
            (Sig::Const(true), Sig::Lit(l)) | (Sig::Lit(l), Sig::Const(true)) => {
                out.push(vec![g.negated(), l]);
                out.push(vec![g, l.negated()]);
            }
        }
    }

    /// A literal of the circuit at clock `t` (its gates already numbered).
    fn sig(&self, t: usize, l: AigLit) -> Sig {
        let base = match self.aig.node(l.node()) {
            Node::False => Sig::Const(false),
            Node::Input(i) => Sig::Lit(self.input[t][i]),
            Node::Latch(k) => Sig::Lit(self.latch[t][k]),
            Node::And(..) => Sig::Lit(self.gate[t][&l.node()]),
        };
        base.with_phase(l.is_complemented())
    }

    /// Clock `t + 1`'s latches equal clock `t`'s next-state literals.
    fn transition(&self, t: usize, out: &mut Vec<Vec<Lit>>) {
        let all: Vec<usize> = (0..self.aig.latches().len()).collect();
        self.transition_of(t, &all, out);
    }

    /// Clock `t + 1`'s latches `which` equal clock `t`'s next-state literals.
    fn transition_of(&self, t: usize, which: &[usize], out: &mut Vec<Vec<Lit>>) {
        for &k in which {
            let s = self.latch[t + 1][k];
            match self.sig(t, self.aig.latches()[k].next) {
                Sig::Const(b) => out.push(vec![if b { s } else { s.negated() }]),
                Sig::Lit(l) => {
                    out.push(vec![s.negated(), l]);
                    out.push(vec![s, l.negated()]);
                }
            }
        }
    }

    /// Clock 0's latches at their initial values.
    fn init(&self, out: &mut Vec<Vec<Lit>>) {
        for (k, latch) in self.aig.latches().iter().enumerate() {
            match latch.init {
                Init::Zero => out.push(vec![self.latch[0][k].negated()]),
                Init::One => out.push(vec![self.latch[0][k]]),
                Init::Free => {}
            }
        }
    }

    /// Every invariant constraint at clock `t`, asserted.
    fn constraints(&self, t: usize, out: &mut Vec<Vec<Lit>>) {
        for c in self.aig.constraints() {
            match self.sig(t, c.lit) {
                Sig::Const(true) => {}
                Sig::Const(false) => out.push(Vec::new()),
                Sig::Lit(l) => out.push(vec![l]),
            }
        }
    }

    /// A literal true only if some bad-state property holds at clock `t` (with its defining clauses), or a constant.
    fn bad(&mut self, t: usize, out: &mut Vec<Vec<Lit>>) -> Sig {
        let sigs: Vec<Sig> = bad_lits(self.aig).into_iter().map(|b| self.sig(t, b)).collect();
        if sigs.contains(&Sig::Const(true)) {
            return Sig::Const(true);
        }
        let lits: Vec<Lit> = sigs.into_iter().filter_map(|s| if let Sig::Lit(l) = s { Some(l) } else { None }).collect();
        if lits.is_empty() {
            return Sig::Const(false);
        }
        let any = self.fresh();
        let mut def = vec![any.negated()];
        def.extend(&lits);
        out.push(def);
        Sig::Lit(any)
    }

    /// A set of states over clock `t`'s latches, Tseitin-encoded into `out`; the literal that holds in it.
    fn region(&mut self, t: usize, region: &Invariant, out: &mut Vec<Vec<Lit>>) -> Lit {
        let latches = self.latch[t].clone();
        encode_cone(&region.aig, region.root, &mut |i| latches[i], out, &mut self.next_var)
    }

    /// Literals `roots` of a set's circuit over clock `t`'s latches, Tseitin-encoded into `out` together.
    fn region_roots(&mut self, t: usize, region: &Invariant, roots: &[AigLit], out: &mut Vec<Vec<Lit>>) -> Vec<Lit> {
        let latches = self.latch[t].clone();
        encode_cones(&region.aig, roots, &mut |i| latches[i], out, &mut self.next_var)
    }
}

/// The initial states as a set over the latches.
fn initial_region(aig: &Aig) -> Invariant {
    let mut r = Aig::new();
    let inputs: Vec<AigLit> = (0..aig.latches().len()).map(|_| r.add_input(None)).collect();
    let mut root = AigLit::TRUE;
    for (k, latch) in aig.latches().iter().enumerate() {
        let lit = match latch.init {
            Init::Zero => !inputs[k],
            Init::One => inputs[k],
            Init::Free => continue,
        };
        root = r.and(root, lit);
    }
    Invariant { aig: r, root }
}

/// `B` of the query at depth `k`: clocks `1..=k` gated so that some `j` has constraints holding on `1..=j` and a bad
/// state at `j`.
fn bad_within(u: &mut Unrolling, k: usize, out: &mut Vec<Vec<Lit>>) {
    let mut reach: Vec<Lit> = Vec::new();
    for j in 1..=k {
        let bad = u.bad(j, out);
        let b = u.fresh();
        match bad {
            Sig::Const(false) => out.push(vec![b.negated()]),
            Sig::Const(true) => {}
            Sig::Lit(l) => out.push(vec![b.negated(), l]),
        }
        for t in 1..=j {
            let mut cs = Vec::new();
            u.constraints(t, &mut cs);
            for c in cs {
                let mut gated = vec![b.negated()];
                gated.extend(c);
                out.push(gated);
            }
        }
        reach.push(b);
    }
    out.push(reach);
}

/// Refute `clauses` with a checked LRAT proof: solve, trim the learned clauses to an LRAT refutation, and replay it
/// with [`crate::lrat::check`]. `Ok(())` when refuted and checked, `Err(Some(model))` when satisfiable, `Err(None)`
/// when the solver's refutation does not check.
fn refute(num_vars: usize, clauses: &[Vec<Lit>]) -> Result<crate::lrat::Lrat, Option<Vec<bool>>> {
    let mut s = Solver::new(num_vars);
    for c in clauses {
        s.add_clause(c.clone());
    }
    match s.solve() {
        SolveResult::Sat(model) => Err(Some(model)),
        SolveResult::Unsat => {
            let learned: Vec<Vec<Lit>> = s.learned().iter().map(|c| c.lits.clone()).collect();
            match crate::lrat::refutation(num_vars, clauses, &learned) {
                Some(p) if crate::lrat::check(num_vars, clauses, &p) => Ok(p),
                _ => Err(None),
            }
        }
    }
}

/// Is a bad state reachable in zero steps? The initial states satisfying the constraints, at a bad state.
fn bad_initially(aig: &Aig) -> Result<Option<Trace>, ()> {
    let mut u = Unrolling::new(aig);
    u.add_frame();
    let mut f = Vec::new();
    u.gates(0, &mut f);
    u.init(&mut f);
    u.constraints(0, &mut f);
    match u.bad(0, &mut f) {
        Sig::Const(false) => return Ok(None),
        Sig::Const(true) => {}
        Sig::Lit(l) => f.push(vec![l]),
    }
    match refute(u.next_var, &f) {
        Ok(_) => Ok(None),
        Err(Some(model)) => Ok(Some(trace_of(&u, &model, 0))),
        Err(None) => Err(()),
    }
}

/// The run a model of an unrolling describes, through clock `last`.
fn trace_of(u: &Unrolling, model: &[bool], last: usize) -> Trace {
    let value = |l: Lit| model[l.var() as usize] == l.is_positive();
    Trace {
        initial: u.latch[0].iter().map(|&l| value(l)).collect(),
        inputs: (0..=last).map(|t| u.input[t].iter().map(|&l| value(l)).collect()).collect(),
    }
}

/// A shortest run of at most `clocks` clocks from an initial state to a bad state, by bounded model checking: the
/// circuit unrolled a clock at a time into one incremental solver as far as its questions read it, the constraints
/// asserted at every clock, and each clock's bad states asked for under an assumption, so the first clock that answers
/// ends the shortest run (see the module documentation for the encoding). A run is returned only once it [`replays`];
/// `None` when there is none that short, or the one found does not replay.
pub fn bmc(aig: &Aig, clocks: usize) -> Option<Trace> {
    match Bmc::new(aig).resume(clocks) {
        Some(Ok(trace)) => Some(trace),
        _ => None,
    }
}

/// A run of at most `clocks` clocks from an initial state to a bad state, by bounded model checking that asks `window`
/// clocks a query (Biere, Cimatti, Clarke & Zhu, TACAS 1999, the window rIC3's `bmc --step` asks): one solve asks
/// whether a run is bad at any clock of the window, the constraints holding up to its bad clock, so a deep run is found
/// without refuting each clock before it on its own. The run ends at the first clock the model is bad at, which need not
/// be the shortest run; it is returned only once it [`replays`]; `None` when there is none that short.
pub fn bmc_windowed(aig: &Aig, clocks: usize, window: usize) -> Option<Trace> {
    let mut bmc = Bmc::new(aig);
    while bmc.clocks < clocks {
        match bmc.window(window.min(clocks - bmc.clocks), 0) {
            Answer::Run(trace) => return Some(trace),
            Answer::Ended => return None,
            Answer::Refuted(_) => {}
            Answer::Budget => unreachable!("a solve without a budget runs to a verdict"),
        }
    }
    None
}

/// A run of at most `clocks` clocks from an initial state to a bad state, by two bounded model checks taken in turns,
/// each turn within a conflict budget that doubles every round from [`INTERLEAVE_FIRST`]: one asking a clock at a time
/// ([`bmc`]), which finds a shallow run as short as it is, and one asking a window of clocks a query
/// ([`bmc_windowed`]) of [`INTERLEAVE_WINDOW`] clocks, which finds a deep run without refuting each clock before it on
/// its own. A cone whose window would unroll more than [`INTERLEAVE_NODES`] nodes of the cover is asked a clock at a
/// time alone, a window's unrolling there outweighing the clocks it saves. The first to answer answers, and which one that is depends on the conflicts each spent, never on timing; either one
/// refuting every clock up to the bound answers that there is none. A run is returned only once it [`replays`].
pub fn bmc_interleaved(aig: &Aig, clocks: usize) -> Option<Trace> {
    let mut single = Bmc::new(aig);
    let window = INTERLEAVE_WINDOW;
    if single.slots.saturating_mul(window) > INTERLEAVE_NODES {
        return bmc(aig, clocks);
    }
    let mut windowed = Bmc::new(aig);
    let mut budget = INTERLEAVE_FIRST;
    loop {
        for (bmc, w) in [(&mut single, 1), (&mut windowed, window)] {
            let end = bmc.solver.conflicts() + budget;
            loop {
                if bmc.clocks >= clocks {
                    return None;
                }
                let left = end.saturating_sub(bmc.solver.conflicts());
                if left == 0 {
                    break;
                }
                let answer = if w == 1 { bmc.ask(left) } else { bmc.window(w.min(clocks - bmc.clocks), left) };
                match answer {
                    Answer::Run(trace) => return Some(trace),
                    Answer::Ended => return None,
                    Answer::Refuted(_) => {}
                    Answer::Budget => break,
                }
            }
        }
        budget = budget.saturating_mul(2);
    }
}

/// The conflicts each bounded model check of [`bmc_interleaved`] may spend in its first turn.
pub const INTERLEAVE_FIRST: u64 = 1000;

/// The clocks a window of [`bmc_interleaved`] asks, and the most nodes of the cover its unrolling may make.
pub const INTERLEAVE_WINDOW: usize = 64;
pub const INTERLEAVE_NODES: usize = 1_000_000;

/// The most leaves a cut of the cover bounded model checking unrolls has ([`aig_cnf::map`]).
pub(crate) const CUT_LEAVES: usize = 6;

/// Bounded model checking that runs in slices ([`Bmc::resume`]): the unrolling and its solver kept between them, so a
/// search resumed clock after clock is the search run whole.
///
/// The circuit is unrolled as its questions read it, over the cover of the cones of its bad states, its constraints
/// and the next states of the latches they reach by cuts ([`aig_cnf::map`]). A node is made at a clock once a
/// constraint or a bad state asked about reads it, through any number of clocks back: a latch at clock `t + 1` is its
/// next-state signal at clock `t`, one at clock 0 its initial value (a variable for one free to start anywhere), an
/// input a variable, and a cut's root the instance of the cut's function over its leaves' signals at that clock
/// ([`Instances::instance`]): a constant or a leaf's signal when they decide it, and otherwise the variable one
/// function over those variables has at every clock. The constraints and the disjunction of the bad states are clauses
/// beside the instances', and a clock whose bad states are refuted keeps the refutation as a clause.
pub(crate) struct Bmc<'a> {
    aig: &'a Aig,
    solver: Solver,
    mapping: Mapping,
    instances: Instances,
    /// Per node of the cover (the constant, an input, a latch, or a cut's root), its place among a clock's `slots`
    /// signals.
    slot: Vec<u32>,
    slots: usize,
    made: Vec<Vec<Signal>>,
    /// Per latch, when the unrolling is tied ([`Bmc::tied`]), the literal under which it holds its initial value at
    /// clock 0 and its next-state signal at the clock before after: a latch whose literal is not assumed is free.
    ties: Vec<Lit>,
    /// Per clock unrolled, its bad states, and the literals of its constraints a window unrolled and has not asserted.
    bads: Vec<Bad>,
    pending: Vec<Vec<Lit>>,
    /// The clocks unrolled and asked, and whether no run of any length is left.
    pub(crate) clocks: usize,
    ended: bool,
}

/// What querying the clock under way found ([`Bmc::query`]).
pub(crate) enum Query {
    /// A model under the ties assumed: its run, and whether the run replays on the circuit.
    Model(Trace, bool),
    /// No model: the latches whose ties the refutation rests on.
    Refuted(Vec<usize>),
    /// No run of this length or longer is left: the constraints admit none.
    Ended,
    /// The conflicts given ran out first.
    Budget,
}

/// The bad states of a clock: never, always, or under a literal.
#[derive(Clone, Copy)]
enum Bad {
    Never,
    Always,
    At(Lit),
}

/// What asking a clock found ([`Bmc::ask`]).
pub(crate) enum Answer {
    /// A shortest run, which replays.
    Run(Trace),
    /// No run is bad at this clock: in a tied unrolling, the latches whose ties the refutation rests on.
    Refuted(Vec<usize>),
    /// No run of this length or longer is left (the constraints admit none), or the one found did not replay.
    Ended,
    /// The conflicts given ran out first.
    Budget,
}

impl<'a> Bmc<'a> {
    pub(crate) fn new(aig: &'a Aig) -> Bmc<'a> {
        Bmc::with_cuts(aig, CUT_LEAVES)
    }

    /// The search over the cover by cuts of at most `k` leaves.
    pub(crate) fn with_cuts(aig: &'a Aig, k: usize) -> Bmc<'a> {
        let mut roots: Vec<AigLit> = bad_lits(aig).into_iter().chain(aig.constraints().iter().map(|c| c.lit)).collect();
        let mut seen = vec![false; aig.num_nodes()];
        let mut stack: Vec<u32> = roots.iter().map(|l| l.node()).collect();
        while let Some(n) = stack.pop() {
            if std::mem::replace(&mut seen[n as usize], true) {
                continue;
            }
            match aig.node(n) {
                Node::And(a, b) => stack.extend([a.node(), b.node()]),
                Node::Latch(j) => {
                    let next = aig.latches()[j].next;
                    roots.push(next);
                    stack.push(next.node());
                }
                Node::False | Node::Input(_) => {}
            }
        }
        let mapping = aig_cnf::map(aig, &roots, k);
        let mut slot = vec![u32::MAX; aig.num_nodes()];
        let mut slots = 0;
        for n in 0..aig.num_nodes() as u32 {
            if !matches!(aig.node(n), Node::And(..)) || mapping.cut(n).is_some() {
                slot[n as usize] = slots as u32;
                slots += 1;
            }
        }
        Bmc {
            aig,
            solver: Solver::new(0),
            mapping,
            instances: Instances::default(),
            slot,
            slots,
            made: Vec::new(),
            ties: Vec::new(),
            bads: Vec::new(),
            pending: Vec::new(),
            clocks: 0,
            ended: false,
        }
    }

    /// The search over the cover by cuts of at most `k` leaves with every latch tied by a literal of its own, each asked
    /// clock assuming every tie: a refuted clock names the latches its refutation used, the latches a localization
    /// abstraction keeps (McMillan & Amla, *Automatic abstraction without counterexamples*, TACAS 2003).
    pub(crate) fn tied(aig: &'a Aig, k: usize) -> Bmc<'a> {
        let mut bmc = Bmc::with_cuts(aig, k);
        let n = aig.latches().len();
        let first = bmc.solver.add_vars(n);
        bmc.ties = (0..n).map(|j| Lit::pos((first + j) as Var)).collect();
        bmc
    }

    fn fresh(&mut self) -> Signal {
        Signal::lit(Lit::pos(self.solver.add_vars(1) as Var))
    }

    fn at(&self, c: usize, n: u32) -> Signal {
        self.made[c][self.slot[n as usize] as usize]
    }

    /// Node `n`'s signal at clock `t`, made now with whatever it reads, at that clock or before, not made yet.
    fn make(&mut self, t: usize, n: u32) -> Signal {
        let mut stack = vec![(t, n)];
        while let Some(&(c, m)) = stack.last() {
            if self.at(c, m) != Signal::UNMADE {
                stack.pop();
                continue;
            }
            let signal = match self.aig.node(m) {
                Node::False => Signal::FALSE,
                Node::Input(_) => self.fresh(),
                Node::Latch(j) if c == 0 => match (self.aig.latches()[j].init, self.ties.get(j).copied()) {
                    (Init::Zero, None) => Signal::FALSE,
                    (Init::One, None) => Signal::TRUE,
                    (Init::Free, _) => self.fresh(),
                    (init, Some(tie)) => {
                        let x = self.fresh();
                        let l = x.as_lit().expect("a fresh signal is a literal");
                        self.solver.add_clause(vec![tie.negated(), if init == Init::One { l } else { l.negated() }]);
                        x
                    }
                },
                Node::Latch(j) => {
                    let next = self.aig.latches()[j].next;
                    let s = match self.at(c - 1, next.node()) {
                        Signal::UNMADE => {
                            stack.push((c - 1, next.node()));
                            continue;
                        }
                        s => s.with_phase(next.is_complemented()),
                    };
                    match self.ties.get(j).copied() {
                        None => s,
                        Some(tie) => {
                            let x = self.fresh();
                            let l = x.as_lit().expect("a fresh signal is a literal");
                            match s.as_lit() {
                                Some(n) => {
                                    self.solver.add_clause(vec![tie.negated(), l.negated(), n]);
                                    self.solver.add_clause(vec![tie.negated(), l, n.negated()]);
                                }
                                None => self.solver.add_clause(vec![tie.negated(), if s == Signal::TRUE { l } else { l.negated() }]),
                            }
                            x
                        }
                    }
                }
                Node::And(..) => {
                    let cut = *self.mapping.cut(m).expect("an And node the unrolling makes roots a cut of the cover");
                    let mut leaves = [Signal::UNMADE; aig_cnf::MAX_LEAVES];
                    let mut open = false;
                    for (s, &leaf) in leaves.iter_mut().zip(cut.leaves()) {
                        *s = self.at(c, leaf);
                        if *s == Signal::UNMADE {
                            stack.push((c, leaf));
                            open = true;
                        }
                    }
                    if open {
                        continue;
                    }
                    self.instances.instance(cut.truth, &leaves[..cut.leaves().len()], &mut self.solver)
                }
            };
            let at = self.slot[m as usize] as usize;
            self.made[c][at] = signal;
            stack.pop();
        }
        self.at(t, n)
    }

    fn sig(&mut self, t: usize, l: AigLit) -> Signal {
        self.make(t, l.node()).with_phase(l.is_complemented())
    }

    /// The run a model describes through clock `last`: a latch the unrolling never made at clock 0 at its initial
    /// value (false for a free one), an input it never made false.
    fn run(&self, model: &[bool], last: usize) -> Trace {
        let value = |s: Signal, unmade: bool| match s.as_lit() {
            Some(l) => model[l.var() as usize] == l.is_positive(),
            None if s == Signal::UNMADE => unmade,
            None => s == Signal::TRUE,
        };
        Trace {
            initial: self.aig.latches().iter().map(|l| value(self.at(0, l.node), l.init == Init::One)).collect(),
            inputs: (0..=last)
                .map(|t| self.aig.inputs().iter().map(|p| value(self.at(t, p.lit.node()), false)).collect())
                .collect(),
        }
    }

    /// Ask the next `clocks` clocks in turn: `Some(Ok(run))` for a shortest run, accepted once it replays;
    /// `Some(Err(()))` once no run of any length is left (the constraints admit no longer one) or the run found does not
    /// replay; `None` when none of these clocks answers.
    pub(crate) fn resume(&mut self, clocks: usize) -> Option<Result<Trace, ()>> {
        if self.ended {
            return Some(Err(()));
        }
        for _ in 0..clocks {
            match self.ask(0) {
                Answer::Run(trace) => return Some(Ok(trace)),
                Answer::Ended => return Some(Err(())),
                Answer::Refuted(_) => {}
                Answer::Budget => unreachable!("a solve without a budget runs to a verdict"),
            }
        }
        None
    }

    /// Unroll and ask the next clock within `budget` conflicts (0: no limit), every tie assumed, and move past it once it
    /// is refuted. A clock left to the budget is asked again by the next call.
    pub(crate) fn ask(&mut self, budget: u64) -> Answer {
        let all = vec![true; self.ties.len()];
        if self.ended {
            return Answer::Ended;
        }
        match self.query(budget, &all) {
            Query::Model(trace, true) => {
                self.ended = true;
                self.clocks += 1;
                Answer::Run(trace)
            }
            Query::Model(_, false) | Query::Ended => {
                self.ended = true;
                self.clocks += 1;
                Answer::Ended
            }
            Query::Refuted(used) => {
                self.advance(&used);
                Answer::Refuted(used)
            }
            Query::Budget => Answer::Budget,
        }
    }

    /// Whether the clock under way has a run bad at it under the ties of the latches `tied` marks (every tie for an
    /// untied unrolling), within `budget` conflicts (0: no limit): the clock stays under way.
    pub(crate) fn query(&mut self, budget: u64, tied: &[bool]) -> Query {
        if self.ended {
            return Query::Ended;
        }
        let t = self.clocks;
        if self.bads.len() == t {
            let Some((bad, held)) = self.unroll(t) else {
                self.ended = true;
                return Query::Ended;
            };
            for l in held {
                self.solver.add_clause(vec![l]);
            }
            self.bads.push(bad);
            self.pending.push(Vec::new());
        }
        for l in std::mem::take(&mut self.pending[t]) {
            self.solver.add_clause(vec![l]);
        }
        let mut assumptions = match self.bads[t] {
            Bad::Never => return Query::Refuted(Vec::new()),
            Bad::Always => Vec::new(),
            Bad::At(a) => vec![a],
        };
        assumptions.extend(self.ties.iter().zip(tied).filter(|(_, &on)| on).map(|(&tie, _)| tie));
        match self.solver.solve_under_assumptions_budgeted(&assumptions, budget) {
            BudgetedResult::Sat(model) => {
                let trace = self.run(&model, t);
                let replays = replays(self.aig, &trace);
                Query::Model(trace, replays)
            }
            BudgetedResult::Unsat if matches!(self.bads[t], Bad::Always) => {
                self.ended = true;
                Query::Ended
            }
            BudgetedResult::Unsat => {
                let core: std::collections::HashSet<Lit> = self.solver.unsat_core().iter().copied().collect();
                Query::Refuted((0..self.ties.len()).filter(|&j| core.contains(&self.ties[j])).collect())
            }
            BudgetedResult::Budget => Query::Budget,
        }
    }

    /// Unroll clock `t`, the next one: its bad states, and the literals of its constraints (one the constant true left
    /// out); `None` when a constraint is the constant false there, so no run reaches the clock.
    fn unroll(&mut self, t: usize) -> Option<(Bad, Vec<Lit>)> {
        self.made.push(vec![Signal::UNMADE; self.slots]);
        let mut held = Vec::new();
        for c in 0..self.aig.constraints().len() {
            match self.sig(t, self.aig.constraints()[c].lit) {
                Signal::TRUE => {}
                Signal::FALSE => return None,
                s => held.push(s.as_lit().expect("a signal neither constant is a literal")),
            }
        }
        let sigs: Vec<Signal> = bad_lits(self.aig).into_iter().map(|b| self.sig(t, b)).collect();
        let lits: Vec<Lit> = sigs.iter().filter_map(|s| s.as_lit()).collect();
        let bad = if sigs.contains(&Signal::TRUE) {
            Bad::Always
        } else if lits.len() > 1 {
            let any = self.fresh().as_lit().expect("a fresh signal is a literal");
            self.solver.add_clause(std::iter::once(any.negated()).chain(lits).collect());
            Bad::At(any)
        } else {
            lits.first().map_or(Bad::Never, |&l| Bad::At(l))
        };
        Some((bad, held))
    }

    /// Ask the next `w` clocks in one query within `budget` conflicts (0: no limit): whether a run is bad at one of
    /// them, the constraints holding at every clock up to that one. A model is the run to the first clock it is bad at,
    /// accepted once it replays, which need not be the shortest run; a refutation moves past every clock asked, the
    /// constraints of each asserted and its bad states refuted, so a run bad at a later clock is asked of the clocks
    /// after them. A window left to the budget leaves every clock asked to be asked again. Untied unrollings only.
    pub(crate) fn window(&mut self, w: usize, budget: u64) -> Answer {
        debug_assert!(self.ties.is_empty(), "a window is asked of an untied unrolling");
        if self.ended {
            return Answer::Ended;
        }
        let from = self.clocks;
        let mut clocks: Vec<(usize, Bad, Vec<Lit>)> = Vec::new();
        let mut last = false;
        for t in from..from + w.max(1) {
            let (bad, held) = if t < self.bads.len() {
                (self.bads[t], self.pending[t].clone())
            } else {
                match self.unroll(t) {
                    Some((bad, held)) => {
                        self.bads.push(bad);
                        self.pending.push(held.clone());
                        (bad, held)
                    }
                    None => {
                        last = true;
                        break;
                    }
                }
            };
            clocks.push((t, bad, held));
        }
        if clocks.is_empty() {
            self.ended = true;
            return Answer::Ended;
        }
        let act = self.fresh().as_lit().expect("a fresh signal is a literal");
        let mut reach: Option<Lit> = None;
        let mut hits = vec![act.negated()];
        let mut reached = Vec::with_capacity(clocks.len());
        for (_, bad, held) in &clocks {
            let r = self.fresh().as_lit().expect("a fresh signal is a literal");
            if let Some(before) = reach {
                self.solver.add_clause(vec![r.negated(), before]);
            }
            for &c in held {
                self.solver.add_clause(vec![r.negated(), c]);
            }
            reach = Some(r);
            reached.push(r);
            match *bad {
                Bad::Never => {}
                Bad::Always => hits.push(r),
                Bad::At(b) => {
                    let hit = self.fresh().as_lit().expect("a fresh signal is a literal");
                    self.solver.add_clause(vec![hit.negated(), b]);
                    self.solver.add_clause(vec![hit.negated(), r]);
                    hits.push(hit);
                }
            }
        }
        self.solver.add_clause(hits);
        let answer = self.solver.solve_under_assumptions_budgeted(&[act], budget);
        self.solver.add_clause(vec![act.negated()]);
        match answer {
            BudgetedResult::Sat(model) => {
                let holds = |l: Lit| model[l.var() as usize] == l.is_positive();
                let first = clocks.iter().zip(&reached).find(|((_, bad, _), &r)| {
                    holds(r) && match *bad {
                        Bad::Never => false,
                        Bad::Always => true,
                        Bad::At(b) => holds(b),
                    }
                });
                let (t, _, _) = first.expect("a model of the window is bad at one of its clocks").0;
                let trace = self.run(&model, *t);
                self.ended = true;
                self.clocks = t + 1;
                if replays(self.aig, &trace) {
                    Answer::Run(trace)
                } else {
                    Answer::Ended
                }
            }
            BudgetedResult::Budget => Answer::Budget,
            BudgetedResult::Unsat => {
                for (t, bad, held) in &clocks {
                    self.pending[*t].clear();
                    for &c in held {
                        self.solver.add_clause(vec![c]);
                    }
                    if let Bad::At(b) = *bad {
                        self.solver.add_clause(vec![b.negated()]);
                    }
                }
                self.clocks = from + clocks.len();
                if last || clocks.iter().any(|(_, bad, _)| matches!(bad, Bad::Always)) {
                    self.ended = true;
                    return Answer::Ended;
                }
                Answer::Refuted(Vec::new())
            }
        }
    }

    /// Move past the clock under way, refuted under the ties of the latches `used`: that refutation kept as a clause.
    pub(crate) fn advance(&mut self, used: &[usize]) {
        if let Bad::At(a) = self.bads[self.clocks] {
            let clause = std::iter::once(a.negated()).chain(used.iter().map(|&j| self.ties[j].negated()));
            self.solver.add_clause(clause.collect());
        }
        self.clocks += 1;
    }
}

/// Decide whether a bad state is reachable, by interpolation. See the module documentation.
pub fn prove_safe(aig: &Aig, opts: &ItpOptions) -> ItpVerdict {
    match bad_initially(aig) {
        Err(()) => return ItpVerdict::Unknown,
        Ok(Some(trace)) => {
            return if replays(aig, &trace) { ItpVerdict::Unsafe(trace) } else { ItpVerdict::Unknown };
        }
        Ok(None) => {}
    }
    if bad_lits(aig).is_empty() {
        let inv = Invariant { aig: initial_region(aig).aig, root: AigLit::TRUE };
        return if check_invariant(aig, &inv) { ItpVerdict::Safe(inv) } else { ItpVerdict::Unknown };
    }
    for k in 1..=opts.max_depth {
        let mut r = initial_region(aig);
        let mut grown = false;
        for _ in 0..opts.max_rounds {
            let mut u = Unrolling::new(aig);
            for _ in 0..=k {
                u.add_frame();
            }
            let mut a = Vec::new();
            let at_r = u.region(0, &r, &mut a);
            a.push(vec![at_r]);
            u.gates(0, &mut a);
            u.constraints(0, &mut a);
            u.transition(0, &mut a);
            let mut b = Vec::new();
            for t in 1..=k {
                u.gates(t, &mut b);
                if t < k {
                    u.transition(t, &mut b);
                }
            }
            bad_within(&mut u, k, &mut b);
            let num_vars = u.next_var;
            let all: Vec<Vec<Lit>> = a.iter().chain(&b).cloned().collect();
            let proof = match refute(num_vars, &all) {
                Ok(proof) => proof,
                Err(Some(model)) => {
                    if grown {
                        break;
                    }
                    let trace = (1..=k)
                        .map(|last| trace_of(&u, &model, last))
                        .find(|t| replays(aig, t));
                    return match trace {
                        Some(t) => ItpVerdict::Unsafe(t),
                        None => ItpVerdict::Unknown,
                    };
                }
                Err(None) => return ItpVerdict::Unknown,
            };
            let Some(itp) = interpolant(num_vars, &a, &b, &proof) else { return ItpVerdict::Unknown };
            let latch_of: HashMap<Var, usize> =
                u.latch[1].iter().enumerate().map(|(k, l)| (l.var(), k)).collect();
            if itp.shared.iter().any(|v| !latch_of.contains_key(v)) {
                return ItpVerdict::Unknown;
            }
            let inputs: Vec<AigLit> = r.aig.inputs().iter().map(|p| p.lit).collect();
            let shared = itp.shared.clone();
            let map = r.aig.import(&itp.aig, &mut |leaf| match leaf {
                Leaf::Input(i) => inputs[latch_of[&shared[i]]],
                Leaf::Latch(_) => AigLit::FALSE,
            });
            let image = map[itp.root.node() as usize].with_phase(itp.root.is_complemented());
            if contained(aig.latches().len(), &r, image) {
                return if check_invariant(aig, &r) { ItpVerdict::Safe(r) } else { ItpVerdict::Unknown };
            }
            r.root = r.aig.or(r.root, image);
            grown = true;
        }
    }
    ItpVerdict::Unknown
}

/// Whether the set `image` (a literal of `r`'s circuit) lies inside `r`: `image ∧ ¬r` refuted.
fn contained(num_latches: usize, r: &Invariant, image: AigLit) -> bool {
    let mut clauses = Vec::new();
    let mut next_var = num_latches;
    let i = encode_cone(&r.aig, image, &mut |k| Lit::pos(k as Var), &mut clauses, &mut next_var);
    let o = encode_cone(&r.aig, r.root, &mut |k| Lit::pos(k as Var), &mut clauses, &mut next_var);
    clauses.push(vec![i]);
    clauses.push(vec![o.negated()]);
    refute(next_var, &clauses).is_ok()
}

/// The literals whose conjunction `root` is in `aig`: through every And node read uncomplemented down to what is not
/// one, each once; none for the constant true.
fn conjuncts(aig: &Aig, root: AigLit) -> Vec<AigLit> {
    let mut parts = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut stack = vec![root];
    while let Some(l) = stack.pop() {
        if l == AigLit::TRUE || !seen.insert(l) {
            continue;
        }
        match aig.node(l.node()) {
            Node::And(a, b) if !l.is_complemented() => {
                stack.push(b);
                stack.push(a);
            }
            _ => parts.push(l),
        }
    }
    parts
}

/// Whether `clauses` imply `goal`, by a checked LRAT derivation of it as a unit. One solver is asked to refute `¬goal`
/// within `budget` conflicts (none with a budget of 0); past that, `¬p` for each of `parts` in turn, literals whose
/// conjunction the clauses make `goal`, each entered as a unit once refuted, and then `goal`, which they make RUP. The
/// clauses the solver learns are the log the derivation is trimmed from, and the derivation must check whatever the
/// parts were: parts that do not make up the goal leave it underived.
fn implies(num_vars: usize, clauses: &[Vec<Lit>], goal: Lit, parts: &[Lit], budget: u64) -> bool {
    let mut s = Solver::new(num_vars);
    for c in clauses {
        s.add_clause(c.clone());
    }
    let mut log: Vec<Vec<Lit>> = Vec::new();
    let whole = if parts.len() <= 1 {
        s.solve_under_assumptions_budgeted(&[goal.negated()], 0)
    } else if budget > 0 {
        s.solve_under_assumptions_budgeted(&[goal.negated()], budget)
    } else {
        BudgetedResult::Budget
    };
    match whole {
        BudgetedResult::Unsat => {}
        BudgetedResult::Sat(_) => return false,
        BudgetedResult::Budget => {
            for &p in parts {
                if !matches!(s.solve_under_assumptions_budgeted(&[p.negated()], 0), BudgetedResult::Unsat) {
                    return false;
                }
                log.extend(s.take_learned().into_iter().map(|c| c.lits));
                log.push(vec![p]);
                s.add_clause(vec![p]);
            }
        }
    }
    log.extend(s.take_learned().into_iter().map(|c| c.lits));
    log.push(vec![goal]);
    match crate::lrat::derivation(num_vars, clauses, &log, &[log.len() - 1]) {
        Some(proof) => crate::lrat::check_derivation(num_vars, clauses, &proof, &[vec![goal]]),
        None => false,
    }
}

/// Whether `inv` is an inductive invariant that excludes every bad state, each obligation established by a checked
/// LRAT proof: the initial states satisfying the constraints lie in it; a state in it satisfying the constraints steps
/// only to states in it; and no state in it satisfying the constraints is bad. Each obligation is encoded over the
/// logic it reads: the constraints, and the next states of the latches the invariant reads or the bad states. The
/// first two derive the invariant's root as a unit, asked of it whole within a conflict for every four of its
/// conjuncts (the literals its uncomplemented And nodes join) and otherwise of each conjunct in turn; the last is one
/// refutation. The circuit `inv.aig` must have one input per latch.
pub fn check_invariant(aig: &Aig, inv: &Invariant) -> bool {
    if inv.aig.inputs().len() != aig.latches().len() || !inv.aig.latches().is_empty() {
        return false;
    }
    let parts = conjuncts(&inv.aig, inv.root);
    let support: Vec<usize> = inv
        .aig
        .cone(&[inv.root])
        .into_iter()
        .filter_map(|n| match inv.aig.node(n) {
            Node::Input(k) => Some(k),
            _ => None,
        })
        .collect();
    let constraints: Vec<AigLit> = aig.constraints().iter().map(|c| c.lit).collect();
    let premises = |reads: &[AigLit], build: &dyn Fn(&mut Unrolling, &mut Vec<Vec<Lit>>)| {
        let mut u = Unrolling::new(aig);
        u.add_frame();
        u.add_frame();
        let mut f = Vec::new();
        let roots: Vec<AigLit> = constraints.iter().chain(reads).copied().collect();
        u.gates_of(0, &roots, &mut f);
        u.constraints(0, &mut f);
        build(&mut u, &mut f);
        (u, f)
    };
    let holds_at = |t: usize, reads: &[AigLit], build: &dyn Fn(&mut Unrolling, &mut Vec<Vec<Lit>>)| {
        let (mut u, mut f) = premises(reads, build);
        let roots: Vec<AigLit> = parts.iter().copied().chain(std::iter::once(inv.root)).collect();
        let mut lits = u.region_roots(t, inv, &roots, &mut f);
        let goal = lits.pop().expect("the root is encoded last");
        implies(u.next_var, &f, goal, &lits, parts.len() as u64 / 4)
    };
    let initial = holds_at(0, &[], &|u, f| u.init(f));
    let next: Vec<AigLit> = support.iter().map(|&k| aig.latches()[k].next).collect();
    let inductive = holds_at(1, &next, &|u, f| {
        let at = u.region(0, inv, f);
        f.push(vec![at]);
        u.transition_of(0, &support, f);
    });
    let (u, f) = premises(&bad_lits(aig), &|u, f| {
        let at = u.region(0, inv, f);
        f.push(vec![at]);
        match u.bad(0, f) {
            Sig::Const(false) => f.push(Vec::new()),
            Sig::Const(true) => {}
            Sig::Lit(l) => f.push(vec![l]),
        }
    });
    let safe = refute(u.next_var, &f).is_ok();
    initial && inductive && safe
}

/// `clocks` clocks of `aig` unrolled over the logic they read, and the clauses that define it: at the last clock the
/// bad states' and constraints' cones, and at each clock before it theirs and the next states of the latches the clock
/// after it reads, each such latch equal to its next state.
fn backward_cone(aig: &Aig, clocks: usize) -> (Unrolling<'_>, Vec<Vec<Lit>>) {
    let mut u = Unrolling::new(aig);
    for _ in 0..clocks {
        u.add_frame();
    }
    let mut f = Vec::new();
    let always: Vec<AigLit> = bad_lits(aig).into_iter().chain(aig.constraints().iter().map(|c| c.lit)).collect();
    let mut read: Vec<usize> = Vec::new();
    for t in (0..clocks).rev() {
        let roots: Vec<AigLit> = always.iter().copied().chain(read.iter().map(|&j| aig.latches()[j].next)).collect();
        u.gates_of(t, &roots, &mut f);
        if t + 1 < clocks {
            u.transition_of(t, &read, &mut f);
        }
        read = aig
            .cone(&roots)
            .into_iter()
            .filter_map(|n| match aig.node(n) {
                Node::Latch(j) => Some(j),
                _ => None,
            })
            .collect();
    }
    (u, f)
}

/// Whether the bad states are `k`-inductive (`k` at least one), by two checked LRAT refutations: no run from an initial
/// state is bad at one of its first `k` clocks, the constraints holding up to that clock; and no `k + 1` consecutive
/// clocks from any state, the constraints holding at each, are bad at the last and at none before it. A run to a bad
/// state has a first bad clock: the first rules out one below `k`, the second the `k` clocks before one at `k` or
/// after. Each is encoded over the logic its clocks read: at the last clock the bad states' and constraints' cones, and
/// at each clock before it theirs and the next states of the latches the clock after it reads.
pub fn check_induction(aig: &Aig, k: usize) -> bool {
    if k == 0 {
        return false;
    }
    let bad_at = |u: &Unrolling, t: usize| -> Vec<Sig> { bad_lits(aig).into_iter().map(|b| u.sig(t, b)).collect() };
    let constraint_at =
        |u: &Unrolling, t: usize| -> Vec<Sig> { aig.constraints().iter().map(|c| u.sig(t, c.lit)).collect() };
    let (mut u, mut f) = backward_cone(aig, k);
    u.init(&mut f);
    let mut some = Vec::new();
    for j in 0..k {
        let b = u.fresh();
        let mut bad = vec![b.negated()];
        let mut always = false;
        for s in bad_at(&u, j) {
            match s {
                Sig::Const(true) => always = true,
                Sig::Const(false) => {}
                Sig::Lit(l) => bad.push(l),
            }
        }
        if !always {
            f.push(bad);
        }
        for t in 0..=j {
            for s in constraint_at(&u, t) {
                match s {
                    Sig::Const(true) => {}
                    Sig::Const(false) => f.push(vec![b.negated()]),
                    Sig::Lit(l) => f.push(vec![b.negated(), l]),
                }
            }
        }
        some.push(b);
    }
    f.push(some);
    if refute(u.next_var, &f).is_err() {
        return false;
    }
    let (u, mut f) = backward_cone(aig, k + 1);
    for t in 0..=k {
        u.constraints(t, &mut f);
    }
    for t in 0..k {
        for s in bad_at(&u, t) {
            match s {
                Sig::Const(true) => f.push(Vec::new()),
                Sig::Const(false) => {}
                Sig::Lit(l) => f.push(vec![l.negated()]),
            }
        }
    }
    let mut last = Vec::new();
    for s in bad_at(&u, k) {
        match s {
            Sig::Const(true) => return refute(u.next_var, &f).is_ok(),
            Sig::Const(false) => {}
            Sig::Lit(l) => last.push(l),
        }
    }
    f.push(last);
    refute(u.next_var, &f).is_ok()
}

/// Whether `trace` is a run of the circuit to a bad state: the initial latch values respect each latch's initial
/// value, every clock's state and inputs satisfy the constraints, and at the last clock a bad-state property holds.
pub fn replays(aig: &Aig, trace: &Trace) -> bool {
    let latches = aig.latches();
    if trace.initial.len() != latches.len()
        || trace.inputs.is_empty()
        || trace.inputs.iter().any(|x| x.len() != aig.inputs().len())
    {
        return false;
    }
    let respects = latches.iter().zip(&trace.initial).all(|(l, &v)| match l.init {
        Init::Zero => !v,
        Init::One => v,
        Init::Free => true,
    });
    if !respects {
        return false;
    }
    let bad = bad_lits(aig);
    let mut state = trace.initial.clone();
    for (t, inputs) in trace.inputs.iter().enumerate() {
        let values = aig.eval(inputs, &state);
        if !aig.constraints().iter().all(|c| Aig::value(&values, c.lit)) {
            return false;
        }
        if t + 1 == trace.inputs.len() {
            return bad.iter().any(|&b| Aig::value(&values, b));
        }
        state = latches.iter().map(|l| Aig::value(&values, l.next)).collect();
    }
    false
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_proof/itp/tests.rs"]
mod tests;
