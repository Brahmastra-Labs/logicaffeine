//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! CHOREOGRAPHY SEMANTICS LOCK — the spatial IR has an executable MEANING, and that meaning is the
//! systolic reference, the VM, and nothing else.
//!
//! `eval_choreography` runs a placed+scheduled program as coroutines over bounded FIFOs, semaphores and
//! accumulators (token semantics). This lock proves:
//!   A. it computes the GEMM exactly — ≡ `eval_tiled_gemm` for every tile split, ≡ `eval_systolic`, ≡ the VM
//!      checksum of the recognized LOGOS program, on scalar-tile and 32×32-tile targets alike;
//!   B. its float lane is bit-exact to `eval_systolic_bf16` (K-ascending f32 accumulation) including
//!      multi-block spills, and a declared spill-rounding policy is modeled exactly (not ignored);
//!   C. the result is scheduler-independent (Kahn determinacy, executable);
//!   D. it is FAULT-COMPLETE: dropping any load-bearing choreography op (a reserve, push, wait, pop, semaphore
//!      step, multicast, read, write, compute, pack, accumulator acquire/release) is DETECTED — a typed
//!      fault or a structural violation — and no mutation is ever `Ok` with a wrong answer. A cheat that
//!      always returns the reference cannot pass this file.
//!
//! Run: `cargo test -p logicaffeine-tests --test spatial_choreography_semantics_lock`
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

#[path = "common/spatial.rs"]
mod spatial_support;

use logicaffeine_compile::codegen_sva::systolic::{
    eval_systolic, eval_systolic_bf16, eval_tiled_gemm, round_bf16, Systolic,
};
use logicaffeine_compile::compile::vm_outcome;
use logicaffeine_compile::spatial::cost::is_streaming;
use logicaffeine_compile::spatial::eval::eval_choreography_with;
use logicaffeine_compile::spatial::eval::{
    eval_choreography, eval_choreography_confluent, eval_choreography_traced, ChoreoFault, Inputs,
    Scheduler, Value,
};
use logicaffeine_compile::spatial::ir::{
    AccPolicy, BufferKind, ChoreoOp, EnginePacking, Granularity, NumericsPolicy, SpatialProgram,
    TensorId,
};
use logicaffeine_compile::spatial::passes::from_tensor_op::MatmulNames;
use logicaffeine_compile::spatial::passes::tile::candidates;
use logicaffeine_compile::spatial::pipeline::{
    compile_to_spatial, run_passes, template_from_source, template_from_source_with,
    template_from_spec,
};
use logicaffeine_compile::spatial::target::{ComputeCap, SpatialTarget};
use logicaffeine_compile::tensor_recognize::{Fill, MatmulSpec};
use spatial_support::*;

fn names() -> MatmulNames {
    MatmulNames {
        a: "a".into(),
        b: "b".into(),
        c: "c".into(),
    }
}

/// A verified program for an external-data int matmul of shape (m,n,k) on `target`, with its inputs.
fn int_program(
    m: usize,
    n: usize,
    k: usize,
    seed: u64,
    target: &SpatialTarget,
) -> (SpatialProgram, Inputs, Vec<i64>, Vec<i64>) {
    let spec = MatmulSpec {
        m,
        n,
        k,
        a: Default::default(),
        b: Default::default(),
        c: Default::default(),
    };
    let p = template_from_spec(
        &spec,
        &names(),
        (&Fill::Unknown, &Fill::Unknown),
        (Some((-128, 127)), Some((-128, 127))),
        NumericsPolicy::Exact,
        target,
    )
    .unwrap_or_else(|e| panic!("{m}x{n}x{k} on {}: {e:?}", target.name));
    let p = run_passes(p, target)
        .unwrap_or_else(|e| panic!("{m}x{n}x{k} on {}: passes: {e:?}", target.name));
    let acts: Vec<i64> = (0..m * k).map(|i| prng(seed, i, -128, 127)).collect();
    let weights: Vec<i64> = (0..k * n).map(|i| prng(seed + 1, i, -128, 127)).collect();
    let mut inputs = Inputs::default();
    inputs.tensors.insert(TensorId(0), Value::Int(acts.clone()));
    inputs
        .tensors
        .insert(TensorId(1), Value::Int(weights.clone()));
    (p, inputs, acts, weights)
}

