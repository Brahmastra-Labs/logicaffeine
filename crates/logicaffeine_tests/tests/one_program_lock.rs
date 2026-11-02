//! ONE LOGICAL REPRESENTATION. A model is written once, as the LOGOS program `model::author` writes from its spec, and
//! every backend lowers a cut of that program and no other (`model::program`): the CUDA lane compiles its decode step
//! and prefill chunks, the D5005 engine reads its decode step off the same step graph. Every artifact records the
//! identity of the cut it was lowered from — the cut and the SHA-256 of the text — so an artifact names the exact
//! program behind it.
//!
//! These locks hold that: each lane form and each engine program carries the identity of the text the author writes
//! for its cut, recomputed here; the lane's decode step and the engine's program are one program; the engine's program
//! directory carries the text it was read off; a spatial chip's bundle names the cut of every program it lowered, and
//! those cuts — embedding, every layer, head — are the decode step cut apart, op for op; the configuration the spatial
//! lowering is written from writes the checkpoint's own program; and two censuses keep the set of places that write a
//! model's program, and of hand-written lanes, from growing.

#![cfg(feature = "codegen")]

mod common;
#[path = "common/tiny_models.rs"]
mod tiny_models;

use std::path::{Path, PathBuf};

use logicaffeine_compile::compile::RduModel;
use logicaffeine_compile::codegen_cuda::knobs::CudaKnobs;
use logicaffeine_compile::codegen_cuda::lane::graph::StepGraph;
use logicaffeine_compile::codegen_cuda::lane::{compile, Lane, LaneOptions};
use logicaffeine_compile::codegen_cuda::provenance::{header, Provenance};
use logicaffeine_compile::codegen_cuda::qwen38::{qwen38_rope_cache, Qwen38Cfg};
use logicaffeine_compile::codegen_cuda::qwen_emit::Q8MatKind;
use logicaffeine_compile::compile::{compile_model_to_rdu_streamed, StepShape};
use logicaffeine_compile::model::bind;
use logicaffeine_compile::model::gguf::Gguf;
use logicaffeine_compile::model::program::{decode_at, prefill_at, written, written_split, Cut, Identity};
use logicaffeine_compile::model::spec::{Mixer, ModelSpec};
use logicaffeine_compile::spatial::d5005::Beat;
use logicaffeine_compile::spatial::decode::GgufHeader;
use logicaffeine_compile::spatial::decode_engine::image::ImageLayout;
use logicaffeine_compile::spatial::decode_engine::machine::Machine;
use logicaffeine_compile::spatial::decode_engine::model::EngineNumerics;
use logicaffeine_compile::spatial::decode_engine::ofs::write_engine_program;
use logicaffeine_compile::spatial::decode_engine::physical::PhysicalMap;
use logicaffeine_compile::spatial::decode_engine::program::lower;
use logicaffeine_compile::spatial::passes::recognize_stages::recognize_stages;
use logicaffeine_compile::spatial::rdu_chip::RduChip;
use tiny_models::Tiny;

const CTX: usize = 24;

/// The tiny models the lane compiles, each with its checkpoint written into `dir`.
fn lane_models(dir: &Path) -> Vec<(Tiny, ModelSpec, PathBuf)> {
    [Tiny::Qwen35, Tiny::Qwen3, Tiny::Llama, Tiny::LlamaTied, Tiny::Gemma3, Tiny::Qwen3Moe, Tiny::GptOss, Tiny::DeepSeek2]
        .into_iter()
        .map(|t| {
            let spec = t.spec(CTX);
            let path = dir.join(format!("{}.gguf", t.name()));
            tiny_models::write_checkpoint(&spec, &path);
            (t, spec, path)
        })
        .collect()
}

fn lane(spec: &ModelSpec, path: &Path, prefill: usize, ctx: usize) -> Lane {
    let opts = LaneOptions { ctx, knobs: CudaKnobs { activations: Q8MatKind::Int8Act, ..CudaKnobs::default() }, prefill };
    compile(spec, path, &opts).unwrap_or_else(|e| panic!("{}: {e}", spec.arch))
}

/// The tiny Gated-DeltaNet hybrid both the lane and the engine run, its attention heads as the engine's units take them.
fn engine_spec() -> ModelSpec {
    let mut spec = Tiny::Qwen35.wide(64);
    for layer in &mut spec.layers {
        if let Mixer::Attention(a) = &mut layer.mixer {
            a.heads = 4;
            a.kv_heads = 2;
        }
    }
    spec
}

