//! Phase 95: The Incompleteness Theorem
//!
//! States Gödel's First Incompleteness Theorem:
//! - Consistent: The system cannot derive False
//! - Godel_I: Consistent -> Not (Provable G)

use logicaffeine_kernel::interface::Repl;

// =============================================================================
// CONSISTENT: THE CONSISTENCY PREDICATE
// =============================================================================

#[test]
fn test_consistent_definition() {
    let mut repl = Repl::new();

    // Set up Provable
    repl.execute(
        r#"
        Definition Provable : Syntax -> Prop :=
          fun s : Syntax => Ex Derivation (fun d : Derivation => Eq Syntax (concludes d) s).
    "#,
    )
    .expect("Define Provable");

    // Define Consistent: the system cannot prove False
    repl.execute(
        r#"
        Definition Consistent : Prop := Not (Provable (SName "False")).
    "#,
    )
    .expect("Define Consistent");

    let result = repl.execute("Check Consistent.").expect("Check");
    assert_eq!(result, "Consistent : Prop");
}

#[test]
fn test_consistent_unfolds() {
    let mut repl = Repl::new();

    repl.execute(
        r#"
        Definition Provable : Syntax -> Prop :=
          fun s : Syntax => Ex Derivation (fun d : Derivation => Eq Syntax (concludes d) s).
    "#,
    )
    .expect("Define Provable");

    repl.execute(
        r#"
        Definition Consistent : Prop := Not (Provable (SName "False")).
    "#,
    )
    .expect("Define Consistent");

    // Consistent should unfold to Not (Provable (SName "False"))
    let result = repl.execute("Eval Consistent.").expect("Eval");
    // The result should contain the unfolded form
    // Not unfolds to P -> False, so we check for "->" or "False"
    assert!(
        result.contains("->") || result.contains("False"),
        "Expected Consistent to unfold, got: {}",
        result
    );
}

#[test]
fn test_sname_false_type() {
    let mut repl = Repl::new();

    // SName "False" should have type Syntax
    let result = repl
        .execute("Check (SName \"False\").")
        .expect("Check");
    assert_eq!(result, "(SName \"False\") : Syntax");
}

#[test]
fn test_provable_sname_false_type() {
    let mut repl = Repl::new();

    repl.execute(
        r#"
        Definition Provable : Syntax -> Prop :=
          fun s : Syntax => Ex Derivation (fun d : Derivation => Eq Syntax (concludes d) s).
    "#,
    )
    .expect("Define Provable");

    // Provable (SName "False") should have type Prop
    let result = repl
        .execute("Check (Provable (SName \"False\")).")
        .expect("Check");
    assert_eq!(result, "(Provable (SName \"False\")) : Prop");
}

// =============================================================================
// THE INCOMPLETENESS THEOREM STATEMENT
// =============================================================================

#[test]
fn test_godel_i_statement_welltyped() {
    let mut repl = Repl::new();

    // Full setup
    repl.execute(
        r#"
        Definition Provable : Syntax -> Prop :=
          fun s : Syntax => Ex Derivation (fun d : Derivation => Eq Syntax (concludes d) s).
    "#,
    )
    .expect("Define Provable");

    repl.execute(
        r#"
        Definition T : Syntax := SApp (SName "Not") (SApp (SName "Provable") (SVar 0)).
    "#,
    )
    .expect("Define T");

    repl.execute("Definition G : Syntax := syn_diag T.")
        .expect("Define G");

    repl.execute(
        r#"
        Definition Consistent : Prop := Not (Provable (SName "False")).
    "#,
    )
    .expect("Define Consistent");

    // THE THEOREM STATEMENT should be well-typed as Prop
    let result = repl
        .execute("Check (Consistent -> Not (Provable G)).")
        .expect("Check");
    assert_eq!(result, "Consistent -> (Not (Provable G)) : Prop");
}

#[test]
fn test_godel_i_components() {
    let mut repl = Repl::new();

    repl.execute(
        r#"
        Definition Provable : Syntax -> Prop :=
          fun s : Syntax => Ex Derivation (fun d : Derivation => Eq Syntax (concludes d) s).
    "#,
    )
    .expect("Define Provable");

    repl.execute(
        r#"
        Definition T : Syntax := SApp (SName "Not") (SApp (SName "Provable") (SVar 0)).
    "#,
    )
    .expect("Define T");

    repl.execute("Definition G : Syntax := syn_diag T.")
        .expect("Define G");

    repl.execute(
        r#"
        Definition Consistent : Prop := Not (Provable (SName "False")).
    "#,
    )
    .expect("Define Consistent");

    // Check each component type
    let consistent = repl.execute("Check Consistent.").expect("Check");
    assert_eq!(consistent, "Consistent : Prop");

    let provable_g = repl.execute("Check (Provable G).").expect("Check");
    assert_eq!(provable_g, "(Provable G) : Prop");

    let not_provable_g = repl.execute("Check (Not (Provable G)).").expect("Check");
    assert_eq!(not_provable_g, "(Not (Provable G)) : Prop");
}

// =============================================================================
// NAMED THEOREM (USING DEFINITION)
// =============================================================================

