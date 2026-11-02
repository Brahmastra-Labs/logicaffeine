//! Combinational equivalence: two circuits' outputs proven equal, or told apart by an input assignment that replays,
//! every equality carried by a lemma whose LRAT proof a small checker re-validates.
//!
//! Two circuits are joined in one and-inverter graph by [`miter`]: inputs of the same name shared, outputs paired by
//! name. Structural hashing already merges every gate both compute the same way. [`decide`] then sweeps the rest
//! (Kuehlmann, *Dynamic transition relation simplification for bounded property checking*, ICCAD 2004; Mishchenko
//! et al., *FRAIGs*, 2005): random simulation groups nodes by their value signatures, each candidate pair is put to
//! the CDCL solver in topological order, a refuted difference becomes a lemma that later queries may use, and a
//! counterexample is added to the simulation patterns to split the classes it disproves. Each output pair is then
//! equal by the lemmas, or decided by one more query.
//!
//! A lemma's formula is [`query_with_cut`]: the Tseitin clauses of the two literals' cone, cut at the nodes the
//! lemma names (left free, their own fan-in not encoded), the two clauses of each earlier lemma lying wholly inside
//! that cone, and the difference itself. Leaving a cut free only removes constraints, so a refutation of the cut
//! formula refutes the whole cone's; a candidate is first put to the solver a few levels deep, where an equality
//! implied by its fan-ins' lemmas refutes at once, and only widened to the whole cone when the narrow formula has a
//! model. The formula is a pure function of the graph, the cut and the lemmas before it, so [`check`] rebuilds it,
//! replays the lemma's LRAT proof with [`crate::lrat::check`], and accepts a claimed equality only when the lemmas
//! join its two literals, with matching phase, in a union-find. A solver whose UNSAT does not replay yields
//! [`Verdict::Unknown`], never an equality; a model that does not separate the two literals under [`Aig::eval`]
//! likewise.
//!
//! [`distinguish`] tells a family of circuits apart: the members that compute one function, each equality certified,
//! and a few input assignments on which every two members that differ give different outputs, with each class's
//! outputs there: what names the member a black box computes from its measured outputs.

use std::collections::{HashMap, HashSet};

use crate::aig::{Aig, AigLit, Leaf, Node};
use crate::cdcl::{BudgetedResult, Lit, Solver};
use crate::lrat::{self, Lrat};

/// A proven equivalence `a ≡ b`: its proof refutes [`query_with_cut`]`(a, b, cut, earlier lemmas)`.
#[derive(Clone, Debug)]
pub struct Lemma {
    pub a: AigLit,
    pub b: AigLit,
    /// The nodes the formula leaves free, in increasing order; empty for the whole cone.
    pub cut: Vec<u32>,
    pub proof: Lrat,
}

/// Lemmas in proof order; a lemma's query includes the lemmas before it.
#[derive(Clone, Debug, Default)]
pub struct Certificate {
    pub lemmas: Vec<Lemma>,
}

/// What [`decide`] found for one pair of literals.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    Equal,
    /// An assignment, of the inputs by index and the latches by index, under which the two literals differ.
    Differ { inputs: Vec<bool>, latches: Vec<bool> },
    /// No verdict: the conflict budget ran out, or the solver's answer did not survive its own check.
    Unknown(String),
}

/// How [`decide`] searches.
#[derive(Clone, Debug)]
pub struct Options {
    /// Sweep internal nodes before deciding the pairs.
    pub sweep: bool,
    /// Random simulation words (64 patterns each) the first signatures are drawn from.
    pub sim_words: usize,
    pub seed: u64,
    /// Conflicts a sweeping query may spend; 0 is unlimited.
    pub sweep_budget: u64,
    /// Conflicts a pair's own query may spend; 0 is unlimited.
    pub final_budget: u64,
    /// What to report the search's cost to as it goes, if anything.
    pub progress: Option<Progress>,
}

/// A report of a long search: `report` is handed the cost so far once every `every` candidates put to the solver.
#[derive(Clone, Copy, Debug)]
pub struct Progress {
    pub every: usize,
    pub report: fn(&Stats),
}

impl Default for Options {
    fn default() -> Options {
        Options { sweep: true, sim_words: 8, seed: 0x5eed_c0de_f00d_a16e, sweep_budget: 20_000, final_budget: 0, progress: None }
    }
}

/// What a decision cost.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub sat_calls: usize,
    pub merges: usize,
    pub counterexamples: usize,
    pub unknown: usize,
    /// The queries that took the most conflicts, most first, at most [`HARDEST`].
    pub hardest: Vec<Hard>,
    /// Every query's cost by where it was cut: at each of [`CUT_DEPTHS`] in order, where the two cones meet, then over
    /// the whole cone.
    pub by_cut: [CutCost; CUT_KINDS],
    /// The nodes of the cone the sweep runs over, and how many it has put to classes.
    pub cone: usize,
    pub swept: usize,
}

/// The queries put at one kind of cut, their verdicts and what they cost, summed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CutCost {
    pub queries: usize,
    pub refuted: usize,
    pub models: usize,
    pub budget: usize,
    pub vars: u64,
    pub clauses: u64,
    pub conflicts: u64,
}

/// One query's cost: the pair, the cut it was put with (empty for the whole cone), its formula's size and the
/// conflicts its solve took.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hard {
    pub a: AigLit,
    pub b: AigLit,
    pub cut: usize,
    pub vars: usize,
    pub conflicts: u64,
    pub verdict: &'static str,
}

/// How many of the hardest queries [`Stats`] keeps.
pub const HARDEST: usize = 16;

impl Stats {
    fn note(&mut self, h: Hard) {
        let at = self.hardest.partition_point(|x| x.conflicts >= h.conflicts);
        if at < HARDEST {
            self.hardest.insert(at, h);
            self.hardest.truncate(HARDEST);
        }
    }
}

