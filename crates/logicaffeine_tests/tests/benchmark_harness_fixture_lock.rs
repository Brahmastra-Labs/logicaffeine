//! ★ LOCK — the real `benchmarks/run.sh`, run end to end on stub toolchains. ★
//!
//! The authoritative benchmark run takes hours on a quiet machine, so its harness has to be right before it starts.
//! This lock runs the actual script over two benchmarks with every external tool replaced by a stub that answers the way
//! the real one does: each compiler writes a program that prints the benchmark's expected bytes, `largo` builds,
//! emits and runs, `hyperfine` writes its JSON, and the machine (`/proc`, `/sys`) is a fixture. Every registry row
//! must then be built, verified at every timed size, timed on the pinned CPUs and assembled into `latest.json` with
//! its registry label, version, build command, generated sources and the run's environment; and a busy machine, a
//! failed `largo` build, a wrong answer, a JIT run that never reached the JIT or an engine row that ran on another engine
//! must each stop the run by name, and
//! every timing must run on the machine's performance cores. The
//! run writes only into its temporary directories, never the checkout's `bin/`, `generated/`, `results/` or `logs/`.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test benchmark_harness_fixture_lock`

#![cfg(all(unix, not(target_arch = "wasm32")))]

use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;
use tempfile::TempDir;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
}

/// The two benchmarks every fixture run covers: one the optimizer collapses, one it does not.
const ONLY: &str = "fib,bubble_sort";

/// One stub for every tool the registry's templates call, dispatched on the name it was invoked by.
const TOOLSTUB: &str = r##"#!/usr/bin/env bash
set -euo pipefail
tool="$(basename "$0")"
programs="$FIXTURE_PROGRAMS"
marker='// ==== LOGOS user code (device-liftable) ===='

for a in "$@"; do
    case "$a" in --version|-version|version) echo "$tool fixture 1.0"; exit 0 ;; esac
done

