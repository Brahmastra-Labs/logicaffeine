use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use tower_lsp::lsp_types::{
    SemanticToken, SemanticTokenType, SemanticTokensLegend, SemanticTokenModifier,
};

use logicaffeine_language::token::{BlockType, Span, Token, TokenType};

use crate::document::DocumentState;
use crate::index::DefinitionKind;
use crate::line_index::LineIndex;

/// Our semantic token types, registered with the client.
/// The legend is APPEND-ONLY — indices are a wire contract with client themes
/// (pinned by `tests/locks.rs`).
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
    SemanticTokenModifier::DECLARATION,     // bit 0
    SemanticTokenModifier::READONLY,        // bit 1
    SemanticTokenModifier::MODIFICATION,    // bit 2 — write sites (Set/Increase/Push targets)
    SemanticTokenModifier::DEFAULT_LIBRARY, // bit 3 — stdlib prelude names
];

pub const TYPE_KEYWORD: u32 = 0;
pub const TYPE_TYPE: u32 = 1;
pub const TYPE_FUNCTION: u32 = 2;
pub const TYPE_VARIABLE: u32 = 3;
pub const TYPE_STRING: u32 = 4;
pub const TYPE_NUMBER: u32 = 5;
pub const TYPE_OPERATOR: u32 = 6;
pub const TYPE_NAMESPACE: u32 = 7;
pub const TYPE_MODIFIER: u32 = 8;
pub const TYPE_PROPERTY: u32 = 9;
pub const TYPE_COMMENT: u32 = 10;
pub const TYPE_PARAMETER: u32 = 11;
pub const TYPE_ENUM_MEMBER: u32 = 12;

pub const MOD_DECLARATION: u32 = 1 << 0;
pub const MOD_READONLY: u32 = 1 << 1;
pub const MOD_MODIFICATION: u32 = 1 << 2;
pub const MOD_DEFAULT_LIBRARY: u32 = 1 << 3;

pub fn legend() -> SemanticTokensLegend {
    SemanticTokensLegend {
        token_types: TOKEN_TYPES.to_vec(),
        token_modifiers: TOKEN_MODIFIERS.to_vec(),
    }
}

/// Encode a document's tokens with the full resolution overlay.
///
/// The base layer is part-of-speech classification ([`classify_token`]);
/// the overlay upgrades identifiers through the `SymbolIndex` — a name that
/// resolves to a parameter IS a parameter, at declaration and at every
/// reference — and adds the modifier facts: DECLARATION only where
/// `token.span == definition.span`, READONLY for immutable `Let`s,
/// MODIFICATION on write sites, DEFAULT_LIBRARY on stdlib prelude names.
/// Prose inside `## Note`/`## Example` blocks recedes to COMMENT.
pub fn encode_document_tokens(doc: &DocumentState) -> Vec<SemanticToken> {
    let overlay = ResolutionOverlay::build(doc);
    let spans = paint_spans(doc, &overlay, None);
    encode_spans(&spans, &doc.line_index)
}

/// The classified paint spans for the document: one per token, EXCEPT
/// interpolated strings, whose `{expr}` interiors expand into real code
/// spans — string segments stay strings, braces paint as operators, and the
/// interior re-lexes and resolves against the document's own index.
fn paint_spans(
    doc: &DocumentState,
    overlay: &ResolutionOverlay,
    range: Option<(usize, usize)>,
) -> Vec<(Span, u32, u32)> {
    let mut spans = Vec::new();
    for token in &doc.tokens {
        if let Some((start, end)) = range {
            if token.span.end <= start || token.span.start >= end {
                continue;
            }
        }
        let (token_type, modifiers) = overlay.classify(token, doc);
        let Some(token_type) = token_type else { continue };
        if matches!(token.kind, TokenType::InterpolatedString(_)) && token_type == TYPE_STRING {
            expand_interpolation(doc, token, &mut spans);
        } else {
            spans.push((token.span, token_type, modifiers));
        }
    }
    spans
}

