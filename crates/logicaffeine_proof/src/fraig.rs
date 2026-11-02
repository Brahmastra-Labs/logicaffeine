//! Combinational equivalence by incremental SAT sweeping, certified by one derivation.
//!
//! [`crate::sweep`] puts each candidate to a fresh solver over a formula of its own and certifies each lemma by an LRAT
//! refutation of that formula. Here every question is a solve under assumptions on one solver, so what one query
//! learns later ones keep (Mishchenko, Chatterjee, Brayton & Een, *Improvements to combinational equivalence
//! checking*, ICCAD 2006, the scheme of ABC's `&cec`). The solver holds the clauses of the miter's formula ([`formula`]:
//! the Tseitin clauses of every And node, node `k` the variable `k`) only for the cones its queries have asked about,
//! encoded as each query needs them, and decides only on those nodes, so a query's search stays in its cone. Its
//! variables are numbered as it meets them, so a solver is as large as what it was asked, and once it has
//! [`Options::recycle`] of them a fresh one replaces it. It holds the reduced graph, not the miter:
//! each node defined over its fan-ins' representatives (`n ↔ rep(x) ∧ rep(y)`, logged the first time a solver needs
//! it), so the logic two sides share once merged is one variable to it and no chain of equalities.
//!
//! The sweep runs in node order, each node with a representative literal. A node whose fan-ins' representatives make
//! a constant, a copy of one of them, or the pair an earlier node has, is that constant, copy or node, with no solver
//! call (structural hashing over representatives). Any other node is put against the earlier node whose simulation
//! signature it shares: `n ∧ ¬r`, then `¬n ∧ r`, under assumptions, within a conflict budget; a model is a
//! counterexample, added to the simulation to split the class. The budget starts small, since a query that runs out
//! is mostly one the merges around it would make easy: when a round leaves an output pair open, the next sweeps afresh
//! at ten times the budget ([`Options::rounds`]).
//!
//! Every clause made goes into one log with its LRAT hints, in the order made, each hint the id of a clause of the
//! formula or of the log before it: a clause a solver learns with the chain its conflict analysis resolved, a solved
//! merge `(¬n ∨ r)`, `(n ∨ ¬r)` with the chain of the refutation of its assumptions, and a structural merge or a reduced
//! definition with the chain unit propagation finds through the definitions and merges it rests on. A solver's learned
//! clauses that no clause made in its time rests on go with it when it is replaced, since no later clause can name
//! them. An output pair proven equal adds its own two clauses, the claims, and the derivation is the steps the claims
//! reach back through their hints, in order. [`Report::check`] rebuilds the formula from the miter and replays it
//! ([`crate::lrat::check_derivation`]). A difference is a model whose inputs, evaluated, separate the pair.

use std::collections::{HashMap, HashSet};

use crate::aig::{Aig, AigLit, Node};
use crate::cdcl::{BudgetedResult, Lit, Solver};
use crate::lrat::{self, Lrat};
use crate::sweep::{miter, Miter, MiterError, Verdict};

/// How [`decide`] searches.
#[derive(Clone, Copy, Debug)]
pub struct Options {
    /// Random simulation words (64 patterns each) the first signatures are drawn from.
    pub sim_words: usize,
    pub seed: u64,
    /// Conflicts a sweeping query of the first round may spend; 0 is unlimited.
    pub budget: u64,
    /// Conflicts an output pair's query of the last round may spend; 0 is unlimited.
    pub final_budget: u64,
    /// The most rounds a decision takes. Each sweeps afresh at ten times the budget of the one before and puts every
    /// output pair to the solver within a hundred times its budget; the first round that leaves no pair open is the
    /// last, and the last round's pairs are put within `final_budget`. A small first budget wastes no conflict on
    /// queries the merges around them make easy later, and a round left short is no worse than one at its budget.
    pub rounds: usize,
    /// How many classes a node is put against before it stays apart.
    pub attempts: usize,
    /// The variables a solver holds before a fresh one replaces it.
    pub recycle: usize,
    /// How many clauses a solver's part of the log holds before what nothing a later clause can name is let go, and
    /// beyond twice what each collection leaves.
    pub collect: usize,
    /// What to report the search's cost to as it goes, if anything.
    pub progress: Option<Progress>,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            sim_words: 16,
            seed: 0x5eed_c0de_f00d_a16e,
            budget: 30,
            final_budget: 0,
            attempts: 4,
            rounds: 3,
            recycle: 100_000,
            collect: 100_000,
            progress: None,
        }
    }
}

/// A report of a long search: `report` is handed the cost so far once every `every` nodes swept, once the sweep is done,
/// and once the output pairs are, before the derivation is taken from the log.
#[derive(Clone, Copy, Debug)]
pub struct Progress {
    pub every: usize,
    pub report: fn(&Stats),
}

