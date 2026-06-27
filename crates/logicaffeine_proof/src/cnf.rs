//! Tseitin clausification: a grounded, quantifier-free `ProofExpr` → CNF over the CDCL
//! core's `Lit`s (Tseitin, 1968 — the standard linear-size equisatisfiable transform that
//! introduces one auxiliary variable per compound subformula). This is the bridge from the
//! logic-grid encoding (closures, at-most-one, of-pair, clue clauses) to the [`crate::cdcl`]
//! SAT engine, giving the **Untrusted** trust tier: solve `premises ∧ ¬goal`; UNSAT ⇒ the
//! goal is entailed. The same CNF feeds the RUP checker (the fast trust tier) and is the
//! input an AllDifferent GAC propagator filters.

use crate::cdcl::{Lit, SolveResult, Solver, Var};
use crate::{ProofExpr, ProofTerm};
use std::collections::{HashMap, HashSet};

// Tseitin operator tags for the structural-hashing cache key.
const OP_AND: u8 = 0;
const OP_OR: u8 = 1;
const OP_IMPL: u8 = 2;
const OP_IFF: u8 = 3;

/// Canonical ordering of two literals (by variable, then sign) — for commutative-operator
/// keys so `a∧b` and `b∧a` share one auxiliary variable.
#[inline]
fn order2(a: Lit, b: Lit) -> (Lit, Lit) {
    if (a.var(), a.is_positive()) <= (b.var(), b.is_positive()) {
        (a, b)
    } else {
        (b, a)
    }
}

/// A CNF formula under construction: a variable table (atoms canonicalised so `a = b` and
/// `b = a` share a variable) plus the accumulating clause set. `Clone` so a prepared
/// premise CNF can be reused per goal — the of-pair Tseitin work is done once, then each
/// cell only adds its `¬goal` unit (incremental solving, the IPASIR pattern).
#[derive(Clone, Default)]
pub struct Cnf {
    atom_of: HashMap<String, Var>,
    num_vars: usize,
    clauses: Vec<Vec<Lit>>,
    /// Structural-hashing cache: `(op, lit, lit) → aux literal`, so a subformula encoded
    /// twice reuses ONE auxiliary variable (and its defining clauses) instead of minting a
    /// fresh one. This is hash-consing of the Tseitin encoding.
    expr_cache: HashMap<(u8, Lit, Lit), Lit>,
    /// Canonicalised (sorted) clauses already added, so a clause produced twice by grounding
    /// is stored once.
    clause_seen: HashSet<Vec<Lit>>,
}

impl Cnf {
    pub fn new() -> Self {
        Self::default()
    }

    /// Clausify a fixed premise set ONCE (the expensive of-pair Tseitin work), so many
    /// goals can be checked against it by cloning + adding `¬goal`. `None` if any premise is
    /// not encodable. This is the incremental entry for solving a whole puzzle (16+ cells)
    /// without re-grounding or re-clausifying the shared premises every time.
    pub fn from_premises(premises: &[ProofExpr]) -> Option<Cnf> {
        let mut cnf = Cnf::new();
        for p in premises {
            cnf.assert(p)?;
        }
        Some(cnf)
    }

    /// Number of CDCL variables allocated (atoms + Tseitin auxiliaries).
    pub fn num_vars(&self) -> usize {
        self.num_vars
    }

    /// Number of distinct ATOM variables (the rest are Tseitin auxiliaries).
    pub fn num_atoms(&self) -> usize {
        self.atom_of.len()
    }

    /// The accumulated clauses.
    pub fn clauses(&self) -> &[Vec<Lit>] {
        &self.clauses
    }

    /// The Boolean value of atom `e` under a SAT `model` (a [`crate::cdcl::SolveResult::Sat`]
    /// assignment, indexed by variable), or `None` if `e` is not a recognised atom or was
    /// never encoded into this CNF. This decodes a model back to source atoms (e.g.
    /// `signal@t`), skipping the Tseitin auxiliaries that carry no source meaning.
    pub fn atom_value(&self, e: &ProofExpr, model: &[bool]) -> Option<bool> {
        let key = atom_key(e)?;
        let v = *self.atom_of.get(&key)?;
        model.get(v as usize).copied()
    }

    fn fresh(&mut self) -> Var {
        let v = self.num_vars as Var;
        self.num_vars += 1;
        v
    }

