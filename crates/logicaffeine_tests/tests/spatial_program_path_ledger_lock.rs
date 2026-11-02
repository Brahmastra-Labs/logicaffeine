//! spatial_program_path_ledger_lock — WHICH tensor ops reach the SN50 RDU through the path a real model
//! takes: the whole-program walk (`compile_program_to_spatial`), composition, one certificate over the
//! composed choreography, the SDK-gated neutral artifact, and an evaluation that shows what the VM shows.
//!
//! `spatial_op_gap_ledger_lock` asks the SINGLE-OP front end (`template_from_source`), which lifts one sugar
//! as one program named after it. That path was never how Qwen3.8 reaches the chip: every real layer goes
//! through the whole-program walk, where a sugar is recognized as a typed stage and lowered as sections of
//! the closed algebra. So the single-op ledger reports ops like attention and gated delta as unreachable
//! while the model built from them lowers, certifies and runs. This ledger measures the path that is used.
//!
//! One cell per op, `Neutral` on the program path, which holds only if ALL of:
//!   • the walk lifts every statement (`ProgramLowering::whole()` is `Some`: no statement left unlifted);
//!   • the composed program is `Phase::Verified` — its own certificate, not a section's;
//!   • `RduSpatialBackend::for_chip(Sn50).lower` is `SdkGated`, and the shipped certificate rechecks against
//!     the program;
//!   • the answer is right, in two parts — because a composed program on a bf16 datapath rounds at EVERY
//!     section, and one ulp of the final value (the single-op rule) is not the error model of a chain:
//!       – WIRING: the SAME source, lowered by the SAME compiler for an SN50 descriptor that also offers Fp32
//!         (`fp32_twin`), agrees with the VM to a relative 2⁻¹⁶ — 1/256 of one bf16 ulp. A wrong dataflow is
//!         off by O(1); f32 rounding, even through layernorm's variance cancellation, is not (MEASURED: every
//!         op exact but `fused_add_rmsnorm` 3.5e-7 and `layernorm` 4.05e-6). So whatever the bf16 program
//!         then misses by is rounding, not wiring.
//!       – CHIP PRECISION: the bf16 program's every shown value is within `r` ulps (at its dtype, at the
//!         VM value's magnitude) of the VM, `r` = the roundings on its path, bounded by the program's
//!         sections + 1 (the input rounding) — the first-order bound for a chain of correctly rounded steps
//!         (MEASURED: the worst op is 0.86 bf16 ulps, `attention`, against a bound of 4). Integer `Show`s stay
//!         exact.
//!
//! The teeth:
//!   • `every_op_reaches_the_rdu_on_the_program_path_or_is_a_listed_gap` + `no_gap_masks_a_cell_that_works`
//!     make `PROGRAM_PATH_GAPS` EXACTLY the live set of failing ops;
//!   • `the_ledger_only_shrinks` freezes the row count; `PROGRAM_PATH_FLOOR` is the count of ops that WORK
//!     and `the_floor_is_the_live_count` pins it exactly, so the kernel census can read it and it can only rise;
//!   • `the_program_path_verdict_has_teeth` — a composed program handed a wrong evaluation is refused;
//!   • `the_float_rules_and_the_certificate_have_teeth` — ten bf16 ulps off breaks the chain bound, the ulp is
//!     pinned, and a composed program's certificate does not recheck against another op's program;
//!   • `a_cell_is_a_real_composition_not_an_empty_program` — every working cell composed at least one
//!     section per lifted statement, and its output is the tensor the program shows.
//!
//! ★ MUTATION PROBES (each run 2026-09-23 — break, observe red, restore):
//!   • `fp32_twin` returns the plain SN50 descriptor (no widening) → every float op fails WIRING (e.g.
//!     `sigmoid`: 878906 vs VM 880797, past 2⁻¹⁶) and the teeth test reds — the twin is load-bearing;
//!   • the chain bound allows zero roundings → every float op fails CHIP PRECISION with its real 0.04–0.48
//!     bf16-ulp error named — the bound is what admits them, not slack elsewhere.
//!
//!   cargo nextest run -p logicaffeine-tests --test spatial_program_path_ledger_lock
#![cfg(feature = "codegen")]

