use std::collections::HashMap;

use tower_lsp::lsp_types::{Diagnostic, Url};

use logicaffeine_base::Interner;
use logicaffeine_compile::analysis::VarState;
use logicaffeine_language::{
    analysis::{TypeRegistry, PolicyRegistry},
    token::Token,
};

use crate::index::SymbolIndex;
use crate::line_index::LineIndex;
use crate::pipeline;

/// Per-document state: source text, analysis results, and cached diagnostics.
pub struct DocumentState {
    pub source: String,
    pub version: i32,
    pub line_index: LineIndex,

    // Analysis results (rebuilt on each change)
    pub tokens: Vec<Token>,
    pub interner: Interner,
    pub diagnostics: Vec<Diagnostic>,
    pub symbol_index: SymbolIndex,
    pub type_registry: TypeRegistry,
    pub policy_registry: PolicyRegistry,
    pub ownership_states: HashMap<String, VarState>,
}

impl DocumentState {
    /// Create a new document state from source text.
    ///
    /// Pass a `uri` to enable `DiagnosticRelatedInformation` on analysis errors.
    pub fn new(source: String, version: i32) -> Self {
        Self::with_uri(source, version, None)
    }

    /// Create a new document state with a document URI for richer diagnostics.
    pub fn with_uri(source: String, version: i32, uri: Option<&Url>) -> Self {
        let line_index = LineIndex::new(&source);

        let analysis = pipeline::analyze(&source);

        let mut diagnostics = crate::diagnostics::convert_errors(
            &analysis.errors,
            &analysis.interner,
            &line_index,
        );
        diagnostics.extend(crate::diagnostics::convert_analysis_errors(
            &analysis.escape_errors,
            &analysis.tokens,
            &analysis.interner,
            &line_index,
            uri,
        ));
        diagnostics.extend(crate::diagnostics::convert_analysis_errors(
            &analysis.ownership_errors,
            &analysis.tokens,
            &analysis.interner,
            &line_index,
            uri,
        ));

        DocumentState {
            source,
            version,
            line_index,
            tokens: analysis.tokens,
            interner: analysis.interner,
            diagnostics,
            symbol_index: analysis.symbol_index,
            type_registry: analysis.type_registry,
            policy_registry: analysis.policy_registry,
            ownership_states: analysis.ownership_states,
        }
    }

    /// Update the document with new source text and re-run analysis.
    pub fn update(&mut self, source: String, version: i32) {
        self.update_with_uri(source, version, None);
    }

    /// Update the document with a URI for richer diagnostics.
    pub fn update_with_uri(&mut self, source: String, version: i32, uri: Option<&Url>) {
        self.line_index = LineIndex::new(&source);
        self.source = source;
        self.version = version;

        let analysis = pipeline::analyze(&self.source);

        let mut diagnostics = crate::diagnostics::convert_errors(
            &analysis.errors,
            &analysis.interner,
            &self.line_index,
        );
        diagnostics.extend(crate::diagnostics::convert_analysis_errors(
            &analysis.escape_errors,
            &analysis.tokens,
            &analysis.interner,
            &self.line_index,
            uri,
        ));
        diagnostics.extend(crate::diagnostics::convert_analysis_errors(
            &analysis.ownership_errors,
            &analysis.tokens,
            &analysis.interner,
            &self.line_index,
            uri,
        ));

        self.diagnostics = diagnostics;
        self.tokens = analysis.tokens;
        self.interner = analysis.interner;
        self.symbol_index = analysis.symbol_index;
        self.type_registry = analysis.type_registry;
        self.policy_registry = analysis.policy_registry;
        self.ownership_states = analysis.ownership_states;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_document_parses_source() {
        let doc = DocumentState::new("## Main\n    Let x be 5.\n".to_string(), 1);
        assert_eq!(doc.version, 1);
        assert!(doc.diagnostics.is_empty(), "Valid source should have no diagnostics: {:?}", doc.diagnostics);
        assert!(!doc.tokens.is_empty());
        assert!(!doc.symbol_index.definitions.is_empty());
    }

    #[test]
    fn update_replaces_analysis() {
        let mut doc = DocumentState::new("## Main\n    Let x be 5.\n".to_string(), 1);
        assert_eq!(doc.symbol_index.definitions_of("x").len(), 1);
        assert_eq!(doc.symbol_index.definitions_of("y").len(), 0);

        doc.update("## Main\n    Let y be 10.\n".to_string(), 2);
        assert_eq!(doc.version, 2);
        assert_eq!(doc.symbol_index.definitions_of("y").len(), 1);
    }

    #[test]
    fn empty_document() {
        let doc = DocumentState::new("".to_string(), 0);
        assert_eq!(doc.version, 0);
        assert_eq!(doc.source, "");
    }

    #[test]
    fn document_source_stored() {
        let source = "## Main\n    Let x be 5.\n";
        let doc = DocumentState::new(source.to_string(), 1);
        assert_eq!(doc.source, source);
    }

    #[test]
    fn update_changes_diagnostics_on_error() {
        let mut doc = DocumentState::new("## Main\n    Let x be 5.\n".to_string(), 1);
        assert!(doc.diagnostics.is_empty(), "Valid source should have no diagnostics");
        doc.update("## Main\n    Let be.\n".to_string(), 2);
        assert!(!doc.diagnostics.is_empty(), "Invalid source should produce diagnostics");
    }

    #[test]
    fn line_index_syncs_after_update() {
        let mut doc = DocumentState::new("ab\ncd\n".to_string(), 1);
        doc.update("line0\nline1\nline2\nline3\nline4\n".to_string(), 2);
        let pos = doc.line_index.position(doc.source.len().saturating_sub(2));
        assert_eq!(pos.line, 4, "After update to 5-line doc, near-end should be line 4");
    }

    #[test]
    fn document_with_move_error_has_diagnostics() {
        // Give x to y moves x; Show x afterward is use-after-move
        let source = "## Main\n    Let x be 5.\n    Let y be 0.\n    Give x to y.\n    Show x.\n";
        let doc = DocumentState::new(source.to_string(), 1);
        let move_diags: Vec<_> = doc.diagnostics.iter()
            .filter(|d| {
                let is_move_code = d.code.as_ref().map_or(false, |c| {
                    matches!(c, tower_lsp::lsp_types::NumberOrString::String(s) if s == "use-after-move")
                });
                let is_move_msg = d.message.contains("after") && d.message.contains("move");
                is_move_code || is_move_msg
            })
            .collect();
        assert!(
            !move_diags.is_empty(),
            "Document with use-after-move should have ownership diagnostics. All diags: {:?}",
            doc.diagnostics
        );
    }

    #[test]
    fn document_ownership_states_available() {
        let source = "## Main\n    Let x be 5.\n    Show x.\n";
        let doc = DocumentState::new(source.to_string(), 1);
        assert!(
            !doc.ownership_states.is_empty(),
            "Document should have ownership states for variables"
        );
    }
}
