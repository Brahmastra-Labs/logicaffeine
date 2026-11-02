//! ★ LOCK — what a `{for each}` uses to identify a row, and what that costs.
//!
//! ★ MEASURED, NOT ASSUMED, AND IT IS A REAL HAZARD. Without `keyed by`, a row's instance — and so
//! its `State:` — is identified by its POSITION in the list. On a list of three counters with the
//! second bumped twice and the third once (`a:0 b:2 c:1`), dropping the FIRST item leaves the page
//! reading **`b:0 c:2`**: every survivor inherits its neighbour's state. Reordering does the same.
//! Both tiers agree, so this is the language's semantics rather than a tier defect, and NOTHING is
//! reported on either one.
//!
//! ★ WHY THIS IS A LOCK AND NOT A REFUSAL. A blanket refusal was written, wired, and measured: it
//! breaks three legitimate tests, including `a_row_in_a_loop_answers_its_own_click`, whose list
//! never changes. Positional identity is exactly right for a STATIC list, and the compiler cannot
//! tell a static list from one that reorders. So the hazard is pinned here rather than forbidden,
//! and the fix an author needs is named in the failure message.
//!
//! What this buys: the behaviour is now written down and cannot drift silently, and the keyed
//! spelling is proven to do what a key is for. `lgx_lower::unkeyed_stateful_loop` detects the
//! shape, for whatever diagnostic surface takes it up.

use logicaffeine_compile::compile::{tw_outcome, vm_outcome};
use logicaffeine_compile::semantics::dom;

/// Three counters, the second bumped twice and the third once, then the list is reshaped.
fn page(loop_head: &str, reshape: &str) -> String {
    format!(
        "## Component Tile (label: Text)\n\
         State:\n\x20   Let n be 0.\n\
         Actions:\n\x20   To bump:\n\x20       Set n to n + 1.\n\
         View:\n\x20   <li>{{label}}:{{n}}<button id=\"b-{{label}}\" on-click={{bump}}>+</button></li>\n\n\
         ## Component App\n\
         State:\n\x20   Let xs be [\"a\", \"b\", \"c\"].\n\
         Actions:\n\x20   To drop:\n\x20       Set xs to [\"b\", \"c\"].\n\
         \x20   To reorder:\n\x20       Set xs to [\"c\", \"b\", \"a\"].\n\
         View:\n\x20   <ul>{loop_head}<Tile label=\"{{x}}\"/>{{end}}</ul>\
         <button id=\"drop\" on-click={{drop}}>d</button>\
         <button id=\"reorder\" on-click={{reorder}}>r</button>\n\n\
         ## Main\nMount App.\nClick \"#b-b\".\nClick \"#b-b\".\nClick \"#b-c\".\n{reshape}"
    )
}

/// Both tiers, because a divergence here would be a second bug on top of the first.
fn agree(label: &str, src: &str) -> String {
    dom::reset();
    assert_eq!(
        tw_outcome(src).error,
        None,
        "`{label}` must run on the tree-walker"
    );
    let tw = dom::render();
    dom::reset();
    assert_eq!(vm_outcome(src).error, None, "`{label}` must run on the VM");
    let vm = dom::render();
    assert_eq!(tw, vm, "`{label}` differs across tiers:\n{src}");
    tw
}

const UNKEYED: &str = "{for each x in xs:}";
const KEYED: &str = "{for each x in xs, keyed by x:}";

/// With no reshape both spellings are correct — so a test that only ever renders once cannot see
/// the difference, which is why this hazard survived so long.
#[test]
fn a_static_list_keeps_each_rows_own_state_either_way() {
    for (label, head) in [("unkeyed", UNKEYED), ("keyed", KEYED)] {
        let out = agree(label, &page(head, ""));
        assert!(out.contains("a:0"), "`{label}`: a was never bumped: {out}");
        assert!(out.contains("b:2"), "`{label}`: b was bumped twice: {out}");
        assert!(out.contains("c:1"), "`{label}`: c was bumped once: {out}");
    }
}

/// ★ KEYED IS CORRECT ACROSS A RESHAPE — a key is what makes a row the same row.
#[test]
fn a_keyed_row_keeps_its_state_when_the_list_changes() {
    let dropped = agree("keyed drop", &page(KEYED, "Click \"#drop\".\n"));
    assert!(
        dropped.contains("b:2") && dropped.contains("c:1"),
        "keyed drop: {dropped}"
    );
    assert!(
        !dropped.contains("a:"),
        "the dropped row is gone: {dropped}"
    );

    let reordered = agree("keyed reorder", &page(KEYED, "Click \"#reorder\".\n"));
    assert!(
        reordered.contains("c:1") && reordered.contains("b:2") && reordered.contains("a:0"),
        "a reorder moves the rows and their state travels with them: {reordered}"
    );
}

/// ★ AND UNKEYED IS NOT — pinned as the HAZARD it is, with the fix named.
///
/// This asserts today's behaviour deliberately. If a future change gives an unkeyed loop stable
/// identity, this test fails and should be REPLACED by the keyed expectations above — a failure
/// here is good news, not a regression.
#[test]
fn an_unkeyed_row_takes_its_neighbours_state_when_the_list_changes() {
    let dropped = agree("unkeyed drop", &page(UNKEYED, "Click \"#drop\".\n"));
    assert!(
        dropped.contains("b:0") && dropped.contains("c:2"),
        "TODAY: dropping the first row shifts every state up one — b shows a's count and c shows \
         b's. Add `, keyed by x` to the loop and the rows keep their own. If this now reads \
         `b:2 c:1`, unkeyed identity was fixed and this test should be deleted: {dropped}"
    );

    let reordered = agree("unkeyed reorder", &page(UNKEYED, "Click \"#reorder\".\n"));
    assert!(
        reordered.contains("c:0") && reordered.contains("b:2") && reordered.contains("a:1"),
        "TODAY: a reorder leaves the state where it sat — position 0 keeps a's count whichever row \
         is drawn there: {reordered}"
    );
}
