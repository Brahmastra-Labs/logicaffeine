//! Every learn-section example, sent through the REAL `compile()` pipeline.
//!
//! This is the safety net the user asked for: the actual English→FOL engine is
//! run over every exercise prompt, so we learn the moment the engine breaks on
//! our teaching content. Many prompts are concept questions or abstract notation
//! that legitimately don't translate to FOL, so a graceful `Err` is fine — the
//! invariant we enforce is the hard one:
//!
//!   **`compile()` must NEVER PANIC on a learn example** (a panic crashes the
//!   page), except for a documented set of prompts that trip a known compiler
//!   robustness bug.
//!
//! KNOWN BUG: the engine panics (rather than erroring) on the "No one is X
//! unless he or she is Y" construction. The eight exercises below are tracked in
//! [`KNOWN_COMPILER_PANIC`]; this test asserts EXACTLY that set still panics, so
//! (a) any NEW panic — from new content or an engine regression — fails the test,
//! and (b) once the compiler is fixed, the now-passing prompts fail this test and
//! prompt removal from the list. The bug itself is reported to the maintainer; it
//! lives in `logicaffeine_language`, outside the learn-content scope.

use std::collections::BTreeSet;
use std::panic;

use logicaffeine_language::compile;
use logicaffeine_web::content::{ContentEngine, ExerciseType};

/// Exercise ids whose prompt currently PANICS `compile()` (the "No one is X
/// unless he or she is Y" construction). A compiler bug, tracked here.
const KNOWN_COMPILER_PANIC: &[&str] = &[
    "A_1.20", "A_1.73", "A_1.76", "A_1.79", "A_1.83", "A_1.87", "A_1.91", "A_1.95",
];

enum Outcome {
    Compiled,
    Rejected,
    Panicked,
}

fn run(prompt: &str) -> Outcome {
    let prev = panic::take_hook();
    panic::set_hook(Box::new(|_| {}));
    let r = panic::catch_unwind(panic::AssertUnwindSafe(|| compile(prompt)));
    panic::set_hook(prev);
    match r {
        Ok(Ok(_)) => Outcome::Compiled,
        Ok(Err(_)) => Outcome::Rejected,
        Err(_) => Outcome::Panicked,
    }
}

#[test]
fn every_example_goes_through_the_compile_pipeline_without_crashing() {
    let engine = ContentEngine::new();

    let known: BTreeSet<&str> = KNOWN_COMPILER_PANIC.iter().copied().collect();
    let mut panicked: BTreeSet<String> = BTreeSet::new();
    let (mut compiled, mut rejected, mut total) = (0usize, 0usize, 0usize);

    for era in engine.eras() {
        for module in &era.modules {
            for ex in &module.exercises {
                if ex.exercise_type != ExerciseType::MultipleChoice {
                    continue;
                }
                total += 1;
                match run(&ex.prompt) {
                    Outcome::Compiled => compiled += 1,
                    Outcome::Rejected => rejected += 1,
                    Outcome::Panicked => {
                        panicked.insert(ex.id.clone());
                    }
                }
            }
        }
    }

    eprintln!(
        "compile pipeline over {total} MC prompts: {compiled} compiled, {rejected} gracefully rejected, {} panicked",
        panicked.len()
    );

    // NEW panics = real regressions (new content or an engine change). Hard fail.
    let new_panics: Vec<&String> = panicked
        .iter()
        .filter(|id| !known.contains(id.as_str()))
        .collect();
    assert!(
        new_panics.is_empty(),
        "\ncompile() PANICS on {} learn prompt(s) NOT in the known-bug list — \
         a crash regression:\n  {:?}\n",
        new_panics.len(),
        new_panics
    );

    // A known-panic prompt that no longer panics means the compiler bug was fixed.
    let recovered: Vec<&&str> = known
        .iter()
        .filter(|id| !panicked.contains(**id))
        .collect();
    assert!(
        recovered.is_empty(),
        "\n{} prompt(s) in KNOWN_COMPILER_PANIC no longer panic — the compiler bug \
         appears fixed; remove them from the list:\n  {:?}\n",
        recovered.len(),
        recovered
    );
}
