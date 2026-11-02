//! Pre-tokenization: a fragment of text split into words before merging, as llama.cpp's `unicode_regex_split` does.
//!
//! The patterns apply in order, each refining the spans the last one left (spans count codepoints). A pattern
//! llama.cpp has a hand-written splitter for runs that splitter ([`Custom`]); every other runs through
//! [`super::regex`], over the *collapsed* text when it names a Unicode category (`\p{L}` …, each non-ASCII codepoint
//! folded to one byte standing for its category) and over the codepoints otherwise. A word of a byte-encoded
//! vocabulary is then written in GPT-2's byte alphabet.

use super::regex::{Regex, RegexError, Width};
use super::unicode::{self, cpts_from_utf8, flags, flags_in, is_han, push_utf8, to_lower, Flags, InvalidCodepoint};

const OUT_OF_RANGE: u32 = 0xFFFF_FFFF;

/// The hand-written splitters, each standing for the pattern llama.cpp dispatches to it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Custom {
    Gpt2,
    Llama3,
    Qwen2,
    Qwen35,
    K2Horizon,
    KimiK2,
    Afmoe,
    Newlines,
}

impl Custom {
    /// The splitter llama.cpp's `unicode_regex_split_custom` runs for `pattern`, if any.
    pub fn for_pattern(pattern: &str) -> Option<Custom> {
        Some(match pattern {
            "'s|'t|'re|'ve|'m|'ll|'d| ?\\p{L}+| ?\\p{N}+| ?[^\\s\\p{L}\\p{N}]+|\\s+(?!\\S)" => Custom::Gpt2,
            "(?i:'s|'t|'re|'ve|'m|'ll|'d)|[^\\r\\n\\p{L}\\p{N}]?\\p{L}+|\\p{N}{1,3}| ?[^\\s\\p{L}\\p{N}]+[\\r\\n]*|\\s*[\\r\\n]+|\\s+(?!\\S)|\\s+"
            | "(?:'[sS]|'[tT]|'[rR][eE]|'[vV][eE]|'[mM]|'[lL][lL]|'[dD])|[^\\r\\n\\p{L}\\p{N}]?\\p{L}+|\\p{N}{1,3}| ?[^\\s\\p{L}\\p{N}]+[\\r\\n]*|\\s*[\\r\\n]+|\\s+(?!\\S)|\\s+" => Custom::Llama3,
            "(?:'[sS]|'[tT]|'[rR][eE]|'[vV][eE]|'[mM]|'[lL][lL]|'[dD])|[^\\r\\n\\p{L}\\p{N}]?\\p{L}+|\\p{N}| ?[^\\s\\p{L}\\p{N}]+[\\r\\n]*|\\s*[\\r\\n]+|\\s+(?!\\S)|\\s+" => Custom::Qwen2,
            "(?:'[sS]|'[tT]|'[rR][eE]|'[vV][eE]|'[mM]|'[lL][lL]|'[dD])|[^\\r\\n\\p{L}\\p{N}]?[\\p{L}\\p{M}]+|\\p{N}| ?[^\\s\\p{L}\\p{M}\\p{N}]+[\\r\\n]*|\\s*[\\r\\n]+|\\s+(?!\\S)|\\s+" => Custom::Qwen35,
            "(?:'[sS]|'[tT]|'[rR][eE]|'[vV][eE]|'[mM]|'[lL][lL]|'[dD])|[^\\r\\n\\p{L}\\p{N}]?(?:\\p{L}|\\p{M}|\\u200C|\\u200D)+|\\p{N}{1,3}| ?[^\\s\\p{L}\\p{N}]+[\\r\\n]*|\\s*[\\r\\n]+|\\s+(?!\\S)|\\s+" => Custom::K2Horizon,
            "\\p{Han}+" => Custom::KimiK2,
            "\\p{AFMoE_digits}" | "\\d{1,3}(?=(?:\\d{3})*\\b)" => Custom::Afmoe,
            "[^\\n]+|[\\n]+" => Custom::Newlines,
            _ => return None,
        })
    }

