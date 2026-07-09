//! ML-KEM-768 K-PKE.KeyGen, assembled from this crate's verified native primitives exactly the
//! way the Logos `mlkem768.lg` orchestration will, and gated **bit-exact** against the RustCrypto
//! `ml-kem` oracle (FIPS 203). This pins down every byte order (matrix index order, PRF nonce,
//! ByteEncode12 reduction, ρ placement, the post-basemul `tomont`) in a fast pure-kernel loop, so
//! the Logos port has a precise, oracle-true target. The oracle is dev-only; nothing here ships.

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_data::LogosSeq;
use logicaffeine_system::keccak::{sha3_256_bytes, sha3_512_bytes, shake256_bytes};
use logicaffeine_system::ntt::{
    mlkem_base_mul, mlkem_byte_decode, mlkem_byte_encode, mlkem_cbd2, mlkem_compress,
    mlkem_decompress, mlkem_inv_ntt, mlkem_ntt, mlkem_sample_a, mlkem_to_mont,
};

const K: usize = 3; // ML-KEM-768 module rank
const Q: i64 = 3329;
const N: usize = 256;
const DU: i64 = 10; // ciphertext u compression
const DV: i64 = 4; //  ciphertext v compression

fn seq(v: Vec<i64>) -> Vec<i64> {
    v
}
fn bytes_seq(b: &[u8]) -> Vec<i64> {
    b.iter().map(|&x| x as i64).collect()
}

/// G = SHA3-512, split into two 32-byte halves (ρ, σ).
fn g_split(input: &[u8]) -> ([u8; 32], [u8; 32]) {
    let h = sha3_512_bytes(input);
    let mut rho = [0u8; 32];
    let mut sigma = [0u8; 32];
    rho.copy_from_slice(&h[..32]);
    sigma.copy_from_slice(&h[32..]);
    (rho, sigma)
}

/// PRF_η1 for ML-KEM-768 (η1 = 2): SHAKE256(σ ‖ nonce, 64·η1 = 128 bytes), then CBD_2.
fn sample_noise_ntt(sigma: &[u8; 32], nonce: u8) -> Vec<i64> {
    let mut prf_in = sigma.to_vec();
    prf_in.push(nonce);
    let buf = shake256_bytes(&prf_in, 128);
    let poly = mlkem_cbd2(&bytes_seq(&buf)).to_vec();
    mlkem_ntt(&seq(poly)).to_vec()
}

/// K-PKE.KeyGen(d) → the ML-KEM encapsulation key ek = ByteEncode12(t̂) ‖ ρ.
fn kpke_keygen_ek(d: &[u8; 32]) -> Vec<u8> {
    let mut g_in = d.to_vec();
    g_in.push(K as u8);
    let (rho, sigma) = g_split(&g_in);

    // Â[i][j] = SampleNTT(XOF(ρ, j, i)) — FIPS 203 absorbs the column byte before the row byte.
    let mut a_hat = vec![vec![Vec::<i64>::new(); K]; K];
    for (i, row) in a_hat.iter_mut().enumerate() {
        for (j, cell) in row.iter_mut().enumerate() {
            *cell = mlkem_sample_a(&bytes_seq(&rho), j as i64, i as i64).to_vec();
        }
    }

    // ŝ, ê = NTT(CBD_2(PRF(σ, N))); nonce runs 0..K for s, then K..2K for e.
    let mut nonce = 0u8;
    let s_hat: Vec<Vec<i64>> = (0..K)
        .map(|_| {
            let p = sample_noise_ntt(&sigma, nonce);
            nonce += 1;
            p
        })
        .collect();
    let e_hat: Vec<Vec<i64>> = (0..K)
        .map(|_| {
            let p = sample_noise_ntt(&sigma, nonce);
            nonce += 1;
            p
        })
        .collect();

    // t̂[i] = tomont( Σ_j Â[i][j] ∘ ŝ[j] ) + ê[i]   (reduced mod q).
    let mut ek = Vec::new();
    for i in 0..K {
        let mut acc = vec![0i64; N];
        for j in 0..K {
            let prod = mlkem_base_mul(&seq(a_hat[i][j].clone()), &seq(s_hat[j].clone())).to_vec();
            for c in 0..N {
                acc[c] = (acc[c] + prod[c]) % Q;
            }
        }
        let mont = mlkem_to_mont(&seq(acc)).to_vec();
        let ti: Vec<i64> = (0..N).map(|c| (mont[c] + e_hat[i][c]) % Q).collect();
        let enc = mlkem_byte_encode(&seq(ti), 12).to_vec();
        ek.extend(enc.iter().map(|&x| x as u8));
    }
    ek.extend_from_slice(&rho);
    ek
}

