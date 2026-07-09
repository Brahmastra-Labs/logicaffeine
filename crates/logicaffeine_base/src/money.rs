//! Money — an exact monetary amount in a specific currency (UNIVERSAL_TYPES Part V).
//!
//! Money rides the exact [`Decimal`] tower, so it **never float-drifts**: `0.10 + 0.20` is exactly
//! `0.30`, not `0.30000000000000004`. The amount is quantised to the currency's minor unit (USD has
//! 2 fractional digits, JPY has 0, BHD has 3), with banker's rounding.
//!
//! Currency is part of the value, and same-currency arithmetic is the only kind that means anything:
//! `5 USD + 1 EUR` has no answer, so `add`/`sub` across currencies return `None` — the exact analogue
//! of the dimensional rule that forbids `meter + gram`. `× ÷` by a plain number scales the amount; a
//! same-currency `Money ÷ Money` is an exact dimensionless ratio.

use core::fmt;
use core::hash::{Hash, Hasher};

use crate::numeric::{Decimal, Rational, RoundingMode};

/// An ISO-4217 currency: its three-letter code and its minor-unit `scale` (the number of fractional
/// digits — `USD`→2, `JPY`→0, `BHD`→3). The code is the identity.
#[derive(Clone, Copy, Debug)]
pub struct Currency {
    pub code: &'static str,
    pub scale: u32,
}

impl PartialEq for Currency {
    fn eq(&self, other: &Self) -> bool {
        self.code == other.code
    }
}
impl Eq for Currency {}
impl Hash for Currency {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.code.hash(state);
    }
}

/// The currency catalog — resolve an ISO-4217 code (case-insensitive) to its [`Currency`]. A growable
/// data table, mirroring [`crate::quantity::units::by_name`]; the minor-unit scales follow ISO-4217.
pub mod currency {
    use super::Currency;

    /// Look up a currency by its ISO-4217 code (case-insensitive); `None` for an unknown code.
    pub fn by_code(code: &str) -> Option<Currency> {
        let c = code.trim().to_ascii_uppercase();
        let cur = |code, scale| Currency { code, scale };
        Some(match c.as_str() {
            "USD" => cur("USD", 2),
            "EUR" => cur("EUR", 2),
            "GBP" => cur("GBP", 2),
            "CHF" => cur("CHF", 2),
            "CAD" => cur("CAD", 2),
            "AUD" => cur("AUD", 2),
            "NZD" => cur("NZD", 2),
            "SGD" => cur("SGD", 2),
            "HKD" => cur("HKD", 2),
            "CNY" => cur("CNY", 2),
            "INR" => cur("INR", 2),
            "BRL" => cur("BRL", 2),
            "MXN" => cur("MXN", 2),
            "ZAR" => cur("ZAR", 2),
            "SEK" => cur("SEK", 2),
            "NOK" => cur("NOK", 2),
            "DKK" => cur("DKK", 2),
            "RUB" => cur("RUB", 2),
            "TRY" => cur("TRY", 2),
            "PLN" => cur("PLN", 2),
            // Zero-decimal currencies (no minor unit).
            "JPY" => cur("JPY", 0),
            "KRW" => cur("KRW", 0),
            "ISK" => cur("ISK", 0),
            "VND" => cur("VND", 0),
            "CLP" => cur("CLP", 0),
            // Three-decimal currencies.
            "BHD" => cur("BHD", 3),
            "KWD" => cur("KWD", 3),
            "OMR" => cur("OMR", 3),
            "JOD" => cur("JOD", 3),
            "TND" => cur("TND", 3),
            _ => return None,
        })
    }
}

/// An exact monetary amount, quantised to its currency's minor unit.
#[derive(Clone, Debug)]
pub struct Money {
    /// The amount, exact, at `currency.scale` fractional digits.
    pub amount: Decimal,
    pub currency: Currency,
}

impl Money {
    /// Build money, quantising the amount to the currency's minor unit with banker's rounding —
    /// `5 USD` is stored as `5.00`, `19.999 USD` rounds to `20.00`, `100 JPY` stays `100`.
    pub fn of(amount: Decimal, currency: Currency) -> Money {
        Money { amount: amount.rescale(currency.scale, RoundingMode::HalfEven), currency }
    }

    /// Sum of two amounts in the **same** currency; `None` across currencies (no common meaning).
    pub fn add(&self, other: &Money) -> Option<Money> {
        if self.currency != other.currency {
            return None;
        }
        Some(Money { amount: self.amount.add(&other.amount), currency: self.currency })
    }

    /// Difference of two amounts in the **same** currency; `None` across currencies.
    pub fn sub(&self, other: &Money) -> Option<Money> {
        if self.currency != other.currency {
            return None;
        }
        Some(Money { amount: self.amount.sub(&other.amount), currency: self.currency })
    }

    /// Scale an amount by a plain integer (`19.99 USD × 3 = 59.97 USD`), re-quantised to the currency.
    pub fn scale_int(&self, k: i64) -> Money {
        Money::of(self.amount.mul(&Decimal::from_i64(k)), self.currency)
    }

    /// The exact dimensionless ratio of two same-currency amounts (`30 USD ÷ 10 USD = 3`); `None`
    /// across currencies or when the divisor is zero.
    pub fn ratio(&self, other: &Money) -> Option<Rational> {
        if self.currency != other.currency {
            return None;
        }
        self.amount.to_rational().div(&other.amount.to_rational())
    }
}

impl PartialEq for Money {
    /// Value equality: same currency and equal amount (`5 USD == 5.00 USD`, `5 USD ≠ 5 EUR`).
    fn eq(&self, other: &Self) -> bool {
        self.currency == other.currency && self.amount.to_rational() == other.amount.to_rational()
    }
}
impl Eq for Money {}

