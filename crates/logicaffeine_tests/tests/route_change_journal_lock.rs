//! ★ LOCK — `Navigate to`/`Go back` starts the route-change turn exactly ONCE. ★
//!
//! A route change is noted once and delivered once: a delivery that did not consume the note it
//! answered would leave it pending, and the next turn's unwind would deliver it again — one
//! `Navigate` running a `When the route changes:` handler twice and journaling it twice. This lock
//! holds the count.

use logicaffeine_compile::compile::tw_outcome;
use logicaffeine_compile::semantics::{dom, world};

const PAGE: &str = "## Component App\n\
    State:\n    Let note be \"idle\".\n\
    Actions:\n    When the route changes:\n        Set note to \"routed\".\n\
    View:\n    <main><p id=\"s\">{note}</p></main>\n\n\
    ## Main\nMount App.\nNavigate to \"/other\".\n";

/// The `When the route changes:` handler journals under the synthesized event-handler action name
/// (`"event"`, same as any other `When …:` trigger) — counting THESE entries, rather than entries
/// that happen to CHANGE a field, is what tells "the handler ran N times" apart from "the Nth run
/// happened to set the same value again" (a second, correctly-single, `Go back` returning to a route
/// already-`"routed"` writes no new change but is still one real turn).
fn route_change_turns<'a>(journal: &'a [world::JournalEntry]) -> Vec<&'a world::JournalEntry> {
    journal.iter().filter(|e| e.action == "event").collect()
}

#[test]
fn one_navigate_journals_its_route_change_turn_exactly_once() {
    dom::reset();
    world::reset();
    let out = tw_outcome(PAGE);
    assert_eq!(out.error, None, "the page must run cleanly: {out:?}");
    let journal = world::journal();
    let route_turns = route_change_turns(&journal);
    assert_eq!(
        route_turns.len(),
        1,
        "one `Navigate` must start the route-change turn exactly once, not {}: {:?}",
        route_turns.len(),
        journal
            .iter()
            .map(|e| (&e.instance, &e.action, &e.changes))
            .collect::<Vec<_>>()
    );
    assert!(
        dom::render().contains("routed"),
        "the route change must still have run at all"
    );
}

/// `Go back` is a second door to a route change, with its own fixture so a delivery that consumes
/// its note on only one verb is still caught.
/// `Go back` is an ACTION-role statement (`action_verb_tier_lock`'s own partition — unlike
/// `Navigate`, it does not compile as a bare top-level driving statement), so it is driven here from
/// inside a component action, wired to a click, exactly as an author would write it.
#[test]
fn going_back_journals_its_route_change_turn_exactly_once() {
    let page = "## Component App\n\
        State:\n    Let note be \"idle\".\n\
        Actions:\n    When the route changes:\n        Set note to \"routed\".\n    \
        To back:\n        Go back.\n\
        View:\n    <main><p id=\"s\">{note}</p><button id=\"back\" on-click={back}>b</button></main>\n\n\
        ## Main\nMount App.\nNavigate to \"/a\".\nClick \"#back\".\n";
    dom::reset();
    world::reset();
    let out = tw_outcome(page);
    assert_eq!(out.error, None, "the page must run cleanly: {out:?}");
    let journal = world::journal();
    let route_turns = route_change_turns(&journal).len();
    // Two route changes in this page (the initial `Navigate`, then `Go back` returning from it) —
    // each must start the turn exactly once, so exactly 2, never 4. (The second turn writes no NEW
    // change — the route it returns to already set `note` to `"routed"` — so counting by action
    // name rather than by an actual field change is what catches it.)
    assert_eq!(
        route_turns,
        2,
        "two route changes (one `Navigate`, one `Go back`) must each start their turn exactly \
         once, not {route_turns}: {:?}",
        journal
            .iter()
            .map(|e| (&e.instance, &e.action, &e.changes))
            .collect::<Vec<_>>()
    );
}