/// Word16-carrier K-PKE.KeyGen — same scheme, coefficients carried as `Word16` and bytes as `u8`,
/// so the kernels never round-trip through i64. The end-to-end speed of the Word16 representation.
fn kpke_keygen_ek_w16(d: &[u8; 32]) -> Vec<u8> {
    use logicaffeine_base::Word16;
    use logicaffeine_system::ntt::*;
    let q = Q as u32;
    let mut g_in = d.to_vec();
    g_in.push(K as u8);
    let (rho, sigma) = g_split(&g_in);

    // 4-way AVX2 SHAKE128 batched matrix expansion: entry (r,c) at slot (r·3+c)·256. Keygen uses
    // Âᵀ, so a_hat[i][j] = entry (j,i).
    let matrix = mlkem_sample_matrix_w16(&rho);
    let mut a_hat = vec![vec![Vec::<Word16>::new(); K]; K];
    for (i, row) in a_hat.iter_mut().enumerate() {
        for (j, cell) in row.iter_mut().enumerate() {
            let e = (j * K + i) * N;
            *cell = matrix[e..e + N].to_vec();
        }
    }
    let mut nonce = 0u8;
    let noise = |sigma: &[u8; 32], n: u8| -> Vec<Word16> {
        let mut pin = sigma.to_vec();
        pin.push(n);
        mlkem_ntt_w16(&mlkem_cbd2_w16(&shake256_bytes(&pin, 128)))
    };
    let s_hat: Vec<Vec<Word16>> = (0..K).map(|_| { let p = noise(&sigma, nonce); nonce += 1; p }).collect();
    let e_hat: Vec<Vec<Word16>> = (0..K).map(|_| { let p = noise(&sigma, nonce); nonce += 1; p }).collect();

    let mut ek = Vec::new();
    for i in 0..K {
        let mut acc = vec![Word16(0); N];
        for j in 0..K {
            let prod = mlkem_base_mul_w16(&a_hat[i][j], &s_hat[j]);
            for c in 0..N {
                acc[c] = Word16(((acc[c].0 as u32 + prod[c].0 as u32) % q) as u16);
            }
        }
        let mont = mlkem_to_mont_w16(&acc);
        let ti: Vec<Word16> =
            (0..N).map(|c| Word16(((mont[c].0 as u32 + e_hat[i][c].0 as u32) % q) as u16)).collect();
        ek.extend(mlkem_byte_encode_w16(&ti, 12));
    }
    ek.extend_from_slice(&rho);
    ek
}

// ── poly/primitive shorthands (each delegates to a verified native kernel) ────────────────────

fn ntt(p: Vec<i64>) -> Vec<i64> {
    mlkem_ntt(&seq(p)).to_vec()
}
fn inv_ntt(p: Vec<i64>) -> Vec<i64> {
    mlkem_inv_ntt(&seq(p)).to_vec()
}
fn basemul(a: &[i64], b: &[i64]) -> Vec<i64> {
    mlkem_base_mul(&seq(a.to_vec()), &seq(b.to_vec())).to_vec()
}
fn compress(p: &[i64], d: i64) -> Vec<i64> {
    mlkem_compress(&seq(p.to_vec()), d).to_vec()
}
fn decompress(p: &[i64], d: i64) -> Vec<i64> {
    mlkem_decompress(&seq(p.to_vec()), d).to_vec()
}
fn byte_encode(p: &[i64], d: i64) -> Vec<u8> {
    mlkem_byte_encode(&seq(p.to_vec()), d).to_vec().iter().map(|&x| x as u8).collect()
}
fn byte_decode(b: &[u8], d: i64) -> Vec<i64> {
    mlkem_byte_decode(&bytes_seq(b), d).to_vec()
}
fn add_modq(a: &[i64], b: &[i64]) -> Vec<i64> {
    (0..N).map(|c| (a[c] + b[c]).rem_euclid(Q)).collect()
}
fn sub_modq(a: &[i64], b: &[i64]) -> Vec<i64> {
    (0..N).map(|c| (a[c] - b[c]).rem_euclid(Q)).collect()
}
/// A CBD_2 noise polynomial (normal domain) from PRF(seed, nonce); not yet NTT-transformed.
fn cbd_poly(seed: &[u8; 32], nonce: u8) -> Vec<i64> {
    let mut prf_in = seed.to_vec();
    prf_in.push(nonce);
    mlkem_cbd2(&bytes_seq(&shake256_bytes(&prf_in, 128))).to_vec()
}

