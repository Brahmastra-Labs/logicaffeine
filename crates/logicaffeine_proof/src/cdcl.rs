//! A conflict-driven clause-learning (CDCL) SAT core — the modern competition-grade
//! engine, built to the lineage that made SAT practical: two-watched-literal unit
//! propagation (Moskewicz et al., *Chaff*, 2001), first-UIP conflict analysis with
//! learned clauses and non-chronological backtracking (Marques-Silva & Sakallah,
//! *GRASP*, 1996), VSIDS-style activity decisions, and Luby restarts (Luby et al.,
//! 1993). The clean reference implementation it follows is Eén & Sörensson's *MiniSat*
//! (2003).
//!
//! This module is the propositional substrate of a **DPLL(T)** engine (Nieuwenhuis,
//! Oliveras & Tinelli, JACM 2006): theory propagators (an AllDifferent GAC filter for
//! grid categories, EUF, LIA, …) plug in through [`Theory`], and every learned clause is
//! a resolvent the solver can log as a **DRAT/LRAT** proof step (Wetzler/Heule/Hunt,
//! 2014; Cruz-Filipe et al., 2017) for a downstream linear checker.
//!
//! It is deliberately self-contained and value-typed (`Var = u32`, `Lit` a packed
//! sign+index) so it can be exercised against a brute-force oracle in isolation before
//! it is wired to the grid encoder — a SAT core is only as good as its cross-checks.

use std::collections::VecDeque;

/// A propositional variable: an index `0..num_vars`.
pub type Var = u32;

/// A literal: a variable plus a sign, packed as `var << 1 | negated`. Packing keeps the
/// watch lists and the trail cache-dense, the way every fast solver stores them.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Lit(u32);

impl Lit {
    /// The positive literal `+v`.
    #[inline]
    pub fn pos(v: Var) -> Lit {
        Lit(v << 1)
    }
    /// The negative literal `¬v`.
    #[inline]
    pub fn neg(v: Var) -> Lit {
        Lit((v << 1) | 1)
    }
    /// Build from a variable and a sign (`true` ⇒ positive).
    #[inline]
    pub fn new(v: Var, positive: bool) -> Lit {
        if positive {
            Lit::pos(v)
        } else {
            Lit::neg(v)
        }
    }
    /// The underlying variable.
    #[inline]
    pub fn var(self) -> Var {
        self.0 >> 1
    }
    /// Whether this is a positive literal.
    #[inline]
    pub fn is_positive(self) -> bool {
        self.0 & 1 == 0
    }
    /// The complementary literal.
    #[inline]
    pub fn negated(self) -> Lit {
        Lit(self.0 ^ 1)
    }
    /// The dense index for watch/seen arrays (`2*var + sign`).
    #[inline]
    fn index(self) -> usize {
        self.0 as usize
    }
}

/// A three-valued assignment cell.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Val {
    True,
    False,
    Unset,
}

/// Reason a variable was assigned, for the implication graph that conflict analysis
/// walks. `Decision` is a branching guess; `Clause(ci)` means clause `ci` became unit
/// and forced this literal (its other literals are all false, earlier on the trail).
#[derive(Clone, Copy, Debug)]
enum Reason {
    Decision,
    Clause(usize),
}

/// The verdict of [`Solver::solve`].
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum SolveResult {
    /// Satisfiable, with a full model over `0..num_vars` (`true`/`false` per variable).
    Sat(Vec<bool>),
    /// Unsatisfiable.
    Unsat,
}

/// A learned clause logged for proof reconstruction. Each is a resolvent derivable from
/// the formula by reverse unit propagation — the unit of a DRAT/LRAT proof.
#[derive(Clone, Debug)]
pub struct LearnedClause {
    pub lits: Vec<Lit>,
}

