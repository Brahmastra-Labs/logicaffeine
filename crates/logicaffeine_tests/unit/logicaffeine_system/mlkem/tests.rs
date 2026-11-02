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

    let (ct, ss_a) = encaps(&ek, &m).expect("in-contract arguments");
    assert_eq!(ct.len(), CT_BYTES, "ct = 1088 bytes");
    let ss_b = decaps(&dk, &ct).expect("in-contract arguments");
    assert_eq!(ss_a, ss_b, "decaps must recover the encaps shared secret");

    // A tampered ciphertext implicit-rejects to a DIFFERENT (z-derived) secret, never the real one.
    let mut bad = ct.clone();
    bad[0] ^= 1;
    assert_ne!(
        decaps(&dk, &bad).expect("in-contract arguments"),
        ss_a,
        "tampered ct ⇒ implicit reject"
    );
}

#[test]
fn distinct_keypairs_give_distinct_secrets() {
    let m = [0x55u8; 32];
    let (ek1, _) = keygen(&[1u8; 32], &[2u8; 32]);
    let (ek2, _) = keygen(&[3u8; 32], &[4u8; 32]);
    assert_ne!(ek1, ek2, "distinct seeds ⇒ distinct public keys");
    assert_ne!(
        encaps(&ek1, &m).expect("in-contract arguments").1,
        encaps(&ek2, &m).expect("in-contract arguments").1,
        "distinct keys ⇒ distinct shared secrets"
    );
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
    let (ct, _) = encaps(&ek, &m).expect("in-contract arguments");

    let mut gin = d.to_vec();
    gin.push(K as u8);
    let (rho, sigma) = g(&gin);

    // keygen phases
    let g_ns = t(IT, || black_box(g(black_box(&gin))));
    let expand_ns = t(IT, || black_box(mlkem_sample_matrix_w16(black_box(&rho))));
    let noise_ns = t(IT, || {
        black_box(noise_batch_raw(black_box(&sigma), 0, 2 * K))
    });
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
    let decrypt_ns = t(IT, || {
        black_box(kpke_decrypt(black_box(dk_pke), black_box(&ct)))
    });
    let encrypt_ns = t(IT, || {
        black_box(kpke_encrypt(
            black_box(&ek),
            black_box(&m),
            black_box(&[7u8; 32]),
        ))
    });
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
    let a16: Vec<Word16> = (0..256)
        .map(|i| Word16((i * 7 % Q as usize) as u16))
        .collect();
    let b16: Vec<Word16> = (0..256)
        .map(|i| Word16((i * 13 % Q as usize) as u16))
        .collect();
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
            println!(
                "  keccak est. in ExpandA    {:>8.0}   (6 × 4-way perm)",
                6.0 * kperm_ns
            );
        }
    }
    println!("  base_mul (1 poly, scalar) {basemul_ns:>8.2}   ← ×9 keygen, ×15 decaps");
}

#[test]
fn addq_and_subq_are_modular_for_every_pair_of_reduced_coefficients() {
    for b in 0..Q as u16 {
        let bs = vec![Word16(b); N];
        for lo in (0..Q as u16).step_by(N) {
            let block: Vec<Word16> = (0..N as u16).map(|i| Word16((lo + i) % Q as u16)).collect();
            let sum = addq(&block, &bs);
            let diff = subq(&block, &bs);
            for i in 0..N {
                let a = block[i].0 as u32;
                assert_eq!(sum[i].0 as u32, (a + b as u32) % Q, "{a} + {b}");
                assert_eq!(diff[i].0 as u32, (a + Q - b as u32) % Q, "{a} − {b}");
            }
        }
    }
}

#[test]
fn ct_eq_mask_is_all_ones_exactly_when_every_byte_matches() {
    let a: Vec<u8> = (0..CT_BYTES).map(|i| (i * 31 + 7) as u8).collect();
    assert_eq!(ct_eq_mask(&a, &a.clone()), 0xFF, "equal ciphertexts");
    for at in [0, 1, 319, 640, 960, 1024, CT_BYTES - 1] {
        for bit in 0..8 {
            let mut b = a.clone();
            b[at] ^= 1 << bit;
            assert_eq!(ct_eq_mask(&a, &b), 0, "a flip of bit {bit} at byte {at}");
        }
    }
    let mut far = a.clone();
    far.iter_mut().for_each(|x| *x = !*x);
    assert_eq!(ct_eq_mask(&a, &far), 0, "every byte differs");
}

#[test]
fn ct_select_takes_the_first_under_an_all_ones_mask_and_the_second_under_zero() {
    let k: [u8; 32] = std::array::from_fn(|i| i as u8);
    let k_bar: [u8; 32] = std::array::from_fn(|i| 0xA0 ^ i as u8);
    assert_eq!(ct_select(0xFF, &k, &k_bar), k);
    assert_eq!(ct_select(0x00, &k, &k_bar), k_bar);
}

/// A fresh ML-KEM-768 key pair and an honest ciphertext under it.
fn fixture() -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let (ek, dk) = keygen(&[0x61; 32], &[0x62; 32]);
    let (ct, _) = encaps(&ek, &[0x63; 32]).expect("an honest encapsulation key encapsulates");
    (ek, dk, ct)
}

