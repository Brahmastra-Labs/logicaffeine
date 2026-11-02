//! ★ LOCK — `vdom.lg`, the retained-tree reconciler every tier renders through (Wave 7.4). ★
//!
//! Every row drives `vdom.lg`'s own surface from a `## Main` program through `assert_all_tiers`,
//! which holds the tree-walker to the expected output, and the VM and the compiled wasm module (run
//! on wasmi) to the tree-walker. A render writes the tape (`vdOpen`/`vdText`/`vdClose`/…), `vdCommit(node)`
//! diffs it against that node's retained children and appends the patch stream, and
//! `vdPatchWords()` hands the stream back — so a row asserts the exact minimal op sequence as
//! legible opcode names, and `vdSerializeChildren` asserts the markup the retained tree holds.
//!
//! Beside the behaviour rows sit three source censuses: the module splices alone (a program that
//! names one `vd` function parses no other stdlib module), `vdFootprint` counts every array the
//! module keeps (so "a steady-state repaint grows nothing" measures all of them), and a real
//! commit's stream decodes through `patch::decode_words`, the wire format's spec.

#![allow(dead_code)]

mod common;
#[path = "compile_common/mod.rs"]
mod compile_common;

/// `common::run_all_tiers` with the third tier every page ships on: the program compiled to wasm
/// and run on wasmi answers exactly what the tree-walker answers, output and failure alike.
fn run_all_tiers(source: &str) -> common::TierAgreement {
    let agreed = common::run_all_tiers(source);
    #[cfg(feature = "wasm-jit")]
    {
        let wasm = compile_common::wasm_host::run_outcome(source).unwrap_or_else(|why| {
            panic!("TIER DIVERGENCE — the wasm module does not run: {why}\nSource:\n{source}")
        });
        assert_eq!(
            (wasm.output.trim(), wasm.failure.as_deref()),
            (agreed.output.trim(), agreed.error.as_deref()),
            "TIER DIVERGENCE — the wasm module disagrees with the tree-walker.\nSource:\n{source}"
        );
    }
    agreed
}

/// The program runs cleanly on every tier, and the tree-walker's output is exactly `expected`.
fn assert_all_tiers(source: &str, expected: &str) {
    let agreed = run_all_tiers(source);
    assert!(
        agreed.error.is_none(),
        "Program should run cleanly on all tiers.\nSource:\n{source}\n\nError: {:?}",
        agreed.error
    );
    assert_eq!(agreed.output.trim(), expected, "\nSource:\n{source}");
}

/// Opcode names for a patch stream's words, a per-opcode width to walk it, and the list
/// renderer the list-shape rows share.
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
    If op == 14:
        Return \"RemoveNode\".
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
    If op == 17:
        Return \"Adopt\".
    If op == 18:
        Return \"AdoptText\".
    If op == 22:
        Return \"Hydrate\".
    Return \"Op{op}\".

## To opWidth(op: Int) -> Int:
    If op == 6:
        Return 4.
    If op == 15:
        Return 4.
    If op == 19:
        Return 4.
    If op == 14:
        Return 2.
    If op == 21:
        Return 2.
    If op == 17:
        Return 4.
    If op == 18:
        Return 4.
    If op == 22:
        Return 2.
    Return 3.

## To summarize(words: Seq of Int) -> Text:
    Let mutable out be \"\".
    Let mutable p be 1.
    Let n be length of words.
    While p <= n:
        Let op be item p of words.
        If out == \"\":
            Set out to opcodeName(op).
        Otherwise:
            Set out to out + \",\" + opcodeName(op).
        Set p to p + opWidth(op).
    Return out.

## To countOps(words: Seq of Int, want: Int) -> Int:
    Let mutable count be 0.
    Let mutable p be 1.
    Let n be length of words.
    While p <= n:
        Let op be item p of words.
        If op == want:
            Set count to count + 1.
        Set p to p + opWidth(op).
    Return count.

