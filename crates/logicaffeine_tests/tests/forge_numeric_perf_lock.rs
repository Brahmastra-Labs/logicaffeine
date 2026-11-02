//! FORGE NUMERIC-BUILTIN TIERING — correctness + speed lock for the no-stencil integer builtins the
//! forge now lowers natively (`abs`, `min`, `max`, composed from existing MicroOps). Two claims per
//! builtin, split by what depends on x86:
//!   • CORRECTNESS (every architecture): the forge run == the pure-VM run. The forge either emits native
//!     code or declines to the VM — either way the answer matches. The VM is the oracle (no hardcoded
//!     expected), so this is a pure differential.
//!   • SPEED (x86_64 only): once the hot region tiers to native it beats the bytecode interpreter by a
//!     wide margin (measured ~30–50×). A DELIBERATELY loose 3× floor with best-of-3 timing can't flake
//!     under load — contention slows both runs on the same box, so the RATIO holds. Off x86 the forge
//!     declines to the VM, so the speed claim is x86-gated; correctness still holds there.

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::compile::vm_outcome_with_args;
use logicaffeine_compile::vm::NativeTier;
use logicaffeine_jit::ForgeTier;
use std::time::{Duration, Instant};

/// (label, hot program) — each calls the builtin ~8M times over values that exercise both branches.
const CASES: &[(&str, &str)] = &[
    (
        "abs",
        "## Main\nLet mutable acc be 0.\nLet mutable k be 1.\nWhile k is at most 8000000:\n\
         \x20   Let x be 4000000 - k.\n\x20   Set acc to acc + abs(x).\n\x20   Set k to k + 1.\nShow acc.\n",
    ),
    (
        "min",
        "## Main\nLet mutable acc be 0.\nLet mutable k be 1.\nWhile k is at most 8000000:\n\
         \x20   Set acc to acc + min(k, 8000000 - k).\n\x20   Set k to k + 1.\nShow acc.\n",
    ),
    (
        "max",
        "## Main\nLet mutable acc be 0.\nLet mutable k be 1.\nWhile k is at most 8000000:\n\
         \x20   Set acc to acc + max(k, 8000000 - k).\n\x20   Set k to k + 1.\nShow acc.\n",
    ),
    // `count_ones(k)` on the register-pinned induction variable — a single x86 `popcnt` via the
    // register-form `V_POPCNT` stencil vs the VM's own `popcnt` bytecode. The VM already lowers
    // `.count_ones()` to one native instruction, so this is the sternest speed case (native-vs-native
    // loop overhead, not composed-vs-native); the copy-and-patch chain still clears the 3× floor.
    (
        "count_ones",
        "## Main\nLet mutable acc be 0.\nLet mutable k be 1.\nWhile k is at most 8000000:\n\
         \x20   Set acc to acc + count_ones(k).\n\x20   Set k to k + 1.\nShow acc.\n",
    ),
    // `leading_zeros(k)` / `trailing_zeros(k)` on the register-pinned induction variable — a single
    // `lzcnt`/`tzcnt` via the register-form stencils vs the VM's own native instruction. Same stern
    // native-vs-native shape as count_ones; the copy-and-patch chain still clears the 3× floor.
    (
        "leading_zeros",
        "## Main\nLet mutable acc be 0.\nLet mutable k be 1.\nWhile k is at most 8000000:\n\
         \x20   Set acc to acc + leading_zeros(k).\n\x20   Set k to k + 1.\nShow acc.\n",
    ),
    (
        "trailing_zeros",
        "## Main\nLet mutable acc be 0.\nLet mutable k be 1.\nWhile k is at most 8000000:\n\
         \x20   Set acc to acc + trailing_zeros(k).\n\x20   Set k to k + 1.\nShow acc.\n",
    ),
    // `floor(fk)` / `ceil(fk)` over a hot XMM-pinned float — a single `roundsd` + `cvttsd2si` per iteration
    // vs the VM's boxed builtin dispatch. The float source stays XMM-pinned (the op is `xmm_pin_safe`).
    (
        "floor",
        "## Main\nLet mutable acc be 0.\nLet mutable fk be 0.5.\nLet mutable k be 1.\nWhile k is at most 8000000:\n\
         \x20   Set acc to acc + floor(fk).\n\x20   Set fk to fk + 1.0.\n\x20   Set k to k + 1.\nShow acc.\n",
    ),
    (
        "ceil",
        "## Main\nLet mutable acc be 0.\nLet mutable fk be 0.5.\nLet mutable k be 1.\nWhile k is at most 8000000:\n\
         \x20   Set acc to acc + ceil(fk).\n\x20   Set fk to fk + 1.0.\n\x20   Set k to k + 1.\nShow acc.\n",
    ),
    // `round(fk)` over a hot XMM-pinned float — roundsd-nearest + a ties-away check per iteration vs the
    // VM's boxed builtin. Every fk is a .5 tie, so the ties-away path runs every iteration.
    (
        "round",
        "## Main\nLet mutable acc be 0.\nLet mutable fk be 0.5.\nLet mutable k be 1.\nWhile k is at most 8000000:\n\
         \x20   Set acc to acc + round(fk).\n\x20   Set fk to fk + 1.0.\n\x20   Set k to k + 1.\nShow acc.\n",
    ),
    // `pow(k, 2)` on the register-pinned induction variable — the constant exponent unrolls to one checked
    // Mul, vs the VM's boxed builtin dispatch. `Set acc to …` (overwrite, not `+`) keeps the value bounded
    // (k² ≤ 6.4e13 < i64::MAX) so it stays native for all 8M iterations instead of promoting to BigInt.
    (
        "pow",
        "## Main\nLet mutable acc be 0.\nLet mutable k be 1.\nWhile k is at most 8000000:\n\
         \x20   Set acc to pow(k, 2).\n\x20   Set k to k + 1.\nShow acc.\n",
    ),
    // WORD lane — a Word rides as a masked i64 (BitAnd/Move MicroOps) vs the VM's BOXED `RuntimeValue::Word`
    // enum dispatch through `call_builtin` per op. `intOfWord32(word32(k))` is the producer+converter
    // round-trip; `word_and(…)` adds the bitwise op. Both stay bounded (masked ≤ 2³²) so acc fits i64.
    (
        "word32_roundtrip",
        "## Main\nLet mutable acc be 0.\nLet mutable k be 1.\nWhile k is at most 8000000:\n\
         \x20   Set acc to intOfWord32(word32(k)).\n\x20   Set k to k + 1.\nShow acc.\n",
    ),
    (
        "word_and",
        "## Main\nLet mutable acc be 0.\nLet mutable k be 1.\nWhile k is at most 8000000:\n\
         \x20   Set acc to intOfWord32(word_and(word32(k), word32(8000000 - k))).\n\x20   Set k to k + 1.\nShow acc.\n",
    ),
    // `length of chr(...)` in a hot loop — the `ChrGuard` validates each code (kept in the valid,
    // surrogate-free range `1 + (k mod 2000)` so it NEVER deopts) and `Utf8Len` computes the UTF-8 byte
    // length, both register-threaded, vs the VM's BOXED `chr` (a heap `Rc<String>`) + boxed `length`
    // dispatch every iteration. The 1..2000 span exercises both the 1-byte (ASCII) and 2-byte (Latin-1/BMP)
    // `Utf8Len` paths; avoiding the heap alloc is where the copy-and-patch chain clears the 3× floor.
    (
        "chr_length",
        "## Main\nLet mutable acc be 0.\nLet mutable k be 1.\nWhile k is at most 8000000:\n\
         \x20   Set acc to acc + length of chr(1 + (k mod 2000)).\n\x20   Set k to k + 1.\nShow acc.\n",
    ),
    // A loop-carried Word32 accumulator (`h`, the `sha256`/MD5 hash-word shape) mixed each iteration by
    // rotate + AND + OR — the whole Word chain rides UNBOXED as a masked i64 in the tiered region (entry
    // guard unboxes the live-in `RuntimeValue::Word`, the back-edge re-boxes) vs the VM's BOXED
    // `RuntimeValue::Word` enum dispatch through `call_builtin` per op AND a boxed re-store of `h` every
    // iteration. This is the state shape that makes real crypto tier, not just per-iteration Word math.
    (
        "word32_state_mix",
        "## Main\nLet mutable h be word32(2654435769).\nLet mutable k be 1.\nWhile k is at most 8000000:\n\
         \x20   Set h to word_and(rotr(h, 7), word_or(h, word32(k))).\n\x20   Set k to k + 1.\nShow intOfWord32(h).\n",
    ),
    // Modular Word32 ADDITION in a hot loop — the `sha256`/chacha20 arithmetic (`hh + bigS1 + ch + …`) as a
    // masked-i64 add + `& 0xFFFF_FFFF` re-mask vs the VM's BOXED `RuntimeValue::Word` `+` through the generic
    // arith dispatch per op. `Set acc to …` (overwrite, bounded) keeps it native for all 8M iters.
    (
        "word32_add",
        "## Main\nLet mutable acc be 0.\nLet mutable k be 1.\nWhile k is at most 8000000:\n\
         \x20   Set acc to intOfWord32(word32(k) + word32(3 * k) + word32(8000000 - k)).\n\x20   Set k to k + 1.\nShow acc.\n",
    ),
    // Word64 ROTATE carried as STATE (the xoshiro256/PRNG + sha512/keccak substrate) — `rotl` composes as
    // `(w « n) | (w »u (64−n))` where the logical `»u` is the new `Ushr` micro-op (native `shr cl`), plus a
    // modular Word64 `+`, all riding UNBOXED as a masked i64 across the loop; only the final `Show` folds to
    // an Int (so the high-bit carrier never forces a per-iteration BigInt). Vs the VM's BOXED
    // `RuntimeValue::Word` `rotate_left` + `add` + boxed re-store of `h` every iteration.
    (
        "word64_rotate",
        "## Main\nLet mutable h be word64(88172645463325252).\nLet mutable k be 1.\nWhile k is at most 8000000:\n\
         \x20   Set h to rotl(h, 23) + word64(k).\n\x20   Set k to k + 1.\nShow intOfWord64(h).\n",
    ),
    // The sha256 message-schedule `s0` — a chained generic `xor` over Word32 rotates/shifts (`rotl(w,25)
    // xor rotl(w,14) xor word32Shr(w,3)`), the generic `xor` riding the plain i64 `xor` MicroOp (mask-
    // preserving) vs the VM's BOXED `RuntimeValue::Word` `bitxor` per op.
    (
        "word32_xor",
        "## Main\nLet mutable acc be 0.\nLet mutable k be 1.\nWhile k is at most 8000000:\n\
         \x20   Set acc to intOfWord32(rotl(word32(k), 25) xor rotl(word32(k), 14) xor word32Shr(word32(k), 3)).\n\x20   Set k to k + 1.\nShow acc.\n",
    ),
    // A `Seq of Word32` TABLE read in a hot hash loop (the sha256 `item i of kk`/`w` shape) — the packed
    // `Words32` list pins with the IntList buffer machinery and `item idx of tbl` loads a `Word32`, fed into
    // a `xor`+`*` mix, vs the VM's BOXED `RuntimeValue::Word` list + per-index box/unbox. The whole hash
    // loop rides native over the pinned table.
    (
        "word32_table_hash",
        "## Main\nLet mutable tbl be a new Seq of Word32.\nLet mutable s be 0.\n\
         While s is less than 64:\n\x20   Push word32(s * 2654435761) to tbl.\n\x20   Set s to s + 1.\n\
         Let mutable h be word32(2166136261).\nLet mutable k be 1.\nWhile k is at most 8000000:\n\
         \x20   Let idx be (k mod 64) + 1.\n\x20   Set h to (h xor (item idx of tbl)) * word32(16777619).\n\x20   Set k to k + 1.\nShow intOfWord32(h).\n",
    ),
];

