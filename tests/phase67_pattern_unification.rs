// =============================================================================
// PHASE 67: HIGHER-ORDER PATTERN UNIFICATION (MILLER'S ALGORITHM)
// =============================================================================
//
// "Teaching the Machine to Infer"
//
// Phase 66 taught the machine to COMPUTE: (λx.P)(a) → P[x:=a]
// Phase 67 teaches it to INFER: ?P(x) = Body → ?P = λx.Body
//
// This is Miller Pattern Unification - the decidable fragment of higher-order
// unification sufficient for:
// - Motive inference in structural induction
// - Type inference in dependent types
// - Implicit argument resolution
//
// The Pattern: ?F(x₁, ..., xₙ) = Body where x_i are distinct bound variables
// The Solution: ?F = λx₁...λxₙ. Body

use logos::proof::{unify::unify_pattern, ProofExpr, ProofTerm};

// =============================================================================
// BASIC PATTERN UNIFICATION
// =============================================================================

#[test]
fn test_simple_pattern_unification() {
    // Goal: ?P(x) = x + 0 = x
    // Should infer: ?P = λx. (x + 0 = x)

    let hole = ProofExpr::Hole("P".to_string());

    // ?P(#x) - hole applied to bound var reference
    let lhs = ProofExpr::App(
        Box::new(hole),
        Box::new(ProofExpr::Term(ProofTerm::BoundVarRef("x".to_string()))),
    );

    // x + 0 = x (in the body, x is a Variable in terms)
    let x = ProofTerm::Variable("x".to_string());
    let rhs = ProofExpr::Identity(
        ProofTerm::Function("add".to_string(), vec![
            x.clone(),
            ProofTerm::Constant("0".to_string()),
        ]),
        x.clone(),
    );

    let result = unify_pattern(&lhs, &rhs);
    assert!(result.is_ok(), "Pattern unification should succeed: {:?}", result);

    let solution = result.unwrap();
    let p_solution = solution.get("P").expect("P should be solved");

    // Verify it's a lambda
    match p_solution {
        ProofExpr::Lambda { variable, .. } => {
            assert_eq!(variable, "x", "Lambda should bind x");
        }
        _ => panic!("P should be a lambda, got: {:?}", p_solution),
    }

    println!("Inferred: ?P = {}", p_solution);
}

// =============================================================================
// PATTERN APPLICATION (INTEGRATION WITH BETA-REDUCTION)
// =============================================================================

#[test]
fn test_pattern_application_after_solve() {
    // After solving ?P = λx.(x + 0 = x)
    // Applying P(5) should give: 5 + 0 = 5
    // This verifies beta-reduction + pattern unification work together

    use logos::proof::unify::beta_reduce;

    // Construct P = λx. (x + 0 = x)
    let x = ProofTerm::Variable("x".to_string());
    let p_lambda = ProofExpr::Lambda {
        variable: "x".to_string(),
        body: Box::new(ProofExpr::Identity(
            ProofTerm::Function("add".to_string(), vec![
                x.clone(),
                ProofTerm::Constant("0".to_string()),
            ]),
            x.clone(),
        )),
    };

    // Apply P(5)
    let application = ProofExpr::App(
        Box::new(p_lambda),
        Box::new(ProofExpr::Term(ProofTerm::Constant("5".to_string()))),
    );

    // Beta-reduce
    let result = beta_reduce(&application);

    // Should be: 5 + 0 = 5
    match &result {
        ProofExpr::Identity(left, right) => {
            match left {
                ProofTerm::Function(name, args) => {
                    assert_eq!(name, "add");
                    assert_eq!(args.len(), 2);
                }
                _ => panic!("Expected function on left side"),
            }
            assert_eq!(right, &ProofTerm::Constant("5".to_string()));
        }
        _ => panic!("Expected Identity, got: {:?}", result),
    }

    println!("P(5) reduces to: {}", result);
}

// =============================================================================
// MULTI-ARGUMENT PATTERNS
// =============================================================================