/// Split one interpolated-string token into string segments, `{`/`}`
/// operators, and re-lexed interior code (`{{` escapes stay string).
fn expand_interpolation(doc: &DocumentState, token: &Token, out: &mut Vec<(Span, u32, u32)>) {
    let Some(src) = doc.source.get(token.span.start..token.span.end) else {
        out.push((token.span, TYPE_STRING, 0));
        return;
    };
    let base = token.span.start;
    let bytes = src.as_bytes();
    let mut segment_start = 0usize;
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'{' if bytes.get(i + 1) == Some(&b'{') => i += 2,
            b'{' => {
                let Some(close_rel) = src[i + 1..].find('}') else {
                    i += 1;
                    continue;
                };
                let close = i + 1 + close_rel;
                if i > segment_start {
                    out.push((Span::new(base + segment_start, base + i), TYPE_STRING, 0));
                }
                out.push((Span::new(base + i, base + i + 1), TYPE_OPERATOR, 0));
                paint_fragment(doc, &src[i + 1..close], base + i + 1, out);
                out.push((Span::new(base + close, base + close + 1), TYPE_OPERATOR, 0));
                segment_start = close + 1;
                i = close + 1;
            }
            _ => i += 1,
        }
    }
    if segment_start < src.len() {
        out.push((Span::new(base + segment_start, base + src.len()), TYPE_STRING, 0));
    }
}

/// Paint an interpolation interior: lex the fragment with the REAL lexer,
/// classify by part of speech, and upgrade identifier-like tokens through
/// the document's definitions — a `{name}` paints exactly like `name` would
/// outside the string.
fn paint_fragment(
    doc: &DocumentState,
    fragment: &str,
    offset: usize,
    out: &mut Vec<(Span, u32, u32)>,
) {
    let mut interner = logicaffeine_base::Interner::new();
    let mut lexer = logicaffeine_language::lexer::Lexer::new(fragment, &mut interner);
    let tokens = lexer.tokenize();
    for token in tokens {
        if token.span.end > fragment.len() || token.span.start >= token.span.end {
            continue;
        }
        let (token_type, mut modifiers) = classify_token(&token.kind);
        let Some(mut token_type) = token_type else { continue };
        // The same surface-form resolution the overlay applies (verbs and
        // ambiguous words resolve by what they LOOK like, not their lemma).
        if let Some(name) = crate::index::resolve_token_name(&token, &interner) {
            if let Some(def) = doc.symbol_index.definitions_of(name).first() {
                token_type = semantic_type_for(&def.kind);
                modifiers = readonly_bit(def);
            }
        }
        out.push((
            Span::new(offset + token.span.start, offset + token.span.end),
            token_type,
            modifiers,
        ));
    }
}

/// Delta-encode classified spans into the LSP wire form.
fn encode_spans(spans: &[(Span, u32, u32)], line_index: &LineIndex) -> Vec<SemanticToken> {
    let mut result = Vec::with_capacity(spans.len());
    let mut prev_line = 0u32;
    let mut prev_start = 0u32;
    for (span, token_type, modifiers) in spans {
        let pos = line_index.position(span.start);
        let length = line_index.utf16_length(span.start, span.end);
        if length == 0 {
            continue;
        }
        let delta_line = pos.line - prev_line;
        let delta_start =
            if delta_line == 0 { pos.character - prev_start } else { pos.character };
        result.push(SemanticToken {
            delta_line,
            delta_start,
            length,
            token_type: *token_type,
            token_modifiers_bitset: *modifiers,
        });
        prev_line = pos.line;
        prev_start = pos.character;
    }
    result
}

