//! ★ THE BYTECODE-C PERFORMANCE LOCK ★ — Invariant C of the bytecode-IR-as-source-of-truth refactor.
//!
//! The re-base's north star is not just "as correct" but "as fast, then FASTER": the bytecode→C Op-walker
//! (`compile_to_c_via_bytecode`) must not regress the runtime of the shipping AST-based emitter
//! (`compile_to_c`). This lock compiles the SAME compute program through BOTH emitters, gcc `-O2`, times the
//! runtime, and asserts the bytecode path is within a RATCHETING ratio of the AST path.
//!
//! **Why a ratio, not an absolute floor:** the two binaries run INTERLEAVED — one run of each per round, the order
//! alternating round to round — so both see the same machine state (clock frequency, caches, load), and the RATIO
//! of their fastest rounds cancels it. Timed in separate blocks, the second binary inherits a clock the first one
//! ramped up, and on a few-millisecond workload that alone reads as a 1.5-3x difference. The MIN across rounds is
//! the least-interfered run, the truest measure of the code's own cost. Every workload runs ~100 ms: an idle core
//! changes clock speed and sleep state over the first tens of milliseconds a process runs, so a shorter program is
//! timed at whatever speed it happened to start at, and two such timings differ by far more than their code does.
//!
//! **Robustness (perf tests are fragile — this one refuses to flake or hang):**
//! - LOAD GUARD — a wall-clock timing on an oversubscribed box is meaningless, so if 1-minute load per core is
//!   high the lock SKIPs clean (benchmarks belong on a quiet box, per `docs/project/VERSIONING.md`).
//! - TIMEOUT — every measured binary runs under a hard wall-clock cap; a pathological run is killed and fails
//!   loudly instead of hanging.
//! - i64-SAFE PROGRAMS — every program is `mod`-bounded so values never leave `i64`; LOGOS integers are exact
//!   (arbitrary precision), and an unbounded-growth loop would make the AST path promote to BIGNUM (semantically
//!   different from the Op-walker's raw i64, and pathologically slow) — a trap, not a perf signal.
//!
//! The ratchet: `MAX_SLOWDOWN` is the ceiling on `bytecode_time / ast_time`. TIGHTEN it as the Op-walker gains
//! the relooper + optimized-IR lowering; NEVER loosen it to make a run pass (rule 7). gcc-gated.
#![cfg(not(target_arch = "wasm32"))]

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use logicaffeine_compile::compile::{compile_to_c, compile_to_c_via_bytecode};

/// Compute programs the Op-walker fully covers, each `mod`-bounded so values stay in `i64` (no bignum), and
/// chosen so the optimizer can NOT collapse them to a closed form — so the comparison measures codegen quality
/// (register allocation, call convention, loop shape), not who ran the optimizer.
const PERF_CORPUS: &[(&str, &str)] = &[
    // A linear-congruential hash, bounded to 31 bits — a data-dependent recurrence, 40M i64 ops.
    ("lcg_hash", "## Main\n    Let mutable h be 7.\n    Let mutable i be 1.\n    While i is at most 40000000:\n        Set h to (h * 1103515 + 12345) mod 2147483647.\n        Set i to i + 1.\n    Show h.\n"),
    // Iterative Fibonacci mod 1e9+7 — a two-variable recurrence the optimizer keeps as a loop, bounded.
    ("fib_iter_mod", "## Main\n    Let mutable a be 0.\n    Let mutable b be 1.\n    Let mutable i be 1.\n    While i is at most 50000000:\n        Let t be (a + b) mod 1000000007.\n        Set a to b.\n        Set b to t.\n        Set i to i + 1.\n    Show b.\n"),
    // Takeuchi's function tak(28, 18, 8) — ~10^8 calls of three arguments that all vary, the register-windowing
    // call convention under load; no argument repeats often enough for a memo table to stand in for the calls.
    ("tak_rec", "## To tak (x: Int) (y: Int) (z: Int) -> Int:\n    If y is less than x:\n        Return tak(tak(x - 1, y, z), tak(y - 1, z, x), tak(z - 1, x, y)).\n    Return z.\n\n## Main\n    Show tak(28, 18, 8).\n"),
];

