//! ML-KEM-768 (FIPS-203) keygen / encapsulation / decapsulation as a Rust composition of the
//! verified native kernels in [`crate::ntt`] (NTT, base-multiply, CBD, compress, byte-encode,
//! 4-way AVX2 matrix expansion) and [`crate::keccak`] (SHA3-256/512, SHAKE256). This is the exact
//! same primitive set the `assets/std/crypto.lg` Logos ML-KEM orchestrates and that the
//! `logicaffeine-tests` AOT gates prove bit-exact vs the FIPS-203 reference — here orchestrated in
//! Rust so the post-quantum channel (`logicaffeine_compile::concurrency::channel`) can run the
//! handshake. Coefficients ride the fast `Word16` carrier throughout.

use crate::keccak::{sha3_256_bytes, sha3_512_bytes, shake256_bytes};
use crate::ntt::{
    mlkem_base_mul_w16, mlkem_byte_decode_w16, mlkem_byte_encode_w16, mlkem_cbd2_w16,
    mlkem_compress_w16, mlkem_decompress_w16, mlkem_inv_ntt_w16, mlkem_ntt_w16,
    mlkem_sample_matrix_w16, mlkem_to_mont_w16,
};
use logicaffeine_base::Word16;

const K: usize = 3; // ML-KEM-768 module rank
const N: usize = 256; // polynomial degree
const Q: u32 = 3329;
const DU: usize = 10; // ciphertext u compression
const DV: usize = 4; //  ciphertext v compression
const POLY_BYTES: usize = 384; // ByteEncode_12 of one 256-coefficient polynomial

/// `ek` = K·384 + 32 (encapsulation / public key).
pub const EK_BYTES: usize = POLY_BYTES * K + 32;
/// `dk` = dk_pke ‖ ek ‖ H(ek) ‖ z (decapsulation / secret key).
pub const DK_BYTES: usize = POLY_BYTES * K + EK_BYTES + 32 + 32;
/// `ct` = K·(32·du) + 32·dv (ciphertext).
pub const CT_BYTES: usize = K * 32 * DU + 32 * DV;
/// Shared secret length.
pub const SS_BYTES: usize = 32;

/// G = SHA3-512, split into (ρ, σ) or (K, r) 32-byte halves.
fn g(input: &[u8]) -> ([u8; 32], [u8; 32]) {
    let h = sha3_512_bytes(input);
    let mut a = [0u8; 32];
    let mut b = [0u8; 32];
    a.copy_from_slice(&h[..32]);
    b.copy_from_slice(&h[32..]);
    (a, b)
}

/// One CBD_2 noise polynomial, raw (no NTT): PRF_η2(seed, nonce) = CBD_2(SHAKE256(seed ‖ nonce, 128)).
fn noise_raw(seed: &[u8; 32], nonce: u8) -> Vec<Word16> {
    let mut pin = seed.to_vec();
    pin.push(nonce);
    mlkem_cbd2_w16(&shake256_bytes(&pin, 128))
}

/// Sample `count` CBD_2 noise polynomials with nonces `base..base+count`, batching the SHAKE256 PRF
/// four independent streams per 4-way AVX2 Keccak permutation (portable scalar fallback). Returns raw
/// polys — the caller NTTs the subset that needs it (s/e/r get NTT; e1/e2 do not). This is the win
/// that closes the keygen/decaps gap vs the hand-tuned reference libs: K-PKE keygen samples 6 noise
/// polys and encrypt (the FO re-encryption inside decaps) samples 7 — otherwise 6–7 serial Keccak
/// permutations; batched, that is ⌈count/4⌉ four-way permutations plus a short scalar tail.
fn noise_batch_raw(seed: &[u8; 32], base: u8, count: usize) -> Vec<Vec<Word16>> {
    let mut raw: Vec<Vec<Word16>> = Vec::with_capacity(count);
    #[cfg(target_arch = "x86_64")]
    {
        if std::is_x86_feature_detected!("avx2") {
            let mut i = 0;
            while i + 4 <= count {
                let ins: [[u8; 33]; 4] = std::array::from_fn(|l| {
                    let mut a = [0u8; 33];
                    a[..32].copy_from_slice(seed);
                    a[32] = base + (i + l) as u8;
                    a
                });
                let refs: [&[u8]; 4] = [&ins[0], &ins[1], &ins[2], &ins[3]];
                let outs = unsafe { crate::keccak::shake256_x4_128(&refs) };
                for o in &outs {
                    raw.push(mlkem_cbd2_w16(o));
                }
                i += 4;
            }
            for j in i..count {
                raw.push(noise_raw(seed, base + j as u8));
            }
            return raw;
        }
    }
    for j in 0..count {
        raw.push(noise_raw(seed, base + j as u8));
    }
    raw
}