struct ResolutionOverlay {
    /// Definition-site span starts → definition index.
    decl_at: HashMap<usize, usize>,
    /// Resolved reference span starts → definition index.
    ref_at: HashMap<usize, usize>,
    /// Span starts of write targets (Set/Increase/Push-to/…).
    mutation_at: HashSet<usize>,
    /// `## Note` / `## Example` block extents.
    prose: Vec<Span>,
    /// Every registered type name (primitives included): `Int` in an
    /// annotation is a TYPE even though no definition site exists for it.
    type_names: HashSet<String>,
}

impl ResolutionOverlay {
    fn build(doc: &DocumentState) -> Self {
        let index = &doc.symbol_index;

        let mut decl_at = HashMap::new();
        for (i, def) in index.definitions.iter().enumerate() {
            if def.span != Span::default() {
                decl_at.insert(def.span.start, i);
            }
        }

        let mut ref_at = HashMap::new();
        for reference in &index.references {
            if let Some(def_idx) = reference.definition_idx {
                ref_at.insert(reference.span.start, def_idx);
            }
        }

        let prose = index
            .block_spans
            .iter()
            .filter(|(_, block_type, _)| {
                matches!(block_type, BlockType::Note | BlockType::Example)
            })
            .map(|(_, _, span)| *span)
            .collect();

        let type_names = doc
            .type_registry
            .iter_types()
            .map(|(sym, _)| doc.interner.resolve(*sym).to_string())
            .collect();

        ResolutionOverlay {
            decl_at,
            ref_at,
            mutation_at: find_mutation_targets(&doc.tokens),
            prose,
            type_names,
        }
    }

    fn classify(&self, token: &Token, doc: &DocumentState) -> (Option<u32>, u32) {
        // Headers keep their identity even when they open a prose block.
        if matches!(token.kind, TokenType::BlockHeader { .. }) {
            return classify_token(&token.kind);
        }

        // Documentation prose recedes: everything inside Note/Example is comment.
        let start = token.span.start;
        if self.prose.iter().any(|p| start > p.start && start < p.end) {
            return (Some(TYPE_COMMENT), 0);
        }

        let (mut token_type, mut modifiers) = classify_token(&token.kind);

        if let Some(&def_idx) = self.decl_at.get(&start) {
            let def = &doc.symbol_index.definitions[def_idx];
            token_type = Some(semantic_type_for(&def.kind));
            modifiers = MOD_DECLARATION | readonly_bit(def);
        } else if let Some(&def_idx) = self.ref_at.get(&start) {
            let def = &doc.symbol_index.definitions[def_idx];
            token_type = Some(semantic_type_for(&def.kind));
            modifiers = readonly_bit(def);
        } else if is_identifier_like(&token.kind)
            && self.type_names.contains(doc.interner.resolve(token.lexeme))
        {
            // A registered type name with no definition site (primitives:
            // `Int`, `Text`, …) is a TYPE wherever it appears.
            token_type = Some(TYPE_TYPE);
            modifiers &= !MOD_DECLARATION;
        } else if matches!(
            doc.interner.resolve(token.lexeme),
            "mutable" | "mut"
        ) {
            // The mutability marker lexes as an English noun; it IS a
            // storage modifier (the grammar layer agrees).
            token_type = Some(TYPE_MODIFIER);
            modifiers = 0;
        } else if matches!(token.kind, TokenType::ProperName(_)) {
            // Base layer marks ProperName as a declaration; without a resolved
            // definition at this exact span it is a reference, not a declaration.
            modifiers &= !MOD_DECLARATION;
        }

        if self.mutation_at.contains(&start) {
            modifiers |= MOD_MODIFICATION;
        }
        if is_identifier_like(&token.kind)
            && prelude_names().contains(doc.interner.resolve(token.lexeme))
        {
            modifiers |= MOD_DEFAULT_LIBRARY;
        }

        (token_type, modifiers)
    }
}

fn readonly_bit(def: &crate::index::Definition) -> u32 {
    match def.mutable {
        Some(false) => MOD_READONLY,
        _ => 0,
    }
}

