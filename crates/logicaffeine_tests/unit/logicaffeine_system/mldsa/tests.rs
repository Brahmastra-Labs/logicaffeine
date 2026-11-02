use super::*;

/// Schoolbook negacyclic convolution mod (X²⁵⁶ + 1) over ℤ_q, the gold-standard reference.
fn schoolbook(a: &[i32; N], b: &[i32; N]) -> [i32; N] {
    let mut c = [0i64; N];
    for i in 0..N {
        for j in 0..N {
            let prod = a[i] as i64 * b[j] as i64;
            if i + j < N {
                c[i + j] += prod;
            } else {
                c[i + j - N] -= prod;
            }
        }
    }
    std::array::from_fn(|i| ((c[i] % Q as i64 + Q as i64) % Q as i64) as i32)
}

fn rand_coeff(s: &mut u64) -> i32 {
    *s = s
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    ((*s >> 33) % Q as u64) as i32
}
fn rand_poly(s: &mut u64) -> [i32; N] {
    std::array::from_fn(|_| rand_coeff(s))
}

#[test]
fn ntt_multiply_matches_schoolbook_convolution() {
    let mut s = 0xD15A_C0DEu64;
    for _ in 0..50 {
        let a = rand_poly(&mut s);
        let b = rand_poly(&mut s);
        let mut na = a;
        let mut nb = b;
        ntt(&mut na);
        ntt(&mut nb);
        let mut nc = pointwise_montgomery(&na, &nb);
        invntt_tomont(&mut nc);
        let got: [i32; N] = std::array::from_fn(|i| freeze(nc[i]));
        assert_eq!(
            got,
            schoolbook(&a, &b),
            "NTT product must equal the schoolbook convolution"
        );
    }
}

#[test]
fn power2round_reconstructs() {
    let mut s = 0xABCD_1234u64;
    for _ in 0..100000 {
        let r = rand_coeff(&mut s);
        let (r1, r0) = power2round(r);
        assert!(
            r0 > -(1 << (D - 1)) && r0 <= (1 << (D - 1)),
            "r0 ∈ (−2¹², 2¹²]"
        );
        assert_eq!(r1 * (1 << D) + r0, r, "r = r1·2ᵈ + r0");
    }
}

#[test]
fn decompose_reconstructs_and_bounds() {
    let mut s = 0x5678_9ABCu64;
    for _ in 0..100000 {
        let r = rand_coeff(&mut s);
        let (r1, r0) = decompose(r);
        assert!(r0 > -GAMMA2 && r0 <= GAMMA2, "r0 ∈ (−γ2, γ2]: got {r0}");
        assert!((0..16).contains(&r1), "r1 ∈ [0,16): got {r1}");
        // r1·2γ2 + r0 ≡ r (mod q)
        assert_eq!(freeze(r1 * 2 * GAMMA2 + r0), r, "r1·2γ2 + r0 = r (mod q)");
    }
}

#[test]
fn use_hint_inverts_make_hint() {
    // FIPS-204: UseHint(MakeHint(z, r), r) = HighBits(r + z) for |z| ≤ γ2.
    let mut s = 0xFEED_FACEu64;
    for _ in 0..100000 {
        let r = rand_coeff(&mut s);
        s = s.wrapping_mul(6364136223846793005).wrapping_add(1);
        let z = ((s >> 40) as i32 % (2 * GAMMA2 + 1)) - GAMMA2; // |z| ≤ γ2
        let h = make_hint(z, r);
        assert_eq!(
            use_hint(h, r),
            highbits(freeze(r + z)),
            "UseHint∘MakeHint = HighBits(r+z)"
        );
    }
}

#[test]
fn sign_verify_round_trips_and_rejects_tamper() {
    let (pk, sk) = keygen(&[0x33; 32]);
    let m = b"the quick brown fox jumps over the lazy dog";
    let ctx = b"ctx";
    let sig = sign(&sk, m, ctx).expect("in-contract arguments");
    assert_eq!(sig.len(), SIG_BYTES, "ML-DSA-65 sig = 3309 bytes");
    assert!(verify(&pk, m, ctx, &sig), "a fresh signature verifies");
    // Tampered message, tampered signature, and wrong context all reject.
    assert!(
        !verify(&pk, b"different message", ctx, &sig),
        "wrong message rejects"
    );
    assert!(!verify(&pk, m, b"other", &sig), "wrong context rejects");
    let mut bad = sig.clone();
    bad[100] ^= 1;
    assert!(!verify(&pk, m, ctx, &bad), "tampered signature rejects");
    // A signature under a different key rejects.
    let (pk2, _) = keygen(&[0x99; 32]);
    assert!(!verify(&pk2, m, ctx, &sig), "wrong public key rejects");
}

