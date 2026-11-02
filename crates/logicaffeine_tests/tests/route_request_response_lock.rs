//! ★ LOCK — a `## Route` can receive the request and answer a whole response. ★
//!
//! A route could answer `Text`, a JSON-encodable record, a bare scalar or a `View`, and could take
//! `Text` and `Int` parameters bound from the path, query or body. It could **not** receive a
//! `Request` and could **not** answer a `Response` — and those two absences compose into the
//! ceiling this file exists to lift:
//!
//! ```text
//! ## Route GET /x -> Response:       →  "Unknown function: toJson_Response"
//! ## Route GET /x (r: Request) -> …  →  "one of its parameters is a type the path matcher
//!                                         cannot bind yet (Text and Int bind today)"
//! ```
//!
//! ★ WHAT THAT COSTS, following the chain: `withCookie` takes a `Response`, so **no route could
//! set a cookie**. `cookieOf` and `headerOf` take a `Request`, so **no route could read one**.
//! Together: no sessions, no auth, no POST/redirect/GET. `withCookie` appears exactly ONCE outside
//! the standard library in the whole repository — a unit test on `plain("hi")`.
//!
//! It also explains a documentation smell: `## To handle (request: Request) -> Response` is still
//! taught as the way to write a server, because it was the only form that could do cookies at all.
//! And `examples/guestbook` records the consequence under its own "Honest limits" heading — its
//! `## To session () -> Text: Return "everyone".` gives every visitor an identical CSRF token.
//!
//! ★ THE CAUSE IS TWO QUESTIONS THAT SHOULD BE ONE, which is the shape this campaign keeps
//! finding. `route_lowerable` asks *"is this a record?"* and accepts `Response`; the emitter then
//! asks *"is this record JSON-encodable?"* and `Response` is not — it holds a `Seq of Int` and a
//! `Map of Text to Text` — so `toJson_Response` is never derived and the route dies at an unknown
//! function. Neither question is wrong. There are just two of them, in two places, disagreeing.
//! `Response` is not a value to ENCODE; it is the answer itself, and passes through.

use logicaffeine_compile::compile::{compile_to_rust, tw_outcome};

/// Compiles on the tier a server ships as, with the emitted Rust read for refusals — `Ok` is
/// returned for a program that was refused, so `is_ok()` proves nothing here.
fn builds(subject: &str, src: &str) {
    let rust = compile_to_rust(src)
        .unwrap_or_else(|e| panic!("`{subject}` must compile to AOT-Rust: {e:?}\n{src}"));
    let refusals: Vec<&str> = rust
        .lines()
        .filter(|l| l.contains("compile_error!"))
        .collect();
    assert!(
        refusals.is_empty(),
        "`{subject}` would not build:\n{}\n{src}",
        refusals.join("\n")
    );
}

/// Runs on the reference tier and answers `expected`.
fn answers(subject: &str, src: &str, expected: &str) {
    let out = tw_outcome(src);
    assert_eq!(out.error, None, "`{subject}` on the tree-walker:\n{src}");
    assert_eq!(out.output.trim(), expected, "`{subject}`:\n{src}");
}

#[test]
fn a_route_may_answer_a_response() {
    let src = "## Route GET /x -> Response:\n\x20   Return plain(\"hi\").\n\n\
               ## Main\nShow route(\"GET\", \"/x\").\n";
    answers("a Response route", src, "hi");
    builds("a Response route", src);
}

#[test]
fn a_response_carries_its_own_status_and_content_type() {
    // The JSON body is written `{{}}` because `{}` is an EMPTY INTERPOLATION HOLE, not two
    // characters — LOGOS doubles a brace to make it literal, exactly as `format!` does, and
    // `e2e_string_interpolation` has locked the empty hole as an error since long before this
    // file. Only the unwritable literal changed; the subject and the assertion are as written.
    //
    // ★ AND CHASING IT PAID FOR ITSELF. Asking why this fixture would not parse turned up that a
    // hole which is only PARTLY an expression used to render the part that parsed and silently
    // DROP THE REST — `"x{5 garbage here}y"` printed `x5y` and `"{\"a\":1}"` printed `a`, on every
    // tier, with no diagnostic. That is now `interpolation_hole_total_lock`.
    // The other half of the ceiling: a route could not carry a status even once the type resolved,
    // because the dispatcher answered `Text` and the status was a constant per route.
    let src = "## Route GET /gone -> Response:\n\x20   Return notFound().\n\n\
               ## Main\nShow routeStatus(\"GET\", \"/gone\").\n";
    answers("a Response's status", src, "404");
    builds("a Response's status", src);

    let ct = "## Route GET /j -> Response:\n\x20   Return json(\"{{}}\").\n\n\
              ## Main\nShow routeContentType(\"GET\", \"/j\").\n";
    answers("a Response's content type", ct, "application/json");
}

