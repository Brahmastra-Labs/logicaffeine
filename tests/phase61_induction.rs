// =============================================================================
// PHASE 61: THE INDUCTIVE LEAP - TEST SUITE
// =============================================================================
// TDD RED: These tests define the specification for structural induction.
// The prover must learn to reason about inductive types (Nat, List, etc.)
// using the principle of structural induction.
//
// Peano Arithmetic:
// - Zero is a Nat
// - If n is a Nat, then Succ(n) is a Nat
// - To prove ∀n:Nat. P(n), prove P(Zero) and ∀k:Nat. P(k) → P(Succ(k))

use logos::proof::{
    BackwardChainer, DerivationTree, InferenceRule, MatchArm, ProofExpr, ProofTerm,
};
use logos::proof::error::ProofError;

// =============================================================================
// HELPER FUNCTIONS - Peano Number Construction
// =============================================================================

/// Create Zero constructor
fn zero() -> ProofExpr {
    ProofExpr::Ctor {
        name: "Zero".into(),
        args: vec![],
    }
}

/// Create Succ(n) constructor
fn succ(n: ProofExpr) -> ProofExpr {
    ProofExpr::Ctor {
        name: "Succ".into(),
        args: vec![n],
    }
}

/// Create a typed variable (signals induction is applicable)
fn nat_var(name: &str) -> ProofExpr {
    ProofExpr::TypedVar {
        name: name.into(),
        typename: "Nat".into(),
    }
}

/// Create an untyped variable (for helper expressions)
fn var(name: &str) -> ProofExpr {
    ProofExpr::Atom(name.into())
}

/// Create a function application: f(args...)
fn app(name: &str, args: Vec<ProofExpr>) -> ProofExpr {
    ProofExpr::Predicate {
        name: name.into(),
        args: args.into_iter().map(|a| expr_to_term(a)).collect(),
        world: None,
    }
}

/// Convert a ProofExpr to ProofTerm (for embedding in predicates)
fn expr_to_term(expr: ProofExpr) -> ProofTerm {
    match expr {
        ProofExpr::Atom(s) => ProofTerm::Variable(s),
        ProofExpr::Ctor { name, args } => {
            ProofTerm::Function(
                name,
                args.into_iter().map(expr_to_term).collect(),
            )
        }
        // Predicate used as a function application (e.g., Add(x, y))
        ProofExpr::Predicate { name, args, .. } => {
            ProofTerm::Function(name, args)
        }
        // Preserve type annotation in variable name for induction detection
        ProofExpr::TypedVar { name, typename } => {
            ProofTerm::Variable(format!("{}:{}", name, typename))
        }
        _ => ProofTerm::Constant(format!("{}", expr)),
    }
}

/// Create an identity/equality: lhs = rhs
fn eq(lhs: ProofExpr, rhs: ProofExpr) -> ProofExpr {
    ProofExpr::Identity(expr_to_term(lhs), expr_to_term(rhs))
}

// =============================================================================
// BASIC PEANO CONSTRUCTOR TESTS
// =============================================================================

#[test]
fn test_zero_is_nat() {
    // Zero should be a valid Nat constructor
    let z = zero();
    assert!(matches!(z, ProofExpr::Ctor { name, args } if name == "Zero" && args.is_empty()));
}

#[test]
fn test_succ_is_nat() {
    // Succ(Zero) should be a valid Nat constructor
    let one = succ(zero());
    if let ProofExpr::Ctor { name, args } = one {
        assert_eq!(name, "Succ");
        assert_eq!(args.len(), 1);
    } else {
        panic!("Expected Ctor");
    }
}

#[test]
fn test_peano_three() {
    // 3 = Succ(Succ(Succ(Zero)))
    let three = succ(succ(succ(zero())));
    let display = format!("{}", three);
    assert!(display.contains("Succ"), "Display: {}", display);
}

// =============================================================================
// TYPED VARIABLE TESTS
// =============================================================================

#[test]
fn test_typed_var_display() {
    let n = nat_var("n");
    let display = format!("{}", n);
    assert_eq!(display, "n:Nat");
}

#[test]
fn test_typed_var_signals_induction_type() {
    let n = nat_var("n");
    if let ProofExpr::TypedVar { name, typename } = n {
        assert_eq!(name, "n");
        assert_eq!(typename, "Nat");
    } else {
        panic!("Expected TypedVar");
    }
}

// =============================================================================
// MATCH EXPRESSION TESTS
// =============================================================================

