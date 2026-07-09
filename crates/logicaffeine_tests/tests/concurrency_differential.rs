//! Phase 5c (work/FINISH_INTERPRETER.md) — cross-tier concurrency differential.
//!
//! The tree-walker and the bytecode VM share ONE deterministic scheduler
//! (`logicaffeine_runtime`) under the same seed, so a determinate concurrency
//! program must produce byte-identical output on both tiers. `interpret_for_ui`
//! routes concurrency to the tree-walker (`run_program_concurrent`);
//! `run_vm_concurrent` drives the same program on the VM. Any divergence is a
//! compiler/VM bug — this turns the concurrency corpus into a differential suite.

use futures::executor::block_on;
use logicaffeine_compile::{
    interpret_for_ui, run_treewalker_concurrent_seeded, run_vm_concurrent,
    run_vm_concurrent_seeded, run_vm_workstealing_seeded,
};

fn assert_tiers_agree(src: &str) {
    let tw = block_on(interpret_for_ui(src));
    let vm = run_vm_concurrent(src);
    assert_eq!(
        tw.error.is_none(),
        vm.error.is_none(),
        "tier error-agreement diverged for:\n{src}\n  tree-walker error: {:?}\n  vm error: {:?}",
        tw.error,
        vm.error
    );
    assert_eq!(
        tw.lines, vm.lines,
        "tree-walker vs VM output diverged for:\n{src}\n  tree-walker: {:?}\n  vm: {:?}",
        tw.lines, vm.lines
    );
}

#[test]
fn diff_producer_consumer() {
    assert_tiers_agree(
        "## To produce (ch: Int):\n\
        \x20   Send 1 into ch.\n\
        \x20   Send 2 into ch.\n\
        \x20   Send 3 into ch.\n\
        \n\
        ## Main\n\
        \x20   Let jobs be a Pipe of Int.\n\
        \x20   Launch a task to produce with jobs.\n\
        \x20   Receive a from jobs.\n\
        \x20   Receive b from jobs.\n\
        \x20   Receive c from jobs.\n\
        \x20   Show a.\n\
        \x20   Show b.\n\
        \x20   Show c.\n",
    );
}

#[test]
fn diff_launch_fire_and_forget() {
    assert_tiers_agree(
        "## To worker:\n\
        \x20   Show \"worker ran\".\n\
        \n\
        ## Main\n\
        \x20   Launch a task to worker.\n\
        \x20   Show \"main ran\".\n",
    );
}

#[test]
fn diff_try_receive_empty() {
    assert_tiers_agree(
        "## Main\n\
        \x20   Let ch be a Pipe of Int.\n\
        \x20   Try to receive x from ch.\n\
        \x20   Show x.\n",
    );
}

#[test]
fn diff_pass_argument_to_task() {
    // The spawned task receives an argument (the channel) AND a scalar, exercising
    // the multi-arg spawn register window on both tiers.
    assert_tiers_agree(
        "## To emit (ch: Int, n: Int):\n\
        \x20   Send n into ch.\n\
        \n\
        ## Main\n\
        \x20   Let ch be a Pipe of Int.\n\
        \x20   Launch a task to emit with ch and 42.\n\
        \x20   Receive got from ch.\n\
        \x20   Show got.\n",
    );
}

#[test]
fn diff_select_receive_wins() {
    assert_tiers_agree(
        "## To produce (ch: Int):\n\
        \x20   Send 7 into ch.\n\
        \n\
        ## Main\n\
        \x20   Let ch be a Pipe of Int.\n\
        \x20   Launch a task to produce with ch.\n\
        \x20   Await the first of:\n\
        \x20       Receive x from ch:\n\
        \x20           Show x.\n\
        \x20       After 1 seconds:\n\
        \x20           Show \"timeout\".\n",
    );
}

#[test]
fn diff_select_timeout_only() {
    assert_tiers_agree(
        "## Main\n\
        \x20   Let p be a Pipe of Text.\n\
        \x20   Await the first of:\n\
        \x20       After 1 seconds:\n\
        \x20           Show \"timeout\".\n",
    );
}

