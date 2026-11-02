//! ★ THE BYTECODE→C WALKER GOES STACKLESS — PIT OF SUCCESS ★ (RED-first, TDD).
//!
//! The bytecode `Op`-walker (`compile_to_c_via_bytecode`) lowers the whole SEQUENTIAL surface, but a task
//! must SUSPEND at a blocking op and RESUME later — which a flat Op stream cannot express. So a concurrency
//! program routes through the portable heap-frame **stackless** CPS transform (the one the CUDA tier uses),
//! wrapped for the host and driven by the deterministic offline scheduler tuned by [`SchedulerKnobs`]
//! (default = the oracle's `Fifo` / `workers=1` / `Logical`). This lock pins that the walker path runs the
//! concurrency corpus byte-for-byte identical to the VM/tree-walker oracle.
//!
//! The OTHER async subsystems — offline networking (`Net*`), the CRDT lattice (`Crdt*`/`NewCrdt`), and runtime
//! authorization (`CheckPolicy`) — are lowered NATIVELY by the bytecode Op-walker (`from_program`, mirroring
//! `codegen_riscv`): it reuses `C_NET_RUNTIME` (loopback transport) + `C_CRDT_RUNTIME` (per-replica counter map
//! + register/sequence layout) and compiles each `Check`'s policy condition against the subject's field cells.
//! The `OFFLINE_CORPUS` + op-partition below pin it: 13 offline ops (incl. `CrdtJoin` via `Converge`) build+run
//! ==VM through the NATIVE walker (`native_offline_ops_do_not_delegate` proves the walker's C differs from — and
//! on `sync_only` is MORE correct than — the AST-C tier), `NetConnect` lowers to a native offline no-op, and the
//! LAST holdout — the real-socket `Tcp` client — now ALSO lowers NATIVELY (over the `C_TCP_RUNTIME` POSIX socket
//! runtime), witnessed end-to-end ≡VM in `tcp_walker_lock`. ★ ENDGAME: the delegation gate is EMPTY and NOTHING
//! is un-emittable — the WHOLE 29-op async set is source-emittable AND covered on the native walker, NO refusals.
//!
//! Discipline (mirrors `c_concurrency_scheduler_lock`, but for the WALKER entry `compile_to_c_via_bytecode`):
//! - Every corpus program is DETERMINATE (the two oracle drivers agree — asserted in
//!   `c_concurrency_scheduler_lock::the_concurrent_oracles_agree_on_every_program`), so its output is unique.
//! - A `Supported` program MUST compile (as C++ — the stackless body uses reference-alias locals) and run
//!   byte-for-byte identical to that oracle. The whole determinate corpus (int/bool/text/real scalars +
//!   Int/Text/Real channels + spawn/sleep/select/blocking-loop) is Supported — `KNOWN_GAPS` is EMPTY.
//! - A `Gap` (a shape the stackless transform does not yet lower) is a documented, shrink-only frontier:
//!   `KNOWN_GAPS` must equal the live `Gap` census exactly; `SUPPORTED_FLOOR` only rises.
//! - PERF: the stackless tier (tiny heap frame + cheap function-return yield) must be at least as fast as the
//!   AST-C UCONTEXT tier (256 KB stack/task + `swapcontext`/yield) — the "C must get FASTER" invariant.
//!
//! ⚠️ RED tests are the spec: if a `Supported` program fails ≡VM, fix the lowering, NEVER weaken this file.
//! The execution half is gated on `g++` and SKIPS when absent; the oracle-agreement + ratchet always run.

use std::collections::BTreeSet;

use logicaffeine_compile::codegen_c::concurrency::{
    SchedulePolicy, SchedulerConfig, SchedulerKnobs,
};
use logicaffeine_compile::compile::{
    compile_to_c, compile_to_c_via_bytecode, compile_to_c_via_bytecode_with_knobs,
};
use logicaffeine_compile::{interpret_for_ui_sync, run_vm_concurrent_with_config};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Status {
    /// Lowered through the stackless transform + proven byte-for-byte ≡ the deterministic concurrent oracle.
    Supported,
    /// Not yet lowered by the stackless transform (a non-int shown/sent value) — a shrink-only gap.
    Gap,
}

