//! The canonical machine-readable diagnostic surface: `logos-diagnostic/1`.
//!
//! One [`Diagnostic`] JSON shape, produced here and consumed identically by
//! `largo check --json`, `largo run --json`, the MCP server, and (for code and
//! severity identity) the language server. Every [`ParseErrorKind`] variant
//! decides its stable kebab-case [`code`](ParseErrorKind::code), its
//! [`severity`](ParseErrorKind::severity), and a one-line
//! [`headline`](ParseErrorKind::headline) in wildcard-free matches — a new
//! error kind does not compile until it has all three.
//!
//! The [`catalog`] enumerates one rendered sample per variant and is the
//! single source for the generated error catalog and `largo explain`.

use serde::{Deserialize, Serialize};

use logicaffeine_base::Interner;

use crate::error::{
    describe_token, pronoun_word, socratic_explanation, ParseError, ParseErrorKind,
};

/// How severely a diagnostic should be presented.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagSeverity {
    Error,
    Warning,
    Info,
}

impl DiagSeverity {
    /// The wire spelling used in `logos-diagnostic/1` payloads.
    pub fn as_str(self) -> &'static str {
        match self {
            DiagSeverity::Error => "error",
            DiagSeverity::Warning => "warning",
            DiagSeverity::Info => "info",
        }
    }
}

/// A source span in both byte offsets and 1-based, char-counted line/column.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiagSpan {
    pub start: usize,
    pub end: usize,
    pub start_line: usize,
    pub start_col: usize,
    pub end_line: usize,
    pub end_col: usize,
}

/// A secondary location attached to a diagnostic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelatedInfo {
    pub file: Option<String>,
    pub span: DiagSpan,
    pub message: String,
}

/// One `logos-diagnostic/1` object.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Diagnostic {
    /// Stable kebab-case identity, e.g. `"undefined-variable"`.
    pub code: String,
    /// `"error"` | `"warning"` | `"info"`.
    pub severity: String,
    /// The file the caller attributes the source to; `None` for snippets.
    pub file: Option<String>,
    pub span: DiagSpan,
    /// One-line headline.
    pub message: String,
    /// The full Socratic explanation prose.
    pub explanation: String,
    /// "did you mean '…'?" when a close known word exists.
    pub suggestion: Option<String>,
    /// The mechanical fix note, when one is safe.
    pub quickfix: Option<String>,
    /// A quickguide URL, when a lesson teaches this construct.
    pub docs: Option<String>,
    pub related: Vec<RelatedInfo>,
}

/// One rendered sample per [`ParseErrorKind`] variant — the source of the
/// generated error catalog and `largo explain <code>`.
#[derive(Debug, Clone, PartialEq)]
pub struct CatalogEntry {
    /// The enum variant name, e.g. `"UnexpectedToken"`.
    pub variant: &'static str,
    pub code: &'static str,
    pub severity: &'static str,
    pub headline: String,
    pub explanation: String,
    pub quickfix: Option<&'static str>,
    pub docs: Option<String>,
}

