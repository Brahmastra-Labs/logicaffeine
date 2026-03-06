use tower_lsp::lsp_types::{
    SemanticToken, SemanticTokenType, SemanticTokensLegend, SemanticTokenModifier,
};

use logicaffeine_language::token::{Token, TokenType};

use crate::line_index::LineIndex;

/// Our semantic token types, registered with the client.
pub const TOKEN_TYPES: &[SemanticTokenType] = &[
    SemanticTokenType::KEYWORD,    // 0
    SemanticTokenType::TYPE,       // 1
    SemanticTokenType::FUNCTION,   // 2
    SemanticTokenType::VARIABLE,   // 3
    SemanticTokenType::STRING,     // 4
    SemanticTokenType::NUMBER,     // 5
    SemanticTokenType::OPERATOR,   // 6
    SemanticTokenType::NAMESPACE,  // 7
    SemanticTokenType::MODIFIER,   // 8
    SemanticTokenType::PROPERTY,   // 9
    SemanticTokenType::COMMENT,    // 10
    SemanticTokenType::PARAMETER,  // 11
    SemanticTokenType::ENUM_MEMBER, // 12
];

pub const TOKEN_MODIFIERS: &[SemanticTokenModifier] = &[
    SemanticTokenModifier::DECLARATION, // 0
    SemanticTokenModifier::READONLY,    // 1
];

pub fn legend() -> SemanticTokensLegend {
    SemanticTokensLegend {
        token_types: TOKEN_TYPES.to_vec(),
        token_modifiers: TOKEN_MODIFIERS.to_vec(),
    }
}

/// Convert a token stream to LSP semantic tokens (delta-encoded).
pub fn encode_tokens(tokens: &[Token], line_index: &LineIndex) -> Vec<SemanticToken> {
    let mut result = Vec::with_capacity(tokens.len());
    let mut prev_line = 0u32;
    let mut prev_start = 0u32;

    for token in tokens {
        let (token_type, modifiers) = classify_token(&token.kind);
        let token_type = match token_type {
            Some(t) => t,
            None => continue, // Skip tokens we don't highlight
        };

        let pos = line_index.position(token.span.start);
        let length = line_index.utf16_length(token.span.start, token.span.end);

        if length == 0 {
            continue;
        }

        let delta_line = pos.line - prev_line;
        let delta_start = if delta_line == 0 {
            pos.character - prev_start
        } else {
            pos.character
        };

        result.push(SemanticToken {
            delta_line,
            delta_start,
            length,
            token_type,
            token_modifiers_bitset: modifiers,
        });

        prev_line = pos.line;
        prev_start = pos.character;
    }

    result
}

/// Map a `TokenType` to a semantic token type index and modifier bitset.
///
/// Returns `(Some(type_index), modifier_bits)` for highlighted tokens,
/// `(None, 0)` for tokens that shouldn't be highlighted.
fn classify_token(kind: &TokenType) -> (Option<u32>, u32) {
    classify_token_inner(kind, 0)
}

const MAX_AMBIGUOUS_DEPTH: usize = 3;

