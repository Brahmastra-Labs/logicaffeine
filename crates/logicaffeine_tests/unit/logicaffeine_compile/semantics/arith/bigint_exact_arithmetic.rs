//! Exhaustive coverage of integer arithmetic at and beyond the i64 boundary: math is
//! EXACT on the tree-walker (overflow promotes to BigInt; in-range results downsize
//! to Int), every operand mix (Int/BigInt/Float) is covered, and a dense differential
//! against i128 proves we equal the machine's exact answer wherever it fits.
use super::*;
use logicaffeine_base::BigInt;

fn int(n: i64) -> RuntimeValue {
    RuntimeValue::Int(n)
}
/// `i64::MAX + 1` = 2^63, the smallest value that does not fit i64 — our canonical
/// "just past the boundary" BigInt.
fn two_pow_63() -> RuntimeValue {
    add(int(i64::MAX), int(1)).unwrap()
}
fn disp(v: &RuntimeValue) -> String {
    v.to_display_string()
}
fn is_big(v: &RuntimeValue) -> bool {
    matches!(v, RuntimeValue::BigInt(_))
}

#[test]
fn add_at_the_boundary_promotes_not_wraps() {
    // Arrange: the classic overflow.  Act:
    let r = add(int(i64::MAX), int(1)).unwrap();
    // Assert: the EXACT value, never the wrapped i64::MIN.
    assert!(
        is_big(&r),
        "i64::MAX + 1 must be a BigInt, not a wrapped Int"
    );
    assert_eq!(disp(&r), "9223372036854775808");
    assert_ne!(
        r,
        int(i64::MIN),
        "the JSON/2's-complement footgun must be gone"
    );
}

#[test]
fn subtract_below_the_boundary_promotes() {
    let r = subtract(int(i64::MIN), int(1)).unwrap();
    assert!(is_big(&r));
    assert_eq!(disp(&r), "-9223372036854775809");
}

#[test]
fn multiply_overflow_promotes() {
    let r = multiply(int(i64::MAX), int(2)).unwrap();
    assert_eq!(disp(&r), "18446744073709551614");
    assert!(is_big(&r));
}

#[test]
fn results_that_fit_downsize_back_to_int() {
    // Arrange a BigInt, then bring it back into range.
    let big = two_pow_63(); // 2^63
                            // 2^63 - 1 == i64::MAX, which fits → must be a narrow Int again.
    let back = subtract(big.clone(), int(1)).unwrap();
    assert_eq!(back, int(i64::MAX), "must downsize to Int");
    assert!(!is_big(&back));
    // big + (-big) == 0 → Int(0).
    let zero = add(big.clone(), subtract(int(0), big).unwrap()).unwrap();
    assert_eq!(zero, int(0));
}

#[test]
fn every_operand_mix_is_handled_for_add_sub_mul() {
    let big = two_pow_63(); // 2^63 = 9223372036854775808
                            // Int ∘ BigInt, BigInt ∘ Int, BigInt ∘ BigInt — add.
    assert_eq!(
        disp(&add(int(1), big.clone()).unwrap()),
        "9223372036854775809"
    );
    assert_eq!(
        disp(&add(big.clone(), int(1)).unwrap()),
        "9223372036854775809"
    );
    assert_eq!(
        disp(&add(big.clone(), big.clone()).unwrap()),
        "18446744073709551616"
    ); // 2^64
       // subtract.
    assert_eq!(disp(&subtract(big.clone(), big.clone()).unwrap()), "0");
    // multiply (2^63 * 2 = 2^64).
    assert_eq!(
        disp(&multiply(big.clone(), int(2)).unwrap()),
        "18446744073709551616"
    );
    assert_eq!(
        disp(&multiply(int(2), big.clone()).unwrap()),
        "18446744073709551616"
    );
}

#[test]
fn divide_and_modulo_cover_the_overflow_and_big_operands() {
    // i64::MIN / -1 = 2^63 (the one i64 division overflow) → promotes.
    let q = divide(int(i64::MIN), int(-1)).unwrap();
    assert_eq!(disp(&q), "9223372036854775808");
    assert_eq!(modulo(int(i64::MIN), int(-1)).unwrap(), int(0));
    // BigInt / Int, BigInt % Int.
    let big = two_pow_63(); // 9223372036854775808
    assert_eq!(
        divide(big.clone(), int(2)).unwrap(),
        int(4611686018427387904)
    );
    assert_eq!(modulo(big.clone(), int(2)).unwrap(), int(0));
    // Int / BigInt: a small number over a huge one truncates to 0.
    assert_eq!(divide(int(5), big.clone()).unwrap(), int(0));
    assert_eq!(modulo(int(5), big).unwrap(), int(5));
    // Division by zero is still an error, never a panic.
    assert!(divide(int(1), int(0)).is_err());
    assert!(modulo(int(1), int(0)).is_err());
}

#[test]
fn mixing_a_bigint_with_a_float_yields_a_float() {
    let big = two_pow_63();
    match add(big.clone(), RuntimeValue::Float(1.0)).unwrap() {
        RuntimeValue::Float(f) => assert!((f - 9223372036854775809.0).abs() < 1e9),
        other => panic!("expected Float, got {}", other.type_name()),
    }
    assert!(matches!(
        multiply(RuntimeValue::Float(2.0), big).unwrap(),
        RuntimeValue::Float(_)
    ));
}

