use lsp_types::{
    CodeAction, CodeActionKind, CodeActionOrCommand, Position, Range, TextEdit, Url, WorkspaceEdit,
};
use std::collections::{HashMap, HashSet};

use logicaffeine_language::suggest::{find_similar, KNOWN_WORDS};

use crate::document::DocumentState;
use crate::workspace::WorkspaceView;

/// Handle code action request.
///
/// Provides quick-fix suggestions based on diagnostics in the given range.
pub fn code_actions(
    doc: &DocumentState,
    range: Range,
    uri: &lsp_types::Url,
) -> Vec<CodeActionOrCommand> {
    let mut actions = Vec::new();

    // Generate code actions from diagnostics that overlap the requested range
    let is_default_range = range == Range::default();
    for diagnostic in &doc.diagnostics {
        if !is_default_range && !ranges_overlap(&diagnostic.range, &range) {
            continue;
        }
        let start_offset = doc.line_index.offset(diagnostic.range.start);
        let end_offset = doc.line_index.offset(diagnostic.range.end);

        let word = doc.source.get(start_offset..end_offset).unwrap_or("");

        // Spelling fix suggestions — only for UN-CODED diagnostics (raw parse
        // errors, where typos live), and never for one-character words (every
        // one-char identifier is within distance 2 of something). A coded
        // diagnostic is about semantics, not spelling: suggesting `a` for a
        // moved `x` is noise, and `undefined-variable` has its own
        // definition-aware suggester below.
        if diagnostic.code.is_none() && word.chars().count() >= 2 {
            if let Some(suggestion) = find_similar(word, KNOWN_WORDS, 2) {
                let mut changes = HashMap::new();
                changes.insert(
                    uri.clone(),
                    vec![TextEdit {
                        range: diagnostic.range,
                        new_text: suggestion.to_string(),
                    }],
                );

                actions.push(CodeActionOrCommand::CodeAction(CodeAction {
                    title: format!("Did you mean '{}'?", suggestion),
                    kind: Some(CodeActionKind::QUICKFIX),
                    diagnostics: Some(vec![diagnostic.clone()]),
                    edit: Some(WorkspaceEdit {
                        changes: Some(changes),
                        ..Default::default()
                    }),
                    ..Default::default()
                }));
            }
        }

        // "x is y" → "x equals y" fix
        let is_value_eq = diagnostic.code.as_ref().map_or(
            false,
            |c| matches!(c, lsp_types::NumberOrString::String(s) if s == "is-value-equality"),
        );
        if is_value_eq {
            if let Some(is_pos) = doc
                .source
                .get(start_offset..end_offset)
                .and_then(|s| s.find(" is "))
            {
                let abs_pos = start_offset + is_pos;
                let is_range = Range {
                    start: doc.line_index.position(abs_pos),
                    end: doc.line_index.position(abs_pos + 4), // " is "
                };

                let mut changes = HashMap::new();
                changes.insert(
                    uri.clone(),
                    vec![TextEdit {
                        range: is_range,
                        new_text: " equals ".to_string(),
                    }],
                );

                actions.push(CodeActionOrCommand::CodeAction(CodeAction {
                    title: "Use 'equals' for value comparison".to_string(),
                    kind: Some(CodeActionKind::QUICKFIX),
                    diagnostics: Some(vec![diagnostic.clone()]),
                    edit: Some(WorkspaceEdit {
                        changes: Some(changes),
                        ..Default::default()
                    }),
                    ..Default::default()
                }));
            }
        }

        // UseAfterMove → suggest "copy of variable". The message names the
        // variable ("Cannot use 'x' …"); the range can sit on statement
        // punctuation on the parse path, so the edit retargets to the
        // variable's own occurrence.
        let is_use_after_move = diagnostic.code.as_ref().map_or(
            false,
            |c| matches!(c, lsp_types::NumberOrString::String(s) if s == "use-after-move"),
        );
        if is_use_after_move {
            let variable = ownership_variable(&diagnostic.message, word);
            let target = if variable == word {
                diagnostic.range
            } else {
                last_word_range(doc, variable, end_offset).unwrap_or(diagnostic.range)
            };
            let mut changes = HashMap::new();
            changes.insert(
                uri.clone(),
                vec![TextEdit {
                    range: target,
                    new_text: format!("a copy of {}", variable),
                }],
            );

            actions.push(CodeActionOrCommand::CodeAction(CodeAction {
                title: format!("Use 'a copy of {}' instead", variable),
                kind: Some(CodeActionKind::QUICKFIX),
                diagnostics: Some(vec![diagnostic.clone()]),
                edit: Some(WorkspaceEdit {
                    changes: Some(changes),
                    ..Default::default()
                }),
                ..Default::default()
            }));
        }

        // Escape errors → suggest copying before return/assignment
        let diag_code_str = diagnostic.code.as_ref().and_then(|c| {
            if let lsp_types::NumberOrString::String(s) = c {
                Some(s.as_str())
            } else {
                None
            }
        });

        if matches!(diag_code_str, Some("escape-return")) {
            let mut changes = HashMap::new();
            changes.insert(
                uri.clone(),
                vec![TextEdit {
                    range: diagnostic.range,
                    new_text: format!("a copy of {}", word),
                }],
            );
            actions.push(CodeActionOrCommand::CodeAction(CodeAction {
                title: "Copy before returning".to_string(),
                kind: Some(CodeActionKind::QUICKFIX),
                diagnostics: Some(vec![diagnostic.clone()]),
                edit: Some(WorkspaceEdit {
                    changes: Some(changes),
                    ..Default::default()
                }),
                ..Default::default()
            }));
        }

        if matches!(diag_code_str, Some("escape-assignment")) {
            let mut changes = HashMap::new();
            changes.insert(
                uri.clone(),
                vec![TextEdit {
                    range: diagnostic.range,
                    new_text: format!("a copy of {}", word),
                }],
            );
            actions.push(CodeActionOrCommand::CodeAction(CodeAction {
                title: "Copy before assignment".to_string(),
                kind: Some(CodeActionKind::QUICKFIX),
                diagnostics: Some(vec![diagnostic.clone()]),
                edit: Some(WorkspaceEdit {
                    changes: Some(changes),
                    ..Default::default()
                }),
                ..Default::default()
            }));
        }

        // DoubleMoved → suggest copying instead of second Give
        if matches!(diag_code_str, Some("double-move")) {
            let variable = ownership_variable(&diagnostic.message, word);
            let target = if variable == word {
                diagnostic.range
            } else {
                last_word_range(doc, variable, end_offset).unwrap_or(diagnostic.range)
            };
            let mut changes = HashMap::new();
            changes.insert(
                uri.clone(),
                vec![TextEdit {
                    range: target,
                    new_text: format!("a copy of {}", variable),
                }],
            );
            actions.push(CodeActionOrCommand::CodeAction(CodeAction {
                title: format!("Give 'a copy of {}' instead", variable),
                kind: Some(CodeActionKind::QUICKFIX),
                diagnostics: Some(vec![diagnostic.clone()]),
                edit: Some(WorkspaceEdit {
                    changes: Some(changes),
                    ..Default::default()
                }),
                ..Default::default()
            }));
        }

        // ZeroIndex → suggest 1-based indexing
        if matches!(diag_code_str, Some("zero-index")) {
            let mut changes = HashMap::new();
            changes.insert(
                uri.clone(),
                vec![TextEdit {
                    range: diagnostic.range,
                    new_text: "1".to_string(),
                }],
            );
            actions.push(CodeActionOrCommand::CodeAction(CodeAction {
                title: "Use 1-based indexing".to_string(),
                kind: Some(CodeActionKind::QUICKFIX),
                diagnostics: Some(vec![diagnostic.clone()]),
                edit: Some(WorkspaceEdit {
                    changes: Some(changes),
                    ..Default::default()
                }),
                ..Default::default()
            }));
        }

        // Unused variable → remove the whole statement (through its newline)
        if matches!(diag_code_str, Some("unused-variable")) {
            let def_offset = doc.line_index.offset(diagnostic.range.start);
            let stmt_span = doc
                .symbol_index
                .statement_spans
                .iter()
                .map(|(_, span)| span)
                .filter(|span| span.start <= def_offset && def_offset <= span.end)
                .min_by_key(|span| span.end - span.start);
            if let Some(span) = stmt_span {
                let mut delete_end = span.end;
                let rest = doc.source.get(delete_end..).unwrap_or("");
                if rest.starts_with("\r\n") {
                    delete_end += 2;
                } else if rest.starts_with('\n') {
                    delete_end += 1;
                }
                let mut changes = HashMap::new();
                changes.insert(
                    uri.clone(),
                    vec![TextEdit {
                        range: Range {
                            start: doc.line_index.position(span.start),
                            end: doc.line_index.position(delete_end),
                        },
                        new_text: String::new(),
                    }],
                );
                actions.push(CodeActionOrCommand::CodeAction(CodeAction {
                    title: format!("Remove unused '{}'", word),
                    kind: Some(CodeActionKind::QUICKFIX),
                    diagnostics: Some(vec![diagnostic.clone()]),
                    edit: Some(WorkspaceEdit {
                        changes: Some(changes),
                        ..Default::default()
                    }),
                    ..Default::default()
                }));
            }
        }

        // UndefinedVariable → suggest closest match from definitions
        if matches!(diag_code_str, Some("undefined-variable")) && !word.is_empty() {
            let def_names: Vec<&str> = doc
                .symbol_index
                .definitions
                .iter()
                .map(|d| d.name.as_str())
                .collect();
            if let Some(suggestion) = find_similar(word, &def_names, 2) {
                let mut changes = HashMap::new();
                changes.insert(
                    uri.clone(),
                    vec![TextEdit {
                        range: diagnostic.range,
                        new_text: suggestion.to_string(),
                    }],
                );
                actions.push(CodeActionOrCommand::CodeAction(CodeAction {
                    title: format!("Did you mean '{}'?", suggestion),
                    kind: Some(CodeActionKind::QUICKFIX),
                    diagnostics: Some(vec![diagnostic.clone()]),
                    edit: Some(WorkspaceEdit {
                        changes: Some(changes),
                        ..Default::default()
                    }),
                    ..Default::default()
                }));
            }
        }
    }

    actions
}