/// What a decision cost.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    /// The miter's nodes, those the pairs reach (all the sweep passes: a node no pair reads cannot help decide one),
    /// and how many of those the sweep has passed.
    pub nodes: usize,
    pub cone: usize,
    pub swept: usize,
    /// The rounds the decision took; the rest of the cost is the last round's.
    pub rounds: usize,
    /// Merges by structure: a constant, a copy, or a pair of representatives an earlier node has.
    pub structural: usize,
    /// Solves under assumptions, the merges they proved, the counterexamples they found and those whose budget ran out.
    pub queries: usize,
    /// Solvers the sweep has used.
    pub solvers: usize,
    pub merges: usize,
    pub counterexamples: usize,
    pub unknown: usize,
    pub conflicts: u64,
    /// The solves by the conflicts each took: none, at most 10, at most 100, at most 1000, more.
    pub by_conflicts: [usize; 5],
    /// Clauses the solvers learned, those the log let go (nothing a later clause could name rested on them), the
    /// clauses and hints the log holds, and the steps of the derivation the claims reach.
    pub learned: usize,
    pub forgotten: usize,
    pub log: usize,
    pub hints: usize,
    pub proof: usize,
}

/// A decided miter: a verdict per output name, the derivation behind the equalities, and the search's cost.
#[derive(Clone, Debug)]
pub struct Report {
    pub miter: Miter,
    pub verdicts: Vec<(String, Verdict)>,
    pub proof: Lrat,
    pub stats: Stats,
}

impl Report {
    /// Every output proven equal.
    pub fn all_equal(&self) -> bool {
        self.verdicts.iter().all(|(_, v)| *v == Verdict::Equal)
    }

    /// The derivation proves every equality claimed, and every difference's assignment separates its pair.
    pub fn check(&self) -> bool {
        let aig = &self.miter.aig;
        let mut claims: Vec<Vec<Lit>> = Vec::new();
        for ((_, a, b), (_, v)) in self.miter.pairs.iter().zip(&self.verdicts) {
            match v {
                Verdict::Equal if a == b => {}
                Verdict::Equal if *a == !*b => return false,
                Verdict::Equal => claims.extend(claim(*a, *b)),
                Verdict::Differ { inputs, latches } => {
                    if inputs.len() != aig.inputs().len() || latches.len() != aig.latches().len() {
                        return false;
                    }
                    let values = aig.eval(inputs, latches);
                    if Aig::value(&values, *a) == Aig::value(&values, *b) {
                        return false;
                    }
                }
                Verdict::Unknown(_) => {}
            }
        }
        lrat::check_derivation(aig.num_nodes(), &formula(aig), &self.proof, &claims)
    }
}

/// An AIG literal as the solver's: node `k` is variable `k`, true when the literal is not complemented.
fn lit(l: AigLit) -> Lit {
    Lit::new(l.node(), !l.is_complemented())
}

/// The two clauses that say `a ≡ b`.
pub(crate) fn claim(a: AigLit, b: AigLit) -> [Vec<Lit>; 2] {
    [vec![lit(a).negated(), lit(b)], vec![lit(a), lit(b).negated()]]
}

/// The miter's formula: for each And node in order its three Tseitin clauses, and node 0 false.
pub fn formula(aig: &Aig) -> Vec<Vec<Lit>> {
    let mut clauses = vec![vec![Lit::neg(0)]];
    for n in 0..aig.num_nodes() as u32 {
        if let Node::And(x, y) = aig.node(n) {
            let o = Lit::pos(n);
            clauses.push(vec![o.negated(), lit(x)]);
            clauses.push(vec![o.negated(), lit(y)]);
            clauses.push(vec![o, lit(x).negated(), lit(y).negated()]);
        }
    }
    clauses
}

/// Pair `left` and `right` by output name and decide every pair.
pub fn equivalent(left: &Aig, right: &Aig, opts: &Options) -> Result<Report, MiterError> {
    let miter = miter(left, right)?;
    let pairs: Vec<(AigLit, AigLit)> = miter.pairs.iter().map(|(_, a, b)| (*a, *b)).collect();
    let (verdicts, proof, stats) = decide(&miter.aig, &pairs, opts);
    let verdicts = miter.pairs.iter().map(|p| p.0.clone()).zip(verdicts).collect();
    Ok(Report { miter, verdicts, proof, stats })
}

