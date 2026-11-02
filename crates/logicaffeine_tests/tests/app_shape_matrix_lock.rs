//! Every combination of the features a real page combines, on both tiers, compared.
//!
//! ★ THE BUGS THAT LIVE HERE ONLY EXIST IN COMBINATION. Each feature below has its own lock and
//! each of those locks was green while this file's subject was broken in 56 of its 64 shapes. A
//! single-instance test cannot see a one-to-many delivery bug; a list without a class toggle
//! cannot see a repaint that clears the list. So the axes are enumerated rather than sampled —
//! the point is the CROSS PRODUCT, and 64 pages cost less than one of the bugs did.
//!
//! Two defects were found by exactly this sweep and are pinned by it:
//!
//!   * `Publish` reached ONE subscriber. Every instance registers on the same
//!     `(body, "message:<topic>")` pair, and the dispatch table behind it holds one listener per
//!     pair, so the last instance to register answered and the rest heard nothing. Two rows and
//!     one `Publish` is the smallest page that shows it. Fixed by fanning out over the live
//!     instance registry in the dispatcher, which is also where `Raise "message:…"` arrives.
//!
//!   * A repaint of a row CLEARED THE LIST. `repaint_def` re-renders into the instance's recorded
//!     mount point and clears it first; for a row that mount point is the enclosing `<ul>`, shared
//!     with every sibling. A `class:` toggle has no patch site, so the turn repaints — and the
//!     other rows left the document. Fixed by redirecting a loop-placed component's repaint to the
//!     paint root, which is what a placed WRAPPER already did for the identical reason.
//!
//! The tree-walker is the spec. Any disagreement is the bytecode tier being wrong.

use logicaffeine_compile::compile::{tw_outcome, vm_outcome};
use logicaffeine_compile::semantics::dom;

const AXES: [&str; 6] = ["tree", "panel", "derived", "tile_store", "keyed", "watcher"];

/// One page, built from the six axes a bitmask selects.
///
/// The fixed parts are the ones that carry the interactions: a store with an action, a row
/// component with per-instance state that subscribes to a topic, and a page that publishes on it.
/// Each axis adds a feature that is independently locked elsewhere — a recursive component, a
/// wrapper taking `{children}`, a derived field driving `class:`, a row that also moves the store,
/// a keyed rather than positional loop, and a store watcher.
fn page(on: u32) -> String {
    let has = |i: u32| on & (1 << i) != 0;
    let (tree, panel, derived, tile_store, keyed, watcher) =
        (has(0), has(1), has(2), has(3), has(4), has(5));
    let mut s = String::from(
        "## Store B\nState:\n    Let m be 0.\nActions:\n    To play:\n        Set m to m + 1.\n\n",
    );
    if tree {
        s.push_str(
            "## Component Node (depth: Int)\nStyle:\n    .node { margin-left: 4px; }\n\
             View:\n    <li class=\"node\">d{depth}{if depth is greater than 0:}\
             <ul><Node depth={depth - 1}/></ul>{end}</li>\n\n",
        );
    }
    if panel {
        s.push_str(
            "## Component Panel (title: Text)\nStyle:\n    .panel { border: 1px solid; }\n\
             View:\n    <section class=\"panel\"><h3>{title}</h3>{children}</section>\n\n",
        );
    }
    s.push_str("## Component Tile (label: Text)\n");
    if tile_store {
        s.push_str("Use B.\n");
    }
    s.push_str("State:\n    Let hits be 0.\n");
    if derived {
        s.push_str("    Let loud always be hits is greater than 1.\n");
    }
    s.push_str("Actions:\n    To tap:\n        Set hits to hits + 1.\n");
    if tile_store {
        s.push_str("        Call B's play.\n");
    }
    s.push_str(
        "    When a message arrives on \"reset\" (m: Text):\n        Set hits to 0.\n\
         View:\n    <li class=\"tile\"",
    );
    if derived {
        s.push_str(" class:loud={loud}");
    }
    s.push_str(">{label}:{hits}<button id=\"t-{label}\" on-click={tap}>+</button></li>\n\n");
    if watcher {
        s.push_str(
            "## Component W\nUse B.\nState:\n    Let seen be 0.\n\
             Actions:\n    When B's m changes:\n        Set seen to seen + 1.\n\
             View:\n    <span id=\"w\">{seen}</span>\n\n",
        );
    }
    s.push_str(
        "## Component App\nUse B.\nState:\n    Let tiles be [\"a\", \"b\"].\n\
         \x20   Let showTree be true.\n\
         Actions:\n    To hideTree:\n        Set showTree to not showTree.\n\
         \x20   To broadcast:\n        Publish \"go\" on \"reset\".\nView:\n    <main>",
    );
    let each = if keyed {
        "{for each t in tiles, keyed by t:}"
    } else {
        "{for each t in tiles:}"
    };
    let rows = format!("<ul>{each}<Tile label=\"{{t}}\"/>{{end}}</ul>");
    if panel {
        s.push_str(&format!("<Panel title=\"Tiles\">{rows}</Panel>"));
    } else {
        s.push_str(&rows);
    }
    if tree {
        s.push_str("{if showTree:}<ul><Node depth={2}/></ul>{end}");
    }
    s.push_str("<p id=\"s\">{B's m}</p>");
    if watcher {
        s.push_str("<W/>");
    }
    s.push_str(
        "<button id=\"hide\" on-click={hideTree}>h</button>\
         <button id=\"reset\" on-click={broadcast}>r</button></main>\n\n\
         ## Main\nMount App.\n",
    );
    s
}

