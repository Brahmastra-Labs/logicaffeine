//! A fast, certified finite-domain (logic-grid) solver.
//!
//! Logic-grid puzzles ground to a quantifier-free clause/rule set; a 4×4 grid is a
//! small SAT(+equality) problem that a determined-CSP solver closes in milliseconds.
//! The wall the previous engine hit was speed: `engine::cert_saturate` re-saturates
//! `O(facts²)` at every search node. This module is the incremental replacement —
//! trail-based watched-literal unit propagation + DPLL — that finds the answer fast.
//!
//! Crucially, certification is DECOUPLED from search (see [`crate::verify`]): the
//! solver finds the model/refutation however it likes, then a post-pass emits a
//! [`DerivationTree`] from the recorded reasons, reusing the certified inference
//! rules. [`crate::verify::check_derivation`] re-checks that tree in the kernel, so
//! the solver sits OUTSIDE the trusted base — a wrong tree yields `verified == false`,
//! never a false claim.
//!
//! Build order (see plan): Phase A de-risks the emitter (this file's tests prove the
//! certified tree SHAPES the solver will produce actually certify), before any search
//! machinery exists.

#![allow(dead_code)]

use crate::{DerivationTree, InferenceRule, ProofExpr};

// ── Tree constructors (the exact certified shapes the emitter produces) ──────────

fn leaf(e: ProofExpr) -> DerivationTree {
    DerivationTree::leaf(e, InferenceRule::PremiseMatch)
}

fn neg(e: &ProofExpr) -> ProofExpr {
    match e {
        ProofExpr::Not(inner) => (**inner).clone(),
        _ => ProofExpr::Not(Box::new(e.clone())),
    }
}

fn falsum() -> ProofExpr {
    ProofExpr::Atom("⊥".into())
}

/// ModusPonens: from `impl_tree : A → C` and `ante_tree : A`, conclude `C`.
fn modus_ponens(c: ProofExpr, impl_tree: DerivationTree, ante_tree: DerivationTree) -> DerivationTree {
    DerivationTree::new(c, InferenceRule::ModusPonens, vec![impl_tree, ante_tree])
}

/// Disjunctive syllogism: from `or_tree : A ∨ B` and `neg_tree : ¬A`, conclude `B`.
fn disjunction_elim(survivor: ProofExpr, or_tree: DerivationTree, neg_tree: DerivationTree) -> DerivationTree {
    DerivationTree::new(survivor, InferenceRule::DisjunctionElim, vec![or_tree, neg_tree])
}

/// Conjunction introduction: from `l : A` and `r : B`, conclude `A ∧ B`.
fn conj_intro(a: ProofExpr, b: ProofExpr, l: DerivationTree, r: DerivationTree) -> DerivationTree {
    DerivationTree::new(ProofExpr::And(Box::new(a), Box::new(b)), InferenceRule::ConjunctionIntro, vec![l, r])
}

/// Contradiction: from `P` and `¬P`, conclude `⊥` (the certifier finds the polarity).
fn contradiction(p: DerivationTree, np: DerivationTree) -> DerivationTree {
    DerivationTree::new(falsum(), InferenceRule::Contradiction, vec![p, np])
}

/// Reductio: assume `assumed`, derive `⊥` (`falsum_tree`), conclude `¬assumed`.
fn reductio(assumed: ProofExpr, falsum_tree: DerivationTree) -> DerivationTree {
    let concl = ProofExpr::Not(Box::new(assumed.clone()));
    DerivationTree::new(concl, InferenceRule::ReductioAdAbsurdum, vec![leaf(assumed), falsum_tree])
}

/// Equality symmetry: from `a = b`, conclude `b = a`.
fn eq_sym(a: crate::ProofTerm, b: crate::ProofTerm, tree: DerivationTree) -> DerivationTree {
    DerivationTree::new(ProofExpr::Identity(b, a), InferenceRule::EqualitySymmetry, vec![tree])
}

/// Disjunction case analysis to a common conclusion: from `A ∨ B` and a proof of the
/// conclusion in each branch (with the disjunct bound as a local hypothesis), conclude
/// the common goal.
fn disjunction_cases(goal: ProofExpr, or_tree: DerivationTree, left: DerivationTree, right: DerivationTree) -> DerivationTree {
    DerivationTree::new(goal, InferenceRule::DisjunctionCases, vec![or_tree, left, right])
}

/// Conjunction elimination: from `A ∧ B`, project the named `conjunct`.
fn conj_elim(conjunct: ProofExpr, and_tree: DerivationTree) -> DerivationTree {
    DerivationTree::new(conjunct, InferenceRule::ConjunctionElim, vec![and_tree])
}

// ── Atoms & literals ─────────────────────────────────────────────────────────────

use std::collections::HashMap;
use crate::ProofTerm;

type Var = usize;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
struct Lit {
    var: Var,
    pos: bool,
}

impl Lit {
    fn negate(self) -> Lit {
        Lit { var: self.var, pos: !self.pos }
    }
}

fn term_key(t: &ProofTerm) -> String {
    t.to_string()
}

/// Canonical key for a POSITIVE atom. `Identity` is normalized to an unordered pair so
/// `a = b` and `b = a` intern to the SAME variable — they are the same proposition.
fn atom_key(e: &ProofExpr) -> Option<String> {
    match e {
        ProofExpr::Predicate { name, args, .. } => {
            let parts: Vec<String> = args.iter().map(term_key).collect();
            Some(format!("P:{}({})", name, parts.join(",")))
        }
        ProofExpr::Identity(a, b) => {
            let (ka, kb) = (term_key(a), term_key(b));
            let (lo, hi) = if ka <= kb { (ka, kb) } else { (kb, ka) };
            Some(format!("=:{}|{}", lo, hi))
        }
        ProofExpr::Atom(s) => Some(format!("A:{}", s)),
        _ => None,
    }
}

fn is_atomish(e: &ProofExpr) -> bool {
    matches!(
        e,
        ProofExpr::Predicate { .. } | ProofExpr::Identity(..) | ProofExpr::Atom(_)
    )
}

fn is_literal(e: &ProofExpr) -> bool {
    match e {
        ProofExpr::Not(inner) => is_atomish(inner),
        _ => is_atomish(e),
    }
}

/// A clause disjunct the solver can decide on: a literal, or a conjunction whose every
/// leaf is a literal (an of-pair disjunct). A disjunct carrying an implication or
/// quantifier (e.g. a grounded `∃`-existence clause) is rejected, so `compile` skips it.
fn is_clause_disjunct(e: &ProofExpr) -> bool {
    match e {
        ProofExpr::And(l, r) | ProofExpr::Or(l, r) => is_clause_disjunct(l) && is_clause_disjunct(r),
        _ => is_literal(e),
    }
}

fn flatten_and(e: &ProofExpr, out: &mut Vec<ProofExpr>) {
    match e {
        ProofExpr::And(l, r) => {
            flatten_and(l, out);
            flatten_and(r, out);
        }
        _ => out.push(e.clone()),
    }
}

fn flatten_or(e: &ProofExpr, out: &mut Vec<ProofExpr>) {
    match e {
        ProofExpr::Or(l, r) => {
            flatten_or(l, out);
            flatten_or(r, out);
        }
        _ => out.push(e.clone()),
    }
}

/// Intern every atom occurring in `e` (so a decision clause's compound disjuncts get
/// variables even though they never appear as standalone literals).
fn intern_all(table: &mut AtomTable, e: &ProofExpr) {
    match e {
        ProofExpr::And(l, r) | ProofExpr::Or(l, r) | ProofExpr::Implies(l, r) | ProofExpr::Iff(l, r) => {
            intern_all(table, l);
            intern_all(table, r);
        }
        ProofExpr::Not(inner) => intern_all(table, inner),
        _ if is_atomish(e) => {
            table.intern(e);
        }
        _ => {}
    }
}

#[derive(Default)]
struct AtomTable {
    by_key: HashMap<String, Var>,
    atoms: Vec<ProofExpr>,
}

impl AtomTable {
    fn intern(&mut self, atom: &ProofExpr) -> Option<Var> {
        let k = atom_key(atom)?;
        if let Some(&v) = self.by_key.get(&k) {
            return Some(v);
        }
        let v = self.atoms.len();
        self.atoms.push(atom.clone());
        self.by_key.insert(k, v);
        Some(v)
    }

