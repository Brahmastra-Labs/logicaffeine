//! ★ LOCK — only a block whose class carries a true count is ever counted or freed (S8, reclaim ON). ★
//!
//! A counted header (`ScalarHeader`/`HandleHeader`, 20 bytes) keeps its true holder count at word 16.
//! A map header is classified (classes 3-6) but not counted, and it is 16 bytes: its word 16 is the
//! first word of whatever follows it — for a map, usually its own entry buffer. A release that
//! trusted the caller's static kind instead of the block's class decremented that word (a map's first
//! key) or, when it read as 0 or 1, listed the map on the turn so the next safepoint freed it with its
//! entries still reachable — the free poisoned the entry buffer's free-list link and the next
//! allocation of that size ran off the heap.
//!
//! Every row drives a page through many turns with a safepoint after each, and holds the page's own
//! values (through the journal's before/after text) and the census to what the program says.

#![cfg(feature = "wasm-jit")]

#[path = "compile_common/mod.rs"]
mod common;
use common::wasm_host::{mount_interactive_checked, InteractivePage};

const TURNS: usize = 40;

/// Clicks the page's one button `TURNS` times, a safepoint after each, and returns every value the
/// journal recorded for `field` after each turn, with the census after each safepoint.
fn drive(src: &str, field: &str) -> (Vec<String>, Vec<i32>) {
    let mut page: InteractivePage = mount_interactive_checked(src);
    page.safepoint();
    let mut censuses = vec![page.census()];
    let button = *page
        .listening("click")
        .last()
        .expect("the page listens for a click");
    for turn in 0..TURNS {
        page.dispatch(button, "click", "")
            .unwrap_or_else(|e| panic!("turn {turn} traps: {e}\n{src}"));
        page.safepoint();
        censuses.push(page.census());
    }
    let after: Vec<String> = page
        .journal()
        .into_iter()
        .filter(|c| c.name == "bump")
        .filter_map(|c| {
            c.changes
                .chunks(3)
                .find(|t| t[0] == field)
                .map(|t| t[2].clone())
        })
        .collect();
    (after, censuses)
}

fn expect(values: Vec<String>, censuses: Vec<i32>, want: impl Fn(usize) -> i64, what: &str) {
    let want: Vec<String> = (1..=TURNS).map(|t| want(t).to_string()).collect();
    assert_eq!(values, want, "{what}: the page's values after each turn");
    assert!(
        censuses.iter().all(|&c| c == 0),
        "{what}: every census after a safepoint is exact: {censuses:?}"
    );
}

#[test]
fn a_state_map_keyed_by_a_small_int_survives_every_safepoint() {
    // The entry buffer's first word is the key `1`: a count read there sits at "one holder left".
    let src = "\
## Component App
State:
    Let counts be a new Map of Int to Int.
    Let total be 0.
Actions:
    To bump:
        If counts contains 1:
            Set item 1 of counts to (item 1 of counts) + 1.
        Otherwise:
            Set item 1 of counts to 1.
        Set total to item 1 of counts.
View:
    <div><button id=\"a\" on-click={bump}>a</button></div>

## Main
Mount App.
";
    let (values, censuses) = drive(src, "total");
    expect(values, censuses, |t| t as i64, "Map of Int to Int");
}

#[test]
fn a_state_map_keyed_by_text_keeps_its_keys_through_every_safepoint() {
    // The entry buffer's first word is the key's Text handle: a decrement there moves the key.
    let src = "\
## Component App
State:
    Let names be a new Map of Text to Int.
    Let total be 0.
Actions:
    To bump:
        If names contains \"k\":
            Set item \"k\" of names to (item \"k\" of names) + 2.
        Otherwise:
            Set item \"k\" of names to 2.
        Set total to item \"k\" of names.
View:
    <div><button id=\"a\" on-click={bump}>a</button></div>

## Main
Mount App.
";
    let (values, censuses) = drive(src, "total");
    expect(values, censuses, |t| 2 * t as i64, "Map of Text to Int");
}

#[test]
fn a_row_pushed_in_place_stays_held_by_its_matrix() {
    // `Push 5 to item 1 of rows` takes the row out, pushes, and puts it back: the row's slot never
    // stopped holding it, so the put-back must not end that hold.
    let src = "\
## Component App
State:
    Let rows be [[1, 2], [3, 4]].
    Let total be 0.
Actions:
    To bump:
        Push 5 to item 1 of rows.
        Set total to length of item 1 of rows.
View:
    <div><button id=\"a\" on-click={bump}>a</button></div>

## Main
Mount App.
";
    let (values, censuses) = drive(src, "total");
    expect(values, censuses, |t| 2 + t as i64, "Seq of Seq of Int");
}

