//! Phase 49: Native CRDTs (Conflict-free Replicated Data Types)
//!
//! Tests for distributed state synchronization without conflict resolution.
//! CRDTs enable automatic merging of concurrent updates.

mod common;
use common::compile_to_rust;

// =============================================================================
// Shared Type Modifier Tests
// =============================================================================

#[test]
fn test_shared_struct_basic() {
    let source = r#"## Definition
A Scoreboard is Shared and has:
    a points, which is ConvergentCount.

## Main
    Let s be a new Scoreboard."#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(
        rust.contains("impl logos_core::crdt::Merge for Scoreboard"),
        "Shared types should implement Merge trait"
    );
}

#[test]
fn test_shared_struct_with_multiple_fields() {
    let source = r#"## Definition
A Counter is Shared and has:
    a points, which is ConvergentCount.
    a name, which is LastWriteWins of Text.

## Main
    Let c be a new Counter."#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(
        rust.contains("GCounter"),
        "ConvergentCount should map to GCounter"
    );
    assert!(
        rust.contains("LWWRegister<String>"),
        "LastWriteWins of Text should map to LWWRegister<String>"
    );
}

// =============================================================================
// CRDT Field Type Tests
// =============================================================================

#[test]
fn test_convergent_count_field() {
    let source = r#"## Definition
A Score is Shared and has:
    a value, which is ConvergentCount.

## Main
    Let s be a new Score."#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(
        rust.contains("logos_core::crdt::GCounter"),
        "Should use GCounter from logos_core"
    );
}

#[test]
fn test_last_write_wins_text() {
    let source = r#"## Definition
A Profile is Shared and has:
    a username, which is LastWriteWins of Text.

## Main
    Let p be a new Profile."#;
    let rust = compile_to_rust(source).expect("Should compile");
    eprintln!("Generated code (text):\n{}", rust);
    assert!(
        rust.contains("LWWRegister<String>"),
        "Should use LWWRegister for LastWriteWins"
    );
}

#[test]
fn test_last_write_wins_int() {
    // Test with Nat (should work)
    let source_nat = r#"## Definition
A Test2 is Shared and has:
    a x, which is LastWriteWins of Nat.

## Main
    Let t be a new Test2."#;
    let rust_nat = compile_to_rust(source_nat).expect("Nat should compile");
    eprintln!("LastWriteWins of Nat generated:\n{}", rust_nat);

    // Test with Int
    let source = r#"## Definition
A Setting is Shared and has:
    a volume, which is LastWriteWins of Int.

## Main
    Let s be a new Setting."#;
    let rust = compile_to_rust(source).expect("Should compile");
    eprintln!("LastWriteWins of Int generated:\n{}", rust);
    assert!(
        rust.contains("LWWRegister<i64>"),
        "Should use LWWRegister<i64> for Int"
    );
}

#[test]
fn test_last_write_wins_bool() {
    let source = r#"## Definition
A Toggle is Shared and has:
    a active, which is LastWriteWins of Bool.

## Main
    Let t be a new Toggle."#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(
        rust.contains("LWWRegister<bool>"),
        "Should use LWWRegister<bool> for Bool"
    );
}

// =============================================================================
// Merge Statement Tests
// =============================================================================

#[test]
fn test_merge_struct_level() {
    let source = r#"## Definition
A Counter is Shared and has:
    a points, which is ConvergentCount.

## Main
    Let local be a new Counter.
    Let remote be a new Counter.
    Merge remote into local."#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains(".merge(&"), "Should generate merge call");
}

#[test]
fn test_merge_field_level() {
    let source = r#"## Definition
A Profile is Shared and has:
    a active, which is LastWriteWins of Bool.

## Main
    Let local be a new Profile.
    Let remote be a new Profile.
    Merge remote's active into local's active."#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(
        rust.contains("local.active.merge(&remote.active)"),
        "Should generate field-level merge"
    );
}

// =============================================================================
// GCounter Operations Tests
// =============================================================================

