//! Error types for program extraction.

use std::fmt;

/// Errors that can occur during extraction.
#[derive(Debug)]
pub enum ExtractError {
    /// Name not found in context.
    NotFound(String),

    /// Cannot extract this kind of term (e.g., Prop).
    NotExtractable { name: String, reason: String },

    /// Internal extraction error.
    Internal(String),
}

impl fmt::Display for ExtractError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExtractError::NotFound(name) => write!(f, "Not found: {}", name),
            ExtractError::NotExtractable { name, reason } => {
                write!(f, "Cannot extract '{}': {}", name, reason)
            }
            ExtractError::Internal(msg) => write!(f, "Internal error: {}", msg),
        }
    }
}

impl std::error::Error for ExtractError {}
