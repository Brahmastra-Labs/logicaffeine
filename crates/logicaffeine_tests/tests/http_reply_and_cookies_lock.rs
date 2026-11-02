//! ★ LOCK — the replies a guard gives, and the cookie a session rides on. ★
//!
//! `http.lg` could say `plain`, `html`, `json` and `notFound`, and nothing else. A middleware
//! guard's whole job is to REFUSE — the plan's own example is `Return some(unauthorized())` — and
//! there was no way to write one: an author had to hand-build a `Response` with the number 401 in
//! it, which is how a codebase ends up with three spellings of the same refusal and one of them
//! wrong.
//!
//! ★ AND THERE WERE NO COOKIES AT ALL. A cookie is how a session survives a request, which makes
//! it the single most security-sensitive header a framework emits — and one an author must not be
//! left to spell by hand:
//!
//!   · `HttpOnly` keeps a script from reading it, which is what makes an XSS a nuisance rather
//!     than a full account takeover.
//!   · `SameSite` is what a CSRF defence rests on: a cookie the browser refuses to attach to a
//!     cross-site request cannot be replayed by another site's form.
//!   · `Secure` keeps it off plaintext HTTP, where anyone on the path can read it.
//!
//! A helper that omits any of those by default hands every caller the insecure spelling, so the
//! defaults here are the safe ones and the caller opts OUT.

mod common;
use common::assert_all_tiers;

#[test]
fn a_guard_can_refuse_with_the_status_that_says_why() {
    // 401 and 403 mean DIFFERENT things — "I do not know who you are" and "I know, and no" — and a
    // guard that conflated them would send an authenticated user to a login page they are already
    // past, forever.
    assert_all_tiers(
        r#"## Main
Let a be unauthorized().
Let b be forbidden().
Show a's status.
Show b's status.
Show a's contentType.
"#,
        "401\n403\ntext/plain",
    );
}

#[test]
fn a_redirect_names_where_it_is_going() {
    // A redirect with no `Location` is a dead end: the browser is told to go elsewhere and not
    // where, so it stays put and the visitor sees nothing happen.
    assert_all_tiers(
        r#"## Main
Let r be redirectTo("/login").
Show r's status.
Show item "location" of r's headers.
"#,
        "302\n/login",
    );
}

#[test]
fn a_permanent_redirect_is_a_different_promise() {
    // 301 tells a browser and every cache to REMEMBER the move — it is close to irreversible, so
    // it must be asked for by name rather than being what `redirectTo` happens to do.
    assert_all_tiers(
        r#"## Main
Show redirectTo("/a")'s status.
Show permanentRedirectTo("/b")'s status.
"#,
        "302\n301",
    );
}

#[test]
fn a_cookie_is_http_only_and_same_site_and_secure_by_default() {
    // ★ THE DEFAULTS ARE THE SAFE ONES. A helper that omitted these would hand every caller the
    // insecure spelling — and the caller who most needs them is the one least likely to know to
    // ask. `HttpOnly` makes an XSS a nuisance rather than an account takeover; `SameSite=Lax` is
    // what stops another site's form replaying the cookie; `Secure` keeps it off plaintext HTTP.
    assert_all_tiers(
        r#"## Main
Let c be cookie("sid", "abc123").
Show c.
"#,
        "sid=abc123; Path=/; HttpOnly; SameSite=Lax; Secure",
    );
}

#[test]
fn a_response_carries_its_cookie_on_the_set_cookie_header() {
    assert_all_tiers(
        r#"## Main
Let r be withCookie(plain("hi"), "sid", "abc123").
Show item "set-cookie" of r's headers.
"#,
        "sid=abc123; Path=/; HttpOnly; SameSite=Lax; Secure",
    );
}

#[test]
fn a_request_reads_the_cookie_it_was_sent() {
    // The other half of the round trip. A `Cookie` header is a `; `-separated list, and a server
    // that only understood one cookie would break the moment anything else set one.
    assert_all_tiers(
        r#"## Main
Let mutable h be a new Map of Text to Text.
Set item "cookie" of h to "theme=dark; sid=abc123; lang=en".
Let r be a new Request with method "GET" and path "/" and body "" and headers h and query (a new Map of Text to Text).
Show cookieOf(r, "sid").
Show cookieOf(r, "theme").
Show cookieOf(r, "missing").
"#,
        // `Show ""` emits NO LINE at all, so a refused/absent value shows as the ABSENCE of a
        // line rather than a blank one — which is why the missing cookie contributes nothing here.
        "abc123\ndark",
    );
}

#[test]
fn a_cookie_value_that_would_break_the_header_is_refused() {
    // ★ THE INJECTION EDGE. A `;` or a newline in a value ends the cookie — or the HEADER — and
    // everything after it becomes something the server never wrote. That is response splitting,
    // and a session cookie is exactly where an attacker would aim it. Refusing beats sanitising:
    // a silently-truncated session id is a bug nobody can see.
    assert_all_tiers(
        r#"## Main
Show cookie("sid", "ok").
Show cookie("sid", "a; Path=/evil").
Show cookie("sid", "a
b").
Show cookie("s;d", "ok").
"#,
        // Three refusals, and `Show ""` emits no line — so a refusal that leaked ANY output
        // would fail this, which is exactly the assertion wanted.
        "sid=ok; Path=/; HttpOnly; SameSite=Lax; Secure",
    );
}

#[test]
fn a_cookie_can_be_expired_so_a_logout_really_logs_out() {
    // Deleting a cookie means SETTING it with a zero lifetime — there is no other way to say it —
    // and a logout that could not do this would leave the session cookie in the browser.
    assert_all_tiers(
        r#"## Main
Show expiredCookie("sid").
"#,
        "sid=; Path=/; Max-Age=0; HttpOnly; SameSite=Lax; Secure",
    );
}

#[test]
fn an_empty_cookie_name_is_refused_rather_than_emitting_a_nameless_cookie() {
    // `=value` is not a cookie. A browser's reading of it is inconsistent, so a server must not
    // emit one and hope.
    assert_all_tiers(
        r#"## Main
Show cookie("", "v").
Show "end".
"#,
        "end",
    );
}
