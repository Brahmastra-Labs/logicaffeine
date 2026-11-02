//! spatial_recognize_stages_lock — the spatial front end's WHOLE-PROGRAM walk: every tensor statement of an
//! authored forward becomes a typed spec, in program order, with its dataflow and its carried state named.
//!
//! `spatial::passes::recognize_stages` walks a program's top-level statements in the CUDA orchestrator's
//! order (composite blocks first, so their inner loops are never claimed by a simpler recognizer), lifting each
//! into a [`TensorOpSpec`] over the shared recognizers of `tensor_recognize` and `tensor_recognize::qwen`.
//! What it cannot lift it RETURNS — `unrecognized` lists the statement by index and binder — never drops, so a
//! composition can refuse by name. `links` is the dataflow by variable name (the producer stage of every
//! tensor and the stages that read it); `state_tensors` is the carried state (the conv rings and the GDN
//! memories a step reads and writes in place). These are the inputs B-P7's `compose` takes.
//!
//! ★ MUTATION PROBE: remove `attention` from `RECOGNIZER_ORDER` → `the_probe_corpus_is_recognized_or_listed`
//! fails (the attention probe is listed, not lifted), and so does the agreement test in
//! `recognizer_migration_lock` (the CUDA walk still lifts it).
//! ★ MUTATION PROBE: make `state_tensors` return only the gated-delta memories → `state_tensors_are_the_
//! carried_ones` fails naming the missing conv rings.
//! ★ MUTATION PROBE: let `recognize_fused_row`'s flat fallback accept any divisor → `no_recognizer_claims_
//! another_sugars_program` fails (fused_row also claims gelu and silu) and so does `a_fragment_is_never_
//! lifted_as_a_whole_op`.
//!
//!   cargo nextest run -p logicaffeine-tests --test spatial_recognize_stages_lock
#![cfg(feature = "codegen")]

use std::collections::{BTreeMap, BTreeSet, HashMap};

use logicaffeine_compile::codegen_cuda::qwen38::{gen_qwen38_lg, Qwen38Cfg};
use logicaffeine_compile::codegen_cuda::qwen_emit::{forward_graph, ForwardError, Q8MatKind, StageRefusal};
use logicaffeine_compile::spatial::passes::recognize_stages::{
    links, recognize_stages, state_tensors, RecognizedProgram, TensorOpSpec,
};
use logicaffeine_compile::spatial::passes::SpatialReject;
use logicaffeine_compile::ui_bridge::with_parsed_program;

#[path = "compile_common/tensor_probes.rs"]
mod tensor_probes;
use tensor_probes::{probe, IMPLEMENTED_PROBES, OP_PROBES, PENDING_SUGAR_PROBES};

fn tiny_forward() -> String {
    gen_qwen38_lg(&Qwen38Cfg::tiny(), "/w").0
}

fn walk(source: &str) -> RecognizedProgram {
    recognize_stages(source).unwrap_or_else(|e| panic!("the walk refused the program: {e:?}"))
}

/// The `Let … be <op>` statements of an authored program — every `Let` that is not a tensor declaration.
fn op_statements(source: &str) -> usize {
    source
        .lines()
        .filter(|l| l.starts_with("Let ") && !l.contains(" tensor of "))
        .count()
}

/// The authored op order of one Gated-DeltaNet layer of `gen_qwen38_lg`, as spec kinds.
const GDN_LAYER: &[&str] = &[
    "FusedRow",
    "Matmul",
    "Matmul",
    "Matmul",
    "Matmul",
    "CausalConv",
    "CausalConv",
    "CausalConv",
    "Silu",
    "Silu",
    "Silu",
    "L2Norm",
    "L2Norm",
    "Matmul",
    "Sigmoid",
    "Matmul",
    "Decay",
    "GatedDelta",
    "Matmul",
    "Elementwise",
    "FusedRow",
    "Matmul",
    "Matmul",
    "SwiGlu",
    "Matmul",
    "Elementwise",
];