#[path = "compile_common/tensor_probes.rs"]
mod tensor_probes;

use logicaffeine_compile::codegen_sva::fabric_target::Precision;
use logicaffeine_compile::compile::vm_outcome;
use logicaffeine_compile::spatial::eval::{eval_choreography, Outputs, Value};
use logicaffeine_compile::spatial::ir::{Phase, SpatialProgram};
use logicaffeine_compile::spatial::lower::rdu::RduSpatialBackend;
use logicaffeine_compile::spatial::lower::{SpatialArtifact, SpatialBackend};
use logicaffeine_compile::spatial::pipeline::{compile_program_to_spatial, ProgramLowering};
use logicaffeine_compile::spatial::rdu_chip::RduChip;
use logicaffeine_compile::spatial::target::{ComputeCap, SpatialTarget};
use tensor_probes::*;

fn target() -> SpatialTarget {
    SpatialTarget::sn50_rdu()
}

/// The SN50 descriptor with Fp32 added wherever it offers a float — the compiler then carries every
/// activation at Fp32 (it picks the widest float a capability offers), so the lowering of a source here is
/// the wide twin of its SN50 lowering: same recognizers, same passes, same composition, wider rounding.
fn fp32_twin() -> SpatialTarget {
    let mut t = target();
    for c in &mut t.compute {
        if let ComputeCap::MatmulTile { precisions, .. }
        | ComputeCap::Elementwise { precisions }
        | ComputeCap::RowReduce { precisions } = c
        {
            if precisions.contains(&Precision::Bf16) {
                precisions.insert(Precision::Fp32);
            }
        }
    }
    t.precisions.insert(Precision::Fp32);
    t
}

fn backend() -> RduSpatialBackend {
    RduSpatialBackend::for_chip(RduChip::Sn50)
}

/// SHRINK-ONLY: every op that does not reach the SN50 artifact on the program path today, with the piece it
/// is waiting on. Exactly the live failing set — see the two ledger tests.
const PROGRAM_PATH_GAPS: &[(&str, &str)] = &[];

/// Today's row count. Re-freeze DOWNWARD when ops start working; never up.
const FROZEN_PROGRAM_PATH_GAP_COUNT: usize = 0;

/// The ops that reach the SN50 artifact on the program path — `OP_PROBES.len() - PROGRAM_PATH_GAPS.len()`,
/// pinned exactly so `kernel_registry_census_lock` reads a proven number. Raise-only.
const PROGRAM_PATH_FLOOR: usize = 35;

/// The universe the floor is out of: the probe table. Pinned as a literal so the census reads it from here.
const PROGRAM_PATH_UNIVERSE: usize = 35;

fn first_line(s: &str) -> &str {
    s.lines().next().unwrap_or(s)
}

fn lowering_on(p: &OpProbe, t: &SpatialTarget) -> Result<ProgramLowering, String> {
    compile_program_to_spatial(p.program, t)
        .map_err(|e| format!("walk: {}", first_line(&format!("{e:?}"))))
}

fn lowering(p: &OpProbe) -> Result<ProgramLowering, String> {
    lowering_on(p, &target())
}

/// The composed, verified program for a probe — or why there is none.
fn composed(p: &OpProbe) -> Result<SpatialProgram, String> {
    composed_on(p, &target())
}

fn composed_on(p: &OpProbe, t: &SpatialTarget) -> Result<SpatialProgram, String> {
    let pl = lowering_on(p, t)?;
    let Some(sp) = pl.whole() else {
        return Err(format!("partial: {}", first_line(&pl.refusal())));
    };
    if sp.phase() != Phase::Verified {
        return Err(format!("phase {:?}, not Verified", sp.phase()));
    }
    Ok(sp.clone())
}

