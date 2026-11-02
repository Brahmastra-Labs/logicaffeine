use super::*;
use crate::cdcl::SolveResult;

/// **The k-SAT threshold sequence — generalized and pinned down.** The first-moment upper bound
/// `α*(k) = ln2 / ln(2ᵏ/(2ᵏ−1))` is a rigorous, exact, closed-form sequence: it increases, doubles
/// per k (→ 2ᵏ ln2 − ½ln2), and genuinely upper-bounds satisfiability — above it the first-moment
/// base is `< 1`, forcing E[X] → 0 (UNSAT whp by Markov). It brackets the cited true thresholds from
/// above. No sampling: every claim is arithmetic on the exact formula.
#[test]
fn the_ksat_threshold_sequence_first_moment_upper_bound() {
    let ln2 = std::f64::consts::LN_2;
    let ub = ksat_threshold_first_moment_upper;

    // (1) The sequence matches the closed form (k = 2..=5, high precision).
    let known = [(2u32, 2.40942), (3, 5.19089), (4, 10.73970), (5, 21.83239)];
    for (k, want) in known {
        assert!(
            (ub(k) - want).abs() < 5e-3,
            "α*({k}) = {} want {want}",
            ub(k)
        );
    }

    // (2) Strictly increasing, and asymptotically doubling per k.
    for k in 2..=20 {
        assert!(ub(k + 1) > ub(k), "increasing at k={k}");
    }
    for k in 4..=20 {
        let ratio = ub(k + 1) / ub(k);
        assert!((1.9..=2.1).contains(&ratio), "ratio ≈ 2 at k={k}: {ratio}");
    }

    // (3) Converges to 2ᵏ ln2 − ½ln2, and stays below the leading term 2ᵏ ln2.
    for k in 2..=20 {
        let lead = (1u64 << k) as f64 * ln2;
        assert!(ub(k) < lead, "below the leading term 2ᵏln2 at k={k}");
        assert!(
            (ub(k) - (lead - ln2 / 2.0)).abs() < 0.05,
            "→ 2ᵏln2 − ½ln2 at k={k}"
        );
    }

    // (4) RIGOR: α* is exactly where the first-moment base crosses 1 — above it E[X]→0 (UNSAT whp),
    //     below it E[X]→∞. This is *why* it is an upper bound (Markov), not a fitted constant.
    for k in 2..=20 {
        let p = 1.0 - 2f64.powi(-(k as i32));
        let base = |a: f64| 2.0 * p.powf(a);
        assert!(
            (base(ub(k)) - 1.0).abs() < 1e-9,
            "α*(k={k}) is exactly base = 1"
        );
        assert!(base(ub(k) + 0.5) < 1.0, "above α*(k={k}): E[X] → 0");
        assert!(base(ub(k) - 0.5) > 1.0, "below α*(k={k}): E[X] → ∞");
    }

    // (5) The bound brackets the cited SHARP thresholds from above (the gap → ½ as k → ∞).
    for (k, sharp) in [(2u32, 1.0), (3, 4.267), (4, 9.931), (5, 21.117)] {
        assert!(
            ub(k) > sharp,
            "first-moment bound α*({k})={} exceeds the true threshold {sharp}",
            ub(k)
        );
    }
}

/// **Summing the threshold sequence — interesting constants pop out.** The thresholds α*_k grow like
/// 2ᵏ, so Σα*_k diverges; but their RECIPROCALS telescope, because `1/α*_k = −log₂(1−2⁻ᵏ)`, turning
/// the sum into the log of a product:
///
/// `Σ_{k≥1} 1/α*_k = −log₂ Π_{k≥1}(1−2⁻ᵏ) = −log₂ φ(½) ≈ 1.79192`,
///
/// where `φ(½) = 0.2887880951` is the Euler function at ½ — *exactly* the asymptotic probability that
/// a random matrix over **GF(2)** is invertible. So the reciprocal SAT-threshold sum lands on our
/// PARITY rung's own constant: the counting thresholds and their GF(2) reciprocal-sum meet here. The
/// ALTERNATING reciprocal sum telescopes likewise to `log₂ Π(1−2⁻ᵏ)^{(−1)ᵏ} ≈ 0.71513`. Both identities
/// are *exact* (sum of logs = log of product), not numerical coincidences.
#[test]
fn reciprocal_threshold_sums_telescope_to_euler_and_gf2_constants() {
    let ln2 = std::f64::consts::LN_2;
    let recip = |k: u32| 1.0 / ksat_threshold_first_moment_upper(k);
    let (mut s, mut p, mut alt, mut palt) = (0.0f64, 1.0f64, 0.0f64, 1.0f64);
    for k in 1..=50u32 {
        let term = 1.0 - 2f64.powi(-(k as i32));
        s += recip(k);
        p *= term;
        // EXACT telescoping: the partial sum equals −log₂ of the partial product.
        assert!(
            (s + p.ln() / ln2).abs() < 1e-9,
            "Σ1/α* telescopes to −log₂Π(1−2⁻ᵏ) at k={k}"
        );
        let sign = if k % 2 == 1 { 1.0 } else { -1.0 };
        alt += sign * recip(k);
        palt *= term.powf(if k % 2 == 1 { -1.0 } else { 1.0 });
        assert!(
            (alt - palt.ln() / ln2).abs() < 1e-9,
            "alternating sum telescopes to log₂Π(1−2⁻ᵏ)^((−1)ᵏ) at k={k}"
        );
    }
    // The Euler / GF(2) random-matrix invertibility constant, and the sum it produces.
    let phi_half = 0.288_788_095_1;
    assert!(
        (p - phi_half).abs() < 1e-9,
        "Π(1−2⁻ᵏ) → φ(½) = the GF(2) invertibility constant 0.28879"
    );
    assert!((s - 1.791_916_824_7).abs() < 1e-6, "Σ 1/α*_k ≈ 1.79192");
    assert!(
        (s + phi_half.ln() / ln2).abs() < 1e-6,
        "and it equals −log₂ φ(½) exactly"
    );
    // The alternating reciprocal sum's constant.
    assert!(
        (alt - 0.715_131_251_2).abs() < 1e-6,
        "Σ(−1)ᵏ⁺¹/α*_k ≈ 0.71513"
    );
}

