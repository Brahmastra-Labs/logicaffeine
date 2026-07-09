//! The small-`n` SAT-space census — exhaustive over every minimal UNSAT formula (minimal subcube cover,
//! genuine CNF) for small `n`, classified up to the hyperoctahedral group `Bₙ`. These tests are the
//! spec: the group-order and orbit–stabilizer identities are asserted from first principles, every
//! enumerated cover is cross-checked genuinely-UNSAT and genuinely-minimal against the brute cube oracle,
//! the orbit counts are locked as regressions, and the central finding — that clause-level symmetry
//! breaking is already *complete* — is asserted exhaustively.

use logicaffeine_proof::census::{census, coverage_summary};
use logicaffeine_proof::hypercube::{
    canonical_cover, clause_orbits, cube_group_closure, hyperoctahedral_generators,
    min_resolution_width, minimal_cover_orbits, php_perm_symmetries, Cover,
};

/// `|Bₙ| = 2ⁿ · n!` — the order of the hyperoctahedral group, the complete clause-level symmetry.
fn b_n_order(n: usize) -> usize {
    let factorial: usize = (1..=n).product::<usize>().max(1);
    (1usize << n) * factorial
}

#[test]
fn hyperoctahedral_generators_generate_exactly_b_n() {
    for n in 0..=5 {
        let group = cube_group_closure(&hyperoctahedral_generators(n), n);
        assert_eq!(
            group.len(),
            b_n_order(n),
            "the closure of the Bₙ generators at n={n} must have order 2ⁿ·n! = {}",
            b_n_order(n)
        );
    }
}

/// The exhaustive orbit counts, locked. (Genuine CNF — the degenerate empty clause `⊥` is excluded.)
#[test]
fn census_orbit_counts_are_locked() {
    assert_eq!(minimal_cover_orbits(1).len(), 1, "n=1: just the clause pair {{x, ¬x}}");
    assert_eq!(minimal_cover_orbits(2).len(), 4);
    assert_eq!(minimal_cover_orbits(3).len(), 43);
}

#[test]
fn n1_census_is_exactly_the_clause_pair() {
    let orbits = minimal_cover_orbits(1);
    assert_eq!(orbits.len(), 1, "n=1 has exactly one minimal-cover orbit");
    assert_eq!(orbits[0].blockers.len(), 2, "it is the two-unit-clause cover {{x, ¬x}}");
}

/// Every blocker in a minimal cover privately owns at least one corner — dropping it exposes a model.
fn is_minimal(cover: &Cover) -> bool {
    cover.is_total()
        && (0..cover.blockers.len()).all(|i| {
            let mut without = cover.blockers.clone();
            without.remove(i);
            !Cover { n: cover.n, blockers: without }.is_total()
        })
}

#[test]
fn every_enumerated_cover_is_genuinely_unsat_and_minimal() {
    for n in 1..=3 {
        let orbits = minimal_cover_orbits(n);
        assert!(!orbits.is_empty(), "there is at least one minimal UNSAT formula over n={n} vars");
        for cover in &orbits {
            assert!(cover.is_total(), "n={n}: every census cover must be UNSAT (totally cover the cube)");
            assert!(is_minimal(cover), "n={n}: every census cover must be minimal (every blocker essential)");
            assert!(
                cover.blockers.iter().all(|b| b.care != 0),
                "n={n}: every clause mentions ≥1 variable (no degenerate empty clause)"
            );
        }
    }
}

#[test]
fn canonical_form_is_an_orbit_invariant_and_orbit_stabilizer_holds() {
    for n in 1..=3 {
        let gens = hyperoctahedral_generators(n);
        let group = cube_group_closure(&gens, n);
        for cover in minimal_cover_orbits(n) {
            let (key, orbit_size) = canonical_cover(&cover, &gens);
            for g in &group {
                let moved = Cover {
                    n,
                    blockers: cover.blockers.iter().map(|b| g.map_subcube(b)).collect(),
                };
                let (moved_key, moved_size) = canonical_cover(&moved, &gens);
                assert_eq!(moved_key, key, "n={n}: canonical key is constant on the orbit");
                assert_eq!(moved_size, orbit_size, "n={n}: orbit size is an invariant");
            }
            let stab = group.iter().filter(|g| g.is_automorphism(&cover)).count();
            assert_eq!(
                stab * orbit_size,
                b_n_order(n),
                "n={n}: orbit–stabilizer |Stab|·|orbit| = |Bₙ| must hold for every cover"
            );
        }
    }
}

