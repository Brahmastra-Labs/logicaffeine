//! The solver benchmark's competitors are the binaries its results name.
//!
//! `benchmarks/results/solvers.json` (baked into the site's /benchmarks page) races our certified prover
//! against external SAT solvers. `solver_bench` runs whatever binary sits at each solver's default path and
//! records the version it reports; `benchmarks/sat/external-solvers.sh` is what puts a binary there. A solver
//! the build script does not build is simply absent at regeneration time, and a table regenerated without it
//! silently loses a competitor — which is how the published comparison was about to drop CaDiCaL and
//! CryptoMiniSat.
//!
//! Invariants:
//!   1. every binary `solver_bench` reads by default is one the build script builds, at the same path;
//!   2. every fetch in the build script is pinned (a release tag, or a full commit for untagged projects);
//!   3. each raced solver's pin is the version `solvers.json` records, so a published number names the
//!      binary that produced it — a pin moves only with a re-recorded result;
//!   4. the solver benchmark requires every solver it races, so a failed build stops the run;
//!   5. a raced solver must answer before it races: a binary that is present but cannot run (its library moved,
//!      say) decides no formula, and the run stops naming it rather than publish a column of errors.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test solver_bench_provenance_lock`

#![cfg(not(target_arch = "wasm32"))]

use std::collections::BTreeMap;
use std::path::PathBuf;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

