//! ★ LOCK — a declared error page is served AS A PAGE. ★
//!
//! `## Error page 404` renders a view, and the router serves its markup with the content type it
//! gives every answer of its own:
//!
//! ```text
//! ## Error page 404
//! View:
//!     <h1>Gone</h1>
//!
//! routeContentType("GET", "/nope")  →  "text/plain; charset=utf-8"
//! route("GET", "/nope")             →  "<h1>Gone</h1>"
//! ```
//!
//! ★ A BROWSER TOLD `text/plain` SHOWS THE MARKUP, so a visitor who mistypes a URL is handed the
//! literal characters `<h1>Gone</h1>` on a white page. The author wrote a view, the framework
//! rendered it, and the one header that decides whether it is a document or a string was left at
//! the value the router uses for `404` — which is right when the answer IS the bare string `404`
//! and wrong the moment somebody declares a page.
//!
//! The router's own answers — a bare status, a redirect — are plain text, because that is what
//! they are. An error PAGE is not one of those, and the difference is exactly whether a
//! `## Error page` was declared for that status. That is a fact the lowering already has: it
//! collects `error_pages` before it builds any of these arms, and simply never asked.

use logicaffeine_compile::compile::{compile_to_rust, tw_outcome};

const ROUTE: &str = "## Route GET /a -> Text:\n\x20   Return \"a\".\n\n";

fn shows(subject: &str, src: &str) -> Vec<String> {
    let out = tw_outcome(src);
    assert_eq!(out.error, None, "`{subject}` must run:\n{src}");
    out.output.lines().map(|l| l.trim().to_string()).collect()
}

#[test]
fn a_declared_404_page_is_served_as_html() {
    let src = format!(
        "## Error page 404\nView:\n\x20   <h1>Gone</h1>\n\n{ROUTE}\
         ## Main\nShow routeContentType(\"GET\", \"/nope\").\nShow route(\"GET\", \"/nope\").\n"
    );
    let out = shows("a declared 404 page", &src);
    assert_eq!(
        out[0], "text/html; charset=utf-8",
        "the author wrote a VIEW; served as text/plain a browser shows the markup"
    );
    assert_eq!(out[1], "<h1>Gone</h1>");
}

#[test]
fn a_declared_405_page_is_served_as_html_too() {
    // ★ NOT A 404 SPECIAL CASE. `## Error page 405` is the other status the router answers with a
    // page, and a fix that knew only about 404 would leave it showing markup as text.
    let src = format!(
        "## Error page 405\nView:\n\x20   <h1>Nope</h1>\n\n{ROUTE}\
         ## Main\nShow routeContentType(\"POST\", \"/a\").\nShow route(\"POST\", \"/a\").\n"
    );
    let out = shows("a declared 405 page", &src);
    assert_eq!(out[0], "text/html; charset=utf-8");
    assert_eq!(out[1], "<h1>Nope</h1>");
}

#[test]
fn a_bare_status_is_still_plain_text() {
    // ★ THE FLOOR, and the reason the answer is not simply "always html": with no page declared
    // the router answers the STRING `404`, which is plain text and must stay so — an API client
    // reading a 404 should not be told it was handed a document.
    let src = format!(
        "{ROUTE}## Main\nShow routeContentType(\"GET\", \"/nope\").\nShow route(\"GET\", \"/nope\").\n"
    );
    let out = shows("a bare 404", &src);
    assert_eq!(out[0], "text/plain; charset=utf-8");
    assert_eq!(out[1], "404");
}

#[test]
fn a_bare_405_is_still_plain_text() {
    let src = format!("{ROUTE}## Main\nShow routeContentType(\"POST\", \"/a\").\n");
    assert_eq!(shows("a bare 405", &src)[0], "text/plain; charset=utf-8");
}

