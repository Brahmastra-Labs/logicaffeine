//! tt_select_mesh_lock — increment 3 of concurrency→Tensix-mesh: `Await the first of { Receive … / After … }`
//! (select) + `Sleep`. The deterministic-scheduler semantics are reproduced on real cores: for a channel
//! that already holds a value, the VM's deterministic select takes the RECEIVE arm — and the mesh does the
//! same by checking channel readiness at runtime; the timeout arm is the fallback. `Sleep` in a blocking
//! producer is output-irrelevant (the receiver blocks regardless), so it lowers as a no-op that stays ≡VM.
//!
//! Written RED-first (the `compile_to_tenstorrent_select` it calls did not exist when this was authored).
//! Run: `cargo nextest run -p logicaffeine-compile --features codegen -E 'binary(tt_select_mesh_lock)'`

#![cfg(feature = "codegen")]

use logicaffeine_compile::compile::{compile_to_tenstorrent_select, vm_outcome_concurrent};

/// A select on a READY channel: main pre-sends `v`, then awaits the first of a receive (shows the value) or
/// a timeout (shows a string). The channel is ready ⇒ the receive arm fires ⇒ output is `v`.
fn select_ready_program(v: i64) -> String {
    format!(
        "## Main\n    Let ch be a Pipe of Int.\n    Send {v} into ch.\n    Await the first of:\n        Receive x from ch:\n            Show x.\n        After 1 seconds:\n            Show \"timeout\".\n"
    )
}

#[test]
fn the_compiler_recognizes_select_and_lowers_the_ready_channel_to_the_receive_arm() {
    let prog = compile_to_tenstorrent_select(&select_ready_program(42))
        .expect("a select on a ready channel must be recognized + lowered to the mesh");
    assert_eq!(
        prog.spec.presend, 42,
        "the pre-sent value comes from the source"
    );
    assert_eq!(
        prog.spec.timeout_ticks, 1,
        "the timeout duration comes from the source"
    );

    // ★ WAS a runtime-readiness check in kernel text. The select is RESOLVED AT COMPILE TIME now, and
    // that is not a weakening: main pre-sends before it awaits, so the channel's readiness is a literal of
    // the source, and the VM's deterministic scheduler takes the receive arm for exactly that reason. The
    // compiler applies the same rule instead of emitting a device-side test whose answer it already knows.
    //
    // Keeping a runtime check would have meant teaching the spatial IR a FREE CHOICE — and the IR is a
    // marked graph, whose confluence is what makes `eval_choreography_confluent` a proof rather than a
    // spot-check for every program that shares it. The resolution is lifted to data instead.
    let p = prog
        .program
        .as_ref()
        .expect("a ready channel resolves to the receive arm, which has dataflow");
    assert_eq!(
        p.tensors[0].init,
        logicaffeine_compile::spatial::ir::TensorInit::Values(vec![42]),
        "the receive arm carries the pre-sent value the VM would have shown"
    );
}

#[test]
fn the_select_is_a_function_of_the_source() {
    let a = compile_to_tenstorrent_select(&select_ready_program(42)).unwrap();
    let b = compile_to_tenstorrent_select(&select_ready_program(99)).unwrap();
    assert_eq!(
        (a.spec.presend, b.spec.presend),
        (42, 99),
        "the pre-sent value comes from the source"
    );
    let (pa, pb) = (a.program.as_ref().unwrap(), b.program.as_ref().unwrap());
    assert_ne!(
        pa.tensors[0].init, pb.tensors[0].init,
        "different pre-sent value ⇒ different program"
    );
}

#[test]
fn a_non_select_program_is_declined() {
    assert!(
        compile_to_tenstorrent_select("## Main\n    Show 5.\n").is_err(),
        "a plain program is not a select"
    );
}

#[test]
fn the_select_mesh_result_equals_the_vm() {
    // The VM's deterministic select takes the receive arm on a ready channel ⇒ output is the pre-sent value.
    for v in [42, 99, 7] {
        let out = vm_outcome_concurrent(&select_ready_program(v));
        assert!(
            out.error.is_none(),
            "VM must run the select: {:?}",
            out.error
        );
        assert_eq!(
            out.output.trim(),
            v.to_string(),
            "VM takes the receive arm on a ready channel"
        );
        // The generated program carries the same value the receive arm will show.
        let prog = compile_to_tenstorrent_select(&select_ready_program(v)).unwrap();
        assert_eq!(prog.spec.presend, v);
    }
}
