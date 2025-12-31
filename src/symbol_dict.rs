//! Symbol Dictionary extraction from First-Order Logic output.
//!
//! This module parses FOL strings and extracts symbols with their meanings,
//! allowing the UI to display a "Symbol Dictionary" that helps students
//! understand the notation.

use std::collections::HashSet;

/// The category of a logical symbol
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SymbolKind {
    /// ∀, ∃ - quantifiers
    Quantifier,
    /// ∧, ∨, →, ↔, ¬ - logical connectives
    Connective,
    /// x, y, z - bound variables
    Variable,
    /// Dog, Bark, Loves - predicate symbols
    Predicate,
    /// J, M, S - individual constants (named entities)
    Constant,
    /// □, ◇ - modal necessity/possibility
    Modal,
    /// ○, G, F, H, P - temporal operators (future, past, etc.)
    Temporal,
}

impl SymbolKind {
    /// Get a display label for this kind
    pub fn label(&self) -> &'static str {
        match self {
            SymbolKind::Quantifier => "Quantifier",
            SymbolKind::Connective => "Connective",
            SymbolKind::Variable => "Variable",
            SymbolKind::Predicate => "Predicate",
            SymbolKind::Constant => "Constant",
            SymbolKind::Modal => "Modal",
            SymbolKind::Temporal => "Temporal",
        }
    }
}

/// A single entry in the symbol dictionary
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolEntry {
    /// The symbol itself (e.g., "∀", "Dog", "x")
    pub symbol: String,
    /// The category of the symbol
    pub kind: SymbolKind,
    /// Human-readable description of what it means
    pub description: String,
}

/// Extract all unique symbols from a First-Order Logic string.
///
/// # Arguments
/// * `logic` - A FOL formula string (e.g., "∀x(Dog(x) → Bark(x))")
///
/// # Returns
/// A vector of `SymbolEntry` items, one for each unique symbol found.
/// Duplicates are automatically removed.
pub fn extract_symbols(logic: &str) -> Vec<SymbolEntry> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut symbols: Vec<SymbolEntry> = Vec::new();

    let mut chars = logic.chars().peekable();

    while let Some(c) = chars.next() {
        let entry = match c {
            // ═══════════════════════════════════════════════════════════════
            // Quantifiers
            // ═══════════════════════════════════════════════════════════════
            '∀' | '\u{2200}' => Some(SymbolEntry {
                symbol: "∀".to_string(),
                kind: SymbolKind::Quantifier,
                description: "Universal quantifier: \"for all\"".to_string(),
            }),
            '∃' | '\u{2203}' => Some(SymbolEntry {
                symbol: "∃".to_string(),
                kind: SymbolKind::Quantifier,
                description: "Existential quantifier: \"there exists\"".to_string(),
            }),

            // ═══════════════════════════════════════════════════════════════
            // Connectives
            // ═══════════════════════════════════════════════════════════════
            '¬' | '\u{00AC}' => Some(SymbolEntry {
                symbol: "¬".to_string(),
                kind: SymbolKind::Connective,
                description: "Negation: \"not\"".to_string(),
            }),
            '∧' | '\u{2227}' => Some(SymbolEntry {
                symbol: "∧".to_string(),
                kind: SymbolKind::Connective,
                description: "Conjunction: \"and\"".to_string(),
            }),
            '∨' | '\u{2228}' => Some(SymbolEntry {
                symbol: "∨".to_string(),
                kind: SymbolKind::Connective,
                description: "Disjunction: \"or\"".to_string(),
            }),
            '→' | '\u{2192}' => Some(SymbolEntry {
                symbol: "→".to_string(),
                kind: SymbolKind::Connective,
                description: "Implication: \"if...then\"".to_string(),
            }),
            '↔' | '\u{2194}' => Some(SymbolEntry {
                symbol: "↔".to_string(),
                kind: SymbolKind::Connective,
                description: "Biconditional: \"if and only if\"".to_string(),
            }),
            '⊃' | '\u{2283}' => Some(SymbolEntry {
                symbol: "⊃".to_string(),
                kind: SymbolKind::Connective,
                description: "Material conditional: \"if...then\" (horseshoe)".to_string(),
            }),
            '≡' | '\u{2261}' => Some(SymbolEntry {
                symbol: "≡".to_string(),
                kind: SymbolKind::Connective,
                description: "Material equivalence: \"if and only if\"".to_string(),
            }),

            // ═══════════════════════════════════════════════════════════════
            // Modal Operators
            // ═══════════════════════════════════════════════════════════════
            '□' | '\u{25A1}' | '\u{25FB}' => Some(SymbolEntry {
                symbol: "□".to_string(),
                kind: SymbolKind::Modal,
                description: "Necessity: \"necessarily\" or \"it must be that\"".to_string(),
            }),
            '◇' | '\u{25C7}' | '\u{25CA}' => Some(SymbolEntry {
                symbol: "◇".to_string(),
                kind: SymbolKind::Modal,
                description: "Possibility: \"possibly\" or \"it might be that\"".to_string(),
            }),

            // ═══════════════════════════════════════════════════════════════
            // Variables (lowercase single letters)
            // ═══════════════════════════════════════════════════════════════
            c if c.is_ascii_lowercase() => {
                // Check if standalone variable (not part of a word)
                let is_standalone = chars.peek().map_or(true, |next| {
                    !next.is_alphanumeric()
                });

                if is_standalone {
                    Some(SymbolEntry {
                        symbol: c.to_string(),
                        kind: SymbolKind::Variable,
                        description: format!("Variable: bound by a quantifier"),
                    })
                } else {
                    None
                }
            }

            // ═══════════════════════════════════════════════════════════════
            // Predicates and Constants (uppercase)
            // ═══════════════════════════════════════════════════════════════
            c if c.is_ascii_uppercase() => {
                // Collect the full word
                let mut word = String::from(c);
                while let Some(&next) = chars.peek() {
                    if next.is_alphanumeric() || next == '_' {
                        word.push(chars.next().unwrap());
                    } else {
                        break;
                    }
                }

                // Check if followed by parenthesis (predicate) or not (constant)
                let is_predicate = chars.peek() == Some(&'(');

                if is_predicate {
                    Some(SymbolEntry {
                        symbol: word.clone(),
                        kind: SymbolKind::Predicate,
                        description: format!("Predicate: {}", humanize_predicate(&word)),
                    })
                } else {
                    // Single uppercase letter is typically a constant
                    Some(SymbolEntry {
                        symbol: word.clone(),
                        kind: SymbolKind::Constant,
                        description: format!("Constant: individual named \"{}\"", word),
                    })
                }
            }

            _ => None,
        };

        if let Some(e) = entry {
            if !seen.contains(&e.symbol) {
                seen.insert(e.symbol.clone());
                symbols.push(e);
            }
        }
    }

    symbols
}

