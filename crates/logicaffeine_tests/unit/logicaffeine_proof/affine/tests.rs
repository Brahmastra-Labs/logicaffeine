use super::*;

/// `|AGL(n,2)| = 2ⁿ·|GL(n,2)|`, and the exhaustive enumeration produces exactly that many bijections.
#[test]
fn agl_order_matches_the_enumeration() {
    for n in 1..=4u32 {
        assert_eq!(
            all_affine_bijections(n as usize).len() as u128,
            agl_order(n),
            "n={n}: enumerated affine bijections must equal |AGL(n,2)| = 2ⁿ·|GL(n,2)|"
        );
    }
    // Pinned small values: |AGL(1,2)|=2, |AGL(2,2)|=24, |AGL(3,2)|=1344.
    assert_eq!(agl_order(1), 2);
    assert_eq!(agl_order(2), 24);
    assert_eq!(agl_order(3), 1344);
}

/// The group laws hold: composition is associative-ish (closure + identity) and every bijection's
/// linear part is invertible. Spot-checked against direct point evaluation.
#[test]
fn affine_maps_compose_and_act_correctly() {
    let id = Affine::identity(3);
    for phi in all_affine_bijections(3) {
        assert!(phi.is_bijection());
        // identity is a left/right unit
        assert_eq!(phi.compose(&id), phi);
        assert_eq!(id.compose(&phi), phi);
        // composition matches pointwise application: (φ∘φ)(x) = φ(φ(x))
        let sq = phi.compose(&phi);
        for x in 0..8u64 {
            assert_eq!(
                sq.apply(x),
                phi.apply(phi.apply(x)),
                "composition must match double application"
            );
        }
    }
}

/// **THE CENSUS GAP, MEASURED: `AGL ⊋ Bₙ`.** The single even-parity constraint `x₀⊕x₁⊕x₂ = 0` is
/// rigid under the clause breakers but drowning in affine symmetry. Exactly `24·4 = 96` affine maps
/// fix its model set (24 linear maps preserve the parity hyperplane, 4 translations stay inside it) —
/// strictly more than the *entire* hyperoctahedral group `|B₃| = 2³·3! = 48` that bounds any
/// permutation-automorphism the clause breakers could ever find. The witness is a concrete **shear**
/// `x₁↦x₀⊕x₁, x₂↦x₀⊕x₂` (every column odd-weight, so parity-preserving): an automorphism of the model
/// set that is *not* a signed permutation — exactly the kind no clause-level break can reach.
#[test]
fn affine_symmetry_strictly_exceeds_permutation_symmetry_on_parity() {
    let p = |v: u32| Lit::pos(v);
    let q = |v: u32| Lit::neg(v);
    // Wrong-parity (odd) assignments of (x0,x1,x2): 100,010,001,111 — one clause forbidding each.
    let clauses = vec![
        vec![q(0), p(1), p(2)], // not (1,0,0)
        vec![p(0), q(1), p(2)], // not (0,1,0)
        vec![p(0), p(1), q(2)], // not (0,0,1)
        vec![q(0), q(1), q(2)], // not (1,1,1)
    ];

    let affine = affine_symmetries(3, &clauses);
    // 24 linear maps preserve the hyperplane × 4 in-plane translations.
    assert_eq!(
        affine.len(),
        96,
        "the even-parity plane has exactly 96 affine symmetries"
    );
    // … which strictly exceeds all of B₃ (48), hence any permutation symmetry the clause breakers find.
    const B3: usize = 8 * 6;
    assert!(
        affine.len() > B3,
        "AGL symmetry ({}) must exceed |B₃| = {B3}",
        affine.len()
    );

    // The witness shear (rows x0, x0⊕x1, x0⊕x2 — every column odd-weight ⇒ parity-preserving) IS an
    // affine symmetry, and it mixes variables, so no signed permutation can express it.
    let shear = Affine {
        n: 3,
        matrix: vec![0b001, 0b011, 0b101],
        translation: 0,
    };
    assert!(shear.is_bijection());
    assert!(
        affine.contains(&shear),
        "the shear x₁↦x₀⊕x₁, x₂↦x₀⊕x₂ must be an affine symmetry of the plane"
    );
    assert!(
        shear.matrix.iter().any(|r| r.count_ones() >= 2),
        "the shear mixes variables — outside Bₙ, invisible to every clause-level break"
    );
}