fn classify_token_inner(kind: &TokenType, depth: usize) -> (Option<u32>, u32) {
    match kind {
        // Quantifiers → keyword
        TokenType::All | TokenType::No | TokenType::Some | TokenType::Any
        | TokenType::Both | TokenType::Most | TokenType::Few | TokenType::Many
        | TokenType::Cardinal(_) | TokenType::AtLeast(_) | TokenType::AtMost(_) => {
            (Some(0), 0) // keyword
        }

        // Logical connectives → operator
        TokenType::And | TokenType::Or | TokenType::Not
        | TokenType::If | TokenType::Then | TokenType::Iff | TokenType::Because => {
            (Some(6), 0) // operator
        }

        // Modal operators → keyword
        TokenType::Must | TokenType::Shall | TokenType::Should | TokenType::Can
        | TokenType::May | TokenType::Cannot | TokenType::Would | TokenType::Could
        | TokenType::Might | TokenType::Had => {
            (Some(0), 0) // keyword
        }

        // Imperative keywords → keyword
        TokenType::Let | TokenType::Set | TokenType::Return | TokenType::Break | TokenType::Be
        | TokenType::While | TokenType::Repeat | TokenType::For | TokenType::In
        | TokenType::From | TokenType::Assert | TokenType::Trust
        | TokenType::Otherwise | TokenType::Else | TokenType::Elif
        | TokenType::Call | TokenType::New | TokenType::Either | TokenType::Inspect
        | TokenType::Native | TokenType::Escape | TokenType::Given | TokenType::Prove
        | TokenType::Auto | TokenType::Read | TokenType::Write | TokenType::Console
        | TokenType::File | TokenType::Give | TokenType::Show | TokenType::Push
        | TokenType::Pop | TokenType::Copy | TokenType::Through | TokenType::Length
        | TokenType::At | TokenType::Add | TokenType::Remove | TokenType::Contains
        | TokenType::Union | TokenType::Intersection | TokenType::Inside
        | TokenType::Zone | TokenType::Called | TokenType::Size | TokenType::Mapped
        | TokenType::Attempt | TokenType::Following | TokenType::Simultaneously
        | TokenType::Spawn | TokenType::Send | TokenType::Await | TokenType::Portable
        | TokenType::Manifest | TokenType::Chunk | TokenType::Shared | TokenType::Merge
        | TokenType::Increase | TokenType::Decrease | TokenType::Tally
        | TokenType::SharedSet | TokenType::SharedSequence | TokenType::CollaborativeSequence
        | TokenType::SharedMap | TokenType::Divergent | TokenType::Append
        | TokenType::Resolve | TokenType::RemoveWins | TokenType::AddWins
        | TokenType::YATA | TokenType::Values | TokenType::Check | TokenType::Listen
        | TokenType::NetConnect | TokenType::Sleep | TokenType::Sync | TokenType::Mount
        | TokenType::Persistent | TokenType::Combined | TokenType::Launch | TokenType::Task
        | TokenType::Pipe | TokenType::Receive | TokenType::Stop | TokenType::Try
        | TokenType::Into | TokenType::First | TokenType::After | TokenType::Mut => {
            (Some(0), 0) // keyword
        }

        // Nouns → type
        TokenType::Noun(_) => (Some(1), 0),

        // Verbs → function
        TokenType::Verb { .. } => (Some(2), 0),

        // Adjectives → modifier
        TokenType::Adjective(_) | TokenType::NonIntersectiveAdjective(_) => {
            (Some(8), 0) // modifier
        }

        // Proper names → variable with declaration modifier
        TokenType::ProperName(_) => (Some(3), 1), // variable + declaration

        // Pronouns → variable
        TokenType::Pronoun { .. } => (Some(3), 0),

        // Articles → keyword
        TokenType::Article(_) => (Some(0), 0),

        // Copula → keyword
        TokenType::Is | TokenType::Are | TokenType::Was | TokenType::Were => {
            (Some(0), 0)
        }

        // Wh-words → keyword
        TokenType::That | TokenType::Who | TokenType::What | TokenType::Where
        | TokenType::When | TokenType::Why | TokenType::Does | TokenType::Do => {
            (Some(0), 0)
        }

        // Identity/reflexive → keyword
        TokenType::Identity | TokenType::Equals | TokenType::Reflexive
        | TokenType::Reciprocal | TokenType::Respectively => {
            (Some(0), 0)
        }

        // String literals → string
        TokenType::StringLiteral(_) | TokenType::InterpolatedString(_) | TokenType::CharLiteral(_) => (Some(4), 0),

        // Number literals → number
        TokenType::Number(_) | TokenType::DurationLiteral { .. }
        | TokenType::DateLiteral { .. } | TokenType::TimeLiteral { .. } => {
            (Some(5), 0)
        }

        // Operators → operator
        TokenType::Plus | TokenType::Minus | TokenType::Star | TokenType::Slash
        | TokenType::Percent | TokenType::Lt | TokenType::Gt | TokenType::LtEq
        | TokenType::GtEq | TokenType::EqEq | TokenType::NotEq | TokenType::Arrow
        | TokenType::Assign | TokenType::Xor | TokenType::Shifted => {
            (Some(6), 0)
        }

        // Block headers → namespace
        TokenType::BlockHeader { .. } => (Some(7), 0),

        // Prepositions → keyword
        TokenType::Preposition(_) => (Some(0), 0),

        // Generic identifier → variable
        TokenType::Identifier => (Some(3), 0),

        // Comparatives/superlatives → modifier
        TokenType::Comparative(_) | TokenType::Superlative(_) | TokenType::Than => {
            (Some(8), 0)
        }

        // Calendar/temporal → keyword
        TokenType::CalendarUnit(_) | TokenType::Ago | TokenType::Hence
        | TokenType::Before | TokenType::TemporalAdverb(_) => {
            (Some(0), 0)
        }

        // Item/Items → keyword
        TokenType::Item | TokenType::Items => (Some(0), 0),

        // Adverbs → modifier
        TokenType::Adverb(_) | TokenType::ScopalAdverb(_) => (Some(8), 0),

        // Focus/presup → keyword
        TokenType::Focus(_) | TokenType::PresupTrigger(_) | TokenType::Measure(_) => {
            (Some(0), 0)
        }

        // Auxiliary → keyword
        TokenType::Auxiliary(_) => (Some(0), 0),

        // Escape blocks → string (raw code)
        TokenType::EscapeBlock(_) => (Some(4), 0),

        // Performative → keyword
        TokenType::Performative(_) => (Some(0), 0),
        TokenType::Exclamation => (Some(0), 0),

        // NPIs → keyword
        TokenType::Anything | TokenType::Anyone | TokenType::Nothing
        | TokenType::Nobody | TokenType::NoOne | TokenType::Nowhere
        | TokenType::Ever | TokenType::Never => {
            (Some(0), 0)
        }

        // Ambiguous → try primary classification with depth guard
        TokenType::Ambiguous { primary, .. } => {
            if depth >= MAX_AMBIGUOUS_DEPTH {
                return (None, 0);
            }
            classify_token_inner(primary, depth + 1)
        }

        // Particles → keyword
        TokenType::Particle(_) => (Some(0), 0),

        // Control → keyword
        TokenType::To => (Some(0), 0),

        // Possessive → operator
        TokenType::Possessive => (Some(6), 0),

        // Punctuation → skip (or operator for comma/period)
        TokenType::Period | TokenType::Comma | TokenType::Colon => (Some(6), 0),
        TokenType::LParen | TokenType::RParen | TokenType::LBracket | TokenType::RBracket => {
            (Some(6), 0)
        }

        // Structural tokens → skip
        TokenType::Indent | TokenType::Dedent | TokenType::Newline | TokenType::EOF => {
            (None, 0)
        }
    }
}

