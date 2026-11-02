//! ★ LOCK — an event modifier is honoured or refused, never silently ignored. ★
//!
//! `on-keydown.enter={add}` says "run `add` when the visitor presses Enter". The reference tier
//! threads the key through and fires only on a match. The browser tier read `modifiers` exactly
//! once — for `"prevent"` — and emitted a listener with NO key comparison in it, so `add` ran on
//! EVERY keystroke.
//!
//! ★★ THE WORST SHAPE A BUG CAN HAVE: it behaved correctly on the tier the author tests on and
//! differently on the tier they ship. A `## Test` passes, `largo test` is green, and the page in
//! somebody's browser fires a submit on every letter typed into the box.
//!
//! ★ IT WAS REFUSED, AND NOW IT LOWERS — and the condition this file set for that is the one that
//! was met. Honouring `.enter` needs the EVENT'S KEY, and no builtin read one; the note here said a
//! gate "would need a new host import, which is a deliberate act against the frozen ABI
//! (`wasm_host_abi_lock`) and not something to smuggle in behind a bug fix." That act has now been
//! taken deliberately: `dom_event_key` is declared in `wasm_host_abi_lock` with its reasoning, and
//! `key_modifier_lock` holds the behaviour. Refusal was the honest interim answer, not the
//! destination — "press Enter to submit" is the second thing anybody writes after a click.
//!
//! ★ THE HOST STAYS A SHIM. `dom_event_key` reports `KeyboardEvent.key` and decides nothing: the
//! comparison, the case-folding and the gate are compiled LOGOS, which is why the dispatcher can
//! apply the same rule from the other end (the driving verb carries the key in the dispatch's
//! value slot, so no import is involved there at all).
//!
//! ★ WHAT IS STILL REFUSED is the BEHAVIOURAL set — `.stop`, `.once`, `.capture`, `.passive` —
//! which shapes how an event is DISPATCHED rather than which event it is.
//!
//! ★ AND THE REFUSED SET IS NAMED, NOT INFERRED. It was once a denylist of
//! `stop|once|capture|passive`, so a key modifier passed simply by not being on it — and was then
//! never honoured, which is the bug above. The four that do not lower are still named explicitly,
//! so a NEW behavioural modifier fails closed rather than being read as a key.
//!
//! ★ ANY OTHER WORD IS A COMPILE ERROR, on both tiers. The catalogue in `lgx/html.rs` classifies
//! every `.qualifier` once, at parse time — behavioural (`.stop`, `.once`, `.capture`, `.passive`),
//! a closed key-name set, or a system key — so neither tier ever sees an unclassified word to
//! guess at. A mistyped modifier is refused with a did-you-mean before the page builds, which is
//! the one way the two tiers cannot disagree about what it means.
//!
//! Found by `lgx-framework-audit` reading the lowering, and confirmed by RUNNING it before anything
//! was changed — the difference that has mattered repeatedly on this campaign.

use logicaffeine_compile::compile::{tw_outcome, vm_outcome};
use logicaffeine_compile::semantics::dom;

fn page(handler: &str, drive: &str) -> String {
    format!(
        "## Component App\n\
         \n\
         State:\n\
        \x20   Let n be 0.\n\
         \n\
         Actions:\n\
        \x20   To add:\n\
        \x20       Set n to n + 1.\n\
         \n\
         View:\n\
        \x20   <p id=\"p\">{{n}}</p><input id=\"d\" {handler}>\n\
         \n\
         ## Main\n\
         Mount App.\n\
         {drive}\n"
    )
}

