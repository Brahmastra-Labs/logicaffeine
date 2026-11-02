//! ★ LOCK — a keyed reorder MOVES the nodes, and the retained tree is the document. ★
//!
//! `{for each x in items, keyed by x:}` promises that a row keeps its identity when the list is
//! reordered: its input keeps what was typed into it, its child component keeps its own state. The
//! reconciler is `vdom.lg`: a paint draws the rows on the tape, `vdCommit` matches each against the
//! retained row with its key, and the rows off the longest run already in order are moved with
//! `InsertBefore`. The stream names every node by the id it was created under, and a host applies
//! it by those ids.
//!
//! ★ A PATCH LIST IS NOT THE DELIVERABLE, THE DOCUMENT IS. So every row paints through `vdom.lg`
//! and hands each commit to BOTH hosts a page runs on (`support/vdom_hosts.rs`): the native
//! recorder, and `runtime.js` under node. The document each host holds must be what the retained
//! tree serializes after every commit — a retained tree that describes another order than the page
//! computes every later diff from a page that does not exist. A row survives a reorder on the native
//! recorder when it keeps the id the stream names it by, and in the browser when it is the same
//! object.

mod common;
#[path = "compile_common/mod.rs"]
mod compile_common;
#[path = "support/vdom_hosts.rs"]
mod vdom_hosts;

use logicaffeine_compile::semantics::dom::DomOp;
use vdom_hosts::{paint, Paint};

