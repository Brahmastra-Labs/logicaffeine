//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! REWRITE LOCK — a rewrite of the spatial IR is answerable to what it COMPUTES, not to what it
//! looks like.
//!
//! Every interesting transformation of a [`SpatialProgram`] changes its structure on purpose: reader
//! fusion replaces Loads with multicast Recvs and a semaphore pair; legalization expands one fused
//! `ComputeOp` into a chain of engine-sized ones; a replan moves the block boundaries. So a
//! structural check can never be the thing that keeps a rewrite honest — the only durable question
//! is whether the two programs put the same numbers in the same tensors, on inputs neither of them
//! was written for.
//!
//! `spatial::equiv::equivalent_under` is that question, and this file is its lock:
//!
//!   A. a GEMM rearranged (readers fused vs not, the same block plan either way) computes the same
//!      output on a seeded corpus — a real rewrite, validated by evaluation alone;
//!   B. every program this file builds is scheduler-independent AND actually reads its corpus: two
//!      corpus points must move its output, or every `Ok(())` below is a program ignoring the data;
//!   C. the detector is NOT vacuous: two programs that genuinely compute different things are
//!      reported as a typed `Divergence` naming the input, the tensor and the element — and it is
//!      not always-`Err` either, since two DIFFERENTLY-BUILT programs of one meaning still pass;
//!   D. the four fail-closed refusals, one test each: an empty corpus, a side that cannot be
//!      evaluated at all, a side that stopped producing a tensor, and an output of a different
//!      shape. None of them may ever come back as "equivalent";
//!   E. the corpus keeps its OWN promise — every element integral, inside the tensor's declared
//!      range and inside the bf16-exact cap — because that promise is the whole reason a divergence
//!      found here belongs to the rewrite rather than to the datapath;
//!   F. `rmsnorm_has_only_its_legalized_form_today` — the partition that stands between this file
//!      and the differential it is FOR. Read it before adding anything to the fused-row family.
//!
//! ★ WHAT THIS FILE IS NOT, SAID PLAINLY. Both sides of every differential here run under the SAME
//! evaluator, so this lock validates REWRITES against the IR's semantics — never the semantics
//! against the mathematics. An operator the compiler has always computed wrongly passes every test
//! below and should: that error is `spatial_algebra_semantics_lock`'s to catch, against references
//! written out longhand from each operator's definition. Neither oracle substitutes for the other,
//! and weakening one because the other is green is how both stop meaning anything.
//!
//! Run: `cargo nextest run -p logicaffeine-tests -E 'binary(spatial_rewrite_lock)'`
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

#[path = "common/spatial.rs"]
mod spatial_support;

use logicaffeine_compile::spatial::equiv::{
    equivalent_under, equivalent_within, seeded_corpus, seeded_inputs, Divergence, Elem, Side,
    APPROX_REL_TOL, CORPUS_MAGNITUDE_CAP,
};
use logicaffeine_compile::spatial::eval::{
    eval_choreography_confluent, ChoreoFault, Inputs, Value,
};
use logicaffeine_compile::spatial::ir::{
    AccPolicy, BlockPlan, NumericsPolicy, SpatialProgram, StageKind, TensorId, TensorRole,
};
use logicaffeine_compile::spatial::passes::from_tensor_op::{
    from_elementwise, from_fused_row, ElementwiseNames, FusedRowFills, FusedRowNames, MatmulNames,
};
use logicaffeine_compile::spatial::passes::fuse_readers::fuse_readers_forced;
use logicaffeine_compile::spatial::passes::persist::persist;
use logicaffeine_compile::spatial::passes::place::place;
use logicaffeine_compile::spatial::passes::size_buffers::size_buffers;
use logicaffeine_compile::spatial::passes::synthesize_handshakes::synthesize_handshakes;
use logicaffeine_compile::spatial::passes::tile::apply_plan;
use logicaffeine_compile::spatial::passes::verify::verify;
use logicaffeine_compile::spatial::pipeline::{run_passes, template_from_spec};
use logicaffeine_compile::spatial::target::SpatialTarget;
use logicaffeine_compile::tensor_recognize::{
    ElementwiseOp, ElementwiseSpec, Epsilon, Fill, FusedRowSpec, MatmulSpec, RowReduceKind,
};
use spatial_support::*;

// ── The programs under test ────────────────────────────────────────────────────────────────────────

fn gemm_names() -> MatmulNames {
    MatmulNames {
        a: "a".into(),
        b: "b".into(),
        c: "c".into(),
    }
}

/// One ARRANGEMENT of a GEMM: the same shape, the same block plan, the same target — differing only in
/// whether the readers were fused into multicast groups.
///
/// ★ THE PLAN IS STATED, NOT CHOSEN. Letting the cost model pick would make this lock a test of the cost
/// model: two arrangements it happened to price the same way would silently become one program and the
/// differential would prove nothing. `fuse_readers_forced` is the calibration facility that exists exactly
/// so an arrangement can be demanded rather than argued for.
fn gemm_arrangement(m: usize, n: usize, k: usize, plan: BlockPlan, fused: bool) -> SpatialProgram {
    let t = fusing_wormhole();
    let spec = MatmulSpec {
        m,
        n,
        k,
        a: Default::default(),
        b: Default::default(),
        c: Default::default(),
    };
    let label = if fused { "fused" } else { "unfused" };
    let template = template_from_spec(
        &spec,
        &gemm_names(),
        (&Fill::Unknown, &Fill::Unknown),
        (Some((-128, 127)), Some((-128, 127))),
        NumericsPolicy::Exact,
        &t,
    )
    .unwrap_or_else(|e| panic!("{m}x{n}x{k} {label}: template: {e:?}"));
    let placed = place(&apply_plan(&template, plan, &t), &t)
        .unwrap_or_else(|e| panic!("{m}x{n}x{k} {label}: place: {e:?}"));
    let arranged = fuse_readers_forced(&placed, &t, fused)
        .unwrap_or_else(|e| panic!("{m}x{n}x{k} {label}: fuse_readers: {e:?}"));
    let sized = size_buffers(&arranged, &t)
        .unwrap_or_else(|e| panic!("{m}x{n}x{k} {label}: size_buffers: {e:?}"));
    let scheduled = synthesize_handshakes(&sized, &t)
        .unwrap_or_else(|e| panic!("{m}x{n}x{k} {label}: synthesize_handshakes: {e:?}"));
    let persisted =
        persist(&scheduled, &t).unwrap_or_else(|e| panic!("{m}x{n}x{k} {label}: persist: {e:?}"));
    verify(&persisted, &t).unwrap_or_else(|e| panic!("{m}x{n}x{k} {label}: verify: {e:?}"))
}

