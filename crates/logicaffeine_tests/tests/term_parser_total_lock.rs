//! ★ LOCK — the kernel's term parser consumes its WHOLE input, or refuses. ★
//!
//! `TermParser::parse_with_implicits` parsed one term, skipped trailing whitespace, and returned.
//! There was no end-of-input check, no `TrailingInput` variant, and none of its six callers
//! inspected the parser's position afterwards — so `Check Nat -> Nat garbage here` parsed as
//! `Nat -> Nat` and the rest was discarded in silence.
//!
//! ★ WHY IT MATTERS MORE HERE THAN ANYWHERE ELSE. This is the parser for a PROOF KERNEL. Every
//! other component in this repository can afford to be lenient and be merely wrong; a kernel that
//! silently ignores part of what it was asked to check reports success about a claim nobody made.
//! The parser has an `at_end` helper already — it was used as the application loop's stop
//! condition and never as a completeness check, which is the whole bug in one sentence.
//!
//! ★ THIS LOCK CLAIMS NOTHING ABOUT SOUNDNESS. No forged proof was demonstrated, and the callers
//! may well be fed pre-split input in practice. "Accepts a prefix" is simply not a defensible
//! default for this component, and the fix costs one comparison.

use logicaffeine_kernel::interface::TermParser;

#[test]
fn a_whole_term_still_parses() {
    // The guard must not cost anything that already worked.
    for src in [
        "Nat",
        "Nat -> Nat",
        "fun x : Nat => x",
        "forall x : Nat, Nat",
        "(Nat -> Nat)",
    ] {
        assert!(
            TermParser::parse(src).is_ok(),
            "`{src}` is a whole term and must parse"
        );
    }
}

#[test]
fn trailing_whitespace_is_still_a_whole_term() {
    // Whitespace is not input the reader meant to give us — the parser already skipped it, and
    // that behaviour is deliberate rather than incidental.
    for src in ["Nat ", "Nat\t", "Nat\n", "Nat -> Nat   \n\t "] {
        assert!(
            TermParser::parse(src).is_ok(),
            "`{src:?}` is a term plus whitespace"
        );
    }
}

#[test]
fn input_that_cannot_continue_a_term_is_refused() {
    // ★ NOTE WHAT IS *NOT* TRAILING INPUT HERE. The grammar is `app ::= atom+`, so juxtaposition
    // is APPLICATION: `Nat Nat -> Nat garbage here` is one whole term — `(Nat Nat) -> (Nat garbage
    // here)` — not two. My first draft of this test asserted it was refused and was simply wrong
    // about the language. The real case is a token that can never continue a term, which is where
    // the parser used to stop reading and return anyway.
    for src in ["Nat )", "Nat -> Nat , x", "Nat => x", "Nat -> Nat | y"] {
        assert!(
            TermParser::parse(src).is_err(),
            "`{src}` leaves input a term cannot absorb, so it must be refused rather than \
             silently truncated to its prefix"
        );
    }
}

#[test]
fn trailing_garbage_is_refused_and_says_where() {
    let err = TermParser::parse("Nat -> Nat }{ nonsense")
        .expect_err("unparseable trailing input must be refused");
    let said = format!("{err}");
    assert!(
        said.to_lowercase().contains("trailing") || said.to_lowercase().contains("end"),
        "the refusal must say that input was left over, not describe some other problem: {said}"
    );
}

#[test]
fn the_implicit_count_still_comes_back_for_a_whole_term() {
    // `parse_with_implicits` is the entry point the elaborator uses; the guard must not disturb
    // the number it exists to report.
    let (_t, implicits) =
        TermParser::parse_with_implicits("Nat -> Nat").expect("a whole term parses");
    assert_eq!(implicits, 0, "no leading implicit binders in a plain arrow");
}

#[test]
fn one_trailing_full_stop_is_a_terminator_and_not_input() {
    // ★ THE COMPLETENESS GUARD MUST NOT REJECT ORDINARY INPUT. This language ends sentences with
    // periods, and the dot-projection parser declines to read a dot before a non-identifier as a
    // projection — `surface_sugar::a_bare_trailing_dot_is_not_a_projection` pins that. A guard
    // that refused `Zero.` would be a bug introduced by the fix for a bug.
    assert!(
        TermParser::parse("Zero.").is_ok(),
        "a terminating full stop is not trailing input"
    );
    assert!(
        TermParser::parse("Nat -> Nat.").is_ok(),
        "and after an arrow too"
    );
    assert!(
        TermParser::parse("Nat.  ").is_ok(),
        "whitespace may follow the terminator"
    );

    // Exactly one, and nothing after it.
    assert!(
        TermParser::parse("Zero..").is_err(),
        "a second dot is input, not a terminator"
    );
    assert!(
        TermParser::parse("Zero. garbage").is_err(),
        "and nothing may follow it"
    );
}
