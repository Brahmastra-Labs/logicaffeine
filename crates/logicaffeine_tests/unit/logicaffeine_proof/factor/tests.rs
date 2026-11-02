use super::*;

fn big(s: &str) -> BigInt {
    BigInt::parse_decimal(s).expect("valid decimal")
}

#[test]
fn primality_and_next_prime_agree_with_known_values() {
    assert!(is_probable_prime(&BigInt::from_i64(2)));
    assert!(is_probable_prime(&BigInt::from_i64(1_000_003)));
    assert!(!is_probable_prime(&BigInt::from_i64(1_000_000)));
    assert!(!is_probable_prime(&big("1000000000000000000000000000000"))); // even
    assert_eq!(
        next_prime(&BigInt::from_i64(1_000_000)),
        BigInt::from_i64(1_000_003)
    );
}

#[test]
fn isqrt_is_exact() {
    assert_eq!(isqrt(&BigInt::from_i64(0)), BigInt::from_i64(0));
    assert_eq!(isqrt(&BigInt::from_i64(15)), BigInt::from_i64(3));
    assert_eq!(isqrt(&BigInt::from_i64(16)), BigInt::from_i64(4));
    let big_sq = big("1000000000000000000000000000000"); // 10³⁰ = (10¹⁵)²
    assert_eq!(isqrt(&big_sq), big("1000000000000000"));
}

#[test]
fn trial_division_catches_a_small_factor() {
    let p = next_prime(&big("1000000000000000000000000000000"));
    let n = BigInt::from_i64(3).mul(&p);
    let (a, b) = trial_division(&n, 100).expect("the small factor 3 is found");
    assert!(verify_factorization(&n, &a, &b));
    assert!(a == BigInt::from_i64(3) || b == BigInt::from_i64(3));
}

#[test]
fn fermat_crushes_close_primes() {
    let p = next_prime(&big("1000000000000000000000000000000"));
    let q = next_prime(&p.add(&BigInt::from_i64(2)));
    let n = p.mul(&q);
    let (a, b) = fermat(&n, 100_000).expect("adjacent primes fall to Fermat");
    assert!(verify_factorization(&n, &a, &b));
}

#[test]
fn pollard_rho_factors_a_moderate_semiprime() {
    let p = next_prime(&big("1000000007"));
    let q = next_prime(&big("2000000011"));
    let n = p.mul(&q);
    let (a, b) = pollard_rho(&n, 500_000).expect("rho factors a ~60-bit semiprime");
    assert!(verify_factorization(&n, &a, &b));
}

#[test]
fn pollard_p_minus_1_crushes_a_smooth_prime() {
    // Build p with p−1 = 37-smooth: p = (∏ primes ≤ 37)·k + 1, prime, k small.
    let mut smooth = one();
    for &pr in &[2u64, 3, 5, 7, 11, 13, 17, 19, 23, 29, 31, 37] {
        smooth = smooth.mul(&BigInt::from_u64(pr));
    }
    let mut p = smooth.add(&one());
    let mut k = 1u64;
    while !is_probable_prime(&p) {
        k += 1;
        assert!(
            k <= 40,
            "a smooth prime is found with a small (≤40-smooth) multiplier"
        );
        p = smooth.mul(&BigInt::from_u64(k)).add(&one());
    }
    let q = next_prime(&big("999999999999999999999999999999"));
    let n = p.mul(&q);
    let (a, b) = pollard_p_minus_1(&n, 40).expect("a smooth p−1 falls to Pollard p−1");
    assert!(verify_factorization(&n, &a, &b));
}

#[test]
fn wiener_crushes_a_small_private_exponent() {
    let p = next_prime(&big("1000000007"));
    let q = next_prime(&big("3000000019"));
    let n = p.mul(&q);
    let phi = p.sub(&one()).mul(&q.sub(&one()));
    let d = BigInt::from_i64(7919); // small: 7919 < ⅓·N^{1/4}
    let e = mod_inverse(&d, &phi).expect("d is coprime to φ");
    let (a, b) = wiener(&e, &n).expect("a small private exponent falls to Wiener");
    assert!(verify_factorization(&n, &a, &b));
}

#[test]
fn batch_gcd_crushes_a_shared_prime() {
    let p = next_prime(&big("1000000000000000000000000000000"));
    let q1 = next_prime(&big("3000000000000000000000000000000"));
    let q2 = next_prime(&big("7000000000000000000000000000000"));
    let (n1, n2) = (p.mul(&q1), p.mul(&q2));
    let hits = batch_gcd(&[n1, n2]);
    assert_eq!(hits.len(), 1, "the shared prime is detected");
    assert_eq!(hits[0].2, p, "and it is exactly p");
}

