use super::*;

/// A deterministic LCG for reproducible fuzz corpora inside tests (no `rand`, no wall clock).
fn lcg(state: &mut u64) -> u64 {
    *state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    *state >> 33
}

fn eval_at(m: u64, p: &ZmPoly, a: u64) -> u64 {
    p.iter().fold(0u64, |acc, (&mo, &c)| if mo & !a == 0 { zm_add(m, acc, c) } else { acc })
}

fn falsifies(clause: &[Lit], a: u64) -> bool {
    !clause.iter().any(|l| ((a >> l.var()) & 1 == 1) == l.is_positive())
}

fn sat(num_vars: usize, clauses: &[Vec<Lit>]) -> bool {
    (0u64..(1u64 << num_vars)).any(|x| {
        clauses.iter().all(|c| c.iter().any(|l| ((x >> l.var()) & 1 != 0) == l.is_positive()))
    })
}

/// An explicit finite abelian group: element ids `0..n` with an addition table; `0` is the zero.
struct AddGroup {
    name: &'static str,
    n: usize,
    add: Vec<Vec<usize>>,
}

fn cyclic_group(n: usize, name: &'static str) -> AddGroup {
    let add = (0..n).map(|a| (0..n).map(|b| (a + b) % n).collect()).collect();
    AddGroup { name, n, add }
}

/// `ℤ/p × ℤ/q` with the pair `(x₁, x₂)` encoded as `x₁·q + x₂`.
fn product_group(p: usize, q: usize, name: &'static str) -> AddGroup {
    let n = p * q;
    let add = (0..n)
        .map(|a| {
            (0..n)
                .map(|b| ((a / q + b / q) % p) * q + ((a % q + b % q) % q))
                .collect()
        })
        .collect();
    AddGroup { name, n, add }
}

fn scalar_mul(g: &AddGroup, k: usize, x: usize) -> usize {
    (0..k).fold(0, |acc, _| g.add[acc][x])
}

fn add_order(g: &AddGroup, x: usize) -> usize {
    let mut acc = x;
    let mut k = 1;
    while acc != 0 {
        acc = g.add[acc][x];
        k += 1;
    }
    k
}

/// The first field axiom a candidate `(add, mul)` violates, or `None` if it is a field. The
/// checks re-verify even what the construction guarantees (distributivity) — zero trust in the
/// composer itself.
fn field_axiom_failure(g: &AddGroup, mul: &[Vec<usize>]) -> Option<&'static str> {
    let n = g.n;
    for a in 0..n {
        for b in 0..n {
            for c in 0..n {
                if mul[a][g.add[b][c]] != g.add[mul[a][b]][mul[a][c]]
                    || mul[g.add[a][b]][c] != g.add[mul[a][c]][mul[b][c]]
                {
                    return Some("distributivity");
                }
            }
        }
    }
    for a in 0..n {
        for b in 0..n {
            if mul[a][b] != mul[b][a] {
                return Some("commutativity");
            }
        }
    }
    for a in 0..n {
        for b in 0..n {
            for c in 0..n {
                if mul[mul[a][b]][c] != mul[a][mul[b][c]] {
                    return Some("associativity");
                }
            }
        }
    }
    let Some(e) = (0..n).find(|&e| (0..n).all(|x| mul[e][x] == x && mul[x][e] == x)) else {
        return Some("no multiplicative identity");
    };
    for a in 1..n {
        for b in 1..n {
            if mul[a][b] == 0 {
                return Some("a zero divisor");
            }
        }
    }
    if (1..n).any(|a| !(1..n).any(|b| mul[a][b] == e)) {
        return Some("a nonzero element without an inverse");
    }
    None
}