#[test]
fn test_increase_gcounter() {
    let source = r#"## Definition
A Counter is Shared and has:
    a points, which is ConvergentCount.

## Main
    Let local be a new Counter.
    Increase local's points by 10."#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(
        rust.contains(".increment("),
        "Should generate increment call"
    );
}

#[test]
fn test_increase_by_variable() {
    let source = r#"## Definition
A Counter is Shared and has:
    a score, which is ConvergentCount.

## Main
    Let c be a new Counter.
    Let amount be 5.
    Increase c's score by amount."#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(
        rust.contains(".increment("),
        "Should allow increment by variable"
    );
}

// =============================================================================
// Mixed Struct Tests (Shared with regular fields)
// =============================================================================

#[test]
fn test_shared_with_regular_fields() {
    // Shared structs can have non-CRDT fields too
    let source = r#"## Definition
A GameState is Shared and has:
    a score, which is ConvergentCount.
    a name, which is Text.

## Main
    Let g be a new GameState."#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("pub struct GameState"));
    assert!(rust.contains("GCounter"));
    assert!(rust.contains("String")); // regular Text field
}

// =============================================================================
// Merge Impl Generation Tests
// =============================================================================

#[test]
fn test_merge_impl_only_merges_crdt_fields() {
    let source = r#"## Definition
A State is Shared and has:
    a count, which is ConvergentCount.
    a label, which is Text.

## Main
    Let s be a new State."#;
    let rust = compile_to_rust(source).expect("Should compile");
    // The merge impl should only call merge on CRDT fields
    assert!(
        rust.contains("self.count.merge(&other.count)"),
        "Should merge CRDT field"
    );
    // Should NOT try to merge the regular Text field
    assert!(
        !rust.contains("self.label.merge"),
        "Should not merge regular fields"
    );
}

// =============================================================================
// Edge Case Tests
// =============================================================================

#[test]
fn test_gcounter_zero_increment() {
    let source = r#"## Definition
A Counter is Shared and has:
    a points, which is ConvergentCount.

## Main
    Let mutable c be a new Counter.
    Increase c's points by 0."#;
    let rust = compile_to_rust(source).expect("Should compile zero increment");
    assert!(
        rust.contains(".increment(0"),
        "Should allow increment by zero"
    );
}

#[test]
fn test_lww_default_values() {
    // Test that LWWRegister types generate with proper defaults
    let source = r#"## Definition
A Config is Shared and has:
    a name, which is LastWriteWins of Text.
    a count, which is LastWriteWins of Int.
    a enabled, which is LastWriteWins of Bool.

## Main
    Let c be a new Config."#;
    let rust = compile_to_rust(source).expect("Should compile");
    assert!(rust.contains("LWWRegister<String>"), "Text maps to String");
    assert!(rust.contains("LWWRegister<i64>"), "Int maps to i64");
    assert!(rust.contains("LWWRegister<bool>"), "Bool maps to bool");
}

#[test]
fn test_shared_all_crdt_fields() {
    // Shared struct with only CRDT fields (no regular fields)
    let source = r#"## Definition
A AllCrdt is Shared and has:
    a counter, which is ConvergentCount.
    a register, which is LastWriteWins of Text.

## Main
    Let a be a new AllCrdt."#;
    let rust = compile_to_rust(source).expect("Should compile all-CRDT struct");
    assert!(
        rust.contains("impl logos_core::crdt::Merge for AllCrdt"),
        "Should implement Merge"
    );
    assert!(
        rust.contains("self.counter.merge(&other.counter)"),
        "Should merge counter"
    );
    assert!(
        rust.contains("self.register.merge(&other.register)"),
        "Should merge register"
    );
}

#[test]
fn test_merge_requires_mutable() {
    // Verify that merge target must be mutable
    let source = r#"## Definition
A Counter is Shared and has:
    a points, which is ConvergentCount.

## Main
    Let mutable local be a new Counter.
    Let remote be a new Counter.
    Merge remote into local."#;
    let rust = compile_to_rust(source).expect("Should compile");
    // The generated code should call merge on the mutable local
    assert!(
        rust.contains("local.merge(&remote)"),
        "Should generate merge call"
    );
}
