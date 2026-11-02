//! FORGE WORD-XOR (generic-operator) LANE — a Word `xor` goes through the GENERIC `Op::BitXor` (there is NO
//! `word_xor` builtin, so `sha256`'s `xor` MUST tier this way — unlike `and`/`or`, which take Words ONLY via
//! the `word_and`/`word_or` builtins; a generic `&`/`|` on Words is a tree-walker error, so it must NEVER
//! tier). `xor` of two masked Words STAYS masked (`a ^ b` of values `< 2ⁿ` is `< 2ⁿ`), so the forge tiers it
//! as the plain i64 `xor` with NO extra mask — the mirror of the Word ARITHMETIC lane (`+`/`-`/`*`) but even
//! simpler. Correctness = forge == the tree-walker; tiering (x86_64) = the hot region went native.

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::compile::{tw_outcome_with_args, vm_outcome_with_args};
use logicaffeine_compile::vm::NativeTier;
use logicaffeine_jit::ForgeTier;

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
            "[{label}] did NOT tier its hot region (region={region_ok}) — a generic Word `xor`/`&`/`|` \
             declined; the BitXor/BitAnd/BitOr kind-flow + gate must accept a same-width Word pair\n{src}"
        );
    }
}

#[test]
fn forge_word32_xor() {
    // Generic `xor` on Word32 — the `sha256` message-schedule shape (`rotl xor rotl xor …`).
    word_eq_tw("word32 xor", "intOfWord32(word32(k) xor word32(600 - k))");
    // A chained xor of three terms (the exact schedule `s0` / round shape).
    word_eq_tw(
        "word32 xor chained",
        "intOfWord32(rotl(word32(k), 7) xor rotl(word32(k), 18) xor word32Shr(word32(k), 3))",
    );
    // The sha256 `ch(e,f,g)` function: `(e & f) xor ((not e) & g)` — builtin `word_and` MIXED with generic `xor`.
    word_eq_tw(
        "sha256 ch shape",
        "intOfWord32(word_and(word32(k), word32(3 * k)) xor word_and(word_not(word32(k)), word32(600 - k)))",
    );
    // The sha256 `maj(a,b,c)`: `(a & b) xor (a & c) xor (b & c)` — two chained generic `xor`s over word_and.
    word_eq_tw(
        "sha256 maj shape",
        "intOfWord32(word_and(word32(k), word32(3 * k)) xor word_and(word32(k), word32(600 - k)) xor word_and(word32(3 * k), word32(600 - k)))",
    );
}

/// A generic `&` (or `|`) on a same-width Word pair is bitwise on every tier — the compiled tier's
/// `Word32 & Word32` is Rust's `BitAnd`, and `word_narrow_all_tiers` locks `a & b` on Word8/Word16. `&`/`|` of
/// two masked Words stays masked, so the forge tiers it as the plain i64 op with NO extra mask — exactly as it
/// tiers `xor`.
#[test]
fn forge_word_and_or_generic_tier_like_xor() {
    word_eq_tw(
        "word32 & (generic)",
        "intOfWord32(word32(k) & word32(600 - k))",
    );
    word_eq_tw(
        "word32 | (generic)",
        "intOfWord32(word32(k) | word32(600 - k))",
    );
    word_eq_tw(
        "word64 & (generic, high-bit carrier)",
        "intOfWord64(word64(0 - k) & word64(k))",
    );
    word_eq_tw(
        "word64 | (generic, high-bit carrier)",
        "intOfWord64(word64(0 - k) | word64(3 * k))",
    );
    // The sha256 `ch(e,f,g)` with generic operators throughout: `(e & f) xor ((not e) & g)`.
    word_eq_tw(
        "sha256 ch shape, generic &",
        "intOfWord32((word32(k) & word32(3 * k)) xor (word_not(word32(k)) & word32(600 - k)))",
    );
}

#[test]
fn forge_word64_xor() {
    // Generic `xor` on Word64 (high-bit carrier via `word64(0 - k)`) — full 64-bit xor.
    word_eq_tw("word64 xor", "intOfWord64(word64(0 - k) xor word64(k))");
    // Chained Word64 xor mixing rotates (the sha512 round shape).
    word_eq_tw(
        "word64 xor chained",
        "intOfWord64(rotl(word64(0 - k), 14) xor rotl(word64(0 - k), 18) xor word64Shr(word64(0 - k), 9))",
    );
}
