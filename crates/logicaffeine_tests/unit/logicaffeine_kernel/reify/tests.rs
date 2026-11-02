use super::*;

#[test]
fn interner_distinct_names_distinct_indices() {
    let mut it = VarInterner::new();
    let a = it.intern("Aa");
    let b = it.intern("BB");
    assert_ne!(a, b);
}

#[test]
fn interner_same_name_same_index() {
    let mut it = VarInterner::new();
    assert_eq!(it.intern("x"), it.intern("x"));
}

#[test]
fn interner_indices_negative_below_svar_range() {
    let mut it = VarInterner::new();
    assert!(it.intern("x") <= -1_000_000);
}