/// The authored op order of one gated-attention layer, as spec kinds.
const ATTN_LAYER: &[&str] = &[
    "FusedRow",
    "Matmul",
    "Matmul",
    "Matmul",
    "Matmul",
    "FusedRow",
    "FusedRow",
    "Rope",
    "Rope",
    "Attention",
    "Sigmoid",
    "Elementwise",
    "Matmul",
    "Elementwise",
    "FusedRow",
    "Matmul",
    "Matmul",
    "SwiGlu",
    "Matmul",
    "Elementwise",
];

/// The final norm and the LM head.
const TAIL: &[&str] = &["FusedRow", "Matmul"];

#[test]
fn every_statement_of_the_tiny_qwen38_forward_is_recognized_into_a_typed_spec() {
    let src = tiny_forward();
    let rp = walk(&src);
    assert!(
        rp.unrecognized.is_empty(),
        "every statement of the authored forward is a known op; unrecognized: {:?}",
        rp.unrecognized
    );
    assert_eq!(
        rp.stages.len(),
        op_statements(&src),
        "one typed spec per `Let … be <op>` statement"
    );
    assert!(
        rp.stages
            .windows(2)
            .all(|w| w[0].stmt_index < w[1].stmt_index),
        "stages come in program order"
    );

    // The tiny config is 4 layers at interval 4: GDN, GDN, GDN, attention — the 3:1 schedule, visible as kinds.
    let want: Vec<&str> = [GDN_LAYER, GDN_LAYER, GDN_LAYER, ATTN_LAYER, TAIL].concat();
    let got: Vec<&str> = rp.stages.iter().map(|s| s.op.kind()).collect();
    assert_eq!(
        got, want,
        "the spec kinds are the authored op sequence, layer by layer"
    );
    let count = |k: &str| got.iter().filter(|g| **g == k).count();
    assert_eq!(
        (count("GatedDelta"), count("Attention")),
        (3, 1),
        "three Gated-DeltaNet layers to one attention layer"
    );
    assert_eq!(
        count("CausalConv"),
        9,
        "each GDN layer convolves q, k and v"
    );

    // Binders and operands come back by NAME, in the authored spelling.
    let last = rp.stages.last().unwrap();
    assert_eq!(last.out, "logits");
    assert_eq!(last.ins, vec!["xf", "Wout"]);
    let gd = rp
        .stages
        .iter()
        .find(|s| s.op.kind() == "GatedDelta")
        .unwrap();
    assert_eq!(gd.out, "od0");
    assert_eq!(
        gd.ins,
        vec!["qn0", "kn0", "vs0", "zz0", "bt0", "dy0", "St0", "Gn0"]
    );
    let TensorOpSpec::GatedDelta(spec) = &gd.op else {
        unreachable!()
    };
    assert_eq!(
        (spec.hv, spec.hk, spec.d),
        (2, 1, 4),
        "48/16 heads at head_dim 128 scale down to 2/1 at 4"
    );
    let conv = rp
        .stages
        .iter()
        .find(|s| s.op.kind() == "CausalConv")
        .unwrap();
    let TensorOpSpec::CausalConv(c) = &conv.op else {
        unreachable!()
    };
    assert_eq!(
        (rp.name(c.cs), c.taps, c.chan),
        ("Csq0", 4, 4),
        "the ring is the declared state; 4 taps over the q width"
    );
}

