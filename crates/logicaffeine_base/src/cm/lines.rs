//! Where a byte sits in its line, for the line contexts: how many bytes in a row this line has matched the line above
//! at the same columns, whether the last byte did, and the kind of byte the line began with — with what the model
//! keeps of the lines around (the column, the bytes above and two lines up, the last two lines' lengths, where this
//! line began). Fixed-width records repeat by column; a line that has matched the one above so far tends to go on
//! matching it; and a line's own bytes so far say what it is about.

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_base/cm/lines/tests.rs"]
mod tests;

use crate::entropy::{hash2, hash3};

/// The contexts a tracker gives.
pub const CONTEXTS: usize = 12;
/// The longest run of matching bytes counted.
const LONGEST_MATCH: i32 = 16;

/// What the model knows of the lines around the next byte. A byte above is 256 plus its value, 0 where that line is
/// too short.
pub struct Around {
    pub column: usize,
    pub above: u64,
    pub above_left: u64,
    pub above2: u64,
    pub previous_length: u64,
    pub previous2_length: u64,
    pub line_start: u64,
    /// The classes of the last three bytes, three bits each.
    pub classes: u64,
    /// Whether a capital came since the line began.
    pub capital: bool,
}

pub struct Lines {
    /// Bytes in a row, from the line's start, that matched the line above at the same column (0 at a new line), or −1
    /// once one did not.
    matched: i32,
    /// Whether the last byte matched the byte above it.
    hit: bool,
    /// The class of the first byte on this line that is not white space, plus one (0 before it).
    first: u64,
}

impl Default for Lines {
    fn default() -> Self {
        Self::new()
    }
}

impl Lines {
    pub fn new() -> Self {
        Self { matched: 0, hit: false, first: 0 }
    }

    /// Read `byte` (of class `class`), the lines around it as the model now sees them (`around.column` is the next
    /// byte's: 0 after a newline).
    pub fn update(&mut self, byte: u8, class: u64, around: &Around) {
        self.hit = around.column > 0 && around.above_left == (u64::from(byte) | 256);
        self.matched = if around.column == 0 {
            0
        } else if self.matched >= 0 && self.hit {
            (self.matched + 1).min(LONGEST_MATCH)
        } else {
            -1
        };
        if around.column == 0 {
            self.first = 0;
        } else if self.first == 0 && !matches!(byte, b' ' | b'\t' | b'\r') {
            self.first = class + 1;
        }
    }

    /// The contexts' keys after `byte` (of class `class`): the byte above with the run matched so far; the previous
    /// line's length with the column and the byte above (and whether the last byte matched its own), with the last
    /// byte's class and with the last byte; the byte above with the last classes where the two lines above agree there,
    /// and the bytes above and two up with them; the column with whether the last byte is a space, with the last byte,
    /// with the last classes, with the previous line's length; the column with the line's first class, whether a
    /// capital came since and the last class; and the line so far, alone and with the last byte.
    pub fn contexts(&self, byte: u8, class: u64, around: &Around) -> [u64; CONTEXTS] {
        let b = u64::from(byte);
        let column = around.column.min(255) as u64;
        let above = around.above << 1 | u64::from(self.hit);
        let length = around.previous_length << 16 | column;
        [
            if self.matched >= 0 { hash3(0x4C4D_4154, around.above, self.matched as u64) } else { 0x4C4D_4154 },
            hash3(0x4C4C_4731, length, above << 8 | class),
            hash3(0x4C4C_4732, length, above << 8 | b),
            if around.above == around.above2 && around.previous_length == around.previous2_length {
                hash3(0x4C46_4931, around.classes, around.above)
            } else {
                0x4C46_4931
            },
            hash3(0x4C46_4932, around.classes, around.above << 9 | around.above2),
            hash2(0x4C43_5331, column << 1 | u64::from(byte == b' ')),
            hash2(0x4C43_4331, column << 8 | b),
            hash3(0x4C43_4D31, column, around.classes),
            hash3(0x4C43_4C31, column, around.previous_length),
            hash3(0x4C43_4631, column << 8 | self.first, u64::from(around.capital) << 8 | class),
            hash2(0x4C4F_4C30, around.line_start),
            hash3(0x4C4F_4C31, around.line_start, b),
        ]
    }
}
