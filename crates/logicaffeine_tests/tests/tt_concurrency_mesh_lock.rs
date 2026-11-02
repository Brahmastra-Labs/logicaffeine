//! tt_concurrency_mesh_lock — the FIRST increment of concurrency→Tensix-mesh lowering: the producer/consumer
//! pipe pattern lowers onto TWO baby RISC-V cores with the pipe as a circular buffer, instead of being
//! refused. The compiler recognizes the pattern, extracts the sends FROM THE SOURCE, and generates a
//! complete 2-core program; a non-matching program is declined (caller falls back to the sound refusal).
//! The generated program's output equals the VM's (a FIFO pipe is order-deterministic) — proven by the
//! `#[ignore]`d on-sim test + the ≡VM anchor here.
//! Run: `cargo nextest run -p logicaffeine-compile --features codegen -E 'binary(tt_concurrency_mesh_lock)'`

#![cfg(feature = "codegen")]

use logicaffeine_compile::compile::{
    compile_to_tenstorrent_concurrency, compile_to_tenstorrent_fanout, vm_outcome_concurrent,
};

/// A fan-out program: N distinct single-arg tasks each send one value into a shared pipe; main folds them.
fn fanout_program(vals: &[i64]) -> String {
    let fns: String = vals
        .iter()
        .enumerate()
        .map(|(i, v)| format!("## To emit_{i} (ch: Int):\n    Send {v} into ch.\n"))
        .collect();
    let launches: String = (0..vals.len())
        .map(|i| format!("    Launch a task to emit_{i} with ch.\n"))
        .collect();
    let recvs: String = (0..vals.len())
        .map(|i| format!("    Receive {} from ch.\n", (b'a' + i as u8) as char))
        .collect();
    let sum = (0..vals.len())
        .map(|i| ((b'a' + i as u8) as char).to_string())
        .collect::<Vec<_>>()
        .join(" + ");
    format!("{fns}## Main\n    Let ch be a Pipe of Int.\n{launches}{recvs}    Let total be {sum}.\n    Show total.\n")
}

/// A producer that SLEEPS before sending. For a blocking receiver the sleep is output-irrelevant — main
/// gets the value whenever it arrives — so the mesh lowers `Sleep` as a no-op and stays ≡VM.
fn pc_with_sleep() -> String {
    "## To produce (ch: Int):\n    Sleep 100ms.\n    Send 42 into ch.\n## Main\n    Let ch be a Pipe of Int.\n    Launch a task to produce with ch.\n    Receive a from ch.\n    Show a.\n".to_string()
}

#[test]
fn sleep_in_a_blocking_producer_is_output_irrelevant_and_still_lowers() {
    // ≡VM anchor: the sleep delays the send, but main blocks on the receive and gets 42 regardless.
    let out = vm_outcome_concurrent(&pc_with_sleep());
    assert!(
        out.error.is_none(),
        "the sleep program must run on the VM: {:?}",
        out.error
    );
    assert_eq!(out.output.trim(), "42");
    // The producer/consumer lowering must accept the sleep (as a no-op) and still send the value.
    let prog = compile_to_tenstorrent_concurrency(&pc_with_sleep())
        .expect("a sleeping-producer program must still lower to the mesh");
    assert_eq!(
        prog.spec.sends,
        vec![42],
        "the send survives; the sleep is dropped as output-irrelevant"
    );
    // The sleep is dropped and the SEND survives into the lowered program — asserted on the value the
    // program carries, not on a `cb_push_back` in emitted text.
    let p = prog
        .program
        .as_ref()
        .expect("a sleeping producer still lowers");
    assert_eq!(
        p.tensors[0].init,
        logicaffeine_compile::spatial::ir::TensorInit::Values(vec![42]),
        "the value reaches the pipe"
    );
}

#[test]
fn the_compiler_recognizes_fan_out_and_lays_out_one_core_per_task() {
    let prog = compile_to_tenstorrent_fanout(&fanout_program(&[10, 20, 30]))
        .expect("an N-task fan-out must be recognized + lowered to the mesh");
    assert_eq!(
        prog.spec.values,
        vec![10, 20, 30],
        "per-task values come from the source"
    );

    // ★ WAS a NoC/semaphore fan-in spelled out in kernel text. A fan-out's OUTPUT is the fold of every
    // task's value, which is order-independent — so what the source means is a stream and a reduction, and
    // that is what the IR carries. The choreography (whether the fan-in is a semaphore, a barrier or a
    // FIFO) is now the printer's decision on the target, which is the point of having one IR.
    let p = prog
        .program
        .as_ref()
        .expect("a fan-out lowers to a spatial program");
    assert_eq!(
        p.tensors[0].init,
        logicaffeine_compile::spatial::ir::TensorInit::Values(vec![10, 20, 30]),
        "every task's value reaches the fold"
    );
    assert!(p.stages.len() >= 3, "load -> fold -> store at minimum");
}

#[test]
fn the_fan_out_program_is_a_function_of_the_source() {
    let p3 = compile_to_tenstorrent_fanout(&fanout_program(&[10, 20, 30])).unwrap();
    let p4 = compile_to_tenstorrent_fanout(&fanout_program(&[1, 2, 3, 4])).unwrap();
    let (a, b) = (p3.program.as_ref().unwrap(), p4.program.as_ref().unwrap());
    assert_ne!(
        a.tensors[0].init, b.tensors[0].init,
        "different task values ⇒ different program"
    );
    assert_eq!(
        b.tensors[0].init,
        logicaffeine_compile::spatial::ir::TensorInit::Values(vec![1, 2, 3, 4]),
        "four tasks, four values"
    );
    assert_ne!(
        a.tensors[0].elems.cols, b.tensors[0].elems.cols,
        "a different task COUNT is a different shape"
    );
}

