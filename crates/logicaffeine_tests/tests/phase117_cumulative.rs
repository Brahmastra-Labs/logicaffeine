//! Phase 117 — §5.2 Cumulative / branching quantification (work/MISSING_ENGLISH.md).
//!
//! A two-cardinal transitive sentence has, beyond its two NESTED scope readings,
//! a CUMULATIVE reading (Scha) irreducible to either nesting:
//!   "Three boys lifted five boxes." →
//!     ∃=3 x(Boy(x) ∧ ∃y(Box(y) ∧ Lift(x,y))) ∧ ∃=5 y(Box(y) ∧ ∃x(Boy(x) ∧ Lift(x,y)))
//! i.e. exactly 3 boys each lifted some box AND exactly 5 boxes were each lifted
//! by some boy. This is first-order over the Link lattice (no ⊕ term needed) and
//! is emitted as an additional reading by `compile_all_scopes`.

use logicaffeine_language::compile_all_scopes;

#[test]
fn two_cardinal_yields_nested_and_cumulative() {
    let rs = compile_all_scopes("Three boys lifted five boxes.").unwrap();
    eprintln!("boys-boxes: {rs:?}");
    // Two nested readings + the cumulative reading.
    assert!(rs.len() >= 3, "nested ×2 + cumulative: {rs:?}");
    // Nested: subject-wide and object-wide.
    assert!(rs.iter().any(|r| r.trim_start().starts_with("∃=3")), "subject-wide nested: {rs:?}");
    assert!(rs.iter().any(|r| r.trim_start().starts_with("∃=5")), "object-wide nested: {rs:?}");
    // Cumulative: a top-level CONJUNCTION of the two counted groups (both cardinals
    // appear as outermost conjuncts, neither nested inside the other).
    let cumulative = rs.iter().find(|r| {
        r.starts_with("(∃=3") && r.contains("∧ ∃=5")
    });
    assert!(cumulative.is_some(), "cumulative (Scha) reading present: {rs:?}");
    let c = cumulative.unwrap();
    assert!(c.contains("Boys") && c.contains("Boxes") && c.contains("Lift"), "cumulative keeps both groups + relation: {c}");
}

#[test]
fn single_cardinal_has_no_cumulative() {
    // Only one cardinal → no cumulative reading (just the two scope readings).
    let rs = compile_all_scopes("Three boys lifted a box.").unwrap();
    eprintln!("one-cardinal: {rs:?}");
    assert!(!rs.iter().any(|r| r.starts_with("(∃=3") && r.contains("∧ ∃")), "no spurious cumulative: {rs:?}");
}

#[test]
fn universal_cardinal_has_no_cumulative() {
    // ∀ + ∃ is not a two-cardinal cumulative configuration.
    let rs = compile_all_scopes("Every student read a book.").unwrap();
    eprintln!("univ: {rs:?}");
    assert!(rs.len() == 2, "just the two nested scope readings: {rs:?}");
}

// ============================================================================
// Entailment spec (§5.2): counting quantifiers have real cardinality
// semantics — ∃=3 entails ∃, and never entails ∀.
// ============================================================================
#[cfg(feature = "verification")]
mod verification_spec {
    use logicaffeine_compile::{check_theorem_premises_consistent, check_theorem_smt};
    use logicaffeine_proof::oracle::{SmtConsistency, SmtVerdict};

    fn theorem(premises: &[&str], goal: &str) -> String {
        let givens: String = premises.iter().map(|p| format!("Given: {p}\n")).collect();
        format!("## Theorem: Phase117V\n{givens}Prove: {goal}\nProof: Auto.\n")
    }

    const CUMUL: &str = "Three boys lifted five boxes.";

    #[test]
    fn counting_entails_existence() {
        let src = theorem(&[CUMUL], "Some boy lifted a box.");
        assert_eq!(
            check_theorem_smt(&src).expect("must parse"),
            SmtVerdict::Entailed,
            "∃=3 boys lifting ⊢ some boy lifted a box"
        );
    }

    #[test]
    fn counting_does_not_entail_universality() {
        let src = theorem(&[CUMUL], "Every boy lifted a box.");
        assert_eq!(
            check_theorem_premises_consistent(&src).expect("must parse"),
            SmtConsistency::Consistent,
            "the counting premise is consistent"
        );
        assert_eq!(
            check_theorem_smt(&src).expect("must parse"),
            SmtVerdict::NotEntailed,
            "three boys lifting must NOT entail every boy lifted"
        );
    }
}
