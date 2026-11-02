//! ★ RATCHET — an update costs what CHANGED, not what is on the page. ★
//!
//! Stage A repaints by clearing the mount and rebuilding the subtree. Every parity lock passes
//! against it, because the resulting TREE is right — and it is O(the whole view) for a change to
//! one field. Typing one character into a draft box beside a thousand-row list destroys and
//! rebuilds a thousand rows, per keystroke.
//!
//! That is not only slow. Every node the visitor was interacting with is a NEW node afterwards:
//! focus, selection, scroll position and any listener a host attached are gone. The reconciler
//! hides most of it today (T4.3a), which is exactly why a correctness lock cannot see the problem
//! and a COUNT can.
//!
//! ★ THE MEASURE IS OPS, NOT TIME. A patch count is deterministic — it says the same thing on a
//! loaded machine as on an idle one, and the same thing on every tier. A timing ratchet under a
//! 15,000-test parallel run reports the machine; this campaign has the scars.
//!
//! ★ THE CEILINGS ARE ABSOLUTE, NOT RELATIVE. "Fewer ops than a full repaint" would be satisfied
//! by a repaint that got 1% cheaper. The question a visitor's browser asks is how many operations
//! it must perform for one keystroke, and the answer has to be a small number that does not grow
//! with the page.

use logicaffeine_compile::compile::tw_outcome;
use logicaffeine_compile::semantics::dom::{self, DomOp};

/// Run `src` and return the ops recorded AFTER the first paint — the update, alone.
///
/// The first paint is O(the view) by nature and is not what this file is about. The same program
/// cut after its `Mount` line records exactly the first paint, op for op, so what the whole run
/// records past that prefix is one turn's work and nothing else.
fn update_ops(src: &str) -> Vec<DomOp> {
    let cut = src
        .find("\nMount ")
        .and_then(|at| src[at + 1..].find('\n').map(|end| at + 1 + end + 1))
        .unwrap_or_else(|| panic!("the page mounts:\n{src}"));
    dom::reset();
    let mounted = tw_outcome(&src[..cut]);
    assert_eq!(mounted.error, None, "the page must mount cleanly:\n{src}");
    let first_paint = dom::op_log();
    dom::reset();
    let out = tw_outcome(src);
    assert_eq!(out.error, None, "the page must run cleanly:\n{src}");
    let log = dom::op_log();
    assert_eq!(
        &log[..first_paint.len()],
        first_paint.as_slice(),
        "the turn starts from the first paint's document:\n{src}"
    );
    let update = log[first_paint.len()..].to_vec();
    assert!(
        !update.is_empty(),
        "expected the turn to reach the document:\n{src}"
    );
    update
}

/// A page with `rows` list items, one draft field, and a `type` action that touches only `draft`.
///
/// The list and the draft are INDEPENDENT: nothing the action writes is read by any row. So a
/// correct fine-grained update touches the draft's site and nothing else, whatever `rows` is.
fn page(rows: usize) -> String {
    let items: String = (1..=rows)
        .map(|i| format!("\"r{i}\""))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "## Component App\n\
         \n\
         State:\n\
        \x20   Let draft be \"\".\n\
        \x20   Let rows be [{items}].\n\
         \n\
         Actions:\n\
        \x20   To edit:\n\
        \x20       Set draft to \"x\".\n\
         \n\
         View:\n\
        \x20   <div><button id=\"go\" on-click={{edit}}>go</button><p>{{draft}}</p>\
         <ul>{{for each r in rows:}}<li>{{r}}</li>{{end}}</ul></div>\n\
         \n\
         ## Main\n\
         Mount App.\n\
         Click \"#go\".\n"
    )
}

/// The same page, driven by a real keystroke into a bound input rather than a click.
fn typing_page(rows: usize) -> String {
    let items: String = (1..=rows)
        .map(|i| format!("\"r{i}\""))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "## Component App\n\
         \n\
         State:\n\
        \x20   Let draft be \"\".\n\
        \x20   Let rows be [{items}].\n\
         \n\
         View:\n\
        \x20   <div><input id=\"d\" bind={{draft}}>\
         <ul>{{for each r in rows:}}<li>{{r}}</li>{{end}}</ul></div>\n\
         \n\
         ## Main\n\
         Mount App.\n\
         Type \"a\" into \"#d\".\n"
    )
}