fn shape(on: u32) -> String {
    let parts: Vec<&str> = (0..6)
        .filter(|i| on & (1 << i) != 0)
        .map(|i| AXES[i as usize])
        .collect();
    if parts.is_empty() {
        "(bare)".into()
    } else {
        parts.join("+")
    }
}

/// Runs one page under one script on both tiers and reports a disagreement.
fn compare(on: u32, script: &str) -> Option<String> {
    let src = page(on) + script;
    dom::reset();
    let tw = tw_outcome(&src);
    let tw_html = dom::render();
    dom::reset();
    let vm = vm_outcome(&src);
    let vm_html = dom::render();
    match (tw.error.as_deref(), vm.error.as_deref()) {
        (None, None) if tw_html == vm_html => None,
        (None, None) => {
            let at = tw_html
                .bytes()
                .zip(vm_html.bytes())
                .position(|(a, b)| a != b)
                .unwrap_or(0);
            let lo = at.saturating_sub(60);
            Some(format!(
                "the two tiers drew different pages\n    tw …{}\n    vm …{}",
                &tw_html[lo..tw_html.len().min(at + 120)],
                &vm_html[lo..vm_html.len().min(at + 120)],
            ))
        }
        (a, b) if a.is_some() == b.is_some() => None,
        (a, b) => Some(format!(
            "one tier refused and the other did not\n    tw {a:?}\n    vm {b:?}"
        )),
    }
}

/// Each script exercises one interaction the axes above can reach.
const SCRIPTS: [(&str, &str); 7] = [
    ("as mounted", ""),
    ("one row tapped twice", "Click \"#t-a\".\nClick \"#t-a\".\n"),
    ("both rows tapped", "Click \"#t-a\".\nClick \"#t-b\".\n"),
    // The one-to-many delivery: two live subscribers, one `Publish`.
    (
        "a tap then a broadcast",
        "Click \"#t-a\".\nClick \"#t-b\".\nClick \"#reset\".\n",
    ),
    ("a broadcast with nothing tapped", "Click \"#reset\".\n"),
    (
        "a subtree removed and restored",
        "Click \"#t-a\".\nClick \"#hide\".\nClick \"#hide\".\n",
    ),
    (
        "everything, in order",
        "Click \"#t-a\".\nClick \"#t-b\".\nClick \"#hide\".\nClick \"#reset\".\nClick \"#t-a\".\n",
    ),
];

#[test]
fn every_shape_of_page_draws_the_same_on_both_tiers() {
    let mut broken: Vec<String> = Vec::new();
    for on in 0u32..64 {
        for (what, script) in SCRIPTS {
            if let Some(why) = compare(on, script) {
                broken.push(format!("  [{}] {what}: {why}", shape(on)));
            }
        }
    }
    assert!(
        broken.is_empty(),
        "{} of {} (page, script) pairs disagree between the tiers:\n{}",
        broken.len(),
        64 * SCRIPTS.len(),
        broken.join("\n"),
    );
}

