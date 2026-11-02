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
    assert!(
        sum.is_empty() && sum.degree() == 0,
        "x+¬x cancels to a trivially-true 0 ≥ 0"
    );
}

#[test]
fn multiply_divide_saturate_are_sound() {
    let c = PbConstraint::at_least(&[(0, true), (1, true), (2, false)], 2);
    assert_implied(&[&c], &c.multiply(3), 3);
    assert_implied(&[&c], &c.saturate(), 3);
    // Division rounds up: a constraint with mixed coefficients.
    let d = PbConstraint {
        terms: [(0, (5, true)), (1, (3, true)), (2, (2, true))].into(),
        degree: 6,
    };
    assert_implied(&[&d], &d.divide_round(2), 3);
    assert_implied(&[&d], &d.divide_round(3), 3);
}

#[test]
fn contradiction_detection() {
    let unsat = PbConstraint {
        terms: BTreeMap::new(),
        degree: 1,
    };
    assert!(unsat.is_contradiction(), "0 ≥ 1 is a contradiction");
    let tight = PbConstraint::at_least(&[(0, true), (1, true)], 2);
    assert!(
        !tight.is_contradiction(),
        "x0+x1 ≥ 2 is satisfiable (both true)"
    );
    let over = PbConstraint::at_least(&[(0, true)], 2);
    assert!(
        over.is_contradiction(),
        "x0 ≥ 2 is impossible for a 0/1 variable"
    );
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
        assert!(
            refutation.is_empty(),
            "every x meets its ¬x and cancels — the LHS is empty"
        );
        assert_eq!(refutation.degree(), 1, "the terminal is exactly 0 ≥ 1");
    }
}

fn php_expr(n: usize) -> ProofExpr {
    let holes = n - 1;
    let p = |i: usize, h: usize| ProofExpr::Atom(format!("p_{i}_{h}"));
    let mut clauses = Vec::new();
    for i in 0..n {
        clauses.push(
            (0..holes)
                .map(|h| p(i, h))
                .reduce(|a, b| ProofExpr::Or(Box::new(a), Box::new(b)))
                .unwrap(),
        );
    }
    for h in 0..holes {
        for i in 0..n {
            for j in (i + 1)..n {
                clauses.push(ProofExpr::Not(Box::new(ProofExpr::And(
                    Box::new(p(i, h)),
                    Box::new(p(j, h)),
                ))));
            }
        }
    }
    clauses
        .into_iter()
        .reduce(|a, b| ProofExpr::And(Box::new(a), Box::new(b)))
        .unwrap()
}

fn feasible_expr(n: usize) -> ProofExpr {
    // n items into n slots — SATISFIABLE; must NOT be refuted.
    let q = |i: usize, h: usize| ProofExpr::Atom(format!("q_{i}_{h}"));
    let mut clauses = Vec::new();
    for i in 0..n {
        clauses.push(
            (0..n)
                .map(|h| q(i, h))
                .reduce(|a, b| ProofExpr::Or(Box::new(a), Box::new(b)))
                .unwrap(),
        );
    }
    for h in 0..n {
        for i in 0..n {
            for j in (i + 1)..n {
                clauses.push(ProofExpr::Not(Box::new(ProofExpr::And(
                    Box::new(q(i, h)),
                    Box::new(q(j, h)),
                ))));
            }
        }
    }
    clauses
        .into_iter()
        .reduce(|a, b| ProofExpr::And(Box::new(a), Box::new(b)))
        .unwrap()
}

#[test]
fn cutting_planes_refutes_pairwise_pigeonhole_from_cnf() {
    // The wiring target: a pairwise-encoded PHP CNF (resolution-EXPONENTIAL) is refuted by
    // recovering the at-most-one cardinality from the exclusion cliques and summing — polynomial.
    for n in 2..=10 {
        assert!(
            refute_clausal(&php_expr(n)),
            "cutting planes must refute pairwise PHP({n}) from CNF"
        );
    }
}