/// A lemma's formula: variable `k` is the `k`-th node of `cone` (the two literals' cone, in increasing order), then
/// of `extra` (nodes outside the cone an earlier lemma relates to a node inside it, free, in increasing order).
#[derive(Clone, Debug)]
pub struct Query {
    pub cone: Vec<u32>,
    pub extra: Vec<u32>,
    pub num_vars: usize,
    pub clauses: Vec<Vec<Lit>>,
}

/// The cone of `roots`, a node of `cut` kept as a leaf, in increasing order.
fn cone_within(aig: &Aig, roots: &[AigLit], cut: &[u32]) -> Vec<u32> {
    let mut seen: HashSet<u32> = HashSet::new();
    let mut stack: Vec<u32> = roots.iter().map(|l| l.node()).collect();
    let mut out = Vec::new();
    while let Some(n) = stack.pop() {
        if !seen.insert(n) {
            continue;
        }
        out.push(n);
        if cut.binary_search(&n).is_ok() {
            continue;
        }
        if let Node::And(x, y) = aig.node(n) {
            stack.push(x.node());
            stack.push(y.node());
        }
    }
    out.sort_unstable();
    out
}

/// Lemmas in proof order, each node of one indexed by the lemmas that relate it: the lemmas touching a cone found in
/// time proportional to the cone, not to every lemma, in lemma order.
#[derive(Clone, Debug, Default)]
pub struct Earlier {
    pairs: Vec<(AigLit, AigLit)>,
    by_node: HashMap<u32, Vec<u32>>,
}

impl Earlier {
    pub fn new() -> Earlier {
        Earlier::default()
    }

    /// Add the lemma `a ≡ b` after every one before it.
    pub fn push(&mut self, a: AigLit, b: AigLit) {
        let k = self.pairs.len() as u32;
        self.pairs.push((a, b));
        self.by_node.entry(a.node()).or_default().push(k);
        if b.node() != a.node() {
            self.by_node.entry(b.node()).or_default().push(k);
        }
    }

    pub fn pairs(&self) -> &[(AigLit, AigLit)] {
        &self.pairs
    }

    /// The lemmas with a literal on a node of `cone`, in lemma order.
    fn touching(&self, cone: &[u32]) -> Vec<(AigLit, AigLit)> {
        let mut at: Vec<u32> = cone.iter().filter_map(|n| self.by_node.get(n)).flatten().copied().collect();
        at.sort_unstable();
        at.dedup();
        at.into_iter().map(|k| self.pairs[k as usize]).collect()
    }
}

impl From<&[(AigLit, AigLit)]> for Earlier {
    fn from(pairs: &[(AigLit, AigLit)]) -> Earlier {
        let mut e = Earlier::new();
        for &(a, b) in pairs {
            e.push(a, b);
        }
        e
    }
}

/// [`query_with_cut`] over the whole cone.
pub fn query(aig: &Aig, a: AigLit, b: AigLit, earlier: &[(AigLit, AigLit)]) -> Query {
    query_with_cut(aig, a, b, &[], earlier)
}

/// The Tseitin clauses of the cone of `a` and `b` in node order, a node of `cut` (sorted) left free, then the two
/// clauses of each earlier lemma with a literal in that cone, in lemma order (its other node, when outside the cone,
/// a free variable of its own: lemmas joined through a representative the cone does not reach still join), then
/// `a ≠ b` as `(a ∨ b) ∧ (¬a ∨ ¬b)`.
pub fn query_with_cut(aig: &Aig, a: AigLit, b: AigLit, cut: &[u32], earlier: &[(AigLit, AigLit)]) -> Query {
    query_indexed(aig, a, b, cut, &Earlier::from(earlier))
}

/// [`query_with_cut`] with the earlier lemmas indexed.
fn query_indexed(aig: &Aig, a: AigLit, b: AigLit, cut: &[u32], earlier: &Earlier) -> Query {
    let cone = cone_within(aig, &[a, b], cut);
    let inside = |n: u32| cone.binary_search(&n).is_ok();
    let touching = earlier.touching(&cone);
    let mut extra: Vec<u32> =
        touching.iter().flat_map(|(p, q)| [p.node(), q.node()]).filter(|&n| !inside(n)).collect();
    extra.sort_unstable();
    extra.dedup();
    let var = |n: u32| match cone.binary_search(&n) {
        Ok(k) => k,
        Err(_) => cone.len() + extra.binary_search(&n).expect("a node of the cone or of a lemma touching it"),
    };
    let lit = |l: AigLit| Lit::new(var(l.node()) as u32, !l.is_complemented());
    let mut clauses = Vec::with_capacity(3 * cone.len() + 2 * touching.len() + 2);
    for (k, &n) in cone.iter().enumerate() {
        if cut.binary_search(&n).is_ok() {
            continue;
        }
        match aig.node(n) {
            Node::False => clauses.push(vec![Lit::neg(k as u32)]),
            Node::And(x, y) => {
                let o = Lit::pos(k as u32);
                clauses.push(vec![o.negated(), lit(x)]);
                clauses.push(vec![o.negated(), lit(y)]);
                clauses.push(vec![o, lit(x).negated(), lit(y).negated()]);
            }
            Node::Input(_) | Node::Latch(_) => {}
        }
    }
    for (p, q) in touching {
        clauses.push(vec![lit(p).negated(), lit(q)]);
        clauses.push(vec![lit(p), lit(q).negated()]);
    }
    clauses.push(vec![lit(a), lit(b)]);
    clauses.push(vec![lit(a).negated(), lit(b).negated()]);
    Query { num_vars: cone.len() + extra.len(), cone, extra, clauses }
}

