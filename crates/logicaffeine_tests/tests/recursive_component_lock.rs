//! ★ LOCK — a component may place itself, and a page that cannot be drawn is an ERROR.
//!
//! ★ THE FRAMEWORK COULD NOT DO RECURSION AT ALL, AND FAILED AT IT BY DYING. A `Tree` whose view
//! contains a `<Tree/>` — a comment thread, a file listing, a nested menu, the shape every real app
//! eventually needs — aborted the process with `fatal runtime error: stack overflow`. Measured
//! BEFORE anything was mounted: merely DECLARING the component killed the compile, because the
//! crash was in a STATIC walk. A `{if depth is greater than 0:}` guard did not help; the walk never
//! reaches the condition.
//!
//! Two separate unguarded recursions had to be closed, and the second only became visible once the
//! first was fixed:
//!
//! 1. `lowerable` ⇄ `nodes_lowerable` — "can this be lowered" asked of a child, which asks it of
//!    the child's children, forever. One answer per component, so a component already on the path
//!    is skipped. Every other refusal still fires, because every other check runs before the
//!    descent.
//! 2. `view_html_lowerable_with` — the same question for the SERVER walk, with the same shape.
//!
//! ★ AND THEN THE RENDER ITSELF, which is a different question: a recursion with NO stopping
//! condition has no finite rendering, and the honest answer is to say so. A DEPTH budget, not a
//! cycle check — recursion is not the fault, endlessness is, and depth is what tells them apart.

use logicaffeine_compile::compile::{tw_outcome, vm_outcome};
use logicaffeine_compile::semantics::dom;

const GUARDED: &str = "\
## Component Tree (depth: Int)
View:
    <li>{depth}{if depth is greater than 0:}<ul><Tree depth={depth - 1}/></ul>{end}</li>

## Component App
View:
    <ul><Tree depth={3}/></ul>

## Main
Mount App.
";

/// ★ THE CAPABILITY. A depth-guarded recursive component renders, nested, on both tiers.
#[test]
fn a_guarded_recursive_component_renders() {
    dom::reset();
    assert_eq!(
        tw_outcome(GUARDED).error,
        None,
        "a guarded recursion must run on the tree-walker"
    );
    let tw = dom::render();
    assert!(
        tw.contains("<li>3<ul><li>2<ul><li>1<ul><li>0</li></ul></li></ul></li></ul></li>"),
        "four levels, each one shallower: {tw}"
    );
    dom::reset();
    assert_eq!(vm_outcome(GUARDED).error, None, "and on the VM");
    assert_eq!(dom::render(), tw, "byte for byte");
}

/// ★ AND DECLARING ONE COSTS NOTHING. The crash was in the static walk, so a recursive component
/// that is never placed used to kill the compile of a page that does not even use it.
#[test]
fn declaring_a_recursive_component_does_not_break_a_page_that_ignores_it() {
    let src = "\
## Component Tree (depth: Int)
View:
    <li>{depth}{if depth is greater than 0:}<ul><Tree depth={depth - 1}/></ul>{end}</li>

## Component App
View:
    <p id=\"p\">no tree here</p>

## Main
Mount App.
";
    dom::reset();
    assert_eq!(
        tw_outcome(src).error,
        None,
        "the page does not place it, so it cannot matter"
    );
    assert!(dom::render().contains("no tree here"));
    dom::reset();
    assert_eq!(vm_outcome(src).error, None, "the VM agrees");
}

