//! Sprint 2: SVA → Bounded Verification IR Translation
//!
//! Tests that SvaExpr is correctly translated to bounded timestep model
//! for Z3 equivalence checking. No Z3 dependency — tests check the
//! structure of BoundedExpr output.

use logicaffeine_compile::codegen_sva::sva_model::{SvaExpr, parse_sva};
use logicaffeine_compile::codegen_sva::sva_to_verify::{
    SvaTranslator, BoundedExpr, count_or_leaves, count_and_leaves,
};

// ═══════════════════════════════════════════════════════════════════════════
// BASIC TRANSLATIONS
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn signal_at_timestep_produces_var() {
    let mut translator = SvaTranslator::new(10);
    let result = translator.translate(&SvaExpr::Signal("req".into()), 3);
    assert_eq!(result, BoundedExpr::Var("req@3".into()));
}

#[test]
fn and_translates_to_binary_and() {
    let mut translator = SvaTranslator::new(10);
    let expr = parse_sva("req && ack").unwrap();
    let result = translator.translate(&expr, 0);
    assert!(matches!(result, BoundedExpr::And(_, _)));
}

#[test]
fn rose_at_t5_is_current_and_not_previous() {
    let mut translator = SvaTranslator::new(10);
    let expr = SvaExpr::Rose(Box::new(SvaExpr::Signal("clk".into())));
    let result = translator.translate(&expr, 5);
    // Should be: And(Var("clk@5"), Not(Var("clk@4")))
    match &result {
        BoundedExpr::And(left, right) => {
            assert_eq!(**left, BoundedExpr::Var("clk@5".into()));
            match right.as_ref() {
                BoundedExpr::Not(inner) => {
                    assert_eq!(**inner, BoundedExpr::Var("clk@4".into()));
                }
                _ => panic!("Expected Not, got {:?}", right),
            }
        }
        _ => panic!("Expected And, got {:?}", result),
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// TEMPORAL UNROLLING
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn s_eventually_unrolls_to_disjunction() {
    let mut translator = SvaTranslator::new(3);
    let expr = SvaExpr::SEventually(Box::new(SvaExpr::Signal("ack".into())));
    let result = translator.translate(&expr, 0);
    // Should be: ack@1 || ack@2 || ack@3 (3 leaves)
    assert_eq!(count_or_leaves(&result), 3);
}

#[test]
fn delay_range_produces_disjunction() {
    let mut translator = SvaTranslator::new(10);
    let expr = SvaExpr::Delay {
        body: Box::new(SvaExpr::Signal("ack".into())),
        min: 1,
        max: Some(3),
    };
    let result = translator.translate(&expr, 0);
    // ##[1:3] ack → ack@1 || ack@2 || ack@3 (3 leaves)
    assert_eq!(count_or_leaves(&result), 3);
}

// ═══════════════════════════════════════════════════════════════════════════
// IMPLICATION TIMESTEP SEMANTICS
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn overlapping_implication_same_timestep() {
    let mut translator = SvaTranslator::new(10);
    let expr = parse_sva("req |-> ack").unwrap();
    let result = translator.translate(&expr, 5);
    // req@5 → ack@5
    match &result {
        BoundedExpr::Implies(left, right) => {
            assert_eq!(**left, BoundedExpr::Var("req@5".into()));
            assert_eq!(**right, BoundedExpr::Var("ack@5".into()));
        }
        _ => panic!("Expected Implies, got {:?}", result),
    }
}

#[test]
fn non_overlapping_implication_next_timestep() {
    let mut translator = SvaTranslator::new(10);
    let expr = parse_sva("req |=> ack").unwrap();
    let result = translator.translate(&expr, 5);
    // req@5 → ack@6
    match &result {
        BoundedExpr::Implies(left, right) => {
            assert_eq!(**left, BoundedExpr::Var("req@5".into()));
            assert_eq!(**right, BoundedExpr::Var("ack@6".into()));
        }
        _ => panic!("Expected Implies, got {:?}", result),
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// PROPERTY-LEVEL TRANSLATION
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn translate_property_conjoins_over_timesteps() {
    let mut translator = SvaTranslator::new(3);
    let expr = SvaExpr::Signal("valid".into());
    let result = translator.translate_property(&expr);
    // valid@0 && valid@1 && valid@2 (3 leaves)
    assert_eq!(count_and_leaves(&result.expr), 3);
}

#[test]
fn declarations_include_all_timestamped_signals() {
    let mut translator = SvaTranslator::new(2);
    let expr = parse_sva("req |-> ack").unwrap();
    let result = translator.translate_property(&expr);
    let decls = &result.declarations;
    // Should have req@0, req@1, ack@0, ack@1
    assert!(decls.contains(&"req@0".to_string()), "Missing req@0. Got: {:?}", decls);
    assert!(decls.contains(&"req@1".to_string()), "Missing req@1. Got: {:?}", decls);
    assert!(decls.contains(&"ack@0".to_string()), "Missing ack@0. Got: {:?}", decls);
    assert!(decls.contains(&"ack@1".to_string()), "Missing ack@1. Got: {:?}", decls);
}
