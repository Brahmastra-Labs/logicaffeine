use lsp_types::TextEdit;

use crate::document::DocumentState;

/// Range formatting: the whole-document structural format (a line's depth
/// depends on the lines above it — depth is a document-level fact), filtered
/// to the edits whose lines intersect the requested range. Sound because
/// `format_source` is line-count preserving: every edit stays on its line.
pub fn format_range(doc: &DocumentState, range: lsp_types::Range) -> Vec<TextEdit> {
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
                range: lsp_types::Range { start, end },
                new_text: new_line,
            });
        }
    }

    edits
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_lsp/formatting/tests.rs"]
mod tests;