impl ParseErrorKind {
    /// The stable kebab-case diagnostic code.
    ///
    /// Codes are the kebab-cased variant name, with the collapses the language
    /// server already shipped kept intact: both type-mismatch variants share
    /// `"type-mismatch"`.
    pub fn code(&self) -> &'static str {
        match self {
            ParseErrorKind::UnexpectedToken { .. } => "unexpected-token",
            ParseErrorKind::ExpectedContentWord { .. } => "expected-content-word",
            ParseErrorKind::ExpectedCopula => "expected-copula",
            ParseErrorKind::UnknownQuantifier { .. } => "unknown-quantifier",
            ParseErrorKind::UnknownModal { .. } => "unknown-modal",
            ParseErrorKind::ExpectedVerb { .. } => "expected-verb",
            ParseErrorKind::ExpectedTemporalAdverb => "expected-temporal-adverb",
            ParseErrorKind::ExpectedPresuppositionTrigger => "expected-presupposition-trigger",
            ParseErrorKind::ExpectedFocusParticle => "expected-focus-particle",
            ParseErrorKind::ExpectedScopalAdverb => "expected-scopal-adverb",
            ParseErrorKind::ExpectedSuperlativeAdjective => "expected-superlative-adjective",
            ParseErrorKind::ExpectedComparativeAdjective => "expected-comparative-adjective",
            ParseErrorKind::ExpectedThan => "expected-than",
            ParseErrorKind::ExpectedNumber => "expected-number",
            ParseErrorKind::EmptyRestriction => "empty-restriction",
            ParseErrorKind::GappingResolutionFailed => "gapping-resolution-failed",
            ParseErrorKind::StativeProgressiveConflict => "stative-progressive-conflict",
            ParseErrorKind::UndefinedVariable { .. } => "undefined-variable",
            ParseErrorKind::UseAfterMove { .. } => "use-after-move",
            ParseErrorKind::IsValueEquality { .. } => "is-value-equality",
            ParseErrorKind::ZeroIndex => "zero-index",
            ParseErrorKind::ExpectedStatement => "expected-statement",
            ParseErrorKind::ExpectedKeyword { .. } => "expected-keyword",
            ParseErrorKind::ExpectedExpression => "expected-expression",
            ParseErrorKind::ExpectedIdentifier => "expected-identifier",
            ParseErrorKind::RespectivelyLengthMismatch { .. } => "respectively-length-mismatch",
            ParseErrorKind::TypeMismatch { .. } | ParseErrorKind::TypeMismatchDetailed { .. } => {
                "type-mismatch"
            }
            ParseErrorKind::InfiniteType { .. } => "infinite-type",
            ParseErrorKind::ArityMismatch { .. } => "arity-mismatch",
            ParseErrorKind::FieldNotFound { .. } => "field-not-found",
            ParseErrorKind::NotAFunction { .. } => "not-a-function",
            ParseErrorKind::InvalidRefinementPredicate => "invalid-refinement-predicate",
            ParseErrorKind::GrammarError(_) => "grammar-error",
            ParseErrorKind::ScopeViolation(_) => "scope-violation",
            ParseErrorKind::UnresolvedPronoun { .. } => "unresolved-pronoun",
            ParseErrorKind::TrailingTokens { .. } => "trailing-tokens",
            ParseErrorKind::AstTooDeep { .. } => "ast-too-deep",
            ParseErrorKind::Custom(_) => "custom",
        }
    }

    /// The presentation severity, agreeing with the LSP decision table.
    pub fn severity(&self) -> DiagSeverity {
        match self {
            // Style: valid programs that read wrong.
            ParseErrorKind::IsValueEquality { .. } | ParseErrorKind::GrammarError(_) => {
                DiagSeverity::Warning
            }
            // Idiom: 1-based indexing is a convention, not a failure.
            ParseErrorKind::ZeroIndex => DiagSeverity::Info,
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
            | ParseErrorKind::EmptyRestriction
            | ParseErrorKind::GappingResolutionFailed
            | ParseErrorKind::StativeProgressiveConflict
            | ParseErrorKind::UndefinedVariable { .. }
            | ParseErrorKind::UseAfterMove { .. }
            | ParseErrorKind::ExpectedStatement
            | ParseErrorKind::ExpectedKeyword { .. }
            | ParseErrorKind::ExpectedExpression
            | ParseErrorKind::ExpectedIdentifier
            | ParseErrorKind::RespectivelyLengthMismatch { .. }
            | ParseErrorKind::TypeMismatch { .. }
            | ParseErrorKind::TypeMismatchDetailed { .. }
            | ParseErrorKind::InfiniteType { .. }
            | ParseErrorKind::ArityMismatch { .. }
            | ParseErrorKind::FieldNotFound { .. }
            | ParseErrorKind::NotAFunction { .. }
            | ParseErrorKind::InvalidRefinementPredicate
            | ParseErrorKind::ScopeViolation(_)
            | ParseErrorKind::UnresolvedPronoun { .. }
            | ParseErrorKind::TrailingTokens { .. }
            | ParseErrorKind::AstTooDeep { .. }
            | ParseErrorKind::Custom(_) => DiagSeverity::Error,
        }
    }

    /// A one-line headline naming what went wrong, without the Socratic prose.
    pub fn headline(&self) -> String {
        match self {
            ParseErrorKind::UnexpectedToken { expected, found } => format!(
                "expected {}, found {}",
                describe_token(expected),
                describe_token(found)
            ),
            ParseErrorKind::ExpectedContentWord { found } => {
                format!("expected a content word, found {}", describe_token(found))
            }
            ParseErrorKind::ExpectedCopula => "expected 'is' or 'are'".to_string(),
            ParseErrorKind::UnknownQuantifier { found } => {
                format!("expected a quantifier, found {}", describe_token(found))
            }
            ParseErrorKind::UnknownModal { found } => {
                format!("expected a modal, found {}", describe_token(found))
            }
            ParseErrorKind::ExpectedVerb { found } => {
                format!("expected a verb, found {}", describe_token(found))
            }
            ParseErrorKind::ExpectedTemporalAdverb => "expected a temporal adverb".to_string(),
            ParseErrorKind::ExpectedPresuppositionTrigger => {
                "expected a presupposition trigger".to_string()
            }
            ParseErrorKind::ExpectedFocusParticle => "expected a focus particle".to_string(),
            ParseErrorKind::ExpectedScopalAdverb => "expected a scopal adverb".to_string(),
            ParseErrorKind::ExpectedSuperlativeAdjective => {
                "expected a superlative adjective".to_string()
            }
            ParseErrorKind::ExpectedComparativeAdjective => {
                "expected a comparative adjective".to_string()
            }
            ParseErrorKind::ExpectedThan => "expected 'than' to complete the comparison".to_string(),
            ParseErrorKind::ExpectedNumber => "expected a number".to_string(),
            ParseErrorKind::EmptyRestriction => "empty relative clause".to_string(),
            ParseErrorKind::GappingResolutionFailed => {
                "no verb to borrow for the gapped clause".to_string()
            }
            ParseErrorKind::StativeProgressiveConflict => {
                "stative verb used in the progressive".to_string()
            }
            ParseErrorKind::UndefinedVariable { name } => format!("'{name}' is not declared"),
            ParseErrorKind::UseAfterMove { name } => {
                format!("cannot use '{name}' after giving it away")
            }
            ParseErrorKind::IsValueEquality { variable, value } => {
                format!("'{variable} is {value}' compares values; use 'equals'")
            }
            ParseErrorKind::ZeroIndex => "indices start at 1; there is no item 0".to_string(),
            ParseErrorKind::ExpectedStatement => "expected a statement".to_string(),
            ParseErrorKind::ExpectedKeyword { keyword } => {
                format!("expected the word '{keyword}'")
            }
            ParseErrorKind::ExpectedExpression => "expected an expression".to_string(),
            ParseErrorKind::ExpectedIdentifier => "expected a name".to_string(),
            ParseErrorKind::RespectivelyLengthMismatch { subject_count, object_count } => format!(
                "'respectively' lists differ: {subject_count} subject(s) vs {object_count} object(s)"
            ),
            ParseErrorKind::TypeMismatch { expected, found } => {
                format!("expected '{expected}', found '{found}'")
            }
            ParseErrorKind::TypeMismatchDetailed { expected, found, context } => {
                if context.is_empty() {
                    format!("expected '{expected}', found '{found}'")
                } else {
                    format!("expected '{expected}', found '{found}' ({context})")
                }
            }
            ParseErrorKind::InfiniteType { var_description, .. } => {
                format!("infinite type: {var_description} would contain itself")
            }
            ParseErrorKind::ArityMismatch { function, expected, found } => {
                format!("'{function}' takes {expected} argument(s), but this call passes {found}")
            }
            ParseErrorKind::FieldNotFound { type_name, field_name, .. } => {
                format!("'{type_name}' has no field named '{field_name}'")
            }
            ParseErrorKind::NotAFunction { found_type } => {
                format!("cannot call a value of type '{found_type}'")
            }
            ParseErrorKind::InvalidRefinementPredicate => {
                "refinement predicate is not checkable".to_string()
            }
            ParseErrorKind::GrammarError(msg) => format!("grammar: {msg}"),
            ParseErrorKind::ScopeViolation(msg) => format!("scope: {msg}"),
            ParseErrorKind::UnresolvedPronoun { gender, number } => {
                format!("pronoun {} has no antecedent", pronoun_word(gender, number))
            }
            ParseErrorKind::TrailingTokens { found } => {
                format!("input left over, beginning with {}", describe_token(found))
            }
            ParseErrorKind::AstTooDeep { depth, max_depth } => {
                format!("AST nests {depth} levels deep; the limit is {max_depth}")
            }
            ParseErrorKind::Custom(msg) => msg.clone(),
        }
    }

    /// The variant's name, for catalog keys and golden stanza headers.
    pub fn variant_name(&self) -> &'static str {
        match self {
            ParseErrorKind::UnexpectedToken { .. } => "UnexpectedToken",
            ParseErrorKind::ExpectedContentWord { .. } => "ExpectedContentWord",
            ParseErrorKind::ExpectedCopula => "ExpectedCopula",
            ParseErrorKind::UnknownQuantifier { .. } => "UnknownQuantifier",
            ParseErrorKind::UnknownModal { .. } => "UnknownModal",
            ParseErrorKind::ExpectedVerb { .. } => "ExpectedVerb",
            ParseErrorKind::ExpectedTemporalAdverb => "ExpectedTemporalAdverb",
            ParseErrorKind::ExpectedPresuppositionTrigger => "ExpectedPresuppositionTrigger",
            ParseErrorKind::ExpectedFocusParticle => "ExpectedFocusParticle",
            ParseErrorKind::ExpectedScopalAdverb => "ExpectedScopalAdverb",
            ParseErrorKind::ExpectedSuperlativeAdjective => "ExpectedSuperlativeAdjective",
            ParseErrorKind::ExpectedComparativeAdjective => "ExpectedComparativeAdjective",
            ParseErrorKind::ExpectedThan => "ExpectedThan",
            ParseErrorKind::ExpectedNumber => "ExpectedNumber",
            ParseErrorKind::EmptyRestriction => "EmptyRestriction",
            ParseErrorKind::GappingResolutionFailed => "GappingResolutionFailed",
            ParseErrorKind::StativeProgressiveConflict => "StativeProgressiveConflict",
            ParseErrorKind::UndefinedVariable { .. } => "UndefinedVariable",
            ParseErrorKind::UseAfterMove { .. } => "UseAfterMove",
            ParseErrorKind::IsValueEquality { .. } => "IsValueEquality",
            ParseErrorKind::ZeroIndex => "ZeroIndex",
            ParseErrorKind::ExpectedStatement => "ExpectedStatement",
            ParseErrorKind::ExpectedKeyword { .. } => "ExpectedKeyword",
            ParseErrorKind::ExpectedExpression => "ExpectedExpression",
            ParseErrorKind::ExpectedIdentifier => "ExpectedIdentifier",
            ParseErrorKind::RespectivelyLengthMismatch { .. } => "RespectivelyLengthMismatch",
            ParseErrorKind::TypeMismatch { .. } => "TypeMismatch",
            ParseErrorKind::TypeMismatchDetailed { .. } => "TypeMismatchDetailed",
            ParseErrorKind::InfiniteType { .. } => "InfiniteType",
            ParseErrorKind::ArityMismatch { .. } => "ArityMismatch",
            ParseErrorKind::FieldNotFound { .. } => "FieldNotFound",
            ParseErrorKind::NotAFunction { .. } => "NotAFunction",
            ParseErrorKind::InvalidRefinementPredicate => "InvalidRefinementPredicate",
            ParseErrorKind::GrammarError(_) => "GrammarError",
            ParseErrorKind::ScopeViolation(_) => "ScopeViolation",
            ParseErrorKind::UnresolvedPronoun { .. } => "UnresolvedPronoun",
            ParseErrorKind::TrailingTokens { .. } => "TrailingTokens",
            ParseErrorKind::AstTooDeep { .. } => "AstTooDeep",
            ParseErrorKind::Custom(_) => "Custom",
        }
    }

    /// The mechanical fix note the LSP offers, when one is safe.
    pub fn quickfix_note(&self) -> Option<&'static str> {
        match self {
            ParseErrorKind::IsValueEquality { .. } => Some("Use 'equals' for value comparison"),
            ParseErrorKind::ZeroIndex => Some("Use 1-based indexing"),
            ParseErrorKind::UseAfterMove { .. } => Some("Use 'a copy of …' instead"),
            ParseErrorKind::UndefinedVariable { .. } => {
                Some("Did you mean '<nearest definition>'?")
            }
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
            | ParseErrorKind::EmptyRestriction
            | ParseErrorKind::GappingResolutionFailed
            | ParseErrorKind::StativeProgressiveConflict
            | ParseErrorKind::ExpectedStatement
            | ParseErrorKind::ExpectedKeyword { .. }
            | ParseErrorKind::ExpectedExpression
            | ParseErrorKind::ExpectedIdentifier
            | ParseErrorKind::RespectivelyLengthMismatch { .. }
            | ParseErrorKind::TypeMismatch { .. }
            | ParseErrorKind::TypeMismatchDetailed { .. }
            | ParseErrorKind::InfiniteType { .. }
            | ParseErrorKind::ArityMismatch { .. }
            | ParseErrorKind::FieldNotFound { .. }
            | ParseErrorKind::NotAFunction { .. }
            | ParseErrorKind::InvalidRefinementPredicate
            | ParseErrorKind::GrammarError(_)
            | ParseErrorKind::ScopeViolation(_)
            | ParseErrorKind::UnresolvedPronoun { .. }
            | ParseErrorKind::TrailingTokens { .. }
            | ParseErrorKind::AstTooDeep { .. }
            | ParseErrorKind::Custom(_) => None,
        }
    }

    /// The quickguide heading slug, when a lesson teaches this construct.
    pub fn docs_anchor(&self) -> Option<&'static str> {
        match self {
            ParseErrorKind::IsValueEquality { .. } => Some("3-arithmetic-comparison-logic-bitwise"),
            ParseErrorKind::ZeroIndex => Some("5-collections"),
            ParseErrorKind::UseAfterMove { .. } => Some("13-output"),
            ParseErrorKind::UndefinedVariable { .. } => Some("2-variables--mutation"),
            ParseErrorKind::TypeMismatch { .. } | ParseErrorKind::TypeMismatchDetailed { .. } => {
                Some("2-variables--mutation")
            }
            ParseErrorKind::ArityMismatch { .. } | ParseErrorKind::NotAFunction { .. } => {
                Some("7-functions--closures")
            }
            ParseErrorKind::FieldNotFound { .. } => Some("8-structs-enums--field-access"),
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
            | ParseErrorKind::EmptyRestriction
            | ParseErrorKind::GappingResolutionFailed
            | ParseErrorKind::StativeProgressiveConflict
            | ParseErrorKind::ExpectedStatement
            | ParseErrorKind::ExpectedKeyword { .. }
            | ParseErrorKind::ExpectedExpression
            | ParseErrorKind::ExpectedIdentifier
            | ParseErrorKind::RespectivelyLengthMismatch { .. }
            | ParseErrorKind::InfiniteType { .. }
            | ParseErrorKind::InvalidRefinementPredicate
            | ParseErrorKind::GrammarError(_)
            | ParseErrorKind::ScopeViolation(_)
            | ParseErrorKind::UnresolvedPronoun { .. }
            | ParseErrorKind::TrailingTokens { .. }
            | ParseErrorKind::AstTooDeep { .. }
            | ParseErrorKind::Custom(_) => None,
        }
    }
}

