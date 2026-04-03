//! SUPERCRUSH Sprint S0C: BoundedExpr Multi-Sorted Extension
//!
//! Tests that BoundedExpr can represent bitvector, array, arithmetic,
//! and quantifier operations, and that bounded_to_verify() correctly
//! translates them to VerifyExpr.

#![cfg(feature = "verification")]

use logicaffeine_compile::codegen_sva::sva_to_verify::{
    BoundedExpr, BitVecBoundedOp, ArithBoundedOp, CmpBoundedOp, BoundedSort,
    bounded_to_verify,
};
use logicaffeine_verify::{VerifyExpr, VerifyOp, VerifyType, BitVecOp};

// ═══════════════════════════════════════════════════════════════════════════
// BITVECTOR BOUNDED EXPRESSIONS
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn bounded_bitvec_const() {
    let b = BoundedExpr::BitVecConst { width: 8, value: 255 };
    let v = bounded_to_verify(&b);
    assert_eq!(v, VerifyExpr::bv_const(8, 255));
}

#[test]
fn bounded_bitvec_var() {
    let b = BoundedExpr::BitVecVar("data@0".into(), 8);
    let v = bounded_to_verify(&b);
    assert_eq!(v, VerifyExpr::Var("data@0".into()));
}

#[test]
fn bounded_bitvec_binary_and() {
    let b = BoundedExpr::BitVecBinary {
        op: BitVecBoundedOp::And,
        left: Box::new(BoundedExpr::BitVecConst { width: 8, value: 0xFF }),
        right: Box::new(BoundedExpr::BitVecConst { width: 8, value: 0x0F }),
    };
    let v = bounded_to_verify(&b);
    assert_eq!(v, VerifyExpr::bv_binary(
        BitVecOp::And,
        VerifyExpr::bv_const(8, 0xFF),
        VerifyExpr::bv_const(8, 0x0F),
    ));
}

#[test]
fn bounded_bitvec_to_verify_all_ops() {
    let ops = vec![
        (BitVecBoundedOp::Or, BitVecOp::Or),
        (BitVecBoundedOp::Xor, BitVecOp::Xor),
        (BitVecBoundedOp::Add, BitVecOp::Add),
        (BitVecBoundedOp::Sub, BitVecOp::Sub),
        (BitVecBoundedOp::Mul, BitVecOp::Mul),
        (BitVecBoundedOp::Shl, BitVecOp::Shl),
        (BitVecBoundedOp::Shr, BitVecOp::Shr),
    ];
    for (bop, vop) in ops {
        let b = BoundedExpr::BitVecBinary {
            op: bop,
            left: Box::new(BoundedExpr::BitVecConst { width: 8, value: 1 }),
            right: Box::new(BoundedExpr::BitVecConst { width: 8, value: 2 }),
        };
        let v = bounded_to_verify(&b);
        assert!(matches!(v, VerifyExpr::BitVecBinary { .. }),
            "Failed for op {:?}", bop);
    }
}

#[test]
fn bounded_bitvec_extract() {
    let b = BoundedExpr::BitVecExtract {
        high: 7,
        low: 0,
        operand: Box::new(BoundedExpr::BitVecConst { width: 16, value: 0x1234 }),
    };
    let v = bounded_to_verify(&b);
    match v {
        VerifyExpr::BitVecExtract { high, low, .. } => {
            assert_eq!(high, 7);
            assert_eq!(low, 0);
        }
        _ => panic!("Expected BitVecExtract"),
    }
}

#[test]
fn bounded_bitvec_concat() {
    let b = BoundedExpr::BitVecConcat(
        Box::new(BoundedExpr::BitVecConst { width: 8, value: 0x12 }),
        Box::new(BoundedExpr::BitVecConst { width: 8, value: 0x34 }),
    );
    let v = bounded_to_verify(&b);
    assert!(matches!(v, VerifyExpr::BitVecConcat(_, _)));
}

// ═══════════════════════════════════════════════════════════════════════════
// ARRAY BOUNDED EXPRESSIONS
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn bounded_array_select() {
    let b = BoundedExpr::ArraySelect {
        array: Box::new(BoundedExpr::Var("mem".into())),
        index: Box::new(BoundedExpr::Int(0)),
    };
    let v = bounded_to_verify(&b);
    assert!(matches!(v, VerifyExpr::Select { .. }));
}

#[test]
fn bounded_array_store() {
    let b = BoundedExpr::ArrayStore {
        array: Box::new(BoundedExpr::Var("mem".into())),
        index: Box::new(BoundedExpr::Int(5)),
        value: Box::new(BoundedExpr::Int(42)),
    };
    let v = bounded_to_verify(&b);
    assert!(matches!(v, VerifyExpr::Store { .. }));
}

