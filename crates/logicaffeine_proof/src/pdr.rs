//! **Property-directed reachability** (Bradley, *SAT-based model checking without unrolling*, VMCAI 2011; Eén,
//! Mishchenko & Brayton, *Efficient implementation of property directed reachability*, FMCAD 2011) over and-inverter
//! graphs, on this crate's own CDCL solver.
//!
//! A circuit is safe when no run from an initial state reaches a bad state ([`crate::itp`] reads bad states, outputs
//! and constraints alike). The procedure keeps frames `F₀ = I, F₁, …, F_k`, each a set of clauses over the latches
//! holding every state reachable within its number of steps, and `F_k` free of bad states: a bad state of `F_k` is an
//! obligation to block, and blocking a cube `s` at frame `i` asks whether `F_{i-1} ∧ ¬s` steps into `s`. If it does,
//! the predecessor is an obligation at `i − 1`, lifted first to the latches the step into `s` needs, the inputs its
//! model set fixed, those it left open free, and the constraints assumed, so a cube keeps no latch only a constraint
//! reads; an obligation at frame 0 is a run from an initial state, and one through such cubes that breaks a constraint
//! on the way has its bad state searched again with every lifted cube keeping the constraints. If not, `¬s` holds at
//! `i`: the latches of `s` the refutation used (the solver's unsat core) and those a literal-dropping pass finds
//! unneeded leave a smaller cube, and its clause is added at the highest frame it stays inductive at. Once `F_k` holds
//! no bad state, every clause is pushed forward where it stays inductive; a frame left with no clause of its own equals
//! the next, and the clauses above it are an inductive invariant.
//!
//! One solver per frame holds the circuit's step as gates (each latch its variable, then the inputs, then a variable
//! for each cut of the cover of the step's cones by cuts of up to four leaves, a gate over its leaves defined by the
//! sums of products of its function and of its complement), the constraints on it, and the clauses at that frame or
//! above; frame 0's holds the initial values instead of clauses. A solver holds a gate once a query reads it, the cones of a query's assumptions given to it
//! before it is asked, so a frame's solver is the part of the step its queries have read, and propagating an
//! assignment never runs into logic nothing asked of it depends on. A query's own clause rides an activation literal
//! assumed for it and retired after. Every solver decides by justification, the frame's clauses and the query's beside
//! the gates ([`crate::cdcl::Solver::add_side_clause`]), so a query's search and its model are the part of the step it
//! needs, and a predecessor's lifting starts from the latches and inputs its model set.
//!
//! As the search begins and at each frame it opens, it asks whether the bad states are inductive one clock deeper than
//! the frames already free of them ([`Options::induction`]), in one solver unrolled forward over each clock's backward
//! cone: a yes is [`Verdict::Induction`], the depth [`crate::itp::check_induction`] refutes.
//!
//! Neither verdict is taken on the procedure's word: an invariant is accepted once [`crate::itp::check_invariant`] has
//! established its three obligations on the circuit asked about, each by an LRAT proof an independent checker replays,
//! and a run once [`crate::itp::replays`] has simulated it. A step that does not check makes the verdict
//! [`Verdict::Unknown`].

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use crate::aig::{Aig, AigLit, Init, Leaf, Node};
use crate::cdcl::{BudgetedResult, Lit, Solver, Var};
use crate::itp::{check_induction, check_invariant, replays, Invariant, Trace};

/// Bounds on the search.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Options {
    /// The most frames opened.
    pub max_frames: usize,
    /// The most obligations taken up.
    pub max_obligations: usize,
    /// The counterexamples to generalization a literal's dropping blocks in a row ([`Search::down`]); 0 drops
    /// literals alone.
    pub ctgs: usize,
    /// Lift each bad state and predecessor with the invariant constraints assumed rather than kept: a cube of the
    /// latches the bad output or the step reads, none that only a constraint reads. A run through such
    /// cubes can break a constraint where it leaves the states it was lifted from, so one that does not replay has its
    /// bad state searched again with every lifted state keeping the constraints. Off, every cube keeps them.
    pub assume_constraints: bool,
    /// The deepest induction step asked ([`Verdict::Induction`]): as the search begins and at each frame it opens, the
    /// bad states one clock deeper than the frames already free of them; 0 asks none.
    pub induction: usize,
    /// The conflicts an induction step may take before it is left unanswered at its depth.
    pub induction_conflicts: u64,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            max_frames: 1000,
            max_obligations: 1_000_000,
            ctgs: 3,
            assume_constraints: true,
            induction: 0,
            induction_conflicts: 1000,
        }
    }
}

/// What [`prove_safe`] found.
#[derive(Clone, Debug)]
pub enum Verdict {
    /// No reachable state is bad: the invariant passed [`crate::itp::check_invariant`].
    Safe(Invariant),
    /// No reachable state is bad: the bad states are inductive at this depth, no run reaching one within it, as
    /// [`crate::itp::check_induction`] refutes.
    Induction(usize),
    /// A bad state is reachable: the run passed [`crate::itp::replays`].
    Unsafe(Trace),
    /// Neither within the bounds, or a step that did not check, and why.
    Unknown(String),
}

/// What the search cost.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    /// The latches searched over: those whose values reach a bad state or a constraint.
    pub latches: usize,
    /// The lemmas blocked for counterexamples to generalization.
    pub ctgs: usize,
    /// Frames opened, obligations taken up, solves made, and the clauses of the invariant.
    pub frames: usize,
    pub obligations: usize,
    pub queries: usize,
    pub invariant: usize,
    /// The bad states searched again with every lifted state made to keep the constraints, after a run through
    /// cubes lifted with the constraints only assumed did not replay.
    pub restarts: usize,
}

/// A verdict and its cost.
#[derive(Clone, Debug)]
pub struct Report {
    pub verdict: Verdict,
    pub stats: Stats,
}