/// **Fields COMPOSE exactly at prime-power orders — shown by building them, and by exhausting
/// the alternatives.** The composer: take every abelian group of order `N` (the additive
/// pieces), and every multiplication that distributes over it — for a cyclic group,
/// bilinearity forces `a·b = (ab)·u` for a single seed `u = 1·1`; for a product group it is
/// determined by the four generator products, each constrained by `ord(eᵢ·x) | gcd(ord(eᵢ),
/// ord(x))`. Every candidate table is then judged against the full field axioms (including
/// re-verifying distributivity — zero trust in the composer). The verdicts:
///
/// - **Order 4**: zero fields on `ℤ/4` (the nilpotent kills it), and the search over
///   `ℤ/2 × ℤ/2` *builds* fields — one is checked isomorphic to the engine's `NsField::Gf4`
///   table. Order **9**: zero on `ℤ/9`, built on `ℤ/3 × ℤ/3`. Composition WORKS: this is
///   exactly how `GF(4)` and `GF(9)` come from their pieces.
/// - **Orders 6 and 10**: zero fields, exhaustively — and the obstruction is *composed into
///   every candidate*, not found by luck: on `ℤ/2 × ℤ/3` (resp. `ℤ/2 × ℤ/5`) the cross
///   products `e₁·e₂`, `e₂·e₁` have additive order dividing `gcd(2, 3) = 1`, so bilinearity
///   **forces `e₁·e₂ = 0`** — two nonzero pieces whose product is zero, in every distributive
///   multiplication whatsoever. Composition at coprime-order pieces *is* the CRT ring with its
///   idempotent zero divisors; a field of order 6 or 10 is not un-found, it is impossible, and
///   the census of candidate deaths is printed per order.
///
/// This is the classification theorem "finite fields exist exactly at prime-power orders" made
/// executable at the orders in question — the same lens that composes `GF(4)` certifies that no
/// lens composes `GF(6)`.
#[test]
fn finite_fields_compose_exactly_at_prime_power_orders() {
    let mut field_counts: std::collections::BTreeMap<(usize, &'static str), usize> =
        std::collections::BTreeMap::new();
    let mut census: std::collections::BTreeMap<(usize, &'static str), std::collections::BTreeMap<&'static str, usize>> =
        std::collections::BTreeMap::new();
    let mut gf4_iso_found = false;

    // Cyclic candidates: a·b = (ab)·u, one seed u per candidate.
    for (order, g) in [
        (4usize, cyclic_group(4, "ℤ/4")),
        (6, cyclic_group(6, "ℤ/6")),
        (9, cyclic_group(9, "ℤ/9")),
        (10, cyclic_group(10, "ℤ/10")),
    ] {
        for u in 0..g.n {
            let mul: Vec<Vec<usize>> =
                (0..g.n).map(|a| (0..g.n).map(|b| scalar_mul(&g, a * b, u)).collect()).collect();
            match field_axiom_failure(&g, &mul) {
                None => *field_counts.entry((order, g.name)).or_insert(0) += 1,
                Some(why) => {
                    *census.entry((order, g.name)).or_default().entry(why).or_insert(0) += 1
                }
            }
        }
        field_counts.entry((order, g.name)).or_insert(0);
    }

    // Product-group candidates: choose the four generator products, each respecting the
    // bilinear order constraint ord(eᵢ·eⱼ) | gcd(ord(eᵢ), ord(eⱼ)).
    for (order, p, q, name) in [
        (4usize, 2usize, 2usize, "ℤ/2×ℤ/2"),
        (6, 2, 3, "ℤ/2×ℤ/3"),
        (9, 3, 3, "ℤ/3×ℤ/3"),
        (10, 2, 5, "ℤ/2×ℤ/5"),
    ] {
        let g = product_group(p, q, name);
        let allowed = |bound: usize| -> Vec<usize> {
            (0..g.n).filter(|&x| x == 0 || bound % add_order(&g, x) == 0).collect()
        };
        let gpq = gcd(p as u64, q as u64) as usize;
        let (c11, c12, c21, c22) = (allowed(p), allowed(gpq), allowed(gpq), allowed(q));
        // The coprime-pieces forcing: when gcd(p, q) = 1, the cross products CANNOT be nonzero.
        if gpq == 1 {
            assert_eq!(c12, vec![0], "{name}: bilinearity forces e1·e2 = 0 — the composed zero divisor");
            assert_eq!(c21, vec![0], "{name}: bilinearity forces e2·e1 = 0");
        }
        for &m11 in &c11 {
            for &m12 in &c12 {
                for &m21 in &c21 {
                    for &m22 in &c22 {
                        let mul: Vec<Vec<usize>> = (0..g.n)
                            .map(|a| {
                                let (a1, a2) = (a / q, a % q);
                                (0..g.n)
                                    .map(|b| {
                                        let (b1, b2) = (b / q, b % q);
                                        let mut acc = scalar_mul(&g, a1 * b1, m11);
                                        acc = g.add[acc][scalar_mul(&g, a1 * b2, m12)];
                                        acc = g.add[acc][scalar_mul(&g, a2 * b1, m21)];
                                        g.add[acc][scalar_mul(&g, a2 * b2, m22)]
                                    })
                                    .collect()
                            })
                            .collect();
                        match field_axiom_failure(&g, &mul) {
                            None => {
                                *field_counts.entry((order, name)).or_insert(0) += 1;
                                // Cross-anchor: a composed order-4 field is the engine's Gf4.
                                if order == 4 && !gf4_iso_found {
                                    let f = crate::polycalc_gfp::NsField::Gf4;
                                    let perms: Vec<[usize; 4]> = vec![
                                        [0, 1, 2, 3], [0, 1, 3, 2], [0, 2, 1, 3],
                                        [0, 2, 3, 1], [0, 3, 1, 2], [0, 3, 2, 1],
                                    ];
                                    gf4_iso_found = perms.iter().any(|phi| {
                                        (0..4).all(|a| {
                                            (0..4).all(|b| {
                                                phi[g.add[a][b]] as u64
                                                    == (phi[a] as u64 ^ phi[b] as u64)
                                                    && phi[mul[a][b]] as u64
                                                        == f.mul(phi[a] as u64, phi[b] as u64)
                                            })
                                        })
                                    });
                                }
                            }
                            Some(why) => {
                                *census.entry((order, name)).or_default().entry(why).or_insert(0) += 1
                            }
                        }
                    }
                }
            }
        }
        field_counts.entry((order, name)).or_insert(0);
    }

    for ((order, name), reasons) in &census {
        eprintln!("order {order} on {name}: candidate deaths {reasons:?}");
    }
    eprintln!("field structures found: {field_counts:?}");

    // Prime powers: composition BUILDS the fields — on the elementary-abelian pieces only.
    assert_eq!(field_counts[&(4, "ℤ/4")], 0, "no field on the nilpotent additive group ℤ/4");
    assert!(field_counts[&(4, "ℤ/2×ℤ/2")] > 0, "GF(4) composes from ℤ/2 × ℤ/2");
    assert!(gf4_iso_found, "a composed order-4 field is isomorphic to the engine's NsField::Gf4");
    assert_eq!(field_counts[&(9, "ℤ/9")], 0, "no field on ℤ/9");
    assert!(field_counts[&(9, "ℤ/3×ℤ/3")] > 0, "GF(9) composes from ℤ/3 × ℤ/3");
    // Composite non-prime-power: EVERY candidate over EVERY abelian group of the order dies.
    for (order, presentations) in
        [(6usize, ["ℤ/6", "ℤ/2×ℤ/3"]), (10, ["ℤ/10", "ℤ/2×ℤ/5"])]
    {
        for name in presentations {
            assert_eq!(
                field_counts[&(order, name)],
                0,
                "GF({order}) is impossible: zero survivors on {name}, exhaustively"
            );
        }
    }
}

/// An odd XOR triangle (`x0 ≠ x1`, `x1 ≠ x2`, `x2 ≠ x0`) — UNSAT by parity; `GF(2)`-easy,
/// `GF(3)`-hard: the family that separates the ring from its residue field.
fn parity_triangle() -> Vec<Vec<Lit>> {
    let p = |v: u32| Lit::pos(v);
    let q = |v: u32| Lit::neg(v);
    vec![
        vec![p(0), p(1)], vec![q(0), q(1)],
        vec![p(1), p(2)], vec![q(1), q(2)],
        vec![p(2), p(0)], vec![q(2), q(0)],
    ]
}

/// The shared measurement corpus: the named separating families plus a deterministic random sweep.
fn ring_corpus() -> Vec<(&'static str, usize, Vec<Vec<Lit>>)> {
    let (php3, _) = crate::families::php(3);
    let (cnt34, _) = crate::families::mod_counting(4, 3);
    let (cnt23, _) = crate::families::mod_counting(3, 2);
    let mut corpus: Vec<(&'static str, usize, Vec<Vec<Lit>>)> = vec![
        ("parity", 3, parity_triangle()),
        ("php3", php3.num_vars, php3.clauses),
        ("cnt34", cnt34.num_vars, cnt34.clauses),
        ("cnt23", cnt23.num_vars, cnt23.clauses),
    ];
    let mut seed = 0x0CA7_C047u64;
    for _ in 0..8 {
        let nv = 3 + (lcg(&mut seed) % 2) as usize;
        let nc = 2 * nv + (lcg(&mut seed) % 6) as usize;
        let clauses: Vec<Vec<Lit>> = (0..nc)
            .map(|_| {
                let width = 2 + (lcg(&mut seed) % 2) as usize;
                let mut vars: Vec<u32> = Vec::new();
                while vars.len() < width {
                    let v = (lcg(&mut seed) % nv as u64) as u32;
                    if !vars.contains(&v) {
                        vars.push(v);
                    }
                }
                vars.iter().map(|&v| Lit::new(v, lcg(&mut seed) & 1 == 1)).collect()
            })
            .collect();
        corpus.push(("rand", nv, clauses));
    }
    corpus
}

/// **Composite moduli intersect — they do not add: `ℤ/m`-NS is the CONJUNCTION of its coprime
/// parts.** CRT splits the coefficient ring, and a certificate's coefficients split with it (any
/// component pair of coefficient choices is realized by one `ℤ/m` choice), so `1` is in the
/// `ℤ/6`-span iff it is in the `GF(2)`-span AND the `GF(3)`-span — measured to hold at every
/// degree across the whole corpus, and likewise `ℤ/12 = ℤ/4 ∧ ℤ/3` (coprime prime-power
/// components, not just squarefree). The consequence, exhibited on named families: the composite
/// ring is *weaker* than each of its parts — parity is `GF(2)`-degree-2 but NOT `ℤ/6`-degree-2
/// (the `GF(3)` component blocks it), and `NS-degree over ℤ/6 = max` of the component degrees
/// (PHP(3): 4, both components 4). So "mod 6 reasoning" buys nothing for Nullstellensatz
/// refutation — the Barrington–Beigel–Rudich mod-6 power lives in polynomial *representation* of
/// Boolean functions, not in bounded-degree ideal membership, where coefficient freedom makes CRT
/// split cleanly.
#[test]
fn zm_ns_at_coprime_composite_moduli_is_the_conjunction_of_its_parts() {
    use crate::polycalc_gfp::{ns_refutes_gfp, NsField};
    for (name, nv, clauses) in &ring_corpus() {
        for d in 1..=(*nv).min(4) {
            let g2 = ns_refutes_gfp(NsField::Prime(2), *nv, clauses, d);
            let g3 = ns_refutes_gfp(NsField::Prime(3), *nv, clauses, d);
            assert_eq!(
                ns_refutes_zm(6, *nv, clauses, d),
                g2 && g3,
                "{name} n={nv} d={d}: ℤ/6-NS = GF(2)-NS ∧ GF(3)-NS"
            );
            assert_eq!(
                ns_refutes_zm(12, *nv, clauses, d),
                ns_refutes_zm(4, *nv, clauses, d) && ns_refutes_zm(3, *nv, clauses, d),
                "{name} n={nv} d={d}: ℤ/12-NS = ℤ/4-NS ∧ ℤ/3-NS"
            );
        }
    }
    // The named separation: parity is GF(2)-degree-2, but ℤ/6 cannot refute it at 2 — the
    // composite ring inherits the WEAKNESS of its GF(3) component.
    let parity = parity_triangle();
    assert!(ns_refutes_gfp(crate::polycalc_gfp::NsField::Prime(2), 3, &parity, 2));
    assert!(!ns_refutes_zm(6, 3, &parity, 2), "ℤ/6 is blocked by its GF(3) part on parity");
    // And the degree law on PHP(3): ℤ/6 degree = max(GF(2) degree, GF(3) degree) = 4.
    let (php3, _) = crate::families::php(3);
    assert!(!ns_refutes_zm(6, php3.num_vars, &php3.clauses, 3), "PHP(3): ℤ/6 degree > 3");
    assert!(ns_refutes_zm(6, php3.num_vars, &php3.clauses, 4), "PHP(3): ℤ/6 degree = 4 = max(4, 4)");
}

/// **A prime witness lifts to a ring witness — with a zero-divisor normalization.** Over a ring
/// the pseudo-expectation normalization is `L(1) ≠ 0` (demanding a unit would be dishonest:
/// `Hom(M, ℤ/m)` separates points because `ℤ/m` is self-injective, but the value at `1` may be a
/// zero divisor). The lift `L = (m/p)·L_p` turns a `GF(p)` witness into a `ℤ/m` one — every
/// pairing picks up the factor `m/p` and dies mod `m`, and `L(1) = m/p ≠ 0` — so the composite
/// ring inherits the LOWER BOUND of each prime part (the witness face of the CRT conjunction).
/// Checked concretely: the `GF(2)` PHP(3) witness lifts to `ℤ/6` with `L(1) = 3`; the `GF(3)`
/// parity witness lifts with `L(1) = 2` and certifies `ℤ/6`-degree > 2 for a family the
/// `GF(2)` component refutes at 2. The checker's soundness is one field-free line — a refutation
/// would force `L(1) = 0` — and it rejects the adversarial corruptions (dropped normalization,
/// annihilated scaling, perturbed constrained monomial).
#[test]
fn a_prime_witness_lifts_to_a_ring_witness_with_zero_divisor_normalization() {
    use crate::polycalc_gfp::{ns_lower_bound_witness_gfp, NsField};
    // GF(2) PHP(3) witness at degree 3 → ℤ/6 witness with L(1) = 3.
    let (php3, _) = crate::families::php(3);
    let w2 = ns_lower_bound_witness_gfp(NsField::Prime(2), php3.num_vars, &php3.clauses, 3)
        .expect("PHP(3) has a GF(2) witness at degree 3");
    let lifted2 = lift_prime_witness_to_zm(6, 2, &w2);
    assert_eq!(
        lifted2.iter().find(|&&(mo, _)| mo == 0).map(|&(_, v)| v),
        Some(3),
        "the GF(2) lift is normalized at the zero divisor L(1) = 6/2 = 3"
    );
    assert!(
        check_ns_lower_bound_zm(6, php3.num_vars, &php3.clauses, 3, &lifted2),
        "the lifted witness certifies ℤ/6-NS-degree(PHP(3)) > 3 with zero trust"
    );
    // GF(3) parity witness at degree 2 → ℤ/6 witness with L(1) = 2: a ring lower bound for a
    // family the OTHER component (GF(2)) refutes at that very degree.
    let parity = parity_triangle();
    let w3 = ns_lower_bound_witness_gfp(NsField::Prime(3), 3, &parity, 2)
        .expect("parity has a GF(3) witness at degree 2");
    let lifted3 = lift_prime_witness_to_zm(6, 3, &w3);
    assert_eq!(
        lifted3.iter().find(|&&(mo, _)| mo == 0).map(|&(_, v)| v),
        Some(2),
        "the GF(3) lift is normalized at L(1) = 6/3 = 2"
    );
    assert!(
        check_ns_lower_bound_zm(6, 3, &parity, 2, &lifted3),
        "one prime part's witness certifies the ring lower bound"
    );
    // Corruptions are rejected: no normalization; scaling that annihilates L(1); a perturbed
    // constrained monomial (bump L on a monomial of an admitted generator product).
    let no_one: Vec<(Mono, u64)> = lifted2.iter().copied().filter(|&(mo, _)| mo != 0).collect();
    assert!(!check_ns_lower_bound_zm(6, php3.num_vars, &php3.clauses, 3, &no_one));
    let annihilated: Vec<(Mono, u64)> =
        lifted2.iter().map(|&(mo, v)| (mo, zm_mul(6, v, 2))).collect(); // 3·2 = 0 (mod 6)
    assert!(!check_ns_lower_bound_zm(6, php3.num_vars, &php3.clauses, 3, &annihilated));
    let gens: Vec<ZmPoly> =
        php3.clauses.iter().map(|c| clause_polynomial_zm(6, c)).collect();
    let target = gens
        .iter()
        .find_map(|g| {
            crate::polycalc::monomials_up_to_degree(php3.num_vars, 3).into_iter().find_map(
                |mo| {
                    let prod = poly_mul_mono(6, g, mo);
                    (!prod.is_empty() && zm_poly_degree(&prod) <= 3)
                        .then(|| *prod.keys().next_back().unwrap())
                },
            )
        })
        .expect("an admitted generator product exists");
    let mut perturbed: Vec<(Mono, u64)> =
        lifted2.iter().copied().filter(|&(mo, _)| mo != target).collect();
    let old = lifted2.iter().find(|&&(mo, _)| mo == target).map_or(0, |&(_, v)| v);
    perturbed.push((target, zm_add(6, old, 1)));
    assert!(!check_ns_lower_bound_zm(6, php3.num_vars, &php3.clauses, 3, &perturbed));
}

/// **The nilpotent ring is STRICTLY weaker than its residue field at fixed degree.** `ℤ/4` maps
/// onto `GF(2)` (a ring homomorphism), so a `ℤ/4` refutation projects to a `GF(2)` refutation at
/// the same degree — soundness, swept across the corpus. The converse FAILS, and the measured
/// separation is parity: `GF(2)` refutes the odd XOR triangle at degree 2, `ℤ/4` cannot (no
/// mismatch of the other direction anywhere), refuting it only at degree 3. The explanation is
/// the Hensel tax: lifting `Σ g·p ≡ 1 (mod 2)` to `mod 4` multiplies by `(2 − s)` — a genuine
/// certificate, but of degree up to `2d`. Nilpotents make the ring *harder to refute in*, never
/// easier: exactly opposite to the naive "more structure, more power" guess, and the reason the
/// prime-power components in the CRT conjunction are `ℤ/pᵏ`, not `GF(p)`.
#[test]
fn z4_is_strictly_weaker_than_its_residue_field_at_fixed_degree() {
    use crate::polycalc_gfp::{ns_refutes_gfp, NsField};
    // Projection soundness: ℤ/4 refutes at d ⟹ GF(2) refutes at d, everywhere on the corpus.
    for (name, nv, clauses) in &ring_corpus() {
        for d in 1..=(*nv).min(4) {
            if ns_refutes_zm(4, *nv, clauses, d) {
                assert!(
                    ns_refutes_gfp(NsField::Prime(2), *nv, clauses, d),
                    "{name} n={nv} d={d}: a ℤ/4 refutation projects to GF(2)"
                );
            }
        }
    }
    // The strict separation, both named instances: GF(2) degree 2, ℤ/4 degree 3.
    for (name, nv, clauses) in
        [("parity", 3usize, parity_triangle()), ("cnt23", 3, crate::families::mod_counting(3, 2).0.clauses)]
    {
        assert!(ns_refutes_gfp(NsField::Prime(2), nv, &clauses, 2), "{name}: GF(2) refutes at 2");
        assert!(!ns_refutes_zm(4, nv, &clauses, 2), "{name}: ℤ/4 cannot refute at 2 — the nilpotent tax");
        assert!(ns_refutes_zm(4, nv, &clauses, 3), "{name}: ℤ/4 refutes at 3 (within the Hensel 2d bound)");
    }
}

/// **The partition-of-unity atom survives zero divisors.** `(1 − x) + x = 1` is a ring identity —
/// no inverses anywhere — so the atom and the full `2ⁿ`-corner cancellation hold over `ℤ/4`
/// (nilpotent 2), `ℤ/6` (idempotents 3, 4), `ℤ/9`, and `ℤ/12` exactly as over the fields. This is
/// the load-bearing fact behind ring completeness.
#[test]
fn the_partition_of_unity_atom_is_one_over_zero_divisor_moduli() {
    for &m in &[4u64, 6, 9, 12] {
        let one: ZmPoly = [(0u64, 1u64)].into_iter().collect();
        for v in 0..8 {
            assert_eq!(pou_atom_zm(m, v), one, "m={m}: the atom (1−x{v})+x{v} reduces to 1");
        }
        for n in 0..=8 {
            assert_eq!(partition_of_unity_zm(m, n), one, "m={m}: Σ_a δ_a = 1 over the {n}-cube");
        }
    }
}

/// **The signed clause indicator works over every modulus, pinned corner-by-corner.** Over `ℤ/6`
/// the positive-literal factor is `1 + 5x` (`5 = −1`); the product is `1` exactly on falsifying
/// corners and `0` elsewhere for `m ∈ {4, 6, 9, 12}` on a deterministic random-clause corpus —
/// the 0/1-valued factor argument is ring-generic.
#[test]
fn z6_clause_polynomial_is_the_signed_false_indicator_on_every_corner() {
    let pos = clause_polynomial_zm(6, &[Lit::pos(0)]);
    let expected: ZmPoly = [(0u64, 1u64), (1u64, 5u64)].into_iter().collect();
    assert_eq!(pos, expected, "positive literal → 1 − x = 1 + 5x over ℤ/6");
    let mut seed = 0x0006_C0DEu64;
    for &m in &[4u64, 6, 9, 12] {
        for _ in 0..30 {
            let n = 2 + (lcg(&mut seed) % 4) as usize; // 2..=5 variables
            let width = 1 + (lcg(&mut seed) % n as u64) as usize;
            let mut vars: Vec<u32> = Vec::new();
            while vars.len() < width {
                let v = (lcg(&mut seed) % n as u64) as u32;
                if !vars.contains(&v) {
                    vars.push(v);
                }
            }
            let clause: Vec<Lit> =
                vars.iter().map(|&v| Lit::new(v, lcg(&mut seed) & 1 == 1)).collect();
            let poly = clause_polynomial_zm(m, &clause);
            for a in 0u64..(1u64 << n) {
                let want = if falsifies(&clause, a) { 1 } else { 0 };
                assert_eq!(eval_at(m, &poly, a), want, "m={m} clause={clause:?} corner={a:b}");
            }
        }
    }
}

/// **The Howell echelon decides `ℤ/m` span membership EXACTLY — proven against the exhaustive
/// oracle.** For small systems the ground truth is enumerable: every one of the `m^rows`
/// coefficient combinations is generated, giving the true span as a set; the echelon's
/// `contains` must then agree on **every** vector of `(ℤ/m)^n` — all `m^n` of them. Swept over
/// the zero-divisor moduli `{4, 6, 9, 12}` (nilpotents, idempotents, both) on a deterministic
/// random corpus. This is the module's trust anchor: no Gaussian intuition survives zero
/// divisors unchecked.
#[test]
fn zm_span_membership_matches_the_exhaustive_oracle() {
    let mut seed = 0x40E1_1000u64;
    for &m in &[4u64, 6, 9, 12] {
        for _ in 0..25 {
            let n = 2 + (lcg(&mut seed) % 3) as usize; // 2..=4 columns
            let nrows = 1 + (lcg(&mut seed) % 3) as usize; // 1..=3 generators
            let rows: Vec<Vec<u64>> = (0..nrows)
                .map(|_| (0..n).map(|_| lcg(&mut seed) % m).collect())
                .collect();
            // Ground truth: every ℤ/m combination of the generators.
            let mut span: std::collections::BTreeSet<Vec<u64>> = std::collections::BTreeSet::new();
            let mut combo = vec![0u64; nrows];
            loop {
                let v: Vec<u64> = (0..n)
                    .map(|j| {
                        rows.iter()
                            .zip(&combo)
                            .fold(0u64, |acc, (r, &c)| zm_add(m, acc, zm_mul(m, c, r[j])))
                    })
                    .collect();
                span.insert(v);
                let mut i = 0;
                while i < nrows {
                    combo[i] += 1;
                    if combo[i] < m {
                        break;
                    }
                    combo[i] = 0;
                    i += 1;
                }
                if i == nrows {
                    break;
                }
            }
            // The echelon, fed the same generators, must agree on EVERY vector of (ℤ/m)^n.
            let mut ech = ZmEchelon::new(m);
            for r in &rows {
                ech.insert(r.clone());
            }
            let mut target = vec![0u64; n];
            loop {
                assert_eq!(
                    ech.contains(target.clone()),
                    span.contains(&target),
                    "m={m} rows={rows:?} target={target:?}"
                );
                let mut i = 0;
                while i < n {
                    target[i] += 1;
                    if target[i] < m {
                        break;
                    }
                    target[i] = 0;
                    i += 1;
                }
                if i == n {
                    break;
                }
            }
        }
    }
}

/// **At prime `m` the ring engine IS the field engine.** `ℤ/p = GF(p)`, so on random CNFs the
/// Howell-based decision must coincide with the Gaussian-based one at every degree — the
/// differential anchor tying the ring machinery to the already-anchored characteristic axis.
#[test]
fn the_ring_engine_at_prime_m_agrees_with_the_field_engine() {
    let mut seed = 0x9219_0E11u64;
    for _ in 0..25 {
        let nv = 3 + (lcg(&mut seed) % 3) as usize; // 3..=5
        let nc = 4 + (lcg(&mut seed) % 8) as usize;
        let clauses: Vec<Vec<Lit>> = (0..nc)
            .map(|_| {
                let width = 1 + (lcg(&mut seed) % 3) as usize;
                let mut vars: Vec<u32> = Vec::new();
                while vars.len() < width {
                    let v = (lcg(&mut seed) % nv as u64) as u32;
                    if !vars.contains(&v) {
                        vars.push(v);
                    }
                }
                vars.iter().map(|&v| Lit::new(v, lcg(&mut seed) & 1 == 1)).collect()
            })
            .collect();
        for &p in &[2u64, 3, 5] {
            for d in 1..=nv.min(4) {
                assert_eq!(
                    ns_refutes_zm(p, nv, &clauses, d),
                    crate::polycalc_gfp::ns_refutes_gfp(
                        crate::polycalc_gfp::NsField::Prime(p),
                        nv,
                        &clauses,
                        d
                    ),
                    "p={p} n={nv} d={d}: ℤ/p ring engine = GF(p) field engine"
                );
            }
        }
    }
}

/// **Ring completeness is total, sound, and fail-closed over `ℤ/6` AND `ℤ/4`.** The
/// partition-of-unity charging never divides, so it survives idempotents (`ℤ/6`) and nilpotents
/// (`ℤ/4`) alike: every UNSAT formula gets a certificate that re-checks (ring and corner-wise),
/// every SAT formula gets a model that satisfies it, and a certificate refuses a clause set it
/// was not built for. Cross-checked against brute-force satisfiability throughout.
#[test]
fn build_ns_certificate_zm_is_total_sound_and_fail_closed_over_z6_and_z4() {
    let mut seed = 0x0BAD_0604u64;
    for &m in &[6u64, 4] {
        let mut unsat_seen = 0usize;
        for _ in 0..60 {
            let nv = 4 + (lcg(&mut seed) % 3) as usize; // 4..=6
            let nc = 2 * nv + (lcg(&mut seed) % (3 * nv as u64)) as usize; // dense: UNSAT-rich
            let clauses: Vec<Vec<Lit>> = (0..nc)
                .map(|_| {
                    let width = 2 + (lcg(&mut seed) % 2) as usize;
                    let mut vars: Vec<u32> = Vec::new();
                    while vars.len() < width {
                        let v = (lcg(&mut seed) % nv as u64) as u32;
                        if !vars.contains(&v) {
                            vars.push(v);
                        }
                    }
                    vars.iter().map(|&v| Lit::new(v, lcg(&mut seed) & 1 == 1)).collect()
                })
                .collect();
            match build_ns_certificate_zm(m, nv, &clauses) {
                Ok(cert) => {
                    unsat_seen += 1;
                    assert_eq!(cert.modulus(), m);
                    assert!(cert.verify(&clauses), "m={m} n={nv}: the ring certificate re-checks");
                    assert!(cert.degree() <= nv, "m={m} n={nv}: certificate degree ≤ n");
                    assert!(!sat(nv, &clauses), "m={m}: certificates only for genuine UNSAT");
                    assert!(
                        !cert.verify(&clauses[..clauses.len() - 1]),
                        "m={m}: a certificate must not verify a different clause set"
                    );
                }
                Err(model) => {
                    assert!(sat(nv, &clauses), "m={m}: SAT verdicts only for satisfiable formulas");
                    assert!(
                        clauses
                            .iter()
                            .all(|c| c.iter().any(|l| model[l.var() as usize] == l.is_positive())),
                        "m={m}: the returned model satisfies every clause"
                    );
                }
            }
        }
        assert!(unsat_seen >= 5, "m={m}: the corpus exercises the UNSAT branch ({unsat_seen})");
    }
}

/// **"Structureless" has no witness over ANY modulus — the two-poles theorem, ring-completed.**
/// Define hardness-as-structurelessness: *no certificate at any degree `≤ n` exists*. This test
/// certifies that **no finite formula fulfills that definition over any `ℤ/m`, `m = 2..12`** —
/// every UNSAT instance in the corpus (pigeonhole, an odd-parity block, modular counting, and a
/// deterministic random sweep) receives a degree-`≤ n` certificate that re-checks with zero
/// trust, and every SAT instance a model. Structure always exists, over every coefficient ring.
/// The honest boundary, stated with the theorem: the certificate lives in the `2ⁿ` basis —
/// existence, not efficiency — and the *cost* pole is real and certified elsewhere (the degree
/// lower bounds grow; the incompressibility theorems are kernel facts). What is refuted here is
/// precisely the existence-form of randomness at finite `n`, not the asymptotic cost-form of
/// NP-hardness, which no algebraic instrument can decide (algebrization).
#[test]
fn no_finite_formula_is_structureless_over_any_modulus() {
    let mut corpus: Vec<(usize, Vec<Vec<Lit>>)> = Vec::new();
    let (php3, _) = crate::families::php(3);
    corpus.push((php3.num_vars, php3.clauses));
    let (cnt34, _) = crate::families::mod_counting(4, 3);
    corpus.push((cnt34.num_vars, cnt34.clauses));
    // An odd XOR cycle (x0≠x1, x1≠x2, x2≠x0) — UNSAT by parity.
    let p = |v: u32| Lit::pos(v);
    let q = |v: u32| Lit::neg(v);
    corpus.push((3, vec![
        vec![p(0), p(1)], vec![q(0), q(1)],
        vec![p(1), p(2)], vec![q(1), q(2)],
        vec![p(2), p(0)], vec![q(2), q(0)],
    ]));
    let mut seed = 0xA11_0DD5u64;
    for _ in 0..10 {
        let nv = 3 + (lcg(&mut seed) % 3) as usize;
        let nc = nv + (lcg(&mut seed) % (2 * nv as u64)) as usize;
        let clauses: Vec<Vec<Lit>> = (0..nc)
            .map(|_| {
                let width = 2 + (lcg(&mut seed) % 2) as usize;
                let mut vars: Vec<u32> = Vec::new();
                while vars.len() < width {
                    let v = (lcg(&mut seed) % nv as u64) as u32;
                    if !vars.contains(&v) {
                        vars.push(v);
                    }
                }
                vars.iter().map(|&v| Lit::new(v, lcg(&mut seed) & 1 == 1)).collect()
            })
            .collect();
        corpus.push((nv, clauses));
    }
    let mut certified = 0usize;
    for m in 2u64..=12 {
        for (nv, clauses) in &corpus {
            match build_ns_certificate_zm(m, *nv, clauses) {
                Ok(cert) => {
                    assert!(cert.verify(clauses), "m={m} n={nv}: the certificate re-checks");
                    assert!(cert.degree() <= *nv, "m={m}: degree ≤ n — structure within the cube");
                    certified += 1;
                }
                Err(model) => {
                    assert!(
                        clauses
                            .iter()
                            .all(|c| c.iter().any(|l| model[l.var() as usize] == l.is_positive())),
                        "m={m}: SAT half — the model satisfies every clause"
                    );
                }
            }
        }
    }
    assert!(certified >= 3 * 11, "the UNSAT half is exercised across all 11 moduli ({certified})");
    eprintln!(
        "structureless-witness count over m = 2..12: 0 of {} (UNSAT instances certified: {certified})",
        corpus.len() * 11
    );
}