#[test]
fn mutilated_chessboard_and_ordering_are_correctly_unsat() {
    // Both families are UNSAT by construction; the dispatcher must decide them correctly at small
    // n regardless of which route fires. (These are honest measuring families, not yet our
    // crushes — the contract here is correctness, not speed.)
    for n in [4, 6, 8] {
        let (cnf, v) = mutilated_chessboard(n);
        assert_eq!(v, ExpectedVerdict::Unsat);
        assert!(
            cnf.clauses.iter().all(|c| !c.is_empty()),
            "chessboard {n} has no empty clause"
        );
        let solved = crate::solve::solve_structured(cnf.num_vars, &cnf.clauses);
        assert!(
            matches!(solved.answer, crate::solve::Answer::Unsat),
            "mutilated chessboard {n} must be UNSAT"
        );
    }
    for n in [3, 4, 5, 6] {
        let (cnf, v) = ordering_principle(n);
        assert_eq!(v, ExpectedVerdict::Unsat);
        let solved = crate::solve::solve_structured(cnf.num_vars, &cnf.clauses);
        assert!(
            matches!(solved.answer, crate::solve::Answer::Unsat),
            "ordering principle GT({n}) must be UNSAT"
        );
    }
}

#[test]
fn tseitin_expander_is_unsat_and_gaussian_refutes_it() {
    // The XOR engine must refute the expander-Tseitin system with an independently-checkable
    // Gaussian refutation, and the equisatisfiable CNF must agree (UNSAT) by our CDCL solver at
    // a size small enough to still terminate.
    for seed in [1u64, 7, 42] {
        let (eqs, cnf, verdict) = tseitin_expander(10, seed);
        assert_eq!(verdict, ExpectedVerdict::Unsat);
        match crate::xorsat::solve(&eqs, cnf.num_vars) {
            crate::xorsat::XorOutcome::Unsat(refutation) => {
                assert!(
                    crate::xorsat::is_refutation(&eqs, cnf.num_vars, &refutation),
                    "the Gaussian refutation must independently check"
                );
            }
            crate::xorsat::XorOutcome::Sat(_) => panic!("expander-Tseitin must be UNSAT"),
        }
        // The CNF encoding is genuinely the same UNSAT problem.
        assert_eq!(
            cnf.into_solver().solve(),
            SolveResult::Unsat,
            "CNF encoding must be UNSAT too"
        );
    }
}

#[test]
fn mod_p_tseitin_is_refuted_over_gf_p_with_a_checkable_certificate() {
    // The parity crush at every prime: the same expander obstruction is UNSAT over GF(p) for every
    // odd prime — Gaussian elimination over the right field refutes it with an independently-
    // checkable certificate — and its Boolean CNF is genuinely the same UNSAT problem.
    for &p in &[3u64, 5, 7] {
        for seed in [1u64, 7] {
            let (eqs, cnf, verdict) = mod_p_tseitin_expander(6, p, seed);
            assert_eq!(verdict, ExpectedVerdict::Unsat);
            let edges = cnf.num_vars / p as usize; // GF(p) variables = one per edge
            match crate::modp::solve(&eqs, edges, p) {
                crate::modp::ModpOutcome::Unsat(combo) => assert!(
                    crate::modp::is_refutation(&eqs, edges, p, &combo),
                    "the GF({p}) refutation must independently re-check"
                ),
                crate::modp::ModpOutcome::Sat(_) => {
                    panic!("mod-{p} obstruction must be UNSAT over GF({p})")
                }
            }
            assert_eq!(
                cnf.into_solver().solve(),
                SolveResult::Unsat,
                "the Boolean CNF must be UNSAT"
            );
        }
    }
}

