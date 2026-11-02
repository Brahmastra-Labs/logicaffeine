use super::*;

fn i(x: i64) -> BigInt {
    BigInt::from_i64(x)
}

#[test]
fn torsion_image_witness_verifies_and_rejects_tampering() {
    use crate::elliptic::point_of_order;
    // The curve y² = x³ + 2x + 25 over 𝔽₄₃ has #E = 45 = 9·5: full rational 3-torsion (the basis) and a
    // rational 5-torsion point (the isogeny kernel; 5 is coprime to 3). μ₃ ⊂ 𝔽₄₃ (3 | 42), p ≡ 3 mod 4.
    let curve = Curve::new(i(2), i(25), i(43));
    let kernel = point_of_order(&curve, 5).expect("𝔽₄₃ 5-torsion");
    let w = certify_isogeny(&curve, &kernel, 5, 3).expect("a genuine isogeny witness");

    // A real witness satisfies the Weil-pairing compatibility law.
    assert!(
        w.verify(),
        "e_N(φP,φQ) = e_N(P,Q)^{{deg φ}} for a genuine isogeny"
    );
    assert_eq!(audit(&w), SidhAudit::ConsistentButTorsionExposed);

    // The numeric law, spelled out: e_3(φP,φQ) = e_3(P,Q)^5.
    let ep = weil_pairing(&w.domain, &w.basis.0, &w.basis.1, 3).unwrap();
    let eq = weil_pairing(&w.codomain, &w.images.0, &w.images.1, 3).unwrap();
    assert_eq!(
        eq,
        modpow(&ep, &i(5), &w.domain.p),
        "the compatibility law holds numerically"
    );

    // Tampering an image breaks the law — the certificate is only as good as its re-check.
    let mut bad = w.clone();
    bad.images.0 = w.codomain.double(&w.images.0); // 2·φ(P) ≠ φ(P)
    assert!(
        !bad.verify(),
        "a tampered torsion image fails the pairing law"
    );
    assert_eq!(audit(&bad), SidhAudit::Inconsistent);
}

#[test]
fn kani_glue_is_lagrangian_exactly_when_deg_is_minus_one_mod_n() {
    // Over 𝔽_{59²} on the supersingular curve y²=x³+x (p+1 = 60 = 2²·3·5), both E[3] and E[5] are
    // fully rational — enough to glue.
    let p = BigInt::parse_decimal("59").unwrap();
    let c = Curve2::new(fp2_const(1, &p), fp2_const(0, &p), p.clone());

    // deg φ = 5, torsion N = 3: 5 ≡ −1 (mod 3), so the graph {(T,φT)} is Lagrangian — an (N,N)-isogeny of
    // the surface E×E' exists. This is the gluing condition Kani's theorem supplies.
    let k5 = point_of_order2(&c, 5).expect("5-torsion");
    let glue = build_kani_glue(&c, &k5, 5, 3).expect("a genuine glue kernel");
    assert!(
        glue.verify(),
        "the gluing relation e_product = e_N^{{1+deg}} holds"
    );
    assert!(
        glue.is_lagrangian(),
        "deg 5 ≡ −1 (mod 3) ⟹ Lagrangian glue kernel"
    );
    assert_eq!(
        glue.glue_pairing().unwrap(),
        fp2_const(1, &p),
        "isotropic: the product pairing is trivial"
    );

    // deg φ = 3, torsion N = 5: 3 ≢ −1 (mod 5), so NOT Lagrangian — yet the relation still re-checks.
    let k3 = point_of_order2(&c, 3).expect("3-torsion");
    let glue2 = build_kani_glue(&c, &k3, 3, 5).expect("a genuine glue kernel");
    assert!(
        glue2.verify(),
        "the gluing relation holds regardless of the degree"
    );
    assert!(
        !glue2.is_lagrangian(),
        "deg 3 ≢ −1 (mod 5) ⟹ not Lagrangian"
    );
}