/// The And nodes first reached `depth` levels back from the roots, in increasing order: the cut that keeps a query
/// that many levels deep. Empty when the cone is shallower.
fn frontier(aig: &Aig, roots: &[AigLit], depth: usize) -> Vec<u32> {
    let mut level: HashMap<u32, usize> = HashMap::new();
    let mut current: Vec<u32> = roots.iter().map(|l| l.node()).collect();
    for &n in &current {
        level.insert(n, 0);
    }
    for d in 1..=depth {
        let mut next = Vec::new();
        for &n in &current {
            if let Node::And(x, y) = aig.node(n) {
                for c in [x.node(), y.node()] {
                    if !level.contains_key(&c) {
                        level.insert(c, d);
                        next.push(c);
                    }
                }
            }
        }
        current = next;
    }
    let mut cut: Vec<u32> = current.into_iter().filter(|&n| matches!(aig.node(n), Node::And(..))).collect();
    cut.sort_unstable();
    cut
}

pub(crate) enum Outcome {
    Equal(Lrat),
    Differ(Vec<bool>, Vec<bool>),
    Unknown(String),
}

/// [`query`] decided: refuted with a checked proof, a model that separates the pair under [`Aig::eval`], or neither.
pub(crate) fn prove(aig: &Aig, a: AigLit, b: AigLit, earlier: &[(AigLit, AigLit)], budget: u64) -> Outcome {
    prove_noted(aig, a, b, &[], &Earlier::from(earlier), budget, None)
}

/// `a ≡ b` decided over the cone cut at `cut`: refuted with a checked proof, a model that separates the pair under
/// [`Aig::eval`], or neither; its cost noted in `noted`'s stats, under the kind of cut given there, when given.
fn prove_noted(
    aig: &Aig,
    a: AigLit,
    b: AigLit,
    cut: &[u32],
    earlier: &Earlier,
    budget: u64,
    noted: Option<(&mut Stats, usize)>,
) -> Outcome {
    let q = query_indexed(aig, a, b, cut, earlier);
    let mut solver = Solver::new(q.num_vars);
    for c in &q.clauses {
        solver.add_clause(c.clone());
    }
    let result = solver.solve_budgeted(budget);
    if let Some((stats, kind)) = noted {
        let cost = &mut stats.by_cut[kind];
        cost.queries += 1;
        cost.vars += q.num_vars as u64;
        cost.clauses += q.clauses.len() as u64;
        cost.conflicts += solver.conflicts();
        let verdict = match &result {
            BudgetedResult::Sat(_) => {
                cost.models += 1;
                "model"
            }
            BudgetedResult::Unsat => {
                cost.refuted += 1;
                "refuted"
            }
            BudgetedResult::Budget => {
                cost.budget += 1;
                "budget"
            }
        };
        stats.note(Hard { a, b, cut: cut.len(), vars: q.num_vars, conflicts: solver.conflicts(), verdict });
    }
    match result {
        BudgetedResult::Sat(model) => {
            let mut inputs = vec![false; aig.inputs().len()];
            let mut latches = vec![false; aig.latches().len()];
            for (k, &n) in q.cone.iter().enumerate() {
                match aig.node(n) {
                    Node::Input(i) => inputs[i] = model[k],
                    Node::Latch(j) => latches[j] = model[k],
                    _ => {}
                }
            }
            let v = aig.eval(&inputs, &latches);
            if Aig::value(&v, a) != Aig::value(&v, b) {
                Outcome::Differ(inputs, latches)
            } else {
                Outcome::Unknown("the solver's model does not separate the pair".into())
            }
        }
        BudgetedResult::Unsat => {
            let learned: Vec<Vec<Lit>> = solver.learned().iter().map(|c| c.lits.clone()).collect();
            match lrat::refutation(q.num_vars, &q.clauses, &learned) {
                Some(proof) if lrat::check(q.num_vars, &q.clauses, &proof) => Outcome::Equal(proof),
                _ => Outcome::Unknown("the solver's refutation does not replay".into()),
            }
        }
        BudgetedResult::Budget => Outcome::Unknown(format!("no verdict within {budget} conflicts")),
    }
}

/// The depths a candidate is first tried at before its whole cone.
pub const CUT_DEPTHS: [usize; 2] = [6, 16];

/// The kinds of cut a query is put at: each of [`CUT_DEPTHS`], where the cones meet, and none.
pub const CUT_KINDS: usize = CUT_DEPTHS.len() + 2;

/// Where the cones of `a` and `b` meet, as each first reaches it: an And node whose class (by `repr`, the classes
/// proven so far) both cones hold a member of, whether the node itself is in both or a proven equal of it is. A
/// rewrite of the same logic, a pipeline of it or a mapping of it differs only above where its two sides meet, so a
/// query cut there sees only what differs. Empty when the cones never meet.
fn meeting(aig: &Aig, a: AigLit, b: AigLit, repr: &[AigLit]) -> Vec<u32> {
    let class = |n: u32| repr[n as usize].node();
    let classes = |r: AigLit| -> HashSet<u32> { cone_within(aig, &[r], &[]).into_iter().map(class).collect() };
    let (left, right) = (classes(a), classes(b));
    let met = |n: u32| {
        let c = class(n);
        left.contains(&c) && right.contains(&c)
    };
    let mut cut = Vec::new();
    let mut seen: HashSet<u32> = HashSet::new();
    let mut stack = vec![a.node(), b.node()];
    while let Some(n) = stack.pop() {
        if !seen.insert(n) {
            continue;
        }
        let Node::And(x, y) = aig.node(n) else { continue };
        if n != a.node() && n != b.node() && met(n) {
            cut.push(n);
            continue;
        }
        stack.push(x.node());
        stack.push(y.node());
    }
    cut.sort_unstable();
    cut
}

