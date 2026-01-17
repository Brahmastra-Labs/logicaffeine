//! Symbol Dictionary Extraction
//!
//! Extracts logical symbols from FOL strings for display in a symbol dictionary.
//! Groups symbols by kind and provides descriptions.

use std::collections::HashSet;

/// Categories of logical symbols
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SymbolKind {
    Quantifier,
    Connective,
    Variable,
    Predicate,
    Constant,
    Modal,
    Identity,
    Punctuation,
    Temporal,
}

impl SymbolKind {
    pub fn label(&self) -> &'static str {
        match self {
            SymbolKind::Quantifier => "Quantifier",
            SymbolKind::Connective => "Connective",
            SymbolKind::Variable => "Variable",
            SymbolKind::Predicate => "Predicate",
            SymbolKind::Constant => "Constant",
            SymbolKind::Modal => "Modal",
            SymbolKind::Identity => "Identity",
            SymbolKind::Punctuation => "Punctuation",
            SymbolKind::Temporal => "Temporal",
        }
    }
}

/// A single symbol entry in the dictionary
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SymbolEntry {
    pub symbol: String,
    pub kind: SymbolKind,
    pub description: String,
}

/// Extract symbols from a FOL logic string
pub fn extract_symbols(logic: &str) -> Vec<SymbolEntry> {
    let mut entries = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    // Quantifiers
    if logic.contains("∀") && seen.insert("∀".to_string()) {
        entries.push(SymbolEntry {
            symbol: "∀".to_string(),
            kind: SymbolKind::Quantifier,
            description: "Universal quantifier: \"for all\"".to_string(),
        });
    }
    if logic.contains("∃") && seen.insert("∃".to_string()) {
        entries.push(SymbolEntry {
            symbol: "∃".to_string(),
            kind: SymbolKind::Quantifier,
            description: "Existential quantifier: \"there exists\"".to_string(),
        });
    }
    if logic.contains("∃!") && seen.insert("∃!".to_string()) {
        entries.push(SymbolEntry {
            symbol: "∃!".to_string(),
            kind: SymbolKind::Quantifier,
            description: "Unique existence: \"there exists exactly one\"".to_string(),
        });
    }
    if logic.contains("MOST") && seen.insert("MOST".to_string()) {
        entries.push(SymbolEntry {
            symbol: "MOST".to_string(),
            kind: SymbolKind::Quantifier,
            description: "Generalized quantifier: \"most\"".to_string(),
        });
    }
    if logic.contains("FEW") && seen.insert("FEW".to_string()) {
        entries.push(SymbolEntry {
            symbol: "FEW".to_string(),
            kind: SymbolKind::Quantifier,
            description: "Generalized quantifier: \"few\"".to_string(),
        });
    }

    // Connectives
    if logic.contains("∧") && seen.insert("∧".to_string()) {
        entries.push(SymbolEntry {
            symbol: "∧".to_string(),
            kind: SymbolKind::Connective,
            description: "Conjunction: \"and\"".to_string(),
        });
    }
    if logic.contains("∨") && seen.insert("∨".to_string()) {
        entries.push(SymbolEntry {
            symbol: "∨".to_string(),
            kind: SymbolKind::Connective,
            description: "Disjunction: \"or\"".to_string(),
        });
    }
    if logic.contains("→") && seen.insert("→".to_string()) {
        entries.push(SymbolEntry {
            symbol: "→".to_string(),
            kind: SymbolKind::Connective,
            description: "Implication: \"if...then\"".to_string(),
        });
    }
    if logic.contains("↔") && seen.insert("↔".to_string()) {
        entries.push(SymbolEntry {
            symbol: "↔".to_string(),
            kind: SymbolKind::Connective,
            description: "Biconditional: \"if and only if\"".to_string(),
        });
    }
    if logic.contains("¬") && seen.insert("¬".to_string()) {
        entries.push(SymbolEntry {
            symbol: "¬".to_string(),
            kind: SymbolKind::Connective,
            description: "Negation: \"not\"".to_string(),
        });
    }

    // Modal operators
    if logic.contains("□") && seen.insert("□".to_string()) {
        entries.push(SymbolEntry {
            symbol: "□".to_string(),
            kind: SymbolKind::Modal,
            description: "Necessity: \"it is necessary that\"".to_string(),
        });
    }
    if logic.contains("◇") && seen.insert("◇".to_string()) {
        entries.push(SymbolEntry {
            symbol: "◇".to_string(),
            kind: SymbolKind::Modal,
            description: "Possibility: \"it is possible that\"".to_string(),
        });
    }
    if logic.contains("O_") && seen.insert("O".to_string()) {
        entries.push(SymbolEntry {
            symbol: "O".to_string(),
            kind: SymbolKind::Modal,
            description: "Deontic obligation: \"it ought to be that\"".to_string(),
        });
    }

    // Identity
    if logic.contains(" = ") && seen.insert("=".to_string()) {
        entries.push(SymbolEntry {
            symbol: "=".to_string(),
            kind: SymbolKind::Identity,
            description: "Identity: \"is identical to\"".to_string(),
        });
    }

    // Extract predicates (uppercase letters followed by parenthesis)
    extract_predicates(logic, &mut entries, &mut seen);

    // Extract variables (lowercase x, y, z, etc.)
    extract_variables(logic, &mut entries, &mut seen);

    // Extract constants (uppercase single letters not followed by parenthesis)
    extract_constants(logic, &mut entries, &mut seen);

    entries
}

