//! The symbol pad beside every typed answer on /learn holds everything a formula needs, and nothing
//! the grader cannot read.
//!
//! Both directions are held, so the pad and the curriculum can never drift apart:
//!
//!   * every symbol a typed key needs is a button: each lesson's typed key, and every key the
//!     practice exercises generate, written the way a learner writes it (the strength LOGOS prints
//!     after a modal, `◇_{0.5}`, left out, as the grader ignores it) uses only letters, digits,
//!     spaces and the pad's symbols;
//!   * every button's symbol is read by the grader: a spelling that only a parser can equate with a
//!     formula using the symbol grades right, and a formula that differs only around the symbol
//!     grades wrong.

#![cfg(feature = "web-tests")]

use logicaffeine_web::composer::symbol_buttons;
use logicaffeine_web::content::{ContentEngine, ExerciseType, LessonQuestion};
use logicaffeine_web::generator::{AnswerType, Generator};
use logicaffeine_web::grader::check_answer;
use rand::rngs::StdRng;
use rand::SeedableRng;
use std::collections::BTreeSet;

/// For each symbol: a formula that uses it, a spelling of the same formula the grader can only
/// equate by parsing it, and a formula the grader can only tell apart from it by reading the symbol.
const READINGS: &[(&str, &str, &str, &str)] = &[
    ("∀", "∀x(Man(x) → Mortal(x))", "∀y(Man(y) → Mortal(y))", "∃x(Man(x) → Mortal(x))"),
    ("∃", "∃x(Man(x) ∧ Happy(x))", "∃z(Happy(z) ∧ Man(z))", "∀x(Man(x) ∧ Happy(x))"),
    ("¬", "¬Happy(John)", "¬(Happy(John))", "Happy(John)"),
    ("∧", "Happy(John) ∧ Sad(Mary)", "Sad(Mary) ∧ Happy(John)", "Happy(John) ∨ Sad(Mary)"),
    ("∨", "Happy(John) ∨ Sad(Mary)", "Sad(Mary) ∨ Happy(John)", "Happy(John) ∧ Sad(Mary)"),
    ("→", "Happy(John) → Sad(Mary)", "(Happy(John) → Sad(Mary))", "Sad(Mary) → Happy(John)"),
    ("↔", "Happy(John) ↔ Sad(Mary)", "Sad(Mary) ↔ Happy(John)", "Happy(John) → Sad(Mary)"),
    ("□", "□∃e(Run(e) ∧ Agent(e, John))", "□(∃e(Agent(e, John) ∧ Run(e)))", "◇∃e(Run(e) ∧ Agent(e, John))"),
    ("◇", "◇∃e(Run(e) ∧ Agent(e, John))", "◇(∃e(Agent(e, John) ∧ Run(e)))", "□∃e(Run(e) ∧ Agent(e, John))"),
    ("=", "Tully = Cicero", "(Tully = Cicero)", "Tully ≠ Cicero"),
    ("(", "Taller(John, Mary)", "(Taller(John, Mary))", "Taller(Mary, John)"),
    (")", "∃x(Man(x) ∧ Happy(x))", "∃x((Man(x)) ∧ (Happy(x)))", "∃x(Man(x)) ∧ Happy(x)"),
    (",", "∃e(Run(e) ∧ Agent(e, John))", "∃d(Agent(d,John) ∧ Run(d))", "∃e(Run(e) ∧ Agent(John, e))"),
];

fn pad() -> BTreeSet<char> {
    symbol_buttons().iter().flat_map(|b| b.symbol.chars()).collect()
}

/// The formula as a learner types it: a modal's printed strength (`_{0.5}`) left out.
fn learner_form(key: &str) -> String {
    let mut out = String::new();
    let mut rest = key;
    while let Some(i) = rest.find("_{") {
        out.push_str(&rest[..i]);
        rest = match rest[i..].find('}') {
            Some(close) => &rest[i + close + 1..],
            None => "",
        };
    }
    out.push_str(rest);
    out
}

