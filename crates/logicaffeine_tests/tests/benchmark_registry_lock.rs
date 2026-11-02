//! The benchmark suite has one registry, `benchmarks/lib/`, and everything reads it.
//!
//! `benchmarks.tsv` (the programs), `languages.tsv` (every timed implementation), `compilers.tsv` (the compile-time
//! rows) and `limits.tsv` (the sizes a runtime cannot reach) are data, so bash, Python and these tests all read the same
//! rows. The invariants:
//!
//! - bash loads exactly the fields a plain tab split reads (an empty field must not shift the columns after it);
//! - every language row is well formed, the three LOGOS tiers form the `logos` family, and the command the page shows
//!   carries the same flags as the command the run executes;
//! - the hand-written C and LOGOS's `largo build --release --emit c` compile with the same flags, every C-family row
//!   (gcc, clang, Nim) takes them with floating-point contraction off, and the other compiled rows take the same
//!   host-CPU, whole-program settings;
//! - no benchmark script keeps its own list or per-benchmark table (the drift that left three programs unmeasured);
//! - every timed command runs pinned to the timed CPUs;
//! - every benchmark's output proves the work: an expected file at every timed size, distinct from size to size, and
//!   never a bare zero.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test benchmark_registry_lock`

#![cfg(not(target_arch = "wasm32"))]

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

type Row = BTreeMap<String, String>;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

fn lib() -> PathBuf {
    repo_root().join("benchmarks/lib")
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn registry(name: &str) -> (Vec<String>, Vec<Row>) {
    let text = read(&lib().join(name));
    let mut lines = text.lines();
    let header: Vec<String> = lines
        .next()
        .expect("a header row")
        .split('\t')
        .map(str::to_string)
        .collect();
    let rows = lines
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let fields: Vec<&str> = l.split('\t').collect();
            assert_eq!(
                fields.len(),
                header.len(),
                "{name}: row width {} ≠ header width {}: {l:?}",
                fields.len(),
                header.len()
            );
            header
                .iter()
                .cloned()
                .zip(fields.iter().map(|f| f.to_string()))
                .collect()
        })
        .collect();
    (header, rows)
}

fn ids(rows: &[Row]) -> Vec<String> {
    rows.iter().map(|r| r["id"].clone()).collect()
}

/// What `lib/suite.sh` loads, as `array → key → value`, read back out of bash.
fn bash_loaded(arrays: &[&str]) -> BTreeMap<String, BTreeMap<String, String>> {
    let mut script = format!(". '{}/suite.sh'\n", lib().display());
    for a in arrays {
        script.push_str(&format!("for k in \"${{!{a}[@]}}\"; do printf '%s\\x1f%s\\x1f%s\\n' {a} \"$k\" \"${{{a}[$k]}}\"; done\n"));
    }
    let out = std::process::Command::new("bash")
        .arg("-c")
        .arg(&script)
        .env_remove("CARGO_TARGET_DIR")
        .output()
        .expect("bash");
    assert!(
        out.status.success(),
        "sourcing lib/suite.sh failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let mut loaded: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let mut parts = line.splitn(3, '\u{1f}');
        let (a, k, v) = (
            parts.next().unwrap(),
            parts.next().unwrap(),
            parts.next().unwrap_or(""),
        );
        loaded
            .entry(a.to_string())
            .or_default()
            .insert(k.to_string(), v.to_string());
    }
    loaded
}

