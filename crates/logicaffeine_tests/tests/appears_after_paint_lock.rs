//! LOCK — `When this component appears:` runs once the paint that drew the instance is complete,
//! and the page that paint drew is drawn once.
//!
//! An appears handler is a turn: it reads its instance in, writes it back and repaints. Run from
//! inside the paint still drawing the page, a placed child's repaint (which is the page's, from its
//! root) cleared the mount point and drew a second page beside the one in progress — the reference
//! drew the page twice and the bytecode tier three times, for three rows that merely said hello.
//! So a paint OWES each instance it draws for the first time its appears handlers, and runs them,
//! in the order it drew the instances, once it has finished (after its disappears sweep: a paint's
//! departures are heard before its arrivals). A handler that traps does not swallow the ones after
//! it. `When this component disappears:` reads the props and state the instance left with.
//!
//! Every row runs on the reference, the lowered page on the tree-walker, and the lowered page on
//! the VM (`## Test` scenarios), and the `## Main` rows on the tree-walker and VM outcomes.

use logicaffeine_compile::compile::{tw_outcome, vm_outcome};
use logicaffeine_compile::page_host::{compare_tests, PageTierKind, PageTrace, TierRun};
use logicaffeine_compile::semantics::dom;

/// The one `## Test` in `source` on every page tier: the reference, then each lowered tier.
fn every_tier(source: &str) -> Vec<(&'static str, PageTrace)> {
    let mut all = compare_tests(
        source,
        &[PageTierKind::LoweredHere, PageTierKind::LoweredVm],
        0,
        50_000_000,
        None,
    )
    .unwrap_or_else(|e| panic!("the page must parse: {e}\n{source}"));
    assert_eq!(all.len(), 1, "exactly one page scenario:\n{source}");
    let c = all.remove(0);
    let mut out = vec![("reference", c.reference)];
    for (kind, run) in c.candidates {
        match run {
            TierRun::Traced(t) => out.push((kind.label(), t)),
            other => panic!(
                "the {} tier must trace the scenario, not {other:?}",
                kind.label()
            ),
        }
    }
    out
}

fn once(haystack: &str, needle: &str) -> usize {
    haystack.matches(needle).count()
}

const ROWS: &str = r##"## Component Row (label: Text)
State:
    Let seen be "no".
Actions:
    When this component appears:
        Set seen to "yes".
View:
    <li id="r-{label}">{label}:{seen}</li>

## Page /
State:
    Let open be true.
Actions:
    To toggle:
        Toggle open.
View:
    <main><ul>{for each r in ["a", "b"]:}<Row label="{r}"/>{end}</ul>{if open:}<ul><Row label="z"/></ul>{end}<button id="t" on-click={toggle}>t</button></main>

## Test: rows that appear
    Click "#t".
    Click "#t".
"##;

/// ★ THE PAGE IS DRAWN ONCE, and every row's appears handler has run and been drawn — at mount,
/// after the child leaves, and after it comes back.
#[test]
fn a_page_whose_rows_appear_is_drawn_once_with_every_handler_shown() {
    for (tier, trace) in every_tier(ROWS) {
        assert!(trace.stopped.is_none(), "{tier}: {:?}", trace.stopped);
        for (k, step) in trace.steps.iter().enumerate() {
            let doc = &step.snapshot.document;
            assert!(
                step.snapshot.failure.is_none(),
                "{tier} @{k}: {:?}",
                step.snapshot.failure
            );
            assert_eq!(
                once(doc, "<main>"),
                1,
                "{tier} @{k}: the page is drawn once:\n{doc}"
            );
            assert!(
                doc.contains("a:yes") && doc.contains("b:yes"),
                "{tier} @{k}: every row's handler is shown:\n{doc}"
            );
        }
        let gone = &trace.steps[1].snapshot.document;
        assert!(
            !gone.contains("r-z"),
            "{tier}: the toggled row left:\n{gone}"
        );
        let back = &trace.steps[2].snapshot.document;
        assert!(
            back.contains("z:yes"),
            "{tier}: the row that came back appeared again:\n{back}"
        );
    }
    let built = logicaffeine_compile::compile::compile_to_wasm(ROWS);
    assert!(
        built.is_ok(),
        "the page builds for the browser: {:?}",
        built.err()
    );
}

const ANNOUNCES: &str = "\
## Component Row (label: Text)
Actions:
    When this component appears:
        Show \"in {label}\".
    When this component disappears:
        Show \"out {label}\".