## To buildList(keys: Seq of Text):
    vdOpen(\"ul\").
    For each k in keys:
        vdOpenKeyed(\"li\", k).
        vdText(k).
        vdClose().
    vdClose().
";

/// Commits list `a_keys`, then list `b_keys` against the same root, and asserts the second
/// commit's stream, as opcode names, is exactly `expect`.
fn shape(a_keys: &str, b_keys: &str, expect: &str) {
    let src = format!(
        "{HELPERS}
## Main
Let root be vdNewRoot(0).
buildList({a_keys}).
Show vdCommit(root).
vdDropPatches().
buildList({b_keys}).
Show vdCommit(root).
Show summarize(vdPatchWords()).
"
    );
    assert_all_tiers(&src, expect);
}

// ---- The thirteen list-mutation shapes: each produces exactly the minimal op sequence. A new row
// is created and put at its final place (CreateElement + one insertion — appended, in order, when
// nothing retained follows it); a kept row that must move takes one InsertBefore; the rows on a
// longest increasing run do not move; every removed row is one FreeNode, emitted after every create
// and move of the commit. ----

#[test]
fn shape_append() {
    shape(
        "[\"a\", \"b\", \"c\"]",
        "[\"a\", \"b\", \"c\", \"d\"]",
        "CreateElement,AppendChild,CreateText,AppendChild",
    );
}

#[test]
fn shape_prepend() {
    shape(
        "[\"a\", \"b\", \"c\"]",
        "[\"d\", \"a\", \"b\", \"c\"]",
        "CreateElement,InsertBefore,CreateText,AppendChild",
    );
}

#[test]
fn shape_insert_middle() {
    shape(
        "[\"a\", \"b\", \"c\"]",
        "[\"a\", \"x\", \"b\", \"c\"]",
        "CreateElement,InsertBefore,CreateText,AppendChild",
    );
}

#[test]
fn shape_remove_first() {
    shape("[\"a\", \"b\", \"c\"]", "[\"b\", \"c\"]", "FreeNode");
}

#[test]
fn shape_remove_last() {
    shape("[\"a\", \"b\", \"c\"]", "[\"a\", \"b\"]", "FreeNode");
}

#[test]
fn shape_remove_middle() {
    shape("[\"a\", \"b\", \"c\"]", "[\"a\", \"c\"]", "FreeNode");
}

#[test]
fn shape_swap_two() {
    // The longest increasing run over the kept rows' old positions [b@2, a@1, c@3] keeps {a, c},
    // so only `b` moves — one `InsertBefore`, not two.
    shape(
        "[\"a\", \"b\", \"c\"]",
        "[\"b\", \"a\", \"c\"]",
        "InsertBefore",
    );
}

#[test]
fn shape_reverse() {
    // A reversal is the longest-run worst case: one row stays, the other two move.
    shape(
        "[\"a\", \"b\", \"c\"]",
        "[\"c\", \"b\", \"a\"]",
        "InsertBefore,InsertBefore",
    );
}

#[test]
fn shape_rotate() {
    // Rotating left by one keeps {b, c} and moves only `a` to the end.
    shape(
        "[\"a\", \"b\", \"c\"]",
        "[\"b\", \"c\", \"a\"]",
        "InsertBefore",
    );
}

#[test]
fn shape_replace_all() {
    shape(
        "[\"a\", \"b\", \"c\"]",
        "[\"x\", \"y\", \"z\"]",
        "CreateElement,CreateElement,CreateElement,AppendChild,AppendChild,AppendChild,\
         CreateText,AppendChild,CreateText,AppendChild,CreateText,AppendChild,\
         FreeNode,FreeNode,FreeNode",
    );
}

#[test]
fn shape_clear() {
    shape(
        "[\"a\", \"b\", \"c\"]",
        "[] of Text",
        "FreeNode,FreeNode,FreeNode",
    );
}

#[test]
fn shape_keyed_reorder_with_lis() {
    // Current order (a,b,c,d,e) → desired (c,a,e,b,d): old positions [3,1,5,2,4], whose longest
    // strictly increasing run {1,2,4} keeps a, b and d; only c and e move.
    shape(
        "[\"a\", \"b\", \"c\", \"d\", \"e\"]",
        "[\"c\", \"a\", \"e\", \"b\", \"d\"]",
        "InsertBefore,InsertBefore",
    );
}

#[test]
fn shape_mixed_insert_remove_move() {
    // Drop a and c, insert x before the kept b, and bring d to the front ahead of x.
    shape(
        "[\"a\", \"b\", \"c\", \"d\"]",
        "[\"d\", \"x\", \"b\"]",
        "CreateElement,InsertBefore,InsertBefore,CreateText,AppendChild,FreeNode,FreeNode",
    );
}

// ---- LIS vectors, checked against an independent reference. ----

/// The longest strictly-increasing run by patience sorting with predecessor links, written in Rust
/// apart from `vdom.lg`, so `vdLongestIncreasingRun`'s output is checked against it per case rather
/// than against a hand-picked expectation that could encode the same bug twice.
fn reference_lis_keep(seq: &[i64]) -> Vec<i64> {
    let n = seq.len();
    let mut keep = vec![0i64; n];
    if n == 0 {
        return keep;
    }
    let mut tails: Vec<usize> = Vec::new();
    let mut prev: Vec<Option<usize>> = vec![None; n];
    for i in 0..n {
        let mut lo = 0usize;
        let mut hi = tails.len();
        while lo < hi {
            let mid = (lo + hi) / 2;
            if seq[tails[mid]] < seq[i] {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        if lo > 0 {
            prev[i] = Some(tails[lo - 1]);
        }
        if lo == tails.len() {
            tails.push(i);
        } else {
            tails[lo] = i;
        }
    }
    let mut cursor = tails.last().copied();
    while let Some(i) = cursor {
        keep[i] = 1;
        cursor = prev[i];
    }
    keep
}

fn seq_literal(xs: &[i64]) -> String {
    format!(
        "[{}]",
        xs.iter().map(i64::to_string).collect::<Vec<_>>().join(", ")
    )
}

#[test]
fn longest_increasing_run_matches_the_reference() {
    let cases: &[&[i64]] = &[
        &[1, 2, 3, 4],
        &[4, 3, 2, 1],
        &[2, 1, 3],
        &[2, 3, 1],
        &[3, 1, 4, 1, 5, 9, 2, 6],
        &[5, 4, 3, 2, 1, 6, 7, 8],
        &[1],
        &[1, 1, 1],
        &[10, 9, 2, 5, 3, 7, 101, 18],
    ];
    let mut src = String::from("## Main\n");
    let mut expected = Vec::new();
    for seq in cases {
        src.push_str(&format!(
            "Show vdLongestIncreasingRun({}).\n",
            seq_literal(seq)
        ));
        expected.push(seq_literal(&reference_lis_keep(seq)));
    }
    assert_all_tiers(&src, &expected.join("\n"));
}

// ---- The property: apply(diff(A, B), render(A)) = render(B). ----

/// Commits `a_body` then `b_body` against one retained root, and `b_body` alone against a fresh
/// root; asserts the two serialize identically AND that the markup is exactly `expected_markup`,
/// so a wrong prediction on both sides can never agree with itself.
fn property_two_line(a_body: &str, b_body: &str, expected_markup: &str) {
    let src = format!(
        "{HELPERS}
## Main
Let rootMain be vdNewRoot(0).
{a_body}
Show vdCommit(rootMain).
{b_body}
Show vdCommit(rootMain).
Let patched be vdSerializeChildren(rootMain).
Let rootFresh be vdNewRoot(0).
{b_body}
Show vdCommit(rootFresh).
Show patched == vdSerializeChildren(rootFresh).
Show patched.
"
    );
    assert_all_tiers(&src, &format!("true\n{expected_markup}"));
}

#[test]
fn property_a_plain_keyed_list_grows_and_shrinks() {
    property_two_line(
        "buildList([\"a\", \"b\", \"c\"]).",
        "buildList([\"c\", \"x\", \"a\"]).",
        "<ul><li>c</li><li>x</li><li>a</li></ul>",
    );
}

#[test]
fn property_nested_elements_with_attributes() {
    property_two_line(
        "vdOpen(\"div\").\nvdSetAttribute(\"class\", \"card\").\nvdOpen(\"span\").\nvdText(\"one\").\nvdClose().\nvdClose().",
        "vdOpen(\"div\").\nvdSetAttribute(\"class\", \"card2\").\nvdOpen(\"span\").\nvdText(\"two\").\nvdClose().\nvdOpen(\"em\").\nvdText(\"new\").\nvdClose().\nvdClose().",
        "<div class=\"card2\"><span>two</span><em>new</em></div>",
    );
}

#[test]
fn property_bound_value_is_preserved_when_the_render_does_not_set_it() {
    // The second render never sets the value, so the value rule leaves the retained property as
    // the first render set it. A fresh mount of the second tape alone has no value at all, so this
    // is asserted on the patched tree's markup directly rather than against a fresh mount.
    let src = format!(
        "{HELPERS}
## Main
Let root be vdNewRoot(0).
vdOpen(\"input\").
vdSetValue(\"typed\").
vdClose().
Show vdCommit(root).
vdOpen(\"input\").
vdSetAttribute(\"class\", \"dirty\").
vdClose().
Show vdCommit(root).
Show vdSerializeChildren(root).
"
    );
    assert_all_tiers(&src, "<input class=\"dirty\" value=\"typed\">");
}

#[test]
fn property_checked_toggles_and_survives_an_unrelated_change() {
    property_two_line(
        "vdOpen(\"input\").\nvdSetChecked(true).\nvdClose().",
        "vdOpen(\"input\").\nvdSetChecked(true).\nvdSetAttribute(\"class\", \"on\").\nvdClose().",
        "<input class=\"on\" checked>",
    );
}

#[test]
fn property_a_removed_row_takes_its_own_children_with_it() {
    property_two_line(
        "vdOpen(\"ul\").\nvdOpenKeyed(\"li\", \"a\").\nvdOpen(\"b\").\nvdText(\"bold\").\nvdClose().\nvdClose().\nvdOpenKeyed(\"li\", \"b\").\nvdText(\"b\").\nvdClose().\nvdClose().",
        "vdOpen(\"ul\").\nvdOpenKeyed(\"li\", \"b\").\nvdText(\"b\").\nvdClose().\nvdClose().",
        "<ul><li>b</li></ul>",
    );
}

// ---- Duplicate keys fail loudly, naming the key and both tape positions. ----

#[test]
fn duplicate_sibling_keys_fail_loudly_naming_the_key_and_both_positions() {
    let src = format!(
        "{HELPERS}
## Main
Let root be vdNewRoot(0).
buildList([\"a\", \"a\"]).
Show vdCommit(root).
Show vdPatchLength().
Show vdSerializeChildren(root).
"
    );
    assert_all_tiers(
        &src,
        "vdom: duplicate key \"a\" on siblings at tape positions 5 and 12\n0",
    );
}

#[test]
fn a_duplicate_among_many_keys_names_the_same_pair_the_short_scan_would() {
    // Past 32 keyed siblings the check sorts by key hash instead of scanning pairs; it must still
    // report the key whose SECOND occurrence comes first in tape order, with its first occurrence.
    let src = format!(
        "{HELPERS}
## Main
Let root be vdNewRoot(0).
Let mutable keys be a new Seq of Text.
Let mutable i be 1.
While i <= 60:
    Push \"k{{i}}\" to keys.
    Set i to i + 1.
Push \"k40\" to keys.
Push \"k7\" to keys.
Push \"k40\" to keys.
buildList(keys).
Show vdCommit(root).
"
    );
    // Each row is OPEN (4 words) + TEXT (2) + CLOSE (1) = 7 words after the list's own 4-word
    // OPEN, so row r opens at tape position 5 + 7 * (r - 1).
    let at = |row: i64| 5 + 7 * (row - 1);
    assert_all_tiers(
        &src,
        &format!(
            "vdom: duplicate key \"k40\" on siblings at tape positions {} and {}",
            at(40),
            at(61)
        ),
    );
}

// ---- An element's own name: `vdOpenSoft` matches by the id an element names itself with, and
// tolerates two siblings sharing one. ----

/// Draws `input`s named `ids` in order under the root (each through `open`), giving the one named
/// `#first` site 1 and the one named `#second` site 2.
fn named_inputs(open: &str, ids: &[&str]) -> String {
    let mut out = String::new();
    for id in ids {
        match open {
            "vdOpenSoft" => out.push_str(&format!("vdOpenSoft(\"input\", \"#{id}\").\n")),
            _ => out.push_str("vdOpen(\"input\").\n"),
        }
        match *id {
            "first" => out.push_str("vdSite(1).\n"),
            "second" => out.push_str("vdSite(2).\n"),
            _ => {}
        }
        out.push_str("vdClose().\n");
    }
    out
}

/// Draws `#first`, `#second`, then `#extra` BEFORE them, and reports the second commit's stream
/// and whether both sites still answer the nodes they answered before.
fn a_sibling_drawn_before(open: &str) -> String {
    format!(
        "{HELPERS}
## Main
Let root be vdNewRoot(0).
{}Show vdCommit(root).
Let a be vdSiteNode(1).
Let b be vdSiteNode(2).
vdDropPatches().
{}Show vdCommit(root).
Show summarize(vdPatchWords()).
Show vdSiteNode(1) == a and vdSiteNode(2) == b.
",
        named_inputs(open, &["first", "second"]),
        named_inputs(open, &["extra", "first", "second"]),
    )
}

#[test]
fn an_element_named_by_its_id_keeps_its_node_when_a_sibling_is_drawn_before_it() {
    // One new node, put in front; the two named ones stay the nodes they were, so whatever a
    // visitor left in them (focus, a typed value, a scroll) stays with the name.
    assert_all_tiers(
        &a_sibling_drawn_before("vdOpenSoft"),
        "CreateElement,InsertBefore\ntrue",
    );
}

#[test]
fn an_unnamed_element_is_matched_by_position_and_a_sibling_before_it_shifts_every_node() {
    // The same drawing without names: the old nodes are taken in order, so the node that WAS the
    // first input now draws the new one, and the last input is the one created.
    assert_all_tiers(
        &a_sibling_drawn_before("vdOpen"),
        "CreateElement,AppendChild\nfalse",
    );
}

#[test]
fn a_named_element_is_never_taken_for_an_unnamed_one_of_its_tag() {
    // Position matching skips a named node: an unnamed `<input>` drawn where a named one stood is
    // a new node, appended since nothing retained follows it, and the named one, no longer drawn,
    // is freed.
    let src = format!(
        "{HELPERS}
## Main
Let root be vdNewRoot(0).
vdOpenSoft(\"input\", \"#first\").
vdClose().
Show vdCommit(root).
vdDropPatches().
vdOpen(\"input\").
vdClose().
Show vdCommit(root).
Show summarize(vdPatchWords()).
"
    );
    assert_all_tiers(&src, "CreateElement,AppendChild,FreeNode");
}

#[test]
fn two_siblings_named_alike_are_drawn_not_refused_and_matched_in_order() {
    // An id is the author's to repeat by mistake, and a repeated one is not a reason to refuse the
    // paint — unlike a `keyed by` row, whose key is the page's own claim to identity. The two
    // are matched to the retained pair in order, so an unchanged repaint emits nothing.
    let src = format!(
        "{HELPERS}
## Main
Let root be vdNewRoot(0).
vdOpenSoft(\"p\", \"#x\").
vdText(\"1\").
vdClose().
vdOpenSoft(\"p\", \"#x\").
vdText(\"2\").
vdClose().
Show vdCommit(root).
Show vdSerializeChildren(root).
vdDropPatches().
vdOpenSoft(\"p\", \"#x\").
vdText(\"1\").
vdClose().
vdOpenSoft(\"p\", \"#x\").
vdText(\"2\").
vdClose().
Show vdCommit(root).
Show vdPatchLength().
"
    );
    assert_all_tiers(&src, "<p>1</p><p>2</p>\n\n0");
}

#[test]
fn a_keyed_row_repeated_is_still_refused_beside_a_named_element() {
    // Tolerating repeated names must not tolerate repeated row keys: the duplicate check counts
    // every `keyed by` key, and only those.
    let src = format!(
        "{HELPERS}
## Main
Let root be vdNewRoot(0).
vdOpenSoft(\"li\", \"#a\").
vdClose().
vdOpenKeyed(\"li\", \"a\").
vdClose().
vdOpenKeyed(\"li\", \"a\").
vdClose().
Show vdCommit(root).
"
    );
    assert_all_tiers(
        &src,
        "vdom: duplicate key \"a\" on siblings at tape positions 6 and 11",
    );
}

// ---- The allocator: a freed id is reused, LIFO, and never handed out twice while still live. ----

#[test]
fn freed_ids_are_reused_lifo_and_never_collide_with_a_still_live_allocation() {
    // Frees `a` then `b`, so the next two allocations must pop `b` then `a` (LIFO) — never the
    // same id twice, which is what an allocator whose pop never lands would hand out.
    let src = "\
## Main
Let a be vdAlloc().
Let b be vdAlloc().
Let c be vdAlloc().
vdFree(a).
vdFree(b).
Let reused1 be vdAlloc().
Let reused2 be vdAlloc().
Let fresh be vdAlloc().
Show \"{reused1},{reused2},{fresh}\".
Show reused1 != reused2.
";
    assert_all_tiers(src, "2,1,4\ntrue");
}

// ---- `vdCommit`'s worklist cascades through every nesting level, not only the root's. ----

#[test]
fn commit_cascades_through_every_nesting_level_not_just_the_root() {
    let src = format!(
        "{HELPERS}
## Main
Let root be vdNewRoot(0).
vdOpen(\"div\").
vdOpen(\"ul\").
vdOpen(\"li\").
vdText(\"x\").
vdClose().
vdClose().
vdClose().
Show vdCommit(root).
Show summarize(vdPatchWords()).
"
    );
    assert_all_tiers(
        &src,
        "CreateElement,AppendChild,CreateElement,AppendChild,CreateElement,AppendChild,\
         CreateText,AppendChild",
    );
}

// ---- `vdFragClose` shares `vdClose`'s open stack and backpatches its own end. ----

#[test]
fn fragment_close_backpatches_its_own_end_position_correctly() {
    // A fragment nested inside a real element, itself wrapping one more element: closing the
    // fragment is not the last pop off the open stack, so a close that backpatched the wrong
    // record would leave the outer div's end slot at 0 and the commit could not walk the tape.
    let src = r#"## Main
Let root be vdNewRoot(0).
Let frag be vdNewFrag().
vdOpen("div").
vdFragOpen(frag).
vdOpen("li").
vdText("hi").
vdClose().
vdFragClose().
vdClose().
Show vdTapeItem(7).
Show vdTapeItem(4).
Show vdCommit(root).
Show vdSerializeChildren(root).
"#;
    assert_all_tiers(src, "15\n16\n\n<div><li>hi</li></div>");
}

// ---- Sites: a marker on an element, never a child, answering the element's node after commit. ----

#[test]
fn a_site_marks_its_element_and_is_not_a_child() {
    let src = format!(
        "{HELPERS}
## Main
Let root be vdNewRoot(0).
vdOpen(\"div\").
vdSite(1).
vdText(\"x\").
vdClose().
Show vdCommit(root).
Show summarize(vdPatchWords()).
Show vdSerializeChildren(root).
"
    );
    assert_all_tiers(
        &src,
        "CreateElement,AppendChild,CreateText,AppendChild\n<div>x</div>",
    );
}

#[test]
fn a_site_resolves_to_its_real_node_after_commit() {
    // The site's node is the id the stream created the element under; it follows the site to a
    // different element, and it is gone once no retained element carries it.
    let src = format!(
        "{HELPERS}
## Main
Let root be vdNewRoot(0).
vdOpen(\"div\").
vdSite(1).
vdClose().
Show vdCommit(root).
Let words be vdPatchWords().
Show vdWire(vdSiteNode(1)) == item 2 of words.
vdDropPatches().
vdOpen(\"div\").
vdClose().
vdOpenKeyed(\"span\", \"s\").
vdSite(1).
vdClose().
Show vdCommit(root).
Let words2 be vdPatchWords().
Show summarize(words2).
Show vdWire(vdSiteNode(1)) == item 2 of words2.
vdDropPatches().
Show vdCommit(root).
Show vdSiteNode(1).
"
    );
    assert_all_tiers(&src, "true\n\nCreateElement,AppendChild\ntrue\n\n0");
}

// ---- Anchors: the sites a paint hands out, kept per instance fragment, given back together. ----

#[test]
fn anchors_are_kept_per_fragment_and_given_back_together() {
    // Each anchor is a fresh site; releasing one fragment's anchors hands back exactly its own —
    // the next sites minted are those, most recent first — and leaves the other fragment's alone.
    let src = format!(
        "{HELPERS}
## Main
Let f be vdNewFrag().
Let g be vdNewFrag().
Let a be vdAnchor(f).
Let b be vdAnchor(f).
Let c be vdAnchor(g).
Show a != b and b != c and a != c.
vdReleaseAnchors(f).
Show vdAnchor(g) == a.
Show vdAnchor(g) == b.
Show vdNewSite() != c.
vdReleaseAnchors(g).
vdReleaseAnchors(g).
vdReleaseAnchors(99).
Show vdAnchor(f) == c.
"
    );
    assert_all_tiers(&src, "true\ntrue\ntrue\ntrue\ntrue");
}

#[test]
fn anchoring_and_releasing_grows_nothing_once_settled() {
    // A paint anchors, the next paint releases and anchors again: after the first round every
    // anchor reuses a pooled entry and a freed site, so the module's arrays stay the size they were.
    let src = format!(
        "{HELPERS}
## Main
Let f be vdNewFrag().
Let mutable round be 1.
Let mutable settled be 0.
While round <= 50:
    vdReleaseAnchors(f).
    Let mutable i be 1.
    While i <= 3:
        Let s be vdAnchor(f).
        Set i to i + 1.
    If round == 2:
        Set settled to vdFootprint().
    Set round to round + 1.
Show vdFootprint() == settled.
"
    );
    assert_all_tiers(&src, "true");
}

// ---- Fragments: identity, keep, subtree commits, and removal reported as gone. ----

#[test]
fn keeping_an_unretained_fragment_is_refused_not_emptied() {
    let src = "\
## Main
Let root be vdNewRoot(0).
Let f be vdNewFrag().
vdKeep(f).
Show vdCommit(root).
Show vdPatchLength().
Show vdSerializeChildren(root).
Show vdTapeLength().
";
    assert_all_tiers(
        src,
        "vdom: keep of fragment 1, which is not retained\n0\n\n0",
    );
}

#[test]
fn a_subtree_commit_touches_only_that_fragment() {
    // Only the fragment's own children are diffed: its text changes in place, and its new child
    // lands inside its range — before the `<p>` that follows the fragment — not after it.
    // Nodes: root 1, div 2, fragment 3, p 4, "tail" 5, span 6, "a" 7; the commit makes em 8 and
    // "c" 9. Every node but the root is named on the wire as `node + 2^31`.
    let src = format!(
        "{HELPERS}
## Main
Let root be vdNewRoot(0).
Let f be vdNewFrag().
vdOpen(\"div\").
vdFragOpen(f).
vdOpen(\"span\").
vdText(\"a\").
vdClose().
vdFragClose().
vdOpen(\"p\").
vdText(\"tail\").
vdClose().
vdClose().
Show vdCommit(root).
vdDropPatches().
vdOpen(\"span\").
vdText(\"b\").
vdClose().
vdOpen(\"em\").
vdText(\"c\").
vdClose().
Show vdCommit(vdFragNode(f)).
Show summarize(vdPatchWords()).
Show vdPatchWords().
Show vdSerializeChildren(root).
"
    );
    assert_all_tiers(
        &src,
        "CreateElement,InsertBefore,CreateText,AppendChild,SetText\n\
         [1, 2147483656, 1, 15, 2147483650, 2147483656, 2147483652, 2, 2147483657, 2, 7, 2147483656, 2147483657, 3, 2147483655, 3]\n\
         <div><span>b</span><em>c</em><p>tail</p></div>",
    );
}

#[test]
fn a_removed_instance_fragment_is_reported_gone() {
    let src = format!(
        "{HELPERS}
## Main
Let root be vdNewRoot(0).
Let f1 be vdNewFrag().
Let f2 be vdNewFrag().
vdOpen(\"ul\").
vdFragOpen(f1).
vdOpen(\"li\").
vdText(\"a\").
vdClose().
vdFragClose().
vdFragOpen(f2).
vdOpen(\"li\").
vdText(\"b\").
vdClose().
vdFragClose().
vdClose().
Show vdCommit(root).
vdDropPatches().
vdOpen(\"ul\").
vdKeep(f2).
vdClose().
Show vdCommit(root).
Show summarize(vdPatchWords()).
Show vdGoneCount().
Show vdGone(1) == f1.
Show vdFragNode(f1).
Show vdFragNode(f2) != 0.
Show vdSerializeChildren(root).
vdClearGone().
Show vdNewFrag() == f1.
"
    );
    assert_all_tiers(
        &src,
        "FreeNode\n1\ntrue\n0\ntrue\n<ul><li>b</li></ul>\ntrue",
    );
}

#[test]
fn a_keyed_reorder_of_component_rows_moves_their_real_nodes() {
    // Each row is a fragment holding two host nodes; moving the row moves both, in order.
    let src = format!(
        "{HELPERS}
## To row(f: Int, label: Text):
    vdFragOpen(f).
    vdOpen(\"dt\").
    vdText(label).
    vdClose().
    vdOpen(\"dd\").
    vdText(label).
    vdClose().
    vdFragClose().

## Main
Let root be vdNewRoot(0).
Let a be vdNewFrag().
Let b be vdNewFrag().
Let c be vdNewFrag().
vdOpen(\"dl\").
row(a, \"a\").
row(b, \"b\").
row(c, \"c\").
vdClose().
Show vdCommit(root).
vdDropPatches().
vdOpen(\"dl\").
row(c, \"c\").
row(a, \"a\").
row(b, \"b\").
vdClose().
Show vdCommit(root).
Show summarize(vdPatchWords()).
Show vdSerializeChildren(root).
"
    );
    assert_all_tiers(
        &src,
        "InsertBefore,InsertBefore\n<dl><dt>c</dt><dd>c</dd><dt>a</dt><dd>a</dd><dt>b</dt><dd>b</dd></dl>",
    );
}

#[test]
fn a_kept_fragment_moves_into_its_new_parent_without_recreation() {
    let src = format!(
        "{HELPERS}
## Main
Let root be vdNewRoot(0).
Let f be vdNewFrag().
vdOpenKeyed(\"div\", \"a\").
vdFragOpen(f).
vdOpen(\"span\").
vdText(\"s\").
vdClose().
vdFragClose().
vdClose().
vdOpenKeyed(\"div\", \"b\").
vdClose().
Show vdCommit(root).
vdDropPatches().
vdOpenKeyed(\"div\", \"a\").
vdClose().
vdOpenKeyed(\"div\", \"b\").
vdKeep(f).
vdClose().
Show vdCommit(root).
Show summarize(vdPatchWords()).
Show vdGoneCount().
Show vdSerializeChildren(root).
"
    );
    assert_all_tiers(
        &src,
        "InsertBefore\n0\n<div></div><div><span>s</span></div>",
    );
}

#[test]
fn a_fragment_inside_a_removed_subtree_survives_being_placed_elsewhere() {
    // The div holding the fragment is removed in the same commit that places the fragment in a
    // new `<p>`; the fragment's node moves out before the div is freed, and is not reported gone.
    let src = format!(
        "{HELPERS}
## Main
Let root be vdNewRoot(0).
Let f be vdNewFrag().
vdOpenKeyed(\"div\", \"x\").
vdFragOpen(f).
vdOpen(\"span\").
vdText(\"s\").
vdClose().
vdFragClose().
vdClose().
Show vdCommit(root).
vdDropPatches().
vdOpen(\"p\").
vdKeep(f).
vdClose().
Show vdCommit(root).
Show summarize(vdPatchWords()).
Show vdGoneCount().
Show vdSerializeChildren(root).
"
    );
    assert_all_tiers(
        &src,
        "CreateElement,AppendChild,InsertBefore,FreeNode\n0\n<p><span>s</span></p>",
    );
}

// ---- Trim, the large keyed middle, and depth. ----

const ROWS: &str = "
## To rows(n: Int, changed: Int):
    vdOpen(\"ul\").
    Let mutable i be 1.
    While i <= n:
        vdOpenKeyed(\"li\", \"k{i}\").
        If i == changed:
            vdText(\"changed\").
        Otherwise:
            vdText(\"t{i}\").
        vdClose().
        Set i to i + 1.
    vdClose().
";

#[test]
fn an_unchanged_repaint_emits_nothing_and_one_changed_row_emits_one_set_text() {
    let src = format!(
        "{HELPERS}{ROWS}
## Main
Let root be vdNewRoot(0).
rows(100, 0).
Show vdCommit(root).
vdDropPatches().
rows(100, 0).
Show vdCommit(root).
Show vdPatchLength().
rows(100, 50).
Show vdCommit(root).
Show summarize(vdPatchWords()).
"
    );
    assert_all_tiers(&src, "0\n\nSetText");
}

#[test]
fn tail_trim_keeps_the_trailing_unkeyed_rows_in_place() {
    // Matching the unkeyed `<p>` from the tail keeps the last paragraph where it is and frees the
    // first; matching it from the head instead would rewrite the first's text and move it.
    let src = format!(
        "{HELPERS}
## Main
Let root be vdNewRoot(0).
vdOpen(\"p\").
vdText(\"1\").
vdClose().
vdOpen(\"div\").
vdText(\"x\").
vdClose().
vdOpen(\"p\").
vdText(\"2\").
vdClose().
Show vdCommit(root).
vdDropPatches().
vdOpen(\"div\").
vdText(\"x\").
vdClose().
vdOpen(\"p\").
vdText(\"2\").
vdClose().
Show vdCommit(root).
Show summarize(vdPatchWords()).
Show vdSerializeChildren(root).
"
    );
    assert_all_tiers(&src, "FreeNode\n<div>x</div><p>2</p>");
}

/// A deterministic shuffle of `1..=n` (xorshift), the same on every run.
fn shuffled(n: i64, mut seed: u64) -> Vec<i64> {
    let mut v: Vec<i64> = (1..=n).collect();
    for i in (1..v.len()).rev() {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        let j = (seed % (i as u64 + 1)) as usize;
        v.swap(i, j);
    }
    v
}

#[test]
fn a_1000_row_keyed_shuffle_moves_exactly_the_rows_off_the_longest_run() {
    // Past 32 kept rows the keyed middle is matched by sorted key hashes instead of a scan. Every
    // row must still be found (no create, no free, no text rewrite), exactly the rows off the
    // longest increasing run of old positions must move, and the result must be the new order.
    let order = shuffled(1000, 0x5eed_2026_0926);
    let moves = order.len() as i64 - reference_lis_keep(&order).iter().sum::<i64>();
    let src = format!(
        "{HELPERS}
## To rowsFrom(order: Seq of Int):
    vdOpen(\"ul\").
    For each k in order:
        vdOpenKeyed(\"li\", \"k{{k}}\").
        vdText(\"t{{k}}\").
        vdClose().
    vdClose().

## Main
Let root be vdNewRoot(0).
Let mutable ids be a new Seq of Int.
Let mutable i be 1.
While i <= 1000:
    Push i to ids.
    Set i to i + 1.
rowsFrom(ids).
Show vdCommit(root).
vdDropPatches().
Let order be {}.
rowsFrom(order).
Show vdCommit(root).
Let words be vdPatchWords().
Show countOps(words, 15).
Show countOps(words, 1) + countOps(words, 2) + countOps(words, 3) + countOps(words, 21).
Let patched be vdSerializeChildren(root).
Let fresh be vdNewRoot(0).
rowsFrom(order).
Show vdCommit(fresh).
Show patched == vdSerializeChildren(fresh).
",
        seq_literal(&order)
    );
    assert!(
        moves > 900,
        "the shuffle must move most rows for this row to test the sorted path: {moves}"
    );
    assert_all_tiers(&src, &format!("{moves}\n0\n\ntrue"));
}

#[test]
fn a_deep_tree_commits_and_serializes_without_recursion() {
    let src = "\
## Main
Let root be vdNewRoot(0).
Let mutable d be 1.
While d <= 10000:
    vdOpen(\"div\").
    Set d to d + 1.
Set d to 1.
While d <= 10000:
    vdClose().
    Set d to d + 1.
Show vdCommit(root).
Show length of vdSerializeChildren(root).
Show vdLiveNodes().
";
    assert_all_tiers(src, "110000\n10001");
}

// ---- Transactions: a failed render leaves the tree and the stream as they were. ----

#[test]
fn an_aborted_paint_leaves_the_retained_tree_and_the_stream_untouched() {
    let src = format!(
        "{HELPERS}
## Main
Let root be vdNewRoot(0).
buildList([\"a\", \"b\"]).
Show vdCommit(root).
vdDropPatches().
Let before be vdSerializeChildren(root).
vdOpen(\"ul\").
vdOpenKeyed(\"li\", \"z\").
vdText(\"half\").
vdAbort().
Show vdPatchLength().
Show vdTapeLength().
buildList([\"a\", \"b\"]).
Show vdCommit(root).
Show vdPatchLength().
Show before == vdSerializeChildren(root).
"
    );
    assert_all_tiers(&src, "0\n0\n\n0\ntrue");
}

#[test]
fn tape_faults_are_refused_by_the_commit_with_the_tree_untouched() {
    let src = "\
## Main
Let root be vdNewRoot(0).
vdOpen(\"div\").
vdText(\"x\").
vdSetAttribute(\"class\", \"late\").
vdClose().
Show vdCommit(root).
vdOpen(\"div\").
Show vdCommit(root).
vdClose().
Show vdCommit(root).
Let f be vdNewFrag().
vdFragOpen(f).
vdClose().
vdFragClose().
Show vdCommit(root).
vdOpen(\"div\").
vdFragClose().
vdClose().
Show vdCommit(root).
vdFragOpen(f).
vdFragClose().
vdFragOpen(f).
vdFragClose().
Show vdCommit(root).
vdOpen(\"div\").
vdSetHtml(\"<b>x</b>\").
vdText(\"child\").
vdClose().
Show vdCommit(root).
Show vdCommit(7).
Show vdPatchLength().
Show vdLiveNodes().
";
    assert_all_tiers(
        src,
        "vdom: vdSetAttribute must follow its element's vdOpen, before any child\n\
         vdom: <div> was opened and never closed\n\
         vdom: vdClose with no element open\n\
         vdom: vdClose would close a fragment; a fragment ends with vdFragClose\n\
         vdom: vdFragClose would close an element; an element ends with vdClose\n\
         vdom: fragment 1 is placed twice in one render\n\
         vdom: <div> at tape position 1 sets raw html and also has children\n\
         vdom: commit into node 7, which does not exist\n\
         0\n\
         1",
    );
}

// ---- Modifiers: sent once, updated in place, removed when dropped. ----

#[test]
fn a_listener_is_sent_once_updated_on_new_flags_and_dropped_with_unlisten() {
    let src = format!(
        "{HELPERS}
## To btn(flags: Int, on: Bool):
    vdOpen(\"button\").
    If on:
        vdListen(\"click\", flags).
    vdText(\"go\").
    vdClose().

## Main
Let root be vdNewRoot(0).
btn(1, true).
Show vdCommit(root).
Show summarize(vdPatchWords()).
vdDropPatches().
btn(1, true).
Show vdCommit(root).
Show vdPatchLength().
btn(3, true).
Show vdCommit(root).
Show vdPatchWords().
vdDropPatches().
btn(3, false).
Show vdCommit(root).
Show summarize(vdPatchWords()).
"
    );
    assert_all_tiers(
        &src,
        "CreateElement,Listen,AppendChild,CreateText,AppendChild\n\n0\n\n[19, 2147483650, 1, 3]\n\nUnlisten",
    );
}

#[test]
fn attributes_are_set_only_when_changed_and_removed_when_dropped() {
    let src = format!(
        "{HELPERS}
## To el(a: Text, b: Text, withB: Bool):
    vdOpen(\"div\").
    vdSetAttribute(\"a\", a).
    If withB:
        vdSetAttribute(\"b\", b).
    vdClose().

## Main
Let root be vdNewRoot(0).
el(\"1\", \"2\", true).
Show vdCommit(root).
vdDropPatches().
el(\"1\", \"2\", true).
Show vdCommit(root).
Show vdPatchLength().
el(\"1\", \"3\", true).
Show vdCommit(root).
Show summarize(vdPatchWords()).
vdDropPatches().
el(\"1\", \"3\", false).
Show vdCommit(root).
Show summarize(vdPatchWords()).
Show vdSerializeChildren(root).
"
    );
    assert_all_tiers(
        &src,
        "0\n\nSetAttribute\n\nRemoveAttribute\n<div a=\"1\"></div>",
    );
}

#[test]
fn raw_html_and_children_switch_both_ways() {
    let src = format!(
        "{HELPERS}
## Main
Let root be vdNewRoot(0).
vdOpen(\"div\").
vdText(\"kid\").
vdClose().
Show vdCommit(root).
vdDropPatches().
vdOpen(\"div\").
vdSetHtml(\"<b>raw</b>\").
vdClose().
Show vdCommit(root).
Show summarize(vdPatchWords()).
Show vdSerializeChildren(root).
vdDropPatches().
vdOpen(\"div\").
vdText(\"kid2\").
vdClose().
Show vdCommit(root).
Show summarize(vdPatchWords()).
Show vdSerializeChildren(root).
"
    );
    assert_all_tiers(
        &src,
        "SetHtml,FreeNode\n<div><b>raw</b></div>\n\nSetHtml,CreateText,AppendChild\n<div>kid2</div>",
    );
}

#[test]
fn a_root_is_addressed_by_its_host_id() {
    let src = "\
## Main
Let root be vdNewRoot(7).
vdOpen(\"p\").
vdClose().
Show vdCommit(root).
Show vdPatchWords().
";
    assert_all_tiers(src, "[1, 2147483650, 1, 7, 7, 2147483650]");
}

#[test]
fn a_steady_state_repaint_grows_nothing() {
    // Once one repaint has warmed the scratch arrays, twenty more — each changing a different
    // row — leave every array the module keeps at the same size and the node count unchanged.
    let src = format!(
        "{HELPERS}{ROWS}
## Main
Let root be vdNewRoot(0).
rows(50, 0).
Show vdCommit(root).
vdDropPatches().
rows(50, 7).
Show vdCommit(root).
vdDropPatches().
rows(50, 0).
Show vdCommit(root).
vdDropPatches().
Let settled be vdFootprint().
Let nodes be vdLiveNodes().
Let mutable turn be 1.
While turn <= 20:
    rows(50, turn).
    Show vdCommit(root).
    vdDropPatches().
    Set turn to turn + 1.
Show vdFootprint() == settled.
Show vdLiveNodes() == nodes.
"
    );
    assert_all_tiers(&src, "true\ntrue");
}

// ---- Serialization: the markup a host serializes the same tree to. ----

#[test]
fn serialization_escapes_voids_and_sorts_like_the_host() {
    // Text escapes `&<>`, attribute values `&<>"`; a void element has no content and no close;
    // attributes print sorted by name, then `checked`, then the value property; a raw text
    // element's text is verbatim; a `<select>` prints its value only where it differs from the
    // option its markup already selects.
    let src = format!(
        "{HELPERS}
## Main
Let root be vdNewRoot(0).
vdOpen(\"a\").
vdSetAttribute(\"title\", \"<\\\"&>\").
vdSetAttribute(\"href\", \"x&y\").
vdText(\"a & b <c> \\\"q\\\"\").
vdClose().
vdOpen(\"br\").
vdClose().
vdOpen(\"input\").
vdSetAttribute(\"type\", \"checkbox\").
vdSetChecked(true).
vdSetValue(\"v\\\"\").
vdClose().
vdOpen(\"style\").
vdText(\".a > b {{{{ }}}}\").
vdClose().
vdOpen(\"select\").
vdSetValue(\"b\").
vdOpen(\"option\").
vdSetAttribute(\"value\", \"a\").
vdText(\"A\").
vdClose().
vdOpen(\"option\").
vdSetAttribute(\"value\", \"b\").
vdSetAttribute(\"selected\", \"\").
vdText(\"B\").
vdClose().
vdClose().
vdOpen(\"select\").
vdSetValue(\"a\").
vdOpen(\"option\").
vdText(\"a\").
vdClose().
vdOpen(\"option\").
vdSetAttribute(\"selected\", \"\").
vdText(\"b\").
vdClose().
vdClose().
Show vdCommit(root).
Show vdSerializeChildren(root).
"
    );
    assert_all_tiers(
        &src,
        "<a href=\"x&amp;y\" title=\"&lt;&quot;&amp;&gt;\">a &amp; b &lt;c&gt; \"q\"</a><br>\
         <input type=\"checkbox\" checked value=\"v&quot;\"><style>.a > b { }</style>\
         <select><option value=\"a\">A</option><option selected=\"\" value=\"b\">B</option></select>\
         <select value=\"a\"><option>a</option><option selected=\"\">b</option></select>",
    );
}

// ---- The wire format: a real commit's stream is what `patch::decode_words` reads. ----

/// Parses a `Show` of a `Seq of Int` (`[1, 2, 3]`).
fn ints(line: &str) -> Vec<i64> {
    let body = line.trim().trim_start_matches('[').trim_end_matches(']');
    if body.trim().is_empty() {
        return Vec::new();
    }
    body.split(',')
        .map(|w| w.trim().parse().expect("an Int word"))
        .collect()
}

#[test]
fn a_real_commit_decodes_through_the_word_format() {
    // The program prints the stream's words, then each string as its UTF-8 bytes (so no string
    // content can be mistaken for a separator), then the node ids it expects the stream to name.
    let src = format!(
        "{HELPERS}
## Main
Let root be vdNewRoot(0).
vdOpen(\"ul\").
vdSetAttribute(\"class\", \"a \\\"b\\\"\").
vdListen(\"click\", 1).
vdOpenKeyed(\"li\", \"k\").
vdSetValue(\"v\").
vdSetChecked(true).
vdText(\"x, y\").
vdClose().
vdClose().
Show vdCommit(root).
Show vdPatchWords().
For each s in vdPatchStrings():
    Show text_bytes(s).
Show \"end\".
"
    );
    let agreed = run_all_tiers(&src);
    assert!(
        agreed.error.is_none(),
        "the program runs: {:?}",
        agreed.error
    );
    let mut lines = agreed.output.lines().filter(|l| !l.is_empty());
    let words = ints(lines.next().expect("the words line"));
    let mut strings = Vec::new();
    for line in lines.by_ref() {
        if line == "end" {
            break;
        }
        let bytes: Vec<u8> = ints(line).into_iter().map(|b| b as u8).collect();
        strings.push(String::from_utf8(bytes).expect("UTF-8"));
    }
    let ops = logicaffeine_compile::semantics::patch::decode_words(&words, &strings)
        .unwrap_or_else(|e| {
            panic!(
                "vdom.lg's stream is not valid wire v2: {e}\nwords {words:?}\nstrings {strings:?}"
            )
        });
    use logicaffeine_compile::semantics::dom::DomOp;
    // Nodes: root 1 (the host's body, 0), ul 2, li 3, the text 4 — each named on the wire in the
    // stream range, `node + 2^31`.
    let w = |node: u32| node + (1 << 31);
    assert_eq!(
        ops,
        vec![
            DomOp::CreateElement {
                node: w(2),
                tag: "ul".into()
            },
            DomOp::SetAttribute {
                node: w(2),
                name: "class".into(),
                value: "a \"b\"".into()
            },
            DomOp::Listen {
                node: w(2),
                event: "click".into(),
                flags: 1
            },
            DomOp::AppendChild {
                parent: 0,
                child: w(2)
            },
            DomOp::CreateElement {
                node: w(3),
                tag: "li".into()
            },
            DomOp::SetValue {
                node: w(3),
                value: "v".into()
            },
            DomOp::SetChecked {
                node: w(3),
                checked: true
            },
            DomOp::AppendChild {
                parent: w(2),
                child: w(3)
            },
            DomOp::CreateTextNode {
                node: w(4),
                text: "x, y".into()
            },
            DomOp::AppendChild {
                parent: w(3),
                child: w(4)
            },
        ]
    );
}

// ---- Source censuses. ----

fn vdom_source() -> &'static str {
    include_str!("../../logicaffeine_compile/assets/std/vdom.lg")
}

/// The body of the `## To <name>` declaration in `src`, up to the next `##` header.
fn function_body<'a>(src: &'a str, name: &str) -> &'a str {
    let header = format!("## To {name}(");
    let start = src
        .find(&header)
        .unwrap_or_else(|| panic!("vdom.lg declares `{name}`"));
    let rest = &src[start + header.len()..];
    let end = rest.find("\n## ").unwrap_or(rest.len());
    &rest[..end]
}

/// Every module-level `Seq` the `## Main` block of `src` declares, by name.
fn module_arrays(src: &str) -> Vec<String> {
    let main = src
        .find("## Main\n")
        .expect("vdom.lg declares its state in `## Main`");
    let block = &src[main + "## Main\n".len()..];
    let block = &block[..block.find("\n## ").unwrap_or(block.len())];
    block
        .lines()
        .filter_map(|l| l.strip_prefix("Let mutable "))
        .filter_map(|l| l.split_once(" be "))
        .filter(|(_, value)| value.starts_with('['))
        .map(|(name, _)| name.to_string())
        .collect()
}

#[test]
fn vd_footprint_counts_every_array_the_module_keeps() {
    let src = vdom_source();
    let arrays = module_arrays(src);
    assert!(
        arrays.len() >= 60,
        "read only {} module arrays out of vdom.lg's `## Main`",
        arrays.len()
    );
    let body = function_body(src, "vdFootprint");
    let uncounted: Vec<&String> = arrays
        .iter()
        .filter(|a| {
            !body.contains(&format!("length of {a} ")) && !body.contains(&format!("length of {a}."))
        })
        .collect();
    assert!(
        uncounted.is_empty(),
        "vdFootprint does not count {uncounted:?}, so a steady-state repaint could grow them unseen"
    );
}

#[test]
fn vd_footprint_census_can_fail() {
    let src = vdom_source().replace("length of __VD_SORTH + ", "");
    let body = function_body(&src, "vdFootprint");
    assert!(
        module_arrays(&src)
            .iter()
            .any(|a| !body.contains(&format!("length of {a} "))
                && !body.contains(&format!("length of {a}."))),
        "dropping one array from vdFootprint must be seen by the census above"
    );
}

#[test]
fn vdom_lg_splices_alone() {
    // A program naming one `vd` function must parse vdom.lg and nothing else: the module calls no
    // other stdlib function, and none of its locals sits where the demand scan reads a use. A
    // local named like another module's function (`Set x to html.`) splices that module and every
    // module it reaches into every page.
    let spliced =
        logicaffeine_compile::loader::apply_prelude("## Main\nShow vdLiveNodes().\n").into_owned();
    assert!(
        spliced.contains("## To vdCommit("),
        "vdom.lg is spliced for a program naming a vd function"
    );
    let vdom_code = vdom_source();
    let mut others = Vec::new();
    for module in logicaffeine_compile::loader::prelude_module_sources() {
        if std::ptr::eq(*module, vdom_code) || module.contains("## To vdCommit(") {
            continue;
        }
        let first = module
            .lines()
            .find(|l| l.starts_with("## To "))
            .map(str::to_string);
        if let Some(header) = first {
            if spliced.contains(&header) {
                others.push(header);
            }
        }
    }
    assert!(
        others.is_empty(),
        "a vdom program also spliced the modules declaring {others:?}"
    );
}

// ---- The host applies what vdom.lg emits: every commit, applied to the native recorder by the ids
// the stream names, leaves the recorder's document exactly as vdom.lg serializes its tree. ----

/// Prints the pending stream (its words, then each string as UTF-8 bytes), then the root's markup
/// as bytes, then drops the stream — one block per commit, so no content can pass for a separator.
const DUMP: &str = "
## To dump(root: Int):
    Show vdPatchWords().
    For each s in vdPatchStrings():
        Show text_bytes(s).
    Show \"--\".
    Show text_bytes(vdSerializeChildren(root)).
    Show \"==\".
    vdDropPatches().

## To commit(root: Int):
    Let problem be vdCommit(root).
    If problem != \"\":
        Show \"REFUSED \" + problem.
    dump(root).
";

/// One commit's stream and the markup vdom.lg serializes after it.
struct Commit {
    words: Vec<i64>,
    strings: Vec<String>,
    markup: String,
}

fn utf8(line: &str) -> String {
    String::from_utf8(ints(line).into_iter().map(|b| b as u8).collect()).expect("UTF-8")
}

/// Runs `main` (with the helpers) on every tier and reads back each `commit(root)` block.
fn commits(main: &str) -> Vec<Commit> {
    let src = format!("{HELPERS}{DUMP}\n## Main\n{main}");
    let agreed = run_all_tiers(&src);
    assert!(
        agreed.error.is_none(),
        "the program runs: {:?}\n{src}",
        agreed.error
    );
    assert!(
        !agreed.output.contains("REFUSED"),
        "a commit was refused:\n{}",
        agreed.output
    );
    let mut out = Vec::new();
    let mut lines = agreed.output.lines().filter(|l| !l.is_empty()).peekable();
    while lines.peek().is_some() {
        let words = ints(lines.next().unwrap());
        let mut strings = Vec::new();
        for line in lines.by_ref() {
            if line == "--" {
                break;
            }
            strings.push(utf8(line));
        }
        let markup = utf8(lines.next().expect("the markup line"));
        assert_eq!(
            lines.next(),
            Some("=="),
            "a commit block ends with its separator"
        );
        out.push(Commit {
            words,
            strings,
            markup,
        });
    }
    out
}

/// Applies every commit of `main` to a fresh native document through `patch::apply`, asserting
/// after each that the recorder renders exactly what vdom.lg serializes.
fn host_follows(main: &str, blocks: usize) {
    use logicaffeine_compile::semantics::{dom, patch};
    let commits = commits(main);
    assert_eq!(
        commits.len(),
        blocks,
        "the program committed {blocks} times"
    );
    dom::reset();
    for (k, c) in commits.iter().enumerate() {
        let ops = patch::decode_words(&c.words, &c.strings)
            .unwrap_or_else(|e| panic!("commit {k}: vdom.lg's stream is not wire v2: {e}"));
        patch::apply(&ops)
            .unwrap_or_else(|e| panic!("commit {k}: the host cannot honour the stream: {e}"));
        assert_eq!(
            dom::render(),
            format!("<body>{}</body>", c.markup),
            "commit {k}: the host's document"
        );
    }
}

#[test]
fn the_host_follows_every_list_shape_by_the_streams_own_ids() {
    let lists = [
        "[\"a\", \"b\", \"c\"]",
        "[\"a\", \"b\", \"c\", \"d\"]",
        "[\"d\", \"a\", \"b\", \"c\"]",
        "[\"a\", \"x\", \"c\"]",
        "[\"c\", \"b\", \"a\"]",
        "[\"x\", \"y\", \"z\"]",
        "[] of Text",
        "[\"q\", \"a\", \"x\", \"c\", \"e\", \"d\"]",
        "[\"e\", \"d\", \"c\"]",
    ];
    let main: String = std::iter::once("Let root be vdNewRoot(0).\n".to_string())
        .chain(
            lists
                .iter()
                .map(|l| format!("buildList({l}).\ncommit(root).\n")),
        )
        .collect();
    host_follows(&main, lists.len());
}

#[test]
fn the_host_follows_fragments_moving_between_parents_and_leaving() {
    let main = "\
Let root be vdNewRoot(0).
Let a be vdNewFrag().
Let b be vdNewFrag().
vdOpenKeyed(\"div\", \"one\").
vdFragOpen(a).
vdOpen(\"dt\").
vdText(\"a\").
vdClose().
vdOpen(\"dd\").
vdText(\"A\").
vdClose().
vdFragClose().
vdFragOpen(b).
vdOpen(\"dt\").
vdText(\"b\").
vdClose().
vdFragClose().
vdClose().
vdOpenKeyed(\"div\", \"two\").
vdClose().
commit(root).
vdOpenKeyed(\"div\", \"one\").
vdKeep(b).
vdClose().
vdOpenKeyed(\"div\", \"two\").
vdKeep(a).
vdClose().
commit(root).
vdOpenKeyed(\"div\", \"two\").
vdKeep(a).
vdClose().
commit(root).
vdOpen(\"p\").
vdKeep(a).
vdClose().
commit(root).
";
    host_follows(main, 4);
}

#[test]
fn the_host_follows_modifiers_escapes_voids_and_raw_html() {
    let main = "\
Let root be vdNewRoot(0).
vdOpen(\"a\").
vdSetAttribute(\"title\", \"a\\\"b&c<d>e'f`g\").
vdSetAttribute(\"href\", \"x&y\").
vdText(\"a\\\"b&c<d>e'f`g\").
vdClose().
vdOpen(\"br\").
vdClose().
vdOpen(\"input\").
vdSetAttribute(\"type\", \"checkbox\").
vdSetAttribute(\"disabled\", \"\").
vdSetChecked(true).
vdSetValue(\"v\\\"\").
vdClose().
vdOpen(\"select\").
vdSetValue(\"b\").
vdOpen(\"option\").
vdSetAttribute(\"value\", \"a\").
vdText(\"A\").
vdClose().
vdOpen(\"option\").
vdSetAttribute(\"value\", \"b\").
vdSetAttribute(\"selected\", \"\").
vdText(\"B\").
vdClose().
vdClose().
vdOpen(\"div\").
vdSetHtml(\"<b>raw</b>\").
vdClose().
commit(root).
vdOpen(\"a\").
vdSetAttribute(\"href\", \"x&y\").
vdText(\"changed\").
vdClose().
vdOpen(\"br\").
vdClose().
vdOpen(\"input\").
vdSetAttribute(\"type\", \"checkbox\").
vdSetChecked(false).
vdSetValue(\"w\").
vdClose().
vdOpen(\"select\").
vdSetValue(\"a\").
vdOpen(\"option\").
vdSetAttribute(\"value\", \"a\").
vdText(\"A\").
vdClose().
vdClose().
vdOpen(\"div\").
vdText(\"kid\").
vdClose().
commit(root).
";
    host_follows(main, 2);
}

#[test]
fn the_host_serializes_a_raw_text_elements_text_child_verbatim() {
    // A `<style>`'s content is CSS, not markup: HTML serializes the text node under a raw text
    // element literally, so `.a > b` stays `.a > b` in the host's document as in vdom.lg's.
    let main = "\
Let root be vdNewRoot(0).
vdOpen(\"style\").
vdText(\".a > b & c\").
vdClose().
commit(root).
";
    host_follows(main, 1);
}

// ---- `vdApply`: the stream reaches the page's host in one call, on every tier. ----

/// A first paint and a keyed repaint, each committed and handed to the host by `vdApply()`.
const APPLIED: &str = "\
## Main
Let root be vdNewRoot(0).
vdOpen(\"ul\").
vdOpenKeyed(\"li\", \"a\").
vdText(\"a\").
vdClose().
vdOpenKeyed(\"li\", \"b\").
vdText(\"b\").
vdClose().
vdClose().
Show vdCommit(root).
Show vdApply().
Show vdPatchLength().
vdOpen(\"ul\").
vdOpenKeyed(\"li\", \"b\").
vdText(\"b\").
vdClose().
vdOpenKeyed(\"li\", \"c\").
vdSetAttribute(\"class\", \"new\").
vdText(\"c\").
vdClose().
vdOpenKeyed(\"li\", \"a\").
vdText(\"A\").
vdClose().
vdClose().
Show vdCommit(root).
Show vdApply().
Show vdSerializeChildren(root).
";

const APPLIED_OUTPUT: &str = "0\n0\n\n0\n<ul><li>b</li><li class=\"new\">c</li><li>A</li></ul>";

/// The host document the program leaves behind, rendered.
fn host_after(run: impl FnOnce()) -> String {
    use logicaffeine_compile::semantics::dom;
    dom::reset();
    run();
    dom::render()
}

#[test]
fn vd_apply_reaches_the_host_on_every_tier() {
    use logicaffeine_compile::compile::{tw_outcome, vm_outcome};
    let expected = "<body><ul><li>b</li><li class=\"new\">c</li><li>A</li></ul></body>";
    let tw = host_after(|| assert_eq!(tw_outcome(APPLIED).output.trim(), APPLIED_OUTPUT));
    assert_eq!(tw, expected, "the tree-walker's host document");
    let vm = host_after(|| assert_eq!(vm_outcome(APPLIED).output.trim(), APPLIED_OUTPUT));
    assert_eq!(vm, expected, "the VM's host document");
    #[cfg(feature = "wasm-jit")]
    {
        let wasm = host_after(|| {
            use compile_common::wasm_host::{link_native_stream, run_outcome_hosted};
            let run = run_outcome_hosted(APPLIED, None, link_native_stream)
                .unwrap_or_else(|why| panic!("the wasm module does not run: {why}"));
            assert_eq!((run.output.trim(), run.failure), (APPLIED_OUTPUT, None));
        });
        assert_eq!(wasm, expected, "the wasm module's host document");
    }
}

// ---- Wave 7.5: what a lowered page renders through — sited text, slot ranges, sites on demand. ----

#[test]
fn a_sited_text_resolves_to_its_node_and_patches_in_place() {
    let src = format!(
        "{HELPERS}
## Main
Let root be vdNewRoot(0).
Let s be vdNewSite().
vdOpen(\"p\").
vdTextSite(\"a\", s).
vdClose().
Show vdCommit(root).
vdDropPatches().
vdPatchText(vdSiteNode(s), \"b\").
Show summarize(vdPatchWords()).
vdDropPatches().
vdPatchText(vdSiteNode(s), \"b\").
Show summarize(vdPatchWords()).
Show vdSerializeChildren(root).
vdOpen(\"p\").
vdTextSite(\"b\", s).
vdClose().
Show vdCommit(root).
Show summarize(vdPatchWords()).
"
    );
    assert_all_tiers(&src, "SetText\n\n<p>b</p>");
}

#[test]
fn a_callers_range_is_placed_by_the_slot_that_names_it() {
    // The caller draws its children where it is (inside the range, apart from the tape around
    // it); the callee places them where its slot stands. A range no slot names is not drawn.
    let src = format!(
        "{HELPERS}
## Main
Let root be vdNewRoot(0).
vdOpen(\"div\").
Let r be vdRangeOpen().
vdOpen(\"b\").
vdText(\"x\").
vdClose().
vdRangeClose().
Let unused be vdRangeOpen().
vdText(\"never\").
vdRangeClose().
vdOpen(\"section\").
vdText(\"before \").
vdSlot(r).
vdText(\" after\").
vdClose().
vdClose().
Show vdCommit(root).
Show vdSerializeChildren(root).
Show summarize(vdPatchWords()).
"
    );
    assert_all_tiers(
        &src,
        "<div><section>before <b>x</b> after</section></div>\nCreateElement,AppendChild,CreateElement,AppendChild,CreateText,CreateElement,CreateText,AppendChild,AppendChild,AppendChild,CreateText,AppendChild",
    );
}

#[test]
fn a_range_placed_inside_another_range_resolves_through_both() {
    // A wrapper placed in a caller's children hands its own children down again: the outer range
    // holds a slot naming the inner one.
    let src = format!(
        "{HELPERS}
## Main
Let root be vdNewRoot(0).
Let inner be vdRangeOpen().
vdText(\"core\").
vdRangeClose().
Let outer be vdRangeOpen().
vdOpen(\"i\").
vdSlot(inner).
vdClose().
vdRangeClose().
vdOpen(\"main\").
vdSlot(outer).
vdClose().
Show vdCommit(root).
Show vdSerializeChildren(root).
"
    );
    assert_all_tiers(&src, "<main><i>core</i></main>");
}

#[test]
fn a_placed_range_keeps_its_fragments_and_their_ends() {
    // Every OPEN and FRAG_OPEN copied into its slot is re-pointed at its own close, so the diff
    // reads the resolved tape exactly as it reads one drawn in place; a second paint of the same
    // shape is empty.
    let src = format!(
        "{HELPERS}
## Main
Let root be vdNewRoot(0).
Let f be vdNewFrag().
Let r be vdRangeOpen().
vdFragOpen(f).
vdOpen(\"em\").
vdText(\"a\").
vdClose().
vdFragClose().
vdRangeClose().
vdOpen(\"div\").
vdSlot(r).
vdOpen(\"hr\").
vdClose().
vdClose().
Show vdCommit(root).
Show vdSerializeChildren(root).
vdDropPatches().
Let r2 be vdRangeOpen().
vdFragOpen(f).
vdOpen(\"em\").
vdText(\"a\").
vdClose().
vdFragClose().
vdRangeClose().
vdOpen(\"div\").
vdSlot(r2).
vdOpen(\"hr\").
vdClose().
vdClose().
Show vdCommit(root).
Show summarize(vdPatchWords()).
Show vdFragNode(f) != 0.
"
    );
    assert_all_tiers(&src, "<div><em>a</em><hr></div>\n\n\ntrue");
}

#[test]
fn a_slot_naming_no_range_is_refused_and_the_tree_is_untouched() {
    let src = format!(
        "{HELPERS}
## Main
Let root be vdNewRoot(0).
vdOpen(\"div\").
vdClose().
Show vdCommit(root).
vdDropPatches().
vdOpen(\"p\").
vdSlot(1).
vdClose().
Show vdCommit(root).
Show summarize(vdPatchWords()).
Show vdSerializeChildren(root).
"
    );
    assert_all_tiers(
        &src,
        "vdom: a slot names tape position 1, which opens no range\n\n<div></div>",
    );
}

#[test]
fn a_sites_ids_are_handed_back_and_reused() {
    let src = format!(
        "{HELPERS}
## Main
Let a be vdNewSite().
Let b be vdNewSite().
Show a != b.
vdFreeSite(a).
Show vdNewSite() == a.
Show vdNewSite() != b.
"
    );
    assert_all_tiers(&src, "true\ntrue\ntrue");
}

#[test]
fn a_host_id_finds_its_node_and_a_root_by_its_host() {
    let src = format!(
        "{HELPERS}
## Main
Let root be vdNewRoot(7).
vdOpen(\"div\").
vdClose().
Show vdCommit(root).
Let words be vdPatchWords().
Let wire be item 2 of words.
Show vdWire(vdNodeOfWire(wire)) == wire.
Show vdNodeOfWire(7) == root.
Show vdNodeOfWire(8).
Show vdNodeOfWire(wire + 1000).
"
    );
    assert_all_tiers(&src, "true\ntrue\n-1\n-1");
}

#[test]
fn a_listener_found_after_its_open_rides_beside_the_tape() {
    // A sentence finds its element once the view is drawn: its listener and site are recorded
    // against the open's position, applied with the open's own modifiers, and follow the open into
    // a slot.
    let src = format!(
        "{HELPERS}
## Main
Let root be vdNewRoot(0).
Let s be vdNewSite().
Let r be vdRangeOpen().
vdOpen(\"button\").
Let at be vdTapeLength() - 3.
vdText(\"go\").
vdClose().
vdRangeClose().
vdListenAt(at, \"click\", 0).
vdSiteAt(at, s).
Show vdTapeSite(at) == s.
vdOpen(\"div\").
vdSlot(r).
vdClose().
Show vdCommit(root).
Show summarize(vdPatchWords()).
Show vdSiteOf(vdSiteNode(s)) == s.
vdDropPatches().
vdOpen(\"div\").
vdOpen(\"button\").
vdText(\"go\").
vdClose().
vdClose().
Show vdCommit(root).
Show summarize(vdPatchWords()).
Show vdSiteNode(s).
"
    );
    assert_all_tiers(
        &src,
        "true\n\nCreateElement,AppendChild,CreateElement,Listen,AppendChild,CreateText,AppendChild\ntrue\n\nUnlisten\n0",
    );
}

#[test]
fn a_node_no_tape_marks_takes_a_site_on_demand_and_keeps_it() {
    let src = format!(
        "{HELPERS}
## Main
Let root be vdNewRoot(0).
Let s be vdEnsureSite(root).
Show s == vdEnsureSite(root).
Show vdSiteNode(s) == root.
Show vdSiteOf(root) == s.
"
    );
    assert_all_tiers(&src, "true\ntrue\ntrue");
}

#[test]
fn a_selector_finds_the_first_element_the_tape_draws_that_it_names() {
    // One compound per list entry; compound i's attributes sit at flat positions attrFrom[i] up
    // to attrFrom[i + 1]. `.card li.done` is a `li` with class `done` anywhere inside a `.card`.
    let src = format!(
        "{HELPERS}
## Main
vdOpen(\"div\").
vdSetAttribute(\"class\", \"card wide\").
vdOpen(\"li\").
vdSetAttribute(\"class\", \"todo\").
vdClose().
vdOpen(\"li\").
vdSetAttribute(\"class\", \"todo done\").
vdSetAttribute(\"data-n\", \"2\").
vdClose().
vdClose().
vdOpen(\"li\").
vdSetAttribute(\"class\", \"done\").
vdSetAttribute(\"id\", \"out\").
vdClose().
Let second be vdQuery(1, [\"\", \"li\"], [\"\", \"\"], [\"card\", \"done\"], [1, 1, 1], [\"\"], [\"\"], [0]).
Show vdTapeTag(second).
Show second > 1.
Let byId be vdQuery(1, [\"\"], [\"out\"], [\"\"], [1, 1], [\"\"], [\"\"], [0]).
Show byId > second.
Let byAttr be vdQuery(1, [\"li\"], [\"\"], [\"\"], [1, 2], [\"data-n\"], [\"2\"], [1]).
Show byAttr == second.
Show vdQuery(1, [\"li\"], [\"\"], [\"\"], [1, 2], [\"data-n\"], [\"3\"], [1]).
Show vdQuery(1, [\"section\", \"li\"], [\"\", \"\"], [\"\", \"\"], [1, 1, 1], [\"\"], [\"\"], [0]).
Show vdQuery(1, [\"LI\"], [\"\"], [\"wide\"], [1, 1], [\"\"], [\"\"], [0]).
Show vdQuery(1, [\"div\"], [\"\"], [\"wide card\"], [1, 1], [\"\"], [\"\"], [0]).
vdAbort().
"
    );
    assert_all_tiers(&src, "li\ntrue\ntrue\ntrue\n0\n0\n0\n1");
}

#[test]
fn a_selector_given_as_text_is_parsed_the_way_the_page_checks_parse_it() {
    let src = format!(
        "{HELPERS}
## Main
vdOpen(\"div\").
vdSetAttribute(\"class\", \"card wide\").
vdOpen(\"li\").
vdSetAttribute(\"class\", \"todo done\").
vdSetAttribute(\"data-n\", \"2\").
vdClose().
vdClose().
vdOpen(\"input\").
vdSetAttribute(\"id\", \"draft\").
vdClose().
Let li be vdQueryText(1, \".card li.done\").
Show vdTapeTag(li).
Show vdQueryText(1, \"LI[data-n=2]\") == li.
Show vdQueryText(1, \"li[data-n='2']\") == li.
Show vdQueryText(1, \"[data-n]\") == li.
Show vdTapeTag(vdQueryText(1, \"#draft\")).
Show vdQueryText(1, \"input#draft.big\").
Show vdQueryText(1, \"ul > li\").
Show vdQueryText(1, \"li:first-child\").
Show vdQueryText(1, \"a, li\").
Show vdQueryText(1, \"*\").
Show vdQueryText(1, \"   \").
Show vdQueryText(1, \"section li\").
vdAbort().
"
    );
    assert_all_tiers(&src, "li\ntrue\ntrue\ntrue\ninput\n0\n0\n0\n0\n0\n0\n0");
}

// ---- Wave 7.6: hydration — the first commit into a marked root adopts the markup its host holds. ----

/// The counter's first paint drawn onto the tape: a paragraph of two texts and a listening button.
fn counter_tape(count: &str) -> String {
    format!(
        "vdOpen(\"main\").
vdOpen(\"p\").
vdSetAttribute(\"id\", \"out\").
vdText(\"Count: \").
vdText(\"{count}\").
vdClose().
vdOpen(\"button\").
vdListen(\"click\", 0).
vdText(\"go\").
vdClose().
vdClose().
"
    )
}

#[test]
fn a_hydrating_commit_claims_the_markup_instead_of_creating_it() {
    let src = format!(
        "{HELPERS}
## Main
Let root be vdNewRoot(0).
vdHydrate(root).
{}Show vdCommit(root).
Show summarize(vdPatchWords()).
vdDropPatches().
{}Show vdCommit(root).
Show summarize(vdPatchWords()).
",
        counter_tape("0"),
        counter_tape("1"),
    );
    assert_all_tiers(
        &src,
        "Hydrate,Adopt,Adopt,SetAttribute,AdoptText,AdoptText,Adopt,Listen,AdoptText\n\nSetText",
    );
}

/// Markup a pre-render left under the body, built by the host's own calls — node ids the host
/// minted (1 `main`, 2 `p`, 3 its text, 4 `button`, 5 its text), none of them the stream's.
const SEEDED_COUNTER: &str = "Let m be createElement(\"main\").
appendChild(body(), m).
Let p be createElement(\"p\").
setAttribute(p, \"id\", \"out\").
appendChild(m, p).
appendChild(p, createTextNode(\"Count: 0\")).
Let b be createElement(\"button\").
appendChild(m, b).
appendChild(b, createTextNode(\"go\")).
";

/// The DOM imports a seeding `## Main` calls, answered by the native recorder, so a wasm module's
/// seeded markup lands in the one document every in-process tier draws into.
#[cfg(feature = "wasm-jit")]
fn link_native_document<T: 'static>(l: &mut wasmi::Linker<T>) {
    use compile_common::wasm_host::read_text;
    use logicaffeine_compile::semantics::dom;
    l.func_wrap("env", "dom_body", |_c: wasmi::Caller<'_, T>| -> i64 {
        dom::body()
    })
    .expect("dom_body");
    l.func_wrap(
        "env",
        "dom_create_element",
        |c: wasmi::Caller<'_, T>, tag: i32| -> i64 { dom::create_element(&read_text(&c, tag)) },
    )
    .expect("dom_create_element");
    l.func_wrap(
        "env",
        "dom_create_text_node",
        |c: wasmi::Caller<'_, T>, text: i32| -> i64 { dom::create_text_node(&read_text(&c, text)) },
    )
    .expect("dom_create_text_node");
    l.func_wrap(
        "env",
        "dom_append_child",
        |_c: wasmi::Caller<'_, T>, parent: i64, child: i64| dom::append_child(parent, child),
    )
    .expect("dom_append_child");
    l.func_wrap(
        "env",
        "dom_set_attribute",
        |c: wasmi::Caller<'_, T>, el: i64, name: i32, value: i32| {
            dom::set_attribute(el, &read_text(&c, name), &read_text(&c, value))
        },
    )
    .expect("dom_set_attribute");
}

/// The host document `main` leaves on every tier, each checked by `check` after its run, with
/// `output` the program's own output on each.
fn hydrated_on_every_tier(main: &str, output: &str, check: impl Fn(&str)) {
    use logicaffeine_compile::compile::{tw_outcome, vm_outcome};
    let src = format!("{HELPERS}\n## Main\n{main}");
    let tw = host_after(|| assert_eq!(tw_outcome(&src).output.trim(), output, "tree-walker"));
    check(&tw);
    let vm = host_after(|| assert_eq!(vm_outcome(&src).output.trim(), output, "VM"));
    check(&vm);
    #[cfg(feature = "wasm-jit")]
    {
        let wasm = host_after(|| {
            use compile_common::wasm_host::{link_native_stream, run_outcome_hosted};
            let run = run_outcome_hosted(&src, None, |l| {
                link_native_stream(l);
                link_native_document(l);
            })
            .unwrap_or_else(|why| panic!("the wasm module does not run: {why}"));
            assert_eq!(
                (run.output.trim(), run.failure.as_deref()),
                (output, None),
                "wasm"
            );
        });
        check(&wasm);
    }
}

/// Whether the recorder's op log shows the stream creating any node: a hydration that adopted its
/// markup creates none.
fn stream_created_any() -> bool {
    use logicaffeine_compile::semantics::dom::{self, DomOp, STREAM_ID_BASE};
    dom::op_log().iter().any(|op| {
        matches!(op, DomOp::CreateElement { node, .. } | DomOp::CreateTextNode { node, .. } if *node >= STREAM_ID_BASE)
    })
}

#[test]
fn hydration_binds_the_hosts_own_nodes_and_later_paints_patch_them() {
    let main = format!(
        "{SEEDED_COUNTER}Let root be vdNewRoot(0).
vdHydrate(root).
{}Show vdCommit(root).
Show vdApply().
{}Show vdCommit(root).
Show vdApply().
",
        counter_tape("0"),
        counter_tape("1"),
    );
    hydrated_on_every_tier(&main, "0\n\n0", |doc| {
        use logicaffeine_compile::semantics::dom;
        assert_eq!(
            doc,
            "<body><main><p id=\"out\">Count: 1</p><button>go</button></main></body>"
        );
        assert!(
            !stream_created_any(),
            "the stream adopts the markup, it creates nothing: {:#?}",
            dom::op_log()
        );
        assert_eq!(
            dom::text_content(2),
            "Count: 1",
            "the host's own paragraph is the one the repaint patched"
        );
        assert_eq!(
            dom::get_text(3),
            "Count: ",
            "and its text node is the claim it split off first"
        );
    });
}

#[test]
fn a_hydration_that_mismatches_is_discarded_and_drawn_afresh() {
    let main = format!(
        "Let m be createElement(\"main\").
appendChild(body(), m).
Let s be createElement(\"span\").
appendChild(m, s).
appendChild(s, createTextNode(\"stale\")).
Let root be vdNewRoot(0).
vdHydrate(root).
{}Show vdCommit(root).
Show vdApply() > 0.
{}Show vdCommit(root).
Show vdApply().
",
        counter_tape("0"),
        counter_tape("1"),
    );
    hydrated_on_every_tier(&main, "true\n\n0", |doc| {
        assert_eq!(
            doc, "<body><main><p id=\"out\">Count: 1</p><button>go</button></main></body>",
            "the markup no claim bound is gone and the page is drawn once, afresh"
        );
    });
}

#[test]
fn text_claims_split_the_markups_text_along_adjacent_and_empty_holes() {
    let main = "Let p be createElement(\"p\").
appendChild(body(), p).
appendChild(p, createTextNode(\"ab\")).
Let root be vdNewRoot(0).
vdHydrate(root).
vdOpen(\"p\").
vdText(\"a\").
vdText(\"\").
vdText(\"b\").
vdClose().
Show vdCommit(root).
Show vdApply().
vdOpen(\"p\").
vdText(\"a\").
vdText(\"X\").
vdText(\"b\").
vdClose().
Show vdCommit(root).
Show vdApply().
";
    hydrated_on_every_tier(main, "0\n\n0", |doc| {
        use logicaffeine_compile::semantics::dom;
        assert_eq!(
            doc, "<body><p>aXb</p></body>",
            "the empty hole is its own node, between the two it split"
        );
        assert_eq!(
            dom::get_text(2),
            "a",
            "the markup's text node is the first claim's"
        );
    });
}

#[test]
fn text_the_markup_spells_differently_is_rebuilt_without_a_mismatch() {
    let main = "Let p be createElement(\"p\").
appendChild(body(), p).
appendChild(p, createTextNode(\"Count: 7\")).
Let root be vdNewRoot(0).
vdHydrate(root).
vdOpen(\"p\").
vdText(\"Count: \").
vdText(\"0\").
vdClose().
Show vdCommit(root).
Show vdApply().
";
    hydrated_on_every_tier(main, "0", |doc| {
        use logicaffeine_compile::semantics::dom;
        assert_eq!(doc, "<body><p>Count: 0</p></body>");
        assert!(
            !stream_created_any_element(),
            "the paragraph is adopted: {:#?}",
            dom::op_log()
        );
    });
}

/// Whether the stream created an element — text a hydration rebuilds is created, elements never.
fn stream_created_any_element() -> bool {
    use logicaffeine_compile::semantics::dom::{self, DomOp, STREAM_ID_BASE};
    dom::op_log()
        .iter()
        .any(|op| matches!(op, DomOp::CreateElement { node, .. } if *node >= STREAM_ID_BASE))
}

#[test]
fn an_adopted_element_drops_attributes_its_paint_does_not_write_and_the_hosts_own_stay() {
    let main = "Let panel be createElement(\"div\").
setAttribute(panel, \"data-logos-host\", \"\").
appendChild(body(), panel).
Let m be createElement(\"main\").
setAttribute(m, \"class\", \"stale\").
setAttribute(m, \"id\", \"page\").
appendChild(body(), m).
Let root be vdNewRoot(0).
vdHydrate(root).
vdOpen(\"main\").
vdSetAttribute(\"id\", \"page\").
vdClose().
Show vdCommit(root).
Show vdApply().
";
    hydrated_on_every_tier(main, "0", |doc| {
        assert_eq!(
            doc,
            "<body><div data-logos-host=\"\"></div><main id=\"page\"></main></body>"
        );
    });
}

#[test]
fn a_hydration_opens_only_on_the_first_commit_into_its_root() {
    let main = format!(
        "{SEEDED_COUNTER}Let root be vdNewRoot(0).
vdHydrate(root).
{}Show vdCommit(root).
Show vdApply().
vdHydrate(root).
{}Show vdCommit(root).
Show summarize(vdPatchWords()).
Show vdApply().
",
        counter_tape("0"),
        counter_tape("2"),
    );
    hydrated_on_every_tier(&main, "0\n\nSetText\n0", |doc| {
        assert_eq!(
            doc,
            "<body><main><p id=\"out\">Count: 2</p><button>go</button></main></body>"
        );
    });
}

#[test]
fn a_selects_value_is_sent_after_the_options_it_chooses_from() {
    let tape = "vdOpen(\"select\").
vdSetValue(\"m\").
vdOpen(\"option\").
vdSetAttribute(\"value\", \"s\").
vdText(\"S\").
vdClose().
vdOpen(\"option\").
vdSetAttribute(\"value\", \"m\").
vdText(\"M\").
vdClose().
vdClose().
";
    let src = format!(
        "{HELPERS}
## Main
Let root be vdNewRoot(0).
{tape}Show vdCommit(root).
Show summarize(vdPatchWords()).
vdDropPatches().
Let again be vdNewRoot(7).
vdHydrate(again).
{tape}Show vdCommit(again).
Show summarize(vdPatchWords()).
"
    );
    let agreed = run_all_tiers(&src);
    assert_eq!(agreed.error, None);
    let streams: Vec<&str> = agreed
        .output
        .trim()
        .lines()
        .filter(|l| !l.is_empty())
        .collect();
    assert_eq!(streams.len(), 2, "{}", agreed.output);
    for stream in streams {
        let ops: Vec<&str> = stream.split(',').collect();
        let value = ops
            .iter()
            .position(|op| *op == "SetValue")
            .unwrap_or_else(|| panic!("the value is sent: {stream}"));
        assert_eq!(
            ops.iter().filter(|op| **op == "SetValue").count(),
            1,
            "once: {stream}"
        );
        assert!(
            ops[value + 1..].iter().all(|op| !matches!(
                *op,
                "CreateElement"
                    | "CreateText"
                    | "AppendChild"
                    | "InsertBefore"
                    | "Adopt"
                    | "AdoptText"
            )),
            "after every option is made and placed, or claimed: {stream}"
        );
    }
}

#[test]
fn markup_no_claim_binds_leaves_and_the_rest_stays_adopted() {
    let main = format!(
        "{SEEDED_COUNTER}Let extra be createElement(\"aside\").
appendChild(body(), extra).
appendChild(extra, createTextNode(\"stale\")).
Let root be vdNewRoot(0).
vdHydrate(root).
{}Show vdCommit(root).
Show vdApply().
",
        counter_tape("0"),
    );
    hydrated_on_every_tier(&main, "0", |doc| {
        use logicaffeine_compile::semantics::dom;
        assert_eq!(
            doc, "<body><main><p id=\"out\">Count: 0</p><button>go</button></main></body>",
            "the markup no claim bound is gone"
        );
        assert!(
            !stream_created_any(),
            "and nothing was drawn again for it: {:#?}",
            dom::op_log()
        );
    });
}

// ---- A render drawn only to be serialized: a root no host holds, discarded whole. ----

#[test]
fn a_root_drawn_only_to_serialize_is_discarded_whole() {
    let src = format!(
        "{HELPERS}
## Main
Let before be vdLiveNodes().
Let r be vdNewRoot(-1).
Let f be vdNewFrag().
vdOpen(\"main\").
vdFragOpen(f).
vdOpen(\"p\").
vdSetAttribute(\"class\", \"a&b\").
vdText(\"x < y\").
vdClose().
vdFragClose().
vdClose().
Show vdCommit(r).
Show vdSerializeChildren(r).
vdDiscardRoot(r).
Show vdPatchLength().
Show vdLiveNodes() == before.
Show vdGoneCount().
Show vdGone(1) == f.
Show vdNodeOfWire(-1).
"
    );
    assert_all_tiers(
        &src,
        "<main><p class=\"a&amp;b\">x &lt; y</p></main>\n0\ntrue\n1\ntrue\n-1",
    );
}

#[test]
fn serializing_again_and_again_grows_nothing() {
    let src = format!(
        "{HELPERS}{ROWS}
## Main
Let mutable round be 1.
Let mutable settled be 0.
Let mutable nodes be 0.
While round <= 12:
    Let r be vdNewRoot(-1).
    rows(20, round).
    Let problem be vdCommit(r).
    Let markup be vdSerializeChildren(r).
    vdDiscardRoot(r).
    vdClearGone().
    If round == 2:
        Set settled to vdFootprint().
        Set nodes to vdLiveNodes().
    Set round to round + 1.
Show vdFootprint() == settled.
Show vdLiveNodes() == nodes.
Show vdPatchLength().
"
    );
    assert_all_tiers(&src, "true\ntrue\n0");
}

// ---- A render drawn inside another: the outer tape is stashed and comes back exactly. ----

#[test]
fn a_root_serialized_inside_another_draw_leaves_the_outer_tape_as_it_was() {
    let src = format!(
        "{HELPERS}
## To tapeWords() -> Text:
    Let mutable out be \"\".
    Let mutable p be 1.
    While p <= vdTapeLength():
        Set out to out + \"{{vdTapeItem(p)}},\".
        Set p to p + 1.
    Return out.

## To inner(label: Text) -> Text:
    Let h be vdTapeStash().
    Let r be vdNewRoot(-1).
    vdOpenKeyed(\"p\", label).
    vdSetAttribute(\"class\", \"inner\").
    vdText(label).
    vdClose().
    Let problem be vdCommit(r).
    Let markup be vdSerializeChildren(r).
    vdDiscardRoot(r).
    vdTapeUnstash(h).
    Return problem + markup.

## Main
Let root be vdNewRoot(0).
vdOpen(\"main\").
vdSetAttribute(\"id\", \"page\").
vdText(\"before\").
Let tape be tapeWords().
Let first be inner(\"a\").
Show first.
Show tapeWords() == tape.
Let h be vdTapeStash().
Let second be inner(\"b\").
vdTapeUnstash(h).
Show second.
Show tapeWords() == tape.
vdText(\"after\").
vdClose().
Show vdCommit(root).
Show vdSerializeChildren(root).
Show summarize(vdPatchWords()).
"
    );
    assert_all_tiers(
        &src,
        "<p class=\"inner\">a</p>\ntrue\n<p class=\"inner\">b</p>\ntrue\n\n<main id=\"page\">beforeafter</main>\nCreateElement,SetAttribute,AppendChild,CreateText,CreateText,AppendChild,AppendChild",
    );
}
