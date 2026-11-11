//! Formula preprocessing for the CDCL core: the simplifications that shrink an industrial formula before search —
//! level-0 unit propagation, equivalent-literal substitution, subsumption with self-subsuming strengthening, and
//! bounded variable elimination with gate definitions (Eén & Biere, *Effective preprocessing in SAT through variable
//! and clause elimination*, SAT 2005, the SatElite scheme that MiniSat, Glucose, CaDiCaL and Kissat all carry).
//!
//! The result is a formula over the same variables that is satisfiable exactly when the input is, a proof, and a
//! reconstruction stack:
//!
//! - **Proof.** Every clause the preprocessor creates is logged as a RUP addition — a resolvent under its two
//!   parents, a substituted clause under the binary clauses of its equivalence class, a strengthened clause under the
//!   clause that strengthened it, a clause with a fixed literal dropped under that fixed unit — and every clause it
//!   removes as a deletion. Reverse unit propagation only gains from more clauses, so each addition is RUP against
//!   the input and the additions before it, and so is every clause a CDCL search then learns on the simplified
//!   formula: the steps followed by the search's learned clauses refute the input whenever the search refutes the
//!   simplified formula.
//! - **Reconstruction.** Each removal of a clause that the simplified formula no longer implies pushes the clause
//!   with a pivot literal: a fixed literal with its unit, a substituted variable with the two clauses of its
//!   equivalence, an eliminated variable with every clause it occurred in, pivoted on its occurrence. A model of the
//!   simplified formula is extended by walking the stack backward and making each pivot true wherever its clause is
//!   false ([`Preprocessed::extend`]); an entry only ever names variables whose values every later entry has settled,
//!   so the walk ends at a model of the input.
//!
//! Congruence closure (Biere, Fleury & Froleyks, *Clausal Congruence Closure*, SAT 2024) reads gates off the clauses —
//! an AND as its binaries and its long clause, an XOR as its parity gadget, an if-then-else as its four ternary
//! clauses — and merges the outputs of two gates of one kind over equal inputs, closing until no gate merges; the two copies of a circuit in an equivalence-checking
//! miter fold into one this way, and the miter's differing outputs into a contradiction. Each merge is proved by RUP
//! (an XOR's or an if-then-else's through a tree of case splits on its inputs) and the merged variables are
//! substituted out.
//!
//! Bounded variable addition (Manthey, Heule & Biere, *Automated Reencoding of Boolean Formulas*, HVC 2012) is the
//! inverse of elimination, opt-in through [`Options::factor`]: where literals `l₁ … l_k` each occur beside the same
//! rests `R₁ … R_m`, a fresh variable `x` replaces the `k·m` clauses `lᵢ ∨ Rⱼ` by the `k + m` clauses `lᵢ ∨ x` and
//! `¬x ∨ Rⱼ`. A pairwise at-most-one of `n` literals, `n(n−1)/2` binaries, shrinks to a few times `n`. The new clauses
//! are RAT on `x`, logged as PR steps, and the result has more variables than the input
//! ([`Preprocessed::num_vars`]).
//!
//! Elimination of `x` replaces the clauses of `x` by their non-tautological resolvents when that adds at most
//! [`Options::bound`] clauses and no resolvent outgrows [`Options::clause_limit`]. When `x` is defined by a gate —
//! an AND (equivalence being its one-input case) or an XOR of other literals — only the resolvents of the gate's
//! clauses against the others are kept: the resolvents of two non-gate clauses follow from those, and of two gate
//! clauses are tautologies (Eén & Biere §4), so the gate form is smaller and the reconstruction stays the same.
//!
//! Model counting needs more than equisatisfiability: [`Options::defined_only`] eliminates a variable only when the
//! others decide it — a gate defines it, or no assignment of the others satisfies the rests of both its positive and
//! its negative clauses on a part of the formula around it (Lagniez & Marquis's elimination of defined variables,
//! *Preprocessing for Propositional Model Counting*, AAAI 2014). Each eliminated variable then has one value in each
//! model, and with `factor` off the simplified formula has as many models over the variables left unsettled
//! ([`Preprocessed::settled`]) as the input over all of its own. [`Options::renew_costs`] keeps the schedule of
//! elimination costs current, so a circuit's outputs go first and each gate follows once its last reader is gone,
//! adding no clause.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, HashSet};

use crate::cdcl::{BudgetedResult, Lit, SolveResult, Solver, Var};
use crate::proof::{ProofStep, Witness};

/// What a preprocessing run may do and spend.
#[derive(Clone, Copy, Debug)]
pub struct Options {
    /// Run bounded variable elimination.
    pub eliminate: bool,
    /// Run equivalent-literal substitution over the binary-implication graph.
    pub substitute: bool,
    /// Run subsumption and self-subsuming strengthening.
    pub subsume: bool,
    /// Run congruence closure over the AND, XOR and if-then-else gates the clauses define.
    pub congruence: bool,
    /// Run bounded variable addition, which introduces variables; off by default, so a default run keeps the
    /// input's variables.
    pub factor: bool,
    /// Eliminate only variables the others define: those a gate defines, and those a local check shows defined
    /// ([`DEFINING_CLAUSES`], [`DEFINING_CONFLICTS`]). With `factor` off the simplified formula then has as many models
    /// over the variables it has not settled ([`Preprocessed::settled`]) as the input has over all of its variables:
    /// every settled variable's value follows from the others' in every model.
    pub defined_only: bool,
    /// Renew a variable's elimination cost whenever an elimination touches its clauses, so the cheapest variable is
    /// always the next one tried, and a variable tried before is tried again once an elimination touches it; off by
    /// default, where each round takes the costs once.
    pub renew_costs: bool,
    /// The clauses an elimination may add beyond those it removes.
    pub bound: usize,
    /// The longest resolvent an elimination may make.
    pub clause_limit: usize,
    /// A variable occurring in more clauses than this on either side is not eliminated.
    pub occurrence_limit: usize,
    /// The literal visits the whole run may spend; each pass is held besides to a multiple of the formula's literals.
    pub effort: u64,
    /// Rounds of substitution, subsumption and elimination; a round that changes nothing ends the run.
    pub rounds: usize,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            eliminate: true,
            substitute: true,
            subsume: true,
            congruence: true,
            factor: false,
            defined_only: false,
            renew_costs: false,
            bound: 0,
            clause_limit: 100,
            occurrence_limit: 1000,
            effort: 400_000_000,
            rounds: 4,
        }
    }
}

/// A preprocessed formula: the simplified clauses, the proof that led there, and the stack that extends a model of
/// the simplified clauses to one of the input.
#[derive(Clone, Debug)]
pub struct Preprocessed {
    /// The simplified formula, over [`Preprocessed::num_vars`] variables: satisfiable exactly when the input is.
    pub clauses: Vec<Vec<Lit>>,
    /// The simplified formula's variables: the input's, followed by those bounded variable addition introduced.
    pub num_vars: usize,
    /// The RUP additions and deletions, in order; ending with the empty clause when the run refuted the input.
    pub steps: Vec<ProofStep>,
    /// Whether preprocessing itself refuted the input.
    pub unsat: bool,
    /// Variables eliminated by resolution.
    pub eliminated: usize,
    /// Variables replaced by the representative of their equivalence class.
    pub substituted: usize,
    /// Clauses removed as subsumed.
    pub subsumed: usize,
    /// Clauses shortened by self-subsuming resolution.
    pub strengthened: usize,
    /// Variables fixed at level 0.
    pub fixed: usize,
    /// Pairs of literals congruence closure found equal.
    pub congruent: usize,
    /// Variables bounded variable addition introduced.
    pub factored: usize,
    /// Per variable, whether preprocessing settled it: fixed it, replaced it by the representative of its
    /// equivalence class, or eliminated it. The simplified clauses hold no settled variable, and
    /// [`Preprocessed::extend`] sets each.
    pub settled: Vec<bool>,
    reconstruction: Vec<(Lit, Vec<Lit>)>,
}

impl Preprocessed {
    /// Turn a model of [`Preprocessed::clauses`] (over [`Preprocessed::num_vars`] variables) into one of the input, in
    /// place: its first variables then satisfy the input.
    pub fn extend(&self, model: &mut [bool]) {
        for (pivot, clause) in self.reconstruction.iter().rev() {
            if !clause.iter().any(|l| model[l.var() as usize] == l.is_positive()) {
                model[pivot.var() as usize] = pivot.is_positive();
            }
        }
    }
}

