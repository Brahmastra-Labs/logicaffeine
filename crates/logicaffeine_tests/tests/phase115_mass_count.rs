//! Phase 115 — §6.2 Mass vs count semantics (work/MISSING_ENGLISH.md).
//!
//! Mass nouns denote cumulative, non-atomic stuff; count nouns have atoms.
//!   "Water is wet."        → kind predication (no ∃ individual)
//!   "John drank water."    → Theme = the stuff (mass, no ∃ atom)
//!   "John drank a water."  → ∃x(Portion(x) ∧ Water(x) ∧ …)   (count coercion)
//!
//! The cumulativity/divisiveness Link-lattice axioms (Water(a)∧Water(b)→Water(a⊕b))
//! are the deeper P5 reasoning layer; here we lock the parse-level distinction:
//! mass vs count compile differently, and a count determiner coerces a portion.

use logicaffeine_language::compile;

#[test]
fn mass_count_use_coerces_to_portion() {
    let out = compile("John drank a water.").unwrap();
    eprintln!("a-water: {out}");
    assert!(out.contains("Portion("), "count use of a mass noun coerces a portion: {out}");
    assert!(out.contains("Water("), "the stuff predicate: {out}");
    assert!(out.contains('∃'), "the portion is an introduced atom: {out}");
}

#[test]
fn mass_count_numeral_coerces_to_portions() {
    let out = compile("John drank two waters.").unwrap();
    eprintln!("two-waters: {out}");
    assert!(out.contains("Portion("), "a numeral on a mass noun counts portions: {out}");
    assert!(out.contains("Water("), "the stuff predicate: {out}");
}

#[test]
fn bare_mass_is_not_a_counted_individual() {
    let out = compile("John drank water.").unwrap();
    eprintln!("bare-water: {out}");
    // Bare mass: the stuff itself, no Portion atom and no ∃x(Water(x)) quantification.
    assert!(!out.contains("Portion("), "bare mass is not portioned: {out}");
    assert!(!out.contains("∃x(Water"), "bare mass is not a counted individual: {out}");
}

#[test]
fn mass_kind_predication() {
    let out = compile("Water is wet.").unwrap();
    eprintln!("water-wet: {out}");
    assert!(out.contains("Wet") && out.contains("Water"), "kind predication over the stuff: {out}");
    assert!(!out.contains("Portion("), "kind predication introduces no portion: {out}");
}

#[test]
fn count_noun_unaffected() {
    // Regression: a genuine count noun must NOT be coerced to a portion.
    let out = compile("John saw a dog.").unwrap();
    eprintln!("a-dog: {out}");
    assert!(!out.contains("Portion("), "count nouns are not portioned: {out}");
    assert!(out.contains("Dog("), "the count noun: {out}");
}

// ============================================================================
// Entailment spec (§6.2): portions are parts of the substance — drinking a
// water is drinking water; the encoding must survive the Z3 translation.
// ============================================================================
#[cfg(feature = "verification")]
mod verification_spec {
    use logicaffeine_compile::{check_theorem_premises_consistent, check_theorem_smt};
    use logicaffeine_proof::oracle::{SmtConsistency, SmtVerdict};

    fn theorem(premises: &[&str], goal: &str) -> String {
        let givens: String = premises.iter().map(|p| format!("Given: {p}\n")).collect();
        format!("## Theorem: Phase115V\n{givens}Prove: {goal}\nProof: Auto.\n")
    }

    #[test]
    fn portion_premise_is_consistent_and_self_entailing() {
        let src = theorem(&["John drank a water."], "John drank a water.");
        assert_eq!(
            check_theorem_premises_consistent(&src).expect("must parse"),
            SmtConsistency::Consistent,
            "the portion-coerced premise is consistent"
        );
        assert_eq!(
            check_theorem_smt(&src).expect("must parse"),
            SmtVerdict::Entailed,
            "identity through the portion encoding"
        );
    }

    #[test]
    fn portion_of_water_is_not_a_portion_for_someone_else() {
        let src = theorem(&["John drank a water."], "Mary drank a water.");
        assert_eq!(
            check_theorem_smt(&src).expect("must parse"),
            SmtVerdict::NotEntailed,
            "sanity: the portion encoding must not over-entail"
        );
    }
}
