//! demo_quality_sides_lock — every scoreboard side says how closely it follows the f32 model, and a quoted win is won
//! at the quality it claims.
//!
//! DEMO-QUALITY-SIDES's RED tests (benchmarks/FUSIONS_WE_NEED.md, section 12.20; the SOTA contract's matched-quality
//! rule, section 2.3). Each row's sides (ours, llama.cpp, and every engine the ledger measured that the row runs) carry
//! an `agreement` object: the golden it was measured against (`model_golden_lock`'s prompts, from the f32 reference),
//! the greedy steps compared, the steps whose top-1 token matched before the prompt's first parting, those leading runs
//! prompt by prompt, and the mean top-logit gap. A prompt is the unit: its steps are not independent draws, since one
//! parting decides every step after it. Ours wins a row only where, over the golden's prompts paired, its leading run
//! less the rival's has a mean at least zero within two standard errors.

use std::path::{Path, PathBuf};

use serde_json::Value;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn scoreboard() -> Value {
    let text = std::fs::read_to_string(root().join("benchmarks/results/models/rtx3060_scoreboard.json")).expect("the scoreboard");
    serde_json::from_str(&text).expect("the scoreboard parses")
}

/// Every side of a row: ours, llama.cpp, and each engine the ledger measured whose side the row holds under the
/// engine's own name.
fn sides<'a>(board: &Value, row: &'a Value) -> Vec<(String, &'a Value)> {
    let mut out = vec![("ours".to_string(), &row["ours"]), ("llama_cpp".to_string(), &row["llama_cpp"])];
    if let Some(engines) = board["engines"].as_object() {
        for name in engines.keys() {
            if row.get(name).is_some() {
                out.push((name.clone(), &row[name.as_str()]));
            }
        }
    }
    out
}

/// A side's agreement: its golden, and each golden prompt's leading run of agreed steps, checked against the golden
/// (one run a prompt, none longer than its continuation, their sum the side's top-1 count, at least 256 steps).
fn agreement(model: &str, name: &str, side: &Value) -> (String, Vec<f64>) {
    let a = &side["agreement"];
    let golden = a["golden"].as_str().unwrap_or_else(|| panic!("{model} {name}: no agreement golden"));
    let text = std::fs::read_to_string(root().join(golden)).unwrap_or_else(|e| panic!("{model} {name}: the golden {golden}: {e}"));
    let g: Value = serde_json::from_str(&text).unwrap_or_else(|e| panic!("{model} {name}: the golden {golden}: {e}"));
    let lengths: Vec<f64> = g["prompts"]
        .as_array()
        .unwrap_or_else(|| panic!("{golden}: no prompts"))
        .iter()
        .map(|p| p["continuation"].as_array().map_or(0, Vec::len) as f64)
        .collect();
    let steps = a["steps"].as_f64().unwrap_or_else(|| panic!("{model} {name}: no agreement steps"));
    let top1 = a["top1"].as_f64().unwrap_or_else(|| panic!("{model} {name}: no top-1 agreement"));
    let runs: Vec<f64> = a["per_prompt"]
        .as_array()
        .unwrap_or_else(|| panic!("{model} {name}: no per-prompt agreement"))
        .iter()
        .map(|r| r.as_f64().unwrap_or_else(|| panic!("{model} {name}: a per-prompt run is not a number")))
        .collect();
    assert_eq!(runs.len(), lengths.len(), "{model} {name}: one leading run for each of the golden's prompts");
    assert!(runs.iter().zip(&lengths).all(|(r, n)| *r >= 0.0 && r <= n), "{model} {name}: a run past its prompt: {runs:?}");
    assert_eq!(steps, lengths.iter().sum::<f64>(), "{model} {name}: the steps are the golden's continuations");
    assert_eq!(top1, runs.iter().sum::<f64>(), "{model} {name}: the top-1 count is the runs' sum");
    assert!(steps >= 256.0, "{model} {name}: {steps} steps");
    assert!(a["top_logit_gap"].as_f64().is_some(), "{model} {name}: no top-logit gap");
    (golden.to_string(), runs)
}

#[test]
fn every_side_records_its_agreement_with_the_f32_reference() {
    let board = scoreboard();
    for row in board["rows"].as_array().expect("rows") {
        let model = row["model"].as_str().unwrap();
        for (name, side) in sides(&board, row) {
            agreement(model, &name, side);
        }
    }
}

#[test]
fn a_quoted_win_agrees_at_least_as_well_as_the_rival_within_two_standard_errors() {
    let board = scoreboard();
    for row in board["rows"].as_array().expect("rows") {
        let model = row["model"].as_str().unwrap();
        let (golden, ours) = agreement(model, "ours", &row["ours"]);
        for (name, side) in sides(&board, row).into_iter().filter(|(n, _)| n != "ours") {
            let (theirs_golden, theirs) = agreement(model, &name, side);
            assert_eq!(theirs_golden, golden, "{model}: {name} is measured on the golden ours is");
            let d: Vec<f64> = ours.iter().zip(&theirs).map(|(o, t)| o - t).collect();
            let n = d.len() as f64;
            let mean = d.iter().sum::<f64>() / n;
            let se = (d.iter().map(|x| (x - mean) * (x - mean)).sum::<f64>() / (n - 1.0) / n).sqrt();
            assert!(
                mean + 2.0 * se >= 0.0,
                "{model}: over {n} prompts ours leads {name} by {mean:.3} agreed steps a prompt, standard error {se:.3}: \
                 a speed win against {name} is not a win at matched quality"
            );
        }
    }
}
