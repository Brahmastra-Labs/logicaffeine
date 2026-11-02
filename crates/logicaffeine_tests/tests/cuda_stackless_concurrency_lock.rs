//! ★ CUDA BLOCKING CONCURRENCY — PIT OF SUCCESS ★ (RED-first, TDD).
//!
//! `cuda_concurrency_lock` holds the programs that never block. This lock holds the ones where a task TRULY
//! BLOCKS mid-body (recv before send, a helper that waits, `Await` a task, `Sleep`, `Select`-wait) and must
//! SUSPEND and RESUME — which a GPU cannot do with `ucontext`. The device-portable answer is the stepped
//! tier: each task body becomes a resumable `switch(pc)` over its scheduling points, every binding a field of
//! a heap frame, driven by the C family's scheduler core on the device (park on a pipe or a timer, wake on a
//! send, a close or a tick).
//!
//! Discipline: the concurrent oracle pair (`interpret_for_ui_sync` + `run_vm_concurrent`) is ground truth
//! (they share the deterministic scheduler and must AGREE — every program here is determinate). A program
//! classified `Supported` MUST compile to CUDA, compile with `nvcc`, run ON THE GPU, and print byte-for-
//! byte the oracle. A `Gap` is a shrink-only frontier. RED tests are the spec — fix `codegen_cuda`/
//! `codegen_c` (the stepped tier), NEVER weaken this file. GPU execution is gated on `nvcc` +
//! `/dev/nvidia0`; the ratchet + oracle-agreement always run.

use std::collections::BTreeSet;

use logicaffeine_compile::compile::compile_to_cuda;
use logicaffeine_compile::{interpret_for_ui_sync, run_vm_concurrent};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Status {
    /// Lowered through the stepped tier + proven ≡ the deterministic oracle ON THE GPU.
    Supported,
    /// Refused or diverging on the device — a gap.
    Gap,
}

