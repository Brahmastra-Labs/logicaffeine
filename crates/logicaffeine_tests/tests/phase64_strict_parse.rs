// =============================================================================
// PHASE 64: STRICT PARSING - TEST SUITE
// =============================================================================
// TDD RED: compile() must consume the ENTIRE input. A parse that stops
// mid-sentence and silently drops the remainder is a lie: it reports success
// while discarding meaning. The spec: every sentence either parses fully
// (its content words appear in the FOL) or returns an error.
//
// Discovered degenerate parses (pre-strictness):
//   "Francisco isn't the dentist."  → Ok("Francisco")   (everything dropped)
//   "Mathew's show aired on April 27th." → Ok("Show")
//   "Jon's episode aired before the episode filmed in Vanuatu." → Ok("Episode")

use logicaffeine_language::compile;

/// The strict-parse contract: full semantic content or an error — never a
/// silent partial parse. Each required word must appear in the FOL if the
/// sentence compiles at all.
fn assert_full_or_error(sentence: &str, required: &[&str]) {
    match compile(sentence) {
        Ok(fol) => {
            for word in required {
                assert!(
                    fol.contains(word),
                    "Silent partial parse of {sentence:?}: FOL {fol:?} is missing {word:?}"
                );
            }
        }
        Err(_) => {}
    }
}

// =============================================================================
// NO SILENT PARTIAL PARSES
// =============================================================================

#[test]
fn negative_contraction_is_not_swallowed() {
    assert_full_or_error("Francisco isn't the dentist.", &["Dentist"]);
}

#[test]
fn possessive_subject_clause_is_not_swallowed() {
    assert_full_or_error("Mathew's show aired on April 27th.", &["Air"]);
}

#[test]
fn bare_temporal_comparison_is_not_swallowed() {
    assert_full_or_error(
        "Jon's episode aired before the episode filmed in Vanuatu.",
        &["Vanuatu"],
    );
}

#[test]
fn trailing_preposition_is_rejected() {
    let result = compile("John runs of.");
    assert!(
        result.is_err(),
        "Expected an error for trailing junk, got {result:?}"
    );
}

// =============================================================================
// GUARDS: FULLY-PARSING SENTENCES ARE UNAFFECTED BY STRICTNESS
// =============================================================================

#[test]
fn simple_transitive_still_compiles() {
    let fol = compile("John loves Mary.").unwrap();
    assert!(fol.contains("Love"), "got {fol:?}");
}

#[test]
fn universal_still_compiles() {
    let fol = compile("All men are mortal.").unwrap();
    assert!(fol.contains("∀"), "got {fol:?}");
    assert!(fol.contains("Mortal"), "got {fol:?}");
}

#[test]
fn copular_identity_still_compiles() {
    let fol = compile("Socrates is a man.").unwrap();
    assert!(fol.contains("Man"), "got {fol:?}");
}

#[test]
fn wh_question_terminator_still_compiles() {
    let fol = compile("Who did John see?").unwrap();
    assert!(fol.contains("See"), "got {fol:?}");
}

#[test]
fn multi_sentence_discourse_still_compiles() {
    let fol = compile("John runs. Mary walks.").unwrap();
    assert!(fol.contains("Run"), "got {fol:?}");
    assert!(fol.contains("Walk"), "got {fol:?}");
}

#[test]
fn exclamation_terminator_still_compiles() {
    let fol = compile("John runs!").unwrap();
    assert!(fol.contains("Run"), "got {fol:?}");
}

// =============================================================================
// NEGATIVE COPULA/AUXILIARY CONTRACTIONS (A1)
// =============================================================================
// Each contraction expands to its base + "not", so the existing negation
// grammar carries the meaning: "isn't" ≡ "is not".

#[test]
fn isnt_negates_copular_predication() {
    let fol = compile("Bill isn't a man.").unwrap();
    assert!(fol.contains('¬'), "got {fol:?}");
    assert!(fol.contains("Man"), "got {fol:?}");
}

#[test]
fn wasnt_negates_past_predication() {
    let fol = compile("Bill wasn't happy.").unwrap();
    assert!(fol.contains('¬'), "got {fol:?}");
    assert!(fol.contains("Happy"), "got {fol:?}");
}

#[test]
fn arent_negates_plural_predication() {
    let fol = compile("Dogs aren't cats.").unwrap();
    assert!(fol.contains('¬'), "got {fol:?}");
}

#[test]
fn werent_negates_plural_past() {
    let fol = compile("The dogs weren't happy.").unwrap();
    assert!(fol.contains('¬'), "got {fol:?}");
    assert!(fol.contains("Happy"), "got {fol:?}");
}

#[test]
fn wouldnt_negates_modal() {
    let fol = compile("Bill wouldn't run.").unwrap();
    assert!(fol.contains('¬'), "got {fol:?}");
    assert!(fol.contains("Run"), "got {fol:?}");
}

#[test]
fn didnt_contraction_still_works() {
    let fol = compile("Bill didn't run.").unwrap();
    assert!(fol.contains('¬'), "got {fol:?}");
    assert!(fol.contains("Run"), "got {fol:?}");
}
