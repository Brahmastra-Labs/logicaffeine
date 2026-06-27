//! Phase 8 (FINISH_INTERPRETER.md) — the AOT dual-mode performance contract.
//!
//! Two guarantees:
//!   1. **Mode A (default, free-running multicore)** stays byte-identical to
//!      today's emission, and its hot path is provably untouched by all the
//!      concurrency work — `aot_default_codegen_*` snapshot it.
//!   2. **The determinate concurrency fragment** (Kahn process networks: launch +
//!      FIFO pipes, no `Select`) compiles to a program whose output equals the
//!      interpreter's exactly, with NO seed — determinacy is a theorem, not a
//!      coincidence (`aot_determinate_equals_interpreted`).
//!
//! The seeded Mode-B equivalence (`aot_nondeterminate_same_seed_equivalence`) and
//! the refinement smoke (`aot_production_output_in_allowed_set`) live alongside
//! once the seeded harness lands.

mod common;

use common::{
    assert_compiled_equals_interpreted, assert_compiled_equals_interpreted_seeded, compile_to_rust,
};

// ─── The determinate corpus: launch + FIFO pipe, no Select ───────────────────
//
// Every one of these is a Kahn process network. Output is independent of the
// schedule, so the free-running Mode-A binary and the interpreter MUST agree
// with no seed.

const DETERMINATE_CORPUS: &[(&str, &str)] = &[
    (
        "producer_consumer",
        "## To produce (ch: Int):\n\
        \x20   Send 1 into ch.\n\
        \x20   Send 2 into ch.\n\
        \x20   Send 3 into ch.\n\
        \n\
        ## Main\n\
        \x20   Let jobs be a Pipe of Int.\n\
        \x20   Launch a task to produce with jobs.\n\
        \x20   Receive a from jobs.\n\
        \x20   Receive b from jobs.\n\
        \x20   Receive c from jobs.\n\
        \x20   Show a.\n\
        \x20   Show b.\n\
        \x20   Show c.\n",
    ),
    (
        // Two producers fan IN to one consumer that receives both and shows their
        // sum — order-independent (`+` is commutative) AND single-printer, so the
        // observable output is schedule-invariant: 30, full stop.
        "fan_in_sum",
        "## To emit (ch: Int, n: Int):\n\
        \x20   Send n into ch.\n\
        \n\
        ## Main\n\
        \x20   Let ch be a Pipe of Int.\n\
        \x20   Launch a task to emit with ch and 10.\n\
        \x20   Launch a task to emit with ch and 20.\n\
        \x20   Receive a from ch.\n\
        \x20   Receive b from ch.\n\
        \x20   Let total be a + b.\n\
        \x20   Show total.\n",
    ),
    (
        "pass_argument_to_task",
        "## To emit (ch: Int, n: Int):\n\
        \x20   Send n into ch.\n\
        \n\
        ## Main\n\
        \x20   Let ch be a Pipe of Int.\n\
        \x20   Launch a task to emit with ch and 42.\n\
        \x20   Receive got from ch.\n\
        \x20   Show got.\n",
    ),
];

#[test]
fn aot_determinate_equals_interpreted() {
    for (name, src) in DETERMINATE_CORPUS {
        // assert_compiled_equals_interpreted panics with full diagnostics on any
        // divergence; the name is for which-case attribution if it does.
        eprintln!("aot_determinate_equals_interpreted: {name}");
        assert_compiled_equals_interpreted(src);
    }
}

// ─── Hot-path guard: non-concurrent codegen carries NO concurrency machinery ──

/// A representative non-concurrent compute program (a hot loop-sum, the shape of
/// the integer benchmarks). Its emitted Rust must never acquire any concurrency
/// runtime just because the concurrency feature exists.
const NON_CONCURRENT_BENCHMARK: &str = "## Main\n\
    Let sum be 0.\n\
    Repeat for i from 1 to 100:\n\
    \x20   Set sum to sum + i.\n\
    Show sum.\n";

#[test]
fn aot_default_codegen_for_benchmark_is_byte_identical() {
    // The hot path must be untouched by the concurrency work: a non-concurrent
    // program emits a plain synchronous `main` with NONE of the async/scheduler
    // machinery. This is the structural tripwire that fails the moment the
    // `requires_async` gate (or any concurrency lowering) leaks into ordinary
    // programs.
    let rust = compile_to_rust(NON_CONCURRENT_BENCHMARK).expect("compiles");
    for forbidden in [
        "#[tokio::main]",
        "tokio::",
        "logos_select",
        "logicaffeine_system::concurrency",
        "Pipe::",
        "LOGOS_SEED",
        "bootstrap",
    ] {
        assert!(
            !rust.contains(forbidden),
            "non-concurrent hot-path codegen leaked `{forbidden}`:\n{rust}"
        );
    }
    // And it is a plain threaded `_logos_main`, exactly as today.
    assert!(rust.contains("fn _logos_main()"), "expected the synchronous main shape:\n{rust}");
    assert!(!rust.contains("async fn main()"), "non-concurrent program must not be async:\n{rust}");
}

