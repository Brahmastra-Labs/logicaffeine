use super::*;

/// The range encoder the model and the RTL share equals the standard library's UTF-8 encoder on EVERY
/// Unicode scalar value — all 1,112,064 of them, not a sample — so `TextBytes` (237) is `s.as_bytes()`.
#[test]
fn the_range_encoder_is_utf8_on_every_scalar_value() {
    let mut buf = [0u8; 4];
    let mut checked = 0u32;
    for cp in 0..=0x10_FFFFu32 {
        let Some(c) = char::from_u32(cp) else {
            continue;
        };
        let want = c.encode_utf8(&mut buf).as_bytes();
        let (got, n) = utf8_of_codepoint(cp);
        assert_eq!(
            &got[..n],
            want,
            "U+{cp:04X}: range encoder != char::encode_utf8"
        );
        assert!(
            got[n..].iter().all(|&b| b == 0),
            "U+{cp:04X}: bytes past the count must be zero"
        );
        checked += 1;
    }
    assert_eq!(
        checked,
        0x11_0000 - 0x800,
        "every scalar value (all codepoints minus the surrogates)"
    );
}

/// Each width boundary lands on the side `char::len_utf8` puts it: the last codepoint of a width and the
/// first of the next.
#[test]
fn the_range_encoder_switches_width_exactly_at_each_boundary() {
    for (cp, len) in [
        (0x00, 1),
        (0x7F, 1),
        (0x80, 2),
        (0x7FF, 2),
        (0x800, 3),
        (0xD7FF, 3),
        (0xE000, 3),
        (0xFFFF, 3),
        (0x1_0000, 4),
        (0x10_FFFF, 4),
    ] {
        assert_eq!(utf8_of_codepoint(cp).1, len, "U+{cp:04X}");
        assert_eq!(
            char::from_u32(cp).unwrap().len_utf8(),
            len,
            "U+{cp:04X} (the oracle)"
        );
    }
}

/// The op itself on a hand-built image: a Text const with one codepoint of every width, `TextBytes` into
/// a fresh Seq, then its length and every byte Shown — the model's heap layout `[n, b0, …]` read back
/// through the ordinary Seq ops, so the header (written last) and the byte order are both observed.
#[test]
fn text_bytes_builds_the_utf8_seq_with_its_byte_count_header() {
    let src = "## Main\n    Let bytes be text_bytes(\"aé€😀\").\n    Show length of bytes.\n    Repeat for b in bytes:\n        Show b.\n";
    let words =
        crate::codegen_sva::full_asm::compile_to_full_words(src).expect("text_bytes encodes");
    assert!(
        words.iter().any(|w| (w >> 24) == 237),
        "the image must contain TextBytes (237)"
    );
    let got: Vec<String> = run_soft_core_typed(&words, 1_000_000)
        .iter()
        .map(ShowVal::fmt)
        .collect();
    let mut want = vec!["10".to_string()];
    want.extend("aé€😀".bytes().map(|b| b.to_string()));
    assert_eq!(got, want);
}
