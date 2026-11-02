//! Every native kernel of `crypto.lg` holds its arguments to a contract, and a program that breaks it
//! stops with the kernel's name and the reason, identically on the tree-walker, the VM, the optimized
//! VM, the native tier and AOT-Rust. Nothing is folded into a value the program did not give: `[256]`
//! is not the byte 0, a 31-byte seed is not padded to 32, a 256-byte context is not the empty one,
//! and a short polynomial is not zero-extended. Two inputs come from the other side of a protocol and
//! are answered, not failed: an encapsulation key that fails FIPS 203's checks makes
//! `mlkemEncapsKem` return an empty sequence, a ciphertext of the wrong length makes
//! `mlkemDecapsKem` return one, and a malformed signature, key or context makes `mldsaVerify`
//! answer 0.

#![cfg(not(target_arch = "wasm32"))]

mod common;
use common::{assert_panics, run_all_tiers};

/// Helpers every case builds its arguments from: bytes, ML-KEM coefficients, and valid keys.
const HELPERS: &str = r#"## To bytesOf (n: Int) and (k: Int) -> Seq of Int:
    Let mutable r be a new Seq of Int.
    Repeat for i from 1 to n:
        Push (i * k + 11) % 256 to r.
    Return r.

## To coeffsOf (n: Int) and (k: Int) -> Seq of Int:
    Let mutable r be a new Seq of Int.
    Repeat for i from 1 to n:
        Push (i * k + 7) % 3329 to r.
    Return r.

## To wordsOf (n: Int) and (k: Int) -> Seq of Word16:
    Let mutable r be a new Seq of Word16.
    Repeat for i from 1 to n:
        Push word16((i * k + 7) % 3329) to r.
    Return r.

## To withItem (s: Seq of Int) and (at: Int) and (x: Int) -> Seq of Int:
    Let mutable r be copy of s.
    Set item at of r to x.
    Return r.

## To withWord (s: Seq of Word16) and (at: Int) and (x: Int) -> Seq of Word16:
    Let mutable r be copy of s.
    Set item at of r to word16(x).
    Return r.

"#;

fn program(main: &str) -> String {
    format!("{HELPERS}## Main\n{main}")
}

/// `main` stops with exactly `reason` on every interpreted tier.
fn fails_with(main: &str, reason: &str) {
    let src = program(main);
    let r = run_all_tiers(&src);
    assert_eq!(
        r.error.as_deref(),
        Some(reason),
        "every tier stops with the kernel's reason\n--- program ---\n{src}\n--- output ---\n{}",
        r.output
    );
}

/// `main` runs cleanly on every interpreted tier and prints exactly `expected`.
fn answers(main: &str, expected: &str) {
    let src = program(main);
    let r = run_all_tiers(&src);
    assert_eq!(r.error, None, "the program runs\n{src}");
    assert_eq!(r.output.trim(), expected, "\n{src}");
}

// ─── the ML-KEM arithmetic kernels (Seq of Int) ───────────────────────────────────────────────

#[test]
fn mlkem_ntt_takes_exactly_one_polynomial() {
    fails_with(
        "Show mlkemNtt([1, 2, 3]).\n",
        "mlkemNtt(): the polynomial has 3 coefficients; it must have 256",
    );
    fails_with(
        "Show mlkemNtt(coeffsOf(257, 3)).\n",
        "mlkemNtt(): the polynomial has 257 coefficients; it must have 256",
    );
}

#[test]
fn mlkem_inv_ntt_takes_one_polynomial_of_reduced_coefficients() {
    fails_with(
        "Show mlkemInvNtt(coeffsOf(255, 3)).\n",
        "mlkemInvNtt(): the polynomial has 255 coefficients; it must have 256",
    );
    fails_with(
        "Show mlkemInvNtt(withItem(coeffsOf(256, 3), 5, 3329)).\n",
        "mlkemInvNtt(): the polynomial must hold values from 0 to 3328; item 5 is 3329",
    );
    fails_with(
        "Show mlkemInvNtt(withItem(coeffsOf(256, 3), 256, -1)).\n",
        "mlkemInvNtt(): the polynomial must hold values from 0 to 3328; item 256 is -1",
    );
}

