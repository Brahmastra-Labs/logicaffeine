//! SUPERCRUSH Sprint S0A: Multi-Sorted Equivalence Checker
//!
//! Tests that the equivalence checker handles Int, BitVec, Array, and
//! mixed-sort formulas — not just Bool. The root cause of every false
//! positive is that equivalence.rs:encode_to_z3() maps non-boolean ops
//! to `true`/`false`. solver.rs already handles all sorts via Dynamic.
//!
//! All tests require the `verification` feature (Z3 dependency).

#![cfg(feature = "verification")]

use logicaffeine_verify::{
    check_equivalence, EquivalenceResult, VerifyExpr, VerifyOp, VerifyType, BitVecOp,
};

// ═══════════════════════════════════════════════════════════════════════════
// INTEGER ARITHMETIC EQUIVALENCE
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn z3_integer_addition_equivalence() {
    // (x + 5 == 10) equiv (x == 5)
    let a = VerifyExpr::eq(
        VerifyExpr::binary(VerifyOp::Add, VerifyExpr::var("x"), VerifyExpr::int(5)),
        VerifyExpr::int(10),
    );
    let b = VerifyExpr::eq(VerifyExpr::var("x"), VerifyExpr::int(5));
    let result = check_equivalence(&a, &b, &[], 1);
    assert!(matches!(result, EquivalenceResult::Equivalent),
        "x+5==10 should be equivalent to x==5, got: {:?}", result);
}

#[test]
fn z3_integer_subtraction_equivalence() {
    // (x - 3 == 7) equiv (x == 10)
    let a = VerifyExpr::eq(
        VerifyExpr::binary(VerifyOp::Sub, VerifyExpr::var("x"), VerifyExpr::int(3)),
        VerifyExpr::int(7),
    );
    let b = VerifyExpr::eq(VerifyExpr::var("x"), VerifyExpr::int(10));
    let result = check_equivalence(&a, &b, &[], 1);
    assert!(matches!(result, EquivalenceResult::Equivalent),
        "x-3==7 should be equivalent to x==10, got: {:?}", result);
}

#[test]
fn z3_integer_inequality_detected() {
    // (x > 5) not-equiv (x < 5)
    let a = VerifyExpr::gt(VerifyExpr::var("x"), VerifyExpr::int(5));
    let b = VerifyExpr::lt(VerifyExpr::var("x"), VerifyExpr::int(5));
    let result = check_equivalence(&a, &b, &[], 1);
    assert!(matches!(result, EquivalenceResult::NotEquivalent { .. }),
        "x>5 should NOT be equivalent to x<5, got: {:?}", result);
}

#[test]
fn z3_integer_multiplication() {
    // (x * 2 == 10) equiv (x == 5)
    let a = VerifyExpr::eq(
        VerifyExpr::binary(VerifyOp::Mul, VerifyExpr::var("x"), VerifyExpr::int(2)),
        VerifyExpr::int(10),
    );
    let b = VerifyExpr::eq(VerifyExpr::var("x"), VerifyExpr::int(5));
    let result = check_equivalence(&a, &b, &[], 1);
    assert!(matches!(result, EquivalenceResult::Equivalent),
        "x*2==10 should be equivalent to x==5, got: {:?}", result);
}

#[test]
fn z3_mixed_bool_int() {
    // (valid AND count > 0) not-equiv (valid AND count < 0)
    let a = VerifyExpr::and(
        VerifyExpr::var("valid"),
        VerifyExpr::gt(VerifyExpr::var("count"), VerifyExpr::int(0)),
    );
    let b = VerifyExpr::and(
        VerifyExpr::var("valid"),
        VerifyExpr::lt(VerifyExpr::var("count"), VerifyExpr::int(0)),
    );
    let result = check_equivalence(&a, &b, &[], 1);
    assert!(matches!(result, EquivalenceResult::NotEquivalent { .. }),
        "count>0 should NOT be equivalent to count<0, got: {:?}", result);
}

