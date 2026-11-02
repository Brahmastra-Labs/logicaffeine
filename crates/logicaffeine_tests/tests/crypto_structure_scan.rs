//! # Cryptanalytic bench: structure-vs-compression as the "answer falls out" instrument
//!
//! Part of the multivariate-cryptography structure study. This
//! is a *first-principles study*, not an attack: we point the existing structure/compression tools at
//! real cryptographic objects and record which verdict falls out — **weakness** (a short description
//! exists) or **incompressible-in-class** (no exploitable structure to this lens).
//!
//! The thesis: structure always exists at finite scale, so we do not *search* for it — the right lens
//! extracts it. `ait::assess_key_material` is that lens for byte material: it returns
//! [`CryptoStrength::Weak`] with a re-checkable description when the data compresses, and
//! [`CryptoStrength::IncompressibleInClass`] when nothing in the generator menu beats storing it raw.
//!
//! **Honest bound (stated on purpose):** byte-incompressibility is *necessary, not sufficient* for
//! cryptographic hardness. A modulus with a small factor is byte-incompressible yet factoring-weak —
//! so the compression lens and the factoring lens are *different tools for different structure*. The
//! final study makes that explicit: the answer falls out of the **right** tool, and no single lens is
//! the whole toolbox.

use logicaffeine_base::BigInt;
use logicaffeine_proof::ait::{assess_key_material, classify_bytes, CryptoStrength};
use logicaffeine_proof::factor::{next_prime, trial_division, verify_factorization};

// ── tiny deterministic helpers (no external randomness) ─────────────────────────────────

