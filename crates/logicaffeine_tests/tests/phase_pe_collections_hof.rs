//! Phase B5 — Maps / Sets / Text / Closures (PE_IMPROVE §5, closes gap G6).
//!
//! Static map/set/text operations fold; a statically-known closure is inlined at the call
//! site (higher-order specialization — the runway for EXODIA defunctionalization).
//! RED-first per CLAUDE.md.
//!
//! NOTE: programs use `"\` + real newlines + real indentation (NOT `\n\` continuation).

mod pe_support;

use pe_support::*;

// ===========================================================================
// Maps / Sets / Text folding.
// ===========================================================================

/// A read of a present key on a statically-built map folds to its value.
#[test]
fn mapget_static_hit() {
    let program = "\
## Main
Let m be a new Map of Text to Int.
Set item \"a\" of m to 1.
Set item \"b\" of m to 2.
Show item \"b\" of m.";
    let residual = decompile(program).expect("PE should not fail");
    assert!(
        residual.contains("Show 2"),
        "present-key mapget must fold to 2:\n{}",
        residual
    );
    assert_run_equals(program, "2");
}

/// `length` folds for a statically-built map.
#[test]
fn len_on_map() {
    let program = "\
## Main
Let m be a new Map of Text to Int.
Set item \"a\" of m to 1.
Set item \"b\" of m to 2.
Show length of m.";
    let residual = decompile(program).expect("PE should not fail");
    assert!(
        residual.contains("Show 2"),
        "length of a 2-entry static map must fold to 2:\n{}",
        residual
    );
    assert_run_equals(program, "2");
}

/// `length` folds for a static set.
#[test]
fn len_on_set() {
    let program = "\
## Main
Let s be a new Set of Int.
Add 5 to s.
Add 7 to s.
Show length of s.";
    let residual = decompile(program).expect("PE should not fail");
    assert!(
        residual.contains("Show 2"),
        "length of a 2-element static set must fold to 2:\n{}",
        residual
    );
    assert_run_equals(program, "2");
}

/// `length` folds for a text literal.
#[test]
fn len_on_text() {
    let program = "\
## Main
Let t be \"hello\".
Show length of t.";
    let residual = decompile(program).expect("PE should not fail");
    assert!(
        residual.contains("Show 5"),
        "length of \"hello\" must fold to 5:\n{}",
        residual
    );
    assert_run_equals(program, "5");
}

/// Membership folds true on a static set containing the element.
#[test]
fn contains_literal_membership_true() {
    let program = "\
## Main
Let s be a new Set of Int.
Add 3 to s.
Add 9 to s.
If s contains 3:
    Show \"yes\".
Otherwise:
    Show \"no\".";
    let residual = decompile(program).expect("PE should not fail");
    assert!(
        residual.contains("yes") && !residual.contains("no"),
        "membership of a present element must fold to the then-branch:\n{}",
        residual
    );
    assert_run_equals(program, "yes");
}

/// Membership folds false on a static set NOT containing the element.
#[test]
fn contains_literal_membership_false() {
    let program = "\
## Main
Let s be a new Set of Int.
Add 3 to s.
Add 9 to s.
If s contains 4:
    Show \"yes\".
Otherwise:
    Show \"no\".";
    let residual = decompile(program).expect("PE should not fail");
    assert!(
        residual.contains("no") && !residual.contains("yes"),
        "membership of an absent element must fold to the else-branch:\n{}",
        residual
    );
    assert_run_equals(program, "no");
}

// ===========================================================================
// Closures / higher-order specialization.
// ===========================================================================

/// A statically-known closure passed to a higher-order function is inlined: the residual
/// folds the whole call to a constant.
#[test]
fn closure_inlined_when_static() {
    let program = "\
## To apply (f: fn(Int) -> Int) and (x: Int) -> Int:
    Return f(x).

## Main
Show apply((n: Int) -> n * 2, 5).";
    let residual = decompile(program).expect("PE should not fail");
    assert!(
        residual.contains("Show 10"),
        "static closure applied to a static arg must fold to 10:\n{}",
        residual
    );
    assert_run_equals(program, "10");
}

/// A static closure applied to a dynamic arg specializes (the indirection is removed; the
/// closure body is inlined over the dynamic arg).
#[test]
fn closure_specialized_args() {
    let program = "\
## To apply (f: fn(Int) -> Int) and (x: Int) -> Int:
    Return f(x).

## Main
Let mutable d be 0.
Repeat for i from 1 to 100:
    Set d to d + 1.
Show apply((n: Int) -> n * 2, d).";
    // d = 100 ⇒ 100 * 2 = 200.
    assert_run_equals(program, "200");
}

/// A closure capturing a static outer variable inlines with the capture folded.
#[test]
fn closure_partial_capture() {
    let program = "\
## To apply (f: fn(Int) -> Int) and (x: Int) -> Int:
    Return f(x).

## Main
Let c be 10.
Let mutable d be 0.
Repeat for i from 1 to 100:
    Set d to d + 1.
Show apply((n: Int) -> n + c, d).";
    // d = 100, c = 10 ⇒ 110.
    assert_run_equals(program, "110");
}
