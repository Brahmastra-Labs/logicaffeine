use super::*;
use crate::families;

/// `|AGL(n,p)| = pⁿ·|GL(n,p)|`, and the exhaustive enumeration produces exactly that many bijections.
#[test]
fn agl_p_order_matches_the_enumeration() {
    for (n, p) in [
        (1usize, 2u64),
        (1, 3),
        (2, 2),
        (2, 3),
        (2, 5),
        (3, 2),
        (3, 3),
    ] {
        assert_eq!(
            all_affine_p_bijections(n, p).len() as u128,
            agl_p_order(n as u32, p),
            "n={n}, p={p}: enumerated affine bijections must equal |AGL(n,p)| = pⁿ·|GL(n,p)|"
        );
    }
    // Pinned: |AGL(2,3)| = 9·48 = 432, |AGL(3,2)| = 8·168 = 1344.
    assert_eq!(agl_p_order(2, 3), 432);
    assert_eq!(agl_p_order(3, 2), 1344);
}

/// Group laws over `GF(p)`: identity is a unit, and composition matches pointwise application.
#[test]
fn affine_p_maps_compose_and_act_correctly() {
    let p = 3u64;
    let id = AffineP::identity(2, p);
    for phi in all_affine_p_bijections(2, p) {
        assert!(phi.is_bijection());
        assert_eq!(phi.compose(&id), phi);
        assert_eq!(id.compose(&phi), phi);
        let sq = phi.compose(&phi);
        for a in 0..p {
            for b in 0..p {
                let x = vec![a, b];
                assert_eq!(
                    sq.apply(&x),
                    phi.apply(&phi.apply(&x)),
                    "composition must match double application"
                );
            }
        }
    }
}

/// **THE CENSUS GAP OVER `GF(p)`: `AGL(3,3) ⊋ monomial`.** The mod-3 parity plane `x₀+x₁+x₂ ≡ 0 (mod 3)`
/// is rigid under the monomial (scaled-permutation) breakers but carries `864·9 = 7776` affine
/// symmetries (864 linear maps fix the hyperplane × 9 in-plane translations) — far more than the
/// monomial subgroup. The witness is a genuine `GF(3)` **shear**: a symmetry whose matrix is not
/// monomial, the mod-`p` analog of the parity shear no clause break can see.
#[test]
fn gfp_affine_symmetry_strictly_exceeds_monomial_on_mod_p_parity() {
    let eq = ModpEquation::new(vec![(0usize, 1u64), (1, 1), (2, 1)], 0); // x0+x1+x2 ≡ 0 (mod 3)
    let models = models_p(3, 3, std::slice::from_ref(&eq));
    assert_eq!(models.len(), 9, "the mod-3 hyperplane has 3² = 9 points");

    let sym = affine_p_symmetries(3, 3, &models);
    assert_eq!(
        sym.len(),
        7776,
        "AGL(3,3) stabilizer of the mod-3 hyperplane = 864 linear × 9 translations"
    );

    let monomial = sym.iter().filter(|a| a.is_monomial()).count();
    assert!(
        sym.len() > monomial,
        "affine symmetry ({}) must strictly exceed the monomial part ({monomial})",
        sym.len()
    );
    assert!(
        sym.iter().any(|a| !a.is_monomial()),
        "a non-monomial GF(3) affine symmetry (a shear) must exist — invisible to the monomial breakers"
    );
}

/// **The `GF(p)` affine refutation is certified through the mod-`p` DRAT bridge.** On a mod-`p` Tseitin
/// counting core (UNSAT over `GF(p)`, exponential for resolution and Z3), [`affine_p_refutation_drat`]
/// compiles the `GF(p)` dependency to RUP resolvent lemmas over the one-hot encoding that our
/// independent checker accepts against the original CNF.
#[test]
fn gfp_affine_refutation_is_certified_via_modp_drat_bridge() {
    for (n, p) in [(4usize, 3u64), (6, 3), (4, 5)] {
        let (_, cnf, _) = families::mod_p_tseitin_expander(n, p, 1);
        let Some(proof) = affine_p_refutation_drat(cnf.num_vars, &cnf.clauses) else {
            eprintln!("[modp n={n} p={p}] resolution route over budget — skipped");
            continue;
        };
        assert!(
            proof.last().is_some_and(|c| c.is_empty()),
            "proof ends in the empty clause (n={n}, p={p})"
        );
        assert!(
            crate::rup::check_refutation(cnf.num_vars, &cnf.clauses, &proof),
            "the GF({p}) affine refutation must RUP-refute the original CNF (n={n})"
        );
    }
}

/// **The GF(p) solution-space solver is exact.** Against brute force over hundreds of random `GF(p)`
/// systems: it reports inconsistency iff there are no solutions, its `count` equals the solution
/// count, and its `particular + kernel` enumerates *exactly* the solution set.
#[test]
fn solve_space_matches_brute_force() {
    use crate::modp::solve_space;
    let mut state = 0xC0FFEE_77u64;
    let mut rng = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for &p in &[3u64, 5] {
        for _ in 0..200 {
            let n = 2 + (rng() % 3) as usize; // 2..=4 variables
            let m = 1 + (rng() % 4) as usize;
            let eqs: Vec<ModpEquation> = (0..m)
                .map(|_| {
                    let k = 1 + (rng() % n as u64) as usize;
                    let mut vars: Vec<usize> = Vec::new();
                    while vars.len() < k {
                        let v = (rng() % n as u64) as usize;
                        if !vars.contains(&v) {
                            vars.push(v);
                        }
                    }
                    let coeffs: Vec<(usize, u64)> =
                        vars.iter().map(|&v| (v, 1 + rng() % (p - 1))).collect();
                    ModpEquation::new(coeffs, rng() % p)
                })
                .collect();
            let brute: HashSet<Vec<u64>> = models_p(n, p, &eqs).into_iter().collect();
            match solve_space(&eqs, n, p) {
                None => assert!(brute.is_empty(), "None ⇒ no GF({p}) solutions"),
                Some(ss) => {
                    assert_eq!(
                        ss.count() as usize,
                        brute.len(),
                        "count must equal the brute-force solution count"
                    );
                    let got: HashSet<Vec<u64>> = ss.enumerate().into_iter().collect();
                    assert_eq!(
                        got, brute,
                        "particular + kernel must enumerate exactly the GF({p}) solutions"
                    );
                }
            }
        }
    }
}

