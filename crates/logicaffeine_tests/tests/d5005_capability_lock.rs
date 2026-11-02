//! The decode engine's capability matrix, `benchmarks/silicon/d5005/capability.json`: the verdict on every checkpoint
//! header ledger (`benchmarks/silicon/decode/headers`), written by the `d5005_capability` example from
//! `decode_engine::capability`. A checkpoint the engine computes is admitted; any other carries each gap that stands in
//! its way, by name. When the engine gains a capability the matrix changes, and is regenerated on purpose.
//!
//! An engine build runs a checkpoint when the engine computes it and the build has every unit, width and format its
//! step needs (`capability::covers`): the dual of `EngineParams::union`, so a build covers a step exactly when the union
//! of the two is the build, and every gap is named.

use std::path::PathBuf;

use logicaffeine_compile::model::quant::GgmlType;
use logicaffeine_compile::spatial::decode::GgufHeader;
use logicaffeine_compile::spatial::decode_engine::capability::{covers, matrix, needs, runs_on, Gap, Needs, Row, Verdict};
use logicaffeine_compile::spatial::decode_engine::rtl::column::ColumnSet;
use logicaffeine_compile::spatial::decode_engine::rtl::engine::{EngineParams, Streaming};
use logicaffeine_compile::spatial::serving::StateFlush;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn judged() -> Vec<Row> {
    matrix(&root().join("benchmarks/silicon/decode/headers")).expect("every header ledger reads")
}

fn recorded() -> Vec<Row> {
    let text = std::fs::read_to_string(root().join("benchmarks/silicon/d5005/capability.json")).unwrap();
    serde_json::from_str(&text).expect("capability.json is a list of rows")
}

fn verdict<'a>(rows: &'a [Row], checkpoint: &str) -> &'a Verdict {
    &rows.iter().find(|r| r.checkpoint == checkpoint).unwrap_or_else(|| panic!("{checkpoint} is in the matrix")).verdict
}

#[test]
fn the_recorded_matrix_is_what_the_engine_judges_today() {
    let (now, then) = (judged(), recorded());
    let find = |rows: &[Row], checkpoint: &str| rows.iter().find(|r| r.checkpoint == checkpoint).map(|r| r.verdict.clone());
    let mut checkpoints: Vec<&str> = now.iter().chain(&then).map(|r| r.checkpoint.as_str()).collect();
    checkpoints.sort_unstable();
    checkpoints.dedup();
    let stale: Vec<String> = checkpoints
        .into_iter()
        .filter_map(|c| match (find(&now, c), find(&then, c)) {
            (Some(a), Some(b)) if a == b => None,
            (Some(a), Some(b)) => Some(format!("  {c}: now {a:?}, recorded {b:?}")),
            (Some(a), None) => Some(format!("  {c}: now {a:?}, not recorded")),
            (None, _) => Some(format!("  {c}: recorded, and no header ledger judges it now")),
        })
        .collect();
    assert!(
        now == then,
        "capability.json is stale; regenerate it with `cargo run -p logicaffeine-compile --example d5005_capability`:\n{}",
        if stale.is_empty() { "  every verdict agrees; the rows' order differs".to_string() } else { stale.join("\n") }
    );
    let text = std::fs::read_to_string(root().join("benchmarks/silicon/d5005/capability.json")).unwrap();
    let mut canonical = serde_json::to_string_pretty(&now).unwrap();
    canonical.push('\n');
    assert!(text == canonical, "capability.json is the example's own output");
}

#[test]
fn the_model_the_card_was_built_for_is_admitted_in_every_quantization() {
    let rows = judged();
    let quants: Vec<&Row> = rows.iter().filter(|r| r.checkpoint.starts_with("qwen3.8-27b-")).collect();
    assert!(quants.len() >= 7, "every Qwen3.8-27B quantization has a header: {}", quants.len());
    for r in quants {
        assert_eq!(r.verdict, Verdict::Admitted, "{}", r.checkpoint);
    }
    assert_eq!(*verdict(&rows, "qwen3.5-0.8b-q8_0"), Verdict::Admitted, "the small qwen35 the card runs first");
}

