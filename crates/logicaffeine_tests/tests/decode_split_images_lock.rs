//! decode_split_images_lock — the decode step split over the two boards, as its ledger records it, is what the compiler
//! prints today and what the boards ran.
//!
//! `benchmarks/silicon/decode_split_images.json` records the two chip images of the tiny transformer's decode step split
//! Megatron's way (`Int8Model::decode_step_tp_lg` over two chips): chip 0 on Board A (the Nexys Video), chip 1 on Board B
//! (the Nexys A7). Each chip's program is lowered by `compile_program_to_chips_returning` for its board's linked fabric
//! target, returning what the step's host reads (`Int8Model::step_reads`), and printed as a `chip_board` image. The ledger also holds the transfers between the chips and the relay the host runs.
//! This lock holds:
//! - each chip is on its board, its vector unit one its board's part has (a power of two of lanes, no wider than the
//!   part's vector unit or its banks; the widest that routes depends on the chip's program), and each image is built
//!   from the sources the compiler prints today at the lanes it records;
//! - each chip's record — its twin's count at each halt and at its end, its links with their words, its inputs and its
//!   result's words — is the compiler's (`fabric_board::chip_record`); the transfers and the relay are the compiler's;
//!   chip 0's prompt steps reply the evaluator's next tokens;
//! - each image runs a clock its route closes, and each bitstream on record is in the tree where the checkout holds
//!   the images (`common::decode_images_at`);
//! - a measured run, once recorded, is the two boards running their own images through the trace: the trainer's tokens,
//!   the evaluator's at every prompt step, and every step's counts the twin's.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test decode_split_images_lock`

#![cfg(not(target_arch = "wasm32"))]

mod common;
#[path = "common/spatial.rs"]
mod spatial_support;

use logicaffeine_compile::spatial::int8_model::Int8Model;
use logicaffeine_compile::spatial::ir::SpatialProgram;
use logicaffeine_compile::spatial::lower::fabric_board::{chip_record, image_digest, program_board_image, relay_record, CoreClock, ImageRole, NexysBoard};
use logicaffeine_compile::spatial::pipeline::compile_program_to_chips_returning;
use logicaffeine_compile::spatial::target::SpatialTarget;
use logicaffeine_compile::spatial::tensor_parallel::{relay_schedule, ChipTransfer};

fn silicon(name: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon").join(name)
}

fn ledger() -> serde_json::Value {
    let text = std::fs::read_to_string(silicon("decode_split_images.json")).expect("benchmarks/silicon/decode_split_images.json");
    serde_json::from_str(&text).expect("the split ledger is JSON")
}

fn model() -> (Int8Model, Vec<usize>, Vec<usize>) {
    let j: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(silicon("tiny_transformer.json")).unwrap()).unwrap();
    let toks = |k: &str| -> Vec<usize> { j["golden_trace"][k].as_array().unwrap().iter().map(|t| t.as_u64().unwrap() as usize).collect() };
    (Int8Model::from_json(&j).expect("the model"), toks("prompt_tokens"), toks("generated_tokens"))
}

/// Chip `c`'s board, part and its part's preset fabric target: chip 0 on the Nexys Video, chip 1 on the Nexys A7.
fn board_of(c: usize) -> (NexysBoard, &'static str, SpatialTarget) {
    match c {
        0 => (NexysBoard::Video, "A", SpatialTarget::xc7a200t_fabric(16, 64).linked()),
        1 => (NexysBoard::A7, "B", SpatialTarget::xc7a100t_fabric(16, 64).linked()),
        other => panic!("a chip {other} the split does not have"),
    }
}

/// The target chip `c` was printed for: its board's preset at the vector lanes its record states.
fn target_of(c: usize, rec: &serde_json::Value) -> SpatialTarget {
    let (_, _, mut target) = board_of(c);
    target.site.vector_lanes = rec["vector_lanes"].as_u64().unwrap_or_else(|| panic!("chip {c}: its `vector_lanes`")) as usize;
    target
}

/// Each chip's program as the compiler lowers it today for its board at its recorded lanes, and the transfers between
/// them.
fn programs() -> (Vec<SpatialProgram>, Vec<ChipTransfer>) {
    let (m, prompt, _) = model();
    let (src, manifest) = m.decode_step_tp_lg(prompt[0], 0, &[], &[], 2).expect("the model splits over two chips");
    let recs = chips();
    let mut chips = Vec::new();
    let mut transfers = Vec::new();
    for c in 0..2 {
        let target = target_of(c, &recs[c]);
        let lowered = compile_program_to_chips_returning(&src, &target, &manifest, &Int8Model::step_reads()).expect("the walk lifts the split step");
        chips.push(lowered.chips[c].whole().unwrap_or_else(|| panic!("chip {c} lowers whole: {}", lowered.chips[c].refusal())).clone());
        transfers = lowered.transfers;
    }
    (chips, transfers)
}