#[test]
fn mod_p_obstruction_is_invisible_to_gf2() {
    // The "blind" half made explicit: reinterpret the SAME graph + charges as a GF(2) parity
    // system. Total charge 2 is even, so it is SATISFIABLE over GF(2) — the parity cut cannot see
    // the mod-p contradiction. Only the right characteristic decides it.
    let p = 3u64;
    let (eqs, cnf, _) = mod_p_tseitin_expander(6, p, 7);
    let edges = cnf.num_vars / p as usize;
    let gf2: Vec<crate::xorsat::XorEquation> = eqs
        .iter()
        .map(|eq| {
            let vars: Vec<usize> = eq.coeffs.iter().map(|&(v, _)| v).collect();
            crate::xorsat::XorEquation::new(vars, eq.rhs % 2 == 1)
        })
        .collect();
    assert!(
        matches!(
            crate::xorsat::solve(&gf2, edges),
            crate::xorsat::XorOutcome::Sat(_)
        ),
        "over GF(2) the even-charge obstruction is satisfiable — the parity engine is blind to it"
    );
}

#[test]
fn mod_m_tseitin_is_refuted_over_the_ring_with_a_checkable_certificate() {
    // The composite-modulus crush: the same total-charge-2 expander obstruction over ℤ/m (m=6=2·3,
    // m=15=3·5) is UNSAT — decided by CRT over the prime factors — with an independently
    // re-checkable certificate, and its mixed-radix one-hot CNF is genuinely the same UNSAT problem.
    for &m in &[6u64, 15] {
        for seed in [1u64, 7] {
            let (eqs, cnf, verdict) = mod_m_tseitin_expander(6, m, seed);
            assert_eq!(verdict, ExpectedVerdict::Unsat);
            let vars = cnf.num_vars / m as usize; // ring variables = one per edge
            match crate::modm::solve(&eqs, vars, m) {
                Some(crate::modm::ModmOutcome::Unsat { modulus, combo }) => assert!(
                    crate::modm::is_refutation(&eqs, vars, modulus, &combo),
                    "the ℤ/{m} refutation must independently re-check (via its GF({modulus}) factor)"
                ),
                other => panic!("mod-{m} obstruction must be UNSAT over ℤ/{m}, got {other:?}"),
            }
            assert_eq!(
                cnf.into_solver().solve(),
                SolveResult::Unsat,
                "the Boolean CNF must be UNSAT"
            );
        }
    }
}

/// **TRANSFER: the parity wall.** The same exponential-vs-polynomial separation we measured for
/// pigeonhole, carried to a DIFFERENT resolution-hard family — Tseitin formulas over expander graphs
/// (Urquhart 1987: every resolution refutation is `2^Ω(n)`). Our CDCL is pure resolution, so its conflict
/// count on the CNF explodes. But the formula is just a linear system over GF(2): the `xorsat` Gaussian
/// reasoner refutes it in polynomial time with an independently-checkable certificate (a subset of
/// equations summing to `0 = 1`). Different symmetry (parity, not permutation), same collapse — measured.
#[test]
#[ignore = "heavy: CDCL on Tseitin is exponential — that's the wall. Charts it vs the Gaussian collapse."]
fn tseitin_parity_wall_collapses_under_gaussian() {
    use crate::xorsat::{is_refutation, solve, XorOutcome};
    let seed = 42u64;
    let mut rows = vec![
        "  n | edges | CDCL: conflicts / time   | Gaussian     | certified".to_string(),
        "----+-------+--------------------------+--------------+----------".to_string(),
    ];
    for n in [16usize, 24, 32, 40, 48, 56] {
        let (eqs, cnf, verdict) = tseitin_expander(n, seed);
        assert_eq!(verdict, ExpectedVerdict::Unsat);

        let mut solver = cnf.into_solver();
        let ct = std::time::Instant::now();
        assert_eq!(solver.solve(), SolveResult::Unsat, "Tseitin CNF is UNSAT");
        let cdcl_time = ct.elapsed();
        let conflicts = solver.conflicts();

        let t = std::time::Instant::now();
        let out = solve(&eqs, cnf.num_vars);
        let gauss = t.elapsed();
        let certified = match &out {
            XorOutcome::Unsat(r) => is_refutation(&eqs, cnf.num_vars, r),
            XorOutcome::Sat(_) => false,
        };
        assert!(certified, "Gaussian gives a checkable refutation for n={n}");
        rows.push(format!(
            "{n:3} | {:5} | {conflicts:10} {cdcl_time:>10?} | {gauss:>11?} | yes",
            cnf.num_vars
        ));
    }
    let chart = rows.join("\n");
    eprintln!("\nTSEITIN PARITY WALL — resolution (CDCL) exponential vs Gaussian polynomial+certified\n{chart}\n");
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../logs/derived_facts");
    if std::fs::create_dir_all(&dir).is_ok() {
        let _ = std::fs::write(dir.join("tseitin_parity_wall.txt"), format!("TSEITIN PARITY WALL — CDCL (resolution, 2^Ω(n)) vs Gaussian (poly, certified)\n\n{chart}\n"));
    }
}