#[test]
fn diff_select_both_ready_seed_sweep() {
    // Both arms are ready when the select runs (main buffers into both pipes
    // first), so the WINNER is the seed's choice. Across the seed sweep, the
    // tree-walker and the VM must pick the SAME arm under each seed (one shared
    // deterministic chooser), and the same seed must reproduce its winner.
    let src = "## Main\n\
        \x20   Let a be a Pipe of Int.\n\
        \x20   Let b be a Pipe of Int.\n\
        \x20   Send 1 into a.\n\
        \x20   Send 2 into b.\n\
        \x20   Await the first of:\n\
        \x20       Receive x from a:\n\
        \x20           Show x.\n\
        \x20       Receive y from b:\n\
        \x20           Show y.\n";
    for seed in [0u64, 1, 2, 7, 42] {
        let tw = run_treewalker_concurrent_seeded(src, seed);
        let vm = run_vm_concurrent_seeded(src, seed);
        assert!(tw.error.is_none(), "tree-walker error at seed {seed}: {:?}", tw.error);
        assert!(vm.error.is_none(), "vm error at seed {seed}: {:?}", vm.error);
        assert_eq!(
            tw.lines, vm.lines,
            "tier divergence at seed {seed}: tree-walker {:?} vs vm {:?}",
            tw.lines, vm.lines
        );
        assert!(
            tw.lines == vec!["1".to_string()] || tw.lines == vec!["2".to_string()],
            "seed {seed} winner is one of the ready arms, got {:?}",
            tw.lines
        );
        // Same seed reproduces its winner.
        assert_eq!(run_vm_concurrent_seeded(src, seed).lines, vm.lines, "seed {seed} reproducible");
    }
}

#[test]
fn diff_deadlock_agreement() {
    // Both tiers must detect the same deadlock (receive with no sender).
    assert_tiers_agree(
        "## Main\n\
        \x20   Let ch be a Pipe of Int.\n\
        \x20   Receive x from ch.\n\
        \x20   Show x.\n",
    );
}

// ─── Phase 7 — cooperative (M:1) vs work-stealing (M:N) differential ─────────
//
// The work-stealing driver polls task bodies on `W` OS-thread workers while one
// coordinator owns the single scheduler + chooser and applies channel ops +
// flushes output in deterministic *pick order*. That construction makes its
// observable output byte-identical to the cooperative single-thread driver at
// the same seed — for ANY worker count. Genuine multicore, zero nondeterminism.

/// The determinate concurrency corpus, reused from the cross-tier cases above.
/// Each must produce identical output under both drivers (a deadlock program
/// must report the same error under both).
const WS_CORPUS: &[(&str, &str)] = &[
    (
        "producer_consumer",
        "## To produce (ch: Int):\n\
        \x20   Send 1 into ch.\n\
        \x20   Send 2 into ch.\n\
        \x20   Send 3 into ch.\n\
        \n\
        ## Main\n\
        \x20   Let jobs be a Pipe of Int.\n\
        \x20   Launch a task to produce with jobs.\n\
        \x20   Receive a from jobs.\n\
        \x20   Receive b from jobs.\n\
        \x20   Receive c from jobs.\n\
        \x20   Show a.\n\
        \x20   Show b.\n\
        \x20   Show c.\n",
    ),
    (
        "launch_fire_and_forget",
        "## To worker:\n\
        \x20   Show \"worker ran\".\n\
        \n\
        ## Main\n\
        \x20   Launch a task to worker.\n\
        \x20   Show \"main ran\".\n",
    ),
    (
        "try_receive_empty",
        "## Main\n\
        \x20   Let ch be a Pipe of Int.\n\
        \x20   Try to receive x from ch.\n\
        \x20   Show x.\n",
    ),
    (
        "pass_argument_to_task",
        "## To emit (ch: Int, n: Int):\n\
        \x20   Send n into ch.\n\
        \n\
        ## Main\n\
        \x20   Let ch be a Pipe of Int.\n\
        \x20   Launch a task to emit with ch and 42.\n\
        \x20   Receive got from ch.\n\
        \x20   Show got.\n",
    ),
    (
        "select_receive_wins",
        "## To produce (ch: Int):\n\
        \x20   Send 7 into ch.\n\
        \n\
        ## Main\n\
        \x20   Let ch be a Pipe of Int.\n\
        \x20   Launch a task to produce with ch.\n\
        \x20   Await the first of:\n\
        \x20       Receive x from ch:\n\
        \x20           Show x.\n\
        \x20       After 1 seconds:\n\
        \x20           Show \"timeout\".\n",
    ),
    (
        "select_both_ready",
        "## Main\n\
        \x20   Let a be a Pipe of Int.\n\
        \x20   Let b be a Pipe of Int.\n\
        \x20   Send 1 into a.\n\
        \x20   Send 2 into b.\n\
        \x20   Await the first of:\n\
        \x20       Receive x from a:\n\
        \x20           Show x.\n\
        \x20       Receive y from b:\n\
        \x20           Show y.\n",
    ),
    (
        "fan_out_three_workers",
        "## To emit (ch: Int, n: Int):\n\
        \x20   Send n into ch.\n\
        \n\
        ## Main\n\
        \x20   Let ch be a Pipe of Int.\n\
        \x20   Launch a task to emit with ch and 10.\n\
        \x20   Launch a task to emit with ch and 20.\n\
        \x20   Launch a task to emit with ch and 30.\n\
        \x20   Receive a from ch.\n\
        \x20   Receive b from ch.\n\
        \x20   Receive c from ch.\n\
        \x20   Show a.\n\
        \x20   Show b.\n\
        \x20   Show c.\n",
    ),
    (
        "deadlock",
        "## Main\n\
        \x20   Let ch be a Pipe of Int.\n\
        \x20   Receive x from ch.\n\
        \x20   Show x.\n",
    ),
];

