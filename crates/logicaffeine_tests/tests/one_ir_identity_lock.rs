//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! EVERY ARTIFACT THE ONE-IR MIGRATION COULD MOVE, PINNED BY DIGEST.
//!
//! The book's critical path (benchmarks/FUSIONS_WE_NEED.md, IR-0 to MIGRATE-4) moves four representations onto one
//! IR: the CUDA lane's step graph and rule table, the older CUDA forward's stage list, the spatial pipeline's tensor
//! ops, and the D5005 engine's block reader. Each move is a refactor: no program any of them prints may change by a
//! byte. These pins are taken on the tree before the move, one family to a fixture under
//! `tests/fixtures/one_ir_identity/` (the lane's, `lane.txt`, are `one_ir_lane_identity_lock`'s):
//!
//!   • `engine.txt` — the engine's encoded words at every knob point for the tiny models whose blocks it runs and the
//!     27B's header ledgers, and the refusal of every other tiny architecture;
//!   • `spatial.txt` — every tensor probe, the int8 decode step and every tiny architecture's authored decode step on
//!     the four spatial backends (Tensix, fabric, RDU, persistent CUDA), each by the one-op path and as a whole program
//!     of sections, every Nexys decode board image the ledger records, and the tiny Qwen3.8's RDU bundles;
//!   • `forward.txt` — every tensor probe, the recognizer's corpus and the fusion census's programs through the older
//!     forward at each of its numerics.
//!
//! A refusal is an artifact too: it is pinned by its text. Each fixture's header names the tree it was recorded on
//! (the commit and the digest of the compiler's sources), and recording generates every artifact twice and refuses to
//! write unless both agree. Re-record a family only with `LOGOS_UPDATE_FIXTURES=1` and a reason: a migration step
//! never re-records, and a deliberate change to a backend re-records its family as it lands.
//!
//!   cargo nextest run -p logicaffeine-tests --test one_ir_identity_lock
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(feature = "codegen")]

#[path = "common/forward_corpus.rs"]
mod forward_corpus;
#[path = "common/identity_pins.rs"]
mod identity_pins;
#[path = "common/scratch.rs"]
mod scratch;
#[path = "common/tiny_models.rs"]
mod tiny_models;
#[path = "compile_common/tensor_probes.rs"]
mod tensor_probes;

use std::collections::HashMap;
use std::path::PathBuf;

use identity_pins::{digest, hold, parallel, refused, root, Pins};
use logicaffeine_compile::compile::RduModel;
use logicaffeine_compile::codegen_cuda::qwen38::Qwen38Cfg;
use logicaffeine_compile::compile::{
    compile_model_to_rdu_streamed, compile_to_cuda_forward, compile_to_cuda_forward_q8, compile_to_cuda_forward_q8_dp4a,
    compile_to_cuda_forward_q8_moe, compile_to_cuda_forward_q8_moe_dp4a, StepShape,
};
use logicaffeine_compile::model::program::{self, Cut};
use logicaffeine_compile::model::spec::ModelSpec;
use logicaffeine_compile::spatial::decode::GgufHeader;
use logicaffeine_compile::spatial::decode_engine::model::EngineNumerics;
use logicaffeine_compile::spatial::decode_engine::program::lower;
use logicaffeine_compile::spatial::int8_model::Int8Model;
use logicaffeine_compile::spatial::lower::cuda::CudaPersistentBackend;
use logicaffeine_compile::spatial::lower::fabric::FabricBackend;
use logicaffeine_compile::spatial::lower::fabric_board::{image_digest, program_board_image, CoreClock, ImageRole, NexysBoard};
use logicaffeine_compile::spatial::lower::rdu::RduSpatialBackend;
use logicaffeine_compile::spatial::lower::tensix::TensixBackend;
use logicaffeine_compile::spatial::lower::{SpatialArtifact, SpatialBackend};
use logicaffeine_compile::spatial::pipeline::{compile_program_to_spatial, compile_program_to_spatial_returning, compile_to_spatial};
use logicaffeine_compile::spatial::rdu_chip::RduChip;
use logicaffeine_compile::spatial::target::SpatialTarget;
use tensor_probes::OP_PROBES;
use tiny_models::{Tiny, ALL};

