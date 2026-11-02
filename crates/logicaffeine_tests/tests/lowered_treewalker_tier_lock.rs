//! ★ LOCK — the tree-walker runs the page's LOWERED program: every page the suite and the examples
//! know runs on it. ★
//!
//! A browser runs a page's lowered LOGOS. The tree-walker runs the very same program: one
//! interpreter runs the page's main (its lowered prelude and definitions) and then the test's own
//! statements, and every statement that acts on the page is carried out by the driver every lowered
//! tier shares — `Mount` calls `__mount_<C>`, `Click` hands its event to `__lgx_dispatch_node`, and
//! so on — with that same interpreter as the engine it calls into (`PageTierKind::LoweredHere`).
//! This tier is the reference every other page tier is held to (`lowered_vm_tier_lock`,
//! `browser_tier_lock`), so a tier that refused the corpus would hold every other one to nothing.
//!
//! ★ THE PAGES ARE DISCOVERED, NOT LISTED (`support/page_corpus.rs`, the same corpus the lowered VM
//! lock walks).

#[path = "support/page_corpus.rs"]
mod page_corpus;
#[path = "support/rust_literals.rs"]
mod rust_literals;

use logicaffeine_compile::page_host::{compare_tests, PageTierKind, TestTierComparison, TierRun};
use page_corpus::corpus;

const BUDGET: u64 = 50_000_000;

/// Run `src`'s page tests on the lowered tree-walker.
fn compare(src: &str) -> Result<Vec<TestTierComparison>, String> {
    compare_tests(src, &[PageTierKind::LoweredHere], 0, BUDGET, None)
}

#[test]
fn the_corpus_runs_on_the_lowered_tree_walker() {
    // A tier that refused everything would make the ledger lock above vacuously green.
    let pages = corpus();
    let mut refused = Vec::new();
    let mut scenarios = 0;
    for page in &pages {
        let Ok(all) = compare(&page.source) else {
            continue;
        };
        for c in all {
            scenarios += 1;
            if let Some((_, TierRun::Refused { statement, reason })) = c
                .candidates
                .iter()
                .find(|(k, _)| *k == PageTierKind::LoweredHere)
            {
                refused.push(format!("{}::{} @ {statement}: {reason}", page.id, c.test));
            }
        }
    }
    assert!(
        scenarios >= 30,
        "the corpus runs {scenarios} scenarios — fewer than it held when written"
    );
    assert!(
        refused.len() * 10 <= scenarios,
        "the lowered tree-walker refused {} of {scenarios} scenarios:\n{}",
        refused.len(),
        refused.join("\n")
    );
}

/// A computed operand is the interpreter's to evaluate — and on this tier the interpreter that
/// evaluates it is the one driving the page, so `Click "#" + id.` drives rather than refuses (the
/// lowered VM, driven from outside, refuses a computed operand).
#[test]
fn a_computed_operand_drives_on_this_tier() {
    let src = "\
## Component App
State:
    Let n be 0.
Actions:
    To bump:
        Set n to n + 1.
View:
    <p id=\"n\">{n}</p><button id=\"go\" on-click={bump}>+</button>

## Test: a computed selector clicks
    Mount App.
    Let which be \"go\".
    Click \"#\" + which.
    Expect that the text of \"#n\" is equal to \"1\".
";
    let all = compare_tests(src, &[PageTierKind::LoweredHere], 0, BUDGET, None).expect("parses");
    assert_eq!(all.len(), 1);
    let trace = match all[0]
        .candidates
        .iter()
        .find(|(k, _)| *k == PageTierKind::LoweredHere)
    {
        Some((_, TierRun::Traced(t))) => t.clone(),
        Some((_, other)) => {
            panic!("the lowered tree-walker must drive a computed operand: {other:?}")
        }
        None => all[0].reference.clone(),
    };
    assert_eq!(
        trace.stopped, None,
        "the scenario runs to the end: {:?}",
        trace.stopped
    );
    let doc = &trace.steps.last().unwrap().snapshot.document;
    assert!(
        doc.contains("<p id=\"n\">1</p>"),
        "the computed click landed: {doc}"
    );
}

/// ★ A DEEP COMPONENT TREE, ON THE THREAD THE CALLER HAPPENS TO HAVE. On this tier a component's
/// render is LOGOS — `__render_Tree` calling the child's `__render_Tree`, each level several
/// interpreter frames. The scenario runs on the tree-walker's own session stack, so an ordinary
/// 2 MiB caller thread draws a tree just inside the render-depth budget.
#[test]
fn a_deep_component_tree_renders_on_the_lowered_tree_walker() {
    const DEPTH: usize = 30;
    let src = format!(
        "\
## Component Tree (depth: Int)
View:
    <li>{{depth}}{{if depth is greater than 0:}}<ul><Tree depth={{depth - 1}}/></ul>{{end}}</li>

## Component App
View:
    <ul><Tree depth={{{DEPTH}}}/></ul>

## Test: the whole tree is drawn
    Mount App.
    Expect that the text of \"li\" is not equal to \"\".
"
    );
    let trace = std::thread::Builder::new()
        .stack_size(2 * 1024 * 1024)
        .spawn(move || {
            let all =
                compare_tests(&src, &[PageTierKind::LoweredHere], 0, BUDGET, None).expect("parses");
            match all[0]
                .candidates
                .iter()
                .find(|(k, _)| *k == PageTierKind::LoweredHere)
            {
                Some((_, TierRun::Traced(t))) => t.clone(),
                Some((_, other)) => panic!("the lowered tree-walker must draw the tree: {other:?}"),
                None => all[0].reference.clone(),
            }
        })
        .expect("spawn")
        .join()
        .expect("an ordinary caller thread must not overflow");
    assert_eq!(trace.stopped, None, "the tree draws: {:?}", trace.stopped);
    let doc = &trace.steps.last().unwrap().snapshot.document;
    assert_eq!(
        doc.matches("<li>").count(),
        DEPTH + 1,
        "every level is drawn: {doc}"
    );
    assert!(doc.contains("<li>0</li>"), "down to the leaf: {doc}");
}
