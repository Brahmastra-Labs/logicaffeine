//! ★ LOCK — R3(e): every lowered turn notes itself in the journal, agreeing with the reference
//! tier's own `journal_push` — an action, a `When …:` real-listener handler, and a bound input's
//! write, in the reference's own commit order. ★
//!
//! `run_tests`/`testrun.rs` only ever proved this by ABSENCE: it hard-filters `Axis::Journal` out
//! of a page test's tier comparison, with a doc comment tying that filter to this exact gap
//! (R3). An absent divergence proves nothing on its own — it is
//! indistinguishable from a comparison that silently never ran. This lock calls
//! `page_host::compare_tests` directly, UNFILTERED, and reads the journal axis itself: the
//! positive proof `testrun.rs`'s exclusion could never give.

use logicaffeine_compile::page_host::{compare_tests, Axis, AxisDivergence, PageTierKind, TierRun};

const BUDGET: u64 = 50_000_000;

/// `source`'s one page `## Test`, run on the reference tier and the lowered VM, with every axis
/// the two disagree on — the journal axis included, unlike every path `run_tests` offers.
fn divergent_axes(source: &str) -> Vec<AxisDivergence> {
    let all = compare_tests(source, &[PageTierKind::LoweredVm], 0, BUDGET, None)
        .expect("the page scenario parses");
    assert_eq!(all.len(), 1, "exactly one `## Test` that works a page");
    let t = match &all[0].candidates[0].1 {
        TierRun::Refused { reason, .. } => panic!("the lowered tier must not refuse: {reason}"),
        TierRun::Traced(t) => t,
    };
    all[0].reference.compare(t).axes
}

fn journal_divergence(source: &str) -> Vec<AxisDivergence> {
    divergent_axes(source)
        .into_iter()
        .filter(|a| a.axis == Axis::Journal)
        .collect()
}

const COUNTER: &str = "## Component App\n\
    State:\n    Let n be 0.\n\
    Actions:\n    To bump:\n        Set n to n + 1.\n    To reset:\n        Set n to 0.\n\
    View:\n    <main>\
        <p id=\"n\">{n}</p>\
        <button id=\"b\" on-click={bump}>x</button>\
        <button id=\"r\" on-click={reset}>y</button>\
    </main>\n\n\
    ## Test: clicking is a turn like any other\n\
    \x20   Mount App.\n\
    \x20   Click \"#b\".\n\
    \x20   Click \"#b\".\n\
    \x20   Click \"#r\".\n\
    \x20   Expect that the text of \"#n\" is equal to \"0\".\n";

#[test]
fn a_lowered_click_journals_the_actions_own_name_in_order() {
    let journal_axis = journal_divergence(COUNTER);
    assert!(
        journal_axis.is_empty(),
        "the journal axis must agree once every lowered turn notes itself: {journal_axis:?}"
    );
}

#[test]
fn a_bound_inputs_write_journals_as_bind() {
    const ECHO: &str = "## Component App\n\
        State:\n    Let text be \"\".\n\
        View:\n    <main><input id=\"i\" bind={text}></main>\n\n\
        ## Test: typing is a turn too\n\
        \x20   Mount App.\n\
        \x20   Type \"hi\" into \"#i\".\n\
        \x20   Expect that the value of \"#i\" is equal to \"hi\".\n";
    let journal_axis = journal_divergence(ECHO);
    assert!(
        journal_axis.is_empty(),
        "a bound write's `bind` entry must agree too: {journal_axis:?}"
    );
}

#[test]
fn a_same_component_field_change_handler_stays_unjournaled() {
    // `When <field> changes:` with no `store:` runs inlined, INSIDE the triggering turn, before
    // that turn's `noteTurn` — never a turn of its own. A page that has one must still agree on
    // `Axis::Journal`, not merely fail to diverge because nothing exercised the path.
    const WATCHED: &str = "## Component App\n\
        State:\n    Let n be 0.\n    Let doubled be 0.\n\
        Actions:\n    To bump:\n        Set n to n + 1.\n    When n changes:\n        Set doubled to n * 2.\n\
        View:\n    <main>\
            <p id=\"d\">{doubled}</p>\
            <button id=\"b\" on-click={bump}>x</button>\
        </main>\n\n\
        ## Test: a bump changes a derived field through a watcher\n\
        \x20   Mount App.\n\
        \x20   Click \"#b\".\n\
        \x20   Click \"#b\".\n\
        \x20   Expect that the text of \"#d\" is equal to \"4\".\n";
    let journal_axis = journal_divergence(WATCHED);
    assert!(
        journal_axis.is_empty(),
        "the watcher must not add its own entry: {journal_axis:?}"
    );
}