/// The shapes the rearrangement differential runs on, each with a block plan that tiles it exactly.
///
/// `128×128×256` on a 4×4 grid blocks its K into TWO inner blocks, so the accumulator spills a partial
/// through the intermediate buffer between them — the path where a rearrangement is most able to lose an
/// accumulation, and the plan `spatial_choreography_semantics_lock`'s `small_fused` already exercises.
/// `128×64×128` keeps ONE K block on a 4×2 grid, so the two mesh axes carry different group counts and the
/// spill path is absent — the two shapes disagree about everything the arrangement can vary.
///
/// Both stay inside `equiv`'s exactness window (`K · 128² ≤ 2²² < 2²⁴`), so a divergence found here is the
/// REWRITE, never the datapath's rounding.
fn gemm_shapes() -> Vec<(usize, usize, usize, BlockPlan)> {
    vec![
        (
            128,
            128,
            256,
            blocked_plan(4, 4, 8, (1, 1), (1, 1, 4), AccPolicy::F32),
        ),
        (
            128,
            64,
            128,
            blocked_plan(4, 2, 4, (1, 1), (1, 1, 4), AccPolicy::F32),
        ),
    ]
}

/// A flat elementwise program with its operand fills stated — the fills decide the corpus WINDOW and the
/// recorded data, and separating them from the op is what lets a differential ask whether the verdict came
/// from the corpus or from the fills.
fn elementwise_program_with(op: ElementwiseOp, n: usize, fills: (&Fill, &Fill)) -> SpatialProgram {
    let t = SpatialTarget::wormhole();
    let spec = ElementwiseSpec {
        a: Default::default(),
        b: Default::default(),
        c: Default::default(),
        op,
        n,
        row: None,
    };
    let names = ElementwiseNames {
        a: "a".into(),
        b: "b".into(),
        c: "c".into(),
    };
    let template = from_elementwise(&spec, &names, fills, &t)
        .unwrap_or_else(|e| panic!("elementwise {op:?} n={n} template: {e:?}"));
    run_passes(template, &t).unwrap_or_else(|e| panic!("elementwise {op:?} n={n} passes: {e:?}"))
}

/// A flat elementwise program, built straight from the spec the recognizer would have produced.
fn elementwise_program(op: ElementwiseOp, n: usize) -> SpatialProgram {
    elementwise_program_with(
        op,
        n,
        (
            &Fill::Pattern {
                modulus: 7,
                shift: -3,
            },
            &Fill::Pattern {
                modulus: 11,
                shift: -5,
            },
        ),
    )
}

fn fused_row_spec(kind: RowReduceKind, rows: usize, cols: usize) -> FusedRowSpec {
    FusedRowSpec {
        rows,
        cols,
        reduce: kind,
        x: Default::default(),
        y: Default::default(),
        weight: None,
        weight_rows: 1,
        residual: None,
        bias: None,
        eps: Epsilon::DEFAULT,
    }
}

fn fused_row_names() -> FusedRowNames {
    FusedRowNames {
        x: "x".into(),
        y: "y".into(),
        weight: None,
        residual: None,
        bias: None,
    }
}

/// `Fill::Unknown` on `x` on purpose: the activations are EXTERNAL, so the corpus is the only thing that
/// can supply them and a corpus that fails to is a typed fault rather than a silent fallback to a fill.
fn fused_row_fills() -> FusedRowFills {
    FusedRowFills {
        x: Fill::Unknown,
        weight: None,
        residual: None,
        bias: None,
    }
}

/// The verified rmsnorm program — which, today, is only ever the expanded chain (see
/// [`rmsnorm_has_only_its_legalized_form_today`]).
fn rmsnorm_chain(rows: usize, cols: usize) -> SpatialProgram {
    let t = SpatialTarget::wormhole();
    let template = from_fused_row(
        &fused_row_spec(RowReduceKind::RmsNorm, rows, cols),
        &fused_row_names(),
        &fused_row_fills(),
        &t,
    )
    .unwrap_or_else(|e| panic!("rmsnorm {rows}x{cols} template: {e:?}"));
    run_passes(template, &t).unwrap_or_else(|e| panic!("rmsnorm {rows}x{cols} passes: {e:?}"))
}

/// The compute ops a program carries, in stage order, named by variant — the readable shape of a rewrite.
fn compute_ops(p: &SpatialProgram) -> Vec<String> {
    p.stages
        .iter()
        .filter_map(|s| match &s.kind {
            StageKind::Compute(op) => Some(
                format!("{op:?}")
                    .split_whitespace()
                    .next()
                    .unwrap_or("?")
                    .to_string(),
            ),
            _ => None,
        })
        .collect()
}

