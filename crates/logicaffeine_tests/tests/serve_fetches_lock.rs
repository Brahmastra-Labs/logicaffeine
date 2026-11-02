//! ★ LOCK — `Serve fetches with routes.` wires a page to its own server. ★
//!
//! The full-stack loop, with no network and no stubs: the page's `Get "/api/todos"` is answered
//! by the program's OWN `## Route GET /api/todos`, through the derived encoder on the way out and
//! the derived decoder on the way back. That round trip is the point — it is the only test that
//! can catch the two halves of the wire contract drifting apart, because a stub answers with
//! whatever the test author typed rather than with what the server would really send.

use logicaffeine_compile::compile::tw_outcome;
use logicaffeine_compile::semantics::dom;

fn output(src: &str) -> Vec<String> {
    dom::reset();
    let out = tw_outcome(src);
    assert_eq!(out.error, None, "the program must run cleanly:\n{src}");
    out.output.lines().map(str::to_string).collect()
}

/// A server and a page in one file, the way a `largo` project's modules compose.
const APP: &str = "\
## A Todo has:
    an id, which is Int.
    a text, which is Text.

## Route GET /api/todo -> Todo:
    Return a new Todo with id 7 and text \"from the server\".

## Component App
State:
    Let label be \"...\".
    Let problem be \"\".
Actions:
    When the page loads:
        Get \"/api/todo\" as Todo, then loaded.
    To loaded (r: Result of Todo and HttpError):
        Inspect r:
            When Ok (v):
                Set label to v's text.
            When Err (e):
                Set problem to e's message.
View:
    <p id=\"label\">{label}</p>
    <p id=\"problem\">{problem}</p>

";

#[test]
fn a_page_reads_its_own_servers_answer() {
    let src = format!(
        "{APP}## Main\nServe fetches with routes.\nMount App.\n\
        Show the text of \"#label\".\n"
    );
    assert_eq!(output(&src), ["from the server"]);
}

#[test]
fn the_round_trip_goes_through_both_derived_codecs() {
    // The route ENCODES a Todo and the page DECODES one. Neither side was written by hand, and
    // this is the only test that would notice them drifting apart — a stub answers with what the
    // test author typed, which agrees with itself no matter what the encoder does.
    let src = format!(
        "{APP}## Main\nServe fetches with routes.\nMount App.\n\
        Show the text of \"#label\".\n"
    );
    assert_eq!(output(&src), ["from the server"]);
    let sent = dom::fetch_requests();
    assert_eq!(sent.len(), 1, "one real request went out: {sent:?}");
    assert_eq!(sent[0].url, "/api/todo");
}

#[test]
fn without_the_sentence_nothing_answers_the_page() {
    // `Serve fetches with routes.` is a DECISION a test makes, not a default. A page whose
    // fetches were silently answered by the local route table would pass tests that a deployed
    // page fails, because in a browser nothing local answers anything.
    let src = format!("{APP}## Main\nMount App.\nShow the text of \"#label\".\n");
    assert_eq!(output(&src), ["..."]);
}

#[test]
fn a_stub_still_wins_over_the_route_table() {
    // The specific answer beats the general one, so a test can serve everything from the routes
    // and still pin ONE endpoint to an awkward answer it wants to exercise.
    let src = format!(
        "{APP}## Main\nServe fetches with routes.\n\
         Given that GET \"/api/todo\" answers 200 with {}.\nMount App.\n\
         Show the text of \"#label\".\n",
        {
            let bytes: Vec<String> =
                r#"{"id":1,"text":"stubbed"}"#.bytes().map(|b| b.to_string()).collect();
            format!("text_from_bytes([{}])", bytes.join(", "))
        }
    );
    assert_eq!(output(&src), ["stubbed"]);
}

#[test]
fn a_url_no_route_serves_reaches_the_page_as_an_error() {
    // The router answers `404` for a path it does not know. That text is not a Todo, so the page
    // hears a decode failure rather than silence — which is the honest outcome until the router
    // carries a status alongside its body.
    let src = "## A Todo has:\n    an id, which is Int.\n\n\
        ## Route GET /api/other -> Text:\n    Return \"hi\".\n\n\
        ## Component App\nState:\n    Let problem be \"\".\n\
        Actions:\n    When the page loads:\n        \
        Get \"/api/missing\" as Todo, then loaded.\n    \
        To loaded (r: Result of Todo and HttpError):\n        Inspect r:\n            \
        When Ok (v):\n                Set problem to \"ok\".\n            \
        When Err (e):\n                Set problem to e's message.\n\
        View:\n    <p id=\"problem\">{problem}</p>\n\n\
        ## Main\nServe fetches with routes.\nMount App.\n\
        Show \"problem=\" + the text of \"#problem\".\n";
    let out = output(src);
    assert!(out[0].starts_with("problem="), "got: {out:?}");
    assert_ne!(out[0], "problem=", "the page was told something went wrong");
    assert_ne!(
        out[0], "problem=ok",
        "and it was NOT told the request succeeded"
    );
}

#[test]
fn a_program_with_no_routes_is_refused_by_name() {
    // `Serve fetches with routes.` in a file that declares none is a mistake worth catching: the
    // page would look wired up and every request would go unanswered.
    let src = "## Component App\nView:\n    <p>hi</p>\n\n\
        ## Main\nServe fetches with routes.\nMount App.\n";
    dom::reset();
    let err = tw_outcome(src)
        .error
        .unwrap_or_else(|| panic!("expected a refusal:\n{src}"));
    assert!(
        err.contains("route"),
        "the refusal names what is missing: {err}"
    );
}

/// With the router carrying a status, a URL nothing serves reaches the page as a REAL 404 rather
/// than as a body that happens not to decode. The page can now tell "the server said no" apart
/// from "the server said something I could not read" — two different bugs that used to look the
/// same from inside a handler.
#[test]
fn a_url_no_route_serves_is_a_404_the_page_can_name() {
    let src = "## A Todo has:\n    an id, which is Int.\n\n\
        ## Route GET /api/other -> Text:\n    Return \"hi\".\n\n\
        ## Component App\nState:\n    Let problem be \"\".\n\
        Actions:\n    When the page loads:\n        \
        Get \"/api/missing\" as Todo, then loaded.\n    \
        To loaded (r: Result of Todo and HttpError):\n        Inspect r:\n            \
        When Ok (v):\n                Set problem to \"ok\".\n            \
        When Err (e):\n                Set problem to \"status \" + e's status.\n\
        View:\n    <p id=\"problem\">{problem}</p>\n\n\
        ## Main\nServe fetches with routes.\nMount App.\n\
        Show the text of \"#problem\".\n";
    assert_eq!(output(src), ["status 404"]);
}

#[test]
fn a_route_under_the_wrong_method_is_a_405_the_page_can_name() {
    let src = "## A Todo has:\n    an id, which is Int.\n\n\
        ## Route POST /api/todo -> Text:\n    Return \"hi\".\n\n\
        ## Component App\nState:\n    Let problem be \"\".\n\
        Actions:\n    When the page loads:\n        \
        Get \"/api/todo\" as Todo, then loaded.\n    \
        To loaded (r: Result of Todo and HttpError):\n        Inspect r:\n            \
        When Ok (v):\n                Set problem to \"ok\".\n            \
        When Err (e):\n                Set problem to \"status \" + e's status.\n\
        View:\n    <p id=\"problem\">{problem}</p>\n\n\
        ## Main\nServe fetches with routes.\nMount App.\n\
        Show the text of \"#problem\".\n";
    assert_eq!(output(src), ["status 405"]);
}