#[test]
fn bounded_array_store_select_roundtrip() {
    let stored = BoundedExpr::ArrayStore {
        array: Box::new(BoundedExpr::Var("mem".into())),
        index: Box::new(BoundedExpr::Int(3)),
        value: Box::new(BoundedExpr::Int(99)),
    };
    let selected = BoundedExpr::ArraySelect {
        array: Box::new(stored),
        index: Box::new(BoundedExpr::Int(3)),
    };
    let v = bounded_to_verify(&selected);
    // Should produce Select(Store(Var("mem"), 3, 99), 3)
    match v {
        VerifyExpr::Select { array, index } => {
            assert!(matches!(*array, VerifyExpr::Store { .. }));
        }
        _ => panic!("Expected Select"),
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// INTEGER ARITHMETIC
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn bounded_int_arithmetic() {
    let b = BoundedExpr::IntBinary {
        op: ArithBoundedOp::Add,
        left: Box::new(BoundedExpr::Var("x@0".into())),
        right: Box::new(BoundedExpr::Int(5)),
    };
    let v = bounded_to_verify(&b);
    assert!(matches!(v, VerifyExpr::Binary { op: VerifyOp::Add, .. }));
}

#[test]
fn bounded_comparison() {
    let b = BoundedExpr::Comparison {
        op: CmpBoundedOp::Gt,
        left: Box::new(BoundedExpr::Var("count@0".into())),
        right: Box::new(BoundedExpr::Int(5)),
    };
    let v = bounded_to_verify(&b);
    assert!(matches!(v, VerifyExpr::Binary { op: VerifyOp::Gt, .. }));
}

// ═══════════════════════════════════════════════════════════════════════════
// QUANTIFIERS
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn bounded_quantifier_forall() {
    let b = BoundedExpr::ForAll {
        var: "x".into(),
        sort: BoundedSort::Int,
        body: Box::new(BoundedExpr::Gt(
            Box::new(BoundedExpr::Var("x".into())),
            Box::new(BoundedExpr::Int(0)),
        )),
    };
    let v = bounded_to_verify(&b);
    match v {
        VerifyExpr::ForAll { vars, .. } => {
            assert_eq!(vars.len(), 1);
            assert_eq!(vars[0].0, "x");
            assert_eq!(vars[0].1, VerifyType::Int);
        }
        _ => panic!("Expected ForAll"),
    }
}

#[test]
fn bounded_quantifier_exists() {
    let b = BoundedExpr::Exists {
        var: "y".into(),
        sort: BoundedSort::BitVec(8),
        body: Box::new(BoundedExpr::Bool(true)),
    };
    let v = bounded_to_verify(&b);
    match v {
        VerifyExpr::Exists { vars, .. } => {
            assert_eq!(vars.len(), 1);
            assert_eq!(vars[0].1, VerifyType::BitVector(8));
        }
        _ => panic!("Expected Exists"),
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// REGRESSION
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn bounded_regression_boolean() {
    let b = BoundedExpr::And(
        Box::new(BoundedExpr::Var("p@0".into())),
        Box::new(BoundedExpr::Not(Box::new(BoundedExpr::Var("q@0".into())))),
    );
    let v = bounded_to_verify(&b);
    assert!(matches!(v, VerifyExpr::Binary { op: VerifyOp::And, .. }));
}

#[test]
fn bounded_nested_bitvec_ops() {
    let inner = BoundedExpr::BitVecBinary {
        op: BitVecBoundedOp::Or,
        left: Box::new(BoundedExpr::BitVecVar("a@0".into(), 8)),
        right: Box::new(BoundedExpr::BitVecVar("b@0".into(), 8)),
    };
    let outer = BoundedExpr::BitVecBinary {
        op: BitVecBoundedOp::And,
        left: Box::new(inner),
        right: Box::new(BoundedExpr::BitVecConst { width: 8, value: 0xFF }),
    };
    let v = bounded_to_verify(&outer);
    assert!(matches!(v, VerifyExpr::BitVecBinary { .. }));
}

#[test]
fn bounded_mixed_sort_formula() {
    // Bool + Int + BV in one formula
    let formula = BoundedExpr::And(
        Box::new(BoundedExpr::Var("valid@0".into())),
        Box::new(BoundedExpr::Comparison {
            op: CmpBoundedOp::Gt,
            left: Box::new(BoundedExpr::Var("count@0".into())),
            right: Box::new(BoundedExpr::Int(0)),
        }),
    );
    let v = bounded_to_verify(&formula);
    assert!(matches!(v, VerifyExpr::Binary { op: VerifyOp::And, .. }));
}