fn as_ints(v: &Value) -> Vec<i64> {
    match v {
        Value::Int(x) => x.clone(),
        Value::Float(x) => x
            .iter()
            .map(|f| {
                assert_eq!(
                    f.fract(),
                    0.0,
                    "an integer-range GEMM must produce integral floats: {f}"
                );
                *f as i64
            })
            .collect(),
    }
}

#[test]
fn choreography_equals_eval_tiled_gemm_over_hundreds_of_int8_cases() {
    let sky = SpatialTarget::sky_fabric(64, 64);
    let mut checked = 0;
    for case in 0..300usize {
        let (m, n, k) = (
            (prng(7, case, 1, 24)) as usize,
            (prng(8, case, 1, 24)) as usize,
            (prng(9, case, 1, 24)) as usize,
        );
        let (p, inputs, acts, weights) = int_program(m, n, k, 100 + case as u64, &sky);
        let out = eval_choreography(&p, &inputs)
            .unwrap_or_else(|f| panic!("case {case} {m}x{n}x{k}: {f:?}"));
        let got = as_ints(&out.tensors[&output_id(&p)]);
        let sys = Systolic::new(k, n, weights.clone());
        assert_eq!(
            got,
            eval_systolic(&sys, &acts, m),
            "case {case} {m}x{n}x{k} != eval_systolic"
        );
        for (tk, tn) in [(1usize, 1usize), (3, 5), (8, 8), (k.max(1), n.max(1))] {
            assert_eq!(
                got,
                eval_tiled_gemm(&sys, &acts, m, tk, tn),
                "case {case}: != eval_tiled_gemm({tk},{tn})"
            );
        }
        checked += 1;
    }
    assert_eq!(checked, 300);
    // 32×32 tiles (the Tensix float lane): int8-range data, K ≤ 192 keeps the f32 partial sums exact.
    let wh = SpatialTarget::wormhole();
    for case in 0..40usize {
        let (m, n, k) = (
            32 * prng(11, case, 1, 6) as usize,
            32 * prng(12, case, 1, 6) as usize,
            32 * prng(13, case, 1, 6) as usize,
        );
        let (p, inputs, acts, weights) = int_program(m, n, k, 500 + case as u64, &wh);
        let out = eval_choreography(&p, &inputs)
            .unwrap_or_else(|f| panic!("wormhole case {case} {m}x{n}x{k}: {f:?}"));
        let got = as_ints(&out.tensors[&output_id(&p)]);
        assert_eq!(
            got,
            eval_systolic(&Systolic::new(k, n, weights), &acts, m),
            "wormhole case {case} {m}x{n}x{k}"
        );
    }
}

#[test]
fn choreography_equals_the_vm_checksum_of_the_recognized_program() {
    for target in [
        SpatialTarget::wormhole(),
        SpatialTarget::sky_fabric(64, 64),
        SpatialTarget::rtx3060(),
        SpatialTarget::sn40l_rdu(),
    ] {
        for (m, n, k) in [(64usize, 64usize, 64usize), (96, 160, 128), (100, 70, 45)] {
            let src = gemm_program(m, n, k);
            let p = compile_to_spatial(&src, &target)
                .unwrap_or_else(|e| panic!("{} {m}x{n}x{k}: {e:?}", target.name));
            let out = eval_choreography(&p, &inputs_from_fills(&p))
                .unwrap_or_else(|f| panic!("{} {m}x{n}x{k}: {f:?}", target.name));
            let got = checksum(&as_ints(&out.tensors[&output_id(&p)]));
            let vm = vm_outcome(&src);
            assert!(vm.error.is_none(), "VM error: {:?}", vm.error);
            assert_eq!(
                got,
                shown_int(&vm.output),
                "{} {m}x{n}x{k}: choreography checksum != VM",
                target.name
            );
            assert_eq!(got, checksum(&gemm_reference(m, n, k)));
        }
    }
}

