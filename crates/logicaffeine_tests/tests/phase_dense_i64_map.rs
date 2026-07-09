//! Dense lowering for `Map of Int to Int` → a direct-addressed flat array
//! (`LogosDenseI64Map` / `LogosDenseI64Set`).
//!
//! When the compiler PROVES (via the kernel LIA bounds prover, the same engine
//! that drives bounds-check elimination) that every key inserted, got, or tested
//! on a non-aliased `Int → Int` map declared `with capacity CAP` lands in
//! `[0, CAP]`, the open-addressing `LogosI64Map` is replaced by a direct-addressed
//! array sized `CAP + 1`: `insert`/`get` become `data[key] = v` / `data[key]` with
//! a presence bit — no hashing, no probing, no sparse table. This is what lets
//! compiled Logos beat C++'s `unordered_map` on the `collect` benchmark.
//!
//! These pin the GENERATED SHAPE (dense fires where the key domain is proven
//! bounded, and refuses — staying `LogosI64Map` — where it is not) and end-to-end
//! correctness through the dense path, including the absent-in-range key that the
//! presence bitset must report as `None`. The data structures themselves are
//! fuzzed against `HashMap`/`HashSet` in `logicaffeine_data::types::tests`.

#![cfg(not(target_arch = "wasm32"))]

mod common;
use common::{assert_compiled_equals_interpreted, assert_compiled_equals_interpreted_eq, compile_to_rust};

/// CONSTANT capacity, keys provably in range, and every queried key provably
/// inserted (the insert loop fully covers `1..21` with unit stride): the `collect`
/// shape lowers all the way to the PRESENCE-ELIDED array — `get` is a bare
/// `data[key]` load, no presence bitset.
#[test]
fn collect_constant_capacity_lowers_to_dense_nopresence() {
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
        rust.contains("LogosDenseI64MapNoPresence::with_bounds("),
        "fully-covered keys must lower to the presence-elided LogosDenseI64MapNoPresence. Got:\n{rust}"
    );
    assert!(
        !rust.contains("LogosI64Map") && !rust.contains("LogosMap<i64, i64>"),
        "dense supersedes the hash-map tiers; neither should remain. Got:\n{rust}"
    );
}

/// THE HEADLINE: a SYMBOLIC capacity `n` (the real benchmark's shape, `n` from a
/// runtime parse) still lowers to the presence-elided dense array, because the
/// relational LIA prover discharges `i <= n` (bound) AND `1 <= i <= n` (coverage)
/// from the loop guard `i < n + 1` — bounds the interval domain alone cannot
/// express. This is the case that beats C++.
#[test]
fn collect_symbolic_capacity_lowers_to_dense() {
    let src = r#"## To run (n: Int) -> Int:
    Let mutable m be a new Map of Int to Int with capacity n.
    Let mutable i be 1.
    While i is less than n + 1:
        Set item i of m to i * 2.
        Set i to i + 1.
    Let mutable found be 0.
    Set i to 1.
    While i is less than n + 1:
        If item i of m equals i * 2:
            Set found to found + 1.
        Set i to i + 1.
    Return found.

## Main
Show run(20).
"#;
    let rust = compile_to_rust(src).unwrap();
    assert!(
        rust.contains("LogosDenseI64MapNoPresence::with_bounds("),
        "a symbolic-capacity map whose keys are proven `<= n` and fully covered \
         must lower to the presence-elided LogosDenseI64MapNoPresence. Got:\n{rust}"
    );
}

/// SOUNDNESS — presence elision must NOT fire without proven full coverage: a
/// NON-unit insert stride (only even keys written) leaves gaps, so the coverage
/// recognizer refuses and the always-correct presence bitset is KEPT
/// (`LogosDenseI64Map`, not `…NoPresence`). The reads here happen to hit only
/// inserted keys, but the compiler cannot prove that, so it stays conservative.
#[test]
fn dense_map_keeps_presence_when_coverage_unprovable() {
    let src = r#"## Main
Let mutable m be a new Map of Int to Int with capacity 20.
Let mutable i be 2.
While i is less than 21:
    Set item i of m to i * 2.
    Set i to i + 2.
Let mutable found be 0.
Set i to 2.
While i is less than 21:
    If item i of m equals i * 2:
        Set found to found + 1.
    Set i to i + 2.
Show found.
"#;
    let rust = compile_to_rust(src).unwrap();
    assert!(
        rust.contains("LogosDenseI64Map::with_bounds("),
        "a bounded-key map must still go dense. Got:\n{rust}"
    );
    assert!(
        !rust.contains("NoPresence"),
        "without proven full coverage the presence bitset must be KEPT. Got:\n{rust}"
    );
    assert_compiled_equals_interpreted_eq(src, "10");
}