#[test]
fn bash_loads_every_field_a_tab_split_reads() {
    let (_, benches) = registry("benchmarks.tsv");
    let (_, langs) = registry("languages.tsv");
    let (_, compilers) = registry("compilers.tsv");
    let bench_fields = [
        ("BENCH_CATEGORY", "category"),
        ("BENCH_NAME", "name"),
        ("BENCH_SHORT", "short"),
        ("BENCH_DESC", "description"),
        ("BENCH_TIME", "time"),
        ("BENCH_PROOF", "proof"),
        ("BENCH_OUTPUT", "output"),
        ("BENCH_SPACE", "space"),
        ("BENCH_REF", "ref_size"),
        ("BENCH_COLLAPSE", "collapse"),
        ("BENCH_INTERP_SIZE", "interp_size"),
        ("BENCH_QUICK_SIZE", "quick_size"),
        ("BENCH_SLOW_SIZE", "slow_size"),
        ("BENCH_SMALL_SIZE", "small_size"),
        ("BENCH_DEV_SIZE", "dev_size"),
        ("BENCH_LINEAR", "linear"),
    ];
    let lang_fields = [
        ("LANG_LABEL", "label"),
        ("LANG_COLOR", "color"),
        ("LANG_TIER", "tier"),
        ("LANG_SOURCE", "source"),
        ("LANG_BUILD", "build"),
        ("LANG_RUN", "run"),
        ("LANG_VERSION", "version"),
        ("LANG_MEM", "mem"),
        ("LANG_SIZE", "size"),
        ("LANG_PROFILE", "profile"),
        ("LANG_SHOWN", "shown"),
        ("LANG_ENGINE", "engine"),
    ];
    let compiler_fields = [("COMPILER_LABEL", "label"), ("COMPILER_COMMAND", "command")];
    let arrays: Vec<&str> = bench_fields
        .iter()
        .chain(&lang_fields)
        .chain(&compiler_fields)
        .map(|(a, _)| *a)
        .collect();
    let loaded = bash_loaded(&arrays);
    for (rows, fields) in [
        (&benches, &bench_fields[..]),
        (&langs, &lang_fields[..]),
        (&compilers, &compiler_fields[..]),
    ] {
        for row in rows {
            for (array, field) in fields {
                let got = loaded
                    .get(*array)
                    .and_then(|m| m.get(&row["id"]))
                    .map(String::as_str)
                    .unwrap_or("");
                assert_eq!(
                    got, row[*field],
                    "lib/suite.sh loads {array}[{}] as {got:?}, the registry says {:?}",
                    row["id"], row[*field]
                );
            }
        }
    }
}

#[test]
fn every_language_row_is_well_formed() {
    let (header, langs) = registry("languages.tsv");
    assert_eq!(
        header,
        [
            "id", "label", "color", "tier", "source", "build", "run", "version", "mem", "size",
            "profile", "shown", "engine"
        ],
        "languages.tsv columns"
    );
    let unique: BTreeSet<String> = ids(&langs).into_iter().collect();
    assert_eq!(unique.len(), langs.len(), "a language id repeats");
    let placeholders = filled_placeholders();
    for p in [
        "{bench}",
        "{src}",
        "{bin}",
        "{dir}",
        "{n}",
        "{largo}",
        "{tmp}",
        "{goamd64}",
    ] {
        assert!(
            placeholders.contains(&p.to_string()),
            "suite_fill no longer fills {p}"
        );
    }
    for row in &langs {
        let id = &row["id"];
        assert!(
            matches!(
                row["tier"].as_str(),
                "logos" | "systems" | "managed" | "transpiled"
            ),
            "{id}: tier {:?}",
            row["tier"]
        );
        let color = row["color"].as_str();
        assert!(
            color.len() == 7
                && color.starts_with('#')
                && color[1..].chars().all(|c| c.is_ascii_hexdigit()),
            "{id}: colour {color:?}"
        );
        for flag in ["mem", "size", "profile"] {
            assert!(
                matches!(row[flag].as_str(), "0" | "1"),
                "{id}: {flag} is 0 or 1"
            );
        }
        for field in ["build", "run", "version"] {
            let mut rest = row[field].as_str();
            while let Some(i) = rest.find('{') {
                let end = rest[i..].find('}').map(|j| i + j + 1).unwrap_or(rest.len());
                assert!(
                    placeholders.contains(&rest[i..end].to_string()),
                    "{id}: {field} uses {}, a placeholder suite_fill does not fill",
                    &rest[i..end]
                );
                rest = &rest[end..];
            }
        }
        assert!(
            row["run"].contains("{n}"),
            "{id}: the run command passes the size"
        );
        assert!(
            repo_root()
                .join("benchmarks/programs/fib")
                .join(&row["source"])
                .exists(),
            "{id}: no {} in programs/fib",
            row["source"]
        );
    }
    let family: Vec<String> = langs
        .iter()
        .filter(|r| r["tier"] == "logos")
        .map(|r| r["id"].clone())
        .collect();
    assert_eq!(
        family,
        [
            "logos_release",
            "logos_c",
            "logos_cbc",
            "logos_jit",
            "logos_vm",
            "logos_tw"
        ],
        "the LOGOS family, in chart order: the built tiers, then the in-process engines"
    );
}

