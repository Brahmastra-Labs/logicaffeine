//! Every `cargo … -p <package> … --test <name>` we write down names a test binary that package builds.
//!
//! Test binaries move: the v0.11.0 consolidation took ~1,350 of them out of their crates and into
//! `logicaffeine-tests`, and a command still naming the old home fails only when someone runs it
//! (`no test target named …`). The commands people and CI actually run live in `scripts/`,
//! `.github/`, `.config/` (`mutants.toml` included), the docs and READMEs, and the `Run:` lines at the top of
//! the suites themselves, and each one is checked here against the binaries its package owns:
//! `tests/<name>.rs` or `tests/<name>/main.rs` of the package's directory.
//!
//! A command that runs in some other workspace is not this workspace's to check: one after
//! `--manifest-path`, or after a `cd` into a placeholder or variable directory (the private
//! mutation workspaces copy suites back into their own crates). A name that is itself a
//! placeholder (`$BIN`, `<name>`, `{stem}`, a glob) is not a name.

#![cfg(not(target_arch = "wasm32"))]

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

/// Package name → the integration-test binaries it builds.
fn test_binaries() -> BTreeMap<String, BTreeSet<String>> {
    let mut out = BTreeMap::new();
    for parent in ["crates", "apps", "scripts"] {
        let Ok(members) = fs::read_dir(root().join(parent)) else {
            continue;
        };
        for member in members.flatten() {
            let dir = member.path();
            let Ok(text) = fs::read_to_string(dir.join("Cargo.toml")) else {
                continue;
            };
            let manifest: toml::Table = text.parse().unwrap();
            let Some(name) = manifest
                .get("package")
                .and_then(|p| p.get("name"))
                .and_then(|n| n.as_str())
            else {
                continue;
            };
            let mut bins = BTreeSet::new();
            if let Ok(entries) = fs::read_dir(dir.join("tests")) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    let stem = entry.file_name().to_string_lossy().into_owned();
                    if path.is_file() && stem.ends_with(".rs") {
                        bins.insert(stem.trim_end_matches(".rs").to_string());
                    } else if path.join("main.rs").is_file() {
                        bins.insert(stem);
                    }
                }
            }
            out.insert(name.to_string(), bins);
        }
    }
    assert!(
        out.len() >= 15,
        "found only {} packages; the discovery walk broke",
        out.len()
    );
    out
}

/// One `cargo` invocation on a line: its `-p` packages and its `--test` names, or `None` when it
/// runs in another workspace.
struct Invocation {
    packages: Vec<String>,
    tests: Vec<String>,
}

const SUBCOMMANDS: &[&[&str]] = &[
    &["nextest", "run"],
    &["test"],
    &["build"],
    &["check"],
    &["run"],
];

fn invocations(line: &str) -> Vec<Invocation> {
    let mut out = Vec::new();
    let mut rest = line;
    while let Some(at) = rest.find("cargo ") {
        let before = &line[..line.len() - rest.len() + at];
        let after = &rest[at + "cargo ".len()..];
        let segment_end = after
            .find(['`', ';', '&', '|', '\n'])
            .unwrap_or(after.len());
        let segment = &after[..segment_end];
        rest = &after[segment_end..];
        let mut words: Vec<&str> = segment.split_whitespace().collect();
        if words.first().is_some_and(|w| w.starts_with('+')) {
            words.remove(0);
        }
        let Some(sub) = SUBCOMMANDS.iter().find(|s| words.starts_with(s)) else {
            continue;
        };
        let args = &words[sub.len()..];
        if args.iter().any(|w| w.starts_with("--manifest-path")) || runs_elsewhere(before) {
            continue;
        }
        let value_of = |flags: &[&str]| -> Vec<String> {
            args.windows(2)
                .filter(|w| flags.contains(&w[0]))
                .map(|w| {
                    w[1].trim_matches(|c: char| c == '"' || c == '\'' || c == '\\')
                        .trim_end_matches([')', ',', '.', ':'])
                        .to_string()
                })
                .collect()
        };
        out.push(Invocation {
            packages: value_of(&["-p", "--package"]),
            tests: value_of(&["--test"]),
        });
    }
    out
}

/// `cd <dir> && cargo …` / `cd "$WS" && cargo …`: the command runs in a directory this workspace
/// does not name.
fn runs_elsewhere(before: &str) -> bool {
    let Some(cd) = before.rfind("cd ") else {
        return false;
    };
    let target = before[cd + 3..].trim_start().trim_start_matches('"');
    target.starts_with('<') || target.starts_with('$')
}

