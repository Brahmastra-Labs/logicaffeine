//! Phase 81: Full Reduction (The Calculator)
//!
//! Teaches the Proof Engine to compute like the Kernel.
//! - Iota Reduction: match (Ctor args) with arms → selected arm
//! - Fix Unfolding: (fix f. body) ctor_arg → body[f := fix] ctor_arg

use logos::proof::{BackwardChainer, InferenceRule, MatchArm, ProofExpr, ProofTerm};

// =============================================================================
// HELPER FUNCTIONS
// =============================================================================

fn zero() -> ProofExpr {
    ProofExpr::Ctor {
        name: "Zero".into(),
        args: vec![],
    }
}

fn succ(n: ProofExpr) -> ProofExpr {
    ProofExpr::Ctor {
        name: "Succ".into(),
        args: vec![n],
    }
}

fn var(name: &str) -> ProofExpr {
    ProofExpr::Atom(name.into())
}

fn expr_to_term(expr: ProofExpr) -> ProofTerm {
    match expr {
        ProofExpr::Atom(s) => ProofTerm::Variable(s),
        ProofExpr::Ctor { name, args } => {
            ProofTerm::Function(name, args.into_iter().map(expr_to_term).collect())
        }
        ProofExpr::Predicate { name, args, .. } => ProofTerm::Function(name, args),
        _ => ProofTerm::Constant(format!("{}", expr)),
    }
}

fn app(name: &str, args: Vec<ProofExpr>) -> ProofExpr {
    ProofExpr::Predicate {
        name: name.into(),
        args: args.into_iter().map(expr_to_term).collect(),
        world: None,
    }
}

fn eq(lhs: ProofExpr, rhs: ProofExpr) -> ProofExpr {
    ProofExpr::Identity(expr_to_term(lhs), expr_to_term(rhs))
}

// =============================================================================
// IOTA REDUCTION TESTS
// =============================================================================

#[test]
fn test_iota_reduction_zero_case() {
    // match Zero with | Zero => Zero | Succ k => k
    // Should reduce to: Zero
    use logos::proof::unify::beta_reduce;

    let match_expr = ProofExpr::Match {
        scrutinee: Box::new(zero()),
        arms: vec![
            MatchArm {
                ctor: "Zero".into(),
                bindings: vec![],
                body: zero(),
            },
            MatchArm {
                ctor: "Succ".into(),
                bindings: vec!["k".into()],
                body: var("k"),
            },
        ],
    };

    let reduced = beta_reduce(&match_expr);

    // Should reduce to Zero, not remain a Match
    assert!(
        matches!(reduced, ProofExpr::Ctor { ref name, .. } if name == "Zero"),
        "Expected Zero after iota reduction, got: {}",
        reduced
    );
}

#[test]
fn test_iota_reduction_succ_case() {
    // match Succ(Zero) with | Zero => Zero | Succ k => k
    // Should reduce to: Zero (the bound 'k')
    use logos::proof::unify::beta_reduce;

    let match_expr = ProofExpr::Match {
        scrutinee: Box::new(succ(zero())),
        arms: vec![
            MatchArm {
                ctor: "Zero".into(),
                bindings: vec![],
                body: zero(),
            },
            MatchArm {
                ctor: "Succ".into(),
                bindings: vec!["k".into()],
                body: var("k"),
            },
        ],
    };

    let reduced = beta_reduce(&match_expr);

    // Should reduce to Zero (the predecessor of Succ(Zero))
    assert!(
        matches!(reduced, ProofExpr::Ctor { ref name, .. } if name == "Zero"),
        "Expected Zero after iota reduction, got: {}",
        reduced
    );
}

#[test]
fn test_iota_reduction_nested() {
    // match Succ(Succ(Zero)) with | Zero => Zero | Succ k => k
    // Should reduce to: Succ(Zero)
    use logos::proof::unify::beta_reduce;

    let match_expr = ProofExpr::Match {
        scrutinee: Box::new(succ(succ(zero()))),
        arms: vec![
            MatchArm {
                ctor: "Zero".into(),
                bindings: vec![],
                body: zero(),
            },
            MatchArm {
                ctor: "Succ".into(),
                bindings: vec!["k".into()],
                body: var("k"),
            },
        ],
    };

    let reduced = beta_reduce(&match_expr);

    // Should reduce to Succ(Zero) (the predecessor of Succ(Succ(Zero)))
    assert!(
        matches!(reduced, ProofExpr::Ctor { ref name, .. } if name == "Succ"),
        "Expected Succ after iota reduction, got: {}",
        reduced
    );
}

// =============================================================================
// FIX UNFOLDING TESTS
// =============================================================================

#[test]
fn test_fix_unfolds_on_constructor() {
    // pred = fix f. λn. match n with Zero → Zero | Succ k → k
    // Applied to Succ(Zero) should unfold and reduce to Zero
    use logos::proof::unify::beta_reduce;

    let pred_fix = ProofExpr::Fixpoint {
        name: "f".into(),
        body: Box::new(ProofExpr::Lambda {
            variable: "n".into(),
            body: Box::new(ProofExpr::Match {
                scrutinee: Box::new(var("n")),
                arms: vec![
                    MatchArm {
                        ctor: "Zero".into(),
                        bindings: vec![],
                        body: zero(),
                    },
                    MatchArm {
                        ctor: "Succ".into(),
                        bindings: vec!["k".into()],
                        body: var("k"),
                    },
                ],
            }),
        }),
    };

    // Apply pred to Succ(Zero)
    let app_expr = ProofExpr::App(Box::new(pred_fix), Box::new(succ(zero())));
    let reduced = beta_reduce(&app_expr);

    // pred(Succ(Zero)) = Zero
    assert!(
        matches!(reduced, ProofExpr::Ctor { ref name, .. } if name == "Zero"),
        "Expected Zero after fix unfolding, got: {}",
        reduced
    );
}