#[test]
fn links_follow_variable_names() {
    let src = tiny_forward();
    let rp = walk(&src);
    let declared: BTreeSet<&str> = rp.declared.iter().map(|d| d.name.as_str()).collect();
    assert!(
        declared.contains("x") && declared.contains("Wout") && declared.contains("St0"),
        "the declarations are read off the program: {declared:?}"
    );

    // Every input a stage reads is a declared tensor or an earlier stage's output — nothing comes from nowhere.
    let mut produced: BTreeSet<&str> = BTreeSet::new();
    for s in &rp.stages {
        for i in &s.ins {
            assert!(
                declared.contains(i.as_str()) || produced.contains(i.as_str()),
                "stage {} (`{}`) reads `{i}`, which is neither declared nor produced earlier",
                s.stmt_index,
                s.out
            );
        }
        produced.insert(s.out.as_str());
    }

    // The links are the dataflow by name: one per producer, consumers strictly later, complete in both directions.
    let ls = links(&rp.stages);
    assert_eq!(ls.len(), rp.stages.len(), "one link per stage output");
    for (i, l) in ls.iter().enumerate() {
        assert_eq!(l.producer, i);
        assert_eq!(l.tensor, rp.stages[i].out);
        for &c in &l.consumers {
            assert!(c > i, "a consumer reads after its producer");
            assert!(
                rp.stages[c].ins.contains(&l.tensor),
                "stage {c} is listed as a consumer of `{}` but does not read it",
                l.tensor
            );
        }
        let readers: Vec<usize> = (i + 1..rp.stages.len())
            .filter(|&j| rp.stages[j].ins.contains(&l.tensor))
            .collect();
        assert_eq!(
            l.consumers, readers,
            "every later reader of `{}` is a consumer",
            l.tensor
        );
    }
    assert!(
        ls.last().unwrap().consumers.is_empty(),
        "the logits are the program's result — nobody reads them"
    );
    // The residual stream fans out: `xr0` feeds the post-attention norm AND the second residual add.
    let xr0 = ls.iter().find(|l| l.tensor == "xr0").unwrap();
    assert_eq!(
        xr0.consumers.len(),
        2,
        "xr0 is read by x2n0 and xo0: {:?}",
        xr0.consumers
    );
}

#[test]
fn state_tensors_are_the_carried_ones() {
    let rp = walk(&tiny_forward());
    let want: Vec<String> = (0..3)
        .flat_map(|l| {
            [
                format!("Csq{l}"),
                format!("Csk{l}"),
                format!("Csv{l}"),
                format!("St{l}"),
            ]
        })
        .collect();
    assert_eq!(
        state_tensors(&rp.stages),
        want,
        "the conv rings and the GDN memory of each of the three GDN layers, in order"
    );
    // Every state tensor is declared (it persists across steps; a step never creates it) and is read by the
    // stage that carries it.
    let declared: BTreeSet<&str> = rp.declared.iter().map(|d| d.name.as_str()).collect();
    for s in state_tensors(&rp.stages) {
        assert!(
            declared.contains(s.as_str()),
            "state `{s}` must be a declared tensor"
        );
        assert!(
            rp.stages.iter().any(|st| st.ins.contains(&s)),
            "state `{s}` is read by its stage"
        );
    }
}

/// The kinds the CUDA forward has kernels for, and the stage name each spatial kind projects to under it.
fn cuda_kind(spec: &TensorOpSpec) -> Option<&'static str> {
    match spec {
        TensorOpSpec::Matmul(_) => Some("Matmul"),
        TensorOpSpec::Elementwise(_) => Some("Elementwise"),
        TensorOpSpec::FusedRow(_) => Some("FusedRow"),
        TensorOpSpec::Rope(_) => Some("Rope"),
        TensorOpSpec::Silu(_) | TensorOpSpec::Gelu(_) => Some("Silu"),
        TensorOpSpec::Transpose(_) => Some("Transpose"),
        TensorOpSpec::Attention(_) => Some("Attention"),
        TensorOpSpec::Moe(_) => Some("Moe"),
        TensorOpSpec::Sigmoid(_)
        | TensorOpSpec::Softcap(_)
        | TensorOpSpec::Scale(_)
        | TensorOpSpec::HeadSlice(_)
        | TensorOpSpec::Join(_)
        | TensorOpSpec::HeadMatmul(_)
        | TensorOpSpec::Decay(_)
        | TensorOpSpec::L2Norm(_)
        | TensorOpSpec::CausalConv(_)
        | TensorOpSpec::GatedDelta(_)
        | TensorOpSpec::SwiGlu(_)
        | TensorOpSpec::FullAttention(_)
        | TensorOpSpec::RowGather(_)
        | TensorOpSpec::RowsGather(_)
        | TensorOpSpec::ArgMax(_)
        | TensorOpSpec::SetRow(_)
        | TensorOpSpec::IntRequant(_)
        | TensorOpSpec::IntClamp(_)
        | TensorOpSpec::Relu(_)
        | TensorOpSpec::LutAttention(_) => None,
    }
}

