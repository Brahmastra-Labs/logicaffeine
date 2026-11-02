//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! FLOAT MATMUL OPERAND WIDTH — a float matmul takes the narrowest float operand its datapath offers.
//!
//! `choose_matmul_numerics` picks the operand precision a float matmul runs at: bf16, then fp16, then fp32 — the first
//! the target's matmul datapath takes — with an fp32 accumulator. A datapath that offers bf16 keeps it even beside
//! fp32 (the throughput width); a datapath whose ONLY float operand is fp32 lowers at fp32 rather than being refused;
//! a datapath with no float operand refuses a float matmul by name.
//!
//! Pinned:
//!   1. The chooser's order: SN50 (bf16) → bf16; bf16 beside fp32 → bf16; fp32 alone → fp32 operands, an fp32
//!      accumulator and result, EXACT inside fp32's mantissa; no float operand → `MissingCompute`.
//!   2. The fp32-only datapath COMPUTES at fp32: a GEMM over values no bf16 holds exactly (tenths) equals the VM to one
//!      part in 10⁶ on the fp32 datapath — and the bf16 SN50 is measurably off on the same program, so the comparison
//!      can tell the widths apart.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test spatial_float_matmul_width_lock`
//!
//! ★ MUTATION PROBE (private copy, RED 2026-09-26): the chooser's float order without fp32 (`[Bf16, Fp16]`) → both
//!   tests: `MissingCompute { needed: "tile matmul" }` on the fp32-only datapath.
//! ════════════════════════════════════════════════════════════════════════════════════════════════
#![cfg(feature = "codegen")]

#[path = "compile_common/tensor_probes.rs"]
mod tensor_probes;

use logicaffeine_compile::codegen_sva::fabric_target::Precision;
use logicaffeine_compile::compile::vm_outcome;
use logicaffeine_compile::spatial::eval::eval_choreography;
use logicaffeine_compile::spatial::ir::{AccPolicy, NumericsClaim};
use logicaffeine_compile::spatial::passes::from_tensor_op::choose_matmul_numerics;
use logicaffeine_compile::spatial::passes::SpatialReject;
use logicaffeine_compile::spatial::pipeline::compile_program_to_spatial;
use logicaffeine_compile::spatial::target::{ComputeCap, SpatialTarget};
use tensor_probes::{inputs_from_fills, oracle_ints, shown_from_outputs};

/// The SN50 with its matmul datapath's float operands replaced by `floats`.
fn with_matmul_floats(floats: &[Precision]) -> SpatialTarget {
    let mut t = SpatialTarget::sn50_rdu();
    for c in &mut t.compute {
        if let ComputeCap::MatmulTile { precisions, .. } = c {
            precisions
                .retain(|p| !matches!(p, Precision::Bf16 | Precision::Fp16 | Precision::Fp32));
            precisions.extend(floats.iter().copied());
        }
    }
    for p in floats {
        t.precisions.insert(*p);
    }
    t
}

/// The SN50 with fp32 datapaths: fp32 in place of bf16 on the matmul, fp32 beside it elsewhere.
fn fp32_datapaths() -> SpatialTarget {
    let mut t = with_matmul_floats(&[Precision::Fp32]);
    for c in &mut t.compute {
        if let ComputeCap::Elementwise { precisions } | ComputeCap::RowReduce { precisions } = c {
            if precisions.contains(&Precision::Bf16) {
                precisions.insert(Precision::Fp32);
            }
        }
    }
    t
}

