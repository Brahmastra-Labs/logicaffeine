//! ★ LOCK — a function that builds and returns a `Seq of Word32` runs as native code. ★
//!
//! SHA-256's compression function takes three `Seq of Word32` lists, builds a fresh one (the message schedule) and
//! returns another (the next state). The JIT typed every fresh list as a list of `Int`s and returned only those, so the
//! function stayed on the VM and the benchmark ran its whole hot path in bytecode (6.7 s at n = 100000 against
//! 0.16 s through Rust). A fresh list now takes the kind of the words pushed into it, and a list the native code hands
//! back to the VM, returned, written back or held at a side exit, keeps the VM's representation of its elements.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test forge_word_list_lane`

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::compile::{tw_outcome_with_args, vm_outcome_tiered};
use logicaffeine_compile::optimization::Tier;
use logicaffeine_compile::vm::{NativeTier, NATIVE_TIER_THRESHOLD};
use logicaffeine_jit::ForgeTier;

/// `src` prints on the run path `largo run --interpret` takes (the optimizer, then the VM with the native tier) what
/// the tree-walker prints; answer the functions it compiled.
fn native_matches(src: &str, args: &[String]) -> u32 {
    native_run(src, args, &ForgeTier::new())
}

fn native_run(src: &str, args: &[String], tier: &ForgeTier) -> u32 {
    let tw = tw_outcome_with_args(src, args);
    assert_eq!(tw.error, None, "the tree-walker refused:\n{src}");
    let run = vm_outcome_tiered(src, args, Tier::T3, Some(tier as &dyn NativeTier));
    assert_eq!(run.error, None, "the native tier refused:\n{src}");
    assert_eq!(
        run.output.trim_end_matches('\n'),
        tw.output.trim_end_matches('\n'),
        "the native tier diverged from the tree-walker:\n{src}"
    );
    tier.function_counts().1
}

#[test]
fn a_function_returning_a_word_list_it_built_compiles() {
    let src = format!(
        "## To mix (seeds: Seq of Word32, k: Int) -> Seq of Word32:
    Let mutable out be a new Seq of Word32.
    Repeat for s in seeds:
        Push rotr(s, 7) xor word32(k) to out.
    Push word32(k * 2654435761) to out.
    Return out.

## Main
Let mutable seeds be a new Seq of Word32.
Push word32(1779033703) to seeds.
Push word32(3144134277) to seeds.
Push word32(1013904242) to seeds.
Let mutable total be 0.
Let mutable last be a new Seq of Word32.
Repeat for k from 1 to {calls}:
    Set last to mix(seeds, k).
    Set total to (total + intOfWord32(item 2 of last)) % 1000000007.
Show total.
Repeat for w in last:
    Show intOfWord32(w).
",
        calls = 3 * NATIVE_TIER_THRESHOLD
    );
    let compiled = native_matches(&src, &[]);
    #[cfg(target_arch = "x86_64")]
    assert!(
        compiled >= 1,
        "mix, which builds and returns a Seq of Word32, must compile; {compiled} functions did"
    );
    let _ = compiled;
}

#[test]
fn the_sha256_compression_function_compiles() {
    let src = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../benchmarks/programs/sha256/main.lg"
    ))
    .expect("sha256/main.lg");
    let compiled = native_matches(&src, &["sha256".to_string(), "300".to_string()]);
    // The run path inlines the round helpers (ssig0 … maj) into sha256Compress, and sha256K runs once: the one
    // function hot enough to compile is the compression function itself.
    #[cfg(target_arch = "x86_64")]
    assert!(
        compiled >= 1,
        "sha256Compress must compile to native code; {compiled} functions did"
    );
    let _ = compiled;
}

#[test]
fn a_side_exit_holding_a_fresh_word_list_hands_the_vm_its_words() {
    let src = format!(
        "## To pack (seed: Word32, n: Int) -> Seq of Word32:
    Let mutable out be a new Seq of Word32.
    Let mutable big be 9223372036854775000.
    Let mutable i be 0.
    While i is less than n:
        Push seed xor word32(i * 40503) to out.
        Set big to big + 100.
        If big is greater than 9223372036854775807:
            Push seed to out.
        Set i to i + 1.
    Return out.

## Main
Let mutable total be 0.
Repeat for k from 1 to {calls}:
    Let words be pack(word32(k), 4).
    Set total to (total + intOfWord32(item 4 of words)) % 1000000007.
Show total.
Let late be pack(word32(7), 20).
Repeat for w in late:
    Show intOfWord32(w).
",
        calls = 3 * NATIVE_TIER_THRESHOLD
    );
    let tier = ForgeTier::new();
    let compiled = native_run(&src, &[], &tier);
    #[cfg(target_arch = "x86_64")]
    {
        assert!(
            compiled >= 1,
            "pack must compile, or this row proves nothing; {compiled} functions did"
        );
        let (_, completed, _, precise) = tier.runtime_stats();
        assert!(completed >= 1, "the warm calls must finish in native code");
        assert!(
            precise >= 1,
            "the late call's Int overflow must leave native code precisely, holding `out`"
        );
    }
    let _ = compiled;
}
