//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! FRONT-END LOCK — English → the spatial IR, through ONE pinned pass order, deterministically.
//!
//! `compile_to_spatial(source, target)` recognizes the tensor op (matmul, elementwise, SwiGLU, fused row op),
//! lifts it to a template, and runs the pinned passes. This lock proves:
//!   A. the tensors are named from the source, every recognized program reaches `Verified`;
//!   B. refusal is typed and complete: a non-tensor program reports all three recognizer reasons, a perturbed
//!      layout reports `NotGemmLayout`, a `mutable` dimension is not a constant (the induction-variable trap);
//!   C. the tensor sugars (matmul, elementwise plus, SwiGLU, weighted / fused-add RMSNorm) compile on every
//!      target that provides the capability and refuse typed where it is absent, ≡ the VM;
//!   D. an integer source on a float-primary engine claims EXACT only below the fp32 mantissa bound;
//!   E. the pass order is pinned, each pass is idempotent on its own output, the pipeline is deterministic;
//!   F. a verified matmul evaluates to the VM's checksum on every preset.
//!
//! Run: `cargo test -p logicaffeine-tests --test spatial_front_end_lock`
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

#[path = "common/spatial.rs"]
mod spatial_support;

use logicaffeine_compile::compile::vm_outcome;
use logicaffeine_compile::spatial::eval::{eval_choreography, round_to, Value};
use logicaffeine_compile::spatial::ir::{
    NumericsClaim, NumericsPolicy, Phase, TensorId, TensorRole,
};
use logicaffeine_compile::spatial::passes::from_tensor_op::MatmulNames;
use logicaffeine_compile::spatial::passes::SpatialReject;
use logicaffeine_compile::spatial::pipeline::{
    compile_to_spatial, run_passes, template_from_source, template_from_spec, PASS_ORDER,
};
use logicaffeine_compile::spatial::target::SpatialTarget;
use logicaffeine_compile::tensor_recognize::{Fill, MatmulReject, MatmulSpec};
use spatial_support::*;

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
Show item 1 of C.
Show item 2 of C.
Show item 3 of C.
Show item 4 of C.
"#;

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
Show item 1 of C.
Show item 2 of C.
Show item 3 of C.
Show item 4 of C.
"#;

const SUGAR_RMSNORM: &str = r#"## Main
Let X be a 2 by 3 tensor of Real.
Set item 1 of X to 1.0.
Set item 2 of X to 2.0.
Set item 3 of X to 3.0.
Set item 4 of X to 4.0.
Set item 5 of X to 5.0.
Set item 6 of X to 6.0.
Let W be a 1 by 3 tensor of Real.
Set item 1 of W to 0.5.
Set item 2 of W to 1.0.
Set item 3 of W to 2.0.
Let Y be rmsnorm of X with weight W.
Show round(item 1 of Y * 1000000).
Show round(item 2 of Y * 1000000).
Show round(item 3 of Y * 1000000).
Show round(item 4 of Y * 1000000).
Show round(item 5 of Y * 1000000).
Show round(item 6 of Y * 1000000).
"#;

const SUGAR_FUSED_ADD: &str = r#"## Main
Let X be a 2 by 3 tensor of Real.
Set item 1 of X to 1.0.
Set item 2 of X to 2.0.
Set item 3 of X to 3.0.
Set item 4 of X to 4.0.
Set item 5 of X to 5.0.
Set item 6 of X to 6.0.
Let R be a 2 by 3 tensor of Real.
Set item 1 of R to 0.5.
Set item 2 of R to 0.5.
Set item 3 of R to 0.5.
Set item 4 of R to 1.0.
Set item 5 of R to 1.0.
Set item 6 of R to 1.0.
Let W be a 1 by 3 tensor of Real.
Set item 1 of W to 0.5.
Set item 2 of W to 1.0.
Set item 3 of W to 2.0.
Let Y be rmsnorm of X plus R with weight W.
Show round(item 1 of Y * 1000000).
Show round(item 2 of Y * 1000000).
Show round(item 3 of Y * 1000000).
Show round(item 4 of Y * 1000000).
Show round(item 5 of Y * 1000000).
Show round(item 6 of Y * 1000000).
"#;

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
Show round(item 2 of Y * 1000000).
Show round(item 3 of Y * 1000000).
Show round(item 4 of Y * 1000000).
"#;

