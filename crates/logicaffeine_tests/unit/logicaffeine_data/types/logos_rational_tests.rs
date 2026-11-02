use super::LogosRational;

#[test]
fn exact_fraction_displays_unreduced_pair() {
    assert_eq!(LogosRational::from_ratio(7, 2).to_string(), "7/2");
    assert_eq!(LogosRational::from_ratio(1, 3).to_string(), "1/3");
}

#[test]
fn whole_value_displays_as_a_bare_integer() {
    assert_eq!(LogosRational::from_ratio(6, 2).to_string(), "3");
    assert_eq!(LogosRational::from_i64(5).to_string(), "5");
}

#[test]
fn arithmetic_is_exact() {
    // 1/3 + 1/6 = 1/2
    let a = LogosRational::from_ratio(1, 3);
    let b = LogosRational::from_ratio(1, 6);
    assert_eq!(a.add(&b).to_string(), "1/2");
    // 1/3 + 1/3 + 1/3 = 1 (where 0.1+0.2 ≠ 0.3 in f64)
    let third = LogosRational::from_ratio(1, 3);
    assert_eq!(third.add(&third).add(&third).to_string(), "1");
    // (2/3) * (3/4) = 1/2 ; (7/2) / (7/2) = 1
    assert_eq!(
        LogosRational::from_ratio(2, 3)
            .mul(&LogosRational::from_ratio(3, 4))
            .to_string(),
        "1/2"
    );
    let r = LogosRational::from_ratio(7, 2);
    assert_eq!(r.div_exact(&r).to_string(), "1");
}

#[test]
#[should_panic(expected = "Division by zero")]
fn zero_denominator_panics_like_integer_division() {
    let _ = LogosRational::from_ratio(1, 0);
}

fn q(n: i64, d: i64) -> LogosRational {
    LogosRational::from_ratio(n, d)
}

fn whole(n: i64) -> LogosRational {
    LogosRational::from_i64(n)
}

#[test]
fn a_quotient_of_two_whole_values_truncates_as_integers_do() {
    assert_eq!(whole(7).quotient(&whole(2)).to_string(), "3");
    assert_eq!(whole(-7).quotient(&whole(2)).to_string(), "-3");
    assert_eq!(q(14, 2).quotient(&whole(4)).to_string(), "1", "a whole value made as a fraction is the integer");
}

#[test]
fn a_quotient_with_a_fraction_is_exact() {
    assert_eq!(q(1, 3).quotient(&whole(2)).to_string(), "1/6");
    assert_eq!(whole(1).quotient(&q(1, 2)).to_string(), "2");
    assert_eq!(q(3, 4).quotient(&q(1, 4)).to_string(), "3");
}

#[test]
#[should_panic(expected = "Division by zero")]
fn a_quotient_by_zero_stops() {
    let _ = whole(1).quotient(&whole(0));
}

#[test]
fn a_remainder_of_two_whole_values_keeps_the_dividends_sign() {
    assert_eq!(whole(7).remainder(&whole(3)).to_string(), "1");
    assert_eq!(whole(-7).remainder(&whole(3)).to_string(), "-1");
    assert_eq!(q(21, 3).remainder(&q(6, 3)).to_string(), "1");
}

#[test]
#[should_panic(expected = "Cannot compute modulo of Rational and Int")]
fn a_fraction_has_no_remainder() {
    let _ = q(1, 3).remainder(&whole(2));
}

#[test]
#[should_panic(expected = "Modulo by zero")]
fn a_remainder_by_zero_stops() {
    let _ = whole(7).remainder(&whole(0));
}

#[test]
fn a_floor_quotient_is_the_floor_of_the_exact_quotient() {
    assert_eq!(whole(-7).floor_quotient(&whole(2)).to_string(), "-4");
    assert_eq!(q(7, 2).floor_quotient(&whole(1)).to_string(), "3");
    assert_eq!(q(-1, 3).floor_quotient(&q(1, 2)).to_string(), "-1");
}