fn read(rel: &str) -> String {
    std::fs::read_to_string(repo_root().join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

const BUILD: &str = "benchmarks/sat/external-solvers.sh";

/// `solver_bin("KISSAT_BIN", "kissat/build/kissat")` → {"KISSAT_BIN": "kissat/build/kissat"}.
fn bench_defaults() -> BTreeMap<String, String> {
    let src = read("crates/logicaffeine_proof/examples/solver_bench.rs");
    let mut out = BTreeMap::new();
    for piece in src.split("solver_bin(\"").skip(1) {
        let Some((var, rest)) = piece.split_once("\", \"") else {
            continue;
        };
        let Some((path, _)) = rest.split_once('"') else {
            continue;
        };
        out.insert(var.to_string(), path.to_string());
    }
    out
}

/// `KISSAT="$EXT/kissat/build/kissat"` → {"kissat/build/kissat"} — the binaries the build script produces.
fn built_paths(script: &str) -> Vec<String> {
    script
        .lines()
        .filter_map(|l| l.split_once("=\"$EXT/"))
        .filter(|(name, _)| {
            !name.is_empty() && name.chars().all(|c| c.is_ascii_uppercase() || c == '_')
        })
        .map(|(_, rest)| rest.trim_end_matches('"').to_string())
        .collect()
}

/// Each `fetch_tag <dir> <url> <tag>` / `fetch_commit <dir> <url> <sha>` call: (dir, pin, is_commit).
fn fetches(script: &str) -> Vec<(String, String, bool)> {
    let mut out = Vec::new();
    for l in script.lines().map(str::trim) {
        for (verb, commit) in [("fetch_tag ", false), ("fetch_commit ", true)] {
            let Some(rest) = l.strip_prefix(verb) else {
                continue;
            };
            let words: Vec<&str> = rest.split_whitespace().collect();
            if words.len() >= 3 && words[1].starts_with("https://") {
                out.push((words[0].to_string(), words[2].to_string(), commit));
            }
        }
    }
    out
}

#[test]
fn every_binary_the_benchmark_reads_is_one_the_build_script_builds() {
    let script = read(BUILD);
    let built = built_paths(&script);
    let defaults = bench_defaults();
    assert!(
        defaults.len() >= 4,
        "solver_bench names its solvers through solver_bin(..): {defaults:?}"
    );
    for (var, path) in &defaults {
        assert!(built.contains(path), "solver_bench reads {var} from $SATBENCH_EXT/{path}, which {BUILD} never builds (it builds {built:?})");
    }
}

#[test]
fn every_fetch_is_pinned() {
    let script = read(BUILD);
    let fetched = fetches(&script);
    assert!(
        fetched.len() >= 5,
        "the build script fetches its solvers through fetch_tag/fetch_commit: {fetched:?}"
    );
    for (dir, pin, commit) in &fetched {
        if *commit {
            assert!(
                pin.len() == 40 && pin.chars().all(|c| c.is_ascii_hexdigit()),
                "{dir}: an untagged project is pinned to a full commit, not `{pin}`"
            );
        } else {
            assert!(
                !pin.is_empty() && pin != "master" && pin != "main",
                "{dir}: a release tag, not the moving branch `{pin}`"
            );
        }
    }
    let bare: Vec<&str> = script
        .lines()
        .filter(|l| l.contains("git clone") && !l.contains("\"$2\" \"$EXT/$1\""))
        .collect();
    assert!(
        bare.is_empty(),
        "every clone goes through a pinned fetch helper: {bare:?}"
    );
    for (dir, pin, _) in &fetched {
        assert!(
            script.contains(&format!("#   {dir}"))
                && script.lines().any(|l| l.starts_with("#")
                    && l.contains(dir.as_str())
                    && l.contains(pin.as_str())),
            "{BUILD}'s header table names {dir} at the pin its fetch uses ({pin})"
        );
    }
}

#[test]
fn each_raced_solver_is_pinned_to_the_version_the_results_record() {
    let script = read(BUILD);
    let pins: BTreeMap<String, String> = fetches(&script)
        .into_iter()
        .map(|(d, p, _)| (d, p))
        .collect();
    let results: serde_json::Value =
        serde_json::from_str(&read("benchmarks/results/solvers.json")).expect("solvers.json");
    let meta = &results["metadata"];
    for solver in ["kissat", "cadical", "cryptominisat"] {
        let recorded = meta[solver]
            .as_str()
            .unwrap_or_else(|| panic!("solvers.json records the {solver} version"));
        let pin = pins
            .get(solver)
            .unwrap_or_else(|| panic!("{BUILD} fetches {solver}"));
        assert_eq!(
            pin.trim_start_matches("rel-"), recorded,
            "{solver}: the build pins {pin} but solvers.json records {recorded} — move the pin and re-record together"
        );
    }
}

#[test]
fn the_solver_benchmark_requires_every_solver_it_races() {
    let runner = read("benchmarks/run-solver-vs-z3.sh");
    let line = runner
        .lines()
        .find(|l| l.trim_start().starts_with("require_solvers "))
        .expect("run-solver-vs-z3.sh builds and requires its solvers");
    for solver in ["kissat", "sadical", "cadical", "cryptominisat"] {
        assert!(
            line.split_whitespace().any(|w| w == solver),
            "run-solver-vs-z3.sh must require {solver}: `{line}`"
        );
    }
    assert!(
        runner.contains("set -euo pipefail"),
        "a failed requirement stops the run"
    );
}

/// A stub SAT solver at `path`: `answers` decides its formula as a competition solver does (`s SATISFIABLE` exit 10,
/// `s UNSATISFIABLE` exit 20); otherwise it fails as a binary whose shared library is gone does.
#[cfg(unix)]
fn stub_solver(path: &std::path::Path, answers: bool) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let body = if answers {
        "#!/bin/sh\nif grep -q -- '^-1 0' \"$1\"; then echo 's UNSATISFIABLE'; exit 20; fi\necho 's SATISFIABLE'; exit 10\n"
    } else {
        "#!/bin/sh\necho 'error while loading shared libraries: libsolver.so: cannot open shared object file' >&2\nexit 127\n"
    };
    std::fs::write(path, body).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// `require_solvers` over a sandbox whose raced solvers are all present (so no builder fetches anything), with
/// CryptoMiniSat answering or not: (exit status, what it wrote).
#[cfg(unix)]
fn require_raced_solvers(cryptominisat_answers: bool) -> (bool, String) {
    let sandbox = tempfile::TempDir::new().unwrap();
    let ext = sandbox.path().join("ext");
    for (rel, answers) in [
        ("kissat/build/kissat", true),
        ("sadical/sadical/sadical", true),
        ("sadical/sadical/tests/dpr-trim", true),
        ("cadical/build/cadical", true),
        ("cryptominisat/build/cryptominisat5", cryptominisat_answers),
    ] {
        stub_solver(&ext.join(rel), answers);
    }
    let out = std::process::Command::new("bash")
        .arg("-c")
        .arg(". \"$SCRIPT\" && require_solvers kissat sadical cadical cryptominisat")
        .env("SCRIPT", repo_root().join("benchmarks/sat/external-solvers.sh"))
        .env("ROOT", sandbox.path())
        .env("LOG", sandbox.path().join("log"))
        .env("SATBENCH_EXT", &ext)
        .output()
        .expect("bash runs the build script");
    let log = std::fs::read_to_string(sandbox.path().join("log")).unwrap_or_default();
    (out.status.success(), format!("{}{log}", String::from_utf8_lossy(&out.stderr)))
}

#[cfg(unix)]
#[test]
fn a_raced_solver_that_cannot_answer_stops_the_run_by_name() {
    let (ok, said) = require_raced_solvers(true);
    assert!(ok, "every raced solver answers, so the run goes ahead:\n{said}");
    let (ok, said) = require_raced_solvers(false);
    assert!(!ok, "a CryptoMiniSat that cannot run must stop the run");
    assert!(
        said.contains("cryptominisat (") && !said.contains("kissat ("),
        "the refusal names the solver that does not answer, and only it:\n{said}"
    );
}
