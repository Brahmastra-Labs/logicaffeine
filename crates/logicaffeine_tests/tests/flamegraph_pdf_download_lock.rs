//! Flamegraph-PDF download ratchet — the benchmark profiling chart is offered as a
//! download on the web Benchmarks page, and that only works if four independent
//! pieces stay wired together:
//!
//!   1. the source PDF exists and is a real PDF (`benchmarks/profile/flamegraphs.pdf`);
//!   2. `.gitignore` un-ignores it despite the global `*.pdf` rule, so CI's fresh
//!      checkout actually has the bytes to serve;
//!   3. `scripts/web/stage-web-data.sh` copies it into the runtime `/data` bundle; and
//!   4. the Benchmarks page links the served path `/data/flamegraphs.pdf`.
//!
//! Drop any one and the download link 404s silently in production. Each check is its
//! own `#[test]` so a failure names the broken link in the chain.
//!
//! Run: `cargo nextest run -p logicaffeine-tests -E 'binary(flamegraph_pdf_download_lock)'`

#![cfg(not(target_arch = "wasm32"))]

use std::path::PathBuf;

/// The source-of-truth PDF, tracked in git despite the global `*.pdf` ignore.
const SRC_PDF: &str = "benchmarks/profile/flamegraphs.pdf";

/// The path the web app serves the PDF from (staged into public/data/ at build time).
const SERVED_PATH: &str = "/data/flamegraphs.pdf";

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

fn read(rel: &str) -> String {
    let path = repo_root().join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}"))
}

#[test]
fn source_pdf_exists_and_is_a_real_pdf() {
    let path = repo_root().join(SRC_PDF);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| {
        panic!(
            "source flamegraph PDF `{SRC_PDF}` is missing ({e}) — regenerate it with \
             `bash scripts/bench/profile-benchmarks.sh`; the Benchmarks page download depends on it"
        )
    });
    assert!(
        bytes.starts_with(b"%PDF-"),
        "`{SRC_PDF}` does not start with the %PDF- magic — it is truncated or not a PDF"
    );
}

#[test]
fn gitignore_untracks_the_pdf_despite_the_global_rule() {
    let ignore = read(".gitignore");
    assert!(
        ignore.contains("*.pdf"),
        ".gitignore no longer carries the global `*.pdf` rule — this lock assumes it does"
    );
    assert!(
        ignore.contains(&format!("!{SRC_PDF}")),
        ".gitignore must negate `!{SRC_PDF}` so the published chart is tracked despite `*.pdf` \
         — without it CI's fresh checkout can't serve the download"
    );
}

#[test]
fn staging_script_copies_the_pdf_into_the_data_bundle() {
    let script = read("scripts/web/stage-web-data.sh");
    assert!(
        script.contains(SRC_PDF),
        "scripts/web/stage-web-data.sh must copy `{SRC_PDF}` — it is the only place the /data bundle \
         is populated, so dropping the copy silently 404s the page download"
    );
    assert!(
        script.contains("flamegraphs.pdf") && script.contains("%PDF-"),
        "stage-web-data.sh must stage flamegraphs.pdf and keep its fail-closed %PDF- check"
    );
}

#[test]
fn benchmarks_page_links_the_served_pdf() {
    let page = read("apps/logicaffeine_web/src/ui/pages/benchmarks/mod.rs");
    let count = page.matches(SERVED_PATH).count();
    assert!(
        count >= 2,
        "benchmarks.rs must link the served download path `{SERVED_PATH}` in both the prominent \
         pill and the Links list (found {count}) — the download is unreachable otherwise"
    );
    assert!(
        page.contains("download: \"logos-flamegraphs.pdf\""),
        "the flamegraph links must carry the `download` attribute so the browser saves the file"
    );
}
