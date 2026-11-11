//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! rdu_drop_lock — a self-checking artifact drop (ACCESS-4 of `benchmarks/PREPARE_FOR_SAMBA.md`).
//!
//! `largo drop CHECKPOINT --ctx N -o DIR` writes the SN50 decode bundle, the checkpoint whose weights its manifest binds
//! (each read through the binding the bundle reads it by), the goldens of its prompts — the tokens our spatial evaluator
//! decodes from an empty state and the digest of their logits, beside the f64 reference's tokens — and `drop.json`, the
//! sha256 of every file. `largo check-drop DIR`, on a machine with only `largo`, checks every digest, rechecks every
//! program's certificate against the schedule digest the manifest records, and decodes the goldens again, bit for bit.
//!
//!   • a drop of a dense model and of Qwen 3.5's hybrid verifies itself from a copy elsewhere, its checkpoint gone;
//!   • one flipped byte in a weight, a program, a certificate or the manifest is refused, naming the file, and so is a
//!     missing or an extra file;
//!   • a program changed and digested again is refused by its certificate, so the recheck is not the digests' alone;
//!   • a drop written with `--fetch-weights` carries the checkpoint's digest in place of its weights and checks against
//!     the checkpoint it is given, refusing another.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --cargo-profile test-opt --test rdu_drop_lock`
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(feature = "codegen")]

#[path = "cli_common/mod.rs"]
mod cli;
#[path = "common/tiny_models.rs"]
mod tiny_models;
#[path = "common/samba_handoff.rs"]
#[allow(dead_code)]
mod samba_handoff;

use samba_handoff::{Model, CTX};
use std::path::{Path, PathBuf};
use tiny_models::Tiny;

/// A drop of `tiny` written into a directory of its own, from a checkpoint in another: the drop's directory, the
/// checkpoint's, and the model.
fn dropped(tiny: Tiny, extra: &[&str]) -> (tempfile::TempDir, PathBuf, Model) {
    let m = Model::write(tiny.spec(CTX), &format!("{tiny:?}"));
    let dir = tempfile::tempdir().unwrap();
    let drop = dir.path().join("drop");
    let mut args = vec!["drop", m.path.to_str().unwrap(), "--ctx", "8", "--prompt", "3,7,1", "--tokens", "3", "-o", drop.to_str().unwrap()];
    args.extend_from_slice(extra);
    let out = cli::largo_in(dir.path(), &args);
    assert_eq!(out.status.code(), Some(0), "{tiny:?}: largo drop failed:\n{}", cli::stderr(&out));
    (dir, drop, m)
}

/// A copy of `drop` in a fresh directory, by content.
fn copied(drop: &Path) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let to = dir.path().join("drop");
    let mut stack = vec![(drop.to_path_buf(), to.clone())];
    while let Some((from, into)) = stack.pop() {
        std::fs::create_dir_all(&into).unwrap();
        for e in std::fs::read_dir(&from).unwrap() {
            let p = e.unwrap().path();
            let q = into.join(p.file_name().unwrap());
            if p.is_dir() {
                stack.push((p, q));
            } else {
                std::fs::write(&q, std::fs::read(&p).unwrap()).unwrap();
            }
        }
    }
    (dir, to)
}

/// `largo check-drop DIR [args]`: its exit code, stdout and stderr.
fn check(drop: &Path, extra: &[&str]) -> (Option<i32>, String, String) {
    let mut args = vec!["check-drop", drop.to_str().unwrap()];
    args.extend_from_slice(extra);
    let out = cli::largo_in(drop.parent().unwrap(), &args);
    (out.status.code(), String::from_utf8_lossy(&out.stdout).into_owned(), cli::stderr(&out))
}

/// Every file of a drop, relative to it, sorted.
fn files(drop: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![drop.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                stack.push(p);
            } else {
                out.push(p.strip_prefix(drop).unwrap().to_string_lossy().into_owned());
            }
        }
    }
    out.sort();
    out
}

#[test]
fn a_drop_verifies_itself_with_only_its_runner() {
    for tiny in [Tiny::Qwen3, Tiny::Qwen35] {
        let (_d, drop, m) = dropped(tiny, &[]);
        let listed = files(&drop);
        assert!(listed.iter().any(|f| f == "drop.json"), "{tiny:?}: the drop's digests: {listed:?}");
        assert!(listed.iter().any(|f| f.starts_with("bundle/weights/") && f.ends_with(".gguf")), "{tiny:?}: the drop carries its weights: {listed:?}");
        assert!(listed.iter().any(|f| f.ends_with("certificate.json")), "{tiny:?}: the drop carries its certificates: {listed:?}");
        let (_c, copy) = copied(&drop);
        drop_checkpoint(m);
        let (code, stdout, stderr) = check(&copy, &[]);
        assert_eq!(code, Some(0), "{tiny:?}: the copy verifies itself:\n{stdout}\n{stderr}");
        assert!(stdout.contains("verified"), "{tiny:?}: check-drop says what it verified:\n{stdout}");
    }
}

/// The checkpoint gone from disk: what is checked from here on is the drop alone.
fn drop_checkpoint(m: Model) {
    let path = m.path.clone();
    drop(m);
    assert!(!path.exists(), "the checkpoint is gone");
}