/// The placeholders `suite_fill` in lib/suite.sh substitutes, read from its body: `${t//\{name\}/…}` → `{name}`.
fn filled_placeholders() -> Vec<String> {
    let suite = read(&lib().join("suite.sh"));
    let start = suite
        .find("suite_fill() {")
        .expect("lib/suite.sh defines suite_fill");
    let body = &suite[start..start + suite[start..].find("\n}\n").expect("suite_fill ends")];
    let mut found = Vec::new();
    let mut rest = body;
    while let Some(i) = rest.find("\\{") {
        let after = &rest[i + 2..];
        let end = after.find("\\}").expect("a filled placeholder closes");
        let name = format!("{{{}}}", &after[..end]);
        if !found.contains(&name) {
            found.push(name);
        }
        rest = &after[end..];
    }
    found
}

/// The command the page shows is the command the run executes, with the scratch paths written plainly — the
/// methodology table can never describe a build the numbers did not come from.
#[test]
fn the_shown_command_is_the_build_command() {
    let (_, langs) = registry("languages.tsv");
    for row in langs
        .iter()
        .filter(|r| r["tier"] != "logos" && r["build"] != "-")
    {
        let mut build = row["build"].clone();
        for setup in ["cd {tmp} && ", "mkdir -p {dir} && "] {
            build = build.replace(setup, "");
        }
        let build = build
            .split_whitespace()
            .filter(|t| !t.contains("{tmp}"))
            .collect::<Vec<_>>()
            .join(" ")
            .replace("{src}", &row["source"])
            .replace("{bin}", "bench")
            .replace("{dir}", "classes");
        assert_eq!(
            build, row["shown"],
            "{}: the page shows a different command than the build runs",
            row["id"]
        );
    }
    let shown: BTreeMap<String, String> = langs
        .iter()
        .map(|r| (r["id"].clone(), r["shown"].clone()))
        .collect();
    assert_eq!(shown["logos_release"], "largo build --release");
    assert_eq!(shown["logos_c"], "largo build --release --emit c");
    assert_eq!(
        shown["logos_cbc"],
        "largo build --release --emit c-bytecode"
    );
    assert_eq!(shown["logos_jit"], "logos-jit-image-build; logos-jit-run");
    assert_eq!(shown["logos_vm"], "largo run --interpret --engine vm");
    assert_eq!(shown["logos_tw"], "largo run --interpret --engine tree");
}

/// A row that runs in process names the engine its run must report (`logos-engine: <engine>`, which the verify phase
/// checks, so a silent fallback is never timed as the engine the bar names), and it is the engine its `--engine`
/// selects: none or `jit` is the VM with the native tier, `vm` the VM alone, `tree` the tree-walker. A row that runs a
/// built artifact names none.
#[test]
fn every_in_process_row_names_the_engine_its_flag_selects() {
    let (_, langs) = registry("languages.tsv");
    for row in &langs {
        let run = row["run"].as_str();
        let want = if row["id"] == "logos_jit" && run.contains("/run-jit") {
            "vm+jit"
        } else if !run.contains(" run --interpret") {
            ""
        } else if run.contains("--engine tree") {
            "treewalker"
        } else if run.contains("--engine vm") {
            "vm"
        } else {
            assert!(
                !run.contains("--engine") || run.contains("--engine jit"),
                "{}: unknown engine in {run:?}",
                row["id"]
            );
            "vm+jit"
        };
        assert_eq!(
            row["engine"], want,
            "{}: runs `{run}`, so its engine is {want:?}",
            row["id"]
        );
    }
    let engines: BTreeSet<&str> = langs
        .iter()
        .map(|r| r["engine"].as_str())
        .filter(|e| !e.is_empty())
        .collect();
    assert_eq!(
        engines,
        BTreeSet::from(["treewalker", "vm", "vm+jit"]),
        "every in-process engine has a bar"
    );
}

#[test]
fn the_c_baseline_and_largo_emit_c_share_their_flags() {
    let (_, langs) = registry("languages.tsv");
    let c = langs.iter().find(|r| r["id"] == "c").expect("a C row");
    let release = logicaffeine_cli::project::build_c::C_RELEASE_FLAGS.join(" ");
    assert!(
        c["build"].starts_with(&format!("gcc {release} ")),
        "the hand-written C builds with {:?}; `largo build --release --emit c` with {release:?}",
        c["build"]
    );
}

