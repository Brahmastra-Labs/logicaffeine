//! ★ LOCK — a lowered page repaints its retained tree: a turn writes what changed, and nothing else. ★
//!
//! A lowered page draws through `vdom.lg`'s tape: every paint is diffed against the tree the last
//! one left, and the commit sends the host only the difference. So a turn that changes one row's
//! text sets that text; a reorder moves the rows it must and keeps every node; a wrapper's repaint
//! keeps the nodes its caller drew into it; a turn that fails sends nothing; and repainting never
//! registers a listener twice.
//!
//! Each row runs on the tree-walker and on the bytecode VM, and reads what one turn sent the
//! document as the difference between two runs: the page mounted and driven up to the turn, and the
//! same run with the turn.

use logicaffeine_compile::compile::{tw_outcome, vm_outcome, RunOutcome};
use logicaffeine_compile::semantics::dom::{self, DomOp};

const TIERS: [(&str, fn(&str) -> RunOutcome); 2] =
    [("tree-walker", tw_outcome), ("VM", vm_outcome)];

/// The op log `page`'s `## Main` leaves: `Mount App.`, then `before`.
fn log_after(tier: fn(&str) -> RunOutcome, page: &str, before: &str) -> (Vec<DomOp>, RunOutcome) {
    dom::reset();
    let out = tier(&format!("{page}\n## Main\nMount App.\n{before}"));
    (dom::op_log(), out)
}

/// What `turn` sent the document on `tier`, after `before`, the document before and after it, and
/// how the run with the turn ended.
fn run_turn(
    tier: fn(&str) -> RunOutcome,
    page: &str,
    before: &str,
    turn: &str,
) -> (Vec<DomOp>, String, String, RunOutcome) {
    let (prefix, run) = log_after(tier, page, before);
    assert_eq!(
        run.error, None,
        "the page mounts and runs up to the turn:\n{page}\n{before}"
    );
    let shown = dom::render();
    let (log, out) = log_after(tier, page, &format!("{before}{turn}"));
    let after = dom::render();
    assert!(
        log.starts_with(&prefix),
        "a run is deterministic: the longer run begins with the shorter one's log"
    );
    (log[prefix.len()..].to_vec(), shown, after, out)
}

/// [`run_turn`] for a turn that succeeds.
fn turn(
    tier: fn(&str) -> RunOutcome,
    page: &str,
    before: &str,
    turn: &str,
) -> (Vec<DomOp>, String, String) {
    let (ops, shown, after, out) = run_turn(tier, page, before, turn);
    assert_eq!(out.error, None, "the turn runs:\n{page}\n{before}{turn}");
    (ops, shown, after)
}

/// Whether `op` builds, removes or moves a node.
fn structural(op: &DomOp) -> bool {
    matches!(
        op,
        DomOp::CreateElement { .. }
            | DomOp::CreateTextNode { .. }
            | DomOp::AppendChild { .. }
            | DomOp::InsertBefore { .. }
            | DomOp::RemoveNode { .. }
            | DomOp::FreeNode { .. }
            | DomOp::ClearChildren { .. }
    )
}

const ROWS: &str = "## Component Row (label: Text)
State:
    Let n be 0.
Actions:
    To bump:
        Set n to n + 1.
View:
    <li><span>{label}</span><button id=\"b-{label}\" on-click={bump}>{n}</button><small>{n > 100}</small></li>

