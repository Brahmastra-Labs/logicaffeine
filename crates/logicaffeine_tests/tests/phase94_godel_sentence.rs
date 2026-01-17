//! Phase 94: The Gödel Sentence (G)
//!
//! Constructs the formal Gödel sentence using:
//! - Provable: The meta-predicate (exists d, concludes d = s)
//! - syn_diag: Self-reference from Phase 93
//! - G: The sentence "I am not provable"

use logicaffeine_kernel::interface::Repl;

// =============================================================================
// PROVABLE: THE META-PREDICATE
// =============================================================================

#[test]
fn test_provable_definition() {
    let mut repl = Repl::new();

    // Define Provable using Ex and Eq
    repl.execute(
        r#"
        Definition Provable : Syntax -> Prop :=
          fun s : Syntax => Ex Derivation (fun d : Derivation => Eq Syntax (concludes d) s).
    "#,
    )
    .expect("Define Provable");

    let result = repl.execute("Check Provable.").expect("Check Provable");
    assert_eq!(result, "Provable : Syntax -> Prop");
}

#[test]
fn test_provable_applied_to_syntax() {
    let mut repl = Repl::new();

    repl.execute(
        r#"
        Definition Provable : Syntax -> Prop :=
          fun s : Syntax => Ex Derivation (fun d : Derivation => Eq Syntax (concludes d) s).
    "#,
    )
    .expect("Define Provable");

    repl.execute("Definition P : Syntax := SName \"P\".")
        .expect("Define P");

    // Provable P should have type Prop
    let result = repl.execute("Check (Provable P).").expect("Check");
    assert_eq!(result, "(Provable P) : Prop");
}

#[test]
fn test_provable_witnesses_axiom() {
    let mut repl = Repl::new();

    repl.execute(
        r#"
        Definition Provable : Syntax -> Prop :=
          fun s : Syntax => Ex Derivation (fun d : Derivation => Eq Syntax (concludes d) s).
    "#,
    )
    .expect("Define Provable");

    // Define a syntax term and its derivation
    repl.execute("Definition P : Syntax := SName \"P\".")
        .expect("Define P");
    repl.execute("Definition d : Derivation := DAxiom P.")
        .expect("Define d");

    // concludes d = P, so we should be able to construct a proof of Provable P
    let result = repl.execute("Check (concludes d).").expect("Check");
    assert_eq!(result, "(concludes d) : Syntax");

    // Verify concludes d evaluates to P
    let concluded = repl.execute("Eval (concludes d).").expect("Eval");
    let p_val = repl.execute("Eval P.").expect("Eval P");
    assert_eq!(concluded, p_val);
}

// =============================================================================
// THE GÖDEL TEMPLATE T
// =============================================================================

#[test]
fn test_godel_template_type() {
    let mut repl = Repl::new();

    // T = SApp (SName "Not") (SApp (SName "Provable") (SVar 0))
    // This represents: Not(Provable(x)) where x is bound by the outer context
    repl.execute(
        r#"
        Definition T : Syntax := SApp (SName "Not") (SApp (SName "Provable") (SVar 0)).
    "#,
    )
    .expect("Define T");

    let result = repl.execute("Check T.").expect("Check T");
    assert_eq!(result, "T : Syntax");
}

#[test]
fn test_godel_template_structure() {
    let mut repl = Repl::new();

    repl.execute(
        r#"
        Definition T : Syntax := SApp (SName "Not") (SApp (SName "Provable") (SVar 0)).
    "#,
    )
    .expect("Define T");

    let result = repl.execute("Eval T.").expect("Eval T");
    // Should show the application structure
    assert!(result.contains("SApp"));
    assert!(result.contains("SName \"Not\""));
    assert!(result.contains("SName \"Provable\""));
    assert!(result.contains("SVar 0"));
}

// =============================================================================
// THE GÖDEL SENTENCE G
// =============================================================================

#[test]
fn test_godel_sentence_type() {
    let mut repl = Repl::new();

    repl.execute(
        r#"
        Definition T : Syntax := SApp (SName "Not") (SApp (SName "Provable") (SVar 0)).
    "#,
    )
    .expect("Define T");

    repl.execute("Definition G : Syntax := syn_diag T.")
        .expect("Define G");

    let result = repl.execute("Check G.").expect("Check G");
    assert_eq!(result, "G : Syntax");
}

#[test]
fn test_godel_sentence_is_fixed_point() {
    let mut repl = Repl::new();

    repl.execute(
        r#"
        Definition T : Syntax := SApp (SName "Not") (SApp (SName "Provable") (SVar 0)).
    "#,
    )
    .expect("Define T");

    repl.execute("Definition G : Syntax := syn_diag T.")
        .expect("Define G");

    // G should equal SApp (SName "Not") (SApp (SName "Provable") (syn_quote T))
    // This is the diagonal lemma: G = T[⌜T⌝/x]
    repl.execute(
        r#"
        Definition expected : Syntax := SApp (SName "Not") (SApp (SName "Provable") (syn_quote T)).
    "#,
    )
    .expect("Define expected");

    let g = repl.execute("Eval G.").expect("Eval G");
    let exp = repl.execute("Eval expected.").expect("Eval expected");
    assert_eq!(g, exp);
}