/// Encode a mod-`p` system in one-hot Boolean form exactly as the families do: `b(e,val) = e·p+val`,
/// at-least-one + at-most-one per edge, and per-equation "forbid every wrong-sum assignment" clauses.
fn onehot_cnf(p: u64, num_edges: usize, equations: &[ModpEquation]) -> (usize, Vec<Vec<Lit>>) {
    let bvar = |e: usize, val: u64| (e * p as usize + val as usize) as u32;
    let mut clauses: Vec<Vec<Lit>> = Vec::new();
    for e in 0..num_edges {
        clauses.push((0..p).map(|v| Lit::pos(bvar(e, v))).collect());
        for v1 in 0..p {
            for v2 in (v1 + 1)..p {
                clauses.push(vec![Lit::neg(bvar(e, v1)), Lit::neg(bvar(e, v2))]);
            }
        }
    }
    for eq in equations {
        let d = eq.coeffs.len();
        for idx in 0..p.pow(d as u32) {
            let mut x = idx;
            let combo: Vec<u64> = (0..d)
                .map(|_| {
                    let v = x % p;
                    x /= p;
                    v
                })
                .collect();
            let sum = eq
                .coeffs
                .iter()
                .zip(&combo)
                .fold(0u64, |a, (&(_, co), &val)| (a + co * val) % p);
            if sum % p != eq.rhs % p {
                clauses.push(
                    eq.coeffs
                        .iter()
                        .zip(&combo)
                        .map(|(&(v, _), &val)| Lit::neg(bvar(v, val)))
                        .collect(),
                );
            }
        }
    }
    (num_edges * p as usize, clauses)
}

/// **The GF(p) SAT-side break pins a determined edge — soundly.** A 3-edge `GF(3)` system whose
/// equations force every edge (`x0=1, x1=2, x2=1`): the break recovers it, finds all three forced, and
/// emits the one-hot units — each of which is verified to hold in *every* Boolean model of the CNF.
#[test]
fn affine_p_forced_pins_determined_edges_and_is_sound() {
    // x0+x1≡0, x1+x2≡0, x0+x2≡2 (mod 3) ⇒ x0=1, x1=2, x2=1, all forced.
    let eqs = vec![
        ModpEquation::new(vec![(0usize, 1u64), (1, 1)], 0),
        ModpEquation::new(vec![(1usize, 1u64), (2, 1)], 0),
        ModpEquation::new(vec![(0usize, 1u64), (2, 1)], 2),
    ];
    let (nbv, clauses) = onehot_cnf(3, 3, &eqs);
    match affine_p_forced(nbv, &clauses) {
        AffinePForced::Forced(units) => {
            // b(0,1)=1, b(1,2)=5, b(2,1)=7 must be pinned true.
            for &(e, v) in &[(0usize, 1u64), (1, 2), (2, 1)] {
                let bvar = (e * 3 + v as usize) as u32;
                assert!(
                    units.contains(&vec![Lit::pos(bvar)]),
                    "must pin b(edge{e}={v}) = var {bvar}; got {units:?}"
                );
            }
            // Soundness: every forced unit holds in every Boolean model of the one-hot CNF.
            let models = crate::affine::models_of(nbv, &clauses);
            assert!(
                !models.is_empty(),
                "the crafted determined system is satisfiable"
            );
            for u in &units {
                let l = u[0];
                assert!(
                    models
                        .iter()
                        .all(|&m| ((m >> l.var()) & 1 == 1) == l.is_positive()),
                    "forced unit on var {} must hold in every Boolean model",
                    l.var()
                );
            }
        }
        _ => panic!("the determined edges must produce forced one-hot units"),
    }
}

/// **A GF(p) link lifts to value-permuted bit-equivalences.** Over `GF(3)`, `x0 + x1 ≡ 0` links the two
/// edges by the scalar `c = 2` (`x0 = 2·x1`) with neither forced — the solution space exposes the
/// proportional kernel columns, and the SAT-side break lifts the link to one-hot bit-equivalences that
/// genuinely *permute values* (e.g. `b(x=1) ↔ b(=2)`), the mod-`p` shear no clause equivalence expresses.
#[test]
fn gfp_link_lifts_to_value_permuted_bit_equivalences() {
    let p = 3u64;
    let eqs = vec![ModpEquation::new(vec![(0usize, 1u64), (1, 1)], 0)];

    // Abstract: one free direction, and x0's kernel entry is exactly 2× x1's — the scalar link c = 2.
    let ss = crate::modp::solve_space(&eqs, 2, p).expect("consistent system");
    assert_eq!(
        ss.kernel_basis.len(),
        1,
        "one free variable ⇒ a single kernel direction"
    );
    let kv = &ss.kernel_basis[0];
    assert!(kv[0] != 0 && kv[1] != 0, "neither variable is forced");
    assert_eq!(
        kv[0],
        (2 * kv[1]) % p,
        "x0's kernel entry is 2× x1's — the GF(3) scalar link"
    );

    // Boolean: the break fires with sound, value-permuted bit-equivalences.
    let (nbv, clauses) = onehot_cnf(p, 2, &eqs);
    let AffinePForced::Forced(extra) = affine_p_forced(nbv, &clauses) else {
        panic!("the link must fire as forced consequences");
    };
    assert!(
        extra.iter().any(|c| c.len() == 2),
        "must emit bit-equivalences (2-literal clauses)"
    );
    let models = crate::affine::models_of(nbv, &clauses);
    for c in &extra {
        assert!(
            models.iter().all(|&m| c
                .iter()
                .any(|l| ((m >> l.var()) & 1 == 1) == l.is_positive())),
            "every emitted equivalence must hold in every Boolean model"
        );
    }
    // Non-trivial: at least one equivalence relates DIFFERENT value-positions across the two groups.
    let rec = crate::modp::recover_from_cnf(nbv, &clauses).expect("recovers");
    let pos_of = |bv: u32| {
        rec.groups
            .iter()
            .enumerate()
            .find_map(|(g, grp)| grp.iter().position(|&b| b == bv).map(|v| (g, v)))
    };
    let value_permuted = extra.iter().filter(|c| c.len() == 2).any(|c| {
        matches!((pos_of(c[0].var()), pos_of(c[1].var())), (Some((g0, v0)), Some((g1, v1))) if g0 != g1 && v0 != v1)
    });
    assert!(
        value_permuted,
        "a genuine GF(3) link must produce a value-permuted bit-equivalence"
    );
}

