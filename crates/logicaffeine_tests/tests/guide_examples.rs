//! Exhaustive runtime audit of every code example in the Syntax Guide.
//!
//! The guide (`ui::pages::guide::content::SECTIONS`) ships interactive code
//! examples. In the browser each one is executed on the SAME engine entrypoints
//! the page wires up:
//!   - `ExampleMode::Logic`      → `compile_for_ui` (English → First-Order Logic)
//!   - `ExampleMode::Imperative` → `interpret_for_ui` (bytecode VM / tree-walker)
//!   - `ExampleMode::Web`        → the page's own `## Test` blocks (`guide::page_verdict`); the
//!     page battery — mounted, rendered on both in-process tiers, compiled for the browser — is
//!     `canonical_teaching_lock`'s
//!
//! Every example DECLARES how it behaves via `CodeExample::runnability` — the
//! single source of truth shared by this harness, the Run-button affordance,
//! and the "(Compiled Only)" badge. This test asserts the declaration in BOTH
//! directions, so drift can never survive:
//!
//!   * [`Runnability::Playground`]: MUST execute with no error and no panic —
//!     and when `expected_output` is `Some`, the captured output must MATCH
//!     (Imperative: exact `lines` match; Logic: whitespace-normalized FOL).
//!
//!   * [`Runnability::CompiledOnly`]: the runtime only exists in compiled Rust;
//!     the playground must surface a graceful diagnostic instead of running
//!     clean. If one starts running clean, this FAILS — the signal to flip it
//!     to `Playground` (which simultaneously fixes the UI badge).
//!
//!   * [`Runnability::ExpectedError`]: the example fails ON PURPOSE — the
//!     displayed Socratic error IS the content. The full diagnostic must match
//!     EXACTLY; on mismatch the actual message is printed so a deliberate
//!     diagnostic rewording is a mechanical guide update.
//!
//! Each example runs on its own thread with a wall-clock timeout so a runaway
//! loop can never wedge the suite, and `catch_unwind` turns the debug
//! shadow-oracle assertions (VM-vs-tree-walker divergence) into a recorded
//! outcome rather than aborting the run.

#![cfg(feature = "web-tests")]

use std::panic;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use logicaffeine_compile::{compile_for_ui, compile_theorem_for_ui, interpret_for_ui_sync};
use logicaffeine_web::ui::pages::guide::content::{ExampleMode, Runnability, SECTIONS};
use logicaffeine_web::ui::pages::guide::{page_verdict, theorem_verdict};

/// Hard ceiling per example. The interpreter for these teaching snippets is
/// sub-millisecond; anything past this is a genuine hang we want surfaced.
const PER_EXAMPLE_TIMEOUT: Duration = Duration::from_secs(20);

/// Exact census of guide examples. Deliberately exact-match (not a floor): an
/// example can never be added or removed silently — every change to the corpus
/// is a conscious edit here too.
const EXAMPLE_CENSUS: usize = 173;

/// Raise-only floor of `Playground` examples. The playground story only ever
/// gets better; a demotion must be argued here in review, never slipped in.
const RUNNABLE_FLOOR: usize = 157;

/// Raise-only floor of examples carrying `Some(expected_output)`. Output
/// coverage never regresses. A web example is locked by its own `## Test`
/// blocks instead of by printed output, so it does not count here.
const OUTPUT_LOCKED_FLOOR: usize = 153;

#[derive(Debug)]
enum Outcome {
    /// Ran clean; carries the captured output (Imperative: joined `lines`;
    /// Logic: the FOL string).
    Ok(String),
    EngineError(String),
    Panicked(String),
    TimedOut,
}