    fn split(self, cpts: &[u32], offsets: &[usize]) -> Vec<usize> {
        match self {
            Custom::Gpt2 => gpt2(cpts, offsets),
            Custom::Llama3 => llama3_family(cpts, offsets, Family::LLAMA3),
            Custom::Qwen2 => llama3_family(cpts, offsets, Family::QWEN2),
            Custom::Qwen35 => llama3_family(cpts, offsets, Family::QWEN35),
            Custom::K2Horizon => llama3_family(cpts, offsets, Family::K2_HORIZON),
            Custom::KimiK2 => kimi_k2(cpts, offsets),
            Custom::Afmoe => afmoe(cpts, offsets),
            Custom::Newlines => newlines(cpts, offsets),
        }
    }
}

/// The Unicode category escapes the collapsed text stands in for.
const CATEGORIES: [(&str, u16); 10] = [
    ("\\p{N}", Flags::NUMBER),
    ("\\p{L}", Flags::LETTER),
    ("\\p{P}", Flags::PUNCTUATION),
    ("\\p{M}", Flags::ACCENT_MARK),
    ("\\p{S}", Flags::SYMBOL),
    ("\\p{Lu}", Flags::LETTER),
    ("\\p{Ll}", Flags::LETTER),
    ("\\p{Lt}", Flags::LETTER),
    ("\\p{Lm}", Flags::LETTER),
    ("\\p{Lo}", Flags::LETTER),
];

/// The byte a category collapses to, and the ASCII members its class keeps (llama.cpp's `k_ucat_cpt`, `k_ucat_map`).
fn collapsed_category(cat: u16) -> Option<(u8, &'static [u8])> {
    Some(match cat {
        Flags::NUMBER => (0xD1, b"0-9"),
        Flags::LETTER => (0xD2, b"A-Za-z"),
        Flags::PUNCTUATION => (0xD3, b"!-#%-*,-/:-;?-@\\[-\\]_\\{\\}"),
        Flags::ACCENT_MARK => (0xD4, b""),
        Flags::SYMBOL => (0xD5, b"\\$\\+<->^`\\|\\~"),
        _ => return None,
    })
}

fn names_category(pattern: &str) -> bool {
    CATEGORIES.iter().any(|(k, _)| pattern.contains(k))
}

/// A pattern naming categories, rewritten over the collapsed alphabet.
fn collapse_pattern(pattern: &str) -> Result<Vec<u32>, RegexError> {
    let re = pattern.as_bytes();
    if !pattern.is_ascii() {
        return Err(RegexError {
            pattern: pattern.to_string(),
            at: 0,
            why: "a pattern names a Unicode category and holds a non-ASCII character",
        });
    }
    let mut out: Vec<u8> = Vec::with_capacity(re.len() * 2);
    let mut inside = false;
    let mut i = 0;
    while i < re.len() {
        if re[i] == b'[' && (i == 0 || re[i - 1] != b'\\') {
            out.push(b'[');
            inside = true;
            i += 1;
            continue;
        }
        if inside && re[i] == b']' && re[i - 1] != b'\\' {
            out.push(b']');
            inside = false;
            i += 1;
            continue;
        }
        if re[i] == b'\\' && i + 3 < re.len() && re[i + 1] == b'p' && re[i + 2] == b'{' {
            if let Some(close) = re[i + 3..].iter().position(|&c| c == b'}').map(|k| k + i + 3) {
                if close <= i + 10 {
                    let name = &pattern[i..=close];
                    if let Some(&(_, cat)) = CATEGORIES.iter().find(|(k, _)| *k == name) {
                        let (byte, ascii) = collapsed_category(cat).expect("every named category collapses");
                        if !inside {
                            out.push(b'[');
                        }
                        out.push(byte);
                        out.extend_from_slice(ascii);
                        if !inside {
                            out.push(b']');
                        }
                        i = close + 1;
                        continue;
                    }
                }
            }
        }
        out.push(re[i]);
        i += 1;
    }
    Ok(out.into_iter().map(u32::from).collect())
}

#[derive(Clone, Debug)]
enum Step {
    Custom(Custom),
    Collapsed(Regex),
    Wide(Regex),
}

/// A compiled pre-tokenizer: its patterns, in order, and whether words are byte-encoded.
#[derive(Clone, Debug)]
pub struct Splitter {
    steps: Vec<Step>,
    collapse: bool,
    byte_encode: bool,
}

