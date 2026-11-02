//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! P4 LOCK — the Tensix printer lowers EVERY workload the front end verifies.
//!
//! The spatial pipeline recognizes four tensor sugars — GEMM, elementwise, SwiGLU and the fused row
//! ops (rmsnorm / softmax / layernorm, with or without weight, residual and bias). All four compile
//! to `Phase::Verified` on `wormhole()` and all four evaluate ≡ VM; `spatial_front_end_lock` proves
//! that today. **Only GEMM reaches a Tensix kernel.** Every other one dies at
//! `spatial/lower/tensix/kernel.rs` with `"a non-matmul compute op on Tensix (P4)"`, and the host
//! dies one step earlier at `"a host for exactly two input tensors"`.
//!
//! ★ WHY THIS FILE EXISTS RATHER THAN FOUR SCATTERED TESTS. Nothing in the suite lowered a
//! non-GEMM spatial program through `TensixBackend`, so that refusal had never once gone red — the
//! gap was invisible, and the legacy `codegen_tt` templates quietly remained the ONLY Tensix path
//! for elementwise, SwiGLU and the row ops. A matrix stated once cannot hide a square: adding a
//! sugar to the front end without teaching the printer fails HERE, by name, immediately.
//!
//! ★ THE CONTRACT IS A BICONDITIONAL, and both halves earn their keep. If the front end VERIFIES a
//! program for a Tensix target then the printer must lower it — a verified program the backend
//! cannot print is a compiler that recognized something it cannot build. And the GEMM control must
//! keep lowering, or a "fix" that refuses everything would satisfy the first half perfectly.
//!
//! Run: `cargo nextest run -p logicaffeine-compile -E 'binary(spatial_tensix_p4_lock)'`
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::spatial::ir::{ComputeOp, Phase, StageKind};
use logicaffeine_compile::spatial::lower::tensix::TensixBackend;
use logicaffeine_compile::spatial::lower::{SpatialBackend, SpatialLowerError};
use logicaffeine_compile::spatial::pipeline::compile_to_spatial;
use logicaffeine_compile::spatial::target::SpatialTarget;

/// A 2×2 int GEMM — the control. This one lowers today.
const SUGAR_MATMUL: &str = r#"## Main
Let A be a 2 by 2 tensor of Int.
Set item 1 of A to 1.
Set item 2 of A to 2.
Set item 3 of A to 3.
Set item 4 of A to 4.
Let B be a 2 by 2 tensor of Int.
Set item 1 of B to 5.
Set item 2 of B to 6.
Set item 3 of B to 7.
Set item 4 of B to 8.
Let C be A matmul B.
Show C.
"#;

/// Elementwise add — `ComputeOp::Elementwise`.
const SUGAR_PLUS: &str = r#"## Main
Let A be a 2 by 2 tensor of Int.
Set item 1 of A to 1.
Set item 2 of A to 2.
Set item 3 of A to 3.
Set item 4 of A to 4.
Let B be a 2 by 2 tensor of Int.
Set item 1 of B to 5.
Set item 2 of B to 6.
Set item 3 of B to 7.
Set item 4 of B to 8.
Let C be A plus B.
Show C.
"#;

/// Weighted rmsnorm — `ComputeOp::RowReduce`, and THREE input tensors, which is also what breaks
/// the host's two-input ceiling.
const SUGAR_RMSNORM: &str = r#"## Main
Let X be a 2 by 3 tensor of Real.
Set item 1 of X to 1.0.
Set item 2 of X to 2.0.
Set item 3 of X to 3.0.
Set item 4 of X to 4.0.
Set item 5 of X to 5.0.
Set item 6 of X to 6.0.
Let W be a 1 by 3 tensor of Real.
Set item 1 of W to 1.0.
Set item 2 of W to 1.0.
Set item 3 of W to 1.0.
Let Y be rmsnorm of X with weight W.
Show round(item 1 of Y * 1000000).
Show round(item 4 of Y * 1000000).
"#;

/// SwiGLU — `ComputeOp::SwiGlu`, an SFPU transcendental rather than a matrix-engine op.
const SUGAR_SWIGLU: &str = r#"## Main
Let G be a 1 by 4 tensor of Real.
Set item 1 of G to -1.0.
Set item 2 of G to 0.0.
Set item 3 of G to 1.0.
Set item 4 of G to 2.0.
Let U be a 1 by 4 tensor of Real.
Set item 1 of U to 1.0.
Set item 2 of U to 2.0.
Set item 3 of U to 3.0.
Set item 4 of U to 4.0.
Let Y be swiglu of G and U.
Show round(item 1 of Y * 1000000).
Show round(item 4 of Y * 1000000).
"#;

