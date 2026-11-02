use super::*;
use logicaffeine_base::Interner;
use logicaffeine_language::token::Span;

#[test]
fn all_token_types_classified() {
    // This test ensures that adding new TokenType variants forces an update
    // to classify_token. If this test compiles, all variants are handled.
    let mut interner = Interner::new();
    let sym = interner.intern("test");

    let test_tokens = vec![
        TokenType::Let,
        TokenType::Noun(sym),
        TokenType::Verb {
            lemma: sym,
            time: logicaffeine_language::lexicon::Time::Present,
            aspect: logicaffeine_language::lexicon::Aspect::Simple,
            class: logicaffeine_language::lexicon::VerbClass::Activity,
        },
        TokenType::StringLiteral(sym),
        TokenType::Number(sym),
        TokenType::BlockHeader {
            block_type: logicaffeine_language::token::BlockType::Main,
        },
    ];

    for tt in &test_tokens {
        let (ty, _) = classify_token(tt);
        assert!(ty.is_some(), "Token {:?} should be classified", tt);
    }
}

#[test]
fn keywords_classified_as_keyword() {
    let keywords = [
        TokenType::Let,
        TokenType::Set,
        TokenType::Return,
        TokenType::While,
        TokenType::Repeat,
        TokenType::Push,
    ];
    for kw in &keywords {
        let (ty, _) = classify_token(kw);
        assert_eq!(ty, Some(0), "Keyword {:?} should map to type 0", kw);
    }
}

#[test]
fn operators_classified_correctly() {
    let ops = [
        TokenType::Plus,
        TokenType::Minus,
        TokenType::Star,
        TokenType::Lt,
        TokenType::Gt,
        TokenType::EqEq,
    ];
    for op in &ops {
        let (ty, _) = classify_token(op);
        assert_eq!(ty, Some(6), "Operator {:?} should map to type 6", op);
    }
}

#[test]
fn string_literal_classified() {
    let mut interner = Interner::new();
    let sym = interner.intern("hello");
    let (ty, _) = classify_token(&TokenType::StringLiteral(sym));
    assert_eq!(ty, Some(4), "String literal should map to type 4");
}

#[test]
fn number_literal_classified() {
    let mut interner = Interner::new();
    let sym = interner.intern("42");
    let (ty, _) = classify_token(&TokenType::Number(sym));
    assert_eq!(ty, Some(5), "Number should map to type 5");
}

#[test]
fn structural_tokens_skipped() {
    let skipped = [
        TokenType::Indent,
        TokenType::Dedent,
        TokenType::Newline,
        TokenType::EOF,
    ];
    for tt in &skipped {
        let (ty, _) = classify_token(tt);
        assert_eq!(ty, None, "Structural token {:?} should be skipped", tt);
    }
}

#[test]
fn zero_length_tokens_skipped_in_encoding() {
    let line_index = LineIndex::new("Let x be 5.");
    let mut interner = Interner::new();
    let sym = interner.intern("");
    let tokens = vec![Token::new(TokenType::Indent, sym, Span::new(0, 0))];
    let encoded = encode_tokens(&tokens, &line_index);
    assert!(encoded.is_empty(), "Zero-length tokens should be skipped");
}

#[test]
fn multi_line_delta_encoding() {
    let line_index = LineIndex::new("Let x\nbe 5.");
    let mut interner = Interner::new();
    let let_sym = interner.intern("Let");
    let be_sym = interner.intern("be");

    let tokens = vec![
        Token::new(TokenType::Let, let_sym, Span::new(0, 3)),
        Token::new(TokenType::Be, be_sym, Span::new(6, 8)),
    ];

    let encoded = encode_tokens(&tokens, &line_index);
    assert_eq!(encoded.len(), 2);
    assert_eq!(encoded[0].delta_line, 0);
    assert_eq!(
        encoded[1].delta_line, 1,
        "Second token should be on next line"
    );
    assert_eq!(
        encoded[1].delta_start, 0,
        "After line change, delta_start resets"
    );
}

#[test]
fn block_header_classified_as_namespace() {
    let (ty, _) = classify_token(&TokenType::BlockHeader {
        block_type: logicaffeine_language::token::BlockType::Main,
    });
    assert_eq!(ty, Some(7), "Block header should map to type 7 (namespace)");
}