#[test]
fn structural_factor_orchestrates_the_arsenal() {
    // A close-primes modulus is caught, and the method is reported.
    let p = next_prime(&big("1000000000000000000000000000000"));
    let q = next_prime(&p.add(&BigInt::from_i64(2)));
    let n = p.mul(&q);
    let w = structural_factor(&n, StructuralBudget::default()).expect("weakness found");
    assert!(verify_factorization(&n, &w.p, &w.q));
    assert_eq!(w.method, "Fermat (close primes)");
}

#[test]
fn a_soundly_generated_modulus_resists_the_entire_structural_arsenal() {
    // Two large, independent, well-separated strong primes: not close (Fermat fails), p−1/q−1 are
    // not smooth (Pollard p−1 fails), no shared prime, and N is far too large for bounded rho. Every
    // STRUCTURAL attack, run to its budget, finds NOTHING — RSA's safety IS this ceiling, the
    // number-theoretic incompressible residue. Only general sub-exponential factoring remains, which
    // real key sizes push out of reach.
    let p = next_prime(&big("1000000000000000000000000000057"));
    let q = next_prime(&big("9000000000000000000000000000000"));
    let n = p.mul(&q);
    assert!(
        structural_factor(&n, StructuralBudget::default()).is_none(),
        "a sound modulus has no structural shortcut — the ceiling stands"
    );
}

#[test]
fn resultant_machinery_eliminates_a_variable() {
    // Fraction-free determinant: |[[1,2],[3,4]]| = −2.
    let det = det_bareiss(vec![
        vec![BigInt::from_i64(1), BigInt::from_i64(2)],
        vec![BigInt::from_i64(3), BigInt::from_i64(4)],
    ]);
    assert_eq!(det, BigInt::from_i64(-2));

    // Eliminate x from g1 = x − y and g2 = x + y − 2 (common root (1, 1)). The resultant is a
    // polynomial in y that must vanish exactly at the shared coordinate y = 1.
    let mut g1 = BPoly::new();
    g1.insert((1, 0), BigInt::from_i64(1));
    g1.insert((0, 1), BigInt::from_i64(-1));
    let mut g2 = BPoly::new();
    g2.insert((1, 0), BigInt::from_i64(1));
    g2.insert((0, 1), BigInt::from_i64(1));
    g2.insert((0, 0), BigInt::from_i64(-2));
    let res = bivariate_resultant_x(&g1, &g2);
    assert!(
        poly_eval(&res, &BigInt::from_i64(1)).is_zero(),
        "resultant vanishes at the shared y = 1"
    );
    assert!(
        !poly_eval(&res, &BigInt::from_i64(0)).is_zero(),
        "and is nonzero away from it"
    );
}

#[test]
#[ignore] // heavy (dim ~72 fpLLL) — run explicitly in release
fn boneh_durfee_breaks_small_d() {
    let p = next_prime(&big("1000003"));
    let q = next_prime(&big("2000003"));
    let n = p.mul(&q);
    let phi = p.sub(&one()).mul(&q.sub(&one()));
    let d = BigInt::from_i64(1423); // δ ≈ 0.255 — past Wiener's 0.25
    let e = mod_inverse(&d, &phi).unwrap();
    assert!(wiener(&e, &n).is_none(), "beyond Wiener (control)");
    let x_bound = BigInt::from_i64(1 << 11);
    let (a, b) = boneh_durfee(&n, &e, 8, 3, &x_bound).expect("Boneh-Durfee breaks small-d");
    eprintln!("SMALL-d BROKEN: {a:?} · {b:?}");
    assert!(
        verify_factorization(&n, &a, &b),
        "the recovered factors check out"
    );
}

#[test]
fn boneh_durfee_lattice_lifts_the_modular_root_to_the_integers() {
    // The verified core of Boneh-Durfee: for a small-d key, build the bivariate lattice of x/y-shifts
    // of f(x,y) = x·y + (N+1)·x + 1 and LLL-reduce it (fast float-Gram-Schmidt). The short polynomials
    // then vanish at the true root (k, −(p+q)) OVER THE INTEGERS — Howgrave-Graham — which is the deep
    // step that lifts "root mod e" to a solvable integer system. Full factor recovery additionally
    // needs two ALGEBRAICALLY-INDEPENDENT vanishing vectors for the resultant elimination; producing
    // them reliably is Boneh-Durfee's own geometrically-progressive sublattice (the documented
    // remaining refinement — in the low-δ regime here the vanishing sublattice collapses to a single
    // dependent family, so the resultant degenerates).
    let p = next_prime(&big("100003"));
    let q = next_prime(&big("200003"));
    let n = p.mul(&q);
    let phi = p.sub(&one()).mul(&q.sub(&one()));
    let d = BigInt::from_i64(43);
    let e = mod_inverse(&d, &phi).unwrap();
    let k = e.mul(&d).sub(&one()).div_rem(&phi).unwrap().0;
    let s = p.add(&q);
    let (polys, _) = bd_reduced_polys(&n, &e, 4, 2, &BigInt::from_i64(1 << 8));
    let vanishing = polys
        .iter()
        .filter(|g| g.len() > 1 && bpoly_eval(g, &k, &s.negated()).is_zero())
        .count();
    assert!(
        vanishing >= 5,
        "the reduced lattice lifts the modular root to integer roots, got {vanishing}"
    );
}