#[test]
fn keygen_sizes_and_determinism() {
    let (pk, sk) = keygen(&[0x11; 32]);
    assert_eq!(pk.len(), PK_BYTES, "ML-DSA-65 pk = 1952 bytes");
    assert_eq!(sk.len(), SK_BYTES, "ML-DSA-65 sk = 4032 bytes");
    assert_eq!(
        keygen(&[0x11; 32]).0,
        pk,
        "keygen is deterministic in the seed"
    );
    assert_ne!(keygen(&[0x22; 32]).0, pk, "distinct seeds ⇒ distinct keys");
}

#[test]
fn ntt_round_trip_is_identity() {
    // invntt_tomont(ntt(a)) = a·(2³²) — multiply by the Montgomery one (ntt of the unit) recovers a.
    let mut s = 0x1234_5678u64;
    let a = rand_poly(&mut s);
    // Multiply a by the constant polynomial 1 via NTT (1 in normal domain).
    let mut one = [0i32; N];
    one[0] = 1;
    let mut na = a;
    let mut n1 = one;
    ntt(&mut na);
    ntt(&mut n1);
    let mut prod = pointwise_montgomery(&na, &n1);
    invntt_tomont(&mut prod);
    let got: [i32; N] = std::array::from_fn(|i| freeze(prod[i]));
    let want: [i32; N] = std::array::from_fn(|i| freeze(a[i]));
    assert_eq!(got, want, "a · 1 through the NTT must recover a");
}

/// `n` bytes of a fixed, position-dependent pattern — distinct at every offset, so a context and a
/// message cut from one buffer never coincide by accident.
fn pattern(n: usize, salt: u8) -> Vec<u8> {
    (0..n)
        .map(|i| {
            (i as u32)
                .wrapping_mul(131)
                .wrapping_add(salt as u32 * 7 + 3) as u8
        })
        .collect()
}

#[test]
fn verify_refuses_a_context_longer_than_255_bytes() {
    // FIPS 204 Algorithm 3, lines 1–2: |ctx| > 255 → ⊥. The reported escape: one signature on
    // (ctx = "", T) must not verify as (ctx = T[..256], T[256..]), whose M' would carry the same
    // bytes once the length byte wraps to 0.
    let (pk, sk) = keygen(&[0x42; 32]);
    let t = pattern(300, 1);
    let sig = sign(&sk, &t, b"").expect("an empty context signs");
    assert!(verify(&pk, &t, b"", &sig), "the genuine pair verifies");
    assert!(
        !verify(&pk, &t[256..], &t[..256], &sig),
        "a 256-byte context must never verify (it aliased the empty context)"
    );
}

#[test]
fn sign_refuses_a_context_longer_than_255_bytes() {
    let (_, sk) = keygen(&[0x42; 32]);
    for len in [256usize, 257, 300, 511, 512, 4096] {
        assert_eq!(
            sign(&sk, b"message", &pattern(len, 2)),
            Err(SignError::ContextTooLong { len }),
            "a {len}-byte context is refused, never signed under another encoding"
        );
    }
}

#[test]
fn no_context_longer_than_255_bytes_aliases_a_shorter_one() {
    // For |C| = 256·k + n the wrapped encoding was M' = 0 ‖ n ‖ C ‖ M — exactly M' for the context
    // C[..n] with message C[n..] ‖ M. Every residue n must stay separated, on both sides.
    let (pk, sk) = keygen(&[0x5A; 32]);
    let m = pattern(40, 3);
    for (k, n) in [(1usize, 0usize), (1, 1), (1, 7), (1, 255), (2, 0), (2, 9)] {
        let c = pattern(256 * k + n, 4u8.wrapping_add(n as u8));
        let mut aliased_msg = c[n..].to_vec();
        aliased_msg.extend_from_slice(&m);
        let sig = sign(&sk, &aliased_msg, &c[..n]).expect("a short context signs");
        assert!(
            !verify(&pk, &m, &c, &sig),
            "|C| = {} must not verify a signature made for the context C[..{n}]",
            c.len()
        );
        assert_eq!(
            sign(&sk, &m, &c),
            Err(SignError::ContextTooLong { len: c.len() }),
            "|C| = {} is refused by sign",
            c.len()
        );
    }
}

