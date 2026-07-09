use tower_lsp::lsp_types::TextEdit;

use crate::document::DocumentState;

/// Range formatting: the whole-document structural format (a line's depth
/// depends on the lines above it — depth is a document-level fact), filtered
/// to the edits whose lines intersect the requested range. Sound because
/// `format_source` is line-count preserving: every edit stays on its line.
pub fn format_range(doc: &DocumentState, range: tower_lsp::lsp_types::Range) -> Vec<TextEdit> {
    format_document(doc)
        .into_iter()
        .filter(|edit| {
            edit.range.start.line <= range.end.line && edit.range.end.line >= range.start.line
        })
        .collect()
}

/// Handle document formatting request.
///
/// Formats the WHOLE document through the canonical LOGOS formatter
/// ([`logicaffeine_language::source_format::format_source`] — identical to
/// `largo fmt`, structural reindent and string/prose protection included)
/// and emits per-line [`TextEdit`]s for the lines that changed.
pub fn format_document(doc: &DocumentState) -> Vec<TextEdit> {
    let mut edits = Vec::new();

    let formatted = logicaffeine_language::source_format::format_source(&doc.source);
    let mut formatted_lines = formatted.lines();

    for (line_num, line) in doc.source.lines().enumerate() {
        // format_source is line-count preserving by construction.
        let new_line = formatted_lines.next().unwrap_or_default().to_string();

        if new_line != line {
            let line_start = doc.line_index.line_start_offset(line_num);
            // The line's content ends at `line_start + line.len()` (always a
            // char boundary). Terminated lines extend the range up to the
            // `\n` so a `\r` (CRLF) is replaced away too; the final
            // unterminated line ends exactly at EOF — never one byte short,
            // which would duplicate the last character (or split a
            // multibyte one and panic).
            let content_end = line_start + line.len();
            let next_start = doc.line_index.line_start_offset(line_num + 1);
            let line_end = if next_start > content_end
                && doc.source.as_bytes().get(next_start.saturating_sub(1)) == Some(&b'\n')
            {
                next_start - 1
            } else {
                content_end
            };

            let start = doc.line_index.position(line_start);
            let end = doc.line_index.position(line_end);

            edits.push(TextEdit {
                range: tower_lsp::lsp_types::Range { start, end },
                new_text: new_line,
            });
        }
    }

    edits
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::DocumentState;

    fn make_doc(source: &str) -> DocumentState {
        DocumentState::new(source.to_string(), 1)
    }

    #[test]
    fn formatting_no_edits_for_spaces() {
        let doc = make_doc("## Main\n    Let x be 5.\n");
        let edits = format_document(&doc);
        assert!(edits.is_empty(), "No edits expected for properly indented code");
    }

    #[test]
    fn formatting_replaces_tabs() {
        let doc = make_doc("## Main\n\tLet x be 5.\n");
        let edits = format_document(&doc);
        assert!(!edits.is_empty(), "Expected edits to replace tab with spaces");
        assert!(
            edits[0].new_text.contains("    "),
            "Tab should be replaced with 4 spaces: {:?}",
            edits[0].new_text
        );
    }

    #[test]
    fn formatting_empty_doc() {
        let doc = make_doc("");
        let edits = format_document(&doc);
        assert!(edits.is_empty());
    }

    #[test]
    fn formatting_reindents_by_lexed_depth_not_tab_width() {
        // A double-tab as the FIRST indent is still one nesting level to the
        // lexer — the canonical form is depth × 4 spaces, not width × 4.
        let doc = make_doc("## Main\n\t\tLet x be 5.\n");
        let edits = format_document(&doc);
        assert!(!edits.is_empty(), "Expected edits for double-tabbed line");
        assert_eq!(
            edits[0].new_text, "    Let x be 5.",
            "depth 1 canonicalizes to 4 spaces regardless of original width"
        );
    }

    #[test]
    fn formatting_large_file_with_tabs() {
        // Regression test: formatting must not be O(n^2) with many tabbed lines
        let mut source = "## Main\n".to_string();
        for i in 0..100 {
            source.push_str(&format!("\tLet x{} be {}.\n", i, i));
        }
        let doc = make_doc(&source);
        let edits = format_document(&doc);
        assert_eq!(edits.len(), 100, "Each tabbed line should produce an edit");
    }

    #[test]
    fn edit_ranges_start_at_correct_line() {
        let doc = make_doc("## Main\n\tLet x be 5.\n");
        let edits = format_document(&doc);
        assert!(!edits.is_empty());
        assert_eq!(edits[0].range.start.line, 1, "Tab line edit should start on line 1");
        assert_eq!(edits[0].range.start.character, 0, "Edit should start at character 0");
    }

    #[test]
    fn no_panic_multi_line_tabs() {
        let doc = make_doc("\tline1\n\tline2\n\tline3\n");
        let edits = format_document(&doc);
        assert_eq!(edits.len(), 3, "Each tabbed line should produce an edit");
        for (i, edit) in edits.iter().enumerate() {
            assert_eq!(edit.range.start.line, i as u32, "Edit {} should be on line {}", i, i);
        }
    }

    #[test]
    fn formatting_tab_replacement_produces_correct_ranges() {
        let doc = make_doc("## Main\n\tLet x be 5.\n\tLet y be 10.\n");
        let edits = format_document(&doc);
        assert_eq!(edits.len(), 2, "Expected 2 edits for 2 tabbed lines");
        // Second edit's range should start on line 2
        assert_eq!(edits[1].range.start.line, 2, "Second edit should be on line 2");
    }

    #[test]
    fn formatting_removes_trailing_whitespace() {
        let doc = make_doc("## Main   \n    Let x be 5.   \n");
        let edits = format_document(&doc);
        assert!(!edits.is_empty(), "Expected edits for trailing whitespace");
        for edit in &edits {
            assert!(!edit.new_text.ends_with(' '),
                "Edit should not end with spaces: {:?}", edit.new_text);
        }
    }

    #[test]
    fn formatting_handles_mixed_tabs_spaces() {
        let doc = make_doc("## Main\n  \tLet x be 5.\n");
        let edits = format_document(&doc);
        assert!(!edits.is_empty(), "Expected edits for mixed tabs/spaces");
        assert!(!edits[0].new_text.contains('\t'),
            "Mixed tabs should be replaced: {:?}", edits[0].new_text);
    }

    #[test]
    fn formatting_final_line_without_trailing_newline_covers_whole_line() {
        // The edit range must span the ENTIRE final line — an off-by-one end
        // would leave the last character outside the replacement and
        // duplicate it when the client applies the edit.
        let doc = make_doc("## Main\n\tX");
        let edits = format_document(&doc);
        assert_eq!(edits.len(), 1, "one edit for the tabbed final line");
        assert_eq!(edits[0].new_text, "    X");
        assert_eq!(edits[0].range.start.line, 1);
        assert_eq!(edits[0].range.start.character, 0);
        assert_eq!(
            edits[0].range.end.character, 2,
            "range must cover both characters of \"\\tX\""
        );
    }

    #[test]
    fn formatting_final_line_ending_in_multibyte_char_does_not_panic() {
        let doc = make_doc("\tπ");
        let edits = format_document(&doc);
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].new_text, "    π");
        assert_eq!(edits[0].range.end.character, 2, "tab + π in UTF-16 units");
    }
}