/// One unit in the last place of `x` at precision `p` (normal range; a zero or subnormal magnitude takes the
/// smallest normal's ulp).
fn ulp(p: Precision, x: f64) -> f64 {
    let (mant_bits, min_exp) = match p {
        Precision::Bf16 => (7, -126),
        Precision::Fp16 => (10, -14),
        Precision::Fp32 => (23, -126),
        _ => return 0.0,
    };
    let e = if x == 0.0 {
        min_exp
    } else {
        (x.abs().log2().floor() as i32).max(min_exp)
    };
    2f64.powi(e - mant_bits)
}

/// CHIP PRECISION: every shown value within `roundings` ulps of the VM at the shown tensor's dtype. Integer
/// `Show`s exact; lengths must match.
fn within_chain_bound(
    program: &str,
    sp: &SpatialProgram,
    outputs: &Outputs,
    oracle_stdout: &str,
    roundings: usize,
) -> Result<(), String> {
    let got = shown_from_outputs(program, sp, outputs)?;
    let vm = oracle_ints(oracle_stdout)?;
    let shows = shows_of(program);
    if got.len() != vm.len() || vm.len() != shows.len() {
        return Err(format!(
            "spatial showed {} values, the VM {}, the program has {} Shows",
            got.len(),
            vm.len(),
            shows.len()
        ));
    }
    for (i, ((g, v), s)) in got.iter().zip(&vm).zip(&shows).enumerate() {
        match s.scale {
            None if g != v => {
                return Err(format!(
                    "Show #{}: spatial {g} vs VM {v} (exact integer)",
                    i + 1
                ))
            }
            None => {}
            Some(scale) => {
                let dtype = shown_tensor(sp, &s.tensor)?.dtype;
                let x = *v as f64 / scale;
                let bound = (roundings as f64 * ulp(dtype, x) * scale).max(1.0);
                if ((*g - v).abs() as f64) > bound {
                    return Err(format!(
                        "Show #{}: spatial {g} vs VM {v} — {:.2} ulps of {dtype:?} at {x}, past the {roundings}-rounding chain bound",
                        i + 1,
                        (*g - v).abs() as f64 / (ulp(dtype, x) * scale)
                    ));
                }
            }
        }
    }
    Ok(())
}

/// The wiring check's relative tolerance: 2⁻¹⁶, 1/256 of a bf16 ulp.
const WIRING_REL: f64 = 1.0 / 65536.0;

/// WIRING: the Fp32 twin of the probe's lowering agrees with the VM to `WIRING_REL` (integers exactly).
fn twin_agrees(p: &OpProbe, stdout: &str) -> Result<(), String> {
    let twin = composed_on(p, &fp32_twin()).map_err(|e| format!("fp32 twin: {e}"))?;
    let out = eval_choreography(&twin, &inputs_from_fills(&twin)?)
        .map_err(|e| format!("fp32 twin eval: {e:?}"))?;
    let got = shown_from_outputs(p.program, &twin, &out)?;
    let vm = oracle_ints(stdout)?;
    let shows = shows_of(p.program);
    if got.len() != vm.len() || vm.len() != shows.len() {
        return Err(format!(
            "fp32 twin showed {} values, the VM {}, the program has {} Shows",
            got.len(),
            vm.len(),
            shows.len()
        ));
    }
    for (i, ((g, v), s)) in got.iter().zip(&vm).zip(&shows).enumerate() {
        let bound = match s.scale {
            None => 0.0,
            Some(scale) => (WIRING_REL * (*v as f64 / scale).abs() * scale).max(1.0),
        };
        if ((*g - v).abs() as f64) > bound {
            return Err(format!(
                "fp32 twin (wiring): Show #{}: {g} vs VM {v}, past a relative {WIRING_REL:e}",
                i + 1
            ));
        }
    }
    Ok(())
}

