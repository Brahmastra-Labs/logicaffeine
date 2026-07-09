use tower_lsp::lsp_types::{
    Diagnostic, DiagnosticRelatedInformation, DiagnosticSeverity, Location, Range, Url,
};

use logicaffeine_base::Interner;
use logicaffeine_language::error::{ParseError, ParseErrorKind, socratic_explanation};
use logicaffeine_language::token::{Token, TokenType};

use crate::index::{find_cause_keyword_span, find_last_token_span_for_name};
use crate::line_index::LineIndex;
use crate::pipeline::AnalysisError;

/// Convert a list of parse errors to LSP diagnostics.
///
/// `tokens` and `uri` enable `DiagnosticRelatedInformation` linking an error
/// to its cause site (e.g. use-after-move → the Give statement). Pass `&[]`
/// and `None` when neither is available.
pub fn convert_errors(
    errors: &[ParseError],
    tokens: &[Token],
    interner: &Interner,
    line_index: &LineIndex,
    uri: Option<&Url>,
) -> Vec<Diagnostic> {
    errors
        .iter()
        .map(|e| error_to_diagnostic(e, tokens, interner, line_index, uri))
        .collect()
}

fn error_to_diagnostic(
    error: &ParseError,
    tokens: &[Token],
    interner: &Interner,
    line_index: &LineIndex,
    uri: Option<&Url>,
) -> Diagnostic {
    let start = line_index.position(error.span.start);
    let end = line_index.position(error.span.end.max(error.span.start + 1));

    let message = socratic_explanation(error, interner);
    let decision = decision_for(&error.kind);
    let related_information =
        uri.and_then(|u| parse_error_related_info(error, tokens, interner, line_index, u));

    Diagnostic {
        range: Range { start, end },
        severity: Some(decision.severity),
        code: decision
            .code
            .map(|c| tower_lsp::lsp_types::NumberOrString::String(c.to_string())),
        code_description: match decision.docs {
            DocsLink::Anchor(anchor) => code_description_for(anchor),
            DocsLink::None(_) => None,
        },
        source: Some("logicaffeine".to_string()),
        message,
        related_information,
        ..Default::default()
    }
}

/// Hints for `Let` bindings nothing ever reads.
///
/// A definition counts as used when any reference RESOLVING TO IT sits at a
/// different span than the definition itself (the definition site indexes a
/// self-reference, which does not count). Variables only — an unused
/// parameter can be a deliberate interface, and fields/variants are shape,
/// not dataflow. Severity HINT + the UNNECESSARY tag: editors fade the
/// binding rather than shouting about it.
pub fn unused_variable_hints(
    index: &crate::index::SymbolIndex,
    line_index: &LineIndex,
) -> Vec<Diagnostic> {
    let mut hints = Vec::new();
    for (def_idx, def) in index.definitions.iter().enumerate() {
        if def.kind != crate::index::DefinitionKind::Variable {
            continue;
        }
        if def.span == logicaffeine_language::token::Span::default() || def.name.starts_with('_') {
            continue;
        }
        let used = index.references.iter().any(|r| {
            r.definition_idx == Some(def_idx) && r.span.start != def.span.start
        });
        if used {
            continue;
        }
        hints.push(Diagnostic {
            range: Range {
                start: line_index.position(def.span.start),
                end: line_index.position(def.span.end),
            },
            severity: Some(DiagnosticSeverity::HINT),
            code: Some(tower_lsp::lsp_types::NumberOrString::String(
                "unused-variable".to_string(),
            )),
            code_description: code_description_for("2-variables--mutation"),
            source: Some("logicaffeine".to_string()),
            message: format!(
                "'{name}' is never used.\n\nNothing reads '{name}' after this Let. \
                Remove the statement, or use the value.",
                name = def.name
            ),
            tags: Some(vec![tower_lsp::lsp_types::DiagnosticTag::UNNECESSARY]),
            ..Default::default()
        });
    }
    hints
}

/// The severity contract for the analysis-layer hint/warning codes — one row
/// per emitted code, locked (both directions) by `tests/locks.rs`.
pub const ANALYSIS_DECISIONS: &[(&str, DiagnosticSeverity, &str)] = &[
    (
        "unused-variable",
        DiagnosticSeverity::HINT,
        "a faded nudge — an unused binding is often work in progress, never a failure",
    ),
    (
        "shadowed-variable",
        DiagnosticSeverity::WARNING,
        "legal but usually unintended — the earlier binding silently becomes unreachable",
    ),
    (
        "unused-function",
        DiagnosticSeverity::HINT,
        "a faded nudge, and only in programs with a Main — a library file's functions are its API",
    ),
];

