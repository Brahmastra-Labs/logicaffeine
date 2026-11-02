//! FORGE WORD-STATE LANE — a Word carried as LOOP-CARRIED REGION STATE (the shape real crypto has:
//! `sha256`/MD5 keep their hash words `h0..h7` live across the compression loop). Unlike `forge_word_lane`
//! — where every Word is produced AND consumed inside one iteration and only an `Int` accumulator is
//! carried — here a `Word32`/`Word64` variable is declared BEFORE the loop and mutated each iteration, so
//! it is a region LIVE-IN. That requires the forge to UNBOX the boxed `RuntimeValue::Word` at region entry
//! (a `SlotKind::Word{32,64}` live-in guard) and RE-BOX it at the loop back-edge / exit — the mirror of the
//! `RegBox::Word` output path already built. Correctness = the forge run == the tree-walker; tiering
//! (x86_64) = the hot region actually went native (else the Word live-in silently declined to the VM).

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::compile::{tw_outcome_with_args, vm_outcome_with_args};
use logicaffeine_compile::vm::NativeTier;
use logicaffeine_jit::ForgeTier;

/// Carry a Word accumulator `h` (seeded by `init`) across a hot loop (`k = 1..400`), updating it by
/// `update` each iteration, then fold to an Int for the `Show`. Assert forge == tree-walker AND (x86_64)
/// that the region tiered — a Word LIVE-IN must marshal, not decline.
fn word_state_eq_tw(label: &str, init: &str, update: &str, fold: &str) {
    let src = format!(
        "## Main\nLet mutable h be {init}.\nLet mutable k be 1.\nWhile k is at most 400:\n\
         \x20   Set h to {update}.\n\x20   Set k to k + 1.\nShow {fold}.\n"
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
        assert!(
            region_ok >= 1,
            "[{label}] did NOT tier its hot region (region={region_ok}) — a loop-carried Word live-in \
             declined; needs a SlotKind::Word entry-unbox + back-edge re-box\n{src}"
        );
    }
}

#[test]
fn forge_word32_state_lane() {
    // A Word32 hash-word shape: rotate + AND + OR mixing, carried across the loop (the sha256 `h` pattern).
    word_state_eq_tw(
        "word32 state: rotr+and mixing",
        "word32(2654435769)",
        "word_and(rotr(h, 7), word_or(h, word32(k)))",
        "intOfWord32(h)",
    );
    // Word32 state updated by a logical shift + complement — exercises word32Shr / word_not on a live-in.
    word_state_eq_tw(
        "word32 state: shr+not",
        "word32(305419896)",
        "word_not(word32Shr(word_or(h, word32(k)), 3))",
        "intOfWord32(h)",
    );
    // Word32 modular ADD carried as state (a + k) mod 2³² via the Int round-trip — the sha256 `h += a` step.
    word_state_eq_tw(
        "word32 state: modular add",
        "word32(0)",
        "word32(intOfWord32(h) + k)",
        "intOfWord32(h)",
    );
}

#[test]
fn forge_word64_state_lane() {
    // A Word64 accumulator carried across the loop (rotate-analog via shifts + AND) — the 64-bit hash shape.
    word_state_eq_tw(
        "word64 state: shl/shr+and mixing",
        "word64(1099511628211)",
        "word64And(word64Shl(h, 1), word64(0 - k))",
        "intOfWord64(h)",
    );
    // Word64 logical right shift + complement carried as state, seeded with the HIGH BIT set (a bit-63-set
    // Word64 exceeds the Int-literal range, so `word64(0 - N)` builds it as `2⁶⁴ − N` — the lane's idiom for
    // a high-bit carrier). Exercises `word64Shr`'s logical (non-sign-extending) shift on a live-in.
    word_state_eq_tw(
        "word64 state: shr+not",
        "word64(0 - 6364136223846793005)",
        "word_not(word64Shr(h, 5))",
        "intOfWord64(h)",
    );
}
