//! ★ LOCK — a program run draws into the document its caller holds, whatever an earlier run drew. ★
//!
//! A page's paint names its nodes by stream ids its retained tree hands out, from the same first id
//! on every run. The in-process document belongs to whoever holds it (`semantics::world`): a test
//! that runs one program after another in one thread keeps one document, and a run starts where the
//! last one left it. So each engine's start retires the stream an earlier program drew with
//! (`semantics::dom::retire_stream`): that page keeps its place in the document under ids the
//! recorder mints, and the new program's paint binds its own ids from the first.

use logicaffeine_compile::compile::{tw_outcome, vm_outcome, RunOutcome};
use logicaffeine_compile::semantics::dom::{self, STREAM_ID_BASE};

const FIRST: &str = "## Component App\nView:\n    <section><p id=\"a\">first</p><input id=\"f\" aria-label=\"f\"></section>\n\n## Main\nMount App.\nShow \"first\".\n";

const SECOND: &str = "## Component App\nView:\n    <main><p id=\"b\">second</p><input id=\"d\" aria-label=\"d\"></main>\n\n## Main\nMount App.\nType \"x\" into \"#d\".\nShow \"after\".\n";

const SECOND_PAGE: &str =
    "<body><main><p id=\"b\">second</p><input aria-label=\"d\" id=\"d\" value=\"x\"></main></body>";

const FIRST_PAGE: &str =
    "<body><section><p id=\"a\">first</p><input aria-label=\"f\" id=\"f\"></section></body>";

const TIERS: [(&str, fn(&str) -> RunOutcome); 2] =
    [("tree-walker", tw_outcome), ("VM", vm_outcome)];

#[test]
fn a_page_drawn_after_another_programs_page_draws_on_every_pairing_of_tiers() {
    for (first_tier, first) in TIERS {
        for (second_tier, second) in TIERS {
            let pairing = format!("{first_tier}, then {second_tier}");
            dom::reset();
            let drew = first(FIRST);
            assert_eq!(
                (drew.output.trim(), drew.error.as_deref()),
                ("first", None),
                "{pairing}: the first page"
            );
            assert_eq!(
                dom::render(),
                FIRST_PAGE,
                "{pairing}: the first page is in the document"
            );
            let then = second(SECOND);
            assert_eq!(
                (then.output.trim(), then.error.as_deref()),
                ("after", None),
                "{pairing}: the second page"
            );
            assert_eq!(
                dom::render(),
                SECOND_PAGE,
                "{pairing}: the second page replaced the first"
            );
        }
    }
}

#[test]
fn what_an_earlier_run_drew_stays_for_whoever_holds_the_document() {
    for (first_tier, first) in TIERS {
        for (second_tier, second) in TIERS {
            let pairing = format!("{first_tier}, then {second_tier}");
            dom::reset();
            assert_eq!(first(FIRST).error, None, "{pairing}");
            let plain = second("## Main\nShow 1 + 1.\n");
            assert_eq!(
                (plain.output.trim(), plain.error.as_deref()),
                ("2", None),
                "{pairing}"
            );
            assert_eq!(
                dom::render(),
                FIRST_PAGE,
                "{pairing}: a run that draws nothing leaves the page as it was"
            );
            let a = dom::query_selector("#a");
            assert!(
                (0..STREAM_ID_BASE as i64).contains(&a),
                "{pairing}: the earlier page answers to an id the recorder minted, not the stream's: {a}"
            );
        }
    }
}

#[test]
fn a_focused_field_of_the_earlier_page_keeps_focus_under_its_new_name() {
    dom::reset();
    assert_eq!(vm_outcome(FIRST).error, None);
    let field = dom::query_selector("#f");
    dom::set_focus(field);
    assert_eq!(tw_outcome("## Main\nShow 1.\n").error, None);
    let renamed = dom::query_selector("#f");
    assert!(
        renamed < STREAM_ID_BASE as i64,
        "the field answers to a minted id: {renamed}"
    );
    assert_eq!(
        dom::focused(),
        renamed,
        "focus follows the field to its new name"
    );
}
