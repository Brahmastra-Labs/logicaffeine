//! ★ LOCK — a `Mount` is the program's own call in `## Main` and a host's entry in a `## Test`. ★
//!
//! `Mount App.` in `## Main` is a call the program's main makes. The bytecode VM compiles it to a call of
//! `__mount_App`, and a browser's page main makes the same call, so whatever fails in it — a
//! `When the page loads:` that traps, a view that cannot be drawn — ends the program there. The tree-walker
//! carries `## Main`'s driving statements through the lowered page's driver (`page_host::lowered`), and makes the
//! same call.
//!
//! A `## Test` is a host driving a page. Its `Mount` is an entry the host makes into the page, as `runtime.js`'s
//! `__logosMount` makes one: what fails in it is the page's failure, heard by the page's failure door and
//! reported with nothing of the failed mount drawn, and the test runs on, as it runs on past an event whose turn
//! fails. That holds on the lowered page on the tree-walker and on the VM, in Chrome, and for `largo test`'s own
//! run of the test body.
//!
//! Neither rule asks which part of the mount failed; both shapes run in every row.

#[path = "cli_common/mod.rs"]
mod common;

use logicaffeine_cli::browser::{self, Browser, PageBundle};
use logicaffeine_compile::compile::{tw_outcome, vm_outcome};
use logicaffeine_compile::page_host::{compare_tests, PageTierKind, PageTrace, TierRun};
use logicaffeine_compile::semantics::dom;
use logicaffeine_compile::testrun::{run_tests, TestConfig, TierEvidence, Verdict};

/// The failure both broken components raise: a read past the end of a one-element list, known only when it runs.
const OUT_OF_BOUNDS: &str = "Index 9 out of bounds";

/// `Broken`, whose first paint reads past the end of a list.
const PAINT_FAILS: &str =
    "## Component Broken\nState:\n    Let n be 9.\nActions:\n    To reset:\n        Set n to 1.\n\
View:\n    <p>{item n of [1]}</p>\n\n";

/// `Broken`, whose `When the page loads:` reads past the end of a list before anything is drawn.
const LOAD_TRAPS: &str =
    "## Component Broken\nState:\n    Let n be 0.\n    Let boom be 0.\nActions:\n\
\x20   When the page loads:\n        Set boom to item 9 of [1].\nView:\n    <p>{n}</p>\n\n";

/// Both ways a mount fails, named.
const BROKEN: [(&str, &str); 2] = [
    ("a paint that fails", PAINT_FAILS),
    ("a load handler that traps", LOAD_TRAPS),
];

/// `Fine`, a page that counts its button's clicks and holds a slot another component can be mounted into.
const FINE: &str = "## Component Fine\nState:\n    Let clicks be 0.\nActions:\n    To press:\n        Set clicks to clicks + 1.\n\
View:\n    <main><button id=\"b\" on-click={press}>{clicks}</button><div id=\"slot\"></div></main>\n\n";

/// The page once the failed mount is behind it and the click after it has run.
const ANSWERED: &str = "<main><button id=\"b\">1</button><div id=\"slot\"></div></main>";

/// `broken` mounted by the program's own `## Main`, with a statement after it.
fn in_main(broken: &str) -> String {
    format!("{broken}## Main\nMount Broken.\nShow \"after\".\n")
}

/// `broken` mounted by a test into a page it already mounted, and a click on that page after it.
fn in_a_test(broken: &str) -> String {
    format!(
        "{FINE}{broken}## Test: a mount that fails\n    Mount Fine.\n    Mount Broken at \"#slot\".\n    Click \"#b\".\n\
         \x20   Expect that the text of \"#b\" is equal to \"1\".\n"
    )
}

/// What one tier did with a program: every line it showed, how it ended, and the page it left.
#[derive(Debug, PartialEq, Eq)]
struct Run {
    output: String,
    error: Option<String>,
    page: String,
}

fn run(src: &str, vm: bool) -> Run {
    dom::reset();
    let out = if vm { vm_outcome(src) } else { tw_outcome(src) };
    let output = out.output.lines().collect::<Vec<_>>().join("\n");
    Run {
        output,
        error: out.error,
        page: dom::render(),
    }
}

#[test]
fn a_mount_in_main_that_fails_ends_the_program_on_the_tree_walker_as_on_the_vm() {
    for (how, broken) in BROKEN {
        let src = in_main(broken);
        let tw = run(&src, false);
        assert_eq!(
            tw.error.as_deref(),
            Some(OUT_OF_BOUNDS),
            "{how}: the failure is the program's:\n{src}"
        );
        assert_eq!(tw.output, "", "{how}: nothing after the mount runs:\n{src}");
        assert_eq!(
            run(&src, true),
            tw,
            "{how}: the VM (left) and the tree-walker (right) disagree:\n{src}"
        );
    }
}

