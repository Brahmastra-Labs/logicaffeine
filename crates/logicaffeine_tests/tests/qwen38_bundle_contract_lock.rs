//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! THE DECODE BUNDLE'S HOST CONTRACT — what a host must hand an RDU bundle is exactly what its manifest says.
//!
//! A `largo emit rdu --model … --ctx N` bundle (a decode step, a batch of them, tensor-parallel, a prefill chunk) is a
//! manifest binding every tensor of every program: to a file (the weights), to state the step carries, to what an
//! earlier program of the bundle wrote, or to a RUNTIME INPUT the host supplies each step. The manifest lists the
//! runtime inputs; nothing else may be left for the host to know.
//!
//! Pinned:
//!   1. Every input of every program of every decode form that no file backs (a whole file, or a chip's slice of one)
//!      is a listed runtime input or the output of an earlier program, and every listed runtime input is read — so the
//!      tokens and positions are the whole per-step interface, and nothing the host is asked for goes unread.
//!   2. The rotary angles are model constants in the bundle: `weights/Rcos.f` and `weights/Rsin.f`, `ctx` rows of
//!      `rope_dim / 2` little-endian f32, bound from those files wherever a program reads them (never as state or an
//!      activation), written with the bundle — and they are
//!      the ONLY rotary tables bound: no per-step table of one position's angles (`RC`, `RS_s1`, `RC_chunk`, …), which
//!      a host would have to recompute every step and which a file cannot hold.
//!   3. Their values are the CUDA tier's rotation: position `p` (1-based) turns pair `i` by `(p − 1)·θ^(−2i/rope_dim)`
//!      — `gk_qk_norm_rope_part` over its 0-based position — for the tiny and the real configuration.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test qwen38_bundle_contract_lock`
//!
//! ★ MUTATION PROBES (private copy, RED 2026-09-26): the CLI writes no constant → `weights/Rcos.f: No such file`; the
//! cache's angle counts positions from 1 → `tiny: cos at position 1, pair 0`.
//! ════════════════════════════════════════════════════════════════════════════════════════════════
#![cfg(not(target_arch = "wasm32"))]

#[path = "cli_common/mod.rs"]
mod common;

use common::*;
use logicaffeine_compile::codegen_cuda::qwen38::{qwen38_rope_cache, Qwen38Cfg};
use std::collections::BTreeSet;
use std::path::Path;
use tempfile::tempdir;

const CTX: usize = 8;

/// Every decode deployment form: `(model, flags, what)`.
fn forms() -> Vec<(&'static str, Vec<&'static str>, &'static str)> {
    vec![
        ("qwen38-tiny", vec![], "a decode step"),
        (
            "qwen38-tiny",
            vec!["--batch", "3"],
            "a batch of three steps",
        ),
        ("qwen38-mini", vec!["--tp", "4"], "a tensor-parallel step"),
        (
            "qwen38-mini",
            vec!["--tp", "4", "--batch", "2"],
            "a tensor-parallel batch",
        ),
        ("qwen38-tiny", vec!["--prefill", "3"], "a prefill chunk"),
    ]
}

fn emit(
    dir: &Path,
    model: &str,
    flags: &[&str],
    what: &str,
) -> (std::path::PathBuf, serde_json::Value) {
    let bundle = dir.join("bundle");
    let ctx = CTX.to_string();
    let mut args = vec!["emit", "rdu", "--model", model, "--ctx", &ctx];
    args.extend_from_slice(flags);
    args.extend(["-o", bundle.to_str().unwrap()]);
    let out = largo_in(dir, &args);
    assert_eq!(
        out.status.code(),
        Some(0),
        "{what}: largo {args:?} failed:\n{}",
        stderr(&out)
    );
    let manifest = serde_json::from_str(
        &std::fs::read_to_string(bundle.join("manifest.json")).expect("manifest.json"),
    )
    .expect("the manifest parses");
    (bundle, manifest)
}

/// Whether a file backs the binding: a whole file, or a chip's slice of one.
fn file_backed(e: &serde_json::Value) -> bool {
    !e["file"].is_null() || !e["slice"]["file"].is_null()
}

/// The manifest's programs' bindings in the order the host runs them: the embedding, every layer, the head.
fn bindings_in_order(manifest: &serde_json::Value) -> Vec<(String, Vec<serde_json::Value>)> {
    let tensors = |b: &serde_json::Value| {
        b["tensors"]
            .as_array()
            .expect("a program's tensors")
            .clone()
    };
    let mut v = vec![("embed".to_string(), tensors(&manifest["embed"]))];
    for layer in manifest["layers"].as_array().expect("layers") {
        v.push((format!("layer {}", layer["layer"]), tensors(layer)));
    }
    v.push(("head".to_string(), tensors(&manifest["head"])));
    v
}