#[test]
fn a_key_modifier_lowers_and_gates_on_its_key() {
    // ★ THIS TEST USED TO ASSERT THE REFUSAL. Its subject is the same — a key modifier must never
    // be silently ignored — but the honest answer moved from "refuse it" to "honour it" the moment
    // `dom_event_key` was added deliberately. The assertion is strictly stronger: it is no longer
    // enough that the page refuses to build, it must run and fire on the right key only.
    dom::reset();
    let matched = page("on-keydown.enter={add}", "Press \"Enter\" in \"#d\".");
    assert_eq!(
        vm_outcome(&matched).error,
        None,
        "a key modifier must reach the browser tier"
    );
    assert!(
        dom::render().contains(">1<"),
        "and fire on its key: {}",
        dom::render()
    );

    dom::reset();
    let other = page("on-keydown.enter={add}", "Press \"a\" in \"#d\".");
    let err = vm_outcome(&other).error;
    assert!(
        err.is_some(),
        "and NOT on another key — a listener with no key comparison is the original bug, and it \
         looks exactly like a working page until somebody types a letter"
    );
}

#[test]
fn the_reference_tier_still_honours_it() {
    // ★ THE CLAIM UNDER THE REFUSAL. "This tier cannot honour it" is only honest if another one
    // does — and this is also the assertion that would have caught the original bug, because it
    // pins the BEHAVIOUR the browser tier was failing to reproduce: the wrong key does nothing.
    dom::reset();
    let matched = page("on-keydown.enter={add}", "Press \"Enter\" in \"#d\".");
    assert_eq!(
        tw_outcome(&matched).error,
        None,
        "Enter must reach the handler"
    );
    assert!(
        dom::render().contains(">1<"),
        "the matching key fires it: {}",
        dom::render()
    );

    dom::reset();
    let other = page("on-keydown.enter={add}", "Press \"a\" in \"#d\".");
    let err = tw_outcome(&other).error;
    assert!(
        err.is_some(),
        "a NON-matching key must not reach the handler — that is what `.enter` means"
    );
}

#[test]
fn prevent_is_the_one_that_still_lowers() {
    // The control. Without it every assertion above is satisfied by refusing every modifier there
    // is, which would take away a capability that works — `.prevent` is a declaration made at
    // registration, which both tiers can make.
    let src = "## Component App\n\
       \n\
       State:\n\
      \x20   Let n be 0.\n\
       \n\
       Actions:\n\
      \x20   To add:\n\
      \x20       Set n to n + 1.\n\
       \n\
       View:\n\
      \x20   <form id=\"f\" on-submit.prevent={add}><input id=\"d\"></form>\n\
       \n\
       ## Main\n\
       Mount App.\n";
    assert_eq!(vm_outcome(src).error, None, "`.prevent` must still lower");
}

#[test]
fn an_unrecognised_modifier_is_a_key_name_on_both_tiers() {
    // ★ THE TIERS AGREE ABOUT WHAT A WORD MEANS — by there being only one place a word becomes a
    // modifier. The catalogue in `lgx/html.rs` classifies every `.qualifier` at PARSE TIME, before
    // either tier ever sees the program, so an unrecognised word is a compile error on both rather
    // than a key name one tier happens to honour and the other happens to gate on. A mistyped
    // modifier is a page that will not build, not a handler that quietly never fires.
    let src = page(
        "on-keydown.somethingnew={add}",
        "Press \"Enter\" in \"#d\".",
    );
    dom::reset();
    let tw = tw_outcome(&src).error;
    dom::reset();
    let vm = vm_outcome(&src).error;
    let names = "`.somethingnew` is not an event qualifier";
    assert!(
        tw.as_deref().is_some_and(|e| e.contains(names)),
        "tw: {tw:?}"
    );
    assert!(
        vm.as_deref().is_some_and(|e| e.contains(names)),
        "vm: {vm:?}"
    );

    // And a near miss of a KNOWN qualifier is a did-you-mean, on both tiers — the same catalogue,
    // the same suggestion, wherever the page is asked to run.
    let src = page("on-keydown.entr={add}", "Press \"Enter\" in \"#d\".");
    dom::reset();
    let tw = tw_outcome(&src).error;
    dom::reset();
    let vm = vm_outcome(&src).error;
    let guess = "did you mean `.enter`?";
    assert!(
        tw.as_deref().is_some_and(|e| e.contains(guess)),
        "tw: {tw:?}"
    );
    assert!(
        vm.as_deref().is_some_and(|e| e.contains(guess)),
        "vm: {vm:?}"
    );
}
