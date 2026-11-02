//! ★ LOCK — a Text literal survives every path into generated code. ★
//!
//! Emitting a LOGOS Text into a Rust or C source file means spelling it as a literal in THAT
//! language, and a value carrying a quote or a backslash must not be able to end the literal it
//! lives in. Getting this wrong does not produce a wrong answer — it produces a file that will
//! not compile, from a program that is perfectly valid.
//!
//! The escaping existed and was correct. What did not exist was ONE of it: an emitter had its
//! own inline copy, an identical helper sat in the statement emitter, and between them three
//! sites had neither. So a bare literal was fine and the SAME text one `+` away was not:
//!
//! ```text
//! Let s be "a\"b" + n.   →   let s = format!("{}{}", "a"b", 5);
//! ```
//!
//! Found by a derived JSON encoder, whose keys are `"name":` — quotes by construction — but
//! nothing about it is about JSON, so it is pinned here in plain LOGOS. Each test drives a
//! DIFFERENT emission path, because the failure was never "escaping is broken", it was "this
//! path forgot".

use logicaffeine_compile::compile::{compile_to_c, compile_to_rust, tw_outcome};

/// Text that is hostile to every literal syntax there is.
const NASTY: &str = "a\\\"b\\\\c";

/// The tree-walker is the reference: whatever it prints is what the AOT tiers must be able to.
fn reference(src: &str) -> String {
    let out = tw_outcome(src);
    assert_eq!(out.error, None, "the tree-walker must run it:\n{src}");
    out.output
}

/// Emitting must SUCCEED on both AOT backends, and the emitted Rust must contain `escaped` —
/// the literal spelled the way Rust needs it rather than the way LOGOS holds it.
///
/// Checked positively, against the exact expected spelling. A negative heuristic ("no raw quote
/// anywhere") cannot work: generated Rust is FULL of legitimate quotes, so such a test passes for
/// the wrong reason on any program whose own text happens to be harmless.
fn emits_cleanly(src: &str, what: &str, escaped: &[&str]) {
    let rust =
        compile_to_rust(src).unwrap_or_else(|e| panic!("{what}: Rust emission failed: {e:?}"));
    for want in escaped {
        assert!(
            rust.contains(want),
            "{what}: the emitted Rust does not spell the literal as `{want}`, so it carries the \
             raw character and will not compile:\n{rust}"
        );
    }
    compile_to_c(src).unwrap_or_else(|e| panic!("{what}: C emission failed: {e:?}"));
}

#[test]
fn a_bare_text_literal_with_a_quote_survives() {
    let src = format!("## Main\nLet s be \"{NASTY}\".\nShow s.\n");
    assert_eq!(reference(&src).trim(), "a\"b\\c");
    emits_cleanly(&src, "a bare literal", &["a\\\"b\\\\c"]);
}

#[test]
fn a_concatenated_text_literal_with_a_quote_survives() {
    // THE path that was broken: string concatenation folds its operands into a `format!`, and
    // that fold spelled the literal without escaping it.
    let src = format!("## Main\nLet n be 5.\nLet s be \"{NASTY}\" + n.\nShow s.\n");
    assert_eq!(reference(&src).trim(), "a\"b\\c5");
    emits_cleanly(&src, "a concatenation", &["a\\\"b\\\\c"]);
}

#[test]
fn an_appended_text_literal_with_a_quote_survives() {
    // `Set s to s + "…"` lowers to `push_str`, a third path with its own literal spelling — and
    // one of its two arms had the escape while the other did not.
    let src = format!(
        "## Main\nLet mutable s be \"\".\nSet s to s + \"{NASTY}\".\nSet s to s + \"{NASTY}\".\nShow s.\n"
    );
    assert_eq!(reference(&src).trim(), "a\"b\\ca\"b\\c");
    emits_cleanly(&src, "an append", &["a\\\"b\\\\c"]);
}

#[test]
fn a_single_quote_character_appended_survives() {
    // The one-character arm takes `push('c')` rather than `push_str`, and a lone `"` or `\` is
    // exactly the character that breaks a Rust char literal.
    for ch in ["\\\"", "\\\\"] {
        let src = format!("## Main\nLet mutable s be \"x\".\nSet s to s + \"{ch}\".\nShow s.\n");
        assert_eq!(
            reference(&src).trim().len(),
            2,
            "the reference tier appends one character"
        );
        emits_cleanly(&src, "a single-character append", &[]);
    }
}

#[test]
fn a_newline_and_a_tab_survive_every_path() {
    // Not just quotes: an unescaped newline ends a Rust string literal on the spot.
    let src = "## Main\nLet mutable s be \"a\\nb\".\nSet s to s + \"c\\td\".\nShow s.\n";
    emits_cleanly(src, "control characters", &["a\\nb", "c\\td"]);
}
