//! ML-KEM-768 (FIPS-203) keygen / encapsulation / decapsulation as a Rust composition of the
//! verified native kernels in [`crate::ntt`] (NTT, base-multiply, CBD, compress, byte-encode,
//! 4-way AVX2 matrix expansion) and [`crate::keccak`] (SHA3-256/512, SHAKE256). This is the exact
//! same primitive set the `assets/std/crypto.lg` Logos ML-KEM orchestrates and that the
//! `logicaffeine-tests` AOT gates prove bit-exact vs the FIPS-203 reference — here orchestrated in
//! Rust so the post-quantum channel (`logicaffeine_compile::concurrency::channel`) can run the
//! handshake. Coefficients ride the fast `Word16` carrier throughout.

use crate::keccak::{sha3_256_bytes, sha3_512_bytes, shake256_bytes};
use crate::kernel_args as args;
use crate::ntt::{
    cadd_q, csub_q, mlkem_base_mul_w16, mlkem_byte_decode_w16, mlkem_byte_encode_w16,
    mlkem_cbd2_w16, mlkem_compress_w16, mlkem_decompress_w16, mlkem_inv_ntt_w16, mlkem_ntt_w16,
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
/// reduction is one masked subtract of q — no division and no branch on the secret sum.
#[inline]
fn addq(a: &[Word16], b: &[Word16]) -> Vec<Word16> {
    (0..N).map(|c| Word16(csub_q(a[c].0 + b[c].0))).collect()
}

/// Coefficient-wise `(a − b) mod q` of reduced operands: `a − b ∈ (−q, q)`, then one masked add of q.
#[inline]
fn subq(a: &[Word16], b: &[Word16]) -> Vec<Word16> {
    (0..N)
        .map(|c| Word16(cadd_q(a[c].0 as i16 - b[c].0 as i16)))
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
    let t_hat: Vec<Vec<Word16>> = (0..K)
        .map(|i| mlkem_byte_decode_w16(&ek[POLY_BYTES * i..POLY_BYTES * (i + 1)], 12))
        .collect();
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
    let w = subq(&v, &mlkem_inv_ntt_w16(&acc));
    let mut m = [0u8; 32];
    m.copy_from_slice(&mlkem_byte_encode_w16(&mlkem_compress_w16(&w, 1), 1));
    m
}

/// Why [`encaps`] or [`decaps`] refuses its inputs: the FIPS 203 §7.2–7.3 input checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KemError {
    /// The encapsulation key is not [`EK_BYTES`] bytes (the ek type check).
    EncapsulationKeyLength { len: usize },
    /// A 12-bit coefficient of the encapsulation key is q or more (the ek modulus check).
    EncapsulationKeyNotReduced,
    /// The decapsulation key is not [`DK_BYTES`] bytes (the dk type check).
    DecapsulationKeyLength { len: usize },
    /// The H(ek) inside the decapsulation key is not the hash of the ek beside it (the hash check).
    DecapsulationKeyCorrupt,
    /// The ciphertext is not [`CT_BYTES`] bytes (the ciphertext type check).
    CiphertextLength { len: usize },
}

impl std::fmt::Display for KemError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            KemError::EncapsulationKeyLength { len } => write!(
                f,
                "the encapsulation key is {len} bytes; an ML-KEM-768 encapsulation key is {EK_BYTES}"
            ),
            KemError::EncapsulationKeyNotReduced => write!(
                f,
                "the encapsulation key holds a coefficient of {Q} or more (FIPS 203 modulus check)"
            ),
            KemError::DecapsulationKeyLength { len } => write!(
                f,
                "the decapsulation key is {len} bytes; an ML-KEM-768 decapsulation key is {DK_BYTES}"
            ),
            KemError::DecapsulationKeyCorrupt => write!(
                f,
                "the decapsulation key's H(ek) is not the hash of its ek (FIPS 203 hash check)"
            ),
            KemError::CiphertextLength { len } => write!(
                f,
                "the ciphertext is {len} bytes; an ML-KEM-768 ciphertext is {CT_BYTES}"
            ),
        }
    }
}

impl std::error::Error for KemError {}