/// `a ≡ b` decided narrow first: at each of [`CUT_DEPTHS`] (a model there may be the free cut's alone, so it only
/// widens the search), then where the two cones [`meeting`] meet, then over the whole cone. An equality comes with the
/// cut its proof refutes. Each cut is found only once every narrower one has failed: where the cones meet costs both
/// whole cones to find.
fn prove_local(
    aig: &Aig,
    a: AigLit,
    b: AigLit,
    earlier: &Earlier,
    repr: &[AigLit],
    budget: u64,
    stats: &mut Stats,
) -> (Outcome, Vec<u32>) {
    let mut tried: Vec<Vec<u32>> = Vec::new();
    for (kind, &depth) in CUT_DEPTHS.iter().enumerate() {
        let cut = frontier(aig, &[a, b], depth);
        if cut.is_empty() {
            break;
        }
        match prove_noted(aig, a, b, &cut, earlier, budget, Some((&mut *stats, kind))) {
            Outcome::Equal(proof) => return (Outcome::Equal(proof), cut),
            Outcome::Differ(inputs, latches) => return (Outcome::Differ(inputs, latches), Vec::new()),
            Outcome::Unknown(_) => tried.push(cut),
        }
    }
    let met = meeting(aig, a, b, repr);
    if !met.is_empty() && !tried.contains(&met) {
        match prove_noted(aig, a, b, &met, earlier, budget, Some((&mut *stats, CUT_DEPTHS.len()))) {
            Outcome::Equal(proof) => return (Outcome::Equal(proof), met),
            Outcome::Differ(inputs, latches) => return (Outcome::Differ(inputs, latches), Vec::new()),
            Outcome::Unknown(_) => {}
        }
    }
    (prove_noted(aig, a, b, &[], earlier, budget, Some((stats, CUT_KINDS - 1))), Vec::new())
}

/// Hand the cost so far to `opts`'s progress report, when one is due.
fn tick(stats: &Stats, opts: &Options) {
    if let Some(p) = opts.progress {
        if p.every > 0 && stats.sat_calls % p.every == 0 {
            (p.report)(stats);
        }
    }
}

/// An input and latch assignment under which `lit` is true, or `None` when it never is (or the budget ran out first:
/// `Err`). Uncertified: a search for a witness, whose finding is replayed by evaluation before it is returned.
pub(crate) fn witness(aig: &Aig, lit: AigLit, budget: u64) -> Result<Option<(Vec<bool>, Vec<bool>)>, String> {
    let q = query(aig, lit, AigLit::FALSE, &[]);
    let mut solver = Solver::new(q.num_vars);
    for c in &q.clauses {
        solver.add_clause(c.clone());
    }
    match solver.solve_budgeted(budget) {
        BudgetedResult::Sat(model) => {
            let mut inputs = vec![false; aig.inputs().len()];
            let mut latches = vec![false; aig.latches().len()];
            for (k, &n) in q.cone.iter().enumerate() {
                match aig.node(n) {
                    Node::Input(i) => inputs[i] = model[k],
                    Node::Latch(j) => latches[j] = model[k],
                    _ => {}
                }
            }
            let v = aig.eval(&inputs, &latches);
            if Aig::value(&v, lit) {
                Ok(Some((inputs, latches)))
            } else {
                Err("the solver's model does not make the literal true".into())
            }
        }
        BudgetedResult::Unsat => Ok(None),
        BudgetedResult::Budget => Err(format!("no verdict within {budget} conflicts")),
    }
}

/// [`decide`]'s verdict for each pair, the certificate behind its equalities, and its cost.
#[derive(Clone, Debug)]
pub struct Decided {
    pub verdicts: Vec<Verdict>,
    pub certificate: Certificate,
    pub stats: Stats,
}

/// Decide `a ≡ b` for every pair of literals of `aig`, latches read as free values.
pub fn decide(aig: &Aig, pairs: &[(AigLit, AigLit)], opts: &Options) -> Decided {
    let mut s = Sweeper {
        aig,
        repr: (0..aig.num_nodes() as u32).map(|n| AigLit::new(n, false)).collect(),
        lemmas: Vec::new(),
        earlier: Earlier::new(),
        stats: Stats::default(),
    };
    if opts.sweep {
        s.sweep(pairs, opts);
    }
    let mut verdicts = Vec::with_capacity(pairs.len());
    for &(a, b) in pairs {
        if s.resolve(a) == s.resolve(b) {
            verdicts.push(Verdict::Equal);
            continue;
        }
        s.stats.sat_calls += 1;
        let (outcome, cut) = prove_local(aig, a, b, &s.earlier, &s.repr, opts.final_budget, &mut s.stats);
        tick(&s.stats, opts);
        verdicts.push(match outcome {
            Outcome::Equal(proof) => {
                s.learn(a, b, cut, proof);
                Verdict::Equal
            }
            Outcome::Differ(inputs, latches) => Verdict::Differ { inputs, latches },
            Outcome::Unknown(why) => {
                s.stats.unknown += 1;
                Verdict::Unknown(why)
            }
        });
    }
    Decided { verdicts, certificate: Certificate { lemmas: s.lemmas }, stats: s.stats }
}

/// How many times a node is put to the solver against a class before it stays apart.
const ATTEMPTS: usize = 8;

struct Sweeper<'a> {
    aig: &'a Aig,
    /// Each node's representative literal: itself, or the class member it was proven equal to.
    repr: Vec<AigLit>,
    lemmas: Vec<Lemma>,
    earlier: Earlier,
    stats: Stats,
}

/// Simulation patterns: per input and per latch, `words` words, the last of them filling with counterexamples.
struct Patterns {
    inputs: Vec<u64>,
    latches: Vec<u64>,
    words: usize,
    /// Patterns of the last word already taken by counterexamples (64 once it is full).
    used: usize,
}

impl Patterns {
    fn random(aig: &Aig, words: usize, seed: u64) -> Patterns {
        let mut state = seed;
        let mut next = || {
            state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let mut z = state;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            z ^ (z >> 31)
        };
        let inputs = (0..aig.inputs().len() * words).map(|_| next()).collect();
        let latches = (0..aig.latches().len() * words).map(|_| next()).collect();
        Patterns { inputs, latches, words, used: 64 }
    }

    /// Add one assignment: a fresh word carrying it in every pattern, or the next free pattern of the last word.
    fn add(&mut self, inputs: &[bool], latches: &[bool]) {
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
            return;
        }
        let (words, bit) = (self.words, self.used);
        for (store, values) in [(&mut self.inputs, inputs), (&mut self.latches, latches)] {
            for (i, &v) in values.iter().enumerate() {
                let cell = &mut store[i * words + words - 1];
                *cell = (*cell & !(1 << bit)) | ((v as u64) << bit);
            }
        }
        self.used += 1;
    }
}

