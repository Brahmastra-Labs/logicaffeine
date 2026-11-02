//! ★ LOCK — every benchmark program prints its expected bytes on every LOGOS backend. ★
//!
//! The page shows LOGOS every way it runs: built through Rust (`largo build --release`), built through C from the
//! program (`largo build --release --emit c`) and from its bytecode (`--emit c-bytecode`), on the JIT (`largo run
//! --interpret`), on the bytecode VM alone (`--engine vm`) and on the tree-walking interpreter (`--engine tree`). Two
//! more backends compile the same programs: WebAssembly (`largo run --emit wasm`, under its Node host) and RISC-V
//! (`compile_to_riscv`, linked with its runtime and run under `qemu-riscv64`). Each program in
//! `benchmarks/lib/benchmarks.tsv` runs
//! here at its `small_size` the way `benchmarks/lib/suite.sh` runs it (a fresh project named `bench`, the product
//! defaults, no `LOGOS_*` tuning left in the environment), and must print `expected_<small_size>.txt` byte for byte. The
//! expected bytes come from the C, C++ and Rust reference ports agreeing (`benchmarks/gen-expected.sh`), never from
//! LOGOS. A timed run verifies its own sizes too; this lock is what makes a tier regression red in the suite, before
//! anyone measures anything.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test benchmark_logos_tiers_lock --run-ignored all`

#![cfg(not(target_arch = "wasm32"))]

#[path = "cli_common/mod.rs"]
mod common;
use common::{largo, require_tool};

#[path = "support/riscv_runner.rs"]
#[allow(dead_code)]
mod riscv_runner;

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::Mutex;
use tempfile::TempDir;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
}

/// One benchmark: its id and the size this lock runs it at.
struct Bench {
    id: String,
    size: String,
}

/// Every benchmark in the registry, in its order, with its `small_size`.
fn benchmarks() -> Vec<Bench> {
    let text = std::fs::read_to_string(repo_root().join("benchmarks/lib/benchmarks.tsv"))
        .expect("benchmarks.tsv");
    let mut lines = text.lines().filter(|l| !l.trim().is_empty());
    let header: Vec<&str> = lines.next().expect("a header row").split('\t').collect();
    let col = |name: &str| {
        header
            .iter()
            .position(|h| *h == name)
            .unwrap_or_else(|| panic!("no `{name}` column"))
    };
    let (id, size) = (col("id"), col("small_size"));
    let rows: Vec<Bench> = lines
        .map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            Bench {
                id: f[id].to_string(),
                size: f[size].to_string(),
            }
        })
        .collect();
    assert!(
        rows.len() >= 35,
        "the registry lists {} benchmarks; the suite has at least 35",
        rows.len()
    );
    rows
}

fn program_dir(bench: &Bench) -> PathBuf {
    repo_root().join("benchmarks/programs").join(&bench.id)
}

fn expected(bench: &Bench) -> Vec<u8> {
    let path = program_dir(bench).join(format!("expected_{}.txt", bench.size));
    std::fs::read(&path).unwrap_or_else(|_| {
        panic!(
            "{} has no expected output at its small_size {}: write it with `bash benchmarks/gen-expected.sh {} {}`",
            bench.id, bench.size, bench.id, bench.size
        )
    })
}

/// A fresh project the way `suite.sh`'s `_logos_scaffold` lays one out.
fn project(bench: &Bench) -> TempDir {
    let dir = TempDir::new().expect("temp dir");
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::copy(
        program_dir(bench).join("main.lg"),
        dir.path().join("src/main.lg"),
    )
    .expect("copy main.lg");
    std::fs::write(
        dir.path().join("Largo.toml"),
        "[package]\nname = \"bench\"\nversion = \"0.1.0\"\nentry = \"src/main.lg\"\n",
    )
    .unwrap();
    dir
}

/// `largo` with every `LOGOS_*` tuning variable removed, as `suite_clean_logos_env` leaves a timed run.
fn product_largo(dir: &Path) -> Command {
    let mut cmd = largo();
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("LOGOS_") && key != "LOGOS_WORKSPACE" {
            cmd.env_remove(&key);
        }
    }
    cmd.current_dir(dir);
    cmd
}