const CTX: usize = 24;

// ── The D5005 engine ──────────────────────────────────────────────────────────────────────────────

/// The models whose every block the engine runs: the tiny architectures at four times their width, and the 27B's header
/// ledgers. (A real checkpoint is pinned by none of these: whether it is downloaded varies by box.)
fn engine_models() -> Vec<(String, ModelSpec)> {
    let mut out: Vec<(String, ModelSpec)> =
        [Tiny::Qwen35, Tiny::Qwen3, Tiny::Llama, Tiny::LlamaTied].into_iter().map(|t| (format!("tiny-{}", t.name()), t.wide(64))).collect();
    for ledger in ["qwen3.8-27b-q4_k_m", "qwen3.8-27b-q8_0"] {
        let path = root().join(format!("benchmarks/silicon/decode/headers/{ledger}.json"));
        let spec = GgufHeader::read_ledger(&path).unwrap_or_else(|e| panic!("{ledger}: {e:?}")).spec().unwrap_or_else(|e| panic!("{ledger}: {e:?}"));
        out.push((ledger.to_string(), spec));
    }
    out
}

/// A knob point's label: its fields, as their debug text has them, without spaces.
fn point(n: &EngineNumerics) -> String {
    format!("{n:?}").chars().filter(|c| !c.is_whitespace()).collect()
}

fn engine_pins() -> Pins {
    let mut out = Pins::new();
    for (name, spec) in engine_models() {
        for numerics in EngineNumerics::all() {
            let pin = match lower(&spec, numerics, 64) {
                Ok(l) => {
                    let words: Vec<u8> = l.program.encode().iter().flat_map(|w| w.iter().flat_map(|v| v.to_le_bytes())).collect();
                    let identity = format!("{:?}", l.identity);
                    digest(&[&words, identity.as_bytes()])
                }
                Err(e) => refused(&format!("{e:?}")),
            };
            out.insert(format!("{name}/{}", point(&numerics)), pin);
        }
    }
    for t in [Tiny::Qwen3Moe, Tiny::Gemma3, Tiny::GptOss, Tiny::DeepSeek2, Tiny::DeepSeek2Split] {
        let pin = match lower(&t.wide(64), EngineNumerics::all()[0], 64) {
            Ok(l) => digest(&[format!("{:?}", l.program).as_bytes()]),
            Err(e) => refused(&format!("{e:?}")),
        };
        out.insert(format!("tiny-{}/refusal", t.name()), pin);
    }
    out
}

// ── The spatial backends ──────────────────────────────────────────────────────────────────────────

/// The spatial backends, by name, each built for the target it lowers to.
const BACKENDS: [&str; 4] = ["tensix", "fabric", "rdu", "cuda-persistent"];

fn backend(kind: &str) -> Box<dyn SpatialBackend> {
    match kind {
        "tensix" => Box::new(TensixBackend::new(SpatialTarget::wormhole())),
        "fabric" => Box::new(FabricBackend::new(SpatialTarget::sky_fabric(64, 64))),
        "rdu" => Box::new(RduSpatialBackend::new()),
        "cuda-persistent" => Box::new(CudaPersistentBackend::new(SpatialTarget::rtx3060())),
        other => panic!("no spatial backend `{other}`"),
    }
}

/// Everything an artifact prints, with its census.
fn artifact_text(art: &SpatialArtifact) -> String {
    match art {
        SpatialArtifact::Tensix(a) => {
            let mut text = format!("{:?}\u{0}{}\u{0}{}\u{0}{}", a.census, a.host_cpp, a.cmake, a.oracle_c);
            for k in &a.kernels {
                text.push_str(&format!("\u{0}{}\u{0}{}", k.name, k.source));
            }
            text
        }
        SpatialArtifact::Verilog(a) => format!("{:?}\u{0}{}\u{0}{}", a.census, a.rtl, a.testbench),
        SpatialArtifact::Cuda(a) => format!("{:?}\u{0}{}\u{0}{}", a.census, a.program_cu, a.bench_cu),
        SpatialArtifact::SdkGated(a) => format!("{:?}\u{0}{}\u{0}{}\u{0}{}", a.census, a.json, a.pretty, a.reason),
    }
}