/// K-PKE.Encrypt(ek, m, r) → ciphertext c = ByteEncode_du(Compress_du(u)) ‖ ByteEncode_dv(Compress_dv(v)).
fn kpke_encrypt(ek: &[u8], m: &[u8; 32], r: &[u8; 32]) -> Vec<u8> {
    let t_hat: Vec<Vec<i64>> = (0..K).map(|i| byte_decode(&ek[384 * i..384 * (i + 1)], 12)).collect();
    let rho = &ek[384 * K..384 * K + 32];

    let r_hat: Vec<Vec<i64>> = (0..K).map(|i| ntt(cbd_poly(r, i as u8))).collect();
    let e1: Vec<Vec<i64>> = (0..K).map(|i| cbd_poly(r, (K + i) as u8)).collect();
    let e2 = cbd_poly(r, (2 * K) as u8);

    // u[i] = NTT⁻¹( Σ_j Âᵀ[i][j] ∘ r̂[j] ) + e1[i];  Âᵀ[i][j] = SampleNTT(XOF(ρ, i, j)).
    // The Kyber invNTT is `tomont`, so its ×R cancels basemul's ×R⁻¹ — no explicit tomont here.
    let mut u = Vec::with_capacity(K);
    for i in 0..K {
        let mut acc = vec![0i64; N];
        for j in 0..K {
            let at = mlkem_sample_a(&bytes_seq(rho), i as i64, j as i64).to_vec();
            let prod = basemul(&at, &r_hat[j]);
            for c in 0..N {
                acc[c] = (acc[c] + prod[c]) % Q;
            }
        }
        u.push(add_modq(&inv_ntt(acc), &e1[i]));
    }

    let mu = decompress(&byte_decode(m, 1), 1);
    let mut acc = vec![0i64; N];
    for i in 0..K {
        let prod = basemul(&t_hat[i], &r_hat[i]);
        for c in 0..N {
            acc[c] = (acc[c] + prod[c]) % Q;
        }
    }
    let v = add_modq(&add_modq(&inv_ntt(acc), &e2), &mu);

    let mut c = Vec::new();
    for poly in &u {
        c.extend(byte_encode(&compress(poly, DU), DU));
    }
    c.extend(byte_encode(&compress(&v, DV), DV));
    c
}

/// K-PKE.Decrypt(dk_pke, c) → recovered message m = ByteEncode1(Compress1(v − NTT⁻¹(ŝᵀ ∘ NTT(u)))).
fn kpke_decrypt(dk_pke: &[u8], c: &[u8]) -> [u8; 32] {
    let s_hat: Vec<Vec<i64>> =
        (0..K).map(|i| byte_decode(&dk_pke[384 * i..384 * (i + 1)], 12)).collect();
    let u: Vec<Vec<i64>> = (0..K)
        .map(|i| {
            let bytes = &c[32 * DU as usize * i..32 * DU as usize * (i + 1)];
            decompress(&byte_decode(bytes, DU), DU)
        })
        .collect();
    let v = decompress(&byte_decode(&c[32 * DU as usize * K..], DV), DV);

    let mut acc = vec![0i64; N];
    for i in 0..K {
        let prod = basemul(&s_hat[i], &ntt(u[i].clone()));
        for cc in 0..N {
            acc[cc] = (acc[cc] + prod[cc]) % Q;
        }
    }
    let w = sub_modq(&v, &inv_ntt(acc));
    let mut m = [0u8; 32];
    m.copy_from_slice(&byte_encode(&compress(&w, 1), 1));
    m
}

