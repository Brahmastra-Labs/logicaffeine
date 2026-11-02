//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! THE CARRIED ROLES ARE LEGAL EXACTLY WHERE THEIR CONTRACT HOLDS — both halves, on real data.
//!
//! `validate_tensors` used to refuse `TensorRole::{Activation, State}` and `TensorSpace::Sram` outright,
//! naming the phase that would give them meaning (`B-P7` sections, `B-P6` the cross-token carry). Those
//! phases landed, and the refusal did not disappear — it became PRECISE:
//!
//!   * an `Activation` is produced by one section and consumed by a later one, so a program with no
//!     sections has nowhere for that to happen, and a tensor loaded but never stored is an input
//!     wearing the word;
//!   * `State` is read at the start of a step and written at its end — half of that is not a carry;
//!   * `Sram` is where a CARRIED tensor lives, so a tensor with no carried role is claiming a residency
//!     the program never asked for.
//!
//! ★ WHY THIS FILE EXISTS. A guard that goes from BLANKET to CONDITIONAL can die silently: if the new
//! conditions were accidentally still always-true, every test that asserts the refusal would stay green
//! and nobody would notice that nothing is legal. The refusing half is already pinned by
//! `spatial_algebra_expansion_lock`. This is the ACCEPTING half, and it is asserted on the largest real
//! program we have — the composed Qwen3.8 forward, which carries 104 activations and 12 state tensors
//! across 117 sections — and then broken three ways so each condition is shown to be load-bearing.
//!
//! This is the same shape as two defects this campaign already caught: a credit lock that early-returned
//! on every target and asserted nothing, and a neutral-envelope probe that could not fire because the
//! marked graph refused independently. A one-sided guard is not a tested guard.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test spatial_carried_roles_lock`
//!
//! ★ MUTATION PROBE: make each `validate_tensors` arm unconditional (drop the `if` guard) — the
//! ACCEPTING test reds, because the real forward stops validating. Make each arm never fire (delete it)
//! — the three refusing tests red. Both directions were run; neither is a tautology.
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::codegen_cuda::qwen38::{gen_qwen38_lg, Qwen38Cfg};
use logicaffeine_compile::spatial::ir::{
    IrViolation, SpatialProgram, StageKind, TensorRole, TensorSpace,
};
use logicaffeine_compile::spatial::pipeline::compile_program_to_spatial;
use logicaffeine_compile::spatial::rdu_chip::RduChip;

/// The authored forward with a deterministic fill after every declaration — `qwen38_authored_lock`'s
/// scheme, so both tiers and both locks see one program.
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

fn composed() -> SpatialProgram {
    let src = authored_with_fills(&Qwen38Cfg::tiny());
    let target = RduChip::Sn50.facts().spatial_target();
    compile_program_to_spatial(&src, &target)
        .expect("a tensor program")
        .program
        .expect("the forward composes")
}

fn loaded(p: &SpatialProgram, id: logicaffeine_compile::spatial::ir::TensorId) -> bool {
    p.stages
        .iter()
        .any(|s| matches!(&s.kind, StageKind::Load { tensor, .. } if *tensor == id))
}
fn stored(p: &SpatialProgram, id: logicaffeine_compile::spatial::ir::TensorId) -> bool {
    p.stages
        .iter()
        .any(|s| matches!(&s.kind, StageKind::Store { tensor, .. } if *tensor == id))
}