#[test]
fn non_multiple_of_32_shapes_are_zero_padded_and_cropped_exactly() {
    let wh = SpatialTarget::wormhole();
    for (m, n, k) in [
        (100usize, 70usize, 45usize),
        (33, 1, 65),
        (1, 33, 1),
        (130, 40, 70),
    ] {
        let (p, inputs, acts, weights) = int_program(m, n, k, 900 + m as u64, &wh);
        assert_eq!(p.tensors[2].tiles.count(), m.div_ceil(32) * n.div_ceil(32));
        let out = eval_choreography(&p, &inputs).unwrap_or_else(|f| panic!("{m}x{n}x{k}: {f:?}"));
        let got = as_ints(&out.tensors[&output_id(&p)]);
        assert_eq!(
            got.len(),
            m * n,
            "the output is cropped to the element shape"
        );
        assert_eq!(
            got,
            eval_systolic(&Systolic::new(k, n, weights), &acts, m),
            "{m}x{n}x{k}"
        );
    }
}

#[test]
fn f32_accumulation_equals_eval_systolic_bf16_bitwise_including_multi_block_spills() {
    // TWO contracts, and they must be pinned apart because one is arithmetic and the other is silicon.
    //
    // A. THE NUMERICS. A small L1 forces a small inner block, so K=256 spills partials through the
    //    intermediate buffer several times; with exact numerics that fp32 spill is lossless, and the result
    //    must be bit-identical to a straight K-ascending f32 accumulation. This is a claim about arithmetic
    //    and it holds on any part that can reload a partial at the accumulator's own width.
    //
    //    Real Wormhole cannot (see B), so the numerics are exercised on a part that declares it can and is
    //    otherwise identical — same bf16 operands, same tile, same register file. Isolating the one capability
    //    under test is what keeps this a numerics lock rather than a silent no-op the day a device changes.
    let mut small_l1 = SpatialTarget::wormhole();
    small_l1.site.local_mem_bytes = 96 * 1024;
    for c in small_l1.compute.iter_mut() {
        if let ComputeCap::MatmulTile {
            wide_spill_reload, ..
        } = c
        {
            *wide_spill_reload = true;
        }
    }
    assert!(
        small_l1.reloads_wide_spills(),
        "contract A needs a part that can reload a wide spill"
    );
    // ★ THE SHAPE MUST GENUINELY FORCE THE SPILL, so it is CHOSEN rather than assumed. What makes a plan
    // multi-block is that the K panel does not fit L1 — and how much of it the planner tries to hold is a
    // plan decision, so a shape that forced a spill under one cost model need not under the next. (It
    // stopped: 64x64x256 at 96 KiB now fits K in one block, and this contract silently tested nothing.)
    // Deeper K at the same L1 cannot fit whatever the planner prefers, and the check below proves each
    // case actually spilled rather than trusting that it would.
    let mut multi_block_cases = 0;
    for (m, n, k) in [(64usize, 64usize, 2048usize), (32, 96, 2048)] {
        let spec = MatmulSpec {
            m,
            n,
            k,
            a: Default::default(),
            b: Default::default(),
            c: Default::default(),
        };
        let p = template_from_spec(
            &spec,
            &names(),
            (&Fill::Unknown, &Fill::Unknown),
            (None, None),
            NumericsPolicy::Exact,
            &small_l1,
        )
        .unwrap();
        let p = run_passes(p, &small_l1).unwrap();
        assert!(
            p.summary().num_blocks > 1,
            "the small L1 must force a multi-block plan: {:?}",
            p.summary()
        );
        multi_block_cases += 1;
        let acts: Vec<f64> = (0..m * k)
            .map(|i| round_bf16(prng(31, i, -800, 800) as f64 / 64.0))
            .collect();
        let weights: Vec<f64> = (0..k * n)
            .map(|i| round_bf16(prng(32, i, -800, 800) as f64 / 64.0))
            .collect();
        let mut inputs = Inputs::default();
        inputs
            .tensors
            .insert(TensorId(0), Value::Float(acts.clone()));
        inputs
            .tensors
            .insert(TensorId(1), Value::Float(weights.clone()));
        let out = eval_choreography(&p, &inputs).unwrap_or_else(|f| panic!("{m}x{n}x{k}: {f:?}"));
        let got = match &out.tensors[&output_id(&p)] {
            Value::Float(v) => v.clone(),
            other => panic!("float output expected, got {other:?}"),
        };
        let reference = eval_systolic_bf16(&weights, &acts, m, k, n);
        assert_eq!(
            got, reference,
            "{m}x{n}x{k}: the F32 lane must be bit-identical to eval_systolic_bf16"
        );
    }
    assert!(multi_block_cases >= 2, "the spill lane must be exercised by at least two multi-block plans, not {multi_block_cases}");

    // B. THE REFUSAL. Real Wormhole rejects the Float32 unpack a wide reload needs — MEASURED under ttsim as
    //    `tensix_unpacr: unpack_to_dst=0 in_data_format=0 out_data_format=0`, even with the intermediate
    //    buffer marked `UnpackToDestFp32`. So on that part an exact-numerics plan may not spill at all, and
    //    the compiler must DECLINE to emit one rather than hand the device a program it will reject.
    //
    //    Pinned as its own assertion so it fails loudly the day the reload starts working: at that point this
    //    is the test that says so, and contract A can move back onto the real part.
    let mut real = SpatialTarget::wormhole();
    real.site.local_mem_bytes = 96 * 1024;
    assert!(
        !real.reloads_wide_spills(),
        "Wormhole must still declare that it cannot reload a wide spill"
    );
    for (m, n, k) in [(64usize, 64usize, 256usize), (32, 96, 512)] {
        let spec = MatmulSpec {
            m,
            n,
            k,
            a: Default::default(),
            b: Default::default(),
            c: Default::default(),
        };
        let t = template_from_spec(
            &spec,
            &names(),
            (&Fill::Unknown, &Fill::Unknown),
            (None, None),
            NumericsPolicy::Exact,
            &real,
        )
        .unwrap();
        // A SPILLING plan is multi-block AND blocked. The streaming family also has many K blocks, but it
        // accumulates every step in the register file and never touches an intermediate buffer, so it needs
        // no reload and is unaffected by the device's refusal — `tile` creates a spill buffer exactly when
        // `num_blocks > 1 && !is_streaming`.
        let spilling: Vec<_> = candidates(&t, &real)
            .into_iter()
            .filter(|c| c.plan.inner.num_blocks > 1 && !is_streaming(&c.plan))
            .collect();
        assert!(
            spilling.is_empty(),
            "{m}x{n}x{k}: exact numerics must offer no spilling plan on a part that cannot reload one, but {} were offered (first {:?})",
            spilling.len(),
            spilling.first().map(|c| c.plan)
        );
        let p = run_passes(t, &real).unwrap();
        let s = p.summary();
        assert!(
            s.num_blocks == 1 || s.streaming,
            "{m}x{n}x{k}: an exact plan on this part either holds the reduction in one block or streams it — never spills: {s:?}"
        );
        assert!(
            !p.buffers.iter().any(|b| b.name.starts_with("interm")),
            "{m}x{n}x{k}: the emitted exact plan carries no intermediate buffer, because it may not reload one"
        );
    }
}

