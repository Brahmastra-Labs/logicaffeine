//! Pseudo-Boolean constraints + cutting planes — a proof system STRICTLY STRONGER than resolution.
//!
//! Resolution (and therefore CDCL — ours and Z3's) needs EXPONENTIALLY long refutations of the
//! pigeonhole principle and of many counting/cardinality problems. **Cutting planes** refutes them
//! in POLYNOMIAL size by reasoning about linear 0/1 inequalities directly: it p-simulates resolution
//! and has short proofs where resolution provably cannot.
//!
//! This is the certified core: a normalized PB constraint `Σ aᵢ·ℓᵢ ≥ d` (coeffs > 0, each variable
//! once, `ℓ` a literal `x` or `¬x`) and the four sound cutting-plane rules — **addition** (with
//! literal cancellation via `x + ¬x = 1`), **multiplication**, **division-with-rounding** (the
//! Gomory–Chvátal cut, sound only because variables are integral), and **saturation** — each of
//! which is *implied by* its premises, so deriving the trivially-false `0 ≥ 1` refutes the inputs.
//! Every rule is pinned against a brute-force oracle. The headline is [`php_refutation`]: the
//! classic Cook–Coullard–Turán **linear-size** refutation of PHP — pure algebra where resolution
//! explodes.

use std::collections::BTreeMap;

/// A normalized pseudo-Boolean constraint `Σ coeff·lit ≥ degree`: every `coeff > 0`, every variable
/// appears at most once, and the literal sign rides the entry — `(coeff, positive)` with
/// `positive == false` meaning the literal `¬x`. `degree` may be ≤ 0 (then the constraint is
/// trivially true) or exceed the coefficient sum (then it is a contradiction).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PbConstraint {
    terms: BTreeMap<usize, (i64, bool)>,
    degree: i64,
}

impl PbConstraint {
    /// `Σ ℓ ≥ k` over the given literals (each coefficient 1). Literals must be over distinct
    /// variables (the caller's responsibility — true for clause/cardinality inputs).
    pub fn at_least(lits: &[(usize, bool)], k: i64) -> PbConstraint {
        let terms = lits.iter().map(|&(v, s)| (v, (1, s))).collect();
        PbConstraint { terms, degree: k }
    }

    /// A CNF clause `ℓ₁ ∨ … ∨ ℓₖ` as the PB constraint `Σ ℓ ≥ 1` — the bridge that lets the cutting
    /// plane engine consume ordinary SAT clauses (and p-simulate resolution on them).
    pub fn clause(lits: &[(usize, bool)]) -> PbConstraint {
        Self::at_least(lits, 1)
    }

    /// `Σ ℓ ≤ k`, normalized to `≥` form by flipping every literal: `Σ ¬ℓ ≥ n − k`.
    pub fn at_most(lits: &[(usize, bool)], k: i64) -> PbConstraint {
        let negated: Vec<(usize, bool)> = lits.iter().map(|&(v, s)| (v, !s)).collect();
        Self::at_least(&negated, lits.len() as i64 - k)
    }

    /// The degree (right-hand side).
    pub fn degree(&self) -> i64 {
        self.degree
    }

    /// Number of (non-cancelled) terms.
    pub fn len(&self) -> usize {
        self.terms.len()
    }

    pub fn is_empty(&self) -> bool {
        self.terms.is_empty()
    }

    /// **Addition** — `LHS₁ + LHS₂ ≥ d₁ + d₂`, then re-normalized. A variable that is `x` in one and
    /// `¬x` in the other partially cancels (`a·x + b·¬x = (a−b)·x + b`), so the shared minimum
    /// `min(a,b)` leaves the literal and is moved to the right, dropping the degree by `min(a,b)`.
    /// Sound: the sum of two valid `≥` constraints is valid, and `x + ¬x = 1` is exact for 0/1.
    pub fn add(&self, other: &PbConstraint) -> PbConstraint {
        let mut terms = self.terms.clone();
        let mut degree = self.degree + other.degree;
        for (&v, &(c2, s2)) in &other.terms {
            match terms.get(&v).copied() {
                None => {
                    terms.insert(v, (c2, s2));
                }
                Some((c1, s1)) if s1 == s2 => {
                    terms.insert(v, (c1 + c2, s1));
                }
                Some((c1, _s1_opposite)) => {
                    let cancel = c1.min(c2);
                    degree -= cancel;
                    match c1.cmp(&c2) {
                        std::cmp::Ordering::Equal => {
                            terms.remove(&v);
                        }
                        std::cmp::Ordering::Greater => {
                            terms.insert(v, (c1 - c2, self.terms[&v].1));
                        }
                        std::cmp::Ordering::Less => {
                            terms.insert(v, (c2 - c1, s2));
                        }
                    }
                }
            }
        }
        PbConstraint { terms, degree }
    }

    /// **Multiplication** by a positive integer — `c·LHS ≥ c·degree`. Trivially sound (scaling a
    /// valid `≥` constraint by `c > 0`).
    pub fn multiply(&self, c: i64) -> PbConstraint {
        assert!(c > 0, "multiply by a positive integer");
        PbConstraint {
            terms: self.terms.iter().map(|(&v, &(coeff, s))| (v, (coeff * c, s))).collect(),
            degree: self.degree * c,
        }
    }

