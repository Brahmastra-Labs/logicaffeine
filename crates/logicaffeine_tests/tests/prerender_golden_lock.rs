//! `largo build`'s documents, byte for byte.
//!
//! The first paint `largo build` writes into every page it ships is rendered by one function, and
//! the web app's front page builds its live preview with the same one. This holds what that function
//! makes of every page the repository ships (each `examples/*/pages/*.lg` and the Studio's website
//! example) to the document recorded in `goldens/prerendered_pages.txt`, by length and a stable
//! FNV-1a hash, so moving or sharing it can change nothing a reader is served. A deliberate change to
//! a page or to the renderer rerecords the golden: `LOGOS_REGEN=1 cargo nextest run -p
//! logicaffeine-tests --test prerender_golden_lock`.

#![cfg(not(target_arch = "wasm32"))]

use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn golden_path() -> PathBuf {
    repo_root().join("crates/logicaffeine_tests/tests/goldens/prerendered_pages.txt")
}

/// FNV-1a over 64 bits: stable across Rust releases, unlike the standard library's hasher.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3))
}

/// Every page the repository ships, by repo-relative path, sorted.
fn pages() -> Vec<String> {
    let root = repo_root();
    let mut found = Vec::new();
    let examples = root.join("examples");
    for project in std::fs::read_dir(&examples).unwrap_or_else(|e| panic!("read {}: {e}", examples.display())) {
        let pages = project.unwrap().path().join("pages");
        let Ok(entries) = std::fs::read_dir(&pages) else { continue };
        for entry in entries {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|e| e == "lg") {
                found.push(path.strip_prefix(&root).unwrap().to_string_lossy().into_owned());
            }
        }
    }
    found.push("apps/logicaffeine_web/src/ui/examples/website.logos".to_string());
    found.sort();
    found
}

/// One golden line: the document's hash, its length, and the page it was built from.
fn line(path: &str) -> String {
    let source = std::fs::read_to_string(repo_root().join(path)).unwrap_or_else(|e| panic!("read {path}: {e}"));
    let stem = Path::new(path).file_stem().unwrap().to_string_lossy();
    let document = logicaffeine_cli::prerender_page_html(&source, &stem);
    format!("{:016x} {} {path}", fnv1a(document.as_bytes()), document.len())
}

#[test]
fn the_hash_is_fnv1a() {
    assert_eq!(fnv1a(b""), 0xcbf2_9ce4_8422_2325);
    assert_eq!(fnv1a(b"a"), 0xaf63_dc4c_8601_ec8c);
    assert_eq!(fnv1a(b"foobar"), 0x8594_4171_f739_67e8);
}

#[test]
fn every_shipped_page_builds_to_its_recorded_document() {
    let lines: Vec<String> = pages().iter().map(|path| line(path)).collect();
    assert!(lines.len() >= 20, "only {} pages were found to build", lines.len());
    let now = lines.join("\n") + "\n";
    if std::env::var_os("LOGOS_REGEN").is_some() {
        std::fs::write(golden_path(), &now).unwrap();
        return;
    }
    let golden = std::fs::read_to_string(golden_path()).unwrap_or_else(|e| panic!("read the golden ({e}); record it with LOGOS_REGEN=1"));
    let changed: Vec<String> = golden
        .lines()
        .zip(now.lines())
        .filter(|(was, is)| was != is)
        .map(|(was, is)| format!("  was {was}\n  now {is}"))
        .collect();
    assert!(changed.is_empty() && golden == now, "largo build serves different documents:\n{}\n(golden {} lines, now {})", changed.join("\n"), golden.lines().count(), now.lines().count());
}

#[test]
fn the_same_page_builds_the_same_document_twice() {
    for path in pages().iter().take(6) {
        assert_eq!(line(path), line(path), "{path} builds to a different document each time");
    }
}
