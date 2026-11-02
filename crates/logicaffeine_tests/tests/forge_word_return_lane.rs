//! ★ LOCK — a function that returns a machine word runs as native code. ★
//!
//! `Word32` and `Word64` ride a native slot unboxed and re-box at the VM boundary, so a function can return one the way
//! it returns an `Int`. The SHA-256 helpers are exactly that shape (`sigma0`, `ch`, `maj`, … each `-> Word32`), and the
//! kind dataflow refused the `Return` of a Word, so every one of them, and the benchmark's whole compression path,
//! stayed on the VM. Each row runs hot on the native tier, prints what the tree-walker prints, and (x86_64) compiles
//! the function.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test forge_word_return_lane`

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::compile::{tw_outcome_with_args, vm_outcome_with_args};
use logicaffeine_compile::vm::NativeTier;
use logicaffeine_jit::ForgeTier;

/// `src` prints what the tree-walker prints with the native tier installed, and at least `floor` functions compiled.
fn native_matches(src: &str, args: &[String], floor: u32) {
    let tw = tw_outcome_with_args(src, args);
    assert_eq!(tw.error, None, "the tree-walker refused:\n{src}");
    let tier = ForgeTier::new();
    let run = vm_outcome_with_args(src, args, Some(&tier as &dyn NativeTier));
    assert_eq!(run.error, None, "the native tier refused:\n{src}");
    // The two helpers differ only in whether the last line keeps its newline.
    assert_eq!(
        run.output.trim_end_matches('\n'),
        tw.output.trim_end_matches('\n'),
        "the native tier diverged from the tree-walker:\n{src}"
    );
    #[cfg(target_arch = "x86_64")]
    {
        let (_, compiled) = tier.function_counts();
        assert!(compiled >= floor, "{floor} function(s) returning a Word must compile to native code; {compiled} did\n{src}");
    }
}

#[test]
fn a_sigma_function_returning_a_word32_compiles() {
    native_matches(
        "## To bigSigma0 (x: Word32) -> Word32:
    Return rotr(x, 2) xor rotr(x, 13) xor rotr(x, 22).

## Main
Let mutable acc be 0.
Let mutable k be 1.
While k is at most 400:
    Set acc to acc + intOfWord32(bigSigma0(word32(k * 2654435761))).
    Set k to k + 1.
Show acc.
",
        &[],
        1,
    );
}

#[test]
fn a_choice_function_over_three_words_compiles() {
    native_matches(
        "## To ch (e: Word32, f: Word32, g: Word32) -> Word32:
    Return (e & f) xor (word_not(e) & g).

## Main
Let mutable acc be 0.
Let mutable k be 1.
While k is at most 400:
    Set acc to acc + intOfWord32(ch(word32(k), word32(k * 7), word32(k * 13))).
    Set k to k + 1.
Show acc.
",
        &[],
        1,
    );
}

#[test]
fn a_word64_mixing_function_compiles() {
    native_matches(
        "## To mix (a: Word64, b: Word64) -> Word64:
    Return (a * word64(6364136223846793005)) + b.

## Main
Let mutable acc be 0.
Let mutable k be 1.
While k is at most 400:
    Set acc to (acc + intOfWord64(mix(word64(k), word64(k * 3))) % 1000003) % 1000003.
    Set k to k + 1.
Show acc.
",
        &[],
        1,
    );
}

#[test]
fn a_self_recursive_word_fold_compiles() {
    native_matches(
        "## To fold (x: Word32, n: Int) -> Word32:
    If n equals 0:
        Return x.
    Return fold(rotl(x, 5) + word32(n), n - 1).

## Main
Let mutable acc be 0.
Let mutable k be 1.
While k is at most 400:
    Set acc to acc + intOfWord32(fold(word32(k), 20)).
    Set k to k + 1.
Show acc.
",
        &[],
        1,
    );
}

/// The benchmark itself: its helpers return Words, and they tier.
#[test]
fn the_sha256_benchmark_tiers_its_word_helpers() {
    let src = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../benchmarks/programs/sha256/main.lg"
    ))
    .expect("sha256/main.lg");
    native_matches(&src, &["sha256".to_string(), "300".to_string()], 4);
}
