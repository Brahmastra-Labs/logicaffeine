//! The editor and the compiler judge the same program: what the Studio underlines as a use after
//! a possible move (or a zone escape) is exactly what `compile_to_rust_checked` refuses, with the
//! same finding. Both read the program as its author wrote it — the editor from the parse, the
//! compiler from the parse, before its optimizer folds a single constant away.

use logicaffeine_compile::compile::compile_to_rust_checked;
use logicaffeine_language::error::ParseErrorKind;
use logicaffeine_lsp::document::DocumentState;
use logicaffeine_lsp::lsp_types::NumberOrString;
use logicaffeine_lsp::pipeline::analyze;

const CONSTANT_GIVEN_IN_A_BRANCH: &str = "\
## Main
Let x be 5.
If x > 0:
    Give x to processor.
Show x.
";

/// Every shape, with the finding both sides must agree on (`None`: both accept).
const SHAPES: &[(&str, &str, Option<&str>)] = &[
    ("a constant given in a branch", CONSTANT_GIVEN_IN_A_BRANCH, Some("maybe-moved")),
    (
        "a constant given in the otherwise branch",
        "## Main\nLet x be 5.\nIf x > 9:\n    Show 1.\nOtherwise:\n    Give x to processor.\nShow x.\n",
        Some("maybe-moved"),
    ),
    (
        "a text constant given in a branch",
        "## Main\nLet x be \"hello\".\nLet c be true.\nIf c:\n    Give x to processor.\nShow x.\n",
        Some("maybe-moved"),
    ),
    (
        "a constant given in a while body",
        "## Main\nLet x be 5.\nLet mutable n be 0.\nWhile n < 1:\n    Give x to processor.\n    Set n to n + 1.\nShow x.\n",
        Some("maybe-moved"),
    ),
    (
        "a constant given in a repeat body",
        "## Main\nLet x be 5.\nRepeat for i from 1 to 2:\n    Give x to processor.\nShow x.\n",
        Some("maybe-moved"),
    ),
    (
        "a constant given as a call argument in a branch",
        "## To keep (n: Int):\n    Show n.\n\n## Main\nLet x be 5.\nIf x > 0:\n    Call keep with give x.\nShow x.\n",
        Some("maybe-moved"),
    ),
    (
        "a constant given in a branch of a function",
        "## To f (c: Bool) -> Int:\n    Let x be 5.\n    If c:\n        Give x to processor.\n    Show x.\n    Return 0.\n\n## Main\nShow f(true).\n",
        Some("maybe-moved"),
    ),
    (
        "a value tested by a condition after a possible move",
        "## Main\nLet x be 5.\nLet c be true.\nIf c:\n    Give x to processor.\nIf x > 3:\n    Show 1.\n",
        Some("maybe-moved"),
    ),
    (
        "a sequence given in a constant-true branch",
        "## Main\nLet x be [1, 2].\nLet c be true.\nIf c:\n    Give x to processor.\nShow x.\n",
        Some("maybe-moved"),
    ),
    (
        "a value given in both branches",
        "## Main\nLet x be 5.\nLet c be true.\nIf c:\n    Give x to processor1.\nOtherwise:\n    Give x to processor2.\nShow x.\n",
        Some("use-after-move"),
    ),
    (
        "a value given twice in a row",
        "## Main\nLet x be 5.\nGive x to processor1.\nGive x to processor2.\n",
        Some("use-after-move"),
    ),
    (
        "a value given in one branch and shown in the other",
        "## Main\nLet x be 5.\nLet c be true.\nIf c:\n    Give x to processor.\nOtherwise:\n    Show x.\n",
        None,
    ),
    (
        "a value given in a branch and never used again",
        "## Main\nLet x be 5.\nLet c be true.\nIf c:\n    Give x to processor.\nShow 1.\n",
        None,
    ),
    (
        "a zone escape in a branch that never runs",
        "## Main\nLet outer be 0.\nIf 1 > 2:\n    Inside a new zone called \"Scratch\":\n        Let x be 5.\n        Set outer to x.\nShow outer.\n",
        Some("escape-assignment"),
    ),
    (
        "a zone escape inside a during span",
        "## Main\nLet outer be 0.\nDuring scratching:\n    Inside a new zone called \"Scratch\":\n        Let x be 5.\n        Set outer to x.\nShow outer.\n",
        Some("escape-assignment"),
    ),
    (
        "a constant given in a branch inside a during span",
        "## Main\nLet x be 5.\nDuring handing:\n    If x > 0:\n        Give x to processor.\nShow x.\n",
        Some("maybe-moved"),
    ),
];