/// ML-KEM.Encaps(ek, m): (K, r) ← G(m ‖ H(ek)); c ← Encrypt(ek, m, r); return (c, K).
fn mlkem_encaps(ek: &[u8], m: &[u8; 32]) -> (Vec<u8>, [u8; 32]) {
    let h = sha3_256_bytes(ek);
    let (k, r) = g_split(&[m.as_slice(), &h].concat());
    (kpke_encrypt(ek, m, &r), k)
}

/// ML-KEM.Decaps(dk, c): m' ← Decrypt; (K', r') ← G(m' ‖ h); re-encrypt; implicit-reject to J(z‖c).
fn mlkem_decaps(dk: &[u8], c: &[u8]) -> [u8; 32] {
    let dk_pke = &dk[0..384 * K];
    let ek = &dk[384 * K..768 * K + 32];
    let h = &dk[768 * K + 32..768 * K + 64];
    let z = &dk[768 * K + 64..768 * K + 96];

    let m_prime = kpke_decrypt(dk_pke, c);
    let (k_prime, r_prime) = g_split(&[m_prime.as_slice(), h].concat());
    let mut k_bar = [0u8; 32];
    k_bar.copy_from_slice(&shake256_bytes(&[z, c].concat(), 32));

    if c == kpke_encrypt(ek, &m_prime, &r_prime) {
        k_prime
    } else {
        k_bar
    }
}

// ── Word16-carrier reference (a faithful Rust proxy for the SHIPPED compiled-Logos `crypto.lg`
// path: Word16 coefficients, the 4-way AVX2 matrix sampler, AVX2 NTT/invNTT). Bit-exact vs the i64
// reference + the oracle; this is what the benchmark measures so the numbers reflect what ships. ──
fn cbd_poly_w16(seed: &[u8], nonce: u8) -> Vec<logicaffeine_base::Word16> {
    let mut pin = seed.to_vec();
    pin.push(nonce);
    logicaffeine_system::ntt::mlkem_cbd2_w16(&shake256_bytes(&pin, 128))
}
fn addq_w16(
    a: &[logicaffeine_base::Word16],
    b: &[logicaffeine_base::Word16],
) -> Vec<logicaffeine_base::Word16> {
    let q = Q as u32;
    (0..N).map(|c| logicaffeine_base::Word16(((a[c].0 as u32 + b[c].0 as u32) % q) as u16)).collect()
}

fn kpke_encrypt_w16(ek: &[u8], m: &[u8; 32], r: &[u8; 32]) -> Vec<u8> {
    use logicaffeine_base::Word16;
    use logicaffeine_system::ntt::*;
    let t_hat: Vec<Vec<Word16>> =
        (0..K).map(|i| mlkem_byte_decode_w16(&ek[384 * i..384 * (i + 1)], 12)).collect();
    let rho = &ek[384 * K..384 * K + 32];
    let matrix = mlkem_sample_matrix_w16(rho); // entry Â[i][j] at slot (i·K+j)·N

    let r_hat: Vec<Vec<Word16>> = (0..K).map(|i| mlkem_ntt_w16(&cbd_poly_w16(r, i as u8))).collect();
    let e1: Vec<Vec<Word16>> = (0..K).map(|i| cbd_poly_w16(r, (K + i) as u8)).collect();
    let e2 = cbd_poly_w16(r, (2 * K) as u8);

    let mut u = Vec::with_capacity(K);
    for i in 0..K {
        let mut acc = vec![Word16(0); N];
        for j in 0..K {
            let e = (i * K + j) * N;
            acc = addq_w16(&acc, &mlkem_base_mul_w16(&matrix[e..e + N], &r_hat[j]));
        }
        u.push(addq_w16(&mlkem_inv_ntt_w16(&acc), &e1[i]));
    }
    let mu = mlkem_decompress_w16(&mlkem_byte_decode_w16(m, 1), 1);
    let mut acc = vec![Word16(0); N];
    for i in 0..K {
        acc = addq_w16(&acc, &mlkem_base_mul_w16(&t_hat[i], &r_hat[i]));
    }
    let v = addq_w16(&addq_w16(&mlkem_inv_ntt_w16(&acc), &e2), &mu);

    let mut c = Vec::new();
    for poly in &u {
        c.extend(mlkem_byte_encode_w16(&mlkem_compress_w16(poly, DU as usize), DU as usize));
    }
    c.extend(mlkem_byte_encode_w16(&mlkem_compress_w16(&v, DV as usize), DV as usize));
    c
}