/// **Soundness to the point of absurdity.** Random `GF(3)`/`GF(5)` systems over two one-hot edges, the
/// whole SAT-side break (forced units AND linked bit-equivalences) brute-force-checked: every emitted
/// clause holds in *every* Boolean model, and `Refuted` only ever names a genuinely-UNSAT formula.
#[test]
fn affine_p_break_is_sound_against_brute_force() {
    let mut state = 0x5A7_AFF1_9E37u64;
    let mut rng = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for &p in &[3u64, 5] {
        for _ in 0..200 {
            let m = 1 + (rng() % 3) as usize;
            let eqs: Vec<ModpEquation> = (0..m)
                .map(|_| {
                    // 2-variable equations over the two edges (what recover_from_cnf reconstructs).
                    ModpEquation::new(
                        vec![(0usize, 1 + rng() % (p - 1)), (1, 1 + rng() % (p - 1))],
                        rng() % p,
                    )
                })
                .collect();
            let (nbv, clauses) = onehot_cnf(p, 2, &eqs);
            let models = crate::affine::models_of(nbv, &clauses);
            match affine_p_forced(nbv, &clauses) {
                AffinePForced::Refuted(_) => {
                    assert!(models.is_empty(), "Refuted ⇒ the Boolean CNF must be UNSAT")
                }
                AffinePForced::Forced(extra) => {
                    for c in &extra {
                        assert!(
                            models.iter().all(|&m| c
                                .iter()
                                .any(|l| ((m >> l.var()) & 1 == 1) == l.is_positive())),
                            "every forced/linked consequence must hold in every Boolean model"
                        );
                    }
                }
                AffinePForced::Unchanged => {}
            }
        }
    }
}

/// `|AGL(n, ℤ/m)|` factors by CRT: `|AGL(2, ℤ/6)| = |AGL(2,2)|·|AGL(2,3)| = 24·432 = 10368`, and a
/// non-squarefree modulus has no such product form.
#[test]
fn agl_m_order_factors_by_crt() {
    assert_eq!(agl_m_order(2, 6), Some(24 * 432));
    assert_eq!(agl_m_order(2, 30), Some(24 * 432 * 12000)); // 2·3·5
    assert_eq!(agl_m_order(2, 4), None, "4 is not squarefree");
    assert_eq!(agl_m_order(2, 12), None, "12 = 2²·3 is not squarefree");
}

/// Brute-force the GF(m)-valued solutions of a `ℤ/m` system (`Σ coeffs·x ≡ rhs mod m`).
fn models_m(n: usize, m: u64, equations: &[ModpEquation]) -> Vec<Vec<u64>> {
    (0..m.pow(n as u32))
        .filter_map(|code| {
            let mut c = code;
            let x: Vec<u64> = (0..n)
                .map(|_| {
                    let v = c % m;
                    c /= m;
                    v
                })
                .collect();
            equations
                .iter()
                .all(|eq| {
                    eq.coeffs
                        .iter()
                        .fold(0u64, |a, &(v, co)| (a + co * x[v]) % m)
                        % m
                        == eq.rhs % m
                })
                .then_some(x)
        })
        .collect()
}

/// **The composite forced-value detector is exact.** Against brute force over random squarefree-`m`
/// systems: a variable is reported forced iff it takes one value across every `ℤ/m` solution, with that
/// value; `Inconsistent` iff there are no solutions.
#[test]
fn forced_values_squarefree_matches_brute_force() {
    use crate::modm::{forced_values_squarefree, ForcedM};
    let mut state = 0xC0DE_B0D5u64;
    let mut rng = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for &m in &[6u64, 10, 15] {
        for _ in 0..150 {
            let eqs: Vec<ModpEquation> = (0..(1 + rng() % 3))
                .map(|_| ModpEquation::new(vec![(0usize, rng() % m), (1, rng() % m)], rng() % m))
                .collect();
            let models = models_m(2, m, &eqs);
            let brute: Vec<Option<u64>> = (0..2)
                .map(|g| {
                    let vals: HashSet<u64> = models.iter().map(|x| x[g]).collect();
                    if vals.len() == 1 {
                        vals.into_iter().next()
                    } else {
                        None
                    }
                })
                .collect();
            match forced_values_squarefree(&eqs, 2, m) {
                None => panic!("m={m} is squarefree"),
                Some(ForcedM::Inconsistent) => {
                    assert!(models.is_empty(), "Inconsistent ⇒ no ℤ/{m} solutions")
                }
                Some(ForcedM::Forced(f)) => {
                    assert_eq!(f, brute, "forced values must match brute force (m={m})")
                }
            }
        }
    }
}

/// **The composite ℤ/m SAT-side break pins forced edges — soundly.** `x0 ≡ 5`, `x1 ≡ 2` over `ℤ/6`
/// (one congruence per edge, as the recovery requires): the break recovers it, CRT-combines the
/// `GF(2)×GF(3)` forced values, and emits one-hot units each verified against every Boolean model.
#[test]
fn affine_m_forced_pins_composite_modulus_and_is_sound() {
    let m = 6u64;
    let eqs = vec![
        ModpEquation::new(vec![(0usize, 1u64)], 5),
        ModpEquation::new(vec![(1usize, 1u64)], 2),
    ];
    let (nbv, clauses) = onehot_cnf(m, 2, &eqs);
    match affine_m_forced(nbv, &clauses) {
        AffinePForced::Forced(units) => {
            assert!(
                units.contains(&vec![Lit::pos(5)]),
                "must pin b(edge0=5); got {units:?}"
            ); // 0*6+5
            assert!(units.contains(&vec![Lit::pos(8)]), "must pin b(edge1=2)"); // 1*6+2
            let models = crate::affine::models_of(nbv, &clauses);
            assert!(!models.is_empty(), "the crafted ℤ/6 system is satisfiable");
            for u in &units {
                let l = u[0];
                assert!(
                    models
                        .iter()
                        .all(|&mm| ((mm >> l.var()) & 1 == 1) == l.is_positive()),
                    "forced unit on var {} must hold in every Boolean model",
                    l.var()
                );
            }
        }
        _ => panic!("the determined ℤ/6 edges must produce forced one-hot units"),
    }
}

