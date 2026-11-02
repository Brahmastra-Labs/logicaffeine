use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use lsp_types::{SemanticToken, SemanticTokenModifier, SemanticTokenType, SemanticTokensLegend};

use logicaffeine_language::token::{BlockType, RawKind, Span, Token, TokenType};

use crate::document::DocumentState;
use crate::index::DefinitionKind;
use crate::line_index::LineIndex;

/// Our semantic token types, registered with the client.
/// The legend is APPEND-ONLY — indices are a wire contract with client themes
/// (pinned by `tests/locks.rs`).
pub const TOKEN_TYPES: &[SemanticTokenType] = &[
    SemanticTokenType::KEYWORD,     // 0
    SemanticTokenType::TYPE,        // 1
    SemanticTokenType::FUNCTION,    // 2
    SemanticTokenType::VARIABLE,    // 3
    SemanticTokenType::STRING,      // 4
    SemanticTokenType::NUMBER,      // 5
    SemanticTokenType::OPERATOR,    // 6
    SemanticTokenType::NAMESPACE,   // 7
    SemanticTokenType::MODIFIER,    // 8
    SemanticTokenType::PROPERTY,    // 9
    SemanticTokenType::COMMENT,     // 10
    SemanticTokenType::PARAMETER,   // 11
    SemanticTokenType::ENUM_MEMBER, // 12
    // A template's element names and plain attribute names. No standard type exists for either;
    // the VS Code manifest maps both onto the TextMate scopes an HTML grammar uses, so a theme
    // paints them as it paints HTML.
    SemanticTokenType::new("tag"),       // 13
    SemanticTokenType::new("attribute"), // 14
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
pub const TYPE_TAG: u32 = 13;
pub const TYPE_ATTRIBUTE: u32 = 14;

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
/// interior re-lexes and resolves against the document's own index — and a
/// `View:` / `Head:` body, which is painted by its own grammar
/// ([`paint_template`]).
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
        if let TokenType::RawBlock { kind, .. } = token.kind {
            if matches!(kind, RawKind::View | RawKind::Head) {
                paint_template(doc, overlay, token.span, &mut spans);
            }
            continue;
        }
        let (token_type, modifiers) = overlay.classify(token, doc);
        let Some(token_type) = token_type else {
            continue;
        };
        if matches!(token.kind, TokenType::InterpolatedString(_)) && token_type == TYPE_STRING {
            expand_interpolation(doc, overlay, token, &mut spans);
        } else {
            spans.push((token.span, token_type, modifiers));
        }
    }
    spans
}

/// Paint a `View:` / `Head:` body by the language it is written in.
///
/// The body is HTML whose `{…}` groups are template tags and LOGOS holes, so it is painted as
/// that: an element's name as a `tag` (a component's as the `type` it names), an attribute's name
/// as an `attribute` (a component's as the `parameter` it binds, a directive as the `keyword` the
/// framework reads, an event modifier as a `modifier`), a static value as a `string`, a tag's
/// phrase and clause words as `keyword`s, and every expression through the resolution the rest of
/// the file gets. Text and markup punctuation carry no token: the editor's HTML grammar paints
/// them, and a semantic token over them would replace its colouring.
///
/// A `Style:` body gets no tokens for the same reason — the CSS grammar paints it and the server
/// knows nothing about CSS it could add.
///
/// The token's span is the body's place in the file with its indentation intact, and indentation
/// is whitespace to HTML, so tokenizing that slice yields spans that are file offsets once shifted
/// by its start. A body that does not tokenize (a tag half-typed) gets no tokens.
fn paint_template(
    doc: &DocumentState,
    overlay: &ResolutionOverlay,
    span: Span,
    out: &mut Vec<(Span, u32, u32)>,
) {
    use logicaffeine_language::lgx::html::{self, HtmlToken};
    let Some(body) = doc.source.get(span.start..span.end) else {
        return;
    };
    let Ok(tokens) = html::tokenize(body) else {
        return;
    };
    let base = span.start;
    for token in &tokens {
        match token {
            HtmlToken::StartTag {
                name,
                attrs,
                span: at,
                ..
            } => {
                let component = html::is_component_name(name);
                paint_tag_name(name, base + at.start + 1, out);
                for attr in attrs {
                    paint_attribute(doc, overlay, body, base, attr, component, out);
                }
            }
            HtmlToken::EndTag { name, span: at } => paint_tag_name(name, base + at.start + 2, out),
            HtmlToken::Brace { inner, span: at } => {
                paint_brace(doc, overlay, inner, base + at.start, out)
            }
            HtmlToken::Text { .. } => {}
        }
    }
}

