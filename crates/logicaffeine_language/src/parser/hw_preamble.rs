//! HW-Spec preamble parser.
//!
//! Consumes the sigil directives that appear at the top of a `## Hardware`
//! block and produces a typed [`HwPreamble`] record. The preamble parser owns
//! the preamble / property boundary — the first non-directive line ends the
//! preamble and [`HwPreambleResult::end_token`] points at it so the caller
//! (the phase-4 `parse_hw_spec` driver) can hand off to the property-sentence
//! parser.
//!
//! The HW-Spec DSL is emitted through this module from day one; no source-
//! level desugaring to LOGOS statements happens. Malformed sigils fail loudly
//! with a span, so a typo like `signls:` produces a parse error rather than
//! being silently reclassified as property text.

use logicaffeine_base::Symbol;

use crate::ast::Expr;
use crate::error::{ParseError, ParseErrorKind};
use crate::parser::{ParseResult, Parser};
use crate::token::TokenType;

// ═══════════════════════════════════════════════════════════════════════════
// HW-Spec preamble IR
// ═══════════════════════════════════════════════════════════════════════════

/// Active edge for a declared clock signal. HW-Spec defaults to `Posedge`
/// when the sigil does not override it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClockEdge {
    Posedge,
    Negedge,
}

/// Role distinguishes the primary sampling clock from secondary domain
/// clocks declared through `clocks:` (plural).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClockRole {
    Primary,
    Secondary,
}

/// Reset polarity — drives `disable iff (!rst_n)` vs `disable iff (rst)`
/// at SVA lowering time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResetPolarity {
    ActiveLow,
    ActiveHigh,
}

/// HW-Spec signal type. Enum values are stored by index into the preamble's
/// own [`HwTypeRegistry`] so the preamble is self-contained — no external
/// type registry is mutated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignalType {
    Scalar,
    Bus { hi: i64, lo: i64 },
    Enum(EnumTypeId),
}

/// Handle returned by [`HwTypeRegistry::register_enum`]. Opaque; only the
/// preamble consumer (phase 4) should dereference it through the registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EnumTypeId(pub usize);

#[derive(Debug, Clone)]
pub struct EnumType {
    pub values: Vec<Symbol>,
}

/// Preamble-local type registry. Anonymous enum types declared inline as
/// `signal_name : enum {V1, V2}` are registered here rather than synthesised
/// into a `## Definition` block.
#[derive(Debug, Default, Clone)]
pub struct HwTypeRegistry {
    enums: Vec<EnumType>,
}

impl HwTypeRegistry {
    pub fn register_enum(&mut self, values: Vec<Symbol>) -> EnumTypeId {
        let id = EnumTypeId(self.enums.len());
        self.enums.push(EnumType { values });
        id
    }

    pub fn enum_values(&self, id: EnumTypeId) -> &[Symbol] {
        &self.enums[id.0].values
    }

    pub fn len(&self) -> usize {
        self.enums.len()
    }