fn extract_predicates(logic: &str, entries: &mut Vec<SymbolEntry>, seen: &mut HashSet<String>) {
    // Match patterns like "Dog(", "Mortal(", "Loves("
    let chars: Vec<char> = logic.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        if chars[i].is_ascii_uppercase() {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            if i < chars.len() && chars[i] == '(' {
                let predicate: String = chars[start..i].iter().collect();
                if seen.insert(format!("pred_{}", predicate)) {
                    entries.push(SymbolEntry {
                        symbol: predicate.clone(),
                        kind: SymbolKind::Predicate,
                        description: format!("Predicate: {}", predicate),
                    });
                }
            }
        }
        i += 1;
    }
}

fn extract_variables(logic: &str, entries: &mut Vec<SymbolEntry>, seen: &mut HashSet<String>) {
    // Variables are lowercase letters typically x, y, z, w
    for var in ['x', 'y', 'z', 'w', 'e'] {
        let var_str = var.to_string();
        // Check if variable appears in context (not as part of a word)
        if logic.contains(&format!("({})", var))
            || logic.contains(&format!("({},", var))
            || logic.contains(&format!(", {})", var))
            || logic.contains(&format!("{}.", var))
            || logic.contains(&format!(" {}", var))
        {
            if seen.insert(format!("var_{}", var)) {
                entries.push(SymbolEntry {
                    symbol: var_str,
                    kind: SymbolKind::Variable,
                    description: "Bound variable".to_string(),
                });
            }
        }
    }
}

fn extract_constants(logic: &str, entries: &mut Vec<SymbolEntry>, seen: &mut HashSet<String>) {
    // Constants are uppercase letters like J (John), M (Mary), etc.
    // But not predicates (followed by parenthesis)
    let chars: Vec<char> = logic.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        if chars[i].is_ascii_uppercase() {
            let start = i;
            // Collect the full name (may have numbers like J2)
            while i < chars.len() && (chars[i].is_ascii_alphanumeric()) {
                i += 1;
            }
            // Check if NOT followed by parenthesis (would be predicate)
            if i >= chars.len() || chars[i] != '(' {
                let constant: String = chars[start..i].iter().collect();
                // Skip very long names (likely predicates) and known quantifiers
                if constant.len() <= 3
                    && !["MOST", "FEW", "ALL", "THE"].contains(&constant.as_str())
                    && seen.insert(format!("const_{}", constant))
                {
                    entries.push(SymbolEntry {
                        symbol: constant.clone(),
                        kind: SymbolKind::Constant,
                        description: format!("Constant: {}", constant),
                    });
                }
            }
        }
        i += 1;
    }
}

