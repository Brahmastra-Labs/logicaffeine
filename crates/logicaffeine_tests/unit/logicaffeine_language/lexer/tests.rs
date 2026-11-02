use super::*;

#[test]
fn lexer_handles_apostrophe() {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new("it's raining", &mut interner);
    let tokens = lexer.tokenize();
    assert!(!tokens.is_empty());
}

#[test]
fn lexer_handles_question_mark() {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new("Is it raining?", &mut interner);
    let tokens = lexer.tokenize();
    assert!(!tokens.is_empty());
}

#[test]
fn ring_is_not_verb() {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new("ring", &mut interner);
    let tokens = lexer.tokenize();
    assert!(matches!(tokens[0].kind, TokenType::Noun(_)));
}

/// Rung 0a, Stride 0: `## Define` introduces a vernacular-logic predicate
/// definition. It must lex to its OWN block type — never collapse into the
/// pre-existing `## Definition` (which `DiscoveryPass` consumes for type
/// defs). This pins both: `## Define` → `Define`, `## Definition` unchanged.
#[test]
fn define_block_header_is_distinct_from_definition() {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new("## Define\n", &mut interner);
    let tokens = lexer.tokenize();
    let header = tokens
        .iter()
        .find(|t| matches!(t.kind, TokenType::BlockHeader { .. }))
        .expect("## Define should produce a block header token");
    assert!(
        matches!(
            header.kind,
            TokenType::BlockHeader {
                block_type: BlockType::Define
            }
        ),
        "## Define must tokenize to BlockType::Define, got {:?}",
        header.kind
    );

    // Regression guard: the long-standing `## Definition` block stays itself.
    let mut interner2 = Interner::new();
    let mut lexer2 = Lexer::new("## Definition\n", &mut interner2);
    let tokens2 = lexer2.tokenize();
    let header2 = tokens2
        .iter()
        .find(|t| matches!(t.kind, TokenType::BlockHeader { .. }))
        .expect("## Definition should produce a block header token");
    assert!(
        matches!(
            header2.kind,
            TokenType::BlockHeader {
                block_type: BlockType::Definition
            }
        ),
        "## Definition must still tokenize to BlockType::Definition, got {:?}",
        header2.kind
    );
}

#[test]
fn debug_that_token() {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new("The cat that runs", &mut interner);
    let tokens = lexer.tokenize();
    for (i, t) in tokens.iter().enumerate() {
        let lex = interner.resolve(t.lexeme);
        eprintln!("Token[{}]: {:?} -> {:?}", i, lex, t.kind);
    }
    let that_token = tokens.iter().find(|t| interner.resolve(t.lexeme) == "that");
    if let Some(t) = that_token {
        // Verify discriminant comparison works
        let check = std::mem::discriminant(&t.kind) == std::mem::discriminant(&TokenType::That);
        eprintln!("Discriminant check for That: {}", check);
        assert!(
            matches!(t.kind, TokenType::That),
            "'that' should be TokenType::That, got {:?}",
            t.kind
        );
    } else {
        panic!("No 'that' token found");
    }
}

#[test]
fn bus_is_not_verb() {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new("bus", &mut interner);
    let tokens = lexer.tokenize();
    assert!(matches!(tokens[0].kind, TokenType::Noun(_)));
}

#[test]
fn lowercase_a_is_article() {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new("a car", &mut interner);
    let tokens = lexer.tokenize();
    for (i, t) in tokens.iter().enumerate() {
        let lex = interner.resolve(t.lexeme);
        eprintln!("Token[{}]: {:?} -> {:?}", i, lex, t.kind);
    }
    assert_eq!(tokens[0].kind, TokenType::Article(Definiteness::Indefinite));
    assert!(
        matches!(tokens[1].kind, TokenType::Noun(_)),
        "Expected Noun, got {:?}",
        tokens[1].kind
    );
}

#[test]
fn open_is_ambiguous() {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new("open", &mut interner);
    let tokens = lexer.tokenize();

    if let TokenType::Ambiguous {
        primary,
        alternatives,
    } = &tokens[0].kind
    {
        assert!(
            matches!(**primary, TokenType::Verb { .. }),
            "Primary should be Verb"
        );
        assert!(
            alternatives
                .iter()
                .any(|t| matches!(t, TokenType::Adjective(_))),
            "Should have Adjective alternative"
        );
    } else {
        panic!(
            "Expected Ambiguous token for 'open', got {:?}",
            tokens[0].kind
        );
    }
}

#[test]
fn basic_tokenization() {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new("All men are mortal.", &mut interner);
    let tokens = lexer.tokenize();
    assert_eq!(tokens[0].kind, TokenType::All);
    assert!(matches!(tokens[1].kind, TokenType::Noun(_)));
    assert_eq!(tokens[2].kind, TokenType::Are);
}