#[test]
fn quadratic_sieve_factors_a_larger_semiprime_by_sieving() {
    // A ~34-bit semiprime — beyond comfortable Dixon range — factored by log-sieving Q(x) and the
    // GF(2) dependency search.
    let p = next_prime(&big("100000"));
    let q = next_prime(&big("100050"));
    let n = p.mul(&q);
    let (a, b) = quadratic_sieve(&n, 500, 50_000).expect("the quadratic sieve factors N");
    assert!(
        verify_factorization(&n, &a, &b),
        "sieving + a GF(2) dependency split N"
    );
}

#[test]
fn dixon_breaks_a_semiprime_with_our_gf2_symmetry_breaking() {
    // N = 179 · 257. Dixon collects r with r² mod N smooth over a small prime base, then OUR GF(2)
    // dependency finder picks a subset whose product is a perfect square → x² ≡ y² (mod N) → factor.
    let n = BigInt::from_i64(179).mul(&BigInt::from_i64(257));
    let base = [2u64, 3, 5, 7, 11, 13, 17, 19, 23, 29, 31];
    let (a, b) = dixon_factor(&n, &base, 50_000).expect("Dixon factors via a GF(2) dependency");
    assert!(
        verify_factorization(&n, &a, &b),
        "the congruence of squares splits N"
    );
}

#[test]
fn crt_and_nth_root_are_correct() {
    // Sunzi's classic: x ≡ 2 (mod 3), 3 (mod 5), 2 (mod 7) → 23.
    let x = crt(
        &[
            BigInt::from_i64(2),
            BigInt::from_i64(3),
            BigInt::from_i64(2),
        ],
        &[
            BigInt::from_i64(3),
            BigInt::from_i64(5),
            BigInt::from_i64(7),
        ],
    )
    .unwrap();
    assert_eq!(x, BigInt::from_i64(23));
    assert_eq!(
        integer_nth_root(&BigInt::from_i64(27), 3),
        BigInt::from_i64(3)
    );
    assert_eq!(
        integer_nth_root(&big("1000000000000000000"), 3),
        BigInt::from_i64(1000000)
    );
    assert_eq!(
        integer_nth_root(&BigInt::from_i64(3125), 5),
        BigInt::from_i64(5)
    );
    assert_eq!(
        integer_nth_root(&BigInt::from_i64(26), 3),
        BigInt::from_i64(2),
        "floor cbrt(26) = 2"
    );
}

#[test]
fn hastad_broadcast_recovers_the_message() {
    // The same plaintext, encrypted under e=3 to three recipients with distinct moduli, all > m.
    let e = 3u32;
    let m = big("123456789");
    let moduli: Vec<BigInt> = vec![
        next_prime(&big("100000007")).mul(&next_prime(&big("200000033"))),
        next_prime(&big("300000007")).mul(&next_prime(&big("400000009"))),
        next_prime(&big("500000003")).mul(&next_prime(&big("600000011"))),
    ];
    let e_big = BigInt::from_u64(e as u64);
    let ciphertexts: Vec<BigInt> = moduli.iter().map(|n| modpow(&m, &e_big, n)).collect();
    let recovered = hastad_broadcast_attack(e, &ciphertexts, &moduli).expect("Håstad recovers m");
    assert_eq!(
        recovered, m,
        "the broadcast plaintext falls out of the ciphertexts alone"
    );
}

#[test]
fn common_modulus_recovers_the_message() {
    // The same message encrypted under the SAME modulus with two coprime public exponents.
    let n = next_prime(&big("1000000007")).mul(&next_prime(&big("2000000011")));
    let m = big("31415926535");
    let (e1, e2) = (BigInt::from_i64(65537), BigInt::from_i64(3));
    let c1 = modpow(&m, &e1, &n);
    let c2 = modpow(&m, &e2, &n);
    let recovered =
        common_modulus_attack(&e1, &e2, &c1, &c2, &n).expect("common modulus recovers m");
    assert_eq!(
        recovered, m,
        "a shared modulus + coprime exponents leaks the message via Bézout"
    );
}