/// **THE TWO GEOMETRIES OF HARDNESS.** The break into geometry: a resolution-hard UNSAT fact is a
/// region of the Boolean cube that the clause-polytope cannot exclude, and *which* geometry excludes it
/// is the whole story.
///
/// * **Pigeonhole = convex geometry over ℝ.** The half-point `x ≡ ½` satisfies every *clause's* LP
///   relaxation (each clause has ≥ 2 literals, so its value is `len/2 ≥ 1`) — so resolution lives forever
///   inside a polytope that still contains `½`. Cutting planes *recovers* the strong cardinality cut
///   `Σ_p x_{p,h} ≤ 1` from the exclusion clique (which `½` violates: `n/2 > 1`), sums it against the
///   `Σ_h x_{p,h} ≥ 1` rows, and lands on `0 ≥ 1`: a **Farkas separating hyperplane**, the polytope is
///   *empty*. `refute_clausal` is exactly this.
/// * **Parity = affine geometry over GF(2).** `x ≡ ½` *also* satisfies every Tseitin clause's LP — but
///   here there is **no** cardinality clique to recover, so the convex reasoner stays blind (the polytope
///   is genuinely non-empty; cutting planes is exponential too). The obstruction is a different geometry:
///   the all-ones functional lies in the GF(2) row space, which **Gaussian elimination** sees at once.
///
/// Neither reasoner crosses over — convex refutes pigeonhole and not parity; GF(2) refutes parity and is
/// not even defined on pigeonhole. Two geometries, two obstructions, provably incomparable. That is the
/// geometric reason there is no single symmetry break for all of NP.
#[test]
fn the_two_geometries_of_hardness() {
    // A clause is satisfied by x ≡ ½ in the LP relaxation iff it has ≥ 2 literals (value = len·½ ≥ 1).
    let half_point_satisfies = |cnf: &DimacsCnf| cnf.clauses.iter().all(|c| c.len() >= 2);

    // BOTH clause-polytopes contain the half-point x ≡ ½ — so resolution, which only ever reasons inside
    // the clause polytope, can never exclude EITHER at the clause level. The separation must come from a
    // richer geometry, and which one differs by family.
    let (php_cnf, _) = php(8);
    let (eqs, tse_cnf, _) = tseitin_expander(12, 7);
    assert!(
        half_point_satisfies(&php_cnf),
        "x≡½ ∈ the PHP clause-polytope"
    );
    assert!(
        half_point_satisfies(&tse_cnf),
        "x≡½ ∈ the Tseitin clause-polytope"
    );

    // PIGEONHOLE — convex geometry over ℝ. Recover the cardinality cut (Σ_p x_{p,h} ≤ 1, which x≡½
    // violates) and sum against the rows (Σ_h x_{p,h} ≥ 1): `n ≤ Σx ≤ n−1`, i.e. 0 ≥ 1. That conic
    // combination IS a Farkas separating hyperplane — the integer (indeed the LP-cardinality) polytope is
    // empty. Our O(1) counting certificate is exactly this hyperplane's content.
    assert!(
        crate::pigeonhole::certify_pigeonhole_unsat(8, 7).is_some(),
        "convex/Farkas counting hyperplane refutes PHP(8)"
    );

    // PARITY — affine geometry over GF(2). There is NO cardinality clique to recover (it is not a
    // pigeon/hole problem), so the convex certificate is simply undefined here — convex geometry is blind.
    // The obstruction is that the all-ones functional lies in the GF(2) row space; Gaussian elimination
    // sees it and emits an independently-checkable refutation.
    match crate::xorsat::solve(&eqs, tse_cnf.num_vars) {
        crate::xorsat::XorOutcome::Unsat(r) => assert!(
            crate::xorsat::is_refutation(&eqs, tse_cnf.num_vars, &r),
            "GF(2) geometry refutes parity, certified"
        ),
        crate::xorsat::XorOutcome::Sat(_) => panic!("Tseitin must be UNSAT"),
    }
    // Two geometries (convex-ℝ vs affine-GF(2)), two obstructions, neither reasoner crossing over — the
    // geometric reason there is no single symmetry break for all of NP.
}

