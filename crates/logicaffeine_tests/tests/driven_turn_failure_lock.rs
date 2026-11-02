//! ★ LOCK — a program that drives its own page runs on past a turn that fails. ★
//!
//! `Click "#go".` in `## Main` is the host reaching into the page: the page hears the click as an
//! entry of its own, exactly as it hears a visitor's. So a turn that fails there — here its repaint
//! divides by zero — is contained to that entry: the page's failure door hears it (the context the
//! page left open, then its `When an action fails:` handler), the program's next statement runs,
//! and a failure no handler absorbed is the program's own failure once nothing is left to run. The
//! tree-walker drives through the page host's guarded door (`page_host::lowered`), the bytecode VM
//! through `BuiltinId::DomDispatchEntry`, the browser through `runtime.js`'s `dom_page_entry`;
//! each row holds every tier to one outcome.

#[path = "cli_common/mod.rs"]
mod common;

use logicaffeine_cli::browser::{self, Browser, PageBundle};
use logicaffeine_compile::compile::{tw_outcome, vm_outcome, RunOutcome};
use logicaffeine_compile::semantics::dom;

/// A page whose repaint divides by `n`, and a click that sets `n` to zero: the click's turn fails
/// in its repaint.
const FAILING: &str =
    "## Component App\nState:\n    Let n be 1.\nActions:\n    To zero:\n        Set n to 0.\n\
View:\n    <p>{10 / n}</p><button id=\"go\" on-click={zero}>0</button>\n\n";

/// A page whose click's turn fails in its body, and a handler that absorbs the failure.
const ABSORBED: &str = "## Component App\nState:\n    Let n be 1.\n    Let seen be \"\".\nActions:\n    To zero:\n        Set n to n / 0.\n    When an action fails (e: Text):\n        Set seen to \"caught\".\n\
View:\n    <p>{n}</p><p id=\"seen\">{seen}</p><button id=\"go\" on-click={zero}>0</button>\n\n";

/// A page with a click whose turn fails inside a loop over a list — the loop's frame still open
/// when it traps — and a click that counts.
const TWO_BUTTONS: &str = "## Component App\nState:\n    Let n be 0.\n    Let xs be [1, 2].\nActions:\n    To bad:\n        Repeat for x in xs:\n            Set n to n + x / 0.\n    To good:\n        Set n to n + 1.\n\
View:\n    <p>{n}</p><button id=\"bad\" on-click={bad}>bad</button><button id=\"good\" on-click={good}>good</button>\n\n";

/// A loop over a list in `## Main` whose first click fails: the failed turn's loop frame must not
/// outlive it, or the program's own loop reads it for its next item.
fn looped() -> String {
    format!(
        "{TWO_BUTTONS}## Main\nMount App.\nRepeat for i in [1, 2, 3]:\n    If i equals 1:\n        Click \"#bad\".\n    Otherwise:\n        Click \"#good\".\n    Show \"after {{i}}\".\nShow \"done\".\n"
    )
}

/// `page`, mounted, clicked, and followed by a statement that is not a driving one.
fn driven(page: &str) -> String {
    format!("{page}## Main\nMount App.\nShow \"before\".\nClick \"#go\".\nShow \"after\".\n")
}

