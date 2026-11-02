//! ★ LOCK — `forms.lg`: what a visitor typed, and what was wrong with it. ★
//!
//! A form is where a web application meets a stranger, so every value arrives untrusted and most
//! arrive wrong. This file drives every function in the module through the reference tier.
//!
//! ★ THE CENTRAL CHOICE IS THAT ERRORS ARE A SEQUENCE AND NOT A MAP, and it is a safety choice
//! rather than a style one. A LOGOS map TRAPS on a missing key: `item "email" of errors` on a form
//! that validated cleanly does not answer `""`, it ends the program. A template asks about a field
//! precisely when it does not know whether that field failed — so a map would make the ordinary
//! case, "render the error under this input if there is one", the crashing one. Every test below
//! that reads an absent field is therefore load-bearing, not a courtesy.

use logicaffeine_compile::compile::tw_outcome;

/// Run `body` inside a `## Main` and report what it showed.
fn shows(subject: &str, body: &str) -> String {
    let src = format!("## Main\n{body}\n");
    let out = tw_outcome(&src);
    assert_eq!(out.error, None, "`{subject}` must run:\n{src}");
    out.output.trim().to_string()
}

#[test]
fn a_clean_form_has_no_errors_and_asking_about_a_field_is_safe() {
    // ★ THE CASE A MAP WOULD HAVE TRAPPED ON. Nothing failed, and the template still asks.
    assert_eq!(
        shows(
            "empty error list",
            "Let e be noErrors().\nShow isValid(e).\nShow length of e.\nShow \"[{errorFor(e, \\\"email\\\")}]\".\nShow hasError(e, \"email\")."
        ),
        "true\n0\n[]\nfalse"
    );
}

#[test]
fn a_required_field_complains_when_blank_and_when_only_whitespace() {
    assert_eq!(
        shows(
            "required",
            "Let mutable e be noErrors().\nSet e to checkRequired(e, \"name\", \"\").\nShow errorFor(e, \"name\").\nShow isValid(e)."
        ),
        "This is required.\nfalse"
    );
    // ★ WHITESPACE IS THE WHOLE REASON `trim` EXISTS. A browser sends what was typed, and a
    // visitor types spaces; without trimming, "required" means "not empty" and a form of three
    // spaces is accepted as a name.
    assert_eq!(
        shows(
            "whitespace only",
            "Let mutable e be noErrors().\nSet e to checkRequired(e, \"name\", \"   \").\nShow hasError(e, \"name\")."
        ),
        "true"
    );
    assert_eq!(
        shows(
            "present",
            "Let mutable e be noErrors().\nSet e to checkRequired(e, \"name\", \"Ada\").\nShow isValid(e)."
        ),
        "true"
    );
}

#[test]
fn trimming_removes_every_kind_of_surrounding_space_and_keeps_the_inside() {
    // `trim` is `strings.lg`'s, not a second one written here — see `isBlank`'s note.
    assert_eq!(shows("plain", "Show \"[{trim(\\\"  hi  \\\")}]\"."), "[hi]");
    assert_eq!(
        shows("inner space kept", "Show \"[{trim(\\\"  a b  \\\")}]\"."),
        "[a b]"
    );
    assert_eq!(shows("all space", "Show \"[{trim(\\\"   \\\")}]\"."), "[]");
    assert_eq!(shows("empty", "Show \"[{trim(\\\"\\\")}]\"."), "[]");
    assert_eq!(
        shows("nothing to trim", "Show \"[{trim(\\\"ab\\\")}]\"."),
        "[ab]"
    );
    assert_eq!(
        shows(
            "blank predicate",
            "Show isBlank(\"  \").\nShow isBlank(\" x \")."
        ),
        "true\nfalse"
    );
}

#[test]
fn length_checks_count_after_trimming() {
    assert_eq!(
        shows(
            "too short",
            "Let mutable e be noErrors().\nSet e to checkMinLength(e, \"pw\", \"abc\", 8).\nShow errorFor(e, \"pw\")."
        ),
        "Use at least 8 characters."
    );
    assert_eq!(
        shows(
            "long enough",
            "Let mutable e be noErrors().\nSet e to checkMinLength(e, \"pw\", \"abcdefgh\", 8).\nShow isValid(e)."
        ),
        "true"
    );
    assert_eq!(
        shows(
            "too long",
            "Let mutable e be noErrors().\nSet e to checkMaxLength(e, \"bio\", \"abcdef\", 3).\nShow errorFor(e, \"bio\")."
        ),
        "Use at most 3 characters."
    );
    // Padding must not buy length.
    assert_eq!(
        shows(
            "spaces do not count",
            "Let mutable e be noErrors().\nSet e to checkMinLength(e, \"pw\", \"  ab  \", 4).\nShow hasError(e, \"pw\")."
        ),
        "true"
    );
}

#[test]
fn only_the_first_complaint_about_a_field_is_kept() {
    // ★ ONE FIELD, ONE MESSAGE. A visitor told three things about one box fixes none of them —
    // and the FIRST check is the fundamental one, so ordering the checks orders the advice.
    // Without this, an empty password box would say "must be at least 8 characters", which is
    // true and unhelpful.
    assert_eq!(
        shows(
            "required wins over length",
            "Let mutable e be noErrors().\nSet e to checkRequired(e, \"pw\", \"\").\nSet e to checkMinLength(e, \"pw\", \"\", 8).\nShow errorFor(e, \"pw\").\nShow length of e."
        ),
        "This is required.\n1"
    );
}