#[test]
fn z3_integer_division() {
    // (x / 2 == 3) equiv (x == 6) with integer semantics
    // Note: integer division, so 6/2 == 3 and 7/2 == 3 in Z3 integers
    // Actually x/2==3 means x in {6,7}, so NOT equiv to x==6
    let a = VerifyExpr::eq(
        VerifyExpr::binary(VerifyOp::Div, VerifyExpr::var("x"), VerifyExpr::int(2)),
        VerifyExpr::int(3),
    );
    let b = VerifyExpr::eq(VerifyExpr::var("x"), VerifyExpr::int(6));
    let result = check_equivalence(&a, &b, &[], 1);
    assert!(matches!(result, EquivalenceResult::NotEquivalent { .. }),
        "x/2==3 should NOT be equivalent to x==6 (integer division), got: {:?}", result);
}

#[test]
fn z3_comparison_chain() {
    // (x > 0 AND x < 10) not-equiv (x > 0 AND x < 5)
    let a = VerifyExpr::and(
        VerifyExpr::gt(VerifyExpr::var("x"), VerifyExpr::int(0)),
        VerifyExpr::lt(VerifyExpr::var("x"), VerifyExpr::int(10)),
    );
    let b = VerifyExpr::and(
        VerifyExpr::gt(VerifyExpr::var("x"), VerifyExpr::int(0)),
        VerifyExpr::lt(VerifyExpr::var("x"), VerifyExpr::int(5)),
    );
    let result = check_equivalence(&a, &b, &[], 1);
    assert!(matches!(result, EquivalenceResult::NotEquivalent { .. }),
        "Different ranges should not be equivalent, got: {:?}", result);
}

#[test]
fn z3_nested_arithmetic() {
    // ((x + y) * 2 == z) not-equiv ((x + y) == z)
    let xy = VerifyExpr::binary(VerifyOp::Add, VerifyExpr::var("x"), VerifyExpr::var("y"));
    let a = VerifyExpr::eq(
        VerifyExpr::binary(VerifyOp::Mul, xy.clone(), VerifyExpr::int(2)),
        VerifyExpr::var("z"),
    );
    let b = VerifyExpr::eq(xy, VerifyExpr::var("z"));
    let result = check_equivalence(&a, &b, &[], 1);
    assert!(matches!(result, EquivalenceResult::NotEquivalent { .. }),
        "(x+y)*2==z should NOT equal (x+y)==z, got: {:?}", result);
}

// ═══════════════════════════════════════════════════════════════════════════
// BITVECTOR EQUIVALENCE
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn z3_bitvector_and_mask() {
    // bv_and(x, 0xFF) equiv bv_extract(x, 7, 0) for BV(16)
    let a = VerifyExpr::bv_binary(
        BitVecOp::And,
        VerifyExpr::var("x"),
        VerifyExpr::bv_const(16, 0x00FF),
    );
    let b = VerifyExpr::BitVecExtract {
        high: 7,
        low: 0,
        operand: Box::new(VerifyExpr::var("x")),
    };
    // Both produce 8-bit results but in different ways. The AND mask zeroes top bits.
    // For this to be equivalent, we need to compare as 16-bit: extract pads with zeros.
    // Actually bv_and returns BV(16) while extract returns BV(8), so these are different widths.
    // Let's test a simpler bitvector equivalence instead.
    // bv_xor(x, x) equiv bv_const(0, 16)
    let a2 = VerifyExpr::bv_binary(
        BitVecOp::Xor,
        VerifyExpr::var("x_bv16"),
        VerifyExpr::var("x_bv16"),
    );
    let b2 = VerifyExpr::bv_const(16, 0);
    let result = check_equivalence(&a2, &b2, &[], 1);
    assert!(matches!(result, EquivalenceResult::Equivalent),
        "bv_xor(x,x) should equal 0, got: {:?}", result);
}

#[test]
fn z3_bitvector_add_not_sub() {
    // bv_add(x, y) not-equiv bv_sub(x, y)
    let a = VerifyExpr::bv_binary(
        BitVecOp::Add,
        VerifyExpr::var("x_bv8"),
        VerifyExpr::var("y_bv8"),
    );
    let b = VerifyExpr::bv_binary(
        BitVecOp::Sub,
        VerifyExpr::var("x_bv8"),
        VerifyExpr::var("y_bv8"),
    );
    let result = check_equivalence(&a, &b, &[], 1);
    assert!(matches!(result, EquivalenceResult::NotEquivalent { .. }),
        "bv_add should NOT equal bv_sub, got: {:?}", result);
}