    /// The variable for an atom, allocating one the first time it is seen.
    fn atom_var(&mut self, key: String) -> Var {
        if let Some(&v) = self.atom_of.get(&key) {
            return v;
        }
        let v = self.fresh();
        self.atom_of.insert(key, v);
        v
    }

    /// Add a clause, canonicalised (sorted + within-clause dedup), dropping tautologies and
    /// EXACT duplicates. Grid grounding emits the same clause many times (≈13% were dups);
    /// storing each once shrinks the solve and the RUP replay for free.
    fn push_clause(&mut self, mut lits: Vec<Lit>) {
        lits.sort_by_key(|l| (l.var(), l.is_positive()));
        lits.dedup();
        // Sorted ⇒ both polarities of a variable are adjacent: that is a tautology, drop it.
        if lits.windows(2).any(|w| w[0].var() == w[1].var()) {
            return;
        }
        if self.clause_seen.insert(lits.clone()) {
            self.clauses.push(lits);
        }
    }

    /// Tseitin-encode `e`, returning a literal whose truth equals `e`'s, and emitting the
    /// defining clauses for any auxiliary variables. Returns `None` if `e` is not a
    /// quantifier-free propositional formula over recognisable atoms (so the caller can
    /// fall back to another engine rather than silently mis-encode).
    pub fn encode(&mut self, e: &ProofExpr) -> Option<Lit> {
        match e {
            ProofExpr::Not(p) => Some(self.encode(p)?.negated()),
            ProofExpr::And(p, q) => {
                let (a, b) = order2(self.encode(p)?, self.encode(q)?);
                if let Some(&x) = self.expr_cache.get(&(OP_AND, a, b)) {
                    return Some(x);
                }
                let x = Lit::pos(self.fresh());
                // x ↔ (a ∧ b)
                self.push_clause(vec![x.negated(), a]);
                self.push_clause(vec![x.negated(), b]);
                self.push_clause(vec![x, a.negated(), b.negated()]);
                self.expr_cache.insert((OP_AND, a, b), x);
                Some(x)
            }
            ProofExpr::Or(p, q) => {
                let (a, b) = order2(self.encode(p)?, self.encode(q)?);
                if let Some(&x) = self.expr_cache.get(&(OP_OR, a, b)) {
                    return Some(x);
                }
                let x = Lit::pos(self.fresh());
                // x ↔ (a ∨ b)
                self.push_clause(vec![x.negated(), a, b]);
                self.push_clause(vec![x, a.negated()]);
                self.push_clause(vec![x, b.negated()]);
                self.expr_cache.insert((OP_OR, a, b), x);
                Some(x)
            }
            ProofExpr::Implies(p, q) => {
                // p → q  ≡  ¬p ∨ q (NOT commutative, so no `order2`).
                let a = self.encode(p)?;
                let b = self.encode(q)?;
                if let Some(&x) = self.expr_cache.get(&(OP_IMPL, a, b)) {
                    return Some(x);
                }
                let x = Lit::pos(self.fresh());
                // x ↔ (¬a ∨ b)
                self.push_clause(vec![x.negated(), a.negated(), b]);
                self.push_clause(vec![x, a]);
                self.push_clause(vec![x, b.negated()]);
                self.expr_cache.insert((OP_IMPL, a, b), x);
                Some(x)
            }
            ProofExpr::Iff(p, q) => {
                let (a, b) = order2(self.encode(p)?, self.encode(q)?);
                if let Some(&x) = self.expr_cache.get(&(OP_IFF, a, b)) {
                    return Some(x);
                }
                let x = Lit::pos(self.fresh());
                // x ↔ (a ↔ b)
                self.push_clause(vec![x.negated(), a.negated(), b]);
                self.push_clause(vec![x.negated(), a, b.negated()]);
                self.push_clause(vec![x, a, b]);
                self.push_clause(vec![x, a.negated(), b.negated()]);
                self.expr_cache.insert((OP_IFF, a, b), x);
                Some(x)
            }
            _ => {
                // An atom (Predicate / Identity / Atom): a single Boolean variable.
                let key = atom_key(e)?;
                Some(Lit::pos(self.atom_var(key)))
            }
        }
    }

