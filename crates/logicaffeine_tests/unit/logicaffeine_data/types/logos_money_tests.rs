use super::{LogosDecimal, LogosMoney};

fn money(s: &str, code: &str) -> LogosMoney {
    LogosMoney::of(
        LogosDecimal(logicaffeine_base::Decimal::parse(s).unwrap()),
        code,
    )
}

#[test]
fn money_aot_arithmetic_is_exact_and_currency_safe() {
    assert_eq!(money("19.99", "USD").to_string(), "19.99 USD");
    assert_eq!(LogosMoney::from_i64(5, "USD").to_string(), "5.00 USD");
    assert_eq!(
        money("0.10", "USD").add(&money("0.20", "USD")).to_string(),
        "0.30 USD"
    );
    assert_eq!(
        money("24.99", "USD").sub(&money("5.00", "USD")).to_string(),
        "19.99 USD"
    );
    assert_eq!(money("19.99", "USD").scale_int(3).to_string(), "59.97 USD");
    assert_eq!(money("10.00", "USD").div_int(4).to_string(), "2.50 USD");
    assert_eq!(money("100", "JPY").to_string(), "100 JPY"); // zero-decimal
                                                            // Same currency orders; different currencies are incomparable.
    assert!(money("5.00", "USD") > money("1.00", "USD"));
    assert_eq!(
        money("5.00", "USD").partial_cmp(&money("1.00", "EUR")),
        None
    );
    assert_ne!(money("5.00", "USD"), money("5.00", "EUR"));
}

#[test]
#[should_panic(expected = "different currencies")]
fn money_aot_cross_currency_add_panics_as_a_backstop() {
    let _ = money("5.00", "USD").add(&money("1.00", "EUR"));
}
