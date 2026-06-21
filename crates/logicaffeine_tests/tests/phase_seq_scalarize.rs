//! O3 gate — small fixed-size Seq scalarization.
//!
//! A `Seq` with a compile-time-known constant size that is never resized and
//! never escapes becomes a fixed Rust array `[T; N]` — C's exact
//! representation for nbody's 5 bodies. Stack-allocated, statically-bounded
//! indices (no `RefCell`, no `panic_bounds_check`), register-allocatable.
//!
//! Qualification (v1, Main only): `Let mutable x be a new Seq of {Int|Float|
//! Bool}` followed by exactly N straight-line `Push x` statements (N ≤ 64)
//! before any other use, and thereafter ONLY Index reads, SetIndex writes,
//! and Length. Anything that could change the size, alias the handle, or let
//! it escape disqualifies — those keep the `LogosSeq` representation.

mod common;

use common::compile_to_rust;

// =============================================================================
// Positive: the array representation forms and runs correctly
// =============================================================================

#[test]
fn scalarize_int_seq() {
    let source = r#"## Main
Let mutable xs be a new Seq of Int.
Push 10 to xs.
Push 20 to xs.
Push 30 to xs.
Let mutable total be 0.
Let mutable i be 1.
While i is at most 3:
    Set total to total + item i of xs.
    Set i to i + 1.
Show total.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains("[i64; 3]"),
        "fixed-size Int Seq should scalarize to [i64; 3]. Got:\n{}",
        rust
    );
    assert!(
        !rust.contains("xs: LogosSeq") && !rust.contains("xs = LogosSeq"),
        "scalarized xs must not be a LogosSeq. Got:\n{}",
        rust
    );
    // OPT-8 zero-basing applies to arrays too: raw `xs[i]`, no `(i - 1)`.
    assert!(
        rust.contains("xs[i as usize]") && !rust.contains("xs[(i - 1) as usize]"),
        "scalarized array should get zero-based direct indexing. Got:\n{}",
        rust
    );
    common::assert_exact_output(source, "60");
}

#[test]
fn scalarize_float_seq_nbody_shape() {
    // Two parallel fixed Seqs of floats with an INTERLEAVED (round-robin) init
    // and a pairwise reduction — nbody in miniature. Because they are co-indexed
    // columns of an array-of-structs, AoS interleaving fuses them into one
    // `[[f64; 2]; 3]` backing (C's struct-array layout) so the fields pack —
    // strictly better than two separate `[f64; 3]` arrays.
    let source = r#"## Main
Let mutable px be a new Seq of Float.
Let mutable vx be a new Seq of Float.
Push 1.0 to px. Push 0.5 to vx.
Push 2.0 to px. Push 1.5 to vx.
Push 3.0 to px. Push 2.5 to vx.
Let mutable acc be 0.0.
Let mutable i be 1.
While i is at most 3:
    Set acc to acc + item i of px * item i of vx.
    Set i to i + 1.
Show "{acc:.2}".
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains("[[f64; 2]; 3]"),
        "co-indexed fixed Float Seqs should fuse into one [[f64; 2]; 3] AoS backing. Got:\n{}",
        rust
    );
    assert!(
        !rust.contains("px: LogosSeq") && !rust.contains("vx: LogosSeq"),
        "the fused px/vx backing must not be a LogosSeq. Got:\n{}",
        rust
    );
    // 1*0.5 + 2*1.5 + 3*2.5 = 0.5 + 3.0 + 7.5 = 11.0
    common::assert_exact_output(source, "11.00");
}

#[test]
fn scalarize_bool_seq() {
    let source = r#"## Main
Let mutable flags be a new Seq of Bool.
Push true to flags.
Push false to flags.
Push true to flags.
Let mutable count be 0.
Let mutable i be 1.
While i is at most 3:
    If item i of flags:
        Set count to count + 1.
    Set i to i + 1.
Show count.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains("[bool; 3]"),
        "fixed-size Bool Seq should scalarize to [bool; 3]. Got:\n{}",
        rust
    );
    common::assert_exact_output(source, "2");
}

#[test]
fn scalarize_with_setindex_and_length() {
    // SetIndex writes and `length of` both work on the array form.
    let source = r#"## Main
Let mutable xs be a new Seq of Int.
Push 0 to xs.
Push 0 to xs.
Push 0 to xs.
Push 0 to xs.
Set item 1 of xs to 7.
Set item 4 of xs to 9.
Let mutable total be 0.
Let mutable i be 1.
While i is at most length of xs:
    Set total to total + item i of xs.
    Set i to i + 1.
Show total.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains("[i64; 4]"),
        "should scalarize to [i64; 4]. Got:\n{}",
        rust
    );
    common::assert_exact_output(source, "16");
}

