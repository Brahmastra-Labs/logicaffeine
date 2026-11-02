//! The decode engine's cycle twin: a step's program priced command by command on the engine's units.
//!
//! `spatial::decode_engine::twin` asks each command of a program for its work and schedules the commands on the
//! card's units. These tests hold its demands to the pricer (`spatial::serving::PassShape`) term for term — the
//! weights a step streams, the state and caches its sequences move, the array's multiply-adds and the lanes' cycles,
//! on the real 27B header ledger (its architecture read from the ledger, the checkpoint's own) at every knob point
//! the pricer describes — and its schedule to its bounds: a window of nothing runs the commands back to back, an
//! unbounded one hides every compute-bound command's weights, and no schedule beats the roofline.

#![cfg(feature = "codegen")]

mod common;

use std::path::Path;

use logicaffeine_compile::model::gguf::Gguf;
use logicaffeine_compile::model::spec::ModelSpec;
use logicaffeine_compile::spatial::d5005::{card, EnginePlan};
use logicaffeine_compile::spatial::decode::GgufHeader;
use logicaffeine_compile::spatial::decode_engine::model::EngineNumerics;
use logicaffeine_compile::spatial::decode_engine::program::lower;
use logicaffeine_compile::spatial::decode_engine::twin::{
    demands, predicted, schedule, serving_of, steady_steps, Demand, SequenceRows, Step, PREDICTED_BATCHES,
};
use logicaffeine_compile::spatial::serving::{PassShape, StateFlush};

const HEADER: &str = "../../benchmarks/silicon/decode/headers/qwen3.8-27b-q4_k_m.json";
const CHECKPOINT: &str = "qwen3.8-27b/Qwen3.8-27B-Q4_K_M.gguf";

/// The 27B's header ledger, and the architecture it describes.
fn qwen38_27b() -> (ModelSpec, GgufHeader) {
    let h = GgufHeader::read_ledger(&Path::new(env!("CARGO_MANIFEST_DIR")).join(HEADER)).unwrap();
    (h.spec().unwrap(), h)
}

/// Every header ledger gives the architecture its checkpoint gives — the same spec, or the same refusal — wherever
/// the checkpoint is downloaded: the file the ledger names under `models/`, or the one its URL names. The 27B the
/// twin prices is among them.
#[test]
fn every_ledger_reads_as_its_checkpoint() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let models = repo.join("models");
    let downloaded = |file: &str| -> Option<std::path::PathBuf> {
        if !file.contains("://") {
            return repo.join(file).is_file().then(|| repo.join(file));
        }
        let base = file.rsplit('/').next()?;
        std::fs::read_dir(&models).ok()?.filter_map(|d| d.ok()).map(|d| d.path().join(base)).find(|p| p.is_file())
    };
    let mut compared = Vec::new();
    for entry in std::fs::read_dir(repo.join("benchmarks/silicon/decode/headers")).unwrap() {
        let path = entry.unwrap().path();
        let ledger: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let Some(file) = ledger["_source"]["file"].as_str().and_then(downloaded) else { continue };
        let h = GgufHeader::read_ledger(&path).unwrap();
        let from_checkpoint = ModelSpec::from_gguf(&Gguf::open(&file).unwrap());
        assert_eq!(h.spec(), from_checkpoint, "{}: the ledger reads otherwise than {}", path.display(), file.display());
        compared.push(path.file_name().unwrap().to_string_lossy().into_owned());
    }
    if common::tool_available(&format!("checkpoint:{CHECKPOINT}")) {
        assert!(compared.iter().any(|c| c == "qwen3.8-27b-q4_k_m.json"), "the 27B's ledger compared: {compared:?}");
    }
}

fn total(ds: &[Demand]) -> Demand {
    let mut t = Demand::default();
    for &d in ds {
        t += d;
    }
    t
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9 * a.abs().max(b.abs()).max(1.0)
}