fn names_of(ops: &[String]) -> Vec<&str> {
    ops.iter().map(|s| s.as_str()).collect()
}

/// A tensor's corpus elements in one tower, so a window assertion does not have to be written twice.
fn elements(v: &Value) -> Vec<f64> {
    match v {
        Value::Int(x) => x.iter().map(|i| *i as f64).collect(),
        Value::Float(x) => x.clone(),
    }
}

/// One corpus point covering BOTH programs: `base`'s entries win, `extra` fills in the ids `base` has no
/// tensor for. Two programs over different workloads only share a corpus if someone builds the union, and
/// building it here — rather than passing one program's inputs and hoping — is what keeps a
/// `MissingInput` fault from masquerading as the divergence a test is actually looking for.
fn merged(mut base: Inputs, extra: Inputs) -> Inputs {
    for (id, v) in extra.tensors {
        if !base.tensors.contains_key(&id) {
            base.tensors.insert(id, v);
        }
    }
    base
}

/// Every program this lock builds, labelled — the population property B quantifies over.
fn programs_under_test() -> Vec<(String, SpatialProgram)> {
    let mut all: Vec<(String, SpatialProgram)> = Vec::new();
    for (m, n, k, plan) in gemm_shapes() {
        all.push((
            format!("gemm {m}x{n}x{k} fused"),
            gemm_arrangement(m, n, k, plan, true),
        ));
        all.push((
            format!("gemm {m}x{n}x{k} unfused"),
            gemm_arrangement(m, n, k, plan, false),
        ));
    }
    all.push((
        "elementwise add".into(),
        elementwise_program(ElementwiseOp::Add, 32),
    ));
    all.push((
        "elementwise mul".into(),
        elementwise_program(ElementwiseOp::Mul, 32),
    ));
    all.push(("rmsnorm chain 32x32".into(), rmsnorm_chain(32, 32)));
    all
}

// ── A. The rewrite, validated by evaluation alone ──────────────────────────────────────────────────

#[test]
fn a_rearranged_gemm_computes_the_same_tensors() {
    for (m, n, k, plan) in gemm_shapes() {
        let fused = gemm_arrangement(m, n, k, plan, true);
        let unfused = gemm_arrangement(m, n, k, plan, false);

        // ★ THE DIFFERENTIAL IS ONLY WORTH RUNNING IF THE TWO PROGRAMS REALLY DIFFER. `fuse_readers`
        // replaces a receiver's Load with a Recv and introduces a semaphore pair per group, so the fused
        // arrangement's structure is not the unfused one's — assert that here rather than trusting it,
        // because a differential between a program and itself is the most convincing vacuous test there is.
        let (fs, us) = (fused.summary(), unfused.summary());
        assert!(
            fs.multicast_groups > 0,
            "{m}x{n}x{k}: forced fusion produced no multicast group, so this is not a rearrangement at all \
             ({fs:?}). Fix `fuse_readers_forced` in spatial/passes/fuse_readers.rs — never this assertion, \
             which is what stops the differential going vacuous."
        );
        assert_eq!(
            us.multicast_groups, 0,
            "{m}x{n}x{k}: the unfused arrangement must carry no multicast group ({us:?}); \
             `fuse_readers_forced(_, _, false)` is defined to return the placement untouched."
        );
        let recvs = |p: &SpatialProgram| {
            p.stages
                .iter()
                .filter(|s| matches!(s.kind, StageKind::Recv { .. }))
                .count()
        };
        assert!(
            recvs(&fused) > 0,
            "{m}x{n}x{k}: fusion is defined as replacing a receiver's Load with a Recv; with no Recv the \
             stage graph never changed and the differential below compares a program with itself."
        );
        assert_eq!(
            recvs(&unfused),
            0,
            "{m}x{n}x{k}: the unfused arrangement reads every operand itself"
        );

        // Same tensors, same ids: one corpus feeds both, so each side genuinely sees the same numbers.
        let corpus = seeded_corpus(&fused, &[0x5EED, 0xA11CE, 0xB0B]);
        equivalent_under(&fused, &unfused, &corpus).unwrap_or_else(|d| {
            panic!(
                "\n★ {m}x{n}x{k}: fusing the readers CHANGED WHAT THE PROGRAM COMPUTES.\n  {d}\n\n\
                 Reader fusion is a movement rewrite: the same tiles reach the same compute stages by a \
                 different route. A divergence here is a real defect in `fuse_readers` \
                 (spatial/passes/fuse_readers.rs) or in the multicast landing rules of `spatial/eval.rs` — \
                 NEVER a reason to relax this assertion or to shrink the corpus.\n"
            )
        });
    }
}

// ── B. Scheduler independence, and a corpus that actually reaches the program ───────────────────────

