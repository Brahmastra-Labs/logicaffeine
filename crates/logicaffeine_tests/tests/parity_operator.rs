//! Stdlib-parity north-star corpus - domain: operator
//!
//! Named function forms of the built-in operators (Python's `operator` module), realized in pure
//! LOGOS (`operator.lg`) and proven byte-identical on every execution tier - so they can be passed
//! to higher-order routines (map/filter/reduce).
#![allow(unused_imports, dead_code)]

mod common;
use common::assert_all_tiers;

/// Function form of `add`. py: operator.add
#[test]
fn parity_operator_add() {
    assert_all_tiers("## Main\nShow opAdd(3, 4).\n", "7");
}

/// Function form of `sub`. py: operator.sub
#[test]
fn parity_operator_sub() {
    assert_all_tiers("## Main\nShow opSub(10, 3).\n", "7");
}

/// Function form of `mul`. py: operator.mul
#[test]
fn parity_operator_mul() {
    assert_all_tiers("## Main\nShow opMul(6, 7).\n", "42");
}

/// Function form of `floordiv`. py: operator.floordiv
#[test]
fn parity_operator_floordiv() {
    assert_all_tiers("## Main\nShow opFloorDiv(17, 5).\n", "3");
}

/// Function form of `mod`. py: operator.mod
#[test]
fn parity_operator_mod() {
    assert_all_tiers("## Main\nShow opMod(17, 5).\n", "2");
}

/// Function form of `neg`. py: operator.neg
#[test]
fn parity_operator_neg() {
    assert_all_tiers("## Main\nShow opNeg(9).\n", "-9");
}

/// Function form of `abs`. py: operator.abs
#[test]
fn parity_operator_abs() {
    assert_all_tiers("## Main\nShow opAbs(0 - 8).\n", "8");
}

/// Function form of `and`. py: operator.and_
#[test]
fn parity_operator_and() {
    assert_all_tiers("## Main\nShow opAnd(12, 10).\n", "8");
}

/// Function form of `or`. py: operator.or_
#[test]
fn parity_operator_or() {
    assert_all_tiers("## Main\nShow opOr(12, 3).\n", "15");
}

/// Function form of `xor`. py: operator.xor
#[test]
fn parity_operator_xor() {
    assert_all_tiers("## Main\nShow opXor(6, 3).\n", "5");
}

/// Function form of `lshift`. py: operator.lshift
#[test]
fn parity_operator_lshift() {
    assert_all_tiers("## Main\nShow opLshift(1, 4).\n", "16");
}

/// Function form of `rshift`. py: operator.rshift
#[test]
fn parity_operator_rshift() {
    assert_all_tiers("## Main\nShow opRshift(255, 3).\n", "31");
}

/// Function form of `lt`. py: operator.lt
#[test]
fn parity_operator_lt() {
    assert_all_tiers("## Main\nShow opLt(3, 5).\n", "true");
}

/// Function form of `le`. py: operator.le
#[test]
fn parity_operator_le() {
    assert_all_tiers("## Main\nShow opLe(5, 5).\n", "true");
}

/// Function form of `gt`. py: operator.gt
#[test]
fn parity_operator_gt() {
    assert_all_tiers("## Main\nShow opGt(5, 3).\n", "true");
}

/// Function form of `ge`. py: operator.ge
#[test]
fn parity_operator_ge() {
    assert_all_tiers("## Main\nShow opGe(5, 5).\n", "true");
}

/// Function form of `eq`. py: operator.eq
#[test]
fn parity_operator_eq() {
    assert_all_tiers("## Main\nShow opEq(4, 4).\n", "true");
}

/// Function form of `ne`. py: operator.ne
#[test]
fn parity_operator_ne() {
    assert_all_tiers("## Main\nShow opNe(4, 5).\n", "true");
}

/// Logical negation. py: operator.not_
#[test]
fn parity_operator_not() {
    assert_all_tiers(
        "## Main\nShow opNot(true).\nShow opNot(false).\n",
        "false\ntrue",
    );
}

/// Truth value. py: operator.truth
#[test]
fn parity_operator_truth() {
    assert_all_tiers("## Main\nShow opTruth(true).\n", "true");
}

/// Unary plus. py: operator.pos
#[test]
fn parity_operator_pos() {
    assert_all_tiers("## Main\nShow opPos(5).\n", "5");
}

/// Bitwise inversion (~x = -x-1). py: operator.invert / operator.inv
#[test]
fn parity_operator_invert() {
    assert_all_tiers("## Main\nShow opInvert(5).\nShow opInvert(0).\n", "-6\n-1");
}

/// True (floating) division. py: operator.truediv
#[test]
fn parity_operator_truediv() {
    assert_all_tiers("## Main\nShow opTrueDiv(7.0, 2.0).\n", "3.5");
}