View:
    <li>{label}</li>

## Component App
State:
    Let open be true.
Actions:
    To toggle:
        Toggle open.
View:
    <main><ul>{for each r in [\"a\", \"b\"]:}<Row label=\"{r}\"/>{end}</ul>{if open:}<ul><Row label=\"z\"/></ul>{end}<button id=\"t\" on-click={toggle}>t</button></main>

## Main
Mount App.
Click \"#t\".
Click \"#t\".
";

/// The handlers run in the order the paint drew the instances; a disappears handler reads the
/// props the instance left with; a paint's departures are heard before its arrivals.
#[test]
fn appears_run_in_draw_order_and_disappears_read_what_the_instance_left_with() {
    let expected = ["in a", "in b", "in z", "out z", "in z"];
    dom::reset();
    let tw = tw_outcome(ANNOUNCES);
    assert_eq!(tw.error, None, "the tree-walker runs it");
    assert_eq!(
        tw.output.lines().collect::<Vec<_>>(),
        expected,
        "tree-walker"
    );
    let tw_doc = dom::render();
    assert_eq!(
        once(&tw_doc, "<main>"),
        1,
        "tree-walker draws one page:\n{tw_doc}"
    );
    dom::reset();
    let vm = vm_outcome(ANNOUNCES);
    assert_eq!(vm.error, None, "the VM runs it");
    assert_eq!(vm.output.lines().collect::<Vec<_>>(), expected, "VM");
    assert_eq!(
        dom::render(),
        tw_doc,
        "the VM draws what the tree-walker draws"
    );
}

const DEPARTS_WITH_STATE: &str = "\
## Component Row (label: Text)
State:
    Let n be 0.
Actions:
    To bump:
        Set n to n + 1.
    When this component disappears:
        Show \"{label} left at {n}\".
View:
    <li><button id=\"b-{label}\" on-click={bump}>{label}:{n}</button></li>

## Component App
State:
    Let open be true.
Actions:
    To toggle:
        Toggle open.
View:
    <main>{if open:}<ul><Row label=\"z\"/></ul>{end}<button id=\"t\" on-click={toggle}>t</button></main>

## Main
Mount App.
Click \"#b-z\".
Click \"#b-z\".
Click \"#t\".
";

/// `When this component disappears:` reads the state the instance left with, not a fresh record.
#[test]
fn a_departing_instance_reports_the_state_it_left_with() {
    dom::reset();
    let tw = tw_outcome(DEPARTS_WITH_STATE);
    assert_eq!(tw.error, None, "tree-walker");
    assert_eq!(tw.output.trim(), "z left at 2", "tree-walker");
    dom::reset();
    let vm = vm_outcome(DEPARTS_WITH_STATE);
    assert_eq!(vm.error, None, "VM");
    assert_eq!(vm.output.trim(), "z left at 2", "VM");
}

const ONE_TRAPS: &str = r##"## Component Row (label: Text)
State:
    Let seen be "no".
    Let boom be 0.
Actions:
    When this component appears:
        If label is "b":
            Set boom to item 9 of [1].
        Set seen to "yes".
View:
    <li>{label}:{seen}</li>

## Page /
State:
    Let open be false.
Actions:
    To show:
        Set open to true.
View:
    <main>{if open:}<ul>{for each r in ["a", "b", "c"]:}<Row label="{r}"/>{end}</ul>{end}<button id="s" on-click={show}>s</button></main>

## Test: one appears handler traps
    Click "#s".
"##;

/// ★ ONE TRAPPING HANDLER DOES NOT SWALLOW THE REST. The row whose handler trapped keeps its state
/// (its turn rolled back), the page reports the trap, and the rows drawn after it still appear —
/// on the lowered page, the failure door runs what the trapping paint still owed.
#[test]
fn a_trapping_appears_handler_does_not_swallow_the_ones_after_it() {
    for (tier, trace) in every_tier(ONE_TRAPS) {
        assert!(trace.stopped.is_none(), "{tier}: {:?}", trace.stopped);
        let first = &trace.steps[1].snapshot;
        assert!(
            first.failure.is_some(),
            "{tier}: the trap is the page's unhandled failure"
        );
        let doc = &first.document;
        assert!(
            doc.contains("a:yes") && doc.contains("b:no") && doc.contains("c:yes"),
            "{tier}:\n{doc}"
        );
        assert_eq!(once(doc, "<main>"), 1, "{tier}: one page:\n{doc}");
    }
}
