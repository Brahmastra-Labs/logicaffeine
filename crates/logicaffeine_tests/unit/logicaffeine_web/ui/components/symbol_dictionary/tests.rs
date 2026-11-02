use super::*;

#[test]
fn test_symbol_kind_class_names() {
    assert_eq!(SymbolKind::Quantifier.label(), "Quantifier");
    assert_eq!(SymbolKind::Connective.label(), "Connective");
}