#[test]
fn images_to_generator_recovery_certifies_itself_and_rejects_tampering() {
    // SIDH-scale instance over 𝔽_{107²}: y²=x³+x (j=1728), #E=(p+1)²=108², so E[3³] and E[2²] are both
    // rank-2 rational — a genuine 27-kernel keyspace.
    let p = BigInt::parse_decimal("107").unwrap();
    let e0 = Curve2::new(fp2_const(1, &p), fp2_const(0, &p), p.clone());
    let (pa, qa) = full_order_basis2(&e0, 3, 3).expect("rank-2 E[3³] basis");
    let (pb, qb) = full_order_basis2(&e0, 2, 2).expect("rank-2 E[2²] basis");

    // A secret isogeny publishes its torsion images.
    let secret = i(13);
    let gen = kernel_generator2(&e0, &pa, &qa, &secret);
    let path = derive_isogeny_path2(&e0, &gen, 3, 3).expect("the secret 3³-isogeny");
    let images = (
        push_through2(&path, 3, &pb).unwrap(),
        push_through2(&path, 3, &qb).unwrap(),
    );

    // Invert AND auto-formalize: recover a generator reproducing the images, and pop out the laws.
    let rec = certify_recovery(&e0, (&pa, &qa), 3, 3, (&pb, &qb), (&images.0, &images.1))
        .expect("images → generator recovery");
    assert_eq!(
        push_through2(&rec.path, 3, &pb).unwrap(),
        images.0,
        "the recovered isogeny reproduces φ(P_B)"
    );
    assert_eq!(
        push_through2(&rec.path, 3, &qb).unwrap(),
        images.1,
        "and φ(Q_B) — inversion succeeded"
    );
    assert!(
        rec.verify(),
        "every law the recovery invoked re-checks from scratch"
    );
    assert_eq!(rec.laws.len(), 5, "the full law set is emitted");
    assert!(
        rec.learnings().iter().any(|l| l.contains("ι(K)")),
        "the Aut-orbit rule is among the learnings"
    );
    // The recovered generator matches the reported scalar (internal consistency of the certificate).
    assert_eq!(kernel_generator2(&e0, &pa, &qa, &rec.secret), rec.generator);

    // The certificate is only as good as its re-check: tampering the secret must fail verification.
    let _ = secret; // the planted secret is a valid solution; recovery may return a symmetry-equivalent one
    let mut forged = rec.clone();
    forged.secret = rec.secret.add(&i(1)); // a different scalar ⟹ a different generator ⟹ rejected
    assert!(
        !forged.verify(),
        "a tampered secret no longer matches its generator ⟹ rejected"
    );
}

#[test]
fn kani_degree_diamond_is_well_formed() {
    // The degree bookkeeping: to attack a degree-3 isogeny, pair it with an auxiliary of degree
    // c = 2^e − 3; the smallest smooth choice is c = 5 (3 + 5 = 8 = 2³).
    assert_eq!(
        kani_diamond_degrees(3, 8, 100),
        Some((3, 5)),
        "3 + 5 = 2³ — smallest smooth diamond"
    );
    assert!(
        is_smooth(5, 100) && is_smooth(8, 3) && !is_smooth(101, 50),
        "smoothness gate"
    );

    // A real diamond over 𝔽_{59²}: φ a 3-isogeny, γ a 5-isogeny, 3 + 5 = 8 = 2³ (p+1 = 60 = 2²·3·5, so
    // both 3- and 5-torsion are rational).
    let p = BigInt::parse_decimal("59").unwrap();
    let e0 = Curve2::new(fp2_const(1, &p), fp2_const(0, &p), p.clone());
    let k3 = point_of_order2(&e0, 3).expect("3-torsion");
    let k5 = point_of_order2(&e0, 5).expect("5-torsion");
    let diamond = build_kani_diamond(&e0, &k3, 3, 1, &k5, 5).expect("a well-formed Kani diamond");

    assert_eq!(
        (diamond.d, diamond.c, diamond.e),
        (3, 5, 3),
        "degrees close to a power of two"
    );
    assert_eq!(
        diamond.d + diamond.c,
        1 << diamond.e,
        "c + d = 2^e — the Kani degree condition"
    );
    // Both isogeny sides are genuine curves (real codomains), and the diamond is nontrivial.
    assert!(
        diamond.e_curve.j_invariant().is_some(),
        "φ lands on a real curve E"
    );
    assert!(
        diamond.c_curve.j_invariant().is_some(),
        "γ lands on a real curve C"
    );
    // A mismatched auxiliary degree (c + d not a power of two) is correctly rejected.
    assert!(build_kani_diamond(&e0, &k3, 3, 1, &k5, 5).is_some());
    assert!(
        build_kani_diamond(&e0, &k3, 3, 1, &k3, 3).is_none(),
        "3 + 3 = 6 is not a power of two"
    );
}