#[test]
fn mlkem_base_mul_takes_two_polynomials_of_reduced_coefficients() {
    fails_with(
        "Show mlkemBaseMul(coeffsOf(256, 3), coeffsOf(128, 5)).\n",
        "mlkemBaseMul(): the second polynomial has 128 coefficients; it must have 256",
    );
    fails_with(
        "Show mlkemBaseMul(withItem(coeffsOf(256, 3), 9, 4000), coeffsOf(256, 5)).\n",
        "mlkemBaseMul(): the first polynomial must hold values from 0 to 3328; item 9 is 4000",
    );
}

#[test]
fn to_mont_takes_reduced_coefficients() {
    fails_with(
        "Show toMont(withItem(coeffsOf(256, 3), 1, 3329)).\n",
        "toMont(): the polynomial must hold values from 0 to 3328; item 1 is 3329",
    );
}

#[test]
fn cbd_takes_exactly_64_eta_bytes() {
    fails_with(
        "Show cbd2(bytesOf(127, 3)).\n",
        "cbd2(): the randomness is 127 bytes; it must be 128",
    );
    fails_with(
        "Show cbd2(withItem(bytesOf(128, 3), 1, 256)).\n",
        "cbd2(): the randomness must be bytes (0 to 255); item 1 is 256",
    );
    fails_with(
        "Show cbd3(bytesOf(193, 3)).\n",
        "cbd3(): the randomness is 193 bytes; it must be 192",
    );
}

#[test]
fn compress_and_decompress_take_a_width_below_twelve() {
    fails_with(
        "Show compress(coeffsOf(256, 3), 12).\n",
        "compress(): the width d must be from 1 to 11; it is 12",
    );
    fails_with(
        "Show compress(coeffsOf(256, 3), 0).\n",
        "compress(): the width d must be from 1 to 11; it is 0",
    );
    fails_with(
        "Show decompress(coeffsOf(4, 0), 64).\n",
        "decompress(): the width d must be from 1 to 11; it is 64",
    );
    fails_with(
        "Show decompress([1, 16, 3], 4).\n",
        "decompress(): the compressed values must hold values from 0 to 15; item 2 is 16",
    );
}

#[test]
fn byte_encode_and_decode_take_one_polynomial_of_the_width() {
    fails_with(
        "Show byteEncode(coeffsOf(256, 3), 13).\n",
        "byteEncode(): the width d must be from 1 to 12; it is 13",
    );
    fails_with(
        "Show byteEncode(coeffsOf(255, 3), 12).\n",
        "byteEncode(): the polynomial has 255 coefficients; it must have 256",
    );
    fails_with(
        "Show byteEncode(withItem(coeffsOf(256, 3), 7, 3329), 12).\n",
        "byteEncode(): the polynomial must hold values from 0 to 3328; item 7 is 3329",
    );
    fails_with(
        "Show byteEncode(withItem(coeffsOf(256, 0), 2, 16), 4).\n",
        "byteEncode(): the polynomial must hold values from 0 to 15; item 2 is 16",
    );
    fails_with(
        "Show byteDecode(bytesOf(383, 3), 12).\n",
        "byteDecode(): the encoding is 383 bytes; it must be 384",
    );
    fails_with(
        "Show byteDecode(withItem(bytesOf(128, 3), 4, 999), 4).\n",
        "byteDecode(): the encoding must be bytes (0 to 255); item 4 is 999",
    );
}

#[test]
fn sample_ntt_needs_a_stream_long_enough_for_256_coefficients() {
    fails_with(
        "Show sampleNtt(bytesOf(3, 0)).\n",
        "sampleNtt(): the stream ran out after 2 of 256 coefficients",
    );
    fails_with(
        "Show sampleNtt(withItem(bytesOf(840, 73), 3, -7)).\n",
        "sampleNtt(): the stream must be bytes (0 to 255); item 3 is -7",
    );
}

