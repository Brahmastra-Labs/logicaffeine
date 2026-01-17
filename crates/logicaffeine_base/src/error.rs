//! Error types with source location tracking.
//!
//! All errors in logicaffeine carry a [`Span`] indicating where in the source
//! text the error occurred. This enables precise, contextual error messages.
//!
//! # Example
//!
//! ```
//! use logicaffeine_base::{SpannedError, Span, Result};
//!
//! fn parse_number(s: &str) -> Result<i32> {
//!     s.parse().map_err(|_| SpannedError::new(
//!         format!("invalid number: '{}'", s),
//!         Span::new(0, s.len()),
//!     ))
//! }
//!
//! let err = parse_number("abc").unwrap_err();
//! assert!(err.to_string().contains("invalid number"));
//! ```

use crate::span::Span;
use std::fmt;

/// An error annotated with its source location.
///
/// Implements [`std::error::Error`] and [`fmt::Display`]. The display format is:
/// `{message} at {start}..{end}`.
#[derive(Debug, Clone)]
pub struct SpannedError {
    /// Human-readable error description.
    pub message: String,
    /// Location in source where the error occurred.
    pub span: Span,
}

impl SpannedError {
    /// Creates an error with the given message and source location.
    pub fn new(message: impl Into<String>, span: Span) -> Self {
        Self {
            message: message.into(),
            span,
        }
    }
}

impl fmt::Display for SpannedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} at {}..{}", self.message, self.span.start, self.span.end)
    }
}

impl std::error::Error for SpannedError {}

/// Alias for `std::result::Result<T, SpannedError>`.
///
/// Use this as the return type for fallible operations in logicaffeine.
pub type Result<T> = std::result::Result<T, SpannedError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spanned_error_display() {
        let err = SpannedError::new("test error", Span::new(5, 10));
        let display = format!("{}", err);
        assert!(display.contains("test error"));
        assert!(display.contains("5..10"));
    }
}
