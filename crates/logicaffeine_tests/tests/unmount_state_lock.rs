//! ★ LOCK — an instance that leaves the page takes its state with it.
//!
//! ★ A TIER DIVERGENCE ON THE TIER THAT SHIPS. A `{if on:}<Tile/>{end}` toggled off and back on
//! came back showing its PREVIOUS count in a browser and a fresh one on the reference tier —
//! measured at 2 versus 0. The tree-walker destroys the instance record; the bytecode tier kept
//! `__S_<C>_<field>` keyed by instance, nothing noticed the instance had gone, and a remount under
//! the same key found the old value waiting. A page that hides and re-shows a panel served stale
//! data, and every in-process test of the reference tier said it was fine.
//!
//! ★ WHAT IT TOOK, AND THE TWO WRONG TURNS ON THE WAY. The registry that answers "was this drawn
//! in the last paint" existed only for components with a timer, a `Disappears` handler or a store
//! watcher, and the sweep over it only FIRED HANDLERS — it never dropped state. Extending it:
//!
//! - Including a `## Store` broke twenty-one tests at once. A store has state and no view, so it
//!   is never drawn, so the sweep called it gone and wiped it every paint. **A predicate that is
//!   nearly right is worse than one that is obviously wrong**, because it fails only where the two
//!   ideas it conflates come apart. Only a mountable component keeps a registry.
//! - REMOVING the key is refused by the wasm backend ("unsupported remove of a non-Int value"),
//!   the same family of limit that forbids iterating a map. Writing the `State:` initializer back
//!   is what a fresh mount produces anyway, so the observable result is identical and it lowers
//!   everywhere.
//!
//! Four things had to agree about which components are tracked — the registry declarations, the
//! per-paint stamp, the sweep, and the pass counter — and they were separate conditions. They ask
//! `keeps_instance_registry` now, because the moment one said yes while the stamp said no, the
//! sweep saw every LIVE instance as gone.

use logicaffeine_compile::compile::{tw_outcome, vm_outcome};
use logicaffeine_compile::semantics::dom;

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
    assert_eq!(
        tw, vm,
        "`{label}` differs across tiers — and the browser is the VM, so a difference here is what \
         a visitor sees:\n{src}"
    );
    tw
}

const TOGGLE: &str = "\
## Component Tile
State:
    Let n be 0.
Actions:
    To bump:
        Set n to n + 1.
View:
    <span id=\"t\">{n}<button id=\"b\" on-click={bump}>+</button></span>

## Component App
State:
    Let on be true.
Actions:
    To toggle:
        Set on to not on.
View:
    <main>{if on:}<Tile/>{end}<button id=\"g\" on-click={toggle}>t</button></main>

## Main
Mount App.
";

/// The teeth: WITHOUT the toggle the count really is 2, so "shows 0" cannot pass by the bumps
/// having failed to register.
#[test]
fn the_bumps_really_do_count_while_the_instance_is_on_the_page() {
    let out = agree(
        "bumped, still mounted",
        &format!("{TOGGLE}Click \"#b\".\nClick \"#b\".\n"),
    );
    assert!(
        out.contains(">2<"),
        "two clicks count while it is on the page: {out}"
    );
}

/// ★ THE BUG. Hide it, show it again — the new instance starts from its `State:` initializer.
#[test]
fn a_remounted_instance_starts_fresh() {
    let out = agree(
        "unmount then remount",
        &format!("{TOGGLE}Click \"#b\".\nClick \"#b\".\nClick \"#g\".\nClick \"#g\".\n"),
    );
    assert!(
        out.contains(">0<") && !out.contains(">2<"),
        "a component that left the page and came back is a NEW instance and shows its initial \
         state, not the count the old one had: {out}"
    );
}

/// And it still counts after coming back — a reset that also broke the handler would satisfy the
/// test above.
#[test]
fn a_remounted_instance_still_works() {
    let out = agree(
        "remount then bump",
        &format!("{TOGGLE}Click \"#b\".\nClick \"#g\".\nClick \"#g\".\nClick \"#b\".\n"),
    );
    assert!(
        out.contains(">1<"),
        "the remounted instance counts from its own zero: {out}"
    );
}

