use super::*;

fn eq(vars: &[usize], rhs: bool) -> XorEquation {
    XorEquation::new(vars.to_vec(), rhs)
}

/// CNF-encode a list of XOR equations into a `ProofExpr` over atoms `x{v}`: each equation becomes
/// its `2^{k-1}` wrong-parity clauses (the inverse of [`refute_via_parity`]'s recognition).
fn xor_system_to_expr(system: &[(Vec<usize>, bool)]) -> crate::ProofExpr {
    use crate::ProofExpr;
    let lit = |v: usize, positive: bool| {
        let a = ProofExpr::Atom(format!("x{v}"));
        if positive {
            a
        } else {
            ProofExpr::Not(Box::new(a))
        }
    };
    let mut clauses = Vec::new();
    for (vars, rhs) in system {
        let k = vars.len();
        for mask in 0u32..(1 << k) {
            let parity = mask.count_ones() % 2 == 1;
            if parity != *rhs {
                let mut it = vars.iter().enumerate();
                let (i0, &v0) = it.next().unwrap();
                let first = lit(v0, mask & (1 << i0) == 0);
                let clause = it.fold(first, |acc, (i, &v)| {
                    ProofExpr::Or(Box::new(acc), Box::new(lit(v, mask & (1 << i) == 0)))
                });
                clauses.push(clause);
            }
        }
    }
    let mut it = clauses.into_iter();
    let first = it.next().expect("non-empty system");
    it.fold(first, |acc, c| {
        crate::ProofExpr::And(Box::new(acc), Box::new(c))
    })
}

/// The GF(2) shadow, stacked into the default prover: an inconsistent parity cover that resolution
/// refutes only exponentially is now decided in polynomial time by Gaussian elimination — recovered
/// straight from the CNF, with no CDCL search, and `prove_unsat` returns a certified `Refuted`.
#[test]
fn parity_shadow_refutes_inconsistent_xor_through_prove_unsat() {
    // x0⊕x1=1, x1⊕x2=1, x0⊕x2=1 — the three sum to 0 = 1, an odd cycle: UNSAT.
    let system = vec![(vec![0, 1], true), (vec![1, 2], true), (vec![0, 2], true)];
    let e = xor_system_to_expr(&system);
    assert!(
        refute_via_parity(&e),
        "the parity shadow must recognize and refute the odd cycle"
    );
    assert_eq!(
        crate::sat::prove_unsat(&e),
        crate::sat::UnsatOutcome::Refuted,
        "prove_unsat now routes the parity cover to Gaussian elimination"
    );
}

/// Soundness: a *satisfiable* parity cover is never falsely refuted — the shadow declines and the
/// prover finds a model.
#[test]
fn parity_shadow_is_sound_on_satisfiable_xor() {
    // x0⊕x1=1, x1⊕x2=1 — consistent (x0=0,x1=1,x2=0).
    let system = vec![(vec![0, 1], true), (vec![1, 2], true)];
    let e = xor_system_to_expr(&system);
    assert!(
        !refute_via_parity(&e),
        "a satisfiable parity system must not be refuted"
    );
    assert!(
        matches!(
            crate::sat::prove_unsat(&e),
            crate::sat::UnsatOutcome::Sat(_)
        ),
        "prove_unsat must find a model for the satisfiable parity cover"
    );
}

/// The shadows do not interfere: pigeonhole is a *counting* obstruction, not a parity one, so the
/// GF(2) recognizer declines it (leaving the counting shadow to decide it). No gadget, no guess.
#[test]
fn parity_shadow_declines_non_parity_pigeonhole() {
    use crate::cdcl::Lit;
    use crate::ProofExpr;
    let (cnf, _) = crate::families::php(4);
    let clause_expr = |c: &[Lit]| {
        let mut it = c.iter().map(|l| {
            let a = ProofExpr::Atom(format!("x{}", l.var()));
            if l.is_positive() {
                a
            } else {
                ProofExpr::Not(Box::new(a))
            }
        });
        let first = it.next().expect("non-empty clause");
        it.fold(first, |acc, l| ProofExpr::Or(Box::new(acc), Box::new(l)))
    };
    let mut it = cnf.clauses.iter().map(|c| clause_expr(c));
    let first = it.next().unwrap();
    let e = it.fold(first, |acc, c| ProofExpr::And(Box::new(acc), Box::new(c)));
    assert!(
        !refute_via_parity(&e),
        "pigeonhole is not a parity cover — the GF(2) shadow declines"
    );
}