/// Decide whether a bad state of `aig` is reachable from its initial states: the search runs over `aig` cut to its
/// bad states' and constraints' cone ([`cone`]), and its certificate, taken back to `aig` (an invariant over every
/// latch, a run with each latch left out at its initial value, a free one at false), is held to `aig`'s checks.
pub fn prove_safe(aig: &Aig, opts: &Options) -> Report {
    let (cut, kept) = cone(aig);
    let mut search = Search::new(&cut);
    let verdict = search.resume(opts, usize::MAX).expect("a search without a budget ends in a verdict");
    let mut stats = search.stats;
    stats.latches = kept.len();
    Report { verdict: whole(aig, &kept, verdict), stats }
}

/// A verdict on `aig` cut to the latches `kept` ([`cone`]) as one on `aig`: an invariant over every latch, a run with
/// each latch left out at its initial value (a free one at false), each held to `aig`'s check.
pub(crate) fn whole(aig: &Aig, kept: &[usize], verdict: Verdict) -> Verdict {
    match verdict {
        Verdict::Safe(inv) => {
            let mut over = Aig::new();
            let all: Vec<AigLit> = (0..aig.latches().len()).map(|j| over.add_input(Some(format!("l{j}")))).collect();
            let map = over.import(&inv.aig, &mut |leaf| match leaf {
                Leaf::Input(j) => all[kept[j]],
                Leaf::Latch(_) => unreachable!("an invariant has no latches"),
            });
            let root = map[inv.root.node() as usize].with_phase(inv.root.is_complemented());
            let invariant = Invariant { aig: over, root };
            if check_invariant(aig, &invariant) {
                Verdict::Safe(invariant)
            } else {
                Verdict::Unknown("the invariant does not check on the whole circuit".into())
            }
        }
        Verdict::Induction(k) => {
            if check_induction(aig, k) {
                Verdict::Induction(k)
            } else {
                Verdict::Unknown("the induction does not check on the whole circuit".into())
            }
        }
        Verdict::Unsafe(trace) => {
            let mut initial: Vec<bool> = aig.latches().iter().map(|l| l.init == Init::One).collect();
            for (&k, &v) in kept.iter().zip(&trace.initial) {
                initial[k] = v;
            }
            let trace = Trace { initial, inputs: trace.inputs };
            if replays(aig, &trace) {
                Verdict::Unsafe(trace)
            } else {
                Verdict::Unknown("the run does not replay on the whole circuit".into())
            }
        }
        unknown => unknown,
    }
}

/// `aig` cut to what its bad states and constraints read through any number of clocks: every input, in order; the
/// latches whose values reach them, in order, each with its index in `aig` returned beside it; the nodes their cones
/// need; and the bad states (the outputs, for a circuit with none) and the constraints. A latch left out never reaches
/// what decides a run's badness, so an invariant over the latches kept is one over all of them, and a run of the cut
/// circuit is a run of `aig` with the others anywhere they may start.
pub(crate) fn cone(aig: &Aig) -> (Aig, Vec<usize>) {
    let roots: Vec<AigLit> = bad_lits(aig).into_iter().chain(aig.constraints().iter().map(|c| c.lit)).collect();
    let mut live = vec![false; aig.num_nodes()];
    let mut stack: Vec<u32> = roots.iter().map(|l| l.node()).collect();
    while let Some(n) = stack.pop() {
        if std::mem::replace(&mut live[n as usize], true) {
            continue;
        }
        match aig.node(n) {
            Node::And(a, b) => stack.extend([a.node(), b.node()]),
            Node::Latch(k) => stack.push(aig.latches()[k].next.node()),
            Node::False | Node::Input(_) => {}
        }
    }
    let mut out = Aig::new();
    let mut map: Vec<AigLit> = vec![AigLit::FALSE; aig.num_nodes()];
    for p in aig.inputs() {
        map[p.lit.node() as usize] = out.add_input(p.name.clone());
    }
    let kept: Vec<usize> = (0..aig.latches().len()).filter(|&k| live[aig.latches()[k].node as usize]).collect();
    let made: Vec<usize> =
        kept.iter().map(|&k| out.add_latch(aig.latches()[k].name.clone(), aig.latches()[k].init)).collect();
    for (&k, &j) in kept.iter().zip(&made) {
        map[aig.latches()[k].node as usize] = out.latch_lit(j);
    }
    let at = |map: &[AigLit], l: AigLit| map[l.node() as usize].with_phase(l.is_complemented());
    for n in 0..aig.num_nodes() as u32 {
        if let (Node::And(a, b), true) = (aig.node(n), live[n as usize]) {
            map[n as usize] = out.and(at(&map, a), at(&map, b));
        }
    }
    for (&k, &j) in kept.iter().zip(&made) {
        out.set_next(j, at(&map, aig.latches()[k].next));
    }
    for b in bad_lits(aig) {
        out.add_bad(at(&map, b), None);
    }
    for c in aig.constraints() {
        out.add_constraint(at(&map, c.lit), None);
    }
    (out, kept)
}

/// The bad-state literals: the bad-state properties, or the outputs of a circuit that has none.
fn bad_lits(aig: &Aig) -> Vec<AigLit> {
    let ports = if aig.bad().is_empty() { aig.outputs() } else { aig.bad() };
    ports.iter().map(|p| p.lit).collect()
}

/// The most leaves a cut of the cover a step is encoded by has ([`crate::aig_cnf::map`]).
pub(crate) const STEP_CUT_LEAVES: usize = 4;

