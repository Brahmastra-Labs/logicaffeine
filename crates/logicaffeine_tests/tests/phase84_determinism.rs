//! LOCK: generated Rust must be byte-identical across recompiles.
//!
//! Re-generating the same source must produce the same Rust every time, or
//! version control thrashes. Codegen collects items from `HashMap`s whose
//! iteration order is randomized per run (each compile builds fresh state with
//! new hash seeds), so without explicit ordering the output drifts. These tests
//! lock determinism for all three paths: imperative, math, and logic.
//!
//! (Scope is the EMITTED RUST only — the VM's internal ordering is free to differ
//! for speed; it produces no versioned artifact.)

use logicaffeine_compile::{
    compile_to_rust_with_proven, extract_logic_rust, extract_math_rust_from_source,
    generate_rust_code,
};

/// Enough repeats that a per-run-randomized HashMap order would almost certainly
/// diverge from the first run at least once.
const RUNS: usize = 16;

// --- math (kernel→Rust extraction) -------------------------------------------

const MATH: &str = r#"
Inductive Nat := Zero : Nat | Succ : Nat -> Nat.
Definition one : Nat := Succ Zero.
Definition two : Nat := Succ one.
Definition three : Nat := Succ two.
Definition id : Nat -> Nat := fun n : Nat => n.
Inductive MyBool := Yes : MyBool | No : MyBool.
Definition not_b : MyBool -> MyBool :=
  fun b : MyBool => match b return (fun _ : MyBool => MyBool) with | Yes => No | No => Yes.
"#;

#[test]
fn math_extraction_is_deterministic() {
    let first = extract_math_rust_from_source(MATH);
    assert!(first.contains("enum"), "sanity: should extract real content:\n{first}");
    for k in 1..RUNS {
        let again = extract_math_rust_from_source(MATH);
        assert_eq!(
            first, again,
            "math extraction not deterministic (run {k} differed):\n--- first ---\n{first}\n--- run {k} ---\n{again}"
        );
    }
}

// --- imperative (LOGOS→Rust codegen) -----------------------------------------
//
// Several user types (structs + enums) with names whose source order differs
// from their sorted order — exposes any HashMap-iteration nondeterminism.

const IMP: &str = r#"## Definition

A Zebra has:
    a public stripes, which is Int.

A Wolf has:
    a public pack, which is Int.

## A Mango is one of:
    A Ripe.
    A Unripe.

## A Banana is one of:
    A Yellow.
    A Green.

## Main

Let z be a new Zebra.
Set z's stripes to 5.
Show z's stripes.
"#;

#[test]
fn imperative_codegen_is_deterministic() {
    let first = generate_rust_code(IMP).expect("imperative program should compile");
    assert!(first.contains("enum") || first.contains("struct"), "sanity: emitted types:\n{first}");
    for k in 1..RUNS {
        let again = generate_rust_code(IMP).expect("imperative program should compile");
        assert_eq!(
            first, again,
            "imperative codegen not deterministic (run {k} differed)"
        );
    }
}

// --- mixed link (imperative + bundled proven module) -------------------------
//
// The proven module is injected as `pub mod proven { … } use proven::*;`. The
// wrapper is a constant string, so the bundled output must be byte-identical across
// recompiles for any fixed (source, proven) pair — no version-control thrash.

const PROVEN_MODULE: &str = "pub fn double(x: i64) -> i64 { x + x }\n\
                             pub fn check_double(n: i64) -> bool { double(n) == n + n }\n";

#[test]
fn bundled_proven_codegen_is_deterministic() {
    let first = compile_to_rust_with_proven(IMP, PROVEN_MODULE).expect("compiles");
    assert!(first.contains("pub mod proven"), "sanity: module bundled:\n{first}");
    for k in 1..RUNS {
        let again = compile_to_rust_with_proven(IMP, PROVEN_MODULE).expect("compiles");
        assert_eq!(first, again, "bundled codegen not deterministic (run {k} differed)");
    }
}

// --- logic (theorem/English→Rust extraction) ---------------------------------

// Relational + quantified: exercises event-role ordering and fact/domain
// collection, which must be deterministic across runs.
const LOGIC: &str = "Every dog chased some cat.";

#[test]
fn logic_extraction_is_deterministic() {
    let first = extract_logic_rust(LOGIC).expect("ok");
    for k in 1..RUNS {
        let again = extract_logic_rust(LOGIC).expect("ok");
        assert_eq!(first, again, "logic extraction not deterministic (run {k} differed)");
    }
}

// Temporal monitor output must also be byte-identical across runs.
const LOGIC_TEMPORAL: &str = "Always, every dog runs.";

#[test]
fn logic_temporal_extraction_is_deterministic() {
    let first = extract_logic_rust(LOGIC_TEMPORAL).expect("ok");
    for k in 1..RUNS {
        let again = extract_logic_rust(LOGIC_TEMPORAL).expect("ok");
        assert_eq!(first, again, "temporal extraction not deterministic (run {k} differed)");
    }
}