#[test]
fn the_longest_context_signs_and_verifies_as_the_fips204_oracle_does() {
    use ml_dsa::{KeyGen, MlDsa65, Signature, B32};
    let seed = [0x6B; 32];
    let (pk, sk) = keygen(&seed);
    let oracle = MlDsa65::key_gen_internal(&B32::from(seed));
    let m = pattern(77, 5);
    for len in [0usize, 1, 128, 254, 255] {
        let ctx = pattern(len, 6);
        let sig = sign(&sk, &m, &ctx).expect("a context of at most 255 bytes signs");
        let want = oracle
            .signing_key()
            .sign_deterministic(&m, &ctx)
            .expect("the oracle signs it too");
        assert_eq!(
            sig.as_slice(),
            want.encode().as_slice(),
            "a {len}-byte context signs byte-for-byte as the oracle does"
        );
        assert!(verify(&pk, &m, &ctx, &sig), "and verifies");
        let parsed = Signature::<MlDsa65>::try_from(sig.as_slice()).unwrap();
        assert!(
            oracle
                .verifying_key()
                .verify_with_context(&m, &ctx, &parsed),
            "the oracle verifies it under the same {len}-byte context"
        );
    }
}

#[test]
fn a_context_the_oracle_refuses_is_refused_here_too() {
    use ml_dsa::{KeyGen, MlDsa65, Signature, B32};
    let seed = [0x6C; 32];
    let (pk, sk) = keygen(&seed);
    let oracle = MlDsa65::key_gen_internal(&B32::from(seed));
    let m = pattern(64, 7);
    let long = pattern(256, 8);
    assert!(
        oracle.signing_key().sign_deterministic(&m, &long).is_err(),
        "the FIPS-204 oracle refuses a 256-byte context"
    );
    assert!(sign(&sk, &m, &long).is_err(), "and so do we");
    let sig = sign(&sk, &m, b"").expect("an empty context signs");
    let parsed = Signature::<MlDsa65>::try_from(sig.as_slice()).unwrap();
    assert!(!oracle
        .verifying_key()
        .verify_with_context(&m, &long, &parsed));
    assert!(!verify(&pk, &m, &long, &sig));
}

#[test]
fn sign_refuses_a_secret_key_of_the_wrong_length() {
    let (_, sk) = keygen(&[0x21; 32]);
    for len in [0usize, 1, 127, SK_BYTES - 1, SK_BYTES + 1, 2 * SK_BYTES] {
        let mut wrong = sk.clone();
        wrong.resize(len, 0x5c);
        assert_eq!(
            sign(&wrong, b"message", b""),
            Err(SignError::SecretKeyLength { len }),
            "a {len}-byte secret key is refused"
        );
    }
}

#[test]
fn sign_refuses_a_secret_key_whose_s1_or_s2_leaves_the_eta_range() {
    // skDecode unpacks each s1/s2 coefficient as η − nibble; a nibble above 2η = 8 decodes outside
    // [−η, η], which no KeyGen produces (Wycheproof mldsa_65_sign_noseed tcId 56 and 57).
    let (_, sk) = keygen(&[0x22; 32]);
    let s1_at = 128;
    let s2_at = 128 + ML * 128;
    for (at, part) in [
        (s1_at, "s1"),
        (s1_at + 127, "s1"),
        (s2_at, "s2"),
        (s2_at + 127, "s2"),
    ] {
        for nibble in [9u8, 15] {
            for high in [false, true] {
                let mut bad = sk.clone();
                bad[at] = if high {
                    (bad[at] & 0x0f) | (nibble << 4)
                } else {
                    (bad[at] & 0xf0) | nibble
                };
                assert_eq!(
                    sign(&bad, b"message", b""),
                    Err(SignError::SecretKeyOutOfRange),
                    "{part} byte {at}: nibble {nibble} ({}) decodes to {} and is refused",
                    if high { "high" } else { "low" },
                    ETA - nibble as i32
                );
            }
        }
    }
    let mut edge = sk.clone();
    edge[s1_at] = 0x88;
    assert!(
        sign(&edge, b"message", b"").is_ok(),
        "nibble 8 decodes to −η, still in range"
    );
}

#[test]
fn sign_error_reports_read_as_the_runtime_states_them() {
    assert_eq!(
        SignError::ContextTooLong { len: 300 }.to_string(),
        "the context is 300 bytes; FIPS 204 allows at most 255"
    );
    assert_eq!(
        SignError::SecretKeyLength { len: 7 }.to_string(),
        "the secret key is 7 bytes; an ML-DSA-65 secret key is 4032"
    );
    assert_eq!(
        SignError::SecretKeyOutOfRange.to_string(),
        "the secret key holds an s1 or s2 coefficient outside [-4, 4]"
    );
}

/// The largest |coefficient| a matrix–vector row reaches before it returns to the normal domain:
/// Verify sums five Montgomery products and subtracts a sixth, each in (−q, q).
const ROW_SUM_BOUND: i32 = 6 * (Q - 1);

/// The normal-domain value of an NTT-domain row, computed inside `invntt_tomont`'s contract
/// (every input reduced into [0, q) first), as canonical representatives.
fn normal_domain_in_contract(acc: &[i32; N]) -> [i32; N] {
    let mut reduced: [i32; N] = std::array::from_fn(|i| acc[i].rem_euclid(Q));
    invntt_tomont(&mut reduced);
    std::array::from_fn(|i| reduced[i].rem_euclid(Q))
}