/// Warnings for re-declared names: a second `Let x` in the same block makes
/// the earlier `x` unreachable from that point on — legal, and usually a
/// misspelled `Set`.
pub fn shadowing_warnings(
    index: &crate::index::SymbolIndex,
    line_index: &LineIndex,
    uri: Option<&Url>,
) -> Vec<Diagnostic> {
    use std::collections::HashMap;
    let mut by_name: HashMap<(&str, Option<usize>), Vec<&crate::index::Definition>> =
        HashMap::new();
    for def in &index.definitions {
        if def.kind != crate::index::DefinitionKind::Variable
            || def.span == logicaffeine_language::token::Span::default()
        {
            continue;
        }
        by_name.entry((def.name.as_str(), def.scope.block_idx)).or_default().push(def);
    }

    let mut warnings = Vec::new();
    for ((name, _), mut defs) in by_name {
        if defs.len() < 2 {
            continue;
        }
        defs.sort_by_key(|d| d.span.start);
        let first = defs[0];
        for shadow in defs.iter().skip(1).filter(|d| d.span != first.span) {
            warnings.push(Diagnostic {
                range: Range {
                    start: line_index.position(shadow.span.start),
                    end: line_index.position(shadow.span.end),
                },
                severity: Some(DiagnosticSeverity::WARNING),
                code: Some(tower_lsp::lsp_types::NumberOrString::String(
                    "shadowed-variable".to_string(),
                )),
                code_description: code_description_for("2-variables--mutation"),
                source: Some("logicaffeine".to_string()),
                message: format!(
                    "'{name}' is declared again — the earlier '{name}' above becomes \
                     unreachable from here on. Did you mean to update it with \
                     'Set {name} to …', or does this new value deserve its own name?"
                ),
                related_information: uri.map(|u| {
                    vec![DiagnosticRelatedInformation {
                        location: Location {
                            uri: u.clone(),
                            range: Range {
                                start: line_index.position(first.span.start),
                                end: line_index.position(first.span.end),
                            },
                        },
                        message: format!("the earlier '{name}' is declared here"),
                    }]
                }),
                ..Default::default()
            });
        }
    }
    warnings
}

/// Hints for functions nothing calls — only in programs WITH a `## Main`
/// (a library file's functions are its API, not dead code).
pub fn unused_function_hints(
    index: &crate::index::SymbolIndex,
    line_index: &LineIndex,
) -> Vec<Diagnostic> {
    let has_main = index
        .block_spans
        .iter()
        .any(|(_, block_type, _)| {
            matches!(block_type, logicaffeine_language::token::BlockType::Main)
        });
    if !has_main {
        return Vec::new();
    }

    let mut hints = Vec::new();
    for (def_idx, def) in index.definitions.iter().enumerate() {
        if def.kind != crate::index::DefinitionKind::Function
            || def.span == logicaffeine_language::token::Span::default()
        {
            continue;
        }
        let called = index.references.iter().any(|r| {
            r.definition_idx == Some(def_idx) && r.span.start != def.span.start
        });
        if called {
            continue;
        }
        hints.push(Diagnostic {
            range: Range {
                start: line_index.position(def.span.start),
                end: line_index.position(def.span.end),
            },
            severity: Some(DiagnosticSeverity::HINT),
            code: Some(tower_lsp::lsp_types::NumberOrString::String(
                "unused-function".to_string(),
            )),
            code_description: code_description_for("7-functions--closures"),
            source: Some("logicaffeine".to_string()),
            message: format!(
                "'{name}' is never called.\n\nNothing in this program calls '{name}'. \
                 Is it wired up yet, or left over from a refactor?",
                name = def.name
            ),
            tags: Some(vec![tower_lsp::lsp_types::DiagnosticTag::UNNECESSARY]),
            ..Default::default()
        });
    }
    hints
}

