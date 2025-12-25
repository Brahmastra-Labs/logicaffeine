use std::collections::HashMap;
use crate::intern::{Interner, Symbol};

pub struct SymbolRegistry {
    mapping: HashMap<String, String>,
    counters: HashMap<char, usize>,
}

impl SymbolRegistry {
    pub fn new() -> Self {
        SymbolRegistry {
            mapping: HashMap::new(),
            counters: HashMap::new(),
        }
    }

    pub fn get_symbol_full(&self, sym: Symbol, interner: &Interner) -> String {
        let word = interner.resolve(sym);
        let mut chars = word.chars();
        match chars.next() {
            Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
            None => String::new(),
        }
    }

    pub fn get_symbol(&mut self, sym: Symbol, interner: &Interner) -> String {
        let word = interner.resolve(sym);
        let normalized = word.to_lowercase();

        if let Some(sym) = self.mapping.get(&normalized) {
            return sym.clone();
        }

        // For hyphenated compounds (non-intersective adjectives), return full form
        // "fake-gun" → "Fake-Gun" (not "F")
        if word.contains('-') {
            let compound: String = word
                .split('-')
                .map(|part| {
                    let mut chars = part.chars();
                    match chars.next() {
                        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
                        None => String::new(),
                    }
                })
                .collect::<Vec<_>>()
                .join("-");
            self.mapping.insert(normalized, compound.clone());
            return compound;
        }

        // Preserve specific relational terms (bridging markers)
        const PRESERVED_TERMS: &[&str] = &["PartOf"];
        if PRESERVED_TERMS.iter().any(|t| t.eq_ignore_ascii_case(word)) {
            self.mapping.insert(normalized, word.to_string());
            return word.to_string();
        }

        let first = normalized
            .chars()
            .next()
            .unwrap()
            .to_uppercase()
            .next()
            .unwrap();

        let counter = self.counters.entry(first).or_insert(0);
        *counter += 1;

        let symbol = if *counter == 1 {
            first.to_string()
        } else {
            format!("{}{}", first, counter)
        };

        self.mapping.insert(normalized, symbol.clone());
        symbol
    }
}

impl Default for SymbolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_word_gets_single_letter() {
        let mut interner = Interner::new();
        let mut reg = SymbolRegistry::new();
        let dog = interner.intern("dog");
        assert_eq!(reg.get_symbol(dog, &interner), "D");
    }

    #[test]
    fn second_word_same_letter_gets_numbered() {
        let mut interner = Interner::new();
        let mut reg = SymbolRegistry::new();
        let dog = interner.intern("dog");
        let dangerous = interner.intern("dangerous");
        reg.get_symbol(dog, &interner);
        assert_eq!(reg.get_symbol(dangerous, &interner), "D2");
    }

    #[test]
    fn same_word_returns_same_symbol() {
        let mut interner = Interner::new();
        let mut reg = SymbolRegistry::new();
        let cat = interner.intern("cat");
        let first = reg.get_symbol(cat, &interner);
        let second = reg.get_symbol(cat, &interner);
        assert_eq!(first, second);
    }

    #[test]
    fn case_insensitive() {
        let mut interner = Interner::new();
        let mut reg = SymbolRegistry::new();
        let dog = interner.intern("dog");
        let dog_upper = interner.intern("DOG");
        let lower = reg.get_symbol(dog, &interner);
        let upper = reg.get_symbol(dog_upper, &interner);
        assert_eq!(lower, upper);
    }
}
