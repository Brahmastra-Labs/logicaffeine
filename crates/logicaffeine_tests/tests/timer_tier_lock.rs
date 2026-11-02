//! ★ CLOSED — a page that ticks is buildable AND drivable for a browser. ★
//!
//! `Every 10 seconds:` and `After 5 seconds:` are how a page polls, auto-saves, retries or
//! animates. They run on the reference tier — `timer_lock` drives eight cases — and now run on the
//! bytecode tier too, which is what the second test here proves by re-running the same `## Test`
//! under `--tiers all` and comparing the output byte for byte.
//!
//! ★ THIS FILE ARGUED THE OPPOSITE, AND THE ARGUMENT IS WORTH KEEPING because it was half right.
//! It said the binding constraint was not the timer LOWERING but the absent virtual clock: the VM
//! compiled `Stmt::AdvanceTime` away to nothing, so a timer table would have produced "a page that
//! compiles for a browser and can never be shown to tick — a green suite proving nothing".
//!
//! That reasoning still holds. What was wrong was the conclusion that a clock had to be an
//! architectural decision: it turned out to be ONE compiled function. Every piece of "which timer
//! is due" is compiled LOGOS in `__lgx_tick(ms)`, and the tiers differ only in what CALLS it — a
//! browser arms one `setInterval` for the whole page (the single import this needed) and hands it
//! the elapsed milliseconds, while `Advance time by …` calls the same function directly. One
//! arithmetic, two ways in, so the driver exercises the code a browser runs rather than a model of
//! it. `Stop the timer …` then needed no import at all: a timer is a countdown held per instance,
//! and stopping it is writing the value that means "not running".
//!
//! ★ AND CLOSING IT EXPOSED A TIER DIVERGENCE THE OLD REFUSAL HAD BEEN HIDING. `25 seconds` lexes
//! as ONE Duration token, so `Advance time by 25 seconds.` left a Duration in a field documented as
//! "the advance in MILLISECONDS". The interpreter converted it at run time; the VM did not, and the
//! page answered "Cannot compare Duration and Int" on one tier and ran clean on the other. The AST
//! node did not hold what its own field said it held — folded at the parser now, where the value is
//! known, so no consumer downstream has to know a duration literal exists.
//!
//! Kept as the pair it always was: the reference tier's behaviour pinned beside the browser
//! tier's, so neither can drift from the other unnoticed.

use logicaffeine_compile::testrun::{run_tests, TestConfig, Verdict};

const TICKER: &str = "\
## Component App
State:
    Let ticks be 0.
Actions:
    Every 10 seconds:
        Set ticks to ticks + 1.
View:
    <p id=\"n\">{ticks}</p>

## Test: a timer ticks on the virtual clock
    Mount App.
    Advance time by 30 seconds.
    Expect that the text of \"#n\" is equal to \"3\".
";

#[test]
fn the_reference_tier_ticks_on_the_virtual_clock() {
    // The contract the browser tier must eventually meet — and it holds today, which is what
    // makes the gap a tier gap rather than a missing feature.
    let report = run_tests(TICKER, &TestConfig::default()).expect("the suite parses");
    assert!(
        matches!(report.outcomes[0].verdict, Verdict::Passed),
        "the reference tier drives timers on the virtual clock: {:?}",
        report.outcomes[0].failures
    );
}

/// ★ The gap itself. See the module comment: the blocker is the ABSENT VIRTUAL CLOCK on the
/// bytecode tier, not the timer lowering, so this cannot be made to pass by emitting more code.
#[test]
fn a_repeating_timer_reaches_the_browser_tier() {
    let src = TICKER.replace(
        "## Test: a timer ticks",
        "## Test: a timer ticks on the browser tier",
    );
    let report = run_tests(
        &src,
        &TestConfig {
            all_tiers: true,
            ..TestConfig::default()
        },
    )
    .expect("the suite parses");
    assert!(
        matches!(report.outcomes[0].verdict, Verdict::Passed),
        "a page that ticks must be buildable AND drivable for a browser: {:?}",
        report.outcomes[0].failures
    );
}