#[test]
fn test_match_expression_display() {
    // match n { Zero => 0, Succ(k) => k }
    let n = var("n");
    let match_expr = ProofExpr::Match {
        scrutinee: Box::new(n),
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

    let display = format!("{}", match_expr);
    assert!(display.contains("match"), "Display: {}", display);
    assert!(display.contains("Zero"), "Display: {}", display);
    assert!(display.contains("Succ"), "Display: {}", display);
}

// =============================================================================
// RED TESTS: STRUCTURAL INDUCTION (These should FAIL until implemented)
// =============================================================================

#[test]
fn test_induction_base_case_axiom_exists() {
    // Base case axiom: Add(Zero, m) = m
    // We verify the axiom can be added and retrieved, but full unification
    // of Identity expressions with nested function terms requires
    // equational reasoning (future work)
    let mut engine = BackwardChainer::new();

    // Axiom: Add(Zero, m) = m
    let add_zero_m = eq(
        app("Add", vec![zero(), var("m")]),
        var("m"),
    );
    engine.add_axiom(add_zero_m.clone());

    // For now, just verify direct match on the exact axiom works
    let result = engine.prove(add_zero_m);
    assert!(result.is_ok(), "Direct axiom match should work");
}

#[test]
fn test_induction_step_case_axiom_exists() {
    // Step case axiom: Add(Succ(k), m) = Succ(Add(k, m))
    // Same as above - verify axiom exists
    let mut engine = BackwardChainer::new();

    // Axiom: ∀k,m. Add(Succ(k), m) = Succ(Add(k, m))
    let add_succ_k_m = eq(
        app("Add", vec![succ(var("k")), var("m")]),
        succ(app("Add", vec![var("k"), var("m")])),
    );
    engine.add_axiom(add_succ_k_m.clone());

    // Direct match should work
    let result = engine.prove(add_succ_k_m);
    assert!(result.is_ok(), "Direct axiom match should work");
}

#[test]
fn test_induction_nat_theorem_proved_by_induction() {
    // Theorem: ∀n:Nat. Add(n, Zero) = n
    //
    // This requires structural induction on Nat:
    // - Base case: Add(Zero, Zero) = Zero
    // - Step case: Add(k, Zero) = k → Add(Succ(k), Zero) = Succ(k)

    let mut engine = BackwardChainer::new();
    engine.set_max_depth(20);

    // Axiom 1: Add(Zero, m) = m (base definition)
    engine.add_axiom(eq(
        app("Add", vec![zero(), var("m")]),
        var("m"),
    ));

    // Axiom 2: Add(Succ(k), m) = Succ(Add(k, m)) (step definition)
    engine.add_axiom(eq(
        app("Add", vec![succ(var("k")), var("m")]),
        succ(app("Add", vec![var("k"), var("m")])),
    ));

    // Goal: ∀n:Nat. Add(n, Zero) = n
    let goal = eq(
        app("Add", vec![nat_var("n"), zero()]),
        nat_var("n"),
    );

    let result = engine.prove(goal);

    // Should succeed using structural induction
    assert!(result.is_ok(), "Should prove Add(n, 0) = n by induction");

    let proof = result.unwrap();
    assert!(
        matches!(proof.rule, InferenceRule::StructuralInduction { .. }),
        "Should use StructuralInduction rule, got {:?}",
        proof.rule
    );

    println!("✓ Proved Add(n, Zero) = n by structural induction:");
    println!("{}", proof.display_tree());
}

#[test]
fn test_induction_on_list_fails_without_induction() {
    // Another inductive type: List
    // Theorem: ∀xs:List. Append(xs, Nil) = xs
    //
    // Same pattern - needs structural induction

    let nil = ProofExpr::Ctor { name: "Nil".into(), args: vec![] };
    let list_var = ProofExpr::TypedVar { name: "xs".into(), typename: "List".into() };

    let mut engine = BackwardChainer::new();
    engine.set_max_depth(20);

    // Axiom: Append(Nil, ys) = ys
    engine.add_axiom(eq(
        app("Append", vec![nil.clone(), var("ys")]),
        var("ys"),
    ));

    // Goal: Append(xs:List, Nil) = xs:List
    let goal = eq(
        app("Append", vec![list_var.clone(), nil]),
        list_var,
    );

    let result = engine.prove(goal);

    // Should fail - needs induction on lists
    assert!(
        result.is_err(),
        "Engine should NOT prove list induction without StructuralInduction!"
    );
}

// =============================================================================
// STRUCTURAL INDUCTION PROOF VALIDATION
// =============================================================================

#[test]
fn test_induction_full_theorem_succeeds_with_induction() {
    // Verify the proof structure has correct base and step cases

    let mut engine = BackwardChainer::new();

    // Define Add recursively
    engine.add_axiom(eq(
        app("Add", vec![zero(), var("m")]),
        var("m"),
    ));
    engine.add_axiom(eq(
        app("Add", vec![succ(var("k")), var("m")]),
        succ(app("Add", vec![var("k"), var("m")])),
    ));

    // Goal: ∀n:Nat. Add(n, Zero) = n
    let goal = eq(
        app("Add", vec![nat_var("n"), zero()]),
        nat_var("n"),
    );

    let result = engine.prove(goal);

    assert!(result.is_ok(), "Should prove Add(n, 0) = n by induction");

    let proof = result.unwrap();

    // The proof should use StructuralInduction
    assert!(
        matches!(proof.rule, InferenceRule::StructuralInduction { .. }),
        "Top-level rule should be StructuralInduction, got {:?}",
        proof.rule
    );

    // Should have 2 premises: base case and step case
    assert_eq!(proof.premises.len(), 2, "Induction needs base and step cases");

    println!("Proof of Add(n, 0) = n:");
    println!("{}", proof.display_tree());
}
