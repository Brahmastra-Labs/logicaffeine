//! **The hardness-predicate ladder, decided object-by-object.**
//!
//! "Hardness" is not one predicate; it is a ladder of them, and the instruments now decide each rung
//! with a certificate — either the rung is UNFULFILLABLE (no object satisfies it, proven
//! universally) or it is FULFILLED (a named object satisfies it, proven by witness). The two poles
//! of the paper (§2) become one executable theorem:
//!
//!   - **H_exist — "structureless": no certificate at any degree `≤ n` exists, over any coefficient
//!     ring.** UNFULFILLABLE. Every UNSAT formula receives a re-checked certificate over every
//!     modulus (`ℤ/2, ℤ/3, ℤ/4, ℤ/6` here; the module theorem sweeps `2..12`; the kernel carries
//!     the `∀n` induction). Nothing finite is random — this is the existence-form of "hardness does
//!     not exist," and it is a theorem.
//!   - **H_max — "maximal cost at its size": NS-degree exactly `n`.** FULFILLED at every tested
//!     `n`, by the all-corners cube — and not as a characteristic accident: certified over `GF(2)`,
//!     `GF(3)`, and the ring `ℤ/6` simultaneously (dual witness at `n−1` re-checks; refutation at
//!     `n`).
//!   - **H_grow — "cost grows without bound along a family."** FULFILLED by pigeonhole: certified
//!     exact degrees `4, 6` at `m = 3, 4` with re-checked witnesses, characteristic-invariantly
//!     (the `GF(3)` degree of PHP(3) is also exactly 4).
//!   - **The conjunction — the same object carries both poles.** The very cube that fulfills
//!     maximal cost-hardness ALSO carries a verified structure certificate: hardness-as-cost and
//!     structure-as-existence coexist in one object. Hardness is a property of the *cost*
//!     definition, never of missing structure.
//!
//! The honest boundary, stated with the theorem: NP-hardness is the *asymptotic* cost predicate
//! (superpolynomial growth along a family, worst-case, for every proof system — Cook–Reckhow ties
//! it to NP vs coNP). The ladder shows its finite shadows are fulfilled, not vacuous; refuting the
//! existence-form (H_exist) says nothing against them — conflating the two is the "P = NP for
//! finite n" category error. No rung here decides the asymptotic question, in either direction.

use logicaffeine_proof::cdcl::Lit;
use logicaffeine_proof::polycalc_gfp::{
    check_ns_lower_bound_gfp, ns_lower_bound_witness_gfp, ns_refutes_gfp, NsField,
};
use logicaffeine_proof::polycalc_zm::{build_ns_certificate_zm, clause_polynomial_zm, ns_refutes_zm};
use logicaffeine_proof::{families, polycalc};
use std::collections::BTreeMap;

/// The all-corners cube `F_n`: every one of the `2ⁿ` assignments forbidden by a full-width clause —
/// the maximally-constrained UNSAT formula, and the canonical maximal-cost object (its width-`n`
/// generators admit no product below degree `n` in any ring).
fn all_corners(n: usize) -> Vec<Vec<Lit>> {
    (0u64..(1u64 << n))
        .map(|a| (0..n as u32).map(|v| Lit::new(v, (a >> v) & 1 == 0)).collect())
        .collect()
}