/// A theory propagator (the DPLL(T) seam). The SAT core calls [`Theory::propagate`] at
/// each fixpoint of Boolean propagation; a theory returns a fresh clause to add (an
/// explanation/conflict) or `None` if it has nothing to say. AllDifferent GAC, EUF, and
/// LIA all implement this without the core knowing the theory.
pub trait Theory {
    /// Given the current partial assignment (`lit_value`), return a clause that is
    /// theory-entailed and currently unit or falsified (so the core will propagate or
    /// conflict on it), or `None` at a theory fixpoint. The returned clause MUST be a
    /// sound consequence of the theory over the shared Boolean variables.
    fn propagate(&mut self, assign: &dyn Fn(Lit) -> Option<bool>) -> Option<Vec<Lit>>;
}

/// The CDCL solver.
pub struct Solver {
    num_vars: usize,
    /// All clauses (original + learned); a clause is a `Vec<Lit>`.
    clauses: Vec<Vec<Lit>>,
    /// Watch lists: for each literal, the clauses watching it. Two-watched-literal
    /// scheme — a clause is visited only when one of its two watched literals is falsified.
    watches: Vec<Vec<usize>>,
    /// Per-variable value.
    value: Vec<Val>,
    /// Per-variable decision level.
    level: Vec<u32>,
    /// Per-variable reason.
    reason: Vec<Reason>,
    /// The assignment trail (literals in assignment order).
    trail: Vec<Lit>,
    /// Index into `trail` where each decision level begins.
    trail_lim: Vec<usize>,
    /// Head of the propagation queue (index into `trail`).
    qhead: usize,
    /// VSIDS activities and the bump/decay.
    activity: Vec<f64>,
    var_inc: f64,
    /// Learned clauses, logged for proof output.
    learned_log: Vec<LearnedClause>,
    /// Scratch `seen` markers for conflict analysis (indexed by var).
    seen: Vec<bool>,
    /// Set once an empty clause is added — the formula is then trivially unsatisfiable.
    empty_clause: bool,
    /// Number of clauses present when the solve began (the original formula); clauses at
    /// indices `>= n_original` are learned. Lets a downstream checker read the original
    /// clauses without a separate copy.
    n_original: usize,
}

impl Solver {
    /// A fresh solver over `num_vars` variables and no clauses.
    pub fn new(num_vars: usize) -> Self {
        Solver {
            num_vars,
            clauses: Vec::new(),
            watches: vec![Vec::new(); num_vars * 2],
            value: vec![Val::Unset; num_vars],
            level: vec![0; num_vars],
            reason: vec![Reason::Decision; num_vars],
            trail: Vec::new(),
            trail_lim: Vec::new(),
            qhead: 0,
            activity: vec![0.0; num_vars],
            var_inc: 1.0,
            learned_log: Vec::new(),
            seen: vec![false; num_vars],
            empty_clause: false,
            n_original: 0,
        }
    }

    /// The learned clauses produced during the last solve (the DRAT/LRAT proof skeleton).
    pub fn learned(&self) -> &[LearnedClause] {
        &self.learned_log
    }

    /// The ORIGINAL clauses (those present before solving), borrowed in place — so a RUP
    /// checker can replay over them without copying the clause set.
    pub fn original_clauses(&self) -> &[Vec<Lit>] {
        &self.clauses[..self.n_original]
    }

    /// Add a clause. An empty clause makes the formula trivially unsatisfiable; a unit
    /// clause is enqueued as a top-level fact.
    pub fn add_clause(&mut self, mut lits: Vec<Lit>) {
        // Dedup and drop a clause that is already a tautology (`v ∨ ¬v`).
        lits.sort_by_key(|l| l.0);
        lits.dedup();
        for w in lits.windows(2) {
            if w[0].var() == w[1].var() {
                // contains both polarities of some var → tautology, skip
                return;
            }
        }
        self.add_clause_raw(lits, false);
    }

