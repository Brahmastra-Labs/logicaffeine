//! ★ LOCK — non-ASCII text never panics the LGX front end. ★
//!
//! Two scanners walked their input BYTE BY BYTE and then sliced the `&str` at the byte they
//! had reached. A UTF-8 continuation byte (0x80..=0xBF) is not a char boundary, so the slice
//! panicked — `byte index N is not a char boundary`. A panic in the parser is not a Socratic
//! error and not a refusal: it takes down `largo check`, `largo build`, the LSP and the dev
//! server, with a Rust backtrace where a message about the author's template should be.
//!
//! ★ WHY THE FUZZER COULD NOT SEE IT. `lgx_fuzz.rs`'s alphabet is 33 characters and every one
//! of them is ASCII, and the three hostile corpora are ASCII too apart from `\u{0}` and
//! `\u{FFFD}` — and `\u{FFFD}` only ever appears in TEXT position, which is boundary-safe
//! because text is copied wholesale rather than scanned byte-wise. So 70,000 generated cases
//! could not reach either site. The lesson is the corpus, not the code: a tokenizer for a
//! language people write prose in must be fuzzed with the bytes prose is made of.
//!
//! Every case here is ordinary content a person would actually write — a French loop variable,
//! a German class name, a Japanese attribute — not a hostile string.

use logicaffeine_language::lgx::css;
use logicaffeine_language::lgx::html::tokenize;
use logicaffeine_language::lgx::template::parse;

/// `{for each …:}` and `{let …}` scan their prelude for ` in ` / ` be ` byte by byte.
/// A non-ASCII BINDING NAME puts a continuation byte before the needle is found.
#[test]
fn a_non_ascii_loop_variable_does_not_panic() {
    for src in [
        "{for each café in cafés:}<li>{café}</li>{end}",
        "{for each élément in liste:}<li>x</li>{end}",
        "{for each 日本 in items:}<li>x</li>{end}",
        "{for each x in café:}<li>x</li>{end}",
    ] {
        let _ = parse(src);
    }
}

#[test]
fn a_non_ascii_let_binding_does_not_panic() {
    for src in [
        "{let café be 1}<p>x</p>",
        "{let x be café}<p>x</p>",
        "{let café}<p>x</p>",
    ] {
        let _ = parse(src);
    }
}

/// An UNQUOTED attribute value is scanned byte by byte to find its end.
#[test]
fn a_non_ascii_unquoted_attribute_value_does_not_panic() {
    for src in [
        "<p class=café>x</p>",
        "<p data-x=日本>x</p>",
        "<p class=über>x</p>",
    ] {
        let _ = tokenize(src);
        let _ = parse(src);
    }
}

/// The whole front end, over ordinary non-ASCII prose in every position that is scanned
/// rather than copied.
#[test]
fn non_ascii_content_never_panics_anywhere_in_the_front_end() {
    for src in [
        "<p>Café — 日本 — Ω</p>",
        "<p title=\"café\">x</p>",
        "<p title=café>x</p>",
        "{if café:}<p>x</p>{end}",
        "{café}",
        "<Café>x</Café>",
        "{for each x in xs, keyed by café:}<li>x</li>{end}",
        "{for each x in xs, at index café:}<li>x</li>{end}",
    ] {
        let _ = tokenize(src);
        let _ = parse(src);
    }
}

/// The CSS scoper splits selectors byte by byte too.
#[test]
fn a_non_ascii_selector_does_not_panic() {
    for sheet in [
        ".café { color: red }",
        "[title=\"日本\"] { color: red }",
        ".über > .a { color: red }",
    ] {
        let _ = css::scope(sheet, "lgx-t");
    }
}