/// An element's name: a component is the type it names, anything else is an HTML tag.
fn paint_tag_name(name: &str, at: usize, out: &mut Vec<(Span, u32, u32)>) {
    let kind = if logicaffeine_language::lgx::html::is_component_name(name) {
        TYPE_TYPE
    } else {
        TYPE_TAG
    };
    out.push((Span::new(at, at + name.len()), kind, 0));
}

/// One attribute of a start tag, name and value. `body` is the template text `attr`'s span indexes
/// and `base` its offset in the file.
fn paint_attribute(
    doc: &DocumentState,
    overlay: &ResolutionOverlay,
    body: &str,
    base: usize,
    attr: &logicaffeine_language::lgx::html::HtmlAttr,
    on_component: bool,
    out: &mut Vec<(Span, u32, u32)>,
) {
    use logicaffeine_language::lgx::html::{AttrPart, AttrValue};
    let name = attr.name.as_str();
    let at = base + attr.span.start;
    if let Some(directive) = name.strip_prefix("on-") {
        let mut pieces = directive.split('.');
        let event = pieces.next().unwrap_or("");
        let mut cursor = at + "on-".len() + event.len();
        out.push((Span::new(at, cursor), TYPE_KEYWORD, 0));
        for modifier in pieces {
            let end = cursor + 1 + modifier.len();
            out.push((Span::new(cursor, end), TYPE_MODIFIER, 0));
            cursor = end;
        }
    } else if let Some(class) = name.strip_prefix("class:") {
        let split = at + "class:".len();
        out.push((Span::new(at, split), TYPE_KEYWORD, 0));
        out.push((Span::new(split, split + class.len()), TYPE_ATTRIBUTE, 0));
    } else if name == "bind" {
        out.push((Span::new(at, at + name.len()), TYPE_KEYWORD, 0));
    } else {
        let kind = if on_component {
            TYPE_PARAMETER
        } else {
            TYPE_ATTRIBUTE
        };
        out.push((Span::new(at, at + name.len()), kind, 0));
    }

    let Some(value) = &attr.value else { return };
    // The value begins after the name, the `=` and any whitespace around it.
    let after_name = attr.span.start + name.len();
    let value_start = body[after_name..attr.span.end]
        .find(|c: char| !c.is_whitespace() && c != '=')
        .map_or(attr.span.end, |k| after_name + k);
    let mut cursor = value_start;
    if let AttrValue::Parts(parts) = value {
        for part in parts {
            let AttrPart::Brace { inner, span } = part else {
                continue;
            };
            if span.start > cursor {
                out.push((Span::new(base + cursor, base + span.start), TYPE_STRING, 0));
            }
            paint_brace(doc, overlay, inner, base + span.start, out);
            cursor = span.end;
        }
    }
    if attr.span.end > cursor {
        out.push((
            Span::new(base + cursor, base + attr.span.end),
            TYPE_STRING,
            0,
        ));
    }
}

/// One `{…}` group whose `{` sits at file offset `open`: a comment, a template tag (its phrase a
/// keyword, the rest code), or a hole (code, then each filter a function applied to it).
fn paint_brace(
    doc: &DocumentState,
    overlay: &ResolutionOverlay,
    inner: &str,
    open: usize,
    out: &mut Vec<(Span, u32, u32)>,
) {
    let close = open + 1 + inner.len();
    if crate::template::is_comment(inner) {
        out.push((Span::new(open, close + 1), TYPE_COMMENT, 0));
        return;
    }
    out.push((Span::new(open, open + 1), TYPE_OPERATOR, 0));
    let at = open + 1;
    match crate::template::tag_phrase(inner) {
        Some((phrase, len)) => {
            out.push((
                Span::new(at + len - phrase.len(), at + len),
                TYPE_KEYWORD,
                0,
            ));
            let rest = &inner[len..];
            if phrase == "for each" {
                paint_with_clause_words(
                    doc,
                    overlay,
                    rest,
                    at + len,
                    crate::template::FOR_EACH_CLAUSES,
                    out,
                );
            } else {
                paint_fragment(doc, overlay, rest, at + len, out);
            }
        }
        None => {
            let pipes = crate::template::filter_pipes(inner);
            let mut segment_start = 0;
            for (k, &pipe) in pipes
                .iter()
                .chain(std::iter::once(&inner.len()))
                .enumerate()
            {
                let segment = &inner[segment_start..pipe];
                if k == 0 {
                    paint_fragment(doc, overlay, segment, at + segment_start, out);
                } else {
                    // `| name args…` — the name is the function the hole's value is passed to.
                    let trimmed = segment.trim_start();
                    let name_at = segment_start + (segment.len() - trimmed.len());
                    let name_len = trimmed
                        .find(|c: char| !c.is_alphanumeric() && c != '_')
                        .unwrap_or(trimmed.len());
                    let modifiers = if prelude_names().contains(&trimmed[..name_len]) {
                        MOD_DEFAULT_LIBRARY
                    } else {
                        0
                    };
                    out.push((
                        Span::new(at + name_at, at + name_at + name_len),
                        TYPE_FUNCTION,
                        modifiers,
                    ));
                    let args_at = name_at + name_len;
                    paint_fragment(doc, overlay, &inner[args_at..pipe], at + args_at, out);
                }
                if pipe < inner.len() {
                    out.push((Span::new(at + pipe, at + pipe + 1), TYPE_OPERATOR, 0));
                }
                segment_start = pipe + 1;
            }
        }
    }
    out.push((Span::new(close, close + 1), TYPE_OPERATOR, 0));
}

