//! Partial-evaluator soundness across concurrency / networking boundaries.
//!
//! A concurrency construct (`Select`, `LaunchTask`, channels, networking) is an opaque,
//! often nondeterministic, side-effecting boundary. The specializer must never fold a
//! static argument *across* it: doing so drops the parameter from the signature while the
//! substitution never reaches inside the concurrency statement (its `substitute_stmt` arm
//! is the verbatim catch-all), leaving a dangling free variable in the residual — a
//! miscompile. These tests pin the specialization gate. They are the RED reproducers for
//! the `effects.rs` mis-classification (concurrency was `Pure`) and the `partial_eval.rs`
//! gate (it only consulted `io`).

mod common;

use common::{compile_to_rust, run_logos};

/// A function that *launches a task passing a static-candidate argument* must not be
/// specialized: the spawn is a concurrency boundary the substitution cannot enter.
#[test]
fn pe_function_with_launch_task_not_specialized() {
    let source = r#"## To noop (n: Int):
    Return.

## To worker (flag: Int, val: Int) -> Int:
    Let total be 0.
    If flag is greater than 100:
        Set total to total + 1.
        Set total to total + 1.
        Set total to total + 1.
    Launch a task to noop with flag.
    Return total.

## Main
    Let v be 5.
    Let r be worker(1, v).
    Show r.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        !rust.contains("worker_s0"),
        "worker contains a LaunchTask — a concurrency boundary — and must NOT be \
         specialized (specializing drops `flag` while leaving it referenced inside the \
         spawn).\nGot:\n{}",
        rust
    );
}

/// The same boundary discipline for `Select` (a nondeterministic choice). A static
/// parameter used in a timeout / branch body must keep the function un-specialized.
#[test]
fn pe_function_with_select_is_not_specialized() {
    let source = r#"## To worker (flag: Int, ch: Int) -> Int:
    Let total be 0.
    If flag is greater than 100:
        Set total to total + 1.
        Set total to total + 1.
        Set total to total + 1.
    Await the first of:
        Receive x from ch:
            Set total to total + x.
        After flag seconds:
            Set total to total + flag.
    Return total.

## Main
    Let ch be a Pipe of Int.
    Send 7 into ch.
    Let r be worker(1, ch).
    Show r.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        !rust.contains("worker_s0"),
        "worker contains a Select (nondeterministic) and must NOT be specialized.\nGot:\n{}",
        rust
    );
}

/// The miscompile reproducer end-to-end: a static argument flows into a `Select` timeout
/// branch. With the bug the specialized body (`worker_s0`) references the dropped `flag`,
/// so the generated Rust fails to build (the structural witness is
/// `pe_function_with_select_is_not_specialized`). The gate must refuse the specialization
/// so the program compiles and runs. A timeout-only `Select` keeps the program free of the
/// unrelated channel-param codegen limitation.
#[test]
fn pe_no_param_drop_dangling_ref() {
    let source = r#"## To worker (flag: Int, base: Int) -> Int:
    Let total be base.
    If flag is greater than 100:
        Set total to total + 1.
        Set total to total + 1.
        Set total to total + 1.
    Await the first of:
        After flag seconds:
            Set total to total + flag.
    Return total.

## Main
    Let b be 5.
    Let r be worker(1, b).
    Show r.
"#;
    let result = run_logos(source);
    assert!(
        result.success,
        "specializing across the Select left a dangling `flag` and broke the build.\n\
         Generated:\n{}\nstderr: {}",
        result.rust_code, result.stderr
    );
    assert!(
        result.stdout.contains('6'),
        "worker(1, 5) should sleep then return 5 + 1 = 6.\nstdout: {}",
        result.stdout
    );
}

/// Regression guard: partial evaluation must not change the observable behavior of a
/// genuinely concurrent program.
#[test]
fn pe_concurrent_program_output_unchanged() {
    let source = r#"## To produce (ch: Int):
    Send 1 into ch.
    Send 2 into ch.

## Main
    Let jobs be a Pipe of Int.
    Launch a task to produce with jobs.
    Receive first from jobs.
    Receive second from jobs.
    Show first.
    Show second.
"#;
    let result = run_logos(source);
    assert!(
        result.success,
        "concurrent program must still compile and run after PE.\nGenerated:\n{}\nstderr: {}",
        result.rust_code, result.stderr
    );
    assert!(result.stdout.contains('1'), "expected 1 in output: {}", result.stdout);
    assert!(result.stdout.contains('2'), "expected 2 in output: {}", result.stdout);
}

// =============================================================================
// Deep specialization: pure sub-computations *inside* a concurrency boundary are
// still specialized. The boundary statement itself is never folded across (the gate
// above), but the partial evaluator descends into Concurrent/Parallel/Select bodies and
// the expression arguments of concurrency statements to specialize the pure calls within
// — without reordering or duplicating any effect.
// =============================================================================

/// A pure call inside an `Attempt all of the following:` (Concurrent) block is specialized.
#[test]
fn pe_specializes_pure_call_inside_concurrent_block() {
    let source = r#"## To native parseInt (s: Text) -> Int

## To scale (factor: Int, x: Int) -> Int:
    Return factor * x.

## Main
    Let n be parseInt("7").
    Attempt all of the following:
        Show scale(3, n).
        Show scale(4, n).
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains("scale_s0"),
        "scale(3, n) inside the Concurrent block is a pure call and should be specialized.\nGot:\n{}",
        rust
    );
    let result = run_logos(source);
    assert!(result.success, "must still run.\nstderr: {}", result.stderr);
    assert!(result.stdout.contains("21"), "scale(3,7)=21 expected: {}", result.stdout);
    assert!(result.stdout.contains("28"), "scale(4,7)=28 expected: {}", result.stdout);
}

/// A pure call inside a `Select` branch body is specialized.
#[test]
fn pe_specializes_pure_call_inside_select_branch() {
    let source = r#"## To native parseInt (s: Text) -> Int

## To scale (factor: Int, x: Int) -> Int:
    Return factor * x.

## Main
    Let n be parseInt("7").
    Let ch be a Pipe of Int.
    Send 1 into ch.
    Await the first of:
        Receive v from ch:
            Show scale(3, n).
        After 1 seconds:
            Show scale(4, n).
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains("scale_s0"),
        "scale(3, n) inside the Select branch is a pure call and should be specialized.\nGot:\n{}",
        rust
    );
}