/// Link a parse error to its cause site where one exists in the token stream.
fn parse_error_related_info(
    error: &ParseError,
    tokens: &[Token],
    interner: &Interner,
    line_index: &LineIndex,
    uri: &Url,
) -> Option<Vec<DiagnosticRelatedInformation>> {
    match &error.kind {
        ParseErrorKind::UseAfterMove { name } => {
            let cause_span = find_cause_keyword_span(
                tokens,
                TokenType::Give,
                name,
                interner,
                error.span.start,
            )?;
            Some(vec![DiagnosticRelatedInformation {
                location: Location {
                    uri: uri.clone(),
                    range: Range {
                        start: line_index.position(cause_span.start),
                        end: line_index.position(cause_span.end),
                    },
                },
                message: format!("'{}' was given away here", name),
            }])
        }
        _ => None,
    }
}

/// Convert analysis errors (escape/ownership) to LSP diagnostics.
///
/// The `uri` parameter is used for `DiagnosticRelatedInformation` locations.
/// Pass `None` to omit related information (e.g., in tests without a real URI).
pub fn convert_analysis_errors(
    errors: &[AnalysisError],
    tokens: &[Token],
    interner: &Interner,
    line_index: &LineIndex,
    uri: Option<&Url>,
) -> Vec<Diagnostic> {
    errors
        .iter()
        .map(|e| analysis_error_to_diagnostic(e, tokens, interner, line_index, uri))
        .collect()
}

fn analysis_error_to_diagnostic(
    error: &AnalysisError,
    tokens: &[Token],
    interner: &Interner,
    line_index: &LineIndex,
    uri: Option<&Url>,
) -> Diagnostic {
    // Primary range: the variable's token INSIDE the erring statement when
    // the checker gave us that statement's span; else the first occurrence.
    let primary = error
        .use_span
        .and_then(|stmt| {
            tokens
                .iter()
                .filter(|t| t.span.start >= stmt.start && t.span.end <= stmt.end)
                .find(|t| {
                    crate::index::resolve_token_name(t, interner)
                        .map(|n| n == error.variable)
                        .unwrap_or(false)
                })
                .map(|t| t.span)
                .or(Some(stmt))
        })
        .or_else(|| find_last_token_span_for_name(tokens, &error.variable, interner));
    let (start, end) = if let Some(span) = primary {
        (line_index.position(span.start), line_index.position(span.end))
    } else {
        // Fallback: point to start of document
        let zero = tower_lsp::lsp_types::Position { line: 0, character: 0 };
        (zero, zero)
    };

    // Build related information pointing to the cause (Give statement, Zone entry, etc.)
    let use_offset = primary.map(|s| s.start).unwrap_or(usize::MAX);
    let related_information =
        uri.and_then(|u| build_related_information(error, tokens, interner, line_index, u, use_offset));

    Diagnostic {
        range: Range { start, end },
        severity: Some(DiagnosticSeverity::ERROR),
        code: Some(tower_lsp::lsp_types::NumberOrString::String(error.code.to_string())),
        code_description: docs_anchor_for_analysis_code(error.code)
            .and_then(code_description_for),
        source: Some("logicaffeine".to_string()),
        message: error.message.clone(),
        related_information,
        ..Default::default()
    }
}

/// Build `DiagnosticRelatedInformation` linking the error to its cause.
fn build_related_information(
    error: &AnalysisError,
    tokens: &[Token],
    interner: &Interner,
    line_index: &LineIndex,
    uri: &Url,
    use_offset: usize,
) -> Option<Vec<DiagnosticRelatedInformation>> {
    let cause_message = error.cause_context.as_ref()?;

    // Determine which keyword token to search for based on error code
    let keyword = match error.code {
        "use-after-move" | "double-move" | "maybe-moved" => TokenType::Give,
        "escape-return" | "escape-assignment" => TokenType::Zone,
        _ => return None,
    };

    // The checker's statement-level cause (from stmt_spans) is exact; the
    // bounded object-position keyword search is the fallback.
    let cause_span = error.cause_span.or_else(|| {
        find_cause_keyword_span(tokens, keyword, &error.variable, interner, use_offset)
    })?;
    let cause_start = line_index.position(cause_span.start);
    let cause_end = line_index.position(cause_span.end);

    Some(vec![DiagnosticRelatedInformation {
        location: Location {
            uri: uri.clone(),
            range: Range {
                start: cause_start,
                end: cause_end,
            },
        },
        message: cause_message.clone(),
    }])
}