#[test]
fn iff_tokenizes_as_single_token() {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new("A if and only if B", &mut interner);
    let tokens = lexer.tokenize();
    assert!(
        tokens.iter().any(|t| t.kind == TokenType::Iff),
        "should contain Iff token: got {:?}",
        tokens
    );
}

#[test]
fn is_equal_to_tokenizes_as_identity() {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new("Socrates is equal to Socrates", &mut interner);
    let tokens = lexer.tokenize();
    assert!(
        tokens.iter().any(|t| t.kind == TokenType::Identity),
        "should contain Identity token: got {:?}",
        tokens
    );
}

#[test]
fn is_identical_to_tokenizes_as_identity() {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new("Clark is identical to Superman", &mut interner);
    let tokens = lexer.tokenize();
    assert!(
        tokens.iter().any(|t| t.kind == TokenType::Identity),
        "should contain Identity token: got {:?}",
        tokens
    );
}

#[test]
fn itself_tokenizes_as_reflexive() {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new("John loves itself", &mut interner);
    let tokens = lexer.tokenize();
    assert!(
        tokens.iter().any(|t| t.kind == TokenType::Reflexive),
        "should contain Reflexive token: got {:?}",
        tokens
    );
}

#[test]
fn himself_tokenizes_as_reflexive() {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new("John sees himself", &mut interner);
    let tokens = lexer.tokenize();
    assert!(
        tokens.iter().any(|t| t.kind == TokenType::Reflexive),
        "should contain Reflexive token: got {:?}",
        tokens
    );
}

#[test]
fn to_stay_tokenizes_correctly() {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new("to stay", &mut interner);
    let tokens = lexer.tokenize();
    assert!(
        tokens.iter().any(|t| t.kind == TokenType::To),
        "should contain To token: got {:?}",
        tokens
    );
    assert!(
        tokens
            .iter()
            .any(|t| matches!(t.kind, TokenType::Verb { .. })),
        "should contain Verb token for stay: got {:?}",
        tokens
    );
}

#[test]
fn possessive_apostrophe_s() {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new("John's dog", &mut interner);
    let tokens = lexer.tokenize();
    assert!(
        tokens.iter().any(|t| t.kind == TokenType::Possessive),
        "should contain Possessive token: got {:?}",
        tokens
    );
    assert!(
        tokens
            .iter()
            .any(|t| matches!(&t.kind, TokenType::ProperName(_))),
        "should have John as proper name: got {:?}",
        tokens
    );
}

#[test]
fn lexer_produces_valid_spans() {
    let input = "All men are mortal.";
    let mut interner = Interner::new();
    let mut lexer = Lexer::new(input, &mut interner);
    let tokens = lexer.tokenize();

    // "All" at 0..3
    assert_eq!(tokens[0].span.start, 0);
    assert_eq!(tokens[0].span.end, 3);
    assert_eq!(&input[tokens[0].span.start..tokens[0].span.end], "All");

    // "men" at 4..7
    assert_eq!(tokens[1].span.start, 4);
    assert_eq!(tokens[1].span.end, 7);
    assert_eq!(&input[tokens[1].span.start..tokens[1].span.end], "men");

    // "are" at 8..11
    assert_eq!(tokens[2].span.start, 8);
    assert_eq!(tokens[2].span.end, 11);
    assert_eq!(&input[tokens[2].span.start..tokens[2].span.end], "are");

    // "mortal" at 12..18
    assert_eq!(tokens[3].span.start, 12);
    assert_eq!(tokens[3].span.end, 18);
    assert_eq!(&input[tokens[3].span.start..tokens[3].span.end], "mortal");

    // "." at 18..19
    assert_eq!(tokens[4].span.start, 18);
    assert_eq!(tokens[4].span.end, 19);

    // EOF at end
    assert_eq!(tokens[5].span.start, input.len());
    assert_eq!(tokens[5].kind, TokenType::EOF);
}

#[test]
fn triple_quote_produces_string_token() {
    let mut interner = Interner::new();
    let source = "## Main\nLet msg be \"\"\"\n    Hello\n    World\n\"\"\".\nShow msg.";
    let mut lexer = Lexer::new(source, &mut interner);
    let tokens = lexer.tokenize();
    // Dump all tokens for debugging
    for (i, t) in tokens.iter().enumerate() {
        let lex = interner.resolve(t.lexeme);
        eprintln!(
            "Token[{}]: {:?} lex={:?} span={}..{}",
            i, t.kind, lex, t.span.start, t.span.end
        );
    }
    // Find the string token
    let str_token = tokens.iter().find(|t| {
        matches!(
            t.kind,
            TokenType::StringLiteral(_) | TokenType::InterpolatedString(_)
        )
    });
    assert!(
        str_token.is_some(),
        "Should have a string token. Tokens: {:?}",
        tokens
            .iter()
            .map(|t| format!("{:?}", t.kind))
            .collect::<Vec<_>>()
    );
    if let Some(tok) = str_token {
        let content = interner.resolve(tok.lexeme);
        eprintln!("Triple-quote content: {:?}", content);
        assert!(
            content.contains("Hello"),
            "Should contain Hello, got: {:?}",
            content
        );
    }
}

