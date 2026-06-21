//! Regression pin for Bug Report #1, BUG-014.
//!
//! Policy-condition discovery must fold the FULL n-ary AND/OR chain; dropping
//! conjuncts/disjuncts past the second atom makes an AND-policy over-permissive
//! and an OR-policy over-restrictive.

use logicaffeine_compile::compile_to_rust;

#[test]
fn policy_three_or_conditions_keeps_all_disjuncts() {
    let source = r#"## Definition
A User has:
    a role, which is Text.

## Policy

A User is privileged if:
    The user's role equals "admin", OR
    The user's role equals "moderator", OR
    The user's role equals "editor".

## Main
    Let u be a new User."#;

    let rust = compile_to_rust(source).expect("Should compile 3-way OR conditions");
    assert!(rust.contains("self.role == \"admin\""), "missing admin disjunct:\n{}", rust);
    assert!(rust.contains("self.role == \"moderator\""), "missing moderator disjunct:\n{}", rust);
    assert!(
        rust.contains("self.role == \"editor\""),
        "THIRD disjunct dropped by parse_policy_condition (returned after first connective):\n{}",
        rust
    );
    assert!(rust.matches("||").count() >= 2, "expected >=2 '||' for a 3-way OR chain:\n{}", rust);
}

#[test]
fn policy_three_and_conditions_keeps_all_conjuncts() {
    // An AND-policy that drops a conjunct is STRICTLY MORE PERMISSIVE than written.
    let source = r#"## Definition
A User has:
    a role, which is Text.
    a status, which is Text.
    a tier, which is Text.

## Policy

A User is allowed if:
    The user's role equals "admin", AND
    The user's status equals "active", AND
    The user's tier equals "gold".

## Main
    Let u be a new User."#;

    let rust = compile_to_rust(source).expect("Should compile 3-way AND conditions");
    assert!(rust.contains("self.role == \"admin\""), "missing role conjunct:\n{}", rust);
    assert!(rust.contains("self.status == \"active\""), "missing status conjunct:\n{}", rust);
    assert!(
        rust.contains("self.tier == \"gold\""),
        "THIRD conjunct dropped (over-permissive policy!):\n{}",
        rust
    );
    assert!(rust.matches("&&").count() >= 2, "expected >=2 '&&' for a 3-way AND chain:\n{}", rust);
}
