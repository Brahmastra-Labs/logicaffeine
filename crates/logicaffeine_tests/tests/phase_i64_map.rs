//! De-Rc for `Map of Int to Int` → the specialized open-addressing
//! `LogosI64Map`.
//!
//! `LogosMap<i64, i64>` is `Rc<RefCell<FxHashMap>>`: every `.get`/`.insert`
//! borrows the cell and clones the value, and the handle has reference
//! semantics. For a map that the alias analysis proves is a non-aliased,
//! non-escaping local used only as construct / insert / get / contains
//! (two_sum's `seen`, collect's `m`), codegen emits `LogosI64Map` instead —
//! two flat `Vec`s, linear probing, `Copy` keys/values, `&mut self` mutation
//! LLVM keeps in registers — the C open-addressing shape.
//!
//! These pin the GENERATED SHAPE (the conversion fires where it is safe and
//! refuses where it is not) plus end-to-end correctness through the converted
//! path. The `LogosI64Map` data structure itself is fuzzed against `HashMap` in
//! `logicaffeine_data::types::tests`, and the benchmark programs are covered
//! end-to-end by `benchmarks/verify-differential.sh`.

#![cfg(not(target_arch = "wasm32"))]

mod common;
use common::{assert_compiled_equals_interpreted_eq, compile_to_rust};

/// two_sum's shape: an `Int → Int` map used only via `contains` + insert, with
/// the value NEVER read. It is a set, so it must lower to a keys-only set (no
/// value array) — never the reference-semantic `LogosMap<i64, i64>`, and not a
/// value-bearing map. The keys here (`i % 5`) are bounded, so the i32-narrowing
/// tier (on by default) further narrows it to the quarter-width `LogosI32Set`.
#[test]
fn int_int_set_usage_map_lowers_to_keys_only_set() {
    let src = r#"## Main
Let mutable seen be a new Map of Int to Int.
Let mutable count be 0.
Let mutable i be 0.
While i is less than 10:
    If seen contains (i % 5):
        Set count to count + 1.
    Set seen at (i % 5) to 1.
    Set i to i + 1.
Show count.
"#;
    let rust = compile_to_rust(src).unwrap();
    assert!(
        rust.contains("LogosI32Set"),
        "a bounded-key contains+insert map with no value read must lower to the keys-only LogosI32Set. Got:\n{rust}"
    );
    assert!(
        !rust.contains("LogosMap<i64, i64>"),
        "the converted set must not keep the reference-semantic LogosMap<i64,i64>. Got:\n{rust}"
    );
}

/// SPECIFICITY: a map whose value IS read back (`item k of m`) needs the value
/// array, so it stays a value-bearing MAP, NOT a keys-only set — the set
/// optimization must NOT over-fire. (With bounded keys it now lowers to the dense
/// `LogosDenseI64Map`; the invariant under test is map-not-set, not the tier.)
#[test]
fn value_read_map_stays_a_map_not_set() {
    let src = r#"## Main
Let mutable m be a new Map of Int to Int.
Let mutable i be 0.
While i is less than 10:
    Set item i of m to i * 7.
    Set i to i + 1.
Let mutable acc be 0.
Set i to 0.
While i is less than 10:
    Set acc to acc + (item i of m).
    Set i to i + 1.
Show acc.
"#;
    let rust = compile_to_rust(src).unwrap();
    let is_map = rust.contains("LogosDenseI64Map")
        || rust.contains("LogosI64Map")
        || rust.contains("LogosI32Map");
    let is_set = rust.contains("LogosDenseI64Set")
        || rust.contains("LogosI64Set")
        || rust.contains("LogosI32Set");
    assert!(
        is_map && !is_set,
        "a map whose value is read must stay a value-bearing map, not a keys-only set. Got:\n{rust}"
    );
}

/// END-TO-END: the converted set (insert + contains, two_sum's shape) computes
/// the SAME result as the tree-walking interpreter.
#[test]
fn converted_set_matches_interpreter() {
    let src = r#"## Main
Let mutable seen be a new Map of Int to Int.
Let mutable hits be 0.
Let mutable i be 0.
While i is less than 20:
    If seen contains (i % 7):
        Set hits to hits + 1.
    Set seen at (i % 7) to 1.
    Set i to i + 1.
Show hits.
"#;
    // Residues 0..6 each first-miss then hit: 20 iters - 7 distinct = 13 hits.
    assert_compiled_equals_interpreted_eq(src, "13");
}