/// BUG-015: a string-literal span must stay BYTE-indexed even when multibyte
/// text precedes it; a byte-start / char-end span underflows in
/// `insert_indentation_tokens` and panics on valid Unicode input.
#[test]
fn string_literal_span_stays_byte_indexed_after_leading_multibyte_text() {
    // 7 leading 2-byte Greek letters, a space, then a quoted string.
    let src = "\u{3b1}\u{3b1}\u{3b1}\u{3b1}\u{3b1}\u{3b1}\u{3b1} \"x\"";
    let mut interner = Interner::new();
    let mut lexer = Lexer::new(src, &mut interner);
    let tokens = lexer.tokenize(); // previously panicked: 'attempt to subtract with overflow'

    let s = tokens
        .iter()
        .find(|t| {
            matches!(
                t.kind,
                TokenType::StringLiteral(_) | TokenType::InterpolatedString(_)
            )
        })
        .expect("should produce a string literal token");

    assert!(
        s.span.end >= s.span.start,
        "span end {} must be >= start {}",
        s.span.end,
        s.span.start
    );
    assert!(
        src.is_char_boundary(s.span.start) && src.is_char_boundary(s.span.end),
        "span [{}, {}) must lie on char boundaries",
        s.span.start,
        s.span.end
    );
    assert_eq!(&src[s.span.start..s.span.end], "\"x\"");
}

/// BUG-030: calendar-impossible dates (Feb 30, Apr 31, Feb 29 in a non-leap
/// year) must NOT tokenize to a DateLiteral — they would otherwise silently
/// map onto a real, different day.
#[test]
fn date_literal_rejects_impossible_day_of_month() {
    let impossible = [
        "2026-02-30",
        "2026-02-31",
        "2026-02-29", // 2026 is not a leap year
        "2026-04-31",
        "2026-06-31",
        "2026-09-31",
        "2026-11-31",
    ];
    for input in impossible {
        let mut interner = Interner::new();
        let mut lexer = Lexer::new(input, &mut interner);
        let tokens = lexer.tokenize();
        assert!(
            !tokens
                .iter()
                .any(|t| matches!(t.kind, TokenType::DateLiteral { .. })),
            "{input} is not a real calendar date and must not become a DateLiteral; got {tokens:?}"
        );
    }
    // Control: a real leap-day must still tokenize.
    let mut interner = Interner::new();
    let mut lexer = Lexer::new("2024-02-29", &mut interner);
    let tokens = lexer.tokenize();
    assert!(
        tokens
            .iter()
            .any(|t| matches!(t.kind, TokenType::DateLiteral { .. })),
        "2024-02-29 is a valid leap-day and must still tokenize; got {tokens:?}"
    );
}

fn kinds_and_lexemes(source: &str) -> Vec<(TokenType, String)> {
    let mut interner = Interner::new();
    let tokens = Lexer::new(source, &mut interner).tokenize();
    tokens
        .into_iter()
        .map(|t| (t.kind.clone(), interner.resolve(t.lexeme).to_string()))
        .collect()
}

#[test]
fn a_phrase_token_covers_its_words_and_keeps_the_punctuation_after_them() {
    let source = "Show that x holds if and only if y holds. The light flashes infinitely often.";
    let mut interner = Interner::new();
    let tokens = Lexer::new(source, &mut interner).tokenize();
    let iff = tokens.iter().find(|t| t.kind == TokenType::Iff).expect("one token for the phrase");
    assert_eq!(interner.resolve(iff.lexeme), "if and only if");
    assert_eq!(&source[iff.span.start..iff.span.end], "if and only if");
    let often = tokens
        .iter()
        .position(|t| t.kind == TokenType::InfinitelyOften)
        .expect("one token for the phrase");
    assert_eq!(interner.resolve(tokens[often].lexeme), "infinitely often");
    assert_eq!(tokens[often + 1].kind, TokenType::Period, "the period after `often` follows the phrase");
}

#[test]
fn a_block_headers_data_words_are_not_its_phrase() {
    let lexed = kinds_and_lexemes("## Route GET /posts/:id (id: Int) -> Text:\n    Return \"post\".\n");
    assert!(matches!(lexed[0].0, TokenType::BlockHeader { .. }));
    assert!(!lexed[0].1.contains("GET"), "the header's lexeme is its own word: {:?}", lexed[0].1);
    assert!(
        lexed.iter().any(|(k, _)| *k == TokenType::Colon),
        "the colon ending the header's signature is still a token: {lexed:?}"
    );
}
