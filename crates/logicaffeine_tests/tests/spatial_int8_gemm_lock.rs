//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! INT8 GEMM LOCK — an int8-range matmul is EXACT on a float-primary engine, element for element
//! against the VM, because it runs as bf16 operands with an fp32 accumulator and never touches the
//! int8 matmul path.
//!
//! ★ THE PATH THIS REPLACES WAS MEASURED WRONG. `codegen_tt/matrix.rs` emitted an int8
//! `matmul_tiles`, and that instruction scored **0 of 4096 elements correct** under ttsim on BOTH
//! Wormhole and Blackhole — the finding is recorded on the target itself
//! (`spatial/target.rs`, the doc comment on `wormhole()`), which is why `Precision::Int8` is
//! deliberately ABSENT from that target's tile precisions. `tt_matrix_lock` asserts the substrings of
//! the kernel that produced those wrong numbers; it is approved for deletion AS WRONG. This file is
//! what takes its place, and it asserts the numbers rather than the kernel text.
//!
//! ★ WHY bf16 IS EXACT HERE, WHICH IS THE WHOLE ARGUMENT. bf16 carries 8 mantissa bits, so every
//! integer up to 256 is represented exactly — an int8 operand is not approximated, it is stored. The
//! products land in an fp32 accumulator with 24 mantissa bits, so a sum of `k` products each bounded
//! by `a·b` is exact while `a·b·k < 2^24`. `choose_matmul_numerics` computes precisely that bound and
//! only then claims `NumericsClaim::Exact`. So this is not "close enough for int8": it is the same
//! integer answer, and the assertions below are equality, not tolerance.
//!
//! Run: `cargo nextest run -p logicaffeine-tests -E 'binary(spatial_int8_gemm_lock)'`
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

mod common;
#[path = "common/spatial.rs"]
mod spatial_support;

use logicaffeine_compile::codegen_sva::fabric_target::Precision;
use logicaffeine_compile::compile::vm_outcome;
use logicaffeine_compile::spatial::eval::{eval_choreography, Value};
use logicaffeine_compile::spatial::ir::{AccPolicy, NumericsClaim, Phase};
use logicaffeine_compile::spatial::passes::from_tensor_op::choose_matmul_numerics;
use logicaffeine_compile::spatial::pipeline::compile_to_spatial;
use logicaffeine_compile::spatial::target::SpatialTarget;
use spatial_support::{
    checksum, gemm_program, gemm_reference, inputs_from_fills, output_id, shown_int,
};

/// The elements of an output tensor as integers, whichever tower it came back in. A float datapath
/// that is EXACT over integer data returns whole numbers, and this asserts that rather than assuming
/// it: a fractional element means the "exact" claim was false and the round below would hide it.
fn as_exact_ints(v: &Value, label: &str) -> Vec<i64> {
    match v {
        Value::Int(x) => x.clone(),
        Value::Float(x) => x
            .iter()
            .map(|f| {
                assert_eq!(
                    *f,
                    f.round(),
                    "{label}: the fp32 accumulator returned {f}, which is not a whole number — the \
                     `NumericsClaim::Exact` this lock relies on is FALSE for this shape. Do not round \
                     it away: either the operand bound or the `a·b·k < 2^24` test in \
                     `choose_matmul_numerics` is wrong."
                );
                *f as i64
            })
            .collect(),
    }
}

/// ★ THE HARDWARE FACT THIS LOCK IS BUILT ON. If a target ever re-declares int8 tile matmul, the
/// numerics chooser will prefer it over bf16 (it tries integer precisions first) and this whole lock
/// silently starts measuring a different, measured-wrong path. So the absence is asserted, with the
/// evidence, rather than assumed.
#[test]
fn the_float_primary_targets_do_not_offer_an_int8_tile_matmul() {
    for target in [SpatialTarget::wormhole(), SpatialTarget::blackhole()] {
        assert!(
            !target.matmul_supports(Precision::Int8),
            "★ {} re-declared an int8 tile matmul. That instruction measured 0/4096 elements correct \
             under ttsim on both Wormhole and Blackhole — see the doc comment on `SpatialTarget::wormhole`. \
             `choose_matmul_numerics` tries integer precisions FIRST, so re-adding it silently routes \
             every int8 GEMM back onto the wrong path and makes this lock measure something else. \
             If silicon or a newer tt-metal fixes it, re-add it WITH the measurement that shows it fixed.",
            target.name
        );
    }
}

