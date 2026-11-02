//! The vendored Arena corpus cannot drift: every file is the byte-pinned upstream case, and the
//! generated test list names every vendored case with its upstream expectation.
//!
//! The heavy cases' inputs (the Lean exports under `reference-kernels/`) stay outside the repository,
//! so a clone does not have them. Every read of that tree asks the input seam first
//! (`common::require_tool("reference-kernels")`): a clone records a counted `LOGOS_TOOL_SKIP`, and the
//! full runner and every runner of a reading binary require the tree, so on the box that has it a
//! missing input fails instead of skipping.

mod common;

use logicaffeine_kernel::interface::lean::package::sha256_hex;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests")
}

fn manifest() -> serde_json::Value {
    let text = std::fs::read_to_string(root().join("fixtures/arena_corpus/manifest.json")).unwrap();
    serde_json::from_str(&text).unwrap()
}

#[test]
fn the_manifest_is_the_pinned_upstream_revision() {
    let m = manifest();
    assert_eq!(m["schema"], "logos-arena-corpus/2");
    assert_eq!(m["revision"], "87512b60eb2483562967ebc93a5fb42eaa135389");
    assert_eq!(
        m["archive_sha256"],
        "609baebb26a438e61a5ee28b7c3551793ab4066ae7af54f20e60e71cea74d6b5"
    );
    assert_eq!(
        m["results_sha256"],
        "1e2c90211db7e67de3734ed5fa4804051f338ffacca0de87a08db0eee213af9e"
    );
    let cases = m["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 236, "the live Arena has 236 cases");
    let outcomes = |o: &str| cases.iter().filter(|c| c["outcome"] == o).count();
    assert_eq!(
        (outcomes("accept"), outcomes("reject"), outcomes("either")),
        (137, 73, 26)
    );
    let official = cases
        .iter()
        .filter(|c| c["provenance"] == "official_archive")
        .count();
    assert_eq!(
        official, 200,
        "every scored test in the official archive is used from its official bytes"
    );
    let absent: Vec<_> = cases.iter().filter(|c| c["location"] == "absent").collect();
    assert!(
        absent.is_empty(),
        "every upstream case has bytes: {absent:?}"
    );
}

#[test]
fn every_vendored_file_matches_its_pinned_hash() {
    for c in manifest()["cases"].as_array().unwrap() {
        if c["location"] != "vendored" {
            continue;
        }
        let path = root()
            .join("fixtures/arena_corpus")
            .join(c["path"].as_str().unwrap());
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(
            sha256_hex(&bytes),
            c["sha256"].as_str().unwrap(),
            "{}",
            c["name"]
        );
    }
}

/// The cases whose Lean build is not reproducible, so that no export of them can be the Arena's bytes:
/// two builds of cedar-spec at the pinned revision with the pinned toolchain export different record
/// counts, neither is the Arena's, and the Arena's own exports of it differ between its rounds. Shrinks,
/// never grows.
const NONDETERMINISTIC_UPSTREAM: &[&str] = &["cedar"];

/// A case's bytes are the Arena's own, wherever they came from: the official archive, or an export
/// regenerated from the pinned sources that hashes to the digest `results.json` publishes. A
/// regenerated input that differs by one byte is a different test. The one exception is a case no
/// build reproduces, which carries its own digest, the Arena's, and the evidence.
#[test]
fn every_case_is_the_arenas_exact_bytes() {
    for c in manifest()["cases"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let upstream = c["upstream_sha256"]
            .as_str()
            .unwrap_or_else(|| panic!("{name}: no upstream digest"));
        assert_eq!(upstream.len(), 64, "{name}: a SHA-256 digest");
        match c["provenance"].as_str() {
            Some("official_archive" | "regenerated_from_pinned_source") => {
                assert_eq!(
                    c["sha256"].as_str(),
                    Some(upstream),
                    "{name}: not the Arena's bytes"
                )
            }
            Some("regenerated_nondeterministic_upstream") => {
                assert!(
                    NONDETERMINISTIC_UPSTREAM.contains(&name),
                    "{name}: only a pinned case may differ from the Arena's bytes"
                );
                assert!(
                    c["nondeterministic"]
                        .as_str()
                        .is_some_and(|e| !e.is_empty()),
                    "{name}: the evidence is recorded"
                );
                assert_ne!(
                    c["sha256"].as_str(),
                    Some(upstream),
                    "{name}: identical bytes are recorded as such"
                );
            }
            other => panic!("{name}: unknown provenance {other:?}"),
        }
    }
}

#[test]
fn the_generated_case_list_covers_the_corpus_exactly() {
    let listed = std::fs::read_to_string(root().join("arena_corpus/cases.rs")).unwrap();
    let mut expected = Vec::new();
    for c in manifest()["cases"].as_array().unwrap() {
        if c["location"] != "vendored" {
            continue;
        }
        let exp = match c["outcome"].as_str().unwrap() {
            "accept" => "Accept",
            "reject" => "Reject",
            _ => "Either",
        };
        expected.push(format!("\"{}\", {exp});", c["path"].as_str().unwrap()));
    }
    let lines: Vec<&str> = listed
        .lines()
        .filter(|l| l.starts_with("arena_case!("))
        .collect();
    assert_eq!(lines.len(), expected.len(), "one test per vendored case");
    let heavy = manifest()["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["location"] == "heavy")
        .count();
    let heavy_lines = listed
        .lines()
        .filter(|l| l.starts_with("arena_heavy_case!("))
        .count();
    assert_eq!(heavy_lines, heavy, "one ignored test per heavy case");
    assert_eq!(
        lines.len() + heavy_lines,
        236,
        "every upstream case has a test"
    );
    for e in &expected {
        assert_eq!(
            lines.iter().filter(|l| l.ends_with(e.as_str())).count(),
            1,
            "missing or duplicated: {e}"
        );
    }
}

/// Files that name `reference-kernels` without reading an input through it, each with its reason.
const INPUT_TREE_MENTIONS: &[(&str, &str)] = &[
    ("common/mod.rs", "the seam itself: the named probe asks whether the tree is there"),
    ("common/public_manifest.rs", "names the gitignored tree in the ignore oracle; reads nothing under it"),
    (
        "lean_common/mod.rs",
        "the pinned Lean exporter under reference-kernels/lean4export, which `toolchain()` gates on the `lean` requirement",
    ),
    ("lean_fixture_provenance_lock.rs", "a provenance record's `source_path`, data under test; reads nothing"),
    (
        "arena_corpus/cases.rs",
        "the case table: each heavy row names its input and reaches it only through `run_heavy`, held below",
    ),
];

/// Every `.rs` under `tests/`, as (path relative to `tests/`, source).
fn test_sources() -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut stack = vec![root()];
    while let Some(dir) = stack.pop() {
        for e in std::fs::read_dir(&dir).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "rs") {
                let rel = p
                    .strip_prefix(root())
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                out.push((rel, std::fs::read_to_string(&p).unwrap()));
            }
        }
    }
    out.sort();
    out
}

