//! ★ LOCK — `Get URL as T, then action.` runs, and a test can answer it. ★
//!
//! The sentence at the centre of every page that talks to a server. It was in the AST and
//! REFUSED ON EVERY TIER, including the reference one — so no LOGOS page could load data, and
//! the plan's own headline example was a sentence nothing could run.
//!
//! A fetch is answered by the derived decoder (`fromJson_<T>`), so the wire format a route
//! writes and the wire format a page reads are ONE contract. The answer is a
//! `Result of T and HttpError`: a server that says 404, a body that will not decode, and a
//! request that never connected are all NORMAL, and a page must be able to say what it will do
//! about each rather than trap.
//!
//! `Given that METHOD URL answers STATUS with BODY.` is how a test answers one without a
//! network — the sealed-world rule the rest of the suite already follows.

use logicaffeine_compile::compile::tw_outcome;
use logicaffeine_compile::semantics::dom;

fn output(src: &str) -> Vec<String> {
    dom::reset();
    let out = tw_outcome(src);
    assert_eq!(out.error, None, "the program must run cleanly:\n{src}");
    out.output.lines().map(str::to_string).collect()
}

fn failure(src: &str) -> String {
    dom::reset();
    tw_outcome(src)
        .error
        .unwrap_or_else(|| panic!("expected a failure:\n{src}"))
}

/// A JSON literal as LOGOS source — `{` opens an interpolation hole, so the bytes are spelled.
fn lg_text(s: &str) -> String {
    let bytes: Vec<String> = s.bytes().map(|b| b.to_string()).collect();
    format!("text_from_bytes([{}])", bytes.join(", "))
}