/// collect's shape: an `Int → Int` map declared `with capacity` and filled by
/// indexed insert over `1..21`, read back over the same range. The keys are
/// PROVABLY within the capacity, so it lowers past `LogosI64Map` all the way to
/// the direct-addressed `LogosDenseI64Map` (no hashing/probing). Pinned in full
/// in `phase_dense_i64_map.rs`; kept here so the i64-map tier and its dense
/// successor stay covered side by side.
#[test]
fn int_int_with_capacity_collect_shape_lowers_to_dense() {
    let src = r#"## Main
Let mutable m be a new Map of Int to Int with capacity 20.
Let mutable i be 1.
While i is less than 21:
    Set item i of m to i * 2.
    Set i to i + 1.
Let mutable found be 0.
Set i to 1.
While i is less than 21:
    If item i of m equals i * 2:
        Set found to found + 1.
    Set i to i + 1.
Show found.
"#;
    let rust = compile_to_rust(src).unwrap();
    assert!(
        rust.contains("LogosDenseI64Map"),
        "a with-capacity map whose key domain is proven bounded must lower to a \
         direct-addressed dense map (presence-tracked or elided). Got:\n{rust}"
    );
    assert!(
        !rust.contains("LogosMap<i64, i64>"),
        "the converted map must not keep LogosMap<i64,i64>. Got:\n{rust}"
    );
}

/// SOUNDNESS: a map that is ALIASED (bound to a second name) has reference
/// semantics the program may depend on, so it must STAY `LogosMap` — the
/// value-semantic `LogosI64Map` would silently break sharing.
#[test]
fn aliased_int_int_map_stays_logos_map() {
    let src = r#"## Main
Let mutable seen be a new Map of Int to Int.
Set seen at 1 to 10.
Let alias be seen.
Set seen at 2 to 20.
Show item 2 of alias.
"#;
    let rust = compile_to_rust(src).unwrap();
    // The reference-semantic map is emitted via the `Map` alias (`Map::<i64,i64>`)
    // and shared with `.clone()` — NOT lowered to the value-semantic LogosI64Map.
    assert!(
        rust.contains("Map::<i64, i64>"),
        "an aliased map must keep the reference-semantic Map (LogosMap alias). Got:\n{rust}"
    );
    assert!(
        !rust.contains("LogosI64Map"),
        "an aliased map must NOT be lowered to the value-semantic LogosI64Map. Got:\n{rust}"
    );
}

/// SOUNDNESS: a map passed to a function ESCAPES its scope, so it must stay
/// `LogosMap` (the function receives the shared handle).
#[test]
fn escaping_int_int_map_stays_logos_map() {
    let src = r#"## To total (m: Map of Int to Int) -> Int:
    Return 0.

## Main
Let mutable seen be a new Map of Int to Int.
Set seen at 1 to 10.
Let t be total(seen).
Show t.
"#;
    let rust = compile_to_rust(src).unwrap();
    assert!(
        !rust.contains("LogosI64Map"),
        "a map passed to a function escapes and must NOT be lowered to LogosI64Map. Got:\n{rust}"
    );
}

/// SPECIFICITY: only `Int → Int` qualifies. A `Text → Int` map keeps
/// `LogosMap` (the specialized map is i64→i64 only).
#[test]
fn non_int_key_map_stays_logos_map() {
    let src = r#"## Main
Let mutable counts be a new Map of Text to Int.
Set counts at "a" to 1.
Show item "a" of counts.
"#;
    let rust = compile_to_rust(src).unwrap();
    assert!(
        !rust.contains("LogosI64Map"),
        "a Text→Int map must stay LogosMap, not the i64-specialized map. Got:\n{rust}"
    );
}

/// END-TO-END: the converted membership map computes the SAME result as the
/// tree-walking interpreter (the conversion preserves meaning).
#[test]
fn converted_membership_map_matches_interpreter() {
    let src = r#"## Main
Let mutable seen be a new Map of Int to Int.
Let mutable count be 0.
Let mutable i be 0.
While i is less than 10:
    If seen contains (i % 5):
        Set count to count + 1.
    Set seen at (i % 5) to 1.
    Set i to i + 1.
Show count.
"#;
    // First insert of each residue misses, the rest hit: 10 - 5 = 5 hits.
    assert_compiled_equals_interpreted_eq(src, "5");
}

/// END-TO-END: the converted pre-sized map (insert + indexed get + equality)
/// matches the interpreter across a full build-then-scan.
#[test]
fn converted_with_capacity_map_matches_interpreter() {
    let src = r#"## Main
Let mutable m be a new Map of Int to Int with capacity 50.
Let mutable i be 1.
While i is less than 51:
    Set item i of m to i * i.
    Set i to i + 1.
Let mutable hits be 0.
Set i to 1.
While i is less than 51:
    If item i of m equals i * i:
        Set hits to hits + 1.
    Set i to i + 1.
Show hits.
"#;
    assert_compiled_equals_interpreted_eq(src, "50");
}

/// END-TO-END across a resize storm: many distinct keys force the open-address
/// table to grow repeatedly; the converted map must still agree exactly.
#[test]
fn converted_map_survives_growth() {
    let src = r#"## Main
Let mutable m be a new Map of Int to Int.
Let mutable i be 0.
While i is less than 1000:
    Set item i of m to i + 7.
    Set i to i + 1.
Let mutable acc be 0.
Set i to 0.
While i is less than 1000:
    If m contains i:
        Set acc to acc + (item i of m).
    Set i to i + 1.
Show acc.
"#;
    // sum_{i=0}^{999} (i + 7) = 999*1000/2 + 7*1000 = 499500 + 7000 = 506500.
    assert_compiled_equals_interpreted_eq(src, "506500");
}
