use tower_lsp::lsp_types::TextEdit;

use crate::document::DocumentState;

/// Handle document formatting request.
///
/// Normalizes indentation (tabs → 4 spaces, mixed leading whitespace)
/// and removes trailing whitespace from all lines.
pub fn format_document(doc: &DocumentState) -> Vec<TextEdit> {
    let mut edits = Vec::new();

    for (line_num, line) in doc.source.lines().enumerate() {
        let mut new_line = String::new();

        // Normalize leading whitespace: replace any tabs with 4 spaces
        let leading_len = line.len() - line.trim_start().len();
        let leading = &line[..leading_len];
        if leading.contains('\t') {
            for ch in leading.chars() {
                if ch == '\t' {
                    new_line.push_str("    ");
                } else {
                    new_line.push(ch);
                }
            }
            new_line.push_str(line[leading_len..].trim_end());
        } else {
            new_line.push_str(line.trim_end());
        }

        if new_line != line {
            let line_start = doc.line_index.line_start_offset(line_num);
            let line_end = doc.line_index.line_start_offset(line_num + 1).saturating_sub(1)
                .max(line_start);

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
    fn formatting_multiple_tabs() {
        let doc = make_doc("## Main\n\t\tLet x be 5.\n");
        let edits = format_document(&doc);
        assert!(!edits.is_empty(), "Expected edits for double-tabbed line");
        assert!(
            edits[0].new_text.starts_with("        "),
            "Two tabs should become 8 spaces: {:?}",
            edits[0].new_text
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
}
