use super::*;

#[test]
fn test_lexicon_loads() {
    let index = LexiconIndex::new();
    assert!(!index.proper_nouns().is_empty());
    assert!(!index.common_nouns().is_empty());
    assert!(!index.intersective_adjectives().is_empty());
}

#[test]
fn from_json_matches_embedded_new() {
    // A caller-supplied document (the wasm app fetches the same bytes at runtime)
    // must build an index indistinguishable from the embedded-JSON constructor.
    let embedded = LexiconIndex::new();
    let parsed = LexiconIndex::from_json(LEXICON_JSON).expect("shipped lexicon.json parses");
    assert_eq!(embedded.proper_nouns().len(), parsed.proper_nouns().len());
    assert_eq!(embedded.common_nouns().len(), parsed.common_nouns().len());
    assert_eq!(
        embedded.transitive_verbs().len(),
        parsed.transitive_verbs().len()
    );
    assert_eq!(
        embedded.intransitive_verbs().len(),
        parsed.intransitive_verbs().len()
    );
    assert_eq!(
        embedded.intersective_adjectives().len(),
        parsed.intersective_adjectives().len()
    );
    // Malformed input is a proper error, never a panic.
    assert!(LexiconIndex::from_json("{").is_err());
}

#[test]
fn test_proper_nouns() {
    let index = LexiconIndex::new();
    let proper = index.proper_nouns();
    assert!(proper.iter().any(|n| n.lemma == "John"));
    assert!(proper.iter().any(|n| n.lemma == "Mary"));
}

#[test]
fn test_intersective_adjectives() {
    let index = LexiconIndex::new();
    let adj = index.intersective_adjectives();
    assert!(adj.iter().any(|a| a.lemma == "Happy"));
    assert!(adj.iter().any(|a| a.lemma == "Red"));
}

#[test]
fn test_pluralize() {
    let noun = NounEntry {
        lemma: "Dog".to_string(),
        forms: HashMap::new(),
        features: vec![],
        sort: None,
    };
    assert_eq!(pluralize(&noun), "dogs");

    let noun_irregular = NounEntry {
        lemma: "Man".to_string(),
        forms: [("plural".to_string(), "men".to_string())].into(),
        features: vec![],
        sort: None,
    };
    assert_eq!(pluralize(&noun_irregular), "men");
}

#[test]
fn test_present_3s() {
    let verb = VerbEntry {
        lemma: "Run".to_string(),
        class: "Activity".to_string(),
        forms: HashMap::new(),
        features: vec![],
    };
    assert_eq!(present_3s(&verb), "runs");
}
