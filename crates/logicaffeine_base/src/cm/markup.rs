//! Where text sits in its markup — HTML, XML or SGML — for the text contexts: inside a tag's name, among its
//! attributes, in an attribute's name or value, between tags, or in a comment or declaration; the elements open around
//! it; and the element last closed. A closing tag's name is then nearly certain (the innermost open element's), an
//! attribute's value follows from its name and its tag, and the text between tags from the elements around it.

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_base/cm/markup/tests.rs"]
mod tests;

use crate::entropy::hash2;

/// Where the next byte falls.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Place {
    Text,
    TagName,
    Tag,
    AttrName,
    AttrValue,
    Comment,
}

/// The open elements a tracker remembers, innermost kept when deeper.
const DEPTH: usize = 32;
/// The most bytes a tag or comment may run before the tracker takes its `<` for plain text.
const LONGEST_TAG: u32 = 1024;

pub struct Markup {
    place: Place,
    /// The tag's name so far (lowercase, hashed); 0 before its first byte.
    name: u64,
    closing: bool,
    empty: bool,
    /// The attribute's name so far, and how many attributes this tag has had.
    attr: u64,
    attrs: u32,
    /// The quote an attribute value opened with (0 for an unquoted one), and whether the value has just begun.
    quote: u8,
    fresh: bool,
    open: Vec<u64>,
    closed: u64,
    /// Bytes since the last `<` or `>`.
    run: u32,
}

impl Default for Markup {
    fn default() -> Self {
        Self::new()
    }
}

fn is_name_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b':' | b'.')
}

impl Markup {
    pub fn new() -> Self {
        Self { place: Place::Text, name: 0, closing: false, empty: false, attr: 0, attrs: 0, quote: 0, fresh: false, open: Vec::new(), closed: 0, run: 0 }
    }

    pub fn place(&self) -> Place {
        self.place
    }

    /// The innermost open element (0 when none).
    pub fn top(&self) -> u64 {
        self.open.last().copied().unwrap_or(0)
    }

    pub fn depth(&self) -> usize {
        self.open.len()
    }

    fn end_tag(&mut self) {
        if self.closing {
            if let Some(at) = self.open.iter().rposition(|&e| e == self.name) {
                self.open.truncate(at);
            }
            self.closed = self.name;
        } else if !self.empty && self.name != 0 {
            if self.open.len() == DEPTH {
                self.open.remove(0);
            }
            self.open.push(self.name);
        }
        self.place = Place::Text;
        self.run = 0;
    }

    pub fn update(&mut self, b: u8) {
        self.run = self.run.saturating_add(1);
        if self.place != Place::Text && self.run > LONGEST_TAG {
            // A `<` that never closed: whatever this is, it is not a tag.
            self.place = Place::Text;
            self.run = 0;
        }
        match self.place {
            Place::Text => {
                if b == b'<' {
                    self.place = Place::TagName;
                    self.name = 0;
                    self.closing = false;
                    self.empty = false;
                    self.attr = 0;
                    self.attrs = 0;
                    self.run = 0;
                }
            }
            Place::TagName => {
                if self.name == 0 && !self.closing && !(b.is_ascii_alphabetic() || matches!(b, b'/' | b'!' | b'?')) {
                    // `<` before a space, a digit or punctuation is a comparison, not a tag.
                    self.place = Place::Text;
                } else if is_name_byte(b) {
                    self.name = hash2(self.name, u64::from(b.to_ascii_lowercase()));
                } else if self.name == 0 && b == b'/' && !self.closing {
                    self.closing = true;
                } else if self.name == 0 && (b == b'!' || b == b'?') {
                    self.place = Place::Comment;
                } else if b == b'>' {
                    self.end_tag();
                } else if b == b'/' {
                    self.empty = true;
                    self.place = Place::Tag;
                } else if b == b'<' {
                    self.name = 0;
                    self.closing = false;
                } else {
                    self.place = Place::Tag;
                }
            }
            Place::Tag => {
                if b == b'>' {
                    self.end_tag();
                } else if b == b'/' {
                    self.empty = true;
                } else if is_name_byte(b) {
                    self.place = Place::AttrName;
                    self.attr = hash2(0, u64::from(b.to_ascii_lowercase()));
                    self.empty = false;
                }
            }
            Place::AttrName => {
                if is_name_byte(b) {
                    self.attr = hash2(self.attr, u64::from(b.to_ascii_lowercase()));
                } else if b == b'=' {
                    self.place = Place::AttrValue;
                    self.quote = 0;
                    self.fresh = true;
                } else if b == b'>' {
                    self.attrs += 1;
                    self.end_tag();
                } else {
                    self.attrs += 1;
                    self.place = Place::Tag;
                }
            }
            Place::AttrValue => {
                let fresh = std::mem::take(&mut self.fresh);
                if fresh && (b == b'"' || b == b'\'') {
                    self.quote = b;
                } else if self.quote != 0 && b == self.quote {
                    self.quote = 0;
                    self.attrs += 1;
                    self.place = Place::Tag;
                } else if self.quote == 0 && (b == b'>' || b.is_ascii_whitespace()) {
                    self.attrs += 1;
                    if b == b'>' {
                        self.end_tag();
                    } else {
                        self.place = Place::Tag;
                    }
                }
            }
            Place::Comment => {
                if b == b'>' {
                    self.place = Place::Text;
                    self.run = 0;
                }
            }
        }
    }

    /// The five contexts' keys: the tag's name so far, whether it closes, and the innermost open element (a closing
    /// tag spells that element's name); the innermost element and the one last closed; the tag with its attribute's
    /// name and how many came before; the two innermost elements; the depth with the bytes since the last bracket.
    pub fn contexts(&self) -> [u64; 5] {
        let place = self.place as u64 + 1;
        let second = self.open.len().checked_sub(2).map_or(0, |i| self.open[i]);
        let quoted = u64::from(self.quote != 0);
        [
            hash2(hash2(place << 1 | u64::from(self.closing), self.name), self.top()),
            hash2(hash2(place, self.top()), self.closed),
            hash2(hash2(place << 1 | quoted, self.name), hash2(self.attr, u64::from(self.attrs.min(7)))),
            hash2(hash2(place, self.top()), second),
            hash2(place, (self.open.len() as u64) << 16 | u64::from(self.run.min(255))),
        ]
    }
}
