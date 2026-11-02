use lsp_types::{
    Diagnostic, DiagnosticRelatedInformation, DiagnosticSeverity, Location, Range, Url,
};

use logicaffeine_base::Interner;
use logicaffeine_language::error::{socratic_explanation, ParseError, ParseErrorKind};
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
            .map(|c| lsp_types::NumberOrString::String(c.to_string())),
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
        let used = index
            .references
            .iter()
            .any(|r| r.definition_idx == Some(def_idx) && r.span.start != def.span.start);
        if used {
            continue;
        }
        hints.push(Diagnostic {
            range: Range {
                start: line_index.position(def.span.start),
                end: line_index.position(def.span.end),
            },
            severity: Some(DiagnosticSeverity::HINT),
            code: Some(lsp_types::NumberOrString::String(
                "unused-variable".to_string(),
            )),
            code_description: code_description_for("2-variables--mutation"),
            source: Some("logicaffeine".to_string()),
            message: format!(
                "'{name}' is never used.\n\nNothing reads '{name}' after this Let. \
                Remove the statement, or use the value.",
                name = def.name
            ),
            tags: Some(vec![lsp_types::DiagnosticTag::UNNECESSARY]),
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
    (
        REFUSAL_CODE,
        DiagnosticSeverity::ERROR,
        "`largo build` stops at it — a page the browser build refuses does not ship, whatever the \
         tree-walker runs",
    ),
];

/// The LGX template checker's findings, as the editor shows them: at the element they are about,
/// with the checker's own severity and message.
///
/// The code is `lgx-` and the finding's kind in kebab case (`lgx-image-without-alt`), one per kind,
/// so a client can key on it the way it keys on a parse error's code.
pub fn lgx_finding_diagnostics(
    findings: &[logicaffeine_language::lgx::check::Finding],
    line_index: &LineIndex,
) -> Vec<Diagnostic> {
    use logicaffeine_language::lgx::check::Severity;
    findings
        .iter()
        .map(|f| Diagnostic {
            range: Range {
                start: line_index.position(f.span.start),
                end: line_index.position(f.span.end.max(f.span.start + 1)),
            },
            severity: Some(match f.severity {
                Severity::Error => DiagnosticSeverity::ERROR,
                Severity::Warning => DiagnosticSeverity::WARNING,
            }),
            code: Some(lsp_types::NumberOrString::String(lgx_finding_code(&f.kind))),
            source: Some("logicaffeine".to_string()),
            message: f.message.clone(),
            ..Default::default()
        })
        .collect()
}

/// `lgx-` and the kind's variant name in kebab case: `ImageWithoutAlt` → `lgx-image-without-alt`.
///
/// Derived from the variant rather than listed, so a kind the checker adds reaches the editor with
/// a code the moment it exists.
pub fn lgx_finding_code(kind: &logicaffeine_language::lgx::check::FindingKind) -> String {
    let debug = format!("{kind:?}");
    let variant = debug
        .split(|c: char| !c.is_ascii_alphanumeric())
        .next()
        .unwrap_or("");
    let mut code = String::from("lgx");
    for c in variant.chars() {
        if c.is_ascii_uppercase() {
            code.push('-');
        }
        code.push(c.to_ascii_lowercase());
    }
    code
}

/// The value-semantics copy lint's findings, as the editor shows them — same treatment
/// `lgx_finding_diagnostics` gives its own, separate finding type: at the mutation the lint is
/// about, with its own severity and message.
///
/// The code is `copy-` and the finding's kind in kebab case (`copy-orphaned-copy`), mirroring
/// `lgx_finding_code`'s own convention.
pub fn copy_finding_diagnostics(
    findings: &[logicaffeine_language::lint::Finding],
    line_index: &LineIndex,
) -> Vec<Diagnostic> {
    use logicaffeine_language::lint::Severity;
    findings
        .iter()
        .map(|f| Diagnostic {
            range: Range {
                start: line_index.position(f.span.start),
                end: line_index.position(f.span.end.max(f.span.start + 1)),
            },
            severity: Some(match f.severity {
                Severity::Warning => DiagnosticSeverity::WARNING,
            }),
            code: Some(lsp_types::NumberOrString::String(copy_finding_code(
                &f.kind,
            ))),
            source: Some("logicaffeine".to_string()),
            message: f.message.clone(),
            ..Default::default()
        })
        .collect()
}

/// `copy-` and the kind's variant name in kebab case: `OrphanedCopy` → `copy-orphaned-copy` —
/// mirrors `lgx_finding_code`'s own convention for its own, separate finding type.
pub fn copy_finding_code(kind: &logicaffeine_language::lint::FindingKind) -> String {
    let debug = format!("{kind:?}");
    let variant = debug
        .split(|c: char| !c.is_ascii_alphanumeric())
        .next()
        .unwrap_or("");
    let mut code = String::from("copy");
    for c in variant.chars() {
        if c.is_ascii_uppercase() {
            code.push('-');
        }
        code.push(c.to_ascii_lowercase());
    }
    code
}

/// The browser build's refusals, at the construct each refuses.
///
/// An error: the tree-walker may run the page under `largo test`, but `largo build` stops here,
/// and a page exists to reach a browser.
pub fn refusal_diagnostics(
    refusals: &[logicaffeine_compile::vm::compiler::Refusal],
    line_index: &LineIndex,
) -> Vec<Diagnostic> {
    refusals
        .iter()
        .filter_map(|r| {
            let at = r.at?;
            Some(Diagnostic {
                range: Range {
                    start: line_index.position(at.start),
                    end: line_index.position(at.end.max(at.start + 1)),
                },
                severity: Some(DiagnosticSeverity::ERROR),
                code: Some(lsp_types::NumberOrString::String(REFUSAL_CODE.to_string())),
                source: Some("logicaffeine".to_string()),
                message: r.message.clone(),
                ..Default::default()
            })
        })
        .collect()
}

/// The diagnostic code of a browser-tier refusal.
pub const REFUSAL_CODE: &str = "browser-tier-refusal";

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
        by_name
            .entry((def.name.as_str(), def.scope.block_idx))
            .or_default()
            .push(def);
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
                code: Some(lsp_types::NumberOrString::String(
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
    let has_main = index.block_spans.iter().any(|(_, block_type, _)| {
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
        let called = index
            .references
            .iter()
            .any(|r| r.definition_idx == Some(def_idx) && r.span.start != def.span.start);
        if called {
            continue;
        }
        hints.push(Diagnostic {
            range: Range {
                start: line_index.position(def.span.start),
                end: line_index.position(def.span.end),
            },
            severity: Some(DiagnosticSeverity::HINT),
            code: Some(lsp_types::NumberOrString::String(
                "unused-function".to_string(),
            )),
            code_description: code_description_for("7-functions--closures"),
            source: Some("logicaffeine".to_string()),
            message: format!(
                "'{name}' is never called.\n\nNothing in this program calls '{name}'. \
                 Is it wired up yet, or left over from a refactor?",
                name = def.name
            ),
            tags: Some(vec![lsp_types::DiagnosticTag::UNNECESSARY]),
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
            let cause_span =
                find_cause_keyword_span(tokens, TokenType::Give, name, interner, error.span.start)?;
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
        // ★ A DIAGNOSTIC ABOUT A NAME THE COMPILER WROTE IS NOISE, NOT HELP.
        //
        // The escape and ownership checks run over the LOWERED program, and an LGX page lowers
        // into synthesized functions the author never sees. Opening any page with a handler put
        // two `use-after-move` errors on it — about `__lgx_sel`, a dispatcher parameter, with the
        // whole file as their range (the synthesized statements carry no source span, so the
        // fallback is everything) and the advice "Use Show instead". Nothing in that sentence
        // names anything the author can find, let alone change.
        //
        // The predicate lives in the module that MINTS the names, so this cannot drift from it.
        .filter(|e| !logicaffeine_language::parser::lgx_lower::is_compiler_written(&e.variable))
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
        (
            line_index.position(span.start),
            line_index.position(span.end),
        )
    } else {
        // Fallback: point to start of document
        let zero = lsp_types::Position {
            line: 0,
            character: 0,
        };
        (zero, zero)
    };

    // Build related information pointing to the cause (Give statement, Zone entry, etc.)
    let use_offset = primary.map(|s| s.start).unwrap_or(usize::MAX);
    let related_information = uri.and_then(|u| {
        build_related_information(error, tokens, interner, line_index, u, use_offset)
    });

    Diagnostic {
        range: Range { start, end },
        severity: Some(DiagnosticSeverity::ERROR),
        code: Some(lsp_types::NumberOrString::String(error.code.to_string())),
        code_description: docs_anchor_for_analysis_code(error.code).and_then(code_description_for),
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
fn code_description_for(anchor: &str) -> Option<lsp_types::CodeDescription> {
    Url::parse(&crate::teach_md::guide_url(anchor))
        .ok()
        .map(|href| lsp_types::CodeDescription { href })
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
    ErrorDecision {
        severity,
        code,
        quickfix,
        docs,
    }
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_lsp/diagnostics/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_lsp/diagnostics/squiggle_anchor_tests.rs"]
mod squiggle_anchor_tests;