    /// **Division with rounding-up** (the Gomory–Chvátal cut) — `Σ ⌈aᵢ/d⌉·ℓᵢ ≥ ⌈degree/d⌉`. Sound
    /// ONLY because variables are integral: `⌈aᵢ/d⌉ ≥ aᵢ/d` so the LHS only grows (≥ degree/d), and
    /// being an integer it is `≥ ⌈degree/d⌉`. This rounding of the fractional slack is exactly what
    /// makes cutting planes stronger than resolution.
    pub fn divide_round(&self, d: i64) -> PbConstraint {
        assert!(d > 0, "divide by a positive integer");
        let ceil = |x: i64| x.div_euclid(d) + i64::from(x.rem_euclid(d) != 0);
        PbConstraint {
            terms: self.terms.iter().map(|(&v, &(c, s))| (v, (ceil(c), s))).collect(),
            degree: ceil(self.degree),
        }
    }

    /// **Saturation** — clamp every coefficient to the degree (`min(aᵢ, degree)`). A single 0/1
    /// variable can contribute at most `degree` toward the bound, so the excess is redundant. Sound,
    /// and it keeps coefficients from blowing up across many additions.
    pub fn saturate(&self) -> PbConstraint {
        let d = self.degree;
        PbConstraint {
            terms: self.terms.iter().map(|(&v, &(c, s))| (v, (c.min(d), s))).collect(),
            degree: d,
        }
    }

    /// A constraint NO 0/1 assignment can satisfy: the maximum possible LHS (every coefficient
    /// counted) is still below the degree. The terminal of a refutation — e.g. `0 ≥ 1`.
    pub fn is_contradiction(&self) -> bool {
        let max_lhs: i64 = self.terms.values().map(|&(c, _)| c).sum();
        self.degree > max_lhs
    }

    /// Evaluate under `assign` (a variable → bool map; unmentioned variables are false): does
    /// `Σ coeff·[literal true] ≥ degree` hold? The oracle the rule soundness tests check against.
    pub fn is_satisfied(&self, assign: &dyn Fn(usize) -> bool) -> bool {
        let lhs: i64 = self
            .terms
            .iter()
            .map(|(&v, &(c, s))| if assign(v) == s { c } else { 0 })
            .sum();
        lhs >= self.degree
    }

    /// `Σ coeffᵢ·litᵢ ≥ degree` directly from weighted terms `(var, coeff, positive)` — for constraints
    /// whose coefficients are not all 1.
    pub fn new_weighted(terms: &[(usize, i64, bool)], degree: i64) -> PbConstraint {
        PbConstraint { terms: terms.iter().map(|&(v, c, s)| (v, (c, s))).collect(), degree }
    }

    /// The `(coeff, positive)` term on variable `v`, or `None` if `v` does not occur.
    pub fn term(&self, v: usize) -> Option<(i64, bool)> {
        self.terms.get(&v).copied()
    }

    /// Iterate the constraint's terms as `(variable, coeff, positive)` — the public view of the
    /// otherwise-private normalized map, so a live theory can read a constraint without owning it.
    pub fn terms(&self) -> impl Iterator<Item = (usize, i64, bool)> + '_ {
        self.terms.iter().map(|(&v, &(c, s))| (v, c, s))
    }
}

/// Does the variable permutation `perm` preserve every constraint? (`term(perm[v]) == term(v)` for all
/// `v` — a relabelling that fixes each constraint's coefficient/sign structure, hence its solution set.)
pub fn is_pb_symmetry(constraints: &[PbConstraint], perm: &[usize]) -> bool {
    constraints
        .iter()
        .all(|c| (0..perm.len()).all(|v| c.term(perm[v]) == c.term(v)))
}

/// The **coefficient-symmetry** generators of a pseudo-Boolean system: adjacent transpositions of variables
/// that share a *coefficient profile* — the same `(coeff, sign)` (or absence) in every constraint. Such
/// variables are interchangeable because each constraint is a weighted sum, blind to which equal-weight
/// variable carries which value. Each generator is a genuine symmetry ([`is_pb_symmetry`]); together they
/// generate the full product of symmetric groups on the profile classes. Variables in no constraint are
/// excluded (free, not a constraint symmetry).
pub fn coeff_symmetry_generators(num_vars: usize, constraints: &[PbConstraint]) -> Vec<Vec<usize>> {
    let profile = |v: usize| -> Vec<Option<(i64, bool)>> {
        constraints.iter().map(|c| c.term(v)).collect()
    };
    let mut classes: BTreeMap<Vec<Option<(i64, bool)>>, Vec<usize>> = BTreeMap::new();
    for v in 0..num_vars {
        let p = profile(v);
        if p.iter().all(|t| t.is_none()) {
            continue; // appears in no constraint
        }
        classes.entry(p).or_default().push(v);
    }
    let mut gens = Vec::new();
    for vars in classes.values() {
        for w in vars.windows(2) {
            let mut p: Vec<usize> = (0..num_vars).collect();
            p.swap(w[0], w[1]);
            gens.push(p);
        }
    }
    gens
}