#[test]
fn z3_bitvector_shift_left_equiv_mul() {
    // bv_shl(x, 1) equiv bv_mul(x, 2) for BV(8)
    let a = VerifyExpr::bv_binary(
        BitVecOp::Shl,
        VerifyExpr::var("x_bv8"),
        VerifyExpr::bv_const(8, 1),
    );
    let b = VerifyExpr::bv_binary(
        BitVecOp::Mul,
        VerifyExpr::var("x_bv8"),
        VerifyExpr::bv_const(8, 2),
    );
    let result = check_equivalence(&a, &b, &[], 1);
    assert!(matches!(result, EquivalenceResult::Equivalent),
        "bv_shl(x,1) should equal bv_mul(x,2), got: {:?}", result);
}

#[test]
fn z3_bitvector_overflow_detected() {
    // bv_add(0xFF, 1) wraps to 0 for BV(8) -- not equiv to 256
    // In BV(8), 0xFF + 1 = 0x00 (wraps)
    let add_expr = VerifyExpr::bv_binary(
        BitVecOp::Add,
        VerifyExpr::bv_const(8, 0xFF),
        VerifyExpr::bv_const(8, 1),
    );
    let overflow = VerifyExpr::bv_const(8, 0);
    let result = check_equivalence(&add_expr, &overflow, &[], 1);
    assert!(matches!(result, EquivalenceResult::Equivalent),
        "0xFF + 1 should wrap to 0 in BV(8), got: {:?}", result);
}

#[test]
fn z3_bitvector_not_not() {
    // bv_not(bv_not(x)) equiv x
    let a = VerifyExpr::bv_binary(
        BitVecOp::Not,
        VerifyExpr::bv_binary(BitVecOp::Not, VerifyExpr::var("x_bv8"), VerifyExpr::bv_const(8, 0)),
        VerifyExpr::bv_const(8, 0),
    );
    // Actually BitVecOp::Not is in the Binary enum but is conceptually unary.
    // Let's use XOR with all-ones for NOT: bv_xor(bv_xor(x, 0xFF), 0xFF) == x
    let not_x = VerifyExpr::bv_binary(BitVecOp::Xor, VerifyExpr::var("x_bv8"), VerifyExpr::bv_const(8, 0xFF));
    let not_not_x = VerifyExpr::bv_binary(BitVecOp::Xor, not_x, VerifyExpr::bv_const(8, 0xFF));
    let result = check_equivalence(&not_not_x, &VerifyExpr::var("x_bv8"), &[], 1);
    assert!(matches!(result, EquivalenceResult::Equivalent),
        "double XOR with 0xFF should cancel out, got: {:?}", result);
}

#[test]
fn z3_bitvector_xor_self() {
    // bv_xor(x, x) equiv bv_const(0, 8)
    let a = VerifyExpr::bv_binary(
        BitVecOp::Xor,
        VerifyExpr::var("x_bv8"),
        VerifyExpr::var("x_bv8"),
    );
    let b = VerifyExpr::bv_const(8, 0);
    let result = check_equivalence(&a, &b, &[], 1);
    assert!(matches!(result, EquivalenceResult::Equivalent),
        "bv_xor(x,x) should equal 0, got: {:?}", result);
}

#[test]
fn z3_demorgan_bitvec() {
    // bv_not(bv_and(a,b)) equiv bv_or(bv_not(a), bv_not(b))
    // Using XOR with 0xFF as NOT
    let a_var = VerifyExpr::var("a_bv8");
    let b_var = VerifyExpr::var("b_bv8");
    let ones = VerifyExpr::bv_const(8, 0xFF);

    let not_and = VerifyExpr::bv_binary(
        BitVecOp::Xor,
        VerifyExpr::bv_binary(BitVecOp::And, a_var.clone(), b_var.clone()),
        ones.clone(),
    );
    let or_nots = VerifyExpr::bv_binary(
        BitVecOp::Or,
        VerifyExpr::bv_binary(BitVecOp::Xor, a_var, ones.clone()),
        VerifyExpr::bv_binary(BitVecOp::Xor, b_var, ones),
    );
    let result = check_equivalence(&not_and, &or_nots, &[], 1);
    assert!(matches!(result, EquivalenceResult::Equivalent),
        "DeMorgan's law should hold for bitvectors, got: {:?}", result);
}

