//! ★ LOCK — a watcher cycle is REFUSED, and does not kill the process.
//!
//! ★ MEASURED BEFORE THE FIX: `fatal runtime error: stack overflow, aborting` — SIGABRT. Not an
//! error, not a refusal; the runtime died. In a browser that is a dead tab, and under `largo test`
//! a dead runner that takes the rest of the suite with it. Two shapes did it:
//!
//! - a `When S's v changes:` handler that calls `S's bump` — it feeds itself;
//! - two stores watching each other, which is the same cycle one step further apart.
//!
//! ★ AND A THIRD SHAPE ALREADY TERMINATED, which is what told us where the bug was.
//! `When <field> changes:` within a component runs ONE ROUND by explicit design — its doc says so:
//! "a handler may write state, but its writes do not start another round, so two fields that watch
//! each other settle instead of looping." The STORE path never got that rule.
//!
//! ★ THE BUDGET HAS TO LAND BEFORE THE STACK DOES. The first ceiling tried was 100 — Vue's number —
//! and it still aborted, because every level here is a real chain of frames — the drain, the
//! watcher it wakes, the store action that watcher calls — each with a scope of its own. See
//! `STORE_CASCADE_LIMIT`.

use logicaffeine_compile::compile::{tw_outcome, vm_outcome};
use logicaffeine_compile::semantics::dom;

/// A page whose watcher feeds the store it watches.
const SELF_FEEDING: &str = "\
## Store S
State:
    Let v be 0.
Actions:
    To bump:
        Set v to v + 1.

## Component W
Use S.
State:
    Let seen be 0.
Actions:
    When S's v changes:
        Set seen to seen + 1.
        Call S's bump.
View:
    <span id=\"w\">{seen}|{S's v}</span>

## Component App
Use S.
Actions:
    To go:
        Call S's bump.
View:
    <main><W/><button id=\"x\" on-click={go}>x</button></main>

## Main
Mount App.
Click \"#x\".
";

/// Two stores watching each other — the same cycle, one step further apart.
const MUTUAL: &str = "\
## Store A
State:
    Let a be 0.
Actions:
    To bump:
        Set a to a + 1.

## Store B
State:
    Let b be 0.
Actions:
    To bump:
        Set b to b + 1.

## Component WA
Use A.
Use B.
Actions:
    When A's a changes:
        Call B's bump.
View:
    <span id=\"wa\">a</span>

## Component WB
Use A.
Use B.
Actions:
    When B's b changes:
        Call A's bump.
View:
    <span id=\"wb\">b</span>

## Component App
Use A.
Use B.
Actions:
    To go:
        Call A's bump.
View:
    <main><WA/><WB/><button id=\"x\" on-click={go}>x</button></main>

## Main
Mount App.
Click \"#x\".
";

/// ★ THE WHOLE POINT: it must REPORT, and the process must still be alive to report it. If this
/// test ever aborts rather than fails, the budget stopped landing before the stack.
#[test]
fn a_self_feeding_watcher_is_reported_rather_than_crashing() {
    dom::reset();
    let why = tw_outcome(SELF_FEEDING)
        .error
        .expect("a cycle that never settles must be reported");
    assert!(
        why.contains("never settles"),
        "the message must say what is wrong in the author's terms, not 'stack overflow': {why}"
    );
    assert!(
        why.contains("When <Store>'s <field> changes:"),
        "and name the construct that did it: {why}"
    );
}

/// The same, for a cycle that passes through two stores — a budget catches shapes a
/// direct-self-reference check would miss.
#[test]
fn mutually_watching_stores_are_reported_rather_than_crashing() {
    dom::reset();
    let why = tw_outcome(MUTUAL)
        .error
        .expect("a two-store cycle must be reported too");
    assert!(
        why.contains("never settles"),
        "same message for a longer cycle: {why}"
    );
}

/// The browser tier survives both as well. Its message is its own generic depth guard rather than
/// the sentence above — less to read, and it is an ERROR rather than a dead tab, which is the
/// property that matters. Recorded as an asymmetry rather than asserted equal.
#[test]
fn the_browser_tier_survives_a_cycle_too() {
    for (label, src) in [("self-feeding", SELF_FEEDING), ("mutual", MUTUAL)] {
        dom::reset();
        assert!(
            vm_outcome(src).error.is_some(),
            "`{label}` must report on the VM rather than abort the process"
        );
    }
}

/// ★ THE TEETH. A budget that refused every cascade would pass all three tests above and make
/// watchers useless. A REAL cascade — one store's watcher updating a second store that something
/// else watches — must still run, and settle.
#[test]
fn a_cascade_that_settles_is_not_refused() {
    let src = "\
## Store S1
State:
    Let a be 0.
Actions:
    To bump:
        Set a to a + 1.

## Store S2
State:
    Let b be 0.
Actions:
    To bump:
        Set b to b + 1.

## Component Mid
Use S1.
Use S2.
Actions:
    When S1's a changes:
        Call S2's bump.
View:
    <span id=\"m\">m</span>

## Component App
Use S1.
Use S2.
Actions:
    To go:
        Call S1's bump.
View:
    <main><Mid/><span id=\"s\">{S1's a}|{S2's b}</span><button id=\"x\" on-click={go}>x</button></main>

## Main
Mount App.
Click \"#x\".
";
    dom::reset();
    assert_eq!(
        tw_outcome(src).error,
        None,
        "a cascade that settles must not be refused"
    );
    let tw = dom::render();
    assert!(
        tw.contains(">1|1<"),
        "and it must actually have cascaded: {tw}"
    );
    dom::reset();
    assert_eq!(vm_outcome(src).error, None, "the VM agrees");
    assert_eq!(dom::render(), tw, "and renders the same");
}

/// A `When <field> changes:` handler writing its OWN field already settled by design — one round,
/// never another. Pinned so that rule cannot be lost while the store side is being changed.
#[test]
fn a_field_watcher_writing_its_own_field_still_settles() {
    let src = "\
## Component App
State:
    Let n be 0.
    Let seen be 0.
Actions:
    When n changes:
        Set seen to seen + 1.
        Set n to n + 1.
    To go:
        Set n to 1.
View:
    <p id=\"p\">{n}|{seen}</p><button id=\"x\" on-click={go}>x</button>

## Main
Mount App.
Click \"#x\".
";
    dom::reset();
    assert_eq!(tw_outcome(src).error, None);
    let tw = dom::render();
    assert!(
        tw.contains(">2|1<"),
        "one round: the handler ran once and its write did not re-fire: {tw}"
    );
    dom::reset();
    assert_eq!(vm_outcome(src).error, None);
    assert_eq!(dom::render(), tw, "both tiers settle alike");
}