#[test]
fn cutting_planes_refutation_is_sound() {
    // Soundness-critical: a SATISFIABLE formula is NEVER refuted, and a non-cardinality formula
    // falls through (false) instead of claiming a contradiction.
    for n in 1..=8 {
        assert!(
            !refute_clausal(&feasible_expr(n)),
            "feasible({n}) must NOT be refuted"
        );
    }
    let lone = ProofExpr::Or(
        Box::new(ProofExpr::Atom("a".into())),
        Box::new(ProofExpr::Atom("b".into())),
    );
    assert!(
        !refute_clausal(&lone),
        "a lone satisfiable clause is not a cutting-plane contradiction"
    );
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
        cs.push(PbConstraint::clause(
            &(0..holes).map(|h| (var(i, h), true)).collect::<Vec<_>>(),
        ));
    }
    for h in 0..holes {
        cs.push(PbConstraint::at_most(
            &(0..n).map(|i| (var(i, h), true)).collect::<Vec<_>>(),
            1,
        ));
    }
    let mut any = false;
    for_all_assignments(n * holes, |a| {
        if cs.iter().all(|c| c.is_satisfied(a)) {
            any = true;
        }
    });
    assert!(
        !any,
        "PHP(3,2) is UNSAT — no assignment satisfies all constraints"
    );
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
        let swapped = |v: usize| {
            a(if v == 0 {
                1
            } else if v == 1 {
                0
            } else {
                v
            })
        };
        assert_eq!(
            weighted.is_satisfied(a),
            weighted.is_satisfied(&swapped),
            "the x0↔x1 swap preserves satisfaction"
        );
    });

    // Distinct coefficients ⇒ no coefficient symmetry.
    let distinct = PbConstraint::new_weighted(&[(0, 1, true), (1, 2, true), (2, 3, true)], 3);
    assert!(
        coeff_symmetry_generators(3, &[distinct]).is_empty(),
        "all-distinct coefficients: no symmetry"
    );

    // Across a SYSTEM: a variable's profile is its coefficient in EVERY constraint. Here x0,x1 share
    // (2 in C₁, 1 in C₂) while x2 differs in C₂, so only x0,x1 remain interchangeable.
    let c1 = PbConstraint::new_weighted(&[(0, 2, true), (1, 2, true), (2, 2, true)], 3);
    let c2 = PbConstraint::new_weighted(&[(0, 1, true), (1, 1, true), (2, 5, true)], 2);
    let sysg = coeff_symmetry_generators(3, &[c1.clone(), c2.clone()]);
    assert_eq!(
        sysg,
        vec![vec![1, 0, 2]],
        "only x0 ↔ x1 survives both constraints"
    );
    assert!(sysg
        .iter()
        .all(|g| is_pb_symmetry(&[c1.clone(), c2.clone()], g)));
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
        assert_eq!(
            free.len(),
            1,
            "each reason is unit under {{x0=false}}: {c:?}"
        );
        assert!(
            free[0].is_positive(),
            "the forced literal is x_i true: {c:?}"
        );
    }
    // x0 = false ∧ x1 = false ⇒ at most one can be true < 2 ⇒ conflict over the two false literals.
    let conf = th.propagate(&[Lit::new(0, false), Lit::new(1, false)]);
    assert!(!conf.is_empty(), "must conflict; got {conf:?}");
    assert!(
        conf.iter().any(|c| !c.is_empty()
            && c.iter()
                .all(|l| (l.var() == 0 || l.var() == 1) && l.is_positive())),
        "a conflict clause of the two now-false literals; got {conf:?}"
    );
}