/// Every row compiled by a C compiler — C and C++ through gcc and through clang, and Nim, which emits C — builds with the
/// release flags `largo build --release --emit c` uses, and none contracts `a*b + c` into a fused multiply-add: Rust,
/// Zig, Go, Java, JavaScript and every LOGOS tier round each operation, and a baseline that fuses computes different
/// floating-point values (and is faster for it). The other compiled rows take the same host-CPU, whole-program settings.
#[test]
fn every_compiled_row_takes_the_same_release_settings_and_strict_floats() {
    let (_, langs) = registry("languages.tsv");
    let row = |id: &str| {
        langs
            .iter()
            .find(|r| r["id"] == id)
            .unwrap_or_else(|| panic!("a {id} row"))
            .clone()
    };
    let release = logicaffeine_cli::project::build_c::C_RELEASE_FLAGS.join(" ");
    assert!(
        logicaffeine_cli::project::build_c::C_RELEASE_FLAGS.contains(&"-ffp-contract=off"),
        "largo's C release build must not contract floating point: {release:?}"
    );
    for (id, compiler, extra) in [
        ("c", "gcc", ""),
        ("c_clang", "clang", ""),
        ("cpp", "g++", " -std=c++17"),
        ("cpp_clang", "clang++", " -std=c++17"),
    ] {
        let r = row(id);
        assert!(
            r["build"].starts_with(&format!("{compiler} {release}{extra} ")),
            "{id} builds with {:?}; the C release flags are {release:?}",
            r["build"]
        );
    }
    assert_ne!(
        row("c")["label"],
        row("c_clang")["label"],
        "the two C compilers are told apart by their labels"
    );
    let nim = row("nim");
    assert!(
        nim["build"].contains("-d:danger"),
        "Nim builds in its fastest release mode: {}",
        nim["build"]
    );
    assert!(
        nim["build"].contains("-ffp-contract=off"),
        "Nim's C compiler must not contract floating point: {}",
        nim["build"]
    );
    let rust = row("rust")["build"].clone();
    for flag in [
        "-C opt-level=3",
        "-C lto=fat",
        "-C codegen-units=1",
        "-C target-cpu=native",
        "-C panic=abort",
    ] {
        assert!(
            rust.contains(flag),
            "the Rust baseline takes LOGOS (Rust)'s release profile ({flag}): {rust}"
        );
    }
    assert!(
        row("go")["build"].starts_with("GOAMD64={goamd64} go build"),
        "Go targets the host CPU's x86-64 level"
    );
}

/// `{goamd64}` is the host's x86-64 microarchitecture level, read from the CPU's flags as `-march=native` reads them.
#[test]
fn the_go_level_is_the_hosts() {
    let dir = tempfile::TempDir::new().unwrap();
    let level = |flags: &str| {
        std::fs::write(
            dir.path().join("cpuinfo"),
            format!("processor\t: 0\nflags\t\t: {flags}\n\n"),
        )
        .unwrap();
        let out = Command::new("bash")
            .arg("-c")
            .arg(format!(
                ". '{}' && suite_goamd64",
                lib().join("suite.sh").display()
            ))
            .env("BENCH_PROC_ROOT", dir.path())
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "suite_goamd64 failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap().trim().to_string()
    };
    let v2 = "fpu cx16 lahf_lm popcnt pni sse4_1 sse4_2 ssse3";
    let v3 = format!("{v2} avx avx2 bmi1 bmi2 f16c fma abm movbe xsave osxsave");
    let v4 = format!("{v3} avx512f avx512bw avx512cd avx512dq avx512vl");
    assert_eq!(level("fpu sse sse2"), "v1");
    assert_eq!(level(v2), "v2");
    assert_eq!(level(&v3), "v3");
    assert_eq!(level(&v4), "v4");
    assert_eq!(
        level(&v3.replace(" fma", "")),
        "v2",
        "v3 needs every one of its features"
    );
}

