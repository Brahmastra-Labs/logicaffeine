//! Offsets inside a captured section body, mapped back to the file.
//!
//! The lexer captures a `View:`, `Head:` or `Style:` body as one raw block: its text with each
//! line's indentation stripped, where its first character sits in the file, and how much
//! indentation it stripped. Every stage that reports a position inside such a body — the template
//! parser, the sub-lexed expressions, the lint, the stylesheet scoper — speaks in offsets into the
//! dedented text, and [`RawMap`] is the one place those become file offsets.

use crate::token::Span;

/// A dedented raw body and where it came from.
#[derive(Debug, Clone)]
pub struct RawMap {
    pub text: String,
    /// The indentation stripped from every non-empty line after the first.
    pub indent: usize,
    /// The file offset of the body's first character.
    pub content_start: usize,
}

impl RawMap {
    pub fn new(text: impl Into<String>, indent: usize, content_start: usize) -> Self {
        RawMap {
            text: text.into(),
            indent,
            content_start,
        }
    }

    /// The file offset of body offset `off`. Each line after the first is preceded by its newline
    /// plus the indentation the capture stripped; a line that is EMPTY in the body came from an
    /// empty line, which had none.
    pub fn offset(&self, off: usize) -> usize {
        let mut body_line_start = 0usize;
        let mut file_line_start = self.content_start;
        for (i, line) in self.text.split('\n').enumerate() {
            if i > 0 {
                file_line_start += 1 + if line.is_empty() { 0 } else { self.indent };
            }
            if off <= body_line_start + line.len() {
                return file_line_start + (off - body_line_start);
            }
            body_line_start += line.len() + 1;
            file_line_start += line.len();
        }
        file_line_start
    }

    /// A body span as a file span.
    pub fn span(&self, s: Span) -> Span {
        Span::new(self.offset(s.start), self.offset(s.end))
    }
}
