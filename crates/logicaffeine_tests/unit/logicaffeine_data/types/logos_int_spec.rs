use super::LogosInt;

#[test]
fn overflow_promotes_and_downsizes_exactly() {
    let max = LogosInt::from_i64(i64::MAX);
    let one = LogosInt::from_i64(1);
    let big = max.add(&one);
    assert_eq!(big.to_string(), "9223372036854775808");
    assert!(matches!(big, LogosInt::Big(_)));
    // Coming back into range NORMALIZES to Small.
    let back = big.sub(&one);
    assert!(matches!(back, LogosInt::Small(_)));
    assert_eq!(back, i64::MAX);
}

#[test]
fn min_div_neg_one_is_exact() {
    let min = LogosInt::from_i64(i64::MIN);
    let neg1 = LogosInt::from_i64(-1);
    let q = min.div(&neg1).unwrap();
    assert_eq!(q.to_string(), "9223372036854775808");
    assert_eq!(min.rem(&neg1).unwrap(), 0i64);
    assert_eq!(min.neg().to_string(), "9223372036854775808");
}

#[test]
fn mul_overflow_matches_exact_value() {
    let max = LogosInt::from_i64(i64::MAX);
    let two = LogosInt::from_i64(2);
    assert_eq!(max.mul(&two).to_string(), "18446744073709551614");
}

#[test]
fn division_errors_are_canonical() {
    let one = LogosInt::from_i64(1);
    let zero = LogosInt::from_i64(0);
    assert_eq!(one.div(&zero).unwrap_err(), "Division by zero");
    assert_eq!(one.rem(&zero).unwrap_err(), "Modulo by zero");
}

#[test]
fn literal_roundtrip_beyond_64_bits() {
    let v = LogosInt::from_literal("18446744073709551614");
    assert_eq!(v.to_string(), "18446744073709551614");
    assert_eq!(LogosInt::from_literal("-42"), LogosInt::from_i64(-42));
}

#[test]
fn ordering_and_eq_cross_representation() {
    let max = LogosInt::from_i64(i64::MAX);
    let big = max.add(&LogosInt::from_i64(1));
    assert!(max < big);
    assert!(big > LogosInt::from_i64(0));
    assert!(LogosInt::from_i64(5) == 5i64);
    assert_ne!(big, LogosInt::from_i64(0));
}