fn kpke_decrypt_w16(dk_pke: &[u8], c: &[u8]) -> [u8; 32] {
    use logicaffeine_base::Word16;
    use logicaffeine_system::ntt::*;
    let q = Q as u32;
    let s_hat: Vec<Vec<Word16>> =
        (0..K).map(|i| mlkem_byte_decode_w16(&dk_pke[384 * i..384 * (i + 1)], 12)).collect();
    let u: Vec<Vec<Word16>> = (0..K)
        .map(|i| {
            let bytes = &c[32 * DU as usize * i..32 * DU as usize * (i + 1)];
            mlkem_decompress_w16(&mlkem_byte_decode_w16(bytes, DU as usize), DU as usize)
        })
        .collect();
    let v = mlkem_decompress_w16(&mlkem_byte_decode_w16(&c[32 * DU as usize * K..], DV as usize), DV as usize);

    let mut acc = vec![Word16(0); N];
    for i in 0..K {
        acc = addq_w16(&acc, &mlkem_base_mul_w16(&s_hat[i], &mlkem_ntt_w16(&u[i])));
    }
    let inv = mlkem_inv_ntt_w16(&acc);
    let w: Vec<Word16> =
        (0..N).map(|c| Word16(((v[c].0 as u32 + q - inv[c].0 as u32) % q) as u16)).collect();
    let mut m = [0u8; 32];
    m.copy_from_slice(&mlkem_byte_encode_w16(&mlkem_compress_w16(&w, 1), 1));
    m
}

fn mlkem_encaps_w16(ek: &[u8], m: &[u8; 32]) -> (Vec<u8>, [u8; 32]) {
    let h = sha3_256_bytes(ek);
    let (k, r) = g_split(&[m.as_slice(), &h].concat());
    (kpke_encrypt_w16(ek, m, &r), k)
}

fn mlkem_decaps_w16(dk: &[u8], c: &[u8]) -> [u8; 32] {
    let dk_pke = &dk[0..384 * K];
    let ek = &dk[384 * K..768 * K + 32];
    let h = &dk[768 * K + 32..768 * K + 64];
    let z = &dk[768 * K + 64..768 * K + 96];

    let m_prime = kpke_decrypt_w16(dk_pke, c);
    let (k_prime, r_prime) = g_split(&[m_prime.as_slice(), h].concat());
    let mut k_bar = [0u8; 32];
    k_bar.copy_from_slice(&shake256_bytes(&[z, c].concat(), 32));

    if c == kpke_encrypt_w16(ek, &m_prime, &r_prime) {
        k_prime
    } else {
        k_bar
    }
}

#[test]
fn word16_encaps_decaps_is_bit_exact_vs_oracle() {
    use ml_kem::kem::Decapsulate;
    use ml_kem::{B32, EncapsulateDeterministic, EncodedSizeUser, KemCore, MlKem768};
    let d = [0x44u8; 32];
    let z = [0x55u8; 32];
    let m = [0x66u8; 32];
    let (dk_o, ek_o) = MlKem768::generate_deterministic(&B32::from(d), &B32::from(z));
    let ek_bytes = ek_o.as_bytes().to_vec();
    let dk_bytes = dk_o.as_bytes().to_vec();

    // Word16 encaps == i64 reference == oracle (ciphertext + shared secret).
    let (our_c, our_k) = mlkem_encaps_w16(&ek_bytes, &m);
    let (ref_c, ref_k) = mlkem_encaps(&ek_bytes, &m);
    assert_eq!(our_c, ref_c, "Word16 encaps ciphertext == i64 reference");
    assert_eq!(our_k, ref_k, "Word16 encaps shared secret == i64 reference");
    let (oracle_ct, oracle_k) = ek_o.encapsulate_deterministic(&B32::from(m)).unwrap();
    assert_eq!(our_c, oracle_ct.as_slice(), "Word16 encaps ciphertext == FIPS-203 oracle");
    assert_eq!(&our_k, oracle_k.as_slice(), "Word16 encaps shared secret == FIPS-203 oracle");

    // Word16 decaps recovers the shared secret, and implicit-rejects a tampered ciphertext.
    assert_eq!(mlkem_decaps_w16(&dk_bytes, &our_c), our_k, "Word16 decaps recovers K");
    assert_eq!(mlkem_decaps_w16(&dk_bytes, &our_c), dk_o.decapsulate(&oracle_ct).unwrap().as_slice());
    let mut tampered = our_c.clone();
    tampered[0] ^= 1;
    assert_ne!(mlkem_decaps_w16(&dk_bytes, &tampered), our_k, "tampered ct ⇒ implicit reject");
}