#[test]
fn sample_a_takes_a_32_byte_seed_and_byte_indices() {
    fails_with(
        "Show sampleA(bytesOf(31, 5), 0, 1).\n",
        "sampleA(): the seed is 31 bytes; it must be 32",
    );
    fails_with(
        "Show sampleA(bytesOf(32, 5), 256, 1).\n",
        "sampleA(): the row index must be from 0 to 255; it is 256",
    );
    fails_with(
        "Show sampleA(bytesOf(32, 5), 0, -1).\n",
        "sampleA(): the column index must be from 0 to 255; it is -1",
    );
}

// ─── the Word16 carrier kernels ───────────────────────────────────────────────────────────────

#[test]
fn the_word16_base_multiply_takes_two_reduced_polynomials() {
    fails_with(
        "Show mlkemBaseMulW16(wordsOf(255, 3), wordsOf(256, 5)).\n",
        "mlkemBaseMulW16(): the first polynomial has 255 coefficients; it must have 256",
    );
    fails_with(
        "Show mlkemBaseMulW16(wordsOf(256, 3), withWord(wordsOf(256, 5), 3, 3329)).\n",
        "mlkemBaseMulW16(): the second polynomial must hold values from 0 to 3328; item 3 is 3329",
    );
}

#[test]
fn the_word16_montgomery_conversion_takes_reduced_coefficients() {
    fails_with(
        "Show toMontW16(withWord(wordsOf(256, 3), 2, 65535)).\n",
        "toMontW16(): the polynomial must hold values from 0 to 3328; item 2 is 65535",
    );
}

#[test]
fn the_word16_compress_takes_a_width_below_twelve_and_reduced_coefficients() {
    fails_with(
        "Show compressW16(wordsOf(256, 3), 12).\n",
        "compressW16(): the width d must be from 1 to 11; it is 12",
    );
    fails_with(
        "Show compressW16(withWord(wordsOf(256, 3), 1, 3329), 10).\n",
        "compressW16(): the polynomial must hold values from 0 to 3328; item 1 is 3329",
    );
}

#[test]
fn the_word16_decompress_takes_values_of_its_width() {
    fails_with(
        "Show decompressW16(withWord(wordsOf(256, 0), 4, 1024), 10).\n",
        "decompressW16(): the compressed values must hold values from 0 to 1023; item 4 is 1024",
    );
}

#[test]
fn the_word16_byte_encode_takes_a_width_to_twelve_and_values_of_it() {
    fails_with(
        "Show byteEncodeW16(wordsOf(256, 3), 0).\n",
        "byteEncodeW16(): the width d must be from 1 to 12; it is 0",
    );
    fails_with(
        "Show byteEncodeW16(withWord(wordsOf(256, 3), 256, 4095), 12).\n",
        "byteEncodeW16(): the polynomial must hold values from 0 to 3328; item 256 is 4095",
    );
}

#[test]
fn the_word16_byte_decode_takes_one_polynomials_encoding() {
    fails_with(
        "Show byteDecodeW16(bytesOf(319, 3), 10).\n",
        "byteDecodeW16(): the encoding is 319 bytes; it must be 320",
    );
}

#[test]
fn the_word16_cbd_takes_exactly_128_bytes() {
    fails_with(
        "Show cbd2W16(bytesOf(129, 3)).\n",
        "cbd2W16(): the randomness is 129 bytes; it must be 128",
    );
}

#[test]
fn the_word16_samplers_take_a_32_byte_seed_and_byte_indices() {
    fails_with(
        "Show sampleAW16(bytesOf(33, 5), 0, 0).\n",
        "sampleAW16(): the seed is 33 bytes; it must be 32",
    );
    fails_with(
        "Show sampleAW16(bytesOf(32, 5), 0, 300).\n",
        "sampleAW16(): the column index must be from 0 to 255; it is 300",
    );
    fails_with(
        "Show sampleMatrixW16(bytesOf(0, 5)).\n",
        "sampleMatrixW16(): the seed is 0 bytes; it must be 32",
    );
}

