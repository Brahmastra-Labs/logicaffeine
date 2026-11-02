//! ★ LOCK — the reactive-semantics problems that are hard in every framework.
//!
//! These are the ones that forced topological scheduling into Vue, Solid and MobX, and the ones a
//! per-feature test never reaches because each needs a SHAPE of dependency rather than a feature.
//!
//! 1. **Glitches (the diamond).** `b` and `c` both derive from `a`; `d` derives from both. A naive
//!    propagation recomputes `d` when `b` changes and again when `c` does, and the first of those
//!    sees a NEW `b` beside a STALE `c` — an inconsistent value that briefly reaches the page. The
//!    arithmetic here is chosen so a glitch cannot hide: `d = 2a + (a+1) = 3a+1`, which no pairing
//!    of a stale and a fresh operand produces.
//!
//! 2. **Read-after-write inside a turn.** A turn reads state into locals, runs, and writes back at
//!    the END — so whether a field sees its own write mid-turn is a real question, not an obvious
//!    one.
//!
//! 3. **Reading a DERIVED field inside an action.** ★ This was broken on the browser tier and is
//!    the reason this file exists. Only non-derived fields were bound as a turn's locals, so an
//!    action that so much as read one — `If remaining is 0:` — failed at run time with "Undefined
//!    variable" in a browser, while the reference tier ran it. The filter was right for what a turn
//!    WRITES BACK and wrong for what a turn can SEE.
//!
//! ★ THE SEMANTICS, MEASURED RATHER THAN CHOSEN: a derived read inside a turn holds its
//! TURN-START value. `Set n to 10. Set seen to dbl.` leaves `seen` at the old `dbl` and the field
//! itself finishing at the new one. That is what the reference tier does and it follows from the
//! transactional model, so the browser tier binds derived fields once, at the top of the turn, in
//! DECLARATION ORDER — which is also what lets a derived read another derived.

use logicaffeine_compile::compile::{tw_outcome, vm_outcome};
use logicaffeine_compile::semantics::dom;

fn agree(label: &str, src: &str) -> String {
    dom::reset();
    assert_eq!(
        tw_outcome(src).error,
        None,
        "`{label}` must run on the tree-walker:\n{src}"
    );
    let tw = dom::render();
    dom::reset();
    assert_eq!(
        vm_outcome(src).error,
        None,
        "`{label}` must run on the VM:\n{src}"
    );
    let vm = dom::render();
    assert_eq!(tw, vm, "`{label}` differs across tiers:\n{src}");
    tw
}

/// ★ NO GLITCH. `d` is only ever `3a+1`; a stale-with-fresh pairing would give `2a+(a_old+1)` or
/// `2a_old+(a+1)`, neither of which equals it for the values used.
#[test]
fn a_diamond_of_derived_fields_never_shows_an_inconsistent_value() {
    let out = agree(
        "diamond",
        "## Component App\nState:\n\x20   Let a be 2.\n\x20   Let b always be a * 2.\n\
         \x20   Let c always be a + 1.\n\x20   Let d always be b + c.\n\
         Actions:\n\x20   To go:\n\x20       Set a to 5.\n\
         View:\n\x20   <p id=\"p\">{a}|{b}|{c}|{d}</p><button id=\"x\" on-click={go}>x</button>\n\n\
         ## Main\nMount App.\nClick \"#x\".\n",
    );
    assert!(
        out.contains(">5|10|6|16<"),
        "d must be 3a+1 = 16 for a = 5 — a glitch would show 2a+(a_old+1)=13 or 2a_old+(a+1)=10: \
         {out}"
    );
}

/// A turn sees its own write to a plain field.
#[test]
fn a_turn_reads_back_what_it_just_wrote() {
    let out = agree(
        "read after write",
        "## Component App\nState:\n\x20   Let n be 1.\n\x20   Let seen be 0.\n\
         Actions:\n\x20   To go:\n\x20       Set n to 10.\n\x20       Set seen to n.\n\
         View:\n\x20   <p id=\"p\">{n}|{seen}</p><button id=\"x\" on-click={go}>x</button>\n\n\
         ## Main\nMount App.\nClick \"#x\".\n",
    );
    assert!(
        out.contains(">10|10<"),
        "the turn sees its own write: {out}"
    );
}

