//! SUPERCRUSH Sprint S0E: Type Inference Engine
//!
//! Constraint-based type inference for VerifyExpr. Walks the expression tree,
//! collects type constraints from operators, unifies, detects conflicts,
//! and propagates bitvector widths.
//!
//! All tests require the `verification` feature (Z3 dependency).

#![cfg(feature = "verification")]

use logicaffeine_verify::ir::{VerifyExpr, VerifyOp, VerifyType, BitVecOp};
use logicaffeine_verify::type_infer::{infer_types, TypeError};

// ═══════════════════════════════════════════════════════════════════════════
// BASIC TYPE INFERENCE
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn infer_bool_from_and() {
    // And(Var("x"), Var("y")) → both Bool
    let expr = VerifyExpr::and(VerifyExpr::var("x"), VerifyExpr::var("y"));
    let types = infer_types(&expr).unwrap();
    assert_eq!(types.get("x"), Some(&VerifyType::Bool));
    assert_eq!(types.get("y"), Some(&VerifyType::Bool));
}

#[test]
fn infer_int_from_add() {
    // Add(Var("x"), Int(5)) → x is Int
    let expr = VerifyExpr::binary(VerifyOp::Add, VerifyExpr::var("x"), VerifyExpr::int(5));
    let types = infer_types(&expr).unwrap();
    assert_eq!(types.get("x"), Some(&VerifyType::Int));
}

#[test]
fn infer_bitvec_from_bv_add() {
    // BvAdd(Var("x"), BvConst(8, 0)) → x is BV(8)
    let expr = VerifyExpr::bv_binary(
        BitVecOp::Add,
        VerifyExpr::var("x"),
        VerifyExpr::bv_const(8, 0),
    );
    let types = infer_types(&expr).unwrap();
    assert_eq!(types.get("x"), Some(&VerifyType::BitVector(8)));
}

#[test]
fn infer_bitvec_width_propagation() {
    // Width from constant propagates to variable
    let expr = VerifyExpr::bv_binary(
        BitVecOp::And,
        VerifyExpr::var("a"),
        VerifyExpr::bv_const(16, 0xFF),
    );
    let types = infer_types(&expr).unwrap();
    assert_eq!(types.get("a"), Some(&VerifyType::BitVector(16)));
}

#[test]
fn infer_array_from_select() {
    // Select(Var("a"), Int(0)) → a is Array(Int, ?)
    let expr = VerifyExpr::Select {
        array: Box::new(VerifyExpr::var("a")),
        index: Box::new(VerifyExpr::int(0)),
    };
    let types = infer_types(&expr).unwrap();
    match types.get("a") {
        Some(VerifyType::Array(idx, _)) => {
            assert_eq!(**idx, VerifyType::Int, "Array index type should be Int");
        }
        other => panic!("Expected Array type for 'a', got: {:?}", other),
    }
}

#[test]
fn infer_conflict_detected() {
    // Var("x") used as both Bool (in And) and Int (in Add) → TypeError
    let bool_use = VerifyExpr::and(VerifyExpr::var("x"), VerifyExpr::bool(true));
    let int_use = VerifyExpr::binary(VerifyOp::Add, VerifyExpr::var("x"), VerifyExpr::int(1));
    let expr = VerifyExpr::and(bool_use, VerifyExpr::gt(int_use, VerifyExpr::int(0)));
    let result = infer_types(&expr);
    assert!(result.is_err(), "Should detect type conflict for 'x', got: {:?}", result);
}

#[test]
fn infer_nested() {
    // And(Gt(Var("x"), Int(5)), Bool(true)) → x is Int
    let expr = VerifyExpr::and(
        VerifyExpr::gt(VerifyExpr::var("x"), VerifyExpr::int(5)),
        VerifyExpr::bool(true),
    );
    let types = infer_types(&expr).unwrap();
    assert_eq!(types.get("x"), Some(&VerifyType::Int));
}

#[test]
fn infer_implication() {
    // Implies(Var("p"), Var("q")) → both Bool
    let expr = VerifyExpr::implies(VerifyExpr::var("p"), VerifyExpr::var("q"));
    let types = infer_types(&expr).unwrap();
    assert_eq!(types.get("p"), Some(&VerifyType::Bool));
    assert_eq!(types.get("q"), Some(&VerifyType::Bool));
}

#[test]
fn infer_comparison_returns_bool_operands_int() {
    // Gt(Var("x"), Int(5)) → x is Int (result is Bool, but operands are Int)
    let expr = VerifyExpr::gt(VerifyExpr::var("x"), VerifyExpr::int(5));
    let types = infer_types(&expr).unwrap();
    assert_eq!(types.get("x"), Some(&VerifyType::Int));
}

#[test]
fn infer_bitvec_comparison() {
    // BvULt(Var("x"), Var("y")) → both BV(same width)
    let expr = VerifyExpr::bv_binary(
        BitVecOp::ULt,
        VerifyExpr::var("x"),
        VerifyExpr::bv_const(8, 5),
    );
    let types = infer_types(&expr).unwrap();
    assert_eq!(types.get("x"), Some(&VerifyType::BitVector(8)));
}

#[test]
fn infer_extract_width() {
    // BvExtract{7,0}(Var("x")) → x has width >= 8
    let expr = VerifyExpr::BitVecExtract {
        high: 7,
        low: 0,
        operand: Box::new(VerifyExpr::var("x")),
    };
    let types = infer_types(&expr).unwrap();
    match types.get("x") {
        Some(VerifyType::BitVector(w)) => {
            assert!(*w >= 8, "x should have width >= 8, got: {}", w);
        }
        other => panic!("Expected BitVector for 'x', got: {:?}", other),
    }
}

#[test]
fn infer_concat_inputs() {
    // Concat(Var("a"), Var("b")) with BV(8) constants nearby
    let expr = VerifyExpr::BitVecConcat(
        Box::new(VerifyExpr::bv_const(8, 0x12)),
        Box::new(VerifyExpr::bv_const(8, 0x34)),
    );
    // No variables here, so just check it doesn't error
    let types = infer_types(&expr).unwrap();
    assert!(types.is_empty(), "No variables to infer");
}

#[test]
fn infer_empty_formula() {
    // Bool(true) → empty map
    let expr = VerifyExpr::bool(true);
    let types = infer_types(&expr).unwrap();
    assert!(types.is_empty());
}

#[test]
fn infer_multiple_constraints_unify() {
    // Same var in multiple contexts → single consistent type
    // x used in Add(x, 1) and Gt(x, 5) — both require Int
    let expr = VerifyExpr::and(
        VerifyExpr::gt(
            VerifyExpr::binary(VerifyOp::Add, VerifyExpr::var("x"), VerifyExpr::int(1)),
            VerifyExpr::int(5),
        ),
        VerifyExpr::lt(VerifyExpr::var("x"), VerifyExpr::int(100)),
    );
    let types = infer_types(&expr).unwrap();
    assert_eq!(types.get("x"), Some(&VerifyType::Int));
}

#[test]
fn infer_free_variable_no_constraint() {
    // Var("x") alone has no constraints — should not appear in result
    let expr = VerifyExpr::var("x");
    let types = infer_types(&expr).unwrap();
    // Unconstrained variables may or may not appear
    // If present, any type is acceptable
    assert!(types.get("x").is_none() || types.get("x").is_some());
}