#[test]
fn every_lane_form_names_the_cut_of_the_one_program_it_compiled() {
    let dir = tempfile::tempdir().unwrap();
    for (t, spec, path) in lane_models(dir.path()) {
        let l = lane(&spec, &path, 4, CTX);
        assert_eq!(l.forms.len(), 3, "{}: a decode step and chunks of 4 and 2 rows", t.name());
        for f in &l.forms {
            let cut = if f.rows == 1 { Cut::Decode { ctx: CTX } } else { Cut::Prefill { rows: f.rows, ctx: CTX } };
            let ((text, _, _), identity) = written(&spec, cut);
            assert_eq!(f.identity, identity, "{}: the form of {} rows", t.name(), f.rows);
            assert_eq!(f.source, text, "{}: the form of {} rows compiles the author's text for its cut", t.name(), f.rows);
            let line = format!("// {}\n", identity.line());
            assert_eq!(l.program.cu.matches(&line).count(), 1, "{}: the program prints {cut}'s identity once", t.name());
            let digest = identity.line().rsplit("sha256 ").next().unwrap().to_string();
            assert!(digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit()), "{}: a whole SHA-256: {digest}", t.name());
        }
        assert!(l.program.cu.starts_with(&header(Provenance::Compiled)), "{}: the lane is compiled, not hand-written", t.name());
    }
}

#[test]
fn a_cut_is_the_authors_text_with_every_input_at_its_placeholder() {
    use logicaffeine_compile::model::author;
    for t in [Tiny::Qwen35, Tiny::Llama, Tiny::Gemma3] {
        let spec = t.spec(CTX);
        assert_eq!(written(&spec, Cut::Decode { ctx: CTX }).0, author::decode_step(&spec, "", CTX, 1, 1), "{}", t.name());
        assert_eq!(written(&spec, Cut::Prefill { rows: 4, ctx: CTX }).0, author::prefill(&spec, "", CTX, &[1, 1, 1, 1], 1), "{}", t.name());
        assert_eq!(written(&spec, Cut::DecodeLayer { layer: 0, ctx: CTX }).0, author::decode_layer(&spec, 0, "", CTX, 1), "{}", t.name());
    }
}

#[test]
fn the_engine_program_is_read_off_the_one_program() {
    let spec = engine_spec();
    for ctx in [1, CTX, 64] {
        for n in EngineNumerics::all() {
            let lowered = lower(&spec, n, ctx).unwrap_or_else(|e| panic!("ctx {ctx}, {n:?}: {e}"));
            assert_eq!(lowered.identity, written(&spec, Cut::Decode { ctx }).1, "ctx {ctx}, {n:?}");
            assert_eq!(
                lowered.program,
                lower(&spec, n, 64).unwrap().program,
                "ctx {ctx}, {n:?}: the commands do not depend on the cache's length"
            );
        }
    }
}

#[test]
fn two_models_of_one_architecture_lower_to_their_own_programs() {
    let a = engine_spec();
    let mut b = engine_spec();
    b.vocab += 32;
    assert_eq!(a.arch, b.arch);
    let n = EngineNumerics::fastest();
    let (la, lb) = (lower(&a, n, 64).unwrap(), lower(&b, n, 64).unwrap());
    assert_eq!(la.identity, written(&a, Cut::Decode { ctx: 64 }).1, "the first model's own program");
    assert_eq!(lb.identity, written(&b, Cut::Decode { ctx: 64 }).1, "the second model's own program, not the first's");
    assert_ne!(la.identity, lb.identity);
    assert_eq!(lower(&a, n, 64).unwrap().identity, la.identity, "and the first again, after the second");
}

#[test]
fn the_lane_and_the_engine_lower_one_program() {
    let dir = tempfile::tempdir().unwrap();
    let spec = engine_spec();
    let path = dir.path().join("hybrid.gguf");
    tiny_models::write_checkpoint(&spec, &path);
    let l = lane(&spec, &path, 1, spec.ctx_len);
    let machine = Machine::new(spec.clone(), Gguf::open(&path).unwrap(), EngineNumerics::fastest(), 1, spec.ctx_len).unwrap();
    assert_eq!(l.decode().identity, machine.identity, "the lane's decode step and the engine's step are one program");
    assert_eq!(machine.identity.cut, Cut::Decode { ctx: spec.ctx_len });
}

