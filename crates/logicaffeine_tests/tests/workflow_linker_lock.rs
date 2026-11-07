//! ★ LOCK — a job that builds Rust installs the linker its workflow links with. ★
//!
//! A workflow can link every x86-64 Linux build through one linker for all its jobs: `test.yml`'s `env:` names clang as
//! the linker (`CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER`) and mold as the one clang drives
//! (`CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS: -Clink-arg=-fuse-ld=mold`). A hosted runner has neither, so a job
//! that installs Rust and not them fails at its first build script — `clang: error: invalid linker name in argument
//! '-fuse-ld=mold'` — as the per-push binary-size job did on its first run. Every such job on a Linux runner installs
//! the packages its workflow's link line names, before it builds.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test workflow_linker_lock`

#![cfg(not(target_arch = "wasm32"))]

#[path = "common/workflows.rs"]
mod workflows;

use std::collections::BTreeSet;

use workflows::{env, jobs, runs_on, script, steps, workflow, workflow_names};

const LINKER: &str = "CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER";
const RUSTFLAGS: &str = "CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS";

/// The packages a workflow's x86-64 Linux link line names: its linker, and the linker `-fuse-ld=` hands that one.
fn link_packages(text: &str) -> BTreeSet<String> {
    let vars = env(text);
    let mut out = BTreeSet::new();
    if let Some(linker) = vars.get(LINKER) {
        out.insert(linker.rsplit('/').next().unwrap_or(linker).to_string());
    }
    if let Some(flags) = vars.get(RUSTFLAGS) {
        for word in flags.split_whitespace() {
            if let Some(ld) = word.split("-fuse-ld=").nth(1) {
                out.insert(ld.to_string());
            }
        }
    }
    out
}

/// The packages a step's `apt-get install` commands name.
fn installed(step: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for line in script(step) {
        let mut rest = line.as_str();
        while let Some(at) = rest.find("apt-get install") {
            rest = &rest[at + "apt-get install".len()..];
            let upto = rest.find("&&").unwrap_or(rest.len());
            out.extend(rest[..upto].split_whitespace().filter(|w| !w.starts_with('-')).map(str::to_string));
        }
    }
    out
}

fn installs_rust(step: &str) -> bool {
    step.lines().any(|l| l.trim_start().trim_start_matches("- ").starts_with("uses: dtolnay/rust-toolchain"))
}

/// Whether a step runs a build: cargo itself, or one of the repository's scripts, which run cargo.
fn builds(step: &str) -> bool {
    script(step).iter().any(|l| l.contains("cargo ") || l.contains("scripts/"))
}

/// Each job of `text` that installs Rust on a Linux runner, with the link-line packages it has not installed by the
/// first step that builds.
fn missing(text: &str) -> Vec<(String, BTreeSet<String>)> {
    let need = link_packages(text);
    if need.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for (name, job) in jobs(text) {
        if !runs_on(&job).is_some_and(|label| label.contains("ubuntu")) {
            continue;
        }
        let steps = steps(&job);
        if !steps.iter().any(|s| installs_rust(s)) {
            continue;
        }
        let first_build = steps.iter().position(|s| builds(s)).unwrap_or(steps.len());
        let ready: BTreeSet<String> = steps[..first_build].iter().flat_map(|s| installed(s)).collect();
        let lacking: BTreeSet<String> = need.difference(&ready).cloned().collect();
        if !lacking.is_empty() {
            out.push((name, lacking));
        }
    }
    out
}

#[test]
fn every_job_that_installs_rust_installs_the_linker_its_workflow_links_with() {
    let mut bad = Vec::new();
    for name in workflow_names() {
        for (job, lacking) in missing(&workflow(&name)) {
            bad.push(format!("{name} job `{job}` builds without {lacking:?}"));
        }
    }
    assert!(bad.is_empty(), "jobs that would fail at their first link:\n{}", bad.join("\n"));
}

#[test]
fn the_test_workflow_names_its_link_line() {
    assert_eq!(
        link_packages(&workflow("test.yml")),
        ["clang", "mold"].into_iter().map(String::from).collect(),
        "test.yml's env no longer names the linker this lock reads"
    );
}

#[test]
fn the_reader_reads_the_shape_it_guards() {
    let text = "name: t\nenv:\n  CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER: clang\n  CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS: -Clink-arg=-fuse-ld=mold\n\njobs:\n  ready:\n    runs-on: ubuntu-latest\n    steps:\n      - uses: dtolnay/rust-toolchain@stable\n      - name: Install mold, clang\n        run: sudo apt-get update && sudo apt-get install -y mold clang\n      - run: cargo build\n  late:\n    runs-on: ubuntu-latest\n    steps:\n      - uses: dtolnay/rust-toolchain@stable\n      - run: cargo build\n      - run: sudo apt-get install -y mold clang\n  bare:\n    runs-on: ubuntu-24.04\n    steps:\n      - uses: dtolnay/rust-toolchain@stable\n      - run: ./scripts/check.sh\n  commented:\n    runs-on: ubuntu-latest-m\n    steps:\n      - name: Install Rust\n        uses: dtolnay/rust-toolchain@stable\n        # its runtime spawns `cargo build --target wasm32` later\n      - name: Install mold, clang\n        run: |\n          # cargo needs these first\n          sudo apt-get update\n          sudo apt-get install -y mold clang\n      - run: cargo nextest run\n  mac:\n    runs-on: macos-14\n    steps:\n      - uses: dtolnay/rust-toolchain@stable\n      - run: cargo build\n  docs:\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo no rust here\n";
    let got: Vec<(String, Vec<String>)> =
        missing(text).into_iter().map(|(j, l)| (j, l.into_iter().collect())).collect();
    let want: Vec<(String, Vec<String>)> = vec![
        ("bare".into(), vec!["clang".into(), "mold".into()]),
        ("late".into(), vec!["clang".into(), "mold".into()]),
    ];
    assert_eq!(got, want);
    assert!(missing("name: t\n\njobs:\n  a:\n    runs-on: ubuntu-latest\n    steps:\n      - uses: dtolnay/rust-toolchain@stable\n      - run: cargo build\n").is_empty(), "a workflow that names no linker asks for none");
}
