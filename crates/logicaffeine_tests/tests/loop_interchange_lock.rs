//! Loop interchange lock.
//!
//! Interchange reorders two adjacent nested counted loops to put the unit-stride
//! access innermost — the `ijk`→`ikj` matmul recipe. It fires ONLY on integer
//! commutative-reduction nests (any permutation bit-identical) and ONLY toward
//! better locality. These locks pin:
//!
//!   1. FIRING — it fires on a suboptimal `ijk` integer matmul and gates off
//!      under `## No interchange`.
//!   2. CORRECTNESS — the reordered (optimized) run's modular checksum equals
//!      the raw (`ijk`) run's, over a spread of matrix sizes. A wrong reordering
//!      changes the checksum and is caught.
//!   3. NO PESSIMIZATION — an already-optimal `ikj` nest is LEFT ALONE
//!      (interchange does not fire), so it never reorders a good loop to a worse
//!      one.

use logicaffeine_compile::compile::{
    compile_to_rust, interpret_program, optimizations_fired_run, vm_run_source,
};
use logicaffeine_compile::optimization::decorate_source;

/// Matmul with the compute loops in `<i, j, k>` order (k innermost) — SUBOPTIMAL,
/// because the innermost `k` gives stride-1 access only to `a`, while `c` and `b`
/// are stride-1 in `j`. Interchange should swap `j`/`k` to make it `<i, k, j>`.
fn ijk_matmul(n: i64) -> String {
    format!(
        "## Main\nLet n be {n}.\n\
         Let a be a new Seq of Int.\nLet b be a new Seq of Int.\nLet c be a new Seq of Int.\n\
         Let i be 0.\nWhile i is less than n:\n    Let j be 0.\n    While j is less than n:\n        Push (i * n + j) % 100 to a.\n        Push (j * n + i) % 100 to b.\n        Push 0 to c.\n        Set j to j + 1.\n    Set i to i + 1.\n\
         Set i to 0.\nWhile i is less than n:\n    Let j be 0.\n    While j is less than n:\n        Let k be 0.\n        While k is less than n:\n            Set item (i * n + j + 1) of c to (item (i * n + j + 1) of c + item (i * n + k + 1) of a * item (k * n + j + 1) of b) % 1000000007.\n            Set k to k + 1.\n        Set j to j + 1.\n    Set i to i + 1.\n\
         Let checksum be 0.\nSet i to 1.\nWhile i is at most n * n:\n    Set checksum to (checksum + item i of c) % 1000000007.\n    Set i to i + 1.\nShow checksum.\n"
    )
}

/// The same matmul already in `<i, k, j>` order (j innermost) — OPTIMAL.
fn ikj_matmul(n: i64) -> String {
    format!(
        "## Main\nLet n be {n}.\n\
         Let a be a new Seq of Int.\nLet b be a new Seq of Int.\nLet c be a new Seq of Int.\n\
         Let i be 0.\nWhile i is less than n:\n    Let j be 0.\n    While j is less than n:\n        Push (i * n + j) % 100 to a.\n        Push (j * n + i) % 100 to b.\n        Push 0 to c.\n        Set j to j + 1.\n    Set i to i + 1.\n\
         Set i to 0.\nWhile i is less than n:\n    Let k be 0.\n    While k is less than n:\n        Let j be 0.\n        While j is less than n:\n            Set item (i * n + j + 1) of c to (item (i * n + j + 1) of c + item (i * n + k + 1) of a * item (k * n + j + 1) of b) % 1000000007.\n            Set j to j + 1.\n        Set k to k + 1.\n    Set i to i + 1.\n\
         Let checksum be 0.\nSet i to 1.\nWhile i is at most n * n:\n    Set checksum to (checksum + item i of c) % 1000000007.\n    Set i to i + 1.\nShow checksum.\n"
    )
}

fn opt(src: &str) -> String {
    interpret_program(src)
        .unwrap_or_else(|e| panic!("optimized run failed: {e:?}"))
        .trim_end()
        .to_string()
}
fn raw(src: &str) -> String {
    vm_run_source(src)
        .unwrap_or_else(|e| panic!("raw run failed: {e}"))
        .trim_end()
        .to_string()
}

#[test]
fn interchange_fires_and_gates() {
    let src = ijk_matmul(12);
    let run = optimizations_fired_run(&src);
    assert!(
        run.contains(&"interchange"),
        "must interchange the suboptimal ijk nest; got {run:?}"
    );
    let off = optimizations_fired_run(&decorate_source(&src, &["interchange"]));
    assert!(
        !off.contains(&"interchange"),
        "`## No interchange` must remove it; got {off:?}"
    );
}

/// Correctness: the reordered nest computes the identical modular checksum
/// (integer reduction is order-invariant), across tiers and on vs off.
#[test]
fn interchange_preserves_matmul_checksum() {
    for n in [2, 3, 5, 8, 12] {
        let src = ijk_matmul(n);
        let off = decorate_source(&src, &["interchange"]);
        assert_eq!(
            opt(&src),
            raw(&src),
            "n={n}: interchange changed the checksum (miscompile!)"
        );
        assert_eq!(
            opt(&src),
            opt(&off),
            "n={n}: interchange diverged from interchange-off"
        );
    }
}

/// No pessimization: an already-optimal `ikj` nest must NOT be interchanged —
/// its inner `j` is already the unit-stride loop.
#[test]
fn interchange_leaves_optimal_ikj_alone() {
    let src = ikj_matmul(12);
    let run = optimizations_fired_run(&src);
    assert!(
        !run.contains(&"interchange"),
        "must not reorder an already-optimal ikj nest; got {run:?}"
    );
    // ...and both orders compute the same value.
    assert_eq!(opt(&src), opt(&ijk_matmul(12)));
}

/// `## Require interchange` compiles on the ijk nest and errors with no site.
#[test]
fn interchange_require_gate() {
    let required = ijk_matmul(8).replace("## Main", "## Require interchange\n## Main");
    assert!(
        compile_to_rust(&required).is_ok(),
        "`## Require interchange` must compile on ijk"
    );
    let no_site = "## Require interchange\n## Main\nShow 42.\n";
    assert!(
        compile_to_rust(no_site).is_err(),
        "`## Require interchange` must error with no site"
    );
}