/// The determinate concurrency corpus + its per-program status on the WALKER path. Identical programs to
/// `c_concurrency_scheduler_lock::CORPUS`, so a program `Supported` there and here proves the walker's
/// stackless path agrees with the AST-C ucontext path — both ≡VM.
const CORPUS: &[(&str, &str, Status)] = &[
    (
        // A spawned task + `main` each `Show` a TEXT literal — now lowered (the stackless transform renders a
        // text-literal `Show` as `printf("%s\n", "…")`). `main` runs past the non-blocking spawn to its own
        // `Show "main"` first, then the worker shows "work" — byte-identical to the VM oracle.
        "fire_and_forget",
        "## To worker:\n    Show \"work\".\n\n## Main\n    Launch a task to worker.\n    Show \"main\".\n",
        Status::Supported,
    ),
    (
        "spawn_with_arg",
        "## To greet (n: Int):\n    Show n.\n\n## Main\n    Launch a task to greet with 7.\n    Show 0.\n",
        Status::Supported,
    ),
    (
        "producer_consumer",
        "## To produce (ch: Int):\n    Send 1 into ch.\n    Send 2 into ch.\n    Send 3 into ch.\n\n\
         ## Main\n    Let jobs be a Pipe of Int.\n    Launch a task to produce with jobs.\n    \
         Receive a from jobs.\n    Receive b from jobs.\n    Receive c from jobs.\n    Show a + b + c.\n",
        Status::Supported,
    ),
    (
        "sleep_virtual_time",
        "## Main\n    Show 1.\n    Sleep 5.\n    Show 2.\n",
        Status::Supported,
    ),
    (
        "select_recv_timeout",
        "## To produce (ch: Int):\n    Send 9 into ch.\n\n## Main\n    Let ch be a Pipe of Int.\n    \
         Launch a task to produce with ch.\n    Await the first of:\n        Receive v from ch:\n            \
         Show v.\n        After 100 seconds:\n            Show 0.\n",
        Status::Supported,
    ),
    (
        "select_timeout_wins",
        "## Main\n    Let ch be a Pipe of Int.\n    Await the first of:\n        Receive v from ch:\n            \
         Show v.\n        After 5 seconds:\n            Show 42.\n",
        Status::Supported,
    ),
    (
        // A TEXT channel end-to-end: a spawned task sends a string, main receives it into a text var and
        // shows it — exercises the `Text` `ChanElem` encoding (a `char*` cast riding the 64-bit cell, valid
        // across the park because a string literal has static storage) that the Int-channel corpus never hits.
        "text_channel_roundtrip",
        "## To produce (ch: Text):\n    Send \"hi\" into ch.\n\n## Main\n    Let ch be a Pipe of Text.\n    \
         Launch a task to produce with ch.\n    Receive m from ch.\n    Show m.\n",
        Status::Supported,
    ),
    (
        // A REAL channel end-to-end: the sent `1.5` rides as its DOUBLE'S BITS (`logos_f64_to_bits`), and the
        // received var renders through the device Dragon4 formatter — proving the float bitcast path + the
        // shortest-round-trip formatter are pulled in and byte-identical to the VM (`1.5`, never `1`).
        "real_channel_roundtrip",
        "## To produce (ch: Real):\n    Send 1.5 into ch.\n\n## Main\n    Let ch be a Pipe of Real.\n    \
         Launch a task to produce with ch.\n    Receive x from ch.\n    Show x.\n",
        Status::Supported,
    ),
    (
        // A FLOAT-LITERAL Show on the stackless path (the program is concurrent via `Sleep`, so it routes to
        // the stackless transform). Each `1.5` rides as its double's bits and renders through Dragon4 — the
        // `uses_real` formatter-inclusion is widened to fire on a float Show even with no Real channel.
        "float_literal_show",
        "## Main\n    Show 1.5.\n    Sleep 1.\n    Show 2.5.\n",
        Status::Supported,
    ),
    (
        // TWO concurrent workers — `main` runs past both non-blocking spawns to `Show 3` first, then the two
        // children run in ready-queue order (FIFO: worker(1) then worker(2)) → `["3","1","2"]`. Hardens the
        // multi-task spawn path (the rest of the corpus spawns at most one child). Not dataflow-determinate:
        // the two children's `Show`s are policy-ordered (correct), so it is excluded from the cross-tier sweep.
        "two_workers",
        "## To worker (n: Int):\n    Show n.\n\n## Main\n    Launch a task to worker with 1.\n    \
         Launch a task to worker with 2.\n    Show 3.\n",
        Status::Supported,
    ),
    (
        // A BLOCKING loop: `main` receives three values in a `While` and accumulates them — the `Receive`'s
        // resume label lands INSIDE the emitted `while`, so a park/wake re-enters mid-iteration. Output is
        // dataflow-fixed (channel FIFO → 10+20+30) regardless of policy.
        "loop_receive",
        "## To produce (ch: Int):\n    Send 10 into ch.\n    Send 20 into ch.\n    Send 30 into ch.\n\n\
         ## Main\n    Let ch be a Pipe of Int.\n    Let total be 0.\n    Launch a task to produce with ch.\n    \
         Let i be 0.\n    While i < 3:\n        Receive v from ch.\n        Set total to total + v.\n        \
         Set i to i + 1.\n    Show total.\n",
        Status::Supported,
    ),
    (
        // A BOOL channel end-to-end: a spawned task sends a boolean, main receives + shows it as `true`/`false`
        // (never `1`). Exercises the `Bool` `ChanElem` + the bool renderer. Dataflow-fixed → `true`.
        "bool_channel_roundtrip",
        "## To produce (ch: Bool):\n    Send true into ch.\n\n## Main\n    Let ch be a Pipe of Bool.\n    \
         Launch a task to produce with ch.\n    Receive b from ch.\n    Show b.\n",
        Status::Supported,
    ),
    (
        // A BLOCKING NESTED CALL: `main` calls a helper `getOne` that itself `Receive`s and `Return`s — the
        // only corpus program that exercises the stackless call machinery (`logos_call_push`/`logos_call_return`:
        // push a callee frame wired to the caller's result cell, run it to its blocking `Receive`, park, and on
        // resume pop back into `main`'s `Let x`). Dataflow-fixed → 42.
        "blocking_call",
        "## To getOne (ch: Int) -> Int:\n    Receive v from ch.\n    Return v.\n\n\
         ## To produce (ch: Int):\n    Send 42 into ch.\n\n\
         ## Main\n    Let ch be a Pipe of Int.\n    Launch a task to produce with ch.\n    \
         Let x be getOne(ch).\n    Show x.\n",
        Status::Supported,
    ),
    (
        // TASK ABORT — the `abort_then_send` DISCRIMINATOR (same as `c_task_abort_lock`, now on the WALKER's
        // stackless path): `main` launches a blocker parked on `Receive`, `Stop`s it, THEN sends to its channel.
        // A FAITHFUL abort drops the aborted receiver, so its `Show "got"` never runs → only "main" prints. A
        // no-op abort would wake it and leak "got". Proves the walker's `logos_task_abort` + drop-on-send is
        // faithful, not a silent no-op. Dataflow-fixed → "main".
        "abort_then_send",
        "## To blocker (ch: Int):\n    Receive x from ch.\n    Show \"got\".\n\n\
         ## Main\n    Let ch be a Pipe of Int.\n    Let h be Launch a task to blocker with ch.\n    \
         Stop h.\n    Send 9 into ch.\n    Show \"main\".\n",
        Status::Supported,
    ),
    (
        // NON-BLOCKING `Try to send` (Op::ChanTrySend) — the value is buffered (an unbounded channel always
        // has room), then received: `logos_chan_try_send` never parks. Covers a concurrency op the stackless
        // transform previously refused. Dataflow-fixed → 5.
        "try_send_roundtrip",
        "## Main\n    Let ch be a Pipe of Int.\n    Try to send 5 into ch.\n    Receive x from ch.\n    Show x.\n",
        Status::Supported,
    ),
    (
        // `Try to receive` (Op::ChanTryRecv) with a value present — binds an OPTIONAL (the new `logos_opt` box);
        // `Show x` renders the SOME value. Proves the Optional extension: a scalar-CPS `Some(v)`. Dataflow → 5.
        "try_receive_some",
        "## Main\n    Let ch be a Pipe of Int.\n    Try to send 5 into ch.\n    Try to receive x from ch.\n    Show x.\n",
        Status::Supported,
    ),
    (
        // `Try to receive` on an EMPTY channel → `nothing` (the Optional's absent case), shown as `nothing`
        // (matching the VM's `RuntimeValue::Nothing`). Dataflow → nothing.
        "try_receive_none",
        "## Main\n    Let ch be a Pipe of Int.\n    Try to receive x from ch.\n    Show x.\n",
        Status::Supported,
    ),
    (
        // `Close ch` (Op::ChanClose) — the producer sends 1,2 then CLOSES; `main` receives 3 (a,b, then a
        // closed+empty read → `nothing`). Because the program closes a channel, blocking receives bind an
        // OPTIONAL (a `logos_opt`; a parked receiver's delivery is wrapped in `Some` via the task's `recv_opt`
        // flag). Dataflow → ["1","2","nothing"], byte-identical to the VM.
        "close_recv_after",
        "## To produce (ch: Int):\n    Send 1 into ch.\n    Send 2 into ch.\n    Close ch.\n\n\
         ## Main\n    Let ch be a Pipe of Int.\n    Launch a task to produce with ch.\n    \
         Receive a from ch.\n    Receive b from ch.\n    Receive c from ch.\n    Show a.\n    Show b.\n    Show c.\n",
        Status::Supported,
    ),
    (
        // `Await h` (Op::TaskAwait) — JOIN: `main` sends, then AWAITS the worker's completion before its final
        // `Show`. The worker receives + shows `9`, completes, wakes `main` (its `awaiter`), then `main` shows 0
        // → ["9","0"]. Proves the task `done`/`awaiter` plumbing. Dataflow-fixed.
        "await_join",
        "## To worker (ch: Int):\n    Receive x from ch.\n    Show x.\n\n## Main\n    Let ch be a Pipe of Int.\n    \
         Let h be Launch a task to worker with ch.\n    Send 9 into ch.\n    Await h.\n    Show 0.\n",
        Status::Supported,
    ),
    (
        // `Await h into r` — the awaited RESULT is `nothing` (the VM's offline scheduler does not propagate a
        // task's return value; the walker matches it EXACTLY). Worker returns 5, but `Show r` → `nothing`.
        "await_result_is_nothing",
        "## To worker -> Int:\n    Return 5.\n\n## Main\n    Let h be Launch a task to worker.\n    \
         Await h into r.\n    Show r.\n",
        Status::Supported,
    ),
];

const SUPPORTED_FLOOR: usize = 20;

/// The EXACT set of currently-`Gap` program labels — a shrink-only ledger. EMPTY: the whole determinate
/// concurrency corpus (channels + tasks + virtual-time sleep + select + text-literal Show) lowers ≡VM on the
/// bytecode walker's stackless path.
const KNOWN_GAPS: &[&str] = &[];