#[test]
fn test_multi_arg_pattern() {
    // ?F(#x, #y) = x + y = y + x
    // Should infer: ?F = λx.λy. (x + y = y + x)

    let hole = ProofExpr::Hole("F".to_string());

    // Build ?F(#x)(#y) - curried application
    let app_x = ProofExpr::App(
        Box::new(hole),
        Box::new(ProofExpr::Term(ProofTerm::BoundVarRef("x".to_string()))),
    );
    let lhs = ProofExpr::App(
        Box::new(app_x),
        Box::new(ProofExpr::Term(ProofTerm::BoundVarRef("y".to_string()))),
    );

    // x + y = y + x
    let x = ProofTerm::Variable("x".to_string());
    let y = ProofTerm::Variable("y".to_string());
    let rhs = ProofExpr::Identity(
        ProofTerm::Function("add".to_string(), vec![x.clone(), y.clone()]),
        ProofTerm::Function("add".to_string(), vec![y.clone(), x.clone()]),
    );

    let result = unify_pattern(&lhs, &rhs);
    assert!(result.is_ok(), "Multi-arg pattern unification should succeed: {:?}", result);

    let solution = result.unwrap();
    let f_solution = solution.get("F").expect("F should be solved");

    // Verify it's a nested lambda: λx.λy.Body
    match f_solution {
        ProofExpr::Lambda { variable: v1, body: inner } => {
            assert_eq!(v1, "x");
            match inner.as_ref() {
                ProofExpr::Lambda { variable: v2, .. } => {
                    assert_eq!(v2, "y");
                }
                _ => panic!("Inner should be lambda, got: {:?}", inner),
            }
        }
        _ => panic!("F should be a lambda, got: {:?}", f_solution),
    }

    println!("Inferred: ?F = {}", f_solution);
}

// =============================================================================
// ERROR CASES
// =============================================================================

#[test]
fn test_non_pattern_fails() {
    // ?P(#x, #x) - duplicate variables should fail (not a Miller pattern)

    let hole = ProofExpr::Hole("P".to_string());

    // ?P(#x)(#x) - same variable twice
    let app1 = ProofExpr::App(
        Box::new(hole),
        Box::new(ProofExpr::Term(ProofTerm::BoundVarRef("x".to_string()))),
    );
    let lhs = ProofExpr::App(
        Box::new(app1),
        Box::new(ProofExpr::Term(ProofTerm::BoundVarRef("x".to_string()))),
    );

    let rhs = ProofExpr::Atom("anything".to_string());

    let result = unify_pattern(&lhs, &rhs);
    assert!(result.is_err(), "Duplicate variables should fail");

    println!("Correctly rejected non-pattern: {:?}", result.err());
}

#[test]
fn test_scope_violation_fails() {
    // ?P(#x) = y + 0  - RHS uses y which is not in pattern scope

    let hole = ProofExpr::Hole("P".to_string());

    let lhs = ProofExpr::App(
        Box::new(hole),
        Box::new(ProofExpr::Term(ProofTerm::BoundVarRef("x".to_string()))),
    );

    // y + 0 - uses y which is NOT in the pattern variables
    let y = ProofTerm::Variable("y".to_string());
    let rhs = ProofExpr::Identity(
        ProofTerm::Function("add".to_string(), vec![
            y.clone(),
            ProofTerm::Constant("0".to_string()),
        ]),
        y.clone(),
    );

    let result = unify_pattern(&lhs, &rhs);
    assert!(result.is_err(), "Scope violation should fail");

    println!("Correctly rejected scope violation: {:?}", result.err());
}

// =============================================================================
// BARE HOLE (NO ARGUMENTS)
// =============================================================================

#[test]
fn test_bare_hole() {
    // ?P = Run(John) - hole without arguments
    // Should simply bind: P = Run(John)

    let hole = ProofExpr::Hole("P".to_string());

    let rhs = ProofExpr::Predicate {
        name: "run".to_string(),
        args: vec![ProofTerm::Constant("John".to_string())],
        world: None,
    };

    let result = unify_pattern(&hole, &rhs);
    assert!(result.is_ok(), "Bare hole should unify directly");

    let solution = result.unwrap();
    let p_solution = solution.get("P").expect("P should be solved");

    // Should be exactly the RHS (no lambda wrapping)
    assert_eq!(p_solution, &rhs);

    println!("Bare hole: ?P = {}", p_solution);
}