    pub fn is_empty(&self) -> bool {
        self.enums.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClockDecl {
    pub name: Symbol,
    pub role: ClockRole,
    pub edge: ClockEdge,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResetDecl {
    pub name: Symbol,
    pub polarity: ResetPolarity,
    pub domain: Option<Symbol>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignalDecl {
    pub name: Symbol,
    pub ty: SignalType,
    pub domain: Option<Symbol>,
}

/// RHS of a `let` binding, parsed through [`Parser::parse_imperative_expr`]
/// which in HW context fires the bitwise / reduction / select / concat
/// operators introduced in phase 2.
#[derive(Debug, Clone)]
pub struct LetBinding<'a> {
    pub name: Symbol,
    pub rhs: &'a Expr<'a>,
    pub inferred_type: Option<SignalType>,
}

impl<'a> PartialEq for LetBinding<'a> {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
            && std::ptr::eq(self.rhs as *const _, other.rhs as *const _)
            && self.inferred_type == other.inferred_type
    }
}
impl<'a> Eq for LetBinding<'a> {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParameterDecl {
    pub name: Symbol,
    pub ty: SignalType,
    pub default: Option<Symbol>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterfaceField {
    pub name: Symbol,
    pub ty: SignalType,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterfaceDecl {
    pub name: Symbol,
    pub fields: Vec<InterfaceField>,
}

/// Canonical symbol table embedded in every [`HwPreamble`]. Holds the per-
/// signal metadata (type, bus width, clock domain, enum membership) that
/// downstream phases query instead of re-threading through side channels.
///
/// Phase 4 exposes a programmatic constructor
/// `HwSymbolTable::from_decls(&[HwSignalDecl])` so the Python-bridge callers
/// in `logicaffeine_compile` continue to work without going through the
/// preamble text path.
#[derive(Debug, Default, Clone)]
pub struct HwSymbolTable {
    entries: Vec<HwSymbolEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HwSymbolEntry {
    pub name: Symbol,
    pub ty: SignalType,
    pub clock_domain: Option<Symbol>,
}

impl HwSymbolTable {
    pub fn insert(&mut self, name: Symbol, ty: SignalType, clock_domain: Option<Symbol>) {
        self.entries.push(HwSymbolEntry { name, ty, clock_domain });
    }

    pub fn get(&self, name: Symbol) -> Option<&HwSymbolEntry> {
        self.entries.iter().find(|e| e.name == name)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &HwSymbolEntry> {
        self.entries.iter()
    }
}

/// Typed record produced by [`parse_hw_preamble`]. `Vec` fields live on the
/// heap (not arena-allocated) because the preamble is per-file metadata, not
/// a hot-path allocation. The `Expr` references inside `lets` are arena-bound
/// via the parser's `AstContext::imperative_exprs` arena.
#[derive(Debug, Default)]
pub struct HwPreamble<'a> {
    pub clocks: Vec<ClockDecl>,
    pub resets: Vec<ResetDecl>,
    pub signals: Vec<SignalDecl>,
    pub lets: Vec<LetBinding<'a>>,
    pub parameters: Vec<ParameterDecl>,
    pub interfaces: Vec<InterfaceDecl>,
    pub types: HwTypeRegistry,
    pub symbols: HwSymbolTable,
}

/// Return value of [`parse_hw_preamble`]. `end_token` indexes into the token
/// stream at the first line beyond the preamble; the caller (phase 4)
/// synthesises a `## Property` block header at that position before handing
/// the remainder to the declarative property-sentence parser.
#[derive(Debug)]
pub struct HwPreambleResult<'a> {
    pub preamble: HwPreamble<'a>,
    pub end_token: usize,
}

// ═══════════════════════════════════════════════════════════════════════════
// Entry point
// ═══════════════════════════════════════════════════════════════════════════

/// Parse a HW-Spec preamble out of the token stream held by `parser`.
///
/// The parser must already be positioned past the `## Hardware` block
/// header and should have its `hw_context` flag set (typically by
/// [`Parser::process_block_headers`] entering the Hardware block).
///
/// On success returns the populated [`HwPreamble`] and the index of the
/// first token beyond the preamble. On a malformed sigil, returns a
/// [`ParseError`] whose span points at the offending token.
pub fn parse_hw_preamble<'a, 'ctx, 'int>(
    parser: &mut Parser<'a, 'ctx, 'int>,
) -> ParseResult<HwPreambleResult<'a>> {
    let mut preamble = HwPreamble::default();

    loop {
        skip_structural(parser);

        if is_eof_or_next_block(parser) {
            break;
        }

        if let Some(kind) = peek_sigil_kind(parser) {
            match kind {
                SigilKind::Clock { plural: _ } => parse_clock_sigil(parser, &mut preamble)?,
                SigilKind::Reset { plural: _ } => parse_reset_sigil(parser, &mut preamble)?,
                SigilKind::Signals => parse_signals_sigil(parser, &mut preamble)?,
                SigilKind::Parameters => parse_parameters_sigil(parser, &mut preamble)?,
                SigilKind::Interfaces => parse_interfaces_sigil(parser, &mut preamble)?,
                SigilKind::Let => parse_let_binding(parser, &mut preamble)?,
                SigilKind::Malformed(raw) => {
                    return Err(ParseError {
                        kind: ParseErrorKind::Custom(format!(
                            "unknown HW-Spec preamble directive `{}:` — expected one of \
                             clock, clocks, reset, resets, signals, parameters, interfaces, let",
                            raw
                        )),
                        span: parser.tokens[parser.current].span,
                    });
                }
            }
        } else {
            // The current line does not begin with a recognised sigil, so
            // the preamble ends here. Downstream property-sentence parsing
            // resumes from this point.
            break;
        }
    }

    let end_token = parser.current;
    rebuild_symbol_table(&mut preamble);
    Ok(HwPreambleResult { preamble, end_token })
}

// ═══════════════════════════════════════════════════════════════════════════
// Sigil dispatcher
// ═══════════════════════════════════════════════════════════════════════════

enum SigilKind {
    Clock { plural: bool },
    Reset { plural: bool },
    Signals,
    Parameters,
    Interfaces,
    Let,
    /// `<word>:` where `<word>` is not one of the accepted sigils. Flagged so
    /// the main loop can surface a helpful error rather than silently
    /// stopping.
    Malformed(String),
}

fn peek_sigil_kind(parser: &Parser) -> Option<SigilKind> {
    // `let` is a dedicated token; no colon suffix.
    if parser.check(&TokenType::Let) {
        return Some(SigilKind::Let);
    }

    let word = read_word(parser, parser.current)?;

    // Sigils are always `<keyword>:`. If the next token is not a colon, this
    // isn't a sigil — the caller falls back to "end of preamble".
    let next = parser.tokens.get(parser.current + 1)?;
    if !matches!(next.kind, TokenType::Colon) {
        return None;
    }

    match word.as_str() {
        "clock" => Some(SigilKind::Clock { plural: false }),
        "clocks" => Some(SigilKind::Clock { plural: true }),
        "reset" => Some(SigilKind::Reset { plural: false }),
        "resets" => Some(SigilKind::Reset { plural: true }),
        "signals" => Some(SigilKind::Signals),
        "parameters" => Some(SigilKind::Parameters),
        "interfaces" => Some(SigilKind::Interfaces),
        // Words that look like a sigil (English noun followed by `:`) but
        // aren't one of the HW-Spec directives. Flag loudly rather than
        // bail silently — this catches typos like `signls:`.
        other => Some(SigilKind::Malformed(other.to_string())),
    }
}

/// Read the resolved lowercase word at `idx` if that token carries an
/// identifier-like POS classification. Returns `None` for punctuation and
/// numeric literals. Accepts the full set of lexical classes that a
/// HW-Spec signal or sigil name can land in — including Articles (for
/// single-letter names like `a` or `an`) and Ambiguous tokens whose
/// lexeme is still the raw identifier.
fn read_word(parser: &Parser, idx: usize) -> Option<String> {
    let token = parser.tokens.get(idx)?;
    if matches!(
        token.kind,
        TokenType::Noun(_)
            | TokenType::ProperName(_)
            | TokenType::Adjective(_)
            | TokenType::Identifier
            | TokenType::Verb { .. }
            | TokenType::Article(_)
            | TokenType::Ambiguous { .. }
    ) {
        Some(parser.interner.resolve(token.lexeme).to_lowercase())
    } else {
        None
    }
}

/// Returns `true` when the token stream at `parser.current` can no longer
/// contribute to an inner block body (signals / parameters / interfaces).
///
/// End markers: EOF, a new block header, an explicit `Dedent`, a `let`
/// keyword (start of a top-level let binding), or a known outer-level sigil
/// (`clock:`, `reset:`, etc.). Unknown words — including field-like
/// declarations such as `WIDTH : scalar` — are **not** end markers; they're
/// parsed as entries of the current block.
fn is_end_of_inner_block(parser: &Parser) -> bool {
    if is_eof_or_next_block(parser) {
        return true;
    }
    if parser.check(&TokenType::Dedent) {
        return true;
    }
    if parser.check(&TokenType::Let) {
        return true;
    }

    let Some(word) = read_word(parser, parser.current) else {
        return false;
    };
    let has_colon_after = parser
        .tokens
        .get(parser.current + 1)
        .map(|t| matches!(t.kind, TokenType::Colon))
        .unwrap_or(false);
    if !has_colon_after {
        return false;
    }

    matches!(
        word.as_str(),
        "clock" | "clocks" | "reset" | "resets" | "signals" | "parameters" | "interfaces"
    )
}

// ═══════════════════════════════════════════════════════════════════════════
// Per-sigil parsers
// ═══════════════════════════════════════════════════════════════════════════

fn parse_clock_sigil(parser: &mut Parser, preamble: &mut HwPreamble) -> ParseResult<()> {
    parser.current += 1; // consume sigil keyword
    expect_colon(parser, "clock")?;

    loop {
        let name = expect_bare_identifier(parser, "clock name")?;
        preamble.clocks.push(ClockDecl {
            name,
            role: if preamble.clocks.is_empty() {
                ClockRole::Primary
            } else {
                ClockRole::Secondary
            },
            edge: ClockEdge::Posedge,
        });
        if !consume_token(parser, &TokenType::Comma) {
            break;
        }
    }

    consume_line_terminator(parser);
    Ok(())
}

fn parse_reset_sigil(parser: &mut Parser, preamble: &mut HwPreamble) -> ParseResult<()> {
    parser.current += 1;
    expect_colon(parser, "reset")?;

    loop {
        let name = expect_bare_identifier(parser, "reset name")?;
        let mut polarity = ResetPolarity::ActiveHigh;

        if consume_token(parser, &TokenType::Comma) {
            expect_word(parser, "active")?;
            if consume_word(parser, "low") {
                polarity = ResetPolarity::ActiveLow;
            } else if consume_word(parser, "high") {
                polarity = ResetPolarity::ActiveHigh;
            } else {
                return err_here(parser, "expected `low` or `high` after `active`");
            }
        }

        preamble.resets.push(ResetDecl {
            name,
            polarity,
            domain: None,
        });

        if !consume_token(parser, &TokenType::Comma) {
            break;
        }
    }

    consume_line_terminator(parser);
    Ok(())
}

fn parse_signals_sigil(parser: &mut Parser, preamble: &mut HwPreamble) -> ParseResult<()> {
    parser.current += 1; // consume "signals"
    expect_colon(parser, "signals")?;
    consume_line_terminator(parser);

    // Optional indented block; if no Indent follows, emit no signals.
    let indented = consume_token(parser, &TokenType::Indent);

    loop {
        skip_structural(parser);
        if indented && parser.check(&TokenType::Dedent) {
            parser.current += 1;
            break;
        }
        if is_eof_or_next_block(parser) {
            break;
        }
        if parser.check(&TokenType::Dedent) {
            parser.current += 1;
            break;
        }
        if is_end_of_inner_block(parser) {
            break;
        }

        let name = expect_bare_identifier(parser, "signal name")?;
        expect_colon(parser, "signal type")?;
        let ty = parse_signal_type(parser, preamble)?;

        preamble.signals.push(SignalDecl {
            name,
            ty,
            domain: None,
        });
        consume_line_terminator(parser);
    }

    Ok(())
}

fn parse_parameters_sigil(parser: &mut Parser, preamble: &mut HwPreamble) -> ParseResult<()> {
    parser.current += 1; // consume "parameters"
    expect_colon(parser, "parameters")?;
    consume_line_terminator(parser);

    let indented = consume_token(parser, &TokenType::Indent);

    loop {
        skip_structural(parser);
        if indented && parser.check(&TokenType::Dedent) {
            parser.current += 1;
            break;
        }
        if is_eof_or_next_block(parser) {
            break;
        }
        if parser.check(&TokenType::Dedent) {
            parser.current += 1;
            break;
        }
        if is_end_of_inner_block(parser) {
            break;
        }

        let name = expect_bare_identifier(parser, "parameter name")?;
        expect_colon(parser, "parameter type")?;
        let ty = parse_signal_type(parser, preamble)?;

        preamble.parameters.push(ParameterDecl {
            name,
            ty,
            default: None,
        });
        consume_line_terminator(parser);
    }

    Ok(())
}

fn parse_interfaces_sigil(parser: &mut Parser, preamble: &mut HwPreamble) -> ParseResult<()> {
    parser.current += 1; // consume "interfaces"
    expect_colon(parser, "interfaces")?;
    consume_line_terminator(parser);

    let indented = consume_token(parser, &TokenType::Indent);

    loop {
        skip_structural(parser);
        if indented && parser.check(&TokenType::Dedent) {
            parser.current += 1;
            break;
        }
        if is_eof_or_next_block(parser) {
            break;
        }
        if parser.check(&TokenType::Dedent) {
            parser.current += 1;
            break;
        }
        if is_end_of_inner_block(parser) {
            break;
        }

        let name = expect_bare_identifier(parser, "interface name")?;
        expect_colon(parser, "interface body")?;
        consume_line_terminator(parser);

        let mut fields = Vec::new();
        let field_block = consume_token(parser, &TokenType::Indent);
        loop {
            skip_structural(parser);
            if field_block && parser.check(&TokenType::Dedent) {
                parser.current += 1;
                break;
            }
            if !field_block {
                break;
            }
            if parser.check(&TokenType::Dedent) {
                parser.current += 1;
                break;
            }
            if is_end_of_inner_block(parser) {
                break;
            }

            let field_name = expect_bare_identifier(parser, "interface field name")?;
            expect_colon(parser, "interface field type")?;
            let ty = parse_signal_type(parser, preamble)?;
            fields.push(InterfaceField { name: field_name, ty });
            consume_line_terminator(parser);
        }

        preamble.interfaces.push(InterfaceDecl { name, fields });
    }

    Ok(())
}

fn parse_let_binding<'a, 'ctx, 'int>(
    parser: &mut Parser<'a, 'ctx, 'int>,
    preamble: &mut HwPreamble<'a>,
) -> ParseResult<()> {
    parser.current += 1; // consume "let"
    let name = expect_bare_identifier(parser, "let binding name")?;

    if !consume_token(parser, &TokenType::Assign) {
        return err_here(parser, "expected `=` after `let <name>`");
    }

    let rhs = parser.parse_imperative_expr()?;

    preamble.lets.push(LetBinding {
        name,
        rhs,
        inferred_type: None,
    });
    consume_line_terminator(parser);
    Ok(())
}

// ═══════════════════════════════════════════════════════════════════════════
// Type and expression helpers
// ═══════════════════════════════════════════════════════════════════════════

fn parse_signal_type(parser: &mut Parser, preamble: &mut HwPreamble) -> ParseResult<SignalType> {
    let token = parser.tokens.get(parser.current).cloned().ok_or_else(|| ParseError {
        kind: ParseErrorKind::Custom("unexpected end of input while parsing signal type".to_string()),
        span: current_span(parser),
    })?;

    let word = match &token.kind {
        TokenType::Noun(_)
        | TokenType::ProperName(_)
        | TokenType::Adjective(_)
        | TokenType::Identifier
        | TokenType::Verb { .. }
        | TokenType::Ambiguous { .. } => {
            parser.interner.resolve(token.lexeme).to_lowercase()
        }
        _ => {
            return err_here(parser, "expected signal type keyword (`scalar`, `bus`, or `enum`)");
        }
    };

    match word.as_str() {
        "scalar" => {
            parser.current += 1;
            Ok(SignalType::Scalar)
        }
        "bus" => {
            parser.current += 1;
            if !consume_token(parser, &TokenType::LBracket) {
                return err_here(parser, "expected `[H:L]` after `bus`");
            }
            let hi = expect_signed_integer(parser, "bus width hi")?;
            if !consume_token(parser, &TokenType::Colon) {
                return err_here(parser, "expected `:` in `bus[H:L]`");
            }
            let lo = expect_signed_integer(parser, "bus width lo")?;
            if !consume_token(parser, &TokenType::RBracket) {
                return err_here(parser, "expected `]` closing `bus[H:L]`");
            }
            Ok(SignalType::Bus { hi, lo })
        }
        "enum" => {
            parser.current += 1;
            if !consume_token(parser, &TokenType::LBrace) {
                return err_here(parser, "expected `{` opening `enum {...}`");
            }
            let mut values = Vec::new();
            loop {
                let value = expect_bare_identifier(parser, "enum variant")?;
                values.push(value);
                if !consume_token(parser, &TokenType::Comma) {
                    break;
                }
            }
            if !consume_token(parser, &TokenType::RBrace) {
                return err_here(parser, "expected `}` closing `enum {...}`");
            }
            let id = preamble.types.register_enum(values);
            Ok(SignalType::Enum(id))
        }
        _ => err_here(
            parser,
            "expected signal type keyword (`scalar`, `bus`, or `enum`)",
        ),
    }
}

fn rebuild_symbol_table(preamble: &mut HwPreamble) {
    let mut table = HwSymbolTable::default();
    for signal in &preamble.signals {
        table.insert(signal.name, signal.ty.clone(), signal.domain);
    }
    preamble.symbols = table;
}

// ═══════════════════════════════════════════════════════════════════════════
// Low-level token helpers
// ═══════════════════════════════════════════════════════════════════════════

fn skip_structural(parser: &mut Parser) {
    while matches!(
        parser.tokens.get(parser.current).map(|t| &t.kind),
        Some(TokenType::Newline)
    ) {
        parser.current += 1;
    }
}

fn is_eof_or_next_block(parser: &Parser) -> bool {
    match parser.tokens.get(parser.current).map(|t| &t.kind) {
        None | Some(TokenType::EOF) => true,
        Some(TokenType::BlockHeader { .. }) => true,
        _ => false,
    }
}

fn consume_token(parser: &mut Parser, kind: &TokenType) -> bool {
    if parser.tokens.get(parser.current).map(|t| &t.kind) == Some(kind) {
        parser.current += 1;
        true
    } else {
        false
    }
}

fn consume_word(parser: &mut Parser, word: &str) -> bool {
    let token = match parser.tokens.get(parser.current) {
        Some(t) => t,
        None => return false,
    };
    match &token.kind {
        TokenType::Noun(_) | TokenType::ProperName(_) | TokenType::Adjective(_) | TokenType::Identifier => {
            let resolved = parser.interner.resolve(token.lexeme).to_lowercase();
            if resolved == word {
                parser.current += 1;
                true
            } else {
                false
            }
        }
        _ => false,
    }
}

fn expect_word(parser: &mut Parser, word: &str) -> ParseResult<()> {
    if consume_word(parser, word) {
        Ok(())
    } else {
        err_here(parser, &format!("expected `{}`", word))
    }
}

fn expect_colon(parser: &mut Parser, after: &str) -> ParseResult<()> {
    if consume_token(parser, &TokenType::Colon) {
        Ok(())
    } else {
        err_here(parser, &format!("expected `:` after `{}`", after))
    }
}

fn expect_bare_identifier(parser: &mut Parser, what: &str) -> ParseResult<Symbol> {
    let token = parser
        .tokens
        .get(parser.current)
        .cloned()
        .ok_or_else(|| ParseError {
            kind: ParseErrorKind::Custom(format!("expected {} but reached end of input", what)),
            span: current_span(parser),
        })?;
    match &token.kind {
        TokenType::Noun(sym)
        | TokenType::ProperName(sym)
        | TokenType::Adjective(sym) => {
            parser.current += 1;
            Ok(*sym)
        }
        // HW-Spec signal names land in a variety of POS classes depending on
        // the general-purpose lexicon: `signals`/`grant` as Verbs, `a`/`an`
        // as Articles, FSM state names like `state` as Ambiguous. All of
        // them carry the raw identifier in `token.lexeme`.
        TokenType::Verb { .. }
        | TokenType::Identifier
        | TokenType::Article(_)
        | TokenType::Ambiguous { .. } => {
            parser.current += 1;
            Ok(token.lexeme)
        }
        _ => err_here(parser, &format!("expected {}", what)),
    }
}

fn expect_signed_integer(parser: &mut Parser, what: &str) -> ParseResult<i64> {
    let token = parser
        .tokens
        .get(parser.current)
        .cloned()
        .ok_or_else(|| ParseError {
            kind: ParseErrorKind::Custom(format!("expected {} but reached end of input", what)),
            span: current_span(parser),
        })?;
    match &token.kind {
        TokenType::Number(sym) => {
            let raw = parser.interner.resolve(*sym);
            parser.current += 1;
            raw.parse::<i64>()
                .map_err(|_| ParseError {
                    kind: ParseErrorKind::Custom(format!("invalid integer `{}` for {}", raw, what)),
                    span: token.span,
                })
        }
        _ => err_here(parser, &format!("expected integer for {}", what)),
    }
}

fn consume_line_terminator(parser: &mut Parser) {
    // HW-Spec preamble directives use newline or indent/dedent as their line
    // terminator; periods are optional. Just eat any whitespace-ish tokens
    // so the next iteration starts clean.
    while matches!(
        parser.tokens.get(parser.current).map(|t| &t.kind),
        Some(TokenType::Newline) | Some(TokenType::Period)
    ) {
        parser.current += 1;
    }
}

fn current_span(parser: &Parser) -> crate::token::Span {
    parser
        .tokens
        .get(parser.current)
        .map(|t| t.span)
        .unwrap_or_else(|| crate::token::Span::new(0, 0))
}

fn err_here<T>(parser: &Parser, msg: &str) -> ParseResult<T> {
    Err(ParseError {
        kind: ParseErrorKind::Custom(msg.to_string()),
        span: current_span(parser),
    })
}