#[test]
fn every_host_input_of_a_decode_bundle_is_a_listed_runtime_input() {
    for (model, flags, what) in forms() {
        let dir = tempdir().unwrap();
        let (_, manifest) = emit(dir.path(), model, &flags, what);
        let runtime: BTreeSet<String> = manifest["runtime_inputs"]
            .as_array()
            .unwrap_or_else(|| panic!("{what}: the manifest lists its runtime inputs"))
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();
        let mut written: BTreeSet<String> = BTreeSet::new();
        let mut unlisted = Vec::new();
        for (program, tensors) in bindings_in_order(&manifest) {
            for e in &tensors {
                let (name, role) = (
                    e["layer"].as_str().unwrap().to_string(),
                    e["role"].as_str().unwrap(),
                );
                if role == "input"
                    && !file_backed(e)
                    && !runtime.contains(&name)
                    && !written.contains(&name)
                {
                    unlisted.push(format!("{program}: `{name}`"));
                }
            }
            written.extend(
                tensors
                    .iter()
                    .filter(|e| e["role"] != "input" && e["role"] != "param")
                    .map(|e| e["layer"].as_str().unwrap().to_string()),
            );
        }
        unlisted.dedup();
        assert!(unlisted.is_empty(), "{what}: inputs the host must supply that the manifest does not list ({} of them): {:?}", unlisted.len(), &unlisted[..unlisted.len().min(12)]);
        let read: BTreeSet<String> = bindings_in_order(&manifest)
            .into_iter()
            .flat_map(|(_, t)| t)
            .filter(|e| e["role"] == "input")
            .map(|e| e["layer"].as_str().unwrap().to_string())
            .collect();
        let unread: Vec<&String> = runtime.iter().filter(|r| !read.contains(*r)).collect();
        assert!(
            unread.is_empty(),
            "{what}: runtime inputs no program reads: {unread:?}"
        );
    }
}

fn read_f32(p: &Path) -> Vec<f32> {
    let bytes = std::fs::read(p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
    assert_eq!(bytes.len() % 4, 0, "{}: whole f32s", p.display());
    bytes
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect()
}

#[test]
fn the_rotary_angles_ship_with_the_bundle_as_parameters() {
    for (model, flags, what) in forms() {
        let cfg = if model == "qwen38-mini" {
            Qwen38Cfg::mini()
        } else {
            Qwen38Cfg::tiny()
        };
        let dir = tempdir().unwrap();
        let (bundle, manifest) = emit(dir.path(), model, &flags, what);
        let (cos, sin) = qwen38_rope_cache(&cfg, CTX);
        for (name, want) in [("Rcos", &cos), ("Rsin", &sin)] {
            let got = read_f32(&bundle.join("weights").join(format!("{name}.f")));
            assert_eq!(
                got.len(),
                CTX * cfg.rope_dim / 2,
                "{what}: {name} is ctx rows of rope_dim / 2"
            );
            assert_eq!(
                got.iter().map(|x| x.to_bits()).collect::<Vec<_>>(),
                want.iter().map(|x| x.to_bits()).collect::<Vec<_>>(),
                "{what}: {name}.f holds the model's angles"
            );
            let programs = bindings_in_order(&manifest);
            let bound: Vec<&serde_json::Value> = programs
                .iter()
                .flat_map(|(_, t)| t.iter())
                .filter(|e| e["layer"] == name)
                .collect();
            assert!(
                !bound.is_empty(),
                "{what}: some attention program reads {name}"
            );
            for e in bound {
                assert_eq!(
                    e["file"].as_str(),
                    Some(format!("weights/{name}.f").as_str()),
                    "{what}: {name} is bound from its file"
                );
                assert!(
                    matches!(e["role"].as_str(), Some("input" | "param")),
                    "{what}: {name} is read, never carried or computed: {e}"
                );
            }
        }
        let per_step: Vec<String> = bindings_in_order(&manifest)
            .into_iter()
            .flat_map(|(program, t)| {
                t.into_iter()
                    .map(move |e| (program.clone(), e["layer"].as_str().unwrap().to_string()))
            })
            .filter(|(_, name)| {
                ["RC", "RS"]
                    .iter()
                    .any(|t| name == t || name.starts_with(&format!("{t}_")))
            })
            .map(|(program, name)| format!("{program}: `{name}`"))
            .collect();
        assert!(per_step.is_empty(), "{what}: a table of one position's angles is bound — the angles come from the caches alone: {:?}", &per_step[..per_step.len().min(8)]);
    }
}

/// `gk_qk_norm_rope_part`'s rotation at 0-based position `pos`, pair `i` of a `rot`-wide rotary slice, in f64.
fn cuda_angle(theta: f64, rot: usize, pos: usize, i: usize) -> f64 {
    pos as f64 * theta.powf(-2.0 * i as f64 / rot as f64)
}

#[test]
fn the_rope_cache_is_the_cuda_tiers_rotation() {
    for (cfg, rows, what) in [
        (Qwen38Cfg::tiny(), 8, "tiny"),
        (Qwen38Cfg::real(), 4096, "Qwen3.8-27B"),
    ] {
        let half = cfg.rope_dim / 2;
        let (cos, sin) = qwen38_rope_cache(&cfg, rows);
        assert_eq!(
            (cos.len(), sin.len()),
            (rows * half, rows * half),
            "{what}: rows × half, both"
        );
        for p in 1..=rows {
            for i in 0..half {
                let a = cuda_angle(cfg.rope_theta, cfg.rope_dim, p - 1, i);
                assert_eq!(
                    cos[(p - 1) * half + i],
                    a.cos() as f32,
                    "{what}: cos at position {p}, pair {i}"
                );
                assert_eq!(
                    sin[(p - 1) * half + i],
                    a.sin() as f32,
                    "{what}: sin at position {p}, pair {i}"
                );
            }
        }
        assert!(
            cos[..half].iter().all(|c| *c == 1.0) && sin[..half].iter().all(|s| *s == 0.0),
            "{what}: the first position does not turn"
        );
    }
}
