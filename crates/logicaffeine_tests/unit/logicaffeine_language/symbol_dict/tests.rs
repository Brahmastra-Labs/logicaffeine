use super::*;

#[test]
fn test_extract_quantifier_symbols() {
    let logic = "∀x(Dog(x) → Mortal(x))";
    let symbols = extract_symbols(logic);

    let quantifiers: Vec<_> = symbols
        .iter()
        .filter(|s| s.kind == SymbolKind::Quantifier)
        .collect();
    assert!(
        quantifiers.iter().any(|s| s.symbol == "∀"),
        "Should find universal quantifier"
    );
}

#[test]
fn test_extract_existential() {
    let logic = "∃x(Cat(x) ∧ Black(x))";
    let symbols = extract_symbols(logic);

    assert!(
        symbols.iter().any(|s| s.symbol == "∃"),
        "Should find existential quantifier"
    );
}

#[test]
fn test_extract_connective_symbols() {
    let logic = "∀x(Dog(x) → (Loyal(x) ∧ Friendly(x)))";
    let symbols = extract_symbols(logic);

    let connectives: Vec<_> = symbols
        .iter()
        .filter(|s| s.kind == SymbolKind::Connective)
        .collect();
    assert!(
        connectives.iter().any(|s| s.symbol == "∧"),
        "Should find conjunction"
    );
    assert!(
        connectives.iter().any(|s| s.symbol == "→"),
        "Should find implication"
    );
}

#[test]
fn test_extract_predicate_names() {
    let logic = "∀x(Dog(x) → Mammal(x))";
    let symbols = extract_symbols(logic);

    let predicates: Vec<_> = symbols
        .iter()
        .filter(|s| s.kind == SymbolKind::Predicate)
        .collect();
    assert!(
        predicates.iter().any(|s| s.symbol == "Dog"),
        "Should find Dog predicate"
    );
    assert!(
        predicates.iter().any(|s| s.symbol == "Mammal"),
        "Should find Mammal predicate"
    );
}

#[test]
fn test_extract_variable_names() {
    let logic = "∀x∃y(Loves(x, y))";
    let symbols = extract_symbols(logic);

    let variables: Vec<_> = symbols
        .iter()
        .filter(|s| s.kind == SymbolKind::Variable)
        .collect();
    assert!(
        variables.iter().any(|s| s.symbol == "x"),
        "Should find variable x"
    );
    assert!(
        variables.iter().any(|s| s.symbol == "y"),
        "Should find variable y"
    );
}

#[test]
fn test_no_duplicate_symbols() {
    let logic = "∀x(Dog(x) → Dog(x))";
    let symbols = extract_symbols(logic);

    let dog_count = symbols.iter().filter(|s| s.symbol == "Dog").count();
    assert_eq!(dog_count, 1, "Should not have duplicate predicates");
}

#[test]
fn test_symbol_has_description() {
    let logic = "∀x(P(x))";
    let symbols = extract_symbols(logic);

    for symbol in &symbols {
        assert!(
            !symbol.description.is_empty(),
            "Every symbol should have a description"
        );
    }
}

#[test]
fn test_modal_symbols() {
    let logic = "□(P(x)) ∧ ◇(Q(y))";
    let symbols = extract_symbols(logic);

    assert!(
        symbols.iter().any(|s| s.symbol == "□"),
        "Should find necessity operator"
    );
    assert!(
        symbols.iter().any(|s| s.symbol == "◇"),
        "Should find possibility operator"
    );
}

#[test]
fn test_group_symbols_by_kind() {
    let logic = "∀x(Dog(x) → ∃y(Loves(x, y)))";
    let symbols = extract_symbols(logic);
    let grouped = group_symbols_by_kind(&symbols);

    // Should have multiple groups
    assert!(!grouped.is_empty(), "Should have grouped symbols");

    // Check quantifiers group exists
    assert!(
        grouped.iter().any(|(k, _)| *k == SymbolKind::Quantifier),
        "Should have quantifier group"
    );
}

fn kind_of(symbols: &[SymbolEntry], symbol: &str) -> Option<SymbolKind> {
    symbols.iter().find(|s| s.symbol == symbol).map(|s| s.kind)
}

/// A tense wraps a formula; it is an operator of the formula, not a predicate.
#[test]
fn a_tense_around_a_formula_is_temporal() {
    let full = extract_symbols("¬P(∃x((Dog(x) ∧ See(John, x))))");
    assert_eq!(kind_of(&full, "P"), Some(SymbolKind::Temporal));
    let entry = full.iter().find(|s| s.symbol == "P").unwrap();
    assert!(
        entry.description.to_lowercase().contains("past"),
        "{}",
        entry.description
    );
    assert_eq!(kind_of(&full, "Dog"), Some(SymbolKind::Predicate));
}