#[test]
fn declaring_one_page_does_not_change_the_other_status() {
    // The two are separate declarations — `server_surface_lock` already pins that declaring only
    // one leaves the other bare, "because 'this URL does not exist' and 'this URL does not do
    // that' are different mistakes". Their content types must follow the same rule.
    let src = format!(
        "## Error page 404\nView:\n\x20   <h1>Gone</h1>\n\n{ROUTE}\
         ## Main\nShow routeContentType(\"GET\", \"/nope\").\nShow routeContentType(\"POST\", \"/a\").\n"
    );
    let out = shows("only a 404 declared", &src);
    assert_eq!(out[0], "text/html; charset=utf-8");
    assert_eq!(
        out[1], "text/plain; charset=utf-8",
        "no 405 page was declared"
    );
}

#[test]
fn an_ordinary_route_keeps_its_own_content_type() {
    let src = format!("{ROUTE}## Main\nShow routeContentType(\"GET\", \"/a\").\n");
    assert_eq!(shows("a Text route", &src)[0], "text/plain; charset=utf-8");
}

#[test]
fn the_served_response_carries_it_too() {
    // The walk a real request runs, not just the convenience — the header is only useful if it
    // reaches the wire.
    let src = format!(
        "## Error page 404\nView:\n\x20   <h1>Gone</h1>\n\n{ROUTE}\
         ## To mk (p: Text) -> Request:\n\
        \x20   Return a new Request with method \"GET\" and path p and body \"\" \
         and headers (a new Map of Text to Text) and query (a new Map of Text to Text).\n\n\
         ## Main\nLet r be __lgx_route_handler(mk(\"/nope\")).\n\
         Show r's contentType.\nShow r's status.\nShow r's body.\n"
    );
    let out = shows("a served 404 page", &src);
    assert_eq!(out[0], "text/html; charset=utf-8");
    assert_eq!(out[1], "404");
    assert_eq!(out[2], "<h1>Gone</h1>");
}

#[test]
fn it_builds_on_the_tier_a_server_ships_as() {
    let src = format!(
        "## Error page 404\nView:\n\x20   <h1>Gone</h1>\n\n{ROUTE}\
         ## Main\nShow route(\"GET\", \"/nope\").\n"
    );
    let rust = compile_to_rust(&src).unwrap_or_else(|e| panic!("must compile: {e:?}\n{src}"));
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

/// ★ AN ERROR PAGE DRAWS WHAT ANY OTHER SERVER-RENDERED VIEW DRAWS — including a `bind`.
///
/// `server_surface_lock` used to hold a `## Error page` carrying `<input bind={draft}>` as its
/// example of a declaration the runtime could not honour, on the stated grounds that a `bind` "is
/// BROWSER BEHAVIOUR — a two-way binding needs an element to listen to, and a served string has
/// none — so there is nothing here to implement later".
///
/// ★ THAT CONFLATED THE TWO HALVES OF A BINDING. The WRITE half is browser behaviour and always
/// was. The READ half — the field's CURRENT VALUE, written as the attribute the element reads it
/// from — is what a server-rendered form is FOR: a reader who never loaded the app sees what is in
/// the fields, and a browser that hydrates finds the values it would have written.
///
/// So the refusal moved, and this is the other half of that move: proving the page now RENDERS
/// rather than merely no longer refusing. A widening that stops a refusal without drawing anything
/// is the "rendered short" outcome the server walk exists to prevent — and the only way to tell
/// those two apart is to read the markup.
#[test]
fn an_error_page_renders_a_bound_field_with_its_value() {
    let src = format!(
        "## Error page 404\nState:\n\x20   Let draft be \"notes\".\n\
         View:\n\x20   <input bind={{draft}}>\n\n{ROUTE}\
         ## Main\nShow route(\"GET\", \"/nope\").\n"
    );
    let out = shows("an error page with a bound field", &src);
    assert!(
        out[0].contains("value=\"notes\""),
        "the served page carries what is in the field, not an empty box: {}",
        out[0]
    );
}