/// Preprocess `clauses` over `num_vars` variables.
pub fn preprocess(num_vars: usize, clauses: &[Vec<Lit>], options: &Options) -> Preprocessed {
    let mut db = Db::new(num_vars, options);
    db.load(clauses);
    db.propagate();
    // Each pass may spend a multiple of the formula's literals, so none runs past linear in the formula and none
    // starves the passes after it; each multiple sits above what its pass spent on nine in ten of the SAT
    // Competition 2024 sample's runs that changed the formula, and the run as a whole stays within `options.effort`.
    let passes: [(bool, u64, fn(&mut Db)); 5] = [
        (options.congruence, 100, Db::congruence),
        (options.substitute, 20, Db::substitute),
        (options.subsume, 100, Db::subsume),
        (options.factor, 200, Db::factor),
        (options.eliminate, 400, Db::eliminate),
    ];
    for _ in 0..options.rounds {
        if db.unsat || db.effort == 0 {
            break;
        }
        let before = db.progress();
        for (on, per_literal, pass) in passes {
            if !on || db.unsat {
                continue;
            }
            db.allowance = db.effort.min(per_literal.saturating_mul(db.literals().max(1) as u64));
            pass(&mut db);
        }
        if db.progress() == before {
            break;
        }
    }
    db.finish()
}

/// Decide `clauses` by preprocessing and then CDCL on the simplified formula. A model is extended and checked against
/// the input; a refutation is the preprocessing steps followed by the search's learned clauses, each RUP against the
/// input and those before it. An extended model that fails the input would be a preprocessing defect: the formula is
/// then decided by plain CDCL instead, never answered wrong.
pub fn solve_preprocessed(num_vars: usize, clauses: &[Vec<Lit>]) -> (SolveResult, Vec<ProofStep>) {
    let p = preprocess(num_vars, clauses, &Options::default());
    if p.unsat {
        return (SolveResult::Unsat, p.steps);
    }
    let mut solver = Solver::new(p.num_vars);
    for c in &p.clauses {
        solver.add_clause(c.clone());
    }
    match solver.solve() {
        SolveResult::Sat(mut model) => {
            p.extend(&mut model);
            model.truncate(num_vars);
            if clauses.iter().all(|c| c.iter().any(|l| model[l.var() as usize] == l.is_positive())) {
                return (SolveResult::Sat(model), Vec::new());
            }
            debug_assert!(false, "an extended model fails the input");
            let mut plain = Solver::new(num_vars);
            for c in clauses {
                plain.add_clause(c.clone());
            }
            let verdict = plain.solve();
            let proof = plain.learned().iter().map(|lc| ProofStep::Rup(lc.lits.clone())).collect();
            (verdict, proof)
        }
        SolveResult::Unsat => {
            let mut steps = p.steps;
            steps.extend(solver.learned().iter().map(|lc| ProofStep::Rup(lc.lits.clone())));
            (SolveResult::Unsat, steps)
        }
    }
}

/// The gate defining variable `x` among `clauses`, as the indices of its clauses: an AND (or equivalence) of other
/// literals in either polarity of `x`, or an XOR of up to four other variables. `None` when `x` defines no such gate.
pub fn definition(clauses: &[&[Lit]], x: Var) -> Option<Vec<usize>> {
    and_definition(clauses, x).or_else(|| xor_definition(clauses, x))
}

/// `g = a₁ ∧ … ∧ aₖ` for `g` either literal of `x`: the binaries `(¬g ∨ aᵢ)` and one clause `(g ∨ ¬a₁ ∨ … ∨ ¬aₖ)`.
/// The implied literals are indexed once, so the search is linear in the clauses of `x`.
fn and_definition(clauses: &[&[Lit]], x: Var) -> Option<Vec<usize>> {
    for g in [Lit::pos(x), Lit::neg(x)] {
        let mut implied: std::collections::HashMap<Lit, usize> = std::collections::HashMap::new();
        for (i, c) in clauses.iter().enumerate() {
            if c.len() == 2 && c.contains(&g.negated()) {
                let other = if c[0] == g.negated() { c[1] } else { c[0] };
                implied.entry(other).or_insert(i);
            }
        }
        if implied.is_empty() {
            continue;
        }
        for (i, c) in clauses.iter().enumerate() {
            if c.len() < 2 || !c.contains(&g) {
                continue;
            }
            let mut gate = vec![i];
            let covered = c.iter().filter(|&&l| l != g).all(|&l| match implied.get(&l.negated()) {
                Some(&j) => {
                    gate.push(j);
                    true
                }
                None => false,
            });
            if covered {
                gate.sort_unstable();
                gate.dedup();
                return Some(gate);
            }
        }
    }
    None
}

/// `x = a₁ ⊕ … ⊕ aₖ ⊕ b` (k ≤ 4): all `2^k` clauses over `x` and the same `k` other variables whose signs make the
/// parity right. The clauses of `x` are grouped by their variables and the parity of their negations, so the search
/// is linear in them; of several complete groups the one whose clauses come first is taken.
fn xor_definition(clauses: &[&[Lit]], x: Var) -> Option<Vec<usize>> {
    let mut groups: std::collections::HashMap<(Vec<Var>, usize), Vec<usize>> = std::collections::HashMap::new();
    for (i, c) in clauses.iter().enumerate() {
        let k = c.len().saturating_sub(1);
        if !(1..=4).contains(&k) || !c.iter().any(|l| l.var() == x) {
            continue;
        }
        let mut vars: Vec<Var> = c.iter().map(|l| l.var()).collect();
        vars.sort_unstable();
        if vars.windows(2).any(|w| w[0] == w[1]) {
            continue;
        }
        let parity = c.iter().filter(|l| !l.is_positive()).count() % 2;
        groups.entry((vars, parity)).or_default().push(i);
    }
    let mut best: Option<Vec<usize>> = None;
    for ((vars, _), members) in groups {
        let k = vars.len() - 1;
        let mut by_pattern: std::collections::BTreeMap<u32, usize> = std::collections::BTreeMap::new();
        for &i in &members {
            let pattern = vars.iter().enumerate().fold(0u32, |code, (bit, v)| {
                if clauses[i].iter().any(|l| l.var() == *v && !l.is_positive()) { code | 1 << bit } else { code }
            });
            by_pattern.entry(pattern).or_insert(i);
        }
        if by_pattern.len() == 1 << k {
            let mut gate: Vec<usize> = by_pattern.into_values().collect();
            gate.sort_unstable();
            if best.as_ref().is_none_or(|b| gate < *b) {
                best = Some(gate);
            }
        }
    }
    best
}

/// The working database: clauses kept sorted by literal index and deduplicated, occurrence lists per literal (with
/// stale entries skipped on use and swept now and then), level-0 values, and the proof and reconstruction logs.
struct Db {
    num_vars: usize,
    options: Options,
    clauses: Vec<Vec<Lit>>,
    alive: Vec<bool>,
    signatures: Vec<u64>,
    occ: Vec<Vec<u32>>,
    value: Vec<Option<bool>>,
    removed: Vec<bool>,
    queue: Vec<Lit>,
    steps: Vec<ProofStep>,
    reconstruction: Vec<(Lit, Vec<Lit>)>,
    effort: u64,
    /// The running pass's share of `effort`.
    allowance: u64,
    unsat: bool,
    eliminated: usize,
    substituted: usize,
    subsumed: usize,
    strengthened: usize,
    fixed: usize,
    congruent: usize,
    factored: usize,
}

/// Sort and deduplicate `c`; `None` when it holds a literal and its negation.
fn normalized(c: &[Lit]) -> Option<Vec<Lit>> {
    let mut c = c.to_vec();
    c.sort_unstable_by_key(|l| l.index());
    c.dedup();
    if c.windows(2).any(|w| w[0].var() == w[1].var()) {
        return None;
    }
    Some(c)
}

/// A 64-bit signature of the variables of `c`, for a cheap subset pre-test.
fn signature(c: &[Lit]) -> u64 {
    c.iter().fold(0u64, |s, l| s | 1u64 << (l.var() % 64))
}

/// How sorted `small` relates to sorted `big`: `Some(None)` when it is a subset (it subsumes `big`), `Some(Some(d))`
/// when it is one but for a single literal whose negation `d` is in `big` (self-subsuming resolution: `big` may drop
/// `d`), `None` otherwise.
fn subsumes(small: &[Lit], big: &[Lit]) -> Option<Option<Lit>> {
    let mut flipped: Option<Lit> = None;
    let mut j = 0;
    for &l in small {
        while j < big.len() && big[j].var() < l.var() {
            j += 1;
        }
        if j == big.len() || big[j].var() != l.var() {
            return None;
        }
        if big[j] != l {
            if flipped.is_some() {
                return None;
            }
            flipped = Some(big[j]);
        }
        j += 1;
    }
    Some(flipped)
}

