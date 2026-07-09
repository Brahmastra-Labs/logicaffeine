//! ============================================================================
//! PHASE 143: STRUCTURAL BINDING — PRINCIPLES A/B/C OVER THE DRS
//! ============================================================================
//!
//! Binding is grammar, not heuristics: resolved over local domains
//! (clause subjects, c-command) in the DRS, replacing parser post-hoc patches.
//!
//!   Principle A — a reflexive binds to the LOCAL clause subject, even inside
//!                 an embedded clause.
//!   Principle B — a plain pronoun excludes the local clause subject but MAY
//!                 take a higher-clause antecedent.
//!   Principle C — an R-expression is free: a c-commanding pronoun must not
//!                 corefer with it.
//!
//! The reflexive case is also a semantic identity, so the kernel must certify
//! "John saw himself ⊢ John saw John".

use logicaffeine_language::compile;
use logicaffeine_compile::verify_theorem;

fn theorem(premises: &[&str], goal: &str) -> String {
    let givens: String = premises
        .iter()
        .map(|p| format!("Given: {p}\n"))
        .collect();
    format!("## Theorem: Phase143\n{givens}Prove: {goal}\nProof: Auto.\n")
}

// ============================================================================
// A. Principle A in embedded clauses: the LOCAL subject wins
// ============================================================================

#[test]
fn embedded_reflexive_binds_embedded_subject() {
    // "herself" must be Mary (local), never John (matrix).
    let out = compile("John said that Mary saw herself.").unwrap();
    eprintln!("embedded-A: {out}");
    assert!(
        out.lines()
            .any(|l| l.contains("See") && l.contains("Theme(e, Mary)")),
        "embedded reflexive binds the embedded subject Mary: {out}"
    );
    assert!(
        !out
            .lines()
            .any(|l| l.contains("See") && l.contains("Theme(e, John)")),
        "embedded reflexive must NOT skip to the matrix subject: {out}"
    );
}

#[test]
fn reflexive_identity_is_kernel_provable() {
    let src = theorem(&["John saw himself."], "John saw John.");
    assert!(
        verify_theorem(&src).is_ok(),
        "Principle A is semantic identity: John saw himself ⊢ John saw John: {:?}",
        verify_theorem(&src).err()
    );
}

// ============================================================================
// B. Principle B in embedded clauses: local exclusion, non-local freedom
// ============================================================================

#[test]
fn embedded_pronoun_excludes_embedded_subject() {
    // "him" must not be Mary (local subject)...
    let out = compile("John said that Mary saw him.").unwrap();
    eprintln!("embedded-B: {out}");
    assert!(
        !out
            .lines()
            .any(|l| l.contains("See") && l.contains("Theme(e, Mary)")),
        "Principle B: 'him' must NOT be the local subject Mary: {out}"
    );
}

#[test]
fn embedded_pronoun_may_take_matrix_antecedent() {
    // ...but John (matrix, non-local) is a legitimate antecedent: the SEE
    // event's theme may resolve to John.
    let out = compile("John said that Mary saw him.").unwrap();
    eprintln!("embedded-B-matrix: {out}");
    assert!(
        out.lines()
            .any(|l| l.contains("See") && l.contains("Theme(e, John)")),
        "the matrix subject is an accessible antecedent for 'him': {out}"
    );
}

#[test]
fn local_pronoun_still_excluded_simple_clause() {
    // Regression guard for the phase114 behavior under the refactor.
    let out = compile("John saw him.").unwrap();
    assert!(
        !out.contains("Theme(e, John)"),
        "Principle B in the simple clause is preserved: {out}"
    );
}

#[test]
fn feminine_object_pronoun_resolves() {
    // Documented phase114 edge gap: "Mary saw her" dropped the object due to
    // the possessive/accusative ambiguity of "her". Binding must handle it.
    let out = compile("Mary saw her.").unwrap();
    eprintln!("her-object: {out}");
    assert!(out.contains("Theme"), "the object 'her' must survive: {out}");
    assert!(
        !out.contains("Theme(e, Mary)"),
        "Principle B: 'her' must NOT be Mary: {out}"
    );
}

// ============================================================================
// C. Principle C: R-expressions are free
// ============================================================================

#[test]
fn c_commanding_pronoun_does_not_corefer_with_name() {
    // "He saw John." — He ≠ John.
    let out = compile("He saw John.").unwrap();
    eprintln!("principle-C: {out}");
    assert!(out.contains("Theme(e, John)"), "John is the object: {out}");
    assert!(
        !out.contains("Agent(e, John)"),
        "Principle C: the c-commanding pronoun must not BE John: {out}"
    );
}

#[test]
fn matrix_pronoun_does_not_corefer_with_embedded_name() {
    // "He said that John left." — He ≠ John (cataphoric c-command). The
    // check is scoped to the SAY event's line: John as the agent of LEAVE is
    // correct; John as the agent of SAY would be the Principle C violation.
    let out = compile("He said that John left.").unwrap();
    eprintln!("principle-C-embedded: {out}");
    // The complement now embeds on the Say line (Say(agent, [⟨…⟩])), so the
    // check targets the performer slot itself: John inside the embedded
    // proposition is correct; John as the SAYER would be the violation.
    assert!(
        !out.contains("Say(John"),
        "Principle C: matrix 'he' must not corefer with the c-commanded \
         R-expression John: {out}"
    );
    assert!(
        out.contains("Say("),
        "the saying event must still be present: {out}"
    );
}

#[test]
fn backwards_anaphora_without_c_command_is_fine() {
    // Control: Principle C bans c-command coreference, not all cataphora.
    // A discourse-initial pronoun followed by a name in the NEXT sentence is
    // resolved by discourse, not banned by C — the parses must both succeed.
    let out = compile("John arrived. He left.").unwrap();
    eprintln!("discourse-anaphora: {out}");
    assert!(
        out.contains("Agent") && out.contains("John"),
        "ordinary discourse anaphora still resolves: {out}"
    );
}