/// The ratchet ceiling on `bytecode_time / ast_time`. MEASURED at PARITY on a quiet box (2026-08-31): worst
/// 1.01x, best 0.96x (bytecode-C actually FASTER on the LCG hash) — even though the Op-walker lowers RAW
/// bytecode with `goto` control flow, gcc -O2 recovers the loop structure and re-derives the magic division,
/// so it matches the AST emitter that ran the full optimizer lattice. The 1.30x ceiling locks that parity with
/// headroom for the ≤1.5-load-per-core measurement noise the guard admits. TIGHTEN toward 1.0 as the relooper +
/// optimized-IR lowering land; NEVER loosen it to make a run pass.
const MAX_SLOWDOWN: f64 = 1.30;

/// Hard per-run wall-clock cap — a measured binary that exceeds this is killed and the lock fails loudly.
const RUN_TIMEOUT: Duration = Duration::from_secs(20);

fn have_gcc() -> bool {
    Command::new("gcc")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// True when the box is too busy for a meaningful wall-clock timing (1-min load per core above a threshold).
fn machine_busy() -> Option<f64> {
    let load1 = std::fs::read_to_string("/proc/loadavg")
        .ok()?
        .split_whitespace()
        .next()?
        .parse::<f64>()
        .ok()?;
    let ncpu = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1) as f64;
    let per_core = load1 / ncpu;
    if per_core > 1.5 {
        Some(per_core)
    } else {
        None
    }
}

/// Compile `c` source with gcc `-O2` into a fresh temp binary.
fn gcc_build(c: &str, tag: &str) -> Result<std::path::PathBuf, String> {
    let dir = std::env::temp_dir().join(format!("logos_perf_{}_{}", std::process::id(), tag));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let cfile = dir.join("prog.c");
    let bin = dir.join("prog");
    std::fs::write(&cfile, c).map_err(|e| e.to_string())?;
    let out = Command::new("gcc")
        .args(["-O2", "-o"])
        .arg(&bin)
        .arg(&cfile)
        .arg("-lm")
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(format!(
            "gcc failed:\n{}",
            String::from_utf8_lossy(&out.stderr)
                .lines()
                .take(4)
                .collect::<Vec<_>>()
                .join("\n")
        ));
    }
    Ok(bin)
}

