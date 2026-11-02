//! Code highlighting agrees with the LOGOS lexer: every word the lexer reads as a statement
//! word (`Let`, `Otherwise`, `Give`, …) or a taught keyword (`If`) is coloured as one, at
//! whatever case it is written — across every shipped Code example — and English function
//! words (`a`, `of`, `is`) are not.
use super::{tokenize, Language, TokenKind};

fn kind_at(source: &str, start: usize) -> Option<TokenKind> {
    let mut at = 0;
    for t in tokenize(source, Language::Logos) {
        if at == start {
            return Some(t.kind);
        }
        at += t.text.len();
    }
    None
}

fn comment_covers(source: &str, offset: usize) -> bool {
    let mut at = 0;
    for t in tokenize(source, Language::Logos) {
        if (at..at + t.text.len()).contains(&offset) {
            return t.kind == TokenKind::Comment;
        }
        at += t.text.len();
    }
    false
}

#[test]
fn statement_keywords_colour_at_any_case() {
    let source = "## Main\nLet x be 1.\nShow x.\nRepeat for i from 1 to 3:\n    Set x to i.\nIf x is greater than 1:\n    Show x.\n";
    for word in ["Let", "Show", "Repeat", "Set", "If"] {
        let at = source.find(word).unwrap();
        assert_eq!(kind_at(source, at), Some(TokenKind::Keyword), "{word} is not coloured as a keyword");
    }
}

/// Every statement word of the language, written as a program writes it — not only the ones
/// with a lesson. `Otherwise` read as plain text beside a coloured `If`.
#[test]
fn every_statement_word_colours_as_a_keyword() {
    let source = "## Main\nLet mutable total be 0.\nRepeat for i from 1 to 3:\n    Set total to total + i.\nIf total is greater than 5:\n    Show \"big\".\nOtherwise:\n    Show \"small\".\nPush 4 to items.\nGive items to keeper.\nCall report with total.\nInspect shape:\n    When Circle (r):\n        Return r.\nWhile total is greater than 0:\n    Break.\n";
    for word in ["Let", "Repeat", "Set", "If", "Show", "Otherwise", "Push", "Give", "Call", "Inspect", "Return", "While", "Break"] {
        let at = source.find(word).unwrap_or_else(|| panic!("{word} is not in the probe"));
        assert_eq!(kind_at(source, at), Some(TokenKind::Keyword), "{word} is not coloured as a keyword");
    }
}

#[test]
fn else_and_otherwise_colour_at_any_case() {
    for branch in ["Else", "otherwise", "OTHERWISE", "Otherwise"] {
        let source = format!("## Main\nIf x is 1:\n    Show 1.\n{branch}:\n    Show 2.\n");
        let at = source.find(&format!("\n{branch}:")).unwrap() + 1;
        assert_eq!(kind_at(&source, at), Some(TokenKind::Keyword), "{branch} is not coloured as a keyword");
    }
}

/// English function words stay the text colour: a statement reads as a sentence with its
/// keywords marked, not as a sentence painted purple.
#[test]
fn function_words_are_not_keywords() {
    let source = "## Main\nLet s be a new Set of Int.\nIf the count is greater than 1:\n    Show s.\n";
    for word in [" a ", " of ", " the ", " is "] {
        let at = source.find(word).unwrap() + 1;
        assert_ne!(kind_at(source, at), Some(TokenKind::Keyword), "{:?} is coloured as a keyword", word.trim());
    }
}

#[test]
fn a_set_of_values_is_a_type_and_setting_a_value_is_a_keyword() {
    let source = "## Main\nLet s be a new Set of Int.\nSet x to 1.\n";
    let the_type = source.find("Set of").unwrap();
    let the_verb = source.rfind("Set x").unwrap();
    assert_eq!(kind_at(source, the_type), Some(TokenKind::Type));
    assert_eq!(kind_at(source, the_verb), Some(TokenKind::Keyword));
}

#[test]
fn every_keyword_in_the_code_examples_is_coloured() {
    use logicaffeine_base::Interner;
    use logicaffeine_language::{teach, token_class, Lexer};
    let mut missing = std::collections::BTreeSet::new();
    for spec in crate::ui::examples::ALL_CODE_EXAMPLES {
        let mut interner = Interner::new();
        let tokens = Lexer::new(spec.source, &mut interner).tokenize();
        for (at, token) in tokens.iter().enumerate() {
            if !token_class::is_imperative_keyword(&token.kind) && teach::doc_for(&token.kind).is_none() {
                continue;
            }
            let word = &spec.source[token.span.start..token.span.end];
            if !word.chars().all(|c| c.is_alphabetic()) {
                continue;
            }
            // Words in a comment are prose, not code: the comment's colour is theirs.
            if comment_covers(spec.source, token.span.start) {
                continue;
            }
            // A word inside a comparison written in English (`at least`, `equals`) is coloured as
            // the operator the phrase is; every other keyword is coloured as a keyword.
            let want = if teach::doc_for_phrase(spec.source, &tokens, at).is_some() { TokenKind::Operator } else { TokenKind::Keyword };
            if kind_at(spec.source, token.span.start) != Some(want) {
                missing.insert(format!("{word} ({})", spec.vfs_path));
            }
        }
    }
    let words: Vec<String> = missing.into_iter().collect();
    assert!(words.is_empty(), "{} keyword use(s) the highlighter leaves uncoloured:\n{}", words.len(), words.join("\n"));
}

/// A comparison written in English colours as the operator it is: every word of `is less than`,
/// `at least` and `is equal to` reads like `<`, `>=` and `==` (the user, 2026-11-01).
#[test]
fn english_comparisons_colour_as_operators() {
    let source = "## Main\nLet n be 1.\nIf n is less than 2:\n    Show n.\nIf n is at least 1:\n    Show n.\nIf n is equal to 1:\n    Show n.\nIf n is greater than or equal to 1:\n    Show n.\n";
    for phrase in ["less than", "at least", "is equal to", "greater than or equal to"] {
        let at = source.find(phrase).unwrap();
        let mut offset = 0;
        for word in phrase.split(' ') {
            let start = at + source[at..].find(word).unwrap().max(offset);
            assert_eq!(kind_at(source, start), Some(TokenKind::Operator), "{word:?} of {phrase:?} is not coloured as an operator");
            offset = start - at + word.len();
        }
    }
    // The `is` of a plain predication is not an operator, and `n` never is.
    let plain = "## Main\nLet n be 1.\nIf n is 1:\n    Show n.\n";
    let at = plain.find("is 1").unwrap();
    assert_ne!(kind_at(plain, at), Some(TokenKind::Operator), "a bare `is` is coloured as an operator");
    assert_eq!(kind_at(plain, plain.find("n is").unwrap()), Some(TokenKind::Identifier));
}
