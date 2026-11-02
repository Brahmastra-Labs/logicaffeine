use super::*;

#[test]
fn levenshtein_identical() {
    assert_eq!(levenshtein("hello", "hello"), 0);
}

#[test]
fn levenshtein_one_char_diff() {
    assert_eq!(levenshtein("hello", "hallo"), 1);
}

#[test]
fn levenshtein_insertion() {
    assert_eq!(levenshtein("hello", "helllo"), 1);
}

#[test]
fn levenshtein_deletion() {
    assert_eq!(levenshtein("hello", "helo"), 1);
}

#[test]
fn levenshtein_empty() {
    assert_eq!(levenshtein("", "abc"), 3);
    assert_eq!(levenshtein("abc", ""), 3);
}

#[test]
fn levenshtein_transposition() {
    assert_eq!(levenshtein("ab", "ba"), 2);
}

#[test]
fn find_similar_typo() {
    let result = find_similar("logoc", KNOWN_WORDS, 2);
    assert_eq!(result, Some("logic"));
}

#[test]
fn find_similar_no_match() {
    let result = find_similar("xyzzy", KNOWN_WORDS, 2);
    assert_eq!(result, None);
}

#[test]
fn did_you_mean_finds_the_nearest_within_a_third_of_the_word() {
    let names = ["Counter", "Card", "count", "counter"];
    assert_eq!(
        did_you_mean("Countr", names),
        Some("Counter"),
        "one deletion"
    );
    assert_eq!(did_you_mean("prevnt", ["prevent", "stop"]), Some("prevent"));
    assert_eq!(did_you_mean("nme", ["name", "title"]), Some("name"));
    assert_eq!(did_you_mean("xyzzy", names), None, "nothing is near");
    assert_eq!(
        did_you_mean("Counter", ["Counter"]),
        None,
        "a word does not suggest itself"
    );
    assert_eq!(
        did_you_mean("ab", ["xy"]),
        None,
        "two edits exceed a two-letter word's budget"
    );
    assert_eq!(
        did_you_mean("todo", ["todos", "tode"]),
        Some("todos"),
        "the first of equals wins"
    );
    assert_eq!(
        did_you_mean("COUNTR", ["Counter"]),
        Some("Counter"),
        "case is not an edit"
    );
    assert_eq!(
        did_you_mean("doen", ["done", "label"]),
        Some("done"),
        "a swap is one edit"
    );
}

#[test]
fn typo_distance_counts_a_swap_once() {
    assert_eq!(typo_distance("doen", "done"), 1);
    assert_eq!(typo_distance("ab", "ba"), 1);
    assert_eq!(typo_distance("hello", "hallo"), 1);
    assert_eq!(typo_distance("", "abc"), 3);
    assert_eq!(typo_distance("abc", "abc"), 0);
    assert_eq!(
        typo_distance("ca", "abc"),
        3,
        "optimal string alignment edits each substring once"
    );
}

#[test]
fn find_similar_case_insensitive() {
    let result = find_similar("LOGIC", KNOWN_WORDS, 2);
    assert_eq!(result, Some("logic"));
}
