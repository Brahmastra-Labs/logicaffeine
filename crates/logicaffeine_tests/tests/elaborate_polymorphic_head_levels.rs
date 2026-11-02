//! A UNIVERSE-POLYMORPHIC HEAD's levels are solved from its arguments by the elaborator, as the
//! kernel's application rule solves them.
//!
//! `Eq` and `refl` are declared at `Sort u`. The kernel instantiates `u` afresh at every
//! application and solves it from the argument (`type_checker::implicit_levels_for_head`); the
//! elaborator's spine kept `u` a rigid variable, so `Nat : Sort u` did not unify (`Type 0 ≤ u` is
//! false: `u` may be `Prop`), typed elaboration fell back to the plain application, and a numeral
//! argument was never given the carrier the type argument had fixed. `refl Nat 2` was refused with
//! "Type mismatch: expected u, found Type0" — and so was every theorem stated with a numeral at
//! `Nat`.

use logicaffeine_kernel::interface::Repl;
use logicaffeine_kernel::{BigInt, Literal, Term};

fn ok(repl: &mut Repl, command: &str) -> String {
    repl.execute(command)
        .unwrap_or_else(|e| panic!("`{command}` must be accepted: {e}"))
}

fn refused(repl: &mut Repl, command: &str) {
    if let Ok(out) = repl.execute(command) {
        panic!("`{command}` must be refused, but printed {out:?}");
    }
}

fn g(n: &str) -> Term {
    Term::Global(n.to_string())
}

fn app(f: Term, a: Term) -> Term {
    Term::App(Box::new(f), Box::new(a))
}

#[test]
fn a_numeral_argument_takes_the_carrier_its_type_argument_fixed() {
    let mut repl = Repl::new();
    assert_eq!(
        ok(&mut repl, "Check refl Nat 2."),
        "((refl Nat) 2) : (((Eq Nat) 2) 2)"
    );
    assert_eq!(
        ok(&mut repl, "Check Eq Nat 2 2."),
        "(((Eq Nat) 2) 2) : Prop"
    );
    ok(&mut repl, "Theorem two_is_two : Eq Nat 2 2 := refl Nat 2.");
    let two = Term::Lit(Literal::Nat(BigInt::from_u64(2)));
    let stated = app(app(app(g("Eq"), g("Nat")), two.clone()), two);
    assert_eq!(
        repl.context().get_global("two_is_two"),
        Some(&stated),
        "the numeral was elaborated at `Nat`, not left an `Int`"
    );
}

#[test]
fn a_theorem_about_a_definition_at_nat_numerals_is_stated_as_written() {
    let mut repl = Repl::new();
    ok(
        &mut repl,
        "Definition myAdd : Nat -> Nat -> Nat := fun m : Nat => fun n : Nat => Nat_add m n.",
    );
    ok(
        &mut repl,
        "Theorem one_one : Eq Nat (myAdd 1 1) 2 := refl Nat 2.",
    );
    assert_eq!(
        ok(&mut repl, "Check one_one."),
        "one_one : (((Eq Nat) ((myAdd 1) 1)) 2)"
    );
}

/// The level is SOLVED, from the argument — never waived.
#[test]
fn the_level_is_solved_from_the_argument_and_nothing_else_is_relaxed() {
    let mut repl = Repl::new();
    // Each carrier fixes its own level: a proposition, a type, a sort.
    assert_eq!(
        ok(&mut repl, "Check refl Prop True."),
        "((refl Prop) True) : (((Eq Prop) True) True)"
    );
    ok(&mut repl, "Check refl Type Nat.");
    assert_eq!(
        ok(&mut repl, "Check refl Int 2."),
        "((refl Int) 2) : (((Eq Int) 2) 2)"
    );
    // What was wrong stays wrong.
    refused(
        &mut repl,
        "Definition two_is_three : Eq Nat 2 3 := refl Nat 2.",
    );
    refused(&mut repl, "Check refl 2 2.");
    refused(&mut repl, "Check refl Nat (-1).");
    refused(&mut repl, "Check Eq Nat 2 true.");
}

/// The expected type is met by CONVERSION when unification cannot decide it: the body's type
/// `Eq Nat 2 2` and the declared `Eq Nat (double 1) 2` are the same proposition only once `double 1`
/// computes, which the kernel's conversion sees and first-order unification does not.
#[test]
fn the_expected_type_is_met_by_conversion_where_unification_cannot_decide() {
    let mut repl = Repl::new();
    ok(
        &mut repl,
        "Definition double : Nat -> Nat := fun n : Nat => Nat_add n n.",
    );
    ok(
        &mut repl,
        "Definition two_by_doubling : Eq Nat (double 1) 2 := refl Nat 2.",
    );
    ok(
        &mut repl,
        "Theorem two_by_doubling_thm : Eq Nat (double 1) 2 := refl Nat 2.",
    );
    refused(
        &mut repl,
        "Theorem three_by_doubling : Eq Nat (double 1) 3 := refl Nat 3.",
    );
}

/// The fallback is asked exactly where the elaborator's own positive check declines — a numeral
/// past `i64`, a constructor chain deeper than its budget — and it DECIDES there: each statement is
/// the same proposition as its body's type only under the kernel's conversion and is accepted, and
/// the same shapes with the wrong numeral are refused.
#[test]
fn the_conversion_fallback_decides_what_the_positive_check_declines() {
    let mut repl = Repl::new();
    ok(
        &mut repl,
        "Definition double : Nat -> Nat := fun n : Nat => Nat_add n n.",
    );
    ok(&mut repl, "Theorem past_i64 : Eq Nat (double 9223372036854775808) 18446744073709551616 := refl Nat 18446744073709551616.");
    refused(&mut repl, "Theorem past_i64_wrong : Eq Nat (double 9223372036854775808) 18446744073709551617 := refl Nat 18446744073709551617.");
    let seventy = (0..70).fold("Zero".to_string(), |acc, _| format!("(Succ {acc})"));
    ok(
        &mut repl,
        &format!("Theorem deep_chain : Eq Nat (double 35) {seventy} := refl Nat 70."),
    );
    refused(
        &mut repl,
        &format!("Theorem deep_chain_wrong : Eq Nat (double 35) {seventy} := refl Nat 71."),
    );
}