/// ★ THE SECOND HALF IS THE ONE THAT KEEPS THE FILE HONEST. "Five schedulers agree" is satisfied
/// perfectly by a program that computes nothing, and "two programs agree" is satisfied perfectly by two
/// programs that both ignore their inputs — so if the corpus never reaches the arithmetic, every `Ok(())`
/// in this file is an artefact. Two corpus points that produce the SAME output would say exactly that, so
/// they are asserted to differ. (Note what this does not need: no reference, no second oracle. Two
/// distinct inputs producing one output is already a broken pipe.)
///
/// `eval_choreography_confluent` is the scheduler oracle and it RETURNS the round-robin outputs, so
/// re-running round-robin and comparing would be comparing a value with itself — the tautology this test
/// deliberately does not contain.
#[test]
fn every_program_this_lock_builds_is_scheduler_independent_and_reads_its_corpus() {
    for (label, p) in programs_under_test() {
        let corpus = seeded_corpus(&p, &[0xC0FFEE, 7]);
        assert_eq!(
            corpus.len(),
            2,
            "{label}: this test needs two corpus points to compare"
        );
        let mut per_point = Vec::new();
        for (i, input) in corpus.iter().enumerate() {
            let out = eval_choreography_confluent(&p, input).unwrap_or_else(|f| {
                panic!(
                    "\n★ {label} input {i}: the five schedulers do not agree (or one faulted): {f:?}\n\n\
                     Kahn determinacy is a property of the IR itself, so a rewrite that preserves the \
                     numbers under round-robin and loses this has still broken the program. Fix the pass \
                     that produced the schedule, never this assertion.\n"
                )
            });
            assert!(!out.tensors.is_empty(), "{label} input {i}: a program that produced no output tensor cannot be the subject of a differential");
            per_point.push(out.tensors);
        }
        assert_ne!(
            per_point[0], per_point[1],
            "\n★ {label}: two DIFFERENT corpus points produced the SAME output, so this program is not \
             reading its corpus — and a differential over inputs a program ignores proves nothing at all.\n\n\
             The likely causes, in order: `seeded_inputs` stopped supplying a tensor the evaluator needs \
             (spatial/equiv.rs), or `Eval::build` stopped preferring supplied inputs over the recorded \
             fill (spatial/eval.rs). Fix whichever it is — this assertion is the tripwire for BOTH, and \
             weakening it silently re-vacuums every other test in this file.\n"
        );
    }
}

// ── C. The detector is not vacuous ─────────────────────────────────────────────────────────────────

#[test]
fn equivalent_under_names_the_tensor_that_diverges() {
    const N: usize = 32;
    let add = elementwise_program(ElementwiseOp::Add, N);
    let mul = elementwise_program(ElementwiseOp::Mul, N);

    // A point at which the two CANNOT agree anywhere: 1 + 1 = 2, 1 · 1 = 1, at every element.
    let mut ones = Inputs::default();
    ones.tensors
        .insert(TensorId::new(0), Value::Float(vec![1.0; N]));
    ones.tensors
        .insert(TensorId::new(1), Value::Float(vec![1.0; N]));
    match equivalent_under(&add, &mul, std::slice::from_ref(&ones)) {
        Err(Divergence::Element { input, tensor, elem, a, b, differing, total }) => {
            assert_eq!(input, 0, "the divergence must name the corpus index it was found at");
            assert_eq!(tensor, output_id(&add), "the divergence must name the OUTPUT tensor, not an operand");
            assert_eq!(elem, 0, "the first differing element of an everywhere-differing tensor is element 0");
            assert_eq!((differing, total), (N, N), "a + b and a · b differ at every element of the output, not just one");
            let (av, bv) = match (a, b) {
                (Elem::Float(x), Elem::Float(y)) => (x, y),
                (Elem::Int(x), Elem::Int(y)) => (x as f64, y as f64),
                other => panic!("the two sides must report the same representation, got {other:?}"),
            };
            assert_eq!((av, bv), (2.0, 1.0), "the reported values must be the ones the two programs actually produced");
        }
        other => panic!(
            "\n★ `equivalent_under` did not detect that `a + b` and `a · b` are different programs: {other:?}\n\n\
             This is the non-vacuity assertion for the whole file: if it does not fire, none of the Ok(()) \
             results above mean anything. Fix `spatial/equiv.rs` — never this test.\n"
        ),
    }

    // The seeded corpus finds it too, without being told where to look.
    let corpus = seeded_corpus(&add, &[1, 2, 3]);
    assert!(
        equivalent_under(&add, &mul, &corpus).is_err(),
        "a seeded corpus must separate `a + b` from `a · b`; if it cannot, the corpus is not exploring the data"
    );

    // ★ AND THE DETECTOR IS NOT SIMPLY ALWAYS-Err. The twin is `a + b` again but built from DIFFERENT
    // recorded fills, so it is a genuinely different program text that happens to mean the same thing —
    // which is the shape every real rewrite has. It also pins WHERE the verdict comes from: the corpus
    // supplies both operands, so a comparison that leaked the recorded fills into the answer would report
    // these two as divergent and this line would fail.
    let twin = elementwise_program_with(
        ElementwiseOp::Add,
        N,
        (
            &Fill::Pattern {
                modulus: 3,
                shift: 1,
            },
            &Fill::Const(2),
        ),
    );
    equivalent_under(&add, &twin, &corpus).unwrap_or_else(|d| {
        panic!(
            "\n★ two builds of `a + b` that differ only in their RECORDED FILLS were reported as \
             inequivalent: {d}\n\n\
             The corpus supplies every operand, so the recorded fill must not reach the answer at all. \
             Either `Eval::build` stopped preferring supplied inputs (spatial/eval.rs) or `equivalent_under` \
             started consulting something other than the outputs (spatial/equiv.rs).\n"
        )
    });
}

// ── D. The fail-closed refusals ────────────────────────────────────────────────────────────────────

#[test]
fn an_empty_corpus_is_never_an_equivalence_proof() {
    let add = elementwise_program(ElementwiseOp::Add, 32);
    let mul = elementwise_program(ElementwiseOp::Mul, 32);
    assert_eq!(
        equivalent_under(&add, &mul, &[]),
        Err(Divergence::EmptyCorpus),
        "checking nothing must never answer `equivalent` — a vacuous pass reads exactly like a proof. \
         The guard lives in `equivalent_under` (spatial/equiv.rs); never remove it to make a caller with \
         no corpus go green, and never weaken this assertion to accept `Ok(())`"
    );
    assert_eq!(
        equivalent_under(&add, &add, &[]),
        Err(Divergence::EmptyCorpus),
        "not even for a program against itself: the guard is about the corpus, not about the programs"
    );
}

