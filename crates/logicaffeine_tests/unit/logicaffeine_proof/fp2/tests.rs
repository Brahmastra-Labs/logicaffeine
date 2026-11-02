use super::*;

fn big(s: &str) -> BigInt {
    BigInt::parse_decimal(s).unwrap()
}

#[test]
fn fp2_is_a_field() {
    let p = big("103"); // 103 ≡ 3 (mod 4)
    let one = fp2_const(1, &p);
    let ii = (ib(0), ib(1)); // the element i
                             // i² = −1.
    assert_eq!(fp2_mul(&ii, &ii, &p), fp2_neg(&one, &p), "i² = −1");
    // Commutativity, distributivity, and inverses over a sweep.
    let elts: Vec<Fp2> = (0..7)
        .flat_map(|a| (0..7).map(move |b| (ib(a * 13 + 1), ib(b * 11 + 2))))
        .collect();
    for x in &elts {
        for y in &elts {
            assert_eq!(fp2_mul(x, y, &p), fp2_mul(y, x, &p), "× commutes");
            assert_eq!(fp2_add(x, y, &p), fp2_add(y, x, &p), "+ commutes");
            // x·(y+1) = x·y + x
            let lhs = fp2_mul(x, &fp2_add(y, &one, &p), &p);
            let rhs = fp2_add(&fp2_mul(x, y, &p), x, &p);
            assert_eq!(lhs, rhs, "distributive");
        }
        if !fp2_is_zero(x) {
            assert_eq!(fp2_mul(x, &fp2_inv(x, &p).unwrap(), &p), one, "x·x⁻¹ = 1");
        }
    }
}

// The supersingular mass: #supersingular j-invariants = ⌊p/12⌋ + ε, ε from p mod 12.
fn ss_mass(p: u64) -> u64 {
    p / 12
        + match p % 12 {
            1 => 0,
            5 | 7 => 1,
            11 => 2,
            _ => 0,
        }
}

#[test]
fn supersingular_graph_has_the_right_mass_is_3_regular_and_connected() {
    for &pu in &[11u64, 19, 23, 31, 43, 47] {
        let p = big(&pu.to_string());
        let g = supersingular_graph(&p).expect("p ≡ 3 (mod 4)");
        // Mass: BFS reached every supersingular j-invariant (the graph is connected, so this proves
        // connectivity too — a disconnected graph would under-count).
        assert_eq!(
            g.len() as u64,
            ss_mass(pu),
            "supersingular mass for p = {pu}"
        );
        // 3-regular: Φ₂ is a cubic and splits completely over 𝔽_{p²} at every supersingular vertex.
        for v in &g {
            assert_eq!(
                v.neighbors.len(),
                3,
                "each vertex has out-degree 3 (p = {pu})"
            );
        }
        // The graph is undirected/consistent: every neighbour is itself a vertex in the graph.
        let js: Vec<&Fp2> = g.iter().map(|v| &v.j).collect();
        for v in &g {
            for nb in &v.neighbors {
                assert!(
                    js.iter().any(|j| *j == nb),
                    "neighbour is a supersingular vertex"
                );
            }
        }
        // j = 1728 is in the graph (our supersingular starting point).
        assert!(
            js.iter().any(|j| **j == fp2_const(1728, &p)),
            "1728 is supersingular for p = {pu}"
        );
    }
}

fn embed(pt: &crate::elliptic::Point) -> Point2 {
    match pt {
        crate::elliptic::Point::Infinity => Point2::Infinity,
        crate::elliptic::Point::Affine(x, y) => {
            Point2::Affine((x.clone(), ib(0)), (y.clone(), ib(0)))
        }
    }
}

#[test]
fn curve2_agrees_with_the_prime_field_on_rational_points() {
    // The 𝔽_{p²} group law, restricted to 𝔽_p-rational points, must reproduce the prime-field layer.
    let p = big("43");
    let cfp = crate::elliptic::Curve::new(ib(2), ib(25), p.clone());
    let base = crate::elliptic::point_of_order(&cfp, 5).expect("a rational 5-torsion point");
    let c2 = Curve2::new(fp2_const(2, &p), fp2_const(25, &p), p.clone());
    let base2 = embed(&base);
    assert!(
        c2.is_on_curve(&base2),
        "the embedded point lies on the 𝔽_{{p²}} curve"
    );
    for k in 1..=5i64 {
        let via_fp = embed(&cfp.mul(&ib(k), &base));
        let via_fp2 = c2.mul(&ib(k), &base2);
        assert_eq!(via_fp2, via_fp, "Curve2 matches Curve on {k}·P");
    }
}

