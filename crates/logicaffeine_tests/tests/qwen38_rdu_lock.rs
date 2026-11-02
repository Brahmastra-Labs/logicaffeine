//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! ★ QWEN3.8 ON THE SN50 RDU ★ — the WHOLE authored forward as ONE certified spatial program.
//!
//! This is the lock the campaign exists for. `codegen_cuda::qwen38::gen_qwen38_lg` authors the entire
//! hybrid forward (Gated-DeltaNet layers + gated-attention layers + a dense SwiGLU FFN each, then the
//! final norm and LM head) as ordinary LOGOS tensor sugar. `spatial::pipeline::compile_program_to_spatial`
//! must lift EVERY op statement, lower each as a composition of the closed algebra, and compose them into
//! one `SpatialProgram` of sections that the SN50 descriptor hosts and the choreography certificate covers.
//!
//! ★ WHY THE COVERAGE ASSERTION IS THE POINT. Before this landed, the tensor lane called
//! `template_from_source`, which lifts the FIRST recognized op and lowers THAT — so this 100-op forward
//! came back as `matmul_1x4x8`, one q-projection, exit 0, with a full four-file bundle and nothing saying
//! so. A partial lowering reported as a whole one is the one failure this work cannot ship: the people who
//! read these bundles design the chip. So `statements_lowered == statements_total` is asserted here, and
//! `compile_to_rdu` refuses by name rather than emit a fraction.
//!
//! ★ AND WHY A CERTIFICATE IS NOT ENOUGH. `certificate.recheck()` proves the CHOREOGRAPHY is sound —
//! deadlock-free, buffer-balanced, every read after its write. It says nothing about whether the program
//! computes Qwen's logits. `the_whole_forward_computes_what_the_vm_computes` is the half that does: the
//! same `.lg`, run on the VM, must agree with `eval_choreography` over the composed program. See
//! `lowering-is-not-correctness` — a backend returning Ok proves a program was PRINTED.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test qwen38_rdu_lock`
//!
//! ★ MUTATION PROBE (each run once, broken → this lock reds → restored byte-identical):
//!   1. `compose`'s single-section name shortcut removed → `the_whole_forward_is_one_composed_program`
//!      reds (the program stops naming its section count).
//!   2. `compile_program_to_spatial` skips the post-compose `verify` → `…_carries_a_rechecking_certificate`
//!      reds with phase `Scheduled` (this is the defect this lock caught in the first place).
//!   3. A section dropped from the compose input → `the_coverage_is_total` reds on lowered != total.
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::codegen_cuda::qwen38::{gen_qwen38_lg, Qwen38Cfg};
use logicaffeine_compile::compile::vm_outcome;
use logicaffeine_compile::spatial::eval::{eval_choreography, Value};
use logicaffeine_compile::spatial::ir::{ComputeOp, NumericsClaim, StageKind};
use logicaffeine_compile::spatial::pipeline::compile_program_to_spatial;
use logicaffeine_compile::spatial::rdu_chip::RduChip;

#[path = "common/spatial.rs"]
mod spatial_support;
use spatial_support::inputs_from_fills;

/// The authored forward with a deterministic fill after every declaration, and the two logits the VM
/// oracle prints. The fill scheme is `qwen38_authored_lock`'s, verbatim, so both tiers see one program.
fn authored_with_fills(cfg: &Qwen38Cfg) -> String {
    let (authored, _f32w, _q8w) = gen_qwen38_lg(cfg, "/w");
    let mut src = String::new();
    for line in authored.lines() {
        src.push_str(line);
        src.push('\n');
        let Some(rest) = line.strip_prefix("Let ") else {
            continue;
        };
        let Some((var, tail)) = rest.split_once(" be a ") else {
            continue;
        };
        let Some(dims) = tail.strip_suffix(" tensor of Real.") else {
            continue;
        };
        let (r, cc) = dims.split_once(" by ").expect("R by C");
        let n = r.parse::<usize>().unwrap() * cc.parse::<usize>().unwrap();
        let salt = var.bytes().map(|b| b as u64).sum::<u64>();
        for i in 0..n {
            let h = (i as u64)
                .wrapping_mul(2654435761)
                .wrapping_add(salt.wrapping_mul(40503));
            let v = ((h % 97) as f64 / 97.0 - 0.5) * 0.4;
            src.push_str(&format!("Set item {} of {var} to {v:?}.\n", i + 1));
        }
    }
    src
}