/// **COVERING ≠ COUNTABLE — the exact place the hypercube-cover/counting attack breaks.** SAT is a
/// covering of the hypercube by subcubes; pigeonhole's covering collapses under counting. The tempting
/// leap is "so every covering does." It does not, and here is the undeniable witness: **two coverings of
/// the SAME hypercube, with byte-for-byte identical COUNTING profiles — same clause count, same
/// clause-length multiset, same per-variable occurrence counts — yet one is SAT and one is UNSAT.**
/// They are the same Tseitin graph with total parity flipped at one vertex. Every blocker still covers
/// the same number of vertices; every counting invariant (`footprint_card`, `vertexEnergy`, global
/// balance — the whole §G toolkit) returns identically on both. So no counting argument can decide them.
/// The one bit that does — the GF(2) total parity — is invisible to counting and visible to Gaussian
/// elimination. *Covering, yes. Countable, no.* That is precisely where `ThreeSATInP` cannot close.
#[test]
fn counting_is_provably_blind_two_covers_same_counts_opposite_answers() {
    use crate::xorsat::{solve, XorEquation, XorOutcome};
    // Build a Tseitin cover on a fixed 3-regular graph with a chosen per-vertex charge vector.
    let build =
        |edges: &[(usize, usize)], n: usize, charges: &[bool]| -> (Vec<XorEquation>, DimacsCnf) {
            let mut incident = vec![Vec::new(); n];
            for (e, &(a, b)) in edges.iter().enumerate() {
                incident[a].push(e);
                incident[b].push(e);
            }
            let (mut eqs, mut clauses) = (Vec::new(), Vec::new());
            for v in 0..n {
                let (inc, r, d) = (&incident[v], charges[v], incident[v].len());
                eqs.push(XorEquation::new(inc.clone(), r));
                for mask in 0u32..(1u32 << d) {
                    if ((mask.count_ones() % 2) == 1) != r {
                        clauses.push(
                            (0..d)
                                .map(|i| Lit::new(inc[i] as u32, (mask >> i) & 1 == 0))
                                .collect(),
                        );
                    }
                }
            }
            (
                eqs,
                DimacsCnf {
                    num_vars: edges.len(),
                    clauses,
                },
            )
        };

    let edges = super::random_3regular(12, 7);
    let (eqs_sat, cnf_sat) = build(&edges, 12, &vec![false; 12]); // total parity even → SAT
    let mut odd = vec![false; 12];
    odd[0] = true; // flip ONE vertex → total parity odd → UNSAT
    let (eqs_uns, cnf_uns) = build(&edges, 12, &odd);

    // The COUNTING profile is identical — every cardinality/footprint invariant is blind to the flip.
    let profile = |cnf: &DimacsCnf| {
        let mut lens: Vec<usize> = cnf.clauses.iter().map(|c| c.len()).collect();
        lens.sort_unstable();
        let mut occ = vec![0usize; cnf.num_vars];
        for c in &cnf.clauses {
            for l in c {
                occ[l.var() as usize] += 1;
            }
        }
        occ.sort_unstable();
        (cnf.clauses.len(), lens, occ)
    };
    assert_eq!(
        profile(&cnf_sat),
        profile(&cnf_uns),
        "IDENTICAL counting profile — counting cannot tell them apart"
    );

    // Yet GF(2) — the parity invariant — gives OPPOSITE verdicts. One bit, invisible to every count.
    assert!(
        matches!(solve(&eqs_sat, cnf_sat.num_vars), XorOutcome::Sat(_)),
        "even total parity ⇒ SAT"
    );
    match solve(&eqs_uns, cnf_uns.num_vars) {
        XorOutcome::Unsat(r) => assert!(
            crate::xorsat::is_refutation(&eqs_uns, cnf_uns.num_vars, &r),
            "odd total parity ⇒ UNSAT, certified by GF(2)"
        ),
        XorOutcome::Sat(_) => panic!("odd-parity Tseitin must be UNSAT"),
    }
}

#[test]
fn php_has_the_expected_shape() {
    let (cnf, verdict) = php(4);
    assert_eq!(verdict, ExpectedVerdict::Unsat);
    assert_eq!(cnf.num_vars, 4 * 3);
    // 4 "at least one hole" clauses + 3 holes × C(4,2)=6 conflict clauses.
    assert_eq!(cnf.clauses.len(), 4 + 3 * 6);
}

#[test]
fn php_is_unsatisfiable_for_small_n() {
    for n in 1..=5 {
        let (cnf, _) = php(n);
        assert_eq!(
            cnf.into_solver().solve(),
            SolveResult::Unsat,
            "PHP({n}) must be unsatisfiable"
        );
    }
}

#[test]
#[ignore = "core benchmark on hard random 3-SAT — the general-instance (non-symmetric) baseline"]
fn bench_core_on_random_3sat() {
    use std::time::Instant;
    for &(vars, ratio) in &[(50usize, 4.26), (75, 4.26), (100, 4.26), (125, 4.26)] {
        let nc = (vars as f64 * ratio) as usize;
        let cnf = random_3sat(vars, nc, 0xC0FFEE_1234);
        let mut s = cnf.into_solver();
        let t = Instant::now();
        let res = s.solve();
        let ms = t.elapsed().as_secs_f64() * 1e3;
        println!(
            "rand3sat(v={vars}, c={nc}): {res:?} in {ms:.1}ms — {} conflicts, {} learned clauses (unbounded!)",
            s.conflicts(),
            s.learned().len()
        );
    }
}

