//! Source map for the diagnostic bridge.
//!
//! Maps generated Rust code back to LOGOS source positions, enabling
//! friendly error messages for ownership/lifetime errors detected by rustc.
//!
//! # Architecture
//!
//! ```text
//! LOGOS Source    CodeGen    Rust Source    rustc    Diagnostics
//!     │              │            │           │           │
//!     │ SourceMap    │            │           │           │
//!     │◄─────────────┼────────────┼───────────┼───────────┤
//!     │   line 5     │            │  line 12  │   E0382   │
//!     │   span       │            │           │   line 12 │
//!     └──────────────┴────────────┴───────────┴───────────┘
//! ```
//!
//! # Usage
//!
//! The source map is built during code generation by calling builder methods
//! at each statement. The diagnostic bridge then uses
//! [`SourceMap::find_nearest_span`] to translate rustc line numbers.

use crate::intern::Symbol;
use crate::token::Span;
use std::collections::HashMap;

/// Semantic role of a variable in LOGOS ownership semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnershipRole {
    /// The object being moved in "Give X to Y"
    GiveObject,
    /// The recipient in "Give X to Y"
    GiveRecipient,
    /// The object being borrowed in "Show X to Y"
    ShowObject,
    /// The recipient in "Show X to Y"
    ShowRecipient,
    /// A Let-bound variable
    LetBinding,
    /// Target of a Set statement
    SetTarget,
    /// Variable allocated inside a Zone
    ZoneLocal,
}

/// Variable origin tracking for error translation.
#[derive(Debug, Clone)]
pub struct VarOrigin {
    pub logos_name: Symbol,
    pub span: Span,
    pub role: OwnershipRole,
}

/// Maps generated Rust code back to LOGOS source.
#[derive(Debug, Clone, Default)]
pub struct SourceMap {
    /// Maps line in generated Rust -> Span in LOGOS source
    line_to_span: HashMap<u32, Span>,

    /// Maps generated Rust variable names -> LOGOS origin info
    var_origins: HashMap<String, VarOrigin>,

    /// The original LOGOS source code (for error display)
    logos_source: String,
}

impl SourceMap {
    /// Create a new empty source map.
    pub fn new(logos_source: String) -> Self {
        Self {
            line_to_span: HashMap::new(),
            var_origins: HashMap::new(),
            logos_source,
        }
    }

    /// Get the LOGOS span for a given Rust line number.
    pub fn get_span_for_line(&self, line: u32) -> Option<Span> {
        self.line_to_span.get(&line).copied()
    }

    /// Get the origin info for a Rust variable name.
    pub fn get_var_origin(&self, rust_var: &str) -> Option<&VarOrigin> {
        self.var_origins.get(rust_var)
    }

    /// Get the original LOGOS source.
    pub fn logos_source(&self) -> &str {
        &self.logos_source
    }

    /// Find the closest LOGOS span by searching nearby lines.
    pub fn find_nearest_span(&self, rust_line: u32) -> Option<Span> {
        // Try exact match first
        if let Some(span) = self.line_to_span.get(&rust_line) {
            return Some(*span);
        }

        // Search nearby lines (within 5 lines)
        for offset in 1..=5 {
            if rust_line > offset {
                if let Some(span) = self.line_to_span.get(&(rust_line - offset)) {
                    return Some(*span);
                }
            }
            if let Some(span) = self.line_to_span.get(&(rust_line + offset)) {
                return Some(*span);
            }
        }

        None
    }
}

/// Builder for constructing a SourceMap during code generation.
#[derive(Debug)]
pub struct SourceMapBuilder {
    current_line: u32,
    map: SourceMap,
}

impl SourceMapBuilder {
    /// Create a new builder with the LOGOS source.
    pub fn new(logos_source: &str) -> Self {
        Self {
            current_line: 1,
            map: SourceMap::new(logos_source.to_string()),
        }
    }

    /// Record a mapping from current Rust line to LOGOS span.
    pub fn record_line(&mut self, logos_span: Span) {
        self.map.line_to_span.insert(self.current_line, logos_span);
    }

    /// Record a variable origin.
    pub fn record_var(&mut self, rust_name: &str, logos_name: Symbol, span: Span, role: OwnershipRole) {
        self.map.var_origins.insert(
            rust_name.to_string(),
            VarOrigin {
                logos_name,
                span,
                role,
            },
        );
    }

    /// Advance to the next line.
    pub fn newline(&mut self) {
        self.current_line += 1;
    }

    /// Add multiple newlines.
    pub fn add_lines(&mut self, count: u32) {
        self.current_line += count;
    }

    /// Get current line number.
    pub fn current_line(&self) -> u32 {
        self.current_line
    }

    /// Build the final source map.
    pub fn build(self) -> SourceMap {
        self.map
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_map_stores_line_mappings() {
        let mut map = SourceMap::new("Let x be 5.".to_string());
        map.line_to_span.insert(1, Span::new(0, 11));

        assert_eq!(map.get_span_for_line(1), Some(Span::new(0, 11)));
        assert_eq!(map.get_span_for_line(2), None);
    }

    #[test]
    fn source_map_builder_tracks_lines() {
        let mut builder = SourceMapBuilder::new("test source");
        assert_eq!(builder.current_line(), 1);

        builder.newline();
        assert_eq!(builder.current_line(), 2);

        builder.add_lines(3);
        assert_eq!(builder.current_line(), 5);
    }

    #[test]
    fn source_map_builder_records_spans() {
        let mut builder = SourceMapBuilder::new("Let x be 5.\nLet y be 10.");
        builder.record_line(Span::new(0, 11));
        builder.newline();
        builder.record_line(Span::new(12, 24));

        let map = builder.build();
        assert_eq!(map.get_span_for_line(1), Some(Span::new(0, 11)));
        assert_eq!(map.get_span_for_line(2), Some(Span::new(12, 24)));
    }

    #[test]
    fn find_nearest_span_searches_nearby() {
        let mut builder = SourceMapBuilder::new("source");
        builder.record_line(Span::new(0, 10));
        builder.add_lines(5);
        // Line 1 has span, lines 2-6 don't

        let map = builder.build();
        // Line 3 should find line 1's span
        assert_eq!(map.find_nearest_span(3), Some(Span::new(0, 10)));
    }
}