#[test]
fn an_engine_program_directory_carries_the_text_it_was_read_off() {
    let dir = tempfile::tempdir().unwrap();
    let spec = engine_spec();
    let path = dir.path().join("hybrid.gguf");
    tiny_models::write_checkpoint(&spec, &path);
    let machine = Machine::new(spec.clone(), Gguf::open(&path).unwrap(), EngineNumerics::fastest(), 1, spec.ctx_len).unwrap();
    let map = PhysicalMap::flat(&machine.layout, &ImageLayout::d5005(Beat::Data64), 512).unwrap();
    let out = dir.path().join("program");
    write_engine_program(&machine, &map, &out).unwrap();
    let ((text, _, _), identity) = written(&spec, Cut::Decode { ctx: spec.ctx_len });
    let on_disk = std::fs::read_to_string(out.join("program.lg")).unwrap();
    assert_eq!(on_disk, text, "program.lg is the author's text");
    assert_eq!(Identity::of(identity.cut, &on_disk), machine.identity, "and it hashes to the machine's identity");
    assert_eq!(std::fs::read_to_string(out.join("program.txt")).unwrap(), format!("{}\n", identity.line()));
}

/// The spec the checkpoint's header ledger describes.
fn header_spec(ledger: &str) -> ModelSpec {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../../benchmarks/silicon/decode/headers/{ledger}.json"));
    GgufHeader::read_ledger(&path).unwrap_or_else(|e| panic!("{ledger}: {e}")).spec().unwrap_or_else(|e| panic!("{ledger}: {e:?}"))
}

#[test]
fn the_spatial_configuration_writes_the_checkpoints_own_program() {
    let cfg = Qwen38Cfg::real();
    for ledger in ["qwen3.8-27b-q4_k_m", "qwen3.8-27b-q8_0"] {
        let spec = header_spec(ledger);
        let ctx = 4096;
        for cut in [
            Cut::Decode { ctx },
            Cut::Prefill { rows: 64, ctx },
            Cut::Batch { rows: 8, ctx },
            Cut::Forward,
            Cut::ForwardHead,
            Cut::DecodeEmbed,
            Cut::DecodeHead,
            Cut::BatchEmbed { rows: 8 },
            Cut::BatchHead { rows: 8 },
            Cut::PrefillHead { rows: 64 },
        ] {
            assert_eq!(written(&cfg.spec(), cut).1, written(&spec, cut).1, "{ledger}: {cut}");
        }
        for l in 0..spec.layers.len() {
            for cut in [
                Cut::ForwardLayer { layer: l },
                Cut::DecodeLayer { layer: l, ctx },
                Cut::BatchLayer { layer: l, rows: 8, ctx },
                Cut::PrefillLayer { layer: l, rows: 64, ctx },
            ] {
                assert_eq!(written(&cfg.layer_spec(l), cut).1, written(&spec, cut).1, "{ledger}: {cut}");
            }
        }
        for tp in [2, 4, 8] {
            for cut in [Cut::Decode { ctx }, Cut::Batch { rows: 8, ctx }, Cut::DecodeLayer { layer: 3, ctx }] {
                let (a, b) = (written_split(&cfg.spec(), cut, "", tp).unwrap(), written_split(&spec, cut, "", tp).unwrap());
                assert_eq!(a.1, b.1, "{ledger}: {cut} over {tp} chips");
            }
        }
        let stepped = |mut s: ModelSpec| {
            for layer in &mut s.layers {
                if let Mixer::Attention(a) = &mut layer.mixer {
                    if let Some(r) = &mut a.rope {
                        r.sections = None;
                    }
                }
            }
            s.mtp_blocks = 0;
            s
        };
        assert_eq!(
            stepped(cfg.spec()),
            stepped(spec.clone()),
            "{ledger}: the configuration is the checkpoint's spec but for the rope's sections, which a text token reads as \
             its position, and the draft blocks, which the decode step does not run"
        );
        let rope = bind::rotations(&spec)[0].clone();
        let (cos, sin) = bind::rope_tables(&rope, 64, None);
        let (rcos, rsin) = qwen38_rope_cache(&cfg, 64);
        assert_eq!(rcos, cos.iter().map(|&x| x as f32).collect::<Vec<_>>(), "{ledger}: the bundle's cosines are the binding's");
        assert_eq!(rsin, sin.iter().map(|&x| x as f32).collect::<Vec<_>>(), "{ledger}: the bundle's sines are the binding's");
    }
}

/// What a program's ops compute, in order: each op's kind, its result, the shapes it reads and the shape it writes —
/// every name but an op's inputs', which a cut renames at its edges.
fn ops_of(source: &str) -> Vec<(&'static str, String, Vec<(usize, usize)>, (usize, usize))> {
    let graph = StepGraph::build(&recognize_stages(source).unwrap_or_else(|e| panic!("{e:?}"))).unwrap_or_else(|e| panic!("{e}"));
    let shape = |n: &str| graph.tensors.get(n).map_or((0, 0), |t| (t.rows, t.cols));
    graph.ops.iter().map(|o| (o.spec.kind(), o.out.clone(), o.ins.iter().map(|i| shape(i)).collect(), shape(&o.out))).collect()
}

