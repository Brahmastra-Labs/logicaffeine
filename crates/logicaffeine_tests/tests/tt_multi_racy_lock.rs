//! tt_multi_racy_lock — increment 7 of concurrency→Tensix-mesh: MULTI-TASK RACES. N tasks each sleep then
//! send into a shared pipe; main awaits the first of {receive, timeout}. The VM's deterministic scheduler
//! fires the GLOBAL-minimum-virtual-time event — so a coordinator core collects every task's (value,
//! send-vtime), picks the MIN send-vtime, and compares it to the timeout. Independent of real timing ⇒ ≡VM.
//!
//! ★ MIGRATED OFF THE HAND-WRITTEN EMITTER (P5): the coordinator's min-vtime scan is a compile-time
//! comparison of source literals, so it is the compiler's job and the assertions are about the arm it
//! resolves to. See `tt_racy_select_lock` for the same move on the single-task race.
//!
//! Written RED-first (`compile_to_tenstorrent_multi_racy` did not exist when authored).
//! Run: `cargo nextest run -p logicaffeine-compile --features codegen -E 'binary(tt_multi_racy_lock)'`

#![cfg(feature = "codegen")]

use logicaffeine_compile::compile::{compile_to_tenstorrent_multi_racy, vm_outcome_concurrent};

/// Two racing tasks (`fast` sleeps `s1` sends `v1`, `slow` sleeps `s2` sends `v2`) + a `timeout_s`-sec
/// timeout. The min-vtime event fires: the earlier send if it beats the timeout, else the timeout.
fn multi_program(s1: i64, v1: i64, s2: i64, v2: i64, timeout_s: i64) -> String {
    format!("## To fast (ch: Int):\n    Sleep {s1}ms.\n    Send {v1} into ch.\n## To slow (ch: Int):\n    Sleep {s2}ms.\n    Send {v2} into ch.\n## Main\n    Let ch be a Pipe of Int.\n    Launch a task to fast with ch.\n    Launch a task to slow with ch.\n    Await the first of:\n        Receive x from ch:\n            Show x.\n        After {timeout_s} seconds:\n            Show \"timeout\".\n")
}

#[test]
fn the_compiler_recognizes_multiple_racing_tasks() {
    let prog = compile_to_tenstorrent_multi_racy(&multi_program(300, 11, 800, 22, 1))
        .expect("a multi-task race must lower to the mesh");
    assert_eq!(
        prog.spec.tasks,
        vec![(11, 300), (22, 800)],
        "(value, send-vtime) per task, from the source"
    );
    assert_eq!(prog.spec.timeout_ms, 1000);
}

#[test]
fn the_coordinator_picks_the_global_min_virtual_time() {
    let prog = compile_to_tenstorrent_multi_racy(&multi_program(300, 11, 800, 22, 1)).unwrap();
    // ★ THE RULE MOVED FROM A KERNEL INTO THE COMPILER. Picking the globally smallest send-vtime and
    // comparing it to the timeout is the VM's scheduler rule, and every term is a literal of the source —
    // so a coordinator that scanned for the minimum on-device was recomputing a constant. What is asserted
    // now is the ANSWER that rule produces.
    let (min_v, min_vt) = *prog.spec.tasks.iter().min_by_key(|(_, vt)| *vt).unwrap();
    assert_eq!(
        (min_v, min_vt),
        (11, 300),
        "the earliest send is task 1's, at 300ms"
    );
    assert!(
        min_vt < prog.spec.timeout_ms,
        "and it beats the timeout, so the receive arm wins"
    );
    let p = prog.program.as_ref().expect("the winning arm has dataflow");
    assert_eq!(
        p.tensors[0].init,
        logicaffeine_compile::spatial::ir::TensorInit::Values(vec![11]),
        "the lowered program carries the WINNER's value, not every racer's"
    );
}

#[test]
fn both_outcomes_equal_the_vm() {
    // FAST wins: min send-vtime 300 < timeout 1000 ⇒ that task's value (11).
    let fast = vm_outcome_concurrent(&multi_program(300, 11, 800, 22, 1));
    assert_eq!(fast.output.trim(), "11");
    let fp = compile_to_tenstorrent_multi_racy(&multi_program(300, 11, 800, 22, 1)).unwrap();
    let min_vt = fp.spec.tasks.iter().map(|t| t.1).min().unwrap();
    assert!(
        min_vt < fp.spec.timeout_ms,
        "compiler agrees the earliest send beats the timeout"
    );

    // TIMEOUT wins: both sends (1200, 1500) after timeout 1000 ⇒ "timeout".
    let to = vm_outcome_concurrent(&multi_program(1200, 11, 1500, 22, 1));
    assert_eq!(to.output.trim(), "timeout");
    let tp = compile_to_tenstorrent_multi_racy(&multi_program(1200, 11, 1500, 22, 1)).unwrap();
    assert!(
        tp.spec.tasks.iter().map(|t| t.1).min().unwrap() > tp.spec.timeout_ms,
        "compiler agrees: timeout"
    );
}