## Component App
State:
    Let items be [\"a\", \"b\", \"c\"].
View:
    <ul>{for each x in items, keyed by x:}<Row label={x}/>{end}</ul>
";

#[test]
fn a_row_turn_touches_only_its_row() {
    for (tier, run) in TIERS {
        let (ops, _, after) = turn(run, ROWS, "", "Click \"#b-b\".\n");
        assert!(
            matches!(ops.as_slice(), [DomOp::SetText { text, .. }] if text == "1"),
            "{tier}: one row's turn sets that row's count and nothing else — not its other text that \
             reads the count and still says the same: {ops:?}"
        );
        assert!(
            after.contains("id=\"b-b\">1</button>") && after.contains("id=\"b-a\">0</button>"),
            "{tier}: {after}"
        );
    }
}

const WRAPPED: &str = "## Component Card
State:
    Let wide be false.
Actions:
    To widen:
        Set wide to true.
View:
    <section class:wide={wide}>{children}<button id=\"w\" on-click={widen}>w</button></section>

## Component Counter
State:
    Let n be 0.
Actions:
    To bump:
        Set n to n + 1.
View:
    <button id=\"c\" on-click={bump}>{n}</button>

## Component App
View:
    <main><Card><p>inside</p><Counter/></Card></main>
";

#[test]
fn a_wrapper_child_turn_keeps_slotted_nodes() {
    for (tier, run) in TIERS {
        let (ops, _, _) = turn(run, WRAPPED, "", "Click \"#c\".\n");
        assert!(
            matches!(ops.as_slice(), [DomOp::SetText { text, .. }] if text == "1"),
            "{tier}: a component in a wrapper's slot repaints its own text only: {ops:?}"
        );
        let (ops, _, after) = turn(run, WRAPPED, "", "Click \"#w\".\n");
        assert!(
            !ops.iter().any(structural)
                && ops.iter().all(|op| matches!(op, DomOp::SetAttribute { .. } | DomOp::RemoveAttribute { .. }))
                && ops.iter().any(|op| matches!(op, DomOp::SetAttribute { value, .. } if value.contains("wide"))),
            "{tier}: the wrapper's own turn writes its class, and nothing else of the page it repaints \
             through — every node its caller drew into it is kept as it was: {ops:?}"
        );
        assert!(after.contains("<p>inside</p>"), "{tier}: {after}");
    }
}

const LIST: &str = "## Component App
State:
    Let items be [\"a\", \"b\", \"c\", \"d\", \"e\"].
Actions:
    To flip:
        Set items to [\"e\", \"d\", \"c\", \"b\", \"a\"].
View:
    <ul>{for each x in items, keyed by x:}<li>{x}</li>{end}</ul><button id=\"go\" on-click={flip}>go</button>
";

#[test]
fn keyed_reorder_is_minimal_on_every_tier() {
    for (tier, run) in TIERS {
        let (ops, _, after) = turn(run, LIST, "", "Click \"#go\".\n");
        assert!(
            ops.iter()
                .all(|op| matches!(op, DomOp::InsertBefore { .. })),
            "{tier}: a reorder only moves rows — no row is built, freed or rewritten: {ops:?}"
        );
        assert_eq!(
            ops.len(),
            4,
            "{tier}: reversing five rows keeps one in place and moves four: {ops:?}"
        );
        assert!(
            after.contains("<ul><li>e</li><li>d</li><li>c</li><li>b</li><li>a</li></ul>"),
            "{tier}: {after}"
        );
    }
}

/// A turn whose repaint fails half-drawn: the hole divides by zero once `n` is one.
const FAILING: &str = "## Component App
State:
    Let n be 0.
Actions:
    To boom:
        Set n to n + 1.
View:
    <p id=\"before\">{n}</p><p id=\"out\">{10 / (1 - n)}</p><button id=\"go\" on-click={boom}>go</button>
";

#[test]
fn a_failed_turn_leaves_the_document_untouched() {
    for (tier, run) in TIERS {
        let (ops, shown, after, out) = run_turn(run, FAILING, "", "Click \"#go\".\n");
        assert!(
            out.error
                .as_deref()
                .is_some_and(|e| e.contains("Division by zero")),
            "{tier}: the turn ran and failed in its repaint: {out:?}"
        );
        assert_eq!(
            ops,
            Vec::<DomOp>::new(),
            "{tier}: a turn whose repaint fails sends the document nothing"
        );
        assert_eq!(
            after, shown,
            "{tier}: the document is the one the turn started from"
        );
    }
}

/// A page every turn repaints whole: `tick` is read by an `{if}`, so no turn is a patch of its sites.
const TICKING: &str = "## Component App
State:
    Let tick be 0.
Actions:
    To step:
        Set tick to tick + 1.
View:
    <ul><li><button id=\"a\" on-click={step}>a {tick}</button></li><li><button id=\"b\" on-click={step}>b {tick}</button></li></ul>{if tick > 0:}<i>on</i>{end}
";

#[test]
fn listener_state_is_bounded_across_repaints() {
    for (tier, run) in TIERS {
        let (once, _, _) = turn(run, TICKING, "Click \"#a\".\n", "Click \"#a\".\n");
        let (many, _, _) = turn(
            run,
            TICKING,
            "Click \"#a\".\n",
            &"Click \"#a\".\nClick \"#b\".\n".repeat(20),
        );
        for (what, ops) in [("one turn", &once), ("forty turns", &many)] {
            assert!(
                !ops.iter().any(|op| matches!(
                    op,
                    DomOp::Listen { .. } | DomOp::Unlisten { .. } | DomOp::AddEventListener { .. }
                )),
                "{tier}, {what}: a repaint re-registers no listener: {ops:?}"
            );
        }
        assert_eq!(
            many.len(),
            40 * once.len(),
            "{tier}: every turn sends what the first one sent, and no more"
        );
    }
}
