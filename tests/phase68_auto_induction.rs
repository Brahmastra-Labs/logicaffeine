// =============================================================================
// PHASE 68: AUTO-INDUCTION (MOTIVE INFERENCE)
// =============================================================================
//
// "The Brain Meets the Body"
//
// Phase 67 taught the engine to SOLVE: ?P(x) = Body → ?P = λx.Body
// Phase 68 teaches it to USE this in induction proofs.
//
// Given: ∀n:Nat. Add(n, Zero) = n
// Infer: Motive = λx. (Add(x, Zero) = x)
// Apply: P(Zero) ∧ ∀k(P(k) → P(Succ(k))) ⊢ ∀n P(n)

use logos::proof::{
    BackwardChainer, DerivationTree, InferenceRule, ProofExpr, ProofTerm,
};

// =============================================================================
// HELPER FUNCTIONS - Same as Phase 61
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

fn nat_var(name: &str) -> ProofExpr {
    ProofExpr::TypedVar {
        name: name.into(),
        typename: "Nat".into(),
    }
}

fn var(name: &str) -> ProofExpr {
    ProofExpr::Atom(name.into())
}

fn app(name: &str, args: Vec<ProofExpr>) -> ProofExpr {
    ProofExpr::Predicate {
        name: name.into(),
        args: args.into_iter().map(|a| expr_to_term(a)).collect(),
        world: None,
    }
}

fn expr_to_term(expr: ProofExpr) -> ProofTerm {
    match expr {
        ProofExpr::Atom(s) => ProofTerm::Variable(s),
        ProofExpr::Ctor { name, args } => {
            ProofTerm::Function(
                name,
                args.into_iter().map(expr_to_term).collect(),
            )
        }
        ProofExpr::Predicate { name, args, .. } => {
            ProofTerm::Function(name, args)
        }
        ProofExpr::TypedVar { name, typename } => {
            ProofTerm::Variable(format!("{}:{}", name, typename))
        }
        _ => ProofTerm::Constant(format!("{}", expr)),
    }
}

fn eq(lhs: ProofExpr, rhs: ProofExpr) -> ProofExpr {
    ProofExpr::Identity(expr_to_term(lhs), expr_to_term(rhs))
}

// =============================================================================
// RIGHT IDENTITY - The Classic Induction Test
// =============================================================================

#[test]
fn test_auto_induction_right_identity() {
    // Theorem: ∀n:Nat. Add(n, Zero) = n
    //
    // The engine must:
    // 1. Recognize the TypedVar signals induction
    // 2. Create ?Motive hole
    // 3. Unify: ?Motive(#n) = (Add(n, Zero) = n)
    // 4. Infer: ?Motive = λx. (Add(x, Zero) = x)
    // 5. Apply motive to Zero → prove base case
    // 6. Apply motive to Succ(k) → prove step case with IH

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
    // This requires motive inference to work properly
    let goal = eq(
        app("Add", vec![nat_var("n"), zero()]),
        nat_var("n"),
    );

    let result = engine.prove(goal);

    // Should succeed using structural induction with auto-inferred motive
    assert!(result.is_ok(), "Should prove Add(n, Zero) = n by auto-induction: {:?}", result);

    let proof = result.unwrap();

    // Verify it used structural induction
    assert!(
        matches!(proof.rule, InferenceRule::StructuralInduction { .. }),
        "Should use StructuralInduction rule, got {:?}",
        proof.rule
    );

    println!("✓ Proved Add(n, Zero) = n with auto-inferred motive:");
    println!("{}", proof.display_tree());
}

// =============================================================================
// LEFT IDENTITY - Should Work Without Induction
// =============================================================================

#[test]
fn test_left_identity_no_induction_needed() {
    // Theorem: Add(Zero, n) = n
    // This follows directly from axiom 1 with universal instantiation
    // No induction needed - just unification

    let mut engine = BackwardChainer::new();
    engine.set_max_depth(10);

    // Axiom: Add(Zero, m) = m
    engine.add_axiom(eq(
        app("Add", vec![zero(), var("m")]),
        var("m"),
    ));

    // Goal: Add(Zero, n) = n (n is not typed, so no induction)
    let goal = eq(
        app("Add", vec![zero(), var("n")]),
        var("n"),
    );

    let result = engine.prove(goal);

    assert!(result.is_ok(), "Left identity should work via unification: {:?}", result);

    let proof = result.unwrap();
    println!("✓ Proved Add(Zero, n) = n:");
    println!("{}", proof.display_tree());
}

// =============================================================================
// VERIFY MOTIVE INFERENCE (Direct API Test)
// =============================================================================