impl Db {
    fn new(num_vars: usize, options: &Options) -> Db {
        Db {
            num_vars,
            options: *options,
            clauses: Vec::new(),
            alive: Vec::new(),
            signatures: Vec::new(),
            occ: vec![Vec::new(); num_vars * 2],
            value: vec![None; num_vars],
            removed: vec![false; num_vars],
            queue: Vec::new(),
            steps: Vec::new(),
            reconstruction: Vec::new(),
            effort: options.effort,
            allowance: options.effort,
            unsat: false,
            eliminated: 0,
            substituted: 0,
            subsumed: 0,
            strengthened: 0,
            fixed: 0,
            congruent: 0,
            factored: 0,
        }
    }

    fn progress(&self) -> (usize, usize, usize, usize, usize, usize, usize) {
        (self.eliminated, self.substituted, self.subsumed, self.strengthened, self.fixed, self.congruent, self.factored)
    }

    /// A variable past every one so far.
    fn fresh(&mut self) -> Var {
        let v = self.num_vars as Var;
        self.num_vars += 1;
        self.value.push(None);
        self.removed.push(false);
        self.occ.push(Vec::new());
        self.occ.push(Vec::new());
        v
    }

    /// Add `c` (sorted, deduplicated) as a PR step certified by `witness`.
    fn add_pr(&mut self, mut c: Vec<Lit>, witness: Witness) {
        c.sort_unstable_by_key(|l| l.index());
        c.dedup();
        self.steps.push(ProofStep::Pr { clause: c.clone(), witness });
        self.insert(c);
    }

    /// Spend `n` units of the run's effort and the running pass's allowance; `false` once either is gone.
    fn spend(&mut self, n: usize) -> bool {
        self.effort = self.effort.saturating_sub(n as u64);
        self.allowance = self.allowance.saturating_sub(n as u64);
        !self.exhausted()
    }

    fn exhausted(&self) -> bool {
        self.effort == 0 || self.allowance == 0
    }

    /// The literals of the live clauses.
    fn literals(&self) -> usize {
        (0..self.clauses.len()).filter(|&i| self.alive[i]).map(|i| self.clauses[i].len()).sum()
    }

    fn lit_value(&self, l: Lit) -> Option<bool> {
        self.value[l.var() as usize].map(|v| v == l.is_positive())
    }

    /// The input clauses enter in their order as they are (sorted, deduplicated: each clause as a set); a tautology
    /// or a repeat of a clause before it does not. Repeats are found by sorting the clauses, not by hashing them.
    fn load(&mut self, input: &[Vec<Lit>]) {
        let normal: Vec<Option<Vec<Lit>>> = input.iter().map(|c| normalized(c)).collect();
        let codes = |i: usize| normal[i].iter().flatten().map(|l| l.index());
        let mut order: Vec<usize> = (0..normal.len()).filter(|&i| normal[i].is_some()).collect();
        order.sort_by(|&a, &b| codes(a).cmp(codes(b)).then(a.cmp(&b)));
        let mut first = vec![false; normal.len()];
        for (k, &i) in order.iter().enumerate() {
            first[i] = k == 0 || !codes(order[k - 1]).eq(codes(i));
        }
        for (i, c) in normal.into_iter().enumerate() {
            if let Some(c) = c.filter(|_| first[i]) {
                self.insert(c);
                if self.unsat {
                    return;
                }
            }
        }
    }

    /// Put `c` in the database (already normalized, already logged when derived); a unit queues its literal, the
    /// empty clause ends the run.
    fn insert(&mut self, c: Vec<Lit>) {
        debug_assert!(c.windows(2).all(|w| w[0].index() < w[1].index()), "a database clause is sorted and duplicate-free");
        match c.len() {
            0 => {
                self.refute();
                return;
            }
            1 => self.queue.push(c[0]),
            _ => {}
        }
        let id = self.clauses.len() as u32;
        for &l in &c {
            self.occ[l.index()].push(id);
        }
        self.signatures.push(signature(&c));
        self.clauses.push(c);
        self.alive.push(true);
    }

    /// Derive `c` (logged as a RUP addition) and insert it.
    fn derive(&mut self, mut c: Vec<Lit>) {
        c.sort_unstable_by_key(|l| l.index());
        c.dedup();
        self.steps.push(ProofStep::Rup(c.clone()));
        self.insert(c);
    }

    fn refute(&mut self) {
        if !self.unsat {
            self.unsat = true;
            if !matches!(self.steps.last(), Some(ProofStep::Rup(c)) if c.is_empty()) {
                self.steps.push(ProofStep::Rup(Vec::new()));
            }
        }
    }

    /// Remove clause `i` (logged as a deletion), pushing it on the reconstruction stack pivoted on `pivot` when one
    /// is given.
    fn remove(&mut self, i: usize, pivot: Option<Lit>) {
        if !self.alive[i] {
            return;
        }
        self.alive[i] = false;
        let c = std::mem::take(&mut self.clauses[i]);
        self.steps.push(ProofStep::Delete(c.clone()));
        if let Some(p) = pivot {
            self.reconstruction.push((p, c));
        }
    }

    /// The live clauses holding literal `l`. A clause is never changed in place — a simplification removes it and
    /// inserts the new one — so a live clause listed under `l` holds `l`.
    fn occurrences(&self, l: Lit) -> Vec<usize> {
        let live: Vec<usize> = self.occ[l.index()].iter().map(|&i| i as usize).filter(|&i| self.alive[i]).collect();
        debug_assert!(live.iter().all(|&i| self.clauses[i].binary_search_by_key(&l.index(), |x| x.index()).is_ok()));
        live
    }

    fn sweep_occurrences(&mut self, l: Lit) {
        let (alive, clauses) = (&self.alive, &self.clauses);
        self.occ[l.index()].retain(|&i| alive[i as usize] && clauses[i as usize].binary_search_by_key(&l.index(), |x| x.index()).is_ok());
    }

    /// Propagate the queued units at level 0: each fixes its variable (its unit logged and pushed for
    /// reconstruction), removes the clauses it satisfies and drops its negation from the rest.
    fn propagate(&mut self) {
        while let Some(l) = self.queue.pop() {
            if self.unsat {
                return;
            }
            match self.lit_value(l) {
                Some(true) => continue,
                Some(false) => {
                    self.refute();
                    return;
                }
                None => {}
            }
            let v = l.var() as usize;
            self.value[v] = Some(l.is_positive());
            self.fixed += 1;
            self.steps.push(ProofStep::Rup(vec![l]));
            self.reconstruction.push((l, vec![l]));
            for i in self.occurrences(l) {
                self.remove(i, None);
            }
            for i in self.occurrences(l.negated()) {
                let shorter: Vec<Lit> = self.clauses[i].iter().copied().filter(|&m| m != l.negated()).collect();
                self.spend(shorter.len() + 1);
                self.derive(shorter);
                if self.unsat {
                    return;
                }
                self.remove(i, None);
            }
            self.occ[l.index()].clear();
            self.occ[l.negated().index()].clear();
        }
    }

    /// Equivalent-literal substitution: the strongly connected components of the binary-implication graph are
    /// classes of equal literals; each variable not its class's representative is replaced by it.
    fn substitute(&mut self) {
        let binaries: Vec<Vec<Lit>> = (0..self.clauses.len()).filter(|&i| self.alive[i] && self.clauses[i].len() == 2).map(|i| self.clauses[i].clone()).collect();
        if binaries.is_empty() {
            return;
        }
        self.spend(binaries.len() * 2);
        let classes = match crate::inprocess::equivalent_literal_scc(self.num_vars, &binaries) {
            crate::inprocess::EquivResult::Unsat(steps) => {
                for step in steps {
                    if let ProofStep::Rup(c) = step {
                        if c.is_empty() {
                            self.refute();
                        } else {
                            self.steps.push(ProofStep::Rup(c));
                        }
                    }
                }
                return;
            }
            crate::inprocess::EquivResult::Classes(classes) => classes,
        };
        let node = |l: Lit| l.var() as usize * 2 + usize::from(!l.is_positive());
        let mut representative: Vec<Option<Lit>> = vec![None; classes.len().max(1)];
        for v in 0..self.num_vars {
            for l in [Lit::pos(v as Var), Lit::neg(v as Var)] {
                let class = classes[node(l)];
                if representative[class].is_none() {
                    representative[class] = Some(l);
                }
            }
        }
        for v in 0..self.num_vars {
            if self.removed[v] || self.value[v].is_some() {
                continue;
            }
            let x = Lit::pos(v as Var);
            let Some(r) = representative[classes[node(x)]] else { continue };
            // A representative fixed meanwhile fixes `x` through the equivalence's binary clauses instead.
            if r.var() as usize == v || self.value[r.var() as usize].is_some() || self.removed[r.var() as usize] {
                continue;
            }
            self.replace(x, r);
            if self.unsat {
                return;
            }
            self.propagate();
            if self.unsat {
                return;
            }
        }
    }