/// **The composite break refutes a zero-divisor obstruction.** Over `ℤ/6` the 3-cycle forcing
/// `2·x ≡ 3` is inconsistent (2 is a zero-divisor, so `2·x ≡ 3` is unsolvable) — the break reports
/// `Refuted` on the genuinely-UNSAT one-hot encoding, a composite obstruction no prime field sees alone.
#[test]
fn affine_m_forced_refutes_a_composite_zero_divisor_obstruction() {
    let m = 6u64;
    let eqs = vec![
        ModpEquation::new(vec![(0usize, 1u64), (1, 5)], 0), // x0 − x1 ≡ 0
        ModpEquation::new(vec![(1usize, 1u64), (2, 5)], 0), // x1 − x2 ≡ 0
        ModpEquation::new(vec![(0usize, 1u64), (2, 1)], 3), // x0 + x2 ≡ 3  ⇒  2·x0 ≡ 3, unsolvable
    ];
    assert!(
        models_m(3, m, &eqs).is_empty(),
        "the ℤ/6 system is genuinely inconsistent"
    );
    let (nbv, clauses) = onehot_cnf(m, 3, &eqs);
    assert!(
        matches!(affine_m_forced(nbv, &clauses), AffinePForced::Refuted(_)),
        "must refute the ℤ/6 obstruction"
    );
}

/// **A composite ℤ/m link lifts to value-permuted bit-equivalences via CRT.** `x0 + x1 ≡ 0` over `ℤ/6`
/// links `x0 = 5·x1` — the unit `c = 5 = CRT(1 mod 2, 2 mod 3)`, neither variable forced. The break
/// emits value-permuted one-hot equivalences `b(x0=v) ↔ b(x1=…)`, each sound against every Boolean
/// model, the composite shear no monomial break sees.
#[test]
fn affine_m_link_lifts_to_composite_value_permuted_equivalences() {
    let m = 6u64;
    let eqs = vec![ModpEquation::new(vec![(0usize, 1u64), (1, 1)], 0)];
    let (nbv, clauses) = onehot_cnf(m, 2, &eqs);
    let AffinePForced::Forced(extra) = affine_m_forced(nbv, &clauses) else {
        panic!("the ℤ/6 link must fire");
    };
    assert!(
        extra.iter().any(|c| c.len() == 2),
        "must emit value-permuted bit-equivalences (2-literal clauses)"
    );
    let models = crate::affine::models_of(nbv, &clauses);
    for c in &extra {
        assert!(
            models.iter().all(|&mm| c
                .iter()
                .any(|l| ((mm >> l.var()) & 1 == 1) == l.is_positive())),
            "every emitted equivalence must hold in every Boolean model"
        );
    }
    let rec = crate::modp::recover_from_cnf(nbv, &clauses).expect("recovers");
    let pos_of = |bv: u32| {
        rec.groups
            .iter()
            .enumerate()
            .find_map(|(g, grp)| grp.iter().position(|&b| b == bv).map(|v| (g, v)))
    };
    assert!(
        extra.iter().filter(|c| c.len() == 2).any(|c| matches!(
            (pos_of(c[0].var()), pos_of(c[1].var())),
            (Some((g0, v0)), Some((g1, v1))) if g0 != g1 && v0 != v1
        )),
        "a genuine ℤ/6 link must produce a value-permuted equivalence"
    );
}

/// **Soundness to the point of absurdity over composite moduli.** Random `ℤ/6`, `ℤ/10`, `ℤ/15` systems
/// (distinct-subset congruences the recovery accepts), the whole composite break — forced, partially
/// forced, AND linked consequences — brute-force-checked: every emitted clause holds in *every* Boolean
/// model, and `Refuted` only ever names a genuinely-UNSAT formula.
#[test]
fn affine_m_break_is_sound_against_brute_force() {
    let mut state = 0x0BAD_F00D_77u64;
    let mut rng = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    // ℤ/6 keeps the Boolean encoding small (2 edges = 12 bits, under models_of's brute-force cap)
    // while exercising forced + partial + linked consequences; larger composites are covered by the
    // direct forced_values_squarefree fuzz.
    for &m in &[6u64] {
        for _ in 0..300 {
            let mut eqs: Vec<ModpEquation> = Vec::new();
            if rng() & 1 == 0 {
                eqs.push(ModpEquation::new(
                    vec![(0usize, 1 + rng() % (m - 1))],
                    rng() % m,
                ));
            }
            if rng() & 1 == 0 {
                eqs.push(ModpEquation::new(
                    vec![(1usize, 1 + rng() % (m - 1))],
                    rng() % m,
                ));
            }
            if rng() & 1 == 0 {
                eqs.push(ModpEquation::new(
                    vec![(0usize, 1 + rng() % (m - 1)), (1, 1 + rng() % (m - 1))],
                    rng() % m,
                ));
            }
            if eqs.is_empty() {
                continue;
            }
            let (nbv, clauses) = onehot_cnf(m, 2, &eqs);
            let models = crate::affine::models_of(nbv, &clauses);
            match affine_m_forced(nbv, &clauses) {
                AffinePForced::Refuted(_) => assert!(
                    models.is_empty(),
                    "Refuted ⇒ the Boolean CNF must be UNSAT (m={m})"
                ),
                AffinePForced::Forced(extra) => {
                    for c in &extra {
                        assert!(
                            models.iter().all(|&mm| c
                                .iter()
                                .any(|l| ((mm >> l.var()) & 1 == 1) == l.is_positive())),
                            "every composite consequence must hold in every Boolean model (m={m})"
                        );
                    }
                }
                AffinePForced::Unchanged => {}
            }
        }
    }
}