#[test]
fn each_view_names_its_own_tense() {
    let simple = extract_symbols_for(
        "¬Past(∃x((Dog(x) ∧ See(John, x))))",
        &crate::formatter::SimpleFOLFormatter,
    );
    assert_eq!(kind_of(&simple, "Past"), Some(SymbolKind::Temporal));
    let later = extract_symbols_for("Future(Run(John))", &crate::formatter::SimpleFOLFormatter);
    assert_eq!(kind_of(&later, "Future"), Some(SymbolKind::Temporal));
}

#[test]
fn an_aspect_around_a_formula_is_temporal() {
    for (logic, op, word) in [
        ("HAB(Bark(Fido))", "HAB", "habitual"),
        ("Prog(Run(John))", "Prog", "progressive"),
        ("Perf(Eat(John))", "Perf", "perfect"),
        ("ITER(Knock(John))", "ITER", "iterative"),
    ] {
        let found = extract_symbols(logic);
        assert_eq!(kind_of(&found, op), Some(SymbolKind::Temporal), "{logic}");
        let entry = found.iter().find(|s| s.symbol == op).unwrap();
        assert!(
            entry.description.to_lowercase().contains(word),
            "{logic}: {}",
            entry.description
        );
    }
}

/// `P` applied to a term is a predicate named P, as a reader writes one.
#[test]
fn an_operator_name_applied_to_a_term_is_a_predicate() {
    assert_eq!(
        kind_of(&extract_symbols("∀x(P(x))"), "P"),
        Some(SymbolKind::Predicate)
    );
    assert_eq!(
        kind_of(&extract_symbols("P(John)"), "P"),
        Some(SymbolKind::Predicate)
    );
}

#[test]
fn a_thematic_role_is_a_role() {
    let found = extract_symbols("∃e((Run(e) ∧ Agent(e, John)))");
    assert_eq!(kind_of(&found, "Agent"), Some(SymbolKind::Role));
    assert!(
        group_symbols_by_kind(&found)
            .iter()
            .any(|(kind, _)| *kind == SymbolKind::Role),
        "the roles are shown as a group"
    );
    assert_eq!(kind_of(&found, "Run"), Some(SymbolKind::Predicate));
}

/// A noun that happens to share a role's name ("every agent") is a one-place predicate.
#[test]
fn a_one_place_role_name_is_a_predicate() {
    assert_eq!(
        kind_of(&extract_symbols("∀x(Agent(x) → Trained(x))"), "Agent"),
        Some(SymbolKind::Predicate)
    );
}

#[test]
fn a_proper_name_is_a_constant() {
    let found = extract_symbols("See(John, Mary)");
    assert_eq!(kind_of(&found, "John"), Some(SymbolKind::Constant));
    assert_eq!(kind_of(&found, "Mary"), Some(SymbolKind::Constant));
}

#[test]
fn every_kind_found_is_shown_in_a_group() {
    let found = extract_symbols("∀x(¬P(∃e((Run(e) ∧ Agent(e, John)))) ∧ HAB(Bark(x)))");
    let grouped = group_symbols_by_kind(&found);
    for entry in &found {
        assert!(
            grouped.iter().any(|(kind, _)| *kind == entry.kind),
            "{:?} is never shown",
            entry.kind
        );
    }
}

fn entry<'a>(symbols: &'a [SymbolEntry], symbol: &str) -> &'a SymbolEntry {
    symbols
        .iter()
        .find(|s| s.symbol == symbol)
        .unwrap_or_else(|| panic!("no {symbol} in {symbols:?}"))
}

/// "Dog — Predicate: Dog" said the symbol twice and meant nothing.
#[test]
fn a_description_never_just_repeats_its_symbol() {
    for logic in [
        "∀x((Dog(x) ∧ Animal(x)) → Bark(x))",
        "¬P(∃x((Dog(x) ∧ See(John, x))))",
        "∃e((Run(e) ∧ Agent(e, John)))",
    ] {
        for s in extract_symbols(logic) {
            assert!(
                !s.description
                    .contains(&format!("{}: {}", s.kind.label(), s.symbol)),
                "{logic}: {s:?}"
            );
            assert_ne!(s.description.trim(), s.symbol, "{logic}: {s:?}");
        }
    }
}