fn show(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn failed(what: &str, out: &Output) -> String {
    format!(
        "{what} exited {:?}\nstdout: {}\nstderr: {}",
        out.status,
        show(&out.stdout),
        show(&out.stderr)
    )
}

/// Run `check` over every benchmark on a few threads and gather every failure, so one red names them all.
fn every_benchmark(threads: usize, check: impl Fn(&Bench) -> Result<(), String> + Sync) {
    let benches = benchmarks();
    let next = Mutex::new(0usize);
    let failures = Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| loop {
                let i = {
                    let mut n = next.lock().unwrap();
                    let i = *n;
                    *n += 1;
                    i
                };
                let Some(bench) = benches.get(i) else { break };
                if let Err(why) = check(bench) {
                    failures
                        .lock()
                        .unwrap()
                        .push(format!("{} at n={}: {why}", bench.id, bench.size));
                }
            });
        }
    });
    let failures = failures.into_inner().unwrap();
    assert!(
        failures.is_empty(),
        "{} of {} benchmarks fail:\n\n{}",
        failures.len(),
        benches.len(),
        failures.join("\n\n")
    );
}

/// The bytes a built artifact prints, against the expected bytes.
fn artifact_prints_expected(bench: &Bench, dir: &Path, tier: &str) -> Result<(), String> {
    let bin = dir.join("target/release/bench");
    if !bin.is_file() {
        return Err(format!(
            "{tier} published no artifact at target/release/bench"
        ));
    }
    let out = Command::new(&bin)
        .arg(&bench.size)
        .output()
        .map_err(|e| format!("run {}: {e}", bin.display()))?;
    if !out.status.success() {
        return Err(failed(tier, &out));
    }
    let want = expected(bench);
    if out.stdout != want {
        return Err(format!(
            "{tier} printed\n{}\nexpected\n{}",
            show(&out.stdout),
            show(&want)
        ));
    }
    Ok(())
}

/// `largo run --interpret` with `engine_args`: it prints the expected bytes, and the engine trace shows the run took the
/// engine `ran_on` accepts, not a fallback.
fn on_engine(
    bench: &Bench,
    engine_args: &[&str],
    tier: &str,
    ran_on: impl Fn(&[&str]) -> bool,
) -> Result<(), String> {
    let dir = project(bench);
    let mut args = vec!["-q", "run", "--interpret"];
    args.extend_from_slice(engine_args);
    args.extend(["--", &bench.size]);
    let out = product_largo(dir.path())
        .env("LOGOS_ENGINE_TRACE", "1")
        .args(&args)
        .output()
        .map_err(|e| format!("spawn largo: {e}"))?;
    if !out.status.success() {
        return Err(failed(&format!("largo {}", args.join(" ")), &out));
    }
    let want = expected(bench);
    if out.stdout != want {
        return Err(format!(
            "{tier} printed\n{}\nexpected\n{}",
            show(&out.stdout),
            show(&want)
        ));
    }
    let trace = show(&out.stderr);
    let engine: Vec<&str> = trace
        .lines()
        .filter(|l| l.starts_with("logos-engine:"))
        .collect();
    if !ran_on(&engine) {
        return Err(format!(
            "{tier} did not run on its engine; the engine said {engine:?}"
        ));
    }
    Ok(())
}

#[test]
fn every_benchmark_prints_its_expected_bytes_on_the_jit() {
    every_benchmark(8, |bench| {
        on_engine(bench, &[], "LOGOS (JIT)", |engine| {
            engine.contains(&"logos-engine: vm+jit")
                && engine
                    .iter()
                    .any(|l| l.starts_with("logos-engine: native-tier "))
        })
    });
}

#[test]
fn every_benchmark_prints_its_expected_bytes_on_the_vm_alone() {
    every_benchmark(8, |bench| {
        on_engine(bench, &["--engine", "vm"], "LOGOS (VM)", |engine| {
            engine == ["logos-engine: vm"]
        })
    });
}

#[test]
fn every_benchmark_prints_its_expected_bytes_on_the_tree_walker() {
    every_benchmark(8, |bench| {
        on_engine(
            bench,
            &["--engine", "tree"],
            "LOGOS (tree-walker)",
            |engine| engine == ["logos-engine: treewalker"],
        )
    });
}

#[test]
fn every_benchmark_built_through_c_prints_its_expected_bytes() {
    if !require_tool("cc") {
        return;
    }
    every_benchmark(8, |bench| {
        let dir = project(bench);
        let out = product_largo(dir.path())
            .env("CARGO_PROFILE_RELEASE_STRIP", "false")
            .args(["-q", "build", "--release", "--emit", "c"])
            .output()
            .map_err(|e| format!("spawn largo: {e}"))?;
        if !out.status.success() {
            return Err(failed("largo build --release --emit c", &out));
        }
        artifact_prints_expected(bench, dir.path(), "LOGOS (C)")
    });
}