#[test]
fn test_motive_inference_direct() {
    // Test that the pattern unification correctly infers the motive
    // This is a unit test for the motive inference mechanism

    use logos::proof::unify::{unify_pattern, beta_reduce};

    // Pattern: ?Motive(#n) where n is bound var
    let motive_hole = ProofExpr::Hole("Motive".to_string());
    let pattern = ProofExpr::App(
        Box::new(motive_hole),
        Box::new(ProofExpr::Term(ProofTerm::BoundVarRef("n".to_string()))),
    );

    // Body: Add(n, Zero) = n (what we want the motive to capture)
    // Note: Using Variable for n since it's a free var in the body
    let n = ProofTerm::Variable("n".to_string());
    let body = ProofExpr::Identity(
        ProofTerm::Function("Add".to_string(), vec![
            n.clone(),
            ProofTerm::Function("Zero".to_string(), vec![]),
        ]),
        n.clone(),
    );

    // Unify: ?Motive(#n) = Add(n, Zero) = n
    let result = unify_pattern(&pattern, &body);

    assert!(result.is_ok(), "Motive inference should succeed: {:?}", result);

    let solution = result.unwrap();
    let motive = solution.get("Motive").expect("Motive should be solved");

    // Motive should be: λn. (Add(n, Zero) = n)
    match motive {
        ProofExpr::Lambda { variable, body: _ } => {
            assert_eq!(variable, "n", "Lambda should bind 'n'");
        }
        _ => panic!("Motive should be a lambda, got: {:?}", motive),
    }

    println!("✓ Inferred motive: {}", motive);

    // Now verify applying motive(Zero) gives correct base case
    let base_app = ProofExpr::App(
        Box::new(motive.clone()),
        Box::new(ProofExpr::Ctor { name: "Zero".to_string(), args: vec![] }),
    );
    let base_case = beta_reduce(&base_app);

    println!("✓ Base case P(Zero): {}", base_case);

    // Should be: Add(Zero, Zero) = Zero
    match &base_case {
        ProofExpr::Identity(lhs, _rhs) => {
            // LHS should be Add(Zero, Zero)
            match lhs {
                ProofTerm::Function(name, args) if name == "Add" => {
                    assert_eq!(args.len(), 2, "Add should have 2 args");
                }
                _ => panic!("Expected Add function on LHS, got: {:?}", lhs),
            }
        }
        _ => panic!("Expected Identity, got: {:?}", base_case),
    }
}

// =============================================================================
// VERIFY MOTIVE INFERENCE TRIGGERS IN ENGINE
// =============================================================================

#[test]
fn test_motive_inference_triggered_in_engine() {
    // This test verifies that the Phase 68 motive inference path is actually
    // being exercised when the engine solves an induction problem.
    //
    // We construct a goal and verify that when solved, the proof structure
    // reflects the motive-based approach (beta reduction of λx.Body applied
    // to Zero and Succ(k)).

    use logos::proof::unify::beta_reduce;

    let mut engine = BackwardChainer::new();
    engine.set_max_depth(20);

    // Axiom 1: Add(Zero, m) = m
    engine.add_axiom(eq(
        app("Add", vec![zero(), var("m")]),
        var("m"),
    ));

    // Axiom 2: Add(Succ(k), m) = Succ(Add(k, m))
    engine.add_axiom(eq(
        app("Add", vec![succ(var("k")), var("m")]),
        succ(app("Add", vec![var("k"), var("m")])),
    ));

    // Goal: Add(n:Nat, Zero) = n:Nat
    let goal = eq(
        app("Add", vec![nat_var("n"), zero()]),
        nat_var("n"),
    );

    let result = engine.prove(goal);
    assert!(result.is_ok(), "Should prove by induction: {:?}", result);

    let proof = result.unwrap();

    // Verify the proof uses StructuralInduction
    assert!(
        matches!(proof.rule, InferenceRule::StructuralInduction { .. }),
        "Should use StructuralInduction rule"
    );

    // The proof should have 2 premises (base and step)
    assert_eq!(proof.premises.len(), 2, "Induction needs base and step cases");

    // Verify base case structure:
    // If motive inference worked, the base case should be proving
    // something like Add(Zero, Zero) = Zero (from applying motive to Zero)
    let base_case = &proof.premises[0];
    println!("Base case conclusion: {}", base_case.conclusion);

    // Verify step case structure:
    // The step case should involve Succ
    let step_case = &proof.premises[1];
    println!("Step case conclusion: {}", step_case.conclusion);

    // The step case conclusion should contain "Succ"
    let step_str = format!("{}", step_case.conclusion);
    assert!(
        step_str.contains("Succ"),
        "Step case should involve Succ, got: {}",
        step_str
    );

    println!("✓ Motive inference triggered successfully in engine!");
    println!("{}", proof.display_tree());
}