/// Refute the pigeonhole principle PHP(`n`, `n−1`) with the classic Cook–Coullard–Turán cutting-
/// plane proof: sum all `n` "each pigeon in ≥1 hole" constraints with all `n−1` "each hole holds
/// ≤1 pigeon" constraints. Every `x` meets its `¬x` and cancels, collapsing the whole system to the
/// trivially-false `0 ≥ 1` in just `2n−1` additions — **linear** size, where resolution needs
/// exponentially many steps. Returns the final derived constraint (a contradiction).
pub fn php_refutation(n: usize) -> PbConstraint {
    let holes = n - 1;
    let var = |i: usize, h: usize| i * holes + h;
    let mut acc: Option<PbConstraint> = None;
    let mut absorb = |c: PbConstraint| {
        acc = Some(match acc.take() {
            None => c,
            Some(a) => a.add(&c),
        });
    };
    // Each pigeon occupies at least one hole: Σ_h x_{i,h} ≥ 1.
    for i in 0..n {
        let lits: Vec<(usize, bool)> = (0..holes).map(|h| (var(i, h), true)).collect();
        absorb(PbConstraint::clause(&lits));
    }
    // Each hole holds at most one pigeon: Σ_i x_{i,h} ≤ 1.
    for h in 0..holes {
        let lits: Vec<(usize, bool)> = (0..n).map(|i| (var(i, h), true)).collect();
        absorb(PbConstraint::at_most(&lits, 1));
    }
    acc.expect("PHP has at least one constraint")
}

// ─────────────────────────────────────────────────────────────────────────────
// Wiring into the general solver: refute a cardinality/pigeonhole CNF by cutting planes.
// ─────────────────────────────────────────────────────────────────────────────

use crate::ProofExpr;
use std::collections::HashMap;

/// Refute `e` by CUTTING PLANES when it is a cardinality/pigeonhole-shaped CNF. The pairwise
/// encoding loses the cardinality structure, so we RECOVER it: the binary exclusions `¬(a∧b)` form
/// conflict cliques, each a genuine "at most one" group; summing those `at_most(group,1)` cardinality
/// constraints with the positive "at least one" clauses cancels every literal against its negation
/// and collapses a pigeonhole-shaped system to `0 ≥ 1` — POLYNOMIAL, where resolution/CDCL explode.
///
/// Returns `true` ONLY on a genuine cutting-plane contradiction (a sound UNSAT proof); `false` (the
/// caller falls through) for any formula that does not collapse. Never a false `true`: each step is
/// a sound cutting-plane inference, and the at-most-one constraints are *implied* by the verified
/// exclusion cliques.
pub fn refute_clausal(e: &ProofExpr) -> bool {
    let Some((rows, exclusions, nvars)) = extract_clausal(e) else {
        return false;
    };
    if rows.is_empty() {
        return false;
    }
    // Recover at-most-one groups: connected components of the exclusion graph that are full cliques.
    let mut uf = UnionFind::new(nvars);
    for &(a, b) in &exclusions {
        uf.union(a, b);
    }
    let excl_set: std::collections::HashSet<(usize, usize)> =
        exclusions.iter().map(|&(a, b)| (a.min(b), a.max(b))).collect();
    let mut comps: HashMap<usize, Vec<usize>> = HashMap::new();
    for v in 0..nvars {
        comps.entry(uf.find(v)).or_default().push(v);
    }
    let mut sum: Option<PbConstraint> = None;
    let mut absorb = |c: PbConstraint| {
        sum = Some(match sum.take() {
            None => c,
            Some(s) => s.add(&c),
        });
    };
    for row in &rows {
        let lits: Vec<(usize, bool)> = row.iter().map(|&v| (v, true)).collect();
        absorb(PbConstraint::at_least(&lits, 1));
    }
    for members in comps.values() {
        if members.len() < 2 {
            continue;
        }
        // Only a FULL clique is a sound at-most-one (else two members could share a slot).
        let is_clique = members.iter().enumerate().all(|(i, &a)| {
            members[i + 1..].iter().all(|&b| excl_set.contains(&(a.min(b), a.max(b))))
        });
        if !is_clique {
            return false; // incomplete at-most-one → can't soundly refute this way
        }
        let lits: Vec<(usize, bool)> = members.iter().map(|&v| (v, true)).collect();
        absorb(PbConstraint::at_most(&lits, 1));
    }
    match sum {
        Some(c) => c.is_contradiction() || c.saturate().is_contradiction(),
        None => false,
    }
}

/// Flatten `e` into `(positive-clause variable lists, binary exclusion pairs, var count)` over dense
/// variable indices, or `None` if any top-level conjunct is neither an all-positive disjunction nor
/// a binary mutual-exclusion. (A standalone reader so the PB path is independent of the others.)
fn extract_clausal(e: &ProofExpr) -> Option<(Vec<Vec<usize>>, Vec<(usize, usize)>, usize)> {
    let mut conjuncts = Vec::new();
    flatten_and(e, &mut conjuncts);
    let mut idx: HashMap<String, usize> = HashMap::new();
    let mut var = |name: &str, idx: &mut HashMap<String, usize>| -> usize {
        let n = idx.len();
        *idx.entry(name.to_string()).or_insert(n)
    };
    let mut rows = Vec::new();
    let mut excl = Vec::new();
    for c in conjuncts {
        if let Some(atoms) = positive_disjunction(c) {
            rows.push(atoms.iter().map(|a| var(a, &mut idx)).collect());
        } else if let Some((a, b)) = exclusion_pair(c) {
            excl.push((var(&a, &mut idx), var(&b, &mut idx)));
        } else {
            return None;
        }
    }
    let nvars = idx.len();
    Some((rows, excl, nvars))
}

fn flatten_and<'a>(e: &'a ProofExpr, out: &mut Vec<&'a ProofExpr>) {
    // Iterative worklist rather than recursion: a flat CNF clausifies into a left-nested `And` spine
    // whose depth is the clause count, so a recursive walk overflows the stack on a few-thousand-clause
    // formula. The explicit stack handles any depth. Pushing `r` before `l` pops `l` first, preserving
    // the left-to-right clause order the recursive version produced.
    let mut stack = vec![e];
    while let Some(node) = stack.pop() {
        match node {
            ProofExpr::And(l, r) => {
                stack.push(r);
                stack.push(l);
            }
            other => out.push(other),
        }
    }
}