#[test]
fn comparison_orders_across_the_narrow_wide_boundary() {
    let big = two_pow_63(); // > every i64
    let neg_big = subtract(int(i64::MIN), int(1)).unwrap(); // < every i64
    let lt = |a: &RuntimeValue, b: &RuntimeValue| {
        matches!(
            super::super::compare::compare(BinaryOpKind::Lt, a, b).unwrap(),
            RuntimeValue::Bool(true)
        )
    };
    assert!(lt(&int(i64::MAX), &big), "i64::MAX < 2^63");
    assert!(lt(&neg_big, &int(i64::MIN)), "-(2^63+1) < i64::MIN");
    assert!(lt(&neg_big, &big), "huge negative < huge positive");
    // equal BigInts are not less-than each other.
    assert!(!lt(&big.clone(), &big));
}

#[test]
fn equality_and_hashing_are_consistent_for_bigints() {
    use std::collections::HashSet;
    let a = two_pow_63();
    let b = add(int(1), int(i64::MAX)).unwrap(); // same value, different path
    assert_eq!(a, b, "equal BigInts compare equal");
    assert_ne!(a, int(0), "a BigInt is never equal to an Int");
    // Hash agreement: a HashSet dedups two equal BigInts to one entry.
    let mut set = HashSet::new();
    set.insert(a);
    set.insert(b);
    assert_eq!(set.len(), 1, "equal BigInts must hash-collapse");
}

#[test]
fn dense_differential_against_i128_through_the_arith_layer() {
    // For every pair whose true result fits i128, our Int/BigInt arithmetic — with
    // promotion AND downsizing — must equal the machine's exact answer.
    let xs: [i64; 9] = [
        0,
        1,
        -1,
        7,
        -7,
        i32::MAX as i64,
        i32::MIN as i64,
        i64::MAX,
        i64::MIN,
    ];
    for &x in &xs {
        for &y in &xs {
            assert_eq!(
                disp(&add(int(x), int(y)).unwrap()),
                (x as i128 + y as i128).to_string(),
                "{x}+{y}"
            );
            assert_eq!(
                disp(&subtract(int(x), int(y)).unwrap()),
                (x as i128 - y as i128).to_string(),
                "{x}-{y}"
            );
            assert_eq!(
                disp(&multiply(int(x), int(y)).unwrap()),
                (x as i128 * y as i128).to_string(),
                "{x}*{y}"
            );
            if y != 0 {
                assert_eq!(
                    disp(&divide(int(x), int(y)).unwrap()),
                    (x as i128 / y as i128).to_string(),
                    "{x}/{y}"
                );
                assert_eq!(
                    disp(&modulo(int(x), int(y)).unwrap()),
                    (x as i128 % y as i128).to_string(),
                    "{x}%{y}"
                );
            }
        }
    }
}

#[test]
fn promoted_values_round_trip_through_a_negation_chain() {
    // A stress walk: repeatedly add 1 starting from i64::MAX-1, crossing the
    // boundary, then subtract back down — every value exact, Int↔BigInt seamless.
    let mut v = int(i64::MAX - 1);
    for _ in 0..4 {
        v = add(v, int(1)).unwrap();
    }
    assert_eq!(disp(&v), "9223372036854775810"); // i64::MAX-1 + 4 = 2^63 + 1
    assert!(is_big(&v));
    for _ in 0..4 {
        v = subtract(v, int(1)).unwrap();
    }
    assert_eq!(v, int(i64::MAX - 1), "back to the exact narrow value");
    assert!(!is_big(&v));
}

#[test]
fn fuzz_promotion_layer_matches_i128_and_obeys_algebra() {
    // Deterministic SplitMix64 — reproducible, no external dependency.
    let mut state = 0x5DEE_CE66_1357_2468u64;
    let mut next = || {
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    };
    for _ in 0..4000 {
        let (x, y) = (next() as i64, next() as i64);
        // Differential against i128 THROUGH the promote/downsize layer.
        assert_eq!(
            disp(&add(int(x), int(y)).unwrap()),
            (x as i128 + y as i128).to_string(),
            "{x}+{y}"
        );
        assert_eq!(
            disp(&subtract(int(x), int(y)).unwrap()),
            (x as i128 - y as i128).to_string(),
            "{x}-{y}"
        );
        assert_eq!(
            disp(&multiply(int(x), int(y)).unwrap()),
            (x as i128 * y as i128).to_string(),
            "{x}*{y}"
        );
        if y != 0 {
            assert_eq!(
                disp(&divide(int(x), int(y)).unwrap()),
                (x as i128 / y as i128).to_string(),
                "{x}/{y}"
            );
            assert_eq!(
                disp(&modulo(int(x), int(y)).unwrap()),
                (x as i128 % y as i128).to_string(),
                "{x}%{y}"
            );
        }
        // Commutativity survives promotion.
        assert_eq!(add(int(x), int(y)).unwrap(), add(int(y), int(x)).unwrap());
        assert_eq!(
            multiply(int(x), int(y)).unwrap(),
            multiply(int(y), int(x)).unwrap()
        );
        // The downsizing invariant: a sum is a BigInt IFF it does not fit i64.
        let exact = x as i128 + y as i128;
        let fits = (i64::MIN as i128..=i64::MAX as i128).contains(&exact);
        assert_eq!(
            is_big(&add(int(x), int(y)).unwrap()),
            !fits,
            "BigInt iff out of i64 range for {x}+{y}"
        );
    }
}

#[test]
fn from_bigint_constructor_maintains_the_downsizing_invariant() {
    // A BigInt that fits i64 must NOT be wrapped in the BigInt variant.
    assert_eq!(RuntimeValue::from_bigint(BigInt::from_i64(42)), int(42));
    assert!(!is_big(&RuntimeValue::from_bigint(BigInt::from_i64(
        i64::MIN
    ))));
    // One that does not fit stays BigInt.
    assert!(is_big(&RuntimeValue::from_bigint(
        BigInt::from_i64(i64::MAX).add(&BigInt::from_i64(1))
    )));
}