#[test]
fn scalarize_composes_with_borrow_hoist() {
    // A scalarized array is never borrow-hoisted (it's not a LogosSeq) — no
    // guards for it — and the program still runs correctly.
    let source = r#"## Main
Let mutable xs be a new Seq of Int.
Push 1 to xs.
Push 2 to xs.
Push 3 to xs.
Push 4 to xs.
Push 5 to xs.
Let mutable total be 0.
Let mutable i be 1.
While i is at most 5:
    Set total to total + item i of xs.
    Set i to i + 1.
Show total.
"#;
    let rust = compile_to_rust(source).unwrap();
    assert!(rust.contains("[i64; 5]"), "should scalarize. Got:\n{}", rust);
    assert!(
        !rust.contains("__xs_g"),
        "scalarized arrays must not be borrow-hoisted. Got:\n{}",
        rust
    );
    common::assert_exact_output(source, "15");
}

// =============================================================================
// Adversarial: each disqualifier keeps the LogosSeq representation, and the
// program still runs correctly.
// =============================================================================

fn assert_not_scalarized(source: &str, expected: &str) {
    let rust = compile_to_rust(source).unwrap();
    assert!(
        !rust.contains("[i64;") && !rust.contains("[f64;") && !rust.contains("[bool;"),
        "must NOT scalarize. Got:\n{}",
        rust
    );
    common::assert_exact_output(source, expected);
}

#[test]
fn no_scalarize_push_after_init() {
    assert_not_scalarized(
        r#"## Main
Let mutable xs be a new Seq of Int.
Push 1 to xs.
Push 2 to xs.
Let mutable total be 0.
Let mutable i be 1.
While i is at most 2:
    Set total to total + item i of xs.
    Set i to i + 1.
Push 3 to xs.
Show total + length of xs.
"#,
        "6",
    );
}

#[test]
fn no_scalarize_conditional_push() {
    assert_not_scalarized(
        r#"## To native parseInt (s: Text) -> Int

## Main
Let k be parseInt("1").
Let mutable xs be a new Seq of Int.
Push 1 to xs.
If k is greater than 0:
    Push 2 to xs.
Show length of xs.
"#,
        "2",
    );
}

#[test]
fn no_scalarize_push_in_loop() {
    assert_not_scalarized(
        r#"## To native parseInt (s: Text) -> Int

## Main
Let n be parseInt("3").
Let mutable xs be a new Seq of Int.
Let mutable i be 0.
While i is less than n:
    Push i to xs.
    Set i to i + 1.
Show length of xs.
"#,
        "3",
    );
}

#[test]
fn no_scalarize_identifier_alias() {
    // `Let b be xs` aliases under reference semantics; arrays copy, so
    // scalarizing would change behavior.
    assert_not_scalarized(
        r#"## Main
Let mutable xs be a new Seq of Int.
Push 1 to xs.
Push 2 to xs.
Let mutable b be xs.
Set item 1 of b to 9.
Show item 1 of xs.
"#,
        "9",
    );
}

#[test]
fn no_scalarize_passed_to_function() {
    assert_not_scalarized(
        r#"## To total (s: Seq of Int) -> Int:
    Return item 1 of s + item 2 of s.

## Main
Let mutable xs be a new Seq of Int.
Push 4 to xs.
Push 6 to xs.
Show total(xs).
"#,
        "10",
    );
}

#[test]
fn no_scalarize_pushed_into_another_seq() {
    // Pushing xs into a Seq-of-Seq makes it an element of another collection
    // (a reference under LOGOS semantics) — arrays copy, so it must not
    // scalarize. (Read back via the container, not via xs, to avoid the
    // separate Seq-of-Seq move-after-use codegen limitation.)
    assert_not_scalarized(
        r#"## Main
Let mutable xs be a new Seq of Int.
Push 1 to xs.
Push 2 to xs.
Let mutable nest be a new Seq of (Seq of Int).
Push xs to nest.
Show item 1 of item 1 of nest.
"#,
        "1",
    );
}

#[test]
fn no_scalarize_pop_after_init() {
    assert_not_scalarized(
        r#"## Main
Let mutable xs be a new Seq of Int.
Push 1 to xs.
Push 2 to xs.
Push 3 to xs.
Pop from xs.
Show length of xs.
"#,
        "2",
    );
}

#[test]
fn no_scalarize_rebound_to_new_seq() {
    assert_not_scalarized(
        r#"## Main
Let mutable xs be a new Seq of Int.
Push 1 to xs.
Push 2 to xs.
Set xs to a new Seq of Int.
Push 9 to xs.
Show length of xs.
"#,
        "1",
    );
}

#[test]
fn no_scalarize_shown_whole() {
    assert_not_scalarized(
        r#"## Main
Let mutable xs be a new Seq of Int.
Push 1 to xs.
Push 2 to xs.
Show xs.
"#,
        "[1, 2]",
    );
}