/// The Neutral criterion on a composed program and an evaluation of it. Takes the evaluation as an argument
/// so the teeth test can hand it a wrong one.
fn neutral(p: &OpProbe, sp: &SpatialProgram, outputs: &Outputs) -> Result<(), String> {
    let artifact = backend().lower(sp).map_err(|e| format!("lower: {e:?}"))?;
    let SpatialArtifact::SdkGated(n) = artifact else {
        return Err("the RDU lane did not produce the SDK-gated neutral artifact".into());
    };
    if !n.certificate.recheck(sp) {
        return Err("the shipped certificate does not recheck against the program".into());
    }
    let vm = vm_outcome(p.program);
    if let Some(e) = vm.error.as_deref() {
        return Err(format!("VM: {}", first_line(e)));
    }
    let floats = sp
        .tensors
        .iter()
        .any(|t| matches!(t.dtype, Precision::Bf16 | Precision::Fp16 | Precision::Fp32));
    if !floats {
        return agrees_with_oracle(p.program, sp, outputs, &vm.output);
    }
    twin_agrees(p, &vm.output)?;
    let sections = lowering(p)?.sections;
    within_chain_bound(p.program, sp, outputs, &vm.output, sections + 1)
}

fn cell(p: &OpProbe) -> Result<(), String> {
    let sp = composed(p)?;
    let inputs = inputs_from_fills(&sp)?;
    let out = eval_choreography(&sp, &inputs).map_err(|e| format!("eval: {e:?}"))?;
    neutral(p, &sp, &out)
}

fn is_gap(op: &str) -> bool {
    PROGRAM_PATH_GAPS.iter().any(|(o, _)| *o == op)
}

#[test]
fn every_op_reaches_the_rdu_on_the_program_path_or_is_a_listed_gap() {
    let mut violations = Vec::new();
    for p in OP_PROBES {
        if is_gap(p.op) {
            continue;
        }
        if let Err(why) = cell(p) {
            violations.push(format!(
                "{}: FAILS on the program path but is not a listed gap — {why}",
                p.op
            ));
        }
    }
    assert!(
        violations.is_empty(),
        "program-path coverage regressed on {}:\n{}",
        target().name,
        violations.join("\n")
    );
}

#[test]
fn no_gap_masks_a_cell_that_works() {
    let fixed: Vec<String> = PROGRAM_PATH_GAPS
        .iter()
        .filter(|(op, _)| cell(probe(op)).is_ok())
        .map(|(op, _)| format!("{op} now WORKS on the program path — delete its row, re-freeze the count, raise the floor"))
        .collect();
    assert!(
        fixed.is_empty(),
        "stale gaps mask real coverage:\n{}",
        fixed.join("\n")
    );
}

#[test]
fn the_ledger_only_shrinks() {
    assert!(
        PROGRAM_PATH_GAPS.len() <= FROZEN_PROGRAM_PATH_GAP_COUNT,
        "the ledger GREW: {} rows > {FROZEN_PROGRAM_PATH_GAP_COUNT} — an op lost the program path; fix it, do not raise the freeze",
        PROGRAM_PATH_GAPS.len()
    );
}

#[test]
fn gaps_reference_real_ops_once_with_a_reason() {
    let mut seen = std::collections::BTreeSet::new();
    for &(op, reason) in PROGRAM_PATH_GAPS {
        assert!(
            OP_PROBES.iter().any(|p| p.op == op),
            "PROGRAM_PATH_GAPS names unknown op `{op}`"
        );
        assert!(
            !reason.trim().is_empty(),
            "{op}: a gap needs a reason naming the missing piece"
        );
        assert!(seen.insert(op), "{op} is listed twice");
    }
}

#[test]
fn the_universe_is_the_probe_table() {
    assert_eq!(
        PROGRAM_PATH_UNIVERSE,
        OP_PROBES.len(),
        "PROGRAM_PATH_UNIVERSE must be the probe table's size"
    );
}

#[test]
fn the_floor_is_the_live_count() {
    assert_eq!(
        PROGRAM_PATH_FLOOR,
        OP_PROBES.len() - PROGRAM_PATH_GAPS.len(),
        "PROGRAM_PATH_FLOOR must be exactly the ops that reach the artifact ({} probes − {} gaps)",
        OP_PROBES.len(),
        PROGRAM_PATH_GAPS.len()
    );
}

