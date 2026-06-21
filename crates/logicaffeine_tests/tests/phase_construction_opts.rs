//! Construction-phase optimizations exercised by string_search's text builder:
//!
//! * **Cascade → affine fold** (`peephole::try_emit_affine_cascade`): a
//!   default-valued single-byte variable reassigned by a cascade of equality
//!   guards on the same expression collapses to one arithmetic assignment, when
//!   the oracle proves the guard expression's range is fully covered and the
//!   value map is affine.
//! * **Cursor-indexed string build** (`peephole::try_emit_indexed_string_build`):
//!   a `""`-initialised string grown only by cursor-lockstep appends in a counted
//!   loop is built into a pre-sized buffer written at the cursor (like C), instead
//!   of `String::push`.
//!
//! Both are general (no string_search specifics) and proof-gated.

#![cfg(not(target_arch = "wasm32"))]

mod common;
use common::{compile_to_rust, run_logos};

/// `ch = 'a' + (i % 5)` written as a default + four equality guards. The oracle
/// proves `i % 5 ∈ [0,4]` (i ≥ 0), so the cascade folds to arithmetic. The same
/// loop is a cursor-lockstep string build, so `out` is also built indexed.
const CASCADE_AND_BUILD: &str = r#"## Main
Let mutable out be "".
Let mutable i be 0.
While i is less than 10:
    Let mutable ch be "a".
    If i % 5 equals 1:
        Set ch to "b".
    If i % 5 equals 2:
        Set ch to "c".
    If i % 5 equals 3:
        Set ch to "d".
    If i % 5 equals 4:
        Set ch to "e".
    Set out to out + ch.
    Set i to i + 1.
Show out.
"#;

#[test]
fn cascade_folds_to_arithmetic() {
    let rust = compile_to_rust(CASCADE_AND_BUILD).unwrap();
    assert!(
        rust.contains("+ 97") && rust.contains("as u8"),
        "the 'a'+i%5 cascade must fold to an affine `(.. ) + 97` assignment. Got:\n{}",
        rust
    );
    assert!(
        !rust.contains("b'b'") && !rust.contains("b'e'"),
        "the per-branch byte assignments must be gone after folding. Got:\n{}",
        rust
    );
}

#[test]
fn indexed_string_build_presizes_and_writes_at_cursor() {
    let rust = compile_to_rust(CASCADE_AND_BUILD).unwrap();
    assert!(
        rust.contains("with_capacity") && rust.contains("set_len("),
        "a cursor-lockstep string build must presize + set_len. Got:\n{}",
        rust
    );
    assert!(
        rust.contains("as_mut_ptr().add("),
        "appends must become cursor-indexed writes, not push. Got:\n{}",
        rust
    );
    assert!(
        !rust.contains("out.push("),
        "no per-byte String::push should remain. Got:\n{}",
        rust
    );
}

#[test]
fn cascade_and_build_are_correct() {
    let r = run_logos(CASCADE_AND_BUILD);
    assert!(r.success, "run failed: {}\n{}", r.stderr, r.rust_code);
    assert_eq!(r.stdout.trim(), "abcdeabcde");
}

/// Multi-byte literal append (`+ "XX"`) paired with `+= 2`, interleaved with a
/// single-byte append paired with `+= 1` — exercises the variable-stride cursor.
const MIXED_STRIDE_BUILD: &str = r#"## Main
Let mutable out be "".
Let mutable i be 0.
While i is less than 6:
    If i % 2 equals 0:
        Set out to out + "XX".
        Set i to i + 2.
    If i % 2 equals 1:
        Set out to out + "y".
        Set i to i + 1.
Show out.
"#;

#[test]
fn mixed_stride_build_is_correct() {
    // i=0:"XX"(i->2), i=2:"XX"(i->4), i=4:"XX"(i->6) -> "XXXXXX".
    let r = run_logos(MIXED_STRIDE_BUILD);
    assert!(r.success, "run failed: {}\n{}", r.stderr, r.rust_code);
    assert_eq!(r.stdout.trim(), "XXXXXX");
    let rust = compile_to_rust(MIXED_STRIDE_BUILD).unwrap();
    assert!(
        rust.contains("copy_nonoverlapping"),
        "multi-byte literal append must lower to a cursor copy. Got:\n{}",
        rust
    );
}

/// SOUNDNESS: when the string is READ mid-build (`length of out` inside the
/// loop), the pre-sized buffer would be observed half-filled — the transform
/// must decline and fall back to ordinary append codegen (still correct).
const READ_MID_BUILD: &str = r#"## Main
Let mutable out be "".
Let mutable total be 0.
Let mutable i be 0.
While i is less than 5:
    Set out to out + "a".
    Set total to total + length of out.
    Set i to i + 1.
Show total.
"#;

#[test]
fn declines_when_string_read_mid_build() {
    let rust = compile_to_rust(READ_MID_BUILD).unwrap();
    assert!(
        !rust.contains("as_mut_ptr().add("),
        "must NOT use the cursor-indexed build when the string is read mid-loop. Got:\n{}",
        rust
    );
    // 1+2+3+4+5 = 15
    let r = run_logos(READ_MID_BUILD);
    assert!(r.success, "run failed: {}\n{}", r.stderr, r.rust_code);
    assert_eq!(r.stdout.trim(), "15");
}

/// SOUNDNESS: an unpaired cursor advance (counter bumped without a matching
/// append) breaks the `cursor == bytes-written` invariant; the transform must
/// decline.
const UNPAIRED_CURSOR: &str = r#"## Main
Let mutable out be "".
Let mutable i be 0.
While i is less than 5:
    Set out to out + "a".
    Set i to i + 2.
Show out.
"#;

#[test]
fn declines_on_unpaired_cursor_advance() {
    let rust = compile_to_rust(UNPAIRED_CURSOR).unwrap();
    assert!(
        !rust.contains("as_mut_ptr().add("),
        "append (+1 byte) paired with a +2 cursor advance must NOT lower to indexed build. Got:\n{}",
        rust
    );
    // i=0:"a"(i->2), i=2:"a"(i->4), i=4:"a"(i->6) -> "aaa"
    let r = run_logos(UNPAIRED_CURSOR);
    assert!(r.success, "run failed: {}\n{}", r.stderr, r.rust_code);
    assert_eq!(r.stdout.trim(), "aaa");
}

/// SOUNDNESS: a cascade whose value map is NOT affine over the proven range must
/// not fold (it would compute the wrong character). Here {0→a,1→z,2→c} is not
/// linear, so the cascade stays.
const NON_AFFINE_CASCADE: &str = r#"## Main
Let mutable out be "".
Let mutable i be 0.
While i is less than 3:
    Let mutable ch be "a".
    If i % 3 equals 1:
        Set ch to "z".
    If i % 3 equals 2:
        Set ch to "c".
    Set out to out + ch.
    Set i to i + 1.
Show out.
"#;

#[test]
fn non_affine_cascade_stays_and_is_correct() {
    // i=0->a, 1->z, 2->c => "azc"
    let r = run_logos(NON_AFFINE_CASCADE);
    assert!(r.success, "run failed: {}\n{}", r.stderr, r.rust_code);
    assert_eq!(r.stdout.trim(), "azc");
}
