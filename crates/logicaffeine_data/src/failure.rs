//! The sentences a program stops with, written once for every tier.
//!
//! This is the lowest crate every engine and every compiled program links, so a runtime failure's wording
//! lives here and nowhere else: the interpreter and the VM report these strings, the generated Rust
//! panics with them, and the hosts written in other languages are generated from them
//! (`logicaffeine_compile::semantics::failcode`).
//!
//! A broken contract (`Require that c, otherwise r.`) reports its reason, which must be Text. Every engine
//! refuses a non-Text reason with the same sentence, naming the LOGOS type it got, so the compiled tier
//! routes the reason value through [`contract_reason`]. A type with no implementation here does not
//! compile — never a report in the wrong words.

/// A value that can stand after `otherwise`.
pub trait ContractReason {
    /// The report: the text itself for a Text reason, the refusal otherwise.
    fn contract_reason(&self) -> String;
}

fn refusal(type_name: &str) -> String {
    format!("The reason after `otherwise` must be Text, but it was {type_name}.")
}

impl ContractReason for String {
    fn contract_reason(&self) -> String {
        self.clone()
    }
}

impl ContractReason for &str {
    fn contract_reason(&self) -> String {
        (*self).to_string()
    }
}

impl ContractReason for i64 {
    fn contract_reason(&self) -> String {
        refusal("Int")
    }
}

impl ContractReason for crate::types::LogosInt {
    fn contract_reason(&self) -> String {
        refusal("Int")
    }
}

impl ContractReason for f64 {
    fn contract_reason(&self) -> String {
        refusal("Real")
    }
}

impl ContractReason for bool {
    fn contract_reason(&self) -> String {
        refusal("Bool")
    }
}

impl ContractReason for char {
    fn contract_reason(&self) -> String {
        refusal("Char")
    }
}

/// The report a broken contract stops with.
pub fn contract_reason<R: ContractReason + ?Sized>(reason: &R) -> String {
    reason.contract_reason()
}

/// The reason an index outside a sequence or a text stops a program: the index as the program wrote it
/// (1-based, or negative from the end).
pub fn index_out_of_bounds(index: impl std::fmt::Display) -> String {
    format!("Index {index} out of bounds")
}

/// The reason popping a list with no items stops a program: the list as the program writes it
/// (`xs`, `b's items`).
pub fn pop_from_empty(list: &str) -> String {
    format!("Cannot pop from {list}: it is empty")
}

/// The stop for reading a field of a value that is not a record: a value of `type_name`.
pub fn field_access_on(type_name: &str) -> String {
    format!("Cannot access field on {type_name}")
}

/// The stop for setting a field of a value that is not a record.
pub const SET_FIELD_ON_NON_RECORD: &str = "Cannot set field on non-struct value";

/// The reason reading a key a map does not hold stops a program: the key as `Show` prints it.
pub fn key_not_found(key: &(impl FailureText + ?Sized)) -> String {
    format!("Key '{}' not found in map", key.failure_text())
}

/// The reason `text_from_bytes` stops a program whose bytes are not UTF-8: the decoder's own account of
/// where they stop being it.
pub fn invalid_utf8(error: impl std::fmt::Display) -> String {
    format!("text_from_bytes(): invalid UTF-8: {error}")
}

// ── Quantities, money, modular arithmetic ─────────────────────────────────────────────────────────

/// `verb` (`add`, `subtract`) applied to two quantities whose dimensions differ.
pub fn quantities_differ(
    verb: &str,
    a: impl std::fmt::Display,
    b: impl std::fmt::Display,
) -> String {
    format!("cannot {verb} quantities of different dimensions ({a} vs {b})")
}

/// Two quantities compared across dimensions.
pub fn quantities_uncomparable(a: impl std::fmt::Display, b: impl std::fmt::Display) -> String {
    format!("Cannot compare quantities of different dimensions ({a} vs {b})")
}

/// A quantity divided by a quantity whose magnitude is zero.
pub const DIVIDE_BY_ZERO_QUANTITY: &str = "cannot divide by a zero quantity";

/// A quantity divided by the number zero.
pub const DIVIDE_QUANTITY_BY_ZERO: &str = "cannot divide a quantity by zero";

/// A unit name no unit table holds.
pub fn unknown_unit(name: &str) -> String {
    format!("Unknown unit '{name}'")
}

/// `convert(q, unit)` to a unit of another dimension: the quantity's own unit and the one asked for.
pub fn cannot_convert(from_unit: &str, to_unit: &str) -> String {
    format!("cannot convert a {from_unit} quantity to '{to_unit}' — different dimension")
}

/// A currency code no currency table holds.
pub fn unknown_currency(code: &str) -> String {
    format!("Unknown currency '{code}'")
}

/// `verb` (`add`, `subtract`, `compare`) applied to money in two currencies.
pub fn currencies_differ(verb: &str, a: &str, b: &str) -> String {
    format!("cannot {verb} money of different currencies ({a} vs {b})")
}

/// Money divided by money whose amount is zero.
pub const DIVIDE_MONEY_BY_ZERO_AMOUNT: &str = "cannot divide money by a zero amount";

/// Money divided by the number zero.
pub const DIVIDE_MONEY_BY_ZERO: &str = "cannot divide money by zero";

/// A conversion between two currencies with no rate for either in scope.
pub fn no_exchange_rate(from: &str, to: &str) -> String {
    format!("no exchange rate for {from} or {to}")
}

/// A conversion with no rates in scope at all.
pub const NO_EXCHANGE_RATES: &str = "no exchange rates in scope (set a rate first)";

/// `modular(v, m)` with `m <= 0`.
pub const MODULUS_NOT_POSITIVE: &str = "modular() requires a positive modulus";