// Every affine point of a curve over 𝔽_{p²} (brute force; small p only).
fn all_points2(c: &Curve2) -> Vec<Point2> {
    let pu = c.p.to_i64().unwrap();
    let mut v = vec![Point2::Infinity];
    for xa in 0..pu {
        for xb in 0..pu {
            let x = (ib(xa), ib(xb));
            for ya in 0..pu {
                for yb in 0..pu {
                    let pt = Point2::Affine(x.clone(), (ib(ya), ib(yb)));
                    if c.is_on_curve(&pt) {
                        v.push(pt);
                    }
                }
            }
        }
    }
    v
}

#[test]
fn supersingular_curve_over_fp2_carries_full_torsion() {
    // y² = x³ + x is supersingular for p ≡ 3 (mod 4); over 𝔽_{p²} it has #E = (p+1)² and E ≅ (ℤ/(p+1))²,
    // so its full ℓ-torsion (ℓ | p+1) is 𝔽_{p²}-rational — the property SIKE's torsion basis needs.
    let p = big("11");
    let c = Curve2::new(fp2_const(1, &p), fp2_const(0, &p), p.clone());
    assert_eq!(
        c.j_invariant().unwrap(),
        fp2_const(1728, &p),
        "j(y²=x³+x) = 1728, a supersingular vertex"
    );
    let pts = all_points2(&c);
    assert_eq!(pts.len(), 144, "#E(𝔽_{{p²}}) = (p+1)² = 12²");
    // Group law is a real abelian group.
    for a in pts.iter().take(6) {
        assert_eq!(c.add(a, &c.negate(a)), Point2::Infinity, "P + (−P) = O");
        for b in pts.iter().take(6) {
            for d in pts.iter().take(6) {
                assert_eq!(
                    c.add(&c.add(a, b), d),
                    c.add(a, &c.add(b, d)),
                    "associative"
                );
            }
        }
    }
    // FULL 3-torsion is 𝔽_{p²}-rational: E[3] ≅ (ℤ/3)² has exactly 9 points (3 | p+1 = 12).
    let e3 = pts
        .iter()
        .filter(|pt| c.mul(&ib(3), pt) == Point2::Infinity)
        .count();
    assert_eq!(e3, 9, "E[3] ≅ (ℤ/3)² is fully rational over 𝔽_{{p²}}");
}

#[test]
fn fp2_sqrt_round_trips() {
    let p = big("59");
    for (a, b) in [(3i64, 7), (1, 0), (0, 5), (11, 2), (23, 41)] {
        let x = (ib(a), ib(b));
        let sq = fp2_mul(&x, &x, &p);
        let r = fp2_sqrt(&sq, &p).expect("a square has a root");
        assert_eq!(fp2_mul(&r, &r, &p), sq, "(√α)² = α");
    }
}

#[test]
fn torsion_image_certificate_lifts_to_fp2_at_sike_scale() {
    // p = 59 ≡ 3 (mod 4), p+1 = 60 = 2²·3·5 — a SIKE-shaped prime: E[3] and E[5] are fully
    // 𝔽_{p²}-rational on the supersingular curve y² = x³ + x. Build a 3-isogeny, take a 5-torsion basis,
    // and verify the torsion-image compatibility law e_5(φP,φQ) = e_5(P,Q)^{deg φ} OVER 𝔽_{p²}.
    let p = big("59");
    let c = Curve2::new(fp2_const(1, &p), fp2_const(0, &p), p.clone());
    let kernel = point_of_order2(&c, 3).expect("3-torsion over 𝔽_{p²}");
    let iso = Isogeny2::from_kernel(&c, &kernel, 3).expect("Vélu over 𝔽_{p²}");
    let (pp, qq) = torsion_basis2(&c, 5).expect("full 5-torsion basis over 𝔽_{p²}");
    let ep = weil_pairing2(&c, &pp, &qq, 5).expect("basis pairing");
    assert_ne!(
        ep,
        fp2_const(1, &p),
        "independent basis ⟹ a nontrivial (primitive 5th root) pairing"
    );
    let (fp_, fq_) = (iso.eval(&pp), iso.eval(&qq));
    assert!(
        iso.codomain.is_on_curve(&fp_) && iso.codomain.is_on_curve(&fq_),
        "images land on the codomain"
    );
    let eq = weil_pairing2(&iso.codomain, &fp_, &fq_, 5).expect("image pairing");
    assert_eq!(
        eq,
        fp2_pow(&ep, iso.degree, &p),
        "e_5(φP,φQ) = e_5(P,Q)^{{deg φ}} over 𝔽_{{p²}}"
    );
}