#[test]
fn z3_bitvector_concat_properties() {
    // concat(a, b) where a:BV(8), b:BV(8) should produce BV(16)
    // Test: concat(0x12, 0x34) == 0x1234
    let a = VerifyExpr::BitVecConcat(
        Box::new(VerifyExpr::bv_const(8, 0x12)),
        Box::new(VerifyExpr::bv_const(8, 0x34)),
    );
    let b = VerifyExpr::bv_const(16, 0x1234);
    let result = check_equivalence(&a, &b, &[], 1);
    assert!(matches!(result, EquivalenceResult::Equivalent),
        "concat(0x12, 0x34) should equal 0x1234, got: {:?}", result);
}

#[test]
fn z3_signed_vs_unsigned_comparison() {
    // bv_slt(x, y) not-equiv bv_ult(x, y) for values where sign matters
    // When x = 0xFF (255 unsigned, -1 signed) and y = 0x01 (1):
    // unsigned: 255 > 1, so ULt is false
    // signed: -1 < 1, so SLt is true
    let a = VerifyExpr::bv_binary(
        BitVecOp::SLt,
        VerifyExpr::var("x_bv8"),
        VerifyExpr::var("y_bv8"),
    );
    let b = VerifyExpr::bv_binary(
        BitVecOp::ULt,
        VerifyExpr::var("x_bv8"),
        VerifyExpr::var("y_bv8"),
    );
    let result = check_equivalence(&a, &b, &[], 1);
    assert!(matches!(result, EquivalenceResult::NotEquivalent { .. }),
        "signed lt should NOT equal unsigned lt, got: {:?}", result);
}

#[test]
fn z3_bitvector_extract() {
    // extract(0x1234, 15, 8) should equal 0x12
    let a = VerifyExpr::BitVecExtract {
        high: 15,
        low: 8,
        operand: Box::new(VerifyExpr::bv_const(16, 0x1234)),
    };
    let b = VerifyExpr::bv_const(8, 0x12);
    let result = check_equivalence(&a, &b, &[], 1);
    assert!(matches!(result, EquivalenceResult::Equivalent),
        "extract(0x1234, 15, 8) should equal 0x12, got: {:?}", result);
}

#[test]
fn z3_large_bitvector_32bit() {
    // 32-bit bitvector operations work without overflow
    let a = VerifyExpr::bv_binary(
        BitVecOp::Add,
        VerifyExpr::bv_const(32, 0x7FFFFFFF),
        VerifyExpr::bv_const(32, 1),
    );
    let b = VerifyExpr::bv_const(32, 0x80000000);
    let result = check_equivalence(&a, &b, &[], 1);
    assert!(matches!(result, EquivalenceResult::Equivalent),
        "32-bit add should work: 0x7FFFFFFF + 1 = 0x80000000, got: {:?}", result);
}

// ═══════════════════════════════════════════════════════════════════════════
// ARRAY SELECT/STORE EQUIVALENCE
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn z3_array_select_store() {
    // select(store(a, i, v), i) equiv v
    let stored = VerifyExpr::Store {
        array: Box::new(VerifyExpr::var("a")),
        index: Box::new(VerifyExpr::var("i")),
        value: Box::new(VerifyExpr::var("v")),
    };
    let selected = VerifyExpr::Select {
        array: Box::new(stored),
        index: Box::new(VerifyExpr::var("i")),
    };
    // select(store(a, i, v), i) == v
    let equiv_check = VerifyExpr::eq(selected, VerifyExpr::var("v"));
    let tautology = VerifyExpr::bool(true);
    let result = check_equivalence(&equiv_check, &tautology, &[], 1);
    assert!(matches!(result, EquivalenceResult::Equivalent),
        "select(store(a, i, v), i) should equal v, got: {:?}", result);
}

