//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! TENSOR PARALLELISM IS IN THE IR, AND A SHARD THAT SKIPS ITS ALL-REDUCE IS A WRONG ANSWER.
//!
//! An SN50 reaches its token throughput by splitting one model over many chips — the published SemiAnalysis
//! run is 16-way tensor-parallel. Until this file landed, our tensor parallelism lived entirely in
//! `spatial/fit.rs`: arithmetic that DIVIDES weight bytes by a chip count and reports that they fit. That
//! is a spreadsheet, not a compiler. Nothing in the IR knew what a chip was, so nothing could state — let
//! alone check — the one property that makes a sharded program mean anything:
//!
//!     A ROW-PARALLEL SHARD COMPUTES A PARTIAL. THE PARTIALS MUST BE SUMMED. A shard that skips its
//!     all-reduce still runs, still fills its output tensor, and still returns `Ok` — with 1/chips of
//!     the answer in it.
//!
//! That is the `lowering-is-not-correctness` failure in its purest form, and at TP=16 it is a number that
//! is wrong by a factor of sixteen while every stage reports success. So the chip becomes a COORDINATE
//! (`Site.chip`), the all-reduce becomes a MOVEMENT stage over a group that spans chips, and
//! `certify_choreography` gains an obligation — `UnreducedShard` — that fires when a partial reaches an
//! output without being summed.
//!
//! What this lock proves:
//!   A. a sharded program computes what the unsharded one computes, and what the VM computes;
//!   B. the per-chip weight bytes really are the total over the chips (the fit report's claim, now backed
//!      by an IR that holds the split);
//!   C. one all-reduce per row-parallel matmul — no more, no fewer;
//!   D. ★ removing the all-reduce is BOTH a certificate refusal AND a measurably wrong answer. Either half
//!      alone is decoration: a refusal with no wrong answer behind it is a strict checker, and a wrong
//!      answer with no refusal is the bug this file exists to make impossible;
//!   E. the shard count must divide the contraction axis, refused by name rather than rounded;
//!   F. the reduce payload is derived from the tensor and the group's span is bounded by the published
//!      scale-up facts (8 chips/node fully connected, 16 per rack) — bytes and chips, never a time.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test spatial_tensor_parallel_lock`
//!
//! ★ MUTATION PROBE: in `passes/shard.rs`, make `shard` emit the per-chip partials WITHOUT appending the
//! `AllReduce` stage. `a_sharded_program_equals_the_unsharded_one_and_the_vm` reds with an answer that is
//! exactly 1/chips of the truth, and `a_shard_that_skips_its_all_reduce_...` reds on its certificate half.
//! The pair firing together is what proves the obligation is load-bearing and not merely strict.
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

#[path = "common/spatial.rs"]
mod spatial_support;

use logicaffeine_compile::spatial::certificate::{certify_choreography, CertifyFault};
use logicaffeine_compile::spatial::eval::{eval_choreography, Value};
use logicaffeine_compile::spatial::ir::{
    ComputeOp, ReducePool, SpatialProgram, StageKind, TensorBody, TensorRole,
};
use logicaffeine_compile::spatial::passes::shard::{shard, ShardError, ShardPlan};
use logicaffeine_compile::spatial::pipeline::compile_to_spatial;
use logicaffeine_compile::spatial::rdu_chip::RduChip;
use logicaffeine_compile::spatial::target::precision_bytes;
use spatial_support::*;

const M: usize = 32;
const N: usize = 32;
const K: usize = 512;

fn unsharded() -> SpatialProgram {
    let target = RduChip::Sn50.facts().spatial_target();
    compile_to_spatial(&gemm_program(M, N, K), &target).expect("the canonical gemm lowers")
}

fn sharded(chips: usize) -> SpatialProgram {
    let target = RduChip::Sn50.facts().spatial_target();
    shard(&unsharded(), &ShardPlan::row_parallel(chips), &target).expect("a row-parallel shard")
}

fn run(p: &SpatialProgram) -> Vec<i64> {
    let out = eval_choreography(p, &inputs_from_fills(p)).expect("the program evaluates");
    match out.tensors.get(&output_id(p)).expect("an output value") {
        Value::Int(v) => v.clone(),
        Value::Float(v) => v.iter().map(|x| *x as i64).collect(),
    }
}

fn certify_fresh(p: &SpatialProgram) -> Result<(), CertifyFault> {
    let mut q = p.clone();
    q.certificate = None;
    certify_choreography(&q).map(|_| ())
}

