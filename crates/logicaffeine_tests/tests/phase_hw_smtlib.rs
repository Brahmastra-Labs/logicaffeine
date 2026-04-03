//! SUPERCRUSH Sprint S3E: SMT-LIB2 Export

#![cfg(feature = "verification")]

use logicaffeine_verify::smtlib::{to_smtlib2, equivalence_to_smtlib2};
use logicaffeine_verify::{VerifyExpr, VerifyOp, VerifyType, BitVecOp};

#[test]
fn smtlib_bool_formula() {
    let expr = VerifyExpr::and(VerifyExpr::var("p"), VerifyExpr::not(VerifyExpr::var("q")));
    let smt = to_smtlib2(&expr, &[("p", VerifyType::Bool), ("q", VerifyType::Bool)]);
    assert!(smt.contains("(set-logic ALL)"));
    assert!(smt.contains("(declare-fun p () Bool)"));
    assert!(smt.contains("(assert"));
    assert!(smt.contains("(check-sat)"));
}

#[test]
fn smtlib_int_formula() {
    let expr = VerifyExpr::gt(
        VerifyExpr::binary(VerifyOp::Add, VerifyExpr::var("x"), VerifyExpr::int(5)),
        VerifyExpr::int(10),
    );
    let smt = to_smtlib2(&expr, &[("x", VerifyType::Int)]);
    assert!(smt.contains("Int"));
    assert!(smt.contains("(> (+ x 5) 10)"));
}

#[test]
fn smtlib_bitvec_formula() {
    let expr = VerifyExpr::bv_binary(
        BitVecOp::Add,
        VerifyExpr::var("x"),
        VerifyExpr::bv_const(8, 5),
    );
    let smt = to_smtlib2(&expr, &[("x", VerifyType::BitVector(8))]);
    assert!(smt.contains("(_ BitVec 8)"));
    assert!(smt.contains("bvadd"));
}

#[test]
fn smtlib_array_formula() {
    let expr = VerifyExpr::Select {
        array: Box::new(VerifyExpr::var("a")),
        index: Box::new(VerifyExpr::int(0)),
    };
    let smt = to_smtlib2(&expr, &[("a", VerifyType::Array(Box::new(VerifyType::Int), Box::new(VerifyType::Int)))]);
    assert!(smt.contains("(Array Int Int)"));
    assert!(smt.contains("(select a 0)"));
}

#[test]
fn smtlib_quantifier() {
    let expr = VerifyExpr::forall(
        vec![("x".into(), VerifyType::Int)],
        VerifyExpr::gt(VerifyExpr::var("x"), VerifyExpr::int(0)),
    );
    let smt = to_smtlib2(&expr, &[]);
    assert!(smt.contains("(forall ((x Int))"));
}

#[test]
fn smtlib_equivalence_query() {
    let a = VerifyExpr::binary(VerifyOp::Add, VerifyExpr::var("x"), VerifyExpr::int(0));
    let b = VerifyExpr::var("x");
    let smt = equivalence_to_smtlib2(&a, &b);
    assert!(smt.contains("(assert (not (="));
    assert!(smt.contains("(check-sat)"));
}

#[test]
fn smtlib_nested_bitvec() {
    let expr = VerifyExpr::bv_binary(
        BitVecOp::And,
        VerifyExpr::bv_binary(BitVecOp::Or, VerifyExpr::var("a"), VerifyExpr::var("b")),
        VerifyExpr::bv_const(8, 0xFF),
    );
    let smt = to_smtlib2(&expr, &[]);
    assert!(smt.contains("bvand"));
    assert!(smt.contains("bvor"));
}

#[test]
fn smtlib_mixed_sorts() {
    let expr = VerifyExpr::and(
        VerifyExpr::gt(VerifyExpr::var("x"), VerifyExpr::int(5)),
        VerifyExpr::var("valid"),
    );
    let smt = to_smtlib2(&expr, &[("x", VerifyType::Int), ("valid", VerifyType::Bool)]);
    assert!(smt.contains("Int"));
    assert!(smt.contains("Bool"));
}

#[test]
fn smtlib_implication() {
    let expr = VerifyExpr::implies(VerifyExpr::var("p"), VerifyExpr::var("q"));
    let smt = to_smtlib2(&expr, &[("p", VerifyType::Bool), ("q", VerifyType::Bool)]);
    assert!(smt.contains("=>"));
}

#[test]
fn smtlib_store_select() {
    let expr = VerifyExpr::Select {
        array: Box::new(VerifyExpr::Store {
            array: Box::new(VerifyExpr::var("a")),
            index: Box::new(VerifyExpr::int(0)),
            value: Box::new(VerifyExpr::int(42)),
        }),
        index: Box::new(VerifyExpr::int(0)),
    };
    let smt = to_smtlib2(&expr, &[]);
    assert!(smt.contains("select"));
    assert!(smt.contains("store"));
}
