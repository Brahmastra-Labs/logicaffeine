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
pub fn lex_leader_clauses(
    num_vars: usize,
    aux_start: usize,
    sigma: &Perm,
) -> (Vec<Vec<Lit>>, usize) {
    let support: Vec<Var> = (0..num_vars as Var)
        .filter(|&v| sigma.apply(Lit::pos(v)) != Lit::pos(v))
        .collect();
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
pub fn certified_unsat(
    num_vars: usize,
    formula: &[Vec<Lit>],
    generators: &[Perm],
) -> CertifiedRefutation {
    let mut db: Vec<Vec<Lit>> = formula.to_vec();
    let mut steps: Vec<ProofStep> = Vec::new();

    for sigma in generators {
        let Some(clause) = lex_leader_lead_clause(num_vars, sigma) else {
            continue;
        };
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

    CertifiedRefutation {
        refuted,
        sbp_clauses,
        steps,
    }
}

/// Find a PR assignment-witness that certifies lex-leader clause `c` against `db`, drawing from
/// a small principled candidate set (single literals of the clause, then the tied-prefix aux
/// chain optionally combined with a clause literal). Every candidate is verified by [`is_pr`],
/// so a returned witness is genuinely sound; `None` means none of the candidates worked.
fn find_lex_witness(
    nv: usize,
    db: &[Vec<Lit>],
    c: &[Lit],
    base_nv: usize,
    sigma: &Perm,
) -> Option<Witness> {
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
    let max_aux = c
        .iter()
        .map(|l| l.var())
        .filter(|&v| (v as usize) >= base_nv)
        .max();
    let mut vars: Vec<Var> = c.iter().map(|l| l.var()).collect();
    if let Some(ma) = max_aux {
        vars.extend(base_nv as Var..=ma);
    }
    vars.sort_unstable();
    vars.dedup();
    let lits: Vec<Lit> = vars
        .iter()
        .flat_map(|&v| [Lit::pos(v), Lit::neg(v)])
        .collect();

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
                steps.push(ProofStep::Pr {
                    clause: c.clone(),
                    witness: w,
                });
                committed = true;
            }
        }
        if committed {
            nv = ext_nv;
        }
    }
    (db, nv, steps)
}

pub fn certified_unsat_lex(
    num_vars: usize,
    formula: &[Vec<Lit>],
    generators: &[Perm],
) -> CertifiedRefutation {
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

    CertifiedRefutation {
        refuted,
        sbp_clauses,
        steps,
    }
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

    CertifiedRefutation {
        refuted,
        sbp_clauses,
        steps,
    }
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

    CertifiedRefutation {
        refuted,
        sbp_clauses,
        steps,
    }
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

    crate::complexity::RankedRefutation {
        refuted,
        steps,
        ranks,
    }
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

    crate::complexity::RankedRefutation {
        refuted,
        steps,
        ranks,
    }
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
            let Some(clause) = lex_leader_lead_clause(num_vars, &sigma) else {
                continue;
            };
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

    CertifiedRefutation {
        refuted,
        sbp_clauses,
        steps,
    }
}

/// The breaks [`conditional_symmetry_breaks`] certified: the database with them added, the PR steps that justify them
/// (each a [`Witness::PartialSymmetry`]), and the transpositions they came from, one entry per added clause.
#[derive(Clone, Debug)]
/// What [`conditional_symmetry_breaks`] added to a formula.
pub struct ConditionalBreaks {
    /// The formula followed by every accepted break, in the order they were certified.
    pub db: Vec<Vec<Lit>>,
    /// One [`ProofStep::Pr`] per accepted break, each witnessed by [`Witness::PartialSymmetry`].
    pub steps: Vec<ProofStep>,
    /// The transposition `(a, b)` behind each step, in step order.
    pub pairs: Vec<(usize, usize)>,
}