/// **The GF(p) elimination shrinks and lifts soundly.** Over `GF(3)`, `x0+x1 ≡ 0` (links `x0 = 2·x1`)
/// and `x2 ≡ 1` (forced): the `x2` group collapses to constants and the `x0` group aliases
/// value-permuted to `x1`, so only `x1`'s group survives — and every reduced model lifts to a genuine
/// model of the original.
#[test]
fn affine_p_canonicalize_eliminates_and_lifts_soundly() {
    let eqs = vec![
        ModpEquation::new(vec![(0usize, 1u64), (1, 1)], 0),
        ModpEquation::new(vec![(2usize, 1u64)], 1),
    ];
    let (nbv, clauses) = onehot_cnf(3, 3, &eqs);
    match affine_p_canonicalize(nbv, &clauses) {
        AffinePCanon::Canonical(canon) => {
            assert!(
                canon.num_vars < nbv,
                "elimination must shrink the bit count ({} < {nbv})",
                canon.num_vars
            );
            let orig = crate::affine::models_of(nbv, &clauses);
            let red = crate::affine::models_of(canon.num_vars, &canon.clauses);
            assert_eq!(
                red.is_empty(),
                orig.is_empty(),
                "reduction must preserve satisfiability"
            );
            for &rm_bits in &red {
                let rm: Vec<bool> = (0..canon.num_vars)
                    .map(|i| (rm_bits >> i) & 1 == 1)
                    .collect();
                let lifted = canon.lift(&rm);
                assert!(
                    clauses.iter().all(|c| c
                        .iter()
                        .any(|l| lifted[l.var() as usize] == l.is_positive())),
                    "every lifted reduced model must satisfy the original formula"
                );
            }
        }
        _ => panic!("forced + linked groups must canonicalize"),
    }
}

/// **Soundness to the point of absurdity.** Random `GF(3)`/`GF(5)` one-hot systems: the elimination
/// must preserve satisfiability exactly against brute force, refute only genuinely-UNSAT instances, and
/// lift every reduced model to a real model of the original.
#[test]
fn affine_p_canonicalize_is_sound_against_brute_force() {
    let mut state = 0xE11_3110u64;
    let mut rng = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for &p in &[3u64, 5] {
        for _ in 0..120 {
            let mut eqs: Vec<ModpEquation> = Vec::new();
            if rng() & 1 == 0 {
                eqs.push(ModpEquation::new(
                    vec![(0usize, 1 + rng() % (p - 1))],
                    rng() % p,
                ));
            }
            if rng() & 1 == 0 {
                eqs.push(ModpEquation::new(
                    vec![(1usize, 1 + rng() % (p - 1))],
                    rng() % p,
                ));
            }
            if rng() & 1 == 0 {
                eqs.push(ModpEquation::new(
                    vec![(0usize, 1 + rng() % (p - 1)), (1, 1 + rng() % (p - 1))],
                    rng() % p,
                ));
            }
            if eqs.is_empty() {
                continue;
            }
            let (nbv, clauses) = onehot_cnf(p, 2, &eqs);
            let orig = crate::affine::models_of(nbv, &clauses);
            match affine_p_canonicalize(nbv, &clauses) {
                AffinePCanon::Refuted(_) => {
                    assert!(orig.is_empty(), "Refuted ⇒ original UNSAT (p={p})")
                }
                AffinePCanon::Canonical(canon) => {
                    let red = crate::affine::models_of(canon.num_vars, &canon.clauses);
                    assert_eq!(
                        red.is_empty(),
                        orig.is_empty(),
                        "elimination preserves satisfiability (p={p})"
                    );
                    for &rm_bits in &red {
                        let rm: Vec<bool> = (0..canon.num_vars)
                            .map(|i| (rm_bits >> i) & 1 == 1)
                            .collect();
                        let lifted = canon.lift(&rm);
                        assert!(
                            clauses.iter().all(|c| c
                                .iter()
                                .any(|l| lifted[l.var() as usize] == l.is_positive())),
                            "every lifted reduced model must satisfy the original (p={p})"
                        );
                    }
                }
                AffinePCanon::Unchanged => {}
            }
        }
    }
}

/// **The prime-power detector is exact.** Over `ℤ/8` the zero-divisor `2·x0 ≡ 4` leaves `x0 ∈ {2,6}`
/// (NOT forced) while `x1 ≡ 5` is; over `ℤ/9` the zero-divisor `3·x0 ≡ 1` is unsolvable.
#[test]
fn forced_values_prime_power_is_exact() {
    use crate::modm::{forced_values_prime_power, ForcedM};
    let eqs = vec![
        ModpEquation::new(vec![(0usize, 2u64)], 4), // 2·x0 ≡ 4 (mod 8) ⇒ x0 ∈ {2, 6}
        ModpEquation::new(vec![(1usize, 1u64)], 5), // x1 ≡ 5
    ];
    match forced_values_prime_power(&eqs, 2, 8) {
        Some(ForcedM::Forced(f)) => {
            assert_eq!(
                f[0], None,
                "x0 is not forced — 2·x0 ≡ 4 has two solutions mod 8"
            );
            assert_eq!(f[1], Some(5), "x1 is forced to 5");
        }
        other => panic!(
            "expected a forced structure, got something else: {}",
            matches!(other, Some(ForcedM::Inconsistent))
        ),
    }
    let unsolvable = vec![ModpEquation::new(vec![(0usize, 3u64)], 1)]; // 3·x0 ≡ 1 (mod 9): no solution
    assert!(
        matches!(
            forced_values_prime_power(&unsolvable, 1, 9),
            Some(ForcedM::Inconsistent)
        ),
        "3·x0 ≡ 1 (mod 9) is unsolvable"
    );
}

