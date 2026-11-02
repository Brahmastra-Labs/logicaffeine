//! ★ LOCK — a reload loads the PAGE again; the world outside it stays as the test arranged it. ★
//!
//! A test's `Given that GET "…" answers …` is the server: what answers a request, whoever sends it
//! and whenever. `Reload the page.` is the visitor pressing reload — the program runs again and the
//! page mounts itself again (`page_scene_lock::reloading_a_page_loads_it_again`), and the requests
//! the fresh page sends are answered by the same server. A reload that also forgot the server
//! would leave a page with a load-time request unable to load after a reload, which no browser
//! does: the page would sit on its placeholder, and a test could not tell that from a page that
//! never asks.
//!
//! Each row runs the authoritative verdict (`largo test`'s runner) and the page host's lowered
//! tier, and holds both to the page a browser shows.

use logicaffeine_compile::page_host::{compare_tests, PageTierKind, TierRun};
use logicaffeine_compile::testrun::{run_tests, TestConfig, Verdict};

const BUDGET: u64 = 50_000_000;

const LOADING_PAGE: &str = "\
## A Todo has:
    an id, which is Int.
    a text, which is Text.

## Page /
State:
    Let label be \"...\".
    Let pokes be 0.
Actions:
    When the page loads:
        Get \"/api/todo\" as Todo, then loaded.
    To loaded (r: Result of Todo and HttpError):
        Inspect r:
            When Ok (v):
                Set label to v's text.
            When Err (e):
                Set label to e's message.
    To poke:
        Set pokes to pokes + 1.
View:
    <p id=\"label\">{label}</p><button id=\"poke\" on-click={poke}>poke</button>

";

/// The runner's verdict on the page's one test: it must pass, and its `Expect`s say what the page
/// showed.
fn passes_on_the_runner(src: &str) {
    let report = run_tests(
        src,
        &TestConfig {
            page_tiers: Vec::new(),
            ..TestConfig::default()
        },
    )
    .unwrap_or_else(|e| panic!("the page must parse:\n{src}\n=> {e}"));
    assert_eq!(report.outcomes.len(), 1, "{:#?}", report.outcomes);
    let o = &report.outcomes[0];
    assert_eq!(
        o.verdict,
        Verdict::Passed,
        "`{}` on the runner: {:#?}\n{src}",
        o.name,
        o.failures
    );
}

/// The lowered tier's last document for the page's one test.
fn lowered_document(src: &str) -> String {
    let all = compare_tests(src, &[PageTierKind::LoweredVm], 0, BUDGET, None).unwrap();
    assert_eq!(all.len(), 1, "one test");
    let TierRun::Traced(t) = &all[0].candidates[0].1 else {
        panic!(
            "the lowered tier must run the test: {:?}",
            all[0].candidates
        )
    };
    t.steps
        .last()
        .expect("a traced step")
        .snapshot
        .document
        .clone()
}

#[test]
fn a_reload_keeps_the_answers_the_test_arranged() {
    let src = format!(
        "{LOADING_PAGE}## Test: the reloaded page asks the same server\n\
         Given that GET \"/api/todo\" answers 200 with \"{{{{\\\"id\\\": 1, \\\"text\\\": \\\"milk\\\"}}}}\".\n\
         Expect that the text of \"#label\" is equal to \"milk\".\n\
         Reload the page.\n\
         Expect that the text of \"#label\" is equal to \"milk\".\n"
    );
    passes_on_the_runner(&src);
    let doc = lowered_document(&src);
    assert!(
        doc.contains(">milk<"),
        "the lowered tier's reloaded page must show the answer too:\n{doc}"
    );
}

#[test]
fn an_answer_arranged_after_the_first_load_answers_the_reload() {
    let src = format!(
        "{LOADING_PAGE}## Test: the server changes, then the visitor reloads\n\
         Click \"#poke\".\n\
         Expect that the text of \"#label\" is equal to \"...\".\n\
         Given that GET \"/api/todo\" answers 200 with \"{{{{\\\"id\\\": 1, \\\"text\\\": \\\"bread\\\"}}}}\".\n\
         Reload the page.\n\
         Expect that the text of \"#label\" is equal to \"bread\".\n"
    );
    passes_on_the_runner(&src);
    let doc = lowered_document(&src);
    assert!(
        doc.contains(">bread<"),
        "the lowered tier's reloaded page must show the answer too:\n{doc}"
    );
}

/// The other direction: a reload is a fresh PAGE, so what the page held in its own state is gone
/// and what it shows comes from asking again.
#[test]
fn a_reload_forgets_the_pages_own_state() {
    let src = format!(
        "{LOADING_PAGE}## Test: the reloaded page starts over\n\
         Given that GET \"/api/todo\" answers 503 with \"busy\".\n\
         Expect that the text of \"#label\" is equal to \"503: busy\".\n\
         Given that GET \"/api/todo\" answers 200 with \"{{{{\\\"id\\\": 1, \\\"text\\\": \\\"eggs\\\"}}}}\".\n\
         Reload the page.\n\
         Expect that the text of \"#label\" is equal to \"eggs\".\n"
    );
    passes_on_the_runner(&src);
    let doc = lowered_document(&src);
    assert!(
        doc.contains(">eggs<"),
        "the lowered tier's reloaded page must show the new answer:\n{doc}"
    );
}
