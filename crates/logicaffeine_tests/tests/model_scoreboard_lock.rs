//! The bring-up scoreboard (`benchmarks/results/models/rtx3060_scoreboard.json`, written by
//! `scripts/accel/model_scoreboard.py`): the compiled lane against llama.cpp and the serving engines (vLLM, ExLlamaV3)
//! on one GPU in one boot, every run kept.
//!
//! These locks hold the ledger to what it claims:
//!   * every row has both sides, the same number of runs on each;
//!   * every statistic is recomputed here from the runs, and llama-bench's own averages agree with the runs it printed;
//!   * each llama.cpp run is its tokens over the nanoseconds llama-bench measured for it;
//!   * our side is the compiled lane — its provenance line is the compiled one, never a hand-written lane's — and it
//!     names the cuts of the model's one program it ran, the decode step's identity recomputed from the checkpoint
//!     wherever the checkpoint is on this machine;
//!   * llama.cpp is the pinned one;
//!   * every engine side keeps every run on the wall's clock and the engine's own, names the checkpoint it ran (its
//!     weight files' bytes recomputed wherever it is on this machine) and runs the one version the ledger names;
//!   * every batch row runs the lane's batch step — the model's batch cut of that many sequences — beside vLLM at the
//!     same concurrency, every run kept on both of vLLM's clocks.
//! And they hold the recorder to timing only a card it has to itself: a side starts once no other process holds the GPU,
//! a side a peer joined runs again, and a card held past the wait is refused, each naming the process.
//! Ratios are derived from the runs by `mean`, never typed: a deck or a demo sheet quotes what this file computes.

#![cfg(feature = "codegen")]