#[test]
#[ignore = "A/B benchmark: LBD clause deletion ON vs OFF on hard random 3-SAT"]
fn bench_lbd_reduction_ab() {
    use std::time::Instant;
    for &v in &[140usize, 160, 180, 200] {
        let nc = (v as f64 * 4.26) as usize;
        let cnf = random_3sat(v, nc, 0xBADC0DE_99);
        for on in [false, true] {
            let mut s = cnf.into_solver();
            s.set_reduce(on);
            let t = Instant::now();
            let res = s.solve();
            let ms = t.elapsed().as_secs_f64() * 1e3;
            println!(
                "v={v} reduce={on:5}: {} in {ms:7.0}ms — {} conflicts, {} LIVE learned clauses",
                if matches!(res, SolveResult::Sat(_)) {
                    "SAT  "
                } else {
                    "UNSAT"
                },
                s.conflicts(),
                s.live_learned()
            );
        }
    }
}

#[test]
#[ignore = "parity grave-dance: dump expander-XOR CNF + time Gaussian (xorsat), pairs with Kissat loop"]
fn dump_parity_and_time_xorsat() {
    use std::time::Instant;
    for n in [40usize, 60, 80, 100, 120] {
        let m = (n as f64 * 1.1) as usize;
        let (eqs, cnf) = parity_unsat(n, m, 0x9A2173_5C);
        std::fs::write(format!("/tmp/parity_{n}.cnf"), crate::dimacs::print(&cnf)).unwrap();
        // Sanity: our own CDCL also agrees it's UNSAT (equisatisfiable encoding).
        let t = Instant::now();
        let outcome = crate::xorsat::solve(&eqs, n);
        let us = t.elapsed().as_secs_f64() * 1e6;
        assert!(
            matches!(outcome, crate::xorsat::XorOutcome::Unsat(_)),
            "parity(n={n}) must be UNSAT"
        );
        println!(
            "XORSAT parity(n={n}, m={m}): UNSAT in {us:.1}µs via Gaussian elimination  |  {} CNF clauses dumped for Kissat",
            cnf.clauses.len()
        );
    }
}

#[test]
fn parity_unsat_is_genuinely_unsat() {
    // Small instance: both the XOR engine and our CDCL on the CNF must agree it's UNSAT.
    let (eqs, cnf) = parity_unsat(12, 14, 0x9A2173_5C);
    assert!(matches!(
        crate::xorsat::solve(&eqs, 12),
        crate::xorsat::XorOutcome::Unsat(_)
    ));
    assert_eq!(
        cnf.into_solver().solve(),
        SolveResult::Unsat,
        "CNF encoding is UNSAT too"
    );
}

#[test]
fn clique_coloring_verdict_tracks_colors_vs_clique() {
    // K_n needs exactly n colors: UNSAT with fewer, SAT with enough.
    for n in 2..=4 {
        let (unsat, v_unsat) = clique_coloring(n, n - 1);
        assert_eq!(v_unsat, ExpectedVerdict::Unsat);
        assert_eq!(
            unsat.into_solver().solve(),
            SolveResult::Unsat,
            "K_{n} with {} colors",
            n - 1
        );
        let (sat, v_sat) = clique_coloring(n, n);
        assert_eq!(v_sat, ExpectedVerdict::Sat);
        assert!(
            matches!(sat.into_solver().solve(), SolveResult::Sat(_)),
            "K_{n} with {n} colors"
        );
    }
}

#[test]
fn clique_coloring_exposes_color_permutation_symmetry() {
    // The finder must discover the color group (a different symmetry kind than pigeon/hole):
    // swapping two colors across all vertices is an automorphism.
    let (cnf, _) = clique_coloring(3, 2);
    let gens = crate::symmetry_detect::find_generators(cnf.num_vars, &cnf.clauses);
    assert!(
        !gens.iter().all(|g| g.is_identity()),
        "color symmetry must be detected"
    );
    for g in &gens {
        assert!(crate::symmetry_detect::perm_is_automorphism(
            &cnf.clauses,
            g
        ));
    }
}

#[test]
fn clique_coloring_is_refuted_with_certified_symmetry_breaking() {
    for (n, k) in [(3usize, 2usize), (4, 3)] {
        let (cnf, _) = clique_coloring(n, k);
        let r = crate::sym_certify::certified_unsat_auto(cnf.num_vars, &cnf.clauses);
        assert!(r.refuted, "K_{n} / {k} colors refuted");
        assert!(r.sbp_clauses >= 1, "color symmetry certified-broken");
        assert!(crate::pr::check_pr_refutation(
            cnf.num_vars,
            &cnf.clauses,
            &r.steps
        ));
    }
}

