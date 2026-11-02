//! ★ LOCK — with a `## Session`, CSRF is CHECKED, not merely required. ★
//!
//! `csrf_lock` already makes forgetting the token a COMPILE ERROR: a `<form>` whose method changes
//! state must carry a `_csrf` field or the page does not build. That is the half nobody can skip.
//!
//! ★ THE OTHER HALF WAS A COMMENT. The refusal's own message ends *"and check it with `csrfValid`
//! in the route that answers"* — so the framework required the token to be SENT and left checking
//! it to the author, in every route, forever. A defence you have to remember in each handler is the
//! same checklist item `csrf_lock` exists to abolish, one layer down: the form is guaranteed, and
//! the route that ignores what it carries is not.
//!
//! Now the router checks it, before the route runs, for every method that changes state.
//!
//! ★ AND THE TOKEN IS BOUND TO A NONCE, NOT TO THE SESSION'S CONTENTS. Deriving it from the
//! session's fields would give every ANONYMOUS visitor the same token — they all carry the same
//! empty session — and a token every visitor shares is one an attacker simply reads off their own
//! page. So a session carries a hidden `__csrfnonce`, minted the first time `csrf()` is called on
//! that visit, which is exactly the moment a page needs one and no earlier. `examples/guestbook`
//! records the old behaviour under its own "Honest limits": `## To session () -> Text: Return
//! "everyone".` — one token for everybody, which protects nobody.
//!
//! ★ GATED ON `## Session`. A program that declares no session is unchanged, so this cannot break
//! a page that was checking by hand.

use logicaffeine_compile::compile::{compile_to_rust, tw_outcome};

const SESSION: &str = "## Session\n\x20   a user, which is Text.\n\n";

/// A LOGOS helper building a `Request` with a cookie jar and a body, so a test can drive the
/// browser's half of the exchange: fetch a page, keep the cookie, submit the form.
const MK: &str = "## To mk (m: Text) and (p: Text) and (c: Text) and (b: Text) -> Request:\n\
                 \x20   Let h be a new Map of Text to Text.\n\
                 \x20   Set item \"cookie\" of h to c.\n\
                 \x20   Return a new Request with method m and path p and body b and headers h \
                  and query (a new Map of Text to Text).\n\n";

const ROUTES: &str = "## Route GET /form -> Text:\n\x20   Return csrf().\n\n\
                      ## Route POST /save -> Text:\n\x20   Return \"saved\".\n\n";

fn run(subject: &str, src: &str) -> String {
    let out = tw_outcome(src);
    assert_eq!(out.error, None, "`{subject}` must run:\n{src}");
    out.output.trim().to_string()
}

/// Fetch `/form` (which mints a token), then POST `/save` with `body`, showing the status.
///
/// `body` is LOGOS source for the request body, so a test can send the right token, a wrong one,
/// or nothing at all.
fn exchange(subject: &str, body: &str) -> String {
    let src = format!(
        "{SESSION}{MK}{ROUTES}\
         ## Main\n\
         Let a be __lgx_route_handler(mk(\"GET\", \"/form\", \"\", \"\")).\n\
         Let token be a's body.\n\
         Let jar be cutBefore(headerIn(a's headers, \"set-cookie\"), \";\").\n\
         Let b be __lgx_route_handler(mk(\"POST\", \"/save\", jar, {body})).\n\
         Show b's status.\n"
    );
    run(subject, &src)
}

#[test]
fn a_state_changing_request_with_the_right_token_is_served() {
    // The floor, and it has to come first: a check that refuses everything is not protection.
    assert_eq!(exchange("a valid token", "\"_csrf=\" + token"), "200");
}

#[test]
fn a_state_changing_request_with_no_token_is_refused() {
    assert_eq!(exchange("a missing token", "\"\""), "403");
}

#[test]
fn a_state_changing_request_with_a_wrong_token_is_refused() {
    assert_eq!(
        exchange("a wrong token", "\"_csrf=\" + token + \"0\""),
        "403"
    );
}

#[test]
fn a_token_minted_for_another_visitor_is_refused() {
    // ★ THE ONE THAT MATTERS. An attacker can always obtain a token for THEIR OWN session — they
    // just load the page. It protects nothing unless the victim's session rejects it.
    let src = format!(
        "{SESSION}{MK}{ROUTES}\
         ## Main\n\
         Let mine be __lgx_route_handler(mk(\"GET\", \"/form\", \"\", \"\")).\n\
         Let theirs be __lgx_route_handler(mk(\"GET\", \"/form\", \"\", \"\")).\n\
         Let victim be cutBefore(headerIn(theirs's headers, \"set-cookie\"), \";\").\n\
         Let b be __lgx_route_handler(mk(\"POST\", \"/save\", victim, \"_csrf=\" + mine's body)).\n\
         Show b's status.\n"
    );
    assert_eq!(
        run("an attacker's own token against a victim's session", &src),
        "403",
        "a token minted for one visitor must not be accepted for another"
    );
}