fn shown_ints(output: &str) -> Vec<i64> {
    output
        .split_whitespace()
        .map(|t| t.parse::<i64>().expect("an integer line"))
        .collect()
}

/// The program's output tensor, `Show`n the way the source shows it (rounded ×1e6 for the float sugars).
fn shown_like_source(
    p: &logicaffeine_compile::spatial::ir::SpatialProgram,
    scale: Option<f64>,
) -> Vec<i64> {
    let out = eval_choreography(p, &inputs_from_fills(p)).unwrap();
    match (&out.tensors[&output_id(p)], scale) {
        (Value::Int(v), None) => v.clone(),
        (Value::Float(v), Some(s)) => v.iter().map(|x| (x * s).round() as i64).collect(),
        (Value::Float(v), None) => v.iter().map(|x| *x as i64).collect(),
        (Value::Int(v), Some(_)) => v.clone(),
    }
}

#[test]
fn compile_to_spatial_recognizes_a_logos_matmul_and_names_the_tensors() {
    for target in SpatialTarget::presets() {
        let p = compile_to_spatial(&gemm_program(64, 128, 96), &target)
            .unwrap_or_else(|e| panic!("{}: {e:?}", target.name));
        assert_eq!(p.phase(), Phase::Verified, "{}", target.name);
        let names: Vec<&str> = p.tensors.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["a", "b", "c"], "{}", target.name);
        assert_eq!((p.tensors[0].elems.rows, p.tensors[0].elems.cols), (64, 96));
        assert_eq!(
            (p.tensors[1].elems.rows, p.tensors[1].elems.cols),
            (96, 128)
        );
        assert_eq!(
            (p.tensors[2].elems.rows, p.tensors[2].elems.cols),
            (64, 128)
        );
        assert_eq!(p.name, "matmul_64x128x96");
    }
}

