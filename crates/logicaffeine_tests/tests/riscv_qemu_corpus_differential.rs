mod common;
#[path = "support/riscv_runner.rs"]
mod riscv_runner;

use logicaffeine_compile::compile::compile_to_riscv;
use std::collections::BTreeMap;

/// The benchmark programs' root, relative to this package.
const ROOT: &str = "../../benchmarks/programs";

/// The argument sizes the differential may run a program at; each program runs at the ones no larger than
/// its own smallest declared workload (`riscv_runner::differential_sizes`).
const SIZES: &[u64] = &[1, 2, 5, 8, 13];

fn programs() -> Vec<(String, String)> {
    let mut out = Vec::new();
    let root = ROOT;
    if let Ok(dirs) = std::fs::read_dir(root) {
        let mut names: Vec<String> = dirs
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_dir())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        names.sort();
        for n in names {
            for variant in ["main", "interp"] {
                let p = format!("{root}/{n}/{variant}.lg");
                if let Ok(s) = std::fs::read_to_string(&p) {
                    out.push((format!("{n}/{variant}"), s));
                }
            }
        }
    }
    out
}

#[test]
fn qa_compile_coverage() {
    let progs = programs();
    let total = progs.len();
    let mut ok = 0usize;
    let mut fails: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (name, src) in &progs {
        match compile_to_riscv(src) {
            Ok(_) => ok += 1,
            Err(e) => {
                let msg = match &e.kind {
                    logicaffeine_compile::error::ParseErrorKind::Custom(m) => m.clone(),
                    other => format!("{other:?}"),
                };
                fails.entry(msg).or_default().push(name.clone());
            }
        }
    }
    println!("\n=== RISC-V COMPILE COVERAGE: {ok}/{total} programs compile ===");
    for (reason, names) in &fails {
        println!("  [{}] {}", names.len(), reason);
        for n in names {
            println!("       - {n}");
        }
    }
}

/// The toolchain, through the honest seam: an absent tool is a recorded skip (`LOGOS_TOOL_SKIP`), and
/// a failure when `LOGOS_REQUIRE_TOOLS` names it, as scripts/run-qemu-corpus-tests.sh does.
fn have_toolchain() -> bool {
    common::require_tool("riscv64-linux-gnu-gcc") && common::require_tool("qemu-riscv64")
}

/// The real proof: every benchmark that COMPILES must RUN byte-for-byte == the VM under qemu, given the
/// same program args, at each of its differential sizes (small, so both oracles stay fast).
/// ★ `#[ignore]`: minutes of single-core emulation over the whole corpus (344 runs, measured 150 s at the
/// test profile; the `qemu-corpus` group in `.config/nextest.toml`), so neither the routine baseline nor a
/// per-crate `cargo nextest run` runs it: `scripts/run-qemu-corpus-tests.sh` does, with
/// `--run-ignored all`.
#[test]
#[ignore = "minutes of qemu emulation over the whole benchmark corpus (344 runs). EXCLUDED from the routine baseline by run-all-tests-fast.sh; run on purpose via scripts/run-qemu-corpus-tests.sh, which passes --run-ignored all."]
fn qa_execution_differential() {
    if !have_toolchain() {
        return;
    }
    let progs = programs();
    let mut pass = 0usize;
    let mut mism: Vec<String> = Vec::new();
    let mut tested = 0usize;
    for (name, src) in &progs {
        let asm = match compile_to_riscv(src) {
            Ok(a) => a,
            Err(_) => continue, // covered by qa_compile_coverage
        };
        tested += 1;
        let dir = std::path::Path::new(ROOT).join(name.split('/').next().unwrap());
        let linked = riscv_runner::link(&asm, name);
        let mut all_ok = true;
        for size in riscv_runner::differential_sizes(&dir, SIZES) {
            let vm_args = vec!["prog".to_string(), size.clone()];
            let vm = logicaffeine_compile::compile::vm_outcome_with_args(src, &vm_args, None);
            let rv = riscv_runner::run(&linked, &[&size], &[]);
            let vm_out = &vm.output;
            let rv_out = &rv.stdout;
            let vm_ok = vm.error.is_none();
            if !same_outcome(&vm.output, vm.error.as_deref(), &rv) {
                mism.push(format!(
                    "{name}[{size}]: MISMATCH\n    vm(ok={vm_ok}): {vm_out:?}\n    rv(ok={}): {rv_out:?}\n    rv {}",
                    rv.success,
                    rv.describe()
                ));
                all_ok = false;
            }
        }
        if all_ok {
            pass += 1;
        }
    }
    println!("\n=== RISC-V EXECUTION ≡ VM (sizes {SIZES:?}, each program below its smallest workload): {pass}/{tested} programs match under qemu ===");
    for m in &mism {
        println!("  {m}");
    }
    assert!(
        mism.is_empty(),
        "{} programs diverged from the VM",
        mism.len()
    );
}

// Shared by the real differential and its adversarial fixtures.
fn same_outcome(stdout: &str, failure: Option<&str>, run: &riscv_runner::RvRun) -> bool {
    let (stderr, code) = match failure {
        Some(message) => (format!("{message}\n"), 1),
        None => (String::new(), 0),
    };
    stdout == run.stdout
        && stderr == run.stderr
        && run.code == Some(code)
        && run.success == (code == 0)
}

fn observed(stdout: &str, stderr: &str, code: Option<i32>) -> riscv_runner::RvRun {
    riscv_runner::RvRun {
        stdout: stdout.into(),
        stderr: stderr.into(),
        code,
        success: code == Some(0),
    }
}

#[test]
fn corpus_comparison_keeps_every_stdout_byte() {
    for wrong in ["answer", " answer\n", "answer \n", "answer\n\n"] {
        assert!(
            !same_outcome("answer\n", None, &observed(wrong, "", Some(0))),
            "accepted changed output: {wrong:?}"
        );
    }
    assert!(
        !same_outcome("", None, &observed("\n", "", Some(0))),
        "an empty line is output"
    );
}

#[test]
fn corpus_comparison_rejects_diagnostics_on_success() {
    assert!(!same_outcome(
        "answer\n",
        None,
        &observed("answer\n", "unexpected\n", Some(0))
    ));
}

#[test]
fn corpus_comparison_requires_the_same_failure_and_exit() {
    let failure = Some("Cannot index Nothing with Int");
    for (stderr, code) in [
        ("wrong reason\n", Some(1)),
        ("Cannot index Nothing with Int", Some(1)),
        ("Cannot index Nothing with Int\n", Some(124)),
        ("Cannot index Nothing with Int\n", None),
    ] {
        assert!(
            !same_outcome("before\n", failure, &observed("before\n", stderr, code)),
            "accepted incorrect failure: {stderr:?}, {code:?}"
        );
    }
}

#[test]
fn corpus_comparison_accepts_complete_matching_outcomes() {
    assert!(same_outcome("", None, &observed("", "", Some(0))));
    assert!(same_outcome(
        " answer\n\n",
        None,
        &observed(" answer\n\n", "", Some(0))
    ));
    assert!(same_outcome(
        "before\n",
        Some("bad"),
        &observed("before\n", "bad\n", Some(1))
    ));
}
