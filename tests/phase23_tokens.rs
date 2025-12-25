use logos::{Interner, Lexer, TokenType};

#[test]
fn let_token_recognized() {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new("Let x be 5.", &mut interner);
    let tokens = lexer.tokenize();

    assert!(
        tokens.iter().any(|t| matches!(t.kind, TokenType::Let)),
        "Should have Let token: {:?}",
        tokens
    );
}

#[test]
fn set_token_recognized() {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new("Set x to 10.", &mut interner);
    let tokens = lexer.tokenize();

    assert!(
        tokens.iter().any(|t| matches!(t.kind, TokenType::Set)),
        "Should have Set token: {:?}",
        tokens
    );
}

#[test]
fn return_token_recognized() {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new("Return x.", &mut interner);
    let tokens = lexer.tokenize();

    assert!(
        tokens.iter().any(|t| matches!(t.kind, TokenType::Return)),
        "Should have Return token: {:?}",
        tokens
    );
}

#[test]
fn be_token_after_let() {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new("Let x be 5.", &mut interner);
    let tokens = lexer.tokenize();

    // "be" is tokenized as Be keyword ONLY after Let
    assert!(
        tokens.iter().any(|t| matches!(t.kind, TokenType::Be)),
        "Should have Be token after Let: {:?}",
        tokens
    );
}

#[test]
fn be_not_keyword_in_passive() {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new("to be seen", &mut interner);
    let tokens = lexer.tokenize();

    // "be" should NOT be a Be keyword in passive constructions
    assert!(
        !tokens.iter().any(|t| matches!(t.kind, TokenType::Be)),
        "Be should NOT be a keyword in passive: {:?}",
        tokens
    );
}

#[test]
fn while_token_recognized() {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new("While x is true.", &mut interner);
    let tokens = lexer.tokenize();

    assert!(
        tokens.iter().any(|t| matches!(t.kind, TokenType::While)),
        "Should have While token: {:?}",
        tokens
    );
}

#[test]
fn assert_token_recognized() {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new("Assert that x.", &mut interner);
    let tokens = lexer.tokenize();

    assert!(
        tokens.iter().any(|t| matches!(t.kind, TokenType::Assert)),
        "Should have Assert token: {:?}",
        tokens
    );
}

#[test]
fn otherwise_token_recognized() {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new("Otherwise return false.", &mut interner);
    let tokens = lexer.tokenize();

    assert!(
        tokens.iter().any(|t| matches!(t.kind, TokenType::Otherwise)),
        "Should have Otherwise token: {:?}",
        tokens
    );
}

#[test]
fn let_be_sequence_tokenizes() {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new("Let count be 0.", &mut interner);
    let tokens = lexer.tokenize();

    let has_let = tokens.iter().any(|t| matches!(t.kind, TokenType::Let));
    let has_be = tokens.iter().any(|t| matches!(t.kind, TokenType::Be));

    assert!(has_let, "Should have Let token: {:?}", tokens);
    assert!(has_be, "Should have Be token: {:?}", tokens);
}
