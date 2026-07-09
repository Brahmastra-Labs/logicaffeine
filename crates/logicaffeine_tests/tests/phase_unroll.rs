//! Loop unrolling — full unrolling of small, compile-time-constant trip-count
//! loops that are NESTED inside a hot outer loop and index a collection.
//!
//! LLVM already fully unrolls and SIMD-vectorizes top-level constant loops on
//! its own (that is why nbody's one-time `energy` block emits packed `vsqrtpd`),
//! so we leave those alone. It declines, however, to unroll a constant-trip
//! loop nested inside a runtime loop (nbody's per-step `advance`) — so that
//! body stays scalar and its `[f64; N]` arrays never get SROA'd. This pass
//! closes exactly that gap: it expands the nested constant loop into
//! straight-line statements with the induction variable replaced by literals.
//! After folding, `item (3-1) of bx` becomes a constant index `bx[2]`, and LLVM
//! SROAs the array into registers and vectorizes the straight-line arithmetic.
//!
//! The transform is purely value-preserving: it reorders nothing and changes no
//! float-operation grouping, so numeric output stays bit-identical (FMA /
//! reassociation are forbidden — they would change rounding).
//!
//! Structural tests feed RUNTIME data via `args()` so the existing
//! CTFE/PE/fold passes cannot fold the loop away — isolating the unroller as
//! the only thing that can remove the inner loop. They nest the constant loop
//! inside a runtime-bounded outer loop so the nesting precondition holds.
//! Correctness tests assert exact output and are robust whether or not
//! unrolling fires.

mod common;

use common::{assert_exact_output, compile_to_rust};

/// Two native decls + a runtime seed read from argv. The seed is unknowable at
/// compile time, so the optimizer cannot constant-fold the loop body and the
/// outer `While ... at most seed` loop cannot be unrolled — only the inner
/// constant loop can be removed, and only by the unroller.
const RUNTIME_PRELUDE: &str = "## To native args () -> Seq of Text\n\
## To native parseInt (s: Text) -> Int\n\n\
## Main\n\
Let arguments be args().\n\
Let seed be parseInt(item 2 of arguments).\n";

// =============================================================================
// (a) A constant-trip `While` nested in a runtime loop unrolls to straight-line
//     constant-index code; the runtime outer loop is untouched.
// =============================================================================

#[test]
fn unroll_nested_while_constant_bound() {
    let source = format!(
        "{}{}",
        RUNTIME_PRELUDE,
        r#"Let mutable xs be a new Seq of Int.
Push seed to xs.
Push seed + 1 to xs.
Push seed + 2 to xs.
Let mutable acc be 0.
Let mutable outer be 1.
While outer is at most seed:
    Let mutable i be 1.
    While i is at most 3:
        Set acc to acc + item i of xs.
        Set i to i + 1.
    Set outer to outer + 1.
Show acc.
"#
    );
    let rust = compile_to_rust(&source).unwrap();
    assert!(
        !rust.contains("for i in"),
        "the nested constant-trip inner loop should unroll — no `for i in` should remain. Got:\n{rust}"
    );
    assert!(
        rust.contains("xs[0") && rust.contains("xs[1") && rust.contains("xs[2"),
        "unrolled body should index xs[0], xs[1], xs[2] directly. Got:\n{rust}"
    );
}

// =============================================================================
// (b) A nested `Repeat for i from 1 to 3` (the Range path) unrolls.
// =============================================================================

#[test]
fn unroll_nested_repeat_range() {
    let source = format!(
        "{}{}",
        RUNTIME_PRELUDE,
        r#"Let mutable xs be a new Seq of Int.
Push seed to xs.
Push seed + 1 to xs.
Push seed + 2 to xs.
Let mutable acc be 0.
Let mutable outer be 1.
While outer is at most seed:
    Repeat for i from 1 to 3:
        Set acc to acc + item i of xs.
    Set outer to outer + 1.
Show acc.
"#
    );
    let rust = compile_to_rust(&source).unwrap();
    assert!(
        !rust.contains("for i in"),
        "the nested constant-trip Repeat-range should unroll — no `for i in` should remain. Got:\n{rust}"
    );
    assert!(
        rust.contains("xs[0") && rust.contains("xs[1") && rust.contains("xs[2"),
        "unrolled Repeat body should index xs[0], xs[1], xs[2] directly. Got:\n{rust}"
    );
}

// =============================================================================
// (c) Nested triangular constant loops (the nbody pairwise shape) fully unroll.
// =============================================================================