/// A node's signature with its phase normalized: complemented so the first pattern reads 0.
fn signature(sim: &[u64], words: usize, n: u32) -> (Vec<u64>, bool) {
    let row = &sim[n as usize * words..(n as usize + 1) * words];
    let phase = row[0] & 1 == 1;
    (row.iter().map(|&w| if phase { !w } else { w }).collect(), phase)
}

impl Sweeper<'_> {
    fn resolve(&self, l: AigLit) -> AigLit {
        self.repr[l.node() as usize].with_phase(l.is_complemented())
    }

    fn learn(&mut self, a: AigLit, b: AigLit, cut: Vec<u32>, proof: Lrat) {
        self.lemmas.push(Lemma { a, b, cut, proof });
        self.earlier.push(a, b);
    }

    fn sweep(&mut self, pairs: &[(AigLit, AigLit)], opts: &Options) {
        let aig = self.aig;
        let roots: Vec<AigLit> = pairs.iter().flat_map(|&(a, b)| [a, b]).collect();
        let mut cone = aig.cone(&roots);
        if cone.first() != Some(&0) {
            cone.insert(0, 0);
        }
        // A node both sides reach is one node to both: merging it proves nothing the pairs need, and an invariant of
        // shared arithmetic (a carry that never rises) can cost a solver everything. It stays a representative.
        let left: HashSet<u32> = aig.cone(&pairs.iter().map(|p| p.0).collect::<Vec<_>>()).into_iter().collect();
        let right: HashSet<u32> = aig.cone(&pairs.iter().map(|p| p.1).collect::<Vec<_>>()).into_iter().collect();
        let mut patterns = Patterns::random(aig, opts.sim_words.max(1), opts.seed);
        let mut sim = aig.simulate(patterns.words, &patterns.inputs, &patterns.latches);
        let mut classes: HashMap<Vec<u64>, u32> = HashMap::new();
        let mut apart: Vec<u32> = Vec::new();
        self.stats.cone = cone.len();
        for &n in &cone {
            self.stats.swept += 1;
            let is_and = matches!(aig.node(n), Node::And(..)) && !(left.contains(&n) && right.contains(&n));
            let mut attempts = 0;
            loop {
                let (key, phase) = signature(&sim, patterns.words, n);
                let Some(&r) = classes.get(&key) else {
                    classes.insert(key, n);
                    break;
                };
                if !is_and || attempts == ATTEMPTS {
                    break;
                }
                attempts += 1;
                let (_, phase_r) = signature(&sim, patterns.words, r);
                let target = AigLit::new(r, phase ^ phase_r);
                self.stats.sat_calls += 1;
                let (outcome, cut) =
                    prove_local(aig, AigLit::new(n, false), target, &self.earlier, &self.repr, opts.sweep_budget, &mut self.stats);
                tick(&self.stats, opts);
                match outcome {
                    Outcome::Equal(proof) => {
                        self.learn(AigLit::new(n, false), target, cut, proof);
                        self.repr[n as usize] = target;
                        self.stats.merges += 1;
                        break;
                    }
                    Outcome::Differ(inputs, latches) => {
                        self.stats.counterexamples += 1;
                        patterns.add(&inputs, &latches);
                        sim = aig.simulate(patterns.words, &patterns.inputs, &patterns.latches);
                        classes.clear();
                        for &m in &apart {
                            classes.entry(signature(&sim, patterns.words, m).0).or_insert(m);
                        }
                    }
                    Outcome::Unknown(_) => {
                        self.stats.unknown += 1;
                        break;
                    }
                }
            }
            if self.repr[n as usize] == AigLit::new(n, false) {
                apart.push(n);
            }
        }
    }
}

/// Union-find over nodes with the parity of each node to its root: `value(n) = value(root) ⊕ parity`.
struct Parity {
    parent: Vec<u32>,
    flip: Vec<bool>,
}

impl Parity {
    fn new(n: usize) -> Parity {
        Parity { parent: (0..n as u32).collect(), flip: vec![false; n] }
    }

    fn find(&mut self, n: u32) -> (u32, bool) {
        let mut path = Vec::new();
        let mut x = n;
        while self.parent[x as usize] != x {
            path.push(x);
            x = self.parent[x as usize];
        }
        let root = x;
        for &p in path.iter().rev() {
            let up = self.parent[p as usize];
            if up != root {
                self.flip[p as usize] ^= self.flip[up as usize];
            }
            self.parent[p as usize] = root;
        }
        (root, self.flip[n as usize] && n != root)
    }

    fn lit(&mut self, l: AigLit) -> (u32, bool) {
        let (r, p) = self.find(l.node());
        (r, p ^ l.is_complemented())
    }

    fn union(&mut self, a: AigLit, b: AigLit) {
        let (ra, pa) = self.lit(a);
        let (rb, pb) = self.lit(b);
        if ra != rb {
            self.parent[ra as usize] = rb;
            self.flip[ra as usize] = pa ^ pb;
        }
    }
}

/// Whether `certificate` proves every claim: each lemma's proof refutes its query in order, and each claimed pair is
/// one literal, or two literals the lemmas join with matching phase.
pub fn check(aig: &Aig, certificate: &Certificate, claims: &[(AigLit, AigLit)]) -> bool {
    let n = aig.num_nodes();
    let in_graph = |l: AigLit| (l.node() as usize) < n;
    let mut earlier = Earlier::new();
    for lemma in &certificate.lemmas {
        if !in_graph(lemma.a) || !in_graph(lemma.b) {
            return false;
        }
        if lemma.cut.windows(2).any(|w| w[0] >= w[1]) || lemma.cut.iter().any(|&c| c as usize >= n) {
            return false;
        }
        let q = query_indexed(aig, lemma.a, lemma.b, &lemma.cut, &earlier);
        if !lrat::check(q.num_vars, &q.clauses, &lemma.proof) {
            return false;
        }
        earlier.push(lemma.a, lemma.b);
    }
    let mut uf = Parity::new(n);
    for &(a, b) in earlier.pairs() {
        uf.union(a, b);
    }
    claims.iter().all(|&(a, b)| in_graph(a) && in_graph(b) && (a == b || uf.lit(a) == uf.lit(b)))
}

