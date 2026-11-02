//! ★ LOCK — a session cookie is SIGNED, and a forged one is refused. ★
//!
//! A session cookie lives in the visitor's browser, where the visitor can edit it. The only thing
//! standing between "logged in as me" and "logged in as anyone" is a signature the server can
//! check and the visitor cannot forge. So the tests that matter here are the NEGATIVE ones: the
//! round trip is the easy half, and a signer that accepts everything passes it.
//!
//! Every property below is a way an attacker actually gets in — edit the value, edit the MAC,
//! sign with your own secret, or exploit a parser that splits on a character the value is allowed
//! to contain. The last one is the subtle one and the reason `verifySession` splits at the LAST
//! separator rather than the first.

use logicaffeine_compile::compile::{tw_outcome, vm_outcome};

/// Both tiers, same answer — a signature that verified on one tier and not the other would be a
/// session that works until the request happens to be served by the other.
fn both(src: &str, want: &[&str]) {
    let tw = tw_outcome(src);
    assert_eq!(tw.error, None, "the tree-walker must run it:\n{src}");
    let vm = vm_outcome(src);
    assert_eq!(vm.error, None, "the VM must run it:\n{src}");
    assert_eq!(
        tw.output.lines().collect::<Vec<_>>(),
        want,
        "tree-walker:\n{src}"
    );
    assert_eq!(vm.output.lines().collect::<Vec<_>>(), want, "VM:\n{src}");
}

/// `unwrap` a verified session to text, or the word `refused`.
const SHOW: &str = "## To showSession (o: Option of Text) -> Text:\n\
    \x20   Inspect o:\n        When Some (v):\n            Return v.\n\
    \x20       When None:\n            Return \"refused\".\n\n";

#[test]
fn a_signed_session_round_trips() {
    let src = format!(
        "{SHOW}## Main\n\
         Let c be signSession(\"s3cret\", \"ada\").\n\
         Show showSession(verifySession(\"s3cret\", c)).\n"
    );
    both(&src, &["ada"]);
}

#[test]
fn a_tampered_value_is_refused() {
    // The whole point. An attacker edits the readable half of the cookie and keeps the MAC.
    let src = format!(
        "{SHOW}## Main\n\
         Let c be signSession(\"s3cret\", \"ada\").\n\
         Let mac be sessionMac(c).\n\
         Show showSession(verifySession(\"s3cret\", \"root|\" + mac)).\n"
    );
    both(&src, &["refused"]);
}

#[test]
fn a_tampered_signature_is_refused() {
    let src = format!(
        "{SHOW}## Main\n\
         Let c be signSession(\"s3cret\", \"ada\").\n\
         Show showSession(verifySession(\"s3cret\", \"ada|0000\")).\n"
    );
    both(&src, &["refused"]);
}

#[test]
fn a_cookie_signed_with_another_secret_is_refused() {
    // Someone who runs their own copy of the app must not be able to mint cookies for yours.
    let src = format!(
        "{SHOW}## Main\n\
         Let c be signSession(\"theirs\", \"root\").\n\
         Show showSession(verifySession(\"ours\", c)).\n"
    );
    both(&src, &["refused"]);
}

#[test]
fn a_malformed_cookie_is_refused_rather_than_crashing() {
    // A cookie is attacker-controlled input, so every shape of nonsense must be an answer and
    // never a trap: no separator, empty, separator only.
    let src = format!(
        "{SHOW}## Main\n\
         Show showSession(verifySession(\"s\", \"nosep\")).\n\
         Show showSession(verifySession(\"s\", \"\")).\n\
         Show showSession(verifySession(\"s\", \"|\")).\n"
    );
    both(&src, &["refused", "refused", "refused"]);
}

#[test]
fn a_value_containing_the_separator_still_round_trips() {
    // ★ The subtle one. If the value may contain `|` and the parser splits at the FIRST one, then
    // signing `a|b` yields `a|b|MAC` and an attacker can present `a` + `|` + `b|MAC` — a
    // different value carrying a signature that checks. Splitting at the LAST separator makes the
    // value unambiguous, so this is a correctness test AND a forgery test.
    let src = format!(
        "{SHOW}## Main\n\
         Let c be signSession(\"s3cret\", \"a|b\").\n\
         Show showSession(verifySession(\"s3cret\", c)).\n"
    );
    both(&src, &["a|b"]);
}

#[test]
fn an_empty_session_value_round_trips() {
    let src = format!(
        "{SHOW}## Main\n\
         Let c be signSession(\"s3cret\", \"\").\n\
         Show \"<\" + showSession(verifySession(\"s3cret\", c)) + \">\".\n"
    );
    // Bracketed because `Show \"\"` emits no line at all on either tier, so a bare assertion here
    // would be comparing nothing to nothing and would pass even if verification had REFUSED.
    both(&src, &["<>"]);
}

#[test]
fn a_csrf_token_is_bound_to_its_session_and_verified() {
    // A CSRF token that any session's token satisfies protects nothing: the attacker just uses
    // their own. Binding it to the session is what makes it unusable from another one.
    let src = "## Main\n\
        Let t be csrfToken(\"s3cret\", \"ada\").\n\
        Show csrfValid(\"s3cret\", \"ada\", t).\n\
        Show csrfValid(\"s3cret\", \"eve\", t).\n\
        Show csrfValid(\"other\", \"ada\", t).\n\
        Show csrfValid(\"s3cret\", \"ada\", \"forged\").\n";
    both(src, &["true", "false", "false", "false"]);
}