/// KILL SWITCH: with BOTH the dense gate and the i32-narrowing tier forced off,
/// the collect shape falls all the way back to the base open-addressing
/// `LogosI64Map`. (Nextest runs each test in its own process, so the env vars do
/// not leak.)
#[test]
fn kill_switch_disables_dense() {
    std::env::set_var("LOGOS_OPT_OFF", "densemap,narrowmap");
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
    std::env::remove_var("LOGOS_OPT_OFF");
    assert!(
        !rust.contains("LogosDense") && !rust.contains("LogosI32") && rust.contains("LogosI64Map"),
        "both kill switches off must fall back to the base LogosI64Map. Got:\n{rust}"
    );
}

/// END-TO-END: the dense collect shape computes the SAME result as the
/// tree-walking interpreter (all 20 keys found).
#[test]
fn dense_collect_matches_interpreter() {
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
    assert_compiled_equals_interpreted_eq(src, "20");
}

/// SOUNDNESS — the presence bitset's ABSENT path: insert only EVEN keys, then
/// test membership of EVERY key. The odd keys were never inserted, so the
/// presence bit is clear and `contains` must report `false` (not a stale hit).
/// Membership is well-defined for absent keys (unlike a value read), so the dense
/// set must agree with the interpreter: 10 even keys present in `2..21`.
/// (The `get → None` path itself is pinned directly in the data-structure fuzz.)
#[test]
fn dense_absent_in_range_key_is_not_a_member() {
    let src = r#"## Main
Let mutable seen be a new Map of Int to Int with capacity 20.
Let mutable i be 2.
While i is less than 21:
    Set item i of seen to 1.
    Set i to i + 2.
Let mutable hits be 0.
Set i to 1.
While i is less than 21:
    If seen contains i:
        Set hits to hits + 1.
    Set i to i + 1.
Show hits.
"#;
    let rust = compile_to_rust(src).unwrap();
    assert!(
        rust.contains("LogosDenseI64Set"),
        "the bounded-key set must go dense even with a sparse insert. Got:\n{rust}"
    );
    assert_compiled_equals_interpreted_eq(src, "10");
}

/// A contains+insert map whose value is NEVER read lowers to the keys-only dense
/// SET (`LogosDenseI64Set`, a presence bitset with no value array).
#[test]
fn dense_set_lowers_to_dense_set() {
    let src = r#"## Main
Let mutable seen be a new Map of Int to Int with capacity 20.
Let mutable i be 1.
While i is less than 21:
    Set item i of seen to 1.
    Set i to i + 1.
Let mutable hits be 0.
Set i to 1.
While i is less than 21:
    If seen contains i:
        Set hits to hits + 1.
    Set i to i + 1.
Show hits.
"#;
    let rust = compile_to_rust(src).unwrap();
    assert!(
        rust.contains("LogosDenseI64Set"),
        "a value-free bounded-key map must lower to the keys-only LogosDenseI64Set. Got:\n{rust}"
    );
    assert_compiled_equals_interpreted_eq(src, "20");
}

/// IMPLICIT capacity: a `Map of Int to Int` with NO `with capacity` whose keys are
/// provably bounded by its fill loop now lowers to a dense direct-addressed array.
/// The implicit-capacity pass infers the loop bound as the candidate cap and the
/// LIA prover discharges `0 <= key <= cap`, so no explicit hint is needed. (The
/// soundness boundary — a no-capacity map with UNBOUNDED keys staying a hash map —
/// is covered by `implicit_capacity_unbounded_keys_stay_hash`.)
#[test]
fn no_capacity_bounded_map_lowers_to_dense() {
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
    assert!(
        rust.contains("LogosDense"),
        "a bare new Map with keys provably bounded by the fill loop now lowers to a \
         dense array (implicit capacity). Got:\n{rust}"
    );
    // And it must be CORRECT through the dense path.
    assert_compiled_equals_interpreted(src);
}