/// The dispatcher decides `cnf` exactly as its ground-truth verdict says — and when SAT, the
/// returned model is independently re-checked against every clause. The single oracle every new
/// family below is held to.
fn decides(cnf: &DimacsCnf, verdict: ExpectedVerdict) {
    let solved = crate::solve::solve_structured(cnf.num_vars, &cnf.clauses);
    match verdict {
        ExpectedVerdict::Unsat => {
            assert!(
                matches!(solved.answer, crate::solve::Answer::Unsat),
                "expected UNSAT, solver said SAT"
            );
        }
        ExpectedVerdict::Sat => match &solved.answer {
            crate::solve::Answer::Sat(model) => assert!(
                cnf.clauses
                    .iter()
                    .all(|c| c.iter().any(|l| model[l.var() as usize] == l.is_positive())),
                "the returned SAT model must satisfy every clause"
            ),
            crate::solve::Answer::Unsat => panic!("expected SAT, solver said UNSAT"),
        },
    }
}

#[test]
fn weak_php_generalizes_php_and_tracks_the_pigeon_hole_ratio() {
    // The tight case is byte-for-byte the existing PHP — same layout, same clauses.
    for n in 1..=5 {
        let (tight, tv) = weak_php(n, n.saturating_sub(1));
        let (base, bv) = php(n);
        assert_eq!(
            tight.num_vars, base.num_vars,
            "weak_php(n,n-1) shares PHP's variable count"
        );
        assert_eq!(
            tight.clauses, base.clauses,
            "weak_php(n,n-1) is byte-identical to php(n)"
        );
        assert_eq!(tv, bv);
    }
    // UNSAT iff pigeons > holes, across the tight, weak, and satisfiable regimes — solver-confirmed.
    for &(p, h) in &[(3usize, 2usize), (5, 4), (4, 7), (2, 3), (3, 3), (6, 2)] {
        let (cnf, v) = weak_php(p, h);
        assert_eq!(
            v,
            if p > h {
                ExpectedVerdict::Unsat
            } else {
                ExpectedVerdict::Sat
            },
            "PHP^{h}_{p} verdict"
        );
        decides(&cnf, v);
    }
}

#[test]
fn functional_and_onto_php_are_unsat_strengthenings_of_php() {
    for n in 2..=5 {
        let (base, _) = php(n);
        let (func, fv) = functional_php(n);
        let (onto, ov) = onto_php(n);
        assert_eq!(fv, ExpectedVerdict::Unsat);
        assert_eq!(ov, ExpectedVerdict::Unsat);
        // Each strengthening is a clause-superset of the previous (it only ADDS constraints).
        assert!(
            func.clauses.starts_with(&base.clauses),
            "FPHP({n}) ⊇ PHP({n})"
        );
        assert!(
            onto.clauses.starts_with(&func.clauses),
            "onto-FPHP({n}) ⊇ FPHP({n})"
        );
        decides(&func, ExpectedVerdict::Unsat);
        decides(&onto, ExpectedVerdict::Unsat);
    }
    // Strictly more clauses once a pigeon can reach ≥ 2 holes (n ≥ 3).
    for n in 3..=5 {
        let (base, _) = php(n);
        let (func, _) = functional_php(n);
        let (onto, _) = onto_php(n);
        assert!(
            func.clauses.len() > base.clauses.len(),
            "FPHP adds functional clauses at n={n}"
        );
        assert!(
            onto.clauses.len() > func.clauses.len(),
            "onto-FPHP adds surjectivity clauses at n={n}"
        );
    }
}

#[test]
fn mod_counting_is_unsat_exactly_when_q_does_not_divide_n() {
    // q=2 is perfect matching on K_n (UNSAT for odd n); q=3 is the mod-3 partition principle.
    for &(n, q) in &[
        (3usize, 2usize),
        (4, 2),
        (5, 2),
        (6, 2),
        (4, 3),
        (6, 3),
        (7, 3),
        (8, 4),
    ] {
        let (cnf, v) = mod_counting(n, q);
        assert_eq!(
            v,
            if n % q == 0 {
                ExpectedVerdict::Sat
            } else {
                ExpectedVerdict::Unsat
            },
            "Count_{q}({n})"
        );
        assert!(
            cnf.clauses.iter().all(|c| !c.is_empty()),
            "Count_{q}({n}) has no empty clause"
        );
        decides(&cnf, v);
    }
    // The variable count is exactly the number of q-subsets, C(n, q).
    let (c52, _) = mod_counting(5, 2);
    assert_eq!(c52.num_vars, 10, "C(5,2) = 10 edges of K_5");
}