    /// Replace `x` by `r` (`x ≡ r`) in every clause, then retire `x` with the two clauses of the equivalence.
    fn replace(&mut self, x: Lit, r: Lit) {
        let map = |l: Lit| {
            if l == x {
                r
            } else if l == x.negated() {
                r.negated()
            } else {
                l
            }
        };
        // Every rewritten clause is RUP through the binary clauses that make `x` and `r` equal, which are among the
        // clauses rewritten: all are derived before any original is deleted.
        let touched: Vec<usize> = self.occurrences(x).into_iter().chain(self.occurrences(x.negated())).collect();
        let mut rewrote = Vec::with_capacity(touched.len());
        for i in touched {
            if !self.alive[i] {
                continue;
            }
            let rewritten: Vec<Lit> = self.clauses[i].iter().map(|&l| map(l)).collect();
            self.spend(rewritten.len() + 1);
            if let Some(c) = normalized(&rewritten) {
                self.derive(c);
                if self.unsat {
                    return;
                }
            }
            rewrote.push(i);
        }
        for i in rewrote {
            self.remove(i, None);
        }
        self.reconstruction.push((x, vec![x, r.negated()]));
        self.reconstruction.push((x.negated(), vec![x.negated(), r]));
        self.removed[x.var() as usize] = true;
        self.occ[x.index()].clear();
        self.occ[x.negated().index()].clear();
        self.substituted += 1;
    }

    /// Backward subsumption and self-subsuming strengthening, shortest clauses first: each clause is tried against
    /// the clauses holding its least-occurring literal or that literal's negation. Each candidate costs a unit of
    /// effort before its signature is read, and a comparison its literals.
    fn subsume(&mut self) {
        let mut order: Vec<usize> = (0..self.clauses.len()).filter(|&i| self.alive[i]).collect();
        order.sort_by_key(|&i| (self.clauses[i].len(), i));
        for i in order {
            if !self.alive[i] || self.unsat || self.exhausted() {
                continue;
            }
            let c = self.clauses[i].clone();
            if c.len() < 2 {
                continue;
            }
            let sig = self.signatures[i];
            let pivot = *c.iter().min_by_key(|l| self.occ[l.index()].len() + self.occ[l.negated().index()].len()).unwrap();
            for target in [pivot, pivot.negated()] {
                for j in self.occurrences(target) {
                    if !self.spend(1) {
                        return;
                    }
                    if j == i || !self.alive[j] || !self.alive[i] {
                        continue;
                    }
                    let d = &self.clauses[j];
                    if d.len() < c.len() || sig & !self.signatures[j] != 0 {
                        continue;
                    }
                    let verdict = subsumes(&c, d);
                    let visits = d.len();
                    match verdict {
                        Some(None) if target == pivot => {
                            self.remove(j, None);
                            self.subsumed += 1;
                        }
                        Some(Some(drop)) => {
                            let shorter: Vec<Lit> = self.clauses[j].iter().copied().filter(|&m| m != drop).collect();
                            self.strengthened += 1;
                            self.derive(shorter);
                            self.remove(j, None);
                            self.propagate();
                        }
                        _ => {}
                    }
                    if !self.spend(visits) || self.unsat {
                        return;
                    }
                }
            }
        }
    }

    /// Bounded variable elimination on a schedule of costs (positive times negative occurrences), the cheapest
    /// variable first, until none qualifies or the effort runs out. With [`Options::renew_costs`] an elimination
    /// renews the costs of the variables it touched and puts them back on the schedule if they were tried already, so
    /// a circuit's outputs go first and the gates under them follow as their last readers go; without, each variable
    /// is tried once at the cost the round began with.
    fn eliminate(&mut self) {
        let renew = self.options.renew_costs;
        let mut schedule: BinaryHeap<Reverse<(usize, Var)>> = (0..self.num_vars as Var)
            .filter(|&v| !self.removed[v as usize] && self.value[v as usize].is_none())
            .map(|v| Reverse((self.cost(v), v)))
            .collect();
        let mut tried = vec![false; self.num_vars];
        while let Some(Reverse((cost, v))) = schedule.pop() {
            if self.unsat || self.exhausted() {
                return;
            }
            if self.removed[v as usize] || self.value[v as usize].is_some() || tried[v as usize] {
                continue;
            }
            if renew {
                let now = self.cost(v);
                if now != cost {
                    schedule.push(Reverse((now, v)));
                    continue;
                }
            }
            tried[v as usize] = true;
            let mut around: Vec<Var> = if renew {
                [Lit::pos(v), Lit::neg(v)]
                    .into_iter()
                    .flat_map(|l| self.occurrences(l))
                    .flat_map(|i| self.clauses[i].iter().map(|l| l.var()).collect::<Vec<Var>>())
                    .filter(|&u| u != v)
                    .collect()
            } else {
                Vec::new()
            };
            self.try_eliminate(v);
            if self.unsat {
                return;
            }
            self.propagate();
            if self.removed[v as usize] {
                around.sort_unstable();
                around.dedup();
                for u in around {
                    if !self.removed[u as usize] && self.value[u as usize].is_none() {
                        tried[u as usize] = false;
                        schedule.push(Reverse((self.cost(u), u)));
                    }
                }
            }
        }
    }

    /// Whether the other variables decide `x`, shown on a part of the formula: no assignment of them satisfies both the
    /// rests of `x`'s positive clauses `pos` and the rests of its negative clauses `neg`, beside the clauses that share
    /// a variable with a rest. With every other clause the check would be exact; with fewer it can only miss a
    /// definition, never claim a false one. It gathers at most [`DEFINING_CLAUSES`] clauses, spends at most
    /// [`DEFINING_CONFLICTS`] conflicts, and charges the effort a unit per literal gathered and per propagation.
    fn defined_by_the_rest(&mut self, x: Var, pos: &[usize], neg: &[usize]) -> bool {
        let mut local: Vec<Vec<Lit>> =
            pos.iter().chain(neg).map(|&i| self.clauses[i].iter().copied().filter(|l| l.var() != x).collect()).collect();
        let mut taken: HashSet<usize> = pos.iter().chain(neg).copied().collect();
        let mut near: Vec<Var> = local.iter().flatten().map(|l| l.var()).collect();
        near.sort_unstable();
        near.dedup();
        'gather: for v in near {
            for l in [Lit::pos(v), Lit::neg(v)] {
                for i in self.occurrences(l) {
                    if local.len() >= DEFINING_CLAUSES {
                        break 'gather;
                    }
                    if taken.insert(i) {
                        local.push(self.clauses[i].clone());
                    }
                }
            }
        }
        if !self.spend(local.iter().map(Vec::len).sum::<usize>() + 1) {
            return false;
        }
        let mut renamed: HashMap<Var, Var> = HashMap::new();
        let mut renumbered: Vec<Vec<Lit>> = Vec::with_capacity(local.len());
        for c in &local {
            renumbered.push(
                c.iter()
                    .map(|l| {
                        let next = renamed.len() as Var;
                        Lit::new(*renamed.entry(l.var()).or_insert(next), l.is_positive())
                    })
                    .collect(),
            );
        }
        let mut solver = Solver::new(renamed.len());
        for c in renumbered {
            solver.add_clause(c);
        }
        let verdict = solver.solve_under_assumptions_budgeted(&[], DEFINING_CONFLICTS);
        self.spend(solver.propagations() as usize) && verdict == BudgetedResult::Unsat
    }

    /// The cost of eliminating `v`: its positive occurrences times its negative ones, counting only live clauses when
    /// costs are renewed (the occurrence lists also hold removed clauses until a sweep).
    fn cost(&self, v: Var) -> usize {
        let count = |l: Lit| {
            let listed = &self.occ[l.index()];
            if self.options.renew_costs {
                listed.iter().filter(|&&i| self.alive[i as usize]).count()
            } else {
                listed.len()
            }
        };
        count(Lit::pos(v)) * count(Lit::neg(v))
    }

    /// Eliminate `x` when its resolvents fit the bounds.
    fn try_eliminate(&mut self, x: Var) {
        let (pos, neg) = (Lit::pos(x), Lit::neg(x));
        self.sweep_occurrences(pos);
        self.sweep_occurrences(neg);
        let p: Vec<usize> = self.occurrences(pos);
        let n: Vec<usize> = self.occurrences(neg);
        if p.len() + n.len() == 0 || p.len() > self.options.occurrence_limit || n.len() > self.options.occurrence_limit {
            return;
        }
        let all: Vec<usize> = p.iter().chain(&n).copied().collect();
        if !self.spend(all.iter().map(|&i| self.clauses[i].len()).sum::<usize>() * 2) {
            return;
        }
        let gate: Vec<usize> = {
            let views: Vec<&[Lit]> = all.iter().map(|&i| self.clauses[i].as_slice()).collect();
            definition(&views, x).map(|g| g.into_iter().map(|k| all[k]).collect()).unwrap_or_default()
        };
        let in_gate = |i: usize| gate.contains(&i);
        let limit = p.len() + n.len() + self.options.bound;
        let mut resolvents: Vec<Vec<Lit>> = Vec::new();
        for &a in &p {
            for &b in &n {
                if !gate.is_empty() && in_gate(a) == in_gate(b) {
                    continue;
                }
                if !self.spend(self.clauses[a].len() + self.clauses[b].len()) {
                    return;
                }
                let joined: Vec<Lit> = self.clauses[a].iter().chain(&self.clauses[b]).copied().filter(|l| l.var() != x).collect();
                let Some(r) = normalized(&joined) else { continue };
                if r.len() > self.options.clause_limit {
                    return;
                }
                resolvents.push(r);
                if resolvents.len() > limit {
                    return;
                }
            }
        }
        if self.options.defined_only && gate.is_empty() && !self.defined_by_the_rest(x, &p, &n) {
            return;
        }
        resolvents.sort_by(|a, b| a.iter().map(|l| l.index()).cmp(b.iter().map(|l| l.index())));
        resolvents.dedup();
        for r in &resolvents {
            self.steps.push(ProofStep::Rup(r.clone()));
        }
        for &i in &p {
            self.remove(i, Some(pos));
        }
        for &i in &n {
            self.remove(i, Some(neg));
        }
        self.removed[x as usize] = true;
        self.occ[pos.index()].clear();
        self.occ[neg.index()].clear();
        self.eliminated += 1;
        for r in resolvents {
            self.insert(r);
            if self.unsat {
                return;
            }
        }
    }

    fn finish(mut self) -> Preprocessed {
        let clauses = if self.unsat {
            Vec::new()
        } else {
            (0..self.clauses.len()).filter(|&i| self.alive[i]).map(|i| std::mem::take(&mut self.clauses[i])).collect()
        };
        Preprocessed {
            clauses,
            num_vars: self.num_vars,
            steps: self.steps,
            unsat: self.unsat,
            eliminated: self.eliminated,
            substituted: self.substituted,
            subsumed: self.subsumed,
            strengthened: self.strengthened,
            fixed: self.fixed,
            congruent: self.congruent,
            factored: self.factored,
            settled: (0..self.num_vars).map(|v| self.removed[v] || self.value[v].is_some()).collect(),
            reconstruction: self.reconstruction,
        }
    }
}