#[test]
fn a_drop_with_one_flipped_byte_refuses_itself() {
    let (_d, drop, _m) = dropped(Tiny::Qwen3, &[]);
    let listed = files(&drop);
    let pick = |what: &str, f: &dyn Fn(&String) -> bool| listed.iter().find(|p| f(p)).unwrap_or_else(|| panic!("a {what} in {listed:?}")).clone();
    for (what, file) in [
        ("weight", pick("weight", &|p| p.starts_with("bundle/weights/"))),
        ("program", pick("program", &|p| p.ends_with("neutral.json"))),
        ("certificate", pick("certificate", &|p| p.ends_with("certificate.json"))),
        ("manifest", "bundle/manifest.json".to_string()),
    ] {
        let (_c, copy) = copied(&drop);
        let path = copy.join(&file);
        let mut bytes = std::fs::read(&path).unwrap();
        let at = bytes.len() / 2;
        bytes[at] ^= 0x01;
        std::fs::write(&path, bytes).unwrap();
        let (code, stdout, stderr) = check(&copy, &[]);
        assert_ne!(code, Some(0), "a flipped byte in the {what} `{file}` is refused:\n{stdout}");
        assert!(stderr.contains(&file), "the refusal names the {what} `{file}`:\n{stderr}");
    }
    let (_c, copy) = copied(&drop);
    let gone = pick("weight", &|p| p.starts_with("bundle/weights/"));
    std::fs::remove_file(copy.join(&gone)).unwrap();
    let (code, _, stderr) = check(&copy, &[]);
    assert_ne!(code, Some(0), "a missing file is refused");
    assert!(stderr.contains(&gone), "the refusal names the missing `{gone}`:\n{stderr}");
    let (_c, copy) = copied(&drop);
    std::fs::write(copy.join("bundle/stray.bin"), b"not part of the drop").unwrap();
    let (code, _, stderr) = check(&copy, &[]);
    assert_ne!(code, Some(0), "an extra file is refused");
    assert!(stderr.contains("bundle/stray.bin"), "the refusal names the extra file:\n{stderr}");
}

/// The first unsigned number in `v`, depth first.
fn first_number(v: &mut serde_json::Value) -> Option<&mut serde_json::Value> {
    if v.as_u64().is_some() {
        return Some(v);
    }
    match v {
        serde_json::Value::Array(items) => items.iter_mut().find_map(first_number),
        serde_json::Value::Object(fields) => fields.values_mut().find_map(first_number),
        _ => None,
    }
}

#[test]
fn a_program_changed_and_digested_again_is_refused_by_its_certificate() {
    // The digests are the drop's own record; the certificate is the program's. A program edited and its digest written
    // again passes the first and fails the second.
    let (_d, drop, _m) = dropped(Tiny::Qwen3, &[]);
    let (_c, copy) = copied(&drop);
    let program = files(&copy).into_iter().find(|p| p.ends_with("neutral.json")).expect("a program");
    let path = copy.join(&program);
    let text = std::fs::read_to_string(&path).unwrap();
    // The first number of the program's schedule moved by one: the choreography its certificate binds is another.
    let mut parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
    let leaf = first_number(&mut parsed["program"]["schedule"]).unwrap_or_else(|| panic!("`{program}`'s schedule holds a number"));
    *leaf = serde_json::json!(leaf.as_u64().expect("a count") + 1);
    let changed = serde_json::to_string(&parsed).unwrap();
    assert_ne!(changed, text, "the edit changes `{program}`");
    std::fs::write(&path, &changed).unwrap();
    let manifest: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(copy.join("drop.json")).unwrap()).unwrap();
    let mut manifest = manifest;
    let digest = sha256_hex(changed.as_bytes());
    manifest["files"][&program] = serde_json::json!(digest);
    std::fs::write(copy.join("drop.json"), serde_json::to_string_pretty(&manifest).unwrap()).unwrap();
    let (code, stdout, stderr) = check(&copy, &[]);
    assert_ne!(code, Some(0), "the edited program is refused:\n{stdout}");
    assert!(stderr.contains(&program) && stderr.contains("certificate"), "the refusal names the program and its certificate:\n{stderr}");
}

fn sha256_hex(bytes: &[u8]) -> String {
    logicaffeine_compile::crypto_accel::sha256(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn a_drop_of_fetched_weights_checks_against_the_checkpoint_it_is_given() {
    let (_d, drop, m) = dropped(Tiny::Qwen3, &["--fetch-weights"]);
    assert!(!files(&drop).iter().any(|f| f.ends_with(".gguf")), "the checkpoint is not carried");
    let (code, _, stderr) = check(&drop, &[]);
    assert_ne!(code, Some(0), "a fetched drop needs its checkpoint");
    assert!(stderr.contains("--weights"), "the refusal says how to give the checkpoint:\n{stderr}");
    let (code, stdout, stderr) = check(&drop, &["--weights", m.path.to_str().unwrap()]);
    assert_eq!(code, Some(0), "the drop verifies against its checkpoint:\n{stdout}\n{stderr}");
    let other = Model::write(Tiny::Llama.spec(CTX), "other");
    let (code, _, stderr) = check(&drop, &["--weights", other.path.to_str().unwrap()]);
    assert_ne!(code, Some(0), "another checkpoint is refused");
    assert!(stderr.contains("sha256"), "the refusal names the digest it wanted:\n{stderr}");
}
