//! Sprint 1: SVA AST Round-Trip + Delay Parsing
//!
//! Tests that SvaExpr can be rendered back to valid SVA text (sva_expr_to_string),
//! and that the round-trip parse → to_string → parse is identity.
//! Also tests delay operator parsing (##N, ##[min:max]).

use logicaffeine_compile::codegen_sva::sva_model::{
    SvaExpr, parse_sva, sva_expr_to_string, sva_exprs_structurally_equivalent,
};

// ═══════════════════════════════════════════════════════════════════════════
// sva_expr_to_string: INDIVIDUAL VARIANTS
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn sva_to_string_signal() {
    let expr = SvaExpr::Signal("req".into());
    assert_eq!(sva_expr_to_string(&expr), "req");
}

#[test]
fn sva_to_string_rose() {
    let expr = SvaExpr::Rose(Box::new(SvaExpr::Signal("clk".into())));
    assert_eq!(sva_expr_to_string(&expr), "$rose(clk)");
}

#[test]
fn sva_to_string_fell() {
    let expr = SvaExpr::Fell(Box::new(SvaExpr::Signal("reset".into())));
    assert_eq!(sva_expr_to_string(&expr), "$fell(reset)");
}

#[test]
fn sva_to_string_past() {
    let expr = SvaExpr::Past(Box::new(SvaExpr::Signal("sig".into())), 3);
    assert_eq!(sva_expr_to_string(&expr), "$past(sig, 3)");
}

#[test]
fn sva_to_string_and() {
    let expr = SvaExpr::And(
        Box::new(SvaExpr::Signal("a".into())),
        Box::new(SvaExpr::Signal("b".into())),
    );
    assert_eq!(sva_expr_to_string(&expr), "(a && b)");
}

#[test]
fn sva_to_string_or() {
    let expr = SvaExpr::Or(
        Box::new(SvaExpr::Signal("a".into())),
        Box::new(SvaExpr::Signal("b".into())),
    );
    assert_eq!(sva_expr_to_string(&expr), "(a || b)");
}

#[test]
fn sva_to_string_not() {
    let expr = SvaExpr::Not(Box::new(SvaExpr::Signal("x".into())));
    assert_eq!(sva_expr_to_string(&expr), "!(x)");
}

#[test]
fn sva_to_string_eq() {
    let expr = SvaExpr::Eq(
        Box::new(SvaExpr::Signal("data_out".into())),
        Box::new(SvaExpr::Signal("data_in".into())),
    );
    assert_eq!(sva_expr_to_string(&expr), "(data_out == data_in)");
}

#[test]
fn sva_to_string_implication_overlapping() {
    let expr = SvaExpr::Implication {
        antecedent: Box::new(SvaExpr::Signal("req".into())),
        consequent: Box::new(SvaExpr::SEventually(Box::new(SvaExpr::Signal("ack".into())))),
        overlapping: true,
    };
    assert_eq!(sva_expr_to_string(&expr), "req |-> s_eventually(ack)");
}

#[test]
fn sva_to_string_implication_non_overlapping() {
    let expr = SvaExpr::Implication {
        antecedent: Box::new(SvaExpr::Signal("req".into())),
        consequent: Box::new(SvaExpr::Signal("ack".into())),
        overlapping: false,
    };
    assert_eq!(sva_expr_to_string(&expr), "req |=> ack");
}

#[test]
fn sva_to_string_delay_range() {
    let expr = SvaExpr::Delay {
        body: Box::new(SvaExpr::Signal("ack".into())),
        min: 1,
        max: Some(5),
    };
    assert_eq!(sva_expr_to_string(&expr), "##[1:5] ack");
}

#[test]
fn sva_to_string_delay_exact() {
    let expr = SvaExpr::Delay {
        body: Box::new(SvaExpr::Signal("ack".into())),
        min: 3,
        max: None,
    };
    assert_eq!(sva_expr_to_string(&expr), "##3 ack");
}

#[test]
fn sva_to_string_s_eventually() {
    let expr = SvaExpr::SEventually(Box::new(SvaExpr::Signal("done".into())));
    assert_eq!(sva_expr_to_string(&expr), "s_eventually(done)");
}

#[test]
fn sva_to_string_const() {
    let expr = SvaExpr::Const(255, 8);
    assert_eq!(sva_expr_to_string(&expr), "8'd255");
}

// ═══════════════════════════════════════════════════════════════════════════
// ROUND-TRIP: parse → to_string → parse → structurally equivalent
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn sva_roundtrip_handshake() {
    let input = "req |-> s_eventually(ack)";
    let parsed = parse_sva(input).unwrap();
    let rendered = sva_expr_to_string(&parsed);
    let reparsed = parse_sva(&rendered).unwrap();
    assert!(sva_exprs_structurally_equivalent(&parsed, &reparsed));
}

#[test]
fn sva_roundtrip_mutex() {
    let input = "!(grant_a && grant_b)";
    let parsed = parse_sva(input).unwrap();
    let rendered = sva_expr_to_string(&parsed);
    let reparsed = parse_sva(&rendered).unwrap();
    assert!(sva_exprs_structurally_equivalent(&parsed, &reparsed));
}

// ═══════════════════════════════════════════════════════════════════════════
// DELAY PARSING
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn sva_parse_delay_range() {
    let expr = parse_sva("##[1:5] ack").unwrap();
    match expr {
        SvaExpr::Delay { min, max, body } => {
            assert_eq!(min, 1);
            assert_eq!(max, Some(5));
            assert!(matches!(*body, SvaExpr::Signal(ref s) if s == "ack"));
        }
        _ => panic!("Expected Delay, got {:?}", expr),
    }
}

#[test]
fn sva_parse_delay_exact() {
    let expr = parse_sva("##3 ack").unwrap();
    match expr {
        SvaExpr::Delay { min, max, body } => {
            assert_eq!(min, 3);
            assert!(max.is_none());
            assert!(matches!(*body, SvaExpr::Signal(ref s) if s == "ack"));
        }
        _ => panic!("Expected Delay, got {:?}", expr),
    }
}

#[test]
fn sva_parse_delay_in_implication() {
    let expr = parse_sva("req |-> ##[1:3] ack").unwrap();
    match expr {
        SvaExpr::Implication { consequent, .. } => {
            assert!(
                matches!(*consequent, SvaExpr::Delay { min: 1, max: Some(3), .. }),
                "Expected Delay consequent, got {:?}",
                consequent
            );
        }
        _ => panic!("Expected Implication with Delay consequent, got {:?}", expr),
    }
}