/// DIAGNOSTIC: the cardinality theory ALONE, with no Boolean clauses, must refute an infeasible system
/// (`≥ 3` and `≤ 1` of three variables) — i.e. the engine's pure-theory `solve_with` drives a theory
/// conflict all the way to UNSAT.
#[test]
fn cardinality_theory_alone_refutes_an_infeasible_system() {
    use crate::cdcl::{SolveResult, Solver, Theory};
    let lits = [(0usize, true), (1, true), (2, true)];
    let card = vec![
        PbConstraint::at_least(&lits, 3),
        PbConstraint::at_most(&lits, 1),
    ];
    let mut s = Solver::new(3);
    let mut t: Vec<Box<dyn Theory>> = vec![Box::new(CardinalityTheory::new(3, &card))];
    assert!(
        matches!(s.solve_with(&mut t), SolveResult::Unsat),
        "≥3 ∧ ≤1 of three is UNSAT"
    );
}

/// DIAGNOSTIC: `IncXor` alone, no Boolean clauses, must refute an inconsistent linear system
/// (`x0 ⊕ x1 = 0 ∧ x0 ⊕ x1 = 1`) — isolating whether pure-theory refutation works for the parity engine.
#[test]
fn incxor_alone_refutes_an_inconsistent_system() {
    use crate::cdcl::{SolveResult, Solver, Theory};
    use crate::xor_engine::IncXor;
    use crate::xorsat::XorEquation;
    let xor = vec![
        XorEquation::new(vec![0, 1], false),
        XorEquation::new(vec![0, 1], true),
    ];
    let mut s = Solver::new(2);
    let mut t: Vec<Box<dyn Theory>> = vec![Box::new(IncXor::new(2, &xor))];
    assert!(
        matches!(s.solve_with(&mut t), SolveResult::Unsat),
        "x0⊕x1=0 ∧ x0⊕x1=1 is UNSAT"
    );
}

/// **The headline fusion.** Parity (`x0 ⊕ x1 ⊕ x2 = 1`, odd) ∧ exactly-two-true is UNSAT (two is
/// even), yet NEITHER theory alone refutes it — parity alone is SAT (one true), cardinality alone is
/// SAT (any two). Only `IncXor` and `CardinalityTheory` reasoning *together* on the shared trail close
/// it, with no Boolean clauses at all.
#[test]
fn fused_parity_and_cardinality_is_unsat_though_neither_alone_is() {
    use crate::cdcl::{SolveResult, Solver, Theory};
    use crate::xor_engine::XorEngine;
    use crate::xorsat::XorEquation;
    // The parity theory is the stateless `XorEngine` (the GF(2) correctness oracle), not the
    // incremental `IncXor`: `IncXor`'s trail-sync matches by variable but not value, so a
    // backtrack-then-flip leaves its matrix stale — harmless when a clausal XOR encoding backs it,
    // unsound in pure-theory fusion (see the docs on `CardinalityTheory`).
    let xor = vec![XorEquation::new(vec![0, 1, 2], true)];
    let lits = [(0usize, true), (1, true), (2, true)];
    let card = vec![
        PbConstraint::at_least(&lits, 2),
        PbConstraint::at_most(&lits, 2),
    ];

    let mut s1 = Solver::new(3);
    let mut t1: Vec<Box<dyn Theory>> = vec![Box::new(XorEngine::new(3, &xor))];
    assert!(
        matches!(s1.solve_with(&mut t1), SolveResult::Sat(_)),
        "parity alone is SAT"
    );

    let mut s2 = Solver::new(3);
    let mut t2: Vec<Box<dyn Theory>> = vec![Box::new(CardinalityTheory::new(3, &card))];
    assert!(
        matches!(s2.solve_with(&mut t2), SolveResult::Sat(_)),
        "exactly-two alone is SAT"
    );

    let mut s = Solver::new(3);
    let mut fused: Vec<Box<dyn Theory>> = vec![
        Box::new(XorEngine::new(3, &xor)),
        Box::new(CardinalityTheory::new(3, &card)),
    ];
    assert!(
        matches!(s.solve_with(&mut fused), SolveResult::Unsat),
        "odd-parity ∧ exactly-two is UNSAT"
    );
}

