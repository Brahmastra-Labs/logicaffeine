//! Milestone B — value symmetry (Sₖ).
//!
//! When a search assigns INTERCHANGEABLE values (its only constraint on the
//! assigned value is an equality/inequality — e.g. a graph coloring where the
//! sole rule is "adjacent nodes get different colors"), the k colors form a
//! symmetric group Sₖ: any color bijection maps a valid assignment to a valid
//! assignment. The FIRST free choice can therefore be pinned to the smallest
//! value and the entry result multiplied by k — the exact analogue of
//! Milestone A's first-row C₂ break, now over Sₖ on the first choice.
//!
//! Soundness is checked the same way as Milestone A: the symmetry-broken
//! AOT/VM output must equal BOTH the raw tree-walker AND the closed-form oracle
//! (`assert_sound`). The rewrite is proven count-preserving: for a path Pₙ with
//! k colors the total is k·(k-1)^(n-1), and pinning the first color to 1 gives
//! (k-1)^(n-1), so ×k reconstructs the total exactly.

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_base::Interner;
use logicaffeine_compile::compile::tw_outcome_with_args;
use logicaffeine_compile::ui_bridge::with_v2_optimized_program;
use logicaffeine_compile::vm::NativeTier;
use logicaffeine_jit::ForgeTier;

fn norm(s: &str) -> String {
    s.lines().map(|l| l.trim_end()).filter(|l| !l.is_empty()).collect::<Vec<_>>().join("\n")
}

/// Cap recursion unroll depth so the debug VM (which runs the sub-calls
/// interpretively past the cap) finishes quickly — the result is unaffected,
/// only the JIT unrolling is bounded. Production AOT (rustc) needs no cap.
fn cap_depth_for_vm() {
    std::env::set_var("LOGOS_RECURSE_DEPTH", "2");
}

fn v2_outcome(src: &str, argv: &[String]) -> (String, Option<String>) {
    let tier = ForgeTier::new();
    with_v2_optimized_program(src, |parsed, interner| match parsed {
        Ok((stmts, types, policies)) => logicaffeine_compile::vm::run_to_outcome_with_args(
            stmts, interner, Some(types), Some(&policies), argv, Some(&tier as &dyn NativeTier),
        ),
        Err(advice) => (String::new(), Some(advice)),
    })
}

/// The symmetry-broken output must match the raw tree-walker AND the oracle.
fn assert_sound(src: &str, argv: &[String], expected: &str) {
    let (out, err) = v2_outcome(src, argv);
    let tw = tw_outcome_with_args(src, argv);
    assert_eq!(err, None, "value-symmetry AOT path errored:\n{src}");
    assert_eq!(
        (norm(&out), &err),
        (norm(&tw.output), &tw.error),
        "value-symmetry-broken VM diverged from the raw tree-walker on:\n{src}"
    );
    assert_eq!(norm(&out), expected, "wrong count on:\n{src}");
}

fn argv2(n: &str, k: &str) -> Vec<String> {
    vec!["bench".to_string(), n.to_string(), k.to_string()]
}

// A path-graph proper-coloring COUNTER. `color(node, prev, n, k)` colors nodes
// node..n; the only rule is `c ≠ prev` (equality-only → Sₖ-symmetric in the
// colors). `countAll` enumerates node 1's color over {1..k} and sums — so the
// first choice ranges freely over all k interchangeable colors. Total = k·(k-1)^(n-1).
const PATH_COLORING: &str = "\
## To native args () -> Seq of Text
## To native parseInt (s: Text) -> Int

## To color (node: Int, prev: Int, n: Int, k: Int) -> Int:
    If node is greater than n:
        Return 1.
    Let mutable count be 0.
    Let mutable c be 1.
    While c is at most k:
        If c is not prev:
            Set count to count + color(node + 1, c, n, k).
        Set c to c + 1.
    Return count.

## To countAll (n: Int, k: Int) -> Int:
    Let mutable total be 0.
    Let mutable c be 1.
    While c is at most k:
        Set total to total + color(2, c, n, k).
        Set c to c + 1.
    Return total.

## Main
Let arguments be args().
Let n be parseInt(item 2 of arguments).
Let k be parseInt(item 3 of arguments).
Show countAll(n, k).
";

// Fail-closed control: the rule is `c > prev` (an ORDERING on the values), which
// is NOT invariant under a color permutation → NOT value-symmetric → the pass
// must NOT fire (pinning the first choice would give the wrong count).
const ORDERED: &str = "\
## To native args () -> Seq of Text
## To native parseInt (s: Text) -> Int

## To chain (node: Int, prev: Int, n: Int, k: Int) -> Int:
    If node is greater than n:
        Return 1.
    Let mutable count be 0.
    Let mutable c be 1.
    While c is at most k:
        If c is greater than prev:
            Set count to count + chain(node + 1, c, n, k).
        Set c to c + 1.
    Return count.

## To countAll (n: Int, k: Int) -> Int:
    Let mutable total be 0.
    Let mutable c be 1.
    While c is at most k:
        Set total to total + chain(2, c, n, k).
        Set c to c + 1.
    Return total.

## Main
Let arguments be args().
Let n be parseInt(item 2 of arguments).
Let k be parseInt(item 3 of arguments).
Show countAll(n, k).
";

#[test]
fn value_symmetric_coloring_matches_oracle() {
    cap_depth_for_vm();
    // path Pₙ, k colors: proper colorings = k·(k-1)^(n-1).
    assert_sound(PATH_COLORING, &argv2("4", "3"), "24"); // 3·2^3
    assert_sound(PATH_COLORING, &argv2("5", "3"), "48"); // 3·2^4
    assert_sound(PATH_COLORING, &argv2("4", "4"), "108"); // 4·3^3
    assert_sound(PATH_COLORING, &argv2("6", "3"), "96"); // 3·2^5
}

#[test]
fn value_symmetric_search_is_rewritten() {
    // The pass must discover the Sₖ value symmetry and break the first choice.
    with_v2_optimized_program(PATH_COLORING, |_parsed, it: &Interner| {
        assert!(
            it.lookup("__valuesym").is_some(),
            "value-symmetry pass did not fire on the equality-only coloring search"
        );
    });
}

#[test]
fn ordered_search_is_left_alone() {
    cap_depth_for_vm();
    // Fail-closed: `c > prev` is an ordering constraint, not value-symmetric.
    with_v2_optimized_program(ORDERED, |_parsed, it: &Interner| {
        assert!(
            it.lookup("__valuesym").is_none(),
            "value-symmetry pass fired on an ordering constraint — unsound"
        );
    });
    // And it still computes correctly (optimized == raw tree-walker).
    let (out, err) = v2_outcome(ORDERED, &argv2("4", "3"));
    let tw = tw_outcome_with_args(ORDERED, &argv2("4", "3"));
    assert_eq!(err, None);
    assert_eq!(norm(&out), norm(&tw.output), "ordered program miscompiled");
}