fn run_example(mode: ExampleMode, code: &'static str) -> Outcome {
    let (tx, rx) = mpsc::channel();
    let handle = thread::Builder::new()
        .name("guide-example".into())
        .stack_size(64 * 1024 * 1024)
        .spawn(move || {
            let result = panic::catch_unwind(|| match mode {
                ExampleMode::Logic => {
                    let r = compile_for_ui(code);
                    match (r.logic, r.error) {
                        (_, Some(e)) => Outcome::EngineError(e),
                        (Some(fol), None) => Outcome::Ok(fol),
                        (None, None) => Outcome::EngineError("no logic output".into()),
                    }
                }
                ExampleMode::Imperative => {
                    let r = interpret_for_ui_sync(code);
                    match r.error {
                        Some(e) => Outcome::EngineError(e),
                        None => Outcome::Ok(r.lines.join("\n")),
                    }
                }
                ExampleMode::Web => match page_verdict(code) {
                    Ok(passed) => Outcome::Ok(format!("{passed} test(s) pass")),
                    Err(e) => Outcome::EngineError(e),
                },
                ExampleMode::Theorem => {
                    // The exact rendering the Run button shows — shared fn.
                    let r = compile_theorem_for_ui(code);
                    match theorem_verdict(&r) {
                        Ok(verdict) => Outcome::Ok(verdict),
                        Err(e) => Outcome::EngineError(e),
                    }
                }
            });
            let outcome = result.unwrap_or_else(|payload| {
                let msg = payload
                    .downcast_ref::<&str>()
                    .map(|s| s.to_string())
                    .or_else(|| payload.downcast_ref::<String>().cloned())
                    .unwrap_or_else(|| "<non-string panic payload>".to_string());
                Outcome::Panicked(msg)
            });
            let _ = tx.send(outcome);
        })
        .expect("spawn guide-example thread");

    match rx.recv_timeout(PER_EXAMPLE_TIMEOUT) {
        Ok(outcome) => {
            let _ = handle.join();
            outcome
        }
        Err(_) => Outcome::TimedOut,
    }
}

/// Whitespace-normalize a FOL string for Logic-mode output comparison (the
/// `doc_examples.rs` idiom — layout is presentation, symbols are semantics).
fn norm(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or("").to_string()
}

