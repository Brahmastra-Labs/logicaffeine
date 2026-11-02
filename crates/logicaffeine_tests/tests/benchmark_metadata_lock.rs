//! Every benchmark the suite runs is described — `benchmarks/lib/benchmarks.tsv` names each one, and the
//! results JSON the site renders takes its category, title, description, declared (time, space) complexity and
//! sizes from that row. A benchmark missing a field ships as an untitled card with a `?` complexity, and nothing
//! else notices.
//!
//! Invariants: the registry's ids are exactly the directories under `benchmarks/programs/`, every row fills
//! every field it must, a run is archived under `results/history/` only when the tree is the release the
//! archive names, and every binary a run verifies and times is one that run built: a failed build stops the run
//! by name instead of leaving the previous run's binary in `bin/` to be measured.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test benchmark_metadata_lock`

#![cfg(not(target_arch = "wasm32"))]

use std::collections::BTreeSet;
use std::path::PathBuf;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

fn run_sh() -> String {
    std::fs::read_to_string(repo_root().join("benchmarks/run.sh")).expect("benchmarks/run.sh")
}

/// The body of shell function `name` — from its `name() {` line to the closing `}` at column 0.
fn function_body<'a>(script: &'a str, name: &str) -> &'a str {
    let head = format!("\n{name}() {{\n");
    let start = script
        .find(&head)
        .unwrap_or_else(|| panic!("run.sh defines {name}()"))
        + head.len();
    let end = start
        + script[start..]
            .find("\n}\n")
            .unwrap_or_else(|| panic!("{name}() closes"));
    &script[start..end]
}

/// The rows of a registry file as `header → field` maps, every row the header's width.
fn registry(name: &str) -> Vec<std::collections::BTreeMap<String, String>> {
    let text = std::fs::read_to_string(repo_root().join("benchmarks/lib").join(name)).expect(name);
    let mut lines = text.lines();
    let header: Vec<&str> = lines.next().expect("a header row").split('\t').collect();
    lines
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let fields: Vec<&str> = l.split('\t').collect();
            assert_eq!(
                fields.len(),
                header.len(),
                "{name}: a row has {} fields, the header {}: {l:?}",
                fields.len(),
                header.len()
            );
            header
                .iter()
                .map(|h| h.to_string())
                .zip(fields.iter().map(|f| f.to_string()))
                .collect()
        })
        .collect()
}

#[test]
fn the_benchmark_list_is_the_program_directories() {
    let list: Vec<String> = registry("benchmarks.tsv")
        .into_iter()
        .map(|r| r["id"].clone())
        .collect();
    let unique: BTreeSet<String> = list.iter().cloned().collect();
    assert_eq!(
        unique.len(),
        list.len(),
        "benchmarks.tsv names a benchmark twice: {list:?}"
    );
    let dirs: BTreeSet<String> = std::fs::read_dir(repo_root().join("benchmarks/programs"))
        .expect("benchmarks/programs")
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        unique, dirs,
        "benchmarks/lib/benchmarks.tsv must list exactly the directories under benchmarks/programs/ — \
         a directory it omits is never run, a name without a directory fails the run"
    );
}

#[test]
fn every_benchmark_has_a_name_a_description_and_a_complexity() {
    for row in registry("benchmarks.tsv") {
        let id = &row["id"];
        for field in ["category", "name", "description", "time", "space"] {
            assert!(
                !row[field].trim().is_empty(),
                "benchmarks.tsv leaves {field} empty for {id} — the results JSON would carry an empty title, \
                 description or `?` complexity for it"
            );
        }
        for field in [
            "ref_size",
            "interp_size",
            "quick_size",
            "slow_size",
            "small_size",
            "dev_size",
        ] {
            assert!(
                row[field].parse::<u64>().is_ok_and(|n| n > 0),
                "benchmarks.tsv {field} for {id} is not a size: {:?}",
                row[field]
            );
        }
        assert!(
            matches!(row["linear"].as_str(), "0" | "1"),
            "benchmarks.tsv linear for {id} is 0 or 1: {:?}",
            row["linear"]
        );
    }
}

/// `results/history/v<version>*.json` is one record per release. The release run comes after the CHANGELOG roll
/// and the version bump (docs/project/VERSIONING.md); a run on any other tree is unreleased code and must not file its
/// numbers under a release's name.
#[test]
fn the_history_archive_is_written_only_for_the_release_it_names() {
    let script = run_sh();
    let body = function_body(&script, "release_archive");
    assert!(
        body.contains("\"$CHANGELOG_TOP\" != \"$LOGOS_VER\"") && body.contains("\"$UNRELEASED_LINES\" -ne 0"),
        "release_archive() archives only when the CHANGELOG's newest release is the version and [Unreleased] is empty:\n{body}"
    );
    let writes: Vec<&str> = script
        .lines()
        .filter(|l| {
            l.contains("history/") && !l.trim_start().starts_with('#') && !body.contains(l.trim())
        })
        .collect();
    assert!(
        writes.is_empty(),
        "every write into results/history goes through release_archive(): {writes:?}"
    );
    assert!(
        script
            .matches("release_archive \"$RESULTS_DIR/latest")
            .count()
            >= 3,
        "the suite, interpreter and codec results are archived"
    );
}

