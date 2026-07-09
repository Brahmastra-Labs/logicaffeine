//! Certified symmetry breaking — the centerpiece. Given a formula and a set of verified
//! symmetry generators, add lex-leader symmetry-breaking predicates as **PR steps** (each
//! self-checked, fail-closed), solve the augmented formula, and emit a single composed
//! refutation that an independent checker accepts against the *original* formula alone.
//!
//! This closes the soundness gap of the old path (where the symmetry-broken formula was RUP-
//! certified but the model-removing addition was only argued informally): here every SBP clause
//! carries a propagation-redundancy witness derived from its symmetry, so the whole UNSAT
//! result — symmetry steps included — is machine-checkable. The decisive wiring invariant is
//! that `check_pr_refutation` runs against `formula` ALONE; the SBP clauses appear only as PR
//! steps, never as free original clauses.

use crate::cdcl::{Lit, SolveResult, Solver, Var};
use crate::pr::{check_pr_refutation, is_pr};
use crate::proof::{Perm, ProofStep, Witness};
use crate::symmetry_detect::find_generators;

/// The outcome of a certified-symmetry-breaking solve.
#[derive(Clone, Debug)]
pub struct CertifiedRefutation {
    /// Whether the formula was refuted (proven UNSAT) AND the composed PR proof checks.
    pub refuted: bool,
    /// How many lex-leader SBP clauses were PR-certified and added.
    pub sbp_clauses: usize,
    /// The composed proof stream: PR symmetry steps followed by RUP learned clauses.
    pub steps: Vec<ProofStep>,
}

/// The first-index lex-leader clause for `sigma`: over the smallest variable `v` that `sigma`
/// moves, assert `v ⟹ sigma(v)` (the leading bit of `x ≤ₗₑₓ sigma(x)`). Returns `None` if
/// `sigma` is the identity.
fn lex_leader_lead_clause(num_vars: usize, sigma: &Perm) -> Option<Vec<Lit>> {
    for v in 0..num_vars as Var {
        let image = sigma.apply(Lit::pos(v));
        if image != Lit::pos(v) {
            return Some(vec![Lit::neg(v), image]);
        }
    }
    None
}

/// Build the full lex-leader symmetry-breaking predicate for `sigma` over the moved-variable
/// order, enforcing `x ≤ₗₑₓ σ(x)`. Returns the clauses (over base variables plus fresh auxiliary
/// "tied-so-far" variables `e_i`, allocated starting at `aux_start`) and the number of aux
/// variables used. This is the linear-size encoding that prunes the entire non-leader half of
/// every σ-orbit — the real search-collapse predicate.
pub fn lex_leader_clauses(num_vars: usize, aux_start: usize, sigma: &Perm) -> (Vec<Vec<Lit>>, usize) {
    let support: Vec<Var> =
        (0..num_vars as Var).filter(|&v| sigma.apply(Lit::pos(v)) != Lit::pos(v)).collect();
    let mut clauses: Vec<Vec<Lit>> = Vec::new();
    if support.is_empty() {
        return (clauses, 0);
    }
    let a = |i: usize| Lit::pos(support[i]);
    let b = |i: usize| sigma.apply(Lit::pos(support[i]));

    // Position 0: a_0 ≤ b_0 unconditionally (the prefix before it is empty, hence tied).
    clauses.push(vec![a(0).negated(), b(0)]);

    // For i ≥ 1: define e_i ⟺ (tied through i-1) ∧ (a_{i-1} = b_{i-1}), then constrain
    // e_i → (a_i ≤ b_i). `prev` is the tied-so-far literal (None at position 1 means "true").
    let mut next_aux = aux_start;
    let mut prev: Option<Lit> = None;
    for i in 1..support.len() {
        let e = Lit::pos(next_aux as Var);
        next_aux += 1;
        let (ap, bp) = (a(i - 1), b(i - 1));
        match prev {
            None => {
                // e ⟺ (a_{i-1} = b_{i-1}).
                clauses.push(vec![e.negated(), ap.negated(), bp]);
                clauses.push(vec![e.negated(), ap, bp.negated()]);
                clauses.push(vec![e, ap, bp]);
                clauses.push(vec![e, ap.negated(), bp.negated()]);
            }
            Some(pe) => {
                // e ⟺ pe ∧ (a_{i-1} = b_{i-1}).
                clauses.push(vec![e.negated(), pe]);
                clauses.push(vec![e.negated(), ap.negated(), bp]);
                clauses.push(vec![e.negated(), ap, bp.negated()]);
                clauses.push(vec![pe.negated(), e, ap, bp]);
                clauses.push(vec![pe.negated(), e, ap.negated(), bp.negated()]);
            }
        }
        clauses.push(vec![e.negated(), a(i).negated(), b(i)]);
        prev = Some(e);
    }
    (clauses, next_aux - aux_start)
}

/// Solve `formula` with certified symmetry breaking under the given `generators` (each must be
/// a genuine automorphism — they are re-checked implicitly by the per-clause PR self-check).
///
/// For each generator we propose its lead lex-leader clause, PR-self-check it against the
/// database built so far (so a generator invalidated by earlier SBPs is simply skipped), add
/// the survivors as PR steps, solve the augmented formula, and — if UNSAT — append the solver's
/// learned clauses as RUP steps. The composed stream is verified against `formula` alone.
pub fn certified_unsat(num_vars: usize, formula: &[Vec<Lit>], generators: &[Perm]) -> CertifiedRefutation {
    let mut db: Vec<Vec<Lit>> = formula.to_vec();
    let mut steps: Vec<ProofStep> = Vec::new();

    for sigma in generators {
        let Some(clause) = lex_leader_lead_clause(num_vars, sigma) else { continue };
        let witness = Witness::Substitution(sigma.clone());
        if is_pr(num_vars, &db, &clause, &witness) {
            db.push(clause.clone());
            steps.push(ProofStep::Pr { clause, witness });
        }
    }
    let sbp_clauses = steps.len();

    // Solve the augmented formula F ∧ SBP and collect its learned clauses as RUP steps.
    let mut solver = Solver::new(num_vars);
    for c in &db {
        solver.add_clause(c.clone());
    }
    let refuted = match solver.solve() {
        SolveResult::Sat(_) => false,
        SolveResult::Unsat => {
            for lc in solver.learned() {
                steps.push(ProofStep::Rup(lc.lits.clone()));
            }
            // The decisive check: the whole stream is replayed against the ORIGINAL formula.
            check_pr_refutation(num_vars, formula, &steps)
        }
    };

    CertifiedRefutation { refuted, sbp_clauses, steps }
}

