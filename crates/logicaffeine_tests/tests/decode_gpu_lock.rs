//! decode_gpu_lock — the RTX 3060's side of the decode receipt: the tiny int8 transformer's decode on the GPU, by a
//! kernel written for it by hand and by the programs LOGOS prints, each held to the trainer's tokens.
//!
//! `benchmarks/silicon/decode_gpu.json` records, for one boot of the box, each GPU program's time per token, how it
//! was measured, and — where it was measured — the board's power while the program ran and the energy a token takes.
//! The hand-written reference is one warp that generates every token of the trace on the device in one launch: no
//! barrier wider than the warp, no launch per token, the weights in constant memory. It is the receipt's GPU best,
//! what the part does when a person writes the kernel for this model.
//!
//! This lock holds:
//! - the reference kernel, printed from the model (`Int8Model::reference_cuda`), generates the golden trace's tokens on
//!   the GPU;
//! - the ledger's arithmetic: a token's energy is its time at its power, every program row is a program this lock
//!   names, and the trace on record is the model's own;
//! - the ledger's recorder (`scripts/accel/decode_gpu_ledger.py`) reads a run of `scripts/accel/decode-gpu-measure.sh`
//!   as the ledger states it, and refuses a run that did not finish, a reference that did not generate the trainer's
//!   tokens, a sustained run another process shared the GPU with, and one that held the GPU below 90% utilization.
//!
//! The LOGOS programs' tokens are held by `cuda_spatial_general_lock`.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test decode_gpu_lock` (the reference run is nvcc- and GPU-gated)

#![cfg(not(target_arch = "wasm32"))]

mod common;

use logicaffeine_compile::spatial::int8_model::Int8Model;

fn silicon(name: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon").join(name)
}

fn model() -> (Int8Model, Vec<i64>, Vec<i64>) {
    let j: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(silicon("tiny_transformer.json")).unwrap()).unwrap();
    let toks = |k: &str| -> Vec<i64> { j["golden_trace"][k].as_array().unwrap().iter().map(|t| t.as_i64().unwrap()).collect() };
    (Int8Model::from_json(&j).expect("the model"), toks("prompt_tokens"), toks("generated_tokens"))
}