#[test]
fn two_visitors_get_different_tokens() {
    // ★ ANTI-VACUITY for the test above: if every visitor got the SAME token, the refusal there
    // would be measuring something else entirely and this whole file would be theatre.
    let src = format!(
        "{SESSION}{MK}{ROUTES}\
         ## Main\n\
         Let a be __lgx_route_handler(mk(\"GET\", \"/form\", \"\", \"\")).\n\
         Let b be __lgx_route_handler(mk(\"GET\", \"/form\", \"\", \"\")).\n\
         If a's body is equal to b's body:\n\x20   Show \"same\".\n\
         Show \"done\".\n"
    );
    let out = run("two visitors", &src);
    assert!(
        !out.contains("same"),
        "two visitors must not share a CSRF token, got {out:?}"
    );
}

#[test]
fn a_read_only_request_is_never_checked() {
    // A `get` changes nothing, so requiring a token on one would break every link on the site.
    let src = format!(
        "{SESSION}{MK}{ROUTES}\
         ## Main\n\
         Let a be __lgx_route_handler(mk(\"GET\", \"/form\", \"\", \"\")).\n\
         Show a's status.\n"
    );
    assert_eq!(run("a GET", &src), "200");
}

#[test]
fn a_program_with_no_session_is_unchanged() {
    // ★ THE GATE. Automatic checking arrives WITH sessions, so a page that has been verifying by
    // hand keeps working exactly as it did and nothing that builds today starts answering 403.
    let src = format!(
        "{MK}## Route POST /save -> Text:\n\x20   Return \"saved\".\n\n\
         ## Main\n\
         Let b be __lgx_route_handler(mk(\"POST\", \"/save\", \"\", \"\")).\n\
         Show b's status.\nShow b's body.\n"
    );
    assert_eq!(run("no session declared", &src), "200\nsaved");
}

#[test]
fn route_status_agrees_with_the_served_handler_about_a_missing_token() {
    // ★ `routeStatus`/`routeStatusWith` used to skip the check entirely: this request has no
    // session cookie, so the served handler refuses it with 403, and `routeStatusWith` — asked
    // the identical question, "what status does this request get" — used to answer 200, the
    // route's own ordinary success. A test written against `routeStatusWith` to prove `/save` is
    // CSRF-protected passed whether or not it was.
    let via_handler = format!(
        "{SESSION}{MK}{ROUTES}\
         ## Main\nLet b be __lgx_route_handler(mk(\"POST\", \"/save\", \"\", \"\")).\nShow b's status.\n"
    );
    let via_status =
        format!("{SESSION}{ROUTES}## Main\nShow routeStatusWith(\"POST\", \"/save\", \"\").\n");
    assert_eq!(run("via the served handler", &via_handler), "403");
    assert_eq!(
        run("via routeStatusWith", &via_status),
        "403",
        "routeStatus must refuse exactly what the served handler refuses"
    );
}

#[test]
fn route_status_still_answers_a_read_only_request_without_checking_csrf() {
    // The other half: `routeStatus` on a GET is unaffected, exactly like the served handler.
    let src =
        format!("{SESSION}{ROUTES}## Main\nShow routeStatusWith(\"GET\", \"/form\", \"\").\n");
    assert_eq!(run("a GET via routeStatusWith", &src), "200");
}

#[test]
fn route_content_type_and_route_are_unchanged_by_a_missing_token() {
    // `route` and `routeContentType` answer a structural question about the URL, not about one
    // incoming request's security posture, and stay exactly as they were — only `routeStatus`
    // changed, because "what status does this request get" is the one question CSRF answers.
    let src = format!(
        "{SESSION}{ROUTES}## Main\nShow route(\"POST\", \"/save\").\nShow routeContentType(\"POST\", \"/save\").\n"
    );
    let out = run("route and routeContentType", &src);
    assert_eq!(
        out, "saved\ntext/plain; charset=utf-8",
        "unaffected by the missing token: {out}"
    );
}

#[test]
fn it_builds_on_the_tier_a_server_ships_as() {
    let src = format!("{SESSION}{ROUTES}## Main\nShow route(\"GET\", \"/form\").\n");
    let rust = compile_to_rust(&src).unwrap_or_else(|e| panic!("must compile: {e:?}\n{src}"));
    let refusals: Vec<&str> = rust
        .lines()
        .filter(|l| l.contains("compile_error!"))
        .collect();
    assert!(
        refusals.is_empty(),
        "would not build:\n{}\n{src}",
        refusals.join("\n")
    );
}