#[test]
fn spill_rounding_is_modeled_exactly_when_declared() {
    // An explicit four-K-block blocked plan under the relaxed policy: the compute engine spills bf16 partials.
    let wh = SpatialTarget::wormhole();
    let (m, n, k) = (64usize, 64usize, 256usize);
    let spec = MatmulSpec {
        m,
        n,
        k,
        a: Default::default(),
        b: Default::default(),
        c: Default::default(),
    };
    let t = template_from_spec(
        &spec,
        &names(),
        (&Fill::Unknown, &Fill::Unknown),
        (None, None),
        NumericsPolicy::AllowSpillRounding,
        &wh,
    )
    .unwrap();
    let plan = logicaffeine_compile::spatial::ir::BlockPlan {
        per_site_m: 1,
        per_site_n: 1,
        grid: (2, 2),
        inner: logicaffeine_compile::spatial::ir::InnerLoop {
            num_blocks: 4,
            block_k_tiles: 2,
        },
        block: logicaffeine_compile::spatial::ir::BlockShape {
            osh: 1,
            osw: 1,
            ibw: 2,
        },
        acc: AccPolicy::F32,
        pack_accumulate: false,
        engines: EnginePacking::LoadsTogether,
        granularity: Granularity::PerTile,
    };
    let p = run_passes(
        logicaffeine_compile::spatial::passes::tile::apply_plan(&t, plan, &wh),
        &wh,
    )
    .unwrap();
    let summary = p.summary();
    assert!(summary.num_blocks > 1 && !summary.streaming, "{summary:?}");
    let interm = p
        .buffers
        .iter()
        .find(|b| matches!(b.kind, BufferKind::Interm { .. }))
        .expect("a spill buffer");
    assert_eq!(
        interm.dtype,
        logicaffeine_compile::codegen_sva::fabric_target::Precision::Bf16,
        "a spill-rounding program spills in bf16"
    );
    let acts: Vec<f64> = (0..m * k)
        .map(|i| round_bf16(prng(41, i, -800, 800) as f64 / 64.0))
        .collect();
    let weights: Vec<f64> = (0..k * n)
        .map(|i| round_bf16(prng(42, i, -800, 800) as f64 / 64.0))
        .collect();
    let mut inputs = Inputs::default();
    inputs
        .tensors
        .insert(TensorId(0), Value::Float(acts.clone()));
    inputs
        .tensors
        .insert(TensorId(1), Value::Float(weights.clone()));
    let got = match &eval_choreography(&p, &inputs).unwrap().tensors[&output_id(&p)] {
        Value::Float(v) => v.clone(),
        other => panic!("{other:?}"),
    };
    // Reference: f32 accumulation inside each K block, rounded to bf16 at every spill and at the output.
    let kb = summary.block.unwrap().ibw * 32;
    let mut reference = vec![0f64; m * n];
    for i in 0..m {
        for j in 0..n {
            let mut acc = 0f32;
            let mut k0 = 0;
            while k0 < k {
                let k1 = (k0 + kb).min(k);
                let mut part = acc;
                for kk in k0..k1 {
                    part += acts[i * k + kk] as f32 * weights[kk * n + j] as f32;
                }
                acc = round_bf16(part as f64) as f32;
                k0 = k1;
            }
            reference[i * n + j] = acc as f64;
        }
    }
    assert_eq!(
        got, reference,
        "declared spill rounding must be modeled at exactly the block boundaries: {summary:?}"
    );
    let exact = eval_systolic_bf16(&weights, &acts, m, k, n);
    assert_ne!(
        got, exact,
        "the witness must actually round somewhere, else the test is vacuous"
    );
}