#[test]
fn the_hand_written_reference_generates_the_golden_tokens() {
    if !common::require_tool("nvcc") || !common::require_gpu() {
        return;
    }
    let (m, prompt, golden) = model();
    let dir = std::env::temp_dir().join(format!("decode_gpu_ref_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("decode.cu"), m.reference_cuda(&prompt, golden.len()).expect("one warp holds the model")).unwrap();
    let build = std::process::Command::new("nvcc").args(["-O3", "-arch=sm_86", "-o"]).arg(dir.join("decode")).arg(dir.join("decode.cu")).output().expect("nvcc");
    assert!(build.status.success(), "nvcc: {}", String::from_utf8_lossy(&build.stderr));
    let run = std::process::Command::new(dir.join("decode")).arg("10").output().expect("the reference runs");
    let out = String::from_utf8_lossy(&run.stdout).to_string();
    let got: Vec<i64> = out
        .lines()
        .filter_map(|l| l.strip_prefix("GEN "))
        .map(|l| l.split_whitespace().nth(1).unwrap().parse().unwrap())
        .collect();
    assert_eq!(got, golden, "the reference generates the trainer's tokens");
    assert!(out.lines().any(|l| l.starts_with("US_PER_TOKEN ")), "the reference times itself");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_ledger_holds_its_arithmetic() {
    let text = std::fs::read_to_string(silicon("decode_gpu.json")).expect("benchmarks/silicon/decode_gpu.json");
    let ledger: serde_json::Value = serde_json::from_str(&text).expect("the ledger is JSON");
    let (_, prompt, golden) = model();
    let trace = |k: &str| -> Vec<i64> { ledger["trace"][k].as_array().expect("the trace on record").iter().map(|t| t.as_i64().unwrap()).collect() };
    assert_eq!((trace("prompt_tokens"), trace("generated_tokens")), (prompt, golden), "the trace on record is the model's own");
    let programs = ledger["programs"].as_array().expect("a `programs` array");
    let names: Vec<&str> = programs.iter().map(|p| p["program"].as_str().expect("a program's name")).collect();
    for want in ["hand-written reference", "LOGOS one-block program", "LOGOS one block per site"] {
        assert!(names.contains(&want), "the ledger records the {want}: {names:?}");
    }
    for p in programs {
        let us = p["us_per_token"].as_f64().expect("a program's time per token");
        assert!(us > 0.0, "{}: a positive time per token", p["program"]);
        if let (Some(w), Some(uj)) = (p["power_w"].as_f64(), p["energy_uj_per_token"].as_f64()) {
            assert!((w * us - uj).abs() < 1e-3 * uj.max(1.0), "{}: a token's energy is its time at its power ({w} W × {us} µs)", p["program"]);
        }
    }
}

/// One run of the measurement, as `decode-gpu-measure.sh` leaves it: the reference generating `generated`, LOGOS's two
/// programs matching the evaluator, each sustained run's samples at `utilization`, and `other` sharing the flat run.
fn gpu_run(d: &std::path::Path, generated: &[i64], finished: bool, utilization: u32, other: &str) {
    std::fs::create_dir_all(d).unwrap();
    let mut log = String::from("boot 2026-10-18 02:53:57 UTC; driver 595.58.02\n");
    if finished {
        log.push_str("done\n");
    }
    std::fs::write(d.join("log.txt"), log).unwrap();
    let gen: String = generated.iter().enumerate().map(|(i, t)| format!("GEN {i} {t}\n")).collect();
    std::fs::write(d.join("ref.out"), format!("{gen}US_PER_TOKEN 11.402\n")).unwrap();
    std::fs::write(d.join("ref_long.out"), "US_PER_TOKEN 11.398\n").unwrap();
    std::fs::write(d.join("logos.out"), "flat: 3 tensors MATCH — US_PER_STEP 53.248\ngeneral: 3 tensors MATCH — US_PER_STEP 120.512\n").unwrap();
    std::fs::write(d.join("flat_sustain.out"), "SUSTAINED_US_PER_STEP 46.258 over 281700 launches\n").unwrap();
    std::fs::write(d.join("general_sustain.out"), "SUSTAINED_US_PER_STEP 105.258 over 123000 launches\n").unwrap();
    let samples: String = (0..40).map(|i| format!("{:.2}, {utilization}, P2\n", 58.0 + (i % 3) as f64)).collect();
    for run in ["ref_long", "flat_sustain", "general_sustain"] {
        std::fs::write(d.join(format!("{run}.power")), &samples).unwrap();
        std::fs::write(d.join(format!("{run}.power.others")), if run == "flat_sustain" { other } else { "none" }).unwrap();
    }
    std::fs::write(d.join("idle.txt"), "17.61, P8\n".repeat(20)).unwrap();
}

fn record_gpu(d: &std::path::Path, out: &std::path::Path) -> std::process::Output {
    std::process::Command::new("python3")
        .arg(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../scripts/accel/decode_gpu_ledger.py"))
        .arg(d)
        .arg(silicon("tiny_transformer.json"))
        .arg(out)
        .output()
        .expect("python3")
}

#[test]
fn the_recorder_reads_a_measured_run_and_refuses_one_that_does_not_hold() {
    if !common::require_tool("python3") {
        return;
    }
    let (_, _, golden) = model();
    let base = std::env::temp_dir().join(format!("decode_gpu_record_{}", std::process::id()));
    let (d, out) = (base.join("run"), base.join("decode_gpu.json"));
    gpu_run(&d, &golden, true, 98, "none");
    let o = record_gpu(&d, &out);
    assert!(o.status.success(), "a clean run is recorded: {}", String::from_utf8_lossy(&o.stderr));
    let l: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&out).unwrap()).unwrap();
    let programs = l["programs"].as_array().unwrap();
    assert_eq!(programs.len(), 3);
    assert!(programs[0]["how"].as_str().unwrap().starts_with("Int8Model::reference_cuda:"), "the reference on record is the library's");
    assert_eq!(programs[0]["us_per_token"].as_f64(), Some(11.398), "the reference's time is its sustained run's");
    assert_eq!(programs[1]["us_per_token"].as_f64(), Some(46.258), "a LOGOS program's time is its sustained run's");
    assert_eq!(programs[1]["best_us_per_step"].as_f64(), Some(53.248), "and its best launch is its bench's");
    assert_eq!(l["boot"].as_str(), Some("2026-10-18 02:53:57 UTC"));
    for (what, finished, generated, utilization, other, says) in [
        ("an unfinished run", false, golden.clone(), 98, "none", "did not finish"),
        ("a reference off the trace", true, golden.iter().map(|t| t + 1).collect(), 98, "none", "the reference generated"),
        ("a run that shared the GPU", true, golden.clone(), 98, "python3", "another process used the GPU"),
        ("a run below 90% utilization", true, golden.clone(), 50, "none", "at 90% utilization or more"),
    ] {
        let _ = std::fs::remove_dir_all(&d);
        gpu_run(&d, &generated, finished, utilization, other);
        let o = record_gpu(&d, &out);
        assert!(!o.status.success(), "{what} is refused");
        assert!(String::from_utf8_lossy(&o.stderr).contains(says), "{what}: {}", String::from_utf8_lossy(&o.stderr));
    }
    let _ = std::fs::remove_dir_all(&base);
}

