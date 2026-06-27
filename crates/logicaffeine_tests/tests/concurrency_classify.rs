//! Phase 0 (FINISH_INTERPRETER.md) — determinacy classifier tests.
//!
//! The classifier labels a LOGOS program as belonging to the determinate
//! (Kahn-deterministic) or nondeterminate fragment. Determinate = spawn + FIFO
//! pipes + data-independent parallel blocks. Nondeterminate = `Select`
//! (`Await the first of:`), `After` timeouts, `Try to send/receive`, `Stop`.

use logicaffeine_compile::concurrency::{Determinacy, NondetKind};
use logicaffeine_compile::{classify_source, first_parallel_block_independent};

fn kinds(d: &Determinacy) -> Vec<NondetKind> {
    d.nondet_kinds()
}

#[test]
fn classify_producer_consumer_is_determinate() {
    let src = "## To produce (ch: Int):\n\
        \x20   Send 1 into ch.\n\
        \x20   Send 2 into ch.\n\
        \n\
        ## Main\n\
        \x20   Let jobs be a Pipe of Int.\n\
        \x20   Launch a task to produce with jobs.\n\
        \x20   Receive first from jobs.\n\
        \x20   Receive second from jobs.\n\
        \x20   Show first.\n\
        \x20   Show second.\n";
    let d = classify_source(src).expect("parse");
    assert!(d.is_determinate(), "producer/consumer must be Determinate, got {:?}", d);
}

#[test]
fn classify_plain_program_is_determinate() {
    let src = "## Main\n\
        \x20   Let x be 1 + 2.\n\
        \x20   Show x.\n";
    let d = classify_source(src).expect("parse");
    assert!(d.is_determinate(), "plain arithmetic must be Determinate, got {:?}", d);
}

#[test]
fn classify_select_is_nondeterminate() {
    let src = "## Main\n\
        \x20   Let ch be a Pipe of Int.\n\
        \x20   Send 42 into ch.\n\
        \x20   Await the first of:\n\
        \x20       Receive x from ch:\n\
        \x20           Show x.\n\
        \x20       After 1 seconds:\n\
        \x20           Show \"timeout\".\n";
    let d = classify_source(src).expect("parse");
    let ks = kinds(&d);
    assert!(ks.contains(&NondetKind::Select), "expected Select witness, got {:?}", d);
    assert!(ks.contains(&NondetKind::AfterTimer), "expected AfterTimer witness, got {:?}", d);
}

#[test]
fn classify_two_concurrent_printers_is_nondeterminate() {
    // Both the main flow AND a fire-and-forget worker write to stdout, so the
    // ORDER of "worker ran" / "main ran" is a race — observably nondeterministic
    // even though no Select/Try/Stop appears. Kahn determinacy covers channel
    // histories, not the shared stdout sink.
    let src = "## To worker:\n\
        \x20   Show \"worker ran\".\n\
        \n\
        ## Main\n\
        \x20   Launch a task to worker.\n\
        \x20   Show \"main ran\".\n";
    let d = classify_source(src).expect("parse");
    assert!(
        kinds(&d).contains(&NondetKind::ConcurrentPrint),
        "two concurrent stdout writers must be ConcurrentPrint-nondeterminate, got {:?}",
        d
    );
}

#[test]
fn classify_silent_worker_with_main_printer_is_determinate() {
    // The spawned task ONLY does channel I/O (no Show); the single printer is the
    // main flow, whose pipe receives causally order everything. One writer ⇒ no
    // race ⇒ Determinate. This is the line the analysis must not over-step.
    let src = "## To emit (ch: Int, n: Int):\n\
        \x20   Send n into ch.\n\
        \n\
        ## Main\n\
        \x20   Let ch be a Pipe of Int.\n\
        \x20   Launch a task to emit with ch and 10.\n\
        \x20   Launch a task to emit with ch and 20.\n\
        \x20   Receive a from ch.\n\
        \x20   Receive b from ch.\n\
        \x20   Let total be a + b.\n\
        \x20   Show total.\n";
    let d = classify_source(src).expect("parse");
    assert!(d.is_determinate(), "single-printer pipeline must stay Determinate, got {:?}", d);
}