/// Convert a predicate name to a more human-readable form
fn humanize_predicate(name: &str) -> String {
    // Handle common patterns
    match name {
        "D" => "is a dog".to_string(),
        "C" => "is a cat".to_string(),
        "B" => "barks".to_string(),
        "M" => "is a man / is mortal".to_string(),
        "W" => "is a woman / walks".to_string(),
        "L" => "loves".to_string(),
        "R" => "runs".to_string(),
        "S" => "sleeps / is a student".to_string(),
        "P" => "is a property".to_string(),
        "Q" => "is a property".to_string(),
        _ if name.len() == 1 => format!("property {}", name),
        _ => format!("\"{}\"", name.to_lowercase()),
    }
}

/// Group symbols by their kind for display
pub fn group_symbols_by_kind(symbols: &[SymbolEntry]) -> Vec<(SymbolKind, Vec<&SymbolEntry>)> {
    let kinds = [
        SymbolKind::Quantifier,
        SymbolKind::Connective,
        SymbolKind::Modal,
        SymbolKind::Predicate,
        SymbolKind::Variable,
        SymbolKind::Constant,
        SymbolKind::Temporal,
    ];

    kinds
        .iter()
        .filter_map(|&kind| {
            let entries: Vec<&SymbolEntry> = symbols
                .iter()
                .filter(|s| s.kind == kind)
                .collect();

            if entries.is_empty() {
                None
            } else {
                Some((kind, entries))
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_humanize_predicate_single_letter() {
        assert!(humanize_predicate("D").contains("dog"));
        assert!(humanize_predicate("L").contains("loves"));
    }

    #[test]
    fn test_humanize_predicate_word() {
        assert!(humanize_predicate("Dog").contains("dog"));
        assert!(humanize_predicate("Loves").contains("loves"));
    }

    #[test]
    fn test_group_symbols_by_kind() {
        let symbols = extract_symbols("∀x(D(x) ∧ B(x))");
        let grouped = group_symbols_by_kind(&symbols);

        assert!(!grouped.is_empty());

        // Find quantifiers group
        let quantifiers = grouped.iter()
            .find(|(k, _)| *k == SymbolKind::Quantifier);
        assert!(quantifiers.is_some());
    }

    #[test]
    fn test_symbol_kind_labels() {
        assert_eq!(SymbolKind::Quantifier.label(), "Quantifier");
        assert_eq!(SymbolKind::Connective.label(), "Connective");
        assert_eq!(SymbolKind::Modal.label(), "Modal");
    }
}