fn silicon(name: &str) -> PathBuf {
    root().join("benchmarks/silicon").join(name)
}

/// The int8 model the boards run, and the source of its decode step at the golden trace's first prompt token.
fn int8_step() -> String {
    let j: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(silicon("tiny_transformer.json")).unwrap()).unwrap();
    let m = Int8Model::from_json(&j).expect("the int8 model");
    m.decode_step_lg(j["golden_trace"]["prompt_tokens"][0].as_u64().unwrap() as usize, 0, &[], &[])
}

/// Each Nexys decode board image the ledger records, as the compiler prints it today: the digest of everything
/// synthesis reads.
fn board_image_pins() -> Pins {
    let ledger: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(silicon("decode_images.json")).unwrap()).unwrap();
    let step = int8_step();
    let mut out = Pins::new();
    for i in ledger["images"].as_array().expect("the images") {
        let mut target = match i["target"].as_str().unwrap() {
            "xc7a100t-fabric" => SpatialTarget::xc7a100t_fabric(16, 64),
            "xc7a200t-fabric" => SpatialTarget::xc7a200t_fabric(16, 64),
            other => panic!("an unknown fabric target {other}"),
        };
        target.site.mem_banks = i["mem_banks"].as_u64().unwrap() as usize;
        target.site.vector_lanes = i["vector_lanes"].as_u64().unwrap() as usize;
        let board = match i["part"].as_str().unwrap() {
            "xc7a100tcsg324-1" => NexysBoard::A7,
            "xc7a200tsbg484-1" => NexysBoard::Video,
            other => panic!("an unknown part {other}"),
        };
        let top = i["top"].as_str().unwrap();
        let pll = |k: &str| i["pll"][k].as_u64().unwrap_or_else(|| panic!("{top}: the PLL's `{k}`")) as u32;
        let clock = CoreClock { mult: pll("mult"), div: pll("div"), out: pll("out") };
        let program = compile_program_to_spatial_returning(&step, &target, &Int8Model::step_reads()).unwrap().program.expect("the step lowers whole");
        let pin = match program_board_image(&program, &target, board, ImageRole::DecodeStep, top, clock) {
            Ok(files) => image_digest(&files),
            Err(e) => refused(&format!("{e:?}")),
        };
        out.insert(format!("board/{top}"), pin);
    }
    out
}

/// The tiny Qwen3.8's RDU bundle in every deployment shape: every program's neutral JSON, placement and certificate,
/// the manifest, the constants it ships and the program text it names.
fn rdu_bundle_pins() -> Pins {
    let shapes: [(&str, Option<StepShape>); 5] = [
        ("forward", None),
        ("decode", Some(StepShape { ctx: 8, batch: 1, tp: 1, prefill: None })),
        ("batch2", Some(StepShape { ctx: 8, batch: 2, tp: 1, prefill: None })),
        ("tp2", Some(StepShape { ctx: 8, batch: 1, tp: 2, prefill: None })),
        ("prefill4", Some(StepShape { ctx: 8, batch: 1, tp: 1, prefill: Some(4) })),
    ];
    let mut out = Pins::new();
    for (name, decode) in shapes {
        let mut files = Vec::new();
        let bundle = compile_model_to_rdu_streamed(&RduModel::qwen38(&Qwen38Cfg::tiny()), RduChip::Sn50, "weights", decode, |f| {
            files.push(format!("{}\u{0}{}\u{0}{}\u{0}{}", f.dir, f.neutral_json, f.placement_txt, f.certificate_json));
            Ok(())
        });
        let pin = match bundle {
            Ok(b) => {
                let mut parts: Vec<Vec<u8>> = files.into_iter().map(String::into_bytes).collect();
                parts.push(b.manifest.into_bytes());
                parts.push(b.program.into_bytes());
                for (path, bytes) in b.constants {
                    parts.push(path.into_bytes());
                    parts.push(bytes);
                }
                digest(&parts.iter().map(Vec::as_slice).collect::<Vec<_>>())
            }
            Err(e) => refused(&e),
        };
        out.insert(format!("rdu-bundle/qwen38-tiny/{name}"), pin);
    }
    out
}