/// Llama 3's rotations are scaled by the checkpoint's frequency factors, which the engine writes into its rotary tables.
#[test]
fn llama_3_is_admitted_its_frequency_factors_in_the_rotary_tables() {
    let rows = judged();
    for llama in ["llama-3.1-8b-instruct-q8_0", "llama-3.2-1b-instruct-q8_0", "llama-3.1-70b-instruct-q4_k_m"] {
        assert_eq!(verdict(&rows, llama), &Verdict::Admitted, "{llama}");
    }
}

#[test]
fn every_header_describes_a_model() {
    for r in judged() {
        assert!(!matches!(r.verdict, Verdict::Undescribed(_)), "{}: {:?}", r.checkpoint, r.verdict);
    }
}

#[test]
fn a_refusal_names_each_gap() {
    let rows = judged();
    let Verdict::Refused(gemma) = verdict(&rows, "gemma-3-1b-it-q8_0") else { panic!("gemma is refused") };
    for gap in [
        "attention with a sliding window",
        "a feed-forward block of GELU (every layer)",
        "a norm on the mixer's output (every layer)",
        "a norm on the feed-forward output (every layer)",
        "a scaled token embedding",
    ] {
        assert!(gemma.iter().any(|g| g.starts_with(gap)), "gemma: {gap} in {gemma:?}");
    }
    let Verdict::Refused(moe) = verdict(&rows, "qwen3-30b-a3b-q8_0") else { panic!("a mixture of experts is refused") };
    assert_eq!(moe, &["a feed-forward block of experts (every layer)"]);
    for r in &rows {
        if let Verdict::Refused(gaps) = &r.verdict {
            assert!(!gaps.is_empty(), "{}: a refusal names its gaps", r.checkpoint);
        }
    }
}

/// The most positions a sequence of the card's engine builds reaches (`d5005_afu`).
const CTX: usize = 4096;

/// What each admitted checkpoint's step needs of a build, by checkpoint.
fn admitted_needs() -> Vec<(String, Needs)> {
    judged()
        .into_iter()
        .filter(|r| r.verdict == Verdict::Admitted)
        .map(|r| {
            let path = root().join(format!("benchmarks/silicon/decode/headers/{}.json", r.checkpoint));
            let header = GgufHeader::read_ledger(&path).unwrap();
            let needs = needs(&header, CTX).unwrap_or_else(|e| panic!("{}: {e}", r.checkpoint));
            (r.checkpoint, needs)
        })
        .collect()
}

#[test]
fn a_build_covers_a_step_exactly_when_their_union_is_the_build() {
    let all = admitted_needs();
    assert!(all.len() >= 9, "the admitted checkpoints: {}", all.len());
    let mut covered = 0;
    for (a, build) in &all {
        for (b, step) in &all {
            let union = EngineParams::union(&[build.engine.clone(), step.engine.clone()]);
            let verdict = covers(&build.engine, step);
            assert_eq!(verdict.is_ok(), union.as_ref() == Ok(&build.engine), "{a} running {b}: {verdict:?}, their union {union:?}");
            covered += verdict.is_ok() as usize;
        }
    }
    assert!(covered > all.len(), "some builds run steps other than their own: {covered}");
}

#[test]
fn one_build_runs_every_quantization_of_the_model_the_card_was_built_for() {
    let quants: Vec<(String, Needs)> = admitted_needs().into_iter().filter(|(n, _)| n.starts_with("qwen3.8-27b-")).collect();
    let engines: Vec<EngineParams> = quants.iter().map(|(_, n)| n.engine.clone()).collect();
    let build = EngineParams::union(&engines).expect("one engine runs every quantization");
    for (name, step) in &quants {
        assert_eq!(covers(&build, step), Ok(()), "{name}");
        assert_eq!(covers(&step.engine, &Needs { engine: build.clone(), streamed: Vec::new() }).is_ok(), step.engine == build, "{name} alone");
    }
}