/// The CUDA forward lowers this walk (MIGRATE-2): it refuses the tiny Qwen3.8 forward by exactly the walk's ops it has no
/// kernel for — the Gated-DeltaNet family and swiglu — and by its rope, which turns part of each head where the
/// forward's kernel turns every dimension; every other kind the walk lifts there it lowers.
#[test]
fn the_forward_lowers_the_walk_and_refuses_what_it_has_no_kernel_for() {
    let cfg = Qwen38Cfg::tiny();
    assert!(cfg.rope_dim < cfg.hd, "the tiny Qwen3.8 rotates part of each head");
    let src = tiny_forward();
    let rp = walk(&src);
    let refused: BTreeMap<&'static str, BTreeSet<&'static str>> = with_parsed_program(&src, |parsed, interner| {
        let (stmts, _t, _p) = parsed.expect("parses");
        match forward_graph(stmts, interner, &HashMap::new(), &HashMap::new(), Q8MatKind::Dequant) {
            Err(ForwardError::Refused(refusals)) => refusals.iter().fold(BTreeMap::new(), |mut by_kind, r| {
                match r {
                    StageRefusal::NoKernel { kind, why, .. } => by_kind.entry(*kind).or_insert_with(BTreeSet::new).insert(*why),
                    other => panic!("the walk lifts every statement of the tiny forward: {other}"),
                };
                by_kind
            }),
            other => panic!("the tiny forward holds ops the forward has no kernel for: {other:?}"),
        }
    });
    let dropped: BTreeSet<&str> = rp.stages.iter().filter(|s| cuda_kind(&s.op).is_none()).map(|s| s.op.kind()).collect();
    assert_eq!(
        dropped,
        ["CausalConv", "Decay", "GatedDelta", "L2Norm", "Sigmoid", "SwiGlu"].into_iter().collect::<BTreeSet<_>>()
    );
    let mut expected = dropped.clone();
    expected.insert("Rope");
    assert_eq!(
        refused.keys().copied().collect::<BTreeSet<_>>(),
        expected,
        "the forward refuses exactly the walk's kinds it has no kernel for, and the partial rotary"
    );
    for kind in &dropped {
        assert_eq!(refused[kind], BTreeSet::from(["this lane has no kernel for the op"]), "{kind}");
    }
    assert_eq!(
        refused["Rope"],
        BTreeSet::from(["a partial rotary — its kernel turns every dimension of the head"]),
        "the rope is refused for turning part of each head, and for nothing else"
    );
}