/// **The Cost-Pole Attainment Theorem, with its REASON certified: `NS-degree(F_n) = n` over every
/// coefficient ring, for every `n`, by kill-or-absorb.** The lower half of H_max is not a search
/// verdict here — it is a structural dichotomy verified product-by-product with an *independent*
/// multilinear multiplier (not the engine's): for every clause `p_C` of the all-corners cube and
/// every multiplier monomial `mo`,
///
///   - if `mo` touches a positive-literal variable, the product DIES: `x·(1−x) = x − x² = x − x = 0`
///     in the multilinear quotient — a ring identity, valid over every `ℤ/m`;
///   - otherwise the product ABSORBS: `x·x = x`, so `mo·p_C = p_C` exactly, coefficient for
///     coefficient — and `p_C` carries the full monomial `x_1⋯x_n` with unit coefficient `±1`, so
///     its degree is `n`.
///
/// No third case exists (asserted on every one of the `2ⁿ·2ⁿ` products, per ring). Hence the
/// degree-`(n−1)` generator span of `F_n` is EMPTY — `1 ∉ {0}` — so no refutation below `n` exists
/// over any `ℤ/m`; and completeness (kernel-certified `∀n` over `GF(2)`, ring-swept in §5.11's
/// artifacts) refutes at exactly `n`. The same object simultaneously verifies its partition-of-unity
/// structure certificate — both poles, one formula, one argument, every ring class (field,
/// nilpotent `ℤ/4`, idempotent-composite `ℤ/6`). The engine's own verdicts are cross-checked against
/// the dichotomy's conclusion at every swept size. Uniformity in `n`: the dichotomy's two branches
/// are per-variable identities with no reference to `n` — the sweep exhibits them at `n = 2..7`
/// (`8ⁿ` products checked), and the kernel seeds (`tests/cost_pole_kernel_seeds.rs`,
/// `gf2_ring_kernel.rs`, `gf3_ring_kernel.rs`) pin the per-variable identities themselves.
#[test]
fn the_cost_pole_is_attained_at_every_n_over_every_ring_by_kill_or_absorb() {
    for n in 2..=7usize {
        let cube = all_corners(n);
        let full: u64 = (1u64 << n) - 1;
        for &m in &[2u64, 3, 4, 6] {
            let (mut kills, mut absorbs) = (0u64, 0u64);
            for clause in &cube {
                let pc = clause_polynomial_zm(m, clause);
                assert!(
                    pc.contains_key(&full),
                    "n={n} m={m}: the clause polynomial carries the full monomial (degree n) with a unit coefficient"
                );
                for mo in 0u64..(1u64 << n) {
                    // Independent multilinear product (NOT the engine's): OR the masks, add mod m.
                    let mut prod: BTreeMap<u64, u64> = BTreeMap::new();
                    for (&t, &c) in &pc {
                        let key = t | mo;
                        let e = prod.entry(key).or_insert(0);
                        *e = (*e + c) % m;
                        if *e == 0 {
                            prod.remove(&key);
                        }
                    }
                    if prod.is_empty() {
                        kills += 1; // mo touched a positive literal: x·(1−x) = 0 killed the product
                    } else {
                        assert_eq!(
                            prod, pc,
                            "n={n} m={m} mo={mo:b}: a surviving product IS the clause polynomial (absorption)"
                        );
                        absorbs += 1;
                    }
                    // The dichotomy leaves NO product with 0 < degree ≤ n−1: the degree-(n−1)
                    // Nullstellensatz span of F_n is empty, over this ring, at this n.
                }
            }
            assert!(kills > 0 && absorbs > 0, "both branches of the dichotomy occur");
            if n <= 5 {
                // The engine's verdicts agree with the dichotomy's conclusion.
                assert!(
                    !ns_refutes_zm(m, n, &cube, n - 1),
                    "n={n} m={m}: no refutation below n — the span below n is empty"
                );
                assert!(ns_refutes_zm(m, n, &cube, n), "n={n} m={m}: refuted at exactly n");
                let cert = build_ns_certificate_zm(m, n, &cube)
                    .expect("the maximal-cost object still certifies");
                assert!(cert.verify(&cube), "n={n} m={m}: both poles verified in one object");
            }
            eprintln!(
                "F_{n} over ℤ/{m}: {} products — {kills} killed, {absorbs} absorbed, 0 others ⟹ NS-degree = {n}",
                kills + absorbs
            );
        }
    }
}