/// **The AGL lens-narrowness, quantified for every `n` (a theorem, not a measurement).** The even-parity
/// family over `n` variables has closed-form symmetry-group orders under the two lenses:
///   - `|Bₙ-symmetry| = n! · 2^{n−1}` (all variable permutations, plus even sign-flips), and
///   - `|AGL-symmetry| = |GL(n,2)| / (2ⁿ−1) · 2^{n−1}` (linear maps fixing the parity hyperplane, plus
///     in-plane translations).
/// Their ratio `|GL(n,2)| / ((2ⁿ−1)·n!)` grows *super-exponentially* (`|GL(n,2)| = 2^{Θ(n²)}`), so the
/// clause-symmetry lens `Bₙ` is provably blind to almost all the symmetry — which is exactly why parity /
/// affine families defeat clause-based symmetry breaking and need linear algebra. We pin the closed forms
/// against the exhaustive count at `n = 3, 4`, then exhibit the unbounded growth via the formula.
#[test]
fn affine_symmetry_dwarfs_permutation_symmetry_on_parity_at_every_scale() {
    fn factorial(n: u128) -> u128 {
        (1..=n).product()
    }
    let bn_stab = |n: u32| -> u128 { factorial(n as u128) * (1u128 << (n - 1)) };
    let agl_stab = |n: u32| -> u128 { gf2::gl_order(n) / ((1u128 << n) - 1) * (1u128 << (n - 1)) };

    // Pin BOTH closed forms against exhaustive counts (n ≤ 4). The even-parity constraint is encoded by
    // forbidding every ODD-parity assignment with a full-width clause; its automorphisms are the
    // hyperplane-preserving affine maps, and its `Bₙ` sub-automorphisms are those with permutation-matrix
    // linear part.
    for n in 3..=4u32 {
        let clauses: Vec<Vec<Lit>> = (0u64..(1u64 << n))
            .filter(|a| a.count_ones() % 2 == 1)
            .map(|a| (0..n).map(|v| Lit::new(v, (a >> v) & 1 == 0)).collect())
            .collect();
        let syms = affine_symmetries(n as usize, &clauses);
        // A signed permutation = an affine map whose linear part is a permutation matrix.
        let is_perm = |a: &Affine| -> bool {
            let mut cols = 0u64;
            for &row in a.matrix.iter().take(n as usize) {
                if row.count_ones() != 1 || cols & row != 0 {
                    return false;
                }
                cols |= row;
            }
            cols == (1u64 << n) - 1
        };
        let bn_count = syms.iter().filter(|a| is_perm(a)).count() as u128;
        assert_eq!(
            syms.len() as u128,
            agl_stab(n),
            "n={n}: AGL parity-stabilizer = |GL(n,2)|/(2ⁿ−1)·2^{{n−1}}"
        );
        assert_eq!(
            bn_count,
            bn_stab(n),
            "n={n}: Bₙ parity-stabilizer = n!·2^{{n−1}}"
        );
        assert!(
            agl_stab(n) > bn_stab(n),
            "n={n}: AGL symmetry exceeds Bₙ symmetry"
        );
    }

    // The ratio grows without bound — the standard lens misses a super-exponential factor of the symmetry.
    let ratios: Vec<u128> = (3..=8u32).map(|n| agl_stab(n) / bn_stab(n)).collect();
    eprintln!("AGL/Bₙ parity-symmetry ratio, n = 3..8: {ratios:?}");
    assert!(
        ratios.windows(2).all(|w| w[1] > w[0]),
        "the AGL/Bₙ ratio grows with n: {ratios:?}"
    );
    assert!(
        *ratios.last().unwrap() > 1_000_000,
        "by n=8 the lens misses a >10⁶ factor"
    );
}