#[test]
fn errors_keep_the_order_they_were_found_in() {
    // The summary at the top of a form reads in the order the checks ran, not in whatever order
    // a hash chose.
    assert_eq!(
        shows(
            "summary order",
            "Let mutable e be noErrors().\nSet e to checkRequired(e, \"name\", \"\").\nSet e to checkRequired(e, \"email\", \"\").\nShow errorSummary(e)."
        ),
        "This is required.\nThis is required."
    );
    assert_eq!(
        shows(
            "fields keep identity",
            "Let mutable e be noErrors().\nSet e to checkMinLength(e, \"a\", \"\", 2).\nSet e to checkMaxLength(e, \"b\", \"xxx\", 1).\nShow errorFor(e, \"a\").\nShow errorFor(e, \"b\")."
        ),
        "Use at least 2 characters.\nUse at most 1 characters."
    );
}

#[test]
fn a_range_check_includes_both_ends() {
    assert_eq!(
        shows(
            "below",
            "Let mutable e be noErrors().\nSet e to checkRange(e, \"n\", 0, 1, 5).\nShow errorFor(e, \"n\")."
        ),
        "Choose a number from 1 to 5."
    );
    assert_eq!(
        shows(
            "above",
            "Let mutable e be noErrors().\nSet e to checkRange(e, \"n\", 6, 1, 5).\nShow hasError(e, \"n\")."
        ),
        "true"
    );
    assert_eq!(
        shows(
            "both ends are in",
            "Let mutable e be noErrors().\nSet e to checkRange(e, \"n\", 1, 1, 5).\nSet e to checkRange(e, \"m\", 5, 1, 5).\nShow isValid(e)."
        ),
        "true"
    );
}

#[test]
fn confirmation_compares_two_values() {
    assert_eq!(
        shows(
            "mismatch",
            "Let mutable e be noErrors().\nSet e to checkSame(e, \"pw2\", \"a\", \"b\", \"Passwords do not match.\").\nShow errorFor(e, \"pw2\")."
        ),
        "Passwords do not match."
    );
    assert_eq!(
        shows(
            "match",
            "Let mutable e be noErrors().\nSet e to checkSame(e, \"pw2\", \"a\", \"a\", \"Passwords do not match.\").\nShow isValid(e)."
        ),
        "true"
    );
}

#[test]
fn the_email_check_asks_the_three_questions_worth_asking() {
    // ★ IT CHECKS THE SHAPE, NOT THE ADDRESS, and the tests say so. A validator matching the real
    // grammar rejects valid addresses more often than it catches invalid ones, and the only proof
    // an address works is a message arriving at it.
    assert_eq!(
        shows("ordinary", "Show looksLikeEmail(\"a@b.co\")."),
        "true"
    );
    assert_eq!(shows("no at", "Show looksLikeEmail(\"ab.co\")."), "false");
    assert_eq!(
        shows("two ats", "Show looksLikeEmail(\"a@b@c.co\")."),
        "false"
    );
    assert_eq!(
        shows("nothing before", "Show looksLikeEmail(\"@b.co\")."),
        "false"
    );
    assert_eq!(
        shows("nothing after", "Show looksLikeEmail(\"a@\")."),
        "false"
    );
    assert_eq!(
        shows("no dot after", "Show looksLikeEmail(\"a@bco\")."),
        "false"
    );
    assert_eq!(
        shows("dot last", "Show looksLikeEmail(\"a@bco.\")."),
        "false"
    );
    assert_eq!(
        shows(
            "surrounding space is fine",
            "Show looksLikeEmail(\"  a@b.co \")."
        ),
        "true"
    );
}

#[test]
fn an_optional_email_may_be_blank_but_a_wrong_one_may_not() {
    // Emptiness is `checkRequired`'s question, not this one — so an optional field left alone
    // does not collect a shape complaint it cannot act on.
    assert_eq!(
        shows(
            "blank is left alone",
            "Let mutable e be noErrors().\nSet e to checkEmail(e, \"email\", \"\").\nShow isValid(e)."
        ),
        "true"
    );
    assert_eq!(
        shows(
            "malformed complains",
            "Let mutable e be noErrors().\nSet e to checkEmail(e, \"email\", \"nope\").\nShow errorFor(e, \"email\")."
        ),
        "This does not look like an email address."
    );
}

#[test]
fn a_whole_signup_form_validates_as_one_pipeline() {
    // The shape a real route writes: thread the errors through every check, then ask once.
    let body = "Let mutable e be noErrors().\n\
        Set e to checkRequired(e, \"name\", \"  \").\n\
        Set e to checkEmail(e, \"email\", \"bad\").\n\
        Set e to checkMinLength(e, \"pw\", \"short\", 8).\n\
        Set e to checkSame(e, \"pw2\", \"short\", \"other\", \"Passwords do not match.\").\n\
        Show length of e.\n\
        Show isValid(e).\n\
        Show errorFor(e, \"name\").\n\
        Show \"[{errorFor(e, \\\"nickname\\\")}]\".";
    assert_eq!(shows("signup", body), "4\nfalse\nThis is required.\n[]");
}

#[test]
fn the_checks_can_actually_fail() {
    // ★ ANTI-VACUITY. If `addError` silently dropped everything, or `isValid` always answered
    // true, every assertion above about a VALID form would still pass. Prove the invalid form is
    // distinguishable from the valid one on the same code path.
    let valid = shows(
        "valid",
        "Let mutable e be noErrors().\nSet e to checkRequired(e, \"a\", \"x\").\nShow isValid(e).\nShow length of e.",
    );
    let invalid = shows(
        "invalid",
        "Let mutable e be noErrors().\nSet e to checkRequired(e, \"a\", \"\").\nShow isValid(e).\nShow length of e.",
    );
    assert_eq!(valid, "true\n0");
    assert_eq!(invalid, "false\n1");
    assert_ne!(
        valid, invalid,
        "the module cannot tell a good form from a bad one"
    );
}