/// Every key a learner may be asked to type on /learn: the lessons' typed questions, and the keys
/// the practice exercises generate over a spread of seeds.
fn typed_keys() -> Vec<(String, String)> {
    let engine = ContentEngine::shared();
    let generator = Generator::new();
    let mut keys = Vec::new();
    for era in engine.eras() {
        for module in &era.modules {
            for section in &module.sections {
                if let Some(LessonQuestion::Translate { answer, .. }) = &section.question {
                    keys.push((format!("lesson {}", section.id), answer.clone()));
                }
            }
            for ex in module.exercises.iter().filter(|e| e.exercise_type != ExerciseType::MultipleChoice) {
                for seed in 0u64..16 {
                    let Some(challenge) = generator.generate(ex, &mut StdRng::seed_from_u64(seed)) else { continue };
                    let at = format!("{} {} ({:?})", module.meta.id, ex.id, challenge.sentence);
                    match challenge.answer {
                        AnswerType::FreeForm { golden_logic } => keys.push((at, golden_logic)),
                        AnswerType::Ambiguity { readings } => keys.extend(readings.into_iter().map(|r| (at.clone(), r))),
                        AnswerType::MultipleChoice { .. } => {}
                    }
                }
            }
        }
    }
    keys.sort();
    keys.dedup();
    keys
}

#[test]
fn every_symbol_a_typed_key_needs_is_a_button() {
    let pad = pad();
    let keys = typed_keys();
    assert!(keys.len() >= 40, "only {} typed keys were found", keys.len());
    let mut v = Vec::new();
    for (at, key) in &keys {
        let typed = learner_form(key);
        let missing: BTreeSet<char> = typed.chars().filter(|c| !c.is_alphanumeric() && !c.is_whitespace() && *c != '_' && !pad.contains(c)).collect();
        if !missing.is_empty() {
            v.push(format!("{at}: {typed:?} needs {missing:?}, which the pad lacks"));
        }
        if !check_answer(&typed, key).correct {
            v.push(format!("{at}: {typed:?}, written with the pad, is marked wrong against {key:?}"));
        }
    }
    assert!(v.is_empty(), "\n{} typed keys the pad cannot write:\n  {}\n", v.len(), v.join("\n  "));
}

#[test]
fn every_button_has_a_reading_and_every_reading_a_button() {
    let buttons: BTreeSet<&str> = symbol_buttons().iter().map(|b| b.symbol).collect();
    let read: BTreeSet<&str> = READINGS.iter().map(|(s, ..)| *s).collect();
    assert_eq!(buttons, read, "the pad's buttons and the symbols this lock reads must be the same set");
}

#[test]
fn the_grader_reads_every_button() {
    let mut v = Vec::new();
    for (symbol, formula, same, different) in READINGS {
        assert!(formula.contains(symbol), "the reading of {symbol} does not use it");
        if !check_answer(same, formula).correct {
            v.push(format!("{symbol}: {same:?} is marked wrong against {formula:?}"));
        }
        if check_answer(different, formula).correct {
            v.push(format!("{symbol}: {different:?} is marked right against {formula:?}"));
        }
    }
    assert!(v.is_empty(), "\n{} buttons the grader misreads:\n  {}\n", v.len(), v.join("\n  "));
}

#[test]
fn the_grader_reads_the_deontic_operators_as_logos_prints_them() {
    // "should" prints O_{0.6}, "may" P_{0.5}: a learner writes the operator without its strength.
    let should = "O_{0.6} ∃e(Run(e) ∧ Agent(e, John))";
    assert!(check_answer("O∃e(Run(e) ∧ Agent(e, John))", should).correct);
    assert!(check_answer("O(∃e(Agent(e, John) ∧ Run(e)))", should).correct);
    assert!(!check_answer("∃e(Run(e) ∧ Agent(e, John))", should).correct, "dropping the obligation is wrong");
    assert!(!check_answer("O∃e(Run(e) ∧ Agent(e, Mary))", should).correct);
    let may = "P_{0.5} ∃e(Leave(e) ∧ Agent(e, Mary))";
    assert!(check_answer("P∃e(Leave(e) ∧ Agent(e, Mary))", may).correct);
    assert!(!check_answer("O∃e(Leave(e) ∧ Agent(e, Mary))", may).correct, "permission is not obligation");
    assert!(check_answer("(P → Q)", "P → Q").correct, "a sentence letter P is still a letter");
    assert!(check_answer("(P ∧ Q) → P", "P ∧ Q → P").correct);
}
