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
            terms: self
                .terms
                .iter()
                .map(|(&v, &(coeff, s))| (v, (coeff * c, s)))
                .collect(),
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
            terms: self
                .terms
                .iter()
                .map(|(&v, &(c, s))| (v, (ceil(c), s)))
                .collect(),
            degree: ceil(self.degree),
        }
    }

    /// **Saturation** — clamp every coefficient to the degree (`min(aᵢ, degree)`). A single 0/1
    /// variable can contribute at most `degree` toward the bound, so the excess is redundant. Sound,
    /// and it keeps coefficients from blowing up across many additions.
    pub fn saturate(&self) -> PbConstraint {
        let d = self.degree;
        PbConstraint {
            terms: self
                .terms
                .iter()
                .map(|(&v, &(c, s))| (v, (c.min(d), s)))
                .collect(),
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
        PbConstraint {
            terms: terms.iter().map(|&(v, c, s)| (v, (c, s))).collect(),
            degree,
        }
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
    let profile =
        |v: usize| -> Vec<Option<(i64, bool)>> { constraints.iter().map(|c| c.term(v)).collect() };
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
    let excl_set: std::collections::HashSet<(usize, usize)> = exclusions
        .iter()
        .map(|&(a, b)| (a.min(b), a.max(b)))
        .collect();
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
            members[i + 1..]
                .iter()
                .all(|&b| excl_set.contains(&(a.min(b), a.max(b))))
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
        UnionFind {
            parent: (0..n).collect(),
        }
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
                        assert_eq!(
                            c, 1,
                            "CardinalityTheory requires unit coefficients (got {c} on var {v})"
                        );
                        (v, s)
                    })
                    .collect();
                (lits, pb.degree())
            })
            .collect();
        CardinalityTheory {
            num_vars,
            constraints,
        }
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
                out.push(
                    false_lits[..take]
                        .iter()
                        .map(|&(v, s)| Lit::new(v as u32, s))
                        .collect(),
                );
            } else if fc == max_false && !unassigned.is_empty() {
                // Every unassigned literal is forced true: reason = {the false literals} ∨ ℓ.
                let base: Vec<Lit> = false_lits
                    .iter()
                    .map(|&(v, s)| Lit::new(v as u32, s))
                    .collect();
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
#[path = "../../logicaffeine_tests/unit/logicaffeine_proof/pseudo_boolean/tests.rs"]
mod tests;