    /// Assert `e` as CNF, introducing auxiliary variables ONLY for genuinely non-clausal
    /// structure (a disjunct that is itself a conjunction — e.g. an of-pair disjunct). A
    /// top-level conjunction splits into separate clauses; a disjunction or implication
    /// flattens into ONE clause; a literal stays a literal. So a closure `A∨B∨C∨D` becomes a
    /// single clause with ZERO aux variables instead of a Tseitin spine. This structure-aware
    /// clausification (Plaisted & Greenbaum, 1986) is what keeps the CNF — and therefore the
    /// solve and the RUP replay — small. `None` if `e` is not encodable.
    pub fn assert(&mut self, e: &ProofExpr) -> Option<()> {
        match e {
            ProofExpr::And(a, b) => {
                self.assert(a)?;
                self.assert(b)
            }
            ProofExpr::Implies(a, b) => {
                // ¬a ∨ b, flattened into one clause.
                let mut lits = self.clause_lits(&negate_expr(a))?;
                lits.extend(self.clause_lits(b)?);
                self.push_clause(lits);
                Some(())
            }
            _ => {
                let lits = self.clause_lits(e)?;
                self.push_clause(lits);
                Some(())
            }
        }
    }

    /// Assert the NEGATION of `e` (used to refute the goal). A literal goal becomes a unit
    /// clause; a compound goal asserts its De-Morgan dual, clause by clause.
    pub fn assert_neg(&mut self, e: &ProofExpr) -> Option<()> {
        self.assert(&negate_expr(e))
    }

    /// The literals of `e` viewed as ONE clause: flatten `∨` and `→`, literalise atoms and
    /// negated atoms, and Tseitin any other (non-literal) disjunct into a single auxiliary
    /// literal.
    fn clause_lits(&mut self, e: &ProofExpr) -> Option<Vec<Lit>> {
        if let Some(l) = self.lit_of_atom(e) {
            return Some(vec![l]);
        }
        match e {
            ProofExpr::Or(a, b) => {
                let mut v = self.clause_lits(a)?;
                v.extend(self.clause_lits(b)?);
                Some(v)
            }
            ProofExpr::Implies(a, b) => {
                let mut v = self.clause_lits(&negate_expr(a))?;
                v.extend(self.clause_lits(b)?);
                Some(v)
            }
            // De Morgan keeps a negated subformula a single clause with NO auxiliary:
            // ¬(a ∧ b) ≡ ¬a ∨ ¬b, and ¬¬x ≡ x. (¬(a ∨ b) is a conjunction, not a clause, so it
            // still falls through to one auxiliary below.) Every at-most-one pair is `¬(a ∧ b)`,
            // so this collapses hundreds of Tseitin spines into plain binary clauses.
            ProofExpr::Not(inner) => match inner.as_ref() {
                ProofExpr::And(a, b) => {
                    let mut v = self.clause_lits(&negate_expr(a))?;
                    v.extend(self.clause_lits(&negate_expr(b))?);
                    Some(v)
                }
                ProofExpr::Not(x) => self.clause_lits(x),
                _ => Some(vec![self.encode(e)?]),
            },
            // A non-clausal subformula (typically a conjunction): one auxiliary variable.
            _ => Some(vec![self.encode(e)?]),
        }
    }

    /// A literal for `e` if it is an atom or a negated atom, else `None`.
    fn lit_of_atom(&mut self, e: &ProofExpr) -> Option<Lit> {
        match e {
            ProofExpr::Not(p) => {
                let key = atom_key(p)?;
                Some(Lit::pos(self.atom_var(key)).negated())
            }
            ProofExpr::Predicate { .. } | ProofExpr::Identity(..) | ProofExpr::Atom(_) => {
                let key = atom_key(e)?;
                Some(Lit::pos(self.atom_var(key)))
            }
            _ => None,
        }
    }

    /// Hand the accumulated CNF to a fresh CDCL solver.
    pub fn into_solver(self) -> Solver {
        self.into_solver_with_atoms().0
    }

    /// Like [`into_solver`](Self::into_solver) but also hands back the atom→variable map (moved,
    /// not cloned). Callers that need to decode a SAT model can do so from this small map instead
    /// of cloning the entire clause database just to keep the table alive.
    pub fn into_solver_with_atoms(self) -> (Solver, HashMap<String, Var>) {
        let mut s = Solver::new(self.num_vars);
        for c in self.clauses {
            s.add_clause(c);
        }
        (s, self.atom_of)
    }
}