/// Ops that BUILD, MOVE or DESTROY nodes — the expensive ones, and the ones that lose focus.
fn structural(ops: &[DomOp]) -> usize {
    ops.iter()
        .filter(|o| {
            matches!(
                o,
                DomOp::CreateElement { .. }
                    | DomOp::CreateTextNode { .. }
                    | DomOp::AppendChild { .. }
                    | DomOp::InsertBefore { .. }
                    | DomOp::ClearChildren { .. }
                    | DomOp::RemoveNode { .. }
                    | DomOp::FreeNode { .. }
            )
        })
        .count()
}

// ---------------------------------------------------------------------------------------------

#[test]
fn a_write_to_one_field_does_not_rebuild_an_unrelated_list() {
    // ★ THE HEADLINE. `edit` writes `draft`; no row reads `draft`. A thousand rows must not be
    // touched, and the number that proves it is a CONSTANT — not "fewer than before".
    let ops = update_ops(&page(1000));
    assert!(
        structural(&ops) <= 3,
        "one field's update built or destroyed {} nodes across a 1,000-row page; \
         a fine-grained update touches what changed:\n{:#?}",
        structural(&ops),
        &ops[..ops.len().min(40)]
    );
}

#[test]
fn the_cost_of_an_update_does_not_grow_with_the_page() {
    // ★ THE PROPERTY, not a single measurement: the same edit against 10 rows and against 1,000
    // must cost the SAME. A ceiling alone can be met by a repaint that happens to be small at the
    // size the test picked; this catches the growth itself.
    let small = structural(&update_ops(&page(10)));
    let large = structural(&update_ops(&page(1000)));
    assert_eq!(
        small, large,
        "an update cost {small} ops at 10 rows and {large} at 1,000 — it is repainting the page, \
         not patching it"
    );
}

#[test]
fn a_keystroke_into_a_bound_input_costs_a_constant() {
    // The case a visitor actually feels: every character typed, against a long list.
    let ops = update_ops(&typing_page(1000));
    assert!(
        structural(&ops) <= 3,
        "one keystroke built or destroyed {} nodes:\n{:#?}",
        structural(&ops),
        &ops[..ops.len().min(40)]
    );
}

#[test]
fn the_input_being_typed_into_is_not_destroyed() {
    // ★ THE REASON THE COUNT MATTERS. Rebuilding the subtree replaces the very element the
    // visitor is typing into — a new node every keystroke, so focus and the caret die. The
    // reconciler papers over it in the browser, which is precisely why the reference tier has to
    // be held to not doing it in the first place.
    let ops = update_ops(&typing_page(50));
    assert!(
        !ops.iter().any(|o| matches!(
            o,
            DomOp::ClearChildren { .. }
                | DomOp::RemoveNode { .. }
                | DomOp::FreeNode { .. }
                | DomOp::CreateElement { .. }
        )),
        "a repaint removed or rebuilt a node instead of patching it:\n{ops:#?}"
    );
}

#[test]
fn a_write_that_a_site_does_read_still_updates_it() {
    // ★ THE OTHER DIRECTION, and the one that makes this file honest. An engine that emitted NO
    // ops would pass every ceiling above and render a dead page. The draft paragraph reads
    // `draft`, so it must really change.
    dom::reset();
    let out = tw_outcome(&page(10));
    assert_eq!(out.error, None);
    let html = dom::render();
    assert!(
        html.contains('x'),
        "the site that reads the written field must show the new value:\n{html}"
    );
}

#[test]
fn every_row_is_still_there_after_the_update() {
    // And the list must survive intact — a patcher that "updated nothing" by dropping the rows
    // would also meet every ceiling.
    dom::reset();
    let out = tw_outcome(&page(25));
    assert_eq!(out.error, None);
    let html = dom::render();
    for i in 1..=25 {
        assert!(
            html.contains(&format!("r{i}")),
            "row r{i} disappeared:\n{html}"
        );
    }
}

#[test]
fn a_write_to_the_list_does_update_the_list() {
    // The converse of the headline: when the collection really changes, the rows must change.
    // A patcher that never touched a list would satisfy every count in this file.
    let src = "## Component App\n\
        \n\
        State:\n\
       \x20   Let rows be [\"a\"].\n\
        \n\
        Actions:\n\
       \x20   To add:\n\
       \x20       Push \"b\" to rows.\n\
        \n\
        View:\n\
       \x20   <div><button id=\"go\" on-click={add}>+</button>\
        <ul>{for each r in rows:}<li>{r}</li>{end}</ul></div>\n\
        \n\
        ## Main\n\
        Mount App.\n\
        Click \"#go\".\n";
    dom::reset();
    let out = tw_outcome(src);
    assert_eq!(out.error, None, "{src}");
    let html = dom::render();
    assert!(
        html.contains('a') && html.contains('b'),
        "the appended row must render:\n{html}"
    );
}