#[test]
fn franklin_reiter_recovers_related_messages() {
    // Two linearly related messages m and m+r under the same N and e=3.
    let n = next_prime(&big("1000000007")).mul(&next_prime(&big("2000000011")));
    let e = 3u32;
    let e_big = BigInt::from_u64(e as u64);
    let m = big("424242424242");
    let r = big("31337");
    let c1 = modpow(&m, &e_big, &n);
    let c2 = modpow(&m.add(&r), &e_big, &n);
    let recovered =
        franklin_reiter_attack(e, &r, &c1, &c2, &n).expect("Franklin-Reiter recovers m");
    assert_eq!(
        recovered, m,
        "the related pair leaks the plaintext via polynomial GCD"
    );
}

#[test]
fn low_exponent_no_padding_recovers_small_message() {
    // A small message under e=3 with no padding: c = m³ over the integers (no reduction).
    let n = next_prime(&big("100000000000000003")).mul(&next_prime(&big("200000000000000003")));
    let m = big("123456");
    let c = modpow(&m, &BigInt::from_i64(3), &n);
    let recovered = low_exponent_message(&c, 3).expect("cube root recovers the small message");
    assert_eq!(
        recovered, m,
        "no padding under a small exponent is just an integer root"
    );
}

#[test]
fn coppersmith_factors_from_known_high_bits() {
    // A ~129-bit modulus. Reveal all but the low 20 bits of p — a partial-key-exposure leak that no
    // factoring shortcut (Fermat/rho/p−1) exploits. The dim-9 Coppersmith lattice (fast now via
    // float-Gram-Schmidt LLL) recovers the missing bits and factors N.
    let p = next_prime(&big("18446744073709551629"));
    let q = next_prime(&big("36893488147419103237"));
    let n = p.mul(&q);
    let unknown_bits = 20u32;
    let mut mask = one();
    for _ in 0..unknown_bits {
        mask = mask.mul(&two());
    }
    let (p_div, _) = p.div_rem(&mask).expect("nonzero");
    let p_high = p_div.mul(&mask); // p with its low 18 bits zeroed
    let (a, b) =
        coppersmith_factor_high_bits(&n, &p_high, unknown_bits).expect("Coppersmith recovers p");
    assert!(
        verify_factorization(&n, &a, &b),
        "the recovered factorization checks out"
    );
}

#[test]
fn rsa_breaks_end_to_end_when_the_modulus_factors() {
    // A weak RSA key: two primes chosen close together.
    let p = next_prime(&big("1000000000000000000000000000057"));
    let q = next_prime(&p.add(&BigInt::from_i64(100)));
    let n = p.mul(&q);
    let e = BigInt::from_i64(65537);
    let d = rsa_private_exponent(&e, &p, &q).expect("e is coprime to φ");
    let m = big("123456789987654321");
    let c = modpow(&m, &e, &n);
    assert_eq!(modpow(&c, &d, &n), m, "RSA encryption round-trips");

    // Factor the modulus (Fermat crushes the close primes), recover d from the factors, decrypt: a
    // complete break with no access to the private key.
    let w = structural_factor(&n, StructuralBudget::default()).expect("close primes factor");
    let d_broken = rsa_private_exponent(&e, &w.p, &w.q).expect("recover d from the factors");
    assert_eq!(
        modpow(&c, &d_broken, &n),
        m,
        "the recovered key decrypts — RSA broken end to end"
    );
}

#[test]
fn recovering_the_private_key_is_equivalent_to_factoring() {
    // A sound RSA key: two large, well-separated primes.
    let p = next_prime(&big("1000000000000000000000000000057"));
    let q = next_prime(&big("9000000000000000000000000000000"));
    let n = p.mul(&q);
    let e = BigInt::from_i64(65537);
    let d = rsa_private_exponent(&e, &p, &q).expect("e is coprime to φ");

    // The attacker's side: the structural arsenal cannot factor it — no shortcut to the key.
    assert!(
        structural_factor(&n, StructuralBudget::default()).is_none(),
        "the sound modulus resists every structural attack"
    );

    // The equivalence: yet the private exponent d, if known, DETERMINISTICALLY factors N. So
    // recovering the key and factoring the modulus are one and the same problem — RSA's security
    // reduces exactly to factoring, which the arsenal cannot shortcut and which (by Chaitin) we
    // cannot prove hard. It neither provably breaks nor is provably unbreakable.
    let (fp, fq) = factor_via_private_exponent(&n, &e, &d).expect("the private exponent factors N");
    assert!(
        verify_factorization(&n, &fp, &fq),
        "recovering d recovers the factorization"
    );
}