/// Does `premises ⊨ goal` (propositionally)? Encodes `premises ∧ ¬goal` to CNF and runs
/// CDCL: UNSAT ⇒ entailed. `None` if the problem is not purely propositional over
/// recognisable atoms (caller falls back). This is the **Untrusted** tier — the verdict
/// with no emitted proof, for the raw-speed comparison against Z3.
pub fn cdcl_entails(premises: &[ProofExpr], goal: &ProofExpr) -> Option<bool> {
    let mut cnf = Cnf::new();
    for p in premises {
        cnf.assert(p)?;
    }
    cnf.assert_neg(goal)?;
    let mut solver = cnf.into_solver();
    Some(solver.solve() == SolveResult::Unsat)
}

/// `¬e`, pushing the negation no further than one `Not` (clausification handles the rest).
fn negate_expr(e: &ProofExpr) -> ProofExpr {
    match e {
        ProofExpr::Not(p) => (**p).clone(),
        _ => ProofExpr::Not(Box::new(e.clone())),
    }
}

/// Canonical string key for an atom, so the same proposition always maps to one variable.
/// Identity is symmetric (`a = b` ≡ `b = a`), so its operands are ordered.
fn atom_key(e: &ProofExpr) -> Option<String> {
    match e {
        ProofExpr::Predicate { name, args, .. } => {
            let mut s = String::new();
            s.push_str(name);
            s.push('(');
            for (i, a) in args.iter().enumerate() {
                if i > 0 {
                    s.push(',');
                }
                s.push_str(&term_key(a));
            }
            s.push(')');
            Some(s)
        }
        ProofExpr::Identity(a, b) => {
            let (ka, kb) = (term_key(a), term_key(b));
            let (lo, hi) = if ka <= kb { (ka, kb) } else { (kb, ka) };
            Some(format!("={}={}", lo, hi))
        }
        ProofExpr::Atom(name) => Some(format!("atom:{name}")),
        _ => None,
    }
}