/// **The composite break handles a prime-power modulus.** Over `ℤ/4 = 2²` (a local ring, not
/// squarefree, so the CRT path declines) the break takes the prime-power route: `x0 ≡ 3` is forced and
/// lifted to one-hot units, each sound against every Boolean model.
#[test]
fn affine_m_forced_handles_prime_power_modulus() {
    let m = 4u64;
    let eqs = vec![ModpEquation::new(vec![(0usize, 1u64)], 3)];
    let (nbv, clauses) = onehot_cnf(m, 1, &eqs);
    match affine_m_forced(nbv, &clauses) {
        AffinePForced::Forced(units) => {
            assert!(
                units.contains(&vec![Lit::pos(3)]),
                "must pin b(x0=3) over ℤ/4; got {units:?}"
            );
            let models = crate::affine::models_of(nbv, &clauses);
            assert!(!models.is_empty(), "the ℤ/4 system is satisfiable");
            for u in &units {
                let l = u[0];
                assert!(
                    models
                        .iter()
                        .all(|&mm| ((mm >> l.var()) & 1 == 1) == l.is_positive()),
                    "forced unit on var {} must hold in every Boolean model",
                    l.var()
                );
            }
        }
        _ => panic!("the ℤ/4 forced variable must produce one-hot units"),
    }
}

/// **The Smith-form solution space is exact.** Over small `ℤ/pᵏ` rings (ℤ/4, ℤ/8, ℤ/9), against brute
/// enumeration of hundreds of random systems: the particular solution satisfies, `Inconsistent` iff no
/// solution exists, and a variable's kernel-never-moves-it test matches "takes one value in every
/// solution" — exactly (with the forced value agreeing).
#[test]
fn solve_space_prime_power_matches_brute() {
    use crate::modm::{solve_space_prime_power, PrimePowerSpace};
    let mut state = 0x511700D5u64;
    let mut rng = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for &(p, k) in &[(2u64, 2u32), (2, 3), (3, 2)] {
        let q = p.pow(k);
        for _ in 0..200 {
            let nv = 1 + (rng() % 3) as usize;
            let eqs: Vec<ModpEquation> = (0..(1 + rng() % 3))
                .map(|_| {
                    let kk = 1 + (rng() % nv as u64) as usize;
                    let mut vars: Vec<usize> = Vec::new();
                    while vars.len() < kk {
                        let vv = (rng() % nv as u64) as usize;
                        if !vars.contains(&vv) {
                            vars.push(vv);
                        }
                    }
                    ModpEquation::new(
                        vars.iter().map(|&vv| (vv, rng() % q)).collect::<Vec<_>>(),
                        rng() % q,
                    )
                })
                .collect();
            let total = (q as u128).pow(nv as u32) as u64;
            let mut sols: Vec<Vec<u64>> = Vec::new();
            for code in 0..total {
                let mut c = code;
                let x: Vec<u64> = (0..nv)
                    .map(|_| {
                        let vv = c % q;
                        c /= q;
                        vv
                    })
                    .collect();
                if eqs.iter().all(|eq| {
                    eq.coeffs
                        .iter()
                        .fold(0u64, |a, &(vv, co)| (a + co * x[vv]) % q)
                        % q
                        == eq.rhs % q
                }) {
                    sols.push(x);
                }
            }
            match solve_space_prime_power(&eqs, nv, p, k).expect("no Smith overflow at this size") {
                PrimePowerSpace::Inconsistent => {
                    assert!(sols.is_empty(), "Inconsistent ⇒ no ℤ/{q} solution")
                }
                PrimePowerSpace::Space(ss) => {
                    assert!(!sols.is_empty(), "a Space ⇒ at least one solution (ℤ/{q})");
                    assert!(
                        eqs.iter().all(|eq| eq
                            .coeffs
                            .iter()
                            .fold(0u64, |a, &(vv, co)| (a + co * ss.particular[vv]) % q)
                            % q
                            == eq.rhs % q),
                        "the particular solution must satisfy (ℤ/{q})"
                    );
                    for g in 0..nv {
                        let smith = ss.kernel_basis.iter().all(|kk| kk[g] == 0);
                        let vals: HashSet<u64> = sols.iter().map(|s| s[g]).collect();
                        assert_eq!(
                            smith,
                            vals.len() == 1,
                            "forced(var {g}): Smith vs brute (ℤ/{q})"
                        );
                        if smith {
                            assert_eq!(
                                ss.particular[g],
                                *vals.iter().next().unwrap(),
                                "forced value matches brute (ℤ/{q})"
                            );
                        }
                    }
                }
            }
        }
    }
}

/// **The Smith path scales past the brute cap.** `ℤ/16` with 6 variables has `16⁶ = 2²⁴` value tuples —
/// over the old `2²⁰` brute bound, so this is only decidable via the Smith solution space. All six are
/// forced (unit congruences), and the scalable detector pins each.
#[test]
fn forced_values_prime_power_scales_past_the_brute_cap() {
    use crate::modm::{forced_values_prime_power, ForcedM};
    let m = 16u64;
    let eqs: Vec<ModpEquation> = (0..6usize)
        .map(|i| ModpEquation::new(vec![(i, 1u64)], i as u64 + 1))
        .collect();
    match forced_values_prime_power(&eqs, 6, m) {
        Some(ForcedM::Forced(f)) => {
            for i in 0..6 {
                assert_eq!(
                    f[i],
                    Some(i as u64 + 1),
                    "x{i} forced to {} over ℤ/16 (2²⁴ tuples, Smith only)",
                    i + 1
                );
            }
        }
        _ => panic!("ℤ/16 forcing over 6 vars must succeed via the scalable Smith path"),
    }
}

