//! Lean 4.14 string literals, from original Lean exports, in both kernels.
//!
//! A Lean kernel reads a string literal as a constructor application, and which constructor
//! depends on the Lean release's `String`:
//! * Lean 4.14 (the pinned Stage-A toolchain of the conformance corpus): `structure String where
//!   mk :: data : List Char`, so `"ab"` is `String.mk [Char.ofNat 97, Char.ofNat 98]`;
//! * Lean 4.29+ (the Arena exports): `structure String where ofByteArray :: …`, and the literal is
//!   `String.ofList [Char.ofNat 97, Char.ofNat 98]`.
//!
//! The reader expanded every literal to `String.ofList`, which a Lean 4.14 environment does not
//! contain, and REJECTED valid Lean (`invalid_proof: string literal requires checked String.ofList`)
//! at every string-valued root of the corpus. This was hidden behind the `isValidChar_UInt32`
//! rejection (`B-FALSE-REJECT-2f01b2e5`) until `Nat.ble` was accelerated.
//!
//! Two signals must agree before a literal is read: the header's Lean release fixes the reading
//! (4.14.0: `String.mk`; 4.29.1, 4.30.0, 4.33.0: `String.ofList`), and the export must hold exactly
//! that release's `String` (its one constructor and the packing constant at `List Char → String`).
//! A contradiction between them is a `MalformedExport` decline; a literal under a release with no
//! pinned reading is an `UnsupportedExportVersion` decline. Neither a header nor a crafted
//! environment alone chooses how a literal is read.
//!
//! Fixtures: `fixtures/lean_scale/LG_StringLit.lean` (witness theorems, each `rfl`, so each checks
//! only if the literal means exactly its list of code points) and the original per-case roots
//! `fixtures/lean_scale/original/T00-008.strings` and `T10-005.strings`, all exported with the
//! pinned Lean 4.14.0 and the pinned lean4export (structural expression keys).
#![allow(dead_code)]

#[path = "harvest_support/checker.rs"]
mod checker;
#[path = "lean_scale_support/witness.rs"]
mod witness;

use checker::{check_in, fixture_in, Root};
use logicaffeine_kernel::interface::lean::reference::Reference;
use logicaffeine_kernel::{defeq_for_test, Term, Universe};
use witness::{
    against, both_accept, both_refuse, kernel_spelling, literal_occurrences, map_literal,
    with_last_argument,
};

const STEM: &str = "LG_StringLit";
const ROOTS: [&str; 6] = [
    "LG_SL.emptyIsMk",
    "LG_SL.abIsMk",
    "LG_SL.unicodeIsMk",
    "LG_SL.hello",
    "LG_SL.helloData",
    "LG_SL.helloLength",
];
const THEOREMS: [&str; 5] = [
    "LG_SL.emptyIsMk",
    "LG_SL.abIsMk",
    "LG_SL.unicodeIsMk",
    "LG_SL.helloData",
    "LG_SL.helloLength",
];

fn kind(root: &str) -> &'static str {
    if root == "LG_SL.hello" {
        "def"
    } else {
        "theorem"
    }
}

