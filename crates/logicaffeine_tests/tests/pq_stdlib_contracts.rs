//! The standard library's cryptography written in LOGOS (`crypto.lg`: the FIPS-202 hashes, ML-KEM-768,
//! ML-DSA-65, the authenticated handshake, and the ChaCha20-Poly1305 AEAD) keeps the contracts its
//! native kernels keep, with the same reports. An argument that is not a byte string of the right shape stops the program
//! with the function's name and the reason; an encapsulation key, ciphertext or signature from the
//! other side of a protocol is answered instead (an empty sequence, or a verification of 0). ML-DSA
//! takes FIPS 204's context string, and the handshake signs its transcripts under the one it is given.

#![cfg(not(target_arch = "wasm32"))]

mod common;
use common::{run_all_tiers, run_logos};

/// Helpers the programs build their arguments from.
const HELPERS: &str = r#"## To bytesOf (n: Int) and (k: Int) -> Seq of Int:
    Let mutable r be a new Seq of Int.
    Repeat for i from 1 to n:
        Push (i * k + 11) % 256 to r.
    Return r.

## To withItem (s: Seq of Int) and (at: Int) and (x: Int) -> Seq of Int:
    Let mutable r be copy of s.
    Set item at of r to x.
    Return r.

"#;

fn program(main: &str) -> String {
    format!("{HELPERS}## Main\n{main}")
}

fn fails_with(main: &str, reason: &str) {
    let src = program(main);
    let r = run_all_tiers(&src);
    assert_eq!(
        r.error.as_deref(),
        Some(reason),
        "every tier stops with the function's reason\n--- program ---\n{src}\n--- output ---\n{}",
        r.output
    );
}

fn answers(main: &str, expected: &str) {
    let src = program(main);
    let r = run_all_tiers(&src);
    assert_eq!(r.error, None, "the program runs\n{src}");
    assert_eq!(r.output.trim(), expected, "\n{src}");
}

// ─── the FIPS-202 hashes ──────────────────────────────────────────────────────────────────────

#[test]
fn the_hashes_take_bytes_and_name_the_first_item_that_is_not_one() {
    fails_with(
        "Show sha3_256([97, 256]).\n",
        "sha3_256(): the message must be bytes (0 to 255); item 2 is 256",
    );
    fails_with(
        "Show sha3_512([-1]).\n",
        "sha3_512(): the message must be bytes (0 to 255); item 1 is -1",
    );
    fails_with(
        "Show shake128([1000], 16).\n",
        "shake128(): the message must be bytes (0 to 255); item 1 is 1000",
    );
    fails_with(
        "Show shake256(withItem(bytesOf(200, 3), 137, 300), 32).\n",
        "shake256(): the message must be bytes (0 to 255); item 137 is 300",
    );
}

#[test]
fn a_shake_output_length_is_not_negative() {
    fails_with(
        "Show shake256([97, 98, 99], -1).\n",
        "shake256(): the output length must not be negative; it is -1",
    );
    fails_with(
        "Show shake128([97, 98, 99], -64).\n",
        "shake128(): the output length must not be negative; it is -64",
    );
    answers("Show length of shake256([97, 98, 99], 0).\n", "0");
}

#[test]
fn a_byte_and_its_value_mod_256_are_different_messages() {
    // `[256]` was hashed as `[0]` and `[-1]` corrupted a whole lane; neither is a byte string.
    answers(
        "Show length of sha3_256([0]).\nShow length of sha3_256(a new Seq of Int).\n",
        "32\n32",
    );
    fails_with(
        "Show sha3_256([256]).\n",
        "sha3_256(): the message must be bytes (0 to 255); item 1 is 256",
    );
}

// ─── ML-KEM-768 in LOGOS ──────────────────────────────────────────────────────────────────────