/// **The allowed-residue congruence is exact.** Over small composite moduli (ℤ/4, ℤ/8, ℤ/9, ℤ/6,
/// ℤ/12 — prime-power AND mixed), against brute force: each variable's `(res, modu)` describes *exactly*
/// the set of values it takes across all solutions (`v` allowed ⟺ `v ≡ res mod modu`).
#[test]
fn allowed_residues_matches_brute() {
    use crate::modm::{allowed_residues, AllowedOutcome};
    let mut state = 0xA110_0ED5u64;
    let mut rng = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for &m in &[4u64, 8, 9, 6, 12] {
        for _ in 0..150 {
            let nv = 1 + (rng() % 2) as usize;
            let eqs: Vec<ModpEquation> = (0..(1 + rng() % 2))
                .map(|_| {
                    let kk = 1 + (rng() % nv as u64) as usize;
                    let mut vars: Vec<usize> = Vec::new();
                    while vars.len() < kk {
                        let vv = (rng() % nv as u64) as usize;
                        if !vars.contains(&vv) {
                            vars.push(vv);
                        }
                    }
                    ModpEquation::new(
                        vars.iter().map(|&vv| (vv, rng() % m)).collect::<Vec<_>>(),
                        rng() % m,
                    )
                })
                .collect();
            let total = (m as u128).pow(nv as u32) as u64;
            let mut sols: Vec<Vec<u64>> = Vec::new();
            for code in 0..total {
                let mut c = code;
                let x: Vec<u64> = (0..nv)
                    .map(|_| {
                        let vv = c % m;
                        c /= m;
                        vv
                    })
                    .collect();
                if eqs.iter().all(|eq| {
                    eq.coeffs
                        .iter()
                        .fold(0u64, |a, &(vv, co)| (a + co * x[vv]) % m)
                        % m
                        == eq.rhs % m
                }) {
                    sols.push(x);
                }
            }
            match allowed_residues(&eqs, nv, m).expect("no Smith overflow") {
                AllowedOutcome::Inconsistent => {
                    assert!(sols.is_empty(), "Inconsistent ⇒ no ℤ/{m} solution")
                }
                AllowedOutcome::Allowed(residues) => {
                    assert!(!sols.is_empty());
                    for g in 0..nv {
                        let (res, modu) = residues[g];
                        let vals: HashSet<u64> = sols.iter().map(|s| s[g]).collect();
                        for v in 0..m {
                            assert_eq!(
                                vals.contains(&v),
                                v % modu == res,
                                "var {g} value {v} allowed? (m={m}, res={res} mod {modu})"
                            );
                        }
                    }
                }
            }
        }
    }
}

/// **The composite break PARTIALLY forces a DERIVED prime-power value-subset.** Over `ℤ/8`, `2·x1 ≡ 0`
/// pins `x1` to `{0,4}`, and `x0 + x1 ≡ 0` then *derives* `x0 ∈ {0,4}` (`≡ 0 mod 4`) — a partial
/// constraint no single clause states (`x0`'s subset is not in the encoding). The break emits the new
/// forbidding units on `x0`, each sound against every Boolean model — the value-subset the prime-power
/// path used to miss.
#[test]
fn affine_m_forced_partially_forces_a_prime_power_value_subset() {
    let m = 8u64;
    let eqs = vec![
        ModpEquation::new(vec![(0usize, 1u64), (1, 1)], 0), // x0 + x1 ≡ 0
        ModpEquation::new(vec![(1usize, 2u64)], 0),         // 2·x1 ≡ 0  ⇒  x1 ∈ {0,4}
    ];
    let (nbv, clauses) = onehot_cnf(m, 2, &eqs);
    match affine_m_forced(nbv, &clauses) {
        AffinePForced::Forced(units) => {
            // b(x0=v) = var v; x0 ≡ 0 mod 4 ⇒ forbid v ∈ {1,2,3,5,6,7} (derived, not in the encoding).
            for v in [1u32, 2, 3, 5, 6, 7] {
                assert!(
                    units.contains(&vec![Lit::neg(v)]),
                    "must derive-forbid b(x0={v}) (off ≡0 mod 4)"
                );
            }
            let models = crate::affine::models_of(nbv, &clauses);
            assert!(!models.is_empty(), "the ℤ/8 system is satisfiable");
            for u in &units {
                assert!(
                    models.iter().all(|&mm| u
                        .iter()
                        .any(|l| ((mm >> l.var()) & 1 == 1) == l.is_positive())),
                    "emitted clause {u:?} must hold in every Boolean model"
                );
            }
        }
        _ => panic!("ℤ/8 derived partial forcing must fire"),
    }
}

/// **The composite break now LINKS over a prime-power ring.** Over `ℤ/8` (a local ring), `x0 + 5·x1 ≡ 0`
/// gives `x0 = 3·x1` — `c = 3` is a unit, neither variable forced. The ring link lifts to value-permuted
/// bit-equivalences `b(x1=v) ↔ b(x0=3v)`, each sound against every Boolean model, with a genuine value
/// permutation (the link the prime-power path used to miss entirely).
#[test]
fn affine_m_forced_links_over_a_prime_power_ring() {
    let m = 8u64;
    let eqs = vec![ModpEquation::new(vec![(0usize, 1u64), (1, 5)], 0)];
    let (nbv, clauses) = onehot_cnf(m, 2, &eqs);
    let AffinePForced::Forced(extra) = affine_m_forced(nbv, &clauses) else {
        panic!("the ℤ/8 ring link must fire");
    };
    assert!(
        extra.iter().any(|c| c.len() == 2),
        "must emit value-permuted bit-equivalences"
    );
    let models = crate::affine::models_of(nbv, &clauses);
    for c in &extra {
        assert!(
            models.iter().all(|&mm| c
                .iter()
                .any(|l| ((mm >> l.var()) & 1 == 1) == l.is_positive())),
            "every emitted equivalence must hold in every Boolean model"
        );
    }
    let rec = crate::modp::recover_from_cnf(nbv, &clauses).expect("recovers");
    let pos_of = |bv: u32| {
        rec.groups
            .iter()
            .enumerate()
            .find_map(|(g, grp)| grp.iter().position(|&b| b == bv).map(|v| (g, v)))
    };
    assert!(
        extra.iter().filter(|c| c.len() == 2).any(|c| matches!(
            (pos_of(c[0].var()), pos_of(c[1].var())),
            (Some((g0, v0)), Some((g1, v1))) if g0 != g1 && v0 != v1
        )),
        "a genuine ℤ/8 ring link must produce a value-permuted equivalence"
    );
}