/// The whole forward lowers onto the SN50 as ONE program of many sections — not one op of many.
#[test]
fn the_whole_forward_is_one_composed_program() {
    let cfg = Qwen38Cfg::tiny();
    let src = authored_with_fills(&cfg);
    let target = RduChip::Sn50.facts().spatial_target();
    let lowering = compile_program_to_spatial(&src, &target)
        .expect("the authored forward is a tensor program");
    let p = lowering.program.as_ref().unwrap_or_else(|| {
        panic!(
            "the whole forward must compose: {} of {} statements lowered into {} sections; missing: {:?}",
            lowering.lowered, lowering.statements, lowering.sections, lowering.missing
        )
    });

    assert!(
        lowering.sections >= lowering.statements,
        "every op statement contributes at least one section"
    );
    assert!(
        p.name.contains("_sections"),
        "a multi-section program names its section count: {}",
        p.name
    );
    assert!(
        p.name.starts_with("logits"),
        "the composed program is named for what it finally produces: {}",
        p.name
    );
    assert!(
        p.stages.len() > 100,
        "a 64-layer-shaped forward is hundreds of stages, got {}",
        p.stages.len()
    );
    println!(
        "Qwen3.8 (tiny) on {}: {} statements -> {} sections, {} stages, {} tensors",
        target.name,
        lowering.statements,
        lowering.sections,
        p.stages.len(),
        p.tensors.len()
    );
}

/// EVERY recognized statement is lowered. The campaign's honesty gate, as an assertion.
#[test]
fn the_coverage_is_total() {
    let src = authored_with_fills(&Qwen38Cfg::tiny());
    let target = RduChip::Sn50.facts().spatial_target();
    let lowering = compile_program_to_spatial(&src, &target).expect("a tensor program");
    assert_eq!(
        lowering.lowered, lowering.statements,
        "{} of {} statements lowered — the tensor lane must never report a fraction as the whole program; missing: {:?}",
        lowering.lowered, lowering.statements, lowering.missing
    );
    assert!(
        lowering.missing.is_empty(),
        "nothing may be left behind: {:?}",
        lowering.missing
    );
    assert!(
        lowering.statements >= 50,
        "the tiny config is still a whole forward, got {} statements",
        lowering.statements
    );
}

/// The composed choreography carries a certificate that rechecks against the program it ships with.
#[test]
fn the_whole_forward_carries_a_rechecking_certificate() {
    let src = authored_with_fills(&Qwen38Cfg::tiny());
    let target = RduChip::Sn50.facts().spatial_target();
    let p = compile_program_to_spatial(&src, &target)
        .expect("a tensor program")
        .program
        .expect("composes");
    let cert = p
        .certificate
        .as_ref()
        .unwrap_or_else(|| panic!("the composed program is certified, phase {:?}", p.phase()));
    assert!(
        cert.recheck(&p),
        "the certificate must recheck against the program it travels with"
    );
}

/// The carried state of the Gated-DeltaNet layers survives composition: the conv rings and the recurrent
/// memory are declared as tensors of the composed program, not invented per section.
#[test]
fn the_gated_deltanet_state_is_carried_by_the_composed_program() {
    let src = authored_with_fills(&Qwen38Cfg::tiny());
    let target = RduChip::Sn50.facts().spatial_target();
    let lowering = compile_program_to_spatial(&src, &target).expect("a tensor program");
    assert!(
        !lowering.state.is_empty(),
        "a hybrid forward carries Gated-DeltaNet state across its layers"
    );
    for needle in ["Csq0", "St0"] {
        assert!(
            lowering.state.iter().any(|s| s == needle),
            "the carried state names {needle}: {:?}",
            lowering.state
        );
    }
}

/// Every compute stage is a member of the CLOSED algebra — the anti-accretion property, on the biggest
/// program we have. A new workload must be a composition, never a new `ComputeOp`.
#[test]
fn every_compute_stage_is_a_chain_of_the_closed_algebra() {
    let src = authored_with_fills(&Qwen38Cfg::tiny());
    let target = RduChip::Sn50.facts().spatial_target();
    let p = compile_program_to_spatial(&src, &target)
        .expect("a tensor program")
        .program
        .expect("composes");
    let mut shapes: std::collections::BTreeMap<&'static str, usize> = Default::default();
    for s in &p.stages {
        if let StageKind::Compute(op) = &s.kind {
            let name = match op {
                ComputeOp::MatmulTiles { .. } => "MatmulTiles",
                ComputeOp::Generic { .. } => "Generic",
                ComputeOp::Requant { .. } => "Requant",
                ComputeOp::Bias { .. } => "Bias",
                ComputeOp::Relu => "Relu",
                ComputeOp::Scalar(_) => "Scalar",
            };
            *shapes.entry(name).or_default() += 1;
            assert!(
                matches!(op, ComputeOp::MatmulTiles { .. } | ComputeOp::Generic { .. }),
                "a tensor workload lowers to the general form or the matrix engine, got {name} in stage {}",
                s.name
            );
        }
    }
    assert!(!shapes.is_empty(), "the forward has compute stages");
    println!("compute shapes across the whole forward: {shapes:?}");
}