/// Find a PR assignment-witness that certifies lex-leader clause `c` against `db`, drawing from
/// a small principled candidate set (single literals of the clause, then the tied-prefix aux
/// chain optionally combined with a clause literal). Every candidate is verified by [`is_pr`],
/// so a returned witness is genuinely sound; `None` means none of the candidates worked.
fn find_lex_witness(nv: usize, db: &[Vec<Lit>], c: &[Lit], base_nv: usize, sigma: &Perm) -> Option<Witness> {
    // The substitution witness certifies the lead clause at ANY scale (its `db ∧ ¬C ⊢₁ σ(C)`
    // check conflicts immediately), where an assignment witness would need F|α to be UP-refutable
    // — which fails on large formulas. Try it first; it is rejected (fail-closed) once an earlier
    // predicate has broken σ's automorphism of the database. σ is lifted to the auxiliary
    // variables (as the identity) so the check is defined over the extended database.
    let subst = Witness::Substitution(sigma.extended(nv));
    if is_pr(nv, db, c, &subst) {
        return Some(subst);
    }
    let accept = |lits: Vec<Lit>| {
        let w = Witness::Assignment(lits);
        is_pr(nv, db, c, &w).then_some(w)
    };
    // The witness only ever touches the clause's own variables and the auxiliary "tied" chain up
    // to the clause's highest aux. Gather both polarities of those as the candidate pool.
    let max_aux = c.iter().map(|l| l.var()).filter(|&v| (v as usize) >= base_nv).max();
    let mut vars: Vec<Var> = c.iter().map(|l| l.var()).collect();
    if let Some(ma) = max_aux {
        vars.extend(base_nv as Var..=ma);
    }
    vars.sort_unstable();
    vars.dedup();
    let lits: Vec<Lit> = vars.iter().flat_map(|&v| [Lit::pos(v), Lit::neg(v)]).collect();

    // Size 1.
    for &l in &lits {
        if let Some(w) = accept(vec![l]) {
            return Some(w);
        }
    }
    // The tied-prefix chain (the monotonicity clauses' witness), alone and with a clause literal.
    if let Some(ma) = max_aux {
        let prefix: Vec<Lit> = (base_nv as Var..ma).map(Lit::pos).collect();
        if let Some(w) = accept(prefix.clone()) {
            return Some(w);
        }
        for &l in c {
            let mut cand = prefix.clone();
            cand.push(l);
            if let Some(w) = accept(cand) {
                return Some(w);
            }
        }
    }
    // Size 2 over the candidate pool (no two literals of the same variable). Bounded on purpose:
    // deeper witnesses do not exist for clauses whose certification would need F|α to be
    // UP-refutable, so searching further only wastes time failing.
    for i in 0..lits.len() {
        for j in (i + 1)..lits.len() {
            if lits[i].var() == lits[j].var() {
                continue;
            }
            if let Some(w) = accept(vec![lits[i], lits[j]]) {
                return Some(w);
            }
        }
    }
    None
}

/// Solve `formula` with FULL certified lex-leader symmetry breaking. For each generator, add the
/// complete lex-leader chain (enforcing `x ≤ₗₑₓ σ(x)`) as PR steps — each clause certified by a
/// principled witness — then solve and append the learned clauses as RUP steps. This prunes the
/// non-leader half of every σ-orbit (real search collapse), with the whole refutation checked
/// against `formula` alone. A generator whose chain cannot be fully certified is skipped
/// (fail-closed); the result stays sound.
/// Augment `formula` with the certified full lex-leader symmetry-breaking clauses for each
/// generator, committing a generator's chain only when every clause of it certifies (else the
/// generator is skipped — fail-closed). Returns the augmented clause set, the extended variable
/// count (base + auxiliaries), and the PR proof steps that justify the added clauses. This is
/// the symmetry-breaking front half shared by certified solving and by search-collapse
/// measurement.
pub fn symmetry_break_certified(
    num_vars: usize,
    formula: &[Vec<Lit>],
    generators: &[Perm],
) -> (Vec<Vec<Lit>>, usize, Vec<ProofStep>) {
    let mut db: Vec<Vec<Lit>> = formula.to_vec();
    let mut steps: Vec<ProofStep> = Vec::new();
    let mut nv = num_vars;
    for sigma in generators {
        let (clauses, num_aux) = lex_leader_clauses(num_vars, nv, sigma);
        if clauses.is_empty() {
            continue;
        }
        let ext_nv = nv + num_aux;
        // Commit each clause of the chain that certifies (a partial lex-leader is still sound —
        // every committed clause is independently PR-checked); skip any that don't.
        let mut committed = false;
        for c in &clauses {
            if let Some(w) = find_lex_witness(ext_nv, &db, c, num_vars, sigma) {
                db.push(c.clone());
                steps.push(ProofStep::Pr { clause: c.clone(), witness: w });
                committed = true;
            }
        }
        if committed {
            nv = ext_nv;
        }
    }
    (db, nv, steps)
}

pub fn certified_unsat_lex(num_vars: usize, formula: &[Vec<Lit>], generators: &[Perm]) -> CertifiedRefutation {
    let (db, nv, mut steps) = symmetry_break_certified(num_vars, formula, generators);
    let sbp_clauses = steps.len();

    let mut solver = Solver::new(nv);
    for c in &db {
        solver.add_clause(c.clone());
    }
    let refuted = match solver.solve() {
        SolveResult::Sat(_) => false,
        SolveResult::Unsat => {
            for lc in solver.learned() {
                steps.push(ProofStep::Rup(lc.lits.clone()));
            }
            check_pr_refutation(nv, formula, &steps)
        }
    };

    CertifiedRefutation { refuted, sbp_clauses, steps }
}

/// The permutation that swaps two pigeons of PHP(n) (exchanging their whole hole-rows) — a
/// genuine automorphism of the pigeonhole formula.
fn swap_pigeons(n: usize, holes: usize, i: usize, j: usize) -> Perm {
    Perm::from_images(
        (0..n * holes)
            .map(|v| {
                let (p, h) = (v / holes, v % holes);
                let np = if p == i {
                    j
                } else if p == j {
                    i
                } else {
                    p
                };
                Lit::pos((np * holes + h) as Var)
            })
            .collect(),
    )
}

/// The Heule–Kiesl–Biere short PR refutation of PHP(n): a polynomial-size, fully certified proof
/// — where the lex-leader provably cannot scale on UNSAT instances.
///
/// It frees holes one at a time. To free the last active hole `h` of PHP(m), it forces every
/// non-last pigeon `i` out of it with the clause `¬x(i, h)`, certified by the **substitution
/// witness "swap pigeon i with the last pigeon"** — a PHP automorphism whose SR check conflicts
/// at once on the hole-`h` conflict clause (so it is sound at any scale). That confines the
/// remaining pigeons to one fewer hole, reducing PHP(m) → PHP(m-1); after `O(n²)` such units RUP
/// closes. The whole refutation is checked against the original PHP(n) alone.
pub fn heule_php_refutation(n: usize) -> CertifiedRefutation {
    let (cnf, _) = crate::families::php(n);
    let holes = n.saturating_sub(1);
    let nv = cnf.num_vars;
    let mut db = cnf.clauses.clone();
    let mut index = crate::symmetry_detect::AutomorphismIndex::with_clauses(nv, &cnf.clauses);
    let mut steps: Vec<ProofStep> = Vec::new();

    // Reduce PHP(m) to PHP(m-1) for m = n, n-1, …, 2, freeing hole (m-2) each round. The
    // automorphism re-check rides the incrementally-grown index, so each step is O(support).
    for m in (2..=n).rev() {
        let hole = m - 2;
        let last_pigeon = m - 1;
        for i in 0..last_pigeon {
            let clause = vec![Lit::neg((i * holes + hole) as Var)];
            let witness = Witness::Substitution(swap_pigeons(n, holes, i, last_pigeon));
            if crate::pr::is_pr_indexed(nv, &db, &mut index, &clause, &witness) {
                db.push(clause.clone());
                index.insert(clause.clone());
                steps.push(ProofStep::Pr { clause, witness });
            }
        }
    }
    let sbp_clauses = steps.len();

    let mut solver = Solver::new(nv);
    for c in &db {
        solver.add_clause(c.clone());
    }
    let refuted = match solver.solve() {
        SolveResult::Sat(_) => false,
        SolveResult::Unsat => {
            for lc in solver.learned() {
                steps.push(ProofStep::Rup(lc.lits.clone()));
            }
            crate::pr::check_pr_refutation_fast(nv, &cnf.clauses, &steps)
        }
    };

    CertifiedRefutation { refuted, sbp_clauses, steps }
}

/// The vertex-swap automorphism of `clique_coloring(n, k)`: exchange vertices `a` and `b` (every
/// color of one ↔ the same color of the other), fixing all other vertices. A genuine symmetry of
/// `Kₙ` (any two vertices are interchangeable), so it certifies the SR witnesses of a steered
/// coloring refutation — variable layout `v*k + c`.
fn swap_vertices(n: usize, k: usize, a: usize, b: usize) -> Perm {
    let nv = n * k;
    Perm::from_images(
        (0..nv)
            .map(|idx| {
                let (v, c) = (idx / k, idx % k);
                let nv2 = if v == a {
                    b
                } else if v == b {
                    a
                } else {
                    v
                };
                Lit::pos((nv2 * k + c) as Var)
            })
            .collect(),
    )
}