#[test]
fn the_noise_batch_takes_a_32_byte_seed_and_nonces_that_are_bytes() {
    fails_with(
        "Show mlkemNoiseBatch(bytesOf(31, 5), 0, 3).\n",
        "mlkemNoiseBatch(): the seed is 31 bytes; it must be 32",
    );
    fails_with(
        "Show mlkemNoiseBatch(bytesOf(32, 5), -1, 3).\n",
        "mlkemNoiseBatch(): the first nonce must be from 0 to 255; it is -1",
    );
    fails_with(
        "Show mlkemNoiseBatch(bytesOf(32, 5), 0, -1).\n",
        "mlkemNoiseBatch(): the count must be from 0 to 256; it is -1",
    );
    fails_with(
        "Show mlkemNoiseBatch(bytesOf(32, 5), 250, 10).\n",
        "mlkemNoiseBatch(): the nonces run from 250 to 259; each must be a byte (0 to 255)",
    );
    answers(
        "Show length of mlkemNoiseBatch(bytesOf(32, 5), 250, 6).\n",
        "1536",
    );
}

#[test]
fn modular_add_and_subtract_take_matching_reduced_polynomials() {
    fails_with(
        "Show addModQW16(wordsOf(256, 3), wordsOf(255, 5)).\n",
        "addModQW16(): the operands have 256 and 255 coefficients; they must match",
    );
    fails_with(
        "Show subModQW16(withWord(wordsOf(4, 3), 1, 3329), wordsOf(4, 5)).\n",
        "subModQW16(): the first operand must hold values from 0 to 3328; item 1 is 3329",
    );
    fails_with(
        "Show addModQW16(wordsOf(4, 3), withWord(wordsOf(4, 5), 4, 40000)).\n",
        "addModQW16(): the second operand must hold values from 0 to 3328; item 4 is 40000",
    );
}

#[test]
fn zeros_takes_a_count_that_is_not_negative() {
    fails_with(
        "Show zerosW16(-1).\n",
        "zerosW16(): the count must not be negative; it is -1",
    );
    answers("Show length of zerosW16(0).\n", "0");
}

// ─── ML-KEM-768 ───────────────────────────────────────────────────────────────────────────────

#[test]
fn mlkem_keypair_takes_two_32_byte_seeds() {
    fails_with(
        "Show mlkemKeypair(bytesOf(31, 5), bytesOf(32, 9)).\n",
        "mlkemKeypair(): the seed d is 31 bytes; it must be 32",
    );
    fails_with(
        "Show mlkemKeypair(bytesOf(32, 5), bytesOf(33, 9)).\n",
        "mlkemKeypair(): the seed z is 33 bytes; it must be 32",
    );
    fails_with(
        "Show mlkemKeypair(withItem(bytesOf(32, 5), 32, 256), bytesOf(32, 9)).\n",
        "mlkemKeypair(): the seed d must be bytes (0 to 255); item 32 is 256",
    );
}

const KEM_KEYS: &str = "Let kp be mlkemKeypair(bytesOf(32, 5), bytesOf(32, 9)).\nLet ek be mlkemSubseq(kp, 0, 1184).\nLet dk be mlkemSubseq(kp, 1184, 2400).\n";

#[test]
fn mlkem_encaps_takes_32_bytes_of_randomness() {
    fails_with(
        &format!("{KEM_KEYS}Show mlkemEncapsKem(ek, bytesOf(31, 17)).\n"),
        "mlkemEncapsKem(): the randomness m is 31 bytes; it must be 32",
    );
    fails_with(
        &format!("{KEM_KEYS}Show mlkemEncapsKem(withItem(ek, 1, 300), bytesOf(32, 17)).\n"),
        "mlkemEncapsKem(): the encapsulation key must be bytes (0 to 255); item 1 is 300",
    );
}

#[test]
fn mlkem_encaps_answers_an_encapsulation_key_that_fails_fips_203s_checks_with_nothing() {
    answers(
        &format!(
            "{KEM_KEYS}Show length of mlkemEncapsKem(mlkemSubseq(ek, 0, 1183), bytesOf(32, 17)).\nShow length of mlkemEncapsKem(ek followed by [0], bytesOf(32, 17)).\nShow length of mlkemEncapsKem(withItem(withItem(ek, 1, 255), 2, 15), bytesOf(32, 17)).\nShow length of mlkemEncapsKem(ek, bytesOf(32, 17)).\n"
        ),
        "0\n0\n0\n1120",
    );
}

