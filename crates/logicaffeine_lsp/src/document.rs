use std::collections::HashMap;

use tower_lsp::lsp_types::{Diagnostic, Range, Url};

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
        let diagnostics = build_diagnostics(&analysis, &line_index, uri);

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

}

/// Apply one LSP content change to a document's text.
///
/// A `None` range is a whole-document replacement; a `Some` range is an
/// incremental edit whose positions are in UTF-16 code units, converted to
/// byte offsets through a fresh [`LineIndex`] over the current text.
pub fn apply_content_change(text: &mut String, range: Option<Range>, new_text: &str) {
    match range {
        None => {
            text.clear();
            text.push_str(new_text);
        }
        Some(range) => {
            let line_index = LineIndex::new(text);
            let start = line_index.offset(range.start);
            let end = line_index.offset(range.end);
            text.replace_range(start..end, new_text);
        }
    }
}

/// Convert every error class one analysis pass produced into LSP diagnostics.
fn build_diagnostics(
    analysis: &pipeline::AnalysisResult,
    line_index: &LineIndex,
    uri: Option<&Url>,
) -> Vec<Diagnostic> {
    let mut diagnostics = crate::diagnostics::convert_errors(
        &analysis.errors,
        &analysis.tokens,
        &analysis.interner,
        line_index,
        uri,
    );
    diagnostics.extend(crate::diagnostics::convert_analysis_errors(
        &analysis.escape_errors,
        &analysis.tokens,
        &analysis.interner,
        line_index,
        uri,
    ));
    diagnostics.extend(crate::diagnostics::convert_analysis_errors(
        &analysis.ownership_errors,
        &analysis.tokens,
        &analysis.interner,
        line_index,
        uri,
    ));
    diagnostics.extend(crate::diagnostics::unused_variable_hints(
        &analysis.symbol_index,
        line_index,
    ));
    diagnostics.extend(crate::diagnostics::shadowing_warnings(
        &analysis.symbol_index,
        line_index,
        uri,
    ));
    diagnostics.extend(crate::diagnostics::unused_function_hints(
        &analysis.symbol_index,
        line_index,
    ));
    diagnostics
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_document_parses_source() {
        let doc = DocumentState::new("## Main\n    Let x be 5.\n    Show x.\n".to_string(), 1);
        assert_eq!(doc.version, 1);
        assert!(doc.diagnostics.is_empty(), "Valid source should have no diagnostics: {:?}", doc.diagnostics);
        assert!(!doc.tokens.is_empty());
        assert!(!doc.symbol_index.definitions.is_empty());
    }

    #[test]
    fn fresh_snapshot_replaces_analysis() {
        let doc = DocumentState::new("## Main\n    Let x be 5.\n".to_string(), 1);
        assert_eq!(doc.symbol_index.definitions_of("x").len(), 1);
        assert_eq!(doc.symbol_index.definitions_of("y").len(), 0);

        // Documents are immutable snapshots: an edit produces a new one.
        let doc = DocumentState::new("## Main\n    Let y be 10.\n".to_string(), 2);
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
    fn snapshot_of_broken_source_carries_diagnostics() {
        let doc = DocumentState::new("## Main\n    Let x be 5.\n    Show x.\n".to_string(), 1);
        assert!(doc.diagnostics.is_empty(), "Valid source should have no diagnostics");
        let doc = DocumentState::new("## Main\n    Let be.\n".to_string(), 2);
        assert!(!doc.diagnostics.is_empty(), "Invalid source should produce diagnostics");
    }

    #[test]
    fn line_index_matches_snapshot_source() {
        let doc = DocumentState::new("line0\nline1\nline2\nline3\nline4\n".to_string(), 2);
        let pos = doc.line_index.position(doc.source.len().saturating_sub(2));
        assert_eq!(pos.line, 4, "In a 5-line doc, near-end should be line 4");
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
