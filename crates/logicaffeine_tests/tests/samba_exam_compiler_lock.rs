//! samba_exam_compiler_lock — the rows of the Samba book's exam E8 (section 10.8 of `benchmarks/PREPARE_FOR_SAMBA.md`,
//! the compiler's own quality) that hold today.
//!
//! A row emits the SN50 bundle `largo emit rdu` writes for a tiny checkpoint of each shared architecture and reads what is
//! on disk: the manifest, and a directory of files a program (the embedding, each layer kind, the head).
//!
//! Run: `cargo nextest run -p logicaffeine-tests --cargo-profile test-opt --test samba_exam_compiler_lock`

#![cfg(feature = "codegen")]

#[path = "cli_common/mod.rs"]
mod cli;
#[path = "common/tiny_models.rs"]
mod tiny_models;
#[path = "common/samba_handoff.rs"]
#[allow(dead_code)]
mod samba_handoff;

use samba_handoff::{Model, CTX};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use tiny_models::Tiny;

/// The architectures every row covers: attention alone, a hybrid with recurrent state, windowed attention, and an
/// expert mixture with sinks.
const SHARED: [Tiny; 4] = [Tiny::Qwen3, Tiny::Qwen35, Tiny::Gemma3, Tiny::GptOss];

/// `largo emit rdu --chip sn50 --model <checkpoint> --ctx CTX -o <out>`: every file of the bundle by its path under `out`.
fn emit(m: &Model, out: &Path) -> BTreeMap<String, Vec<u8>> {
    std::fs::create_dir_all(out.parent().expect("a parent")).unwrap();
    let o = cli::largo_in(
        out.parent().unwrap(),
        &["emit", "rdu", "--chip", "sn50", "--model", m.path.to_str().unwrap(), "--ctx", &CTX.to_string(), "-o", out.to_str().unwrap()],
    );
    assert_eq!(o.status.code(), Some(0), "largo emit rdu failed:\n{}", cli::stderr(&o));
    let mut files = BTreeMap::new();
    let mut dirs = vec![out.to_path_buf()];
    while let Some(d) = dirs.pop() {
        for e in std::fs::read_dir(&d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                dirs.push(p);
            } else {
                files.insert(p.strip_prefix(out).unwrap().display().to_string(), std::fs::read(&p).unwrap());
            }
        }
    }
    files
}

fn manifest(files: &BTreeMap<String, Vec<u8>>) -> Value {
    serde_json::from_slice(files.get("manifest.json").expect("the bundle has a manifest")).expect("the manifest is JSON")
}

/// E8.2 — one program a layer kind: a checkpoint of 64 layers, its architecture's layers repeated, compiles each kind once.
/// Its bundle holds the files the architecture's own few layers do, its manifest the same kinds, and each of its layers runs
/// the program of the few layers' layer it equals.
#[test]
fn layers_of_one_kind_compile_once() {
    let dir = tempfile::tempdir().unwrap();
    for t in SHARED {
        let base = t.spec(CTX);
        let few = base.layers.len();
        let mut spec = base.clone();
        spec.layers = (0..64).map(|l| base.layers[l % few].clone()).collect();
        let (small, large) = (Model::write(base, &format!("{t:?}")), Model::write(spec, &format!("{t:?}-64")));
        let (s, l) = (emit(&small, &dir.path().join(format!("{t:?}-few"))), emit(&large, &dir.path().join(format!("{t:?}-64"))));
        let (ms, ml) = (manifest(&s), manifest(&l));
        assert_eq!(ml["model"]["layers"], 64, "{t:?}: the checkpoint has 64 layers");
        let names = |m: &Value| -> Vec<String> { m["kinds"].as_array().expect("kinds").iter().map(|k| k["name"].as_str().unwrap().to_string()).collect() };
        assert!(!names(&ms).is_empty() && names(&ms).len() <= few, "{t:?}: {few} layers are at most {few} kinds: {:?}", names(&ms));
        assert_eq!(names(&ml), names(&ms), "{t:?}: 64 layers are the kinds their {few} are");
        assert_eq!(
            l.keys().collect::<BTreeSet<_>>(),
            s.keys().collect::<BTreeSet<_>>(),
            "{t:?}: the 64-layer bundle's files are the {few}-layer bundle's — a program a kind, not a layer"
        );
        let layers = ml["layers"].as_array().expect("layers");
        assert_eq!(layers.len(), 64, "{t:?}: every layer is bound");
        for (i, layer) in layers.iter().enumerate() {
            assert_eq!(layer["layer"], i, "{t:?}: the layers in order");
            // A layer as the checkpoint reads it back: gpt-oss's windowed and full layers alternate by index.
            let j = small.spec.layers.iter().position(|s| *s == large.spec.layers[i]).unwrap_or_else(|| panic!("{t:?}: layer {i} is a layer the {few}-layer checkpoint holds"));
            assert_eq!(layer["kind"], ms["layers"][j]["kind"], "{t:?}: layer {i} runs the kind of the few layers' layer {j}, its equal");
        }
    }
}

/// E8.3 — one checkpoint compiled twice, into two directories, gives the same bundle byte for byte: every file, every
/// program, every certificate and the manifest.
#[test]
fn a_checkpoint_compiles_to_identical_bytes() {
    let dir = tempfile::tempdir().unwrap();
    for t in SHARED {
        let m = Model::write(t.spec(CTX), &format!("{t:?}"));
        let a = emit(&m, &dir.path().join(format!("{t:?}-a")));
        let b = emit(&m, &dir.path().join("elsewhere").join(format!("{t:?}-b")));
        assert!(a.len() >= 4, "{t:?}: a manifest and a program's files at least: {:?}", a.keys().collect::<Vec<_>>());
        assert_eq!(a.keys().collect::<Vec<_>>(), b.keys().collect::<Vec<_>>(), "{t:?}: the same files");
        for (path, bytes) in &a {
            assert!(bytes == &b[path], "{t:?}: `{path}` differs between two compiles of one checkpoint");
        }
    }
}