fn positive_disjunction(e: &ProofExpr) -> Option<Vec<String>> {
    fn walk(e: &ProofExpr, out: &mut Vec<String>) -> bool {
        match e {
            ProofExpr::Or(l, r) => walk(l, out) && walk(r, out),
            ProofExpr::Atom(a) => {
                out.push(a.clone());
                true
            }
            _ => false,
        }
    }
    let mut atoms = Vec::new();
    walk(e, &mut atoms).then_some(atoms)
}

fn exclusion_pair(e: &ProofExpr) -> Option<(String, String)> {
    match e {
        ProofExpr::Not(inner) => match inner.as_ref() {
            ProofExpr::And(a, b) => match (a.as_ref(), b.as_ref()) {
                (ProofExpr::Atom(a), ProofExpr::Atom(b)) => Some((a.clone(), b.clone())),
                _ => None,
            },
            _ => None,
        },
        ProofExpr::Or(l, r) => match (l.as_ref(), r.as_ref()) {
            (ProofExpr::Not(a), ProofExpr::Not(b)) => match (a.as_ref(), b.as_ref()) {
                (ProofExpr::Atom(a), ProofExpr::Atom(b)) => Some((a.clone(), b.clone())),
                _ => None,
            },
            _ => None,
        },
        _ => None,
    }
}

struct UnionFind {
    parent: Vec<usize>,
}
impl UnionFind {
    fn new(n: usize) -> Self {
        UnionFind { parent: (0..n).collect() }
    }
    fn find(&mut self, x: usize) -> usize {
        if self.parent[x] != x {
            let r = self.find(self.parent[x]);
            self.parent[x] = r;
        }
        self.parent[x]
    }
    fn union(&mut self, a: usize, b: usize) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra != rb {
            self.parent[ra] = rb;
        }
    }
}

use crate::cdcl::Lit;

/// A **live cardinality theory** for the CDCL engine ([`crate::cdcl::Theory`]) — the trail-driven twin of
/// the static cutting-plane refuter [`refute_clausal`]. It propagates and conflicts on unit-coefficient
/// cardinality constraints (`Σ ℓ ≥ k`, covering clauses, at-least-`k`, and at-most-`k`) directly on the
/// solver's trail (stateless: it rebuilds the partial assignment each call, so it can never desync), so it
/// FUSES with a GF(2) parity theory ([`crate::xor_engine::XorEngine`]) under one
/// [`crate::cdcl::Solver::solve_with`]: parity and counting reason *together* on the shared trail. That is
/// exactly the structure that defeats either alone — e.g. minimal-disagreement parity, a parity core plus a
/// residual cardinality core, where Gaussian elimination handles the parity and cardinality propagation
/// handles the counting, but neither is a refutation by itself.
///
/// Pair it with the stateless [`crate::xor_engine::XorEngine`], not the incremental
/// [`crate::xor_engine::IncXor`]: `IncXor`'s trail-sync matches by variable but not value, so a
/// backtrack-then-flip leaves its matrix stale — correct whenever a clausal XOR encoding backs it (the
/// dispatcher's case), but unsound in a pure-theory fusion with no Boolean clauses to mask it.
///
/// **Soundness** rests on the cardinality reason clause: for `Σ ℓ ≥ k` over `n` literals, ANY `n − k + 1`
/// of them form a clause entailed by the constraint (at most `n − k` of the literals can be false, so any
/// `n − k + 1` contain a true one). Hence when more than `n − k` are false, `n − k + 1` false literals are
/// an all-false entailed clause — a **conflict**; when exactly `n − k` are false, every unassigned literal
/// is forced true with reason `{the false literals} ∨ ℓ` (size `n − k + 1`, entailed, currently unit).
/// Every returned clause is a logical consequence of the constraint, so `Solver::solve_with` may carry it
/// into the learned database soundly.
pub struct CardinalityTheory {
    num_vars: usize,
    /// Each constraint as `(literals, k)` in `Σ ℓ ≥ k` form. `at_most`/`clause` normalize into this.
    constraints: Vec<(Vec<(usize, bool)>, i64)>,
}

impl CardinalityTheory {
    /// Build from normalized [`PbConstraint`]s. Every input MUST be unit-coefficient (a cardinality
    /// constraint); a weighted term is rejected (`panic`) rather than risk an unsound weighted reason —
    /// general weighted PB belongs to the static cutting-plane engine ([`refute_clausal`]), not this live
    /// clausal-reason theory.
    pub fn new(num_vars: usize, constraints: &[PbConstraint]) -> Self {
        let constraints = constraints
            .iter()
            .map(|pb| {
                let lits: Vec<(usize, bool)> = pb
                    .terms()
                    .map(|(v, c, s)| {
                        assert_eq!(c, 1, "CardinalityTheory requires unit coefficients (got {c} on var {v})");
                        (v, s)
                    })
                    .collect();
                (lits, pb.degree())
            })
            .collect();
        CardinalityTheory { num_vars, constraints }
    }
}

