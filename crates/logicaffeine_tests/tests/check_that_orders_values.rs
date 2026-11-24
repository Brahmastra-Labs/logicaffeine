//! `Check that <condition>.` is the mandatory contract: the quick guide's own example is
//! `Check that balance is at least amount.` It did not parse in `## Main`. The parser read every
//! `Check` as a security guard, `Check that <subject> is <predicate>.` with one word on each side,
//! so `capped is at most watts` failed with "I expected a statement here" while the same
//! condition after `Assert that` parsed.
//!
//! A one-word predicate after `is` (`Check that the user is admin.`) and a capability
//! (`Check that the user can edit the document.`) are still security guards. Every other
//! condition is a value check that stops the program when it is false, like `Require that`.

use logicaffeine_compile::interpret_for_ui_sync;

fn run(main_body: &str) -> (Vec<String>, Option<String>) {
    let r = interpret_for_ui_sync(&format!("## Main\n{main_body}"));
    (r.lines, r.error)
}

#[test]
fn check_that_a_value_is_at_most_another_parses_and_passes_when_true() {
    let (lines, error) = run("Let capped be 300.\nLet watts be 450.\nCheck that capped is at most watts.\nShow \"checked\".\n");
    assert_eq!(error, None, "a true check must not stop the program");
    assert_eq!(lines, vec!["checked".to_string()]);
}

#[test]
fn check_that_a_value_is_at_most_another_stops_the_program_when_false() {
    let (lines, error) = run("Let capped be 900.\nLet watts be 450.\nCheck that capped is at most watts.\nShow \"after\".\n");
    assert!(error.is_some(), "a false check must stop the program, it printed {lines:?}");
    assert!(lines.is_empty(), "nothing after a failed check may run: {lines:?}");
}

#[test]
fn check_accepts_every_spelling_assert_accepts() {
    for (condition, holds) in [
        ("capped is at most watts", true),
        ("capped is at least watts", false),
        ("capped is less than watts", true),
        ("capped is greater than watts", false),
        ("capped <= watts", true),
        ("capped > watts", false),
        ("capped is less than or equal to watts", true),
        ("capped is not watts", true),
        ("capped is at most watts and watts is at least 0", true),
        ("capped + 200 is at most watts", false),
    ] {
        let (lines, error) =
            run(&format!("Let capped be 300.\nLet watts be 450.\nCheck that {condition}.\nShow \"after\".\n"));
        if holds {
            assert_eq!(error, None, "`Check that {condition}.` holds for 300 and 450");
            assert_eq!(lines, vec!["after".to_string()], "`Check that {condition}.`");
        } else {
            assert!(error.is_some(), "`Check that {condition}.` is false for 300 and 450, yet it passed");
        }
    }
}

#[test]
fn check_and_assert_agree_on_every_condition() {
    for condition in ["capped is at most watts", "capped is greater than watts", "capped equals 300"] {
        let body = |verb: &str| format!("Let capped be 300.\nLet watts be 450.\n{verb} that {condition}.\nShow \"after\".\n");
        let (check_lines, check_error) = run(&body("Check"));
        let (assert_lines, assert_error) = run(&body("Assert"));
        assert_eq!(check_lines, assert_lines, "`Check` and `Assert` disagree on `{condition}`");
        assert_eq!(check_error.is_some(), assert_error.is_some(), "`Check` and `Assert` disagree on `{condition}`");
    }
}

#[test]
fn check_that_a_value_holds_inside_a_function() {
    let r = interpret_for_ui_sync(
        "## To cap (watts: Int) -> Int:\n    Let capped be watts.\n    If capped is greater than 450:\n        Set capped to 450.\n    Check that capped is at most 450.\n    Return capped.\n\n## Main\nShow cap(900).\n",
    );
    assert_eq!(r.error, None);
    assert_eq!(r.lines, vec!["450".to_string()]);
}

#[test]
fn a_one_word_check_is_still_a_security_guard() {
    let source = "## Definition\nA User has:\n    a role: Text.\n\n## Policy\nA User is admin if the user's role equals \"admin\".\n\n## Main\nLet u be a new User with role \"admin\".\nCheck that u is admin.\nShow \"allowed\".\nLet v be a new User with role \"guest\".\nCheck that v is admin.\nShow \"leaked\".\n";
    let r = interpret_for_ui_sync(source);
    assert_eq!(r.lines, vec!["allowed".to_string()], "the guard must let the admin through and stop the guest");
    assert!(r.error.is_some(), "a failed security guard must stop the program");
}