# A program that prints benchmark $1's expected output at the size it is given; `wrong` prints something else.
program() {  # bench out [wrong]
    if [ "${3:-}" = wrong ]; then
        printf '#!/bin/sh\necho not-the-answer\n' > "$2"
    else
        printf '#!/bin/sh\nexec cat "%s/%s/expected_$1.txt"\n' "$programs" "$1" > "$2"
    fi
    chmod +x "$2"
}
bench_of() { basename "$(dirname "$1")"; }
source_of() { for a in "$@"; do case "$a" in *.c|*.cpp|*.rs|*.zig|*.go|*.nim|*.java|*.js) echo "$a"; return ;; esac; done; }
# The benchmark a LOGOS project in the current directory holds.
project_bench() {
    local d
    for d in "$programs"/*/; do
        if cmp -s "$d/main.lg" src/main.lg; then basename "$d"; return; fi
    done
    echo "fixture largo: src/main.lg is no benchmark program" >&2; exit 2
}

case "$tool" in
    gcc|g++|clang|clang++|rustc|go)
        src="$(source_of "$@")"; out=""
        while [ $# -gt 0 ]; do [ "$1" = -o ] && out="$2"; shift; done
        program "$(bench_of "$src")" "$out" ;;
    zig)
        src="$(source_of "$@")"
        for a in "$@"; do case "$a" in -femit-bin=*) out="${a#-femit-bin=}" ;; esac; done
        program "$(bench_of "$src")" "$out" ;;
    nim)
        src="$(source_of "$@")"
        for a in "$@"; do case "$a" in -o:*) out="${a#-o:}" ;; esac; done
        program "$(bench_of "$src")" "$out" ;;
    javac)
        src="$(source_of "$@")"; dir=""
        while [ $# -gt 0 ]; do [ "$1" = -d ] && dir="$2"; shift; done
        mkdir -p "$dir"; echo fixture > "$dir/Main.class"; bench_of "$src" > "$dir/bench" ;;
    java)
        dir=""; prev=""
        for a in "$@"; do [ "$prev" = -cp ] && dir="$a"; prev="$a"; done
        n="${!#}"; exec cat "$programs/$(cat "$dir/bench")/expected_$n.txt" ;;
    node)
        src="$(source_of "$@")"; n="${!#}"; exec cat "$programs/$(bench_of "$src")/expected_$n.txt" ;;
    git)
        echo fixture ;;
    cargo)
        if [ -n "${FIXTURE_CARGO_FAILS:-}" ]; then echo "error: fixture cargo failure" >&2; exit 101; fi
        mkdir -p "$CARGO_TARGET_DIR/release"
        cp "$FIXTURE_STUBS/toolstub" "$CARGO_TARGET_DIR/release/largo"
        ;;
    largo)
        args=" $* "
        if [[ "$args" == *" opts "* ]]; then
            echo '{"fired":["fixture-pass"],"paths":{},"stages":{"compiled":{"optimizer":["fixture-pass"]},"c":{"optimizer":["fixture-pass"]},"c_bytecode":{"optimizer":["fixture-pass"]},"run":{"optimizer":["fixture-pass"]},"vm":{"optimizer":["fixture-pass"]},"jit":{"optimizer":["fixture-pass"]}},"jit_native":{"lowering":[],"functions":{"attempted":0,"compiled":0},"loops":{"attempted":0,"compiled":0},"declines":""},"changes_algorithm":[],"blockers":[],"dependencies":[]}'
        elif [[ "$args" == *" build "* ]]; then
            bench="$(project_bench)"; mode=debug; [[ "$args" == *" --release "* ]] && mode=release
            mkdir -p "target/$mode/build/src" "target/$mode/build-c"
            if [[ "$args" == *" --emit c "* ]]; then
                printf '/* fixture runtime */\n%s\nint main(void) { return 0; } /* %s %s */\n' "$marker" "$bench" "$FIXTURE_TOKEN" > "target/$mode/build-c/main.c"
                program "$bench" "target/$mode/bench" "$([ "${FIXTURE_WRONG:-}" = logos_c ] && echo wrong || true)"
            elif [[ "$args" == *" --emit c-bytecode "* ]]; then
                printf '/* fixture runtime */\n%s\nint main(void) { return 0; } /* bytecode %s %s */\n' "$marker" "$bench" "$FIXTURE_TOKEN" > "target/$mode/build-c/main.c"
                program "$bench" "target/$mode/bench"
            else
                printf '// fixture Rust for %s %s\nfn main() {}\n' "$bench" "$FIXTURE_TOKEN" > "target/$mode/build/src/main.rs"
                program "$bench" "target/$mode/bench"
            fi
        elif [[ "$args" == *" run "* ]]; then
            bench="$(project_bench)"; n="${!#}"
            engine=jit
            [[ "$args" == *" --engine vm "* ]] && engine=vm
            [[ "$args" == *" --engine tree "* ]] && engine=tree
            [ -n "${FIXTURE_ENGINE_IGNORED:-}" ] && engine=jit
            if [ -n "${LOGOS_ENGINE_TRACE:-}" ]; then
                case "$engine" in
                    vm) echo "logos-engine: vm" >&2 ;;
                    tree) echo "logos-engine: treewalker" >&2 ;;
                    *) if [ -z "${FIXTURE_NO_JIT:-}" ]; then
                           echo "logos-engine: vm+jit" >&2
                           case "${FIXTURE_JIT_DECLINE:-}" in
                               missing)
                                   echo "logos-engine: native-tier fixture: 1/2 functions, 3/3 loops compiled" >&2 ;;
                               unclassified)
                                   echo "logos-engine: native-tier fixture: 1/2 functions, 3/3 loops compiled" >&2
                                   echo "logos-engine: native-declines fixture: functions[unclassified=1]" >&2 ;;
                               *) echo "logos-engine: native-tier fixture: 2/2 functions, 3/3 loops compiled" >&2 ;;
                           esac
                       fi ;;
                esac
            fi
            # A stalled engine at one size: what a slow engine looks like at a size past its reach.
            if [ "${FIXTURE_STALL:-}" = "$engine:$n" ]; then sleep 3; fi
            if [ "$engine" = jit ] && [ -n "${LOGOS_JIT_CODE:-}" ]; then
                mkdir -p "$LOGOS_JIT_CODE"
                printf '\303\220\220' > "$LOGOS_JIT_CODE/0.bin"
                printf '0\t1000\t3\tlogos::%s [jit]\n' "$bench" > "$LOGOS_JIT_CODE/index.tsv"
            fi
            exec cat "$programs/$bench/expected_$n.txt"
        else
            echo "fixture largo: unexpected arguments: $*" >&2; exit 2
        fi ;;
    objdump)
        file="${!#}"
        echo "$file:     file format binary"
        echo
        echo "Disassembly of section .data:"
        echo
        echo "0000000000001000 <.data>:"
        echo "    1000:	c3                   	ret    fixture-objdump $FIXTURE_TOKEN" ;;
    hyperfine)
        json=""; name=""; cmd=""
        while [ $# -gt 0 ]; do
            case "$1" in
                --export-json) json="$2"; shift ;;
                -n) name="$2"; shift ;;
                --warmup|--runs|--time-unit) shift ;;
                --*) ;;
                *) cmd="$1" ;;
            esac
            shift
        done
        taskset -cp $$ | sed 's/.*: //' >> "$FIXTURE_AFFINITY"
        bash -c "$cmd" > /dev/null
        case "$name" in
            c) s=0.010 ;; logos_release) s=0.008 ;; logos_c) s=0.009 ;; logos_cbc) s=0.0095 ;; logos_jit) s=0.020 ;;
            logos_vm) s=0.050 ;; logos_tw) s=0.200 ;; *) s=0.012 ;;
        esac
        printf '{"results":[{"command":"%s","mean":%s,"stddev":0.0001,"median":%s,"user":%s,"system":0.0,"min":%s,"max":%s,"times":[%s,%s]}]}\n' \
            "$name" "$s" "$s" "$s" "$s" "$s" "$s" "$s" > "$json" ;;
    *)
        echo "toolstub: no stub for $tool" >&2; exit 2 ;;
