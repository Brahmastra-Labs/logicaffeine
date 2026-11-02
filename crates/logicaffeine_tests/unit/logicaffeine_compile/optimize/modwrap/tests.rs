use super::*;

#[test]
fn a_value_below_the_divisor_is_its_own_remainder() {
    assert_eq!(mod_wrap(10, Some((0, 9))), Some(ModWrap::Identity));
    assert_eq!(mod_wrap(10, Some((3, 3))), Some(ModWrap::Identity));
}

#[test]
fn a_value_below_twice_the_divisor_wraps_once() {
    assert_eq!(mod_wrap(10, Some((0, 10))), Some(ModWrap::Once));
    assert_eq!(mod_wrap(10, Some((0, 19))), Some(ModWrap::Once));
    assert_eq!(mod_wrap(1_000_000_007, Some((0, 2_000_000_012))), Some(ModWrap::Once));
}

#[test]
fn every_other_value_divides() {
    // `20 % 10` is 0, where one subtraction leaves 10.
    assert_eq!(mod_wrap(10, Some((0, 20))), None);
    // A negative value's remainder keeps its sign.
    assert_eq!(mod_wrap(10, Some((-1, 5))), None);
    assert_eq!(mod_wrap(10, None), None);
    assert_eq!(mod_wrap(0, Some((0, 5))), None);
    assert_eq!(mod_wrap(-10, Some((0, 5))), None);
}

#[test]
fn the_largest_divisor_does_not_overflow_the_bound() {
    assert_eq!(mod_wrap(i64::MAX, Some((0, i64::MAX as i128))), Some(ModWrap::Once));
    assert_eq!(mod_wrap(i64::MAX, Some((0, i64::MAX as i128 - 1))), Some(ModWrap::Identity));
}
