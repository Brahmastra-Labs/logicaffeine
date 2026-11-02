//! G2 — HTTP header awareness for the LOGOS `http.lg` surface.
//!
//! The wire core (`logicaffeine_system::http::wire`) already parses request
//! headers (lowercased names) and the query string; this lock pins the LANGUAGE
//! side: `Request` carries `headers` + `query` maps, `Response` carries a
//! `headers` map, and the pure-LOGOS helpers (`headerOf`/`hasHeader`/`queryOf`/
//! `hasQuery`/`withHeader`) give handlers typed, case-insensitive access with a
//! hop-by-hop denylist and a CRLF response-splitting guard.
//!
//! Decided-and-locked semantics:
//! - `headerOf` is CASE-INSENSITIVE and returns `""` when absent (total, like
//!   `counterGet`); `hasHeader` distinguishes an empty value from absence.
//! - When two keys of a hand-built map differ only by case, the FIRST in map
//!   (insertion) order wins.
//! - `queryOf` is case-sensitive (query keys are data, not protocol tokens).
//! - `withHeader` returns `Result of Response and Text` (the stdlib's error
//!   convention — file.lg/tcp.lg): `Err` on hop-by-hop names (connection,
//!   content-length, transfer-encoding, keep-alive, upgrade), on content-type
//!   (reserved to the `contentType` field), on a non-token name, and on CR/LF
//!   or NUL anywhere in the value. Never a silent drop.
//! - `withHeader` stores names lowercased, so a re-set under different casing
//!   OVERWRITES (one wire line, last write wins), and it never mutates its
//!   input Response (value semantics).

mod common;
use common::{assert_all_tiers, assert_compiled_equals_interpreted_eq};

// =========================================================================
// SPIKE LOCK — the representation this whole feature stands on: a struct
// field typed `Map of Text to Text` must survive the AOT-Rust tier
// (construction with the field, construction with the field OMITTED,
// field access, entry iteration inside a fn, and a `Result of Response
// and Text`-shaped return). If this breaks, everything above it breaks.
// =========================================================================