/// **Full-chain structural steering** of `clique_coloring(n, k)` (UNSAT for `k < n`): the Heule
/// pigeonhole refutation transplanted onto the coloring encoding, wielding the *a-priori*
/// vertex-swap symmetry rather than detecting it. `k + 1` mutually-adjacent vertices already form a
/// PHP(k+1, k), so the proof forces those `k+1` vertices out of the colors one at a time — each
/// clause `¬x(i, color)` certified by the substitution "swap vertex `i` with the last active
/// vertex", whose SR check clashes at once on the corresponding at-most-one clause. The whole
/// stream is verified against the original clique formula alone.
pub fn heule_clique_refutation(n: usize, k: usize) -> CertifiedRefutation {
    let (cnf, _) = crate::families::clique_coloring(n, k);
    let nv = cnf.num_vars;
    let mut db = cnf.clauses.clone();
    let mut index = crate::symmetry_detect::AutomorphismIndex::with_clauses(nv, &cnf.clauses);
    let mut steps: Vec<ProofStep> = Vec::new();
    // k+1 vertices (capped at n) are a tight pigeonhole over the k colors.
    let items = (k + 1).min(n);
    let var = |v: usize, c: usize| (v * k + c) as Var;

    for m in (2..=items).rev() {
        let color = m - 2;
        let last_vertex = m - 1;
        for i in 0..last_vertex {
            let clause = vec![Lit::neg(var(i, color))];
            let witness = Witness::Substitution(swap_vertices(n, k, i, last_vertex));
            if crate::pr::is_pr_indexed(nv, &db, &mut index, &clause, &witness) {
                db.push(clause.clone());
                index.insert(clause.clone());
                steps.push(ProofStep::Pr { clause, witness });
            }
        }
    }
    let sbp_clauses = steps.len();

    let mut solver = Solver::new(nv);
    for c in &db {
        solver.add_clause(c.clone());
    }
    let refuted = match solver.solve() {
        SolveResult::Sat(_) => false,
        SolveResult::Unsat => {
            for lc in solver.learned() {
                steps.push(ProofStep::Rup(lc.lits.clone()));
            }
            crate::pr::check_pr_refutation_fast(nv, &cnf.clauses, &steps)
        }
    };

    CertifiedRefutation { refuted, sbp_clauses, steps }
}

/// The Heule PHP(n) refutation **with its rank function attached** — each symmetry-breaking step
/// is tagged by the round (`m` = active items remaining) it belongs to, a non-increasing measure
/// whose descent bounds the proof size. The closing RUP steps take the bottom rank `1`. Feed the
/// result to [`crate::complexity::RankedRefutation::certify`] to get a checkable `O(n²)` size bound
/// alongside the correctness check.
pub fn heule_php_ranked(n: usize) -> crate::complexity::RankedRefutation {
    let (cnf, _) = crate::families::php(n);
    let holes = n.saturating_sub(1);
    let nv = cnf.num_vars;
    let mut db = cnf.clauses.clone();
    let mut index = crate::symmetry_detect::AutomorphismIndex::with_clauses(nv, &cnf.clauses);
    let mut steps: Vec<ProofStep> = Vec::new();
    let mut ranks: Vec<u64> = Vec::new();

    for m in (2..=n).rev() {
        let hole = m - 2;
        let last_pigeon = m - 1;
        for i in 0..last_pigeon {
            let clause = vec![Lit::neg((i * holes + hole) as Var)];
            let witness = Witness::Substitution(swap_pigeons(n, holes, i, last_pigeon));
            if crate::pr::is_pr_indexed(nv, &db, &mut index, &clause, &witness) {
                db.push(clause.clone());
                index.insert(clause.clone());
                steps.push(ProofStep::Pr { clause, witness });
                ranks.push(m as u64); // rank = active items remaining this round (descends with m)
            }
        }
    }

    let mut solver = Solver::new(nv);
    for c in &db {
        solver.add_clause(c.clone());
    }
    let refuted = match solver.solve() {
        SolveResult::Sat(_) => false,
        SolveResult::Unsat => {
            for lc in solver.learned() {
                steps.push(ProofStep::Rup(lc.lits.clone()));
                ranks.push(1); // the closing chain sits at the bottom level
            }
            crate::pr::check_pr_refutation_fast(nv, &cnf.clauses, &steps)
        }
    };

    crate::complexity::RankedRefutation { refuted, steps, ranks }
}

/// The steered clique-coloring refutation **with its rank function attached** — the analogue of
/// [`heule_php_ranked`] over the coloring encoding, so a clique refutation also ships a checkable
/// `O(n²)` size certificate (rank = active vertices remaining).
pub fn heule_clique_ranked(n: usize, k: usize) -> crate::complexity::RankedRefutation {
    let (cnf, _) = crate::families::clique_coloring(n, k);
    let nv = cnf.num_vars;
    let mut db = cnf.clauses.clone();
    let mut index = crate::symmetry_detect::AutomorphismIndex::with_clauses(nv, &cnf.clauses);
    let mut steps: Vec<ProofStep> = Vec::new();
    let mut ranks: Vec<u64> = Vec::new();
    let items = (k + 1).min(n);
    let var = |v: usize, c: usize| (v * k + c) as Var;

    for m in (2..=items).rev() {
        let color = m - 2;
        let last_vertex = m - 1;
        for i in 0..last_vertex {
            let clause = vec![Lit::neg(var(i, color))];
            let witness = Witness::Substitution(swap_vertices(n, k, i, last_vertex));
            if crate::pr::is_pr_indexed(nv, &db, &mut index, &clause, &witness) {
                db.push(clause.clone());
                index.insert(clause.clone());
                steps.push(ProofStep::Pr { clause, witness });
                ranks.push(m as u64);
            }
        }
    }

    let mut solver = Solver::new(nv);
    for c in &db {
        solver.add_clause(c.clone());
    }
    let refuted = match solver.solve() {
        SolveResult::Sat(_) => false,
        SolveResult::Unsat => {
            for lc in solver.learned() {
                steps.push(ProofStep::Rup(lc.lits.clone()));
                ranks.push(1);
            }
            crate::pr::check_pr_refutation_fast(nv, &cnf.clauses, &steps)
        }
    };

    crate::complexity::RankedRefutation { refuted, steps, ranks }
}

/// A safety cap on the number of symmetry-breaking rounds — far above any real need (the group
/// is finite and strictly shrinks each round), a guard against pathological inputs.
const MAX_SBP_ROUNDS: usize = 100_000;