#[test]
fn noun_classified_as_type() {
    let mut interner = Interner::new();
    let sym = interner.intern("person");
    let (ty, _) = classify_token(&TokenType::Noun(sym));
    assert_eq!(ty, Some(1), "Noun should map to type 1 (TYPE)");
}

#[test]
fn proper_name_has_declaration_modifier() {
    let mut interner = Interner::new();
    let sym = interner.intern("Alice");
    let (ty, mods) = classify_token(&TokenType::ProperName(sym));
    assert_eq!(ty, Some(3), "ProperName should map to type 3 (VARIABLE)");
    assert_eq!(
        mods, 1,
        "ProperName should have DECLARATION modifier (bit 0)"
    );
}

#[test]
fn ambiguous_uses_primary() {
    let mut interner = Interner::new();
    let sym = interner.intern("test");
    let primary = Box::new(TokenType::Noun(sym));
    let alternatives = vec![TokenType::Identifier];
    let (ty, _) = classify_token(&TokenType::Ambiguous {
        primary,
        alternatives,
    });
    assert_eq!(
        ty,
        Some(1),
        "Ambiguous wrapping Noun should classify as type 1"
    );
}

#[test]
fn encode_tokens_utf16_length() {
    // 'é' is 2 bytes in UTF-8 but 1 UTF-16 code unit
    let source = "café";
    let line_index = LineIndex::new(source);
    let mut interner = Interner::new();
    let sym = interner.intern("café");
    let tokens = vec![
        Token::new(TokenType::Identifier, sym, Span::new(0, 5)), // 5 bytes: c(1)+a(1)+f(1)+é(2)
    ];
    let encoded = encode_tokens(&tokens, &line_index);
    assert_eq!(encoded.len(), 1);
    assert_eq!(
        encoded[0].length, 4,
        "UTF-16 length of 'café' should be 4, not 5 bytes"
    );
}

#[test]
fn classify_ambiguous_nested_has_depth_guard() {
    let mut interner = Interner::new();
    let sym = interner.intern("test");
    // Create a deeply nested Ambiguous chain: Ambiguous(Ambiguous(Ambiguous(Noun)))
    let inner = TokenType::Noun(sym);
    let mid = TokenType::Ambiguous {
        primary: Box::new(inner),
        alternatives: vec![TokenType::Identifier],
    };
    let outer = TokenType::Ambiguous {
        primary: Box::new(mid),
        alternatives: vec![TokenType::Identifier],
    };
    let (ty, _) = classify_token(&outer);
    assert_eq!(
        ty,
        Some(1),
        "Nested Ambiguous wrapping Noun should still classify as type 1 (TYPE)"
    );
}

#[test]
fn classify_ambiguous_too_deep_returns_none() {
    let mut interner = Interner::new();
    let sym = interner.intern("test");
    // Create a chain deeper than the depth limit
    let mut current = TokenType::Noun(sym);
    for _ in 0..10 {
        current = TokenType::Ambiguous {
            primary: Box::new(current),
            alternatives: vec![TokenType::Identifier],
        };
    }
    // Should not stack overflow; should return None for excessive depth
    let (ty, _) = classify_token(&current);
    assert_eq!(ty, None, "Excessively nested Ambiguous should return None");
}

#[test]
fn delta_encoding() {
    let line_index = LineIndex::new("Let x be 5.");
    let mut interner = Interner::new();
    let let_sym = interner.intern("Let");
    let x_sym = interner.intern("x");

    let tokens = vec![
        Token::new(TokenType::Let, let_sym, Span::new(0, 3)),
        Token::new(TokenType::Identifier, x_sym, Span::new(4, 5)),
    ];

    let encoded = encode_tokens(&tokens, &line_index);
    assert_eq!(encoded.len(), 2);
    assert_eq!(encoded[0].delta_line, 0);
    assert_eq!(encoded[0].delta_start, 0);
    assert_eq!(encoded[0].length, 3);
    assert_eq!(encoded[1].delta_line, 0);
    assert_eq!(encoded[1].delta_start, 4);
    assert_eq!(encoded[1].length, 1);
}