/// The explicit presentation decision for one `ParseErrorKind`.
///
/// Every kind decides all three axes. The wildcard-free match in
/// [`decision_for`] is the compile-time ratchet: a new error kind does not
/// build until someone chooses its severity, its diagnostic code, and whether
/// a quickfix is safe — `tests/locks.rs` pins the runtime side.
pub struct ErrorDecision {
    pub severity: DiagnosticSeverity,
    /// Stable kebab-case code that code actions and clients key on.
    pub code: Option<&'static str>,
    pub quickfix: Quickfix,
    /// Whether the diagnostic links a quickguide lesson via `codeDescription`.
    pub docs: DocsLink,
}

/// Whether `code_actions` offers a mechanical fix for an error kind.
pub enum Quickfix {
    /// A fix exists, keyed on the diagnostic code; the string names it.
    Provided(&'static str),
    /// Deliberately no fix; the reason is part of the decision.
    None(&'static str),
}

/// Whether a diagnostic carries a "read more" link into LOGOS_QUICKGUIDE.md.
pub enum DocsLink {
    /// A quickguide heading slug (rendered through `teach_md::guide_url`).
    /// Clients show `codeDescription` alongside the code, so a code is
    /// required — the decision lock enforces it.
    Anchor(&'static str),
    /// Deliberately no link; the reason is part of the decision.
    None(&'static str),
}

/// The quickguide anchor for an analysis-path diagnostic code (ownership and
/// escape findings arrive as `AnalysisError`, outside `decision_for`).
fn docs_anchor_for_analysis_code(code: &str) -> Option<&'static str> {
    match code {
        "use-after-move" | "double-move" | "maybe-moved" => Some("13-output"),
        "escape-return" | "escape-assignment" => {
            Some("12-distributed-crdt-concurrency-networking-zones")
        }
        _ => None,
    }
}

/// A ready `codeDescription` for a quickguide anchor.
fn code_description_for(anchor: &str) -> Option<tower_lsp::lsp_types::CodeDescription> {
    Url::parse(&crate::teach_md::guide_url(anchor))
        .ok()
        .map(|href| tower_lsp::lsp_types::CodeDescription { href })
}

pub fn decision_for(kind: &ParseErrorKind) -> ErrorDecision {
    use DiagnosticSeverity as S;
    let (severity, code, quickfix, docs) = match kind {
        // Style: valid programs that read wrong.
        ParseErrorKind::IsValueEquality { .. } => (
            S::WARNING,
            Some("is-value-equality"),
            Quickfix::Provided("Use 'equals' for value comparison"),
            DocsLink::Anchor("3-arithmetic-comparison-logic-bitwise"),
        ),
        ParseErrorKind::GrammarError(_) => (
            S::WARNING,
            Some("grammar-error"),
            Quickfix::None("the message itself names the correction; no single edit is safe"),
            DocsLink::None("the guide documents constructs, not English orthography"),
        ),

        // Idiom: 1-based indexing is a convention, not a failure.
        ParseErrorKind::ZeroIndex => (
            S::INFORMATION,
            Some("zero-index"),
            Quickfix::Provided("Use 1-based indexing"),
            DocsLink::Anchor("5-collections"),
        ),

        // Ownership.
        ParseErrorKind::UseAfterMove { .. } => (
            S::ERROR,
            Some("use-after-move"),
            Quickfix::Provided("Use 'a copy of …' instead"),
            DocsLink::Anchor("13-output"),
        ),

        // Name resolution.
        ParseErrorKind::UndefinedVariable { .. } => (
            S::ERROR,
            Some("undefined-variable"),
            Quickfix::Provided("Did you mean '<nearest definition>'?"),
            DocsLink::Anchor("2-variables--mutation"),
        ),

        // Types.
        ParseErrorKind::TypeMismatch { .. } | ParseErrorKind::TypeMismatchDetailed { .. } => (
            S::ERROR,
            Some("type-mismatch"),
            Quickfix::None("no conversion is safe for arbitrary expected/found pairs"),
            DocsLink::Anchor("2-variables--mutation"),
        ),
        ParseErrorKind::InfiniteType { .. } => (
            S::ERROR,
            Some("infinite-type"),
            Quickfix::None("breaking a type cycle requires restructuring, not an edit"),
            DocsLink::None("the fix is structural; no guide section teaches type cycles"),
        ),
        ParseErrorKind::ArityMismatch { .. } => (
            S::ERROR,
            Some("arity-mismatch"),
            Quickfix::None("which argument to add or drop is the author's call"),
            DocsLink::Anchor("7-functions--closures"),
        ),
        ParseErrorKind::FieldNotFound { .. } => (
            S::ERROR,
            Some("field-not-found"),
            Quickfix::None(
                "the diagnostic anchors on the whole statement; the message lists the fields that exist",
            ),
            DocsLink::Anchor("8-structs-enums--field-access"),
        ),
        ParseErrorKind::NotAFunction { .. } => (
            S::ERROR,
            Some("not-a-function"),
            Quickfix::None("the intended callee is unknowable from the call site"),
            DocsLink::Anchor("7-functions--closures"),
        ),
        ParseErrorKind::InvalidRefinementPredicate => (
            S::ERROR,
            None,
            Quickfix::None("a refinement predicate must be rethought, not patched"),
            DocsLink::None("no diagnostic code to hang a codeDescription on"),
        ),
        ParseErrorKind::AstTooDeep { .. } => (
            S::ERROR,
            Some("ast-too-deep"),
            Quickfix::None("nesting past the engine's recursion gate needs restructuring, not an edit"),
            DocsLink::None("the depth limit is an engine gate, not a construct the guide teaches"),
        ),

        // Sentence structure: the socratic message lists example words; the
        // choice of word is the author's, so no insertion is safe.
        ParseErrorKind::UnexpectedToken { .. }
        | ParseErrorKind::ExpectedContentWord { .. }
        | ParseErrorKind::ExpectedCopula
        | ParseErrorKind::UnknownQuantifier { .. }
        | ParseErrorKind::UnknownModal { .. }
        | ParseErrorKind::ExpectedVerb { .. }
        | ParseErrorKind::ExpectedTemporalAdverb
        | ParseErrorKind::ExpectedPresuppositionTrigger
        | ParseErrorKind::ExpectedFocusParticle
        | ParseErrorKind::ExpectedScopalAdverb
        | ParseErrorKind::ExpectedSuperlativeAdjective
        | ParseErrorKind::ExpectedComparativeAdjective
        | ParseErrorKind::ExpectedThan
        | ParseErrorKind::ExpectedNumber
        | ParseErrorKind::ExpectedStatement
        | ParseErrorKind::ExpectedKeyword { .. }
        | ParseErrorKind::ExpectedExpression
        | ParseErrorKind::ExpectedIdentifier
        | ParseErrorKind::TrailingTokens { .. } => (
            S::ERROR,
            None,
            Quickfix::None("word choice is the author's; the message lists examples"),
            DocsLink::None("no diagnostic code; the socratic message already names example words"),
        ),

        // Linguistic restructuring: the sentence needs rethinking as a whole.
        ParseErrorKind::EmptyRestriction
        | ParseErrorKind::GappingResolutionFailed
        | ParseErrorKind::StativeProgressiveConflict
        | ParseErrorKind::RespectivelyLengthMismatch { .. }
        | ParseErrorKind::ScopeViolation(_)
        | ParseErrorKind::UnresolvedPronoun { .. } => (
            S::ERROR,
            None,
            Quickfix::None("requires restructuring the sentence, not a mechanical edit"),
            DocsLink::None("no diagnostic code; logic-mode phenomena are beyond the quickguide"),
        ),

        // Escape/zone analysis reports through AnalysisError with its own
        // codes; the Custom passthrough carries only prose here.
        ParseErrorKind::Custom(_) => (
            S::ERROR,
            None,
            Quickfix::None("a catch-all message carries no structure to key a fix on"),
            DocsLink::None("caller prose carries no stable identity to link"),
        ),
    };
    ErrorDecision { severity, code, quickfix, docs }
}

#[cfg(test)]
mod tests {
    use super::*;
    use logicaffeine_language::token::Span;

    #[test]
    fn parse_error_produces_diagnostic() {
        let interner = Interner::new();
        let line_index = LineIndex::new("Let x be 5.\nSet x to 10.");

        let error = ParseError {
            kind: ParseErrorKind::ExpectedExpression,
            span: Span::new(12, 15),
        };

        let diagnostics = convert_errors(&[error], &[], &interner, &line_index, None);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].range.start.line, 1);
        assert_eq!(diagnostics[0].severity, Some(DiagnosticSeverity::ERROR));
        assert!(diagnostics[0].message.contains("expression"));
    }

    #[test]
    fn is_value_equality_is_warning() {
        let interner = Interner::new();
        let line_index = LineIndex::new("x is 5");

        let error = ParseError {
            kind: ParseErrorKind::IsValueEquality {
                variable: "x".to_string(),
                value: "5".to_string(),
            },
            span: Span::new(0, 6),
        };

        let diagnostics = convert_errors(&[error], &[], &interner, &line_index, None);
        assert_eq!(diagnostics[0].severity, Some(DiagnosticSeverity::WARNING));
    }

    #[test]
    fn grammar_error_is_warning() {
        let interner = Interner::new();
        let line_index = LineIndex::new("bad grammar");

        let error = ParseError {
            kind: ParseErrorKind::GrammarError("test".to_string()),
            span: Span::new(0, 3),
        };

        let diagnostics = convert_errors(&[error], &[], &interner, &line_index, None);
        assert_eq!(diagnostics[0].severity, Some(DiagnosticSeverity::WARNING));
    }

    #[test]
    fn zero_index_is_information() {
        let interner = Interner::new();
        let line_index = LineIndex::new("list[0]");

        let error = ParseError {
            kind: ParseErrorKind::ZeroIndex,
            span: Span::new(4, 7),
        };

        let diagnostics = convert_errors(&[error], &[], &interner, &line_index, None);
        assert_eq!(diagnostics[0].severity, Some(DiagnosticSeverity::INFORMATION));
    }

    #[test]
    fn multiple_errors_produce_multiple_diagnostics() {
        let interner = Interner::new();
        let line_index = LineIndex::new("abc\ndef\nghi");

        let errors = vec![
            ParseError {
                kind: ParseErrorKind::ExpectedExpression,
                span: Span::new(0, 3),
            },
            ParseError {
                kind: ParseErrorKind::ExpectedExpression,
                span: Span::new(4, 7),
            },
        ];

        let diagnostics = convert_errors(&errors, &[], &interner, &line_index, None);
        assert_eq!(diagnostics.len(), 2);
        assert_eq!(diagnostics[0].range.start.line, 0);
        assert_eq!(diagnostics[1].range.start.line, 1);
    }

    #[test]
    fn empty_errors_produce_empty_diagnostics() {
        let interner = Interner::new();
        let line_index = LineIndex::new("fine");
        let diagnostics = convert_errors(&[], &[], &interner, &line_index, None);
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn use_after_move_is_error_severity() {
        let decision = decision_for(&ParseErrorKind::UseAfterMove {
            name: "x".to_string(),
        });
        assert_eq!(decision.severity, DiagnosticSeverity::ERROR);
    }

    #[test]
    fn diagnostic_range_spans_correct_positions() {
        let interner = Interner::new();
        let line_index = LineIndex::new("abc\ndef");
        let error = ParseError {
            kind: ParseErrorKind::ExpectedExpression,
            span: Span::new(4, 7),
        };
        let diagnostics = convert_errors(&[error], &[], &interner, &line_index, None);
        assert_eq!(diagnostics[0].range.start.line, 1);
        assert_eq!(diagnostics[0].range.start.character, 0);
        assert_eq!(diagnostics[0].range.end.line, 1);
        assert_eq!(diagnostics[0].range.end.character, 3);
    }

    #[test]
    fn is_value_equality_has_diagnostic_code() {
        let interner = Interner::new();
        let line_index = LineIndex::new("x is 5");

        let error = ParseError {
            kind: ParseErrorKind::IsValueEquality {
                variable: "x".to_string(),
                value: "5".to_string(),
            },
            span: Span::new(0, 6),
        };

        let diagnostics = convert_errors(&[error], &[], &interner, &line_index, None);
        assert_eq!(
            diagnostics[0].code,
            Some(tower_lsp::lsp_types::NumberOrString::String("is-value-equality".to_string())),
            "IsValueEquality should have diagnostic code"
        );
    }

    #[test]
    fn use_after_move_has_diagnostic_code() {
        let interner = Interner::new();
        let line_index = LineIndex::new("x moved");

        let error = ParseError {
            kind: ParseErrorKind::UseAfterMove {
                name: "x".to_string(),
            },
            span: Span::new(0, 1),
        };

        let diagnostics = convert_errors(&[error], &[], &interner, &line_index, None);
        assert_eq!(
            diagnostics[0].code,
            Some(tower_lsp::lsp_types::NumberOrString::String("use-after-move".to_string())),
            "UseAfterMove should have diagnostic code"
        );
    }

    #[test]
    fn regular_error_has_no_diagnostic_code() {
        let interner = Interner::new();
        let line_index = LineIndex::new("bad");

        let error = ParseError {
            kind: ParseErrorKind::ExpectedExpression,
            span: Span::new(0, 3),
        };

        let diagnostics = convert_errors(&[error], &[], &interner, &line_index, None);
        assert_eq!(diagnostics[0].code, None, "ExpectedExpression should have no code");
    }

    #[test]
    fn diagnostic_has_source() {
        let interner = Interner::new();
        let line_index = LineIndex::new("bad");

        let error = ParseError {
            kind: ParseErrorKind::ExpectedExpression,
            span: Span::new(0, 3),
        };

        let diagnostics = convert_errors(&[error], &[], &interner, &line_index, None);
        assert_eq!(diagnostics[0].source, Some("logicaffeine".to_string()));
    }

    #[test]
    fn escape_error_produces_diagnostic_with_code() {
        let error = AnalysisError {
            message: "Reference 'x' cannot escape zone 'temp'.".to_string(),
            variable: "x".to_string(),
            code: "escape-return",
            cause_context: Some("zone 'temp'".to_string()),
            use_span: None,
            cause_span: None,
        };
        let interner = Interner::new();
        let line_index = LineIndex::new("Let x be 5.");
        let diagnostics = convert_analysis_errors(&[error], &[], &interner, &line_index, None);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].code,
            Some(tower_lsp::lsp_types::NumberOrString::String("escape-return".to_string()))
        );
        assert_eq!(diagnostics[0].severity, Some(DiagnosticSeverity::ERROR));
    }