/// The circuit's step as gates: latch `j` the variable `j`, input `i` the variable `latches + i`, then a variable for
/// the constant and one for each cut of the cover of the cones of the bad states, the constraints and the next states
/// ([`crate::aig_cnf::map`]), a cover gate over its leaves' variables ([`Solver::set_cover_gate`]) defined by the
/// irredundant sums of products of its function and its complement, and one for each And gate of the disjunction of
/// the bad-state properties.
struct Step {
    latches: usize,
    inputs: usize,
    num_vars: usize,
    constant: Var,
    /// Each And gate's variable and its two fan-ins.
    gates: Vec<(Var, Lit, Lit)>,
    /// Each cover gate's variable, the first of its products in `starts`, how many are its true value's and how many its
    /// false value's, and the first of its leaves' variables in `leaves` and how many: each product's literals in
    /// `products` from its start to the next one's.
    covers: Vec<(Var, u32, u16, u16, u32, u8)>,
    starts: Vec<u32>,
    products: Vec<Lit>,
    leaves: Vec<Var>,
    /// Per latch, the literal of its next value; the bad literal; the constraints' literals; the initial values.
    next: Vec<Lit>,
    bad: Lit,
    constraints: Vec<Lit>,
    init: Vec<Init>,
    /// Per variable, its gate's place among the And gates and then the cover gates, plus one (0 for a latch, an input
    /// or the constant).
    gate_at: Vec<u32>,
}

impl Step {
    /// The step over the cover by cuts of at most `k` leaves.
    fn with_cuts(aig: &Aig, k: usize) -> Step {
        let (latches, inputs) = (aig.latches().len(), aig.inputs().len());
        let mut roots = bad_lits(aig);
        roots.extend(aig.constraints().iter().map(|c| c.lit));
        roots.extend(aig.latches().iter().map(|l| l.next));
        let mapping = crate::aig_cnf::map(aig, &roots, k);
        let mut var: Vec<Var> = vec![Var::MAX; aig.num_nodes()];
        let mut num_vars = latches + inputs;
        for n in 0..aig.num_nodes() as u32 {
            var[n as usize] = match aig.node(n) {
                Node::Latch(k) => k as Var,
                Node::Input(k) => (latches + k) as Var,
                Node::And(..) if mapping.cut(n).is_none() => continue,
                Node::False | Node::And(..) => {
                    num_vars += 1;
                    (num_vars - 1) as Var
                }
            };
        }
        let lit = |l: AigLit| Lit::new(var[l.node() as usize], !l.is_complemented());
        let (mut covers, mut starts, mut products, mut leaves) = (Vec::new(), vec![0u32], Vec::new(), Vec::new());
        let mut sums = std::collections::HashMap::new();
        for n in 0..aig.num_nodes() as u32 {
            let Some(cut) = mapping.cut(n) else { continue };
            let of: Vec<Var> = cut.leaves().iter().map(|&l| var[l as usize]).collect();
            let (on, off) = sums
                .entry((cut.truth, of.len()))
                .or_insert_with(|| (crate::aig_cnf::isop(cut.truth, of.len()), crate::aig_cnf::isop(!cut.truth, of.len())));
            let first = starts.len() as u32 - 1;
            for c in on.iter().chain(off.iter()) {
                for (i, &v) in of.iter().enumerate() {
                    if c.pos >> i & 1 == 1 {
                        products.push(Lit::pos(v));
                    } else if c.neg >> i & 1 == 1 {
                        products.push(Lit::neg(v));
                    }
                }
                starts.push(products.len() as u32);
            }
            covers.push((var[n as usize], first, on.len() as u16, off.len() as u16, leaves.len() as u32, of.len() as u8));
            leaves.extend(of);
        }
        let mut gates = Vec::new();
        let bads: Vec<Lit> = bad_lits(aig).into_iter().map(lit).collect();
        let mut bad = bads.first().copied().unwrap_or(Lit::pos(var[0]));
        for &b in &bads[1.min(bads.len())..] {
            let g = num_vars as Var;
            num_vars += 1;
            gates.push((g, bad.negated(), b.negated()));
            bad = Lit::neg(g);
        }
        let mut gate_at = vec![0u32; num_vars];
        for (k, &(g, _, _)) in gates.iter().enumerate() {
            gate_at[g as usize] = k as u32 + 1;
        }
        for (k, &(g, ..)) in covers.iter().enumerate() {
            gate_at[g as usize] = (gates.len() + k) as u32 + 1;
        }
        Step {
            latches,
            inputs,
            num_vars,
            constant: var[0],
            gates,
            covers,
            starts,
            products,
            leaves,
            gate_at,
            next: aig.latches().iter().map(|l| lit(l.next)).collect(),
            bad,
            constraints: aig.constraints().iter().map(|c| lit(c.lit)).collect(),
            init: aig.latches().iter().map(|l| l.init).collect(),
        }
    }

    /// A solver over the step, deciding by justification, holding the gates in the constraints' cones, and the
    /// constraints when `constrained` says so; and which of the step's gates it holds. A query's search, and the state
    /// and inputs of its model, are the part of the step its assumptions and the clauses it holds beside the step
    /// need, so a gate is held once a query reads it ([`Step::hold`]): one outside every cone a solver's queries read
    /// is a definition nothing it is asked depends on.
    fn solver(&self, constrained: bool) -> (Solver, Vec<bool>) {
        let mut s = Solver::new(self.num_vars);
        s.add_clause(vec![Lit::neg(self.constant)]);
        s.set_justifying(true);
        let mut held = vec![false; self.gates.len() + self.covers.len()];
        self.hold(&mut s, &mut held, &self.constraints);
        if constrained {
            for &c in &self.constraints {
                s.add_clause(vec![c]);
            }
        }
        (s, held)
    }

