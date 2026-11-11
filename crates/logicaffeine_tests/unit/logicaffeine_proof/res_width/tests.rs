use super::*;
use crate::hypercube::{min_resolution_width, minimal_cover_orbits, Subcube};

/// **The clause-mask closure agrees with the geometric oracle.** `hypercube::min_resolution_width`
/// computes the width on subcube covers (`Subcube::resolve` — the same single-pivot rule seen from the
/// blocker side). On every minimal-UNSAT orbit representative at `n ≤ 3`, the mask-clause closure
/// under [`WidthConvention::Strict`] must reproduce that width exactly; and on the SAT side, no width
/// ever refutes.
#[test]
fn width_closure_matches_the_subcube_resolution_width_on_every_minimal_cover() {
    for n in 1..=3usize {
        let covers = minimal_cover_orbits(n);
        assert!(!covers.is_empty(), "n={n}: there are minimal UNSAT covers to compare on");
        for cover in covers {
            let clauses: Vec<Vec<Lit>> = cover
                .blockers
                .iter()
                .map(|b: &Subcube| {
                    b.clause_literals()
                        .into_iter()
                        .map(|(v, positive)| Lit::new(v as u32, positive))
                        .collect()
                })
                .collect();
            let geometric = min_resolution_width(&cover).expect("a minimal cover is UNSAT");
            let ours = min_res_width_clauses(n, &clauses, WidthConvention::Strict);
            assert_eq!(
                ours,
                Some(geometric),
                "n={n}: mask-clause closure width = subcube resolution width on the orbit representative"
            );
        }
    }
    // SAT ⟹ no refutation at any width, under either convention.
    let sat = vec![vec![Lit::pos(0), Lit::pos(1)], vec![Lit::neg(0), Lit::pos(2)]];
    for conv in [WidthConvention::Strict, WidthConvention::WideAxioms] {
        assert_eq!(min_res_width_clauses(3, &sat, conv), None, "a satisfiable formula has no width");
    }
}

/// **The closed clause set is a re-checkable lower-bound certificate.** For UNSAT formulas with
/// measured minimum width `w*`: at every `w < w*` the closure (without `⊥`) passes the independent
/// checker — certifying `res-width > w` with zero trust in the producer — and the checker fails
/// closed on every mutilation: a set containing `⊥`, a set missing an axiom, a set with a closure
/// hole, and a set smuggling a tautology.
#[test]
fn resolution_width_lower_bounds_are_certified_by_the_closed_clause_set() {
    let p = |v: u32| Lit::pos(v);
    let q = |v: u32| Lit::neg(v);
    // The transitive-XOR core (min width 3 among Strict derivations) and PHP(3).
    let xor_core = vec![
        vec![q(0), p(1)], vec![p(0), q(1)],
        vec![q(1), p(2)], vec![p(1), q(2)],
        vec![p(0), p(2)], vec![q(0), q(2)],
    ];
    let (php3, _) = crate::families::php(3);
    for (nv, clauses) in [(3usize, xor_core), (php3.num_vars, php3.clauses)] {
        for conv in [WidthConvention::Strict, WidthConvention::WideAxioms] {
            let wstar = min_res_width_clauses(nv, &clauses, conv)
                .expect("UNSAT ⟹ some width refutes");
            for w in 0..wstar {
                let closed = resolution_width_closure(&clauses, w, conv);
                assert!(
                    !closed.contains(&(0, 0)),
                    "below the minimum width the closure must not refute (w={w})"
                );
                assert!(
                    check_res_width_lower_bound(&clauses, w, conv, &closed),
                    "the closure certifies res-width > {w}"
                );
                // Fail-closed on mutilations.
                let mut with_empty = closed.clone();
                with_empty.insert((0, 0));
                assert!(
                    !check_res_width_lower_bound(&clauses, w, conv, &with_empty),
                    "a set containing ⊥ is no lower bound"
                );
                if let Some(&first) = closed.iter().next() {
                    let mut missing = closed.clone();
                    missing.remove(&first);
                    assert!(
                        !check_res_width_lower_bound(&clauses, w, conv, &missing),
                        "a set with a hole (missing axiom or closure gap) must be rejected"
                    );
                }
                let mut with_taut = closed.clone();
                with_taut.insert((1, 1));
                assert!(
                    !check_res_width_lower_bound(&clauses, w, conv, &with_taut),
                    "a tautology-smuggling set must be rejected"
                );
            }
            // At w*, the closure genuinely refutes — the two-sidedness of the measure.
            assert!(width_refutes(&clauses, wstar, conv), "the closure refutes at w*");
        }
    }
}

