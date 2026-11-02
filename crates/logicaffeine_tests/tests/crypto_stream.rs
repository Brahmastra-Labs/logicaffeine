//! ChaCha20 stream cipher (RFC 8439) exposed to the Logos language: `encrypt`/`decrypt`,
//! written in `crypto.lg` on top of `chacha20Block` (no Rust duplication of the cipher).
//!
//! This is the E2E symmetric primitive for media — encrypt, then FEC-protect, and you have
//! confidential audio/video that also survives a lossy link. XOR is its own inverse, so
//! `decrypt` is `encrypt`. Correctness is pinned to a native ChaCha20 oracle that is itself
//! grounded on the published RFC 8439 §2.3.2 keystream, plus an encrypt∘decrypt = identity
//! round-trip over a non-block-aligned message. These tests are the spec; they must fail until
//! `encrypt`/`decrypt` exist, and they bite (a wrong keystream, byte order, or block counter
//! diverges from the oracle).

use logicaffeine_compile::compile::tw_outcome;

fn qr(s: &mut [u32; 16], a: usize, b: usize, c: usize, d: usize) {
    s[a] = s[a].wrapping_add(s[b]);
    s[d] ^= s[a];
    s[d] = s[d].rotate_left(16);
    s[c] = s[c].wrapping_add(s[d]);
    s[b] ^= s[c];
    s[b] = s[b].rotate_left(12);
    s[a] = s[a].wrapping_add(s[b]);
    s[d] ^= s[a];
    s[d] = s[d].rotate_left(8);
    s[c] = s[c].wrapping_add(s[d]);
    s[b] ^= s[c];
    s[b] = s[b].rotate_left(7);
}

/// A native RFC 8439 ChaCha20 block — the reference the Logos cipher is checked against.
fn chacha20_block(key: &[u32; 8], counter: u32, nonce: &[u32; 3]) -> [u32; 16] {
    let mut s = [0u32; 16];
    s[0] = 0x6170_7865;
    s[1] = 0x3320_646e;
    s[2] = 0x7962_2d32;
    s[3] = 0x6b20_6574;
    s[4..12].copy_from_slice(key);
    s[12] = counter;
    s[13] = nonce[0];
    s[14] = nonce[1];
    s[15] = nonce[2];
    let mut w = s;
    for _ in 0..10 {
        qr(&mut w, 0, 4, 8, 12);
        qr(&mut w, 1, 5, 9, 13);
        qr(&mut w, 2, 6, 10, 14);
        qr(&mut w, 3, 7, 11, 15);
        qr(&mut w, 0, 5, 10, 15);
        qr(&mut w, 1, 6, 11, 12);
        qr(&mut w, 2, 7, 8, 13);
        qr(&mut w, 3, 4, 9, 14);
    }
    for i in 0..16 {
        w[i] = w[i].wrapping_add(s[i]);
    }
    w
}

fn le_words(bytes: &[u8]) -> Vec<u32> {
    bytes
        .chunks(4)
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

/// Native ChaCha20 keystream-XOR (RFC 8439 §2.4): byte `i` uses block `counter + i/64`, word
/// `(i%64)/4`, little-endian byte `i%4`.
fn chacha20_xor(key: &[u32; 8], nonce: &[u32; 3], counter: u32, data: &[u8]) -> Vec<u8> {
    data.iter()
        .enumerate()
        .map(|(i, &byte)| {
            let ks = chacha20_block(key, counter.wrapping_add((i / 64) as u32), nonce);
            let ksb = (ks[(i % 64) / 4] >> (8 * (i % 4))) as u8;
            byte ^ ksb
        })
        .collect()
}

fn as_key(v: &[u32]) -> [u32; 8] {
    let mut k = [0u32; 8];
    k.copy_from_slice(v);
    k
}
fn as_nonce(v: &[u32]) -> [u32; 3] {
    let mut n = [0u32; 3];
    n.copy_from_slice(v);
    n
}
fn words_lit(w: &[u32]) -> String {
    w.iter()
        .map(|x| format!("word32({x})"))
        .collect::<Vec<_>>()
        .join(", ")
}
fn ints_lit(b: &[u8]) -> String {
    b.iter()
        .map(|x| x.to_string())
        .collect::<Vec<_>>()
        .join(", ")
}
fn parse_bytes(out: &str) -> Vec<u8> {
    out.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.trim().parse::<i64>().expect("byte line") as u8)
        .collect()
}