/// The clauses the local definability check ([`Options::defined_only`]) gathers around a variable at most.
pub const DEFINING_CLAUSES: usize = 2000;

/// The conflicts the local definability check spends at most before it gives the variable up as undefined.
pub const DEFINING_CONFLICTS: u64 = 1000;

/// The longest clause bounded variable addition factors.
const FACTOR_SIZE: usize = 5;

/// The live clauses of two to [`FACTOR_SIZE`] literals by length and literal, for factoring: a rest of `s − 1`
/// literals is matched only against the clauses of length `s`. Stale entries (removed clauses) are skipped by
/// liveness where they are read.
struct ByLength {
    lists: Vec<Vec<Vec<u32>>>,
}

impl ByLength {
    fn of(db: &Db) -> ByLength {
        let mut lists = vec![vec![Vec::new(); 2 * db.num_vars]; FACTOR_SIZE + 1];
        for (i, c) in db.clauses.iter().enumerate() {
            if db.alive[i] && (2..=FACTOR_SIZE).contains(&c.len()) {
                for l in c {
                    lists[c.len()][l.index()].push(i as u32);
                }
            }
        }
        ByLength { lists }
    }

    fn add(&mut self, id: usize, c: &[Lit]) {
        if (2..=FACTOR_SIZE).contains(&c.len()) {
            for list in self.lists.iter_mut() {
                while list.len() <= c.iter().map(|l| l.index()).max().unwrap_or(0) {
                    list.push(Vec::new());
                }
            }
            for l in c {
                self.lists[c.len()][l.index()].push(id as u32);
            }
        }
    }

    fn holding(&self, db: &Db, l: Lit) -> Vec<usize> {
        (2..=FACTOR_SIZE).flat_map(|len| self.lists[len].get(l.index()).into_iter().flatten()).map(|&i| i as usize).filter(|&i| db.alive[i]).collect()
    }
}

impl Db {
    /// Bounded variable addition, the most frequent literal first. From a literal `l`, the clauses `l ∨ Rⱼ` holding it
    /// are matched with clauses `l' ∨ Rⱼ` over the same rests (found among the clauses of each rest's length plus one
    /// that hold its least frequent literal), and the literal `l'` matching the most rests joins while the saving
    /// `k·m − k − m` grows, the rests narrowing to those it matched. A positive saving replaces the `k·m` clauses by
    /// `lᵢ ∨ x` and `¬x ∨ Rⱼ` over a fresh `x`: each `lᵢ ∨ x` is RAT on `x`, no clause holding `¬x` yet, and each
    /// `¬x ∨ Rⱼ` is RAT on `¬x`, its resolvents `lᵢ ∨ Rⱼ` still present, so both are PR steps witnessed by their pivot;
    /// the replaced clauses are deleted after.
    fn factor(&mut self) {
        use std::collections::{BinaryHeap, HashMap};
        let mut index = ByLength::of(self);
        let mut heap: BinaryHeap<(usize, std::cmp::Reverse<u32>)> = (0..2 * self.num_vars)
            .map(|i| ((2..=FACTOR_SIZE).map(|len| index.lists[len][i].len()).sum::<usize>(), std::cmp::Reverse(i as u32)))
            .filter(|&(n, _)| n >= 2)
            .collect();
        while let Some((listed, std::cmp::Reverse(code))) = heap.pop() {
            if self.unsat || self.exhausted() {
                return;
            }
            let l = Lit::new(code / 2, code % 2 == 0);
            if self.removed[l.var() as usize] || self.value[l.var() as usize].is_some() {
                continue;
            }
            let holding = index.holding(self, l);
            if holding.len() != listed {
                if holding.len() >= 2 && holding.len() < listed {
                    heap.push((holding.len(), std::cmp::Reverse(code)));
                }
                continue;
            }
            // Every clause matching a rest of `l` beside another literal, found once: the rests only narrow as
            // literals join, so a candidate's matches among the rests left are its matches filtered.
            let rests_all: Vec<usize> = holding;
            let mut matches: HashMap<Lit, Vec<(usize, usize)>> = HashMap::new();
            let mut scanned = 0usize;
            for (j, &ci) in rests_all.iter().enumerate() {
                let c = &self.clauses[ci];
                let same = &index.lists[c.len()];
                let Some(&least) = c.iter().filter(|&&m| m != l).min_by_key(|m| same.get(m.index()).map_or(0, |v| v.len())) else { continue };
                let candidates = &same[least.index()];
                scanned += candidates.len() + 1;
                for &di in candidates {
                    let di = di as usize;
                    if di == ci || !self.alive[di] {
                        continue;
                    }
                    let d = &self.clauses[di];
                    let mut extra = d.iter().filter(|m| !c.contains(m));
                    let (Some(&other), None) = (extra.next(), extra.next()) else { continue };
                    if other.var() == l.var() || !c.iter().filter(|&&m| m != l).all(|m| d.contains(m)) {
                        continue;
                    }
                    matches.entry(other).or_default().push((j, di));
                }
            }
            if !self.spend(scanned) {
                return;
            }
            let mut lits = vec![l];
            let mut live = vec![true; rests_all.len()];
            let mut kept = rests_all.len();
            // partners[j]: the clauses of rest j beside each literal of `lits`, in order.
            let mut partners: Vec<Vec<usize>> = rests_all.iter().map(|&i| vec![i]).collect();
            loop {
                let best = matches
                    .iter()
                    .filter(|(m, _)| !lits.contains(m))
                    .map(|(&m, pairs)| (pairs.iter().filter(|&&(j, _)| live[j]).count(), m))
                    .max_by_key(|&(count, m)| (count, std::cmp::Reverse(m.index())));
                let Some((count, other)) = best else { break };
                let saving = |k: usize, m: usize| (k * m) as i64 - (k + m) as i64;
                if saving(lits.len() + 1, count) <= saving(lits.len(), kept) {
                    break;
                }
                self.spend(matches[&other].len());
                let mut matched = vec![false; rests_all.len()];
                for &(j, di) in &matches[&other] {
                    if live[j] {
                        matched[j] = true;
                        partners[j].push(di);
                    }
                }
                for j in 0..rests_all.len() {
                    live[j] &= matched[j];
                }
                kept = count;
                lits.push(other);
            }
            let rests: Vec<usize> = (0..rests_all.len()).filter(|&j| live[j]).map(|j| rests_all[j]).collect();
            let partners: Vec<Vec<usize>> = (0..rests_all.len()).filter(|&j| live[j]).map(|j| std::mem::take(&mut partners[j])).collect();
            if lits.len() < 2 || (lits.len() * rests.len()) as i64 - ((lits.len() + rests.len()) as i64) <= 0 {
                continue;
            }
            let x = self.fresh();
            for &li in &lits {
                self.add_pr(vec![li, Lit::pos(x)], Witness::Assignment(vec![Lit::pos(x)]));
                index.add(self.clauses.len() - 1, &self.clauses[self.clauses.len() - 1]);
            }
            for &ci in &rests {
                let clause: Vec<Lit> = std::iter::once(Lit::neg(x)).chain(self.clauses[ci].iter().copied().filter(|&m| m != l)).collect();
                self.add_pr(clause, Witness::Assignment(vec![Lit::neg(x)]));
                index.add(self.clauses.len() - 1, &self.clauses[self.clauses.len() - 1]);
            }
            for p in &partners {
                for &di in p {
                    self.remove(di, None);
                }
            }
            self.factored += 1;
            // The matched literals lost clauses; the fresh literals have them, and may factor in turn.
            for li in lits.iter().copied().chain([Lit::pos(x), Lit::neg(x)]) {
                let n = index.holding(self, li).len();
                if n >= 2 {
                    heap.push((n, std::cmp::Reverse(li.index() as u32)));
                }
            }
        }
    }
}

