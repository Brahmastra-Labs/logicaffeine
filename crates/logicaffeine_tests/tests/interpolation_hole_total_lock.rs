//! ★ LOCK — an interpolation hole is read WHOLE, or refused by name. ★
//!
//! A `"…{expr}…"` hole was parsed with a sub-parser and the sub-parser's LEFTOVER TOKENS WERE
//! DISCARDED, unexamined. So a hole whose contents were not entirely an expression silently
//! rendered the prefix that happened to parse and threw the rest away:
//!
//! ```text
//! Show "x{5 garbage here}y".   →  x5y
//! Show "{1 2 3}".              →  1
//! Show "{\"a\":1}".            →  a
//! ```
//!
//! ★ THE THIRD LINE IS WHY THIS MATTERS FOR A WEB FRAMEWORK. `{"a":1}` is a JSON object — the most
//! ordinary thing an author of a route, a fixture or a fetch body ever types — and it printed `a`.
//! No error, no warning, on every tier. It was found from the other end: `route_request_response_lock`
//! wrote `json("{}")` in a fixture and the failure it produced named an EMPTY hole, which sent me
//! looking at what a NON-empty unparseable hole does.
//!
//! A hole is written `{…}` and a literal brace is doubled — `{{` and `}}` — exactly as Rust's
//! `format!` does it, and that escape has always worked. What was missing is the other half of the
//! contract: if it is not a hole it must SAY SO. Silently rendering half of one is the worst of the
//! three options, because the page looks like it worked.
//!
//! ★ AND THE REFUSAL TEACHES THE ESCAPE. Someone who types `json("{}")` or a CSS rule in a string
//! is not looking for a lecture on holes; they want the two characters that make it literal. Every
//! message here names them.

use logicaffeine_compile::compile::tw_outcome;

/// The tree-walker's refusal for `src`, or a panic naming what it printed instead.
fn refusal(subject: &str, src: &str) -> String {
    let out = tw_outcome(src);
    match out.error {
        Some(e) => e,
        None => panic!(
            "`{subject}` must be refused, and instead printed {:?} — a hole that is not read whole \
             renders the piece that parsed and silently drops the rest:\n{src}",
            out.output.trim()
        ),
    }
}

/// What `src` prints on the tree-walker, with a refusal reported as a failure.
fn prints(subject: &str, src: &str) -> String {
    let out = tw_outcome(src);
    assert_eq!(out.error, None, "`{subject}` must run:\n{src}");
    out.output.trim().to_string()
}

#[test]
fn a_hole_with_text_after_its_expression_is_refused() {
    let e = refusal(
        "trailing text in a hole",
        "## Main\nShow \"x{5 garbage here}y\".\n",
    );
    assert!(
        e.contains("garbage here"),
        "the refusal must quote the text it could not read, so the author can see WHICH part of \
         their hole was the problem — got {e:?}"
    );
}

#[test]
fn a_json_object_written_in_a_string_is_refused_rather_than_truncated() {
    // ★ THE HEADLINE CASE. This printed `a` before.
    let e = refusal(
        "a JSON object in a string",
        "## Main\nShow \"{\\\"a\\\":1}\".\n",
    );
    assert!(
        e.contains("{{"),
        "someone typing JSON in a string wants the escape, not a lecture — the refusal must name \
         `{{{{` and `}}}}` — got {e:?}"
    );
}

#[test]
fn several_expressions_in_one_hole_are_refused() {
    refusal(
        "three expressions in one hole",
        "## Main\nShow \"{1 2 3}\".\n",
    );
    refusal(
        "a repeated name in one hole",
        "## Main\nLet n be 7.\nShow \"{n n n}\".\n",
    );
}

#[test]
fn the_empty_hole_refusal_also_teaches_the_escape() {
    // `json("{}")` is how this was found, and `{}` on its own was ALREADY refused — but with a
    // message that named the problem and not the fix.
    let e = refusal("an empty hole", "## Main\nShow \"{}\".\n");
    assert!(
        e.contains("{{"),
        "the empty-hole refusal must name the escape too — got {e:?}"
    );
}

#[test]
fn the_escape_still_writes_a_literal_brace() {
    // ★ THE OTHER HALF OF THE CONTRACT, and the reason the refusals above are the right answer
    // rather than "treat unparseable braces as literal": there is already a way to say it.
    assert_eq!(
        prints("an escaped empty object", "## Main\nShow \"{{}}\".\n"),
        "{}"
    );
    assert_eq!(
        prints(
            "an escaped JSON object",
            "## Main\nShow \"{{\\\"a\\\":1}}\".\n"
        ),
        "{\"a\":1}"
    );
    assert_eq!(
        prints(
            "a CSS rule in a string",
            "## Main\nShow \"p {{ color: red; }}\".\n"
        ),
        "p { color: red; }"
    );
}