#[test]
fn mlkem768_keygen_takes_a_32_byte_seed() {
    fails_with(
        "Show mlkem768Keygen(bytesOf(31, 5)).\n",
        "mlkem768Keygen(): the seed d is 31 bytes; it must be 32",
    );
    fails_with(
        "Show mlkem768Keygen(withItem(bytesOf(32, 5), 4, 256)).\n",
        "mlkem768Keygen(): the seed d must be bytes (0 to 255); item 4 is 256",
    );
}

const KEM_KEYS: &str = "Let kp be mlkemKeypair(bytesOf(32, 5), bytesOf(32, 9)).\nLet ek be mlkemSubseq(kp, 0, 1184).\nLet dk be mlkemSubseq(kp, 1184, 2400).\n";

#[test]
fn mlkem768_encaps_takes_32_bytes_of_randomness_and_byte_keys() {
    fails_with(
        &format!("{KEM_KEYS}Show mlkem768Encaps(ek, bytesOf(33, 17)).\n"),
        "mlkem768Encaps(): the randomness m is 33 bytes; it must be 32",
    );
    fails_with(
        &format!("{KEM_KEYS}Show mlkem768Encaps(withItem(ek, 1184, -2), bytesOf(32, 17)).\n"),
        "mlkem768Encaps(): the encapsulation key must be bytes (0 to 255); item 1184 is -2",
    );
}

#[test]
fn mlkem768_encaps_answers_a_key_that_fails_fips_203s_checks_with_nothing() {
    answers(
        &format!(
            "{KEM_KEYS}Show length of mlkem768Encaps(mlkemSubseq(ek, 0, 1183), bytesOf(32, 17)).\nShow length of mlkem768Encaps(ek followed by [0], bytesOf(32, 17)).\nShow length of mlkem768Encaps(withItem(withItem(ek, 1, 255), 2, 15), bytesOf(32, 17)).\nShow length of mlkem768Encaps(withItem(withItem(ek, 1151, 240), 1152, 255), bytesOf(32, 17)).\nShow length of mlkem768Encaps(ek, bytesOf(32, 17)).\n"
        ),
        "0\n0\n0\n0\n1120",
    );
}

#[test]
fn mlkem768_decaps_fails_on_a_decapsulation_key_that_is_not_one() {
    fails_with(
        &format!("{KEM_KEYS}Let sealed be mlkemEncapsKem(ek, bytesOf(32, 17)).\nShow mlkem768Decaps(dk followed by [0], mlkemSubseq(sealed, 0, 1088)).\n"),
        "mlkem768Decaps(): the decapsulation key is 2401 bytes; an ML-KEM-768 decapsulation key is 2400",
    );
    fails_with(
        &format!("{KEM_KEYS}Let sealed be mlkemEncapsKem(ek, bytesOf(32, 17)).\nShow mlkem768Decaps(withItem(dk, 2368, (item 2368 of dk + 1) % 256), mlkemSubseq(sealed, 0, 1088)).\n"),
        "mlkem768Decaps(): the decapsulation key's H(ek) is not the hash of its ek (FIPS 203 hash check)",
    );
    fails_with(
        &format!("{KEM_KEYS}Let sealed be mlkemEncapsKem(ek, bytesOf(32, 17)).\nShow mlkem768Decaps(withItem(dk, 1300, (item 1300 of dk + 1) % 256), mlkemSubseq(sealed, 0, 1088)).\n"),
        "mlkem768Decaps(): the decapsulation key's H(ek) is not the hash of its ek (FIPS 203 hash check)",
    );
}

#[test]
fn mlkem768_decaps_answers_a_ciphertext_of_the_wrong_length_with_nothing() {
    answers(
        &format!("{KEM_KEYS}Let sealed be mlkemEncapsKem(ek, bytesOf(32, 17)).\nShow length of mlkem768Decaps(dk, mlkemSubseq(sealed, 0, 1087)).\nShow length of mlkem768Decaps(dk, sealed).\n"),
        "0\n0",
    );
}

