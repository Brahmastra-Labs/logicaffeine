//! The /data runtime bundle can never be missing from a web build.
//!
//! The wasm client fetches its heavyweight data (the lexicon every Learn exercise is drawn
//! from, benchmark results, legal pages) from `/data/*`, staged by
//! `scripts/web/stage-web-data.sh`. When that bundle was a manual pre-step, any `dx serve` or
//! `dx build` run without it served the SPA fallback HTML for `/data/lexicon.json` and every
//! Learn module opened on "The word bank failed to load". Both collin-datacenter servers ran
//! that way. The crate's build script now runs the staging script for every wasm build, and
//! re-runs it whenever anything the script copies changes; these tests pin that contract
//! (the browser suite's `word_bank_is_served_as_json` proves it end to end).

use std::path::{Path, PathBuf};

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../apps/logicaffeine_web")
}

fn repo_root() -> PathBuf {
    crate_dir().join("../..").canonicalize().unwrap()
}

fn build_script() -> String {
    let path = crate_dir().join("build.rs");
    std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("{} is missing — nothing stages /data for a web build", path.display()))
}

/// Every repo path `scripts/web/stage-web-data.sh` reads from.
fn staged_sources() -> Vec<String> {
    let script = std::fs::read_to_string(repo_root().join("scripts/web/stage-web-data.sh")).unwrap();
    let mut sources = Vec::new();
    let mut in_cp = false;
    for line in script.lines() {
        let line = line.trim();
        if line.starts_with("cp ") {
            in_cp = true;
        }
        if in_cp {
            for word in line.trim_start_matches("cp ").trim_end_matches('\\').split_whitespace() {
                if !word.starts_with('"') && !word.starts_with('$') && word.contains('/') {
                    sources.push(word.to_string());
                }
            }
            in_cp = line.ends_with('\\');
        }
        if line.contains("pathlib.Path(\"benchmarks/programs\")") {
            sources.push("benchmarks/programs".to_string());
        }
    }
    sources.sort();
    sources.dedup();
    sources
}

#[test]
fn the_staging_script_reads_the_sources_this_lock_knows_about() {
    let sources = staged_sources();
    for expected in [
        "benchmarks/results/latest.json",
        "crates/logicaffeine_lexicon/assets/lexicon.json",
        "benchmarks/programs",
        "apps/logicaffeine_web/privacy.html",
    ] {
        assert!(sources.iter().any(|s| s == expected), "stage-web-data.sh no longer reads {expected}; sources: {sources:?}");
    }
}

#[test]
fn the_build_script_stages_the_bundle_for_every_wasm_build() {
    let text = build_script();
    assert!(text.contains("scripts/web/stage-web-data.sh"), "build.rs must run scripts/web/stage-web-data.sh");
    assert!(
        text.contains("CARGO_CFG_TARGET_ARCH") && text.contains("wasm32"),
        "build.rs must stage for the wasm client (native builds compile the same bytes in)"
    );
    assert!(text.contains("panic!"), "a failed staging must fail the build, never ship a site without /data");
}

#[test]
fn the_build_script_reruns_whenever_a_staged_source_changes() {
    let text = build_script();
    let mut missing = Vec::new();
    for source in staged_sources().iter().chain(std::iter::once(&"scripts/web/stage-web-data.sh".to_string())) {
        if !text.contains(&format!("\"{source}\"")) {
            missing.push(source.clone());
        }
    }
    assert!(missing.is_empty(), "build.rs must declare rerun-if-changed for every staged source; missing: {missing:?}");
    assert!(text.contains("cargo:rerun-if-changed"), "build.rs must use rerun-if-changed, or it re-stages on every build");
}

#[test]
fn the_build_script_notices_a_deleted_bundle() {
    let text = build_script();
    assert!(
        text.contains("public/data"),
        "build.rs must watch the staged output too, so deleting public/data re-stages it on the next build"
    );
}

#[test]
fn every_staged_source_exists() {
    let root = repo_root();
    for source in staged_sources() {
        assert!(Path::new(&root.join(&source)).exists(), "stage-web-data.sh reads {source}, which does not exist");
    }
}

// `dx serve` watches `public/`, gitignore or not, and rebuilds when anything under it moves.
// The build script re-stages on every wasm build, so a staging run that rewrites unchanged
// files is a rebuild that triggers itself: build → re-stage → watcher fires → build, forever,
// reloading every open tab every ~12 seconds. Staging must therefore be a no-op on disk when
// nothing it derives from changed. The script finds the repo from its own path, so these tests
// stage a scratch copy of everything it reads — the script included — and its default output
// lands inside the copy, where the build script's own run lands in the checkout; the checkout's
// /data is never touched.

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("web_data_bundle_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn copy_tree(from: &Path, to: &Path) {
    if from.is_dir() {
        std::fs::create_dir_all(to).unwrap();
        for entry in std::fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            copy_tree(&entry.path(), &to.join(entry.file_name()));
        }
    } else {
        std::fs::create_dir_all(to.parent().unwrap()).unwrap();
        std::fs::copy(from, to).unwrap_or_else(|e| panic!("copy {}: {e}", from.display()));
    }
}