/// One program on one spatial backend, by the one-op path and as a whole program of sections.
fn spatial_program_pins(name: &str, src: &str, kind: &str) -> Pins {
    let backend = backend(kind);
    let mut out = Pins::new();
    let pin = match compile_to_spatial(src, backend.target()) {
        Err(reject) => refused(&format!("front end: {reject:?}")),
        Ok(p) => match backend.lower(&p) {
            Ok(art) => digest(&[artifact_text(&art).as_bytes()]),
            Err(e) => refused(&format!("{e:?}")),
        },
    };
    out.insert(format!("{name}/{kind}"), pin);
    // The whole program as sections: the lowering itself (every count, the composed program, its state), then the
    // backend's artifact of it where it is whole.
    let pin = match compile_program_to_spatial(src, backend.target()) {
        Err(reject) => refused(&format!("front end: {reject:?}")),
        Ok(lowering) => {
            let lowered = format!("{lowering:?}");
            let art = match lowering.whole().map(|p| backend.lower(p)) {
                Some(Ok(art)) => artifact_text(&art),
                Some(Err(e)) => format!("refused: {e:?}"),
                None => format!("partial: {}", lowering.refusal()),
            };
            digest(&[lowered.as_bytes(), art.as_bytes()])
        }
    };
    out.insert(format!("{name}/program/{kind}"), pin);
    out
}

fn spatial_pins() -> Pins {
    let mut programs: Vec<(String, String)> = OP_PROBES.iter().map(|p| (format!("probe:{}", p.op), p.program.to_string())).collect();
    programs.push(("int8-decode-step".to_string(), int8_step()));
    for &t in ALL {
        programs.push((format!("{}-decode-step", t.name()), program::written(&t.spec(CTX), Cut::Decode { ctx: CTX }).0 .0));
    }
    let items: Vec<(&str, &str, &str)> =
        programs.iter().flat_map(|(name, src)| BACKENDS.iter().map(move |kind| (name.as_str(), src.as_str(), *kind))).collect();
    let mut out = parallel(&items, |(name, src, kind)| spatial_program_pins(name, src, kind));
    out.extend(board_image_pins());
    out.extend(rdu_bundle_pins());
    out
}

// ── The older CUDA forward ────────────────────────────────────────────────────────────────────────

fn forward_pin(compiled: Result<String, String>) -> String {
    match compiled {
        Ok(cu) => digest(&[cu.as_bytes()]),
        Err(e) => refused(&e),
    }
}

fn forward_pins() -> Pins {
    let mut out = Pins::new();
    let mut plain = forward_corpus::model_programs();
    plain.extend(OP_PROBES.iter().map(|p| (format!("probe:{}", p.op), p.program.to_string())));
    for (name, src) in &plain {
        out.insert(format!("{name}/plain"), forward_pin(compile_to_cuda_forward(src)));
    }
    let none = HashMap::new();
    for (name, src, q8, moe) in forward_corpus::quantized_programs() {
        out.insert(format!("{name}/q8"), forward_pin(compile_to_cuda_forward_q8(&src, &none, &q8)));
        out.insert(format!("{name}/q8-dp4a"), forward_pin(compile_to_cuda_forward_q8_dp4a(&src, &none, &q8)));
        out.insert(format!("{name}/q8-moe"), forward_pin(compile_to_cuda_forward_q8_moe(&src, &none, &q8, &moe)));
        out.insert(format!("{name}/q8-moe-dp4a"), forward_pin(compile_to_cuda_forward_q8_moe_dp4a(&src, &none, &q8, &moe)));
    }
    out
}

#[test]
fn every_engine_program_is_the_pinned_one() {
    hold("engine", |_| engine_pins());
}

#[test]
fn every_spatial_artifact_is_the_pinned_one() {
    hold("spatial", |_| spatial_pins());
}

#[test]
fn every_forward_program_is_the_pinned_one() {
    hold("forward", |_| forward_pins());
}
