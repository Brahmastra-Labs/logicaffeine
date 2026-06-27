//! Exhaustive runtime audit of every code example in the Syntax Guide.
//!
//! The guide (`ui::pages::guide::content::SECTIONS`) ships ~90 interactive code
//! examples. In the browser each one is executed on the SAME engine entrypoints
//! the page wires up:
//!   - `ExampleMode::Logic`      → `compile_for_ui` (English → First-Order Logic)
//!   - `ExampleMode::Imperative` → `interpret_for_ui` (bytecode VM / tree-walker)
//!
//! Before this harness there was NOTHING asserting those examples still compile
//! and run — a lexicon change or a parser tweak could silently rot any example
//! and the guide would display a "Run" button that errors.
//!
//! This test asserts a PARTITION of every example into two buckets, so it stays
//! green while still catching drift in BOTH directions:
//!
//!   * **Playground-runnable** (the default): the example MUST execute with no
//!     error and no panic. If one breaks, this fails — a real regression.
//!
//!   * **[`REQUIRES_COMPILATION`]**: the example exercises a runtime that only
//!     exists in compiled Rust (P2P networking, pipes/channels, `Sync`/`Mount`,
//!     memory-mapped zones, and the advanced CRDT mutations the tree-walker/VM
//!     explicitly defer to codegen). The playground interpreter CANNOT run these;
//!     it must surface a graceful diagnostic instead of running clean. If one
//!     starts running clean, this fails too — a signal to promote it in the guide
//!     and drop it from the list.
//!
//! Each example runs on its own thread with a wall-clock timeout so a runaway
//! loop can never wedge the suite, and `catch_unwind` turns the debug
//! shadow-oracle assertions (VM-vs-tree-walker divergence) into a recorded
//! outcome rather than aborting the run.

use std::panic;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use logicaffeine_compile::{compile_for_ui, interpret_for_ui_sync};
use logicaffeine_web::ui::pages::guide::content::{ExampleMode, SECTIONS};

/// Hard ceiling per example. The interpreter for these teaching snippets is
/// sub-millisecond; anything past this is a genuine hang we want surfaced.
const PER_EXAMPLE_TIMEOUT: Duration = Duration::from_secs(20);

/// Examples whose runtime lives only in compiled Rust, so the browser/interpreter
/// playground cannot execute them — it reports a graceful "use compiled Rust" /
/// "requires a prior Connect" diagnostic (or, in the debug shadow-oracle build, a
/// caught VM-vs-tree-walker divergence). The guide labels most of these
/// "(Compiled Only)" and carries section notes saying they don't run in the
/// playground. Tracked explicitly so the harness can assert they DON'T silently
/// start running clean (which would mean the guide should promote them).
///
/// NOTE (audit finding): several §13 CRDT examples below are NOT labeled
/// "(Compiled Only)" in the guide even though the interpreter defers them to
/// compiled Rust. See SYNTAX_GUIDE_WORK.md — the guide prose understates which
/// CRDT operations are playground-runnable.
const REQUIRES_COMPILATION: &[&str] = &[
    // (§12 pipe-send-receive / select-timeout were here, but once the `a new Pipe of T`
    //  parse bug was fixed they actually RUN on the interpreter — promoted to runnable.
    //  Their guide labels still say "(Compiled Only)"; see SYNTAX_GUIDE_WORK.md.)
    // §13 CRDT network sync / persistence: GossipSub + journal runtime.
    "crdt-sync-counter",
    "crdt-sync-profile",
    "crdt-persistent",
    // §13 SharedMap (ORMap) is the one rich CRDT the tree-walker still defers to codegen;
    // OR-Set / RGA / MV-register / counters now run natively in the interpreter.
    "crdt-sharedmap",
    // §15 P2P networking: libp2p transport + relay only exist when compiled.
    "network-listen",
    "network-connect",
    "network-peer-agent",
    "network-send-message",
    "network-distributed",
    "network-mdns",
    "network-file-transfer",
];

#[derive(Debug)]
enum Outcome {
    Ok,
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
                        (Some(_), None) => Outcome::Ok,
                        (None, None) => Outcome::EngineError("no logic output".into()),
                    }
                }
                ExampleMode::Imperative => {
                    let r = interpret_for_ui_sync(code);
                    match r.error {
                        Some(e) => Outcome::EngineError(e),
                        None => Outcome::Ok,
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

#[test]
fn every_guide_example_matches_its_runnability() {
    std::env::remove_var("LOGOS_ENGINE_TRACE");
    // Suppress the default panic hook's backtrace spam for the divergence/reactor
    // panics we deliberately catch below; our own reporting covers real failures.
    let prev_hook = panic::take_hook();
    panic::set_hook(Box::new(|_| {}));

    let mut wrong: Vec<String> = Vec::new();
    let mut total = 0usize;
    let mut runnable = 0usize;
    let mut compiled_only = 0usize;

    for section in SECTIONS.iter() {
        for example in section.examples.iter() {
            total += 1;
            let outcome = run_example(example.mode, example.code);
            let label = format!(
                "[§{} {}] `{}` ({:?})",
                section.number, section.title, example.id, example.mode
            );
            let requires_comp = REQUIRES_COMPILATION.contains(&example.id);
            let ran_clean = matches!(outcome, Outcome::Ok);

            if requires_comp {
                compiled_only += 1;
                if ran_clean {
                    wrong.push(format!(
                        "{label}\n    UNEXPECTEDLY RUNS CLEAN in the interpreter now — \
                         promote it in the guide and drop it from REQUIRES_COMPILATION."
                    ));
                }
            } else {
                runnable += 1;
                if !ran_clean {
                    let detail = match outcome {
                        Outcome::EngineError(e) => format!("ENGINE ERROR: {}", first_line(&e)),
                        Outcome::Panicked(e) => format!("PANIC: {}", first_line(&e)),
                        Outcome::TimedOut => format!("TIMED OUT (> {PER_EXAMPLE_TIMEOUT:?})"),
                        Outcome::Ok => unreachable!(),
                    };
                    wrong.push(format!("{label}\n    {detail}"));
                }
            }
        }
    }

    panic::set_hook(prev_hook);

    eprintln!(
        "guide examples: {total} total = {runnable} playground-runnable + {compiled_only} compiled-only"
    );

    assert!(
        wrong.is_empty(),
        "\n{} guide example(s) no longer match their declared runnability:\n\n{}\n",
        wrong.len(),
        wrong.join("\n\n")
    );
}

fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or("").to_string()
}