/// The transposition of variables `a` and `b` over `num_vars` variables.
fn transposition(num_vars: usize, a: usize, b: usize) -> Perm {
    Perm::from_images(
        (0..num_vars)
            .map(|v| Lit::pos(if v == a { b } else if v == b { a } else { v } as u32))
            .collect(),
    )
}

/// The most clauses one transposition's guard may expand to (one per choice of a literal from each broken image).
const MAX_GUARD_CLAUSES: usize = 64;

/// **Certified conditional symmetry breaking.** A transposition `σ = (a b)` that maps every clause of `formula` but a
/// few onto clauses of it ([`crate::solve::almost_symmetry_pairs`]) is a partial symmetry: it carries a model `τ` to a
/// model exactly when `τ` satisfies the images `E₁ … E_r` of the clauses it breaks. The break keeps `x_a ≤ x_b` where
/// that guard holds, `(E₁ ∧ … ∧ E_r) → (¬x_a ∨ x_b)`, as the clauses `¬e₁ ∨ … ∨ ¬e_r ∨ ¬x_a ∨ x_b`, one per choice of a
/// literal `e_j` from each `E_j`. Each clause is added only once [`is_pr`] accepts it with the witness
/// [`Witness::PartialSymmetry`]`(σ)` against the database so far: under its negation each `e_j` holds, so each broken
/// image is satisfied, and every other image is a clause of the formula. Because every check is against every break
/// before it, the breaks of different transpositions compose — what the single uncertified guarded break could not.
/// By [`crate::groupoid::ModelGroupoid`], every component of the model groupoid keeps a model.
pub fn conditional_symmetry_breaks(num_vars: usize, formula: &[Vec<Lit>], max_broken: usize) -> ConditionalBreaks {
    let mut db: Vec<Vec<Lit>> = formula.to_vec();
    let mut steps: Vec<ProofStep> = Vec::new();
    let mut pairs: Vec<(usize, usize)> = Vec::new();
    for (a, b, images) in crate::solve::almost_symmetry_pairs(num_vars, formula, max_broken) {
        let product: usize = images.iter().map(|e| e.len().max(1)).product();
        if images.iter().any(|e| e.is_empty()) || product > MAX_GUARD_CLAUSES {
            continue;
        }
        let sigma = transposition(num_vars, a, b);
        let mut choice = vec![0usize; images.len()];
        loop {
            let mut clause: Vec<Lit> = images.iter().zip(&choice).map(|(e, &k)| e[k].negated()).collect();
            clause.push(Lit::neg(a as u32));
            clause.push(Lit::pos(b as u32));
            clause.sort_by_key(|l| (l.var(), l.is_positive()));
            clause.dedup();
            let tautology = clause.windows(2).any(|w| w[0].var() == w[1].var());
            if !tautology {
                let witness = Witness::PartialSymmetry(sigma.clone());
                if is_pr(num_vars, &db, &clause, &witness) {
                    db.push(clause.clone());
                    steps.push(ProofStep::Pr { clause, witness });
                    pairs.push((a, b));
                }
            }
            let mut j = 0;
            while j < choice.len() {
                choice[j] += 1;
                if choice[j] < images[j].len() {
                    break;
                }
                choice[j] = 0;
                j += 1;
            }
            if j == choice.len() {
                break;
            }
        }
    }
    ConditionalBreaks { db, steps, pairs }
}

/// Refute `formula` with certified conditional symmetry breaking: add the breaks of
/// [`conditional_symmetry_breaks`], solve, append the learned clauses as RUP steps, and check the whole stream
/// against `formula` alone.
pub fn certified_unsat_conditional(num_vars: usize, formula: &[Vec<Lit>], max_broken: usize) -> CertifiedRefutation {
    let ConditionalBreaks { db, mut steps, .. } = conditional_symmetry_breaks(num_vars, formula, max_broken);
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
#[path = "../../logicaffeine_tests/unit/logicaffeine_proof/sym_certify/tests.rs"]
mod tests;