#[cfg(test)]
mod tests {
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
            TokenType::BlockHeader { block_type: logicaffeine_language::token::BlockType::Main },
        ];

        for tt in &test_tokens {
            let (ty, _) = classify_token(tt);
            assert!(ty.is_some(), "Token {:?} should be classified", tt);
        }
    }

    #[test]
    fn keywords_classified_as_keyword() {
        let keywords = [
            TokenType::Let, TokenType::Set, TokenType::Return,
            TokenType::While, TokenType::Repeat, TokenType::Push,
        ];
        for kw in &keywords {
            let (ty, _) = classify_token(kw);
            assert_eq!(ty, Some(0), "Keyword {:?} should map to type 0", kw);
        }
    }

    #[test]
    fn operators_classified_correctly() {
        let ops = [
            TokenType::Plus, TokenType::Minus, TokenType::Star,
            TokenType::Lt, TokenType::Gt, TokenType::EqEq,
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
            TokenType::Indent, TokenType::Dedent,
            TokenType::Newline, TokenType::EOF,
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
        let tokens = vec![
            Token::new(TokenType::Indent, sym, Span::new(0, 0)),
        ];
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
        assert_eq!(encoded[1].delta_line, 1, "Second token should be on next line");
        assert_eq!(encoded[1].delta_start, 0, "After line change, delta_start resets");
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
        assert_eq!(mods, 1, "ProperName should have DECLARATION modifier (bit 0)");
    }

    #[test]
    fn ambiguous_uses_primary() {
        let mut interner = Interner::new();
        let sym = interner.intern("test");
        let primary = Box::new(TokenType::Noun(sym));
        let alternatives = vec![TokenType::Identifier];
        let (ty, _) = classify_token(&TokenType::Ambiguous { primary, alternatives });
        assert_eq!(ty, Some(1), "Ambiguous wrapping Noun should classify as type 1");
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
        assert_eq!(encoded[0].length, 4, "UTF-16 length of 'café' should be 4, not 5 bytes");
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
        assert_eq!(ty, Some(1), "Nested Ambiguous wrapping Noun should still classify as type 1 (TYPE)");
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
}
