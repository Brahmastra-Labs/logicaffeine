//! ★ SPATIAL PIPE LOWERING LOCK ★ — a producer/consumer pipe is a SPATIAL PROGRAM, not a hand-written
//! two-core template.
//!
//! This is the first increment of P4c (concurrency → the IR). `codegen_tt::concurrency_mesh` emitted the
//! pipe as two baby-RISC-V kernel strings around a circular buffer, with the recognized sends baked in as
//! a `const uint32_t vals[N]`. The shape was always expressible in the IR and needed nothing new:
//!
//!   * a task sending N values  → a **Load** on one engine, filling a FIFO buffer
//!   * `Send`/`Receive`         → the buffer's `Reserve`/`Push`/`Wait`/`Pop` choreography
//!   * main folding the values  → a **reduction**, which is not an operator but an `IterKind::Reduce` axis
//!   * the sends themselves     → the input tensor's `Values` init
//!
//! That last one is the whole difference between a template and a lowering. The same numbers, but reachable
//! by every pass, the census and the evaluator instead of sealed inside C++ text.
//!
//! ★ WHY THE ASSERTION IS A NUMBER. The test this pattern replaces asserted that a kernel CONTAINED
//! `cb_push_back`. `tt_matrix_lock`'s int8 test asserted a kernel contained `matmul_tiles` — and passed for
//! its whole life while that kernel returned 196465 against a golden of 131 (measured 0/4096 on Wormhole
//! AND Blackhole). A lowering that prints is not a lowering that computes, so what is locked here is the
//! FOLD, evaluated.
//!
//! Run: `cargo nextest run -p logicaffeine-compile --features codegen -E 'binary(spatial_pipe_lowering_lock)'`
#![cfg(feature = "codegen")]

use logicaffeine_compile::spatial::eval::{eval_choreography, Inputs, Value};
use logicaffeine_compile::spatial::ir::TensorId;
use logicaffeine_compile::spatial::lower::tensix::TensixBackend;
use logicaffeine_compile::spatial::passes::from_tensor_op::from_pipe;
use logicaffeine_compile::spatial::pipeline::run_passes;
use logicaffeine_compile::spatial::target::SpatialTarget;

/// Lower a pipe and evaluate its fold. Returns the folded total.
fn fold_of(sends: &[i64]) -> i64 {
    let t = SpatialTarget::wormhole();
    let tpl = from_pipe(sends, &t).unwrap_or_else(|e| panic!("template refused {sends:?}: {e:?}"));
    let p = run_passes(tpl, &t).unwrap_or_else(|e| panic!("passes refused {sends:?}: {e:?}"));
    TensixBackend::new(t)
        .compute_kernel_source(&p)
        .unwrap_or_else(|e| panic!("the Tensix printer refused {sends:?}: {e:?}"));
    let inputs = Inputs {
        tensors: std::collections::BTreeMap::from([(
            TensorId(0),
            Value::Float(sends.iter().map(|v| *v as f64).collect()),
        )]),
    };
    let out =
        eval_choreography(&p, &inputs).unwrap_or_else(|e| panic!("eval faulted {sends:?}: {e:?}"));
    match &out.tensors[&TensorId(1)] {
        Value::Int(v) => v.iter().sum(),
        Value::Float(v) => v.iter().sum::<f64>() as i64,
    }
}

#[test]
fn a_pipe_lowers_through_the_ir_and_its_fold_is_the_sum_of_its_sends() {
    // Several shapes, so a fold that returned a constant — or read the wrong tensor — cannot pass.
    for sends in [
        vec![42i64],
        vec![10, 20, 30],
        vec![1, 2, 3, 4],
        vec![7, -3, 11, -5, 2],
    ] {
        let want: i64 = sends.iter().sum();
        assert_eq!(
            fold_of(&sends),
            want,
            "the pipe's fold must be the sum of {sends:?}"
        );
    }
}

#[test]
fn the_lowered_program_is_a_function_of_the_recognized_sends() {
    // The template-era property ("the producer kernel carries the source's sends") restated over the IR:
    // different sends are a different PROGRAM, not merely different kernel text.
    let t = SpatialTarget::wormhole();
    let a = from_pipe(&[10, 20, 30], &t).unwrap();
    let b = from_pipe(&[1, 2, 3, 4], &t).unwrap();
    assert_ne!(
        a.tensors[0].init, b.tensors[0].init,
        "the sends ARE the input tensor's init"
    );
    assert_ne!(
        a.tensors[0].elems.cols, b.tensors[0].elems.cols,
        "a different send count is a different shape"
    );
}

#[test]
fn a_pipe_whose_fold_leaves_the_exact_fp32_range_is_refused_rather_than_rounded() {
    // The fold must be EXACT. Int64 would force `AccPolicy::I64`, which the Tensix printer refuses because
    // the matrix engine is float-primary — the hardware fact that retired the int8 GEMM path. Fp32 carries
    // integers exactly below 2^23, so the honest move at the boundary is a typed refusal, never a silently
    // rounded sum.
    let t = SpatialTarget::wormhole();
    assert!(
        from_pipe(&[(1i64 << 23) - 1], &t).is_ok(),
        "inside the exact range this must lower"
    );
    assert!(
        from_pipe(&[1i64 << 23], &t).is_err(),
        "at the boundary it must refuse, not round"
    );
    assert!(
        from_pipe(&[1i64 << 22, 1 << 22], &t).is_err(),
        "the SUM is what leaves the range, not any one value"
    );
    assert!(
        from_pipe(&[], &t).is_err(),
        "a pipe with no sends is not a pipe"
    );
}
