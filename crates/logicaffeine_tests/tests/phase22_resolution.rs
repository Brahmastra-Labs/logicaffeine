use logicaffeine_language::scope::{ScopeStack, ScopeEntry};

#[test]
fn resolution_order_scope_before_fallback() {
    let mut stack = ScopeStack::new();
    stack.push_scope();

    // Bind a variable "x"
    stack.bind("x", ScopeEntry::variable("x"));

    // Resolution should find "x" in scope
    assert!(stack.lookup("x").is_some(), "Should find x in scope");

    // Unknown variable should not be found
    assert!(stack.lookup("unknown").is_none(), "Should not find unknown");
}

#[test]
fn resolution_shadowing_works() {
    let mut stack = ScopeStack::new();
    stack.push_scope();
    stack.bind("x", ScopeEntry::variable("outer_x"));
    stack.push_scope();
    stack.bind("x", ScopeEntry::variable("inner_x"));

    // Should find the inner (most recent) binding
    let entry = stack.lookup("x").unwrap();
    assert_eq!(entry.symbol, "inner_x");

    // Pop inner scope
    stack.pop_scope();

    // Should find outer binding now
    let entry = stack.lookup("x").unwrap();
    assert_eq!(entry.symbol, "outer_x");
}

#[test]
fn resolution_ownership_tracked() {
    use logicaffeine_language::drs::OwnershipState;

    let mut stack = ScopeStack::new();
    stack.push_scope();
    stack.bind("data", ScopeEntry::variable("data"));

    // Default is Owned
    let entry = stack.lookup("data").unwrap();
    assert_eq!(entry.ownership, OwnershipState::Owned);

    // Can modify ownership
    if let Some(entry) = stack.lookup_mut("data") {
        entry.ownership = OwnershipState::Moved;
    }

    let entry = stack.lookup("data").unwrap();
    assert_eq!(entry.ownership, OwnershipState::Moved);
}