#[test]
fn test_godel_sentence_contains_not() {
    let mut repl = Repl::new();

    repl.execute(
        r#"
        Definition T : Syntax := SApp (SName "Not") (SApp (SName "Provable") (SVar 0)).
    "#,
    )
    .expect("Define T");

    repl.execute("Definition G : Syntax := syn_diag T.")
        .expect("Define G");

    let result = repl.execute("Eval G.").expect("Eval G");
    assert!(result.contains("SName \"Not\""), "G should contain Not");
}

#[test]
fn test_godel_sentence_contains_provable() {
    let mut repl = Repl::new();

    repl.execute(
        r#"
        Definition T : Syntax := SApp (SName "Not") (SApp (SName "Provable") (SVar 0)).
    "#,
    )
    .expect("Define T");

    repl.execute("Definition G : Syntax := syn_diag T.")
        .expect("Define G");

    let result = repl.execute("Eval G.").expect("Eval G");
    assert!(
        result.contains("SName \"Provable\""),
        "G should contain Provable"
    );
}

#[test]
fn test_godel_sentence_contains_self_reference() {
    let mut repl = Repl::new();

    repl.execute(
        r#"
        Definition T : Syntax := SApp (SName "Not") (SApp (SName "Provable") (SVar 0)).
    "#,
    )
    .expect("Define T");

    repl.execute("Definition G : Syntax := syn_diag T.")
        .expect("Define G");

    // G contains syn_quote T, which should include "SApp", "SName", etc.
    // representing the structure of T itself
    let _result = repl.execute("Eval G.").expect("Eval G");

    // The quoted form should be deeply nested (quoting produces SApp (SName ...) ...)
    // Count nested structures - G should be significantly larger than T
    let g_size: i64 = repl
        .execute("Eval (syn_size G).")
        .expect("size")
        .parse()
        .unwrap();
    let t_size: i64 = repl
        .execute("Eval (syn_size T).")
        .expect("size")
        .parse()
        .unwrap();
    assert!(
        g_size > t_size,
        "G should be larger than T due to self-reference"
    );
}

// =============================================================================
// PROVABLE G: THE META-LEVEL STATEMENT
// =============================================================================

#[test]
fn test_provable_g_type() {
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

    // "Provable G" should be a well-typed proposition
    let result = repl.execute("Check (Provable G).").expect("Check");
    assert_eq!(result, "(Provable G) : Prop");
}

#[test]
fn test_not_provable_g_type() {
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

    // "Not (Provable G)" should also be well-typed
    // Note: We need the actual Not, not SName "Not"
    let result = repl.execute("Check (Not (Provable G)).").expect("Check");
    assert_eq!(result, "(Not (Provable G)) : Prop");
}

// =============================================================================
// VARIANT GÖDEL SENTENCES
// =============================================================================

#[test]
fn test_liar_template() {
    let mut repl = Repl::new();

    // The Liar: "I am not true" (Tarski's version)
    // T = SApp (SName "Not") (SApp (SName "True") (SVar 0))
    repl.execute(
        r#"
        Definition T_liar : Syntax := SApp (SName "Not") (SApp (SName "True") (SVar 0)).
    "#,
    )
    .expect("Define T_liar");

    repl.execute("Definition L : Syntax := syn_diag T_liar.")
        .expect("Define L");

    let result = repl.execute("Eval L.").expect("Eval L");
    assert!(result.contains("SName \"Not\""));
    assert!(result.contains("SName \"True\""));
}

#[test]
fn test_henkin_sentence() {
    let mut repl = Repl::new();

    // Henkin's sentence: "I am provable" (proves itself!)
    // T = SApp (SName "Provable") (SVar 0)
    repl.execute(
        r#"
        Definition T_henkin : Syntax := SApp (SName "Provable") (SVar 0).
    "#,
    )
    .expect("Define T_henkin");

    repl.execute("Definition H : Syntax := syn_diag T_henkin.")
        .expect("Define H");

    let result = repl.execute("Eval H.").expect("Eval H");
    assert!(result.contains("SName \"Provable\""));
    // H should NOT contain "Not"
    assert!(!result.contains("SName \"Not\""));
}

// =============================================================================
// ERROR CASES
// =============================================================================

#[test]
fn test_provable_type_error() {
    let mut repl = Repl::new();

    repl.execute(
        r#"
        Definition Provable : Syntax -> Prop :=
          fun s : Syntax => Ex Derivation (fun d : Derivation => Eq Syntax (concludes d) s).
    "#,
    )
    .expect("Define Provable");

    // Provable expects Syntax, not Int
    let result = repl.execute("Check (Provable 42).");
    assert!(result.is_err(), "Should reject Int where Syntax expected");
}
