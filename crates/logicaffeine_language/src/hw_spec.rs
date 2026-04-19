//! HW-Spec one-stop parser.
//!
//! `parse_hw_spec_with` is the single boundary through which raw `.hwspec`
//! text enters the language crate. It produces an [`HwSpec`] record
//! containing:
//!
//! * the typed [`HwPreamble`] from phase 3 (clocks, resets, signals, let
//!   bindings, parameters, interfaces, anonymous-enum type registry, and
//!   the canonical [`HwSymbolTable`])
//! * the property sentences lowered to `Vec<&LogicExpr>` via the
//!   declarative parser plus the axiom and Kripke passes
//!
//! Downstream lowering (`logicaffeine_compile::codegen_sva::*`) consumes
//! `&HwSpec` rather than re-tokenising raw text. Per-signal metadata flows
//! through `HwSpec.preamble.symbols`; the legacy `fol_signals: &[String] +
//! clock: &str` side channels retire in phase 4.

use crate::arena_ctx::AstContext;
use crate::ast::LogicExpr;
use crate::error::{ParseError, ParseErrorKind};
use crate::parser::{parse_hw_preamble, ClauseParsing, HwPreamble};
use crate::token::TokenType;
use crate::{analysis, drs, mwe, semantics, Interner, Lexer, Parser};
use logicaffeine_base::Arena;

/// Typed `.hwspec` IR.
///
/// Both fields are arena-lifetime: `preamble` holds `Expr` references for
/// `let` binding RHSes and `properties` holds `LogicExpr` references for
/// property sentences. The callback pattern in [`parse_hw_spec_with`]
/// ensures both arenas outlive any borrow.
pub struct HwSpec<'a> {
    pub preamble: HwPreamble<'a>,
    pub properties: Vec<&'a LogicExpr<'a>>,
}

/// Parse a `.hwspec` source string and hand the resulting [`HwSpec`] plus
/// the [`Interner`] to `f`.
///
/// The source may omit the `## Hardware` block header; this function will
/// prepend one if the first non-whitespace token is not already a block
/// header. The preamble/property boundary is decided by the phase-3
/// [`parse_hw_preamble`] recogniser — the first line that does not begin
/// with a known sigil terminates the preamble.
///
/// Property sentences are parsed individually through
/// [`ClauseParsing::parse_sentence`] so that each assertion in the
/// `.hwspec` file lands as its own `LogicExpr`. Kripke lowering and axiom
/// application run per-property, matching the way `compile_kripke_with`
/// already treats single sentences.
///
/// # Span offsets with auto-wrapped sources
///
/// When the caller supplies a headerless source, this function prepends
/// `## Hardware\n` (12 bytes) before tokenising. Any `ParseError` span
/// reported downstream refers to positions in the wrapped string, not the
/// caller's original input. Tooling that uses spans for highlighting
/// should either supply the `## Hardware` header explicitly or subtract
/// 12 from every span when the auto-wrap path was taken.
pub fn parse_hw_spec_with<F, R>(source: &str, f: F) -> Result<R, ParseError>
where
    F: FnOnce(&HwSpec<'_>, &Interner) -> R,
{
    if source.trim().is_empty() {
        return Err(ParseError {
            kind: ParseErrorKind::Custom("Empty .hwspec input".to_string()),
            span: crate::token::Span { start: 0, end: 0 },
        });
    }

    // Accept bare preamble+property text and fabricate the block header.
    // Callers that already embed `## Hardware` retain their original bytes,
    // so spans reported to user tooling line up with the source they wrote.
    let wrapped = if starts_with_block_header(source) {
        source.to_string()
    } else {
        format!("## Hardware\n{}\n", source)
    };

    let mut interner = Interner::new();
    let mut lexer = Lexer::new(&wrapped, &mut interner);
    let tokens = lexer.tokenize();

    let mwe_trie = mwe::build_mwe_trie();
    let tokens = mwe::apply_mwe_pipeline(tokens, &mwe_trie, &mut interner);

    let type_registry = {
        let mut discovery = analysis::DiscoveryPass::new(&tokens, &mut interner);
        discovery.run()
    };

    // Full imperative arena set: HW-Spec let RHSes allocate into
    // `imperative_expr_arena` while property sentences allocate into the
    // logical `expr_arena`. Sharing one `AstContext` keeps both lifetimes
    // equal so they can live inside the same `HwSpec<'a>`.
    let expr_arena = Arena::new();
    let term_arena = Arena::new();
    let np_arena = Arena::new();
    let sym_arena = Arena::new();
    let role_arena = Arena::new();
    let pp_arena = Arena::new();
    let stmt_arena = Arena::new();
    let imperative_expr_arena = Arena::new();

    let ctx = AstContext::with_imperative(
        &expr_arena,
        &term_arena,
        &np_arena,
        &sym_arena,
        &role_arena,
        &pp_arena,
        &stmt_arena,
        &imperative_expr_arena,
    );

    let mut world_state = drs::WorldState::new();
    let mut parser = Parser::new(tokens, &mut world_state, &mut interner, ctx, type_registry);
    parser.process_block_headers();

    let preamble_result = parse_hw_preamble(&mut parser)?;

    // Drop any leftover line-terminators before the property pass so the
    // declarative parser sees the first sentence head directly.
    skip_structural_tokens(&mut parser);

    let mut properties: Vec<&LogicExpr<'_>> = Vec::new();
    while !parser.is_at_end_public() {
        skip_structural_tokens(&mut parser);
        if parser.is_at_end_public() {
            break;
        }
        if parser.peek_is_block_header() {
            // A downstream block (e.g. a `## Note`) ends the preamble scope.
            break;
        }

        let sentence = parser.parse_sentence()?;
        let sentence = semantics::apply_axioms(
            sentence,
            ctx.exprs,
            ctx.terms,
            parser.interner_mut(),
        );
        let sentence = semantics::apply_kripke_lowering(
            sentence,
            ctx.exprs,
            ctx.terms,
            parser.interner_mut(),
        );
        properties.push(sentence);

        // Consume sentence terminators so the next iteration starts clean.
        while parser.consume_terminator() {}
    }

    let hw_spec = HwSpec {
        preamble: preamble_result.preamble,
        properties,
    };

    drop(parser);
    Ok(f(&hw_spec, &interner))
}

fn starts_with_block_header(source: &str) -> bool {
    for line in source.lines() {
        let trimmed = line.trim_start();
        if trimmed.is_empty() {
            continue;
        }
        return trimmed.starts_with("##");
    }
    false
}

fn skip_structural_tokens(parser: &mut Parser) {
    while matches!(
        parser.peek_kind(),
        Some(TokenType::Newline) | Some(TokenType::Indent) | Some(TokenType::Dedent)
    ) {
        parser.advance_public();
    }
}