/// Get symbols grouped by kind for display
pub fn group_symbols_by_kind(entries: &[SymbolEntry]) -> Vec<(SymbolKind, Vec<&SymbolEntry>)> {
    let kinds = [
        SymbolKind::Quantifier,
        SymbolKind::Connective,
        SymbolKind::Modal,
        SymbolKind::Identity,
        SymbolKind::Predicate,
        SymbolKind::Variable,
        SymbolKind::Constant,
    ];

    kinds
        .iter()
        .filter_map(|&kind| {
            let matching: Vec<_> = entries.iter().filter(|e| e.kind == kind).collect();
            if matching.is_empty() {
                None
            } else {
                Some((kind, matching))
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_quantifier_symbols() {
        let logic = "∀x(Dog(x) → Mortal(x))";
        let symbols = extract_symbols(logic);

        let quantifiers: Vec<_> = symbols.iter().filter(|s| s.kind == SymbolKind::Quantifier).collect();
        assert!(quantifiers.iter().any(|s| s.symbol == "∀"), "Should find universal quantifier");
    }

    #[test]
    fn test_extract_existential() {
        let logic = "∃x(Cat(x) ∧ Black(x))";
        let symbols = extract_symbols(logic);

        assert!(symbols.iter().any(|s| s.symbol == "∃"), "Should find existential quantifier");
    }

    #[test]
    fn test_extract_connective_symbols() {
        let logic = "∀x(Dog(x) → (Loyal(x) ∧ Friendly(x)))";
        let symbols = extract_symbols(logic);

        let connectives: Vec<_> = symbols.iter().filter(|s| s.kind == SymbolKind::Connective).collect();
        assert!(connectives.iter().any(|s| s.symbol == "∧"), "Should find conjunction");
        assert!(connectives.iter().any(|s| s.symbol == "→"), "Should find implication");
    }

    #[test]
    fn test_extract_predicate_names() {
        let logic = "∀x(Dog(x) → Mammal(x))";
        let symbols = extract_symbols(logic);

        let predicates: Vec<_> = symbols.iter().filter(|s| s.kind == SymbolKind::Predicate).collect();
        assert!(predicates.iter().any(|s| s.symbol == "Dog"), "Should find Dog predicate");
        assert!(predicates.iter().any(|s| s.symbol == "Mammal"), "Should find Mammal predicate");
    }

    #[test]
    fn test_extract_variable_names() {
        let logic = "∀x∃y(Loves(x, y))";
        let symbols = extract_symbols(logic);

        let variables: Vec<_> = symbols.iter().filter(|s| s.kind == SymbolKind::Variable).collect();
        assert!(variables.iter().any(|s| s.symbol == "x"), "Should find variable x");
        assert!(variables.iter().any(|s| s.symbol == "y"), "Should find variable y");
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
            assert!(!symbol.description.is_empty(), "Every symbol should have a description");
        }
    }

    #[test]
    fn test_modal_symbols() {
        let logic = "□(P(x)) ∧ ◇(Q(y))";
        let symbols = extract_symbols(logic);

        assert!(symbols.iter().any(|s| s.symbol == "□"), "Should find necessity operator");
        assert!(symbols.iter().any(|s| s.symbol == "◇"), "Should find possibility operator");
    }

    #[test]
    fn test_group_symbols_by_kind() {
        let logic = "∀x(Dog(x) → ∃y(Loves(x, y)))";
        let symbols = extract_symbols(logic);
        let grouped = group_symbols_by_kind(&symbols);

        // Should have multiple groups
        assert!(!grouped.is_empty(), "Should have grouped symbols");

        // Check quantifiers group exists
        assert!(grouped.iter().any(|(k, _)| *k == SymbolKind::Quantifier), "Should have quantifier group");
    }
}
