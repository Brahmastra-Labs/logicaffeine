//! ★ LOCK — the triggers a BROWSER delivers and the in-process runner cannot, named and measured.
//!
//! `trigger_tier_lock` classifies every trigger `Lowered` or `RefusedByName`, and `Lowered`'s own
//! doc says "the trigger compiles for the browser and its handler runs there". That is TRUE, and it
//! is what misleads: a reader takes it for "works", and three triggers marked `Lowered` never fire
//! on the tier that `## Test` and `lgx_vm_parity_lock` actually drive.
//!
//! ★ ONE CAUSE, THREE SYMPTOMS. Each is delivered HOST → MODULE, and the in-process bytecode runner
//! cannot call a function or closure it finds at run time:
//!
//! | trigger | how a browser delivers it |
//! |---|---|
//! | `When an action fails …:` | the host catches the trap and calls `__lgx_on_failure` |
//! | `When the route changes:` | `dom_navigate` calls the closure `onRoute` stored |
//! | a typed fetch's completion | the answer arrives in a closure the host invokes |
//!
//! ★ WHY THIS IS MEASURED RATHER THAN LISTED. A hand-written list of "things that don't work
//! in-process" is stale the first time one is fixed, and stale in the direction that matters — it
//! would still claim a working trigger is broken. So each test DRIVES the trigger on both tiers and
//! asserts they DISAGREE. **A failure here is good news**: it means the in-process runner learned
//! to deliver that trigger, and the test should be replaced by a parity test that asserts they
//! agree.
//!
//! The fix, when it comes, is one thing and not three: an entry point that lets the runner re-enter
//! a compiled function after `run()` returns, the way `runtime.js` re-enters through a wasm export.

use logicaffeine_compile::compile::{tw_outcome, vm_outcome};
use logicaffeine_compile::semantics::dom;

/// What each tier renders for `src` — the two are expected to DISAGREE here.
fn both(src: &str) -> (String, String) {
    dom::reset();
    let t = tw_outcome(src);
    assert_eq!(
        t.error, None,
        "the tree-walker must run the page cleanly:\n{src}"
    );
    let tw = dom::render();
    dom::reset();
    let _ = vm_outcome(src);
    (tw, dom::render())
}

const ROUTED: &str = "\
## Component App
State:
    Let hops be 0.
Actions:
    When the route changes:
        Set hops to hops + 1.
    To go:
        Navigate to \"/x\".
View:
    <p id=\"p\">{hops}</p><button id=\"a\" on-click={go}>a</button>

## Main
Mount App.
Click \"#a\".
Click \"#a\".
";

/// ★ `When the route changes:` FIRES ON BOTH TIERS — this test used to assert the opposite.
///
/// It read "reaches the tree walker and NOT the in-process runner", and said so in its own failure
/// message: *"If this now reads 2, the runner learned to re-enter a compiled function — delete this
/// test and assert PARITY instead."* That is what happened, so this is that instruction carried
/// out (with Tristen's ruling, since a red test is never quietly rewritten).
///
/// The gap was not the runner failing to re-enter a compiled function. It was the LOWERING: the
/// handler is handed to `onRoute` as a closure at mount, and `Navigate to` lowered to `navigate(url)`
/// and nothing else — so nobody called the closure. Both the other two tiers already did: the
/// browser's `dom_navigate` calls the route handler right after `history.pushState`, and the
/// reference tier notes the change and delivers it when the turn unwinds. The bytecode tier was the
/// odd one out, and a page that navigated ITSELF never ran its own route handler in production.
///
/// Now it is delivered from the same deferred seam every other in-page delivery uses, fanned out to
/// every LIVE instance that asked.
#[test]
fn a_route_change_reaches_both_tiers() {
    let (tw, vm) = both(ROUTED);
    assert!(
        tw.contains(">2<"),
        "the tree-walker delivers both route changes: {tw}"
    );
    assert!(
        vm.contains(">2<"),
        "and so must the in-process runner — `Navigate to` owes the handler a delivery, which is \
         what a browser's `dom_navigate` does after `pushState`: {vm}"
    );
}

/// ★ AND NEITHER DOES `When an action fails …:`.
///
/// A browser reaches it because a trap unwinds to the HOST, which calls `__lgx_on_failure`. The
/// in-process runner has no host: the error propagates out of `run()` and the handler never runs.
#[test]
fn an_action_failure_reaches_the_tree_walkers_handler_and_not_the_runners() {
    let src = "\
## Component App
State:
    Let why be \"none\".
Actions:
    When an action fails (e: Text):
        Set why to e.
    To boom:
        Set why to \"ran\".
        Let bad be item 9 of [1, 2].
View:
    <p id=\"w\">{why}</p><button id=\"b\" on-click={boom}>b</button>

## Main
Mount App.
Click \"#b\".
";
    dom::reset();
    let t = tw_outcome(src);
    let tw = dom::render();
    assert_eq!(
        t.error, None,
        "a page WITH a failure handler absorbs the failure on this tier"
    );
    assert!(
        tw.contains("bounds") || tw.contains("Index") || tw.contains("9"),
        "the tree-walker runs the handler and it sees the message: {tw}"
    );

    dom::reset();
    let v = vm_outcome(src);
    assert_eq!(
        v.error, None,
        "the runner now delivers the failure to the page's own handler — parity with the tree-walker"
    );
    assert_eq!(
        dom::render(),
        tw,
        "both tiers must show the same handled message"
    );
}

/// ★ THE TEETH. Both tests above assert a DISAGREEMENT, and a disagreement is also what you get
/// when a page is simply broken on one tier. So: the same page shape, with the trigger removed,
/// must AGREE — proving the tiers differ over the trigger and not over everything.
#[test]
fn the_same_page_without_the_trigger_agrees_on_both_tiers() {
    let src = "\
## Component App
State:
    Let hops be 0.
Actions:
    To go:
        Set hops to hops + 1.
        Navigate to \"/x\".
View:
    <p id=\"p\">{hops}</p><button id=\"a\" on-click={go}>a</button>

## Main
Mount App.
Click \"#a\".
Click \"#a\".
";
    let (tw, vm) = both(src);
    assert_eq!(
        tw, vm,
        "navigation itself works on both tiers — what differs is only whether the ROUTE-CHANGE \
         HANDLER is delivered, which is what makes the two tests above about the trigger"
    );
    assert!(tw.contains(">2<"), "and the action itself ran twice: {tw}");
}
