//! ★ LOCK — `Reload the page.` is the page loading again, on every in-process tier: a fresh
//! document, the page's main run again, and the page it mounts by itself mounted again — once. ★
//!
//! A browser reload re-runs the page's module: its globals start over and a `## Page` mounts itself,
//! exactly as it did the first time. One driver carries that out for every in-process tier
//! (`page_host`'s `Drive::Reload`), so nothing is placed after a reload by whoever runs the program
//! or the test. These rows hold each in-process tier to that: the page is back, fresh, and there is ONE of it — a
//! second mount placed after the reload would draw the page twice.

use logicaffeine_compile::compile::{tw_outcome, vm_outcome};
use logicaffeine_compile::page_host::{compare_tests, PageTierKind, TierRun};
use logicaffeine_compile::semantics::dom;

const BUDGET: u64 = 50_000_000;

const PAGE: &str = "\
## Page /
State:
    Let n be 0.
Actions:
    To bump:
        Set n to n + 1.
View:
    <p id=\"n\">{n}</p>
    <button id=\"b\" on-click={bump}>b</button>

";

/// The page's markup the run left in the in-process document.
fn document_after(run: impl FnOnce() -> Option<String>) -> String {
    dom::reset();
    if let Some(error) = run() {
        panic!("the program must run cleanly: {error}");
    }
    dom::render()
}

fn assert_one_fresh_page(tier: &str, doc: &str) {
    assert_eq!(
        doc.matches("id=\"n\"").count(),
        1,
        "{tier}: the reloaded page is drawn exactly once: {doc}"
    );
    assert!(
        doc.contains("<p id=\"n\">0</p>"),
        "{tier}: the reloaded page starts over: {doc}"
    );
}

#[test]
fn a_main_scripts_reload_mounts_its_page_again_on_the_tree_walker() {
    let src = format!("{PAGE}## Main\nClick \"#b\".\nReload the page.\n");
    let doc = document_after(|| tw_outcome(&src).error);
    assert_one_fresh_page("tree-walker", &doc);
}

#[test]
fn a_main_scripts_reload_mounts_its_page_again_on_the_vm() {
    let src = format!("{PAGE}## Main\nClick \"#b\".\nReload the page.\n");
    let doc = document_after(|| vm_outcome(&src).error);
    assert_one_fresh_page("bytecode VM", &doc);
}

#[test]
fn a_main_scripts_page_is_live_again_after_a_reload_on_every_in_process_tier() {
    let src =
        format!("{PAGE}## Main\nClick \"#b\".\nReload the page.\nClick \"#b\".\nClick \"#b\".\n");
    for (tier, doc) in [
        ("tree-walker", document_after(|| tw_outcome(&src).error)),
        ("bytecode VM", document_after(|| vm_outcome(&src).error)),
    ] {
        assert_eq!(
            doc.matches("id=\"n\"").count(),
            1,
            "{tier}: one page: {doc}"
        );
        assert!(
            doc.contains("<p id=\"n\">2</p>"),
            "{tier}: the page mounted by the reload answers clicks, from a fresh count: {doc}"
        );
    }
}

#[test]
fn a_tests_reload_mounts_the_page_once_on_every_page_tier() {
    let src = format!(
        "{PAGE}## Test: reload\nClick \"#b\".\nReload the page.\nExpect that the text of \"#n\" is equal to \"0\".\nClick \"#b\".\nExpect that the text of \"#n\" is equal to \"1\".\n"
    );
    let all = compare_tests(
        &src,
        &[PageTierKind::LoweredHere, PageTierKind::LoweredVm],
        0,
        BUDGET,
        None,
    )
    .expect("the page parses");
    assert_eq!(all.len(), 1);
    let c = &all[0];
    let mut traces = vec![("reference", c.reference.clone())];
    for (kind, run) in &c.candidates {
        match run {
            TierRun::Traced(t) => traces.push((kind.label(), t.clone())),
            TierRun::Refused { statement, reason } => {
                panic!(
                    "{} refused the reload scenario at {statement}: {reason}",
                    kind.label()
                )
            }
        }
    }
    assert!(
        traces.len() >= 3,
        "the reference and both lowered tiers ran: {:?}",
        c.candidates
    );
    for (tier, trace) in traces {
        assert_eq!(trace.stopped, None, "{tier}: runs to the end");
        let after_reload = &trace.steps[2].snapshot.document;
        assert_one_fresh_page(tier, after_reload);
        let last = &trace.steps.last().unwrap().snapshot.document;
        assert!(
            last.contains("<p id=\"n\">1</p>"),
            "{tier}: the reloaded page answers: {last}"
        );
        assert_eq!(
            last.matches("id=\"n\"").count(),
            1,
            "{tier}: still one page: {last}"
        );
    }
}