    /// Internal: register a clause and set up its two watches. `learned` clauses are also
    /// logged. Returns the clause index.
    fn add_clause_raw(&mut self, lits: Vec<Lit>, learned: bool) -> usize {
        let ci = self.clauses.len();
        if learned {
            self.learned_log.push(LearnedClause { lits: lits.clone() });
        }
        if lits.is_empty() {
            self.empty_clause = true;
            self.clauses.push(lits);
            return ci;
        }
        if lits.len() == 1 {
            let l = lits[0];
            self.clauses.push(lits);
            // A unit clause forces its literal. If it is already FALSE on the trail, the
            // formula is unsatisfiable (a top-level conflict); if already true, nothing to
            // do; else enqueue it.
            match self.val_of(l) {
                Val::True => {}
                Val::False => self.empty_clause = true,
                Val::Unset => self.enqueue(l, Reason::Clause(ci)),
            }
            return ci;
        }
        // Watch the first two literals.
        self.watches[lits[0].index()].push(ci);
        self.watches[lits[1].index()].push(ci);
        self.clauses.push(lits);
        ci
    }

    #[inline]
    fn val_of(&self, l: Lit) -> Val {
        match self.value[l.var() as usize] {
            Val::Unset => Val::Unset,
            v => {
                if l.is_positive() {
                    v
                } else {
                    match v {
                        Val::True => Val::False,
                        Val::False => Val::True,
                        Val::Unset => Val::Unset,
                    }
                }
            }
        }
    }

    #[inline]
    fn lit_true(&self, l: Lit) -> bool {
        self.val_of(l) == Val::True
    }
    #[inline]
    fn lit_false(&self, l: Lit) -> bool {
        self.val_of(l) == Val::False
    }

    /// Assign `l` true with the given reason, push it on the trail. Assumes `l` is currently
    /// unset (callers check).
    fn enqueue(&mut self, l: Lit, r: Reason) {
        let v = l.var() as usize;
        self.value[v] = if l.is_positive() { Val::True } else { Val::False };
        self.level[v] = self.trail_lim.len() as u32;
        self.reason[v] = r;
        self.trail.push(l);
    }