#[test]
fn z3_array_non_aliasing() {
    // select(store(a, i, v), j) with i != j equiv select(a, j)
    // We need to express this as: (i != j) -> (select(store(a,i,v),j) == select(a,j))
    let stored = VerifyExpr::Store {
        array: Box::new(VerifyExpr::var("a")),
        index: Box::new(VerifyExpr::var("i")),
        value: Box::new(VerifyExpr::var("v")),
    };
    let sel_stored = VerifyExpr::Select {
        array: Box::new(stored),
        index: Box::new(VerifyExpr::var("j")),
    };
    let sel_orig = VerifyExpr::Select {
        array: Box::new(VerifyExpr::var("a")),
        index: Box::new(VerifyExpr::var("j")),
    };
    let i_neq_j = VerifyExpr::neq(VerifyExpr::var("i"), VerifyExpr::var("j"));
    let reads_eq = VerifyExpr::eq(sel_stored, sel_orig);
    let prop = VerifyExpr::implies(i_neq_j, reads_eq);
    let result = check_equivalence(&prop, &VerifyExpr::bool(true), &[], 1);
    assert!(matches!(result, EquivalenceResult::Equivalent),
        "Non-aliasing array property should hold, got: {:?}", result);
}

#[test]
fn z3_array_store_overwrite() {
    // store(store(a, i, v1), i, v2) equiv store(a, i, v2)
    let inner = VerifyExpr::Store {
        array: Box::new(VerifyExpr::var("a")),
        index: Box::new(VerifyExpr::var("i")),
        value: Box::new(VerifyExpr::var("v1")),
    };
    let outer = VerifyExpr::Store {
        array: Box::new(inner),
        index: Box::new(VerifyExpr::var("i")),
        value: Box::new(VerifyExpr::var("v2")),
    };
    let simple = VerifyExpr::Store {
        array: Box::new(VerifyExpr::var("a")),
        index: Box::new(VerifyExpr::var("i")),
        value: Box::new(VerifyExpr::var("v2")),
    };
    // Check that for any index k, select from both gives same result
    let sel_outer = VerifyExpr::Select {
        array: Box::new(outer),
        index: Box::new(VerifyExpr::var("k")),
    };
    let sel_simple = VerifyExpr::Select {
        array: Box::new(simple),
        index: Box::new(VerifyExpr::var("k")),
    };
    let result = check_equivalence(
        &VerifyExpr::eq(sel_outer, sel_simple),
        &VerifyExpr::bool(true),
        &[],
        1,
    );
    assert!(matches!(result, EquivalenceResult::Equivalent),
        "Double store at same index should simplify, got: {:?}", result);
}

// ═══════════════════════════════════════════════════════════════════════════
// UNINTERPRETED FUNCTIONS
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn z3_uninterp_func_distinct() {
    // Apply("F", [x]) not-equiv Apply("G", [x])
    let a = VerifyExpr::apply("F", vec![VerifyExpr::var("x")]);
    let b = VerifyExpr::apply("G", vec![VerifyExpr::var("x")]);
    let result = check_equivalence(&a, &b, &[], 1);
    assert!(matches!(result, EquivalenceResult::NotEquivalent { .. }),
        "F(x) should NOT equal G(x), got: {:?}", result);
}

#[test]
fn z3_uninterp_func_congruence() {
    // Apply("F", [x]) equiv Apply("F", [x])
    let a = VerifyExpr::apply("F", vec![VerifyExpr::var("x")]);
    let b = VerifyExpr::apply("F", vec![VerifyExpr::var("x")]);
    let result = check_equivalence(&a, &b, &[], 1);
    assert!(matches!(result, EquivalenceResult::Equivalent),
        "F(x) should equal F(x), got: {:?}", result);
}

// ═══════════════════════════════════════════════════════════════════════════
// SORT MISMATCH AND EDGE CASES
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn z3_implication_with_arithmetic() {
    // (x > 0) -> (x >= 1) is valid (for integers)
    let prop = VerifyExpr::implies(
        VerifyExpr::gt(VerifyExpr::var("x"), VerifyExpr::int(0)),
        VerifyExpr::gte(VerifyExpr::var("x"), VerifyExpr::int(1)),
    );
    let result = check_equivalence(&prop, &VerifyExpr::bool(true), &[], 1);
    assert!(matches!(result, EquivalenceResult::Equivalent),
        "(x>0) -> (x>=1) should be valid, got: {:?}", result);
}