const PAGE: &str = "\
## A Todo has:
    an id, which is Int.
    a text, which is Text.

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
fn a_page_loads_its_data_and_shows_it() {
    let src = format!(
        "{PAGE}## Main\nGiven that GET \"/api/todo\" answers 200 with {}.\nMount App.\n\
         Show the text of \"#label\".\n",
        lg_text(r#"{"id":1,"text":"milk"}"#)
    );
    assert_eq!(output(&src), ["milk"]);
}

#[test]
fn a_server_error_reaches_the_page_as_an_error_and_not_a_trap() {
    // 404 is an ANSWER. A page whose fetch trapped on one would die on a URL a user typed.
    let src = format!(
        "{PAGE}## Main\nGiven that GET \"/api/todo\" answers 404 with \"missing\".\nMount App.\n\
         Show the text of \"#problem\".\nShow the text of \"#label\".\n"
    );
    let out = output(&src);
    assert!(out[0].contains("404"), "the page hears the status: {out:?}");
    assert_eq!(out[1], "...", "and the value it never got is untouched");
}

#[test]
fn a_body_that_will_not_decode_is_an_error_and_not_a_half_built_record() {
    // The server answered, so the status is 200 — but the body is not a Todo, and handing the
    // page a Todo with an id nobody sent would be the silent wrong answer.
    let src = format!(
        "{PAGE}## Main\nGiven that GET \"/api/todo\" answers 200 with \"not json\".\nMount App.\n\
         Show the text of \"#problem\".\nShow the text of \"#label\".\n"
    );
    let out = output(&src);
    assert!(
        !out[0].is_empty(),
        "the decode failure reaches the page: {out:?}"
    );
    assert_eq!(out[1], "...", "and no value was invented");
}

#[test]
fn a_fetch_nobody_answered_leaves_the_page_as_it_was() {
    // A real fetch is asynchronous and may simply not have come back. That is not an error, and
    // the page must show its initial state rather than a blank or a trap.
    let src = format!("{PAGE}## Main\nMount App.\nShow the text of \"#label\".\n");
    assert_eq!(output(&src), ["..."]);
}

#[test]
fn the_stub_is_matched_by_method_and_url_together() {
    // A stub for POST must not answer a GET — that is how a test ends up proving the wrong
    // request was sent.
    let src = format!(
        "{PAGE}## Main\nGiven that POST \"/api/todo\" answers 200 with {}.\nMount App.\n\
         Show the text of \"#label\".\n",
        lg_text(r#"{"id":1,"text":"milk"}"#)
    );
    assert_eq!(output(&src), ["..."], "the GET went unanswered");
}

#[test]
fn a_post_sends_its_body_and_reads_the_answer() {
    let src = format!(
        "## A Todo has:\n    an id, which is Int.\n    a text, which is Text.\n\n\
         ## Component App\nState:\n    Let label be \"...\".\n\
         Actions:\n    When the page loads:\n        \
         Post \"milk\" to \"/api/todos\" as Todo, then loaded.\n    \
         To loaded (r: Result of Todo and HttpError):\n        Inspect r:\n            \
         When Ok (v):\n                Set label to v's text.\n            \
         When Err (e):\n                Set label to e's message.\n\
         View:\n    <p id=\"label\">{{label}}</p>\n\n\
         ## Main\nGiven that POST \"/api/todos\" answers 200 with {}.\nMount App.\n\
         Show the text of \"#label\".\n",
        lg_text(r#"{"id":2,"text":"bought"}"#)
    );
    assert_eq!(output(&src), ["bought"]);
}

#[test]
fn the_request_actually_went_out_with_its_method_and_url() {
    // The page's side of the contract, asserted directly rather than inferred from the answer.
    let src = format!(
        "{PAGE}## Main\nGiven that GET \"/api/todo\" answers 200 with {}.\nMount App.\n",
        lg_text(r#"{"id":1,"text":"milk"}"#)
    );
    dom::reset();
    assert_eq!(tw_outcome(&src).error, None);
    let sent = dom::fetch_requests();
    assert_eq!(sent.len(), 1, "exactly one request: {sent:?}");
    assert_eq!(sent[0].method, "GET");
    assert_eq!(sent[0].url, "/api/todo");
}

#[test]
fn a_type_the_decoder_cannot_read_is_refused_by_name() {
    // The honesty rule. A page whose load silently never happened is the worst outcome for
    // whoever wrote it, so a shape the derive cannot read is refused with that shape in the
    // message. `Map of Text to Todo` is one; `Seq of Todo` USED to be, and the fixture moved
    // when that gap closed rather than being deleted — a test standing on a gap tests nothing
    // once the gap is gone.
    let src = "## A Todo has:\n    an id, which is Int.\n\n\
        ## Component App\nActions:\n    When the page loads:\n        \
        Get \"/api/todos\" as Map of Text to Todo, then loaded.\n    \
        To loaded (r: Result of Todo and HttpError):\n        Show \"x\".\n\
        View:\n    <p>hi</p>\n\n## Main\nMount App.\n";
    let err = failure(src);
    assert!(
        err.contains("Map"),
        "the refusal names the type it cannot read: {err}"
    );
}

/// `App`, loading `/api/todos` as `ty` into an action that takes it.
fn loading_as(ty: &str) -> String {
    format!(
        "## A Todo has:\n    an id, which is Int.\n\n\
         ## Component App\nActions:\n    When the page loads:\n        \
         Get \"/api/todos\" as {ty}, then loaded.\n    \
         To loaded (r: Result of {ty} and HttpError):\n        Show \"x\".\n\
         View:\n    <p>hi</p>\n\n## Main\nMount App.\n"
    )
}

/// The lowered page's gate refuses a type the codec cannot read in one sentence naming it —
/// the same question the fetch lowering asks to choose its decoder, so a page the gate admits
/// always has its mount, and one it refuses is refused for this and nothing vaguer.
#[test]
fn the_lowered_gate_refuses_an_unreadable_fetch_by_its_type() {
    for (ty, named) in [
        ("Map of Text to Todo", "Map of Text and Todo"),
        ("Seq of Seq of Int", "Seq of Seq of Int"),
        ("Real", "Real"),
    ] {
        let err = format!(
            "{:?}",
            logicaffeine_compile::compile::compile_to_program(&loading_as(ty)).err()
        );
        assert!(
            err.contains(&format!(
                "`as {named}` is not a type this fetch can read yet"
            )),
            "`as {ty}` is refused by its type: {err}"
        );
    }
}

#[test]
fn every_type_the_codec_reads_lowers_with_its_mount() {
    for ty in [
        "Todo",
        "Seq of Todo",
        "Seq of Int",
        "Seq of Text",
        "Text",
        "Int",
        "Bool",
    ] {
        let built = logicaffeine_compile::compile::compile_to_program(&loading_as(ty));
        assert!(
            built.is_ok(),
            "`as {ty}` lowers, mount and all: {:?}",
            built.err()
        );
    }
}

#[test]
fn a_fetch_updates_the_page_through_a_real_turn() {
    // The completion is a TURN: derived fields recompute and the page repaints, exactly as for a
    // click. A fetch that wrote state without a turn would leave the page showing stale data.
    let src = format!(
        "## A Todo has:\n    an id, which is Int.\n    a text, which is Text.\n\n\
         ## Component App\nState:\n    Let label be \"...\".\n    \
         Let shout always be label + \"!\".\n\
         Actions:\n    When the page loads:\n        \
         Get \"/api/todo\" as Todo, then loaded.\n    \
         To loaded (r: Result of Todo and HttpError):\n        Inspect r:\n            \
         When Ok (v):\n                Set label to v's text.\n            \
         When Err (e):\n                Set label to \"err\".\n\
         View:\n    <p id=\"s\">{{shout}}</p>\n\n\
         ## Main\nGiven that GET \"/api/todo\" answers 200 with {}.\nMount App.\n\
         Show the text of \"#s\".\n",
        lg_text(r#"{"id":1,"text":"milk"}"#)
    );
    assert_eq!(
        output(&src),
        ["milk!"],
        "the derived field recomputed and the page repainted"
    );
}

#[test]
fn the_journal_records_the_completion_as_its_own_turn() {
    let src = format!(
        "{PAGE}## Main\nGiven that GET \"/api/todo\" answers 200 with {}.\nMount App.\n\
         Show the journal.\n",
        lg_text(r#"{"id":1,"text":"milk"}"#)
    );
    let out = output(&src);
    assert!(
        out[0].contains("loaded"),
        "the completion is a journalled turn: {out:?}"
    );
}

// ---- `Seq of T`: the shape the plan's own headline example asks for. ----

const LIST_PAGE: &str = "\
## A Todo has:
    an id, which is Int.
    a text, which is Text.

## Component App
State:
    Let todos be a new Seq of Todo.
    Let problem be \"\".
Actions:
    When the page loads:
        Get \"/api/todos\" as Seq of Todo, then loaded.
    To loaded (r: Result of Seq of Todo and HttpError):
        Inspect r:
            When Ok (xs):
                Set todos to xs.
            When Err (e):
                Set problem to e's message.
View:
    <ul>{for each t in todos:}<li class=\"row\">{t's text}</li>{end}</ul>
    <p id=\"problem\">{problem}</p>

";

#[test]
fn a_list_page_loads_its_rows() {
    let src = format!(
        "{LIST_PAGE}## Main\nGiven that GET \"/api/todos\" answers 200 with {}.\nMount App.\n\
         Show the count of \".row\".\nShow the text of \".row\".\n",
        lg_text(r#"[{"id":1,"text":"milk"},{"id":2,"text":"eggs"}]"#)
    );
    let out = output(&src);
    assert_eq!(out[0], "2", "both rows rendered");
    assert_eq!(out[1], "milk", "and the first one reads its own field");
}

#[test]
fn an_empty_list_renders_an_empty_page_rather_than_failing() {
    let src = format!(
        "{LIST_PAGE}## Main\nGiven that GET \"/api/todos\" answers 200 with {}.\nMount App.\n\
         Show the count of \".row\".\nShow \"problem=\" + the text of \"#problem\".\n",
        lg_text("[]")
    );
    // `Show ""` emits NO LINE on either tier, so the empty answer is bracketed rather than
    // compared to nothing — a bare assertion here would pass even if the page had errored.
    assert_eq!(
        output(&src),
        ["0", "problem="],
        "no rows, and no error either"
    );
}

#[test]
fn one_bad_row_fails_the_whole_load_rather_than_dropping_it() {
    // All-or-nothing at the page level: showing three rows when the server sent four, silently,
    // is worse than showing an error.
    let src = format!(
        "{LIST_PAGE}## Main\nGiven that GET \"/api/todos\" answers 200 with {}.\nMount App.\n\
         Show the count of \".row\".\n",
        lg_text(r#"[{"id":1,"text":"milk"},{"id":2}]"#)
    );
    assert_eq!(
        output(&src),
        ["0"],
        "nothing was rendered from a broken answer"
    );
}

// ---- A scalar answer: `as Int`, `as Bool`, beside the record and the list. ----

/// A page loading `/api/value` as `ty` on a click, showing what it decoded or why it could not,
/// with one `## Test` answering `answer` and expecting `shown` (or a problem mentioning `problem`).
fn scalar_page(ty: &str, answer: &str, expect: &str) -> String {
    format!(
        "## Page /\n\nState:\n    Let shown be \"...\".\n    Let problem be \"\".\n\n\
         Actions:\n    To load:\n        Get \"/api/value\" as {ty}, then loaded.\n    \
         To loaded (r: Result of {ty} and HttpError):\n        Inspect r:\n            \
         When Ok (v):\n                Set shown to \"got \" + v.\n            \
         When Err (e):\n                Set problem to e's message.\n\n\
         View:\n    <button id=\"load\" on-click={{load}}>load</button>\n    \
         <p id=\"shown\">{{shown}}</p>\n    <p id=\"problem\">{{problem}}</p>\n\n\
         ## Test: the answer decodes\n\nGiven that GET \"/api/value\" answers 200 with {answer}.\n\
         Click \"#load\".\n{expect}\n"
    )
}

/// `page`'s one `## Test` passes on the reference, runs step for step the same lowered on the
/// tree-walker and on the VM, and the page builds for the browser.
fn decodes_on_every_tier(page: &str) {
    use logicaffeine_compile::page_host::{compare_tests, PageTierKind, TierRun};
    use logicaffeine_compile::testrun::{run_tests, TestConfig, Verdict};
    let report = run_tests(page, &TestConfig::default())
        .unwrap_or_else(|e| panic!("the page parses: {e}\n{page}"));
    for outcome in &report.outcomes {
        assert_eq!(
            outcome.verdict,
            Verdict::Passed,
            "`{}` passes on the reference: {:?}\n{page}",
            outcome.name,
            outcome.failures
        );
    }
    let compared = compare_tests(
        page,
        &[PageTierKind::LoweredHere, PageTierKind::LoweredVm],
        0,
        50_000_000,
        None,
    )
    .unwrap_or_else(|e| panic!("the page parses: {e}\n{page}"));
    assert_eq!(compared.len(), 1, "one scenario:\n{page}");
    let test = &compared[0];
    assert!(
        test.reference.stopped.is_none(),
        "the reference runs it: {:?}\n{page}",
        test.reference.stopped
    );
    let failed: Vec<_> = test
        .reference
        .steps
        .iter()
        .filter_map(|s| s.snapshot.failure.as_deref())
        .collect();
    assert!(
        failed.is_empty(),
        "no step fails on the reference: {failed:?}\n{page}"
    );
    for (kind, run) in &test.candidates {
        assert!(
            matches!(run, TierRun::Traced(_)),
            "the {} runs the page lowered: {run:?}",
            kind.label()
        );
        let differs = test.divergence(*kind).expect("a traced run");
        assert!(
            differs.is_empty(),
            "the {} runs as the reference does: {differs:?}",
            kind.label()
        );
    }
    let built = logicaffeine_compile::compile::compile_to_wasm(page);
    assert!(
        built.is_ok(),
        "the page builds for the browser: {:?}",
        built.err()
    );
}

#[test]
fn an_int_answer_decodes_on_every_tier() {
    decodes_on_every_tier(&scalar_page(
        "Int",
        "\"42\"",
        "Expect that the text of \"#shown\" is equal to \"got 42\".",
    ));
}

#[test]
fn a_bool_answer_decodes_on_every_tier() {
    decodes_on_every_tier(&scalar_page(
        "Bool",
        "\"true\"",
        "Expect that the text of \"#shown\" is equal to \"got true\".",
    ));
}

/// A body that is not the scalar asked for is the page's error to act on, named by what it
/// expected — never a trap and never a value made up.
#[test]
fn an_answer_that_is_not_the_scalar_asked_for_is_an_error_on_every_tier() {
    decodes_on_every_tier(&scalar_page(
        "Int",
        "\"[1]\"",
        "Expect that the text of \"#shown\" is equal to \"...\".\n\
         Expect that the text of \"#problem\" is equal to \"Int: expected a JSON integer\".",
    ));
}

/// A list of records answered to a page decodes on every tier — the shape the examples load.
#[test]
fn a_list_of_records_decodes_on_every_tier() {
    let page = format!(
        "## A Todo has:\n    an id, which is Int.\n    a text, which is Text.\n\n\
         ## Page /\n\nState:\n    Let items be a new Seq of Todo.\n\n\
         Actions:\n    To load:\n        Get \"/api/todos\" as Seq of Todo, then loaded.\n    \
         To loaded (r: Result of Seq of Todo and HttpError):\n        Inspect r:\n            \
         When Ok (v):\n                Set items to v.\n            \
         When Err (e):\n                Set items to a new Seq of Todo.\n\n\
         View:\n    <button id=\"load\" on-click={{load}}>load</button>\n    <p id=\"count\">{{length of items}}</p>\n\n\
         ## Test: rows arrive\n\nGiven that GET \"/api/todos\" answers 200 with \"[{{{{\\\"id\\\": 1, \\\"text\\\": \\\"milk\\\"}}}}]\".\n\
         Click \"#load\".\nExpect that the text of \"#count\" is equal to \"1\".\n"
    );
    decodes_on_every_tier(&page);
}

/// A page the lowering refuses — its actions reach each other — is refused by the reference in the
/// lowering's own words: the reference is the lowered page, and no tier runs a page some other way.
/// That `as Int` decodes through the derived function on every tier is
/// `an_int_answer_decodes_on_every_tier`.
#[test]
fn a_page_that_does_not_lower_is_refused_by_the_reference_in_the_lowerings_words() {
    let src = "## Component App\nState:\n    Let shown be \"...\".\n\
        Actions:\n    When the page loads:\n        Get \"/api/value\" as Int, then loaded.\n    \
        To loaded (r: Result of Int and HttpError):\n        Inspect r:\n            \
        When Ok (v):\n                Set shown to \"got \" + v.\n            \
        When Err (e):\n                Set shown to e's message.\n    \
        To ping:\n        Call pong.\n    To pong:\n        Call ping.\n\
        View:\n    <p id=\"shown\">{shown}</p>\n\n\
        ## Main\nGiven that GET \"/api/value\" answers 200 with \"7\".\nMount App.\nShow the text of \"#shown\".\n";
    assert!(
        logicaffeine_compile::compile::compile_to_program(src).is_err(),
        "the page does not lower"
    );
    let refused = failure(src);
    assert!(
        refused.contains("reaches itself through `Call <action>.`"),
        "in the lowering's words: {refused}"
    );
}