impl Splitter {
    /// Compile the patterns of a pre-tokenizer; a pattern the regular-expression engine refuses is named.
    pub fn new(patterns: &[&str], byte_encode: bool) -> Result<Splitter, RegexError> {
        let mut steps = Vec::with_capacity(patterns.len());
        for p in patterns {
            steps.push(match Custom::for_pattern(p) {
                Some(c) => Step::Custom(c),
                None if names_category(p) => {
                    Step::Collapsed(Regex::from_units(&collapse_pattern(p)?, p, Width::Narrow)?)
                }
                None => Step::Wide(Regex::new(p, Width::Wide)?),
            });
        }
        Ok(Splitter { steps, collapse: patterns.iter().any(|p| names_category(p)), byte_encode })
    }

    /// The words `text` splits into, each byte-encoded when the vocabulary is.
    pub fn split(&self, text: &[u8]) -> Result<Vec<Vec<u8>>, InvalidCodepoint> {
        if text.is_empty() {
            return Ok(Vec::new());
        }
        let cpts = cpts_from_utf8(text);
        let collapsed: Vec<u32> = if self.collapse {
            cpts.iter()
                .map(|&c| {
                    if c < 128 {
                        return c;
                    }
                    let f = flags(c);
                    if f.is_whitespace() {
                        0x0B
                    } else {
                        collapsed_category(f.category()).map_or(0xD0, |(b, _)| b as u32)
                    }
                })
                .collect()
        } else {
            Vec::new()
        };
        let mut offsets = vec![cpts.len()];
        for step in &self.steps {
            offsets = match step {
                Step::Custom(c) => c.split(&cpts, &offsets),
                Step::Collapsed(re) => stl_split(re, &collapsed, &offsets),
                Step::Wide(re) => {
                    let wide: Vec<u32> =
                        cpts.iter().map(|&c| if c > 0x7F && flags(c).is_whitespace() { 0x0B } else { c }).collect();
                    stl_split(re, &wide, &offsets)
                }
            };
        }
        let mut words = Vec::with_capacity(offsets.len());
        let mut start = 0;
        for len in offsets {
            let mut w = Vec::new();
            for &c in &cpts[start..start + len] {
                push_utf8(c, &mut w)?;
            }
            start += len;
            if self.byte_encode {
                let mut enc = Vec::with_capacity(w.len() * 2);
                for b in w {
                    unicode::byte_to_utf8(b, &mut enc);
                }
                w = enc;
            }
            words.push(w);
        }
        Ok(words)
    }
}

/// `unicode_regex_split_stl`: each span split at the pattern's matches, the text between matches kept as words.
fn stl_split(re: &Regex, units: &[u32], offsets: &[usize]) -> Vec<usize> {
    let mut out = Vec::with_capacity(offsets.len());
    let mut start = 0;
    for &offset in offsets {
        let span = &units[start..start + offset];
        let mut at = 0;
        for (pos, len) in re.matches(span) {
            if pos > at {
                out.push(pos - at);
            }
            out.push(len);
            at = pos + len;
        }
        if at < offset {
            out.push(offset - at);
        }
        start += offset;
    }
    out
}

/// One span of the hand-written splitters: its bounds over `cpts`, and the words emitted so far.
struct Span<'a> {
    cpts: &'a [u32],
    lo: usize,
    hi: usize,
    prev_end: usize,
}

impl Span<'_> {
    fn cpt(&self, pos: usize) -> u32 {
        if self.lo <= pos && pos < self.hi {
            self.cpts[pos]
        } else {
            OUT_OF_RANGE
        }
    }

    fn flags(&self, pos: usize) -> Flags {
        flags_in(self.cpts, self.lo, self.hi, pos)
    }

    /// Emit the word ending at `end` (nothing when it is empty); its length.
    fn add(&mut self, end: usize, out: &mut Vec<usize>) -> usize {
        let len = end - self.prev_end;
        if len > 0 {
            out.push(len);
        }
        self.prev_end = end;
        len
    }
}

fn spans<'a>(cpts: &'a [u32], offsets: &'a [usize]) -> impl Iterator<Item = Span<'a>> + 'a {
    let mut start = 0;
    offsets.iter().map(move |&offset| {
        let s = Span { cpts, lo: start, hi: start + offset, prev_end: start };
        start += offset;
        s
    })
}