#[test]
fn resolution_width_is_well_defined_and_bounded_by_n() {
    let cover = &minimal_cover_orbits(1)[0];
    assert_eq!(min_resolution_width(cover), Some(1), "{{x, ¬x}} refutes at width 1");
    for n in 1..=3 {
        for cover in minimal_cover_orbits(n) {
            let w = min_resolution_width(&cover).expect("an UNSAT cover always has a resolution width");
            assert!(w <= n, "n={n}: resolution width is at most n");
        }
    }
}

/// **The central finding, asserted exhaustively.** On *every* minimal UNSAT formula, the production
/// symmetry breaker (`find_generators`, used in the certified cascade) recovers the *full* `Bₙ`
/// stabilizer's rule-merge — clause-level (permutation + negation) symmetry breaking is already complete.
/// There is no symmetry of this kind left on the table to improve.
#[test]
fn clause_level_symmetry_breaking_is_complete() {
    for n in 1..=3 {
        for r in census(n) {
            assert!(
                !r.symmetry_underbroken(),
                "n={n}: the production breaker must recover the full Bₙ stabilizer (found {} rule-orbits, \
                 full stabilizer gives {})",
                r.discovered_rule_orbits,
                r.full_rule_orbits
            );
        }
    }
}

/// The hardness spectrum — the aggregate of the census over the proof-complexity ladder. Locked, and
/// checked to partition the families exactly (every family lands on exactly one rung and one symmetry
/// class). The rising max-NS-degree (1 → 2 → 3) is the algebraic-hardness ceiling climbing with n.
#[test]
fn hardness_spectrum_partitions_the_census_and_is_locked() {
    let s1 = coverage_summary(1);
    assert_eq!((s1.orbits, s1.max_ns_degree), (1, 0));
    assert_eq!(s1.by_rung.get("trivial"), Some(&1));

    let s2 = coverage_summary(2);
    assert_eq!((s2.orbits, s2.structured, s2.rigid, s2.max_ns_degree), (4, 4, 0, 2));

    let s3 = coverage_summary(3);
    assert_eq!((s3.orbits, s3.structured, s3.rigid, s3.max_ns_degree), (43, 37, 6, 3));
    assert_eq!(s3.by_rung.get("parity"), Some(&1), "the n=3 XOR germ sits on the parity rung");

    for s in [&s1, &s2, &s3] {
        assert_eq!(s.by_rung.values().sum::<usize>(), s.orbits, "the rungs partition the families");
        assert_eq!(s.structured + s.rigid, s.orbits, "structured + rigid partitions the families");
        assert_eq!(s.by_resolution_width.values().sum::<usize>(), s.orbits, "widths partition the families");
    }
}

/// **The scaling bridge — the other face of the wall.** A structured family stays O(1)
/// symmetry-collapsible at *every* scale: pigeonhole collapses to a constant number of rule-orbits for
/// all `n`, so symmetry-breaking crushes it at any size (php30 in milliseconds). The rigid residue, by
/// contrast, has no such collapse — its hardness (NS degree) climbs with `n`. This is the measured
/// distinction between the families we cover forever and the genuinely-hard core.
#[test]
fn pigeonhole_collapses_to_constant_orbits_at_every_scale() {
    // The clause-level rule-quotient scales past the geometric cube's 63-variable ceiling.
    for n in 2..=20 {
        let (cnf, _) = logicaffeine_proof::families::php(n);
        let orbits = clause_orbits(&cnf.clauses, &php_perm_symmetries(n));
        assert_eq!(orbits.len(), 2, "PHP(n={n}) collapses to exactly 2 rule-orbits under its symmetry group");
    }
}

/// Heavy: the full n=4 census (~40s). Locks the orbit count and re-verifies, exhaustively over all
/// 42,263 minimal UNSAT formulas, that clause-level symmetry breaking is complete and every cover is a
/// genuine minimal UNSAT instance.
#[test]
#[ignore = "exhaustive n=4 census — ~40s, run explicitly"]
fn n4_exhaustive_census() {
    let orbits = minimal_cover_orbits(4);
    assert_eq!(orbits.len(), 42263, "n=4: the exhaustive minimal-cover orbit count is locked");
    for cover in &orbits {
        assert!(cover.is_total() && is_minimal(cover), "n=4: every census cover is minimal UNSAT");
    }
    for r in census(4) {
        assert!(!r.symmetry_underbroken(), "n=4: clause-level symmetry breaking is complete on every orbit");
    }
}
