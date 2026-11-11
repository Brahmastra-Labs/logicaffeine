use super::*;
use crate::cdcl::Lit;
use crate::proof::Perm;

fn p(v: u32) -> Lit {
    Lit::pos(v)
}
fn n(v: u32) -> Lit {
    Lit::neg(v)
}

/// Brute-force satisfiability over `num_vars` variables — the independent oracle.
fn sat_brute(num_vars: usize, clauses: &[Vec<Lit>]) -> bool {
    for mask in 0u32..(1u32 << num_vars) {
        let model: Vec<bool> = (0..num_vars).map(|v| (mask >> v) & 1 == 1).collect();
        if clauses
            .iter()
            .all(|c| c.iter().any(|l| model[l.var() as usize] == l.is_positive()))
        {
            return true;
        }
    }
    false
}

#[test]
fn pr_accepts_a_model_removing_clause_that_rup_rejects() {
    // F = (a ∨ b) is satisfiable and symmetric under a↔b. The lex-leader C = (¬a ∨ b)
    // deletes the model a=1,b=0 — model-removing, so NOT RUP — but is satisfiability-
    // preserving, certified by the assignment witness ω = {a=0, b=1}.
    let (a, b) = (0u32, 1u32);
    let f = vec![vec![p(a), p(b)]];
    let c = vec![n(a), p(b)];

    assert!(
        !rup::is_rup(2, &f, &c),
        "a model-removing clause is not RUP"
    );
    let omega = Witness::Assignment(vec![n(a), p(b)]);
    assert!(is_pr(2, &f, &c, &omega), "but it IS propagation-redundant");
    // And the addition really is satisfiability-preserving.
    let mut fc = f.clone();
    fc.push(c.clone());
    assert_eq!(sat_brute(2, &f), sat_brute(2, &fc));
}

#[test]
fn pr_accepts_via_a_substitution_witness() {
    // The same break, certified by the symmetry σ = (a↔b) instead of a hand-written ω.
    let (a, b) = (0u32, 1u32);
    let f = vec![vec![p(a), p(b)]];
    let c = vec![n(a), p(b)];
    let sigma = Perm::from_images(vec![p(b), p(a)]); // +a↦+b, +b↦+a
    assert!(is_pr(2, &f, &c, &Witness::Substitution(sigma)));
}

#[test]
fn pr_rejects_a_witness_that_does_not_satisfy_the_clause() {
    // ω = α itself (a=1,b=0) falsifies C, so it is not a witness — reject, fail-closed.
    let (a, b) = (0u32, 1u32);
    let f = vec![vec![p(a), p(b)]];
    let c = vec![n(a), p(b)];
    assert!(!is_pr(2, &f, &c, &Witness::Assignment(vec![p(a), n(b)])));
}

#[test]
fn pr_rejects_a_bogus_substitution_on_an_asymmetric_formula() {
    // F = (a ∨ b) ∧ (¬b) forces the unique model a=1,b=0; it is NOT symmetric under a↔b
    // (σ maps ¬b to ¬a, which is not a clause of F). The lex-leader C = (¬a ∨ b) deletes
    // that only model, so blindly trusting σ would flip SAT→UNSAT — PR must reject it.
    let (a, b) = (0u32, 1u32);
    let f = vec![vec![p(a), p(b)], vec![n(b)]];
    let c = vec![n(a), p(b)];
    let sigma = Perm::from_images(vec![p(b), p(a)]);
    assert!(!is_pr(2, &f, &c, &Witness::Substitution(sigma)));
    // Confirm the trap: adding C really would have been unsound.
    let mut fc = f.clone();
    fc.push(c.clone());
    assert!(
        sat_brute(2, &f) && !sat_brute(2, &fc),
        "C would flip SAT→UNSAT"
    );
}

#[test]
fn check_pr_refutation_agrees_with_rup_on_pure_rup_proofs() {
    // With only RUP steps, the PR driver must behave exactly like the RUP checker.
    let (a, b) = (0u32, 1u32);
    let f = vec![
        vec![p(a), p(b)],
        vec![p(a), n(b)],
        vec![n(a), p(b)],
        vec![n(a), n(b)],
    ];
    let steps = vec![ProofStep::Rup(vec![p(a)]), ProofStep::Rup(vec![n(a)])];
    assert!(check_pr_refutation(2, &f, &steps));
    // Without the resolvents it cannot close.
    assert!(!check_pr_refutation(2, &f, &[]));
}

