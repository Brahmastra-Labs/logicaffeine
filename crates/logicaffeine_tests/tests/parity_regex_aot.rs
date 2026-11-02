//! AOT-Rust lock for `regex.lg` — in particular the `Seq of Text` builders that the
//! entry-guard off-by-one panicked on.
//!
//! `regexSplit`/`regexFindAllText` call `reSlice`, a slice-copy `while j < hi { item j of tb }`.
//! The function-entry precondition guard asserted `hi <= tb.len()`, but the loop is EXCLUSIVE
//! (max 1-based index `hi-1`), so a legitimate `reSlice(tb, start, len+1)` — one past the end,
//! every access in range — hit a spurious `assert!` panic on AOT-Rust. The fix
//! (`entry_guard::hi_len_adjust`) relaxes the precondition to `hi <= len+1` only when the loop is
//! exclusive and indexed solely by the bare counter. This pins every regex surface on AOT-Rust so
//! that regression — or any future change to the bounds-elision entry guard — is caught.
#![allow(unused_imports, dead_code)]
mod common;
use common::assert_compiled_equals_interpreted;

#[test]
fn regex_all_on_aot_rust() {
    for src in [
        "## Main\nShow regexMatch(\"l.o\", \"hello\").\n",
        "## Main\nShow regexFind(\"l.o\", \"hello\").\n",
        "## Main\nShow length of regexFindAll(\"a\", \"banana\").\n",
        "## Main\nShow regexReplaceAll(\"a\", \"banana\", \"X\").\n",
        "## Main\nShow length of regexFindAllText(\"a\", \"banana\").\nShow item 2 of regexFindAllText(\"a\", \"banana\").\n",
        "## Main\nShow length of regexSplit(\",\", \"a,b,c\").\nShow item 2 of regexSplit(\",\", \"a,b,c\").\n",
    ] {
        assert_compiled_equals_interpreted(src);
    }
}
