use super::*;

#[test]
fn split_single_sentence() {
    assert_eq!(split_sentences("Req is valid."), vec!["Req is valid."]);
}

#[test]
fn split_two_sentences() {
    assert_eq!(
        split_sentences("Req is valid. Ack is valid."),
        vec!["Req is valid.", "Ack is valid."]
    );
}

#[test]
fn split_three_sentences() {
    assert_eq!(
        split_sentences("Req is valid. Ack is valid. Data is valid."),
        vec!["Req is valid.", "Ack is valid.", "Data is valid."]
    );
}

#[test]
fn split_empty() {
    assert!(split_sentences("").is_empty());
    assert!(split_sentences("   ").is_empty());
}

#[test]
fn split_no_period() {
    assert_eq!(split_sentences("Req is valid"), vec!["Req is valid"]);
}

#[test]
fn split_preserves_internal_periods() {
    // "e.g." should not be split
    assert_eq!(
        split_sentences("If req is valid. Then ack follows."),
        vec!["If req is valid.", "Then ack follows."]
    );
}