#[test]
fn mlkem768_decaps_agrees_with_the_native_kem_on_honest_and_forged_ciphertexts() {
    // The constant-shape Fujisaki–Okamoto choice keeps K′ for the honest ciphertext and K̄ = J(z ‖ c)
    // for every forged one, byte for byte what the native kernel derives.
    answers(
        &format!(
            "{KEM_KEYS}Let sealed be mlkemEncapsKem(ek, bytesOf(32, 17)).\nLet c be mlkemSubseq(sealed, 0, 1088).\nShow mlkem768Decaps(dk, c) equals mlkemSubseq(sealed, 1088, 32).\nShow mlkem768Decaps(dk, c) equals mlkemDecapsKem(dk, c).\nLet forged be withItem(c, 1, (item 1 of c + 1) % 256).\nShow mlkem768Decaps(dk, forged) equals mlkemDecapsKem(dk, forged).\nShow mlkem768Decaps(dk, forged) equals mlkem768Decaps(dk, c).\nLet late be withItem(c, 1088, (item 1088 of c + 128) % 256).\nShow mlkem768Decaps(dk, late) equals mlkemDecapsKem(dk, late).\n"
        ),
        "true\ntrue\ntrue\nfalse\ntrue",
    );
}

#[test]
fn the_byte_difference_count_takes_no_branch_and_counts_exactly() {
    answers(
        "Show mlkemDiffCount([0, 1, 255, 7], [0, 1, 255, 7]).\nShow mlkemDiffCount([0, 1, 255, 7], [1, 1, 0, 7]).\nShow mlkemDiffCount([255, 0], [0, 255]).\nShow mlkemDiffCount([3], [4]).\n",
        "0\n2\n2\n1",
    );
}

// ─── ML-DSA-65 in LOGOS ───────────────────────────────────────────────────────────────────────

const DSA_KEYS: &str = "Let dkp be mldsaKeypair(bytesOf(32, 5)).\nLet pk be mlkemSubseq(dkp, 0, 1952).\nLet sk be mlkemSubseq(dkp, 1952, 4032).\nLet empty be a new Seq of Int.\n";

#[test]
fn the_logos_key_generators_take_a_32_byte_seed() {
    fails_with(
        "Show mldsaKeygenLogos(bytesOf(31, 5)).\n",
        "mldsaKeygenLogos(): the seed is 31 bytes; it must be 32",
    );
    fails_with(
        "Show mldsaKeypairLogos(bytesOf(33, 5)).\n",
        "mldsaKeypairLogos(): the seed is 33 bytes; it must be 32",
    );
    fails_with(
        "Show mldsaKeypairLogos(withItem(bytesOf(32, 5), 1, 512)).\n",
        "mldsaKeypairLogos(): the seed must be bytes (0 to 255); item 1 is 512",
    );
}

#[test]
fn the_logos_signer_takes_a_secret_key_and_a_context_of_at_most_255_bytes() {
    fails_with(
        &format!("{DSA_KEYS}Show mldsaSignLogos(mlkemSubseq(sk, 0, 4031), bytesOf(40, 31)).\n"),
        "mldsaSignLogos(): the secret key is 4031 bytes; an ML-DSA-65 secret key is 4032",
    );
    fails_with(
        &format!("{DSA_KEYS}Show mldsaSignLogos(withItem(sk, 1536, 9), bytesOf(40, 31)).\n"),
        "mldsaSignLogos(): the secret key holds an s1 or s2 coefficient outside [-4, 4]",
    );
    fails_with(
        &format!("{DSA_KEYS}Show mldsaSignLogos(sk, [1, 2, 256]).\n"),
        "mldsaSignLogos(): the message must be bytes (0 to 255); item 3 is 256",
    );
    fails_with(
        &format!("{DSA_KEYS}Show mldsaSignCtxLogos(sk, bytesOf(40, 31), bytesOf(256, 7)).\n"),
        "mldsaSignCtxLogos(): the context is 256 bytes; FIPS 204 allows at most 255",
    );
    fails_with(
        &format!("{DSA_KEYS}Show mldsaSignCtxLogos(sk, bytesOf(40, 31), [-3]).\n"),
        "mldsaSignCtxLogos(): the context must be bytes (0 to 255); item 1 is -3",
    );
}