/// A step whose memories are flushed a window behind runs on no build whose recurrence unit replays a whole log at once,
/// one of as many tokens a window or any other, nor on one that rewrites its memories every pass.
#[test]
fn a_step_flushed_a_window_behind_runs_on_no_build_that_replays_a_whole_log() {
    let all = admitted_needs();
    let (_, step) = all.iter().find(|(n, _)| n == "qwen3.8-27b-q4_k_m").unwrap();
    let (width, flush) = step.engine.delta.expect("the 27B is a Gated-DeltaNet model");
    let StateFlush::Every { tokens } = flush else { panic!("the fastest point flushes every window, {flush:?}") };
    let trailing = (width, StateFlush::Trailing { tokens: tokens / 2 });
    let behind = Needs { engine: EngineParams { delta: Some(trailing), ..step.engine.clone() }, streamed: step.streamed.clone() };
    for has in [flush, StateFlush::Every { tokens: tokens / 2 }, StateFlush::EveryPass] {
        let build = EngineParams { delta: Some((width, has)), ..step.engine.clone() };
        assert_eq!(covers(&build, &behind), Err(vec![Gap::Recurrence { has: Some((width, has)), needs: trailing }]), "on a build of {has:?}");
    }
    assert_eq!(covers(&behind.engine, &behind), Ok(()), "a build of the step's own recurrence");
    assert_eq!(
        Gap::Recurrence { has: Some((width, flush)), needs: trailing }.to_string(),
        format!(
            "a recurrence unit for heads of {width} with a log of {}-token windows replayed a window behind, where the build's is for heads of {width} with a log of {tokens} token(s)",
            tokens / 2
        )
    );
}

#[test]
fn every_gap_is_named_by_what_the_step_needs_and_the_build_holds() {
    let all = admitted_needs();
    let (_, step) = all.iter().find(|(n, _)| n == "qwen3.8-27b-q4_k_m").unwrap();
    let e = &step.engine;
    let short = |what: &str, change: &dyn Fn(&mut EngineParams)| {
        let mut build = e.clone();
        change(&mut build);
        covers(&build, step).expect_err(what)
    };
    assert_eq!(short("rows", &|b| b.rows -= 1), [Gap::Short { what: "token-rows", has: e.rows - 1, needs: e.rows }]);
    assert_eq!(short("vector rows", &|b| b.vector_rows -= 1), [Gap::Short { what: "vector rows", has: e.vector_rows - 1, needs: e.vector_rows }]);
    assert_eq!(short("pair rows", &|b| b.pair_rows -= 1), [Gap::Short { what: "pair rows", has: e.pair_rows - 1, needs: e.pair_rows }]);
    assert_eq!(short("side rows", &|b| b.side_rows -= 1), [Gap::Short { what: "side rows", has: e.side_rows - 1, needs: e.side_rows }]);
    assert_eq!(short("beats", &|b| b.store_beats -= 1), [Gap::Short { what: "store beats", has: e.store_beats - 1, needs: e.store_beats }]);
    assert_eq!(short("words", &|b| b.job_words -= 1), [Gap::Short { what: "job words", has: e.job_words - 1, needs: e.job_words }]);
    assert_eq!(short("tensors", &|b| b.tensors -= 1), [Gap::Short { what: "tensor-table entries", has: e.tensors - 1, needs: e.tensors }]);
    assert_eq!(short("layers", &|b| b.layers -= 1), [Gap::Short { what: "layers", has: e.layers - 1, needs: e.layers }]);
    assert_eq!(short("ropes", &|b| b.ropes -= 1), [Gap::Short { what: "rotary tables", has: e.ropes - 1, needs: e.ropes }]);
    let (format, blocks) = e.formats[0];
    assert_eq!(short("a format", &|b| { b.formats.remove(0); }), [Gap::Format { format, has: None, needs: blocks }]);
    assert_eq!(short("a format's rows", &|b| b.formats[0].1 -= 1), [Gap::Format { format, has: Some(blocks - 1), needs: blocks }]);
    let delta = e.delta.expect("the 27B is a Gated-DeltaNet model");
    assert_eq!(short("no recurrence", &|b| b.delta = None), [Gap::Recurrence { has: None, needs: delta }]);
    assert_eq!(short("other heads", &|b| b.delta = Some((delta.0 * 2, delta.1))), [Gap::Recurrence { has: Some((delta.0 * 2, delta.1)), needs: delta }]);
    let attend = e.attend.expect("the 27B attends too");
    assert_eq!(short("no attention", &|b| b.attend = None), [Gap::Attention { has: None, needs: attend }]);
    let fewer = (attend.0, attend.1, attend.2 - 1, attend.3);
    assert_eq!(short("fewer positions", &|b| b.attend = Some(fewer)), [Gap::Attention { has: Some(fewer), needs: attend }]);
    let folding = Needs { engine: EngineParams { folds: true, ..e.clone() }, streamed: step.streamed.clone() };
    assert_eq!(covers(e, &folding), Err(vec![Gap::Folds]), "a step that folds its matrices' sums on a build that writes them");
    assert_eq!(covers(&folding.engine, step), Ok(()), "a build that folds runs a step that does not");
    let every = short("everything", &|b| {
        b.rows -= 1;
        b.formats.clear();
        b.delta = None;
    });
    assert_eq!(every.len(), 1 + e.formats.len() + 1, "one gap each: {every:?}");
    for gap in &every {
        assert!(!gap.to_string().is_empty());
    }
    assert_eq!(
        Gap::Format { format: GgmlType::Q6K, has: None, needs: 32 }.to_string(),
        "q6_K rows of 32 blocks, a format the build's matrix unit does not read"
    );
}

