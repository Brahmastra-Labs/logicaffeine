//! Phase 82: Global Definitions (Delta Reduction)
//!
//! Teaches the Kernel to remember definitions and unfold them.
//! - Definitions: name : T := v (transparent, unfoldable)
//! - Axioms: name : T (opaque, not unfoldable)

use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{normalize, Context, Term};

// =============================================================================
// HELPER FUNCTIONS
// =============================================================================

fn nat() -> Term {
    Term::Global("Nat".to_string())
}

fn zero() -> Term {
    Term::Global("Zero".to_string())
}

fn succ(n: Term) -> Term {
    Term::App(Box::new(Term::Global("Succ".to_string())), Box::new(n))
}

// =============================================================================
// DELTA REDUCTION TESTS
// =============================================================================

#[test]
fn test_definition_unfolds_during_normalization() {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);

    // Define: my_two : Nat := Succ(Succ(Zero))
    let two_value = succ(succ(zero()));
    ctx.add_definition("my_two".to_string(), nat(), two_value.clone());

    // Normalize Term::Global("my_two")
    let term = Term::Global("my_two".to_string());
    let reduced = normalize(&ctx, &term);

    // Should unfold to Succ(Succ(Zero))
    assert_eq!(
        format!("{}", reduced),
        format!("{}", two_value),
        "Definition should unfold to its body"
    );
}

#[test]
fn test_axiom_does_not_unfold() {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);

    // Declare axiom: magic : Nat (no body)
    ctx.add_declaration("magic", nat());

    // Normalize Term::Global("magic")
    let term = Term::Global("magic".to_string());
    let reduced = normalize(&ctx, &term);

    // Should remain as Global("magic") - axioms are opaque
    assert!(
        matches!(reduced, Term::Global(ref name) if name == "magic"),
        "Axiom should not unfold, got: {}",
        reduced
    );
}

#[test]
fn test_constructor_does_not_delta_reduce() {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);

    // Zero and Succ are constructors, not definitions
    let term = Term::Global("Zero".to_string());
    let reduced = normalize(&ctx, &term);

    // Constructors are reduced by iota, not delta
    assert!(
        matches!(reduced, Term::Global(ref name) if name == "Zero"),
        "Constructor should not delta-reduce, got: {}",
        reduced
    );
}

#[test]
fn test_definition_in_application() {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);

    // Define: one : Nat := Succ(Zero)
    ctx.add_definition("one".to_string(), nat(), succ(zero()));

    // Normalize: Succ(one) should become Succ(Succ(Zero))
    let term = succ(Term::Global("one".to_string()));
    let reduced = normalize(&ctx, &term);

    let expected = succ(succ(zero()));
    assert_eq!(
        format!("{}", reduced),
        format!("{}", expected),
        "Definition should unfold inside application"
    );
}

#[test]
fn test_nested_definitions() {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);

    // Define: one := Succ(Zero)
    ctx.add_definition("one".to_string(), nat(), succ(zero()));

    // Define: two := Succ(one)  (references another definition)
    ctx.add_definition(
        "two".to_string(),
        nat(),
        succ(Term::Global("one".to_string())),
    );

    // Normalize: two should become Succ(Succ(Zero))
    let term = Term::Global("two".to_string());
    let reduced = normalize(&ctx, &term);

    let expected = succ(succ(zero()));
    assert_eq!(
        format!("{}", reduced),
        format!("{}", expected),
        "Nested definitions should fully unfold"
    );
}

#[test]
fn test_definition_equality_by_computation() {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);

    // Define: two := Succ(Succ(Zero))
    let two_value = succ(succ(zero()));
    ctx.add_definition("two".to_string(), nat(), two_value.clone());

    // Normalize both sides
    let left = normalize(&ctx, &Term::Global("two".to_string()));
    let right = normalize(&ctx, &two_value);

    // Both should normalize to the same term
    assert_eq!(
        format!("{}", left),
        format!("{}", right),
        "two and Succ(Succ(Zero)) should be equal after normalization"
    );
}

#[test]
fn test_is_definition_query() {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);

    // Add a definition
    ctx.add_definition("my_def".to_string(), nat(), zero());

    // Add an axiom
    ctx.add_declaration("my_axiom", nat());

    // Query the context
    assert!(ctx.is_definition("my_def"), "my_def should be a definition");
    assert!(
        !ctx.is_definition("my_axiom"),
        "my_axiom should not be a definition"
    );
    assert!(
        !ctx.is_definition("Zero"),
        "Zero (constructor) should not be a definition"
    );
    assert!(
        !ctx.is_definition("Nat"),
        "Nat (inductive) should not be a definition"
    );
}

#[test]
fn test_get_definition_body() {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);

    let body = succ(zero());
    ctx.add_definition("one".to_string(), nat(), body.clone());

    // Should retrieve the body
    let retrieved = ctx.get_definition_body("one");
    assert!(retrieved.is_some(), "Should have definition body");
    assert_eq!(
        format!("{}", retrieved.unwrap()),
        format!("{}", body),
        "Retrieved body should match"
    );

    // Axioms have no body
    ctx.add_declaration("axiom", nat());
    assert!(
        ctx.get_definition_body("axiom").is_none(),
        "Axiom should have no body"
    );
}