/// Coefficient-wise `(a + b) mod q`. Operands are reduced (∈ [0, q)), so `a + b ∈ [0, 2q)` and the
/// reduction is a single conditional subtract — no integer division. Written to auto-vectorize 16-wide
/// (`vpaddw` + `vpcmpgtw`/`vpsubw` blend) rather than 256 scalar `idiv`s (the keygen/decaps hot path).
#[inline]
fn addq(a: &[Word16], b: &[Word16]) -> Vec<Word16> {
    let q = Q as u16;
    (0..N)
        .map(|c| {
            let s = a[c].0 + b[c].0; // ≤ 2·3328 = 6656, no u16 overflow
            Word16(if s >= q { s - q } else { s })
        })
        .collect()
}

/// K-PKE.KeyGen(d) → (ek_pke = ByteEncode₁₂(t̂) ‖ ρ, dk_pke = ByteEncode₁₂(ŝ)).
fn kpke_keygen(d: &[u8; 32]) -> (Vec<u8>, Vec<u8>) {
    let mut gin = d.to_vec();
    gin.push(K as u8);
    let (rho, sigma) = g(&gin);
    let matrix = mlkem_sample_matrix_w16(&rho); // Â[r][c] at slot (r·K+c)·N
    // s (nonces 0..K) and e (nonces K..2K) — all NTT'd — in one 4-way-batched CBD noise pass.
    let se = noise_batch_raw(&sigma, 0, 2 * K);
    let s_hat: Vec<Vec<Word16>> = se[0..K].iter().map(|p| mlkem_ntt_w16(p)).collect();
    let e_hat: Vec<Vec<Word16>> = se[K..2 * K].iter().map(|p| mlkem_ntt_w16(p)).collect();

    let mut ek = Vec::with_capacity(EK_BYTES);
    for i in 0..K {
        let mut acc = vec![Word16(0); N];
        for j in 0..K {
            let e = (j * K + i) * N; // keygen uses Âᵀ: a_hat[i][j] = Â[j][i]
            acc = addq(&acc, &mlkem_base_mul_w16(&matrix[e..e + N], &s_hat[j]));
        }
        let ti = addq(&mlkem_to_mont_w16(&acc), &e_hat[i]);
        ek.extend(mlkem_byte_encode_w16(&ti, 12));
    }
    ek.extend_from_slice(&rho);

    let mut dk_pke = Vec::with_capacity(POLY_BYTES * K);
    for poly in &s_hat {
        dk_pke.extend(mlkem_byte_encode_w16(poly, 12));
    }
    (ek, dk_pke)
}

/// K-PKE.Encrypt(ek, m, r) → c = ByteEncode_du(Compress_du(u)) ‖ ByteEncode_dv(Compress_dv(v)).
fn kpke_encrypt(ek: &[u8], m: &[u8; 32], r: &[u8; 32]) -> Vec<u8> {
    let t_hat: Vec<Vec<Word16>> =
        (0..K).map(|i| mlkem_byte_decode_w16(&ek[POLY_BYTES * i..POLY_BYTES * (i + 1)], 12)).collect();
    let rho = &ek[POLY_BYTES * K..POLY_BYTES * K + 32];
    let matrix = mlkem_sample_matrix_w16(rho); // Â[i][j] at slot (i·K+j)·N

    // r (nonces 0..K, NTT'd) + e1 (nonces K..2K, raw) + e2 (nonce 2K, raw) in one 4-way-batched pass.
    let all = noise_batch_raw(r, 0, 2 * K + 1);
    let r_hat: Vec<Vec<Word16>> = all[0..K].iter().map(|p| mlkem_ntt_w16(p)).collect();
    let e1: Vec<Vec<Word16>> = all[K..2 * K].to_vec();
    let e2 = all[2 * K].clone();

    let mut c = Vec::with_capacity(CT_BYTES);
    for i in 0..K {
        let mut acc = vec![Word16(0); N];
        for j in 0..K {
            let e = (i * K + j) * N;
            acc = addq(&acc, &mlkem_base_mul_w16(&matrix[e..e + N], &r_hat[j]));
        }
        let ui = addq(&mlkem_inv_ntt_w16(&acc), &e1[i]);
        c.extend(mlkem_byte_encode_w16(&mlkem_compress_w16(&ui, DU), DU));
    }
    let mu = mlkem_decompress_w16(&mlkem_byte_decode_w16(m, 1), 1);
    let mut acc = vec![Word16(0); N];
    for i in 0..K {
        acc = addq(&acc, &mlkem_base_mul_w16(&t_hat[i], &r_hat[i]));
    }
    let v = addq(&addq(&mlkem_inv_ntt_w16(&acc), &e2), &mu);
    c.extend(mlkem_byte_encode_w16(&mlkem_compress_w16(&v, DV), DV));
    c
}

