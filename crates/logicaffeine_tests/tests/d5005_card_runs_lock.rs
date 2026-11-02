//! d5005_card_runs_lock — the D5005 card's decode runs are receipts in the tree, and every figure quoted from them is
//! derived.
//!
//! `benchmarks/silicon/d5005/card_runs.json` holds one row per run of the decode engine on the card, each pointing at
//! its committed log. This lock re-reads every log: the tokens, the core cycles and the faults the ledger states are
//! the log's; a step's seconds are its last position's core cycles times the core's clock divider over the user
//! clock; a run's tokens a second are its sequences over that. Every sequence's continuation is the f32 golden's,
//! token for token, and every run names the image it ran on, which met its user clock, and the program it ran.

use std::path::{Path, PathBuf};

use serde_json::Value;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn ledger() -> Value {
    let text = std::fs::read_to_string(root().join("benchmarks/silicon/d5005/card_runs.json")).expect("card_runs.json");
    serde_json::from_str(&text).expect("card_runs.json parses")
}

fn read(path: &str) -> String {
    std::fs::read_to_string(root().join(path)).unwrap_or_else(|e| panic!("{path}: {e}"))
}

/// The run record a card log prints: its one JSON line.
fn log_record(path: &str) -> Value {
    let text = read(path);
    let line = text.lines().find(|l| l.starts_with('{')).unwrap_or_else(|| panic!("{path}: no run record"));
    serde_json::from_str(line).unwrap_or_else(|e| panic!("{path}: {e}"))
}

fn runs(l: &Value) -> &Vec<Value> {
    l["runs"].as_array().expect("runs")
}

#[test]
fn every_figure_is_derived_from_its_log() {
    let l = ledger();
    let clock_mhz = l["clock"]["user_clock_mhz"].as_f64().expect("user clock");
    let divider = l["clock"]["core_divider"].as_f64().expect("core divider");
    assert!(runs(&l).len() >= 3, "the ledger holds {} runs", runs(&l).len());
    for r in runs(&l) {
        let log = r["log"].as_str().expect("log");
        let rec = log_record(log);
        assert_eq!(r["tokens"], rec["tokens"], "{log}: the ledger's tokens are not the log's");
        assert_eq!(r["prompt"], rec["prompt"], "{log}: the prompt length");
        assert_eq!(r["faults"], rec["faults"], "{log}: faults");
        assert_eq!(r["checks_failed"], rec["checks_failed"], "{log}: failed checks");
        let sequences = rec["sequences"].as_array().expect("sequences").len();
        assert_eq!(r["sequences"].as_u64(), Some(sequences as u64), "{log}: sequences");
        let last = rec["cycles"].as_array().expect("cycles").last().expect("a step").clone();
        assert_eq!(r["core_cycles_last_step"], last, "{log}: the last step's cycles");
        let seconds = last.as_f64().unwrap() * divider / (clock_mhz * 1e6);
        let stated = r["seconds_a_step"].as_f64().expect("seconds_a_step");
        assert!((stated - seconds).abs() < 1e-6, "{log}: {stated} s a step, the log gives {seconds}");
        let rate = sequences as f64 / seconds;
        let stated_rate = r["tokens_a_second"].as_f64().expect("tokens_a_second");
        assert!((stated_rate - rate).abs() < 1e-4, "{log}: {stated_rate} tok/s, the log gives {rate}");
        let ran_at = read(log);
        assert!(
            ran_at.contains(&format!("High clock = {clock_mhz:.1} MHz")),
            "{log}: the run's user clock is not the ledger's {clock_mhz} MHz"
        );
    }
}

