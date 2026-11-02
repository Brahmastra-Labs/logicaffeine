use lsp_types::Position;

/// Maps between byte offsets and LSP `Position` (line, character).
///
/// LSP positions use zero-based line and UTF-16 code unit offsets.
/// Our source strings use byte offsets. This struct pre-computes
/// line start byte offsets for efficient bidirectional conversion.
#[derive(Debug, Clone)]
pub struct LineIndex {
    /// Byte offset of each line start. `line_starts[0]` is always 0.
    line_starts: Vec<usize>,
    /// The full source text (needed for UTF-16 offset computation).
    source: String,
}

impl LineIndex {
    pub fn new(source: &str) -> Self {
        let mut line_starts = vec![0];
        for (i, b) in source.bytes().enumerate() {
            if b == b'\n' {
                line_starts.push(i + 1);
            }
        }
        LineIndex {
            line_starts,
            source: source.to_string(),
        }
    }

    /// Largest char boundary `<= i` (clamped to the source length). Keeps a
    /// COMPUTED offset (e.g. `span.start + name.len()`) from slicing mid-char and
    /// panicking — position math must be total over every `usize`.
    fn floor_char_boundary(&self, i: usize) -> usize {
        let mut i = i.min(self.source.len());
        while i > 0 && !self.source.is_char_boundary(i) {
            i -= 1;
        }
        i
    }

    /// Convert a byte offset to an LSP `Position`.
    ///
    /// Returns `(line, character)` where character is a UTF-16 code unit offset.
    pub fn position(&self, byte_offset: usize) -> Position {
        let byte_offset = self.floor_char_boundary(byte_offset);

        let line = self
            .line_starts
            .partition_point(|&start| start <= byte_offset)
            .saturating_sub(1);

        let line_start = self.line_starts[line];
        let line_text = &self.source[line_start..byte_offset];
        let character = line_text.encode_utf16().count() as u32;

        Position {
            line: line as u32,
            character,
        }
    }

    /// Byte offset of the end of `line`'s CONTENT — just before its `\n` (and a
    /// preceding `\r`), or `source.len()` for the last line. Used to clip a
    /// multi-line token span to a single line for the semantic-token encoder.
    pub fn line_end_offset(&self, line: usize) -> usize {
        let end = match self.line_starts.get(line + 1) {
            Some(&next) => next.saturating_sub(1), // the '\n'
            None => self.source.len(),
        };
        if end > 0 && self.source.as_bytes().get(end - 1) == Some(&b'\r') {
            end - 1
        } else {
            end
        }
    }

    /// Return the byte offset of the start of `line` (0-indexed).
    /// Returns `source.len()` if `line` is out of bounds.
    pub fn line_start_offset(&self, line: usize) -> usize {
        self.line_starts
            .get(line)
            .copied()
            .unwrap_or(self.source.len())
    }

    /// Compute the UTF-16 length of a byte range in the source. Offsets are
    /// snapped to char boundaries so a computed range can never panic the slice.
    pub fn utf16_length(&self, byte_start: usize, byte_end: usize) -> u32 {
        let start = self.floor_char_boundary(byte_start);
        let end = self.floor_char_boundary(byte_end);
        if start >= end {
            return 0;
        }
        self.source[start..end].encode_utf16().count() as u32
    }

    /// Convert an LSP `Position` to a byte offset.
    pub fn offset(&self, position: Position) -> usize {
        let line = position.line as usize;
        if line >= self.line_starts.len() {
            return self.source.len();
        }

        let line_start = self.line_starts[line];
        // Scan only the line's CONTENT (terminator excluded): LSP clamps a
        // character past the line to the line length, so an out-of-range Position
        // must resolve to this line's end, not cross the `\n` into the next line.
        let content_end = self.line_end_offset(line);

        let line_text = &self.source[line_start..content_end];
        let mut utf16_offset = 0u32;
        let target = position.character;

        for (byte_idx, ch) in line_text.char_indices() {
            if utf16_offset >= target {
                return line_start + byte_idx;
            }
            utf16_offset += ch.len_utf16() as u32;
        }

        content_end
    }
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_lsp/line_index/tests.rs"]
mod tests;