fn ranges_overlap(a: &Range, b: &Range) -> bool {
    !(a.end.line < b.start.line
        || (a.end.line == b.start.line && a.end.character < b.start.character)
        || b.end.line < a.start.line
        || (b.end.line == a.start.line && b.end.character < a.start.character))
}

/// AUTO-IMPORT quickfixes: for each unresolved reference in `range` that some
/// project module publicly exports but this file does NOT import, offer an
/// `Import <Module>.` action whose single edit inserts the statement in the right
/// place (after existing imports, else before the first `## ` block). An ambiguous
/// name (exported by several modules) offers one action per module. THE lever —
/// the compiler error becomes a one-click fix before a build is ever run.
pub fn import_code_actions(
    doc: &DocumentState,
    range: Range,
    uri: &Url,
    ws: &dyn WorkspaceView,
) -> Vec<CodeActionOrCommand> {
    let mut actions = Vec::new();
    let is_default_range = range == Range::default();
    let (insert_pos, blank_after) = import_insertion(&doc.source);
    let mut seen: HashSet<(String, String)> = HashSet::new();

    for reference in &doc.symbol_index.references {
        // Only names the file can't already resolve locally.
        if reference.definition_idx.is_some() {
            continue;
        }
        let ref_range = Range {
            start: doc.line_index.position(reference.span.start),
            end: doc.line_index.position(reference.span.end),
        };
        if !is_default_range && !ranges_overlap(&ref_range, &range) {
            continue;
        }
        let name = &reference.name;
        if ws.is_imported(uri, name) {
            continue;
        }
        for export in ws.module_exporting(name) {
            if !seen.insert((name.clone(), export.module_name.clone())) {
                continue;
            }
            let new_text = if blank_after {
                format!("Import {}.\n\n", export.module_name)
            } else {
                format!("Import {}.\n", export.module_name)
            };
            let mut changes = HashMap::new();
            changes.insert(
                uri.clone(),
                vec![TextEdit {
                    range: Range {
                        start: insert_pos,
                        end: insert_pos,
                    },
                    new_text,
                }],
            );
            actions.push(CodeActionOrCommand::CodeAction(CodeAction {
                title: format!("Import {}", export.module_name),
                kind: Some(CodeActionKind::QUICKFIX),
                edit: Some(WorkspaceEdit {
                    changes: Some(changes),
                    ..Default::default()
                }),
                ..Default::default()
            }));
        }
    }
    actions
}