/// Two circuits in one graph: inputs of the same name shared (unnamed and one-sided inputs kept apart), each named
/// output of the left paired with the right's output of that name, and an output a pair's difference.
#[derive(Clone, Debug)]
pub struct Miter {
    pub aig: Aig,
    /// Each output's name, its literal in the left circuit and in the right.
    pub pairs: Vec<(String, AigLit, AigLit)>,
}

/// Why two circuits cannot be paired.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MiterError {
    /// A circuit holds latches: unroll it first ([`crate::seq_equiv`]).
    Sequential,
    /// An output has no name to pair it by.
    Unnamed,
    /// An output name one circuit has and the other lacks.
    Unmatched(String),
    /// An output name a circuit uses twice.
    Duplicate(String),
}

pub fn miter(left: &Aig, right: &Aig) -> Result<Miter, MiterError> {
    if !left.latches().is_empty() || !right.latches().is_empty() {
        return Err(MiterError::Sequential);
    }
    for g in [left, right] {
        let mut seen = std::collections::HashSet::new();
        for o in g.outputs() {
            let name = o.name.as_ref().ok_or(MiterError::Unnamed)?;
            if !seen.insert(name) {
                return Err(MiterError::Duplicate(name.clone()));
            }
        }
    }
    let mut aig = Aig::new();
    let mut by_name: HashMap<String, AigLit> = HashMap::new();
    let left_inputs: Vec<AigLit> = left
        .inputs()
        .iter()
        .map(|p| {
            let l = aig.add_input(p.name.clone());
            if let Some(n) = &p.name {
                by_name.insert(n.clone(), l);
            }
            l
        })
        .collect();
    let right_inputs: Vec<AigLit> = right
        .inputs()
        .iter()
        .map(|p| match p.name.as_ref().and_then(|n| by_name.get(n)) {
            Some(&l) => l,
            None => aig.add_input(p.name.clone()),
        })
        .collect();
    let leaf = |inputs: &[AigLit], l: Leaf| match l {
        Leaf::Input(i) => inputs[i],
        Leaf::Latch(_) => unreachable!("a miter's circuits are combinational"),
    };
    let lmap = aig.import(left, &mut |l| leaf(&left_inputs, l));
    let rmap = aig.import(right, &mut |l| leaf(&right_inputs, l));
    let at = |map: &[AigLit], l: AigLit| map[l.node() as usize].with_phase(l.is_complemented());
    let mut pairs = Vec::with_capacity(left.outputs().len());
    for o in left.outputs() {
        let name = o.name.clone().expect("named above");
        let k = right.output_index(&name).ok_or_else(|| MiterError::Unmatched(name.clone()))?;
        pairs.push((name, at(&lmap, o.lit), at(&rmap, right.outputs()[k].lit)));
    }
    if let Some(o) = right.outputs().iter().find(|o| left.output_index(o.name.as_deref().unwrap()).is_none()) {
        return Err(MiterError::Unmatched(o.name.clone().unwrap()));
    }
    for (name, a, b) in &pairs {
        let d = aig.xor(*a, *b);
        aig.add_output(d, Some(name.clone()));
    }
    // Only the outputs' cones decide anything: what neither side's outputs read is left out.
    let operands: Vec<AigLit> = pairs.iter().flat_map(|(_, a, b)| [*a, *b]).collect();
    let (aig, map) = aig.compact_keeping(&operands);
    let moved = |l: AigLit| map[l.node() as usize].expect("an operand is kept").with_phase(l.is_complemented());
    let pairs = pairs.into_iter().map(|(name, a, b)| (name, moved(a), moved(b))).collect();
    Ok(Miter { aig, pairs })
}

/// A decided miter: a verdict per output name, the certificate behind the equalities, and the search's cost.
#[derive(Clone, Debug)]
pub struct Report {
    pub miter: Miter,
    pub verdicts: Vec<(String, Verdict)>,
    pub certificate: Certificate,
    pub stats: Stats,
}

impl Report {
    /// Every output proven equal.
    pub fn all_equal(&self) -> bool {
        self.verdicts.iter().all(|(_, v)| *v == Verdict::Equal)
    }

    /// The literal pairs of the outputs proven equal.
    pub fn equal_pairs(&self) -> Vec<(AigLit, AigLit)> {
        self.miter
            .pairs
            .iter()
            .zip(&self.verdicts)
            .filter(|(_, (_, v))| *v == Verdict::Equal)
            .map(|((_, a, b), _)| (*a, *b))
            .collect()
    }

    /// The certificate proves every equality, and every difference's assignment separates its pair.
    pub fn check(&self) -> bool {
        let differences_replay = self.miter.pairs.iter().zip(&self.verdicts).all(|((_, a, b), (_, v))| match v {
            Verdict::Differ { inputs, latches } => {
                let values = self.miter.aig.eval(inputs, latches);
                Aig::value(&values, *a) != Aig::value(&values, *b)
            }
            _ => true,
        });
        differences_replay && check(&self.miter.aig, &self.certificate, &self.equal_pairs())
    }

    /// An assignment of the miter's inputs, by name (unnamed inputs left out).
    pub fn named_inputs(&self, inputs: &[bool]) -> Vec<(String, bool)> {
        self.miter
            .aig
            .inputs()
            .iter()
            .zip(inputs)
            .filter_map(|(p, &v)| p.name.clone().map(|n| (n, v)))
            .collect()
    }
}