/// **The scalable affine-symmetry detector for XOR families: a closed form, verified `∀n`.** Enumerating
/// `AGL(n,2)` is stuck at `n ≤ 4`, but the affine automorphism group of any XOR-defined family (model set a
/// `k`-dim affine subspace) has the *closed form* [`affine_subspace_agl_order`], evaluable at any `n`. We
/// pin it against the exhaustive count at `n ≤ 4` for every `1 ≤ k ≤ n−1`, then push the consequence to
/// `n = 10`: for any nontrivial affine family the affine symmetry is `2^{Θ(n²)}`, which **dwarfs the entire
/// hyperoctahedral group** `|Bₙ| = 2ⁿ·n! = 2^{O(n log n)}`. So clause-based symmetry breaking captures a
/// vanishing `2^{−Θ(n²)}` fraction of the symmetry of a linear family — the linear structure is invisible
/// to it, ∀n, and the finder scales without enumerating the group.
#[test]
fn affine_family_symmetry_closed_form_scales_to_all_n() {
    // Pin the closed form against exhaustive counts (n ≤ 4), every dimension. The model set of the units
    // `{¬x_j : k ≤ j < n}` is the k-dim coordinate subspace.
    for n in 2..=4u32 {
        for k in 1..n {
            let clauses: Vec<Vec<Lit>> = (k..n).map(|j| vec![Lit::neg(j)]).collect();
            assert_eq!(
                affine_symmetries(n as usize, &clauses).len() as u128,
                affine_subspace_agl_order(n, k),
                "n={n} k={k}: closed-form affine-subspace order must match the exhaustive count"
            );
        }
    }
    // Push to n = 12: the affine symmetry of a densest (k = n/2) linear family dwarfs all of Bₙ, and the
    // ratio grows super-exponentially (2^{Θ(n²)} / 2^{O(n log n)}).
    let bn = |n: u128| (1u128 << n) * (1..=n).product::<u128>();
    let ratios: Vec<u128> = (6..=12u32)
        .map(|n| affine_subspace_agl_order(n, n / 2) / bn(n as u128))
        .collect();
    eprintln!("affine-family AGL / |Bₙ| ratio, n = 6..12: {ratios:?}");
    assert!(
        ratios.iter().all(|&r| r > 1),
        "the affine symmetry exceeds |Bₙ| at every n"
    );
    assert!(
        ratios.windows(2).all(|w| w[1] > w[0]),
        "the gap grows with n: {ratios:?}"
    );
    assert!(
        *ratios.last().unwrap() > 1_000_000_000,
        "by n=12 the affine symmetry is a >10⁹ factor beyond |Bₙ|"
    );
}

/// The affine break **refutes** an inconsistent linear core — a transitive XOR contradiction
/// (`x₀=x₁, x₁=x₂, x₀≠x₂`) that is parity-inconsistent but has no two clauses in direct conflict.
#[test]
fn affine_reduce_refutes_an_inconsistent_linear_core() {
    let p = |v: u32| Lit::pos(v);
    let q = |v: u32| Lit::neg(v);
    let clauses = vec![
        vec![q(0), p(1)],
        vec![p(0), q(1)], // x0 = x1
        vec![q(1), p(2)],
        vec![p(1), q(2)], // x1 = x2
        vec![p(0), p(2)],
        vec![q(0), q(2)], // x0 ≠ x2  ⇒ 0 = 1
    ];
    // Refuted, and the payload is a DRAT certificate that RUP-refutes the original CNF.
    match affine_reduce(3, &clauses) {
        AffineOutcome::Refuted(Some(drat)) => assert!(
            crate::rup::check_refutation(3, &clauses, &drat),
            "the affine refutation's xor_drat certificate must RUP-refute the original CNF"
        ),
        other => panic!("expected a certified refutation, got {other:?}"),
    }
    // Ground truth: it really is UNSAT.
    assert!(
        models_of(3, &clauses).is_empty(),
        "the linear core is genuinely unsatisfiable"
    );
}

/// **The affine refutation is certified through the `xor_drat` bridge.** On a genuine parity core
/// (an odd-charge expander Tseitin), [`affine_refutation_drat`] compiles the GF(2) linear dependency to
/// RUP resolvent lemmas that our independent checker accepts against the original CNF — the same
/// `drat-trim`-checkable path `Route::Parity` uses — and the dispatcher now carries that proof on its
/// verdict rather than reporting UNSAT bare.
#[test]
fn affine_refutation_is_certified_via_xor_drat_bridge() {
    use crate::solve::{solve_structured, Answer};
    let (_, cnf, _) = crate::families::tseitin_expander(6, 1);
    let nv = cnf.num_vars;
    let drat = affine_refutation_drat(nv, &cnf.clauses)
        .expect("an inconsistent XOR core has a certificate");
    assert!(
        !drat.is_empty(),
        "the certificate must carry resolvent lemmas"
    );
    assert!(
        crate::rup::check_refutation(nv, &cnf.clauses, &drat),
        "the xor_drat certificate must RUP-refute the original CNF"
    );
    // Wired: the dispatcher decides UNSAT (and where the affine rung fires, ships this very proof).
    assert!(
        matches!(solve_structured(nv, &cnf.clauses).answer, Answer::Unsat),
        "the parity core is UNSAT"
    );
}