#[test]
fn results_are_scheduler_independent() {
    let sky = SpatialTarget::sky_fabric(64, 64);
    let fusing = fusing_wormhole();
    for case in 0..50usize {
        let (target, m, n, k) = if case % 2 == 0 {
            (
                &sky,
                prng(51, case, 1, 16) as usize,
                prng(52, case, 1, 16) as usize,
                prng(53, case, 1, 16) as usize,
            )
        } else {
            (
                &fusing,
                32 * prng(54, case, 1, 4) as usize,
                32 * prng(55, case, 1, 4) as usize,
                32 * prng(56, case, 1, 3) as usize,
            )
        };
        let (p, inputs, _, _) = int_program(m, n, k, 700 + case as u64, target);
        let a = eval_choreography(&p, &inputs).unwrap();
        let c = eval_choreography_confluent(&p, &inputs)
            .unwrap_or_else(|f| panic!("case {case}: {f:?}"));
        assert_eq!(
            a.tensors, c.tensors,
            "case {case}: five schedulers must agree"
        );
        for s in [
            Scheduler::Reverse,
            Scheduler::Seeded(case as u64),
            Scheduler::MovementFirst,
            Scheduler::ComputeFirst,
        ] {
            assert_eq!(
                eval_choreography_with(&p, &inputs, s).unwrap().tensors,
                a.tensors,
                "case {case} {s:?}"
            );
        }
    }
}

