//! The shared `ParseErrorKind` enumeration surface for ratchet tests.
//!
//! One representative instance per variant, a wildcard-free guard that breaks
//! the build when the enum grows, and a wildcard-free label for rendering
//! goldens. The decision lock, the socratic corpus, and the quickfix parity
//! ratchet all iterate this list — one place to extend when the language
//! learns a new way to fail.
#![allow(dead_code)]

use logicaffeine_language::error::ParseErrorKind;
use logicaffeine_language::lexicon::{Gender, Number as GrammaticalNumber};
use logicaffeine_language::token::TokenType;

/// One entry per `ParseErrorKind` variant. `parse_error_kind_guard` breaks the
/// build when the enum grows; update the guard, this list, and
/// `ALL_PARSE_ERROR_KIND_COUNT` together.
pub const ALL_PARSE_ERROR_KIND_COUNT: usize = 39;

pub fn all_parse_error_kinds() -> Vec<ParseErrorKind> {
    vec![
        ParseErrorKind::UnexpectedToken {
            expected: TokenType::Period,
            found: TokenType::Comma,
        },
        ParseErrorKind::ExpectedContentWord { found: TokenType::Comma },
        ParseErrorKind::ExpectedCopula,
        ParseErrorKind::UnknownQuantifier { found: TokenType::Comma },
        ParseErrorKind::UnknownModal { found: TokenType::Comma },
        ParseErrorKind::ExpectedVerb { found: TokenType::Comma },
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
        ParseErrorKind::UndefinedVariable { name: "x".to_string() },
        ParseErrorKind::UseAfterMove { name: "x".to_string() },
        ParseErrorKind::IsValueEquality {
            variable: "x".to_string(),
            value: "5".to_string(),
        },
        ParseErrorKind::ZeroIndex,
        ParseErrorKind::ExpectedStatement,
        ParseErrorKind::ExpectedKeyword { keyword: "to".to_string() },
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
        ParseErrorKind::NotAFunction { found_type: "Int".to_string() },
        ParseErrorKind::InvalidRefinementPredicate,
        ParseErrorKind::GrammarError("its vs it's".to_string()),
        ParseErrorKind::ScopeViolation("pronoun trapped in negation".to_string()),
        ParseErrorKind::UnresolvedPronoun {
            gender: Gender::Female,
            number: GrammaticalNumber::Singular,
        },
        ParseErrorKind::TrailingTokens { found: TokenType::Comma },
        ParseErrorKind::AstTooDeep { depth: 3000, max_depth: 2048 },
        ParseErrorKind::Custom("zone escape".to_string()),
    ]
}

/// Wildcard-free: a new `ParseErrorKind` variant fails to compile here,
/// forcing an update to `all_parse_error_kinds`, the count, and — because
/// `decision_for` is also wildcard-free — an explicit severity/code/quickfix
/// decision in the server.
pub fn parse_error_kind_guard(kind: &ParseErrorKind) {
    match kind {
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
        | ParseErrorKind::IsValueEquality { .. }
        | ParseErrorKind::ZeroIndex
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
        | ParseErrorKind::Custom(_) => {}
    }
}

/// The variant's name, for golden stanza headers. Wildcard-free so a new
/// variant cannot ship without a label (and therefore without a golden
/// stanza).
pub fn kind_label(kind: &ParseErrorKind) -> &'static str {
    match kind {
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