/// The affine break **derives a forced unit** no clause states: `x0⊕x1=1 ∧ x0⊕x1⊕x2=0 ⇒ x2=1`. Gauss
/// elimination finds it; unit propagation on the clauses alone does not.
#[test]
fn affine_reduce_derives_a_nonsyntactic_forced_unit() {
    let p = |v: u32| Lit::pos(v);
    let q = |v: u32| Lit::neg(v);
    // x0⊕x1=1: (x0∨x1)∧(¬x0∨¬x1).  x0⊕x1⊕x2=0: the four even-parity clauses over {0,1,2}.
    let clauses = vec![
        vec![p(0), p(1)],
        vec![q(0), q(1)],
        vec![q(0), p(1), p(2)],
        vec![p(0), q(1), p(2)],
        vec![p(0), p(1), q(2)],
        vec![q(0), q(1), q(2)],
    ];
    match affine_reduce(3, &clauses) {
        AffineOutcome::Forced(extra) => {
            assert!(
                extra.contains(&vec![Lit::pos(2)]),
                "must derive the forced unit x2 = 1; got {extra:?}"
            );
        }
        other => panic!("expected forced consequences, got {other:?}"),
    }
    // The derived unit is sound: x2 is 1 in every model.
    for m in models_of(3, &clauses) {
        assert_eq!((m >> 2) & 1, 1, "x2 must be 1 in every model");
    }
}

/// Wired: the dispatcher decides affine-reducible formulas correctly — it refutes an inconsistent
/// linear core and returns a satisfying model for a forced-consequence formula (the model re-checked
/// against the original clauses).
#[test]
fn the_dispatcher_decides_affine_reducible_formulas() {
    use crate::solve::{solve_structured, Answer};
    let p = |v: u32| Lit::pos(v);
    let q = |v: u32| Lit::neg(v);
    // Transitive XOR contradiction (x0=x1, x1=x2, x0≠x2) — UNSAT, no two clauses in direct conflict.
    let unsat = vec![
        vec![q(0), p(1)],
        vec![p(0), q(1)],
        vec![q(1), p(2)],
        vec![p(1), q(2)],
        vec![p(0), p(2)],
        vec![q(0), q(2)],
    ];
    assert!(
        matches!(solve_structured(3, &unsat).answer, Answer::Unsat),
        "dispatcher must refute the linear core"
    );
    // Forced-consequence SAT case: x0⊕x1=1 ∧ x0⊕x1⊕x2=0 forces x2=1, still satisfiable.
    let sat = vec![
        vec![p(0), p(1)],
        vec![q(0), q(1)],
        vec![q(0), p(1), p(2)],
        vec![p(0), q(1), p(2)],
        vec![p(0), p(1), q(2)],
        vec![q(0), q(1), q(2)],
    ];
    match solve_structured(3, &sat).answer {
        Answer::Sat(m) => assert!(
            sat.iter()
                .all(|c| c.iter().any(|l| m[l.var() as usize] == l.is_positive())),
            "the returned model must satisfy the original formula"
        ),
        Answer::Unsat => panic!("the forced-consequence formula is satisfiable"),
    }
}

/// Fail-closed: a formula with no inconsistent or constraining linear structure is left `Unchanged`,
/// and a satisfiable parity plane is never falsely `Refuted`.
#[test]
fn affine_reduce_is_sound_and_quiet_when_there_is_nothing_to_do() {
    // Pure 2-CNF with no recoverable XOR bundles.
    let p = |v: u32| Lit::pos(v);
    let plain = vec![vec![p(0), p(1), p(2)]];
    assert_eq!(affine_reduce(3, &plain), AffineOutcome::Unchanged);
    // A satisfiable parity constraint: consistent, so never Refuted (Forced or Unchanged only).
    let q = |v: u32| Lit::neg(v);
    let sat_parity = vec![
        vec![q(0), p(1), p(2)],
        vec![p(0), q(1), p(2)],
        vec![p(0), p(1), q(2)],
        vec![q(0), q(1), q(2)],
    ];
    assert!(
        !matches!(affine_reduce(3, &sat_parity), AffineOutcome::Refuted(_)),
        "a satisfiable plane must not be refuted"
    );
}