#[test]
fn a_row_sum_at_its_bound_returns_to_the_normal_domain_exactly() {
    // invntt_tomont's butterflies grow a coefficient up to 256-fold, so its input must stay below q
    // in magnitude; a row of six products reaches 6(q − 1). The return must reduce first.
    let mut s = 0xB0_0D5u64;
    let mut random = [0i32; N];
    for slot in random.iter_mut() {
        s = s
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        *slot = ((s >> 33) % (2 * ROW_SUM_BOUND as u64 + 1)) as i32 - ROW_SUM_BOUND;
    }
    let rows: [(&str, [i32; N]); 5] = [
        ("every coefficient at +6(q−1)", [ROW_SUM_BOUND; N]),
        ("every coefficient at −6(q−1)", [-ROW_SUM_BOUND; N]),
        (
            "alternating ±6(q−1)",
            std::array::from_fn(|i| {
                if i % 2 == 0 {
                    ROW_SUM_BOUND
                } else {
                    -ROW_SUM_BOUND
                }
            }),
        ),
        (
            "+6(q−1) in the first half, −6(q−1) in the second",
            std::array::from_fn(|i| {
                if i < N / 2 {
                    ROW_SUM_BOUND
                } else {
                    -ROW_SUM_BOUND
                }
            }),
        ),
        ("pseudo-random within ±6(q−1)", random),
    ];
    for (name, row) in rows {
        let want = normal_domain_in_contract(&row);
        let mut got = row;
        to_normal_domain(&mut got);
        let got: [i32; N] = std::array::from_fn(|i| got[i].rem_euclid(Q));
        assert_eq!(
            got, want,
            "{name}: the row returns to the normal domain exactly"
        );
    }
}

/// The domain `reduce32` and `freeze` are exact on: [−2³¹, 2³¹ − 2²² − 1]. Dense near zero (every
/// value a coefficient reaches), every value at both ends, and a stride across the rest.
fn reduction_domain() -> impl Iterator<Item = i32> {
    const TOP: i32 = i32::MAX - (1 << 22);
    (-(1 << 25)..=(1 << 25))
        .chain(i32::MIN..=i32::MIN + (1 << 16))
        .chain(TOP - (1 << 16)..=TOP)
        .chain((i32::MIN..=TOP).step_by(4093))
}

#[test]
fn reduce32_is_congruent_and_within_its_bound_over_its_domain() {
    for a in reduction_domain() {
        let r = reduce32(a);
        assert!(
            (-6_283_009..=6_283_008).contains(&r),
            "reduce32({a}) = {r} is outside [−6283009, 6283008]"
        );
        assert_eq!(
            r.rem_euclid(Q),
            a.rem_euclid(Q),
            "reduce32({a}) ≡ {a} (mod q)"
        );
    }
}

#[test]
fn freeze_is_the_canonical_representative_over_its_domain() {
    for a in reduction_domain() {
        assert_eq!(freeze(a), a.rem_euclid(Q), "freeze({a})");
    }
}

#[test]
fn caddq_lifts_every_representative_of_minus_q_to_q_into_zero_to_q() {
    for a in -(Q - 1)..Q {
        assert_eq!(caddq(a), a.rem_euclid(Q), "caddq({a})");
    }
}

#[test]
fn to_signed_is_the_centered_representative_over_its_domain() {
    for a in reduction_domain() {
        let r = a.rem_euclid(Q);
        let want = if r > Q / 2 { r - Q } else { r };
        assert_eq!(to_signed(a), want, "to_signed({a})");
    }
}

#[test]
fn the_compile_time_twiddle_table_is_zeta_to_the_bit_reversed_powers() {
    let mut pow = [1i64; 256];
    for i in 1..256 {
        pow[i] = pow[i - 1] * ZETA % Q as i64;
    }
    for (i, &z) in ZETAS.iter().enumerate() {
        let br = (i as u8).reverse_bits() as usize;
        let mut v = pow[br] * MONT % Q as i64;
        if v > Q as i64 / 2 {
            v -= Q as i64;
        }
        assert_eq!(z as i64, v, "zetas[{i}] = ζ^brv₈({i})·R mod q, centered");
    }
}

#[test]
fn the_matrix_entry_table_walks_the_rows_of_a_hat() {
    for (e, &(r, c)) in ENTRY_COORDS.iter().enumerate() {
        assert_eq!((r, c), (e / ML, e % ML), "entry {e} of the 6×5 Â");
    }
    assert_eq!(ENTRY_COORDS.len(), MK * ML);
}