mod common;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use logicaffeine_compile::codegen_cuda::provenance::{header, Provenance};
use logicaffeine_compile::model::gguf::Gguf;
use logicaffeine_compile::model::program::{written, Cut};
use logicaffeine_compile::model::spec::ModelSpec;
use serde_json::{json, Value};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn ledger() -> Value {
    let path = root().join("benchmarks/results/models/rtx3060_scoreboard.json");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn runs(v: &Value, what: &str) -> Vec<f64> {
    let runs: Vec<f64> = v.as_array().unwrap_or_else(|| panic!("{what}: runs")).iter().map(|x| x.as_f64().unwrap()).collect();
    assert!(runs.iter().all(|x| x.is_finite() && *x > 0.0), "{what}: every run a positive rate: {runs:?}");
    runs
}

/// The mean of a side's runs.
fn mean(v: &[f64]) -> f64 {
    v.iter().sum::<f64>() / v.len() as f64
}

/// The sample standard deviation, as llama-bench reports its own.
fn stddev(v: &[f64]) -> f64 {
    let m = mean(v);
    (v.iter().map(|x| (x - m) * (x - m)).sum::<f64>() / (v.len() - 1) as f64).sqrt()
}

#[test]
fn every_row_has_both_sides_and_the_same_number_of_runs() {
    let l = ledger();
    assert_eq!(l["schema"], 1);
    let repeats = l["protocol"]["repeats"].as_u64().unwrap() as usize;
    assert!(repeats >= 5, "at least llama-bench's default five repetitions: {repeats}");
    let rows = l["rows"].as_array().unwrap();
    assert!(rows.len() >= 6, "every local family on the scoreboard: {}", rows.len());
    let mut names: Vec<&str> = rows.iter().map(|r| r["model"].as_str().unwrap()).collect();
    names.sort();
    names.dedup();
    assert_eq!(names.len(), rows.len(), "one row a model");
    for r in rows {
        let m = r["model"].as_str().unwrap();
        for (side, v) in [
            ("ours decode", &r["ours"]["short"]["decode_tok_s"]),
            ("ours prefill", &r["ours"]["long"]["prefill_tok_s"]),
            ("llama.cpp tg128", &r["llama_cpp"]["tg128"]["tok_s"]),
            ("llama.cpp pp512", &r["llama_cpp"]["pp512"]["tok_s"]),
        ] {
            assert_eq!(runs(v, &format!("{m} {side}")).len(), repeats, "{m} {side}");
        }
        assert_eq!(r["ours"]["short"]["n_gen"], 128, "{m}: our decode is over 128 tokens, as tg128");
        assert_eq!(r["ours"]["long"]["prompt_tokens"], 512, "{m}: our prefill is one 512-token prompt, as pp512");
        assert_eq!(r["ours"]["long"]["prefill_chunk"], 512, "{m}: in one chunk");
        let load = r["load_average"].as_array().unwrap();
        assert_eq!(load.len(), 2, "{m}: the box's load before and after the row");
    }
}

#[test]
fn llama_benchs_own_averages_are_the_runs_it_printed() {
    for r in ledger()["rows"].as_array().unwrap() {
        for test in ["tg128", "pp512"] {
            let t = &r["llama_cpp"][test];
            let v = runs(&t["tok_s"], test);
            let (avg, sd) = (t["reported_avg"].as_f64().unwrap(), t["reported_stddev"].as_f64().unwrap());
            assert!((mean(&v) - avg).abs() <= 1e-6 * avg.max(1.0), "{} {test}: mean {} vs reported {avg}", r["model"], mean(&v));
            assert!((stddev(&v) - sd).abs() <= 1e-6 * avg.max(1.0), "{} {test}: sd {} vs reported {sd}", r["model"], stddev(&v));
        }
    }
}

/// Each llama.cpp run is its own timing: the rate is the test's tokens over the nanoseconds llama-bench measured for it
/// (`samples_ns`), not the six-digit `samples_ts` it prints.
#[test]
fn every_llama_cpp_run_is_its_tokens_over_its_own_nanoseconds() {
    for r in ledger()["rows"].as_array().unwrap() {
        for (test, tokens) in [("tg128", 128u64), ("pp512", 512)] {
            let t = &r["llama_cpp"][test];
            let what = format!("{} {test}", r["model"]);
            assert_eq!(t["tokens"].as_u64(), Some(tokens), "{what}: the test's tokens");
            let ns: Vec<u64> = t["ns"].as_array().unwrap_or_else(|| panic!("{what}: ns")).iter().map(|x| x.as_u64().unwrap()).collect();
            let rates = runs(&t["tok_s"], &what);
            assert_eq!(ns.len(), rates.len(), "{what}: a timing for every run");
            for (n, rate) in ns.iter().zip(&rates) {
                let exact = tokens as f64 * 1e9 / *n as f64;
                assert!((rate - exact).abs() <= 1e-9 * exact, "{what}: {rate} is not {tokens} tokens in {n} ns ({exact})");
            }
        }
    }
}

#[test]
fn our_side_is_the_compiled_lane_running_the_models_one_program() {
    let compiled = header(Provenance::Compiled);
    for r in ledger()["rows"].as_array().unwrap() {
        let m = r["model"].as_str().unwrap();
        for run in ["short", "long"] {
            let o = &r["ours"][run];
            assert_eq!(o["provenance"].as_str().unwrap(), compiled.trim_end(), "{m} {run}: the compiled lane");
            let ctx = o["ctx"].as_u64().unwrap() as usize;
            let programs: Vec<&str> = o["programs"].as_array().unwrap().iter().map(|p| p.as_str().unwrap()).collect();
            assert!(
                programs[0].starts_with(&format!("LOGOS program: decode step, ctx {ctx}, sha256 ")),
                "{m} {run}: the decode step first: {programs:?}"
            );
            let file = root().join(r["file"].as_str().unwrap());
            if file.is_file() {
                assert_eq!(std::fs::metadata(&file).unwrap().len(), r["file_bytes"].as_u64().unwrap(), "{m}: the checkpoint measured");
                let spec = ModelSpec::from_gguf(&Gguf::open(&file).unwrap()).unwrap();
                let decode = written(&spec, Cut::Decode { ctx }).1;
                assert_eq!(programs[0], decode.line(), "{m} {run}: the decode step is the one program's");
                let chunk = o["prefill_chunk"].as_u64().unwrap() as usize;
                let mut want = vec![decode.line()];
                let mut rows = chunk;
                while rows > 1 {
                    want.push(written(&spec, Cut::Prefill { rows, ctx }).1.line());
                    rows /= 2;
                }
                assert_eq!(programs, want, "{m} {run}: every chunk the lane compiled is a cut of the one program");
            }
        }
    }
}

/// The card's streaming read ceiling is measured in the scoreboard's own boot (`scripts/accel/read_ceiling.cu`, every
/// run kept), and every row records the weight bytes its decode step reads a token (`Gguf::decode_read_bytes`,
/// recomputed here from the checkpoint wherever it is on this machine), so the share of the ceiling a decode rate
/// reaches is derived from the ledger, never typed. A dense model's rate never streams its bytes past the ceiling:
/// that would be a byte count the step does not read.
#[test]
fn the_decode_rates_stand_beside_the_cards_measured_read_ceiling() {
    let l = ledger();
    let ceiling = runs(&l["machine"]["read_ceiling_gb_s"], "the read ceiling");
    assert!(ceiling.len() >= 2, "the read ceiling over at least two runs: {ceiling:?}");
    assert!(ceiling.iter().all(|&g| g < 1000.0), "a consumer card's read ceiling, in GB/s: {ceiling:?}");
    let best = ceiling.iter().cloned().fold(0.0, f64::max);
    for r in l["rows"].as_array().unwrap() {
        let m = r["model"].as_str().unwrap();
        let bytes = r["decode_bytes_per_token"].as_u64().unwrap_or_else(|| panic!("{m}: the decode step's bytes a token"));
        assert!(bytes > 0 && bytes <= r["file_bytes"].as_u64().unwrap(), "{m}: {bytes} bytes, within the checkpoint");
        let file = root().join(r["file"].as_str().unwrap());
        let routed = if file.is_file() {
            let g = Gguf::open(&file).unwrap();
            assert_eq!(bytes, g.decode_read_bytes().unwrap(), "{m}: the bytes the checkpoint's decode step reads");
            g.tensors().iter().any(|t| t.name.contains("_exps."))
        } else {
            r["decode_routed"].as_bool().unwrap_or(true)
        };
        if !routed {
            let rate = mean(&runs(&r["ours"]["short"]["decode_tok_s"], m)) * bytes as f64 / 1e9;
            assert!(rate <= best * 1.02, "{m}: decodes {rate:.1} GB/s of weights, past the card's {best:.1} GB/s ceiling");
        }
    }
}

#[test]
fn llama_cpp_is_the_pinned_build() {
    let pins = std::fs::read_to_string(root().join("external/competitors/PINS.tsv")).unwrap_or_default();
    let l = ledger();
    let pin = &l["llama_cpp"];
    assert_eq!(pin["repo"], "ggml-org/llama.cpp");
    let commit = pin["commit"].as_str().unwrap();
    assert_eq!(commit.len(), 40, "a full commit: {commit}");
    if !pins.is_empty() {
        let row = pins.lines().find(|l| l.starts_with("llama.cpp\t")).expect("llama.cpp is pinned");
        assert!(row.ends_with(commit), "the ledger names the pinned llama.cpp: {row} vs {commit}");
    }
}

/// What the ledger says of a model: (ours over llama.cpp's decode, ours over llama.cpp's pp512), mean over mean.
pub fn ratios(row: &Value) -> (f64, f64) {
    let ours_d = mean(&runs(&row["ours"]["short"]["decode_tok_s"], "ours decode"));
    let ours_p = mean(&runs(&row["ours"]["long"]["prefill_tok_s"], "ours prefill"));
    let theirs_d = mean(&runs(&row["llama_cpp"]["tg128"]["tok_s"], "tg128"));
    let theirs_p = mean(&runs(&row["llama_cpp"]["pp512"]["tok_s"], "pp512"));
    (ours_d / theirs_d, ours_p / theirs_p)
}

#[test]
fn every_ratio_is_derived_from_the_runs() {
    for r in ledger()["rows"].as_array().unwrap() {
        let (d, p) = ratios(r);
        assert!(d.is_finite() && d > 0.0 && p.is_finite() && p > 0.0, "{}: {d} {p}", r["model"]);
        assert!(r.get("ratio").is_none() && r.get("speedup").is_none(), "{}: a ratio is computed, never stored", r["model"]);
    }
}

/// The serving engines a row may carry beside llama.cpp, each in its own 8-bit weight form of the model.
const ENGINES: [&str; 2] = ["vllm", "exllamav3"];

/// An engine side's runs: by the wall and by the engine's own clock, decode and prefill.
const ENGINE_RUNS: [&str; 4] = ["decode_tok_s", "prefill_tok_s", "decode_engine_tok_s", "prefill_engine_tok_s"];

/// Every engine the ledger names has a version and at least one row; every engine side keeps `repeats` runs on both
/// clocks, names its checkpoint — a directory of the model's own under `models/<model>/engines/`, its weight files'
/// bytes (recomputed wherever the checkpoint is on this machine) and digest — and runs the version the ledger names.
#[test]
fn every_engine_side_keeps_every_run_on_both_clocks_and_names_its_checkpoint() {
    let l = ledger();
    let repeats = l["protocol"]["repeats"].as_u64().unwrap() as usize;
    assert!(l["protocol"]["engines"].as_str().is_some_and(|p| !p.is_empty()), "the engines' protocol is written down");
    let named = l["engines"].as_object().expect("the engines the ledger measured");
    for (engine, e) in named {
        assert!(ENGINES.contains(&engine.as_str()), "an engine of the scoreboard's: {engine}");
        assert!(e["version"].as_str().is_some_and(|v| !v.is_empty()), "{engine}: its version");
    }
    let mut measured = BTreeSet::new();
    for r in l["rows"].as_array().unwrap() {
        let m = r["model"].as_str().unwrap();
        for engine in ENGINES {
            let Some(side) = r.get(engine) else { continue };
            measured.insert(engine.to_string());
            let what = format!("{m} {engine}");
            for key in ENGINE_RUNS {
                assert_eq!(runs(&side[key], &format!("{what} {key}")).len(), repeats, "{what} {key}");
            }
            assert_eq!(side["version"], named[engine]["version"], "{what}: the version the ledger names");
            let ckpt = side["checkpoint"].as_str().unwrap_or_else(|| panic!("{what}: its checkpoint"));
            assert!(ckpt.starts_with(&format!("models/{m}/engines/")), "{what}: the model's own checkpoint: {ckpt}");
            let digest = side["checkpoint_sha256"].as_str().unwrap_or_else(|| panic!("{what}: its checkpoint's digest"));
            assert!(digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit()), "{what}: a sha256: {digest}");
            let bytes = side["checkpoint_bytes"].as_u64().unwrap_or_else(|| panic!("{what}: its checkpoint's bytes"));
            let dir = root().join(ckpt);
            if dir.is_dir() {
                let on_disk: u64 = std::fs::read_dir(&dir)
                    .unwrap()
                    .flatten()
                    .filter(|e| e.path().extension().is_some_and(|x| x == "safetensors"))
                    .map(|e| e.metadata().unwrap().len())
                    .sum();
                assert_eq!(on_disk, bytes, "{what}: the weights measured are the ones on this machine");
            }
        }
    }
    let named: BTreeSet<String> = named.keys().cloned().collect();
    assert_eq!(measured, named, "every engine the ledger names measured a row, and only those");
}