/// ★ A STORE'S CHANGE REACHES THE PAGE EVEN WHEN THE CALLER ALSO CHANGED ITS OWN STATE.
///
/// A separate bug this test found on its way in, present on BOTH tiers before today: an action
/// that calls a store action AND sets one of its own fields showed the local change and NOT the
/// store's. Measured — `Call Counter's hit.` alone counted to 2; the same action with
/// `Set n to n + 1.` beside it left the store reading 0 while the local field counted correctly.
///
/// The cause: a store's fields belong to the STORE, not to the component whose action called it, so
/// a PATCH — which redraws the sites this component owns — never touches the site showing
/// `{Counter's hits}`. A turn which changes nothing of its own repaints anyway, because its empty
/// dirty set forces a repaint; a turn which also changed itself patches, and must repaint what the
/// store moved as well.
///
/// A store is also NOT swept as an instance: it has state and no view, so "was it drawn" is not a
/// question about it, and treating it as one wiped every store's state on the first paint.
#[test]
fn a_stores_change_reaches_the_page_when_the_caller_also_changed_itself() {
    let out = agree(
        "store across paints",
        "## Store Counter\nState:\n\x20   Let hits be 0.\n\
         Actions:\n\x20   To hit:\n\x20       Set hits to hits + 1.\n\n\
         ## Component App\nUse Counter.\nState:\n\x20   Let n be 0.\n\
         Actions:\n\x20   To go:\n\x20       Call Counter's hit.\n\x20       Set n to n + 1.\n\
         View:\n\x20   <p id=\"p\">{Counter's hits}|{n}</p>\
         <button id=\"a\" on-click={go}>a</button>\n\n\
         ## Main\nMount App.\nClick \"#a\".\nClick \"#a\".\n",
    );
    assert!(
        out.contains(">2|2<"),
        "a store accumulates across paints — it is not an instance that can leave the page: {out}"
    );
}

/// ★ A TIMER DOES NOT RUN FOR A COMPONENT THAT IS OFF THE PAGE, and the proof is written so it
/// cannot pass by accident.
///
/// The obvious test — unmount, remount, expect the counter at 0 — passes whether or not the timer
/// kept firing, because the remount resets that counter anyway. So the ticker writes into a STORE,
/// which outlives the instance: mounted for two seconds and away for five, the store must read 2.
/// The value a leak would produce is 7, and `the_leak_value_is_refuted` names it.
fn ticker_page(clicks: &str, expect: &str) -> String {
    format!(
        "## Store Log\nState:\n\x20   Let ticks be 0.\n\
         Actions:\n\x20   To tick:\n\x20       Set ticks to ticks + 1.\n\n\
         ## Component Ticker\nUse Log.\nState:\n\x20   Let n be 0.\n\
         Actions:\n\x20   Every 1 seconds:\n\x20       Set n to n + 1.\n\x20       Call Log's tick.\n\
         View:\n\x20   <span id=\"t\">{{n}}</span>\n\n\
         ## Component App\nUse Log.\nState:\n\x20   Let on be true.\n\
         Actions:\n\x20   To toggle:\n\x20       Set on to not on.\n\
         View:\n\x20   <main>{{if on:}}<Ticker/>{{end}}<span id=\"L\">{{Log's ticks}}</span>\
         <button id=\"g\" on-click={{toggle}}>t</button></main>\n\n\
         ## Test: what the clock did\n\x20   Mount App.\n{clicks}\
         \x20   Expect that the text of \"#L\" is equal to \"{expect}\".\n"
    )
}

fn verdict(src: &str, all_tiers: bool) -> logicaffeine_compile::testrun::Verdict {
    let cfg = logicaffeine_compile::testrun::TestConfig {
        all_tiers,
        ..Default::default()
    };
    logicaffeine_compile::testrun::run_tests(src, &cfg)
        .expect("the suite parses")
        .outcomes[0]
        .verdict
        .clone()
}

const AWAY: &str = "\x20   Advance time by 2 seconds.\n\x20   Click \"#g\".\n\
                    \x20   Advance time by 5 seconds.\n\x20   Click \"#g\".\n";

#[test]
fn a_timer_does_not_tick_while_its_component_is_off_the_page() {
    use logicaffeine_compile::testrun::Verdict;
    for all in [false, true] {
        assert!(
            matches!(verdict(&ticker_page(AWAY, "2"), all), Verdict::Passed),
            "two seconds mounted, five away — only the mounted seconds tick (all_tiers={all})"
        );
    }
}

/// The teeth. If the timer DID keep running while unmounted the store would read 7, so this
/// failing is what makes the test above mean something.
#[test]
fn the_leak_value_is_refuted() {
    use logicaffeine_compile::testrun::Verdict;
    assert!(
        !matches!(verdict(&ticker_page(AWAY, "7"), false), Verdict::Passed),
        "7 is what a leaking timer would produce — if this passes, the test above proves nothing"
    );
}

/// ★ AND THE PAGE SURVIVES THE TICK. A store change repaints, and repainting the COMPONENT that
/// called the store clears its mount point — which for a placed child is its PARENT's container,
/// taking the parent's other children with it. `<Ticker/>` beside `<button id="g">` in one `<main>`
/// lost the button, and the next click reported "No element matches". Caught by the suite's own
/// F18 tier-differential, on a bug written while fixing a different one.
#[test]
fn a_store_change_from_a_child_leaves_its_siblings_alone() {
    use logicaffeine_compile::testrun::Verdict;
    assert!(
        matches!(verdict(&ticker_page(AWAY, "2"), true), Verdict::Passed),
        "the button beside the ticker must still be clickable after a tick moved the store"
    );
}
