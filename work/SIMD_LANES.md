# SIMD lane-vector types — Logos-native vectorized crypto (the L3-via-types build)

## Thesis

No compiler auto-synthesizes 8-way ChaCha / 4-way Poly1305 from scalar source (LLVM/GCC/ICC
included — every crypto lib hand-writes the SIMD). But that is the *impossible* framing of L3.
The achievable, **better** framing — the one Jasmin and Rust `std::simd` use — is:

> Add a **lane-vector type** to Logos whose ops map **1:1 to SIMD intrinsics**. Then the
> vectorized algorithm is *written in Logos* and compiles to the **same instructions** as the
> hand-written AVX2. "As good as hand-written" holds **by construction** — the Logos lane op
> *is* the intrinsic, spelled in Logos.

This is the **Word playbook one level up**: `Word8/16/32/64` added a ℤ/2ⁿ *scalar* ring; this
adds a fixed-width **lane vector** over that ring. The hand-written kernels (`chacha20_xor_avx2`,
`poly1305_avx2`, `mlkem_ntt_w16`, `keccak_f1600_x4`) are **kept as the differential oracle** the
Logos-vector versions are proven byte-equal against.

This **supersedes L2** (recognize-scalar → call hand-written kernel): we don't recognize and
substitute, we *compile* Logos-vector source to those instructions.

## Surface (string-matched primitives, exactly like Word)

A 256-bit AVX2 register, typed by lane config. Mirror `Word8/16/32/64`:

| Logos type | hardware | lanes | kernels that use it |
|---|---|---|---|
| `Lanes8Word32` | `__m256i` | 8×u32 | ChaCha (add/xor/rot/shuffle) |
| `Lanes16Word16` | `__m256i` | 16×u16 | NTT (mullo/mulhi/sub/blend) |
| `Lanes4Word64` | `__m256i` | 4×u64 | Poly1305 (mul_epu32/add64), Keccak (xor/andnot/rot64) |

Generic `Lanes of N T` is a later sugar; fixed names first (the Word precedent: string-matched
primitives, no parametric machinery needed to start). wasm `v128` is 128-bit → each `Lanes*`
lowers to **two** `v128` (or the scalar fallback); portability preserved.

## Op vocabulary (sized from the actual kernels — closed, not open-ended)

- **Lane arithmetic**: `+ - *` (mul-lo), widening-even-mul (`vpmuludq`, Poly1305) — per element width.
- **Lane bitwise**: `and or xor`, `andnot` (Keccak), `not`, `shift left/right`, `rotate` (compose).
- **Cross-lane**: `splat`(broadcast), lane-init from a Seq, `shuffle`(`vpshufb`, ChaCha rot16/8 +
  transposes), `permute`(32-bit lane), `blend/select`, `unpack/interleave`(transpose + hsum),
  `reduce`(horizontal sum).
- **Memory**: `load`/`store` ↔ `Seq of Word*` (+ the lane-major↔block-major transpose helper).

Each maps to one intrinsic family: `_mm256_add/sub/mullo/mul_epu32`, `_xor/and/andnot/or`,
`_slli/srli`, `_shuffle_epi8/_permutevar8x32`, `_blendv`, `_unpacklo/hi`, `set1/setr`, `loadu/storeu`.

## Seams (parallel to Word — from the seam survey)

1. **Parser primitive gate** — `logicaffeine_language/src/parser/mod.rs:689` & `:703`
   (`"Word8"|...|"Word64"` → add `"Lanes8Word32"|"Lanes16Word16"|"Lanes4Word64"`).
2. **AST** — `TypeExpr::Primitive(Symbol)` (`ast/stmt.rs:28`) already carries it as a name; no new variant for fixed types.
3. **Type registry / discovery** — `analysis/registry.rs:158`, `analysis/discovery.rs:975` & `:1408`.
4. **Runtime value** — `interpreter.rs:1031` (`RuntimeValue::Word`) → add `RuntimeValue::Lanes(LanesVal)`; `type_name()` `:1228`.
5. **Scalar-lane semantics (the SPEC + oracle)** — `semantics/arith.rs:283` `word_binary_op` → lane path = element-wise zip over `[Word;N]`.
6. **Builtins** (constructors: splat, from-Seq, lane-get, reduce, shuffle) — `semantics/builtins.rs:98/152/828`.
7. **Codegen type→Rust** — `codegen/types.rs:108` (`Lanes8Word32` → a `repr(transparent)` newtype over `__m256i` w/ a scalar `[u32;8]` fallback).
8. **Codegen op emit** — `codegen/expr.rs:1016` (lane ops → the newtype's `impl Add/BitXor/...` which wrap the intrinsics; portable, like Word's trait overloads).
9. **Base substrate** — `logicaffeine_base/src/word.rs:171` → add `impl_lanes!` defining `Lanes8Word32([u32;8])` etc. with **two backends behind a cfg**: AVX2 intrinsics (native+`target_feature`) and the scalar `[T;N]` fallback (wasm/portability/oracle).

## Proof (two obligations, both gate-able today, ratcheting to formal)

1. **Codegen correctness**: AOT(AVX2) == tw(scalar-lane) per width — the **existing tw==vm/aot
   differential invariant** already does this for free (tw runs scalar lanes, aot runs AVX2; a
   mismatch fails the differential). Deepen with bitblast that each intrinsic = its lane semantics (F3).
2. **Spec equivalence**: the vectorized Logos crypto ≡ the scalar Logos spec (8-way ChaCha ≡
   1-way = reassociating 8 independent blocks; 4-way Poly1305 = Horner with precomputed r⁴).
   Differential KAT now; e-graph/algebra (the ring reassociation) is the formal ratchet.

## Build order (Word's path)

1. `Lanes8Word32` type: parser → AST → registry → `RuntimeValue::Lanes` → scalar-lane `xor` in tw → codegen newtype+AVX2 → **RED test: tw==aot for lane xor**. *(the foundational increment)*
2. Grow ops: add32, shift, `splat`, lane-get, `vpshufb` shuffle, rotate. Re-express **ChaCha** `quarterRound`/block in Logos lanes → compiles to == `chacha20_xor_avx2`, proven byte-equal (KAT + oracle).
3. `Lanes4Word64` + `vpmuludq`/add64/hsum → re-express **Poly1305** → == `poly1305_avx2`.
4. `Lanes16Word16` + mullo/mulhi/sub/blend → re-express **NTT** → == `mlkem_ntt_w16`; this is the path to the AVX2 ML-DSA NTT too (write its NTT in lanes).
5. Retire the hand-written kernels (or keep `#[cfg(test)]` as the oracle). Crypto SIMD is now 100% Logos, compiled, proven.

## Non-goals (stated)
- L3-as-auto-synthesis (vectorize scalar with no vector types) — not attempted; no compiler does it.
- New width beyond 256-bit (AVX-512) — later; 256-bit covers the win.
