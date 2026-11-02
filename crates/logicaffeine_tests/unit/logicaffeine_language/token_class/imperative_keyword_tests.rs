use super::*;
use crate::Lexer;
use logicaffeine_base::Interner;

fn kind_of(source: &str, word: &str) -> TokenType {
    let mut interner = Interner::new();
    Lexer::new(source, &mut interner)
        .tokenize()
        .into_iter()
        .find(|t| &source[t.span.start..t.span.end] == word)
        .unwrap_or_else(|| panic!("the lexer read no token {word:?} in {source:?}"))
        .kind
}

#[test]
fn statement_words_are_imperative_keywords() {
    let source = "## Main\nLet x be 1.\nIf x is 1:\n    Show x.\nOtherwise:\n    Give x to y.\nElse:\n    Call f with x.\n";
    for word in ["Let", "Show", "Otherwise", "Give", "Call", "Else"] {
        let kind = kind_of(source, word);
        assert!(is_imperative_keyword(&kind), "{word} ({kind:?}) is not an imperative keyword");
        assert_eq!(classify(&kind), Some(TokenClass::Keyword), "{word} is an imperative keyword the classifier does not call a keyword");
    }
}

#[test]
fn function_words_are_not_imperative_keywords() {
    let source = "## Main\nLet s be a new Set of Int.\nIf the count is 2:\n    Show s.\n";
    for word in ["a", "of", "the", "is"] {
        let kind = kind_of(source, word);
        assert!(!is_imperative_keyword(&kind), "{word} ({kind:?}) is called an imperative keyword");
    }
}

#[test]
fn a_set_type_is_not_the_set_statement() {
    let kind = kind_of("## Main\nLet s be a new Set of Int.\n", "Set");
    assert!(!is_imperative_keyword(&kind), "`Set of Int` names a type, not the Set statement ({kind:?})");
}
