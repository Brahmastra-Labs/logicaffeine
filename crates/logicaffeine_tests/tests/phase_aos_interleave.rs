//! Co-indexed array interleaving (struct-of-arrays → array-of-structs).
//!
//! When N scalarizable fixed arrays of the SAME element type are built by an
//! interleaved (round-robin) push pattern — `Push x to ax; Push y to ay;
//! Push z to az;` repeated per entity — they are the columns of an
//! array-of-structs. Fusing them into one `[[T; W]; N]` backing array makes the
//! per-entity fields memory-adjacent, so LLVM packs them with `movupd` instead
//! of gathering separate arrays with shuffles (the 117-shuffle tax measured on
//! nbody). This is exactly C's `struct Body { x, y, z, … } bodies[N]` layout.
//!
//! GENERAL over W (number of co-indexed arrays) and N (length) — not specialized
//! to any benchmark. Value-preserving: a pure layout change, identical f64 bits.
//!
//! Structural tests feed RUNTIME data via `args()` and read inside a
//! runtime-bounded loop so the arrays cannot be constant-folded away — isolating
//! the interleaving transform as the only thing that can produce the `[[T; W]; N]`
//! backing. Correctness tests assert exact output.

mod common;

use common::{assert_exact_output, compile_to_rust};

const RUNTIME_PRELUDE: &str = "## To native args () -> Seq of Text\n\
## To native parseInt (s: Text) -> Int\n\n\
## Main\n\
Let arguments be args().\n\
Let seed be parseInt(item 2 of arguments).\n";

// =============================================================================
// (a) Three co-indexed float arrays fuse into one [[f64; 3]; N] backing.
// =============================================================================

#[test]
fn interleave_three_float_arrays() {
    let source = format!(
        "{}{}",
        RUNTIME_PRELUDE,
        r#"Let mutable ax be a new Seq of Float.
Let mutable ay be a new Seq of Float.
Let mutable az be a new Seq of Float.
Push 1.0 to ax. Push 2.0 to ay. Push 3.0 to az.
Push 4.0 to ax. Push 5.0 to ay. Push 6.0 to az.
Let mutable total be 0.0.
Let mutable i be 1.
While i is at most seed:
    Set total to total + item i of ax + item i of ay + item i of az.
    Set i to i + 1.
Show "{total:.1}".
"#
    );
    let rust = compile_to_rust(&source).unwrap();
    assert!(
        rust.contains("[[f64; 3]; 2]"),
        "three co-indexed float arrays should fuse into one [[f64; 3]; 2] AoS backing. Got:\n{rust}"
    );
}

// =============================================================================
// (b) Generality over W: two co-indexed arrays → [[f64; 2]; N].
// =============================================================================

#[test]
fn interleave_two_float_arrays() {
    let source = format!(
        "{}{}",
        RUNTIME_PRELUDE,
        r#"Let mutable px be a new Seq of Float.
Let mutable py be a new Seq of Float.
Push 1.0 to px. Push 2.0 to py.
Push 3.0 to px. Push 4.0 to py.
Push 5.0 to px. Push 6.0 to py.
Let mutable total be 0.0.
Let mutable i be 1.
While i is at most seed:
    Set total to total + item i of px + item i of py.
    Set i to i + 1.
Show "{total:.1}".
"#
    );
    let rust = compile_to_rust(&source).unwrap();
    assert!(
        rust.contains("[[f64; 2]; 3]"),
        "two co-indexed float arrays of length 3 should fuse into [[f64; 2]; 3]. Got:\n{rust}"
    );
}

// =============================================================================
// (c) Value-preserving: the fused layout computes the same result.
// =============================================================================

#[test]
fn interleave_preserves_value() {
    // (1+2+3) + (4+5+6) = 6 + 15 = 21.0
    let source = r#"## Main
Let mutable ax be a new Seq of Float.
Let mutable ay be a new Seq of Float.
Let mutable az be a new Seq of Float.
Push 1.0 to ax. Push 2.0 to ay. Push 3.0 to az.
Push 4.0 to ax. Push 5.0 to ay. Push 6.0 to az.
Let mutable total be 0.0.
Let mutable i be 1.
While i is at most 2:
    Set total to total + item i of ax + item i of ay + item i of az.
    Set i to i + 1.
Show "{total:.1}".
"#;
    assert_exact_output(source, "21.0");
}

// =============================================================================
// (d) Index/SetIndex round-trip through the fused layout is correct.
// =============================================================================

#[test]
fn interleave_set_index_correct() {
    // SetIndex + Index through the fused backing, all variable-indexed (the
    // rolled regime where AoS fires). ax[i] += ay[i], then sum ax.
    let source = r#"## Main
Let mutable ax be a new Seq of Float.
Let mutable ay be a new Seq of Float.
Push 10.0 to ax. Push 30.0 to ay.
Push 40.0 to ax. Push 40.0 to ay.
Let mutable i be 1.
While i is at most 2:
    Set item i of ax to item i of ax + item i of ay.
    Set i to i + 1.
Let mutable total be 0.0.
Let mutable j be 1.
While j is at most 2:
    Set total to total + item j of ax.
    Set j to j + 1.
Show "{total:.1}".
"#;
    // ax = [10+30, 40+40] = [40, 80]; total = 40 + 80 = 120.0
    assert_exact_output(source, "120.0");
}

// =============================================================================
// (e) NEGATIVE: arrays NOT pushed round-robin are not a co-indexed group.
// =============================================================================

#[test]
fn block_pushed_arrays_not_fused() {
    let source = format!(
        "{}{}",
        RUNTIME_PRELUDE,
        r#"Let mutable ax be a new Seq of Float.
Let mutable ay be a new Seq of Float.
Push 1.0 to ax. Push 4.0 to ax.
Push 2.0 to ay. Push 5.0 to ay.
Let mutable total be 0.0.
Let mutable i be 1.
While i is at most seed:
    Set total to total + item i of ax + item i of ay.
    Set i to i + 1.
Show "{total:.1}".
"#
    );
    let rust = compile_to_rust(&source).unwrap();
    assert!(
        !rust.contains("[[f64"),
        "block-pushed (non-interleaved) arrays must NOT be fused into an AoS. Got:\n{rust}"
    );
}

// =============================================================================
// (f) NEGATIVE: a mixed element-type group is not fused (homogeneous only).
// =============================================================================

#[test]
fn mixed_type_arrays_not_fused() {
    let source = format!(
        "{}{}",
        RUNTIME_PRELUDE,
        r#"Let mutable ax be a new Seq of Float.
Let mutable ac be a new Seq of Int.
Push 1.0 to ax. Push 7 to ac.
Push 4.0 to ax. Push 8 to ac.
Let mutable total be 0.0.
Let mutable i be 1.
While i is at most seed:
    Set total to total + item i of ax.
    Set i to i + 1.
Show "{total:.1}".
"#
    );
    let rust = compile_to_rust(&source).unwrap();
    assert!(
        !rust.contains("[[f64") && !rust.contains("[[i64"),
        "a mixed float/int group must NOT be fused (homogeneous backing only). Got:\n{rust}"
    );
}
