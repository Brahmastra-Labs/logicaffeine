use std::collections::HashMap;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Symbol(u32);

impl Symbol {
    pub const EMPTY: Symbol = Symbol(0);

    pub fn index(self) -> usize {
        self.0 as usize
    }
}

impl Default for Symbol {
    fn default() -> Self {
        Self::EMPTY
    }
}

pub struct Interner {
    map: HashMap<String, Symbol>,
    vec: Vec<String>,
}

impl Interner {
    pub fn new() -> Self {
        let mut interner = Interner {
            map: HashMap::new(),
            vec: Vec::new(),
        };
        interner.vec.push(String::new());
        interner
    }

    pub fn intern(&mut self, s: &str) -> Symbol {
        if let Some(&sym) = self.map.get(s) {
            return sym;
        }
        let sym = Symbol(self.vec.len() as u32);
        self.vec.push(s.to_string());
        self.map.insert(s.to_string(), sym);
        sym
    }

    pub fn resolve(&self, sym: Symbol) -> &str {
        &self.vec[sym.0 as usize]
    }

    /// Look up an existing interned string without creating a new entry.
    /// Returns None if the string has not been interned.
    pub fn lookup(&self, s: &str) -> Option<Symbol> {
        self.map.get(s).copied()
    }

    pub fn len(&self) -> usize {
        self.vec.len()
    }

    pub fn is_empty(&self) -> bool {
        self.vec.len() <= 1
    }
}

impl Default for Interner {
    fn default() -> Self {
        Self::new()
    }
}

pub trait SymbolEq {
    fn is(&self, interner: &Interner, s: &str) -> bool;
}

impl SymbolEq for Symbol {
    #[inline]
    fn is(&self, interner: &Interner, s: &str) -> bool {
        interner.resolve(*self) == s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intern_returns_same_symbol_for_same_string() {
        let mut interner = Interner::new();
        let s1 = interner.intern("hello");
        let s2 = interner.intern("hello");
        assert_eq!(s1, s2);
    }

    #[test]
    fn intern_returns_different_symbols_for_different_strings() {
        let mut interner = Interner::new();
        let s1 = interner.intern("hello");
        let s2 = interner.intern("world");
        assert_ne!(s1, s2);
    }

    #[test]
    fn resolve_returns_original_string() {
        let mut interner = Interner::new();
        let sym = interner.intern("test");
        assert_eq!(interner.resolve(sym), "test");
    }

    #[test]
    fn empty_symbol_resolves_to_empty_string() {
        let interner = Interner::new();
        assert_eq!(interner.resolve(Symbol::EMPTY), "");
    }

    #[test]
    fn symbols_are_copy() {
        let mut interner = Interner::new();
        let s1 = interner.intern("copy_test");
        let s2 = s1;
        assert_eq!(s1, s2);
        assert_eq!(interner.resolve(s1), interner.resolve(s2));
    }

    #[test]
    fn symbol_equality_is_fast() {
        let mut interner = Interner::new();
        let s1 = interner.intern("a_very_long_string_that_would_be_slow_to_compare");
        let s2 = interner.intern("a_very_long_string_that_would_be_slow_to_compare");
        assert_eq!(s1, s2);
    }

    #[test]
    fn len_tracks_interned_count() {
        let mut interner = Interner::new();
        assert_eq!(interner.len(), 1);
        interner.intern("first");
        assert_eq!(interner.len(), 2);
        interner.intern("second");
        assert_eq!(interner.len(), 3);
        interner.intern("first");
        assert_eq!(interner.len(), 3);
    }

    #[test]
    fn is_empty_after_new() {
        let interner = Interner::new();
        assert!(interner.is_empty());
    }

    #[test]
    fn not_empty_after_intern() {
        let mut interner = Interner::new();
        interner.intern("something");
        assert!(!interner.is_empty());
    }

    #[test]
    fn symbol_index_matches_position() {
        let mut interner = Interner::new();
        let s1 = interner.intern("first");
        let s2 = interner.intern("second");
        assert_eq!(s1.index(), 1);
        assert_eq!(s2.index(), 2);
    }

    #[test]
    fn symbol_is_matches_interned_string() {
        let mut interner = Interner::new();
        let sym = interner.intern("test");
        assert!(sym.is(&interner, "test"));
    }

    #[test]
    fn symbol_is_rejects_different_string() {
        let mut interner = Interner::new();
        let sym = interner.intern("hello");
        assert!(!sym.is(&interner, "world"));
    }

    #[test]
    fn symbol_is_case_sensitive() {
        let mut interner = Interner::new();
        let sym = interner.intern("Test");
        assert!(!sym.is(&interner, "test"));
        assert!(sym.is(&interner, "Test"));
    }

    #[test]
    fn symbol_empty_is_empty_string() {
        let interner = Interner::new();
        assert!(Symbol::EMPTY.is(&interner, ""));
    }
}