/// The widest AND gate congruence reads: an output and at most this many inputs.
const AND_ARITY: usize = 100;

/// The widest XOR gadget congruence reads, in variables (an output and up to one fewer inputs).
const XOR_ARITY: usize = 5;

/// A gate the clauses define: `output = inputs₁ ∧ … ∧ inputs_k` (an AND of literals), `output = inputs₁ ⊕ … ⊕ inputs_k`
/// (an XOR of positive literals, the output's sign carrying the parity), or `output = c ? t : e` (an if-then-else,
/// its inputs `[c, t, e]`).
#[derive(Clone, Debug)]
struct Gate {
    kind: GateKind,
    output: Lit,
    inputs: Vec<Lit>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum GateKind {
    And,
    Xor,
    Ite,
}

/// A gate's inputs read through the classes merged so far.
enum Normal {
    /// The gate's key — its kind and its inputs' representatives, sorted (an if-then-else's in their roles) — and the
    /// output literal equal to the gate of that key; `clean` when no two inputs shared a class and no input was fixed.
    Key { key: (GateKind, Vec<u32>), output: Lit, clean: bool },
    /// The literal the gate makes true.
    Fixed(Lit),
    /// The two literals the gate makes equal: its output and its one remaining input.
    Equal(Lit, Lit),
    /// A gate whose output is fixed or read among its own inputs.
    Skip,
}

/// Variables merged into classes, each with a parity: `value(v) = value(root) ⊕ parity(v)`, the parity relative to
/// the parent and compressed onto the root as paths are walked.
struct Classes {
    parent: Vec<u32>,
    parity: Vec<bool>,
    members: Vec<Vec<u32>>,
}

impl Classes {
    fn new(num_vars: usize) -> Classes {
        Classes { parent: (0..num_vars as u32).collect(), parity: vec![false; num_vars], members: (0..num_vars as u32).map(|v| vec![v]).collect() }
    }

    fn root(&mut self, v: usize) -> (usize, bool) {
        let mut path = Vec::new();
        let mut u = v;
        while self.parent[u] as usize != u {
            path.push(u);
            u = self.parent[u] as usize;
        }
        let mut to_root = false;
        for &w in path.iter().rev() {
            to_root ^= self.parity[w];
            self.parity[w] = to_root;
            self.parent[w] = u as u32;
        }
        (u, v != u && self.parity[v])
    }

    /// The literal of `l`'s class root with `l`'s value.
    fn find(&mut self, l: Lit) -> Lit {
        let (root, parity) = self.root(l.var() as usize);
        Lit::new(root as Var, l.is_positive() != parity)
    }

    /// Merge so that `a` and `b` are equal; the variables of the class merged into the other, `None` when they were
    /// already in one class.
    fn union(&mut self, a: Lit, b: Lit) -> Option<Vec<u32>> {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra.var() == rb.var() {
            return None;
        }
        let (keep, gone) = if self.members[ra.var() as usize].len() >= self.members[rb.var() as usize].len() { (ra, rb) } else { (rb, ra) };
        let gone_var = gone.var() as usize;
        self.parent[gone_var] = keep.var();
        self.parity[gone_var] = keep.is_positive() != gone.is_positive();
        let moved = std::mem::take(&mut self.members[gone_var]);
        self.members[keep.var() as usize].extend(&moved);
        Some(moved)
    }
}

impl Db {
    /// Congruence closure: every AND and XOR gate the live clauses define is hashed by its inputs' representatives,
    /// and two gates of one key merge their outputs; each merge requeues the gates reading the merged class, until no
    /// gate merges. A gate whose inputs collapse fixes its output or makes it equal to its one input. Every merge
    /// and fixed literal is proved by RUP before the next is drawn; then each merged variable is substituted by its
    /// class's root.
    fn congruence(&mut self) {
        let gates = self.gates();
        if gates.is_empty() {
            return;
        }
        let mut classes = Classes::new(self.num_vars);
        if !self.seed_equivalences(&mut classes) {
            return;
        }
        let mut readers: Vec<Vec<u32>> = vec![Vec::new(); self.num_vars];
        for (i, g) in gates.iter().enumerate() {
            for l in &g.inputs {
                readers[l.var() as usize].push(i as u32);
            }
        }
        let mut table: std::collections::HashMap<(GateKind, Vec<u32>), (usize, Lit, bool)> = std::collections::HashMap::new();
        let mut queue: std::collections::VecDeque<usize> = (0..gates.len()).collect();
        let mut queued = vec![true; gates.len()];
        let mut merged = false;
        while let Some(i) = queue.pop_front() {
            queued[i] = false;
            if self.unsat || !self.spend(gates[i].inputs.len() + 1) {
                break;
            }
            let moved: Vec<u32> = match self.normal(&gates[i], &mut classes) {
                Normal::Skip => continue,
                Normal::Fixed(l) => {
                    self.prove(&[l], &gates[i], None, false);
                    self.propagate();
                    vec![l.var()]
                }
                Normal::Equal(a, b) => match self.merge(&mut classes, a, b, &gates[i], None, false) {
                    Some(moved) => moved,
                    None => continue,
                },
                Normal::Key { key, output, clean } => match table.get(&key) {
                    None => {
                        table.insert(key, (i, output, clean));
                        continue;
                    }
                    Some(&(j, other, other_clean)) => {
                        let (gi, gj) = (&gates[i], &gates[j]);
                        if gi.inputs.iter().any(|l| l.var() == gj.output.var()) || gj.inputs.iter().any(|l| l.var() == gi.output.var()) {
                            continue;
                        }
                        match self.merge(&mut classes, output, other, &gates[i], Some(&gates[j]), clean && other_clean) {
                            Some(moved) => moved,
                            None => continue,
                        }
                    }
                },
            };
            merged = true;
            for v in moved {
                for &r in &readers[v as usize] {
                    if !queued[r as usize] {
                        queued[r as usize] = true;
                        queue.push_back(r as usize);
                    }
                }
            }
        }
        if !merged || self.unsat {
            return;
        }
        for v in 0..self.num_vars {
            let x = Lit::pos(v as Var);
            let r = classes.find(x);
            if r.var() as usize == v || self.removed[v] || self.value[v].is_some() {
                continue;
            }
            if self.value[r.var() as usize].is_some() || self.removed[r.var() as usize] {
                continue;
            }
            self.replace(x, r);
            if self.unsat {
                return;
            }
            self.propagate();
            if self.unsat {
                return;
            }
        }
    }