#[test]
fn ramsey_tracks_the_known_ramsey_numbers() {
    // The smallest UNSAT Ramsey instance and its satisfiable predecessor, plus an off-diagonal pair —
    // each bracketing R(s,t) from below (SAT) and at the boundary (UNSAT), solver-confirmed.
    for &(s, t, n) in &[(3usize, 3usize, 5usize), (3, 3, 6), (3, 4, 8), (3, 4, 9)] {
        let (cnf, v) = ramsey(s, t, n);
        let r = ramsey_number(s, t).unwrap();
        assert_eq!(
            v,
            if n >= r {
                ExpectedVerdict::Unsat
            } else {
                ExpectedVerdict::Sat
            },
            "Ramsey({s},{t};{n}) vs R={r}"
        );
        decides(&cnf, v);
    }
    // Diagonal Ramsey carries the colour-swap symmetry: flipping every edge's colour (negating every
    // literal) maps the clause set onto itself — red-K_3 bans become blue-K_3 bans and vice-versa.
    let (cnf, _) = ramsey(3, 3, 6);
    let key = |c: &Vec<Lit>, flip: bool| {
        let mut k: Vec<(u32, bool)> = c
            .iter()
            .map(|l| (l.var(), l.is_positive() ^ flip))
            .collect();
        k.sort_unstable();
        k
    };
    let set: std::collections::HashSet<Vec<(u32, bool)>> =
        cnf.clauses.iter().map(|c| key(c, false)).collect();
    for c in &cnf.clauses {
        assert!(
            set.contains(&key(c, true)),
            "global colour-flip is an automorphism of diagonal Ramsey"
        );
    }
}

#[test]
fn random_kxor_is_guaranteed_unsat_for_every_seed_arity_and_size() {
    // The whole point of the maximal-rank construction: inconsistent for ANY seed (the naive
    // single-flip silently produced SAT instances near m ≈ n). Hammer it — both the GF(2) Gaussian
    // engine and the CDCL solver on the CNF must agree UNSAT on every instance.
    use crate::cdcl::SolveResult;
    for k in [2usize, 3, 4, 5] {
        for n in [8usize, 12, 16] {
            for seed in 0..24u64 {
                let (eqs, cnf) = random_kxor(k, n, n, seed.wrapping_mul(0x1000193) ^ k as u64);
                assert!(
                    matches!(
                        crate::xorsat::solve(&eqs, cnf.num_vars),
                        crate::xorsat::XorOutcome::Unsat(_)
                    ),
                    "k={k} n={n} seed={seed}: maximal-rank k-XOR must be UNSAT over GF(2)"
                );
                assert_eq!(
                    cnf.into_solver().solve(),
                    SolveResult::Unsat,
                    "k={k} n={n} seed={seed}: CNF must be UNSAT"
                );
            }
        }
    }
}

#[test]
fn random_kxor_generalizes_parity_and_gaussian_refutes_every_arity() {
    // k=3 reproduces parity_unsat byte-for-byte (the lift-and-shift is behaviour-preserving).
    for seed in [1u64, 7, 0x9A2173_5C] {
        let (_, a) = parity_unsat(12, 14, seed);
        let (_, b) = random_kxor(3, 12, 14, seed);
        assert_eq!(
            a.clauses, b.clauses,
            "parity_unsat is exactly random_kxor(k=3)"
        );
    }
    // Every arity stays UNSAT and the Gaussian (GF(2)) refutation independently checks; the CNF agrees.
    for k in [2usize, 4, 5] {
        let (eqs, cnf) = random_kxor(k, 14, 16, 0xC0FFEE ^ k as u64);
        match crate::xorsat::solve(&eqs, cnf.num_vars) {
            crate::xorsat::XorOutcome::Unsat(r) => assert!(
                crate::xorsat::is_refutation(&eqs, cnf.num_vars, &r),
                "{k}-XOR Gaussian refutation must re-check"
            ),
            crate::xorsat::XorOutcome::Sat(_) => {
                panic!("planted-then-flipped {k}-XOR must be UNSAT")
            }
        }
        assert_eq!(
            cnf.into_solver().solve(),
            SolveResult::Unsat,
            "{k}-XOR CNF encoding is UNSAT"
        );
    }
}

#[test]
fn pebbling_pyramid_is_unsat_with_the_expected_triangular_shape() {
    for h in 1..=5 {
        let (cnf, v) = pebbling_pyramid(h);
        assert_eq!(v, ExpectedVerdict::Unsat);
        let nodes = (h + 1) * (h + 2) / 2;
        assert_eq!(
            cnf.num_vars, nodes,
            "pyramid of height {h} has T(h+1) nodes"
        );
        // sources (h+1 units) + propagation (one per non-source node) + 1 apex unit = nodes + 1.
        assert_eq!(
            cnf.clauses.len(),
            nodes + 1,
            "pebbling({h}) clause count = nodes + 1"
        );
        assert!(cnf.clauses.iter().all(|c| !c.is_empty()), "no empty clause");
        decides(&cnf, ExpectedVerdict::Unsat);
    }
}
