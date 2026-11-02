use super::*;

#[test]
fn single_line() {
    let idx = LineIndex::new("hello world");
    assert_eq!(
        idx.position(0),
        Position {
            line: 0,
            character: 0
        }
    );
    assert_eq!(
        idx.position(5),
        Position {
            line: 0,
            character: 5
        }
    );
    assert_eq!(
        idx.position(11),
        Position {
            line: 0,
            character: 11
        }
    );
}

#[test]
fn multi_line() {
    let idx = LineIndex::new("abc\ndef\nghi");
    assert_eq!(
        idx.position(0),
        Position {
            line: 0,
            character: 0
        }
    );
    assert_eq!(
        idx.position(3),
        Position {
            line: 0,
            character: 3
        }
    );
    assert_eq!(
        idx.position(4),
        Position {
            line: 1,
            character: 0
        }
    );
    assert_eq!(
        idx.position(7),
        Position {
            line: 1,
            character: 3
        }
    );
    assert_eq!(
        idx.position(8),
        Position {
            line: 2,
            character: 0
        }
    );
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
    assert_eq!(
        idx.offset(Position {
            line: 0,
            character: 0
        }),
        0
    );
    assert_eq!(
        idx.offset(Position {
            line: 1,
            character: 0
        }),
        4
    );
    assert_eq!(
        idx.offset(Position {
            line: 2,
            character: 2
        }),
        10
    );
}

#[test]
fn empty_source() {
    let idx = LineIndex::new("");
    assert_eq!(
        idx.position(0),
        Position {
            line: 0,
            character: 0
        }
    );
    assert_eq!(
        idx.offset(Position {
            line: 0,
            character: 0
        }),
        0
    );
}

#[test]
fn out_of_bounds_offset() {
    let idx = LineIndex::new("abc");
    let pos = idx.position(100);
    assert_eq!(
        pos,
        Position {
            line: 0,
            character: 3
        }
    );
}

#[test]
fn position_snaps_a_mid_char_offset_to_a_boundary() {
    // `é` is two bytes (0xC3 0xA9); byte offset 2 lands INSIDE it. A computed
    // offset like `span.start + name.len()` (document_symbols) can reach here,
    // and `position` must never panic slicing on a non-char-boundary.
    let idx = LineIndex::new("aéb");
    let pos = idx.position(2);
    // Snapped down to the boundary after `a` (byte 1 → 1 UTF-16 unit).
    assert_eq!(
        pos,
        Position {
            line: 0,
            character: 1
        }
    );
}

#[test]
fn utf16_length_tolerates_mid_char_offsets() {
    // Both ends inside multibyte chars must not panic.
    let idx = LineIndex::new("aéb");
    let _ = idx.utf16_length(2, 3);
    let _ = idx.utf16_length(1, 2);
}

#[test]
fn out_of_bounds_position() {
    let idx = LineIndex::new("abc");
    let offset = idx.offset(Position {
        line: 5,
        character: 0,
    });
    assert_eq!(offset, 3);
}

#[test]
fn offset_past_line_end_clamps_to_content_not_next_line() {
    let idx = LineIndex::new("abc\ndef");
    // A character past line 0's content clamps to its end (byte 3), NOT
    // across the `\n` into line 1 (byte 4).
    assert_eq!(
        idx.offset(Position {
            line: 0,
            character: 99
        }),
        3
    );
    assert_eq!(
        idx.offset(Position {
            line: 0,
            character: 3
        }),
        3
    );
    // The last line (no terminator) still clamps to its own end.
    assert_eq!(
        idx.offset(Position {
            line: 1,
            character: 99
        }),
        7
    );
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
    assert_eq!(
        pos,
        Position {
            line: 1,
            character: 0
        }
    );
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
    assert_eq!(
        pos_e_accent.character, 3,
        "UTF-16 offset of 'é' should be 3"
    );
    let pos_world = idx.position(6);
    assert_eq!(
        pos_world,
        Position {
            line: 1,
            character: 0
        }
    );
    let back = idx.offset(pos_world);
    assert_eq!(back, 6);
}