/// Two components placing each other — the same cycle one step apart.
#[test]
fn mutually_recursive_components_compile() {
    let src = "\
## Component Branch (depth: Int)
View:
    <li>b{depth}{if depth is greater than 0:}<Leaf depth={depth - 1}/>{end}</li>

## Component Leaf (depth: Int)
View:
    <span>l{depth}{if depth is greater than 0:}<ul><Branch depth={depth - 1}/></ul>{end}</span>

## Component App
View:
    <ul><Branch depth={3}/></ul>

## Main
Mount App.
";
    dom::reset();
    assert_eq!(
        tw_outcome(src).error,
        None,
        "mutual recursion with a guard must run"
    );
    let tw = dom::render();
    assert!(
        tw.contains("b3") && tw.contains("l2") && tw.contains("b1"),
        "it alternates: {tw}"
    );
    dom::reset();
    assert_eq!(vm_outcome(src).error, None);
    assert_eq!(dom::render(), tw);
}

/// ★ AND AN ENDLESS ONE IS REPORTED, NOT SURVIVED BY LUCK. If this test ever ABORTS instead of
/// failing, the budget stopped landing before the native stack — see `RENDER_DEPTH_LIMIT`, whose
/// value is measured rather than chosen for exactly this reason.
#[test]
fn an_endless_recursion_is_reported_rather_than_crashing() {
    let src = "\
## Component Tree
View:
    <li>node<ul><Tree/></ul></li>

## Component App
View:
    <ul><Tree/></ul>

## Main
Mount App.
";
    dom::reset();
    let why = tw_outcome(src)
        .error
        .expect("a recursion with no stopping condition must be reported");
    assert!(
        why.contains("Tree") && why.contains("no finite rendering"),
        "the message names the component and says what is wrong in the author's terms: {why}"
    );
    assert!(
        why.contains("{if depth is greater than 0:}"),
        "and shows the shape that fixes it: {why}"
    );
    dom::reset();
    assert!(
        vm_outcome(src).error.is_some(),
        "the browser tier must report rather than abort too — its own depth guard catches it, with \
         a less specific message, which is recorded as an asymmetry rather than asserted equal"
    );
}

/// The page scenario `source` on both lowered tiers: `(tier, trace)`.
fn lowered_tiers(source: &str) -> Vec<(&'static str, logicaffeine_compile::page_host::PageTrace)> {
    use logicaffeine_compile::page_host::{compare_tests, PageTierKind, TierRun};
    let mut all = compare_tests(
        source,
        &[PageTierKind::LoweredHere, PageTierKind::LoweredVm],
        0,
        50_000_000,
        None,
    )
    .unwrap_or_else(|e| panic!("the page must parse: {e}\n{source}"));
    assert_eq!(all.len(), 1, "exactly one page scenario:\n{source}");
    all.remove(0)
        .candidates
        .into_iter()
        .map(|(kind, run)| match run {
            TierRun::Traced(t) => (kind.label(), t),
            other => panic!(
                "the {} tier must trace the scenario, not {other:?}",
                kind.label()
            ),
        })
        .collect()
}

/// A mount whose paint is refused is the page's unhandled failure, with the paint abandoned and the
/// run going on — as a browser reports it — never a crashed tier.
fn refused(tier: &str, trace: &logicaffeine_compile::page_host::PageTrace, needle: &str) {
    assert!(
        trace.stopped.is_none(),
        "{tier}: the refusal fails the paint, not the run: {:?}",
        trace.stopped
    );
    let last = &trace.steps.last().expect("a traced step").snapshot;
    let failure = last
        .failure
        .as_deref()
        .unwrap_or_else(|| panic!("{tier}: an endless recursion is refused"));
    assert!(failure.contains(needle), "{tier}: {failure}");
    assert_eq!(
        last.document, "<body></body>",
        "{tier}: nothing of the refused paint is drawn"
    );
}

const ENDLESS_SCENARIO: &str = "\
## Component Tree
View:
    <li>node<ul><Tree/></ul></li>

## Component App
View:
    <ul><Tree/></ul>

## Test: an endless recursion
    Mount App.
";

