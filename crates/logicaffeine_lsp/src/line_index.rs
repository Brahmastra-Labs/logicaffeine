use tower_lsp::lsp_types::Position;

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

    /// Convert a byte offset to an LSP `Position`.
    ///
    /// Returns `(line, character)` where character is a UTF-16 code unit offset.
    pub fn position(&self, byte_offset: usize) -> Position {
        let byte_offset = byte_offset.min(self.source.len());

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

    /// Return the byte offset of the start of `line` (0-indexed).
    /// Returns `source.len()` if `line` is out of bounds.
    pub fn line_start_offset(&self, line: usize) -> usize {
        self.line_starts
            .get(line)
            .copied()
            .unwrap_or(self.source.len())
    }

    /// Compute the UTF-16 length of a byte range in the source.
    pub fn utf16_length(&self, byte_start: usize, byte_end: usize) -> u32 {
        let start = byte_start.min(self.source.len());
        let end = byte_end.min(self.source.len());
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
        let line_end = self
            .line_starts
            .get(line + 1)
            .copied()
            .unwrap_or(self.source.len());

        let line_text = &self.source[line_start..line_end];
        let mut utf16_offset = 0u32;
        let target = position.character;

        for (byte_idx, ch) in line_text.char_indices() {
            if utf16_offset >= target {
                return line_start + byte_idx;
            }
            utf16_offset += ch.len_utf16() as u32;
        }

        line_end
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_line() {
        let idx = LineIndex::new("hello world");
        assert_eq!(idx.position(0), Position { line: 0, character: 0 });
        assert_eq!(idx.position(5), Position { line: 0, character: 5 });
        assert_eq!(idx.position(11), Position { line: 0, character: 11 });
    }

    #[test]
    fn multi_line() {
        let idx = LineIndex::new("abc\ndef\nghi");
        assert_eq!(idx.position(0), Position { line: 0, character: 0 });
        assert_eq!(idx.position(3), Position { line: 0, character: 3 });
        assert_eq!(idx.position(4), Position { line: 1, character: 0 });
        assert_eq!(idx.position(7), Position { line: 1, character: 3 });
        assert_eq!(idx.position(8), Position { line: 2, character: 0 });
    }

    #[test]
    fn roundtrip() {
        let src = "Let x be 5.\nSet x to 10.\nShow x.\n";
        let idx = LineIndex::new(src);
        for offset in 0..src.len() {
            let pos = idx.position(offset);
            let back = idx.offset(pos);
            assert_eq!(back, offset, "roundtrip failed at offset {offset}");
        }
    }

    #[test]
    fn offset_from_position() {
        let idx = LineIndex::new("abc\ndef\nghi");
        assert_eq!(idx.offset(Position { line: 0, character: 0 }), 0);
        assert_eq!(idx.offset(Position { line: 1, character: 0 }), 4);
        assert_eq!(idx.offset(Position { line: 2, character: 2 }), 10);
    }

    #[test]
    fn empty_source() {
        let idx = LineIndex::new("");
        assert_eq!(idx.position(0), Position { line: 0, character: 0 });
        assert_eq!(idx.offset(Position { line: 0, character: 0 }), 0);
    }

    #[test]
    fn out_of_bounds_offset() {
        let idx = LineIndex::new("abc");
        let pos = idx.position(100);
        assert_eq!(pos, Position { line: 0, character: 3 });
    }

    #[test]
    fn out_of_bounds_position() {
        let idx = LineIndex::new("abc");
        let offset = idx.offset(Position { line: 5, character: 0 });
        assert_eq!(offset, 3);
    }

    #[test]
    fn line_start_offset_returns_correct_values() {
        let idx = LineIndex::new("abc\ndef\nghi");
        assert_eq!(idx.line_start_offset(0), 0);
        assert_eq!(idx.line_start_offset(1), 4);
        assert_eq!(idx.line_start_offset(2), 8);
    }

    #[test]
    fn line_start_offset_out_of_bounds() {
        let idx = LineIndex::new("abc\ndef");
        // Out of bounds should return source length
        assert_eq!(idx.line_start_offset(99), 7);
    }

    #[test]
    fn windows_line_endings() {
        let src = "abc\r\ndef\r\nghi";
        let idx = LineIndex::new(src);
        // \r\n: the \n is at byte 4, so line 1 starts at byte 5
        let pos = idx.position(5);
        assert_eq!(pos, Position { line: 1, character: 0 });
        let back = idx.offset(pos);
        assert_eq!(back, 5);
    }

    #[test]
    fn multibyte_utf8_roundtrip() {
        // 'é' is 2 bytes in UTF-8 but 1 UTF-16 code unit
        let src = "café\nworld";
        let idx = LineIndex::new(src);
        // 'c'=0, 'a'=1, 'f'=2, 'é'=3..4, '\n'=5, 'w'=6
        let pos_e_accent = idx.position(3);
        assert_eq!(pos_e_accent.line, 0);
        assert_eq!(pos_e_accent.character, 3, "UTF-16 offset of 'é' should be 3");
        let pos_world = idx.position(6);
        assert_eq!(pos_world, Position { line: 1, character: 0 });
        let back = idx.offset(pos_world);
        assert_eq!(back, 6);
    }
}