fn best_of<F: Fn() -> String>(n: usize, run: F) -> (Duration, String) {
    let mut best = Duration::MAX;
    let mut out = String::new();
    for _ in 0..n {
        let t = Instant::now();
        out = run();
        best = best.min(t.elapsed());
    }
    (best, out)
}

#[test]
fn forge_numeric_builtins_match_vm_and_tier_faster() {
    for (label, src) in CASES {
        let (forge_t, forge_out) = best_of(3, || {
            let tier = ForgeTier::new();
            vm_outcome_with_args(src, &[], Some(&tier as &dyn NativeTier)).output
        });
        let (vm_t, vm_out) = best_of(3, || vm_outcome_with_args(src, &[], None).output);

        // CORRECTNESS — every architecture; the VM is the oracle.
        assert_eq!(
            forge_out.trim(),
            vm_out.trim(),
            "[{label}] forge disagrees with the VM (forge={:?} vm={:?})",
            forge_out.trim(),
            vm_out.trim()
        );

        eprintln!(
            "forge {label}: forge={:.1}ms vm={:.1}ms speedup={:.1}x",
            forge_t.as_secs_f64() * 1e3,
            vm_t.as_secs_f64() * 1e3,
            vm_t.as_secs_f64() / forge_t.as_secs_f64()
        );

        // SPEED — x86_64 only (native stencils); loose 3× floor vs a measured ~30–50×.
        #[cfg(target_arch = "x86_64")]
        {
            assert!(
                forge_t.as_secs_f64() * 3.0 < vm_t.as_secs_f64(),
                "[{label}] forge was not >=3x faster than the VM (forge={:.1}ms vm={:.1}ms) — the hot \
                 region stopped tiering; check the codegen / tier-up",
                forge_t.as_secs_f64() * 1e3,
                vm_t.as_secs_f64() * 1e3
            );
        }
    }
}