#[test]
fn ordinary_holes_are_untouched() {
    // The floor: everything that legitimately parses as one expression keeps working exactly as
    // it did. A lock that made holes strict and broke interpolation would be no bargain.
    assert_eq!(
        prints("a name", "## Main\nLet n be 7.\nShow \"{n}\".\n"),
        "7"
    );
    assert_eq!(prints("arithmetic", "## Main\nShow \"{1 + 2}\".\n"), "3");
    assert_eq!(
        prints("a call", "## Main\nShow \"{lower(\\\"AB\\\")}\".\n"),
        "ab"
    );
    assert_eq!(
        prints(
            "two holes and literals",
            "## Main\nLet n be 2.\nShow \"a{n}b{n + 1}c\".\n"
        ),
        "a2b3c"
    );
    assert_eq!(
        prints("a field read", "## A P has:\n    an x, which is Int.\n\n## Main\nLet p be a new P with x 4.\nShow \"{p's x}\".\n"),
        "4"
    );
}

#[test]
fn an_unclosed_brace_is_still_refused() {
    refusal("an unclosed hole", "## Main\nShow \"a{b\".\n");
}

// ─────────────────────────────────────────────────────────────────────────────
// ★ A HOLE SPEAKS THE LANGUAGE IT IS WRITTEN IN.
//
// The fragment between the braces was lexed by a FRESH `Lexer::new`, and
// `Lexer::new` is DECLARATIVE mode — while every string that can contain a hole
// lives inside imperative code. So the entire imperative vocabulary was invisible
// to a hole: `length`, `copy`, `sum`, `count` and every other word the imperative
// lexer promotes to a keyword lexed as a bare identifier instead.
//
// The visible symptom was `Undefined variable: length` — an error naming a
// variable the author never wrote, for a phrase that is correct LOGOS three
// characters to the left of the quote. `Show length of tasks.` worked;
// `Show "{length of tasks}".` did not, and could not say why.
//
// The hole is a fragment of the surrounding program. It is read with the
// surrounding program's vocabulary, which is what `self.mode` already knows.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn a_hole_speaks_the_imperative_vocabulary() {
    assert_eq!(
        prints(
            "length of",
            "## Main\nLet tasks be [\"write\", \"test\"].\nShow \"{length of tasks}\".\n"
        ),
        "2"
    );
    assert_eq!(
        prints(
            "length of, in a sentence",
            "## Main\nLet tasks be [\"write\", \"test\"].\nShow \"You have {length of tasks} tasks.\".\n"
        ),
        "You have 2 tasks."
    );
    assert_eq!(
        prints(
            "length of text",
            "## Main\nLet s be \"hello\".\nShow \"{length of s}\".\n"
        ),
        "5"
    );
}

#[test]
fn an_of_phrase_composes_inside_a_hole() {
    // Not a special case for `length`: the hole runs the ordinary expression
    // parser, so an of-phrase takes part in arithmetic and comparison like any
    // other operand.
    assert_eq!(
        prints(
            "arithmetic on it",
            "## Main\nLet t be [1, 2, 3].\nShow \"{length of t + 1}\".\n"
        ),
        "4"
    );
    assert_eq!(
        prints(
            "two of-phrases",
            "## Main\nLet a be [1, 2].\nLet b be [1].\nShow \"{length of a + length of b}\".\n"
        ),
        "3"
    );
}

#[test]
fn the_hole_and_the_statement_agree() {
    // ★ THE INVARIANT, not the examples. Whatever `Show <expr>.` prints, the hole
    // `Show "{<expr>}".` prints too — that is what "a hole holds one value" means,
    // and it is the property the mode mismatch broke. If a future keyword is
    // promoted in imperative mode and the hole is not told, this goes red.
    for expr in [
        "length of xs",
        "length of xs + 1",
        "item 1 of xs",
        "xs",
        "1 + 2",
    ] {
        let prelude = "## Main\nLet xs be [10, 20, 30].\n";
        let bare = prints(expr, &format!("{prelude}Show {expr}.\n"));
        let hole = prints(expr, &format!("{prelude}Show \"{{{expr}}}\".\n"));
        assert_eq!(
            bare, hole,
            "`Show {expr}.` printed {bare:?} but `Show \"{{{expr}}}\".` printed {hole:?} — a hole \
             must hold exactly the value the same expression has outside one"
        );
    }
}