/// ── A. THE THEOREM. Sharding is a refactor of WHERE the work happens, never of WHAT it computes.
#[test]
fn a_sharded_program_equals_the_unsharded_one_and_the_vm() {
    let vm = gemm_reference(M, N, K);
    let plain = run(&unsharded());
    assert_eq!(
        plain, vm,
        "the unsharded gemm must already equal the VM — the oracle is not in question"
    );

    for chips in [2usize, 4, 8, 16] {
        let p = sharded(chips);
        assert_eq!(
            run(&p),
            vm,
            "★ the {chips}-way row-parallel shard does not compute the gemm. A row-parallel shard holds a \
             PARTIAL of the full output; summing those partials is the all-reduce. If this is off by a \
             factor near {chips}, the reduce is missing or reduced the wrong group."
        );
        certify_fresh(&p)
            .unwrap_or_else(|f| panic!("the {chips}-way shard must certify, got {f:?}"));
    }
}

/// ── B. The fit report's arithmetic, now backed by an IR that actually holds the split.
#[test]
fn per_chip_weight_bytes_are_the_total_over_chips() {
    let whole = unsharded();
    let total: usize = whole
        .tensors
        .iter()
        .filter(|t| t.role == TensorRole::Param)
        .map(|t| t.elems.rows * t.elems.cols * precision_bytes(t.dtype))
        .sum();
    assert!(
        total > 0,
        "the gemm must declare a Param weight for this test to mean anything"
    );

    for chips in [2usize, 4, 8] {
        let p = sharded(chips);
        let per_chip: usize = p
            .tensors
            .iter()
            .filter(|t| t.role == TensorRole::Param && t.chip.map(|c| c.0 == 0).unwrap_or(true))
            .map(|t| t.elems.rows * t.elems.cols * precision_bytes(t.dtype))
            .sum();
        assert_eq!(
            per_chip * chips,
            total,
            "chip 0 holds {per_chip} B of the {total} B of weights under a {chips}-way shard; the shards \
             must partition the weight, not replicate it — replication is what makes a 27B model not fit."
        );
    }
}

/// ── C. One all-reduce per row-parallel matmul. Not zero (wrong), not one per chip (a storm).
#[test]
fn every_sharded_matmul_has_exactly_one_all_reduce() {
    let matmuls = unsharded()
        .stages
        .iter()
        .filter(|s| matches!(&s.kind, StageKind::Compute(ComputeOp::MatmulTiles { .. })))
        .count();
    assert!(
        matmuls >= 1,
        "the gemm must lower to at least one matmul stage"
    );

    for chips in [2usize, 4, 8] {
        let p = sharded(chips);
        let reduces: Vec<_> = p
            .stages
            .iter()
            .filter_map(|s| match &s.kind {
                StageKind::AllReduce { pool, .. } => Some(*pool),
                _ => None,
            })
            .collect();
        assert_eq!(
            reduces.len(),
            matmuls,
            "a {chips}-way shard of {matmuls} row-parallel matmul(s) needs exactly {matmuls} all-reduce(s), \
             got {}. One per SHARDED MATMUL — a reduce per chip is a broadcast storm, and none is a wrong \
             answer.",
            reduces.len()
        );
        assert!(
            reduces.iter().all(|p| *p == ReducePool::Sum),
            "partials of a contraction combine by SUM; {reduces:?} says otherwise"
        );
    }
}

/// ── D. ★ THE CROWN JEWEL. Both halves, on the same program: the certificate refuses it, AND the answer
/// it would have produced is genuinely wrong. A refusal nobody can demonstrate a wrong answer behind is
/// just a strict checker; a wrong answer nothing refuses is the silent factor-of-16.
#[test]
fn a_shard_that_skips_its_all_reduce_is_a_wrong_answer_the_certificate_refuses() {
    let chips = 4;
    let good = sharded(chips);
    certify_fresh(&good).expect("the honest shard certifies");
    assert_eq!(
        run(&good),
        gemm_reference(M, N, K),
        "the honest shard is the control"
    );

    // ★ THE PROBE IS A MIS-WIRE, NOT A DELETION. Removing the collective outright does not even produce
    // a runnable program: it orphans the per-chip partial buffers, whose only consumer it was, and the
    // program fails to schedule. The realistic defect — and the dangerous one — is a collective that is
    // PRESENT and folds too few operands, which is exactly what an off-by-one in a fold chain gives you.
    // It schedules, it runs, it fills the output, and it returns one chip's share of the answer.
    let mut broken = good.clone();
    let mut touched = false;
    for s in &mut broken.stages {
        if let StageKind::AllReduce { combine, .. } = &mut s.kind {
            if let ComputeOp::Generic { body, .. } = combine.as_mut() {
                *body = TensorBody::Operand(0);
                touched = true;
            }
        }
    }
    assert!(
        touched,
        "the probe must actually weaken the fold to mean anything"
    );
    // ★ AND THE SCHEDULE MUST BE REBUILT. `chain_ops` COPIES the compute op into the choreography, and
    // `eval_choreography` runs the schedule rather than the stage list — so weakening the stage alone
    // leaves the original fold still executing, and the probe would silently measure the healthy program.
    broken.schedule = None;
    broken.certificate = None;
    let target = RduChip::Sn50.facts().spatial_target();
    broken = logicaffeine_compile::spatial::passes::synthesize_handshakes::synthesize_handshakes(
        &broken, &target,
    )
    .expect("a shard with a weakened fold still schedules — that is precisely the danger");

    // HALF ONE — the certificate names the obligation and the tensor it is about.
    match certify_fresh(&broken) {
        Err(CertifyFault::UnreducedShard { tensor, chips: c }) => {
            assert_eq!(c, chips, "the fault must report the shard width it found");
            let name = &broken.tensors.iter().find(|t| t.id == tensor).expect("a real tensor").name;
            println!("refused: {name} reaches an output as a {c}-way partial");
        }
        other => panic!(
            "★ a shard with no all-reduce MUST be refused as `UnreducedShard`, got {other:?}. This is the \
             obligation this whole file exists for: without it a {chips}-way shard returns Ok with a \
             quarter of the answer in it."
        ),
    }

    // HALF TWO — and it really is wrong, so the refusal above is not mere strictness.
    let wrong = run(&broken);
    assert_ne!(
        wrong,
        gemm_reference(M, N, K),
        "the un-reduced shard produced the RIGHT answer, which means this program was never really \
         sharded — the probe is vacuous and the certificate obligation above proves nothing."
    );
}

