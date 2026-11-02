//! LOCK — a page's ONE global timer interval is armed by whichever mount function actually RUNS
//! at runtime, not by whichever component happens to be declared (and so compiled) first.
//!
//! `lgx_lower.rs::lower_component_mount` emits the arm-and-`setInterval` code into EVERY props-
//! less component's mount function — one is generated for every such component regardless of
//! whether the program ever calls `Mount` on it, because a component may be PLACED rather than
//! mounted. If that arming is gated by a flag that is claimed the first time such a function is
//! COMPILED, declaring any props-less component (used or not) before the page's real root lets
//! that component's own dead, uncalled mount function claim the flag — leaving the root's real
//! mount function with no arming code at all, so the page's clock never starts and no `Every …`/
//! `After …` timer anywhere in the program ever fires, on a browser or in a test.

use logicaffeine_compile::testrun::{run_tests, TestConfig, Verdict};

/// A page with a timer-bearing, PLACED-only `Ticker` and a `Mount`ed `App` root — the two
/// declared in the order `first`/`second` names, so the only variable between the two tests below
/// is which one the parser compiles first.
fn ticks(first: &str, second: &str) -> String {
    let ticker = "## Component Ticker\nState:\n\x20   Let n be 0.\n\
        Actions:\n\x20   Every 1 seconds:\n\x20       Set n to n + 1.\n\
        View:\n\x20   <span id=\"t\">{n}</span>\n\n";
    let app = "## Component App\nState:\n\x20   Let on be true.\n\
        View:\n\x20   <main><Ticker/><button id=\"g\">t</button></main>\n\n";
    let bodies = match (first, second) {
        ("Ticker", "App") => format!("{ticker}{app}"),
        ("App", "Ticker") => format!("{app}{ticker}"),
        _ => unreachable!(),
    };
    format!(
        "{bodies}## Test: the page's clock ticks\n\x20   Mount App.\n\x20   Advance time by 2 seconds.\n\
         \x20   Expect that the text of \"#t\" is equal to \"2\".\n"
    )
}

fn assert_passes(src: &str, label: &str) {
    let cfg = TestConfig {
        all_tiers: true,
        ..Default::default()
    };
    let outcome = run_tests(src, &cfg)
        .expect("the suite parses")
        .outcomes
        .remove(0);
    assert!(
        matches!(outcome.verdict, Verdict::Passed),
        "{label}: {:?}\n{:#?}\n{src}",
        outcome.verdict,
        outcome.failures
    );
}

#[test]
fn a_placed_only_component_declared_before_the_root_does_not_silence_the_pages_timers() {
    assert_passes(
        &ticks("Ticker", "App"),
        "a placed-only component declared before the page's root must not silence its timers",
    );
}

#[test]
fn the_same_page_with_the_root_declared_first_still_ticks() {
    // The control: this declaration order already worked before the fix (the root's own mount
    // function was the first one compiled, so it claimed the arming) — it must keep working, so
    // the fixture itself, not its ordering, stays the only thing this file exercises.
    assert_passes(
        &ticks("App", "Ticker"),
        "the control ordering must keep passing",
    );
}