/// The consistent twin: even parity ∧ exactly-two-true IS satisfiable, and the fused solver returns a
/// model that genuinely has even parity and exactly two true.
#[test]
fn fused_parity_and_cardinality_sat_model_is_valid() {
    use crate::cdcl::{SolveResult, Solver, Theory};
    use crate::xor_engine::XorEngine;
    use crate::xorsat::XorEquation;
    let xor = vec![XorEquation::new(vec![0, 1, 2], false)];
    let lits = [(0usize, true), (1, true), (2, true)];
    let card = vec![
        PbConstraint::at_least(&lits, 2),
        PbConstraint::at_most(&lits, 2),
    ];
    let mut s = Solver::new(3);
    let mut fused: Vec<Box<dyn Theory>> = vec![
        Box::new(XorEngine::new(3, &xor)),
        Box::new(CardinalityTheory::new(3, &card)),
    ];
    match s.solve_with(&mut fused) {
        SolveResult::Sat(m) => {
            assert_eq!(
                m.iter().filter(|&&b| b).count(),
                2,
                "exactly two true: {m:?}"
            );
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
    use crate::cdcl::{Lit, SolveResult, Solver, Theory};
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
            let lits: Vec<(usize, bool)> = (0..n)
                .filter_map(|v| (rng() % 2 == 0).then(|| (v, rng() % 2 == 0)))
                .collect();
            if lits.len() < 2 {
                continue;
            }
            let k = (rng() % (lits.len() as u64 + 1)) as i64;
            pbs.push(if rng() % 2 == 0 {
                PbConstraint::at_least(&lits, k)
            } else {
                PbConstraint::at_most(&lits, k)
            });
        }
        let mut clauses: Vec<Vec<Lit>> = Vec::new();
        for _ in 0..(rng() % 3) {
            let c: Vec<Lit> = (0..n)
                .filter_map(|v| (rng() % 2 == 0).then(|| Lit::new(v as u32, rng() % 2 == 0)))
                .collect();
            if !c.is_empty() {
                clauses.push(c);
            }
        }

        let xor_ok = |x: u64| {
            xor_specs.iter().all(|(vars, rhs)| {
                (vars.iter().filter(|&&v| (x >> v) & 1 == 1).count() % 2 == 1) == *rhs
            })
        };
        let pb_ok = |x: u64| {
            pbs.iter().all(|pb| {
                let sum: i64 = pb
                    .terms()
                    .map(|(v, c, s)| if (((x >> v) & 1 == 1) == s) { c } else { 0 })
                    .sum();
                sum >= pb.degree()
            })
        };
        let cl_ok = |x: u64| {
            clauses.iter().all(|c| {
                c.iter()
                    .any(|l| ((x >> l.var()) & 1 == 1) == l.is_positive())
            })
        };
        let brute_sat = (0u64..(1u64 << n)).any(|x| xor_ok(x) && pb_ok(x) && cl_ok(x));

        let xeqs: Vec<XorEquation> = xor_specs
            .iter()
            .map(|(v, r)| XorEquation::new(v.clone(), *r))
            .collect();
        let mut s = Solver::new(n);
        for c in &clauses {
            s.add_clause(c.clone());
        }
        let mut theories: Vec<Box<dyn Theory>> = vec![
            Box::new(XorEngine::new(n, &xeqs)),
            Box::new(CardinalityTheory::new(n, &pbs)),
        ];
        let got = s.solve_with(&mut theories);
        assert_eq!(
            matches!(got, SolveResult::Sat(_)),
            brute_sat,
            "fused verdict must match brute force (n={n}, xor={xor_specs:?}, pbs={pbs:?}, clauses={clauses:?})"
        );
        if let SolveResult::Sat(m) = got {
            let x = (0..n).fold(0u64, |acc, v| acc | ((m[v] as u64) << v));
            assert!(
                xor_ok(x) && pb_ok(x) && cl_ok(x),
                "the fused model must satisfy every constraint: {m:?}"
            );
        }
    }
}
