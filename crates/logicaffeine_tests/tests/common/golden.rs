//! Committed artifacts a test derives. A test run never writes a tracked file: it derives the artifact and compares
//! it with the committed one, failing with the command that regenerates it. `LOGOS_REGEN=1` (set by
//! `scripts/test/gen-silicon-goldens.sh`, never by a suite runner) writes the derived artifact first, then compares.

#![allow(dead_code)]

use std::path::{Path, PathBuf};

pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

pub fn regenerating() -> bool {
    std::env::var_os("LOGOS_REGEN").is_some()
}

/// The committed file at `rel` (repo-relative) holds exactly `want`.
pub fn committed_bytes(rel: &str, want: &[u8], regen: &str) {
    let path = repo_root().join(rel);
    if regenerating() {
        std::fs::write(&path, want).unwrap_or_else(|e| panic!("write {rel}: {e}"));
    }
    let have = std::fs::read(&path)
        .unwrap_or_else(|e| panic!("read {rel}: {e} — regenerate with `{regen}`"));
    if have != want {
        let (h, w) = (
            String::from_utf8_lossy(&have),
            String::from_utf8_lossy(want),
        );
        let at = h
            .lines()
            .zip(w.lines())
            .position(|(a, b)| a != b)
            .unwrap_or_else(|| h.lines().count().min(w.lines().count()));
        panic!(
            "{rel} is not what this test derives (first difference at line {}):\n  committed: {:?}\n  derived:   {:?}\n\
             regenerate with `{regen}`, then review the diff",
            at + 1,
            h.lines().nth(at).unwrap_or("<end of file>"),
            w.lines().nth(at).unwrap_or("<end of file>"),
        );
    }
}

/// The committed JSON document at `rel` equals `want` as a value (key order and whitespace aside); regeneration
/// writes it pretty-printed.
pub fn committed_json(rel: &str, want: &serde_json::Value, regen: &str) {
    let path = repo_root().join(rel);
    if regenerating() {
        std::fs::write(&path, serde_json::to_string_pretty(want).unwrap())
            .unwrap_or_else(|e| panic!("write {rel}: {e}"));
    }
    let have: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {rel}: {e} — regenerate with `{regen}`")),
    )
    .unwrap_or_else(|e| panic!("{rel} is not JSON: {e}"));
    assert!(
        &have == want,
        "{rel} is not what this test derives — regenerate with `{regen}`, then review the diff\n  committed: {}\n  derived:   {}",
        serde_json::to_string(&have).unwrap(),
        serde_json::to_string(want).unwrap(),
    );
}

/// The committed JSON document at `rel`, parsed.
pub fn committed_json_value(rel: &str, regen: &str) -> serde_json::Value {
    let raw = std::fs::read_to_string(repo_root().join(rel))
        .unwrap_or_else(|e| panic!("read {rel}: {e} — regenerate with `{regen}`"));
    serde_json::from_str(&raw).unwrap_or_else(|e| panic!("{rel} is not JSON: {e}"))
}

/// Where a generator writes: the committed directory `rel` when regenerating, else a scratch directory named `tag`.
pub fn output_dir(rel: &str, tag: &str) -> PathBuf {
    let dir = if regenerating() {
        repo_root().join(rel)
    } else {
        std::env::temp_dir().join(tag)
    };
    std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("mkdir {}: {e}", dir.display()));
    dir
}