fn chips() -> Vec<serde_json::Value> {
    let l = ledger();
    let mut v = l["chips"].as_array().expect("the ledger's `chips`").clone();
    v.sort_by_key(|c| c["chip"].as_u64());
    v
}

fn num(v: &serde_json::Value, field: &str) -> f64 {
    v[field].as_f64().unwrap_or_else(|| panic!("a chip's `{field}`"))
}

fn clock_of(c: &serde_json::Value) -> CoreClock {
    let pll = &c["pll"];
    let field = |k: &str| pll[k].as_u64().unwrap_or_else(|| panic!("chip {}: the PLL's `{k}`", c["chip"])) as u32;
    CoreClock { mult: field("mult"), div: field("div"), out: field("out") }
}

#[test]
fn each_chip_is_on_its_board() {
    let cs = chips();
    assert_eq!(cs.len(), 2, "two chips");
    for (c, rec) in cs.iter().enumerate() {
        let (board, letter, target) = board_of(c);
        assert_eq!(rec["chip"].as_u64(), Some(c as u64));
        assert_eq!(rec["board"].as_str(), Some(letter), "chip {c} on board {letter}");
        assert_eq!(rec["part"].as_str(), Some(board.part()), "chip {c} on the {:?}'s part", board);
        assert_eq!(rec["target"].as_str(), Some(target.name), "chip {c} lowered for its board's fabric");
    }
}

/// A chip's vector unit is one its board's part has: a power of two of lanes, no wider than the part's own vector unit
/// and no wider than its banks.
#[test]
fn each_chip_s_vector_unit_is_one_its_board_has() {
    for (c, rec) in chips().iter().enumerate() {
        let (_, _, preset) = board_of(c);
        let lanes = rec["vector_lanes"].as_u64().unwrap_or_else(|| panic!("chip {c}: its `vector_lanes`")) as usize;
        assert!(lanes.is_power_of_two(), "chip {c}: {lanes} lanes is a power of two");
        assert!(lanes <= preset.site.vector_lanes, "chip {c}: {lanes} lanes within its part's {}", preset.site.vector_lanes);
        assert_eq!(rec["mem_banks"].as_u64(), Some(preset.site.mem_banks as u64), "chip {c}: printed at its part's banks");
        assert!(lanes <= preset.site.mem_banks, "chip {c}: {lanes} lanes within its banks");
    }
}

#[test]
fn each_chip_image_is_built_from_the_sources_the_compiler_prints_today() {
    let (programs, _) = programs();
    for (c, rec) in chips().iter().enumerate() {
        let (board, _, _) = board_of(c);
        let target = target_of(c, rec);
        let top = rec["top"].as_str().expect("a chip's `top`");
        let files = program_board_image(&programs[c], &target, board, ImageRole::Chip, top, clock_of(rec))
            .unwrap_or_else(|e| panic!("chip {c}: its board image: {e:?}"));
        assert_eq!(image_digest(&files), rec["sources_sha256"].as_str().unwrap(), "chip {c}: the image on record is what the compiler prints today");
    }
}

#[test]
fn each_chip_record_is_the_compilers() {
    let (programs, transfers) = programs();
    let (m, prompt, golden) = model();
    for (c, rec) in chips().iter().enumerate() {
        let target = target_of(c, rec);
        let want = chip_record(&programs[c], &target).expect("the fabric lowers the chip");
        for field in ["segments", "links", "inputs", "next"] {
            assert_eq!(rec[field], want[field], "chip {c}: its `{field}` is the compiler's");
        }
        assert_eq!(rec["read_latency"].as_u64(), target.site.local_mem_read_latency.map(|l| l as u64), "chip {c}: its read latency");
    }
    let l = ledger();
    let xfer: Vec<serde_json::Value> = transfers.iter().map(|t| serde_json::json!({ "tensor": t.tensor, "from": t.from, "to": t.to })).collect();
    assert_eq!(l["transfers"], serde_json::Value::Array(xfer), "the transfers are the compiler's");
    assert_eq!(l["relay"], relay_record(&relay_schedule(&programs, &transfers).unwrap()), "the relay is the compiler's");
    let target = target_of(0, &chips()[0]);
    let recorded: Vec<usize> = chips()[0]["prompt_next_tokens"].as_array().expect("chip 0's prompt tokens").iter().map(|t| t.as_u64().unwrap() as usize).collect();
    assert_eq!(recorded, spatial_support::prompt_next_tokens(&m, &prompt, &target), "chip 0's prompt steps reply the evaluator's next tokens");
    assert_eq!(recorded.last(), golden.first(), "the prompt's last step predicts the trainer's first token");
}

