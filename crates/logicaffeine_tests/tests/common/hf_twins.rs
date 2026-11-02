//! The Hugging Face intake's oracle: a model family's tiny random checkpoint and the GGUF llama.cpp's converter writes
//! from it (`scripts/accel/hf-tiny-twin.py`), built once into `target/hf-twins/<family>/` and rebuilt when the script
//! changes. Callers gate on `common::require_tool("hf-twins")`.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn script() -> PathBuf {
    root().join("scripts/accel/hf-tiny-twin.py")
}

/// The SHA-256 of the twin script, which a twin's `STAMP` records.
fn script_digest() -> String {
    let out = Command::new("sha256sum").arg(script()).output().expect("sha256sum");
    String::from_utf8_lossy(&out.stdout).split_whitespace().next().unwrap_or_default().to_string()
}

fn current(dir: &Path, digest: &str) -> bool {
    std::fs::read_to_string(dir.join("STAMP")).is_ok_and(|s| s.contains(digest))
        && dir.join("model-f32.gguf").is_file()
        && dir.join("hf/config.json").is_file()
}

/// An exclusive lock on building `what` under `dir`, held until it is dropped: test processes running at once build a
/// twin or a conversion once, the rest waiting and then finding it current.
fn build_lock(dir: &Path, what: &str) -> std::fs::File {
    std::fs::create_dir_all(dir).expect("the build directory");
    let f = std::fs::File::create(dir.join(format!(".{what}.lock"))).expect("the build lock");
    f.lock().expect("lock the build");
    f
}

/// The family's twin: `(<dir>/hf, <dir>/model-f32.gguf)`. Built by the twin script when absent or stale, by one process
/// at a time; a build runs in a directory of its own and is renamed into place, so no test reads a half-written twin.
pub fn twin(family: &str) -> (PathBuf, PathBuf) {
    let dir = root().join("target/hf-twins").join(family);
    let digest = script_digest();
    let _lock = build_lock(&root().join("target/hf-twins"), family);
    if !current(&dir, &digest) {
        let staging = root().join(format!("target/hf-twins/.{family}.{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&staging);
        std::fs::create_dir_all(&staging).expect("twin staging directory");
        let out = Command::new(super::python_for_tools())
            .arg(script())
            .arg(family)
            .arg(&staging)
            .output()
            .expect("run the twin script");
        assert!(
            out.status.success(),
            "the {family} twin did not build:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        if !current(&dir, &digest) {
            let _ = std::fs::remove_dir_all(&dir);
            if std::fs::rename(&staging, &dir).is_err() {
                assert!(current(&dir, &digest), "the {family} twin could not be placed");
            }
        }
        let _ = std::fs::remove_dir_all(&staging);
    }
    (dir.join("hf"), dir.join("model-f32.gguf"))
}

/// A published checkpoint the golden lock holds (`models/<model>/hf`, the directory as its maintainers publish it) and
/// the GGUF the pinned converter writes from it (`--outtype bf16`, the checkpoint's own dtype), built once into
/// `target/hf-published/<model>/` and rebuilt when the directory's configuration or the converter changes.
pub fn published(model: &str) -> (PathBuf, PathBuf) {
    let dir = root().join("models").join(model).join("hf");
    let out = root().join("target/hf-published").join(model);
    let gguf = out.join("model-bf16.gguf");
    let converter = root().join("external/competitors/llama.cpp/convert_hf_to_gguf.py");
    let sums = Command::new("sha256sum").arg(dir.join("config.json")).arg(&converter).output().expect("sha256sum");
    let stamp: String = String::from_utf8_lossy(&sums.stdout).split_whitespace().step_by(2).collect::<Vec<_>>().join(" ");
    let _lock = build_lock(&root().join("target/hf-published"), model);
    let current = std::fs::read_to_string(out.join("STAMP")).is_ok_and(|s| s == stamp) && gguf.is_file();
    if !current {
        let staging = root().join(format!("target/hf-published/.{model}.{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&staging);
        std::fs::create_dir_all(&staging).expect("conversion staging directory");
        let result = Command::new(super::python_for_tools())
            .arg(&converter)
            .arg(&dir)
            .args(["--outtype", "bf16", "--outfile"])
            .arg(staging.join("model-bf16.gguf"))
            .output()
            .expect("run the converter");
        assert!(result.status.success(), "the {model} conversion failed:\n{}", String::from_utf8_lossy(&result.stderr));
        std::fs::write(staging.join("STAMP"), &stamp).unwrap();
        let _ = std::fs::remove_dir_all(&out);
        std::fs::create_dir_all(out.parent().unwrap()).unwrap();
        if std::fs::rename(&staging, &out).is_err() {
            assert!(gguf.is_file(), "the {model} conversion could not be placed");
            let _ = std::fs::remove_dir_all(&staging);
        }
    }
    (dir, gguf)
}

/// Transformers' own forward of the checkpoint in `dir` over `prompt` at f64, one pass
/// (`scripts/accel/hf-oracle.py logits-at`): the logits at each of `positions`, each a prefix's own.
pub fn transformers_logits_at(dir: &Path, prompt: &[u32], positions: &[usize]) -> Vec<Vec<f64>> {
    let scratch = root().join("target/hf-twins/scratch");
    std::fs::create_dir_all(&scratch).expect("the oracle's scratch directory");
    let input = serde_json::json!({ "tokens": prompt, "positions": positions }).to_string();
    let path = scratch.join(format!("oracle-logits-at-{}-{}.json", std::process::id(), input.len()));
    std::fs::write(&path, &input).expect("the oracle's input");
    let out = Command::new(super::python_for_tools())
        .arg(root().join("scripts/accel/hf-oracle.py"))
        .arg("logits-at")
        .arg(dir)
        .arg(&path)
        .output()
        .expect("run the oracle");
    let _ = std::fs::remove_file(&path);
    assert!(out.status.success(), "the oracle failed: {}", String::from_utf8_lossy(&out.stderr));
    let rows: serde_json::Value = serde_json::from_slice(&out.stdout).expect("the oracle's JSON");
    rows.as_array()
        .expect("a row a position")
        .iter()
        .map(|r| r.as_array().expect("the logits").iter().map(|v| v.as_f64().expect("a logit")).collect())
        .collect()
}

/// Transformers' own forward of the checkpoint in `dir` over `prompt` at f64 (`scripts/accel/hf-oracle.py logits`): the
/// last position's logits.
pub fn transformers_logits(dir: &Path, prompt: &[u32]) -> Vec<f64> {
    let scratch = root().join("target/hf-twins/scratch");
    std::fs::create_dir_all(&scratch).expect("the oracle's scratch directory");
    let input = serde_json::json!(prompt).to_string();
    let path = scratch.join(format!("oracle-logits-{}-{}.json", std::process::id(), input.len()));
    std::fs::write(&path, &input).expect("the oracle's input");
    let out = Command::new(super::python_for_tools())
        .arg(root().join("scripts/accel/hf-oracle.py"))
        .arg("logits")
        .arg(dir)
        .arg(&path)
        .output()
        .expect("run the oracle");
    let _ = std::fs::remove_file(&path);
    assert!(out.status.success(), "the oracle failed: {}", String::from_utf8_lossy(&out.stderr));
    let logits: serde_json::Value = serde_json::from_slice(&out.stdout).expect("the oracle's JSON");
    logits.as_array().expect("the logits").iter().map(|v| v.as_f64().expect("a logit")).collect()
}