/// ML-KEM.KeyGen(d, z) → (ek, dk) where dk = dk_pke ‖ ek ‖ H(ek) ‖ z.
pub fn keygen(d: &[u8; 32], z: &[u8; 32]) -> (Vec<u8>, Vec<u8>) {
    let (ek, dk_pke) = kpke_keygen(d);
    let mut dk = dk_pke;
    dk.extend_from_slice(&ek);
    dk.extend_from_slice(&sha3_256_bytes(&ek));
    dk.extend_from_slice(z);
    (ek, dk)
}

/// ML-KEM.Encaps(ek, m) → (ciphertext, shared secret). `m` is the 32-byte message randomness. An
/// `ek` that fails the FIPS 203 §7.2 input checks is refused.
pub fn encaps(ek: &[u8], m: &[u8; 32]) -> Result<(Vec<u8>, [u8; 32]), KemError> {
    check_encapsulation_key(ek)?;
    let h = sha3_256_bytes(ek);
    let (kk, r) = g(&[m.as_slice(), &h].concat());
    Ok((kpke_encrypt(ek, m, &r), kk))
}

/// ML-KEM.Decaps(dk, c) → shared secret, with the Fujisaki-Okamoto implicit reject. The FIPS 203
/// §7.3 input checks run first, the key's before the ciphertext's, so a malformed key is reported
/// whatever ciphertext arrives with it.
pub fn decaps(dk: &[u8], c: &[u8]) -> Result<[u8; 32], KemError> {
    if dk.len() != DK_BYTES {
        return Err(KemError::DecapsulationKeyLength { len: dk.len() });
    }
    let (dk_pke, rest) = dk.split_at(POLY_BYTES * K);
    let (ek, rest) = rest.split_at(EK_BYTES);
    let (h, z) = rest.split_at(32);
    if sha3_256_bytes(ek) != h {
        return Err(KemError::DecapsulationKeyCorrupt);
    }
    if c.len() != CT_BYTES {
        return Err(KemError::CiphertextLength { len: c.len() });
    }
    Ok(fo_decapsulate(dk_pke, ek, h, z, c))
}

/// FIPS 203 §7.2: the type check (exactly [`EK_BYTES`]) and the modulus check — every 12-bit field of
/// t̂ below q, which is `ByteEncode₁₂(ByteDecode₁₂(ek[0:384k])) = ek[0:384k]`.
fn check_encapsulation_key(ek: &[u8]) -> Result<(), KemError> {
    if ek.len() != EK_BYTES {
        return Err(KemError::EncapsulationKeyLength { len: ek.len() });
    }
    let reduced = ek[..POLY_BYTES * K].chunks_exact(3).all(|pair| {
        let lo = pair[0] as u16 | ((pair[1] as u16 & 0x0F) << 8);
        let hi = (pair[1] as u16 >> 4) | ((pair[2] as u16) << 4);
        lo < Q as u16 && hi < Q as u16
    });
    if reduced {
        Ok(())
    } else {
        Err(KemError::EncapsulationKeyNotReduced)
    }
}

/// The Fujisaki–Okamoto core of ML-KEM.Decaps_internal (FIPS 203 Algorithm 18, lines 7–11):
/// decrypt, re-encrypt, and keep K′ only if the re-encryption is the ciphertext, else K̄ = J(z ‖ c).
/// Both keys are always derived and the choice is a mask over the full-length compare, so the
/// time it takes says nothing about whether c was honest.
fn fo_decapsulate(dk_pke: &[u8], ek: &[u8], h: &[u8], z: &[u8], c: &[u8]) -> [u8; 32] {
    let m_prime = kpke_decrypt(dk_pke, c);
    let (k_prime, r_prime) = g(&[m_prime.as_slice(), h].concat());
    let mut k_bar = [0u8; 32];
    k_bar.copy_from_slice(&shake256_bytes(&[z, c].concat(), 32));
    let c_prime = kpke_encrypt(ek, &m_prime, &r_prime);
    ct_select(ct_eq_mask(c, &c_prime), &k_prime, &k_bar)
}

