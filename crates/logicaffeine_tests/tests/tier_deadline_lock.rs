//! The deadline on a cross-tier run: a program that never finishes fails its test at the deadline
//! instead of hanging the suite, and a program that finishes passes through untouched.

mod common;
use std::time::Duration;

fn panic_text(payload: Box<dyn std::any::Any + Send>) -> String {
    payload
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| payload.downcast_ref::<&str>().map(|s| s.to_string()))
        .unwrap_or_default()
}

#[test]
fn a_program_that_never_finishes_fails_at_the_deadline() {
    let spin = "## Main\nLet x be 0.\nWhile true:\n    Set x to x + 1.";
    let outcome = std::panic::catch_unwind(|| common::run_all_tiers_within(spin, Duration::from_secs(2)));
    let message = panic_text(outcome.err().expect("a spinning program cannot pass"));
    assert!(message.contains("did not finish on every tier within 2s"), "{message}");
    assert!(message.contains("While true"), "the failure shows the program: {message}");
}

#[test]
fn a_program_that_finishes_passes_through() {
    common::assert_all_tiers_within("## Main\nLet x be 2.\nShow x * 21.", "42", Duration::from_secs(120));
    let agreed = common::run_all_tiers_within("## Main\nShow 1 / 0.", Duration::from_secs(120));
    assert!(agreed.error.is_some(), "an error inside the deadline is the program's own outcome");
}

#[test]
fn a_failed_assertion_inside_the_deadline_is_reported_as_itself() {
    let outcome = std::panic::catch_unwind(|| {
        common::assert_all_tiers_within("## Main\nShow 7.", "8", Duration::from_secs(120))
    });
    let message = panic_text(outcome.err().expect("7 is not 8"));
    assert!(!message.contains("did not finish"), "{message}");
}