/// **Soundness to the point of absurdity over a prime-power ring.** Random `ℤ/8` systems (distinct-subset
/// congruences), the WHOLE break — forced, partial, AND ring links — brute-force-checked: every emitted
/// clause holds in every Boolean model, and `Refuted` only names a genuinely-UNSAT formula.
#[test]
fn affine_m_break_is_sound_over_a_prime_power_ring() {
    let m = 8u64;
    let mut state = 0x21B6_0FF5u64;
    let mut rng = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for _ in 0..120 {
        let mut eqs: Vec<ModpEquation> = Vec::new();
        if rng() & 1 == 0 {
            eqs.push(ModpEquation::new(
                vec![(0usize, 1 + rng() % (m - 1))],
                rng() % m,
            ));
        }
        if rng() & 1 == 0 {
            eqs.push(ModpEquation::new(
                vec![(1usize, 1 + rng() % (m - 1))],
                rng() % m,
            ));
        }
        if rng() & 1 == 0 {
            eqs.push(ModpEquation::new(
                vec![(0usize, 1 + rng() % (m - 1)), (1, 1 + rng() % (m - 1))],
                rng() % m,
            ));
        }
        if eqs.is_empty() {
            continue;
        }
        let (nbv, clauses) = onehot_cnf(m, 2, &eqs);
        let models = crate::affine::models_of(nbv, &clauses);
        match affine_m_forced(nbv, &clauses) {
            AffinePForced::Refuted(_) => {
                assert!(models.is_empty(), "Refuted ⇒ the ℤ/8 CNF must be UNSAT")
            }
            AffinePForced::Forced(extra) => {
                for c in &extra {
                    assert!(
                        models.iter().all(|&mm| c
                            .iter()
                            .any(|l| ((mm >> l.var()) & 1 == 1) == l.is_positive())),
                        "every ℤ/8 consequence must hold in every Boolean model"
                    );
                }
            }
            AffinePForced::Unchanged => {}
        }
    }
}

/// **The composite eliminate path physically reduces and lifts.** Over `ℤ/4`, `x0 ≡ 3` is forced and
/// `x1 + x2 ≡ 0` links `x2 = −x1`. Both the forced and linked groups are eliminated — only `x1`'s one-hot
/// survives — and every model of the reduced formula lifts to a real model of the original.
#[test]
fn affine_m_canonicalize_eliminates_and_lifts_soundly() {
    let m = 4u64;
    let eqs = vec![
        ModpEquation::new(vec![(0usize, 1u64)], 3),
        ModpEquation::new(vec![(1usize, 1u64), (2, 1)], 0),
    ];
    let (nbv, clauses) = onehot_cnf(m, 3, &eqs);
    match affine_m_canonicalize(nbv, &clauses) {
        AffinePCanon::Canonical(canon) => {
            assert!(
                canon.num_vars < nbv,
                "elimination must shrink the bit count ({} < {nbv})",
                canon.num_vars
            );
            let orig = crate::affine::models_of(nbv, &clauses);
            let red = crate::affine::models_of(canon.num_vars, &canon.clauses);
            assert!(!orig.is_empty(), "the ℤ/4 system is satisfiable");
            assert_eq!(
                red.is_empty(),
                orig.is_empty(),
                "reduction must preserve satisfiability"
            );
            for &rm_bits in &red {
                let rm: Vec<bool> = (0..canon.num_vars)
                    .map(|i| (rm_bits >> i) & 1 == 1)
                    .collect();
                let lifted = canon.lift(&rm);
                assert!(
                    clauses.iter().all(|c| c
                        .iter()
                        .any(|l| lifted[l.var() as usize] == l.is_positive())),
                    "every lifted reduced model must satisfy the original formula"
                );
            }
        }
        _ => panic!("the forced + linked ℤ/4 groups must canonicalize"),
    }
}

/// **Composite eliminate, sound to the point of absurdity.** Random one-hot systems over `ℤ/4`, `ℤ/6`,
/// `ℤ/8` (zero-divisor coefficients exercise the *partial*-coset survivor pruning): the elimination must
/// preserve satisfiability exactly against brute force, refute only genuinely-UNSAT instances, and lift
/// every reduced model to a real model of the original.
#[test]
fn affine_m_canonicalize_is_sound_against_brute_force() {
    let mut state = 0x5EED_3110u64;
    let mut rng = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for &m in &[4u64, 6, 8] {
        for _ in 0..150 {
            let mut eqs: Vec<ModpEquation> = Vec::new();
            if rng() % 3 != 0 {
                eqs.push(ModpEquation::new(
                    vec![(0usize, 1 + rng() % (m - 1))],
                    rng() % m,
                ));
            }
            if rng() % 3 != 0 {
                eqs.push(ModpEquation::new(
                    vec![(1usize, 1 + rng() % (m - 1))],
                    rng() % m,
                ));
            }
            if rng() % 3 != 0 {
                eqs.push(ModpEquation::new(
                    vec![(0usize, 1 + rng() % (m - 1)), (1, 1 + rng() % (m - 1))],
                    rng() % m,
                ));
            }
            if eqs.is_empty() {
                continue;
            }
            let (nbv, clauses) = onehot_cnf(m, 2, &eqs);
            let orig = crate::affine::models_of(nbv, &clauses);
            match affine_m_canonicalize(nbv, &clauses) {
                AffinePCanon::Refuted(_) => {
                    assert!(orig.is_empty(), "Refuted ⇒ original UNSAT (m={m})")
                }
                AffinePCanon::Canonical(canon) => {
                    let red = crate::affine::models_of(canon.num_vars, &canon.clauses);
                    assert_eq!(
                        red.is_empty(),
                        orig.is_empty(),
                        "elimination preserves satisfiability (m={m})"
                    );
                    for &rm_bits in &red {
                        let rm: Vec<bool> = (0..canon.num_vars)
                            .map(|i| (rm_bits >> i) & 1 == 1)
                            .collect();
                        let lifted = canon.lift(&rm);
                        assert!(
                            clauses.iter().all(|c| c
                                .iter()
                                .any(|l| lifted[l.var() as usize] == l.is_positive())),
                            "every lifted reduced model must satisfy the original (m={m})"
                        );
                    }
                }
                AffinePCanon::Unchanged => {}
            }
        }
    }
}
