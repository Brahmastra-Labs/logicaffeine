//! Error types for the Vernacular interface.

use crate::kernel::KernelError;
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