fn is_placeholder(name: &str) -> bool {
    name.is_empty()
        || name.contains(['$', '<', '{', '*', '…'])
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// (file, line, package, name) for every `--test` name its package does not build, plus how many
/// names were checked.
fn stale(
    surfaces: &[(String, String)],
    bins: &BTreeMap<String, BTreeSet<String>>,
) -> (Vec<String>, usize) {
    let mut bad = Vec::new();
    let mut checked = 0;
    for (file, text) in surfaces {
        for (n, line) in text.lines().enumerate() {
            for inv in invocations(line) {
                let [package] = inv.packages.as_slice() else {
                    continue;
                };
                for name in inv.tests.iter().filter(|t| !is_placeholder(t)) {
                    checked += 1;
                    match bins.get(package) {
                        Some(owned) if owned.contains(name) => {}
                        Some(_) => bad.push(format!("{file}:{}: `-p {package} --test {name}`: {package} builds no test binary `{name}`", n + 1)),
                        None => bad.push(format!("{file}:{}: `-p {package}`: no workspace package is named {package}", n + 1)),
                    }
                }
            }
        }
    }
    (bad, checked)
}

fn collect(dir: &Path, exts: &[&str], recurse: bool, out: &mut Vec<(String, String)>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if recurse && !path.join("CACHEDIR.TAG").exists() {
                collect(&path, exts, recurse, out);
            }
        } else if exts
            .iter()
            .any(|e| path.extension().is_some_and(|x| x == *e))
        {
            if let Ok(text) = fs::read_to_string(&path) {
                out.push((
                    path.strip_prefix(root()).unwrap().display().to_string(),
                    text,
                ));
            }
        }
    }
}

fn surfaces() -> Vec<(String, String)> {
    let r = root();
    let mut out = Vec::new();
    collect(&r.join("scripts"), &["sh", "py"], true, &mut out);
    collect(&r.join(".github"), &["yml", "yaml"], true, &mut out);
    collect(&r.join(".config"), &["toml"], false, &mut out);
    collect(&r.join("docs"), &["md"], true, &mut out);
    collect(&r, &["md", "toml"], false, &mut out);
    collect(
        &r.join("crates/logicaffeine_tests/tests"),
        &["rs"],
        false,
        &mut out,
    );
    for parent in ["crates", "apps"] {
        for member in fs::read_dir(r.join(parent)).unwrap().flatten() {
            let readme = member.path().join("README.md");
            if let Ok(text) = fs::read_to_string(&readme) {
                out.push((readme.strip_prefix(&r).unwrap().display().to_string(), text));
            }
        }
    }
    out
}

#[test]
fn every_named_test_target_is_a_binary_its_package_builds() {
    let (bad, checked) = stale(&surfaces(), &test_binaries());
    assert!(
        checked >= 100,
        "only {checked} `-p … --test …` names were found across scripts, CI, docs and suites; the command scanner stopped matching"
    );
    assert!(
        bad.is_empty(),
        "{} command(s) name a test binary that is not where they say (the suite moved, or was renamed; \
         integration suites live in `-p logicaffeine-tests`):\n  {}",
        bad.len(),
        bad.join("\n  ")
    );
}

#[test]
fn the_scanner_flags_a_stale_target_and_passes_over_other_workspaces_and_placeholders() {
    let bins: BTreeMap<String, BTreeSet<String>> = [
        (
            "logicaffeine-tests".to_string(),
            ["phase1_garden_path".to_string()].into(),
        ),
        ("logicaffeine-kernel".to_string(), BTreeSet::new()),
    ]
    .into();
    let doc = |s: &str| vec![("doc.md".to_string(), s.to_string())];

    let (bad, checked) = stale(
        &doc("`cargo nextest run -p logicaffeine-tests --test phase1_garden_path`"),
        &bins,
    );
    assert_eq!((bad.len(), checked), (0, 1));

    let (bad, _) = stale(
        &doc("cargo nextest run -p logicaffeine-kernel --test phase1_garden_path"),
        &bins,
    );
    assert_eq!(
        bad.len(),
        1,
        "a suite named under the crate it left is stale: {bad:?}"
    );

    let (bad, _) = stale(
        &doc(
            "cargo test -p logicaffeine-tests --features web-tests --test guide_coverage --no-run",
        ),
        &bins,
    );
    assert_eq!(
        bad.len(),
        1,
        "flags between -p and --test do not hide a missing binary: {bad:?}"
    );

    let (bad, _) = stale(
        &doc("cargo +nightly test -p logicaffeine-nope --test phase1_garden_path"),
        &bins,
    );
    assert_eq!(bad.len(), 1, "an unknown package is reported: {bad:?}");

    for skipped in [
        "(cd <dir> && cargo test -p logicaffeine-kernel --test dag_sem_lsub)",
        "cd \"$WS\" && cargo test -p logicaffeine-kernel --test dag_sem_lsub",
        "cargo test --manifest-path work/elsewhere/Cargo.toml -p logicaffeine-kernel --test anything",
        "cargo nextest run -p logicaffeine-tests --test $BIN",
        "cargo nextest run -p logicaffeine-tests --test <name>",
        "cargo test -p logicaffeine-kernel -p logicaffeine-tests --test phase1_garden_path",
    ] {
        let (bad, _) = stale(&doc(skipped), &bins);
        assert!(bad.is_empty(), "`{skipped}` is not this workspace's to check: {bad:?}");
    }
}