/// Over a batch of sequences a row each, the twin's weights, its traffic rewritten in place and appended, its array
/// multiply-adds and its lane cycles are the pricer's, at every knob point the pricer describes: a lazily flushed one
/// over every phase of its flush window, whose mean is the pass the pricer prices.
#[test]
fn the_twins_weights_traffic_array_and_lanes_are_the_pricers() {
    let (spec, h) = qwen38_27b();
    for n in EngineNumerics::all() {
        for (batch, position) in [(1usize, 0usize), (32, 2047), (134, 511)] {
            let steps = steady_steps(n, batch, position);
            let program = lower(&spec, n, spec.ctx_len).unwrap().program;
            let twin = total(&steps.iter().map(|s| total(&demands(&spec, &h, &program, s).unwrap())).collect::<Vec<_>>());
            let shape = PassShape::of(&h, &serving_of(&spec, n, batch, (position + 1) as f64)).unwrap();
            // Every phase is one pass of the batch.
            let passes = steps.len() as f64;
            let b = batch as f64 * passes;
            let what = format!("{n:?}, {batch} at {position}");
            let weights = passes * shape.weight_bytes + b * shape.embed_bytes;
            assert!(close(twin.weights, weights), "{what}: weights {} vs {weights}", twin.weights);
            let rewritten = b * shape.rewrite_bytes;
            assert!(close(twin.rewritten, rewritten), "{what}: rewritten {} vs {rewritten}", twin.rewritten);
            let appended = b * (shape.read_bytes + shape.write_bytes - shape.rewrite_bytes);
            assert!(close(twin.appended, appended), "{what}: appended {} vs {appended}", twin.appended);
            assert!(close(twin.array, b * shape.array_macs), "{what}: array {} vs {}", twin.array, b * shape.array_macs);
            assert!(close(twin.lanes, b * shape.scale_cycles), "{what}: lanes {} vs {}", twin.lanes, b * shape.scale_cycles);
            assert!(twin.vector >= b * shape.vector_macs * (1.0 - 1e-9), "{what}: the twin's vector work {} below the pricer's {}", twin.vector, b * shape.vector_macs);
        }
    }
}

/// A state rewritten each pass is all rewritten in place; a lazily flushed one only in the step that flushes it, and
/// read alone in every other.
#[test]
fn a_state_is_rewritten_only_in_the_step_that_writes_it() {
    let (spec, h) = qwen38_27b();
    let rewritten = |n: EngineNumerics, logged: usize| {
        let step = Step { sequences: vec![SequenceRows { rows: 1, position: 2047, logged }], drafted: false };
        total(&demands(&spec, &h, &lower(&spec, n, spec.ctx_len).unwrap().program, &step).unwrap()).rewritten
    };
    let eager = EngineNumerics { flush: StateFlush::EveryPass, ..EngineNumerics::fastest() };
    let StateFlush::Every { tokens } = EngineNumerics::fastest().flush else { panic!("the fastest point flushes lazily") };
    let lazy = EngineNumerics::fastest();
    assert!(rewritten(eager, 0) > 0.0);
    for logged in 0..tokens as usize - 1 {
        assert_eq!(rewritten(lazy, logged), 0.0, "a step that leaves {} of {tokens} logged rewrites nothing", logged + 1);
    }
    assert_eq!(rewritten(lazy, tokens as usize - 1), rewritten(eager, 0), "the flushing step rewrites the state as an eager pass does");
}

/// A window of nothing runs every command alone, so the step is the sum of each command's slowest unit; an unbounded
/// window lets the memory run ahead, so the step is never longer and never shorter than the busiest unit's time.
#[test]
fn the_schedule_lies_between_its_bounds() {
    let (spec, h) = qwen38_27b();
    let card = card(&EnginePlan::standard());
    for n in [EngineNumerics::precise(), EngineNumerics::fastest()] {
        for batch in [1usize, 16, 134] {
            let step = Step { sequences: vec![SequenceRows { rows: 1, position: 2047, logged: 0 }; batch], drafted: false };
            let ds = demands(&spec, &h, &lower(&spec, n, spec.ctx_len).unwrap().program, &step).unwrap();
            let serial: f64 = ds
                .iter()
                .map(|d| d.memory_s(&card, d.weights).max(d.array / card.array.value).max(d.lanes / card.scale.value).max(d.vector / card.vector.value))
                .sum();
            let closed = schedule(&ds, &card, 0.0);
            assert!(close(closed.step_s, serial), "{n:?} × {batch}: a window of nothing gives {} against the serial {serial}", closed.step_s);
            let open = schedule(&ds, &card, f64::INFINITY);
            let busiest = open.memory_s.max(open.array_s).max(open.lanes_s).max(open.vector_s);
            assert!(open.step_s <= closed.step_s * (1.0 + 1e-12), "{n:?} × {batch}: room to prefetch made the step longer");
            assert!(open.step_s >= busiest * (1.0 - 1e-12), "{n:?} × {batch}: the step {} shorter than its busiest unit {busiest}", open.step_s);
            let t = total(&ds);
            assert!(close(open.memory_s, t.memory_s(&card, t.weights)), "{n:?} × {batch}: the memory moved other than the step's bytes");
        }
    }
}

