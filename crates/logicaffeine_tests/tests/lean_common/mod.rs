//! Shared harness for the `largo lean` tests (`lean_cli_e2e.rs`, `lean_intake_contract.rs`).
//!
//! Every run goes through the real `largo` binary and is held to the output contract before a test
//! looks at anything: stdout is exactly one `logos-lean-result/1` JSON document (or empty, for a
//! usage error), and every stderr line is one JSON diagnostic.

#![allow(dead_code)]

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

pub const RESULT_SCHEMA: &str = "logos-lean-result/1";
pub const STAGES: [&str; 7] = [
    "lean_accepted",
    "translated",
    "primary_check",
    "independent_recheck",
    "assumption_policy",
    "emission_reparsed",
    "fresh_replay",
];

#[path = "../support/under_test.rs"]
mod under_test;

pub fn largo() -> Command {
    Command::new(under_test::largo())
}

pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

pub fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/lean")
}

/// A pinned Lean 4.14.0 export of `name` (see `tests/fixtures/lean/exports/PROVENANCE.json`).
pub fn export(name: &str) -> PathBuf {
    fixtures().join("exports").join(format!("{name}.ndjson"))
}

pub fn export_text(name: &str) -> String {
    std::fs::read_to_string(export(name)).unwrap()
}

pub fn project(name: &str) -> PathBuf {
    fixtures().join("projects").join(name)
}

/// The kernel crate's pinned corpus exports.
pub fn corpus_exports() -> Vec<PathBuf> {
    let dir =
        repo_root().join("crates/logicaffeine_tests/tests/fixtures/lean_conformance/cases_lean414");
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "ndjson"))
        .collect();
    files.sort();
    files
}

/// One `largo lean …` run, held to the output contract.
pub struct Run {
    pub exit: i32,
    pub stdout: String,
    pub stderr: String,
    /// The one result document (`Value::Null` when stdout is empty).
    pub result: Value,
    /// Every stderr line, parsed.
    pub diagnostics: Vec<Value>,
}

impl Run {
    pub fn outcome(&self) -> &str {
        self.result["outcome"]
            .as_str()
            .unwrap_or_else(|| panic!("no outcome in {}", self.stdout))
    }
    pub fn stage(&self, name: &str) -> &str {
        self.result["stages"][name]
            .as_str()
            .unwrap_or_else(|| panic!("no stage {name} in {}", self.stdout))
    }
    /// Codes of the result's diagnostics, in order.
    pub fn codes(&self) -> Vec<String> {
        self.result["diagnostics"]
            .as_array()
            .unwrap_or_else(|| panic!("no diagnostics array in {}", self.stdout))
            .iter()
            .map(|d| d["code"].as_str().unwrap().to_string())
            .collect()
    }
    /// The first diagnostic carrying `code`.
    pub fn diagnostic(&self, code: &str) -> &Value {
        self.result["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["code"] == code)
            .unwrap_or_else(|| panic!("no {code} diagnostic in {}", self.stdout))
    }
    pub fn describe(&self) -> String {
        format!(
            "exit {}\n--- stdout\n{}\n--- stderr\n{}",
            self.exit, self.stdout, self.stderr
        )
    }
}

pub fn run_in(dir: &Path, args: &[&str]) -> Run {
    let output = largo()
        .args(args)
        .current_dir(dir)
        .output()
        .expect("largo spawns");
    held_to_contract(output)
}

pub fn run(args: &[&str]) -> Run {
    run_in(&repo_root(), args)
}

fn held_to_contract(output: Output) -> Run {
    let exit = output
        .status
        .code()
        .expect("largo exits with a code, not a signal");
    let stdout = String::from_utf8(output.stdout).expect("stdout is UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr is UTF-8");
    let result = if stdout.trim().is_empty() {
        Value::Null
    } else {
        let mut documents = serde_json::Deserializer::from_str(&stdout).into_iter::<Value>();
        let first = documents
            .next()
            .expect("one document")
            .unwrap_or_else(|e| panic!("stdout is not JSON ({e}):\n{stdout}"));
        assert!(
            documents.next().is_none(),
            "stdout carries exactly one JSON document:\n{stdout}"
        );
        first
    };
    let diagnostics = stderr
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            serde_json::from_str::<Value>(l)
                .unwrap_or_else(|e| panic!("stderr line is not JSON ({e}): {l:?}"))
        })
        .collect();
    let run = Run {
        exit,
        stdout,
        stderr,
        result,
        diagnostics,
    };
    if !run.result.is_null() && run.result.get("schema").is_some_and(|s| s == RESULT_SCHEMA) {
        assert_result_shape(&run);
    }
    run
}

