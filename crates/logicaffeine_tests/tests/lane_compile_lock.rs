//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! THE COMPILED GENERATION LANE: a model's decode step, authored in LOGOS from its spec, lifted by the shared front
//! end, lowered to the kernel library and printed as one CUDA program — for every architecture the author writes, with
//! nothing in the path written for one model.
//!
//! These are the lane's compile-side locks (no GPU): the step graph names the token and position as the step's
//! parameters and the caches, rings and memories as its carried state; every op is lowered exactly once; every
//! quantized projection is read as rows straight from the checkpoint, in the order its binding picks them; the fusion
//! rules fire exactly where their dataflow allows; each knob changes the program; what the lane cannot compile is
//! refused by name; and the program compiles with `nvcc` and refuses a checkpoint or a sidecar it was not compiled
//! with. `cuda_lane_lock` runs the programs on the GPU.
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(feature = "codegen")]

mod common;
#[path = "common/scratch.rs"]
mod scratch;
#[path = "common/tiny_models.rs"]
mod tiny_models;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use logicaffeine_compile::codegen_cuda::knobs::CudaKnobs;
use logicaffeine_compile::codegen_cuda::lane::graph::{Role, StepGraph};
use logicaffeine_compile::codegen_cuda::lane::arena::{array_bytes, scratch, ALIGN as ARENA_ALIGN};
use logicaffeine_compile::codegen_cuda::lane::plan::{
    kind, plan, quant_in_place, ConvPart, Dst, Epilogue, Fill, Launch, NormedPart, Part, Router, RowPart, RowSel, ScaleBy, Schedule, SharedExperts, Split,
    Touch, RULES,
};
use logicaffeine_compile::tensor_recognize::qwen::MoeAct;
use logicaffeine_compile::tensor_recognize::ElementwiseOp;
use logicaffeine_compile::codegen_cuda::lane::weights::{Placement, Rows};
use logicaffeine_compile::codegen_cuda::lane::residence::{matrix_bytes, place, pooled, CONTEXT_RESERVE, POOL_ALIGN};
use logicaffeine_compile::codegen_cuda::lane::{compile, compile_holding, compile_batch, compile_batch_holding, compile_split, compile_split_holding, decode_io, FormKind, Lane, LaneError, LaneOptions};
use logicaffeine_compile::codegen_cuda::provenance::{header, Provenance};
use logicaffeine_compile::codegen_cuda::qwen_emit::Q8MatKind;
use logicaffeine_compile::model::bind::{bind, rope_tables, values, Source};
use logicaffeine_compile::model::gguf::Gguf;
use logicaffeine_compile::model::quant::GgmlType;
use logicaffeine_compile::model::program::Cut;
use logicaffeine_compile::model::spec::{Ffn, Mixer, Mla, ModelSpec};
use logicaffeine_compile::spatial::passes::recognize_stages::recognize_stages;
use tiny_models::{Tiny, ALL, BLOCK_FORMATS};

const CTX: usize = 24;
const NUMERICS: [Q8MatKind; 3] = [Q8MatKind::Dequant, Q8MatKind::Int8Act, Q8MatKind::TwoPassInt8];

fn dir() -> PathBuf {
    scratch::dir("logos_lane_compile")
}

/// The tiny model and its checkpoint, written under a name of the caller's.
fn checkpoint(t: Tiny, tag: &str) -> (ModelSpec, PathBuf) {
    let spec = t.spec(CTX);
    let path = dir().join(format!("{}-{tag}.gguf", t.name()));
    tiny_models::write_checkpoint(&spec, &path);
    (spec, path)
}

fn options(numerics: Q8MatKind) -> LaneOptions {
    LaneOptions { ctx: CTX, knobs: CudaKnobs { activations: numerics, ..CudaKnobs::default() }, prefill: 1 }
}

fn chunked(numerics: Q8MatKind, prefill: usize) -> LaneOptions {
    LaneOptions { prefill, ..options(numerics) }
}

fn compiled(t: Tiny, numerics: Q8MatKind, tag: &str) -> Lane {
    let (spec, path) = checkpoint(t, tag);
    compile(&spec, &path, &options(numerics)).unwrap_or_else(|e| panic!("{}: {e}", t.name()))
}

fn compiled_chunks(t: Tiny, numerics: Q8MatKind, prefill: usize, tag: &str) -> Lane {
    let (spec, path) = checkpoint(t, tag);
    compile(&spec, &path, &chunked(numerics, prefill)).unwrap_or_else(|e| panic!("{}: {e}", t.name()))
}

fn counts(pairs: &[(&'static str, usize)]) -> BTreeMap<&'static str, usize> {
    pairs.iter().copied().collect()
}

#[test]
fn every_architecture_lowers_every_op_of_every_form_exactly_once() {
    for &t in ALL {
        for numerics in NUMERICS {
            let lane = compiled_chunks(t, numerics, 16, "once");
            assert_eq!(lane.forms.iter().map(|f| f.rows).collect::<Vec<_>>(), [1, 16, 8, 4, 2], "{}: the decode step and every chunk", t.name());
            for form in &lane.forms {
                let mut covered: Vec<usize> = form.schedule.ops.iter().flatten().copied().collect();
                covered.sort_unstable();
                assert_eq!(
                    covered,
                    (0..form.graph.ops.len()).collect::<Vec<_>>(),
                    "{} {numerics:?} rows {}: each op in one launch",
                    t.name(),
                    form.rows
                );
                for line in form.source.lines() {
                    assert!(lane.program.cu.contains(&format!("//   {line}\n")), "{}: the program carries `{line}`", t.name());
                }
            }
            assert_eq!(
                format!("{}\n", lane.program.cu.lines().next().unwrap()),
                header(Provenance::Compiled),
                "{}: the program says it was compiled",
                t.name()
            );
        }
    }
}

#[test]
fn the_step_reads_its_token_and_position_and_carries_its_state() {
    let lane = compiled_chunks(Tiny::Qwen35, Q8MatKind::Int8Act, 4, "roles");
    let decode = lane.decode();
    let of = |r: Role| decode.graph.of_role(r).map(|t| t.name.clone()).collect::<BTreeSet<_>>();
    assert_eq!(of(Role::Param), ["pos", "tok"].map(String::from).into());
    let state: BTreeSet<String> = ["Csk0", "Csk1", "Csq0", "Csq1", "Csv0", "Csv1", "Kc2", "St0", "St1", "Vc2"].map(String::from).into();
    assert_eq!(of(Role::State), state);
    for name in ["Emb", "Wout", "Rcos", "Rsin", "an0", "Wq0", "Wb1", "Gn1", "Qn2", "Wqg2"] {
        assert!(of(Role::Constant).contains(name), "{name} is a constant");
    }
    for name in &state {
        assert_eq!(decode.schedule.buffers[name].fill, Fill::Zero, "{name} starts at zero");
    }
    let chunk = lane.chunk(4).unwrap();
    let params: BTreeSet<String> = chunk.graph.of_role(Role::Param).map(|t| t.name.clone()).collect();
    assert_eq!(params, ["pos_s0", "toks"].map(String::from).into(), "a chunk reads its tokens and its first position");
    assert_eq!(chunk.graph.tensors["toks"].len(), 4);
    let carried: BTreeSet<String> = chunk.graph.of_role(Role::State).map(|t| t.name.clone()).collect();
    assert_eq!(carried, state, "a chunk carries the decode step's state, and no other");
}

/// The census of a step and the rules that made it: the embedding's gather with the first attention norm
/// (`embed_norm`); per layer of the dense model, the query, key and value projections in one launch with each head's
/// QK-norm and rotation in its epilogue (`gemv_parts`, `qkv_head_epilogue`: the value projected and the key rotated
/// straight into their cache rows), the attention, its quantize, the output projection with the residual add and the
/// FFN norm in its last block (`gemv_norm_epilogue`), the gate and up projections and their SwiGLU in one launch
/// (`glu_pair`) quantizing for the down projection, and the down projection with the residual add and the next norm —
/// the head's, after the last layer — in its last block; then the LM head and the advancing arg-max.
#[test]
fn the_dense_steps_fuse_where_their_dataflow_allows() {
    for t in [Tiny::Qwen3, Tiny::Llama, Tiny::LlamaTied] {
        let lane = compiled(t, Q8MatKind::Int8Act, "dense");
        let s = &lane.decode().schedule;
        assert_eq!(
            s.census(),
            counts(&[
                ("argmax", 1),
                ("attention", 3),
                ("gather_norm", 1),
                ("gemv_glu", 3),
                ("gemv_norm", 6),
                ("gemv_q8", 1),
                ("gemv_rope", 3),
                ("quant", 3)
            ]),
            "{}",
            t.name()
        );
        let qk_norm = t == Tiny::Qwen3;
        for (rule, n) in [
            ("add_norm", 6),
            ("head_norm_rope", if qk_norm { 6 } else { 0 }),
            ("row_write", 6),
            ("argmax_advance", 1),
            ("fused_quant", 10),
            ("glu_pair", 3),
            ("gemv_parts", 3),
            ("qkv_head_epilogue", 6),
            ("gemv_norm_epilogue", 6),
            ("embed_norm", 1),
            ("rope_parts", 0),
        ] {
            assert_eq!(s.fired(rule), n, "{}: {rule}", t.name());
        }
    }
}

/// A lane compiled for a batch (`compile_batch`) carries the model's batch cut beside its decode step and its chunks: one
/// step decoding `B` sequences a token each, every weight read once for all of them (`Cut::Batch`), each sequence with
/// its own position, caches and next token — for every tiny architecture under every numerics, its program building.
/// Apart (`sequences` held), sequence `b`'s arg-max advances its own token, item `b` of the step's tokens, and its own
/// position, and so does each launch of its own a sequence computes; together, each layer's launches of every sequence
/// alike run as one launch whose rows are the sequences, so the step's census is the same at every batch size, its one
/// arg-max advancing every sequence from the first's token and position; either way every op is lowered once. A tensor
/// a step assembles a row at a time (the sequences' attended rows, read by the output projection) is quantized once its
/// last row has landed, before any projection reads it. The batch of four and the chunk of four are two steps. A batch
/// of one is refused: the decode step is it.
#[test]
fn a_batch_lane_decodes_its_sequences_in_one_step() {
    for &t in ALL {
        let (spec, path) = checkpoint(t, "batch");
        for numerics in NUMERICS {
            let what = format!("{} {numerics:?}", t.name());
            let lane = compile_batch(&spec, &path, &LaneOptions { prefill: 4, ..options(numerics) }, 4).unwrap_or_else(|e| panic!("{what}: {e}"));
            let batch = lane.batch().unwrap_or_else(|| panic!("{what}: the batch step"));
            assert_eq!((batch.rows, batch.kind), (4, FormKind::Batch), "{what}");
            assert_eq!(batch.identity.cut, Cut::Batch { rows: 4, ctx: CTX }, "{what}: the model's batch cut");
            assert_eq!(lane.chunk(4).map(|c| c.kind), Some(FormKind::Prefill), "{what}: the chunk of four beside the batch of four");
            let advances = |s: &Schedule| -> Vec<(String, String, usize, usize)> {
                s.launches
                    .iter()
                    .filter_map(|l| if let Launch::ArgMax { advance: Some(a), rows, .. } = l { Some((a.token.clone(), a.position.clone(), a.item, *rows)) } else { None })
                    .collect()
            };
            let apart = compile_batch_holding(&spec, &path, &LaneOptions { prefill: 4, ..options(numerics) }, 4, &["sequences"])
                .unwrap_or_else(|e| panic!("{what} apart: {e}"));
            let apart = &apart.batch().unwrap_or_else(|| panic!("{what} apart: the batch step")).schedule;
            assert_eq!(apart.fired("sequences"), 0, "{what} apart");
            let each: Vec<(String, String, usize, usize)> = (0..4).map(|b| ("toks".to_string(), format!("pos_s{b}"), b, 1)).collect();
            assert_eq!(advances(apart), each, "{what} apart: each sequence advances its own token and position");
            assert!(batch.schedule.fired("sequences") > 0, "{what}: {:?}", batch.schedule.fusions);
            assert_eq!(advances(&batch.schedule), [("toks".to_string(), "pos_s0".to_string(), 0, 4)], "{what}: one arg-max over the sequences");
            let census = |s: &Schedule| -> BTreeMap<&'static str, usize> {
                let mut c = BTreeMap::new();
                for l in s.launches.iter().filter(|l| kind(l) != "view") {
                    *c.entry(kind(l)).or_insert(0) += 1;
                }
                c
            };
            let pair = compile_batch(&spec, &path, &LaneOptions { prefill: 4, ..options(numerics) }, 2).unwrap_or_else(|e| panic!("{what} pair: {e}"));
            let pair = &pair.batch().unwrap_or_else(|| panic!("{what} pair: the batch step")).schedule;
            assert_eq!(census(&batch.schedule), census(pair), "{what}: the same launches at four sequences as at two");
            assert!(census(&batch.schedule).values().sum::<usize>() < census(apart).values().sum::<usize>(), "{what}: fewer launches than apart");
            for (how, s) in [("together", &batch.schedule), ("apart", apart)] {
                let mut lowered: Vec<usize> = s.ops.iter().flatten().copied().collect();
                lowered.sort_unstable();
                assert_eq!(lowered, (0..batch.graph.ops.len()).collect::<Vec<_>>(), "{what} {how}: every op lowered once");
            }
            for form in &lane.forms {
                let s = &form.schedule;
                for (q, l) in s.launches.iter().enumerate() {
                    let Launch::Quant { x, .. } = l else { continue };
                    let writers: Vec<usize> =
                        s.launches.iter().enumerate().filter(|&(i, w)| i != q && w.writes().contains(&x.as_str())).map(|(i, _)| i).collect();
                    let projections: Vec<usize> =
                        s.launches.iter().enumerate().filter(|(_, r)| r.projected_input() == Some(x.as_str())).map(|(i, _)| i).collect();
                    assert!(writers.iter().all(|&w| w < q), "{what} {}: `{x}` quantized at launch {q}, before its writes at {writers:?}", form.tag());
                    assert!(projections.iter().all(|&r| r > q), "{what} {}: `{x}` quantized at launch {q}, after its reads at {projections:?}", form.tag());
                }
            }
            if numerics == Q8MatKind::TwoPassInt8 && common::require_tool("nvcc") {
                nvcc(&lane.program.cu, &dir().join(format!("lane-batch-{}", t.name())));
            }
        }
    }
    let (spec, path) = checkpoint(Tiny::Llama, "batch-one");
    assert!(
        matches!(compile_batch(&spec, &path, &options(Q8MatKind::Int8Act), 1), Err(LaneError::Batch { batch: 1 })),
        "a batch of one is the decode step"
    );
}

/// A batch's projections sum each sequence's row as the decode step sums its one row: whatever the batch size — past
/// the GEMV's rows too, where a prefill chunk's projections take the tensor cores — no projection, gated pair, merged
/// projection or mixture of a batch step runs on the tensor cores' GEMM, whose sums run in another order; each is a
/// batch's (`Launch::is_batch`), and no projection of the decode step or a chunk is. Under the quantized numerics its
/// Q8_0 projections run on the tensor cores in their rows' own order (`Launch::batch_on_tensor_cores`, `lk_gemv_mma`);
/// the precise numerics keep the GEMV. A chunk past the GEMV's rows still takes the tensor cores' GEMM
/// (`a_wide_chunk_runs_its_projections_on_the_tensor_cores`).
#[test]
fn a_batchs_projections_sum_each_sequence_as_the_decode_step_does() {
    let projection = |l: &Launch| matches!(l, Launch::Gemv { .. } | Launch::GemmParts { .. } | Launch::GemvGlu { .. } | Launch::Moe { .. });
    for &t in ALL {
        let (spec, path) = checkpoint(t, "batch-rows");
        for numerics in NUMERICS {
            let what = format!("{} {numerics:?}", t.name());
            for rows in [4, 24, 64] {
                let lane = compile_batch(&spec, &path, &LaneOptions { prefill: 4, ..options(numerics) }, rows)
                    .unwrap_or_else(|e| panic!("{what} {rows}: {e}"));
                let s = &lane.batch().unwrap_or_else(|| panic!("{what} {rows}: the batch step")).schedule;
                let on_cores: Vec<&Launch> = s.launches.iter().flat_map(|l| l.parts()).filter(|l| projection(l) && l.on_tensor_cores(numerics)).collect();
                assert!(on_cores.is_empty(), "{what}, {rows} sequences: summed in the GEMM's order: {on_cores:?}");
                assert!(
                    s.launches.iter().flat_map(|l| l.parts()).filter(|l| projection(l)).all(|l| l.is_batch()),
                    "{what}, {rows} sequences: every projection a batch's"
                );
                for f in lane.forms.iter().filter(|f| f.kind != FormKind::Batch) {
                    assert!(!f.schedule.launches.iter().any(|l| l.is_batch()), "{what} {}: no projection of a step of one sequence is a batch's", f.tag());
                }
                // Under the quantized numerics the Q8_0 projections take the tensor cores in their rows' own order.
                let mma = s.launches.iter().filter(|l| l.batch_on_tensor_cores(numerics)).count();
                if numerics == Q8MatKind::Dequant {
                    assert_eq!(mma, 0, "{what}, {rows} sequences: the precise numerics keep the GEMV");
                } else {
                    assert!(mma > 0 && lane.program.cu.contains("lk_gemv_mma<"), "{what}, {rows} sequences: on the tensor cores by `lk_gemv_mma`");
                }
            }
        }
    }
}

/// A batch's projections over Q4_K and Q6_K rows take the tensor cores in their rows' own order (`lk_gemv_mma_blk`,
/// `Launch::batch_blk_format`) under one word a block: at 2 and at 16 sequences, every projection, merged projection and
/// gated pair of Llama's and Qwen3's medium shapes whose matrices are of the format, whose `k` is whole super-blocks and
/// whose parts are whole 64-output blocks reports the format, the program instantiates the kernel and sets its shared
/// memory before the step runs; past 16 sequences, under two words and under the precise numerics none does (the GEMV
/// keeps them); and no step of one sequence takes it.
#[test]
fn a_batchs_k_quant_projections_take_the_tensor_cores_in_their_rows_own_order() {
    use logicaffeine_compile::codegen_cuda::lane::kernels::{format_arg, gemv_mma_blk_smem_bytes, GEMV_MMA_BLK_OUTPUTS};
    let projection = |l: &Launch| matches!(l, Launch::Gemv { .. } | Launch::GemmParts { .. } | Launch::GemvGlu { .. });
    for ty in [GgmlType::Q4K, GgmlType::Q6K] {
        for t in [Tiny::Llama, Tiny::Qwen3] {
            let spec = t.medium(CTX);
            let path = dir().join(format!("batch-blk-{}-{}.gguf", t.name(), ty.name()));
            let matrices: BTreeSet<String> = tiny_models::tensors(&spec)
                .into_iter()
                .filter(|(n, dims)| dims.len() == 2 && dims[0] % 256 == 0 && !n.contains("norm") && !n.contains("token_embd"))
                .map(|(n, _)| n)
                .collect();
            tiny_models::write_checkpoint_as(&spec, &path, &|n| if matrices.contains(n) { ty } else { GgmlType::Q8_0 });
            let f = format_arg(ty);
            let whole = |l: &Launch| match l {
                Launch::Gemv { format, k, n, .. } => *format == ty && k % 256 == 0 && n % GEMV_MMA_BLK_OUTPUTS == 0,
                Launch::GemmParts { format, k, parts, .. } => *format == ty && k % 256 == 0 && parts.iter().all(|p| p.n % GEMV_MMA_BLK_OUTPUTS == 0),
                Launch::GemvGlu { formats, k, n, .. } => formats.iter().all(|x| *x == ty) && k % 256 == 0 && n % GEMV_MMA_BLK_OUTPUTS == 0,
                _ => false,
            };
            for numerics in [Q8MatKind::Int8Act, Q8MatKind::TwoPassInt8, Q8MatKind::Dequant] {
                for rows in [2, 16, 17] {
                    let what = format!("{} {} {numerics:?}, {rows} sequences", t.name(), ty.name());
                    let lane = compile_batch(&spec, &path, &options(numerics), rows).unwrap_or_else(|e| panic!("{what}: {e}"));
                    let s = &lane.batch().unwrap_or_else(|| panic!("{what}: the batch step")).schedule;
                    let eligible: Vec<&Launch> = s.launches.iter().flat_map(|l| l.parts()).filter(|l| projection(l) && whole(l)).collect();
                    let taken = eligible.iter().filter(|l| l.batch_blk_format(numerics) == Some(ty)).count();
                    let cu = &lane.program.cu;
                    if numerics == Q8MatKind::Int8Act && rows <= 16 {
                        assert!(!eligible.is_empty(), "{what}: projections of the format");
                        assert_eq!(taken, eligible.len(), "{what}: every projection of the format on the tensor cores: {eligible:?}");
                        assert!(cu.contains(&format!("lk_gemv_mma_blk<{f}><<<")), "{what}: launched by lk_gemv_mma_blk<{f}>");
                        let set = format!("cudaFuncSetAttribute(lk_gemv_mma_blk<{f}>, cudaFuncAttributeMaxDynamicSharedMemorySize, {}));", gemv_mma_blk_smem_bytes(f));
                        assert!(cu.contains(&set), "{what}: its shared memory set before the step: {set}");
                    } else {
                        assert_eq!(taken, 0, "{what}: the GEMV keeps them");
                        assert!(!cu.contains("lk_gemv_mma_blk<"), "{what}: no lk_gemv_mma_blk");
                    }
                    for form in lane.forms.iter().filter(|form| form.kind != FormKind::Batch) {
                        assert!(
                            !form.schedule.launches.iter().flat_map(|l| l.parts()).any(|l| l.batch_blk_format(numerics).is_some()),
                            "{what} {}: no step of one sequence takes it",
                            form.tag()
                        );
                    }
                }
            }
        }
    }
}

/// A batch's projections over 32-value blocks — Q4_0, Q4_1, Q5_0, Q5_1 — take the tensor cores in their rows' own
/// order (`lk_gemv_mma_blk`, `Launch::batch_blk_format`) under one word a block: at 2 and at 16 sequences, every
/// projection, merged projection and gated pair of Llama's and Qwen3's medium shapes whose matrices are of the format,
/// whose `k` is whole blocks and whose parts are whole 64-output blocks reports the format, the program instantiates the
/// kernel and sets its shared memory before the step runs; past 16 sequences, under two words and under the precise
/// numerics none does (the GEMV keeps them); and no step of one sequence takes it.
#[test]
fn a_batchs_32_value_block_projections_take_the_tensor_cores_in_their_rows_own_order() {
    use logicaffeine_compile::codegen_cuda::lane::kernels::{format_arg, gemv_mma_blk_smem_bytes, GEMV_MMA_BLK_OUTPUTS};
    let projection = |l: &Launch| matches!(l, Launch::Gemv { .. } | Launch::GemmParts { .. } | Launch::GemvGlu { .. });
    for ty in [GgmlType::Q4_0, GgmlType::Q4_1, GgmlType::Q5_0, GgmlType::Q5_1] {
        for t in [Tiny::Llama, Tiny::Qwen3] {
            let spec = t.medium(CTX);
            let path = dir().join(format!("batch-blk32-{}-{}.gguf", t.name(), ty.name()));
            let matrices: BTreeSet<String> = tiny_models::tensors(&spec)
                .into_iter()
                .filter(|(n, dims)| dims.len() == 2 && dims[0] % 32 == 0 && !n.contains("norm") && !n.contains("token_embd"))
                .map(|(n, _)| n)
                .collect();
            tiny_models::write_checkpoint_as(&spec, &path, &|n| if matrices.contains(n) { ty } else { GgmlType::Q8_0 });
            let f = format_arg(ty);
            let whole = |l: &Launch| match l {
                Launch::Gemv { format, k, n, .. } => *format == ty && k % 32 == 0 && n % GEMV_MMA_BLK_OUTPUTS == 0,
                Launch::GemmParts { format, k, parts, .. } => *format == ty && k % 32 == 0 && parts.iter().all(|p| p.n % GEMV_MMA_BLK_OUTPUTS == 0),
                Launch::GemvGlu { formats, k, n, .. } => formats.iter().all(|x| *x == ty) && k % 32 == 0 && n % GEMV_MMA_BLK_OUTPUTS == 0,
                _ => false,
            };
            for numerics in [Q8MatKind::Int8Act, Q8MatKind::TwoPassInt8, Q8MatKind::Dequant] {
                for rows in [2, 16, 17] {
                    let what = format!("{} {} {numerics:?}, {rows} sequences", t.name(), ty.name());
                    let lane = compile_batch(&spec, &path, &options(numerics), rows).unwrap_or_else(|e| panic!("{what}: {e}"));
                    let s = &lane.batch().unwrap_or_else(|| panic!("{what}: the batch step")).schedule;
                    let eligible: Vec<&Launch> = s.launches.iter().flat_map(|l| l.parts()).filter(|l| projection(l) && whole(l)).collect();
                    let taken = eligible.iter().filter(|l| l.batch_blk_format(numerics) == Some(ty)).count();
                    let cu = &lane.program.cu;
                    if numerics == Q8MatKind::Int8Act && rows <= 16 {
                        assert!(!eligible.is_empty(), "{what}: projections of the format");
                        assert_eq!(taken, eligible.len(), "{what}: every projection of the format on the tensor cores: {eligible:?}");
                        assert!(cu.contains(&format!("lk_gemv_mma_blk<{f}><<<")), "{what}: launched by lk_gemv_mma_blk<{f}>");
                        let set = format!("cudaFuncSetAttribute(lk_gemv_mma_blk<{f}>, cudaFuncAttributeMaxDynamicSharedMemorySize, {}));", gemv_mma_blk_smem_bytes(f));
                        assert!(cu.contains(&set), "{what}: its shared memory set before the step: {set}");
                    } else {
                        assert_eq!(taken, 0, "{what}: the GEMV keeps them");
                        assert!(!cu.contains("lk_gemv_mma_blk<"), "{what}: no lk_gemv_mma_blk");
                    }
                    for form in lane.forms.iter().filter(|form| form.kind != FormKind::Batch) {
                        assert!(
                            !form.schedule.launches.iter().flat_map(|l| l.parts()).any(|l| l.batch_blk_format(numerics).is_some()),
                            "{what} {}: no step of one sequence takes it",
                            form.tag()
                        );
                    }
                }
            }
        }
    }
}

/// A batch's block-format kernel holds its sums in registers (`lk_gemv_mma_blk`). In the lanes of Llama's and Qwen3's
/// medium shapes at 16 sequences in Q4_K and Q6_K, and of Llama's in Q4_0, Q4_1, Q5_0 and Q5_1, no instantiation keeps
/// an array in local memory (`__local_depot` in the PTX: a level of the closing tree left a loop put its 16 level-one
/// sums there, and the Q6_K head ran at a quarter of its speed), and those whose warp holds both leaves' sums — Q4_K's
/// and the 32-value blocks' — keep no stack frame: eight n-tiles of two leaves' sums fill the registers 512 threads a
/// block leave, and the 260 bytes Q4_K's spilled holding its part's outputs through the sums halved its speed.
#[test]
fn a_batchs_block_format_kernels_hold_their_sums_in_registers() {
    if !common::require_tool("nvcc") {
        return;
    }
    let mut programs: Vec<(String, PathBuf)> = Vec::new();
    let lanes = [GgmlType::Q4K, GgmlType::Q6K].into_iter().flat_map(|ty| [(ty, Tiny::Llama), (ty, Tiny::Qwen3)])
        .chain([GgmlType::Q4_0, GgmlType::Q4_1, GgmlType::Q5_0, GgmlType::Q5_1].into_iter().map(|ty| (ty, Tiny::Llama)));
    for (ty, t) in lanes {
        let spec = t.medium(CTX);
        let path = dir().join(format!("batch-blk-regs-{}-{}.gguf", t.name(), ty.name()));
        let matrices: BTreeSet<String> = tiny_models::tensors(&spec)
            .into_iter()
            .filter(|(n, dims)| dims.len() == 2 && dims[0] % if matches!(ty, GgmlType::Q4K | GgmlType::Q6K) { 256 } else { 32 } == 0 && !n.contains("norm") && !n.contains("token_embd"))
            .map(|(n, _)| n)
            .collect();
        tiny_models::write_checkpoint_as(&spec, &path, &|n| if matrices.contains(n) { ty } else { GgmlType::Q8_0 });
        let what = format!("{} {}", t.name(), ty.name());
        let lane = compile_batch(&spec, &path, &options(Q8MatKind::Int8Act), 16).unwrap_or_else(|e| panic!("{what}: {e}"));
        let src = dir().join(format!("lane-blk-regs-{}-{}.cu", t.name(), ty.name()));
        std::fs::write(&src, &lane.program.cu).unwrap();
        programs.push((what, src));
    }
    // Each program's PTX and ptxas's report of its kernels.
    let build = |src: &Path| -> (String, String) {
        let ptx = Command::new("nvcc").args(["-O3", "-std=c++17", "-arch=sm_86", "-ptx", "-o"]).arg(src.with_extension("ptx")).arg(src).output().expect("nvcc runs");
        assert!(ptx.status.success(), "{} compiles to PTX:\n{}", src.display(), String::from_utf8_lossy(&ptx.stderr));
        let obj = Command::new("nvcc").args(["-O3", "-std=c++17", "-arch=sm_86", "-Xptxas", "-v", "-c", "-o"]).arg(src.with_extension("o")).arg(src).output().expect("nvcc runs");
        assert!(obj.status.success(), "{} compiles:\n{}", src.display(), String::from_utf8_lossy(&obj.stderr));
        (std::fs::read_to_string(src.with_extension("ptx")).unwrap(), String::from_utf8_lossy(&obj.stderr).into_owned())
    };
    let compiled: Vec<(String, (String, String))> = std::thread::scope(|scope| {
        let workers: Vec<_> = programs.iter().map(|(what, src)| scope.spawn(move || (what.clone(), build(src)))).collect();
        workers.into_iter().map(|w| w.join().unwrap()).collect()
    });
    // (program, kernel, stack frame bytes, spill store bytes) of each instantiation; the kernels whose PTX keeps a depot.
    let mut frames: Vec<(String, String, u64, u64)> = Vec::new();
    let mut depots: Vec<(String, String)> = Vec::new();
    for (what, (ptx, report)) in &compiled {
        for entry in ptx.split(".entry ").skip(1) {
            let name = entry.split('(').next().unwrap_or("").trim();
            if name.contains("lk_gemv_mma_blk") && entry[..entry.find("\n}\n").unwrap_or(entry.len())].contains("__local_depot") {
                depots.push((what.clone(), name.to_string()));
            }
        }
        let mut kernel: Option<String> = None;
        for line in report.lines() {
            if let Some(at) = line.find("Function properties for ") {
                kernel = Some(line[at + "Function properties for ".len()..].trim().to_string());
            } else if let (Some(k), Some(at), Some(sp)) = (&kernel, line.find(" bytes stack frame, "), line.find(" bytes spill stores")) {
                let bytes = |s: &str| -> u64 { s.trim().rsplit(' ').next().and_then(|n| n.parse().ok()).expect("a count of bytes") };
                if k.contains("lk_gemv_mma_blk") {
                    frames.push((what.clone(), k.clone(), bytes(&line[..at]), bytes(&line[at + " bytes stack frame, ".len()..sp])));
                }
                kernel = None;
            }
        }
    }
    for f in [12, 14, 2, 3, 6, 7] {
        assert!(
            frames.iter().any(|(_, k, _, _)| k.contains(&format!("lk_gemv_mma_blkILi{f}E"))),
            "some lane instantiates lk_gemv_mma_blk<{f}>: {frames:?}"
        );
    }
    assert!(depots.is_empty(), "a batch's block-format kernel keeps an array in local memory: {depots:?}");
    let both = |k: &str| [12, 2, 3, 6, 7].iter().any(|f| k.contains(&format!("lk_gemv_mma_blkILi{f}E")));
    let framed: Vec<_> = frames.iter().filter(|(_, k, stack, spill)| both(k) && (*stack > 0 || *spill > 0)).collect();
    assert!(framed.is_empty(), "a batch kernel whose warp holds both leaves' sums holds them in its registers, with no stack frame: {framed:?}");
}

/// A batch's Q8_0 projections take the widest tile of the tensor cores their rows fill (`kernels::gemv_mma_shape`): up
/// to sixteen rows, four n-tiles of 8 outputs a block, a warp each; up to 32, eight n-tiles, a warp each over both
/// m-tiles; up to 64, sixteen n-tiles over all four m-tiles, a warp each two n-tiles over two m-tiles — a block of 128
/// outputs by 64 rows, whose activation rows each block stages once for four times the outputs of the narrowest tile,
/// each fragment a warp loads (an activation tile's, a matrix tile's) feeding two products. A batch past 64 rows takes
/// a launch of 64 and one of the rest. Each launch's template is its rows' tiles, its n-tiles, its warp's m-tiles and
/// its warp's n-tiles, and its shared memory past the default is set before the step runs.
#[test]
fn a_batchs_projections_take_the_widest_tile_their_rows_fill() {
    use logicaffeine_compile::codegen_cuda::lane::kernels::gemv_mma_smem_bytes;
    let (spec, path) = checkpoint(Tiny::Llama, "batch-tiles");
    for (numerics, q) in [(Q8MatKind::Int8Act, 1), (Q8MatKind::TwoPassInt8, 2)] {
        let cases: [(usize, &[(usize, usize, usize, usize)]); 7] = [
            (8, &[(1, 4, 1, 1)]),
            (16, &[(1, 4, 1, 1)]),
            (24, &[(2, 8, 2, 1)]),
            (32, &[(2, 8, 2, 1)]),
            (48, &[(4, 16, 2, 2)]),
            (64, &[(4, 16, 2, 2)]),
            (80, &[(4, 16, 2, 2), (1, 4, 1, 1)]),
        ];
        for (rows, shapes) in cases {
            let lane = compile_batch(&spec, &path, &options(numerics), rows).unwrap_or_else(|e| panic!("{rows} sequences: {e}"));
            let cu = &lane.program.cu;
            let launched: BTreeSet<&str> =
                cu.split("lk_gemv_mma<").skip(1).filter_map(|s| s.split_once('>').filter(|(_, rest)| rest.starts_with("<<<")).map(|(a, _)| a)).collect();
            let want: BTreeSet<String> = shapes.iter().map(|(mt, nt, mw, nw)| format!("{q}, {mt}, {nt}, {mw}, {nw}")).collect();
            assert_eq!(
                launched.iter().map(|s| s.to_string()).collect::<BTreeSet<_>>(),
                want,
                "{numerics:?}, {rows} sequences: every projection at the widest tile its rows fill"
            );
            for &(mt, nt, mw, nw) in shapes {
                let set = format!("cudaFuncSetAttribute(lk_gemv_mma<{q}, {mt}, {nt}, {mw}, {nw}>, cudaFuncAttributeMaxDynamicSharedMemorySize, {}));", gemv_mma_smem_bytes(q == 2, mt, nt));
                assert_eq!(
                    cu.contains(&set),
                    gemv_mma_smem_bytes(q == 2, mt, nt) > 48 * 1024,
                    "{numerics:?}, {rows} sequences: the tile {mt} by {nt} sets the shared memory its stages need past the default"
                );
            }
        }
    }
}

/// A decode step's projections of one activation in one format at one row a warp — each layer's query, key and value —
/// run as one GEMV over their output rows side by side (`gemv_parts`: a `Launch::GemmParts` of the step's one row,
/// `lk_gemv_parts`), each part written where its projection wrote it, under every numerics; held back, each projection
/// runs alone. The heads' epilogues (`qkv_head_epilogue`, which takes this launch into a fold) are held throughout. The
/// program apart computes what the merged one computes bit for bit
/// (`cuda_lane_lock::every_fusion_computes_what_its_launches_compute_apart`).
#[test]
fn the_decode_step_projects_one_activation_by_its_matrices_in_one_launch() {
    for t in [Tiny::Qwen3, Tiny::Llama, Tiny::LlamaTied] {
        let (spec, path) = checkpoint(t, "gemv-parts");
        for numerics in NUMERICS {
            let what = format!("{} {numerics:?}", t.name());
            let lane = compile_holding(&spec, &path, &options(numerics), &["qkv_head_epilogue"]).unwrap_or_else(|e| panic!("{what}: {e}"));
            let s = &lane.decode().schedule;
            assert_eq!(s.fired("gemv_parts"), 3, "{what}: one a layer: {:?}", s.fusions);
            for layer in 0..3 {
                let names = [format!("qr{layer}"), format!("kr{layer}"), format!("Vc{layer}")];
                assert!(
                    s.launches.iter().any(|l| matches!(l, Launch::GemmParts { m: 1, parts, .. }
                        if parts.iter().map(|p| p.y.buf.as_str()).eq(names.iter().map(String::as_str)))),
                    "{what}: layer {layer}'s query, key and value in one launch: {:?}",
                    s.launches
                );
                assert!(!s.launches.iter().any(|l| matches!(l, Launch::Gemv { y, .. } if names.contains(&y.buf))), "{what}: none alone");
            }
            assert!(lane.program.cu.contains("lk_gemv_parts<"), "{what}: by the GEMV in parts");
            let held = compile_holding(&spec, &path, &options(numerics), &["gemv_parts", "qkv_head_epilogue"]).unwrap_or_else(|e| panic!("{what} held: {e}"));
            let h = &held.decode().schedule;
            assert_eq!(h.fired("gemv_parts"), 0, "{what} held");
            assert!(!h.launches.iter().any(|l| matches!(l, Launch::GemmParts { .. })), "{what} held: each projection alone");
        }
    }
}

/// A chunk wider than the GEMV's sixteen rows multiplies a 16-bit float matrix's projections of one activation — each
/// layer's query, key and value — as one launch of the 16-bit GEMM in parts (a `Launch::GemmParts` of the chunk's rows,
/// `lk_gemm_h16`'s `G` 4, merged as the int8 tensor cores' products are), each part written where its projection writes
/// it alone, under one word (tiles of 64 outputs) and two (of 128); none of them alone. The launch in parts sums each
/// part as the part's own GEMM sums it (`cuda_lane_lock::a_sixteen_bit_gemm_in_parts_sums_each_part_as_its_own_gemm_sums_it`).
#[test]
fn a_sixteen_bit_chunk_projects_one_activation_by_its_matrices_in_one_launch() {
    let spec = Tiny::Llama.wide(CTX);
    let path = dir().join("bf16-chunk-parts.gguf");
    tiny_models::write_checkpoint_in(&spec, &path, GgmlType::Bf16);
    let Mixer::Attention(a) = &spec.layers[0].mixer else { panic!("Llama's layers attend") };
    let widths = [a.heads * a.head_dim, a.kv_heads * a.head_dim, a.kv_heads * a.value_dim];
    for (numerics, parted) in [(Q8MatKind::Int8Act, "lk_gemm_h16<30, 1, 4, 0, 64><<<"), (Q8MatKind::TwoPassInt8, "lk_gemm_h16<30, 2, 4><<<")] {
        let lane = compile(&spec, &path, &LaneOptions { ctx: 128, prefill: 64, ..options(numerics) }).unwrap_or_else(|e| panic!("{numerics:?}: {e}"));
        let wide: Vec<&_> = lane.forms.iter().filter(|f| f.rows > 16).collect();
        assert_eq!(wide.iter().map(|f| f.rows).collect::<Vec<_>>(), [64, 32], "{numerics:?}: two chunks past the GEMV's rows");
        for f in wide {
            let what = format!("{numerics:?}, the {}-row chunk", f.rows);
            let merged: Vec<_> = f
                .schedule
                .launches
                .iter()
                .filter_map(|l| match l {
                    Launch::GemmParts { m, parts, format: GgmlType::Bf16, .. } if *m == f.rows => Some(parts),
                    _ => None,
                })
                .collect();
            assert_eq!(merged.len(), spec.layers.len(), "{what}: one launch in parts a layer: {:?}", f.schedule.launches);
            for parts in merged {
                assert_eq!(parts.iter().map(|p| p.n).collect::<Vec<_>>(), widths, "{what}: the query's, the key's and the value's outputs side by side");
                for p in parts {
                    assert!(!f.schedule.launches.iter().any(|l| matches!(l, Launch::Gemv { y, .. } if y.buf == p.y.buf)), "{what}: {} only in parts", p.y.buf);
                }
            }
        }
        assert_eq!(lane.program.cu.matches(parted).count(), 2 * spec.layers.len(), "{numerics:?}: each wide chunk's layers by the 16-bit GEMM in parts");
        if common::require_tool("nvcc") {
            nvcc(&lane.program.cu, &dir().join(format!("lane-bf16-parts-{numerics:?}")));
        }
    }
}

/// A block-format GEMM on the int8 tensor cores whose every output is its sum as it is — a dense product or a launch in
/// parts with no bias and no epilogue step, into `f32` arrays — stores its sums plainly (`lk_gemm_blk`'s `PLAIN`, named
/// with `true` last and given its shared memory under that name), and every other keeps the general epilogue: Qwen3 at
/// four times the tiny width in Q8_0, whose chunks of 64 rows multiply unbiased, and gpt-oss's biased projections,
/// under both quantized numerics. The plain stores are the general epilogue's bits
/// (`cuda_lane_lock::a_block_gemm_whose_outputs_are_its_sums_stores_what_its_epilogue_stores`).
#[test]
fn a_block_gemm_stores_its_sums_plainly_where_its_outputs_are_them() {
    use logicaffeine_compile::codegen_cuda::lane::plan::DType;
    for t in [Tiny::Qwen3, Tiny::GptOss] {
        let (spec, path) = wide_checkpoint(t, "plain", &|_| GgmlType::Q8_0);
        for numerics in [Q8MatKind::Int8Act, Q8MatKind::TwoPassInt8] {
            let what = format!("{} {numerics:?}", t.name());
            let lane = compile(&spec, &path, &LaneOptions { ctx: 128, prefill: 64, ..options(numerics) }).unwrap_or_else(|e| panic!("{what}: {e}"));
            let cu = &lane.program.cu;
            let mut plain = 0;
            for f in &lane.forms {
                let f32 = |d: &Dst| d.unit.is_none() && f.schedule.buffers.get(&d.buf).is_some_and(|b| b.dtype == DType::F32);
                for l in &f.schedule.launches {
                    if !l.on_tensor_cores(numerics) {
                        continue;
                    }
                    plain += usize::from(match l {
                        Launch::Gemv { bias, epi, y, .. } => bias.is_none() && epi.is_none() && f32(y),
                        Launch::GemmParts { parts, .. } => parts.iter().all(|p| p.bias.is_none() && f32(&p.y)),
                        _ => false,
                    });
                }
            }
            // Each block GEMM's template arguments, launched or given its shared memory.
            let named = |tail: &str| -> usize {
                cu.match_indices("lk_gemm_blk<")
                    .filter(|(at, _)| {
                        let rest = &cu[*at..];
                        let close = rest.find('>').unwrap();
                        rest[..close].ends_with(", true") && rest[close..].starts_with(tail)
                    })
                    .count()
            };
            assert_eq!(named("><<<"), plain, "{what}: a plain launch for every product whose outputs are its sums");
            if t == Tiny::GptOss {
                assert_eq!(plain, 0, "{what}: every projection biased");
            } else {
                assert!(plain > 0, "{what}: the unbiased chunks' products stored plainly");
                assert!(named(">, cudaFuncAttributeMaxDynamicSharedMemorySize, ") > 0, "{what}: the plain kernel given its shared memory");
            }
            if common::require_tool("nvcc") {
                nvcc(cu, &dir().join(format!("lane-plain-{}-{numerics:?}", t.name())));
            }
        }
    }
}

/// A rotation its own launch turns its heads a thread a slot of a head where it norms none (`lk_rope_pairs`, a grid of
/// the heads' pairs and passed-through values) and a block a head where it norms them first (`lk_rope<true>`, the
/// norm's reduction): Llama's chunks of 64 and 32 rows turn their queries and keys by pairs, Qwen3's by heads, under
/// every numerics. The pairs turn each head as the head kernel does
/// (`cuda_lane_lock::the_rotation_by_pairs_turns_each_head_as_the_head_kernel_does`).
#[test]
fn an_unnormed_rotation_turns_a_thread_a_pair() {
    for (t, normed) in [(Tiny::Llama, false), (Tiny::Qwen3, true)] {
        let (spec, path) = checkpoint(t, "rope-pairs");
        for numerics in NUMERICS {
            let what = format!("{} {numerics:?}", t.name());
            let lane = compile(&spec, &path, &LaneOptions { ctx: 128, prefill: 64, ..options(numerics) }).unwrap_or_else(|e| panic!("{what}: {e}"));
            let cu = &lane.program.cu;
            let mut turned = 0;
            for f in &lane.forms {
                for l in &f.schedule.launches {
                    let Launch::Rope { norm, heads, hd, half, .. } = l else { continue };
                    assert_eq!(norm.is_some(), normed, "{what}, {} rows: {l:?}", f.rows);
                    turned += 1;
                    if !normed {
                        let grid = format!("lk_rope_pairs<<<{}, 256, 0, st>>>(", (heads * (hd - half)).div_ceil(256));
                        assert!(cu.contains(&grid), "{what}, {} rows: `{grid}`", f.rows);
                    }
                }
            }
            assert!(turned > 0, "{what}: some rotation its own launch");
            assert_eq!(cu.contains("lk_rope_pairs<<<"), !normed, "{what}: by pairs exactly where no head is normed");
            assert_eq!(cu.contains("lk_rope<true"), normed, "{what}: a normed head by its block");
            assert!(!cu.contains("lk_rope<false"), "{what}: no unnormed head by a block");
        }
    }
}

/// A norm over a chunk's rows takes blocks a multiprocessor holds six of, each playing the norm's own threads
/// (`lk_rmsnorm`'s and `lk_gather_norm`'s `VW`, `kernels::norm_block`): Llama, Gemma-3 (its sandwich pairs) and
/// Qwen3.5 at their tiny width and past 256 (2048, and Gemma-3-1B's 1152), prompted in chunks of 64 rows under every
/// numerics — every norm launched over more than one
/// row takes at most 256 threads, each of its warps `VW` of the norm's, its threads `VW` times over the norm's own
/// (`norm_threads` of the model's width) — `VW` past 1 exactly where the norm's own block passes 256 threads — and a
/// decode step's takes the norm's own block. Six blocks of 256 fit the 3060's
/// 1536 threads a multiprocessor where one of the norm's 1024 fits alone: a 512-row chunk's norm in three waves, not
/// eighteen (Gemma-3's 1152-wide norm 37.9 µs against 55.3, Qwen3.5's 1024-wide 31.7 against 47.1). A block playing the
/// norm's threads computes the norm's bits (`cuda_lane_lock::a_norm_in_blocks_playing_its_threads_computes_its_bits`).
#[test]
fn a_chunks_norm_takes_blocks_a_multiprocessor_holds_six_of() {
    use logicaffeine_compile::codegen_cuda::lane::kernels::norm_threads;
    // Every launch of `kernel` in `cu`: its rows, its threads, and the norm's warps a warp plays — its template's
    // `VW`, the argument past the kernel's others (`args` of them), 1 where the launch names none.
    let launches = |cu: &str, kernel: &str, args: usize| -> Vec<(usize, usize, usize)> {
        let open = format!("{kernel}<");
        cu.match_indices(&open)
            .filter_map(|(i, _)| {
                let (template, rest) = cu[i + open.len()..].split_once("><<<")?;
                let (rows, rest) = rest.split_once(", ")?;
                let threads = rest.split_once(',')?.0;
                let named: Vec<&str> = template.split(", ").collect();
                let vw = if named.len() > args { named[args].trim().parse().ok()? } else { 1 };
                Some((rows.parse().ok()?, threads.trim().parse().ok()?, vw))
            })
            .collect()
    };
    // Each architecture at its tiny width, whose norm's own block is under 256 threads, and past it: Llama's and
    // Qwen3.5's medium 2048 and Gemma-3-1B's own 1152.
    for (t, d) in [(Tiny::Llama, 0), (Tiny::Llama, 2048), (Tiny::Gemma3, 0), (Tiny::Gemma3, 1152), (Tiny::Qwen35, 0), (Tiny::Qwen35, 2048)] {
        let spec = if d == 0 {
            t.spec(CTX)
        } else {
            let mut spec = t.medium(CTX);
            spec.d = d;
            spec
        };
        let path = dir().join(format!("{}-norm-block-{}.gguf", t.name(), spec.d));
        tiny_models::write_checkpoint(&spec, &path);
        let norm = norm_threads(spec.d);
        for numerics in NUMERICS {
            let what = format!("{} at {} {numerics:?}", t.name(), spec.d);
            let lane = compile(&spec, &path, &LaneOptions { ctx: 128, prefill: 64, ..options(numerics) }).unwrap_or_else(|e| panic!("{what}: {e}"));
            let cu = &lane.program.cu;
            let (mut chunk, mut played) = (0, 0);
            // `lk_rmsnorm<ADD, SUM, OUTF, Q, PER, A, PRE, FN, VW>`, `lk_gather_norm<F, OUTF, Q, PER, A, VW>`.
            for (kernel, args) in [("lk_rmsnorm", 8), ("lk_gather_norm", 5)] {
                for (rows, threads, vw) in launches(cu, kernel, args) {
                    if rows > 1 {
                        assert!(threads <= 256, "{what}: a {rows}-row `{kernel}` in blocks of {threads}");
                        assert_eq!(threads * vw, norm, "{what}: a {rows}-row `{kernel}`'s blocks of {threads}, {vw} of the norm's warps a warp, play the norm's {norm} threads");
                        chunk += 1;
                        played += usize::from(vw > 1);
                    } else {
                        assert_eq!((threads, vw), (norm, 1), "{what}: the decode step's `{kernel}` takes the norm's own block");
                    }
                }
            }
            assert!(chunk > 0, "{what}: a chunk's norm launched");
            assert_eq!(played > 0, norm > 256, "{what}: a chunk's blocks play the norm's warps exactly where the norm's own block passes 256 threads");
        }
    }
}

/// A launch of the 16-bit GEMM takes tiles of 96 outputs under one word where they fill four waves of the chip's two
/// blocks a multiprocessor, and tiles of 64 below (`kernels::gemm_h16_launch_tile`): Llama at four times the tiny width
/// with a feed-forward of 8192, prompted in chunks up to 512 rows, multiplies the 512- and 256-row chunks' gated pairs
/// in tiles of 96 and every other 16-bit GEMM in tiles of 64 — each launch's grid its tiles, each kernel given the
/// shared memory its stages and staged sums take (`gemm_h16_smem_bytes`) — and under two words in tiles of 128. Tiles
/// of 96 sum each output as tiles of 64 do
/// (`cuda_lane_lock::a_sixteen_bit_gemm_in_tiles_of_96_sums_each_output_as_tiles_of_64_do`).
#[test]
fn a_sixteen_bit_launch_takes_tiles_of_96_where_they_fill_four_waves() {
    use logicaffeine_compile::codegen_cuda::lane::kernels::{gemm_h16_launch_tile, gemm_h16_smem_bytes, GEMM_H16_ROWS};
    let mut spec = Tiny::Llama.wide(CTX);
    for layer in &mut spec.layers {
        if let Ffn::Dense { dff, .. } = &mut layer.ffn {
            *dff = 8192;
        }
    }
    let path = dir().join("bf16-wide-ffn.gguf");
    tiny_models::write_checkpoint_in(&spec, &path, GgmlType::Bf16);
    let sms = CudaKnobs::default().chip.sms;
    for numerics in [Q8MatKind::Int8Act, Q8MatKind::TwoPassInt8] {
        let two = numerics == Q8MatKind::TwoPassInt8;
        let lane = compile(&spec, &path, &LaneOptions { ctx: 600, prefill: 512, ..options(numerics) }).unwrap_or_else(|e| panic!("{numerics:?}: {e}"));
        let cu = &lane.program.cu;
        let (mut pairs96, mut checked) = (0, 0);
        for f in lane.forms.iter().filter(|f| f.rows > 16) {
            for l in &f.schedule.launches {
                let (group, outputs, cols, rows) = match l {
                    Launch::Gemv { n, m, format: GgmlType::Bf16, .. } => (0, *n, *n, *m),
                    Launch::GemvGlu { n, m, formats: [GgmlType::Bf16, _], .. } => (5, 2 * *n, *n, *m),
                    _ => continue,
                };
                if !l.on_tensor_cores(numerics) {
                    continue;
                }
                let tn = gemm_h16_launch_tile(two, outputs, rows, sms);
                let want = match (two, group, f.rows) {
                    (true, ..) => 128,
                    (false, 5, 512 | 256) => 96,
                    _ => 64,
                };
                assert_eq!(tn, want, "{numerics:?}, the {}-row chunk: {l:?} in tiles of {want}", f.rows);
                pairs96 += usize::from(tn == 96 && group == 5);
                let kernel = match (group, tn) {
                    (0, 128) => "lk_gemm_h16<30, 2>".to_string(),
                    (5, 128) => "lk_gemm_h16<30, 2, 5, -1>".to_string(),
                    (0, _) => format!("lk_gemm_h16<30, 1, 0, 0, {tn}>"),
                    (_, _) => format!("lk_gemm_h16<30, 1, 5, -1, {tn}>"),
                };
                let gx = if group == 5 { cols.div_ceil(tn / 2) } else { cols.div_ceil(tn) };
                let launch = format!("{kernel}<<<dim3({gx}, {}), 256, {}, st>>>(", rows.div_ceil(GEMM_H16_ROWS), gemm_h16_smem_bytes(two, tn));
                assert!(cu.contains(&launch), "{numerics:?}, the {}-row chunk: `{launch}`", f.rows);
                assert!(
                    cu.contains(&format!("CK(cudaFuncSetAttribute({kernel}, cudaFuncAttributeMaxDynamicSharedMemorySize, {}));", gemm_h16_smem_bytes(two, tn))),
                    "{numerics:?}: {kernel} given its shared memory"
                );
                checked += 1;
            }
        }
        assert!(checked > 0, "{numerics:?}: the wide chunks' 16-bit GEMMs checked");
        assert_eq!(pairs96, if two { 0 } else { 2 * spec.layers.len() }, "{numerics:?}: the 512- and 256-row chunks' pairs in tiles of 96");
        if common::require_tool("nvcc") {
            nvcc(cu, &dir().join(format!("lane-bf16-tiles96-{numerics:?}")));
        }
    }
}

/// A recurrent layer's six projections of its activation — the delta rule's query, key, value and output gate and its
/// two gates, kept in Q8_0 as the real Qwen3.5 checkpoints keep them — run as one launch of six parts (`gemv_parts`, as
/// many as `kernels::GEMM_PARTS` takes), at the decode step and in a batch of 64 on the tensor cores, under every
/// numerics; none of them alone. The kernels' parts hold as many as the planner merges (`LK_PARTS`). The layer's input
/// epilogue (`gdn_input_epilogue`, which takes the six into its launch with the convolutions and gates) is held
/// throughout. The program apart computes what the merged one computes bit for bit
/// (`cuda_lane_lock::every_fusion_computes_what_its_launches_compute_apart`).
#[test]
fn a_recurrent_layers_six_projections_run_as_one_launch() {
    use logicaffeine_compile::codegen_cuda::lane::kernels::{Kernel, GEMM_PARTS};
    assert!(Kernel::Blocks.source().contains(&format!("#define LK_PARTS {GEMM_PARTS}\n")), "the kernels' parts are the planner's");
    let spec = Tiny::Qwen35.spec(CTX);
    let path = dir().join("qwen35-six-parts.gguf");
    tiny_models::write_checkpoint_as(&spec, &path, &|t| {
        let quantized = tiny_models::tensors(&spec).iter().any(|(n, dims)| n == t && dims.len() == 2 && dims[0] % 32 == 0 && !n.contains("norm"));
        if quantized && !t.contains("ssm_conv") {
            GgmlType::Q8_0
        } else {
            GgmlType::F32
        }
    });
    let recurrent = spec.layers.iter().filter(|l| matches!(l.mixer, Mixer::GatedDelta(_))).count();
    assert!(recurrent > 0, "a hybrid with recurrent layers");
    for numerics in NUMERICS {
        let lane = compile_batch_holding(&spec, &path, &options(numerics), 64, &["gdn_input_epilogue"]).unwrap_or_else(|e| panic!("{numerics:?}: {e}"));
        for (what, s) in [("decode", &lane.decode().schedule), ("batch", &lane.batch().unwrap().schedule)] {
            let six: Vec<&Launch> = s.launches.iter().filter(|l| matches!(l, Launch::GemmParts { parts, .. } if parts.len() == 6)).collect();
            assert_eq!(six.len(), recurrent, "{what} {numerics:?}: each recurrent layer's six projections in one launch: {:?}", s.launches);
            for l in six {
                let Launch::GemmParts { parts, .. } = l else { unreachable!() };
                for p in parts {
                    assert!(
                        !s.launches.iter().any(|o| matches!(o, Launch::Gemv { y, .. } if y.buf == p.y.buf)),
                        "{what} {numerics:?}: {} only in the merged launch",
                        p.y.buf
                    );
                }
            }
        }
    }
}

/// A step's rotations of its query heads and key heads by one table at one position — each normed first where the model
/// norms them — run as one launch over both tensors' heads (`rope_parts`: a `Launch::RopeParts`, `lk_rope_parts`), the
/// query's heads first and the key's written into the cache's row as its own rotation wrote them, under every numerics;
/// held back, each rotation is a launch of its own. A prefill chunk's rotations stay apart. The heads' epilogues
/// (`qkv_head_epilogue`, which turn the heads in the projections' launch) are held throughout. The program apart computes
/// what the paired one computes bit for bit (`cuda_lane_lock::every_fusion_computes_what_its_launches_compute_apart`).
#[test]
fn the_step_rotates_its_query_and_key_heads_in_one_launch() {
    for t in [Tiny::Qwen3, Tiny::Llama, Tiny::Gemma3, Tiny::Qwen35] {
        let (spec, path) = checkpoint(t, "rope-parts");
        for numerics in NUMERICS {
            let what = format!("{} {numerics:?}", t.name());
            let lane = compile_holding(&spec, &path, &LaneOptions { prefill: 4, ..options(numerics) }, &["qkv_head_epilogue"]).unwrap_or_else(|e| panic!("{what}: {e}"));
            let held = compile_holding(&spec, &path, &options(numerics), &["rope_parts", "qkv_head_epilogue"]).unwrap_or_else(|e| panic!("{what} held: {e}"));
            let alone = held.decode().schedule.launches.iter().filter(|l| matches!(l, Launch::Rope { .. })).count();
            assert!(alone > 0 && alone % 2 == 0, "{what} held: a query's and a key's rotation an attention layer: {alone}");
            assert_eq!(held.decode().schedule.fired("rope_parts"), 0, "{what} held");
            let s = &lane.decode().schedule;
            assert_eq!(s.fired("rope_parts"), alone / 2, "{what}: one an attention layer: {:?}", s.fusions);
            assert!(!s.launches.iter().any(|l| matches!(l, Launch::Rope { .. })), "{what}: no rotation alone");
            for l in &s.launches {
                if let Launch::RopeParts { sides, .. } = l {
                    assert_eq!(sides.len(), 2, "{what}: the query's and the key's");
                    assert!(sides[0].y.row.is_none() && sides[1].y.row.is_some(), "{what}: the query into its own buffer, the key into the cache's row: {sides:?}");
                    assert_eq!(sides[0].norm.is_some(), sides[1].norm.is_some(), "{what}: normed alike");
                }
            }
            assert!(lane.program.cu.contains("lk_rope_parts<"), "{what}: by the rotations in parts");
            let chunk = lane.chunk(4).unwrap_or_else(|| panic!("{what}: the chunk of 4")).schedule.clone();
            assert_eq!(chunk.fired("rope_parts"), 0, "{what}: a chunk's rotations over its rows stay apart");
        }
    }
}

/// A recurrent layer's two gates and the two `f32` projections of its activation they read — its write strength's and
/// its forget gate's — run as one launch (`gate_projections`: a `Launch::GateProj`, `lk_gate_proj`), each head's dots
/// summed as its own projections sum them, under every numerics; held back, the projections and the gates run apart.
/// A prefill chunk's projections over its rows stay apart. The layer's input epilogue (`gdn_input_epilogue`, which takes
/// the gates into its projections' launch) is held throughout. The program apart computes what the merged one computes
/// bit for bit (`cuda_lane_lock::every_fusion_computes_what_its_launches_compute_apart`).
#[test]
fn the_hybrid_step_takes_its_gates_from_its_activation_in_one_launch() {
    let (spec, path) = checkpoint(Tiny::Qwen35, "gate-proj");
    for numerics in NUMERICS {
        let what = format!("qwen35 {numerics:?}");
        let lane = compile_holding(&spec, &path, &LaneOptions { prefill: 4, ..options(numerics) }, &["gdn_input_epilogue"]).unwrap_or_else(|e| panic!("{what}: {e}"));
        let s = &lane.decode().schedule;
        assert_eq!(s.fired("gate_projections"), 2, "{what}: one a recurrent layer: {:?}", s.fusions);
        assert_eq!(s.launches.iter().filter(|l| matches!(l, Launch::GateProj { .. })).count(), 2, "{what}: a launch a recurrent layer");
        assert!(!s.launches.iter().any(|l| matches!(l, Launch::GemvF32 { .. } | Launch::Gates { .. })), "{what}: no projection or gates apart");
        assert!(lane.program.cu.contains("lk_gate_proj<<<"), "{what}: by the gates from the activation");
        let chunk = lane.chunk(4).unwrap_or_else(|| panic!("{what}: the chunk of 4")).schedule.clone();
        assert_eq!(chunk.fired("gate_projections"), 0, "{what}: a chunk's projections over its rows stay apart");
        let held = compile_holding(&spec, &path, &options(numerics), &["gate_projections", "gdn_input_epilogue"]).unwrap_or_else(|e| panic!("{what} held: {e}"));
        let h = &held.decode().schedule;
        assert_eq!(h.fired("gate_projections"), 0, "{what} held");
        assert_eq!(h.launches.iter().filter(|l| matches!(l, Launch::GemvF32 { .. })).count(), 4, "{what} held: two a recurrent layer");
        assert_eq!(h.launches.iter().filter(|l| matches!(l, Launch::Gates { .. })).count(), 2, "{what} held: the gates apart");
    }
}

/// The `recurrent_state` knob stores a gated-delta layer's memory in bf16 between steps: at `StatePrecision::Bf16` every
/// memory the decode step, its chunks and its batch carry is a `DType::Bf16` buffer the program declares as
/// `__nv_bfloat16`, and every delta-rule launch reads and writes it as such; at the default every one stays `f32`. The
/// pool the plan sizes shrinks by exactly what the program's own allocations of those memories save, two bytes an
/// element, so the program carves the whole pool it asks for.
#[test]
fn a_bf16_recurrent_state_is_carried_in_bf16() {
    use logicaffeine_compile::codegen_cuda::lane::plan::DType;
    use logicaffeine_compile::codegen_cuda::StatePrecision;
    let (spec, path) = checkpoint(Tiny::Qwen35, "state-bf16");
    for numerics in NUMERICS {
        let mut fixed = Vec::new();
        let mut saved = 0;
        for state in [StatePrecision::F32, StatePrecision::Bf16] {
            let what = format!("{numerics:?} {state:?}");
            let opts = LaneOptions { prefill: 4, knobs: CudaKnobs { recurrent_state: state, ..options(numerics).knobs }, ..options(numerics) };
            let lane = compile_batch(&spec, &path, &opts, 4).unwrap_or_else(|e| panic!("{what}: {e}"));
            let mut memories = BTreeSet::new();
            for f in &lane.forms {
                let s = &f.schedule;
                for l in &s.launches {
                    if let Launch::Delta { st, .. } = l {
                        memories.insert(st.clone());
                        let want = if state == StatePrecision::Bf16 { DType::Bf16 } else { DType::F32 };
                        assert_eq!(s.buffers[st].dtype, want, "{what} {}: the memory `{st}`", f.tag());
                    }
                }
            }
            assert!(!memories.is_empty(), "{what}: the step carries gated-delta memories");
            let cu = &lane.program.cu;
            let delta = cu.matches("lk_delta_step<").count() + cu.matches("lk_delta_state<").count();
            assert!(delta > 0, "{what}: the delta rule launches");
            let launched_bf16 = cu.matches(", __nv_bfloat16><<<").count();
            // A memory's own array, declared once under its form's prefix (`t_` for one every form shares).
            let arrays: Vec<&str> =
                cu.lines().filter_map(|l| l.trim().strip_prefix("__nv_bfloat16* ")?.strip_suffix(" = nullptr;")).collect();
            let declared: Vec<&str> =
                arrays.iter().filter_map(|a| a.strip_prefix('t')?.split_once('_').map(|(_, name)| name)).collect();
            assert_eq!(declared.len(), arrays.len(), "{what}: every bf16 array is a form's: {arrays:?}");
            if state == StatePrecision::Bf16 {
                let mut names: Vec<&str> = declared.clone();
                names.sort_unstable();
                names.dedup();
                assert_eq!(names, memories.iter().map(String::as_str).collect::<Vec<_>>(), "{what}: every memory declared in bf16, and no other array");
                assert_eq!(launched_bf16, delta, "{what}: every delta-rule launch reads its memory in bf16");
                for a in &arrays {
                    let at = format!("lk_malloc(&{a}, ");
                    let mut lines = cu.lines().filter_map(|l| Some(&l[l.find(&at)? + at.len()..]));
                    let len: usize = lines
                        .next()
                        .and_then(|rest| rest.split_once("UL * 2);"))
                        .and_then(|(len, _)| len.parse().ok())
                        .unwrap_or_else(|| panic!("{what}: `{a}` allocated at two bytes an element"));
                    assert!(lines.next().is_none(), "{what}: `{a}` allocated once");
                    saved += pooled(len * 4) - pooled(len * 2);
                }
            } else {
                assert_eq!((declared.len(), launched_bf16), (0, 0), "{what}: the default keeps every memory f32");
            }
            fixed.push(lane.plan.fixed);
        }
        assert!(saved > 0, "{numerics:?}: the memories take less in bf16");
        assert_eq!(
            fixed[0] - fixed[1],
            saved,
            "{numerics:?}: the pool shrinks by what the program's bf16 memories save, and by nothing else"
        );
    }
}

/// The `cache_precision` knob stores every attention's cache in binary16. At `CachePrecision::F16` every array an
/// attention of any form reads its keys or values from (followed through the views into it to the carried array they
/// view) is a `DType::F16` buffer in every form that holds it, and so is every view into one; no other array is; the
/// program declares each cache `__half`, and nothing else; and the pool the plan sizes shrinks by exactly what the
/// program's own allocations of those caches save, two bytes an element. At the default every array stays `f32`.
/// Every architecture, under every numerics, with the decode step, its chunks and its batch.
#[test]
fn an_f16_cache_is_carried_in_f16() {
    use logicaffeine_compile::codegen_cuda::lane::plan::DType;
    use logicaffeine_compile::codegen_cuda::CachePrecision;
    let mangled = |name: &str| -> String { name.chars().map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' }).collect() };
    for &t in ALL {
        let (spec, path) = checkpoint(t, "cache-f16");
        for numerics in NUMERICS {
            let mut fixed = Vec::new();
            let mut saved = 0;
            for cache in [CachePrecision::F32, CachePrecision::F16] {
                let what = format!("{} {numerics:?} {cache:?}", t.name());
                let opts = LaneOptions { prefill: 4, knobs: CudaKnobs { cache_precision: cache, ..options(numerics).knobs }, ..options(numerics) };
                let lane = compile_batch(&spec, &path, &opts, 4).unwrap_or_else(|e| panic!("{what}: {e}"));
                let root = |s: &Schedule, name: &str| -> String {
                    let mut n = name.to_string();
                    while let Some(Fill::Alias { of, .. }) = s.buffers.get(&n).map(|b| &b.fill) {
                        n = of.clone();
                    }
                    n
                };
                let mut caches = BTreeSet::new();
                for f in &lane.forms {
                    for l in &f.schedule.launches {
                        if let Launch::Attention { kc, vc, .. } = l {
                            for c in [kc, vc] {
                                let r = root(&f.schedule, c);
                                if f.schedule.buffers[&r].fill == Fill::Zero {
                                    caches.insert(r);
                                }
                            }
                        }
                    }
                }
                assert!(!caches.is_empty(), "{what}: the step attends over a cache");
                let want = if cache == CachePrecision::F16 { DType::F16 } else { DType::F32 };
                for f in &lane.forms {
                    let s = &f.schedule;
                    for (name, b) in &s.buffers {
                        if caches.contains(&root(s, name)) {
                            assert_eq!(b.dtype, want, "{what} {}: the cache `{name}`", f.tag());
                        } else {
                            assert_ne!(b.dtype, DType::F16, "{what} {}: `{name}` is no cache", f.tag());
                        }
                    }
                }
                let cu = &lane.program.cu;
                let arrays: Vec<&str> = cu.lines().filter_map(|l| l.trim().strip_prefix("__half* ")?.strip_suffix(" = nullptr;")).collect();
                let declared: BTreeSet<&str> =
                    arrays.iter().filter_map(|a| a.strip_prefix('t')?.split_once('_').map(|(_, name)| name)).collect();
                if cache == CachePrecision::F16 {
                    assert_eq!(
                        declared.iter().map(|d| d.to_string()).collect::<BTreeSet<_>>(),
                        caches.iter().map(|c| mangled(c)).collect::<BTreeSet<_>>(),
                        "{what}: every cache declared in binary16, and no other array"
                    );
                    for a in &arrays {
                        let at = format!("lk_malloc(&{a}, ");
                        let mut lines = cu.lines().filter_map(|l| Some(&l[l.find(&at)? + at.len()..]));
                        let len: usize = lines
                            .next()
                            .and_then(|rest| rest.split_once("UL * 2);"))
                            .and_then(|(len, _)| len.parse().ok())
                            .unwrap_or_else(|| panic!("{what}: `{a}` allocated at two bytes an element"));
                        assert!(lines.next().is_none(), "{what}: `{a}` allocated once");
                        saved += pooled(len * 4) - pooled(len * 2);
                    }
                } else {
                    assert!(arrays.is_empty(), "{what}: the default keeps every cache f32: {arrays:?}");
                }
                fixed.push(lane.plan.fixed);
            }
            assert!(saved > 0, "{} {numerics:?}: the caches take less in binary16", t.name());
            assert_eq!(fixed[0] - fixed[1], saved, "{} {numerics:?}: the pool shrinks by what the program's binary16 caches save, and by nothing else", t.name());
        }
    }
}

/// A batch's narrow projection of long rows takes a block of 64 outputs (`kernels::gemv_mma_launch`): at 64 sequences
/// a down projection of 1024 outputs and 4096 values a row, whose tiles of 128 outputs would leave most of the chip
/// idle, launches eight n-tiles over all 64 rows — a warp each two n-tiles over two m-tiles, twice the blocks, every
/// row's fragments staged once — where the 32-row tile once took the rows in two slabs; the wider projections keep
/// the block of 128 outputs, and every launch's shared memory past the default is set.
#[test]
fn a_narrow_projection_of_long_rows_takes_a_block_of_64_outputs() {
    use logicaffeine_compile::codegen_cuda::lane::kernels::{gemv_mma_launch, gemv_mma_smem_bytes};
    let mut spec = Tiny::Llama.medium(CTX);
    spec.d = 1024;
    for layer in &mut spec.layers {
        if let Ffn::Dense { dff, .. } = &mut layer.ffn {
            *dff = 4096;
        }
    }
    let (spec, path) = sized_checkpoint(spec, "llama-narrow-long", &|_| GgmlType::Q8_0);
    let sms = options(Q8MatKind::Int8Act).knobs.chip.sms;
    assert_eq!(gemv_mma_launch(64, 1024, 4096, sms), (4, 8, 2, 2), "the rule narrows the down projection's block");
    assert_eq!(gemv_mma_launch(64, 4096, 1024, sms), (4, 16, 2, 2), "and keeps the wider projections' block");
    for (numerics, q) in [(Q8MatKind::Int8Act, 1), (Q8MatKind::TwoPassInt8, 2)] {
        let lane = compile_batch(&spec, &path, &options(numerics), 64).unwrap_or_else(|e| panic!("{numerics:?}: {e}"));
        let cu = &lane.program.cu;
        let down = format!("lk_gemv_mma<{q}, 4, 8, 2, 2><<<{}, ", 1024 / (8 * 8));
        assert_eq!(cu.matches(&down).count(), spec.layers.len(), "{numerics:?}: each layer's down projection in blocks of 64 outputs");
        assert!(cu.contains(&format!("lk_gemv_mma<{q}, 4, 16, 2, 2><<<")), "{numerics:?}: the wider projections in blocks of 128");
        let grids: Vec<&str> = cu
            .split("lk_gemv_mma<")
            .skip(1)
            .filter_map(|s| s.split_once('>').and_then(|(_, rest)| rest.strip_prefix("<<<")))
            .map(|g| g.split_once(", ").map_or(g, |(grid, _)| grid))
            .collect();
        assert!(!grids.is_empty() && grids.iter().all(|g| !g.starts_with("dim3(")), "{numerics:?}: no projection in slabs: {grids:?}");
        for (mt, nt, mw, nw) in [(4, 8, 2, 2), (4, 16, 2, 2)] {
            let set = format!("cudaFuncSetAttribute(lk_gemv_mma<{q}, {mt}, {nt}, {mw}, {nw}>, cudaFuncAttributeMaxDynamicSharedMemorySize");
            assert_eq!(cu.contains(&set), gemv_mma_smem_bytes(q == 2, mt, nt) > 48 * 1024, "{numerics:?}: the tile {mt} by {nt}'s shared memory");
        }
    }
}

/// An attention whose chunks take a tile of keys at a time, in a launch of more blocks than the chip holds at two a
/// multiprocessor — a batch's sequences, whose blocks come in waves — holds its registers to three blocks a
/// multiprocessor (`lk_attention`'s `MB`, `kernels::attn_blocks_a_multiprocessor`) under the `f32` sums: occupancy,
/// not a block's latency, bounds a wave there. A decode step's attention, under one wave, keeps its registers, and so
/// does every attention summing in `f64`.
#[test]
fn an_attention_in_waves_holds_three_blocks_a_multiprocessor() {
    let (spec, path) = checkpoint(Tiny::Llama, "attn-waves");
    for numerics in NUMERICS {
        let lane = compile_batch(&spec, &path, &options(numerics), 64).unwrap_or_else(|e| panic!("{numerics:?}: {e}"));
        let cu = &lane.program.cu;
        let launches: Vec<&str> = cu.split("lk_attention<").skip(1).filter_map(|s| s.split_once('>').filter(|(_, r)| r.starts_with("<<<")).map(|(a, _)| a)).collect();
        assert!(!launches.is_empty(), "{numerics:?}: attentions launched");
        let three = launches.iter().filter(|a| a.split(", ").nth(3) == Some("3")).count();
        if numerics == Q8MatKind::Dequant {
            assert_eq!(three, 0, "the f64 sums keep their registers: {launches:?}");
        } else {
            let batch = lane.batch().unwrap().schedule.launches.iter().filter(|l| matches!(l, Launch::Attention { .. })).count();
            assert_eq!(three, batch, "{numerics:?}: the batch's attentions, in waves, three a multiprocessor: {launches:?}");
            let decode = lane.decode().schedule.launches.iter().filter(|l| matches!(l, Launch::Attention { .. })).count();
            assert_eq!(launches.len() - three, decode, "{numerics:?}: the decode step's keep their registers: {launches:?}");
            assert!(cu.contains("__launch_bounds__(256, MB) lk_attention("), "{numerics:?}: the bound is the launch's");
        }
    }
}

/// An attention whose blocks take their chunks warp-local (`kernels::attn_warp_heads`: at most eight heads a block, 64,
/// 128 or 256 wide, the values as wide, a lane's share within its registers) keeps two blocks a multiprocessor in waves
/// too (`lk_attention`'s `MB`, `kernels::attn_blocks_a_multiprocessor`): its runs keep their queries and weighed values
/// in registers, which a third block's share spills. Llama's and Qwen3's batch of 64 at four times their width (heads 64
/// wide, two a block) under the `f32` sums: every attention a block of warp-local heads, some of the batch's grids past
/// two blocks a multiprocessor, and every attention launched at two.
#[test]
fn a_warp_local_attention_in_waves_keeps_two_blocks_a_multiprocessor() {
    use logicaffeine_compile::codegen_cuda::lane::kernels::{accumulator, attn_warp_heads, mode};
    let sms = CudaKnobs::default().chip.sms;
    for t in [Tiny::Llama, Tiny::Qwen3] {
        let (spec, path) = wide_checkpoint(t, "attn-waves-warps", &|_| GgmlType::Q4K);
        for numerics in [Q8MatKind::Int8Act, Q8MatKind::TwoPassInt8] {
            let what = format!("{} wide {numerics:?}", t.name());
            let lane = compile_batch(&spec, &path, &options(numerics), 64).unwrap_or_else(|e| panic!("{what}: {e}"));
            let attentions: Vec<&Launch> = lane
                .forms
                .iter()
                .flat_map(|f| f.schedule.launches.iter())
                .filter(|l| matches!(l, Launch::Attention { shared: None, .. }) && !l.attends_in_tiles(numerics))
                .collect();
            assert!(!attentions.is_empty(), "{what}: attentions scheduled");
            let mut waves = 0;
            for l in &attentions {
                let Launch::Attention { heads, hd, vd, rows, per_block, splits, parallel, .. } = l else { unreachable!() };
                assert!(attn_warp_heads(*per_block, *hd, accumulator(mode(numerics)).0) && vd == hd, "{what}: {per_block} heads {hd} wide a block take their chunks warp-local");
                waves += usize::from(heads / per_block * rows * if *parallel { *splits } else { 1 } > 2 * sms);
            }
            assert!(waves > 0, "{what}: some attention's grid is past two blocks a multiprocessor");
            let cu = &lane.program.cu;
            let launches: Vec<&str> = cu.split("lk_attention<").skip(1).filter_map(|s| s.split_once('>').filter(|(_, r)| r.starts_with("<<<")).map(|(a, _)| a)).collect();
            assert!(!launches.is_empty(), "{what}: attentions launched");
            for a in &launches {
                assert_eq!(a.split(", ").nth(3), Some("2"), "{what}: `lk_attention<{a}>` keeps two blocks a multiprocessor");
            }
        }
    }
}

/// A row an attention block takes whole — a batch's sequence — and whose chunks' partials it holds weighs its chunks
/// side by side, `kernels::attn_side_chunks` of them at a time: as many as its threads' value slots hold, each chunk in
/// the operations it takes alone (`lk_attn_side`), so the sequence still attends as the decode step's blocks attend
/// it. The launch names that many as its last argument and takes the shared memory they need — its queries, a tile of
/// scores for each, every chunk's sums and its partials; a row whose partials it does not hold names none. Llama's,
/// Gemma's and Qwen3's batch of 32 over a cache of 256 positions, under every numerics; some launch weighs several.
#[test]
fn a_row_taken_whole_weighs_its_chunks_side_by_side() {
    use logicaffeine_compile::codegen_cuda::lane::kernels::{accumulator, attn_held_bytes, attn_holds, attn_side_bytes, attn_side_chunks, attn_smem_bytes, mode};
    const LONG: usize = 256;
    let mut several = false;
    for t in [Tiny::Llama, Tiny::Gemma3, Tiny::Qwen3] {
        let spec = t.spec(LONG);
        let path = dir().join(format!("{}-side.gguf", t.name()));
        tiny_models::write_checkpoint(&spec, &path);
        for numerics in NUMERICS {
            let what = format!("{} {numerics:?}", t.name());
            let lane = compile_batch(&spec, &path, &LaneOptions { ctx: LONG, ..options(numerics) }, 32).unwrap_or_else(|e| panic!("{what}: {e}"));
            let acc = accumulator(mode(numerics)).0;
            let want: Vec<(usize, usize)> = lane
                .batch()
                .unwrap()
                .schedule
                .launches
                .iter()
                .filter_map(|l| match l {
                    Launch::Attention { hd, vd, per_block, splits, parallel: false, shared: None, .. } if !l.attends_in_tiles(numerics) => {
                        let side = if attn_holds(*per_block, *vd, *splits, acc) { attn_side_chunks(*vd, *splits) } else { 0 };
                        let held = if side > 0 { attn_held_bytes(*per_block, *vd, *splits, acc) + attn_side_bytes(*per_block, side, acc) } else { 0 };
                        Some((attn_smem_bytes(*per_block, *hd, *splits, acc) + held, side))
                    }
                    _ => None,
                })
                .collect();
            assert!(!want.is_empty(), "{what}: the batch attends");
            // The batch's attentions as the program launches them: their shared memory, and their last argument.
            let got: Vec<(usize, usize)> = lane
                .program
                .cu
                .lines()
                .map(str::trim)
                .filter(|l| l.starts_with("lk_attention<") && l.contains("tb32_"))
                .map(|l| {
                    let smem = l.split_once("), 256, ").and_then(|(_, r)| r.split_once(", st>>>")).map(|(m, _)| m.parse().expect("the shared memory"));
                    let last = l.strip_suffix(");").and_then(|a| a.rsplit(", ").next()).map(|a| a.parse().expect("the last argument"));
                    (smem.unwrap_or_else(|| panic!("{what}: `{l}`")), last.unwrap_or_else(|| panic!("{what}: `{l}`")))
                })
                .collect();
            assert_eq!(got, want, "{what}: each launch's shared memory and the chunks it weighs side by side");
            several |= want.iter().any(|&(_, side)| side > 1);
        }
    }
    assert!(several, "some batch weighs several chunks side by side");
}

/// The lane sizes a warp-local attention block's shared memory as its kernel lays it out: for one to sixteen heads 4 to
/// 576 wide summing in `f32` and in `f64`, `kernels::attn_lane_values`, `attn_warp_heads`, `attn_warp_pass` and
/// `attn_warp_slots` give what the kernel's `lk_attn_lane_values`, `lk_attn_warp_heads`, `lk_attn_warp_pass` and
/// `lk_attn_warp_slots` give, evaluated by nvcc's host compiler from the attention's own source — and some of those
/// shapes take their chunks warp-local, some in a pass of fewer heads than the block's, some (their lanes' share past
/// its registers) a tile of keys at a time.
#[test]
fn a_warp_local_attention_blocks_shared_memory_is_what_its_kernel_lays_out() {
    use logicaffeine_compile::codegen_cuda::lane::kernels::{attn_lane_values, attn_warp_heads, attn_warp_pass, attn_warp_slots, Kernel, PRELUDE};
    if !common::require_tool("nvcc") {
        return;
    }
    let program = format!(
        "#include <cstdio>\n#include <cuda_runtime.h>\n#include <cuda_fp16.h>\n{PRELUDE}{}\n\
         int main(){{\n  for(int acc = 4; acc <= 8; acc += 4)\n    for(int g = 1; g <= 16; ++g)\n      for(int hd = 4; hd <= 576; hd += 4)\n\
           printf(\"%d %d %d %d %d %d %d\\n\", g, hd, acc, lk_attn_lane_values(g, hd, acc), (int)lk_attn_warp_heads(g, hd, acc), lk_attn_warp_pass(g, hd), lk_attn_warp_slots(g, hd, acc));\n  return 0;\n}}\n",
        Kernel::Attention.source()
    );
    let bin = dir().join("warp-layout");
    nvcc(&program, &bin);
    let ran = Command::new(&bin).output().expect("the layout program runs");
    assert!(ran.status.success(), "the layout program exits cleanly");
    let (mut shapes, mut warps, mut passes, mut tiles) = (0, 0, 0, 0);
    for line in String::from_utf8(ran.stdout).unwrap().lines() {
        let v: Vec<usize> = line.split(' ').map(|x| x.parse().expect("a number")).collect();
        let (g, hd, acc) = (v[0], v[1], v[2]);
        assert_eq!(
            (v[3], v[4] == 1, v[5], v[6]),
            (attn_lane_values(g, hd, acc), attn_warp_heads(g, hd, acc), attn_warp_pass(g, hd), attn_warp_slots(g, hd, acc)),
            "{g} heads {hd} wide, {acc}-byte sums: the warp-local layout (a lane's values of a key, whether, heads a pass, slots)"
        );
        shapes += 1;
        warps += usize::from(attn_warp_heads(g, hd, acc));
        passes += usize::from(attn_warp_heads(g, hd, acc) && attn_warp_pass(g, hd) < g);
        tiles += usize::from(g <= 8 && matches!(hd, 64 | 128 | 256) && !attn_warp_heads(g, hd, acc));
    }
    assert_eq!(shapes, 2 * 16 * 144, "every shape printed");
    assert!(warps > 0 && passes > 0 && tiles > 0, "warp-local shapes ({warps}), some in passes ({passes}), some past a lane's registers ({tiles})");
}

/// A recurrent layer's gate projections are read in the checkpoint's own format, as the reference and llama.cpp read
/// them: kept in Q8_0 (as the real Qwen3.5 checkpoints keep them), each is a quantized projection of the layer's
/// activation — no `f32` GEMV and no fused gates from the activation, in the decode step or the batch — and the author
/// declares them quantized matrices; kept in `f32`, they take the `f32` GEMV and the gates fuse with them
/// (`gate_projections`), under every numerics. The layer's input epilogue (`gdn_input_epilogue`, which takes the gates
/// into its projections' launch) is held throughout.
#[test]
fn a_recurrent_layers_gate_projections_are_read_in_the_checkpoints_format() {
    use logicaffeine_compile::model::program;
    let spec = Tiny::Qwen35.spec(CTX);
    assert!(program::written(&spec, Cut::Decode { ctx: CTX }).0 .2.contains_key("Wb0"), "the author declares the write gate's projection quantized");
    for (gates, ty) in [("q8", GgmlType::Q8_0), ("f32", GgmlType::F32)] {
        let path = dir().join(format!("qwen35-gates-{gates}.gguf"));
        tiny_models::write_checkpoint_as(&spec, &path, &|t| {
            let quantized = tiny_models::tensors(&spec).iter().any(|(n, dims)| n == t && dims.len() == 2 && dims[0] % 32 == 0 && !n.contains("norm"));
            if t.contains("ssm_alpha") || t.contains("ssm_beta") {
                ty
            } else if quantized && !t.contains("ssm_conv") {
                GgmlType::Q8_0
            } else {
                GgmlType::F32
            }
        });
        for numerics in NUMERICS {
            let what = format!("{gates} gates, {numerics:?}");
            let lane = compile_batch_holding(&spec, &path, &options(numerics), 4, &["gdn_input_epilogue"]).unwrap_or_else(|e| panic!("{what}: {e}"));
            for f in [lane.decode(), lane.batch().unwrap()] {
                let s = &f.schedule;
                let f32_gates = s.launches.iter().filter(|l| matches!(l, Launch::GemvF32 { .. } | Launch::GateProj { .. })).count();
                if ty == GgmlType::Q8_0 {
                    assert_eq!(f32_gates, 0, "{what} {}: the gate projections are quantized projections: {:?}", f.tag(), s.fusions);
                    assert_eq!(s.fired("gate_projections"), 0, "{what} {}", f.tag());
                } else {
                    assert!(f32_gates > 0, "{what} {}: the gate projections in f32", f.tag());
                }
            }
            if ty == GgmlType::F32 {
                assert!(lane.decode().schedule.fired("gate_projections") > 0, "{what}: the decode step's gates fuse with their f32 projections");
            }
        }
    }
}

/// A recurrent layer's gates from its activation cut each head's dots over warps as the projections they replace cut
/// them (`kernels::gemv_f32_parts` over the heads and `k`), so the fused launch sums what its launches sum apart: at the
/// medium width (2048 values a row, the chip's multiprocessors) the decode step's `Launch::GateProj` takes the cut the
/// held-back program's `Launch::GemvF32` projections take, under every numerics — the gates' projections kept in `f32`.
/// The layer's input epilogue (`gdn_input_epilogue`, which takes the gates into its projections' launch) is held
/// throughout.
#[test]
fn a_fused_gate_projection_cuts_its_dots_as_its_projections_apart_do() {
    use logicaffeine_compile::codegen_cuda::lane::kernels::gemv_f32_parts;
    // Every matrix whose rows are whole 32-value blocks in Q8_0 but the gates' projections, in `f32` as a checkpoint
    // that keeps them so — the projections the fused gates replace — and the rest in `f32`.
    let blocked: Vec<String> = tiny_models::tensors(&Tiny::Qwen35.medium(CTX))
        .into_iter()
        .filter(|(n, dims)| dims.len() == 2 && dims[0] % 32 == 0 && !n.contains("norm") && !n.contains("ssm_alpha") && !n.contains("ssm_beta"))
        .map(|(n, _)| n)
        .collect();
    let (spec, path) = medium_checkpoint(Tiny::Qwen35, "gate-cut", &|t| if blocked.iter().any(|b| b == t) { GgmlType::Q8_0 } else { GgmlType::F32 });
    for numerics in NUMERICS {
        let what = format!("medium qwen35 {numerics:?}");
        let fused = compile_holding(&spec, &path, &options(numerics), &["gdn_input_epilogue"]).unwrap_or_else(|e| panic!("{what}: {e}"));
        let held = compile_holding(&spec, &path, &options(numerics), &["gate_projections", "gdn_input_epilogue"]).unwrap_or_else(|e| panic!("{what} held: {e}"));
        let sms = options(numerics).knobs.chip.sms;
        let cuts: Vec<(usize, usize, usize)> = fused
            .decode()
            .schedule
            .launches
            .iter()
            .filter_map(|l| match l {
                Launch::GateProj { k, heads, parts, .. } => Some((*k, *heads, *parts)),
                _ => None,
            })
            .collect();
        let apart: Vec<(usize, usize, usize)> = held
            .decode()
            .schedule
            .launches
            .iter()
            .filter_map(|l| match l {
                Launch::GemvF32 { k, n, split, m: 1, .. } => Some((*k, *n, *split)),
                _ => None,
            })
            .collect();
        assert!(!cuts.is_empty() && apart.len() == 2 * cuts.len(), "{what}: a fused launch a recurrent layer, two projections apart: {cuts:?} {apart:?}");
        for &(k, heads, parts) in &cuts {
            assert!(gemv_f32_parts(heads, k, sms) > 1, "{what}: the width cuts each dot over warps");
            assert_eq!(parts, gemv_f32_parts(heads, k, sms), "{what}: the fused launch cuts each dot as the GEMV does");
        }
        assert!(apart.iter().all(|&(k, n, split)| split == gemv_f32_parts(n, k, sms)), "{what}: the projections apart: {apart:?}");
    }
}

/// A decode step's causal convolutions of one row — a recurrent layer's query and key, each head normed to unit length,
/// and its value — run as one launch over their channels (`conv_parts`: a `Launch::ConvParts`, `lk_conv_parts`), each
/// written where its own convolution wrote it, under every numerics; held back, each runs alone. A prefill chunk's
/// convolutions over its rows stay apart. The layer's input epilogue (`gdn_input_epilogue`, which takes the
/// convolutions into its projections' launch) is held throughout. The program apart computes what the merged one computes
/// bit for bit (`cuda_lane_lock::every_fusion_computes_what_its_launches_compute_apart`).
#[test]
fn the_hybrid_step_convolves_its_query_key_and_value_in_one_launch() {
    let (spec, path) = checkpoint(Tiny::Qwen35, "conv-parts");
    for numerics in NUMERICS {
        let what = format!("qwen35 {numerics:?}");
        let lane = compile_holding(&spec, &path, &LaneOptions { prefill: 4, ..options(numerics) }, &["gdn_input_epilogue"]).unwrap_or_else(|e| panic!("{what}: {e}"));
        let s = &lane.decode().schedule;
        assert_eq!(s.fired("conv_parts"), 2, "{what}: one a recurrent layer: {:?}", s.fusions);
        let merged: Vec<&Vec<ConvPart>> = s
            .launches
            .iter()
            .filter_map(|l| if let Launch::ConvParts { parts, .. } = l { Some(parts) } else { None })
            .collect();
        assert_eq!(merged.len(), 2, "{what}: a launch a recurrent layer");
        for parts in merged {
            let outs: Vec<&str> = parts.iter().map(|p| p.y.as_str()).collect();
            assert_eq!(parts.iter().map(|p| p.l2).collect::<Vec<_>>(), [true, true, false], "{what}: the query and key normed, the value not: {outs:?}");
            assert!(outs[0].starts_with("qn") && outs[1].starts_with("kn") && outs[2].starts_with("vs"), "{what}: query, key, value: {outs:?}");
        }
        assert!(!s.launches.iter().any(|l| matches!(l, Launch::Conv { .. })), "{what}: no convolution alone");
        assert!(lane.program.cu.contains("lk_conv_parts<"), "{what}: by the convolution in parts");
        let chunk = lane.chunk(4).unwrap_or_else(|| panic!("{what}: the chunk of 4")).schedule.clone();
        assert_eq!(chunk.fired("conv_parts"), 0, "{what}: a chunk's convolutions over its rows stay apart");
        let held = compile_holding(&spec, &path, &options(numerics), &["conv_parts", "gdn_input_epilogue"]).unwrap_or_else(|e| panic!("{what} held: {e}"));
        let h = &held.decode().schedule;
        assert_eq!(h.fired("conv_parts"), 0, "{what} held");
        assert_eq!(h.launches.iter().filter(|l| matches!(l, Launch::Conv { .. })).count(), 6, "{what} held: three a recurrent layer");
    }
}

/// A sandwiched step's post-norm — a sublayer's output normed before the residual takes it — runs inside the norm of
/// the sum it feeds (`sandwich_norm`: the add-and-norm's added operand normed in the same block, by the same reduction
/// a launch of its own takes), each layer's attention and feed-forward outputs alike, under every numerics; held back,
/// each post-norm is a launch of its own. The projections' norm epilogues (`gemv_norm_epilogue`, which run these norms in
/// the last block of the projection before them) are held throughout. The fused step computes what the launches compute
/// apart, bit for bit (`cuda_lane_lock::every_fusion_computes_what_its_launches_compute_apart`).
#[test]
fn the_sandwiched_step_norms_each_sublayer_output_inside_the_sums_norm() {
    let (spec, path) = checkpoint(Tiny::Gemma3, "sandwich");
    for numerics in NUMERICS {
        let what = format!("gemma3 {numerics:?}");
        let lane = compile_holding(&spec, &path, &options(numerics), &["gemv_norm_epilogue"]).unwrap_or_else(|e| panic!("{what}: {e}"));
        let s = &lane.decode().schedule;
        assert_eq!(s.fired("sandwich_norm"), 6, "{what}: two a layer: {:?}", s.fusions);
        let folded = s.launches.iter().filter(|l| matches!(l, Launch::Norm { pre: Some(_), .. })).count();
        assert_eq!(folded, 6, "{what}: six norms each norm their added operand first");
        for layer in 0..3 {
            for post in [format!("ao{layer}n"), format!("dn{layer}n")] {
                assert!(!s.launches.iter().any(|l| l.writes().contains(&post.as_str())), "{what}: no launch writes `{post}`");
            }
        }
        let held = compile_holding(&spec, &path, &options(numerics), &["sandwich_norm", "gemv_norm_epilogue"]).unwrap_or_else(|e| panic!("{what} held: {e}"));
        let h = &held.decode().schedule;
        assert_eq!(h.fired("sandwich_norm"), 0, "{what} held");
        assert_eq!(h.census()["rmsnorm"], s.census()["rmsnorm"] + 6, "{what}: held back, each post-norm a launch of its own");
    }
}

/// The decode step attends a long cache in one launch a layer, its span cut into chunks a block each and the last block
/// to arrive folding them (split-K): the scores, the softmax and the weighted sum in that one launch, so the step's
/// census is the short cache's.
#[test]
fn the_decode_step_attends_a_long_cache_in_chunks_one_launch_a_layer() {
    let (spec, path) = checkpoint(Tiny::Qwen3, "split-decode");
    let short = compile(&spec, &path, &options(Q8MatKind::Int8Act)).unwrap();
    let long = compile(&spec, &path, &LaneOptions { ctx: 30_000, ..options(Q8MatKind::Int8Act) }).unwrap();
    let s = &long.decode().schedule;
    assert_eq!(s.census(), short.decode().schedule.census(), "cutting the span adds no launch");
    let attentions: Vec<&Launch> = s.launches.iter().filter(|l| matches!(l, Launch::Attention { .. })).collect();
    assert_eq!(attentions.len(), 3, "one attention a layer");
    for l in attentions {
        assert!(
            matches!(l, Launch::Attention { splits, parallel: true, shared: None, .. } if *splits > 1),
            "each attention's span cut into chunks, a block each: {l:?}"
        );
    }
}

/// Every expert mixture of `s` reads its activation as the int8 the norm before it writes in its epilogue, and nothing
/// reads that norm's `f32`: straight from the norm, in a decode step and over a chunk's rows alike.
fn experts_read_their_norms_quantized(s: &Schedule, what: &str) {
    let mut experts = 0;
    for l in mixture_launches(s) {
        let Launch::Moe { x, .. } = l else { continue };
        experts += 1;
        let read = &s.buffers[x];
        let normed = match &read.fill {
            Fill::Alias { of, offset } => {
                assert_eq!(offset % 32, 0, "{what}: the routed row `{x}` starts on a whole block of `{of}`");
                of.as_str()
            }
            _ => x.as_str(),
        };
        assert!(read.quantized && !read.plain, "{what}: the mixture reads `{x}` quantized, and only quantized");
        assert!(s.buffers[normed].quantized && !s.buffers[normed].plain, "{what}: `{normed}` is kept only quantized");
        assert!(
            s.launches.iter().flat_map(|l| l.parts()).any(|p| matches!(p, Launch::Norm { out, quant: true, f32: false, .. } if out == normed)),
            "{what}: the norm computing `{normed}` quantizes it in its epilogue"
        );
    }
    assert!(experts > 0, "{what}: the step routes through expert mixtures");
}

/// A decode step's expert mixture routes its token in one launch set, reading int8 activations as the projections do
/// (llama.cpp's experts read theirs quantized too): each layer's three norms before a quantized read — the attention's,
/// the mixture's and the head's — quantize in their epilogues, the first in the embedding's gather and the mixture's in
/// the output projection's last block (`gemv_norm_epilogue`). Each layer's query, key and value project in one launch
/// with each head's norm and rotation in its epilogue (`qkv_head_epilogue`).
#[test]
fn the_expert_step_routes_each_token_in_one_launch_set() {
    let lane = compiled(Tiny::Qwen3Moe, Q8MatKind::Int8Act, "moe");
    assert_eq!(
        lane.decode().schedule.census(),
        counts(&[
            ("argmax", 1),
            ("attention", 3),
            ("gather_norm", 1),
            ("gemv_norm", 3),
            ("gemv_q8", 1),
            ("gemv_rope", 3),
            ("moe_norm", 3),
            ("quant", 3)
        ])
    );
    assert_eq!(lane.decode().schedule.fired("fused_quant"), 3 + 3 + 1);
    assert!(mixture_launches(&lane.decode().schedule).iter().all(|l| matches!(l, Launch::Moe { rows: 1, .. })), "a token's mixture");
    experts_read_their_norms_quantized(&lane.decode().schedule, "decode");
}

/// Gemma 3's step: the embedding row scaled inside its gather, which norms it for the first layer (`embed_norm`); two
/// windowed layers and a global one, each turning by its own rotation's tables, the query, key and value projections in
/// one launch with each head's norm and rotation in its epilogue (`gemv_parts`, `qkv_head_epilogue`); a norm after the
/// mixer and after the FFN, each inside the norm of the sum it is added into (`sandwich_norm`), the residual add of the
/// next one inside the norm that reads it first, all of it in the last block of the projection before
/// (`gemv_norm_epilogue`); the gate and up projections and the GeLU gate in one launch, which quantizes for the down
/// projection.
#[test]
fn the_sandwiched_step_windows_its_local_layers_and_fuses_its_gates() {
    let lane = compiled(Tiny::Gemma3, Q8MatKind::Int8Act, "gemma");
    let s = &lane.decode().schedule;
    assert_eq!(
        s.census(),
        counts(&[
            ("argmax", 1),
            ("attention", 3),
            ("gather_norm", 1),
            ("gemv_glu", 3),
            ("gemv_norm", 6),
            ("gemv_q8", 1),
            ("gemv_rope", 3),
            ("quant", 3)
        ])
    );
    for (rule, n) in [
        ("gather_scale", 1),
        ("add_norm", 6),
        ("sandwich_norm", 6),
        ("head_norm_rope", 6),
        ("row_write", 6),
        ("act_gate", 3),
        ("glu_pair", 3),
        ("fused_quant", 10),
        ("argmax_advance", 1),
        ("gemv_parts", 3),
        ("qkv_head_epilogue", 6),
        ("gemv_norm_epilogue", 6),
        ("embed_norm", 1),
        ("rope_parts", 0),
    ] {
        assert_eq!(s.fired(rule), n, "{rule}");
    }
    let windows: Vec<Option<usize>> =
        s.launches.iter().filter_map(|l| if let Launch::Attention { window, .. } = l { Some(*window) } else { None }).collect();
    assert_eq!(windows, [Some(tiny_models::WINDOW), Some(tiny_models::WINDOW), None], "the local layers attend their window");
    let tables: Vec<(&str, &str)> = s
        .launches
        .iter()
        .flat_map(|l| l.parts())
        .flat_map(|l| match l {
            Launch::Rope { cos, sin, .. } => vec![(cos.as_str(), sin.as_str())],
            Launch::RopeParts { sides, cos, sin, .. } => vec![(cos.as_str(), sin.as_str()); sides.len()],
            _ => vec![],
        })
        .collect();
    assert_eq!(
        tables,
        [("Rcos", "Rsin"), ("Rcos", "Rsin"), ("Rcos", "Rsin"), ("Rcos", "Rsin"), ("Rcos1", "Rsin1"), ("Rcos1", "Rsin1")],
        "the local layers turn by the first rotation, the global one by the second"
    );
    assert!(
        s.launches.iter().flat_map(|l| l.parts()).any(|l| matches!(l, Launch::Gather { scale: Some(f), .. } if *f == 8.0)),
        "the embedding row is read scaled by √64"
    );
    let pairs: Vec<&Launch> = s.launches.iter().filter(|l| matches!(l, Launch::GemvGlu { .. })).collect();
    assert!(
        pairs.len() == 3
            && pairs.iter().all(|l| matches!(l, Launch::GemvGlu { act: Some(logicaffeine_compile::tensor_recognize::qwen::Activation::Gelu), quant: true, f32: false, .. })),
        "every gated pair is the GeLU gate, quantized for the down projection and kept only so: {pairs:?}"
    );
}

/// gpt-oss's step: every projection's bias inside its GEMV, the query, key and value in one with each head's rotation in
/// its epilogue (`gemv_parts`, `qkv_head_epilogue`); the output projection with the residual and the mixture's norm in
/// its last block (`gemv_norm_epilogue`); the even layers windowed, every layer's sinks in its softmax; each mixture routed by its `f32` router (so the norm before it keeps its `f32` output beside the quantized
/// one the experts read), with the router's and the experts' biases and the clamped SwiGLU inside the expert kernels.
#[test]
fn the_biased_step_folds_its_biases_and_routes_by_an_f32_router() {
    let lane = compiled(Tiny::GptOss, Q8MatKind::Int8Act, "gptoss");
    let s = &lane.decode().schedule;
    assert_eq!(
        s.census(),
        counts(&[
            ("argmax", 1),
            ("attention", 3),
            ("gather_norm", 1),
            ("gemv_norm", 3),
            ("gemv_q8", 1),
            ("gemv_rope", 3),
            ("moe_norm", 3),
            ("quant", 3)
        ])
    );
    for (rule, n) in [
        ("gemv_bias", 12),
        ("add_norm", 6),
        ("row_write", 6),
        ("fused_quant", 7),
        ("argmax_advance", 1),
        ("gemv_parts", 3),
        ("qkv_head_epilogue", 6),
        ("gemv_norm_epilogue", 3),
        ("embed_norm", 1),
        ("rope_parts", 0),
    ] {
        assert_eq!(s.fired(rule), n, "{rule}");
    }
    let attention: Vec<(Option<usize>, bool)> = s
        .launches
        .iter()
        .filter_map(|l| if let Launch::Attention { window, sinks, .. } = l { Some((*window, sinks.is_some())) } else { None })
        .collect();
    assert_eq!(
        attention,
        [(Some(tiny_models::WINDOW), true), (None, true), (Some(tiny_models::WINDOW), true)],
        "the even layers windowed, every layer with its sinks"
    );
    for l in mixture_launches(s) {
        if let Launch::Moe { x, router, rb, biases, act, .. } = l {
            assert_eq!(*router, Router::F32, "gpt-oss routes by an f32 router");
            assert!(rb.is_some() && biases.iter().all(Option::is_some), "every bias reaches the expert kernels");
            assert_eq!(*act, MoeAct::ClampedSwiglu { limit: 7.0f64.to_bits(), slope: 1.702f64.to_bits() });
            let read = &s.buffers[x];
            assert!(read.plain && read.quantized, "the router reads `{x}` f32 and the experts quantized");
        }
    }
    assert!(
        s.launches.iter().all(|l| !matches!(l, Launch::Gemv { bias: None, .. }) || matches!(l, Launch::Gemv { n, .. } if *n == tiny_models::VOCAB)),
        "every projection but the head carries its bias"
    );
}

/// DeepSeek-V2's latent attention, absorbed: per layer the query and the latent projections, the latent's norm, two
/// rotations, the absorption by `Wkb` and the expansion by `Wvb` as per-head products, and an attention that reads its
/// values in place in the cache (`strided_values`) at the model's scale; a dense first layer, then mixtures that weigh
/// their experts by the whole softmax beside a shared expert; the embedding's gather norms its row (`embed_norm`), and
/// each Q8_0 projection a residual add and its norm follow takes them into its last block (`gemv_norm_epilogue`). The
/// per-head products read their activations `f32`, so no
/// launch quantizes a head's unrotated query or its attended latent. No launch slices, joins or copies a row: the latent's norm
/// and the shared key's rotation read their parts of the latent projection in place and write the cache row's two parts
/// straight into the cache, and the absorption and the queries' rotation write each head's query's two parts — the
/// rotation reading its part of the query projection in place, the absorption too when a head is whole blocks wide.
#[test]
fn the_latent_step_absorbs_each_heads_query_and_reads_its_values_in_the_cache() {
    for (t, kb_rows_out) in [(Tiny::DeepSeek2, false), (Tiny::DeepSeek2Split, true)] {
        let lane = compiled(t, Q8MatKind::Int8Act, &format!("latent-{}", t.name()));
        let s = &lane.decode().schedule;
        let what = t.name();
        // The split checkpoint's low-rank query adds a projection and a norm a layer.
        let low_rank = usize::from(t == Tiny::DeepSeek2Split);
        // A head's query is 72 values: its unrotated part, not whole quantized blocks, is sliced for the absorption. The
        // dense first layer's gate and up are one launch; each mixture takes its shared expert into its own launches.
        assert_eq!(
            s.census(),
            counts(&[
                ("argmax", 1),
                ("attention", 3),
                ("gather_norm", 1),
                ("gemv_glu", 1),
                ("gemv_norm", 4),
                ("gemv_parts", 3),
                ("gemv_q8", 1 + 3 * low_rank),
                ("head_gemv", 6),
                ("moe_norm", 2),
                ("quant", 3),
                ("rmsnorm", 3 + 3 * low_rank),
                ("rope", 3),
                ("slice", 3)
            ]),
            "{what}"
        );
        for (rule, n) in [
            ("add_norm", 6),
            ("fused_quant", 8 + 3 * low_rank),
            ("slice_read", 9),
            ("row_write", 3),
            ("join_in_place", 6),
            ("strided_values", 3),
            ("glu_pair", 3),
            ("shared_expert", 2),
            ("argmax_advance", 1),
            ("embed_norm", 1),
            ("gemv_norm_epilogue", 4),
        ] {
            assert_eq!(s.fired(rule), n, "{what}: {rule}");
        }
        let spec = t.spec(CTX);
        let shared: Vec<&SharedExperts> =
            mixture_launches(s).into_iter().filter_map(|l| if let Launch::Moe { shared, .. } = l { shared.as_ref() } else { None }).collect();
        let ffn = |l: usize| match &spec.layers[l].ffn {
            logicaffeine_compile::model::spec::Ffn::Moe(m) => m.shared_dff,
            _ => panic!("{what}: layer {l} is a mixture"),
        };
        assert_eq!(
            shared,
            [1, 2]
                .map(|l| SharedExperts {
                    wg: format!("Wsg{l}"),
                    wu: format!("Wsu{l}"),
                    wd: format!("Wsd{l}"),
                    formats: [GgmlType::Q8_0; 3],
                    dff: ffn(l),
                })
                .iter()
                .collect::<Vec<_>>(),
            "{what}: each mixture's shared expert, read in place"
        );
        let Mixer::Mla(m) = &spec.layers[0].mixer else { panic!("{what} has latent attention") };
        latent_parts_land_in_place(s, m, what);
        let kw = m.kv_lora_rank + m.qk_rope;
        for l in s.launches.iter().flat_map(|l| l.parts()) {
            if let Launch::Attention { kc, vc, hd, vd, group, heads, vstride, voff, scale, .. } = l {
                assert_eq!(vc, kc, "{what}: the values are read in the cache");
                assert_eq!((*hd, *vd, *vstride, *voff), (kw, m.kv_lora_rank, kw, 0), "{what}: the latent part of every cache row");
                assert_eq!((*heads, *group), (m.heads, m.heads), "{what}: every head over the one cache row");
                assert_eq!(*scale, m.scale, "{what}: the model's score scale");
            }
            if let Launch::Moe { renormalize, .. } = l {
                assert!(!renormalize, "{what}: the kept experts weigh their softmax over every expert");
            }
        }
        let heads: Vec<(usize, usize, bool)> = s
            .launches
            .iter()
            .filter_map(|l| if let Launch::HeadGemv { a, b, rows_out, .. } = l { Some((*a, *b, *rows_out)) } else { None })
            .collect();
        let per_layer = [(m.qk_nope, m.kv_lora_rank, kb_rows_out), (m.kv_lora_rank, m.v_dim, true)];
        assert_eq!(heads, per_layer.repeat(3), "{what}: the absorption and the expansion, a head's rows read in place");
    }
    // A wide head's query is whole blocks: the absorption reads its unrotated part in place too, and no launch slices.
    for t in [Tiny::DeepSeek2, Tiny::DeepSeek2Split] {
        let (spec, path) = wide_checkpoint(t, "latent", &|_| GgmlType::Q8_0);
        let what = format!("wide {}", t.name());
        let lane = compile(&spec, &path, &options(Q8MatKind::Int8Act)).unwrap_or_else(|e| panic!("{what}: {e}"));
        let s = &lane.decode().schedule;
        assert_eq!(s.census().get("slice"), None, "{what}");
        assert_eq!(s.fired("slice_read"), 12, "{what}");
        let Mixer::Mla(m) = &spec.layers[0].mixer else { panic!("{what} has latent attention") };
        latent_parts_land_in_place(s, m, &what);
    }
}

/// Each latent layer's cache row and queries land in place: the latent's norm reads the latent part of every row of
/// the latent projection `kva` and writes the first `kv_lora_rank` of the cache's row at the position, the shared key's
/// rotation reads the rotated part and writes the row's last `qk_rope`; the absorption writes the first
/// `kv_lora_rank` of each head's query `qa` and the queries' rotation, reading each head's rotated part of `qr`, its
/// last `qk_rope` — the absorption reading each head's unrotated part of `qr` in place when it is whole blocks. A
/// rotation is a launch's own or a side of the layer's rotations in one launch (`rope_parts`).
fn latent_parts_land_in_place(s: &Schedule, m: &Mla, what: &str) {
    let turns = |l: &Launch, x: &str, p: Option<Part>, y: &Dst| match l {
        Launch::Rope { x: rx, part, y: ry, .. } => rx == x && *part == p && ry == y,
        Launch::RopeParts { sides, .. } => sides.iter().any(|side| side.x == x && side.part == p && side.y == *y),
        _ => false,
    };
    let (lora, rope, nope) = (m.kv_lora_rank, m.qk_rope, m.qk_nope);
    let (kw, qk) = (lora + rope, nope + rope);
    let part = |pitch, offset| Some(Part { pitch, offset });
    let into = |buf: &str, row: bool, pitch, offset| Dst {
        buf: buf.to_string(),
        row: row.then(|| (RowSel::Param("pos".into()), kw)),
        unit: part(pitch, offset),
    };
    for l in 0..3 {
        let (kva, kc, qr, qa) = (format!("kva{l}"), format!("Kc{l}"), format!("qr{l}"), format!("qa{l}"));
        let writers = |t: &str| s.launches.iter().filter(|x| x.writes().contains(&t)).collect::<Vec<_>>();
        let row = writers(&kc);
        assert_eq!(row.len(), 2, "{what}: the cache row's two producers write it, nothing copies it: {row:?}");
        assert!(
            matches!(row[0], Launch::Norm { x, part: p, into: Some(d), .. } if *x == kva && *p == part(kw, 0) && *d == into(&kc, true, kw, 0)),
            "{what}: the latent normed from its part of `{kva}` into the row's first {lora}: {:?}",
            row[0]
        );
        assert!(
            turns(row[1], &kva, part(kw, lora), &into(&kc, true, kw, lora)),
            "{what}: the shared key turned from its part of `{kva}` into the row's last {rope}: {:?}",
            row[1]
        );
        let query = writers(&qa);
        assert_eq!(query.len(), 2, "{what}: each head's query's two producers write it, nothing joins it: {query:?}");
        let whole = qk % 32 == 0;
        assert!(
            matches!(query[0], Launch::HeadGemv { x, part: p, y, .. }
                if (*x == qr) == whole && *p == part(qk, 0).filter(|_| whole) && *y == into(&qa, false, lora + rope, 0)),
            "{what}: the absorbed part of each head's query, its unrotated part read in place when whole blocks: {:?}",
            query[0]
        );
        assert!(
            turns(query[1], &qr, part(qk, nope), &into(&qa, false, lora + rope, lora)),
            "{what}: each head's rotated query part turned from its part of `{qr}` into the query's last {rope}: {:?}",
            query[1]
        );
    }
}

/// A hand-written step planned on its own: every constant a value buffer, the precise numerics.
fn plan_of(src: &str) -> Result<Schedule, LaneError> {
    let rp = recognize_stages(src).unwrap_or_else(|e| panic!("{e:?}\n{src}"));
    let graph = StepGraph::build(&rp).unwrap_or_else(|e| panic!("{e}\n{src}"));
    let constants = graph.of_role(Role::Constant).map(|t| (t.name.clone(), Placement::Values(vec![0.0; t.len()]))).collect();
    plan(&graph, &|s| rp.name(s).to_string(), constants, Q8MatKind::Dequant, &CudaKnobs::default(), decode_io(), &[])
}

fn planned(src: &str) -> Schedule {
    plan_of(src).unwrap_or_else(|e| panic!("{e}\n{src}"))
}

/// A latent cache row's step by hand: the latent and the shared key cut from one row, the latent normed, the key
/// turned, the two joined and set into the cache at the position; `between` after the norm, `after` after the write.
fn cache_row_step(between: &str, after: &str) -> String {
    format!(
        "## Main\nLet tok be 1.\nLet pos be 1.\nLet Emb be a 8 by 72 tensor of Real.\nLet w be a 1 by 64 tensor of Real.\n\
         Let Rcos be a 24 by 4 tensor of Real.\nLet Rsin be a 24 by 4 tensor of Real.\nLet Kc be a 24 by 72 tensor of Real.\n\
         Let x be row tok of Emb.\nLet kvl be the first 64 of every 72 of x.\nLet kpe be the last 8 of every 72 of x.\n\
         Let kvn be rmsnorm of kvl with weight w.\n{between}\
         Let kpp be rope of kpe with cos Rcos and sin Rsin from position pos across 1 heads.\nLet kc be kvn joined with kpp.\n\
         Set row pos of Kc to kc.\n{after}Let logits be rmsnorm of Kc.\nLet next be argmax of logits.\n"
    )
}

/// The three folds that land the latent cache row in place fire exactly where their dataflow allows: on the plain
/// step all three; a launch reading the cache between the halves' producers and the join keeps the join (its row still
/// written in place); a half read by anything but the join keeps the join; a part of every row of a table read after
/// the table is written keeps its slice.
#[test]
fn the_in_place_folds_hold_back_where_a_launch_between_would_see_the_difference() {
    let fired = |s: &Schedule| ["slice_read", "row_write", "join_in_place"].map(|r| s.fired(r));
    let plain = planned(&cache_row_step("", ""));
    assert_eq!(fired(&plain), [2, 1, 1], "every fold on the plain step: {:?}", plain.fusions);
    for kind in ["slice", "join", "set_row"] {
        assert_eq!(plain.census().get(kind), None, "no {kind} launch: {:?}", plain.census());
    }
    let peeked = planned(&cache_row_step("Let peek be rmsnorm of Kc.\n", ""));
    assert_eq!(fired(&peeked), [2, 1, 0], "the cache read between the norm and the join holds the join back");
    assert!(
        peeked.launches.iter().any(|l| matches!(l, Launch::Join { y, .. } if y.buf == "Kc" && y.row.is_some() && y.unit.is_none())),
        "the join writes the whole row straight into the cache: {:?}",
        peeked.launches
    );
    let shared = planned(&cache_row_step("", "Let again be rmsnorm of kvn.\n"));
    assert_eq!(fired(&shared), [2, 1, 0], "a half read past the join is kept, and so is the join");
    let stale = planned(&cache_row_step(
        "",
        "Let early be the first 64 of every 72 of Kc.\nSet row tok of Kc to kc.\nLet late be rmsnorm of early.\n",
    ));
    assert_eq!(stale.fired("slice_read"), 2, "the slice of a table written before its reader stays a launch");
    assert!(
        stale.launches.iter().any(|l| matches!(l, Launch::Slice { x, .. } if x == "Kc")),
        "the cache's slice is taken before the second write: {:?}",
        stale.launches
    );
}

/// A hand-written step planned under `numerics` with its weights `rows` read as Q8_0 checkpoint rows — each with
/// whether the checkpoint holds it transposed (a projection's or a router's outputs as rows) or as declared (an expert
/// stack) — and every other constant as values.
fn planned_rows(src: &str, rows: &[(&str, bool)], numerics: Q8MatKind) -> Schedule {
    planned_rows_as(src, rows, numerics, GgmlType::Q8_0)
}

/// [`planned_rows`] with the weights `rows` in format `ty`.
fn planned_rows_as(src: &str, rows: &[(&str, bool)], numerics: Q8MatKind, ty: GgmlType) -> Schedule {
    plan_rows_as(src, rows, numerics, ty).unwrap_or_else(|e| panic!("{e}\n{src}"))
}

/// [`planned_rows_as`]'s plan, or the lane's refusal.
fn plan_rows_as(src: &str, rows: &[(&str, bool)], numerics: Q8MatKind, ty: GgmlType) -> Result<Schedule, LaneError> {
    let rp = recognize_stages(src).unwrap_or_else(|e| panic!("{e:?}\n{src}"));
    let graph = StepGraph::build(&rp).unwrap_or_else(|e| panic!("{e}\n{src}"));
    let constants = graph
        .of_role(Role::Constant)
        .map(|t| {
            let placed = match rows.iter().find(|(name, _)| *name == t.name) {
                Some(&(_, transposed)) => Placement::Rows(Rows {
                    tensor: t.name.clone(),
                    ty,
                    rows: if transposed { t.cols } else { t.rows },
                    cols: if transposed { t.rows } else { t.cols },
                    transposed,
                    runs: Vec::new(),
                }),
                None => Placement::Values(vec![0.0; t.len()]),
            };
            (t.name.clone(), placed)
        })
        .collect();
    let knobs = CudaKnobs { activations: numerics, ..CudaKnobs::default() };
    plan(&graph, &|s| rp.name(s).to_string(), constants, numerics, &knobs, decode_io(), &[])
}

/// A projection that reads its activation quantized reads it in whole 32-value blocks: a quantized projection of an
/// activation of 40 values is refused by name under each quantized numerics — the activation, its length and the
/// next whole blocks named — and planned under the precise numerics, which read no quantized form.
#[test]
fn a_quantized_projection_of_an_activation_short_of_whole_blocks_is_refused() {
    let src = "## Main\nLet tok be 1.\nLet pos be 1.\nLet Emb be a 8 by 40 tensor of Real.\nLet W be a 40 by 64 tensor of Real.\n\
               Let x be row tok of Emb.\nLet y be x matmul W.\nLet logits be rmsnorm of y.\nLet next be argmax of logits.\n";
    for numerics in [Q8MatKind::Int8Act, Q8MatKind::TwoPassInt8] {
        match plan_rows_as(src, &[("W", true)], numerics, GgmlType::Q8_0) {
            Err(LaneError::Shape { var, declared: 40, bound: 64 }) => assert_eq!(var, "x", "{numerics:?}"),
            other => panic!("{numerics:?}: a quantized read of 40 values is refused by name: {other:?}"),
        }
    }
    assert!(plan_rows_as(src, &[("W", true)], Q8MatKind::Dequant, GgmlType::Q8_0).is_ok(), "the precise numerics read no quantized form");
}

/// A mixture of four experts of 32 keeping two over a 64-wide activation, and a shared SwiGLU expert of 64 beside it,
/// summed onto the residual — by hand.
fn shared_expert_step() -> String {
    "## Main\nLet tok be 1.\nLet pos be 1.\nLet Emb be a 8 by 64 tensor of Real.\nLet Wr be a 64 by 4 tensor of Real.\n\
     Let Wg be a 128 by 64 tensor of Real.\nLet Wu be a 128 by 64 tensor of Real.\nLet Wd be a 256 by 32 tensor of Real.\n\
     Let Wsg be a 64 by 64 tensor of Real.\nLet Wsu be a 64 by 64 tensor of Real.\nLet Wsd be a 64 by 64 tensor of Real.\n\
     Let x be row tok of Emb.\nLet y be rmsnorm of x.\n\
     Let mo be moe of x with router Wr gate Wg up Wu down Wd choosing 2 of 4.\n\
     Let sg be x matmul Wsg.\nLet su be x matmul Wsu.\nLet sh be swiglu of sg and su.\nLet sd be sh matmul Wsd.\n\
     Let me be mo plus sd.\nLet xo be x plus me.\nLet logits be rmsnorm of xo.\nLet next be argmax of logits.\n"
        .to_string()
}

const SHARED_EXPERT_ROWS: [(&str, bool); 7] =
    [("Wr", true), ("Wg", false), ("Wu", false), ("Wd", false), ("Wsg", true), ("Wsu", true), ("Wsd", true)];

/// The shared expert folds into its mixture's launch — its gate and up rows computed beside the kept experts', its
/// down row summed after theirs — exactly where nothing else sees the parts: either order of the sum folds; a mixture,
/// a down projection or a gated pair read by anything but the sum, a shared expert of another activation, a mixture
/// gated other than by a SwiGLU, and a mixture keeping 32 experts (a block of 33 warps is past the launch limit) each
/// keep the shared expert's own launches.
#[test]
fn the_shared_expert_folds_into_its_mixture_where_nothing_else_reads_its_parts() {
    let plain = shared_expert_step();
    for numerics in NUMERICS {
        let s = planned_rows(&plain, &SHARED_EXPERT_ROWS, numerics);
        assert_eq!(s.fired("shared_expert"), 1, "{numerics:?}: {:?}", s.fusions);
        assert_eq!(s.fired("glu_pair"), 1, "{numerics:?}: the pair fused first, then folded");
        let kinds: BTreeSet<&str> = s.census().into_keys().collect();
        assert!(
            !kinds.contains("gemv_glu") && !kinds.contains("gemv_q8") && !kinds.contains("binary"),
            "{numerics:?}: the shared expert and the sum leave no launch of their own: {:?}",
            s.census()
        );
        let moe: Vec<&Launch> = mixture_launches(&s);
        assert!(
            matches!(moe[..], [Launch::Moe { y, shared: Some(sh), x, .. }]
                if *y == Dst::to("me") && x == "x" && *sh == SharedExperts {
                    wg: "Wsg".into(), wu: "Wsu".into(), wd: "Wsd".into(), formats: [GgmlType::Q8_0; 3], dff: 64
                }),
            "{numerics:?}: one mixture launch writes the sum, its shared expert's rows read in place: {moe:?}"
        );
    }
    let swapped = planned_rows(&plain.replace("Let me be mo plus sd.", "Let me be sd plus mo."), &SHARED_EXPERT_ROWS, Q8MatKind::Int8Act);
    assert_eq!(swapped.fired("shared_expert"), 1, "the sum in either order");
    let held = |what: &str, src: String, rows: &[(&str, bool)]| {
        let s = planned_rows(&src, rows, Q8MatKind::Int8Act);
        assert_eq!(s.fired("shared_expert"), 0, "{what} keeps the shared expert apart: {:?}", s.fusions);
        assert!(
            mixture_launches(&s).iter().all(|l| !matches!(l, Launch::Moe { shared: Some(_), .. })),
            "{what}: no mixture carries the shared expert"
        );
        assert!(s.launches.iter().flat_map(|l| l.parts()).any(|l| matches!(l, Launch::Binary { .. })), "{what}: the sum is a launch's");
    };
    let before_sum = |line: &str| plain.replace("Let me be mo plus sd.", &format!("{line}\nLet me be mo plus sd."));
    held("the mixture read past the sum", before_sum("Let peek be rmsnorm of mo."), &SHARED_EXPERT_ROWS);
    held("the down projection read past the sum", before_sum("Let peek be rmsnorm of sd."), &SHARED_EXPERT_ROWS);
    held("the gated pair read past its down projection", before_sum("Let peek be rmsnorm of sh."), &SHARED_EXPERT_ROWS);
    held(
        "a shared expert of another activation",
        plain.replace("Let sg be x matmul", "Let sg be y matmul").replace("Let su be x matmul", "Let su be y matmul"),
        &SHARED_EXPERT_ROWS,
    );
    held(
        "a mixture under the clamped SwiGLU",
        plain.replace("choosing 2 of 4", "choosing 2 of 4 using a swiglu clamped at 7.0 with slope 1.702"),
        &SHARED_EXPERT_ROWS,
    );
    held(
        "a mixture keeping 32 experts",
        plain
            .replace("Let Wr be a 64 by 4 tensor", "Let Wr be a 64 by 64 tensor")
            .replace("Let Wg be a 128 by 64 tensor", "Let Wg be a 2048 by 64 tensor")
            .replace("Let Wu be a 128 by 64 tensor", "Let Wu be a 2048 by 64 tensor")
            .replace("Let Wd be a 256 by 32 tensor", "Let Wd be a 4096 by 32 tensor")
            .replace("choosing 2 of 4", "choosing 32 of 64"),
        &SHARED_EXPERT_ROWS,
    );
}

/// Every staged per-head launch fits its rows and the kernel's own sums (`kernels::AXPY_SUMS`) in a block's default
/// 48 KB, all a launch takes without its limit raised: DeepSeek-V2's wide program with every matrix in each block format
/// the staged decode reads, under both quantized numerics, in chunks of 64.
#[test]
fn every_staged_per_head_launch_fits_the_default_shared_memory() {
    use logicaffeine_compile::codegen_cuda::lane::kernels::{axpy_staged, head_mode, AXPY_SUMS};
    let mut staged = 0;
    for &ty in BLOCK_FORMATS.iter().filter(|&&ty| axpy_staged(ty, head_mode(Q8MatKind::Int8Act))) {
        let (spec, path) = wide_checkpoint(Tiny::DeepSeek2, &format!("staged-{}", ty.name()), &|_| ty);
        for numerics in [Q8MatKind::Int8Act, Q8MatKind::TwoPassInt8] {
            let lane = compile(&spec, &path, &LaneOptions { ctx: 128, prefill: 64, ..options(numerics) })
                .unwrap_or_else(|e| panic!("{} {numerics:?}: {e}", ty.name()));
            for line in lane.program.cu.lines().map(str::trim_start).filter(|l| l.starts_with("lk_head_axpy_staged<")) {
                let config = line.split_once("<<<").and_then(|(_, r)| r.split_once(">>>")).expect("a launch configuration").0;
                let rows: usize = config.rsplit(", ").nth(1).and_then(|b| b.trim().parse().ok()).expect("its shared memory");
                staged += 1;
                assert!(
                    rows + AXPY_SUMS <= 48 * 1024,
                    "{} {numerics:?}: {line} takes {rows} bytes of rows beside the {AXPY_SUMS} of its sums",
                    ty.name()
                );
            }
        }
    }
    assert!(staged > 0, "the absorption stages its rows");
}

/// Every launch that takes more than a block's default 48 KB of dynamic shared memory has its kernel's limit raised
/// to at least what it takes (`cudaFuncSetAttribute`, the instantiation it launches), or the launch fails: every tiny
/// architecture's program under every numerics, and Qwen3's, gpt-oss's, DeepSeek-V2's and Qwen3.5's wide programs,
/// whose chunks of 64 take the block GEMMs, the attentions in tiles and the delta rule in chunks.
#[test]
fn every_launch_past_the_default_shared_memory_raises_its_limit() {
    // The launches of a program: (kernel instantiation, dynamic shared memory bytes) for each `<<<…>>>` naming them.
    let launches = |cu: &str| -> Vec<(String, usize)> {
        cu.lines()
            .filter_map(|line| {
                let line = line.trim_start();
                let (name, rest) = line.split_once("<<<")?;
                let config = rest.split_once(">>>")?.0;
                let (mut depth, mut parts, mut at) = (0i32, Vec::new(), 0);
                for (i, c) in config.char_indices() {
                    match c {
                        '(' => depth += 1,
                        ')' => depth -= 1,
                        ',' if depth == 0 => {
                            parts.push(config[at..i].trim());
                            at = i + 1;
                        }
                        _ => {}
                    }
                }
                parts.push(config[at..].trim());
                let smem = parts.get(2).and_then(|s| s.parse::<usize>().ok()).unwrap_or(0);
                Some((name.trim().to_string(), smem))
            })
            .collect()
    };
    let raised = |cu: &str| -> Vec<(String, usize)> {
        cu.lines()
            .filter_map(|line| {
                let inner = line.trim_start().strip_prefix("CK(cudaFuncSetAttribute(")?;
                let (name, rest) = inner.split_once(", cudaFuncAttributeMaxDynamicSharedMemorySize, ")?;
                let bytes = rest.split_once(')')?.0.trim().parse::<usize>().ok()?;
                Some((name.trim().to_string(), bytes))
            })
            .collect()
    };
    let mut programs: Vec<(String, String)> = Vec::new();
    for numerics in NUMERICS {
        for &t in ALL {
            let (spec, path) = checkpoint(t, "smem");
            let lane = compile(&spec, &path, &chunked(numerics, 4)).unwrap_or_else(|e| panic!("{} {numerics:?}: {e}", t.name()));
            programs.push((format!("{} {numerics:?}", t.name()), lane.program.cu));
        }
        for t in [Tiny::Qwen3, Tiny::GptOss, Tiny::DeepSeek2, Tiny::Qwen35] {
            // A conv's rows are its taps, too short for a block.
            let (spec, path) = wide_checkpoint(t, "smem", &|name| if name.contains("ssm_conv1d") { GgmlType::F32 } else { GgmlType::Q4K });
            let lane = compile(&spec, &path, &LaneOptions { ctx: 128, prefill: 64, ..options(numerics) })
                .unwrap_or_else(|e| panic!("{} wide {numerics:?}: {e}", t.name()));
            programs.push((format!("{} wide {numerics:?}", t.name()), lane.program.cu));
        }
    }
    let mut past = 0;
    for (what, cu) in &programs {
        let limits = raised(cu);
        for (kernel, smem) in launches(cu).into_iter().filter(|&(_, smem)| smem > 48 * 1024) {
            past += 1;
            let limit = limits.iter().filter(|(k, _)| *k == kernel).map(|&(_, b)| b).max();
            assert!(limit.is_some_and(|b| b >= smem), "{what}: `{kernel}` launches with {smem} bytes of shared memory, its limit raised to {limit:?}");
        }
    }
    assert!(past > 0, "some launch takes more than the default");
}

/// Every attention takes the same kernel with every rule held back as with the rules applied — in tiles, every head in
/// one block reading each cached row once over its chunks, or its heads a block over their chunks — so the program apart
/// computes what the fused one computes: every tiny architecture in chunks of four and the decode step, and Qwen3,
/// gpt-oss and DeepSeek-V2 wide in chunks of 64, under every numerics.
#[test]
fn every_attention_takes_the_same_kernel_with_the_rules_held_back() {
    let choices = |lane: &Lane, numerics: Q8MatKind| -> Vec<Vec<(bool, Option<usize>, usize, usize, bool)>> {
        lane.forms
            .iter()
            .map(|f| {
                f.schedule
                    .launches
                    .iter()
                    .filter_map(|l| match l {
                        Launch::Attention { per_block, splits, parallel, shared, .. } => Some((l.attends_in_tiles(numerics), *shared, *per_block, *splits, *parallel)),
                        _ => None,
                    })
                    .collect()
            })
            .collect()
    };
    let mut cases: Vec<(String, ModelSpec, PathBuf, LaneOptions)> = Vec::new();
    for numerics in NUMERICS {
        for &t in ALL {
            let (spec, path) = checkpoint(t, "held-kernels");
            cases.push((format!("{} {numerics:?}", t.name()), spec, path, chunked(numerics, 4)));
        }
        for t in [Tiny::Qwen3, Tiny::GptOss, Tiny::DeepSeek2] {
            let (spec, path) = wide_checkpoint(t, "held-kernels", &|_| GgmlType::Q4K);
            cases.push((format!("{} wide {numerics:?}", t.name()), spec, path, LaneOptions { ctx: 128, prefill: 64, ..options(numerics) }));
        }
    }
    let mut shared = 0;
    for (what, spec, path, opts) in &cases {
        let numerics = opts.knobs.activations;
        let fused = compile(spec, path, opts).unwrap_or_else(|e| panic!("{what}: {e}"));
        let apart = compile_holding(spec, path, opts, &RULES).unwrap_or_else(|e| panic!("{what} held: {e}"));
        let (a, b) = (choices(&fused, numerics), choices(&apart, numerics));
        assert_eq!(b, a, "{what}: the attentions' kernels with every rule held back");
        shared += a.iter().flatten().filter(|c| c.1.is_some()).count();
    }
    assert!(shared > 0, "the shared-row attention is among the kernels chosen");
}

/// Every rule the planner fires is one it can hold back, save the arg-max's advance (it closes the step) and the
/// activation quantizer's fusion (the `fuse_quant` knob); every rule held back fires nowhere, in every form of every
/// architecture, which then still lowers every op once.
#[test]
fn every_fusion_rule_can_be_held_back() {
    for &t in ALL {
        let (spec, path) = checkpoint(t, "held");
        for numerics in NUMERICS {
            let what = format!("{} {numerics:?}", t.name());
            let lane = compile(&spec, &path, &chunked(numerics, 4)).unwrap_or_else(|e| panic!("{what}: {e}"));
            for f in &lane.forms {
                for fusion in &f.schedule.fusions {
                    assert!(
                        RULES.contains(&fusion.rule) || ["argmax_advance", "fused_quant"].contains(&fusion.rule),
                        "{what}: `{}` is a rule the planner names",
                        fusion.rule
                    );
                }
            }
            let apart = compile_holding(&spec, &path, &chunked(numerics, 4), &RULES).unwrap_or_else(|e| panic!("{what} held: {e}"));
            assert_eq!(apart.forms.len(), lane.forms.len(), "{what}");
            for f in &apart.forms {
                let fired: Vec<&str> = f.schedule.fusions.iter().map(|x| x.rule).filter(|r| RULES.contains(r)).collect();
                assert!(fired.is_empty(), "{what} rows {}: held rules fired: {fired:?}", f.rows);
                let mut lowered: Vec<usize> = f.schedule.ops.iter().flatten().copied().collect();
                lowered.sort_unstable();
                assert_eq!(lowered, (0..f.graph.ops.len()).collect::<Vec<_>>(), "{what} rows {}: every op lowered once", f.rows);
            }
        }
    }
}

/// A projection, a gated pair or a mixture of at least 2048 output rows of at least 64 sub-blocks in a block format, met
/// one activation row at a time, takes four rows a warp (`rows_a_warp`), and a projection of four rows a warp that a
/// projection reads quantized quantizes in its epilogue (a hand-written step's projection of a projection); a narrower
/// shape, a prefill chunk (its mixture's rows run in slabs, each expert's weights read once for them) and a Q8_0
/// checkpoint keep one row a warp. The latent's query projection, read `f32` alone — by the absorption and the
/// rotation — is never quantized.
#[test]
fn the_wide_shapes_take_four_rows_a_warp() {
    let per_warp = |l: &Launch| match l {
        Launch::Gemv { per_warp, .. } | Launch::GemvGlu { per_warp, .. } | Launch::Moe { per_warp, .. } => Some(*per_warp),
        _ => None,
    };
    let (spec, path) = medium_checkpoint(Tiny::DeepSeek2, "rows", &|_| GgmlType::Q4K);
    for numerics in NUMERICS {
        let lane = compile(&spec, &path, &chunked(numerics, 4)).unwrap_or_else(|e| panic!("{numerics:?}: {e}"));
        let s = &lane.decode().schedule;
        // Each layer's query projection (16·192 of 2048) and output projection (2048 of 16·128), the dense layer's
        // gated pair and down projection (2048 of 2048), and each mixture (two experts of 1024 and the shared expert).
        assert_eq!(s.fired("rows_a_warp"), 3 + 3 + 1 + 1 + 2, "{numerics:?}: {:?}", s.fusions);
        for l in s.launches.iter().flat_map(|l| l.parts()) {
            let wide = match l {
                Launch::Gemv { n, k, .. } => *n >= 2048 && *k >= 2048,
                Launch::GemvGlu { .. } | Launch::Moe { .. } => true,
                _ => continue,
            };
            assert_eq!(per_warp(l), Some(if wide { 4 } else { 1 }), "{numerics:?}: {l:?}");
        }
        for layer in 0..3 {
            let qr = format!("qr{layer}");
            assert!(
                s.launches.iter().any(|l| matches!(l, Launch::Gemv { y, per_warp: 4, quant: false, f32: true, .. } if y.buf == qr)),
                "{numerics:?}: `{qr}` is projected `f32` alone, at four rows a warp"
            );
            assert!(
                !s.launches.iter().any(|l| matches!(l, Launch::Quant { x, .. } if *x == qr)),
                "{numerics:?}: no launch quantizes `{qr}`"
            );
        }
        // A wide projection read by another projection quantizes its result for it in its epilogue, and only there.
        let step = "## Main\nLet tok be 1.\nLet pos be 1.\nLet Emb be a 8 by 2048 tensor of Real.\n\
                    Let W1 be a 2048 by 2048 tensor of Real.\nLet W2 be a 2048 by 2048 tensor of Real.\n\
                    Let x be row tok of Emb.\nLet h be x matmul W1.\nLet y be h matmul W2.\n\
                    Let logits be rmsnorm of y.\nLet next be argmax of logits.\n";
        let fed = planned_rows_as(step, &[("W1", true), ("W2", true)], numerics, GgmlType::Q4K);
        let quantized = numerics != Q8MatKind::Dequant;
        assert!(
            fed.launches.iter().any(|l| matches!(l, Launch::Gemv { y, per_warp: 4, quant, f32, .. }
                if y.buf == "h" && *quant == quantized && *f32 == !quantized)),
            "{numerics:?}: `h` is quantized for the projection that reads it in its own projection's epilogue: {:?}",
            fed.launches
        );
        assert!(
            !fed.launches.iter().any(|l| matches!(l, Launch::Quant { x, .. } if x == "h")),
            "{numerics:?}: no launch quantizes `h` after its projection"
        );
        for f in &lane.forms[1..] {
            assert_eq!(f.schedule.fired("rows_a_warp"), 0, "{numerics:?}: a chunk of {} rows", f.rows);
            assert!(f.schedule.launches.iter().filter_map(per_warp).all(|w| w == 1), "{numerics:?}: a chunk of {} rows", f.rows);
        }
    }
    // An f32 router's row is read by four warps an expert; a quantized one by one.
    for (router, wpe) in [(GgmlType::F32, ", 4><<<"), (GgmlType::Q4K, "><<<")] {
        let (spec, path) = medium_checkpoint(Tiny::Qwen3Moe, &format!("route-{}", router.name()), &|n| {
            if n.contains("ffn_gate_inp") {
                router
            } else {
                GgmlType::Q4K
            }
        });
        let lane = compile(&spec, &path, &options(Q8MatKind::Int8Act)).unwrap();
        let fr = if router == GgmlType::F32 { 8 } else { router.id() };
        let rf = u32::from(router == GgmlType::F32);
        assert!(lane.program.cu.contains(&format!("lk_moe_route<1, {rf}, {fr}{wpe}")), "a {} router", router.name());
    }
    let (spec8, path8) = medium_checkpoint(Tiny::DeepSeek2, "rows-q8", &|_| GgmlType::Q8_0);
    let q8 = compile(&spec8, &path8, &options(Q8MatKind::Int8Act)).unwrap();
    assert_eq!(q8.decode().schedule.fired("rows_a_warp"), 0, "a Q8_0 row already moves sixteen bytes a lane");
    let tiny = compiled(Tiny::DeepSeek2, Q8MatKind::Int8Act, "rows-tiny");
    assert_eq!(tiny.decode().schedule.fired("rows_a_warp"), 0, "a tiny model's matrices are too narrow");
}

/// Four rows a warp is a step of the schedule a plan can hold back: the shapes that take them
/// ([`the_wide_shapes_take_four_rows_a_warp`]), with `rows_a_warp` held, keep one row a warp in every launch of every
/// form under every numerics, and no firing of it is recorded.
#[test]
fn the_wide_shapes_keep_one_row_a_warp_with_rows_a_warp_held() {
    let (spec, path) = medium_checkpoint(Tiny::DeepSeek2, "rows-held", &|_| GgmlType::Q4K);
    for numerics in NUMERICS {
        let lane = compile_holding(&spec, &path, &chunked(numerics, 4), &["rows_a_warp"]).unwrap_or_else(|e| panic!("{numerics:?}: {e}"));
        for f in &lane.forms {
            assert_eq!(f.schedule.fired("rows_a_warp"), 0, "{numerics:?} {}: rows_a_warp held fired", f.tag());
            for l in f.schedule.launches.iter().flat_map(|l| l.parts()) {
                let per_warp = match l {
                    Launch::Gemv { per_warp, .. } | Launch::GemvGlu { per_warp, .. } | Launch::Moe { per_warp, .. } | Launch::GemmParts { per_warp, .. } => *per_warp,
                    _ => continue,
                };
                assert_eq!(per_warp, 1, "{numerics:?} {}: rows_a_warp held, a launch takes {per_warp} rows a warp: {l:?}", f.tag());
            }
        }
    }
}

/// A decode step's launches by layer, each launch named by its kernel: the layer of a launch is that of the first op
/// it computes, read off the trailing digits the one program names every layer's tensors with (`qr3`, `ao3`); the
/// embedding's launches before the first layer and the head's after the last are named so.
fn census_by_layer(form: &logicaffeine_compile::codegen_cuda::lane::Form) -> String {
    let layer_of = |op: usize| -> Option<usize> {
        let out = &form.graph.ops[op].out;
        let digits: String = out.chars().rev().take_while(|c| c.is_ascii_digit()).collect::<Vec<_>>().into_iter().rev().collect();
        (!digits.is_empty()).then(|| digits.parse().expect("digits"))
    };
    let mut lines: Vec<(String, Vec<&str>)> = Vec::new();
    let mut seen_layer = false;
    for (l, ops) in form.schedule.launches.iter().zip(&form.schedule.ops) {
        let group = match ops.first().and_then(|&o| layer_of(o)) {
            Some(n) => {
                seen_layer = true;
                format!("layer {n}")
            }
            // A launch computing no lifted op — a quantize — runs with the launch before it.
            None if ops.is_empty() => lines.last().map_or_else(|| "embedding".to_string(), |(g, _)| g.clone()),
            None if seen_layer => "head".to_string(),
            None => "embedding".to_string(),
        };
        match lines.last_mut() {
            Some((g, kinds)) if *g == group => kinds.push(kind(l)),
            _ => lines.push((group, vec![kind(l)])),
        }
    }
    let total: usize = lines.iter().map(|(_, k)| k.len()).sum();
    let mut out = format!("{total} launches a token\n");
    for (g, kinds) in lines {
        out += &format!("{g} ({}): {}\n", kinds.len(), kinds.join(" "));
    }
    out
}

/// Each real model's decode step launches what its census says (K-01 of benchmarks/FUSIONS_WE_NEED.md): Gemma-3-1B,
/// Llama-3.2-1B, Qwen3-8B, Qwen3.5-0.8B, DeepSeek-V2-Lite and gpt-oss-20b, each layer's launches in order under the
/// default knobs, held to `tests/fixtures/lane_census/<model>.txt`. A change that lowers a count re-records its model
/// on purpose (`LOGOS_UPDATE_FIXTURES=1`), saying why; a checkpoint absent from `models/` is skipped unless
/// `LOGOS_REQUIRE_TOOLS` names it.
#[test]
fn every_decode_layer_launches_what_its_census_says() {
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/lane_census");
    let update = std::env::var("LOGOS_UPDATE_FIXTURES").is_ok_and(|v| v == "1");
    let mut held = 0;
    for model in ["gemma3-1b", "llama3.2-1b", "qwen3-8b", "qwen3.5-0.8b", "deepseek-v2-lite", "gpt-oss-20b"] {
        if !common::require_tool(&format!("checkpoint:{model}")) {
            continue;
        }
        let path = common::checkpoint(model).expect("the checkpoint");
        let spec = ModelSpec::from_gguf(&Gguf::open(&path).unwrap()).unwrap_or_else(|e| panic!("{model}: {e:?}"));
        let lane = compile(&spec, &path, &LaneOptions { ctx: 512, knobs: CudaKnobs::default(), prefill: 1 }).unwrap_or_else(|e| panic!("{model}: {e}"));
        let census = census_by_layer(lane.decode());
        let file = fixtures.join(format!("{model}.txt"));
        if update {
            std::fs::create_dir_all(&fixtures).unwrap();
            std::fs::write(&file, &census).unwrap();
        } else {
            let want = std::fs::read_to_string(&file).unwrap_or_else(|e| panic!("{}: {e} — record it with LOGOS_UPDATE_FIXTURES=1", file.display()));
            assert_eq!(census, want, "{model}: the decode step's launches moved; a change that lowers a count re-records it on purpose");
        }
        held += 1;
    }
    eprintln!("{held} model censuses held");
}

/// A decode layer's output projection and its down projection each finish the residual add and the next norm in their
/// last-arriving block (PARITY-GEMV-NORM-EPILOGUE, K-05 and K-07): on Llama, Qwen3 and Gemma-3 — Gemma's post-norm and
/// pre-norm both in the one fold — under every numerics, two folds a layer, the last layer's down projection folding the
/// step's final norm; each fold's norm reads the projection's own result, and no norm launch takes a projection's
/// result after it. Held back, every fold is two launches again.
#[test]
fn a_projection_feeding_an_add_and_norm_finishes_them_in_its_last_block() {
    for t in [Tiny::Llama, Tiny::Qwen3, Tiny::Gemma3] {
        for numerics in NUMERICS {
            let what = format!("{} {numerics:?}", t.name());
            let lane = compiled(t, numerics, &format!("fold-{}", t.name()));
            let s = &lane.decode().schedule;
            let folds: Vec<&Launch> = s.launches.iter().filter(|l| kind(l) == "gemv_norm").collect();
            assert_eq!(folds.len(), 2 * t.spec(CTX).layers.len(), "{what}: two folds a layer: {:?}", s.census());
            for f in &folds {
                let Launch::Fold { first, then } = f else { unreachable!() };
                let (Launch::Gemv { y, .. }, Launch::Norm { x, add, .. }) = (first.as_ref(), then.as_ref()) else {
                    panic!("{what}: a fold of a projection and a norm: {f:?}")
                };
                assert!(*x == y.buf || add.as_deref() == Some(y.buf.as_str()), "{what}: the fold's norm reads its projection's result: {f:?}");
            }
            for (i, l) in s.launches.iter().enumerate() {
                if let Launch::Norm { x, add, .. } = l {
                    let fed = s.launches[..i].iter().any(|p| matches!(p, Launch::Gemv { y, .. } if *x == y.buf || add.as_deref() == Some(y.buf.as_str())));
                    assert!(!fed, "{what}: a norm launch takes a projection's result after it: {l:?}");
                }
            }
            let (spec, path) = checkpoint(t, &format!("fold-held-{}", t.name()));
            let apart = compile_holding(&spec, &path, &options(numerics), &["gemv_norm_epilogue"]).unwrap_or_else(|e| panic!("{what} held: {e}"));
            let held = &apart.decode().schedule;
            assert!(!held.launches.iter().any(|l| kind(l) == "gemv_norm"), "{what}: held, no projection folds its norm");
            assert_eq!(held.launches.len(), s.launches.len() + folds.len(), "{what}: held, each fold is its two launches");
        }
    }
}

/// The mixtures of a spec: its layers whose feed-forward is a mixture of experts, and whether they take a shared expert.
fn mixtures(spec: &ModelSpec) -> (usize, bool) {
    let moes: Vec<_> = spec.layers.iter().filter_map(|l| match &l.ffn {
        Ffn::Moe(m) => Some(m.shared_experts > 0),
        _ => None,
    }).collect();
    (moes.len(), moes.iter().any(|s| *s))
}

/// Every launch of `s` that reads `t` but the folds that finish it.
fn plain_readers<'a>(s: &'a Schedule, t: &str) -> Vec<&'a Launch> {
    s.launches.iter().filter(|l| l.finished_mixture().is_none() && l.reads().contains(&t)).collect()
}

/// The mixtures `s` launches, alone or inside the folds that finish them, in order.
fn mixture_launches(s: &Schedule) -> Vec<&Launch> {
    s.launches.iter().filter_map(|l| if matches!(l, Launch::Moe { .. }) { Some(l) } else { l.finished_mixture().map(|(m, ..)| m) }).collect()
}

/// A decode step's mixture finishes the residual add and the next norm in its down projection's last-arriving block
/// (PARITY-MOE-DEFERRED-FINALIZE, K-18) where that block is at least the norm's width (`kernels::block_holds_its_norm`):
/// on Qwen3-MoE, gpt-oss and DeepSeek-V2 — its shared expert inside the mixture — under every numerics, every mixture
/// layer's mixture is one fold with the norm that adds its result, the last layer's with the step's final norm; each
/// fold's mixture is the one the program apart launches; the fold counts its down blocks in (its `gc` count); no launch
/// but the fold reads a mixture's result. Held back, every fold is its mixture and its norm again. DeepSeek-V2 at 2048
/// wide in Q4_K, its mixture four rows a warp, has down blocks of six warps where the norm takes 32: every mixture and its
/// norm run apart, the decode step the one the program with the rule held runs.
#[test]
fn a_decode_mixture_adds_and_norms_in_its_last_down_block() {
    let cases: Vec<(String, Tiny, ModelSpec, PathBuf)> = [Tiny::Qwen3Moe, Tiny::GptOss, Tiny::DeepSeek2]
        .into_iter()
        .map(|t| {
            let (spec, path) = checkpoint(t, "moe-fold");
            (t.name().to_string(), t, spec, path)
        })
        .collect();
    for (name, t, spec, path) in &cases {
        let t = *t;
        let (count, _) = mixtures(spec);
        for numerics in NUMERICS {
            let what = format!("{name} {numerics:?}");
            let lane = compile(spec, path, &options(numerics)).unwrap_or_else(|e| panic!("{what}: {e}"));
            let s = &lane.decode().schedule;
            let folds: Vec<&Launch> = s.launches.iter().filter(|l| kind(l) == "moe_norm").collect();
            assert_eq!(folds.len(), count, "{what}: a fold a mixture layer: {:?}", s.census());
            assert!(!s.launches.iter().any(|l| kind(l) == "moe"), "{what}: no mixture launches alone: {:?}", s.census());
            for f in &folds {
                let Some((Launch::Moe { y, rows: 1, shared, .. }, Launch::Norm { x, add, rows: 1, .. }, None)) = f.finished_mixture() else {
                    panic!("{what}: a fold of a step's mixture, its shared expert inside, and the norm of its result: {f:?}")
                };
                assert_eq!(shared.is_some(), t == Tiny::DeepSeek2, "{what}: the shared expert is inside the mixture: {f:?}");
                assert!(*x == y.buf || add.as_deref() == Some(y.buf.as_str()), "{what}: the fold's norm reads its mixture's result: {f:?}");
                assert!(plain_readers(s, &y.buf).is_empty(), "{what}: only the fold reads `{}`", y.buf);
                assert!(scratch(f, numerics).iter().any(|sc| sc.tag == "gc" && sc.counter), "{what}: the fold counts its down blocks in");
            }
            assert!(s.fusions.iter().filter(|x| x.rule == "moe_deferred_finalize").count() == count, "{what}: the rule fires a mixture layer");
            let apart = compile_holding(spec, path, &options(numerics), &["moe_deferred_finalize"]).unwrap_or_else(|e| panic!("{what} held: {e}"));
            let held = &apart.decode().schedule;
            assert!(!held.launches.iter().any(|l| kind(l) == "moe_norm"), "{what}: held, no mixture folds its norm");
            assert_eq!(held.launches.len(), s.launches.len() + folds.len(), "{what}: held, each fold is its two launches");
            assert_eq!(mixture_launches(s), mixture_launches(held), "{what}: each fold's mixture is the one the program apart launches");
        }
    }
    let (spec, path) = medium_checkpoint(Tiny::DeepSeek2, "moe-fold-q4k", &|_| GgmlType::Q4K);
    for numerics in NUMERICS {
        let what = format!("deepseek2 medium Q4_K {numerics:?}");
        let lane = compile(&spec, &path, &options(numerics)).unwrap_or_else(|e| panic!("{what}: {e}"));
        let apart = compile_holding(&spec, &path, &options(numerics), &["moe_deferred_finalize"]).unwrap_or_else(|e| panic!("{what} held: {e}"));
        let s = &lane.decode().schedule;
        assert!(!s.launches.iter().any(|l| kind(l) == "moe_norm"), "{what}: no mixture norms in a down block narrower than its norm: {:?}", s.census());
        assert_eq!(s.fired("moe_deferred_finalize"), 0, "{what}: the rule fires on no decode mixture");
        assert_eq!(s.launches, apart.decode().schedule.launches, "{what}: the decode step is the one the program with the rule held runs");
        assert!(mixture_launches(s).iter().all(|m| matches!(m, Launch::Moe { per_warp: 4, .. })), "{what}: the mixtures take four rows a warp");
    }
}

/// A chunk's mixture and a batch's combine their rows' choices and finish the residual add and the next norm in one
/// launch (PARITY-MOE-DEFERRED-FINALIZE): on Qwen3-MoE, gpt-oss and DeepSeek-V2, chunks of 32 down to 2 — on the tensor
/// cores past 16 rows under the quantized numerics — and a batch of two sequences, under every numerics, every mixture
/// an add and norm reads is one fold whose combine adds the residual, DeepSeek's shared expert's product summed with the
/// mixture first, and norms, a block of the norm's threads a row and no count: every mixture of the batch, and every
/// one of a chunk but its last layer's, whose result the residual add alone reads, the head norming one row of the sum;
/// no launch but the fold reads the mixture's result or its sum. Held back, every fold is its launches again: the
/// mixture, DeepSeek's sum and the norm.
#[test]
fn a_wide_chunks_expert_combine_folds_into_the_next_add_and_norm() {
    for t in [Tiny::Qwen3Moe, Tiny::GptOss, Tiny::DeepSeek2] {
        let (spec, path) = checkpoint(t, "moe-chunk-fold");
        let (count, with_shared) = mixtures(&spec);
        for numerics in NUMERICS {
            let what = format!("{} {numerics:?}", t.name());
            let opts = LaneOptions { ctx: 64, ..chunked(numerics, 32) };
            let lane = compile_batch(&spec, &path, &opts, 2).unwrap_or_else(|e| panic!("{what}: {e}"));
            let apart = compile_batch_holding(&spec, &path, &opts, 2, &["moe_deferred_finalize"]).unwrap_or_else(|e| panic!("{what} held: {e}"));
            let wide = lane.forms.iter().filter(|f| f.rows > 1).flat_map(|f| f.schedule.launches.iter()).any(|l| l.on_tensor_cores(numerics) && matches!(l.parts()[0], Launch::Moe { .. }));
            assert_eq!(wide, numerics != Q8MatKind::Dequant, "{what}: the chunk of 32's experts run on the tensor cores under the quantized numerics");
            let mut forms = 0;
            for (f, h) in lane.forms.iter().zip(&apart.forms).filter(|(f, _)| f.rows > 1 || f.kind == FormKind::Batch) {
                let (s, held) = (&f.schedule, &h.schedule);
                let what = format!("{what}, {:?} of {} rows", f.kind, f.rows);
                let folds: Vec<&Launch> = s.launches.iter().filter(|l| kind(l) == "moe_norm").collect();
                let last_alone = usize::from(f.kind == FormKind::Prefill);
                assert_eq!(folds.len(), count - last_alone, "{what}: a fold a mixture layer, a chunk's last but: {:?}", s.census());
                let alone: Vec<&Launch> = s.launches.iter().filter(|l| matches!(l, Launch::Moe { .. })).collect();
                assert_eq!(alone.len(), last_alone, "{what}: the mixtures launched alone: {:?}", s.census());
                for m in &alone {
                    let Launch::Moe { y, .. } = m else { unreachable!() };
                    // The residual add, or DeepSeek's sum with its shared expert first, the residual add after.
                    let readers: Vec<&Launch> = s.launches.iter().filter(|l| l.reads().contains(&y.buf.as_str())).collect();
                    assert!(
                        matches!(readers[..], [Launch::Binary { op: ElementwiseOp::Add, gate: None, .. }]),
                        "{what}: the last layer's mixture is read by an add alone: {readers:?}"
                    );
                }
                let mut summed = 0;
                for l in &folds {
                    let Some((Launch::Moe { y, rows, shared, .. }, Launch::Norm { x, add, rows: normed, .. }, sum)) = l.finished_mixture() else {
                        panic!("{what}: a fold of a mixture and the norm of its result: {l:?}")
                    };
                    assert_eq!((*rows, *normed), (f.rows, f.rows), "{what}: the fold combines and norms every row");
                    assert_eq!(sum.is_some() || shared.is_some(), with_shared, "{what}: a shared expert is inside the mixture or summed with it in the fold: {l:?}");
                    let result = sum.map_or(y.buf.as_str(), |(_, to)| to);
                    assert!(x == result || add.as_deref() == Some(result), "{what}: the fold's norm reads the mixture's result: {l:?}");
                    for t in [Some(y.buf.as_str()), sum.map(|(_, to)| to)].into_iter().flatten() {
                        assert!(plain_readers(s, t).is_empty(), "{what}: only the fold reads `{t}`");
                    }
                    assert!(!scratch(l, numerics).iter().any(|sc| sc.tag == "gc"), "{what}: a chunk's fold counts nothing");
                    summed += usize::from(sum.is_some());
                }
                assert!(!held.launches.iter().any(|l| kind(l) == "moe_norm"), "{what}: held, no mixture folds its norm");
                assert_eq!(held.launches.len(), s.launches.len() + folds.len() + summed, "{what}: held, each fold is its launches");
                assert_eq!(mixture_launches(s), mixture_launches(held), "{what}: each fold's mixture is the one the program apart launches, its router's split included");
                forms += 1;
            }
            assert!(forms >= 3, "{what}: chunks and the batch: {forms}");
        }
    }
}

/// A delta rule the step walks a row at a time norms each head in the step kernel's last block for that head (K-10):
/// on Qwen3.5 and at its wide width, heads of 32 and 128, under every numerics, the decode step, a batch of two
/// sequences and the chunks of 32 down to 2 launch each such rule as `lk_delta_step` given the norm's operands — one
/// kernel for the update, the gated norm and its quantize, a count a head (a sequence's own in the batch) — and launch
/// `lk_delta_norm` only after a rule taken in chunks.
#[test]
fn a_stepped_delta_rule_norms_each_head_in_its_last_block() {
    let wide = (Tiny::Qwen35.wide(CTX), dir().join("qwen35-wide-delta-norm.gguf"));
    tiny_models::write_checkpoint(&wide.0, &wide.1);
    for (name, (spec, path)) in [("qwen35", checkpoint(Tiny::Qwen35, "delta-norm")), ("qwen35 wide", wide)] {
        for numerics in NUMERICS {
            let what = format!("{name} {numerics:?}");
            let lane = compile_batch(&spec, &path, &LaneOptions { ctx: 64, ..chunked(numerics, 32) }, 2).unwrap_or_else(|e| panic!("{what}: {e}"));
            let (mut stepped, mut in_chunks) = (0, 0);
            for f in &lane.forms {
                for l in f.schedule.launches.iter().filter(|l| matches!(l, Launch::Delta { .. })) {
                    let Launch::Delta { hv, seqs, .. } = l else { unreachable!() };
                    let counts: Vec<usize> = scratch(l, numerics).into_iter().filter(|sc| sc.tag == "dc").map(|sc| sc.bytes).collect();
                    if l.delta_in_chunks(numerics) {
                        assert!(counts.is_empty(), "{what}, {} rows: a rule taken in chunks counts nothing: {l:?}", f.rows);
                        in_chunks += 1;
                    } else {
                        assert_eq!(counts, vec![hv * seqs * 4], "{what}, {} rows: a count a head, a sequence's own: {l:?}", f.rows);
                        stepped += 1;
                    }
                }
            }
            assert!(stepped > 0, "{what}: the step walks its rules a row at a time");
            let cu = &lane.program.cu;
            // Each launch of `kernel`, as the commas of its template arguments: `lk_delta_step` given the norm's operands takes
            // six arguments, the bare step three at most.
            let launches = |kernel: &str| -> Vec<usize> {
                cu.match_indices(&format!("{kernel}<"))
                    .filter_map(|(at, _)| cu[at..].split("><<<").next().filter(|args| !args.contains('\n')))
                    .map(|args| args.matches(',').count())
                    .collect()
            };
            assert_eq!(launches("lk_delta_step"), vec![5; stepped], "{what}: every stepped rule launches the step with its norm, and no bare step");
            assert_eq!(launches("lk_delta_norm").len(), in_chunks, "{what}: a norm kernel only after a rule taken in chunks");
        }
    }
}

/// The program's profile names the matrix bytes each launch streams (K-08's measure: `--profile`'s rate is those bytes
/// over its time): over a decode step, its launches' bytes are exactly the checkpoint's matrices as the device keeps
/// them — every matrix once at its rows' bytes (`kernels::row_bytes`), an expert stack at the share of its experts a
/// token routes to, the token table's one row gathered (and the whole table where it is the head too) — for Llama,
/// Qwen3 and Gemma-3 in Q8_0, Llama in BF16 and in MXFP4 (whose rows the upload regroups, so the device keeps more bytes
/// than the checkpoint), and Qwen3's mixture, under every numerics.
#[test]
fn every_profiled_launch_names_the_matrix_bytes_it_streams() {
    use logicaffeine_compile::codegen_cuda::lane::kernels::row_bytes;
    let array = |cu: &str, name: &str| -> Vec<u64> {
        let at = cu.find(&format!("{name}[] = {{")).unwrap_or_else(|| panic!("the profile table names no {name}"));
        let body = &cu[at..];
        let body = &body[body.find('{').unwrap() + 1..body.find('}').unwrap()];
        body.split(',').map(|v| v.trim().trim_end_matches("ULL")).filter(|v| !v.is_empty()).map(|v| v.parse().unwrap()).collect()
    };
    let cases: Vec<(&str, ModelSpec, Option<GgmlType>)> = vec![
        ("llama", Tiny::Llama.spec(CTX), None),
        ("qwen3", Tiny::Qwen3.spec(CTX), None),
        ("gemma3", Tiny::Gemma3.spec(CTX), None),
        ("llama bf16", Tiny::Llama.spec(CTX), Some(GgmlType::Bf16)),
        ("llama mxfp4", Tiny::Llama.spec(CTX), Some(GgmlType::Mxfp4)),
        ("qwen3moe", Tiny::Qwen3Moe.spec(CTX), None),
    ];
    for (name, spec, ty) in cases {
        let path = dir().join(format!("profile-bytes-{}.gguf", name.replace(' ', "-")));
        match ty {
            Some(ty) => tiny_models::write_checkpoint_in(&spec, &path, ty),
            None => tiny_models::write_checkpoint(&spec, &path),
        }
        let g = Gguf::open(&path).unwrap();
        let untied = g.info("output.weight").is_some();
        let (experts, routed) = (spec.layers.iter().find_map(|l| match &l.ffn {
            Ffn::Moe(m) => Some((m.experts, m.top_k)),
            _ => None,
        }))
        .unwrap_or((0, 0));
        // The checkpoint's matrices as the device keeps them, a decode step's share of each.
        let mut want = 0u64;
        for (t, dims, id) in g.tensor_names() {
            if dims.len() < 2 {
                continue;
            }
            let ty = GgmlType::from_id(id).unwrap();
            let rows: u64 = dims[1..].iter().product();
            let full = rows * row_bytes(ty, dims[0] as usize) as u64;
            want += match t.as_str() {
                "token_embd.weight" if untied => row_bytes(ty, dims[0] as usize) as u64,
                "token_embd.weight" => full + row_bytes(ty, dims[0] as usize) as u64,
                _ if t.contains("_exps.") => full * routed as u64 / experts as u64,
                _ => full,
            };
        }
        for numerics in NUMERICS {
            let what = format!("{name} {numerics:?}");
            let lane = compile(&spec, &path, &options(numerics)).unwrap_or_else(|e| panic!("{what}: {e}"));
            let cu = &lane.program.cu;
            let (form, bytes) = (array(cu, "PROF_ROWS"), array(cu, "PROF_BYTES"));
            assert_eq!(form.len(), bytes.len(), "{what}: a launch's bytes beside its form");
            let step: u64 = form.iter().zip(&bytes).filter(|(f, _)| **f == 1).map(|(_, b)| *b).sum();
            assert_eq!(step, want, "{what}: the decode step's launches stream the checkpoint's matrices once: {:?}", lane.decode().schedule.census());
        }
    }
}

/// A latent attention's launches name the matrix bytes they stream, as every launch does
/// (`every_profiled_launch_names_the_matrix_bytes_it_streams`): DeepSeek-V2 at its medium shape — its real heads, 16 of
/// 128 unrotated and 64 rotated values and a latent of 512 — every matrix of whole 256-value rows in Q4_K, and again in
/// Q6_K (grouped at upload), the rest in `f32`, under every numerics: the decode step's launches stream the checkpoint's
/// matrices once as the device keeps them, its absorbed per-head products each head's rows of the latent's expansion —
/// `b` rows of the head's `a` inputs where the matrix's rows are its outputs, `a` rows of `b` where they are its inputs.
/// Counted the other way round, a head's 512 latent values into its 128 outputs asked for rows of 128 values, which no
/// K-quant block holds, and DeepSeek-V2-Lite no longer compiled.
#[test]
fn a_latent_attentions_launches_name_the_matrix_bytes_they_stream() {
    use logicaffeine_compile::codegen_cuda::lane::kernels::row_bytes;
    let array = |cu: &str, name: &str| -> Vec<u64> {
        let at = cu.find(&format!("{name}[] = {{")).unwrap_or_else(|| panic!("the profile table names no {name}"));
        let body = &cu[at..];
        let body = &body[body.find('{').unwrap() + 1..body.find('}').unwrap()];
        body.split(',').map(|v| v.trim().trim_end_matches("ULL")).filter(|v| !v.is_empty()).map(|v| v.parse().unwrap()).collect()
    };
    let spec = Tiny::DeepSeek2.medium(CTX);
    for ty in [GgmlType::Q4K, GgmlType::Q6K] {
        let path = dir().join(format!("profile-bytes-deepseek2-medium-{}.gguf", ty.name()));
        let matrices: BTreeSet<String> = tiny_models::tensors(&spec)
            .into_iter()
            .filter(|(n, dims)| dims.len() >= 2 && dims[0] % 256 == 0 && !n.contains("norm"))
            .map(|(n, _)| n)
            .collect();
        tiny_models::write_checkpoint_as(&spec, &path, &|n| if matrices.contains(n) { ty } else { GgmlType::F32 });
        let g = Gguf::open(&path).unwrap();
        let untied = g.info("output.weight").is_some();
        let (experts, routed) = spec
            .layers
            .iter()
            .find_map(|l| match &l.ffn {
                Ffn::Moe(m) => Some((m.experts, m.top_k)),
                _ => None,
            })
            .unwrap_or((0, 0));
        let mut want = 0u64;
        for (t, dims, id) in g.tensor_names() {
            if dims.len() < 2 {
                continue;
            }
            let format = GgmlType::from_id(id).unwrap();
            let rows: u64 = dims[1..].iter().product();
            let full = rows * row_bytes(format, dims[0] as usize) as u64;
            want += match t.as_str() {
                "token_embd.weight" if untied => row_bytes(format, dims[0] as usize) as u64,
                "token_embd.weight" => full + row_bytes(format, dims[0] as usize) as u64,
                _ if t.contains("_exps.") => full * routed as u64 / experts as u64,
                _ => full,
            };
        }
        for numerics in NUMERICS {
            let what = format!("deepseek2 medium {} {numerics:?}", ty.name());
            let lane = compile(&spec, &path, &options(numerics)).unwrap_or_else(|e| panic!("{what}: {e}"));
            let cu = &lane.program.cu;
            let (form, bytes) = (array(cu, "PROF_ROWS"), array(cu, "PROF_BYTES"));
            assert_eq!(form.len(), bytes.len(), "{what}: a launch's bytes beside its form");
            let step: u64 = form.iter().zip(&bytes).filter(|(f, _)| **f == 1).map(|(_, b)| *b).sum();
            assert_eq!(step, want, "{what}: the decode step's launches stream the checkpoint's matrices once: {:?}", lane.decode().schedule.census());
        }
    }
}

/// A one-row gated delta layer's every input projection is one launch with its convolutions, SiLU, L2 norms and gates
/// in its epilogue (PARITY-GDN-INPUT-EPILOGUE, K-09): on every case of `tiny_models::gated_delta_cases`, under every
/// numerics and with `fuse_quant` off, the decode step launches each recurrent layer's inputs as one fold (`gemv_gdn`)
/// — its query's, key's, values' and output gate's products, and its gates' in Q8_0, as one GEMV in parts of the
/// layer's activation; its query, key and value convolutions through the SiLU over those parts' outputs, the query and
/// key heads left for the step's delta rule to norm (`delta_qk_norm`); then the gates, from their parts in the GEMV, or
/// from the activation by their `f32` projections — and no convolution, gate or projection of the layer apart, by
/// `lk_gemv_gdn`. Held back, each fold is its three launches again. A prefill chunk's and a batch's products are of
/// many rows and stay apart.
#[test]
fn a_gated_delta_layers_input_projections_are_one_launch_with_conv_and_gates() {
    for (name, spec, path, f32_gates) in tiny_models::gated_delta_cases(&dir(), "gdn-inputs", CTX) {
        let recurrent = spec.layers.iter().filter(|l| matches!(l.mixer, Mixer::GatedDelta(_))).count();
        assert!(recurrent > 0, "{name}: a hybrid with recurrent layers");
        for (numerics, fuse_quant) in NUMERICS.map(|n| (n, true)).into_iter().chain([(Q8MatKind::Int8Act, false)]) {
            let what = format!("{name} {numerics:?} fuse_quant {fuse_quant}");
            let opts = LaneOptions { prefill: 4, knobs: CudaKnobs { activations: numerics, fuse_quant, ..CudaKnobs::default() }, ..options(numerics) };
            let lane = compile(&spec, &path, &opts).unwrap_or_else(|e| panic!("{what}: {e}"));
            let s = &lane.decode().schedule;
            assert_eq!(s.fired("gdn_input_epilogue"), recurrent, "{what}: one a recurrent layer: {:?}", s.fusions);
            let folds: Vec<&Launch> = s.launches.iter().filter(|l| kind(l) == "gemv_gdn").collect();
            assert_eq!(folds.len(), recurrent, "{what}: a launch a recurrent layer: {:?}", s.census());
            for f in folds {
                let Launch::Fold { first, then: gates } = f else { panic!("{what}: a fold: {f:?}") };
                let Launch::Fold { first: products, then: convs } = first.as_ref() else { panic!("{what}: the products, then their convolutions: {f:?}") };
                let (Launch::GemmParts { parts, m: 1, .. }, Launch::ConvParts { parts: convs, .. }) = (products.as_ref(), convs.as_ref()) else {
                    panic!("{what}: one row's GEMV in parts and its convolutions in parts: {f:?}")
                };
                assert_eq!(parts.len(), if f32_gates { 4 } else { 6 }, "{what}: the query, key, values, output gate{} in one GEMV: {f:?}", if f32_gates { "" } else { " and both gates" });
                assert_eq!(convs.iter().map(|c| c.l2).collect::<Vec<_>>(), [false, false, false], "{what}: no head normed in the convolutions, the delta rule norms the query and key: {f:?}");
                let made = |t: &str| parts.iter().any(|p| p.y.own() == Some(t));
                assert!(convs.iter().all(|c| made(&c.x)), "{what}: each convolution reads a part's output: {f:?}");
                match gates.as_ref() {
                    Launch::GateProj { k, heads, parts, .. } => {
                        assert!(f32_gates, "{what}: Q8_0 gates take their rows in the GEMV: {f:?}");
                        let cut = logicaffeine_compile::codegen_cuda::lane::kernels::gemv_f32_parts(*heads, *k, opts.knobs.chip.sms);
                        assert_eq!(*parts, cut, "{what}: the gates' blocks cut each dot as their projections apart do: {f:?}");
                    }
                    Launch::Gates { br, ar, .. } => assert!(!f32_gates && made(br) && made(ar), "{what}: the gates read their parts of the GEMV: {f:?}"),
                    other => panic!("{what}: the gates close the fold: {other:?}"),
                }
            }
            let apart = s.launches.iter().filter(|l| matches!(l, Launch::Conv { .. } | Launch::ConvParts { .. } | Launch::Gates { .. } | Launch::GateProj { .. } | Launch::L2Norm { .. })).count();
            assert_eq!(apart, 0, "{what}: no convolution or gate apart: {:?}", s.census());
            assert!(lane.program.cu.contains("lk_gemv_gdn<"), "{what}: by the GEMV with its gated delta inputs");
            let chunk = lane.chunk(4).unwrap_or_else(|| panic!("{what}: the chunk of 4")).schedule.clone();
            assert_eq!(chunk.fired("gdn_input_epilogue"), 0, "{what}: a chunk's products over its rows stay apart");
            let held = compile_holding(&spec, &path, &opts, &["gdn_input_epilogue"]).unwrap_or_else(|e| panic!("{what} held: {e}"));
            let h = &held.decode().schedule;
            assert!(!h.launches.iter().any(|l| kind(l) == "gemv_gdn"), "{what} held: no fold");
            assert_eq!(h.launches.len(), s.launches.len() + 2 * recurrent, "{what} held: each fold is its three launches: {:?}", h.census());
        }
        let batch = compile_batch(&spec, &path, &options(Q8MatKind::Int8Act), 2).unwrap_or_else(|e| panic!("{name} batch: {e}"));
        assert_eq!(batch.batch().unwrap().schedule.fired("gdn_input_epilogue"), 0, "{name}: a batch's products are of its sequences' rows");
    }
}

/// A gated delta layer whose convolution takes more taps than its products' launch copies in while the row's dot
/// streams (`kernels::GDN_TAPS`, four) keeps its inputs' launches apart: Qwen3.5 tiny with five taps compiles, its
/// decode step folding no layer's inputs, refused by name.
#[test]
fn a_gated_delta_layer_of_five_taps_keeps_its_inputs_apart() {
    use logicaffeine_compile::codegen_cuda::lane::kernels::GDN_TAPS;
    let mut spec = Tiny::Qwen35.spec(CTX);
    for layer in &mut spec.layers {
        if let Mixer::GatedDelta(g) = &mut layer.mixer {
            g.conv_taps = GDN_TAPS + 1;
        }
    }
    let path = dir().join("qwen35-five-taps.gguf");
    tiny_models::write_checkpoint(&spec, &path);
    for numerics in NUMERICS {
        let lane = compile(&spec, &path, &options(numerics)).unwrap_or_else(|e| panic!("{numerics:?}: {e}"));
        let s = &lane.decode().schedule;
        assert_eq!(s.fired("gdn_input_epilogue"), 0, "{numerics:?}: no layer's inputs fold: {:?}", s.census());
        assert!(!s.launches.iter().any(|l| kind(l) == "gemv_gdn"), "{numerics:?}: no inputs' launch: {:?}", s.census());
    }
}

/// A step's delta rule norms its query and key heads itself (`delta_qk_norm`): on every case of
/// `tiny_models::gated_delta_cases`, under every numerics, the decode step's delta rules take the L2 norm's epsilon,
/// the convolutions in their layer's inputs' launch norm no head, and the rule fires once a recurrent layer; held
/// back, and with the inputs' launch held (`gdn_input_epilogue`, each convolution's own launch a block a head), the
/// convolutions norm the query and key heads and the steps norm none. A chunk's delta rule reads heads its convolution
/// normed.
#[test]
fn a_steps_delta_rule_norms_its_query_and_key_heads() {
    for (name, spec, path, _) in tiny_models::gated_delta_cases(&dir(), "delta-qk", CTX) {
        let recurrent = spec.layers.iter().filter(|l| matches!(l.mixer, Mixer::GatedDelta(_))).count();
        for numerics in NUMERICS {
            let opts = LaneOptions { prefill: 4, ..options(numerics) };
            for held in [&[][..], &["gdn_input_epilogue"][..], &["delta_qk_norm"][..]] {
                let what = format!("{name} {numerics:?} holding {held:?}");
                let lane = compile_holding(&spec, &path, &opts, held).unwrap_or_else(|e| panic!("{what}: {e}"));
                let s = &lane.decode().schedule;
                let moved = held.is_empty();
                assert_eq!(s.fired("delta_qk_norm"), if moved { recurrent } else { 0 }, "{what}: {:?}", s.fusions);
                let deltas: Vec<&Launch> = s.launches.iter().filter(|l| matches!(l, Launch::Delta { .. })).collect();
                assert_eq!(deltas.len(), recurrent, "{what}: a step a recurrent layer");
                for d in deltas {
                    let Launch::Delta { qk, .. } = d else { unreachable!() };
                    assert_eq!(qk.is_some(), moved, "{what}: {d:?}");
                }
                let normed: usize = s
                    .launches
                    .iter()
                    .flat_map(|l| l.parts())
                    .filter_map(|l| match l {
                        Launch::ConvParts { parts, .. } => Some(parts.iter().filter(|c| c.l2).count()),
                        _ => None,
                    })
                    .sum();
                assert_eq!(normed, if moved { 0 } else { 2 * recurrent }, "{what}: the convolutions' normed heads: {:?}", s.census());
                for f in lane.forms.iter().filter(|f| f.rows > 1) {
                    for l in &f.schedule.launches {
                        if let Launch::Delta { qk, rows, .. } = l {
                            assert!(*rows == 1 || qk.is_none(), "{what}: a chunk's delta rule reads normed heads: {l:?}");
                        }
                    }
                }
            }
        }
    }
}

/// The step takes its query and key heads' norms (`delta_qk_norm`) exactly where its layer's inputs' launch holds no
/// whole head in a block: on every case of `tiny_models::gated_delta_cases` and every 16-bit case (Qwen3.5 tiny, at
/// 2048 wide and with heads of 512, in BF16 and F16), under every numerics, the decode step with the rule is the one
/// with it held but for each inputs' launch whose blocks — eight warps, or a head's width of threads where that is
/// more, `per_warp` rows a warp — hold less than a head: that launch's convolutions norm no head (their parts write the
/// heads unnormed, and the launch keeps no head's width or epsilon), and the delta rule reading its query and key reads
/// them so, norming them by the convolutions' epsilon. A launch holding whole heads keeps its norms and its step reads
/// them normed. The rule fires once a launch it moves; both outcomes are met.
#[test]
fn a_steps_delta_rule_takes_its_heads_norms_only_where_its_inputs_launch_holds_no_whole_head() {
    let mut cases: Vec<(String, ModelSpec, PathBuf)> =
        tiny_models::gated_delta_cases(&dir(), "delta-qk-where", CTX).into_iter().map(|(name, spec, path, _)| (name, spec, path)).collect();
    let mut wide_heads = Tiny::Qwen35.spec(CTX);
    for layer in &mut wide_heads.layers {
        if let Mixer::GatedDelta(g) = &mut layer.mixer {
            g.head_dim = 512;
        }
    }
    for (name, spec) in [("qwen35", Tiny::Qwen35.spec(CTX)), ("qwen35-medium", Tiny::Qwen35.medium(CTX)), ("qwen35-heads512", wide_heads)] {
        for ty in [GgmlType::Bf16, GgmlType::F16] {
            let path = dir().join(format!("{name}-{}-delta-qk-where.gguf", ty.name()));
            tiny_models::write_checkpoint_in(&spec, &path, ty);
            cases.push((format!("{name} {}", ty.name()), spec.clone(), path));
        }
    }
    let (mut moved_anywhere, mut kept_anywhere) = (0usize, 0usize);
    for (name, spec, path) in &cases {
        for numerics in NUMERICS {
            let what = format!("{name} {numerics:?}");
            let opts = LaneOptions { prefill: 4, ..options(numerics) };
            let moved = compile(spec, path, &opts).unwrap_or_else(|e| panic!("{what}: {e}"));
            let held = compile_holding(spec, path, &opts, &["delta_qk_norm"]).unwrap_or_else(|e| panic!("{what} held: {e}"));
            let (m, h) = (&moved.decode().schedule, &held.decode().schedule);
            assert_eq!(h.fired("delta_qk_norm"), 0, "{what}: held, the rule fires nowhere");
            assert_eq!(m.launches.len(), h.launches.len(), "{what}: the rule moves norms and adds no launch: {:?} / {:?}", m.census(), h.census());
            // The held step's every inputs' launch that norms its heads in blocks holding none whole, with the unnormed
            // names the moved step's convolutions write in place of the normed ones.
            let mut renamed: BTreeMap<String, String> = BTreeMap::new();
            let mut eps_of: BTreeMap<String, _> = BTreeMap::new();
            let mut moves = 0usize;
            for (hl, ml) in h.launches.iter().zip(&m.launches) {
                let Some((Launch::GemmParts { per_warp, .. }, Launch::ConvParts { parts, block, eps, .. }, _)) = hl.gdn_inputs() else { continue };
                let normed = parts.iter().any(|c| c.l2);
                if !normed || (*block).max(256) / 32 * per_warp >= *block {
                    kept_anywhere += usize::from(normed);
                    assert_eq!(ml, hl, "{what}: an inputs' launch whose blocks hold whole heads keeps its norms");
                    continue;
                }
                moves += 1;
                let Some((_, Launch::ConvParts { parts: written, .. }, _)) = ml.gdn_inputs() else { panic!("{what}: the inputs' launch stays one launch: {ml:?}") };
                assert_eq!(written.len(), parts.len(), "{what}: the same convolutions: {ml:?}");
                let mut want = hl.clone();
                for part in want.parts_mut() {
                    if let Launch::ConvParts { parts, block, eps: e, .. } = part {
                        for (c, w) in parts.iter_mut().zip(written) {
                            if c.l2 {
                                assert_ne!(w.y, c.y, "{what}: a moved norm leaves its convolution writing the head unnormed, under its own name");
                                renamed.insert(c.y.clone(), w.y.clone());
                                eps_of.insert(w.y.clone(), eps.expect("a normed launch's epsilon"));
                                (c.y, c.l2) = (w.y.clone(), false);
                            }
                        }
                        (*block, *e) = (256, None);
                    }
                }
                assert_eq!(ml, &want, "{what}: the inputs' launch hands its query and key norms to the step and nothing else");
            }
            moved_anywhere += moves;
            assert_eq!(m.fired("delta_qk_norm"), moves, "{what}: once a launch it moves: {:?}", m.fusions);
            for (hl, ml) in h.launches.iter().zip(&m.launches).filter(|(hl, _)| hl.gdn_inputs().is_none()) {
                let mut want = hl.clone();
                if let Launch::Delta { q, k, qk, .. } = &mut want {
                    if let (Some(nq), Some(nk)) = (renamed.get(q.as_str()).cloned(), renamed.get(k.as_str()).cloned()) {
                        assert_eq!(*qk, None, "{what}: held, the step norms no head");
                        *qk = Some(eps_of[&nq]);
                        (*q, *k) = (nq, nk);
                    }
                }
                assert_eq!(ml, &want, "{what}: every other launch is the held step's, and the delta rule that reads moved heads norms them");
            }
            let normed_read = m.launches.iter().filter(|l| matches!(l, Launch::Delta { qk: Some(_), .. })).count();
            assert_eq!(normed_read, renamed.len() / 2, "{what}: a step norms the heads of each launch that moved them");
        }
    }
    assert!(moved_anywhere > 0 && kept_anywhere > 0, "both outcomes are met: {moved_anywhere} moved, {kept_anywhere} kept");
}

/// A one-row projection in a block format finishes the residual add and the next norm in its last-arriving block as a
/// Q8_0 one does (PARITY-GEMV-NORM-EPILOGUE, K-05, K-07, K-16) where its block is at least the norm's width
/// (`kernels::block_holds_its_norm`): Llama wide in every block format the library decodes, one row a warp, under every
/// numerics: every output and down projection is one fold with the norm that adds its result, launched as
/// `lk_gemv_blk_norm`, its projection the one the program apart launches, its rows a warp included. Held back, every
/// fold is its two launches again. Llama, Qwen3 and DeepSeek-V2 at 2048 wide in Q4_K — four rows a warp, a block of
/// four warps where the norm takes 32 — run every projection and its norm apart, the decode step the one the program
/// with the rule held runs.
#[test]
fn a_block_format_projection_finishes_its_add_and_norm_in_its_last_block() {
    for t in [Tiny::Llama, Tiny::Qwen3, Tiny::DeepSeek2] {
        let (spec, path) = medium_checkpoint(t, "blk-norm-q4k", &|_| GgmlType::Q4K);
        for numerics in NUMERICS {
            let what = format!("{} medium Q4_K {numerics:?}", t.name());
            let lane = compile(&spec, &path, &options(numerics)).unwrap_or_else(|e| panic!("{what}: {e}"));
            let apart = compile_holding(&spec, &path, &options(numerics), &["gemv_norm_epilogue"]).unwrap_or_else(|e| panic!("{what} held: {e}"));
            let s = &lane.decode().schedule;
            assert!(!s.launches.iter().any(|l| kind(l) == "gemv_norm"), "{what}: no projection norms in a block narrower than its norm: {:?}", s.census());
            assert!(!lane.program.cu.contains("lk_gemv_blk_norm<"), "{what}: the program launches no block GEMV with its norm");
            assert_eq!(s.launches, apart.decode().schedule.launches, "{what}: the decode step is the one the program with the rule held runs");
            assert!(
                s.launches.iter().any(|l| matches!(l, Launch::Gemv { per_warp: 4, format: GgmlType::Q4K, m: 1, .. })),
                "{what}: its projections take four rows a warp"
            );
        }
    }
    let mut cases: Vec<(String, ModelSpec, PathBuf, usize)> = Vec::new();
    for ty in BLOCK_FORMATS {
        let (spec, path) = wide_checkpoint(Tiny::Llama, &format!("blk-norm-{}", ty.name()), &|_| ty);
        let folds = 2 * spec.layers.len();
        cases.push((format!("llama wide {}", ty.name()), spec, path, folds));
    }
    for (name, spec, path, want) in &cases {
        for numerics in NUMERICS {
            let what = format!("{name} {numerics:?}");
            let lane = compile(spec, path, &options(numerics)).unwrap_or_else(|e| panic!("{what}: {e}"));
            let apart = compile_holding(spec, path, &options(numerics), &["gemv_norm_epilogue"]).unwrap_or_else(|e| panic!("{what} held: {e}"));
            let (s, held) = (&lane.decode().schedule, &apart.decode().schedule);
            let folds: Vec<&Launch> = s.launches.iter().filter(|l| kind(l) == "gemv_norm").collect();
            assert_eq!(folds.len(), *want, "{what}: a fold for each projection an add and norm reads: {:?}", s.census());
            for f in &folds {
                let Launch::Fold { first, then } = f else { unreachable!() };
                let (Launch::Gemv { y, format, .. }, Launch::Norm { x, add, .. }) = (first.as_ref(), then.as_ref()) else {
                    panic!("{what}: a fold of a projection and a norm: {f:?}")
                };
                assert_ne!(*format, GgmlType::Q8_0, "{what}: a block format's projection");
                assert!(*x == y.buf || add.as_deref() == Some(y.buf.as_str()), "{what}: the fold's norm reads its projection's result: {f:?}");
                let alone = held.launches.iter().find(|l| matches!(l, Launch::Gemv { y: hy, .. } if hy == y));
                assert_eq!(alone, Some(first.as_ref()), "{what}: the fold's projection is the one the program apart launches");
            }
            assert!(lane.program.cu.contains("lk_gemv_blk_norm<"), "{what}: the program launches the block GEMV with its norm");
            assert!(!held.launches.iter().any(|l| kind(l) == "gemv_norm"), "{what}: held, no projection folds its norm");
            assert_eq!(held.launches.len(), s.launches.len() + folds.len(), "{what}: held, each fold is its two launches");
        }
    }
}

/// A fold's norm runs on its first part's block as the norm's own block runs it only where that block is at least the
/// norm's width (`kernels::block_holds_its_norm`): a narrower block plays several of the norm's warps a warp, one after
/// another, and the row's norm waits on that one block. DeepSeek-V2-Lite's 320-thread down blocks and 128-thread Q4_K
/// projections playing the norm's 1024 decoded at 142.2 tokens a second on the 3060, 152.9 with those folds apart.
/// Every `lk_gemv_blk_norm` and `lk_moe_down_norm` launch — every tiny architecture's program, the checkpoints 2048 wide
/// in Q4_K and Llama's wide ones in every block format, under every numerics — plays one of the norm's warps a warp (its
/// `VW` 1), and both kinds occur.
#[test]
fn every_folded_norm_runs_on_a_block_of_its_width() {
    let mut programs: Vec<(String, String)> = Vec::new();
    for numerics in NUMERICS {
        for &t in ALL {
            let (spec, path) = checkpoint(t, "norm-width");
            let lane = compile(&spec, &path, &options(numerics)).unwrap_or_else(|e| panic!("{} {numerics:?}: {e}", t.name()));
            programs.push((format!("{} {numerics:?}", t.name()), lane.program.cu));
        }
        for t in [Tiny::Llama, Tiny::Qwen3, Tiny::DeepSeek2] {
            let (spec, path) = medium_checkpoint(t, "norm-width-q4k", &|_| GgmlType::Q4K);
            let lane = compile(&spec, &path, &options(numerics)).unwrap_or_else(|e| panic!("{} medium {numerics:?}: {e}", t.name()));
            programs.push((format!("{} medium Q4_K {numerics:?}", t.name()), lane.program.cu));
        }
        for ty in BLOCK_FORMATS {
            let (spec, path) = wide_checkpoint(Tiny::Llama, &format!("norm-width-{}", ty.name()), &|_| ty);
            let lane = compile(&spec, &path, &options(numerics)).unwrap_or_else(|e| panic!("llama wide {} {numerics:?}: {e}", ty.name()));
            programs.push((format!("llama wide {} {numerics:?}", ty.name()), lane.program.cu));
        }
    }
    // A folded norm kernel's warps played a warp is its last template argument.
    let mut seen: BTreeMap<&str, usize> = BTreeMap::new();
    for (what, cu) in &programs {
        for kernel in ["lk_gemv_blk_norm<", "lk_moe_down_norm<"] {
            for launch in cu.split(kernel).skip(1).filter_map(|r| r.split_once("><<<").map(|(a, _)| a)) {
                assert_eq!(launch.rsplit(", ").next(), Some("1"), "{what}: `{kernel}{launch}>` plays one of its norm's warps a warp");
                *seen.entry(kernel).or_insert(0) += 1;
            }
        }
    }
    for kernel in ["lk_gemv_blk_norm<", "lk_moe_down_norm<"] {
        assert!(seen.contains_key(kernel), "some program launches {kernel}…>: {seen:?}");
    }
}

/// A gated attention multiplies its output by its gate's activation as it stores it (`attn_output_gate`, K-13): Qwen3.5
/// tiny (heads of 16) and wide (heads of 64), under every numerics, the decode step and a short chunk launch no gated
/// product after the attention — the attention reads its gate in place and writes the gated output, quantized in its
/// store too where its heads are whole 32-value groups and a projection reads it so; a chunk the attention takes in
/// tiles gates in a launch of its own. Held back, every such attention is its two launches again.
#[test]
fn a_gated_attention_gates_its_output_in_its_store() {
    let wide = (Tiny::Qwen35.wide(CTX), dir().join("qwen35-wide-attn-gate.gguf"));
    tiny_models::write_checkpoint(&wide.0, &wide.1);
    for (name, (spec, path)) in [("qwen35", checkpoint(Tiny::Qwen35, "attn-gate")), ("qwen35 wide", wide)] {
        for numerics in NUMERICS {
            let what = format!("{name} {numerics:?}");
            let lane = compile(&spec, &path, &chunked(numerics, 4)).unwrap_or_else(|e| panic!("{what}: {e}"));
            let apart = compile_holding(&spec, &path, &chunked(numerics, 4), &["attn_output_gate"]).unwrap_or_else(|e| panic!("{what} held: {e}"));
            for (f, h) in lane.forms.iter().zip(&apart.forms) {
                let (s, held) = (&f.schedule, &h.schedule);
                let what = format!("{what}, {} rows", f.rows);
                let gated: Vec<&Launch> = s.launches.iter().filter(|l| matches!(l, Launch::Attention { gate: Some(_), .. })).collect();
                let attentions = s.launches.iter().filter(|l| matches!(l, Launch::Attention { .. })).count();
                assert_eq!(gated.len(), attentions, "{what}: every attention gates its output: {:?}", s.census());
                for l in &gated {
                    let Launch::Attention { out, gate: Some((g, act)), .. } = l else { unreachable!() };
                    assert_eq!(*act, logicaffeine_compile::tensor_recognize::qwen::Activation::Sigmoid, "{what}: by the gate's sigmoid");
                    assert!(s.launches.iter().any(|p| p.writes().contains(&g.as_str())), "{what}: the gate `{g}` is a projection's");
                    assert!(!s.launches.iter().any(|r| matches!(r, Launch::Binary { gate: Some(_), .. } if r.reads().contains(&out.as_str()))), "{what}: no gated product reads `{out}`");
                }
                assert_eq!(s.fired("attn_output_gate"), attentions, "{what}: the rule fires an attention");
                assert_eq!(held.fired("attn_output_gate"), 0, "{what}: held, none gates");
                assert_eq!(held.launches.len(), s.launches.len() + attentions, "{what}: held, each gated attention is its two launches");
            }
        }
    }
}

/// A batch's gated attentions gate their rows as they store them (`attn_output_gate` over the sequences' rows, K-13):
/// Qwen3.5 tiny (heads of 16) and wide (heads of 64), under every numerics, a batch step of two sequences and of four
/// runs each gated attention layer as one attention over its sequences that reads the batch's gate table in place,
/// from its first row, each sequence's row of it gating that sequence's output — the gate written by a projection —
/// and stores the gated rows the output projection reads: no product reads the attention's output. Held back, every
/// gated product is a launch after its attention again: under the precise numerics one launch more a gated attention,
/// and never fewer launches under any.
#[test]
fn a_batchs_gated_attentions_gate_their_rows_in_their_store() {
    let wide = (Tiny::Qwen35.wide(CTX), dir().join("qwen35-wide-batch-gate.gguf"));
    tiny_models::write_checkpoint(&wide.0, &wide.1);
    for (name, (spec, path)) in [("qwen35", checkpoint(Tiny::Qwen35, "batch-gate")), ("qwen35 wide", wide)] {
        for numerics in NUMERICS {
            for rows in [2, 4] {
                let what = format!("{name} {numerics:?}, {rows} sequences");
                let opts = LaneOptions { prefill: 4, ..options(numerics) };
                let lane = compile_batch(&spec, &path, &opts, rows).unwrap_or_else(|e| panic!("{what}: {e}"));
                let apart = compile_batch_holding(&spec, &path, &opts, rows, &["attn_output_gate"]).unwrap_or_else(|e| panic!("{what} held: {e}"));
                let s = &lane.batch().unwrap_or_else(|| panic!("{what}: the batch step")).schedule;
                let held = &apart.batch().unwrap_or_else(|| panic!("{what} held: the batch step")).schedule;
                let attentions: Vec<&Launch> = s.launches.iter().filter(|l| matches!(l, Launch::Attention { .. })).collect();
                assert!(!attentions.is_empty(), "{what}: the gated attention layers attend: {:?}", s.census());
                for l in &attentions {
                    let Launch::Attention { out, gate, goff, rows: r, seq, .. } = l else { unreachable!() };
                    assert_eq!((*r, seq.is_some()), (rows, true), "{what}: one attention over the sequences into `{out}`");
                    let Some((g, act)) = gate else { panic!("{what}: the attention into `{out}` gates its rows: {:?}", s.census()) };
                    assert_eq!(*act, logicaffeine_compile::tensor_recognize::qwen::Activation::Sigmoid, "{what}: by the gate's sigmoid");
                    assert_eq!(*goff, 0, "{what}: the gate read from its first row, each sequence its own row");
                    assert!(s.launches.iter().any(|p| p.writes().contains(&g.as_str()) && p.projected_input().is_some()), "{what}: the gate `{g}` is a projection's");
                    assert!(!s.launches.iter().any(|b| matches!(b, Launch::Binary { .. }) && b.reads().contains(&out.as_str())), "{what}: no product reads `{out}`");
                    assert!(s.launches.iter().any(|p| p.projected_input() == Some(out.as_str())), "{what}: the output projection reads the gated rows `{out}`");
                }
                assert_eq!(s.fired("attn_output_gate"), attentions.len(), "{what}: the rule fires a gated attention layer");
                assert_eq!(held.fired("attn_output_gate"), 0, "{what}: held, none gates");
                let gated_products = held.launches.iter().filter(|l| matches!(l, Launch::Binary { gate: Some(_), .. })).count();
                assert_eq!(gated_products, attentions.len(), "{what}: held, each gated attention's product is a launch of its own");
                if numerics == Q8MatKind::Dequant {
                    assert_eq!(held.launches.len(), s.launches.len() + attentions.len(), "{what}: held, each gated attention is its two launches");
                } else {
                    assert!(held.launches.len() >= s.launches.len(), "{what}: held, never fewer launches");
                }
            }
        }
    }
}

/// A batch's sequences quantize their rows where they store them (PARITY-BATCH-ROW-QUANT): on every tiny architecture at
/// its wide width — heads of 64, whole 32-value groups — under both quantized numerics, in batches of 2, 4 and 20, every
/// attention and delta rule whose rows a projection reads quantized writes them quantized in its own store, and no
/// launch quantizes them after it; with the sequences' launches held apart, each sequence's row set carries its row's
/// quantized form into the table's.
#[test]
fn a_batchs_attentions_and_delta_rules_quantize_their_rows_in_their_store() {
    let read_quantized = |s: &Schedule, out: &str| {
        s.launches.iter().any(|l| l.touches(s.numerics).0.iter().any(|(t, touch)| *t == out && *touch == Touch::Quant))
    };
    let produced = |l: &Launch| match l {
        Launch::Attention { out, quant, .. } => Some(("attention", out.clone(), *quant)),
        Launch::Delta { out, quant, .. } => Some(("delta rule", out.clone(), *quant)),
        _ => None,
    };
    let checkpoints: Vec<(Tiny, ModelSpec, PathBuf)> = ALL
        .iter()
        .map(|&t| {
            let (spec, path) = (t.wide(CTX), dir().join(format!("{}-wide-batch-rows.gguf", t.name())));
            tiny_models::write_checkpoint(&spec, &path);
            (t, spec, path)
        })
        .collect();
    for numerics in [Q8MatKind::Int8Act, Q8MatKind::TwoPassInt8] {
        for rows in [2, 4, 20] {
            let mut stored = 0;
            for (t, spec, path) in &checkpoints {
                let what = format!("{} {numerics:?}, {rows} sequences", t.name());
                let opts = LaneOptions { prefill: 4, ..options(numerics) };
                let lane = compile_batch(spec, path, &opts, rows).unwrap_or_else(|e| panic!("{what}: {e}"));
                let s = &lane.batch().unwrap_or_else(|| panic!("{what}: the batch step")).schedule;
                for (kind, out, quant) in s.launches.iter().flat_map(|l| l.parts()).filter_map(produced) {
                    if !read_quantized(s, &out) {
                        continue;
                    }
                    assert!(quant, "{what}: the {kind} writing `{out}` quantizes its rows in its store: {:?}", s.census());
                    assert!(!s.launches.iter().any(|q| matches!(q, Launch::Quant { x, .. } if *x == out)), "{what}: no launch quantizes `{out}` after its {kind}");
                    stored += 1;
                }
                let apart = compile_batch_holding(spec, path, &opts, rows, &["sequences"]).unwrap_or_else(|e| panic!("{what} apart: {e}"));
                let held = &apart.batch().unwrap_or_else(|| panic!("{what} apart: the batch step")).schedule;
                for l in &held.launches {
                    let Launch::SetRow { table, src, quant, .. } = l else { continue };
                    let from_producer = held.launches.iter().flat_map(|p| p.parts()).filter_map(produced).any(|(_, out, q)| out == *src && q);
                    if from_producer && read_quantized(held, table) {
                        assert!(*quant, "{what} apart: the set of `{src}` into `{table}` carries its row's quantized form");
                    }
                }
            }
            assert!(stored > 0, "{numerics:?}, {rows} sequences: some architecture's attention or delta rule rows are read quantized");
        }
    }
}

/// The step's first launch gathers each token's embedding row and norms it in the same block (PARITY-EMBED-NORM):
/// every architecture's decode step and prefill chunk, under every numerics, begins with one fold of its gather — Q8_0's,
/// a block format's or an `f32` table's, Gemma's scale inside, a tied table the head reads too — and the first layer's
/// norm, the gathered row written for the residual and the normed row for the first projections, and no plain norm
/// launch reads the gathered row; held, the fold is its gather and its norm again, one launch more.
#[test]
fn the_embedding_gather_norms_its_row_in_the_same_launch() {
    let mut cases: Vec<(String, ModelSpec, PathBuf)> = Vec::new();
    for t in [Tiny::Llama, Tiny::LlamaTied, Tiny::Qwen3, Tiny::Gemma3, Tiny::GptOss, Tiny::Qwen35] {
        let (spec, path) = checkpoint(t, "embed");
        cases.push((t.name().to_string(), spec, path));
    }
    let (spec, path) = sized_checkpoint(Tiny::Llama.wide(CTX), "llama-wide-embed-q4k", &|_| GgmlType::Q4K);
    cases.push(("llama wide Q4_K".to_string(), spec, path));
    let (spec, path) = sized_checkpoint(Tiny::Llama.spec(CTX), "llama-embed-f32", &|t| if t == "token_embd.weight" { GgmlType::F32 } else { GgmlType::Q8_0 });
    cases.push(("llama f32 table".to_string(), spec, path));
    for (name, spec, path) in &cases {
        for numerics in NUMERICS {
            let what = format!("{name} {numerics:?}");
            let lane = compile(spec, path, &chunked(numerics, 4)).unwrap_or_else(|e| panic!("{what}: {e}"));
            let apart = compile_holding(spec, path, &chunked(numerics, 4), &["embed_norm"]).unwrap_or_else(|e| panic!("{what} held: {e}"));
            for (f, held) in lane.forms.iter().zip(&apart.forms) {
                let (s, h) = (&f.schedule, &held.schedule);
                let what = format!("{what}, {} rows", f.rows);
                let Some(Launch::Fold { first, then }) = s.launches.first() else { panic!("{what}: the step begins with a fold: {:?}", s.census()) };
                let (Launch::Gather { out: raw, rows, .. }, Launch::Norm { x, add: None, rows: normed, .. }) = (first.as_ref(), then.as_ref()) else {
                    panic!("{what}: a gather and the norm of its rows: {:?}", s.launches[0])
                };
                assert_eq!((x, rows), (raw, normed), "{what}: the norm takes every row the gather wrote");
                assert_eq!(*rows, f.rows, "{what}: a row a token");
                assert!(s.launches.iter().filter_map(|l| match l {
                    Launch::Norm { x, add: None, .. } => Some(x),
                    _ => None,
                }).all(|x| x != raw), "{what}: no plain norm launch reads the gathered row");
                assert!(s.launches[1..].iter().any(|l| l.reads().contains(&raw.as_str())), "{what}: the gathered row is read on as the residual");
                assert!(!h.launches.iter().any(|l| kind(l) == "gather_norm"), "{what}: held, no gather norms its rows");
                assert_eq!(h.launches.len(), s.launches.len() + 1, "{what}: held, the fold is its two launches");
            }
        }
    }
}

/// A step's query, key and value projections turn their heads in the epilogue of the one GEMV in parts that writes
/// them (PARITY-QKV-HEAD-EPILOGUE): every attention layer's decode step runs one fold of its three projections and the
/// rotations of its query and key — each side reading the part its head rows land in, normed where the model norms its
/// heads, the key's rotation landing in its cache row — and no launch of rotations of its own; held, each fold is its
/// GEMV in parts and its launch of rotations, one launch more a layer. A query projection that takes four rows a warp,
/// where its key's takes one, turns its heads alone, and the key's turn beside the values.
#[test]
fn a_decode_layers_query_key_and_value_are_one_launch_with_their_head_epilogue() {
    let mut medium = Tiny::Qwen3.medium(CTX);
    for layer in &mut medium.layers {
        if let Mixer::Attention(a) = &mut layer.mixer {
            (a.heads, a.kv_heads) = (32, 8);
        }
    }
    let (spec, path) = sized_checkpoint(medium, "qwen3-medium-heads", &|_| GgmlType::Q4K);
    for numerics in NUMERICS {
        let lane = compile(&spec, &path, &options(numerics)).unwrap_or_else(|e| panic!("{numerics:?}: {e}"));
        let s = &lane.decode().schedule;
        let folds: Vec<(usize, usize, usize, bool)> = s
            .launches
            .iter()
            .filter_map(|l| match l {
                Launch::Fold { first, then } => match (first.as_ref(), then.as_ref()) {
                    (Launch::GemmParts { parts, per_warp, .. }, Launch::RopeParts { sides, .. }) => Some((parts.len(), *per_warp, sides.len(), sides[0].y.row.is_some())),
                    _ => None,
                },
                _ => None,
            })
            .collect();
        let layers = spec.layers.len();
        assert_eq!(folds.iter().filter(|f| **f == (1, 4, 1, false)).count(), layers, "{numerics:?}: the query turns its heads alone, four rows a warp: {folds:?}");
        assert_eq!(folds.iter().filter(|f| **f == (2, 1, 1, true)).count(), layers, "{numerics:?}: the key turns its heads beside the values: {folds:?}");
        assert_eq!(folds.len(), 2 * layers, "{numerics:?}: {folds:?}");
        let held = compile_holding(&spec, &path, &options(numerics), &["qkv_head_epilogue"]).unwrap_or_else(|e| panic!("{numerics:?} held: {e}"));
        assert_eq!(held.decode().schedule.launches.len(), s.launches.len() + layers, "{numerics:?}: held, a launch more a layer");
    }

    for t in [Tiny::Llama, Tiny::Qwen3, Tiny::Gemma3, Tiny::GptOss] {
        for numerics in NUMERICS {
            let what = format!("{} {numerics:?}", t.name());
            let lane = compiled(t, numerics, &format!("heads-{}", t.name()));
            let s = &lane.decode().schedule;
            let spec = t.spec(CTX);
            let folds: Vec<&Launch> = s.launches.iter().filter(|l| kind(l) == "gemv_rope").collect();
            assert_eq!(folds.len(), spec.layers.len(), "{what}: a fold a layer: {:?}", s.census());
            for (f, layer) in folds.iter().zip(&spec.layers) {
                let Mixer::Attention(a) = &layer.mixer else { panic!("{what}: an attention layer") };
                let Launch::Fold { first, then } = f else { unreachable!() };
                let (Launch::GemmParts { parts, m: 1, .. }, Launch::RopeParts { sides, hd, .. }) = (first.as_ref(), then.as_ref()) else {
                    panic!("{what}: a fold of one row's projections in parts and their rotations: {f:?}")
                };
                assert_eq!(parts.len(), 3, "{what}: the query, key and value projections in one launch: {f:?}");
                assert_eq!(sides.len(), 2, "{what}: the query's and the key's rotations in its epilogue: {f:?}");
                assert_eq!(*hd, a.head_dim, "{what}");
                for side in sides {
                    let read = parts.iter().find(|p| p.y.own() == Some(side.x.as_str()));
                    assert!(read.is_some_and(|p| p.n == side.heads * hd), "{what}: a side reads whole the part its heads are written to: {f:?}");
                    assert_eq!(side.norm.is_some(), a.qk_norm, "{what}: a side norms its heads where the model does: {f:?}");
                }
                assert_eq!(sides.iter().filter(|s| s.y.row.is_some()).count(), 1, "{what}: the key's rotation lands in its cache row: {f:?}");
            }
            assert!(!s.launches.iter().any(|l| matches!(l, Launch::Rope { .. } | Launch::RopeParts { .. })), "{what}: no rotation launches apart: {:?}", s.census());
            let (spec, path) = checkpoint(t, &format!("heads-held-{}", t.name()));
            let apart = compile_holding(&spec, &path, &options(numerics), &["qkv_head_epilogue"]).unwrap_or_else(|e| panic!("{what} held: {e}"));
            let held = &apart.decode().schedule;
            assert!(!held.launches.iter().any(|l| kind(l) == "gemv_rope"), "{what}: held, no projection turns its heads");
            assert_eq!(held.launches.iter().filter(|l| matches!(l, Launch::RopeParts { .. })).count(), folds.len(), "{what}: held, a launch of rotations a layer");
            assert_eq!(held.launches.len(), s.launches.len() + folds.len(), "{what}: held, each fold is its two launches");
        }
    }
}

/// A decode step's attention quantizes its output in its own store for the projection that reads it
/// (PARITY-ATTN-QUANT-IN-FOLD): under both quantized numerics, on every architecture at its wide width — heads of 64,
/// whole 32-value groups — whose attention a projection reads quantized, the decode step and a short chunk launch no
/// quantize of an attention's output — the attention writes the words and scales, and its `f32` output only where
/// something reads that; a batch's attention, each sequence's output a row of one table, quantizes its rows in its
/// store too (PARITY-BATCH-ROW-QUANT); heads of 16, half a group, and a chunk the attention takes in tiles quantize in a
/// launch of their own; Qwen3.5's gated attention gates its
/// output in the same store (`attn_output_gate`) and quantizes it as every attention does; with `fuse_quant` off every
/// attention writes `f32` alone and a quantize follows it.
#[test]
fn a_decode_attention_quantizes_its_output_in_its_fold() {
    let quantized_after = |s: &Schedule, out: &str| s.launches.iter().any(|l| matches!(l, Launch::Quant { x, .. } if x == out));
    let read_quantized = |s: &Schedule, out: &str| {
        s.launches.iter().any(|l| l.touches(s.numerics).0.iter().any(|(t, touch)| *t == out && *touch == Touch::Quant))
    };
    let narrow = compiled(Tiny::Llama, Q8MatKind::Int8Act, "attn-quant-narrow");
    let s = &narrow.decode().schedule;
    for l in &s.launches {
        if let Launch::Attention { out, quant, .. } = l {
            assert!(!quant && quantized_after(s, out), "heads of 16: `{out}` is quantized in a launch of its own");
        }
    }
    for t in [Tiny::Llama, Tiny::Qwen3, Tiny::Gemma3, Tiny::GptOss, Tiny::Qwen3Moe, Tiny::Qwen35] {
        let (spec, path) = (t.wide(CTX), dir().join(format!("{}-wide-attn-quant.gguf", t.name())));
        tiny_models::write_checkpoint(&spec, &path);
        for numerics in [Q8MatKind::Int8Act, Q8MatKind::TwoPassInt8] {
            let what = format!("{} {numerics:?}", t.name());
            let lane = compile_batch(&spec, &path, &LaneOptions { prefill: 4, ..options(numerics) }, 2).unwrap_or_else(|e| panic!("{what}: {e}"));
            let mut folded = 0;
            for f in &lane.forms {
                let s = &f.schedule;
                for l in &s.launches {
                    let Launch::Attention { out, quant, f32, gate, .. } = l else { continue };
                    if t == Tiny::Qwen35 {
                        assert!(
                            matches!(gate, Some((_, logicaffeine_compile::tensor_recognize::qwen::Activation::Sigmoid))),
                            "{what}, {} rows: the gated attention writing `{out}` gates it by its sigmoid in its store",
                            f.rows
                        );
                    }
                    if !read_quantized(s, out) {
                        continue;
                    }
                    assert!(*quant, "{what}, {} rows: the attention writing `{out}` quantizes it in its store", f.rows);
                    assert!(!quantized_after(s, out), "{what}, {} rows: no launch quantizes `{out}` after its attention", f.rows);
                    let plain = s.launches.iter().any(|r| r.touches(s.numerics).0.iter().any(|(x, touch)| *x == out.as_str() && *touch == Touch::Plain));
                    assert_eq!(*f32, plain, "{what}, {} rows: `{out}` is written `f32` exactly where something reads it so", f.rows);
                    folded += 1;
                }
            }
            assert!(folded > 0, "{what}: some attention's output is read quantized, or gated first");
            let apart = compile(&spec, &path, &LaneOptions { prefill: 4, knobs: CudaKnobs { fuse_quant: false, ..options(numerics).knobs }, ..options(numerics) })
                .unwrap_or_else(|e| panic!("{what} apart: {e}"));
            for f in &apart.forms {
                for l in &f.schedule.launches {
                    if let Launch::Attention { out, quant, f32, .. } = l {
                        assert!(!quant && *f32, "{what} apart, {} rows: `{out}` is written f32 alone", f.rows);
                        if read_quantized(&f.schedule, out) {
                            assert!(quantized_after(&f.schedule, out), "{what} apart, {} rows: a quantize follows the attention", f.rows);
                        }
                    }
                }
            }
        }
    }
    let (spec, path) = checkpoint(Tiny::Qwen3, "attn-quant-tiles");
    let wide = compile(&spec, &path, &LaneOptions { ctx: 128, prefill: 64, ..options(Q8MatKind::Int8Act) }).unwrap();
    let chunk = wide.chunk(64).expect("the chunk of 64");
    let tiled: Vec<&Launch> = chunk.schedule.launches.iter().filter(|l| l.attends_in_tiles(chunk.schedule.numerics)).collect();
    assert!(!tiled.is_empty(), "a chunk of 64 attends in tiles");
    for l in tiled {
        let Launch::Attention { out, quant, .. } = l else { unreachable!() };
        assert!(!quant && quantized_after(&chunk.schedule, out), "a tiled attention's `{out}` is quantized in a launch of its own");
    }
}

/// A latent absorption — a per-head matmul whose rows are its inputs — stages its rows in shared memory under the
/// quantized numerics in a block format but Q6_K and Q8_0, in every form alike (so a prompt sums alike in chunks or a
/// token at a time); the precise numerics, Q6_K and Q8_0 keep the per-lane kernel.
#[test]
fn the_absorption_stages_its_rows_where_staging_wins() {
    let staged = |lane: &Lane| (lane.program.cu.matches("lk_head_axpy_staged<").count(), lane.program.cu.matches("lk_head_axpy<").count());
    for (format, numerics, stages) in [
        (GgmlType::Q4K, Q8MatKind::Int8Act, true),
        (GgmlType::Q4K, Q8MatKind::TwoPassInt8, true),
        (GgmlType::Q4K, Q8MatKind::Dequant, false),
        (GgmlType::Q5_0, Q8MatKind::Int8Act, true),
        (GgmlType::Q6K, Q8MatKind::Int8Act, false),
        (GgmlType::Q8_0, Q8MatKind::Int8Act, false),
    ] {
        let (spec, path) = wide_checkpoint(Tiny::DeepSeek2, &format!("absorb-{}", format.name()), &|_| format);
        let what = format!("{} {numerics:?}", format.name());
        let lane = compile(&spec, &path, &chunked(numerics, 4)).unwrap_or_else(|e| panic!("{what}: {e}"));
        let (s, plain) = staged(&lane);
        // Three absorptions a step; the decode step's and the chunks' launches all print into the one program.
        let every = 3 * lane.forms.len();
        assert_eq!((s, plain), if stages { (every, 0) } else { (0, every) }, "{what}: staged and per-lane absorptions");
    }
}

/// A literal row of a table is a view into it — unless the table is written before the row's last reader, which would
/// then read the write: that row is copied where the program takes it.
#[test]
fn a_row_of_a_table_written_before_its_last_reader_is_copied_where_it_is_taken() {
    let row_then_write = |reader_first: bool| {
        let read = "Let late be rmsnorm of early.\n";
        cache_row_step(
            "",
            &format!(
                "Let early be row 2 of Kc.\n{}Set row tok of Kc to kc.\n{}",
                if reader_first { read } else { "" },
                if reader_first { "" } else { read }
            ),
        )
    };
    let copied = planned(&row_then_write(false));
    assert!(
        copied.launches.iter().any(|l| matches!(l, Launch::Slice { x, y, groups: 1, from: 72, width: 72, .. } if x == "Kc" && y == "early")),
        "the row read after the table's write is copied before it: {:?}",
        copied.launches
    );
    assert!(!copied.launches.iter().any(|l| matches!(l, Launch::View { out, .. } if out == "early")));
    let viewed = planned(&row_then_write(true));
    assert!(
        viewed.launches.iter().any(|l| matches!(l, Launch::View { src, offset: 72, out } if src == "Kc" && out == "early")),
        "the row read before the write stays a view: {:?}",
        viewed.launches
    );
}

/// Qwen3.5's hybrid: each Gated DeltaNet block's inputs are one launch (`gdn_input_epilogue`): its query, key, value and
/// gate projections as one GEMV in parts, its three convolutions carrying their silu and the query's and key's L2 norm
/// (`conv_parts`) in its epilogue, and its two gates with the two `f32` projections they read (`gate_projections`) in
/// its last blocks; its output norm quantizes for the output projection; the gated attention's projections are one
/// launch (`gemv_parts`) with its heads' norms and rotations in its
/// epilogue (`qkv_head_epilogue`), and its gate's sigmoid and product run in the attention's store, which quantizes for
/// the output projection; every output and down projection takes the residual add and the next norm into its last
/// block (`gemv_norm_epilogue`), and the embedding's gather the first norm (`embed_norm`).
#[test]
fn the_hybrid_step_fuses_the_recurrence_and_the_gate() {
    let lane = compiled(Tiny::Qwen35, Q8MatKind::Int8Act, "hybrid");
    let s = &lane.decode().schedule;
    assert_eq!(
        s.census(),
        counts(&[
            ("argmax", 1),
            ("attention", 1),
            ("delta", 2),
            ("gather_norm", 1),
            ("gemv_gdn", 2),
            ("gemv_glu", 3),
            ("gemv_norm", 6),
            ("gemv_q8", 1),
            ("gemv_rope", 1)
        ])
    );
    for (rule, n) in [
        ("add_norm", 6),
        ("conv_silu", 6),
        ("conv_silu_l2", 4),
        ("gates", 2),
        ("act_gate", 1),
        ("attn_output_gate", 1),
        ("head_norm_rope", 2),
        ("row_write", 2),
        ("argmax_advance", 1),
        ("glu_pair", 3),
        ("fused_quant", 13),
        ("gemv_parts", 1),
        ("conv_parts", 2),
        ("gate_projections", 2),
        ("gdn_input_epilogue", 2),
        ("qkv_head_epilogue", 2),
        ("gemv_norm_epilogue", 6),
        ("embed_norm", 1),
        ("rope_parts", 0),
    ] {
        assert_eq!(s.fired(rule), n, "{rule}");
    }
}

/// A prefill chunk is the decode step's census over a block of rows: the embedding's gather norms every row it gathers
/// (`embed_norm`), each projection multiplies the chunk's rows in one launch (a layer's query, key and value in one
/// launch together, `gemv_parts`), the keys and values land in the caches as a block, the head reads the last row in
/// place — and the last
/// layer's residual add stays a launch of its own, since the row the head reads is its first reader. An expert
/// mixture routes each row on its own and runs as one launch set over the chunk's rows (`moe_rows`): each expert reads
/// its weights once for every row that chose it, reading the norm's quantized block, the rows' results written into
/// the stacked output.
#[test]
fn a_prefill_chunk_takes_its_rows_in_one_launch_per_op() {
    let census = |t: Tiny| compiled_chunks(t, Q8MatKind::Int8Act, 4, "chunk").chunk(4).unwrap().schedule.clone();
    let dense = census(Tiny::Qwen3);
    assert_eq!(
        dense.census(),
        counts(&[
            ("argmax", 1),
            ("attention", 3),
            ("binary", 1),
            ("gather_norm", 1),
            ("gemv_glu", 3),
            ("gemv_parts", 3),
            ("gemv_q8", 7),
            ("quant", 3),
            ("rmsnorm", 6),
            ("rope", 6)
        ])
    );
    for (rule, n) in [
        ("add_norm", 5),
        ("head_norm_rope", 6),
        ("row_write", 6),
        ("argmax_advance", 1),
        ("fused_quant", 10),
        ("glu_pair", 3),
        ("gemv_parts", 3),
        ("embed_norm", 1),
    ] {
        assert_eq!(dense.fired(rule), n, "{rule}");
    }
    let rows: Vec<usize> = dense
        .launches
        .iter()
        .filter_map(|l| if let Launch::Gemv { m, .. } = l { Some(*m) } else { None })
        .collect();
    let merged: Vec<usize> =
        dense.launches.iter().filter_map(|l| if let Launch::GemmParts { m: 4, parts, .. } = l { Some(parts.len()) } else { None }).collect();
    assert_eq!(merged, [3, 3, 3], "each layer's query, key and value in one launch over the chunk's four rows");
    assert_eq!(rows.iter().filter(|&&m| m == 4).count() + merged.iter().sum::<usize>(), 15, "every layer projection takes the chunk's four rows");
    assert_eq!(
        dense.launches.iter().filter(|l| matches!(l, Launch::GemvGlu { m: 4, .. })).count(),
        3,
        "and every gated pair"
    );
    assert_eq!(rows.iter().filter(|&&m| m == 1).count(), 1, "the LM head takes the last row alone");

    let hybrid = census(Tiny::Qwen35);
    assert_eq!(
        hybrid.census(),
        counts(&[
            ("argmax", 1),
            ("attention", 1),
            ("binary", 1),
            ("conv", 6),
            ("delta", 2),
            ("gates", 2),
            ("gather_norm", 1),
            ("gemv_f32", 4),
            ("gemv_glu", 3),
            ("gemv_parts", 3),
            ("gemv_q8", 7),
            ("rmsnorm", 6),
            ("rope", 2)
        ])
    );
    for (rule, n) in [
        ("add_norm", 5),
        ("conv_silu", 6),
        ("conv_silu_l2", 4),
        ("gates", 2),
        ("act_gate", 1),
        ("attn_output_gate", 1),
        ("row_write", 2),
        ("glu_pair", 3),
        ("fused_quant", 13),
        ("gemv_parts", 3),
        ("conv_parts", 0),
        ("gate_projections", 0),
        ("rope_parts", 0),
    ] {
        assert_eq!(hybrid.fired(rule), n, "hybrid {rule}");
    }

    let experts = census(Tiny::Qwen3Moe);
    assert_eq!(
        experts.census(),
        counts(&[
            ("argmax", 1),
            ("attention", 3),
            ("binary", 1),
            ("gather_norm", 1),
            ("gemv_parts", 3),
            ("gemv_q8", 4),
            ("moe", 1),
            ("moe_norm", 2),
            ("quant", 3),
            ("rmsnorm", 4),
            ("rope", 6)
        ])
    );
    assert_eq!(experts.fired("row_write"), 6 + 12, "the caches' blocks and every routed row");
    assert_eq!(experts.fired("moe_rows"), 3, "each layer's routed rows in one mixture");
    assert!(
        mixture_launches(&experts).iter().all(|l| matches!(l, Launch::Moe { rows: 4, .. })),
        "every mixture of the chunk takes its four rows"
    );
    assert_eq!(experts.fired("fused_quant"), 3 + 3 + 1, "each block's norms quantize once for every row they feed");
    experts_read_their_norms_quantized(&experts, "chunk");
}

/// A chunk wider than the GEMV's sixteen rows multiplies its projections on the int8 tensor cores — a Q8_0 matrix's
/// split arrays through the block GEMM every format takes (`lk_gemm_blk`), under two words in tiles of 64 rows — and
/// under the precise numerics as slabs of sixteen rows, each projection apart; the chunk of sixteen, one GEMV's rows,
/// takes each layer's query, key and value in one GEMV in parts. Both programs build.
#[test]
fn a_wide_chunk_runs_its_projections_on_the_tensor_cores() {
    let (spec, path) = checkpoint(Tiny::Qwen3, "wide");
    let wide = |numerics| compile(&spec, &path, &LaneOptions { ctx: 128, prefill: 64, ..options(numerics) }).unwrap();
    let fast = wide(Q8MatKind::Int8Act);
    assert_eq!(fast.forms.iter().map(|f| f.rows).collect::<Vec<_>>(), [1, 64, 32, 16, 8, 4, 2]);
    // A dense GEMM's launch, with the general epilogue or the plain one (`blk_plain`: its sums into an `f32` array).
    let dense_launch = ["lk_gemm_blk<8, 1><<<", "lk_gemm_blk<8, 1, 0, 128, 0, true><<<"];
    assert!(dense_launch.iter().any(|k| fast.program.cu.contains(k)), "the chunk of 64 runs the tensor-core GEMM");
    assert!(
        ["lk_gemm_blk<8, 1>", "lk_gemm_blk<8, 1, 0, 128, 0, true>"].iter().any(|k| fast.program.cu.contains(&format!("cudaFuncSetAttribute({k}"))),
        "with the shared memory its stages need"
    );
    assert!(fast.program.cu.contains("lk_gemv_q8<1, 16>"), "the chunk of 16 keeps the GEMV");
    let two = wide(Q8MatKind::TwoPassInt8);
    assert!(
        two.program.cu.contains("lk_gemm_blk<8, 2, 0, 64><<<") || two.program.cu.contains("lk_gemm_blk<8, 2, 0, 64, 0, true><<<"),
        "two words take the Q8_0 GEMM in tiles of 64 rows"
    );
    assert!(!two.program.cu.contains("lk_gemm_blk<8, 2><<<"), "and never the 128-row tile");
    let precise = wide(Q8MatKind::Dequant);
    assert!(!precise.program.cu.contains("lk_gemm_blk"), "the precise numerics never take the int8 tensor cores");
    assert!(![&fast, &two, &precise].iter().any(|l| l.program.cu.contains("lk_gemm_q8")), "no Q8_0 GEMM of its own");
    let dense: usize = dense_launch.iter().map(|k| fast.program.cu.matches(k).count()).sum();
    assert_eq!(dense, 21 * 2, "the 64- and 32-row chunks keep every projection, the gated pair's too, on the tensor cores");
    assert_eq!(fast.program.cu.matches("lk_gemv_glu<8, 8, 1, 16").count(), 3, "the 16-row chunk takes its gated pairs in one launch each");
    let slabs = precise.program.cu.matches("lk_gemv_q8<0, 16>").count();
    assert_eq!(slabs, 15 * (4 + 2) + (15 - 9), "every layer projection of the 64- and 32-row chunks in slabs of sixteen, the 16-row chunk's but its query, key and value");
    assert_eq!(precise.program.cu.matches("lk_gemv_parts<8, 0, 16").count(), 3, "the 16-row chunk's query, key and value, one slab together a layer");
    let pairs = precise.program.cu.matches("lk_gemv_glu<8, 8, 0, 16").count();
    assert_eq!(pairs, 3 * (4 + 2 + 1), "and every gated pair, in slabs of sixteen");
    if common::require_tool("nvcc") {
        nvcc(&fast.program.cu, &dir().join("lane-wide"));
        nvcc(&two.program.cu, &dir().join("lane-wide2"));
        nvcc(&precise.program.cu, &dir().join("lane-wide0"));
    }
}

/// A chunk wider than the GEMV's sixteen rows multiplies its `f32` projections (Qwen3.5's decay and update gates) as one
/// `f32` GEMM of 64 × 64 tiles over its rows (`lk_gemm_f32`), each tile's `k` cut into the parts that give the chip's
/// blocks work — where the GEMV would sum each row's dots in `f64`, a warp a dot. The chunk of 16 keeps the GEMV, and the
/// precise numerics keep its `f64` sums at every width; the decode step's two take their gates' launch
/// (`gate_projections`), which sums them in `f64` too — the layer's input epilogue (`gdn_input_epilogue`, which takes the
/// gates' launch into its projections') held.
#[test]
fn a_wide_chunk_multiplies_its_f32_projections_in_tiles() {
    use logicaffeine_compile::codegen_cuda::lane::kernels::gemm_f32_splits;
    let (spec, path) = checkpoint(Tiny::Qwen35, "wide-f32");
    let wide = |numerics| compile_holding(&spec, &path, &LaneOptions { ctx: 128, prefill: 64, ..options(numerics) }, &["gdn_input_epilogue"]).unwrap();
    let f32s = |lane: &Lane, wide: bool| -> Vec<(usize, usize, usize)> {
        lane.forms
            .iter()
            .flat_map(|f| f.schedule.launches.iter())
            .filter_map(|l| match l {
                Launch::GemvF32 { k, n, m, .. } if (*m > 16) == wide => Some((*k, *n, *m)),
                _ => None,
            })
            .collect()
    };
    for numerics in [Q8MatKind::Int8Act, Q8MatKind::TwoPassInt8] {
        let lane = wide(numerics);
        assert_eq!(lane.forms.iter().map(|f| f.rows).collect::<Vec<_>>(), [1, 64, 32, 16, 8, 4, 2]);
        let tiled = f32s(&lane, true);
        assert_eq!(tiled.len(), 4 * 2, "{numerics:?}: the 64- and 32-row chunks' every f32 projection");
        let mut launched: Vec<&str> = lane
            .program
            .cu
            .match_indices("lk_gemm_f32<<<")
            .map(|(at, _)| &lane.program.cu[at..at + lane.program.cu[at..].find(");").unwrap()])
            .collect();
        assert_eq!(launched.len(), tiled.len(), "{numerics:?}: each in tiles");
        for (k, n, m) in tiled {
            let split = gemm_f32_splits(n.div_ceil(64) * m.div_ceil(64), CudaKnobs::default().chip.sms, k.div_ceil(32));
            let grid = format!("lk_gemm_f32<<<dim3({}, {}, {split}), 256, 0, st>>>(", n.div_ceil(64), m.div_ceil(64));
            let shape = format!(", {m}, {n}, {k}");
            let at = launched
                .iter()
                .position(|l| l.starts_with(&grid) && l.contains(&shape))
                .unwrap_or_else(|| panic!("{numerics:?}: no `{grid}` multiplying {m} × {k} by {n} in {launched:?}"));
            let args = &launched.swap_remove(at)[grid.len()..];
            assert_eq!(args.matches(", ").count(), if split > 1 { 9 } else { 6 }, "{numerics:?}: `{args}` parks its parts when cut");
        }
        assert_eq!(lane.program.cu.matches("lk_gemv_f32<<<").count(), f32s(&lane, false).len(), "{numerics:?}: the rest keep the GEMV");
        if common::require_tool("nvcc") {
            nvcc(&lane.program.cu, &dir().join(format!("lane-wide-f32-{numerics:?}")));
        }
    }
    let precise = wide(Q8MatKind::Dequant);
    assert!(!precise.program.cu.contains("lk_gemm_f32"), "the precise numerics sum every f32 projection in f64");
    assert_eq!(precise.program.cu.matches("lk_gemv_f32<<<").count(), 4 * 6, "every chunk's f32 projections in the GEMV");
    assert_eq!(precise.program.cu.matches("lk_gate_proj<<<").count(), 2, "the decode step's in its gates' launch, which sums them in f64 too");
}

/// The programs the stream locks walk: every tiny architecture at its own width and at four times it in Q8_0 (a
/// convolution's four taps in `f32`), and Qwen3
/// and DeepSeek-V2 at their medium widths in Q4_K — so the wide chunks' GEMMs in parts, paired GEMMs, chunked delta
/// rules and their scratch are among them — each with a decode step and chunks to 64 rows, under every numerics.
fn stream_corpus(tag: &str) -> Vec<(Tiny, String, Q8MatKind, Lane)> {
    let mut sized: Vec<(Tiny, String, ModelSpec, PathBuf)> = Vec::new();
    for &t in ALL {
        let (spec, path) = checkpoint(t, tag);
        sized.push((t, t.name().to_string(), spec, path));
        let (spec, path) = wide_checkpoint(t, tag, &|name| if name.contains("ssm_conv1d") { GgmlType::F32 } else { GgmlType::Q8_0 });
        sized.push((t, format!("{} wide", t.name()), spec, path));
    }
    for t in [Tiny::Qwen3, Tiny::DeepSeek2] {
        let (spec, path) = medium_checkpoint(t, tag, &|_| GgmlType::Q4K);
        sized.push((t, format!("{} medium", t.name()), spec, path));
    }
    let mut out = Vec::new();
    for (t, name, spec, path) in sized {
        for numerics in [Q8MatKind::Int8Act, Q8MatKind::TwoPassInt8, Q8MatKind::Dequant] {
            let lane = compile(&spec, &path, &LaneOptions { ctx: 128, prefill: 64, ..options(numerics) })
                .unwrap_or_else(|e| panic!("{name} {numerics:?}: {e}"));
            out.push((t, name.clone(), numerics, lane));
        }
    }
    out
}

/// Every form's launches over several streams (`branches::branch`) keep every dependency: a launch that writes memory
/// a later one reads or writes, or reads memory a later one writes (`branches::regions`), happens before it — by their
/// stream's order, or by an event the later one's stream waits on after the earlier one's records it, through any
/// chain of those. Every side stream joins the step's own at its end. Over the stream corpus (`stream_corpus`); and the
/// hybrid's decode step does run launches side by side. The arg-max that closes a step writes the token and the
/// position it moves on, so it follows every launch that reads them. Every launch of the program can be held back by
/// `--stagger`'s stall.
#[test]
fn every_branched_launch_follows_every_launch_it_depends_on() {
    use logicaffeine_compile::codegen_cuda::lane::branches::{branch, regions, Region, STREAMS};
    let overlap = |p: &Region, q: &Region| match (p, q) {
        (Region::Arena(a0, a1), Region::Arena(b0, b1)) => a0 < b1 && b0 < a1,
        (Region::Named(a), Region::Named(b)) => a == b,
        _ => false,
    };
    let mut sides = 0;
    for (t, name, numerics, lane) in stream_corpus("branched") {
        let launched = lane.program.cu.matches("{ cudaStream_t st = S[").count();
        assert!(launched > 0, "{name} {numerics:?}: the steps launch on their streams");
        assert_eq!(lane.program.cu.matches("if(stagger){ lk_stall<<<").count(), launched, "{name} {numerics:?}: `--stagger` holds every launch back");
        for f in &lane.forms {
            let what = format!("{name} {numerics:?} {} rows", f.rows);
            let s = &f.schedule;
            let closing: Vec<&Launch> = s.launches.iter().filter(|l| matches!(l, Launch::ArgMax { advance: Some(_), .. })).collect();
            assert_eq!(closing.len(), 1, "{what}: one arg-max closes the step");
            let moved = closing[0].writes();
            assert!(
                moved.contains(&s.io.token.as_str()) && moved.contains(&s.io.position.as_str()),
                "{what}: the closing arg-max writes the token and the position it moves on: {moved:?}"
            );
            let b = branch(s, &f.arena);
            let n = s.launches.len();
            assert!(b.stream.iter().all(|&t| t < STREAMS), "{what}: every launch on one of the streams");
            // What each launch follows: its stream's previous launch and the launch whose event it waits on, and
            // everything those follow.
            let mut recorder = BTreeMap::new();
            for (i, r) in b.records.iter().enumerate() {
                if let Some(e) = r {
                    assert!(recorder.insert(*e, i).is_none(), "{what}: event {e} recorded once");
                }
            }
            let mut before: Vec<Vec<bool>> = Vec::with_capacity(n);
            let mut last_on: Vec<Option<usize>> = vec![None; STREAMS];
            for j in 0..n {
                let mut row = vec![false; n];
                let direct = last_on[b.stream[j]].into_iter().chain(b.waits[j].iter().map(|e| *recorder.get(e).unwrap_or_else(|| panic!("{what}: event {e} recorded"))));
                for i in direct {
                    assert!(i < j, "{what}: launch {j} waits on a later launch {i}");
                    row[i] = true;
                    for (x, &y) in row.iter_mut().zip(&before[i]) {
                        *x |= y;
                    }
                }
                before.push(row);
                last_on[b.stream[j]] = Some(j);
            }
            let mem: Vec<_> = (0..n).map(|i| regions(s, &f.arena, i)).collect();
            for j in 0..n {
                for i in 0..j {
                    let hits = |x: &[Region], y: &[Region]| x.iter().any(|p| y.iter().any(|q| overlap(p, q)));
                    let conflict = hits(&mem[i].1, &mem[j].0) || hits(&mem[i].1, &mem[j].1) || hits(&mem[i].0, &mem[j].1);
                    assert!(!conflict || before[j][i], "{what}: launch {j} ({}) touches what launch {i} ({}) does, unordered", kind(&s.launches[j]), kind(&s.launches[i]));
                }
            }
            // The step's own stream ends after every side stream's last launch.
            for &t in &b.sides {
                let tail = (0..n).rev().find(|&i| b.stream[i] == t).expect("a side stream's launch");
                assert!(b.joins.iter().any(|e| recorder.get(e) == Some(&tail)), "{what}: side stream {t} joins the step's own after its last launch");
            }
            if t == Tiny::Qwen35 && name == t.name() && f.rows == 1 && numerics == Q8MatKind::Int8Act {
                sides = b.sides.len();
            }
        }
    }
    assert!(sides > 0, "the hybrid's decode step runs launches side by side");
}

/// The first byte in the arena of each pointer the program sets there — `p = (T*)(lk_arena + n)`, or an alias of one,
/// `T* p = q + n` (`n` of `T`).
fn arena_bases(cu: &str) -> BTreeMap<&str, usize> {
    let stmts: Vec<&str> = cu.split(';').map(str::trim).collect();
    let mut base: BTreeMap<&str, usize> = stmts
        .iter()
        .filter_map(|st| {
            let (lhs, rhs) = st.split_once(" = (")?;
            let (_, n) = rhs.split_once("*)(lk_arena + ")?;
            Some((lhs.trim(), n.trim_end_matches(')').parse().ok()?))
        })
        .collect();
    let aliases: Vec<(&str, &str, usize)> = stmts
        .iter()
        .filter_map(|st| {
            let (decl, rhs) = st.split_once(" = ")?;
            let (ty, name) = decl.rsplit_once("* ")?;
            let size = match ty.trim() {
                "float" | "int" => 4,
                "signed char" => 1,
                _ => return None,
            };
            let (src, n) = rhs.split_once(" + ")?;
            Some((name, src, n.parse::<usize>().ok()? * size))
        })
        .collect();
    loop {
        let found: Vec<(&str, usize)> =
            aliases.iter().filter(|(name, ..)| !base.contains_key(name)).filter_map(|&(name, src, n)| Some((name, base.get(src)? + n))).collect();
        if found.is_empty() {
            return base;
        }
        base.extend(found);
    }
}

/// The statements of each launch of the step of `rows` rows, in order: the block it runs on its stream.
fn launch_blocks(cu: &str, rows: usize) -> Vec<&str> {
    let head = format!("auto step{rows} = [&](cudaStream_t st){{\n");
    let at = cu.find(&head).unwrap_or_else(|| panic!("the step of {rows} rows")) + head.len();
    let body = &cu[at..at + cu[at..].find("\n  };\n").expect("the step's end")];
    body.split("{ cudaStream_t st = S[").skip(1).map(|b| &b[..b.find("\n  }\n").expect("the block's end")]).collect()
}

/// The memory the streams order launches by is the memory the program's launches touch: every array in the arena a
/// launch's statements name — a step-local tensor's `f32` buffer, its quantized words, low words and scales, a view of
/// one, a scratch — starts inside one of the arena regions `branches::regions` gives the launch. Over the stream corpus
/// (`stream_corpus`).
#[test]
fn every_arena_array_a_launch_names_lies_in_its_regions() {
    use logicaffeine_compile::codegen_cuda::lane::branches::{regions, Region};
    for (_, name, numerics, lane) in stream_corpus("regions") {
        let cu = &lane.program.cu;
        let bases = arena_bases(cu);
        for f in &lane.forms {
            let what = format!("{name} {numerics:?} {} rows", f.rows);
            let s = &f.schedule;
            let launched: Vec<usize> = (0..s.launches.len()).filter(|&i| !matches!(s.launches[i], Launch::View { .. })).collect();
            let blocks = launch_blocks(cu, f.rows);
            assert_eq!(blocks.len(), launched.len(), "{what}: a block of statements a launch");
            let mut named = 0;
            for (block, &i) in blocks.iter().zip(&launched) {
                let (r, w) = regions(s, &f.arena, i);
                for id in block.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')) {
                    if let Some(&b) = bases.get(id) {
                        named += 1;
                        assert!(
                            r.iter().chain(&w).any(|g| matches!(g, Region::Arena(lo, hi) if *lo <= b && b < *hi)),
                            "{what}: launch {i} ({}) names `{id}`, at byte {b} of the arena, in none of its regions {r:?} {w:?}",
                            kind(&s.launches[i])
                        );
                    }
                }
            }
            assert!(named > 0, "{what}: the step's launches name arrays in the arena");
        }
    }
}

#[test]
fn every_knob_changes_the_program() {
    let (spec, path) = checkpoint(Tiny::Qwen3, "knobs");
    let base = options(Q8MatKind::Int8Act);
    let with = |knobs: CudaKnobs| compile(&spec, &path, &LaneOptions { knobs, ..base }).unwrap();
    let lane = with(base.knobs);
    assert!(lane.program.cu.contains("const bool graphs = true;"));
    let ungraphed = with(CudaKnobs { cuda_graphs: false, ..base.knobs });
    assert!(ungraphed.program.cu.contains("const bool graphs = false;"), "cuda_graphs off launches the step directly");
    let unfused = with(CudaKnobs { fuse_quant: false, ..base.knobs });
    assert_eq!(unfused.decode().schedule.fired("fused_quant"), 0);
    assert_eq!(unfused.decode().schedule.census()["quant"], 13, "every quantized activation gets its own launch");
    let precise = compile(&spec, &path, &options(Q8MatKind::Dequant)).unwrap();
    assert_eq!(precise.decode().schedule.census().get("quant"), None, "the precise lane quantizes nothing");
    assert!(precise.decode().schedule.buffers.values().all(|b| !b.quantized));
    assert!(precise.program.cu.contains("lk_gemv_q8<0, 1>") && !precise.program.cu.contains("lk_gemv_q8<1, "));
    let two = compile(&spec, &path, &options(Q8MatKind::TwoPassInt8)).unwrap();
    assert!(two.program.cu.contains("lk_gemv_q8<2, 1>"));
    let quantized = two.decode().arena.placed.iter().filter(|p| p.touch == Touch::Quant).count();
    assert!(quantized > 0, "the decode step's quantized activations live in the arena");
    assert_eq!(two.program.cu.matches("; l1_").count(), quantized, "the two-word numerics point each one's low words into the arena");
    assert_eq!(lane.program.cu.matches("; l1_").count(), 0, "the one-word numerics have none");
}

/// A lane compiled for an RTX 3060 of `mem` bytes, in chunks of up to 16 rows.
fn on_a_chip_of(spec: &ModelSpec, path: &Path, numerics: Q8MatKind, mem: usize) -> Result<Lane, LaneError> {
    let mut opts = chunked(numerics, 16);
    opts.knobs.chip.mem_bytes = mem;
    compile(spec, path, &opts)
}

/// The lane's uploads: each set of checkpoint rows its constant matrices read (the same format, width and file runs),
/// named by the first matrix by name that reads it, with every matrix reading it and the bytes it takes.
fn uploads_of(lane: &Lane) -> BTreeMap<String, (BTreeSet<String>, usize)> {
    let mut by_rows: BTreeMap<(u32, usize, Vec<(u64, usize)>), (BTreeSet<String>, usize)> = BTreeMap::new();
    for b in lane.forms.iter().flat_map(|f| f.schedule.buffers.values()) {
        if let Fill::Constant(Placement::Rows(r)) = &b.fill {
            let runs = r.runs.iter().map(|run| (run.offset, run.rows)).collect();
            by_rows.entry((r.ty.id(), r.cols, runs)).or_insert_with(|| (BTreeSet::new(), matrix_bytes(r.rows, r.cols, r.ty))).0.insert(b.name.clone());
        }
    }
    by_rows.into_values().map(|(names, bytes)| (names.first().unwrap().clone(), (names, bytes))).collect()
}

/// What a step pays to read an upload from the host, by what reads the matrices `names` that read it: 0 for a table
/// only gathered from (a row a token), 1 for an expert stack (a step reads its chosen experts' rows), 2 for any other
/// (a step reads it whole).
fn host_cost(lane: &Lane, names: &BTreeSet<String>) -> u8 {
    lane.forms
        .iter()
        .flat_map(|f| &f.schedule.launches)
        .flat_map(|l| l.parts())
        .flat_map(|l| l.reads().into_iter().filter(|t| names.contains(*t)).map(move |t| (l, t)))
        .map(|(l, t)| match l {
            Launch::Gather { .. } => 0,
            Launch::Moe { wg, wu, wd, .. } if [wg, wu, wd].iter().any(|w| w.as_str() == t) => 1,
            _ => 2,
        })
        .max()
        .unwrap_or_else(|| panic!("{names:?} are read"))
}

/// The value of the program's placement constant `name` (`static const size_t LK_ARENA = 1024UL, …`).
fn constant(cu: &str, name: &str) -> usize {
    let at = cu.find(&format!(" {name} = ")).unwrap_or_else(|| panic!("the program declares {name}")) + name.len() + 4;
    cu[at..].split(|c: char| !c.is_ascii_digit()).next().unwrap().parse().unwrap()
}

/// The program's uploads: each uploaded matrix's name and the rank in the order past which it lives in host memory
/// (`upload_rows(file, R_Wq0, …, lk_moved > 7);`).
fn upload_ranks(cu: &str) -> BTreeMap<String, usize> {
    cu.lines()
        .filter(|l| l.trim_start().starts_with("upload_"))
        .map(|l| {
            let name = l.split("R_").nth(1).and_then(|t| t.split(',').next()).expect("an upload names its runs");
            let rank = l.rsplit("lk_moved > ").next().and_then(|t| t.strip_suffix(");")).unwrap_or_else(|| panic!("{l} says where it lives"));
            (name.to_string(), rank.parse().unwrap())
        })
        .collect()
}

/// A matrix's bytes are what the program carves for it from a pool: a Q8_0 matrix's levels and its half scales, a block
/// format's blocks as the checkpoint lays them out and the 32 bytes a staged read runs past them — an MXFP4 matrix's
/// rows regrouped, 16 sub-blocks a group (272 bytes; a short last group its 16 bytes of exponents and a 16-byte word
/// a sub-block), and the 256 bytes a staged read of a short last group runs past them — each array rounded up to the
/// pool's alignment, `cudaMalloc`'s.
#[test]
fn a_matrix_takes_the_bytes_its_upload_carves() {
    assert_eq!(POOL_ALIGN, 256);
    assert_eq!((pooled(0), pooled(1), pooled(256), pooled(257)), (0, 256, 256, 512));
    assert_eq!(matrix_bytes(4, 64, GgmlType::Q8_0), pooled(4 * 64) + pooled(4 * 2 * 2));
    assert_eq!(matrix_bytes(16, 512, GgmlType::Q8_0), 16 * 512 + 16 * 16 * 2, "arrays whole multiples of the alignment take their bytes");
    assert_eq!(matrix_bytes(3, 256, GgmlType::Q4K), pooled(3 * 144 + 32));
    assert_eq!(matrix_bytes(5, 64, GgmlType::F32), pooled(5 * 64 * 4 + 32));
    assert_eq!(matrix_bytes(2, 32, GgmlType::Mxfp4), pooled(2 * 32 + 256));
    assert_eq!(matrix_bytes(3, 512, GgmlType::Mxfp4), pooled(3 * 272 + 256));
    assert_eq!(matrix_bytes(32 * 2880, 2880, GgmlType::Mxfp4), pooled(32 * 2880 * 1536 + 256), "gpt-oss's experts: five groups and ten sub-blocks a row");
}

/// Every array stays on the device while they fit it, and the program knows its plan: every architecture under every
/// numerics, compiled for the RTX 3060's 12 GiB, keeps every matrix there; matrices reading the same rows (a tied head
/// and its table) share one upload; the whole is the arena, the fixed arrays and every upload, and the arrays but the
/// matrices are what a chip with nothing past the context's share and the program's spare refuses; a chip of exactly
/// the whole compiles the same program; and the program carries the plan — its arena, fixed and spare bytes, every
/// matrix's bytes in the order, and each upload living in host memory past its rank in it.
#[test]
fn every_array_stays_on_the_device_while_the_arrays_fit() {
    for &t in ALL {
        let (spec, path) = checkpoint(t, "resident");
        for numerics in NUMERICS {
            let what = format!("{} {numerics:?}", t.name());
            let lane = on_a_chip_of(&spec, &path, numerics, CudaKnobs::default().chip.mem_bytes).unwrap_or_else(|e| panic!("{what}: {e}"));
            let plan = &lane.plan;
            assert_eq!((lane.residence.moved, lane.residence.host_bytes), (0, 0), "{what}: every matrix on the device");
            assert_eq!(lane.residence.device_bytes, plan.whole(), "{what}");
            let uploads = uploads_of(&lane);
            let shares: BTreeMap<String, String> =
                uploads.iter().flat_map(|(first, (names, _))| names.iter().filter(move |n| *n != first).map(move |n| (n.clone(), first.clone()))).collect();
            assert_eq!(plan.shares, shares, "{what}: matrices reading the same rows share their upload");
            if spec.tied_output {
                assert_eq!(shares, BTreeMap::from([("Wout".to_string(), "Emb".to_string())]), "{what}: the tied head reads the table's upload");
            } else {
                assert_eq!(shares, BTreeMap::new(), "{what}: every matrix reads rows of its own");
            }
            let ordered: BTreeMap<String, usize> = plan.order.iter().cloned().collect();
            assert_eq!(ordered.len(), plan.order.len(), "{what}: each matrix once in the order");
            assert_eq!(ordered, uploads.iter().map(|(m, (_, b))| (m.clone(), *b)).collect(), "{what}: the order holds every upload and its bytes");
            assert_eq!(plan.arena, lane.forms.iter().map(|f| f.arena.bytes).max().unwrap(), "{what}: the largest form's arena");
            let Err(LaneError::Memory { needed, budget: 0 }) = on_a_chip_of(&spec, &path, numerics, CONTEXT_RESERVE + plan.spare) else {
                panic!("{what}: a chip of the context's share and the spare holds no array")
            };
            assert_eq!(needed, plan.arena + plan.fixed, "{what}: the arrays that cannot move");
            let snug = on_a_chip_of(&spec, &path, numerics, CONTEXT_RESERVE + plan.spare + plan.whole()).unwrap();
            assert_eq!(snug.residence, lane.residence, "{what}: a chip of the whole keeps every matrix");
            assert_eq!(snug.program.cu, lane.program.cu, "{what}: and emits the same program");
            let cu = &lane.program.cu;
            assert_eq!(
                [constant(cu, "LK_ALIGN"), constant(cu, "LK_ARENA"), constant(cu, "LK_FIXED"), constant(cu, "LK_SPARE"), constant(cu, "LK_MOVABLE")],
                [POOL_ALIGN, plan.arena, plan.fixed, plan.spare, plan.order.len()],
                "{what}: the program's plan"
            );
            let table = format!("LK_ORDER[] = {{{}}};", plan.order.iter().map(|(_, b)| format!("{b}UL")).collect::<Vec<_>>().join(", "));
            assert!(cu.contains(&table), "{what}: the program's order {table}");
            let ranks: BTreeMap<String, usize> = plan.order.iter().enumerate().map(|(i, (m, _))| (m.clone(), i)).collect();
            assert_eq!(upload_ranks(cu), ranks, "{what}: each upload lives in host memory past its rank");
        }
    }
}

/// A matrix in host memory that a launch reads whole streams to the device a launch at a time (`residence::Plan::streamed`;
/// the program's `lk_stage_in`, `lk_stage_go`, `lk_stage_out`): every tiny architecture's lane, compiled with chunks of four rows under
/// every numerics, streams exactly the matrices some launch reads whole — not a table only gathered from, not an expert
/// stack — each with its arrays' bytes (a Q8_0 matrix's levels and scales); its slot is the most bytes one launch's
/// streamed matrices take; its copies are the arrays each step's graph stages, the lean step's too; and its program
/// says the same: which of its order streams (`LK_STREAMS`), its slot, a staged array before each launch that reads one
/// under the matrix's rank (`lk_moved > rank`), and the copy stream forked and joined by each step that stages.
#[test]
fn a_lane_streams_each_matrix_it_reads_whole_from_host_memory_through_two_slots() {
    use logicaffeine_compile::codegen_cuda::lane::emit::output_only;
    use logicaffeine_compile::codegen_cuda::lane::residence::matrix_arrays;
    for &t in ALL {
        let (spec, path) = checkpoint(t, "stream");
        for numerics in NUMERICS {
            let what = format!("{} {numerics:?}", t.name());
            let lane = compile(&spec, &path, &chunked(numerics, 4)).unwrap_or_else(|e| panic!("{what}: {e}"));
            let plan = &lane.plan;
            let up = |n: &str| plan.shares.get(n).cloned().unwrap_or_else(|| n.to_string());
            let mut rows: BTreeMap<String, (GgmlType, usize, usize)> = BTreeMap::new();
            for f in &lane.forms {
                for b in f.schedule.buffers.values() {
                    if let Fill::Constant(Placement::Rows(r)) = &b.fill {
                        rows.entry(up(&b.name)).or_insert((r.ty, r.rows, r.cols));
                    }
                }
            }
            let (mut whole, mut sparse) = (BTreeSet::new(), BTreeSet::new());
            for l in lane.forms.iter().flat_map(|f| f.schedule.launches.iter().flat_map(|l| l.parts())) {
                let experts: Vec<String> = match l {
                    Launch::Moe { wg, wu, wd, .. } => vec![up(wg), up(wu), up(wd)],
                    _ => Vec::new(),
                };
                for m in l.reads().into_iter().map(up).filter(|m| rows.contains_key(m)) {
                    if matches!(l, Launch::Gather { .. }) || experts.contains(&m) {
                        sparse.insert(m);
                    } else {
                        whole.insert(m);
                    }
                }
            }
            let expected: BTreeMap<String, Vec<usize>> = whole
                .difference(&sparse)
                .map(|m| {
                    let (ty, r, c) = rows[m];
                    (m.clone(), matrix_arrays(r, c, ty))
                })
                .collect();
            assert!(!expected.is_empty(), "{what}: some matrix is read whole");
            assert_eq!(plan.streamed, expected, "{what}: the matrices read whole stream, each with its arrays");
            let staged = |l: &Launch| -> Vec<String> {
                let mut seen = BTreeSet::new();
                l.parts().into_iter().flat_map(|p| p.reads()).filter(|t| plan.streamed.contains_key(&up(t)) && seen.insert(t.to_string())).map(str::to_string).collect()
            };
            let launches = lane.forms.iter().flat_map(|f| f.schedule.launches.iter());
            let slot = launches.clone().map(|l| staged(l).iter().map(|t| plan.streamed[&up(t)].iter().sum::<usize>()).sum::<usize>()).max().unwrap();
            assert_eq!(plan.slot, slot, "{what}: a slot holds the widest launch's streamed matrices");
            let arrays = |l: &Launch| staged(l).iter().map(|t| plan.streamed[&up(t)].len()).sum::<usize>();
            let copies: usize = lane
                .forms
                .iter()
                .map(|f| {
                    let lean = output_only(f, &lane.forms);
                    let all: usize = f.schedule.launches.iter().map(arrays).sum();
                    all + if lean.iter().any(|&x| x) { f.schedule.launches.iter().zip(&lean).filter(|(_, &o)| !o).map(|(l, _)| arrays(l)).sum() } else { 0 }
                })
                .sum();
            assert_eq!(plan.copies, copies, "{what}: the copies every graph stages, the lean steps' too");
            let cu = &lane.program.cu;
            let table = format!("LK_STREAMS[] = {{{}}};", plan.order.iter().map(|(m, _)| if plan.streamed.contains_key(m) { "1" } else { "0" }).collect::<Vec<_>>().join(", "));
            assert!(cu.contains(&table), "{what}: the program's {table}");
            assert!(cu.contains(&format!("LK_SLOT = {}UL,", plan.slot)), "{what}: the program's slot");
            let staged_launches: usize = launches.clone().filter(|l| !staged(l).is_empty()).count();
            let staged_arrays: usize = launches.clone().map(arrays).sum();
            assert_eq!(cu.matches("lk_stage_in((void**)&").count(), staged_arrays, "{what}: an array staged for each launch that reads it");
            assert_eq!(cu.matches("  lk_stage_go(st);\n").count(), staged_launches, "{what}: each staging launch waits for its copies");
            assert_eq!(cu.matches("  lk_stage_out(st);\n").count(), staged_launches, "{what}: and frees its slot after it");
            let ranks: BTreeMap<&str, usize> = plan.order.iter().enumerate().map(|(i, (m, _))| (m.as_str(), i)).collect();
            for m in plan.streamed.keys() {
                assert!(cu.contains(&format!("if(lk_streaming && lk_moved > {}){{", ranks[m.as_str()])), "{what}: {m} stages under its rank");
            }
            let forked = lane.forms.iter().filter(|f| f.schedule.launches.iter().any(|l| !staged(l).is_empty())).count();
            assert_eq!(cu.matches("  lk_stage_fork(st);\n").count(), forked, "{what}: each staging step forks its copies");
            assert_eq!(cu.matches("  lk_stage_join(st);\n").count(), forked, "{what}: and joins them");
        }
    }
}

/// The matrices leave the device in the order a step misses them least: on a budget a byte under what the last one
/// kept on the device, exactly one more matrix (those reading the same rows as one) moves to mapped host memory, until
/// every matrix has and a byte less refuses by name. The order: each table only gathered from, then each expert stack,
/// then every other matrix, and within each class from the decode step's last read back (the last layer's first). At
/// every budget the device bytes fit it, the two sides sum to the whole, and the host bytes are the moved matrices';
/// and a lane compiled for a chip of a budget places its matrices as the budget does. Every tiny architecture, and the
/// expert model with its head tied, whose table the head reads whole: its expert stacks leave before it.
#[test]
fn the_matrices_leave_the_device_in_the_order_a_step_misses_them_least() {
    let mut tied = Tiny::Qwen3Moe.spec(CTX);
    tied.tied_output = true;
    let tied_path = dir().join("qwen3moe-tied-evict.gguf");
    tiny_models::write_checkpoint(&tied, &tied_path);
    let mut models: Vec<(String, ModelSpec, PathBuf)> = ALL
        .iter()
        .map(|&t| {
            let (spec, path) = checkpoint(t, "evict");
            (t.name().to_string(), spec, path)
        })
        .collect();
    models.push(("qwen3moe-tied".to_string(), tied, tied_path));
    std::thread::scope(|s| {
        for (name, spec, path) in &models {
            s.spawn(move || leaves_in_the_order_a_step_misses_least(name, spec, path));
        }
    });
}

/// [`the_matrices_leave_the_device_in_the_order_a_step_misses_them_least`] for the model `spec`, its checkpoint at `path`.
fn leaves_in_the_order_a_step_misses_least(name: &str, spec: &ModelSpec, path: &Path) {
    for numerics in NUMERICS {
        let what = format!("{name} {numerics:?}");
        let lane = on_a_chip_of(spec, path, numerics, CudaKnobs::default().chip.mem_bytes).unwrap_or_else(|e| panic!("{what}: {e}"));
        let plan = &lane.plan;
        let whole = plan.whole();
        let matrices = uploads_of(&lane);
        let launches = &lane.decode().schedule.launches;
        let last_read: BTreeMap<String, Option<usize>> = matrices
            .iter()
            .map(|(m, (names, _))| (m.clone(), launches.iter().rposition(|l| l.reads().iter().any(|t| names.contains(*t)))))
            .collect();
        let mut order: Vec<String> = Vec::new();
        let mut r = place(plan, whole).unwrap();
        assert_eq!((r.moved, r.device_bytes, r.host_bytes), (0, whole, 0), "{what}: the whole fits a budget of the whole");
        while order.len() < matrices.len() {
            let budget = r.device_bytes - 1;
            r = place(plan, budget).unwrap_or_else(|e| panic!("{what}: {} of {} matrices moved: {e}", order.len(), matrices.len()));
            let moved: Vec<&String> = r.host.iter().filter(|m| !order.contains(m)).collect();
            assert_eq!(moved.len(), 1, "{what}: a byte less moves one matrix more: {moved:?}");
            assert!(order.iter().all(|m| r.host.contains(m)), "{what}: a moved matrix stays moved");
            order.push(moved[0].clone());
            assert_eq!(r.moved, order.len(), "{what}");
            assert!(r.device_bytes <= budget, "{what}: {} device bytes on a budget of {budget}", r.device_bytes);
            assert_eq!(r.device_bytes + r.host_bytes, whole, "{what}: the two sides hold the lane's arrays");
            assert_eq!(r.host_bytes, r.host.iter().map(|m| matrices[m].1).sum::<usize>(), "{what}: the host holds the moved matrices");
        }
        assert_eq!(order, plan.order.iter().map(|(m, _)| m.clone()).collect::<Vec<_>>(), "{what}: the walk moves the plan's order");
        let fixed = r.device_bytes;
        assert_eq!(fixed, plan.arena + plan.fixed, "{what}: with every matrix moved the device holds the rest");
        let e = place(plan, fixed - 1).expect_err("a byte under the arrays but the matrices refuses");
        assert_eq!(e, LaneError::Memory { needed: fixed, budget: fixed - 1 }, "{what}");
        assert_eq!(
            e.to_string(),
            format!("the caches, arena and buffers take {fixed} bytes of the device's {}, every matrix moved to the host", fixed - 1)
        );
        let half = fixed + (whole - fixed) / 2;
        let on_half = on_a_chip_of(spec, path, numerics, CONTEXT_RESERVE + plan.spare + half).unwrap();
        assert_eq!(on_half.residence, place(plan, half).unwrap(), "{what}: the chip's budget places as the budget does");
        assert!(on_half.residence.moved > 0 && on_half.residence.moved < order.len(), "{what}: half the matrices' bytes moves some");
        let costs: Vec<(u8, std::cmp::Reverse<Option<usize>>)> =
            order.iter().map(|m| (host_cost(&lane, &matrices[m].0), std::cmp::Reverse(last_read[m]))).collect();
        assert!(costs.windows(2).all(|w| w[0] <= w[1]), "{what}: the order a step misses them least: {order:?} {costs:?}");
        let gathered = order.iter().take_while(|m| host_cost(&lane, &matrices[*m].0) == 0).count();
        if spec.tied_output {
            assert_eq!(gathered, 0, "{what}: a tied table is read whole by the head");
        } else {
            assert_eq!(gathered, 1, "{what}: the embedding table goes first");
        }
        let experts = order.iter().filter(|m| host_cost(&lane, &matrices[*m].0) == 1).count();
        let moe_layers = spec.layers.iter().filter(|l| matches!(l.ffn, Ffn::Moe(_))).count();
        assert_eq!(experts, 3 * moe_layers, "{what}: three expert stacks a mixture layer");
        if spec.tied_output && moe_layers > 0 {
            assert_eq!(order[experts], "Emb", "{what}: the tied table leaves right after the expert stacks: {order:?}");
        }
    }
}

/// The rows of a projection, in the order its binding picks them, at their byte offsets in the file.
fn rows_of(lane: &Lane, var: &str) -> Rows {
    match &lane.decode().schedule.buffers[var].fill {
        Fill::Constant(Placement::Rows(r)) => r.clone(),
        other => panic!("{var} is not read as rows: {other:?}"),
    }
}

fn picked_rows(g: &Gguf, r: &Rows) -> Vec<usize> {
    let info = g.info(&r.tensor).unwrap();
    let row_bytes = r.ty.bytes_for(r.cols) as u64;
    let base = g.data_start() + info.offset;
    r.runs
        .iter()
        .flat_map(|run| {
            assert_eq!((run.offset - base) % row_bytes, 0, "{}: a run starts on a row", r.tensor);
            let first = ((run.offset - base) / row_bytes) as usize;
            first..first + run.rows
        })
        .collect()
}

#[test]
fn every_projection_is_read_in_place_in_its_bindings_row_order() {
    let (_, path) = checkpoint(Tiny::Llama, "rows");
    let llama = compile(&Tiny::Llama.spec(CTX), &path, &options(Q8MatKind::Int8Act)).unwrap();
    let g = Gguf::open(&path).unwrap();
    let wq = rows_of(&llama, "Wq0");
    assert!(wq.transposed && wq.rows == 64 && wq.cols == 64 && wq.ty == GgmlType::Q8_0);
    let neox: Vec<usize> = (0..64).map(|r| (r / 16) * 16 + if r % 16 < 8 { 2 * (r % 16) } else { 2 * (r % 16 - 8) + 1 }).collect();
    assert_eq!(picked_rows(&g, &wq), neox, "an interleaved rotation's query rows are read in NEOX order");
    let emb = rows_of(&llama, "Emb");
    assert!(!emb.transposed && emb.rows == tiny_models::VOCAB && emb.runs.len() == 1, "the table is read whole, row for row");

    let (_, path) = checkpoint(Tiny::Qwen35, "rows");
    let hybrid = compile(&Tiny::Qwen35.spec(CTX), &path, &options(Q8MatKind::Int8Act)).unwrap();
    let g = Gguf::open(&path).unwrap();
    let wq = rows_of(&hybrid, "Wq2");
    assert_eq!(picked_rows(&g, &wq), (0..32).chain(64..96).collect::<Vec<_>>(), "the queries of each gated head");
    let wqg = rows_of(&hybrid, "Wqg2");
    assert_eq!(picked_rows(&g, &wqg), (32..64).chain(96..128).collect::<Vec<_>>(), "the gates of each gated head");
    assert_eq!(picked_rows(&g, &rows_of(&hybrid, "Wk0")), (64..128).collect::<Vec<_>>(), "the keys of the fused q|k|v");
    assert_eq!(picked_rows(&g, &rows_of(&hybrid, "Wv1")), (128..256).collect::<Vec<_>>(), "the values of the fused q|k|v");
    assert_eq!(rows_of(&hybrid, "Wv1").runs.len(), 1, "a range of rows is one run");
}

#[test]
fn computed_constants_are_the_bindings_values() {
    let (spec, path) = checkpoint(Tiny::Qwen35, "values");
    let lane = compile(&spec, &path, &options(Q8MatKind::Int8Act)).unwrap();
    let mut g = Gguf::open(&path).unwrap();
    let held = |var: &str| match &lane.decode().schedule.buffers[var].fill {
        Fill::Constant(Placement::Values(v)) => v.clone(),
        other => panic!("{var} is not a computed constant: {other:?}"),
    };
    let bound = |g: &mut Gguf, var: &str| -> Vec<f32> {
        let Source::Weight(b) = bind(&spec, var).unwrap() else { panic!("{var} is a weight") };
        values(&b, g).unwrap().into_iter().map(|v| v as f32).collect()
    };
    for var in ["an0", "Gn1", "Dt0", "Al1", "Wcq0", "Wcv1", "Qn2", "on"] {
        assert_eq!(held(var), bound(&mut g, var), "{var}");
    }
    let wb = bound(&mut g, "Wb0");
    let (k, n) = (spec.d, 4);
    let transposed: Vec<f32> = (0..n).flat_map(|o| (0..k).map(move |i| (i, o))).map(|(i, o)| wb[i * n + o]).collect();
    assert_eq!(held("Wb0"), transposed, "an f32 projection is stored a row per output");
    let rope = match &spec.layers[2].mixer {
        Mixer::Attention(a) => a.rope.clone().unwrap(),
        _ => unreachable!(),
    };
    let (cos, sin) = rope_tables(&rope, CTX, None);
    assert_eq!(held("Rcos"), cos.iter().map(|&v| v as f32).collect::<Vec<_>>());
    assert_eq!(held("Rsin"), sin.iter().map(|&v| v as f32).collect::<Vec<_>>());
}

#[test]
fn what_the_lane_cannot_compile_is_refused_by_name() {
    let (spec, path) = checkpoint(Tiny::Qwen3, "refuse");
    let mut fused = spec.clone();
    for layer in &mut fused.layers {
        if let Mixer::Attention(a) = &mut layer.mixer {
            a.fused_qkv = true;
        }
    }
    assert!(matches!(compile(&fused, &path, &options(Q8MatKind::Int8Act)), Err(LaneError::NotYet(_))));

    for prefill in [3, 32, 1024] {
        assert_eq!(
            compile(&spec, &path, &chunked(Q8MatKind::Int8Act, prefill)).unwrap_err(),
            LaneError::Chunk { rows: prefill, ctx: CTX },
            "a chunk that is no power of two, longer than the caches, or past the largest"
        );
    }
    let long = LaneOptions { ctx: 30_000, ..options(Q8MatKind::Int8Act) };
    let lane = compile(&spec, &path, &long).expect("a cache of any length is attended in chunks");
    assert!(
        lane.decode().schedule.launches.iter().any(|l| matches!(l, Launch::Attention { splits, .. } if *splits > 1)),
        "the long cache's span is cut into chunks"
    );
    let wide = "## Main\nLet tok be 1.\nLet pos be 1.\nLet Q be a 8 by 30000 tensor of Real.\nLet Kc be a 4 by 30000 tensor of Real.\n\
                Let Vc be a 4 by 30000 tensor of Real.\nLet q be row tok of Q.\nSet row pos of Kc to q.\nSet row pos of Vc to q.\n\
                Let logits be attention of q over Kc and Vc with 1 heads 1 groups up to pos.\nLet next be argmax of logits.\n";
    match plan_of(wide) {
        Err(LaneError::NoKernel { kind: "Attention", why, .. }) => assert!(why.contains("shared memory"), "{why}"),
        other => panic!("a head past one block's shared memory is refused: {:?}", other.map(|s| s.census())),
    }
    let odd = wide.replace("30000", "30");
    match plan_of(&odd) {
        Err(LaneError::NoKernel { kind: "Attention", why, .. }) => assert!(why.contains("four-value words"), "{why}"),
        other => panic!("a key head the vector loads cannot read is refused: {:?}", other.map(|s| s.census())),
    }
}

/// A wide chunk in a block format multiplies on the int8 tensor cores too (`lk_gemm_blk`): a format of 32-value scales
/// and one whose halves scale apart, in the 64- and 32-row chunks. Each layer's query, key and value — projections of
/// one activation, the query and key each into a buffer of its own, the value into the cache's rows — run as one GEMM in
/// parts; its gated pair as one paired GEMM, the SwiGLU in its epilogue; its output and down projections a GEMM each.
/// The 16-row
/// chunk keeps the block GEMV and its gated pairs; the precise numerics keep the GEMV's slabs. Every program builds.
#[test]
fn a_wide_chunk_in_a_block_format_runs_on_the_tensor_cores() {
    for format in [GgmlType::Q4K, GgmlType::Q6K] {
        let (spec, path) = wide_checkpoint(Tiny::Qwen3, &format!("gemm-{}", format.name()), &|_| format);
        let what = format.name();
        let wide = |numerics| compile(&spec, &path, &LaneOptions { ctx: 128, prefill: 64, ..options(numerics) }).unwrap();
        let fast = wide(Q8MatKind::Int8Act);
        let id = format.id();
        // A dense GEMM's launch, with the general epilogue or the plain one (`blk_plain`: its sums into an `f32` array).
        let dense = |q: u32| [format!("lk_gemm_blk<{id}, {q}>"), format!("lk_gemm_blk<{id}, {q}, 0, 128, 0, true>")];
        let in_parts = |q: u32| [format!("lk_gemm_blk<{id}, {q}, 4>"), format!("lk_gemm_blk<{id}, {q}, 4, 128, 0, true>")];
        let launched = |cu: &str, q: u32| dense(q).iter().map(|k| cu.matches(&format!("{k}<<<")).count()).sum::<usize>();
        let launched_in_parts = |cu: &str, q: u32| in_parts(q).iter().map(|k| cu.matches(&format!("{k}<<<")).count()).sum::<usize>();
        // Each of the three layers in the 64- and 32-row chunks: its output and down GEMMs, its query, key and value in
        // parts, its gated pair paired.
        assert_eq!(launched(&fast.program.cu, 1), 6 * 2, "{what}: the wide chunks' projections on the tensor cores");
        assert_eq!(launched_in_parts(&fast.program.cu, 1), 3 * 2, "{what}: each query, key and value in parts");
        assert_eq!(fast.program.cu.matches(&format!("lk_gemm_blk<{id}, 1, 5, 128, -1><<<")).count(), 3 * 2, "{what}: each gated pair paired");
        assert!(
            dense(1).iter().any(|k| fast.program.cu.contains(&format!("cudaFuncSetAttribute({k}, "))),
            "{what}: the dense GEMM with the shared memory its stage needs"
        );
        assert!(
            in_parts(1).iter().any(|k| fast.program.cu.contains(&format!("cudaFuncSetAttribute({k}, "))),
            "{what}: the GEMM in parts with the shared memory its stage needs"
        );
        for kernel in [format!("lk_gemm_blk<{id}, 1, 5, 128, -1>")] {
            assert!(fast.program.cu.contains(&format!("cudaFuncSetAttribute({kernel}, ")), "{what}: {kernel} with the shared memory its stage needs");
        }
        assert_eq!(fast.program.cu.matches(&format!("lk_gemv_glu<{id}, {id}, 1, 16")).count(), 3, "{what}: the 16-row chunk's gated pairs");
        let chunk = &fast.chunk(64).unwrap().schedule;
        assert_eq!(chunk.fired("glu_pair"), 3, "{what}: each layer's gated pair one paired GEMM");
        assert_eq!(chunk.census().get("gemm_parts"), Some(&3), "{what}: each layer's query, key and value one GEMM in parts");
        assert!(
            chunk.launches.iter().all(|l| !matches!(l, Launch::Gemv { y, .. } if y.row.is_some())),
            "{what}: no projection writes the cache's rows on its own"
        );
        let two = wide(Q8MatKind::TwoPassInt8);
        assert_eq!(launched(&two.program.cu, 2), 6 * 2, "{what}: two words alike");
        assert_eq!(launched_in_parts(&two.program.cu, 2), 3 * 2, "{what}: two words in parts");
        assert_eq!(two.program.cu.matches(&format!("lk_gemm_blk<{id}, 2, 5, 128, -1><<<")).count(), 3 * 2, "{what}: two words paired");
        let precise = wide(Q8MatKind::Dequant);
        assert!(!precise.program.cu.contains("lk_gemm_blk"), "{what}: the precise numerics never take the int8 tensor cores");
        if common::require_tool("nvcc") {
            nvcc(&fast.program.cu, &dir().join(format!("lane-wide-{what}")));
            nvcc(&two.program.cu, &dir().join(format!("lane-wide2-{what}")));
        }
    }
}

/// A wide chunk's expert mixtures multiply their experts on the int8 tensor cores (the grouped `lk_gemm_blk`): over the
/// gathered rows of each expert's choices its gate and up GEMMs paired in one launch, the gated activation (the clamped
/// one under gpt-oss) and its quantizer in their epilogue, and its down GEMM weighing each choice's row by its weight —
/// in a format of 32-value scales, one whose halves scale apart, MXFP4 and Q8_0 alike, with the shared memory each
/// stage needs. A gate and an up in two formats keep two GEMMs and the gated activation apart (`lk_moe_glu`). They route
/// their rows by one GEMM of logits — `lk_gemm_f32` for gpt-oss's `f32` router, the block GEMM for a router in a
/// quantized format — and a pick a row (`lk_moe_pick`). A chunk of at most sixteen rows keeps the slabs and the
/// router a block a row, and the precise numerics keep them in every chunk. Every program builds.
#[test]
fn a_wide_chunks_experts_run_on_the_tensor_cores() {
    // (architecture, every matrix's format but the experts' up, the experts' up's, layers of experts)
    for (t, format, up, layers) in [
        (Tiny::Qwen3Moe, GgmlType::Q4K, GgmlType::Q4K, 3),
        (Tiny::DeepSeek2, GgmlType::Q6K, GgmlType::Q6K, 2),
        (Tiny::GptOss, GgmlType::Mxfp4, GgmlType::Mxfp4, 3),
        (Tiny::Qwen3Moe, GgmlType::Q8_0, GgmlType::Q8_0, 3),
        (Tiny::Qwen3Moe, GgmlType::Q4K, GgmlType::Q6K, 3),
    ] {
        let (spec, path) = wide_checkpoint(t, &format!("experts-{}-{}", format.name(), up.name()), &|n| {
            if n.contains("ffn_up_exps") {
                up
            } else {
                format
            }
        });
        let what = format!("{} {} up {}", t.name(), format.name(), up.name());
        let wide = |numerics| {
            compile(&spec, &path, &LaneOptions { ctx: 128, prefill: 64, ..options(numerics) }).unwrap_or_else(|e| panic!("{what}: {e}"))
        };
        let (id, uid, act) = (format.id(), up.id(), u32::from(t == Tiny::GptOss));
        for (numerics, m) in [(Q8MatKind::Int8Act, 1), (Q8MatKind::TwoPassInt8, 2)] {
            let lane = wide(numerics);
            let cu = &lane.program.cu;
            let chunks = |wide: bool| lane.forms.iter().filter(|f| f.rows > 1 && (f.rows > 16) == wide).count();
            assert_eq!(chunks(true), 2, "{what}: the 64- and 32-row chunks");
            let (on_cores, slabs) = (chunks(true) * layers, chunks(false) * layers);
            if format == up {
                // The paired GEMM's name spells its tile's rows where it names a gated activation other than SwiGLU.
                let paired = if act == 1 { format!("lk_gemm_blk<{id}, {m}, 3, 64, 1>") } else { format!("lk_gemm_blk<{id}, {m}, 3>") };
                assert_eq!(
                    cu.matches(&format!("{paired}<<<")).count(),
                    on_cores,
                    "{what} {numerics:?}: each wide chunk's mixtures' gate and up paired, the gated activation and its quantizer in their epilogue"
                );
                assert!(!cu.contains(&format!("lk_gemm_blk<{id}, {m}, 1>")), "{what} {numerics:?}: no gate or up GEMM apart");
                assert!(!cu.contains("lk_moe_glu<"), "{what} {numerics:?}: no gated activation apart");
                assert!(cu.contains(&format!("cudaFuncSetAttribute({paired}")), "{what} {numerics:?}: the paired GEMM with the shared memory its stage needs");
            } else {
                for f in [id, uid] {
                    assert_eq!(cu.matches(&format!("lk_gemm_blk<{f}, {m}, 1><<<")).count(), on_cores, "{what} {numerics:?}: the gate and the up GEMMs apart");
                    assert!(cu.contains(&format!("cudaFuncSetAttribute(lk_gemm_blk<{f}, {m}, 1>")), "{what} {numerics:?}: each with its shared memory");
                }
                assert!(!cu.contains(&format!("lk_gemm_blk<{id}, {m}, 3")), "{what} {numerics:?}: no paired GEMM");
                assert_eq!(cu.matches(&format!("lk_moe_glu<{act}><<<")).count(), on_cores, "{what} {numerics:?}: their gated activations");
            }
            assert_eq!(cu.matches(&format!("lk_gemm_blk<{id}, {m}, 2><<<")).count(), on_cores, "{what} {numerics:?}: their down GEMMs");
            assert!(
                cu.contains(&format!("cudaFuncSetAttribute(lk_gemm_blk<{id}, {m}, 2>")),
                "{what} {numerics:?}: the down GEMM with the shared memory its stage needs"
            );
            assert_eq!(
                cu.matches(&format!("lk_moe_up_rows<{m}, {act}, {id}, {uid}><<<")).count(),
                slabs,
                "{what} {numerics:?}: a chunk of at most sixteen rows keeps the slabs"
            );
            assert_eq!(cu.matches(&format!("lk_moe_down_rows<{m}, {id}><<<")).count(), slabs, "{what} {numerics:?}: and its down slabs");
            assert_eq!(cu.matches(&format!("lk_moe_pick<{m}><<<")).count(), on_cores, "{what} {numerics:?}: each wide chunk's mixtures pick from their logits");
            let logits = if t == Tiny::GptOss { "lk_gemm_f32<<<".to_string() } else { format!("lk_gemm_blk<{id}, {m}><<<") };
            // Each pick's launch is preceded (its launch check aside) by the GEMM of the logits it picks from.
            let lines: Vec<&str> = cu.lines().map(str::trim).filter(|l| !l.is_empty() && *l != "CKL();").collect();
            let router_gemms = lines
                .windows(2)
                .filter(|w| w[1].starts_with(&format!("lk_moe_pick<{m}><<<")) && w[0].starts_with(&logits))
                .count();
            assert_eq!(router_gemms, on_cores, "{what} {numerics:?}: their logits one GEMM, right before each pick");
            assert_eq!(cu.matches(&format!("lk_moe_route<{m}, ")).count(), slabs + layers, "{what} {numerics:?}: the narrow chunks and the decode step route a block a row");
            if common::require_tool("nvcc") {
                nvcc(cu, &dir().join(format!("lane-experts-{}-{}-{m}", t.name(), format.name())));
            }
        }
        let precise = wide(Q8MatKind::Dequant);
        let chunks = precise.forms.iter().filter(|f| f.rows > 1).count();
        assert!(!precise.program.cu.contains("lk_moe_glu<"), "{what}: the precise numerics never take the int8 tensor cores");
        assert!(!precise.program.cu.contains("lk_gemm_blk"), "{what}: nor a grouped GEMM");
        assert_eq!(
            precise.program.cu.matches(&format!("lk_moe_up_rows<0, {act}, {id}, {uid}><<<")).count(),
            chunks * layers,
            "{what}: every chunk's mixtures in slabs"
        );
    }
}

/// A wide chunk attends in tiles under the quantized numerics, every attention of the 64- and 32-row chunks — grouped
/// heads, gpt-oss's windows and sinks, and heads 256 wide, on the f16 tensor cores under the one-word numerics
/// (`lk_attention_mma`, heads and values whole multiples of 16 at most 256 wide) and on the int8 tensor cores in two
/// words under the two-word numerics (`lk_attention_i8`, whole multiples of 32 at most 256 wide, each after the launch
/// that takes its keys and values into two words, `lk_kv_two_words`); in the general tiles (`lk_attention_tiled`) past
/// those: DeepSeek's
/// latent attention expanded, each head over its own 288-wide keys and its values, and heads 320 wide. The 16-row chunk
/// and the decode step keep the attention whose
/// rows each take their own blocks — DeepSeek's latent attention every head in one block, each cached row read once for
/// all of them (`lk_attention_shared`, its chunks folded by `lk_attention_fold`) — and so do the precise numerics in
/// every form. The flash attention's shared memory fits every chip's block at every width it takes. Every program
/// builds.
#[test]
fn a_wide_chunk_attends_in_tiles() {
    for t in [Tiny::Qwen3, Tiny::GptOss, Tiny::DeepSeek2] {
        let (spec, path) = wide_checkpoint(t, "tiles", &|_| GgmlType::Q4K);
        let what = t.name();
        let wide = |numerics| compile(&spec, &path, &LaneOptions { ctx: 128, prefill: 64, ..options(numerics) }).unwrap_or_else(|e| panic!("{what}: {e}"));
        let attentions = |f: &logicaffeine_compile::codegen_cuda::lane::Form| f.schedule.launches.iter().filter(|l| matches!(l, Launch::Attention { .. })).count();
        for numerics in [Q8MatKind::Int8Act, Q8MatKind::TwoPassInt8] {
            let lane = wide(numerics);
            let (tiled, apart): (usize, usize) = lane.forms.iter().fold((0, 0), |(t, a), f| if f.rows > 16 { (t + attentions(f), a) } else { (t, a + attentions(f)) });
            assert_eq!(tiled, 3 * 2, "{what} {numerics:?}: each layer's attention in the 64- and 32-row chunks");
            let launches = |kernel: &str| lane.program.cu.lines().filter(|l| l.trim_start().starts_with(kernel) && l.contains("<<<")).count();
            // DeepSeek's expanded heads here are 288 wide (256 unrotated and 32 rotated), past the flash attention's.
            let (mma, two, general) = match (t, numerics) {
                (Tiny::DeepSeek2, _) => (0, 0, tiled),
                (_, Q8MatKind::Int8Act) => (tiled, 0, 0),
                _ => (0, tiled, 0),
            };
            assert_eq!(launches("lk_attention_mma<"), mma, "{what} {numerics:?}: the one-word numerics attend on the f16 tensor cores");
            assert_eq!(launches("lk_attention_i8<"), two, "{what} {numerics:?}: the two-word numerics attend on the int8 tensor cores in two words");
            assert_eq!(launches("lk_kv_two_words<"), two, "{what} {numerics:?}: each after its keys and values are taken into two words");
            assert_eq!(launches("lk_attention_flash<"), 0, "{what} {numerics:?}: nothing attends flash style");
            assert_eq!(launches("lk_attention_tiled<"), general, "{what} {numerics:?}: wider heads in the general tiles");
            let shared = t == Tiny::DeepSeek2;
            let (plain, rows_once) = if shared { (0, apart) } else { (apart, 0) };
            assert_eq!(launches("lk_attention<"), plain, "{what} {numerics:?}: the narrow forms keep the attention a block a row");
            assert_eq!(launches("lk_attention_shared<"), rows_once, "{what} {numerics:?}: the latent attention reads each cached row once");
            assert_eq!(launches("lk_attention_fold<<<"), rows_once, "{what} {numerics:?}: and folds its chunks");
            if common::require_tool("nvcc") {
                nvcc(&lane.program.cu, &dir().join(format!("lane-tiles-{what}-{numerics:?}")));
            }
        }
        let precise = wide(Q8MatKind::Dequant);
        let cu = &precise.program.cu;
        assert!(
            !cu.contains("lk_attention_tiled<") && !cu.contains("lk_attention_flash<") && !cu.contains("lk_attention_mma<") && !cu.contains("lk_attention_i8<"),
            "{what}: the precise numerics never attend in tiles"
        );
    }
    let mut heads256 = Tiny::Qwen3.wide(CTX);
    for layer in &mut heads256.layers {
        if let Mixer::Attention(a) = &mut layer.mixer {
            (a.head_dim, a.value_dim, a.scale) = (256, 256, 1.0 / 16.0);
            if let Some(r) = &mut a.rope {
                r.dims = 256;
            }
        }
    }
    let (spec, path) = sized_checkpoint(heads256, "qwen3-wide-heads256", &|_| GgmlType::Q4K);
    for (numerics, expected) in [(Q8MatKind::Int8Act, (0, 0, 3 * 2, 0)), (Q8MatKind::TwoPassInt8, (0, 0, 0, 3 * 2))] {
        let lane = compile(&spec, &path, &LaneOptions { ctx: 128, prefill: 64, ..options(numerics) }).unwrap_or_else(|e| panic!("heads 256 wide: {e}"));
        let launches = |kernel: &str| lane.program.cu.lines().filter(|l| l.trim_start().starts_with(kernel) && l.contains("<<<")).count();
        assert_eq!(
            (launches("lk_attention_tiled<"), launches("lk_attention_flash<"), launches("lk_attention_mma<"), launches("lk_attention_i8<")),
            expected,
            "heads 256 wide under {numerics:?}: the f16 tensor cores under one word, the int8 tensor cores under two"
        );
        if common::require_tool("nvcc") {
            nvcc(&lane.program.cu, &dir().join(format!("lane-tiles-heads256-{numerics:?}")));
        }
    }
    let mut heads320 = Tiny::Qwen3.wide(CTX);
    for layer in &mut heads320.layers {
        if let Mixer::Attention(a) = &mut layer.mixer {
            (a.head_dim, a.value_dim, a.scale) = (320, 320, 1.0 / 320f64.sqrt());
            if let Some(r) = &mut a.rope {
                r.dims = 320;
            }
        }
    }
    let (spec, path) = sized_checkpoint(heads320, "qwen3-wide-heads320", &|_| GgmlType::Q4K);
    let lane = compile(&spec, &path, &LaneOptions { ctx: 128, prefill: 64, ..options(Q8MatKind::Int8Act) }).unwrap_or_else(|e| panic!("heads 320 wide: {e}"));
    let launches = |kernel: &str| lane.program.cu.lines().filter(|l| l.trim_start().starts_with(kernel) && l.contains("<<<")).count();
    assert_eq!(
        (launches("lk_attention_tiled<"), launches("lk_attention_flash<"), launches("lk_attention_mma<")),
        (3 * 2, 0, 0),
        "heads 320 wide attend in the general tiles"
    );
    if common::require_tool("nvcc") {
        nvcc(&lane.program.cu, &dir().join("lane-tiles-heads320"));
    }
    use logicaffeine_compile::codegen_cuda::chip::CudaChip;
    use logicaffeine_compile::codegen_cuda::lane::kernels::attn_flash_smem_bytes;
    let smallest = [CudaChip::rtx3060(), CudaChip::a100(), CudaChip::h100(), CudaChip::b200()].iter().map(|c| c.smem_kib_per_block * 1024).min().unwrap();
    for hd in (4..=256).step_by(4) {
        for vd in (4..=256).step_by(4) {
            assert!(attn_flash_smem_bytes(hd, vd) <= smallest, "heads {hd} wide, values {vd}: the flash block's shared memory fits every chip");
        }
    }
}

/// A chunk's attention on the f16 tensor cores takes the stream-K schedule where `attn_mma_stream` takes it, under the
/// one-word numerics: every attention of Qwen3's and gpt-oss's 64-row chunks (windows and sinks) over the `f32` cache and
/// over the binary16 one, and of heads 256 wide over the binary16 cache, keeps the chip's resident blocks as its stream-K
/// blocks (`stream`, `attn_mma_stream_blocks`: the 3060's 28 multiprocessors by `attn_mma_blocks`) and launches
/// `lk_attention_mma<…, MB, 1>` over `attn_mma_schedule`'s grid, past its rows an `lk_amma_split` of its partials and
/// their counts — named where it splits: two partials a stream-K block (`attn_mma_part_floats`) placed in the arena, a
/// count an item in a zeroed buffer of its own — and the items a block each. Heads 256 wide over the `f32` cache keep one block an item: no
/// stream-K blocks, the grid of row blocks by groups, no scratch. The schedule: whole waves a block an item, the rest past
/// them split, every item split under one wave. Every program builds.
#[test]
fn a_chunks_tensor_core_attention_splits_its_last_wave_over_the_chip() {
    use logicaffeine_compile::codegen_cuda::chip::CudaChip;
    use logicaffeine_compile::codegen_cuda::lane::kernels::{
        attn_mma_blocks, attn_mma_items, attn_mma_part_floats, attn_mma_schedule, attn_mma_smem_bytes, attn_mma_stream_blocks, attn_tile_heads,
    };
    use logicaffeine_compile::codegen_cuda::CachePrecision;
    assert_eq!(attn_mma_schedule(256, 84), (252, 336), "Llama's 512-row chunk on the 3060: three whole waves, the fourth's four items split");
    assert_eq!(attn_mma_schedule(128, 84), (84, 168), "its 256-row chunk: one whole wave, 44 items split");
    assert_eq!(attn_mma_schedule(32, 56), (0, 56), "Gemma-3's 512-row chunk: under one wave, every item split");
    assert_eq!(attn_mma_schedule(168, 84), (168, 168), "whole waves: an item a block");
    let sms = CudaChip::rtx3060().sms;
    let mangled = |name: &str| -> String { name.chars().map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' }).collect() };
    let mut heads256 = Tiny::Qwen3.wide(CTX);
    for layer in &mut heads256.layers {
        if let Mixer::Attention(a) = &mut layer.mixer {
            (a.head_dim, a.value_dim, a.scale) = (256, 256, 1.0 / 16.0);
            if let Some(r) = &mut a.rope {
                r.dims = 256;
            }
        }
    }
    let wide256 = sized_checkpoint(heads256, "qwen3-wide-heads256", &|_| GgmlType::Q4K);
    let qwen3 = wide_checkpoint(Tiny::Qwen3, "stream", &|_| GgmlType::Q4K);
    let oss = wide_checkpoint(Tiny::GptOss, "stream", &|_| GgmlType::Q4K);
    for (what, (spec, path)) in [("qwen3", &qwen3), ("gpt-oss", &oss), ("heads 256 wide", &wide256)] {
        for cache in [CachePrecision::F32, CachePrecision::F16] {
            let what = format!("{what} {cache:?}");
            let knobs = CudaKnobs { cache_precision: cache, ..options(Q8MatKind::Int8Act).knobs };
            let lane = compile(spec, path, &LaneOptions { ctx: 128, prefill: 64, knobs, ..options(Q8MatKind::Int8Act) }).unwrap_or_else(|e| panic!("{what}: {e}"));
            let cu = &lane.program.cu;
            let mut seen = 0;
            for f in lane.forms.iter().filter(|f| f.rows > 16) {
                for (i, l) in f.schedule.launches.iter().enumerate() {
                    let Launch::Attention { out, heads, group, hd, vd, rows, stream, .. } = l else { continue };
                    if !l.attends_on_tensor_cores(Q8MatKind::Int8Act) {
                        continue;
                    }
                    seen += 1;
                    let (hg, smem) = (attn_tile_heads(*group), attn_mma_smem_bytes(*hd, *vd));
                    let split = cache == CachePrecision::F16 || (*hd <= 128 && *vd <= 128);
                    let line = cu
                        .lines()
                        .map(str::trim)
                        .find(|t| t.starts_with("lk_attention_mma<") && t.contains(&format!(", t{}_{}, ", f.tag(), mangled(out))))
                        .unwrap_or_else(|| panic!("{what} {}: the attention into `{out}` launches", f.tag()));
                    let tags: Vec<&str> = scratch(l, Q8MatKind::Int8Act).iter().map(|s| s.tag).collect();
                    if !split {
                        assert_eq!(*stream, 0, "{what} {}: `{out}` keeps one block an item", f.tag());
                        assert!(line.starts_with(&format!("lk_attention_mma<{hg}, {hd}, {vd}")) && line.contains(&format!("<<<dim3({}, {}), 128, {smem}, st>>>", rows.div_ceil(64 / hg), heads / hg)), "{what} {}: one block an item: {line}", f.tag());
                        assert!(tags.is_empty(), "{what} {}: no stream-K scratch: {tags:?}", f.tag());
                        continue;
                    }
                    assert_eq!(*stream, attn_mma_stream_blocks(*hd, *vd, sms), "{what} {}: `{out}` splits over the 3060's resident blocks", f.tag());
                    let items = attn_mma_items(*rows, *heads, hg);
                    let (ndp, grid) = attn_mma_schedule(items, *stream);
                    let elem = if cache == CachePrecision::F16 { "__half" } else { "float" };
                    assert!(
                        line.starts_with(&format!("lk_attention_mma<{hg}, {hd}, {vd}, {elem}, {elem}, {elem}, {}, 1><<<{grid}, 128, {smem}, st>>>", attn_mma_blocks(*hd, *vd))),
                        "{what} {}: the stream-K kernel over {grid} blocks: {line}",
                        f.tag()
                    );
                    let splits = grid > ndp;
                    let (ap, ac) = (format!("ap{}_{}", f.tag(), mangled(out)), format!("ac{}_{}", f.tag(), mangled(out)));
                    let named = if splits { format!(", {rows}, lk_amma_split{{{ap}, {ac}, {ndp}}}") } else { format!(", {rows}, lk_amma_split{{nullptr, nullptr, {ndp}}}") };
                    assert!(line.contains(&named), "{what} {}: past its rows `{named}`, its partials and their counts named exactly where the schedule splits: {line}", f.tag());
                    if splits {
                        let sizes: Vec<(&str, usize, bool)> = scratch(l, Q8MatKind::Int8Act).iter().map(|s| (s.tag, s.bytes, s.counter)).collect();
                        assert_eq!(sizes, vec![("ap", 2 * stream * attn_mma_part_floats(*vd) * 4, false), ("ac", items * 4, true)], "{what} {}: two partials a stream-K block, a count an item", f.tag());
                        assert!(f.arena.scratch_at(i, "ap").is_some(), "{what} {}: the partials live in the arena", f.tag());
                    } else {
                        assert!(tags.is_empty(), "{what} {}: whole waves keep no scratch", f.tag());
                    }
                }
            }
            assert!(seen > 0, "{what}: the chunks attend on the f16 tensor cores");
            if common::require_tool("nvcc") {
                nvcc(cu, &dir().join(format!("lane-stream-{}-{cache:?}", what.replace(' ', "-"))));
            }
        }
    }
}

/// Every expert mixture's grouped GEMM under one word takes two blocks a multiprocessor (`lk_gemm_blk_blocks`): Qwen3-MoE
/// at its wide shape in Q8_0 and every block format, and gpt-oss in MXFP4 (its clamped SwiGLU), compiled with chunks of
/// 64 rows — each `lk_gemm_blk<F, 1, G, 64>` of the paired gate and up (`G` 3), the experts apart (`G` 1) and the down
/// projection (`G` 2) uses at most 128 registers a thread with no stack frame and no spill, as `ptxas` reports it, and
/// two of its blocks' shared memory (`gemm_blk_launch_smem`), with the kilobyte the driver reserves for each, fit the
/// multiprocessor's 100 KB. Q2_K, Q3_K and Q5_K, whose halves' scales and offsets beside the multiply need more
/// registers than two blocks leave a thread, take one block, still with no stack frame and no spill. On the 3060 one
/// 256-thread block of 133 registers a multiprocessor ran gpt-oss-20b's 512-row prefill in 232.9 ms, two in 179.7.
#[test]
fn every_one_word_expert_gemm_takes_two_blocks_a_multiprocessor() {
    use logicaffeine_compile::codegen_cuda::lane::kernels::{format_arg, gemm_blk_launch_smem};
    if !common::require_tool("nvcc") {
        return;
    }
    let formats: Vec<GgmlType> = std::iter::once(GgmlType::Q8_0).chain(BLOCK_FORMATS).collect();
    let jobs: Vec<(Tiny, GgmlType)> =
        formats.iter().map(|&ty| (Tiny::Qwen3Moe, ty)).chain(std::iter::once((Tiny::GptOss, GgmlType::Mxfp4))).collect();
    let reports: Vec<(String, ModelSpec, GgmlType, String)> = std::thread::scope(|s| {
        let handles: Vec<_> = jobs
            .iter()
            .map(|&(t, ty)| {
                s.spawn(move || {
                    let what = format!("{} {}", t.name(), ty.name());
                    let (spec, path) = wide_checkpoint(t, &format!("two-blocks-{}", ty.name()), &|_| ty);
                    let lane = compile(&spec, &path, &LaneOptions { ctx: 128, prefill: 64, ..options(Q8MatKind::Int8Act) })
                        .unwrap_or_else(|e| panic!("{what}: {e}"));
                    let src = dir().join(format!("two-blocks-{}-{}.cu", t.name(), ty.name()));
                    std::fs::write(&src, &lane.program.cu).unwrap();
                    let out = Command::new("nvcc")
                        .args(["-O3", "-std=c++17", "-arch=sm_86", "-cubin", "-Xptxas", "-v", "-o", "/dev/null"])
                        .arg(&src)
                        .output()
                        .expect("nvcc runs");
                    assert!(out.status.success(), "{what} compiles:\n{}", String::from_utf8_lossy(&out.stderr));
                    (what, spec, ty, String::from_utf8_lossy(&out.stderr).into_owned())
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let (mut checked, mut short) = (0, Vec::new());
    for (what, spec, ty, report) in &reports {
        let f = format_arg(*ty);
        let two = !matches!(ty, GgmlType::Q2K | GgmlType::Q3K | GgmlType::Q5K);
        let grouped: Vec<String> = (1..=3).map(|g| format!("_Z11lk_gemm_blkILi{f}ELi1ELi{g}ELi64E")).collect();
        let mut kernel: Option<String> = None;
        let mut frame: Option<String> = None;
        for line in report.lines() {
            if let Some(name) = line.split("Function properties for ").nth(1) {
                kernel = Some(name.trim().to_string());
                frame = None;
            } else if line.contains("bytes stack frame") {
                frame = Some(line.trim().to_string());
            } else if let (Some(k), Some(regs)) = (&kernel, line.split("Used ").nth(1).and_then(|r| r.split(' ').next())) {
                if grouped.iter().any(|g| k.starts_with(g.as_str())) {
                    checked += 1;
                    let regs: usize = regs.parse().unwrap();
                    let spilled = frame.as_deref().map_or(true, |fr| {
                        fr.split(',').map(|p| p.trim().split(' ').next().unwrap().parse::<usize>().unwrap()).any(|b| b != 0)
                    });
                    if (two && regs > 128) || spilled {
                        short.push(format!("{what}: `{k}` uses {regs} registers ({})", frame.as_deref().unwrap_or("no frame report")));
                    }
                }
                kernel = None;
            }
        }
        for k in [spec.d, 256] {
            let smem = gemm_blk_launch_smem(f, false, 64, k);
            if two && 2 * (smem + 1024) > 100 * 1024 {
                short.push(format!("{what}: two blocks of {smem} bytes of shared memory over k {k} pass the multiprocessor's 100 KB"));
            }
        }
    }
    assert!(checked >= jobs.len() * 2, "every program's grouped GEMMs are reported: {checked}");
    assert!(short.is_empty(), "grouped expert GEMMs short of their blocks a multiprocessor:\n{}", short.join("\n"));
}

/// A state a chunk's form holds alone lives in the step's arena (`arena::place`'s `own`): gpt-oss at its wide shape in
/// MXFP4 — its experts' output declared zeroed and set a row at a time (`bmo{l}`), held by no step but the chunks' —
/// compiled with chunks of 16 rows: every chunk form places each layer's `bmo` in its arena, so the lane's fixed arrays
/// hold none of it, and the decode step's zeroed states (its caches) stay out of every arena, read by every form. At
/// gpt-oss-20b's 8,448 positions the outputs took 269 MiB of fixed arrays, a layer a form, and its 8K decode ran out of
/// memory on the 3060.
#[test]
fn a_chunks_own_state_lives_in_the_arena() {
    let (spec, path) = wide_checkpoint(Tiny::GptOss, "own-state", &|_| GgmlType::Mxfp4);
    let lane = compile(&spec, &path, &LaneOptions { prefill: 16, ..options(Q8MatKind::Int8Act) }).unwrap_or_else(|e| panic!("gpt-oss: {e}"));
    let mut placed = 0;
    for f in lane.forms.iter().filter(|f| f.rows > 1) {
        for (name, b) in f.schedule.buffers.iter().filter(|(n, _)| n.starts_with("bmo")) {
            assert_eq!(b.fill, Fill::Zero, "{}: `{name}` is declared zeroed", f.tag());
            assert!(f.arena.at(name, Touch::Plain).is_some(), "{}: `{name}` lives in the arena", f.tag());
            placed += 1;
        }
    }
    assert!(placed >= 2 * spec.layers.len(), "every chunk form's experts' outputs are placed: {placed}");
    let decode = lane.forms.iter().find(|f| f.rows == 1).expect("a decode step");
    let states: Vec<&String> = decode.schedule.buffers.iter().filter(|(_, b)| b.fill == Fill::Zero).map(|(n, _)| n).collect();
    assert!(!states.is_empty(), "the decode step holds zeroed states");
    for name in states {
        assert!(decode.arena.at(name, Touch::Plain).is_none(), "the decode step's state `{name}` is every form's, in no arena");
        for f in &lane.forms {
            assert!(f.arena.at(name, Touch::Plain).is_none(), "{}: the decode step's state `{name}` in no arena", f.tag());
        }
    }
}

/// Every copy a kernel the lane ships waits for is committed: in the PTX of each program a wide chunk's lane writes —
/// Qwen3 at its wide shape in Q4_K, Q6_K and Q2_K (the block GEMM's stages over two decoded tiles and over one), in
/// Q8_0 (the streamed path) and in BF16 (the 16-bit GEMM), Qwen3-MoE in Q4_K (the grouped GEMMs), and Qwen3.5 in Q8_0
/// (its chunked delta rule and attention), under both quantized numerics — each kernel that waits on groups of
/// `cp.async` copies (`cp.async.wait_group`) commits copies to groups (`cp.async.commit_group`): a copy in no group is
/// one no `cp.async.wait_group` waits for, and a stage that reads it reads whatever has landed. And each kernel that
/// copies by `cp.async` has its copies committed, waited on all at once (`cp.async.wait_all`) or tracked by an mbarrier
/// (`cp.async.mbarrier.arrive`).
#[test]
fn every_copy_a_kernel_waits_for_is_committed() {
    if !common::require_tool("nvcc") {
        return;
    }
    let dir = dir().join("commits");
    std::fs::create_dir_all(&dir).unwrap();
    let mut jobs: Vec<(String, String)> = Vec::new();
    for (t, ty) in [
        (Tiny::Qwen3, GgmlType::Q4K),
        (Tiny::Qwen3, GgmlType::Q6K),
        (Tiny::Qwen3, GgmlType::Q2K),
        (Tiny::Qwen3, GgmlType::Q8_0),
        (Tiny::Qwen3, GgmlType::Bf16),
        (Tiny::Qwen3Moe, GgmlType::Q4K),
        (Tiny::Qwen35, GgmlType::Q8_0),
    ] {
        // A gated delta layer's convolution weights stay `f32`: their rows are a few taps, no whole blocks.
        let (spec, path) = wide_checkpoint(t, &format!("commits-{}", ty.name()), &|n| if n.contains("conv") { GgmlType::F32 } else { ty });
        for numerics in [Q8MatKind::Int8Act, Q8MatKind::TwoPassInt8] {
            let what = format!("{} {} {numerics:?}", t.name(), ty.name());
            let lane = compile(&spec, &path, &LaneOptions { ctx: 128, prefill: 64, ..options(numerics) }).unwrap_or_else(|e| panic!("{what}: {e}"));
            jobs.push((what, lane.program.cu.clone()));
        }
    }
    let reports: Vec<(String, String)> = std::thread::scope(|s| {
        let handles: Vec<_> = jobs
            .iter()
            .enumerate()
            .map(|(i, (what, cu))| {
                let dir = &dir;
                s.spawn(move || {
                    let (src, ptx) = (dir.join(format!("p{i}.cu")), dir.join(format!("p{i}.ptx")));
                    std::fs::write(&src, cu).unwrap();
                    let out = Command::new("nvcc").args(["-O3", "-std=c++17", "-arch=sm_86", "-ptx", "-o"]).arg(&ptx).arg(&src).output().expect("nvcc runs");
                    assert!(out.status.success(), "{what} compiles:\n{}", String::from_utf8_lossy(&out.stderr));
                    (what.clone(), std::fs::read_to_string(&ptx).unwrap())
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let mut copying = 0;
    for (what, ptx) in &reports {
        // Each kernel's body: from the brace after its parameters to the brace that closes it, its inline assembly's
        // braced blocks inside it.
        for (at, _) in ptx.match_indices(".entry ") {
            let name: String = ptx[at + 7..].chars().take_while(|c| *c != '(').collect();
            let params = at + ptx[at..].find(')').unwrap();
            let open = params + ptx[params..].find('{').unwrap();
            let mut depth = 0;
            let close = ptx[open..]
                .char_indices()
                .find(|&(_, c)| {
                    depth += match c {
                        '{' => 1,
                        '}' => -1,
                        _ => 0,
                    };
                    depth == 0
                })
                .map(|(i, _)| open + i)
                .unwrap();
            let body = &ptx[open..close];
            let copies = body.matches("cp.async.cg.").count() + body.matches("cp.async.ca.").count();
            let (commits, waits) = (body.matches("cp.async.commit_group").count(), body.matches("cp.async.wait_group").count());
            assert!(waits == 0 || commits > 0, "{what}: `{name}` waits on groups of cp.async copies {waits} times and commits none");
            if copies == 0 {
                continue;
            }
            copying += 1;
            assert!(
                commits > 0 || body.contains("cp.async.wait_all") || body.contains("cp.async.mbarrier.arrive"),
                "{what}: `{name}` copies {copies} times by cp.async and neither commits, waits on all nor tracks them"
            );
        }
    }
    assert!(copying > 0, "the programs' kernels copy by cp.async");
}

/// A GEMM in parts reads its parts from its launch's parameters (`lk_take_part`): its part is taken by selects over
/// constant indices, never a part index computed at run time, which would copy the parts to every thread's stack — so
/// every GEMM in parts a wide chunk launches, in Q8_0 (the streamed path), Q4_K, Q6_K and BF16 (the 16-bit tensor
/// cores), under both quantized numerics, keeps a stack frame smaller than one copy of
/// `lk_parts`, as `ptxas` reports it.
#[test]
fn a_gemm_in_parts_reads_its_parts_where_its_launch_passed_them() {
    use logicaffeine_compile::codegen_cuda::lane::kernels::{Kernel, PRELUDE};
    if !common::require_tool("nvcc") {
        return;
    }
    let dir = dir().join("parts-frame");
    std::fs::create_dir_all(&dir).unwrap();
    let size_src = dir.join("size.cu");
    std::fs::write(&size_src, format!("#include <cstdio>\n#include <cuda_runtime.h>\n#include <cuda_fp16.h>\n{PRELUDE}{}\nint main(){{ printf(\"%zu\\n\", sizeof(lk_parts)); return 0; }}\n", Kernel::Blocks.source())).unwrap();
    let size_bin = dir.join("size");
    let built = Command::new("nvcc").args(["-std=c++17", "-arch=sm_86", "-o"]).arg(&size_bin).arg(&size_src).output().expect("nvcc runs");
    assert!(built.status.success(), "the size probe builds:\n{}", String::from_utf8_lossy(&built.stderr));
    let parts: usize = String::from_utf8_lossy(&Command::new(&size_bin).output().unwrap().stdout).trim().parse().unwrap();
    let jobs: Vec<(GgmlType, Q8MatKind)> = [GgmlType::Q8_0, GgmlType::Q4K, GgmlType::Q6K, GgmlType::Bf16]
        .into_iter()
        .flat_map(|ty| [Q8MatKind::Int8Act, Q8MatKind::TwoPassInt8].map(|n| (ty, n)))
        .collect();
    let reports: Vec<(String, String)> = std::thread::scope(|s| {
        let handles: Vec<_> = jobs
            .iter()
            .map(|&(ty, numerics)| {
                let dir = &dir;
                s.spawn(move || {
                    let what = format!("qwen3 {} {numerics:?}", ty.name());
                    let (spec, path) = wide_checkpoint(Tiny::Qwen3, &format!("parts-frame-{}", ty.name()), &|_| ty);
                    let lane = compile(&spec, &path, &LaneOptions { ctx: 128, prefill: 64, ..options(numerics) }).unwrap_or_else(|e| panic!("{what}: {e}"));
                    assert!(lane.program.cu.contains(if ty == GgmlType::Bf16 { "lk_gemm_h16<" } else { "lk_gemm_blk<" }), "{what}: its products on the tensor cores");
                    let src = dir.join(format!("{}-{numerics:?}.cu", ty.name()));
                    std::fs::write(&src, &lane.program.cu).unwrap();
                    let out = Command::new("nvcc").args(["-O3", "-std=c++17", "-arch=sm_86", "-cubin", "-Xptxas", "-v", "-o", "/dev/null"]).arg(&src).output().expect("nvcc runs");
                    assert!(out.status.success(), "{what} compiles:\n{}", String::from_utf8_lossy(&out.stderr));
                    (what, String::from_utf8_lossy(&out.stderr).into_owned())
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    for (what, report) in &reports {
        let mut kernel: Option<String> = None;
        let mut seen = 0;
        for line in report.lines() {
            if let Some(name) = line.split("Function properties for ").nth(1) {
                kernel = Some(name.trim().to_string());
            } else if let (Some(k), Some(frame)) = (&kernel, line.trim().split(" bytes stack frame").next().filter(|_| line.contains("bytes stack frame"))) {
                // The GEMMs in parts: `G` 4, the third template argument.
                let in_parts = ["_Z11lk_gemm_blkI", "_Z11lk_gemm_h16I"].iter().any(|p| k.starts_with(p))
                    && k.split('E').nth(2).is_some_and(|g| g == "Li4");
                if in_parts {
                    seen += 1;
                    let bytes: usize = frame.trim().parse().unwrap();
                    assert!(bytes < parts, "{what}: `{k}` keeps a {bytes}-byte stack frame, one copy of its {parts}-byte parts or more");
                }
                kernel = None;
            }
        }
        assert!(seen > 0, "{what}: a GEMM in parts reported");
    }
}

/// Every block GEMM launches with the shared memory its kernel lays out (`gemm_blk_smem_bytes`, the block path): for
/// Q8_0 and every block format, under one word and two, over tiles of 64 and 128 rows, the kernel's own stage sizes
/// (`lk_stage_acts`, `lk_stage_wt`, `lk_stage_sums`) evaluated by `nvcc` on the host and laid out as the block path
/// lays them out — its activation buffers (two where `lk_stage_pipelined`), its decoded weight tiles (two where
/// `lk_stage_over`), then the activation's sums, at least the paired epilogue's sums — end at the bytes the launch
/// takes; a block takes two decoded tiles only where it holds its multiprocessor alone (`lk_gemm_blk_blocks`), and the
/// 128-row tiles of Qwen3-8B's formats, Q4_K and Q6_K under one word, take them.
#[test]
fn every_block_gemms_shared_memory_is_what_its_kernel_lays_out() {
    use logicaffeine_compile::codegen_cuda::lane::kernels::{gemm_blk_launch_smem, Kernel, PRELUDE};
    if !common::require_tool("nvcc") {
        return;
    }
    let formats: Vec<GgmlType> = std::iter::once(GgmlType::Q8_0).chain(BLOCK_FORMATS).collect();
    let mut rows = String::new();
    for ty in &formats {
        for q in [1, 2] {
            for bm in [64, 128] {
                let f = ty.id();
                rows.push_str(&format!(
                    "  row<{f}, {q}, {bm}>(\"{} {q} {bm}\");\n",
                    ty.name()
                ));
            }
        }
    }
    let main = format!(
        "#include <cstdio>\n#include <cuda_runtime.h>\n#include <cuda_fp16.h>\n{PRELUDE}{}{}{}{}\n\
         template<int F, int Q, int BM> void row(const char* what){{\n\
           constexpr bool PIPE = lk_stage_pipelined<F, Q, BM>(), OVER = lk_stage_over<F, Q, BM>();\n\
           constexpr int end = (PIPE ? 2 : 1) * lk_stage_acts<Q, BM>() + (OVER ? 2 : 1) * lk_stage_wt<F>() + lk_stage_sums<F, Q, BM>();\n\
           constexpr int pair = BM * LK_BN * 4;\n\
           printf(\"%s %d %d %d\\n\", what, end > pair ? end : pair, (int)OVER, lk_gemm_blk_blocks<F, Q, BM>());\n\
         }}\n\
         int main(){{\n{rows}  return 0;\n}}\n",
        Kernel::Blocks.source(),
        Kernel::Quant.source(),
        Kernel::Mma.source(),
        Kernel::GemmBlk.source()
    );
    let dir = dir().join("blk-smem");
    std::fs::create_dir_all(&dir).unwrap();
    let (cu, bin) = (dir.join("layout.cu"), dir.join("layout"));
    std::fs::write(&cu, main).unwrap();
    let built = Command::new("nvcc").args(["-std=c++17", "-arch=sm_86", "-o"]).arg(&bin).arg(&cu).output().expect("nvcc runs");
    assert!(built.status.success(), "the layout probe builds:\n{}", String::from_utf8_lossy(&built.stderr));
    let ran = Command::new(&bin).output().expect("the layout probe runs");
    assert!(ran.status.success(), "the layout probe runs:\n{}", String::from_utf8_lossy(&ran.stderr));
    let report = String::from_utf8_lossy(&ran.stdout).into_owned();
    let lines: Vec<&str> = report.lines().collect();
    assert_eq!(lines.len(), formats.len() * 4, "every format, numerics and tile reported:\n{report}");
    let mut over = Vec::new();
    for (line, (ty, q, bm)) in lines.iter().zip(formats.iter().flat_map(|&ty| [1, 2].into_iter().flat_map(move |q| [64, 128].into_iter().map(move |bm| (ty, q, bm))))) {
        let fields: Vec<&str> = line.split(' ').collect();
        let (bytes, two_tiles, blocks): (usize, usize, usize) = (fields[3].parse().unwrap(), fields[4].parse().unwrap(), fields[5].parse().unwrap());
        let what = format!("{} under {q} words, {bm} rows", ty.name());
        // A Q8_0 matrix's launch over a `k` of whole 128-value scale groups streams; every other takes the block path.
        let launched = gemm_blk_launch_smem(ty.id(), q == 2, bm, if ty == GgmlType::Q8_0 { 96 } else { 128 });
        assert_eq!(launched, bytes, "{what}: the launch takes the bytes the kernel lays out");
        assert!(two_tiles == 0 || blocks == 1, "{what}: two decoded tiles only where a block holds its multiprocessor alone");
        if two_tiles == 1 {
            over.push((ty, q, bm));
        }
    }
    for ty in [GgmlType::Q4K, GgmlType::Q6K] {
        assert!(over.contains(&(ty, 1, 128)), "{}'s dense 128-row tile under one word decodes its next stage beside its multiply", ty.name());
    }
}

/// A wide chunk's per-head products run in tiles under the quantized numerics (`lk_head_tiled`): DeepSeek's latent
/// expansion, each cached latent row through the key half and the value half, with one expansion and with the halves
/// split, every product of the 64- and 32-row chunks in one launch; the 16-row chunk and the decode step keep the
/// GEMV's slabs for the absorption and the attended latent's expansion, and so do the precise numerics in every form.
/// Every program builds.
#[test]
fn a_wide_chunks_per_head_products_run_in_tiles() {
    for t in [Tiny::DeepSeek2, Tiny::DeepSeek2Split] {
        let (spec, path) = wide_checkpoint(t, "heads", &|_| GgmlType::Q4K);
        let what = t.name();
        let wide = |numerics| compile(&spec, &path, &LaneOptions { ctx: 128, prefill: 64, ..options(numerics) }).unwrap_or_else(|e| panic!("{what}: {e}"));
        let products = |f: &logicaffeine_compile::codegen_cuda::lane::Form| f.schedule.launches.iter().filter(|l| matches!(l, Launch::HeadGemv { .. })).count();
        for numerics in [Q8MatKind::Int8Act, Q8MatKind::TwoPassInt8] {
            let lane = wide(numerics);
            let launches = |kernel: &str| lane.program.cu.lines().filter(|l| l.trim_start().starts_with(kernel) && l.contains("<<<")).count();
            let tiled: usize = lane.forms.iter().filter(|f| f.rows > 16).map(products).sum();
            let slabs: usize = lane.forms.iter().filter(|f| f.rows <= 16).map(|f| products(f) * f.rows.div_ceil(16)).sum();
            assert_eq!(tiled, 3 * 2 * 2, "{what} {numerics:?}: each layer's two products in the 64- and 32-row chunks");
            assert_eq!(launches("lk_head_tiled<"), tiled, "{what} {numerics:?}: the wide chunks' products in tiles");
            assert_eq!(
                launches("lk_head_gemv<") + launches("lk_head_axpy<") + launches("lk_head_axpy_staged<"),
                slabs,
                "{what} {numerics:?}: the narrow forms' products in slabs of sixteen rows"
            );
            if common::require_tool("nvcc") {
                nvcc(&lane.program.cu, &dir().join(format!("lane-heads-{what}-{numerics:?}")));
            }
        }
        let precise = wide(Q8MatKind::Dequant);
        assert!(!precise.program.cu.contains("lk_head_tiled<"), "{what}: the precise numerics keep the slabs");
    }
}

/// A wide chunk's latent attention attends expanded under the quantized numerics (`latent_expanded`): each cached
/// latent row runs through both halves of the latent's expansion — two per-head products over the cache's rows before
/// the chunk's last position, every head reading the row's one latent, the key half's rows its outputs where the
/// absorption's are its inputs — and each head's query attends its own keys and values, writing what the output
/// projection reads. The query is its unrotated part, read in place in the query projection, then its rotated part;
/// the key is the head's expansion, then the rotated key every head shares, read in place in the cache. No launch
/// slices the query, absorbs it, joins it, or expands an attended latent. The 16-row chunk, the decode step and the
/// precise numerics keep the absorbed attention, with one expansion and with the halves split. Every program builds.
#[test]
fn a_wide_chunks_latent_attention_attends_expanded() {
    for t in [Tiny::DeepSeek2, Tiny::DeepSeek2Split] {
        let (spec, path) = wide_checkpoint(t, "expanded", &|_| GgmlType::Q4K);
        let what = t.name();
        let Mixer::Mla(m) = &spec.layers[0].mixer else { panic!("{what} has latent attention") };
        let (h, nope, rope, lora, vd) = (m.heads, m.qk_nope, m.qk_rope, m.kv_lora_rank, m.v_dim);
        let (qk, kw, ctx) = (nope + rope, lora + rope, 128);
        // The absorption's key half reads rows that are its inputs in one expansion, its outputs with the halves split.
        let absorbed_rows_out = t == Tiny::DeepSeek2Split;
        let wide = |numerics| compile(&spec, &path, &LaneOptions { ctx, prefill: 64, ..options(numerics) }).unwrap_or_else(|e| panic!("{what}: {e}"));
        for numerics in [Q8MatKind::Int8Act, Q8MatKind::TwoPassInt8] {
            let lane = wide(numerics);
            for f in &lane.forms {
                let s = &f.schedule;
                let at = format!("{what} {numerics:?} {} rows", f.rows);
                let attentions: Vec<&Launch> = s.launches.iter().filter(|l| matches!(l, Launch::Attention { .. })).collect();
                let products: Vec<&Launch> = s.launches.iter().filter(|l| matches!(l, Launch::HeadGemv { .. })).collect();
                assert_eq!((attentions.len(), products.len()), (3, 6), "{at}: each layer's attention and two per-head products");
                if f.rows <= 16 {
                    assert_eq!(s.fired("latent_expanded"), 0, "{at}");
                    for a in &attentions {
                        assert!(matches!(a, Launch::Attention { kc, vc, group, split: None, .. } if kc == vc && *group == h), "{at}: absorbed: {a:?}");
                    }
                    continue;
                }
                assert_eq!(s.fired("latent_expanded"), 3, "{at}");
                assert_eq!(s.census().get("join"), None, "{at}: nothing joins a query");
                for l in 0..3 {
                    let (kc, qr, qpp, ct, wkb, wvb) =
                        (format!("Kc{l}"), format!("bqr{l}"), format!("bqpp{l}"), format!("bct{l}"), format!("Wkb{l}"), format!("Wvb{l}"));
                    let half = |w: &str| {
                        products
                            .iter()
                            .find(|p| matches!(p, Launch::HeadGemv { w: pw, .. } if pw == w))
                            .unwrap_or_else(|| panic!("{at}: the product by `{w}`"))
                    };
                    let expansion = |p: &Launch, b: usize, rows_out: bool| match p {
                        Launch::HeadGemv { x, part, y, m, heads, a, b: pb, rows_out: ro, shared, upto, .. } => {
                            assert_eq!(x, &kc, "{at}: the expansion reads the cache");
                            assert_eq!(*part, Some(Part { pitch: kw, offset: 0 }), "{at}: each cache row's latent");
                            assert!(*shared, "{at}: every head reads the row's one latent");
                            assert_eq!((*m, *heads, *a, *pb, *ro), (ctx, h, lora, b, rows_out), "{at}: {p:?}");
                            assert_eq!(*upto, Some(("pos_s0".to_string(), f.rows - 1)), "{at}: the rows before the chunk's last position");
                            assert!(y.row.is_none() && y.unit.is_none(), "{at}: into a tensor of its own");
                            y.buf.clone()
                        }
                        _ => unreachable!(),
                    };
                    let keys = expansion(half(&wkb), nope, !absorbed_rows_out);
                    let values = expansion(half(&wvb), vd, true);
                    let attend = attentions
                        .iter()
                        .find(|a| matches!(a, Launch::Attention { out, .. } if *out == ct))
                        .unwrap_or_else(|| panic!("{at}: an attention writes `{ct}`"));
                    let Launch::Attention { q, kc: k, vc: v, pos, heads, group, hd, vd: avd, vstride, voff, scale, split, .. } = attend else { unreachable!() };
                    assert_eq!((q, k, v), (&qr, &keys, &values), "{at}: the query projection over the expansion's keys and values");
                    assert_eq!((*heads, *group, *hd, *avd, *vstride, *voff), (h, 1, qk, vd, h * vd, 0), "{at}: each head its own keys and values");
                    assert_eq!(pos.as_deref(), Some("pos_s0"), "{at}");
                    assert_eq!(*scale, m.scale, "{at}: the model's score scale");
                    assert_eq!(
                        split,
                        &Some(Split {
                            own: nope,
                            q_pitch: qk,
                            q_tail: (qpp.clone(), Part { pitch: rope, offset: 0 }),
                            k_tail: (kc.clone(), Part { pitch: kw, offset: lora }),
                        }),
                        "{at}: the query's and key's rotated parts"
                    );
                    assert!(
                        s.launches.iter().any(|x| matches!(x, Launch::Rope { x, part, y, .. }
                            if *x == qr && *part == Some(Part { pitch: qk, offset: nope }) && *y == Dst::to(&qpp))),
                        "{at}: each head's rotated query part turned from its part of `{qr}`"
                    );
                    assert!(!s.launches.iter().any(|x| x.reads().contains(&format!("bol{l}").as_str())), "{at}: no attended latent is expanded");
                    assert_eq!(s.buffers[&qpp].len, f.rows * h * rope, "{at}: each row's heads' rotated parts, and no more");
                }
            }
            if common::require_tool("nvcc") {
                nvcc(&lane.program.cu, &dir().join(format!("lane-expanded-{what}-{numerics:?}")));
            }
        }
        let precise = wide(Q8MatKind::Dequant);
        for f in &precise.forms {
            assert_eq!(f.schedule.fired("latent_expanded"), 0, "{what}: the precise numerics keep the absorbed attention in a chunk of {} rows", f.rows);
        }
    }
}

/// The array a touch of `name` lands in: through every view to the tensor it views — its quantized form too, when the
/// view starts on a whole block and so reads its tensor's quantized form in place.
fn viewed(s: &Schedule, name: &str, touch: Touch) -> (String, Touch) {
    let mut at = name.to_string();
    while let Some(Fill::Alias { of, offset }) = s.buffers.get(&at).map(|b| &b.fill) {
        if touch == Touch::Quant && !quant_in_place(*offset) {
            break;
        }
        at = of.clone();
    }
    (at, touch)
}

/// Every launch's touches of each array, in launch order: `(launch, reads, writes)`.
fn lifetimes(s: &Schedule) -> BTreeMap<(String, Touch), Vec<(usize, bool, bool)>> {
    let mut out: BTreeMap<(String, Touch), Vec<(usize, bool, bool)>> = BTreeMap::new();
    for (i, l) in s.launches.iter().enumerate() {
        let (reads, writes) = l.touches(s.numerics);
        let mut here: BTreeMap<(String, Touch), (bool, bool)> = BTreeMap::new();
        for (t, touch) in reads {
            here.entry(viewed(s, t, touch)).or_default().0 = true;
        }
        for (t, touch) in writes {
            here.entry(viewed(s, t, touch)).or_default().1 = true;
        }
        for (k, (r, w)) in here {
            out.entry(k).or_default().push((i, r, w));
        }
    }
    out
}

/// What launch `l` writes of tensor `t` in its own name, as its destination says: all of it, the part `[from, from +
/// width)` of every unit of `pitch` (`join_in_place`), or something less — rows of a table.
#[derive(Debug)]
enum Wrote {
    Whole,
    Part(usize, usize, usize),
    Less,
}

fn wrote(l: &Launch, t: &str) -> Wrote {
    let (dst, width) = match l {
        Launch::Gemv { y, n, .. } | Launch::GemvF32 { y, n, .. } => (Some(y), *n),
        Launch::Rope { y, hd, .. } => (Some(y), *hd),
        Launch::HeadGemv { y, b, .. } => (Some(y), *b),
        Launch::Moe { y, d, .. } => (Some(y), *d),
        Launch::Join { y, heads, ha, hb, .. } => (Some(y), heads * (ha + hb)),
        Launch::Norm { into: Some(d), cols, .. } if d.buf == t => (Some(d), *cols),
        Launch::SetRow { .. } => return Wrote::Less,
        _ => (None, 0),
    };
    match dst {
        Some(d) if d.buf == t => match (&d.row, &d.unit) {
            (None, None) => Wrote::Whole,
            (None, Some(p)) => Wrote::Part(p.pitch, p.offset, width),
            _ => Wrote::Less,
        },
        _ => Wrote::Whole,
    }
}

/// Every tensor a step computes and reads within the step lives in the program's one arena by its lifetime
/// (`Form::arena`): each array placed — a tensor's `f32` buffer or its quantized form — is first touched by a launch
/// that writes it and reads nothing of it, is all written before any launch reads it (whole, or as parts of every unit
/// that together cover each unit — never rows, never through a view), stays placed until the last launch that touches
/// it (through any view of it), never shares a byte with an array live beside it, and is a computed tensor's or a
/// zeroed state the form holds alone (`arena::place`'s `own`: one the decode step, which every form reads, does not
/// carry), never a constant's, carried state's, an input's or an output's. Every launch's scratch but its arrival
/// counts lives there too while the launch runs, clear of every array live then and of the launch's other scratch; the
/// counts, zero before the launch and after it, keep zeroed buffers of their own. The program allocates the arena once,
/// at the largest form's size, filled with `0xff` so any read of a byte no launch of the step wrote is a NaN. A 64-row
/// chunk of DeepSeek-V2 at its medium width places nearly all of its step's bytes, and they fit in a third of their
/// sum.
#[test]
fn every_step_local_array_lives_in_one_arena_by_its_lifetime() {
    let mut lanes: Vec<(String, Lane)> = Vec::new();
    for &t in ALL {
        for numerics in NUMERICS {
            lanes.push((format!("{} {numerics:?}", t.name()), compiled_chunks(t, numerics, 8, "arena")));
        }
    }
    let (spec, path) = medium_checkpoint(Tiny::DeepSeek2, "arena", &|_| GgmlType::Q4K);
    for numerics in NUMERICS {
        let opts = LaneOptions { ctx: 128, prefill: 64, ..options(numerics) };
        lanes.push((format!("medium deepseek2 {numerics:?}"), compile(&spec, &path, &opts).unwrap()));
    }
    for (what, lane) in &lanes {
        let two = lane.forms[0].schedule.numerics == Q8MatKind::TwoPassInt8;
        let mut pointers = 0;
        for f in &lane.forms {
            let (s, a) = (&f.schedule, &f.arena);
            let what = format!("{what}, {} rows", f.rows);
            let io = [&s.io.token, &s.io.position, &s.io.logits, &s.io.next];
            let life = lifetimes(s);
            for p in &a.placed {
                let b = &s.buffers[&p.name];
                let own = b.fill == Fill::Zero && !lane.forms[0].schedule.buffers.contains_key(&p.name);
                assert!(
                    (b.fill == Fill::Computed || own) && !io.contains(&&p.name),
                    "{what}: `{}` is a computed tensor of the step's own, or a zeroed state the form holds alone",
                    p.name
                );
                assert!(if p.touch == Touch::Plain { b.plain } else { b.quantized }, "{what}: `{}` {:?} is allocated", p.name, p.touch);
                let touches = &life[&(p.name.clone(), p.touch)];
                let (i0, read, writes) = touches[0];
                assert!(writes && !read && i0 == p.first, "{what}: `{}` {:?} is first written, at launch {}: {touches:?}", p.name, p.touch, p.first);
                let (mut whole, mut parts): (bool, Vec<(usize, usize, usize)>) = (false, Vec::new());
                for &(i, read, write) in touches {
                    if read {
                        break;
                    }
                    let l = &s.launches[i];
                    let direct = l.touches(s.numerics).1.iter().any(|(t, touch)| *t == p.name && *touch == p.touch);
                    match (direct, write, p.touch, wrote(l, &p.name)) {
                        (true, true, Touch::Quant, _) | (true, true, Touch::Plain, Wrote::Whole) => whole = true,
                        (true, true, Touch::Plain, Wrote::Part(pitch, from, width)) => parts.push((pitch, from, width)),
                        _ => panic!("{what}: `{}` {:?} is written less than whole before it is read, at launch {i}: {l:?}", p.name, p.touch),
                    }
                }
                if !whole {
                    let pitch = parts[0].0;
                    let mut covered = vec![false; pitch];
                    for &(q, from, width) in &parts {
                        assert_eq!(q, pitch, "{what}: `{}`'s parts are of one unit", p.name);
                        covered[from..from + width].iter_mut().for_each(|c| *c = true);
                    }
                    assert!(covered.iter().all(|c| *c), "{what}: `{}`'s parts {parts:?} cover every unit before it is read", p.name);
                }
                assert_eq!(p.last, touches.last().unwrap().0, "{what}: `{}` {:?} lives to its last touch", p.name, p.touch);
                assert_eq!(p.offset % ARENA_ALIGN, 0, "{what}: `{}` aligned", p.name);
                assert_eq!(p.bytes, array_bytes(b.len, p.touch, two), "{what}: `{}` {:?} sized", p.name, p.touch);
                assert!(p.offset + p.bytes <= a.bytes, "{what}: `{}` within the arena", p.name);
                pointers += if p.touch == Touch::Plain { 1 } else if two { 3 } else { 2 };
            }
            for (at, l) in s.launches.iter().enumerate() {
                let wanted = scratch(l, s.numerics);
                let here: Vec<_> = a.scratch.iter().filter(|p| p.launch == at).collect();
                let transient: Vec<_> = wanted.iter().filter(|w| !w.counter).collect();
                assert_eq!(here.len(), transient.len(), "{what}: launch {at}'s scratch but its counts is placed: {here:?} of {wanted:?}");
                for p in &here {
                    let w = transient.iter().find(|w| w.tag == p.tag).unwrap_or_else(|| panic!("{what}: launch {at} takes no scratch `{}`", p.tag));
                    assert_eq!(p.bytes, w.bytes.next_multiple_of(ARENA_ALIGN), "{what}: launch {at}'s `{}` sized", p.tag);
                    assert!(p.offset % ARENA_ALIGN == 0 && p.offset + p.bytes <= a.bytes, "{what}: launch {at}'s `{}` aligned within the arena", p.tag);
                    let clear = |o: usize, b: usize| p.offset + p.bytes <= o || o + b <= p.offset;
                    for x in a.placed.iter().filter(|x| x.first <= at && at <= x.last) {
                        assert!(clear(x.offset, x.bytes), "{what}: launch {at}'s `{}` is clear of `{}` {:?}, live then", p.tag, x.name, x.touch);
                    }
                    for q in here.iter().filter(|q| q.tag != p.tag) {
                        assert!(clear(q.offset, q.bytes), "{what}: launch {at}'s `{}` and `{}` apart", p.tag, q.tag);
                    }
                }
                pointers += here.len();
            }
            for (i, x) in a.placed.iter().enumerate() {
                for y in &a.placed[i + 1..] {
                    if x.first <= y.last && y.first <= x.last {
                        assert!(
                            x.offset + x.bytes <= y.offset || y.offset + y.bytes <= x.offset,
                            "{what}: `{}` {:?} and `{}` {:?} are live together and apart in the arena",
                            x.name,
                            x.touch,
                            y.name,
                            y.touch
                        );
                    }
                }
            }
        }
        let largest = lane.forms.iter().map(|f| f.arena.bytes).max().unwrap();
        assert!(
            lane.program.cu.contains(&format!("CK(cudaMalloc(&lk_arena, {largest}UL)); CK(cudaMemset(lk_arena, 0xff, {largest}UL));")),
            "{what}: one arena, the largest form's, poisoned"
        );
        assert_eq!(lane.program.cu.matches("(lk_arena + ").count(), pointers, "{what}: every placed array's pointers into it");
    }
    for (what, lane) in lanes.iter().filter(|(w, _)| w.starts_with("medium")) {
        let f = lane.chunk(64).unwrap();
        let s = &f.schedule;
        let placed: usize = f.arena.placed.iter().map(|p| p.bytes).sum();
        let two = s.numerics == Q8MatKind::TwoPassInt8;
        let computed: usize = s
            .buffers
            .values()
            .filter(|b| b.fill == Fill::Computed && ![&s.io.logits, &s.io.next].contains(&&b.name))
            .map(|b| {
                usize::from(b.plain) * array_bytes(b.len, Touch::Plain, two) + usize::from(b.quantized) * array_bytes(b.len, Touch::Quant, two)
            })
            .sum();
        assert!(placed * 100 >= computed * 95, "{what}: the chunk places {placed} of its {computed} step-local bytes");
        assert!(f.arena.bytes * 3 < placed, "{what}: they fit in {} bytes of the arena", f.arena.bytes);
    }
}

/// The wide tiny model `t` written with each matrix in the format `format_of` gives its name (norms, biases, sinks and
/// an f32 router in `f32`).
fn wide_checkpoint(t: Tiny, tag: &str, format_of: &dyn Fn(&str) -> GgmlType) -> (ModelSpec, PathBuf) {
    sized_checkpoint(t.wide(CTX), &format!("{}-wide-{tag}", t.name()), format_of)
}

/// The tiny model at its medium width (`Tiny::medium`), its matrices in the formats `format_of` names.
fn medium_checkpoint(t: Tiny, tag: &str, format_of: &dyn Fn(&str) -> GgmlType) -> (ModelSpec, PathBuf) {
    sized_checkpoint(t.medium(CTX), &format!("{}-medium-{tag}", t.name()), format_of)
}

/// A checkpoint of `spec` under `name`, its matrices in the formats `format_of` names and the rest in `f32`.
fn sized_checkpoint(spec: ModelSpec, name: &str, format_of: &dyn Fn(&str) -> GgmlType) -> (ModelSpec, PathBuf) {
    let path = dir().join(format!("{name}.gguf"));
    let matrices: Vec<String> = tiny_models::tensors(&spec)
        .into_iter()
        .map(|(n, _)| n)
        .filter(|n| n.ends_with(".weight") && !n.contains("norm") && !n.contains("attn_sinks"))
        .filter(|n| !(spec.arch == "gpt-oss" && n.contains("ffn_gate_inp")))
        .collect();
    tiny_models::write_checkpoint_as(&spec, &path, &|name| {
        if matrices.iter().any(|m| m == name) {
            format_of(name)
        } else {
            GgmlType::F32
        }
    });
    (spec, path)
}

/// Every weight format the checkpoint reader dequantizes, the lane reads in place: the float formats as values, Q8_0 as
/// its split levels and scales, and every other through the block library's decoders (`kernels::decodes`) — and the
/// lane locks run each of those (`BLOCK_FORMATS`). A format the reader gains turns this red until the lane decodes it and
/// the locks run it.
#[test]
fn every_format_the_reader_dequantizes_the_lane_reads() {
    use logicaffeine_compile::codegen_cuda::lane::kernels::decodes;
    let read: Vec<GgmlType> = (0..=255u32).filter_map(|id| GgmlType::from_id(id).ok()).collect();
    let blocks: Vec<GgmlType> = read.iter().copied().filter(|t| !matches!(t, GgmlType::F32 | GgmlType::F16 | GgmlType::Bf16 | GgmlType::Q8_0)).collect();
    for ty in &blocks {
        assert!(decodes(*ty), "the lane decodes {}", ty.name());
        assert!(BLOCK_FORMATS.contains(ty), "the lane locks run {}", ty.name());
    }
    assert_eq!(BLOCK_FORMATS.len(), blocks.len(), "the locks run each block format once");
    for ty in [GgmlType::F32, GgmlType::F16, GgmlType::Bf16, GgmlType::Q8_0] {
        assert!(read.contains(&ty) && !decodes(ty), "{} is read by its own path", ty.name());
    }
}

/// Every quantized format the checkpoint reader knows is read in place: a dense model and the expert mixtures written
/// all in one format, for each, read every matrix as rows of that format through the block library (the embedding
/// table by its block gather, the projections by the block GEMV, the experts by the expert kernels in that format); a
/// matrix kept in `f32` is read by the f32 GEMV; and a checkpoint mixing every format builds. The dense model's first
/// norm is held apart from its gather (`embed_norm`), so the block gather stands as a launch of its own.
#[test]
fn every_block_format_is_read_in_place_through_its_decoder() {
    for &ty in &BLOCK_FORMATS {
        let (spec, path) = wide_checkpoint(Tiny::Qwen3, ty.name(), &|_| ty);
        let lane = compile_holding(&spec, &path, &options(Q8MatKind::Int8Act), &["embed_norm"]).unwrap_or_else(|e| panic!("{}: {e}", ty.name()));
        let s = &lane.decode().schedule;
        let gemvs: Vec<GgmlType> = s.launches.iter().filter_map(|l| if let Launch::Gemv { format, .. } = l { Some(*format) } else { None }).collect();
        assert!(!gemvs.is_empty() && gemvs.iter().all(|f| *f == ty), "{}: every projection is read in its format: {gemvs:?}", ty.name());
        assert!(
            s.launches.iter().any(|l| matches!(l, Launch::Gather { format: Some(f), .. } if *f == ty)),
            "{}: the embedding row is gathered from its blocks",
            ty.name()
        );
        assert!(lane.program.cu.contains(&format!("lk_gemv_blk<{}, 1, 1>", ty.id())), "{}: the block GEMV of its format", ty.name());
        for t in [Tiny::Qwen3Moe, Tiny::GptOss, Tiny::DeepSeek2] {
            let (spec, path) = wide_checkpoint(t, ty.name(), &|_| ty);
            let lane = compile(&spec, &path, &options(Q8MatKind::Int8Act)).unwrap_or_else(|e| panic!("{} {}: {e}", t.name(), ty.name()));
            for l in lane.decode().schedule.launches.iter().flat_map(|l| l.parts()) {
                if let Launch::Moe { formats, .. } = l {
                    assert_eq!(*formats, [ty; 3], "{} {}: the experts are read in their format", t.name(), ty.name());
                }
                if let Launch::HeadGemv { format, .. } = l {
                    assert_eq!(*format, ty, "{} {}: each head's rows are read in their format", t.name(), ty.name());
                }
            }
        }
    }
    let (spec, path) = wide_checkpoint(Tiny::Qwen3, "f32v", &|name| if name == "blk.1.attn_v.weight" { GgmlType::F32 } else { GgmlType::Q8_0 });
    let lane = compile(&spec, &path, &options(Q8MatKind::Int8Act)).expect("an f32 projection compiles");
    assert!(
        lane.decode().schedule.launches.iter().any(|l| matches!(l, Launch::GemvF32 { w, .. } if w == "Wv1")),
        "the f32 projection is read by the f32 GEMV"
    );
    if common::require_tool("nvcc") {
        for t in [Tiny::Qwen3, Tiny::Qwen3Moe, Tiny::GptOss, Tiny::DeepSeek2] {
            let (spec, path) = wide_checkpoint(t, "mixed", &|name| {
                BLOCK_FORMATS[name.bytes().fold(0usize, |h, b| h.wrapping_mul(31).wrapping_add(b as usize)) % BLOCK_FORMATS.len()]
            });
            for numerics in NUMERICS {
                let opts = LaneOptions { ctx: 64, prefill: 32, ..options(numerics) };
                let lane = compile(&spec, &path, &opts).unwrap_or_else(|e| panic!("{} mixed: {e}", t.name()));
                nvcc(&lane.program.cu, &dir().join(format!("{}-mixed-{numerics:?}", t.name())));
            }
        }
    }
}

/// Every kernel that reads a matrix's rows keeps its decode in registers: the decode step of an expert model with every
/// matrix in each format — its projections (`lk_gemv_blk`, `lk_gemv_q8`), gated pairs (`lk_gemv_glu`) and experts
/// (`lk_moe_up`, `lk_moe_down`) — under both quantized numerics compiles with no stack frame and no spill, as `ptxas`
/// reports it: a table a lane indexes by its weights' nibbles, read from local memory, cost MXFP4's experts a third of
/// their bandwidth.
#[test]
fn every_row_reading_kernel_keeps_its_decode_in_registers() {
    if !common::require_tool("nvcc") {
        return;
    }
    const ROW_READERS: [&str; 5] = ["lk_gemv_blk", "lk_gemv_q8", "lk_gemv_glu", "lk_moe_up", "lk_moe_down"];
    let formats: Vec<GgmlType> = std::iter::once(GgmlType::Q8_0).chain(BLOCK_FORMATS).collect();
    let jobs: Vec<(GgmlType, Q8MatKind)> =
        formats.iter().flat_map(|&ty| [Q8MatKind::Int8Act, Q8MatKind::TwoPassInt8].map(|n| (ty, n))).collect();
    let reports: Vec<(String, String)> = std::thread::scope(|s| {
        let handles: Vec<_> = jobs
            .iter()
            .map(|&(ty, numerics)| {
                s.spawn(move || {
                    let what = format!("{} {numerics:?}", ty.name());
                    let (spec, path) = wide_checkpoint(Tiny::Qwen3Moe, &format!("regs-{}", ty.name()), &|_| ty);
                    let lane = compile(&spec, &path, &options(numerics)).unwrap_or_else(|e| panic!("{what}: {e}"));
                    let src = dir().join(format!("regs-{}-{numerics:?}.cu", ty.name()));
                    std::fs::write(&src, &lane.program.cu).unwrap();
                    let out = Command::new("nvcc")
                        .args(["-O3", "-std=c++17", "-arch=sm_86", "-cubin", "-Xptxas", "-v", "-o", "/dev/null"])
                        .arg(&src)
                        .output()
                        .expect("nvcc runs");
                    assert!(out.status.success(), "{what} compiles:\n{}", String::from_utf8_lossy(&out.stderr));
                    (what, String::from_utf8_lossy(&out.stderr).into_owned())
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let (mut checked, mut kept) = (0, Vec::new());
    for (what, report) in &reports {
        let mut kernel: Option<&str> = None;
        for line in report.lines() {
            if let Some(name) = line.split("Function properties for ").nth(1) {
                kernel = Some(name.trim());
            } else if let (Some(k), true) = (kernel, line.contains("bytes stack frame")) {
                if ROW_READERS.iter().any(|r| k.contains(r)) {
                    checked += 1;
                    let bytes: Vec<usize> = line.split(',').map(|p| p.trim().split(' ').next().unwrap().parse().unwrap()).collect();
                    if bytes != [0, 0, 0] {
                        kept.push(format!("{what}: `{k}` keeps {}", line.trim()));
                    }
                }
                kernel = None;
            }
        }
    }
    assert!(checked >= jobs.len() * 3, "the row readers of every program are reported: {checked}");
    assert!(kept.is_empty(), "row readers keeping memory beside their registers:\n{}", kept.join("\n"));
}

/// Every block format's unit, as the kernels stage it, is the unit it decodes alone (`kernels::unit`): `lk_unit_bytes<F>`
/// its bytes and `lk_unit_subs<F>` its 32-weight sub-blocks — the checkpoint's block (`GgmlType::block_bytes`,
/// `block_len`) in every format but MXFP4, whose unit is its regrouped group of 16 sub-blocks, 272 bytes, and Q6_K,
/// whose unit is its group of eight blocks as uploaded (`lk_q6_regroup`), 1680 bytes — and `lk_axpy_pitch<F, AXPY_W>`
/// the staged row's bytes the emitter sizes its tile by (`kernels::axpy_pitch`) — the constexpr facts evaluated on the
/// host from the kernels' own source.
#[test]
fn every_block_formats_staged_unit_is_its_decoded_unit() {
    use logicaffeine_compile::codegen_cuda::lane::kernels::{axpy_pitch, unit, Kernel, AXPY_W, PRELUDE};
    if !common::require_tool("nvcc") {
        return;
    }
    let prints: String = BLOCK_FORMATS
        .iter()
        .map(|ty| {
            let f = ty.id();
            format!("  printf(\"{f} %d %d %d\\n\", lk_unit_bytes<{f}>(), lk_unit_subs<{f}>(), lk_axpy_pitch<{f}, {AXPY_W}>());\n")
        })
        .collect();
    let cu = format!(
        "#include <cstdio>\n#include <cuda_runtime.h>\n#include <cuda_fp16.h>\n{PRELUDE}{}{}\nint main(){{\n{prints}}}\n",
        Kernel::Blocks.source(),
        Kernel::Heads.source()
    );
    let src = dir().join("units.cu");
    let bin = dir().join("units");
    std::fs::write(&src, cu).unwrap();
    let built = Command::new("nvcc").args(["-O0", "-std=c++17", "-arch=sm_86", "-o"]).arg(&bin).arg(&src).output().expect("nvcc runs");
    assert!(built.status.success(), "the probe builds:\n{}", String::from_utf8_lossy(&built.stderr));
    let out = Command::new(&bin).output().expect("the probe runs");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let units: Vec<Vec<usize>> = stdout.lines().map(|l| l.split_whitespace().map(|v| v.parse().unwrap()).collect()).collect();
    assert_eq!(units.len(), BLOCK_FORMATS.len(), "a unit a format:\n{stdout}");
    for (ty, u) in BLOCK_FORMATS.iter().zip(&units) {
        assert_eq!(u[0], ty.id() as usize, "{}", ty.name());
        assert_eq!((u[1], u[2]), unit(*ty), "{}: the unit the kernels stage is the unit the emitter sizes", ty.name());
        if *ty == GgmlType::Mxfp4 {
            assert_eq!(unit(*ty), (272, 16), "MXFP4: a unit is its regrouped group of 16 sub-blocks");
        } else if *ty == GgmlType::Q6K {
            assert_eq!(unit(*ty), (1680, 64), "Q6_K: a unit is its group of eight blocks");
        } else {
            assert_eq!(unit(*ty), (ty.block_bytes(), ty.block_len() / 32), "{}: a unit is the checkpoint's block", ty.name());
        }
        assert_eq!(u[3], axpy_pitch(*ty), "{}: the staged pitch the emitter sizes its tile by", ty.name());
    }
}

/// Build `cu` with `nvcc` into `bin`.
fn nvcc(cu: &str, bin: &Path) {
    let src = bin.with_extension("cu");
    std::fs::write(&src, cu).unwrap();
    let out = Command::new("nvcc")
        .args(["-O3", "-std=c++17", "-arch=sm_86", "-o"])
        .arg(bin)
        .arg(&src)
        .output()
        .expect("nvcc runs");
    assert!(out.status.success(), "{} does not compile:\n{}", src.display(), String::from_utf8_lossy(&out.stderr));
}

#[test]
fn every_program_compiles_with_nvcc() {
    if !common::require_tool("nvcc") {
        return;
    }
    let jobs: Vec<(Tiny, Q8MatKind)> = ALL.iter().flat_map(|&t| NUMERICS.map(|n| (t, n))).collect();
    let lanes: Vec<(String, Lane)> =
        jobs.iter().map(|&(t, n)| (format!("{}-{n:?}", t.name()), compiled_chunks(t, n, 16, "nvcc"))).collect();
    std::thread::scope(|s| {
        for chunk in lanes.chunks(lanes.len().div_ceil(4)) {
            s.spawn(move || {
                for (name, lane) in chunk {
                    nvcc(&lane.program.cu, &dir().join(format!("lane-{name}")));
                }
            });
        }
    });
}

/// The program checks, before it touches the GPU, that it reads the checkpoint and the sidecar it was compiled with.
#[test]
fn a_program_refuses_a_checkpoint_or_sidecar_it_was_not_compiled_with() {
    if !common::require_tool("nvcc") {
        return;
    }
    let (spec, path) = checkpoint(Tiny::Qwen3, "identity");
    let lane = compile(&spec, &path, &options(Q8MatKind::Int8Act)).unwrap();
    let bin = dir().join("lane-identity");
    nvcc(&lane.program.cu, &bin);
    let side = dir().join("identity.side");
    std::fs::write(&side, &lane.program.sidecar).unwrap();
    let run = |ckpt: &Path, side: &Path| Command::new(&bin).arg(ckpt).arg(side).args(["2", "5"]).output().unwrap();

    let other = dir().join("identity-other.gguf");
    tiny_models::write_checkpoint(&Tiny::Llama.spec(CTX), &other);
    let out = run(&other, &side);
    assert_eq!(out.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&out.stderr).contains("is not the checkpoint this program was compiled against"));

    let mut bytes = std::fs::read(&path).unwrap();
    let g = Gguf::open(&path).unwrap();
    let flip = g.data_start() as usize - 1;
    bytes[flip] ^= 1;
    let altered = dir().join("identity-altered.gguf");
    std::fs::write(&altered, &bytes).unwrap();
    let out = run(&altered, &side);
    assert_eq!(out.status.code(), Some(3), "a header that differs by one bit is another checkpoint");

    let short = dir().join("identity-short.side");
    std::fs::write(&short, &lane.program.sidecar[..lane.program.sidecar.len() - 4]).unwrap();
    let out = run(&path, &short);
    assert_eq!(out.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&out.stderr).contains("is not this program's sidecar"));
}

/// A rotation reads the side it turns through a fixed branch. A by-value `lk_rope_side` chosen at run time
/// (`side ? b : a`) is copied into a local stack frame, and every field the head's loops read goes to local memory:
/// the fused projection-and-rotation ran 41 µs a launch against 11.5 µs without the copy, and Gemma-3-1B's decode fell
/// from 235 to 190 tokens a second. Every rotation kernel any tiny architecture's lane instantiates — the rotation, the
/// paired rotation and the projection that rotates its heads in its epilogue — compiles with no stack frame
/// (`ptxas -v`).
#[test]
fn the_rotation_kernels_keep_no_stack_frame() {
    if !common::require_tool("nvcc") {
        return;
    }
    let mut frames: Vec<(String, String, u64)> = Vec::new();
    for &t in ALL {
        let lane = compiled_chunks(t, Q8MatKind::Int8Act, 4, "rope-stack");
        let src = dir().join(format!("lane-rope-stack-{}.cu", t.name()));
        std::fs::write(&src, &lane.program.cu).unwrap();
        let out = Command::new("nvcc")
            .args(["-O3", "-std=c++17", "-arch=sm_86", "-Xptxas", "-v", "-c", "-o"])
            .arg(src.with_extension("o"))
            .arg(&src)
            .output()
            .expect("nvcc runs");
        let report = String::from_utf8_lossy(&out.stderr);
        assert!(out.status.success(), "{}'s lane compiles:\n{report}", t.name());
        let mut kernel: Option<String> = None;
        for line in report.lines() {
            if let Some(at) = line.find("Function properties for ") {
                kernel = Some(line[at + "Function properties for ".len()..].trim().to_string());
            } else if let (Some(k), Some(at)) = (&kernel, line.find(" bytes stack frame")) {
                let bytes: u64 = line[..at].trim().rsplit(' ').next().and_then(|n| n.parse().ok()).expect("the frame's bytes");
                if k.contains("lk_rope") || k.contains("lk_gemv_parts_rope") {
                    frames.push((t.name().to_string(), k.clone(), bytes));
                }
                kernel = None;
            }
        }
    }
    for kind in ["lk_gemv_parts_rope", "lk_rope_parts"] {
        assert!(frames.iter().any(|(_, k, _)| k.contains(kind)), "some tiny architecture's lane instantiates {kind}: {frames:?}");
    }
    let framed: Vec<_> = frames.iter().filter(|(_, _, b)| *b > 0).collect();
    assert!(framed.is_empty(), "a rotation kernel keeps a stack frame (a side chosen at run time?): {framed:?}");
}

/// An array a kernel indexes by a value known only at run time cannot live in registers: the front end gives it a
/// local depot (`__local_depot` in the PTX) and every access becomes a load or store through local memory, as the
/// attention's score runs once read back which of their keys had rows (`rw[b]`, `b` from the halves a lane kept) and
/// the router kept its top `K` weights (`pick[r]`). The spills ptxas chooses and the registers a call saves never reach
/// the PTX, so a depot names exactly an array the source indexes so. No kernel of any tiny architecture's lane — under
/// every numerics, its chunks and its batch of 64 — keeps one.
#[test]
fn no_lane_kernel_keeps_an_array_in_local_memory() {
    if !common::require_tool("nvcc") {
        return;
    }
    let mut programs: Vec<(String, PathBuf)> = Vec::new();
    for &t in ALL {
        let (spec, path) = checkpoint(t, "depot");
        for numerics in NUMERICS {
            let chunks = compile(&spec, &path, &chunked(numerics, 4)).unwrap_or_else(|e| panic!("{} {numerics:?}: {e}", t.name()));
            let batch = compile_batch(&spec, &path, &chunked(numerics, 4), 64).unwrap_or_else(|e| panic!("{} {numerics:?} batch: {e}", t.name()));
            for (form, lane) in [("chunks", chunks), ("batch", batch)] {
                let src = dir().join(format!("lane-depot-{}-{numerics:?}-{form}.cu", t.name()));
                std::fs::write(&src, &lane.program.cu).unwrap();
                programs.push((format!("{} {numerics:?} {form}", t.name()), src));
            }
        }
    }
    let ptx = |src: &Path| -> String {
        let out = Command::new("nvcc").args(["-O3", "-std=c++17", "-arch=sm_86", "-ptx", "-o"]).arg(src.with_extension("ptx")).arg(src).output().expect("nvcc runs");
        assert!(out.status.success(), "{} compiles to PTX:\n{}", src.display(), String::from_utf8_lossy(&out.stderr));
        std::fs::read_to_string(src.with_extension("ptx")).unwrap()
    };
    let compiled: Vec<(String, String)> = std::thread::scope(|scope| {
        let workers: Vec<_> = programs.chunks(programs.len().div_ceil(4)).map(|part| scope.spawn(move || part.iter().map(|(what, src)| (what.clone(), ptx(src))).collect::<Vec<_>>())).collect();
        workers.into_iter().flat_map(|w| w.join().unwrap()).collect()
    });
    let mut kernels: BTreeSet<String> = BTreeSet::new();
    let mut depots: Vec<(String, String)> = Vec::new();
    for (what, text) in &compiled {
        for entry in text.split(".entry ").skip(1) {
            let name = entry.split('(').next().unwrap_or("").trim().to_string();
            if entry.contains("__local_depot") {
                depots.push((what.clone(), name.clone()));
            }
            kernels.insert(name);
        }
    }
    for kind in ["lk_attention", "lk_moe_route", "lk_rope", "lk_gemv"] {
        assert!(kernels.iter().any(|k| k.contains(kind)), "some tiny architecture's lane instantiates {kind}: {kernels:?}");
    }
    assert!(depots.is_empty(), "a kernel keeps an array in local memory (indexed by a value known only at run time): {depots:?}");
}

/// An attention launch compiles the one path it takes past a row of one chunk and its run width (`lk_attention`'s
/// `PATH` and `RW`), named in its kernel: a grid of split blocks (its third dimension past 1) folded by the last to
/// arrive, a row's chunks held side by side by its one block (its last argument, the chunks at a time, past 0), weighed
/// one at a time by it (its splits past 1), or a row of one chunk; a run a key of a half warp for a head of 9 to 16
/// fours, the whole warp otherwise. Compiled together the paths take the registers of all of them, and the kernel
/// spilled under two blocks a multiprocessor and three; each alone takes its own. Every tiny architecture's chunks and
/// batch of 64 over a cache of 64 positions, under every numerics; split, held and one-chunk launches all occur.
#[test]
fn an_attention_launch_compiles_only_the_path_it_takes() {
    // A launch's comma-separated list, commas inside brackets and braces kept.
    let top = |list: &str| -> Vec<String> {
        let (mut depth, mut at, mut out) = (0i32, 0, Vec::new());
        for (i, c) in list.char_indices() {
            match c {
                '(' | '{' | '<' => depth += 1,
                ')' | '}' | '>' => depth -= 1,
                ',' if depth == 0 => {
                    out.push(list[at..i].trim().to_string());
                    at = i + 1;
                }
                _ => {}
            }
        }
        out.push(list[at..].trim().to_string());
        out
    };
    // A cache long enough that a row cuts into chunks: split, held and one-chunk launches all occur.
    const LONG: usize = 64;
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    for &t in ALL {
        let spec = t.spec(LONG);
        let path = dir().join(format!("{}-attn-path.gguf", t.name()));
        tiny_models::write_checkpoint(&spec, &path);
        for numerics in NUMERICS {
            let opts = LaneOptions { ctx: LONG, ..chunked(numerics, 4) };
            let chunks = compile(&spec, &path, &opts).unwrap_or_else(|e| panic!("{} {numerics:?}: {e}", t.name()));
            let batch = compile_batch(&spec, &path, &opts, 64).unwrap_or_else(|e| panic!("{} {numerics:?} batch: {e}", t.name()));
            for (form, lane) in [("chunks", chunks), ("batch", batch)] {
                let what = format!("{} {numerics:?} {form}", t.name());
                for line in lane.program.cu.lines().map(str::trim).filter(|l| l.starts_with("lk_attention<")) {
                    let (targs, rest) = line["lk_attention<".len()..].split_once("><<<").unwrap_or_else(|| panic!("{what}: `{line}`"));
                    let (config, args) = rest.split_once(">>>(").unwrap_or_else(|| panic!("{what}: `{line}`"));
                    let (targs, config, args) = (top(targs), top(config), top(args.trim_end_matches(");")));
                    let grid = top(config[0].trim_start_matches("dim3(").trim_end_matches(')'));
                    let z: usize = grid.get(2).map_or(1, |z| z.parse().unwrap());
                    let num = |i: usize| -> usize { args[i].parse().unwrap_or_else(|_| panic!("{what}: argument {i} of `{line}`")) };
                    let (hd, splits, held) = (num(10), num(19), num(21));
                    let path = if z > 1 {
                        "LK_ATTN_SPLIT"
                    } else if held > 0 {
                        "LK_ATTN_HELD"
                    } else if splits > 1 {
                        "LK_ATTN_CHUNKS"
                    } else {
                        "LK_ATTN_ONE"
                    };
                    let width = if hd / 4 > 8 && hd / 4 <= 16 { "16" } else { "32" };
                    assert_eq!(targs.len(), 9, "{what}: `{line}` names every template argument");
                    assert_eq!((targs[7].as_str(), targs[8].as_str()), (path, width), "{what}: `{line}` compiles the path it takes and its run width");
                    *seen.entry(path.to_string()).or_insert(0) += 1;
                }
            }
        }
    }
    for path in ["LK_ATTN_SPLIT", "LK_ATTN_HELD", "LK_ATTN_ONE"] {
        assert!(seen.contains_key(path), "some tiny architecture's lane launches an attention on {path}: {seen:?}");
    }
}

/// The launch of `s` that writes `t`.
fn writer<'a>(s: &'a Schedule, t: &str) -> &'a Launch {
    s.launches.iter().find(|l| l.writes().contains(&t)).unwrap_or_else(|| panic!("no launch writes `{t}`"))
}

/// The tensors `s`'s heads read: each product to the vocabulary's width.
fn head_inputs(s: &Schedule) -> Vec<&str> {
    s.launches
        .iter()
        .flat_map(|l| l.parts())
        .filter_map(|l| match l {
            Launch::Gemv { x, n, .. } if *n == tiny_models::VOCAB => Some(x.as_str()),
            _ => None,
        })
        .collect()
}

/// MiniCPM's head (PARITY-NORM-SCALE): the output norm's result times `256/d` before the head, the scale inside the
/// norm's launch (`norm_scale`) — in the decode step the launch that adds the last residual and norms the sum, in a chunk
/// of four the norm of its last row — the head's activation written by that norm, quantized in its epilogue where the
/// head reads it so, no launch scaling or quantizing it apart, under every numerics.
#[test]
fn a_scaled_norm_is_one_row_kernel() {
    for numerics in NUMERICS {
        let lane = compiled_chunks(Tiny::MiniCpm, numerics, 4, "norm-scale");
        for f in &lane.forms {
            let what = format!("minicpm {numerics:?} rows {}", f.rows);
            let s = &f.schedule;
            assert_eq!(s.fired("norm_scale"), 1, "{what}: the output norm takes the head's scale");
            let heads = head_inputs(s);
            assert_eq!(heads.len(), 1, "{what}: one head");
            let norm = writer(s, heads[0]);
            assert!(matches!(norm, Launch::Norm { scale: Some(c), .. } if *c == 0.25), "{what}: the norm writes the head's activation times 256/d: {norm:?}");
            if f.rows == 1 {
                assert!(matches!(norm, Launch::Norm { add: Some(_), .. }), "{what}: the decode step's head norm adds the last residual: {norm:?}");
            }
            let quantized = s.launches.iter().any(|l| matches!(l, Launch::Quant { x, .. } if x == heads[0]));
            assert!(!quantized, "{what}: the scaled norm quantizes the head's activation in its own epilogue");
        }
    }
}

/// Granite's step (PARITY-GEMM-SCALE): each sublayer's output projection times `residual_scale` before the residual, and
/// the logits times `1/logit_scale` after the head, every scale inside its product's epilogue (`gemm_scale`) — six
/// residual scales and the head's, no launch scaling a tensor apart, the arg-max reading the head's own result — in the
/// decode step and a chunk of four, under every numerics.
#[test]
fn a_scaled_projection_scales_in_its_epilogue() {
    for numerics in NUMERICS {
        let lane = compiled_chunks(Tiny::Granite, numerics, 4, "gemm-scale");
        for f in &lane.forms {
            let what = format!("granite {numerics:?} rows {}", f.rows);
            let s = &f.schedule;
            assert_eq!(s.fired("gemm_scale"), 7, "{what}: every scaled product scales in its epilogue");
            let scaled: Vec<f64> = s
                .launches
                .iter()
                .flat_map(|l| l.parts())
                .filter_map(|l| match l {
                    Launch::Gemv { epi: Some(Epilogue { scale: Some(ScaleBy::Factor(c)), after: None, relu_sq: false }), .. } => Some(*c),
                    _ => None,
                })
                .collect();
            assert_eq!(scaled, [0.25, 0.25, 0.25, 0.25, 0.25, 0.25, 0.125], "{what}: each sublayer's output projection times 1/4, the head times 1/8");
            let apart: Vec<&Launch> = s.launches.iter().flat_map(|l| l.parts()).filter(|l| matches!(l, Launch::Scale { .. })).collect();
            assert!(apart.is_empty(), "{what}: no launch scales apart: {apart:?}");
            for l in s.launches.iter().filter(|l| matches!(l, Launch::ArgMax { .. })) {
                let Launch::ArgMax { x, .. } = l else { unreachable!() };
                let head = writer(s, x);
                assert!(matches!(head, Launch::Gemv { n, .. } if *n == tiny_models::VOCAB), "{what}: the head writes the scaled logits the arg-max reads: {head:?}");
            }
        }
    }
}

/// Arcee's FFN (PARITY-RELU-SQR): relu² of the up projection, no gate, every layer, in the decode step and a chunk of
/// four — folded into the up projection's epilogue where nothing reads it quantized (the precise numerics) or where the
/// projection quantizes its outputs too (the medium width in Q4_K, four rows a warp: one launch for the projection, the
/// relu² and the down projection's quantized activation); otherwise one launch of its own that quantizes its result for
/// the down projection (a Q8_0 projection takes one row a warp and keeps its plain result). No launch squares apart and
/// no launch quantizes it apart.
#[test]
fn a_relu_square_compiles_and_folds_into_its_producer() {
    let (tiny, tiny_path) = checkpoint(Tiny::Arcee, "relu-sqr");
    let (medium, medium_path) = medium_checkpoint(Tiny::Arcee, "relu-sqr", &|_| GgmlType::Q4K);
    let cases = NUMERICS.iter().map(|&n| ("arcee", &tiny, &tiny_path, n)).chain([("arcee medium q4k", &medium, &medium_path, Q8MatKind::Int8Act)]);
    for (name, spec, path, numerics) in cases {
        let lane = compile(spec, path, &chunked(numerics, 4)).unwrap_or_else(|e| panic!("{name} {numerics:?}: {e}"));
        for f in &lane.forms {
            let what = format!("{name} {numerics:?} rows {}", f.rows);
            let s = &f.schedule;
            assert_eq!(s.fired("relu_sqr"), 3, "{what}: every layer squares its ReLU in one launch");
            let alone: Vec<&Launch> = s.launches.iter().flat_map(|l| l.parts()).filter(|l| matches!(l, Launch::Relu { .. })).collect();
            let folded: Vec<&Launch> =
                s.launches.iter().flat_map(|l| l.parts()).filter(|l| matches!(l, Launch::Gemv { epi: Some(Epilogue { relu_sq: true, .. }), .. })).collect();
            let fold = numerics == Q8MatKind::Dequant || (name == "arcee medium q4k" && f.rows == 1);
            if fold {
                assert!(alone.is_empty(), "{what}: relu² rides its producer: {alone:?}");
                assert_eq!(folded.len(), 3, "{what}: each up projection squares the ReLU of its result: {folded:?}");
                for l in &folded {
                    let Launch::Gemv { quant, f32, per_warp, .. } = l else { unreachable!() };
                    let q = numerics != Q8MatKind::Dequant;
                    assert_eq!((*quant, *f32), (q, !q), "{what}: the projection writes what the down projection reads of it: {l:?}");
                    if q {
                        assert_eq!(*per_warp, 4, "{what}: four rows a warp, so the projection quantizes in its epilogue");
                    }
                }
            } else {
                assert!(folded.is_empty(), "{what}: the projection keeps its result plain: {folded:?}");
                assert_eq!(alone.len(), 3, "{what}: one launch squares each layer's ReLU: {alone:?}");
                for l in &alone {
                    let Launch::Relu { square, f32, quant, .. } = l else { unreachable!() };
                    assert!(*square, "{what}: the ReLU squares its result: {l:?}");
                    assert_eq!((*quant, *f32), (true, false), "{what}: quantized for the down projection and kept only so: {l:?}");
                }
            }
            let hidden: Vec<&str> = alone.iter().chain(&folded).flat_map(|l| l.writes()).collect();
            for t in hidden {
                assert!(!s.launches.iter().any(|q| matches!(q, Launch::Quant { x, .. } if x == t)), "{what}: `{t}` is quantized in its producer's epilogue");
            }
            let squares: Vec<&Launch> = s.launches.iter().flat_map(|l| l.parts()).filter(|l| matches!(l, Launch::Binary { a, b, .. } if a == b)).collect();
            assert!(squares.is_empty(), "{what}: no launch squares apart: {squares:?}");
        }
    }
}

/// Gemma 2's step (PARITY-SOFTCAP): every score soft-capped at 50 inside its attention's own kernel — the even layers
/// windowed — and the logits capped at 30 inside the arg-max that reads them, reading the head's own result and writing
/// the capped logits the host reads, under every numerics, in the decode step and a chunk of four; each attention launch
/// the program prints passes its cap.
#[test]
fn a_capped_attention_caps_inside_its_kernel() {
    for numerics in NUMERICS {
        let lane = compiled_chunks(Tiny::Gemma2, numerics, 4, "softcap");
        for f in &lane.forms {
            let what = format!("gemma2 {numerics:?} rows {}", f.rows);
            let s = &f.schedule;
            let attention: Vec<(Option<usize>, Option<f64>)> =
                s.launches.iter().filter_map(|l| if let Launch::Attention { window, cap, .. } = l { Some((*window, *cap)) } else { None }).collect();
            let w = Some(tiny_models::WINDOW);
            assert_eq!(attention, [(w, Some(50.0)), (None, Some(50.0)), (w, Some(50.0))], "{what}: every layer caps its scores, the even ones windowed");
            assert!(!s.launches.iter().any(|l| matches!(l, Launch::Softcap { .. })), "{what}: no launch caps the logits apart: {:?}", s.census());
            let picks: Vec<&Launch> = s.launches.iter().filter(|l| matches!(l, Launch::ArgMax { .. })).collect();
            let [Launch::ArgMax { x, cap, y, .. }] = picks[..] else { panic!("{what}: one arg-max picks the token: {picks:?}") };
            assert_eq!(*cap, Some(30.0), "{what}: the logits are capped at 30 inside the arg-max");
            assert_eq!(y.as_deref(), Some(s.io.logits.as_str()), "{what}: the arg-max writes the capped logits the host reads");
            assert!(matches!(writer(s, x), Launch::Gemv { n, .. } if *n == tiny_models::VOCAB), "{what}: the arg-max reads the head's result");
        }
        let lines: Vec<&str> =
            lane.program.cu.lines().map(str::trim).filter(|l| l.starts_with("lk_attention") && l.contains("<<<") && !l.starts_with("lk_attention_fold")).collect();
        assert!(!lines.is_empty(), "gemma2 {numerics:?}: the program attends");
        for line in lines {
            assert!(line.ends_with(", 5e1f);"), "gemma2 {numerics:?}: `{line}` passes its cap");
        }
    }
}

/// Gemma 2's final cap rides its arg-max (PARITY-SOFTCAP, `cap_argmax`): in every form — the decode step and its chunks
/// and a batch of two, under every numerics — no launch caps the logits; every arg-max reads the head's own result and
/// caps each value in its registers at 30, one fold a sequence, and writes the capped logits exactly where the host reads
/// them — not where it reads none (`CudaKnobs::logits` `Unread`), whose program writes no logits. Held back
/// (`cap_argmax`), the cap is its own launch. A model with no cap compiles the same launches either way.
#[test]
fn a_greedy_step_folds_its_final_cap_into_its_arg_max() {
    use logicaffeine_compile::codegen_cuda::Logits;
    let (spec, path) = checkpoint(Tiny::Gemma2, "greedy");
    let caps = |s: &Schedule| s.launches.iter().filter(|l| matches!(l, Launch::Softcap { .. })).count();
    let picks = |s: &Schedule| -> Vec<(Option<f64>, bool)> {
        s.launches.iter().filter_map(|l| if let Launch::ArgMax { cap, y, .. } = l { Some((*cap, y.is_some())) } else { None }).collect()
    };
    for numerics in NUMERICS {
        let read = LaneOptions { prefill: 4, ..options(numerics) };
        let greedy = LaneOptions { knobs: CudaKnobs { logits: Logits::Unread, ..read.knobs }, ..read };
        for batch in [None, Some(2)] {
            let build = |opts: &LaneOptions, held: &[&str]| {
                match batch {
                    None => compile_holding(&spec, &path, opts, held),
                    Some(b) => compile_batch_holding(&spec, &path, opts, b, held),
                }
                .unwrap_or_else(|e| panic!("gemma2 {numerics:?} batch {batch:?}: {e}"))
            };
            let (lane, kept, held) = (build(&greedy, &[]), build(&read, &[]), build(&read, &["cap_argmax"]));
            for ((f, k), h) in lane.forms.iter().zip(&kept.forms).zip(&held.forms) {
                let what = format!("gemma2 {numerics:?} batch {batch:?} rows {}", f.rows);
                for (s, written, how) in [(&f.schedule, false, "unread"), (&k.schedule, true, "read")] {
                    assert_eq!(caps(s), 0, "{what}, logits {how}: no launch caps the logits apart: {:?}", s.census());
                    assert_eq!(s.fired("cap_argmax"), 1 + s.io.more.len(), "{what}, logits {how}: one fold a sequence");
                    assert!(!picks(s).is_empty() && picks(s).iter().all(|&p| p == (Some(30.0), written)), "{what}, logits {how}: every arg-max caps each value it reads at 30 and writes the capped logits exactly where the host reads them: {:?}", picks(s));
                    for l in s.launches.iter().filter(|l| matches!(l, Launch::ArgMax { .. })) {
                        let Launch::ArgMax { x, .. } = l else { unreachable!() };
                        assert!(matches!(writer(s, x), Launch::Gemv { n, .. } if *n == tiny_models::VOCAB), "{what}, logits {how}: the arg-max reads the head's own result: {l:?}");
                    }
                }
                let hs = &h.schedule;
                assert!(caps(hs) > 0 && hs.fired("cap_argmax") == 0 && picks(hs).iter().all(|p| p.0.is_none()), "{what}: held back, the cap is its own launch");
            }
            assert!(lane.program.cu.contains("static const bool LOGITS = false;"), "gemma2 {numerics:?}: the greedy program writes no logits");
            assert!(kept.program.cu.contains("static const bool LOGITS = true;"), "gemma2 {numerics:?}: the program writes the logits its host reads");
        }
    }
    for t in [Tiny::Llama, Tiny::Qwen35] {
        let (spec, path) = checkpoint(t, "greedy");
        let read = LaneOptions { prefill: 4, ..options(Q8MatKind::Int8Act) };
        let greedy = LaneOptions { knobs: CudaKnobs { logits: Logits::Unread, ..read.knobs }, ..read };
        let (a, b) = (compile(&spec, &path, &read).unwrap(), compile(&spec, &path, &greedy).unwrap());
        for (x, y) in a.forms.iter().zip(&b.forms) {
            assert_eq!(x.schedule.launches, y.schedule.launches, "{} rows {}: no cap, the same launches", t.name(), x.rows);
        }
    }
}

/// Every attention kernel takes the soft cap and the window (PARITY-ATTENTION-VARIANTS-INLINE): Gemma 2 at its own width
/// in the decode step (`lk_attention`) and in chunks of thirty-two (`lk_attention_flash` under the two-word numerics, its
/// heads short of a two-word block; `lk_attention_mma` under one word), four times as wide in chunks of sixty-four
/// (`lk_attention_i8`, `lk_attention_mma`), and with heads of 288 (`lk_attention_tiled`) — every attention launch capped
/// at 50 and the even layers windowed, every launch line passing its cap, and every program building.
#[test]
fn every_attention_kernel_takes_a_cap_and_a_mask() {
    let mut wide_heads = Tiny::Gemma2.spec(32);
    for layer in &mut wide_heads.layers {
        if let Mixer::Attention(a) = &mut layer.mixer {
            (a.head_dim, a.value_dim, a.scale) = (288, 288, 1.0 / 288f64.sqrt());
            if let Some(r) = &mut a.rope {
                r.dims = 288;
            }
        }
    }
    let wide = Tiny::Gemma2.wide(128);
    let cases: Vec<(&str, ModelSpec, usize, usize)> = vec![("gemma2", Tiny::Gemma2.spec(64), 64, 32), ("gemma2 wide", wide, 128, 64), ("gemma2 heads 288", wide_heads, 32, 32)];
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    let mut programs: Vec<(String, String)> = Vec::new();
    for (name, spec, ctx, prefill) in &cases {
        let path = dir().join(format!("{}-caps.gguf", name.replace(' ', "-")));
        tiny_models::write_checkpoint(spec, &path);
        for numerics in NUMERICS {
            let what = format!("{name} {numerics:?}");
            let opts = LaneOptions { ctx: *ctx, prefill: *prefill, ..options(numerics) };
            let lane = compile(spec, &path, &opts).unwrap_or_else(|e| panic!("{what}: {e}"));
            for f in &lane.forms {
                let windows: Vec<Option<usize>> = f
                    .schedule
                    .launches
                    .iter()
                    .filter_map(|l| match l {
                        Launch::Attention { window, cap, .. } => {
                            assert_eq!(*cap, Some(50.0), "{what} rows {}: {l:?}", f.rows);
                            Some(*window)
                        }
                        _ => None,
                    })
                    .collect();
                assert_eq!(windows, [Some(tiny_models::WINDOW), None, Some(tiny_models::WINDOW)], "{what} rows {}: the even layers windowed", f.rows);
            }
            for line in lane.program.cu.lines().map(str::trim).filter(|l| l.starts_with("lk_attention") && l.contains("<<<") && !l.starts_with("lk_attention_fold")) {
                assert!(line.ends_with(", 5e1f);"), "{what}: `{line}` passes its cap");
                for kernel in ["lk_attention_mma", "lk_attention_i8", "lk_attention_flash", "lk_attention_tiled", "lk_attention<"] {
                    if line.starts_with(kernel) {
                        seen.insert(kernel);
                    }
                }
            }
            programs.push((format!("caps-{}-{numerics:?}", name.replace(' ', "-")), lane.program.cu.clone()));
        }
    }
    assert_eq!(seen.len(), 5, "every attention kernel the lane picks for a model's heads attends a capped model: {seen:?}");
    if common::require_tool("nvcc") {
        for batch in programs.chunks(6) {
            std::thread::scope(|scope| {
                for (name, cu) in batch {
                    scope.spawn(move || nvcc(cu, &dir().join(name)));
                }
            });
        }
    }
}

/// The `cache_precision` knob stores every attention's cache quantized, one byte a value in fp8 e4m3
/// (PARITY-KV-CACHE-QUANT): at `CachePrecision::E4m3` every array an attention of any form reads its keys or values from
/// (followed through the views into it to the carried array they view) is a `DType::E4m3` buffer in every form that
/// holds it, and so is every view into one; no other array is narrower than `f32`; the program declares each cache
/// `__nv_fp8_e4m3`, allocated once at one byte an element, and nothing else; the pool the plan sizes shrinks by exactly
/// what those allocations save against `f32`. Every architecture under every numerics, with the decode step, its chunks
/// and its batch; and each program builds — every writer rounding into the cache, every attention kernel reading it
/// (the decode step's and the shared-row kernel at every architecture's own width, the tiles four times as wide in
/// chunks of 64 under both quantized numerics, the flash tiles of Gemma 2's narrow heads).
#[test]
fn a_quantized_cache_is_written_and_read_in_its_format() {
    use logicaffeine_compile::codegen_cuda::lane::plan::DType;
    use logicaffeine_compile::codegen_cuda::CachePrecision;
    let mangled = |name: &str| -> String { name.chars().map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' }).collect() };
    let root = |s: &Schedule, name: &str| -> String {
        let mut n = name.to_string();
        while let Some(Fill::Alias { of, .. }) = s.buffers.get(&n).map(|b| &b.fill) {
            n = of.clone();
        }
        n
    };
    let e4m3 = |numerics: Q8MatKind, prefill: usize, ctx: usize| LaneOptions {
        ctx,
        prefill,
        knobs: CudaKnobs { cache_precision: CachePrecision::E4m3, ..options(numerics).knobs },
    };
    let mut programs: Vec<(String, String)> = Vec::new();
    for &t in ALL {
        let (spec, path) = checkpoint(t, "cache-e4m3");
        for numerics in NUMERICS {
            let what = format!("{} {numerics:?}", t.name());
            let plain = compile_batch(&spec, &path, &LaneOptions { prefill: 4, ..options(numerics) }, 4).unwrap_or_else(|e| panic!("{what}: {e}"));
            let lane = compile_batch(&spec, &path, &e4m3(numerics, 4, CTX), 4).unwrap_or_else(|e| panic!("{what} e4m3: {e}"));
            let mut caches = BTreeSet::new();
            for f in &lane.forms {
                for l in &f.schedule.launches {
                    if let Launch::Attention { kc, vc, .. } = l {
                        for c in [kc, vc] {
                            let r = root(&f.schedule, c);
                            if f.schedule.buffers[&r].fill == Fill::Zero {
                                caches.insert(r);
                            }
                        }
                    }
                }
            }
            assert!(!caches.is_empty(), "{what}: the step attends over a cache");
            for f in &lane.forms {
                let s = &f.schedule;
                for (name, b) in &s.buffers {
                    if caches.contains(&root(s, name)) {
                        assert_eq!(b.dtype, DType::E4m3, "{what} {}: the cache `{name}`", f.tag());
                    } else {
                        assert!(b.dtype.bytes() == 4 || b.dtype == DType::Bf16, "{what} {}: `{name}` is no cache", f.tag());
                    }
                }
            }
            let cu = &lane.program.cu;
            let arrays: Vec<&str> = cu.lines().filter_map(|l| l.trim().strip_prefix("__nv_fp8_e4m3* ")?.strip_suffix(" = nullptr;")).collect();
            let declared: BTreeSet<String> = arrays.iter().filter_map(|a| a.strip_prefix('t')?.split_once('_').map(|(_, name)| name.to_string())).collect();
            assert_eq!(declared, caches.iter().map(|c| mangled(c)).collect::<BTreeSet<_>>(), "{what}: every cache declared in e4m3, and no other array");
            let mut saved = 0;
            for a in &arrays {
                let at = format!("lk_malloc(&{a}, ");
                let mut lines = cu.lines().filter_map(|l| Some(&l[l.find(&at)? + at.len()..]));
                let len: usize = lines
                    .next()
                    .and_then(|rest| rest.split_once("UL * 1);"))
                    .and_then(|(len, _)| len.parse().ok())
                    .unwrap_or_else(|| panic!("{what}: `{a}` allocated at one byte an element"));
                assert!(lines.next().is_none(), "{what}: `{a}` allocated once");
                saved += pooled(len * 4) - pooled(len);
            }
            assert!(saved > 0, "{what}: the caches take less in e4m3");
            assert_eq!(plain.plan.fixed - lane.plan.fixed, saved, "{what}: the pool shrinks by what the program's e4m3 caches save, and by nothing else");
            if numerics == Q8MatKind::Int8Act {
                programs.push((format!("cache-e4m3-{}", t.name()), lane.program.cu.clone()));
            }
        }
    }
    for numerics in [Q8MatKind::Int8Act, Q8MatKind::TwoPassInt8] {
        for t in [Tiny::Qwen3, Tiny::Gemma3, Tiny::DeepSeek2] {
            let (spec, path) = wide_checkpoint(t, "cache-e4m3", &|_| GgmlType::Q4K);
            let lane = compile(&spec, &path, &e4m3(numerics, 64, 128)).unwrap_or_else(|e| panic!("{} wide {numerics:?}: {e}", t.name()));
            programs.push((format!("cache-e4m3-{}-wide-{numerics:?}", t.name()), lane.program.cu.clone()));
        }
    }
    let (spec, path) = checkpoint(Tiny::Gemma2, "cache-e4m3-flash");
    let lane = compile(&spec, &path, &e4m3(Q8MatKind::TwoPassInt8, 32, 64)).unwrap_or_else(|e| panic!("gemma2 flash: {e}"));
    assert!(lane.program.cu.contains("lk_attention_flash<"), "Gemma 2's narrow heads take the flash tiles");
    programs.push(("cache-e4m3-gemma2-flash".to_string(), lane.program.cu.clone()));
    if common::require_tool("nvcc") {
        for batch in programs.chunks(6) {
            std::thread::scope(|scope| {
                for (name, cu) in batch {
                    scope.spawn(move || nvcc(cu, &dir().join(name)));
                }
            });
        }
    }
}

/// A decode step split over chips (tensor parallelism, PARITY-ALLREDUCE-NORM), compiled for one device: Qwen3 and Llama
/// four times as wide over two and four ranks, under every numerics — every chip's slice of a projection read in place
/// from the checkpoint (a run of output rows, or a run of whole blocks of every row), every all-reduce of the attention's
/// and the FFN's partials folded into the add and norm its sum feeds (`allreduce_norm`): one norm launch a sum, reading
/// every rank's partial, each written by its own product into its own array, and no launch adding partials apart.
/// Held back, the all-reduce adds its partials a launch an add.
#[test]
fn a_tensor_parallel_step_folds_each_all_reduce_into_its_norm() {
    for t in [Tiny::Qwen3, Tiny::Llama] {
        let (spec, path) = wide_checkpoint(t, "tp", &|_| GgmlType::Q8_0);
        for tp in [2, 4] {
            for numerics in NUMERICS {
                let what = format!("{} wide over {tp} {numerics:?}", t.name());
                let lane = compile_split(&spec, &path, &options(numerics), tp).unwrap_or_else(|e| panic!("{what}: {e}"));
                assert_eq!(lane.forms.len(), 1, "{what}: the decode step alone");
                let s = &lane.decode().schedule;
                let sums = 2 * spec.layers.len();
                assert_eq!(s.fired("allreduce_norm"), sums, "{what}: every all-reduce rides its norm");
                let folding: Vec<&Launch> = s.launches.iter().filter(|l| matches!(l, Launch::Norm { fold, .. } if !fold.is_empty())).collect();
                assert_eq!(folding.len(), sums, "{what}: one norm launch an all-reduce: {folding:?}");
                for (i, l) in folding.iter().enumerate() {
                    let Launch::Norm { fold, add, sum, .. } = l else { unreachable!() };
                    assert_eq!(fold.len(), tp, "{what}: every rank's partial: {l:?}");
                    assert!(add.is_none(), "{what}: the partials' fold is the added operand: {l:?}");
                    assert_eq!(sum.is_some(), i + 1 < sums, "{what}: every sum but the last is the residual the next layer reads, written on; the last feeds the output norm alone: {l:?}");
                    let distinct: BTreeSet<&String> = fold.iter().collect();
                    assert_eq!(distinct.len(), tp, "{what}: each rank's partial its own array: {fold:?}");
                    for p in fold {
                        let w = writer(s, p);
                        assert!(matches!(w, Launch::Gemv { .. }), "{what}: `{p}` is its rank's product: {w:?}");
                    }
                }
                let partials: BTreeSet<&str> = folding.iter().flat_map(|l| match l {
                    Launch::Norm { fold, .. } => fold.iter().map(String::as_str).collect::<Vec<_>>(),
                    _ => Vec::new(),
                }).collect();
                let apart: Vec<&Launch> = s.launches.iter().filter(|l| matches!(l, Launch::Binary { a, b, .. } if partials.contains(a.as_str()) || partials.contains(b.as_str()))).collect();
                assert!(apart.is_empty(), "{what}: no launch adds partials apart: {apart:?}");
                let held = compile_split_holding(&spec, &path, &options(numerics), tp, &["allreduce_norm"]).unwrap_or_else(|e| panic!("{what} held: {e}"));
                let hs = &held.decode().schedule;
                assert_eq!(hs.fired("allreduce_norm"), 0, "{what}: held, it fires nowhere");
                let adds = hs.launches.iter().filter(|l| matches!(l, Launch::Binary { op: ElementwiseOp::Add, .. })).count();
                assert_eq!(adds, sums * (tp - 1), "{what}: held, each all-reduce adds its partials a launch an add");
                let mut constants = BTreeSet::new();
                for f in &lane.forms {
                    for l in &f.schedule.launches {
                        if let Launch::Gemv { w, .. } = l {
                            constants.insert(w.clone());
                        }
                    }
                }
                assert!(constants.iter().any(|w| w.contains("_c")), "{what}: the ranks' slices are read in place: {constants:?}");
            }
        }
    }
}

/// A decode step split over chips whose sublayers post-norm their outputs (Gemma 3, PARITY-ALLREDUCE-NORM), four times as
/// wide over two and four ranks under every numerics: each sublayer's all-reduce folds into its post-norm, two a layer —
/// one norm launch reading its last partial onto the left fold of the others, each written by its own product, no
/// residual added in it and no sum written. Held back, the fold's adds before its last run a launch an add, the last in
/// the post-norm's add and norm.
#[test]
fn a_tensor_parallel_post_norm_folds_its_all_reduce_into_the_norm() {
    let (spec, path) = wide_checkpoint(Tiny::Gemma3, "tp-post", &|_| GgmlType::Q8_0);
    for tp in [2, 4] {
        for numerics in NUMERICS {
            let what = format!("gemma3 wide over {tp} {numerics:?}");
            let lane = compile_split(&spec, &path, &options(numerics), tp).unwrap_or_else(|e| panic!("{what}: {e}"));
            let s = &lane.decode().schedule;
            let sums = 2 * spec.layers.len();
            assert_eq!(s.fired("allreduce_norm"), sums, "{what}: every all-reduce rides its post-norm");
            let folding: Vec<&Launch> = s.launches.iter().filter(|l| matches!(l, Launch::Norm { fold, .. } if !fold.is_empty())).collect();
            assert_eq!(folding.len(), sums, "{what}: one norm launch an all-reduce: {folding:?}");
            for l in &folding {
                let Launch::Norm { x, fold, add, sum, .. } = l else { unreachable!() };
                assert_eq!(fold.len(), tp - 1, "{what}: the other ranks' partials: {l:?}");
                assert!(add.is_none() && sum.is_none(), "{what}: the post-norm takes the sum alone and writes none: {l:?}");
                let partials: BTreeSet<&String> = fold.iter().chain([x]).collect();
                assert_eq!(partials.len(), tp, "{what}: each rank's partial its own array: {l:?}");
                for p in partials {
                    assert!(matches!(writer(s, p), Launch::Gemv { .. }), "{what}: `{p}` is its rank's product: {:?}", writer(s, p));
                }
            }
            let held = compile_split_holding(&spec, &path, &options(numerics), tp, &["allreduce_norm"]).unwrap_or_else(|e| panic!("{what} held: {e}"));
            let hs = &held.decode().schedule;
            assert_eq!(hs.fired("allreduce_norm"), 0, "{what}: held, it fires nowhere");
            let adds = hs.launches.iter().filter(|l| matches!(l, Launch::Binary { op: ElementwiseOp::Add, .. })).count();
            assert_eq!(adds, sums * (tp - 2), "{what}: held, the fold's adds before its last run a launch an add");
        }
    }
}

/// The spec of `t` four times as wide with MiniMax-M2's QK norm — across every head at once — and, where given, its
/// query and KV heads.
fn whole_qk_norm(t: Tiny, heads: Option<(usize, usize)>) -> ModelSpec {
    let mut spec = t.wide(CTX);
    for layer in &mut spec.layers {
        if let Mixer::Attention(a) = &mut layer.mixer {
            a.qk_norm_whole = true;
            if let Some((h, kv)) = heads {
                (a.heads, a.kv_heads) = (h, kv);
            }
        }
    }
    spec
}

/// A decode step split over chips whose QK norm spans every head (MiniMax-M2's attention, PARITY-ALLREDUCE-NORM), on
/// Qwen3's layout four times as wide over two chips and, with four KV heads, over four, under every numerics: each
/// layer's queries and keys normed by their chips' parts where they live (`allreduce_sumsq_norm`) — one norm launch a
/// side, reading every chip's raw projection in the array its product wrote and writing every chip's normed heads in its
/// own array, its only scratch one sum of squares a part — and no join of the chips' rows launched. Held back, the norm
/// sums the joined row by the same parts, each read at its place in the row. Both programs build.
#[test]
fn a_tensor_parallel_whole_qk_norm_sends_one_sum_of_squares_a_chip() {
    let mut programs = Vec::new();
    for (spec, tp, name) in [(whole_qk_norm(Tiny::Qwen3, None), 2, "qwen3-whole"), (whole_qk_norm(Tiny::Qwen3, Some((8, 4))), 4, "qwen3-whole-kv4")] {
        let (spec, path) = sized_checkpoint(spec, name, &|_| GgmlType::Q8_0);
        let Mixer::Attention(a) = &spec.layers[0].mixer else { unreachable!("Qwen3 attends") };
        let widths = [a.heads / tp * a.head_dim, a.kv_heads / tp * a.head_dim];
        for numerics in NUMERICS {
            let what = format!("{name} over {tp} {numerics:?}");
            let lane = compile_split(&spec, &path, &options(numerics), tp).unwrap_or_else(|e| panic!("{what}: {e}"));
            let s = &lane.decode().schedule;
            let sides = 2 * spec.layers.len();
            assert_eq!(s.fired("allreduce_sumsq_norm"), sides, "{what}: every layer's queries and keys are normed by their chips' parts");
            let normed: Vec<&Launch> = s.launches.iter().filter(|l| matches!(l, Launch::NormParts { .. })).collect();
            assert_eq!(normed.len(), sides, "{what}: one norm launch a side: {normed:?}");
            for (i, l) in normed.iter().enumerate() {
                let Launch::NormParts { parts, normed: out, rows, cols, .. } = l else { unreachable!() };
                let (layer, (raw, taken)) = (i / 2, if i % 2 == 0 { ("qr", "qm") } else { ("kr", "km") });
                let want: Vec<RowPart> = (0..tp).map(|c| RowPart { x: format!("{raw}{layer}_c{c}"), part: None, cols: widths[i % 2] }).collect();
                assert_eq!(parts, &want, "{what}: every chip's raw projection read where its product wrote it: {l:?}");
                let back: Vec<NormedPart> = (0..tp).map(|c| NormedPart { part: c, y: Dst::to(&format!("{taken}{layer}_c{c}")) }).collect();
                assert_eq!(out, &back, "{what}: every chip's normed heads written in its own array: {l:?}");
                assert_eq!((*rows, *cols), (1, tp * widths[i % 2]), "{what}: the step's one row of every chip's heads: {l:?}");
                for p in parts {
                    assert!(matches!(writer(s, &p.x), Launch::Gemv { .. }), "{what}: `{}` is its chip's product: {:?}", p.x, writer(s, &p.x));
                }
                let sums: Vec<usize> = scratch(l, numerics).into_iter().filter(|sc| sc.tag == "ss").map(|sc| sc.bytes).collect();
                let acc = if numerics == Q8MatKind::Dequant { 8 } else { 4 };
                assert_eq!(sums, [tp * acc], "{what}: the launch's one scratch, a sum of squares a chip: {l:?}");
            }
            let joins: Vec<&Launch> = s.launches.iter().filter(|l| matches!(l, Launch::Join { .. })).collect();
            assert!(joins.is_empty(), "{what}: no chip's row is joined to another's: {joins:?}");
            assert!(lane.program.cu.contains("lk_sumsq<") && lane.program.cu.contains("lk_norm_by_sums<"), "{what}: the norm by parts' two kernels");
            programs.push((format!("whole-qk-{name}-{numerics:?}"), lane.program.cu.clone()));
            let held = compile_split_holding(&spec, &path, &options(numerics), tp, &["allreduce_sumsq_norm"]).unwrap_or_else(|e| panic!("{what} held: {e}"));
            let hs = &held.decode().schedule;
            assert_eq!(hs.fired("allreduce_sumsq_norm"), 0, "{what}: held, it fires nowhere");
            let apart: Vec<&Launch> = hs.launches.iter().filter(|l| matches!(l, Launch::NormParts { .. })).collect();
            assert_eq!(apart.len(), sides, "{what}: held, each side's norm still sums its row by its parts: {apart:?}");
            for (i, l) in apart.iter().enumerate() {
                let Launch::NormParts { parts, normed: out, cols, .. } = l else { unreachable!() };
                let row: BTreeSet<&String> = parts.iter().map(|p| &p.x).collect();
                assert_eq!(row.len(), 1, "{what}: held, every part read in the one joined row: {l:?}");
                let places: Vec<(Option<Part>, usize)> = parts.iter().map(|p| (p.part, p.cols)).collect();
                let want: Vec<(Option<Part>, usize)> = (0..tp).map(|c| (Some(Part { pitch: *cols, offset: c * widths[i % 2] }), widths[i % 2])).collect();
                assert_eq!(places, want, "{what}: held, each part at its place in the row: {l:?}");
                assert!(out.iter().all(|o| o.y.unit == Some(Part { pitch: *cols, offset: o.part * widths[i % 2] })), "{what}: held, each part normed into its place: {l:?}");
            }
            programs.push((format!("whole-qk-{name}-{numerics:?}-held"), held.program.cu.clone()));
        }
    }
    if common::require_tool("nvcc") {
        for batch in programs.chunks(6) {
            std::thread::scope(|scope| {
                for (name, cu) in batch {
                    scope.spawn(move || nvcc(cu, &dir().join(name)));
                }
            });
        }
    }
}

/// Llama with its first two layers attending chunks of four (Llama 4's chunked-local attention), the third every cached
/// row: a mask inside every attention kernel (PARITY-ATTENTION-VARIANTS-INLINE) — each chunked layer's attention keeps
/// its own chunk (`chunked`, a window of 4), the global layer every row — at its own width in the decode step and chunks
/// of 32, and four times as wide in chunks of 64, under every numerics; each launch line passes the window negative, the
/// way the kernels tell a chunk from a sliding window.
#[test]
fn a_chunked_local_attention_keeps_its_own_chunk_inside_its_kernel() {
    use logicaffeine_compile::tensor_recognize::qwen::Window;
    let chunked = |mut spec: ModelSpec| {
        for layer in spec.layers.iter_mut().take(2) {
            if let Mixer::Attention(a) = &mut layer.mixer {
                a.window = Some(Window::Chunked(4));
            }
        }
        spec
    };
    let cases = [("llama", chunked(Tiny::Llama.spec(64)), 64, 32), ("llama wide", chunked(Tiny::Llama.wide(128)), 128, 64)];
    let mut kernels: BTreeSet<String> = BTreeSet::new();
    for (name, spec, ctx, prefill) in &cases {
        let path = dir().join(format!("{}-chunked.gguf", name.replace(' ', "-")));
        tiny_models::write_checkpoint(spec, &path);
        for numerics in NUMERICS {
            let what = format!("{name} {numerics:?}");
            let lane = compile(spec, &path, &LaneOptions { ctx: *ctx, prefill: *prefill, ..options(numerics) }).unwrap_or_else(|e| panic!("{what}: {e}"));
            for f in &lane.forms {
                let attention: Vec<(Option<usize>, bool)> =
                    f.schedule.launches.iter().filter_map(|l| if let Launch::Attention { window, chunked, .. } = l { Some((*window, *chunked)) } else { None }).collect();
                assert_eq!(attention, [(Some(4), true), (Some(4), true), (None, false)], "{what} rows {}: the first two layers keep their own chunk", f.rows);
            }
            let lines: Vec<&str> =
                lane.program.cu.lines().map(str::trim).filter(|l| l.starts_with("lk_attention") && l.contains("<<<") && !l.starts_with("lk_attention_fold")).collect();
            for line in &lines {
                kernels.insert(line[..line.find('<').unwrap_or(line.len())].to_string());
            }
            let negative = lines.iter().filter(|l| l.contains(", -4, ")).count();
            assert_eq!(negative, 2 * lane.forms.len(), "{what}: each form's two chunked layers pass their chunk negative");
        }
    }
    for k in ["lk_attention", "lk_attention_mma", "lk_attention_i8", "lk_attention_flash"] {
        assert!(kernels.contains(k), "a chunked-local attention runs on `{k}`: {kernels:?}");
    }
}

/// Every chunked program runs a chunk before the prompt's last lean: on every tiny architecture, under every numerics,
/// each prefill form whose launches include some only its output reads captures a second graph of its step without
/// them (`exec…_lean`), its arg-max an advance of the position and the run (`lk_advance`); the program counts the lean
/// chunks it ran (`LEAN_CHUNKS`) and runs every chunk whole under `--whole-chunks`.
#[test]
fn every_chunked_program_runs_a_chunk_before_the_prompts_last_lean() {
    for &t in ALL {
        for numerics in NUMERICS {
            let what = format!("{} {numerics:?}", t.name());
            let lane = compiled_chunks(t, numerics, 8, &format!("lean-text-{numerics:?}"));
            let cu = &lane.program.cu;
            for f in lane.forms.iter().filter(|f| f.kind == FormKind::Prefill) {
                assert!(cu.contains(&format!("cudaGraphExec_t exec{}_lean", f.tag())), "{what}: the chunk of {} captures its lean graph", f.rows);
                assert!(cu.contains(&format!(">>>(d_pos, {}, d_gen, d_count);", f.rows)), "{what}: the chunk of {} advances without picking", f.rows);
            }
            assert!(cu.contains("__global__ void lk_advance(") && cu.contains(" else lk_advance<<<1, 1, 0, S["), "{what}: the advance's kernel, the lean step's arg-max");
            assert!(cu.contains("\"--whole-chunks\"") && cu.contains("LEAN_CHUNKS: %d"), "{what}: the flag and the count");
        }
    }
}

/// A prefill chunk's launches that compute only what its output reads (`emit::output_only`) are what liveness from the
/// steps' carried tensors leaves: on every tiny architecture, chunked, under every numerics, every attention's caches
/// and every recurrent memory are carried (`emit::carried`) and a mixture's sum, made anew each step, is not; no
/// output-only launch writes a carried tensor (through any alias of one) but the arg-max, which a chunk before the
/// prompt's last runs as its advance; every other launch writes one, or a tensor a later kept launch reads; the head's
/// product is among them, and so is the last layer's attention where that layer attends; the decode step has none.
#[test]
fn a_chunks_output_only_launches_are_what_liveness_from_its_carried_tensors_leaves() {
    use logicaffeine_compile::codegen_cuda::lane::emit::{alias_root, carried, output_only};
    for &t in ALL {
        let last_attends = matches!(t.spec(CTX).layers.last().map(|l| &l.mixer), Some(Mixer::Attention(_)));
        for numerics in NUMERICS {
            let what = format!("{} {numerics:?}", t.name());
            let lane = compiled_chunks(t, numerics, 8, &format!("lean-live-{numerics:?}"));
            for f in &lane.forms {
                for l in &f.schedule.launches {
                    match l {
                        Launch::Attention { kc, vc, .. } => assert!(carried(&lane.forms, kc) && carried(&lane.forms, vc), "{what}: an attention's caches are carried"),
                        Launch::Delta { st, .. } => assert!(carried(&lane.forms, st), "{what}: a recurrent memory is carried"),
                        Launch::Moe { y, .. } if y.own().is_some() => assert!(!carried(&lane.forms, &y.buf), "{what}: a mixture's sum is made anew each step"),
                        _ => {}
                    }
                }
                let lean = output_only(f, &lane.forms);
                if f.kind != FormKind::Prefill {
                    assert!(lean.iter().all(|&x| !x), "{what}: the step of {} computes nothing only its output reads", f.rows);
                    continue;
                }
                let root = |t: &str| alias_root(&f.schedule.buffers, t).to_string();
                let carried = |t: &str| carried(&lane.forms, t);
                let ls = &f.schedule.launches;
                for (i, l) in ls.iter().enumerate() {
                    let writes = l.writes();
                    if lean[i] {
                        assert!(
                            matches!(l, Launch::ArgMax { .. }) || !writes.iter().any(|&w| carried(w)),
                            "{what} chunk of {}: launch {i} ({}) writes what the step carries but runs only for the output",
                            f.rows,
                            kind(l)
                        );
                    } else {
                        let read_later = writes
                            .iter()
                            .any(|&w| ls[i + 1..].iter().zip(&lean[i + 1..]).any(|(m, &out)| !out && m.reads().iter().any(|&r| root(r) == root(w))));
                        assert!(
                            writes.is_empty() || writes.iter().any(|&w| carried(w)) || read_later,
                            "{what} chunk of {}: launch {i} ({}) is kept, but it carries nothing and nothing kept reads it",
                            f.rows,
                            kind(l)
                        );
                    }
                }
                let argmax = ls.iter().position(|l| matches!(l, Launch::ArgMax { .. })).expect("a chunk picks a token");
                assert!(lean[argmax], "{what} chunk of {}: the arg-max is the output's", f.rows);
                let logits = root(ls[argmax].reads()[0]);
                let head = ls.iter().rposition(|l| l.writes().iter().any(|&w| root(w) == logits)).expect("the head's product");
                assert!(lean[head], "{what} chunk of {}: the head's product ({}) is the output's", f.rows, kind(&ls[head]));
                if last_attends {
                    let attention = ls.iter().rposition(|l| matches!(l, Launch::Attention { .. })).expect("the last layer's attention");
                    assert!(lean[attention], "{what} chunk of {}: the last layer's attention is the output's", f.rows);
                }
            }
        }
    }
}
