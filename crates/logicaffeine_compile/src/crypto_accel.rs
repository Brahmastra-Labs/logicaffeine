//! Native SHA-256 / HMAC-SHA256 / PBKDF2-HMAC-SHA256, byte-identical to the
//! pure-LOGOS reference in `assets/std/crypto.lg`.
//!
//! The pure-LOGOS crypto is the SPEC — it is what the AOT tiers (Rust, C, WASM)
//! compile to native code, and it is FIPS/RFC vector-locked. But the tree-walking
//! interpreter INTERPRETS every `Word32` operation, so PBKDF2's mandated 4096
//! iterations (16k SHA-256 compressions) take ~32s — far too slow for a real
//! SCRAM handshake. This module accelerates exactly that hot primitive on the
//! interpreted tier: the interpreter intercepts `pbkdf2Sha256` and runs this
//! instead, differentially locked to produce the identical bytes as the pure
//! reference (see `crypto_accel_lock.rs`). No corner is cut — the pure code is
//! still the single source the compiled tiers use; this is the same "the
//! reference implementation is native C" arrangement every language's `hashlib`
//! uses.

const H0: [u32; 8] = [
    0x6a09_e667,
    0xbb67_ae85,
    0x3c6e_f372,
    0xa54f_f53a,
    0x510e_527f,
    0x9b05_688c,
    0x1f83_d9ab,
    0x5be0_cd19,
];

const K: [u32; 64] = [
    0x428a_2f98,
    0x7137_4491,
    0xb5c0_fbcf,
    0xe9b5_dba5,
    0x3956_c25b,
    0x59f1_11f1,
    0x923f_82a4,
    0xab1c_5ed5,
    0xd807_aa98,
    0x1283_5b01,
    0x2431_85be,
    0x550c_7dc3,
    0x72be_5d74,
    0x80de_b1fe,
    0x9bdc_06a7,
    0xc19b_f174,
    0xe49b_69c1,
    0xefbe_4786,
    0x0fc1_9dc6,
    0x240c_a1cc,
    0x2de9_2c6f,
    0x4a74_84aa,
    0x5cb0_a9dc,
    0x76f9_88da,
    0x983e_5152,
    0xa831_c66d,
    0xb003_27c8,
    0xbf59_7fc7,
    0xc6e0_0bf3,
    0xd5a7_9147,
    0x06ca_6351,
    0x1429_2967,
    0x27b7_0a85,
    0x2e1b_2138,
    0x4d2c_6dfc,
    0x5338_0d13,
    0x650a_7354,
    0x766a_0abb,
    0x81c2_c92e,
    0x9272_2c85,
    0xa2bf_e8a1,
    0xa81a_664b,
    0xc24b_8b70,
    0xc76c_51a3,
    0xd192_e819,
    0xd699_0624,
    0xf40e_3585,
    0x106a_a070,
    0x19a4_c116,
    0x1e37_6c08,
    0x2748_774c,
    0x34b0_bcb5,
    0x391c_0cb3,
    0x4ed8_aa4a,
    0x5b9c_ca4f,
    0x682e_6ff3,
    0x748f_82ee,
    0x78a5_636f,
    0x84c8_7814,
    0x8cc7_0208,
    0x90be_fffa,
    0xa450_6ceb,
    0xbef9_a3f7,
    0xc671_78f2,
];

fn compress(h: &mut [u32; 8], block: &[u8; 64]) {
    let mut w = [0u32; 64];
    for i in 0..16 {
        w[i] = u32::from_be_bytes([
            block[i * 4],
            block[i * 4 + 1],
            block[i * 4 + 2],
            block[i * 4 + 3],
        ]);
    }
    for i in 16..64 {
        let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
        let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
        w[i] = w[i - 16]
            .wrapping_add(s0)
            .wrapping_add(w[i - 7])
            .wrapping_add(s1);
    }
    let (mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh) =
        (h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7]);
    for i in 0..64 {
        let big_s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
        let ch = (e & f) ^ ((!e) & g);
        let t1 = hh
            .wrapping_add(big_s1)
            .wrapping_add(ch)
            .wrapping_add(K[i])
            .wrapping_add(w[i]);
        let big_s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
        let maj = (a & b) ^ (a & c) ^ (b & c);
        let t2 = big_s0.wrapping_add(maj);
        hh = g;
        g = f;
        f = e;
        e = d.wrapping_add(t1);
        d = c;
        c = b;
        b = a;
        a = t1.wrapping_add(t2);
    }
    h[0] = h[0].wrapping_add(a);
    h[1] = h[1].wrapping_add(b);
    h[2] = h[2].wrapping_add(c);
    h[3] = h[3].wrapping_add(d);
    h[4] = h[4].wrapping_add(e);
    h[5] = h[5].wrapping_add(f);
    h[6] = h[6].wrapping_add(g);
    h[7] = h[7].wrapping_add(hh);
}