    fn lit(&mut self, e: &ProofExpr) -> Option<Lit> {
        match e {
            ProofExpr::Not(inner) => Some(Lit { var: self.intern(inner)?, pos: false }),
            _ => Some(Lit { var: self.intern(e)?, pos: true }),
        }
    }
}

// ── Clauses & rules ──────────────────────────────────────────────────────────────

enum ClauseSrc {
    /// A bare disjunction premise; the exact `Or(...)` expression (for emit).
    Bare(ProofExpr),
    /// Materialized when the disjunctive-consequent rule `usize` fires.
    FromRule(usize),
    /// Added inside a decision branch as the assumed disjunct of an enclosing
    /// case-split (resolves to the bound conjunct hypothesis at emit time).
    Assumed(ProofExpr),
}

/// A disjunction. Its disjuncts may be literals (enabling unit propagation, `lits`
/// `Some`) or compound conjunctions (decision clauses, `lits` `None`). `or_expr`
/// preserves the exact `Or` nesting the emitter must follow.
struct Clause {
    or_expr: ProofExpr,
    disjuncts: Vec<ProofExpr>,
    lits: Option<Vec<Lit>>,
    src: ClauseSrc,
}

enum RuleCons {
    Lit(Lit),
    Disj(Vec<Lit>),
}

/// Three-valued status of a disjunct (or a whole conjunction) under the trail.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Tri {
    True,
    False,
    Live,
}

struct Rule {
    ante: Vec<Lit>,
    cons: RuleCons,
    /// The exact `Implies(...)` premise (cited by ModusPonens emit).
    src: ProofExpr,
}

/// How a variable came to be assigned — enough for the emitter to rebuild the
/// certified sub-proof.
#[derive(Clone)]
enum Reason {
    /// A unit premise; the exact premise expression.
    Premise(ProofExpr),
    /// ModusPonens: rule `usize` fired (all antecedents true) yielding this literal.
    Rule(usize),
    /// Exclusion (ModusTollens shape): rule `usize`'s consequent is violated and all
    /// antecedents but this one are true, so this antecedent is forced false.
    Exclude(usize),
    /// Unit propagation: this literal is the survivor of clause `usize`.
    Clause(usize),
    /// Disjunctive syllogism over a COMPOUND clause: clause `clause` has a single live
    /// disjunct (index `disjunct`); every other disjunct is refuted by a false conjunct.
    /// This literal is one of that surviving disjunct's conjuncts — proved by peeling the
    /// refuted disjuncts (`DisjunctionElim`) then projecting the conjunct (`ConjunctionElim`).
    /// This is what lets an of-pair clue contribute a LINEAR deduction instead of a search.
    CompoundClause { clause: usize, disjunct: usize },
    /// A search decision / assumption (justified by a bound hypothesis at emit time).
    Assumption,
}

// ── The solver ───────────────────────────────────────────────────────────────────

struct Solver {
    table: AtomTable,
    clauses: Vec<Clause>,
    rules: Vec<Rule>,
    assign: Vec<Option<bool>>,
    reason: Vec<Option<Reason>>,
    trail: Vec<Var>,
    /// Per-variable assignment order (index into `trail`). Lets the emitter refute a
    /// disjunct using the conjunct that became false EARLIEST, keeping the reconstructed
    /// proof's recursion strictly backward in trail order (acyclic).
    pos: Vec<usize>,
    fired_disj: Vec<bool>,
    /// Clauses currently mid-case-split (so the recursive search does not re-pick a
    /// clause whose chosen disjunct still has a pending sub-disjunction).
    deciding: std::collections::HashSet<usize>,
    /// Whether propagation is running at decision level 0 (the root fixpoint, never
    /// backtracked). Compound-clause unit propagation fires ONLY here: a root-forced
    /// literal is permanent, so its reason chain is monotone in trail order and the
    /// linear emit is acyclic. Under a backtrackable assumption (search branch), an
    /// of-pair clause's XOR-coupled disjuncts can force each other circularly, so there
    /// the clause is left to the case-split fallback instead.
    at_root: bool,
}

enum Conflict {
    /// Rule `usize`'s antecedents are all true but its consequent is violated.
    Rule(usize),
    /// Clause `usize` has every literal violated.
    Clause(usize),
}

/// Classify each ground premise into the solver's clause/rule/unit forms. Returns
/// `None` on anything it cannot classify (e.g. a surviving quantifier), so the caller
/// can fall back to the general engine.
fn compile(premises: &[ProofExpr]) -> Option<Solver> {
    let mut table = AtomTable::default();
    let mut clauses = Vec::new();
    let mut rules = Vec::new();
    let mut units: Vec<(Lit, ProofExpr)> = Vec::new();

    // Grounding conjoins each universal's instances into one top-level `And`; split
    // those back into independent facts (a conjunction asserts each conjunct).
    let mut flat: Vec<ProofExpr> = Vec::new();
    for p in premises {
        flatten_and(p, &mut flat);
    }

    // Classify each fact; a premise that does not fit the clause/rule/unit grammar is
    // SKIPPED, not fatal. Skipping is sound — `check_derivation` re-checks every emitted
    // tree against the FULL premise set, so a dropped (e.g. redundant grounded `∃`
    // existence) premise can only make the solver fail to prove, never prove falsely.
    for p in &flat {
        match p {
            ProofExpr::Or(..) => {
                let mut ds = Vec::new();
                flatten_or(p, &mut ds);
                if !ds.iter().all(is_clause_disjunct) {
                    continue;
                }
                intern_all(&mut table, p);
                let lits = if ds.iter().all(is_literal) {
                    ds.iter().map(|d| table.lit(d)).collect::<Option<Vec<Lit>>>()
                } else {
                    None
                };
                clauses.push(Clause {
                    or_expr: p.clone(),
                    disjuncts: ds,
                    lits,
                    src: ClauseSrc::Bare(p.clone()),
                });
            }
            ProofExpr::Implies(ante, cons) => {
                let mut as_ = Vec::new();
                flatten_and(ante, &mut as_);
                if !as_.iter().all(is_literal) {
                    continue;
                }
                let cons_form = if is_literal(cons) {
                    intern_all(&mut table, p);
                    RuleCons::Lit(table.lit(cons).unwrap())
                } else if matches!(cons.as_ref(), ProofExpr::Or(..)) {
                    let mut ds = Vec::new();
                    flatten_or(cons, &mut ds);
                    if !ds.iter().all(is_literal) {
                        continue;
                    }
                    intern_all(&mut table, p);
                    RuleCons::Disj(ds.iter().map(|d| table.lit(d).unwrap()).collect())
                } else {
                    continue;
                };
                let ante_lits = as_.iter().map(|a| table.lit(a).unwrap()).collect();
                rules.push(Rule { ante: ante_lits, cons: cons_form, src: p.clone() });
            }
            _ if is_literal(p) => {
                intern_all(&mut table, p);
                units.push((table.lit(p).unwrap(), p.clone()));
            }
            _ => continue,
        }
    }

    let n = table.atoms.len();
    let fired_disj = vec![false; rules.len()];
    let mut solver = Solver {
        table,
        clauses,
        rules,
        assign: vec![None; n],
        reason: vec![None; n],
        trail: Vec::new(),
        pos: vec![usize::MAX; n],
        fired_disj,
        deciding: std::collections::HashSet::new(),
        at_root: false,
    };
    for (l, expr) in units {
        let _ = solver.set(l, Reason::Premise(expr));
    }
    Some(solver)
}

impl Solver {
    /// `Some(true)` if `l` holds, `Some(false)` if `l` is violated, `None` if unknown.
    fn value(&self, l: Lit) -> Option<bool> {
        self.assign[l.var].map(|b| b == l.pos)
    }

    fn var_of_atom(&self, atom: &ProofExpr) -> Option<Var> {
        self.table.by_key.get(&atom_key(atom)?).copied()
    }

    fn lit_of(&self, e: &ProofExpr) -> Option<Lit> {
        match e {
            ProofExpr::Not(inner) => Some(Lit { var: self.var_of_atom(inner)?, pos: false }),
            _ => Some(Lit { var: self.var_of_atom(e)?, pos: true }),
        }
    }

