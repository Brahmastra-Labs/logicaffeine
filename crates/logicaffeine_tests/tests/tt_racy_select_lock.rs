//! tt_racy_select_lock — increment 4 of concurrency→Tensix-mesh: the RACY select where the timeout can
//! actually fire. A task sleeps, then sends; main awaits the first of {receive, timeout}. The VM's
//! DETERMINISTIC virtual-time scheduler fires whichever event has the smaller virtual time — the send
//! (at vtime = the task's cumulative sleep) or the timeout. So this is NOT a real-time race: a COORDINATOR
//! core reproduces the outcome by COMPARING VIRTUAL TIMES (the reported send-vtime vs the timeout), which is
//! independent of real hardware timing — hence ≡VM on both branches.
//!
//! Written RED-first (`compile_to_tenstorrent_racy_select` did not exist when authored).
//! Run: `cargo nextest run -p logicaffeine-compile --features codegen -E 'binary(tt_racy_select_lock)'`

#![cfg(feature = "codegen")]

use logicaffeine_compile::compile::{compile_to_tenstorrent_racy_select, vm_outcome_concurrent};

/// A racy select: a task sleeps `sleep_ms`, then sends `v`; main awaits the first of a receive or a
/// `timeout_s`-second timeout. The receive fires iff `sleep_ms < timeout_s*1000` (VM virtual-time order).
fn racy_program(sleep_ms: i64, timeout_s: i64, v: i64) -> String {
    format!("## To slow (ch: Int):\n    Sleep {sleep_ms}ms.\n    Send {v} into ch.\n## Main\n    Let ch be a Pipe of Int.\n    Launch a task to slow with ch.\n    Await the first of:\n        Receive x from ch:\n            Show x.\n        After {timeout_s} seconds:\n            Show \"timeout\".\n")
}

#[test]
fn the_compiler_recognizes_the_racy_select_and_extracts_the_virtual_times() {
    let fast = compile_to_tenstorrent_racy_select(&racy_program(500, 1, 42))
        .expect("racy select must lower");
    assert_eq!(fast.spec.value, 42, "the sent value comes from the source");
    assert_eq!(
        fast.spec.send_vtime_ms, 500,
        "send virtual time = the task's cumulative sleep (ms)"
    );
    assert_eq!(fast.spec.timeout_ms, 1000, "timeout normalized to ms");
}

#[test]
fn the_coordinator_compares_virtual_times_not_real_time() {
    // ★ THE COORDINATOR IS GONE AND THE COMPARISON REMAINS — it moved from a device kernel into the
    // compiler, which is where it always belonged. This was never a real-time race: the VM fires whichever
    // event has the smaller VIRTUAL time, and both times are literals of the source. A kernel that compared
    // them on-device was recomputing a constant.
    //
    // So the assertion is the RESOLUTION itself: the send wins, therefore the program is the one-value pipe
    // the receive arm feeds.
    let fast = compile_to_tenstorrent_racy_select(&racy_program(500, 1, 42)).unwrap();
    assert!(
        fast.spec.send_vtime_ms < fast.spec.timeout_ms,
        "the send's virtual time beats the timeout"
    );
    let p = fast
        .program
        .as_ref()
        .expect("the receive arm has dataflow: the sent value reaches main");
    assert_eq!(
        p.tensors[0].init,
        logicaffeine_compile::spatial::ir::TensorInit::Values(vec![42]),
        "the lowered program carries the value the receive arm shows"
    );
    // And the other side of the resolution: when the timeout wins there is no dataflow to place, because
    // the answer is a constant string. A `None` here is the honest artifact, not a missing lowering.
    let slow = compile_to_tenstorrent_racy_select(&racy_program(2000, 1, 42)).unwrap();
    assert!(
        slow.program.is_none(),
        "the timeout arm's answer is a constant — nothing to place"
    );
}

#[test]
fn the_racy_program_is_a_function_of_the_source() {
    let a = compile_to_tenstorrent_racy_select(&racy_program(500, 1, 42)).unwrap();
    let b = compile_to_tenstorrent_racy_select(&racy_program(2000, 1, 42)).unwrap();
    assert_eq!(b.spec.send_vtime_ms, 2000);
    // Different sleep ⇒ different program, and here the difference is the WHOLE ARM: 500ms resolves to the
    // receive, 2000ms to the timeout.
    assert!(
        a.program.is_some() && b.program.is_none(),
        "the sleep decides which arm the compiler lowers"
    );
}

#[test]
fn both_branches_equal_the_vm() {
    // FAST: send-vtime 500 < timeout 1000 ⇒ receive fires ⇒ 42.
    let fast_vm = vm_outcome_concurrent(&racy_program(500, 1, 42));
    assert_eq!(fast_vm.output.trim(), "42");
    let fast = compile_to_tenstorrent_racy_select(&racy_program(500, 1, 42)).unwrap();
    assert!(
        fast.spec.send_vtime_ms < fast.spec.timeout_ms,
        "compiler agrees: receive arm fires"
    );
    assert!(fast.program.is_some(), "and it lowered that arm");

    // SLOW: send-vtime 2000 > timeout 1000 ⇒ timeout fires ⇒ "timeout".
    let slow_vm = vm_outcome_concurrent(&racy_program(2000, 1, 42));
    assert_eq!(slow_vm.output.trim(), "timeout");
    let slow = compile_to_tenstorrent_racy_select(&racy_program(2000, 1, 42)).unwrap();
    assert!(
        slow.spec.send_vtime_ms > slow.spec.timeout_ms,
        "compiler agrees: timeout arm fires"
    );
    assert!(
        slow.program.is_none(),
        "and it lowered no dataflow, because the answer is a constant"
    );
}