#[test]
fn test_fix_does_not_unfold_on_variable() {
    // Fix should NOT unfold when applied to a non-constructor (prevents divergence)
    use logos::proof::unify::beta_reduce;

    let pred_fix = ProofExpr::Fixpoint {
        name: "f".into(),
        body: Box::new(ProofExpr::Lambda {
            variable: "n".into(),
            body: Box::new(ProofExpr::Match {
                scrutinee: Box::new(var("n")),
                arms: vec![
                    MatchArm {
                        ctor: "Zero".into(),
                        bindings: vec![],
                        body: zero(),
                    },
                    MatchArm {
                        ctor: "Succ".into(),
                        bindings: vec!["k".into()],
                        body: var("k"),
                    },
                ],
            }),
        }),
    };

    // Apply pred to a variable (not a constructor)
    let app_expr = ProofExpr::App(Box::new(pred_fix), Box::new(var("x")));
    let reduced = beta_reduce(&app_expr);

    // Should remain as an application (not unfold infinitely)
    assert!(
        matches!(reduced, ProofExpr::App(..)),
        "Expected App to remain when argument is not a constructor, got: {}",
        reduced
    );
}

// =============================================================================
// PROOF BY COMPUTATION (The Main Test)
// =============================================================================

#[test]
fn test_prove_zero_plus_n_is_n() {
    // Simpler: Add(Zero, n) = n should follow directly from the axiom
    let mut engine = BackwardChainer::new();

    engine.add_axiom(eq(app("Add", vec![zero(), var("n")]), var("n")));

    let goal = eq(app("Add", vec![zero(), var("x")]), var("x"));

    let result = engine.prove(goal);
    assert!(
        result.is_ok(),
        "Should prove Add(Zero, x) = x by axiom instantiation"
    );
}

#[test]
fn test_prove_one_plus_one_is_two() {
    // The engine should prove: Add(Succ(Zero), Succ(Zero)) = Succ(Succ(Zero))
    // By using equational rewriting with the Add axioms

    let mut engine = BackwardChainer::new();
    engine.set_max_depth(20);

    // Axiom 1: Add(Zero, n) = n
    engine.add_axiom(eq(app("Add", vec![zero(), var("n")]), var("n")));

    // Axiom 2: Add(Succ(k), n) = Succ(Add(k, n))
    engine.add_axiom(eq(
        app("Add", vec![succ(var("k")), var("n")]),
        succ(app("Add", vec![var("k"), var("n")])),
    ));

    // Goal: 1 + 1 = 2  (i.e., Add(Succ(Zero), Succ(Zero)) = Succ(Succ(Zero)))
    let one = succ(zero());
    let two = succ(succ(zero()));
    let goal = eq(app("Add", vec![one.clone(), one]), two);

    let result = engine.prove(goal);

    assert!(result.is_ok(), "Should prove 1 + 1 = 2 by computation");

    let proof = result.unwrap();
    println!("Proof of 1 + 1 = 2:");
    println!("{}", proof.display_tree());
}

#[test]
fn test_prove_two_plus_one_is_three() {
    // Slightly harder: 2 + 1 = 3

    let mut engine = BackwardChainer::new();
    engine.set_max_depth(25);

    // Add axioms
    engine.add_axiom(eq(app("Add", vec![zero(), var("n")]), var("n")));
    engine.add_axiom(eq(
        app("Add", vec![succ(var("k")), var("n")]),
        succ(app("Add", vec![var("k"), var("n")])),
    ));

    // Goal: 2 + 1 = 3
    let one = succ(zero());
    let two = succ(succ(zero()));
    let three = succ(succ(succ(zero())));
    let goal = eq(app("Add", vec![two, one]), three);

    let result = engine.prove(goal);

    assert!(result.is_ok(), "Should prove 2 + 1 = 3 by computation");
}

// =============================================================================
// REFLEXIVITY BY COMPUTATION
// =============================================================================

#[test]
fn test_reflexivity_simple() {
    // Zero = Zero should be provable by reflexivity
    let mut engine = BackwardChainer::new();

    let goal = eq(zero(), zero());

    let result = engine.prove(goal);
    assert!(result.is_ok(), "Should prove Zero = Zero by reflexivity");

    let proof = result.unwrap();
    assert!(
        matches!(proof.rule, InferenceRule::Reflexivity),
        "Expected Reflexivity rule, got: {:?}",
        proof.rule
    );
}

#[test]
fn test_reflexivity_nested_ctor() {
    // Succ(Succ(Zero)) = Succ(Succ(Zero)) should be reflexivity
    let mut engine = BackwardChainer::new();

    let two = succ(succ(zero()));
    let goal = eq(two.clone(), two);

    let result = engine.prove(goal);
    assert!(
        result.is_ok(),
        "Should prove Succ(Succ(Zero)) = Succ(Succ(Zero)) by reflexivity"
    );
}
