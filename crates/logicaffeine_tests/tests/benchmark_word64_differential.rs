//! Word64 same-semantics benchmark variants — the fair apples-to-apples integer track.
//!
//! Each integer-arithmetic benchmark ships a `main_word64.lg` alongside `main.lg`:
//! the identical algorithm, but the hot-loop integers are `Word64` (the wrapping
//! ring, native `u64`) so there is NO overflow guard — matching the semantics the
//! C/Zig/Rust/Go/… ports already use (raw machine ints). At the benchmark sizes no
//! value overflows, so the Word64 variant is BYTE-IDENTICAL to the sound-`Int`
//! `main.lg` — this differential locks that on BOTH the tree-walker and the VM —
//! while its codegen is guard-free (locked separately). The point LOGOS makes to
//! the "removes performance features / no unsigned types" critique: the integer
//! "losses" are the sound-`Int` correctness tax; at EQUAL semantics LOGOS emits the
//! same raw ops the systems languages do.
//!
//! Correctness is locked here (box-light: tree-walker + VM + codegen shape, no
//! rustc); wall-clock parity/win vs Zig is measured through `largo` on a quiet box.
//!
//! Run: `cargo nextest run -p logicaffeine-compile -E 'binary(benchmark_word64_differential)'`

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::compile::{compile_to_rust, tw_outcome_with_args, vm_outcome_with_args};

fn norm(s: &str) -> String {
    s.lines()
        .map(|l| l.trim_end())
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn read_variant(name: &str, file: &str) -> String {
    let path = format!(
        "{}/../../benchmarks/programs/{name}/{file}",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("reading {path}: {e}"))
}

/// The Word64 variant produces output BYTE-IDENTICAL to the sound-`Int` original on
/// BOTH the tree-walker and the VM, at a size where nothing overflows — so the two
/// programs are the same computation, only the integer representation differs.
fn assert_word64_matches_int(name: &str, arg: &str) {
    let int_src = read_variant(name, "main.lg");
    let w64_src = read_variant(name, "main_word64.lg");
    let argv = vec![name.to_string(), arg.to_string()];

    let int_tw = tw_outcome_with_args(&int_src, &argv);
    let w64_tw = tw_outcome_with_args(&w64_src, &argv);
    assert!(
        int_tw.error.is_none(),
        "Int {name} tree-walker error: {:?}",
        int_tw.error
    );
    assert!(
        w64_tw.error.is_none(),
        "Word64 {name} tree-walker error: {:?}",
        w64_tw.error
    );
    assert_eq!(
        norm(&w64_tw.output),
        norm(&int_tw.output),
        "Word64 `{name}` diverged from the sound-Int original on the tree-walker (arg {arg})"
    );

    let int_vm = vm_outcome_with_args(&int_src, &argv, None);
    let w64_vm = vm_outcome_with_args(&w64_src, &argv, None);
    assert!(
        w64_vm.error.is_none(),
        "Word64 {name} VM error: {:?}",
        w64_vm.error
    );
    assert_eq!(
        norm(&w64_vm.output),
        norm(&int_vm.output),
        "Word64 `{name}` diverged from the sound-Int original on the VM (arg {arg})"
    );
    assert_eq!(
        norm(&w64_vm.output),
        norm(&w64_tw.output),
        "Word64 `{name}` VM diverged from its own tree-walker (arg {arg})"
    );
}

/// The Word64 variant's hot loop must lower to RAW wrapping ops — no checked/exact
/// `Int` helper. This is the whole mechanism: guard-free == native-speed integer
/// arithmetic, the exact thing a `Word64` (machine word) buys over a sound `Int`.
fn assert_word64_guard_free(name: &str) {
    let w64_src = read_variant(name, "main_word64.lg");
    let rust = compile_to_rust(&w64_src)
        .unwrap_or_else(|e| panic!("Word64 {name} should compile to Rust: {e:?}"));
    for helper in [
        "logos_add_i64",
        "logos_sub_i64",
        "logos_mul_i64",
        "logos_add_exact",
        "checked_add",
        "checked_mul",
        "checked_sub",
    ] {
        assert!(
            !rust.contains(helper),
            "Word64 `{name}` must be guard-free (native wrapping), but `{helper}` leaked in:\n{rust}"
        );
    }
}

// ── collatz: total & k are UNBOUNDED (stopping time / k-growth aren't statically
//    bounded), so sound-Int MUST keep the overflow guard — the compiler cannot
//    legally remove it. Word64 is the only equal-semantics route, and at the
//    benchmark sizes `3k+1` never exceeds u64, so it stays byte-identical. ────────
#[test]
fn collatz_word64_matches_int() {
    assert_word64_matches_int("collatz", "300");
}
#[test]
fn collatz_word64_is_guard_free() {
    assert_word64_guard_free("collatz");
}

// ── fib_iterative (Ray's example): `(a+b) % 1e9+7`; bounded, but the Word64 form
//    is the direct rebuttal to "no unsigned types" — same value, raw wrapping. ────
#[test]
fn fib_iterative_word64_matches_int() {
    assert_word64_matches_int("fib_iterative", "500");
}
#[test]
fn fib_iterative_word64_is_guard_free() {
    assert_word64_guard_free("fib_iterative");
}

// ── loop_sum: `(sum+i) % 1e9+7` accumulator. ─────────────────────────────────────
#[test]
fn loop_sum_word64_matches_int() {
    assert_word64_matches_int("loop_sum", "500");
}
#[test]
fn loop_sum_word64_is_guard_free() {
    assert_word64_guard_free("loop_sum");
}

// ── primes: trial division — a monotone `count` counter plus `d*d <= i` and `i % d`
//    (all in-range at benchmark sizes). ─────────────────────────────────────────
#[test]
fn primes_word64_matches_int() {
    assert_word64_matches_int("primes", "500");
}
#[test]
fn primes_word64_is_guard_free() {
    assert_word64_guard_free("primes");
}

// ── gcd: exercises Word64 FUNCTION PARAMETERS (`## To gcd (a: Word64, b: Word64)
//    -> Word64`) and the Euclid `x % y` loop, summed over an n×n grid. ──────────
#[test]
fn gcd_word64_matches_int() {
    assert_word64_matches_int("gcd", "80");
}
#[test]
fn gcd_word64_is_guard_free() {
    assert_word64_guard_free("gcd");
}
