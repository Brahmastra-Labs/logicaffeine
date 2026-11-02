//! The delight layer's locks: exercises grade correctly by construction, and
//! the interactive affordances (predict-toggle, search, expand-all) render.
//!
//! The exercise lock is the load-bearing one: every exercise ships with a
//! hidden REFERENCE solution, and this test grades that reference with the
//! exact machinery the page uses — an exercise whose own answer key fails is
//! impossible to ship.

#![cfg(feature = "web-tests")]

use dioxus::prelude::*;
use dioxus_history::{History, MemoryHistory};
use logicaffeine_compile::interpret_for_ui_sync;
use logicaffeine_web::grader::check_answer;
use logicaffeine_web::ui::pages::guide::content::{ExampleMode, SECTIONS};
use std::rc::Rc;

/// Raise-only floor: the guide always carries at least this many exercises.
const EXERCISE_FLOOR: usize = 66;

#[test]
fn every_exercise_reference_solution_grades_correct() {
    let mut total = 0usize;
    let mut wrong: Vec<String> = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for section in SECTIONS.iter() {
        for ex in section.exercises.iter() {
            total += 1;
            if !seen.insert(ex.id) {
                wrong.push(format!("duplicate exercise id `{}`", ex.id));
            }
            match ex.mode {
                ExampleMode::Imperative => {
                    let r = interpret_for_ui_sync(ex.reference);
                    if let Some(e) = r.error {
                        wrong.push(format!(
                            "`{}`: reference solution errors: {}",
                            ex.id,
                            e.lines().next().unwrap_or("")
                        ));
                    } else if r.lines.join("\n") != ex.expected {
                        wrong.push(format!(
                            "`{}`: reference output {:?} != expected {:?}",
                            ex.id,
                            r.lines.join("\n"),
                            ex.expected
                        ));
                    }
                }
                ExampleMode::Logic => {
                    let grade = check_answer(ex.reference, ex.expected);
                    if !grade.correct {
                        wrong.push(format!(
                            "`{}`: reference FOL does not grade correct: {}",
                            ex.id, grade.feedback
                        ));
                    }
                }
                ExampleMode::Web => {
                    if !ex.expected.is_empty() {
                        wrong.push(format!(
                            "`{}`: a web exercise is graded by its own `## Test` blocks — its `expected` stays empty",
                            ex.id
                        ));
                    }
                    if let Err(e) = logicaffeine_web::ui::pages::guide::page_verdict(ex.reference) {
                        wrong.push(format!(
                            "`{}`: reference page's tests do not pass: {e}",
                            ex.id
                        ));
                    }
                    if logicaffeine_web::ui::pages::guide::page_verdict(ex.starter).is_ok() {
                        wrong.push(format!(
                            "`{}`: the starter already passes — there is nothing to do",
                            ex.id
                        ));
                    }
                }
                ExampleMode::Theorem => {
                    wrong.push(format!(
                        "`{}`: theorem-mode exercises are not supported yet",
                        ex.id
                    ));
                }
            }
        }
    }

    assert!(
        wrong.is_empty(),
        "\nexercise answer-key violation(s):\n{}\n",
        wrong.join("\n")
    );
    assert!(
        total >= EXERCISE_FLOOR,
        "exercise count fell below the floor ({total} < {EXERCISE_FLOOR})"
    );
}

fn render_guide() -> String {
    let mut vdom = VirtualDom::new(logicaffeine_web::App);
    vdom.provide_root_context(
        Rc::new(MemoryHistory::with_initial_path("/guide")) as Rc<dyn History>
    );
    vdom.rebuild_in_place();
    dioxus_ssr::render(&vdom)
}