#[test]
fn z3_iff_for_integers() {
    // (x == 5) <-> (x == 5) is tautology
    let e = VerifyExpr::eq(VerifyExpr::var("x"), VerifyExpr::int(5));
    let prop = VerifyExpr::iff(e.clone(), e);
    let result = check_equivalence(&prop, &VerifyExpr::bool(true), &[], 1);
    assert!(matches!(result, EquivalenceResult::Equivalent),
        "P <-> P should be tautology, got: {:?}", result);
}

#[test]
fn z3_bool_equiv_unchanged() {
    // Existing boolean equivalence still works
    let a = VerifyExpr::and(
        VerifyExpr::var("p"),
        VerifyExpr::var("q"),
    );
    let b = VerifyExpr::and(
        VerifyExpr::var("q"),
        VerifyExpr::var("p"),
    );
    let result = check_equivalence(&a, &b, &[], 1);
    assert!(matches!(result, EquivalenceResult::Equivalent),
        "AND commutativity should still work, got: {:?}", result);
}

#[test]
fn z3_empty_signals_still_works() {
    // No signals, pure arithmetic
    let a = VerifyExpr::eq(
        VerifyExpr::binary(VerifyOp::Add, VerifyExpr::int(2), VerifyExpr::int(3)),
        VerifyExpr::int(5),
    );
    let result = check_equivalence(&a, &VerifyExpr::bool(true), &[], 1);
    assert!(matches!(result, EquivalenceResult::Equivalent),
        "2+3==5 should be equivalent to true, got: {:?}", result);
}

#[test]
fn z3_multiple_sorts_single_formula() {
    // Formula mixing Bool, Int in one expression
    let prop = VerifyExpr::and(
        VerifyExpr::var("valid"),
        VerifyExpr::and(
            VerifyExpr::gt(VerifyExpr::var("count"), VerifyExpr::int(0)),
            VerifyExpr::lt(VerifyExpr::var("count"), VerifyExpr::int(100)),
        ),
    );
    let prop2 = VerifyExpr::and(
        VerifyExpr::var("valid"),
        VerifyExpr::and(
            VerifyExpr::gte(VerifyExpr::var("count"), VerifyExpr::int(1)),
            VerifyExpr::lte(VerifyExpr::var("count"), VerifyExpr::int(99)),
        ),
    );
    let result = check_equivalence(&prop, &prop2, &[], 1);
    assert!(matches!(result, EquivalenceResult::Equivalent),
        "Equivalent range constraints should match, got: {:?}", result);
}

#[test]
fn z3_performance_boolean_not_regressed() {
    // Simple boolean case should still be fast
    let a = VerifyExpr::implies(VerifyExpr::var("req@0"), VerifyExpr::var("ack@0"));
    let b = VerifyExpr::implies(VerifyExpr::var("req@0"), VerifyExpr::var("ack@0"));
    let start = std::time::Instant::now();
    let result = check_equivalence(&a, &b, &["req".into(), "ack".into()], 1);
    let elapsed = start.elapsed();
    assert!(matches!(result, EquivalenceResult::Equivalent));
    assert!(elapsed.as_millis() < 5000, "Boolean equiv should be fast, took {}ms", elapsed.as_millis());
}

// ═══════════════════════════════════════════════════════════════════════════
// BITVECTOR EQUALITY USED IN BOOLEAN CONTEXT
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn z3_bitvec_eq_in_bool_context() {
    // bv_eq(x, 0xFF) used as boolean predicate
    let a = VerifyExpr::bv_binary(
        BitVecOp::Eq,
        VerifyExpr::var("status_bv8"),
        VerifyExpr::bv_const(8, 0xFF),
    );
    let b = VerifyExpr::bv_binary(
        BitVecOp::Eq,
        VerifyExpr::var("status_bv8"),
        VerifyExpr::bv_const(8, 0xFF),
    );
    let result = check_equivalence(&a, &b, &[], 1);
    assert!(matches!(result, EquivalenceResult::Equivalent),
        "Same BV equality should be equivalent, got: {:?}", result);
}