#[test]
fn a_cell_is_a_real_composition_not_an_empty_program() {
    let mut bad = Vec::new();
    for p in OP_PROBES.iter().filter(|p| !is_gap(p.op)) {
        let pl = lowering(p).expect("a working cell walks");
        if pl.lowered == 0 || pl.sections < pl.lowered {
            bad.push(format!(
                "{}: {} statements lowered into {} sections",
                p.op, pl.lowered, pl.sections
            ));
        }
        let sp = composed(p).expect("a working cell composes");
        for s in shows_of(p.program) {
            if let Err(e) = shown_tensor(&sp, &s.tensor) {
                bad.push(format!("{}: {e}", p.op));
            }
        }
    }
    assert!(
        bad.is_empty(),
        "cells that are not real compositions:\n{}",
        bad.join("\n")
    );
}

#[test]
fn the_program_path_verdict_has_teeth() {
    let p = OP_PROBES
        .iter()
        .find(|p| !is_gap(p.op) && p.expect.is_some())
        .expect("an exact op works on the program path");
    let sp = composed(p).expect("composes");
    let truth = eval_choreography(&sp, &inputs_from_fills(&sp).unwrap()).unwrap();
    assert_eq!(
        neutral(p, &sp, &truth),
        Ok(()),
        "{}: the true evaluation is Neutral",
        p.op
    );

    let shown = shown_tensor(&sp, &shows_of(p.program)[0].tensor)
        .unwrap()
        .id;
    let mut wrong = truth.clone();
    let nudged = match &truth.tensors[&shown] {
        Value::Int(v) => Value::Int(v.iter().map(|x| x + 1).collect()),
        Value::Float(v) => Value::Float(v.iter().map(|x| x + 1.0).collect()),
    };
    wrong.tensors.insert(shown, nudged);
    assert!(
        neutral(p, &sp, &wrong).is_err(),
        "{}: an evaluation off by one everywhere must be refused",
        p.op
    );

    let mut missing = truth.clone();
    missing.tensors.remove(&shown);
    assert!(
        neutral(p, &sp, &missing).is_err(),
        "{}: no value for the shown tensor is not agreement",
        p.op
    );
}

#[test]
fn the_float_rules_and_the_certificate_have_teeth() {
    // The ulp is the unit the chain bound is written in: pin it.
    assert_eq!(ulp(Precision::Bf16, 1.0), 2f64.powi(-7));
    assert_eq!(ulp(Precision::Bf16, 3.99), 2f64.powi(-6));
    assert_eq!(ulp(Precision::Fp32, 1.0), 2f64.powi(-23));

    // A float op on the bf16 datapath: its true evaluation is Neutral; the same evaluation nudged by ten bf16
    // ulps at the shown magnitude is far past its chain bound and must be refused.
    let p = probe("gated_delta");
    let sp = composed(p).expect("gated_delta composes on the program path");
    let truth = eval_choreography(&sp, &inputs_from_fills(&sp).unwrap()).unwrap();
    assert_eq!(
        neutral(p, &sp, &truth),
        Ok(()),
        "gated_delta: the true evaluation is Neutral"
    );
    let shown = shown_tensor(&sp, &shows_of(p.program)[0].tensor)
        .unwrap()
        .id;
    let Value::Float(v) = truth.tensors[&shown].clone() else {
        panic!("gated_delta shows a float")
    };
    let nudged: Vec<f64> = v
        .iter()
        .map(|x| x + 10.0 * ulp(Precision::Bf16, *x))
        .collect();
    let mut wrong = truth.clone();
    wrong.tensors.insert(shown, Value::Float(nudged));
    let refused = neutral(p, &sp, &wrong);
    assert!(
        refused.as_ref().is_err_and(|e| e.contains("chain bound")),
        "ten bf16 ulps off must break the chain bound: {refused:?}"
    );

    // The shipped certificate is about THIS program: another op's composed program does not recheck under it.
    let other = composed(probe("attention")).expect("attention composes on the program path");
    let SpatialArtifact::SdkGated(n) = backend().lower(&sp).expect("lowers") else {
        panic!("SDK-gated")
    };
    assert!(
        n.certificate.recheck(&sp),
        "the certificate rechecks against its own program"
    );
    assert!(
        !n.certificate.recheck(&other),
        "a certificate must not recheck against a different program"
    );
}
