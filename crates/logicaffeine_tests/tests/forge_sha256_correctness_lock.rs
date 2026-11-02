//! FORGE ≡ TREE-WALKER on REAL `sha256` — a correctness lock proving the Word scalar/state/arithmetic
//! lanes (scalar `Word32` state `a..hh`, modular `Word32 +`, `rotl`/`word_and`/`word_not`/`xor`) are
//! byte-exact when the forge runs the actual stdlib `sha256`, not just isolated micro-cases. The eprintln
//! also reports tiering counts for diagnostics: today only Main's byte-build loop tiers (the compression
//! loop is never attempted — per-invocation loop counting across the many short `sha256Compress` calls
//! never gets hot enough, a separate profiling matter from op coverage). Correctness is the invariant here.

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::compile::{tw_outcome_with_args, vm_outcome_with_args};
use logicaffeine_compile::vm::NativeTier;
use logicaffeine_jit::ForgeTier;

#[test]
fn forge_sha256_matches_tree_walker() {
    // A multi-block message so the 64-round compression loop crosses its back-edge thousands of times.
    let src = "## Main\n\
        Let mutable bytes be a new Seq of Int.\n\
        Let mutable i be 0.\n\
        While i is less than 20000:\n\
        \x20   Push (i mod 256) to bytes.\n\
        \x20   Set i to i + 1.\n\
        Let d be sha256(bytes).\n\
        Show item 1 of d.\n";
    let tw = tw_outcome_with_args(src, &[]);
    let tier = ForgeTier::new();
    let forge = vm_outcome_with_args(src, &[], Some(&tier as &dyn NativeTier));
    let (fn_all, fn_ok) = tier.function_counts();
    let (rg_all, rg_ok) = tier.region_counts();
    eprintln!(
        "SHA256 PROBE: tw={:?} forge={:?} tw_err={:?} forge_err={:?} | fn {fn_ok}/{fn_all} region {rg_ok}/{rg_all}",
        tw.output.trim(),
        forge.output.trim(),
        tw.error.is_some(),
        forge.error.is_some(),
    );
    // Correctness must hold regardless of tiering (this is the real invariant).
    assert_eq!(
        forge.output.trim(),
        tw.output.trim(),
        "sha256 forge diverged from tree-walker"
    );
    assert_eq!(
        forge.error.is_some(),
        tw.error.is_some(),
        "sha256 error mismatch"
    );
}