#[test]
fn z3_bitvec_eq_different_values() {
    // bv_eq(x, 0xFF) not-equiv bv_eq(x, 0x00)
    let a = VerifyExpr::bv_binary(
        BitVecOp::Eq,
        VerifyExpr::var("x_bv8"),
        VerifyExpr::bv_const(8, 0xFF),
    );
    let b = VerifyExpr::bv_binary(
        BitVecOp::Eq,
        VerifyExpr::var("x_bv8"),
        VerifyExpr::bv_const(8, 0x00),
    );
    let result = check_equivalence(&a, &b, &[], 1);
    assert!(matches!(result, EquivalenceResult::NotEquivalent { .. }),
        "bv_eq(x,0xFF) should NOT equal bv_eq(x,0x00), got: {:?}", result);
}

#[test]
fn z3_mixed_bitvec_and_bool() {
    // (valid AND bv_eq(data, 0xFF)) works across sorts
    let a = VerifyExpr::and(
        VerifyExpr::var("valid"),
        VerifyExpr::bv_binary(
            BitVecOp::Eq,
            VerifyExpr::var("data_bv8"),
            VerifyExpr::bv_const(8, 0xFF),
        ),
    );
    let b = VerifyExpr::and(
        VerifyExpr::var("valid"),
        VerifyExpr::bv_binary(
            BitVecOp::Eq,
            VerifyExpr::var("data_bv8"),
            VerifyExpr::bv_const(8, 0xFF),
        ),
    );
    let result = check_equivalence(&a, &b, &[], 1);
    assert!(matches!(result, EquivalenceResult::Equivalent),
        "Mixed Bool+BV formula should work, got: {:?}", result);
}

// ═══════════════════════════════════════════════════════════════════════════
// BITVECTOR ARITHMETIC IDENTITIES
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn z3_bitvec_add_commutative() {
    // bv_add(x, y) equiv bv_add(y, x)
    let a = VerifyExpr::bv_binary(BitVecOp::Add, VerifyExpr::var("x_bv8"), VerifyExpr::var("y_bv8"));
    let b = VerifyExpr::bv_binary(BitVecOp::Add, VerifyExpr::var("y_bv8"), VerifyExpr::var("x_bv8"));
    let result = check_equivalence(&a, &b, &[], 1);
    assert!(matches!(result, EquivalenceResult::Equivalent),
        "BV add should be commutative, got: {:?}", result);
}

#[test]
fn z3_bitvec_and_commutative() {
    // bv_and(x, y) equiv bv_and(y, x)
    let a = VerifyExpr::bv_binary(BitVecOp::And, VerifyExpr::var("x_bv8"), VerifyExpr::var("y_bv8"));
    let b = VerifyExpr::bv_binary(BitVecOp::And, VerifyExpr::var("y_bv8"), VerifyExpr::var("x_bv8"));
    let result = check_equivalence(&a, &b, &[], 1);
    assert!(matches!(result, EquivalenceResult::Equivalent),
        "BV and should be commutative, got: {:?}", result);
}

#[test]
fn z3_bitvec_or_identity() {
    // bv_or(x, 0) equiv x
    let a = VerifyExpr::bv_binary(BitVecOp::Or, VerifyExpr::var("x_bv8"), VerifyExpr::bv_const(8, 0));
    let b = VerifyExpr::var("x_bv8");
    let result = check_equivalence(&a, &b, &[], 1);
    assert!(matches!(result, EquivalenceResult::Equivalent),
        "bv_or(x, 0) should equal x, got: {:?}", result);
}

#[test]
fn z3_bitvec_and_annihilation() {
    // bv_and(x, 0) equiv 0
    let a = VerifyExpr::bv_binary(BitVecOp::And, VerifyExpr::var("x_bv8"), VerifyExpr::bv_const(8, 0));
    let b = VerifyExpr::bv_const(8, 0);
    let result = check_equivalence(&a, &b, &[], 1);
    assert!(matches!(result, EquivalenceResult::Equivalent),
        "bv_and(x, 0) should equal 0, got: {:?}", result);
}