/// Phase 1 builds every implementation into `benchmarks/bin/`, which outlives the run: a compiler writes its
/// output only on success, and `calibrate-c.sh` builds `-O2` binaries under the same names. So the real
/// `run.sh` runs here in a fixture tree with stub compilers on `PATH`: `bin/` holds binaries from an earlier
/// run, the C build fails, the C++ one is skipped, and `largo` succeeds without producing a binary while the
/// shared target still holds the previous benchmark's `bench`. The run must stop in Phase 1 naming `fib/C`
/// and `fib/LOGOS (Rust)`, must not build what `SKIP_LANGS` skips, and must leave no earlier binary
/// behind to be verified and timed.
#[test]
fn a_failed_build_stops_the_run_and_leaves_no_stale_binary() {
    let fixture =
        std::env::temp_dir().join(format!("benchmark_metadata_lock-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&fixture);
    let bench = fixture.join("benchmarks");
    let stubs = fixture.join("stubs");
    for dir in [bench.join("programs/fib"), bench.join("bin"), stubs.clone()] {
        std::fs::create_dir_all(&dir).unwrap();
    }
    std::fs::copy(repo_root().join("benchmarks/run.sh"), bench.join("run.sh")).unwrap();
    std::fs::create_dir_all(bench.join("lib")).unwrap();
    for entry in std::fs::read_dir(repo_root().join("benchmarks/lib"))
        .expect("benchmarks/lib")
        .flatten()
    {
        if entry.path().is_file() {
            std::fs::copy(entry.path(), bench.join("lib").join(entry.file_name())).unwrap();
        }
    }
    let one_program: String = {
        let tsv =
            std::fs::read_to_string(repo_root().join("benchmarks/lib/benchmarks.tsv")).unwrap();
        let mut lines = tsv.lines();
        let header = lines.next().unwrap();
        let fib = lines
            .find(|l| l.starts_with("fib\t"))
            .expect("benchmarks.tsv has fib");
        format!("{header}\n{fib}\n")
    };
    std::fs::write(bench.join("lib/benchmarks.tsv"), one_program).unwrap();
    std::fs::write(
        fixture.join("Cargo.toml"),
        "[workspace.package]\nversion = \"0.0.0\"\n",
    )
    .unwrap();
    std::fs::write(
        fixture.join("CHANGELOG.md"),
        "# Changelog\n\n## [Unreleased]\n",
    )
    .unwrap();
    std::fs::write(
        bench.join("programs/fib/main.c"),
        "int main(void) { return 0; }\n",
    )
    .unwrap();
    std::fs::write(
        bench.join("programs/fib/main.cpp"),
        "int main() { return 0; }\n",
    )
    .unwrap();
    std::fs::write(
        bench.join("programs/fib/main.lg"),
        "## Main\nShow 832040.\n",
    )
    .unwrap();
    std::fs::write(bench.join("programs/fib/expected_30.txt"), "832040\n").unwrap();
    std::fs::write(bench.join("programs/fib/sizes.txt"), "30\n").unwrap();
    std::fs::create_dir_all(bench.join(".logos-bench-target/release")).unwrap();
    std::fs::write(
        bench.join(".logos-bench-target/release/bench"),
        "#!/bin/sh\necho 832040\n",
    )
    .unwrap();
    for stale in ["fib_c", "fib_cpp", "sieve_c"] {
        std::fs::write(bench.join("bin").join(stale), "#!/bin/sh\necho 832040\n").unwrap();
    }
    let stub = |name: &str, body: &str| {
        use std::os::unix::fs::PermissionsExt;
        let path = stubs.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    };
    stub("cargo", BUILDS_LARGO_AND_THE_JIT_IMAGE_TOOLS);
    stub(
        "gcc",
        "echo 'main.c: error: the fixture fails this build' >&2; exit 1",
    );
    stub(
        "g++",
        "echo 'g++ ran although SKIP_LANGS skips cpp' >&2; exit 1",
    );
    let path = format!(
        "{}:{}",
        stubs.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let out = std::process::Command::new("bash")
        .arg(bench.join("run.sh"))
        .env("PATH", path)
        .env("SKIP_LANGS", "cpp")
        .env("RUN_INTERP", "0")
        .env("RUN_CODEC", "0")
        .env("BENCH_QUIET", "off")
        .env("BENCH_MEMORY_MAX", "off")
        .env_remove("CARGO_TARGET_DIR")
        .output()
        .expect("bash runs benchmarks/run.sh");
    let log = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let left: Vec<String> = std::fs::read_dir(bench.join("bin"))
        .map(|rd| {
            rd.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    let _ = std::fs::remove_dir_all(&fixture);
    assert_eq!(
        out.status.code(),
        Some(1),
        "a failed build must stop the run before anything is verified or timed:\n{log}"
    );
    assert!(
        !log.contains("Phase 2"),
        "the run went on past Phase 1 with a build failed:\n{log}"
    );
    assert!(
        log.contains("fib/C") && !log.contains("fib/C++"),
        "the failure names fib/C, and C++ (skipped) was never built:\n{log}"
    );
    assert!(
        log.contains("fib/LOGOS (Rust)") && log.contains("fib/LOGOS (C)"),
        "largo produced no binary, so both LOGOS builds failed — the previous benchmark's `bench` in the shared target is not this one's:\n{log}"
    );
    assert!(
        !log.contains("g++ ran"),
        "SKIP_LANGS=cpp must stop Phase 1 building C++:\n{log}"
    );
    assert!(left.is_empty(), "binaries from an earlier run survived Phase 1 and would be measured as this run's: {left:?}");
}

/// A stub `cargo` that builds what `largo_build` asks for: `largo`, and the JIT image tools beside it
/// (`logos-jit-image-build`, `logos-jit-run`), executables in the bench target's release directory.
const BUILDS_LARGO_AND_THE_JIT_IMAGE_TOOLS: &str = "mkdir -p \"$CARGO_TARGET_DIR/release\" && for b in largo logos-jit-image-build logos-jit-run; do printf '#!/bin/sh\\n' > \"$CARGO_TARGET_DIR/release/$b\" && chmod +x \"$CARGO_TARGET_DIR/release/$b\"; done";

/// `line` gives `LARGO` a value — anything but empty (`LARGO=""` only declares it), wherever the assignment sits:
/// at the start, after `||`, in a candidate loop, from `$(…)`.
fn assigns_largo(line: &str) -> bool {
    let mut rest = line;
    while let Some(i) = rest.find("LARGO=") {
        let before = rest[..i].chars().next_back();
        let after = &rest[i + "LARGO=".len()..];
        let empty = after.is_empty()
            || after.starts_with("\"\"")
            || after.starts_with("''")
            || after.starts_with(char::is_whitespace);
        if !before.is_some_and(|c| c.is_alphanumeric() || c == '_') && !empty {
            return true;
        }
        rest = after;
    }
    false
}

/// Every benchmark script that runs `largo` gets it from ONE place, `benchmarks/lib/largo.sh` (sourced directly or
/// through `lib/suite.sh`, which sources it): built into the bench target, the build's failure loud, and never a
/// fallback to another directory's `largo` — a stale binary from a previous checkout would be measured as this
/// tree's. Run here on fixtures with a stub `cargo`: the helper resolves
/// the bench target's binary, fails when cargo fails, and fails — rather than falling back — when cargo succeeds
/// without producing a binary while a stale one sits in `target/release/`.
#[test]
fn every_benchmark_script_builds_largo_through_one_helper() {
    use std::os::unix::fs::PermissionsExt;
    let helper = repo_root().join("benchmarks/lib/largo.sh");
    assert!(
        helper.is_file(),
        "benchmarks/lib/largo.sh defines largo_build for every benchmark script"
    );
    let run = |name: &str, cargo_body: &str, stale: bool| {
        let root = std::env::temp_dir().join(format!("largo_helper-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let bench = root.join("benchmarks");
        std::fs::create_dir_all(bench.join("lib")).unwrap();
        std::fs::copy(&helper, bench.join("lib/largo.sh")).unwrap();
        std::fs::write(root.join("Cargo.toml"), "[workspace]\n").unwrap();
        if stale {
            std::fs::create_dir_all(root.join("target/release")).unwrap();
            std::fs::write(root.join("target/release/largo"), "#!/bin/sh\n").unwrap();
            std::fs::create_dir_all(bench.join(".logos-bench-target/release")).unwrap();
            std::fs::write(
                bench.join(".logos-bench-target/release/logicaffeine-cli"),
                "#!/bin/sh\n",
            )
            .unwrap();
        }
        let stubs = root.join("stubs");
        std::fs::create_dir_all(&stubs).unwrap();
        std::fs::write(stubs.join("cargo"), format!("#!/bin/sh\n{cargo_body}\n")).unwrap();
        std::fs::set_permissions(stubs.join("cargo"), std::fs::Permissions::from_mode(0o755))
            .unwrap();
        let out = std::process::Command::new("bash")
            .arg("-c")
            .arg(format!(
                ". '{}/lib/largo.sh' && largo_build && echo \"LARGO=$LARGO\"",
                bench.display()
            ))
            .env(
                "PATH",
                format!(
                    "{}:{}",
                    stubs.display(),
                    std::env::var("PATH").unwrap_or_default()
                ),
            )
            .env_remove("CARGO_TARGET_DIR")
            .output()
            .unwrap();
        let log = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        let _ = std::fs::remove_dir_all(&root);
        (out.status.success(), log, bench)
    };
    let (ok, log, bench) = run("built", BUILDS_LARGO_AND_THE_JIT_IMAGE_TOOLS, false);
    assert!(
        ok,
        "a successful build yields the bench target's largo:\n{log}"
    );
    let want = format!(
        "LARGO={}",
        bench.join(".logos-bench-target/release/largo").display()
    );
    assert!(
        log.contains(&want),
        "largo is the bench target's binary ({want}):\n{log}"
    );
    let (ok, log, _) = run(
        "failed",
        "echo 'error[E0433]: the fixture fails this build' >&2; exit 101",
        false,
    );
    assert!(
        !ok && log.contains("E0433"),
        "a failed build stops the script and shows cargo's error:\n{log}"
    );
    let (ok, log, _) = run(
        "no-image-tools",
        "mkdir -p \"$CARGO_TARGET_DIR/release\" && printf '#!/bin/sh\\n' > \"$CARGO_TARGET_DIR/release/largo\" && chmod +x \"$CARGO_TARGET_DIR/release/largo\"",
        false,
    );
    assert!(
        !ok && log.contains("image tools do not exist"),
        "a build that leaves the JIT image tools out stops the script, for the JIT rows would run without them:\n{log}"
    );
    let (ok, log, _) = run("stale", "exit 0", true);
    assert!(!ok, "a build that produced no largo must fail, never fall back to a stale binary elsewhere:\n{log}");

    let mut offenders = Vec::new();
    for dir in [
        "benchmarks",
        "benchmarks/sat",
        "benchmarks/arena",
        "benchmarks/parity",
    ] {
        let Ok(rd) = std::fs::read_dir(repo_root().join(dir)) else {
            continue;
        };
        for e in rd.flatten() {
            let path = e.path();
            if path.extension().is_none_or(|x| x != "sh") {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap_or_default();
            let rel = path
                .strip_prefix(repo_root())
                .unwrap()
                .display()
                .to_string();
            if text.contains("cargo build -p logicaffeine-cli") {
                offenders.push(format!("{rel}: builds largo itself"));
            }
            if text.lines().any(assigns_largo) {
                offenders.push(format!("{rel}: locates largo itself"));
            }
            if text.contains("$LARGO")
                && !text.contains("lib/largo.sh")
                && !text.contains("lib/suite.sh")
            {
                offenders.push(format!("{rel}: runs $LARGO without lib/largo.sh"));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "largo comes from benchmarks/lib/largo.sh only:\n  {}",
        offenders.join("\n  ")
    );
    for shape in [
        "LARGO=\"$SCRIPT_DIR/../target/release/largo\"",
        "[ -x \"$LARGO\" ] || LARGO=\"$HOME/largo\"",
        "    [ -f \"$c\" ] && LARGO=\"$c\" && break",
        "LARGO=$(find \"$T\" -name largo)",
    ] {
        assert!(assigns_largo(shape), "the rule must see: {shape}");
    }
    for fine in [
        "LARGO=\"\"",
        "MY_LARGO=\"/x\"",
        "\"$LARGO\" run --interpret 1",
    ] {
        assert!(!assigns_largo(fine), "the rule must accept: {fine}");
    }
}

#[test]
fn the_registry_reader_reads_the_rows_it_guards() {
    let rows = registry("benchmarks.tsv");
    let ids: Vec<&str> = rows.iter().map(|r| r["id"].as_str()).collect();
    assert!(
        ids.contains(&"fib") && ids.contains(&"ackermann") && ids.contains(&"string_search"),
        "{ids:?}"
    );
    let fib = rows.iter().find(|r| r["id"] == "fib").unwrap();
    assert_eq!(
        (fib["time"].as_str(), fib["space"].as_str()),
        ("O(2^n)", "O(n)")
    );
    assert!(
        !fib["collapse"].is_empty(),
        "fib is a collapse, and its note is a field like any other"
    );
    let two_sum = rows.iter().find(|r| r["id"] == "two_sum").unwrap();
    assert!(
        two_sum["collapse"].is_empty() && !two_sum["dev_size"].is_empty(),
        "an empty field keeps the columns after it in place"
    );
}