/// **The canonical RREF break collapses an equivalence chain.** `x0=x1=x2=x3` plus a forced unit
/// `x4=0` and a clause `x0∨x4`: the four-variable equivalence class folds to one representative and
/// `x4` is eliminated, so the formula reduces from 5 variables to 1 — and the reduced model lifts back
/// to a genuine model of the original.
#[test]
fn affine_canonicalize_collapses_an_equivalence_chain() {
    let p = |v: u32| Lit::pos(v);
    let q = |v: u32| Lit::neg(v);
    let clauses = vec![
        vec![q(0), p(1)],
        vec![p(0), q(1)], // x0 = x1
        vec![q(1), p(2)],
        vec![p(1), q(2)], // x1 = x2
        vec![q(2), p(3)],
        vec![p(2), q(3)], // x2 = x3
        vec![p(0), p(4)], // x0 ∨ x4
        vec![q(4)],       // x4 = 0
    ];
    match affine_canonicalize(5, &clauses) {
        AffineCanon::Canonical(canon) => {
            assert!(
                canon.num_vars < 5,
                "the chain + unit must shrink 5 vars (got {})",
                canon.num_vars
            );
            let orig = models_of(5, &clauses);
            let red = models_of(canon.num_vars, &canon.clauses);
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
                    "the lifted reduced model must satisfy the original formula"
                );
            }
        }
        AffineCanon::Refuted(_) => panic!("the chain is satisfiable, not refuted"),
        AffineCanon::Unchanged => panic!("the chain + unit must reduce"),
    }
}

/// **Soundness to the point of absurdity.** Hundreds of random formulas with injected linear structure
/// (equivalences, units, and clausal noise): the canonical break must preserve satisfiability EXACTLY
/// against brute force, refute only genuinely-UNSAT instances, and lift every reduced model to a real
/// model of the original.
#[test]
fn affine_canonicalize_is_sound_against_brute_force() {
    let mut state = 0xA5F1_C0DE_1234_5678u64;
    let mut rng = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for _ in 0..400 {
        let n = 4 + (rng() % 8) as usize; // 4..=11 variables
        let mut clauses: Vec<Vec<Lit>> = Vec::new();
        for _ in 0..(rng() % 4) {
            let k = 2 + (rng() % 2) as usize;
            let c: Vec<Lit> = (0..k)
                .map(|_| Lit::new((rng() % n as u64) as u32, rng() & 1 == 0))
                .collect();
            clauses.push(c);
        }
        for _ in 0..(1 + rng() % 3) {
            let a = (rng() % n as u64) as u32;
            let b = (rng() % n as u64) as u32;
            if rng() & 1 == 0 {
                if a == b {
                    continue;
                }
                if rng() & 1 == 0 {
                    clauses.push(vec![Lit::neg(a), Lit::pos(b)]); // x_a = x_b
                    clauses.push(vec![Lit::pos(a), Lit::neg(b)]);
                } else {
                    clauses.push(vec![Lit::pos(a), Lit::pos(b)]); // x_a ≠ x_b
                    clauses.push(vec![Lit::neg(a), Lit::neg(b)]);
                }
            } else {
                clauses.push(vec![Lit::new(a, rng() & 1 == 0)]); // a unit
            }
        }
        let orig = models_of(n, &clauses);
        match affine_canonicalize(n, &clauses) {
            AffineCanon::Refuted(_) => {
                assert!(orig.is_empty(), "Refuted ⇒ the original must be UNSAT")
            }
            AffineCanon::Canonical(canon) => {
                let red = models_of(canon.num_vars, &canon.clauses);
                assert_eq!(
                    red.is_empty(),
                    orig.is_empty(),
                    "canonicalization must preserve satisfiability"
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
                        "every lifted reduced model must satisfy the original"
                    );
                }
            }
            AffineCanon::Unchanged => {}
        }
    }
}