/// SplitMix64 — a high-quality PRNG; its byte stream carries no structure the MDL menu can see.
struct SplitMix(u64);
impl SplitMix {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

/// `n` cryptographic-quality pseudorandom bytes.
fn random_bytes(seed: u64, n: usize) -> Vec<u8> {
    let mut s = SplitMix(seed);
    let mut v = Vec::with_capacity(n + 8);
    while v.len() < n {
        v.extend_from_slice(&s.next().to_le_bytes());
    }
    v.truncate(n);
    v
}

/// A short-period keystream — the archetypal weak PRNG (a small cycle repeated).
fn short_period(pattern: &[u8], n: usize) -> Vec<u8> {
    (0..n).map(|i| pattern[i % pattern.len()]).collect()
}

/// A modular-affine sequence `b[i] = (a·i + c) mod 256` — structure the affine/periodic lens catches.
fn modular_affine(a: u32, c: u32, n: usize) -> Vec<u8> {
    (0..n)
        .map(|i| ((a as usize * i + c as usize) & 0xff) as u8)
        .collect()
}

/// `base^e` in `BigInt`.
fn pow(base: &BigInt, e: usize) -> BigInt {
    let mut r = BigInt::from_i64(1);
    for _ in 0..e {
        r = r.mul(base);
    }
    r
}

/// A prime near `base^e` (≈ 30·e bits).
fn prime_near(base_seed: i64, e: usize) -> BigInt {
    next_prime(&pow(&BigInt::from_i64(base_seed), e))
}

fn magnitude_bytes(n: &BigInt) -> Vec<u8> {
    n.to_le_bytes().1
}

// ── STUDY 1: structure falls out of the compression lens ────────────────────────────────

/// The instrument cleanly separates *structured* byte material (a description falls out → WEAK) from
/// *cryptographic-random* byte material (nothing beats raw → INCOMPRESSIBLE). Robust to absurdity:
/// three independent structured families, all caught; one PRNG, provably not.
#[test]
fn structure_falls_out_and_randomness_certifies_incompressible() {
    let len = 1024;

    let weak_cases: [(&str, Vec<u8>); 3] = [
        (
            "short-period (4-byte cycle)",
            short_period(&[0xAA, 0xBB, 0xCC, 0xDD], len),
        ),
        ("constant run (RLE)", short_period(&[0x42], len)),
        ("modular-affine a=7,c=3", modular_affine(7, 3, len)),
    ];

    let mut worst_weak_ratio = 0.0f64;
    for (name, data) in &weak_cases {
        let ratio = classify_bytes(data).ratio;
        let verdict = assess_key_material(data);
        println!(
            "WEAK  {name:<28} ratio={ratio:.4}  verdict={}",
            tag(&verdict)
        );
        assert!(
            ratio < 0.6,
            "{name}: a structured stream must compress well (ratio={ratio:.4})"
        );
        assert!(
            matches!(verdict, CryptoStrength::Weak { .. }),
            "{name}: the lens must expose a certified weakness"
        );
        worst_weak_ratio = worst_weak_ratio.max(ratio);
    }

    // Cryptographic-quality randomness: no lens fires → incompressible-in-class.
    let rnd = random_bytes(0xD1D_C0FFEE, len);
    let rnd_ratio = classify_bytes(&rnd).ratio;
    let rnd_verdict = assess_key_material(&rnd);
    println!(
        "RAND  splitmix64                  ratio={rnd_ratio:.4}  verdict={}",
        tag(&rnd_verdict)
    );
    assert!(
        rnd_ratio > 0.85,
        "cryptographic randomness must not compress (ratio={rnd_ratio:.4})"
    );
    assert!(
        matches!(rnd_verdict, CryptoStrength::IncompressibleInClass { .. }),
        "randomness must certify incompressible-in-class"
    );

    // The separation is the whole point: every weak family sits well below the random floor.
    assert!(
        worst_weak_ratio < rnd_ratio,
        "the instrument must separate structure ({worst_weak_ratio:.4}) from randomness ({rnd_ratio:.4})"
    );
}

// ── STUDY 2: a well-formed modulus certifies incompressible-in-class ────────────────────

/// A balanced semiprime `N = p·q` (~480-bit) shows NO representational structure to the compression
/// lens — a certified *necessary* hardness signature. We assert incompressible AND record the ratio.
/// (Necessary, not sufficient: see Study 3.)
#[test]
fn a_well_formed_modulus_is_incompressible_in_class() {
    let p = prime_near(1_000_000_007, 8);
    let q = prime_near(1_000_003, 9); // different seed → distinct, well-separated primes
    let n = p.mul(&q);
    assert!(verify_factorization(&n, &p, &q), "sanity: N = p·q");

    let bytes = magnitude_bytes(&n);
    let ratio = classify_bytes(&bytes).ratio;
    let verdict = assess_key_material(&bytes);
    println!(
        "MODULUS balanced semiprime ({} bytes)  ratio={ratio:.4}  verdict={}",
        bytes.len(),
        tag(&verdict)
    );
    assert!(
        ratio > 0.75,
        "a well-formed modulus must be byte-incompressible (necessary hardness signature); ratio={ratio:.4}"
    );
    assert!(matches!(
        verdict,
        CryptoStrength::IncompressibleInClass { .. }
    ));
}

// ── STUDY 3: the answer falls out of the RIGHT tool (compression ≠ factoring) ────────────

/// The honest boundary made executable. A modulus with a small factor is **byte-incompressible**
/// (the compression lens says "strong") yet **factoring-weak** (the factoring lens splits it in
/// microseconds). Different structure needs a different tool — that is the toolbox thesis: the answer
/// falls out of the *right* lens, and incompressibility alone never certifies hardness.
#[test]
fn compression_and_factoring_are_complementary_lenses() {
    let big = prime_near(1_000_000_007, 9); // ~270-bit prime
    let small = BigInt::from_i64(101);
    let weak_modulus = small.mul(&big);

    // Compression lens: sees no byte structure — would call it "incompressible".
    let bytes = magnitude_bytes(&weak_modulus);
    let comp_ratio = classify_bytes(&bytes).ratio;
    println!(
        "SMALL-FACTOR modulus  compression-ratio={comp_ratio:.4} (looks strong to compression)"
    );
    assert!(
        comp_ratio > 0.6,
        "the small-factor modulus is still byte-incompressible"
    );

    // Factoring lens: the RIGHT tool splits it instantly.
    let split =
        trial_division(&weak_modulus, 1_000).expect("factoring lens finds the small factor");
    println!(
        "SMALL-FACTOR modulus  factoring-lens split = {} · {}",
        split.0, split.1
    );
    assert!(
        split.0 == small || split.1 == small,
        "the factoring lens recovers the planted small factor"
    );
    assert!(verify_factorization(&weak_modulus, &split.0, &split.1));

    // A balanced semiprime resists the factoring lens at the same budget — no small structure to find.
    let p = prime_near(1_000_000_007, 8);
    let q = prime_near(999_983, 8);
    let balanced = p.mul(&q);
    assert!(
        trial_division(&balanced, 100_000).is_none(),
        "a balanced semiprime has no small factor for the trial-division lens to catch"
    );
    println!(
        "BALANCED  semiprime   factoring-lens (limit 1e5) = no split  (right tool, no structure)"
    );
}

/// Short human tag for a [`CryptoStrength`] verdict.
fn tag(s: &CryptoStrength) -> String {
    match s {
        CryptoStrength::Weak { ratio, .. } => format!("WEAK(ratio={ratio:.4})"),
        CryptoStrength::IncompressibleInClass { ratio } => {
            format!("INCOMPRESSIBLE(ratio={ratio:.4})")
        }
    }
}
