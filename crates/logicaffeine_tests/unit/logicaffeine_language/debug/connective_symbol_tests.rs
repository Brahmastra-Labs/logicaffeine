use super::connective_symbol;
use crate::formatter::{LogicFormatter, UnicodeFormatter};
use crate::token::TokenType;

/// The parser builds implications from `If`, `Then` and `Implies`; the tree and the formatter
/// both draw each of them as the implication.
#[test]
fn every_implication_token_draws_as_an_arrow() {
    for op in [TokenType::If, TokenType::Then, TokenType::Implies] {
        assert_eq!(connective_symbol(&op), "→", "{op:?}");
        assert_eq!(UnicodeFormatter.binary_op(&op, "P", "Q"), "(P → Q)", "{op:?}");
    }
}

#[test]
fn the_tree_and_the_formatter_agree_on_every_binary_connective() {
    for op in [TokenType::And, TokenType::Or, TokenType::If, TokenType::Then, TokenType::Implies, TokenType::Iff] {
        let drawn = UnicodeFormatter.binary_op(&op, "P", "Q");
        assert_eq!(drawn, format!("(P {} Q)", connective_symbol(&op)), "{op:?}");
    }
}