/// The `logos-lean-result/1` shape, checked on every result any test sees.
pub fn assert_result_shape(run: &Run) {
    let r = &run.result;
    let outcome = r["outcome"].as_str().expect("outcome is a string");
    let expected_exit = match outcome {
        "verified" => 0,
        "invalid_source" => 1,
        "unsupported_feature" => 2,
        "disallowed_assumptions" => 3,
        "resource_limit" => 4,
        "infrastructure_failure" => 5,
        other => panic!("unknown outcome {other:?}\n{}", run.describe()),
    };
    assert_eq!(
        run.exit,
        expected_exit,
        "the exit code is the outcome's\n{}",
        run.describe()
    );
    let stages = r["stages"].as_object().expect("stages is an object");
    let names: Vec<&str> = stages.keys().map(String::as_str).collect();
    let mut sorted_expected = STAGES.to_vec();
    sorted_expected.sort();
    let mut sorted_names = names.clone();
    sorted_names.sort();
    assert_eq!(
        sorted_names,
        sorted_expected,
        "every stage is named, and no other\n{}",
        run.describe()
    );
    for (name, status) in stages {
        let s = status.as_str().expect("a stage status is a string");
        assert!(
            ["passed", "failed", "not_run", "unsupported"].contains(&s),
            "{name}: {s}"
        );
    }
    let all_passed = stages.values().all(|s| s == "passed");
    assert_eq!(
        outcome == "verified",
        all_passed,
        "verified iff every stage passed\n{}",
        run.describe()
    );
    if !all_passed {
        // Every stage that did not pass is explained by at least one diagnostic of that stage.
        let diagnostics = r["diagnostics"].as_array().expect("diagnostics");
        for (name, status) in stages {
            if status != "passed" && status != "not_run" {
                assert!(
                    diagnostics.iter().any(|d| d["stage"] == name.as_str()),
                    "stage {name} is {status} with no diagnostic\n{}",
                    run.describe()
                );
            }
        }
        assert!(
            !diagnostics.is_empty(),
            "a result that is not verified says why\n{}",
            run.describe()
        );
        assert!(
            r["package"].is_null(),
            "only a verified result publishes a package\n{}",
            run.describe()
        );
    }
    for d in r["diagnostics"]
        .as_array()
        .expect("diagnostics is an array")
    {
        assert!(
            d["code"].is_string() && d["stage"].is_string() && d["message"].is_string(),
            "{d}"
        );
        assert!(
            d["declaration"].is_string() || d["declaration"].is_null(),
            "{d}"
        );
        assert!(d["line"].is_u64() || d["line"].is_null(), "{d}");
    }
    assert!(
        r["target"]["lean"].is_string(),
        "the Lean target is named\n{}",
        run.describe()
    );
    // Once the user's input translated and the kernel accepted it, a later failure is never theirs.
    if stages["translated"] == "passed" && stages["primary_check"] == "passed" {
        assert_ne!(
            outcome,
            "invalid_source",
            "a failure after the checks passed is not the user's\n{}",
            run.describe()
        );
    }
    assert!(r["assumptions"].is_array() && r["information_losses"].is_array());
    // The stderr channel carries the same diagnostics, one per line.
    assert_eq!(
        run.diagnostics.len(),
        r["diagnostics"].as_array().unwrap().len(),
        "stderr carries one NDJSON line per diagnostic\n{}",
        run.describe()
    );
    for (line, d) in run
        .diagnostics
        .iter()
        .zip(r["diagnostics"].as_array().unwrap())
    {
        assert_eq!(line, d, "the stderr diagnostic is the result's diagnostic");
    }
}

/// The pinned Lean toolchain and exporter this box provides, if it does.
pub struct Toolchain {
    pub exporter: PathBuf,
}