/// The text of the `fn` whose body contains byte offset `at`, from its `fn` keyword to `at`; `None` at
/// item level. `at` is the end of the line in question, so a seam on that line counts.
fn enclosing_fn(src: &str, at: usize) -> Option<&str> {
    let before = &src[..at];
    let mut from = before.len();
    while let Some(i) = before[..from].rfind("fn ") {
        let boundary = i == 0 || before[..i].ends_with([' ', '\n', '\t']);
        let named = before[i + 3..].starts_with(|c: char| c.is_alphabetic() || c == '_');
        if boundary && named {
            let text = &before[i..];
            let open = text.matches('{').count() as i64 - text.matches('}').count() as i64;
            return (open > 0).then_some(text);
        }
        from = i;
    }
    None
}

/// The body of `fn <name>` in `src`, from its `fn` keyword to its closing brace at column 0.
fn fn_body<'a>(src: &'a str, name: &str) -> Option<&'a str> {
    let start = src.find(&format!("\nfn {name}("))? + 1;
    let end = src[start..].find("\n}\n")? + start + 2;
    Some(&src[start..end])
}

/// The body of `macro_rules! <name> { … }` in `src`, its braces matched.
fn macro_body<'a>(src: &'a str, name: &str) -> Option<&'a str> {
    let at = src.find(&format!("macro_rules! {name}"))?;
    let open = at + src[at..].find('{')?;
    let mut depth = 0usize;
    for (i, c) in src[open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&src[open..=open + i]);
                }
            }
            _ => {}
        }
    }
    None
}

