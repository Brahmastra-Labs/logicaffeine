use super::*;

fn money(s: &str, code: &str) -> Money {
    Money::of(Decimal::parse(s).unwrap(), currency::by_code(code).unwrap())
}

#[test]
fn currency_catalog_resolves_codes_with_their_iso_scales() {
    assert_eq!(currency::by_code("USD").unwrap().scale, 2);
    assert_eq!(currency::by_code("usd").unwrap().scale, 2); // case-insensitive
    assert_eq!(currency::by_code("JPY").unwrap().scale, 0); // zero-decimal
    assert_eq!(currency::by_code("BHD").unwrap().scale, 3); // three-decimal
    assert_eq!(currency::by_code("KWD").unwrap().scale, 3);
    assert_eq!(currency::by_code("XYZ"), None);
}

#[test]
fn money_quantises_to_the_currency_minor_unit() {
    assert_eq!(money("19.99", "USD").to_string(), "19.99 USD");
    assert_eq!(money("5", "USD").to_string(), "5.00 USD"); // padded to 2dp
    assert_eq!(money("100", "JPY").to_string(), "100 JPY"); // no decimals
    assert_eq!(money("1.5", "BHD").to_string(), "1.500 BHD"); // 3dp
                                                              // Banker's rounding to the minor unit.
    assert_eq!(money("19.999", "USD").to_string(), "20.00 USD");
    assert_eq!(money("2.345", "USD").to_string(), "2.34 USD"); // HalfEven: 4 is even
}

#[test]
fn same_currency_add_sub_are_exact_and_keep_the_currency() {
    // The "JSON numbers ruin lives" footgun, gone: 0.10 + 0.20 is EXACTLY 0.30.
    assert_eq!(
        money("0.10", "USD")
            .add(&money("0.20", "USD"))
            .unwrap()
            .to_string(),
        "0.30 USD"
    );
    assert_eq!(
        money("19.99", "USD")
            .add(&money("5.00", "USD"))
            .unwrap()
            .to_string(),
        "24.99 USD"
    );
    assert_eq!(
        money("24.99", "USD")
            .sub(&money("5.00", "USD"))
            .unwrap()
            .to_string(),
        "19.99 USD"
    );
    assert_eq!(
        money("100", "JPY")
            .add(&money("50", "JPY"))
            .unwrap()
            .to_string(),
        "150 JPY"
    );
}

#[test]
fn cross_currency_arithmetic_is_forbidden() {
    // A dollar plus a euro has no answer — like meter + gram.
    assert_eq!(money("5.00", "USD").add(&money("1.00", "EUR")), None);
    assert_eq!(money("5.00", "USD").sub(&money("1.00", "EUR")), None);
    assert_eq!(money("5.00", "USD").ratio(&money("1.00", "EUR")), None);
}

#[test]
fn scaling_and_ratio() {
    assert_eq!(money("19.99", "USD").scale_int(3).to_string(), "59.97 USD");
    assert_eq!(
        money("30.00", "USD").ratio(&money("10.00", "USD")).unwrap(),
        Rational::from_i64(3)
    );
    assert_eq!(money("1.00", "USD").ratio(&money("0.00", "USD")), None); // divide by zero
}

#[test]
fn universal_money_amount_converts_exactly_via_a_rate_table() {
    // Reference = USD. 1 EUR = 1.10 USD, 1 GBP = 1.25 USD.
    let mut rates = RateTable::new();
    rates.set("USD", Rational::from_i64(1));
    rates.set("EUR", Rational::from_ratio_i64(11, 10).unwrap());
    rates.set("GBP", Rational::from_ratio_i64(5, 4).unwrap());
    let usd = currency::by_code("USD").unwrap();
    let eur = currency::by_code("EUR").unwrap();
    let gbp = currency::by_code("GBP").unwrap();

    // 10 EUR → USD = 11.00 USD; and back to EUR = exactly 10.00 (lossless round-trip).
    assert_eq!(
        rates
            .convert(&money("10.00", "EUR"), usd)
            .unwrap()
            .to_string(),
        "11.00 USD"
    );
    assert_eq!(
        rates
            .convert(&money("11.00", "USD"), eur)
            .unwrap()
            .to_string(),
        "10.00 EUR"
    );
    // Converting to the same currency is the identity.
    assert_eq!(
        rates
            .convert(&money("42.00", "USD"), usd)
            .unwrap()
            .to_string(),
        "42.00 USD"
    );
    // Cross-rate via the reference: 10 GBP = 12.50 USD = 12.50/1.10 EUR ≈ 11.36 EUR.
    assert_eq!(
        rates
            .convert(&money("10.00", "GBP"), eur)
            .unwrap()
            .to_string(),
        "11.36 EUR"
    );
    // An unknown currency yields None, never a wrong number.
    assert_eq!(
        rates.convert(&money("1.00", "USD"), currency::by_code("JPY").unwrap()),
        None
    );
}

#[test]
fn value_equality() {
    assert_eq!(money("5", "USD"), money("5.00", "USD")); // value-equal across input forms
    assert_ne!(money("5.00", "USD"), money("5.00", "EUR")); // currency matters
    assert_ne!(money("5.00", "USD"), money("6.00", "USD"));
}