/// ★ TWO SIDES THAT BOTH FAIL ARE NOT TWO SIDES THAT AGREE. The rmsnorm chain's `x` is external data, so
/// an empty `Inputs` cannot evaluate it — on EITHER side, identically. The tempting bug is to notice both
/// sides did the same thing and call that equivalence; a validator that does is one that reports "these
/// two programs agree" about a pair it never managed to run.
#[test]
fn a_program_that_cannot_be_evaluated_is_never_reported_equivalent() {
    let chain = rmsnorm_chain(1, 32);
    let nothing = vec![Inputs::default()];
    match equivalent_under(&chain, &chain, &nothing) {
        Err(Divergence::Faulted { side, input, fault }) => {
            assert_eq!(side, Side::A, "the first side evaluated is the one whose fault is reported");
            assert_eq!(input, 0, "the fault must name the corpus index it was found at");
            assert_eq!(
                fault,
                ChoreoFault::MissingInput(TensorId::new(0)),
                "the fault must be the evaluator's own typed one, carried through verbatim — an \
                 `equivalent_under` that flattened it to `not equivalent` would hide that the corpus, not \
                 the rewrite, is what is wrong"
            );
        }
        other => panic!(
            "\n★ a pair that could not be evaluated at all came back as {other:?}.\n\n\
             `Ok(())` here would be the worst possible answer: an equivalence claim discharged over a run \
             that never happened. Fix `equivalent_under` (spatial/equiv.rs) — never this test.\n"
        ),
    }
}

/// ★ THE UNION OF THE OUTPUT KEYS, LOCKED. `compare_outputs` walks `a`'s tensors AND `b`'s; a version that
/// walked only `a`'s would call a program that quietly stopped producing tensor 1 equivalent to one that
/// still does. The rmsnorm chain writes its output at id 1 and the elementwise program writes its at id 2,
/// so the two output key sets are disjoint — exactly the shape that failure mode has.
#[test]
fn a_side_that_stops_producing_a_tensor_is_named() {
    let chain = rmsnorm_chain(1, 32);
    let ew = elementwise_program(ElementwiseOp::Add, 32);
    // One corpus point that satisfies BOTH: `x`/`a` at id 0 (both 1×32), the elementwise `b` at id 1, and
    // the chain's planted `rms_scaler` at id 2. Each side ignores the ids its own program marks Output.
    let corpus = vec![merged(
        seeded_inputs(&chain, 0x0DDBA11),
        seeded_inputs(&ew, 0x0DDBA11),
    )];
    match equivalent_under(&chain, &ew, &corpus) {
        Err(Divergence::MissingTensor { side, input, tensor }) => {
            assert_eq!(side, Side::B, "`b` is the side that never produced it");
            assert_eq!(input, 0, "the divergence must name the corpus index it was found at");
            assert_eq!(
                tensor,
                output_id(&chain),
                "the divergence must name the tensor that went missing, which is the one only `a` wrote"
            );
        }
        other => panic!(
            "\n★ a side that produced NO output tensor 1 came back as {other:?}.\n\n\
             `Ok(())` here means `compare_outputs` (spatial/equiv.rs) stopped taking the UNION of the two \
             key sets — the single failure mode a rewrite validator must never have, since a pass that \
             deletes an output would then be certified correct. Fix it there, never here.\n"
        ),
    }
}

/// ★ A DIFFERENT-SHAPED OUTPUT IS A DIVERGENCE, NOT A ZIP. Two rmsnorm chains over two and one row write
/// the same tensor id at two different lengths; comparing them element-wise up to the shorter one would
/// certify a rewrite that truncated a tensor.
///
/// Both rows fit ONE 32×32 tile and both declare 32 columns, so the two programs agree on tensor ids, on
/// tiling, and on the planted `1/cols` scaler — the ONLY thing that differs is how many elements come out,
/// which is exactly the variable under test.
#[test]
fn an_output_of_a_different_shape_is_named_not_zipped() {
    let wide = rmsnorm_chain(2, 32);
    let narrow = rmsnorm_chain(1, 32);
    // Built from the WIDER program, so both sides find every element they read: the narrow one indexes
    // only its own 32, and a corpus taken from the narrow side would panic the wide one out of range.
    let corpus = seeded_corpus(&wide, &[0xFEED]);
    match equivalent_under(&wide, &narrow, &corpus) {
        Err(Divergence::Representation { input, tensor, a, b, a_len, b_len }) => {
            assert_eq!(input, 0, "the divergence must name the corpus index it was found at");
            assert_eq!(tensor, output_id(&wide), "the divergence must name the output tensor whose shape moved");
            assert_eq!((a, b), ("Float", "Float"), "both sides live in the float tower here; only the LENGTH differs");
            assert_eq!((a_len, b_len), (64, 32), "the report must carry both lengths — 'they differ' does not say which way");
        }
        other => panic!(
            "\n★ outputs of 64 and 32 elements came back as {other:?}.\n\n\
             `Ok(())` here means the comparison zipped to the shorter side, which certifies any rewrite \
             that truncates a tensor. The length check lives in `compare_value` (spatial/equiv.rs).\n"
        ),
    }
}

// ── E. The corpus keeps its own promise ────────────────────────────────────────────────────────────