/// The determinate BLOCKING corpus, simplest first. Every task here suspends at a real blocking point.
const CORPUS: &[(&str, &str, Status)] = &[
    (
        // Consumer runs first (spawn-to-front), recv on an empty channel → BLOCKS; main then sends 5 →
        // consumer wakes, x=5, Show 5. The single simplest true-suspend program. Oracle: `5`.
        "recv_before_send",
        "## To consume (ch: Int):\n    Receive x from ch.\n    Show x.\n\n## Main\n    \
         Let ch be a Pipe of Int.\n    Launch a task to consume with ch.\n    Send 5 into ch.\n",
        Status::Supported,
    ),
    (
        // Two blocking recvs interleaved with the producer's sends — the consumer suspends twice.
        "two_blocking_recvs",
        "## To consume (ch: Int):\n    Receive a from ch.\n    Show a.\n    Receive b from ch.\n    Show b.\n\n\
         ## Main\n    Let ch be a Pipe of Int.\n    Launch a task to consume with ch.\n    \
         Send 1 into ch.\n    Send 2 into ch.\n",
        Status::Supported,
    ),
    (
        // Virtual-time block: main suspends on the timer at `Sleep`, the scheduler advances the logical
        // clock (no other ready task), then resumes main. Exercises the timer-park/resume path. Oracle:
        // `1`,`2`.
        "sleep_resume",
        "## Main\n    Show 1.\n    Sleep 5.\n    Show 2.\n",
        Status::Supported,
    ),
    // ── The NON-LINEAR frontier: a blocking op inside a LOOP / CONDITIONAL (control flow crossing a
    // yield). The linear state machine is not enough — the resume point sits INSIDE a `while`/`if`, so the
    // transform must place the resume label within the emitted control structure (Duff's-device async),
    // with the loop/branch state living in the frame. ──────────────────────────────────────────────────
    (
        // A worker loop that blocks on `Receive` EVERY iteration → the resume label is inside the `while`.
        "loop_recv",
        "## To consume (ch: Int):\n    Let i be 0.\n    While i is less than 3:\n        \
         Receive x from ch.\n        Show x.\n        Set i to i + 1.\n\n## Main\n    \
         Let ch be a Pipe of Int.\n    Launch a task to consume with ch.\n    \
         Send 10 into ch.\n    Send 20 into ch.\n    Send 30 into ch.\n",
        Status::Supported,
    ),
    (
        // Accumulate across a blocking loop — the loop carries frame state (`total`,`n`) across yields.
        "loop_sum",
        "## To sum_until (ch: Int):\n    Let total be 0.\n    Let n be 0.\n    While n is less than 3:\n        \
         Receive v from ch.\n        Set total to total + v.\n        Set n to n + 1.\n    Show total.\n\n\
         ## Main\n    Let ch be a Pipe of Int.\n    Launch a task to sum_until with ch.\n    \
         Send 5 into ch.\n    Send 7 into ch.\n    Send 9 into ch.\n",
        Status::Supported,
    ),
    (
        // A blocking `Receive` inside an `If` branch — the resume label sits inside the conditional.
        "cond_recv",
        "## To consume (ch: Int):\n    Receive x from ch.\n    If x is greater than 5:\n        \
         Receive y from ch.\n        Show y.\n\n## Main\n    Let ch be a Pipe of Int.\n    \
         Launch a task to consume with ch.\n    Send 10 into ch.\n    Send 20 into ch.\n",
        Status::Supported,
    ),
    // ── The NESTED-CALL frontier: a task calls a HELPER that itself blocks. When the helper suspends,
    // the WHOLE call chain must suspend and resume — the stackless equivalent of a native call stack. The
    // transform needs an explicit per-task FRAME STACK: `Call` pushes a callee frame, `Return` pops it and
    // delivers the result to the caller's resume state. ─────────────────────────────────────────────────
    (
        // consume calls `get(ch)`; `get` blocks on `Receive` → the block propagates up through the call.
        "nested_call",
        "## To get (ch: Int) -> Int:\n    Receive x from ch.\n    Return x.\n\n\
         ## To consume (ch: Int):\n    Let y be get(ch).\n    Show y.\n\n## Main\n    \
         Let ch be a Pipe of Int.\n    Launch a task to consume with ch.\n    Send 42 into ch.\n",
        Status::Supported,
    ),
    (
        // The blocking helper is called TWICE — two push/block/pop cycles on the frame stack.
        "nested_twice",
        "## To get (ch: Int) -> Int:\n    Receive x from ch.\n    Return x.\n\n\
         ## To consume (ch: Int):\n    Let a be get(ch).\n    Let b be get(ch).\n    Show a + b.\n\n\
         ## Main\n    Let ch be a Pipe of Int.\n    Launch a task to consume with ch.\n    \
         Send 3 into ch.\n    Send 4 into ch.\n",
        Status::Supported,
    ),
    // ── The SELECT-WAIT frontier: `Await the first of:` blocks on MULTIPLE arms (recv arms + timeout
    // arms), waking with the WINNING arm. The transform registers all arms, parks on every channel + the
    // timers, and dispatches multi-way on the winner. Determinate: exactly one arm ready. ───────────────
    (
        // The producer (spawn-to-front) sends 9 before Main selects, so the recv arm is READY and wins
        // immediately (no park) → `9`.
        "select_recv_ready",
        "## To produce (ch: Int):\n    Send 9 into ch.\n\n## Main\n    Let ch be a Pipe of Int.\n    \
         Launch a task to produce with ch.\n    Await the first of:\n        Receive v from ch:\n            \
         Show v.\n        After 100 seconds:\n            Show 0.\n",
        Status::Supported,
    ),
    (
        // No producer — the recv arm can never fire, so Main PARKS on the select and the TIMEOUT arm wins
        // when the logical clock reaches it → `42`. A genuinely-blocking select (park + virtual time).
        "select_timeout_wins",
        "## Main\n    Let ch be a Pipe of Int.\n    Await the first of:\n        Receive v from ch:\n            \
         Show v.\n        After 5 seconds:\n            Show 42.\n",
        Status::Supported,
    ),
    // ── The NON-INT PAYLOAD frontier: a channel carries a Real/Text/Bool value across a yield. The 64-bit
    // frame slot + channel cell already fit a double's BITS or a `char*`; the transform must thread the
    // channel's ELEMENT TYPE — bit-reinterpret a float on send/recv (like the VM's `f64::to_bits`), carry a
    // `char*` for text — and `Show` with the right renderer (`show_f64`/`show_str`, not `show_i64`). ──────
    (
        // A `Pipe of Real` carries `1.5` across a blocking recv → the consumer prints `1.5` (NOT truncated
        // to `1`). The VM bit-reinterprets the double; the device pipe must too.
        "float_chan",
        "## To consume (ch: Real):\n    Receive x from ch.\n    Show x.\n\n## Main\n    \
         Let ch be a Pipe of Real.\n    Launch a task to consume with ch.\n    Send 1.5 into ch.\n",
        Status::Supported,
    ),
    (
        // A `Pipe of Text` carries a `char*` across a blocking recv → the consumer prints `hello` (NOT the
        // pointer as an integer). The device string persists on the heap across the yield.
        "text_chan",
        "## To consume (ch: Text):\n    Receive x from ch.\n    Show x.\n\n## Main\n    \
         Let ch be a Pipe of Text.\n    Launch a task to consume with ch.\n    Send \"hello\" into ch.\n",
        Status::Supported,
    ),
    // ── The RECURSION frontier: a BLOCKING helper that calls ITSELF. The explicit frame stack already
    // handles arbitrary depth (each recursion pushes a heap frame, popped on return) — the transform must
    // stop conservatively refusing the direct-recursion cycle for a bounded, terminating recursion. ──────
    (
        // `drain` recurses, blocking on `Receive` at each level, until it receives 0 → `2`,`1`.
        "blocking_recursion",
        "## To drain (ch: Int):\n    Receive x from ch.\n    If x is greater than 0:\n        \
         Show x.\n        drain(ch).\n\n## Main\n    Let ch be a Pipe of Int.\n    \
         Launch a task to drain with ch.\n    Send 2 into ch.\n    Send 1 into ch.\n    Send 0 into ch.\n",
        Status::Supported,
    ),
    // ── The ABORT frontier (Op::TaskAbort): `Stop h` cancels a spawned task. `Let h be Launch …` captures the
    // child task pointer into a frame cell; `Stop h` marks it aborted so the scheduler skips it (lazy teardown
    // ≡ the VM's `VmBlock::Abort`) — its post-`Receive` `Show 7` never runs. ────────────────────────────────
    (
        // blocker parks on recv; main aborts it, then ends → only main's `1` prints (blocker never resumes).
        "abort_blocked",
        "## To blocker (ch: Int):\n    Receive x from ch.\n    Show 7.\n\n## Main\n    Let ch be a Pipe of Int.\n    \
         Let h be Launch a task to blocker with ch.\n    Stop h.\n    Show 1.\n",
        Status::Supported,
    ),
    (
        // THE DISCRIMINATOR: abort the blocked task, THEN send to its channel. A faithful abort DROPS the
        // aborted receiver, so the send buffers and `7` never prints → `1`. A no-op abort would wake it → `7`.
        "abort_then_send",
        "## To blocker (ch: Int):\n    Receive x from ch.\n    Show 7.\n\n## Main\n    Let ch be a Pipe of Int.\n    \
         Let h be Launch a task to blocker with ch.\n    Stop h.\n    Send 9 into ch.\n    Show 1.\n",
        Status::Supported,
    ),
    // ── The STEPPED frontier: shapes the narrow int-scalar transform refused, which the stepped tier — the
    // C tier's own lowering under the scheduler core — runs as it runs everything else. ──────────────────
    (
        // Both receive arms are ready when Main selects; the scheduler's chooser breaks the tie by the seed,
        // as the runtime does (seed 0 → the second arm) → `2`.
        "select_two_ready_recv_arms",
        "## To p (a: Int):\n    Send 1 into a.\n\n## To q (b: Int):\n    Send 2 into b.\n\n## Main\n    \
         Let a be a Pipe of Int.\n    Let b be a Pipe of Int.\n    Launch a task to p with a.\n    \
         Launch a task to q with b.\n    Await the first of:\n        Receive x from a:\n            Show x.\n        \
         Receive y from b:\n            Show y.\n",
        Status::Supported,
    ),
    (
        // Two functions that wait, recursing through each other until a received 0 → `2`,`1`.
        "mutual_recursion",
        "## To ping (ch: Int):\n    Receive x from ch.\n    If x is greater than 0:\n        Show x.\n        pong(ch).\n\n\
         ## To pong (ch: Int):\n    Receive y from ch.\n    If y is greater than 0:\n        Show y.\n        ping(ch).\n\n\
         ## Main\n    Let ch be a Pipe of Int.\n    Launch a task to ping with ch.\n    Send 2 into ch.\n    \
         Send 1 into ch.\n    Send 0 into ch.\n",
        Status::Supported,
    ),
    (
        // Main waits for a task; a task answers `nothing` → `nothing`.
        "await_a_task",
        "## To worker -> Int:\n    Return 5.\n\n## Main\n    Let h be Launch a task to worker.\n    Await h into r.\n    \
         Show r.\n",
        Status::Supported,
    ),
    (
        // A pure helper called after a wait stays an ordinary device function → `10`.
        "pure_helper_after_a_wait",
        "## To double (n: Int) -> Int:\n    Return n + n.\n\n## To consume (ch: Int):\n    Receive x from ch.\n    \
         Let y be double(x).\n    Show y.\n\n## Main\n    Let ch be a Pipe of Int.\n    \
         Launch a task to consume with ch.\n    Send 5 into ch.\n",
        Status::Supported,
    ),
    (
        // A received Real does Real arithmetic → `2.5`.
        "real_arithmetic_after_a_wait",
        "## To consume (ch: Real):\n    Receive x from ch.\n    Show x + 1.\n\n## Main\n    \
         Let ch be a Pipe of Real.\n    Launch a task to consume with ch.\n    Send 1.5 into ch.\n",
        Status::Supported,
    ),
    (
        // A select's timeout arm shows text → `gone`.
        "select_arm_shows_text",
        "## Main\n    Let ch be a Pipe of Int.\n    Await the first of:\n        Receive v from ch:\n            \
         Show v.\n        After 5 seconds:\n            Show \"gone\".\n",
        Status::Supported,
    ),
];

