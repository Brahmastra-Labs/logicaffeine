//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! spatial_decode_lock — what one token of batch-1 decode costs a machine, counted from the checkpoint.
//!
//! The step is read off a GGUF header: every tensor's bytes from ggml's own block arithmetic, every
//! tensor's role from llama.cpp's tensor names, the KV cache and the recurrent state from the header's
//! metadata. So the lock first holds the inventory to the FILE — the tensor table must account for every
//! byte from the data offset to the end of the checkpoint — and only then holds the step to the
//! architecture: one read of every projection, one embedding row, no speculative block unless asked, a
//! chain of dependent stages per block with the two tensor-parallel reductions where Megatron puts them.
//!
//! The machines are facts with sources, by the rule `spatial_roofs_lock` holds the roofs to. The
//! calibration holds the model to measurement the only way a floor can be held: no measured decode — ours,
//! llama.cpp's, vLLM's, or a vendor's published per-user rate — may run faster than the floor its machine
//! and checkpoint imply, and every measured number is read back from the ledger it cites, so the
//! calibration table cannot drift from the run it names.
//!
//!   cargo nextest run -p logicaffeine-tests --test spatial_decode_lock
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

use std::path::{Path, PathBuf};

use logicaffeine_compile::codegen_sva::fabric_target::Precision;
use logicaffeine_compile::spatial::decode::{
    price, Chip, DecodeRefusal, DecodeStep, Deployment, GgmlType, GgufHeader, Machine, StageKind,
    TensorRow, TierKind, WeightBytes,
};
use logicaffeine_compile::spatial::rdu_chip::Provenance;

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn header(name: &str) -> GgufHeader {
    let path = repo().join(format!("benchmarks/silicon/decode/headers/{name}.json"));
    GgufHeader::read_ledger(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn every_header() -> Vec<(String, GgufHeader)> {
    let dir = repo().join("benchmarks/silicon/decode/headers");
    let mut out: Vec<(String, GgufHeader)> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|e| e.expect("a directory entry").path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .map(|p| {
            let name = p.file_stem().unwrap().to_string_lossy().into_owned();
            let h = GgufHeader::read_ledger(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
            (name, h)
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    assert!(out.len() >= 5, "the header ledgers: {:?}", out.iter().map(|o| &o.0).collect::<Vec<_>>());
    out
}

fn bytes_of(h: &GgufHeader, t: &TensorRow) -> u64 {
    h.tensor_bytes(t).unwrap_or_else(|e| panic!("{}: {e:?}", t.name))
}

fn sum_bytes(h: &GgufHeader, keep: impl Fn(&str) -> bool) -> u64 {
    h.tensors.iter().filter(|t| keep(&t.name)).map(|t| bytes_of(h, t)).sum()
}

fn close(a: f64, b: f64, rel: f64) -> bool {
    (a - b).abs() <= rel * b.abs()
}

// ── The inventory, held to the file ────────────────────────────────────────────────────────────────

#[test]
fn every_header_accounts_for_every_byte_of_its_checkpoint() {
    for (name, h) in every_header() {
        h.byte_audit().unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(!h.tensors.is_empty(), "{name}: an empty tensor table");
    }
}

#[test]
fn the_type_table_is_ggmls_block_arithmetic() {
    // Transcribed from the `static_assert(sizeof(block_*) == …)` lines of ggml/src/ggml-common.h, with
    // ggml_half two bytes and QK_K 256 — the arithmetic, not the table under test.
    use GgmlType::*;
    let (half, qk_k) = (2u64, 256u64);
    let expect: Vec<(GgmlType, u32, u64, u64)> = vec![
        (F32, 0, 1, 4),
        (F16, 1, 1, 2),
        (Q4_0, 2, 32, half + 32 / 2),
        (Q4_1, 3, 32, 2 * half + 32 / 2),
        (Q5_0, 6, 32, half + 4 + 32 / 2),
        (Q5_1, 7, 32, 2 * half + 4 + 32 / 2),
        (Q8_0, 8, 32, half + 32),
        (Q8_1, 9, 32, 2 * half + 32),
        (Q2K, 10, qk_k, 2 * half + qk_k / 16 + qk_k / 4),
        (Q3K, 11, qk_k, half + qk_k / 4 + qk_k / 8 + 12),
        (Q4K, 12, qk_k, 2 * half + 12 + qk_k / 2),
        (Q5K, 13, qk_k, 2 * half + 12 + qk_k / 2 + qk_k / 8),
        (Q6K, 14, qk_k, half + qk_k / 16 + 3 * qk_k / 4),
        (Q8K, 15, qk_k, 4 + qk_k + qk_k / 16 * 2),
        (Iq2Xxs, 16, qk_k, half + qk_k / 8 * 2),
        (Iq2Xs, 17, qk_k, half + qk_k / 8 * 2 + qk_k / 32),
        (Iq3Xxs, 18, qk_k, half + 3 * (qk_k / 8)),
        (Iq1S, 19, qk_k, half + qk_k / 8 + qk_k / 16),
        (Iq4Nl, 20, 32, half + 32 / 2),
        (Iq3S, 21, qk_k, half + 13 * (qk_k / 32) + qk_k / 64),
        (Iq2S, 22, qk_k, half + qk_k / 4 + qk_k / 16),
        (Iq4Xs, 23, qk_k, half + 2 + qk_k / 64 + qk_k / 2),
        (I8, 24, 1, 1),
        (I16, 25, 1, 2),
        (I32, 26, 1, 4),
        (I64, 27, 1, 8),
        (F64, 28, 1, 8),
        (Iq1M, 29, qk_k, qk_k / 8 + qk_k / 16 + qk_k / 32),
        (Bf16, 30, 1, 2),
        (Tq1_0, 34, qk_k, half + qk_k / 64 + (qk_k - 4 * qk_k / 64) / 5),
        (Tq2_0, 35, qk_k, half + qk_k / 4),
        (Mxfp4, 39, 32, 1 + 32 / 2),
        (Nvfp4, 40, 64, 64 / 16 + 64 / 2),
        (Q1_0, 41, 128, half + 128 / 8),
        (Q2_0, 42, 64, half + 64 / 4),
    ];
    for &(ty, id, elems, bytes) in &expect {
        assert_eq!((ty.id(), ty.block_elems(), ty.block_bytes()), (id, elems, bytes), "{ty:?}");
        assert_eq!(GgmlType::from_id(id), Some(ty), "id {id}");
    }
    assert_eq!(
        GgmlType::ALL.len(),
        expect.len(),
        "every type the table knows is checked against the arithmetic"
    );
    for retired in [4u32, 5, 31, 32, 33, 36, 37, 38, 43, 999] {
        assert_eq!(GgmlType::from_id(retired), None, "id {retired} is retired or unknown and must not be guessed");
    }
    assert_eq!(GgmlType::Q8_0.bytes(64), Some(68));
    assert_eq!(GgmlType::Q8_0.bytes(33), None, "a ragged block has no byte count");
}

// ── The step, held to the architecture ─────────────────────────────────────────────────────────────

#[test]
fn one_token_reads_every_projection_once_one_embedding_row_and_no_speculative_block() {
    for (name, speculative_prefix) in [
        ("qwen3.8-27b-q8_0", Some("blk.64.")),
        ("qwen3-8b-q8_0", None),
        ("llama-3.1-8b-instruct-q8_0", None),
        ("llama-3.1-70b-instruct-q4_k_m", None),
        ("llama-2-70b-chat-q4_k_m", None),
    ] {
        let h = header(name);
        let total = sum_bytes(&h, |_| true);
        let embd = h.tensors.iter().find(|t| t.name == "token_embd.weight").expect("an embedding");
        let table = bytes_of(&h, embd);
        let row = table / embd.dims[1];
        let mtp = speculative_prefix.map_or(0, |p| sum_bytes(&h, |n| n.starts_with(p)));
        let greedy = DecodeStep::of(&h, &Deployment::as_stored(4096)).unwrap();
        assert_eq!(
            greedy.weight_bytes(),
            (total - table + row - mtp) as f64,
            "{name}: every projection once, one embedding row, and no speculative block"
        );
        let spec = DecodeStep::of(&h, &Deployment { speculative: true, ..Deployment::as_stored(4096) }).unwrap();
        assert_eq!(
            spec.weight_bytes(),
            (total - table + row) as f64,
            "{name}: a speculating deployment also reads its draft block"
        );
    }
}

#[test]
fn attention_reads_its_cache_and_the_recurrence_reads_and_writes_its_state() {
    let ctx = 4096u64;
    let llama = DecodeStep::of(&header("llama-3.1-8b-instruct-q8_0"), &Deployment::as_stored(ctx)).unwrap();
    // 32 layers x 8 KV heads x (key 128 + value 128) at f16.
    assert_eq!(llama.state_read_bytes(), (32 * 8 * ctx * (128 + 128) * 2) as f64);
    assert_eq!(llama.state_write_bytes(), (32 * 8 * (128 + 128) * 2) as f64);

    let qwen = DecodeStep::of(&header("qwen3.8-27b-q8_0"), &Deployment::as_stored(ctx)).unwrap();
    // 16 attention layers x 4 KV heads x (256 + 256) at f16; 48 Gated-DeltaNet layers each carrying a
    // 48 x 128 x 128 recurrent state and a (4 - 1) x 10240 convolution window, at f32.
    let kv_read = 16 * 4 * ctx * (256 + 256) * 2;
    let kv_write = 16 * 4 * (256 + 256) * 2;
    let recurrent = 48 * (48 * 128 * 128 + 3 * 10240) * 4;
    assert_eq!(qwen.state_read_bytes(), (kv_read + recurrent) as f64);
    assert_eq!(qwen.state_write_bytes(), (kv_write + recurrent) as f64);
}

#[test]
fn a_block_is_a_chain_of_dependent_stages() {
    use StageKind::*;
    let dense = [Norm, Project, Attention, Project, Norm, Project, Project];
    let llama = DecodeStep::of(&header("llama-3.1-8b-instruct-q8_0"), &Deployment::as_stored(1024)).unwrap();
    assert_eq!(llama.stages.first().map(|s| s.kind), Some(Embed));
    let tail: Vec<StageKind> = llama.stages.iter().rev().take(2).map(|s| s.kind).collect();
    assert_eq!(tail, vec![Head, Norm], "the head ends the chain: its norm, then the vocabulary projection");
    assert_eq!(llama.stages.len(), 1 + 32 * dense.len() + 2);
    let block0: Vec<StageKind> = llama.stages.iter().filter(|s| s.layer == Some(0)).map(|s| s.kind).collect();
    assert_eq!(block0, dense);
    let reducing: Vec<usize> = llama
        .stages
        .iter()
        .enumerate()
        .filter(|(_, s)| s.layer == Some(0) && s.reduces)
        .map(|(i, _)| i - 1)
        .collect();
    assert_eq!(reducing, vec![3, 6], "Megatron reduces after the attention output and after the FFN down");

    let qwen = DecodeStep::of(&header("qwen3.8-27b-q8_0"), &Deployment::as_stored(1024)).unwrap();
    let kinds = |l: u32| -> Vec<StageKind> { qwen.stages.iter().filter(|s| s.layer == Some(l)).map(|s| s.kind).collect() };
    assert_eq!(kinds(0), vec![Norm, Project, Recurrence, Project, Norm, Project, Project]);
    assert_eq!(kinds(3), vec![Norm, Project, Attention, Project, Norm, Project, Project]);
    assert!(kinds(64).is_empty(), "the speculative block is not part of a greedy step");
    for l in 0..64 {
        let reduces = qwen.stages.iter().filter(|s| s.layer == Some(l) && s.reduces).count();
        assert_eq!(reduces, 2, "layer {l}: two reductions per block");
    }
}

#[test]
fn an_unrecognized_tensor_is_refused_by_name() {
    let mut h = header("llama-3.1-8b-instruct-q8_0");
    let mut extra = h.tensors[0].clone();
    extra.name = "blk.0.mystery_proj.weight".into();
    h.tensors.push(extra);
    match DecodeStep::of(&h, &Deployment::as_stored(1024)) {
        Err(DecodeRefusal::UnknownTensor(n)) => assert_eq!(n, "blk.0.mystery_proj.weight"),
        other => panic!("an unrecognized tensor must be refused by name, got {other:?}"),
    }
}

#[test]
fn an_unknown_type_is_refused_not_guessed() {
    let mut h = header("llama-3.1-8b-instruct-q8_0");
    h.tensors[1].type_id = 4;
    match DecodeStep::of(&h, &Deployment::as_stored(1024)) {
        Err(DecodeRefusal::UnknownType { tensor, id }) => assert_eq!((tensor.as_str(), id), (h.tensors[1].name.as_str(), 4)),
        other => panic!("a retired ggml type id must be refused, got {other:?}"),
    }
}

#[test]
fn requantizing_prices_the_projections_and_leaves_the_norms() {
    let h = header("llama-3.1-8b-instruct-q8_0");
    let fp8 = DecodeStep::of(&h, &Deployment { weights: WeightBytes::PerWeight(1.0), ..Deployment::as_stored(1024) }).unwrap();
    let elems = |t: &TensorRow| t.dims.iter().product::<u64>();
    let mut expect = 0u64;
    for t in &h.tensors {
        if t.name == "token_embd.weight" {
            expect += t.dims[0];
        } else if t.dims.len() >= 2 {
            expect += elems(t);
        } else {
            expect += bytes_of(&h, t);
        }
    }
    assert_eq!(fp8.weight_bytes(), expect as f64, "one byte per projection weight, norms as stored");
}

#[test]
fn a_mixture_of_experts_reads_only_the_experts_it_routes_to() {
    // A synthetic two-block MoE checkpoint in llama.cpp's naming: 8 experts, 2 used per token.
    let mut h = header("llama-3.1-8b-instruct-q8_0");
    let arch = h.arch().expect("an architecture").to_string();
    h.metadata.insert(format!("{arch}.expert_count"), serde_json::json!(8));
    h.metadata.insert(format!("{arch}.expert_used_count"), serde_json::json!(2));
    let dense = DecodeStep::of(&h, &Deployment::as_stored(1024)).unwrap();
    let mut moe = h.clone();
    let mut extra = Vec::new();
    for t in moe.tensors.iter_mut() {
        for (from, to) in [("ffn_gate.weight", "ffn_gate_exps.weight"), ("ffn_up.weight", "ffn_up_exps.weight"), ("ffn_down.weight", "ffn_down_exps.weight")] {
            if t.name.ends_with(from) && t.name.starts_with("blk.") {
                t.name = t.name.replace(from, to);
                t.dims.push(8);
            }
        }
        if t.name.ends_with("ffn_gate_exps.weight") {
            let mut router = t.clone();
            router.name = t.name.replace("ffn_gate_exps.weight", "ffn_gate_inp.weight");
            router.dims = vec![4096, 8];
            router.type_id = 0;
            extra.push(router);
        }
    }
    moe.tensors.extend(extra);
    let step = DecodeStep::of(&moe, &Deployment::as_stored(1024)).unwrap();
    let router = 32.0 * 4096.0 * 8.0 * 4.0;
    assert_eq!(
        step.weight_bytes(),
        dense.weight_bytes() * 1.0 + router + {
            let ffn = sum_bytes(&h, |n| n.contains(".ffn_gate.") || n.contains(".ffn_up.") || n.contains(".ffn_down.")) as f64;
            ffn * 8.0 * 2.0 / 8.0 - ffn
        },
        "two of eight experts' FFN bytes, plus the router"
    );
    let router_stages = step.stages.iter().filter(|s| s.kind == StageKind::Router).count();
    assert_eq!(router_stages, 32, "the router is a dependent stage before the experts");
}

// ── The machines ───────────────────────────────────────────────────────────────────────────────────

fn assert_cited(name: &str, facts: &[(String, Provenance, &'static str)], sources: &[&str]) {
    assert!(!facts.is_empty(), "{name}: no facts");
    for (path, provenance, cite) in facts {
        assert!(!cite.trim().is_empty(), "{name} `{path}`: an empty citation");
        match provenance {
            Provenance::Published | Provenance::Derived => assert!(
                sources.iter().any(|s| cite.contains(s)),
                "{name} `{path}` is {provenance:?} but cites `{cite}`, which names none of its sources {sources:?}"
            ),
            Provenance::Measured => assert!(
                cite.contains("benchmarks/") && cite.contains(".json"),
                "{name} `{path}` is measured but cites `{cite}`, which names no run ledger"
            ),
            Provenance::Modeled => assert!(
                ["not public", "not published", "inherited", "not tabulated"].iter().any(|m| cite.contains(m)),
                "{name} `{path}` is modeled but its cite `{cite}` does not say what gap it fills"
            ),
        }
    }
}

#[test]
fn every_machine_fact_names_its_source() {
    for chip in Chip::catalog() {
        assert_cited(chip.name, &chip.facts(), chip.sources);
        for tier in &chip.tiers {
            assert!(tier.capacity.value > 0.0 && tier.bandwidth.value > 0.0, "{} {:?}", chip.name, tier.kind);
        }
    }
}

#[test]
fn the_gpu_numbers_are_nvidias_tables() {
    let gb = 1e9;
    let tera = 1e12;
    for (chip, capacity, bandwidth, int8_dense) in [
        (Chip::h100_sxm(), 80.0 * gb, 3.35e12, 3958.0 * tera / 2.0),
        (Chip::h100_nvl(), 94.0 * gb, 3938.0 * gb, 3341.0 * tera / 2.0),
        (Chip::b200(), 1440.0 * gb / 8.0, 64e12 / 8.0, 72e15 / 2.0 / 8.0),
    ] {
        let [hbm] = chip.tiers.as_slice() else { panic!("{}: one HBM tier", chip.name) };
        assert_eq!(hbm.kind, TierKind::Hbm, "{}", chip.name);
        assert_eq!((hbm.capacity.value, hbm.bandwidth.value), (capacity, bandwidth), "{}", chip.name);
        let int8 = chip.matrix_rate(Precision::Int8).expect("an int8 tensor rate");
        assert!(close(int8.value, int8_dense, 1e-12), "{}: the dense int8 rate is half the sparse figure, got {}", chip.name, int8.value);
    }
}

// ── Pricing semantics ──────────────────────────────────────────────────────────────────────────────

#[test]
fn one_chip_with_no_overhead_is_bytes_over_bandwidth() {
    let h = header("qwen3-8b-q8_0");
    let step = DecodeStep::of(&h, &Deployment::as_stored(512)).unwrap();
    let mut chip = Chip::rtx3060();
    chip.stage_overhead = None;
    let cost = price(&step, &Machine::of(chip.clone(), 1), Precision::Int8).unwrap();
    let bytes = step.weight_bytes() + step.state_read_bytes() + step.state_write_bytes();
    let bw = chip.tiers[0].bandwidth.value;
    assert!(close(cost.floor_s, bytes / bw, 1e-9), "floor {} vs bytes/bw {}", cost.floor_s, bytes / bw);
    assert!(cost.time_s.is_none(), "an undeclared per-stage overhead prices a floor and refuses a prediction");
}

#[test]
fn the_floor_is_never_above_the_prediction() {
    use logicaffeine_compile::spatial::decode::Link;
    use logicaffeine_compile::spatial::rdu_chip::Fact;
    let step = DecodeStep::of(&header("llama-3.1-70b-instruct-q4_k_m"), &Deployment::as_stored(4096)).unwrap();
    let mut priced = 0;
    for mut chip in Chip::catalog() {
        // Declare every term the prediction needs, so the comparison is made on every chip.
        chip.stage_overhead = Some(Fact::modeled(1e-6, "not published; a test overhead"));
        chip.link = Some(Link {
            bandwidth: Some(Fact::modeled(100e9, "not published; a test link")),
            hop_latency: Some(Fact::modeled(500e-9, "not published; a test link")),
        });
        for chips in [1u32, 2, 8, 64, 1024] {
            let Ok(cost) = price(&step, &Machine::of(chip.clone(), chips), Precision::Int8) else { continue };
            let t = cost.time_s.expect("every term declared");
            assert!(cost.floor_s <= t * (1.0 + 1e-12), "{} x{chips}: floor {} above prediction {t}", chip.name, cost.floor_s);
            priced += 1;
        }
    }
    assert!(priced >= 20, "the comparison ran on {priced} machines");
}

#[test]
fn tensor_parallel_pays_two_reductions_per_block_and_one_for_the_head() {
    let step = DecodeStep::of(&header("llama-3.1-8b-instruct-q8_0"), &Deployment::as_stored(1024)).unwrap();
    let one = price(&step, &Machine::of(Chip::h100_sxm(), 1), Precision::Int8).unwrap();
    let eight = price(&step, &Machine::of(Chip::h100_sxm(), 8), Precision::Int8).unwrap();
    assert_eq!(one.collectives, 0, "one chip reduces nothing");
    assert_eq!(eight.collectives, 32 * 2 + 1, "two all-reduces per block and the head's arg-max");
    assert!(eight.floor_s < one.floor_s, "eight chips share the bytes");
}

/// Residency fills the fastest tier with what a token moves the most of per byte held. With SRAM sized to hold exactly
/// the weights and the KV cache, the embedding table (one row read per token, the coldest class) stays in DDR. The floor
/// is then the projections and the cache over SRAM's bandwidth, plus one embedding row over DDR's. Filled coldest
/// first, the table would take SRAM and push projections out to DDR.
#[test]
fn the_fastest_tier_holds_what_a_token_moves_most_of_per_byte() {
    use logicaffeine_compile::spatial::decode::{HeldClass, Tier};
    use logicaffeine_compile::spatial::rdu_chip::Fact;
    let step = DecodeStep::of(&header("qwen3-8b-q8_0"), &Deployment::as_stored(512)).unwrap();
    let held = |c: HeldClass| step.held.iter().filter(|h| h.class == c).map(|h| h.bytes).sum::<f64>();
    let (sram_bw, ddr_bw) = (10e12, 100e9);
    let mut chip = Chip::rtx3060();
    chip.tiers = vec![
        Tier {
            kind: TierKind::Sram,
            capacity: Fact::modeled(held(HeldClass::Weights) + held(HeldClass::KvCache), "a test SRAM: the weights and the cache"),
            bandwidth: Fact::modeled(sram_bw, "a test SRAM"),
        },
        Tier { kind: TierKind::Ddr, capacity: Fact::modeled(1e12, "a test DDR"), bandwidth: Fact::modeled(ddr_bw, "a test DDR") },
    ];
    chip.matrix = vec![];
    chip.stage_overhead = None;
    chip.link = None;
    let cost = price(&step, &Machine::of(chip, 1), Precision::Int8).unwrap();
    let embed: f64 = step.stages.iter().filter(|s| s.kind == StageKind::Embed).map(|s| s.weight_bytes).sum();
    assert!(held(HeldClass::EmbeddingTable) > embed && embed > 0.0, "a token reads one row of the embedding table");
    let state = step.state_read_bytes() + step.state_write_bytes();
    let want = (step.weight_bytes() - embed + state) / sram_bw + embed / ddr_bw;
    assert!(close(cost.floor_s, want, 1e-9), "floor {} vs {want}: the projections and the cache stream from SRAM, the table's row from DDR", cost.floor_s);
}

/// The fewest chips that hold a model is the least such count: the model is resident on that many chips and refused
/// on one fewer.
#[test]
fn the_fewest_chips_are_the_least_that_hold_the_model() {
    use logicaffeine_compile::spatial::decode_chip::fewest_chips;
    let mut checked = 0;
    for name in ["qwen3-8b-q8_0", "llama-3.1-70b-instruct-q4_k_m", "qwen3.8-27b-q8_0"] {
        let step = DecodeStep::of(&header(name), &Deployment::as_stored(4096)).unwrap();
        for chip in Chip::catalog() {
            let n = fewest_chips(&step, &chip, Precision::Int8).unwrap_or_else(|e| panic!("{name} on {}: {e:?}", chip.name));
            assert!(price(&step, &Machine::of(chip.clone(), n), Precision::Int8).is_ok(), "{name} on {}: resident on {n}", chip.name);
            if n > 1 {
                match price(&step, &Machine::of(chip.clone(), n - 1), Precision::Int8) {
                    Err(DecodeRefusal::NotResident { .. }) => checked += 1,
                    other => panic!("{name} on {}: {n} chips is the fewest, yet {} holds it ({other:?})", chip.name, n - 1),
                }
            }
        }
    }
    assert!(checked >= 3, "the minimality was checked on {checked} machines that need more than one chip");
}

#[test]
fn a_checkpoint_that_does_not_fit_is_refused_with_its_size() {
    let step = DecodeStep::of(&header("qwen3.8-27b-q8_0"), &Deployment::as_stored(4096)).unwrap();
    match price(&step, &Machine::of(Chip::rtx3060(), 1), Precision::Int8) {
        Err(DecodeRefusal::NotResident { needed, capacity }) => {
            assert!(needed > capacity, "{needed} vs {capacity}");
            assert_eq!(capacity, Chip::rtx3060().tiers[0].capacity.value);
        }
        other => panic!("a 29 GB checkpoint does not fit 12 GB, got {other:?}"),
    }
}

// ── Calibration: no measured decode beats its floor ────────────────────────────────────────────────

fn deref<'a>(v: &'a serde_json::Value, pointer: &str) -> &'a serde_json::Value {
    v.pointer(pointer).unwrap_or_else(|| panic!("no `{pointer}` in the cited ledger"))
}

fn deployment_of(row: &serde_json::Value) -> Deployment {
    let ctx = row["ctx"].as_u64().expect("ctx");
    let mut d = Deployment::as_stored(ctx);
    if let Some(w) = row["bytes_per_weight"].as_f64() {
        d.weights = WeightBytes::PerWeight(w);
    }
    if let Some(kv) = row["kv_bytes"].as_f64() {
        d.kv_bytes = kv;
    }
    d
}

#[test]
fn a_measured_decode_never_beats_its_floor() {
    let ledger: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(repo().join("benchmarks/silicon/decode/measured.json")).expect("the measured ledger"),
    )
    .expect("it parses");
    let rows = ledger["rows"].as_array().expect("rows");
    assert!(rows.len() >= 4, "the calibration rows");
    for row in rows {
        let label = row["label"].as_str().expect("label");
        let source = row["source"]["ledger"].as_str().expect("a source ledger");
        let cited: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(repo().join(source)).unwrap_or_else(|e| panic!("{label}: {source}: {e}")),
        )
        .expect("the cited ledger parses");
        let measured = row["tok_s"].as_f64().expect("tok_s");
        assert_eq!(
            deref(&cited, row["source"]["pointer"].as_str().expect("pointer")).as_f64(),
            Some(measured),
            "{label}: the row's number is not the number its ledger records"
        );
        let chip = Chip::by_name(row["machine"].as_str().expect("machine")).expect("a catalog chip");
        let step = DecodeStep::of(&header(row["header"].as_str().expect("header")), &deployment_of(row)).unwrap();
        let cost = price(&step, &Machine::of(chip, row["chips"].as_u64().unwrap_or(1) as u32), Precision::Int8)
            .unwrap_or_else(|e| panic!("{label}: {e:?}"));
        let floor = cost.floor_tokens_per_s();
        assert!(measured <= floor, "{label}: measured {measured} tok/s beats its floor {floor:.2} — a wrong byte count or a wrong roof");
    }
}

#[test]
fn a_published_per_user_rate_never_beats_its_floor() {
    let ledger: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(repo().join("benchmarks/silicon/decode/published.json")).expect("the published ledger"),
    )
    .expect("it parses");
    let rows = ledger["rows"].as_array().expect("rows");
    assert!(rows.len() >= 3, "the published rows");
    for row in rows {
        let label = row["label"].as_str().expect("label");
        assert!(
            row["source"]["url"].as_str().is_some_and(|u| u.starts_with("https://")) && row["source"]["quote"].as_str().is_some_and(|q| !q.is_empty()),
            "{label}: a published rate quotes its source"
        );
        let chip = Chip::by_name(row["machine"].as_str().expect("machine")).expect("a catalog chip");
        let step = DecodeStep::of(&header(row["header"].as_str().expect("header")), &deployment_of(row)).unwrap();
        // The fewest chips that hold the checkpoint: more chips only raise a floor whose collective
        // latencies are undeclared, so this is the strictest floor any real deployment of it has.
        let chips = match row["chips"].as_u64() {
            Some(c) => c as u32,
            None => (1..=4096).find(|&c| price(&step, &Machine::of(chip.clone(), c), Precision::Int8).is_ok()).expect("some count holds it"),
        };
        let cost = price(&step, &Machine::of(chip, chips), Precision::Int8).unwrap_or_else(|e| panic!("{label}: {e:?}"));
        let published = row["tok_s"].as_f64().expect("tok_s");
        assert!(published <= cost.floor_tokens_per_s(), "{label}: published {published} beats the floor {:.1}", cost.floor_tokens_per_s());
    }
}

#[test]
fn the_rtx3060_stage_overhead_is_its_measurement() {
    let ledger: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(repo().join("benchmarks/silicon/decode/gpu_stage_overhead.json")).expect("the overhead ledger"),
    )
    .expect("it parses");
    let median = ledger["graph_node_us_median"].as_f64().expect("graph_node_us_median");
    let runs: Vec<f64> = ledger["runs"].as_array().expect("runs").iter().map(|r| r["graph_node_us"].as_f64().unwrap()).collect();
    assert_eq!(runs.len(), 3, "three runs");
    let mut sorted = runs.clone();
    sorted.sort_by(f64::total_cmp);
    assert_eq!(sorted[1], median, "the recorded median is the median of the runs");
    let o = Chip::rtx3060().stage_overhead.expect("the 3060's overhead is declared");
    assert!(close(o.value, median * 1e-6, 1e-12) && o.provenance == Provenance::Measured, "{o:?} vs the ledger's {median} us");
    assert!(o.cite.contains("benchmarks/silicon/decode/gpu_stage_overhead.json"), "{}", o.cite);
    // With it declared, one 3060 prices a prediction, and the prediction is never below the floor.
    let step = DecodeStep::of(&header("qwen3-8b-q8_0"), &Deployment::as_stored(512)).unwrap();
    let cost = price(&step, &Machine::of(Chip::rtx3060(), 1), Precision::Int8).unwrap();
    let t = cost.time_s.expect("a prediction");
    assert!(t >= cost.floor_s && t <= cost.floor_s + step.stages.len() as f64 * o.value * (1.0 + 1e-9));
}
