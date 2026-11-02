//! The codepoint facts the tokenizers read, as llama.cpp computes them (`src/unicode.cpp`): category flags, the
//! lowercase map, the lenient UTF-8 decoder (an invalid byte decodes to U+FFFD and advances one byte), and GPT-2's
//! byte-to-codepoint map. The tables in `unicode_tables` are generated from llama.cpp's running code and
//! held equal to it for every codepoint by `model_tokenizer_lock`.
//!
//! The flags kept are the ones a tokenizer reads: the category and whitespace bits. llama.cpp's case and NFD bits
//! never change a split (every codepoint carries exactly one category bit, so a flag word is non-zero either way).

use std::sync::OnceLock;

use super::unicode_tables::{FLAG_RANGES, LOWERCASE_RUNS};

/// The number of codepoints the flag table covers; a larger value reads as undefined.
pub const MAX_CODEPOINTS: u32 = 0x110000;

/// A codepoint's flags, bit for bit llama.cpp's `unicode_cpt_flags`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Flags(pub u16);

impl Flags {
    pub const UNDEFINED: u16 = 0x0001;
    pub const NUMBER: u16 = 0x0002;
    pub const LETTER: u16 = 0x0004;
    pub const SEPARATOR: u16 = 0x0008;
    pub const ACCENT_MARK: u16 = 0x0010;
    pub const PUNCTUATION: u16 = 0x0020;
    pub const SYMBOL: u16 = 0x0040;
    pub const CONTROL: u16 = 0x0080;
    pub const MASK_CATEGORIES: u16 = 0x00FF;
    pub const WHITESPACE: u16 = 0x0100;
    /// Every bit the table keeps.
    pub const KEPT: u16 = 0x01FF;

    pub fn is_undefined(self) -> bool {
        self.0 & Self::UNDEFINED != 0
    }
    pub fn is_number(self) -> bool {
        self.0 & Self::NUMBER != 0
    }
    pub fn is_letter(self) -> bool {
        self.0 & Self::LETTER != 0
    }
    pub fn is_separator(self) -> bool {
        self.0 & Self::SEPARATOR != 0
    }
    pub fn is_accent_mark(self) -> bool {
        self.0 & Self::ACCENT_MARK != 0
    }
    pub fn is_punctuation(self) -> bool {
        self.0 & Self::PUNCTUATION != 0
    }
    pub fn is_symbol(self) -> bool {
        self.0 & Self::SYMBOL != 0
    }
    pub fn is_control(self) -> bool {
        self.0 & Self::CONTROL != 0
    }
    pub fn is_whitespace(self) -> bool {
        self.0 & Self::WHITESPACE != 0
    }

    /// The category bits alone (`category_flag()`).
    pub fn category(self) -> u16 {
        self.0 & Self::MASK_CATEGORIES
    }
}

/// The flags of `cpt`; past the table every codepoint is undefined. A table entry packs a range's first codepoint
/// above its nine flag bits.
pub fn flags(cpt: u32) -> Flags {
    if cpt >= MAX_CODEPOINTS {
        return Flags(Flags::UNDEFINED);
    }
    let i = FLAG_RANGES.partition_point(|&e| e >> 9 <= cpt) - 1;
    Flags((FLAG_RANGES[i] & 0x1FF) as u16)
}

/// The flags of the codepoint at `pos` of `cpts[lo..hi]`, or none outside it (`_get_flags` in the splitters).
pub fn flags_in(cpts: &[u32], lo: usize, hi: usize, pos: usize) -> Flags {
    if lo <= pos && pos < hi {
        flags(cpts[pos])
    } else {
        Flags(0)
    }
}

/// `unicode_tolower`: the simple lowercase mapping, or the codepoint itself. The map is stored as runs
/// `(first, count, stride, delta)`: `first + k·stride` lowercases to itself plus `delta` for `k < count`.
pub fn to_lower(cpt: u32) -> u32 {
    let i = LOWERCASE_RUNS.partition_point(|r| r.0 <= cpt);
    if i == 0 {
        return cpt;
    }
    let (first, count, stride, delta) = LOWERCASE_RUNS[i - 1];
    let off = cpt - first;
    if off % stride == 0 && off / stride < count {
        (cpt as i64 + delta as i64) as u32
    } else {
        cpt
    }
}

/// `unicode_cpt_is_han`: the CJK ideograph blocks the Kimi-K2 pre-tokenizer splits on.
pub fn is_han(cpt: u32) -> bool {
    matches!(cpt,
        0x4E00..=0x9FFF
        | 0x3400..=0x4DBF
        | 0x20000..=0x2A6DF
        | 0x2A700..=0x2B73F
        | 0x2B740..=0x2B81F
        | 0x2B820..=0x2CEAF
        | 0x2CEB0..=0x2EBEF
        | 0xF900..=0xFAFF
        | 0x2F800..=0x2FA1F)
}