// A SIDH-scale supersingular curve over 𝔽_{107²}: y²=x³+x (j=1728), #E=(p+1)²=108², so E[3³] and E[2²]
// are both rank-2 rational — a genuine 27-kernel keyspace with the order-4 automorphism ι acting on it.
fn sidh_scale_setup() -> (BigInt, Curve2, (Point2, Point2), (Point2, Point2)) {
    let p = big("107");
    let e0 = Curve2::new(fp2_const(1, &p), fp2_const(0, &p), p.clone());
    let basis_a = full_order_basis2(&e0, 3, 3).expect("rank-2 E[3³] basis");
    let basis_b = full_order_basis2(&e0, 2, 2).expect("rank-2 E[2²] basis");
    (p, e0, basis_a, basis_b)
}

#[test]
fn images_reconstruct_the_secret_generator_at_sidh_scale() {
    let (_p, e0, (pa, qa), (pb, qb)) = sidh_scale_setup();
    // Secret scalar → kernel generator → 3³-isogeny → published torsion images.
    let secret = ib(7);
    let gen = kernel_generator2(&e0, &pa, &qa, &secret);
    assert_eq!(
        point_order2(&e0, &gen, 28),
        Some(27),
        "the secret kernel generates order 3³"
    );
    let path = derive_isogeny_path2(&e0, &gen, 3, 3).expect("the secret 3³-isogeny");
    let images = (
        push_through2(&path, 3, &pb).expect("push P_B"),
        push_through2(&path, 3, &qb).expect("push Q_B"),
    );
    // Invert: recover a generator from ONLY (E₀, the two bases, the images).
    let (rs, rgen, rpath) =
        recover_secret2(&e0, (&pa, &qa), 3, 3, (&pb, &qb), (&images.0, &images.1))
            .expect("recovery");
    // The recovered isogeny reproduces the published images and lands on the same codomain — a genuine
    // images → generator inversion. (The scalar may be symmetry-equivalent to the planted one, not
    // identical: that is exactly the Aut-orbit structure exercised below.)
    assert_eq!(
        push_through2(&rpath, 3, &pb).unwrap(),
        images.0,
        "recovered chain reproduces φ(P_B)"
    );
    assert_eq!(
        push_through2(&rpath, 3, &qb).unwrap(),
        images.1,
        "recovered chain reproduces φ(Q_B)"
    );
    assert_eq!(
        rpath.last().unwrap().codomain,
        path.last().unwrap().codomain,
        "and lands on the target E"
    );
    assert_eq!(
        kernel_generator2(&e0, &pa, &qa, &rs),
        rgen,
        "the reported scalar yields the reported gen"
    );
    let _ = gen; // the planted generator is one valid preimage among its orbit
}

#[test]
fn the_structured_tree_recovery_matches_the_flat_oracle() {
    let (_p, e0, (pa, qa), (pb, qb)) = sidh_scale_setup();
    let secret = ib(11);
    let gen = kernel_generator2(&e0, &pa, &qa, &secret);
    let path = derive_isogeny_path2(&e0, &gen, 3, 3).unwrap();
    let images = (
        push_through2(&path, 3, &pb).unwrap(),
        push_through2(&path, 3, &qb).unwrap(),
    );
    // The ℓ-adic tree recursion recovers a generator digit by digit, walking the keyspace partition.
    let (digits, rs, rgen) =
        recover_secret_recursive2(&e0, (&pa, &qa), 3, 3, (&pb, &qb), (&images.0, &images.1))
            .expect("tree");
    assert_eq!(digits.len(), 3, "one ℓ-adic digit per isogeny step");
    assert!(digits.iter().all(|&d| d < 3), "each digit lies in 0..ℓ");
    // The reported scalar is exactly the digit reconstruction s = Σ dₖ·ℓᵏ.
    let s_check: i64 = digits
        .iter()
        .enumerate()
        .map(|(k, &d)| d as i64 * 3i64.pow(k as u32))
        .sum();
    assert_eq!(
        rs,
        ib(s_check),
        "the reported scalar is the ℓ-adic digit reconstruction"
    );
    // And it reproduces the images on the same codomain — the tree walk inverts, like the flat oracle.
    let rpath = derive_isogeny_path2(&e0, &rgen, 3, 3).unwrap();
    assert_eq!(
        push_through2(&rpath, 3, &pb).unwrap(),
        images.0,
        "the recovered generator reproduces φ(P_B)"
    );
    assert_eq!(
        push_through2(&rpath, 3, &qb).unwrap(),
        images.1,
        "and φ(Q_B)"
    );
    assert_eq!(
        rpath.last().unwrap().codomain,
        path.last().unwrap().codomain,
        "and lands on the target E"
    );
    let _ = secret; // planted secret is a valid solution; the tree may return a symmetry-equivalent one
}

