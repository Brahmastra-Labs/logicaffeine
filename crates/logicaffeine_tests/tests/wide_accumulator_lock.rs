//! Lock for `Opt::WideAccumulator` — i128 widening of a bounded counting accumulator.
//!
//! A `Let X be 0` accumulator that is incremented ONLY by `+1` (guarded or not), inside a SINGLE
//! counted loop whose bound is an i64, and read ONLY by `Show X`, widens to `i128` for the loop:
//! `let mut X: i128 = 0; … X = X + (cond as i128); … show(&(X as i64))`. The plain i128 add has no
//! overflow branch (unlike `logos_add_i64`), so the counting loop AUTO-VECTORIZES — the final lever
//! that lets these loops BEAT C. SOUND: X ≤ trip-count ≤ i64::MAX (the loop bound is i64), so X fits
//! i64 (the `as i64` sink is exact) and i128 cannot overflow. Byte-identical; gates off.

use logicaffeine_compile::compile::{
    compile_to_program, compile_to_rust, program_outcome, run_logos_source,
};
use logicaffeine_compile::optimization::decorate_source;

fn vm_output(src: &str) -> String {
    program_outcome(&compile_to_program(src).expect("VM compiles"))
        .output
        .trim()
        .to_string()
}
fn aot_output(src: &str) -> String {
    run_logos_source(src)
        .expect("AOT compiles and runs")
        .trim()
        .to_string()
}

/// The chars/text `is_*` benchmark shape: count qualifying codepoints, then show. `sum` is a
/// `Let 0` accumulator, `+1` under a guard, in one counted loop, read only by `Show`. i%7==0 over
/// 0..99999 → 14286.
const PROG: &str = "\
## Main
Let mutable sum be 0.
Let mutable i be 0.
While i is less than 100000:
    If (i % 7) is equal to 0:
        Set sum to sum + 1.
    Set i to i + 1.
Show sum.
";

#[test]
fn wide_accumulator_widens_to_i128() {
    let on = compile_to_rust(PROG).expect("PROG compiles");
    assert!(
        on.contains("sum: i128"),
        "the bounded counting accumulator `sum` must widen to i128; got:\n{on}"
    );
    assert!(
        !on.contains("logos_add_i64(sum"),
        "the i128 accumulator must use a plain add (no checked-add branch), not logos_add_i64; got:\n{on}"
    );
}

#[test]
fn wide_accumulator_gates_off() {
    let off = compile_to_rust(&decorate_source(PROG, &["wideaccumulator"])).expect("off compiles");
    assert!(
        !off.contains("sum: i128"),
        "`## No wideaccumulator` must keep `sum` as the i64 checked accumulator; got:\n{off}"
    );
}

/// PIT-OF-SUCCESS correctness lock: the i128 accumulator must be BYTE-IDENTICAL to the VM oracle
/// (which runs arbitrary-precision Int) AND to the AOT i64 path with the opt disabled. Catches any
/// widening/narrowing error or a misfire on an accumulator that could exceed i64.
#[test]
fn wide_accumulator_byte_identical_all_tiers_and_toggle() {
    let vm = vm_output(PROG);
    let on = aot_output(PROG);
    let off = aot_output(&decorate_source(PROG, &["wideaccumulator"]));
    assert_eq!(on, "14286", "count of i%7==0 over 0..99999 must be 14286");
    assert_eq!(
        on, vm,
        "AOT i128 accumulator diverged from the VM oracle (widen/narrow bug?)"
    );
    assert_eq!(
        on, off,
        "WideAccumulator ON changed output vs OFF — not behavior-preserving"
    );
}

/// A counter read only through an item's index (`item (v + 1) of arr`) is read, so it never widens: the affine
/// closed form turns that item into `v` itself, and an i128 `v` added to an i64 total would not build. Every read
/// counts at any depth, an index, an argument or a field's owner as surely as an operand.
const READ_THROUGH_AN_INDEX: &str = "\
## Main
Let n be 50.
Let mutable arr be a new Seq of Int.
Let mutable i be 0.
While i is less than n + 50:
    Push i to arr.
    Set i to i + 1.
Let mutable v be 0.
Let mutable total be 0.
Set i to 1.
While i is at most n:
    Set total to total + item (v + 1) of arr.
    Set v to v + 1.
    Set i to i + 1.
Show total.
Show v.
";

#[test]
fn a_counter_read_through_an_index_stays_narrow_and_builds() {
    let rust = compile_to_rust(READ_THROUGH_AN_INDEX).expect("compiles");
    assert!(
        !rust.contains("v: i128"),
        "a counter read through an index never widens:\n{rust}"
    );
    assert_eq!(
        aot_output(READ_THROUGH_AN_INDEX),
        vm_output(READ_THROUGH_AN_INDEX)
    );
}