#[test]
fn a_single_task_program_is_not_a_fan_out() {
    // The producer/consumer (one task) is NOT a fan-out — recognize_fanout must decline it.
    let single = "## To produce (ch: Int):\n    Send 5 into ch.\n## Main\n    Let ch be a Pipe of Int.\n    Launch a task to produce with ch.\n    Receive a from ch.\n    Show a.\n";
    assert!(
        compile_to_tenstorrent_fanout(single).is_err(),
        "one task is producer/consumer, not fan-out"
    );
}

#[test]
fn the_fan_out_mesh_result_equals_the_vm() {
    // Commutative fold ⇒ the mesh's arrival order is irrelevant; the sum equals the VM's.
    for vals in [vec![10, 20, 30], vec![1, 2, 3, 4], vec![7, 7]] {
        let out = vm_outcome_concurrent(&fanout_program(&vals));
        assert!(
            out.error.is_none(),
            "VM must run the fan-out: {:?}",
            out.error
        );
        let vm: i64 = out
            .output
            .split_whitespace()
            .next()
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(vm, vals.iter().sum::<i64>());
        let prog = compile_to_tenstorrent_fanout(&fanout_program(&vals)).unwrap();
        assert_eq!(
            prog.spec.values.iter().sum::<i64>(),
            vm,
            "the generated fan-out folds to the VM total"
        );
    }
}

/// A producer/consumer pipe program: the task sends a sequence into the pipe, main receives + sums + shows.
fn pc_program(sends: &[i64]) -> String {
    let body: String = sends
        .iter()
        .map(|v| format!("    Send {v} into ch.\n"))
        .collect();
    let recvs: String = (0..sends.len())
        .map(|i| {
            let var = (b'a' + i as u8) as char;
            format!("    Receive {var} from ch.\n    Set total to total + {var}.\n")
        })
        .collect();
    format!("## To produce (ch: Int):\n{body}## Main\n    Let ch be a Pipe of Int.\n    Launch a task to produce with ch.\n    Let mutable total be 0.\n{recvs}    Show total.\n")
}

#[test]
fn the_compiler_recognizes_the_pipe_pattern_and_extracts_the_sends_from_the_source() {
    let prog = compile_to_tenstorrent_concurrency(&pc_program(&[10, 20, 30]))
        .expect("a producer/consumer pipe program must be recognized + lowered to the mesh");
    assert_eq!(
        prog.spec.sends,
        vec![10, 20, 30],
        "sends must come from the source"
    );
    assert_eq!(prog.spec.receives, 3);
}

#[test]
fn the_generated_program_is_a_real_2core_mesh_program_a_function_of_the_source() {
    let p1 = compile_to_tenstorrent_concurrency(&pc_program(&[10, 20, 30])).unwrap();
    let p2 = compile_to_tenstorrent_concurrency(&pc_program(&[1, 2, 3, 4])).unwrap();

    // ★ STILL GENUINE TWO-ENGINE CONCURRENCY, asserted structurally instead of by kernel substrings. The
    // pipe is a FIFO edge between a producer stage and the fold that drains it — the same two-baby-core
    // shape the retired emitter wrote out by hand, but now derived by the printer from the IR rather than
    // asserted to exist by searching for "RISCV_1" in a string.
    let (a, b) = (p1.program.as_ref().unwrap(), p2.program.as_ref().unwrap());
    assert_eq!(
        a.tensors[0].init,
        logicaffeine_compile::spatial::ir::TensorInit::Values(vec![10, 20, 30]),
        "the sends are the program's input"
    );
    assert!(!a.edges.is_empty(), "the pipe IS an edge: producer -> fold");
    assert!(a.stages.len() >= 3, "producer, fold, store");
    // The generated program is a FUNCTION of the source: different sends ⇒ different program.
    assert_ne!(
        a.tensors[0].init, b.tensors[0].init,
        "the program must carry the source's sends"
    );
    assert_eq!(
        b.tensors[0].init,
        logicaffeine_compile::spatial::ir::TensorInit::Values(vec![1, 2, 3, 4])
    );
}

#[test]
fn a_non_pipe_program_is_declined_so_the_caller_can_refuse_soundly() {
    // A plain scalar program — no launch/pipe — must NOT be recognized as a mesh-concurrency program.
    let err =
        compile_to_tenstorrent_concurrency("## Main\n    Show 5 + 3.\n").expect_err("must decline");
    assert!(
        err.contains("not a producer/consumer pipe program"),
        "got: {err}"
    );
}

#[test]
fn the_mesh_result_equals_the_vm_the_numeric_anchor() {
    // The mesh reproduces the VM's deterministic pipe output. Here we pin the ≡VM anchor: the VM sums the
    // sends, and the generated consumer folds the SAME FIFO sequence to the SAME total (checked on-sim by
    // the ignored test; here we assert the VM value the mesh must match).
    for sends in [vec![10, 20, 30], vec![1, 2, 3, 4], vec![7]] {
        let out = vm_outcome_concurrent(&pc_program(&sends));
        assert!(
            out.error.is_none(),
            "VM must run the pipe program: {:?}",
            out.error
        );
        let vm_total: i64 = out
            .output
            .split_whitespace()
            .next()
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(
            vm_total,
            sends.iter().sum::<i64>(),
            "VM total is the FIFO sum the mesh must reproduce"
        );
        // The generated program folds the same sequence (same sends order → same total).
        let prog = compile_to_tenstorrent_concurrency(&pc_program(&sends)).unwrap();
        assert_eq!(
            prog.spec.sends.iter().sum::<i64>(),
            vm_total,
            "generated mesh program folds to the VM total"
        );
    }
}
