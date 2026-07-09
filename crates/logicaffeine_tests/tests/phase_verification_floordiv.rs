//! Floor division `//` is modeled EXACTLY in the verification IR — not declined.
//!
//! `BinaryOpKind::FloorDivide` lowers to `VerifyOp::FloorDiv`, encoded as
//! `to_int(to_real(a) / to_real(b))` (Z3's `to_int` is the floor function), which is precise
//! toward negative infinity for every sign — unlike the Euclidean `VerifyOp::Div`, whose rounding
//! only coincides with floor when the divisor is positive. These tests make Z3 PROVE the encoding:
//! each concrete floor value is proven valid and its off-by-one neighbour refuted, the divergence
//! from Euclidean division on a negative divisor is proven, and the defining floor identity is
//! proven to hold symbolically.
//!
//! Requires the `verification` feature (Z3).

#![cfg(feature = "verification")]

use logicaffeine_verify::{VerificationSession, VerifyExpr, VerifyOp, VerifyType};

fn floordiv(a: VerifyExpr, b: VerifyExpr) -> VerifyExpr {
    VerifyExpr::binary(VerifyOp::FloorDiv, a, b)
}

#[test]
fn floordiv_is_modeled_exactly_across_the_sign_matrix() {
    let session = VerificationSession::new();
    // (a, b, floor(a/b)) — every sign combination, including a zero dividend.
    for (a, b, expected) in [
        (7, 2, 3),
        (8, 3, 2),
        (-7, 2, -4),
        (7, -2, -4),
        (-7, -2, 3),
        (0, 5, 0),
        (-1, 5, -1),
        (1, -5, -1),
    ] {
        let q = floordiv(VerifyExpr::int(a), VerifyExpr::int(b));
        // Z3 must PROVE the floored value.
        let correct = VerifyExpr::eq(q.clone(), VerifyExpr::int(expected));
        assert!(
            session.verify(&correct).is_ok(),
            "Z3 must prove {a} // {b} == {expected}"
        );
        // ...and REFUTE its off-by-one neighbour (the truncation answer, where they differ).
        let wrong = VerifyExpr::eq(q, VerifyExpr::int(expected + 1));
        assert!(
            session.verify(&wrong).is_err(),
            "Z3 must refute {a} // {b} == {}",
            expected + 1
        );
    }
}

#[test]
fn floordiv_diverges_from_euclidean_div_on_a_negative_divisor() {
    // The whole reason `FloorDiv` is its OWN op: for a negative divisor it floors toward -inf
    // (`7 // -2 == -4`), while the Euclidean `VerifyOp::Div` rounds toward the non-negative
    // remainder (`7 div -2 == -3`). Z3 proves each — so a decline-to-`Div` would have been unsound.
    let session = VerificationSession::new();
    let fd = floordiv(VerifyExpr::int(7), VerifyExpr::int(-2));
    let ed = VerifyExpr::binary(VerifyOp::Div, VerifyExpr::int(7), VerifyExpr::int(-2));
    assert!(session.verify(&VerifyExpr::eq(fd, VerifyExpr::int(-4))).is_ok(), "7 // -2 == -4");
    assert!(session.verify(&VerifyExpr::eq(ed, VerifyExpr::int(-3))).is_ok(), "7 div -2 == -3 (Euclidean)");
}

#[test]
fn floordiv_defining_identity_holds_symbolically() {
    // For a symbolic dividend and a fixed positive divisor, prove the floor bracket:
    //   (a // 4) * 4 <= a   AND   a < (a // 4) * 4 + 4.
    // A constant divisor keeps the mixed int/real reasoning decidable for Z3.
    let mut session = VerificationSession::new();
    session.declare("a", VerifyType::Int);
    let q = floordiv(VerifyExpr::var("a"), VerifyExpr::int(4));
    let qb = VerifyExpr::binary(VerifyOp::Mul, q, VerifyExpr::int(4));
    // (a // 4) * 4 <= a
    assert!(
        session.verify(&VerifyExpr::lte(qb.clone(), VerifyExpr::var("a"))).is_ok(),
        "floor bracket lower bound: (a // 4) * 4 <= a"
    );
    // a < (a // 4) * 4 + 4
    let qb_plus_4 = VerifyExpr::binary(VerifyOp::Add, qb, VerifyExpr::int(4));
    assert!(
        session.verify(&VerifyExpr::lt(VerifyExpr::var("a"), qb_plus_4)).is_ok(),
        "floor bracket upper bound: a < (a // 4) * 4 + 4"
    );
}
