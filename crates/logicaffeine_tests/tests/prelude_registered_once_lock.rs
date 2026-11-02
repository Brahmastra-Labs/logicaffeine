//! The standard library is registered once per process, and production code asks that one
//! registration: `StandardLibrary::context()`.
//!
//! Registering the prelude checks a thousand declarations and generates and checks their
//! equation lemmas: about 0.3 s in an optimized build and seconds in a debug one. A registered
//! `Context` is copy-on-write throughout, so a caller that needs a context of its own clones the
//! shared one for about a microsecond, and a caller that only asks questions borrows it. A path
//! that registered per call would pay the whole prelude per theorem a certifier proves, per
//! certificate a checker re-checks, per rule an optimizer verifies.
//!
//! The lock: no source under a production package's `src/` (every package but this test crate)
//! calls `StandardLibrary::register`; the prelude's own cache reaches it as `Self::register`.
//! Tests may register afresh, since pinning the shared registration against a fresh one is what
//! `the_cached_prelude_is_indistinguishable_from_a_fresh_one` does.

#[path = "common/public_manifest.rs"]
mod public_manifest;

use std::path::{Path, PathBuf};

use public_manifest::{packages, repo_root};

const TEST_CRATE: &str = "crates/logicaffeine_tests";
const CALL: &str = "StandardLibrary::register(";

/// The 1-based lines of `source` whose code (not a comment) calls `StandardLibrary::register`.
fn registrations_in(source: &str) -> Vec<usize> {
    source
        .lines()
        .enumerate()
        .filter(|(_, line)| {
            let code = line.trim_start();
            !code.starts_with("//") && code.contains(CALL)
        })
        .map(|(i, _)| i + 1)
        .collect()
}

fn rust_sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_sources(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn no_production_source_registers_the_standard_library_again() {
    let root = repo_root();
    let mut scanned = 0usize;
    let mut offenders = Vec::new();
    for dir in packages().into_keys().filter(|dir| dir != TEST_CRATE) {
        let mut files = Vec::new();
        rust_sources(&root.join(&dir).join("src"), &mut files);
        for file in files {
            let source = std::fs::read_to_string(&file)
                .unwrap_or_else(|e| panic!("{}: {e}", file.display()));
            scanned += 1;
            for line in registrations_in(&source) {
                let rel = file
                    .strip_prefix(&root)
                    .unwrap_or(&file)
                    .display()
                    .to_string();
                offenders.push(format!("{rel}:{line}"));
            }
        }
    }
    assert!(
        scanned > 500,
        "scanned only {scanned} production sources: the package walk broke"
    );
    assert!(
        offenders.is_empty(),
        "these production sites register the standard library again instead of asking the process's \
         one registration (`StandardLibrary::context()`, cloned when the caller needs its own \
         context):\n  {}",
        offenders.join("\n  ")
    );
}

#[test]
fn the_scan_finds_a_registration_and_only_a_registration() {
    let source = "\
fn fresh() -> Context {
    let mut ctx = Context::new();
    logicaffeine_kernel::prelude::StandardLibrary::register(&mut ctx);
    ctx
}
/// StandardLibrary::register(&mut ctx);
// StandardLibrary::register(&mut ctx);
fn cached() { Self::register(&mut ctx); StandardLibrary::register_core(&mut ctx); }
";
    assert_eq!(registrations_in(source), vec![3]);
}
