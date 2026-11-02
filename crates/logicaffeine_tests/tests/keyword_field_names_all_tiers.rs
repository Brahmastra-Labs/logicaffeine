//! A record field may be named with any word, a Rust keyword included, on every tier.
//!
//! `type`, `match`, `fn`, `self`… are ordinary English field names (a dropped file has a `type`), and the
//! tree-walker has always accepted them. The AOT-Rust tier wrote field names verbatim, so every one of
//! them failed rustc wherever the field was declared, built, read, set, shown or matched; the C tier has
//! keywords of its own. Every Rust keyword — strict, reserved and weak — is a field name here, in one
//! record and in a variant, and each program must print the same on the tree-walker, the VM, AOT-Rust
//! and C.

mod common;
use common::{assert_all_tiers, assert_c_output, assert_compiled_equals_interpreted};

/// Every Rust keyword: the strict and reserved sets of the 2021 edition, the weak keywords, and `gen`.
const KEYWORDS: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern",
    "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub",
    "ref", "return", "self", "Self", "static", "struct", "super", "trait", "true", "type",
    "unsafe", "use", "where", "while", "abstract", "become", "box", "do", "final", "macro",
    "override", "priv", "try", "typeof", "unsized", "virtual", "yield", "union", "gen",
];

fn on_every_tier(src: &str, expected: &str) {
    assert_all_tiers(src, expected);
    assert_compiled_equals_interpreted(src);
    assert_c_output(src, expected);
}

#[test]
fn a_record_whose_fields_are_every_rust_keyword_runs_on_every_tier() {
    let mut src = String::from("## A Thing has:\n");
    for k in KEYWORDS {
        src.push_str(&format!("    A {k}: Text.\n"));
    }
    let inits: Vec<String> = KEYWORDS.iter().map(|k| format!("{k} \"v_{k}\"")).collect();
    src.push_str(&format!(
        "\n## Main\nLet t be a new Thing with {}.\n",
        inits.join(" and ")
    ));
    let mut expected: Vec<String> = Vec::new();
    for k in KEYWORDS {
        src.push_str(&format!("Show t's {k}.\n"));
        expected.push(format!("v_{k}"));
    }
    src.push_str("Show t.\n");
    let mut sorted: Vec<&str> = KEYWORDS.to_vec();
    sorted.sort_unstable();
    let fields: Vec<String> = sorted.iter().map(|k| format!("{k}: v_{k}")).collect();
    expected.push(format!("Thing {{ {} }}", fields.join(", ")));
    on_every_tier(&src, &expected.join("\n"));
}

#[test]
fn a_keyword_field_is_set_and_read_back_through_a_function() {
    on_every_tier(
        "## A Document has:\n    A name: Text.\n    A type: Text.\n    A size: Int.\n\n## To describe (f: Document) -> Text:\n    Return f's name + \" is \" + f's type.\n\n## Main\nLet mutable f be a new Document with name \"a.png\" and type \"image\" and size 3.\nSet f's type to \"image/png\".\nShow describe(f).\nShow f.\n",
        "a.png is image/png\nDocument { name: a.png, size: 3, type: image/png }",
    );
}

#[test]
fn a_variant_whose_fields_are_rust_keywords_is_built_shown_and_matched() {
    on_every_tier(
        "## A Token is either:\n    A Word with type Text and match Int.\n    A Gap.\n\n## Main\nLet w be a new Word with type \"noun\" and match 3.\nShow w.\nInspect w:\n    When Word (t, m): Show \"{t} {m}\".\n    When Gap: Show \"gap\".\n",
        "Word(noun, 3)\nnoun 3",
    );
}
