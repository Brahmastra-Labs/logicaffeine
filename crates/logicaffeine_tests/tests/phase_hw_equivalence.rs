//! Sprint G: SVA Semantic Model & Equivalence Checking
//!
//! Tests for parsing SVA syntax, translating to a semantic model,
//! and checking structural equivalence between FOL and SVA properties.
//!
//! The SvaExpr model lives in codegen_sva/sva_model.rs and provides
//! parse_sva() and structural equivalence checking.

use logicaffeine_compile::codegen_sva::sva_model::{
    SvaExpr, parse_sva, sva_exprs_structurally_equivalent,
};

// ═══════════════════════════════════════════════════════════════════════════
// SVA PARSING
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn sva_parse_simple_signal() {
    let expr = parse_sva("valid").unwrap();
    assert!(matches!(expr, SvaExpr::Signal(_)));
}

#[test]
fn sva_parse_negation() {
    let expr = parse_sva("!(grant_a && grant_b)").unwrap();
    assert!(matches!(expr, SvaExpr::Not(_)));
}

#[test]
fn sva_parse_rose() {
    let expr = parse_sva("$rose(req)").unwrap();
    assert!(matches!(expr, SvaExpr::Rose(_)));
}

#[test]
fn sva_parse_fell() {
    let expr = parse_sva("$fell(ack)").unwrap();
    assert!(matches!(expr, SvaExpr::Fell(_)));
}

#[test]
fn sva_parse_implication_overlapping() {
    let expr = parse_sva("valid |-> ready").unwrap();
    match expr {
        SvaExpr::Implication { overlapping, .. } => {
            assert!(overlapping, "|-> should be overlapping");
        }
        _ => panic!("Expected Implication, got {:?}", expr),
    }
}

#[test]
fn sva_parse_implication_non_overlapping() {
    let expr = parse_sva("valid |=> ready").unwrap();
    match expr {
        SvaExpr::Implication { overlapping, .. } => {
            assert!(!overlapping, "|=> should be non-overlapping");
        }
        _ => panic!("Expected Implication, got {:?}", expr),
    }
}

#[test]
fn sva_parse_s_eventually() {
    let expr = parse_sva("s_eventually(ack)").unwrap();
    assert!(matches!(expr, SvaExpr::SEventually(_)));
}

#[test]
fn sva_parse_equality() {
    let expr = parse_sva("data_out == data_in").unwrap();
    assert!(matches!(expr, SvaExpr::Eq(_, _)));
}

// ═══════════════════════════════════════════════════════════════════════════
// STRUCTURAL EQUIVALENCE
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn equivalence_identical_signals() {
    let a = parse_sva("valid").unwrap();
    let b = parse_sva("valid").unwrap();
    assert!(sva_exprs_structurally_equivalent(&a, &b));
}

#[test]
fn equivalence_different_signals_fail() {
    let a = parse_sva("valid").unwrap();
    let b = parse_sva("ready").unwrap();
    assert!(!sva_exprs_structurally_equivalent(&a, &b));
}

#[test]
fn equivalence_same_negation() {
    let a = parse_sva("!(grant_a && grant_b)").unwrap();
    let b = parse_sva("!(grant_a && grant_b)").unwrap();
    assert!(sva_exprs_structurally_equivalent(&a, &b));
}

#[test]
fn equivalence_same_implication() {
    let a = parse_sva("req |-> ack").unwrap();
    let b = parse_sva("req |-> ack").unwrap();
    assert!(sva_exprs_structurally_equivalent(&a, &b));
}

#[test]
fn equivalence_different_implication_type_fail() {
    let a = parse_sva("req |-> ack").unwrap();
    let b = parse_sva("req |=> ack").unwrap();
    assert!(!sva_exprs_structurally_equivalent(&a, &b));
}
