use logos::*;

#[test]
fn equals_is_tokenized() {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new("x equals 5", &mut interner);
    let tokens = lexer.tokenize();

    assert!(
        tokens.iter().any(|t| matches!(t.kind, TokenType::Equals)),
        "Should have Equals token: {:?}",
        tokens
    );
}

#[test]
fn equals_distinct_from_is() {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new("x is mortal. y equals 5.", &mut interner);
    let tokens = lexer.tokenize();

    let is_count = tokens.iter().filter(|t| matches!(t.kind, TokenType::Is)).count();
    let equals_count = tokens.iter().filter(|t| matches!(t.kind, TokenType::Equals)).count();

    assert_eq!(is_count, 1, "Should have 1 Is token");
    assert_eq!(equals_count, 1, "Should have 1 Equals token");
}

#[test]
fn equals_in_conditional() {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new("If x equals 5, then y equals 10.", &mut interner);
    let tokens = lexer.tokenize();

    let equals_count = tokens.iter().filter(|t| matches!(t.kind, TokenType::Equals)).count();
    assert_eq!(equals_count, 2, "Should have 2 Equals tokens");
}
