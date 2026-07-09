//! **The hardness-retreat ladder: every hardness we can exhibit dissolves one rung up — both sides
//! certified.**
//!
//! Hardness of a *family* for a *fixed* proof system is real and exhibitable — we exhibit it
//! ourselves, with zero-trust certificates (dual witnesses, closed clause sets). The pattern this
//! test locks: **in the entire certified corpus, no family is hard at its top rung.** Every
//! certified lower bound comes with a certified dissolution in a stronger or characteristic-shifted
//! system:
//!
//! | family | certified HARD for | certified EASY for |
//! |---|---|---|
//! | `PHP(m)`     | resolution width (closed set); `GF(2)` NS degree (dual witness) | SR — `m(m−1)/2` steps (PR/SR checker) |
//! | Tseitin      | resolution width (closed set) | `GF(2)` Gaussian (`xorsat::is_refutation`) |
//! | `Count_3(n)` | `GF(2)` NS degree (dual witness, linear encoding) | `GF(3)` Gaussian (`modp::is_refutation`) |
//!
//! This is the machine-certified face of the honest observation behind "NP-hardness has never been
//! pointed at": hardness-for-a-fixed-system is everywhere exhibitable (rows above), while
//! hardness-for-ALL-systems has never been exhibited for any family, by anyone — every exhibit ever
//! named has dissolved one rung up. Whether the retreat continues forever is exactly NP vs coNP
//! (`work/PAPER.md` §8.2); the corpus's one honestly-open cell — random 3-CNF at the SR rung — is a
//! measurement (`ef_class_probe.rs`), not a certificate, and is labeled so.

use logicaffeine_proof::families;
use logicaffeine_proof::modp::{self, ModpEquation, ModpOutcome};
use logicaffeine_proof::polycalc::{
    check_ns_lower_bound, check_ns_lower_bound_polys, exactly_one_linear_generators,
    ns_lower_bound_witness_polys, php_is_hole_injective,
};
use logicaffeine_proof::pr::check_pr_refutation;
use logicaffeine_proof::res_width::{
    check_res_width_lower_bound, min_res_width_clauses, resolution_width_closure, WidthConvention,
};
use logicaffeine_proof::sym_certify::heule_php_refutation;
use logicaffeine_proof::xorsat::{self, XorOutcome};

#[test]
fn every_exhibited_hardness_dissolves_one_rung_up() {
    let mut ladder: Vec<String> = Vec::new();

    // ── PHP: hard for resolution-width AND for GF(2)-NS; dissolved by SR ────────────────────────
    for m in [3usize, 4] {
        let (php, _) = families::php(m);
        let holes = m - 1;
        // HARD, exhibit 1: resolution width (wide-axiom convention), certified by the closed set.
        let w = min_res_width_clauses(php.num_vars, &php.clauses, WidthConvention::WideAxioms)
            .expect("PHP is UNSAT");
        let closed = resolution_width_closure(&php.clauses, w - 1, WidthConvention::WideAxioms);
        assert!(
            check_res_width_lower_bound(&php.clauses, w - 1, WidthConvention::WideAxioms, &closed),
            "PHP({m}): exhibited hard — res-width > {}",
            w - 1
        );
        // HARD, exhibit 2: GF(2) NS degree, certified by the hole-injective dual witness.
        let d = 2 * holes - 1;
        let witness: Vec<u64> = (0u64..(1u64 << php.num_vars))
            .filter(|&mo| mo.count_ones() as usize <= d && php_is_hole_injective(mo, holes))
            .collect();
        assert!(
            check_ns_lower_bound(php.num_vars, &php.clauses, d, &witness),
            "PHP({m}): exhibited hard — NS-degree ≥ {}",
            2 * holes
        );
        // DISSOLVED: the SR (EF-class) refutation, quadratic with its own clock, re-checked.
        let sr = heule_php_refutation(m);
        assert!(sr.refuted && sr.sbp_clauses == m * (m - 1) / 2);
        assert!(
            check_pr_refutation(php.num_vars, &php.clauses, &sr.steps),
            "PHP({m}): the hardness dissolves — SR refutes in {} steps",
            sr.sbp_clauses
        );
        ladder.push(format!(
            "PHP({m}): HARD[res-width>{}, NS≥{}] → EASY[SR, {} steps]",
            w - 1,
            2 * holes,
            sr.sbp_clauses
        ));
    }

    // ── Tseitin: hard for resolution width; dissolved by GF(2) Gaussian ─────────────────────────
    for n in [6usize, 8] {
        let (eqs, cnf, _) = families::tseitin_expander(n, 0xC0FFEE + n as u64);
        let w = min_res_width_clauses(cnf.num_vars, &cnf.clauses, WidthConvention::Strict)
            .expect("UNSAT");
        let closed = resolution_width_closure(&cnf.clauses, w - 1, WidthConvention::Strict);
        assert!(
            check_res_width_lower_bound(&cnf.clauses, w - 1, WidthConvention::Strict, &closed),
            "tseitin({n}): exhibited hard — res-width > {}",
            w - 1
        );
        let XorOutcome::Unsat(refutation) = xorsat::solve(&eqs, cnf.num_vars) else {
            panic!("tseitin({n}) refutes over GF(2)");
        };
        assert!(
            xorsat::is_refutation(&eqs, cnf.num_vars, &refutation),
            "tseitin({n}): the hardness dissolves — GF(2) refutes with {} equations",
            refutation.len()
        );
        ladder.push(format!(
            "tseitin({n}): HARD[res-width>{}] → EASY[GF(2), {} eqs]",
            w - 1,
            refutation.len()
        ));
    }

    // ── Count_3: hard for GF(2)-NS; dissolved by GF(3) Gaussian — the characteristic retreat ───
    for n in [7usize, 8] {
        let (cnf, _) = families::mod_counting(n, 3);
        let groups = families::mod_counting_groups(n, 3);
        let gens = exactly_one_linear_generators(&groups);
        let w = ns_lower_bound_witness_polys(cnf.num_vars, &gens, 2).expect("degree-2 witness");
        assert!(
            check_ns_lower_bound_polys(cnf.num_vars, &gens, 2, &w),
            "Count_3({n}): exhibited hard — GF(2) NS-degree ≥ 3"
        );
        let equations: Vec<ModpEquation> = groups
            .iter()
            .map(|g| ModpEquation::new(g.iter().map(|&v| (v as usize, 1u64)).collect::<Vec<_>>(), 1))
            .collect();
        let ModpOutcome::Unsat(combo) = modp::solve(&equations, cnf.num_vars, 3) else {
            panic!("Count_3({n}) refutes over GF(3)");
        };
        assert!(
            modp::is_refutation(&equations, cnf.num_vars, 3, &combo),
            "Count_3({n}): the hardness dissolves — GF(3) refutes with {} equations",
            combo.len()
        );
        ladder.push(format!(
            "Count_3({n}): HARD[GF(2)-NS≥3] → EASY[GF(3), {} eqs]",
            combo.len()
        ));
    }

    for line in &ladder {
        eprintln!("RETREAT | {line}");
    }
    // The lock: every exhibited hardness in the corpus carries its certified dissolution.
    assert_eq!(ladder.len(), 6, "every hard family in the corpus dissolves one rung up");
}