/// Decide `a ≡ b` for every pair of literals of `aig`, latches read as free values: a verdict per pair, the derivation
/// of every equality from [`formula`], and the cost.
pub fn decide(aig: &Aig, pairs: &[(AigLit, AigLit)], opts: &Options) -> (Vec<Verdict>, Lrat, Stats) {
    let rounds = opts.rounds.max(1);
    let mut budget = opts.budget;
    for round in 1..=rounds {
        let last = round == rounds;
        let pair_budget = if last { opts.final_budget } else { budget.saturating_mul(100) };
        let (verdicts, proof, stats) = sweep_round(aig, pairs, opts, round, budget, pair_budget);
        if last || !verdicts.iter().any(|v| matches!(v, Verdict::Unknown(_))) {
            return (verdicts, proof, stats);
        }
        budget = budget.saturating_mul(10);
    }
    unreachable!("the last round returns")
}

/// Round `round`: the sweep at `budget` conflicts a query, then every pair within `pair_budget`.
fn sweep_round(aig: &Aig, pairs: &[(AigLit, AigLit)], opts: &Options, round: usize, budget: u64, pair_budget: u64) -> (Vec<Verdict>, Lrat, Stats) {
    let mut e = Engine::new(aig, pairs, opts);
    e.stats.rounds = round;
    e.sweep(opts, budget);
    e.report(opts);
    let mut verdicts = Vec::with_capacity(pairs.len());
    let mut claims: Vec<u64> = Vec::new();
    for &(a, b) in pairs {
        verdicts.push(e.decide_pair(a, b, pair_budget, &mut claims));
    }
    e.sync();
    e.report(opts);
    e.stats.log = e.log.len();
    e.stats.conflicts += e.solver.conflicts();
    let proof = e.derivation(&claims);
    e.stats.proof = proof.steps.len();
    (verdicts, proof, e.stats)
}

/// Simulation values: `words` 64-bit words for each node simulated (the nodes the pairs reach: no other is ever asked
/// for), the last word filling with counterexamples.
struct Sim {
    words: usize,
    inputs: Vec<u64>,
    latches: Vec<u64>,
    /// The nodes simulated in increasing order, and each node's row of values (`u32::MAX` for one not simulated).
    nodes: Vec<u32>,
    row: Vec<u32>,
    values: Vec<u64>,
    /// Patterns of the last word taken by counterexamples (64 once it is full).
    used: usize,
}

impl Sim {
    /// Random values for every input and latch, the same whichever nodes are simulated, and `nodes` (closed under
    /// fan-in, in increasing order) simulated over them.
    fn random(aig: &Aig, words: usize, seed: u64, nodes: Vec<u32>) -> Sim {
        let mut state = seed;
        let mut next = || {
            state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let mut z = state;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            z ^ (z >> 31)
        };
        let inputs: Vec<u64> = (0..aig.inputs().len() * words).map(|_| next()).collect();
        let latches: Vec<u64> = (0..aig.latches().len() * words).map(|_| next()).collect();
        let mut row = vec![u32::MAX; aig.num_nodes()];
        for (i, &n) in nodes.iter().enumerate() {
            row[n as usize] = i as u32;
        }
        let mut sim = Sim { words, inputs, latches, nodes, row, values: Vec::new(), used: 64 };
        sim.run(aig);
        sim
    }

    /// Simulate the nodes held over the inputs' and latches' values.
    fn run(&mut self, aig: &Aig) {
        let words = self.words;
        let mut v = vec![0u64; self.nodes.len() * words];
        for (i, &n) in self.nodes.iter().enumerate() {
            match aig.node(n) {
                Node::False => {}
                Node::Input(k) => v[i * words..(i + 1) * words].copy_from_slice(&self.inputs[k * words..(k + 1) * words]),
                Node::Latch(k) => v[i * words..(i + 1) * words].copy_from_slice(&self.latches[k * words..(k + 1) * words]),
                Node::And(a, b) => {
                    let (ra, rb) = (self.row[a.node() as usize] as usize, self.row[b.node() as usize] as usize);
                    let (ma, mb) = (if a.is_complemented() { !0 } else { 0 }, if b.is_complemented() { !0 } else { 0 });
                    for w in 0..words {
                        v[i * words + w] = (v[ra * words + w] ^ ma) & (v[rb * words + w] ^ mb);
                    }
                }
            }
        }
        self.values = v;
    }

