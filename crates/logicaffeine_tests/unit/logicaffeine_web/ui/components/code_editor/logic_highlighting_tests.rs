use super::{tokenize, Language, TokenKind};

fn kind_of(source: &str, word: &str) -> Option<TokenKind> {
    tokenize(source, Language::Logic).into_iter().find(|t| t.text == word).map(|t| t.kind)
}

#[test]
fn a_theorem_highlights_its_proof_structure() {
    let theorem = "## Theorem: Socrates_Mortality\nGiven: All men are mortal.\nProve: Socrates is mortal.\nProof: Auto.";
    for word in ["Theorem", "Given", "Prove", "Proof", "Auto"] {
        assert!(kind_of(theorem, word) == Some(TokenKind::Keyword), "{word} is not a keyword in {theorem:?}");
    }
    assert!(kind_of(theorem, "Socrates") == Some(TokenKind::Identifier), "a name is not a keyword");
}

#[test]
fn quantifiers_stand_out() {
    for (sentence, word) in [("Every cat sleeps.", "Every"), ("No student failed.", "No"), ("Exactly one trip is in 2003.", "Exactly"), ("Few birds can fly.", "Few")] {
        assert!(kind_of(sentence, word) == Some(TokenKind::Builtin), "{word} is not marked as a quantifier in {sentence:?}");
    }
}

#[test]
fn connectives_read_as_operators() {
    let sentence = "If Alice is happy then Alice is dancing and Bob is not sad or tired.";
    for word in ["If", "then", "and", "not", "or"] {
        assert!(kind_of(sentence, word) == Some(TokenKind::Operator), "{word} is not a connective in {sentence:?}");
    }
}

#[test]
fn modals_are_marked() {
    for (sentence, word) in [("All students must study.", "must"), ("Few birds can fly.", "can"), ("Some birds cannot fly.", "cannot")] {
        assert!(kind_of(sentence, word) == Some(TokenKind::Type), "{word} is not marked as a modal in {sentence:?}");
    }
}

#[test]
fn a_title_line_is_a_comment() {
    let tokens = tokenize("# Simple Sentences\nEvery cat sleeps.", Language::Logic);
    assert!(tokens.iter().any(|t| t.text == "# Simple Sentences" && t.kind == TokenKind::Comment), "the title is not a comment");
    assert!(tokens.iter().any(|t| t.text == "Every" && t.kind == TokenKind::Builtin), "the sentence after the title lost its highlighting");
}

#[test]
fn a_block_header_is_not_a_comment() {
    let tokens = tokenize("## Theorem: By_Hand\nProve: Bob is tall.", Language::Logic);
    assert!(!tokens.iter().any(|t| t.kind == TokenKind::Comment), "## Theorem is a header, not a comment");
}

/// Tactics are the engine's own vocabulary, synonyms and all — but only in a proof, where
/// `both`, `left` and `use` are steps; in a sentence they are just English.
#[test]
fn tactics_are_keywords_only_in_a_proof() {
    let proof = "Proof: cases h. split. assumption. suppose x. trivially.";
    for word in ["cases", "split", "assumption", "suppose", "trivially"] {
        assert!(kind_of(proof, word) == Some(TokenKind::Keyword), "{word} is not a tactic in {proof:?}");
    }
    let sentence = "Given: Both the left door and the right door are open.";
    for word in ["Both", "left", "right"] {
        assert!(kind_of(sentence, word) != Some(TokenKind::Keyword), "{word} is a tactic outside a proof in {sentence:?}");
    }
}

/// A counting quantifier is one phrase — "at least three", "at most 2", "exactly one",
/// "more than two", "fewer than 3" — the way the lexer reads it as one quantifier.
#[test]
fn a_counting_quantifier_is_one_phrase() {
    for (sentence, phrase) in [
        ("At least three students passed.", ["At", "least", "three"]),
        ("at most 2 cats purr.", ["at", "most", "2"]),
        ("Exactly one trip is in 2003.", ["Exactly", "one", ""]),
        ("More than two dogs bark.", ["More", "than", "two"]),
        ("Fewer than 3 birds fly.", ["Fewer", "than", "3"]),
    ] {
        for word in phrase.into_iter().filter(|w| !w.is_empty()) {
            assert!(kind_of(sentence, word) == Some(TokenKind::Builtin), "{word} is not part of the quantifier in {sentence:?}");
        }
    }
    assert!(kind_of("In 2003 the trip ended.", "2003") == Some(TokenKind::Number), "a number that counts nothing stays a number");
    assert!(kind_of("John is at the park.", "at") != Some(TokenKind::Builtin), "at without least or most is a preposition");
}

#[test]
fn a_proof_ends_with_its_line() {
    let source = "Proof: split.\nGiven: The left door is open.";
    assert!(kind_of(source, "left") != Some(TokenKind::Keyword), "the proof ran past its line");
}