    #[test]
    fn ownership_error_has_error_severity() {
        let error = AnalysisError {
            message: "Cannot use 'x' after giving it away.".to_string(),
            variable: "x".to_string(),
            code: "use-after-move",
            cause_context: None,
            use_span: None,
            cause_span: None,
        };
        let interner = Interner::new();
        let line_index = LineIndex::new("Let x be 5.");
        let diagnostics = convert_analysis_errors(&[error], &[], &interner, &line_index, None);
        assert_eq!(diagnostics[0].severity, Some(DiagnosticSeverity::ERROR));
    }

    #[test]
    fn ownership_error_message_is_socratic() {
        let error = AnalysisError {
            message: "Cannot use 'x' after giving it away.\n\nYou transferred ownership.".to_string(),
            variable: "x".to_string(),
            code: "use-after-move",
            cause_context: None,
            use_span: None,
            cause_span: None,
        };
        let interner = Interner::new();
        let line_index = LineIndex::new("Let x be 5.");
        let diagnostics = convert_analysis_errors(&[error], &[], &interner, &line_index, None);
        assert!(diagnostics[0].message.contains("giving it away"), "Message should be Socratic: {}", diagnostics[0].message);
        assert!(diagnostics[0].message.contains("ownership"), "Message should explain ownership: {}", diagnostics[0].message);
    }

