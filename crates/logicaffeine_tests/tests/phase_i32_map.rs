//! i32-narrowing for a NON-dense `Map of Int to Int` → the half-width
//! `LogosI32Map` / quarter-width `LogosI32Set`.
//!
//! When a map cannot be made dense (no proven contiguous key window) but the
//! compiler proves every key — and, for a value-read map, every stored value —
//! fits `i32`, the open-addressing table narrows from 16-byte `(i64,i64)` slots
//! to 8-byte `(i32,i32)` (or a 4-byte keys-only set). Halving the slot width
//! halves the memory traffic that dominates this random-access workload. The
//! `i64` call surface is unchanged (keys/values cast at the boundary; the proof
//! makes the cast lossless), so codegen emits it exactly like `LogosI64Map`.
//!
//! These pin the GENERATED SHAPE (narrowing fires where keys/values are proven
//! in range, refuses where they are not) and end-to-end correctness; the data
//! structures are fuzzed against `HashMap`/`HashSet` in `logicaffeine_data`.
//!
//! Narrowing is ON by default (a sound, lossless optimization) and can be forced
//! off with `LOGOS_NARROW_MAP=0`. Nextest runs each test in its own process, so
//! setting the env var inside a test does not leak to others.

#![cfg(not(target_arch = "wasm32"))]

mod common;
use common::{assert_compiled_equals_interpreted_eq, compile_to_rust};

/// A value-read map with NO capacity hint (so not dense) but keys (`i % 50`) and
/// values (`i % 1000`) provably in `i32` range → narrows to `LogosI32Map`.
#[test]
fn bounded_keys_values_narrow_to_i32_map() {
    let src = r#"## Main
Let mutable m be a new Map of Int to Int.
Let mutable i be 0.
While i is less than 100:
    Set item (i % 50) of m to (i % 1000).
    Set i to i + 1.
Let mutable acc be 0.
Set i to 0.
While i is less than 50:
    Set acc to acc + (item i of m).
    Set i to i + 1.
Show acc.
"#;
    let rust = compile_to_rust(src).unwrap();
    assert!(
        rust.contains("LogosI32Map"),
        "a non-dense map with proven i32-range keys+values must narrow to LogosI32Map. Got:\n{rust}"
    );
    assert!(
        !rust.contains("LogosI64Map"),
        "the narrowed map must not keep the i64-width table. Got:\n{rust}"
    );
    // i=0..99 write (i%50)->(i%1000); the last writer of key k is k+50 → value k+50.
    // acc = sum over k in 0..49 of (k + 50) = sum(50..99) = (50+99)*50/2 = 3725.
    assert_compiled_equals_interpreted_eq(src, "3725");
}

/// A set-usage map (value never read) with bounded keys narrows to the keys-only
/// `LogosI32Set`.
#[test]
fn bounded_keys_set_narrows_to_i32_set() {
    let src = r#"## Main
Let mutable seen be a new Map of Int to Int.
Let mutable hits be 0.
Let mutable i be 0.
While i is less than 100:
    If seen contains (i % 30):
        Set hits to hits + 1.
    Set seen at (i % 30) to 1.
    Set i to i + 1.
Show hits.
"#;
    let rust = compile_to_rust(src).unwrap();
    assert!(
        rust.contains("LogosI32Set"),
        "a value-free map with proven i32-range keys must narrow to LogosI32Set. Got:\n{rust}"
    );
    // Residues 0..29 each first-miss then hit: 100 iters - 30 distinct = 70 hits.
    assert_compiled_equals_interpreted_eq(src, "70");
}

/// SOUNDNESS — keys exceed i32: even with narrowing ENABLED, a key
/// `i * 100000000` reaches ~9.9e9 > i32::MAX, so narrowing must REFUSE (an
/// `as i32` cast would truncate). The map keeps the full-width `LogosI64Map`.
#[test]
fn keys_exceeding_i32_stay_logos_i64_map() {
    let src = r#"## Main
Let mutable m be a new Map of Int to Int.
Let mutable i be 0.
While i is less than 100:
    Set item (i * 100000000) of m to 1.
    Set i to i + 1.
Let mutable acc be 0.
Set i to 0.
While i is less than 100:
    Set acc to acc + (item (i * 100000000) of m).
    Set i to i + 1.
Show acc.
"#;
    let rust = compile_to_rust(src).unwrap();
    assert!(
        !rust.contains("LogosI32"),
        "keys provably exceeding i32 must NOT narrow. Got:\n{rust}"
    );
    assert!(
        rust.contains("LogosI64Map"),
        "such a map keeps the full-width LogosI64Map. Got:\n{rust}"
    );
}

/// KILL SWITCH: `LOGOS_NARROW_MAP=0` forces the i32 tier off; the bounded map
/// falls back to `LogosI64Map`.
#[test]
fn kill_switch_disables_i32_narrowing() {
    std::env::set_var("LOGOS_OPT_OFF", "narrowmap");
    let src = r#"## Main
Let mutable m be a new Map of Int to Int.
Let mutable i be 0.
While i is less than 100:
    Set item (i % 50) of m to (i % 1000).
    Set i to i + 1.
Let mutable acc be 0.
Set i to 0.
While i is less than 50:
    Set acc to acc + (item i of m).
    Set i to i + 1.
Show acc.
"#;
    let rust = compile_to_rust(src).unwrap();
    std::env::remove_var("LOGOS_OPT_OFF");
    assert!(
        !rust.contains("LogosI32") && rust.contains("LogosI64Map"),
        "LOGOS_NARROW_MAP=0 must fall back to LogosI64Map. Got:\n{rust}"
    );
}