/// Paint `text` (at file offset `at`) as code, with every whole-word occurrence of `clauses`
/// outside a string painted as a keyword — the `in`, `keyed by` and `at index` of a
/// `{for each …:}` tag, which the expression lexer would read as ordinary words.
fn paint_with_clause_words(
    doc: &DocumentState,
    overlay: &ResolutionOverlay,
    text: &str,
    at: usize,
    clauses: &[&str],
    out: &mut Vec<(Span, u32, u32)>,
) {
    let mut words: Vec<(usize, usize)> = Vec::new();
    let mut in_string = false;
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < text.len() {
        if bytes[i] == b'"' {
            in_string = !in_string;
        } else if !in_string {
            let boundary_before =
                i == 0 || !(bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_');
            if boundary_before {
                if let Some(word) = clauses.iter().find(|w| {
                    text[i..].starts_with(**w)
                        && text[i + w.len()..]
                            .chars()
                            .next()
                            .is_none_or(|c| !c.is_alphanumeric() && c != '_')
                }) {
                    words.push((i, i + word.len()));
                    i += word.len();
                    continue;
                }
            }
        }
        i += 1;
    }
    let mut cursor = 0;
    for (start, end) in words {
        paint_fragment(doc, overlay, &text[cursor..start], at + cursor, out);
        out.push((Span::new(at + start, at + end), TYPE_KEYWORD, 0));
        cursor = end;
    }
    paint_fragment(doc, overlay, &text[cursor..], at + cursor, out);
}

/// Split one interpolated-string token into string segments, `{`/`}`
/// operators, and re-lexed interior code (`{{` escapes stay string).
fn expand_interpolation(
    doc: &DocumentState,
    overlay: &ResolutionOverlay,
    token: &Token,
    out: &mut Vec<(Span, u32, u32)>,
) {
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
                paint_fragment(doc, overlay, &src[i + 1..close], base + i + 1, out);
                out.push((Span::new(base + close, base + close + 1), TYPE_OPERATOR, 0));
                segment_start = close + 1;
                i = close + 1;
            }
            _ => i += 1,
        }
    }
    if segment_start < src.len() {
        out.push((
            Span::new(base + segment_start, base + src.len()),
            TYPE_STRING,
            0,
        ));
    }
}

/// Paint an embedded code fragment — an interpolation interior, a template hole or a tag's
/// expression — at file offset `offset`: lex it with the REAL lexer, classify by part of speech,
/// and upgrade identifier-like tokens through resolution — a `{name}` paints exactly like `name`
/// would outside the string.
///
/// A reference the index already resolved at this exact file position (every name a template
/// mentions is one) is taken as resolved; any other name falls back to its definitions by name.
fn paint_fragment(
    doc: &DocumentState,
    overlay: &ResolutionOverlay,
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
        let Some(mut token_type) = token_type else {
            continue;
        };
        let resolved = overlay
            .ref_at
            .get(&(offset + token.span.start))
            .map(|&i| &doc.symbol_index.definitions[i])
            // The same surface-form resolution the overlay applies (verbs and
            // ambiguous words resolve by what they LOOK like, not their lemma).
            .or_else(|| {
                crate::index::resolve_token_name(&token, &interner)
                    .and_then(|name| doc.symbol_index.definitions_of(name).first().copied())
            });
        if let Some(def) = resolved {
            token_type = semantic_type_for(&def.kind);
            modifiers = readonly_bit(def);
        }
        out.push((
            Span::new(offset + token.span.start, offset + token.span.end),
            token_type,
            modifiers,
        ));
    }
}