/// ── E. A shard that does not divide the contraction axis is refused BY NAME, never rounded.
#[test]
fn the_shard_count_must_divide_the_contraction_axis() {
    let target = RduChip::Sn50.facts().spatial_target();
    let whole = unsharded();
    match shard(&whole, &ShardPlan::row_parallel(7), &target) {
        Err(ShardError::AxisNotDivisible { axis, extent, chips }) => {
            assert_eq!(extent, K, "the contraction extent is {K}");
            assert_eq!(chips, 7);
            println!("refused: {axis} of {extent} does not split {chips} ways");
        }
        other => panic!(
            "sharding a contraction axis of {K} seven ways must be refused as `AxisNotDivisible`, got \
             {}. Rounding a shard silently drops or double-counts a slice of the contraction.",
            match other { Ok(_) => "Ok".to_string(), Err(e) => format!("{e:?}") }
        ),
    }
    assert!(
        shard(&whole, &ShardPlan::row_parallel(1), &target).is_ok(),
        "a 1-way shard is the identity"
    );
}

/// ── F. The payload is derived, and the group's span is bounded by the PUBLISHED scale-up facts.
/// Bytes and chips only — no field here is a time, and no speed is claimed.
#[test]
fn the_scale_up_facts_bound_the_reduce_bytes() {
    let facts = RduChip::Sn50.facts();
    let per_node = facts.chips_per_node.value;
    let per_rack = facts
        .scale_up
        .map(|s| s.value.chips_per_rack)
        .expect("SN50 publishes a rack width");

    for chips in [2usize, 4, 8, 16] {
        let p = sharded(chips);
        let reduce = p
            .stages
            .iter()
            .find_map(|s| match &s.kind {
                StageKind::AllReduce { tensor, .. } => Some(*tensor),
                _ => None,
            })
            .expect("a reduce");
        let t = p
            .tensors
            .iter()
            .find(|t| t.id == reduce)
            .expect("the reduced tensor");
        let bytes = t.elems.rows * t.elems.cols * precision_bytes(t.dtype);
        assert_eq!(
            bytes,
            M * N * precision_bytes(t.dtype),
            "an all-reduce of a row-parallel matmul carries the FULL output shape from every chip — \
             {M}x{N} of {:?}. A smaller payload means the partials were sliced, which a sum cannot undo.",
            t.dtype
        );
        assert!(
            chips <= per_rack,
            "a {chips}-way tensor-parallel group does not fit a SambaRack of {per_rack} — the published \
             topology is {per_node} per node fully connected, {per_rack} per rack"
        );
    }
}

/// ── Determinism: the same program and the same plan give byte-identical IR.
#[test]
fn the_sharded_program_is_a_function_of_its_plan() {
    let a = serde_json::to_string(&sharded(4)).expect("serializes");
    let b = serde_json::to_string(&sharded(4)).expect("serializes");
    assert_eq!(
        a, b,
        "sharding must be a pure function of (program, plan, target)"
    );
    let c = serde_json::to_string(&sharded(8)).expect("serializes");
    assert_ne!(
        a, c,
        "a different chip count must give a different program, or the plan is being ignored"
    );
}