#[test]
fn usage_shows_the_symbol_as_the_formula_writes_it() {
    let found = extract_symbols("¬P(∃x((Dog(x) ∧ See(John, x))))");
    assert_eq!(entry(&found, "Dog").usage, "Dog(x)");
    assert_eq!(entry(&found, "See").usage, "See(John, x)");
    assert_eq!(entry(&found, "P").usage, "P(…)");
    assert_eq!(entry(&found, "∃").usage, "∃");
}

#[test]
fn a_noun_predicate_reads_as_being_one() {
    let found = extract_symbols("∀x((Dog(x) ∧ Animal(x)) → Mammal(x))");
    assert_eq!(entry(&found, "Dog").description, "x is a dog");
    assert_eq!(entry(&found, "Animal").description, "x is an animal");
}

#[test]
fn a_verb_predicate_reads_as_its_sentence() {
    let found = extract_symbols("∀x((Dog(x) ∧ See(John, x)) → Bark(x))");
    assert_eq!(entry(&found, "See").description, "John sees x");
    assert_eq!(entry(&found, "Bark").description, "x barks");
}

#[test]
fn a_role_reads_as_who_plays_it() {
    let found = extract_symbols("∃e((Run(e) ∧ Agent(e, John)))");
    assert_eq!(entry(&found, "Agent").description, "John is the agent of e");
}

#[test]
fn a_variable_says_what_binds_it() {
    let found = extract_symbols("∀x(∃y(Loves(x, y)))");
    assert!(
        entry(&found, "x").description.contains('∀'),
        "{:?}",
        entry(&found, "x")
    );
    assert!(
        entry(&found, "y").description.contains('∃'),
        "{:?}",
        entry(&found, "y")
    );
    let events = extract_symbols("∃e((Run(e) ∧ Agent(e, John)))");
    assert!(
        entry(&events, "e").description.contains("event"),
        "{:?}",
        entry(&events, "e")
    );
}

#[test]
fn a_constant_is_a_named_individual() {
    let found = extract_symbols("See(John, Mary)");
    assert!(
        entry(&found, "John").description.contains("individual"),
        "{:?}",
        entry(&found, "John")
    );
}

/// The Deep view spells its symbols as words and evaluates in worlds: it is read in its own
/// words, and the world a predicate holds in is not one of its participants.
#[test]
fn the_deep_view_is_read_in_its_own_words() {
    let found = extract_symbols_for(
        "Past(Exists x((Dogs(x, w0) And See(John, x, w0))))",
        &crate::formatter::KripkeFormatter,
    );
    assert_eq!(kind_of(&found, "Past"), Some(SymbolKind::Temporal));
    assert_eq!(kind_of(&found, "Exists"), Some(SymbolKind::Quantifier));
    assert_eq!(kind_of(&found, "And"), Some(SymbolKind::Connective));
    assert_eq!(entry(&found, "Dogs").description, "x is a dog");
    assert_eq!(entry(&found, "See").description, "John sees x");
    assert_eq!(kind_of(&found, "John"), Some(SymbolKind::Constant));
    assert!(
        entry(&found, "x").description.contains("Exists"),
        "{:?}",
        entry(&found, "x")
    );
    let constants: Vec<&str> = found
        .iter()
        .filter(|s| s.kind == SymbolKind::Constant)
        .map(|s| s.symbol.as_str())
        .collect();
    assert_eq!(constants, ["John"], "the view's words are not individuals");
}

/// The Full view says an event's tense of the event itself: `Past(e)` places e in the past.
#[test]
fn a_tense_of_an_event_places_the_event() {
    let found = extract_symbols("∃e(Run(e) ∧ Agent(e, John) ∧ Quickly(e) ∧ Past(e))");
    assert_eq!(kind_of(&found, "Past"), Some(SymbolKind::Temporal));
    assert_eq!(entry(&found, "Past").description, "e lies in the past");
    assert_eq!(entry(&found, "Past").usage, "Past(e)");
}

/// A manner adverb says how the event is done, not what anyone is.
#[test]
fn a_manner_adverb_reads_as_how_it_is_done() {
    let full = extract_symbols("∃e(Run(e) ∧ Agent(e, John) ∧ Quickly(e) ∧ Past(e))");
    assert_eq!(entry(&full, "Quickly").description, "e happens quickly");
    let simple = extract_symbols_for(
        "Past(Run(John) ∧ Quickly(John))",
        &crate::formatter::SimpleFOLFormatter,
    );
    assert_eq!(
        entry(&simple, "Quickly").description,
        "John does it quickly"
    );
}

/// "the bread" prints as the constant Bread: a particular individual, not a named one.
#[test]
fn a_constant_is_a_particular_individual() {
    let found = extract_symbols("Slice(Mary, Bread)");
    assert_eq!(
        entry(&found, "Bread").description,
        "a particular individual"
    );
}