#[test]
fn every_benchmark_built_through_c_from_its_bytecode_prints_its_expected_bytes() {
    if !require_tool("cc") {
        return;
    }
    every_benchmark(8, |bench| {
        let dir = project(bench);
        let out = product_largo(dir.path())
            .env("CARGO_PROFILE_RELEASE_STRIP", "false")
            .args(["-q", "build", "--release", "--emit", "c-bytecode"])
            .output()
            .map_err(|e| format!("spawn largo: {e}"))?;
        if !out.status.success() {
            return Err(failed("largo build --release --emit c-bytecode", &out));
        }
        artifact_prints_expected(bench, dir.path(), "LOGOS (C, bytecode)")
    });
}

/// The heaviest of the builds: 35 release builds through cargo. Cargo holds a target directory's build lock for a whole
/// build, so builds that share one directory run one at a time however many workers start them: each worker builds
/// into a target directory of its own, where the dependency graph compiles once (largo gives each project its own
/// artifact name inside it).
#[test]
#[ignore = "heavy: 35 release builds through cargo; the full suite runs it"]
fn every_benchmark_built_through_rust_prints_its_expected_bytes() {
    const WORKERS: usize = 4;
    let targets = Mutex::new(
        (0..WORKERS)
            .map(|_| TempDir::new().expect("a worker's target dir"))
            .collect::<Vec<_>>(),
    );
    every_benchmark(WORKERS, |bench| {
        let target = targets.lock().unwrap().pop().expect("a target dir for every worker");
        let dir = project(bench);
        let out = product_largo(dir.path())
            .env("CARGO_TARGET_DIR", target.path())
            .env("CARGO_PROFILE_RELEASE_STRIP", "false")
            .args(["-q", "build", "--release"])
            .output();
        targets.lock().unwrap().push(target);
        let out = out.map_err(|e| format!("spawn largo: {e}"))?;
        if !out.status.success() {
            return Err(failed("largo build --release", &out));
        }
        artifact_prints_expected(bench, dir.path(), "LOGOS (Rust)")
    });
}

#[test]
fn every_benchmark_compiled_to_webassembly_prints_its_expected_bytes() {
    if !require_tool("node") {
        return;
    }
    every_benchmark(8, |bench| {
        let dir = project(bench);
        let out = product_largo(dir.path())
            .args(["-q", "run", "--emit", "wasm", "--", &bench.size])
            .output()
            .map_err(|e| format!("spawn largo: {e}"))?;
        if !out.status.success() {
            return Err(failed("largo run --emit wasm", &out));
        }
        let want = expected(bench);
        if out.stdout != want {
            return Err(format!(
                "LOGOS (WebAssembly) printed\n{}\nexpected\n{}",
                show(&out.stdout),
                show(&want)
            ));
        }
        Ok(())
    });
}

#[test]
fn every_benchmark_compiled_to_risc_v_prints_its_expected_bytes() {
    if !require_tool("riscv64-linux-gnu-gcc") || !require_tool("qemu-riscv64") {
        return;
    }
    every_benchmark(8, |bench| {
        let src = std::fs::read_to_string(program_dir(bench).join("main.lg"))
            .map_err(|e| format!("read main.lg: {e}"))?;
        let asm = logicaffeine_compile::compile::compile_to_riscv(&src)
            .map_err(|e| format!("compile_to_riscv refused: {e:?}"))?;
        let linked =
            std::panic::catch_unwind(|| riscv_runner::link(&asm, &bench.id)).map_err(|_| {
                "riscv64-linux-gnu-gcc did not link the program (its report is above)".to_string()
            })?;
        let run = riscv_runner::run(&linked, &[&bench.size], &[]);
        if !run.success {
            return Err(format!(
                "LOGOS (RISC-V) failed under qemu: {}",
                run.describe()
            ));
        }
        let want = expected(bench);
        if run.stdout.as_bytes() != want.as_slice() {
            return Err(format!(
                "LOGOS (RISC-V) printed\n{}\nexpected\n{}",
                run.stdout,
                show(&want)
            ));
        }
        Ok(())
    });
}
