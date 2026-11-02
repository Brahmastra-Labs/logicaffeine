//! ★ LOCK — a form that changes something cannot forget its CSRF token. ★
//!
//! CSRF protection you have to REMEMBER is not protection: it is a checklist item that gets
//! skipped on the one form somebody added in a hurry, and nothing anywhere says so until a
//! visitor's browser is made to submit it from another site.
//!
//! So forgetting is a COMPILE ERROR. A `<form>` whose method changes state — POST, PUT, PATCH,
//! DELETE — must carry a `_csrf` field, and a page that omits one does not build. That is the
//! difference between a framework that offers a defence and one that applies it.
//!
//! The token itself is `session.lg`'s, already locked there against forgery, the wrong session
//! and the wrong secret. What this file adds is that you cannot ship a form without it.

use logicaffeine_compile::compile::{tw_outcome, vm_outcome};
use logicaffeine_compile::semantics::dom;

fn refusal(src: &str) -> String {
    tw_outcome(src)
        .error
        .unwrap_or_else(|| panic!("expected a refusal:\n{src}"))
}

/// Each tier runs the page in a document of its own, as each would load it.
fn runs(src: &str) {
    dom::reset();
    let tw = tw_outcome(src);
    assert_eq!(tw.error, None, "the tree-walker must run it:\n{src}");
    dom::reset();
    let vm = vm_outcome(src);
    assert_eq!(vm.error, None, "the VM must run it too:\n{src}");
}

fn page(form: &str) -> String {
    format!("## Component App\nView:\n    {form}\n\n## Main\nMount App.\n")
}

#[test]
fn a_post_form_without_a_csrf_field_is_refused() {
    let err = refusal(&page(
        "<form method=\"post\" action=\"/save\"><input name=\"title\"><button>Go</button></form>",
    ));
    assert!(
        err.contains("_csrf"),
        "the refusal names the field that is missing: {err}"
    );
    assert!(
        err.to_lowercase().contains("csrf") || err.to_lowercase().contains("forg"),
        "and says what the problem is: {err}"
    );
}

#[test]
fn every_state_changing_method_is_covered() {
    // POST is the one everybody thinks of. A form that PUTs or DELETEs is exactly as forgeable.
    for method in ["post", "put", "patch", "delete", "POST", "Delete"] {
        let src = page(&format!(
            "<form method=\"{method}\" action=\"/x\"><button>Go</button></form>"
        ));
        let err = refusal(&src);
        assert!(
            err.contains("_csrf"),
            "`method=\"{method}\"` must be covered: {err}"
        );
    }
}

#[test]
fn a_form_carrying_the_field_builds() {
    runs(&page(
        "<form method=\"post\" action=\"/save\">\
         <input type=\"hidden\" name=\"_csrf\" value=\"t\">\
         <input name=\"title\"><button>Go</button></form>",
    ));
}

#[test]
fn a_get_form_is_left_alone() {
    // A GET form changes nothing, so demanding a token there would be noise — and noise is how a
    // check gets switched off.
    runs(&page(
        "<form method=\"get\" action=\"/search\"><input name=\"q\"><button>Go</button></form>",
    ));
    runs(&page(
        "<form action=\"/search\"><input name=\"q\"><button>Go</button></form>",
    ));
}

#[test]
fn a_page_with_no_form_at_all_is_unaffected() {
    runs(&page("<p>nothing to submit</p>"));
}

#[test]
fn the_field_may_be_filled_from_a_hole() {
    // The realistic shape: the token is computed, not typed. The check is about the FIELD being
    // present, never about what fills it.
    runs(&format!(
        "## Component App\nState:\n    Let token be \"abc\".\nView:\n    \
         <form method=\"post\" action=\"/save\">\
         <input type=\"hidden\" name=\"_csrf\" value={{token}}>\
         <button>Go</button></form>\n\n## Main\nMount App.\n"
    ));
}

#[test]
fn a_nested_form_is_found_too() {
    // A form is usually inside a card, a section, a list item. A check that only looked at the
    // top level would pass every real page.
    let err = refusal(&page(
        "<div class=\"card\"><section><form method=\"post\" action=\"/x\">\
         <button>Go</button></form></section></div>",
    ));
    assert!(err.contains("_csrf"), "a nested form must be found: {err}");
}

#[test]
fn a_server_rendered_route_form_is_covered_as_well() {
    // The tier that matters most for CSRF: a page a browser was handed by the server, which is
    // where a forged submission is aimed.
    let src = "# Server\n## Route GET /new -> View:\n    \
        <form method=\"post\" action=\"/save\"><button>Go</button></form>\n\n\
        ## Main\nShow route(\"GET\", \"/new\").\n";
    let err = refusal(src);
    assert!(
        err.contains("_csrf"),
        "a route's form must be covered: {err}"
    );
}

