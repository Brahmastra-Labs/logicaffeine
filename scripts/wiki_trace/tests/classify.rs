//! Classifier regression tests — the IP of the triage harness.
//!
//! Each test runs a sentence through the genuine LOGOS compiler via `classify`
//! and pins the bucket it must land in. These guard the guardrails: function
//! words are never proposed as lexicon entries, abbreviations are isolated (not
//! "fixed"), and a fronted PP is recognized as a parser gap whose spec is written
//! by its trailing paraphrase.

use wiki_trace::{classify, quarantine, Category, Gate, IsolateKind};

fn has(words: &[String], w: &str) -> bool {
    words.iter().any(|x| x.eq_ignore_ascii_case(w))
}

#[test]
fn clean_sentence_is_clean_and_not_auto() {
    let r = classify(1, "Every philosopher is wise.");
    assert_eq!(r.category, Category::Clean, "got {:?} / fol {:?}", r.category, r.fol);
    assert_ne!(r.gate, Gate::Auto, "clean sentences are never an auto work item");
}

#[test]
fn fronted_pp_is_parser_gap_with_self_written_oracle() {
    // The bug we found: a sentence-initial PP fails, while its trailing twin parses.
    let r = classify(1, "After the meeting, Mary left.");
    assert_eq!(r.category, Category::ParserGap, "fronted PP should be a parser gap");
    assert_eq!(r.subsystem, wiki_trace::Subsystem::Parser);
    let oracle = r.oracle.expect("the trailing paraphrase parses → oracle must be derived");
    assert_eq!(oracle.transform, "pp_fronting_to_trailing");
    assert_eq!(oracle.variant_sentence, "Mary left after the meeting.");
    assert!(!oracle.expected_fol.is_empty(), "oracle must carry the expected FOL");
    assert!(
        r.proposal.red_test.is_some(),
        "a parser gap with an oracle must propose a RED test"
    );
}

#[test]
fn lexicon_gap_flags_unknown_word_but_never_a_function_word() {
    // "blorptastic" is unknown (lexer falls back to Adjective); "is"/"the"/"cat"
    // are known/closed-class and must NOT be proposed as lexicon entries.
    let r = classify(1, "The cat is blorptastic.");
    assert_eq!(r.category, Category::ActionableLexiconGap);
    assert_eq!(r.gate, Gate::Auto, "a lexicon gap with a proposal is auto-eligible");
    let suspect = &r.localization.suspect_words;
    assert!(has(suspect, "blorptastic"), "unknown word must be flagged: {suspect:?}");
    for fw in ["is", "the", "cat"] {
        assert!(!has(suspect, fw), "must never flag {fw:?} as a gap: {suspect:?}");
    }
    assert!(r.proposal.lexicon_entry.is_some(), "must propose a candidate entry");
}

#[test]
fn abbreviation_is_isolated_not_a_lexicon_gap() {
    let r = classify(1, "He earned a PhD.");
    let iso = &r.localization.isolated_spans;
    assert!(
        iso.iter().any(|s| s.text == "PhD" && s.kind == IsolateKind::Abbreviation),
        "PhD must be isolated as an abbreviation: {iso:?}"
    );
    assert!(
        !has(&r.localization.suspect_words, "PhD"),
        "an abbreviation must never be proposed as a lexicon entry"
    );
}

#[test]
fn quarantine_detects_quotes_parens_and_brackets() {
    let q = quarantine("He said \"hello there\" loudly.");
    assert!(q.iter().any(|s| s.kind == IsolateKind::Quote), "quote span: {q:?}");

    let q = quarantine("He was born (1989) in Paris.");
    assert!(q.iter().any(|s| s.kind == IsolateKind::Parenthetical), "paren span: {q:?}");

    let q = quarantine("The result holds [1] in general.");
    assert!(q.iter().any(|s| s.kind == IsolateKind::Citation), "citation span: {q:?}");

    // Acronyms and slashed compounds.
    let q = quarantine("It uses EEG and ERPs.");
    assert!(
        q.iter().filter(|s| s.kind == IsolateKind::Abbreviation).count() >= 2,
        "EEG and ERPs must both be isolated: {q:?}"
    );
}

#[test]
fn ordinary_capitalized_word_is_not_an_abbreviation() {
    // A normal proper noun (first-letter cap only) must not be quarantined.
    let q = quarantine("Mary visited Paris.");
    assert!(
        !q.iter().any(|s| s.kind == IsolateKind::Abbreviation),
        "Mary/Paris are names, not abbreviations: {q:?}"
    );
}