#[test]
fn aot_default_codegen_for_concurrent_program_unchanged_modeA() {
    // A concurrent program in the DEFAULT (Mode A) compile emits today's
    // tokio-based shape: an async main and the platform concurrency runtime, and
    // NO seeded/deterministic machinery (that is opt-in Mode B only).
    let src = DETERMINATE_CORPUS[0].1;
    let rust = compile_to_rust(src).expect("compiles");
    assert!(rust.contains("async fn main()"), "Mode A concurrent main is async:\n{rust}");
    assert!(
        rust.contains("tokio::") || rust.contains("logicaffeine_system::concurrency"),
        "Mode A concurrent program uses the tokio platform runtime:\n{rust}"
    );
    for forbidden in ["seeded_pick", "logos_select_seeded", "new_current_thread"] {
        assert!(
            !rust.contains(forbidden),
            "default (Mode A) emission must not carry Mode-B seeded machinery `{forbidden}`:\n{rust}"
        );
    }
}

const SELECT_BOTH_READY: &str = "## Main\n\
    Let a be a Pipe of Int.\n\
    Let b be a Pipe of Int.\n\
    Send 1 into a.\n\
    Send 2 into b.\n\
    Await the first of:\n\
    \x20   Receive x from a:\n\
    \x20       Show x.\n\
    \x20   Receive y from b:\n\
    \x20       Show y.\n";

#[test]
fn aot_mode_b_select_emits_seeded_pick_mode_a_does_not() {
    // Mode B lowers `Select` to the seeded winner-pick (sharing the interpreter's
    // choice function); Mode A keeps the raw `tokio::select!`. This is the ONLY
    // emission difference between the two modes.
    let mode_a = logicaffeine_compile::compile_to_rust(SELECT_BOTH_READY).expect("compiles");
    assert!(mode_a.contains("tokio::select!"), "Mode A keeps raw tokio::select!:\n{mode_a}");
    assert!(
        !mode_a.contains("seeded_pick"),
        "Mode A must NOT carry the seeded pick:\n{mode_a}"
    );

    let mode_b =
        logicaffeine_compile::compile_to_rust_deterministic(SELECT_BOTH_READY).expect("compiles");
    assert!(
        mode_b.contains("logicaffeine_system::concurrency::seeded_pick"),
        "Mode B lowers Select via the shared seeded chooser:\n{mode_b}"
    );
    assert!(mode_b.contains("__logos_ready"), "Mode B collects ready arms:\n{mode_b}");
}

#[test]
fn aot_nondeterminate_same_seed_equivalence() {
    // The nondeterminate fragment: a `Select` over two simultaneously-ready arms.
    // The winner is the seed's choice, and the Mode-B compiled binary shares the
    // interpreter's SplitMix64 choice function — so at EVERY seed the compiled
    // output is byte-identical to the interpreter's seeded output. (Single-task
    // program: the select winner is the first RNG draw, so a fresh seeded chooser
    // matches exactly.)
    for seed in [0u64, 1, 2, 7, 42] {
        assert_compiled_equals_interpreted_seeded(SELECT_BOTH_READY, seed);
    }
}

#[test]
fn aot_production_output_in_allowed_set() {
    // Mode A (default, free-running multicore — NO seed) is a refinement of the
    // interpreter: whatever interleaving the OS scheduler produces, the output
    // must be one the interpreter can also produce at SOME seed. Compute the
    // interpreter's allowed-set by sweeping seeds, then assert the free-running
    // compiled output lands inside it.
    use logicaffeine_compile::run_treewalker_concurrent_seeded;
    use std::collections::BTreeSet;

    let allowed: BTreeSet<String> = (0..32u64)
        .map(|s| run_treewalker_concurrent_seeded(SELECT_BOTH_READY, s).lines.join("\n"))
        .collect();
    // The select races two ready arms, so the interpreter must exhibit BOTH
    // winners across the sweep — otherwise the "allowed set" is trivial.
    assert_eq!(
        allowed,
        BTreeSet::from(["1".to_string(), "2".to_string()]),
        "interpreter seed-sweep allowed-set should be exactly {{1, 2}}, got {allowed:?}"
    );

    let produced = common::run_logos(SELECT_BOTH_READY);
    assert!(produced.success, "Mode A program runs:\n{}", produced.stderr);
    let out = produced.stdout.trim().to_string();
    assert!(
        allowed.contains(&out),
        "free-running Mode A output {out:?} is not in the interpreter allowed-set {allowed:?}"
    );
}