/// `(output, error, document)` on the tree-walker and on the VM, each output without the line end
/// the VM's own convention closes it with.
fn tiers(src: &str) -> [(&'static str, RunOutcome, String); 2] {
    let settled = |mut o: RunOutcome| {
        o.output = o.output.trim_end_matches('\n').to_string();
        o
    };
    dom::reset();
    let tw = settled(tw_outcome(src));
    let tw_doc = dom::render();
    dom::reset();
    let vm = settled(vm_outcome(src));
    let vm_doc = dom::render();
    [("tree-walker", tw, tw_doc), ("VM", vm, vm_doc)]
}

#[test]
fn a_failed_driven_turn_is_the_programs_failure_and_the_program_runs_on() {
    let src = driven(FAILING);
    for (tier, outcome, doc) in tiers(&src) {
        assert_eq!(
            outcome.output, "before\nafter",
            "{tier}: the statement after the failed turn runs:\n{src}"
        );
        assert_eq!(
            outcome.error.as_deref(),
            Some("Division by zero"),
            "{tier}: the unabsorbed failure is the program's:\n{src}"
        );
        assert_eq!(
            doc, "<body><p>10</p><button id=\"go\">0</button></body>",
            "{tier}: the failed repaint drew nothing:\n{src}"
        );
    }
}

#[test]
fn a_failure_the_page_absorbs_is_not_the_programs() {
    let src = driven(ABSORBED);
    let [(_, tw, tw_doc), (_, vm, vm_doc)] = tiers(&src);
    assert_eq!(
        tw.error, None,
        "the tree-walker: the page's handler absorbed it:\n{src}"
    );
    assert_eq!(
        tw.output, "before\nafter",
        "the tree-walker runs on:\n{src}"
    );
    assert_eq!(
        (vm.output, vm.error, vm_doc),
        (tw.output, tw.error, tw_doc),
        "the VM is the tree-walker:\n{src}"
    );
}

#[test]
fn a_failed_turn_at_the_programs_end_keeps_what_the_program_showed() {
    let src = format!("{FAILING}## Main\nMount App.\nShow \"before\".\nClick \"#go\".\n");
    for (tier, outcome, _) in tiers(&src) {
        assert_eq!(outcome.output, "before", "{tier}:\n{src}");
        assert_eq!(
            outcome.error.as_deref(),
            Some("Division by zero"),
            "{tier}:\n{src}"
        );
    }
}

/// A compiled click fails before a closing driving script whose own click succeeds: the host that
/// carries the script takes the contained failure as the page's, and the script still runs.
#[test]
fn a_failure_before_a_driving_script_leaves_the_script_to_run() {
    let src = format!(
        "{TWO_BUTTONS}## Main\nMount App.\nClick \"#bad\".\nShow \"between\".\nClick \"#good\".\n"
    );
    for (tier, outcome, _) in tiers(&src) {
        assert_eq!(outcome.output, "between", "{tier}:\n{src}");
        assert_eq!(
            outcome.error.as_deref(),
            Some("Division by zero"),
            "{tier}:\n{src}"
        );
    }
}

/// A driven turn fails inside a directive's block: the failure door names the directive, as every
/// tier's does, before the program runs on.
#[test]
fn a_failed_turn_in_a_directive_is_named_by_it() {
    let src = "## Directive bad (el: Element) and (v: Text):\n    Show \"on\".\n    When removed:\n        Show item 9 of [\"a\"].\n\n\
## Component App\nState:\n    Let open be true.\nActions:\n    To toggle:\n        Set open to not open.\n\
View:\n    <main>{if open:}<p bad=\"x\">t</p>{end}<button id=\"t\" on-click={toggle}>t</button></main>\n\n\
## Main\nMount App.\nClick \"#t\".\nShow \"after\".\n";
    let prefix = logicaffeine_language::ast::component::directive_failure_prefix("bad", true);
    let [(_, tw, _), (_, vm, _)] = tiers(src);
    assert_eq!(tw.output, "on\nafter", "the tree-walker runs on:\n{src}");
    assert!(
        tw.error.as_deref().is_some_and(|e| e.starts_with(&prefix)),
        "the tree-walker names the directive: {:?}",
        tw.error
    );
    assert_eq!(
        (vm.output, vm.error),
        (tw.output, tw.error),
        "the VM is the tree-walker:\n{src}"
    );
}

#[test]
fn a_loop_of_driven_turns_runs_on_past_the_one_that_fails() {
    let src = looped();
    for (tier, outcome, doc) in tiers(&src) {
        assert_eq!(
            outcome.output, "after 1\nafter 2\nafter 3\ndone",
            "{tier}: every iteration runs:\n{src}"
        );
        assert_eq!(
            outcome.error.as_deref(),
            Some("Division by zero"),
            "{tier}:\n{src}"
        );
        assert_eq!(
            doc,
            "<body><p>2</p><button id=\"bad\">bad</button><button id=\"good\">good</button></body>",
            "{tier}: the later turns drew:\n{src}"
        );
    }
}

fn chrome() -> Option<&'static Browser> {
    let Some(chrome) = browser::locate_chrome() else {
        let spec = std::env::var("LOGOS_REQUIRE_TOOLS").unwrap_or_default();
        common::require_tool_strict("chrome", &spec);
        return None;
    };
    Some(browser::shared(&chrome).unwrap_or_else(|e| {
        panic!(
            "Chrome is installed at {} and must start: {e}",
            chrome.display()
        )
    }))
}

/// The page's own markup in a browser, the host's failure panel aside.
const PAGE_MARKUP: &str =
    "Array.from(document.body.children).filter((e) => !e.hasAttribute('data-logos-host')).map((e) => e.outerHTML).join('')";

#[test]
fn a_browser_runs_on_past_a_failed_driven_turn() {
    let Some(b) = chrome() else { return };
    let src = driven(FAILING);
    let bundle = PageBundle::page(&src).unwrap_or_else(|e| panic!("the page bundles: {e}\n{src}"));
    let p = b
        .open(&bundle)
        .unwrap_or_else(|e| panic!("the page opens: {e}"));
    let logs: Vec<String> = p
        .console()
        .into_iter()
        .filter(|c| c.level == "log")
        .map(|c| c.text)
        .collect();
    assert_eq!(
        logs,
        ["before", "after"],
        "the browser runs the statement after the failed turn:\n{src}"
    );
    let errors: Vec<String> = p
        .console()
        .into_iter()
        .filter(|c| c.level == "error")
        .map(|c| c.text)
        .collect();
    assert!(
        errors.iter().any(|e| e.contains("Division by zero")),
        "the browser reports the failure: {errors:?}"
    );
    let markup = p
        .eval(PAGE_MARKUP)
        .unwrap_or_else(|e| panic!("markup: {e}"));
    assert_eq!(
        markup.as_str(),
        Some("<p>10</p><button id=\"go\">0</button>"),
        "the failed repaint drew nothing"
    );
}

#[test]
fn a_browsers_loop_of_driven_turns_runs_on_past_the_one_that_fails() {
    let Some(b) = chrome() else { return };
    let src = looped();
    let bundle = PageBundle::page(&src).unwrap_or_else(|e| panic!("the page bundles: {e}\n{src}"));
    let p = b
        .open(&bundle)
        .unwrap_or_else(|e| panic!("the page opens: {e}"));
    let logs: Vec<String> = p
        .console()
        .into_iter()
        .filter(|c| c.level == "log")
        .map(|c| c.text)
        .collect();
    assert_eq!(
        logs,
        ["after 1", "after 2", "after 3", "done"],
        "every iteration runs in the browser:\n{src}"
    );
    let markup = p
        .eval(PAGE_MARKUP)
        .unwrap_or_else(|e| panic!("markup: {e}"));
    assert_eq!(
        markup.as_str(),
        Some("<p>2</p><button id=\"bad\">bad</button><button id=\"good\">good</button>"),
        "the later turns drew"
    );
}