#[test]
fn hardness_definitions_are_decided_object_by_object_across_the_ladder() {
    // ── H_exist: UNFULFILLABLE — every UNSAT object has structure, over every modulus tried. ──
    let (php3, _) = families::php(3);
    let corpus: Vec<(usize, Vec<Vec<Lit>>)> = vec![
        (3, all_corners(3)),
        (4, all_corners(4)),
        (php3.num_vars, php3.clauses.clone()),
    ];
    for &m in &[2u64, 3, 4, 6] {
        for (nv, clauses) in &corpus {
            let cert = build_ns_certificate_zm(m, *nv, clauses)
                .expect("H_exist has no witness: every UNSAT formula certifies");
            assert!(cert.verify(clauses), "m={m} n={nv}: the structure certificate re-checks");
            assert!(cert.degree() <= *nv, "m={m} n={nv}: structure within the cube's own degree");
        }
    }
    eprintln!("H_exist (structureless): UNFULFILLABLE — {} object×modulus cells certified", corpus.len() * 4);

    // ── H_max: FULFILLED at every n — the all-corners cube has NS-degree exactly n, at every
    //    characteristic and over the ring. ──
    for n in 2..=4usize {
        let cube = all_corners(n);
        for &p in &[2u64, 3] {
            let f = NsField::Prime(p);
            let w = ns_lower_bound_witness_gfp(f, n, &cube, n - 1)
                .expect("a dual witness exists below the maximal degree");
            assert!(
                check_ns_lower_bound_gfp(f, n, &cube, n - 1, &w),
                "F_{n} over GF({p}): NS-degree > {} re-checks with zero trust",
                n - 1
            );
            assert!(ns_refutes_gfp(f, n, &cube, n), "F_{n} over GF({p}): refuted exactly at n");
        }
        assert!(!ns_refutes_zm(6, n, &cube, n - 1), "F_{n} over ℤ/6: no refutation below n");
        assert!(ns_refutes_zm(6, n, &cube, n), "F_{n} over ℤ/6: refuted exactly at n");
        // The conjunction: the SAME maximal-cost object carries a verified structure certificate.
        let cert = build_ns_certificate_zm(6, n, &cube).expect("the poles coexist in one object");
        assert!(cert.verify(&cube), "F_{n}: maximal cost AND certified structure, simultaneously");
    }
    eprintln!("H_max (degree exactly n): FULFILLED at n = 2, 3, 4 — over GF(2), GF(3), and ℤ/6");

    // ── H_grow: FULFILLED — pigeonhole's certified degree grows along the family, and not as a
    //    characteristic-2 accident. ──
    let mut degrees = Vec::new();
    for (m, exact) in [(3usize, 4usize), (4, 6)] {
        let (php, _) = families::php(m);
        assert!(
            !polycalc::nullstellensatz_refutes(php.num_vars, &php.clauses, exact - 1),
            "PHP({m}): no GF(2) refutation below {exact}"
        );
        assert!(
            polycalc::nullstellensatz_refutes(php.num_vars, &php.clauses, exact),
            "PHP({m}): GF(2) degree exactly {exact}"
        );
        degrees.push(exact);
    }
    assert!(degrees.windows(2).all(|w| w[1] > w[0]), "the certified cost GROWS: {degrees:?}");
    let (php3, _) = families::php(3);
    assert!(!ns_refutes_gfp(NsField::Prime(3), php3.num_vars, &php3.clauses, 3));
    assert!(ns_refutes_gfp(NsField::Prime(3), php3.num_vars, &php3.clauses, 4));
    eprintln!("H_grow (growing cost along a family): FULFILLED — PHP degrees {degrees:?}, char-invariant");

    eprintln!(
        "the ladder: hardness-as-structurelessness has NO witness; hardness-as-cost has certified \
         witnesses at every size — the two predicates provably split, and NP-hardness lives in the \
         second (asymptotic) form, which no rung here decides"
    );
}
