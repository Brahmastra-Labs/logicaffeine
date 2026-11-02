//! ★ LOCK — `## Session` is server state that survives a request, or it does not exist. ★
//!
//! ★ TODAY IT PARSES AND DOES NOTHING. `## Session\n    a user, which is Text.` compiles clean,
//! runs, and the declaration has no effect whatsoever — which is the exact failure
//! `server_surface_lock`'s own header condemns in the four server headers: *"a declaration that is
//! accepted and then quietly does nothing, because the author has been told their intent was
//! understood."* Three of those four refused loudly; this one is not even known to the parser, so
//! it falls through to prose.
//!
//! ★ AND IT IS THE LAST PIECE OF THE LOGIN. `route_request_response_lock` lifted the ceiling that
//! stopped a route receiving a `Request` or answering a `Response`, so a cookie can now be set and
//! read. What is still missing is the thing worth putting IN one: state that belongs to a visitor
//! and outlives a single request. Without it every page is anonymous, and `examples/guestbook`
//! records the consequence under its own "Honest limits" heading — its
//! `## To session () -> Text: Return "everyone".` gives every visitor an identical CSRF token.
//!
//! **The design**, and the reason for each half:
//!
//! * The session rides in ONE cookie, signed with `signSession`. `HttpResponse.headers` is a
//!   `BTreeMap`, so a response carries exactly one `Set-Cookie` — a second silently overwrites the
//!   first. One cookie is not a simplification, it is the only correct number.
//! * Its payload is the record's fields, each HEX-ENCODED, joined with `.`. Hex because a cookie
//!   value may not contain `;`, `,`, `"`, a space or a control character (`isCookieSafe` enforces
//!   exactly that), and a field is whatever text the program put in it. Hex has no escaping rules
//!   to get wrong, no `.` of its own, and `bytesToHex` already exists.
//! * A cookie is attacker-controlled input, so a payload whose signature does not check is not an
//!   error and not a crash — it is simply no session, the same as a visitor arriving with none.
//! * A request that does not WRITE to the session sets no cookie. Re-issuing one on every response
//!   would put a `Set-Cookie` on every 404 and every static asset, and would collide with a route
//!   that sets a cookie of its own.

use logicaffeine_compile::compile::{compile_to_rust, tw_outcome};

const SESSION: &str = "## Session\n\x20   a user, which is Text.\n\n";

/// A LOGOS helper that builds a `Request` carrying `cookie`, so a test can thread the reply of one
/// request into the next the way a browser does.
const MK: &str = "## To mk (m: Text) and (p: Text) and (c: Text) -> Request:\n\
                 \x20   Let h be a new Map of Text to Text.\n\
                 \x20   Set item \"cookie\" of h to c.\n\
                 \x20   Return a new Request with method m and path p and body \"\" and headers h \
                  and query (a new Map of Text to Text).\n\n";

fn run(subject: &str, src: &str) -> String {
    let out = tw_outcome(src);
    assert_eq!(out.error, None, "`{subject}` must run:\n{src}");
    out.output.trim().to_string()
}

/// The two-request round trip: write the session, take the `Set-Cookie`, send it back, read it.
///
/// ★ THE WRITING REQUEST IS A `GET`, ON PURPOSE. A `POST` with a `## Session` declared is CSRF-
/// checked by the router, so a POST fixture here would be measuring two features at once and would
/// fail for the CSRF reason while claiming to be about sessions. Writing a session on a GET is a
/// real shape anyway — a visit counter is exactly that. The realistic composition, a form that
/// fetches a token and then POSTs it, is locked next door in `csrf_automatic_lock`, which asserts
/// the same round trip end to end and answers `200`.
///
/// `tamper` is applied to the cookie in LOGOS between the two requests — `""` leaves it alone.
fn round_trip(tamper: &str) -> String {
    let src = format!(
        "{SESSION}{MK}\
         ## Route GET /login -> Response:\n\
        \x20   Set the session's user to \"ada\".\n\
        \x20   Return plain(\"in\").\n\n\
         ## Route GET /me -> Text:\n\
        \x20   Return the session's user.\n\n\
         ## Main\n\
         Let a be __lgx_route_handler(mk(\"GET\", \"/login\", \"\")).\n\
         Let raw be headerIn(a's headers, \"set-cookie\").\n\
         Let sent be cutBefore(raw, \";\"){tamper}.\n\
         Let b be __lgx_route_handler(mk(\"GET\", \"/me\", sent)).\n\
         Show b's body.\n"
    );
    run("a session round trip", &src)
}

#[test]
fn a_session_declaration_is_not_silently_ignored() {
    // ★ THE FLOOR, and the whole reason this file exists. Whatever else is true, a `## Session`
    // block must CHANGE something — either it works, or the program is refused by name. Compiling
    // clean and doing nothing is the one outcome that is not allowed.
    // The evidence is that the fields it declared are THERE. Before, `## Session` fell through to
    // prose, so this program died at `Undefined variable: session` — the declaration had produced
    // nothing at all.
    let src = format!("{SESSION}## Main\nShow \"[\" + the session's user + \"]\".\n");
    assert_eq!(
        run("a bare session declaration", &src),
        "[]",
        "a `## Session` must actually declare its fields:\n{src}"
    );
}

