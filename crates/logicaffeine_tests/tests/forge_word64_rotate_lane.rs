//! FORGE WORD64 ROTATE / VARIABLE-SHIFT LANE — `rotl`/`rotr` on a `Word64` and a VARIABLE-count
//! `word64Shr` need a LOGICAL (unsigned) shift-right, which the i64 `Shr` (arithmetic) can't provide for a
//! high-bit-set carrier. The forge grows a `Ushr` micro-op (`(x as u64) >> n`) and composes the rotates
//! from it: `rotl(w,n) = (w << n) | (w ushr (64-n))`, `rotr(w,n) = (w ushr n) | (w << (64-n))`, all masked
//! to 2⁶⁴ by the i64 width. This is the scalar-Word64 substrate of PRNGs (xoshiro256, PCG) and the 64-bit
//! hash families (sha512, keccak, blake2). Correctness = forge == the tree-walker; tiering (x86_64) = the
//! hot region went native (else the Word64 rotate/variable-shift declined to the VM).

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::compile::{tw_outcome_with_args, vm_outcome_with_args};
use logicaffeine_compile::vm::NativeTier;
use logicaffeine_jit::ForgeTier;

/// Fold a Word64 expression to an Int accumulator over a hot loop (`k = 1..400`); assert forge ==
/// tree-walker AND (x86_64) that the region tiered.
fn w64_eq_tw(label: &str, expr: &str) {
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
        assert!(
            region_ok >= 1,
            "[{label}] did NOT tier its hot region (region={region_ok}) — Word64 rotate / variable-shift \
             declined; the forge needs a `Ushr` (logical shift-right) micro-op\n{src}"
        );
    }
}

#[test]
fn forge_word64_rotate() {
    // Word64 rotate-LEFT — the wraparound is the point; use a HIGH-bit-set carrier (`word64(0 - k)`) so the
    // logical `ushr` (not arithmetic) is exercised. Include identity/full-turn edges.
    w64_eq_tw("rotl (Word64)", "intOfWord64(rotl(word64(0 - k), 23))");
    w64_eq_tw("rotr (Word64)", "intOfWord64(rotr(word64(0 - k), 17))");
    w64_eq_tw("rotl (Word64) small n", "intOfWord64(rotl(word64(k), 1))");
    w64_eq_tw(
        "rotr (Word64) large n",
        "intOfWord64(rotr(word64(0 - k), 63))",
    );
    w64_eq_tw(
        "rotl by 0 (identity)",
        "intOfWord64(rotl(word64(0 - k), 0))",
    );
    // The xoshiro256++ inner term shape: `rotl(a + b, 23) + a` on Word64 scalars.
    w64_eq_tw(
        "xoshiro rotl term",
        "intOfWord64(rotl(word64(0 - k) + word64(k * 3), 23) + word64(0 - k))",
    );
}

#[test]
fn forge_word64_variable_shift() {
    // VARIABLE-count logical right shift on a high-bit-set Word64 — the count (`k mod 63`) is not a
    // constant, so the const-shift lane can't apply; `Ushr` handles it directly.
    w64_eq_tw(
        "word64Shr variable count",
        "intOfWord64(word64Shr(word64(0 - k), k mod 63))",
    );
    // A variable shift then complement (the mixing shape).
    w64_eq_tw(
        "word64Shr variable + not",
        "intOfWord64(word_not(word64Shr(word64(0 - k), (k mod 31) + 1)))",
    );
}