/// ★ THE BUG THIS FILE WAS WRITTEN FOR. Reading a derived field in an action must WORK, and must
/// give the same answer on both tiers.
#[test]
fn an_action_can_read_a_derived_field() {
    let out = agree(
        "derived in an action",
        "## Component App\nState:\n\x20   Let n be 1.\n\x20   Let dbl always be n * 2.\n\
         \x20   Let seen be 0.\n\
         Actions:\n\x20   To go:\n\x20       Set n to 10.\n\x20       Set seen to dbl.\n\
         View:\n\x20   <p id=\"p\">{n}|{dbl}|{seen}</p><button id=\"x\" on-click={go}>x</button>\n\n\
         ## Main\nMount App.\nClick \"#x\".\n",
    );
    assert!(
        out.contains(">10|20|2<"),
        "the derived FIELD ends at 20, and the value the turn READ is its turn-start 2 — a turn is \
         a transaction, so it sees the state it began with: {out}"
    );
}

/// A derived may read another derived, inside an action — which only works if they are bound in
/// declaration order.
#[test]
fn a_derived_may_read_another_derived_inside_an_action() {
    let out = agree(
        "derived chain in an action",
        "## Component App\nState:\n\x20   Let a be 2.\n\x20   Let b always be a * 2.\n\
         \x20   Let c always be b + 1.\n\x20   Let seen be 0.\n\
         Actions:\n\x20   To go:\n\x20       Set seen to c.\n\x20       Set a to 5.\n\
         View:\n\x20   <p id=\"p\">{a}|{b}|{c}|{seen}</p><button id=\"x\" on-click={go}>x</button>\n\n\
         ## Main\nMount App.\nClick \"#x\".\n",
    );
    assert!(
        out.contains(">5|10|11|5<"),
        "c = b+1 = 5 at turn start, and 11 after: {out}"
    );
}

/// The most ordinary sentence of all: branch on a derived.
#[test]
fn an_action_can_branch_on_a_derived_field() {
    let out = agree(
        "branch on derived",
        "## Component App\nState:\n\x20   Let xs be [1, 2].\n\
         \x20   Let total always be length of xs.\n\x20   Let msg be \"-\".\n\
         Actions:\n\x20   To go:\n\x20       If total is greater than 1:\n\
         \x20           Set msg to \"many\".\n\
         View:\n\x20   <p id=\"p\">{total}|{msg}</p><button id=\"x\" on-click={go}>x</button>\n\n\
         ## Main\nMount App.\nClick \"#x\".\n",
    );
    assert!(
        out.contains(">2|many<"),
        "`If total is greater than 1:` must run: {out}"
    );
}

/// A derived that reads a STORE recomputes when the store moves.
#[test]
fn a_derived_reading_a_store_recomputes_when_the_store_moves() {
    let out = agree(
        "derived over a store",
        "## Store S\nState:\n\x20   Let v be 1.\nActions:\n\x20   To bump:\n\x20       Set v to v + 1.\n\n\
         ## Component App\nUse S.\nState:\n\x20   Let twice always be S's v * 2.\n\
         Actions:\n\x20   To go:\n\x20       Call S's bump.\n\
         View:\n\x20   <p id=\"p\">{S's v}|{twice}</p><button id=\"x\" on-click={go}>x</button>\n\n\
         ## Main\nMount App.\nClick \"#x\".\n",
    );
    assert!(
        out.contains(">2|4<"),
        "the derived follows the store: {out}"
    );
}

/// A derived may be the thing a loop iterates.
#[test]
fn a_derived_may_be_a_loops_iterable() {
    let out = agree(
        "derived iterable",
        "## Component App\nState:\n\x20   Let xs be [1, 2].\n\x20   Let ys always be xs.\n\
         Actions:\n\x20   To go:\n\x20       Set xs to [7, 8, 9].\n\
         View:\n\x20   <ul>{for each y in ys:}<li>{y}</li>{end}</ul>\
         <button id=\"x\" on-click={go}>x</button>\n\n\
         ## Main\nMount App.\nClick \"#x\".\n",
    );
    assert!(
        out.contains("<li>7</li><li>8</li><li>9</li>"),
        "the loop follows the derived: {out}"
    );
}