#[test]
fn an_unrecognized_statement_is_reported_not_dropped() {
    let src = "## Main\n\
               Let A be a 2 by 2 tensor of Int.\n\
               Set item 1 of A to 1.\nSet item 2 of A to 2.\nSet item 3 of A to 3.\nSet item 4 of A to 4.\n\
               Let B be a 2 by 2 tensor of Int.\n\
               Set item 1 of B to 5.\nSet item 2 of B to 6.\nSet item 3 of B to 7.\nSet item 4 of B to 8.\n\
               Let C be A plus B.\n\
               Let mutable i be 1.\n\
               While i is at most 4:\n\
               \x20   Set item i of C to (item i of C) * 2.\n\
               \x20   Set i to i + 1.\n\
               Show item 1 of C.\n";
    let rp = walk(src);
    assert_eq!(
        rp.stages.len(),
        1,
        "the `plus` is recognized: {:?}",
        rp.stages.iter().map(|s| s.op.kind()).collect::<Vec<_>>()
    );
    assert_eq!(rp.stages[0].op.kind(), "Elementwise");
    // Top-level statements: A(0) Set×4(1-4) B(5) Set×4(6-9) C(10) Let i(11) While(12) Show(13).
    assert_eq!(rp.stages[0].stmt_index, 10);
    assert_eq!(
        rp.unrecognized,
        vec![(12, "While i".to_string())],
        "the loop that is not a tensor op is listed by index, not skipped"
    );
}

/// The spec kind each implemented probe's op lifts to — the probe's own `kind`, stated once in `tensor_probes`, or
/// `None` for a sugar the walk has no spec for (it must then be LISTED, not dropped).
fn expected_kind(op: &str) -> Option<&'static str> {
    Some(probe(op).kind)
}

#[test]
fn the_probe_corpus_is_recognized_or_listed() {
    for p in IMPLEMENTED_PROBES {
        let rp = walk(p.program);
        match expected_kind(p.op) {
            Some(kind) => {
                let kinds: Vec<&str> = rp.stages.iter().map(|s| s.op.kind()).collect();
                assert_eq!(
                    kinds,
                    vec![kind],
                    "probe `{}` lifts to exactly one `{kind}` spec",
                    p.op
                );
                assert!(
                    rp.unrecognized.is_empty(),
                    "probe `{}` leaves nothing unrecognized: {:?}",
                    p.op,
                    rp.unrecognized
                );
                let stage = &rp.stages[0];
                assert!(
                    !stage.ins.is_empty() && !stage.out.is_empty(),
                    "probe `{}`: the stage names its operands and binder",
                    p.op
                );
                for i in &stage.ins {
                    assert!(
                        rp.declared.iter().any(|d| &d.name == i),
                        "probe `{}`: input `{i}` is a declared tensor",
                        p.op
                    );
                }
            }
            None => {
                assert!(rp.stages.is_empty(), "probe `{}` has no spec yet and must not be claimed by another recognizer: {:?}", p.op, rp.stages.iter().map(|s| s.op.kind()).collect::<Vec<_>>());
                assert_eq!(
                    rp.unrecognized.len(),
                    1,
                    "probe `{}` is listed exactly once: {:?}",
                    p.op,
                    rp.unrecognized
                );
            }
        }
    }
    for p in PENDING_SUGAR_PROBES {
        assert!(
            matches!(recognize_stages(p.program), Err(SpatialReject::Parse(_))),
            "pending sugar `{}` is a typed parse refusal",
            p.op
        );
    }
    assert_eq!(
        IMPLEMENTED_PROBES.len() + PENDING_SUGAR_PROBES.len(),
        OP_PROBES.len(),
        "the probe tables partition the universe"
    );
}

