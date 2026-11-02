//! A program's own variant wins over a standard-library variant of the same name, however the
//! program spells it.
//!
//! The prelude splices a standard-library module in when a program names one of its declarations,
//! and a name the program declares itself wins: the library's copy is renamed out of the way. A
//! variant's article is optional, and the scan that finds the program's declarations counted a
//! variant only after one — so an `Outcome` written `Failure (message: Text).` did not declare
//! `Failure`, `io.lg`'s `Severity` came along unrenamed, and `a new Failure` built a `Severity`
//! (the Studio's "Outcomes, Not Crashes" example). Each program must print the same on the
//! tree-walker, the VM and AOT-Rust.

mod common;
use common::{assert_all_tiers, assert_compiled_equals_interpreted};

fn on_every_tier(src: &str, expected: &str) {
    assert_all_tiers(src, expected);
    assert_compiled_equals_interpreted(src);
}

fn outcome_program(declaration: &str) -> String {
    format!(
        "## Definition\nAn Outcome is either:\n{declaration}\n\n## To checked_divide (a: Int) and (b: Int) -> Outcome:\n    If b equals 0:\n        Return a new Failure with message \"cannot divide {{a}} by zero\".\n    Return a new Success with value (a / b).\n\n## Main\nLet good be checked_divide(10, 2).\nInspect good:\n    When Success (v): Show \"10 / 2 = {{v}}\".\n    When Failure (m): Show \"error: {{m}}\".\nLet bad be checked_divide(5, 0).\nInspect bad:\n    When Success (v): Show \"5 / 0 = {{v}}\".\n    When Failure (m): Show \"error: {{m}}\".\n"
    )
}

const EXPECTED: &str = "10 / 2 = 5\nerror: cannot divide 5 by zero";

#[test]
fn concise_variants_without_articles() {
    on_every_tier(
        &outcome_program("    Success (value: Int).\n    Failure (message: Text)."),
        EXPECTED,
    );
}

#[test]
fn natural_variants_without_articles() {
    on_every_tier(
        &outcome_program("    Success with value Int.\n    Failure with message Text."),
        EXPECTED,
    );
}

#[test]
fn variants_with_articles() {
    on_every_tier(
        &outcome_program("    A Success (value: Int).\n    A Failure (message: Text)."),
        EXPECTED,
    );
}

/// A unit variant with no article, next to the library's own unit `Failure`.
#[test]
fn a_bare_unit_variant() {
    on_every_tier(
        "## Definition\nA Verdict is either:\n    Pass.\n    Failure.\n\n## Main\nLet v be a new Failure.\nInspect v:\n    When Pass: Show \"pass\".\n    When Failure: Show \"fail\".\n",
        "fail",
    );
}