#[test]
fn a_non_tensor_program_is_refused_with_all_three_recognizer_reasons() {
    let wh = SpatialTarget::wormhole();
    match compile_to_spatial("## Main\nShow 1.\n", &wh) {
        Err(SpatialReject::Unrecognized {
            matmul: MatmulReject::NoLoopNest,
            ..
        }) => {}
        other => panic!("a scalar program is not a tensor op: {other:?}"),
    }
    // A triple loop whose B index is transposed is not a row-major GEMM.
    let wrong = gemm_program(64, 64, 64).replace(
        "item (k * cols + j + 1) of b",
        "item (j * cols + k + 1) of b",
    );
    match compile_to_spatial(&wrong, &wh) {
        Err(SpatialReject::Unrecognized {
            matmul: MatmulReject::NotGemmLayout,
            ..
        }) => {}
        other => panic!("a transposed operand must be refused as NotGemmLayout: {other:?}"),
    }
    match compile_to_spatial("## Main\nLet x be [1, 2].\n", &wh) {
        Err(SpatialReject::Parse(_)) | Err(SpatialReject::Unrecognized { .. }) => {}
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_mutable_dimension_binding_is_not_a_constant() {
    // `Let mutable rows be 64` is a variable, not a dimension: the recognizer must not freeze it.
    let src = gemm_program(64, 64, 64).replace("Let rows be 64.", "Let mutable rows be 64.");
    match compile_to_spatial(&src, &SpatialTarget::wormhole()) {
        Err(SpatialReject::Unrecognized { .. }) => {}
        other => panic!("a mutable bound is not a constant dimension: {other:?}"),
    }
}

#[test]
fn the_matmul_sugar_compiles_and_shows_the_vm_entries() {
    for target in [
        SpatialTarget::wormhole(),
        SpatialTarget::sky_fabric(64, 64),
        SpatialTarget::rtx3060(),
    ] {
        let p = compile_to_spatial(SUGAR_MATMUL, &target)
            .unwrap_or_else(|e| panic!("{}: {e:?}", target.name));
        assert_eq!(
            shown_like_source(&p, None),
            vec![19, 22, 43, 50],
            "{}",
            target.name
        );
        assert_eq!(
            shown_ints(&vm_outcome(SUGAR_MATMUL).output),
            vec![19, 22, 43, 50]
        );
    }
}

#[test]
fn the_elementwise_sugar_compiles_where_the_target_provides_it() {
    for target in [
        SpatialTarget::wormhole(),
        SpatialTarget::rtx3060(),
        SpatialTarget::sn40l_rdu(),
        SpatialTarget::sky_fabric(64, 64),
    ] {
        let p = compile_to_spatial(SUGAR_PLUS, &target)
            .unwrap_or_else(|e| panic!("{}: {e:?}", target.name));
        assert_eq!(
            shown_like_source(&p, None),
            vec![6, 8, 10, 12],
            "{}",
            target.name
        );
        assert!(p.stages.iter().any(|s| matches!(
            s.kind,
            logicaffeine_compile::spatial::ir::StageKind::Compute(
                logicaffeine_compile::spatial::ir::ComputeOp::Generic { .. }
            )
        )));
    }
}

#[test]
fn the_row_reduce_and_swiglu_sugars_compile_where_provided_and_refuse_typed_elsewhere() {
    for (src, scale, label) in [
        (SUGAR_RMSNORM, 1e6, "rmsnorm"),
        (SUGAR_FUSED_ADD, 1e6, "fused-add rmsnorm"),
        (SUGAR_SWIGLU, 1e6, "swiglu"),
    ] {
        let vm = shown_ints(&vm_outcome(src).output);
        for target in [
            SpatialTarget::wormhole(),
            SpatialTarget::rtx3060(),
            SpatialTarget::sn40l_rdu(),
        ] {
            let p = compile_to_spatial(src, &target)
                .unwrap_or_else(|e| panic!("{label} on {}: {e:?}", target.name));
            assert_eq!(p.phase(), Phase::Verified);
            let got = shown_like_source(&p, Some(scale));
            let out_dtype = p
                .tensors
                .iter()
                .find(|t| t.role == TensorRole::Output)
                .unwrap()
                .dtype;
            for (g, v) in got.iter().zip(vm.iter()) {
                // The spatial answer is the VM's answer as the target's float datapath stores it (fp32 on Tensix and
                // the GPU; the RDU's witnessed precision is bf16), within one ulp of the ×1e6 rounding.
                let want = (round_to(out_dtype, *v as f64 / scale) * scale).round() as i64;
                assert!((g - want).abs() <= 1, "{label} on {}: {g} vs VM {v} at {out_dtype:?} = {want} (one ulp of the ×1e6 rounding allowed)", target.name);
            }
            assert_eq!(got.len(), vm.len());
        }
        match compile_to_spatial(src, &SpatialTarget::sky_fabric(64, 64)) {
            Err(SpatialReject::MissingCompute { needed, .. }) => {
                assert!(!needed.is_empty(), "{label}")
            }
            other => panic!(
                "{label}: the scalar fabric has no row-reduce / transcendental engine: {other:?}"
            ),
        }
    }
}

#[test]
fn an_int_source_on_a_float_primary_engine_claims_exact_only_below_the_fp32_bound() {
    let wh = SpatialTarget::wormhole();
    let names = MatmulNames {
        a: "a".into(),
        b: "b".into(),
        c: "c".into(),
    };
    let mk = |k: usize| {
        template_from_spec(
            &MatmulSpec {
                m: 64,
                n: 64,
                k,
                a: Default::default(),
                b: Default::default(),
                c: Default::default(),
            },
            &names,
            (&Fill::Unknown, &Fill::Unknown),
            (Some((-127, 127)), Some((-127, 127))),
            NumericsPolicy::Exact,
            &wh,
        )
        .unwrap()
    };
    assert_eq!(mk(512).claim, NumericsClaim::Exact, "127·127·512 < 2^24");
    assert!(
        matches!(mk(2048).claim, NumericsClaim::Approx { .. }),
        "127·127·2048 ≥ 2^24: partial sums may round"
    );
    let fill = template_from_source(&gemm_program(64, 64, 4096), &wh).unwrap();
    assert_eq!(
        fill.claim,
        NumericsClaim::Exact,
        "the canonical fills are tiny: 3·2·4096 < 2^24"
    );
    let unknown = template_from_spec(
        &MatmulSpec {
            m: 64,
            n: 64,
            k: 64,
            a: Default::default(),
            b: Default::default(),
            c: Default::default(),
        },
        &names,
        (&Fill::Unknown, &Fill::Unknown),
        (None, None),
        NumericsPolicy::Exact,
        &wh,
    )
    .unwrap();
    assert!(
        matches!(unknown.claim, NumericsClaim::Approx { .. }),
        "an unknown operand range is never claimed exact"
    );
    // On an exact integer engine the same programs are exact regardless of K.
    let sky = SpatialTarget::sky_fabric(64, 64);
    let p = template_from_source(&gemm_program(64, 64, 4096), &sky).unwrap();
    assert_eq!(p.claim, NumericsClaim::Exact);
    assert_eq!(
        p.tensors[2].dtype,
        logicaffeine_compile::codegen_sva::fabric_target::Precision::Int64
    );
}

#[test]
fn the_pass_order_is_pinned_and_each_pass_is_idempotent() {
    let names: Vec<&str> = PASS_ORDER.iter().map(|(n, _)| *n).collect();
    assert_eq!(
        names,
        [
            // `fuse_generic` runs FIRST, on the template: legalization splits a workload into stages so
            // the split can be checked against its specification, and this puts back together whatever
            // the destination registers can carry — before anything is tiled, placed or scheduled.
            "fuse_generic",
            "tile",
            "place",
            "fuse_readers",
            "size_buffers",
            "synthesize_handshakes",
            "persist",
            "verify",
        ]
    );
    for target in [
        SpatialTarget::wormhole(),
        fusing_wormhole(),
        SpatialTarget::sky_fabric(64, 64),
    ] {
        let mut p = template_from_source(&gemm_program(128, 128, 64), &target).unwrap();
        for (name, pass) in PASS_ORDER {
            let once =
                pass(&p, &target).unwrap_or_else(|e| panic!("{} {name}: {e:?}", target.name));
            once.validate()
                .unwrap_or_else(|e| panic!("{} after {name}: {e:?}", target.name));
            let twice = pass(&once, &target)
                .unwrap_or_else(|e| panic!("{} {name} twice: {e:?}", target.name));
            assert_eq!(
                twice, once,
                "{} {name}: a pass must be idempotent on its own output",
                target.name
            );
            p = once;
        }
        assert_eq!(p.phase(), Phase::Verified);
        assert_eq!(
            p,
            run_passes(
                template_from_source(&gemm_program(128, 128, 64), &target).unwrap(),
                &target
            )
            .unwrap()
        );
    }
}

#[test]
fn compile_to_spatial_is_deterministic() {
    for target in SpatialTarget::presets() {
        let a = compile_to_spatial(&gemm_program(256, 128, 192), &target).unwrap();
        let b = compile_to_spatial(&gemm_program(256, 128, 192), &target).unwrap();
        assert_eq!(a.neutral_dump(), b.neutral_dump(), "{}", target.name);
        assert_eq!(
            a.certificate.as_ref().unwrap().schedule_digest,
            b.certificate.as_ref().unwrap().schedule_digest
        );
    }
}

#[test]
fn a_verified_matmul_evaluates_to_the_vm_checksum_on_every_preset() {
    let src = gemm_program(96, 64, 128);
    let vm = shown_int(&vm_outcome(&src).output);
    for target in SpatialTarget::presets() {
        let p = compile_to_spatial(&src, &target).unwrap();
        let out = eval_choreography(&p, &inputs_from_fills(&p)).unwrap();
        let got = match &out.tensors[&TensorId(2)] {
            Value::Int(v) => checksum(v),
            Value::Float(v) => checksum(&v.iter().map(|x| *x as i64).collect::<Vec<_>>()),
        };
        assert_eq!(got, vm, "{}: choreography checksum != VM", target.name);
    }
}