#[test]
fn mlkem_decaps_fails_on_a_decapsulation_key_that_is_not_one() {
    fails_with(
        &format!("{KEM_KEYS}Let sealed be mlkemEncapsKem(ek, bytesOf(32, 17)).\nShow mlkemDecapsKem(mlkemSubseq(dk, 0, 2399), mlkemSubseq(sealed, 0, 1088)).\n"),
        "mlkemDecapsKem(): the decapsulation key is 2399 bytes; an ML-KEM-768 decapsulation key is 2400",
    );
    fails_with(
        &format!("{KEM_KEYS}Let sealed be mlkemEncapsKem(ek, bytesOf(32, 17)).\nShow mlkemDecapsKem(withItem(dk, 2337, (item 2337 of dk + 1) % 256), mlkemSubseq(sealed, 0, 1088)).\n"),
        "mlkemDecapsKem(): the decapsulation key's H(ek) is not the hash of its ek (FIPS 203 hash check)",
    );
    fails_with(
        &format!("{KEM_KEYS}Let sealed be mlkemEncapsKem(ek, bytesOf(32, 17)).\nShow mlkemDecapsKem(dk, withItem(mlkemSubseq(sealed, 0, 1088), 10, 256)).\n"),
        "mlkemDecapsKem(): the ciphertext must be bytes (0 to 255); item 10 is 256",
    );
}

#[test]
fn mlkem_decaps_answers_a_ciphertext_of_the_wrong_length_with_nothing() {
    answers(
        &format!("{KEM_KEYS}Let sealed be mlkemEncapsKem(ek, bytesOf(32, 17)).\nShow length of mlkemDecapsKem(dk, mlkemSubseq(sealed, 0, 1087)).\nShow length of mlkemDecapsKem(dk, sealed).\nShow length of mlkemDecapsKem(dk, mlkemSubseq(sealed, 0, 1088)).\n"),
        "0\n0\n32",
    );
}

// ─── ML-DSA-65 ────────────────────────────────────────────────────────────────────────────────

#[test]
fn mldsa_keypair_takes_a_32_byte_seed() {
    fails_with(
        "Show mldsaKeypair(bytesOf(31, 5)).\n",
        "mldsaKeypair(): the seed is 31 bytes; it must be 32",
    );
    fails_with(
        "Show mldsaKeypair(bytesOf(33, 5)).\n",
        "mldsaKeypair(): the seed is 33 bytes; it must be 32",
    );
    fails_with(
        "Show mldsaKeypair(withItem(bytesOf(32, 5), 9, -1)).\n",
        "mldsaKeypair(): the seed must be bytes (0 to 255); item 9 is -1",
    );
}

const DSA_KEYS: &str = "Let dkp be mldsaKeypair(bytesOf(32, 5)).\nLet pk be mlkemSubseq(dkp, 0, 1952).\nLet sk be mlkemSubseq(dkp, 1952, 4032).\nLet empty be a new Seq of Int.\n";

#[test]
fn mldsa_sign_refuses_a_context_longer_than_255_bytes() {
    fails_with(
        &format!("{DSA_KEYS}Show mldsaSign(sk, bytesOf(40, 31), bytesOf(256, 7)).\n"),
        "mldsaSign(): the context is 256 bytes; FIPS 204 allows at most 255",
    );
    fails_with(
        &format!("{DSA_KEYS}Show mldsaSign(sk, bytesOf(40, 31), bytesOf(300, 7)).\n"),
        "mldsaSign(): the context is 300 bytes; FIPS 204 allows at most 255",
    );
    answers(
        &format!("{DSA_KEYS}Let sig be mldsaSign(sk, bytesOf(40, 31), bytesOf(255, 7)).\nShow length of sig.\nShow mldsaVerify(pk, bytesOf(40, 31), bytesOf(255, 7), sig).\n"),
        "3309\n1",
    );
}

