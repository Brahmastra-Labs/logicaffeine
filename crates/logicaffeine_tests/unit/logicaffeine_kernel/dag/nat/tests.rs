use super::*;

fn n(s: &str) -> Nat {
    Nat::parse_decimal(s).unwrap()
}

#[test]
fn lean_conventions() {
    assert_eq!(n("3").sub(&n("5")), n("0"));
    assert_eq!(n("7").div(&n("0")), n("0"));
    assert_eq!(n("7").rem(&n("0")), n("7"));
    assert_eq!(
        n("18446744073709551615").succ().to_decimal(),
        "18446744073709551616"
    );
    assert!(Nat::parse_decimal("-1").is_none());
    assert!(Nat::parse_decimal("+1").is_none());
}