    fn lit_expr(&self, l: Lit) -> ProofExpr {
        let atom = self.table.atoms[l.var].clone();
        if l.pos { atom } else { ProofExpr::Not(Box::new(atom)) }
    }

    fn set(&mut self, l: Lit, r: Reason) -> Result<(), ()> {
        match self.assign[l.var] {
            Some(b) => {
                if b == l.pos {
                    Ok(())
                } else {
                    Err(())
                }
            }
            None => {
                self.assign[l.var] = Some(l.pos);
                self.reason[l.var] = Some(r);
                self.pos[l.var] = self.trail.len();
                self.trail.push(l.var);
                Ok(())
            }
        }
    }

    /// Trail-based fixpoint propagation: forward ModusPonens, backward exclusion
    /// (ModusTollens), disjunctive-consequent materialization, and unit-clause
    /// propagation. Returns the conflict if one is reached.
    fn propagate(&mut self) -> Result<(), Conflict> {
        loop {
            let mut changed = false;

            for ri in 0..self.rules.len() {
                // Tally the antecedents.
                let mut n_nonsat = 0usize;
                let mut the_nonsat: Option<Lit> = None;
                let mut any_false = false;
                for i in 0..self.rules[ri].ante.len() {
                    let a = self.rules[ri].ante[i];
                    match self.value(a) {
                        Some(true) => {}
                        Some(false) => {
                            any_false = true;
                            n_nonsat += 1;
                            the_nonsat = Some(a);
                        }
                        None => {
                            n_nonsat += 1;
                            the_nonsat = Some(a);
                        }
                    }
                }

                // A false antecedent makes the rule vacuously true.
                if any_false {
                    continue;
                }

                if n_nonsat == 0 {
                    // All antecedents hold → fire forward.
                    match &self.rules[ri].cons {
                        RuleCons::Lit(l) => {
                            let l = *l;
                            match self.value(l) {
                                Some(true) => {}
                                Some(false) => return Err(Conflict::Rule(ri)),
                                None => {
                                    let _ = self.set(l, Reason::Rule(ri));
                                    changed = true;
                                }
                            }
                        }
                        RuleCons::Disj(lits) => {
                            if !self.fired_disj[ri] {
                                let lits = lits.clone();
                                self.fired_disj[ri] = true;
                                let (_, cons_form) = Self::split_implies(&self.rules[ri].src);
                                let mut ds = Vec::new();
                                flatten_or(&cons_form, &mut ds);
                                self.clauses.push(Clause {
                                    or_expr: cons_form,
                                    disjuncts: ds,
                                    lits: Some(lits),
                                    src: ClauseSrc::FromRule(ri),
                                });
                                changed = true;
                            }
                        }
                    }
                } else if n_nonsat == 1 {
                    // Exactly one antecedent open. If the consequent is a literal that is
                    // already violated, that antecedent must be false (ModusTollens).
                    if let RuleCons::Lit(l) = &self.rules[ri].cons {
                        if self.value(*l) == Some(false) {
                            let open = the_nonsat.unwrap();
                            if self.value(open).is_none() {
                                let _ = self.set(open.negate(), Reason::Exclude(ri));
                                changed = true;
                            }
                        }
                    }
                }
            }

            for ci in 0..self.clauses.len() {
                let (has_true, live) = self.clause_status(ci);
                if has_true {
                    continue;
                }
                if live.is_empty() {
                    return Err(Conflict::Clause(ci));
                }
                // Unit propagation on a literal clause: the lone live literal is forced.
                if self.clauses[ci].lits.is_some() && live.len() == 1 {
                    if let Some(l) = self.lit_of(&self.clauses[ci].disjuncts[live[0]]) {
                        if self.set(l, Reason::Clause(ci)).is_ok() {
                            changed = true;
                        }
                    }
                }
                // Compound (of-pair / either-or) clause collapsed to a SINGLE live
                // disjunct: disjunctive syllogism forces that disjunct, so assert each of
                // its literal conjuncts. This is the deduction that lets the whole grid
                // be forced by propagation (no search) → linear proofs. Gated to the root
                // fixpoint: under a backtrackable assumption the survivor's conjuncts can
                // be the only thing refuting a sibling (shared anchor), making the reason
                // chain cyclic — those clauses are left to the case-split fallback.
                if self.at_root && self.clauses[ci].lits.is_none() && live.len() == 1 {
                    let di = live[0];
                    let mut conjs = Vec::new();
                    flatten_and(&self.clauses[ci].disjuncts[di].clone(), &mut conjs);
                    for conj in &conjs {
                        // Inner disjunction conjuncts (a nested either-or) are left to the
                        // search; only the literal conjuncts are forced here.
                        if !is_literal(conj) {
                            continue;
                        }
                        let Some(l) = self.lit_of(conj) else { continue };
                        match self.value(l) {
                            Some(true) => {}
                            Some(false) => return Err(Conflict::Clause(ci)),
                            None => {
                                let _ = self.set(l, Reason::CompoundClause { clause: ci, disjunct: di });
                                changed = true;
                            }
                        }
                    }
                }
            }

            if !changed {
                break;
            }
        }
        Ok(())
    }

    // ── Emit: turn the trail into a certified DerivationTree ─────────────────────

    /// Prove the literal assigned to variable `v` (in its assigned polarity).
    fn prove_var(&self, v: Var) -> DerivationTree {
        let val = self.assign[v].expect("prove_var on unassigned variable");
        match self.reason[v].as_ref().expect("assigned variable lacks a reason") {
            Reason::Premise(expr) => leaf(expr.clone()),
            Reason::Assumption => leaf(self.lit_expr(Lit { var: v, pos: val })),
            Reason::Rule(ri) => self.emit_rule_forward(*ri),
            Reason::Exclude(ri) => self.emit_rule_exclude(*ri, v),
            Reason::Clause(ci) => self.emit_clause_survivor(*ci, Lit { var: v, pos: val }),
            Reason::CompoundClause { clause, disjunct } => {
                self.emit_compound_clause(*clause, *disjunct, Lit { var: v, pos: val })
            }
        }
    }

    /// Emit a literal forced as a conjunct of a compound clause's single surviving
    /// disjunct: peel the refuted disjuncts (disjunctive syllogism) to prove the
    /// surviving conjunction, then ∧-eliminate down to the wanted literal. The
    /// disjunction's `Or`-type appears ONCE here, not multiplied by a search tree.
    fn emit_compound_clause(&self, ci: usize, di: usize, want: Lit) -> DerivationTree {
        let or_expr = self.clauses[ci].or_expr.clone();
        let or_tree = self.materialize_clause(ci);
        let survivor = self.clauses[ci].disjuncts[di].clone();
        let before = self.pos[want.var];
        let surv_tree = self.extract_survivor(&or_expr, or_tree, &survivor, before);
        let proj = self.project_conjunct(&survivor, surv_tree, want);
        align_to(proj, &self.lit_expr(want))
    }

    /// From a proof of a (possibly nested) conjunction, ∧-eliminate down to the conjunct
    /// whose literal is `want`.
    fn project_conjunct(&self, conj: &ProofExpr, conj_tree: DerivationTree, want: Lit) -> DerivationTree {
        match conj {
            ProofExpr::And(l, r) => {
                if self.conjunct_has(l, want) {
                    let lt = conj_elim((**l).clone(), conj_tree);
                    self.project_conjunct(l, lt, want)
                } else {
                    let rt = conj_elim((**r).clone(), conj_tree);
                    self.project_conjunct(r, rt, want)
                }
            }
            _ => conj_tree,
        }
    }

    /// Does the conjunction `e` contain a literal conjunct equal to `want`?
    fn conjunct_has(&self, e: &ProofExpr, want: Lit) -> bool {
        match e {
            ProofExpr::And(l, r) => self.conjunct_has(l, want) || self.conjunct_has(r, want),
            _ => self.lit_of(e) == Some(want),
        }
    }