#[test]
fn simple_consistent_system_is_solved() {
    // x0 ⊕ x1 = 1, x1 = 1  ⇒  x1 = 1, x0 = 0.
    let sys = vec![eq(&[0, 1], true), eq(&[1], true)];
    match solve(&sys, 2) {
        XorOutcome::Sat(a) => {
            assert!(satisfies(&sys, &a), "assignment must satisfy: {a:?}");
            assert_eq!(a, vec![false, true]);
        }
        o => panic!("expected Sat, got {o:?}"),
    }
}

#[test]
fn direct_contradiction_is_refuted() {
    // x0 ⊕ x1 = 0 and x0 ⊕ x1 = 1 — summing them gives 0 = 1.
    let sys = vec![eq(&[0, 1], false), eq(&[0, 1], true)];
    match solve(&sys, 2) {
        XorOutcome::Unsat(r) => {
            assert!(
                is_refutation(&sys, 2, &r),
                "refutation must re-check: {r:?}"
            );
            assert_eq!(r.len(), 2, "both equations are needed");
        }
        o => panic!("expected Unsat, got {o:?}"),
    }
}

#[test]
fn parity_chain_summing_to_one_is_refuted() {
    // x_i ⊕ x_{i+1} = 0 for a chain, plus x0 ⊕ x_{n-1} = 1 — all equal yet endpoints differ.
    let n = 8;
    let mut sys: Vec<XorEquation> = (0..n - 1).map(|i| eq(&[i, i + 1], false)).collect();
    sys.push(eq(&[0, n - 1], true));
    match solve(&sys, n) {
        XorOutcome::Unsat(r) => assert!(is_refutation(&sys, n, &r), "refutation invalid: {r:?}"),
        o => panic!("inconsistent chain must be Unsat, got {o:?}"),
    }
}

#[test]
fn duplicate_variables_cancel() {
    // x0 ⊕ x0 ⊕ x1 = 1  ≡  x1 = 1.
    let sys = vec![eq(&[0, 0, 1], true)];
    match solve(&sys, 2) {
        XorOutcome::Sat(a) => assert!(a[1], "x1 must be true: {a:?}"),
        o => panic!("expected Sat, got {o:?}"),
    }
}

#[test]
fn empty_system_is_trivially_sat() {
    assert!(matches!(solve(&[], 3), XorOutcome::Sat(_)));
}

#[test]
fn matches_brute_force_on_random_systems() {
    // Independent oracle: enumerate all 2^num_vars assignments; the system is SAT iff some
    // assignment satisfies every equation. Cross-check verdict + re-check every witness.
    let mut s: u64 = 0xD1B54A32D192ED03;
    let mut next = || {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        s
    };
    for _ in 0..400 {
        let num_vars = (next() % 6) as usize + 1; // 1..=6
        let m = (next() % 8) as usize + 1; // 1..=8 equations
        let sys: Vec<XorEquation> = (0..m)
            .map(|_| {
                let vars: Vec<usize> = (0..num_vars).filter(|_| next() % 2 == 0).collect();
                eq(&vars, next() % 2 == 0)
            })
            .collect();
        let brute_sat = (0..(1u32 << num_vars)).any(|mask| {
            let a: Vec<bool> = (0..num_vars).map(|i| (mask >> i) & 1 == 1).collect();
            satisfies(&sys, &a)
        });
        match solve(&sys, num_vars) {
            XorOutcome::Sat(a) => {
                assert!(brute_sat, "we said SAT but brute force says UNSAT: {sys:?}");
                assert!(satisfies(&sys, &a), "returned assignment is wrong: {a:?}");
            }
            XorOutcome::Unsat(r) => {
                assert!(
                    !brute_sat,
                    "we said UNSAT but brute force found a model: {sys:?}"
                );
                assert!(is_refutation(&sys, num_vars, &r), "bogus refutation {r:?}");
            }
        }
    }
}

#[test]
fn a_bad_refutation_is_rejected() {
    let sys = vec![eq(&[0, 1], false), eq(&[0, 1], true)];
    assert!(!is_refutation(&sys, 2, &[]), "empty is not a refutation");
    assert!(
        !is_refutation(&sys, 2, &[0]),
        "one consistent equation is not 0=1"
    );
    assert!(is_refutation(&sys, 2, &[0, 1]), "the pair sums to 0=1");
}
