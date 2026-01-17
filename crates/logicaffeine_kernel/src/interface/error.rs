//! Error types for the Vernacular interface.

use crate::KernelError;
use std::fmt;

/// Errors that can occur in the interface layer.
#[derive(Debug)]
pub enum InterfaceError {
    /// Parse error in command or term syntax.
    Parse(ParseError),

    /// Error from the kernel (type checking, etc.)
    Kernel(KernelError),
}

/// Parse errors for the Vernacular.
#[derive(Debug, Clone)]
pub enum ParseError {
    /// Unexpected end of input.
    UnexpectedEof,

    /// Unknown command keyword.
    UnknownCommand(String),

    /// Expected a specific token.
    Expected { expected: String, found: String },

    /// Invalid identifier.
    InvalidIdent(String),

    /// Invalid number literal.
    InvalidNumber(String),

    /// Missing required component.
    Missing(String),

    // ============================================================
    // Literate Syntax Errors
    // ============================================================

    /// Missing "is either" clause in inductive definition.
    MissingEitherClause,

    /// Missing variant name in inductive definition.
    MissingVariantName,

    /// Invalid Consider/When/Yield syntax.
    InvalidConsiderSyntax(String),

    /// Missing Yield in function body.
    MissingYield,

    /// Missing When clause in Consider block.
    MissingWhenClause,

    /// Unclosed Consider block.
    UnclosedConsiderBlock,
}

impl fmt::Display for InterfaceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InterfaceError::Parse(e) => write!(f, "Parse error: {}", e),
            InterfaceError::Kernel(e) => write!(f, "Kernel error: {}", e),
        }
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::UnexpectedEof => write!(f, "Unexpected end of input"),
            ParseError::UnknownCommand(cmd) => write!(f, "Unknown command: {}", cmd),
            ParseError::Expected { expected, found } => {
                write!(f, "Expected {}, found {}", expected, found)
            }
            ParseError::InvalidIdent(s) => write!(f, "Invalid identifier: {}", s),
            ParseError::InvalidNumber(s) => write!(f, "Invalid number literal: {}", s),
            ParseError::Missing(what) => write!(f, "Missing {}", what),
            // Literate syntax errors
            ParseError::MissingEitherClause => write!(f, "Missing 'is either' clause in type definition"),
            ParseError::MissingVariantName => write!(f, "Missing variant name in type definition"),
            ParseError::InvalidConsiderSyntax(msg) => write!(f, "Invalid Consider syntax: {}", msg),
            ParseError::MissingYield => write!(f, "Missing 'Yield' in function body"),
            ParseError::MissingWhenClause => write!(f, "Missing 'When' clause in Consider block"),
            ParseError::UnclosedConsiderBlock => write!(f, "Unclosed Consider block"),
        }
    }
}

impl std::error::Error for InterfaceError {}
impl std::error::Error for ParseError {}

impl From<ParseError> for InterfaceError {
    fn from(e: ParseError) -> Self {
        InterfaceError::Parse(e)
    }
}

impl From<KernelError> for InterfaceError {
    fn from(e: KernelError) -> Self {
        InterfaceError::Kernel(e)
    }
}