    /// Prove a literal expression `want` exactly (aligning Identity orientation).
    fn prove_lit_expr(&self, want: &ProofExpr) -> DerivationTree {
        let lit = self.lit_of(want).expect("prove_lit_expr on an un-interned atom");
        let base = self.prove_var(lit.var);
        align_to(base, want)
    }

    fn split_implies(src: &ProofExpr) -> (ProofExpr, ProofExpr) {
        match src {
            ProofExpr::Implies(a, c) => ((**a).clone(), (**c).clone()),
            _ => unreachable!("rule src is always an implication"),
        }
    }

    /// Prove a conjunction/literal antecedent form. `forced` (if any) is proved by a
    /// bound hypothesis leaf instead of recursing (it is the reductio assumption).
    fn prove_ante(&self, form: &ProofExpr, forced: Option<Var>) -> DerivationTree {
        match form {
            ProofExpr::And(l, r) => conj_intro(
                (**l).clone(),
                (**r).clone(),
                self.prove_ante(l, forced),
                self.prove_ante(r, forced),
            ),
            _ => {
                if let (Some(fv), Some(lit)) = (forced, self.lit_of(form)) {
                    if lit.var == fv {
                        return leaf(form.clone());
                    }
                }
                self.prove_lit_expr(form)
            }
        }
    }

    fn emit_rule_forward(&self, ri: usize) -> DerivationTree {
        let src = self.rules[ri].src.clone();
        let (ante_form, cons_form) = Self::split_implies(&src);
        let ante_proof = self.prove_ante(&ante_form, None);
        modus_ponens(cons_form, leaf(src), ante_proof)
    }

    fn emit_rule_exclude(&self, ri: usize, forced_v: Var) -> DerivationTree {
        let src = self.rules[ri].src.clone();
        let (ante_form, cons_form) = Self::split_implies(&src);
        let assumed = self.table.atoms[forced_v].clone();
        let ante_proof = self.prove_ante(&ante_form, Some(forced_v));
        let cons_tree = modus_ponens(cons_form.clone(), leaf(src), ante_proof);
        let cons_lit = self.lit_of(&cons_form).expect("exclusion consequent must be a literal");
        let neg_cons = self.prove_var(cons_lit.var);
        // The consequent may be negative (a functionality rule `Li → ¬Lj`), so the
        // contradiction's polarity order is not fixed — let `make_contradiction` sort it.
        let bot = make_contradiction(cons_tree, neg_cons);
        reductio(assumed, bot)
    }

    fn materialize_clause(&self, ci: usize) -> DerivationTree {
        match &self.clauses[ci].src {
            ClauseSrc::Bare(or_expr) | ClauseSrc::Assumed(or_expr) => leaf(or_expr.clone()),
            ClauseSrc::FromRule(ri) => {
                let src = self.rules[*ri].src.clone();
                let (ante_form, cons_form) = Self::split_implies(&src);
                let ante_proof = self.prove_ante(&ante_form, None);
                modus_ponens(cons_form, leaf(src), ante_proof)
            }
        }
    }

    fn emit_clause_survivor(&self, ci: usize, survivor: Lit) -> DerivationTree {
        let or_tree = self.materialize_clause(ci);
        let or_expr = or_tree.conclusion.clone();
        let survivor_expr = self.lit_expr(survivor);
        let before = self.pos[survivor.var];
        self.extract_survivor(&or_expr, or_tree, &survivor_expr, before)
    }

    /// Extract the surviving disjunct `target` from a (left-nested) disjunction whose
    /// every other disjunct is violated, by iterated disjunctive syllogism. `before` caps
    /// the trail position of the facts used to refute the other disjuncts (acyclicity).
    fn extract_survivor(&self, or_expr: &ProofExpr, or_tree: DerivationTree, target: &ProofExpr, before: usize) -> DerivationTree {
        match or_expr {
            ProofExpr::Or(l, r) => {
                if r.as_ref() == target {
                    let lneg = self.prove_neg_disjunct(l, before);
                    disjunction_elim(target.clone(), or_tree, lneg)
                } else {
                    let rneg = self.prove_neg_disjunct(r, before);
                    let l_tree = disjunction_elim((**l).clone(), or_tree, rneg);
                    self.extract_survivor(l, l_tree, target, before)
                }
            }
            _ => or_tree,
        }
    }

    /// Prove `¬e` where `e` is a violated disjunct: a single literal (directly) or a
    /// sub-disjunction (by reductio whose branches close to ⊥). `before` caps the
    /// refuting facts' trail positions.
    fn prove_neg_disjunct(&self, e: &ProofExpr, before: usize) -> DerivationTree {
        if is_literal(e) {
            let lit = self.lit_of(e).expect("disjunct atom must be interned");
            let proof = self.prove_var(lit.var);
            align_to(proof, &neg_of(e))
        } else {
            let bot = self.falsum_from_disj(e, leaf(e.clone()), before);
            reductio(e.clone(), bot)
        }
    }

    /// Given `e_tree : e` (a disjunction, conjunction, or literal, all violated), derive ⊥,
    /// refuting using only facts assigned strictly before `before` where possible.
    fn falsum_from_disj(&self, e: &ProofExpr, e_tree: DerivationTree, before: usize) -> DerivationTree {
        match e {
            ProofExpr::Or(l, r) => {
                let lb = self.falsum_from_disj(l, leaf((**l).clone()), before);
                let rb = self.falsum_from_disj(r, leaf((**r).clone()), before);
                disjunction_cases(falsum(), e_tree, lb, rb)
            }
            ProofExpr::And(l, r) => {
                // A false conjunction disjunct: project the conjunct that became false
                // EARLIEST — preferring one false strictly BEFORE the survivor was forced
                // (`before`), else the globally earliest. Picking by trail order (not
                // structural order, and capped at `before`) keeps the emit's recursion
                // strictly backward, so the reconstructed proof is acyclic even when a
                // later exclusion would otherwise refer back through the forcing clause.
                let key = |x: &ProofExpr| {
                    self.false_at(x, before)
                        .map(|p| (0u8, p))
                        .or_else(|| self.false_at(x, usize::MAX).map(|p| (1u8, p)))
                };
                let pick_left = match (key(l), key(r)) {
                    (Some(a), Some(b)) => a <= b,
                    (Some(_), None) => true,
                    (None, Some(_)) => false,
                    (None, None) => true,
                };
                let (conjunct, proj) = if pick_left {
                    ((**l).clone(), conj_elim((**l).clone(), e_tree))
                } else {
                    ((**r).clone(), conj_elim((**r).clone(), e_tree))
                };
                self.falsum_from_disj(&conjunct, proj, before)
            }
            _ => {
                let lit = self.lit_of(e).expect("disjunct atom must be interned");
                let neg = self.prove_var(lit.var);
                contra_align(e_tree, neg)
            }
        }
    }

    // ── Decision search (DPLL) ───────────────────────────────────────────────────