/// Where to insert a new `Import` statement, and whether to follow it with a blank
/// line: after the last existing import if there is one, else immediately before
/// the first `## ` block (with a blank line), else after the title. The position
/// always lands inside the region the dependency scanner reads.
fn import_insertion(source: &str) -> (Position, bool) {
    let mut last_import: Option<u32> = None;
    let mut first_block: Option<u32> = None;
    for (i, line) in source.lines().enumerate() {
        let t = line.trim();
        if t.starts_with("## ") {
            first_block = Some(i as u32);
            break;
        }
        if t.starts_with("Import ") {
            last_import = Some(i as u32);
        }
    }
    if let Some(i) = last_import {
        return (
            Position {
                line: i + 1,
                character: 0,
            },
            false,
        );
    }
    if let Some(i) = first_block {
        return (
            Position {
                line: i,
                character: 0,
            },
            true,
        );
    }
    let after_title = source
        .lines()
        .next()
        .is_some_and(|l| l.trim_start().starts_with('#'));
    (
        Position {
            line: if after_title { 1 } else { 0 },
            character: 0,
        },
        false,
    )
}

/// "Remove unused import" quickfixes: an `Import X.` whose module's names are
/// never referenced in this file. The editor complement of the `largo check`
/// unused-import warning — a one-click removal.
pub fn unused_import_actions(
    doc: &DocumentState,
    range: Range,
    uri: &Url,
    ws: &dyn WorkspaceView,
) -> Vec<CodeActionOrCommand> {
    let referenced: HashSet<&str> = doc
        .symbol_index
        .references
        .iter()
        .map(|r| r.name.as_str())
        .collect();
    let module_urls: HashMap<String, Url> = ws.module_names().into_iter().collect();
    let is_default_range = range == Range::default();
    let mut actions = Vec::new();

    for dep in logicaffeine_language::analysis::scan_dependencies(&doc.source) {
        let start = doc.line_index.position(dep.start);
        let end = doc.line_index.position(dep.end);
        if !is_default_range && !ranges_overlap(&Range { start, end }, &range) {
            continue;
        }
        // Resolve the module and the names this import brings.
        let Some(module_url) = module_urls.get(&dep.alias) else {
            continue;
        };
        let exports = ws.exports_of(module_url);
        if exports.is_empty() {
            continue; // unknown module — never guess "unused"
        }
        let names: Vec<String> = match &dep.selection {
            Some(sel) => sel
                .iter()
                .map(|s| s.alias.clone().unwrap_or_else(|| s.name.clone()))
                .collect(),
            None => exports,
        };
        if names.iter().any(|n| referenced.contains(n.as_str())) {
            continue; // used — keep it
        }
        // Unused → remove the whole import line (through its trailing newline).
        let del = Range {
            start: Position {
                line: start.line,
                character: 0,
            },
            end: Position {
                line: start.line + 1,
                character: 0,
            },
        };
        let mut changes = HashMap::new();
        changes.insert(
            uri.clone(),
            vec![TextEdit {
                range: del,
                new_text: String::new(),
            }],
        );
        actions.push(CodeActionOrCommand::CodeAction(CodeAction {
            title: format!("Remove unused import `{}`", dep.alias),
            kind: Some(CodeActionKind::QUICKFIX),
            edit: Some(WorkspaceEdit {
                changes: Some(changes),
                ..Default::default()
            }),
            ..Default::default()
        }));
    }
    actions
}

/// The variable an ownership diagnostic is about: the first 'quoted' name in
/// its socratic message (the text always names it), else the range's text.
fn ownership_variable<'a>(message: &'a str, range_word: &'a str) -> &'a str {
    message
        .split('\'')
        .nth(1)
        .filter(|name| !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_'))
        .unwrap_or(range_word)
}

/// The last whole-word occurrence of `name` at or before `before` — where an
/// ownership fix must land when the diagnostic range sits on punctuation.
fn last_word_range(doc: &DocumentState, name: &str, before: usize) -> Option<Range> {
    let hay = doc.source.get(..before.min(doc.source.len()))?;
    let bytes = hay.as_bytes();
    let is_word = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    let mut search_end = hay.len();
    while let Some(pos) = hay[..search_end].rfind(name) {
        let end = pos + name.len();
        let boundary_before = pos == 0 || !is_word(bytes[pos - 1]);
        let boundary_after = end >= hay.len() || !is_word(bytes[end]);
        if boundary_before && boundary_after {
            return Some(Range {
                start: doc.line_index.position(pos),
                end: doc.line_index.position(end),
            });
        }
        if pos == 0 {
            break;
        }
        search_end = pos;
    }
    None
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_lsp/code_actions/tests.rs"]
mod tests;