/// A fused 4×4-grid program small enough to evaluate exhaustively under mutation.
/// 128×128×256 with its plan stated: a 4×4 grid of single-tile sites, two K blocks of four tiles (so the
/// sender's counter reset and the spill path are load-bearing), readers fused along rows and columns.
fn small_fused() -> (SpatialProgram, Inputs, Vec<i64>) {
    use logicaffeine_compile::spatial::ir::{BlockPlan, BlockShape, InnerLoop};
    use logicaffeine_compile::spatial::passes::{
        fuse_readers::fuse_readers_forced, persist::persist, place::place,
        size_buffers::size_buffers, synthesize_handshakes::synthesize_handshakes, tile::apply_plan,
        verify::verify,
    };
    let fusing = fusing_wormhole();
    let (m, n, k) = (128usize, 128usize, 256usize);
    let spec = MatmulSpec {
        m,
        n,
        k,
        a: Default::default(),
        b: Default::default(),
        c: Default::default(),
    };
    let t = template_from_spec(
        &spec,
        &names(),
        (&Fill::Unknown, &Fill::Unknown),
        (Some((-128, 127)), Some((-128, 127))),
        NumericsPolicy::Exact,
        &fusing,
    )
    .unwrap();
    let plan = BlockPlan {
        per_site_m: 1,
        per_site_n: 1,
        grid: (4, 4),
        inner: InnerLoop {
            num_blocks: 2,
            block_k_tiles: 4,
        },
        block: BlockShape {
            osh: 1,
            osw: 1,
            ibw: 4,
        },
        acc: AccPolicy::F32,
        pack_accumulate: false,
        engines: EnginePacking::LoadsTogether,
        granularity: Granularity::PerTile,
    };
    let placed = place(&apply_plan(&t, plan, &fusing), &fusing).unwrap();
    let fused = fuse_readers_forced(&placed, &fusing, true).unwrap();
    let p = verify(
        &persist(
            &synthesize_handshakes(&size_buffers(&fused, &fusing).unwrap(), &fusing).unwrap(),
            &fusing,
        )
        .unwrap(),
        &fusing,
    )
    .unwrap();
    assert!(
        p.summary().multicast_groups > 0 && p.summary().num_blocks == 2,
        "{:?}",
        p.summary()
    );
    let acts: Vec<i64> = (0..m * k).map(|i| prng(77, i, -128, 127)).collect();
    let weights: Vec<i64> = (0..k * n).map(|i| prng(78, i, -128, 127)).collect();
    let mut inputs = Inputs::default();
    inputs.tensors.insert(TensorId(0), Value::Int(acts.clone()));
    inputs
        .tensors
        .insert(TensorId(1), Value::Int(weights.clone()));
    let reference = eval_systolic(&Systolic::new(k, n, weights), &acts, m);
    (p, inputs, reference)
}

fn small_streaming() -> (SpatialProgram, Inputs, Vec<i64>) {
    let wh = SpatialTarget::wormhole();
    let (p, inputs, acts, weights) = int_program(96, 64, 64, 78, &wh);
    let reference = eval_systolic(&Systolic::new(64, 64, weights), &acts, 96);
    (p, inputs, reference)
}

fn load_bearing(op: &ChoreoOp) -> bool {
    !matches!(
        op,
        ChoreoOp::ReadBarrier
            | ChoreoOp::WriteBarrier
            | ChoreoOp::Flush { .. }
            | ChoreoOp::Loop { .. }
    )
}