#[test]
fn kpke_keygen_encapsulation_key_is_bit_exact_vs_oracle() {
    use ml_kem::{B32, EncodedSizeUser, KemCore, MlKem768};

    let d = [0x11u8; 32];
    let z = [0x22u8; 32];

    let ours = kpke_keygen_ek(&d);
    assert_eq!(ours.len(), 384 * K + 32, "ek = {K}·384 + 32 = 1184 bytes");

    let (_dk, ek) = MlKem768::generate_deterministic(&B32::from(d), &B32::from(z));
    let oracle: Vec<u8> = ek.as_bytes().to_vec();

    assert_eq!(
        ours, oracle,
        "Logos-primitive K-PKE.KeyGen must reproduce the FIPS-203 encapsulation key byte-for-byte"
    );
}

#[test]
fn full_mlkem768_encaps_decaps_is_bit_exact_vs_oracle() {
    use ml_kem::kem::Decapsulate;
    use ml_kem::{B32, EncapsulateDeterministic, EncodedSizeUser, KemCore, MlKem768};

    let d = [0x11u8; 32];
    let z = [0x22u8; 32];
    let m = [0x33u8; 32];

    let (dk_oracle, ek_oracle) = MlKem768::generate_deterministic(&B32::from(d), &B32::from(z));
    let ek_bytes: Vec<u8> = ek_oracle.as_bytes().to_vec();
    let dk_bytes: Vec<u8> = dk_oracle.as_bytes().to_vec();

    // Encaps: ciphertext + shared secret must both be bit-exact.
    let (our_c, our_k) = mlkem_encaps(&ek_bytes, &m);
    let (oracle_ct, oracle_k) = ek_oracle.encapsulate_deterministic(&B32::from(m)).expect("encaps");
    assert_eq!(our_c, oracle_ct.to_vec(), "ML-KEM-768 ciphertext must match the FIPS-203 oracle");
    assert_eq!(our_k.to_vec(), oracle_k.to_vec(), "encapsulated shared secret must match the oracle");

    // Decaps: our decapsulation recovers the shared secret (FO round-trip), and the oracle agrees.
    let our_recovered = mlkem_decaps(&dk_bytes, &our_c);
    assert_eq!(our_recovered, our_k, "Decaps∘Encaps must recover the shared secret");
    let oracle_dec = dk_oracle.decapsulate(&oracle_ct).expect("decaps");
    assert_eq!(our_recovered.to_vec(), oracle_dec.to_vec(), "our Decaps must equal the oracle's");

    // Implicit-rejection branch: a tampered ciphertext must yield the J(z‖c) pseudo-secret, never K.
    let mut bad = our_c.clone();
    bad[0] ^= 0x01;
    let rejected = mlkem_decaps(&dk_bytes, &bad);
    assert_ne!(rejected, our_k, "a corrupted ciphertext must not decapsulate to the true secret");
    assert_eq!(
        rejected.to_vec(),
        dk_oracle.decapsulate(&oracle_ct_from(&bad)).expect("decaps").to_vec(),
        "implicit-reject output must match the oracle on the same corrupted ciphertext"
    );
}

/// Rebuild an oracle `Ciphertext` from raw bytes so we can feed our corrupted ciphertext to the
/// oracle's decapsulate and compare the implicit-rejection secret.
fn oracle_ct_from(bytes: &[u8]) -> ml_kem::Ciphertext<ml_kem::MlKem768> {
    use ml_kem::array::Array;
    let mut a = Array::default();
    a.copy_from_slice(bytes);
    a
}

