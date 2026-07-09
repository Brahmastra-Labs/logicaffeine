// =============================================================================
// PHASE 64b: VP PARITY — quantified subjects get the FULL verb phrase grammar
// =============================================================================
// Under strict parsing, a quantified subject must support every VP frame a
// simple subject does, with the variable bound through the whole clause.
// These pin the exact semantic shapes recovered from former silent partial
// parses (reflexives, control, ditransitives, passives, reciprocals, donkey
// anaphora, adverbs).

use logicaffeine_language::{compile, compile_all_scopes};

fn assert_shape(sentence: &str, required: &[&str]) {
    let fol = compile(sentence).unwrap_or_else(|e| panic!("{sentence:?} failed: {e:?}"));
    for piece in required {
        assert!(
            fol.contains(piece),
            "{sentence:?} → {fol:?} is missing {piece:?}"
        );
    }
}

#[test]
fn reflexive_binds_the_quantified_variable() {
    assert_shape(
        "Every man loves himself.",
        &["∀x", "Agent(e, x)", "Theme(e, x)"],
    );
}

#[test]
fn control_infinitive_binds_into_the_complement() {
    assert_shape("Every child wants to play.", &["∀x", "W(x, Play(x))"]);
}

#[test]
fn temporal_anchor_survives_quantification() {
    assert_shape(
        "All students studied yesterday.",
        &["∀x", "Yesterday(", "Agent(e, x)"],
    );
}

#[test]
fn ditransitive_double_object_with_quantified_first_object() {
    assert_shape(
        "Every teacher gave some student a book.",
        &["∀x", "∃y", "Recipient(e, y)", "Theme(e, Book)"],
    );
}

#[test]
fn recipient_pp_with_quantified_recipient() {
    assert_shape(
        "Every student gave a book to some teacher.",
        &["∀x", "∃y", "∃z", "Theme(e, y)", "Recipient(e, z)"],
    );
}

#[test]
fn three_quantifiers_yield_multiple_scope_readings() {
    let readings = compile_all_scopes("Every student gave a book to some teacher.").unwrap();
    assert!(
        readings.len() >= 2,
        "expected multiple readings, got {}",
        readings.len()
    );
}

#[test]
fn object_control_distributes_over_quantified_object() {
    assert_shape(
        "Some rain caused all flowers to bloom.",
        &["∃x", "∀y", "Bloom(y)"],
    );
}

#[test]
fn object_control_with_definite_object() {
    assert_shape("Rain caused the flowers to bloom.", &["Bloom("]);
}

#[test]
fn progressive_with_adverb_under_quantifier() {
    assert_shape(
        "All dogs were running quickly.",
        &["∀x", "Quickly(e)", "Progressive(e)"],
    );
}

#[test]
fn progressive_with_adverb_simple_subject() {
    assert_shape("John was running quickly.", &["Prog(", "Q(e)"]);
}

#[test]
fn quantified_passive_with_quantified_agent() {
    assert_shape(
        "All books were read by some students.",
        &["∀x", "∃y", "Theme(e, x)", "Agent(e, y)"],
    );
}

#[test]
fn reciprocal_under_quantifier_is_pairwise() {
    assert_shape(
        "All students helped each other.",
        &["∀x", "∀y", "¬y = x", "Agent(e, x)", "Theme(e, y)"],
    );
}

#[test]
fn donkey_pronoun_binds_through_the_vp() {
    assert_shape(
        "Every man who owns a book gives it to a woman.",
        &["∀x", "∀y", "Theme(e, y)"],
    );
}

// Guards: simple-subject frames keep their canonical shapes.

#[test]
fn simple_ditransitive_double_object_unchanged() {
    assert_shape(
        "John gave Mary a book.",
        &["Recipient(e, Mary)", "Theme(e, Book)"],
    );
}

#[test]
fn simple_reciprocal_unchanged() {
    assert_shape(
        "Tom and Jerry helped each other.",
        &["Help(Tom, Jerry)", "Help(Jerry, Tom)"],
    );
}

#[test]
fn simple_passive_unchanged() {
    assert_shape("The book was read by John.", &["Read(John"]);
}

#[test]
fn simple_control_unchanged() {
    assert_shape("John decided to run.", &["D(J, Run(John))"]);
}

#[test]
fn simple_adverb_event_modifier_unchanged() {
    assert_shape("John ran quickly.", &["Quickly(e)"]);
}

// Lexical-ambiguity forest: per-token resolution, strict-filtered.

#[test]
fn duck_forest_yields_both_true_readings() {
    let readings = logicaffeine_language::compile_forest("I saw her duck.");
    assert!(
        readings.iter().any(|r| r.contains("[Duck(Her)]")),
        "perception small-clause reading: {readings:?}"
    );
    assert!(
        readings.iter().any(|r| r.contains("Theme(e, Duck)")),
        "possessed-bird object reading: {readings:?}"
    );
}

#[test]
fn time_flies_canonical_reading_survives() {
    let readings = logicaffeine_language::compile_forest("Time flies like an arrow.");
    assert!(
        readings
            .iter()
            .any(|r| r.contains("Fly(e)") && r.contains("Like(e, Arrow)")),
        "Time flies (V) like an arrow (PP): {readings:?}"
    );
}

#[test]
fn gerund_subject_predication() {
    let fol = compile("Flying planes can be dangerous.").unwrap();
    assert!(fol.contains("Fly"), "got {fol:?}");
    assert!(fol.contains("Dangerous"), "got {fol:?}");
    assert!(fol.contains("Planes"), "got {fol:?}");
}
