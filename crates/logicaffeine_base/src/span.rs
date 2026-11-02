//! Source location tracking for error reporting.
//!
//! A [`Span`] represents a contiguous region of source text using byte offsets.
//! Every token, expression, and error in logicaffeine carries a span, enabling
//! precise error messages that point to the exact location of problems.
//!
//! # Byte Offsets
//!
//! Spans use byte offsets, not character indices. This matches Rust's string
//! slicing semantics: `&source[span.start..span.end]` extracts the spanned text.
//!
//! # Example
//!
//! ```
//! use logicaffeine_base::Span;
//!
//! let source = "hello world";
//! let span = Span::new(0, 5);
//!
//! assert_eq!(&source[span.start..span.end], "hello");
//! assert_eq!(span.len(), 5);
//! ```

/// A byte-offset range in source text.
///
/// Spans are `Copy` and cheap to pass around. Use [`Span::merge`] to combine
/// spans when building compound expressions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Span {
    /// Byte offset of the first character (inclusive).
    pub start: usize,
    /// Byte offset past the last character (exclusive).
    pub end: usize,
}

impl Span {
    /// Creates a span from byte offsets.
    ///
    /// No validation is performed; `start` may exceed `end`.
    pub fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }

    /// Creates a span covering from the start of `self` to the end of `other`.
    ///
    /// Useful for building compound expressions: the span of `a + b` is
    /// `a.span.merge(b.span)`.
    pub fn merge(self, other: Span) -> Span {
        Span {
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        }
    }

    /// Returns the length of the span in bytes.
    pub fn len(&self) -> usize {
        self.end.saturating_sub(self.start)
    }

    /// Returns `true` if this span covers no bytes.
    pub fn is_empty(&self) -> bool {
        self.start >= self.end
    }
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_base/span/tests.rs"]
mod tests;
