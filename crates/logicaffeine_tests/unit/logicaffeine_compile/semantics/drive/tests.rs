use super::*;

#[test]
fn an_answer_is_a_failure_exactly_when_it_says_so() {
    let ev = Verb::Click.events();
    assert_eq!(answer_failure(Answer::Answered, ev, None, "#b", true), None);
    assert_eq!(
        answer_failure(Answer::Unanswered, ev, None, "#b", false),
        None
    );
    assert_eq!(
        answer_failure(Answer::Unanswered, ev, None, "#b", true).as_deref(),
        Some(nothing_answers(ev, None, "#b").as_str())
    );
    // A missing element is a failure whether or not an answer was required: a raise that
    // may go unanswered still needs something to raise it on.
    assert_eq!(
        answer_failure(Answer::NoElement, ev, None, "#b", false).as_deref(),
        Some(no_element("#b").as_str())
    );
}

#[test]
fn a_press_names_its_key_and_the_other_verbs_name_their_events() {
    let press = nothing_answers(Verb::Press.events(), Some("Enter"), "#d");
    assert!(
        press.starts_with("Nothing answers a Enter press on \"#d\"."),
        "{press}"
    );
    assert!(press.contains("`on-keydown={action}`"), "{press}");
    let typed = nothing_answers(Verb::Type.events(), None, "#d");
    assert!(
        typed.starts_with("Nothing answers a input or change on \"#d\"."),
        "{typed}"
    );
    assert!(typed.contains("`on-input={action}`"), "{typed}");
}

#[test]
fn a_selector_is_quoted_with_its_escapes() {
    assert!(no_element("a\"b").starts_with("No element matches \"a\\\"b\"."));
}

#[test]
fn every_verb_raises_at_least_one_event() {
    for v in Verb::ALL {
        assert!(!v.events().is_empty(), "{v:?}");
    }
}
