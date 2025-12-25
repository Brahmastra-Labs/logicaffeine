use logos::compile;
use logos::lexicon::Sort;

#[test]
fn sort_lookup_human() {
    assert_eq!(logos::lexicon::lookup_sort("Juliet"), Some(Sort::Human));
    assert_eq!(logos::lexicon::lookup_sort("John"), Some(Sort::Human));
    assert_eq!(logos::lexicon::lookup_sort("Mary"), Some(Sort::Human));
}

#[test]
fn sort_lookup_celestial() {
    assert_eq!(logos::lexicon::lookup_sort("Sun"), Some(Sort::Celestial));
    assert_eq!(logos::lexicon::lookup_sort("Moon"), Some(Sort::Celestial));
}

#[test]
fn sort_lookup_abstract() {
    assert_eq!(logos::lexicon::lookup_sort("Time"), Some(Sort::Abstract));
    assert_eq!(logos::lexicon::lookup_sort("Justice"), Some(Sort::Abstract));
}

#[test]
fn sort_lookup_physical() {
    assert_eq!(logos::lexicon::lookup_sort("Rock"), Some(Sort::Physical));
    assert_eq!(logos::lexicon::lookup_sort("Money"), Some(Sort::Value));
}

#[test]
fn sort_compatibility() {
    assert!(Sort::Human.is_compatible_with(Sort::Animate));
    assert!(Sort::Human.is_compatible_with(Sort::Human));
    assert!(!Sort::Human.is_compatible_with(Sort::Celestial));
    assert!(!Sort::Celestial.is_compatible_with(Sort::Abstract));
}

#[test]
fn literal_copula_preserved() {
    let output = compile("The king is bald.").unwrap();
    assert!(output.contains("B(") || output.contains("Bald"),
        "Literal copula should produce predication. Output: {}", output);
    assert!(output.contains("K(") || output.contains("King"),
        "Subject should be present. Output: {}", output);
    assert!(!output.contains("Metaphor"),
        "Literal should NOT be a metaphor. Output: {}", output);
}