/// ★ THE HALF A CERTIFICATE CANNOT GIVE YOU. The choreography certificate proves the SCHEDULE is sound —
/// deadlock-free, buffer-balanced, every read after its write. It is silent on arithmetic. So the same
/// authored `.lg` runs on the VM and on the composed choreography and the LOGITS are compared, whole.
///
/// ★ AND THE COMPARISON IS NOT f64 EQUALITY, FOR A STATED REASON. The SN50 witnesses `{Int8, Bf16}`, so
/// the composed forward runs a bf16 datapath while the VM oracle is f64. MEASURED here: the whole-forward
/// logits land within ~1.1% relative on `sn50-rdu` and ~0.3% on `wormhole_b0` — the SAME source and the
/// same declared dtypes, differing only in the target's accumulation. A miscompile does not get 4x better
/// when the accumulator widens; precision does. That is the evidence this is the datapath and not a bug.
///
/// So the assertion is the honest pair: the drift must stay inside a stated bound, the ARGMAX must agree
/// EXACTLY (the token is what a model actually emits), and — the self-policing half — a program that
/// drifts at all must not be shipping a `NumericsClaim::Exact` certificate. A bf16 forward calling itself
/// exact is the kind of claim that does not survive a question from the people who designed the chip.
#[test]
fn the_whole_forward_computes_what_the_vm_computes() {
    let cfg = Qwen38Cfg::tiny();
    let mut src = authored_with_fills(&cfg);
    // The whole logit vector, so the argmax is a real comparison and not a two-sample proxy.
    for i in 1..=cfg.vocab {
        src.push_str(&format!("Show round(item {i} of logits * 1000000).\n"));
    }

    let vm = vm_outcome(&src);
    assert!(
        vm.error.is_none(),
        "the authored forward runs on the VM: {:?}",
        vm.error
    );
    let want: Vec<i64> = vm
        .output
        .split_whitespace()
        .map(|t| t.parse().expect("an integer logit"))
        .collect();
    assert_eq!(
        want.len(),
        cfg.vocab,
        "the oracle prints the whole logit vector"
    );

    let target = RduChip::Sn50.facts().spatial_target();
    let p = compile_program_to_spatial(&src, &target)
        .expect("a tensor program")
        .program
        .expect("composes");
    let logits = p
        .tensors
        .iter()
        .find(|t| t.name == "logits")
        .expect("the forward produces `logits`");
    let out =
        eval_choreography(&p, &inputs_from_fills(&p)).expect("the composed choreography evaluates");
    let vals: Vec<f64> = match out
        .tensors
        .get(&logits.id)
        .expect("the choreography writes `logits`")
    {
        Value::Float(v) => v.clone(),
        Value::Int(v) => v.iter().map(|x| *x as f64).collect(),
    };
    assert_eq!(vals.len(), cfg.vocab, "the logit vector is vocab-wide");
    let mine: Vec<i64> = vals
        .iter()
        .map(|x| (x * 1_000_000.0).round() as i64)
        .collect();

    // ── the argmax must agree EXACTLY: the token is the output that matters.
    let am = |v: &[i64]| {
        v.iter()
            .enumerate()
            .max_by_key(|(_, x)| **x)
            .map(|(i, _)| i)
            .unwrap()
    };
    assert_eq!(
        am(&mine),
        am(&want),
        "the choreography picks a different token than the VM\n  ours: {mine:?}\n  vm:   {want:?}"
    );

    // ── the drift must stay inside a STATED bound, and the bound is measured, not chosen to fit.
    const BF16_WHOLE_FORWARD_REL: f64 = 0.02;
    let scale = want.iter().map(|x| x.abs()).max().unwrap_or(1).max(1) as f64;
    let worst = mine
        .iter()
        .zip(&want)
        .map(|(a, b)| ((a - b) as f64).abs() / scale)
        .fold(0.0, f64::max);
    assert!(
        worst <= BF16_WHOLE_FORWARD_REL,
        "the composed choreography drifts {worst:.4} from the VM, past the {BF16_WHOLE_FORWARD_REL} a bf16 whole-forward may — lowering is not correctness\n  ours: {mine:?}\n  vm:   {want:?}"
    );

    // ── the self-policing half: drift at all ⇒ the certificate must SAY it is approximate.
    let claim = &p.claim;
    if worst > 0.0 {
        match claim {
            NumericsClaim::Approx { reason } => assert!(!reason.trim().is_empty(), "an approximate claim names its reason"),
            NumericsClaim::Exact => panic!(
                "the program drifts {worst:.4} from the f64 oracle while its certificate claims NumericsClaim::Exact — a bf16 forward may be approximate, but it must not call itself exact"
            ),
        }
    }
    println!("whole Qwen3.8 (tiny) forward on {}: argmax agrees, worst drift {worst:.5}, claim {claim:?}", target.name);
}
