//! ★ RATCHET — the host operations `vdom.lg` emits for the list workloads a keyed UI lives on. ★
//!
//! The js-framework-benchmark operations over a keyed table of rows — each row
//! `<tr><td>{id}</td><td>{label}</td></tr>` keyed by its id — committed through `vdom.lg` and
//! counted per wire opcode, on the tree-walker and the VM alike (`assert_all_tiers`). The counts are
//! exact: each is the minimum the retained tree can do its job with, argued beside the row, so a
//! change that costs a host call more fails here by name, and a change that saves one is a
//! deliberate update of the number.

mod common;
use common::assert_all_tiers;

/// Opcode names, a census of a stream by opcode, and the row renderer.
const HELPERS: &str = "\
## To opcodeName(op: Int) -> Text:
    If op == 1:
        Return \"CreateElement\".
    If op == 2:
        Return \"CreateText\".
    If op == 3:
        Return \"SetText\".
    If op == 4:
        Return \"SetHtml\".
    If op == 6:
        Return \"SetAttribute\".
    If op == 7:
        Return \"AppendChild\".
    If op == 8:
        Return \"SetValue\".
    If op == 9:
        Return \"SetChecked\".
    If op == 15:
        Return \"InsertBefore\".
    If op == 16:
        Return \"RemoveAttribute\".
    If op == 19:
        Return \"Listen\".
    If op == 20:
        Return \"Unlisten\".
    If op == 21:
        Return \"FreeNode\".
    Return \"Op{op}\".

## To opWidth(op: Int) -> Int:
    If op == 6:
        Return 4.
    If op == 15:
        Return 4.
    If op == 19:
        Return 4.
    If op == 21:
        Return 2.
    Return 3.

## To census(words: Seq of Int) -> Text:
    Let mutable counts be a new Seq of Int.
    Let mutable k be 1.
    While k <= 21:
        Push 0 to counts.
        Set k to k + 1.
    Let mutable p be 1.
    Let n be length of words.
    While p <= n:
        Let op be item p of words.
        Set item op of counts to (item op of counts) + 1.
        Set p to p + opWidth(op).
    Let mutable out be \"\".
    Set k to 1.
    While k <= 21:
        If item k of counts > 0:
            If out != \"\":
                Set out to out + \",\".
            Set out to out + opcodeName(k) + \"=\" + \"{item k of counts}\".
        Set k to k + 1.
    If out == \"\":
        Return \"nothing\".
    Return out.

## To table(ids: Seq of Int, labels: Seq of Text):
    vdOpen(\"tbody\").
    Let mutable i be 1.
    While i <= length of ids:
        Let id be item i of ids.
        vdOpenKeyed(\"tr\", \"{id}\").
        vdOpen(\"td\").
        vdText(\"{id}\").
        vdClose().
        vdOpen(\"td\").
        vdText(item i of labels).
        vdClose().
        vdClose().
        Set i to i + 1.
    vdClose().

## To commitCensus(root: Int) -> Text:
    Let problem be vdCommit(root).
    If problem != \"\":
        Return problem.
    Let out be census(vdPatchWords()).
    vdDropPatches().
    Return out.
";

/// A program that builds rows `1..=n` labelled `row <id>` into `ids`/`labels`, commits them, then
/// runs `step` (statements over `root`, `ids`, `labels`) and shows the census of its last commit.
fn after(n: usize, step: &str) -> String {
    format!(
        "{HELPERS}
## Main
Let root be vdNewRoot(0).
Let mutable ids be a new Seq of Int.
Let mutable labels be a new Seq of Text.
Let mutable i be 1.
While i <= {n}:
    Push i to ids.
    Push \"row {{i}}\" to labels.
    Set i to i + 1.
table(ids, labels).
Let first be commitCensus(root).
{step}
"
    )
}

#[test]
fn create_1000_rows() {
    // Per row: the `<tr>` and its two `<td>`s created (3), the two texts created (2), and each of the
    // five placed once (5); the `<tbody>` created and placed once (2).
    let src = after(1000, "Show first.");
    assert_all_tiers(&src, "CreateElement=3001,CreateText=2000,AppendChild=5001");
}

#[test]
fn replace_all_1000_rows() {
    // Every key is new: 1000 rows created and appended in order (the same 10 ops each, the tbody
    // kept), and the 1000 old rows freed — one FreeNode per row, which releases its cells with it.
    let src = after(
        1000,
        "Let mutable fresh be a new Seq of Int.
Set i to 1.
While i <= 1000:
    Push (i + 1000) to fresh.
    Set i to i + 1.
table(fresh, labels).
Show commitCensus(root).",
    );
    assert_all_tiers(
        &src,
        "CreateElement=3000,CreateText=2000,AppendChild=5000,FreeNode=1000",
    );
}

#[test]
fn update_every_10th_row_label() {
    // A changed label is one SetText on the retained text node; nothing else moves or is made.
    let src = after(
        1000,
        "Set i to 1.
While i <= 1000:
    Set item i of labels to (item i of labels) + \" !!!\".
    Set i to i + 10.
table(ids, labels).
Show commitCensus(root).",
    );
    assert_all_tiers(&src, "SetText=100");
}

#[test]
fn swap_rows_2_and_999() {
    // The rows between keep their order, which is the longest increasing run, so exactly the two
    // swapped rows move.
    let src = after(
        1000,
        "Let a be item 2 of ids.
Set item 2 of ids to item 999 of ids.
Set item 999 of ids to a.
Let la be item 2 of labels.
Set item 2 of labels to item 999 of labels.
Set item 999 of labels to la.
table(ids, labels).
Show commitCensus(root).",
    );
    assert_all_tiers(&src, "InsertBefore=2");
}

#[test]
fn remove_one_row() {
    let src = after(
        1000,
        "Let mutable keptIds be a new Seq of Int.
Let mutable keptLabels be a new Seq of Text.
Set i to 1.
While i <= 1000:
    If i != 500:
        Push (item i of ids) to keptIds.
        Push (item i of labels) to keptLabels.
    Set i to i + 1.
table(keptIds, keptLabels).
Show commitCensus(root).",
    );
    assert_all_tiers(&src, "FreeNode=1");
}

#[test]
fn append_1000_rows_to_1000() {
    // The kept rows are the head the diff trims; the 1000 new rows are made and appended once each.
    let src = after(
        1000,
        "Set i to 1001.
While i <= 2000:
    Push i to ids.
    Push \"row {i}\" to labels.
    Set i to i + 1.
table(ids, labels).
Show commitCensus(root).",
    );
    assert_all_tiers(&src, "CreateElement=3000,CreateText=2000,AppendChild=5000");
}

#[test]
fn clear_1000_rows() {
    let src = after(
        1000,
        "table([] of Int, [] of Text).\nShow commitCensus(root).",
    );
    assert_all_tiers(&src, "FreeNode=1000");
}

#[test]
fn an_unchanged_repaint_of_1000_rows_emits_nothing() {
    let src = after(1000, "table(ids, labels).\nShow commitCensus(root).");
    assert_all_tiers(&src, "nothing");
}
