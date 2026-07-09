//! Phase: loop-local scratch-buffer allocation hoisting.
//!
//! A collection that is, inside a loop body, (a) freshly bound each iteration
//! by a full copy of a source slice, (b) used/mutated in place, and (c) does
//! not escape the iteration, is hoisted out of the loop and its allocation
//! reused: `let mut buf = Vec::new()` before the loop, then `buf.clear();
//! buf.extend_from_slice(&src[..])` each iteration — eliminating one heap
//! allocation per iteration (the `fannkuch` `perm` buffer: ~n! of them).
//!
//! This is value-identical to the per-iteration `.to_vec()` (clear+extend
//! reproduces exactly the copied contents) and matches C's reused-buffer
//! `memcpy`. It is a general transform (any loop-local fully-overwritten
//! non-escaping scratch buffer), gated on the de-Rc non-escape/non-alias proof.

#![cfg(not(target_arch = "wasm32"))]

mod common;

use common::{assert_exact_output, compile_to_rust};

/// The fannkuch hot loop copies `perm1` into a fresh `perm` every outer
/// iteration, flips `perm` in place, then discards it. `perm` is loop-local,
/// de-Rc'd, and never escapes → its allocation must be hoisted and reused.
/// (The args-free n=7 fannkuch; golden output 228 / 16.)
const FANNKUCH_N7: &str = r#"## Main
Let n be 7.
Let mutable perm1 be a new Seq of Int.
Let mutable count be a new Seq of Int.
Let mutable i be 0.
While i is less than n:
    Push i to perm1.
    Push 0 to count.
    Set i to i + 1.
Let mutable maxFlips be 0.
Let mutable checksum be 0.
Let mutable permCount be 0.
Let mutable r be n.
Let mutable done be 0.
While done equals 0:
    While r is greater than 1:
        Set item r of count to r.
        Set r to r - 1.
    Let mutable perm be a new Seq of Int.
    Set i to 1.
    While i is at most n:
        Push item i of perm1 to perm.
        Set i to i + 1.
    Let mutable flips be 0.
    While item 1 of perm is not 0:
        Let k be item 1 of perm + 1.
        Let mutable lo be 1.
        Let mutable hi be k.
        While lo is less than hi:
            Let tmp be item lo of perm.
            Set item lo of perm to item hi of perm.
            Set item hi of perm to tmp.
            Set lo to lo + 1.
            Set hi to hi - 1.
        Set flips to flips + 1.
    If flips is greater than maxFlips:
        Set maxFlips to flips.
    If permCount % 2 equals 0:
        Set checksum to checksum + flips.
    Otherwise:
        Set checksum to checksum - flips.
    Set permCount to permCount + 1.
    Set done to 1.
    While done equals 1:
        If r equals n:
            Set done to 2.
        Otherwise:
            Let perm0 be item 1 of perm1.
            Set i to 1.
            While i is at most r:
                Set item i of perm1 to item (i + 1) of perm1.
                Set i to i + 1.
            Set item (r + 1) of perm1 to perm0.
            Set item (r + 1) of count to (item (r + 1) of count) - 1.
            If item (r + 1) of count is greater than 0:
                Set done to 0.
            Otherwise:
                Set r to r + 1.
Show checksum.
Show maxFlips.
"#;

/// RED: the per-iteration `perm = perm1[..n].to_vec()` fresh allocation must
/// become a hoisted buffer reused via `clear()` + `extend_from_slice`.
#[test]
fn scratch_hoist_fannkuch_reuses_buffer() {
    let rust = compile_to_rust(FANNKUCH_N7).unwrap();
    assert!(
        rust.contains("perm.clear()"),
        "scratch buffer must be cleared for reuse, got:\n{rust}"
    );
    // (`n` is constant-folded to 7 in this self-contained program, so match the
    // src prefix rather than the literal bound.)
    assert!(
        rust.contains("perm.extend_from_slice(&perm1["),
        "scratch buffer must be refilled via extend_from_slice (not to_vec), got:\n{rust}"
    );
    assert!(
        !rust.contains(".to_vec()"),
        "the per-iteration fresh allocation must be eliminated, got:\n{rust}"
    );
}

/// The hoist must be value-preserving: fannkuch(7) = checksum 228, maxFlips 16.
#[test]
fn scratch_hoist_fannkuch_value_equivalence() {
    assert_exact_output(FANNKUCH_N7, "228\n16");
}