/// The oracle must reproduce the RFC 8439 §2.3.2 keystream — grounding it to the standard before
/// it is trusted as the reference for the Logos cipher.
#[test]
fn native_oracle_matches_rfc8439_keystream() {
    let key = as_key(&le_words(&(0u8..32).collect::<Vec<_>>()));
    let nonce = as_nonce(&le_words(&[0, 0, 0, 0x09, 0, 0, 0, 0x4a, 0, 0, 0, 0]));
    assert_eq!(
        chacha20_block(&key, 1, &nonce),
        [
            0xe4e7_f110,
            0x1559_3bd1,
            0x1fdd_0f50,
            0xc471_20a3,
            0xc7f4_d1c7,
            0x0368_c033,
            0x9aaa_2204,
            0x4e6c_d4c3,
            0x4664_82d2,
            0x09aa_9f07,
            0x05d7_c214,
            0xa202_8bd9,
            0xd19c_12b5,
            0xb94e_16de,
            0xe883_d0cb,
            0x4e3c_50a2,
        ],
        "native ChaCha20 oracle must match RFC 8439 §2.3.2"
    );
}

/// Logos `encrypt` must equal the native ChaCha20 keystream XOR over a message that spans TWO
/// 64-byte blocks — pinning the keystream, little-endian byte order, and per-block counter.
#[test]
fn logos_encrypt_matches_native_chacha20_over_multiple_blocks() {
    let key_w = le_words(&(0u8..32).collect::<Vec<_>>());
    let nonce_w = le_words(&[0, 0, 0, 0, 0, 0, 0, 0x4a, 0, 0, 0, 0]);
    let plaintext: Vec<u8> = (0u8..70).collect(); // 70 bytes → two blocks
    let expected = chacha20_xor(&as_key(&key_w), &as_nonce(&nonce_w), 1, &plaintext);

    let program = format!(
        "## Main\n\
         Let key be [{}].\n\
         Let nonce be [{}].\n\
         Let message be [{}].\n\
         Let ciphertext be encrypt(key, nonce, message, word32(1)).\n\
         Repeat for i from 1 to {}:\n\
         \x20   Show item i of ciphertext.\n",
        words_lit(&key_w),
        words_lit(&nonce_w),
        ints_lit(&plaintext),
        plaintext.len()
    );
    let r = tw_outcome(&program);
    assert_eq!(r.error, None, "encrypt compiles and runs: {:?}", r.error);
    assert_eq!(
        parse_bytes(&r.output),
        expected,
        "Logos encrypt must equal the native ChaCha20 keystream XOR"
    );
}

/// `decrypt ∘ encrypt = identity` for a message that is NOT block-aligned (13 bytes) — the XOR
/// symmetry plus correct handling of a partial final block.
#[test]
fn logos_encrypt_then_decrypt_is_the_identity() {
    let key_w = le_words(&(1u8..33).collect::<Vec<_>>());
    let nonce_w = le_words(&[7, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 9]);
    let plaintext: Vec<u8> = vec![200, 5, 99, 42, 0, 255, 17, 128, 64, 1, 2, 3, 4];

    let program = format!(
        "## Main\n\
         Let key be [{}].\n\
         Let nonce be [{}].\n\
         Let plaintext be [{}].\n\
         Let ciphertext be encrypt(key, nonce, plaintext, word32(0)).\n\
         Let recovered be decrypt(key, nonce, ciphertext, word32(0)).\n\
         Repeat for i from 1 to {}:\n\
         \x20   Show item i of recovered.\n",
        words_lit(&key_w),
        words_lit(&nonce_w),
        ints_lit(&plaintext),
        plaintext.len()
    );
    let r = tw_outcome(&program);
    assert_eq!(
        r.error, None,
        "encrypt/decrypt compile and run: {:?}",
        r.error
    );
    assert_eq!(
        parse_bytes(&r.output),
        plaintext,
        "decrypt∘encrypt must return the original plaintext"
    );
}