/// The numerics decision itself: int8-range operands on a float-primary engine must choose bf16
/// operands, an fp32 accumulator, and claim EXACT.
#[test]
fn an_int8_range_gemm_chooses_bf16_with_an_fp32_accumulator_and_claims_exact() {
    let wh = SpatialTarget::wormhole();
    // The corpus fills are `p % 7 - 3` and `p % 11 - 5`, so |a| <= 3 and |b| <= 5 — int8 range by a
    // wide margin, which is what makes these shapes the honest subject for an int8 lock.
    for (k, bound_a, bound_b) in [(64usize, 3i64, 5i64), (128, 127, 127), (256, 127, 127)] {
        let (operand, acc, out, claim) =
            choose_matmul_numerics(&wh, Some(bound_a), Some(bound_b), k).unwrap_or_else(|e| {
                panic!("k={k}: the numerics chooser refused an int8-range GEMM: {e:?}")
            });
        assert_eq!(
            operand,
            Precision::Bf16,
            "k={k}: int8 range must run on bf16 operands"
        );
        assert_eq!(
            acc,
            AccPolicy::F32,
            "k={k}: the products must accumulate in fp32"
        );
        assert_eq!(out, Precision::Fp32, "k={k}");
        let product_bound = bound_a * bound_b * k as i64;
        if product_bound < (1 << 24) {
            assert_eq!(
                claim,
                NumericsClaim::Exact,
                "k={k}: a·b·k = {product_bound} < 2^24, so every partial sum fits fp32's mantissa \
                 exactly and the claim must be Exact. An Approx claim here would be the compiler \
                 under-promising on a path it can prove."
            );
        } else {
            assert!(
                matches!(claim, NumericsClaim::Approx { .. }),
                "k={k}: a·b·k = {product_bound} exceeds 2^24, so the claim must NOT be Exact — \
                 over-promising is the failure that matters here."
            );
        }
    }
}

/// The numbers. Element for element against the VM — not a checksum, which can agree by cancellation.
#[test]
fn an_int8_range_gemm_equals_the_vm_element_for_element() {
    let wh = SpatialTarget::wormhole();
    for (m, n, k) in [(64usize, 64usize, 64usize), (96, 160, 128), (100, 70, 45)] {
        let src = gemm_program(m, n, k);
        let p = compile_to_spatial(&src, &wh).unwrap_or_else(|e| panic!("{m}x{n}x{k}: {e:?}"));
        assert_eq!(p.phase(), Phase::Verified, "{m}x{n}x{k}");
        assert_eq!(
            p.claim,
            NumericsClaim::Exact,
            "{m}x{n}x{k}: this lock only means something on a program that CLAIMS exactness — an \
             Approx program would make the equality below a coincidence rather than a contract."
        );

        let out = eval_choreography(&p, &inputs_from_fills(&p))
            .unwrap_or_else(|f| panic!("{m}x{n}x{k}: {f:?}"));
        let got = as_exact_ints(&out.tensors[&output_id(&p)], &format!("{m}x{n}x{k}"));
        let want = gemm_reference(m, n, k);

        // ★ ELEMENT FOR ELEMENT, NOT BY CHECKSUM. A sum can agree while its terms do not — two
        // compensating sign errors cancel perfectly in a total. The checksum is asserted too, but
        // only after the elements, so a failure says WHICH element rather than "the totals differ".
        assert_eq!(got.len(), want.len(), "{m}x{n}x{k}: output length");
        if let Some(i) = got.iter().zip(&want).position(|(a, b)| a != b) {
            panic!(
                "\n★ int8-range GEMM {m}x{n}x{k} disagrees with the VM at element {i}: got {}, want {}\n\
                 ({} of {} elements differ)\n\n\
                 The program claims `NumericsClaim::Exact`, so this is a WRONG ANSWER, not a rounding \
                 difference. bf16 stores every integer to 256 exactly and fp32 sums them exactly while \
                 a·b·k < 2^24 — if that reasoning still holds, the fault is in the lowering or the \
                 evaluator, not in the numerics choice. Never relax this to a tolerance.\n",
                got[i],
                want[i],
                got.iter().zip(&want).filter(|(a, b)| a != b).count(),
                got.len()
            );
        }
        let vm = vm_outcome(&src);
        assert!(vm.error.is_none(), "{m}x{n}x{k}: VM error: {:?}", vm.error);
        assert_eq!(
            checksum(&got),
            shown_int(&vm.output),
            "{m}x{n}x{k}: checksum vs the VM"
        );
    }
}

/// Non-vacuity: the comparison above can actually detect a wrong answer.
#[test]
fn a_perturbed_result_is_caught() {
    let (m, n, k) = (64usize, 64usize, 64usize);
    let want = gemm_reference(m, n, k);
    let mut wrong = want.clone();
    wrong[7] += 1;
    assert_ne!(
        wrong, want,
        "the reference is degenerate — a one-element perturbation must be visible, or \
         `an_int8_range_gemm_equals_the_vm_element_for_element` proves nothing"
    );
    assert_ne!(
        checksum(&wrong),
        checksum(&want),
        "the checksum cannot see a single-element error, so the element-wise comparison above is \
         load-bearing rather than decorative"
    );
}