#[test]
fn a_shared_row_written_through_its_matrix_stays_held_by_the_matrix() {
    // `Set item 1 of (item 1 of rows) to …` takes the row out, writes it, and puts it back; `keep`
    // shares the row, so the write copies it first, and what goes back is the COPY — which the
    // matrix slot must hold beyond the turn that made it.
    let src = "\
## Component App
State:
    Let rows be [[1, 2], [3, 4]].
    Let total be 0.
Actions:
    To bump:
        Let keep be item 1 of rows.
        Set item 1 of (item 1 of rows) to (item 1 of keep) + 1.
        Set total to item 1 of (item 1 of rows).
View:
    <div><button id=\"a\" on-click={bump}>a</button></div>

## Main
Mount App.
";
    let (values, censuses) = drive(src, "total");
    expect(
        values,
        censuses,
        |t| 1 + t as i64,
        "a shared row of a matrix",
    );
}

#[test]
fn a_shared_row_pushed_through_its_matrix_puts_back_a_copy_the_matrix_holds() {
    // `Push 5 to item 1 of rows` takes the row out by its index register, pushes, and puts it back.
    // `keep` shares the row, so the push copies it first: what goes back is the COPY, which only the
    // put-back's own hold keeps alive past the turn.
    let src = "\
## Component App
State:
    Let rows be [[1, 2], [3, 4]].
    Let total be 0.
Actions:
    To bump:
        Let keep be item 1 of rows.
        Push 5 to item 1 of rows.
        Set total to (length of item 1 of rows) + (length of keep).
View:
    <div><button id=\"a\" on-click={bump}>a</button></div>

## Main
Mount App.
";
    let (values, censuses) = drive(src, "total");
    expect(
        values,
        censuses,
        |t| 2 * t as i64 + 3,
        "a shared row pushed through its matrix",
    );
}

#[test]
fn a_copy_put_back_into_a_nested_record_field_is_held_by_the_record_past_the_turn() {
    // `Push 5 to (item 1 of boxes)'s tape` takes the record out of the list and the field out of the
    // record, pushes, and PUTS each level BACK into its parent. `keep` shares the tape, so the push
    // copies it and the put-back stores the copy; the copy's only holder beyond the turn is that
    // field, so a put-back that did not hold it frees it under the record.
    let src = "\
## A Box has:
    a tape, which is Seq of Int.

## Component App
State:
    Let boxes be [a new Box with tape [1, 2]].
    Let total be 0.
Actions:
    To bump:
        Let keep be (item 1 of boxes)'s tape.
        Push 5 to (item 1 of boxes)'s tape.
        Set total to (length of (item 1 of boxes)'s tape) + (length of keep).
View:
    <div><button id=\"a\" on-click={bump}>a</button></div>

## Main
Mount App.
";
    let (values, censuses) = drive(src, "total");
    expect(
        values,
        censuses,
        |t| 2 * t as i64 + 3,
        "a copy put back into a nested record field",
    );
}

#[test]
fn a_window_sentence_alone_runs_turn_after_turn() {
    // The page render's repro: no element listener at all, one window sentence.
    let src = "\
## Component App
State:
    Let n be 0.
Actions:
    When the window resizes:
        Set n to n + 1.
View:
    <p>{n}</p>

## Main
Mount App.
";
    let mut page = mount_interactive_checked(src);
    page.safepoint();
    let handler = *page
        .handlers()
        .last()
        .expect("the window sentence registers its closure");
    for turn in 0..TURNS {
        page.call_closure(handler, 0)
            .unwrap_or_else(|e| panic!("turn {turn} traps: {e}"));
        page.safepoint();
        assert_eq!(
            page.census(),
            0,
            "turn {turn}: the census after the safepoint is exact"
        );
    }
    let n: Vec<String> = page
        .journal()
        .into_iter()
        .filter_map(|c| {
            c.changes
                .chunks(3)
                .find(|t| t[0] == "n")
                .map(|t| t[2].clone())
        })
        .collect();
    assert_eq!(
        n,
        (1..=TURNS).map(|t| t.to_string()).collect::<Vec<_>>(),
        "every turn counted"
    );
}