/// A scratch repo holding the staging script and every source it reads, at their repo paths.
fn scratch_repo(tag: &str) -> PathBuf {
    let repo = scratch_dir(tag);
    let root = repo_root();
    for source in staged_sources().iter().chain(std::iter::once(&"scripts/web/stage-web-data.sh".to_string())) {
        copy_tree(&root.join(source), &repo.join(source));
    }
    repo
}

/// Where the script publishes when it is given no output directory, in `repo`.
fn published(repo: &Path) -> PathBuf {
    repo.join("apps/logicaffeine_web/public/data")
}

/// Run `repo`'s staging script with `args`; its output on success, its error text on failure.
fn run_stage(repo: &Path, args: &[&Path]) -> Result<String, String> {
    let out = std::process::Command::new("bash")
        .arg(repo.join("scripts/web/stage-web-data.sh"))
        .args(args)
        .output()
        .expect("bash runs scripts/web/stage-web-data.sh");
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    if out.status.success() { Ok(text) } else { Err(format!("({}) {text}", out.status)) }
}

fn stage(repo: &Path, args: &[&Path]) -> String {
    run_stage(repo, args).unwrap_or_else(|e| panic!("stage-web-data.sh failed {e}"))
}

/// Every file in the bundle with the stamp a filesystem watcher keys on.
fn snapshot(dir: &Path) -> Vec<(String, u64, std::time::SystemTime)> {
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{} was not staged: {e}", dir.display()))
        .map(|e| {
            let e = e.unwrap();
            let meta = e.metadata().unwrap();
            (e.file_name().to_string_lossy().into_owned(), meta.len(), meta.modified().unwrap())
        })
        .collect();
    files.sort();
    files
}

const BUNDLE: [&str; 5] = ["lexicon.json", "latest.json", "bench-sources.json", "privacy.html", "flamegraphs.pdf"];

#[test]
fn staging_with_no_output_directory_publishes_the_bundle_in_the_web_app() {
    let repo = scratch_repo("default");
    stage(&repo, &[]);
    let names: Vec<String> = snapshot(&published(&repo)).into_iter().map(|(n, _, _)| n).collect();
    for expected in BUNDLE {
        assert!(names.iter().any(|n| n == expected), "{expected} was not published; got {names:?}");
    }
    let _ = std::fs::remove_dir_all(&repo);
}

#[test]
fn staging_into_an_output_directory_publishes_the_whole_bundle_there() {
    let repo = scratch_repo("out");
    let out = scratch_dir("out_dir");
    stage(&repo, &[&out]);
    let names: Vec<String> = snapshot(&out).into_iter().map(|(n, _, _)| n).collect();
    for expected in BUNDLE {
        assert!(names.iter().any(|n| n == expected), "{expected} was not staged into the output dir; got {names:?}");
    }
    assert!(!published(&repo).exists(), "staging somewhere else must not touch the web app's own /data");
    let _ = std::fs::remove_dir_all(&out);
    let _ = std::fs::remove_dir_all(&repo);
}

#[test]
fn an_output_directory_that_holds_anything_is_refused() {
    // Publishing drops every file the bundle does not carry, so a mistyped directory would lose
    // whatever it held; the script takes only an absent or empty one.
    let repo = scratch_repo("occupied");
    let out = scratch_dir("occupied_dir");
    std::fs::create_dir_all(&out).unwrap();
    std::fs::write(out.join("keep.txt"), "mine").unwrap();
    let refused = run_stage(&repo, &[&out]).expect_err("a non-empty output directory must be refused");
    assert!(refused.contains("is not empty"), "{refused}");
    assert_eq!(std::fs::read_to_string(out.join("keep.txt")).unwrap(), "mine", "the refusal touched the directory");
    let _ = std::fs::remove_dir_all(&out);
    let _ = std::fs::remove_dir_all(&repo);
}

#[test]
fn restaging_an_unchanged_bundle_touches_nothing() {
    let repo = scratch_repo("idempotent");
    stage(&repo, &[]);
    let before = snapshot(&published(&repo));
    assert!(!before.is_empty(), "the first staging run produced nothing");
    // Past any coarse mtime granularity, so a rewrite cannot hide inside the same tick.
    std::thread::sleep(std::time::Duration::from_millis(1100));
    stage(&repo, &[]);
    let after = snapshot(&published(&repo));
    assert_eq!(
        before, after,
        "re-staging an unchanged bundle rewrote files — a dev server watching public/ rebuilds on every build"
    );
    let _ = std::fs::remove_dir_all(&repo);
}

#[test]
fn restaging_repairs_an_edited_file_and_drops_a_stale_one() {
    let repo = scratch_repo("repair");
    stage(&repo, &[]);
    let data = published(&repo);
    std::fs::write(data.join("lexicon.json"), "{}").unwrap();
    std::fs::write(data.join("stale.json"), "{}").unwrap();
    stage(&repo, &[]);
    let lexicon = std::fs::read(repo_root().join("crates/logicaffeine_lexicon/assets/lexicon.json")).unwrap();
    assert_eq!(
        std::fs::read(data.join("lexicon.json")).unwrap(),
        lexicon,
        "an edited staged file must be restored from its source"
    );
    assert!(
        !data.join("stale.json").exists(),
        "a file the bundle no longer carries must be removed, so the output never drifts from the sources"
    );
    let _ = std::fs::remove_dir_all(&repo);
}
