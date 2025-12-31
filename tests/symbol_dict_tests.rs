/// Tests for Symbol Dictionary extraction
///
/// The Symbol Dictionary auto-generates from FOL output,
/// showing users what each symbol means (quantifiers, connectives, predicates, etc.)

use logos::symbol_dict::{extract_symbols, SymbolEntry, SymbolKind};

// ═══════════════════════════════════════════════════════════════════
// Quantifier Extraction Tests
// ═══════════════════════════════════════════════════════════════════

#[test]
fn test_extract_universal_quantifier() {
    let symbols = extract_symbols("∀x(D(x) → B(x))");

    let quantifiers: Vec<_> = symbols.iter()
        .filter(|s| s.kind == SymbolKind::Quantifier)
        .collect();

    assert!(quantifiers.iter().any(|s| s.symbol == "∀"), "Should find universal quantifier ∀");
}

#[test]
fn test_extract_existential_quantifier() {
    let symbols = extract_symbols("∃x(C(x) ∧ B(x))");

    let quantifiers: Vec<_> = symbols.iter()
        .filter(|s| s.kind == SymbolKind::Quantifier)
        .collect();

    assert!(quantifiers.iter().any(|s| s.symbol == "∃"), "Should find existential quantifier ∃");
}

#[test]
fn test_quantifier_has_description() {
    let symbols = extract_symbols("∀x(P(x))");

    let forall = symbols.iter().find(|s| s.symbol == "∀").unwrap();
    assert!(!forall.description.is_empty(), "Quantifier should have description");
    assert!(forall.description.to_lowercase().contains("all") ||
            forall.description.to_lowercase().contains("every") ||
            forall.description.to_lowercase().contains("universal"),
            "Universal quantifier description should explain 'for all': got '{}'", forall.description);
}

// ═══════════════════════════════════════════════════════════════════
// Connective Extraction Tests
// ═══════════════════════════════════════════════════════════════════

#[test]
fn test_extract_conjunction() {
    let symbols = extract_symbols("A ∧ B");

    let connectives: Vec<_> = symbols.iter()
        .filter(|s| s.kind == SymbolKind::Connective)
        .collect();

    assert!(connectives.iter().any(|s| s.symbol == "∧"), "Should find conjunction ∧");
}

#[test]
fn test_extract_disjunction() {
    let symbols = extract_symbols("A ∨ B");

    let connectives: Vec<_> = symbols.iter()
        .filter(|s| s.kind == SymbolKind::Connective)
        .collect();

    assert!(connectives.iter().any(|s| s.symbol == "∨"), "Should find disjunction ∨");
}

#[test]
fn test_extract_implication() {
    let symbols = extract_symbols("A → B");

    let connectives: Vec<_> = symbols.iter()
        .filter(|s| s.kind == SymbolKind::Connective)
        .collect();

    assert!(connectives.iter().any(|s| s.symbol == "→"), "Should find implication →");
}

#[test]
fn test_extract_biconditional() {
    let symbols = extract_symbols("A ↔ B");

    let connectives: Vec<_> = symbols.iter()
        .filter(|s| s.kind == SymbolKind::Connective)
        .collect();

    assert!(connectives.iter().any(|s| s.symbol == "↔"), "Should find biconditional ↔");
}

#[test]
fn test_extract_negation() {
    let symbols = extract_symbols("¬P(x)");

    let connectives: Vec<_> = symbols.iter()
        .filter(|s| s.kind == SymbolKind::Connective)
        .collect();

    assert!(connectives.iter().any(|s| s.symbol == "¬"), "Should find negation ¬");
}

// ═══════════════════════════════════════════════════════════════════
// Predicate Extraction Tests
// ═══════════════════════════════════════════════════════════════════

#[test]
fn test_extract_predicate_names() {
    let symbols = extract_symbols("Dog(x) ∧ Bark(x)");

    let predicates: Vec<_> = symbols.iter()
        .filter(|s| s.kind == SymbolKind::Predicate)
        .collect();

    assert!(predicates.iter().any(|s| s.symbol == "Dog"), "Should find Dog predicate");
    assert!(predicates.iter().any(|s| s.symbol == "Bark"), "Should find Bark predicate");
}

#[test]
fn test_extract_single_letter_predicate() {
    let symbols = extract_symbols("D(x) ∧ B(x)");

    let predicates: Vec<_> = symbols.iter()
        .filter(|s| s.kind == SymbolKind::Predicate)
        .collect();

    assert!(predicates.iter().any(|s| s.symbol == "D"), "Should find D predicate");
    assert!(predicates.iter().any(|s| s.symbol == "B"), "Should find B predicate");
}

// ═══════════════════════════════════════════════════════════════════
// Variable Extraction Tests
// ═══════════════════════════════════════════════════════════════════