/// K-PKE.Decrypt(dk_pke, c) → m = ByteEncode₁(Compress₁(v − NTT⁻¹(ŝᵀ ∘ NTT(u)))).
fn kpke_decrypt(dk_pke: &[u8], c: &[u8]) -> [u8; 32] {
    let s_hat: Vec<Vec<Word16>> = (0..K)
        .map(|i| mlkem_byte_decode_w16(&dk_pke[POLY_BYTES * i..POLY_BYTES * (i + 1)], 12))
        .collect();
    let u: Vec<Vec<Word16>> = (0..K)
        .map(|i| {
            let bytes = &c[32 * DU * i..32 * DU * (i + 1)];
            mlkem_decompress_w16(&mlkem_byte_decode_w16(bytes, DU), DU)
        })
        .collect();
    let v = mlkem_decompress_w16(&mlkem_byte_decode_w16(&c[32 * DU * K..], DV), DV);

    let mut acc = vec![Word16(0); N];
    for i in 0..K {
        acc = addq(&acc, &mlkem_base_mul_w16(&s_hat[i], &mlkem_ntt_w16(&u[i])));
    }
    let inv = mlkem_inv_ntt_w16(&acc);
    // v, inv ∈ [0, q) ⇒ v + q − inv ∈ (0, 2q): one conditional subtract, not a division.
    let q = Q as u16;
    let w: Vec<Word16> = (0..N)
        .map(|c| {
            let s = v[c].0 + q - inv[c].0;
            Word16(if s >= q { s - q } else { s })
        })
        .collect();
    let mut m = [0u8; 32];
    m.copy_from_slice(&mlkem_byte_encode_w16(&mlkem_compress_w16(&w, 1), 1));
    m
}

/// ML-KEM.KeyGen(d, z) → (ek, dk) where dk = dk_pke ‖ ek ‖ H(ek) ‖ z.
pub fn keygen(d: &[u8; 32], z: &[u8; 32]) -> (Vec<u8>, Vec<u8>) {
    let (ek, dk_pke) = kpke_keygen(d);
    let mut dk = dk_pke;
    dk.extend_from_slice(&ek);
    dk.extend_from_slice(&sha3_256_bytes(&ek));
    dk.extend_from_slice(z);
    (ek, dk)
}

/// ML-KEM.Encaps(ek, m) → (ciphertext, shared secret). `m` is the 32-byte message randomness.
pub fn encaps(ek: &[u8], m: &[u8; 32]) -> (Vec<u8>, [u8; 32]) {
    let h = sha3_256_bytes(ek);
    let (kk, r) = g(&[m.as_slice(), &h].concat());
    (kpke_encrypt(ek, m, &r), kk)
}

/// ML-KEM.Decaps(dk, c) → shared secret, with the Fujisaki-Okamoto implicit reject.
pub fn decaps(dk: &[u8], c: &[u8]) -> [u8; 32] {
    let dk_pke = &dk[0..POLY_BYTES * K];
    let ek = &dk[POLY_BYTES * K..POLY_BYTES * K + EK_BYTES];
    let h = &dk[POLY_BYTES * K + EK_BYTES..POLY_BYTES * K + EK_BYTES + 32];
    let z = &dk[POLY_BYTES * K + EK_BYTES + 32..POLY_BYTES * K + EK_BYTES + 64];

    let m_prime = kpke_decrypt(dk_pke, c);
    let (k_prime, r_prime) = g(&[m_prime.as_slice(), h].concat());
    let mut k_bar = [0u8; 32];
    k_bar.copy_from_slice(&shake256_bytes(&[z, c].concat(), 32));

    if c == kpke_encrypt(ek, &m_prime, &r_prime).as_slice() {
        k_prime
    } else {
        k_bar
    }
}

// ── Logos-facing wrappers (Seq of Int bytes 0..255) — the natives crypto.lg's handshake calls ────