/// SHA-256 (FIPS 180-4) of an arbitrary byte string.
pub fn sha256(msg: &[u8]) -> [u8; 32] {
    let mut h = H0;
    let mut block = [0u8; 64];
    let mut chunks = msg.chunks_exact(64);
    for chunk in chunks.by_ref() {
        block.copy_from_slice(chunk);
        compress(&mut h, &block);
    }
    // Final block(s): remainder + 0x80 + zero pad + 64-bit big-endian bit length.
    let rem = chunks.remainder();
    let mut tail = [0u8; 128];
    tail[..rem.len()].copy_from_slice(rem);
    tail[rem.len()] = 0x80;
    let bit_len = (msg.len() as u64) * 8;
    let pad_len = if rem.len() < 56 { 64 } else { 128 };
    tail[pad_len - 8..pad_len].copy_from_slice(&bit_len.to_be_bytes());
    block.copy_from_slice(&tail[..64]);
    compress(&mut h, &block);
    if pad_len == 128 {
        block.copy_from_slice(&tail[64..128]);
        compress(&mut h, &block);
    }
    let mut out = [0u8; 32];
    for (i, word) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}

/// HMAC-SHA256 (RFC 2104 / RFC 4231).
pub fn hmac_sha256(key: &[u8], msg: &[u8]) -> [u8; 32] {
    let mut k = [0u8; 64];
    if key.len() > 64 {
        k[..32].copy_from_slice(&sha256(key));
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let mut inner = Vec::with_capacity(64 + msg.len());
    inner.extend(k.iter().map(|b| b ^ 0x36));
    inner.extend_from_slice(msg);
    let inner_hash = sha256(&inner);
    let mut outer = Vec::with_capacity(64 + 32);
    outer.extend(k.iter().map(|b| b ^ 0x5c));
    outer.extend_from_slice(&inner_hash);
    sha256(&outer)
}

/// PBKDF2-HMAC-SHA256 with dkLen fixed at 32 — exactly SCRAM's Hi()
/// (RFC 5802 / RFC 7677): one output block, so U1 = HMAC(password, salt ‖ 0x00000001)
/// and the derived key is the running XOR of the U chain over `iters` iterations.
///
/// HMAC over a FIXED key precomputes the inner/outer block midstates once, so each
/// iteration costs two compressions instead of four — the pure LOGOS reference does
/// the naive four; the result is byte-identical.
pub fn pbkdf2_sha256(password: &[u8], salt: &[u8], iters: u32) -> [u8; 32] {
    // Precompute the key-derived padded block and the two HMAC midstates.
    let mut k = [0u8; 64];
    if password.len() > 64 {
        k[..32].copy_from_slice(&sha256(password));
    } else {
        k[..password.len()].copy_from_slice(password);
    }
    let mut ipad = [0u8; 64];
    let mut opad = [0u8; 64];
    for i in 0..64 {
        ipad[i] = k[i] ^ 0x36;
        opad[i] = k[i] ^ 0x5c;
    }
    let mut inner_state = H0;
    compress(&mut inner_state, &ipad);
    let mut outer_state = H0;
    compress(&mut outer_state, &opad);

    // U1 = HMAC(password, salt ‖ INT(1)). The salt may span multiple blocks;
    // fall back to the straightforward HMAC for that first message, then use the
    // midstate fast path (the per-iteration message is always a fixed 32 bytes).
    let mut first_msg = Vec::with_capacity(salt.len() + 4);
    first_msg.extend_from_slice(salt);
    first_msg.extend_from_slice(&1u32.to_be_bytes());
    let mut u = hmac_with_states(&inner_state, &outer_state, &first_msg);
    let mut t = u;
    for _ in 1..iters {
        u = hmac_with_states(&inner_state, &outer_state, &u);
        for j in 0..32 {
            t[j] ^= u[j];
        }
    }
    t
}

/// HMAC-SHA256 given precomputed inner/outer midstates (after absorbing the
/// 64-byte key⊕ipad and key⊕opad blocks). `msg` is appended to each midstate.
fn hmac_with_states(inner_state: &[u32; 8], outer_state: &[u32; 8], msg: &[u8]) -> [u8; 32] {
    let inner_hash = finalize_from(inner_state, msg, 64);
    finalize_from(outer_state, &inner_hash, 64)
}

/// Continue a SHA-256 that has already absorbed `prefix_len` bytes (whose state
/// is `state`), hashing `tail` and finalizing. Only the tail's remaining blocks
/// are compressed.
fn finalize_from(state: &[u32; 8], tail: &[u8], prefix_len: u64) -> [u8; 32] {
    let mut h = *state;
    let total_bits = (prefix_len + tail.len() as u64) * 8;
    let mut block = [0u8; 64];
    let mut chunks = tail.chunks_exact(64);
    for chunk in chunks.by_ref() {
        block.copy_from_slice(chunk);
        compress(&mut h, &block);
    }
    let rem = chunks.remainder();
    let mut pad = [0u8; 128];
    pad[..rem.len()].copy_from_slice(rem);
    pad[rem.len()] = 0x80;
    let pad_len = if rem.len() < 56 { 64 } else { 128 };
    pad[pad_len - 8..pad_len].copy_from_slice(&total_bits.to_be_bytes());
    block.copy_from_slice(&pad[..64]);
    compress(&mut h, &block);
    if pad_len == 128 {
        block.copy_from_slice(&pad[64..128]);
        compress(&mut h, &block);
    }
    let mut out = [0u8; 32];
    for (i, word) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_compile/crypto_accel/tests.rs"]
mod tests;
