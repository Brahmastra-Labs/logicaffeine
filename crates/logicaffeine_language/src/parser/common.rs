//! Common parser utilities and constants shared across parser submodules.
//!
//! This module provides shared definitions used by multiple parser submodules,
//! including the list of copula verbs (is, are, was, were) for grammatical checks.

use crate::token::TokenType;

/// Copula verb tokens for copula checking in the parser.
pub const COPULAS: &[TokenType] = &[
    TokenType::Is,
    TokenType::Are,
    TokenType::Was,
    TokenType::Were,
];