/// What the ledger says of a model against an engine: ours over the engine's decode and prefill, mean over mean, the
/// engine's rate the faster of its two clocks' means — its benefit of the doubt.
pub fn engine_ratios(row: &Value, engine: &str) -> Option<(f64, f64)> {
    let side = row.get(engine)?;
    let best = |wall: &str, own: &str| mean(&runs(&side[wall], wall)).max(mean(&runs(&side[own], own)));
    let ours_d = mean(&runs(&row["ours"]["short"]["decode_tok_s"], "ours decode"));
    let ours_p = mean(&runs(&row["ours"]["long"]["prefill_tok_s"], "ours prefill"));
    Some((ours_d / best("decode_tok_s", "decode_engine_tok_s"), ours_p / best("prefill_tok_s", "prefill_engine_tok_s")))
}

#[test]
fn every_engine_ratio_is_derived_from_the_runs() {
    for r in ledger()["rows"].as_array().unwrap() {
        for engine in ENGINES {
            if let Some((d, p)) = engine_ratios(r, engine) {
                assert!(d.is_finite() && d > 0.0 && p.is_finite() && p > 0.0, "{} {engine}: {d} {p}", r["model"]);
                assert!(r[engine].get("ratio").is_none() && r[engine].get("speedup").is_none(), "{} {engine}: a ratio is computed, never stored", r["model"]);
            }
        }
    }
}