esac
"##;

const TOOLS: &[&str] = &[
    "gcc",
    "g++",
    "clang",
    "clang++",
    "rustc",
    "zig",
    "go",
    "javac",
    "java",
    "node",
    "nim",
    "git",
    "cargo",
    "hyperfine",
    "objdump",
];

/// A fixture machine and toolchain, and the directories one run writes into.
struct Fixture {
    root: TempDir,
}

impl Fixture {
    fn new(load: &str) -> Fixture {
        let root = TempDir::new().expect("temp dir");
        let f = Fixture { root };
        let stubs = f.path("stubs");
        std::fs::create_dir_all(&stubs).unwrap();
        let stub = stubs.join("toolstub");
        std::fs::write(&stub, TOOLSTUB).unwrap();
        std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).unwrap();
        for tool in TOOLS {
            symlink(&stub, stubs.join(tool)).unwrap();
        }
        let proc = f.path("proc");
        std::fs::create_dir_all(&proc).unwrap();
        std::fs::write(
            proc.join("loadavg"),
            format!("{load} 0.10 0.10 1/100 12345\n"),
        )
        .unwrap();
        std::fs::write(
            proc.join("cpuinfo"),
            "model name\t: fixture\nflags\t\t: fpu sse2\n",
        )
        .unwrap();
        let sys = f.path("sys");
        std::fs::create_dir_all(sys.join("devices/cpu_core")).unwrap();
        std::fs::write(sys.join("devices/cpu_core/cpus"), "0\n").unwrap();
        let freq = sys.join("devices/system/cpu/cpu0/cpufreq");
        std::fs::create_dir_all(&freq).unwrap();
        std::fs::write(freq.join("scaling_governor"), "performance\n").unwrap();
        f
    }

    fn path(&self, rel: &str) -> PathBuf {
        self.root.path().join(rel)
    }

    /// A mark unique to this fixture that its stub `largo` writes into every source it generates, so a recorded
    /// source left over from another run can never pass for this run's.
    fn token(&self) -> String {
        self.root
            .path()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned()
    }

    /// Run the real `benchmarks/run.sh` against this fixture, with `extra` added to its environment.
    fn run(&self, extra: &[(&str, &str)]) -> Output {
        let path = format!(
            "{}:{}",
            self.path("stubs").display(),
            std::env::var("PATH").unwrap_or_default()
        );
        let mut cmd = Command::new("bash");
        cmd.arg(repo_root().join("benchmarks/run.sh"));
        for (key, _) in std::env::vars_os() {
            let key = key.to_string_lossy().into_owned();
            if key.starts_with("BENCH_") || key.starts_with("LOGOS_") || key.starts_with("FIXTURE_")
            {
                cmd.env_remove(&key);
            }
        }
        cmd.env("PATH", path)
            .env("FIXTURE_PROGRAMS", repo_root().join("benchmarks/programs"))
            .env("FIXTURE_STUBS", self.path("stubs"))
            .env("FIXTURE_AFFINITY", self.path("affinity"))
            .env("FIXTURE_TOKEN", self.token())
            .env("BENCH_ONLY", ONLY)
            .env("BENCH_RESULTS_DIR", self.path("results"))
            .env("BENCH_WORK_DIR", self.path("work"))
            .env("BENCH_TARGET_DIR", self.path("target"))
            .env("BENCH_LOG_DIR", self.path("logs"))
            .env("BENCH_PROC_ROOT", self.path("proc"))
            .env("BENCH_SYS_ROOT", self.path("sys"))
            .env("BENCH_MEMORY_MAX", "off")
            .env("BENCH_QUIET_WAIT", "0")
            .env("BENCH_RUNS", "2")
            .env("BENCH_WARMUP", "0")
            .env("RUN_INTERP", "0")
            .env("RUN_CODEC", "0");
        for (k, v) in extra {
            cmd.env(k, v);
        }
        cmd.output().expect("run bash benchmarks/run.sh")
    }
}