#[test]
fn the_logos_verifier_answers_zero_for_every_malformed_input() {
    // A short or long signature or key, bytes that are not bytes, and an over-long context all verify
    // nothing, and none of them reaches the verification's arithmetic.
    answers(
        &format!(
            "{DSA_KEYS}Let msg be bytesOf(40, 31).\nLet sig be bytesOf(3309, 3).\nShow mldsaVerifyLogos(pk, msg, mlkemSubseq(sig, 0, 3308)).\nShow mldsaVerifyLogos(pk, msg, sig followed by [0]).\nShow mldsaVerifyLogos(mlkemSubseq(pk, 0, 1951), msg, sig).\nShow mldsaVerifyLogos(pk followed by [7], msg, sig).\nShow mldsaVerifyLogos(withItem(pk, 1, 256), msg, sig).\nShow mldsaVerifyLogos(pk, [300], sig).\nShow mldsaVerifyLogos(pk, msg, withItem(sig, 3309, -1)).\nShow mldsaVerifyCtxLogos(pk, msg, bytesOf(256, 7), sig).\nShow mldsaVerifyCtxLogos(pk, msg, [999], sig).\n"
        ),
        "0\n0\n0\n0\n0\n0\n0\n0\n0",
    );
}

#[test]
fn a_matrix_row_at_its_bound_returns_to_the_normal_domain_exactly() {
    // A row of six pointwise products reaches 6(q − 1) per coefficient; the inverse NTT takes inputs
    // below q, so the row is reduced first. Every coefficient at +6(q−1) and at −6(q−1) (as its
    // 32-bit two's complement) must come back as the inverse NTT of the row taken mod q.
    answers(
        "Let mutable hi be a new Seq of Word32.\nLet mutable lo be a new Seq of Word32.\nLet mutable hiMod be a new Seq of Word32.\nLet mutable loMod be a new Seq of Word32.\nRepeat for i from 1 to 256:\n    Push word32(50282496) to hi.\n    Push word32(4244684800) to lo.\n    Push word32(50282496 % 8380417) to hiMod.\n    Push word32((0 - 50282496) % 8380417 + 8380417) to loMod.\nShow (mldsaInvNttToMont(mldsaReduceRow(hi))) equals (mldsaInvNttToMont(hiMod)).\nShow (mldsaInvNttToMont(mldsaReduceRow(lo))) equals (mldsaInvNttToMont(loMod)).\n",
        "true\ntrue",
    );
}

// ─── the ChaCha20-Poly1305 AEAD ───────────────────────────────────────────────────────────────

#[test]
fn chacha20_takes_a_32_byte_key_a_12_byte_nonce_and_a_32_bit_block_counter() {
    fails_with(
        "Show chacha20Encrypt(bytesOf(31, 1), bytesOf(12, 2), 1, bytesOf(64, 3)).\n",
        "chacha20Encrypt(): the key is 31 bytes; it must be 32",
    );
    fails_with(
        "Show chacha20Encrypt(bytesOf(33, 1), bytesOf(12, 2), 1, bytesOf(64, 3)).\n",
        "chacha20Encrypt(): the key is 33 bytes; it must be 32",
    );
    fails_with(
        "Show chacha20Encrypt(bytesOf(32, 1), bytesOf(13, 2), 1, bytesOf(64, 3)).\n",
        "chacha20Encrypt(): the nonce is 13 bytes; it must be 12",
    );
    fails_with(
        "Show chacha20Encrypt(bytesOf(32, 1), bytesOf(12, 2), -1, bytesOf(64, 3)).\n",
        "chacha20Encrypt(): the block counter must be from 0 to 4294967295; it is -1",
    );
    fails_with(
        "Show chacha20Encrypt(bytesOf(32, 1), bytesOf(12, 2), 4294967295, bytesOf(65, 3)).\n",
        "chacha20Encrypt(): the data runs the block counter from 4294967295 to 4294967296, past its 32 bits",
    );
    fails_with(
        "Show chacha20Encrypt(bytesOf(32, 1), bytesOf(12, 2), 1, [1, 256]).\n",
        "chacha20Encrypt(): the data must be bytes (0 to 255); item 2 is 256",
    );
    answers(
        "Show length of chacha20Encrypt(bytesOf(32, 1), bytesOf(12, 2), 4294967295, bytesOf(64, 3)).\n",
        "64",
    );
}

