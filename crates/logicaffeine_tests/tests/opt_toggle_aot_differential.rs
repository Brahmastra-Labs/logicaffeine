//! PHASE D — heavy AOT toggle differential (#[ignore]d; the codegen-only tier).
//!
//! The fast tier (`vm_opt_differential::clean_disable_paths_preserve_output`)
//! fuzzes the RUN path (VM+JIT); it cannot see the codegen-only optimizations
//! (de-Rc/`Unbox`, `Interleave`/AoS, `Narrow`, oracle-`Unchecked`, `DenseMap`,
//! `NarrowMap`, …) which only materialize in emitted Rust. This tier covers them:
//! it compiles each program to Rust → rustc → runs it, under a spread of
//! codegen-opt-disabled configs, and asserts the COMPILED program's output is
//! IDENTICAL to the all-on build — disabling an optimization changes the Rust's
//! shape, never its result. It also asserts every config still compiles+runs.
//!
//! `#[ignore]`d because it invokes rustc per config (minutes); the default fast
//! script runs it via `--run-ignored all`. Programs are self-contained (no argv)
//! and SHAPED to trigger the codegen optimizations, so leave-one-out actually
//! exercises each one (this doubles as the per-optimization trigger corpus).

#![cfg(not(target_arch = "wasm32"))]

mod common;
use common::run_logos;

/// Build-then-scan an int array: triggers de-Rc (`Unbox`), i32 `Narrow`, and
/// oracle-`Unchecked` indexing.
const ARRAY_SUM: &str = "## Main
Let mutable a be a new Seq of Int.
Let mutable i be 1.
While i is at most 100:
    Push (i % 7) to a.
    Set i to i + 1.
Let mutable s be 0.
Set i to 1.
While i is at most 100:
    Set s to s + item i of a.
    Set i to i + 1.
Show s.
";

/// A dense-int-keyed map: triggers `DenseMap` and `NarrowMap`.
const INT_MAP: &str = "## Main
Let mutable m be a new Map of Int to Int.
Let mutable i be 1.
While i is at most 50:
    Set item i of m to i * 3.
    Set i to i + 1.
Let mutable s be 0.
Set i to 1.
While i is at most 50:
    Set s to s + (item i of m).
    Set i to i + 1.
Show s.
";

/// Two co-indexed fixed `Seq of Float`: triggers `Interleave` (AoS) / `Scalarize`.
const COINDEXED_FLOAT: &str = "## Main
Let mutable x be a new Seq of Float.
Let mutable y be a new Seq of Float.
Let mutable i be 1.
While i is at most 20:
    Push 1.5 to x.
    Push 2.5 to y.
    Set i to i + 1.
Let mutable t be 0.0.
Set i to 1.
While i is at most 20:
    Set t to t + item i of x + item i of y.
    Set i to i + 1.
Show \"{t:.1}\".
";

const TRIGGERS: &[(&str, &str)] = &[
    ("array_sum", ARRAY_SUM),
    ("int_map", INT_MAP),
    ("coindexed_float", COINDEXED_FLOAT),
];

#[ignore]
#[test]
fn aot_codegen_clean_disable_preserves_output() {
    // The codegen-relevant optimizations (plus a couple of shared ones). Disabling
    // any one — or all of them — must keep the compiled output identical and valid.
    let opts = [
        "unbox", "interleave", "narrow", "unchecked", "scalarize", "densemap",
        "narrowmap", "oracle", "oraclehints", "cse", "unroll", "fastdiv", "deadcode",
    ];

    for &(name, src) in TRIGGERS {
        std::env::remove_var("LOGOS_OPT_OFF");
        std::env::remove_var("LOGOS_OPT");
        let base = run_logos(src);
        assert!(base.success, "{name}: must compile+run at all-on:\n{}", base.stderr);

        // Leave-one-out: disable exactly one codegen optimization, the rest on.
        for kw in opts {
            std::env::set_var("LOGOS_OPT_OFF", kw);
            let got = run_logos(src);
            std::env::remove_var("LOGOS_OPT_OFF");
            assert!(
                got.success,
                "{name}: `## No {kw}` broke codegen (compile/run failed):\n{}",
                got.stderr
            );
            assert_eq!(
                got.stdout, base.stdout,
                "{name}: disabling `{kw}` changed the COMPILED output — an optimization \
                 is not semantics-preserving"
            );
        }

        // The boring all-off build must produce the same result, too.
        std::env::set_var("LOGOS_OPT", "off");
        let off = run_logos(src);
        std::env::remove_var("LOGOS_OPT");
        assert!(off.success, "{name}: all-off (boring) build failed:\n{}", off.stderr);
        assert_eq!(off.stdout, base.stdout, "{name}: all-off changed the compiled output");
    }
}