fn term_key(t: &ProofTerm) -> String {
    match t {
        ProofTerm::Constant(s) => s.clone(),
        ProofTerm::Variable(s) | ProofTerm::BoundVarRef(s) => format!("?{s}"),
        ProofTerm::Function(n, args) => {
            let inner: Vec<String> = args.iter().map(term_key).collect();
            format!("{n}({})", inner.join(","))
        }
        ProofTerm::Group(_) => "<group>".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ProofExpr, ProofTerm};

    fn c(s: &str) -> ProofTerm {
        ProofTerm::Constant(s.to_string())
    }
    fn pred(name: &str, args: Vec<ProofTerm>) -> ProofExpr {
        ProofExpr::Predicate { name: name.to_string(), args, world: None }
    }
    fn not(e: ProofExpr) -> ProofExpr {
        ProofExpr::Not(Box::new(e))
    }
    fn or(a: ProofExpr, b: ProofExpr) -> ProofExpr {
        ProofExpr::Or(Box::new(a), Box::new(b))
    }
    fn and(a: ProofExpr, b: ProofExpr) -> ProofExpr {
        ProofExpr::And(Box::new(a), Box::new(b))
    }
    fn imp(a: ProofExpr, b: ProofExpr) -> ProofExpr {
        ProofExpr::Implies(Box::new(a), Box::new(b))
    }
    fn id(a: ProofTerm, b: ProofTerm) -> ProofExpr {
        ProofExpr::Identity(a, b)
    }

    #[test]
    fn reflexive_entailment() {
        let p = pred("P", vec![c("a")]);
        assert_eq!(cdcl_entails(&[p.clone()], &p), Some(true));
    }

    #[test]
    fn disjunctive_syllogism() {
        // P ∨ Q, ¬P ⊨ Q
        let p = pred("P", vec![c("a")]);
        let q = pred("Q", vec![c("a")]);
        let prem = vec![or(p.clone(), q.clone()), not(p.clone())];
        assert_eq!(cdcl_entails(&prem, &q), Some(true));
        // …but P ∨ Q alone does NOT entail Q.
        assert_eq!(cdcl_entails(&[or(p.clone(), q.clone())], &q), Some(false));
    }

    #[test]
    fn modus_ponens() {
        // P, P → Q ⊨ Q
        let p = pred("P", vec![c("a")]);
        let q = pred("Q", vec![c("a")]);
        let prem = vec![p.clone(), imp(p.clone(), q.clone())];
        assert_eq!(cdcl_entails(&prem, &q), Some(true));
    }

    #[test]
    fn non_entailment_is_false_not_none() {
        let p = pred("P", vec![c("a")]);
        let q = pred("Q", vec![c("a")]);
        assert_eq!(cdcl_entails(&[p], &q), Some(false));
    }

    #[test]
    fn identity_is_symmetric() {
        // a = b ⊨ b = a (same variable, so ¬(b=a) clashes immediately).
        let prem = vec![id(c("a"), c("b"))];
        assert_eq!(cdcl_entails(&prem, &id(c("b"), c("a"))), Some(true));
    }

    #[test]
    fn two_value_grid_forces_the_cell() {
        // A 2-trip × 2-state bijection: each trip in FL or ME; at most one trip per state;
        // Alpha in FL ⇒ Beta in ME. The Untrusted CDCL tier must force it.
        let in_ = |t: &str, s: &str| pred("In", vec![c(t), c(s)]);
        let fl = "Florida";
        let me = "Maine";
        let prem = vec![
            // closures: each trip takes a state
            or(in_("Alpha", fl), in_("Alpha", me)),
            or(in_("Beta", fl), in_("Beta", me)),
            // at-most-one per state (functionality): two trips can't share a state
            imp(in_("Alpha", fl), not(in_("Beta", fl))),
            imp(in_("Alpha", me), not(in_("Beta", me))),
            // each state taken by some trip (column closure)
            or(in_("Alpha", fl), in_("Beta", fl)),
            or(in_("Alpha", me), in_("Beta", me)),
            // the pin
            in_("Alpha", fl),
        ];
        assert_eq!(cdcl_entails(&prem, &in_("Beta", me)), Some(true), "Beta∈Maine is forced");
        assert_eq!(cdcl_entails(&prem, &in_("Beta", fl)), Some(false), "Beta∈Florida is refuted");
    }

    #[test]
    fn negated_conjunction_clausifies_directly_without_aux() {
        // ¬(P ∧ Q) ≡ ¬P ∨ ¬Q — one binary clause, NO Tseitin auxiliary. Every at-most-one
        // constraint is this shape; minting an aux per pair bloated the CNF and the solve.
        let p = pred("P", vec![c("a")]);
        let q = pred("Q", vec![c("a")]);
        let mut cnf = Cnf::new();
        cnf.assert(&not(and(p, q))).unwrap();
        assert_eq!(cnf.num_atoms(), 2, "only P and Q are atoms");
        assert_eq!(cnf.num_vars(), 2, "no auxiliary variable should be minted");
        assert_eq!(cnf.clauses().len(), 1, "exactly one clause");
        assert_eq!(cnf.clauses()[0].len(), 2, "the binary clause ¬P ∨ ¬Q");
    }

    #[test]
    fn negated_conjunction_preserves_models() {
        // Soundness guard for the De Morgan clause: P ∧ ¬(P ∧ Q) ⊨ ¬Q, but ¬(P ∧ Q) alone does not.
        let p = pred("P", vec![c("a")]);
        let q = pred("Q", vec![c("a")]);
        assert_eq!(
            cdcl_entails(&[p.clone(), not(and(p.clone(), q.clone()))], &not(q.clone())),
            Some(true)
        );
        assert_eq!(cdcl_entails(&[not(and(p, q.clone()))], &not(q)), Some(false));
    }

    #[test]
    fn nested_demorgan_and_double_negation_clausify() {
        // ¬((P ∧ Q) ∧ R) ≡ ¬P ∨ ¬Q ∨ ¬R (one ternary clause, no aux); ¬¬P ≡ P.
        let (p, q, r) = (pred("P", vec![c("a")]), pred("Q", vec![c("a")]), pred("R", vec![c("a")]));
        let mut cnf = Cnf::new();
        cnf.assert(&not(and(and(p, q), r))).unwrap();
        assert_eq!(cnf.num_vars(), 3, "no aux for a nested negated conjunction");
        assert_eq!(cnf.clauses().len(), 1);
        assert_eq!(cnf.clauses()[0].len(), 3, "ternary clause ¬P ∨ ¬Q ∨ ¬R");

        let p2 = pred("P", vec![c("a")]);
        let mut cnf2 = Cnf::new();
        cnf2.assert(&not(not(p2))).unwrap();
        assert_eq!(cnf2.num_vars(), 1, "¬¬P is just P");
    }
}