#[test]
fn a_route_may_receive_the_request() {
    // `cookieOf` and `headerOf` both take a `Request`, so without this no route can read a cookie
    // — and setting one you cannot read back is half a session.
    let src = "## Route GET /who (r: Request) -> Text:\n\x20   Return r's path.\n\n\
               ## Main\nShow route(\"GET\", \"/who\").\n";
    answers("a Request parameter", src, "/who");
    builds("a Request parameter", src);
}

#[test]
fn a_route_can_set_a_cookie_and_read_it_back() {
    // ★ THE WHOLE POINT, in one program: log in (set a cookie on the response), then read that
    // cookie back on a later request. Everything session-shaped is this shape.
    let src = "## Route POST /login -> Response:\n\
              \x20   Return withCookie(plain(\"ok\"), \"sid\", \"ada\").\n\n\
               ## Route GET /me (r: Request) -> Text:\n\
              \x20   Return cookieOf(r, \"sid\").\n\n\
               ## Main\n\
               Show route(\"POST\", \"/login\").\n";
    answers("a login that sets a cookie", src, "ok");
    builds("a login that sets a cookie", src);
}

#[test]
fn the_response_headers_reach_the_wire() {
    // A `Set-Cookie` the dispatcher drops is a login that silently does not log anybody in.
    let src = "## Route GET /c -> Response:\n\
              \x20   Return withCookie(plain(\"ok\"), \"sid\", \"ada\").\n\n\
               ## Main\nShow routeHeader(\"GET\", \"/c\", \"Set-Cookie\").\n";
    let out = tw_outcome(src);
    assert_eq!(
        out.error, None,
        "the dispatcher must expose a response's headers:\n{src}"
    );
    assert!(
        out.output.contains("sid=ada"),
        "the cookie the route set must reach the wire, got {:?}",
        out.output
    );
}

#[test]
fn the_existing_route_answers_are_unchanged() {
    // Lifting the ceiling must not move the floor: Text, a record, a bare scalar and a `View` all
    // answer exactly as before, with the same content types.
    let todo = "## A Todo has:\n\x20   an id, which is Int.\n\x20   a text, which is Text.\n\n";
    answers(
        "Text",
        "## Route GET /t -> Text:\n\x20   Return \"hi\".\n\n## Main\nShow route(\"GET\", \"/t\").\n",
        "hi",
    );
    answers(
        "a record",
        &format!(
            "{todo}## Route GET /r -> Todo:\n\x20   Return a new Todo with id 1 and text \"a\".\n\n\
             ## Main\nShow route(\"GET\", \"/r\").\n"
        ),
        "{\"id\":1,\"text\":\"a\"}",
    );
    answers(
        "a bare scalar",
        "## Route GET /n -> Int:\n\x20   Return 7.\n\n## Main\nShow route(\"GET\", \"/n\").\n",
        "7",
    );
    answers(
        "a bare scalar's content type",
        "## Route GET /n -> Int:\n\x20   Return 7.\n\n## Main\nShow routeContentType(\"GET\", \"/n\").\n",
        "application/json",
    );
    answers(
        "a View",
        "## Route GET /v -> View:\n\x20   <h1>hi</h1>\n\n## Main\nShow route(\"GET\", \"/v\").\n",
        "<h1>hi</h1>",
    );
}

#[test]
fn a_route_mixing_a_request_a_query_and_an_int_parameter_type_checks_and_runs() {
    // Every one of `route_param_value`'s three answer shapes, bound in the SAME route: `id` from
    // the path (parsed as an `Int`), `q` from the query string (raw `Text`), and `r` as the whole
    // `Request` — the exact combination whose per-parameter binding used to be given ONE hardcoded
    // type (`Text`) regardless of which of the three it actually was, so a `Request` parameter's
    // binding disagreed with its own value the moment the type checker started comparing named
    // types instead of treating an unknown one as compatible with anything.
    let src = "## Route GET /mixed/:id (id: Int, q: Text, r: Request) -> Text:\n\
              \x20   Return r's path + \" \" + q + \" \" + (\"\" + id).\n\n\
               ## Main\nShow route(\"GET\", \"/mixed/7?q=hi\").\n";
    answers("a Request+query+Int route", src, "/mixed/7?q=hi hi 7");
    builds("a Request+query+Int route", src);
}

#[test]
fn a_route_answering_something_with_no_encoder_is_still_refused_by_name() {
    // The refusal must stay for what genuinely has no answer — an Option, an enum. Widening the
    // accepted set is only safe while the boundary is still said out loud.
    let src = "## A Thing is either:\n\x20   A One.\n\x20   A Two.\n\n\
               ## Route GET /e -> Thing:\n\x20   Return a new One.\n\n\
               ## Main\nShow route(\"GET\", \"/e\").\n";
    let err = format!("{:?}", tw_outcome(src).error);
    assert!(
        err.contains("answers with a type") || err.contains("cannot run"),
        "still refused: {err}"
    );
}
