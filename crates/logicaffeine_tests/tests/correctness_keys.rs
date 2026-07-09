//! Part I correctness: map keys hash by CONTENT — and mutable containers are
//! rejected as keys.
//!
//! The audit's row: struct/tuple keys hashed by STUB (Struct by type name,
//! collections by length), so distinct keys silently collided. The spec:
//!   - Tuples and structs are VALUE keys — content-hashed, content-equal.
//!   - `1` and `1.0` are the SAME key (numeric `==` coerces, so hashing
//!     canonicalizes — the hash/equality coherence law).
//!   - A List/Set/Map key is REJECTED with a catchable error at insert:
//!     mutating a live key would silently corrupt the map, so the language
//!     refuses the footgun outright (a frozen set lands later).

mod common;
use common::{assert_interpreter_output, run_interpreter};

// NOTE ON SCOPE: the key-unification law is a DYNAMIC-value law — it lives
// where map keys are `RuntimeValue`s (the tree-walker and the VM, which the
// interpreter harness shadow-oracles). AOT maps are STATICALLY typed
// (`LogosMap<i64, _>`), so a mixed-type key there is a LOUD rustc error
// today — never silent corruption; the exact-coercion design that upgrades
// these to full differentials rides the typed-map/numeric-tower work.

// =====================================================================
// Content-keyed tuples
// =====================================================================

#[test]
fn tuple_keys_are_content_keys() {
    // A board keyed by COORDINATES — the motivating tuple-key shape. The
    // `{…}` literal is the type-honest vehicle (tuple KEY TYPES like
    // `Map of (Int, Int) to Text` have no surface syntax yet); writes use
    // the `Set m at KEY to VALUE` spelling, reads `item KEY of m`.
    assert_interpreter_output(
        r#"## Main
Let mutable board be {(1, 1): "rook"}.
Set board at (1, 2) to "pawn".
Show item (1, 1) of board.
"#,
        "rook",
    );
}

#[test]
fn distinct_tuple_keys_do_not_collide() {
    // The stub hash (length-only) made (1,1) and (1,2) the same bucket AND
    // the stub equality made lookups miss; content semantics keeps the two
    // squares distinct and findable.
    assert_interpreter_output(
        r#"## Main
Let mutable board be {(1, 1): "rook"}.
Set board at (1, 2) to "pawn".
Show length of board.
"#,
        "2",
    );
}

// =====================================================================
// Numeric key coherence: 1 and 1.0 are one key
// =====================================================================

#[test]
fn int_and_float_are_the_same_key() {
    assert_interpreter_output(
        r#"## Main
Let mutable m be {1: "int"}.
Set item 1.0 of m to "float".
Show length of m.
"#,
        "1",
    );
}

// =====================================================================
// Mutable containers are rejected as keys
// =====================================================================

#[test]
fn list_key_is_rejected_with_a_clear_error() {
    let result = run_interpreter(
        r#"## Main
Let mutable m be a new Map of Int to Text.
Set item [1, 2] of m to "x".
"#,
    );
    assert!(
        !result.success,
        "a List key must be rejected (mutating a live key corrupts the map), got success"
    );
    assert!(
        result.error.contains("key"),
        "the rejection should name the key problem, got: {}",
        result.error
    );
}