    /// Merge the classes of literals equal through the binary clauses — the strongly connected components of the
    /// binary-implication graph, one-input gates — so gates over equal inputs meet before substitution rewrites their
    /// clauses. Each pair is equal by a chain of binary implications present, so the merges need no proof. `false`
    /// when a literal is equal to its own negation, the formula then refuted.
    fn seed_equivalences(&mut self, classes: &mut Classes) -> bool {
        let binaries: Vec<Vec<Lit>> =
            (0..self.clauses.len()).filter(|&i| self.alive[i] && self.clauses[i].len() == 2).map(|i| self.clauses[i].clone()).collect();
        if binaries.is_empty() {
            return true;
        }
        self.spend(binaries.len() * 2);
        let components = match crate::inprocess::equivalent_literal_scc(self.num_vars, &binaries) {
            crate::inprocess::EquivResult::Unsat(steps) => {
                for step in steps {
                    if let ProofStep::Rup(c) = step {
                        if c.is_empty() {
                            self.refute();
                        } else {
                            self.steps.push(ProofStep::Rup(c));
                        }
                    }
                }
                self.refute();
                return false;
            }
            crate::inprocess::EquivResult::Classes(components) => components,
        };
        let node = |l: Lit| l.var() as usize * 2 + usize::from(!l.is_positive());
        let mut first: Vec<Option<Lit>> = vec![None; components.len().max(1)];
        for v in 0..self.num_vars {
            if self.removed[v] || self.value[v].is_some() {
                continue;
            }
            let x = Lit::pos(v as Var);
            match first[components[node(x)]] {
                Some(r) => {
                    classes.union(x, r);
                }
                None => {
                    first[components[node(x)]] = Some(x);
                    first[components[node(x.negated())]] = Some(x.negated());
                }
            }
        }
        true
    }

    /// The gates the live clauses define. An AND `g = a₁ ∧ … ∧ a_k` (`k ≥ 2`) is a clause `g ∨ ¬a₁ ∨ … ∨ ¬a_k` beside
    /// the binaries `¬g ∨ aᵢ`; an XOR over three to [`XOR_ARITY`] variables is a parity gadget
    /// ([`crate::xorsat::xor_gadgets`]), read once with each of its variables as the output; an if-then-else
    /// `g = c ? t : e` is the ternary clauses `¬c ∨ ¬t ∨ g`, `¬c ∨ t ∨ ¬g`, `c ∨ ¬e ∨ g` and `c ∨ e ∨ ¬g`.
    fn gates(&mut self) -> Vec<Gate> {
        let n = self.num_vars;
        let live: Vec<usize> = (0..self.clauses.len()).filter(|&i| self.alive[i]).collect();
        let mut implied: Vec<Vec<u32>> = vec![Vec::new(); 2 * n];
        for &i in &live {
            if let [a, b] = self.clauses[i][..] {
                implied[a.index()].push(b.index() as u32);
                implied[b.index()].push(a.index() as u32);
            }
        }
        for list in implied.iter_mut() {
            list.sort_unstable();
            list.dedup();
        }
        let mut gates = Vec::new();
        for &i in &live {
            let d = &self.clauses[i];
            if d.len() < 3 || d.len() > AND_ARITY + 1 {
                continue;
            }
            let mut spent = d.len();
            for &g in d {
                // The binary `¬g ∨ x` lists `x` under `¬g`: `g` is the output when it lists `¬ℓ` for every other `ℓ`.
                let list = &implied[g.negated().index()];
                if list.len() + 1 < d.len() {
                    continue;
                }
                spent += d.len();
                if d.iter().all(|&l| l == g || list.binary_search(&(l.negated().index() as u32)).is_ok()) {
                    gates.push(Gate { kind: GateKind::And, output: g, inputs: d.iter().filter(|&&l| l != g).map(|l| l.negated()).collect() });
                }
            }
            if !self.spend(spent) {
                return gates;
            }
        }
        let short: Vec<Vec<Lit>> = live.iter().map(|&i| &self.clauses[i]).filter(|c| c.len() >= 3 && c.len() <= XOR_ARITY).cloned().collect();
        self.spend(short.iter().map(|c| c.len()).sum());
        for eq in crate::xorsat::xor_gadgets(n, &short, XOR_ARITY) {
            for (k, &v) in eq.vars.iter().enumerate() {
                let inputs: Vec<Lit> = eq.vars.iter().enumerate().filter(|&(j, _)| j != k).map(|(_, &u)| Lit::pos(u as Var)).collect();
                gates.push(Gate { kind: GateKind::Xor, output: Lit::new(v as Var, !eq.rhs), inputs });
            }
        }
        self.if_then_else_gates(&live, &mut gates);
        gates
    }

    /// The if-then-else gates among the live ternary clauses: for each clause `¬c ∨ ¬t ∨ g` with its partner
    /// `¬c ∨ t ∨ ¬g`, each clause `c ∨ ¬e ∨ g` (found among the ternary clauses holding `c`) with its partner
    /// `c ∨ e ∨ ¬g` makes `g = c ? t : e`.
    fn if_then_else_gates(&mut self, live: &[usize], gates: &mut Vec<Gate>) {
        let key = |a: Lit, b: Lit, c: Lit| {
            let mut k = [a.index() as u32, b.index() as u32, c.index() as u32];
            k.sort_unstable();
            k
        };
        let ternary: Vec<usize> = live.iter().copied().filter(|&i| self.clauses[i].len() == 3).collect();
        let present: std::collections::HashSet<[u32; 3]> = ternary.iter().map(|&i| key(self.clauses[i][0], self.clauses[i][1], self.clauses[i][2])).collect();
        let mut holding: Vec<Vec<u32>> = vec![Vec::new(); 2 * self.num_vars];
        for &i in &ternary {
            for l in &self.clauses[i] {
                holding[l.index()].push(i as u32);
            }
        }
        let mut found: std::collections::HashSet<[u32; 4]> = std::collections::HashSet::new();
        for &i in &ternary {
            let clause = [self.clauses[i][0], self.clauses[i][1], self.clauses[i][2]];
            for gi in 0..3 {
                for ci in 0..3 {
                    if ci == gi {
                        continue;
                    }
                    let (g, c) = (clause[gi], clause[ci].negated());
                    let t = clause[3 - gi - ci].negated();
                    if !present.contains(&key(c.negated(), t, g.negated())) {
                        continue;
                    }
                    let candidates = &holding[c.index()];
                    if !self.spend(candidates.len() + 1) {
                        return;
                    }
                    for &j in candidates {
                        let d = &self.clauses[j as usize];
                        if !d.contains(&g) {
                            continue;
                        }
                        let Some(&not_e) = d.iter().find(|&&l| l != c && l != g) else { continue };
                        let e = not_e.negated();
                        if [c.var(), t.var(), g.var()].contains(&e.var()) || !present.contains(&key(c, e, g.negated())) {
                            continue;
                        }
                        if found.insert([g.index() as u32, c.index() as u32, t.index() as u32, e.index() as u32]) {
                            gates.push(Gate { kind: GateKind::Ite, output: g, inputs: vec![c, t, e] });
                        }
                    }
                }
            }
        }
    }

