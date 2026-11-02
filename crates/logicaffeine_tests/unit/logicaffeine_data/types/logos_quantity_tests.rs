use super::{LogosQuantity, LogosRational};

#[test]
fn golden_two_inches_plus_five_cm_in_feet_is_exactly_42_over_127() {
    let a = LogosQuantity::of(2, "inch");
    let b = LogosQuantity::of(5, "centimeter");
    assert_eq!(a.to_string(), "2 in");
    assert_eq!(a.add(&b).convert("foot").to_string(), "42/127 ft");
}

#[test]
fn arithmetic_scaling_equality_and_ordering_are_exact() {
    let m = |v: i64, u: &str| LogosQuantity::of(v, u);
    // Same-dimension subtraction keeps the left operand's unit, exactly.
    assert_eq!(m(1, "meter").sub(&m(50, "centimeter")).to_string(), "1/2 m");
    // Scalar scaling preserves the unit.
    assert_eq!(
        m(2, "inch").scale(&LogosRational::from_i64(3)).to_string(),
        "6 in"
    );
    assert_eq!(
        m(6, "inch")
            .div_scalar(&LogosRational::from_i64(2))
            .to_string(),
        "3 in"
    );
    // Dimension-combining product shows in dimension form.
    assert_eq!(m(3, "meter").mul(&m(4, "meter")).to_string(), "12 L^2");
    assert_eq!(
        m(100, "meter").div_exact(&m(10, "second")).to_string(),
        "10 L·T^-1"
    );
    // Physical equality (display unit ignored): 100 cm == 1 m.
    assert_eq!(m(100, "centimeter"), m(1, "meter"));
    // Ordering by magnitude within a shared dimension.
    assert!(m(2, "meter") > m(1, "meter"));
    assert!(m(100, "centimeter") <= m(1, "meter"));
    // Cross-dimension ordering is undefined (no shared magnitude).
    assert_eq!(m(1, "meter").partial_cmp(&m(1, "kilogram")), None);
}

#[test]
#[should_panic(expected = "different dimensions")]
fn adding_different_dimensions_panics_as_a_backstop() {
    let _ = LogosQuantity::of(1, "meter").add(&LogosQuantity::of(1, "kilogram"));
}

#[test]
#[should_panic(expected = "cannot convert a m quantity to 'kg' — different dimension")]
fn converting_across_dimensions_panics_as_a_backstop() {
    let _ = LogosQuantity::of(1, "meter").convert("kilogram");
}