#[test]
fn diff_cooperative_eq_workstealing_seeded() {
    // For every corpus program × seed × worker count, the work-stealing output
    // must match the cooperative VM output at the same seed exactly (including
    // error agreement). The worker count must NOT change the observable result.
    for (name, src) in WS_CORPUS {
        for seed in [0u64, 1, 2, 7, 42] {
            let coop = run_vm_concurrent_seeded(src, seed);
            for workers in [1usize, 2, 4, 8] {
                let ws = run_vm_workstealing_seeded(src, seed, workers);
                assert_eq!(
                    coop.error.is_none(),
                    ws.error.is_none(),
                    "[{name}] error-agreement diverged at seed {seed}, {workers} workers\n\
                     cooperative error: {:?}\n  work-stealing error: {:?}",
                    coop.error,
                    ws.error,
                );
                assert_eq!(
                    coop.lines, ws.lines,
                    "[{name}] output diverged at seed {seed}, {workers} workers\n\
                     cooperative: {:?}\n  work-stealing: {:?}",
                    coop.lines, ws.lines,
                );
            }
        }
    }
}

#[test]
fn workstealing_is_seed_and_worker_reproducible() {
    // The same seed reproduces identical output regardless of worker count — the
    // determinism contract the M:N driver must uphold.
    let src = WS_CORPUS[0].1;
    let baseline = run_vm_workstealing_seeded(src, 7, 4).lines;
    for workers in [1usize, 2, 3, 4, 8, 16] {
        assert_eq!(
            run_vm_workstealing_seeded(src, 7, workers).lines,
            baseline,
            "work-stealing output changed with {workers} workers (must be worker-count-invariant)",
        );
    }
}

#[test]
fn diff_stream_into_pipe_is_cross_tier() {
    // The `Stream … into <pipe>` knob lowers to the cross-tier channel send (`SendPipe`), so
    // streaming a batch through an in-process pipe runs IDENTICALLY on the tree-walker and the
    // bytecode VM — the cross-tier pro of the streaming surface (the `to <peer>` knob is the
    // network pro, on the async tree-walker tier).
    assert_tiers_agree(
        "## To produce (ch: List of Int):\n\
        \x20   Stream [1, 2, 3] into ch.\n\
        \n\
        ## Main\n\
        \x20   Let jobs be a Pipe of List of Int.\n\
        \x20   Launch a task to produce with jobs.\n\
        \x20   Receive batch from jobs.\n\
        \x20   Show batch.\n",
    );
}