#[test]
fn every_guide_example_matches_its_declared_behavior() {
    std::env::remove_var("LOGOS_ENGINE_TRACE");
    // Suppress the default panic hook's backtrace spam for the divergence/reactor
    // panics we deliberately catch below; our own reporting covers real failures.
    let prev_hook = panic::take_hook();
    panic::set_hook(Box::new(|_| {}));

    let mut wrong: Vec<String> = Vec::new();
    let mut total = 0usize;
    let mut playground = 0usize;
    let mut compiled_only = 0usize;
    let mut expected_error = 0usize;
    let mut output_locked = 0usize;

    for section in SECTIONS.iter() {
        for example in section.examples.iter() {
            total += 1;
            let outcome = run_example(example.mode, example.code);
            let label = format!(
                "[§{} {}] `{}` ({:?})",
                section.number, section.title, example.id, example.mode
            );

            match example.runnability {
                Runnability::Playground => {
                    playground += 1;
                    match outcome {
                        Outcome::Ok(actual) => {
                            if let Some(expected) = example.expected_output {
                                output_locked += 1;
                                let matches = match example.mode {
                                    ExampleMode::Imperative
                                    | ExampleMode::Theorem
                                    | ExampleMode::Web => actual == expected,
                                    ExampleMode::Logic => norm(&actual) == norm(expected),
                                };
                                if !matches {
                                    wrong.push(format!(
                                        "{label}\n    OUTPUT MISMATCH\n    expected: {expected:?}\n    actual:   {actual:?}"
                                    ));
                                }
                            }
                        }
                        Outcome::EngineError(e) => {
                            wrong.push(format!("{label}\n    ENGINE ERROR: {}", first_line(&e)))
                        }
                        Outcome::Panicked(e) => {
                            wrong.push(format!("{label}\n    PANIC: {}", first_line(&e)))
                        }
                        Outcome::TimedOut => wrong.push(format!(
                            "{label}\n    TIMED OUT (> {PER_EXAMPLE_TIMEOUT:?})"
                        )),
                    }
                }
                Runnability::CompiledOnly { reason } => {
                    compiled_only += 1;
                    match outcome {
                        Outcome::Ok(_) => wrong.push(format!(
                            "{label}\n    UNEXPECTEDLY RUNS CLEAN in the playground now \
                             (declared CompiledOnly: {reason}) — flip it to \
                             Runnability::Playground; the badge and Run button follow \
                             automatically."
                        )),
                        // Graceful engine error or a caught shadow-oracle panic both
                        // count as "the playground surfaced a diagnostic".
                        Outcome::EngineError(_) | Outcome::Panicked(_) => {}
                        Outcome::TimedOut => wrong.push(format!(
                            "{label}\n    TIMED OUT (> {PER_EXAMPLE_TIMEOUT:?}) — a \
                             CompiledOnly example must fail FAST and gracefully, not hang."
                        )),
                    }
                }
                Runnability::ExpectedError { message } => {
                    expected_error += 1;
                    match outcome {
                        Outcome::EngineError(actual) => {
                            if actual != message {
                                wrong.push(format!(
                                    "{label}\n    ERROR MESSAGE MISMATCH (full-message lock)\n    expected: {message:?}\n    actual:   {actual:?}"
                                ));
                            }
                        }
                        Outcome::Ok(out) => wrong.push(format!(
                            "{label}\n    RAN CLEAN but is declared ExpectedError — either \
                             the diagnostic disappeared (promote the example) or the code \
                             drifted. output: {out:?}"
                        )),
                        Outcome::Panicked(e) => wrong.push(format!(
                            "{label}\n    PANICKED instead of erroring gracefully: {}",
                            first_line(&e)
                        )),
                        Outcome::TimedOut => wrong.push(format!(
                            "{label}\n    TIMED OUT (> {PER_EXAMPLE_TIMEOUT:?})"
                        )),
                    }
                }
            }
        }
    }

    panic::set_hook(prev_hook);

    eprintln!(
        "guide examples: {total} total = {playground} playground + {compiled_only} compiled-only \
         + {expected_error} expected-error ({output_locked} output-locked)"
    );

    assert!(
        wrong.is_empty(),
        "\n{} guide example(s) no longer match their declared behavior:\n\n{}\n",
        wrong.len(),
        wrong.join("\n\n")
    );

    assert_eq!(
        total, EXAMPLE_CENSUS,
        "guide example census changed ({total} != {EXAMPLE_CENSUS}) — update EXAMPLE_CENSUS \
         consciously (examples are never added or removed silently)"
    );
    assert!(
        playground >= RUNNABLE_FLOOR,
        "playground-runnable examples fell below the floor ({playground} < {RUNNABLE_FLOOR}) — \
         demotions must be argued here, not slipped in"
    );
}

/// `CompiledOnly` and `ExpectedError` examples must not carry `expected_output`
/// — the reason/message field owns their story; a stray `Some` would be dead,
/// misleading data.
#[test]
fn only_playground_examples_carry_expected_output() {
    let offenders: Vec<String> = SECTIONS
        .iter()
        .flat_map(|s| s.examples.iter())
        .filter(|e| {
            e.expected_output.is_some() && !matches!(e.runnability, Runnability::Playground)
        })
        .map(|e| e.id.to_string())
        .collect();

    assert!(
        offenders.is_empty(),
        "\nnon-Playground example(s) carry expected_output: {offenders:?}\n"
    );
}

/// Output coverage never regresses: the count of Playground examples with a
/// locked `expected_output` may only grow. Raise the floor with every backfill.
#[test]
fn output_locked_floor_never_regresses() {
    let locked = SECTIONS
        .iter()
        .flat_map(|s| s.examples.iter())
        .filter(|e| e.expected_output.is_some())
        .count();

    assert!(
        locked >= OUTPUT_LOCKED_FLOOR,
        "output-locked examples fell below the floor ({locked} < {OUTPUT_LOCKED_FLOOR})"
    );
}

