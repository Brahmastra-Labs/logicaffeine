use super::*;

#[test]
fn the_i64_window_is_gone_but_the_spelling_is_unchanged() {
    assert_eq!(int_of_constant("0").and_then(|n| n.to_i64()), Some(0));
    assert_eq!(int_of_constant("-7").and_then(|n| n.to_i64()), Some(-7));
    assert_eq!(
        int_of_constant("9223372036854775807").and_then(|n| n.to_i64()),
        Some(i64::MAX)
    );
    // One past the old ceiling: a NUMBER now, and still not an `i64`.
    let big = int_of_constant("9223372036854775808").expect("a numeral");
    assert_eq!(big.to_i64(), None);
    assert_eq!(
        big.add(&BigInt::from_i64(1)),
        int_of_constant("9223372036854775809").expect("a numeral")
    );
    // …and canonical: it must NOT be a `BigInt` when it fits.
    assert!(matches!(
        literal_of_constant("5"),
        Some(logicaffeine_kernel::Literal::Int(5))
    ));
    assert!(matches!(
        literal_of_constant("9223372036854775808"),
        Some(logicaffeine_kernel::Literal::BigInt(_))
    ));
}

#[test]
fn a_name_is_not_a_number() {
    for name in [
        "", "-", "butler", "x1", "1x", " 1", "1 ", "+1", "1_000", "0x10", "1.5",
    ] {
        assert!(
            int_of_constant(name).is_none(),
            "`{name}` must not read as a numeral — the four layers that ask this question \
                 must agree, and `i64::from_str` accepted none of these either"
        );
    }
}
