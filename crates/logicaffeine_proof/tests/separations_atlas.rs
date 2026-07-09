//! **The certified separations atlas** — one artifact, every proof-system row two-sided, every half
//! re-checked by its independent verifier with zero trust in the producer.
//!
//! Each row is one family measured against multiple proof systems, its LOWER half a certified
//! impossibility (a dual witness / a closed clause set) and its UPPER half a certified proof in a
//! stronger or characteristic-matched system:
//!
//! | family | lower half (certified) | upper half (certified) |
//! |---|---|---|
//! | `PHP_m` | `GF(2)` NS-degree ≥ 2(m−1) (pseudo-expectation) + res-width certificate | SR refutation, exactly `m(m−1)/2` symmetry steps (`pr::check_pr_refutation`) |
//! | Tseitin | res-width > w (closed-set certificate) | `GF(2)` linear refutation (`xorsat::is_refutation`) |
//! | `Count_3` | `GF(2)` NS-degree ≥ 3 (pseudo-expectation, linear encoding) — ∀n ≡ 3 (mod 4) via the stabilized collapsed dual | `GF(3)` linear refutation (`modp::is_refutation`) |
//!
//! The `Count_3` row is the marquee **characteristic-mismatch** cell: the *same* family carries a
//! certified `GF(2)` hardness floor and a one-Gaussian-pass `GF(3)` refutation — algebra sees the
//! obstruction exactly when its characteristic divides the count. External verification artifacts
//! for the upper halves (DRAT via drat-trim, `.sr` via sr2drat) live in `proofs/algebraic/` and
//! `benchmarks/sat/proofs/`; this test walks every half through the in-repo independent checkers.

use logicaffeine_proof::cdcl::Lit;
use logicaffeine_proof::families;
use logicaffeine_proof::modp::{self, ModpEquation, ModpOutcome};
use logicaffeine_proof::polycalc::{
    check_ns_lower_bound, check_ns_lower_bound_polys, exactly_one_linear_generators,
    monomials_up_to_degree, ns_lower_bound_witness_polys, php_is_hole_injective,
};
use logicaffeine_proof::pr::check_pr_refutation;
use logicaffeine_proof::res_width::{
    check_res_width_lower_bound, min_res_width_clauses, resolution_width_closure, WidthConvention,
};
use logicaffeine_proof::sym_certify::heule_php_refutation;
use logicaffeine_proof::xorsat::{self, XorOutcome};