/// The pinned Lean 4.14.0 (through elan) and the pinned lean4export checkout, when both are here.
/// When `LOGOS_REQUIRE_TOOLS` names `lean`, their absence fails the test instead of switching it to
/// the refusal path.
pub fn toolchain() -> Option<Toolchain> {
    let lean = Command::new("lean")
        .arg("--version")
        .env("ELAN_TOOLCHAIN", "leanprover/lean4:v4.14.0")
        .output()
        .ok()
        .filter(|o| {
            o.status.success() && String::from_utf8_lossy(&o.stdout).contains("version 4.14.0")
        });
    let exporter = repo_root().join("reference-kernels/lean4export");
    let exporter_ok = exporter.join(".lake/build/lib/Export.olean").is_file()
        && exporter.join("Main.lean").is_file();
    let present = lean.is_some() && exporter_ok;
    if !present {
        let spec = std::env::var("LOGOS_REQUIRE_TOOLS").unwrap_or_default();
        let required = spec.trim() == "1"
            || spec.eq_ignore_ascii_case("all")
            || spec.split([',', ' ']).any(|t| t.trim() == "lean");
        assert!(
            !required,
            "LOGOS_REQUIRE_TOOLS demands the pinned Lean toolchain, which is absent"
        );
        eprintln!(
            "LOGOS_TOOL_SKIP: lean (project-mode test asserts the infrastructure refusal path)"
        );
        return None;
    }
    Some(Toolchain { exporter })
}

/// `lean import --project <project> --target <target> --out <out>` plus the exporter location.
pub fn project_import(project: &Path, target: &str, out: &Path, extra: &[&str]) -> Run {
    let exporter = repo_root().join("reference-kernels/lean4export");
    let mut args = vec![
        "lean".to_string(),
        "import".into(),
        "--project".into(),
        project.display().to_string(),
        "--target".into(),
        target.into(),
        "--out".into(),
        out.display().to_string(),
        "--exporter".into(),
        exporter.display().to_string(),
    ];
    args.extend(extra.iter().map(|s| s.to_string()));
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    run(&args)
}

/// `lean import <export> --target <target> --out <out>`.
pub fn export_import(export: &Path, target: &str, out: &Path, extra: &[&str]) -> Run {
    let mut args = vec![
        "lean".to_string(),
        "import".into(),
        export.display().to_string(),
        "--target".into(),
        target.into(),
        "--out".into(),
        out.display().to_string(),
    ];
    args.extend(extra.iter().map(|s| s.to_string()));
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    run(&args)
}

/// Write `text` as an export file in `dir`.
pub fn write_export(dir: &Path, name: &str, text: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, text).unwrap();
    path
}

/// The 1-based line of the first export record satisfying `pick`.
pub fn line_of(text: &str, pick: impl Fn(&Value) -> bool) -> usize {
    text.lines()
        .position(|l| serde_json::from_str::<Value>(l).is_ok_and(|v| pick(&v)))
        .expect("the record exists")
        + 1
}

/// The line of the declaration record (`thm`/`def`/`axiom`/`opaque`) whose name index is the name
/// index of `lean` (a dotted Lean name).
pub fn declaration_line(text: &str, lean: &str) -> usize {
    let index = name_index(text, lean);
    line_of(text, |v| {
        ["thm", "def", "axiom", "opaque"]
            .iter()
            .any(|k| v.get(*k).is_some_and(|d| d["name"] == index))
    })
}

/// The name index of the dotted Lean name `lean` in an export.
pub fn name_index(text: &str, lean: &str) -> u64 {
    let mut names: std::collections::HashMap<u64, String> =
        std::collections::HashMap::from([(0, String::new())]);
    for line in text.lines().skip(1) {
        let v: Value = serde_json::from_str(line).unwrap();
        if let (Some(i), Some(s)) = (v.get("in"), v.get("str")) {
            let pre = &names[&s["pre"].as_u64().unwrap()];
            let full = if pre.is_empty() {
                s["str"].as_str().unwrap().to_string()
            } else {
                format!("{pre}.{}", s["str"].as_str().unwrap())
            };
            names.insert(i.as_u64().unwrap(), full);
        }
    }
    *names
        .iter()
        .find(|(_, n)| n.as_str() == lean)
        .map(|(i, _)| i)
        .expect("the name is in the export")
}

/// A fresh scratch directory; `out` inside it is where a package would be published.
pub fn scratch() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

/// Every entry of `dir`, sorted, as names.
pub fn entries(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    logicaffeine_compile::crypto_accel::sha256(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
