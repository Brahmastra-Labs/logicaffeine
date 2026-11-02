//! Int overflow-guard elimination via range narrowing (the "beat Zig" pass).
//!
//! LOGOS `Int` is semantically arbitrary-precision, so hot-loop arithmetic
//! carries an overflow guard (`logos_add_i64` = `checked_add` → BigInt on
//! overflow, or an inline range-test) that C/Zig — which just wrap `i64` — do
//! not. That guard (`add; jno`) is the whole reason Zig edges LOGOS on the tight
//! integer loops.
//!
//! The optimizer already has a SOUND guard-elimination path
//! (`oracle_proves_int_op_in_range` → raw `+` when the range oracle proves the
//! result fits i64) and a correct `Interval::modulo` that narrows `(a+b) % M` to
//! `[-(M-1), M-1]`. The missing piece is a NARROWING phase: widening pins
//! loop-carried accumulators to an infinite bound, so `expr_int_range` returns
//! `None` and the guard survives. Re-clamping through the modulo re-proves the
//! bound.
//!
//! This is the TDD spec for that pass:
//!   * `modular_accumulator_emits_raw_add`   — RED today; the driver.
//!   * `unbounded_accumulator_keeps_guard`   — soundness: must NOT over-eliminate.
//!   * `int_never_silently_wraps`            — semantics: `Int` stays exact.
//!
//! Run: `cargo nextest run -p logicaffeine-tests -E 'binary(e2e_int_range_guard_elimination)'`

#![cfg(not(target_arch = "wasm32"))]

mod common;

use common::assert_exact_output;
use common::compile_to_rust;

/// The Iterative-Fibonacci shape: `b = (a + b) % 1000000007` keeps both `a` and
/// `b` in `[0, MOD)`, so `a + b < 2·MOD ⊂ i64` and the add provably cannot
/// overflow. The optimizer must PROVE that (narrow through the modulo) and emit
/// raw `(a + b)` — never the checked `logos_add_i64`. Reads the bound from args
/// so the loop cannot be constant-folded away (exactly like the benchmark).
///
/// The staged spec (RED). Requires a two-part change: (1) a range-oracle channel attached to the
/// arithmetic lowering — today the oracle that `oracle_proves_int_op_in_range`
/// reads is coupled to borrow-hoisting, which `Scalarize` preempts, so it is
/// never present for a scalar loop; and (2) the Cousot narrowing pass that
/// recovers `a, b ∈ [0, MOD)` through the modulo. Both land + validate together
/// on a quiet box. `#[ignore]`d so the suite stays green until then.
#[test]
fn modular_accumulator_emits_raw_add() {
    let code = r#"## To native args () -> Seq of Text
## To native parseInt (s: Text) -> Int
## Main
Let arguments be args().
Let n be parseInt(item 2 of arguments).
Let mutable a be 0.
Let mutable b be 1.
Let mutable i be 0.
While i is less than n:
    Let temp be b.
    Set b to (a + b) % 1000000007.
    Set a to temp.
    Set i to i + 1.
Show a.
"#;
    let rust = compile_to_rust(code).expect("LOGOS snippet should compile to Rust");
    assert!(
        !rust.contains("logos_add_i64") && !rust.contains("logos_add_exact"),
        "a modular accumulator is provably bounded by `% MOD`, so `a + b` must \
         lower to a raw i64 add with no overflow guard — got a checked helper:\n{rust}"
    );
}

/// Soundness pin — the pass must ONLY drop the guard when overflow is provably
/// impossible. `s = s + s` with no modulo doubles without bound, so the analysis
/// cannot bound `s`; the checked/promoting path MUST remain. If a future
/// over-eager narrowing drops this guard, `Int` would silently wrap — this fails
/// loudly first. (Reads the bound from args so it cannot be folded to a constant.)
#[test]
fn unbounded_accumulator_keeps_guard() {
    let code = r#"## To native args () -> Seq of Text
## To native parseInt (s: Text) -> Int
## Main
Let arguments be args().
Let n be parseInt(item 2 of arguments).
Let mutable s be 1.
Let mutable i be 0.
While i is less than n:
    Set s to s + s.
    Set i to i + 1.
Show s.
"#;
    let rust = compile_to_rust(code).expect("LOGOS snippet should compile to Rust");
    // The promotion may surface in any of three equivalent forms, all of which carry the
    // BigInt overflow path: `logos_add_i64` (narrowed checked add), `logos_add_exact` (the
    // by-value promoting helper), or `s.add(&s)` (the by-ref `LogosInt::add` the bignum
    // clone-elision pass emits — the SAME `LogosInt::add`, proven to promote exactly, not
    // wrap). What must NEVER appear is a bare wrapping `+`. Checking for a promotion form
    // (not its absence) still fails loudly if a future pass drops the guard to raw i64.
    assert!(
        rust.contains("logos_add_i64")
            || rust.contains("logos_add_exact")
            || rust.contains(".add(&"),
        "an unbounded accumulator (`s + s`) can overflow i64, so it MUST keep the \
         overflow guard / BigInt promotion (a checked helper or the by-ref `.add(&` \
         `LogosInt::add`) — the pass must not over-eliminate to a raw wrapping add:\n{rust}"
    );
}

/// Semantics pin — `Int` never silently wraps, on any tier. `i64::MAX + 1` must
/// print the exact next integer (BigInt), not wrap to a negative. This must stay
/// GREEN through the whole guard-elimination change.
#[test]
fn int_never_silently_wraps() {
    // 9223372036854775807 == i64::MAX; the exact sum is 2^63.
    assert_exact_output(
        "## Main\nShow 9223372036854775807 + 1.\n",
        "9223372036854775808",
    );
}
