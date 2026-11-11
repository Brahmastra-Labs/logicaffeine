//! samba_exam_recompile_lock — the row of the Samba book's exam E8 (section 10.8 of `benchmarks/PREPARE_FOR_SAMBA.md`,
//! the compiler's own quality) on recompiling: a new batch size recompiles only what the batch changes.
//!
//! Each shared architecture's SN50 decode bundle is compiled at a batch of 2 and then of 3, both through one section
//! cache (`largo emit rdu --cache DIR`), and the batch of 3 once more through an empty one. A batch stacks its sequences'
//! hidden states, so every product and norm over the stack, and every gather of one sequence's row out of it, is a new
//! section at a new batch; a sequence's own norm, rotation, cache writes and attention are not. Held three ways:
//! - the batch of 3 compiled through the first batch's cache is the bundle the empty cache's compile writes, byte for
//!   byte: no section the cache supplied is other than compiling it makes;
//! - the sections it compiled are exactly the ones the empty cache's compile makes that the first batch's did not — the
//!   entries each compile added, by their keys — and it says so, compiling one section for each;
//! - it compiled some and reused some: the batch changed sections, and left others alone.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --cargo-profile test-opt --test samba_exam_recompile_lock`

#![cfg(feature = "codegen")]

#[path = "cli_common/mod.rs"]
mod cli;
#[path = "common/tiny_models.rs"]
mod tiny_models;
#[path = "common/samba_handoff.rs"]
#[allow(dead_code)]
mod samba_handoff;

use samba_handoff::{Model, CTX};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use tiny_models::Tiny;

/// The architectures every row covers: attention alone, a hybrid with recurrent state, windowed attention, and an
/// expert mixture with sinks.
const SHARED: [Tiny; 4] = [Tiny::Qwen3, Tiny::Qwen35, Tiny::Gemma3, Tiny::GptOss];

/// `largo emit rdu --chip sn50 --model <checkpoint> --ctx CTX --batch <batch> --cache <cache> -o <out>`: every file of the
/// bundle by its path under `out`, and the sections the compile says it compiled and reused.
fn emit(m: &Model, batch: usize, cache: &Path, out: &Path) -> (BTreeMap<String, Vec<u8>>, (usize, usize)) {
    let o = cli::largo_in(
        out.parent().expect("a parent"),
        &[
            "emit",
            "rdu",
            "--chip",
            "sn50",
            "--model",
            m.path.to_str().unwrap(),
            "--ctx",
            &CTX.to_string(),
            "--batch",
            &batch.to_string(),
            "--cache",
            cache.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
        ],
    );
    let err = cli::strip_ansi(&cli::stderr(&o));
    assert_eq!(o.status.code(), Some(0), "largo emit rdu --batch {batch} --cache failed:\n{err}");
    let line = err.lines().find(|l| l.contains("sections compiled")).unwrap_or_else(|| panic!("the compile says what it compiled:\n{err}"));
    let count = |before: &str| -> usize {
        let head = &line[..line.find(before).unwrap_or_else(|| panic!("`{before}` in `{line}`"))];
        head.trim_end().rsplit(|c: char| !c.is_ascii_digit()).next().and_then(|n| n.parse().ok()).unwrap_or_else(|| panic!("a count before `{before}` in `{line}`"))
    };
    let said = (count(" sections compiled"), count(" reused"));
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
    (files, said)
}

/// The entries a section cache holds, by key.
fn entries(cache: &Path) -> BTreeSet<String> {
    std::fs::read_dir(cache)
        .map(|d| d.map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).filter_map(|n| n.strip_suffix(".json").map(String::from)).collect())
        .unwrap_or_default()
}

/// E8.4 — a new batch size recompiles only what the batch changes.
#[test]
fn a_new_batch_size_recompiles_only_what_it_changes() {
    let dir = tempfile::tempdir().unwrap();
    for t in SHARED {
        let m = Model::write(t.spec(CTX), &format!("{t:?}"));
        let (shared, empty) = (dir.path().join(format!("{t:?}-cache")), dir.path().join(format!("{t:?}-cache-empty")));
        let (_, first) = emit(&m, 2, &shared, &dir.path().join(format!("{t:?}-b2")));
        let before = entries(&shared);
        assert_eq!(first.0, before.len(), "{t:?}: the first compile into an empty cache compiles each section it keeps once");
        let (again, said) = emit(&m, 3, &shared, &dir.path().join(format!("{t:?}-b3")));
        let after = entries(&shared);
        let (whole, alone) = emit(&m, 3, &empty, &dir.path().join(format!("{t:?}-b3-empty")));
        let fresh = entries(&empty);

        assert_eq!(again.keys().collect::<Vec<_>>(), whole.keys().collect::<Vec<_>>(), "{t:?}: the same files through either cache");
        for (path, bytes) in &again {
            assert!(bytes == &whole[path], "{t:?}: `{path}` of the batch of 3 differs between the first batch's cache and an empty one");
        }

        let added: BTreeSet<&String> = after.difference(&before).collect();
        let changed: BTreeSet<&String> = fresh.difference(&before).collect();
        assert_eq!(added, changed, "{t:?}: the batch of 3 compiled exactly its sections the batch of 2 did not make");
        assert_eq!(said.0, added.len(), "{t:?}: one section compiled for each the batch changed: it says {said:?}");
        assert_eq!(said.0 + said.1, alone.0 + alone.1, "{t:?}: the same sections asked for through either cache: {said:?} and {alone:?}");
        assert!(said.0 > 0, "{t:?}: a new batch size changes some sections: {said:?}");
        assert!(said.1 > 0 && fresh.intersection(&before).count() > 0, "{t:?}: a new batch size leaves some sections alone: {said:?}");
    }
}