    /// Add one assignment and simulate again: a fresh word carrying it in every pattern, or the next free pattern of
    /// the last word.
    fn add(&mut self, aig: &Aig, inputs: &[bool], latches: &[bool]) {
        if self.used == 64 {
            let grow = |old: &[u64], values: &[bool], words: usize| -> Vec<u64> {
                let mut out = Vec::with_capacity(values.len() * (words + 1));
                for (i, &v) in values.iter().enumerate() {
                    out.extend_from_slice(&old[i * words..(i + 1) * words]);
                    out.push(if v { !0 } else { 0 });
                }
                out
            };
            self.inputs = grow(&self.inputs, inputs, self.words);
            self.latches = grow(&self.latches, latches, self.words);
            self.words += 1;
            self.used = 1;
        } else {
            let (words, bit) = (self.words, self.used);
            for (store, values) in [(&mut self.inputs, inputs), (&mut self.latches, latches)] {
                for (i, &v) in values.iter().enumerate() {
                    let cell = &mut store[i * words + words - 1];
                    *cell = (*cell & !(1 << bit)) | ((v as u64) << bit);
                }
            }
            self.used += 1;
        }
        self.run(aig);
    }

    /// A node's signature with its phase normalised (complemented so the first pattern reads 0), and that phase.
    fn signature(&self, n: u32) -> (Vec<u64>, bool) {
        let at = self.row[n as usize] as usize;
        let row = &self.values[at * self.words..(at + 1) * self.words];
        let phase = row[0] & 1 == 1;
        (row.iter().map(|&w| if phase { !w } else { w }).collect(), phase)
    }
}

/// Where each solver numbers the clauses it learns from: above every id the log gives, so the two never meet.
const LEARNED_IDS: u64 = 1 << 62;


/// A clause of the derivation log: its id, its literals in the miter's numbering, its LRAT hints, and whether a solver
/// learned it.
struct Step {
    id: u64,
    clause: Vec<Lit>,
    hints: Vec<u64>,
    learned: bool,
}

struct Engine<'a> {
    aig: &'a Aig,
    formula: Vec<Vec<Lit>>,
    /// For each And node, the id of the first of its three clauses in the formula.
    def_id: Vec<u64>,
    solver: Solver,
    /// The derivation log after the formula, in id order, the id its next clause takes, and for every id the log has
    /// given (from the formula's last on) its place in the log plus one, 0 once it is let go.
    log: Vec<Step>,
    next_id: u64,
    at: Vec<u32>,
    /// Where the current solver's time in the log begins, the log's id for each clause of the solver's a hint can
    /// still name, and how long the solver's part of the log grows before what nothing can name is let go.
    segment: usize,
    learned: HashMap<u64, u64>,
    collect: usize,
    collect_at: usize,
    /// Each node's definition over its fan-ins' representatives by the ids of its three clauses, once a solver has
    /// needed it.
    defs: Vec<Option<[u64; 3]>>,
    /// Each merged node's two clauses with its representative, `(¬n ∨ r)` then `(n ∨ ¬r)`, by id.
    lemmas: Vec<Option<[u64; 2]>>,
    /// Per node, the current solver's variable for it plus one (0: none yet), and whether its definition is held.
    local: Vec<u32>,
    defined: Vec<bool>,
    /// Per variable of the current solver, its node.
    nodes: Vec<u32>,
    recycle: usize,
    /// Each node's representative literal: itself, or what it was proven equal to.
    repr: Vec<AigLit>,
    /// The node each pair of fan-in representatives was first seen at.
    strash: HashMap<(AigLit, AigLit), u32>,
    /// Nodes both sides of the miter reach: a merge of one proves nothing the pairs need, and an invariant of shared
    /// logic (a carry that never rises) can cost a solver everything, so they are never put to it.
    shared: HashSet<u32>,
    /// Per node, whether either side of the pairs reaches it.
    reached: Vec<bool>,
    sim: Sim,
    stats: Stats,
}