#[test]
fn unroll_nested_triangular() {
    let source = format!(
        "{}{}",
        RUNTIME_PRELUDE,
        r#"Let mutable xs be a new Seq of Int.
Push seed to xs.
Push seed + 1 to xs.
Push seed + 2 to xs.
Let mutable acc be 0.
Let mutable step be 1.
While step is at most seed:
    Let mutable i be 1.
    While i is at most 3:
        Let mutable j be i + 1.
        While j is at most 3:
            Set acc to acc + item i of xs * item j of xs.
            Set j to j + 1.
        Set i to i + 1.
    Set step to step + 1.
Show acc.
"#
    );
    let rust = compile_to_rust(&source).unwrap();
    assert!(
        !rust.contains("for i in") && !rust.contains("for j in"),
        "both nested triangular constant loops should fully unroll. Got:\n{rust}"
    );
}

// =============================================================================
// (d) nbody-shape: parallel float SoA arrays, a constant-bound triangular force
//     loop nested in a runtime step loop, RUNTIME-driven iteration count — must
//     compile to constant-index accesses with no inner loop. The load-bearing
//     pin. (The float DATA is constant, but the per-step mutation count is
//     runtime, so the optimizer cannot fold the result away.)
// =============================================================================

#[test]
fn unroll_nbody_force_shape_constant_indices() {
    let source = format!(
        "{}{}",
        RUNTIME_PRELUDE,
        r#"Let mutable bx be a new Seq of Float.
Let mutable bvx be a new Seq of Float.
Let mutable bm be a new Seq of Float.
Push 1.0 to bx. Push 2.0 to bx. Push 3.0 to bx.
Push 0.0 to bvx. Push 0.0 to bvx. Push 0.0 to bvx.
Push 0.5 to bm. Push 0.5 to bm. Push 0.5 to bm.
Let mutable step be 1.
While step is at most seed:
    Let mutable i be 1.
    While i is at most 3:
        Let mutable j be i + 1.
        While j is at most 3:
            Let dx be item i of bx - item j of bx.
            Set item i of bvx to item i of bvx - dx * item j of bm.
            Set item j of bvx to item j of bvx + dx * item i of bm.
            Set j to j + 1.
        Set i to i + 1.
    Set step to step + 1.
Show "{item 1 of bvx:.3}".
"#
    );
    let rust = compile_to_rust(&source).unwrap();
    assert!(
        rust.contains("[f64; 3]"),
        "parallel fixed float Seqs should scalarize to [f64; 3]. Got:\n{rust}"
    );
    assert!(
        !rust.contains("for i in") && !rust.contains("for j in"),
        "the constant-bound triangular force loop must fully unroll (no inner loops). Got:\n{rust}"
    );
    assert!(
        rust.contains("bvx[0") && rust.contains("bvx[1") && rust.contains("bvx[2"),
        "unrolled force body should write bvx at constant indices 0,1,2. Got:\n{rust}"
    );
}

// =============================================================================
// (e) A nested loop with a RUNTIME bound is NOT unrolled — soundness guard.
// =============================================================================

#[test]
fn runtime_bound_not_unrolled() {
    let source = format!(
        "{}{}",
        RUNTIME_PRELUDE,
        r#"Let mutable xs be a new Seq of Int.
Push seed to xs.
Let mutable acc be 0.
Let mutable outer be 1.
While outer is at most seed:
    Let mutable i be 1.
    While i is at most seed:
        Set acc to acc + item 1 of xs.
        Set i to i + 1.
    Set outer to outer + 1.
Show acc.
"#
    );
    let rust = compile_to_rust(&source).unwrap();
    assert!(
        rust.contains("for i in") || rust.contains("while i"),
        "a loop with a runtime bound must NOT be unrolled — a loop must remain. Got:\n{rust}"
    );
}

// =============================================================================
// (f) A nested loop whose trip count exceeds the threshold is NOT unrolled.
// =============================================================================

#[test]
fn over_threshold_not_unrolled() {
    let source = format!(
        "{}{}",
        RUNTIME_PRELUDE,
        r#"Let mutable xs be a new Seq of Int.
Push seed to xs.
Let mutable acc be 0.
Let mutable outer be 1.
While outer is at most seed:
    Let mutable i be 1.
    While i is at most 100:
        Set acc to acc + item 1 of xs.
        Set i to i + 1.
    Set outer to outer + 1.
Show acc.
"#
    );
    let rust = compile_to_rust(&source).unwrap();
    assert!(
        rust.contains("for i in") || rust.contains("while i"),
        "a 100-iteration loop exceeds the unroll threshold and must stay rolled. Got:\n{rust}"
    );
}