const SUPPORTED_FLOOR: usize = 21;

/// The EXACT set of currently-`Gap` program labels — a shrink-only ledger. EMPTY: every blocking shape here
/// runs ≡VM on the GPU through the stepped tier — linear and non-linear waits (inside loops and branches), a
/// helper that waits (called once, twice, recursively, or through another that waits), selects (a ready
/// arm, a timeout, two ready arms broken by the seed, an arm showing text), Real/Text/Bool pipes (a Real by
/// its bits, a Text by its pointer), `Stop`, and `Await` of a task. What the device refuses is what the C tier
/// refuses (a record crossing a pipe, in `cuda_concurrency_lock`), never a wrong GPU answer.
const KNOWN_GAPS: &[&str] = &[];

fn have_nvcc_gpu() -> bool {
    std::path::Path::new("/dev/nvidia0").exists()
        && std::process::Command::new("nvcc")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
}

fn nvcc_run(cu: &str, label: &str) -> Option<Vec<String>> {
    let dir = std::env::temp_dir().join(format!("logos_cuda_sl_{}_{}", std::process::id(), label));
    let _ = std::fs::create_dir_all(&dir);
    let src = dir.join("k.cu");
    let bin = dir.join("k");
    std::fs::write(&src, cu).ok()?;
    let c = std::process::Command::new("nvcc")
        .args(["-O2", "-arch=sm_86", "-o"])
        .arg(&bin)
        .arg(&src)
        .output()
        .ok()?;
    if !c.status.success() {
        return None;
    }
    let r = std::process::Command::new(&bin).output().ok()?;
    let _ = std::fs::remove_dir_all(&dir);
    if !r.status.success() {
        return None;
    }
    Some(
        String::from_utf8_lossy(&r.stdout)
            .lines()
            .map(|s| s.to_string())
            .collect(),
    )
}

