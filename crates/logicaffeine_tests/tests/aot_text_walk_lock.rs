//! ★ LOCK — walking a `Text` and asking whether it contains something COMPILE on the AOT tier. ★
//!
//! `For each c in t:` over a Text and `contains(haystack, needle)` are two of the most ordinary
//! sentences in the language — they are how the standard library escapes HTML, validates a number,
//! decodes a percent-escape and splits a header. They ran on the tree-walker and the VM and emitted
//! Rust that DOES NOT COMPILE:
//!
//! * `For each c in t` became `for c in t.clone()`, and a `String` is not an iterator.
//! * `contains(a, b)` became a call to a free function `contains` that does not exist.
//!
//! ★ WHY IT HID FOR SO LONG, and the lesson that matters more than the fix: `compile_to_rust`
//! returned `Ok`. The refusal is not an `Err` — it is invalid Rust in the returned STRING, which
//! fails later, when cargo builds it. Every test that asserted "compiles to Rust" by checking for
//! `Ok` was therefore green on a program that could not be built, and this whole class of gap sat
//! behind that one weak measure. These tests BUILD AND RUN the Rust and compare it to the
//! tree-walker, which is the only measure that cannot be satisfied by a program that never builds.
//!
//! ★ HOW IT SURFACED: `largo build` on a project with a `## Route`. Routes had never reached this
//! tier at all (their declaration emitted a `compile_error!`), so nothing had ever dragged
//! `web.lg`'s `htmlEscape` and `isIntText` into an AOT-Rust program — the moment routes lowered,
//! two stdlib functions the whole web surface rests on turned out not to compile. A gap in the
//! GENERAL language, found through the frontend, so the lock is written in plain LOGOS.

mod common;
use common::run_logos;

/// Build the Rust, run it, and hold the answer to what the reference tier says.
fn e2e(src: &str, expected: &str) {
    let tw = logicaffeine_compile::compile::tw_outcome(src);
    assert_eq!(tw.error, None, "tree-walker:\n{src}");
    assert_eq!(
        tw.output.trim_end(),
        expected,
        "the reference tier must agree with the test:\n{src}"
    );
    let r = run_logos(src);
    assert!(
        r.success,
        "the generated Rust must BUILD and RUN:\n{}\n{src}",
        r.stderr
    );
    assert_eq!(
        r.stdout.trim_end(),
        expected,
        "AOT must answer what the tree-walker answers:\n{src}"
    );
}

#[test]
fn a_for_each_over_a_text_runs_on_the_aot_tier() {
    // ★ `for c in t.clone()` — a `String` is not an iterator, so this never built.
    e2e(
        "## To countChars (t: Text) -> Int:\n\
        \x20   Let mutable n be 0.\n\
        \x20   For each c in t:\n\
        \x20       Set n to n + 1.\n\
        \x20   Return n.\n\
         \n\
         ## Main\n\
         Show countChars(\"abc\").\n",
        "3",
    );
}

#[test]
fn the_characters_are_the_real_ones_in_order() {
    // Counting them proves the loop RUNS; only reading them proves it walks the right thing.
    e2e(
        "## To firstLast (t: Text) -> Text:\n\
        \x20   Let mutable out be \"\".\n\
        \x20   For each c in t:\n\
        \x20       Set out to c + out.\n\
        \x20   Return out.\n\
         \n\
         ## Main\n\
         Show firstLast(\"abc\").\n",
        "cba",
    );
}

#[test]
fn a_multi_byte_character_is_one_character() {
    // ★ THE EDGE CASE A BYTE LOOP GETS WRONG. A check mark is three UTF-8 bytes and one character;
    // a fix that iterated bytes would count three and hand the body a fragment that is not a
    // character at all — the mojibake failure `urlDecode` was written to avoid.
    e2e(
        "## To countChars (t: Text) -> Int:\n\
        \x20   Let mutable n be 0.\n\
        \x20   For each c in t:\n\
        \x20       Set n to n + 1.\n\
        \x20   Return n.\n\
         \n\
         ## Main\n\
         Show countChars(\"a✓b\").\n",
        "3",
    );
}

#[test]
fn an_empty_text_walks_zero_times() {
    e2e(
        "## To countChars (t: Text) -> Int:\n\
        \x20   Let mutable n be 0.\n\
        \x20   For each c in t:\n\
        \x20       Set n to n + 1.\n\
        \x20   Return n.\n\
         \n\
         ## Main\n\
         Show countChars(\"\").\n",
        "0",
    );
}

#[test]
fn contains_runs_on_the_aot_tier() {
    // ★ Emitted as a call to a free function `contains` that does not exist in the generated crate.
    e2e("## Main\nShow contains(\"abc\", \"b\").\n", "true");
}

#[test]
fn contains_answers_false_and_handles_the_empty_needle() {
    // The false case and the empty needle — a needle everything contains, which a naive
    // implementation returns false for.
    e2e(
        "## Main\n\
         Show contains(\"abc\", \"z\").\n\
         Show contains(\"abc\", \"\").\n\
         Show contains(\"\", \"a\").\n",
        "false\ntrue\nfalse",
    );
}

#[test]
fn the_two_compose_the_way_the_standard_library_uses_them() {
    // ★ EXACTLY `isIntText`'s shape — a character walk asking `contains` about each character.
    // This is the sentence that made `largo build` fail for a project with a route in it.
    e2e(
        "## To digits (t: Text) -> Int:\n\
        \x20   Let mutable n be 0.\n\
        \x20   For each c in t:\n\
        \x20       If contains(\"0123456789\", c):\n\
        \x20           Set n to n + 1.\n\
        \x20   Return n.\n\
         \n\
         ## Main\n\
         Show digits(\"a1b22\").\n",
        "3",
    );
}

#[test]
fn the_standard_library_functions_themselves_run() {
    // ★ THE ONES THAT BROKE THE BUILD, called directly. A lock on the shapes is worth more, but a
    // lock on the actual functions is what says the web surface is usable on this tier.
    e2e(
        "## Main\n\
         Show htmlEscape(\"<a href=\\\"x\\\">&\").\n\
         Show isIntText(\"42\").\n\
         Show isIntText(\"4a\").\n\
         Show isIntText(\"\").\n",
        "&lt;a href=&quot;x&quot;&gt;&amp;\ntrue\nfalse\nfalse",
    );
}