#[test]
fn test_extract_variable_names() {
    let symbols = extract_symbols("∀x∃y(L(x, y))");

    let variables: Vec<_> = symbols.iter()
        .filter(|s| s.kind == SymbolKind::Variable)
        .collect();

    assert!(variables.iter().any(|s| s.symbol == "x"), "Should find variable x");
    assert!(variables.iter().any(|s| s.symbol == "y"), "Should find variable y");
}

// ═══════════════════════════════════════════════════════════════════
// Constant Extraction Tests
// ═══════════════════════════════════════════════════════════════════

#[test]
fn test_extract_constants() {
    let symbols = extract_symbols("Love(J, M)");

    let constants: Vec<_> = symbols.iter()
        .filter(|s| s.kind == SymbolKind::Constant)
        .collect();

    assert!(constants.iter().any(|s| s.symbol == "J"), "Should find constant J");
    assert!(constants.iter().any(|s| s.symbol == "M"), "Should find constant M");
}

// ═══════════════════════════════════════════════════════════════════
// Modal Operator Extraction Tests
// ═══════════════════════════════════════════════════════════════════

#[test]
fn test_extract_necessity_operator() {
    let symbols = extract_symbols("□P(x)");

    let modals: Vec<_> = symbols.iter()
        .filter(|s| s.kind == SymbolKind::Modal)
        .collect();

    assert!(modals.iter().any(|s| s.symbol == "□"), "Should find necessity operator □");
}

#[test]
fn test_extract_possibility_operator() {
    let symbols = extract_symbols("◇P(x)");

    let modals: Vec<_> = symbols.iter()
        .filter(|s| s.kind == SymbolKind::Modal)
        .collect();

    assert!(modals.iter().any(|s| s.symbol == "◇"), "Should find possibility operator ◇");
}

// ═══════════════════════════════════════════════════════════════════
// Deduplication Tests
// ═══════════════════════════════════════════════════════════════════

#[test]
fn test_no_duplicate_symbols() {
    let symbols = extract_symbols("∀x(D(x) ∧ D(x) ∧ D(x))");

    let d_count = symbols.iter().filter(|s| s.symbol == "D").count();
    assert_eq!(d_count, 1, "Should have exactly 1 D symbol (no duplicates)");
}

#[test]
fn test_no_duplicate_quantifiers() {
    let symbols = extract_symbols("∀x(P(x)) ∧ ∀y(Q(y))");

    let forall_count = symbols.iter().filter(|s| s.symbol == "∀").count();
    assert_eq!(forall_count, 1, "Should have exactly 1 ∀ symbol (no duplicates)");
}

// ═══════════════════════════════════════════════════════════════════
// Edge Cases
// ═══════════════════════════════════════════════════════════════════

#[test]
fn test_empty_string_returns_empty() {
    let symbols = extract_symbols("");
    assert!(symbols.is_empty(), "Empty string should return no symbols");
}

#[test]
fn test_extract_all_symbol_kinds() {
    // Complex formula with all symbol types
    let symbols = extract_symbols("∀x(Dog(x) → ◇∃y(Cat(y) ∧ Chase(x, y)))");

    assert!(symbols.iter().any(|s| s.kind == SymbolKind::Quantifier), "Should have quantifiers");
    assert!(symbols.iter().any(|s| s.kind == SymbolKind::Connective), "Should have connectives");
    assert!(symbols.iter().any(|s| s.kind == SymbolKind::Predicate), "Should have predicates");
    assert!(symbols.iter().any(|s| s.kind == SymbolKind::Variable), "Should have variables");
    assert!(symbols.iter().any(|s| s.kind == SymbolKind::Modal), "Should have modal operators");
}

// ═══════════════════════════════════════════════════════════════════
// Symbol Entry Tests
// ═══════════════════════════════════════════════════════════════════

#[test]
fn test_symbol_entry_has_all_fields() {
    let symbols = extract_symbols("∀x(P(x))");

    for symbol in &symbols {
        assert!(!symbol.symbol.is_empty(), "Symbol string should not be empty");
        assert!(!symbol.description.is_empty(), "Description should not be empty");
    }
}

#[test]
fn test_connective_descriptions() {
    let symbols = extract_symbols("A ∧ B ∨ C → D");

    let and_sym = symbols.iter().find(|s| s.symbol == "∧").unwrap();
    assert!(and_sym.description.to_lowercase().contains("and") ||
            and_sym.description.to_lowercase().contains("conjunc"),
            "Conjunction description: {}", and_sym.description);

    let or_sym = symbols.iter().find(|s| s.symbol == "∨").unwrap();
    assert!(or_sym.description.to_lowercase().contains("or") ||
            or_sym.description.to_lowercase().contains("disjunc"),
            "Disjunction description: {}", or_sym.description);
}