/// The guide never promises. "Coming Soon" rots the moment a feature ships
/// (the stdlib section promised File/Time/Random/Env long after all four had
/// shipped). If something isn't available, the guide doesn't document it; if
/// it is, the guide states it as present — so the phrase is banned outright.
#[test]
fn guide_never_promises_coming_soon() {
    let offenders: Vec<String> = SECTIONS
        .iter()
        .filter(|s| s.content.contains("Coming Soon") || s.content.contains("coming soon"))
        .map(|s| format!("[§{} {}] ({})", s.number, s.title, s.id))
        .collect();

    assert!(
        offenders.is_empty(),
        "\nguide section(s) contain \"Coming Soon\" promises — document shipped reality instead:\n{}\n",
        offenders.join("\n")
    );
}

/// The badge owns the compiled-only claim now — labels may never hardcode it.
/// A label saying "(Compiled Only)" is presentation duplicating (and inevitably
/// drifting from) the `runnability` field the badge renders from.
#[test]
fn labels_never_claim_compiled_only() {
    let offenders: Vec<String> = SECTIONS
        .iter()
        .flat_map(|s| s.examples.iter())
        .filter(|e| e.label.contains("Compiled Only"))
        .map(|e| format!("`{}` (label: {:?})", e.id, e.label))
        .collect();

    assert!(
        offenders.is_empty(),
        "\nexample label(s) hardcode \"Compiled Only\" — the Runnability badge owns that claim:\n{}\n",
        offenders.join("\n")
    );
}

/// Section ids are DOM anchors (`/guide#<id>`) and example ids are DOM ids —
/// a duplicate would make one entry shadow another silently in both the router
/// and this harness.
#[test]
fn section_and_example_ids_are_unique() {
    let mut seen_sections = std::collections::HashSet::new();
    let mut seen_examples = std::collections::HashSet::new();
    let mut dupes: Vec<String> = Vec::new();

    for section in SECTIONS.iter() {
        if !seen_sections.insert(section.id) {
            dupes.push(format!("duplicate section id `{}`", section.id));
        }
        for example in section.examples.iter() {
            if !seen_examples.insert(example.id) {
                dupes.push(format!(
                    "duplicate example id `{}` (in §{} {})",
                    example.id, section.number, section.title
                ));
            }
        }
    }

    assert!(dupes.is_empty(), "\n{}\n", dupes.join("\n"));
}

/// Authoring aid, not a lock: dumps the REAL engine output for every Playground
/// example still missing `expected_output`, in a paste-ready form. Run it twice
/// and diff — only stable outputs get backfilled; unstable ones (clock, entropy,
/// interleaving) stay `None` with a `// nondeterministic:` comment.
///
/// `cargo nextest run -p logicaffeine-web -E 'test(dump_expected_outputs)' \
///      --run-ignored ignored-only --no-capture`
#[test]
#[ignore = "authoring aid — dumps real outputs for expected_output backfill"]
fn dump_expected_outputs() {
    std::env::remove_var("LOGOS_ENGINE_TRACE");
    let prev_hook = panic::take_hook();
    panic::set_hook(Box::new(|_| {}));

    for section in SECTIONS.iter() {
        for example in section.examples.iter() {
            if !matches!(example.runnability, Runnability::Playground)
                || example.expected_output.is_some()
            {
                continue;
            }
            match run_example(example.mode, example.code) {
                Outcome::Ok(actual) => {
                    println!("=== {} (§{} {})", example.id, section.number, section.title);
                    println!("{actual:?}");
                }
                other => println!("=== {} DID NOT RUN CLEAN: {other:?}", example.id),
            }
        }
    }

    panic::set_hook(prev_hook);
}