    /// Two-watched-literal unit propagation. Returns the index of a conflicting clause, or
    /// `None` at a Boolean fixpoint.
    fn propagate(&mut self) -> Option<usize> {
        while self.qhead < self.trail.len() {
            let p = self.trail[self.qhead];
            self.qhead += 1;
            // Clauses watching ¬p may have become unit/false.
            let false_lit = p.negated();
            let mut wi = 0;
            // Take the watch list out to satisfy the borrow checker; we rebuild it.
            let mut watchers = std::mem::take(&mut self.watches[false_lit.index()]);
            'next_clause: while wi < watchers.len() {
                let ci = watchers[wi];
                let clause_len = self.clauses[ci].len();
                // Ensure the watched literal we keep is in slot 0/1; make slot 1 = false_lit.
                if self.clauses[ci][0] == false_lit {
                    self.clauses[ci].swap(0, 1);
                }
                // If slot 0 is already satisfied, the clause is fine — keep watching false_lit.
                let other = self.clauses[ci][0];
                if self.lit_true(other) {
                    wi += 1;
                    continue;
                }
                // Look for a new, non-false literal to watch (slots 2..).
                for k in 2..clause_len {
                    let lk = self.clauses[ci][k];
                    if !self.lit_false(lk) {
                        // Move lk into slot 1, watch it, drop this watch.
                        self.clauses[ci].swap(1, k);
                        self.watches[lk.index()].push(ci);
                        watchers.swap_remove(wi);
                        continue 'next_clause;
                    }
                }
                // No new watch: clause is unit on `other` or conflicting.
                if self.lit_false(other) {
                    // Conflict: restore the rest of this watch list and report.
                    self.watches[false_lit.index()] = watchers;
                    return Some(ci);
                }
                // Unit: propagate `other`.
                self.enqueue(other, Reason::Clause(ci));
                wi += 1;
            }
            self.watches[false_lit.index()] = watchers;
        }
        None
    }

    /// First-UIP conflict analysis (GRASP/MiniSat). Walks the implication graph back from
    /// the conflicting clause to the first unique implication point at the current level,
    /// producing a learned clause and the level to backjump to.
    fn analyze(&mut self, conflict: usize) -> (Vec<Lit>, u32) {
        let decision_level = self.trail_lim.len() as u32;
        let mut learned: Vec<Lit> = vec![Lit::pos(0)]; // slot 0 reserved for the UIP
        let mut counter = 0usize; // unresolved literals at the current level
        let mut p: Option<Lit> = None;
        let mut trail_idx = self.trail.len();
        let mut clause = conflict;

        loop {
            // Resolve with `clause` (its literals are all false; for the conflict clause,
            // and for each antecedent reason clause thereafter).
            let lits = self.clauses[clause].clone();
            for &q in &lits {
                if Some(q) == p {
                    continue; // skip the pivot we just resolved on
                }
                let v = q.var() as usize;
                if !self.seen[v] && self.level[v] > 0 {
                    self.bump(q.var());
                    self.seen[v] = true;
                    if self.level[v] == decision_level {
                        counter += 1;
                    } else {
                        learned.push(q);
                    }
                }
            }
            // Pick the next literal to resolve: the most recent seen literal on the trail.
            loop {
                trail_idx -= 1;
                let l = self.trail[trail_idx];
                if self.seen[l.var() as usize] {
                    p = Some(l);
                    break;
                }
            }
            let pv = p.unwrap().var() as usize;
            self.seen[pv] = false;
            counter -= 1;
            if counter == 0 {
                break;
            }
            clause = match self.reason[pv] {
                Reason::Clause(ci) => ci,
                Reason::Decision => unreachable!("UIP reached a decision before counter hit 0"),
            };
        }
        // The asserting literal is ¬p (p is the UIP, currently true).
        learned[0] = p.unwrap().negated();
        // Clear remaining `seen`, and move the highest-level literal into slot 1. The
        // two-watched-literal invariant requires the second watch to be the most recently
        // falsified literal (the backjump level); otherwise the learned clause can be
        // falsified later without the watch firing — silently missing conflicts.
        let mut backjump = 0u32;
        let mut max_idx = 1usize;
        for i in 1..learned.len() {
            let lv = self.level[learned[i].var() as usize];
            self.seen[learned[i].var() as usize] = false;
            if lv > backjump {
                backjump = lv;
                max_idx = i;
            }
        }
        if learned.len() >= 2 {
            learned.swap(1, max_idx);
        }
        (learned, backjump)
    }

    fn bump(&mut self, v: Var) {
        self.activity[v as usize] += self.var_inc;
        if self.activity[v as usize] > 1e100 {
            for a in self.activity.iter_mut() {
                *a *= 1e-100;
            }
            self.var_inc *= 1e-100;
        }
    }

    fn decay(&mut self) {
        self.var_inc /= 0.95;
    }

    /// Undo assignments down to (but not including) `level`.
    fn backtrack_to(&mut self, level: u32) {
        if self.trail_lim.len() as u32 <= level {
            return;
        }
        let target = self.trail_lim[level as usize];
        while self.trail.len() > target {
            let l = self.trail.pop().unwrap();
            self.value[l.var() as usize] = Val::Unset;
        }
        self.qhead = target;
        self.trail_lim.truncate(level as usize);
    }

    /// Pick an unassigned variable of highest VSIDS activity; `None` if all assigned.
    fn pick_branch(&self) -> Option<Var> {
        let mut best: Option<Var> = None;
        let mut best_act = f64::NEG_INFINITY;
        for v in 0..self.num_vars {
            if self.value[v] == Val::Unset && self.activity[v] > best_act {
                best_act = self.activity[v];
                best = Some(v as Var);
            }
        }
        best
    }

    /// Solve, optionally under a list of theory propagators (DPLL(T)). Returns a model or
    /// `Unsat`. The learned-clause log is available afterwards via [`Solver::learned`].
    pub fn solve(&mut self) -> SolveResult {
        self.solve_with(&mut [])
    }

    /// Solve with theory propagators. Each is consulted at every Boolean fixpoint; a
    /// returned clause is added to the formula (and may immediately propagate or conflict).
    pub fn solve_with(&mut self, theories: &mut [Box<dyn Theory>]) -> SolveResult {
        self.n_original = self.clauses.len();
        if self.empty_clause {
            return SolveResult::Unsat;
        }
        // Top-level propagation of any unit clauses already enqueued.
        if self.propagate().is_some() {
            return SolveResult::Unsat;
        }
        let mut conflicts_since_restart = 0u64;
        let mut restart_limit = luby(1) * 100;
        let mut restart_no = 1u64;

        loop {
            let conflict = self.propagate();
            if let Some(ci) = conflict {
                if self.trail_lim.is_empty() {
                    return SolveResult::Unsat; // conflict at level 0
                }
                let (learned, backjump) = self.analyze(ci);
                self.backtrack_to(backjump);
                let asserting = learned[0];
                let unit = learned.len() == 1;
                let new_ci = self.add_clause_raw(learned, true);
                // The learned clause is asserting: enqueue its UIP literal — but a unit
                // learned clause was ALREADY enqueued by `add_clause_raw`'s unit path, so
                // only enqueue here for the multi-literal case (avoid a double-push).
                if !unit {
                    self.enqueue(asserting, Reason::Clause(new_ci));
                }
                self.decay();
                conflicts_since_restart += 1;
                if conflicts_since_restart >= restart_limit {
                    self.backtrack_to(0);
                    conflicts_since_restart = 0;
                    restart_no += 1;
                    restart_limit = luby(restart_no) * 100;
                }
                continue;
            }
            // Boolean fixpoint — consult theories before branching.
            let mut theory_added = false;
            for t in theories.iter_mut() {
                let snapshot_value = &self.value;
                let assign = |l: Lit| -> Option<bool> {
                    match snapshot_value[l.var() as usize] {
                        Val::Unset => None,
                        Val::True => Some(l.is_positive()),
                        Val::False => Some(!l.is_positive()),
                    }
                };
                if let Some(clause) = t.propagate(&assign) {
                    self.add_clause(clause);
                    theory_added = true;
                    break;
                }
            }
            if theory_added {
                continue;
            }
            // Decide.
            match self.pick_branch() {
                None => {
                    // Full assignment → SAT.
                    let model = (0..self.num_vars)
                        .map(|v| self.value[v] == Val::True)
                        .collect();
                    return SolveResult::Sat(model);
                }
                Some(v) => {
                    self.trail_lim.push(self.trail.len());
                    // Phase: default false (MiniSat-style negative-first is a fine default).
                    self.enqueue(Lit::neg(v), Reason::Decision);
                }
            }
        }
    }
}