// =============================================================================
// (g) A top-level constant loop is left to LLVM (not unrolled by us) — this
//     keeps the existing scalarization/peephole codegen contracts intact.
// =============================================================================

#[test]
fn top_level_constant_loop_not_unrolled_by_us() {
    let source = format!(
        "{}{}",
        RUNTIME_PRELUDE,
        r#"Let mutable xs be a new Seq of Int.
Push seed to xs.
Push seed + 1 to xs.
Push seed + 2 to xs.
Let mutable acc be 0.
Let mutable i be 1.
While i is at most 3:
    Set acc to acc + item i of xs.
    Set i to i + 1.
Show acc.
"#
    );
    let rust = compile_to_rust(&source).unwrap();
    // The for-range peephole still converts the top-level while to a `for`.
    assert!(
        rust.contains("for i in"),
        "a top-level constant loop is LLVM's job — we must leave the `for i in` form. Got:\n{rust}"
    );
}

// =============================================================================
// Correctness — value-preserving (exact output). Nested in a once-running outer
// loop so the inner constant loop is exercised by the unroller.
// =============================================================================

#[test]
fn unroll_nested_correct_value() {
    let source = r#"## Main
Let mutable xs be a new Seq of Int.
Push 10 to xs.
Push 20 to xs.
Push 30 to xs.
Let mutable acc be 0.
Let mutable outer be 1.
While outer is at most 1:
    Let mutable i be 1.
    While i is at most 3:
        Set acc to acc + item i of xs.
        Set i to i + 1.
    Set outer to outer + 1.
Show acc.
"#;
    assert_exact_output(source, "60");
}

#[test]
fn unroll_nested_triangular_correct_value() {
    // Pairwise products over [10,20,30]: 10*20 + 10*30 + 20*30 = 1100.
    let source = r#"## Main
Let mutable xs be a new Seq of Int.
Push 10 to xs.
Push 20 to xs.
Push 30 to xs.
Let mutable acc be 0.
Let mutable outer be 1.
While outer is at most 1:
    Let mutable i be 1.
    While i is at most 3:
        Let mutable j be i + 1.
        While j is at most 3:
            Set acc to acc + item i of xs * item j of xs.
            Set j to j + 1.
        Set i to i + 1.
    Set outer to outer + 1.
Show acc.
"#;
    assert_exact_output(source, "1100");
}

#[test]
fn unroll_float_order_preserved() {
    // Subtraction is non-associative; unrolling must preserve the exact
    // left-to-right accumulation order, bit-identical to the rolled loop.
    let source = r#"## Main
Let mutable xs be a new Seq of Float.
Push 0.1 to xs.
Push 0.2 to xs.
Push 0.3 to xs.
Let mutable acc be 1.0.
Let mutable outer be 1.
While outer is at most 1:
    Let mutable i be 1.
    While i is at most 3:
        Set acc to acc - item i of xs.
        Set i to i + 1.
    Set outer to outer + 1.
Show "{acc:.17}".
"#;
    let expected = format!("{:.17}", 1.0f64 - 0.1 - 0.2 - 0.3);
    assert_exact_output(source, &expected);
}

// =============================================================================
// Guards — must not miscompile or alter semantics.
// =============================================================================

#[test]
fn break_loop_not_miscompiled() {
    // A loop that breaks mid-iteration must NOT be unrolled (a `break` outside a
    // loop is meaningless). It stays a real loop and runs correctly.
    let source = r#"## Main
Let mutable xs be a new Seq of Int.
Push 10 to xs.
Push 20 to xs.
Push 30 to xs.
Let mutable acc be 0.
Let mutable outer be 1.
While outer is at most 1:
    Let mutable i be 1.
    While i is at most 3:
        Set acc to acc + item i of xs.
        If i is at least 2:
            Break.
        Set i to i + 1.
    Set outer to outer + 1.
Show acc.
"#;
    // i=1 -> acc=10; i=2 -> acc=30, break. Expect 30.
    assert_exact_output(source, "30");
}

#[test]
fn empty_trip_loop_drops_cleanly() {
    // i starts at 1, bound is 0 -> zero iterations -> body never runs.
    let source = r#"## Main
Let mutable xs be a new Seq of Int.
Push 5 to xs.
Let mutable acc be 7.
Let mutable outer be 1.
While outer is at most 1:
    Let mutable i be 1.
    While i is at most 0:
        Set acc to acc + item 1 of xs.
        Set i to i + 1.
    Set outer to outer + 1.
Show acc.
"#;
    assert_exact_output(source, "7");
}