    #[test]
    fn undefined_variable_has_diagnostic_code() {
        let interner = Interner::new();
        let line_index = LineIndex::new("Show z.");

        let error = ParseError {
            kind: ParseErrorKind::UndefinedVariable {
                name: "z".to_string(),
            },
            span: Span::new(5, 6),
        };

        let diagnostics = convert_errors(&[error], &[], &interner, &line_index, None);
        assert_eq!(
            diagnostics[0].code,
            Some(tower_lsp::lsp_types::NumberOrString::String("undefined-variable".to_string()))
        );
    }

    #[test]
    fn type_mismatch_has_diagnostic_code() {
        let interner = Interner::new();
        let line_index = LineIndex::new("Let x: Int be \"hello\".");

        let error = ParseError {
            kind: ParseErrorKind::TypeMismatch {
                expected: "Int".to_string(),
                found: "Text".to_string(),
            },
            span: Span::new(0, 22),
        };

        let diagnostics = convert_errors(&[error], &[], &interner, &line_index, None);
        assert_eq!(
            diagnostics[0].code,
            Some(tower_lsp::lsp_types::NumberOrString::String("type-mismatch".to_string()))
        );
    }

    #[test]
    fn use_after_move_diagnostic_has_related_information() {
        // Use the full pipeline to get real tokens with Give
        let source = "## Main\n    Let x be 5.\n    Let y be 0.\n    Give x to y.\n    Show x.\n";
        let result = crate::pipeline::analyze(source);
        let line_index = LineIndex::new(source);
        let uri = Url::parse("file:///test.la").unwrap();

        // Create a move error with cause_context (as pipeline does)
        let error = AnalysisError {
            message: "Cannot use 'x' after giving it away.".to_string(),
            variable: "x".to_string(),
            code: "use-after-move",
            cause_context: Some("'x' was given away here".to_string()),
            use_span: None,
            cause_span: None,
        };

        let diagnostics = convert_analysis_errors(
            &[error],
            &result.tokens,
            &result.interner,
            &line_index,
            Some(&uri),
        );

        assert_eq!(diagnostics.len(), 1);
        let related = diagnostics[0].related_information.as_ref();
        assert!(
            related.is_some(),
            "Use-after-move diagnostic should have related information"
        );
        let related = related.unwrap();
        assert_eq!(related.len(), 1);
        assert!(
            related[0].message.contains("given away"),
            "Related info should mention giving away: {}",
            related[0].message
        );
        assert_eq!(related[0].location.uri, uri);
    }