#[test]
fn every_run_is_token_exact_to_its_golden() {
    let l = ledger();
    for r in runs(&l) {
        let log = r["log"].as_str().unwrap();
        let rec = log_record(log);
        let golden: Value = serde_json::from_str(&read(r["golden"].as_str().expect("golden"))).expect("the golden parses");
        let prompt = &golden["prompts"][r["golden_prompt"].as_u64().expect("golden_prompt") as usize];
        let want_prompt: Vec<u64> = prompt["tokens"].as_array().unwrap().iter().map(|t| t.as_u64().unwrap()).collect();
        let want: Vec<u64> = prompt["continuation"].as_array().unwrap().iter().map(|t| t.as_u64().unwrap()).collect();
        let n = rec["prompt"].as_u64().unwrap() as usize;
        for seq in rec["sequences"].as_array().unwrap() {
            let toks: Vec<u64> = seq.as_array().unwrap().iter().map(|t| t.as_u64().unwrap()).collect();
            assert_eq!(&toks[..n], want_prompt.as_slice(), "{log}: the prompt is not the golden's");
            let generated = &toks[n..];
            assert!(!generated.is_empty(), "{log}: no generated tokens");
            assert_eq!(generated, &want[..generated.len()], "{log}: a sequence left the f32 golden");
        }
    }
}

#[test]
fn every_run_ran_without_faults() {
    for r in runs(&ledger()) {
        assert_eq!(r["faults"].as_u64(), Some(0), "{}: faults", r["log"]);
        assert_eq!(r["checks_failed"].as_u64(), Some(0), "{}: failed checks", r["log"]);
    }
}

#[test]
fn every_run_names_its_image_and_its_program() {
    let l = ledger();
    let image = &l["image"];
    let fit = read(image["fit_summary"].as_str().expect("fit_summary"));
    assert!(fit.contains("Fitter Status : Successful"), "the image's fit did not succeed:\n{fit}");
    let fmax = read(image["fmax_report"].as_str().expect("fmax_report"));
    let user_clock = fmax
        .lines()
        .find(|l| l.contains("qph_user_clk"))
        .and_then(|l| l.trim_start_matches(';').split_whitespace().next().and_then(|v| v.parse::<f64>().ok()))
        .expect("the user clock's Fmax");
    let clock_mhz = l["clock"]["user_clock_mhz"].as_f64().unwrap();
    assert!(user_clock >= clock_mhz, "the runs' {clock_mhz} MHz is past the image's user-clock Fmax {user_clock} MHz");
    let sof = image["sof_sha256"].as_str().expect("sof_sha256");
    assert!(sof.len() == 64 && sof.bytes().all(|b| b.is_ascii_hexdigit()), "the image's digest: {sof}");
    for r in runs(&l) {
        let program = r["program"].as_str().expect("program");
        let (head, sha) = program.rsplit_once(", sha256 ").unwrap_or_else(|| panic!("{program}: no digest"));
        assert!(head.starts_with("LOGOS program: decode step, ctx "), "{program}: not a decode step's identity");
        assert!(sha.len() == 64 && sha.bytes().all(|b| b.is_ascii_hexdigit()), "{program}: digest");
    }
}

/// The card ran the program `model::author` writes for each checkpoint: the identity a run names is recomputed from
/// the checkpoint's own spec. On demand: needs the checkpoints under `models/`.
#[test]
#[ignore = "reads the real checkpoints: cargo nextest run -p logicaffeine-tests --test d5005_card_runs_lock --run-ignored only"]
fn the_card_ran_the_program_the_author_writes() {
    use logicaffeine_compile::model::gguf::Gguf;
    use logicaffeine_compile::model::program::{written, Cut};
    use logicaffeine_compile::model::spec::ModelSpec;
    for r in runs(&ledger()) {
        let model = r["model"].as_str().unwrap();
        let dir = root().join("models").join(model);
        let gguf = std::fs::read_dir(&dir)
            .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
            .filter_map(|e| e.ok().map(|e| e.path()))
            .find(|p| p.extension().is_some_and(|x| x == "gguf"))
            .unwrap_or_else(|| panic!("no checkpoint under {}", dir.display()));
        let spec = ModelSpec::from_gguf(&Gguf::open(&gguf).unwrap()).unwrap();
        let program = r["program"].as_str().unwrap();
        let ctx: usize = program["LOGOS program: decode step, ctx ".len()..].split(',').next().unwrap().parse().unwrap();
        assert_eq!(written(&spec, Cut::Decode { ctx }).1.line(), program, "{model}: the card ran another program");
    }
}