#[test]
fn a_whole_base_takes_an_integers_power_and_a_fraction_a_rationals() {
    assert_eq!(whole(2).power(&whole(3)).to_string(), "8");
    assert_eq!(whole(2).power(&whole(70)).to_string(), "1180591620717411303424", "past i64 stays exact");
    assert_eq!(q(1, 2).power(&whole(2)).to_string(), "1/4");
    assert_eq!(q(1, 2).power(&whole(-1)).to_string(), "2");
}

#[test]
#[should_panic(expected = "negative exponent on an integer")]
fn a_whole_base_refuses_a_negative_power() {
    let _ = q(3, 3).power(&whole(-1));
}

#[test]
fn bitwise_operators_take_two_whole_values_as_integers() {
    assert_eq!(whole(6).bit_and(&whole(3)), 2);
    assert_eq!(q(12, 2).bit_or(&whole(1)), 7);
    assert_eq!(whole(6).bit_xor(&whole(3)), 5);
    assert_eq!(whole(1).shl(&whole(4)), 16);
    assert_eq!(whole(-16).shr(&whole(2)), -4);
}

#[test]
#[should_panic(expected = "requires integer, boolean, or Set operands")]
fn a_fraction_refuses_a_bitwise_operator() {
    let _ = q(1, 2).bit_and(&whole(1));
}

#[test]
fn a_rational_meets_a_real_by_the_reals_exact_value() {
    use std::cmp::Ordering;
    assert_eq!(q(1, 3).to_f64(), 1.0 / 3.0);
    assert_eq!(q(1, 3).order_real(1.0 / 3.0), Some(Ordering::Greater), "1/3 exceeds the double nearest it");
    assert_eq!(q(1, 2).order_real(0.5), Some(Ordering::Equal));
    assert_eq!(whole(2).order_real(f64::INFINITY), Some(Ordering::Less));
    assert_eq!(whole(2).order_real(f64::NAN), None);
}

#[test]
fn an_integer_place_takes_a_whole_value_as_its_integer() {
    assert_eq!(q(6, 3).integer("past a word", "a fraction"), 2);
    assert_eq!(whole(-5).integer("past a word", "a fraction"), -5);
    assert_eq!(whole(i64::MIN).integer("past a word", "a fraction"), i64::MIN);
}

#[test]
#[should_panic(expected = "a fraction")]
fn an_integer_place_refuses_a_fraction_in_its_own_words() {
    let _ = q(1, 2).integer("past a word", "a fraction");
}

#[test]
#[should_panic(expected = "past a word")]
fn an_integer_place_refuses_a_value_past_a_word_in_its_own_words() {
    let _ = whole(i64::MAX).add(&whole(1)).integer("past a word", "a fraction");
}

#[test]
fn integers_held_as_rationals_are_the_rationals_they_equal_at_any_depth() {
    use crate::{LogosInt, LogosSeq, ToRationals};
    assert_eq!(7i64.to_rationals(), whole(7));
    assert_eq!(q(1, 3).to_rationals(), q(1, 3));
    let xs = LogosSeq::from_vec(vec![1i64, -2, 3]).to_rationals();
    assert_eq!(xs.0.borrow().clone(), vec![whole(1), whole(-2), whole(3)]);
    let grid = LogosSeq::from_vec(vec![LogosSeq::from_vec(vec![4i64]), LogosSeq::from_vec(vec![])]).to_rationals();
    assert_eq!(grid.0.borrow()[0].0.borrow().clone(), vec![whole(4)]);
    assert!(grid.0.borrow()[1].0.borrow().is_empty());
    assert_eq!(vec![5i64].to_rationals().0.borrow().clone(), vec![whole(5)]);
    assert_eq!([6i64][..].to_rationals().0.borrow().clone(), vec![whole(6)]);
    let big = LogosInt::from(i64::MAX).add(&LogosInt::from(1));
    assert_eq!(LogosRational::from(big).to_string(), "9223372036854775808");
    assert_eq!(LogosRational::from(LogosInt::from(-3)), whole(-3));
}
