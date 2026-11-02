//! A prepositional phrase that modifies an event is a condition on THAT event: "John sees Mary
//! in the park" is `∃e(See(e) ∧ Agent(e, John) ∧ Theme(e, Mary) ∧ In(e, Park))`. Conjoined
//! outside the event's own `∃e`, `In(e, Park)` is about no event at all — `e` is free — and the
//! Studio lists it as a sentence of its own.

use logicaffeine_compile::{compile_for_ui, compile_theorem_for_ui};

fn theorem(name: &str, givens: &[&str], goal: &str) -> logicaffeine_compile::TheoremCompileResult {
    let mut source = format!("## Theorem: {name}\n");
    for given in givens {
        source.push_str(&format!("Given: {given}\n"));
    }
    source.push_str(&format!("Prove: {goal}\nProof: Auto.\n"));
    compile_theorem_for_ui(&source)
}

fn certifies(name: &str, givens: &[&str], goal: &str) {
    let result = theorem(name, givens, goal);
    assert!(result.error.is_none(), "{name} did not compile: {:?}", result.error);
    assert!(
        result.verified,
        "{name}: from {givens:?} the prover does not certify {goal:?} ({:?}); premises {:?}",
        result.verification_error, result.premises
    );
}

fn shown(sentence: &str) -> String {
    let result = compile_for_ui(sentence);
    assert!(result.error.is_none(), "{sentence} did not parse: {:?}", result.error);
    result.logic.unwrap_or_default()
}

/// Every `In(e, …)`-style condition in `logic` sits inside a quantifier over `e`: scanning left
/// to right, an `(e,` argument only ever appears while some `∃e(`/`∀e(` is still open.
fn every_event_condition_is_bound(logic: &str) -> bool {
    let mut open: Vec<bool> = Vec::new();
    let chars: Vec<char> = logic.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let rest: String = chars[i..].iter().take(3).collect();
        if rest == "∃e(" || rest == "∀e(" {
            open.push(true);
            i += 3;
            continue;
        }
        match chars[i] {
            '(' => open.push(false),
            ')' => {
                open.pop();
            }
            _ => {}
        }
        let arg: String = chars[i..].iter().take(3).collect();
        if (arg == "(e," || arg == "(e)") && !open.iter().any(|&binds| binds) {
            return false;
        }
        i += 1;
    }
    true
}

#[test]
fn the_park_is_where_the_seeing_happens() {
    let logic = shown("John sees Mary in the park.");
    assert!(logic.contains("In(e, Park)"), "{logic}");
    assert!(!logic.contains("\n2)"), "the place is part of the sentence, not a sentence of its own: {logic}");
    assert!(every_event_condition_is_bound(&logic), "a free event variable: {logic}");
}

#[test]
fn every_subject_keeps_its_place_inside_the_event() {
    for sentence in ["Every man sees Mary in the park.", "A man sees Mary in the park.", "The dog sleeps in the house."] {
        let logic = shown(sentence);
        assert!(every_event_condition_is_bound(&logic), "{sentence}: a free event variable: {logic}");
    }
}

#[test]
fn a_seeing_in_the_park_is_a_seeing() {
    certifies("Sees_Mary", &["John sees Mary in the park."], "John sees Mary.");
    certifies("Sees_Mary_There", &["John sees Mary in the park."], "John sees Mary in the park.");
}

#[test]
fn the_checker_itself_sees_a_free_event() {
    assert!(!every_event_condition_is_bound("1) ∃e(See(e) ∧ Agent(e, John))\n2) In(e, Park)"));
    assert!(every_event_condition_is_bound("∃e(See(e) ∧ Agent(e, John) ∧ In(e, Park))"));
    assert!(every_event_condition_is_bound("∀x((Man(x) → ∃e((See(e) ∧ In(e, Park)))))"));
}