/// No schedule beats the roofline: the twin's step at any window is at least the pricer's pass, wherever the card
/// holds the batch.
#[test]
fn no_schedule_beats_the_roofline() {
    let (spec, h) = qwen38_27b();
    let card = card(&EnginePlan::standard());
    for n in EngineNumerics::all().into_iter().filter(|n| n.flush == StateFlush::EveryPass) {
        for batch in [1usize, 32, 134] {
            let step = Step { sequences: vec![SequenceRows { rows: 1, position: 2047, logged: 0 }; batch], drafted: false };
            let ds = demands(&spec, &h, &lower(&spec, n, spec.ctx_len).unwrap().program, &step).unwrap();
            // A batch the card cannot hold at this knob point has no roofline to beat.
            let Ok(cost) = PassShape::of(&h, &serving_of(&spec, n, batch, 2048.0)).unwrap().cost(&card, batch as u32, 0) else { continue };
            let pass = cost.pass_s;
            for window in [0.0, 4e6, f64::INFINITY] {
                let t = schedule(&ds, &card, window).step_s;
                assert!(t >= pass * (1.0 - 1e-9), "{n:?} × {batch} window {window}: {t} s beats the roofline's {pass} s");
            }
        }
    }
}

fn first_difference(a: &serde_json::Value, b: &serde_json::Value, at: String) -> Option<String> {
    use serde_json::Value::*;
    match (a, b) {
        (Number(x), Number(y)) => {
            let (x, y) = (x.as_f64()?, y.as_f64()?);
            (!close(x, y)).then_some(format!("{at}: {x} vs {y}"))
        }
        (Array(x), Array(y)) if x.len() == y.len() => {
            x.iter().zip(y).enumerate().find_map(|(i, (x, y))| first_difference(x, y, format!("{at}/{i}")))
        }
        (Object(x), Object(y)) if x.len() == y.len() && x.keys().all(|k| y.contains_key(k)) => {
            x.iter().find_map(|(k, v)| first_difference(v, &y[k], format!("{at}/{k}")))
        }
        _ => (a != b).then_some(format!("{at}: {a} vs {b}")),
    }
}

/// `benchmarks/silicon/d5005/predicted.json` is what the twin computes from the header ledgers.
#[test]
fn the_committed_prediction_is_what_the_twin_computes() {
    let silicon = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon");
    let path = silicon.join("d5005/predicted.json");
    let committed: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))).expect("the prediction parses");
    let fresh = serde_json::to_value(predicted(&silicon.join("decode")).expect("the twin predicts")).expect("serializes");
    if let Some(at) = first_difference(&committed, &fresh, String::new()) {
        panic!("benchmarks/silicon/d5005/predicted.json is stale at `{at}`: regenerate it with `cargo run -p logicaffeine-compile --example d5005_twin`");
    }
}

/// Every predicted row is a batch the card holds, at or below its roofline, and its step is its units' work scheduled:
/// never shorter than the busiest unit. Each knob point ends at the largest batch the card holds.
#[test]
fn every_prediction_is_at_or_below_its_roofline() {
    let silicon = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon");
    let p = predicted(&silicon.join("decode")).unwrap();
    let (spec, h) = qwen38_27b();
    let card = card(&EnginePlan::standard());
    for n in EngineNumerics::real_points() {
        let rows: Vec<_> = p.rows.iter().filter(|r| r.numerics == format!("{n:?}")).collect();
        let most = logicaffeine_compile::spatial::serving::max_batch(&h, &card, &serving_of(&spec, n, 1, 2048.0)).unwrap() as usize;
        assert_eq!(rows.last().map(|r| r.batch), Some(most), "{n:?}: the prediction ends at the largest batch the card holds");
        assert!(rows.iter().all(|r| r.batch <= most) && rows.len() <= PREDICTED_BATCHES.len() + 1, "{n:?}: a batch the card cannot hold");
    }
    for row in &p.rows {
        assert!(row.roofline_tokens_per_s > 0.0, "{} × {}: the pricer holds no such batch", row.numerics, row.batch);
        assert!(row.tokens_per_s <= row.roofline_tokens_per_s * (1.0 + 1e-9), "{} × {}: {} tok/s past the roofline's {}", row.numerics, row.batch, row.tokens_per_s, row.roofline_tokens_per_s);
        let busiest = row.memory_s.max(row.array_s).max(row.lanes_s).max(row.vector_s);
        assert!(row.step_s >= busiest * (1.0 - 1e-12), "{} × {}: a step shorter than its busiest unit", row.numerics, row.batch);
        let shares: f64 = row.bound_share.values().sum();
        assert!((shares - 1.0).abs() < 1e-9, "{} × {}: bound shares sum to {shares}", row.numerics, row.batch);
    }
}