/// One protocol-legal (single-line) semantic token in absolute coordinates,
/// before delta-encoding.
struct AbsToken {
    line: u32,
    character: u32,
    length: u32,
    token_type: u32,
    modifiers: u32,
}

/// Split a byte span into one `AbsToken` per source line it covers (each a
/// single line, terminator excluded) and push them into `out`. A single-line
/// span (the common case) yields exactly one; a multi-line `"""…"""` string
/// yields one token per line — the protocol forbids a token that crosses a
/// newline, and an over-long `length` desyncs every token after it.
fn push_span_pieces(
    span: &Span,
    token_type: u32,
    modifiers: u32,
    li: &LineIndex,
    out: &mut Vec<AbsToken>,
) {
    let start = li.position(span.start);
    let end = li.position(span.end);
    let mut emit = |line: u32, character: u32, length: u32| {
        if length > 0 {
            out.push(AbsToken {
                line,
                character,
                length,
                token_type,
                modifiers,
            });
        }
    };
    if start.line == end.line {
        emit(
            start.line,
            start.character,
            end.character.saturating_sub(start.character),
        );
        return;
    }
    // First covered line: from the span start to that line's content end.
    let first_end = li.line_end_offset(start.line as usize);
    emit(
        start.line,
        start.character,
        li.utf16_length(span.start, first_end),
    );
    // Whole intermediate lines.
    for line in (start.line as usize + 1)..(end.line as usize) {
        let ls = li.line_start_offset(line);
        let le = li.line_end_offset(line);
        emit(line as u32, 0, li.utf16_length(ls, le));
    }
    // Last covered line: from its start to the span end.
    emit(end.line, 0, end.character);
}

/// Delta-encode absolute tokens (sorted by position) into the LSP wire form.
/// Sorting + saturating deltas make the encoder total: the wire contract
/// (monotonic, non-negative deltas) holds even if the input spans are out of
/// order — one negative delta would otherwise wrap and corrupt the whole stream.
fn delta_encode(mut abs: Vec<AbsToken>) -> Vec<SemanticToken> {
    abs.sort_by_key(|t| (t.line, t.character));
    let mut result = Vec::with_capacity(abs.len());
    let mut prev_line = 0u32;
    let mut prev_start = 0u32;
    for t in abs {
        let delta_line = t.line.saturating_sub(prev_line);
        let delta_start = if delta_line == 0 {
            t.character.saturating_sub(prev_start)
        } else {
            t.character
        };
        result.push(SemanticToken {
            delta_line,
            delta_start,
            length: t.length,
            token_type: t.token_type,
            token_modifiers_bitset: t.modifiers,
        });
        prev_line = t.line;
        prev_start = t.character;
    }
    result
}

/// Delta-encode classified spans into the LSP wire form.
fn encode_spans(spans: &[(Span, u32, u32)], line_index: &LineIndex) -> Vec<SemanticToken> {
    let mut abs = Vec::with_capacity(spans.len());
    for (span, token_type, modifiers) in spans {
        push_span_pieces(span, *token_type, *modifiers, line_index, &mut abs);
    }
    delta_encode(abs)
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
            .filter(|(_, block_type, _)| matches!(block_type, BlockType::Note | BlockType::Example))
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
        } else if matches!(doc.interner.resolve(token.lexeme), "mutable" | "mut") {
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
        DefinitionKind::Component => TYPE_TYPE,
        DefinitionKind::Action => TYPE_FUNCTION,
        DefinitionKind::StateField => TYPE_PROPERTY,
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
                        kind if is_identifier_like(kind) => last_ident = Some(tokens[j].span.start),
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
                if let Some(next) = tokens[i + 1..].iter().find(|t| is_identifier_like(&t.kind)) {
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
            kind if seen_keyword && is_identifier_like(kind) => return Some(token.span.start),
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
) -> Vec<lsp_types::SemanticTokensEdit> {
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

    vec![lsp_types::SemanticTokensEdit {
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
    let mut abs = Vec::with_capacity(tokens.len());
    for token in tokens {
        let (token_type, modifiers) = classify(token);
        let Some(token_type) = token_type else {
            continue;
        }; // Skip tokens we don't highlight
        push_span_pieces(&token.span, token_type, modifiers, line_index, &mut abs);
    }
    delta_encode(abs)
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
#[path = "../../logicaffeine_tests/unit/logicaffeine_lsp/semantic_tokens/tests.rs"]
mod tests;