#[test]
fn the_concurrent_oracles_agree_on_every_program() {
    for (label, src, _) in CORPUS {
        let tw = interpret_for_ui_sync(src);
        let vm = run_vm_concurrent(src);
        assert_eq!(
            (&tw.lines, tw.error.is_some()),
            (&vm.lines, vm.error.is_some()),
            "concurrent oracle divergence on `{label}` — not determinate: tw={:?} vm={:?}",
            tw.lines,
            vm.lines,
        );
    }
}

#[test]
fn the_gap_ledger_is_pinned_and_shrink_only() {
    let live: BTreeSet<&str> = CORPUS
        .iter()
        .filter(|(_, _, s)| *s == Status::Gap)
        .map(|(l, _, _)| *l)
        .collect();
    let declared: BTreeSet<&str> = KNOWN_GAPS.iter().copied().collect();
    assert_eq!(
        declared.len(),
        KNOWN_GAPS.len(),
        "KNOWN_GAPS has duplicates"
    );
    assert!(
        live.difference(&declared).next().is_none(),
        "NEW blocking gap not in KNOWN_GAPS: {:?}",
        live.difference(&declared).collect::<Vec<_>>()
    );
    assert!(
        declared.difference(&live).next().is_none(),
        "KNOWN_GAPS lists a now-Supported program: {:?} — remove it, raise the floor",
        declared.difference(&live).collect::<Vec<_>>()
    );
}

#[test]
fn the_supported_count_never_regresses() {
    let n = CORPUS
        .iter()
        .filter(|(_, _, s)| *s == Status::Supported)
        .count();
    assert!(
        n >= SUPPORTED_FLOOR,
        "CUDA blocking-program supported count fell to {n} (floor {SUPPORTED_FLOOR})"
    );
}

#[test]
fn every_supported_blocking_program_runs_equiv_vm_on_the_gpu() {
    if !have_nvcc_gpu() {
        eprintln!("SKIP cuda blocking-program execution: needs nvcc + /dev/nvidia0");
        return;
    }
    for (label, src, status) in CORPUS {
        if *status != Status::Supported {
            continue;
        }
        let want = interpret_for_ui_sync(src).lines;
        let art = compile_to_cuda(src)
            .unwrap_or_else(|e| panic!("`{label}` is Supported but CUDA refused it: {e:?}"));
        let got = nvcc_run(&art.kernel_cu, label).unwrap_or_else(|| {
            panic!("`{label}` is Supported but its .cu failed nvcc/run — fix the device path")
        });
        assert_eq!(
            got, want,
            "`{label}`: CUDA output must equal the deterministic oracle"
        );
    }
}