#[test]
fn mldsa_sign_refuses_a_secret_key_that_is_not_one_and_items_that_are_not_bytes() {
    fails_with(
        &format!(
            "{DSA_KEYS}Show mldsaSign(mlkemSubseq(sk, 0, 4031), bytesOf(40, 31), bytesOf(3, 1)).\n"
        ),
        "mldsaSign(): the secret key is 4031 bytes; an ML-DSA-65 secret key is 4032",
    );
    fails_with(
        &format!(
            "{DSA_KEYS}Show mldsaSign(withItem(sk, 129, 255), bytesOf(40, 31), bytesOf(3, 1)).\n"
        ),
        "mldsaSign(): the secret key holds an s1 or s2 coefficient outside [-4, 4]",
    );
    fails_with(
        &format!("{DSA_KEYS}Show mldsaSign(sk, [1, 300], bytesOf(3, 1)).\n"),
        "mldsaSign(): the message must be bytes (0 to 255); item 2 is 300",
    );
    fails_with(
        &format!("{DSA_KEYS}Show mldsaSign(sk, bytesOf(40, 31), [-1]).\n"),
        "mldsaSign(): the context must be bytes (0 to 255); item 1 is -1",
    );
    fails_with(
        &format!(
            "{DSA_KEYS}Show mldsaSign(withItem(sk, 1, 256), bytesOf(40, 31), bytesOf(3, 1)).\n"
        ),
        "mldsaSign(): the secret key must be bytes (0 to 255); item 1 is 256",
    );
}

#[test]
fn mldsa_verify_answers_zero_for_anything_malformed_including_the_reported_alias() {
    // The reported escape: a signature on (ctx = "", T) presented as (ctx = T[..256], T[256..]).
    answers(
        &format!(
            "{DSA_KEYS}Let t be bytesOf(300, 13).\nLet sig be mldsaSign(sk, t, empty).\nShow mldsaVerify(pk, t, empty, sig).\nShow mldsaVerify(pk, mlkemSubseq(t, 256, 44), mlkemSubseq(t, 0, 256), sig).\nShow mldsaVerify(pk, withItem(t, 1, 256 + item 1 of t), empty, sig).\nShow mldsaVerify(pk, t, [256], sig).\nShow mldsaVerify(pk, t, empty, mlkemSubseq(sig, 0, 3308)).\nShow mldsaVerify(pk, t, empty, sig followed by [0]).\nShow mldsaVerify(pk followed by [0], t, empty, sig).\nShow mldsaVerify(withItem(pk, 1, -1), t, empty, sig).\n"
        ),
        "1\n0\n0\n0\n0\n0\n0\n0",
    );
}

// ─── the compiled tier ────────────────────────────────────────────────────────────────────────

#[test]
#[ignore = "compiles cargo projects via rustc (slow) — a kernel's refusal reads the same compiled"]
fn a_kernels_refusal_reads_the_same_when_compiled() {
    for (main, reason) in [
        (
            format!("{DSA_KEYS}Show mldsaSign(sk, bytesOf(40, 31), bytesOf(256, 7)).\n"),
            "mldsaSign(): the context is 256 bytes; FIPS 204 allows at most 255",
        ),
        (
            "Show compress(coeffsOf(256, 3), 12).\n".to_string(),
            "compress(): the width d must be from 1 to 11; it is 12",
        ),
        (
            "Show mldsaKeypair(bytesOf(31, 5)).\n".to_string(),
            "mldsaKeypair(): the seed is 31 bytes; it must be 32",
        ),
    ] {
        fails_with(&main, reason);
        assert_panics(&program(&main), reason);
    }
}

#[test]
#[ignore = "compiles a cargo project via rustc (slow) — the peer answers read the same compiled"]
fn the_peer_answers_read_the_same_when_compiled() {
    let main = format!(
        "{KEM_KEYS}Show length of mlkemEncapsKem(mlkemSubseq(ek, 0, 1183), bytesOf(32, 17)).\nLet sealed be mlkemEncapsKem(ek, bytesOf(32, 17)).\nShow length of mlkemDecapsKem(dk, mlkemSubseq(sealed, 0, 1087)).\n{DSA_KEYS}Let t be bytesOf(300, 13).\nLet sig be mldsaSign(sk, t, empty).\nShow mldsaVerify(pk, mlkemSubseq(t, 256, 44), mlkemSubseq(t, 0, 256), sig).\n"
    );
    answers(&main, "0\n0\n0");
    let aot = common::run_logos(&program(&main));
    assert!(aot.success, "the compiled program runs:\n{}", aot.stderr);
    assert_eq!(
        aot.stdout.trim(),
        "0\n0\n0",
        "compiled, the peer answers are the same"
    );
}

