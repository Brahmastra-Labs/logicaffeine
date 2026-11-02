use super::*;

#[test]
fn span_new_stores_positions() {
    let span = Span::new(5, 10);
    assert_eq!(span.start, 5);
    assert_eq!(span.end, 10);
}

#[test]
fn span_default_is_zero() {
    let span = Span::default();
    assert_eq!(span.start, 0);
    assert_eq!(span.end, 0);
}

#[test]
fn token_has_span_field() {
    use logicaffeine_base::Interner;
    let mut interner = Interner::new();
    let lexeme = interner.intern("test");
    let token = Token::new(TokenType::Noun(lexeme), lexeme, Span::new(0, 4));
    assert_eq!(token.span.start, 0);
    assert_eq!(token.span.end, 4);
}

#[test]
fn wh_words_contains_all_wh_tokens() {
    assert_eq!(TokenType::WH_WORDS.len(), 5);
    assert!(TokenType::WH_WORDS.contains(&TokenType::Who));
    assert!(TokenType::WH_WORDS.contains(&TokenType::What));
    assert!(TokenType::WH_WORDS.contains(&TokenType::Where));
    assert!(TokenType::WH_WORDS.contains(&TokenType::When));
    assert!(TokenType::WH_WORDS.contains(&TokenType::Why));
}

#[test]
fn modals_contains_all_modal_tokens() {
    assert_eq!(TokenType::MODALS.len(), 9);
    assert!(TokenType::MODALS.contains(&TokenType::Must));
    assert!(TokenType::MODALS.contains(&TokenType::Shall));
    assert!(TokenType::MODALS.contains(&TokenType::Should));
    assert!(TokenType::MODALS.contains(&TokenType::Can));
    assert!(TokenType::MODALS.contains(&TokenType::May));
    assert!(TokenType::MODALS.contains(&TokenType::Cannot));
    assert!(TokenType::MODALS.contains(&TokenType::Would));
    assert!(TokenType::MODALS.contains(&TokenType::Could));
    assert!(TokenType::MODALS.contains(&TokenType::Might));
}
