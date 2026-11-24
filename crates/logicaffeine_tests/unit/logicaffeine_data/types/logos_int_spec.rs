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

#[test]
fn an_exact_integer_meets_a_real_as_its_nearest_float_and_compares_by_its_exact_value() {
    let big = LogosInt::from_i128(1i128 << 70);
    assert_eq!(big.to_f64(), 2f64.powi(70));
    assert_eq!(LogosInt::from_i128(-(1i128 << 70)).to_f64(), -(2f64.powi(70)));
    assert_eq!(LogosInt::from_i64(9_007_199_254_740_993).to_f64(), 9_007_199_254_740_992.0, "ties go to even");
    assert_eq!(LogosInt::from_i128((1i128 << 70) + 1).to_f64(), 2f64.powi(70));
    use std::cmp::Ordering::*;
    assert_eq!(big.cmp_real(2f64.powi(70)), Some(Equal));
    assert_eq!(LogosInt::from_i128((1i128 << 70) + 1).cmp_real(2f64.powi(70)), Some(Greater), "never through a float");
    assert_eq!(big.cmp_real(f64::INFINITY), Some(Less));
    assert_eq!(big.cmp_real(f64::NAN), None);
    assert_eq!(LogosInt::from_i64(9_007_199_254_740_993).cmp_real(9_007_199_254_740_992.0), Some(Greater));
    assert_eq!(LogosInt::from_i64(-3).cmp_real(-2.5), Some(Less));
}

#[test]
fn an_exact_integer_names_a_place_only_within_the_word() {
    assert_eq!(LogosInt::from_i64(7).integer_at("too wide"), Ok(7));
    assert_eq!(LogosInt::from_i128(1i128 << 64).integer_at("too wide"), Err("too wide".to_string()));
}

#[test]
fn an_exact_integer_has_its_exact_magnitude_and_is_true_unless_zero() {
    use crate::ops::Truthy;
    assert_eq!(LogosInt::from_i64(i64::MIN).abs(), LogosInt::from_i128(1i128 << 63));
    assert_eq!(LogosInt::from_i128(-(1i128 << 70)).abs(), LogosInt::from_i128(1i128 << 70));
    assert_eq!(LogosInt::from_i64(-7).abs(), LogosInt::from_i64(7));
    assert_eq!(LogosInt::from_i64(0).abs(), LogosInt::from_i64(0));
    assert!(LogosInt::from_i128(1i128 << 70).truthy());
    assert!(LogosInt::from_i64(-1).truthy());
    assert!(!LogosInt::from_i64(0).truthy());
}

#[test]
fn the_checked_word_magnitude_stops_at_the_least_word_with_the_overflow_sentence() {
    assert_eq!(crate::ops::logos_abs_i64(-7), 7);
    assert_eq!(crate::ops::logos_abs_i64(i64::MAX), i64::MAX);
    assert_eq!(crate::ops::logos_abs_i64(i64::MIN + 1), i64::MAX);
    let stopped = std::panic::catch_unwind(|| crate::ops::logos_abs_i64(i64::MIN)).unwrap_err();
    let why = stopped
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| stopped.downcast_ref::<&str>().map(|s| s.to_string()))
        .unwrap_or_default();
    assert!(why.contains("9223372036854775808"), "the sentence names the value past the word: {why}");
}

#[test]
fn an_exact_range_walks_every_integer_through_its_end_past_the_word() {
    let walked: Vec<LogosInt> =
        crate::ops::logos_exact_range(LogosInt::from_i64(i64::MAX - 1), LogosInt::from_i128(i64::MAX as i128 + 2)).collect();
    let expected: Vec<LogosInt> = (-1i128..=2).map(|d| LogosInt::from_i128(i64::MAX as i128 + d)).collect();
    assert_eq!(walked, expected);
    assert_eq!(crate::ops::logos_exact_range(LogosInt::from_i64(3), LogosInt::from_i64(2)).count(), 0, "an empty range");
    assert_eq!(crate::ops::logos_exact_range(LogosInt::from_i64(-2), LogosInt::from_i64(-2)).count(), 1);
}