/// An input assignment on which some pair of `left`'s and `right`'s outputs differ, found by one search over every
/// difference at once and replayed before it is returned, with the miter it is an assignment of; `Ok(None)` when the
/// search shows they never differ. A search, not a certificate: an equality is claimed only by [`equivalent`].
pub fn separating_input(left: &Aig, right: &Aig, budget: u64) -> Result<Option<(Miter, Vec<bool>)>, String> {
    let mut m = miter(left, right).map_err(|e| format!("{e:?}"))?;
    let differences: Vec<AigLit> = m.aig.outputs().iter().map(|o| o.lit).collect();
    let any = differences.into_iter().fold(AigLit::FALSE, |acc, d| m.aig.or(acc, d));
    match witness(&m.aig, any, budget)? {
        Some((inputs, _)) => Ok(Some((m, inputs))),
        None => Ok(None),
    }
}

/// Pair `left` and `right` by output name and decide every pair.
pub fn equivalent(left: &Aig, right: &Aig, opts: &Options) -> Result<Report, MiterError> {
    let miter = miter(left, right)?;
    let pairs: Vec<(AigLit, AigLit)> = miter.pairs.iter().map(|(_, a, b)| (*a, *b)).collect();
    let decided = decide(&miter.aig, &pairs, opts);
    let verdicts = miter.pairs.iter().map(|p| p.0.clone()).zip(decided.verdicts).collect();
    Ok(Report { miter, verdicts, certificate: decided.certificate, stats: decided.stats })
}

/// A family of circuits told apart by [`distinguish`].
#[derive(Clone, Debug)]
pub struct Distinction {
    /// Members that compute one function, by index: each class in member order, the classes by their first member.
    pub classes: Vec<Vec<usize>>,
    /// For each member after the first of its class: that first member, the member, and the report proving them equal.
    pub proofs: Vec<(usize, usize, Report)>,
    /// The input names a vector assigns, in order of first appearance across the members.
    pub inputs: Vec<String>,
    /// The output names, in the first member's order.
    pub outputs: Vec<String>,
    /// The seeds, then every input assignment found to separate two members, over `inputs`.
    pub vectors: Vec<Vec<bool>>,
    /// Vectors, by index, on one of which every two classes differ.
    pub cover: Vec<usize>,
    /// Each class's outputs on each cover vector, over `outputs`.
    pub table: Vec<Vec<Vec<bool>>>,
}

impl Distinction {
    /// The classes partition the members, every proof checks and binds the two members it names, every two classes
    /// differ on a cover vector, and the table is what each class's members compute there.
    pub fn check(&self, members: &[Aig]) -> bool {
        let mut seen = vec![false; members.len()];
        for &m in self.classes.iter().flatten() {
            if m >= members.len() || std::mem::replace(&mut seen[m], true) {
                return false;
            }
        }
        if seen.contains(&false) || self.classes.iter().any(|c| c.is_empty()) {
            return false;
        }
        let proven: HashSet<(usize, usize)> = self.proofs.iter().map(|(r, m, _)| (*r, *m)).collect();
        if self.classes.iter().any(|c| c[1..].iter().any(|&m| !proven.contains(&(c[0], m)))) {
            return false;
        }
        for (rep, m, report) in &self.proofs {
            let bound = miter(&members[*rep], &members[*m]).is_ok_and(|again| again.aig.to_binary() == report.miter.aig.to_binary());
            if !bound || !report.all_equal() || !report.check() {
                return false;
            }
        }
        let Ok(seen) = Outputs::of(members, &self.inputs, &self.outputs) else { return false };
        if self.table.len() != self.classes.len() || self.cover.iter().any(|&v| v >= self.vectors.len()) {
            return false;
        }
        let on: Vec<Vec<Vec<bool>>> = self.classes.iter().map(|c| self.cover.iter().map(|&v| seen.eval(c[0], &self.vectors[v])).collect()).collect();
        let agree = self.classes.iter().zip(&on).all(|(c, rows)| c.iter().all(|&m| self.cover.iter().zip(rows).all(|(&v, row)| seen.eval(m, &self.vectors[v]) == *row)));
        let apart = (0..on.len()).all(|i| (i + 1..on.len()).all(|j| on[i] != on[j]));
        agree && apart && on == self.table
    }
}

/// The members' inputs and outputs by name: what a vector over `inputs` gives each member's outputs over `outputs`.
struct Outputs<'a> {
    members: &'a [Aig],
    /// For each member, each of its inputs' place in `inputs`; `None` for one without a name, which no output reads.
    places: Vec<Vec<Option<usize>>>,
    /// For each member, its literal for each name of `outputs`.
    lits: Vec<Vec<AigLit>>,
}