/// One `<li key=k>k</li>` per key, straight under the root.
const ROWS: &str = "
## To rows(keys: Seq of Text):
    For each k in keys:
        vdOpenKeyed(\"li\", k).
        vdText(k).
        vdClose().
";

/// The rows `lists` paint, one commit per list, into a root standing for the body.
fn painted(lists: &[&[&str]]) -> Paint {
    let mut main = String::from("Let root be vdNewRoot(0).\n");
    for keys in lists {
        let quoted: Vec<String> = keys.iter().map(|k| format!("\"{k}\"")).collect();
        let list = if keys.is_empty() {
            "[] of Text".to_string()
        } else {
            format!("[{}]", quoted.join(", "))
        };
        main.push_str(&format!("rows({list}).\ncommit(root).\n"));
    }
    let p = paint(ROWS, &main);
    p.assert_hosts_follow();
    p
}

/// The markup the rows `keys` are, in order.
fn markup(keys: &[&str]) -> String {
    keys.iter().map(|k| format!("<li>{k}</li>")).collect()
}

/// How many of `ops` satisfy `pick`.
fn count(ops: &[DomOp], pick: impl Fn(&DomOp) -> bool) -> usize {
    ops.iter().filter(|op| pick(op)).count()
}

/// Whether `ops` create, free or detach any node — what a row that kept its identity never is.
fn recreates(ops: &[DomOp]) -> bool {
    ops.iter().any(|op| {
        matches!(
            op,
            DomOp::CreateElement { .. }
                | DomOp::CreateTextNode { .. }
                | DomOp::FreeNode { .. }
                | DomOp::RemoveNode { .. }
        )
    })
}

/// Asserts that after step `k` both hosts show `rows_now`, each of them the very node it was after
/// step `k - 1` (found there by its key in `rows_before`).
fn same_rows_moved(p: &Paint, k: usize, rows_before: &[&str], rows_now: &[&str]) {
    let at = |rows: &[&str], key: &str| {
        rows.iter()
            .position(|r| *r == key)
            .expect("a kept row was drawn before")
    };
    let native_before = p.native_ids(k - 1, "li");
    let native_now = p.native_ids(k, "li");
    let expected: Vec<i64> = rows_now
        .iter()
        .map(|r| native_before[at(rows_before, r)])
        .collect();
    assert_eq!(
        native_now, expected,
        "step {k}: the native recorder's rows are not the rows it had, moved"
    );
    if let (Some(before), Some(now)) = (p.browser(k - 1), p.browser(k)) {
        for tag in ["li", "#text"] {
            let (b, n) = (before.uids(tag), now.uids(tag));
            let expected: Vec<u64> = rows_now.iter().map(|r| b[at(rows_before, r)]).collect();
            assert_eq!(
                n, expected,
                "step {k}: the browser's `{tag}` nodes are not the ones it had, moved"
            );
        }
    }
}

#[test]
fn a_reorder_reaches_the_document() {
    // ★ THE HEADLINE. Two rows swap; the page must show them swapped, on both hosts.
    let p = painted(&[&["a", "b"], &["b", "a"]]);
    assert_eq!(
        p.steps[0].retained,
        markup(&["a", "b"]),
        "the first commit mounts both rows"
    );
    assert_eq!(
        p.steps[1].native,
        markup(&["b", "a"]),
        "the native recorder shows the reorder: {:?}",
        p.steps[1].ops
    );
    if let Some(b) = p.browser(1) {
        assert_eq!(
            b.markup,
            markup(&["b", "a"]),
            "the browser shows the reorder: {:?}",
            p.steps[1].ops
        );
    }
}

#[test]
fn a_reorder_keeps_every_node_it_had() {
    // The whole point of a key: identity survives. A move implemented as a recreate would take a
    // row's typed input and its child component's state with it.
    let p = painted(&[&["a", "b"], &["b", "a"]]);
    let ops = &p.steps[1].ops;
    assert!(
        !recreates(ops),
        "a pure reorder creates, frees or detaches nothing: {ops:?}"
    );
    assert_eq!(
        count(ops, |op| matches!(op, DomOp::InsertBefore { .. })),
        1,
        "one row moves past the other: {ops:?}"
    );
    same_rows_moved(&p, 1, &["a", "b"], &["b", "a"]);
}

#[test]
fn an_insert_in_the_middle_lands_in_the_middle() {
    // A new row appears where it belongs, not at the bottom, and its neighbours stay the nodes they were.
    let p = painted(&[&["a", "c"], &["a", "b", "c"]]);
    let ops = &p.steps[1].ops;
    assert_eq!(p.steps[1].native, markup(&["a", "b", "c"]), "{ops:?}");
    assert_eq!(
        count(ops, |op| matches!(op, DomOp::CreateElement { .. })),
        1,
        "only the new row is created: {ops:?}"
    );
    assert!(
        !ops.iter().any(|op| matches!(op, DomOp::FreeNode { .. })),
        "no row is freed: {ops:?}"
    );
    let native_before = p.native_ids(0, "li");
    let native_now = p.native_ids(1, "li");
    assert_eq!(
        (native_now[0], native_now[2]),
        (native_before[0], native_before[1]),
        "a and c keep their nodes"
    );
    if let (Some(before), Some(now)) = (p.browser(0), p.browser(1)) {
        let (b, n) = (before.uids("li"), now.uids("li"));
        assert_eq!(
            (n[0], n[2]),
            (b[0], b[1]),
            "a and c are the browser's same objects"
        );
        assert!(!b.contains(&n[1]), "b is a new node");
    }
}

#[test]
fn a_new_row_placed_during_a_reorder_lands_where_it_belongs() {
    // A reorder that also brings a new row: the kept rows move by key, and the new row is created
    // and placed before the row that follows it — not appended at the end — in the same stream.
    let p = painted(&[&["a", "b", "c"], &["c", "x", "a", "b"]]);
    let ops = &p.steps[1].ops;
    assert_eq!(p.steps[1].native, markup(&["c", "x", "a", "b"]), "{ops:?}");
    assert_eq!(
        count(ops, |op| matches!(op, DomOp::CreateElement { .. })),
        1,
        "only the new row is created: {ops:?}"
    );
    assert!(
        !ops.iter().any(|op| matches!(op, DomOp::FreeNode { .. })),
        "no row is freed: {ops:?}"
    );
    let before = p.native_ids(0, "li");
    let now = p.native_ids(1, "li");
    assert_eq!(
        (now[0], now[2], now[3]),
        (before[2], before[0], before[1]),
        "c, a and b are the nodes they were"
    );
    assert!(!before.contains(&now[1]), "x is a new node");
    if let (Some(b), Some(n)) = (p.browser(0), p.browser(1)) {
        let (b, n) = (b.uids("li"), n.uids("li"));
        assert_eq!(
            (n[0], n[2], n[3]),
            (b[2], b[0], b[1]),
            "c, a and b are the browser's same objects"
        );
        assert!(!b.contains(&n[1]), "x is a new object");
    }
}

#[test]
fn a_removal_and_a_reorder_together_land_correctly() {
    let p = painted(&[&["a", "b", "c"], &["c", "a"]]);
    let ops = &p.steps[1].ops;
    assert_eq!(p.steps[1].native, markup(&["c", "a"]), "{ops:?}");
    assert!(
        !ops.iter()
            .any(|op| matches!(op, DomOp::CreateElement { .. })),
        "nothing is created: {ops:?}"
    );
    assert_eq!(
        count(ops, |op| matches!(op, DomOp::FreeNode { .. })),
        1,
        "exactly the dropped row is freed: {ops:?}"
    );
    same_rows_moved(&p, 1, &["a", "b", "c"], &["c", "a"]);
}

#[test]
fn a_reversal_of_a_longer_list_lands_correctly() {
    // A reversal keeps one row on the longest increasing run, so exactly the other four move.
    let p = painted(&[&["a", "b", "c", "d", "e"], &["e", "d", "c", "b", "a"]]);
    let ops = &p.steps[1].ops;
    assert_eq!(
        p.steps[1].native,
        markup(&["e", "d", "c", "b", "a"]),
        "{ops:?}"
    );
    assert!(
        !recreates(ops),
        "a reversal creates, frees or detaches nothing: {ops:?}"
    );
    assert_eq!(
        count(ops, |op| matches!(op, DomOp::InsertBefore { .. })),
        4,
        "the four rows off the run move: {ops:?}"
    );
    same_rows_moved(
        &p,
        1,
        &["a", "b", "c", "d", "e"],
        &["e", "d", "c", "b", "a"],
    );
}

#[test]
fn an_unchanged_list_emits_no_moves() {
    // ★ THE COST GUARD. Re-appending every row on every commit would satisfy every row above and
    // make each update O(n) DOM writes — the exact thing keys exist to avoid.
    let p = painted(&[&["a", "b", "c"], &["a", "b", "c"]]);
    assert!(
        p.steps[1].ops.is_empty(),
        "re-painting an unchanged keyed list hands over nothing: {:?}",
        p.steps[1].ops
    );
    same_rows_moved(&p, 1, &["a", "b", "c"], &["a", "b", "c"]);
}

#[test]
fn the_retained_tree_still_matches_the_document_after_a_reorder() {
    // ★ THE PROGRESSIVE-CORRUPTION GUARD. After a reorder the retained tree and both documents are
    // one tree (`painted` asserts that at every step), so painting the same order again is a no-op;
    // a stream here would be the reconciler still correcting the reorder it already sent.
    let p = painted(&[&["a", "b", "c"], &["c", "b", "a"], &["c", "b", "a"]]);
    assert_eq!(p.steps[1].native, markup(&["c", "b", "a"]));
    assert!(
        p.steps[2].ops.is_empty(),
        "the retained tree disagrees with the document: {:?}",
        p.steps[2].ops
    );
    same_rows_moved(&p, 2, &["c", "b", "a"], &["c", "b", "a"]);
}