#[test]
fn the_certified_separations_atlas_is_two_sided_and_re_checkable() {
    let mut atlas: Vec<String> = Vec::new();

    // ── Row 1: pigeonhole ────────────────────────────────────────────────────────────────────────
    for m in [3usize, 4] {
        let (php, _) = families::php(m);
        let holes = m - 1;

        // Lower half A — GF(2) NS-degree ≥ 2(m−1): the uniform hole-injective pseudo-expectation,
        // re-checked with zero trust in any solver.
        let d = 2 * holes - 1;
        let witness: Vec<u64> = (0u64..(1u64 << php.num_vars))
            .filter(|&mo| mo.count_ones() as usize <= d && php_is_hole_injective(mo, holes))
            .collect();
        assert!(
            check_ns_lower_bound(php.num_vars, &php.clauses, d, &witness),
            "PHP({m}): NS-degree ≥ {} certified by the hole-injective witness",
            2 * holes
        );

        // Lower half B — resolution width: the closed clause set certifies width > w* − 1.
        let wstar = min_res_width_clauses(php.num_vars, &php.clauses, WidthConvention::WideAxioms)
            .expect("PHP is UNSAT");
        let closed = resolution_width_closure(&php.clauses, wstar - 1, WidthConvention::WideAxioms);
        assert!(
            check_res_width_lower_bound(&php.clauses, wstar - 1, WidthConvention::WideAxioms, &closed),
            "PHP({m}): res-width > {} certified by the closed set",
            wstar - 1
        );

        // Upper half — the polynomial SR refutation: exactly m(m−1)/2 symmetry steps, the whole
        // composed proof re-checked against the ORIGINAL clauses by the independent PR/SR checker.
        let sr = heule_php_refutation(m);
        assert!(sr.refuted, "PHP({m}): the steered SR refutation refutes");
        assert_eq!(
            sr.sbp_clauses,
            m * (m - 1) / 2,
            "PHP({m}): the SR proof is exactly quadratic — the certificate carries its own clock"
        );
        assert!(
            check_pr_refutation(php.num_vars, &php.clauses, &sr.steps),
            "PHP({m}): the composed SR proof re-checks against the original formula alone"
        );
        atlas.push(format!(
            "PHP({m}): NS-degree ≥ {} | res-width = {wstar} | SR size = {} steps",
            2 * holes,
            sr.sbp_clauses
        ));
    }

    // ── Row 2: Tseitin expanders ─────────────────────────────────────────────────────────────────
    for n in [6usize, 8] {
        let (eqs, cnf, _) = families::tseitin_expander(n, 0xC0FFEE + n as u64);

        // Lower half — resolution width, certified by the closed set.
        let wstar = min_res_width_clauses(cnf.num_vars, &cnf.clauses, WidthConvention::Strict)
            .expect("Tseitin expanders are UNSAT");
        let closed = resolution_width_closure(&cnf.clauses, wstar - 1, WidthConvention::Strict);
        assert!(
            check_res_width_lower_bound(&cnf.clauses, wstar - 1, WidthConvention::Strict, &closed),
            "tseitin({n}): res-width > {} certified",
            wstar - 1
        );

        // Upper half — the GF(2) refutation: a subset of equations XOR-ing to 0 = 1, re-checked.
        let XorOutcome::Unsat(refutation) = xorsat::solve(&eqs, cnf.num_vars) else {
            panic!("tseitin({n}): the parity system refutes");
        };
        assert!(
            xorsat::is_refutation(&eqs, cnf.num_vars, &refutation),
            "tseitin({n}): the GF(2) refutation re-checks"
        );
        atlas.push(format!(
            "tseitin({n}): res-width = {wstar} | GF(2) refutation = {} equations",
            refutation.len()
        ));
    }

    // ── Row 3: modular counting — the characteristic-mismatch marquee ───────────────────────────
    for n in [7usize, 8] {
        let (cnf, _) = families::mod_counting(n, 3);
        let groups = families::mod_counting_groups(n, 3);
        let gens = exactly_one_linear_generators(&groups);

        // Lower half — GF(2) NS-degree ≥ 3 on the linear encoding, certified by a re-checked
        // pseudo-expectation. (The stabilized collapsed dual extends the n ≡ 3 (mod 4) class to
        // EVERY scale — `fixed_degree_symmetric_ns_verdict_for_php_is_decided_for_all_m_by_finite_computation`.)
        let w = ns_lower_bound_witness_polys(cnf.num_vars, &gens, 2)
            .expect("Count_3 carries a degree-2 pseudo-expectation");
        assert!(
            check_ns_lower_bound_polys(cnf.num_vars, &gens, 2, &w),
            "Count_3({n}): GF(2) NS-degree ≥ 3 certified"
        );

        // Upper half — the GF(3) refutation: the point equations Σ_{e∋i} x_e ≡ 1 (mod 3) are
        // linearly inconsistent (summing all counts each block 3 ≡ 0 times against n ≢ 0), and the
        // dependency combination re-checks. Transfer: a CNF model IS a 0/1 solution of the system
        // (exactly-one per point), so the linear refutation refutes the CNF.
        let equations: Vec<ModpEquation> = groups
            .iter()
            .map(|g| ModpEquation::new(g.iter().map(|&v| (v as usize, 1u64)).collect::<Vec<_>>(), 1))
            .collect();
        let ModpOutcome::Unsat(combo) = modp::solve(&equations, cnf.num_vars, 3) else {
            panic!("Count_3({n}): the GF(3) system refutes");
        };
        assert!(
            modp::is_refutation(&equations, cnf.num_vars, 3, &combo),
            "Count_3({n}): the GF(3) refutation re-checks"
        );
        atlas.push(format!(
            "Count_3({n}): GF(2) NS-degree ≥ 3 | GF(3) refutation = {} equations — the char mismatch",
            combo.len()
        ));
    }
    // The SAT control for the transfer direction: at n = 6 (3 | 6) the GF(3) system is consistent.
    {
        let groups = families::mod_counting_groups(6, 3);
        let (cnf6, _) = families::mod_counting(6, 3);
        let equations: Vec<ModpEquation> = groups
            .iter()
            .map(|g| ModpEquation::new(g.iter().map(|&v| (v as usize, 1u64)).collect::<Vec<_>>(), 1))
            .collect();
        assert!(
            matches!(modp::solve(&equations, cnf6.num_vars, 3), ModpOutcome::Sat(_)),
            "Count_3(6): the GF(3) system is consistent exactly when the formula is satisfiable"
        );
    }

    // ── The dense degenerate regime as an atlas footnote: Count_3(4,5) fall at degree 2 ─────────
    for n in [4usize, 5] {
        let (cnf, _) = families::mod_counting(n, 3);
        let gens = exactly_one_linear_generators(&families::mod_counting_groups(n, 3));
        assert!(
            logicaffeine_proof::polycalc::ns_refutes_polys(cnf.num_vars, &gens, 2),
            "Count_3({n}): the dense regime refutes at degree 2"
        );
    }
    let _ = monomials_up_to_degree(4, 2); // linkage sanity for the bounded-basis API used above

    for line in &atlas {
        eprintln!("ATLAS | {line}");
    }
    assert_eq!(atlas.len(), 6, "every row of the atlas is present and two-sided");
}

/// The atlas rows carry literal Lit-level data — pin the clause conventions the rows rely on
/// (all-positive covering clauses in `mod_counting`, negative pairs in PHP) so a families refactor
/// cannot silently rot the atlas.
#[test]
fn atlas_family_layouts_are_pinned() {
    let (php3, _) = families::php(3);
    assert!(php3.clauses.iter().any(|c| c.iter().all(|l| l.is_positive())), "PHP has pigeon rows");
    assert!(
        php3.clauses.iter().any(|c| c.len() == 2 && c.iter().all(|l| !l.is_positive())),
        "PHP has hole AMO pairs"
    );
    let (cnt, _) = families::mod_counting(5, 3);
    let groups = families::mod_counting_groups(5, 3);
    assert_eq!(groups.len(), 5, "one covering group per point");
    let edges = families::mod_counting_edges(5, 3);
    assert_eq!(edges.len(), cnt.num_vars, "one edge per variable, in variable order");
    let lit_ok = cnt.clauses.iter().all(|c| c.iter().all(|l| (l.var() as usize) < cnt.num_vars));
    assert!(lit_ok, "every literal indexes a real variable");
    let _ = Lit::pos(0);
}