#[test]
fn fast_checker_agrees_with_the_trusted_checker_on_php() {
    // The accelerated checker must reach the SAME verdict as the stateless trusted checker on
    // the real certified PHP refutations — and on a deliberately corrupted one (both reject).
    for n in 3..=6 {
        let cr = crate::sym_certify::heule_php_refutation(n);
        let (cnf, _) = crate::families::php(n);
        assert_eq!(
            check_pr_refutation_fast(cnf.num_vars, &cnf.clauses, &cr.steps),
            check_pr_refutation(cnf.num_vars, &cnf.clauses, &cr.steps),
            "fast vs trusted disagree on PHP({n})"
        );
        assert!(
            check_pr_refutation_fast(cnf.num_vars, &cnf.clauses, &cr.steps),
            "PHP({n}) refutes"
        );

        // Corrupt the first PR witness → both checkers must reject.
        let mut bad = cr.steps.clone();
        if let Some(ProofStep::Pr { witness, .. }) =
            bad.iter_mut().find(|s| matches!(s, ProofStep::Pr { .. }))
        {
            *witness = Witness::Substitution(Perm::identity(cnf.num_vars));
        }
        assert_eq!(
            check_pr_refutation_fast(cnf.num_vars, &cnf.clauses, &bad),
            check_pr_refutation(cnf.num_vars, &cnf.clauses, &bad),
            "fast vs trusted disagree on corrupted PHP({n})"
        );
    }
}

#[test]
fn pr_never_blesses_an_unsound_addition_random() {
    // Robustness to the point of absurdity: over many seeded random tiny formulas and
    // random candidate (clause, witness) pairs, WHENEVER is_pr accepts, brute force must
    // confirm the addition preserved satisfiability. A single false-accept is a hard fail.
    let mut state = 0x9E3779B97F4A7C15u64;
    let mut next = || {
        // SplitMix64 — deterministic, no wall-clock seeding.
        state = state.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    };
    let num_vars = 4usize;
    let rand_clause = |next: &mut dyn FnMut() -> u64| -> Vec<Lit> {
        let len = 1 + (next() as usize % 3);
        let mut c = Vec::new();
        for _ in 0..len {
            let v = (next() as u32) % num_vars as u32;
            let lit = Lit::new(v, next() & 1 == 0);
            if !c.contains(&lit) && !c.contains(&lit.negated()) {
                c.push(lit);
            }
        }
        c
    };

    let mut accepted = 0;
    for _ in 0..20_000 {
        // Random small formula.
        let nclauses = next() as usize % 5;
        let f: Vec<Vec<Lit>> = (0..nclauses)
            .map(|_| rand_clause(&mut next))
            .filter(|c| !c.is_empty())
            .collect();
        let c = rand_clause(&mut next);
        if c.is_empty() {
            continue;
        }
        // Random assignment witness over a random subset of variables.
        let mut omega = Vec::new();
        for v in 0..num_vars as u32 {
            if next() & 1 == 0 {
                omega.push(Lit::new(v, next() & 1 == 0));
            }
        }
        if is_pr(num_vars, &f, &c, &Witness::Assignment(omega)) {
            accepted += 1;
            let before = sat_brute(num_vars, &f);
            let mut fc = f.clone();
            fc.push(c.clone());
            let after = sat_brute(num_vars, &fc);
            assert_eq!(
                before, after,
                "PR accepted a clause that changed satisfiability: F={f:?} C={c:?}"
            );
        }
    }
    assert!(accepted > 0, "the test must actually exercise acceptances");
}

