//! ★ LOCK — every run the IDE, a guide, an exercise or the REPL starts paints a document of its own. ★
//!
//! A page's paint names its nodes by the ids its retained tree hands out, from the same start on
//! every run. The web app runs one code block after another in one thread, and the REPL runs its
//! whole program again on every entry, so the document an earlier run painted is still there when the
//! next one starts. Each run is a page load: it starts from an empty document, as a browser's does,
//! and the ids its paint names never meet nodes an earlier run left behind.

use logicaffeine_compile::semantics::dom;

const PAGE: &str = r#"## Component App
State:
    Let n be 2.
View:
    <main><p id="out">Count: {n}</p></main>

## Main
Mount App.
Show "drawn".
"#;

fn runs_clean(
    what: &str,
    run: impl Fn(&str) -> logicaffeine_compile::interpreter::InterpreterResult,
) {
    for round in 1..=3 {
        let result = run(PAGE);
        assert_eq!(
            result.error, None,
            "{what}, run {round}: the page runs again in the same thread"
        );
        assert_eq!(
            result.lines,
            vec!["drawn".to_string()],
            "{what}, run {round}"
        );
        assert_eq!(
            dom::render(),
            "<body><main><p id=\"out\">Count: 2</p></main></body>",
            "{what}, run {round}: the document holds this run's page, once"
        );
    }
}

#[test]
fn the_synchronous_entries_start_each_run_on_an_empty_document() {
    runs_clean(
        "interpret_for_ui_sync",
        logicaffeine_compile::interpret_for_ui_sync,
    );
    runs_clean("interpret_for_ui_baseline_sync_with_args", |s| {
        logicaffeine_compile::interpret_for_ui_baseline_sync_with_args(s, &[])
    });
}

#[test]
fn the_async_entries_start_each_run_on_an_empty_document() {
    runs_clean("interpret_for_ui", |s| {
        futures::executor::block_on(logicaffeine_compile::interpret_for_ui(s))
    });
    runs_clean("interpret_for_ui_baseline", |s| {
        futures::executor::block_on(logicaffeine_compile::interpret_for_ui_baseline(s))
    });
}

#[test]
fn a_run_after_a_different_page_shows_only_its_own() {
    let other = "## Component App\nView:\n    <aside>other</aside>\n\n## Main\nMount App.\n";
    assert_eq!(
        logicaffeine_compile::interpret_for_ui_sync(other).error,
        None
    );
    assert_eq!(
        logicaffeine_compile::interpret_for_ui_sync(PAGE).error,
        None
    );
    assert_eq!(
        dom::render(),
        "<body><main><p id=\"out\">Count: 2</p></main></body>"
    );
}
