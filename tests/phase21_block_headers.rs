use logos::{Interner, Lexer, TokenType};

#[test]
fn theorem_block_detected() {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new("## Theorem\nAll men are mortal.", &mut interner);
    let tokens = lexer.tokenize();

    assert!(!tokens.is_empty(), "Expected tokens from input");
    assert!(
        matches!(tokens[0].kind, TokenType::BlockHeader { .. }),
        "First token should be BlockHeader, got {:?}",
        tokens[0].kind
    );
}

#[test]
fn main_block_detected() {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new("## Main\nLet x be 5.", &mut interner);
    let tokens = lexer.tokenize();

    assert!(!tokens.is_empty(), "Expected tokens from input");
    assert!(
        matches!(tokens[0].kind, TokenType::BlockHeader { .. }),
        "First token should be BlockHeader, got {:?}",
        tokens[0].kind
    );
}

#[test]
fn definition_block_detected() {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new("## Definition\nA bachelor is an unmarried man.", &mut interner);
    let tokens = lexer.tokenize();

    assert!(!tokens.is_empty(), "Expected tokens from input");
    assert!(
        matches!(tokens[0].kind, TokenType::BlockHeader { .. }),
        "First token should be BlockHeader, got {:?}",
        tokens[0].kind
    );
}

#[test]
fn block_header_preserves_content() {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new("## Theorem\nAll men are mortal.", &mut interner);
    let tokens = lexer.tokenize();

    // After the BlockHeader, we should have normal tokens
    // "All" should be a quantifier
    let has_all = tokens.iter().any(|t| matches!(t.kind, TokenType::All));
    assert!(has_all, "Should have 'All' quantifier token after block header");
}

#[test]
fn non_block_input_unchanged() {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new("All men are mortal.", &mut interner);
    let tokens = lexer.tokenize();

    // First token should be "All", not a BlockHeader
    assert!(
        matches!(tokens[0].kind, TokenType::All),
        "First token should be All quantifier without block header, got {:?}",
        tokens[0].kind
    );
}

#[test]
fn single_hash_not_block_header() {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new("# Note\nThis is a comment.", &mut interner);
    let tokens = lexer.tokenize();

    // Single # should NOT create a BlockHeader (only ## does)
    assert!(
        !matches!(tokens[0].kind, TokenType::BlockHeader { .. }),
        "Single # should not create BlockHeader"
    );
}