#[test]
fn sr_never_blesses_an_unsound_addition_random() {
    // The substitution-redundancy path's soundness net: over many seeded random formulas,
    // random literal permutations, and random clauses, WHENEVER is_pr accepts under a
    // substitution witness, brute force must confirm the addition kept satisfiability.
    let mut state = 0xD1B54A32D192ED03u64;
    let mut next = || {
        state = state.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    };
    let num_vars = 4usize;
    let rand_clause = |next: &mut dyn FnMut() -> u64| -> Vec<Lit> {
        let len = 1 + (next() as usize % 3);
        let mut c = Vec::new();
        for _ in 0..len {
            let v = (next() as u32) % num_vars as u32;
            let lit = Lit::new(v, next() & 1 == 0);
            if !c.contains(&lit) && !c.contains(&lit.negated()) {
                c.push(lit);
            }
        }
        c
    };
    // A random literal permutation: a Fisher-Yates shuffle of variables, each image given a
    // random phase (identity a quarter of the time, to reliably exercise acceptances).
    let rand_perm = |next: &mut dyn FnMut() -> u64| -> Perm {
        if next() % 4 == 0 {
            return Perm::identity(num_vars);
        }
        let mut order: Vec<u32> = (0..num_vars as u32).collect();
        for i in (1..num_vars).rev() {
            let j = next() as usize % (i + 1);
            order.swap(i, j);
        }
        Perm::from_images(
            (0..num_vars)
                .map(|v| Lit::new(order[v], next() & 1 == 0))
                .collect(),
        )
    };

    let mut accepted = 0;
    for _ in 0..20_000 {
        let nclauses = next() as usize % 5;
        let f: Vec<Vec<Lit>> = (0..nclauses)
            .map(|_| rand_clause(&mut next))
            .filter(|c| !c.is_empty())
            .collect();
        let c = rand_clause(&mut next);
        if c.is_empty() {
            continue;
        }
        let sigma = rand_perm(&mut next);
        if is_pr(num_vars, &f, &c, &Witness::Substitution(sigma)) {
            accepted += 1;
            let before = sat_brute(num_vars, &f);
            let mut fc = f.clone();
            fc.push(c.clone());
            let after = sat_brute(num_vars, &fc);
            assert_eq!(
                before, after,
                "SR accepted a clause that changed satisfiability: F={f:?} C={c:?}"
            );
        }
    }
    assert!(
        accepted > 0,
        "the substitution path must actually be exercised"
    );
}