/// Generality: a non-fannkuch loop that copies `base` into a fresh `work` each
/// iteration, mutates the copy in place, reads a result, and discards it. The
/// outer loop is a COUNTED `for` (a different codegen path than fannkuch's
/// sentinel `while`), exercising the for-range hoist site. `work` is loop-local
/// and non-escaping → the same hoist must fire.
///
/// base starts [0,1,2,3,4] and base[0] increments each iteration; work copies
/// the current base, work[0] += iter:
///   iter 0: work[0]=0  total=0   base[0]->1
///   iter 1: work[0]=2  total=2   base[0]->2
///   iter 2: work[0]=4  total=6   base[0]->3
///   iter 3: work[0]=6  total=12  base[0]->4
///   iter 4: work[0]=8  total=20  base[0]->5
/// → total = 20.
const GENERIC_SCRATCH: &str = r#"## Main
Let n be 5.
Let mutable base be a new Seq of Int.
Let mutable i be 0.
While i is less than n:
    Push i to base.
    Set i to i + 1.
Let mutable total be 0.
Let mutable iter be 0.
While iter is less than n:
    Let mutable work be a new Seq of Int.
    Set i to 1.
    While i is at most n:
        Push item i of base to work.
        Set i to i + 1.
    Set item 1 of work to (item 1 of work) + iter.
    Set total to total + item 1 of work.
    Set item 1 of base to (item 1 of base) + 1.
    Set iter to iter + 1.
Show total.
"#;

#[test]
fn scratch_hoist_generalizes_non_fannkuch() {
    let rust = compile_to_rust(GENERIC_SCRATCH).unwrap();
    assert!(
        rust.contains("work.clear()") && rust.contains("work.extend_from_slice(&base["),
        "a general loop-local scratch copy must also hoist+reuse, got:\n{rust}"
    );
    assert!(
        !rust.contains(".to_vec()"),
        "the per-iteration fresh allocation must be eliminated, got:\n{rust}"
    );
}

#[test]
fn scratch_hoist_generic_value_equivalence() {
    assert_exact_output(GENERIC_SCRATCH, "20");
}

/// Soundness negative: a buffer that ESCAPES the iteration (pushed into an
/// outer collection as an element) is NOT uniquely owned (`is_de_rc` is false),
/// so the hoist must NOT fire — reuse would alias the stored copy.
const ESCAPING_SCRATCH: &str = r#"## Main
Let n be 3.
Let mutable base be a new Seq of Int.
Let mutable i be 0.
While i is less than n:
    Push i to base.
    Set i to i + 1.
Let mutable collected be a new Seq of Seq of Int.
Let mutable iter be 0.
While iter is less than n:
    Let mutable work be a new Seq of Int.
    Set i to 1.
    While i is at most n:
        Push item i of base to work.
        Set i to i + 1.
    Push work to collected.
    Set iter to iter + 1.
Show length of collected.
"#;

#[test]
fn scratch_hoist_skips_escaping_buffer() {
    // Checked at the codegen-shape level: a buffer that escapes is not de-Rc'd,
    // so the hoist must not fire. (Run-correctness of escaping nested LogosSeq in
    // a counted loop is governed by a separate codegen path, out of scope here.)
    let rust = compile_to_rust(ESCAPING_SCRATCH).unwrap();
    assert!(
        !rust.contains("work.clear()") && !rust.contains("work.extend_from_slice("),
        "an escaping (stored) buffer must not be scratch-hoisted, got:\n{rust}"
    );
}

/// Mutual exclusion: the existing buffer-reuse swap shape (`Set outer to inner`)
/// is handled by `try_emit_buffer_reuse_while` (mem::swap); the scratch hoist
/// must not interfere — the swap path still fires, not an extend_from_slice.
const BUFFER_REUSE_SWAP: &str = r#"## Main
Let n be 5.
Let mutable outer be a new Seq of Int.
Push 0 to outer.
Let mutable i be 0.
While i is less than n:
    Let mutable inner be a new Seq of Int.
    Let mutable j be 0.
    While j is less than 3:
        Push (i + 1) * (j + 1) to inner.
        Set j to j + 1.
    Set outer to inner.
    Set i to i + 1.
Show outer.
"#;

#[test]
fn scratch_hoist_skips_buffer_reuse_swap_partner() {
    let rust = compile_to_rust(BUFFER_REUSE_SWAP).unwrap();
    assert!(
        rust.contains("mem::swap"),
        "buffer-reuse swap shape must still take the mem::swap path, got:\n{rust}"
    );
    assert!(
        !rust.contains("inner.extend_from_slice("),
        "a buffer-reuse swap partner must not be scratch-hoisted, got:\n{rust}"
    );
    assert_exact_output(BUFFER_REUSE_SWAP, "[5, 10, 15]");
}