/// ★ THIS IS THE ASSUMPTION EVERY OTHER TEST IN THE FILE RESTS ON. `equiv`'s claim is that a divergence it
/// finds belongs to the REWRITE and not to the datapath, and that claim is bought entirely by the corpus
/// staying integral, inside each tensor's declared range, and inside `CORPUS_MAGNITUDE_CAP` (which is
/// where bf16 stops holding integers exactly). A corpus element outside any of those turns "the rewrite
/// diverged" into "one side rounded" with nothing in the report to tell them apart — so the promise is
/// asserted, not assumed.
#[test]
fn every_corpus_element_is_integral_and_inside_the_window_it_promised() {
    let wh = SpatialTarget::wormhole();
    let spec = MatmulSpec {
        m: 64,
        n: 64,
        k: 64,
        a: Default::default(),
        b: Default::default(),
        c: Default::default(),
    };
    let gemm = template_from_spec(
        &spec,
        &gemm_names(),
        (&Fill::Unknown, &Fill::Unknown),
        (Some((-128, 127)), Some((-128, 127))),
        NumericsPolicy::Exact,
        &wh,
    )
    .unwrap_or_else(|e| panic!("the 64³ external-data template must build on wormhole: {e:?}"));

    let mut streams_by_seed: Vec<Vec<f64>> = Vec::new();
    for seed in [0u64, 0x5EED, u64::MAX] {
        let inputs = seeded_inputs(&gemm, seed);
        for t in gemm.tensors.iter().filter(|t| t.role != TensorRole::Output) {
            let v = inputs
                .tensors
                .get(&t.id)
                .unwrap_or_else(|| panic!("seed {seed}: tensor {} (`{}`) got no corpus data, so the evaluator would fault on it", t.id.idx(), t.name));
            let got = elements(v);
            assert_eq!(
                got.len(),
                t.elems.rows * t.elems.cols,
                "seed {seed}: tensor `{}` must be supplied in full, element for element",
                t.name
            );
            let (lo, hi) = t.range.unwrap_or_else(|| panic!("this template declares a range on `{}`; the test is written against that declaration", t.name));
            for (i, x) in got.iter().enumerate() {
                assert_eq!(x.fract(), 0.0, "seed {seed}: `{}`[{i}] = {x} is not integral; a fractional corpus can round in the datapath and be blamed on the rewrite", t.name);
                assert!(
                    *x >= lo as f64 && *x <= hi as f64,
                    "seed {seed}: `{}`[{i}] = {x} left the DECLARED range ({lo}, {hi}) the program's numerics claim was derived from. \
                     Fix `corpus_range` in spatial/equiv.rs — never widen this assertion.",
                    t.name
                );
                assert!(
                    x.abs() <= CORPUS_MAGNITUDE_CAP as f64,
                    "seed {seed}: `{}`[{i}] = {x} exceeds CORPUS_MAGNITUDE_CAP ({}), past which bf16 no longer holds \
                     integers exactly and a divergence can no longer be attributed to the rewrite",
                    t.name,
                    CORPUS_MAGNITUDE_CAP
                );
            }
        }
        let a = elements(inputs.tensors.get(&TensorId::new(0)).expect("operand a"));
        let b = elements(inputs.tensors.get(&TensorId::new(1)).expect("operand b"));
        assert_ne!(
            a, b,
            "seed {seed}: both operands were handed the SAME stream. `seeded_inputs` mixes the tensor id into \
             the seed precisely so a matmul is never multiplied against a copy of itself, which is the one \
             input at which a swapped-operand bug is invisible."
        );
        streams_by_seed.push(a);
    }
    for i in 0..streams_by_seed.len() {
        for j in i + 1..streams_by_seed.len() {
            assert_ne!(streams_by_seed[i], streams_by_seed[j], "two different seeds produced the same corpus, so a multi-seed corpus is one point wearing three hats");
        }
    }

    // The other half of the contract, stated the other way round: a Param the COMPILER planted is
    // reproduced exactly and never varies, because it is program text (the reduction's scaler) rather than data.
    let chain = from_fused_row(
        &fused_row_spec(RowReduceKind::RmsNorm, 32, 32),
        &fused_row_names(),
        &fused_row_fills(),
        &wh,
    )
    .unwrap_or_else(|e| panic!("the rmsnorm 32×32 template must build on wormhole: {e:?}"));
    let scaler = chain
        .tensors
        .iter()
        .find(|t| t.name == "rms_scaler")
        .expect("the chain plants a `rms_scaler` Param carrying the reduction's scaler");
    let x = chain
        .tensors
        .iter()
        .find(|t| t.name == "x")
        .expect("the chain reads an `x` input");
    let (one, two) = (seeded_inputs(&chain, 1), seeded_inputs(&chain, 2));
    assert_eq!(
        one.tensors.get(&scaler.id),
        two.tensors.get(&scaler.id),
        "the planted scaler must be IDENTICAL at every seed — it is the constant that makes the reduction a mean, not data to explore"
    );
    assert_eq!(
        elements(one.tensors.get(&scaler.id).expect("the scaler is supplied")),
        // ★ 1.0, NOT 1/cols — AND THE CHANGE IS THE POINT. The scaler used to carry `1/cols` so the
        // reduce engine produced a mean directly. But that engine multiplies its scaler into EVERY
        // TERM, so the chain computed `Σ(xᵢ²·k)` where the specification computes `(Σxᵢ²)/cols` —
        // equal in exact arithmetic, and a full bf16 ulp apart on the RDU, which
        // `spatial_front_end_lock` caught. The divide now happens ONCE, in
        // `SfpuKind::Rsqrt { over }`. What this assertion protects is unchanged: the scaler is
        // program text, identical at every seed, never data for a corpus to explore.
        vec![1.0f64; 32 * 32],
        "the planted scaler must reproduce its own recorded fill exactly (1.0 over one whole tile — the \
         divide moved into the rsqrt so it happens once); a corpus that overwrote it would compare two \
         programs on data neither was built for"
    );
    assert_ne!(
        one.tensors.get(&x.id),
        two.tensors.get(&x.id),
        "the external activations must VARY with the seed, or the multi-seed corpus explores nothing"
    );
}