/// A deterministic SplitMix64 stream for the sweeps below.
fn splitmix(seed: u64) -> impl FnMut() -> u64 {
    let mut state = seed;
    move || {
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

/// A guarded break that no automorphism certifies and the partial symmetry does. `F = (a ∨ x)(b ∨ x)(a ∨ y)` is
/// symmetric in `a, b` except for `(a ∨ y)`, whose image `(b ∨ y)` is not a clause. The break `¬y ∨ ¬a ∨ b` — where
/// the image's literal `y` holds, `a ≤ b` — removes models (not RUP), is refused under the automorphism witness, and
/// is accepted under the partial-symmetry witness; adding it keeps `F` satisfiable. The unguarded `¬a ∨ b` is refused:
/// under it the broken image does not follow.
#[test]
fn a_partial_symmetry_certifies_a_guarded_break_no_automorphism_can() {
    let (a, b, x, y) = (0u32, 1u32, 2u32, 3u32);
    let f = vec![vec![p(a), p(x)], vec![p(b), p(x)], vec![p(a), p(y)]];
    let guarded = vec![n(y), n(a), p(b)];
    let swap = Perm::from_images(vec![p(b), p(a), p(x), p(y)]);
    assert!(!crate::rup::is_rup(4, &f, &guarded), "the break removes models: it is not RUP");
    assert!(!is_pr(4, &f, &guarded, &Witness::Substitution(swap.clone())), "a↔b is no automorphism of F");
    assert!(is_pr(4, &f, &guarded, &Witness::PartialSymmetry(swap.clone())), "a↔b is a partial symmetry on ¬C");
    let mut fc = f.clone();
    fc.push(guarded);
    assert_eq!(sat_brute(4, &f), sat_brute(4, &fc), "the guarded break keeps F satisfiable");
    assert!(
        !is_pr(4, &f, &[n(a), p(b)], &Witness::PartialSymmetry(swap)),
        "without the guard the image (b ∨ y) does not follow from ¬C"
    );
}

/// The partial-symmetry check's soundness net, over tens of thousands of random tiny formulas, clauses and mappings —
/// permutations with phases and arbitrary literal maps that are not even bijections: whenever it accepts, brute force
/// confirms the addition kept satisfiability. It must accept where no automorphism exists (the case it adds), and it
/// accepts everything the automorphism check accepts.
#[test]
fn partial_symmetry_never_blesses_an_unsound_addition_random() {
    let mut next = splitmix(0x5EED_0F_6A0C_u64);
    let num_vars = 4usize;
    let rand_clause = |next: &mut dyn FnMut() -> u64| -> Vec<Lit> {
        let len = 1 + (next() as usize % 3);
        let mut c: Vec<Lit> = Vec::new();
        for _ in 0..len {
            let lit = Lit::new((next() as u32) % num_vars as u32, next() & 1 == 0);
            if !c.contains(&lit) && !c.contains(&lit.negated()) {
                c.push(lit);
            }
        }
        c
    };
    let (mut accepted, mut beyond_automorphism) = (0, 0);
    for round in 0..30_000u32 {
        let nclauses = (next() % 6) as usize;
        let f: Vec<Vec<Lit>> = (0..nclauses).map(|_| rand_clause(&mut next)).filter(|c| !c.is_empty()).collect();
        let c = rand_clause(&mut next);
        if c.is_empty() {
            continue;
        }
        let sigma = if round % 3 == 0 {
            // An arbitrary literal map: every variable to a random literal, collisions allowed.
            Perm::from_images((0..num_vars).map(|_| Lit::new((next() as u32) % num_vars as u32, next() & 1 == 0)).collect())
        } else {
            let mut order: Vec<u32> = (0..num_vars as u32).collect();
            for i in (1..num_vars).rev() {
                order.swap(i, next() as usize % (i + 1));
            }
            Perm::from_images((0..num_vars).map(|v| Lit::new(order[v], next() % 4 != 0)).collect())
        };
        let partial = is_pr(num_vars, &f, &c, &Witness::PartialSymmetry(sigma.clone()));
        if is_pr(num_vars, &f, &c, &Witness::Substitution(sigma.clone())) {
            assert!(partial, "an automorphism witness the fast check accepts is a partial symmetry: F={f:?} C={c:?}");
        }
        if partial {
            accepted += 1;
            if !crate::symmetry_detect::perm_is_automorphism(&f, &sigma) {
                beyond_automorphism += 1;
            }
            let mut fc = f.clone();
            fc.push(c.clone());
            assert_eq!(
                sat_brute(num_vars, &f),
                sat_brute(num_vars, &fc),
                "a partial symmetry blessed a clause that changed satisfiability: F={f:?} C={c:?} σ={sigma:?}"
            );
        }
    }
    assert!(accepted > 1000, "the partial-symmetry path accepted only {accepted} times");
    assert!(beyond_automorphism > 100, "only {beyond_automorphism} acceptances went beyond automorphisms");
}

/// The bogus swap of the asymmetric formula stays refused under the general criterion: `F = (a ∨ b) ∧ ¬b` has the
/// single model `a ∧ ¬b`, and the lex-leader `¬a ∨ b` would delete it.
#[test]
fn partial_symmetry_refuses_a_break_that_deletes_the_only_model() {
    let (a, b) = (0u32, 1u32);
    let f = vec![vec![p(a), p(b)], vec![n(b)]];
    let c = vec![n(a), p(b)];
    let sigma = Perm::from_images(vec![p(b), p(a)]);
    assert!(!is_pr(2, &f, &c, &Witness::PartialSymmetry(sigma)));
}

/// A mapping naming a variable outside the formula certifies nothing rather than indexing out of bounds.
#[test]
fn partial_symmetry_refuses_a_mapping_outside_the_variables() {
    let f = vec![vec![p(0), p(1)]];
    let c = vec![n(0), p(1)];
    let outside = Perm::from_images(vec![p(5), p(0)]);
    assert!(!is_pr(2, &f, &c, &Witness::PartialSymmetry(outside)));
    let short = Perm::from_images(vec![p(0)]);
    assert!(!is_pr(2, &f, &c, &Witness::PartialSymmetry(short)), "a mapping that misses a variable of F is refused");
}

/// Both checkers agree on refutations that carry partial-symmetry steps, and both refuse one whose witness is
/// replaced by the identity.
#[test]
fn both_checkers_agree_on_partial_symmetry_refutations() {
    let mut checked = 0;
    for seed in 0..400u64 {
        let mut next = splitmix(seed);
        let nv = 5;
        let f: Vec<Vec<Lit>> = (0..14)
            .map(|_| {
                let mut c: Vec<Lit> = Vec::new();
                while c.len() < 2 + (seed % 2) as usize {
                    let l = Lit::new((next() % nv as u64) as u32, next() & 1 == 0);
                    if !c.iter().any(|m| m.var() == l.var()) {
                        c.push(l);
                    }
                }
                c
            })
            .collect();
        let r = crate::sym_certify::certified_unsat_conditional(nv, &f, 2);
        if !r.refuted || r.sbp_clauses == 0 {
            continue;
        }
        assert!(check_pr_refutation(nv, &f, &r.steps));
        assert!(check_pr_refutation_fast(nv, &f, &r.steps));
        let mut bad = r.steps.clone();
        if let Some(ProofStep::Pr { witness, .. }) = bad.iter_mut().find(|s| matches!(s, ProofStep::Pr { .. })) {
            *witness = Witness::PartialSymmetry(Perm::identity(nv));
        }
        assert_eq!(check_pr_refutation(nv, &f, &bad), check_pr_refutation_fast(nv, &f, &bad));
        checked += 1;
    }
    assert!(checked >= 5, "only {checked} refutations carried partial-symmetry steps");
}

/// A refutation whose preprocessing introduced variables (bounded variable addition's RAT definitions) names
/// variables past the formula's: both checkers accept it, the database's variables widened to the proof's.
#[test]
fn a_refutation_naming_variables_past_the_formula_checks() {
    use crate::cdcl::{SolveResult, Solver};
    // A pairwise at-most-one over eight variables and two disjoint at-least-ones within it: unsatisfiable, and
    // factorable.
    let mut formula: Vec<Vec<Lit>> = Vec::new();
    for a in 0..8u32 {
        for b in a + 1..8 {
            formula.push(vec![Lit::neg(a), Lit::neg(b)]);
        }
    }
    formula.push(vec![Lit::pos(0), Lit::pos(1), Lit::pos(2)]);
    formula.push(vec![Lit::pos(3), Lit::pos(4), Lit::pos(5)]);
    let options = crate::preprocess::Options { factor: true, eliminate: false, ..Default::default() };
    let pre = crate::preprocess::preprocess(8, &formula, &options);
    assert!(pre.factored >= 1 && pre.num_vars > 8, "the at-most-one is factored");
    let mut proof = pre.steps.clone();
    if !pre.unsat {
        let mut s = Solver::new(pre.num_vars);
        for c in &pre.clauses {
            s.add_clause(c.clone());
        }
        assert_eq!(s.solve(), SolveResult::Unsat);
        proof.extend(s.learned().iter().map(|lc| ProofStep::Rup(lc.lits.clone())));
    }
    assert!(proof.iter().any(|s| s.clause().iter().any(|l| l.var() >= 8)), "the proof names an introduced variable");
    assert!(check_pr_refutation(8, &formula, &proof), "the checker widens to the proof's variables");
    assert!(check_pr_refutation_fast(8, &formula, &proof), "and so does the fast one");
}

/// Widening is no licence: a PR step on a variable past the stated count that the formula already mentions is
/// checked against the clauses holding its negation like any other, and refused when a resolvent does not follow.
#[test]
fn a_variable_past_the_stated_count_is_not_fresh_when_the_formula_mentions_it() {
    // x4 → x5 is in the formula; claiming x0 ∨ x4 by the witness x4 needs x0 ∨ x5, which does not follow.
    let formula = vec![vec![Lit::neg(4), Lit::pos(5)]];
    let steps = vec![
        ProofStep::Pr { clause: vec![Lit::pos(0), Lit::pos(4)], witness: Witness::Assignment(vec![Lit::pos(4)]) },
        ProofStep::Rup(Vec::new()),
    ];
    assert!(!check_pr_refutation(1, &formula, &steps));
    assert!(!check_pr_refutation_fast(1, &formula, &steps));
    // The same claim with nothing holding ¬x4 is RAT: x4 is defined, and the step checks (the refutation still
    // fails at the empty clause, which nothing implies).
    let free = vec![vec![Lit::pos(5), Lit::pos(6)]];
    assert!(crate::pr::is_pr(7, &free, &[Lit::pos(0), Lit::pos(4)], &Witness::Assignment(vec![Lit::pos(4)])));
    assert!(!check_pr_refutation(1, &free, &steps));
}

/// A refutation of `formula` the way the CDCL route makes one: preprocessing with factoring (RUP and PR additions,
/// deletions), then the search's learned clauses.
fn route_proof(num_vars: usize, formula: &[Vec<Lit>]) -> Option<Vec<ProofStep>> {
    use crate::cdcl::{SolveResult, Solver};
    let options = crate::preprocess::Options { factor: true, ..Default::default() };
    let pre = crate::preprocess::preprocess(num_vars, formula, &options);
    let mut proof = pre.steps.clone();
    if !pre.unsat {
        let mut s = Solver::new(pre.num_vars);
        for c in &pre.clauses {
            s.add_clause(c.clone());
        }
        if s.solve() != SolveResult::Unsat {
            return None;
        }
        proof.extend(s.learned().iter().map(|lc| ProofStep::Rup(lc.lits.clone())));
    }
    Some(proof)
}

/// The fast checker decides every refutation as the reference checker does — the CDCL route's proofs, with their
/// PR steps and deletions, and the same proofs with a step corrupted: a literal flipped, a step dropped, a witness
/// moved off its clause, a clause deleted early, the proof cut to its first half.
#[test]
fn the_fast_checker_decides_every_refutation_as_the_reference_does() {
    let mut state = 0xF0E7u64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    let (mut accepted, mut rejected) = (0usize, 0usize);
    for round in 0..300u64 {
        let n = 6 + (next() % 10) as usize;
        let mut formula: Vec<Vec<Lit>> = (0..(4.5 * n as f64) as usize)
            .map(|_| (0..3).map(|_| Lit::new((next() % n as u64) as u32, next() & 1 == 0)).collect())
            .collect();
        // A product to factor, so the proofs carry PR steps.
        for a in 0..3u32 {
            for b in 0..3u32 {
                formula.push(vec![Lit::neg(a % n as u32), Lit::pos((3 + b) % n as u32)]);
            }
        }
        let Some(proof) = route_proof(n, &formula) else { continue };
        let mut proofs = vec![proof.clone()];
        if !proof.is_empty() {
            let at = (next() % proof.len() as u64) as usize;
            let mut flipped = proof.clone();
            match &mut flipped[at] {
                ProofStep::Rup(c) | ProofStep::Delete(c) | ProofStep::Pr { clause: c, .. } if !c.is_empty() => {
                    let k = (next() % c.len() as u64) as usize;
                    c[k] = c[k].negated();
                }
                _ => {}
            }
            proofs.push(flipped);
            let mut dropped = proof.clone();
            dropped.remove(at);
            proofs.push(dropped);
            let mut early = proof.clone();
            if let Some(d) = proof.iter().position(|s| matches!(s, ProofStep::Delete(_))) {
                let step = early.remove(d);
                early.insert(0, step);
            }
            proofs.push(early);
            let mut moved = proof.clone();
            if let Some(ProofStep::Pr { witness: Witness::Assignment(w), .. }) = moved.iter_mut().find(|s| matches!(s, ProofStep::Pr { .. })) {
                for l in w.iter_mut() {
                    *l = l.negated();
                }
            }
            proofs.push(moved);
            proofs.push(proof[..proof.len() / 2].to_vec());
        }
        for (k, p) in proofs.iter().enumerate() {
            let reference = check_pr_refutation(n, &formula, p);
            assert_eq!(check_pr_refutation_fast(n, &formula, p), reference, "round {round}, proof {k}");
            if reference {
                accepted += 1;
            } else {
                rejected += 1;
            }
        }
    }
    assert!(accepted >= 100 && rejected >= 100, "both verdicts are exercised: {accepted} accepted, {rejected} rejected");
}

/// Checking a refutation costs its propagations, not a scan of the database per step: a factored pairwise
/// at-most-one over 250 variables beside two at-least-ones, refuted through preprocessing and search, checks within
/// a minute unoptimized on a loaded box.
#[test]
fn checking_a_long_refutation_costs_its_propagations() {
    let mut formula: Vec<Vec<Lit>> = Vec::new();
    for a in 0..250u32 {
        for b in a + 1..250 {
            formula.push(vec![Lit::neg(a), Lit::neg(b)]);
        }
    }
    formula.push((0..125).map(Lit::pos).collect());
    formula.push((125..250).map(Lit::pos).collect());
    let proof = route_proof(250, &formula).expect("two at-least-ones inside an at-most-one are unsatisfiable");
    assert!(proof.iter().any(|s| matches!(s, ProofStep::Pr { .. })) && proof.iter().any(|s| matches!(s, ProofStep::Delete(_))));
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(check_pr_refutation_fast(250, &formula, &proof));
    });
    assert!(rx.recv_timeout(std::time::Duration::from_secs(60)).expect("the check costs its propagations"));
}