#[test]
fn map_of_text_field_survives_the_aot_tier() {
    assert_compiled_equals_interpreted_eq(
        r#"## Definition
A Pack has:
    a tag, which is Text.
    a labels, which is Map of Text to Text.

## To pickCI (p: Pack) and (name: Text) -> Text:
    Let want be lower(name).
    Let m be p's labels.
    Repeat for (k, v) in m:
        If lower(k) is equal to want:
            Return v.
    Return "".

## Main
Let mutable m be a new Map of Text to Text.
Set item "Host" of m to "example.com".
Set item "x-a" of m to "1".
Let p be a new Pack with tag "t" and labels m.
Show pickCI(p, "host").
Show pickCI(p, "X-A").
Show pickCI(p, "missing").
Let bare be a new Pack with tag "empty".
Show length of (bare's labels).
"#,
        "example.com\n1\n\n0",
    );
}

#[test]
fn result_of_struct_return_survives_the_aot_tier() {
    assert_compiled_equals_interpreted_eq(
        r#"## Definition
A Wrap has:
    a note, which is Text.
    a labels, which is Map of Text to Text.

## To stamp (w: Wrap) and (k: Text) and (v: Text) -> Result of Wrap and Text:
    If k is equal to "":
        Return a new Err with error "empty key".
    Let mutable fresh be a new Map of Text to Text.
    Let m be w's labels.
    Repeat for (a, b) in m:
        Set item a of fresh to b.
    Set item k of fresh to v.
    Let n be w's note.
    Let out be a new Wrap with note n and labels fresh.
    Return a new Ok with value out.

## Main
Let w be a new Wrap with note "hi".
Let r be stamp(w, "color", "red").
Inspect r:
    When Ok (w2):
        Show w2's note.
        Show item "color" of w2's labels.
    When Err (e):
        Show e.
Let bad be stamp(w, "", "x").
Inspect bad:
    When Ok (w3):
        Show "wrong".
    When Err (e):
        Show e.
Show length of (w's labels).
"#,
        "hi\nred\nempty key\n0",
    );
}

// =========================================================================
// Request carries headers + query
// =========================================================================

#[test]
fn request_struct_carries_headers_and_query_maps() {
    assert_all_tiers(
        r#"## Main
Let mutable h be a new Map of Text to Text.
Set item "host" of h to "example.com".
Let mutable q be a new Map of Text to Text.
Set item "page" of q to "2".
Let r be a new Request with method "GET" and path "/" and body "" and headers h and query q.
Show length of (r's headers).
Show length of (r's query).
Show item "host" of (r's headers).
Show item "page" of (r's query).
"#,
        "1\n1\nexample.com\n2",
    );
}

#[test]
fn request_constructed_without_headers_or_query_defaults_to_empty_maps() {
    // Every existing hand-built `a new Request with method … and path … and
    // body …` keeps working: the new fields default to empty maps.
    assert_all_tiers(
        r#"## Main
Let r be a new Request with method "GET" and path "/" and body "".
Show length of (r's headers).
Show length of (r's query).
Show headerOf(r, "host").
Show hasHeader(r, "host").
Show queryOf(r, "page").
Show hasQuery(r, "page").
"#,
        "0\n0\n\nfalse\n\nfalse",
    );
}

// =========================================================================
// headerOf / hasHeader — case-insensitive request-header lookup
// =========================================================================

/// A request whose headers arrived from the wire core: names already
/// lowercased, exactly as `parse_request` normalizes them.
fn wire_shaped_request(extra: &str, expected: &str) -> (String, String) {
    (
        format!(
            r#"## Main
Let mutable h be a new Map of Text to Text.
Set item "host" of h to "example.com".
Set item "content-type" of h to "application/json".
Set item "x-empty" of h to "".
Let r be a new Request with method "GET" and path "/" and body "" and headers h.
{extra}"#
        ),
        expected.to_string(),
    )
}

#[test]
fn header_lookup_is_case_insensitive_battery() {
    // Every casing of the probe name resolves to the same wire-lowercased key.
    let (src, expected) = wire_shaped_request(
        r#"Show headerOf(r, "host").
Show headerOf(r, "Host").
Show headerOf(r, "HOST").
Show headerOf(r, "hOsT").
Show headerOf(r, "Content-Type").
Show headerOf(r, "CONTENT-TYPE").
"#,
        "example.com\nexample.com\nexample.com\nexample.com\napplication/json\napplication/json",
    );
    assert_all_tiers(&src, &expected);
}

#[test]
fn absent_header_reads_as_empty_and_has_header_distinguishes() {
    // `headerOf` is total: "" for absent. An EMPTY header value also reads as
    // "" — `hasHeader` is the disambiguator, and it is case-insensitive too.
    // Note the ordering: the all-tiers harness trims the joined output, so an
    // empty line (a shown "") must never be first or last.
    let (src, expected) = wire_shaped_request(
        r#"Show hasHeader(r, "x-missing").
Show headerOf(r, "x-missing").
Show hasHeader(r, "x-empty").
Show headerOf(r, "x-empty").
Show hasHeader(r, "X-Empty").
"#,
        "false\n\ntrue\n\ntrue",
    );
    assert_all_tiers(&src, &expected);
}

#[test]
fn case_duplicate_keys_in_a_hand_built_map_first_in_map_order_wins() {
    // The wire core can never produce two keys differing only by case (it
    // lowercases), but a hand-built Request can. Locked: the FIRST matching
    // key in map (insertion) order wins.
    assert_all_tiers(
        r#"## Main
Let mutable h be a new Map of Text to Text.
Set item "X-A" of h to "first".
Set item "x-a" of h to "second".
Let r be a new Request with method "GET" and path "/" and body "" and headers h.
Show headerOf(r, "x-a").
Show headerOf(r, "X-A").
"#,
        "first\nfirst",
    );
}

// =========================================================================
// queryOf / hasQuery — query access without manual path parsing
// =========================================================================

#[test]
fn query_lookup_reads_decoded_values_and_is_case_sensitive() {
    // Query values arrive percent-decoded from the wire core; keys are DATA,
    // so lookup is exact (case-sensitive), unlike header names.
    assert_all_tiers(
        r#"## Main
Let mutable q be a new Map of Text to Text.
Set item "q" of q to "hello world".
Set item "Page" of q to "2".
Set item "empty" of q to "".
Let r be a new Request with method "GET" and path "/search" and body "" and query q.
Show queryOf(r, "q").
Show queryOf(r, "Page").
Show queryOf(r, "page").
Show hasQuery(r, "page").
Show queryOf(r, "empty").
Show hasQuery(r, "empty").
Show hasQuery(r, "missing").
"#,
        "hello world\n2\n\nfalse\n\ntrue\nfalse",
    );
}

// =========================================================================
// withHeader — response headers with a hop-by-hop denylist and a
// response-splitting guard. Result of Response and Text, never silent.
// =========================================================================

#[test]
fn with_header_sets_a_header_without_mutating_the_original() {
    assert_all_tiers(
        r#"## Main
Let base be plain("hi").
Let r be withHeader(base, "X-Request-Id", "abc-123").
Inspect r:
    When Ok (resp):
        Show item "x-request-id" of (resp's headers).
        Show resp's status.
        Show resp's body.
    When Err (e):
        Show e.
Show length of (base's headers).
"#,
        "abc-123\n200\nhi\n0",
    );
}

#[test]
fn with_header_stores_lowercased_so_recasing_overwrites() {
    // One logical header = one wire line: names are stored lowercased, so
    // setting the same name under different casing overwrites (last wins).
    assert_all_tiers(
        r#"## Main
Let base be plain("hi").
Let step1 be withHeader(base, "X-Trace", "one").
Inspect step1:
    When Ok (a):
        Let step2 be withHeader(a, "x-TRACE", "two").
        Inspect step2:
            When Ok (b):
                Show length of (b's headers).
                Show item "x-trace" of (b's headers).
            When Err (e):
                Show e.
    When Err (e):
        Show e.
"#,
        "1\ntwo",
    );
}

#[test]
fn with_header_rejects_every_hop_by_hop_name_in_any_casing() {
    // connection / content-length / transfer-encoding / keep-alive / upgrade
    // are the wire's business, never the handler's; content-type is reserved
    // to the `contentType` field. Rejection is an Err, NEVER a silent drop.
    assert_all_tiers(
        r#"## To verdict (r: Result of Response and Text) -> Text:
    Inspect r:
        When Ok (resp):
            Return "accepted".
        When Err (e):
            Return e.

## Main
Let base be plain("hi").
Show verdict(withHeader(base, "Connection", "keep-alive")).
Show verdict(withHeader(base, "CONTENT-LENGTH", "999")).
Show verdict(withHeader(base, "Transfer-Encoding", "chunked")).
Show verdict(withHeader(base, "keep-alive", "timeout=5")).
Show verdict(withHeader(base, "Upgrade", "websocket")).
Show verdict(withHeader(base, "Content-Type", "text/evil")).
"#,
        "withHeader: connection is hop-by-hop; the wire owns it\n\
         withHeader: content-length is hop-by-hop; the wire owns it\n\
         withHeader: transfer-encoding is hop-by-hop; the wire owns it\n\
         withHeader: keep-alive is hop-by-hop; the wire owns it\n\
         withHeader: upgrade is hop-by-hop; the wire owns it\n\
         withHeader: set contentType on the Response instead of a content-type header",
    );
}

#[test]
fn with_header_rejects_crlf_injection_in_name_and_value() {
    // Response splitting: CR or LF anywhere in the name or value would let a
    // handler forge extra headers or an entire second response. NUL is
    // rejected in values too. All are Errs, never silent drops.
    assert_all_tiers(
        r#"## To verdict (r: Result of Response and Text) -> Text:
    Inspect r:
        When Ok (resp):
            Return "accepted".
        When Err (e):
            Return e.

## Main
Let base be plain("hi").
Let cr be text_from_bytes([13]).
Let lf be text_from_bytes([10]).
Let nul be text_from_bytes([0]).
Show verdict(withHeader(base, "X-A{cr}X-B", "v")).
Show verdict(withHeader(base, "X-A{lf}X-B", "v")).
Show verdict(withHeader(base, "X-A", "v{cr}{lf}Set-Cookie: pwned")).
Show verdict(withHeader(base, "X-A", "v{lf}injected")).
Show verdict(withHeader(base, "X-A", "v{nul}")).
Show verdict(withHeader(base, "X-A", "clean")).
"#,
        "withHeader: header name is not a valid token\n\
         withHeader: header name is not a valid token\n\
         withHeader: header value contains a forbidden control byte\n\
         withHeader: header value contains a forbidden control byte\n\
         withHeader: header value contains a forbidden control byte\n\
         accepted",
    );
}

#[test]
fn with_header_rejects_non_token_names() {
    // RFC 7230 token = ALPHA / DIGIT / ! # $ % & ' * + - . ^ _ ` | ~ — a
    // space, a colon, or an empty name can't be a header name on the wire.
    assert_all_tiers(
        r#"## To verdict (r: Result of Response and Text) -> Text:
    Inspect r:
        When Ok (resp):
            Return "accepted".
        When Err (e):
            Return e.

## Main
Let base be plain("hi").
Show verdict(withHeader(base, "", "v")).
Show verdict(withHeader(base, "X A", "v")).
Show verdict(withHeader(base, "X:A", "v")).
Show verdict(withHeader(base, "X-Good_1.token~", "v")).
"#,
        "withHeader: header name is not a valid token\n\
         withHeader: header name is not a valid token\n\
         withHeader: header name is not a valid token\n\
         accepted",
    );
}

#[test]
fn with_header_chains_accumulate_headers() {
    assert_all_tiers(
        r#"## To must (r: Result of Response and Text) -> Response:
    Inspect r:
        When Ok (resp):
            Return resp.
        When Err (e):
            Return plain(e).

## Main
Let base be json("{{}}").
Let r be must(withHeader(must(withHeader(base, "X-One", "1")), "X-Two", "2")).
Show length of (r's headers).
Show item "x-one" of (r's headers).
Show item "x-two" of (r's headers).
Show r's contentType.
"#,
        "2\n1\n2\napplication/json",
    );
}

// =========================================================================
// Handler-shaped composition: the audit's end shape — a handler that routes
// on a query param and echoes an identity header, all typed.
// =========================================================================

#[test]
fn a_handler_can_route_on_query_and_echo_headers() {
    assert_all_tiers(
        r#"## To handle (request: Request) -> Response:
    If queryOf(request, "who") is equal to "me":
        Return plain(headerOf(request, "X-User")).
    Return notFound().

## Main
Let mutable h be a new Map of Text to Text.
Set item "x-user" of h to "alice".
Let mutable q be a new Map of Text to Text.
Set item "who" of q to "me".
Let hit be a new Request with method "GET" and path "/" and body "" and headers h and query q.
Let miss be a new Request with method "GET" and path "/" and body "" and headers h.
Let a be handle(hit).
Let b be handle(miss).
Show a's body.
Show b's status.
"#,
        "alice\n404",
    );
}