    /// The trail position by which `e` is FALSE using only assignments strictly BEFORE
    /// `before` (or `None`). A conjunction is false as soon as ANY conjunct is (earliest),
    /// a disjunction only once ALL disjuncts are (latest), a literal at its own position.
    /// The `before` cap keeps the reconstructed proof's recursion strictly backward.
    fn false_at(&self, e: &ProofExpr, before: usize) -> Option<usize> {
        match e {
            ProofExpr::And(l, r) => match (self.false_at(l, before), self.false_at(r, before)) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            },
            ProofExpr::Or(l, r) => match (self.false_at(l, before), self.false_at(r, before)) {
                (Some(a), Some(b)) => Some(a.max(b)),
                _ => None,
            },
            _ => match self.lit_of(e) {
                Some(l) if self.value(l) == Some(false) && self.pos[l.var] < before => Some(self.pos[l.var]),
                _ => None,
            },
        }
    }

    /// Three-valued evaluation of a disjunct (or any conjunction/disjunction/literal)
    /// under the current trail.
    fn eval_disjunct(&self, e: &ProofExpr) -> Tri {
        match e {
            ProofExpr::And(l, r) => match (self.eval_disjunct(l), self.eval_disjunct(r)) {
                (Tri::False, _) | (_, Tri::False) => Tri::False,
                (Tri::True, Tri::True) => Tri::True,
                _ => Tri::Live,
            },
            ProofExpr::Or(l, r) => match (self.eval_disjunct(l), self.eval_disjunct(r)) {
                (Tri::True, _) | (_, Tri::True) => Tri::True,
                (Tri::False, Tri::False) => Tri::False,
                _ => Tri::Live,
            },
            _ => match self.lit_of(e).and_then(|l| self.value(l)) {
                Some(true) => Tri::True,
                Some(false) => Tri::False,
                None => Tri::Live,
            },
        }
    }

    /// `(has_true, indices of live disjuncts)` for clause `ci`.
    fn clause_status(&self, ci: usize) -> (bool, Vec<usize>) {
        let mut has_true = false;
        let mut live = Vec::new();
        for (i, d) in self.clauses[ci].disjuncts.iter().enumerate() {
            match self.eval_disjunct(d) {
                Tri::True => has_true = true,
                Tri::Live => live.push(i),
                Tri::False => {}
            }
        }
        (has_true, live)
    }

    /// The first unsatisfied clause with a live disjunct that the search must split.
    /// (After propagation, literal clauses with a single live disjunct are already
    /// unit-propagated, so only genuine decisions remain.)
    fn pick_split(&self) -> Option<usize> {
        // Split the MOST-CONSTRAINED open clause — the one with the fewest live disjuncts
        // (minimum-remaining-values). A 2-live of-pair branches two ways and each branch
        // re-propagates to a near-forced state; a 12-live one fans out twelve deep. Picking
        // the smallest branching first keeps the case-analysis — and therefore the certified
        // proof — shallow, the search analogue of the min-live closure selection.
        (0..self.clauses.len())
            .filter(|&ci| {
                if self.deciding.contains(&ci) {
                    return false;
                }
                let (has_true, live) = self.clause_status(ci);
                if has_true || live.is_empty() {
                    return false;
                }
                // A literal clause with a single live disjunct is already unit-propagated.
                !(self.clauses[ci].lits.is_some() && live.len() < 2)
            })
            .min_by_key(|&ci| self.clause_status(ci).1.len())
    }

    /// Derive ⊥ from the current assumptions: propagate, and on a stall, case-split an
    /// unresolved clause and refute every branch. `None` if a branch is satisfiable.
    fn derive_falsum(&mut self, depth: usize) -> Option<DerivationTree> {
        if depth > MAX_DEPTH {
            return None;
        }
        if let Err(c) = self.propagate() {
            return Some(self.emit_conflict(c));
        }
        let ci = self.pick_split()?;
        let or_expr = self.clauses[ci].or_expr.clone();
        let or_tree = self.materialize_clause(ci);
        self.deciding.insert(ci);
        let result = self.split_to_falsum(&or_expr, or_tree, depth);
        self.deciding.remove(&ci);
        result
    }

    fn split_to_falsum(&mut self, or_expr: &ProofExpr, or_tree: DerivationTree, depth: usize) -> Option<DerivationTree> {
        match or_expr {
            ProofExpr::Or(l, r) => {
                let lb = self.branch(l, depth)?;
                let rb = self.branch(r, depth)?;
                Some(disjunction_cases(falsum(), or_tree, lb, rb))
            }
            single => self.branch(single, depth),
        }
    }

    /// Assume `disjunct` (bound as a hypothesis by the enclosing case-split), refute it
    /// to ⊥, then undo the assumption.
    fn branch(&mut self, disjunct: &ProofExpr, depth: usize) -> Option<DerivationTree> {
        let trail_snap = self.trail.len();
        let clause_snap = self.clauses.len();
        let fired_snap = self.fired_disj.clone();
        let result = match self.assume_disjunct(disjunct) {
            Some(info) => Some(self.emit_conflict_info(info)),
            None => self.derive_falsum(depth + 1),
        };
        self.backtrack(trail_snap, clause_snap, fired_snap);
        result
    }

    /// Assert a disjunct: set its literal conjuncts as assumptions and add any
    /// disjunction conjuncts as branch-local clauses. Returns the first conflict, if any.
    fn assume_disjunct(&mut self, d: &ProofExpr) -> Option<ConflictInfo> {
        match d {
            ProofExpr::And(l, r) => self.assume_disjunct(l).or_else(|| self.assume_disjunct(r)),
            ProofExpr::Or(..) => {
                let mut ds = Vec::new();
                flatten_or(d, &mut ds);
                let lits = if ds.iter().all(is_literal) {
                    ds.iter().map(|x| self.lit_of(x)).collect::<Option<Vec<Lit>>>()
                } else {
                    None
                };
                self.clauses.push(Clause {
                    or_expr: d.clone(),
                    disjuncts: ds,
                    lits,
                    src: ClauseSrc::Assumed(d.clone()),
                });
                None
            }
            _ => {
                let lit = self.lit_of(d).expect("assumed atom must be interned");
                if self.set(lit, Reason::Assumption).is_err() {
                    Some(ConflictInfo::Clash { assumed: d.clone() })
                } else {
                    None
                }
            }
        }
    }

    fn backtrack(&mut self, trail_snap: usize, clause_snap: usize, fired_snap: Vec<bool>) {
        while self.trail.len() > trail_snap {
            let v = self.trail.pop().unwrap();
            self.assign[v] = None;
            self.reason[v] = None;
        }
        self.clauses.truncate(clause_snap);
        self.fired_disj = fired_snap;
    }

    fn emit_conflict(&self, c: Conflict) -> DerivationTree {
        match c {
            Conflict::Rule(ri) => {
                let src = self.rules[ri].src.clone();
                let (ante_form, cons_form) = Self::split_implies(&src);
                let ante_proof = self.prove_ante(&ante_form, None);
                let cons_tree = modus_ponens(cons_form.clone(), leaf(src), ante_proof);
                let cons_lit = self.lit_of(&cons_form).expect("rule consequent is a literal");
                let opp = self.prove_var(cons_lit.var);
                make_contradiction(cons_tree, opp)
            }
            Conflict::Clause(ci) => {
                let or_expr = self.clauses[ci].or_expr.clone();
                let or_tree = self.materialize_clause(ci);
                self.falsum_from_disj(&or_expr, or_tree, usize::MAX)
            }
        }
    }

    fn emit_conflict_info(&self, info: ConflictInfo) -> DerivationTree {
        match info {
            ConflictInfo::Clash { assumed } => {
                let lit = self.lit_of(&assumed).expect("clashing atom must be interned");
                let existing = self.prove_var(lit.var);
                make_contradiction(leaf(assumed), existing)
            }
            ConflictInfo::Rule(ri) => self.emit_conflict(Conflict::Rule(ri)),
            ConflictInfo::Clause(ci) => self.emit_conflict(Conflict::Clause(ci)),
        }
    }

    // ── Positive-cell goal by closure elimination + search ───────────────────────

    /// Prove a positive cell `goal` that propagation alone does not force: find the
    /// closure clause it sits in (the row's value disjunction), refute every OTHER
    /// value by DPLL search, then the goal survives by disjunctive syllogism.
    fn prove_positive_by_closure(&mut self, goal: &ProofExpr) -> Option<DerivationTree> {
        let goal_lit = self.lit_of(goal)?;
        // The cell can sit in more than one closure clause — its row (the values it may
        // take) AND, once column closures are added, its column (the rows that may take
        // its value). Both prove it by refuting their OTHER disjuncts. Try them
        // FEWEST-LIVE first (the rest already refuted on the trail ⇒ the search opens the
        // fewest branches and the proof stays small), but FALL THROUGH to the next on
        // failure: trying only one closure would lose completeness when that one's
        // refutations exceed the depth bound while another's would have closed.
        let mut candidates: Vec<usize> = (0..self.clauses.len())
            .filter(|&ci| {
                self.clauses[ci]
                    .lits
                    .as_ref()
                    .map_or(false, |ls| ls.contains(&goal_lit) && ls.len() >= 2)
            })
            .collect();
        candidates.sort_by_key(|&ci| self.clause_status(ci).1.len());
        for ci in candidates {
            let or_expr = self.clauses[ci].or_expr.clone();
            let or_tree = self.materialize_clause(ci);
            if let Some(tree) = self.extract_by_search(&or_expr, or_tree, goal) {
                return Some(tree);
            }
        }
        None
    }

    /// Like `extract_survivor`, but the non-surviving disjuncts are refuted by SEARCH
    /// (assume + derive ⊥ + reductio) rather than read from the trail.
    fn extract_by_search(&mut self, or_expr: &ProofExpr, or_tree: DerivationTree, target: &ProofExpr) -> Option<DerivationTree> {
        debug_assert!(&or_tree.conclusion == or_expr, "extract_by_search mismatch:\n tree={:?}\n expr={:?}", or_tree.conclusion, or_expr);
        match or_expr {
            ProofExpr::Or(l, r) => {
                if r.as_ref() == target {
                    let lneg = self.prove_neg_by_search(l)?;
                    Some(disjunction_elim(target.clone(), or_tree, lneg))
                } else {
                    let rneg = self.prove_neg_by_search(r)?;
                    let l_tree = disjunction_elim((**l).clone(), or_tree, rneg);
                    self.extract_by_search(l, l_tree, target)
                }
            }
            _ => Some(or_tree),
        }
    }

    /// Prove `¬e` where `e` is a single cell literal OR a sub-disjunction. A literal:
    /// directly if already false on the trail, else by assuming it and refuting it (DPLL),
    /// discharged by reductio. A disjunction `A ∨ B` (e.g. the whole `CT ∨ FL ∨ KY` to the
    /// LEFT of `Maine` when the goal cell is the closure's RIGHTMOST disjunct): assume it
    /// and case-split each disjunct to ⊥. Without the disjunction case, a cell that is the
    /// last value of its closure could never be proved positively (its left subtree is not
    /// a single literal).
    fn prove_neg_by_search(&mut self, e: &ProofExpr) -> Option<DerivationTree> {
        if let ProofExpr::Or(..) = e {
            let bot = self.disj_to_falsum(e)?;
            return Some(reductio(e.clone(), bot));
        }
        let lit = self.lit_of(e)?;
        match self.value(lit) {
            Some(false) => Some(align_to(self.prove_var(lit.var), &neg_of(e))),
            Some(true) => None,
            None => {
                let trail_snap = self.trail.len();
                let clause_snap = self.clauses.len();
                let fired_snap = self.fired_disj.clone();
                let _ = self.set(lit, Reason::Assumption);
                let bot = self.derive_falsum(0);
                let result = bot.map(|b| reductio(e.clone(), b));
                self.backtrack(trail_snap, clause_snap, fired_snap);
                result
            }
        }
    }

    /// Derive ⊥ from `e` ASSUMED true (bound as a hypothesis by an enclosing case-split):
    /// a disjunction recurses (case-split each arm); a literal contradicts its own searched
    /// refutation. Mirrors `falsum_from_disj`, but the disjuncts are refuted by SEARCH
    /// rather than read off the trail.
    fn disj_to_falsum(&mut self, e: &ProofExpr) -> Option<DerivationTree> {
        if let ProofExpr::Or(l, r) = e {
            let lb = self.disj_to_falsum(l)?;
            let rb = self.disj_to_falsum(r)?;
            return Some(disjunction_cases(falsum(), leaf(e.clone()), lb, rb));
        }
        // `e` is a literal assumed true here; its searched negation closes the branch.
        let neg = self.prove_neg_by_search(e)?;
        Some(make_contradiction(leaf(e.clone()), neg))
    }
}