#[test]
fn a_sender_that_skips_the_ready_wait_is_caught() {
    let (p, inputs, _) = small_fused();
    let waits: Vec<_> = enumerate_ops(p.schedule.as_ref().unwrap())
        .into_iter()
        .filter(|(_, op)| matches!(op, ChoreoOp::SemWait { .. }))
        .collect();
    assert!(!waits.is_empty());
    let mut caught = 0;
    for (path, _) in &waits {
        let q = without_op(&p, path);
        match eval_choreography(&q, &inputs) {
            // A sender that skips its ready wait multicasts into an unreserved receiver; a receiver that skips
            // its valid wait publishes a page that never landed; either can also wedge the choreography.
            Err(ChoreoFault::McastIntoUnreserved { .. })
            | Err(ChoreoFault::Deadlock { .. })
            | Err(ChoreoFault::SemaphoreProtocol { .. })
            | Err(ChoreoFault::PushWithoutData { .. }) => caught += 1,
            Err(other) => {
                panic!("a skipped semaphore wait must surface as a protocol fault, got {other:?}")
            }
            Ok(_) => panic!("a skipped semaphore wait at {path:?} went undetected"),
        }
    }
    assert_eq!(caught, waits.len());
}

#[test]
fn dropping_any_load_bearing_op_is_never_silently_correct() {
    for (label, (p, inputs, reference)) in
        [("fused", small_fused()), ("streaming", small_streaming())]
    {
        let ops = enumerate_ops(p.schedule.as_ref().unwrap());
        assert!(ops.len() > 20, "{label}: a real choreography");
        let mut detected = 0;
        let mut harmless = 0;
        for (path, op) in &ops {
            let q = without_op(&p, path);
            let structural = q.validate().is_err();
            let outcome = eval_choreography(&q, &inputs);
            match outcome {
                Ok(out) => {
                    let got = as_ints(&out.tensors[&output_id(&q)]);
                    assert_eq!(got, reference, "{label}: dropping {op:?} at {path:?} was Ok with a WRONG answer — the fault model has a hole");
                    if load_bearing(op) {
                        assert!(structural, "{label}: dropping load-bearing {op:?} at {path:?} was Ok+correct and validate() did not object");
                        detected += 1;
                    } else {
                        harmless += 1;
                    }
                }
                Err(_) => detected += 1,
            }
        }
        assert!(
            detected > 0 && harmless > 0,
            "{label}: detected {detected}, harmless {harmless}"
        );
    }
}

#[test]
fn over_reserving_beyond_capacity_deadlocks_with_the_blocking_op_named() {
    let (p, inputs, _) = small_streaming();
    let reserve = enumerate_ops(p.schedule.as_ref().unwrap())
        .into_iter()
        .find(|(_, op)| matches!(op, ChoreoOp::Reserve { .. }))
        .expect("a reserve");
    let (path, ChoreoOp::Reserve { buf, .. }) = reserve else {
        unreachable!()
    };
    let depth = p.buffers[buf.idx()].kind.depth_pages();
    let q = with_op_replaced(
        &p,
        &path,
        ChoreoOp::Reserve {
            buf,
            pages: depth + 1,
        },
    );
    match eval_choreography(&q, &inputs) {
        Err(ChoreoFault::Deadlock { blocked_at, .. }) => {
            assert!(blocked_at.iter().any(|(_, _, _, op)| matches!(op, ChoreoOp::Reserve { pages, .. } if *pages == depth + 1)), "the deadlock names the over-reserve: {blocked_at:?}");
        }
        Err(ChoreoFault::BufferOverflow { .. }) => {}
        other => panic!("an over-reserve must deadlock or overflow, got {other:?}"),
    }
}

#[test]
fn accumulator_overflow_faults() {
    let (p, inputs, _) = small_streaming();
    let acq = enumerate_ops(p.schedule.as_ref().unwrap())
        .into_iter()
        .find(|(_, op)| matches!(op, ChoreoOp::AcquireAcc { .. }))
        .expect("an acquire");
    let q = with_op_replaced(
        &p,
        &acq.0,
        ChoreoOp::AcquireAcc {
            tiles: p.acc_tiles_used + 1,
        },
    );
    assert!(
        q.validate().is_err(),
        "an acquire beyond the accumulator file must fail validation"
    );
    assert!(matches!(
        eval_choreography(&q, &inputs),
        Err(ChoreoFault::AccOverflow { .. })
    ));
}