/// The Luby restart sequence `1,1,2,1,1,2,4,1,…` (Luby, Sinclair & Zuckerman, 1993) —
/// the optimal universal restart schedule.
fn luby(mut i: u64) -> u64 {
    // 1-indexed Luby.
    let mut k = 1u32;
    loop {
        let span = (1u64 << k) - 1;
        if i == span {
            return 1u64 << (k - 1);
        }
        if i < span {
            i -= (1u64 << (k - 1)) - 1;
            k = 1;
            continue;
        }
        k += 1;
    }
}

/// Suppress the unused-queue warning until the inprocessing pass that will use it lands.
#[allow(dead_code)]
type _UnusedQueue = VecDeque<Lit>;

#[cfg(test)]
mod tests {
    use super::*;

    fn sat_brute(num_vars: usize, clauses: &[Vec<Lit>]) -> bool {
        // Enumerate all 2^n assignments; true iff some assignment satisfies every clause.
        for mask in 0u64..(1u64 << num_vars) {
            let val = |v: Var| (mask >> v) & 1 == 1;
            let ok = clauses.iter().all(|c| {
                c.iter().any(|l| {
                    let b = val(l.var());
                    if l.is_positive() {
                        b
                    } else {
                        !b
                    }
                })
            });
            if ok {
                return true;
            }
        }
        false
    }