#[test]
fn a_session_written_in_one_request_is_read_in_the_next() {
    // ★ THE HEADLINE: a login. This is the thing that could not be written at all before.
    assert_eq!(round_trip(""), "ada");
}

#[test]
fn a_forged_cookie_is_no_session_rather_than_a_wrong_one() {
    // Flip one character of the signed payload. The signature stops checking, so the visitor is
    // simply anonymous — not an error, not a crash, and emphatically not "ada".
    let forged = round_trip(" + \"0\"");
    assert_eq!(
        forged, "",
        "a cookie whose signature does not check must be NO session; got {forged:?}"
    );
}

#[test]
fn a_cookie_with_no_signature_at_all_is_refused() {
    let src = format!(
        "{SESSION}{MK}\
         ## Route GET /me -> Text:\n\x20   Return the session's user.\n\n\
         ## Main\n\
         Let b be __lgx_route_handler(mk(\"GET\", \"/me\", \"lgxsession=deadbeef\")).\n\
         Show b's body.\n"
    );
    assert_eq!(run("an unsigned cookie", &src), "");
}

#[test]
fn a_request_that_does_not_write_the_session_sets_no_cookie() {
    // Re-issuing the cookie on every reply would put a `Set-Cookie` on every 404 and every asset,
    // and would silently overwrite a cookie a route set for itself — there is only one slot.
    let src = format!(
        "{SESSION}{MK}\
         ## Route GET /hi -> Text:\n\x20   Return \"hi\".\n\n\
         ## Main\n\
         Let a be __lgx_route_handler(mk(\"GET\", \"/hi\", \"\")).\n\
         Show \"[\" + headerIn(a's headers, \"set-cookie\") + \"]\".\n"
    );
    assert_eq!(run("a read-only request", &src), "[]");
}

fn is_lower_hex(h: &str) -> bool {
    h.len() % 2 == 0 && h.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// `h` as the UTF-8 text its lowercase-hex bytes spell, or `None` when it is not lowercase hex.
fn unhex(h: &str) -> Option<String> {
    if !is_lower_hex(h) {
        return None;
    }
    let bytes = (0..h.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&h[i..i + 2], 16).ok())
        .collect::<Option<Vec<u8>>>()?;
    String::from_utf8(bytes).ok()
}

#[test]
fn the_session_cookie_is_signed_rather_than_readable() {
    // The value is hex fields joined with `.`, then `|` and a MAC. Asserting the VISITOR'S NAME IS
    // NOT IN IT plainly is the point: a cookie a visitor can read and edit is not a session, it is a
    // suggestion. The name has letters no hex digit spells, so the MAC under a fresh random key can
    // never write it into the header by chance, and the absence is read across the whole header.
    let src = format!(
        "{SESSION}{MK}\
         ## Route GET /login -> Response:\n\
        \x20   Set the session's user to \"turing\".\n\
        \x20   Return plain(\"in\").\n\n\
         ## Main\n\
         Let a be __lgx_route_handler(mk(\"GET\", \"/login\", \"\")).\n\
         Show headerIn(a's headers, \"set-cookie\").\n"
    );
    let cookie = run("a signed cookie", &src);
    assert!(
        !cookie.contains("turing"),
        "the visitor's own name is sitting in the cookie in plain text: {cookie:?}"
    );
    let value = cookie
        .strip_prefix("lgxsession=")
        .and_then(|rest| rest.split(';').next())
        .unwrap_or_else(|| panic!("the session rides in the `lgxsession` cookie: {cookie:?}"));
    let (payload, mac) = value
        .rsplit_once('|')
        .unwrap_or_else(|| panic!("the cookie must carry its signature: {cookie:?}"));
    assert!(
        mac.len() == 64 && is_lower_hex(mac),
        "the signature is an HMAC-SHA-256 in lowercase hex: {cookie:?}"
    );
    let fields = payload
        .split('.')
        .map(unhex)
        .collect::<Option<Vec<String>>>()
        .unwrap_or_else(|| panic!("every payload field is lowercase hex: {cookie:?}"));
    assert!(
        fields.iter().any(|f| f == "turing"),
        "the payload carries the visitor's name, encoded: {cookie:?}"
    );
}

#[test]
fn a_session_builds_on_the_tier_a_server_ships_as() {
    // ★ AOT-Rust is the only tier that runs the static type checker, and a session synthesizes a
    // record, a global and two functions around a derived JSON codec — every one of which is a
    // place a type can disagree. `Ok` proves nothing; the emitted Rust is read for refusals.
    let src = format!(
        "{SESSION}\
         ## Route POST /login -> Response:\n\
        \x20   Set the session's user to \"ada\".\n\
        \x20   Return redirectTo(\"/\").\n\n\
         ## Route GET /me -> Text:\n\x20   Return the session's user.\n\n\
         ## Main\nShow route(\"GET\", \"/me\").\n"
    );
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
