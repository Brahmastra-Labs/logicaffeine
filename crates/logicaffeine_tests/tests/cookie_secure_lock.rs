//! ★ LOCK — `Secure` is a DECISION, and the default is still secure. ★
//!
//! `cookie(name, value)` appends `; Path=/; HttpOnly; SameSite=Lax; Secure` — unconditionally. A
//! `Secure` cookie is never sent over plain HTTP, so **a session could not survive one hop on any
//! address that is not HTTPS**: the browser accepts the `Set-Cookie` and then declines to send it
//! back, which looks exactly like a login that silently does not log anybody in.
//!
//! ★ `localhost` IS NOT THE CASE THAT BITES. Browsers treat it as a trustworthy origin and do send
//! `Secure` cookies to `http://localhost`. The one that bites is the LAN address — `http://192.168.x.x`,
//! which is how this framework's own end-to-end proof serves a real site to a real device, and how
//! anybody tests a page on their phone. There, `Secure` is silently fatal.
//!
//! ★ SO THE DEFAULT DOES NOT MOVE. `Secure` stays on unless a developer asks for it off, because
//! the failure modes are not symmetrical: a missing `Secure` in production is a session token
//! travelling in clear text on every request, and an unwanted one in development is a login that
//! does not stick. Only the second is noticed immediately, and only the first is a breach. Opting
//! OUT is a decision somebody makes on purpose; opting IN is one they forget.
//!
//! `cookieFor(name, value, secure)` is the form that says which it wants, `cookie` is `cookieFor`
//! with the secure answer, and `LOGOS_COOKIE_INSECURE` is how a developer says otherwise.

use logicaffeine_compile::compile::{compile_to_rust, tw_outcome};

fn prints(subject: &str, src: &str) -> String {
    let out = tw_outcome(src);
    assert_eq!(out.error, None, "`{subject}` must run:\n{src}");
    out.output.trim().to_string()
}

#[test]
fn the_default_cookie_is_still_secure() {
    // ★ THE FLOOR, and the most important test in the file: making `Secure` optional must not
    // make it absent. Everything that ships today keeps the attributes it had.
    let got = prints("the default", "## Main\nShow cookie(\"sid\", \"ada\").\n");
    assert_eq!(got, "sid=ada; Path=/; HttpOnly; SameSite=Lax; Secure");
}

#[test]
fn a_cookie_can_be_asked_for_without_secure() {
    let got = prints(
        "insecure",
        "## Main\nShow cookieFor(\"sid\", \"ada\", false).\n",
    );
    assert_eq!(
        got, "sid=ada; Path=/; HttpOnly; SameSite=Lax",
        "dropping `Secure` must drop ONLY `Secure` — `HttpOnly` and `SameSite` defend against \
         different attacks and neither has anything to do with the transport"
    );
}

#[test]
fn asking_for_secure_explicitly_gives_the_default() {
    let got = prints(
        "explicit",
        "## Main\nShow cookieFor(\"sid\", \"ada\", true).\n",
    );
    assert_eq!(
        got,
        prints("default", "## Main\nShow cookie(\"sid\", \"ada\").\n")
    );
}

#[test]
fn an_unsafe_name_or_value_is_still_refused_either_way() {
    // The existing guard is a `""` answer, and it must not be reachable around the new form —
    // a cookie containing a `;` is a second cookie the caller did not write.
    assert_eq!(
        prints(
            "bad value",
            "## Main\nShow \"[\" + cookie(\"a\", \"b;c\") + \"]\".\n"
        ),
        "[]"
    );
    assert_eq!(
        prints(
            "bad value, insecure",
            "## Main\nShow \"[\" + cookieFor(\"a\", \"b;c\", false) + \"]\".\n"
        ),
        "[]"
    );
    assert_eq!(
        prints(
            "bad name",
            "## Main\nShow \"[\" + cookieFor(\"a b\", \"c\", false) + \"]\".\n"
        ),
        "[]"
    );
}

#[test]
fn it_builds_on_the_tier_a_server_ships_as() {
    let src = "## Main\nShow cookieFor(\"sid\", \"ada\", false).\nShow cookie(\"sid\", \"ada\").\n";
    let rust = compile_to_rust(src).unwrap_or_else(|e| panic!("must compile: {e:?}"));
    let refusals: Vec<&str> = rust
        .lines()
        .filter(|l| l.contains("compile_error!"))
        .collect();
    assert!(
        refusals.is_empty(),
        "would not build:\n{}",
        refusals.join("\n")
    );
}