#[test]
fn an_engines_rate_is_the_faster_of_its_two_clocks() {
    let row = serde_json::json!({
        "ours": {"short": {"decode_tok_s": [200.0, 200.0]}, "long": {"prefill_tok_s": [1000.0, 1000.0]}},
        "vllm": {"decode_tok_s": [100.0, 100.0], "decode_engine_tok_s": [125.0, 125.0], "prefill_tok_s": [500.0, 500.0], "prefill_engine_tok_s": [400.0, 400.0]}
    });
    assert_eq!(engine_ratios(&row, "vllm"), Some((1.6, 2.0)), "200 over the engine clock's 125; 1000 over the wall's 500");
    assert_eq!(engine_ratios(&row, "exllamav3"), None, "no side, no ratio");
}

/// The sequence counts every batch row measures, where the model's batch of them fits the card.
const BATCHES: [usize; 3] = [4, 16, 64];

/// The batch rows: the protocol is written down and names its sequence counts — at least `BATCHES` — and every
/// count is measured by some model. Each of a row's batches keeps `repeats` runs of our batch step's decode rate
/// (`model_lane --bench … --batch=<B>`, every sequence's tokens a second): the compiled lane, its programs naming the
/// model's batch cut of that many sequences, recomputed from the checkpoint wherever it is on this machine. Where the
/// model has a vLLM checkpoint, vLLM at the same concurrency keeps `repeats` runs on the wall's clock and its own, its
/// version the ledger's and its checkpoint the row's.
#[test]
fn every_batch_row_runs_the_lanes_batch_step_beside_vllm_at_the_same_concurrency() {
    let l = ledger();
    let repeats = l["protocol"]["repeats"].as_u64().unwrap() as usize;
    assert!(l["protocol"]["batch"].as_str().is_some_and(|p| !p.is_empty()), "the batch protocol is written down");
    let counts: Vec<usize> = l["protocol"]["batch_sequences"]
        .as_array()
        .expect("the batch protocol's sequence counts")
        .iter()
        .map(|c| c.as_u64().unwrap() as usize)
        .collect();
    assert!(BATCHES.iter().all(|b| counts.contains(b)), "the protocol measures {BATCHES:?}: {counts:?}");
    let compiled = header(Provenance::Compiled);
    let mut measured = BTreeSet::new();
    for r in l["rows"].as_array().unwrap() {
        let m = r["model"].as_str().unwrap();
        let Some(batches) = r["batches"].as_array() else { continue };
        let file = root().join(r["file"].as_str().unwrap());
        let spec = file.is_file().then(|| ModelSpec::from_gguf(&Gguf::open(&file).unwrap()).unwrap());
        for b in batches {
            let n = b["sequences"].as_u64().unwrap() as usize;
            assert!(counts.contains(&n), "{m}: {n} sequences is a count of the protocol's");
            measured.insert(n);
            let what = format!("{m} at {n}");
            let o = &b["ours"];
            assert_eq!(o["provenance"].as_str().unwrap(), compiled.trim_end(), "{what}: the compiled lane");
            assert_eq!(runs(&o["decode_tok_s"], &format!("{what} ours")).len(), repeats, "{what}: our runs");
            let programs: Vec<&str> = o["programs"].as_array().unwrap().iter().map(|p| p.as_str().unwrap()).collect();
            let ctx = o["ctx"].as_u64().unwrap() as usize;
            assert!(
                programs.iter().any(|p| p.starts_with(&format!("LOGOS program: batch of {n} sequences, ctx {ctx}, sha256 "))),
                "{what}: the batch cut among the programs: {programs:?}"
            );
            if let Some(spec) = &spec {
                let batch = written(spec, Cut::Batch { rows: n, ctx }).1.line();
                assert!(programs.contains(&batch.as_str()), "{what}: the batch step is the one program's batch cut");
            }
            if let Some(v) = b.get("vllm") {
                for key in ["decode_tok_s", "decode_engine_tok_s"] {
                    assert_eq!(runs(&v[key], &format!("{what} vllm {key}")).len(), repeats, "{what} vllm {key}");
                }
                assert_eq!(v["version"], l["engines"]["vllm"]["version"], "{what}: the version the ledger names");
                assert_eq!(v["checkpoint"], r["vllm"]["checkpoint"], "{what}: the row's vLLM checkpoint");
                assert!(v.get("ratio").is_none() && v.get("speedup").is_none(), "{what}: a ratio is computed, never stored");
            }
        }
    }
    assert!(BATCHES.iter().all(|b| measured.contains(b)), "some model measured at each of {BATCHES:?}: {measured:?}");
}