impl Hash for Money {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.currency.hash(state);
        self.amount.to_rational().hash(state);
    }
}

impl fmt::Display for Money {
    /// `"19.99 USD"` — the amount at the currency's minor-unit scale, then the code.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.amount, self.currency.code)
    }
}

/// A pluggable exchange-rate table — the substrate of the **Universal Money Amount** (money's UTC).
/// Each currency code maps to its rate **versus a chosen reference currency**: how many reference
/// units one unit of the currency is worth, exact on the Rational tower (the reference itself is 1).
/// The *source* is interchangeable — a literal table, a CRDT-synced table, an API fetch — they all
/// produce this same value, so with rates in scope money of different currencies shares one
/// coordinate and cross-currency math becomes meaningful (and exact).
#[derive(Clone, Debug, Default)]
pub struct RateTable {
    rates: std::collections::HashMap<String, Rational>,
}

impl RateTable {
    pub fn new() -> Self {
        RateTable { rates: std::collections::HashMap::new() }
    }

    /// Record that `1 <code> = rate` reference units (case-insensitive code).
    pub fn set(&mut self, code: &str, rate: Rational) {
        self.rates.insert(code.trim().to_ascii_uppercase(), rate);
    }

    /// The rate of `code` versus the reference, if known.
    pub fn rate(&self, code: &str) -> Option<&Rational> {
        self.rates.get(&code.trim().to_ascii_uppercase())
    }

    /// Convert `m` into `to`, exact via the Rational tower (normalise to the reference, then to the
    /// target), quantised to `to`'s minor unit. `None` if either currency's rate is missing.
    pub fn convert(&self, m: &Money, to: Currency) -> Option<Money> {
        let from_rate = self.rate(m.currency.code)?;
        let to_rate = self.rate(to.code)?;
        let value_in_reference = m.amount.to_rational().mul(from_rate);
        let amount_in_to = value_in_reference.div(to_rate)?;
        Some(Money::of(Decimal::from_rational(&amount_in_to, to.scale, RoundingMode::HalfEven), to))
    }
}

thread_local! {
    /// The in-scope exchange-rate context for `<money> in <currency>` conversion. Pluggable: a
    /// program installs it from a literal table, a CRDT-synced table, or an API fetch — they all land
    /// here. `None` until rates are provided (conversion then errors, rather than guessing).
    static AMBIENT_RATES: std::cell::RefCell<Option<RateTable>> =
        const { std::cell::RefCell::new(None) };
}

/// Install (replace) the ambient rate table.
pub fn set_ambient_rates(table: RateTable) {
    AMBIENT_RATES.with(|r| *r.borrow_mut() = Some(table));
}

/// Add or replace one rate in the ambient table (`1 <code> = rate` reference units), creating the
/// table if none is in scope yet.
pub fn set_ambient_rate(code: &str, rate: Rational) {
    AMBIENT_RATES.with(|r| r.borrow_mut().get_or_insert_with(RateTable::new).set(code, rate));
}

/// Drop the ambient rate table (back to "no rates in scope").
pub fn clear_ambient_rates() {
    AMBIENT_RATES.with(|r| *r.borrow_mut() = None);
}

/// True if any rate context is in scope.
pub fn has_ambient_rates() -> bool {
    AMBIENT_RATES.with(|r| r.borrow().is_some())
}

/// Convert `m` to `to` using the ambient rate table. `None` if no rates are in scope or a currency's
/// rate is missing — the caller surfaces a clean error rather than a wrong number.
pub fn ambient_convert(m: &Money, to: Currency) -> Option<Money> {
    AMBIENT_RATES.with(|r| r.borrow().as_ref().and_then(|t| t.convert(m, to)))
}

#[cfg(test)]
mod tests {
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
        assert_eq!(money("0.10", "USD").add(&money("0.20", "USD")).unwrap().to_string(), "0.30 USD");
        assert_eq!(money("19.99", "USD").add(&money("5.00", "USD")).unwrap().to_string(), "24.99 USD");
        assert_eq!(money("24.99", "USD").sub(&money("5.00", "USD")).unwrap().to_string(), "19.99 USD");
        assert_eq!(money("100", "JPY").add(&money("50", "JPY")).unwrap().to_string(), "150 JPY");
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
        assert_eq!(money("30.00", "USD").ratio(&money("10.00", "USD")).unwrap(), Rational::from_i64(3));
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
        assert_eq!(rates.convert(&money("10.00", "EUR"), usd).unwrap().to_string(), "11.00 USD");
        assert_eq!(rates.convert(&money("11.00", "USD"), eur).unwrap().to_string(), "10.00 EUR");
        // Converting to the same currency is the identity.
        assert_eq!(rates.convert(&money("42.00", "USD"), usd).unwrap().to_string(), "42.00 USD");
        // Cross-rate via the reference: 10 GBP = 12.50 USD = 12.50/1.10 EUR ≈ 11.36 EUR.
        assert_eq!(rates.convert(&money("10.00", "GBP"), eur).unwrap().to_string(), "11.36 EUR");
        // An unknown currency yields None, never a wrong number.
        assert_eq!(rates.convert(&money("1.00", "USD"), currency::by_code("JPY").unwrap()), None);
    }

    #[test]
    fn value_equality() {
        assert_eq!(money("5", "USD"), money("5.00", "USD")); // value-equal across input forms
        assert_ne!(money("5.00", "USD"), money("5.00", "EUR")); // currency matters
        assert_ne!(money("5.00", "USD"), money("6.00", "USD"));
    }
}
