//! THE NUMERAL BOUNDARY — one reading of an integer literal, shared by every layer.
//!
//! A `ProofTerm::Constant("42")` is an integer; a `ProofTerm::Constant("butler")` is
//! not. Four layers used to answer that question independently, each with its own
//! `s.parse::<i64>()`:
//!
//! * [`crate::decide`]'s evaluator, deciding whether a ground goal is true;
//! * [`crate::certifier::proof_term_to_kernel_term`], choosing `Term::Lit` over
//!   `Term::Global`;
//! * `certifier::term_domain`, choosing `Int` over `Entity`;
//! * `verify::SymbolCollector::mark_int`, choosing not to declare it at all.
//!
//! Every one of them was an `i64` window, and the four together put the system's
//! arithmetic ceiling at `i64::MAX` — while BOTH kernels compute integers in exact
//! arbitrary precision ([`logicaffeine_kernel::int_lit`], `Literal::BigInt`, and the
//! EXACT PATH in `reduction.rs`/`recheck.rs`). A numeral one past `i64::MAX` was
//! therefore not "too big to prove": it silently became an UNDECLARED GLOBAL of sort
//! `Entity`, so `9223372036854775808 + 1 = 9223372036854775809` was unprovable for a
//! reason no layer reported and neither kernel had.
//!
//! This module is that decision, once. Widening happens here or not at all.

use logicaffeine_base::BigInt;

/// The integer a constant NAME denotes, at arbitrary precision — `None` when the
/// name is not a numeral.
///
/// The accepted spelling is exactly what `i64::from_str` accepted, minus the `+`
/// sign it also took: an optional `-`, then one or more ASCII digits, nothing else.
/// Deliberately strict — `" 1"`, `"1_000"`, `"0x10"` and `""` are names, not
/// numbers, and a layer that treated one of them as a number would disagree with
/// the layer next to it.
pub(crate) fn int_of_constant(s: &str) -> Option<BigInt> {
    let digits = s.strip_prefix('-').unwrap_or(s);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    BigInt::parse_decimal(s)
}

/// The kernel literal for a numeral name, canonical by construction: `Int(i64)`
/// when the value fits, `BigInt` only when it cannot. That one-representation-per-
/// value invariant is what makes definitional equality of literals sound, so this
/// goes through [`logicaffeine_kernel::int_lit`] rather than choosing a variant.
pub(crate) fn literal_of_constant(s: &str) -> Option<logicaffeine_kernel::Literal> {
    int_of_constant(s).map(logicaffeine_kernel::int_lit)
}

/// Is this constant name a numeral? The predicate form, for the layers that only
/// need the yes/no (registration, domain classification).
pub(crate) fn is_numeral(s: &str) -> bool {
    int_of_constant(s).is_some()
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_proof/numeral/tests.rs"]
mod tests;