/// The semantic token type an identifier gets once resolution names its kind.
/// Wildcard-free: a new `DefinitionKind` must decide its color here.
pub fn semantic_type_for(kind: &DefinitionKind) -> u32 {
    match kind {
        DefinitionKind::Variable => TYPE_VARIABLE,
        DefinitionKind::Function => TYPE_FUNCTION,
        DefinitionKind::Struct => TYPE_TYPE,
        DefinitionKind::Enum => TYPE_TYPE,
        DefinitionKind::Field => TYPE_PROPERTY,
        DefinitionKind::Parameter => TYPE_PARAMETER,
        DefinitionKind::Block => TYPE_NAMESPACE,
        DefinitionKind::Variant => TYPE_ENUM_MEMBER,
        DefinitionKind::Theorem => TYPE_NAMESPACE,
    }
}

fn is_identifier_like(kind: &TokenType) -> bool {
    matches!(
        kind,
        TokenType::Identifier
            | TokenType::ProperName(_)
            | TokenType::Noun(_)
            | TokenType::Verb { .. }
            | TokenType::Adjective(_)
    )
}

/// The stdlib prelude vocabulary (`md5`, `uuidV3`, `flush`, …), computed once.
fn prelude_names() -> &'static HashSet<String> {
    static NAMES: OnceLock<HashSet<String>> = OnceLock::new();
    NAMES.get_or_init(|| {
        logicaffeine_compile::loader::prelude_vocabulary()
            .into_iter()
            .collect()
    })
}

/// Span starts of tokens that a statement WRITES:
/// `Set <x> to …` / `Set … at <k> to …`-style targets take the identifier
/// nearest the write (the last one before `to`, or before `at` when a keyed
/// write follows), `Increase`/`Decrease <x>`, `Push`/`Add`/`Append … to <x>`,
/// `Remove`/`Pop … from <x>`.
pub fn find_mutation_targets(tokens: &[Token]) -> HashSet<usize> {
    let mut targets = HashSet::new();
    let mut i = 0;
    while i < tokens.len() {
        match &tokens[i].kind {
            TokenType::Set => {
                let mut last_ident: Option<usize> = None;
                let mut before_at: Option<usize> = None;
                let mut j = i + 1;
                while j < tokens.len() {
                    match &tokens[j].kind {
                        TokenType::To => break,
                        TokenType::At => before_at = last_ident,
                        TokenType::Period => break,
                        kind if is_identifier_like(kind) => {
                            last_ident = Some(tokens[j].span.start)
                        }
                        _ => {}
                    }
                    j += 1;
                }
                if let Some(start) = before_at.or(last_ident) {
                    targets.insert(start);
                }
                i = j;
            }
            TokenType::Increase | TokenType::Decrease => {
                if let Some(next) = tokens[i + 1..]
                    .iter()
                    .find(|t| is_identifier_like(&t.kind))
                {
                    targets.insert(next.span.start);
                }
                i += 1;
            }
            TokenType::Push | TokenType::Add | TokenType::Append => {
                // The collection after `to` is what changes.
                if let Some(start) = ident_after_keyword(tokens, i, &TokenType::To) {
                    targets.insert(start);
                }
                i += 1;
            }
            TokenType::Pop | TokenType::Remove => {
                if let Some(start) = ident_after_keyword(tokens, i, &TokenType::From) {
                    targets.insert(start);
                }
                i += 1;
            }
            _ => i += 1,
        }
    }
    targets
}

/// The first identifier after `keyword` within the current sentence.
fn ident_after_keyword(tokens: &[Token], from: usize, keyword: &TokenType) -> Option<usize> {
    let mut seen_keyword = false;
    for token in &tokens[from + 1..] {
        match &token.kind {
            TokenType::Period => return None,
            kind if kind == keyword => seen_keyword = true,
            kind if seen_keyword && is_identifier_like(kind) => {
                return Some(token.span.start)
            }
            _ => {}
        }
    }
    None
}