fn bytes(s: &[i64]) -> Vec<u8> {
    s.iter().map(|&x| x.rem_euclid(256) as u8).collect()
}
fn seq(v: &[u8]) -> logicaffeine_data::LogosSeq<i64> {
    logicaffeine_data::LogosSeq::from_vec(v.iter().map(|&b| b as i64).collect())
}
fn seed32(s: &[i64]) -> [u8; 32] {
    let mut a = [0u8; 32];
    for (b, &x) in a.iter_mut().zip(s) {
        *b = x.rem_euclid(256) as u8;
    }
    a
}

/// `mlkemNoiseBatch(seed, base, count)` → `count·256` raw CBD_2 Word16 coefficients (nonces
/// `base..base+count`, no NTT), the flat concatenation of `count` noise polynomials. The fast
/// primitive the Logos `mlkem768Keygen`/`mlkemEncrypt` call once instead of `count` scalar
/// `mlkemPrfNoise`s — one 4-way SHAKE256 permutation covers four PRF streams (see [`noise_batch_raw`]).
pub fn mlkem_noise_batch_from_int(seed: &[i64], base: i64, count: i64) -> logicaffeine_data::LogosSeq<Word16> {
    let s = seed32(seed);
    let n = count.max(0) as usize;
    let polys = noise_batch_raw(&s, base.max(0) as u8, n);
    let mut out: Vec<Word16> = Vec::with_capacity(n * N);
    for p in &polys {
        out.extend_from_slice(p);
    }
    logicaffeine_data::LogosSeq::from_vec(out)
}