/// Both tiers run the page and must draw exactly `expected`.
fn both_tiers(src: &str, expected: &str, note: &str) {
    dom::reset();
    let tw = tw_outcome(src);
    assert!(
        tw.error.is_none(),
        "the tree-walker refused the page: {:?}",
        tw.error
    );
    assert_eq!(dom::render(), expected, "tree-walker: {note}");
    dom::reset();
    let vm = vm_outcome(src);
    assert!(
        vm.error.is_none(),
        "the bytecode tier refused the page: {:?}",
        vm.error
    );
    assert_eq!(dom::render(), expected, "bytecode tier: {note}");
}

/// The delivery bug on its own, in the smallest page that shows it — so a regression names the
/// cause rather than pointing at a 64-way sweep.
#[test]
fn a_published_message_reaches_every_subscriber_not_just_the_last() {
    let src = "\
## Component Row (label: Text)
State:
    Let hits be 0.
Actions:
    To tap:
        Set hits to hits + 1.
    When a message arrives on \"reset\" (m: Text):
        Set hits to 0.
View:
    <li>{label}:{hits}<button id=\"t-{label}\" on-click={tap}>+</button></li>

## Component App
Actions:
    To broadcast:
        Publish \"go\" on \"reset\".
View:
    <main><ul>{for each t in [\"a\", \"b\"], keyed by t:}<Row label=\"{t}\"/>{end}</ul>\
<button id=\"r\" on-click={broadcast}>r</button></main>

## Main
Mount App.
Click \"#t-a\".
Click \"#t-b\".
Click \"#r\".
";
    let expected = "<body><main><ul><li>a:0<button id=\"t-a\">+</button></li>\
<li>b:0<button id=\"t-b\">+</button></li></ul><button id=\"r\">r</button></main></body>";
    both_tiers(src, expected, "both rows must hear one Publish");
}

/// The repaint bug on its own: a row whose turn cannot be patched must not take the list with it.
#[test]
fn a_rows_repaint_does_not_clear_its_siblings() {
    let src = "\
## Component Row (label: Text)
State:
    Let on be false.
Actions:
    To tap:
        Set on to true.
View:
    <li class=\"r\" class:lit={on}>{label}<button id=\"t-{label}\" on-click={tap}>+</button></li>

## Component App
View:
    <main><ul>{for each t in [\"a\", \"b\"]:}<Row label=\"{t}\"/>{end}</ul></main>

## Main
Mount App.
Click \"#t-a\".
";
    let expected = "<body><main><ul><li class=\"r lit\">a<button id=\"t-a\">+</button></li>\
<li class=\"r\">b<button id=\"t-b\">+</button></li></ul></main></body>";
    both_tiers(src, expected, "row b must survive row a's repaint");
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// A second family, around the parts of a page the first one does not reach: a form, a list of
// lists, lifecycle handlers, and two DIFFERENT components subscribing to one topic.
//
// ★ IT FOUND A THIRD BUG, AND A LOUD ONE. `When this component appears:` was inlined into the
// MOUNT function beside `When the page loads:` — and a mount only ever names the ROOT, so on the
// bytecode tier a PLACED child never ran it. `When this component disappears:` is driven by the
// unmount sweep, which walks every registry-keeping component, so the two ends of an instance's
// life were decided in different places: a child could say "out" having never said "in".
//
// No lock caught it because `event_vocabulary_lock`'s lifecycle tests read `output(src)`, which
// runs the TREE-WALKER alone. A tier gap is invisible to a test that only asks one tier.
// ─────────────────────────────────────────────────────────────────────────────────────────────

const FORM_AXES: [&str; 6] = [
    "bind",
    "nested",
    "lifecycle",
    "unmount",
    "timer",
    "two_subs",
];

fn form_page(on: u32) -> String {
    let has = |i: u32| on & (1 << i) != 0;
    let (bind, nested, lifecycle, unmount, timer, two_subs) =
        (has(0), has(1), has(2), has(3), has(4), has(5));
    let mut s = String::new();
    s.push_str("## Component Row (label: Text)\nState:\n    Let n be 0.\nActions:\n");
    s.push_str("    To bump:\n        Set n to n + 1.\n");
    s.push_str("    When a message arrives on \"wipe\" (m: Text):\n        Set n to 0.\n");
    if lifecycle {
        s.push_str("    When this component appears:\n        Show \"in {label}\".\n");
        s.push_str("    When this component disappears:\n        Show \"out {label}\".\n");
    }
    if timer {
        s.push_str("    Every 10 seconds:\n        Set n to n.\n");
    }
    s.push_str("View:\n    <li>{label}:{n}");
    // A structural `{if}` inside a row: the other way to reach a turn that cannot be patched.
    s.push_str("{if n is greater than 0:}<em>up</em>{end}");
    if nested {
        s.push_str("<ul>{for each c in [\"x\", \"y\"]:}<i>{c}</i>{end}</ul>");
    }
    s.push_str("<button id=\"b-{label}\" on-click={bump}>+</button></li>\n\n");
    if two_subs {
        s.push_str(
            "## Component Tally\nState:\n    Let wipes be 0.\n\
             Actions:\n    When a message arrives on \"wipe\" (m: Text):\n\
             \x20       Set wipes to wipes + 1.\nView:\n    <b id=\"tally\">{wipes}</b>\n\n",
        );
    }
    s.push_str(
        "## Component App\nState:\n    Let rows be [\"a\", \"b\"].\n    Let open be true.\n",
    );
    if bind {
        s.push_str("    Let draft be \"\".\n");
    }
    s.push_str(
        "Actions:\n    To toggle:\n        Toggle open.\n\
         \x20   To wipe:\n        Publish \"go\" on \"wipe\".\nView:\n    <main>",
    );
    if bind {
        s.push_str("<input id=\"d\" bind={draft}><p id=\"echo\">{draft}</p>");
    }
    s.push_str("<ul>{for each r in rows, keyed by r:}<Row label=\"{r}\"/>{end}</ul>");
    if unmount {
        s.push_str("{if open:}<ul><Row label=\"z\"/></ul>{end}");
    }
    if two_subs {
        s.push_str("<Tally/>");
    }
    s.push_str(
        "<button id=\"t\" on-click={toggle}>t</button>\
         <button id=\"w\" on-click={wipe}>w</button></main>\n\n## Main\nMount App.\n",
    );
    s
}

fn form_shape(on: u32) -> String {
    let parts: Vec<&str> = (0..6)
        .filter(|i| on & (1 << i) != 0)
        .map(|i| FORM_AXES[i as usize])
        .collect();
    if parts.is_empty() {
        "(bare)".into()
    } else {
        parts.join("+")
    }
}

const FORM_SCRIPTS: [(&str, &str); 6] = [
    ("as mounted", ""),
    ("one row bumped", "Click \"#b-a\".\n"),
    ("both rows bumped", "Click \"#b-a\".\nClick \"#b-b\".\n"),
    (
        "bump then wipe",
        "Click \"#b-a\".\nClick \"#b-b\".\nClick \"#w\".\n",
    ),
    (
        "the optional subtree toggled",
        "Click \"#b-a\".\nClick \"#t\".\nClick \"#t\".\n",
    ),
    (
        "everything, in order",
        "Click \"#b-a\".\nClick \"#b-b\".\nClick \"#t\".\nClick \"#w\".\nClick \"#b-a\".\n",
    ),
];

#[test]
fn every_form_shaped_page_behaves_the_same_on_both_tiers() {
    let mut broken: Vec<String> = Vec::new();
    for on in 0u32..64 {
        for (what, script) in FORM_SCRIPTS {
            let src = form_page(on) + script;
            dom::reset();
            let tw = tw_outcome(&src);
            let tw_html = dom::render();
            // Compared by LINE: the two tiers differ only in a trailing newline, which is the
            // harness's and not the page's.
            let tw_said: Vec<&str> = tw.output.lines().collect();
            dom::reset();
            let vm = vm_outcome(&src);
            let vm_html = dom::render();
            let vm_said: Vec<&str> = vm.output.lines().collect();
            let why = match (tw.error.as_deref(), vm.error.as_deref()) {
                (None, None) if tw_html == vm_html && tw_said == vm_said => continue,
                (None, None) if tw_html != vm_html => {
                    let at = tw_html
                        .bytes()
                        .zip(vm_html.bytes())
                        .position(|(a, b)| a != b)
                        .unwrap_or(0);
                    let lo = at.saturating_sub(60);
                    format!(
                        "the two tiers drew different pages\n    tw …{}\n    vm …{}",
                        &tw_html[lo..tw_html.len().min(at + 120)],
                        &vm_html[lo..vm_html.len().min(at + 120)],
                    )
                }
                (None, None) => {
                    format!("the two tiers said different things\n    tw {tw_said:?}\n    vm {vm_said:?}")
                }
                (a, b) if a.is_some() == b.is_some() => continue,
                (a, b) => {
                    format!("one tier refused and the other did not\n    tw {a:?}\n    vm {b:?}")
                }
            };
            broken.push(format!("  [{}] {what}: {why}", form_shape(on)));
        }
    }
    assert!(
        broken.is_empty(),
        "{} of {} (page, script) pairs disagree between the tiers:\n{}",
        broken.len(),
        64 * FORM_SCRIPTS.len(),
        broken.join("\n"),
    );
}

/// The lifecycle bug on its own: a PLACED child must announce its arrival, not only the root.
#[test]
fn a_placed_child_appears_on_both_tiers() {
    let src = "\
## Component Row (label: Text)
Actions:
    When this component appears:
        Show \"in {label}\".
View:
    <li>{label}</li>

## Component App
View:
    <main><ul>{for each r in [\"a\", \"b\"]:}<Row label=\"{r}\"/>{end}</ul></main>

## Main
Mount App.
";
    dom::reset();
    let tw = tw_outcome(src);
    assert_eq!(tw.output.lines().collect::<Vec<_>>(), ["in a", "in b"]);
    dom::reset();
    let vm = vm_outcome(src);
    assert_eq!(
        vm.output.lines().collect::<Vec<_>>(),
        ["in a", "in b"],
        "a child placed by a loop must run `appears` on the bytecode tier too",
    );
}

/// And an instance must never leave without having arrived — the asymmetry that hid the bug.
#[test]
fn nothing_disappears_without_having_appeared() {
    let src = "\
## Component Row
Actions:
    When this component appears:
        Show \"in\".
    When this component disappears:
        Show \"out\".
View:
    <li>r</li>

## Component App
State:
    Let showing be true.
Actions:
    To hide:
        Toggle showing.
View:
    <button id=\"go\" on-click={hide}>x</button>
    {if showing:}<Row/>{end}

## Main
Mount App.
Click \"#go\".
";
    for said in [
        {
            dom::reset();
            tw_outcome(src).output
        },
        {
            dom::reset();
            vm_outcome(src).output
        },
    ] {
        assert_eq!(said.lines().collect::<Vec<_>>(), ["in", "out"]);
    }
}

/// ★ MOVING THE ROUTE OWES `When the route changes:` A DELIVERY — on both tiers.
///
/// `Navigate to` records the new path and nothing else: the handler registered with `onRoute` is
/// what a real browser's POPSTATE fires, and a pushState fires nothing at all. The reference tier
/// has always known that — it notes the change and delivers it when the turn unwinds — while the
/// bytecode tier called `navigate` and stopped. So a page that navigated ITSELF never ran its own
/// route handler on the one tier a visitor uses.
///
/// Measured before the fix: the tree-walker counted 2 after two navigations, the bytecode tier 0.
/// `trigger_tier_lock` called `RouteChanges` "Lowered" throughout — which was true, and is the
/// difference this file exists to catch: it compiled, and it did not happen.
#[test]
fn navigating_runs_the_route_handler_on_both_tiers() {
    let app = "\
## Component App
State:
    Let seen be 0.
Actions:
    When the route changes:
        Set seen to seen + 1.
    To go:
        Navigate to \"/about\".
View:
    <p id=\"s\">{seen}</p><button id=\"b\" on-click={go}>go</button>

## Main
Mount App.
";
    // Mounting alone is not a route change — the page did not move.
    both_tiers(
        app,
        "<body><p id=\"s\">0</p><button id=\"b\">go</button></body>",
        "mounting is not a navigation",
    );
    both_tiers(
        &format!("{app}Click \"#b\".\n"),
        "<body><p id=\"s\">1</p><button id=\"b\">go</button></body>",
        "one navigation is one route change",
    );
    // Twice, because a handler that fires once could be a mount-time accident.
    both_tiers(
        &format!("{app}Click \"#b\".\nClick \"#b\".\n"),
        "<body><p id=\"s\">2</p><button id=\"b\">go</button></body>",
        "every navigation is a route change",
    );
}
