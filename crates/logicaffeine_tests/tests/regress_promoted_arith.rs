//! Regression: a promoted `LogosInt` (the overflow-safe Int the numeric-tower campaign introduced)
//! divided / modulo'd / multiplied by a literal must route through the exact helpers in AOT
//! codegen, NOT emit a raw `LogosInt / N` (which has no operator impl → `error[E0369]`). The
//! campaign wired multiply/add/floor-divide through the exact helpers but left three raw-operator
//! fast-paths (safe-literal `/`%`, oracle-proven-in-range, and fast-libdivide) unguarded, so a
//! promoted operand slipped through and the generated Rust failed to compile — killing every
//! crypto/PQC AOT test that does integer division or modulo (sha3, chacha20, mlkem, mldsa, …).
//!
//! The generated line that failed, verbatim, was:
//!   `signBits = logicaffeine_data::LogosInt::from((signBits.clone() / 2));`  // E0369

mod common;

use logicaffeine_compile::compile::tw_outcome;

/// Compile `prog` through the AOT path and require (a) it compiles at all — the E0369 gate — and
/// (b) its output equals the tree-walker's (the oracle), so the exact-helper routing is not only
/// compilable but numerically correct.
fn aot_matches_treewalker(prog: &str) {
    let tw = tw_outcome(prog);
    assert_eq!(tw.error, None, "tree-walker must run cleanly: {:?}", tw.error);
    let aot = common::run_logos_with_args(prog, &[]);
    assert!(
        aot.success,
        "AOT compile FAILED on promoted-int arithmetic (the LogosInt raw-operator bug):\n\
         --- stderr ---\n{}\n--- generated rust ---\n{}",
        aot.stderr, aot.rust_code
    );
    assert_eq!(aot.stdout.trim(), tw.output.trim(), "AOT output must match the tree-walker exactly");
}

// A doubling additive accumulator (`x = x + x`) is exactly the shape `bigint_promote` promotes to
// `LogosInt`. After the loop `x` is a `LogosInt`, so the following op is `LogosInt <op> literal`.

#[test]
fn promoted_int_divided_by_literal() {
    aot_matches_treewalker(
        "Let x be 1.\nRepeat for i from 1 to 50:\n    Set x to x + x.\nShow x / 2.\n",
    );
}

#[test]
fn promoted_int_modulo_literal() {
    aot_matches_treewalker(
        "Let x be 1.\nRepeat for i from 1 to 50:\n    Set x to x + x.\nShow x % 7.\n",
    );
}

#[test]
fn promoted_int_multiplied_by_literal() {
    // Multiply hits the oracle-proven-in-range raw path, distinct from the `/`%` safe-literal path.
    aot_matches_treewalker(
        "Let x be 1.\nRepeat for i from 1 to 40:\n    Set x to x + x.\nShow x * 3.\n",
    );
}

#[test]
fn promoted_self_halving_reassignment() {
    // The exact crypto shape: reassigning a promoted variable by division —
    // `Set x to x / 2` generated `signBits = LogosInt::from((signBits.clone() / 2))`.
    aot_matches_treewalker(
        "Let x be 1.\nRepeat for i from 1 to 50:\n    Set x to x + x.\nRepeat for j from 1 to 10:\n    Set x to x / 2.\nShow x.\n",
    );
}

#[test]
fn inline_bignum_call_results_added() {
    // The ML-KEM CBD shape: two bignum-RETURNING calls added inline —
    // `mlkemBit(buf, 4*c) + mlkemBit(buf, 4*c + 1)` generated a raw `LogosInt + LogosInt`.
    // The operand is a CALL (not a variable), so the detector must recognise bignum-returning
    // functions, not just promoted variables. `grow` is promoted (doubling accumulator), so it
    // returns `LogosInt`; adding two of its results inline must route through `logos_add_exact`.
    aot_matches_treewalker(
        "## To grow (n: Int) -> Int:\n    Let x be n.\n    Repeat for i from 1 to 50:\n        Set x to x + x.\n    Return x.\n\nShow grow(1) + grow(1).\n",
    );
}

#[test]
fn inline_bignum_call_result_divided() {
    // A bignum-returning call divided by a literal — the call-operand twin of the `/`% fix.
    aot_matches_treewalker(
        "## To grow (n: Int) -> Int:\n    Let x be n.\n    Repeat for i from 1 to 50:\n        Set x to x + x.\n    Return x.\n\nShow grow(1) / 4.\n",
    );
}

#[test]
fn promoted_index_written_through_setindex() {
    // The NTT butterfly shape (ntt_logos AOT race): `idx` derives from a promoted `len`
    // (loop-carried doubling), and is used both to READ (`item (idx + 1) of a` — narrowed via
    // `.expect_i64` by the index-context emitter) and to WRITE (`Set item (idx + 1) of a to …`).
    // The write path's index went through the type-blind `simplify_1based_index`, emitting a raw
    // `LogosInt` into `as usize` / `+ i64` positions → E0605/E0369.
    aot_matches_treewalker(
        "Let mutable a be [1, 2, 3, 4, 5, 6, 7, 8].\n\
         Let mutable len be 2.\n\
         Repeat for stage from 1 to 3:\n\
         \x20   Let half be len / 2.\n\
         \x20   Let m be 8 / len.\n\
         \x20   Repeat for blk from 0 to m - 1:\n\
         \x20       Let start be blk * len.\n\
         \x20       Repeat for j from 0 to half - 1:\n\
         \x20           Let idx be start + j.\n\
         \x20           Let u be item (idx + 1) of a.\n\
         \x20           Let t be item (idx + half + 1) of a.\n\
         \x20           Set item (idx + 1) of a to u + t.\n\
         \x20           Set item (idx + half + 1) of a to u - t.\n\
         \x20   Set len to len * 2.\n\
         Show item 1 of a.\n",
    );
}

#[test]
fn word_wrapping_on_bare_loop_counter() {
    // Distinct from the LogosInt bug: a loop counter used ONLY inside a `wordN(...)` wrapping op is
    // an ambiguous `{integer}` in the emitted Rust, and `.wrapping_add` exists on every int type →
    // E0689. The wrapping receiver must be pinned to i64. (The ML-KEM lane-xor `typed` program hit
    // this.) The result is unchanged (word32-truncated), so AOT must still match the tree-walker.
    aot_matches_treewalker("Repeat for i from 1 to 8:\n    Show word32(i + 9).\n");
}