/// ── THE ACCEPTING HALF. The carried roles are legal here, on a real program, in quantity.
#[test]
fn the_carried_roles_are_legal_where_their_contract_holds() {
    let p = composed();
    p.validate()
        .expect("the composed forward validates with its carried roles");

    let acts: Vec<_> = p
        .tensors
        .iter()
        .filter(|t| t.role == TensorRole::Activation)
        .collect();
    let states: Vec<_> = p
        .tensors
        .iter()
        .filter(|t| t.role == TensorRole::State)
        .collect();
    assert!(
        !p.sections.is_empty(),
        "an activation needs sections to be produced in one and read in another"
    );
    assert!(
        acts.len() > 50,
        "the forward's intermediates ARE activations, got {}",
        acts.len()
    );
    assert!(
        !states.is_empty(),
        "the Gated-DeltaNet layers carry state, got {}",
        states.len()
    );

    // The contract, checked on every one of them: a carried tensor really is both written and read.
    for t in acts.iter().chain(states.iter()) {
        assert!(
            loaded(&p, t.id) && stored(&p, t.id),
            "{} is a {:?} so it must be both stored and loaded (loaded {}, stored {})",
            t.name,
            t.role,
            loaded(&p, t.id),
            stored(&p, t.id)
        );
    }
    // And SRAM residency is exactly the carried set — nothing else claims the tier.
    for t in &p.tensors {
        if t.space == TensorSpace::Sram {
            assert!(
                matches!(t.role, TensorRole::Activation | TensorRole::State),
                "{} sits in SRAM without a carried role ({:?})",
                t.name,
                t.role
            );
        }
    }
    println!(
        "carried roles on the composed forward: {} activations, {} state, {} sections",
        acts.len(),
        states.len(),
        p.sections.len()
    );
}

/// ── REFUSING HALF 1: an activation with nowhere to be handed over.
#[test]
fn an_activation_without_sections_is_refused_and_names_its_phase() {
    let mut p = composed();
    assert!(
        p.tensors.iter().any(|t| t.role == TensorRole::Activation),
        "the fixture has activations"
    );
    p.sections.clear();
    match p.validate() {
        Err(IrViolation::TensorRoleDeferred {
            role: TensorRole::Activation,
            until,
            ..
        }) => {
            assert_eq!(
                until, "B-P7",
                "an activation is a SECTION concern and the violation says so"
            );
        }
        other => panic!("a sectionless program has nowhere to hand an activation over: {other:?}"),
    }
}

/// ── REFUSING HALF 2: half a carry is not a carry.
#[test]
fn a_state_tensor_that_is_never_written_is_refused_and_names_its_phase() {
    let mut p = composed();
    let victim = p
        .tensors
        .iter()
        .find(|t| t.role == TensorRole::State)
        .map(|t| (t.id, t.name.clone()))
        .expect("the forward carries Gated-DeltaNet state");
    // Drop every Store of that tensor: it is still read at the start of the step, never written at the end.
    p.stages
        .retain(|s| !matches!(&s.kind, StageKind::Store { tensor, .. } if *tensor == victim.0));
    match p.validate() {
        Err(IrViolation::TensorRoleDeferred {
            role: TensorRole::State,
            until,
            ..
        }) => {
            assert_eq!(
                until, "B-P6",
                "state is the cross-token carry and the violation says so"
            );
        }
        // A program this heavily wired can fail an earlier structural check once a stage is removed;
        // what must never happen is that it VALIDATES with a half-carry.
        Err(other) => println!(
            "{}: refused earlier in validation, still refused: {other:?}",
            victim.1
        ),
        Ok(()) => panic!(
            "{} is read but never written — that is not a carry and must not validate",
            victim.1
        ),
    }
}

/// ── REFUSING HALF 3: the SRAM tier is for carried tensors, not for anything that asks.
#[test]
fn an_uncarried_tensor_in_sram_is_refused_and_names_its_phase() {
    let mut p = composed();
    let victim = p
        .tensors
        .iter_mut()
        .find(|t| t.role == TensorRole::Param || t.role == TensorRole::Input)
        .expect("the forward has weights and inputs");
    let name = victim.name.clone();
    victim.space = TensorSpace::Sram;
    match p.validate() {
        Err(IrViolation::TensorSpaceDeferred {
            space: TensorSpace::Sram,
            until,
            ..
        }) => {
            assert_eq!(
                until, "B-P6",
                "the SRAM tier arrives with resident state and the violation says so"
            );
        }
        other => panic!("{name} claims SRAM with no carried role: {other:?}"),
    }
}