impl<'a> Outputs<'a> {
    fn of(members: &'a [Aig], inputs: &[String], outputs: &[String]) -> Result<Outputs<'a>, String> {
        let mut places = Vec::with_capacity(members.len());
        let mut lits = Vec::with_capacity(members.len());
        for (k, g) in members.iter().enumerate() {
            if !g.latches().is_empty() {
                return Err(format!("member {k} holds latches"));
            }
            let read: HashSet<u32> = g.cone(&g.outputs().iter().map(|o| o.lit).collect::<Vec<_>>()).into_iter().collect();
            let place = g
                .inputs()
                .iter()
                .map(|p| match &p.name {
                    Some(n) => inputs.iter().position(|i| i == n).map(Some).ok_or_else(|| format!("member {k}: input {n} is not among the inputs")),
                    None if read.contains(&p.lit.node()) => Err(format!("member {k}: an input without a name that an output reads, which no vector can assign")),
                    None => Ok(None),
                })
                .collect::<Result<Vec<Option<usize>>, String>>()?;
            if g.outputs().len() != outputs.len() {
                return Err(format!("member {k} has {} outputs, the first {}", g.outputs().len(), outputs.len()));
            }
            let lit = outputs
                .iter()
                .map(|o| g.output_index(o).map(|i| g.outputs()[i].lit).ok_or_else(|| format!("member {k} has no output {o}")))
                .collect::<Result<Vec<AigLit>, String>>()?;
            places.push(place);
            lits.push(lit);
        }
        Ok(Outputs { members, places, lits })
    }

    fn eval(&self, m: usize, vector: &[bool]) -> Vec<bool> {
        let inputs: Vec<bool> = self.places[m].iter().map(|&p| p.is_some_and(|p| vector[p])).collect();
        let values = self.members[m].eval(&inputs, &[]);
        self.lits[m].iter().map(|&l| Aig::value(&values, l)).collect()
    }

    /// Member `m`'s outputs on every vector, sixty-four vectors a word, output by output.
    fn signature(&self, m: usize, vectors: &[Vec<bool>]) -> Vec<u64> {
        let words = vectors.len().div_ceil(64);
        if words == 0 {
            return Vec::new();
        }
        let g = &self.members[m];
        let mut inputs = vec![0u64; g.inputs().len() * words];
        for (k, v) in vectors.iter().enumerate() {
            for (i, &p) in self.places[m].iter().enumerate() {
                if p.is_some_and(|p| v[p]) {
                    inputs[i * words + k / 64] |= 1 << (k % 64);
                }
            }
        }
        let sim = g.simulate(words, &inputs, &[]);
        let tail = if vectors.len() % 64 == 0 { u64::MAX } else { (1u64 << (vectors.len() % 64)) - 1 };
        self.lits[m]
            .iter()
            .flat_map(|&l| (0..words).map(move |w| (l, w)))
            .map(|(l, w)| Aig::word(&sim, words, l, w) & if w + 1 == words { tail } else { u64::MAX })
            .collect()
    }
}

/// Tell a family of circuits apart: which members compute the same function, each such equality proven and
/// certified ([`equivalent`]), and input assignments on which every two members that differ give different outputs.
/// The members share input and output names; an input without a name is refused unless no output reads it. Simulation on the vectors so far groups the members by their outputs;
/// within a group each member is put against the group's first, and an input that separates them ([`separating_input`],
/// `budget` conflicts) becomes a vector that splits the group, while none found makes it proven equal. The cover is
/// chosen greedily from the `seeds` and the found vectors, each pick the vector that tells the most pairs of classes
/// apart that no earlier pick does; the table is each class's outputs there, what names the class of a black box
/// whose outputs on the cover are measured.
pub fn distinguish(members: &[Aig], seeds: &[Vec<(String, bool)>], budget: u64, opts: &Options) -> Result<Distinction, String> {
    let mut inputs: Vec<String> = Vec::new();
    for g in members {
        for n in g.inputs().iter().filter_map(|p| p.name.as_ref()) {
            if !inputs.contains(n) {
                inputs.push(n.clone());
            }
        }
    }
    let outputs: Vec<String> = match members.first() {
        Some(g) => g.outputs().iter().map(|o| o.name.clone().ok_or("an output without a name")).collect::<Result<_, _>>()?,
        None => Vec::new(),
    };
    let seen = Outputs::of(members, &inputs, &outputs)?;
    let over = |named: &[(String, bool)]| -> Vec<bool> { inputs.iter().map(|i| named.iter().any(|(n, v)| n == i && *v)).collect() };
    let mut vectors: Vec<Vec<bool>> = seeds.iter().map(|s| over(s)).collect();
    let mut proofs: Vec<(usize, usize, Report)> = Vec::new();
    let mut proven = vec![false; members.len()];
    let groups = loop {
        let signatures: Vec<Vec<u64>> = (0..members.len()).map(|m| seen.signature(m, &vectors)).collect();
        let mut groups: Vec<Vec<usize>> = Vec::new();
        for m in 0..members.len() {
            match groups.iter_mut().find(|g| signatures[g[0]] == signatures[m]) {
                Some(g) => g.push(m),
                None => groups.push(vec![m]),
            }
        }
        let mut split = None;
        'groups: for g in &groups {
            for &m in &g[1..] {
                if proven[m] {
                    continue;
                }
                match separating_input(&members[g[0]], &members[m], budget)? {
                    Some((found, assignment)) => {
                        let named: Vec<(String, bool)> =
                            found.aig.inputs().iter().zip(&assignment).filter_map(|(p, &v)| p.name.clone().map(|n| (n, v))).collect();
                        split = Some(over(&named));
                        break 'groups;
                    }
                    None => {
                        let report = equivalent(&members[g[0]], &members[m], opts).map_err(|e| format!("{e:?}"))?;
                        if !report.all_equal() || !report.check() {
                            return Err(format!("members {} and {m}: no input separates them, yet no certified proof", g[0]));
                        }
                        proven[m] = true;
                        proofs.push((g[0], m, report));
                    }
                }
            }
        }
        match split {
            Some(v) => vectors.push(v),
            None => break groups,
        }
    };
    let on: Vec<Vec<Vec<bool>>> = groups.iter().map(|g| vectors.iter().map(|v| seen.eval(g[0], v)).collect()).collect();
    let mut apart: Vec<(usize, usize)> = (0..groups.len()).flat_map(|i| (i + 1..groups.len()).map(move |j| (i, j))).collect();
    let mut cover: Vec<usize> = Vec::new();
    while !apart.is_empty() {
        let tells = |v: usize| apart.iter().filter(|&&(i, j)| on[i][v] != on[j][v]).count();
        let best = (0..vectors.len()).max_by_key(|&v| (tells(v), std::cmp::Reverse(v))).filter(|&v| tells(v) > 0);
        let Some(best) = best else { return Err("two groups the vectors do not tell apart".into()) };
        cover.push(best);
        apart.retain(|&(i, j)| on[i][best] == on[j][best]);
    }
    let table = on.iter().map(|rows| cover.iter().map(|&v| rows[v].clone()).collect()).collect();
    Ok(Distinction { classes: groups, proofs, inputs, outputs, vectors, cover, table })
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_proof/sweep/tests.rs"]
mod tests;