/// Speed race: our native-primitive composition (the floor for the compiled-Logos path — the Logos
/// AOT calls these very kernels) vs RustCrypto `ml-kem`. Both are bit-exact; this is purely timing.
#[test]
#[ignore = "benchmark — run explicitly with --ignored --nocapture"]
fn bench_mlkem768_vs_rustcrypto() {
    use ml_kem::kem::Decapsulate;
    use ml_kem::{B32, EncapsulateDeterministic, EncodedSizeUser, KemCore, MlKem768};
    use std::time::Instant;

    const ITERS: u32 = 2000;
    let d = [0x11u8; 32];
    let z = [0x22u8; 32];
    let m = [0x33u8; 32];

    let (dk_o, ek_o) = MlKem768::generate_deterministic(&B32::from(d), &B32::from(z));
    let ek_bytes: Vec<u8> = ek_o.as_bytes().to_vec();
    let dk_bytes: Vec<u8> = dk_o.as_bytes().to_vec();
    let (our_c, _k) = mlkem_encaps(&ek_bytes, &m);
    let (oracle_ct, _ok) = ek_o.encapsulate_deterministic(&B32::from(m)).unwrap();

    macro_rules! time {
        ($label:expr, $body:expr) => {{
            for _ in 0..50 {
                std::hint::black_box($body);
            }
            let t = Instant::now();
            for _ in 0..ITERS {
                std::hint::black_box($body);
            }
            let ns = t.elapsed().as_nanos() as f64 / ITERS as f64;
            eprintln!("{:<28} {:>10.0} ns/op", $label, ns);
            ns
        }};
    }

    // Word16-carrier keygen must be bit-exact vs the oracle, then race it.
    assert_eq!(kpke_keygen_ek_w16(&d), ek_bytes, "Word16 keygen must match the FIPS-203 oracle ek");

    eprintln!("\n=== ML-KEM-768 timings ({} iters, this box) ===", ITERS);
    let lk = time!("ours  keygen", kpke_keygen_ek(&d));
    let lkw = time!("ours  keygen (Word16)", kpke_keygen_ek_w16(&d));
    let ok = time!("oracle keygen", {
        MlKem768::generate_deterministic(&B32::from(d), &B32::from(z)).1.as_bytes().to_vec()
    });
    let le = time!("ours  encaps (i64)", mlkem_encaps(&ek_bytes, &m));
    let lew = time!("ours  encaps (Word16)", mlkem_encaps_w16(&ek_bytes, &m));
    let oe = time!("oracle encaps", ek_o.encapsulate_deterministic(&B32::from(m)).unwrap());
    let ld = time!("ours  decaps (i64)", mlkem_decaps(&dk_bytes, &our_c));
    let ldw = time!("ours  decaps (Word16)", mlkem_decaps_w16(&dk_bytes, &our_c));
    let od = time!("oracle decaps", dk_o.decapsulate(&oracle_ct).unwrap());
    eprintln!("\n--- ratio ours/oracle (>1 = we are slower) ---");
    eprintln!("keygen(i64) {:.2}x   keygen(Word16) {:.2}x", lk / ok, lkw / ok);
    eprintln!("encaps(i64) {:.2}x   encaps(Word16) {:.2}x", le / oe, lew / oe);
    eprintln!("decaps(i64) {:.2}x   decaps(Word16) {:.2}x", ld / od, ldw / od);
}