const SEAM: &str = "common::require_tool(\"reference-kernels\")";

/// The test binaries that read an input under `reference-kernels/`, each proven to ask the seam
/// first; the violations, if any, name the file and the line.
fn input_readers() -> (Vec<String>, Vec<String>) {
    let this_file = file!().rsplit(['/', '\\']).next().unwrap();
    let mut readers = Vec::new();
    let mut violations = Vec::new();
    for (file, src) in test_sources() {
        if file == this_file {
            continue;
        }
        let binary = (!file.contains('/')).then(|| file.trim_end_matches(".rs").to_string());
        if src.contains("include!(\"arena_corpus/cases.rs\")") {
            let expands = macro_body(&src, "arena_heavy_case").unwrap_or_else(|| {
                panic!("{file} includes the case table and must define `arena_heavy_case!`")
            });
            if expands.contains("run_heavy(") {
                match fn_body(&src, "run_heavy") {
                    Some(body) => {
                        let seam = body.find(SEAM);
                        let read = body.find(".join(");
                        if !matches!((seam, read), (Some(s), Some(r)) if s < r) {
                            violations.push(format!(
                                "{file}: `run_heavy` must ask `{SEAM}` before it reads the input"
                            ));
                        }
                    }
                    None => violations.push(format!(
                        "{file}: `arena_heavy_case!` calls `run_heavy`, which is not defined"
                    )),
                }
                readers.extend(binary.clone());
            } else if expands.contains("fn ") {
                violations.push(format!("{file}: `arena_heavy_case!` defines a test that does not go through `run_heavy`"));
            }
        }
        if INPUT_TREE_MENTIONS.iter().any(|(f, _)| *f == file) {
            continue;
        }
        let mut offset = 0;
        for (n, line) in src.split_inclusive('\n').enumerate() {
            offset += line.len();
            if !line.contains("reference-kernels") || line.trim_start().starts_with("//") {
                continue;
            }
            match enclosing_fn(&src, offset) {
                Some(f) if f.contains(SEAM) => readers.extend(binary.clone()),
                _ => violations.push(format!(
                    "{file}:{}: reads reference-kernels/ without asking `{SEAM}` first",
                    n + 1
                )),
            }
        }
    }
    readers.sort();
    readers.dedup();
    (readers, violations)
}

#[test]
fn every_read_of_the_large_inputs_asks_the_input_seam_first() {
    let (readers, violations) = input_readers();
    assert!(
        violations.is_empty(),
        "a clone without reference-kernels/ would fail these instead of skipping:\n  {}",
        violations.join("\n  ")
    );
    for known in [
        "arena_corpus",
        "dag_dual_corpus",
        "dag_nbe_differential",
        "dag_sem_corpus",
        "dag_sem_probe",
    ] {
        assert!(
            readers.iter().any(|r| r == known),
            "the census must see `{known}` read the inputs; it found {readers:?}"
        );
    }
    for (file, reason) in INPUT_TREE_MENTIONS {
        assert!(!reason.trim().is_empty(), "`{file}` needs its reason");
        let src = std::fs::read_to_string(root().join(file)).unwrap_or_default();
        assert!(
            src.contains("reference-kernels"),
            "`{file}` no longer names reference-kernels: drop its entry"
        );
    }
}

