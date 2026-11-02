use std::collections::HashMap;

use lsp_types::{Diagnostic, Range, Url};

use logicaffeine_base::Interner;
use logicaffeine_compile::analysis::VarState;
use logicaffeine_language::{
    analysis::{PolicyRegistry, TypeRegistry},
    token::Token,
};

use crate::index::SymbolIndex;
use crate::line_index::LineIndex;
use crate::pipeline;

/// Per-document state: source text, analysis results, and cached diagnostics.
/// The NAME under the cursor, and where it is written.
///
/// ★ ONE RULE, FOUR ASKERS. Go-to-definition, hover, rename and find-references each asked this
/// question for themselves, and each answered it the same wrong way: find the token at the offset,
/// read its text. Inside a `View:` that token is the whole `RawBlock`, so all four read the ENTIRE
/// TEMPLATE as the name and matched nothing — the same bug four times, discoverable four separate
/// times. An author hovering `{count}` got nothing; renaming it was not even offered.
///
/// A recorded REFERENCE wins over the token, and inside a template it is the only thing there is:
/// the parser noted every name a template mentions with its file span, precisely because that
/// information exists nowhere else by the time the tokens are all that is left. It is the better
/// answer outside a template too — a reference is a name the index has already placed, where a
/// token is only a string that happens to sit under the cursor.
pub fn name_at(
    doc: &DocumentState,
    offset: usize,
) -> Option<(String, logicaffeine_language::token::Span)> {
    if let Some(reference) = doc
        .symbol_index
        .references
        .iter()
        .find(|r| offset >= r.span.start && offset < r.span.end)
    {
        return Some((reference.name.clone(), reference.span));
    }
    let token = doc
        .tokens
        .iter()
        .find(|t| offset >= t.span.start && offset < t.span.end)?;
    let text = doc.source.get(token.span.start..token.span.end)?;
    Some((text.to_string(), token.span))
}

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
    diagnostics.extend(crate::diagnostics::lgx_finding_diagnostics(
        &analysis.lgx_findings,
        line_index,
    ));
    diagnostics.extend(crate::diagnostics::copy_finding_diagnostics(
        &analysis.copy_findings,
        line_index,
    ));
    diagnostics.extend(crate::diagnostics::refusal_diagnostics(
        &analysis.refusals,
        line_index,
    ));
    diagnostics
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_lsp/document/tests.rs"]
mod tests;