/// The fragment miscompiles the gap ledger found — `gelu` and `silu` lifted as a flat rmsnorm (their
/// `x / (1 + e^…)` normalize loop taken for one), `moe` lifted as a softmax (its router softmax taken for the
/// whole block). Each must be recognized as ITS OWN spec kind by the walk, and the single-op front end
/// (`template_from_source`) must never return a program of another family for it: it lifts the op as itself
/// or refuses it typed.
///
/// ★ MUTATION PROBE: make `recognize_fused_row`'s flat fallback accept any divisor again (drop the
/// `binds_rms_denominator` check) → gelu and silu lower as `rmsnorm_1x4` here; drop the `splice_binder`
/// rule → moe lowers as `softmax_1x2`.
#[test]
fn a_fragment_is_never_lifted_as_a_whole_op() {
    use logicaffeine_compile::spatial::pipeline::template_from_source;
    use logicaffeine_compile::spatial::target::SpatialTarget;
    let target = SpatialTarget::sn50_rdu();
    for (op, kind) in [("gelu", "Gelu"), ("silu", "Silu"), ("moe", "Moe")] {
        let p = tensor_probes::probe(op);
        let rp = walk(p.program);
        let kinds: Vec<&str> = rp.stages.iter().map(|s| s.op.kind()).collect();
        assert_eq!(kinds, vec![kind], "the walk lifts `{op}` as itself");
        match template_from_source(p.program, &target) {
            Ok(sp) => assert!(sp.name.starts_with(p.family), "`{op}` lowered as `{}` — a fragment of its desugar taken for the whole program", sp.name),
            Err(SpatialReject::Unrecognized { .. }) => {}
            Err(other) => panic!("`{op}`: the single-op front end must lift it as itself or refuse it as unrecognized, not {other:?}"),
        }
    }
}

/// The recognizer each implemented sugar belongs to.
fn own_recognizer(op: &str) -> Option<&'static str> {
    Some(match op {
        "matmul" => "matmul",
        "plus" | "hadamard" => "elementwise",
        "rmsnorm" | "softmax" | "rmsnorm_weighted" | "fused_add_rmsnorm" | "layernorm"
        | "softmax_rows" => "fused_row",
        "swiglu" => "swiglu",
        "sigmoid" => "sigmoid",
        "gelu" => "gelu",
        "silu" => "silu",
        "l2norm" | "l2norm_heads" => "l2norm",
        "rope_partial" => "rope",
        "causal_conv" => "causal_conv",
        "gated_delta" => "gated_delta",
        "attention" => "attention",
        "moe" => "moe",
        "transpose" => "transpose",
        "decay" => "decay",
        "full_attention" => "full_attention",
        "row_of" => "row_gather",
        "rows_of" => "rows_gather",
        "argmax" => "argmax",
        "set_row" => "set_row",
        "scale" => "scale",
        "softcap" => "softcap",
        "head_slice" => "head_slice",
        "join" => "join",
        "head_matmul" => "head_matmul",
        "int_requant" => "int_requant",
        "int_clamp" => "int_clamp",
        "relu" => "relu",
        "lut_attention" => "lut_attention",
        other => {
            panic!("probe `{other}` has no owning recognizer — add it here when its spec lands")
        }
    })
}

/// Every recognizer is keyed on its own sugar's WHOLE desugar shape, so on each probe program exactly one of
/// them answers — the op's own — and the walk's order is not load-bearing on the corpus. This is the property
/// that makes a fragment lift impossible: a recognizer that matched a piece of another sugar (the softmax
/// inside `moe`, the `x / (1 + e^…)` loop of gelu) would answer here too.
#[test]
fn no_recognizer_claims_another_sugars_program() {
    use logicaffeine_compile::spatial::passes::recognize_stages::RECOGNIZER_ORDER;
    use logicaffeine_language::ast::stmt::Stmt;
    for p in IMPLEMENTED_PROBES {
        let claimed: Vec<&str> = with_parsed_program(p.program, |parsed, interner| {
            let (stmts, _t, _p) = parsed.expect("parses");
            let mut names = Vec::new();
            for s in stmts
                .iter()
                .filter(|s| matches!(s, Stmt::Splice { .. } | Stmt::While { .. }))
            {
                let one = std::slice::from_ref(s);
                for (name, r) in RECOGNIZER_ORDER {
                    if r(one, interner).is_some() && !names.contains(name) {
                        names.push(*name);
                    }
                }
            }
            names
        });
        let want: Vec<&str> = own_recognizer(p.op).into_iter().collect();
        assert_eq!(
            claimed, want,
            "probe `{}`: the recognizers that claim some statement of it must be exactly its own",
            p.op
        );
    }
}