#[test]
fn the_input_probe_answers_whether_the_tree_is_there() {
    let checkout = tempfile::tempdir().unwrap();
    assert!(
        !common::reference_kernels_at(checkout.path()),
        "a checkout without reference-kernels/ does not have the inputs"
    );
    std::fs::write(checkout.path().join("reference-kernels"), "").unwrap();
    assert!(
        !common::reference_kernels_at(checkout.path()),
        "a file named reference-kernels is not the tree"
    );
    std::fs::remove_file(checkout.path().join("reference-kernels")).unwrap();
    std::fs::create_dir(checkout.path().join("reference-kernels")).unwrap();
    assert!(
        common::reference_kernels_at(checkout.path()),
        "a checkout with reference-kernels/ has the tree"
    );
    #[cfg(unix)]
    {
        let linked = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(elsewhere.path(), linked.path().join("reference-kernels"))
            .unwrap();
        assert!(
            common::reference_kernels_at(linked.path()),
            "a linked tree (a private copy, the fresh-clone gate) is the tree"
        );
    }
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    assert_eq!(
        common::tool_available("reference-kernels"),
        common::reference_kernels_at(&repo),
        "the named probe asks about this checkout"
    );
}

#[test]
fn the_full_runner_and_every_runner_of_a_reader_require_the_inputs() {
    let scripts = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts");
    let full = std::fs::read_to_string(scripts.join("run-all-tests-fast.sh")).unwrap();
    let default = common::required_by_runner(&full, "run-all-tests-fast.sh")
        .expect("the full runner sets LOGOS_REQUIRE_TOOLS");
    assert!(
        common::tool_required_by(&default, "reference-kernels"),
        "the full runner must fail when the inputs are missing — its LOGOS_REQUIRE_TOOLS default is {default:?}"
    );
    let (readers, _) = input_readers();
    let mut runners = Vec::new();
    for e in std::fs::read_dir(&scripts).unwrap().flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if !(name.starts_with("run-") && name.ends_with(".sh")) || name == "run-all-tests-fast.sh" {
            continue;
        }
        let script = std::fs::read_to_string(e.path()).unwrap();
        let names_a_reader = script
            .lines()
            .filter(|l| !l.trim_start().starts_with('#'))
            .any(|l| {
                l.split(|c: char| !(c.is_alphanumeric() || c == '_'))
                    .any(|w| readers.iter().any(|r| r == w))
            });
        if !names_a_reader {
            continue;
        }
        let required = common::required_by_runner(&script, &name).unwrap_or_default();
        assert!(
            common::tool_required_by(&required, "reference-kernels"),
            "{name} runs a binary that reads reference-kernels/ and must require the tree \
             (`export LOGOS_REQUIRE_TOOLS=\"${{LOGOS_REQUIRE_TOOLS:+$LOGOS_REQUIRE_TOOLS,}}reference-kernels\"`); it requires {required:?}"
        );
        runners.push(name);
    }
    for known in ["run-arena-mathlib-tests.sh", "run-opt-in-feature-tests.sh"] {
        assert!(
            runners.iter().any(|r| r == known),
            "the census must see `{known}` run a reading binary; it found {runners:?}"
        );
    }
}

#[test]
fn the_census_reads_exactly_a_macros_own_body() {
    let src = "macro_rules! arena_heavy_case {\n    ($($t:tt)*) => { { let x = 1; } };\n}\n\n#[test]\nfn after() {}\n";
    assert_eq!(
        macro_body(src, "arena_heavy_case"),
        Some("{\n    ($($t:tt)*) => { { let x = 1; } };\n}")
    );
    assert!(
        !macro_body(src, "arena_heavy_case").unwrap().contains("fn "),
        "code after the macro is not its body"
    );
    assert_eq!(macro_body("fn only() {}", "arena_heavy_case"), None);
}
