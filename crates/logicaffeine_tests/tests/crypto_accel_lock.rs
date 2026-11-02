//! Native crypto-acceleration lock.
//!
//! The pure-LOGOS `sha256`/`hmacSha256`/`pbkdf2Sha256` in `assets/std/crypto.lg`
//! are the FIPS/RFC-locked SPEC the AOT tiers compile (see `crypto_sha2_lock.rs`).
//! But `pbkdf2Sha256`'s Hi() runs 4096 iterations — ~16k SHA-256 compressions —
//! which is far too slow to INTERPRET (≈32s on the tree-walker) for a live SCRAM
//! handshake. The tree-walker and the bytecode VM therefore intercept
//! `pbkdf2Sha256` and run the native reference in `logicaffeine_compile::crypto_accel`.
//!
//! That substitution is only sound if the native reference is BYTE-IDENTICAL to
//! the pure body. This file pins that invariant three ways:
//!   1. the native reference matches the official FIPS 180-4 / RFC 4231 / RFC 7677
//!      vectors directly (Rust level);
//!   2. the native PBKDF2 (with its HMAC-midstate fast path) equals a naive
//!      independent HMAC-chain over a broad fuzz of inputs — sizes crossing every
//!      SHA-256 block boundary, embedded NULs, many iteration counts;
//!   3. a fresh, non-official PBKDF2 input produces the SAME bytes on the
//!      interpreter AND VM (native path) AND the C AOT tier (pure compiled body),
//!      the actual cross-tier native≡pure differential.
#![cfg(not(target_arch = "wasm32"))]

mod common;

use common::{assert_all_tiers, assert_c_output};
use logicaffeine_compile::crypto_accel;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// An INDEPENDENT PBKDF2-HMAC-SHA256 (dkLen=32) — the textbook U-chain with a
/// fresh `hmac_sha256` per iteration (no midstate reuse). This is the oracle the
/// native midstate implementation must equal.
fn pbkdf2_naive(password: &[u8], salt: &[u8], iters: u32) -> [u8; 32] {
    let mut block = salt.to_vec();
    block.extend_from_slice(&1u32.to_be_bytes());
    let mut u = crypto_accel::hmac_sha256(password, &block);
    let mut t = u;
    for _ in 1..iters {
        u = crypto_accel::hmac_sha256(password, &u);
        for j in 0..32 {
            t[j] ^= u[j];
        }
    }
    t
}

/// (1) The native reference reproduces the official primitive vectors.
#[test]
fn native_reference_matches_official_vectors() {
    // FIPS 180-4 SHA-256.
    assert_eq!(
        hex(&crypto_accel::sha256(b"abc")),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    // RFC 4231 HMAC-SHA256 case 2.
    assert_eq!(
        hex(&crypto_accel::hmac_sha256(
            b"Jefe",
            b"what do ya want for nothing?"
        )),
        "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
    );
    // RFC 7677 SCRAM-SHA-256 example: Hi("pencil", base64_decode("W22ZaJ0SNY7soEsUEjb6gQ=="), 4096).
    let salt = [
        0x5b, 0x6d, 0x99, 0x68, 0x9d, 0x12, 0x35, 0x8e, 0xec, 0xa0, 0x4b, 0x14, 0x12, 0x36, 0xfa,
        0x81,
    ];
    assert_eq!(
        hex(&crypto_accel::pbkdf2_sha256(b"pencil", &salt, 4096)),
        "c4a49510323ab4f952cac1fa99441939e78ea74d6be81ddf7096e87513dc615d"
    );
}

/// (2) Native PBKDF2 (HMAC-midstate fast path) ≡ the naive independent HMAC-chain,
/// over a broad deterministic fuzz. Password and salt lengths sweep every SHA-256
/// block boundary (0..80 bytes) so the internal padding logic is exercised on both
/// sides of 55/56/63/64; iteration counts include 1 and the SCRAM-mandated 4096.
#[test]
fn native_pbkdf2_equals_naive_over_fuzz() {
    // A cheap deterministic PRNG (no external rand dep; reproducible on every run).
    let mut state: u64 = 0x9e37_79b9_7f4a_7c15;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for _ in 0..256 {
        let pw_len = (next() % 80) as usize;
        let salt_len = (next() % 80) as usize;
        let pw: Vec<u8> = (0..pw_len).map(|_| (next() & 0xff) as u8).collect();
        let salt: Vec<u8> = (0..salt_len).map(|_| (next() & 0xff) as u8).collect();
        let iters = 1 + (next() % 64) as u32;
        assert_eq!(
            crypto_accel::pbkdf2_sha256(&pw, &salt, iters),
            pbkdf2_naive(&pw, &salt, iters),
            "native≠naive for pw_len={pw_len} salt_len={salt_len} iters={iters}"
        );
    }
    // The two block-boundary lengths that matter for the finalize padding, at 4096.
    for len in [55usize, 56, 63, 64, 65] {
        let pw: Vec<u8> = (0..len).map(|i| (i as u8) ^ 0x5a).collect();
        let salt: Vec<u8> = (0..len).map(|i| (i as u8).wrapping_add(3)).collect();
        assert_eq!(
            crypto_accel::pbkdf2_sha256(&pw, &salt, 4096),
            pbkdf2_naive(&pw, &salt, 4096),
            "native≠naive at block boundary len={len}"
        );
    }
}

/// (3) The cross-tier native≡pure differential on a FRESH (non-official) input:
/// the tree-walker and VM take the native path, the C AOT tier compiles the pure
/// LOGOS body. Both must emit the identical derived key. The expected hex is
/// computed here by the independent naive oracle, so this simultaneously pins
/// native(interp), native(VM), pure(C AOT), and the naive reference to one value.
#[test]
fn cross_tier_pbkdf2_native_equals_pure() {
    // password = "correct horse", salt = bytes 0x10..0x20 (16 bytes), iters = 2048.
    let password = "correct horse";
    let salt: Vec<u8> = (0x10u8..0x20).collect();
    let expected = hex(&pbkdf2_naive(password.as_bytes(), &salt, 2048));

    let program = format!(
        "## Main\n\
         Let pw be text_bytes(\"{password}\").\n\
         Let mutable salt be a new Seq of Int.\n\
         Repeat for i from 16 to 31:\n    \
             Push i to salt.\n\
         Show bytesToHex(pbkdf2Sha256(pw, salt, 2048)).\n"
    );

    // interpreter + VM (native intercept).
    assert_all_tiers(&program, &expected);
    // C AOT (pure compiled body).
    assert_c_output(&program, &expected);
}