/// The interactive affordances all render on the guide page: the in-guide
/// search box, the expand-all-examples control, and (per example with a locked
/// output) the predict-toggle. Exercises render their prompts.
#[test]
fn delight_affordances_render_on_the_guide_page() {
    let html = render_guide();
    assert!(
        html.contains("guide-search-input"),
        "the in-guide search box must render"
    );
    assert!(
        html.contains("guide-expand-all"),
        "the collapse/expand control must render"
    );
    // Example groups are EXPANDED by default — the runnable examples are the
    // guide's best asset and must never hide behind a click.
    assert!(
        html.contains("guide-examples-collapsible expanded"),
        "collapsible example groups must default to expanded"
    );
    assert!(
        !html.contains("\"guide-examples-collapsible\""),
        "no example group may render collapsed by default"
    );
    assert!(
        html.contains("guide-predict-toggle"),
        "the predict-toggle must render on output-locked examples"
    );
    assert!(
        html.contains("guide-exercise"),
        "exercise blocks must render"
    );
    assert!(
        html.contains("guide-section-check"),
        "section completion checkmarks must render in the sidebar"
    );
}

/// The pedagogy affordances that exist from the infrastructure wave onward:
/// exercise kind badges (the fading ladder made visible) and the sidebar
/// mastery ring (progress = exercises passed, not pages scrolled).
#[test]
fn pedagogy_affordances_render_on_the_guide_page() {
    let html = render_guide();
    assert!(
        html.contains("guide-exercise-kind"),
        "exercise kind badges must render on the guide page"
    );
    assert!(
        html.contains("guide-exercise-hint"),
        "the hint affordance must render on exercises"
    );
    assert!(
        html.contains("guide-mastery-ring"),
        "the sidebar must render a mastery ring for exercise-bearing sections"
    );
}

/// The search index is a pure function — locked natively: title hits, prose
/// hits, code hits, and a non-hit.
#[test]
fn guide_search_finds_sections() {
    use logicaffeine_web::ui::pages::guide::search_sections;

    let by_title = search_sections("machine words");
    assert!(
        by_title.iter().any(|s| s.id == "machine-words"),
        "title search must hit machine-words: {by_title:?}"
    );
    let by_code = search_sections("xorshift");
    assert!(
        by_code.iter().any(|s| s.id == "machine-words"),
        "code search must hit the xorshift example's section"
    );
    let by_prose = search_sections("least surprise");
    assert!(
        by_prose.iter().any(|s| s.id == "reading-english"),
        "prose search must hit reading-english"
    );
    assert!(
        search_sections("zzzzqqqq").is_empty(),
        "a nonsense query matches nothing"
    );
    assert!(
        search_sections("").is_empty(),
        "an empty query matches nothing (no noise)"
    );
}

/// The sentence a Logic exercise asks the reader to translate: the prompt names it after "for: ".
fn sentence_named_by(prompt: &str) -> Option<&str> {
    let rest = prompt.split_once("for: ")?.1;
    let end = rest.find('.')? + 1;
    Some(&rest[..end])
}

/// A reader who runs an exercise's sentence and copies the formula LOGOS shows has written that
/// sentence's First-Order Logic, dictionary categories and all, so the answer key must accept it.
#[test]
fn every_fol_exercise_accepts_what_logos_shows_for_its_sentence() {
    let mut checked = 0usize;
    let mut wrong: Vec<String> = Vec::new();

    for section in SECTIONS.iter() {
        for ex in section
            .exercises
            .iter()
            .filter(|ex| ex.mode == ExampleMode::Logic)
        {
            let Some(sentence) = sentence_named_by(ex.prompt) else {
                wrong.push(format!(
                    "`{}`: the prompt names no sentence after \"for: \": {:?}",
                    ex.id, ex.prompt
                ));
                continue;
            };
            let shown = logicaffeine_compile::compile_for_ui(sentence);
            let Some(formula) = shown.logic else {
                wrong.push(format!(
                    "`{}`: LOGOS shows no formula for {sentence:?}: {:?}",
                    ex.id, shown.error
                ));
                continue;
            };
            checked += 1;
            let grade = check_answer(&formula, ex.expected);
            if !grade.correct {
                wrong.push(format!(
                    "`{}`: LOGOS shows {formula} for {sentence:?}; the key {} marks it wrong ({})",
                    ex.id, ex.expected, grade.feedback
                ));
            }
        }
    }

    assert!(checked > 0, "no Logic exercise was checked");
    assert!(
        wrong.is_empty(),
        "\nFOL exercise key(s) that reject what LOGOS shows:\n{}\n",
        wrong.join("\n")
    );
}
