//! The clausifier validated to ABSURDITY by an exhaustive oracle. Over random NESTED
//! propositional formulas — which force the Tseitin `encode` And/Or/Implies/Iff branches that a
//! flat CNF never reaches — `cdcl_entails` must agree with brute-force truth-table entailment for
//! EVERY generated `(premises ⊨ goal)` instance. A dropped connective arm makes `encode` return
//! `None` (its fallback is `atom_key`, which fails on a compound), a flipped defining-clause
//! polarity changes the models — either way the oracle disagrees on the first witnessing formula.
use super::*;
use crate::{ProofExpr, ProofTerm};

const N_ATOMS: usize = 3;
const NAMES: [&str; N_ATOMS] = ["P", "Q", "R"];

fn atom(i: usize) -> ProofExpr {
    ProofExpr::Predicate {
        name: NAMES[i].to_string(),
        args: vec![ProofTerm::Constant("a".to_string())],
        world: None,
    }
}
fn not(e: ProofExpr) -> ProofExpr {
    ProofExpr::Not(Box::new(e))
}
fn and(a: ProofExpr, b: ProofExpr) -> ProofExpr {
    ProofExpr::And(Box::new(a), Box::new(b))
}
fn or(a: ProofExpr, b: ProofExpr) -> ProofExpr {
    ProofExpr::Or(Box::new(a), Box::new(b))
}
fn imp(a: ProofExpr, b: ProofExpr) -> ProofExpr {
    ProofExpr::Implies(Box::new(a), Box::new(b))
}
fn iff(a: ProofExpr, b: ProofExpr) -> ProofExpr {
    ProofExpr::Iff(Box::new(a), Box::new(b))
}

/// Evaluate a formula under a truth assignment to {P, Q, R} — the oracle's ground truth.
fn eval(e: &ProofExpr, m: &[bool; N_ATOMS]) -> bool {
    match e {
        ProofExpr::Predicate { name, .. } => {
            m[NAMES.iter().position(|n| n == name).expect("known atom")]
        }
        ProofExpr::Not(p) => !eval(p, m),
        ProofExpr::And(p, q) => eval(p, m) && eval(q, m),
        ProofExpr::Or(p, q) => eval(p, m) || eval(q, m),
        ProofExpr::Implies(p, q) => !eval(p, m) || eval(q, m),
        ProofExpr::Iff(p, q) => eval(p, m) == eval(q, m),
        other => unreachable!("the generator only builds boolean formulas, got {other:?}"),
    }
}

/// `premises ⊨ goal` by exhaustive truth-table: no assignment satisfies all premises yet
/// falsifies the goal.
fn brute_entails(premises: &[ProofExpr], goal: &ProofExpr) -> bool {
    for bits in 0..(1u32 << N_ATOMS) {
        let m = [bits & 1 != 0, bits & 2 != 0, bits & 4 != 0];
        if premises.iter().all(|p| eval(p, &m)) && !eval(goal, &m) {
            return false;
        }
    }
    true
}

/// Deterministic SplitMix64 — reproducible, no ambient randomness.
fn next(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// A random formula of bounded depth over the 3 atoms, biased to build genuinely nested
/// (non-clausal) structure so the Tseitin `encode` branches are actually exercised.
fn gen(state: &mut u64, depth: u32) -> ProofExpr {
    if depth == 0 || next(state) % 4 == 0 {
        return atom((next(state) % N_ATOMS as u64) as usize);
    }
    match next(state) % 5 {
        0 => not(gen(state, depth - 1)),
        1 => and(gen(state, depth - 1), gen(state, depth - 1)),
        2 => or(gen(state, depth - 1), gen(state, depth - 1)),
        3 => imp(gen(state, depth - 1), gen(state, depth - 1)),
        _ => iff(gen(state, depth - 1), gen(state, depth - 1)),
    }
}

#[test]
fn encode_defining_clauses_are_semantically_exact_for_every_connective() {
    // Each case puts a COMPOUND where clausification cannot flatten it (a disjunct / an Iff
    // side), forcing the corresponding `encode` branch, and is a genuine entailment — so a
    // broken (`None`) or mis-polarised encode gives the wrong verdict.
    let (p, q, r) = (atom(0), atom(1), atom(2));
    let cases: Vec<(Vec<ProofExpr>, ProofExpr)> = vec![
        // (P∧Q) ∨ R, ¬R  ⊨  P   and   ⊨ Q     [encode And]
        (
            vec![or(and(p.clone(), q.clone()), r.clone()), not(r.clone())],
            p.clone(),
        ),
        (
            vec![or(and(p.clone(), q.clone()), r.clone()), not(r.clone())],
            q.clone(),
        ),
        // (P∨Q) forced under an Iff:  (R ↔ (P∨Q)), R, ¬P  ⊨  Q          [encode Or + Iff]
        (
            vec![
                iff(r.clone(), or(p.clone(), q.clone())),
                r.clone(),
                not(p.clone()),
            ],
            q.clone(),
        ),
        // (P→Q) as a disjunct:  (P→Q) ∨ R, ¬R, P  ⊨  Q                 [encode Implies]
        (
            vec![
                or(imp(p.clone(), q.clone()), r.clone()),
                not(r.clone()),
                p.clone(),
            ],
            q.clone(),
        ),
        // Iff with a conjunction side:  (P ↔ (Q∧R)), P  ⊨  Q  and  ⊨ R  [encode Iff + And]
        (
            vec![iff(p.clone(), and(q.clone(), r.clone())), p.clone()],
            q.clone(),
        ),
        (
            vec![iff(p.clone(), and(q.clone(), r.clone())), p.clone()],
            r.clone(),
        ),
    ];
    for (prem, goal) in cases {
        assert!(
            brute_entails(&prem, &goal),
            "self-check: case must truly be entailed"
        );
        assert_eq!(
            cdcl_entails(&prem, &goal),
            Some(true),
            "encode must certify the entailment: premises={prem:?} goal={goal:?}",
        );
    }
}

#[test]
fn cdcl_entails_matches_the_brute_force_oracle_over_random_nested_formulas() {
    let (mut entailed, mut not_entailed) = (0u32, 0u32);
    for seed in 0..4000u64 {
        let mut state = seed.wrapping_mul(0x2545_F491_4F6C_DD1D).wrapping_add(1);
        let n_prem = 1 + (next(&mut state) % 3) as usize;
        let premises: Vec<ProofExpr> = (0..n_prem).map(|_| gen(&mut state, 3)).collect();
        let goal = gen(&mut state, 3);
        let want = brute_entails(&premises, &goal);
        assert_eq!(
            cdcl_entails(&premises, &goal),
            Some(want),
            "seed {seed}: premises={premises:?} goal={goal:?}",
        );
        if want {
            entailed += 1;
        } else {
            not_entailed += 1;
        }
    }
    // The corpus must exercise BOTH verdicts, or it proves nothing about `NotEntailed`.
    assert!(entailed > 100, "too few entailed instances: {entailed}");
    assert!(
        not_entailed > 100,
        "too few non-entailed instances: {not_entailed}"
    );
}