/// `op` (`add`, `subtract`, `multiply`, `divide`) applied to two residues of different moduli.
pub fn modular_rings_differ(op: &str) -> String {
    format!("cannot {op} values in different modular rings")
}

/// A modular division whose divisor shares a factor with the modulus.
pub const NO_MODULAR_INVERSE: &str = "modular divisor has no inverse (not coprime to the modulus)";

/// Text `decimal(…)` cannot read as a base-10 number.
pub fn not_a_decimal(text: &str) -> String {
    format!("Cannot parse '{text}' as Decimal")
}

/// `decimal(…)` given a value of another kind than Text, Int or Decimal, named as the engines name it.
pub fn decimal_needs_text_or_int(type_name: &str) -> String {
    format!("decimal() requires a Text or Int, got {type_name}")
}

// ── Identifiers, time, files ──────────────────────────────────────────────────────────────────────

/// Text that is not a UUID.
pub fn invalid_uuid(text: &str) -> String {
    format!("invalid UUID '{text}'")
}

/// `uuid_from_bytes` given the wrong number of bytes.
pub fn uuid_needs_16_bytes(got: usize) -> String {
    format!("uuid_from_bytes() needs 16 bytes, got {got}")
}

/// Text that is not an RFC 3339 timestamp.
pub fn not_a_timestamp(text: &str) -> String {
    format!("Cannot parse '{text}' as an RFC 3339 timestamp")
}

/// A time zone name no zone table holds.
pub fn unknown_time_zone(name: &str) -> String {
    format!("Unknown time zone '{name}'")
}

/// `readBytes(path)` on a file that cannot be read, with the reason the system gave.
pub fn cannot_read_bytes(path: &str, reason: impl std::fmt::Display) -> String {
    format!("readBytes: cannot read '{path}': {reason}")
}

/// The value a map lookup found for `key`, or the stop [`key_not_found`] names.
#[inline]
pub fn found<V, K: FailureText + ?Sized>(value: Option<V>, key: &K) -> V {
    match value {
        Some(value) => value,
        None => no_value_for(key),
    }
}

#[cold]
#[inline(never)]
fn no_value_for<K: FailureText + ?Sized>(key: &K) -> ! {
    panic!("{}", key_not_found(key))
}

/// The value an operation of the exact extension answered, or the stop its refusal names: the sentence
/// the interpreter reports for the same operation on the same values.
#[inline]
pub fn answered<V>(answer: Result<V, String>) -> V {
    match answer {
        Ok(value) => value,
        Err(why) => refused(why),
    }
}

#[cold]
#[inline(never)]
fn refused(why: String) -> ! {
    panic!("{why}")
}

/// The reason a program stops when its calls nest deeper than its stack holds: the interpreter and the VM at their
/// call-depth limit, a compiled program at the end of its stack.
pub const CALL_DEPTH_EXCEEDED: &str = "Stack overflow: maximum call depth exceeded";

/// The reason an integer division by zero stops a program.
pub const DIVISION_BY_ZERO: &str = "Division by zero";

/// The reason an integer remainder by zero stops a program.
pub const MODULO_BY_ZERO: &str = "Modulo by zero";

/// The reason a compiled program stops when an integer past a machine word reaches a place it holds one.
pub const INT_TOO_WIDE: &str = "integer value out of range for this int64 context";

/// The reason `&` stops a program: an operand it has no bits for (a negative big integer, a big integer beside
/// a value that is not a low mask `2^k - 1`, or a kind `&` does not combine).
pub const BITAND_OPERANDS: &str = "`&` requires integer, boolean, or Set operands";

/// The reason `|` stops a program: a big integer, or a kind `|` does not combine.
pub const BITOR_OPERANDS: &str = "`|` requires integer, boolean, or Set operands";

/// The reason `^` (`xor`) stops a program: a big integer, or a kind it does not combine.
pub const BITXOR_OPERANDS: &str = "Bitwise XOR requires integer, boolean, or Set operands";

/// The reason a left shift stops a program: an operand that is not a machine-word integer.
pub const SHL_OPERANDS: &str = "Left shift requires integer operands";

/// The reason a right shift stops a program: an operand that is not a machine-word integer.
pub const SHR_OPERANDS: &str = "Right shift requires integer operands";

/// A value as a failure sentence names it: exactly as `Show` prints it.
pub trait FailureText {
    fn failure_text(&self) -> String;
}

impl FailureText for i64 {
    fn failure_text(&self) -> String {
        self.to_string()
    }
}

impl FailureText for String {
    fn failure_text(&self) -> String {
        self.clone()
    }
}

impl FailureText for str {
    fn failure_text(&self) -> String {
        self.to_string()
    }
}

impl FailureText for bool {
    fn failure_text(&self) -> String {
        if *self { "true" } else { "false" }.to_string()
    }
}

impl FailureText for char {
    fn failure_text(&self) -> String {
        self.to_string()
    }
}

impl FailureText for f64 {
    fn failure_text(&self) -> String {
        crate::fmt::fmt_f64(*self)
    }
}

impl FailureText for crate::types::LogosInt {
    fn failure_text(&self) -> String {
        self.to_string()
    }
}

impl<T: FailureText + ?Sized> FailureText for &T {
    fn failure_text(&self) -> String {
        (**self).failure_text()
    }
}

impl<A: FailureText, B: FailureText> FailureText for (A, B) {
    fn failure_text(&self) -> String {
        format!("({}, {})", self.0.failure_text(), self.1.failure_text())
    }
}

impl<A: FailureText, B: FailureText, C: FailureText> FailureText for (A, B, C) {
    fn failure_text(&self) -> String {
        format!(
            "({}, {}, {})",
            self.0.failure_text(),
            self.1.failure_text(),
            self.2.failure_text()
        )
    }
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_data/failure/tests.rs"]
mod tests;