/// Profile the components of our keygen to find the dominant cost.
#[test]
#[ignore = "profiler — run explicitly with --ignored --nocapture"]
fn profile_mlkem768_components() {
    use logicaffeine_system::keccak::{sha3_512_bytes, shake256_bytes};
    use std::time::Instant;
    const N: u32 = 5000;
    let rho = [0x11u8; 32];
    let rho_i: Vec<i64> = rho.iter().map(|&x| x as i64).collect();
    let sigma = [0x22u8; 32];
    // NTT-domain coefficients in [0, q): base_mul / to_mont consume reduced inputs by contract.
    let poly: Vec<i64> = (0..256).map(|i| (i * 37) % 3329).collect();

    macro_rules! t {
        ($label:expr, $n:expr, $body:expr) => {{
            for _ in 0..50 { std::hint::black_box($body); }
            let t = Instant::now();
            for _ in 0..$n { std::hint::black_box($body); }
            eprintln!("{:<26} {:>9.0} ns/op", $label, t.elapsed().as_nanos() as f64 / $n as f64);
        }};
    }
    eprintln!("\n=== keygen component costs ===");
    let rho_bytes: Vec<u8> = rho.to_vec();
    // The matrix-expansion keystone: 9 entries one-at-a-time vs the 4-way AVX2 batched sampler.
    t!("matrix ×9 single-lane", N / 4, {
        let mut all = Vec::new();
        for r in 0..3i64 {
            for c in 0..3i64 {
                all.push(logicaffeine_system::ntt::mlkem_sample_a_w16(&rho_bytes, r, c));
            }
        }
        all
    });
    t!("matrix 4-way batched", N / 4, logicaffeine_system::ntt::mlkem_sample_matrix_w16(&rho_bytes));
    t!("sampleA (×9 in keygen)", N, mlkem_sample_a(&rho_i, 0, 1));
    t!("shake256 128B (cbd prf)", N, shake256_bytes(&sigma, 128));
    t!("cbd2 of 128B", N, mlkem_cbd2(&vec![7i64; 128]));
    t!("ntt", N, mlkem_ntt(&poly));
    t!("sha3_512 (G)", N, sha3_512_bytes(&[1u8; 33]));
    t!("base_mul", N, mlkem_base_mul(&poly, &poly));
    t!("to_mont", N, mlkem_to_mont(&poly));
    t!("byte_encode d=12", N, mlkem_byte_encode(&poly, 12));

    // Word16 vs i64 NTT: the representation experiment.
    use logicaffeine_base::Word16;
    let poly_pos: Vec<i64> = (0..256).map(|i| (i * 37) % 3329).collect();
    let poly_w16: Vec<Word16> = poly_pos.iter().map(|&x| Word16(x as u16)).collect();
    // correctness: Word16 NTT == i64 NTT on the same [0,q) input.
    let a = logicaffeine_system::ntt::mlkem_ntt(&poly_pos).to_vec();
    let b: Vec<i64> = logicaffeine_system::ntt::mlkem_ntt_w16(&poly_w16).iter().map(|w| w.0 as i64).collect();
    assert_eq!(a, b, "Word16 NTT must match the i64 NTT bit-for-bit");
    use logicaffeine_system::ntt as nt;
    // correctness for the quartet
    assert_eq!(nt::mlkem_base_mul(&poly_pos, &poly_pos).to_vec(),
               nt::mlkem_base_mul_w16(&poly_w16, &poly_w16).iter().map(|w| w.0 as i64).collect::<Vec<_>>());
    assert_eq!(nt::mlkem_inv_ntt(&poly_pos).to_vec(),
               nt::mlkem_inv_ntt_w16(&poly_w16).iter().map(|w| w.0 as i64).collect::<Vec<_>>());
    assert_eq!(nt::mlkem_to_mont(&poly_pos).to_vec(),
               nt::mlkem_to_mont_w16(&poly_w16).iter().map(|w| w.0 as i64).collect::<Vec<_>>());
    eprintln!("\n=== representation experiment (i64 carrier vs Word16 carrier) ===");
    t!("ntt       i64", N, nt::mlkem_ntt(&poly_pos));
    t!("ntt       w16", N, nt::mlkem_ntt_w16(&poly_w16));
    t!("base_mul  i64", N, nt::mlkem_base_mul(&poly_pos, &poly_pos));
    t!("base_mul  w16", N, nt::mlkem_base_mul_w16(&poly_w16, &poly_w16));
    t!("inv_ntt   i64", N, nt::mlkem_inv_ntt(&poly_pos));
    t!("inv_ntt   w16", N, nt::mlkem_inv_ntt_w16(&poly_w16));
    t!("to_mont   i64", N, nt::mlkem_to_mont(&poly_pos));
    t!("to_mont   w16", N, nt::mlkem_to_mont_w16(&poly_w16));
}