    #[test]
    fn related_info_points_to_give_statement() {
        let source = "## Main\n    Let x be 5.\n    Let y be 0.\n    Give x to y.\n    Show x.\n";
        let result = crate::pipeline::analyze(source);
        let line_index = LineIndex::new(source);
        let uri = Url::parse("file:///test.la").unwrap();

        let error = AnalysisError {
            message: "Cannot use 'x' after giving it away.".to_string(),
            variable: "x".to_string(),
            code: "use-after-move",
            cause_context: Some("'x' was given away here".to_string()),
            use_span: None,
            cause_span: None,
        };

        let diagnostics = convert_analysis_errors(
            &[error],
            &result.tokens,
            &result.interner,
            &line_index,
            Some(&uri),
        );

        let related = diagnostics[0].related_information.as_ref().unwrap();
        // The Give keyword should be on line 3 (0-indexed): "    Give x to y."
        assert_eq!(
            related[0].location.range.start.line, 3,
            "Related info should point to the Give statement on line 3"
        );
    }

    #[test]
    fn no_related_info_without_uri() {
        let source = "## Main\n    Let x be 5.\n    Let y be 0.\n    Give x to y.\n    Show x.\n";
        let result = crate::pipeline::analyze(source);
        let line_index = LineIndex::new(source);

        let error = AnalysisError {
            message: "Cannot use 'x' after giving it away.".to_string(),
            variable: "x".to_string(),
            code: "use-after-move",
            cause_context: Some("'x' was given away here".to_string()),
            use_span: None,
            cause_span: None,
        };

        let diagnostics = convert_analysis_errors(
            &[error],
            &result.tokens,
            &result.interner,
            &line_index,
            None,
        );

        assert!(
            diagnostics[0].related_information.is_none(),
            "Without URI, related information should be None"
        );
    }

    #[test]
    fn escape_diagnostic_has_related_info_pointing_to_zone() {
        // Escape errors should have related info pointing to the Zone keyword
        let error = AnalysisError {
            message: "Reference 'x' cannot escape zone 'temp'.".to_string(),
            variable: "x".to_string(),
            code: "escape-return",
            cause_context: Some("zone 'temp'".to_string()),
            use_span: None,
            cause_span: None,
        };

        // Without real tokens containing a Zone keyword, related info won't resolve.
        // This tests the fallback when no matching keyword is found.
        let interner = Interner::new();
        let line_index = LineIndex::new("Zone temp: Let x be 5.");
        let uri = Url::parse("file:///test.la").unwrap();
        let diagnostics = convert_analysis_errors(
            &[error],
            &[],
            &interner,
            &line_index,
            Some(&uri),
        );
        // With empty tokens, the keyword search fails gracefully
        assert_eq!(diagnostics.len(), 1);
        // related_information is None because no tokens matched
        assert!(
            diagnostics[0].related_information.is_none(),
            "With no tokens, related info should be None"
        );
    }
}
