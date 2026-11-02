use super::*;
use crate::ui::examples::ALL_MATH_EXAMPLES;

/// Words the kernel reads that are English prose in a literate sentence, left plain.
const PROSE: &[&str] = &["a", "an", "A", "An", "the", "of", "such", "that", "there"];

/// Whether the kernel's word `word` is coloured, or named as plain prose.
fn is_known_word(word: &str) -> bool {
    [KEYWORDS, CONNECTIVES, TACTICS, SORTS, PROSE].iter().any(|list| list.contains(&word))
}

fn kinds(source: &str) -> Vec<(String, TokenKind)> {
    tokens(source).into_iter().filter(|t| !t.text.trim().is_empty()).map(|t| (t.text, t.kind)).collect()
}

fn kind_of(source: &str, text: &str) -> TokenKind {
    kinds(source)
        .into_iter()
        .find(|(t, _)| t == text)
        .unwrap_or_else(|| panic!("no token {text:?} in {source:?}: {:?}", kinds(source)))
        .1
}

#[test]
fn the_tokens_spell_every_example_back() {
    for example in ALL_MATH_EXAMPLES {
        let spelled: String = tokens(example.source).into_iter().map(|t| t.text).collect();
        assert_eq!(spelled, example.source, "{}", example.vfs_path);
    }
}

#[test]
fn commands_and_binders_are_keywords() {
    let src = "Inductive Nat := Zero : Nat | Succ : Nat -> Nat.\nDefinition id : forall (A : Type), A -> A := fun A x => x.\nCheck id.\nEval (id Nat Zero).";
    for word in ["Inductive", "Definition", "forall", "fun", "Check", "Eval"] {
        assert_eq!(kind_of(src, word), TokenKind::Keyword, "{word}");
    }
}

#[test]
fn sorts_and_constructors_are_types() {
    let src = "Inductive Nat := Zero : Nat | Succ : Nat -> Nat.\nDefinition p : Prop := q True.";
    for word in ["Nat", "Zero", "Succ", "Prop", "True"] {
        assert_eq!(kind_of(src, word), TokenKind::Type, "{word}");
    }
    assert_eq!(kind_of(src, "q"), TokenKind::Identifier, "a lowercase name in use");
    assert_eq!(kind_of(src, "p"), TokenKind::Builtin, "the name a definition declares");
}

#[test]
fn the_literate_forms_are_keywords() {
    let src = "## To check_parity (n: Nat) -> Decision:\n    Consider n:\n        When Zero: Yield Yes.\n\n## Theorem: FxRefl\n    Statement: (Eq (f x) (f x)).\n    Proof: cc.";
    for word in ["To", "Consider", "When", "Yield", "Theorem", "Statement", "Proof"] {
        assert_eq!(kind_of(src, word), TokenKind::Keyword, "{word}");
    }
    assert_eq!(kind_of(src, "##"), TokenKind::Punctuation);
    assert_eq!(kind_of(src, "check_parity"), TokenKind::Builtin, "the name a header declares");
    assert_eq!(kind_of(src, "FxRefl"), TokenKind::Builtin, "the name a theorem declares");
}

#[test]
fn tactics_are_builtins() {
    let src = "## Theorem: T\n    Statement: (Eq x x).\n    Proof: intro. assumption. auto. ring. lia. omega. simp. induction n.";
    for word in ["intro", "assumption", "auto", "ring", "lia", "omega", "simp", "induction"] {
        assert_eq!(kind_of(src, word), TokenKind::Builtin, "{word}");
    }
}

#[test]
fn comments_run_to_the_end_of_their_line() {
    let src = "-- ∀x. P(x) is a comment\nCheck Zero. -- and so is this";
    assert_eq!(kind_of(src, "-- ∀x. P(x) is a comment"), TokenKind::Comment);
    assert_eq!(kind_of(src, "-- and so is this"), TokenKind::Comment);
    assert_eq!(kind_of(src, "Check"), TokenKind::Keyword);
}

#[test]
fn arrows_and_definitions_are_operators() {
    let src = "Definition f : Nat -> Nat := fun n => n.";
    for op in ["->", ":=", "=>", ":"] {
        assert_eq!(kind_of(src, op), TokenKind::Operator, "{op}");
    }
    assert_eq!(kind_of(src, "."), TokenKind::Punctuation);
}

#[test]
fn symbols_are_operators() {
    let src = "-- plain\nTheorem t : ∀ n, n ≤ n → ¬ (n < n).";
    for symbol in ["∀", "≤", "→", "¬"] {
        assert_eq!(kind_of(src, symbol), TokenKind::Operator, "{symbol}");
    }
}