/// GPT-2: `'s|'t|'re|'ve|'m|'ll|'d| ?\p{L}+| ?\p{N}+| ?[^\s\p{L}\p{N}]+|\s+(?!\S)|\s+`.
fn gpt2(cpts: &[u32], offsets: &[usize]) -> Vec<usize> {
    let mut out = Vec::with_capacity(offsets.len());
    for mut s in spans(cpts, offsets) {
        let mut pos = s.lo;
        while pos < s.hi {
            let cpt = s.cpt(pos);
            let flags = s.flags(pos);
            if cpt == '\'' as u32 && pos + 1 < s.hi {
                let next = s.cpt(pos + 1);
                if [b's', b't', b'm', b'd'].iter().any(|&c| next == c as u32) {
                    pos += s.add(pos + 2, &mut out);
                    continue;
                }
                if pos + 2 < s.hi {
                    let nn = s.cpt(pos + 2);
                    if (next == 'r' as u32 && nn == 'e' as u32)
                        || (next == 'v' as u32 && nn == 'e' as u32)
                        || (next == 'l' as u32 && nn == 'l' as u32)
                    {
                        pos += s.add(pos + 3, &mut out);
                        continue;
                    }
                }
            }
            let mut flags2 = if cpt == ' ' as u32 { s.flags(pos + 1) } else { flags };
            if flags2.is_letter() {
                pos += (cpt == ' ' as u32) as usize;
                while flags2.is_letter() {
                    pos += 1;
                    flags2 = s.flags(pos);
                }
                s.add(pos, &mut out);
                continue;
            }
            if flags2.is_number() {
                pos += (cpt == ' ' as u32) as usize;
                while flags2.is_number() {
                    pos += 1;
                    flags2 = s.flags(pos);
                }
                s.add(pos, &mut out);
                continue;
            }
            let other = |f: Flags| !(f.is_whitespace() || f.is_letter() || f.is_number()) && f.0 != 0;
            if other(flags2) {
                pos += (cpt == ' ' as u32) as usize;
                while other(flags2) {
                    pos += 1;
                    flags2 = s.flags(pos);
                }
                s.add(pos, &mut out);
                continue;
            }
            let mut ws = 0;
            while s.flags(pos + ws).is_whitespace() {
                ws += 1;
            }
            if ws > 1 && s.cpt(pos + ws) != OUT_OF_RANGE {
                pos += ws - 1;
                s.add(pos, &mut out);
                continue;
            }
            if ws > 0 {
                pos += ws;
                s.add(pos, &mut out);
                continue;
            }
            pos += 1;
            s.add(pos, &mut out);
        }
    }
    out
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum WordRun {
    /// `\p{L}+`
    Letters,
    /// `[\p{L}\p{M}]+`
    LettersMarks,
    /// `(?:\p{L}|\p{M}|‌|‍)+`
    LettersMarksJoiners,
}

/// The splitters llama.cpp writes for the Llama 3 pattern and its relatives: they differ in what a word run holds,
/// whether digits group by three, whether a run of punctuation stops at marks, and K2-Horizon's long-s fold.
#[derive(Clone, Copy)]
struct Family {
    word: WordRun,
    digits_by_three: bool,
    punct_stops_at_marks: bool,
    long_s: bool,
}

impl Family {
    const LLAMA3: Family =
        Family { word: WordRun::Letters, digits_by_three: true, punct_stops_at_marks: false, long_s: false };
    const QWEN2: Family =
        Family { word: WordRun::Letters, digits_by_three: false, punct_stops_at_marks: false, long_s: false };
    const QWEN35: Family =
        Family { word: WordRun::LettersMarks, digits_by_three: false, punct_stops_at_marks: true, long_s: false };
    const K2_HORIZON: Family =
        Family { word: WordRun::LettersMarksJoiners, digits_by_three: true, punct_stops_at_marks: false, long_s: true };
}

/// `(?i:'s|'t|'re|'ve|'m|'ll|'d)|[^\r\n\p{L}\p{N}]?<word>+|\p{N}{1,3}| ?[^\s\p{L}\p{N}]+[\r\n]*|\s*[\r\n]+|\s+(?!\S)|\s+`
/// and its relatives.
fn llama3_family(cpts: &[u32], offsets: &[usize], fam: Family) -> Vec<usize> {
    let mut out = Vec::with_capacity(offsets.len());
    for mut s in spans(cpts, offsets) {
        let word = |s: &Span, p: usize| -> bool {
            let f = s.flags(p);
            match fam.word {
                WordRun::Letters => f.is_letter(),
                WordRun::LettersMarks => f.is_letter() || f.is_accent_mark(),
                WordRun::LettersMarksJoiners => {
                    let c = s.cpt(p);
                    c == 0x200C || c == 0x200D || f.is_letter() || f.is_accent_mark()
                }
            }
        };
        let mut pos = s.lo;
        while pos < s.hi {
            let cpt = s.cpt(pos);
            let flags = s.flags(pos);
            if cpt == '\'' as u32 && pos + 1 < s.hi {
                let mut next = to_lower(s.cpt(pos + 1));
                if fam.long_s && next == 0x017F {
                    next = 's' as u32;
                }
                if [b's', b't', b'm', b'd'].iter().any(|&c| next == c as u32) {
                    pos += s.add(pos + 2, &mut out);
                    continue;
                }
                if pos + 2 < s.hi {
                    let nn = to_lower(s.cpt(pos + 2));
                    if (next == 'r' as u32 && nn == 'e' as u32)
                        || (next == 'v' as u32 && nn == 'e' as u32)
                        || (next == 'l' as u32 && nn == 'l' as u32)
                    {
                        pos += s.add(pos + 3, &mut out);
                        continue;
                    }
                }
            }
            if !(cpt == '\r' as u32 || cpt == '\n' as u32 || flags.is_number()) && (word(&s, pos) || word(&s, pos + 1)) {
                pos += 1;
                while word(&s, pos) {
                    pos += 1;
                }
                s.add(pos, &mut out);
                continue;
            }
            if flags.is_number() {
                if fam.digits_by_three {
                    let mut ini = pos;
                    while s.flags(pos).is_number() {
                        pos += 1;
                        if pos - ini >= 3 {
                            s.add(pos, &mut out);
                            ini = pos;
                        }
                    }
                } else {
                    pos += 1;
                }
                s.add(pos, &mut out);
                continue;
            }
            let excluded = |f: Flags| {
                f.is_whitespace() || f.is_letter() || f.is_number() || (fam.punct_stops_at_marks && f.is_accent_mark())
            };
            let mut flags2 = if cpt == ' ' as u32 { s.flags(pos + 1) } else { flags };
            if !excluded(flags2) && flags.0 != 0 {
                pos += (cpt == ' ' as u32) as usize;
                while !excluded(flags2) && flags2.0 != 0 {
                    pos += 1;
                    flags2 = s.flags(pos);
                }
                let mut c2 = s.cpt(pos);
                while c2 == '\r' as u32 || c2 == '\n' as u32 {
                    pos += 1;
                    c2 = s.cpt(pos);
                }
                s.add(pos, &mut out);
                continue;
            }
            let mut ws = 0;
            let mut last_rn = 0;
            while s.flags(pos + ws).is_whitespace() {
                let c2 = s.cpt(pos + ws);
                if c2 == '\r' as u32 || c2 == '\n' as u32 {
                    last_rn = pos + ws + 1;
                }
                ws += 1;
            }
            if last_rn > 0 {
                pos = last_rn;
                s.add(pos, &mut out);
                continue;
            }
            if ws > 1 && s.cpt(pos + ws) != OUT_OF_RANGE {
                pos += ws - 1;
                s.add(pos, &mut out);
                continue;
            }
            if ws > 0 {
                pos += ws;
                s.add(pos, &mut out);
                continue;
            }
            pos += 1;
            s.add(pos, &mut out);
        }
    }
    out
}

/// Kimi-K2: Han runs, letter words excluding Han with optional contractions, digits by three, punctuation,
/// whitespace.
fn kimi_k2(cpts: &[u32], offsets: &[usize]) -> Vec<usize> {
    let mut out = Vec::with_capacity(offsets.len());
    for mut s in spans(cpts, offsets) {
        let letter = |s: &Span, p: usize| s.flags(p).is_letter() && !is_han(s.cpt(p));
        let mut pos = s.lo;
        while pos < s.hi {
            let cpt = s.cpt(pos);
            let flags = s.flags(pos);
            if is_han(cpt) {
                while is_han(s.cpt(pos)) {
                    pos += 1;
                }
                s.add(pos, &mut out);
                continue;
            }
            let lead_ok = !(cpt == '\r' as u32 || cpt == '\n' as u32 || flags.is_letter() || flags.is_number());
            if (flags.is_letter() && !is_han(cpt)) || (lead_ok && letter(&s, pos + 1)) {
                let mut has_leading = false;
                if lead_ok {
                    has_leading = true;
                    pos += 1;
                }
                let mut has_letters = false;
                while letter(&s, pos) {
                    has_letters = true;
                    pos += 1;
                }
                if has_letters || (!has_leading && letter(&s, pos)) {
                    if !has_letters {
                        pos += 1;
                    }
                    while letter(&s, pos) {
                        pos += 1;
                    }
                    if s.cpt(pos) == '\'' as u32 && pos + 1 < s.hi {
                        let next = to_lower(s.cpt(pos + 1));
                        if [b's', b't', b'm', b'd'].iter().any(|&c| next == c as u32) {
                            pos += 2;
                        } else if pos + 2 < s.hi {
                            let nn = to_lower(s.cpt(pos + 2));
                            if (next == 'r' as u32 && nn == 'e' as u32)
                                || (next == 'v' as u32 && nn == 'e' as u32)
                                || (next == 'l' as u32 && nn == 'l' as u32)
                            {
                                pos += 3;
                            }
                        }
                    }
                    s.add(pos, &mut out);
                    continue;
                } else if has_leading {
                    pos -= 1;
                }
            }
            if flags.is_number() {
                let mut ini = pos;
                while s.flags(pos).is_number() {
                    pos += 1;
                    if pos - ini >= 3 {
                        s.add(pos, &mut out);
                        ini = pos;
                    }
                }
                s.add(pos, &mut out);
                continue;
            }
            let other = |f: Flags| !(f.is_whitespace() || f.is_letter() || f.is_number()) && f.0 != 0;
            let mut flags2 = if cpt == ' ' as u32 { s.flags(pos + 1) } else { flags };
            if other(flags2) {
                pos += (cpt == ' ' as u32) as usize;
                while other(flags2) {
                    pos += 1;
                    flags2 = s.flags(pos);
                }
                let mut c2 = s.cpt(pos);
                while c2 == '\r' as u32 || c2 == '\n' as u32 {
                    pos += 1;
                    c2 = s.cpt(pos);
                }
                s.add(pos, &mut out);
                continue;
            }
            let mut ws = 0;
            let mut last_rn = 0;
            while s.flags(pos + ws).is_whitespace() {
                let c2 = s.cpt(pos + ws);
                if c2 == '\r' as u32 || c2 == '\n' as u32 {
                    last_rn = pos + ws + 1;
                }
                ws += 1;
            }
            if last_rn > 0 {
                pos = last_rn;
                s.add(pos, &mut out);
                continue;
            }
            if ws > 1 && s.cpt(pos + ws) != OUT_OF_RANGE {
                pos += ws - 1;
                s.add(pos, &mut out);
                continue;
            }
            if ws > 0 {
                pos += ws;
                s.add(pos, &mut out);
                continue;
            }
            pos += 1;
            s.add(pos, &mut out);
        }
    }
    out
}

/// AFMoE digits: a digit run leads with its length mod 3 digits, then groups of three. The text before a run joins
/// the run's first group, as llama.cpp's splitter emits it.
fn afmoe(cpts: &[u32], offsets: &[usize]) -> Vec<usize> {
    let mut out = Vec::with_capacity(offsets.len());
    for mut s in spans(cpts, offsets) {
        let mut pos = s.lo;
        while pos < s.hi {
            if s.flags(pos).is_number() {
                let digit_start = pos;
                let mut count = 0;
                while s.flags(pos).is_number() && pos < s.hi {
                    count += 1;
                    pos += 1;
                }
                let rem = count % 3;
                let mut cur = digit_start;
                if rem > 0 {
                    s.add(cur + rem, &mut out);
                    cur += rem;
                }
                while cur < digit_start + count {
                    s.add(cur + 3, &mut out);
                    cur += 3;
                }
                continue;
            }
            pos += 1;
        }
        if s.prev_end < s.hi {
            let hi = s.hi;
            s.add(hi, &mut out);
        }
    }
    out
}

/// `[^\n]+|[\n]+`: runs of non-newlines and runs of newlines.
fn newlines(cpts: &[u32], offsets: &[usize]) -> Vec<usize> {
    let mut out = Vec::with_capacity(offsets.len());
    let mut start = 0;
    for &offset in offsets {
        let (lo, hi) = (start, start + offset);
        start = hi;
        let mut pos = lo;
        while pos < hi {
            let nl = cpts[pos] == '\n' as u32;
            let run = pos;
            while pos < hi && (cpts[pos] == '\n' as u32) == nl {
                pos += 1;
            }
            out.push(pos - run);
        }
    }
    out
}