// ── F. The differential this lock is FOR, and cannot yet run ───────────────────────────────────────

/// The swiglu TEMPLATE — before any pass, so the differential can run the pipeline twice, once with
/// `fuse_generic` and once without, and attribute any difference to that pass alone.
fn swiglu_template(target: &SpatialTarget) -> SpatialProgram {
    use logicaffeine_compile::spatial::passes::from_tensor_op::{from_swiglu, SwiGluNames};
    use logicaffeine_compile::tensor_recognize::SwiGluSpec;
    let spec = SwiGluSpec {
        total: 32,
        gate: Default::default(),
        up: Default::default(),
        y: Default::default(),
    };
    let names = SwiGluNames {
        gate: "g".into(),
        up: "u".into(),
        y: "y".into(),
    };
    from_swiglu(
        &spec,
        &names,
        (
            &Fill::Pattern {
                modulus: 5,
                shift: -2,
            },
            &Fill::Pattern {
                modulus: 3,
                shift: 1,
            },
        ),
        target,
    )
    .expect("the swiglu template must build on wormhole")
}

/// ★ THE DIFFERENTIAL THIS LOCK EXISTS FOR, AND IT NOW RUNS.
///
/// It was `#[ignore]`d while every row workload had exactly ONE form, because a differential needs two.
/// The second form arrived when `fuse_generic` became a real pass: the SAME computation exists both as
/// the chain legalization produced and as the chain fusion collapsed, and they are built here by
/// running the pipeline with and without that one pass.
///
/// This is the property that makes a fused kernel trustworthy at all. A hand-written fused operator is
/// an opaque claim; a fused one that provably agrees with its unfused source over a corpus is a
/// rewrite. `swiglu` is the case that actually fuses today (two stages become one register window),
/// which is why it is the subject.
#[test]
fn fusion_computes_what_the_unfused_chain_computes() {
    use logicaffeine_compile::spatial::passes::{
        fuse_generic, fuse_readers, persist, place, size_buffers, synthesize_handshakes, tile,
        verify,
    };

    let wh = SpatialTarget::wormhole();
    // Everything after fusion, so the two sides differ ONLY by that pass.
    let rest = |p: SpatialProgram| -> SpatialProgram {
        let p = tile::tile(&p, &wh).expect("tile");
        let p = place::place(&p, &wh).expect("place");
        let p = fuse_readers::fuse_readers(&p, &wh).expect("fuse_readers");
        let p = size_buffers::size_buffers(&p, &wh).expect("size_buffers");
        let p =
            synthesize_handshakes::synthesize_handshakes(&p, &wh).expect("synthesize_handshakes");
        let p = persist::persist(&p, &wh).expect("persist");
        verify::verify(&p, &wh).expect("verify")
    };

    let template = swiglu_template(&wh);
    let unfused = rest(template.clone());
    let fused = rest(fuse_generic::fuse_generic(&template, &wh).expect("fuse_generic"));

    // Non-vacuity: if the pass fused nothing, the two sides are the same program and the comparison
    // below proves only that evaluation is deterministic.
    let (nu, nf) = (compute_stage_count(&unfused), compute_stage_count(&fused));
    assert!(
        nf < nu,
        "fusion changed nothing ({nu} compute stages before, {nf} after), so this differential is \
         vacuous. Either `fuse_generic` stopped firing on swiglu or the chain stopped being fusable — \
         find out which; do not delete this assertion, it is what stops the test passing by default."
    );

    // ★ THE CLAIM THE PROGRAM MADE IS THE QUESTION ASKED. swiglu declares `NumericsClaim::Approx`
    // (a sigmoid on a bf16 datapath), and fusion legitimately rounds LESS than the chain it replaces —
    // it keeps the inner result in the destination registers instead of packing and re-reading it. So
    // this discharges the approximate claim rather than demanding bit-equality, at a bound four orders
    // of magnitude tighter than the bf16 ulp a wrong answer would have to hide inside.
    let corpus = seeded_corpus(&unfused, &[1, 2, 3, 7, 42]);
    equivalent_within(&unfused, &fused, &corpus, APPROX_REL_TOL).unwrap_or_else(|d| {
        panic!(
            "\n★ `fuse_generic` CHANGED WHAT THE PROGRAM COMPUTES: {d}\n\n\
             The unfused chain is the specification here — it is what legalization produced from the \
             recognized workload. Fusion may only rearrange how that is executed, never what it means. \
             Fix `spatial/passes/fuse_generic.rs` (its operand-order rule and its emittability guard are \
             the usual culprits), never this comparison.\n"
        )
    });
}