#[test]
fn numbers_are_numbers() {
    assert_eq!(kind_of("Eval (add 2 30).", "30"), TokenKind::Number);
}

#[test]
fn a_name_that_contains_a_keyword_is_a_name() {
    let src = "Check (funny forallish).";
    assert_eq!(kind_of(src, "funny"), TokenKind::Identifier);
    assert_eq!(kind_of(src, "forallish"), TokenKind::Identifier);
}

#[test]
fn the_ligatures_are_whole_words_and_whole_operators() {
    let at = |text: &str| ligatures(text).into_iter().map(|r| text[r].to_string()).collect::<Vec<_>>();
    assert_eq!(at("Check forall."), ["forall"]);
    assert_eq!(at("(forall (A : Type), A -> A)"), ["forall", "->"]);
    assert_eq!(at("fun x => x"), ["fun", "=>"]);
    assert_eq!(at("exists n, n <= m >= k"), ["exists", "<=", ">="]);
    assert!(at("funny function defun foralls forall_x x_forall exists' fun1").is_empty());
    assert!(at("a --> b <-> c ->> d =>> e <=> f >== g <<= h").is_empty());
    assert!(at("x := y :: z").is_empty());
}

#[test]
fn a_caret_inside_a_ligature_moves_to_its_edge() {
    let text = "Check forall.";
    let start = text.find("forall").unwrap();
    let end = start + "forall".len();
    assert_eq!(snap(text, start + 1, Snap::Nearest), start, "near the start");
    assert_eq!(snap(text, start + 4, Snap::Nearest), end, "near the end");
    assert_eq!(snap(text, start + 3, Snap::Nearest), end, "halfway goes on");
    assert_eq!(snap(text, start + 5, Snap::Start), start);
    assert_eq!(snap(text, start + 1, Snap::End), end);
    for edge in [start, end, 0, text.len()] {
        assert_eq!(snap(text, edge, Snap::Nearest), edge, "an edge stays");
    }
    let arrow = "A -> B";
    assert_eq!(snap(arrow, 3, Snap::End), 4, "inside ->");
}

/// The editor snaps the caret over exactly the spellings the font draws as one symbol: the
/// word and operator lists and the characters that break a ligature are the build script's.
#[test]
fn the_editor_and_the_font_agree_on_the_ligatures() {
    let script = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../scripts/web/build-math-font.py"))
        .expect("scripts/web/build-math-font.py");
    let assigned = |name: &str| {
        let at = script.find(&format!("{name} = \"")).unwrap_or_else(|| panic!("{name} in the build script"));
        let rest = &script[at + name.len() + 4..];
        rest[..rest.find("\"\n").expect("a one-line string")].replace("\\\\", "\\")
    };
    assert_eq!(assigned("WORD_CHARS"), WORD_CHARS);
    assert_eq!(assigned("OPERATOR_CHARS"), OPERATOR_CHARS);
    for word in LIGATURE_WORDS.iter().chain(LIGATURE_OPERATORS) {
        assert!(script.contains(&format!("(\"{word}\", 0x")), "the font draws {word:?}");
    }
    let drawn = script.matches(".liga\")").count();
    assert_eq!(drawn, LIGATURE_WORDS.len() + LIGATURE_OPERATORS.len(), "the font draws no spelling the editor does not know");
}

/// Every word the kernel's parsers read as a keyword — `consume_keyword("…")`,
/// `try_consume_keyword("…")`, `peek_keyword("…")` and the command dispatch's
/// `starts_with("…")` — is coloured here, or named as plain prose.
#[test]
fn every_keyword_the_kernel_reads_is_coloured() {
    let mut sources = Vec::new();
    let mut dirs = vec![std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../crates/logicaffeine_kernel/src/interface")];
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(&dir).expect("the kernel's parsers") {
            let path = entry.unwrap().path();
            if path.is_dir() {
                dirs.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                sources.push(path);
            }
        }
    }
    let mut read = std::collections::BTreeSet::new();
    for path in sources {
        let text = std::fs::read_to_string(path).unwrap();
        for call in ["consume_keyword(\"", "try_consume_keyword(\"", "peek_keyword(\"", "expect_keyword(\"", "input.starts_with(\""] {
            for tail in text.split(call).skip(1) {
                let word = tail.split('"').next().unwrap_or_default().trim();
                if !word.is_empty() && word.chars().all(|c| c.is_alphabetic() || c == '_') {
                    read.insert(word.to_string());
                }
            }
        }
    }
    assert!(read.len() >= 20, "the scan found the kernel's keywords: {read:?}");
    let missing: Vec<_> = read.iter().filter(|w| !is_known_word(w)).collect();
    assert!(missing.is_empty(), "the kernel reads these words but Math mode does not colour them: {missing:?}");
}