#[test]
fn a_hybrid_build_also_streams_every_format_the_step_streams() {
    let all = admitted_needs();
    let (_, step) = all.iter().find(|(n, _)| n == "qwen3.8-27b-q4_k_m").unwrap();
    assert!(!step.streamed.is_empty(), "the 27B streams its matrices");
    let set = |formats: &[GgmlType]| ColumnSet::hybrid(formats, logicaffeine_compile::spatial::decode_engine::model::EngineNumerics::fastest().activations);
    let layout = logicaffeine_compile::spatial::decode_engine::image::ImageLayout {
        stripe: Some(8),
        ..logicaffeine_compile::spatial::decode_engine::image::ImageLayout::d5005(logicaffeine_compile::spatial::d5005::Beat::Data64)
    };
    let mut build = step.engine.clone();
    build.streaming = Some(Streaming::new(set(&step.streamed), layout));
    assert_eq!(covers(&build, step), Ok(()));
    let missing = step.streamed[0];
    build.streaming = Some(Streaming::new(set(&step.streamed[1..]), layout));
    assert_eq!(covers(&build, step), Err(vec![Gap::Streamed { format: missing }]));
}

#[test]
fn a_checkpoint_runs_on_a_build_when_the_engine_computes_it_and_the_build_covers_its_step() {
    let header = |name: &str| GgufHeader::read_ledger(&root().join(format!("benchmarks/silicon/decode/headers/{name}.json"))).unwrap();
    let small = needs(&header("qwen3.5-0.8b-q8_0"), CTX).unwrap().engine;
    let big = needs(&header("qwen3.8-27b-q4_k_m"), CTX).unwrap().engine;
    assert_eq!(runs_on(&header("qwen3.5-0.8b-q8_0"), &small, CTX), Ok(()));
    let refused = runs_on(&header("qwen3.8-27b-q4_k_m"), &small, CTX).expect_err("the 0.8B build is too small for the 27B");
    for gap in ["64 layers, where the build holds 24", "a format the build's matrix unit does not read"] {
        assert!(refused.iter().any(|g| g.contains(gap)), "{gap} in {refused:?}");
    }
    assert_eq!(runs_on(&header("qwen3.8-27b-q4_k_m"), &big, CTX), Ok(()));
    let gemma = runs_on(&header("gemma-3-1b-it-q8_0"), &big, CTX).expect_err("the engine does not compute gemma");
    assert!(gemma.iter().any(|g| g.starts_with("a feed-forward block of GELU")), "{gemma:?}");
}