/// One in-contract call of every kernel, so the Rust backend emits every kernel's wrapper.
const EVERY_KERNEL_CALL: &str = "Let a be coeffsOf(256, 13).
Let b be coeffsOf(256, 29).
Let aw be wordsOf(256, 13).
Let bw be wordsOf(256, 29).
Let seed be bytesOf(32, 5).
Show length of mlkemNtt(a).
Show length of mlkemInvNtt(a).
Show length of mlkemBaseMul(a, b).
Show length of cbd2(bytesOf(128, 3)).
Show length of cbd3(bytesOf(192, 7)).
Show length of compress(a, 4).
Show length of decompress(compress(a, 4), 4).
Show length of byteEncode(a, 12).
Show length of byteDecode(byteEncode(a, 12), 12).
Show length of sampleNtt(bytesOf(840, 73)).
Show length of sampleA(seed, 1, 2).
Show length of toMont(a).
Show length of mlkemBaseMulW16(aw, bw).
Show length of toMontW16(aw).
Show length of cbd2W16(bytesOf(128, 3)).
Show length of compressW16(aw, 10).
Show length of decompressW16(compressW16(aw, 10), 10).
Show length of byteEncodeW16(aw, 12).
Show length of byteDecodeW16(byteEncodeW16(aw, 12), 12).
Show length of sampleAW16(seed, 2, 1).
Show length of sampleMatrixW16(seed).
Show length of mlkemNoiseBatch(seed, 0, 3).
Show length of addModQW16(aw, bw).
Show length of subModQW16(aw, bw).
Show length of zerosW16(5).
Let kp be mlkemKeypair(seed, bytesOf(32, 9)).
Let sealed be mlkemEncapsKem(mlkemSubseq(kp, 0, 1184), bytesOf(32, 17)).
Show length of mlkemDecapsKem(mlkemSubseq(kp, 1184, 2400), mlkemSubseq(sealed, 0, 1088)).
Let dkp be mldsaKeypair(seed).
Let sig be mldsaSign(mlkemSubseq(dkp, 1952, 4032), bytesOf(40, 31), bytesOf(3, 1)).
Show mldsaVerify(mlkemSubseq(dkp, 0, 1952), bytesOf(40, 31), bytesOf(3, 1), sig).
";

#[test]
fn every_kernel_wrapper_stops_a_compiled_program_with_the_tree_walkers_report() {
    // The Rust backend calls each kernel's `logicaffeine_system` function and turns its `Err` into the
    // `name(): reason` failure `NativeKernel::failure` gives the interpreters.
    use logicaffeine_compile::semantics::native_kernels::NativeKernel;
    let rust = logicaffeine_compile::compile::compile_to_rust(&program(EVERY_KERNEL_CALL))
        .expect("the program lowers to Rust");
    let mut wrong = Vec::new();
    for k in NativeKernel::ALL {
        let (module, function) = k.rust_path();
        let call = format!("logicaffeine_system::{module}::{function}(");
        let refusal = format!(
            "::std::result::Result::Err(why) => panic!(\"{{}}(): {{}}\", \"{}\", why),",
            k.name()
        );
        let Some(at) = rust.find(&call) else {
            wrong.push(format!(
                "{}: the program never lowered a call to {call}",
                k.name()
            ));
            continue;
        };
        let wrapper = &rust[at..rust[at..].find("\n}").map_or(rust.len(), |end| at + end)];
        if !wrapper.contains(&refusal) {
            wrong.push(format!(
                "{}: its wrapper does not fail with the kernel's report:\n{wrapper}",
                k.name()
            ));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n\n"));
    assert_eq!(
        NativeKernel::from_name("zerosW16")
            .unwrap()
            .failure("the count must not be negative; it is -1"),
        "zerosW16(): the count must not be negative; it is -1",
        "the interpreters' report has the shape the compiled wrapper prints"
    );
}
