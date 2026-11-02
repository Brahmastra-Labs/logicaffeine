//! FORGE WORD-ARITHMETIC LANE — modular `+`/`-`/`*` on ring values tiers the forge. Unlike the bitwise
//! Word lane (`word_and`/`rotr`/… are builtins), `Word + Word` / `Word - Word` / `Word * Word` go through
//! the GENERIC `Op::Add`/`Sub`/`Mul` (arith.rs dispatches both-Word operands to `WordVal::add`/`sub`/`mul`,
//! i.e. modular 2ⁿ). The forge must tier those with the SAME masked-i64 carrier: a Word32 op is the i64
//! op then a `& 0xFFFF_FFFF` re-mask (the operands are `< 2³²`, so their sum/product can't overflow i64 —
//! no BigInt deopt); a Word64 op is a WRAPPING i64 op (modular 2⁶⁴, no mask, no overflow deopt). This is
//! the arithmetic `sha256`/chacha20 need (`hh + bigS1 + ch + …`, all Word32). Correctness = forge == the
//! tree-walker; tiering (x86_64) = the hot region went native (else the Word arithmetic declined to VM).

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::compile::{tw_outcome_with_args, vm_outcome_with_args};
use logicaffeine_compile::vm::NativeTier;
use logicaffeine_jit::ForgeTier;

/// Fold a Word expression back to an Int accumulator over a hot loop (`k = 1..400`); assert forge ==
/// tree-walker AND (x86_64) that the region tiered — a Word `+`/`-`/`*` must lower natively, not decline.
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
        assert!(
            region_ok >= 1,
            "[{label}] did NOT tier its hot region (region={region_ok}) — Word arithmetic declined; the \
             Add/Sub/Mul gate + translate must accept a masked-i64 Word carrier\n{src}"
        );
    }
}

#[test]
fn forge_word32_arithmetic() {
    // Modular ADD: two Word32 summed then folded — must wrap at 2³² (the `word32(600 - k)` term is small
    // here, but the SUM path is what `sha256` walks). The result masks to 32 bits.
    word_eq_tw("word32 +", "intOfWord32(word32(k) + word32(600 - k))");
    // A three-term add (the `t1 = hh + bigS1 + ch` shape) — chained modular adds, each masked.
    word_eq_tw(
        "word32 + chained",
        "intOfWord32(word32(k) + word32(2 * k) + word32(600 - k))",
    );
    // Modular SUBTRACT — wraps to a large Word32 when the difference is negative (`k - (600-k)` < 0 for
    // small k), matching `WordVal::sub`'s 2³² modular borrow.
    word_eq_tw("word32 -", "intOfWord32(word32(k) - word32(600 - k))");
    // Modular MULTIPLY — the product of two Word32 can exceed 2³² and must mask to the low 32 bits.
    word_eq_tw("word32 *", "intOfWord32(word32(k) * word32(600 - k))");
    // ADD mixed with the bitwise lane (the real crypto shape: `rotl(a,7) + word32(k)`).
    word_eq_tw(
        "word32 + with rotate",
        "intOfWord32(rotl(word32(k), 7) + word32(k))",
    );
}

#[test]
fn forge_word64_arithmetic() {
    // Modular ADD on Word64 — WRAPPING at 2⁶⁴ (no mask); the high-bit-set carrier (`word64(0 - k)`) plus
    // another Word64 must wrap, matching `WordVal::add` on W64.
    word_eq_tw("word64 +", "intOfWord64(word64(0 - k) + word64(k))");
    // Modular SUBTRACT on Word64.
    word_eq_tw("word64 -", "intOfWord64(word64(k) - word64(2 * k))");
    // Modular MULTIPLY on Word64 — the product wraps at 2⁶⁴.
    word_eq_tw("word64 *", "intOfWord64(word64(k) * word64(k))");
}