#[test]
fn a_float_matmul_takes_the_narrowest_operand_its_datapath_offers() {
    // Operands of unknown range: no integer path fits, so the float order alone decides.
    let pick = |t: &SpatialTarget| choose_matmul_numerics(t, None, None, 64);
    let (operand, acc, out, _) =
        pick(&SpatialTarget::sn50_rdu()).expect("the SN50 multiplies floats");
    assert_eq!(
        (operand, acc, out),
        (Precision::Bf16, AccPolicy::F32, Precision::Fp32),
        "the SN50: bf16 operands, fp32 accumulate"
    );

    let (operand, ..) =
        pick(&with_matmul_floats(&[Precision::Bf16, Precision::Fp32])).expect("bf16 beside fp32");
    assert_eq!(
        operand,
        Precision::Bf16,
        "a datapath offering bf16 keeps it beside fp32 — the throughput width"
    );

    let (operand, acc, out, claim) = pick(&with_matmul_floats(&[Precision::Fp32]))
        .expect("an fp32-only datapath multiplies floats");
    assert_eq!(
        (operand, acc, out),
        (Precision::Fp32, AccPolicy::F32, Precision::Fp32),
        "fp32 alone: fp32 operands, accumulator and result"
    );
    assert!(
        matches!(claim, NumericsClaim::Approx { .. }),
        "operands of unknown range claim nothing exact: {claim:?}"
    );

    // Small integer operands whose products and sums fit fp32's 24-bit mantissa are exact on it.
    let (operand, _, _, claim) = choose_matmul_numerics(
        &with_matmul_floats(&[Precision::Fp32]),
        Some(3000),
        Some(5),
        64,
    )
    .expect("fp32");
    assert_eq!(
        (operand, claim),
        (Precision::Fp32, NumericsClaim::Exact),
        "|a|·|b|·k = 960000 < 2^24 is exact at fp32 (and past bf16's 256)"
    );

    let none = choose_matmul_numerics(&with_matmul_floats(&[]), None, None, 64);
    assert!(
        matches!(none, Err(SpatialReject::MissingCompute { .. })),
        "no float operand refuses a float matmul by name: {none:?}"
    );
}

fn tenths_gemm() -> String {
    let (m, k, n) = (3usize, 40usize, 5usize);
    let mut s = String::from("## Main\n");
    for (name, rows, cols, salt) in [("A", m, k, 3usize), ("B", k, n, 7)] {
        s += &format!("Let {name} be a {rows} by {cols} tensor of Real.\n");
        for i in 0..rows * cols {
            let v = (((i * 37 + salt) % 19) as f64 - 9.0) / 10.0;
            s += &format!("Set item {} of {name} to {v:?}.\n", i + 1);
        }
    }
    s += "Let C be A matmul B.\n";
    for i in 1..=m * n {
        s += &format!("Show round(item {i} of C * 1000000).\n");
    }
    s
}

/// The program's shown values on `target`, and the VM's.
fn shown_on(src: &str, target: &SpatialTarget) -> (Vec<i64>, Vec<i64>) {
    let vm = vm_outcome(src);
    assert_eq!(vm.error, None, "the VM runs the GEMM");
    let want = oracle_ints(&vm.output).expect("integers");
    let pl = compile_program_to_spatial(src, target)
        .unwrap_or_else(|e| panic!("{}: {e:?}", target.name));
    let p = pl
        .whole()
        .unwrap_or_else(|| panic!("{}: {}", target.name, pl.refusal()))
        .clone();
    let got = shown_from_outputs(
        src,
        &p,
        &eval_choreography(&p, &inputs_from_fills(&p).expect("fills")).expect("evaluates"),
    )
    .expect("shown");
    (got, want)
}

#[test]
fn an_fp32_only_matmul_datapath_computes_at_fp32() {
    let src = tenths_gemm();
    let (got, want) = shown_on(&src, &fp32_datapaths());
    for (i, (g, v)) in got.iter().zip(&want).enumerate() {
        assert!((g - v).abs() <= 1, "Show #{}: the fp32 datapath gives {g}, the VM {v} — fp32 operands hold tenths to one part in 10⁷", i + 1);
    }
    let (bf16, _) = shown_on(&src, &SpatialTarget::sn50_rdu());
    let worst = bf16
        .iter()
        .zip(&want)
        .map(|(g, v)| (g - v).abs())
        .max()
        .unwrap_or(0);
    assert!(worst > 100, "the bf16 SN50 must be measurably off on tenths (worst {worst}) — or this lock cannot tell the widths apart");
}