const MAX_DEPTH: usize = 64;

enum ConflictInfo {
    /// An assumed literal clashes with the existing assignment of its variable.
    Clash { assumed: ProofExpr },
    Rule(usize),
    Clause(usize),
}

/// `Contradiction` from two proofs of opposite polarity, in either order.
fn make_contradiction(a: DerivationTree, b: DerivationTree) -> DerivationTree {
    if matches!(a.conclusion, ProofExpr::Not(_)) {
        contra_align(b, a)
    } else {
        contra_align(a, b)
    }
}

/// `¬e` (peeling a double negation).
fn neg_of(e: &ProofExpr) -> ProofExpr {
    match e {
        ProofExpr::Not(inner) => (**inner).clone(),
        _ => ProofExpr::Not(Box::new(e.clone())),
    }
}

/// Re-orient `tree` (a literal proof) to conclude exactly `want`, inserting an equality
/// symmetry (or a symmetry-reductio under a negation) when only the Identity order
/// differs.
fn align_to(tree: DerivationTree, want: &ProofExpr) -> DerivationTree {
    if &tree.conclusion == want {
        return tree;
    }
    match (&tree.conclusion, want) {
        (ProofExpr::Identity(a, b), ProofExpr::Identity(c, d)) if a == d && b == c => {
            eq_sym(a.clone(), b.clone(), tree)
        }
        (ProofExpr::Not(ti), ProofExpr::Not(wi)) => match (ti.as_ref(), wi.as_ref()) {
            (ProofExpr::Identity(a, b), ProofExpr::Identity(c, d)) if a == d && b == c => {
                let assumed = ProofExpr::Identity(c.clone(), d.clone());
                let flipped = eq_sym(c.clone(), d.clone(), leaf(assumed.clone()));
                let bot = contradiction(flipped, tree);
                reductio(assumed, bot)
            }
            _ => tree,
        },
        _ => tree,
    }
}

/// Build `Contradiction` from a positive proof and a negation proof, aligning the
/// positive side's Identity orientation to the negation's inner term.
fn contra_align(pos_tree: DerivationTree, neg_tree: DerivationTree) -> DerivationTree {
    let inner = match &neg_tree.conclusion {
        ProofExpr::Not(i) => (**i).clone(),
        other => other.clone(),
    };
    let aligned = align_to(pos_tree, &inner);
    contradiction(aligned, neg_tree)
}