/// ★ ACROSS AN FP32 PAGE, FUSION CHANGES NO VALUE. Fusion keeps an inner result in the destination registers where the
/// chain packed it to a page; across a bf16 page the two round differently and `fuse_generic` leaves them apart, but an
/// fp32 page holds exactly what an fp32 register holds. So on the SN50's fp32 islands the fused and the unfused chain
/// are the same arithmetic, and must agree to the last bit — a fused body that carried its inner result wider than the
/// datapath does would make a value depend on whether a pass fused, and with it on how a program's size chose its form.
#[test]
fn across_an_fp32_page_fusion_changes_no_value() {
    use logicaffeine_compile::spatial::passes::{fuse_generic, persist, place, size_buffers, synthesize_handshakes, tile, verify};
    let sn50 = SpatialTarget::sn50_rdu();
    let rest = |p: SpatialProgram| -> SpatialProgram {
        let p = tile::tile(&p, &sn50).expect("tile");
        let p = place::place(&p, &sn50).expect("place");
        let p = logicaffeine_compile::spatial::passes::fuse_readers::fuse_readers(&p, &sn50).expect("fuse_readers");
        let p = size_buffers::size_buffers(&p, &sn50).expect("size_buffers");
        let p = synthesize_handshakes::synthesize_handshakes(&p, &sn50).expect("synthesize_handshakes");
        let p = persist::persist(&p, &sn50).expect("persist");
        verify::verify(&p, &sn50).expect("verify")
    };
    let template = swiglu_template(&sn50);
    assert!(
        template.buffers.iter().all(|b| b.dtype != logicaffeine_compile::codegen_sva::fabric_target::Precision::Bf16),
        "the chain's pages are the islands' fp32"
    );
    let unfused = rest(template.clone());
    let fused = rest(fuse_generic::fuse_generic(&template, &sn50).expect("fuse_generic"));
    assert!(compute_stage_count(&fused) < compute_stage_count(&unfused), "fusion fired, so the comparison is not vacuous");
    let corpus = seeded_corpus(&unfused, &[1, 2, 3, 7, 42]);
    equivalent_under(&unfused, &fused, &corpus).unwrap_or_else(|d| panic!("the fused chain computes other bits than the unfused one: {d}"));
}

/// ★ A NORM'S FORMS ARE ONE ARITHMETIC. A per-head norm is lowered flat while its heads fit a page and blocked, a head an
/// item, when they do not, so a prefill chunk's row count chooses the form. Each head's sum, root and rescale are the
/// same operations in either, and the same heads come out the same bits whatever else the program holds.
#[test]
fn a_per_head_norm_is_the_same_bits_in_its_flat_and_blocked_forms() {
    use logicaffeine_compile::spatial::eval::eval_choreography_every_tensor;
    use logicaffeine_compile::spatial::pipeline::compile_program_to_spatial;
    let sn50 = SpatialTarget::sn50_rdu();
    let d = sn50.tile.cols;
    let x = |i: usize| (((i * 7919 + 13) % 2000) as f64 - 1000.0) / 997.0;
    let first_two = |heads: usize, op: &str| -> Vec<f64> {
        let n = heads * d;
        let mut src = format!("## Main\nLet X be a 1 by {n} tensor of Real.\nLet G be a 1 by {d} tensor of Real.\n");
        for i in 0..n {
            src += &format!("Set item {} of X to {:?}.\n", i + 1, x(i));
        }
        for i in 0..d {
            src += &format!("Set item {} of G to {:?}.\n", i + 1, 0.5 + x(i + 7) / 4.0);
        }
        src += &format!("Let Y be {}.\nShow item 1 of Y.\n", op.replace("{heads}", &heads.to_string()));
        let p = compile_program_to_spatial(&src, &sn50).expect("a tensor program").into_whole().unwrap_or_else(|why| panic!("{heads} heads: {why}"));
        let out = eval_choreography_every_tensor(&p, &inputs_from_fills(&p)).expect("evaluates");
        let y = p.tensors.iter().find(|t| t.name == "Y").expect("Y");
        let Value::Float(v) = out.tensors.get(&y.id).expect("written").clone() else { panic!("Y through integers") };
        v[..2 * d].to_vec()
    };
    for op in ["l2norm of X across {heads} heads", "rmsnorm of X with weight G across {heads} heads"] {
        let flat = first_two(2, op);
        for heads in [8, 14, 40] {
            let other = first_two(heads, op);
            let differ: Vec<usize> = (0..flat.len()).filter(|&i| flat[i].to_bits() != other[i].to_bits()).collect();
            assert!(differ.is_empty(), "`{op}`: the first two heads of {heads} differ from 2 heads' at {} of {} values, first {:?}", differ.len(), flat.len(), differ.first().map(|&i| (flat[i], other[i])));
        }
    }
}

/// The compute stages of a program — how many register windows its site will open.
fn compute_stage_count(p: &SpatialProgram) -> usize {
    p.stages
        .iter()
        .filter(|s| matches!(s.kind, StageKind::Compute(_)))
        .count()
}

/// Every row workload has exactly ONE legalized form: a chain of the general form, never a
/// workload-shaped operator. `spatial_algebra_lock` owns the printer-arm half of this claim.
#[test]
fn every_row_workload_legalizes_into_the_general_form() {
    let wh = SpatialTarget::wormhole();
    let (rows, cols) = (32usize, 32usize);
    for kind in [
        RowReduceKind::RmsNorm,
        RowReduceKind::Softmax,
        RowReduceKind::LayerNorm,
    ] {
        let p = from_fused_row(
            &fused_row_spec(kind, rows, cols),
            &fused_row_names(),
            &fused_row_fills(),
            &wh,
        )
        .unwrap_or_else(|e| panic!("{kind:?} must build on wormhole: {e:?}"));
        let ops = compute_ops(&p);
        assert!(
            !ops.is_empty(),
            "{kind:?} produced no compute stages, so every check here is vacuous"
        );
        assert!(
            names_of(&ops).iter().all(|n| *n == "Generic"),
            "★ {kind:?} lowered to {:?} — something other than the general form. A workload-shaped \
             operator is one print arm per backend for one workload, which is the thing this whole \
             campaign removed. Fix `from_fused_row` in `spatial/passes/from_tensor_op.rs`.",
            names_of(&ops)
        );
    }
}