/// `mlkemKeypair(d, z)` → ek(1184) ‖ dk(2400).
pub fn mlkem_keypair_seq(d: &[i64], z: &[i64]) -> logicaffeine_data::LogosSeq<i64> {
    let (ek, dk) = keygen(&seed32(d), &seed32(z));
    let mut out = ek;
    out.extend(dk);
    seq(&out)
}
/// `mlkemEncapsKem(ek, m)` → ciphertext(1088) ‖ shared_secret(32).
pub fn mlkem_encaps_seq(ek: &[i64], m: &[i64]) -> logicaffeine_data::LogosSeq<i64> {
    let (ct, ss) = encaps(&bytes(ek), &seed32(m));
    let mut out = ct;
    out.extend_from_slice(&ss);
    seq(&out)
}
/// `mlkemDecapsKem(dk, ct)` → shared_secret(32).
pub fn mlkem_decaps_seq(dk: &[i64], ct: &[i64]) -> logicaffeine_data::LogosSeq<i64> {
    seq(&decaps(&bytes(dk), &bytes(ct)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keygen_encaps_decaps_round_trips() {
        // Deterministic seeds — full handshake: keygen, encaps to ek, decaps recovers the secret.
        let d = [0x11u8; 32];
        let z = [0x22u8; 32];
        let m = [0x33u8; 32];
        let (ek, dk) = keygen(&d, &z);
        assert_eq!(ek.len(), EK_BYTES, "ek = 1184 bytes");
        assert_eq!(dk.len(), DK_BYTES, "dk = 2400 bytes");

        let (ct, ss_a) = encaps(&ek, &m);
        assert_eq!(ct.len(), CT_BYTES, "ct = 1088 bytes");
        let ss_b = decaps(&dk, &ct);
        assert_eq!(ss_a, ss_b, "decaps must recover the encaps shared secret");

        // A tampered ciphertext implicit-rejects to a DIFFERENT (z-derived) secret, never the real one.
        let mut bad = ct.clone();
        bad[0] ^= 1;
        assert_ne!(decaps(&dk, &bad), ss_a, "tampered ct ⇒ implicit reject");
    }

    #[test]
    fn distinct_keypairs_give_distinct_secrets() {
        let m = [0x55u8; 32];
        let (ek1, _) = keygen(&[1u8; 32], &[2u8; 32]);
        let (ek2, _) = keygen(&[3u8; 32], &[4u8; 32]);
        assert_ne!(ek1, ek2, "distinct seeds ⇒ distinct public keys");
        assert_ne!(encaps(&ek1, &m).1, encaps(&ek2, &m).1, "distinct keys ⇒ distinct shared secrets");
    }

    /// Per-phase profiler for keygen + decaps — locates the cost vs libcrux (run with
    /// `-C target-cpu=native --ignored --nocapture`). No kernel/oracle deps → builds fast.
    #[test]
    #[ignore = "profiler — cargo test -p logicaffeine-system profile_mlkem -- --ignored --nocapture"]
    fn profile_mlkem_keygen_decaps_phases() {
        use std::hint::black_box;
        use std::time::Instant;
        fn t<R, F: FnMut() -> R>(iters: usize, mut f: F) -> f64 {
            for _ in 0..iters / 5 + 1 {
                std::hint::black_box(f());
            }
            let s = Instant::now();
            for _ in 0..iters {
                std::hint::black_box(f());
            }
            s.elapsed().as_nanos() as f64 / iters as f64
        }
        const IT: usize = 4000;
        let d = [0x11u8; 32];
        let z = [0x22u8; 32];
        let m = [0x33u8; 32];
        let (ek, dk) = keygen(&d, &z);
        let (ct, _) = encaps(&ek, &m);

        let mut gin = d.to_vec();
        gin.push(K as u8);
        let (rho, sigma) = g(&gin);

        // keygen phases
        let g_ns = t(IT, || black_box(g(black_box(&gin))));
        let expand_ns = t(IT, || black_box(mlkem_sample_matrix_w16(black_box(&rho))));
        let noise_ns = t(IT, || black_box(noise_batch_raw(black_box(&sigma), 0, 2 * K)));
        let noise_ntt_ns = t(IT, || {
            let se = noise_batch_raw(&sigma, 0, 2 * K);
            black_box(se.iter().map(|p| mlkem_ntt_w16(p)).collect::<Vec<_>>())
        });
        let one_enc12_ns = t(IT, || {
            black_box(mlkem_byte_encode_w16(black_box(&vec![Word16(1234); N]), 12))
        });
        let keygen_ns = t(IT, || black_box(keygen(black_box(&d), black_box(&z))));

        // decaps phases
        let dk_pke = &dk[0..POLY_BYTES * K];
        let decrypt_ns = t(IT, || black_box(kpke_decrypt(black_box(dk_pke), black_box(&ct))));
        let encrypt_ns = t(IT, || black_box(kpke_encrypt(black_box(&ek), black_box(&m), black_box(&[7u8; 32]))));
        let decaps_ns = t(IT, || black_box(decaps(black_box(&dk), black_box(&ct))));
        let encaps_ns = t(IT, || black_box(encaps(black_box(&ek), black_box(&m))));

        println!("\n=== ML-KEM-768 phase profile (ns/op, IT={IT}, native) ===");
        println!("  G(SHA3-512)         {g_ns:>8.0}");
        println!("  ExpandA (matrix)    {expand_ns:>8.0}   ← 4-way SHAKE128 rejection");
        println!("  noise CBD (raw)     {noise_ns:>8.0}   ← 4-way SHAKE256 (batched)");
        println!("  noise CBD + NTT     {noise_ntt_ns:>8.0}");
        println!("  ByteEncode12 (×1)   {one_enc12_ns:>8.0}   (keygen does ×{K}, dk ×{K})");
        println!("  -------------------------------------------");
        println!("  KEYGEN full         {keygen_ns:>8.0}   (libcrux 16658)");
        println!("  kpke_decrypt        {decrypt_ns:>8.0}");
        println!("  kpke_encrypt        {encrypt_ns:>8.0}   ← re-encrypt (ExpandA+noise+compress)");
        println!("  ENCAPS full         {encaps_ns:>8.0}   (libcrux 28479)");
        println!("  DECAPS full         {decaps_ns:>8.0}   (libcrux 21351) = decrypt + re-encrypt");

        // ── ExpandA internal split: pure Keccak permutation vs the rest (rejection/pack) ──
        let a16: Vec<Word16> = (0..256).map(|i| Word16((i * 7 % Q as usize) as u16)).collect();
        let b16: Vec<Word16> = (0..256).map(|i| Word16((i * 13 % Q as usize) as u16)).collect();
        let basemul_ns = t(IT, || mlkem_base_mul_w16(black_box(&a16), black_box(&b16)));
        #[cfg(target_arch = "x86_64")]
        {
            if std::is_x86_feature_detected!("avx2") {
                use std::arch::x86_64::*;
                let mut kst = [unsafe { _mm256_setzero_si256() }; 25];
                let kperm_ns = t(IT * 4, || {
                    unsafe { crate::keccak::keccak_f1600_x4(&mut kst) };
                    kst[0]
                });
                println!("  -------------------------------------------");
                println!("  keccak_f1600_x4 (1 perm)  {kperm_ns:>8.2}   ← ExpandA ≈ 6 of these (2×4-way) + 3 scalar");
                println!("  keccak est. in ExpandA    {:>8.0}   (6 × 4-way perm)", 6.0 * kperm_ns);
            }
        }
        println!("  base_mul (1 poly, scalar) {basemul_ns:>8.2}   ← ×9 keygen, ×15 decaps");
    }
}