#[test]
fn a_spatial_chips_programs_are_the_decode_step_cut_apart() {
    for (what, spec) in [("the tiny configuration", Qwen38Cfg::tiny().spec()), ("the 27B header", header_spec("qwen3.8-27b-q4_k_m"))] {
        let ctx = 64;
        let whole = ops_of(&written(&spec, Cut::Decode { ctx }).0 .0);
        let mut parts = ops_of(&written(&spec, Cut::DecodeEmbed).0 .0);
        for l in 0..spec.layers.len() {
            parts.extend(ops_of(&written(&spec, Cut::DecodeLayer { layer: l, ctx }).0 .0));
        }
        parts.extend(ops_of(&written(&spec, Cut::DecodeHead).0 .0));
        assert_eq!(parts.len(), whole.len(), "{what}: the embedding, every layer and the head are the step's ops");
        for (i, (a, b)) in parts.iter().zip(&whole).enumerate() {
            assert_eq!(a, b, "{what}: op {i}");
        }
    }
}

#[test]
fn an_sn50_bundle_names_the_cut_of_every_program_it_lowered() {
    let cfg = Qwen38Cfg::tiny();
    let spec = cfg.spec();
    let ctx = 8;
    let bundle = compile_model_to_rdu_streamed(&RduModel::qwen38(&cfg), RduChip::Sn50, "weights", Some(StepShape { ctx, batch: 1, tp: 1, prefill: None }), |_| Ok(()))
        .unwrap_or_else(|e| panic!("{e}"));
    let m: serde_json::Value = serde_json::from_str(&bundle.manifest).unwrap();
    let whole = written(&spec, Cut::Decode { ctx }).1;
    assert_eq!(m["program"], whole.line(), "the bundle names the decode step it deploys");
    assert_eq!(Identity::of(Cut::Decode { ctx }, &bundle.program), whole, "and carries its text");
    assert_eq!(m["embed"]["program"], written(&spec, Cut::DecodeEmbed).1.line());
    assert_eq!(m["head"]["program"], written(&spec, Cut::DecodeHead).1.line());
    let layers = m["layers"].as_array().unwrap();
    assert_eq!(layers.len(), cfg.layers);
    for b in layers {
        let l = b["layer"].as_u64().unwrap() as usize;
        assert_eq!(b["program"], written(&cfg.layer_spec(l), Cut::DecodeLayer { layer: l, ctx }).1.line(), "layer {l}");
    }
    for k in m["kinds"].as_array().unwrap() {
        let rep = k["representative_layer"].as_u64().unwrap() as usize;
        assert_eq!(k["program"], written(&cfg.layer_spec(rep), Cut::DecodeLayer { layer: rep, ctx }).1.line(), "{}", k["name"]);
    }
}

/// Every Rust source under the compiler, its examples and the apps.
fn sources() -> Vec<(String, String)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut out = Vec::new();
    let mut stack: Vec<PathBuf> =
        ["crates/logicaffeine_compile/src", "crates/logicaffeine_compile/examples", "apps"].iter().map(|d| root.join(d)).collect();
    while let Some(d) = stack.pop() {
        for entry in std::fs::read_dir(&d).unwrap_or_else(|e| panic!("{}: {e}", d.display())) {
            let p = entry.unwrap().path();
            if p.is_dir() {
                if p.file_name().is_some_and(|n| n != "target" && n != "node_modules") {
                    stack.push(p);
                }
            } else if p.extension().is_some_and(|e| e == "rs") {
                let rel = p.strip_prefix(&root).unwrap().to_string_lossy().into_owned();
                out.push((rel, std::fs::read_to_string(&p).unwrap()));
            }
        }
    }
    out.sort();
    out
}

/// The author's writers: every function that writes a model's program, or a cut of it.
const WRITERS: [&str; 17] = [
    "forward",
    "decode_step",
    "prefill",
    "decode_batch",
    "layer",
    "decode_layer",
    "head",
    "decode_embed",
    "decode_head",
    "decode_batch_embed",
    "stacked_layer",
    "prefill_head",
    "decode_batch_head",
    "tp_decode_step",
    "tp_decode_batch",
    "tp_decode_layer",
    "tp_stacked_layer",
];

