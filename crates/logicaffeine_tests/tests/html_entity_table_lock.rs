//! ★ LOCK — the NAMED character references are the standard's list, whole and in order. ★
//!
//! The tokenizer used to know FIFTEEN names. Everything else fell through to "not an entity" and
//! was emitted as literal text, so `&eacute;` painted the six characters `&eacute;` onto the page
//! where `é` belonged — in ordinary prose, silently, with nothing anywhere reporting it. Fifteen
//! out of 2,125 is not a table, it is the handful somebody needed that afternoon.
//!
//! ★ THE LIST IS NOT OURS TO INVENT, so it is GENERATED (`scripts/dev/gen-html-entities.py`, from
//! `html.entities.html5`, which IS the WHATWG list). This file guards the properties a generated
//! table can still lose — truncation, a bad sort, a duplicate — because `entities::named` binary
//! searches, and a binary search over an unsorted array does not fail loudly. It silently misses.
//!
//! ★ AND 93 OF THEM ARE NOT ONE CHARACTER. `&acE;` is two code points, a base plus a combining
//! mark. A table typed as `char` cannot hold them at all, which is the kind of thing that decides a
//! signature — so the value is text, and `a_multi_code_point_name_survives_whole` is the case that
//! stops anyone narrowing it back.

use logicaffeine_language::lgx::entities::{named, LONGEST_NAME, NAMED};
use logicaffeine_language::lgx::html::decode_entities;

#[test]
fn the_table_is_sorted_and_has_no_duplicates() {
    // ★ `named()` binary searches. An unsorted table does not return a wrong answer loudly — it
    // returns `None` for a name that is right there, which reads exactly like "not an entity" and
    // renders as literal text. This is the invariant the lookup silently depends on.
    for pair in NAMED.windows(2) {
        assert!(
            pair[0].0 < pair[1].0,
            "the table must be strictly sorted by name for the binary search to be correct, but \
             {:?} is not before {:?}",
            pair[0].0,
            pair[1].0
        );
    }
}

#[test]
fn the_table_is_the_whole_standard_list() {
    // A floor, not an equality: the standard may only grow. A REGENERATION that truncated — the
    // realistic corruption for a generated file — lands far below this.
    assert_eq!(
        NAMED.len(),
        2125,
        "the WHATWG list has 2,125 names spelled with a semicolon; regenerate with \
         `python3 scripts/dev/gen-html-entities.py` rather than editing this table by hand"
    );
}

#[test]
fn every_entry_is_a_usable_name_and_a_non_empty_value() {
    for (name, value) in NAMED {
        assert!(!name.is_empty(), "a nameless entry cannot be looked up");
        assert!(
            !value.is_empty(),
            "{name} stands for nothing, which decodes to a vanished character"
        );
        assert!(
            !name.contains('&') && !name.contains(';'),
            "{name:?} carries its own delimiters — the tokenizer strips those before looking up"
        );
        assert!(
            name.len() <= LONGEST_NAME,
            "{name:?} is longer than LONGEST_NAME ({LONGEST_NAME}), so the tokenizer's scan window \
             stops before its closing `;` and it can never be found"
        );
    }
}

#[test]
fn longest_name_is_actually_the_longest() {
    // The scan window is derived from this constant. If it drifted ABOVE the true maximum the
    // tokenizer would merely look further than it needs to; the failure that matters is the other
    // direction, and only an exact check catches both.
    let real = NAMED
        .iter()
        .map(|(n, _)| n.len())
        .max()
        .expect("the table is not empty");
    assert_eq!(
        LONGEST_NAME, real,
        "LONGEST_NAME must equal the longest name in the table"
    );
}

#[test]
fn the_names_that_used_to_be_the_whole_table_still_decode() {
    // The fifteen the tokenizer knew before. A replacement table is worthless if it moves them.
    for (src, want) in [
        ("&amp;", "&"),
        ("&lt;", "<"),
        ("&gt;", ">"),
        ("&quot;", "\""),
        ("&apos;", "'"),
        ("&nbsp;", "\u{a0}"),
        ("&copy;", "©"),
        ("&reg;", "®"),
        ("&hellip;", "…"),
        ("&mdash;", "—"),
        ("&ndash;", "–"),
        ("&laquo;", "«"),
        ("&raquo;", "»"),
        ("&times;", "×"),
        ("&middot;", "·"),
    ] {
        assert_eq!(
            decode_entities(src),
            want,
            "{src} must still decode to {want:?}"
        );
    }
}

#[test]
fn the_names_that_used_to_render_as_their_own_markup_now_decode() {
    // Ordinary prose. Every one of these painted its own source text onto the page before.
    for (src, want) in [
        ("&eacute;", "é"),
        ("&uuml;", "ü"),
        ("&ntilde;", "ñ"),
        ("&ccedil;", "ç"),
        ("&aring;", "å"),
        ("&pound;", "£"),
        ("&euro;", "€"),
        ("&deg;", "°"),
        ("&frac12;", "½"),
        ("&rsquo;", "’"),
        ("&ldquo;", "“"),
        ("&rdquo;", "”"),
        ("&bull;", "•"),
        ("&dagger;", "†"),
        ("&trade;", "™"),
        ("&alpha;", "α"),
        ("&Omega;", "Ω"),
        ("&rarr;", "→"),
        ("&ne;", "≠"),
        ("&le;", "≤"),
        ("&infin;", "∞"),
        ("&notin;", "∉"),
    ] {
        assert_eq!(
            decode_entities(src),
            want,
            "{src} must decode to {want:?}, not render as itself"
        );
    }
}

#[test]
fn a_multi_code_point_name_survives_whole() {
    // ★ 93 names stand for more than one code point. This is why the table's value is text: a
    // `char` cannot hold `&acE;` at all, and narrowing the type would silently drop the combining
    // mark rather than fail to compile.
    let got = decode_entities("&acE;");
    assert_eq!(
        got.chars().count(),
        2,
        "&acE; is a base plus a combining mark, not one character: {got:?}"
    );
    assert_eq!(got, named("acE").expect("acE is in the table"));
}

#[test]
fn case_is_significant_and_a_near_miss_is_not_decoded() {
    // `&Copy;` is not `&copy;` — HTML names are case-sensitive, and several differ only in case
    // (`&Aacute;`/`&aacute;`). A typo stays as typed rather than becoming a character nobody wrote.
    assert_eq!(decode_entities("&Copy;"), "&Copy;");
    assert_eq!(
        decode_entities("&AMP;"),
        "&",
        "but &AMP; IS in the standard's list"
    );
    assert_eq!(decode_entities("&ampp;"), "&ampp;");
    assert_eq!(decode_entities("&notaname;"), "&notaname;");
}

#[test]
fn a_semicolon_less_reference_is_left_alone() {
    // ★ DELIBERATE, and a divergence worth stating. The standard's legacy forms (`&amp` with no
    // `;`) apply only in parser states this tokenizer does not have. Decoding them here would
    // change text a browser leaves alone — and `&ampersand` in prose is the case that would break.
    assert_eq!(decode_entities("&amp"), "&amp");
    assert_eq!(decode_entities("Tom &amp Jerry"), "Tom &amp Jerry");
}

#[test]
fn references_decode_inside_real_prose() {
    assert_eq!(
        decode_entities("caf&eacute; &mdash; 5&nbsp;&euro; &frac12; off"),
        "café — 5\u{a0}€ ½ off"
    );
}