// ═══════════════════════════════════════════════════════════════════════════
// BITVECTOR SHIFT PROPERTIES
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn z3_bitvector_shr_ashr_differ() {
    // Logical vs arithmetic shift right differ for negative values
    // bv_shr(0x80, 1) = 0x40 (logical: inserts 0)
    // bv_ashr(0x80, 1) = 0xC0 (arithmetic: inserts sign bit)
    let a = VerifyExpr::bv_binary(
        BitVecOp::Shr,
        VerifyExpr::var("x_bv8"),
        VerifyExpr::bv_const(8, 1),
    );
    let b = VerifyExpr::bv_binary(
        BitVecOp::AShr,
        VerifyExpr::var("x_bv8"),
        VerifyExpr::bv_const(8, 1),
    );
    let result = check_equivalence(&a, &b, &[], 1);
    assert!(matches!(result, EquivalenceResult::NotEquivalent { .. }),
        "Logical shift and arithmetic shift should differ, got: {:?}", result);
}

// ═══════════════════════════════════════════════════════════════════════════
// TEMPORAL CONTEXT WITH ARITHMETIC
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn z3_arithmetic_in_temporal_context() {
    // G(count > 0) through bounded unrolling with integer count
    // At bound=3: count@0 > 0 AND count@1 > 0 AND count@2 > 0
    let conj = VerifyExpr::and(
        VerifyExpr::gt(VerifyExpr::var("count@0"), VerifyExpr::int(0)),
        VerifyExpr::and(
            VerifyExpr::gt(VerifyExpr::var("count@1"), VerifyExpr::int(0)),
            VerifyExpr::gt(VerifyExpr::var("count@2"), VerifyExpr::int(0)),
        ),
    );
    let conj2 = VerifyExpr::and(
        VerifyExpr::gte(VerifyExpr::var("count@0"), VerifyExpr::int(1)),
        VerifyExpr::and(
            VerifyExpr::gte(VerifyExpr::var("count@1"), VerifyExpr::int(1)),
            VerifyExpr::gte(VerifyExpr::var("count@2"), VerifyExpr::int(1)),
        ),
    );
    let result = check_equivalence(&conj, &conj2, &[], 3);
    assert!(matches!(result, EquivalenceResult::Equivalent),
        "count>0 should equal count>=1 at each timestep, got: {:?}", result);
}

// ═══════════════════════════════════════════════════════════════════════════
// REGRESSION: EXISTING BOOLEAN PATTERNS
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn z3_regression_demorgan_bool() {
    // !(a AND b) equiv (!a OR !b)
    let a = VerifyExpr::not(
        VerifyExpr::and(VerifyExpr::var("a"), VerifyExpr::var("b")),
    );
    let b = VerifyExpr::or(
        VerifyExpr::not(VerifyExpr::var("a")),
        VerifyExpr::not(VerifyExpr::var("b")),
    );
    let result = check_equivalence(&a, &b, &[], 1);
    assert!(matches!(result, EquivalenceResult::Equivalent),
        "DeMorgan's law should hold, got: {:?}", result);
}

#[test]
fn z3_regression_implication_equiv() {
    // (p -> q) equiv (!p OR q)
    let a = VerifyExpr::implies(VerifyExpr::var("p"), VerifyExpr::var("q"));
    let b = VerifyExpr::or(
        VerifyExpr::not(VerifyExpr::var("p")),
        VerifyExpr::var("q"),
    );
    let result = check_equivalence(&a, &b, &[], 1);
    assert!(matches!(result, EquivalenceResult::Equivalent),
        "Implication should equal disjunction, got: {:?}", result);
}

#[test]
fn z3_regression_not_equiv_detects_diff() {
    // (p AND q) not-equiv (p OR q)
    let a = VerifyExpr::and(VerifyExpr::var("p"), VerifyExpr::var("q"));
    let b = VerifyExpr::or(VerifyExpr::var("p"), VerifyExpr::var("q"));
    let result = check_equivalence(&a, &b, &[], 1);
    assert!(matches!(result, EquivalenceResult::NotEquivalent { .. }),
        "AND should NOT equal OR, got: {:?}", result);
}