impl crate::cdcl::Theory for CardinalityTheory {
    fn propagate(&mut self, trail: &[Lit]) -> Vec<Vec<Lit>> {
        let mut a: Vec<Option<bool>> = vec![None; self.num_vars];
        for &l in trail {
            a[l.var() as usize] = Some(l.is_positive());
        }
        let mut out: Vec<Vec<Lit>> = Vec::new();
        for (lits, k) in &self.constraints {
            let k = *k;
            if k <= 0 {
                continue; // trivially satisfied
            }
            let n = lits.len() as i64;
            if k > n {
                out.push(Vec::new()); // Σ ℓ ≥ k > n is unsatisfiable — an unconditional contradiction
                continue;
            }
            let mut false_lits: Vec<(usize, bool)> = Vec::new();
            let mut unassigned: Vec<(usize, bool)> = Vec::new();
            let mut true_count = 0i64;
            for &(v, s) in lits {
                match a[v] {
                    Some(val) if val == s => true_count += 1,
                    Some(_) => false_lits.push((v, s)),
                    None => unassigned.push((v, s)),
                }
            }
            if true_count >= k {
                continue; // already satisfied
            }
            let max_false = n - k; // the constraint tolerates at most this many false literals
            let fc = false_lits.len() as i64;
            if fc > max_false {
                // CONFLICT: any (max_false + 1) of the false literals are an entailed, all-false clause.
                let take = (max_false + 1) as usize;
                out.push(false_lits[..take].iter().map(|&(v, s)| Lit::new(v as u32, s)).collect());
            } else if fc == max_false && !unassigned.is_empty() {
                // Every unassigned literal is forced true: reason = {the false literals} ∨ ℓ.
                let base: Vec<Lit> = false_lits.iter().map(|&(v, s)| Lit::new(v as u32, s)).collect();
                for &(v, s) in &unassigned {
                    let mut c = base.clone();
                    c.push(Lit::new(v as u32, s));
                    out.push(c);
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn for_all_assignments(nvars: usize, mut f: impl FnMut(&dyn Fn(usize) -> bool)) {
        for mask in 0..(1u32 << nvars) {
            let assign = move |v: usize| (mask >> v) & 1 == 1;
            f(&assign);
        }
    }

    /// A rule is sound iff its conclusion is IMPLIED by its premises: every assignment satisfying
    /// all premises also satisfies the derived constraint.
    fn assert_implied(premises: &[&PbConstraint], derived: &PbConstraint, nvars: usize) {
        for_all_assignments(nvars, |a| {
            if premises.iter().all(|c| c.is_satisfied(a)) {
                assert!(
                    derived.is_satisfied(a),
                    "UNSOUND rule: a premise-satisfying assignment falsifies the conclusion"
                );
            }
        });
    }

    #[test]
    fn addition_is_sound() {
        // Exhaustive over small constraints, including the x/¬x cancellation case.
        let c1 = PbConstraint::at_least(&[(0, true), (1, true), (2, true)], 2);
        let c2 = PbConstraint::at_least(&[(0, false), (1, true), (3, true)], 2);
        assert_implied(&[&c1, &c2], &c1.add(&c2), 4);

        // Full cancellation: (x0+x1≥1) + (¬x0+¬x1≥1) — degree drops by 2 → 0 ≥ 0 (trivially true).
        let a = PbConstraint::clause(&[(0, true), (1, true)]);
        let b = PbConstraint::at_least(&[(0, false), (1, false)], 1);
        let sum = a.add(&b);
        assert_implied(&[&a, &b], &sum, 2);
        assert!(sum.is_empty() && sum.degree() == 0, "x+¬x cancels to a trivially-true 0 ≥ 0");
    }

    #[test]
    fn multiply_divide_saturate_are_sound() {
        let c = PbConstraint::at_least(&[(0, true), (1, true), (2, false)], 2);
        assert_implied(&[&c], &c.multiply(3), 3);
        assert_implied(&[&c], &c.saturate(), 3);
        // Division rounds up: a constraint with mixed coefficients.
        let d = PbConstraint { terms: [(0, (5, true)), (1, (3, true)), (2, (2, true))].into(), degree: 6 };
        assert_implied(&[&d], &d.divide_round(2), 3);
        assert_implied(&[&d], &d.divide_round(3), 3);
    }

    #[test]
    fn contradiction_detection() {
        let unsat = PbConstraint { terms: BTreeMap::new(), degree: 1 };
        assert!(unsat.is_contradiction(), "0 ≥ 1 is a contradiction");
        let tight = PbConstraint::at_least(&[(0, true), (1, true)], 2);
        assert!(!tight.is_contradiction(), "x0+x1 ≥ 2 is satisfiable (both true)");
        let over = PbConstraint::at_least(&[(0, true)], 2);
        assert!(over.is_contradiction(), "x0 ≥ 2 is impossible for a 0/1 variable");
    }

    #[test]
    fn pigeonhole_is_refuted_in_linear_size() {
        // THE HEADLINE: cutting planes refutes PHP(n, n-1) with a LINEAR-size derivation (2n-1
        // additions), collapsing to `0 ≥ 1` — where resolution/CDCL need exponentially many steps.
        for n in 2..=30 {
            let refutation = php_refutation(n);
            assert!(
                refutation.is_contradiction(),
                "PHP({n}) must collapse to a contradiction under cutting planes"
            );
            assert!(refutation.is_empty(), "every x meets its ¬x and cancels — the LHS is empty");
            assert_eq!(refutation.degree(), 1, "the terminal is exactly 0 ≥ 1");
        }
    }

    fn php_expr(n: usize) -> ProofExpr {
        let holes = n - 1;
        let p = |i: usize, h: usize| ProofExpr::Atom(format!("p_{i}_{h}"));
        let mut clauses = Vec::new();
        for i in 0..n {
            clauses.push(
                (0..holes).map(|h| p(i, h)).reduce(|a, b| ProofExpr::Or(Box::new(a), Box::new(b))).unwrap(),
            );
        }
        for h in 0..holes {
            for i in 0..n {
                for j in (i + 1)..n {
                    clauses.push(ProofExpr::Not(Box::new(ProofExpr::And(Box::new(p(i, h)), Box::new(p(j, h))))));
                }
            }
        }
        clauses.into_iter().reduce(|a, b| ProofExpr::And(Box::new(a), Box::new(b))).unwrap()
    }

    fn feasible_expr(n: usize) -> ProofExpr {
        // n items into n slots — SATISFIABLE; must NOT be refuted.
        let q = |i: usize, h: usize| ProofExpr::Atom(format!("q_{i}_{h}"));
        let mut clauses = Vec::new();
        for i in 0..n {
            clauses.push((0..n).map(|h| q(i, h)).reduce(|a, b| ProofExpr::Or(Box::new(a), Box::new(b))).unwrap());
        }
        for h in 0..n {
            for i in 0..n {
                for j in (i + 1)..n {
                    clauses.push(ProofExpr::Not(Box::new(ProofExpr::And(Box::new(q(i, h)), Box::new(q(j, h))))));
                }
            }
        }
        clauses.into_iter().reduce(|a, b| ProofExpr::And(Box::new(a), Box::new(b))).unwrap()
    }

    #[test]
    fn cutting_planes_refutes_pairwise_pigeonhole_from_cnf() {
        // The wiring target: a pairwise-encoded PHP CNF (resolution-EXPONENTIAL) is refuted by
        // recovering the at-most-one cardinality from the exclusion cliques and summing — polynomial.
        for n in 2..=10 {
            assert!(refute_clausal(&php_expr(n)), "cutting planes must refute pairwise PHP({n}) from CNF");
        }
    }

    #[test]
    fn cutting_planes_refutation_is_sound() {
        // Soundness-critical: a SATISFIABLE formula is NEVER refuted, and a non-cardinality formula
        // falls through (false) instead of claiming a contradiction.
        for n in 1..=8 {
            assert!(!refute_clausal(&feasible_expr(n)), "feasible({n}) must NOT be refuted");
        }
        let lone = ProofExpr::Or(
            Box::new(ProofExpr::Atom("a".into())),
            Box::new(ProofExpr::Atom("b".into())),
        );
        assert!(!refute_clausal(&lone), "a lone satisfiable clause is not a cutting-plane contradiction");
    }

    #[test]
    fn pigeonhole_refutation_is_genuinely_unsat_small() {
        // Sanity: the PHP(3,2) constraints really are jointly unsatisfiable (the refutation isn't
        // refuting a satisfiable system). Brute-force over the 6 variables.
        let n = 3usize;
        let holes = n - 1;
        let var = |i: usize, h: usize| i * holes + h;
        let mut cs = Vec::new();
        for i in 0..n {
            cs.push(PbConstraint::clause(&(0..holes).map(|h| (var(i, h), true)).collect::<Vec<_>>()));
        }
        for h in 0..holes {
            cs.push(PbConstraint::at_most(&(0..n).map(|i| (var(i, h), true)).collect::<Vec<_>>(), 1));
        }
        let mut any = false;
        for_all_assignments(n * holes, |a| {
            if cs.iter().all(|c| c.is_satisfied(a)) {
                any = true;
            }
        });
        assert!(!any, "PHP(3,2) is UNSAT — no assignment satisfies all constraints");
    }

    #[test]
    fn coefficient_symmetry_is_detected_and_sound() {
        // 2·x0 + 2·x1 + 3·x2 ≥ 4: x0 and x1 share the coefficient 2, so they are interchangeable; x2 (coeff
        // 3) is alone. The symmetry is in the WEIGHTS, not the clause structure.
        let weighted = PbConstraint::new_weighted(&[(0, 2, true), (1, 2, true), (2, 3, true)], 4);
        let gens = coeff_symmetry_generators(3, &[weighted.clone()]);
        assert_eq!(gens.len(), 1, "one generator: the x0 ↔ x1 swap");
        assert_eq!(gens[0], vec![1, 0, 2], "x0 ↔ x1");
        // Every generator is a genuine symmetry…
        assert!(gens.iter().all(|g| is_pb_symmetry(&[weighted.clone()], g)));
        // …and it really preserves the solution set (brute force: swapping x0,x1 maps models to models).
        for_all_assignments(3, |a| {
            let swapped = |v: usize| a(if v == 0 { 1 } else if v == 1 { 0 } else { v });
            assert_eq!(
                weighted.is_satisfied(a),
                weighted.is_satisfied(&swapped),
                "the x0↔x1 swap preserves satisfaction"
            );
        });

        // Distinct coefficients ⇒ no coefficient symmetry.
        let distinct = PbConstraint::new_weighted(&[(0, 1, true), (1, 2, true), (2, 3, true)], 3);
        assert!(coeff_symmetry_generators(3, &[distinct]).is_empty(), "all-distinct coefficients: no symmetry");

        // Across a SYSTEM: a variable's profile is its coefficient in EVERY constraint. Here x0,x1 share
        // (2 in C₁, 1 in C₂) while x2 differs in C₂, so only x0,x1 remain interchangeable.
        let c1 = PbConstraint::new_weighted(&[(0, 2, true), (1, 2, true), (2, 2, true)], 3);
        let c2 = PbConstraint::new_weighted(&[(0, 1, true), (1, 1, true), (2, 5, true)], 2);
        let sysg = coeff_symmetry_generators(3, &[c1.clone(), c2.clone()]);
        assert_eq!(sysg, vec![vec![1, 0, 2]], "only x0 ↔ x1 survives both constraints");
        assert!(sysg.iter().all(|g| is_pb_symmetry(&[c1.clone(), c2.clone()], g)));
    }

    /// The live cardinality theory forces the right literals and conflicts at the right moment, with
    /// every returned clause currently unit (a forced literal) or all-false (a conflict).
    #[test]
    fn cardinality_theory_forces_then_conflicts_on_at_least_two() {
        use crate::cdcl::{Lit, Theory};
        let lits = [(0usize, true), (1, true), (2, true)];
        let mut th = CardinalityTheory::new(3, &[PbConstraint::at_least(&lits, 2)]);
        // x0 = false ⇒ with one of three false, the other two must be true to reach ≥ 2.
        let forced = th.propagate(&[Lit::new(0, false)]);
        assert_eq!(forced.len(), 2, "two literals forced; got {forced:?}");
        for c in &forced {
            let free: Vec<&Lit> = c.iter().filter(|l| l.var() != 0).collect();
            assert_eq!(free.len(), 1, "each reason is unit under {{x0=false}}: {c:?}");
            assert!(free[0].is_positive(), "the forced literal is x_i true: {c:?}");
        }
        // x0 = false ∧ x1 = false ⇒ at most one can be true < 2 ⇒ conflict over the two false literals.
        let conf = th.propagate(&[Lit::new(0, false), Lit::new(1, false)]);
        assert!(!conf.is_empty(), "must conflict; got {conf:?}");
        assert!(
            conf.iter().any(|c| !c.is_empty() && c.iter().all(|l| (l.var() == 0 || l.var() == 1) && l.is_positive())),
            "a conflict clause of the two now-false literals; got {conf:?}"
        );
    }

    /// DIAGNOSTIC: the cardinality theory ALONE, with no Boolean clauses, must refute an infeasible system
    /// (`≥ 3` and `≤ 1` of three variables) — i.e. the engine's pure-theory `solve_with` drives a theory
    /// conflict all the way to UNSAT.
    #[test]
    fn cardinality_theory_alone_refutes_an_infeasible_system() {
        use crate::cdcl::{Solver, SolveResult, Theory};
        let lits = [(0usize, true), (1, true), (2, true)];
        let card = vec![PbConstraint::at_least(&lits, 3), PbConstraint::at_most(&lits, 1)];
        let mut s = Solver::new(3);
        let mut t: Vec<Box<dyn Theory>> = vec![Box::new(CardinalityTheory::new(3, &card))];
        assert!(matches!(s.solve_with(&mut t), SolveResult::Unsat), "≥3 ∧ ≤1 of three is UNSAT");
    }

    /// DIAGNOSTIC: `IncXor` alone, no Boolean clauses, must refute an inconsistent linear system
    /// (`x0 ⊕ x1 = 0 ∧ x0 ⊕ x1 = 1`) — isolating whether pure-theory refutation works for the parity engine.
    #[test]
    fn incxor_alone_refutes_an_inconsistent_system() {
        use crate::cdcl::{Solver, SolveResult, Theory};
        use crate::xor_engine::IncXor;
        use crate::xorsat::XorEquation;
        let xor = vec![XorEquation::new(vec![0, 1], false), XorEquation::new(vec![0, 1], true)];
        let mut s = Solver::new(2);
        let mut t: Vec<Box<dyn Theory>> = vec![Box::new(IncXor::new(2, &xor))];
        assert!(matches!(s.solve_with(&mut t), SolveResult::Unsat), "x0⊕x1=0 ∧ x0⊕x1=1 is UNSAT");
    }

    /// **The headline fusion.** Parity (`x0 ⊕ x1 ⊕ x2 = 1`, odd) ∧ exactly-two-true is UNSAT (two is
    /// even), yet NEITHER theory alone refutes it — parity alone is SAT (one true), cardinality alone is
    /// SAT (any two). Only `IncXor` and `CardinalityTheory` reasoning *together* on the shared trail close
    /// it, with no Boolean clauses at all.
    #[test]
    fn fused_parity_and_cardinality_is_unsat_though_neither_alone_is() {
        use crate::cdcl::{Solver, SolveResult, Theory};
        use crate::xor_engine::XorEngine;
        use crate::xorsat::XorEquation;
        // The parity theory is the stateless `XorEngine` (the GF(2) correctness oracle), not the
        // incremental `IncXor`: `IncXor`'s trail-sync matches by variable but not value, so a
        // backtrack-then-flip leaves its matrix stale — harmless when a clausal XOR encoding backs it,
        // unsound in pure-theory fusion (see the docs on `CardinalityTheory`).
        let xor = vec![XorEquation::new(vec![0, 1, 2], true)];
        let lits = [(0usize, true), (1, true), (2, true)];
        let card = vec![PbConstraint::at_least(&lits, 2), PbConstraint::at_most(&lits, 2)];

        let mut s1 = Solver::new(3);
        let mut t1: Vec<Box<dyn Theory>> = vec![Box::new(XorEngine::new(3, &xor))];
        assert!(matches!(s1.solve_with(&mut t1), SolveResult::Sat(_)), "parity alone is SAT");

        let mut s2 = Solver::new(3);
        let mut t2: Vec<Box<dyn Theory>> = vec![Box::new(CardinalityTheory::new(3, &card))];
        assert!(matches!(s2.solve_with(&mut t2), SolveResult::Sat(_)), "exactly-two alone is SAT");

        let mut s = Solver::new(3);
        let mut fused: Vec<Box<dyn Theory>> =
            vec![Box::new(XorEngine::new(3, &xor)), Box::new(CardinalityTheory::new(3, &card))];
        assert!(matches!(s.solve_with(&mut fused), SolveResult::Unsat), "odd-parity ∧ exactly-two is UNSAT");
    }

    /// The consistent twin: even parity ∧ exactly-two-true IS satisfiable, and the fused solver returns a
    /// model that genuinely has even parity and exactly two true.
    #[test]
    fn fused_parity_and_cardinality_sat_model_is_valid() {
        use crate::cdcl::{Solver, SolveResult, Theory};
        use crate::xor_engine::XorEngine;
        use crate::xorsat::XorEquation;
        let xor = vec![XorEquation::new(vec![0, 1, 2], false)];
        let lits = [(0usize, true), (1, true), (2, true)];
        let card = vec![PbConstraint::at_least(&lits, 2), PbConstraint::at_most(&lits, 2)];
        let mut s = Solver::new(3);
        let mut fused: Vec<Box<dyn Theory>> =
            vec![Box::new(XorEngine::new(3, &xor)), Box::new(CardinalityTheory::new(3, &card))];
        match s.solve_with(&mut fused) {
            SolveResult::Sat(m) => {
                assert_eq!(m.iter().filter(|&&b| b).count(), 2, "exactly two true: {m:?}");
                assert!(!(m[0] ^ m[1] ^ m[2]), "even parity: {m:?}");
            }
            SolveResult::Unsat => panic!("even-parity ∧ exactly-two is SAT"),
        }
    }

    /// **Soundness to the point of absurdity.** Random instances combining Boolean clauses + XOR
    /// equations (`IncXor`) + cardinality constraints (`CardinalityTheory`): the fused `solve_with`
    /// verdict must match brute-force enumeration of all `2ⁿ` assignments exactly, and every reported SAT
    /// model must satisfy every clause, every parity, and every cardinality constraint.
    #[test]
    fn fused_solve_matches_brute_force() {
        use crate::cdcl::{Lit, Solver, SolveResult, Theory};
        use crate::xor_engine::XorEngine;
        use crate::xorsat::XorEquation;
        let mut st = 0xCA11_AB1Eu64;
        let mut rng = || {
            st ^= st << 13;
            st ^= st >> 7;
            st ^= st << 17;
            st
        };
        for _ in 0..300 {
            let n = 3 + (rng() % 3) as usize; // 3..=5 vars
            let mut xor_specs: Vec<(Vec<usize>, bool)> = Vec::new();
            for _ in 0..(1 + rng() % 2) {
                let vars: Vec<usize> = (0..n).filter(|_| rng() % 2 == 0).collect();
                if vars.len() >= 2 {
                    xor_specs.push((vars, rng() % 2 == 0));
                }
            }
            let mut pbs: Vec<PbConstraint> = Vec::new();
            for _ in 0..(1 + rng() % 2) {
                let lits: Vec<(usize, bool)> =
                    (0..n).filter_map(|v| (rng() % 2 == 0).then(|| (v, rng() % 2 == 0))).collect();
                if lits.len() < 2 {
                    continue;
                }
                let k = (rng() % (lits.len() as u64 + 1)) as i64;
                pbs.push(if rng() % 2 == 0 { PbConstraint::at_least(&lits, k) } else { PbConstraint::at_most(&lits, k) });
            }
            let mut clauses: Vec<Vec<Lit>> = Vec::new();
            for _ in 0..(rng() % 3) {
                let c: Vec<Lit> =
                    (0..n).filter_map(|v| (rng() % 2 == 0).then(|| Lit::new(v as u32, rng() % 2 == 0))).collect();
                if !c.is_empty() {
                    clauses.push(c);
                }
            }

            let xor_ok = |x: u64| xor_specs.iter().all(|(vars, rhs)| (vars.iter().filter(|&&v| (x >> v) & 1 == 1).count() % 2 == 1) == *rhs);
            let pb_ok = |x: u64| {
                pbs.iter().all(|pb| {
                    let sum: i64 = pb.terms().map(|(v, c, s)| if (((x >> v) & 1 == 1) == s) { c } else { 0 }).sum();
                    sum >= pb.degree()
                })
            };
            let cl_ok = |x: u64| clauses.iter().all(|c| c.iter().any(|l| ((x >> l.var()) & 1 == 1) == l.is_positive()));
            let brute_sat = (0u64..(1u64 << n)).any(|x| xor_ok(x) && pb_ok(x) && cl_ok(x));

            let xeqs: Vec<XorEquation> = xor_specs.iter().map(|(v, r)| XorEquation::new(v.clone(), *r)).collect();
            let mut s = Solver::new(n);
            for c in &clauses {
                s.add_clause(c.clone());
            }
            let mut theories: Vec<Box<dyn Theory>> =
                vec![Box::new(XorEngine::new(n, &xeqs)), Box::new(CardinalityTheory::new(n, &pbs))];
            let got = s.solve_with(&mut theories);
            assert_eq!(
                matches!(got, SolveResult::Sat(_)),
                brute_sat,
                "fused verdict must match brute force (n={n}, xor={xor_specs:?}, pbs={pbs:?}, clauses={clauses:?})"
            );
            if let SolveResult::Sat(m) = got {
                let x = (0..n).fold(0u64, |acc, v| acc | ((m[v] as u64) << v));
                assert!(xor_ok(x) && pb_ok(x) && cl_ok(x), "the fused model must satisfy every constraint: {m:?}");
            }
        }
    }
}
