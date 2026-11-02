//! Unit coverage for the kernel's built-in Int arithmetic/comparison (`prim_arith`) — the
//! checked semantics `native_decide` trusts. Locks the exact value of every op, the
//! boundary of every comparison (`<=`/`<`/`>=`/`>`), the fail-SAFE Stuck on ÷0 and unknown
//! ops, and the BigInt promotion on i64 overflow (never a silently wrapped answer). NOTE: the
//! individual `delete match arm` mutants on prim_arith are EQUIVALENT — every op has a fast
//! i64 arm AND a BigInt-fallback arm, so deleting one leaves the other computing the same
//! value (documented in .config/mutants/known-survivors.txt); these tests kill the OPERATOR mutants.
use super::*;

fn i(n: i64) -> Literal {
    Literal::Int(n)
}
fn expect_int(r: PrimResult, want: i64) {
    match r {
        PrimResult::Int(Literal::Int(v)) => assert_eq!(v, want),
        _ => panic!("expected Int({want})"),
    }
}
fn expect_bool(r: PrimResult, want: bool) {
    match r {
        PrimResult::Bool(v) => assert_eq!(v, want),
        _ => panic!("expected Bool({want})"),
    }
}

#[test]
fn integer_primitives_compute_the_right_value() {
    expect_int(prim_arith("add", &i(2), &i(3)), 5);
    expect_int(prim_arith("sub", &i(10), &i(4)), 6);
    expect_int(prim_arith("mul", &i(6), &i(7)), 42);
    expect_int(prim_arith("div", &i(20), &i(5)), 4);
    expect_int(prim_arith("mod", &i(17), &i(5)), 2);
    expect_int(prim_arith("sub", &i(3), &i(10)), -7);
    expect_int(prim_arith("div", &i(-20), &i(5)), -4);
}

#[test]
fn comparison_primitives_are_exact_at_the_boundary() {
    // Equal operands pin `<=` vs `<` and `>=` vs `>` — kills the off-by-one operator mutants.
    expect_bool(prim_arith("le", &i(3), &i(3)), true);
    expect_bool(prim_arith("lt", &i(3), &i(3)), false);
    expect_bool(prim_arith("ge", &i(3), &i(3)), true);
    expect_bool(prim_arith("gt", &i(3), &i(3)), false);
    expect_bool(prim_arith("le", &i(2), &i(3)), true);
    expect_bool(prim_arith("lt", &i(2), &i(3)), true);
    expect_bool(prim_arith("ge", &i(4), &i(3)), true);
    expect_bool(prim_arith("gt", &i(4), &i(3)), true);
}

/// Division is TOTAL, exactly as it is in Lean: `div : Int → Int → Int` is a total
/// function's type, so a closed well-typed `div 1 0` with no canonical form would be a
/// canonicity failure, not a safe refusal. Lean answers `x / 0 = 0` and `x % 0 = x`, and so
/// do all five of our evaluators. An UNKNOWN operator is the genuinely stuck case, and that
/// is what this test still pins: a name the table does not know must never be guessed at.
#[test]
fn division_by_zero_is_total_like_leans_and_unknown_ops_are_stuck_never_wrong() {
    expect_int(prim_arith("div", &i(1), &i(0)), 0);
    expect_int(prim_arith("mod", &i(1), &i(0)), 1);
    assert!(matches!(
        prim_arith("nonsense", &i(1), &i(1)),
        PrimResult::Stuck
    ));
}

#[test]
fn overflow_promotes_to_bigint_instead_of_wrapping() {
    // i64::MAX + 1 must NOT wrap to i64::MIN; it promotes to an exact BigInt.
    match prim_arith("add", &i(i64::MAX), &i(1)) {
        PrimResult::Int(lit) => {
            let got = lit_bigint(&lit).expect("promoted to a bigint");
            let want = lit_bigint(&i(i64::MAX))
                .unwrap()
                .add(&lit_bigint(&i(1)).unwrap());
            assert_eq!(got, want, "i64::MAX + 1 must be exact, not wrapped");
        }
        _ => panic!("i64 overflow must promote to a BigInt Int, not get Stuck/Bool"),
    }
}

#[test]
fn bigint_path_comparisons_and_arithmetic_are_exact() {
    use logicaffeine_base::BigInt;
    // Operands beyond i64::MAX force the BigInt path (distinct from the fast i64 path) —
    // kills the `<`/`<=`/`>=`/`>` operator mutants and the BigInt `sub`/`add` arm mutants there.
    let big = |n: u64| int_lit(BigInt::from_u64(n));
    let a = big(u64::MAX); // 2^64 − 1  (> i64::MAX ⇒ a BigInt literal)
    let b = big(u64::MAX - 1);
    expect_bool(prim_arith("le", &a, &a), true);
    expect_bool(prim_arith("lt", &a, &a), false);
    expect_bool(prim_arith("ge", &a, &a), true);
    expect_bool(prim_arith("gt", &a, &a), false);
    expect_bool(prim_arith("le", &b, &a), true);
    expect_bool(prim_arith("lt", &b, &a), true);
    expect_bool(prim_arith("gt", &a, &b), true);
    expect_bool(prim_arith("ge", &b, &a), false);
    // a − b = 1 fits i64, exercising the BigInt `sub` arm with a checkable result.
    expect_int(prim_arith("sub", &a, &b), 1);
    // mul/div/mod on the BigInt path (a is BigInt): a·1 = a, a÷a = 1, a mod a = 0.
    match prim_arith("mul", &a, &big(1)) {
        PrimResult::Int(l) => {
            assert_eq!(
                lit_bigint(&l).unwrap(),
                lit_bigint(&a).unwrap(),
                "bigint a·1 = a"
            )
        }
        _ => panic!("bigint mul must compute"),
    }
    expect_int(prim_arith("div", &a, &a), 1);
    expect_int(prim_arith("mod", &a, &a), 0);
}
