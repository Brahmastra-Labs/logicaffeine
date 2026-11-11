//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! EVERY CUDA KNOB EITHER CHANGES WHAT AN EMITTER PRINTS, OR IS LISTED AS ONE NOTHING READS YET.
//!
//! `CudaKnobs` documents its trade-offs, and `cuda_knobs_lock` holds their defaults and their documented wins and
//! losses. Neither checks that a knob does anything, and a knob nothing reads is a trade-off an autotuner would
//! search and never move.
//!
//! The two lists below partition the knobs. A knob in `HONORED` must change the program some emitter prints when
//! it is flipped from its default — the kernel emitters, and the compiled generation lane of every tiny architecture.
//! A knob in `NOT_YET_READ` must change nothing anywhere — so wiring one up turns this lock red until the knob moves
//! into `HONORED`, and the second list can only shrink.
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(feature = "codegen")]

#[path = "common/tiny_models.rs"]
mod tiny_models;

use std::path::PathBuf;
use std::sync::OnceLock;

use logicaffeine_compile::codegen_cuda::knobs::KnobTradeoff;
use logicaffeine_compile::codegen_cuda::lane::{compile, LaneOptions};
use logicaffeine_compile::codegen_cuda::qwen_emit::Q8MatKind;
use logicaffeine_compile::codegen_cuda::{
    emit_fused_row, emit_fused_row_solution, emit_swiglu, lower_matmul_to_cuda_with, CudaChip,
    CachePrecision, CudaKnobs, LoadWidth, Logits, MatmulPrecision, StatePrecision,
};
use logicaffeine_compile::intern::Interner;
use logicaffeine_compile::model::spec::ModelSpec;
use logicaffeine_compile::tensor_recognize::{
    Epsilon, FusedRowSpec, MatmulSpec, RowReduceKind, SwiGluSpec,
};

const HONORED: &[&str] = &["precision", "chip", "cuda_graphs", "fuse_quant", "activations", "recurrent_state", "cache_precision", "logits"];
const NOT_YET_READ: &[&str] = &["gemv_load", "kv_cache"];

/// Every tiny architecture with its checkpoint, written once per process.
fn tiny_checkpoints() -> &'static [(ModelSpec, PathBuf)] {
    static WRITTEN: OnceLock<Vec<(ModelSpec, PathBuf)>> = OnceLock::new();
    WRITTEN.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("logos_knob_consumers_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        tiny_models::ALL
            .iter()
            .map(|t| {
                let spec = t.spec(24);
                let path = dir.join(format!("{}.gguf", t.name()));
                tiny_models::write_checkpoint(&spec, &path);
                (spec, path)
            })
            .collect()
    })
}

/// Every program a knob-taking emitter prints for a fixed set of specs.
fn everything_printed(knobs: &CudaKnobs) -> Vec<String> {
    let mut i = Interner::new();
    let (a, b, c, x, y, w, g, u) = (
        i.intern("a"),
        i.intern("b"),
        i.intern("c"),
        i.intern("x"),
        i.intern("y"),
        i.intern("w"),
        i.intern("g"),
        i.intern("u"),
    );
    let matmul = MatmulSpec {
        m: 64,
        n: 64,
        k: 64,
        a,
        b,
        c,
    };
    let row = |reduce, weight| FusedRowSpec {
        rows: 4,
        cols: 256,
        reduce,
        x,
        y,
        weight,
        weight_rows: 1,
        residual: None,
        bias: None,
        eps: Epsilon::DEFAULT,
    };
    let norms = [
        row(RowReduceKind::RmsNorm, Some(w)),
        row(RowReduceKind::Softmax, None),
    ];
    let mut out = vec![lower_matmul_to_cuda_with(&matmul, knobs)];
    for n in &norms {
        out.push(emit_fused_row(n, knobs));
        let sol = emit_fused_row_solution(n, knobs, "def", &["x"], "y");
        out.push(format!("{sol:?}"));
    }
    out.push(emit_swiglu(
        &SwiGluSpec {
            total: 1024,
            gate: g,
            up: u,
            y,
        },
        knobs,
    ));
    for (spec, path) in tiny_checkpoints() {
        let lane = compile(spec, path, &LaneOptions { ctx: 24, knobs: *knobs, prefill: 4 })
            .unwrap_or_else(|e| panic!("the tiny lane compiles: {e}"));
        out.push(lane.program.cu);
    }
    out
}

/// The default with one knob moved to a non-default setting.
fn flipped(knob: &str) -> CudaKnobs {
    let mut k = CudaKnobs::default();
    match knob {
        "precision" => k.precision = MatmulPrecision::Int8Dp4a,
        "chip" => k.chip = CudaChip::b200(),
        "gemv_load" => k.gemv_load = LoadWidth::Char4,
        "kv_cache" => k.kv_cache = !k.kv_cache,
        "cuda_graphs" => k.cuda_graphs = !k.cuda_graphs,
        "fuse_quant" => k.fuse_quant = !k.fuse_quant,
        "activations" => k.activations = Q8MatKind::Int8Act,
        "recurrent_state" => k.recurrent_state = StatePrecision::Bf16,
        "cache_precision" => k.cache_precision = CachePrecision::F16,
        "logits" => k.logits = Logits::Unread,
        other => panic!("`{other}` is a knob this lock does not know how to flip — add it"),
    }
    assert_ne!(k, CudaKnobs::default(), "flipping `{knob}` must move it off its default");
    k
}

#[test]
fn the_two_lists_partition_the_documented_knobs() {
    let documented: Vec<&str> = CudaKnobs::tradeoffs().iter().map(|t: &KnobTradeoff| t.knob).collect();
    let mut listed: Vec<&str> = HONORED.iter().chain(NOT_YET_READ).copied().collect();
    let mut want = documented.clone();
    listed.sort_unstable();
    want.sort_unstable();
    assert_eq!(
        listed, want,
        "every documented knob is in exactly one list, and nothing else is"
    );
}

#[test]
fn every_honored_knob_changes_what_an_emitter_prints() {
    let base = everything_printed(&CudaKnobs::default());
    for knob in HONORED {
        let moved = everything_printed(&flipped(knob));
        assert_ne!(
            base, moved,
            "`{knob}` is listed as honored, but flipping it changes no emitted program"
        );
    }
}

/// The honesty of the second list: a knob nothing reads must leave every emitted program byte-identical. When one
/// gets wired, this fails and the knob moves to `HONORED`.
#[test]
fn every_knob_not_yet_read_changes_nothing() {
    let base = everything_printed(&CudaKnobs::default());
    for knob in NOT_YET_READ {
        assert_eq!(
            base,
            everything_printed(&flipped(knob)),
            "`{knob}` now changes an emitted program — move it from NOT_YET_READ to HONORED"
        );
    }
}

/// The list of unread knobs only shrinks.
#[test]
fn the_unread_knobs_never_grow() {
    assert!(
        NOT_YET_READ.len() <= 2,
        "a new knob must be honored by an emitter when it is added, not parked here"
    );
}