/// What the ledger says of a batch against vLLM: our decode rate over vLLM's at the same concurrency, mean over mean,
/// vLLM's the faster of its two clocks' means.
pub fn batch_ratio(batch: &Value) -> Option<f64> {
    let v = batch.get("vllm")?;
    let best = mean(&runs(&v["decode_tok_s"], "vllm wall")).max(mean(&runs(&v["decode_engine_tok_s"], "vllm engine")));
    Some(mean(&runs(&batch["ours"]["decode_tok_s"], "ours")) / best)
}

#[test]
fn a_batchs_ratio_is_ours_over_vllms_faster_clock() {
    let b = serde_json::json!({
        "ours": {"decode_tok_s": [3000.0, 3000.0]},
        "vllm": {"decode_tok_s": [2000.0, 2000.0], "decode_engine_tok_s": [2500.0, 2500.0]}
    });
    assert_eq!(batch_ratio(&b), Some(1.2), "3000 over the engine clock's 2500");
    assert_eq!(batch_ratio(&serde_json::json!({"ours": {"decode_tok_s": [1.0, 1.0]}})), None, "no vLLM side, no ratio");
}

#[test]
fn the_statistics_are_the_ones_llama_bench_reports() {
    let v = [100.0, 102.0, 98.0, 101.0, 99.0];
    assert!((mean(&v) - 100.0).abs() < 1e-12);
    assert!((stddev(&v) - 1.5811388300841898).abs() < 1e-12, "the sample deviation, n - 1");
}