fn have_gpp() -> bool {
    std::process::Command::new("g++")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn have_gcc() -> bool {
    std::process::Command::new("gcc")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Build `c` with compiler `cc` at `-O2`, run it `runs` times, return (stdout lines, BEST wall-clock millis).
/// Best-of-N (the minimum) rejects scheduler/contention noise so the comparison reflects the code, not the box.
fn build_run_best_of(
    c: &str,
    cc: &str,
    ext: &str,
    label: &str,
    runs: usize,
) -> Option<(Vec<String>, u128)> {
    let dir = std::env::temp_dir().join(format!("logos_perf_{}_{}", std::process::id(), label));
    let _ = std::fs::create_dir_all(&dir);
    let cfile = dir.join(format!("prog.{ext}"));
    let bin = dir.join("prog");
    std::fs::write(&cfile, c).ok()?;
    let build = std::process::Command::new(cc)
        .args(["-O2", "-pthread", "-o"])
        .arg(&bin)
        .arg(&cfile)
        .arg("-lm")
        .output()
        .ok()?;
    if !build.status.success() {
        eprintln!(
            "[perf] {label} build failed:\n{}",
            String::from_utf8_lossy(&build.stderr)
                .lines()
                .take(4)
                .collect::<Vec<_>>()
                .join("\n")
        );
        return None;
    }
    let mut best = u128::MAX;
    let mut lines = Vec::new();
    for _ in 0..runs {
        let t0 = std::time::Instant::now();
        let run = std::process::Command::new(&bin).output().ok()?;
        let ms = t0.elapsed().as_millis();
        if !run.status.success() {
            return None;
        }
        lines = String::from_utf8_lossy(&run.stdout)
            .lines()
            .map(|s| s.to_string())
            .collect();
        best = best.min(ms);
    }
    let _ = std::fs::remove_dir_all(&dir);
    Some((lines, best))
}

/// Compile a program to C via the WALKER entry (`compile_to_c_via_bytecode`), build with **g++** (the
/// stackless body is C++), run, and return stdout lines. `None` if compile/build/run fails.
fn walker_lines(src: &str, label: &str) -> Option<Vec<String>> {
    let c = compile_to_c_via_bytecode(src).ok()?;
    let dir = std::env::temp_dir().join(format!("logos_wconc_{}_{}", std::process::id(), label));
    let _ = std::fs::create_dir_all(&dir);
    let cfile = dir.join("prog.cpp");
    let bin = dir.join("prog");
    std::fs::write(&cfile, &c).ok()?;
    let build = std::process::Command::new("g++")
        .args(["-O2", "-o"])
        .arg(&bin)
        .arg(&cfile)
        .arg("-lm")
        .output()
        .ok()?;
    if !build.status.success() {
        return None;
    }
    let run = std::process::Command::new(&bin).output().ok()?;
    let _ = std::fs::remove_dir_all(&dir);
    if !run.status.success() {
        return None;
    }
    Some(
        String::from_utf8_lossy(&run.stdout)
            .lines()
            .map(|s| s.to_string())
            .collect(),
    )
}

// ============================================================
//  Static ratchet — the pit-of-success census
// ============================================================

#[test]
fn the_walker_concurrency_gap_ledger_is_pinned_and_shrink_only() {
    let live: BTreeSet<&str> = CORPUS
        .iter()
        .filter(|(_, _, s)| *s == Status::Gap)
        .map(|(l, _, _)| *l)
        .collect();
    let declared: BTreeSet<&str> = KNOWN_GAPS.iter().copied().collect();
    assert_eq!(
        declared.len(),
        KNOWN_GAPS.len(),
        "KNOWN_GAPS has duplicate entries"
    );
    let new_gaps: Vec<_> = live.difference(&declared).collect();
    let closed_but_listed: Vec<_> = declared.difference(&live).collect();
    assert!(
        new_gaps.is_empty(),
        "NEW walker-concurrency gap(s) not in KNOWN_GAPS: {new_gaps:?}"
    );
    assert!(closed_but_listed.is_empty(), "KNOWN_GAPS lists now-Supported program(s): {closed_but_listed:?} — remove them and raise SUPPORTED_FLOOR");
}

#[test]
fn the_walker_supported_count_never_regresses() {
    let n = CORPUS
        .iter()
        .filter(|(_, _, s)| *s == Status::Supported)
        .count();
    assert!(n >= SUPPORTED_FLOOR, "walker-concurrency supported count fell to {n} (floor {SUPPORTED_FLOOR}) — fix the lowering, do not lower the floor");
}

/// A `Gap` program must genuinely REFUSE (typed error), never silently emit wrong code.
#[test]
fn every_gap_program_is_a_typed_refusal() {
    for (label, src, status) in CORPUS {
        if *status != Status::Gap {
            continue;
        }
        assert!(
            compile_to_c_via_bytecode(src).is_err(),
            "`{label}` is a Gap but the walker did NOT refuse it — either it now works (move to Supported) or it emits wrong code"
        );
    }
}

// ============================================================
//  Behavioural — the ≡VM biconditional (gated on g++)
// ============================================================

#[test]
fn every_supported_concurrency_program_runs_equiv_vm_on_the_walker() {
    if !have_gpp() {
        eprintln!("SKIP walker-concurrency execution: g++ not installed");
        return;
    }
    for (label, src, status) in CORPUS {
        if *status != Status::Supported {
            continue;
        }
        let want = interpret_for_ui_sync(src).lines;
        let got = walker_lines(src, label)
            .unwrap_or_else(|| panic!("`{label}` is Supported but its walker C++ failed to compile/run — fix the stackless host lowering"));
        assert_eq!(
            got, want,
            "`{label}`: walker stackless output must equal the deterministic scheduler oracle"
        );
    }
}

// ============================================================
//  The SCHEDULER KNOBS — policy is a real, output-preserving tunable
// ============================================================

/// The five ready-queue policies (mirror `logicaffeine_runtime::SchedulePolicy`) and the `#define` token
/// the emitted scheduler must carry for each — proof the knob threads all the way into the emitted C.
const POLICIES: &[(SchedulePolicy, &str)] = &[
    (SchedulePolicy::Fifo, "LOGOS_SCHED_FIFO"),
    (SchedulePolicy::Lifo, "LOGOS_SCHED_LIFO"),
    (SchedulePolicy::RoundRobin, "LOGOS_SCHED_ROUNDROBIN"),
    (SchedulePolicy::Random, "LOGOS_SCHED_RANDOM"),
    (SchedulePolicy::Priority, "LOGOS_SCHED_PRIORITY"),
];

fn knobs_for(policy: SchedulePolicy) -> SchedulerKnobs {
    SchedulerKnobs {
        config: SchedulerConfig::default().with_policy(policy),
        seed: 0,
    }
}

fn walker_lines_with_knobs(src: &str, label: &str, knobs: &SchedulerKnobs) -> Option<Vec<String>> {
    let c = compile_to_c_via_bytecode_with_knobs(src, knobs).ok()?;
    let dir = std::env::temp_dir().join(format!("logos_wpol_{}_{}", std::process::id(), label));
    let _ = std::fs::create_dir_all(&dir);
    let cfile = dir.join("prog.cpp");
    let bin = dir.join("prog");
    std::fs::write(&cfile, &c).ok()?;
    // `-pthread` so a `workers > 1` build links the work-stealing pool (harmless for the cooperative default).
    let build = std::process::Command::new("g++")
        .args(["-O2", "-pthread", "-o"])
        .arg(&bin)
        .arg(&cfile)
        .arg("-lm")
        .output()
        .ok()?;
    if !build.status.success() {
        return None;
    }
    let run = std::process::Command::new(&bin).output().ok()?;
    let _ = std::fs::remove_dir_all(&dir);
    if !run.status.success() {
        return None;
    }
    Some(
        String::from_utf8_lossy(&run.stdout)
            .lines()
            .map(|s| s.to_string())
            .collect(),
    )
}

fn knobs_workers(workers: usize) -> SchedulerKnobs {
    SchedulerKnobs {
        config: SchedulerConfig::default().with_workers(workers),
        seed: 0,
    }
}

// ============================================================
//  The WORKERS knob — work-stealing (M:N) vs single-core, output-invariant
// ============================================================

/// The `workers` knob emits a REAL pthreads work-stealing pool for `workers > 1`, and NOTHING extra for the
/// cooperative default (`workers == 1`, also the device path) — so a non-concurrency / default build never
/// pays for pthreads. (Static: no compiler needed.)
#[test]
fn the_workers_knob_emits_a_pthreads_pool() {
    let (_, src, _) = CORPUS
        .iter()
        .find(|(l, _, _)| *l == "producer_consumer")
        .unwrap();
    let one = compile_to_c_via_bytecode_with_knobs(src, &knobs_workers(1))
        .expect("concurrency program lowers");
    assert!(
        one.contains("#define LOGOS_SCHED_WORKERS 1"),
        "workers=1 define must be present"
    );
    let four = compile_to_c_via_bytecode_with_knobs(src, &knobs_workers(4))
        .expect("concurrency program lowers");
    assert!(
        four.contains("#define LOGOS_SCHED_WORKERS 4"),
        "workers=4 define must be present"
    );
    // The work-stealing pool is always emitted, GUARDED by the workers knob — active only for workers>1
    // (so the cooperative default / device path preprocesses it out and never links pthreads).
    assert!(
        four.contains("pthread_create") && four.contains("#if LOGOS_SCHED_WORKERS > 1"),
        "workers>1 must carry a real pthreads work-stealing pool guarded by the workers knob"
    );
}

/// The WORKER-COUNT INVARIANCE (mirrors `logicaffeine_runtime`'s proven property): a program run on the
/// `workers = 4` work-stealing pool produces BYTE-IDENTICAL output to the `workers = 1` cooperative loop —
/// and both equal the VM oracle. The knob buys cores, never a different answer: a worker holds the scheduler
/// lock across each step, so execution is serialized in the same deterministic order regardless of thread
/// count. This is the proof the scheduler "can become work-stealing or single-cored" without losing determinism.
#[test]
fn the_worker_count_is_output_invariant() {
    if !have_gpp() {
        eprintln!("SKIP walker-concurrency worker-count sweep: g++ not installed");
        return;
    }
    for (label, src, status) in CORPUS {
        if *status != Status::Supported {
            continue;
        }
        let want = interpret_for_ui_sync(src).lines;
        let one = walker_lines_with_knobs(src, &format!("{label}_w1"), &knobs_workers(1))
            .unwrap_or_else(|| panic!("`{label}` workers=1 failed to compile/run"));
        let four = walker_lines_with_knobs(src, &format!("{label}_w4"), &knobs_workers(4))
            .unwrap_or_else(|| panic!("`{label}` workers=4 failed to compile/run"));
        assert_eq!(one, four, "`{label}`: workers=4 (work-stealing) must produce byte-identical output to workers=1 (cooperative)");
        assert_eq!(
            four, want,
            "`{label}`: workers=4 must still equal the VM oracle"
        );
    }
}

/// Each policy must thread all the way into the emitted scheduler as its distinct `#define` — the knob is
/// not silently ignored. (Static: no compiler needed.)
#[test]
fn the_policy_knob_threads_into_the_emitted_scheduler() {
    // A representative Supported concurrency program.
    let (_, src, _) = CORPUS
        .iter()
        .find(|(l, _, _)| *l == "producer_consumer")
        .unwrap();
    for (policy, token) in POLICIES {
        let c = compile_to_c_via_bytecode_with_knobs(src, &knobs_for(*policy))
            .expect("concurrency program lowers");
        let want = format!("#define LOGOS_SCHED_POLICY {token}");
        assert!(
            c.contains(&want),
            "policy {policy:?} must emit `{want}` into the scheduler"
        );
    }
}

/// Programs whose OUTPUT is fixed by DATAFLOW (channel values / a single task's own order), independent of
/// the ready-queue policy — so the AOT walker and the VM oracle must agree at EVERY knob (the cross-tier
/// knob equivalence `walker-at-P == VM-at-P`). `spawn_with_arg` is EXCLUDED: its two concurrent `Show`s have
/// a policy-DEFINED order, so a non-Fifo policy legitimately reorders them (a correct scheduler difference,
/// NOT a bug — see `the_policy_knob_genuinely_reorders_concurrent_shows`).
const DATAFLOW_DETERMINATE: &[&str] = &[
    "producer_consumer",
    "sleep_virtual_time",
    "select_recv_timeout",
    "select_timeout_wins",
    "text_channel_roundtrip",
    "real_channel_roundtrip",
    "float_literal_show",
    "loop_receive",
    "blocking_call",
    "abort_then_send",
    "try_send_roundtrip",
    "try_receive_some",
    "try_receive_none",
    "close_recv_after",
    "await_join",
    "await_result_is_nothing",
];

/// Every policy is DETERMINISTIC on the walker: the same program under the same knob produces the same output
/// on repeated compiles/runs (the emitted scheduler has no wall-clock / real-thread nondeterminism).
#[test]
fn every_policy_is_deterministic_on_the_walker() {
    if !have_gpp() {
        eprintln!("SKIP walker-concurrency determinism sweep: g++ not installed");
        return;
    }
    for (label, src, status) in CORPUS {
        if *status != Status::Supported {
            continue;
        }
        for (policy, _) in POLICIES {
            let a =
                walker_lines_with_knobs(src, &format!("{label}_{policy:?}_a"), &knobs_for(*policy))
                    .unwrap_or_else(|| panic!("`{label}` under {policy:?} failed to compile/run"));
            let b =
                walker_lines_with_knobs(src, &format!("{label}_{policy:?}_b"), &knobs_for(*policy))
                    .unwrap_or_else(|| panic!("`{label}` under {policy:?} failed to compile/run"));
            assert_eq!(
                a, b,
                "`{label}` under {policy:?} is nondeterministic across runs"
            );
        }
    }
}

/// The CROSS-TIER KNOB EQUIVALENCE: for a dataflow-determinate program, the AOT walker at knob `P` matches
/// the VM oracle at the SAME knob `P`, for every policy — the emitted scheduler and the oracle scheduler are
/// the same deterministic machine, tuned by the same `SchedulerConfig`. (Output is fixed by dataflow here, so
/// the ready-queue reordering cannot change it — isolating the knob-equivalence from the concurrent-Show
/// ordering which is legitimately policy-dependent.)
#[test]
fn dataflow_determinate_programs_agree_cross_tier_at_every_knob() {
    if !have_gpp() {
        eprintln!("SKIP walker-concurrency cross-tier knob sweep: g++ not installed");
        return;
    }
    for (label, src, status) in CORPUS {
        if *status != Status::Supported || !DATAFLOW_DETERMINATE.contains(label) {
            continue;
        }
        for (policy, _) in POLICIES {
            let config = SchedulerConfig::default().with_policy(*policy);
            let vm = run_vm_concurrent_with_config(src, config, 0).lines;
            let got = walker_lines_with_knobs(
                src,
                &format!("{label}_{policy:?}_xt"),
                &knobs_for(*policy),
            )
            .unwrap_or_else(|| panic!("`{label}` under {policy:?} failed to compile/run"));
            assert_eq!(got, vm, "`{label}` at knob {policy:?}: AOT walker must equal the VM oracle at the SAME knob");
        }
    }
}

/// The policy knob is REAL, not cosmetic: a concurrent program with two schedulable `Show`s (`spawn_with_arg`
/// — a spawned `Show 7` racing `main`'s `Show 0`) is REORDERED by at least one non-Fifo policy. This proves
/// `logos_ready_pop` genuinely dispatches by policy (LIFO surfaces the newest-ready task first), so switching
/// the knob switches the schedule — exactly the tunability the design promises.
#[test]
fn the_policy_knob_genuinely_reorders_concurrent_shows() {
    if !have_gpp() {
        eprintln!("SKIP walker-concurrency knob-reality check: g++ not installed");
        return;
    }
    let (_, src, _) = CORPUS
        .iter()
        .find(|(l, _, _)| *l == "spawn_with_arg")
        .unwrap();
    let fifo = walker_lines_with_knobs(src, "spawn_fifo", &knobs_for(SchedulePolicy::Fifo))
        .expect("compiles");
    let reordered = POLICIES.iter().any(|(p, _)| {
        walker_lines_with_knobs(src, &format!("spawn_{p:?}"), &knobs_for(*p))
            .map(|l| l != fifo)
            .unwrap_or(false)
    });
    assert!(reordered, "no policy reordered spawn_with_arg's concurrent Shows — the knob is not actually dispatching by policy");
}

// ============================================================
//  PERF — the stackless tier must be FASTER than the ucontext tier
// ============================================================

/// The walker's STACKLESS concurrency (tiny heap frame + a cheap function-return yield per block) must be at
/// least as fast as — in practice much faster than — the shipping AST-C UCONTEXT concurrency (a 256 KB stack
/// per task + a `swapcontext` per yield). On a spawn + channel-heavy benchmark (500 tasks fanning one value
/// each into `main`), ucontext pays 500 × 256 KB of stacks + ~1000 context switches; stackless pays 500 tiny
/// frames + cheap returns. This locks the "C must get FASTER" invariant for the concurrency tier — measured
/// best-of-5 (min) so build/box contention cannot flake it, and both tiers must agree on the answer (500).
#[test]
fn stackless_concurrency_is_faster_than_ucontext() {
    if !have_gpp() || !have_gcc() {
        eprintln!("SKIP stackless-vs-ucontext perf: need both g++ and gcc");
        return;
    }
    const BENCH: &str = "## To worker (ch: Int):\n    Send 1 into ch.\n\n## Main\n    Let ch be a Pipe of Int.\n    \
        Let i be 0.\n    While i < 500:\n        Launch a task to worker with ch.\n        Set i to i + 1.\n    \
        Let total be 0.\n    Let j be 0.\n    While j < 500:\n        Receive v from ch.\n        \
        Set total to total + v.\n        Set j to j + 1.\n    Show total.\n";

    let sl_c = compile_to_c_via_bytecode(BENCH)
        .expect("stackless walker lowers the spawn/channel benchmark");
    let uc_c = compile_to_c(BENCH).expect("ucontext AST-C lowers the spawn/channel benchmark");

    // Structural: the stackless output must NOT use the expensive ucontext machinery — its speed is by
    // construction (a function-return yield, no per-switch context save/restore). The ucontext baseline does.
    assert!(!sl_c.contains("swapcontext") && !sl_c.contains("makecontext"),
        "the stackless tier must not use ucontext context-switching (its cheap-yield design is why it's faster)");

    let (sl_out, sl_ms) = build_run_best_of(&sl_c, "g++", "cpp", "stackless", 5)
        .expect("stackless benchmark builds+runs");
    let (uc_out, uc_ms) = build_run_best_of(&uc_c, "gcc", "c", "ucontext", 5)
        .expect("ucontext benchmark builds+runs");

    assert_eq!(
        sl_out,
        vec!["500".to_string()],
        "stackless benchmark must compute 500"
    );
    assert_eq!(
        uc_out,
        vec!["500".to_string()],
        "ucontext benchmark must compute 500"
    );

    let speedup = uc_ms as f64 / (sl_ms.max(1)) as f64;
    eprintln!("[perf] stackless={sl_ms}ms  ucontext={uc_ms}ms  ({speedup:.2}x faster)");
    assert!(
        sl_ms <= uc_ms,
        "the stackless concurrency tier ({sl_ms}ms) must be at least as fast as ucontext ({uc_ms}ms) — the \
         'C must get FASTER' invariant for concurrency"
    );
}

// ============================================================
//  FUZZ — random DETERMINATE concurrency programs, walker ≡ VM
// ============================================================

/// A self-contained SplitMix64 for the fuzzer (deterministic; no `rand` dep).
struct FRng(u64);
impl FRng {
    fn new(seed: u64) -> Self {
        FRng(seed ^ 0x1234_5678_9abc_def0)
    }
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// One CONSTRUCT the fuzzer can emit — a code path in the stackless transform. The generator TAGS each program
/// with the constructs it used; `the_fuzzer_combinatorially_covers_every_construct` proves that over a bounded
/// seed sweep the UNION of tags equals `COVER_CATALOG` — i.e. there is NO declared path the random fuzzer
/// cannot reach. Each tag maps to a `stackless::Node` variant / `ChanElem` / `render_*` branch:
/// shapes → Spawn/Send/Recv/Sleep/Select(Recv+Timeout)/Abort/Call/While/If/Return; elems → the 4 `ChanElem`
/// send-encode + typed-`Show` renderers.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Cover {
    ShapeIntFanIn,       // spawn(s) + channel + blocking While + sum
    ShapeTypedRoundtrip, // one send + one recv + typed Show, per ChanElem
    ShapeSelectRecv,     // Select — the recv arm wins
    ShapeSelectTimeout,  // Select — the timeout arm wins
    ShapeAbort,          // Stop a parked task, then send (faithful drop)
    ShapeTrySend,        // non-blocking `Try to send` then receive (Op::ChanTrySend)
    ShapeTryRecv, // non-blocking `Try to receive` → an OPTIONAL (Some/nothing), Op::ChanTryRecv
    ShapeClose,   // `Close ch` + a blocking receive past the buffer → nothing (Op::ChanClose)
    ShapeAwait,   // `Await h into r` — join a task; the result binding is `nothing` (Op::TaskAwait)
    MultiSpawn,   // >1 concurrent producer
    Sleep,        // a virtual-time park inside a task
    RecvViaCall,  // drain through a blocking helper (Node::Call in a loop)
    Branch,       // a conditional (Node::If)
    ElemInt,
    ElemBool,
    ElemText,
    ElemReal,
}

/// The TOTAL set of constructs the fuzzer must be able to reach. The coverage lock proves the generator can
/// combinatorially produce every one; a new construct here that the generator never emits makes the lock RED.
const COVER_CATALOG: &[Cover] = &[
    Cover::ShapeIntFanIn,
    Cover::ShapeTypedRoundtrip,
    Cover::ShapeSelectRecv,
    Cover::ShapeSelectTimeout,
    Cover::ShapeAbort,
    Cover::ShapeTrySend,
    Cover::ShapeTryRecv,
    Cover::ShapeClose,
    Cover::ShapeAwait,
    Cover::MultiSpawn,
    Cover::Sleep,
    Cover::RecvViaCall,
    Cover::Branch,
    Cover::ElemInt,
    Cover::ElemBool,
    Cover::ElemText,
    Cover::ElemReal,
];

/// Generate a DETERMINATE concurrency program from `seed`, returning the source + the CONSTRUCTS it exercises.
/// The `seed % 5` picks one of five determinate SHAPES; `seed / 5` (the "variant") deterministically walks the
/// cross-cutting features (multi-spawn / sleep / recv-via-call / branch, and the four channel element types),
/// so a bounded seed sweep reaches EVERY construct (see the coverage lock). Every shape's OUTPUT is fixed by
/// dataflow (a sum, a single value, a select winner, an aborted-task drop) — never a concurrent-`Show` race —
/// so the VM oracle is a unique target the walker must match.
fn fuzz_concurrency_program(seed: u64) -> (String, Vec<Cover>) {
    let mut r = FRng::new(seed);
    let variant = seed / 5;
    let mut cov: Vec<Cover> = Vec::new();
    let mut src = String::new();
    match seed % 9 {
        0 => {
            cov.push(Cover::ShapeIntFanIn);
            cov.push(Cover::ElemInt);
            let use_call = variant & 1 == 1;
            let use_sleep = variant & 2 == 2;
            let use_branch = variant & 4 == 4;
            let producers = 1 + (variant % 3) as usize; // 1..=3
            if producers > 1 {
                cov.push(Cover::MultiSpawn);
            }
            if use_call {
                cov.push(Cover::RecvViaCall);
                src.push_str(
                    "## To getOne (ch: Int) -> Int:\n    Receive v from ch.\n    Return v.\n\n",
                );
            }
            let mut count = 0usize;
            for p in 0..producers {
                src.push_str(&format!("## To produce{p} (ch: Int):\n"));
                // When sleeping, force >=2 sends so a `Sleep` is actually emitted (guaranteed coverage).
                let sends = if use_sleep {
                    2 + r.below(2) as usize
                } else {
                    1 + r.below(3) as usize
                };
                for k in 0..sends {
                    src.push_str(&format!("    Send {} into ch.\n", r.below(50)));
                    count += 1;
                    if use_sleep && k + 1 < sends {
                        src.push_str("    Sleep 1.\n");
                    }
                }
                src.push('\n');
            }
            if use_sleep {
                cov.push(Cover::Sleep);
            }
            src.push_str("## Main\n    Let ch be a Pipe of Int.\n    Let total be 0.\n");
            for p in 0..producers {
                src.push_str(&format!("    Launch a task to produce{p} with ch.\n"));
            }
            let recv = if use_call {
                "Let v be getOne(ch)."
            } else {
                "Receive v from ch."
            };
            src.push_str(&format!(
                "    Let i be 0.\n    While i < {count}:\n        {recv}\n        \
                 Set total to total + v.\n        Set i to i + 1.\n"
            ));
            if use_branch {
                cov.push(Cover::Branch);
                src.push_str(
                    "    If total > 0:\n        Show total.\n    Otherwise:\n        Show 0.\n",
                );
            } else {
                src.push_str("    Show total.\n");
            }
        }
        1 => {
            cov.push(Cover::ShapeTypedRoundtrip);
            let (ty, val, elem) = match variant % 4 {
                0 => ("Int", format!("{}", r.below(100)), Cover::ElemInt),
                1 => (
                    "Bool",
                    if r.below(2) == 1 {
                        "true".into()
                    } else {
                        "false".into()
                    },
                    Cover::ElemBool,
                ),
                2 => (
                    "Text",
                    if r.below(2) == 1 {
                        "\"hi\"".into()
                    } else {
                        "\"yo\"".into()
                    },
                    Cover::ElemText,
                ),
                _ => (
                    "Real",
                    if r.below(2) == 1 {
                        "1.5".into()
                    } else {
                        "2.5".into()
                    },
                    Cover::ElemReal,
                ),
            };
            cov.push(elem);
            src.push_str(&format!(
                "## To produce (ch: {ty}):\n    Send {val} into ch.\n\n## Main\n    Let ch be a Pipe of {ty}.\n    \
                 Launch a task to produce with ch.\n    Receive x from ch.\n    Show x.\n"
            ));
        }
        2 => {
            cov.push(Cover::ShapeSelectRecv);
            cov.push(Cover::ElemInt);
            src.push_str(&format!(
                "## To produce (ch: Int):\n    Send {} into ch.\n\n## Main\n    Let ch be a Pipe of Int.\n    \
                 Launch a task to produce with ch.\n    Await the first of:\n        Receive v from ch:\n            \
                 Show v.\n        After 100 seconds:\n            Show 0.\n",
                r.below(100)
            ));
        }
        3 => {
            cov.push(Cover::ShapeSelectTimeout);
            cov.push(Cover::ElemInt);
            src.push_str(&format!(
                "## Main\n    Let ch be a Pipe of Int.\n    Await the first of:\n        Receive v from ch:\n            \
                 Show v.\n        After 5 seconds:\n            Show {}.\n",
                r.below(100)
            ));
        }
        4 => {
            cov.push(Cover::ShapeAbort);
            cov.push(Cover::ElemInt);
            src.push_str(
                "## To blocker (ch: Int):\n    Receive x from ch.\n    Show \"got\".\n\n## Main\n    \
                 Let ch be a Pipe of Int.\n    Let h be Launch a task to blocker with ch.\n    Stop h.\n    \
                 Send 9 into ch.\n    Show \"main\".\n",
            );
        }
        5 => {
            // NON-BLOCKING try-send then receive — Op::ChanTrySend (buffers, never parks) → the value round-trips.
            cov.push(Cover::ShapeTrySend);
            cov.push(Cover::ElemInt);
            src.push_str(&format!(
                "## Main\n    Let ch be a Pipe of Int.\n    Try to send {} into ch.\n    Receive x from ch.\n    Show x.\n",
                r.below(100)
            ));
        }
        6 => {
            // `Try to receive` → an OPTIONAL (Op::ChanTryRecv). Half the variants try-send first (Some), half
            // receive from empty (nothing) — both determinate single-threaded.
            cov.push(Cover::ShapeTryRecv);
            cov.push(Cover::ElemInt);
            if variant & 1 == 1 {
                src.push_str(&format!(
                    "## Main\n    Let ch be a Pipe of Int.\n    Try to send {} into ch.\n    \
                     Try to receive x from ch.\n    Show x.\n",
                    r.below(100)
                ));
            } else {
                src.push_str("## Main\n    Let ch be a Pipe of Int.\n    Try to receive x from ch.\n    Show x.\n");
            }
        }
        7 => {
            // `Close ch` (Op::ChanClose): a producer sends a value then closes; `main` receives it (Some) and
            // then reads past the buffer (nothing). Blocking receives bind OPTIONALs because the program closes.
            cov.push(Cover::ShapeClose);
            cov.push(Cover::ElemInt);
            src.push_str(&format!(
                "## To produce (ch: Int):\n    Send {} into ch.\n    Close ch.\n\n## Main\n    \
                 Let ch be a Pipe of Int.\n    Launch a task to produce with ch.\n    Receive a from ch.\n    \
                 Receive b from ch.\n    Show a.\n    Show b.\n",
                r.below(100)
            ));
        }
        _ => {
            // `Await h into r` (Op::TaskAwait): a task returns a value; `main` awaits + shows the result, which
            // is `nothing` (the VM's offline await discards the return — matched exactly). Determinate.
            cov.push(Cover::ShapeAwait);
            cov.push(Cover::ElemInt);
            src.push_str(&format!(
                "## To worker -> Int:\n    Return {}.\n\n## Main\n    Let h be Launch a task to worker.\n    \
                 Await h into r.\n    Show r.\n",
                r.below(100)
            ));
        }
    }
    cov.sort();
    cov.dedup();
    (src, cov)
}

/// The concurrency analogue of the sequential op-walker fuzzer: over many seeds, a random determinate
/// concurrency program must (a) run on the VM oracle to a SCHEDULE-INDEPENDENT output (asserted by comparing the
/// oracle under Fifo vs Lifo — a real determinacy guard, not a same-seed tautology), and (b) on the walker's
/// stackless path either LOWER and match byte-for-byte, or cleanly REFUSE (typed `Err`). An `Ok` C++ body that
/// fails to build/run, or diverges, is a HARD panic (silent bad-codegen). Default 40 seeds; `LOGOS_CONC_FUZZ_SEEDS=N`.
#[test]
fn stackless_concurrency_fuzz_differential() {
    if !have_gpp() {
        eprintln!("SKIP stackless concurrency fuzz: g++ not installed");
        return;
    }
    let n: u64 = std::env::var("LOGOS_CONC_FUZZ_SEEDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(40);
    let mut ran = 0u64;
    let mut refused = 0u64;
    for seed in 0..n {
        let (src, _cov) = fuzz_concurrency_program(seed);
        // Oracle + determinacy guard: the output must be schedule-INDEPENDENT (Fifo == Lifo), so it is a unique
        // target. (Every generated shape is confluent by construction; this catches a generator that isn't.)
        let vm = run_vm_concurrent_with_config(&src, SchedulerConfig::default(), 0);
        assert!(
            vm.error.is_none(),
            "seed {seed}: oracle errored: {:?}\n{src}",
            vm.error
        );
        let vm_lifo = run_vm_concurrent_with_config(
            &src,
            SchedulerConfig::default().with_policy(SchedulePolicy::Lifo),
            0,
        );
        assert_eq!(
            vm.lines, vm_lifo.lines,
            "seed {seed}: generated program is NONDETERMINATE (Fifo != Lifo)\n{src}"
        );
        // Walker: a clean typed refusal is allowed (counted); an `Ok` body MUST build + run + match.
        match compile_to_c_via_bytecode(&src) {
            Err(_) => refused += 1,
            Ok(c) => {
                let (lines, _ms) = build_run_best_of(&c, "g++", "cpp", &format!("cfuzz{seed}"), 1)
                    .unwrap_or_else(|| panic!("seed {seed}: walker emitted C++ that failed to build/run (bad codegen)\n{src}"));
                assert_eq!(
                    lines, vm.lines,
                    "seed {seed}: walker != VM — SILENT MISCOMPILE\n{src}"
                );
                ran += 1;
            }
        }
    }
    assert!(
        ran >= n * 8 / 10,
        "walker ran only {ran}/{n} fuzzed concurrency programs (>=80% expected)"
    );
    eprintln!("[conc-fuzz] {ran} ran ≡VM, {refused} refused, 0 divergences over {n} seeds");
}

/// ★ COMBINATORIAL COVERAGE COMPLETENESS: over a bounded seed sweep, the fuzzer generator must produce a program
/// exercising EVERY construct in `COVER_CATALOG` — proving there is no declared stackless-transform path the
/// random fuzzer CANNOT reach (the user's ask: "ensure there's no code path we couldn't exercise"). Bidirectional:
/// every catalog construct is reached (no unreachable path) AND every reached tag is in the catalog (no untracked
/// construct). Pure generation (no compiler needed), so it always runs. If a construct is unreachable in the
/// sweep, either widen the generator or shrink the catalog — never silently ship a path the fuzzer can't hit.
#[test]
fn the_fuzzer_combinatorially_covers_every_construct() {
    const SWEEP: u64 = 100; // > enough for the seed%9 shape × variant walk to hit every construct
    let mut covered: BTreeSet<Cover> = BTreeSet::new();
    for seed in 0..SWEEP {
        let (_src, cov) = fuzz_concurrency_program(seed);
        covered.extend(cov);
    }
    let catalog: BTreeSet<Cover> = COVER_CATALOG.iter().copied().collect();
    assert_eq!(
        catalog.len(),
        COVER_CATALOG.len(),
        "duplicate entry in COVER_CATALOG"
    );
    let unreached: Vec<_> = catalog.difference(&covered).collect();
    let untracked: Vec<_> = covered.difference(&catalog).collect();
    assert!(unreached.is_empty(), "fuzzer CANNOT reach these constructs in {SWEEP} seeds: {unreached:?} — widen the generator");
    assert!(
        untracked.is_empty(),
        "fuzzer emitted constructs NOT in COVER_CATALOG: {untracked:?} — add them to the catalog"
    );
    eprintln!(
        "[conc-fuzz-coverage] all {} constructs reachable within {SWEEP} seeds",
        COVER_CATALOG.len()
    );
}

// The whole BLOCKING-concurrency subset (channels + tasks + select + sleep + abort + try-send + try-recv +
// close + await) is now covered ≡VM. Refusal SAFETY for anything outside it (a non-scalar channel payload, a
// Net/Crdt/Tcp op) is enforced by `stackless_concurrency_fuzz_differential`: an `Ok` body must match the VM,
// while a typed `Err` is an allowed, counted refusal — a construct can never silently miscompile.

// ============================================================
//  OP-LEVEL PARTITION — every concurrency op is covered ≡VM (with a witness) or out-of-subset
// ============================================================

#[derive(Clone, Copy, PartialEq, Eq)]
enum OpCov {
    /// Lowered ≡VM through the walker's OWN stackless path — `witness` names a Supported `CORPUS` program that
    /// exercises it, proven by `every_supported_concurrency_program_runs_equiv_vm_on_the_walker`.
    Covered(&'static str),
    /// Lowered NATIVELY by the bytecode Op-walker (`from_program`, mirroring `codegen_riscv`) — no delegation.
    /// `witness` names an `OFFLINE_CORPUS` `NativeRun` program whose walker C build+runs byte-for-byte ==VM
    /// (offline CRDT lattice / loopback networking / policy check — deterministic, CI-safe). Proven by
    /// `offline_subsystem_ops_run_equiv_vm_on_the_native_walker` + `native_offline_ops_do_not_delegate`.
    NativeRun(&'static str),
    /// Lowered NATIVELY by the Op-walker, but the sync VM oracle cannot run it offline (it dials a REAL socket via
    /// the async reactor and panics under `interpret_for_ui_sync`), so there is no deterministic OFFLINE ≡VM-run
    /// witness in this lock — only compile-native. Two ops: `NetConnect` (the offline-loopback no-op) and `Tcp`
    /// (the real-socket client over `C_TCP_RUNTIME`; its genuine end-to-end ≡VM proof lives in `tcp_walker_lock`).
    NativeNoOracle(&'static str),
    /// Still DELEGATED to the shipping AST-C tier (`compile_to_c`). ★ EMPTY: no op delegates anymore — the LAST
    /// holdout (the real-socket `Tcp` client) went native this campaign. The variant is kept so the partition can
    /// name a future op that genuinely cannot be lowered; `nothing_delegates_the_gate_is_empty` asserts the gate.
    Delegated(&'static str),
    /// No LOGOS source emits this op — it exists only as internal/hand-bytecode (`CrdtJoin`'s per-field MAX
    /// G-Counter join). Nothing to witness at the source level; matches the `wprog_op_reachability_lock` census.
    NotEmittable(&'static str),
}

/// The VM's WHOLE concurrency op set (the 29 in `c_bytecode_lock::OPS_ASYNC_REFUSED`), each classified. This is
/// the definitive FULL-COVERAGE partition: every blocking-concurrency op is Covered ≡VM with a corpus witness;
/// the rest are a documented, out-of-subset frontier. Total + witnessed + shrink-only (a covered op can never
/// regress to refused without failing here). If the ISA gains a concurrency op, this list (and the census in
/// `c_bytecode_lock`) must classify it — nothing silently unaccounted for.
const CONCURRENCY_OPS: &[(&str, OpCov)] = &[
    ("ChanNew", OpCov::Covered("producer_consumer")),
    ("ChanSend", OpCov::Covered("producer_consumer")),
    ("ChanRecv", OpCov::Covered("producer_consumer")),
    ("ChanClose", OpCov::Covered("close_recv_after")),
    ("ChanTrySend", OpCov::Covered("try_send_roundtrip")),
    ("ChanTryRecv", OpCov::Covered("try_receive_some")),
    ("Spawn", OpCov::Covered("spawn_with_arg")),
    ("SpawnHandle", OpCov::Covered("abort_then_send")),
    ("Sleep", OpCov::Covered("sleep_virtual_time")),
    ("SelectWait", OpCov::Covered("select_recv_timeout")),
    ("SelectArmRecv", OpCov::Covered("select_recv_timeout")),
    ("SelectArmTimeout", OpCov::Covered("select_timeout_wins")),
    ("TaskAwait", OpCov::Covered("await_join")),
    ("TaskAbort", OpCov::Covered("abort_then_send")),
    // Offline networking — lowered NATIVELY by the Op-walker (loopback `C_NET_RUNTIME`), build+run ==VM.
    ("NetListen", OpCov::NativeRun("net_listen")),
    ("NetMakePeer", OpCov::NativeRun("peer_send_await")),
    ("NetSend", OpCov::NativeRun("peer_send_await")),
    ("NetStream", OpCov::NativeRun("net_stream")),
    ("NetSync", OpCov::NativeRun("sync_only")),
    ("NetAwait", OpCov::NativeRun("peer_send_await")),
    // `Connect to` — native offline no-op (like AST-C/RISC-V); the sync VM oracle dials a real socket → no run.
    ("NetConnect", OpCov::NativeNoOracle("connect_sync")),
    // Real-socket TCP client — lowered NATIVELY over the `C_TCP_RUNTIME` POSIX socket runtime; like `NetConnect`
    // it dials a REAL socket, so `interpret_for_ui_sync` panics and there is no OFFLINE ≡VM-run witness here (its
    // genuine end-to-end round-trip + refused-connect ≡VM proof lives in `tcp_walker_lock`).
    ("Tcp", OpCov::NativeNoOracle("tcp")),
    // CRDT lattice — lowered NATIVELY by the Op-walker (counter map + register/sequence), build+run ==VM.
    ("NewCrdt", OpCov::NativeRun("crdt_new_bump")),
    ("CrdtBump", OpCov::NativeRun("crdt_new_bump")),
    ("CrdtAppend", OpCov::NativeRun("crdt_append")),
    // `Converge x into y` (added this campaign) emits CrdtJoin — the idempotent per-field-MAX lattice join —
    // so it is now source-emittable AND lowered ≡VM by the native Op-walker (its `is_join` arm).
    ("CrdtJoin", OpCov::NativeRun("converge")),
    ("CrdtMerge", OpCov::NativeRun("crdt_merge")),
    ("CrdtResolve", OpCov::NativeRun("crdt_resolve")),
    // Runtime authorization — lowered NATIVELY (policy condition compiled against the subject's field cells).
    ("CheckPolicy", OpCov::NativeRun("check_policy")),
];

/// How each offline-subsystem program is witnessed against the native Op-walker:
///   - `NativeRun(expected)` — the walker lowers it NATIVELY (no delegation) and its C build+runs byte-for-byte
///     ==VM == `expected` (offline CRDT lattice + loopback networking + policy check — deterministic, CI-safe).
///   - `NativeCompileOnly` — the walker lowers it NATIVELY, but the sync VM oracle dials a REAL socket and panics
///     offline, so there is no OFFLINE ≡VM-run here (the `Connect to` offline no-op, and the real-socket `Tcp`
///     client whose genuine end-to-end ≡VM proof lives in `tcp_walker_lock`).
///   - `Delegate` — routed to the shipping AST-C tier. ★ EMPTY: no program delegates anymore (the gate is closed).
#[derive(Clone, Copy, PartialEq, Eq)]
enum DTier {
    NativeRun(&'static [&'static str]),
    NativeCompileOnly,
    Delegate,
}

/// The offline-subsystem corpus (loopback networking / CRDT lattice / policy / TCP). All but one lower NATIVELY
/// through the bytecode Op-walker (`from_program`, mirroring `codegen_riscv`) — reusing `C_NET_RUNTIME` /
/// `C_CRDT_RUNTIME` + the policy-condition compiler; only the real-socket `Tcp` client still delegates to AST-C.
/// Each names the op(s) it emits (comment) + its witnessing tier — the source-level witness set the partition
/// draws on: the 12 native ops name a `NativeRun` program (or `connect_sync` for `NetConnect`); `Tcp` a `Delegate`.
const OFFLINE_CORPUS: &[(&str, &str, DTier)] = &[
    // ── CRDT lattice + policy — native, build+run ==VM ──
    (
        "crdt_new_bump", // NewCrdt, CrdtBump
        "## Definition\nA Counter is Shared and has:\n    points: ConvergentCount.\n\n## Main\nLet mutable c be a new Counter.\nIncrease c's points by 5.\nShow c's points.\n",
        DTier::NativeRun(&["5"]),
    ),
    (
        "crdt_merge", // NewCrdt, CrdtBump, CrdtMerge
        "## Definition\nA Counter is Shared and has:\n    points: ConvergentCount.\n\n## Main\nLet mutable a be a new Counter.\nLet mutable b be a new Counter.\nIncrease a's points by 3.\nIncrease b's points by 5.\nMerge b into a.\nShow a's points.\n",
        DTier::NativeRun(&["8"]),
    ),
    (
        "converge", // CrdtJoin — the idempotent per-field-MAX lattice join (`Converge`); MAX(3,5)=5, MAX(10,2)=10.
        "## Definition\nA Vec has:\n    x: Int.\n    y: Int.\n## Main\nLet mutable a be a new Vec with x 3 and y 10.\nLet mutable b be a new Vec with x 5 and y 2.\nConverge b into a.\nShow a's x.\nShow a's y.\n",
        DTier::NativeRun(&["5", "10"]),
    ),
    (
        "crdt_append", // NewCrdt, CrdtAppend
        "## Definition\nA Doc is Shared and has:\n    lines: SharedSequence of Text.\n\n## Main\nLet mutable d be a new Doc.\nAppend \"L1\" to d's lines.\nAppend \"L2\" to d's lines.\nShow the length of d's lines.\n",
        DTier::NativeRun(&["2"]),
    ),
    (
        "crdt_resolve", // CrdtResolve
        "## Definition\nA WikiPage is Shared and has:\n    a title, which is Divergent Text.\n\n## Main\nLet mutable page be a new WikiPage.\nSet page's title to \"Draft\".\nShow page's title.\nResolve page's title to \"Final\".\nShow page's title.\n",
        DTier::NativeRun(&["Draft", "Final"]),
    ),
    (
        "check_policy", // CheckPolicy (predicate on a Bool field)
        "## Definition\nA User has:\n    a verified, which is Bool.\n\n## Policy\nA User is trusted if the user's verified equals true.\n\n## Main\nLet u be a new User with verified true.\nShow \"before\".\nCheck that the u is trusted.\nShow \"after\".\n",
        DTier::NativeRun(&["before", "after"]),
    ),
    // ── Loopback networking — native, build+run ==VM ──
    (
        "net_listen", // NetListen
        "## Main\nListen on \"/ip4/0.0.0.0/tcp/0\".\nShow 1.\n",
        DTier::NativeRun(&["1"]),
    ),
    (
        "sync_only", // CrdtBump, NetSync
        "## Definition\nA Tally has:\n    n: a Counter.\n## Main\n    Let mutable state be a new Tally.\n    Increase state's n by 3.\n    Sync state on \"topic\".\n    Show state's n.\n",
        DTier::NativeRun(&["3"]),
    ),
    (
        "peer_send_await", // NetListen, NetMakePeer, NetSend, NetAwait
        "## Main\n    Listen on \"me\".\n    Let peer be a PeerAgent at \"me\".\n    Send 42 to peer.\n    Await response from peer into got.\n    Show got.\n",
        DTier::NativeRun(&["42"]),
    ),
    (
        "net_stream", // NetListen, NetStream
        "## Main\n    Listen on \"me\".\n    Let items be [1, 2, 3].\n    Stream items to \"sink\".\n    Show 7.\n",
        DTier::NativeRun(&["7"]),
    ),
    // ── `Connect to` — native offline no-op; the sync VM oracle dials a real socket, so it cannot be run ──
    (
        "connect_sync", // NetConnect, NetListen, CrdtBump, NetSync
        "## A Counter is Shared and has:\n    A total: Int.\n\n## Main\n    Connect to \"ws://127.0.0.1:7799\".\n    Listen on \"me\".\n    Let mutable c be a new Counter with total 5.\n    Increase c's total by 3.\n    Sync c on \"room\".\n    Show c's total.\n",
        DTier::NativeCompileOnly,
    ),
    // ── Real-socket TCP client — native (over `C_TCP_RUNTIME`); the sync oracle dials a real socket, so it is
    //    compile-native here and ≡VM-witnessed end-to-end in `tcp_walker_lock`. ──
    (
        "tcp", // Tcp
        "## Main\n    Let r be tcpConnect(\"127.0.0.1\", 1).\n    Show 7.\n",
        DTier::NativeCompileOnly,
    ),
];

/// FULL PROPER COVERAGE — the op-level partition is TOTAL (29 = the ISA's whole async set), has no duplicates,
/// and EVERY op names a concrete witness. The 14 blocking ops are `Covered` ≡VM through the walker's own stackless
/// path; the 13 offline CRDT/networking/policy ops (incl. `CrdtJoin` via `Converge`) are `NativeRun` (the Op-walker
/// lowers them itself and the C build+runs ==VM); `NetConnect` AND `Tcp` are `NativeNoOracle` (native, but the sync
/// VM oracle dials a real socket so can't run offline — `Tcp`'s end-to-end ≡VM proof is `tcp_walker_lock`). ★ NOTHING
/// is `Delegated` (the gate is empty) and NOTHING is `NotEmittable` (`Converge` made CrdtJoin emittable). So ALL 29
/// async ops are source-emittable AND covered on the native walker — no delegation, no un-emittable holdout.
#[test]
fn the_concurrency_op_partition_is_total_and_witnessed() {
    // Total against the VM's async op set (kept in lockstep with `c_bytecode_lock`'s census).
    assert_eq!(
        CONCURRENCY_OPS.len(),
        29,
        "the concurrency op set is the ISA's 29 async ops"
    );
    let names: BTreeSet<&str> = CONCURRENCY_OPS.iter().map(|(n, _)| *n).collect();
    assert_eq!(names.len(), 29, "duplicate concurrency op in the partition");

    let count = |pred: fn(&OpCov) -> bool| CONCURRENCY_OPS.iter().filter(|(_, c)| pred(c)).count();
    let covered = count(|c| matches!(c, OpCov::Covered(_)));
    let native_run = count(|c| matches!(c, OpCov::NativeRun(_)));
    let native_no_oracle = count(|c| matches!(c, OpCov::NativeNoOracle(_)));
    let delegated = count(|c| matches!(c, OpCov::Delegated(_)));
    let not_emittable = count(|c| matches!(c, OpCov::NotEmittable(_)));
    assert_eq!(
        covered, 14,
        "the blocking-concurrency subset (14 ops) is Covered ≡VM on the stackless walker"
    );
    assert_eq!(native_run, 13, "the offline CRDT/networking/policy ops incl. CrdtJoin (13) are NativeRun (walker C build+runs ==VM)");
    assert_eq!(native_no_oracle, 2, "exactly two ops (NetConnect, Tcp) are NativeNoOracle (native, but the sync oracle dials a real socket so can't run offline)");
    assert_eq!(
        delegated, 0,
        "NOTHING delegates to AST-C anymore — the real-socket Tcp client now lowers natively too"
    );
    assert_eq!(
        not_emittable, 0,
        "NOTHING is un-emittable anymore — `Converge` made CrdtJoin source-emittable this campaign"
    );
    assert_eq!(
        covered + native_run + native_no_oracle + delegated + not_emittable,
        29,
        "the partition is total"
    );
    // ★ ENDGAME: EVERY one of the 29 async ops is now source-emittable AND covered — 0 delegated, 0 un-emittable.
    // The native Op-walker (stackless blocking + flat CRDT/net/policy/Tcp) covers all 29.
    assert_eq!(
        covered + native_run + native_no_oracle + delegated,
        29,
        "every async op is source-emittable and covered"
    );
    assert_eq!(
        covered + native_run + native_no_oracle,
        29,
        "the native walker now covers EVERY async op (no delegation, nothing un-emittable)"
    );

    // `Covered` ops name a Supported concurrency-CORPUS program (≡VM on the stackless walker).
    let supported: BTreeSet<&str> = CORPUS
        .iter()
        .filter(|(_, _, s)| *s == Status::Supported)
        .map(|(l, _, _)| *l)
        .collect();
    // The offline ops name an OFFLINE_CORPUS program; `NativeRun` witnesses must be `NativeRun`-tier.
    let offline: BTreeSet<&str> = OFFLINE_CORPUS.iter().map(|(l, _, _)| *l).collect();
    let offline_run: BTreeSet<&str> = OFFLINE_CORPUS
        .iter()
        .filter(|(_, _, t)| matches!(t, DTier::NativeRun(_)))
        .map(|(l, _, _)| *l)
        .collect();
    for (op, cov) in CONCURRENCY_OPS {
        match cov {
            OpCov::Covered(w) => assert!(
                supported.contains(w),
                "op `{op}` is Covered but witness `{w}` is not a Supported concurrency-CORPUS program"
            ),
            OpCov::NativeRun(w) => assert!(
                offline_run.contains(w),
                "op `{op}` is NativeRun but witness `{w}` is not a NativeRun-tier OFFLINE_CORPUS program"
            ),
            OpCov::NativeNoOracle(w) | OpCov::Delegated(w) => assert!(
                offline.contains(w),
                "op `{op}`'s witness `{w}` is not an OFFLINE_CORPUS program"
            ),
            OpCov::NotEmittable(_) => {}
        }
    }
    eprintln!("[conc-op-partition] 14 stackless-≡VM + 13 native-run-≡VM + 2 native-no-oracle + 0 delegated + 0 not-emittable = 29 async ops — ★ ALL 29 source-emittable + covered on the NATIVE walker; NOTHING delegates, NOTHING un-emittable (Tcp went native ≡VM in tcp_walker_lock; Converge made CrdtJoin emittable)");
}

// ============================================================
//  The OFFLINE subsystems — Net / CRDT / policy lower NATIVELY on the bytecode Op-walker
// ============================================================

/// Build the WALKER's C for an offline program (native Op-walker output is plain C → `gcc`; the stackless C++
/// marker would pick `g++`, but these never take that path), run it, return stdout lines. `None` on any failure.
fn walker_run_offline(src: &str, label: &str) -> Option<Vec<String>> {
    let c = compile_to_c_via_bytecode(src).ok()?;
    let (cc, ext) = if c.contains("long long &") {
        ("g++", "cpp")
    } else {
        ("gcc", "c")
    };
    build_run_best_of(&c, cc, ext, label, 1).map(|(lines, _)| lines)
}

/// The `NativeRun` offline programs (CRDT lattice + loopback networking + policy) build+run byte-for-byte ==VM
/// through the NATIVE Op-walker — the strong ≡VM witness for the 12 natively-lowered ops. No real sockets, so
/// CI-deterministic.
#[test]
fn offline_subsystem_ops_run_equiv_vm_on_the_native_walker() {
    if !have_gcc() {
        eprintln!("SKIP offline-subsystem execution: gcc not installed");
        return;
    }
    for (label, src, tier) in OFFLINE_CORPUS {
        let DTier::NativeRun(expected) = tier else {
            continue;
        };
        let want = interpret_for_ui_sync(src).lines;
        let got = walker_run_offline(src, label).unwrap_or_else(|| {
            panic!(
                "`{label}` is a NativeRun offline program but its walker C failed to compile/run"
            )
        });
        assert_eq!(
            got, want,
            "`{label}`: native walker C must equal the VM oracle"
        );
        let expect: Vec<String> = expected.iter().map(|s| s.to_string()).collect();
        assert_eq!(
            got, expect,
            "`{label}`: native walker C must equal the pinned expected output {expected:?}"
        );
    }
}

/// The natively-lowered offline programs are NOT delegated: their walker C is DIFFERENT from the shipping AST-C
/// (`compile_to_c`) — proving the Op-walker lowers them itself, not by handing off. The killer case: `sync_only`
/// runs to `3` on the native walker (==VM) but the AST-C tier mis-lowers it to `0` — so native lowering is not
/// just independent, it is strictly MORE correct here. Only `Tcp` remains byte-identical to AST-C.
#[test]
fn native_offline_ops_do_not_delegate() {
    for (label, src, tier) in OFFLINE_CORPUS {
        if matches!(tier, DTier::Delegate) {
            continue;
        }
        let walker = compile_to_c_via_bytecode(src)
            .unwrap_or_else(|e| panic!("`{label}`: native walker refused: {e:?}"));
        let astc = compile_to_c(src).unwrap_or_else(|e| panic!("`{label}`: AST-C refused: {e:?}"));
        assert_ne!(
            walker, astc,
            "`{label}`: the native walker must NOT be byte-identical to AST-C (it lowers the op itself)"
        );
    }
    // The killer: native == VM but AST-C != VM on `sync_only` (a `Tally.n: a Counter` where "Counter" is
    // undeclared — the native walker matches the VM's plain-int bump; AST-C drops it to 0).
    if have_gcc() {
        let src = OFFLINE_CORPUS
            .iter()
            .find(|(l, _, _)| *l == "sync_only")
            .unwrap()
            .1;
        let vm = interpret_for_ui_sync(src).lines;
        let native = walker_run_offline(src, "sync_only_native").expect("native compiles+runs");
        assert_eq!(native, vm, "native walker matches the VM on sync_only");
        assert_eq!(vm, vec!["3".to_string()], "the VM oracle shows 3");
        let (cc, ext) = ("gcc", "c");
        if let Ok(a) = compile_to_c(src) {
            if let Some((astc_out, _)) = build_run_best_of(&a, cc, ext, "sync_only_astc", 1) {
                assert_ne!(astc_out, vm, "AST-C mis-lowers sync_only (≠ VM) — the native walker is strictly more correct");
            }
        }
    }
}

/// NOTHING delegates anymore — the delegation gate is EMPTY. There is no `Delegate`-tier program in the
/// corpus, and the ONE-time holdout (the real-socket `Tcp` client) now lowers NATIVELY: its walker C is
/// DIFFERENT from the shipping AST-C tier (`compile_to_c`) AND emits the real socket runtime. (The genuine
/// end-to-end ≡VM proof — a TCP round-trip + a refused connect, VM async ≡ walker C — is `tcp_walker_lock`.)
#[test]
fn nothing_delegates_the_gate_is_empty() {
    // The corpus carries no `Delegate`-tier program — the gate is closed.
    assert!(
        !OFFLINE_CORPUS
            .iter()
            .any(|(_, _, t)| matches!(t, DTier::Delegate)),
        "no OFFLINE_CORPUS program may be `Delegate`-tier — the delegation gate is empty"
    );
    // The former holdout (`tcp`) lowers natively: distinct from AST-C, and carries the real socket runtime.
    let (label, src, _) = OFFLINE_CORPUS
        .iter()
        .find(|(l, _, _)| *l == "tcp")
        .expect("the tcp program is in the corpus");
    let walker = compile_to_c_via_bytecode(src)
        .unwrap_or_else(|e| panic!("`{label}`: walker refused the tcp program: {e:?}"));
    let astc =
        compile_to_c(src).unwrap_or_else(|e| panic!("`{label}`: shipping AST-C refused: {e:?}"));
    assert_ne!(walker, astc, "`{label}`: the walker's native Tcp C must NOT be byte-identical to AST-C (it lowers the op itself now)");
    assert!(
        walker.contains("logos_tcp_connect"),
        "`{label}`: the native walker must emit the real socket runtime"
    );
}

/// EVERY offline program compiles on the walker — none is refused. The native ones lower on the Op-walker, the
/// one delegated one hands to AST-C; either way `compile_to_c_via_bytecode` returns `Ok`.
#[test]
fn every_offline_program_compiles_on_the_walker() {
    for (label, src, _tier) in OFFLINE_CORPUS {
        assert!(
            compile_to_c_via_bytecode(src).is_ok(),
            "`{label}`: the walker must compile the offline program (native or delegated), never refuse it"
        );
    }
}

/// The native offline lowering is NARROW — a pure blocking-concurrency program still stays on the walker's
/// stackless path (its C carries the stackless scheduler and is NOT the AST-C ucontext C), and a purely
/// sequential program stays on the flat op-walker. This guards the routing from over-reaching.
#[test]
fn delegation_is_narrow_only_net_crdt_policy_hand_off() {
    // A real Supported blocking-concurrency program → stackless walker, distinct from AST-C's ucontext lowering.
    let conc = CORPUS
        .iter()
        .find(|(_, _, s)| *s == Status::Supported)
        .map(|(_, src, _)| *src)
        .expect("the concurrency CORPUS has at least one Supported program");
    let w = compile_to_c_via_bytecode(conc)
        .expect("a Supported concurrency program compiles on the walker");
    assert!(
        w.contains("LOGOS_SCHED_POLICY") || w.contains("logos_stackless_main"),
        "a concurrency program must stay on the stackless walker (carry its scheduler), not delegate"
    );
    let a = compile_to_c(conc).expect("the concurrency program compiles on AST-C too");
    assert_ne!(
        w, a,
        "a concurrency program must NOT be byte-identical to AST-C (it uses the stackless path)"
    );
    // A sequential program → flat op-walker, distinct from AST-C's structured codegen.
    let seq = "## Main\nLet x be 40.\nShow x + 2.\n";
    let w = compile_to_c_via_bytecode(seq).expect("sequential program compiles on the op-walker");
    let a = compile_to_c(seq).expect("sequential program compiles on AST-C");
    assert_ne!(
        w, a,
        "a sequential program must stay on the flat op-walker, not delegate to AST-C"
    );
}
