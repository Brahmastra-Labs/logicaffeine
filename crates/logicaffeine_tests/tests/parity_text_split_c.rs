//! C-backend lock for the split-family `Seq of Text`-returning text functions.
//!
//! `assert_all_tiers` only checks tree-walker≡VM, so the C tier of these functions was never
//! verified. Auditing it exposed a C-path optimizer bug in the Oracle (`optimize::abstract_interp`):
//! it never widened a loop-mutated *collection length*, so a `(length of cur) > 0` guard folded
//! against a stale `[0,0]` length and the pass DROPPED the live `Push` inside a split-builder —
//! silently miscompiling `words` (split_whitespace) and `splitAfter` on C. That bug is now fixed
//! (`oracle_loop_length_soundness.rs` pins the minimal repro across all tiers), so `words` and
//! `splitAfter` are back to full 5-tier and their C output is locked here alongside the flat members.

#![allow(unused_imports, dead_code)]
mod common;
use common::assert_c_output;

#[test]
fn split_on_c() {
    assert_c_output(
        "## Main\nShow length of split(\"a,b,c\", \",\").\nShow item 2 of split(\"a,b,c\", \",\").\n",
        "3\nb",
    );
}

#[test]
fn lines_on_c() {
    assert_c_output(
        "## Main\nShow length of lines(\"x\ny\nz\").\nShow item 3 of lines(\"x\ny\nz\").\n",
        "3\nz",
    );
}

#[test]
fn split_n_on_c() {
    assert_c_output(
        "## Main\nShow length of splitN(\"a,b,c,d\", \",\", 2).\n",
        "2",
    );
}

/// `words` (split_whitespace) — the split-builder whose live `Push` the Oracle used to drop on C.
#[test]
fn words_on_c() {
    assert_c_output(
        "## Main\nLet w be words(\"  hello   world  \").\nShow length of w.\nShow item 1 of w.\nShow item 2 of w.\n",
        "2\nhello\nworld",
    );
}

/// `splitAfter` — the other split-builder that shared the dropped-`Push` miscompile on C.
#[test]
fn split_after_on_c() {
    assert_c_output(
        "## Main\nLet p be splitAfter(\"a,b,c\", \",\").\nShow length of p.\nShow item 1 of p.\n",
        "3\na,",
    );
}

/// The regex `Seq of Text` builders `regexSplit`/`regexFindAllText` on C. (They also lower on
/// AOT-Rust now — the entry-guard off-by-one that panicked on `reSlice(tb, start, len+1)` is fixed;
/// `parity_regex_aot.rs` locks the AOT-Rust tier — so they are back to full 5-tier.)
#[test]
fn regex_seq_text_builders_on_c() {
    assert_c_output(
        "## Main\nShow length of regexSplit(\",\", \"a,b,c\").\nShow item 1 of regexSplit(\",\", \"a,b,c\").\n",
        "3\na",
    );
    assert_c_output(
        "## Main\nShow length of regexFindAllText(\"a\", \"banana\").\nShow item 2 of regexFindAllText(\"a\", \"banana\").\n",
        "3\na",
    );
}