/// The workloads the front end accepts, each with the compute op it produces.
const WORKLOADS: &[(&str, &str)] = &[
    ("matmul", SUGAR_MATMUL),
    ("elementwise", SUGAR_PLUS),
    ("rmsnorm (weighted)", SUGAR_RMSNORM),
    ("swiglu", SUGAR_SWIGLU),
];

/// The compute ops a program actually carries — named, so a failure says WHAT could not be printed
/// rather than only that something could not.
fn compute_ops(p: &logicaffeine_compile::spatial::ir::SpatialProgram) -> Vec<String> {
    p.stages
        .iter()
        .filter_map(|s| match &s.kind {
            StageKind::Compute(op) => Some(match op {
                ComputeOp::MatmulTiles { .. } => "MatmulTiles".to_string(),
                other => format!("{other:?}")
                    .split_whitespace()
                    .next()
                    .unwrap_or("?")
                    .to_string(),
            }),
            _ => None,
        })
        .collect()
}

#[test]
fn every_workload_the_front_end_verifies_is_lowered_by_the_tensix_printer() {
    let wh = SpatialTarget::wormhole();
    let mut unprintable: Vec<String> = Vec::new();

    for (label, src) in WORKLOADS {
        // The front end's own verdict. A target that cannot host the workload at all is not this
        // lock's business — that is a typed `MissingCompute` and `spatial_front_end_lock` owns it.
        let p = match compile_to_spatial(src, &wh) {
            Ok(p) => p,
            Err(e) => panic!(
                "{label}: the front end must verify this on wormhole — spatial_front_end_lock \
                 asserts it does. Got {e:?}"
            ),
        };
        assert_eq!(
            p.phase(),
            Phase::Verified,
            "{label}: the pipeline must reach Verified"
        );

        match TensixBackend::new(wh.clone()).lower(&p) {
            Ok(_) => {}
            Err(e) => unprintable.push(format!(
                "  {label}: {e:?}\n      compute ops in the program: {:?}",
                compute_ops(&p)
            )),
        }
    }

    assert!(
        unprintable.is_empty(),
        "\nthe front end verifies {} workload(s) the Tensix printer cannot lower:\n{}\n\n\
         Each is a workload for which the legacy `codegen_tt` templates are still the ONLY Tensix \
         path, which is what keeps them alive. The fix is in the PRINTER \
         (`spatial/lower/tensix/kernel.rs::emit_compute` for the compute op, \
         `host.rs::print_host` for the input arity) — never in this assertion.\n",
        unprintable.len(),
        unprintable.join("\n")
    );
}

#[test]
fn the_gemm_control_still_lowers() {
    // ★ The half that stops a false fix. A printer changed to refuse nothing — or to accept
    // everything by emitting nothing — would satisfy the biconditional's first half and be useless.
    // GEMM lowering to a real artifact with kernels and a host is what "lowered" has to mean.
    let wh = SpatialTarget::wormhole();
    let p = compile_to_spatial(SUGAR_MATMUL, &wh).expect("the GEMM sugar verifies");
    let art = match TensixBackend::new(wh)
        .lower(&p)
        .expect("GEMM lowers today and must keep doing so")
    {
        logicaffeine_compile::spatial::lower::SpatialArtifact::Tensix(a) => a,
        other => panic!("the Tensix backend must answer a Tensix artifact: {other:?}"),
    };
    assert!(!art.kernels.is_empty(), "a lowered program has kernels");
    assert!(
        art.host_cpp.contains("int main("),
        "and a host that runs them"
    );
    assert!(
        art.kernels.iter().any(|k| k.source.contains("matmul")),
        "and the compute kernel really is a matmul"
    );
}

#[test]
fn the_printer_refuses_by_name_rather_than_by_panic() {
    // ★ A refusal is a promise about HOW it fails. While P4 is unfinished the printer must decline
    // with a typed `Unsupported` naming the construct — never unwind — so a caller can report the
    // gap instead of dying inside it. This stays true after P4: the set it refuses shrinks to
    // empty, but nothing in it may ever become a panic.
    let wh = SpatialTarget::wormhole();
    for (label, src) in WORKLOADS {
        let Ok(p) = compile_to_spatial(src, &wh) else {
            continue;
        };
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            TensixBackend::new(wh.clone()).lower(&p)
        }));
        match outcome {
            Ok(Ok(_)) => {}
            Ok(Err(SpatialLowerError::Unsupported(why))) => assert!(
                !why.is_empty(),
                "{label}: a refusal must name what it could not print"
            ),
            Ok(Err(other)) => panic!("{label}: an unexpected typed error: {other:?}"),
            Err(_) => panic!("{label}: the printer PANICKED — a gap must be a typed refusal"),
        }
    }
}
