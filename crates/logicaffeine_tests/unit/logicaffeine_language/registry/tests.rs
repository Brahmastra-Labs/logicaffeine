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