/// The test's page on `tiers`, traced: `(tier, trace)` in the order asked.
fn traced(
    src: &str,
    tiers: &[PageTierKind],
    launcher: Option<&dyn logicaffeine_compile::page_host::RemotePageLauncher>,
) -> Vec<(&'static str, PageTrace)> {
    let mut all = compare_tests(src, tiers, 0, 50_000_000, launcher)
        .unwrap_or_else(|e| panic!("the page must parse: {e}\n{src}"));
    assert_eq!(all.len(), 1, "exactly one page scenario:\n{src}");
    all.remove(0)
        .candidates
        .into_iter()
        .map(|(kind, run)| match run {
            TierRun::Traced(t) => (kind.label(), t),
            other => panic!(
                "the {} tier must trace the scenario, not {other:?}\n{src}",
                kind.label()
            ),
        })
        .collect()
}

/// The failed mount is the page's failure and the test runs on: nothing stops the walk, the failure is the first
/// the page reports from the mount's own step on, and the click after it lands.
fn ran_on(how: &str, tier: &str, trace: &PageTrace, src: &str, failure: impl Fn(&str) -> bool) {
    assert!(
        trace.stopped.is_none(),
        "{how}, {tier}: the mount fails the page, not the test: {:?}\n{src}",
        trace.stopped
    );
    let at_mount = &trace
        .steps
        .iter()
        .find(|s| s.statement == 2)
        .expect("the mount's step")
        .snapshot;
    let reported = at_mount
        .failure
        .as_deref()
        .unwrap_or_else(|| panic!("{how}, {tier}: the mount's failure is reported\n{src}"));
    assert!(failure(reported), "{how}, {tier}: {reported}\n{src}");
    assert!(
        !at_mount.document.contains("<p>"),
        "{how}, {tier}: nothing of the failed mount is drawn:\n{}",
        at_mount.document
    );
    let last = &trace.steps.last().expect("a traced step").snapshot;
    assert!(
        last.document.contains(ANSWERED),
        "{how}, {tier}: the click after the mount ran:\n{}",
        last.document
    );
}

#[test]
fn a_mount_a_test_makes_that_fails_fails_the_page_and_the_test_runs_on_every_lowered_tier() {
    for (how, broken) in BROKEN {
        let src = in_a_test(broken);
        for (tier, trace) in traced(
            &src,
            &[PageTierKind::LoweredHere, PageTierKind::LoweredVm],
            None,
        ) {
            ran_on(how, tier, &trace, &src, |f| f == OUT_OF_BOUNDS);
        }
    }
}

#[test]
fn largo_test_runs_a_test_on_past_a_mount_that_fails_and_agrees_with_its_page_tiers() {
    for (how, broken) in BROKEN {
        let src = in_a_test(broken);
        let report = run_tests(&src, &TestConfig::default()).expect("must parse");
        let o = &report.outcomes[0];
        assert_eq!(
            o.verdict,
            Verdict::Passed,
            "{how}: the Expect after the mount is what decides: {:?}\n{src}",
            o.failures
        );
        assert_eq!(
            o.tier,
            Some(TierEvidence::Agreed),
            "{how}: the lowered VM agrees with the test's own run\n{src}"
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

#[test]
fn a_mount_a_test_makes_that_fails_fails_the_page_and_the_test_runs_on_in_chrome() {
    if chrome().is_none() {
        return;
    }
    for (how, broken) in BROKEN {
        let src = in_a_test(broken);
        let launcher = browser::ChromeLauncher::default();
        for (tier, trace) in traced(&src, &[PageTierKind::Browser], Some(&launcher)) {
            ran_on(how, tier, &trace, &src, |_| true);
        }
    }
}

#[test]
fn a_mount_in_main_that_fails_ends_the_page_main_in_chrome() {
    let Some(b) = chrome() else { return };
    for (how, broken) in BROKEN {
        let src = in_main(broken);
        let bundle =
            PageBundle::page(&src).unwrap_or_else(|e| panic!("the page bundles: {e}\n{src}"));
        // A page whose main stops is one the harness cannot start, and its refusal carries what the page logged.
        let why = match b.open(&bundle) {
            Err(e) => e.to_string(),
            Ok(_) => panic!("{how}: the page's main ran past the mount in the browser:\n{src}"),
        };
        assert!(
            why.contains("`main` \"threw\""),
            "{how}: the page's main stops at the mount: {why}\n{src}"
        );
        assert!(
            why.contains(&format!("LOGOS runtime failure: {OUT_OF_BOUNDS}")),
            "{how}: the browser reports the failure: {why}\n{src}"
        );
        assert!(
            !why.contains("[log] after"),
            "{how}: nothing after the mount runs in the browser: {why}\n{src}"
        );
    }
}
