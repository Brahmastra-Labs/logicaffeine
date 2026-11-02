//! A2 — the `http.lg` stdlib surface.
//!
//! `Request`/`Response` value types and the pure response constructors
//! (`plain`/`html`/`json`/`notFound`) must auto-import on first reference and run
//! on the interpreter, so a handler can build a response with ordinary LOGOS.
//! These are the types the `Serve` keystone (A3) marshals across the wire.

mod common;
use common::assert_all_tiers;

#[test]
fn plain_response_constructs_and_reads_back() {
    // Referencing `plain` alone must pull in http.lg (auto-import), construct a
    // Response, and expose its fields.
    assert_all_tiers(
        r#"## Main
Let r be plain("hello").
Show r's status.
Show r's contentType.
Show r's body.
"#,
        "200\ntext/plain\nhello",
    );
}

#[test]
fn html_and_json_set_their_content_types() {
    assert_all_tiers(
        r#"## Main
Let a be html("<h1>hi</h1>").
Let b be json("{{}}").
Show a's contentType.
Show a's body.
Show b's contentType.
Show b's body.
"#,
        "text/html\n<h1>hi</h1>\napplication/json\n{}",
    );
}

#[test]
fn not_found_is_a_404() {
    assert_all_tiers(
        r#"## Main
Let r be notFound().
Show r's status.
Show r's body.
"#,
        "404\nNot Found",
    );
}

#[test]
fn a_handler_can_branch_and_return_a_response() {
    // The shape a real handler takes: a function Request -> Response that
    // dispatches on the path. Proves the types compose into handler logic.
    assert_all_tiers(
        r#"## To handle (request: Request) -> Response:
    If request's path is equal to "/":
        Return html("<h1>home</h1>").
    Return notFound().

## Main
Let home be a new Request with method "GET" and path "/" and body "".
Let missing be a new Request with method "GET" and path "/nope" and body "".
Let homeResponse be handle(home).
Let missingResponse be handle(missing).
Show homeResponse's status.
Show missingResponse's status.
"#,
        "200\n404",
    );
}