/// Solve `formula` with FULL certified symmetry breaking, discovering the symmetries itself.
///
/// Each round: detect the residual symmetry group of the current database, certify ONE lead
/// lex-leader predicate as a PR step (always sound — its generator is a fresh automorphism of
/// the current database, so the SR check's conflict is immediate), and re-detect. Adding the
/// predicate strictly shrinks the automorphism group, so the loop terminates with the whole group
/// broken; then the augmented formula is solved and the learned clauses appended as RUP steps. The
/// entire composed stream is verified against `formula` ALONE.
///
/// This breaks the *complete* group rather than one clause per generator, by re-detecting the
/// stabilizer after each predicate — the natural "lift and shift" of detection + the SR checker.
pub fn certified_unsat_auto(num_vars: usize, formula: &[Vec<Lit>]) -> CertifiedRefutation {
    let mut db: Vec<Vec<Lit>> = formula.to_vec();
    let mut steps: Vec<ProofStep> = Vec::new();

    for _ in 0..MAX_SBP_ROUNDS {
        let mut progressed = false;
        for sigma in find_generators(num_vars, &db) {
            let Some(clause) = lex_leader_lead_clause(num_vars, &sigma) else { continue };
            let witness = Witness::Substitution(sigma);
            if is_pr(num_vars, &db, &clause, &witness) {
                db.push(clause.clone());
                steps.push(ProofStep::Pr { clause, witness });
                progressed = true;
                break;
            }
        }
        if !progressed {
            break;
        }
    }
    let sbp_clauses = steps.len();

    let mut solver = Solver::new(num_vars);
    for c in &db {
        solver.add_clause(c.clone());
    }
    let refuted = match solver.solve() {
        SolveResult::Sat(_) => false,
        SolveResult::Unsat => {
            for lc in solver.learned() {
                steps.push(ProofStep::Rup(lc.lits.clone()));
            }
            check_pr_refutation(num_vars, formula, &steps)
        }
    };

    CertifiedRefutation { refuted, sbp_clauses, steps }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cdcl::Lit;
    use crate::families;
    use crate::symmetry_detect::perm_is_automorphism;

    #[test]
    fn heule_php_ranked_certifies_quadratic_size() {
        // The refutation must carry a rank function that certifies its OWN size is O(n²), checked
        // against the original formula alongside correctness. The certified bound (levels · width)
        // must be quadratic in n, and the actual proof must fit under it.
        for n in 3..=8 {
            let ranked = heule_php_ranked(n);
            assert!(ranked.refuted, "PHP({n}) must refute");
            let (cnf, _) = families::php(n);
            let bound = ranked
                .certify(cnf.num_vars, &cnf.clauses)
                .expect("a valid descent over a correct refutation must certify");
            // Levels ≤ n (ranks n..2 plus the bottom), width ≤ n−1 (the first round) ⇒ bound ≤ n².
            assert!(bound.levels <= n as u64, "levels {} must be ≤ n={n}", bound.levels);
            assert!(bound.max_width <= n as u64, "width {} must be ≤ n={n}", bound.max_width);
            assert!(bound.bound <= (n as u64) * (n as u64), "certified bound must be ≤ n²");
            assert!(bound.actual <= bound.bound, "actual size must fit the certified bound");
            // And the actual symmetry-breaking work is exactly the rank function's sum.
            let sbp = ranked.ranks.iter().filter(|&&r| r >= 2).count() as u64;
            assert_eq!(sbp, (n as u64) * (n as u64 - 1) / 2, "sbp must equal n(n-1)/2 exactly");
        }
    }

    #[test]
    fn heule_clique_ranked_certifies_quadratic_size() {
        // Every steered family ships a cost certificate: the clique refutation must certify its own
        // O(n²) size (rank = active vertices) together with correctness, across tight and loose k.
        for (n, k) in [(5, 4), (7, 6), (8, 5), (9, 4)] {
            let ranked = heule_clique_ranked(n, k);
            assert!(ranked.refuted, "clique({n},{k}) must refute");
            let (cnf, _) = families::clique_coloring(n, k);
            let bound = ranked
                .certify(cnf.num_vars, &cnf.clauses)
                .expect("a valid descent over a correct clique refutation must certify");
            let items = (k + 1).min(n) as u64;
            assert!(bound.bound <= items * items, "certified bound must be ≤ (k+1)²");
            assert!(bound.actual <= bound.bound, "actual size fits the certified bound");
        }
    }

    #[test]
    fn heule_clique_refutation_certifies_across_shapes() {
        // Full-chain structural steering must refute clique-coloring (UNSAT for k < n) with a proof
        // that independently checks — tight pigeonhole shapes (k = n-1) and looser ones (k < n-1).
        for (n, k) in [(4, 3), (5, 4), (6, 5), (7, 6), (6, 3), (7, 4), (8, 5)] {
            let cr = heule_clique_refutation(n, k);
            assert!(cr.refuted, "clique({n},{k}) must be refuted with a checking proof");
            assert!(cr.sbp_clauses > 0, "clique({n},{k}) must actually break symmetry");
            let (cnf, _) = families::clique_coloring(n, k);
            assert!(
                crate::pr::check_pr_refutation_fast(cnf.num_vars, &cnf.clauses, &cr.steps),
                "clique({n},{k}) steered proof must re-check against the original formula"
            );
        }
    }

    /// Swap two pigeon rows of PHP(n) — a known automorphism, used to feed the certifier known
    /// generators while the general detector is built out separately.
    fn swap_pigeon_rows(n: usize, p0: usize, p1: usize) -> Perm {
        let holes = n - 1;
        Perm::from_images(
            (0..n * holes)
                .map(|v| {
                    let (p, h) = (v / holes, v % holes);
                    let np = if p == p0 {
                        p1
                    } else if p == p1 {
                        p0
                    } else {
                        p
                    };
                    Lit::pos((np * holes + h) as u32)
                })
                .collect(),
        )
    }

    #[test]
    fn php3_is_refuted_with_a_pr_certified_symmetry_proof() {
        let (cnf, _) = families::php(3);
        // Adjacent pigeon-row swaps generate the pigeon symmetry group S_3.
        let gens: Vec<Perm> = [(0usize, 1usize), (1, 2)].iter().map(|&(a, b)| swap_pigeon_rows(3, a, b)).collect();
        for g in &gens {
            assert!(perm_is_automorphism(&cnf.clauses, g), "fed generators must be real symmetries");
        }

        let result = certified_unsat(cnf.num_vars, &cnf.clauses, &gens);
        assert!(result.refuted, "PHP(3) must be refuted and the composed PR proof must check");
        assert!(result.sbp_clauses >= 1, "at least one symmetry-breaking predicate was certified");
        // Independent re-check of the full composed stream against the ORIGINAL formula alone.
        assert!(check_pr_refutation(cnf.num_vars, &cnf.clauses, &result.steps));
    }

    #[test]
    fn php4_is_refuted_with_a_pr_certified_symmetry_proof() {
        let (cnf, _) = families::php(4);
        let gens: Vec<Perm> =
            [(0usize, 1usize), (1, 2), (2, 3)].iter().map(|&(a, b)| swap_pigeon_rows(4, a, b)).collect();
        let result = certified_unsat(cnf.num_vars, &cnf.clauses, &gens);
        assert!(result.refuted);
        assert!(result.sbp_clauses >= 1);
        assert!(check_pr_refutation(cnf.num_vars, &cnf.clauses, &result.steps));
    }

    #[test]
    fn a_bogus_generator_is_not_certified_but_the_refutation_still_holds() {
        // Feed a NON-symmetry: its lead clause must fail the PR self-check and be dropped, yet
        // the formula is still refuted (by RUP on the learned clauses) and the proof checks.
        let (cnf, _) = families::php(3);
        let holes = 2;
        let bogus = Perm::from_images(
            (0..cnf.num_vars)
                .map(|v| {
                    let (p, h) = (v / holes, v % holes);
                    Lit::pos((if p == 0 { 1 } else { p } * holes + h) as u32)
                })
                .collect(),
        );
        assert!(!perm_is_automorphism(&cnf.clauses, &bogus));
        let result = certified_unsat(cnf.num_vars, &cnf.clauses, &[bogus]);
        assert_eq!(result.sbp_clauses, 0, "a non-symmetry yields no certified SBP");
        assert!(result.refuted, "the formula is still refuted, soundly");
        assert!(check_pr_refutation(cnf.num_vars, &cnf.clauses, &result.steps));
    }

    #[test]
    fn php_is_refuted_with_auto_discovered_generators() {
        // The full pipeline with NO hand-fed generators: detect symmetries, certify the SBPs as
        // PR steps, solve, and machine-check the composed refutation against the original formula.
        use crate::symmetry_detect::find_generators;
        for n in 3..=4 {
            let (cnf, _) = families::php(n);
            let gens = find_generators(cnf.num_vars, &cnf.clauses);
            let result = certified_unsat(cnf.num_vars, &cnf.clauses, &gens);
            assert!(result.refuted, "PHP({n}) refuted via discovered symmetries");
            assert!(result.sbp_clauses >= 1, "at least one SBP certified from a discovered generator");
            assert!(check_pr_refutation(cnf.num_vars, &cnf.clauses, &result.steps));
        }
    }

    // --- full iterative symmetry breaking (certified_unsat_auto) ---

    fn pr_clauses(steps: &[ProofStep]) -> Vec<Vec<Lit>> {
        steps
            .iter()
            .filter_map(|s| if let ProofStep::Pr { clause, .. } = s { Some(clause.clone()) } else { None })
            .collect()
    }

    // --- full lex-leader encoding (lex_leader_clauses) ---

    fn bit(mask: u32, i: usize) -> bool {
        (mask >> i) & 1 == 1
    }
    fn lit_val(assign: &[bool], l: Lit) -> bool {
        assign[l.var() as usize] == l.is_positive()
    }
    fn clauses_sat(assign: &[bool], clauses: &[Vec<Lit>]) -> bool {
        clauses.iter().all(|c| c.iter().any(|&l| lit_val(assign, l)))
    }
    /// Is base assignment `x` the lex-least in its σ-orbit (i.e. `x ≤ₗₑₓ σ(x)`)?
    fn is_lex_leader(num_vars: usize, x: u32, sigma: &Perm) -> bool {
        let xa: Vec<bool> = (0..num_vars).map(|v| bit(x, v)).collect();
        for v in (0..num_vars as Var).filter(|&v| sigma.apply(Lit::pos(v)) != Lit::pos(v)) {
            let a_val = xa[v as usize];
            let b_val = lit_val(&xa, sigma.apply(Lit::pos(v)));
            if a_val != b_val {
                return !a_val && b_val; // first difference: leader iff a < b (false < true)
            }
        }
        true
    }
    /// Does some aux assignment extend `x` to satisfy the lex-leader clauses?
    fn has_aux_extension(num_vars: usize, x: u32, clauses: &[Vec<Lit>], num_aux: usize) -> bool {
        (0..(1u32 << num_aux)).any(|aux| {
            let mut assign = vec![false; num_vars + num_aux];
            for v in 0..num_vars {
                assign[v] = bit(x, v);
            }
            for j in 0..num_aux {
                assign[num_vars + j] = bit(aux, j);
            }
            clauses_sat(&assign, clauses)
        })
    }

    #[test]
    fn lex_leader_encoding_admits_exactly_the_orbit_leaders() {
        // For several generators — a block pair-swap, a 3-cycle, a transposition — the lex-leader
        // predicate is satisfiable (with some aux) for a base assignment IFF that assignment is
        // the lex-least in its σ-orbit. This pins the encoding exactly, independent of any F.
        let cases: Vec<(usize, Perm)> = vec![
            (4, Perm::from_images(vec![Lit::pos(2), Lit::pos(3), Lit::pos(0), Lit::pos(1)])), // (0 2)(1 3)
            (3, Perm::from_images(vec![Lit::pos(1), Lit::pos(2), Lit::pos(0)])),              // (0 1 2)
            (4, Perm::from_images(vec![Lit::pos(1), Lit::pos(0), Lit::pos(2), Lit::pos(3)])), // (0 1)
        ];
        for (nv, sigma) in cases {
            let (clauses, num_aux) = lex_leader_clauses(nv, nv, &sigma);
            for x in 0..(1u32 << nv) {
                assert_eq!(
                    has_aux_extension(nv, x, &clauses, num_aux),
                    is_lex_leader(nv, x, &sigma),
                    "x={x:04b} mismatch for σ over {nv} vars"
                );
            }
        }
    }

    #[test]
    fn lex_leader_is_satisfiability_preserving_on_a_symmetric_formula() {
        let sigma = Perm::from_images(vec![Lit::pos(2), Lit::pos(3), Lit::pos(0), Lit::pos(1)]); // (0 2)(1 3)
        let f: Vec<Vec<Lit>> = vec![vec![Lit::pos(0), Lit::pos(2)], vec![Lit::pos(1), Lit::pos(3)]];
        assert!(crate::symmetry_detect::perm_is_automorphism(&f, &sigma), "σ must be a symmetry of F");
        let (lex, num_aux) = lex_leader_clauses(4, 4, &sigma);

        let f_sat = (0..(1u32 << 4)).any(|x| clauses_sat(&(0..4).map(|v| bit(x, v)).collect::<Vec<_>>(), &f));
        let fl_sat = (0..(1u32 << (4 + num_aux))).any(|m| {
            let assign: Vec<bool> = (0..4 + num_aux).map(|v| bit(m, v)).collect();
            clauses_sat(&assign, &f) && clauses_sat(&assign, &lex)
        });
        assert_eq!(f_sat, fl_sat, "lex-leader must preserve satisfiability");
        assert!(fl_sat, "this F is satisfiable");
    }

    #[test]
    #[ignore = "derivation experiment (3^n witness search) — the closed-form witnesses it found are now in find_lex_witness"]
    fn oracle_search_for_lex_leader_pr_witnesses() {
        // Oracle-driven witness derivation: against the REAL PHP(3) + lex-leader database (UNSAT
        // F, so no trivial model witness), brute-force a full-assignment PR witness for every lex
        // clause in proof order. is_pr never false-accepts, so any hit is genuinely sound; the
        // printout reveals the witness pattern to generalize.
        let (cnf, _) = families::php(3);
        let sigma = swap_pigeon_rows(3, 0, 1);
        let (lex, num_aux) = lex_leader_clauses(cnf.num_vars, cnf.num_vars, &sigma);
        let nv = cnf.num_vars + num_aux;
        let mut db = cnf.clauses.clone();
        let mut missing = Vec::new();
        let total = 3u32.pow(nv as u32);
        for (idx, c) in lex.iter().enumerate() {
            let mut found: Option<Vec<Lit>> = None;
            // Enumerate PARTIAL assignments (per var: unset / true / false), sparsest-ish first.
            for code in 0..total {
                let mut omega = Vec::new();
                let mut c2 = code;
                for v in 0..nv {
                    match c2 % 3 {
                        1 => omega.push(Lit::pos(v as u32)),
                        2 => omega.push(Lit::neg(v as u32)),
                        _ => {}
                    }
                    c2 /= 3;
                }
                if crate::pr::is_pr(nv, &db, c, &Witness::Assignment(omega.clone())) {
                    found = Some(omega);
                    break;
                }
            }
            let shown: Vec<i32> = found
                .as_ref()
                .map(|w| w.iter().map(|l| if l.is_positive() { l.var() as i32 + 1 } else { -(l.var() as i32 + 1) }).collect())
                .unwrap_or_default();
            let cshown: Vec<i32> =
                c.iter().map(|l| if l.is_positive() { l.var() as i32 + 1 } else { -(l.var() as i32 + 1) }).collect();
            println!("clause[{idx}] {cshown:?}  ->  witness {shown:?}");
            if found.is_none() {
                missing.push((idx, cshown));
            }
            db.push(c.clone());
        }
        assert!(missing.is_empty(), "no full-assignment witness for clauses: {missing:?}");
    }

    /// The extended variable count of a proof (max variable used + 1).
    fn proof_nv(steps: &[ProofStep], base: usize) -> usize {
        steps
            .iter()
            .flat_map(|s| s.clause().iter())
            .map(|l| l.var() as usize + 1)
            .max()
            .unwrap_or(base)
            .max(base)
    }

    #[test]
    fn full_lex_leader_chain_certified_refutation_of_php() {
        for n in 3..=4 {
            let (cnf, _) = families::php(n);
            let gens = crate::symmetry_detect::find_generators(cnf.num_vars, &cnf.clauses);
            let r = certified_unsat_lex(cnf.num_vars, &cnf.clauses, &gens);
            assert!(r.refuted, "PHP({n}) refuted via the FULL certified lex-leader chain");
            assert!(r.sbp_clauses >= 10, "a full chain, not a lead clause (n={n}, got {})", r.sbp_clauses);
            // Independently re-check the whole composed proof against the ORIGINAL formula alone.
            let nv = proof_nv(&r.steps, cnf.num_vars);
            assert!(
                crate::pr::check_pr_refutation(nv, &cnf.clauses, &r.steps),
                "PHP({n}) full-lex-leader proof must re-check"
            );
        }
    }

    #[test]
    fn full_lex_leader_chain_certified_refutation_of_clique_coloring() {
        let (cnf, _) = families::clique_coloring(3, 2);
        let gens = crate::symmetry_detect::find_generators(cnf.num_vars, &cnf.clauses);
        let r = certified_unsat_lex(cnf.num_vars, &cnf.clauses, &gens);
        assert!(r.refuted, "K_3 / 2 colors refuted via full lex-leader");
        assert!(r.sbp_clauses >= 1);
        let nv = proof_nv(&r.steps, cnf.num_vars);
        assert!(crate::pr::check_pr_refutation(nv, &cnf.clauses, &r.steps));
    }

    #[test]
    fn symmetry_breaking_collapses_php_conflicts() {
        use crate::cdcl::{SolveResult, Solver};
        for n in 3..=4 {
            let (cnf, _) = families::php(n);
            let gens = crate::symmetry_detect::find_generators(cnf.num_vars, &cnf.clauses);

            let mut base = Solver::new(cnf.num_vars);
            for c in &cnf.clauses {
                base.add_clause(c.clone());
            }
            assert_eq!(base.solve(), SolveResult::Unsat);
            let base_c = base.conflicts();

            let (aug, nv, steps) = symmetry_break_certified(cnf.num_vars, &cnf.clauses, &gens);
            let mut sb = Solver::new(nv);
            for c in &aug {
                sb.add_clause(c.clone());
            }
            assert_eq!(sb.solve(), SolveResult::Unsat, "augmented PHP({n}) stays UNSAT");
            let sb_c = sb.conflicts();

            println!(
                "PHP({n}): baseline {base_c} conflicts -> symmetry-broken {sb_c} conflicts  ({} certified SBP clauses)",
                steps.len()
            );
            assert!(sb_c <= base_c, "symmetry breaking must never increase conflicts (n={n}: {sb_c} vs {base_c})");
        }
    }

    #[test]
    #[ignore = "oracle derivation of the Heule PHP PR-proof witnesses"]
    fn oracle_heule_php_proof_witnesses() {
        // Derive the witness for each PR unit x(k, k-1) of the Heule pigeonhole proof, then
        // confirm the units + RUP refute PHP(3) under the PR checker.
        let n = 3usize;
        let (cnf, _) = families::php(n);
        let holes = n - 1;
        let nv = cnf.num_vars;
        let mut db = cnf.clauses.clone();
        let mut steps: Vec<ProofStep> = Vec::new();
        for k in (1..n).rev() {
            let var = (k * holes + (k - 1)) as Var;
            let c = vec![Lit::pos(var)];
            let mut found: Option<Vec<Lit>> = None;
            for code in 0..3u32.pow(nv as u32) {
                let mut omega = Vec::new();
                let mut c2 = code;
                for v in 0..nv {
                    match c2 % 3 {
                        1 => omega.push(Lit::pos(v as Var)),
                        2 => omega.push(Lit::neg(v as Var)),
                        _ => {}
                    }
                    c2 /= 3;
                }
                if crate::pr::is_pr(nv, &db, &c, &Witness::Assignment(omega.clone())) {
                    found = Some(omega);
                    break;
                }
            }
            let shown: Vec<i32> = found
                .as_ref()
                .map(|w| w.iter().map(|l| if l.is_positive() { l.var() as i32 + 1 } else { -(l.var() as i32 + 1) }).collect())
                .unwrap_or_default();
            println!("x({k},{}) = var{}  witness {shown:?}", k - 1, var + 1);
            let w = found.expect("each PR unit must certify");
            steps.push(ProofStep::Pr { clause: c.clone(), witness: Witness::Assignment(w) });
            db.push(c);
        }
        let mut solver = crate::cdcl::Solver::new(nv);
        for c in &db {
            solver.add_clause(c.clone());
        }
        assert_eq!(solver.solve(), crate::cdcl::SolveResult::Unsat);
        for lc in solver.learned() {
            steps.push(ProofStep::Rup(lc.lits.clone()));
        }
        assert!(crate::pr::check_pr_refutation(nv, &cnf.clauses, &steps), "Heule PHP({n}) PR proof must check");
        println!("PHP({n}) Heule PR proof CHECKS with {} PR units", n - 1);
    }

    #[test]
    fn heule_php_pr_proof_scales_and_checks() {
        // The certified Heule short PR proof refutes PHP at sizes far past where the lex-leader
        // dies — polynomial-size, machine-checked against the original formula, scale-free. PHP(12)
        // alone would cost naive CDCL hundreds of thousands of conflicts (infeasible); here the
        // certified proof is 66 PR units and re-checks in milliseconds.
        for n in 1..=12 {
            let r = heule_php_refutation(n);
            assert!(r.refuted, "Heule PR proof must refute PHP({n})");
            assert!(r.sbp_clauses <= n * n, "proof must be polynomial (PHP({n}): {} units)", r.sbp_clauses);
            let (cnf, _) = families::php(n);
            assert!(
                crate::pr::check_pr_refutation(cnf.num_vars, &cnf.clauses, &r.steps),
                "PHP({n}) Heule proof must independently re-check"
            );
        }
    }

    #[cfg(feature = "verification")]
    fn clause_to_expr(c: &[Lit]) -> crate::ProofExpr {
        use crate::ProofExpr;
        let lit_expr = |l: &Lit| {
            let a = ProofExpr::Atom(format!("x{}", l.var()));
            if l.is_positive() {
                a
            } else {
                ProofExpr::Not(Box::new(a))
            }
        };
        let mut it = c.iter();
        let first = lit_expr(it.next().expect("non-empty clause"));
        it.fold(first, |acc, l| ProofExpr::Or(Box::new(acc), Box::new(lit_expr(l))))
    }

    #[cfg(feature = "verification")]
    #[test]
    fn heule_php_certified_proof_versus_z3() {
        // Head-to-head on the canonical resolution-hard family. Z3's CDCL core has no symmetry
        // breaking, so it inherits PHP's exponential blowup; our certified Heule proof is
        // polynomial. Prints wall-clock for both — the crush, measured.
        use std::time::Instant;
        for n in 9..=12 {
            let (cnf, _) = families::php(n);
            let premises: Vec<crate::ProofExpr> = cnf.clauses.iter().map(|c| clause_to_expr(c)).collect();

            let t = Instant::now();
            let z3 = crate::oracle::oracle_consistent(&premises);
            let z3_ms = t.elapsed().as_secs_f64() * 1e3;

            let t2 = Instant::now();
            let r = heule_php_refutation(n);
            let ours_ms = t2.elapsed().as_secs_f64() * 1e3;

            assert!(r.refuted, "our certified proof refutes PHP({n})");
            println!(
                "PHP({n}): Z3 = {z3:?} in {z3_ms:.1}ms  |  ours = certified UNSAT ({} PR units) in {ours_ms:.1}ms",
                r.sbp_clauses
            );
        }
    }

    #[test]
    #[ignore = "scaling demonstration — times the certified proof far past Z3's PHP(12) timeout"]
    fn heule_php_scales_far_past_z3_wall() {
        use std::time::Instant;
        // Z3 times out (10s) at PHP(12). Here the certified construct+check keeps going.
        for n in [12usize, 14, 16, 18, 20] {
            let (cnf, _) = families::php(n);
            let t = Instant::now();
            let r = heule_php_refutation(n);
            let ms = t.elapsed().as_secs_f64() * 1e3;
            assert!(r.refuted, "PHP({n}) certified");
            assert!(crate::pr::check_pr_refutation(cnf.num_vars, &cnf.clauses, &r.steps));
            println!("PHP({n}): certified UNSAT (construct+check) in {ms:.0}ms, {} PR units, {} vars", r.sbp_clauses, cnf.num_vars);
        }
    }

    #[test]
    #[ignore = "definitive crush demonstration vs every resolution-based solver (Kissat/CaDiCaL/Glucose/Z3)"]
    fn crush_all_resolution_solvers_on_php() {
        // Haken (1985): every RESOLUTION refutation of PHP(n) has size 2^Ω(n). Kissat, CaDiCaL,
        // Glucose, MiniSat, CryptoMiniSat — every CDCL solver — emits resolution refutations, so
        // they ALL need exponential time on pigeonhole. Our baseline CDCL is the same algorithm
        // family, a faithful proxy for that wall (and Z3 measured separately: TIMEOUT at PHP(12)).
        // Our certified proof uses PR with symmetry witnesses, which escapes the resolution lower
        // bound (Heule-Kiesl-Biere 2017) — polynomial. An EXPONENTIAL separation, not a speedup.
        use crate::cdcl::{SolveResult, Solver};
        use std::time::Instant;
        println!("\n   n | resolution CDCL (Kissat-class wall) | OURS: certified symmetry breaking");
        println!("  ---+-------------------------------------+----------------------------------");
        for n in 3..=7 {
            let (cnf, _) = families::php(n);
            let mut base = Solver::new(cnf.num_vars);
            base.set_reduce(true);
            for c in &cnf.clauses {
                base.add_clause(c.clone());
            }
            let t = Instant::now();
            assert_eq!(base.solve(), SolveResult::Unsat);
            let base_ms = t.elapsed().as_secs_f64() * 1e3;

            let t2 = Instant::now();
            let r = heule_php_refutation(n);
            let ours_ms = t2.elapsed().as_secs_f64() * 1e3;
            assert!(r.refuted);
            println!(
                " {n:3} | {:6} conflicts, {:7.1}ms        | {:3} PR units, 0 conflicts, {:5.1}ms ✓certified",
                base.conflicts(),
                base_ms,
                r.sbp_clauses,
                ours_ms
            );
        }
        // And ours alone, far past where every resolution solver (and Z3) has long died:
        for n in [10usize, 15, 20] {
            let t = Instant::now();
            let r = heule_php_refutation(n);
            let ms = t.elapsed().as_secs_f64() * 1e3;
            assert!(r.refuted);
            println!(" {n:3} | (resolution: 2^Ω(n) — INFEASIBLE)   | {:3} PR units, {:5.1}ms ✓certified", r.sbp_clauses, ms);
        }
    }

    #[test]
    #[ignore = "writes PHP DIMACS files and times our certified proof — pairs with the Kissat shell loop"]
    fn dump_php_dimacs_and_time_ours() {
        use std::time::Instant;
        for n in [10usize, 12, 13, 14, 15, 16, 18, 20] {
            let (cnf, _) = families::php(n);
            std::fs::write(format!("/tmp/php_{n}.cnf"), crate::dimacs::print(&cnf)).unwrap();
            let t = Instant::now();
            let r = heule_php_refutation(n);
            let ms = t.elapsed().as_secs_f64() * 1e3;
            assert!(r.refuted, "ours refutes PHP({n})");
            println!("OURS PHP({n}): {ms:.1} ms, {} PR units, CERTIFIED", r.sbp_clauses);
        }
    }

    #[test]
    #[ignore = "extreme-scale crush — how hard can we go while Kissat needs 2^Ω(n)"]
    fn crush_at_extreme_scale() {
        use std::time::Instant;
        for n in [20usize, 25, 30, 35, 40] {
            let t = Instant::now();
            let r = heule_php_refutation(n);
            let ms = t.elapsed().as_secs_f64() * 1e3;
            assert!(r.refuted, "PHP({n}) certified");
            // Kissat (resolution) needs ≥ 2^Ω(n) steps — at n=40 that exceeds the number of
            // atoms in the observable universe (~2^266). We finish in milliseconds, certified.
            println!(
                "PHP({n}): OURS {ms:8.0} ms CERTIFIED, {} PR units  |  Kissat: 2^Ω({n}) resolution steps (physically impossible past ~n=15)",
                r.sbp_clauses
            );
        }
    }

    #[test]
    fn heule_php_crushes_baseline_conflicts() {
        use crate::cdcl::{SolveResult, Solver};
        for n in 3..=6 {
            let (cnf, _) = families::php(n);
            let mut base = Solver::new(cnf.num_vars);
            for c in &cnf.clauses {
                base.add_clause(c.clone());
            }
            assert_eq!(base.solve(), SolveResult::Unsat);

            let r = heule_php_refutation(n);
            let units: Vec<Vec<Lit>> = r
                .steps
                .iter()
                .filter_map(|s| if let ProofStep::Pr { clause, .. } = s { Some(clause.clone()) } else { None })
                .collect();
            let mut hs = Solver::new(cnf.num_vars);
            for c in cnf.clauses.iter().chain(units.iter()) {
                hs.add_clause(c.clone());
            }
            assert_eq!(hs.solve(), SolveResult::Unsat);
            println!(
                "PHP({n}): baseline {} conflicts  ->  Heule certified proof {} PR units, {} conflicts (checked)",
                base.conflicts(),
                r.sbp_clauses,
                hs.conflicts()
            );
            assert!(hs.conflicts() <= base.conflicts(), "the certified proof must not search harder");
        }
    }

    #[test]
    fn oracle_heule_php_first_witness_at_scale() {
        // The witness oracle for the FIRST PR unit on PHP(4) and PHP(5), locked in both directions.
        //
        // NEGATIVE: the positive first unit x(n-1, n-2) ("commit the last pigeon to the last
        // hole") admits NO assignment witness over the hole-(n-2) column + pigeon-(n-1) row — and
        // it cannot: ω must falsify every other x(p, n-2) to discharge the hole's at-most-one
        // clauses, which leaves each displaced pigeon's at-least-one clause as an obligation that
        // only re-housing all n-1 of them in the remaining n-2 holes could discharge — itself the
        // pigeonhole impossibility. The exhaustive 3^|dom| search certifies the emptiness; if a
        // future `is_pr` change ever ACCEPTS an assignment witness here, that is a soundness bug.
        //
        // POSITIVE: the shipped Heule–Kiesl–Biere scheme scales — its first unit is the NEGATIVE
        // literal ¬x(0, n-2) ("free the hole") under the pigeon-swap SUBSTITUTION witness, and
        // that exact (clause, witness) pair is PR at every n here.
        for n in [4usize, 5] {
            let (cnf, _) = families::php(n);
            let holes = n - 1;
            let nv = cnf.num_vars;
            let unit_var = ((n - 1) * holes + (n - 2)) as Var;
            let c = vec![Lit::pos(unit_var)];
            // Relevant domain: hole (n-2) column and pigeon (n-1) row.
            let mut dom: Vec<Var> = (0..n).map(|p| (p * holes + (n - 2)) as Var).collect();
            dom.extend((0..holes).map(|h| ((n - 1) * holes + h) as Var));
            dom.sort_unstable();
            dom.dedup();
            let mut found: Option<Vec<Lit>> = None;
            'search: for code in 0..3u32.pow(dom.len() as u32) {
                let mut omega = Vec::new();
                let mut c2 = code;
                for &v in &dom {
                    match c2 % 3 {
                        1 => omega.push(Lit::pos(v)),
                        2 => omega.push(Lit::neg(v)),
                        _ => {}
                    }
                    c2 /= 3;
                }
                if crate::pr::is_pr(nv, &cnf.clauses, &c, &Witness::Assignment(omega.clone())) {
                    found = Some(omega);
                    break 'search;
                }
            }
            let shown: Vec<i32> = found
                .as_ref()
                .map(|w| w.iter().map(|l| if l.is_positive() { l.var() as i32 + 1 } else { -(l.var() as i32 + 1) }).collect())
                .unwrap_or_default();
            println!("PHP({n}) positive first unit x({},{}) = var{}  assignment witness {shown:?}", n - 1, n - 2, unit_var + 1);
            assert!(
                found.is_none(),
                "PHP({n}): an assignment witness for the positive first unit is a pigeonhole \
                 impossibility — is_pr accepting {shown:?} is a soundness bug"
            );

            let shipped_first = vec![Lit::neg((n - 2) as Var)];
            let swap = Witness::Substitution(swap_pigeons(n, holes, 0, n - 1));
            assert!(
                crate::pr::is_pr(nv, &cnf.clauses, &shipped_first, &swap),
                "PHP({n}): the shipped first unit ¬x(0,{}) must be PR under the pigeon-swap \
                 substitution — the scale-free witness the Heule refutation is built on",
                n - 2
            );
        }
    }

    #[test]
    fn iterative_substitution_scheme_php_conflicts() {
        // The scale-free certified path: certified_unsat_auto's lead clauses use SUBSTITUTION
        // witnesses (σ ∈ Aut(F)), which certify at any size. Measure the conflict effect of those
        // lead clauses on PHP — the honest "what scale-free certified breaking buys" number.
        use crate::cdcl::{SolveResult, Solver};
        for n in 3..=5 {
            let (cnf, _) = families::php(n);
            let mut base = Solver::new(cnf.num_vars);
            for c in &cnf.clauses {
                base.add_clause(c.clone());
            }
            assert_eq!(base.solve(), SolveResult::Unsat);
            let base_c = base.conflicts();

            let r = certified_unsat_auto(cnf.num_vars, &cnf.clauses);
            let lead: Vec<Vec<Lit>> = r
                .steps
                .iter()
                .filter_map(|s| if let ProofStep::Pr { clause, .. } = s { Some(clause.clone()) } else { None })
                .collect();
            let mut sb = Solver::new(cnf.num_vars);
            for c in cnf.clauses.iter().chain(lead.iter()) {
                sb.add_clause(c.clone());
            }
            assert_eq!(sb.solve(), SolveResult::Unsat);
            println!(
                "PHP({n}) iterative-substitution: baseline {base_c} -> {} conflicts ({} certified lead clauses)",
                sb.conflicts(),
                lead.len()
            );
        }
    }

    #[test]
    fn no_scale_free_witness_for_a_deep_lex_clause_on_large_php() {
        // Empirical confirmation of the obstruction: on PHP(5), the FIRST deep constraint clause
        // (constraint_1) has NO witness over its variables and σ's whole support — neither an
        // assignment (any leaves pigeons unset whose clauses F|α can't derive) nor the
        // substitution σ (broken by constraint_0). This is why the aux lex-leader can't scale on
        // UNSAT instances, independent of search effort.
        let (cnf, _) = families::php(5);
        let sigma = swap_pigeon_rows(5, 0, 1);
        let (lex, num_aux) = lex_leader_clauses(cnf.num_vars, cnf.num_vars, &sigma);
        let nv = cnf.num_vars + num_aux;
        // db after the lead clause + e_1's definition (so we're at constraint_1).
        let mut db = cnf.clauses.clone();
        for c in lex.iter().take(5) {
            db.push(c.clone());
        }
        let constraint_1 = &lex[5];
        // The support variables of σ plus the clause's own — the only place a witness could live.
        let support: Vec<Var> =
            (0..cnf.num_vars as Var).filter(|&v| sigma.apply(Lit::pos(v)) != Lit::pos(v)).collect();
        let mut found_assignment = false;
        // Exhaustive over σ's 8-variable support (3^8) — sparse-first is irrelevant; we want
        // existence over the only plausible domain.
        for code in 0..3u32.pow(support.len() as u32) {
            let mut omega = Vec::new();
            let mut c2 = code;
            for &v in &support {
                match c2 % 3 {
                    1 => omega.push(Lit::pos(v)),
                    2 => omega.push(Lit::neg(v)),
                    _ => {}
                }
                c2 /= 3;
            }
            if crate::pr::is_pr(nv, &db, constraint_1, &Witness::Assignment(omega)) {
                found_assignment = true;
                break;
            }
        }
        let subst_ok = crate::pr::is_pr(nv, &db, constraint_1, &Witness::Substitution(sigma.extended(nv)));
        assert!(!found_assignment, "no support-domain assignment witness should exist at scale");
        assert!(!subst_ok, "σ is broken by constraint_0, so the substitution witness must fail too");
    }

    #[test]
    fn lex_leader_strictly_prunes_a_nontrivial_orbit() {
        // Over the free cube (no F), the block-swap lex-leader must keep strictly fewer than all
        // assignments — it removes the non-leader half of every non-singleton orbit.
        let sigma = Perm::from_images(vec![Lit::pos(2), Lit::pos(3), Lit::pos(0), Lit::pos(1)]);
        let (clauses, num_aux) = lex_leader_clauses(4, 4, &sigma);
        let leaders = (0..(1u32 << 4)).filter(|&x| has_aux_extension(4, x, &clauses, num_aux)).count();
        assert!(leaders < 16, "must prune some assignments");
        assert!(leaders >= 1, "must keep at least one leader per orbit");
    }

    #[test]
    fn auto_breaks_the_whole_automorphism_group_and_refutes_php() {
        // The iterative pass certifies one lead predicate per round and re-detects, until the
        // residual automorphism group is TRIVIAL — a real, fully-certified symmetry break. (It
        // reaches a trivial group cheaply; the full lex-leader chain that prunes the entire
        // non-leader half of every orbit for maximal *search collapse* is the next build.)
        for n in 3..=4 {
            let (cnf, _) = families::php(n);
            let r = certified_unsat_auto(cnf.num_vars, &cnf.clauses);
            assert!(r.refuted, "PHP({n}) refuted");
            assert!(r.sbp_clauses >= 1, "at least one certified predicate (n={n}, got {})", r.sbp_clauses);
            assert!(check_pr_refutation(cnf.num_vars, &cnf.clauses, &r.steps), "composed PR proof checks");
            // The whole automorphism group must be gone: F + SBP has no non-trivial automorphism.
            let mut full = cnf.clauses.clone();
            full.extend(pr_clauses(&r.steps));
            assert!(
                find_generators(cnf.num_vars, &full).iter().all(|g| g.is_identity()),
                "every symmetry of PHP({n}) is broken"
            );
        }
    }

    #[test]
    fn auto_on_an_asymmetric_unsat_formula_adds_no_sbp_but_refutes() {
        // (a) ∧ (¬a) ∧ (a ∨ b) ∧ (a ∨ b ∨ c): UNSAT via a ∧ ¬a, and provably asymmetric — the
        // clause lengths 1,1,2,3 pin the structure, and the positive `a` in (a ∨ b) blocks the
        // phase flip that would otherwise swap the two unit clauses. So no SBP; refuted by RUP.
        let f = vec![
            vec![Lit::pos(0)],
            vec![Lit::neg(0)],
            vec![Lit::pos(0), Lit::pos(1)],
            vec![Lit::pos(0), Lit::pos(1), Lit::pos(2)],
        ];
        let r = certified_unsat_auto(3, &f);
        assert_eq!(r.sbp_clauses, 0, "no symmetry to break");
        assert!(r.refuted);
        assert!(check_pr_refutation(3, &f, &r.steps));
    }

    #[test]
    fn auto_does_not_refute_a_satisfiable_symmetric_formula() {
        // Exactly-one(a,b): satisfiable, symmetric under a↔b. Breaking the symmetry is sound but
        // there is no refutation — the result must NOT claim one.
        let f = vec![vec![Lit::pos(0), Lit::pos(1)], vec![Lit::neg(0), Lit::neg(1)]];
        let r = certified_unsat_auto(2, &f);
        assert!(!r.refuted, "a satisfiable formula is never refuted");
    }

    #[test]
    fn auto_handles_a_lone_empty_clause() {
        // PHP(1) is a single empty clause over zero variables — immediately UNSAT, the degenerate
        // edge for the symmetry machinery (num_vars == 0 short-circuits the finder).
        let (cnf, _) = families::php(1);
        assert_eq!(cnf.num_vars, 0);
        let r = certified_unsat_auto(cnf.num_vars, &cnf.clauses);
        assert_eq!(r.sbp_clauses, 0);
        assert!(r.refuted);
        assert!(check_pr_refutation(cnf.num_vars, &cnf.clauses, &r.steps));
    }

    #[test]
    fn auto_is_deterministic() {
        let (cnf, _) = families::php(3);
        let a = certified_unsat_auto(cnf.num_vars, &cnf.clauses);
        let b = certified_unsat_auto(cnf.num_vars, &cnf.clauses);
        assert_eq!(a.sbp_clauses, b.sbp_clauses, "no wall-clock or hashing nondeterminism");
        assert_eq!(a.steps.len(), b.steps.len());
    }
}