#[test]
fn every_output_tile_is_stored_exactly_once_and_conflicts_are_detected() {
    let (p, inputs, _) = small_streaming();
    let write = enumerate_ops(p.schedule.as_ref().unwrap())
        .into_iter()
        .find(|(_, op)| matches!(op, ChoreoOp::WriteTiles { .. }))
        .expect("a write");
    let dup = with_op_duplicated(&p, &write.0);
    assert!(
        matches!(
            eval_choreography(&dup, &inputs),
            Err(ChoreoFault::StoreConflict { .. })
                | Err(ChoreoFault::Underflow { .. })
                | Err(ChoreoFault::Deadlock { .. })
        ),
        "a duplicated store must fault"
    );
    let gone = without_op(&p, &write.0);
    assert!(
        matches!(eval_choreography(&gone, &inputs), Err(_)),
        "a dropped store must fault (a tile is never stored or a page is never consumed)"
    );
}

#[test]
fn the_trace_reports_buffer_high_water_within_depth() {
    let (p, inputs, _) = small_fused();
    let (out, trace) = eval_choreography_traced(&p, &inputs).unwrap();
    assert!(!out.tensors.is_empty());
    for b in &p.buffers {
        if let Some(hw) = trace.high_water.get(&b.id) {
            assert!(
                *hw <= b.kind.depth_pages(),
                "buffer {} occupancy {hw} exceeded its depth {}",
                b.name,
                b.kind.depth_pages()
            );
            assert!(*hw >= 1, "buffer {} was never used", b.name);
        }
    }
    assert!(trace.steps > 0);
}

#[test]
#[ignore = "2,000-mutation fuzz — minutes; run with --ignored"]
fn random_schedule_mutations_never_produce_a_silent_wrong_answer() {
    let mut mutations = 0usize;
    for case in 0..20usize {
        let (p, inputs, reference) = if case % 2 == 0 {
            small_fused()
        } else {
            small_streaming()
        };
        let ops = enumerate_ops(p.schedule.as_ref().unwrap());
        for round in 0..100usize {
            let pick = prng(1000 + case as u64, round, 0, ops.len() as i64 - 1) as usize;
            let (path, op) = &ops[pick];
            let q = match round % 3 {
                0 => without_op(&p, path),
                1 => with_op_duplicated(&p, path),
                _ => match op {
                    ChoreoOp::Reserve { buf, pages } => with_op_replaced(
                        &p,
                        path,
                        ChoreoOp::Reserve {
                            buf: *buf,
                            pages: pages + 1,
                        },
                    ),
                    ChoreoOp::Pop { buf, pages } => with_op_replaced(
                        &p,
                        path,
                        ChoreoOp::Pop {
                            buf: *buf,
                            pages: pages + 1,
                        },
                    ),
                    _ => with_op_duplicated(&p, path),
                },
            };
            mutations += 1;
            if let Ok(out) = eval_choreography(&q, &inputs) {
                let got = as_ints(&out.tensors[&output_id(&q)]);
                assert_eq!(got, reference, "case {case} round {round}: mutation of {op:?} at {path:?} was Ok with a wrong answer");
            }
        }
    }
    assert_eq!(mutations, 2000);
}

#[test]
fn template_from_source_with_a_relaxed_policy_packs_in_the_operand_width() {
    let wh = SpatialTarget::wormhole();
    let exact = template_from_source(&gemm_program(640, 640, 640), &wh).unwrap();
    let relaxed = template_from_source_with(
        &gemm_program(640, 640, 640),
        &wh,
        NumericsPolicy::AllowSpillRounding,
    )
    .unwrap();
    assert_eq!(
        exact.tensors[2].dtype,
        logicaffeine_compile::codegen_sva::fabric_target::Precision::Fp32
    );
    assert_eq!(
        relaxed.tensors[2].dtype,
        logicaffeine_compile::codegen_sva::fabric_target::Precision::Bf16
    );
    assert!(matches!(
        relaxed.claim,
        logicaffeine_compile::spatial::ir::NumericsClaim::Approx { .. }
    ));
    assert_eq!(
        relaxed.acc_tiles_used, 8,
        "no fp32 DST halving when the program accepts bf16 packing"
    );
    assert_eq!(exact.acc_tiles_used, 4);
}