fn text(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// A benchmark's column in `lib/benchmarks.tsv`.
fn benchmark_column(id: &str, column: &str) -> String {
    let tsv = std::fs::read_to_string(repo_root().join("benchmarks/lib/benchmarks.tsv")).unwrap();
    let mut lines = tsv.lines();
    let header: Vec<&str> = lines.next().unwrap().split('\t').collect();
    let at = header
        .iter()
        .position(|h| *h == column)
        .unwrap_or_else(|| panic!("no {column} column"));
    let row = lines
        .find(|l| l.split('\t').next() == Some(id))
        .unwrap_or_else(|| panic!("no {id} row"));
    row.split('\t').nth(at).unwrap().to_string()
}

/// The registry's language rows: (id, label).
fn languages() -> Vec<(String, String)> {
    let tsv = std::fs::read_to_string(repo_root().join("benchmarks/lib/languages.tsv")).unwrap();
    tsv.lines()
        .skip(1)
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            (f[0].to_string(), f[1].to_string())
        })
        .collect()
}

/// What the run must never touch in the checkout: the committed results and the default work directories.
fn checkout_state() -> Vec<(PathBuf, Option<std::time::SystemTime>)> {
    let b = repo_root().join("benchmarks");
    [
        b.join("results/latest.json"),
        b.join("bin"),
        b.join("generated"),
        b.join("results/raw"),
    ]
    .into_iter()
    .map(|p| {
        let t = std::fs::metadata(&p).and_then(|m| m.modified()).ok();
        (p, t)
    })
    .collect()
}