/// SOUNDNESS — keys exceed capacity: `with capacity 10` but keys range to ~999.
/// The bound proof `key <= 10` fails for the high keys, so the map must NOT go
/// dense (a `data[999]` into a 11-slot array would be out of bounds). It stays an
/// open-addressing hash map, whose hashing handles any key.
#[test]
fn keys_exceeding_capacity_are_not_dense() {
    let src = r#"## Main
Let mutable m be a new Map of Int to Int with capacity 10.
Let mutable i be 1.
While i is less than 1000:
    Set item i of m to i * 2.
    Set i to i + 1.
Let mutable found be 0.
Set i to 1.
While i is less than 1000:
    If item i of m equals i * 2:
        Set found to found + 1.
    Set i to i + 1.
Show found.
"#;
    let rust = compile_to_rust(src).unwrap();
    assert!(
        !rust.contains("LogosDense"),
        "keys provably exceeding the capacity must NOT lower to a dense array. Got:\n{rust}"
    );
    assert!(
        rust.contains("LogosI32Map") || rust.contains("LogosI64Map"),
        "such a map stays an open-addressing hash map. Got:\n{rust}"
    );
    // And it must still be CORRECT through the hash-map path.
    assert_compiled_equals_interpreted_eq(src, "999");
}

/// IMPLICIT capacity + ELEMENT-DERIVED keys — two_sum's `seen`. A BARE
/// `new Map of Int to Int` (no `with capacity`) used as a SET, filled in a counted
/// loop, whose keys are NOT loop counters but `x = item i of arr` (insert) and
/// `n - x` (lookup). The implicit-capacity pass takes the fill loop's bound `n` as
/// the candidate key-domain cap; the TRANSITIVE scalar-def chain proves
/// `0 <= x <= n` and (through `complement = n - x`) `0 <= n - x <= n`, lowering
/// `seen` to the direct-addressed bitset — the lever that makes two_sum beat C.
#[test]
fn implicit_capacity_element_keyed_set_lowers_to_dense() {
    let src = r#"## To run (n: Int) -> Int:
    Let mutable arr be a new Seq of Int.
    Let mutable seed be 42.
    Let mutable j be 0.
    While j is less than n:
        Set seed to (seed * 1103515245 + 12345) % 2147483648.
        Push ((seed / 65536) % 32768) % n to arr.
        Set j to j + 1.
    Let mutable seen be a new Map of Int to Int.
    Let mutable count be 0.
    Let mutable i be 1.
    While i is at most n:
        Let x be item i of arr.
        Let complement be n - x.
        If complement is at least 0:
            If seen contains complement:
                Set count to count + 1.
        Set seen at x to 1.
        Set i to i + 1.
    Return count.

## Main
Show run(1000).
"#;
    let rust = compile_to_rust(src).unwrap();
    assert!(
        rust.contains("LogosDenseI64Set::with_bounds("),
        "a bare new Map used as a set, with element-derived keys bounded by the fill \
         loop, must lower to the direct-addressed LogosDenseI64Set. Got:\n{rust}"
    );
    // End-to-end: the dense bitset computes the SAME count as the interpreter.
    assert_compiled_equals_interpreted(src);
}

/// SOUNDNESS — implicit capacity, but keys EXCEED the inferred bound. A bare
/// `new Map` inserted at `i * 1000` while `i <= n`: the candidate cap `n` cannot
/// bound `i * 1000` (which reaches `n * 1000`), so the proof fails and the map
/// stays an open-addressing hash map — never a `data[n*1000]` into an `n+1`-slot
/// array. The proof-gate, not a heuristic, is what keeps this sound.
#[test]
fn implicit_capacity_unbounded_keys_stay_hash() {
    let src = r#"## To run (n: Int) -> Int:
    Let mutable m be a new Map of Int to Int.
    Let mutable i be 1.
    While i is at most n:
        Set item (i * 1000) of m to 1.
        Set i to i + 1.
    Return 0.

## Main
Show run(10).
"#;
    let rust = compile_to_rust(src).unwrap();
    assert!(
        !rust.contains("LogosDense"),
        "keys exceeding the inferred capacity must keep the hash map (sound). Got:\n{rust}"
    );
}
