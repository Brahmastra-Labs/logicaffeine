//! FORGE WORD (crypto) LANE — the `word32`/`word64`/`word16` ring builtins tier the forge with a Word
//! carried UNBOXED as a masked i64 (`Word32`/`16` zero-extended, `Word64` = full bits), re-boxing to
//! `RuntimeValue::Word` at a deopt/exit via `RegBox::Word{32,64}`. Each case produces a Word, computes with
//! it, and folds back to an Int (via `intOfWordN`) so the region output is a scalar — asserted equal to the
//! tree-walker AND (on x86_64) that the hot region actually tiered to native.

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::compile::{tw_outcome_with_args, vm_outcome_with_args};
use logicaffeine_compile::vm::NativeTier;
use logicaffeine_jit::ForgeTier;

/// Run `acc += (expr)` over a hot loop (`k = 1..400`) on the forge and the tree-walker; assert byte-equal,
/// and — on x86_64 — that the region tiered to native (else the Word lowering silently declined to VM).
fn word_eq_tw(label: &str, expr: &str) {
    let src = format!(
        "## Main\nLet mutable acc be 0.\nLet mutable k be 1.\nWhile k is at most 400:\n\
         \x20   Set acc to acc + ({expr}).\n\x20   Set k to k + 1.\nShow acc.\n"
    );
    let tw = tw_outcome_with_args(&src, &[]);
    let tier = ForgeTier::new();
    let forge = vm_outcome_with_args(&src, &[], Some(&tier as &dyn NativeTier));
    assert!(
        tw.error.is_none(),
        "[{label}] tree-walker errored: {:?}\n{src}",
        tw.error
    );
    assert!(
        forge.error.is_none(),
        "[{label}] forge errored: {:?}\n{src}",
        forge.error
    );
    assert_eq!(
        forge.output.trim(),
        tw.output.trim(),
        "[{label}] forge diverged from the tree-walker\n{src}"
    );
    #[cfg(target_arch = "x86_64")]
    {
        let (_, region_ok) = tier.region_counts();
        assert!(region_ok >= 1, "[{label}] did NOT tier its hot region (region={region_ok}) — the Word lowering declined");
    }
}

#[test]
fn forge_word_lane_easy_ops() {
    // Producers + converters round-trip: mask an Int to the width, then read it back.
    word_eq_tw("word32 → intOfWord32", "intOfWord32(word32(k))");
    word_eq_tw("word64 → intOfWord64", "intOfWord64(word64(k))");
    word_eq_tw("word16 → intOfWord16", "intOfWord16(word16(k))");
    // A wide Int fed to word16 keeps only its LOW 16 bits (k · 65537 sets bit 16 and above).
    word_eq_tw(
        "word16 of a wide Int → intOfWord16 (low-16 mask)",
        "intOfWord16(word16(k * 65537))",
    );
    // Bitwise AND/OR over same-width Words (mask preserved).
    word_eq_tw(
        "word_and (Word32)",
        "intOfWord32(word_and(word32(k), word32(600 - k)))",
    );
    word_eq_tw(
        "word_or (Word32)",
        "intOfWord32(word_or(word32(k), word32(600 - k)))",
    );
    word_eq_tw(
        "word64And",
        "intOfWord64(word64And(word64(k), word64(600 - k)))",
    );
    // A Word64 with the high bit set (k · 2⁵⁰ exceeds i63) then read back — exercises the full 64-bit carrier.
    word_eq_tw("word64 high-bit round-trip", "intOfWord64(word64(0 - k))");
    // Phase 2a: shifts + complement.
    word_eq_tw("word64Shl", "intOfWord64(word64Shl(word64(k), 5))");
    word_eq_tw("word32Shr", "intOfWord32(word32Shr(word32(k), 3))");
    // word32Shr count ≥ 32 must mask to `& 31` (n=33 ⇒ shift by 1, not 33).
    word_eq_tw(
        "word32Shr count masks to 31",
        "intOfWord32(word32Shr(word32(k), 33))",
    );
    word_eq_tw("word_not (Word32)", "intOfWord32(word_not(word32(k)))");
    // word_not on a Word64 (full 64-bit complement), incl. the high bit via a negative-seeded Word64.
    word_eq_tw("word_not (Word64)", "intOfWord64(word_not(word64(k)))");
    word_eq_tw(
        "word_not (Word64 high-bit)",
        "intOfWord64(word_not(word64(0 - k)))",
    );
    // Word32 rotates — the wraparound is the point, so include a value with high bits set (word32 of a big
    // Int) and the identity/full-turn edges (rotate by 0 and by 32).
    word_eq_tw("rotl (Word32)", "intOfWord32(rotl(word32(k), 7))");
    word_eq_tw("rotr (Word32)", "intOfWord32(rotr(word32(k), 7))");
    word_eq_tw("rotl high bits", "intOfWord32(rotl(word32(k * 65537), 13))");
    word_eq_tw("rotl by 0 (identity)", "intOfWord32(rotl(word32(k), 0))");
    word_eq_tw("rotl by 32 (full turn)", "intOfWord32(rotl(word32(k), 32))");
    word_eq_tw("rotr by 0 (identity)", "intOfWord32(rotr(word32(k), 0))");
    // word64Shr LOGICAL right shift by a constant — the high-bit-set carrier must NOT sign-extend.
    word_eq_tw(
        "word64Shr high-bit by 5",
        "intOfWord64(word64Shr(word64(0 - k), 5))",
    );
    word_eq_tw(
        "word64Shr high-bit by 1",
        "intOfWord64(word64Shr(word64(0 - k), 1))",
    );
    word_eq_tw(
        "word64Shr high-bit by 63",
        "intOfWord64(word64Shr(word64(0 - k), 63))",
    );
    word_eq_tw(
        "word64Shr by 0 (identity)",
        "intOfWord64(word64Shr(word64(0 - k), 0))",
    );
    word_eq_tw(
        "word64Shr positive by 8",
        "intOfWord64(word64Shr(word64(k), 8))",
    );
}

/// `intOfWord16` of a `Word32` is a width error on every tier (the compiled tier's `int_of_word16` takes a
/// `Word16`); the forge must decline it so the VM reports the same error.
#[test]
fn forge_int_of_word16_refuses_a_word32_on_both_tiers() {
    let src = "## Main\nLet mutable acc be 0.\nLet mutable k be 1.\nWhile k is at most 400:\n\
               \x20   Set acc to acc + (intOfWord16(word32(k))).\n\x20   Set k to k + 1.\nShow acc.\n";
    let tw = tw_outcome_with_args(src, &[]);
    let tier = ForgeTier::new();
    let forge = vm_outcome_with_args(src, &[], Some(&tier as &dyn NativeTier));
    assert!(
        tw.error.is_some(),
        "the tree-walker must refuse `intOfWord16` of a Word32, got output {:?}",
        tw.output
    );
    assert_eq!(
        forge.error, tw.error,
        "the forge must report the tree-walker's width error\n{src}"
    );
}