#[test]
fn the_keyspace_partitions_into_aut_orbits_so_one_recovery_yields_the_orbit() {
    let (p, e0, (pa, qa), _b) = sidh_scale_setup();
    // Partition the 27-kernel keyspace by codomain j-invariant.
    let classes = keyspace_codomain_classes(&e0, (&pa, &qa), 3, 3);
    let total: usize = classes.iter().map(|(_, ss)| ss.len()).sum();
    assert_eq!(total, 27, "the classes partition the whole keyspace");
    assert!(
        classes.len() < 27,
        "the keyspace COLLAPSES — kernels share codomains ⟹ nontrivial symmetry"
    );
    // The rule, verified across the entire keyspace: E₀/⟨K⟩ ≅ E₀/⟨ι(K)⟩. So if ONE kernel is invertible
    // (its codomain is the target E), its whole ι-orbit maps to curves ≅ E — recovering one recovers the
    // orbit. This is symmetry forcing the answer out, the same lever as our SAT clause-orbit quotients.
    for s in 0..27u64 {
        let gen = kernel_generator2(&e0, &pa, &qa, &ib(s as i64));
        let j_gen = derive_isogeny_path2(&e0, &gen, 3, 3)
            .unwrap()
            .last()
            .unwrap()
            .codomain
            .j_invariant();
        let igen = aut_1728(&p, &gen);
        assert!(e0.is_on_curve(&igen), "ι(K) lands back on E₀");
        assert_eq!(
            point_order2(&e0, &igen, 28),
            Some(27),
            "ι preserves the order-27 kernel"
        );
        let j_igen = derive_isogeny_path2(&e0, &igen, 3, 3)
            .unwrap()
            .last()
            .unwrap()
            .codomain
            .j_invariant();
        assert_eq!(
            j_gen, j_igen,
            "ι(K) shares K's codomain j — the orbit shares its target curve"
        );
    }
}

#[test]
fn the_graph_reachability_oracle_prunes_the_tree_to_the_aut_orbit() {
    let (_p, e0, (pa, qa), (pb, qb)) = sidh_scale_setup();
    let secret = ib(11);
    let gen = kernel_generator2(&e0, &pa, &qa, &secret);
    let path = derive_isogeny_path2(&e0, &gen, 3, 3).unwrap();
    let target = path.last().unwrap().codomain.clone(); // the PUBLIC codomain E
    let images = (
        push_through2(&path, 3, &pb).unwrap(),
        push_through2(&path, 3, &qb).unwrap(),
    );

    // Backward BFS maps the reachable neighbourhood of E in the supersingular ℓ-isogeny graph.
    let dists = isogeny_graph_distances(&target, 3, 3);
    assert!(
        dists.len() > 1,
        "the ℓ-isogeny graph around E has genuine structure (a Ramanujan expander)"
    );
    assert!(
        dists.iter().any(|(_, d)| *d == 0),
        "E sits at distance 0 from itself"
    );

    // Oracle-pruned recovery still inverts (sound: the true path is never pruned) — reproduces the images
    // and lands on E.
    let (_rs, rgen, leaves) = recover_secret_pruned2(
        &e0,
        (&pa, &qa),
        3,
        3,
        (&pb, &qb),
        (&images.0, &images.1),
        &target,
    )
    .expect("pruned recovery");
    let rpath = derive_isogeny_path2(&e0, &rgen, 3, 3).unwrap();
    assert_eq!(
        push_through2(&rpath, 3, &pb).unwrap(),
        images.0,
        "pruned recovery reproduces φ(P_B)"
    );
    assert_eq!(
        push_through2(&rpath, 3, &qb).unwrap(),
        images.1,
        "and φ(Q_B)"
    );
    assert_eq!(
        rpath.last().unwrap().codomain,
        target,
        "and lands on the target E"
    );

    // The oracle confines leaf image-tests to the target's Aut-orbit: the reachability prune and the
    // symmetry collapse are the SAME cut. Only orbit members (same codomain j as E) can survive to a leaf.
    let classes = keyspace_codomain_classes(&e0, (&pa, &qa), 3, 3);
    let tj = target.j_invariant().unwrap();
    let orbit_size = classes
        .iter()
        .find(|(j, _)| *j == tj)
        .map(|(_, ss)| ss.len())
        .unwrap();
    assert!(
        orbit_size < 27,
        "the keyspace collapses to a strictly smaller orbit"
    );
    assert!(
        leaves <= orbit_size,
        "the graph oracle leaf-tests only the target's Aut-orbit, not all 27"
    );
}