    /// Give solver `s` the gates of the cones of `roots` it does not hold yet, marking them in `held`: what it holds is
    /// closed under fan-ins, so a gate held stops the walk.
    fn hold(&self, s: &mut Solver, held: &mut [bool], roots: &[Lit]) {
        let mut stack: Vec<Var> = roots.iter().map(|l| l.var()).collect();
        while let Some(v) = stack.pop() {
            let k = self.gate_at[v as usize];
            if k == 0 || std::mem::replace(&mut held[k as usize - 1], true) {
                continue;
            }
            let Some(&(g, first, on, off, leaf, n)) = (k as usize - 1).checked_sub(self.gates.len()).map(|c| &self.covers[c])
            else {
                let (g, a, b) = self.gates[k as usize - 1];
                let out = Lit::pos(g);
                s.add_clause(vec![out.negated(), a]);
                s.add_clause(vec![out.negated(), b]);
                s.add_clause(vec![out, a.negated(), b.negated()]);
                s.set_gate(g as usize, a, b);
                stack.push(a.var());
                stack.push(b.var());
                continue;
            };
            let product = |c: u32| self.products[self.starts[c as usize] as usize..self.starts[c as usize + 1] as usize].to_vec();
            let ons: Vec<Vec<Lit>> = (first..first + on as u32).map(product).collect();
            let offs: Vec<Vec<Lit>> = (first + on as u32..first + on as u32 + off as u32).map(product).collect();
            for (sums, out) in [(&ons, Lit::pos(g)), (&offs, Lit::neg(g))] {
                for c in sums {
                    s.add_clause(std::iter::once(out).chain(c.iter().map(|l| l.negated())).collect());
                }
            }
            s.set_cover_gate(g as usize, &ons, &offs);
            stack.extend(&self.leaves[leaf as usize..leaf as usize + n as usize]);
        }
    }

    /// Whether cube `s` (literals of latches) holds an initial state: none of its literals contradicts an initial
    /// value.
    fn meets_init(&self, s: &[Lit]) -> bool {
        s.iter().all(|l| match self.init[l.var() as usize] {
            Init::Zero => !l.is_positive(),
            Init::One => l.is_positive(),
            Init::Free => true,
        })
    }

    /// The state of cube `s` (one meeting the initial states) with every latch it leaves free at its initial value,
    /// or at `fallback`'s for a latch free to start anywhere.
    fn initial_in(&self, s: &[Lit], fallback: &[bool]) -> Vec<bool> {
        let mut state: Vec<bool> = (0..self.latches)
            .map(|j| match self.init[j] {
                Init::Zero => false,
                Init::One => true,
                Init::Free => fallback[j],
            })
            .collect();
        for l in s {
            state[l.var() as usize] = l.is_positive();
        }
        state
    }

    /// Latch literal `l` read on the next state.
    fn primed(&self, l: Lit) -> Lit {
        let n = self.next[l.var() as usize];
        if l.is_positive() {
            n
        } else {
            n.negated()
        }
    }
}

/// Values a model left some of open, each open one at false.
fn complete(state: &[Option<bool>]) -> Vec<bool> {
    state.iter().map(|v| v.unwrap_or(false)).collect()
}

/// How deep a counterexample to generalization's own generalization takes up counterexamples in turn, as Bradley's
/// IC3ref does.
const CTG_DEPTH: usize = 1;

/// A cube to block: its latch literals, the inputs that step its states into the cube of the obligation it came from
/// (or, for the first, into a bad state), and that obligation.
#[derive(Clone, Debug)]
struct Obligation {
    cube: Vec<Lit>,
    inputs: Vec<bool>,
    parent: Option<usize>,
}

/// A frame's solver and how many of its activation literals are retired, the clauses blocked at each frame (at
/// frame `i`, those that hold at `i` and not yet known to hold at `i + 1`).
pub(crate) struct Search<'a> {
    aig: &'a Aig,
    step: Step,
    solvers: Vec<Solver>,
    /// Per frame's solver, and the lifter's, the step's gates it holds.
    held: Vec<Vec<bool>>,
    retired: Vec<usize>,
    levels: Vec<Vec<Vec<Lit>>>,
    lifter: Solver,
    lifter_held: Vec<bool>,
    lifts: usize,
    /// Whether the bad state under search lifts its cubes keeping the constraints: so with
    /// [`Options::assume_constraints`] off, and once a run through cubes that assumed them did not replay.
    strict: bool,
    obligations: Vec<Obligation>,
    /// The induction step's solver, once a step has been asked.
    induction: Option<Induction>,
    /// The obligations still to take up, least frame first, and whether the search has begun.
    queue: BinaryHeap<Reverse<(usize, usize)>>,
    begun: bool,
    pub(crate) stats: Stats,
}

/// What a solve on a frame found.
enum Found {
    /// A model: the latch and input values it sets, any value doing for one it leaves open.
    Model(Vec<Option<bool>>, Vec<Option<bool>>),
    /// A refutation, and the assumptions it rests on.
    Refuted(Vec<Lit>),
}