/// `unicode_len_utf8`: the sequence length a lead byte announces (a continuation byte counts as one).
pub fn len_utf8(lead: u8) -> usize {
    const LOOKUP: [usize; 16] = [1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 2, 2, 3, 4];
    LOOKUP[(lead >> 4) as usize]
}

/// `unicode_cpt_from_utf8`: one codepoint, or `None` for an invalid sequence. Overlong forms and surrogates decode
/// as written, the way llama.cpp's decoder reads them.
fn cpt_from_utf8(b: &[u8], offset: &mut usize) -> Option<u32> {
    let o = *offset;
    let b0 = b[o] as u32;
    let cont = |k: usize| b.get(o + k).is_some_and(|&c| c & 0xC0 == 0x80);
    if b0 & 0x80 == 0 {
        *offset += 1;
        return Some(b0);
    }
    if b0 & 0x40 == 0 {
        return None;
    }
    if b0 & 0x20 == 0 {
        if !cont(1) {
            return None;
        }
        *offset += 2;
        return Some(((b0 & 0x1F) << 6) | (b[o + 1] as u32 & 0x3F));
    }
    if b0 & 0x10 == 0 {
        if !(cont(1) && cont(2)) {
            return None;
        }
        *offset += 3;
        return Some(((b0 & 0x0F) << 12) | ((b[o + 1] as u32 & 0x3F) << 6) | (b[o + 2] as u32 & 0x3F));
    }
    if b0 & 0x08 == 0 {
        if !(cont(1) && cont(2) && cont(3)) {
            return None;
        }
        *offset += 4;
        return Some(
            ((b0 & 0x07) << 18) | ((b[o + 1] as u32 & 0x3F) << 12) | ((b[o + 2] as u32 & 0x3F) << 6) | (b[o + 3] as u32 & 0x3F),
        );
    }
    None
}

/// `unicode_cpts_from_utf8`: every codepoint of `text`, an invalid byte reading as U+FFFD.
pub fn cpts_from_utf8(text: &[u8]) -> Vec<u32> {
    let mut out = Vec::with_capacity(text.len());
    let mut offset = 0;
    while offset < text.len() {
        match cpt_from_utf8(text, &mut offset) {
            Some(c) => out.push(c),
            None => {
                offset += 1;
                out.push(0xFFFD);
            }
        }
    }
    out
}

/// A codepoint no UTF-8 sequence encodes: llama.cpp's `unicode_cpt_to_utf8` throws on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InvalidCodepoint(pub u32);

/// `unicode_cpt_to_utf8`, appending to `out`. Surrogates encode as their three bytes.
pub fn push_utf8(cpt: u32, out: &mut Vec<u8>) -> Result<(), InvalidCodepoint> {
    match cpt {
        0..=0x7F => out.push(cpt as u8),
        0x80..=0x7FF => out.extend_from_slice(&[0xC0 | ((cpt >> 6) & 0x1F) as u8, 0x80 | (cpt & 0x3F) as u8]),
        0x800..=0xFFFF => out.extend_from_slice(&[
            0xE0 | ((cpt >> 12) & 0x0F) as u8,
            0x80 | ((cpt >> 6) & 0x3F) as u8,
            0x80 | (cpt & 0x3F) as u8,
        ]),
        0x10000..=0x10FFFF => out.extend_from_slice(&[
            0xF0 | ((cpt >> 18) & 0x07) as u8,
            0x80 | ((cpt >> 12) & 0x3F) as u8,
            0x80 | ((cpt >> 6) & 0x3F) as u8,
            0x80 | (cpt & 0x3F) as u8,
        ]),
        _ => return Err(InvalidCodepoint(cpt)),
    }
    Ok(())
}

/// GPT-2's byte-level alphabet: the codepoint each byte is written as inside a byte-encoded token.
fn byte_cpts() -> &'static [u32; 256] {
    static MAP: OnceLock<[u32; 256]> = OnceLock::new();
    MAP.get_or_init(|| {
        let mut map = [u32::MAX; 256];
        for ch in (0x21..=0x7E).chain(0xA1..=0xAC).chain(0xAE..=0xFF) {
            map[ch as usize] = ch;
        }
        let mut n = 0;
        for ch in 0..256 {
            if map[ch] == u32::MAX {
                map[ch] = 256 + n;
                n += 1;
            }
        }
        map
    })
}

/// `unicode_byte_to_utf8`: the UTF-8 of the codepoint byte `b` is written as.
pub fn byte_to_utf8(b: u8, out: &mut Vec<u8>) {
    push_utf8(byte_cpts()[b as usize], out).expect("the byte alphabet is below U+0200");
}

/// `unicode_utf8_to_byte`: the byte a byte-level codepoint stands for, when it is one.
pub fn utf8_to_byte(cpt: u32) -> Option<u8> {
    static INVERSE: OnceLock<std::collections::HashMap<u32, u8>> = OnceLock::new();
    INVERSE
        .get_or_init(|| byte_cpts().iter().enumerate().map(|(b, &c)| (c, b as u8)).collect())
        .get(&cpt)
        .copied()
}