/// Convert a token stream to LSP semantic tokens (delta-encoded) using the
/// base part-of-speech layer only. [`encode_document_tokens`] is the
/// resolution-aware entry point.
pub fn encode_tokens(tokens: &[Token], line_index: &LineIndex) -> Vec<SemanticToken> {
    encode_with(tokens, line_index, |token| classify_token(&token.kind))
}

/// Resolution-aware encoding restricted to tokens overlapping a byte range.
/// The first emitted token stays absolute from the document start, exactly as
/// the LSP range response requires.
pub fn encode_document_tokens_in_range(
    doc: &DocumentState,
    start_offset: usize,
    end_offset: usize,
) -> Vec<SemanticToken> {
    let overlay = ResolutionOverlay::build(doc);
    let spans = paint_spans(doc, &overlay, Some((start_offset, end_offset)));
    encode_spans(&spans, &doc.line_index)
}

/// The minimal single-splice edit list turning `prev` into `next`.
///
/// Deltas are compared raw: a positional shift changes the first token after
/// the edited region, so prefix/suffix agreement on the encoded stream is
/// exactly the correct splice boundary. Offsets are in INTEGERS (5 per
/// token), as the LSP delta contract requires.
pub fn semantic_token_edits(
    prev: &[SemanticToken],
    next: &[SemanticToken],
) -> Vec<tower_lsp::lsp_types::SemanticTokensEdit> {
    let prefix = prev
        .iter()
        .zip(next.iter())
        .take_while(|(a, b)| a == b)
        .count();
    if prefix == prev.len() && prefix == next.len() {
        return Vec::new();
    }
    let max_suffix = prev.len().min(next.len()) - prefix;
    let suffix = prev
        .iter()
        .rev()
        .zip(next.iter().rev())
        .take_while(|(a, b)| a == b)
        .count()
        .min(max_suffix);

    vec![tower_lsp::lsp_types::SemanticTokensEdit {
        start: (prefix * 5) as u32,
        delete_count: ((prev.len() - prefix - suffix) * 5) as u32,
        data: Some(next[prefix..next.len() - suffix].to_vec()),
    }]
}

fn encode_with(
    tokens: &[Token],
    line_index: &LineIndex,
    classify: impl Fn(&Token) -> (Option<u32>, u32),
) -> Vec<SemanticToken> {
    let mut result = Vec::with_capacity(tokens.len());
    let mut prev_line = 0u32;
    let mut prev_start = 0u32;

    for token in tokens {
        let (token_type, modifiers) = classify(token);
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
/// Delegates the CLASS decision to the language crate's
/// [`logicaffeine_language::token_class`] — the single classification truth
/// every surface (this server, the terminal REPL) shares — and adds the
/// legend-level concerns: index mapping and the ProperName declaration bit.
/// `tests/locks.rs` pins every classification inside the advertised legend.
pub fn classify_token(kind: &TokenType) -> (Option<u32>, u32) {
    use logicaffeine_language::token_class::{classify, TokenClass};
    let class = classify(kind);
    let index = class.map(|c| match c {
        TokenClass::Keyword => TYPE_KEYWORD,
        TokenClass::Type => TYPE_TYPE,
        TokenClass::Function => TYPE_FUNCTION,
        TokenClass::Variable => TYPE_VARIABLE,
        TokenClass::String => TYPE_STRING,
        TokenClass::Number => TYPE_NUMBER,
        TokenClass::Operator => TYPE_OPERATOR,
        TokenClass::Namespace => TYPE_NAMESPACE,
        TokenClass::Modifier => TYPE_MODIFIER,
        TokenClass::Property => TYPE_PROPERTY,
        TokenClass::Comment => TYPE_COMMENT,
        TokenClass::Parameter => TYPE_PARAMETER,
        TokenClass::EnumMember => TYPE_ENUM_MEMBER,
    });
    let modifiers = if matches!(kind, TokenType::ProperName(_)) {
        MOD_DECLARATION
    } else {
        0
    };
    (index, modifiers)
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