#[test]
fn poly1305_takes_a_32_byte_one_time_key_and_bytes() {
    fails_with(
        "Show poly1305Mac(bytesOf(31, 1), bytesOf(40, 2)).\n",
        "poly1305Mac(): the one-time key is 31 bytes; it must be 32",
    );
    fails_with(
        "Show poly1305Mac(bytesOf(32, 1), withItem(bytesOf(40, 2), 40, 300)).\n",
        "poly1305Mac(): the message must be bytes (0 to 255); item 40 is 300",
    );
}

#[test]
fn the_aead_takes_its_own_key_nonce_and_data_at_their_word() {
    fails_with(
        "Show aeadSeal(bytesOf(16, 1), bytesOf(12, 2), bytesOf(5, 3), bytesOf(40, 4)).\n",
        "aeadSeal(): the key is 16 bytes; it must be 32",
    );
    fails_with(
        "Show aeadSeal(bytesOf(32, 1), bytesOf(8, 2), bytesOf(5, 3), bytesOf(40, 4)).\n",
        "aeadSeal(): the nonce is 8 bytes; it must be 12",
    );
    fails_with(
        "Show aeadSeal(bytesOf(32, 1), bytesOf(12, 2), [256], bytesOf(40, 4)).\n",
        "aeadSeal(): the associated data must be bytes (0 to 255); item 1 is 256",
    );
    fails_with(
        "Show aeadSeal(bytesOf(32, 1), bytesOf(12, 2), bytesOf(5, 3), [-1]).\n",
        "aeadSeal(): the plaintext must be bytes (0 to 255); item 1 is -1",
    );
    fails_with(
        "Show aeadOpen(bytesOf(32, 1), bytesOf(11, 2), bytesOf(5, 3), bytesOf(56, 4)).\n",
        "aeadOpen(): the nonce is 11 bytes; it must be 12",
    );
}

#[test]
fn the_aead_opens_what_it_sealed_and_answers_a_forged_frame_with_nothing() {
    // The sealed frame is the peer's: one that does not authenticate, or holds an item that is not a
    // byte, opens to nothing; a byte that was the frame's value mod 256 does not authenticate.
    answers(
        "Let key be bytesOf(32, 1).\nLet nonce be bytesOf(12, 2).\nLet aad be bytesOf(5, 3).\nLet pt be bytesOf(40, 4).\nLet sealed be aeadSeal(key, nonce, aad, pt).\nShow aeadOpen(key, nonce, aad, sealed) equals pt.\nShow length of aeadOpen(key, nonce, aad, withItem(sealed, 3, (item 3 of sealed + 1) % 256)).\nShow length of aeadOpen(key, nonce, aad, withItem(sealed, 3, (item 3 of sealed) + 256)).\nShow length of aeadOpen(key, nonce, aad, withItem(sealed, 56, (item 56 of sealed + 1) % 256)).\nShow length of aeadOpen(key, nonce, bytesOf(5, 9), sealed).\n",
        "true\n0\n0\n0\n0",
    );
}

