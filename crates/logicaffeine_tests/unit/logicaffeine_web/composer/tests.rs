use super::*;
use logicaffeine_language::symbol_dict::notation;
use std::collections::HashSet;

fn symbols() -> Vec<&'static str> {
    symbol_buttons().iter().map(|b| b.symbol).collect()
}

// --- The buttons ---------------------------------------------------------------------

#[test]
fn the_pad_offers_the_quantifiers_connectives_modals_identity_and_grouping() {
    assert_eq!(symbols(), ["∀", "∃", "¬", "∧", "∨", "→", "↔", "□", "◇", "=", "(", ")", ","]);
}

#[test]
fn every_button_types_a_different_symbol() {
    let unique: HashSet<_> = symbols().into_iter().collect();
    assert_eq!(unique.len(), symbol_buttons().len());
}

#[test]
fn every_button_has_a_name_and_a_meaning() {
    for b in symbol_buttons() {
        assert!(!b.name.trim().is_empty(), "{} has no name", b.symbol);
        assert!(!b.meaning.trim().is_empty(), "{} has no meaning", b.symbol);
    }
}

#[test]
fn a_logical_symbol_means_what_the_symbol_dictionary_says() {
    // One source for what a symbol means: the dictionary the Studio shows beside a formula.
    let dictionary = notation();
    let mut checked = 0;
    for b in symbol_buttons() {
        if let Some(entry) = dictionary.iter().find(|e| e.symbol == b.symbol) {
            assert_eq!(b.meaning, entry.description, "{} means {:?} on the pad but {:?} in the dictionary", b.symbol, b.meaning, entry.description);
            checked += 1;
        }
    }
    assert_eq!(checked, 10, "every quantifier, connective, modal and identity button is in the dictionary");
}

#[test]
fn the_notation_table_lists_the_fixed_symbols_with_their_meanings() {
    let table = notation();
    let find = |s: &str| table.iter().find(|e| e.symbol == s).map(|e| e.description.clone());
    assert_eq!(find("∀").as_deref(), Some("for every …"));
    assert_eq!(find("∃").as_deref(), Some("there is at least one …"));
    assert_eq!(find("¬").as_deref(), Some("it is not the case that …"));
    assert_eq!(find("→").as_deref(), Some("if … then …"));
    assert_eq!(find("=").as_deref(), Some("… is the same individual as …"));
    assert!(table.iter().all(|e| e.usage == e.symbol), "a fixed symbol is written as itself");
}

#[test]
fn no_button_types_a_bang() {
    // The grader reads `!` as ¬, so a ∃! button would turn "exactly one" into "not".
    assert!(symbol_buttons().iter().all(|b| !b.symbol.contains('!')));
}

#[test]
fn a_button_is_named_by_its_symbol_and_its_name() {
    let every = &symbol_buttons()[0];
    assert_eq!(every.label(), format!("Insert ∀, {}", every.name));
    for b in symbol_buttons() {
        assert!(b.label().starts_with(&format!("Insert {}", b.symbol)), "{}", b.label());
    }
}

// --- Where a click puts its symbol ---------------------------------------------------

#[test]
fn a_symbol_goes_in_at_the_caret() {
    assert_eq!(insert_at("Dog(x)", 0, 0, "∀x"), ("∀xDog(x)".to_string(), 2));
    assert_eq!(insert_at("P  Q", 2, 2, "∧"), ("P ∧ Q".to_string(), 3));
}

#[test]
fn a_symbol_goes_in_at_the_end() {
    assert_eq!(insert_at("Happy(John) ", 12, 12, "→"), ("Happy(John) →".to_string(), 13));
}

#[test]
fn a_symbol_replaces_the_selection() {
    assert_eq!(insert_at("P and Q", 2, 5, "∧"), ("P ∧ Q".to_string(), 3));
}

#[test]
fn a_backwards_selection_is_the_same_selection() {
    assert_eq!(insert_at("P and Q", 5, 2, "∧"), ("P ∧ Q".to_string(), 3));
}

#[test]
fn a_caret_past_the_end_lands_at_the_end() {
    assert_eq!(insert_at("Q", 40, 90, "¬"), ("Q¬".to_string(), 2));
}

#[test]
fn positions_count_utf16_units_as_the_browser_does() {
    // ∀ is one UTF-16 unit; an emoji outside the basic plane is two.
    assert_eq!(insert_at("∀x", 1, 1, "∃"), ("∀∃x".to_string(), 2));
    assert_eq!(insert_at("😀x", 2, 2, "¬"), ("😀¬x".to_string(), 3));
    assert_eq!(insert_at("😀x", 1, 1, "¬"), ("😀¬x".to_string(), 3), "a caret inside a surrogate pair moves past it");
}

#[test]
fn inserting_into_an_empty_answer() {
    assert_eq!(insert_at("", 0, 0, "("), ("(".to_string(), 1));
}
