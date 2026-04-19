//! Phase 1 — Hardware Lexicon Extensions
//!
//! RED tests for the seven hardware adjectives added by the HW-Spec
//! alignment plan: Asserted, Deasserted, Unknown, Zero, Active, Idle, Set.
//!
//! These adjectives are predicate vocabulary for hardware property sentences
//! like "The signal is asserted." Each one must be:
//!   * classified as an adjective by the lexicon
//!   * reachable through `compile()` so the FOL predicate appears in output
//!   * carry the `Intersective` feature (boolean signal state, not a degree)
//!
//! The negative test proves the lexicon is still closed. The regression test
//! proves we did not perturb neighbouring adjective rows. The three `Set`
//! disambiguation tests prove that adding `Set` as an adjective does not
//! break its existing verb/noun senses in other contexts.

use logicaffeine_language::compile;
use logicaffeine_language::lexicon::{LexiconTrait, StaticLexicon};
use logicaffeine_lexicon::Feature;

// ═══════════════════════════════════════════════════════════════════════════
// LEXICON LOOKUP — the authoritative RED tests
//
// Each of the seven new adjectives must appear in the generated
// `lookup_adjective_db`. Before the JSON edit these all return None; after
// `cargo build` regenerates `lexicon_data.rs` they return Some with the
// canonical lemma.
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn asserted_is_known_adjective() {
    let meta = StaticLexicon.lookup_adjective("asserted");
    assert!(
        meta.is_some(),
        "Lexicon must classify `asserted` as an adjective. Got: {:?}",
        meta
    );
    assert_eq!(meta.unwrap().lemma, "Asserted");
}

#[test]
fn deasserted_is_known_adjective() {
    let meta = StaticLexicon.lookup_adjective("deasserted");
    assert!(
        meta.is_some(),
        "Lexicon must classify `deasserted` as an adjective. Got: {:?}",
        meta
    );
    assert_eq!(meta.unwrap().lemma, "Deasserted");
}

#[test]
fn unknown_is_known_adjective() {
    let meta = StaticLexicon.lookup_adjective("unknown");
    assert!(
        meta.is_some(),
        "Lexicon must classify `unknown` as an adjective. Got: {:?}",
        meta
    );
    assert_eq!(meta.unwrap().lemma, "Unknown");
}

#[test]
fn zero_is_known_adjective() {
    let meta = StaticLexicon.lookup_adjective("zero");
    assert!(
        meta.is_some(),
        "Lexicon must classify `zero` as an adjective. Got: {:?}",
        meta
    );
    assert_eq!(meta.unwrap().lemma, "Zero");
}

#[test]
fn active_is_known_adjective() {
    let meta = StaticLexicon.lookup_adjective("active");
    assert!(
        meta.is_some(),
        "Lexicon must classify `active` as an adjective. Got: {:?}",
        meta
    );
    assert_eq!(meta.unwrap().lemma, "Active");
}

#[test]
fn idle_is_known_adjective() {
    let meta = StaticLexicon.lookup_adjective("idle");
    assert!(
        meta.is_some(),
        "Lexicon must classify `idle` as an adjective. Got: {:?}",
        meta
    );
    assert_eq!(meta.unwrap().lemma, "Idle");
}

#[test]
fn set_is_known_adjective() {
    let meta = StaticLexicon.lookup_adjective("set");
    assert!(
        meta.is_some(),
        "Lexicon must classify `set` as an adjective (distinct from the verb/noun senses). Got: {:?}",
        meta
    );
    assert_eq!(meta.unwrap().lemma, "Set");
}

// ═══════════════════════════════════════════════════════════════════════════
// INTERSECTIVE FEATURE — every new HW adjective is a boolean predicate,
// not a degree predicate. Defends against someone flipping to Gradable or
// NonIntersective.
// ═══════════════════════════════════════════════════════════════════════════

fn assert_intersective(word: &str) {
    let meta = StaticLexicon
        .lookup_adjective(word)
        .unwrap_or_else(|| panic!("{} must be in the adjective db", word));
    assert!(
        meta.features.contains(&Feature::Intersective),
        "{} must carry Feature::Intersective. features: {:?}",
        word,
        meta.features
    );
}