#[test]
fn a_full_run_builds_verifies_times_and_assembles_every_registry_row() {
    let before = checkout_state();
    let fx = Fixture::new("0.05");
    let out = fx.run(&[]);
    let log = text(&out);
    assert!(out.status.success(), "the fixture run failed:\n{log}");
    assert_eq!(
        checkout_state(),
        before,
        "the run touched the checkout's results, bin/ or generated/"
    );

    let latest: Value = serde_json::from_str(
        &std::fs::read_to_string(fx.path("results/latest.json")).expect("latest.json"),
    )
    .unwrap();
    let rows = languages();

    let shown: Vec<(&str, &str)> = latest["languages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| (l["id"].as_str().unwrap(), l["label"].as_str().unwrap()))
        .collect();
    for (id, label) in &rows {
        assert!(
            shown.contains(&(id.as_str(), label.as_str())),
            "{id} must appear as `{label}`; the run showed {shown:?}"
        );
    }

    let meta = &latest["metadata"];
    for (id, _) in &rows {
        assert!(
            meta["versions"][id].is_string(),
            "versions must be keyed by registry id; {id} is missing"
        );
        assert!(
            meta["build_commands"][id].is_string(),
            "build_commands must carry {id}"
        );
    }
    assert!(
        meta["versions"]["hyperfine"].is_string(),
        "the hyperfine version is recorded"
    );
    let env = &meta["environment"];
    assert_eq!(
        env["pinning"]["cpus"], "0",
        "timed CPUs come from the machine's performance cores"
    );
    assert_eq!(env["pinning"]["source"], "performance-cores");
    assert_eq!(env["governor"], "performance");
    assert_eq!(env["quiet_box"]["gate"], "passed");
    assert_eq!(env["load"]["start"], 0.05);

    let summary = &latest["summary"];
    for tier in [
        "logos_release",
        "logos_c",
        "logos_cbc",
        "logos_jit",
        "logos_vm",
        "logos_tw",
    ] {
        assert!(
            summary["geometric_mean_speedup_vs_c"][tier].is_number(),
            "the summary carries {tier}"
        );
        assert_eq!(
            summary["coverage"][tier]["benchmarks"], 2,
            "{tier} covers both benchmarks"
        );
    }
    assert_eq!(
        summary["geometric_mean_speedup_vs_c"]["logos_release"], 1.25,
        "the geomean is C's time over the tier's (10 ms / 8 ms)"
    );

    for id in ONLY.split(',') {
        let bench = latest["benchmarks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|b| b["id"] == id)
            .expect("benchmark record");
        let sizes = std::fs::read_to_string(
            repo_root().join(format!("benchmarks/programs/{id}/sizes.txt")),
        )
        .unwrap();
        for n in sizes.split_whitespace() {
            for (lang, _) in &rows {
                assert!(
                    bench["scaling"][n][lang]["mean_ms"].is_number(),
                    "{id} has a {lang} timing at n={n}"
                );
            }
        }
        let c = bench["generated_c"].as_str().unwrap();
        assert!(
            c.contains(&format!("/* {id} {} */", fx.token())),
            "{id}'s generated C is recorded from this run:\n{c}"
        );
        assert!(
            !c.contains("fixture runtime"),
            "only the program's part of the C is recorded:\n{c}"
        );
        let rust = bench["generated_rust"].as_str().unwrap();
        assert!(
            rust.contains(&format!("{id} {}", fx.token())),
            "{id}'s generated Rust is recorded from this run:\n{rust}"
        );
        let interp = &bench["scaling"][benchmark_column(id, "interp_size")];
        for (lang, _) in &rows {
            assert!(
                interp[lang]["mean_ms"].is_number(),
                "{id} has a {lang} timing at its interpreter-scale size"
            );
        }
        let bytecode_c = bench["generated_c_bytecode"].as_str().unwrap();
        assert!(
            bytecode_c.contains(&format!("/* bytecode {id} {} */", fx.token())),
            "{id}'s C from its bytecode is recorded from this run:\n{bytecode_c}"
        );
        assert!(
            !bytecode_c.contains("fixture runtime"),
            "only the program's part of the bytecode C is recorded"
        );
        assert_eq!(
            bench["jit"]["engine"], "vm+jit",
            "{id}'s JIT run is recorded as the VM with the JIT"
        );
        assert_eq!(
            bench["fired"][0], "fixture-pass",
            "{id}'s optimizer trace is recorded"
        );
        assert!(
            bench["compilation"]["logos_c"]["mean_ms"].is_number(),
            "{id}'s `largo build --release --emit c` is timed"
        );
    }
    assert!(
        fx.path("work/bin").is_dir() && fx.path("work/generated/fib.c").is_file(),
        "the run's work stays in BENCH_WORK_DIR"
    );

    // The machine code the JIT emitted, disassembled, beside latest.json: what the LOGOS (JIT) tab shows.
    let code: Value = serde_json::from_str(
        &std::fs::read_to_string(fx.path("results/latest-jit-code.json"))
            .expect("latest-jit-code.json"),
    )
    .unwrap();
    for id in ONLY.split(',') {
        let regions = code["benchmarks"][id]
            .as_array()
            .unwrap_or_else(|| panic!("{id} has its JIT code recorded"));
        let region = regions
            .iter()
            .find(|r| r["name"] == format!("logos::{id} [jit]"))
            .expect("the dumped region");
        assert_eq!(region["bytes"], 3, "{id}: the region's size");
        let asm = region["asm"].as_str().unwrap();
        assert!(
            asm.contains(&format!("fixture-objdump {}", fx.token())),
            "{id}: disassembled by this run:\n{asm}"
        );
    }

    // Every timing and compile timing ran on the machine's performance cores (the fixture's are CPU 0).
    let affinity = std::fs::read_to_string(fx.path("affinity")).expect("hyperfine ran");
    let masks: Vec<&str> = affinity.lines().collect();
    assert!(
        masks.len() >= 2 * 3 * rows.len(),
        "hyperfine ran for every row at every size: {} runs",
        masks.len()
    );
    assert!(
        masks.iter().all(|m| *m == "0"),
        "every hyperfine run is pinned to CPU 0: {masks:?}"
    );
}

#[test]
fn a_busy_machine_stops_the_run_before_anything_is_built() {
    let fx = Fixture::new("30.00");
    let out = fx.run(&[]);
    let log = text(&out);
    assert!(
        !out.status.success(),
        "a run on a loaded machine must stop:\n{log}"
    );
    assert!(
        log.contains("the machine never went quiet"),
        "the run must say why:\n{log}"
    );
    assert!(
        log.contains("load 30.00"),
        "the gate must name the load:\n{log}"
    );
    assert!(
        !fx.path("work/bin").exists(),
        "nothing is built on a busy machine"
    );
}

#[test]
fn a_failed_largo_build_stops_the_run() {
    let fx = Fixture::new("0.05");
    let out = fx.run(&[("FIXTURE_CARGO_FAILS", "1")]);
    let log = text(&out);
    assert!(
        !out.status.success(),
        "a run without largo must stop:\n{log}"
    );
    assert!(
        log.contains("largo build failed"),
        "the run must say largo did not build:\n{log}"
    );
    assert!(
        !fx.path("results/latest.json").exists(),
        "no results are written"
    );
}

#[test]
fn a_wrong_answer_stops_the_run_by_name() {
    let fx = Fixture::new("0.05");
    let out = fx.run(&[("FIXTURE_WRONG", "logos_c")]);
    let log = text(&out);
    assert!(
        !out.status.success(),
        "a tier printing the wrong bytes must stop the run:\n{log}"
    );
    assert!(
        log.contains("LOGOS (C): printed"),
        "the failure must name the tier:\n{log}"
    );
    assert!(
        log.contains("Phase 2 failed"),
        "it stops in verification:\n{log}"
    );
    assert!(
        !fx.path("results/latest.json").exists(),
        "no results are written"
    );
}

#[test]
fn a_jit_decline_without_a_reason_stops_the_run() {
    let fx = Fixture::new("0.05");
    let out = fx.run(&[("FIXTURE_JIT_DECLINE", "missing"), ("BENCH_ONLY", "fib")]);
    let log = text(&out);
    assert!(
        !out.status.success(),
        "an unexplained native fallback must stop the run:\n{log}"
    );
    assert!(
        log.contains("native compile(s) declined without a reason"),
        "the ratchet must say why it stopped:\n{log}"
    );
    assert!(
        !fx.path("results/latest.json").exists(),
        "no results are written"
    );
}

#[test]
fn an_unclassified_jit_decline_stops_the_run() {
    let fx = Fixture::new("0.05");
    let out = fx.run(&[
        ("FIXTURE_JIT_DECLINE", "unclassified"),
        ("BENCH_ONLY", "fib"),
    ]);
    let log = text(&out);
    assert!(
        !out.status.success(),
        "an unclassified native fallback must stop the run:\n{log}"
    );
    assert!(
        log.contains("unclassified native fallback"),
        "the ratchet must name the bad trace:\n{log}"
    );
    assert!(
        !fx.path("results/latest.json").exists(),
        "no results are written"
    );
}

#[test]
fn an_engine_row_that_ran_on_another_engine_stops_the_run() {
    let fx = Fixture::new("0.05");
    let out = fx.run(&[("FIXTURE_ENGINE_IGNORED", "1")]);
    let log = text(&out);
    assert!(
        !out.status.success(),
        "a LOGOS (VM) row that ran on the JIT must stop the run:\n{log}"
    );
    assert!(
        log.contains("LOGOS (VM): did not run on the VM"),
        "the failure must name the row and its engine:\n{log}"
    );
    assert!(
        log.contains("LOGOS (tree-walker): did not run on the tree-walker"),
        "every misrouted row is named:\n{log}"
    );
}

/// A run that skips compile timing still records the sources this run generated, never a checkout's older ones: the
/// assembler reads them from the run's own work directory.
#[test]
fn a_run_without_compile_timing_records_its_own_generated_sources() {
    let fx = Fixture::new("0.05");
    let out = fx.run(&[("MEASURE_COMPILE", "0"), ("BENCH_ONLY", "fib")]);
    let log = text(&out);
    assert!(out.status.success(), "the run fails:\n{log}");
    let latest: Value = serde_json::from_str(
        &std::fs::read_to_string(fx.path("results/latest.json")).expect("latest.json"),
    )
    .unwrap();
    let fib = latest["benchmarks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["id"] == "fib")
        .expect("fib");
    let token = fx.token();
    for (field, mark) in [
        ("generated_c", format!("/* fib {token} */")),
        (
            "generated_c_bytecode",
            format!("/* bytecode fib {token} */"),
        ),
        ("generated_rust", format!("fib {token}")),
    ] {
        let text = fib[field].as_str().unwrap_or_default();
        assert!(text.contains(&mark), "{field} is this run's:\n{text}");
    }
}

/// A row that runs out of time at a size is skipped there and at every larger size, and still timed at every smaller
/// one: the tree-walker stalls only at fib's largest size.
#[test]
fn a_row_that_times_out_at_its_largest_size_keeps_its_smaller_ones() {
    let fx = Fixture::new("0.05");
    let out = fx.run(&[
        ("FIXTURE_STALL", "tree:40"),
        ("BENCH_TIMEOUT", "1"),
        ("BENCH_ONLY", "fib"),
    ]);
    let log = text(&out);
    assert!(out.status.success(), "a timeout is not a failure:\n{log}");
    let latest: Value = serde_json::from_str(
        &std::fs::read_to_string(fx.path("results/latest.json")).expect("latest.json"),
    )
    .unwrap();
    let fib = latest["benchmarks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["id"] == "fib")
        .expect("fib");
    for n in ["30", "31", "35"] {
        assert!(
            fib["scaling"][n]["logos_tw"]["mean_ms"].is_number(),
            "the tree-walker is timed at n={n}:\n{log}"
        );
    }
    assert!(
        fib["scaling"]["40"]["logos_tw"].is_null(),
        "and not at the size it ran out of time at"
    );
    assert!(
        fib["timed_out"]["40"]["logos_tw"].is_number(),
        "where it ran out of time is recorded"
    );
}

#[test]
fn a_jit_row_that_never_reached_the_jit_stops_the_run() {
    let fx = Fixture::new("0.05");
    let out = fx.run(&[("FIXTURE_NO_JIT", "1")]);
    let log = text(&out);
    assert!(
        !out.status.success(),
        "a LOGOS (JIT) row that ran without the JIT must stop the run:\n{log}"
    );
    assert!(
        log.contains("did not run on the JIT"),
        "the failure must say so:\n{log}"
    );
}

/// A phase its knob enables runs, or stops the run by name. `run.sh` runs every script of its own through an
/// interpreter (`.`, `bash`, `python3`), so no file mode can turn a phase into a skip: a script that is not
/// executable still runs, and one that is missing fails its phase loudly.
#[test]
fn no_phase_waits_on_a_scripts_file_mode() {
    let run = std::fs::read_to_string(repo_root().join("benchmarks/run.sh")).expect("read benchmarks/run.sh");
    let code: Vec<&str> = run.lines().filter(|l| !l.trim_start().starts_with('#')).collect();
    let gated: Vec<&str> = code.iter().copied().filter(|l| l.contains("-x \"$SCRIPT_DIR/")).collect();
    assert!(gated.is_empty(), "run.sh gates a script of its own on its executable bit:\n{}", gated.join("\n"));
    let mut runs = Vec::new();
    for line in &code {
        let mut rest = *line;
        while let Some(at) = rest.find("\"$SCRIPT_DIR/") {
            let tail = &rest[at + 1..];
            let end = tail.find('"').expect("a quoted script path closes");
            let script = &tail[..end];
            if script.ends_with(".sh") || script.ends_with(".py") {
                let before = rest[..at].trim_end();
                assert!(
                    before.ends_with("bash") || before.ends_with("python3") || before.ends_with('.'),
                    "run.sh reaches {script} other than through an interpreter: {line}"
                );
                runs.push(script.trim_start_matches("$SCRIPT_DIR/").to_string());
            }
            rest = &tail[end..];
        }
    }
    for script in ["calibrate-interp.sh", "run-interp-vs-js.sh", "measure-sizes.sh", "gen-readme-charts.py"] {
        assert!(runs.iter().any(|r| r == script), "run.sh runs {script}: {runs:?}");
    }
}

/// `quiet_box_wait` answers what the gate answers. A machine that goes quiet after any number of busy checks is quiet —
/// including after the retry delay has reached its 120 s cap, where a condition left as the loop's last statement once
/// made the wait report a passing gate as "never went quiet" — and a machine still busy when `BENCH_QUIET_WAIT` runs out
/// is not.
#[test]
fn a_machine_that_goes_quiet_after_any_number_of_busy_checks_is_quiet() {
    let wait = |busy_checks: u64, budget: &str| -> bool {
        let script = format!(
            ". lib/suite.sh > /dev/null 2>&1 || exit 2\n\
             checks=0\n\
             quiet_box_gate() {{ checks=$((checks + 1)); [ \"$checks\" -gt {busy_checks} ]; }}\n\
             sleep() {{ :; }}\n\
             quiet_box_wait 2> /dev/null"
        );
        let out = Command::new("bash")
            .arg("-c")
            .arg(&script)
            .current_dir(repo_root().join("benchmarks"))
            .env("BENCH_QUIET_WAIT", budget)
            .output()
            .expect("run bash");
        match out.status.code() {
            Some(0) => true,
            Some(1) => false,
            other => panic!("quiet_box_wait exited {other:?}: {}", text(&out)),
        }
    };
    for busy in [0, 1, 2, 3, 4, 5, 8, 20] {
        assert!(wait(busy, "100000"), "a machine quiet after {busy} busy checks was reported busy");
    }
    assert!(!wait(1_000_000, "300"), "a machine busy past BENCH_QUIET_WAIT was reported quiet");
}