impl<'a> Engine<'a> {
    fn new(aig: &'a Aig, pairs: &[(AigLit, AigLit)], opts: &Options) -> Engine<'a> {
        let left: HashSet<u32> = aig.cone(&pairs.iter().map(|p| p.0).collect::<Vec<_>>()).into_iter().collect();
        let right: HashSet<u32> = aig.cone(&pairs.iter().map(|p| p.1).collect::<Vec<_>>()).into_iter().collect();
        let mut reached = vec![false; aig.num_nodes()];
        for &n in left.iter().chain(&right) {
            reached[n as usize] = true;
        }
        let cone: Vec<u32> = (0..aig.num_nodes() as u32).filter(|&n| reached[n as usize]).collect();
        let formula = formula(aig);
        let mut def_id = vec![0u64; aig.num_nodes()];
        let mut next = 2u64;
        for n in 0..aig.num_nodes() as u32 {
            if let Node::And(..) = aig.node(n) {
                def_id[n as usize] = next;
                next += 3;
            }
        }
        let mut e = Engine {
            aig,
            next_id: formula.len() as u64 + 1,
            at: Vec::new(),
            formula,
            def_id,
            solver: Solver::new(0),
            log: Vec::new(),
            segment: 0,
            learned: HashMap::new(),
            collect: opts.collect.max(1),
            collect_at: opts.collect.max(1),
            defs: vec![None; aig.num_nodes()],
            lemmas: vec![None; aig.num_nodes()],
            local: vec![0; aig.num_nodes()],
            defined: vec![false; aig.num_nodes()],
            nodes: Vec::new(),
            recycle: opts.recycle.max(1),
            repr: (0..aig.num_nodes() as u32).map(|n| AigLit::new(n, false)).collect(),
            strash: HashMap::new(),
            shared: left.intersection(&right).copied().collect(),
            stats: Stats { nodes: aig.num_nodes(), cone: cone.len(), ..Stats::default() },
            reached,
            sim: Sim::random(aig, opts.sim_words.max(1), opts.seed, cone),
        };
        e.fresh();
        e
    }

    /// Where the log holds clause `id`, if it does.
    fn place(&self, id: u64) -> Option<usize> {
        let k = id.checked_sub(self.formula.len() as u64 + 1)?;
        match self.at.get(k as usize) {
            Some(&p) if p > 0 => Some(p as usize - 1),
            _ => None,
        }
    }

    /// Which steps of the log from `from` on the ids of `roots` reach back through their hints. An id the log does not
    /// hold there (the formula's, or one from before `from`) ends its path.
    fn reached(&self, from: usize, roots: impl IntoIterator<Item = u64>) -> Vec<bool> {
        let mut keep = vec![false; self.log.len() - from];
        let mut stack: Vec<u64> = roots.into_iter().collect();
        while let Some(id) = stack.pop() {
            if let Some(k) = self.place(id).and_then(|p| p.checked_sub(from)) {
                if !std::mem::replace(&mut keep[k], true) {
                    stack.extend(&self.log[from + k].hints);
                }
            }
        }
        keep
    }

    /// Let go of the current solver's learned clauses that nothing a later clause can name rests on: no clause the
    /// engine made, and (unless the solver is being replaced) no clause the solver still holds.
    fn collect(&mut self, replacing: bool) {
        let start = self.segment;
        let mut roots: Vec<u64> = self.log[start..].iter().filter(|s| !s.learned).map(|s| s.id).collect();
        let live: HashSet<u64> = if replacing { HashSet::new() } else { self.solver.live_ids().iter().copied().collect() };
        self.learned.retain(|local, _| live.contains(local));
        roots.extend(self.learned.values());
        let mut keep = self.reached(start, roots).into_iter();
        let tail = self.log.split_off(start);
        let held = tail.len();
        let base = self.formula.len() as u64 + 1;
        for (step, kept) in tail.into_iter().zip(&mut keep) {
            let k = (step.id - base) as usize;
            if kept {
                self.at[k] = self.log.len() as u32 + 1;
                self.log.push(step);
            } else {
                self.at[k] = 0;
                self.stats.hints -= step.hints.len();
            }
        }
        self.stats.forgotten += held - (self.log.len() - start);
        self.collect_at = (2 * (self.log.len() - start)).saturating_add(self.collect);
    }

    /// A fresh solver holding only node 0, false. The old one's learned clauses are logged first, and those no clause
    /// the engine made in its time rests on are let go: no later clause can name them.
    fn fresh(&mut self) {
        self.sync();
        self.collect(true);
        self.segment = self.log.len();
        self.collect_at = self.collect;
        self.stats.conflicts += self.solver.conflicts();
        for &n in &self.nodes {
            self.local[n as usize] = 0;
            self.defined[n as usize] = false;
        }
        self.nodes.clear();
        self.learned.clear();
        self.solver = Solver::new(0);
        self.solver.set_lrat(true);
        self.solver.set_next_clause_id(LEARNED_IDS);
        self.solver.set_decision_vars(&[]);
        let zero = self.var(0);
        self.solver.add_clause_with_id(vec![Lit::neg(zero)], 1);
        self.defined[0] = true;
        self.stats.solvers += 1;
    }

    /// The current solver's variable for node `n`, made a decision variable the first time it is asked for.
    fn var(&mut self, n: u32) -> u32 {
        if self.local[n as usize] == 0 {
            let v = self.solver.add_vars(1);
            self.solver.add_decision_vars(&[v]);
            self.local[n as usize] = v as u32 + 1;
            self.nodes.push(n);
        }
        self.local[n as usize] - 1
    }

    /// A literal of the miter as the current solver's, its node given a variable if it had none.
    fn to_local(&mut self, l: Lit) -> Lit {
        Lit::new(self.var(l.var()), l.is_positive())
    }

    /// The clause the formula or the log has under `id`.
    fn clause_of(&self, id: u64) -> Option<&[Lit]> {
        if id == 0 {
            return None;
        }
        match self.formula.get(id as usize - 1) {
            Some(c) => Some(c),
            None => self.place(id).map(|k| self.log[k].clause.as_slice()),
        }
    }

    /// Give the solver the log's clause `id`, under that id.
    fn give(&mut self, id: u64) {
        let Some(clause) = self.clause_of(id).map(<[Lit]>::to_vec) else { return };
        let local: Vec<Lit> = clause.into_iter().map(|l| self.to_local(l)).collect();
        self.solver.add_clause_with_id(local, id);
    }

    /// Log `clause` with `hints`; its id.
    fn push(&mut self, clause: Vec<Lit>, hints: Vec<u64>, learned: bool) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.at.push(self.log.len() as u32 + 1);
        self.stats.hints += hints.len();
        self.log.push(Step { id, clause, hints, learned });
        id
    }