/// A stand-in `nvidia-smi` in a scratch directory, answering the one query the scoreboard asks of the card as the real
/// tool does — `pid, process_name, used_memory` a line, no header, no units, nothing when the card is idle — and
/// refusing any other as the real one refuses (its message on stdout, exit 2). It reports:
///   * a foreign process (pid `$FOREIGN_PID`, `/opt/other/bin/peer`, 900 MiB) on the next N queries, N read from
///     `foreign` and counted down, a query a time;
///   * the process whose pid is in `ours.pid`, while that file is there (a side's own GPU process).
/// Each query appends `q` to `events`, so a side's own `ran` lines there are ordered against the queries.
fn the_card(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("model_scoreboard_card_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let smi = dir.join("nvidia-smi");
    std::fs::write(
        &smi,
        "#!/bin/sh\n\
         if [ \"$*\" != '--query-compute-apps=pid,process_name,used_memory --format=csv,noheader,nounits' ]; then\n\
         \x20 printf 'Field \"%s\" is not a valid field to query.\\n\\n' \"$1\"; exit 2\n\
         fi\n\
         echo q >> \"$CARD/events\"\n\
         n=$(cat \"$CARD/foreign\" 2>/dev/null || echo 0)\n\
         if [ \"$n\" -gt 0 ]; then echo \"$FOREIGN_PID, /opt/other/bin/peer, 900\"; echo $((n - 1)) > \"$CARD/foreign\"; fi\n\
         if [ -f \"$CARD/ours.pid\" ]; then echo \"$(cat \"$CARD/ours.pid\"), /usr/bin/python3, 8900\"; fi\n\
         exit 0\n",
    )
    .expect("write the stand-in nvidia-smi");
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&smi, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    dir
}

/// Run `cmd` (a shell line) as one side through the scoreboard's `run`, against the stand-in card in `dir`, sampling
/// the card every 50 ms and waiting at most `wait_s` for it: `Ok(stdout)` or `Err(the refusal)`, and the side's log.
fn a_side(dir: &Path, cmd: &str, wait_s: f64) -> (Result<String, String>, String) {
    let log = dir.join("side.log");
    let o = std::process::Command::new("/usr/bin/python3")
        .args([
            "-c",
            "import json, sys; sys.path.insert(0, 'scripts/accel'); import model_scoreboard as s\n\
             s.CARD_POLL_S = 0.05; s.CARD_WAIT_S = float(sys.argv[3])\n\
             with open(sys.argv[1], 'w') as log:\n\
             \x20   try: print(json.dumps({'ok': s.run(['/bin/sh', '-c', sys.argv[2]], log, 60)}))\n\
             \x20   except s.Refused as e: print(json.dumps({'refused': str(e)}))\n",
            &log.display().to_string(),
            cmd,
            &wait_s.to_string(),
        ])
        .current_dir(root())
        .env("PATH", format!("{}:/usr/bin:/bin", dir.display()))
        .env("CARD", dir)
        .env("FOREIGN_PID", std::process::id().to_string())
        .output()
        .expect("launch python3");
    assert!(o.status.success(), "run raised:\n{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
    let v: Value = serde_json::from_slice(&o.stdout).unwrap_or_else(|e| panic!("{e}: {}", String::from_utf8_lossy(&o.stdout)));
    let result = match (v.get("ok"), v.get("refused")) {
        (Some(ok), None) => Ok(ok.as_str().expect("stdout").to_string()),
        (None, Some(r)) => Err(r.as_str().expect("refusal").to_string()),
        _ => panic!("neither ok nor refused: {v}"),
    };
    (result, std::fs::read_to_string(&log).unwrap_or_default())
}

fn events(dir: &Path) -> Vec<String> {
    std::fs::read_to_string(dir.join("events")).unwrap_or_default().lines().map(str::to_string).collect()
}

/// A side starts only once no other process holds the card: the scoreboard times nothing beside a peer's GPU job, and
/// its log names whoever it waited for (pid, binary, memory), so a run that waited says on whom.
#[test]
fn a_side_starts_only_once_no_other_process_holds_the_card() {
    let dir = the_card("waits");
    std::fs::write(dir.join("foreign"), "3").unwrap();
    let (result, log) = a_side(&dir, "echo ran >> \"$CARD/events\"; echo out", 30.0);
    assert_eq!(result, Ok("out\n".to_string()), "{log}");
    let ev = events(&dir);
    let ran = ev.iter().position(|e| e == "ran").unwrap_or_else(|| panic!("the side never ran: {ev:?}"));
    assert!(ran >= 4, "three queries saw the peer and a fourth the card free before the side ran: {ev:?}");
    let pid = std::process::id().to_string();
    assert!(
        log.contains(&pid) && log.contains("/opt/other/bin/peer") && log.contains("900 MiB"),
        "the log names the process the side waited for: {log}"
    );
}

/// A peer that joins the card while a side runs voids that side: its numbers shared the card, so it runs again once
/// the card is free, and what the scoreboard keeps is the clean run's. The log names the process that joined.
#[test]
fn a_side_another_process_joined_runs_again_once_the_card_is_free() {
    let dir = the_card("joins");
    let cmd = "echo ran >> \"$CARD/events\"; \
               if [ ! -f \"$CARD/joined\" ]; then touch \"$CARD/joined\"; echo 2 > \"$CARD/foreign\"; sleep 0.6; echo first; \
               else sleep 0.3; echo second; fi";
    let (result, log) = a_side(&dir, cmd, 30.0);
    assert_eq!(result, Ok("second\n".to_string()), "the clean run's output is the side's: {log}");
    assert_eq!(events(&dir).iter().filter(|e| *e == "ran").count(), 2, "run once, joined, run again: {log}");
    assert!(
        log.contains(&std::process::id().to_string()) && log.contains("/opt/other/bin/peer"),
        "the log names the process that joined: {log}"
    );
}

/// A side's own GPU processes, its child and every process under it (vLLM's engine core is a grandchild), are the side
/// itself, never a peer: the side runs once.
#[test]
fn a_sides_own_processes_on_the_card_are_not_a_peer() {
    let dir = the_card("ours");
    let cmd = "echo ran >> \"$CARD/events\"; \
               sh -c 'echo $$ > \"$CARD/ours.pid\"; sleep 0.5'; rm -f \"$CARD/ours.pid\"; echo done";
    let (result, log) = a_side(&dir, cmd, 30.0);
    assert_eq!(result, Ok("done\n".to_string()), "{log}");
    assert_eq!(events(&dir).iter().filter(|e| *e == "ran").count(), 1, "one run: {log}");
    assert!(events(&dir).iter().filter(|e| *e == "q").count() >= 3, "the card was sampled while the side ran: {:?}", events(&dir));
}

/// A card a peer holds past the wait is refused, nothing run, the refusal naming the holder; a side that fails on a
/// card it had to itself is refused as before, by its exit status.
#[test]
fn a_card_held_past_the_wait_refuses_and_a_side_that_fails_alone_still_refuses() {
    let dir = the_card("held");
    std::fs::write(dir.join("foreign"), "100000").unwrap();
    let (result, log) = a_side(&dir, "echo ran >> \"$CARD/events\"", 0.3);
    let refusal = result.expect_err("a card held past the wait is refused");
    assert!(
        refusal.contains(&std::process::id().to_string()) && refusal.contains("/opt/other/bin/peer") && refusal.contains("900 MiB"),
        "the refusal names the holder: {refusal}"
    );
    assert!(!events(&dir).iter().any(|e| e == "ran"), "nothing ran: {log}");

    let dir = the_card("fails");
    let (result, log) = a_side(&dir, "echo ran >> \"$CARD/events\"; exit 3", 30.0);
    let refusal = result.expect_err("a failing side is refused");
    assert!(refusal.contains("exited 3"), "refused by its exit status: {refusal}");
    assert_eq!(events(&dir).iter().filter(|e| *e == "ran").count(), 1, "a failure on a card it had alone is not rerun: {log}");
}

/// A stand-in `cargo` in a scratch directory that records its arguments in `cargo-args` and prints `messages` on stdout,
/// as `cargo build --message-format=json…` prints one JSON message a line.
fn a_cargo(tag: &str, messages: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("model_scoreboard_cargo_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    std::fs::write(dir.join("messages"), messages).expect("write the messages");
    let cargo = dir.join("cargo");
    std::fs::write(&cargo, "#!/bin/sh\necho \"$*\" > \"$CARD/cargo-args\"\ncat \"$CARD/messages\"\n").expect("write the stand-in cargo");
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&cargo, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    dir
}

/// The scoreboard's `build_lane` against the stand-in `cargo` in `dir`: `Ok(the lane it times)` or `Err(the refusal)`.
fn the_lane(dir: &Path) -> Result<String, String> {
    let o = std::process::Command::new("/usr/bin/python3")
        .args([
            "-c",
            "import json, sys; sys.path.insert(0, 'scripts/accel'); import model_scoreboard as s\n\
             with open(sys.argv[1], 'w') as log:\n\
             \x20   try: print(json.dumps({'ok': s.build_lane(log)}))\n\
             \x20   except s.Refused as e: print(json.dumps({'refused': str(e)}))\n",
            &dir.join("build.log").display().to_string(),
        ])
        .current_dir(root())
        .env("PATH", format!("{}:/usr/bin:/bin", dir.display()))
        .env("CARD", dir)
        .output()
        .expect("launch python3");
    assert!(o.status.success(), "build_lane raised:\n{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
    let v: Value = serde_json::from_slice(&o.stdout).unwrap_or_else(|e| panic!("{e}: {}", String::from_utf8_lossy(&o.stdout)));
    match (v.get("ok"), v.get("refused")) {
        (Some(ok), None) => Ok(ok.as_str().expect("a path").to_string()),
        (None, Some(r)) => Err(r.as_str().expect("refusal").to_string()),
        _ => panic!("neither ok nor refused: {v}"),
    }
}

/// The lane the scoreboard times is the executable its build wrote: the one `cargo build` reports in its
/// `compiler-artifact` message for `model_lane` — not another executable the build reports — wherever `CARGO_TARGET_DIR`
/// or the cargo configuration put it; a path assumed instead would time whatever binary last sat there.
#[test]
fn the_lane_timed_is_the_executable_its_build_reports() {
    let dir = a_cargo("reports", "");
    let lane = dir.join("elsewhere").join("release").join("examples").join("model_lane");
    let messages = [
        json!({"reason": "compiler-artifact", "target": {"name": "logicaffeine_compile", "kind": ["lib"]}, "executable": null}),
        json!({"reason": "compiler-artifact", "target": {"name": "largo", "kind": ["bin"]}, "executable": dir.join("largo").display().to_string()}),
        json!({"reason": "compiler-artifact", "target": {"name": "model_lane", "kind": ["example"]}, "executable": lane.display().to_string()}),
        json!({"reason": "build-finished", "success": true}),
    ]
    .iter()
    .map(|m| format!("{m}\n"))
    .collect::<String>();
    std::fs::write(dir.join("messages"), messages).unwrap();
    assert_eq!(the_lane(&dir), Ok(lane.display().to_string()));
    let args = std::fs::read_to_string(dir.join("cargo-args")).expect("cargo ran");
    for want in ["build", "--release", "-p logicaffeine-compile", "--example model_lane", "--message-format=json"] {
        assert!(args.contains(want), "the build asks for `{want}`: {args}");
    }
}

/// A build that reports no `model_lane` executable is refused: the scoreboard never times a binary its build did not
/// name.
#[test]
fn a_build_that_reports_no_lane_is_refused() {
    let lib = json!({"reason": "compiler-artifact", "target": {"name": "logicaffeine_compile", "kind": ["lib"]}, "executable": null});
    let dir = a_cargo("none", &format!("{lib}\n{}\n", json!({"reason": "build-finished", "success": true})));
    let refusal = the_lane(&dir).expect_err("no lane reported");
    assert!(refusal.contains("model_lane"), "the refusal names the missing executable: {refusal}");
}