/// The quickguide URL for an anchor — same rendering the LSP ships in
/// `codeDescription`.
pub fn guide_url(anchor: &str) -> String {
    format!(
        "https://github.com/Brahmastra-Labs/logicaffeine/blob/main/LOGOS_QUICKGUIDE.md#{anchor}"
    )
}

/// 1-based `(line, column)` for a byte offset; columns count characters.
pub fn line_col(source: &str, byte: usize) -> (usize, usize) {
    let mut line = 1usize;
    let mut col = 1usize;
    for (i, c) in source.char_indices() {
        if i >= byte {
            break;
        }
        if c == '\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }
    (line, col)
}

/// Render one parse error into the canonical `logos-diagnostic/1` shape.
pub fn diagnostic_for(
    error: &ParseError,
    source: &str,
    file: Option<&str>,
    interner: &Interner,
) -> Diagnostic {
    let (start_line, start_col) = line_col(source, error.span.start);
    let (end_line, end_col) = line_col(source, error.span.end);
    Diagnostic {
        code: error.kind.code().to_string(),
        severity: error.kind.severity().as_str().to_string(),
        file: file.map(str::to_string),
        span: DiagSpan {
            start: error.span.start,
            end: error.span.end,
            start_line,
            start_col,
            end_line,
            end_col,
        },
        message: error.kind.headline(),
        explanation: socratic_explanation(error, interner),
        suggestion: error
            .suggestion(source)
            .map(|word| format!("did you mean '{word}'?")),
        quickfix: error.kind.quickfix_note().map(str::to_string),
        docs: error.kind.docs_anchor().map(guide_url),
        related: Vec::new(),
    }
}

/// One representative `(variant name, kind)` per [`ParseErrorKind`] variant.
///
/// This is the canonical enumeration surface: the catalog, the schema lock,
/// and the LSP agreement lock all iterate it.
pub fn sample_kinds() -> Vec<(&'static str, ParseErrorKind)> {
    use crate::drs::{Gender, Number};
    use crate::token::TokenType;

    let kinds = vec![
        ParseErrorKind::UnexpectedToken {
            expected: TokenType::Period,
            found: TokenType::Comma,
        },
        ParseErrorKind::ExpectedContentWord {
            found: TokenType::Comma,
        },
        ParseErrorKind::ExpectedCopula,
        ParseErrorKind::UnknownQuantifier {
            found: TokenType::Comma,
        },
        ParseErrorKind::UnknownModal {
            found: TokenType::Comma,
        },
        ParseErrorKind::ExpectedVerb {
            found: TokenType::Comma,
        },
        ParseErrorKind::ExpectedTemporalAdverb,
        ParseErrorKind::ExpectedPresuppositionTrigger,
        ParseErrorKind::ExpectedFocusParticle,
        ParseErrorKind::ExpectedScopalAdverb,
        ParseErrorKind::ExpectedSuperlativeAdjective,
        ParseErrorKind::ExpectedComparativeAdjective,
        ParseErrorKind::ExpectedThan,
        ParseErrorKind::ExpectedNumber,
        ParseErrorKind::EmptyRestriction,
        ParseErrorKind::GappingResolutionFailed,
        ParseErrorKind::StativeProgressiveConflict,
        ParseErrorKind::UndefinedVariable {
            name: "x".to_string(),
        },
        ParseErrorKind::UseAfterMove {
            name: "x".to_string(),
        },
        ParseErrorKind::IsValueEquality {
            variable: "x".to_string(),
            value: "5".to_string(),
        },
        ParseErrorKind::ZeroIndex,
        ParseErrorKind::ExpectedStatement,
        ParseErrorKind::ExpectedKeyword {
            keyword: "to".to_string(),
        },
        ParseErrorKind::ExpectedExpression,
        ParseErrorKind::ExpectedIdentifier,
        ParseErrorKind::RespectivelyLengthMismatch {
            subject_count: 2,
            object_count: 3,
        },
        ParseErrorKind::TypeMismatch {
            expected: "Int".to_string(),
            found: "Text".to_string(),
        },
        ParseErrorKind::TypeMismatchDetailed {
            expected: "Int".to_string(),
            found: "Text".to_string(),
            context: "in argument 2 of 'compute'".to_string(),
        },
        ParseErrorKind::InfiniteType {
            var_description: "T".to_string(),
            type_description: "Seq of T".to_string(),
        },
        ParseErrorKind::ArityMismatch {
            function: "greet".to_string(),
            expected: 2,
            found: 1,
        },
        ParseErrorKind::FieldNotFound {
            type_name: "Point".to_string(),
            field_name: "z".to_string(),
            available: vec!["x".to_string(), "y".to_string()],
        },
        ParseErrorKind::NotAFunction {
            found_type: "Int".to_string(),
        },
        ParseErrorKind::InvalidRefinementPredicate,
        ParseErrorKind::GrammarError("its vs it's".to_string()),
        ParseErrorKind::ScopeViolation("pronoun trapped in negation".to_string()),
        ParseErrorKind::UnresolvedPronoun {
            gender: Gender::Female,
            number: Number::Singular,
        },
        ParseErrorKind::TrailingTokens {
            found: TokenType::Comma,
        },
        ParseErrorKind::AstTooDeep {
            depth: 3000,
            max_depth: 2048,
        },
        ParseErrorKind::Custom("zone escape".to_string()),
    ];

    kinds
        .into_iter()
        .map(|kind| (kind.variant_name(), kind))
        .collect()
}

/// The full rendered catalog, one entry per variant.
pub fn catalog() -> Vec<CatalogEntry> {
    let interner = Interner::new();
    sample_kinds()
        .into_iter()
        .map(|(variant, kind)| {
            let error = ParseError {
                kind,
                span: crate::token::Span::new(0, 0),
            };
            CatalogEntry {
                variant,
                code: error.kind.code(),
                severity: error.kind.severity().as_str(),
                headline: error.kind.headline(),
                explanation: socratic_explanation(&error, &interner),
                quickfix: error.kind.quickfix_note(),
                docs: error.kind.docs_anchor().map(guide_url),
            }
        })
        .collect()
}

/// Catalog entries grouped by code, groups in first-appearance (enum) order.
fn catalog_by_code(entries: &[CatalogEntry]) -> Vec<(&'static str, Vec<&CatalogEntry>)> {
    let mut order: Vec<&'static str> = Vec::new();
    let mut groups: std::collections::BTreeMap<&'static str, Vec<&CatalogEntry>> =
        std::collections::BTreeMap::new();
    for entry in entries {
        if !groups.contains_key(entry.code) {
            order.push(entry.code);
        }
        groups.entry(entry.code).or_default().push(entry);
    }
    order
        .into_iter()
        .map(|code| (code, groups[code].clone()))
        .collect()
}

/// The catalog as the reference guide `docs/ERROR_CATALOG.md`: one section per code (variants
/// sharing a code share a section), each sample headline fenced as `text`, never as `logos`.
pub fn catalog_markdown() -> String {
    let entries = catalog();
    let mut out = String::new();
    out.push_str("# LOGOS Error Catalog\n\n");
    out.push_str(
        "Every diagnostic the LOGOS toolchain emits, keyed by its stable\n\
         `logos-diagnostic/1` code — the `code` field of `largo check --json`,\n\
         `largo run --json`, and the MCP `logos_check` tool. Look a code up here\n\
         or with `largo explain <code>`.\n\n\
         Generated from `logicaffeine_language::diagnostic::catalog()`. Do not\n\
         edit by hand; run `./scripts/release/gen-error-catalog.sh` after changing\n\
         `ParseErrorKind`.\n",
    );
    for (code, group) in catalog_by_code(&entries) {
        out.push_str(&format!("\n## `{code}`\n\n"));
        let severity = group[0].severity;
        out.push_str(&format!("- **Severity**: {severity}\n"));
        let variants: Vec<&str> = group.iter().map(|entry| entry.variant).collect();
        out.push_str(&format!("- **Variant**: `{}`\n", variants.join("`, `")));
        if let Some(quickfix) = group[0].quickfix {
            out.push_str(&format!("- **Quickfix**: {quickfix}\n"));
        }
        if let Some(docs) = &group[0].docs {
            out.push_str(&format!("- **Guide**: <{docs}>\n"));
        }
        for entry in &group {
            out.push_str("\n**Example**:\n\n```text\n");
            out.push_str(&entry.headline);
            out.push_str("\n```\n\n");
            out.push_str(&format!("> {}\n", entry.explanation));
        }
    }
    out
}

/// The catalog as `errors.json` (schema `logos-errors/1`): one object per variant, pretty-printed,
/// newline-terminated.
pub fn catalog_json() -> String {
    let items: Vec<serde_json::Value> = catalog()
        .iter()
        .map(|entry| {
            serde_json::json!({
                "code": entry.code,
                "severity": entry.severity,
                "variant": entry.variant,
                "headline": entry.headline,
                "explanation": entry.explanation,
                "quickfix": entry.quickfix,
                "docs": entry.docs,
            })
        })
        .collect();
    let doc = serde_json::json!({
        "schema": "logos-errors/1",
        "errors": items,
    });
    let mut text = serde_json::to_string_pretty(&doc).expect("the catalog serializes");
    text.push('\n');
    text
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_language/diagnostic/tests.rs"]
mod tests;