#[test]
fn each_chip_runs_a_clock_its_route_closes() {
    for rec in chips() {
        let clock = clock_of(&rec);
        assert!(clock.legal(), "chip {}: {clock:?} is a PLLE2 setting a -1 part takes", rec["chip"]);
        let core = num(&rec, "core_mhz");
        assert!((core - clock.mhz()).abs() < 1e-9, "chip {}: the core clock is the PLL's", rec["chip"]);
        assert!(core <= num(&rec, "fmax_mhz"), "chip {}: within its routed Fmax", rec["chip"]);
        let residual = rec["timing"]["residual_ns"].as_f64().expect("hard-cell timing's residual");
        let uncertainty = rec["timing"]["clock_uncertainty_ns"].as_f64().expect("the clock's allowance");
        assert!(1000.0 / core >= 1000.0 / num(&rec, "fmax_mhz") + residual + uncertainty, "chip {}: its period covers the routed path, the residual and the allowance", rec["chip"]);
    }
}

#[test]
fn each_bitstream_on_record_is_in_the_tree() {
    if !common::require_tool("decode-images") {
        return;
    }
    for rec in chips() {
        let name = rec["bitstream"].as_str().unwrap();
        let bytes = std::fs::read(silicon(name)).unwrap_or_else(|_| panic!("benchmarks/silicon/{name}"));
        assert_eq!(bytes.len() as u64, num(&rec, "bytes") as u64, "{name}: its recorded size");
        let hash: String = logicaffeine_compile::crypto_accel::sha256(&bytes).iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(hash, rec["sha256"].as_str().unwrap(), "{name}: its recorded hash");
    }
}

/// The `decode-images` probe reads the chips' bitstreams, no more and no fewer.
#[test]
fn the_decode_images_probe_reads_the_chips_bitstreams() {
    let named: Vec<String> = chips().iter().map(|c| c["bitstream"].as_str().unwrap().to_string()).collect();
    let mut probed = common::decode_ledger_bitstreams(&ledger());
    probed.sort();
    let mut want = named.clone();
    want.sort();
    assert_eq!(probed, want, "the probe reads the chips' bitstreams");
}

/// The boards' measured run, once recorded, is the split host's record of both images running the golden trace: the
/// trainer's tokens, the evaluator's at every prompt step, every step ending on each chip at its twin's count.
#[test]
fn a_measured_run_is_the_two_boards_running_their_own_images() {
    let l = ledger();
    let m = &l["measured"];
    let Some(fields) = m.as_object() else { panic!("`measured` is an object") };
    if fields.is_empty() {
        return;
    }
    let (_, prompt, golden) = model();
    let cs = chips();
    for rec in &cs {
        let b = &m["boards"][rec["board"].as_str().unwrap()];
        assert_eq!(b["bitstream_sha256"], rec["sha256"], "board {}: ran its own image", rec["board"]);
        assert_eq!(b["part"], rec["part"], "board {}: on its own part", rec["board"]);
    }
    assert_eq!(m["replayed"].as_bool(), Some(false), "measured on the boards, not replayed");
    let toks = |k: &str| -> Vec<usize> { m[k].as_array().unwrap_or_else(|| panic!("the measured `{k}`")).iter().map(|t| t.as_u64().unwrap() as usize).collect() };
    assert_eq!(toks("tokens"), golden, "the boards' tokens are the trainer's");
    assert_eq!(m["prompt_next_tokens"], cs[0]["prompt_next_tokens"], "the prompt steps reply the evaluator's tokens");
    let ends: Vec<u64> = cs.iter().map(|c| c["segments"].as_array().unwrap().last().unwrap().as_u64().unwrap()).collect();
    let steps = m["step_cycles"].as_array().expect("each step's counts");
    assert_eq!(steps.len(), prompt.len() + golden.len() - 1, "a step a token of the trace");
    for (s, counts) in steps.iter().enumerate() {
        let counts: Vec<u64> = counts.as_array().unwrap().iter().map(|c| c.as_u64().unwrap()).collect();
        assert_eq!(counts, ends, "step {s}: each chip ends at its twin's count");
    }
    assert!(m["date"].as_str().is_some_and(|d| d.len() >= 10), "the run's date");
    assert!(m["link"].as_str().is_some_and(|d| d.contains("host")), "the record says what the link was");
}