#[test]
fn every_limit_names_a_real_language_and_benchmark() {
    let (_, benches) = registry("benchmarks.tsv");
    let (_, langs) = registry("languages.tsv");
    let text = read(&lib().join("limits.tsv"));
    for line in text.lines().skip(1).filter(|l| !l.trim().is_empty()) {
        let f: Vec<&str> = line.split('\t').collect();
        assert_eq!(
            f.len(),
            4,
            "limits.tsv: lang, bench, max_n, reason: {line:?}"
        );
        assert!(
            langs.iter().any(|r| r["id"] == f[0]),
            "limits.tsv: unknown language {}",
            f[0]
        );
        assert!(
            benches.iter().any(|r| r["id"] == f[1]),
            "limits.tsv: unknown benchmark {}",
            f[1]
        );
        assert!(
            f[2].parse::<u64>().is_ok() && !f[3].trim().is_empty(),
            "limits.tsv: a size and a reason: {line:?}"
        );
    }
}

/// The scripts that run benchmarks, and the chart generator.
fn suite_scripts() -> Vec<PathBuf> {
    let mut files = Vec::new();
    for dir in ["benchmarks", "scripts/bench", "scripts/dev"] {
        for e in std::fs::read_dir(repo_root().join(dir))
            .expect(dir)
            .flatten()
        {
            let p = e.path();
            if p.extension().is_some_and(|x| x == "sh" || x == "py") {
                files.push(p);
            }
        }
    }
    files.sort();
    files
}

/// No script carries its own copy of the benchmark list or a per-benchmark table: a list goes stale the day a
/// program is added (three were left unmeasured that way), a copied name or size drifts from the registry.
#[test]
fn no_script_keeps_its_own_benchmark_list_or_table() {
    let (_, benches) = registry("benchmarks.tsv");
    let all = ids(&benches);
    let mut offenders = Vec::new();
    for path in suite_scripts() {
        let text = read(&path);
        let rel = path
            .strip_prefix(repo_root())
            .unwrap()
            .display()
            .to_string();
        let literal_list = text.match_indices("BENCHMARKS=(").any(|(i, _)| {
            let after = text[i + "BENCHMARKS=(".len()..].trim_start();
            all.iter().any(|b| after.starts_with(b.as_str()))
        });
        if literal_list || text.contains("BENCH_ORDER = [") || text.contains("COLLAPSES = (\"") {
            offenders.push(format!("{rel}: declares BENCHMARKS=( … )"));
        }
        let arms = all
            .iter()
            .filter(|b| {
                text.contains(&format!("{b})"))
                    || text.contains(&format!("[{b}]="))
                    || text.contains(&format!("\"{b}\": \""))
            })
            .count();
        if arms >= 10 {
            offenders.push(format!("{rel}: a table with {arms} per-benchmark arms"));
        }
        if text.contains("fib ackermann nqueens") || text.contains("\"fib\", \"ackermann\"") {
            offenders.push(format!("{rel}: lists the benchmarks inline"));
        }
    }
    assert!(
        offenders.is_empty(),
        "the registry is the one list:\n  {}",
        offenders.join("\n  ")
    );
}

/// Every timed or profiled command in every benchmark script — the publishable run, the development loops, the parity
/// gate and the profiler — runs pinned to the timed CPUs, so no measurement lands on an efficiency core while the one
/// it is compared with ran on a performance core. A tool asked for its version is not a measurement.
#[test]
fn every_timed_command_runs_pinned() {
    let mut scripts = Vec::new();
    for dir in ["benchmarks", "scripts/bench"] {
        for e in std::fs::read_dir(repo_root().join(dir))
            .expect(dir)
            .flatten()
        {
            if e.path().extension().is_some_and(|x| x == "sh") {
                scripts.push(e.path());
            }
        }
    }
    scripts.sort();
    assert!(
        scripts.iter().any(|p| p.ends_with("benchmarks/run.sh"))
            && scripts
                .iter()
                .any(|p| p.ends_with("scripts/bench/profile-benchmarks.sh")),
        "the scan covers the publishable run and the profiler: {scripts:?}"
    );
    let mut unpinned = Vec::new();
    for path in &scripts {
        let rel = path
            .strip_prefix(repo_root())
            .unwrap()
            .display()
            .to_string();
        let text = read(path);
        for (i, line) in text.lines().enumerate() {
            let code = line.split('#').next().unwrap_or("");
            for tool in [
                "hyperfine \\",
                "hyperfine --",
                "hyperfine \"",
                "hyperfine $",
                "/usr/bin/time -",
                "perf record",
                "perf stat",
            ] {
                let as_command = code
                    .match_indices(tool)
                    .any(|(at, _)| code[..at].matches('"').count() % 2 == 0);
                if as_command
                    && !code.contains(&format!(
                        "{} --version",
                        tool.split_whitespace().next().unwrap()
                    ))
                {
                    let pinned = code.contains("pinned ") || code.contains("taskset -c");
                    if !pinned {
                        unpinned.push(format!("{rel}:{}: {}", i + 1, line.trim()));
                    }
                }
            }
        }
    }
    assert!(
        unpinned.is_empty(),
        "timed commands must run through `pinned` (taskset to the timed CPUs):\n  {}",
        unpinned.join("\n  ")
    );
    let suite = read(&lib().join("suite.sh"));
    assert!(
        suite.contains("pinned() { taskset -c \"$(suite_cpus)\" \"$@\"; }"),
        "pinned() is taskset to suite_cpus"
    );
}