    /// `gate`'s inputs through the classes and the fixed values.
    fn normal(&self, gate: &Gate, classes: &mut Classes) -> Normal {
        if self.value[gate.output.var() as usize].is_some() || self.removed[gate.output.var() as usize] {
            return Normal::Skip;
        }
        let out = classes.find(gate.output);
        let mut clean = true;
        match gate.kind {
            GateKind::Xor => {
                let mut parity = false;
                let mut reps: Vec<u32> = Vec::with_capacity(gate.inputs.len());
                for &l in &gate.inputs {
                    match self.value[l.var() as usize] {
                        Some(v) => {
                            parity ^= v == l.is_positive();
                            clean = false;
                        }
                        None => {
                            let r = classes.find(l);
                            parity ^= !r.is_positive();
                            reps.push(r.var());
                        }
                    }
                }
                reps.sort_unstable();
                let before = reps.len();
                let mut kept: Vec<u32> = Vec::with_capacity(reps.len());
                for r in reps {
                    if kept.last() == Some(&r) {
                        kept.pop();
                    } else {
                        kept.push(r);
                    }
                }
                clean &= kept.len() == before;
                if kept.contains(&out.var()) {
                    return Normal::Skip;
                }
                // out = ⊕ kept ⊕ parity: the output literal equal to ⊕ kept carries the parity.
                let output = if parity { gate.output.negated() } else { gate.output };
                match kept.len() {
                    0 => Normal::Fixed(output.negated()),
                    1 => Normal::Equal(output, Lit::pos(kept[0])),
                    _ => Normal::Key { key: (GateKind::Xor, kept), output, clean },
                }
            }
            GateKind::And => {
                let mut reps: Vec<u32> = Vec::with_capacity(gate.inputs.len());
                for &l in &gate.inputs {
                    match self.value[l.var() as usize] {
                        Some(v) if v == l.is_positive() => clean = false,
                        Some(_) => return Normal::Fixed(gate.output.negated()),
                        None => reps.push(classes.find(l).index() as u32),
                    }
                }
                and_normal(reps, gate.output, out, clean)
            }
            GateKind::Ite => {
                let value_of = |l: Lit| self.value[l.var() as usize].map(|v| v == l.is_positive());
                let (c, t, e) = (gate.inputs[0], gate.inputs[1], gate.inputs[2]);
                let g = gate.output;
                if let Some(cv) = value_of(c) {
                    let branch = if cv { t } else { e };
                    return match value_of(branch) {
                        Some(true) => Normal::Fixed(g),
                        Some(false) => Normal::Fixed(g.negated()),
                        None => Normal::Equal(g, classes.find(branch)),
                    };
                }
                let c = classes.find(c);
                let code = |l: Lit| l.index() as u32;
                // A fixed branch makes an AND: c ? 1 : e = ¬(¬c ∧ ¬e), c ? 0 : e = ¬c ∧ e, and so on.
                match (value_of(t), value_of(e)) {
                    (Some(true), Some(true)) => return Normal::Fixed(g),
                    (Some(false), Some(false)) => return Normal::Fixed(g.negated()),
                    (Some(true), Some(false)) => return Normal::Equal(g, c),
                    (Some(false), Some(true)) => return Normal::Equal(g, c.negated()),
                    (Some(true), None) => return and_normal(vec![code(c.negated()), code(classes.find(e).negated())], g.negated(), out, false),
                    (Some(false), None) => return and_normal(vec![code(c.negated()), code(classes.find(e))], g, out, false),
                    (None, Some(true)) => return and_normal(vec![code(c), code(classes.find(t).negated())], g.negated(), out, false),
                    (None, Some(false)) => return and_normal(vec![code(c), code(classes.find(t))], g, out, false),
                    (None, None) => {}
                }
                let (mut c, mut t, mut e) = (c, classes.find(t), classes.find(e));
                // c ? t : e = ¬c ? e : t, and c ? ¬t : ¬e = ¬(c ? t : e): the condition and the then-branch positive.
                if !c.is_positive() {
                    c = c.negated();
                    std::mem::swap(&mut t, &mut e);
                }
                if t == e {
                    return Normal::Equal(g, t);
                }
                if t == e.negated() {
                    // c ? t : ¬t = ¬(c ⊕ t): the literal equal to the XOR of the two variables.
                    if t.var() == c.var() || t.var() == out.var() || c.var() == out.var() {
                        return Normal::Skip;
                    }
                    let output = if t.is_positive() { g.negated() } else { g };
                    let mut key = vec![c.var(), t.var()];
                    key.sort_unstable();
                    return Normal::Key { key: (GateKind::Xor, key), output, clean: false };
                }
                // A branch on the condition makes an AND: c ? c : e = ¬(¬c ∧ ¬e), c ? ¬c : e = ¬c ∧ e, c ? t : c = c ∧ t,
                // c ? t : ¬c = ¬(c ∧ ¬t).
                if t == c {
                    return and_normal(vec![code(c.negated()), code(e.negated())], g.negated(), out, false);
                }
                if t == c.negated() {
                    return and_normal(vec![code(c.negated()), code(e)], g, out, false);
                }
                if e == c {
                    return and_normal(vec![code(c), code(t)], g, out, false);
                }
                if e == c.negated() {
                    return and_normal(vec![code(c), code(t.negated())], g.negated(), out, false);
                }
                if [c.var(), t.var(), e.var()].contains(&out.var()) {
                    return Normal::Skip;
                }
                let mut output = g;
                if !t.is_positive() {
                    t = t.negated();
                    e = e.negated();
                    output = output.negated();
                }
                Normal::Key { key: (GateKind::Ite, vec![code(c), code(t), code(e)]), output, clean }
            }
        }
    }

    /// Merge `a` and `b`, proving both directions — through `first` alone when one gate makes them equal, through
    /// `first` and `second` when two congruent gates do — and refuting when they were already opposite. The variables
    /// whose class changed, `None` when nothing did.
    fn merge(&mut self, classes: &mut Classes, a: Lit, b: Lit, first: &Gate, second: Option<&Gate>, clean: bool) -> Option<Vec<u32>> {
        let (ra, rb) = (classes.find(a), classes.find(b));
        if ra == rb {
            return None;
        }
        self.prove(&[a.negated(), b], first, second, clean);
        self.prove(&[a, b.negated()], first, second, clean);
        self.congruent += 1;
        if ra == rb.negated() {
            self.derive(vec![a.negated()]);
            self.derive(vec![a]);
            self.refute();
            return None;
        }
        classes.union(a, b)
    }

    /// Derive `clause`, which `first` (with `second`, two gates over equal inputs) implies under the equalities and
    /// fixed literals derived before it. An AND's consequence is RUP outright: unit propagation runs through the
    /// gates' binaries and the equalities' binaries to a falsified long clause. An XOR's or an if-then-else's is not,
    /// so it is proved by a tree of case splits: every leaf adds signs on the split variables, each RUP because its
    /// negation assigns all but at most one variable of a gadget (an if-then-else's condition decides which branch
    /// its output follows), which unit propagation then finishes into a conflict, and each inner node is RUP from its
    /// two children. Two clean XORs (inputs one to one) split on all but the first gate's last input; otherwise every
    /// XOR splits on all its inputs and every if-then-else on its condition. The tree's clauses are deleted once
    /// `clause` stands.
    fn prove(&mut self, clause: &[Lit], first: &Gate, second: Option<&Gate>, clean: bool) {
        let splits_of = |g: &Gate| -> Vec<Var> {
            match g.kind {
                GateKind::And => Vec::new(),
                GateKind::Ite => vec![g.inputs[0].var()],
                GateKind::Xor => g.inputs.iter().map(|l| l.var()).collect(),
            }
        };
        if first.kind == GateKind::And && second.is_none_or(|g| g.kind == GateKind::And) {
            self.derive(clause.to_vec());
            return;
        }
        let mut splits: Vec<Var> = match second {
            Some(other) if clean && first.kind == GateKind::Xor && other.kind == GateKind::Xor => {
                first.inputs[..first.inputs.len() - 1].iter().map(|l| l.var()).collect()
            }
            _ => splits_of(first).into_iter().chain(second.map(splits_of).unwrap_or_default()).collect(),
        };
        splits.sort_unstable();
        splits.dedup();
        splits.retain(|&v| clause.iter().all(|l| l.var() != v));
        let mut tree: Vec<Vec<Lit>> = Vec::new();
        let mut signs: Vec<Lit> = Vec::new();
        cases(clause, &splits, &mut signs, &mut tree);
        self.spend(tree.len() * (clause.len() + splits.len()));
        for c in &tree {
            self.steps.push(ProofStep::Rup(c.clone()));
        }
        self.derive(clause.to_vec());
        for c in tree {
            self.steps.push(ProofStep::Delete(c));
        }
    }
}

/// An AND's normal form over the literal codes of its inputs' representatives: a complementary pair fixes `output`
/// false, no input fixes it true, one input makes it that input, and otherwise the sorted codes are its key. `root`
/// is the output's class root, which no input may name.
fn and_normal(mut reps: Vec<u32>, output: Lit, root: Lit, clean: bool) -> Normal {
    reps.sort_unstable();
    let before = reps.len();
    reps.dedup();
    let clean = clean && reps.len() == before;
    if reps.windows(2).any(|w| w[0] ^ 1 == w[1]) {
        return Normal::Fixed(output.negated());
    }
    if reps.iter().any(|&r| r / 2 == root.var()) {
        return Normal::Skip;
    }
    match reps.len() {
        0 => Normal::Fixed(output),
        1 => Normal::Equal(output, Lit::new(reps[0] / 2, reps[0] % 2 == 0)),
        _ => Normal::Key { key: (GateKind::And, reps), output, clean },
    }
}

/// The clauses of a case-split tree under `clause`, leaves before the node they resolve to: `clause` with `signs` and
/// every sign pattern on `splits`.
fn cases(clause: &[Lit], splits: &[Var], signs: &mut Vec<Lit>, out: &mut Vec<Vec<Lit>>) {
    let Some((&v, rest)) = splits.split_first() else {
        out.push(clause.iter().chain(signs.iter()).copied().collect());
        return;
    };
    for l in [Lit::pos(v), Lit::neg(v)] {
        signs.push(l);
        cases(clause, rest, signs, out);
        signs.pop();
    }
    if !signs.is_empty() {
        out.push(clause.iter().chain(signs.iter()).copied().collect());
    }
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_proof/preprocess/tests.rs"]
mod tests;
