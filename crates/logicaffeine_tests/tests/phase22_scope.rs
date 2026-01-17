use logicaffeine_language::scope::{ScopeStack, ScopeEntry};

#[test]
fn scope_stack_starts_empty() {
    let stack = ScopeStack::new();
    assert!(stack.lookup("x").is_none());
}

#[test]
fn can_bind_and_lookup_variable() {
    let mut stack = ScopeStack::new();
    stack.push_scope();
    stack.bind("count", ScopeEntry::variable("count"));
    assert!(stack.lookup("count").is_some());
}

#[test]
fn nested_scopes_shadow_outer() {
    let mut stack = ScopeStack::new();
    stack.push_scope();
    stack.bind("x", ScopeEntry::variable("x"));
    stack.push_scope();
    stack.bind("x", ScopeEntry::variable("inner_x"));
    assert_eq!(stack.lookup("x").unwrap().symbol, "inner_x");
}

#[test]
fn pop_scope_removes_bindings() {
    let mut stack = ScopeStack::new();
    stack.push_scope();
    stack.bind("temp", ScopeEntry::variable("temp"));
    stack.pop_scope();
    assert!(stack.lookup("temp").is_none());
}

#[test]
fn shadowed_variable_restored_after_pop() {
    let mut stack = ScopeStack::new();
    stack.push_scope();
    stack.bind("x", ScopeEntry::variable("outer"));
    stack.push_scope();
    stack.bind("x", ScopeEntry::variable("inner"));
    assert_eq!(stack.lookup("x").unwrap().symbol, "inner");
    stack.pop_scope();
    assert_eq!(stack.lookup("x").unwrap().symbol, "outer");
}

#[test]
fn lookup_mut_can_modify_entry() {
    let mut stack = ScopeStack::new();
    stack.push_scope();
    stack.bind("x", ScopeEntry::variable("x"));

    if let Some(entry) = stack.lookup_mut("x") {
        entry.symbol = "modified".to_string();
    }

    assert_eq!(stack.lookup("x").unwrap().symbol, "modified");
}
