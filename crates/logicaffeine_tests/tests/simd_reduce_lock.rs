//! SIMD-reduce transcendental accumulator — SOUNDNESS lock.
//!
//! `simd_reduce::try_lift` rewrites `Set acc to acc + trunc(<liftable float expr>)` over a counted
//! loop into an AVX2 four-lane `f64` reduction (flips `math.sin`/`cos`/`atan`). The per-lane `f64`
//! result MUST be truncated to the FULL i64 range, byte-identical to the scalar `trunc`. A packed
//! `_mm256_cvttpd_epi32` (f64 -> i32) silently wraps any lane past ±2^31 to `i32::MIN` — sound for
//! bounded outputs (sin/cos in [-1,1]) but a SILENT WRONG ANSWER for unbounded ones (`degrees` =
//! `x·57.3`, `tan` near a pole). That shipped once (caught by the stdlib-parity `math.degrees`/`tan`
//! benches, `LOGOS_WRONG`); this lock keeps it fixed. The VM tier is the correctness oracle.

use logicaffeine_compile::compile::{
    compile_to_program, compile_to_rust, program_outcome, run_logos_source,
};

/// The VM tier runs the scalar loop (no SIMD lift) — the correctness ORACLE.
fn vm_output(src: &str) -> String {
    program_outcome(&compile_to_program(src).expect("VM compiles"))
        .output
        .trim()
        .to_string()
}
/// The AOT Rust tier — where SIMD-reduce actually fires.
fn aot_output(src: &str) -> String {
    run_logos_source(src)
        .expect("AOT compiles and runs")
        .trim()
        .to_string()
}

/// `sum += trunc(toReal(i) * 100000.0)` for i = 1..=100000. Each term `i·100000` is exact; the true
/// sum is `100000 · Σi = 100000 · 5000050000 = 500005000000000` (fits i64). The per-term value grows
/// past 2^31 at i ≈ 21475, so an i32-truncating SIMD kernel wraps and diverges — while the scalar tail
/// and the VM stay correct. The literal bound keeps the program argument-free.
const UNBOUNDED_ACCUM: &str = "\
## Main
Let mutable sum be 0.
Let mutable i be 1.
While i is at most 100000:
    Set sum to sum + trunc(toReal(i) * 100000.0).
    Set i to i + 1.
Show sum.
";

/// A bounded shape the pass is meant to keep vectorizing — `trunc(i·0.5) = floor(i/2)`, whose terms
/// never leave i32 range. Sum over i = 1..=1000 is `Σ floor(i/2) = 250000`.
const BOUNDED_ACCUM: &str = "\
## Main
Let mutable sum be 0.
Let mutable i be 1.
While i is at most 1000:
    Set sum to sum + trunc(toReal(i) * 0.5).
    Set i to i + 1.
Show sum.
";

#[test]
fn unbounded_trunc_accumulator_is_correct_on_all_tiers() {
    // Before the fix the AOT/SIMD path wrapped through i32 and produced a negative, wrong sum while
    // the VM was correct — AOT must equal the VM oracle AND the known true sum.
    let expected = "500005000000000";
    assert_eq!(vm_output(UNBOUNDED_ACCUM), expected, "VM oracle sanity");
    assert_eq!(
        aot_output(UNBOUNDED_ACCUM),
        expected,
        "AOT/SIMD diverged from the scalar sum — the f64->i32 truncation is back"
    );
}

#[test]
fn simd_kernel_never_truncates_through_i32() {
    let rust = compile_to_rust(UNBOUNDED_ACCUM).expect("compiles to rust");
    // The pass fired (else there is nothing to keep sound)...
    assert!(
        rust.contains("__logos_simd_chunks"),
        "SIMD-reduce must fire on the trunc-accumulator shape. emit:\n{rust}"
    );
    // ...and the per-lane truncation must NOT go through the i32-narrowing intrinsic.
    assert!(
        !rust.contains("_mm256_cvttpd_epi32"),
        "SIMD kernel truncates f64 -> i32 (silently wraps past ±2^31). emit:\n{rust}"
    );
    assert!(
        rust.contains("_mm256_set_epi64x"),
        "SIMD kernel must truncate per-lane to full i64. emit:\n{rust}"
    );
}

#[test]
fn simd_reduce_still_fires_and_is_correct_on_a_bounded_shape() {
    // The soundness fix must not disable the pass on the bounded shapes it wins on.
    let rust = compile_to_rust(BOUNDED_ACCUM).expect("compiles to rust");
    assert!(
        rust.contains("__logos_simd_chunks"),
        "SIMD-reduce must still fire on a bounded trunc-accumulator. emit:\n{rust}"
    );
    let expected = "250000";
    assert_eq!(vm_output(BOUNDED_ACCUM), expected, "VM oracle sanity");
    assert_eq!(
        aot_output(BOUNDED_ACCUM),
        expected,
        "AOT diverged on the bounded shape"
    );
}