    fn check_model(clauses: &[Vec<Lit>], model: &[bool]) -> bool {
        clauses.iter().all(|c| {
            c.iter().any(|l| {
                let b = model[l.var() as usize];
                if l.is_positive() {
                    b
                } else {
                    !b
                }
            })
        })
    }

    #[test]
    fn unit_and_empty() {
        // Empty clause ⇒ Unsat.
        let mut s = Solver::new(1);
        s.add_clause(vec![]);
        assert_eq!(s.solve(), SolveResult::Unsat);

        // x ∧ ¬x ⇒ Unsat.
        let mut s = Solver::new(1);
        s.add_clause(vec![Lit::pos(0)]);
        s.add_clause(vec![Lit::neg(0)]);
        assert_eq!(s.solve(), SolveResult::Unsat);
    }

    #[test]
    fn tiny_sat() {
        // (x ∨ y) ∧ (¬x ∨ y) ∧ (¬y ∨ z): forces y, then z; x free.
        let mut s = Solver::new(3);
        s.add_clause(vec![Lit::pos(0), Lit::pos(1)]);
        s.add_clause(vec![Lit::neg(0), Lit::pos(1)]);
        s.add_clause(vec![Lit::neg(1), Lit::pos(2)]);
        match s.solve() {
            SolveResult::Sat(m) => {
                assert!(m[1] && m[2], "y and z forced true");
            }
            SolveResult::Unsat => panic!("should be SAT"),
        }
    }

    #[test]
    fn pigeonhole_3_into_2_unsat() {
        // 3 pigeons, 2 holes: p_{i,h} = pigeon i in hole h. Each pigeon in some hole; no
        // two pigeons share a hole. Classic UNSAT (PHP) — exercises conflict learning.
        let var = |i: usize, h: usize| (i * 2 + h) as Var;
        let mut s = Solver::new(6);
        for i in 0..3 {
            s.add_clause(vec![Lit::pos(var(i, 0)), Lit::pos(var(i, 1))]);
        }
        for h in 0..2 {
            for i in 0..3 {
                for j in (i + 1)..3 {
                    s.add_clause(vec![Lit::neg(var(i, h)), Lit::neg(var(j, h))]);
                }
            }
        }
        assert_eq!(s.solve(), SolveResult::Unsat);
    }

    #[test]
    fn random_against_brute_force() {
        // Deterministic pseudo-random 3-CNFs over up to 6 vars; cross-check SAT/UNSAT and
        // validate every returned model. The only honest way to trust a SAT core.
        let mut state = 0x9e3779b97f4a7c15u64;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for _trial in 0..400 {
            let num_vars = 3 + (next() % 4) as usize; // 3..6
            let num_clauses = 3 + (next() % 12) as usize;
            let mut clauses = Vec::new();
            for _ in 0..num_clauses {
                let mut c = Vec::new();
                for _ in 0..3 {
                    let v = (next() % num_vars as u64) as Var;
                    let positive = next() & 1 == 0;
                    c.push(Lit::new(v, positive));
                }
                clauses.push(c);
            }
            let expected = sat_brute(num_vars, &clauses);
            let mut s = Solver::new(num_vars);
            for c in &clauses {
                s.add_clause(c.clone());
            }
            match s.solve() {
                SolveResult::Sat(m) => {
                    assert!(expected, "solver said SAT but brute force says UNSAT");
                    assert!(check_model(&clauses, &m), "returned model does not satisfy the formula");
                }
                SolveResult::Unsat => {
                    assert!(!expected, "solver said UNSAT but brute force found a model");
                }
            }
        }
    }
}