/// Run `bin` once under `RUN_TIMEOUT`, returning (wall-time, trimmed stdout). The wait blocks until the program
/// exits, so the timing ends at its exit and not at the next poll; a watchdog kills a run that outlives the cap,
/// which then fails loudly.
fn run_once(bin: &std::path::Path) -> Result<(Duration, String), String> {
    let child = Command::new(bin)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())?;
    let pid = child.id();
    let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let killed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let watchdog = {
        let (done, killed) = (done.clone(), killed.clone());
        std::thread::spawn(move || {
            let deadline = Instant::now() + RUN_TIMEOUT;
            while Instant::now() < deadline {
                if done.load(std::sync::atomic::Ordering::SeqCst) {
                    return;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            if !done.load(std::sync::atomic::Ordering::SeqCst) {
                killed.store(true, std::sync::atomic::Ordering::SeqCst);
                // SAFETY: kill(2) on the child this run spawned; ESRCH after it exits is ignored.
                unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
            }
        })
    };
    let t0 = Instant::now();
    let out = child.wait_with_output().map_err(|e| e.to_string());
    let dt = t0.elapsed();
    done.store(true, std::sync::atomic::Ordering::SeqCst);
    let _ = watchdog.join();
    let out = out?;
    if killed.load(std::sync::atomic::Ordering::SeqCst) {
        return Err(format!(
            "run exceeded {RUN_TIMEOUT:?} — killed (pathological runtime)"
        ));
    }
    if !out.status.success() {
        return Err(format!("program exited with {}", out.status));
    }
    Ok((dt, String::from_utf8_lossy(&out.stdout).trim().to_string()))
}

/// The fastest of `rounds` interleaved runs of `a` and `b` — one run of each per round, `a` first on even rounds and
/// `b` first on odd ones — with each program's (stable) trimmed stdout.
fn fastest_interleaved(
    a: &std::path::Path,
    b: &std::path::Path,
    rounds: usize,
) -> Result<((Duration, String), (Duration, String)), String> {
    let (mut best_a, mut best_b) = (Duration::MAX, Duration::MAX);
    let (mut out_a, mut out_b) = (String::new(), String::new());
    for round in 0..rounds {
        let order: [bool; 2] = if round % 2 == 0 {
            [true, false]
        } else {
            [false, true]
        };
        for first_is_a in order {
            let (dt, o) = run_once(if first_is_a { a } else { b })?;
            if first_is_a {
                best_a = best_a.min(dt);
                out_a = o;
            } else {
                best_b = best_b.min(dt);
                out_b = o;
            }
        }
    }
    Ok(((best_a, out_a), (best_b, out_b)))
}

/// Each probe measures the code it names: no optimizer pass the bytecode path runs collapses its work away. A
/// memoized recursion runs once per distinct argument, so a memoized probe times the memo table, not the calls.
#[test]
fn every_perf_probe_keeps_the_work_it_measures() {
    for (label, src) in PERF_CORPUS {
        logicaffeine_compile::optimize::begin_fired_trace();
        let lowered = compile_to_c_via_bytecode(src);
        let fired = logicaffeine_compile::optimize::end_fired_trace()
            .map(|f| f.keywords())
            .unwrap_or_default();
        lowered.unwrap_or_else(|e| panic!("[{label}] compile_to_c_via_bytecode: {e:?}"));
        assert!(
            !fired.contains(&"memo"),
            "[{label}] the memoizer rewrites the probe on the bytecode path (fired: {fired:?})"
        );
    }
}

#[test]
fn the_bytecode_c_op_walker_is_not_slower_than_the_ast_c_emitter() {
    if !have_gcc() {
        eprintln!("SKIP: gcc absent — cannot run the bytecode-C performance lock");
        return;
    }
    if let Some(per_core) = machine_busy() {
        eprintln!("SKIP: machine busy (load/core = {per_core:.1} > 1.5) — wall-clock timing would be meaningless; run on a quiet box");
        return;
    }

    let rounds = 8;
    let mut worst_ratio = 0.0_f64;
    for (label, src) in PERF_CORPUS {
        let ast_c =
            compile_to_c(src).unwrap_or_else(|e| panic!("[{label}] compile_to_c (AST): {e:?}"));
        let bc_c = compile_to_c_via_bytecode(src)
            .unwrap_or_else(|e| panic!("[{label}] compile_to_c_via_bytecode: {e:?}"));
        let ast_bin = gcc_build(&ast_c, &format!("{label}_ast"))
            .unwrap_or_else(|e| panic!("[{label}] AST {e}"));
        let bc_bin =
            gcc_build(&bc_c, &format!("{label}_bc")).unwrap_or_else(|e| panic!("[{label}] BC {e}"));

        let ((ast_t, ast_out), (bc_t, bc_out)) = fastest_interleaved(&ast_bin, &bc_bin, rounds)
            .unwrap_or_else(|e| panic!("[{label}] run: {e}"));

        // Correctness gate: the two emitters must produce identical output (perf is meaningless otherwise).
        assert_eq!(
            bc_out, ast_out,
            "[{label}] bytecode-C and AST-C disagree on output — not a valid perf comparison"
        );

        let ratio = bc_t.as_secs_f64() / ast_t.as_secs_f64().max(1e-9);
        worst_ratio = worst_ratio.max(ratio);
        eprintln!(
            "[c_bytecode PERF] {label:12} bytecode {:>8.2}ms  ast {:>8.2}ms  ratio {ratio:.2}x",
            bc_t.as_secs_f64() * 1e3,
            ast_t.as_secs_f64() * 1e3
        );

        assert!(
            ratio <= MAX_SLOWDOWN,
            "[{label}] bytecode-C is {ratio:.2}x slower than AST-C (ceiling {MAX_SLOWDOWN:.2}x) — the re-base \
             regressed runtime. Fix the emitter (relooper / optimized-IR lowering); do NOT loosen the ceiling."
        );
    }
    eprintln!("[c_bytecode PERF] worst ratio {worst_ratio:.2}x (ceiling {MAX_SLOWDOWN:.2}x)");
    if worst_ratio > 0.0 && worst_ratio < MAX_SLOWDOWN * 0.8 {
        eprintln!("[c_bytecode PERF] ↓ headroom — consider TIGHTENING MAX_SLOWDOWN toward {worst_ratio:.2}x");
    }
}