#[test]
fn lean_4_14_string_literals_mean_string_mk_of_their_code_points_in_both_kernels() {
    let (export, _) = witness::witness(STEM, &ROOTS);
    let reference = Reference::read(&export).expect("the reference translator reads the export");
    let (ctx, report) = witness::imported(&export, &THEOREMS).unwrap_or_else(|e| {
        panic!("the production reader must accept Lean 4.14's string literals: {e}")
    });
    let mut failures = Vec::new();
    for lean in ROOTS {
        if let Err(why) = check_in(
            &export,
            &ctx,
            &report,
            &reference,
            &Root {
                lean,
                kind: kind(lean),
                axioms: &[],
            },
        ) {
            failures.push(format!("{lean}: {why}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn a_literal_is_never_equal_to_a_different_list_of_code_points() {
    let (export, _) = witness::witness(STEM, &ROOTS);
    let reference = Reference::read(&export).expect("the reference translator reads the export");
    let (ctx, _) = witness::imported(&export, &THEOREMS).unwrap_or_else(|e| {
        panic!("the production reader must accept Lean 4.14's string literals: {e}")
    });
    let statement = |lean: &str| {
        reference
            .declaration(&kernel_spelling(lean))
            .expect("reference translation")
            .ty
            .clone()
    };
    let mk_ab = match statement("LG_SL.abIsMk") {
        Term::App(_, rhs) => *rhs,
        other => panic!("an equation: {other}"),
    };
    // Each false statement changes exactly one code point (or the list) on the right-hand side.
    let controls: [(&str, Term); 5] = [
        // `"" = String.mk ['a', 'b']`
        (
            "LG_SL.emptyIsMk",
            with_last_argument(&statement("LG_SL.emptyIsMk"), |_| mk_ab.clone()),
        ),
        // `"ab" = String.mk ['b', 'b']`
        (
            "LG_SL.abIsMk",
            with_last_argument(&statement("LG_SL.abIsMk"), |rhs| {
                assert_eq!(
                    literal_occurrences(rhs, 'a' as u64),
                    1,
                    "the right-hand side spells 'a' once"
                );
                map_literal(rhs, 'a' as u64, 'b' as u64)
            }),
        ),
        // The right-hand side's combining acute accent U+0301 becomes the grave accent U+0300.
        (
            "LG_SL.unicodeIsMk",
            with_last_argument(&statement("LG_SL.unicodeIsMk"), |rhs| {
                assert_eq!(
                    literal_occurrences(rhs, 0x301),
                    1,
                    "the right-hand side spells U+0301 once"
                );
                map_literal(rhs, 0x301, 0x300)
            }),
        ),
        // `hello.data = ['h', 'e', 'l', 'l', 'p']`
        (
            "LG_SL.helloData",
            with_last_argument(&statement("LG_SL.helloData"), |rhs| {
                assert_eq!(
                    literal_occurrences(rhs, 'o' as u64),
                    1,
                    "the right-hand side spells 'o' once"
                );
                map_literal(rhs, 'o' as u64, 'p' as u64)
            }),
        ),
        // `hello.length = 4`
        (
            "LG_SL.helloLength",
            with_last_argument(&statement("LG_SL.helloLength"), |rhs| {
                assert!(
                    literal_occurrences(rhs, 5) >= 1,
                    "the right-hand side is the numeral 5"
                );
                map_literal(rhs, 5, 4)
            }),
        ),
    ];
    for (lean, altered) in controls {
        let name = kernel_spelling(lean);
        let original = statement(lean);
        let (_, proof) = ctx.retained(&name).expect("the proof is retained");
        both_accept(&ctx, &against(proof, &original))
            .unwrap_or_else(|e| panic!("{lean} at its own statement: {e}"));
        assert!(
            !defeq_for_test(&ctx, &original, &altered),
            "{lean}: the altered statement must differ: {altered}"
        );
        both_accept(&ctx, &altered).unwrap_or_else(|e| {
            panic!("{lean}: the altered statement is still a proposition: {e}")
        });
        assert!(
            both_refuse(&ctx, &against(proof, &altered)),
            "{lean}: its proof must be refused by both kernels at the false statement {altered}"
        );
    }
}

/// The corpus's original string-valued roots: each imports, is the reference translation of its Lean
/// type, checks in both kernels, and carries exactly Lean's `#print axioms` (none).
fn original_group(stem: &str, roots: &[&'static str]) {
    let dir = witness::scale_fixtures().join("original");
    let (export, provenance) = fixture_in(&dir, stem);
    let declared: Vec<&str> = provenance["declarations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(declared, roots, "{stem}: the export's roots");
    for root in roots {
        assert_eq!(
            provenance["lean_axioms"][*root],
            serde_json::json!([]),
            "{root}: Lean records no axioms"
        );
        assert_eq!(
            provenance["lean_kind"][*root], "def",
            "{root}: a definition"
        );
    }
    let reference = Reference::read(&export).expect("the reference translator reads the export");
    let (ctx, report) = witness::imported(&export, &[]).unwrap_or_else(|e| {
        panic!("{stem}: the production reader must accept Lean 4.14's string literals: {e}")
    });
    let mut failures = Vec::new();
    let mut control = None;
    for lean in roots {
        match check_in(
            &export,
            &ctx,
            &report,
            &reference,
            &Root {
                lean,
                kind: "def",
                axioms: &[],
            },
        ) {
            Ok(checked) => {
                control.get_or_insert(checked);
            }
            Err(why) => failures.push(format!("{lean}: {why}")),
        }
    }
    assert!(failures.is_empty(), "{stem}: {}", failures.join("\n"));
    let checked = control.expect("a checked root");
    let altered = Term::Pi {
        param: "__corrupted".into(),
        param_type: Box::new(Term::Sort(Universe::Prop)),
        body_type: Box::new(checked.statement.clone()),
    };
    assert!(
        !defeq_for_test(&ctx, &checked.statement, &altered),
        "{stem}: the conversion must discriminate"
    );
    assert!(
        both_refuse(&ctx, &against(&checked.constant, &altered)),
        "{stem}: an altered statement must be refused by both checkers"
    );
}

#[test]
fn the_original_t00_008_string_roots_import_and_check() {
    original_group(
        "T00-008.strings",
        &["T00_008.emptyStr", "T00_008.helloStr", "T00_008.unicodeStr"],
    );
}

#[test]
fn the_original_t10_005_string_roots_import_and_check() {
    original_group(
        "T10-005.strings",
        &[
            "T10_005.empty",
            "T10_005.escaped",
            "T10_005.greeting",
            "T10_005.unicode",
        ],
    );
}

// ---- Both signals must agree: the header's release and the export's own `String`. -----------------

use logicaffeine_kernel::interface::lean::{self as adapter, DiagnosticCode, Verdict};

/// The same export with its header naming `release` (or no release at all).
fn relabelled(export: &str, release: Option<&str>) -> String {
    let mut lines = export.lines();
    let mut header: serde_json::Value =
        serde_json::from_str(lines.next().expect("a header")).expect("header parses");
    let lean = header["meta"]["lean"].as_object_mut().expect("meta.lean");
    match release {
        Some(release) => {
            lean.insert("version".into(), serde_json::json!(release));
        }
        None => {
            lean.remove("version");
        }
    }
    std::iter::once(header.to_string())
        .chain(lines.map(str::to_owned))
        .map(|line| line + "\n")
        .collect()
}

/// The Arena's own Lean 4.29.1 prefix through its first string-literal declaration.
fn arena_4_29_prefix() -> String {
    let input: String = include_str!("fixtures/lean_arena/init-prelude.ndjson")
        .lines()
        .take(19545)
        .map(|line| format!("{line}\n"))
        .collect();
    let header: serde_json::Value = serde_json::from_str(input.lines().next().unwrap()).unwrap();
    assert_eq!(
        header["meta"]["lean"]["version"], "4.29.1",
        "the Arena prefix is a Lean 4.29.1 export"
    );
    assert!(
        input.contains("\"strVal\""),
        "the prefix contains a string literal"
    );
    input
}

fn declined_with(input: &str, code: DiagnosticCode) {
    match adapter::check_str(input) {
        Err(verdict @ Verdict::Decline(_)) => {
            assert_eq!(verdict.reason().code(), code, "{verdict:?}")
        }
        other => panic!("expected a {code:?} decline, got {other:?}"),
    }
}

#[test]
fn each_release_reads_its_own_literal_shape() {
    let (lean_4_14, _) = witness::witness(STEM, &ROOTS);
    adapter::check_str(&lean_4_14).expect("Lean 4.14.0 with the List Char String: String.mk");
    adapter::check_str(&arena_4_29_prefix())
        .expect("Lean 4.29.1 with the UTF-8 String: String.ofList");
}

#[test]
fn a_header_whose_release_contradicts_the_exported_string_is_declined_as_malformed() {
    let (lean_4_14, _) = witness::witness(STEM, &ROOTS);
    // A Lean 4.14 environment (String.mk over List Char) claiming to be Lean 4.29.1.
    declined_with(
        &relabelled(&lean_4_14, Some("4.29.1")),
        DiagnosticCode::MalformedExport,
    );
    // A Lean 4.29.1 environment (String.ofByteArray, String.ofList) claiming to be Lean 4.14.0.
    declined_with(
        &relabelled(&arena_4_29_prefix(), Some("4.14.0")),
        DiagnosticCode::MalformedExport,
    );
}

#[test]
fn a_string_literal_under_an_unpinned_release_is_declined_as_an_unsupported_export_version() {
    let (lean_4_14, _) = witness::witness(STEM, &ROOTS);
    declined_with(
        &relabelled(&lean_4_14, Some("4.99.0")),
        DiagnosticCode::UnsupportedExportVersion,
    );
    declined_with(
        &relabelled(&lean_4_14, None),
        DiagnosticCode::UnsupportedExportVersion,
    );
    declined_with(
        &relabelled(&arena_4_29_prefix(), Some("4.21.0")),
        DiagnosticCode::UnsupportedExportVersion,
    );
}

#[test]
fn the_release_is_consulted_only_for_string_literals() {
    // The comparison witness has no string literal; an unpinned release does not refuse it.
    let comparisons =
        std::fs::read_to_string(witness::scale_fixtures().join("LG_NatCompare.ndjson")).unwrap();
    assert!(
        !comparisons.contains("\"strVal\""),
        "the comparison witness has no string literal"
    );
    adapter::check_str(&relabelled(&comparisons, Some("4.99.0")))
        .expect("no literal, no release question");
}

#[test]
fn a_reference_reading_of_a_different_literal_is_refused_by_the_statement_binding() {
    let (export, _) = witness::witness(STEM, &ROOTS);
    let (ctx, report) = witness::imported(&export, &THEOREMS).unwrap_or_else(|e| {
        panic!("the production reader must accept Lean 4.14's string literals: {e}")
    });
    let reference = Reference::read(&export).expect("the reference translator reads the export");
    for (lean, from, to) in [
        ("LG_SL.abIsMk", r#""strVal":"ab""#, r#""strVal":"ba""#),
        (
            "LG_SL.unicodeIsMk",
            "\"strVal\":\"é𝄞e\u{301}\"",
            "\"strVal\":\"é𝄞e\u{300}\"",
        ),
    ] {
        let root = Root {
            lean,
            kind: "theorem",
            axioms: &[],
        };
        check_in(&export, &ctx, &report, &reference, &root)
            .unwrap_or_else(|e| panic!("{lean}: {e}"));
        // The reference reads a copy of the export whose literal differs in one code point; the
        // production import is unchanged. R8 compares the two statements, so it must refuse.
        assert_eq!(
            export.matches(from).count(),
            1,
            "{lean}: the literal {from} occurs once in the export"
        );
        let other =
            Reference::read(&export.replacen(from, to, 1)).expect("the altered export still reads");
        let Err(why) = check_in(&export, &ctx, &report, &other, &root) else {
            panic!("{lean}: a different literal is not the statement, but the binding accepted it");
        };
        assert!(
            why.contains("not the reference translation"),
            "{lean}: refused for the right reason: {why}"
        );
    }
}
