//! Where a byte sits in source code, for the code contexts: the identifier being spelled (letters, digits and `_`, its
//! case kept) and the two before it, how deep the parentheses, braces and brackets are open, the classes of the last
//! tokens, the punctuation lately, the line's indentation and first identifier, and whether a comment or a string is
//! open. A statement's `;`, a call's `(` and `)` and an argument's `,` follow from the identifiers and the brackets
//! around them more than from the bytes just before, and the text model splits identifiers at every `_` and digit.

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_base/cm/code/tests.rs"]
mod tests;

use crate::entropy::{hash2, hash3};

/// The contexts a tracker gives.
pub const CONTEXTS: usize = 8;

/// The class of a token's byte: white space, an identifier, a number, an opening or a closing bracket, a separator
/// (`;` or `,`), a quote, a newline, anything else.
const SPACE: u64 = 0;
const IDENT: u64 = 1;
const NUMBER: u64 = 2;
const OPEN: u64 = 3;
const CLOSE: u64 = 4;
const SEPARATOR: u64 = 5;
const QUOTE: u64 = 6;
const NEWLINE: u64 = 7;
const OTHER: u64 = 8;

/// The comment open: none, `/* … */`, or `//` to the line's end.
const NO_COMMENT: u8 = 0;
const BLOCK_COMMENT: u8 = 1;
const LINE_COMMENT: u8 = 2;

#[derive(Clone)]
pub struct Code {
    /// The hash of the identifier being spelled (0 between identifiers), and of the last two before it.
    ident: u64,
    prev: u64,
    prev2: u64,
    /// Open parentheses, braces and brackets outside comments and strings, none below zero.
    paren: u64,
    brace: u64,
    square: u64,
    /// The classes of the last tokens, four bits each, newest lowest: a class enters when it differs from the last one,
    /// and every bracket, separator, quote or newline enters.
    classes: u64,
    /// The punctuation lately (brackets, separators, quotes, newlines and the rest), a byte each, newest lowest.
    punct: u64,
    /// The line's indentation (a tab counts eight) before its first identifier, and that identifier.
    indent: u64,
    first: u64,
    comment: u8,
    /// The quote of the string open (`"` or `'`), 0 when none is.
    string: u8,
}

impl Default for Code {
    fn default() -> Self {
        Self::new()
    }
}

impl Code {
    pub fn new() -> Self {
        Self { ident: 0, prev: 0, prev2: 0, paren: 0, brace: 0, square: 0, classes: 0, punct: 0, indent: 0, first: 0, comment: NO_COMMENT, string: 0 }
    }

    /// Read `byte`, `previous` being the byte before it.
    pub fn update(&mut self, byte: u8, previous: u8) {
        if self.comment == NO_COMMENT && self.string == 0 {
            if previous == b'/' && byte == b'*' {
                self.comment = BLOCK_COMMENT;
            } else if previous == b'/' && byte == b'/' {
                self.comment = LINE_COMMENT;
            } else if byte == b'"' || byte == b'\'' {
                self.string = byte;
            }
        } else if self.comment == BLOCK_COMMENT && previous == b'*' && byte == b'/' {
            self.comment = NO_COMMENT;
        } else if self.comment == LINE_COMMENT && byte == b'\n' {
            self.comment = NO_COMMENT;
        } else if self.string != 0 && byte == self.string && previous != b'\\' {
            self.string = 0;
        }
        let ident_byte = byte.is_ascii_alphanumeric() || byte == b'_';
        let class = if ident_byte {
            if byte.is_ascii_digit() && self.ident == 0 { NUMBER } else { IDENT }
        } else {
            match byte {
                b' ' | b'\t' => SPACE,
                b'(' | b'[' | b'{' => OPEN,
                b')' | b']' | b'}' => CLOSE,
                b';' | b',' => SEPARATOR,
                b'"' | b'\'' => QUOTE,
                b'\n' => NEWLINE,
                _ => OTHER,
            }
        };
        if ident_byte {
            self.ident = hash2(self.ident | 1, u64::from(byte));
        } else {
            if self.ident != 0 {
                self.prev2 = self.prev;
                self.prev = self.ident;
                if self.first == 0 {
                    self.first = self.ident;
                }
            }
            self.ident = 0;
            if class >= OPEN {
                self.punct = self.punct << 8 | u64::from(byte);
            }
        }
        if self.comment == NO_COMMENT && self.string == 0 {
            match byte {
                b'(' => self.paren += 1,
                b')' => self.paren = self.paren.saturating_sub(1),
                b'{' => self.brace += 1,
                b'}' => self.brace = self.brace.saturating_sub(1),
                b'[' => self.square += 1,
                b']' => self.square = self.square.saturating_sub(1),
                _ => {}
            }
        }
        if self.classes & 0xF != class || class >= OPEN {
            self.classes = (self.classes << 4 | class) & 0xFFFF_FFFF;
        }
        if byte == b'\n' {
            self.indent = 0;
            self.first = 0;
        } else if self.first == 0 && self.ident == 0 && (byte == b'\t' || byte == b' ') {
            self.indent += if byte == b'\t' { 8 } else { 1 };
        }
    }

    /// The contexts' keys after `byte`: the identifier so far with whether a comment or string is open; the last
    /// identifier with it; the bracket depths and the state with the last three token classes; the depths and the last
    /// two punctuation bytes with the last byte; the last identifier and punctuation with the identifier so far; the
    /// indentation and the last two classes with the line's first identifier; the last two identifiers with the one so
    /// far; and the depths and the state with the last identifier and the last byte.
    pub fn contexts(&self, byte: u8) -> [u64; CONTEXTS] {
        let b = u64::from(byte);
        let depth = self.paren.min(7) << 6 | self.brace.min(7) << 3 | self.square.min(7);
        let state = u64::from(self.comment) << 8 | u64::from(self.string);
        [
            hash3(0x434F_4431, self.ident, state),
            hash3(0x434F_4432, self.prev, self.ident),
            hash3(0x434F_4433, depth << 16 | state, self.classes & 0xFFF),
            hash3(0x434F_4434, depth << 16 | (self.punct & 0xFFFF), b),
            hash3(0x434F_4435, hash2(self.prev, self.punct & 0xFF), self.ident),
            hash3(0x434F_4436, self.indent.min(63) << 8 | (self.classes & 0xFF), self.first),
            hash3(0x434F_4437, hash2(self.prev2, self.prev), self.ident),
            hash3(0x434F_4438, depth << 16 | state, self.prev << 8 | b),
        ]
    }
}