/// A benchmark's output is its proof of work. At every size it is timed at, the run verifies it byte for byte, so it
/// must exist; it must change from one size to the next (an answer that ignores n proves nothing about the work at n);
/// and it must never be a bare zero (two_sum once printed 0 at every timed size: no pair could ever match).
#[test]
fn every_benchmark_output_proves_the_work() {
    let (_, benches) = registry("benchmarks.tsv");
    for id in ids(&benches) {
        let dir = repo_root().join("benchmarks/programs").join(&id);
        let sizes: Vec<String> = read(&dir.join("sizes.txt"))
            .split_whitespace()
            .map(str::to_string)
            .collect();
        assert!(!sizes.is_empty(), "{id}: sizes.txt is empty");
        let mut seen: BTreeMap<String, String> = BTreeMap::new();
        for n in &sizes {
            let path = dir.join(format!("expected_{n}.txt"));
            let text = read(&path);
            assert!(
                text.ends_with('\n') && !text.trim().is_empty(),
                "{id} at n={n}: output must be non-empty lines"
            );
            let digits: String = text.chars().filter(|c| c.is_ascii_digit()).collect();
            assert!(
                digits.chars().any(|c| c != '0'),
                "{id} at n={n}: the output {text:?} is all zeros — it proves no work"
            );
            if let Some(other) = seen.insert(text.clone(), n.clone()) {
                panic!("{id}: the output at n={n} is the same as at n={other} ({text:?}) — it does not depend on the work");
            }
        }
    }
}

/// The output rule, ratcheted across the whole suite: a benchmark that builds a result collection prints the additive
/// fold `h = (h * 31 + v) & 268435455` over all of it, in order (an order-sensitive proof: a sort that left two
/// elements swapped, a reversal that missed the middle, a product computed from row sums all change it); a benchmark
/// whose result is one value or a fixed tuple prints that answer. benchmarks.tsv says which, and says in words what the
/// output is. Every implementation of a `fold` benchmark — all eleven languages and interp.lg — carries the fold.
#[test]
fn every_benchmark_declares_its_proof_and_fold_benchmarks_fold_in_every_language() {
    let (_, benches) = registry("benchmarks.tsv");
    let (_, langs) = registry("languages.tsv");
    let mut sources: Vec<String> = langs.iter().map(|r| r["source"].clone()).collect();
    sources.extend([
        "main.py".to_string(),
        "main.rb".to_string(),
        "interp.lg".to_string(),
    ]);
    sources.sort();
    sources.dedup();
    let mut missing = Vec::new();
    for row in &benches {
        let id = &row["id"];
        assert!(
            matches!(row["proof"].as_str(), "fold" | "answer"),
            "{id}: proof is `fold` or `answer`, not {:?}",
            row["proof"]
        );
        assert!(
            !row["output"].trim().is_empty(),
            "{id}: benchmarks.tsv says what the output is"
        );
        if row["proof"] != "fold" {
            continue;
        }
        for source in &sources {
            let path = repo_root()
                .join("benchmarks/programs")
                .join(id)
                .join(source);
            let text = read(&path);
            let folds = text.contains("268435455") && text.contains("31");
            if !folds {
                missing.push(format!("{id}/{source}"));
            }
        }
    }
    assert!(
        missing.is_empty(),
        "these implementations of a `fold` benchmark do not fold their result:\n  {}",
        missing.join("\n  ")
    );
    let folds = benches.iter().filter(|r| r["proof"] == "fold").count();
    assert!(
        folds >= 18,
        "the fold rule covers every benchmark that builds a collection; only {folds} do"
    );
}
