//! Composing a formula by hand: the symbol buttons beside a typed answer, and where a click puts
//! its symbol.
//!
//! The pad holds every symbol a typed answer on /learn needs and nothing the grader cannot read
//! (`learn_symbol_pad.rs` holds both directions). A logical symbol means what the Symbol Dictionary
//! says it means, read from one table in the language crate.

use logicaffeine_language::symbol_dict::notation;
use std::sync::LazyLock;

/// One button of the symbol pad.
#[derive(Debug, Clone, PartialEq)]
pub struct SymbolButton {
    /// What a click types.
    pub symbol: &'static str,
    /// The short name under the symbol.
    pub name: &'static str,
    /// What the symbol means in a formula.
    pub meaning: String,
}

impl SymbolButton {
    /// The button's accessible name.
    pub fn label(&self) -> String {
        format!("Insert {}, {}", self.symbol, self.name)
    }
}

/// The pad, in the order it shows its buttons: each symbol with its short name, and the meaning of a
/// symbol the dictionary does not cover (the grouping marks).
const PAD: &[(&str, &str, Option<&str>)] = &[
    ("∀", "for every", None),
    ("∃", "there is", None),
    ("¬", "not", None),
    ("∧", "and", None),
    ("∨", "or", None),
    ("→", "if then", None),
    ("↔", "if and only if", None),
    ("□", "necessarily", None),
    ("◇", "possibly", None),
    ("=", "is the same as", None),
    ("(", "open", Some("starts a group, or a list of arguments")),
    (")", "close", Some("ends a group, or a list of arguments")),
    (",", "comma", Some("separates the arguments of a predicate")),
];

static BUTTONS: LazyLock<Vec<SymbolButton>> = LazyLock::new(|| {
    let dictionary = notation();
    PAD.iter()
        .map(|&(symbol, name, own)| {
            let meaning = match own {
                Some(meaning) => meaning.to_string(),
                None => dictionary
                    .iter()
                    .find(|entry| entry.symbol == symbol)
                    .map(|entry| entry.description.clone())
                    .unwrap_or_else(|| panic!("the symbol dictionary has no meaning for {symbol}")),
            };
            SymbolButton { symbol, name, meaning }
        })
        .collect()
});

/// Every button of the pad, in the order it shows them.
pub fn symbol_buttons() -> &'static [SymbolButton] {
    &BUTTONS
}

/// The byte offset in `text` at `units` UTF-16 code units from its start, rounded forward to a
/// character boundary, and at most the end of the text.
fn byte_at(text: &str, units: usize) -> usize {
    let mut counted = 0;
    for (at, c) in text.char_indices() {
        if counted >= units {
            return at;
        }
        counted += c.len_utf16();
    }
    text.len()
}

/// `text` with `insert` put in place of the selection from `start` to `end`, and where the caret
/// lands after it. Positions count UTF-16 code units, as the browser counts them; a backwards
/// selection is the same selection, and a position past the end is the end.
pub fn insert_at(text: &str, start: usize, end: usize, insert: &str) -> (String, usize) {
    let (from, to) = (byte_at(text, start.min(end)), byte_at(text, start.max(end)));
    let mut out = String::with_capacity(text.len() + insert.len());
    out.push_str(&text[..from]);
    out.push_str(insert);
    let caret = out.encode_utf16().count();
    out.push_str(&text[to..]);
    (out, caret)
}

#[cfg(test)]
#[path = "../../../crates/logicaffeine_tests/unit/logicaffeine_web/composer/tests.rs"]
mod tests;
