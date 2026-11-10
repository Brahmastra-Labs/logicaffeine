//! Where text sits in its prose, for the text contexts: the words before this one in the same sentence (none at its
//! start), the phrase it continues (words joined by single spaces — quotes and apostrophes passed over, any other mark
//! ends a phrase), the current word's pattern of letter kinds (vowel, e, y, th, other consonant) with its length, the
//! quote or bracket last opened, the keyword that introduced the section (the word before the last `:` or `=`), the
//! paragraph it is in (from its start: what this paragraph has held), the kind of byte its line began with and the
//! column, and the groups of the last bytes read (lower, upper, digit, non-ASCII, or the byte itself). A sentence's
//! words predict the next one better than the words before its start; a phrase is the stretch over which word n-grams
//! hold; the letter pattern carries a word's shape where its spelling is new; a paragraph keeps to its subject.

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_base/cm/prose/tests.rs"]
mod tests;

use crate::entropy::{hash2, hash3};

/// The phrase words a tracker keeps, the current one first.
const PHRASE: usize = 5;
/// The contexts a tracker gives.
pub const CONTEXTS: usize = 18;

pub struct Prose {
    /// The current word so far (lowercase letters, hashed; 0 between words) and its length.
    word: u64,
    len: u32,
    /// The two words before the current one in this sentence, the nearer first.
    sentence: [u64; 2],
    phrase: [u64; PHRASE],
    /// The kinds of the current word's last four letters, a byte each, the latest lowest.
    pattern: u32,
    opened: u8,
    keyword: u64,
    /// Bytes read; where the current paragraph began (0 for the first).
    read: u64,
    paragraph: u64,
    /// Newlines since the last byte that is not white space: two make a paragraph break.
    newlines: u32,
    /// The group of the first byte on this line that is not white space (0 before it), and the bytes since the newline.
    line_first: u8,
    column: u32,
    /// The groups of the last eight bytes, a byte each, the latest lowest.
    groups: u64,
    last: u8,
}

impl Default for Prose {
    fn default() -> Self {
        Self::new()
    }
}

/// A letter's kind for the word pattern: `e`, another vowel (`a`), `y`, or another consonant (`b`).
fn kind(lower: u8) -> u8 {
    match lower {
        b'e' => b'e',
        b'a' | b'i' | b'o' | b'u' => b'a',
        b'y' => b'y',
        _ => b'b',
    }
}

/// A byte's group for the group history: lower, upper, digit, non-ASCII, or the byte itself.
fn group(b: u8) -> u8 {
    match b {
        b'a'..=b'z' => b'a',
        b'A'..=b'Z' => b'A',
        b'0'..=b'9' => b'0',
        128..=255 => 128,
        _ => b,
    }
}

impl Prose {
    pub fn new() -> Self {
        Self {
            word: 0,
            len: 0,
            sentence: [0; 2],
            phrase: [0; PHRASE],
            pattern: 0,
            opened: 0,
            keyword: 0,
            read: 0,
            paragraph: 0,
            newlines: 0,
            line_first: 0,
            column: 0,
            groups: 0,
            last: 0,
        }
    }

    pub fn update(&mut self, b: u8) {
        let lower = b.to_ascii_lowercase();
        if b.is_ascii_alphabetic() {
            // `th` is one kind: the `t` already counted becomes it.
            self.pattern = if self.word != 0 && lower == b'h' && self.last.to_ascii_lowercase() == b't' {
                self.pattern & !0xFF | u32::from(b't')
            } else {
                (if self.word == 0 { 0 } else { self.pattern }) << 8 | u32::from(kind(lower))
            };
            self.word = hash2(self.word, u64::from(lower));
            self.len += 1;
            self.phrase[0] = hash2(self.phrase[0], u64::from(lower));
        } else {
            if self.word != 0 {
                self.sentence = [self.word, self.sentence[0]];
                if b == b':' || b == b'=' {
                    self.keyword = self.word;
                }
                self.word = 0;
                self.len = 0;
            }
            if matches!(b, b' ' | b'\n' | b'\r') {
                if self.phrase[0] != 0 {
                    self.phrase.copy_within(..PHRASE - 1, 1);
                    self.phrase[0] = 0;
                }
            } else if !matches!(b, b'"' | b'\'') {
                self.phrase = [0; PHRASE];
            }
            match b {
                b'.' | b'!' | b'?' => self.sentence = [0; 2],
                b'(' | b'[' | b'{' | b'"' => self.opened = b,
                b')' | b']' | b'}' => self.opened = 0,
                _ => {}
            }
        }
        self.read += 1;
        if b == b'\n' {
            self.newlines += 1;
            if self.newlines >= 2 {
                self.paragraph = self.read;
            }
            self.line_first = 0;
            self.column = 0;
        } else {
            if !matches!(b, b'\r' | b' ' | b'\t') {
                self.newlines = 0;
                if self.line_first == 0 {
                    self.line_first = group(b);
                }
            }
            self.column = self.column.saturating_add(1);
        }
        self.groups = self.groups << 8 | u64::from(group(b));
        self.last = b;
    }

    /// The contexts' keys given the last two bytes, the last two letters across word breaks, the current gap token
    /// (the bytes since the last word) and how long ago the previous word last occurred: the word with the sentence's
    /// previous one, and with its previous two; the phrase's last five words, and its last three; the letter pattern
    /// with the word's length, and with the last byte too, and with the last two letters; the opened quote or bracket
    /// with the word, with the last byte and the previous word's recurrence, and with the letter pattern; the keyword
    /// with the gap token, and the word and last byte with the keyword; the paragraph alone and with the last byte; the
    /// line's first group with the last byte and the column; the last eight groups, the last six with the last byte,
    /// and the last four with the last two bytes.
    pub fn contexts(&self, c1: u8, c2: u8, letters2: u64, gap: u64, recurrence: u64) -> [u64; CONTEXTS] {
        let (c1, c2) = (u64::from(c1), u64::from(c2));
        let len = u64::from(self.len.min(15));
        let pattern = u64::from(self.pattern);
        let opened = u64::from(self.opened);
        let p = &self.phrase;
        [
            hash3(0x5357_3031, self.word, self.sentence[0]),
            hash3(0x5357_3032, hash2(self.word, self.sentence[0]), self.sentence[1]),
            hash3(0x4558_5035, hash2(hash2(p[0], p[1]), p[2]), hash2(p[3], p[4])),
            hash3(0x4558_5033, hash2(p[0], p[1]), p[2]),
            hash3(0x4D41_534B, len, pattern),
            hash3(0x4D41_5343, len << 32 | pattern, c1),
            hash3(0x4D41_534C, pattern, letters2),
            hash3(0x4F50_4E57, opened, self.word),
            hash3(0x4F50_4E43, opened << 8 | c1, recurrence),
            hash3(0x4F50_4D4B, opened, len << 32 | pattern),
            hash3(0x4B45_5947, self.keyword, gap),
            hash3(0x4B45_5957, self.word << 8 | c1, self.keyword),
            hash2(0x5041_4431, self.paragraph),
            hash3(0x5041_4432, self.paragraph, c1),
            hash3(0x4C46_4331, u64::from(self.line_first), c1 << 8 | u64::from(self.column.min(63))),
            hash2(0x4752_5031, self.groups),
            hash3(0x4752_5033, self.groups & 0xFFFF_FFFF_FFFF, c1),
            hash3(0x4752_5032, self.groups & 0xFFFF_FFFF, c1 << 8 | c2),
        ]
    }
}