// ---- The token itself, end to end. ----

#[test]
fn a_token_this_session_carries_is_accepted_and_another_sessions_is_not() {
    // `session.lg` locks the token's forgery properties; this is the round trip a page performs:
    // mint one for the session, and check the one that came back.
    let src = "## Main\n\
        Let mine be csrfToken(\"secret\", \"ada\").\n\
        Show csrfValid(\"secret\", \"ada\", mine).\n\
        Show csrfValid(\"secret\", \"bob\", mine).\n\
        Show csrfValid(\"other-secret\", \"ada\", mine).\n\
        Show csrfValid(\"secret\", \"ada\", \"forged\").\n";
    let tw = tw_outcome(src);
    assert_eq!(tw.error, None);
    assert_eq!(
        tw.output.lines().collect::<Vec<_>>(),
        ["true", "false", "false", "false"],
        "a token is good for ONE session under ONE secret, and a forgery is refused"
    );
}

// ---------------------------------------------------------------------------------------------
// ★ THE TWO WAYS THE CHECK FAILS OPEN. ★
//
// The lint above decides a form is state-changing by reading its `method`, and it reads that
// attribute with `static_attr`, which matches ONLY `TAttr::Static`. Two ordinary pieces of HTML
// therefore walk straight past it, and both ship a forgeable form with no diagnostic:
//
//   1. a computed method — `<form method={m}>` is `TAttr::Dynamic`, so `static_attr` answers
//      `None`, `.unwrap_or("get")` calls it a GET, and a GET needs no token;
//   2. `formmethod` on the submit button — HTML lets a button override its form's method, and
//      nothing in `lgx/check.rs` models that attribute at all.
//
// ★ A GUARD MUST FAIL CLOSED. The sibling reader `has_attr` is already dynamic-aware and is what
// `alt` / `aria-label` / `href` use; the CSRF path is the one place that reaches for the stricter
// one, and the strictness lands on the wrong side — an attribute it cannot read becomes "not a
// state-changing form" rather than "I cannot tell, so demand the token".
//
// This is the same shape as the audit's other findings: the gap is in what was never enumerated,
// and a denylist-by-omission fails open. Compare `event_modifier_lock`, where a modifier that was
// simply not on a list lowered silently wrong.

/// A method the checker cannot read is a method it must not wave through.
#[test]
fn a_computed_method_still_demands_the_token() {
    let src = "## Component App\nState:\n    Let m be \"post\".\nView:\n    \
               <form method={m} action=\"/save\"><input name=\"title\"><button>Go</button></form>\n\n\
               ## Main\nMount App.\n";
    let err = tw_outcome(src)
        .error
        .unwrap_or_else(|| panic!("a form whose method is computed must still be refused:\n{src}"));
    assert!(
        err.contains("_csrf"),
        "the refusal names the field that is missing: {err}"
    );
}

/// The same form, written the safe way, must still build — or the fix is a refusal of everything
/// rather than a guard, and an author cannot get past it.
#[test]
fn a_computed_method_with_the_token_is_accepted() {
    runs(&format!(
        "## Component App\nState:\n    Let m be \"post\".\n    Let token be \"t\".\nView:\n    \
         <form method={{m}} action=\"/save\">\
         <input type=\"hidden\" name=\"_csrf\" value={{token}}>\
         <input name=\"title\"><button>Go</button></form>\n\n## Main\nMount App.\n"
    ));
}

/// A `get` form that stays a literal `get` is still left alone. The fix must key on "cannot tell",
/// not on "is not literally get" — otherwise every ordinary search box starts demanding a token.
#[test]
fn a_literal_get_form_is_still_left_alone_after_the_fix() {
    runs(&page(
        "<form method=\"get\" action=\"/search\"><input name=\"q\"><button>Find</button></form>",
    ));
}

/// HTML lets a submit button override its form's method. A form the checker reads as `get` can
/// therefore POST, and today that is invisible to it.
#[test]
fn a_button_that_overrides_the_method_demands_the_token_too() {
    let err = tw_outcome(&page(
        "<form action=\"/delete\"><button type=\"submit\" formmethod=\"post\">Delete</button></form>",
    ))
    .error
    .unwrap_or_else(|| {
        panic!("`formmethod=\"post\"` makes this a state-changing submission and it must be refused")
    });
    assert!(
        err.contains("_csrf"),
        "the refusal names the field that is missing: {err}"
    );
}

/// `formmethod` is only interesting when it CHANGES something — a button that spells out the
/// default must not start demanding a token that no other `get` form needs.
#[test]
fn a_button_whose_formmethod_is_get_is_left_alone() {
    runs(&page(
        "<form action=\"/search\"><button type=\"submit\" formmethod=\"get\">Find</button></form>",
    ));
}