    /// A hint of the current solver's as the log's id. A learned clause the log has not met maps to 0, which no clause
    /// has, so the derivation then does not check.
    fn global(&self, hint: u64) -> u64 {
        if hint >= LEARNED_IDS {
            self.learned.get(&hint).copied().unwrap_or(0)
        } else {
            hint
        }
    }

    /// Log the clauses the solver learned since the log last looked, each with its chain in the log's ids, and let go
    /// of what nothing can name once the solver's part of the log has doubled.
    fn sync(&mut self) {
        let learned = self.solver.take_learned();
        self.stats.learned += learned.len();
        for c in learned {
            let mut clause = c.lits;
            for l in &mut clause {
                *l = Lit::new(self.nodes[l.var() as usize], l.is_positive());
            }
            let mut hints = c.hints;
            for h in &mut hints {
                *h = self.global(*h);
            }
            let id = self.push(clause, hints, true);
            self.learned.insert(c.id, id);
        }
        if self.log.len() - self.segment >= self.collect_at {
            self.collect(false);
        }
    }

    /// The LRAT chain of `clause` through `candidates`: under the clause's negation, each candidate in the order it
    /// turns unit, ending at the first that conflicts. `None` when propagation through them stops short of a conflict.
    fn chain(&self, clause: &[Lit], candidates: &[u64]) -> Option<Vec<u64>> {
        let mut value: HashMap<u32, bool> = clause.iter().map(|l| (l.var(), !l.is_positive())).collect();
        let mut used = vec![false; candidates.len()];
        let mut hints = Vec::new();
        loop {
            let mut progressed = false;
            for (k, &id) in candidates.iter().enumerate() {
                let Some(c) = self.clause_of(id).filter(|_| !used[k]) else { continue };
                let mut open: Option<Lit> = None;
                let mut quiet = false;
                for &l in c {
                    match value.get(&l.var()) {
                        Some(&v) if v == l.is_positive() => quiet = true,
                        Some(_) => {}
                        None if open.is_none() || open == Some(l) => open = Some(l),
                        None => quiet = true,
                    }
                    if quiet {
                        break;
                    }
                }
                if quiet {
                    continue;
                }
                used[k] = true;
                hints.push(id);
                let Some(u) = open else { return Some(hints) };
                value.insert(u.var(), u.is_positive());
                progressed = true;
            }
            if !progressed {
                return None;
            }
        }
    }

    /// Log `clause` with its chain through `candidates`; its id. Every clause the engine derives this way follows from
    /// what it names, so a chain not found is the engine's fault, and the derivation then does not check.
    fn derive(&mut self, clause: Vec<Lit>, candidates: &[u64]) -> u64 {
        let hints = self.chain(&clause, candidates).unwrap_or_default();
        self.sync();
        self.push(clause, hints, false)
    }