/// `0xFF` when `a` and `b` (of equal length) hold the same bytes, else 0: every byte is folded in
/// before the verdict, which `black_box` keeps the optimizer from turning back into a branch.
#[inline]
fn ct_eq_mask(a: &[u8], b: &[u8]) -> u8 {
    debug_assert_eq!(a.len(), b.len());
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b) {
        diff |= x ^ y;
    }
    std::hint::black_box(((diff as u16).wrapping_sub(1) >> 8) as u8)
}

/// `a` under an all-ones `mask`, `b` under zero, byte by byte.
#[inline]
fn ct_select(mask: u8, a: &[u8; 32], b: &[u8; 32]) -> [u8; 32] {
    std::array::from_fn(|i| (a[i] & mask) | (b[i] & !mask))
}

// ── Logos-facing wrappers (Seq of Int bytes 0..255) — the natives crypto.lg's handshake calls ────

/// `mlkemNoiseBatch(seed, base, count)` → `count·256` raw CBD_2 Word16 coefficients (nonces
/// `base..base+count`, no NTT), the flat concatenation of `count` noise polynomials. The fast
/// primitive the Logos `mlkem768Keygen`/`mlkemEncrypt` call once instead of `count` scalar
/// `mlkemPrfNoise`s — one 4-way SHAKE256 permutation covers four PRF streams (see `noise_batch_raw`).
pub fn mlkem_noise_batch_from_int(
    seed: &[i64],
    base: i64,
    count: i64,
) -> Result<logicaffeine_data::LogosSeq<Word16>, String> {
    let s = args::byte_array::<32>("seed", seed)?;
    let base = args::int_in("first nonce", base, 0, 255)?;
    let n = args::int_in("count", count, 0, 256)?;
    if n > 0 && base + n - 1 > 255 {
        return Err(format!(
            "the nonces run from {base} to {}; each must be a byte (0 to 255)",
            base + n - 1
        ));
    }
    let n = n as usize;
    let polys = noise_batch_raw(&s, base as u8, n);
    let mut out: Vec<Word16> = Vec::with_capacity(n * N);
    for p in &polys {
        out.extend_from_slice(p);
    }
    Ok(logicaffeine_data::LogosSeq::from_vec(out))
}

/// `mlkemKeypair(d, z)` → ek(1184) ‖ dk(2400).
pub fn mlkem_keypair_seq(d: &[i64], z: &[i64]) -> Result<logicaffeine_data::LogosSeq<i64>, String> {
    let d = args::byte_array::<32>("seed d", d)?;
    let z = args::byte_array::<32>("seed z", z)?;
    let (ek, dk) = keygen(&d, &z);
    let mut out = ek;
    out.extend(dk);
    Ok(args::seq_of_bytes(&out))
}
/// `mlkemEncapsKem(ek, m)` → ciphertext(1088) ‖ shared_secret(32). The ek is the peer's: one that
/// fails FIPS 203's checks is answered with an empty sequence, never encapsulated to.
pub fn mlkem_encaps_seq(ek: &[i64], m: &[i64]) -> Result<logicaffeine_data::LogosSeq<i64>, String> {
    let ek = args::bytes("encapsulation key", ek)?;
    let m = args::byte_array::<32>("randomness m", m)?;
    Ok(match encaps(&ek, &m) {
        Ok((ct, ss)) => {
            let mut out = ct;
            out.extend_from_slice(&ss);
            args::seq_of_bytes(&out)
        }
        Err(_) => logicaffeine_data::LogosSeq::from_vec(Vec::new()),
    })
}
/// `mlkemDecapsKem(dk, ct)` → shared_secret(32). The ciphertext is the peer's: one of the wrong
/// length is answered with an empty sequence; a malformed decapsulation key fails the program.
pub fn mlkem_decaps_seq(
    dk: &[i64],
    ct: &[i64],
) -> Result<logicaffeine_data::LogosSeq<i64>, String> {
    let dk = args::bytes("decapsulation key", dk)?;
    let ct = args::bytes("ciphertext", ct)?;
    match decaps(&dk, &ct) {
        Ok(ss) => Ok(args::seq_of_bytes(&ss)),
        Err(KemError::CiphertextLength { .. }) => {
            Ok(logicaffeine_data::LogosSeq::from_vec(Vec::new()))
        }
        Err(e) => Err(e.to_string()),
    }
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_system/mlkem/tests.rs"]
mod tests;