/// Overwrite the 12-bit coefficient `index` of the ByteEncode₁₂ polynomial starting at `poly_at`.
fn set_coefficient12(bytes: &mut [u8], poly_at: usize, index: usize, value: u16) {
    let bit = index * 12;
    let at = poly_at + bit / 8;
    if bit % 8 == 0 {
        bytes[at] = value as u8;
        bytes[at + 1] = (bytes[at + 1] & 0xF0) | (value >> 8) as u8;
    } else {
        bytes[at] = (bytes[at] & 0x0F) | ((value as u8 & 0x0F) << 4);
        bytes[at + 1] = (value >> 4) as u8;
    }
}

#[test]
fn encaps_refuses_an_encapsulation_key_of_the_wrong_length() {
    // FIPS 203 §7.2, the type check: ek must be 384·k + 32 bytes.
    let (ek, _, _) = fixture();
    for len in [0usize, 1, 32, EK_BYTES - 1, EK_BYTES + 1, 2 * EK_BYTES] {
        let mut wrong = ek.clone();
        wrong.resize(len, 0x11);
        assert_eq!(
            encaps(&wrong, &[0x63; 32]),
            Err(KemError::EncapsulationKeyLength { len }),
            "a {len}-byte encapsulation key is refused"
        );
    }
}

#[test]
fn encaps_refuses_an_encapsulation_key_that_is_not_reduced_mod_q() {
    // FIPS 203 §7.2, the modulus check: ByteEncode₁₂(ByteDecode₁₂(ek[0:384k])) = ek[0:384k], i.e.
    // every 12-bit field of t̂ is below q (Wycheproof mlkem_768_encaps ModulusOverflow).
    let (ek, _, _) = fixture();
    for poly in 0..3 {
        for index in [0usize, 1, 128, 254, 255] {
            for value in [3329u16, 3330, 4095] {
                let mut bad = ek.clone();
                set_coefficient12(&mut bad, 384 * poly, index, value);
                assert_eq!(
                    encaps(&bad, &[0x63; 32]),
                    Err(KemError::EncapsulationKeyNotReduced),
                    "t̂[{poly}][{index}] = {value} is refused"
                );
            }
        }
    }
    let mut edge = ek.clone();
    set_coefficient12(&mut edge, 0, 0, 3328);
    assert!(encaps(&edge, &[0x63; 32]).is_ok(), "q − 1 is reduced");
}

#[test]
fn decaps_refuses_a_ciphertext_of_the_wrong_length() {
    // FIPS 203 §7.3, the ciphertext type check: c must be 32·(du·k + dv) bytes.
    let (_, dk, ct) = fixture();
    for len in [0usize, 1, 960, CT_BYTES - 1, CT_BYTES + 1, 2 * CT_BYTES] {
        let mut wrong = ct.clone();
        wrong.resize(len, 0x22);
        assert_eq!(
            decaps(&dk, &wrong),
            Err(KemError::CiphertextLength { len }),
            "a {len}-byte ciphertext is refused"
        );
    }
}

#[test]
fn decaps_refuses_a_decapsulation_key_of_the_wrong_length() {
    // FIPS 203 §7.3, the decapsulation-key type check: dk must be 768·k + 96 bytes.
    let (_, dk, ct) = fixture();
    for len in [0usize, 1, 1152, DK_BYTES - 1, DK_BYTES + 1, 2 * DK_BYTES] {
        let mut wrong = dk.clone();
        wrong.resize(len, 0x33);
        assert_eq!(
            decaps(&wrong, &ct),
            Err(KemError::DecapsulationKeyLength { len }),
            "a {len}-byte decapsulation key is refused"
        );
    }
}

#[test]
fn decaps_refuses_a_decapsulation_key_whose_hash_is_not_its_eks() {
    // FIPS 203 §7.3, the hash check: H(dk[384k : 768k+32]) = dk[768k+32 : 768k+64].
    let (_, dk, ct) = fixture();
    let ek_at = 1152;
    let h_at = 1152 + EK_BYTES;
    for at in [
        h_at,
        h_at + 17,
        h_at + 31,
        ek_at,
        ek_at + 600,
        ek_at + EK_BYTES - 1,
    ] {
        let mut bad = dk.clone();
        bad[at] ^= 0x04;
        assert_eq!(
            decaps(&bad, &ct),
            Err(KemError::DecapsulationKeyCorrupt),
            "a flipped bit at dk byte {at} breaks H(ek) and is refused"
        );
    }
    assert!(decaps(&dk, &ct).is_ok(), "the untouched key decapsulates");
}

#[test]
fn kem_error_reports_read_as_the_runtime_states_them() {
    assert_eq!(
        KemError::EncapsulationKeyLength { len: 7 }.to_string(),
        "the encapsulation key is 7 bytes; an ML-KEM-768 encapsulation key is 1184"
    );
    assert_eq!(
        KemError::EncapsulationKeyNotReduced.to_string(),
        "the encapsulation key holds a coefficient of 3329 or more (FIPS 203 modulus check)"
    );
    assert_eq!(
        KemError::DecapsulationKeyLength { len: 9 }.to_string(),
        "the decapsulation key is 9 bytes; an ML-KEM-768 decapsulation key is 2400"
    );
    assert_eq!(
        KemError::DecapsulationKeyCorrupt.to_string(),
        "the decapsulation key's H(ek) is not the hash of its ek (FIPS 203 hash check)"
    );
    assert_eq!(
        KemError::CiphertextLength { len: 3 }.to_string(),
        "the ciphertext is 3 bytes; an ML-KEM-768 ciphertext is 1088"
    );
}