/// The files that call the author's writers, and why each may. The set may shrink and never grow: a backend reads a cut
/// through `model::program`, never by calling the author itself.
const WRITES_THE_PROGRAM: [(&str, &str); 2] = [
    ("crates/logicaffeine_compile/src/model/program.rs", "the cuts every backend lowers"),
    ("crates/logicaffeine_compile/src/codegen_cuda/qwen38.rs", "Qwen3.8's cuts as the spatial lowering reads them"),
];

/// Whether `text` calls one of the author's writers, by path or by a name it imports from the author.
fn calls_a_writer(text: &str) -> bool {
    let imported = |w: &str| {
        text.lines().filter(|l| l.trim_start().starts_with("use ")).filter_map(|l| l.split("author::").nth(1)).any(|list| {
            list.split(|c: char| !(c.is_alphanumeric() || c == '_')).any(|name| name == w)
        })
    };
    WRITERS.iter().any(|w| text.contains(&format!("author::{w}(")) || imported(w))
}

/// A run executes a cut at its own inputs (`prefill_at`, `decode_at`); at the placeholders those are exactly the cut
/// the program identifies, so what a run executes is the identified program with its inputs filled in, never another.
#[test]
fn a_run_writes_the_cut_the_program_identifies() {
    let spec = engine_spec();
    let ctx = 16;
    let (prefill, _) = written(&spec, Cut::Prefill { rows: 4, ctx });
    assert_eq!(prefill_at(&spec, "", ctx, &[1; 4], 1).0, prefill.0, "a prefill chunk at its placeholders is the cut");
    let (decode, _) = written(&spec, Cut::Decode { ctx });
    assert_eq!(decode_at(&spec, "", ctx, 1, 1).0, decode.0, "a decode step at its placeholders is the cut");
    assert_ne!(decode_at(&spec, "", ctx, 7, 5).0, decode.0, "a run's own inputs are written into its text");
}

#[test]
fn only_the_one_program_calls_the_authors_writers() {
    let found: Vec<String> = sources().into_iter().filter(|(_, text)| calls_a_writer(text)).map(|(f, _)| f).collect();
    let allowed: Vec<String> = WRITES_THE_PROGRAM.iter().map(|(f, _)| f.to_string()).collect();
    let strays: Vec<&String> = found.iter().filter(|f| !allowed.contains(f)).collect();
    assert!(strays.is_empty(), "a model's program written outside model::program: {strays:?}");
    let gone: Vec<&String> = allowed.iter().filter(|f| !found.contains(f)).collect();
    assert!(gone.is_empty(), "no longer calls the author, so drop it from WRITES_THE_PROGRAM: {gone:?}");
}

#[test]
fn the_writer_census_sees_a_call_and_an_import() {
    assert!(calls_a_writer("let p = author::decode_step(&spec, \"\", 8, 1, 1);"));
    assert!(calls_a_writer("use crate::model::author::{check, prefill};\nlet p = prefill(&s, \"\", 8, &[1], 1);"));
    assert!(!calls_a_writer("use crate::model::author::{check, NotYet};\nauthor::check(&spec)?;"));
    assert!(!calls_a_writer("use crate::model::author::{layer_input, stacked_layer_input};"));
}

/// The lanes written by hand for one model, each held to its model's program by golden-token locks rather than compiled
/// from it. The set may shrink and never grow; none of them produces a number the one program's backends are quoted by.
const HAND_WRITTEN: [&str; 4] = [
    "crates/logicaffeine_compile/src/codegen_cuda/qwen38.rs",
    "crates/logicaffeine_compile/src/codegen_cuda/qwen38_batched.rs",
    "crates/logicaffeine_compile/src/codegen_cuda/qwen_batched.rs",
    "crates/logicaffeine_compile/src/codegen_cuda/qwen_emit.rs",
];

#[test]
fn the_hand_written_lanes_are_a_closed_set() {
    let found: Vec<String> =
        sources().into_iter().filter(|(_, text)| text.contains("Provenance::HandWritten {")).map(|(f, _)| f).collect();
    let mut want: Vec<String> = HAND_WRITTEN.iter().map(|f| f.to_string()).collect();
    want.push("crates/logicaffeine_compile/src/codegen_cuda/provenance.rs".to_string());
    want.sort();
    assert_eq!(found, want, "a new hand-written lane, or one retired without leaving HAND_WRITTEN");
}

#[test]
fn the_hand_lowering_is_only_an_oracle() {
    let found: Vec<String> =
        sources().into_iter().filter(|(_, text)| text.contains("hand_lowering(")).map(|(f, _)| f).collect();
    assert_eq!(
        found,
        vec!["crates/logicaffeine_compile/src/spatial/decode_engine/program.rs".to_string()],
        "the engine is given the program read off the step graph; the hand lowering is the oracle its lock compares"
    );
}