#[test]
fn classify_two_silent_workers_one_main_printer_is_determinate() {
    // Many spawned tasks, but only ONE thread (main) prints — determinate.
    let src = "## To produce (ch: Int):\n\
        \x20   Send 1 into ch.\n\
        \n\
        ## Main\n\
        \x20   Let a be a Pipe of Int.\n\
        \x20   Let b be a Pipe of Int.\n\
        \x20   Launch a task to produce with a.\n\
        \x20   Launch a task to produce with b.\n\
        \x20   Receive x from a.\n\
        \x20   Receive y from b.\n\
        \x20   Show x.\n";
    let d = classify_source(src).expect("parse");
    assert!(d.is_determinate(), "one printer + silent workers must stay Determinate, got {:?}", d);
}

#[test]
fn classify_after_timeout_is_nondeterminate() {
    let src = "## Main\n\
        \x20   Let p be a Pipe of Text.\n\
        \x20   Await the first of:\n\
        \x20       After 1 seconds:\n\
        \x20           Show \"timeout\".\n";
    let d = classify_source(src).expect("parse");
    assert!(kinds(&d).contains(&NondetKind::AfterTimer), "expected AfterTimer, got {:?}", d);
}

#[test]
fn classify_try_receive_is_nondeterminate() {
    let src = "## Main\n\
        \x20   Let ch be a Pipe of Int.\n\
        \x20   Try to receive x from ch.\n\
        \x20   Show \"tried\".\n";
    let d = classify_source(src).expect("parse");
    assert!(kinds(&d).contains(&NondetKind::TryRecv), "expected TryRecv, got {:?}", d);
}

#[test]
fn classify_try_send_is_nondeterminate() {
    let src = "## Main\n\
        \x20   Let ch be a Pipe of Int.\n\
        \x20   Try to send 99 into ch.\n\
        \x20   Show \"sent\".\n";
    let d = classify_source(src).expect("parse");
    assert!(kinds(&d).contains(&NondetKind::TrySend), "expected TrySend, got {:?}", d);
}

#[test]
fn classify_stop_task_is_nondeterminate() {
    let src = "## To infinite:\n\
        \x20   Show \"started\".\n\
        \n\
        ## Main\n\
        \x20   Let handle be Launch a task to infinite.\n\
        \x20   Stop handle.\n\
        \x20   Show \"stopped\".\n";
    let d = classify_source(src).expect("parse");
    assert!(kinds(&d).contains(&NondetKind::StopTask), "expected StopTask, got {:?}", d);
}

#[test]
fn classify_transitive_through_launch() {
    // `g` (launched as a task) contains a Select; the whole program is Nondeterminate.
    let src = "## To g (ch: Int):\n\
        \x20   Await the first of:\n\
        \x20       Receive x from ch:\n\
        \x20           Show x.\n\
        \x20       After 1 seconds:\n\
        \x20           Show \"timeout\".\n\
        \n\
        ## Main\n\
        \x20   Let ch be a Pipe of Int.\n\
        \x20   Launch a task to g with ch.\n\
        \x20   Send 5 into ch.\n";
    let d = classify_source(src).expect("parse");
    assert!(!d.is_determinate(), "Select in a launched fn must make the program Nondeterminate, got {:?}", d);
    assert!(kinds(&d).contains(&NondetKind::Select), "expected Select witness, got {:?}", d);
}

#[test]
fn branches_independent_when_disjoint() {
    let src = "## Main\n\
        \x20   Simultaneously:\n\
        \x20       Let a be 100.\n\
        \x20       Let b be 200.\n\
        \x20   Show a.\n\
        \x20   Show b.\n";
    let r = first_parallel_block_independent(src).expect("parse");
    assert_eq!(r, Some(true), "disjoint parallel branches are independent");
}

#[test]
fn branches_dependent_when_sharing_a_pipe() {
    let src = "## Main\n\
        \x20   Let ch be a Pipe of Int.\n\
        \x20   Simultaneously:\n\
        \x20       Send 1 into ch.\n\
        \x20       Receive x from ch.\n";
    let r = first_parallel_block_independent(src).expect("parse");
    assert_eq!(r, Some(false), "branches sharing a pipe are dependent");
}