/// **Tseitin expanders carry certified resolution-width lower bounds.** Parity on a 3-regular
/// expander is the classical width-hard family (Ben-Sasson–Wigderson: width `Ω(expansion)`). We pin
/// the exact minimum width on small expanders under both conventions (they agree — Tseitin axioms
/// are already narrow, width 3) and certify each `w < w*` with the re-checked closed-set
/// certificate. This is the resolution half of Tseitin's separation (the GF(2)-rank refutation is its
/// polynomial upper half).
#[test]
fn tseitin_expander_has_certified_resolution_width_lower_bounds() {
    for n in [6usize, 8] {
        let (_eqs, cnf, verdict) = crate::families::tseitin_expander(n, 0xC0FFEE + n as u64);
        assert!(matches!(verdict, crate::families::ExpectedVerdict::Unsat));
        let ws = min_res_width_clauses(cnf.num_vars, &cnf.clauses, WidthConvention::Strict)
            .expect("Tseitin expanders are UNSAT");
        let ww = min_res_width_clauses(cnf.num_vars, &cnf.clauses, WidthConvention::WideAxioms)
            .expect("Tseitin expanders are UNSAT");
        assert_eq!(ws, ww, "n={n}: width-3 axioms ⟹ the conventions coincide");
        assert!(ws > 3, "n={n}: the width exceeds the axiom width — a genuine, non-axiom bound");
        for w in [ws - 1] {
            let closed = resolution_width_closure(&cnf.clauses, w, WidthConvention::Strict);
            assert!(
                check_res_width_lower_bound(&cnf.clauses, w, WidthConvention::Strict, &closed),
                "n={n}: certified res-width > {w}"
            );
        }
        eprintln!("tseitin({n}): {} vars, certified min res-width = {ws}", cnf.num_vars);
    }
}

/// **Pigeonhole's certified width beside its certified NS degree and SR upper bound.** Under
/// [`WidthConvention::WideAxioms`] (the convention that makes wide-axiom width non-trivial) PHP(m)'s
/// minimum refutation width is measured exactly and its lower half certified by the closed set —
/// alongside the certified NS degree `2(m−1)` (dual witness) and the `m(m−1)/2`-step SR upper bound,
/// this is the third machine-certified coordinate of the same family. The width grows with `m`.
#[test]
fn php_resolution_width_certificate_completes_the_three_system_row() {
    let mut widths = Vec::new();
    for m in [3usize, 4] {
        let (php, _) = crate::families::php(m);
        let w = min_res_width_clauses(php.num_vars, &php.clauses, WidthConvention::WideAxioms)
            .expect("PHP is UNSAT");
        let closed = resolution_width_closure(&php.clauses, w - 1, WidthConvention::WideAxioms);
        assert!(
            check_res_width_lower_bound(&php.clauses, w - 1, WidthConvention::WideAxioms, &closed),
            "PHP({m}): certified res-width > {}",
            w - 1
        );
        eprintln!("PHP({m}): certified min res-width (wide axioms) = {w}");
        widths.push(w);
    }
    assert!(widths[1] > widths[0], "the pigeonhole width grows with m: {widths:?}");
}
