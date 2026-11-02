//! ★ LOCK — a NUMERIC character reference decodes the way HTML says it does. ★
//!
//! `&#128;` is `€`. Not U+0080. Every browser on earth agrees, because the HTML standard's
//! numeric-character-reference step does not hand the number straight to a code point: the range
//! `0x80..=0x9F` is a fixed 32-entry REPLACEMENT TABLE, a legacy of documents that declared
//! windows-1252 and wrote the raw byte. Skipping that table is not an exotic edge — `&#128;`,
//! `&#147;`/`&#148;` (curly quotes) and `&#151;` (an em dash) are what a word processor pastes,
//! so they arrive in real page copy constantly.
//!
//! ★ AND IT FAILED SILENTLY, which is the part that matters. `char::from_u32(128)` succeeds and
//! yields U+0080, an invisible C1 control. The page did not refuse, did not warn, and rendered a
//! character nobody typed and nobody can see — the exact failure mode this campaign keeps finding,
//! where a check that cannot fail sits in front of a wrong answer.
//!
//! ★ THE ERROR CASES WERE WRONG IN THE OTHER DIRECTION. `&#0;`, a surrogate like `&#xD800;`, and
//! anything past `&#x10FFFF;` must all become U+FFFD, the replacement character. Instead
//! `char::from_u32` answered `None` and the decoder fell through to "not an entity", emitting the
//! literal text `&#xD800;` into the document. A reference that cannot be represented is not a
//! reference that was never written.
//!
//! The tokenizer is where this has to be right: it runs over static markup in a `View:`, and its
//! output is the text a page paints on every tier.

use logicaffeine_language::lgx::html::decode_entities;

/// The whole windows-1252 replacement range, spelled out. A table is exactly the kind of thing that
/// is right in the two entries someone checked and wrong in the other thirty.
const C1_REPLACEMENTS: &[(u32, char)] = &[
    (0x80, '\u{20AC}'), // €
    (0x82, '\u{201A}'),
    (0x83, '\u{0192}'),
    (0x84, '\u{201E}'),
    (0x85, '\u{2026}'), // …
    (0x86, '\u{2020}'),
    (0x87, '\u{2021}'),
    (0x88, '\u{02C6}'),
    (0x89, '\u{2030}'),
    (0x8A, '\u{0160}'),
    (0x8B, '\u{2039}'),
    (0x8C, '\u{0152}'),
    (0x8E, '\u{017D}'),
    (0x91, '\u{2018}'), // ‘
    (0x92, '\u{2019}'), // ’
    (0x93, '\u{201C}'), // “
    (0x94, '\u{201D}'), // ”
    (0x95, '\u{2022}'), // •
    (0x96, '\u{2013}'), // –
    (0x97, '\u{2014}'), // —
    (0x98, '\u{02DC}'),
    (0x99, '\u{2122}'), // ™
    (0x9A, '\u{0161}'),
    (0x9B, '\u{203A}'),
    (0x9C, '\u{0153}'),
    (0x9E, '\u{017E}'),
    (0x9F, '\u{0178}'),
];

#[test]
fn the_windows_1252_range_uses_the_replacement_table() {
    for (code, expected) in C1_REPLACEMENTS {
        let decimal = format!("&#{code};");
        assert_eq!(
            decode_entities(&decimal),
            expected.to_string(),
            "&#{code}; must decode to U+{:04X}, not to the raw code point U+{code:04X}",
            *expected as u32
        );
        let hex = format!("&#x{code:X};");
        assert_eq!(
            decode_entities(&hex),
            expected.to_string(),
            "&#x{code:X}; must decode the same way its decimal spelling does"
        );
    }
}

#[test]
fn the_three_unassigned_slots_in_that_range_become_the_replacement_character() {
    // 0x81, 0x8D, 0x8F, 0x90 and 0x9D have no windows-1252 character. HTML maps them to U+FFFD
    // rather than to the C1 control, so no branch of this range ever yields an invisible control.
    for code in [0x81u32, 0x8D, 0x8F, 0x90, 0x9D] {
        assert_eq!(
            decode_entities(&format!("&#{code};")),
            "\u{FFFD}",
            "&#{code}; is unassigned in the replacement table and must become U+FFFD"
        );
    }
}

#[test]
fn a_reference_that_cannot_be_a_character_becomes_the_replacement_character() {
    // ★ NOT literal text. The author wrote a reference; it is malformed, and the standard says what
    // a malformed one renders as. Passing `&#xD800;` through verbatim shows the reader the markup.
    // The last one OVERFLOWS a u32 outright, which is a different failure from "not digits" and
    // must not be confused with it: it denotes nothing, so it is U+FFFD, while `&#xZZ;` was never
    // a reference and stays as typed (the case below).
    for src in [
        "&#0;",
        "&#xD800;",
        "&#xDFFF;",
        "&#x110000;",
        "&#99999999;",
        "&#99999999999999999999;",
    ] {
        assert_eq!(
            decode_entities(src),
            "\u{FFFD}",
            "{src} is not a representable character and must become U+FFFD, not survive as markup"
        );
    }
}

#[test]
fn ordinary_references_are_untouched_by_any_of_this() {
    // The controls. A fix to the table is worthless if it moves the cases that already worked.
    for (src, want) in [
        ("&#169;", "©"),
        ("&#x41;", "A"),
        ("&#65;", "A"),
        ("&#x1F600;", "\u{1F600}"),
        ("&amp;", "&"),
        ("&lt;", "<"),
        ("&#10;", "\n"),
        ("&#9;", "\t"),
    ] {
        assert_eq!(
            decode_entities(src),
            want,
            "{src} must still decode to {want:?}"
        );
    }
}

#[test]
fn something_that_is_not_a_reference_at_all_is_still_left_alone() {
    // `&` is legal text. A decoder that got eager here would corrupt every page mentioning
    // "Tom & Jerry" or a query string with `?a=1&b=2`.
    for src in ["&", "a & b", "&#;", "&#xZZ;", "?a=1&b=2", "&nope"] {
        assert_eq!(
            decode_entities(src),
            src,
            "{src:?} holds no complete reference and must survive verbatim"
        );
    }
}

#[test]
fn a_reference_in_the_middle_of_real_text_decodes_in_place() {
    assert_eq!(
        decode_entities("price: &#128;5 &#151; a bargain"),
        "price: €5 — a bargain"
    );
}