/// ★ THE LOWERED PAGE COUNTS TOO, AND SAYS THE REFERENCE'S SENTENCE. Its render of a component that
/// can be drawn inside itself counts how deep it is and fails one level past `RENDER_DEPTH_LIMIT`
/// with `render_depth_message` — on the tree-walker running the lowered page, on the VM, and in the
/// browser build, where the alternative is a dead tab.
#[test]
fn an_endless_recursion_is_refused_in_the_references_words_on_every_lowered_tier() {
    use logicaffeine_language::ast::component::render_depth_message;
    for (tier, trace) in lowered_tiers(ENDLESS_SCENARIO) {
        refused(tier, &trace, &render_depth_message("Tree"));
    }
    let built = logicaffeine_compile::compile::compile_to_wasm(ENDLESS_SCENARIO);
    assert!(
        built.is_ok(),
        "the page builds for the browser, guard and all: {:?}",
        built.err()
    );
}

/// Two components placing each other with nothing to stop them are one cycle, counted the same way.
#[test]
fn an_endless_mutual_recursion_is_refused_on_every_lowered_tier() {
    let source = "\
## Component Branch
View:
    <li>b<Leaf/></li>

## Component Leaf
View:
    <span>l<ul><Branch/></ul></span>

## Component App
View:
    <ul><Branch/></ul>

## Test: an endless mutual recursion
    Mount App.
";
    for (tier, trace) in lowered_tiers(source) {
        refused(tier, &trace, "no finite rendering");
    }
}

fn nested(depth: usize) -> String {
    format!(
        "\
## Component Tree (depth: Int)
View:
    <li>{{depth}}{{if depth is greater than 0:}}<ul><Tree depth={{depth - 1}}/></ul>{{end}}</li>

## Component App
View:
    <ul><Tree depth={{{depth}}}/></ul>

## Test: a recursion {depth} deep
    Mount App.
"
    )
}

/// ★ THE BUDGET IS THE LIMIT, EXACTLY. A guarded recursion that draws `RENDER_DEPTH_LIMIT` levels
/// renders them all, and one level more is refused. The lowered page counts the renders of
/// components that can be drawn inside themselves — the only nesting with no bound — so `App`
/// around the tree is not a level; the AST reference counts every component, and its boundary sits
/// one level earlier here.
#[test]
fn a_recursion_as_deep_as_the_limit_renders_and_one_level_more_is_refused() {
    use logicaffeine_language::ast::component::{render_depth_message, RENDER_DEPTH_LIMIT};
    for (tier, trace) in lowered_tiers(&nested(RENDER_DEPTH_LIMIT - 1)) {
        assert!(
            trace.stopped.is_none(),
            "{tier}: {RENDER_DEPTH_LIMIT} levels render: {:?}",
            trace.stopped
        );
        assert!(
            trace.steps.last().unwrap().snapshot.failure.is_none(),
            "{tier}: nothing is refused"
        );
        let shown = &trace.steps.last().unwrap().snapshot.document;
        assert!(
            shown.contains("<li>0</li>"),
            "{tier}: the deepest level is drawn:\n{shown}"
        );
        assert!(
            shown.contains(&format!("<li>{}<ul>", RENDER_DEPTH_LIMIT - 1)),
            "{tier}: from the top:\n{shown}"
        );
    }
    for (tier, trace) in lowered_tiers(&nested(RENDER_DEPTH_LIMIT)) {
        refused(tier, &trace, &render_depth_message("Tree"));
    }
}

/// A guarded recursion draws the same tree on the lowered tiers as on the reference.
#[test]
fn a_guarded_recursion_renders_on_every_lowered_tier() {
    for (tier, trace) in lowered_tiers(&nested(3)) {
        assert!(trace.stopped.is_none(), "{tier}: {:?}", trace.stopped);
        let shown = &trace.steps.last().unwrap().snapshot.document;
        assert!(
            shown.contains("<li>3<ul><li>2<ul><li>1<ul><li>0</li></ul></li></ul></li></ul></li>"),
            "{tier}: four levels, each one shallower:\n{shown}"
        );
    }
}