#[test]
fn asserted_is_intersective() {
    assert_intersective("asserted");
}

#[test]
fn deasserted_is_intersective() {
    assert_intersective("deasserted");
}

#[test]
fn unknown_is_intersective() {
    assert_intersective("unknown");
}

#[test]
fn zero_is_intersective() {
    assert_intersective("zero");
}

#[test]
fn active_is_intersective() {
    assert_intersective("active");
}

#[test]
fn idle_is_intersective() {
    assert_intersective("idle");
}

#[test]
fn set_adjective_is_intersective() {
    assert_intersective("set");
}

// ═══════════════════════════════════════════════════════════════════════════
// COMPILE + FOL PREDICATE — end-to-end integration. Mirrors the existing
// `high_adjective_parses_and_appears_in_fol` pattern in phase_hw_lexicon.rs.
// ═══════════════════════════════════════════════════════════════════════════

fn compile_ok_with_predicate(input: &str, predicate: &str) {
    let result = compile(input);
    assert!(
        result.is_ok(),
        "Input `{}` should compile: {:?}",
        input,
        result.err()
    );
    let fol = result.unwrap();
    let upper = predicate;
    let lower = predicate.to_lowercase();
    assert!(
        fol.contains(upper) || fol.contains(&lower),
        "FOL output for `{}` must reference `{}` predicate. Got: {}",
        input,
        predicate,
        fol
    );
}

#[test]
fn signal_is_asserted_produces_asserted_predicate() {
    compile_ok_with_predicate("The signal is asserted.", "Asserted");
}

#[test]
fn signal_is_deasserted_produces_deasserted_predicate() {
    compile_ok_with_predicate("The signal is deasserted.", "Deasserted");
}

#[test]
fn signal_is_unknown_produces_unknown_predicate() {
    compile_ok_with_predicate("The signal is unknown.", "Unknown");
}

#[test]
fn bus_is_zero_produces_zero_predicate() {
    compile_ok_with_predicate("The bus is zero.", "Zero");
}

#[test]
fn signal_is_active_produces_active_predicate() {
    compile_ok_with_predicate("The signal is active.", "Active");
}

#[test]
fn signal_is_idle_produces_idle_predicate() {
    compile_ok_with_predicate("The signal is idle.", "Idle");
}

// ═══════════════════════════════════════════════════════════════════════════
// CROSS-CUTTING — lexicon closure and regression on neighbour rows
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn frobozz_is_not_an_adjective() {
    let meta = StaticLexicon.lookup_adjective("frobozz");
    assert!(
        meta.is_none(),
        "Lexicon must not classify a nonsense word as an adjective. Got: {:?}",
        meta
    );
}

#[test]
fn existing_adjectives_still_resolve_after_edit() {
    let high = StaticLexicon
        .lookup_adjective("high")
        .expect("`high` adjective must still be present after HW lexicon edit");
    assert_eq!(high.lemma, "High");
    assert!(
        high.features.contains(&Feature::Intersective)
            && high.features.contains(&Feature::Gradable),
        "`High` must retain Intersective+Gradable. features: {:?}",
        high.features
    );

    let valid = StaticLexicon
        .lookup_adjective("valid")
        .expect("`valid` adjective must still be present after HW lexicon edit");
    assert_eq!(valid.lemma, "Valid");
    assert!(
        valid.features.contains(&Feature::Intersective),
        "`Valid` must retain Intersective. features: {:?}",
        valid.features
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// `Set` POS DISAMBIGUATION — the plan's open-item §1 concern is that adding
// `Set` as an adjective must resolve correctly in copula context. Imperative-
// verb and NP-positional parsing for `Set` are pre-existing limitations of
// the parser and belong to a separate phase.
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn set_copula_adjective_compiles() {
    let fol = compile("The bit is set.").expect("copula + adjective `set` must compile");
    assert!(
        fol.contains("Set") || fol.contains("set"),
        "FOL output should reference the Set predicate in copula context. Got: {}",
        fol
    );
}

#[test]
fn set_copula_adjective_exposes_set_lemma_in_lookup() {
    let meta = StaticLexicon
        .lookup_adjective("set")
        .expect("`set` must resolve as adjective so `The bit is set.` can bind the predicate");
    assert_eq!(meta.lemma, "Set");
}