/// Solve a single grid cell by propagation and emit its certified derivation. The
/// premises must be ground (call `grounding::ground_problem` first). Returns `None`
/// when the cell is not forced by propagation alone (it needs search — Phase C) or the
/// premises do not classify as a grid.
pub fn grid_prove(premises: &[ProofExpr], goal: &ProofExpr) -> Option<DerivationTree> {
    let mut solver = compile(premises)?;
    // The root fixpoint runs at decision level 0: its assignments are permanent, so
    // compound-clause unit propagation is sound to emit linearly here. Every later
    // `propagate()` runs under a backtrackable assumption with `at_root` false.
    //
    solver.at_root = true;
    let root = solver.propagate();
    solver.at_root = false;
    if root.is_err() {
        return None;
    }
    // A goal already forced by propagation (the Phase-B positive-cell path).
    if let Some(lit) = solver.lit_of(goal) {
        if solver.value(lit) == Some(true) {
            return Some(align_to(solver.prove_var(lit.var), goal));
        }
    }
    // A positive cell not forced by propagation: prove it by closure elimination,
    // refuting the row's other values via DPLL search.
    if !matches!(goal, ProofExpr::Not(_)) && solver.lit_of(goal).is_some() {
        if let Some(tree) = solver.prove_positive_by_closure(goal) {
            return Some(align_to(tree, goal));
        }
    }
    // A negative goal: assume the atom and refute it by search (DPLL), then reductio.
    if let ProofExpr::Not(inner) = goal {
        let x_lit = solver.lit_of(inner)?;
        match solver.value(x_lit) {
            Some(false) => return Some(align_to(solver.prove_var(x_lit.var), goal)),
            Some(true) => return None,
            None => {
                let _ = solver.set(x_lit, Reason::Assumption);
                let bot = solver.derive_falsum(0)?;
                return Some(reductio((**inner).clone(), bot));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ProofExpr, ProofTerm};

    fn c(s: &str) -> ProofTerm {
        ProofTerm::Constant(s.to_string())
    }
    fn v(s: &str) -> ProofTerm {
        ProofTerm::Variable(s.to_string())
    }
    fn pred(name: &str, args: Vec<ProofTerm>) -> ProofExpr {
        ProofExpr::Predicate { name: name.to_string(), args, world: None }
    }
    fn imp(a: ProofExpr, b: ProofExpr) -> ProofExpr {
        ProofExpr::Implies(Box::new(a), Box::new(b))
    }
    fn or(a: ProofExpr, b: ProofExpr) -> ProofExpr {
        ProofExpr::Or(Box::new(a), Box::new(b))
    }
    fn and(a: ProofExpr, b: ProofExpr) -> ProofExpr {
        ProofExpr::And(Box::new(a), Box::new(b))
    }
    fn not(e: ProofExpr) -> ProofExpr {
        ProofExpr::Not(Box::new(e))
    }
    fn id(a: ProofTerm, b: ProofTerm) -> ProofExpr {
        ProofExpr::Identity(a, b)
    }
    fn forall(var: &str, body: ProofExpr) -> ProofExpr {
        ProofExpr::ForAll { variable: var.to_string(), body: Box::new(body) }
    }
    fn exists(var: &str, body: ProofExpr) -> ProofExpr {
        ProofExpr::Exists { variable: var.to_string(), body: Box::new(body) }
    }

    fn verify(premises: &[ProofExpr], goal: &ProofExpr, tree: DerivationTree) -> crate::verify::VerifiedProof {
        crate::verify::check_derivation(premises, goal, tree)
    }

    // ── PHASE A: prove the emitter's tree SHAPES certify, before any search ──────

    /// A propagation step (ModusPonens from a rule's reason) certifies.
    #[test]
    fn modus_ponens_shape_certifies() {
        let p = pred("P", vec![c("Obj")]);
        let q = pred("Q", vec![c("Obj")]);
        let premises = vec![p.clone(), imp(p.clone(), q.clone())];
        let tree = modus_ponens(q.clone(), leaf(imp(p.clone(), q.clone())), leaf(p));
        let r = verify(&premises, &q, tree);
        assert!(r.verified, "ModusPonens shape must certify; err: {:?}", r.verification_error);
    }

    /// A unit-clause propagation (disjunctive syllogism from the clause's reason) certifies.
    #[test]
    fn disjunction_elim_shape_certifies() {
        let p = pred("P", vec![c("Obj")]);
        let q = pred("Q", vec![c("Obj")]);
        let premises = vec![or(p.clone(), q.clone()), not(p.clone())];
        let tree = disjunction_elim(q.clone(), leaf(or(p.clone(), q.clone())), leaf(not(p)));
        let r = verify(&premises, &q, tree);
        assert!(r.verified, "DisjunctionElim shape must certify; err: {:?}", r.verification_error);
    }

    /// THE real grid shape, hand-built end to end: the 2-value bijection cell
    /// `In(Beta, Maine)`, proved by closure (ModusPonens) + a reductio that fires the
    /// at-most-one rule and closes the equality contradiction. Every node is a rule
    /// the solver's emitter will produce — if this certifies, the emitter is sound.
    #[test]
    fn two_value_grid_cell_handbuilt_certifies() {
        let trip = |t: ProofTerm| pred("Trip", vec![t]);
        let in_ = |t: ProofTerm, s: ProofTerm| pred("In", vec![t, s]);
        let fl = || c("Florida");
        let me_ = || c("Maine");

        // Grounded premises (what `ground_problem` yields, split into the instances
        // the certifier registers as hypotheses):
        let closure_beta = imp(trip(c("Beta")), or(in_(c("Beta"), fl()), in_(c("Beta"), me_())));
        // at-most-one instance for the pair (Beta, Alpha):
        //   ((Trip(Beta)∧In(Beta,FL)) ∧ (Trip(Alpha)∧In(Alpha,FL))) → Beta = Alpha
        let amo_beta_alpha = imp(
            and(and(trip(c("Beta")), in_(c("Beta"), fl())), and(trip(c("Alpha")), in_(c("Alpha"), fl()))),
            id(c("Beta"), c("Alpha")),
        );
        let premises = vec![
            trip(c("Alpha")),
            trip(c("Beta")),
            not(id(c("Alpha"), c("Beta"))),
            closure_beta.clone(),
            amo_beta_alpha.clone(),
            in_(c("Alpha"), fl()),
        ];
        let goal = in_(c("Beta"), me_());

        // ¬In(Beta,FL): assume In(Beta,FL); fire the at-most-one rule to get
        // Beta=Alpha; symmetrise to Alpha=Beta; contradict ¬(Alpha=Beta).
        let ante = conj_intro(
            and(trip(c("Beta")), in_(c("Beta"), fl())),
            and(trip(c("Alpha")), in_(c("Alpha"), fl())),
            conj_intro(trip(c("Beta")), in_(c("Beta"), fl()), leaf(trip(c("Beta"))), leaf(in_(c("Beta"), fl()))),
            conj_intro(trip(c("Alpha")), in_(c("Alpha"), fl()), leaf(trip(c("Alpha"))), leaf(in_(c("Alpha"), fl()))),
        );
        let beta_eq_alpha = modus_ponens(id(c("Beta"), c("Alpha")), leaf(amo_beta_alpha), ante);
        let alpha_eq_beta = eq_sym(c("Beta"), c("Alpha"), beta_eq_alpha);
        let contra = contradiction(alpha_eq_beta, leaf(not(id(c("Alpha"), c("Beta")))));
        let neg_in_beta_fl = reductio(in_(c("Beta"), fl()), contra);

        // closure: Trip(Beta) → In(Beta,FL)∨In(Beta,ME); MP with Trip(Beta).
        let beta_disj = modus_ponens(
            or(in_(c("Beta"), fl()), in_(c("Beta"), me_())),
            leaf(closure_beta),
            leaf(trip(c("Beta"))),
        );
        // Disjunctive syllogism: (In(Beta,FL)∨In(Beta,ME)), ¬In(Beta,FL) ⊢ In(Beta,ME).
        let tree = disjunction_elim(in_(c("Beta"), me_()), beta_disj, neg_in_beta_fl);

        let r = verify(&premises, &goal, tree);
        assert!(r.verified, "hand-built 2-value grid cell must certify; err: {:?}", r.verification_error);
    }

    // ── PHASE B: the solver PROPAGATES and EMITS the certified tree itself ───────

    /// The 2-value bijection, solved end to end by the incremental solver: ground the
    /// universal premises, `grid_prove` propagates `¬In(Beta,FL)` (exclusion) then the
    /// closure forces `In(Beta,Maine)`, and the emitted derivation kernel-certifies.
    #[test]
    fn two_value_grid_propagates_and_certifies() {
        let trip = |t: ProofTerm| pred("Trip", vec![t]);
        let in_ = |t: ProofTerm, s: ProofTerm| pred("In", vec![t, s]);
        let fl = || c("Florida");
        let me_ = || c("Maine");
        let closure = forall("x", imp(trip(v("x")), or(in_(v("x"), fl()), in_(v("x"), me_()))));
        let exactly_one_fl = forall(
            "x",
            forall(
                "y",
                imp(
                    and(and(trip(v("x")), in_(v("x"), fl())), and(trip(v("y")), in_(v("y"), fl()))),
                    id(v("x"), v("y")),
                ),
            ),
        );
        let premises = vec![
            trip(c("Alpha")),
            trip(c("Beta")),
            not(id(c("Alpha"), c("Beta"))),
            closure,
            exactly_one_fl,
            in_(c("Alpha"), fl()),
        ];
        let goal = in_(c("Beta"), me_());
        let (gp, gg) = crate::grounding::ground_problem(&premises, &goal);
        let tree = grid_prove(&gp, &gg).expect("2-value cell must be forced by propagation");
        let r = verify(&gp, &gg, tree);
        assert!(r.verified, "solver-emitted 2-value proof must certify; err: {:?}", r.verification_error);
    }

    /// A full-size multi-category grid (4 trips × 3 four-value categories), solved by
    /// pure propagation: three states pinned, the fourth (`Delta → Maine`) forced by
    /// exclusion over the state closure + at-most-one, with the irrelevant year/friend
    /// categories present as noise. The solver emits a certified derivation.
    #[test]
    fn multi_category_grid_propagates_and_certifies() {
        let trips = ["Alpha", "Beta", "Gamma", "Delta"];
        let neq = |a: &str, b: &str| not(id(c(a), c(b)));
        let mut premises: Vec<ProofExpr> = Vec::new();
        let category = |rel: &str, vals: &[&str], out: &mut Vec<ProofExpr>| {
            for t in trips {
                let mut it = vals.iter().map(|val| pred(rel, vec![c(t), c(val)]));
                let first = it.next().unwrap();
                out.push(it.fold(first, or));
            }
            for val in vals {
                for (i, t) in trips.iter().enumerate() {
                    for u in &trips[i + 1..] {
                        out.push(imp(
                            and(pred(rel, vec![c(t), c(val)]), pred(rel, vec![c(u), c(val)])),
                            id(c(t), c(u)),
                        ));
                    }
                }
            }
        };
        for (i, t) in trips.iter().enumerate() {
            for u in &trips[i + 1..] {
                premises.push(neq(t, u));
            }
        }
        category("In", &["2001", "2002", "2003", "2004"], &mut premises);
        category("In", &["CT", "FL", "KY", "ME"], &mut premises);
        category("With", &["Bill", "Lillie", "Neal", "Yvonne"], &mut premises);
        premises.push(pred("In", vec![c("Alpha"), c("FL")]));
        premises.push(pred("In", vec![c("Beta"), c("KY")]));
        premises.push(pred("In", vec![c("Gamma"), c("CT")]));
        let goal = pred("In", vec![c("Delta"), c("ME")]);
        let tree = grid_prove(&premises, &goal).expect("Delta→ME must be forced by propagation");
        let r = verify(&premises, &goal, tree);
        assert!(r.verified, "solver-emitted multi-category proof must certify; err: {:?}", r.verification_error);
    }

    // ── PHASE C: DPLL decision search over compound (of-pair) clauses ────────────

    /// An of-pair clue ((In(A,FL) ∧ With(B,Neal)) ∨ (In(B,FL) ∧ With(A,Neal))) forces a
    /// third trip OUT of Florida: assuming In(C,FL), the at-most-one rules exclude both
    /// In(A,FL) and In(B,FL), collapsing the of-pair clause to ⊥. The solver searches
    /// and emits a certified case-analysis derivation.
    #[test]
    fn of_pair_forces_cell_certifies() {
        let in_fl = |t: &str| pred("In", vec![c(t), c("FL")]);
        let amo = |t: &str, u: &str| imp(and(in_fl(t), in_fl(u)), id(c(t), c(u)));
        let of_pair = or(
            and(in_fl("A"), pred("With", vec![c("B"), c("Neal")])),
            and(in_fl("B"), pred("With", vec![c("A"), c("Neal")])),
        );
        let premises = vec![
            of_pair,
            amo("A", "C"),
            amo("B", "C"),
            not(id(c("A"), c("C"))),
            not(id(c("B"), c("C"))),
            not(id(c("A"), c("B"))),
        ];
        let goal = not(in_fl("C"));
        let tree = grid_prove(&premises, &goal).expect("of-pair must force ¬In(C,FL)");
        let r = verify(&premises, &goal, tree);
        assert!(r.verified, "solver-emitted of-pair proof must certify; err: {:?}", r.verification_error);
    }

    /// The compound of-pair shape: `(A ∧ (P ∨ Q)) ∨ (Bad ∧ C)` with `¬Bad`, `P→¬R`,
    /// `Q→¬R`. Assuming `R`, the `(Bad ∧ C)` arm collapses on `¬Bad` and the `A∧(P∨Q)`
    /// arm case-splits the inner `P ∨ Q`, both arms contradicting `R`. The solver finds
    /// the nested case analysis and emits a certified ⊥, discharged by reductio to `¬R`.
    #[test]
    fn compound_of_pair_resolves_certifies() {
        let a = pred("A", vec![c("Obj")]);
        let p = pred("P", vec![c("Obj")]);
        let q = pred("Q", vec![c("Obj")]);
        let bad = pred("Bad", vec![c("Obj")]);
        let cc = pred("C", vec![c("Obj")]);
        let rr = pred("R", vec![c("Obj")]);
        let of_pair = or(and(a, or(p.clone(), q.clone())), and(bad.clone(), cc));
        let premises = vec![
            of_pair,
            not(bad),
            imp(p, not(rr.clone())),
            imp(q, not(rr.clone())),
        ];
        let goal = not(rr);
        let tree = grid_prove(&premises, &goal).expect("compound of-pair must force ¬R");
        let r = verify(&premises, &goal, tree);
        assert!(r.verified, "solver-emitted compound of-pair proof must certify; err: {:?}", r.verification_error);
    }

    fn count_rule(t: &DerivationTree, rule_dbg: &str) -> usize {
        let here = usize::from(format!("{:?}", t.rule).starts_with(rule_dbg));
        here + t.premises.iter().map(|c| count_rule(c, rule_dbg)).sum::<usize>()
    }

    /// WAVE 1: a compound (of-pair-shaped) clause `(P∧Q) ∨ (R∧S)` with `¬R` refuting the
    /// second disjunct — disjunctive syllogism forces the first, so `P` and `Q` are forced
    /// by PROPAGATION (no search). The emitted proof must be LINEAR: zero `DisjunctionCases`.
    #[test]
    fn compound_clause_unit_propagates_linearly() {
        let o = || c("Obj");
        let p = pred("P", vec![o()]);
        let q = pred("Q", vec![o()]);
        let r = pred("R", vec![o()]);
        let s = pred("S", vec![o()]);
        let clause = or(and(p.clone(), q.clone()), and(r.clone(), s.clone()));
        let premises = vec![clause, not(r)];
        let tree = grid_prove(&premises, &p).expect("P must be forced by compound-clause propagation");
        assert_eq!(
            count_rule(&tree, "DisjunctionCases"),
            0,
            "compound-clause propagation must emit a LINEAR proof (no DisjunctionCases)"
        );
        let res = verify(&premises, &p, tree);
        assert!(res.verified, "linear compound-clause proof must certify; err: {:?}", res.verification_error);
    }

    // ── PHASE E (de-risk): the REAL studio Simon, through prepare_premises ───────

    /// The studio `Simon` example (2 trips × 2 categories: year {2003,2004} + state
    /// {Florida,Maine}), run through the SAME preparation `ui_bridge::prepare_premises`
    /// uses — `at_most_one_lemmas` + sort-aware grounding + `discharge_unary_facts` —
    /// then solved and certified by `grid_solver`. The grounded `∃`-existence clauses
    /// are skipped by `compile`; closures + at-most-one + the pins force `Beta∈Maine`.
    #[test]
    fn studio_simon_two_category_certifies() {
        use crate::grounding::{
            at_most_one_lemmas, discharge_unary_facts, domain_constants, ground_sorted, sort_domains,
        };
        let trip = |t: ProofTerm| pred("Trip", vec![t]);
        let in_ = |t: ProofTerm, s: ProofTerm| pred("In", vec![t, s]);
        let exactly_one = |val: &str| {
            let phi = |t: ProofTerm| and(trip(t.clone()), in_(t, c(val)));
            exists(
                "x",
                and(phi(v("x")), forall("y", imp(phi(v("y")), id(v("y"), v("x"))))),
            )
        };
        let mut premises = vec![
            trip(c("Alpha")),
            trip(c("Beta")),
            not(id(c("Alpha"), c("Beta"))),
            forall("x", imp(trip(v("x")), or(in_(v("x"), c("2003")), in_(v("x"), c("2004"))))),
            forall("x", imp(trip(v("x")), or(in_(v("x"), c("Florida")), in_(v("x"), c("Maine"))))),
            exactly_one("2003"),
            exactly_one("Florida"),
            in_(c("Alpha"), c("2003")),
            in_(c("Alpha"), c("Florida")),
        ];
        let goal = in_(c("Beta"), c("Maine"));
        // prepare_premises (tense erasure is a no-op for present-tense "is in").
        premises.extend(at_most_one_lemmas(&premises));
        let mut all = premises.clone();
        all.push(goal.clone());
        let fallback = domain_constants(&all);
        let sorts = sort_domains(&premises);
        let grounded: Vec<ProofExpr> = premises.iter().map(|p| ground_sorted(p, &sorts, &fallback)).collect();
        let prepared = discharge_unary_facts(&grounded);
        let tree = grid_prove(&prepared, &goal).expect("studio Simon must solve by propagation");
        let r = verify(&prepared, &goal, tree);
        assert!(r.verified, "studio Simon (2-category) must certify via grid_solver; err: {:?}", r.verification_error);
    }
}
