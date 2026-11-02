//! tt_dynamic_mesh_lock — increments 5+6 of concurrency→Tensix-mesh: DYNAMIC CHANNELS (a loop-driven
//! producer feeds the pipe) + COMPUTED SEND VALUES (each send is an expression of the loop variable). The
//! producer runs on its own baby RISC-V and pushes `f(i)` for `i` over a range; the pipe is a bounded
//! circular buffer, so producer and consumer interleave (real bounded-channel concurrency); the consumer
//! folds the stream. The bounded FIFO's output is order-deterministic ⇒ ≡VM.
//!
//! ★ MIGRATED OFF THE HAND-WRITTEN EMITTER (P5). The kernel-text assertions here became assertions about
//! the lowered PROGRAM: the loop's values, the tensor init they become, and the fold. That is stronger
//! than the text it replaces — a producer that printed `i * i` and sent the wrong numbers passed the old
//! form, and cannot pass this one.
//!
//! Written RED-first (`compile_to_tenstorrent_dynamic` did not exist when authored).
//! Run: `cargo nextest run -p logicaffeine-compile --features codegen -E 'binary(tt_dynamic_mesh_lock)'`

#![cfg(feature = "codegen")]

use logicaffeine_compile::compile::{compile_to_tenstorrent_dynamic, vm_outcome_concurrent};

/// A loop-driven producer: `Repeat for i from lo to hi: Send <send_expr> into ch`; main folds the stream.
fn loop_program(lo: i64, hi: i64, send_expr: &str) -> String {
    let n = (hi - lo + 1) as usize;
    let recvs: String = (0..n)
        .map(|i| format!("    Receive {} from ch.\n", (b'a' + i as u8) as char))
        .collect();
    let sum = (0..n)
        .map(|i| ((b'a' + i as u8) as char).to_string())
        .collect::<Vec<_>>()
        .join(" + ");
    format!("## To produce (ch: Int):\n    Repeat for i from {lo} to {hi}:\n        Send {send_expr} into ch.\n## Main\n    Let ch be a Pipe of Int.\n    Launch a task to produce with ch.\n{recvs}    Let total be {sum}.\n    Show total.\n")
}

#[test]
fn the_compiler_recognizes_a_dynamic_loop_producer() {
    let prog = compile_to_tenstorrent_dynamic(&loop_program(1, 3, "i"))
        .expect("a loop-driven producer must lower to the mesh");
    assert_eq!(
        (prog.spec.lo, prog.spec.hi),
        (1, 3),
        "the loop bounds come from the source"
    );
    // ★ WAS a kernel-text assertion (`producer_cpp` contains "for (" + "cb_push_back"). The pattern now
    // LOWERS through the spatial IR, so the stream is a fact about the program rather than a substring of
    // emitted C++: the loop's values are what the producer feeds the pipe.
    assert_eq!(
        prog.spec.values,
        vec![1, 2, 3],
        "the stream is f(i) over the source's bounds"
    );
    let p = prog
        .program
        .as_ref()
        .expect("a loop producer lowers to a spatial program");
    assert!(
        p.stages.len() >= 3,
        "load -> fold -> store at minimum, got {}",
        p.stages.len()
    );
    assert!(
        !p.edges.is_empty(),
        "the pipe is an edge between the producer and the fold"
    );
}

#[test]
fn computed_send_values_are_lowered_to_the_producer() {
    // `Send i * i into ch` — the sent value is a COMPUTED expression of the loop variable.
    let prog = compile_to_tenstorrent_dynamic(&loop_program(1, 3, "i * i")).unwrap();
    // ★ STRICTLY STRONGER THAN THE TEXT IT REPLACES. The old assertion checked that the STRING "i * i"
    // appeared in the emitted kernel, which a producer that printed the expression and then sent the wrong
    // numbers would have passed. What matters is the stream: i*i over 1..=3 is 1, 4, 9.
    assert_eq!(
        prog.spec.values,
        vec![1, 4, 9],
        "the computed send expression is EVALUATED, not just printed"
    );
    // A different expression ⇒ a different program, asserted where the difference lives: the values the
    // input tensor is initialised with.
    let other = compile_to_tenstorrent_dynamic(&loop_program(1, 3, "i + i")).unwrap();
    assert_eq!(other.spec.values, vec![2, 4, 6]);
    let (a, b) = (
        prog.program.as_ref().unwrap(),
        other.program.as_ref().unwrap(),
    );
    assert_ne!(
        a.tensors[0].init, b.tensors[0].init,
        "a different expression is a different program"
    );
}

#[test]
fn a_non_loop_producer_is_declined_by_the_dynamic_tier() {
    // A static-literal producer is the increment-1 case, not the dynamic tier.
    let static_prog = "## To produce (ch: Int):\n    Send 5 into ch.\n## Main\n    Let ch be a Pipe of Int.\n    Launch a task to produce with ch.\n    Receive a from ch.\n    Show a.\n";
    assert!(
        compile_to_tenstorrent_dynamic(static_prog).is_err(),
        "static producer is not a dynamic loop"
    );
}

#[test]
fn the_dynamic_stream_folds_to_the_vm_total() {
    for (lo, hi, expr, name) in [
        (1, 3, "i", "sum"),
        (1, 4, "i * i", "sumsq"),
        (1, 3, "i + i", "double"),
    ] {
        let src = loop_program(lo, hi, expr);
        let out = vm_outcome_concurrent(&src);
        assert!(out.error.is_none(), "[{name}] VM must run: {:?}", out.error);
        let vm: i64 = out
            .output
            .split_whitespace()
            .next()
            .unwrap()
            .parse()
            .unwrap();
        // The generated producer streams f(i) for i in lo..=hi; the fold is order-independent (sum).
        let expected: i64 = (lo..=hi)
            .map(|i| match expr {
                "i" => i,
                "i * i" => i * i,
                "i + i" => i + i,
                _ => unreachable!(),
            })
            .sum();
        assert_eq!(vm, expected, "[{name}] VM total");
        let prog = compile_to_tenstorrent_dynamic(&src).unwrap();
        assert_eq!((prog.spec.lo, prog.spec.hi), (lo, hi));
        // The lowered stream must fold to the SAME total the VM produced — the ≡VM anchor stated over the
        // values the program actually carries, not over the shape of its source.
        assert_eq!(
            prog.spec.values.iter().sum::<i64>(),
            expected,
            "[{name}] the lowered stream's fold"
        );
    }
}