    /// The ids of what says what node `n` is: its three clauses in the formula, and its merge with its representative.
    fn about(&self, n: u32) -> impl Iterator<Item = u64> + '_ {
        let f = self.def_id[n as usize];
        let definition = matches!(self.aig.node(n), Node::And(..)).then_some([f, f + 1, f + 2]);
        definition.into_iter().flatten().chain(self.lemmas[n as usize].into_iter().flatten())
    }

    /// The ids of what says what each of `nodes` and their fan-ins is, and node 0 false.
    fn around(&self, nodes: &[u32]) -> Vec<u64> {
        let mut near: Vec<u32> = nodes.to_vec();
        for &n in nodes {
            if let Node::And(x, y) = self.aig.node(n) {
                near.extend([x.node(), y.node()]);
            }
        }
        near.sort_unstable();
        near.dedup();
        let mut ids = vec![1];
        for n in near {
            ids.extend(self.about(n));
        }
        ids
    }

    /// Give the solver the definitions of every node of the reduced cones of `roots` it does not hold yet: each node's
    /// three clauses over its fan-ins' representatives, logged with their chains the first time a solver needs them.
    fn encode(&mut self, roots: &[AigLit]) {
        let mut stack: Vec<u32> = roots.iter().map(|l| l.node()).collect();
        while let Some(n) = stack.pop() {
            self.var(n);
            if std::mem::replace(&mut self.defined[n as usize], true) {
                continue;
            }
            if let Node::And(x, y) = self.aig.node(n) {
                let (rx, ry) = (self.resolve(x), self.resolve(y));
                let ids = match self.defs[n as usize] {
                    Some(ids) => ids,
                    None => {
                        let o = Lit::pos(n);
                        let near = self.around(&[n]);
                        let ids = [
                            self.derive(vec![o.negated(), lit(rx)], &near),
                            self.derive(vec![o.negated(), lit(ry)], &near),
                            self.derive(vec![o, lit(rx).negated(), lit(ry).negated()], &near),
                        ];
                        self.defs[n as usize] = Some(ids);
                        ids
                    }
                };
                for id in ids {
                    self.give(id);
                }
                stack.push(rx.node());
                stack.push(ry.node());
            }
        }
    }

    /// Hand the cost so far to the progress report, if there is one.
    fn report(&self, opts: &Options) {
        if let Some(p) = opts.progress {
            let mut so_far = self.stats.clone();
            so_far.conflicts += self.solver.conflicts();
            so_far.log = self.log.len();
            (p.report)(&so_far);
        }
    }

    fn resolve(&self, l: AigLit) -> AigLit {
        self.repr[l.node() as usize].with_phase(l.is_complemented())
    }

    /// Record `n ≡ t` by structure, `t` the representative of `n`: both clauses logged with their chains through the
    /// definitions and merges of `n`, of `m` (the earlier node with the same pair of representatives, when there is
    /// one) and of their fan-ins. A solver never needs them: it holds the reduced graph, where `n` is `t`.
    fn merge(&mut self, n: u32, t: AigLit, m: Option<u32>) {
        let near = self.around(&[n].into_iter().chain(m).collect::<Vec<_>>());
        let [a, b] = claim(AigLit::new(n, false), t);
        self.lemmas[n as usize] = Some([self.derive(a, &near), self.derive(b, &near)]);
        self.repr[n as usize] = t;
    }

    /// `a ≡ b` put to the solver within `budget`: both clauses, each the negation of the assumptions an unsatisfiable
    /// solve refuted, logged with the refutation's chain and given to it; a model of the miter separating the pair; or
    /// the budget run out.
    fn prove(&mut self, a: AigLit, b: AigLit, budget: u64) -> Proved {
        if self.nodes.len() >= self.recycle {
            self.fresh();
        }
        self.encode(&[a, b]);
        let mut ids = [0u64; 2];
        for (k, (x, y)) in [(a, !b), (!a, b)].into_iter().enumerate() {
            self.stats.queries += 1;
            let before = self.solver.conflicts();
            let assumptions = [self.to_local(lit(x)), self.to_local(lit(y))];
            let result = self.solver.solve_under_assumptions_budgeted(&assumptions, budget);
            let took = self.solver.conflicts() - before;
            self.stats.by_conflicts[[0, 10, 100, 1000].iter().take_while(|&&b| took > b).count()] += 1;
            match result {
                BudgetedResult::Unsat => {
                    self.sync();
                    let hints: Vec<u64> = self.solver.unsat_hints().iter().map(|&h| self.global(h)).collect();
                    ids[k] = self.push(vec![lit(x).negated(), lit(y).negated()], hints, false);
                    self.give(ids[k]);
                }
                BudgetedResult::Sat(model) => {
                    let value = |n: u32| self.local[n as usize] > 0 && model[self.local[n as usize] as usize - 1];
                    let inputs: Vec<bool> = self.aig.inputs().iter().map(|p| value(p.lit.node())).collect();
                    let latches: Vec<bool> = (0..self.aig.latches().len()).map(|j| value(self.aig.latch_lit(j).node())).collect();
                    let values = self.aig.eval(&inputs, &latches);
                    return if Aig::value(&values, a) != Aig::value(&values, b) {
                        Proved::Differ(inputs, latches)
                    } else {
                        Proved::Unknown("the solver's model does not separate the pair".into())
                    };
                }
                BudgetedResult::Budget => return Proved::Unknown(format!("no verdict within {budget} conflicts")),
            }
        }
        Proved::Equal(ids)
    }

    /// What node `n` is by structure: a constant, one of its fan-ins' representatives, or the representative of the
    /// earlier node with the same pair of them (and that node).
    fn structural(&mut self, n: u32, x: AigLit, y: AigLit) -> Option<(AigLit, Option<u32>)> {
        let (rx, ry) = (self.resolve(x), self.resolve(y));
        if rx == AigLit::FALSE || ry == AigLit::FALSE || rx == !ry {
            return Some((AigLit::FALSE, None));
        }
        if rx == AigLit::TRUE || rx == ry {
            return Some((ry, None));
        }
        if ry == AigLit::TRUE {
            return Some((rx, None));
        }
        let key = if rx.code() <= ry.code() { (rx, ry) } else { (ry, rx) };
        match self.strash.get(&key) {
            Some(&m) => Some((self.resolve(AigLit::new(m, false)), Some(m))),
            None => {
                self.strash.insert(key, n);
                None
            }
        }
    }

    fn sweep(&mut self, opts: &Options, budget: u64) {
        let aig = self.aig;
        // Each simulation class by its normalised signature, its representative the first node seen with it.
        let mut classes: HashMap<Vec<u64>, u32> = HashMap::new();
        let mut reps: Vec<u32> = Vec::new();
        for n in 0..aig.num_nodes() as u32 {
            if !self.reached[n as usize] {
                continue;
            }
            self.stats.swept += 1;
            if opts.progress.is_some_and(|p| p.every > 0 && self.stats.swept % p.every == 0) {
                self.report(opts);
            }
            if let Node::And(x, y) = aig.node(n) {
                if let Some((t, m)) = self.structural(n, x, y) {
                    self.merge(n, t, m);
                    self.stats.structural += 1;
                    continue;
                }
            }
            let candidate = matches!(aig.node(n), Node::And(..)) && !self.shared.contains(&n);
            let mut attempts = 0;
            loop {
                let (key, phase) = self.sim.signature(n);
                let Some(&r) = classes.get(&key) else {
                    classes.insert(key, n);
                    break;
                };
                if !candidate || attempts == opts.attempts {
                    break;
                }
                attempts += 1;
                let (_, phase_r) = self.sim.signature(r);
                let target = AigLit::new(r, phase ^ phase_r);
                match self.prove(AigLit::new(n, false), target, budget) {
                    Proved::Equal(ids) => {
                        self.lemmas[n as usize] = Some(ids);
                        self.repr[n as usize] = target;
                        self.stats.merges += 1;
                        break;
                    }
                    Proved::Differ(inputs, latches) => {
                        self.stats.counterexamples += 1;
                        self.sim.add(aig, &inputs, &latches);
                        classes.clear();
                        for &m in &reps {
                            classes.entry(self.sim.signature(m).0).or_insert(m);
                        }
                    }
                    Proved::Unknown(_) => {
                        self.stats.unknown += 1;
                        break;
                    }
                }
            }
            if self.repr[n as usize] == AigLit::new(n, false) {
                reps.push(n);
            }
        }
    }

    /// One output pair's verdict, the ids of its two claim clauses pushed to `claims` when it is equal: each chained
    /// through the outputs' merges and, when their representatives differ, the merge proven between those.
    fn decide_pair(&mut self, a: AigLit, b: AigLit, budget: u64, claims: &mut Vec<u64>) -> Verdict {
        if a == b {
            return Verdict::Equal;
        }
        let (ra, rb) = (self.resolve(a), self.resolve(b));
        let mut near: Vec<u64> = vec![1];
        near.extend(self.lemmas[a.node() as usize].into_iter().flatten());
        near.extend(self.lemmas[b.node() as usize].into_iter().flatten());
        if ra != rb {
            match self.prove(ra, rb, budget) {
                Proved::Equal(ids) => near.extend(ids),
                Proved::Differ(inputs, latches) => return Verdict::Differ { inputs, latches },
                Proved::Unknown(why) => {
                    self.stats.unknown += 1;
                    return Verdict::Unknown(why);
                }
            }
        }
        let [x, y] = claim(a, b);
        claims.push(self.derive(x, &near));
        claims.push(self.derive(y, &near));
        Verdict::Equal
    }

    /// The steps of the log the claims reach back through their hints, in order.
    fn derivation(&self, claims: &[u64]) -> Lrat {
        let keep = self.reached(0, claims.iter().copied());
        let steps = self
            .log
            .iter()
            .zip(keep)
            .filter(|(_, k)| *k)
            .map(|(s, _)| lrat::LratStep { id: s.id, clause: s.clause.clone(), hints: s.hints.clone() })
            .collect();
        Lrat { steps }
    }
}

/// What [`Engine::prove`] found.
enum Proved {
    /// Both clauses logged, by their ids.
    Equal([u64; 2]),
    Differ(Vec<bool>, Vec<bool>),
    Unknown(String),
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_proof/fraig/tests.rs"]
mod tests;