#[test]
fn test_godel_i_as_type_alias() {
    let mut repl = Repl::new();

    repl.execute(
        r#"
        Definition Provable : Syntax -> Prop :=
          fun s : Syntax => Ex Derivation (fun d : Derivation => Eq Syntax (concludes d) s).
    "#,
    )
    .expect("Define Provable");

    repl.execute(
        r#"
        Definition T : Syntax := SApp (SName "Not") (SApp (SName "Provable") (SVar 0)).
    "#,
    )
    .expect("Define T");

    repl.execute("Definition G : Syntax := syn_diag T.")
        .expect("Define G");

    repl.execute(
        r#"
        Definition Consistent : Prop := Not (Provable (SName "False")).
    "#,
    )
    .expect("Define Consistent");

    // Define the theorem statement as a type alias
    repl.execute(
        r#"
        Definition Godel_I : Prop := Consistent -> Not (Provable G).
    "#,
    )
    .expect("Define Godel_I");

    let result = repl.execute("Check Godel_I.").expect("Check");
    assert_eq!(result, "Godel_I : Prop");
}

// =============================================================================
// SECOND INCOMPLETENESS (PREVIEW)
// =============================================================================

#[test]
fn test_quoted_consistent_type() {
    let mut repl = Repl::new();

    repl.execute(
        r#"
        Definition Provable : Syntax -> Prop :=
          fun s : Syntax => Ex Derivation (fun d : Derivation => Eq Syntax (concludes d) s).
    "#,
    )
    .expect("Define Provable");

    // For Gödel II, we need to quote "Consistent" as syntax
    // Consistent = Not (Provable (SName "False"))
    // So quoted consistent would be: SApp (SName "Not") (SApp (SName "Provable") (SName "False"))
    repl.execute(
        r#"
        Definition ConsistentSyn : Syntax :=
          SApp (SName "Not") (SApp (SName "Provable") (SName "False")).
    "#,
    )
    .expect("Define ConsistentSyn");

    let result = repl.execute("Check ConsistentSyn.").expect("Check");
    assert_eq!(result, "ConsistentSyn : Syntax");
}

#[test]
fn test_godel_ii_statement_welltyped() {
    let mut repl = Repl::new();

    repl.execute(
        r#"
        Definition Provable : Syntax -> Prop :=
          fun s : Syntax => Ex Derivation (fun d : Derivation => Eq Syntax (concludes d) s).
    "#,
    )
    .expect("Define Provable");

    repl.execute(
        r#"
        Definition Consistent : Prop := Not (Provable (SName "False")).
    "#,
    )
    .expect("Define Consistent");

    repl.execute(
        r#"
        Definition ConsistentSyn : Syntax :=
          SApp (SName "Not") (SApp (SName "Provable") (SName "False")).
    "#,
    )
    .expect("Define ConsistentSyn");

    // Gödel II: Consistent -> Not (Provable ConsistentSyn)
    // "If LOGOS is consistent, it cannot prove its own consistency"
    let result = repl
        .execute("Check (Consistent -> Not (Provable ConsistentSyn)).")
        .expect("Check");
    assert_eq!(
        result,
        "Consistent -> (Not (Provable ConsistentSyn)) : Prop"
    );
}

// =============================================================================
// THE REFLECTION PRINCIPLE (FOUNDATION)
// =============================================================================

#[test]
fn test_reflection_type() {
    let mut repl = Repl::new();

    repl.execute(
        r#"
        Definition Provable : Syntax -> Prop :=
          fun s : Syntax => Ex Derivation (fun d : Derivation => Eq Syntax (concludes d) s).
    "#,
    )
    .expect("Define Provable");

    // The reflection principle would be: Provable s -> (interpret s)
    // But we don't have interpret yet. For now, check Provable -> Provable pattern
    // This is a placeholder for future work

    // Check that we can form implications with Provable
    repl.execute("Definition P : Syntax := SName \"P\".")
        .expect("Define P");

    let result = repl
        .execute("Check (Provable P -> Provable P).")
        .expect("Check");
    assert_eq!(result, "(Provable P) -> (Provable P) : Prop");
}

// =============================================================================
// NEGATION SOUNDNESS
// =============================================================================

#[test]
fn test_not_provable_false_from_consistent() {
    let mut repl = Repl::new();

    repl.execute(
        r#"
        Definition Provable : Syntax -> Prop :=
          fun s : Syntax => Ex Derivation (fun d : Derivation => Eq Syntax (concludes d) s).
    "#,
    )
    .expect("Define Provable");

    repl.execute(
        r#"
        Definition Consistent : Prop := Not (Provable (SName "False")).
    "#,
    )
    .expect("Define Consistent");

    // Consistent is DEFINED as Not (Provable (SName "False"))
    // So Consistent -> Not (Provable (SName "False")) should be trivial
    let result = repl
        .execute("Check (Consistent -> Not (Provable (SName \"False\"))).")
        .expect("Check");
    assert_eq!(
        result,
        "Consistent -> (Not (Provable (SName \"False\"))) : Prop"
    );
}

// =============================================================================
// ERROR CASES
// =============================================================================

#[test]
fn test_consistent_requires_provable() {
    let mut repl = Repl::new();

    // Without Provable defined, Consistent should fail
    let result = repl.execute(
        r#"
        Definition Consistent : Prop := Not (Provable (SName "False")).
    "#,
    );
    assert!(result.is_err(), "Should fail without Provable defined");
}

#[test]
fn test_sname_type_error() {
    let mut repl = Repl::new();

    // SName expects Text, not Int
    let result = repl.execute("Check (SName 42).");
    assert!(result.is_err(), "Should reject Int where Text expected");
}