impl<'a> Search<'a> {
    pub(crate) fn new(aig: &'a Aig) -> Search<'a> {
        Search::with_cuts(aig, STEP_CUT_LEAVES)
    }

    /// The search over a step encoded by the cover with cuts of at most `k` leaves.
    pub(crate) fn with_cuts(aig: &'a Aig, k: usize) -> Search<'a> {
        let step = Step::with_cuts(aig, k);
        let (mut init, init_held) = step.solver(true);
        for (j, i) in step.init.iter().enumerate() {
            match i {
                Init::Zero => init.add_clause(vec![Lit::neg(j as Var)]),
                Init::One => init.add_clause(vec![Lit::pos(j as Var)]),
                Init::Free => {}
            }
        }
        let (lifter, lifter_held) = step.solver(false);
        let stats = Stats { latches: aig.latches().len(), ..Stats::default() };
        Search {
            aig,
            step,
            solvers: vec![init],
            held: vec![init_held],
            retired: vec![0],
            levels: vec![Vec::new()],
            lifter,
            lifter_held,
            lifts: 0,
            strict: false,
            obligations: Vec::new(),
            induction: None,
            queue: BinaryHeap::new(),
            begun: false,
            stats,
        }
    }

    /// Solve frame `i`'s solver under `assumptions`, with `clause` held for this solve only.
    fn solve(&mut self, i: usize, clause: Option<&[Lit]>, assumptions: &[Lit]) -> Found {
        self.stats.queries += 1;
        self.step.hold(&mut self.solvers[i], &mut self.held[i], assumptions);
        let s = &mut self.solvers[i];
        let mut all = assumptions.to_vec();
        let act = clause.map(|c| {
            let a = Lit::pos(s.add_vars(1) as Var);
            s.add_side_clause(std::iter::once(a.negated()).chain(c.iter().copied()).collect());
            all.insert(0, a);
            a
        });
        let found = match s.solve_under_assumptions_budgeted(&all, 0) {
            BudgetedResult::Sat(_) => {
                let (l, n) = (self.step.latches, self.step.inputs);
                Found::Model((0..l).map(|v| s.value(v)).collect(), (l..l + n).map(|v| s.value(v)).collect())
            }
            BudgetedResult::Unsat => Found::Refuted(s.unsat_core().to_vec()),
            BudgetedResult::Budget => unreachable!("a solve without a budget runs to a verdict"),
        };
        if let Some(a) = act {
            s.add_clause(vec![a.negated()]);
            self.retired[i] += 1;
        }
        if self.retired[i] > 2000 {
            self.rebuild(i);
        }
        found
    }

    /// Frame `i`'s solver made afresh: the step, its constraints, and the clauses at `i` or above (frame 0's initial
    /// values).
    fn rebuild(&mut self, i: usize) {
        let (mut s, held) = self.step.solver(true);
        self.held[i] = held;
        if i == 0 {
            for (j, init) in self.step.init.iter().enumerate() {
                match init {
                    Init::Zero => s.add_clause(vec![Lit::neg(j as Var)]),
                    Init::One => s.add_clause(vec![Lit::pos(j as Var)]),
                    Init::Free => {}
                }
            }
        } else {
            for level in &self.levels[i..] {
                for c in level {
                    s.add_side_clause(c.clone());
                }
            }
        }
        self.solvers[i] = s;
        self.retired[i] = 0;
    }

    /// The latch literals of `state` (those a model set) that the step needs under the inputs it set (`inputs`, each a
    /// value or `None` for one left open) to reach `target` (next-state literals), whatever the inputs it left open: an
    /// unsat core of `state ∧ inputs ∧ constraints ∧ ¬target`, the constraints assumed, or of `state ∧ inputs ∧
    /// ¬(target ∧ constraints)` when the search is strict, every state of the cube then keeping them; the whole state,
    /// a latch left open at false, should that not be refuted.
    fn lift<I: Copy + Into<Option<bool>>>(&mut self, state: &[Option<bool>], inputs: &[I], target: &[Lit]) -> Vec<Lit> {
        let set: Vec<Lit> = state.iter().enumerate().filter_map(|(j, v)| v.map(|b| Lit::new(j as Var, b))).collect();
        self.step.hold(&mut self.lifter, &mut self.lifter_held, target);
        let a = Lit::pos(self.lifter.add_vars(1) as Var);
        let assumed = !self.strict;
        let negated: Vec<Lit> = if assumed {
            target.iter().map(|l| l.negated()).collect()
        } else {
            target.iter().chain(&self.step.constraints).map(|l| l.negated()).collect()
        };
        self.lifter.add_side_clause(std::iter::once(a.negated()).chain(negated).collect());
        let mut assumptions = vec![a];
        if assumed {
            assumptions.extend(&self.step.constraints);
        }
        assumptions.extend(&set);
        let l = self.step.latches;
        let set_inputs = inputs.iter().enumerate().filter_map(|(i, &v)| v.into().map(|b| Lit::new((l + i) as Var, b)));
        assumptions.extend(set_inputs);
        let lifted = match self.lifter.solve_under_assumptions_budgeted(&assumptions, 0) {
            BudgetedResult::Unsat => {
                let core = self.lifter.unsat_core();
                set.into_iter().filter(|x| core.contains(x)).collect()
            }
            _ => complete(state).iter().enumerate().map(|(j, &b)| Lit::new(j as Var, b)).collect(),
        };
        self.lifter.add_clause(vec![a.negated()]);
        self.lifts += 1;
        if self.lifts > 2000 {
            (self.lifter, self.lifter_held) = self.step.solver(false);
            self.lifts = 0;
        }
        lifted
    }

    /// A bad state of frame `k` and its inputs, lifted to the latches the bad state needs.
    fn bad_state(&mut self, k: usize) -> Option<(Vec<Lit>, Vec<bool>)> {
        let bad = self.step.bad;
        match self.solve(k, None, &[bad]) {
            Found::Model(state, inputs) => Some((self.lift(&state, &inputs, &[bad]), complete(&inputs))),
            Found::Refuted(_) => None,
        }
    }

    /// Take up frame `k`'s next bad state, if it has one, as the first obligation: `Some(Err(run))` when its cube holds
    /// an initial state, the run from one, and `Some(Ok(()))` once it is queued. A cube that holds an initial state is
    /// never blocked, so the invariant holds every initial state.
    fn take_bad_state(&mut self, k: usize) -> Option<Result<(), Trace>> {
        let (cube, inputs) = self.bad_state(k)?;
        self.obligations.clear();
        if self.step.meets_init(&cube) {
            let initial = self.step.initial_in(&cube, &vec![false; self.step.latches]);
            return Some(Err(Trace { initial, inputs: vec![inputs] }));
        }
        self.obligations.push(Obligation { cube, inputs, parent: None });
        self.queue.push(Reverse((k, 0)));
        Some(Ok(()))
    }

    /// The verdict a run from an initial state gives: unsafe once it replays, or once the search is strict and
    /// [`Search::unsafe_`] has the last word. A run through cubes that assumed the constraints can break one on the
    /// way: then `None`, frame `k`'s bad state searched again with every lifted cube keeping them.
    fn settle(&mut self, trace: Trace, k: usize) -> Option<Verdict> {
        if self.strict || replays(self.aig, &trace) {
            return Some(self.unsafe_(trace));
        }
        self.strict = true;
        self.stats.restarts += 1;
        self.queue.clear();
        match self.take_bad_state(k) {
            Some(Err(trace)) => Some(self.unsafe_(trace)),
            _ => None,
        }
    }

    /// Whether `F_{i-1} ∧ ¬s` steps into `s`: a predecessor state and its inputs, or the latches of `s` the refutation
    /// rests on, a cube that still holds no initial state.
    fn relative(&mut self, i: usize, s: &[Lit]) -> Result<Vec<Lit>, (Vec<Option<bool>>, Vec<Option<bool>>)> {
        let negated: Vec<Lit> = s.iter().map(|l| l.negated()).collect();
        let next: Vec<Lit> = s.iter().map(|&l| self.step.primed(l)).collect();
        match self.solve(i - 1, Some(&negated), &next) {
            Found::Model(state, inputs) => Err((state, inputs)),
            Found::Refuted(core) => {
                let used = s.iter().zip(&next).filter(|(_, n)| core.contains(n));
                let mut kept: Vec<Lit> = used.map(|(&l, _)| l).collect();
                if self.step.meets_init(&kept) {
                    let contradicts = s.iter().copied().find(|l| !self.step.meets_init(&[*l]));
                    kept.extend(contradicts);
                }
                kept.sort_by_key(|l| l.var());
                Ok(kept)
            }
        }
    }

    /// Cube `s`, already blocked at frame `i`, with every literal dropped whose dropping keeps it blocked there: each
    /// literal tried in turn ([`Search::down`]), at most `ctgs` counterexamples to generalization in a row taken up, no
    /// deeper than [`CTG_DEPTH`].
    fn generalize(&mut self, i: usize, s: Vec<Lit>, ctgs: usize) -> Vec<Lit> {
        self.generalize_at(i, s, ctgs, 0)
    }

    fn generalize_at(&mut self, i: usize, mut s: Vec<Lit>, ctgs: usize, depth: usize) -> Vec<Lit> {
        let mut k = 0;
        while k < s.len() && s.len() > 1 {
            let mut fewer = s.clone();
            fewer.remove(k);
            match self.down(i, fewer, ctgs, depth) {
                Some(core) => s = core,
                None => k += 1,
            }
        }
        s
    }

    /// Cube `s` made blocked at frame `i` if it can be (Hassan, Bradley & Somenzi, *Better generalization in IC3*,
    /// FMCAD 2013): its blocked core, once `F_{i−1} ∧ ¬s` no longer steps into it. A predecessor `t` that does (a
    /// counterexample to generalization) is blocked at `i − 1` in its own generalized form when it is blocked there,
    /// up to `most` in a row, the lemma raised to the highest frame it holds at; otherwise `s` keeps only the literals
    /// `t` sets as `s` has them, the smaller cube holding `t`, and is asked again. `None` once `s` holds an initial
    /// state or `t` agrees with all of it.
    fn down(&mut self, i: usize, mut s: Vec<Lit>, most: usize, depth: usize) -> Option<Vec<Lit>> {
        let mut ctgs = 0;
        loop {
            if s.is_empty() || self.step.meets_init(&s) {
                return None;
            }
            let (state, _) = match self.relative(i, &s) {
                Ok(core) => return Some(core),
                Err(predecessor) => predecessor,
            };
            let t: Vec<Lit> = state.iter().enumerate().filter_map(|(j, v)| v.map(|b| Lit::new(j as Var, b))).collect();
            if depth < CTG_DEPTH && ctgs < most && i >= 2 && !self.step.meets_init(&t) {
                if let Ok(core) = self.relative(i - 1, &t) {
                    ctgs += 1;
                    self.stats.ctgs += 1;
                    let lemma = self.generalize_at(i - 1, core, most, depth + 1);
                    let top = self.levels.len() - 1;
                    let mut level = i - 1;
                    while level < top && self.relative(level + 1, &lemma).is_ok() {
                        level += 1;
                    }
                    self.block(&lemma, level);
                    continue;
                }
            }
            ctgs = 0;
            let joined: Vec<Lit> = s.iter().copied().filter(|l| t.contains(l)).collect();
            if joined.len() == s.len() {
                return None;
            }
            s = joined;
        }
    }

    /// Add `¬s` at frame `level`: to the solvers of frames 1 to `level`, dropping the clauses it subsumes at those
    /// frames.
    fn block(&mut self, s: &[Lit], level: usize) {
        let clause: Vec<Lit> = s.iter().map(|l| l.negated()).collect();
        for frame in 1..=level {
            self.levels[frame].retain(|d| !clause.iter().all(|l| d.contains(l)));
            self.solvers[frame].add_side_clause(clause.clone());
        }
        self.levels[level].push(clause);
    }

    /// Move clause `c` from frame `i` to frame `i + 1`, where the step keeps it.
    fn promote(&mut self, c: &[Lit], i: usize) {
        self.levels[i].retain(|d| d.as_slice() != c);
        self.solvers[i + 1].add_side_clause(c.to_vec());
        self.levels[i + 1].push(c.to_vec());
    }

    /// Whether cube `s` holds no state of frame `i` already.
    fn excluded(&mut self, i: usize, s: &[Lit]) -> bool {
        matches!(self.solve(i, None, s), Found::Refuted(_))
    }

    /// The obligation of the predecessor `state` that steps under `inputs` into the cube of obligation `at`, lifted to
    /// the latches that step needs: its index, or, when the lifted cube holds an initial state, the run from that
    /// state.
    fn predecessor<I: Copy + Into<Option<bool>>>(
        &mut self,
        at: usize,
        state: Vec<Option<bool>>,
        inputs: Vec<I>,
    ) -> Result<usize, Trace> {
        let target: Vec<Lit> = self.obligations[at].cube.iter().map(|&l| self.step.primed(l)).collect();
        let pred = self.lift(&state, &inputs, &target);
        let initial = self.step.meets_init(&pred).then(|| self.step.initial_in(&pred, &complete(&state)));
        let inputs: Vec<Option<bool>> = inputs.into_iter().map(Into::into).collect();
        self.obligations.push(Obligation { cube: pred, inputs: complete(&inputs), parent: Some(at) });
        match initial {
            Some(initial) => Err(self.trace(self.obligations.len() - 1, initial)),
            None => Ok(self.obligations.len() - 1),
        }
    }

    /// Open frame `k + 1`.
    fn open(&mut self) {
        let (s, held) = self.step.solver(true);
        self.solvers.push(s);
        self.held.push(held);
        self.retired.push(0);
        self.levels.push(Vec::new());
        self.stats.frames = self.levels.len() - 1;
    }

    /// The run the obligation at index `at` (at frame 0) begins.
    fn trace(&self, at: usize, initial: Vec<bool>) -> Trace {
        let mut inputs = Vec::new();
        let mut next = Some(at);
        while let Some(i) = next {
            inputs.push(self.obligations[i].inputs.clone());
            next = self.obligations[i].parent;
        }
        Trace { initial, inputs }
    }

    /// Continue the search through at most `budget` more obligations: its verdict once it has one, `None` while the
    /// budget ends it first. A search resumed in slices is the search run whole.
    pub(crate) fn resume(&mut self, opts: &Options, budget: usize) -> Option<Verdict> {
        if !self.begun {
            self.begun = true;
            if bad_lits(self.aig).is_empty() {
                return Some(self.safe(0));
            }
            let bad = self.step.bad;
            if let Found::Model(state, inputs) = self.solve(0, None, &[bad]) {
                return Some(self.unsafe_(Trace { initial: complete(&state), inputs: vec![complete(&inputs)] }));
            }
            self.open();
            if self.inductive(opts, 1) {
                return Some(Verdict::Induction(1));
            }
        }
        let mut spent = 0;
        loop {
            let k = self.levels.len() - 1;
            if self.queue.is_empty() {
                self.strict = !opts.assume_constraints;
                match self.take_bad_state(k) {
                    Some(Ok(())) => {}
                    Some(Err(trace)) => {
                        if let Some(verdict) = self.settle(trace, k) {
                            return Some(verdict);
                        }
                    }
                    None => {
                        if k >= opts.max_frames {
                            return Some(Verdict::Unknown(format!("no verdict within {} frames", opts.max_frames)));
                        }
                        self.open();
                        if let Some(i) = self.propagate(k) {
                            return Some(self.safe(i + 1));
                        }
                        if self.inductive(opts, k + 1) {
                            return Some(Verdict::Induction(k + 1));
                        }
                        continue;
                    }
                }
            }
            while let Some(Reverse((frame, at))) = self.queue.pop() {
                if spent == budget {
                    self.queue.push(Reverse((frame, at)));
                    return None;
                }
                spent += 1;
                self.stats.obligations += 1;
                if self.stats.obligations > opts.max_obligations {
                    return Some(Verdict::Unknown(format!("no verdict within {} obligations", opts.max_obligations)));
                }
                let cube = self.obligations[at].cube.clone();
                if self.excluded(frame, &cube) {
                    continue;
                }
                match self.relative(frame, &cube) {
                    Err((state, inputs)) => {
                        let found = if frame == 1 {
                            let inputs = complete(&inputs);
                            self.obligations.push(Obligation { cube: Vec::new(), inputs, parent: Some(at) });
                            Err(self.trace(self.obligations.len() - 1, complete(&state)))
                        } else {
                            self.predecessor(at, state, inputs)
                        };
                        match found {
                            Ok(pred) => {
                                self.queue.push(Reverse((frame - 1, pred)));
                                self.queue.push(Reverse((frame, at)));
                            }
                            Err(trace) => {
                                if let Some(verdict) = self.settle(trace, k) {
                                    return Some(verdict);
                                }
                            }
                        }
                    }
                    Ok(core) => {
                        let g = self.generalize(frame, core, opts.ctgs);
                        let mut level = frame;
                        while level < k && self.relative(level + 1, &g).is_ok() {
                            level += 1;
                        }
                        self.block(&g, level);
                        if level < k {
                            self.queue.push(Reverse((level + 1, at)));
                        }
                    }
                }
            }
        }
    }

    /// Push every clause of frames 1 to `k` to the next frame where the step keeps it, frame by frame: the first frame
    /// left with no clause of its own, if one is.
    fn propagate(&mut self, k: usize) -> Option<usize> {
        for i in 1..=k {
            let clauses = self.levels[i].clone();
            for c in clauses {
                let cube: Vec<Lit> = c.iter().map(|l| l.negated()).collect();
                let next: Vec<Lit> = cube.iter().map(|&l| self.step.primed(l)).collect();
                if let Found::Refuted(_) = self.solve(i, None, &next) {
                    self.promote(&c, i);
                }
            }
            if self.levels[i].is_empty() {
                return Some(i);
            }
        }
        None
    }

    /// Whether the bad states are inductive at depth `k`, asked when the frames below `k` hold no bad state, so no run
    /// reaches one within `k` clocks: when `k` is at most [`Options::induction`] and the step refutes within
    /// [`Options::induction_conflicts`].
    fn inductive(&mut self, opts: &Options, k: usize) -> bool {
        if k > opts.induction {
            return false;
        }
        let step = &self.step;
        let induction = self.induction.get_or_insert_with(|| Induction::new(step));
        induction.ask(step, k, opts.induction_conflicts) == Some(true)
    }

    /// The clauses at frame `from` and above as an invariant, the conjunction of their disjunctions, which [`whole`]
    /// holds to the circuit's checks.
    fn safe(&mut self, from: usize) -> Verdict {
        let mut inv = Aig::new();
        let latches: Vec<AigLit> = (0..self.step.latches).map(|j| inv.add_input(Some(format!("l{j}")))).collect();
        let mut root = AigLit::TRUE;
        let clauses: Vec<Vec<Lit>> = self.levels.iter().skip(from).flatten().cloned().collect();
        self.stats.invariant = clauses.len();
        for c in &clauses {
            let any = c.iter().fold(AigLit::FALSE, |acc, l| {
                let x = latches[l.var() as usize].with_phase(!l.is_positive());
                inv.or(acc, x)
            });
            root = inv.and(root, any);
        }
        Verdict::Safe(Invariant { aig: inv, root })
    }

    /// A run to a bad state, accepted once it replays.
    fn unsafe_(&self, trace: Trace) -> Verdict {
        if replays(self.aig, &trace) {
            Verdict::Unsafe(trace)
        } else {
            Verdict::Unknown("the run does not replay".into())
        }
    }
}

/// The step of k-induction unrolled forward in one incremental solver: clock `t`'s variables the step's shifted by `t`
/// copies, each clock holding the gates in the cones of its constraints and bad state (an And gate by its three
/// clauses, a cover gate by its products'), and each latch a held gate reads tied to its next state at the clock before (the latches of the first clock free), so the solver is a
/// segment's backward cone. Depth `k` asks for `k + 1` clocks bad at the last, those before it made good as the
/// solver deepens.
struct Induction {
    solver: Solver,
    nv: usize,
    /// Per clock, the step's gates held, and the latches tied to the clock before.
    held: Vec<Vec<bool>>,
    tied: Vec<Vec<bool>>,
    /// Per clock, its bad literal.
    bad: Vec<Lit>,
}

impl Induction {
    fn new(step: &Step) -> Induction {
        Induction { solver: Solver::new(0), nv: step.num_vars, held: Vec::new(), tied: Vec::new(), bad: Vec::new() }
    }

    /// Step literal `l` at clock `t`.
    fn at(&self, t: usize, l: Lit) -> Lit {
        Lit::new((t * self.nv) as Var + l.var(), l.is_positive())
    }

    /// Hold the cones of `roots` at clock `t`, and through each latch they read, the cone of its next state at the
    /// clock before.
    fn hold(&mut self, step: &Step, t: usize, roots: &[Lit]) {
        let mut stack: Vec<(usize, Var)> = roots.iter().map(|l| (t, l.var())).collect();
        while let Some((t, v)) = stack.pop() {
            let k = step.gate_at[v as usize];
            if k != 0 {
                if std::mem::replace(&mut self.held[t][k as usize - 1], true) {
                    continue;
                }
                if let Some(&(g, first, on, off, leaf, n)) = (k as usize - 1).checked_sub(step.gates.len()).map(|c| &step.covers[c]) {
                    for c in first..first + on as u32 + off as u32 {
                        let value = if c < first + on as u32 { Lit::pos(g) } else { Lit::neg(g) };
                        let product = &step.products[step.starts[c as usize] as usize..step.starts[c as usize + 1] as usize];
                        let clause = std::iter::once(self.at(t, value)).chain(product.iter().map(|&l| self.at(t, l).negated()));
                        self.solver.add_clause(clause.collect());
                    }
                    stack.extend(step.leaves[leaf as usize..leaf as usize + n as usize].iter().map(|&v| (t, v)));
                    continue;
                }
                let (g, a, b) = step.gates[k as usize - 1];
                let (out, a2, b2) = (self.at(t, Lit::pos(g)), self.at(t, a), self.at(t, b));
                self.solver.add_clause(vec![out.negated(), a2]);
                self.solver.add_clause(vec![out.negated(), b2]);
                self.solver.add_clause(vec![out, a2.negated(), b2.negated()]);
                stack.push((t, a.var()));
                stack.push((t, b.var()));
            } else if (v as usize) < step.latches && t > 0 && !std::mem::replace(&mut self.tied[t][v as usize], true) {
                let (now, before) = (self.at(t, Lit::pos(v)), self.at(t - 1, step.next[v as usize]));
                self.solver.add_clause(vec![now.negated(), before]);
                self.solver.add_clause(vec![now, before.negated()]);
                stack.push((t - 1, step.next[v as usize].var()));
            }
        }
    }

    /// One more clock: its constant false, its constraints held and asserted, its bad state held.
    fn deepen(&mut self, step: &Step) {
        let t = self.held.len();
        self.solver.add_vars(self.nv);
        self.held.push(vec![false; step.gates.len() + step.covers.len()]);
        self.tied.push(vec![false; step.latches]);
        let constant = self.at(t, Lit::neg(step.constant));
        self.solver.add_clause(vec![constant]);
        let roots: Vec<Lit> = step.constraints.iter().copied().chain(std::iter::once(step.bad)).collect();
        self.hold(step, t, &roots);
        for &c in &step.constraints {
            let c = self.at(t, c);
            self.solver.add_clause(vec![c]);
        }
        let bad = self.at(t, step.bad);
        self.bad.push(bad);
    }

    /// Whether no `k + 1` clocks, the constraints holding at each, are bad at the last and at none before: `None` when
    /// `budget` conflicts (0 for none) end the search first. Depths are asked in increasing order.
    fn ask(&mut self, step: &Step, k: usize, budget: u64) -> Option<bool> {
        while self.held.len() <= k {
            if let Some(&b) = self.bad.last() {
                self.solver.add_clause(vec![b.negated()]);
            }
            self.deepen(step);
        }
        match self.solver.solve_under_assumptions_budgeted(&[self.bad[k]], budget) {
            BudgetedResult::Unsat => Some(true),
            BudgetedResult::Sat(_) => Some(false),
            BudgetedResult::Budget => None,
        }
    }
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_proof/pdr/tests.rs"]
mod tests;
