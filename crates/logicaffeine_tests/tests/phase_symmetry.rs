//! Reflection symmetry-breaking on the AOT path (kernel-proved).
//!
//! `optimize::symmetry` halves an N-Queens first-row enumeration (×2 + odd-n
//! middle) when `logicaffeine_kernel::bitvector` proves the reflection
//! invariance for all n. These run the program through the REAL AOT optimizer
//! (`ui_bridge::with_v2_optimized_program` → `optimize::optimize_program`) on
//! the bytecode VM and assert the output equals the UNOPTIMIZED tree-walker
//! oracle — so the halving must be exactly count-preserving. Structural tests
//! prove it FIRED on the symmetric search and stayed FAIL-CLOSED on an
//! asymmetric one.

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_base::Interner;
use logicaffeine_compile::compile::tw_outcome_with_args;
use logicaffeine_compile::ui_bridge::with_v2_optimized_program;
use logicaffeine_compile::vm::NativeTier;
use logicaffeine_jit::ForgeTier;

fn norm(s: &str) -> String {
    s.lines()
        .map(|l| l.trim_end())
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Cap recursion unroll depth so the debug VM (which runs the half-enumeration's
/// sub-calls) finishes quickly. Production AOT (rustc) needs no cap.
fn cap_depth_for_vm() {
    std::env::set_var("LOGOS_RECURSE_DEPTH", "2");
}

fn v2_outcome(src: &str, argv: &[String]) -> (String, Option<String>) {
    let tier = ForgeTier::new();
    with_v2_optimized_program(src, |parsed, interner| match parsed {
        Ok((stmts, types, policies)) => logicaffeine_compile::vm::run_to_outcome_with_args(
            stmts,
            interner,
            Some(types),
            Some(&policies),
            argv,
            Some(&tier as &dyn NativeTier),
        ),
        Err(advice) => (String::new(), Some(advice)),
    })
}

fn assert_sound(src: &str, argv: &[String], expected: &str) {
    let (out, err) = v2_outcome(src, argv);
    let tw = tw_outcome_with_args(src, argv);
    assert_eq!(err, None, "AOT-optimized path errored:\n{src}");
    assert_eq!(
        (norm(&out), &err),
        (norm(&tw.output), &tw.error),
        "symmetry-broken VM diverged from the raw tree-walker on:\n{src}"
    );
    assert_eq!(norm(&out), expected, "wrong answer on:\n{src}");
}

const NQUEENS: &str = "\
## To native args () -> Seq of Text
## To native parseInt (s: Text) -> Int

## To solve (row: Int, cols: Int, diag1: Int, diag2: Int, n: Int) -> Int:
    If row equals n:
        Return 1.
    Let all be (1 shifted left by n) - 1.
    Let mutable available be all & ~(cols | diag1 | diag2).
    Let mutable count be 0.
    While available is not 0:
        Let bit be available & (0 - available).
        Set available to available xor bit.
        Set count to count + solve(row + 1, cols | bit, (diag1 | bit) shifted left by 1, (diag2 | bit) shifted right by 1, n).
    Return count.

## Main
Let arguments be args().
Let n be parseInt(item 2 of arguments).
Show solve(0, 0, 0, 0, n).
";

// Asymmetric variant: BOTH diagonals shift left, so there is no conjugate
// `<<1`/`>>1` pair → the reflection structure does not hold → the pass must NOT
// fire (it would be unsound). (This computes a different, non-nqueens count;
// we only use it to check fail-closed behaviour, not a specific value.)
const ASYMMETRIC: &str = "\
## To native args () -> Seq of Text
## To native parseInt (s: Text) -> Int

## To solve (row: Int, cols: Int, diag1: Int, diag2: Int, n: Int) -> Int:
    If row equals n:
        Return 1.
    Let all be (1 shifted left by n) - 1.
    Let mutable available be all & ~(cols | diag1 | diag2).
    Let mutable count be 0.
    While available is not 0:
        Let bit be available & (0 - available).
        Set available to available xor bit.
        Set count to count + solve(row + 1, cols | bit, (diag1 | bit) shifted left by 1, (diag2 | bit) shifted left by 1, n).
    Return count.

## Main
Let arguments be args().
Let n be parseInt(item 2 of arguments).
Show solve(0, 0, 0, 0, n).
";

fn argv(n: &str) -> Vec<String> {
    vec!["bench".to_string(), n.to_string()]
}

/// Even AND odd n must come out right (odd exercises the middle-column term).
#[test]
fn nqueens_symmetry_matches_oracle() {
    cap_depth_for_vm();
    assert_sound(NQUEENS, &argv("6"), "4"); // even
    assert_sound(NQUEENS, &argv("8"), "92"); // even
    assert_sound(NQUEENS, &argv("9"), "352"); // odd → middle column
    assert_sound(NQUEENS, &argv("10"), "724"); // even
}

// ---------------------------------------------------------------------------
// Structural: the pass FIRED on the symmetric search, and did NOT fire on the
// asymmetric one. The rewrite interns a unique `__sym_mid` symbol for the
// odd-n middle column; interning is permanent and survives later passes
// (which strength-reduce the `×2` into `<<1`), so its presence is a robust
// "the symmetry rewrite ran" witness.
// ---------------------------------------------------------------------------

#[test]
fn symmetric_search_is_rewritten() {
    cap_depth_for_vm();
    with_v2_optimized_program(NQUEENS, |_parsed, it: &Interner| {
        assert!(
            it.lookup("__sym_mid").is_some(),
            "symmetry pass did not fire on the reflection-symmetric n-queens search"
        );
    });
}

#[test]
fn asymmetric_search_is_left_alone() {
    cap_depth_for_vm();
    // Fail-closed: no conjugate diagonal pair → no rewrite → no `__sym_mid`.
    with_v2_optimized_program(ASYMMETRIC, |_parsed, it: &Interner| {
        assert!(
            it.lookup("__sym_mid").is_none(),
            "symmetry pass fired on an asymmetric search — unsound"
        );
    });
    // And it still runs correctly (output matches the oracle for the same program).
    let (out, err) = v2_outcome(ASYMMETRIC, &argv("6"));
    let tw = tw_outcome_with_args(ASYMMETRIC, &argv("6"));
    assert_eq!(err, None);
    assert_eq!(norm(&out), norm(&tw.output), "asymmetric program miscompiled");
}

// ---------------------------------------------------------------------------
// Milestone A: the SAME left-right reflection must be broken on a RANGE-
// enumerated N-Queens — the identical algorithm as the bitmask `NQUEENS` above
// (and the validated benchmarks/programs/nqueens/main.lg), but it walks the
// columns with a `While col is less than n` range loop + an inline availability
// test (`(available & bit) is not 0`) instead of the `avail and (0-avail)`
// bit-trick loop. The state updates fed to the recursion are byte-for-byte the
// reflection-symmetric triple (cols | bit, (diag1|bit)<<1, (diag2|bit)>>1), so
// the geometric LIA certificate is reused unchanged; only the CHOICE-domain
// recognizer must generalize from the bit-iteration form to RangeChoices.
// Columns are 0-based, so the reflection is col ↦ (n-1)-col; the fixed middle
// (n-1)/2 exists iff n is odd. Today's bit-loop-only matcher does not fire here.
// ---------------------------------------------------------------------------

const RANGE_NQUEENS: &str = "\
## To native args () -> Seq of Text
## To native parseInt (s: Text) -> Int

## To solve (row: Int, cols: Int, diag1: Int, diag2: Int, n: Int) -> Int:
    If row equals n:
        Return 1.
    Let all be (1 shifted left by n) - 1.
    Let available be all & ~(cols | diag1 | diag2).
    Let mutable count be 0.
    Let mutable col be 0.
    While col is less than n:
        Let bit be 1 shifted left by col.
        If (available & bit) is not 0:
            Set count to count + solve(row + 1, cols | bit, (diag1 | bit) shifted left by 1, (diag2 | bit) shifted right by 1, n).
        Set col to col + 1.
    Return count.

## Main
Let arguments be args().
Let n be parseInt(item 2 of arguments).
Show solve(0, 0, 0, 0, n).
";

/// Soundness oracle: the range/Seq form must stay count-exact across even and
/// odd n (odd n exercises the fixed middle column). Passes whether or not the
/// pass fires; it is the regression guard that the Mode-A rewrite is exact.
#[test]
fn range_nqueens_symmetry_matches_oracle() {
    cap_depth_for_vm();
    assert_sound(RANGE_NQUEENS, &argv("4"), "2"); // even
    assert_sound(RANGE_NQUEENS, &argv("5"), "10"); // odd → middle column
    assert_sound(RANGE_NQUEENS, &argv("6"), "4"); // even
}

/// Firing spec (RED until the recognizer goes domain-agnostic): the pass must
/// break the reflection symmetry of the range/Seq search, witnessed by the
/// interned `__sym_mid` middle-column symbol.
#[test]
fn range_nqueens_search_is_rewritten() {
    cap_depth_for_vm();
    with_v2_optimized_program(RANGE_NQUEENS, |_parsed, it: &Interner| {
        assert!(
            it.lookup("__sym_mid").is_some(),
            "symmetry pass did not fire on the range/Seq reflection-symmetric n-queens search"
        );
    });
}