/// The editor's first finding on `source`: the parser's refusal of a certain move, else an
/// escape, else an ownership finding — the order the checked compile refuses them in.
fn editor(source: &str) -> Option<String> {
    let analysis = analyze(source);
    if analysis.errors.iter().any(|e| matches!(e.kind, ParseErrorKind::UseAfterMove { .. })) {
        return Some("use-after-move".to_string());
    }
    analysis
        .escape_errors
        .first()
        .or(analysis.ownership_errors.first())
        .map(|finding| finding.code.to_string())
}

/// The checked compile's refusal of `source`, named by the editor's code for the same finding.
fn compiler(source: &str) -> Option<String> {
    let error = compile_to_rust_checked(source).err()?;
    Some(match &error.kind {
        ParseErrorKind::UseAfterMove { .. } => "use-after-move".to_string(),
        ParseErrorKind::Custom(message) if message.contains("might have been given away") => {
            "maybe-moved".to_string()
        }
        ParseErrorKind::Custom(message) if message.contains("after giving it away") => {
            "use-after-move".to_string()
        }
        ParseErrorKind::Custom(message) if message.starts_with("Cannot give") => "double-move".to_string(),
        ParseErrorKind::Custom(message) if message.contains("via assignment to") => {
            "escape-assignment".to_string()
        }
        ParseErrorKind::Custom(message) if message.contains("cannot escape zone") => {
            "escape-return".to_string()
        }
        other => format!("{other:?}"),
    })
}

#[test]
fn the_editor_reports_exactly_what_the_checked_compile_refuses() {
    let disagreements: Vec<String> = SHAPES
        .iter()
        .filter_map(|(name, source, expected)| {
            let expected = expected.map(str::to_string);
            let (editor, compiler) = (editor(source), compiler(source));
            (editor != expected || compiler != expected).then(|| {
                format!("{name}: expected {expected:?}, the editor said {editor:?}, the compiler said {compiler:?}")
            })
        })
        .collect();
    assert!(disagreements.is_empty(), "\n{}", disagreements.join("\n"));
}

/// The Studio links this crate's `DocumentState`; its diagnostics are what the author sees.
#[test]
fn the_studio_underlines_a_possible_move_of_a_constant() {
    let document = DocumentState::new(CONSTANT_GIVEN_IN_A_BRANCH.to_string(), 0);
    let codes: Vec<String> = document
        .diagnostics
        .iter()
        .filter_map(|d| match &d.code {
            Some(NumberOrString::String(code)) => Some(code.clone()),
            _ => None,
        })
        .collect();
    assert!(codes.iter().any(|c| c == "maybe-moved"), "{codes:?}");
}

/// A broken sentence elsewhere sends the editor through per-sentence recovery; the possible move
/// is still reported from the statements that survive.
#[test]
fn the_editor_reports_a_possible_move_after_recovering_from_a_broken_sentence() {
    let source = "## Main\nLet x be 5.\nLet be.\nIf x > 0:\n    Give x to processor.\nShow x.\n";
    let analysis = analyze(source);
    assert!(!analysis.errors.is_empty(), "the broken sentence is reported");
    let codes: Vec<&str> = analysis.ownership_errors.iter().map(|e| e.code).collect();
    assert!(codes.contains(&"maybe-moved"), "{codes:?}");
}