// ─── compiled: the context reaches the signature ──────────────────────────────────────────────

#[test]
#[ignore = "compiles a cargo project via rustc (slow) — LOGOS ML-DSA under a context, against the native kernel"]
fn the_logos_signer_signs_under_a_context_exactly_as_the_native_kernel_does() {
    let main = format!(
        "{DSA_KEYS}Let msg be bytesOf(40, 31).\nRepeat for n in [0, 3, 255]:\n    Let ctx be bytesOf(n, 7).\n    Let ours be mldsaSignCtxLogos(sk, msg, ctx).\n    Show ours equals mldsaSign(sk, msg, ctx).\n    Show mldsaVerifyCtxLogos(pk, msg, ctx, ours).\n    Show mldsaVerifyCtxLogos(pk, msg, bytesOf(n + 1, 7), ours).\nShow mldsaSignLogos(sk, msg) equals mldsaSign(sk, msg, empty).\n"
    );
    let aot = run_logos(&program(&main));
    assert!(aot.success, "the compiled program runs:\n{}", aot.stderr);
    assert_eq!(
        aot.stdout.trim(),
        "true\n1\n0\ntrue\n1\n0\ntrue\n1\n0\ntrue",
        "byte-for-byte the native signature under each context; verified under it and under no other"
    );
}

#[test]
#[ignore = "compiles a cargo project via rustc (slow) — the handshake signs under its context"]
fn the_handshake_signs_its_transcripts_under_its_context() {
    let seed = |name: &str, v: u8| format!("Let {name} be bytesOf(32, {v}).\n");
    let mut main = String::new();
    for (name, v) in [
        ("seedI", 1),
        ("seedR", 2),
        ("kemD", 16),
        ("kemZ", 32),
        ("kemM", 48),
    ] {
        main.push_str(&seed(name, v));
    }
    main.push_str("Let idi be mldsaKeypair(seedI).\nLet idiPk be mlkemSubseq(idi, 0, 1952).\nLet idiSk be mlkemSubseq(idi, 1952, 4032).\nLet idr be mldsaKeypair(seedR).\nLet idrPk be mlkemSubseq(idr, 0, 1952).\nLet idrSk be mlkemSubseq(idr, 1952, 4032).\n");
    main.push_str("Show length of pqAuthHandshake(idiPk, idiSk, idrPk, idrSk, kemD, kemZ, kemM, bytesOf(255, 9)).\n");
    main.push_str("Show length of pqAuthHandshake(idiPk, idiSk, idrPk, idrSk, kemD, kemZ, kemM, bytesOf(256, 9)).\n");
    let aot = run_logos(&program(&main));
    assert!(!aot.success, "a 256-byte context stops the handshake");
    assert_eq!(
        aot.stdout.trim(),
        "64",
        "a 255-byte context agrees on the 64-byte secret"
    );
    assert!(
        aot.stderr
            .contains("mldsaSignCtxLogos(): the context is 256 bytes; FIPS 204 allows at most 255"),
        "the handshake's context reaches its signatures:\n{}",
        aot.stderr
    );
}

#[test]
#[ignore = "compiles a cargo project via rustc (slow) — a trailing byte makes a LOGOS signature verify nothing"]
fn the_logos_verifier_refuses_a_signature_with_trailing_bytes() {
    let main = format!(
        "{DSA_KEYS}Let msg be bytesOf(40, 31).\nLet sig be mldsaSign(sk, msg, empty).\nShow mldsaVerifyLogos(pk, msg, sig).\nShow mldsaVerifyLogos(pk, msg, sig followed by [0]).\nShow mldsaVerifyLogos(pk, msg, sig followed by sig).\n"
    );
    let aot = run_logos(&program(&main));
    assert!(aot.success, "the compiled program runs:\n{}", aot.stderr);
    assert_eq!(aot.stdout.trim(), "1\n0\n0");
}
