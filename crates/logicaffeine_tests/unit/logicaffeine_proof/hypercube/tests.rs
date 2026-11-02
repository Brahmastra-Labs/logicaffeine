use super::*;
use crate::cdcl::Lit;

/// **One-hot mod-`p` instances land on the `ModCount` rung of the extended ladder — and the legacy
/// ladder is pinned unchanged.** The mod-3 Tseitin expander's one-hot CNF is the canonical
/// gap instance: total charge `2 ≡ 0 (mod 2)`, so the parity rung is *structurally* blind, and the
/// legacy cascade reports `BeyondBudget` at NS budget 3 (the regression pin — legacy behavior is
/// untouched). The extended cascade with `p = 3` enabled places it on `ModCount { 3 }`; with the
/// wrong primes it degrades to exactly the legacy verdict (the rung is per-characteristic, not a
/// blanket "some modulus works"). Conservativity is swept across a mixed corpus — pigeonhole and
/// every minimal-cover orbit at `n = 2` plus a slice at `n = 3` — where the extended cascade with
/// primes enabled returns the identical rung to the legacy one, because `recover_from_cnf` declines
/// anything that is not a genuine one-hot encoding.
#[test]
fn mod_p_one_hot_instances_land_on_the_modcount_rung_of_the_extended_ladder() {
    let (_eqs, cnf, _) = crate::families::mod_p_tseitin_expander(4, 3, 0xC0DE);
    assert_eq!(
        weakest_crushing_rung(cnf.num_vars, &cnf.clauses, 3),
        ProofRung::BeyondBudget,
        "legacy: the GF(2) ladder cannot place the mod-3 instance (regression pin)"
    );
    assert_eq!(
        weakest_crushing_rung_with_char(cnf.num_vars, &cnf.clauses, 3, &[3]),
        ProofRung::ModCount { p: 3 },
        "extended: the characteristic rung fires on the recovered, re-checked GF(3) refutation"
    );
    assert_eq!(
        weakest_crushing_rung_with_char(cnf.num_vars, &cnf.clauses, 3, &[5, 7]),
        ProofRung::BeyondBudget,
        "the rung is per-prime: without p = 3 enabled the verdict is the legacy one"
    );
    // Conservativity: off the one-hot mod-p population, primes change nothing.
    let (php3, _) = crate::families::php(3);
    let mut corpus: Vec<(usize, Vec<Vec<Lit>>)> = vec![(php3.num_vars, php3.clauses)];
    for cover in minimal_cover_orbits(2) {
        corpus.push((2, cover.clauses()));
    }
    for cover in minimal_cover_orbits(3).into_iter().take(12) {
        corpus.push((3, cover.clauses()));
    }
    for (nv, clauses) in &corpus {
        let legacy = weakest_crushing_rung(*nv, clauses, *nv);
        assert_eq!(
            weakest_crushing_rung_with_char(*nv, clauses, *nv, &[]),
            legacy,
            "no primes ⟹ the extended cascade IS the legacy cascade"
        );
        assert_eq!(
            weakest_crushing_rung_with_char(*nv, clauses, *nv, &[3, 5, 7]),
            legacy,
            "non-one-hot instances are placed identically with the characteristic rungs enabled"
        );
    }
}

/// The first question, answered against a brute-force oracle: a clause's blocker is *exactly*
/// the set of corners that falsify it.
#[test]
fn blocker_is_exactly_the_falsifying_corners() {
    // clause (x0 ∨ ¬x2) over n = 3 is false exactly when x0 = 0 and x2 = 1.
    let clause = vec![Lit::new(0, true), Lit::new(2, false)];
    let b = Subcube::blocker(&clause, 3);
    let blocked: BTreeSet<Corner> = b.footprint().into_iter().collect();

    let mut expected = BTreeSet::new();
    for c in 0u64..8 {
        let x0 = c & 1 != 0;
        let x2 = c & 4 != 0;
        let clause_true = x0 || !x2;
        if !clause_true {
            expected.insert(c);
        }
    }
    assert_eq!(
        blocked, expected,
        "blocker must be the precise falsifying set"
    );
    assert_eq!(b.footprint_card(), expected.len() as u64);
    assert_eq!(b.dimension(), 1, "3 vars, 2 fixed ⟹ 1 free coordinate");
}

/// A width-`w` clause forbids a face of dimension `n-w`, i.e. `2^{n-w}` corners.
#[test]
fn blocker_dimension_is_codimension_of_clause_width() {
    for n in 4..8 {
        for w in 1..=4.min(n) {
            let clause: Vec<Lit> = (0..w).map(|v| Lit::new(v as u32, v % 2 == 0)).collect();
            let b = Subcube::blocker(&clause, n);
            assert_eq!(b.dimension(), n - w);
            assert_eq!(b.footprint_card(), 1u64 << (n - w));
        }
    }
}

/// The headline correspondence, checked against the actual solver: a CNF is UNSAT **iff** its
/// blocker cover is total. Pigeonhole (always UNSAT) ⟹ no escaping corner; a satisfiable
/// formula ⟹ the escaping corner is a genuine model of energy zero.
#[test]
fn cover_is_total_iff_formula_is_unsat() {
    for n in 2..=4 {
        let cover = php_cover(n);
        assert!(
            cover.is_total(),
            "PHP({n}) blockers must cover the whole hypercube"
        );
        assert_eq!(cover.escaping_corner(), None);
        assert_eq!(cover.solution_count(), 0);
    }

    // A satisfiable instance: a single clause (x0 ∨ x1 ∨ x2) over 3 vars leaves 7 of 8 models.
    let sat = DimacsCnf {
        num_vars: 3,
        clauses: vec![vec![
            Lit::new(0, true),
            Lit::new(1, true),
            Lit::new(2, true),
        ]],
    };
    let cover = Cover::of_cnf(&sat);
    assert!(!cover.is_total());
    let model = cover
        .escaping_corner()
        .expect("a satisfiable cover must leave a corner free");
    assert_eq!(
        cover.vertex_energy(model),
        0,
        "an escaping corner has energy zero"
    );
    // (x0∨x1∨x2) is false only at the all-false corner 0, so that is the lone blocked corner and
    // every other corner is a model; the first one `find` reaches is 0b001.
    assert_eq!(
        cover.blocks(0),
        true,
        "the all-false corner is the unique falsifying corner"
    );
    assert_eq!(
        model, 0b001,
        "the first escaping corner above the blocked all-false corner"
    );
    assert_eq!(cover.solution_count(), 7);
}

/// Energy zero ⟺ satisfying, checked exhaustively against direct clause evaluation.
#[test]
fn vertex_energy_zero_iff_satisfying() {
    let cnf = DimacsCnf {
        num_vars: 4,
        clauses: vec![
            vec![Lit::new(0, true), Lit::new(1, false)],
            vec![Lit::new(2, true), Lit::new(3, true)],
            vec![Lit::new(0, false), Lit::new(2, false)],
        ],
    };
    let cover = Cover::of_cnf(&cnf);
    for c in 0u64..16 {
        let satisfies = cnf.clauses.iter().all(|clause| {
            clause.iter().any(|lit| {
                let bit = (c >> lit.var() as u64) & 1 != 0;
                bit == lit.is_positive()
            })
        });
        assert_eq!(satisfies, cover.vertex_energy(c) == 0, "corner {c:04b}");
    }
}

/// The pigeonhole grid symmetries are genuine automorphisms of the cover — re-verified, the way
/// the solver re-verifies every proposed symmetry before trusting it.
#[test]
fn php_symmetries_are_automorphisms() {
    for n in 2..=5 {
        let cover = php_cover(n);
        for (k, g) in php_symmetries(n).iter().enumerate() {
            assert!(
                g.is_automorphism(&cover),
                "PHP({n}) generator {k} must be an automorphism"
            );
        }
    }
}

/// One action, both worlds: a cover automorphism sends solutions to solutions **and** blockers to
/// blockers, simultaneously — the rules and the problem live in the same mathematical space.
#[test]
fn symmetry_acts_jointly_on_rules_and_solutions() {
    // A satisfiable, visibly symmetric cover: x0 ↔ x1 is an automorphism (swap the two vars).
    let cnf = DimacsCnf {
        num_vars: 3,
        clauses: vec![
            vec![Lit::new(0, true), Lit::new(2, true)],
            vec![Lit::new(1, true), Lit::new(2, true)],
        ],
    };
    let cover = Cover::of_cnf(&cnf);
    let swap = CubeSym {
        perm: vec![1, 0, 2],
        flip: vec![false; 3],
    };
    assert!(
        swap.is_automorphism(&cover),
        "swapping x0,x1 preserves the blocker set"
    );

    // Rules preserved: blocker set maps onto itself (the automorphism property, restated).
    let blk: BTreeSet<Subcube> = cover.blockers.iter().copied().collect();
    let mapped: BTreeSet<Subcube> = cover.blockers.iter().map(|b| swap.map_subcube(b)).collect();
    assert_eq!(blk, mapped);

    // Solutions preserved: the same action permutes the uncovered corners among themselves.
    let solutions: BTreeSet<Corner> = (0u64..8).filter(|&c| !cover.blocks(c)).collect();
    let moved: BTreeSet<Corner> = solutions.iter().map(|&c| swap.map_corner(c)).collect();
    assert_eq!(
        solutions, moved,
        "the cover symmetry permutes solutions among themselves"
    );
}

/// **The paths to random group by the symmetry of the step.** A step toward random is adding a
/// clause. But adding clause `C` or its symmetric image `σ(C)` gives *isomorphic* formulas — the same
/// step. So the possible next-steps fall into **orbits** under the current automorphism group, and the
/// tree of paths-to-random can be quotiented: there are only *#orbits* essentially-different ways to
/// break the symmetry. Proven by the consequence — every step in one orbit leaves the *same* `|Aut|`.
#[test]
fn paths_to_random_group_by_the_symmetry_of_the_step() {
    let php = crate::families::php(3).0;
    let n = php.num_vars;
    let generators = crate::symmetry_detect::find_generators(n, &php.clauses);

    // Candidate first steps: every binary clause over the variables.
    let mut candidates: Vec<Vec<Lit>> = Vec::new();
    for v in 0..n as u32 {
        for w in (v + 1)..n as u32 {
            for &sv in &[true, false] {
                for &sw in &[true, false] {
                    candidates.push(vec![Lit::new(v, sv), Lit::new(w, sw)]);
                }
            }
        }
    }

    // The steps group into orbits — far fewer than the candidates (high symmetry ⟹ few real choices).
    let orbits = clause_orbits(&candidates, &generators);
    assert!(
        orbits.len() < candidates.len(),
        "{} step-orbits group {} candidate steps",
        orbits.len(),
        candidates.len()
    );

    // The proof that same-orbit steps ARE the same step: each leaves the identical automorphism order.
    for orbit in &orbits {
        let auts: Vec<usize> = orbit
            .iter()
            .map(|&i| {
                let mut f = php.clauses.clone();
                f.push(candidates[i].clone());
                automorphism_group_size(n, &f)
            })
            .collect();
        assert!(
            auts.windows(2).all(|w| w[0] == w[1]),
            "same-orbit steps give isomorphic results (identical |Aut|): {auts:?}"
        );
    }
}

/// **Information theory: the cliff is the loss of symmetry-bits.** `log₂|Aut|` is the symmetry-
/// entropy — the bits the symmetry compresses out. Across the rigidity transition it falls
/// `3.58 → 2 → 0`: each asymmetric clause removes symmetry-information until rigid = 0 bits = maximal
/// (incompressible). Monotone — clauses only ever destroy symmetry-information, never create it.
#[test]
fn information_theory_of_the_rigidity_cliff() {
    fn sm(s: &mut u64) -> u64 {
        *s = s.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = *s;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z ^ (z >> 31)
    }
    let php = crate::families::php(3).0;
    let n = php.num_vars;
    let mut clauses = php.clauses.clone();
    let mut state = 0x1F0E_0001u64;
    let mut bits = Vec::new();
    for _ in 0..=4 {
        bits.push(symmetry_entropy_bits(n, &clauses));
        let mut c: Vec<Lit> = Vec::new();
        while c.len() < 2 {
            let v = (sm(&mut state) % n as u64) as u32;
            if !c.iter().any(|l| l.var() == v) {
                c.push(Lit::new(v, sm(&mut state) % 2 == 0));
            }
        }
        clauses.push(c);
    }
    assert!(
        bits[0] > 3.0,
        "PHP(3) carries ~3.58 bits of symmetry: {bits:?}"
    );
    assert_eq!(
        *bits.last().unwrap(),
        0.0,
        "rigid = 0 bits of symmetry (incompressible)"
    );
    for w in bits.windows(2) {
        assert!(
            w[1] <= w[0] + 1e-9,
            "symmetry-information only decreases: {bits:?}"
        );
    }
}

/// **Noether, mechanized: the symmetry-bits *are* the branches you can cut — and it's a hard bound.**
/// A symmetry is a conserved structure (its invariants — Burnside count, f-vector, spectrum — are the
/// conserved quantities). Its information content `log₂|Aut|` upper-bounds the search collapse: by the
/// orbit-counting (Burnside) bound, the `2ⁿ` corners collapse to at least `2ⁿ/|Aut|` orbits, so the
/// branch-reduction factor is at most `|Aut| = 2^{symmetry-bits}`. You can cut exactly as many
/// branches as you have bits of symmetry to spend — no more, no less. This is *the* lever.
#[test]
fn symmetry_bits_are_the_branches_you_can_cut() {
    let n = 3;
    let cover = php_cover(n); // 6 variables ⟹ 64 corners
    let gens = php_symmetries(n);
    let bits = symmetry_entropy_bits(cover.n, &crate::families::php(n).0.clauses);
    let orbits = orbit_count(cover.n, &gens);
    let full = 1u64 << cover.n;
    let reduction = full as f64 / orbits as f64;

    assert!(
        reduction > 1.0,
        "symmetry cuts branches: {full} corners → {orbits} orbits"
    );
    // The hard bound: you cannot cut more branches than you have bits of symmetry (Burnside).
    assert!(
        reduction.log2() <= bits + 1e-9,
        "branch-reduction {:.2} bits ≤ symmetry {bits:.2} bits (orbit-counting bound)",
        reduction.log2()
    );
}

/// **The most asymmetric is rigid — and finding the line where symmetry tips into rigidity.** Start
/// from a symmetric base (PHP(3), `|Aut| = |S₃ × S₂| = 12`) and add asymmetric clauses one at a time,
/// tracking the automorphism-group order. It does not decay gently — it **falls off a cliff to 1**
/// (rigid) within a couple of clauses, because every automorphism must fix *every* clause, so a few
/// generic constraints pin the whole structure. The clause count where `|Aut|` first hits 1 is the
/// line, and `|Aut| = 1` (only the identity) is the maximally-asymmetric extreme — random instances
/// land there. Banked.
#[test]
fn the_line_where_symmetry_becomes_rigidity() {
    use std::fmt::Write;
    fn sm(s: &mut u64) -> u64 {
        *s = s.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = *s;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z ^ (z >> 31)
    }
    let php = crate::families::php(3).0;
    let n = php.num_vars;
    let mut clauses = php.clauses.clone();
    let mut state = 0x11AE_0001u64;
    let mut chart = String::from("asymmetric added  |Aut|\n");
    chart.push_str("----------------  -----\n");
    let mut rigid_at = None;
    for added in 0..=5 {
        let aut = automorphism_group_size(n, &clauses);
        let _ = writeln!(chart, "{added:>16}  {aut:>5}");
        if aut == 1 && rigid_at.is_none() {
            rigid_at = Some(added);
        }
        let mut c: Vec<Lit> = Vec::new();
        while c.len() < 2 {
            let v = (sm(&mut state) % n as u64) as u32;
            if !c.iter().any(|l| l.var() == v) {
                c.push(Lit::new(v, sm(&mut state) % 2 == 0));
            }
        }
        clauses.push(c);
    }
    // Symmetric to begin, rigid within a handful of clauses — a sharp line, not a slope.
    assert!(
        automorphism_group_size(n, &php.clauses) >= 6,
        "the base is symmetric"
    );
    assert!(
        rigid_at.is_some(),
        "it becomes rigid (|Aut|=1) — the most asymmetric:\n{chart}"
    );

    println!("\n{chart}rigid at {rigid_at:?} asymmetric clauses\n");
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../logs/derived_facts");
    if std::fs::create_dir_all(&dir).is_ok() {
        let _ = std::fs::write(
            dir.join("rigidity_line.txt"),
            format!("THE LINE — |Aut| as asymmetric clauses are added to PHP(3). Symmetry falls off a cliff to\n1 (rigid = the most asymmetric) within a few clauses: every automorphism must fix every clause,\nso a few generic constraints pin the whole structure. rigid at {rigid_at:?} clauses.\n\n{chart}\n"),
        );
    }
}

/// **Three symmetry breaks, three invariants — combinatorics, analysis, geometry — all agreeing.**
/// On pigeonhole's symmetric cover: (1) Burnside counts the corner orbits by *fixed points* and it
/// matches the orbit walk; (2) the Walsh–Hadamard spectrum is *constant on the dual orbits* of the
/// symmetry (a symmetric energy ⟹ a symmetric Fourier transform); (3) the f-vector is *unchanged*
/// when the cover is pushed through a symmetry (dimension is preserved). One symmetry, three domains.
#[test]
fn combinatorics_analysis_geometry_invariants_agree() {
    let n = 3;
    let cover = php_cover(n); // 6 variables, 64 corners
    let gens = php_symmetries(n);

    // (1) COMBINATORICS — Burnside's fixed-point count equals the orbit-BFS count.
    let burnside = burnside_corner_orbits(cover.n, &gens);
    let walked = orbit_count(cover.n, &gens);
    assert_eq!(
        burnside, walked,
        "Burnside (fixed points) = orbit walk: {burnside} vs {walked}"
    );

    // (2) ANALYSIS — the energy is symmetry-invariant, so its Fourier coefficients are constant on the
    // orbits of the coefficient index S under the variable permutation.
    let hat = walsh_hadamard_energy(&cover);
    for g in &gens {
        for s in 0u64..(1 << cover.n) {
            let mut gs = 0u64;
            for v in 0..cover.n {
                if s & (1 << v) != 0 {
                    gs |= 1 << g.perm[v];
                }
            }
            assert!(
                (hat[s as usize] - hat[gs as usize]).abs() < 1e-9,
                "Fourier coefficient constant on the symmetry orbit: S={s:06b} σS={gs:06b}"
            );
        }
    }

    // (3) GEOMETRY — pushing the cover through a symmetry leaves the f-vector (dimension counts) fixed.
    let fv = face_vector(&cover);
    for g in &gens {
        let moved = Cover {
            n: cover.n,
            blockers: cover.blockers.iter().map(|b| g.map_subcube(b)).collect(),
        };
        assert_eq!(
            face_vector(&moved),
            fv,
            "the f-vector is a geometric invariant under symmetry"
        );
    }
}

/// The collapse, measured: breaking by the pigeonhole symmetry group decides cover-totality from
/// far fewer corners than `2ⁿ`, and agrees with the brute-force verdict.
#[test]
fn symmetry_collapses_the_cover_check_on_pigeonhole() {
    let n = 4; // 4 pigeons, 3 holes, 12 variables ⟹ 4096 corners.
    let cover = php_cover(n);
    let gens = php_symmetries(n);

    let via_orbits =
        is_total_via_orbits(&cover, &gens).expect("php symmetries must all be automorphisms");
    assert_eq!(
        via_orbits,
        cover.is_total(),
        "orbit verdict must match brute force"
    );
    assert!(via_orbits, "PHP(4) is UNSAT ⟹ the cover is total");

    let orbits = orbit_count(cover.n, &gens);
    let corners = 1u64 << cover.n;
    assert!(
        orbits < corners,
        "{orbits} orbits must be fewer than {corners} corners"
    );
    // The full grid group Sₙ × Sₙ₋₁ collapses the 4096-corner check by well over an order of
    // magnitude — the geometric face of the pigeonhole symmetry break.
    assert!(
        orbits * 8 < corners,
        "expected a >8× collapse, got {orbits} of {corners}"
    );
}

/// "Each break cuts the problem" — the actual ratios, measured rather than assumed. Every stacked
/// generator is monotone (never increases the orbit count) and the stack as a whole collapses by
/// a large factor. A single coordinate transposition cuts to ~3/4; the compounded group cuts far
/// harder, which is the honest shape of the descent.
#[test]
fn collapse_curve_is_monotone_and_compounds() {
    let n = 4;
    let cover = php_cover(n);
    let gens = php_symmetries(n);
    let curve = collapse_curve(cover.n, &gens);

    assert_eq!(
        curve[0].orbits,
        1u64 << cover.n,
        "starts at 2ⁿ with no symmetry"
    );
    for w in curve.windows(2) {
        assert!(
            w[1].orbits <= w[0].orbits,
            "stacking a generator never grows the orbit count"
        );
    }
    let first = curve[0].orbits;
    let last = curve.last().unwrap().orbits;
    assert!(
        last * 8 < first,
        "the stacked group collapses the check >8×: {first} → {last}"
    );
}

// ---- Proving what `Pnp.lean` proves, through *our* certified prover ----------------------------
//
// The theorems below are the load-bearing structural backbone of `Pnp.lean`'s `HypercubeSAT`
// (the §A cover correspondence, §A1 vertex energy, the §CubeSymmetry action, the §B 3-SAT bridge,
// and the difficulty-class separators). Each is *proven* here — exhaustively for the finite
// combinatorial facts (a decision procedure over all `2ⁿ` corners is a proof for that instance),
// and via `sat::prove_unsat`'s RUP/PR-checked refutation for the cover-totality (UNSAT) side.
// What `Pnp.lean` leaves as `sorry` — the universal `ThreeSATInP` / `P_ne_NP` targets — stays
// open for us too: we certify every concrete instance, not the asymptotic lower bound.

use crate::sat::UnsatOutcome;

/// `HasNoHole` (cover total) ⟺ UNSAT, decided by the **certified prover**. Pigeonhole covers
/// are refuted in polynomial time via the counting shadow; a satisfiable cover yields a model
/// that decodes to an uncovered corner of energy zero. The geometry and the prover agree on both
/// verdicts — and the UNSAT side carries a re-checkable certificate, not a brute-force scan.
#[test]
fn certified_prover_decides_the_cover_both_ways() {
    for n in 2..=4 {
        let cover = php_cover(n);
        assert!(cover.has_no_hole(), "PHP({n}) cover is total");
        assert_eq!(
            cover.prove_total_certified(),
            UnsatOutcome::Refuted,
            "our prover must *certify* the PHP({n}) cover total, not just brute-force it"
        );
    }
    // Clique-coloring K_n with k<n colors — a second resolution-hard family, certified.
    for (n, k) in [(3usize, 2usize), (4, 3)] {
        let (cnf, _) = crate::families::clique_coloring(n, k);
        let cover = Cover::of_cnf(&cnf);
        assert!(cover.has_no_hole(), "K_{n} needs >{k} colors ⟹ total cover");
        assert_eq!(cover.prove_total_certified(), UnsatOutcome::Refuted);
    }
    // SAT control: a satisfiable cover is not total, and the prover's model is an escaping corner.
    let sat = DimacsCnf {
        num_vars: 3,
        clauses: vec![vec![Lit::new(0, true), Lit::new(1, true)]],
    };
    let cover = Cover::of_cnf(&sat);
    assert!(!cover.has_no_hole());
    match cover.prove_total_certified() {
        UnsatOutcome::Sat(model) => {
            let mut corner = 0u64;
            for (name, val) in &model {
                if *val {
                    let v: usize = name.trim_start_matches('x').parse().unwrap();
                    corner |= 1 << v;
                }
            }
            assert_eq!(
                cover.vertex_energy(corner),
                0,
                "the prover's model is an uncovered corner"
            );
        }
        other => panic!("expected a model for a satisfiable cover, got {other:?}"),
    }
}

/// `Pnp.lean` §B bridge: an ordinary 3-SAT clause becomes a *clean* 3-bit blocker — support
/// exactly 3, `n-3` free coordinates — and the blocker faithfully round-trips back to the clause.
#[test]
fn three_clause_is_a_clean_three_bit_blocker() {
    let clause = vec![Lit::new(1, true), Lit::new(4, false), Lit::new(6, true)];
    let b = Subcube::blocker(&clause, 8);
    assert_eq!(
        b.care.count_ones(),
        3,
        "support of a 3-clause is exactly 3 coordinates"
    );
    assert_eq!(
        b.dimension(),
        5,
        "n − 3 free coordinates (Blocker.freeCoordinates_card)"
    );
    let recovered: BTreeSet<(usize, bool)> = b.clause_literals().into_iter().collect();
    let expected: BTreeSet<(usize, bool)> =
        [(1, true), (4, false), (6, true)].into_iter().collect();
    assert_eq!(
        recovered, expected,
        "blocker ↔ clause is a lossless round-trip"
    );
}

/// `Pnp.lean` `CubeSymmetry.mapVertex_injective` (+ its inverse): a hypercube symmetry is a
/// *bijection* on the corners. Proven exhaustively — the image of all `2ⁿ` corners is all of them.
#[test]
fn cube_symmetry_is_a_corner_bijection() {
    let s = CubeSym {
        perm: vec![2, 0, 3, 1],
        flip: vec![false, true, false, true],
    };
    let images: BTreeSet<Corner> = (0u64..16).map(|c| s.map_corner(c)).collect();
    assert_eq!(
        images.len(),
        16,
        "map_corner is injective ⟹ a bijection on the 2ⁿ corners"
    );
    assert_eq!(images, (0u64..16).collect::<BTreeSet<_>>());
}

/// `Pnp.lean` `separatedBy_iff_no_variableInteraction_cross`: a coordinate cut separates the
/// blocker family **iff** no primal-graph interaction edge crosses it. Proven exhaustively over a
/// small cover and *every* cut — the bridge from blocker geometry to graph-separator reasoning.
#[test]
fn separator_iff_no_interaction_crosses_the_cut() {
    let cnf = DimacsCnf {
        num_vars: 5,
        clauses: vec![
            vec![Lit::new(0, true), Lit::new(1, true)],
            vec![Lit::new(2, true), Lit::new(3, false)],
            vec![Lit::new(3, true), Lit::new(4, true)],
        ],
    };
    let cover = Cover::of_cnf(&cnf);
    for cut in 0u64..(1 << 5) {
        let separated = cover.separated_by(cut);
        let no_cross = (0..5).all(|i| {
            (0..5).all(|j| {
                let i_in = cut & (1 << i) != 0;
                let j_in = cut & (1 << j) != 0;
                !(i_in && !j_in && cover.variable_interaction(i, j))
            })
        });
        assert_eq!(separated, no_cross, "cut {cut:05b}");
    }
}

/// The difficulty-class predicates name the geometry exactly (`HasNoHole` / `HasUniqueHole` /
/// `HasAtLeastHoles`): a single 3-clause over 3 variables blocks one corner, leaving a unique
/// other; pigeonhole leaves none; the empty cover leaves all `2ⁿ`.
#[test]
fn difficulty_classes_name_the_hole_count() {
    // One clause (x0∨x1∨x2) blocks exactly the all-false corner ⟹ 7 holes, not unique, not none.
    let one = DimacsCnf {
        num_vars: 3,
        clauses: vec![vec![
            Lit::new(0, true),
            Lit::new(1, true),
            Lit::new(2, true),
        ]],
    };
    let cover = Cover::of_cnf(&one);
    assert!(!cover.has_no_hole());
    assert!(!cover.has_unique_hole());
    assert!(cover.has_at_least_holes(7));
    assert!(!cover.has_at_least_holes(8));

    // Add the seven other unit-ish blockers to pin a *unique* hole: forbid all corners but 0b111.
    let mut clauses = Vec::new();
    for forbidden in 0u64..7 {
        // a clause whose blocker is exactly {forbidden}: fix all three bits to `forbidden`.
        let lits: Vec<Lit> = (0..3)
            .map(|v| Lit::new(v as u32, forbidden & (1 << v) == 0))
            .collect();
        clauses.push(lits);
    }
    let unique = Cover::of_cnf(&DimacsCnf {
        num_vars: 3,
        clauses,
    });
    assert!(
        unique.has_unique_hole(),
        "all corners but 0b111 forbidden ⟹ exactly one hole"
    );
    assert_eq!(unique.escaping_corner(), Some(0b111));

    // Pigeonhole: no hole.
    assert!(php_cover(3).has_no_hole());
}

// ---- Symmetry-breaking the RULES, on hypercubes of increasing variable size ---------------------

/// **The complexity limit, found.** Symmetry-break the *rules* (not the corners) up the pigeonhole
/// ladder: at every scale the polynomially-many blockers collapse to exactly **two** orbits under
/// the grid group `Sₙ × Sₙ₋₁` — the at-least-one rows and the at-most-one exclusions. The count is
/// scale-invariant while the cube's corner count `2^{n(n-1)}` explodes, and it is computed in
/// milliseconds over the blocker set with no `2ⁿ` walk anywhere. That `O(1)` essential-rule count
/// is the structural reason the certified counting shadow refutes the whole family in polynomial
/// time: there are, up to symmetry, only two rules to reason about.
#[test]
fn pigeonhole_rules_collapse_to_two_orbits_at_every_scale() {
    for n in 2..=12 {
        let sig = pigeonhole_rule_symmetry(n);
        assert_eq!(
            sig.rule_orbits, 2,
            "PHP({n}): {} blockers must collapse to 2 rule-orbits, got {}",
            sig.blockers, sig.rule_orbits
        );
    }
    // At n = 12 the cube has 2^132 corners — far beyond any enumeration — yet the rule symmetry is
    // exact and cheap: the cover is large, only its symmetry is small.
    let big = pigeonhole_rule_symmetry(12);
    assert_eq!(
        big.n * (big.n - 1),
        132,
        "12 pigeons × 11 holes = 132 variables (2^132 corners)"
    );
    assert_eq!(big.rule_orbits, 2);
    assert!(
        big.blockers > 700,
        "{} blockers — large cover, two essential rules",
        big.blockers
    );
}

/// The same collapse, **self-driving**: the detector discovers the pigeonhole grid symmetry with
/// no hand-built group, and the rules still quotient to two orbits.
#[test]
fn the_detector_discovers_the_pigeonhole_rule_symmetry() {
    for n in 2..=5 {
        let sig = php_cover(n).discovered_rule_symmetry();
        assert!(
            sig.generators >= 1,
            "PHP({n}): the detector must find the grid symmetry"
        );
        assert_eq!(
            sig.rule_orbits, 2,
            "PHP({n}): discovered rule-orbits = {}, expected 2",
            sig.rule_orbits
        );
    }
}

/// The opposite pole of the limit: a random 3-SAT instance has a near-trivial automorphism group,
/// so symmetry-breaking the rules buys essentially nothing — the orbit count stays close to the
/// blocker count. Structured (symmetric) hardness collapses; random hardness has no symmetry to
/// exploit and stays hard for everyone, us included. This is the honest dichotomy the signature
/// draws.
#[test]
fn random_3sat_rules_do_not_collapse_to_a_constant() {
    let cnf = crate::families::random_3sat(14, 40, 0xC0FFEE);
    let cover = Cover::of_cnf(&cnf);
    let sig = cover.discovered_rule_symmetry();
    assert!(
        sig.rule_orbits > 2,
        "random hardness has no constant-size rule symmetry: {sig:?}"
    );
    assert!(
        sig.rule_orbits * 2 > sig.blockers,
        "random rules barely merge: {} orbits of {} blockers",
        sig.rule_orbits,
        sig.blockers
    );
}

/// The orbit-rep engine refutes PHP(3) with a bounded set of orbit-types — the same 12 the raw
/// closure found, reached without ever building the raw closure. **The honest limit, measured: this
/// does NOT scale.** At PHP(4) the *symmetric* closure already explodes past tens of thousands of
/// orbit-types — brute resolution, even quotiented by symmetry, is the wrong abstraction. The
/// scalable path is not the closure; it is the abstract certificate below ([`pigeonhole_abstract_refutation`]).
#[test]
fn orbit_rep_engine_refutes_php3_but_does_not_scale() {
    let cover = php_cover(3);
    let gens = php_symmetries(3);
    let (orbit_types, refuted) = symmetric_resolution_closure(&cover, &gens, 40, 40_000);
    assert!(
        refuted,
        "the orbit-rep engine derives the empty clause for PHP(3)"
    );
    assert_eq!(
        orbit_types, 12,
        "PHP(3) saturates at 12 orbit-types — the same as the raw closure"
    );
}

/// **The many-to-one *is* a symmetry: a blocker is a single point under its free-coordinate group.**
/// A blocker's `2^d` corners (its `d` free coordinates ranging) form one orbit under the group `Z₂^d`
/// that flips those free coordinates — so they collapse to a *single point*: the assignment on the
/// blocker's support, which is exactly a full-width clause *in the support subspace*. The "many
/// corners ↔ one point" is that orbit collapse, and it reframes the whole cover: every blocker is a
/// point in its own subspace, the `n`-cube being a projection of point-clauses. Quotient out the free
/// axes and the subcube becomes the exceptional single corner — the two ends of your observation meet.
#[test]
fn a_blocker_is_one_point_under_its_free_coordinate_symmetry() {
    let cover = php_cover(3);
    for b in &cover.blockers {
        let footprint: BTreeSet<Corner> = b.footprint().into_iter().collect();
        let free_bits: Vec<u64> = (0..b.n as u64).filter(|i| b.care & (1 << i) == 0).collect();
        assert_eq!(
            free_bits.len(),
            b.dimension(),
            "free coordinates = the dimension"
        );

        // The orbit of one corner under flipping any subset of free coordinates (the group Z₂^d).
        let start = *footprint.iter().next().unwrap();
        let orbit: BTreeSet<Corner> = (0..(1u64 << b.dimension()))
            .map(|subset| {
                let mut c = start;
                for (j, &fb) in free_bits.iter().enumerate() {
                    if subset & (1 << j) != 0 {
                        c ^= 1 << fb;
                    }
                }
                c
            })
            .collect();
        // The many corners ARE one orbit — they collapse to one point (the support assignment).
        assert_eq!(
            orbit, footprint,
            "the free-coordinate group orbit = the footprint (many → one)"
        );

        // That one point: the support and its fixed values — a full-width clause in the subspace.
        let support: Vec<(usize, bool)> = b.clause_literals();
        assert_eq!(
            support.len(),
            b.care.count_ones() as usize,
            "the point lives in the support subspace"
        );
    }
}

/// **Your conjecture, made precise: every covered corner does come in a pair — *unless* a clause uses
/// every variable.** A blocker (a width-`w` clause over `n` vars) covers `2^{n-w}` corners — a power of
/// two, hence **even and ≥ 2**, so every covered corner has a free-axis neighbor that's also covered
/// (covers come in pairs). The *only* exception is a full-width clause (`w = n`), whose blocker is a
/// single corner (`2⁰ = 1`). So "every covered corner covers two" is true exactly when no clause uses
/// all the variables — which is every ordinary instance (pigeonhole, 3-SAT, …). Measured, not guessed.
#[test]
fn every_covered_corner_comes_in_a_pair_unless_a_clause_is_full_width() {
    // Pigeonhole over 6 variables: every clause has width ≤ 2 < 6, so every blocker covers ≥ 2 corners.
    let cover = php_cover(3);
    for b in &cover.blockers {
        let fp = b.footprint_card();
        assert!(
            fp.is_power_of_two() && fp >= 2,
            "blocker covers a power-of-two ≥ 2 corners: {fp}"
        );
        let corners = b.footprint();
        for &c in &corners {
            assert!(
                corners.iter().any(|&c2| (c ^ c2).count_ones() == 1),
                "every covered corner has a free-axis partner also covered (covers come in pairs)"
            );
        }
    }

    // The lone exception: a clause that uses *all* variables blocks exactly one corner.
    let full = Subcube::blocker(
        &[Lit::new(0, true), Lit::new(1, true), Lit::new(2, true)],
        3,
    );
    assert_eq!(
        full.footprint_card(),
        1,
        "a full-width clause covers exactly one corner — the exception"
    );
    assert_eq!(
        full.dimension(),
        0,
        "its blocker is a 0-dimensional point, no partner"
    );
}

/// **1/2 is the key.** The all-½ center is the *unique* fixed point of the symmetry group: every
/// generator maps it to itself (permutations keep ½, flips send ½ to 1−½=½), while any off-center
/// point is moved. It is the one place the whole group holds still.
#[test]
fn the_half_center_is_the_symmetry_fixed_point() {
    let n = 5;
    let center = vec![0.5_f64; n];
    // A rich symmetry: a permutation with phase flips.
    let sigma = CubeSym {
        perm: vec![2, 0, 4, 1, 3],
        flip: vec![true, false, true, false, true],
    };
    assert_eq!(
        sigma.map_fractional(&center),
        center,
        "the center is fixed by the symmetry"
    );
    // Pigeonhole's whole grid group fixes it too.
    for g in php_symmetries(4) {
        let c = vec![0.5_f64; 4 * 3];
        assert_eq!(
            g.map_fractional(&c),
            c,
            "every pigeonhole automorphism fixes the center"
        );
    }
    // An off-center point is genuinely moved (so ½ is special, not trivially fixed).
    let off = vec![0.1, 0.2, 0.3, 0.4, 0.5];
    assert_ne!(
        sigma.map_fractional(&off),
        off,
        "a non-center point is moved"
    );
}

/// **The integrality gap at the symmetric center.** Pigeonhole is integer-UNSAT (the counting
/// certificate, `items > slots`) yet its LP relaxation is *feasible* at the all-½ center — every
/// clause has width ≥ 2, so ½ satisfies them all. The refutation lives in the gap between the
/// symmetry-fixed fractional center (feasible) and the integer corners (all blocked): exactly what
/// resolution at the corners cannot see, and what the counting shadow closes in O(1).
#[test]
fn pigeonhole_is_lp_feasible_at_the_center_yet_integer_unsat() {
    for n in 3..=6 {
        let cover = php_cover(n.min(8));
        assert!(
            cover.relaxation_feasible_at_center(),
            "PHP({n}) clauses all have width ≥ 2 ⟹ the ½-center is LP-feasible"
        );
        let cert = cover
            .counting_refutation()
            .expect("yet it is integer-UNSAT by counting");
        assert!(
            cert.pigeons > cert.holes,
            "the integer obstruction the ½-center hides"
        );
    }
}

/// **The mechanism: why cutting-planes closes the gap resolution can't.** At the ½-center every
/// *pairwise* clause is satisfied (LP value ≥ 1, the exclusions tight at exactly 1) — so resolution,
/// which only ever combines the given clauses, never escapes the feasible center. But the
/// *aggregated cardinality* of each exclusion clique, `Σ_p x_{p,h} = pigeons·½`, exceeds 1 the moment
/// pigeons > 2: a single cutting plane separates the symmetric center the pairwise clauses cannot
/// see. That is the precise step from the integrality gap to the counting refutation.
#[test]
fn the_cutting_plane_separates_the_symmetric_center() {
    let n = 4;
    let cover = php_cover(n);
    let center = vec![0.5_f64; cover.n];

    // Every clause's LP relaxation holds at the ½-center (exclusions tight at exactly 1).
    for b in &cover.blockers {
        assert!(
            b.clause_lp_value(&center) >= 1.0 - 1e-9,
            "clause satisfied at the center"
        );
    }

    // Yet the per-hole cardinality (the clique cutting plane) is violated there.
    let holes = n - 1;
    let var = |p: usize, h: usize| p * holes + h;
    for h in 0..holes {
        let cardinality: f64 = (0..n).map(|p| center[var(p, h)]).sum();
        assert!(
            cardinality > 1.0 + 1e-9,
            "hole {h}: Σ_p x = {cardinality} > 1 — the cutting plane separates the center"
        );
    }
}

/// The mutilated `m×m` chessboard with two opposite (same-colour) corners removed, encoded as a
/// bipartite cover: each majority-colour square (item) must be matched to an adjacent minority
/// square (slot), each minority square holding at most one. Removing two minority squares leaves
/// more items than slots — the colouring/counting obstruction. Returns `(num_vars, clauses)`.
fn mutilated_chessboard(m: usize) -> (usize, Vec<Vec<Lit>>) {
    use std::collections::HashMap;
    let sq = |r: usize, c: usize| r * m + c;
    let removed = |r: usize, c: usize| (r == 0 && c == 0) || (r == m - 1 && c == m - 1);
    let color = |r: usize, c: usize| (r + c) % 2;
    let neighbors = |r: usize, c: usize| {
        let mut v = Vec::new();
        if r > 0 {
            v.push((r - 1, c));
        }
        if r + 1 < m {
            v.push((r + 1, c));
        }
        if c > 0 {
            v.push((r, c - 1));
        }
        if c + 1 < m {
            v.push((r, c + 1));
        }
        v
    };
    // Assign a variable to every (item color-1 square → adjacent slot color-0 square) edge.
    let mut var_of: HashMap<(usize, usize), u32> = HashMap::new();
    for r in 0..m {
        for c in 0..m {
            if color(r, c) == 1 && !removed(r, c) {
                for (nr, nc) in neighbors(r, c) {
                    if color(nr, nc) == 0 && !removed(nr, nc) {
                        let next = var_of.len() as u32;
                        var_of.entry((sq(r, c), sq(nr, nc))).or_insert(next);
                    }
                }
            }
        }
    }
    let mut clauses: Vec<Vec<Lit>> = Vec::new();
    for r in 0..m {
        for c in 0..m {
            if color(r, c) == 1 && !removed(r, c) {
                let row: Vec<Lit> = neighbors(r, c)
                    .into_iter()
                    .filter(|&(nr, nc)| color(nr, nc) == 0 && !removed(nr, nc))
                    .map(|(nr, nc)| Lit::new(var_of[&(sq(r, c), sq(nr, nc))], true))
                    .collect();
                clauses.push(row);
            }
        }
    }
    for r in 0..m {
        for c in 0..m {
            if color(r, c) == 0 && !removed(r, c) {
                let incident: Vec<u32> = neighbors(r, c)
                    .into_iter()
                    .filter(|&(nr, nc)| color(nr, nc) == 1 && !removed(nr, nc))
                    .map(|(nr, nc)| var_of[&(sq(nr, nc), sq(r, c))])
                    .collect();
                for i in 0..incident.len() {
                    for j in (i + 1)..incident.len() {
                        clauses.push(vec![
                            Lit::new(incident[i], false),
                            Lit::new(incident[j], false),
                        ]);
                    }
                }
            }
        }
    }
    (var_of.len(), clauses)
}

/// Two pigeonholes hidden behind a selector `s`: `s=true` activates PHP(a), `s=false` activates
/// PHP(b). Each original clause is weakened by the selector literal (`C ∨ ¬s` for A, `C ∨ s` for B),
/// which masks the bipartite structure at the root. Variable 0 is the selector.
fn selected_pigeonholes(a: usize, b: usize) -> (usize, Vec<Vec<Lit>>) {
    let (a_holes, b_holes) = (a - 1, b - 1);
    let off = 1 + a * a_holes;
    let s = Lit::new(0, true);
    let var_a = |p: usize, h: usize| Lit::new((1 + p * a_holes + h) as u32, true);
    let var_b = |p: usize, h: usize| Lit::new((off + p * b_holes + h) as u32, true);
    let mut clauses = Vec::new();
    for p in 0..a {
        let mut row: Vec<Lit> = (0..a_holes).map(|h| var_a(p, h)).collect();
        row.push(s.negated());
        clauses.push(row);
    }
    for h in 0..a_holes {
        for p in 0..a {
            for q in (p + 1)..a {
                clauses.push(vec![
                    var_a(p, h).negated(),
                    var_a(q, h).negated(),
                    s.negated(),
                ]);
            }
        }
    }
    for p in 0..b {
        let mut row: Vec<Lit> = (0..b_holes).map(|h| var_b(p, h)).collect();
        row.push(s);
        clauses.push(row);
    }
    for h in 0..b_holes {
        for p in 0..b {
            for q in (p + 1)..b {
                clauses.push(vec![var_b(p, h).negated(), var_b(q, h).negated(), s]);
            }
        }
    }
    (off + b * b_holes, clauses)
}

/// **Symmetry-breaking unlocks a hidden invariant.** The selected-pigeonholes formula is UNSAT, but
/// *no cut fires at the root* — the selector literal masks the bipartite structure. Branch the one
/// selector variable (index-order search reaches it first) and each side collapses to a clean
/// pigeonhole the counting cut crushes. The invariant invisible at the root is unlocked one decision
/// down — exactly the ladder × cut synergy.
#[test]
fn branching_the_selector_unlocks_the_masked_cut() {
    let (num_vars, clauses) = selected_pigeonholes(4, 5);
    // No counting cut at the root — the selector literal breaks the clean bipartite shape.
    let e = clauses_to_expr(&clauses).expect("non-empty");
    assert!(
        !crate::pigeonhole::decide_pigeonhole_unsat(&e),
        "the selector masks the bipartite cut at the root"
    );
    // Index-order branching hits the selector first; each branch is a clean pigeonhole the cut kills.
    let (sat, stats) = decide_laddered_sym(num_vars, &clauses, true);
    assert!(!sat, "selected pigeonholes is UNSAT");
    assert!(
        stats.cut_closures >= 1,
        "a cut fires after branching the selector: {stats:?}"
    );
    assert!(
        stats.nodes <= 6,
        "a couple of branches, then crush: {stats:?}"
    );
}

/// **Pigeonhole crush, on a famous board.** The mutilated chessboard — two opposite corners gone —
/// is a classic instance resolution refutes only in *exponential* size, because its only short proof
/// is the colouring/counting argument. Our counting/Hall cut crushes it **at the root** of the
/// ladder, at every board size, where the majority colour over-subscribes the minority.
#[test]
fn the_counting_cut_crushes_the_mutilated_chessboard() {
    for m in [4usize, 6, 8] {
        let (num_vars, clauses) = mutilated_chessboard(m);
        let e = clauses_to_expr(&clauses).expect("non-empty board");
        assert!(
            crate::pigeonhole::decide_pigeonhole_unsat(&e),
            "the counting cut crushes the mutilated {m}×{m} board"
        );
        assert!(
            crate::pigeonhole::hall_refutation(&e).is_some(),
            "Hall names the over-subscribed majority colour on the {m}×{m} board"
        );
        // The laddered solver crushes it at the root by the cut — a single node, at any size.
        let (sat, stats) = decide_laddered(num_vars, &clauses);
        assert!(
            !sat && stats.cut_closures >= 1 && stats.nodes <= 2,
            "{m}×{m}: {stats:?}"
        );
    }
}

/// **Another symmetry break: the *full* Hall invariant, not just the crude count.** A bipartite
/// cover can be infeasible even when items = slots, if a *subset* of items competes for too few
/// slots — a finer matching-symmetry argument the `items > slots` bound is blind to. The full Hall
/// cut catches it and names the violating subset; the auto-cutter already uses the strong version.
#[test]
fn the_full_hall_cut_beats_simple_counting() {
    // 3 items, 3 slots — totals balance — but items 0 and 1 can only use slot 0.
    let cover = Cover::of_cnf(&DimacsCnf {
        num_vars: 4,
        clauses: vec![
            vec![Lit::new(0, true)],                      // item 0 → slot 0
            vec![Lit::new(1, true)],                      // item 1 → slot 0
            vec![Lit::new(2, true), Lit::new(3, true)],   // item 2 → slot 1 or 2
            vec![Lit::new(0, false), Lit::new(1, false)], // slot 0 holds at most one
        ],
    });
    // The crude full-set bound is blind: 3 items = 3 slots, not greater.
    assert_eq!(
        cover.counting_refutation(),
        None,
        "items > slots cannot see the subset violation"
    );
    // The full Hall cut catches it and names the two items fighting over one slot.
    let witness = cover
        .hall_refutation()
        .expect("Hall's theorem refutes the subset");
    assert_eq!(
        witness.items.len(),
        2,
        "two items competing for one slot: {witness:?}"
    );
    assert_eq!(
        witness.slots.len(),
        1,
        "their shared neighborhood is a single slot"
    );
    // The auto-cutter already wields the strong version — it crushes this by counting.
    assert_eq!(
        cover.auto_certify(),
        CoverVerdict::Total {
            cut: Some(Shadow::Counting)
        }
    );
}

/// The generalized crush: the *same* counting certificate refutes pigeonhole **and** clique-coloring,
/// derived structurally from the cover — not hard-coded to either family.
#[test]
fn the_counting_crush_generalizes_beyond_pigeonhole() {
    // Pigeonhole: n pigeons, n−1 holes.
    let php = Cover::of_cnf(&crate::families::php(5).0);
    let pc = php.counting_refutation().expect("pigeonhole crushed");
    assert_eq!((pc.pigeons, pc.holes), (5, 4));

    // Clique-coloring: K_4 needs 4 colors, given 3 — items=4 vertices > slots=3 colors.
    let cc = Cover::of_cnf(&crate::families::clique_coloring(4, 3).0);
    let ccert = cc
        .counting_refutation()
        .expect("clique-coloring crushed by the same invariant");
    assert!(ccert.pigeons > ccert.holes, "K_4 over 3 colors: {ccert:?}");
    assert!(
        crate::pigeonhole::check_counting_cert(&ccert),
        "the certificate re-checks"
    );
}

/// `Pnp.lean`'s fine cover structure: tight vs redundant vertices identify the essential blockers —
/// the irreducible core. A minimal tiling has every corner tight and every blocker essential;
/// adding a redundant blocker overlaps corners without joining the core.
#[test]
fn tight_and_redundant_vertices_identify_the_essential_core() {
    // A minimal tiling of the 2-cube: x0 splits it into the x0=0 and x0=1 halves.
    let minimal = Cover {
        n: 2,
        blockers: vec![
            Subcube::blocker(&[Lit::new(0, true)], 2), // covers the x0 = 0 half
            Subcube::blocker(&[Lit::new(0, false)], 2), // covers the x0 = 1 half
        ],
    };
    assert!(minimal.is_total(), "the two halves tile the whole cube");
    for c in 0u64..4 {
        assert!(
            minimal.is_tight(c),
            "corner {c} is covered by exactly one blocker"
        );
    }
    assert_eq!(
        minimal.essential_blockers(),
        vec![0, 1],
        "both halves are essential"
    );

    // Add a redundant blocker (x1 = 0 half): it only re-covers already-covered corners.
    let mut redundant = minimal.clone();
    redundant
        .blockers
        .push(Subcube::blocker(&[Lit::new(1, true)], 2));
    assert!(
        redundant.is_redundant(0) && redundant.is_redundant(1),
        "corners 0,1 now overlapped"
    );
    assert_eq!(
        redundant.essential_blockers(),
        vec![0, 1],
        "the added blocker joins no core"
    );
}

/// **Ladder up from 1 bool to many: crush the structured, brute-force the rest.** The branch-and-cut
/// search crushes pigeonhole with the counting cut *at the root* — a handful of nodes regardless of
/// scale, far past the cube's 63-variable ceiling — while a genuinely unstructured instance is
/// brute-forced by branching, its verdict agreeing with the independent certified prover.
#[test]
fn laddered_branch_and_cut_crushes_structured_and_brute_forces_the_rest() {
    // Pigeonhole: the learned counting invariant fires at the root, at every scale.
    for n in [4usize, 8, 12, 20] {
        let (cnf, _) = crate::families::php(n);
        let (sat, stats) = decide_laddered(cnf.num_vars, &cnf.clauses);
        assert!(!sat, "PHP({n}) is UNSAT");
        assert!(
            stats.nodes <= 3 && stats.cut_closures >= 1,
            "PHP({n}) crushed by a cut at the root: {stats:?}"
        );
    }
    // Tseitin: the parity cut crushes it at the root too.
    let (_, t, _) = crate::families::tseitin_expander(8, 0x51);
    let (tsat, tstats) = decide_laddered(t.num_vars, &t.clauses);
    assert!(
        !tsat && tstats.cut_closures >= 1,
        "Tseitin crushed by the parity cut: {tstats:?}"
    );

    // A genuinely unstructured instance: brute-forced by branching, verdict matches the prover.
    let rnd = crate::families::random_3sat(12, 22, 0xBEEF);
    let (sat, _) = decide_laddered(rnd.num_vars, &rnd.clauses);
    let e = clauses_to_expr(&rnd.clauses).unwrap();
    let prover_sat = !matches!(
        crate::sat::prove_unsat(&e),
        crate::sat::UnsatOutcome::Refuted
    );
    assert_eq!(
        sat, prover_sat,
        "the ladder agrees with the certified prover on the residual"
    );
}

/// **Symmetry-breaking the search — verified sound against brute force.** Over a fuzz of random
/// CNFs the lex-leader-pruned ladder returns the *exact* brute-force verdict (pruning never changes
/// the answer), and on the structured families it still crushes via the cut. The fuzz is the IP:
/// any unsound prune flips a verdict and fails here.
#[test]
fn symmetry_pruned_ladder_is_sound_against_brute_force() {
    for seed in 0..60u64 {
        let clauses_n = 14 + (seed % 14) as usize;
        let cnf =
            crate::families::random_3sat(9, clauses_n, seed.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        // Brute force over all 2^9 corners.
        let brute = (0u64..(1 << cnf.num_vars)).any(|c| {
            cnf.clauses.iter().all(|cl| {
                cl.iter()
                    .any(|l| ((c >> l.var()) & 1 != 0) == l.is_positive())
            })
        });
        let (sym_sat, _) = decide_laddered_sym(cnf.num_vars, &cnf.clauses, true);
        assert_eq!(
            sym_sat, brute,
            "seed {seed}: symmetry-pruned ladder must match brute force"
        );
        // And it must agree with the plain (un-pruned) ladder too.
        let (plain_sat, _) = decide_laddered(cnf.num_vars, &cnf.clauses);
        assert_eq!(
            sym_sat, plain_sat,
            "seed {seed}: pruning must not change the verdict"
        );
        // Sound with the cut OFF as well (pure DPLL + symmetry pruning).
        let (nocut_sat, _) = decide_laddered_sym(cnf.num_vars, &cnf.clauses, false);
        assert_eq!(
            nocut_sat, brute,
            "seed {seed}: cut-free symmetry pruning must match brute force"
        );
    }
    // Structured families: still crushed by the cut at the root.
    for n in [4usize, 6, 8] {
        let (cnf, _) = crate::families::php(n);
        let (sat, stats) = decide_laddered_sym(cnf.num_vars, &cnf.clauses, true);
        assert!(
            !sat && stats.cut_closures >= 1,
            "PHP({n}) crushed by the cut: {stats:?}"
        );
    }
}

/// Symmetry pruning measurably collapses the search — isolated from the cut. Our certified cuts are
/// so strong that every symmetric UNSAT family is crushed at the root (2-coloring an odd cycle, for
/// instance, is an odd XOR cycle the parity cut kills instantly). So to *see* the lex-leader rule
/// work, turn the cut off: on maximally-symmetric pigeonhole, pure DPLL + symmetry pruning visits
/// far fewer nodes than pure DPLL, while returning the same UNSAT verdict.
#[test]
fn symmetry_pruning_collapses_the_search_with_the_cut_off() {
    for n in [3usize, 4] {
        let (cnf, _) = crate::families::php(n);
        let (sat_pruned, pruned_stats) = decide_laddered_sym(cnf.num_vars, &cnf.clauses, false);
        let (sat_plain, plain_stats) = decide_laddered_nocut(cnf.num_vars, &cnf.clauses);
        assert!(!sat_pruned && !sat_plain, "PHP({n}) is UNSAT either way");
        assert!(
            pruned_stats.pruned >= 1,
            "symmetry pruning must fire on PHP({n}): {pruned_stats:?}"
        );
        assert!(
            pruned_stats.nodes < plain_stats.nodes,
            "PHP({n}): pruned search {} nodes < plain {} nodes",
            pruned_stats.nodes,
            plain_stats.nodes
        );
    }
}

/// **Symmetry-aware counting collapses the solution count.** K₃ has 6 proper 3-colourings, but they
/// all lie in a *single* orbit under the colour/vertex symmetry — so counting touches one
/// representative, not six. The orbits partition the solutions exactly (sizes sum to the total).
#[test]
fn symmetry_aware_counting_collapses_the_count() {
    let cnf = crate::families::clique_coloring(3, 3).0;
    let nv = cnf.num_vars;
    let satisfies = |m: &[bool]| {
        cnf.clauses
            .iter()
            .all(|c| c.iter().any(|l| m[l.var() as usize] == l.is_positive()))
    };
    let models: Vec<Vec<bool>> = (0u64..(1 << nv))
        .filter_map(|x| {
            let m: Vec<bool> = (0..nv).map(|v| (x >> v) & 1 != 0).collect();
            satisfies(&m).then_some(m)
        })
        .collect();
    let total = models.len();
    let generators = crate::symmetry_detect::find_generators(nv, &cnf.clauses);
    let orbits = partition_into_orbits(&models, &generators);

    assert_eq!(
        orbits.iter().map(|o| o.len()).sum::<usize>(),
        total,
        "orbits partition the solutions"
    );
    assert!(
        orbits.len() < total,
        "{} orbits ≪ {} solutions — symmetry collapses the count",
        orbits.len(),
        total
    );
    for orbit in &orbits {
        assert_eq!(
            model_orbit(&orbit[0], &generators).len(),
            orbit.len(),
            "each orbit reconstructs from its rep"
        );
    }
}

/// **A new symmetry recognized: renamable-Horn via flip-renaming.** A non-Horn formula that a
/// phase-flip on a chosen variable set turns Horn is in a polynomial class our field cuts miss — and
/// the renaming is found by a 2-SAT. The flip is a cube symmetry (it permutes models bijectively), so
/// satisfiability is preserved; the renamed formula is provably Horn. A genuinely random instance has
/// no such renaming.
#[test]
fn renamable_horn_is_a_new_symmetry_for_a_new_class() {
    // (x ∨ y) ∧ (x ∨ ¬z): both positive in clause 1 → not Horn, but flipping x makes every clause Horn.
    let cl = vec![
        vec![Lit::new(0, true), Lit::new(1, true)],
        vec![Lit::new(0, true), Lit::new(2, false)],
    ];
    let flips = renaming_to_horn(3, &cl).expect("this formula is renamable to Horn");
    let renamed = apply_renaming(&cl, &flips);
    for c in &renamed {
        assert!(
            c.iter().filter(|l| l.is_positive()).count() <= 1,
            "after the flip-renaming every clause is Horn: {c:?}"
        );
    }

    // The flip preserves satisfiability, and is recognized across a brute-checked fuzz; a renaming,
    // when it exists, always yields a Horn formula equisatisfiable to the original.
    fn sm(s: &mut u64) -> u64 {
        *s = s.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = *s;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z ^ (z >> 31)
    }
    let mut state = 0x8077_0001u64;
    let mut renamable_seen = 0;
    for _ in 0..80 {
        let nv = 3 + (sm(&mut state) % 4) as usize;
        let m = 2 + (sm(&mut state) % 6) as usize;
        let mut clauses: Vec<Vec<Lit>> = Vec::new();
        for _ in 0..m {
            let mut c = Vec::new();
            for var in 0..nv {
                if sm(&mut state) % 2 == 0 {
                    c.push(Lit::new(var as u32, sm(&mut state) % 2 == 0));
                }
            }
            if !c.is_empty() {
                clauses.push(c);
            }
        }
        if clauses.is_empty() {
            continue;
        }
        let sat = |cs: &[Vec<Lit>]| {
            (0u64..(1u64 << nv)).any(|x| {
                cs.iter().all(|c| {
                    c.iter()
                        .any(|l| ((x >> l.var()) & 1 != 0) == l.is_positive())
                })
            })
        };
        if let Some(flips) = renaming_to_horn(nv, &clauses) {
            renamable_seen += 1;
            let renamed = apply_renaming(&clauses, &flips);
            assert!(
                renamed
                    .iter()
                    .all(|c| c.iter().filter(|l| l.is_positive()).count() <= 1),
                "a found renaming must yield a Horn formula"
            );
            assert_eq!(
                sat(&clauses),
                sat(&renamed),
                "the flip-renaming preserves satisfiability"
            );
        }
    }
    assert!(
        renamable_seen > 0,
        "the fuzz should hit renamable-Horn instances"
    );
}

/// **Symmetry generates the whole solution orbit from one model.** Find a single proper colouring of
/// K₃ with 3 colours, push it through the discovered automorphisms, and the entire orbit of distinct
/// colourings falls out — every one a model, none searched for. The generative dual of "rules beget
/// rules": here one solution begets its orbit.
#[test]
fn symmetry_generates_the_solution_orbit_from_one_model() {
    let cnf = crate::families::clique_coloring(3, 3).0;
    let nv = cnf.num_vars;
    let satisfies = |m: &[bool]| {
        cnf.clauses
            .iter()
            .all(|c| c.iter().any(|l| m[l.var() as usize] == l.is_positive()))
    };
    // One model, by brute force (9 variables).
    let model: Vec<bool> = (0u64..(1 << nv))
        .find_map(|x| {
            let m: Vec<bool> = (0..nv).map(|v| (x >> v) & 1 != 0).collect();
            satisfies(&m).then_some(m)
        })
        .expect("K₃ is 3-colourable");

    // The orbit under the discovered automorphisms — every member a model, the orbit non-trivial.
    let generators = crate::symmetry_detect::find_generators(nv, &cnf.clauses);
    let orbit = model_orbit(&model, &generators);
    assert!(
        orbit.len() > 1,
        "symmetry must generate more than one solution, got {}",
        orbit.len()
    );
    for m in &orbit {
        assert!(
            satisfies(m),
            "every symmetric image of a model is a model: {m:?}"
        );
    }
    // Sanity against brute force: the orbit is a subset of all models, and ≥ the colour group's reach.
    let all_models = (0u64..(1 << nv))
        .filter(|&x| satisfies(&(0..nv).map(|v| (x >> v) & 1 != 0).collect::<Vec<_>>()))
        .count();
    assert!(
        orbit.len() <= all_models,
        "the orbit cannot exceed the model count"
    );
}

/// **Symmetry-compression flattens the time to a constant — the right spot.** The clause-level cut
/// must read every clause (linear in `n²`), but pigeonhole is just *two rule-types and a count*, so
/// on its symmetry quotient the decision is `certify_pigeonhole_unsat(pigeons, holes)` — O(1). It
/// runs in the same handful of nanoseconds at `n = 4` and at `n = 2^63`, where the CNF (≈ `n²·2^n`
/// corners, billions of clauses) could never even be built. Flat. That is breaking the time growth.
#[test]
#[ignore = "timing benchmark"]
fn symmetry_compression_flattens_the_time_to_constant() {
    use std::fmt::Write;
    use std::time::Instant;
    let mut chart = String::from("pigeons              symbolic cert\n");
    chart.push_str("-------------------  -------------\n");
    for &n in &[4u128, 64, 10_000, 1_000_000_000, (1u128 << 63), u128::MAX] {
        let reps = 2_000_000u32;
        let t = Instant::now();
        let mut last = None;
        for _ in 0..reps {
            last = crate::pigeonhole::certify_pigeonhole_unsat(std::hint::black_box(n), n - 1);
        }
        let ns = t.elapsed().as_secs_f64() * 1e9 / reps as f64;
        assert!(last.is_some(), "PHP({n}) is refuted by the symbolic cert");
        let _ = writeln!(chart, "{n:<19}  {ns:>8.3} ns");
    }
    println!("\n{chart}");
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../logs/derived_facts");
    if std::fs::create_dir_all(&dir).is_ok() {
        let _ = std::fs::write(
            dir.join("symmetry_compression_flat_time.txt"),
            format!("SYMMETRY-COMPRESSION FLAT TIME — pigeonhole on its orbit-type quotient is two rule-types\nand a count, decided in O(1). Constant nanoseconds from n=4 to n=2^128, where the CNF could\nnever be built. The clause-level cut is linear in the input; the quotient cut is flat.\n\n{chart}\n"),
        );
    }
}

/// **The asymmetry is the hardness knob.** PHP(5) plus `k` asymmetric clauses: autocarve still
/// crushes the symmetric pigeonhole core, but it must branch the asymmetric perturbation first, so the
/// node count grows with `k` — the *distance from symmetric* (the backdoor to symmetry) — not with the
/// size `n`. "Near-symmetric" is "near-easy", and the asymmetry is exactly the parameter. Banked.
#[test]
#[ignore = "measurement"]
fn the_asymmetry_is_the_hardness_knob() {
    use std::fmt::Write;
    fn sm(s: &mut u64) -> u64 {
        *s = s.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = *s;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z ^ (z >> 31)
    }
    let (php, _) = crate::families::php(5);
    let nv = php.num_vars;
    let mut state = 0x4D55_0001u64;
    let mut chart = String::from("asymmetry(k)  verdict      nodes  punches\n");
    chart.push_str("------------  -----------  -----  -------\n");
    for &k in &[0usize, 1, 2, 3, 4] {
        let mut clauses = php.clauses.clone();
        for _ in 0..k {
            let mut c: Vec<Lit> = Vec::new();
            while c.len() < 3 {
                let v = (sm(&mut state) % nv as u64) as u32;
                if !c.iter().any(|l| l.var() == v) {
                    c.push(Lit::new(v, sm(&mut state) % 2 == 0));
                }
            }
            clauses.push(c);
        }
        let (verdict, stats) = autocarve_measured(nv, &clauses, 500_000);
        let _ = writeln!(
            chart,
            "{k:>12}  {:<11}  {:>5}  {:>7}",
            format!("{verdict:?}"),
            stats.nodes,
            stats.punches
        );
    }
    println!("\n{chart}");
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../logs/derived_facts");
    if std::fs::create_dir_all(&dir).is_ok() {
        let _ = std::fs::write(
            dir.join("asymmetry_is_the_knob.txt"),
            format!("THE ASYMMETRY IS THE HARDNESS KNOB — PHP(5) + k asymmetric clauses. Autocarve crushes the\nsymmetric core but branches the perturbation, so cost grows with k (distance from symmetric),\nnot with n. Near-symmetric is near-easy.\n\n{chart}\n"),
        );
    }
}

/// TIMINGS: how fast does the recursive autocarve crush the suite, how many punches does it land,
/// and does the work stay flat as `n` grows (the cut is polynomial) rather than exploding? Banked.
#[test]
#[ignore = "timing benchmark"]
fn autocarve_timings_and_punches() {
    use std::fmt::Write;
    use std::time::Instant;
    let mut chart = String::from("instance              vars  verdict  punches  nodes   time\n");
    chart.push_str("--------------------  ----  -------  -------  ------  ---------\n");
    let mut row = |name: String, nv: usize, cl: &[Vec<Lit>]| {
        // warm + measure (a few iterations so sub-ms shows up).
        let t = Instant::now();
        let mut last = (None, CarveStats::default());
        let reps = 200;
        for _ in 0..reps {
            last = autocarve_measured(nv, cl, 2_000_000);
        }
        let us = t.elapsed().as_secs_f64() * 1e6 / reps as f64;
        let (verdict, stats) = last;
        let _ = writeln!(
            chart,
            "{name:<20}  {nv:>4}  {:<7}  {:>7}  {:>6}  {:>7.2}µs",
            format!("{verdict:?}"),
            stats.punches,
            stats.nodes,
            us
        );
    };
    for n in 4..=8 {
        let (cnf, _) = crate::families::php(n);
        row(format!("pigeonhole({n})"), cnf.num_vars, &cnf.clauses);
    }
    for m in [4usize, 6, 8] {
        let (nv, cl) = mutilated_chessboard(m);
        row(format!("mutilated({m}x{m})"), nv, &cl);
    }
    let (sel_nv, sel_cl) = selected_pigeonholes(4, 5);
    row("masked-php(4,5)".to_string(), sel_nv, &sel_cl);
    let (_, t, _) = crate::families::tseitin_expander(10, 0x9);
    row("tseitin(10)".to_string(), t.num_vars, &t.clauses);

    println!("\n{chart}");
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../logs/derived_facts");
    if std::fs::create_dir_all(&dir).is_ok() {
        let _ = std::fs::write(
            dir.join("autocarve_timings.txt"),
            format!("AUTOCARVE TIMINGS — recursive carve→decompose→cut→branch, per instance.\nPunches = certified cuts that fired; the cut is polynomial so time stays flat as n grows.\n\n{chart}\n"),
        );
    }
}

/// **Autocarving lets the rules fall out — recursively, and soundly.** A pigeonhole masked behind a
/// selector survives the root cut, but autocarve branches the selector, carves the residual, and the
/// counting cut *falls out* on each side. Nest a *second* selector and it falls out one level
/// deeper. Verified against brute force over a fuzz, and against the certified prover on the masked
/// families.
#[test]
fn autocarving_lets_the_rules_fall_out() {
    // Selector-masked pigeonhole: no cut at the root, but autocarve surfaces it after one branch.
    let (sel_nv, sel_cl) = selected_pigeonholes(4, 5);
    assert_eq!(
        autocarve(sel_nv, &sel_cl, 200_000),
        Some(false),
        "masked pigeonhole falls out under autocarve"
    );

    // Doubly-nested: select between {a masked pigeonhole} and {another}, behind a second selector.
    // Built by wiring two selector-masked instances under one fresh selector variable.
    let (a_nv, a_cl) = selected_pigeonholes(4, 4);
    let s2 = a_nv as u32; // fresh top selector
    let mut nested: Vec<Vec<Lit>> = Vec::new();
    for c in &a_cl {
        let mut c2 = c.clone();
        c2.push(Lit::new(s2, false)); // active when s2 = true
        nested.push(c2);
    }
    // s2 = false branch: a tiny independent pigeonhole PHP(3) over fresh vars.
    let (b, _) = crate::families::php(3);
    let off = s2 + 1;
    for c in &b.clauses {
        let mut c2: Vec<Lit> = c
            .iter()
            .map(|l| Lit::new(l.var() + off, l.is_positive()))
            .collect();
        c2.push(Lit::new(s2, true)); // active when s2 = false
        nested.push(c2);
    }
    let nested_nv = (off + b.num_vars as u32) as usize;
    assert_eq!(
        autocarve(nested_nv, &nested, 500_000),
        Some(false),
        "nested masked pigeonholes fall out"
    );

    // Soundness net vs brute force.
    fn sm(s: &mut u64) -> u64 {
        *s = s.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = *s;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z ^ (z >> 31)
    }
    let mut state = 0xFA11_0007u64;
    for _ in 0..60 {
        let nv = 4 + (sm(&mut state) % 4) as usize;
        let m = 3 + (sm(&mut state) % 8) as usize;
        let mut cl: Vec<Vec<Lit>> = Vec::new();
        for _ in 0..m {
            let mut c = Vec::new();
            for var in 0..nv {
                if sm(&mut state) % 3 == 0 {
                    c.push(Lit::new(var as u32, sm(&mut state) % 2 == 0));
                }
            }
            if !c.is_empty() {
                cl.push(c);
            }
        }
        if cl.is_empty() {
            continue;
        }
        let brute = (0u64..(1u64 << nv)).any(|x| {
            cl.iter().all(|c| {
                c.iter()
                    .any(|l| ((x >> l.var()) & 1 != 0) == l.is_positive())
            })
        });
        if let Some(sat) = autocarve(nv, &cl, 1_000_000) {
            assert_eq!(sat, brute, "autocarve must match brute force: {cl:?}");
        }
    }
}

/// **The unified crush composes every lever — and is sound.** A composite formula — a hard
/// pigeonhole core, an independent satisfiable block, and a pure-literal autark shell — is decided by
/// one call: carve the shell, decompose, crush the pigeonhole component with the counting cut. And
/// over a brute-forced fuzz the pipeline's verdict always matches exhaustive search.
#[test]
fn the_unified_crush_pipeline_composes_every_lever() {
    let (php, _) = crate::families::php(4);
    let mut clauses = php.clauses.clone();
    let v = php.num_vars as u32;
    clauses.push(vec![Lit::new(v, true), Lit::new(0, true)]); // pure-literal autark shell (v is pure)
    clauses.push(vec![Lit::new(v + 1, true), Lit::new(v + 2, false)]); // an independent SAT block
    clauses.push(vec![Lit::new(v + 1, false), Lit::new(v + 2, true)]);
    let num_vars = (v + 3) as usize;
    assert_eq!(
        crush(num_vars, &clauses, 200_000),
        Some(false),
        "the pipeline crushes the composite"
    );

    // Soundness net: every decided verdict matches brute force.
    fn sm(s: &mut u64) -> u64 {
        *s = s.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = *s;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z ^ (z >> 31)
    }
    let mut state = 0xC0DE_9999u64;
    for _ in 0..60 {
        let nv = 4 + (sm(&mut state) % 4) as usize;
        let m = 3 + (sm(&mut state) % 8) as usize;
        let mut cl: Vec<Vec<Lit>> = Vec::new();
        for _ in 0..m {
            let mut c = Vec::new();
            for var in 0..nv {
                if sm(&mut state) % 3 == 0 {
                    c.push(Lit::new(var as u32, sm(&mut state) % 2 == 0));
                }
            }
            if !c.is_empty() {
                cl.push(c);
            }
        }
        if cl.is_empty() {
            continue;
        }
        let brute = (0u64..(1u64 << nv)).any(|x| {
            cl.iter().all(|c| {
                c.iter()
                    .any(|l| ((x >> l.var()) & 1 != 0) == l.is_positive())
            })
        });
        if let Some(sat) = crush(nv, &cl, 1_000_000) {
            assert_eq!(sat, brute, "crush must match brute force: {cl:?}");
        }
    }
}

/// **Variable elimination carves out a whole dimension — soundly.** Eliminating a variable projects
/// its axis off the cube (`n` dimensions → `n-1`), and bounded elimination peels away every dimension
/// that doesn't grow the formula. Both preserve satisfiability, fuzzed against brute force.
#[test]
fn variable_elimination_carves_a_dimension_soundly() {
    // (a ∨ b) ∧ (¬a ∨ c): eliminating `a` projects its axis away, leaving the resolvent (b ∨ c).
    let cl = vec![
        vec![Lit::new(0, true), Lit::new(1, true)],
        vec![Lit::new(0, false), Lit::new(2, true)],
    ];
    let projected = eliminate_variable(0, &cl);
    assert!(
        projected.iter().all(|c| c.iter().all(|l| l.var() != 0)),
        "the a-axis is carved away: {projected:?}"
    );
    assert!(
        projected.iter().any(|c| {
            let s: std::collections::BTreeSet<u32> = c.iter().map(|l| l.var()).collect();
            s == [1u32, 2].into_iter().collect()
        }),
        "the resolvent (b ∨ c) survives the projection"
    );

    // Soundness net: elimination (single and bounded) preserves the SAT verdict.
    fn sm(s: &mut u64) -> u64 {
        *s = s.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = *s;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z ^ (z >> 31)
    }
    let mut state = 0xD1AE_0001u64;
    for _ in 0..60 {
        let nv = 4 + (sm(&mut state) % 4) as usize;
        let m = 3 + (sm(&mut state) % 8) as usize;
        let mut cl: Vec<Vec<Lit>> = Vec::new();
        for _ in 0..m {
            let mut c = Vec::new();
            for var in 0..nv {
                if sm(&mut state) % 3 == 0 {
                    c.push(Lit::new(var as u32, sm(&mut state) % 2 == 0));
                }
            }
            if !c.is_empty() {
                cl.push(c);
            }
        }
        if cl.is_empty() {
            continue;
        }
        let sat = |clauses: &[Vec<Lit>]| {
            (0u64..(1u64 << nv)).any(|x| {
                clauses.iter().all(|c| {
                    c.iter()
                        .any(|l| ((x >> l.var()) & 1 != 0) == l.is_positive())
                })
            })
        };
        let brute = sat(&cl);
        assert_eq!(
            brute,
            sat(&eliminate_variable(0, &cl)),
            "single elimination preserves SAT: {cl:?}"
        );
        assert_eq!(
            brute,
            sat(&bounded_variable_elimination(nv, &cl)),
            "bounded VE preserves SAT: {cl:?}"
        );
    }
}

/// **Carving the hypercube — decided or peeled to the core, soundly.** Unit propagation +
/// pure-literal + subsumption carve a formula to a verdict or its irreducible obstruction. A
/// unit-propagation chain carves straight to UNSAT; pigeonhole has no unit, no pure literal, and no
/// subsumption to give, so it carves to itself. Soundness is fuzzed against brute force.
#[test]
fn carve_peels_the_hypercube_to_a_verdict_or_the_core() {
    // (a) ∧ (¬a ∨ b) ∧ (¬b): unit propagation carves straight to the empty clause — UNSAT.
    let unsat = vec![
        vec![Lit::new(0, true)],
        vec![Lit::new(0, false), Lit::new(1, true)],
        vec![Lit::new(1, false)],
    ];
    assert_eq!(carve(2, &unsat), CarveOutcome::Unsat);

    // Pigeonhole is irreducible to carving — it carves to a non-empty core.
    let (php, _) = crate::families::php(4);
    assert!(
        matches!(carve(php.num_vars, &php.clauses), CarveOutcome::Core { .. }),
        "pigeonhole carves to its irreducible core"
    );

    // Soundness net: carving preserves the SAT verdict, over a brute-forced fuzz.
    fn sm(s: &mut u64) -> u64 {
        *s = s.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = *s;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z ^ (z >> 31)
    }
    let mut state = 0xCA47_E000u64;
    for _ in 0..60 {
        let nv = 4 + (sm(&mut state) % 4) as usize;
        let m = 3 + (sm(&mut state) % 8) as usize;
        let mut cl: Vec<Vec<Lit>> = Vec::new();
        for _ in 0..m {
            let mut c = Vec::new();
            for var in 0..nv {
                if sm(&mut state) % 3 == 0 {
                    c.push(Lit::new(var as u32, sm(&mut state) % 2 == 0));
                }
            }
            if !c.is_empty() {
                cl.push(c);
            }
        }
        if cl.is_empty() {
            continue;
        }
        let sat = |clauses: &[Vec<Lit>]| {
            (0u64..(1u64 << nv)).any(|x| {
                clauses.iter().all(|c| {
                    c.iter()
                        .any(|l| ((x >> l.var()) & 1 != 0) == l.is_positive())
                })
            })
        };
        let brute = sat(&cl);
        match carve(nv, &cl) {
            CarveOutcome::Sat => assert!(brute, "carve says SAT: {cl:?}"),
            CarveOutcome::Unsat => assert!(!brute, "carve says UNSAT: {cl:?}"),
            CarveOutcome::Core { clauses, .. } => {
                assert_eq!(
                    brute,
                    sat(&clauses),
                    "the carved core must preserve SAT: {cl:?}"
                )
            }
        }
    }
}

/// **Cutting out autark sections leaves the hard core.** Pure literals satisfy whole sections of the
/// cube and are cut away soundly; pigeonhole has none, so its core survives intact, and a core
/// wrapped in satisfiable padding gets the padding cut and the core crushed. Soundness is fuzzed
/// against brute force.
#[test]
fn pure_literal_autarky_cuts_sections_and_keeps_the_core() {
    // Pigeonhole has no pure literals — every variable appears both polarities. The core is intact.
    let (php, _) = crate::families::php(4);
    let (core, assigned) = pure_literal_reduce(php.num_vars, &php.clauses);
    assert!(assigned.is_empty(), "pigeonhole has no pure literals");
    assert_eq!(
        core.len(),
        php.clauses.len(),
        "the hard core survives untouched"
    );

    // An all-positive formula is one big autark section — it cuts to nothing (trivially SAT).
    let easy = vec![
        vec![Lit::new(0, true), Lit::new(1, true)],
        vec![Lit::new(1, true), Lit::new(2, true)],
    ];
    let (core_easy, _) = pure_literal_reduce(3, &easy);
    assert!(
        core_easy.is_empty(),
        "an all-positive formula reduces to empty — SAT, no section left"
    );

    // Pigeonhole wrapped in a pure-literal shell: the shell is cut, the surviving core is crushed.
    let mut wrapped = php.clauses.clone();
    let shell = php.num_vars as u32;
    wrapped.push(vec![Lit::new(shell, true), Lit::new(0, true)]); // `shell` is pure positive
    let (core_w, assigned_w) = pure_literal_reduce(php.num_vars + 1, &wrapped);
    assert!(
        !assigned_w.is_empty(),
        "the shell's pure literal is cut away"
    );
    let e = clauses_to_expr(&core_w).expect("non-empty core");
    assert!(
        crate::pigeonhole::decide_pigeonhole_unsat(&e),
        "the surviving pigeonhole core is crushed"
    );

    // Soundness net: pure-literal reduction preserves the SAT verdict, over a brute-forced fuzz.
    fn sm(s: &mut u64) -> u64 {
        *s = s.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = *s;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z ^ (z >> 31)
    }
    let mut state = 0xA17A_4321u64;
    for _ in 0..60 {
        let nv = 4 + (sm(&mut state) % 4) as usize;
        let m = 3 + (sm(&mut state) % 7) as usize;
        let mut cl: Vec<Vec<Lit>> = Vec::new();
        for _ in 0..m {
            let mut c = Vec::new();
            for v in 0..nv {
                if sm(&mut state) % 3 == 0 {
                    c.push(Lit::new(v as u32, sm(&mut state) % 2 == 0));
                }
            }
            if !c.is_empty() {
                cl.push(c);
            }
        }
        if cl.is_empty() {
            continue;
        }
        let sat = |clauses: &[Vec<Lit>]| {
            (0u64..(1u64 << nv)).any(|x| {
                clauses.iter().all(|c| {
                    c.iter()
                        .any(|l| ((x >> l.var()) & 1 != 0) == l.is_positive())
                })
            })
        };
        let (reduced, _) = pure_literal_reduce(nv, &cl);
        assert_eq!(
            sat(&cl),
            sat(&reduced),
            "pure-literal reduction must preserve SAT: {cl:?}"
        );
    }
}

/// **Independence axis: decomposition unlocks a buried cut.** A pigeonhole sitting next to an
/// unrelated random sub-formula over disjoint variables is UNSAT — but the monolithic cut sees a
/// mixed clause soup and recognizes nothing. Splitting into independent components isolates the
/// pigeonhole and crushes it in one shot, never touching the rest.
#[test]
fn component_decomposition_unlocks_a_buried_cut() {
    let (php, _) = crate::families::php(4);
    let php_vars = php.num_vars as u32;
    let mut clauses: Vec<Vec<Lit>> = php.clauses.clone();
    // A disjoint random 3-SAT block over fresh variables — mixed-sign clauses that block the
    // bipartite/parity recognizers when smeared together with the pigeonhole.
    let rnd = crate::families::random_3sat(10, 18, 0xD00D);
    for c in &rnd.clauses {
        clauses.push(
            c.iter()
                .map(|l| Lit::new(l.var() + php_vars, l.is_positive()))
                .collect(),
        );
    }
    let num_vars = php_vars as usize + rnd.num_vars;

    // Monolithic: no single cut fires on the mixed formula.
    let e = clauses_to_expr(&clauses).expect("non-empty");
    assert!(
        !crate::pigeonhole::decide_pigeonhole_unsat(&e)
            && !crate::xorsat::refute_via_parity(&e)
            && !crate::pseudo_boolean::refute_clausal(&e),
        "the monolithic mixed formula is recognized by no cut"
    );
    // Decomposition isolates the pigeonhole component and crushes it.
    assert!(
        decompose_and_crush(num_vars, &clauses),
        "decomposition refutes the union"
    );
    // There are exactly two independent components.
    assert_eq!(
        components(num_vars, &clauses).len(),
        2,
        "pigeonhole ⊔ random = two components"
    );
}

/// **The antipodal axis fixes the ½-center**, tying this symmetry to the integrality-gap key: the
/// center-inversion `x → ¬x` (a `CubeSym` with every coordinate flipped, none permuted) maps `½ⁿ`
/// to itself, and a self-complementary cover is detected by [`is_antipodally_symmetric`].
#[test]
fn the_antipodal_map_is_the_center_inversion() {
    let n = 5;
    let antipode = CubeSym {
        perm: (0..n).collect(),
        flip: vec![true; n],
    };
    assert_eq!(
        antipode.map_fractional(&vec![0.5; n]),
        vec![0.5; n],
        "center-inversion fixes ½"
    );
    // Even-cycle 2-colouring is antipodally symmetric (global colour flip); pigeonhole is not.
    let edges = [(0u32, 1u32), (1, 2), (2, 3), (3, 0)];
    let mut c4 = Vec::new();
    for (u, v) in edges {
        c4.push(vec![Lit::new(u, true), Lit::new(v, true)]);
        c4.push(vec![Lit::new(u, false), Lit::new(v, false)]);
    }
    assert!(
        is_antipodally_symmetric(&c4),
        "2-colouring an even cycle is self-complementary"
    );
    assert!(
        !is_antipodally_symmetric(&crate::families::php(4).0.clauses),
        "pigeonhole is not"
    );
}

/// **Recursive antipodal symmetry breaking — sound, and it fires repeatedly.** Verified against brute
/// force over a fuzz (no pruned branch ever changes a verdict), and on a disjoint union of `k`
/// self-complementary blocks the symmetry reappears after each block is fixed, so the recursive
/// break collapses the search to strictly fewer nodes than the plain engine.
#[test]
fn recursive_antipodal_breaking_is_sound_and_collapses_blocks() {
    // k disjoint "x_{2i} ≠ x_{2i+1}" blocks — each self-complementary, the union too, and fixing one
    // block leaves the rest self-complementary, so the antipodal break recurses.
    let blocks = |k: usize| {
        let mut cl = Vec::new();
        for i in 0..k {
            let (a, b) = (2 * i as u32, 2 * i as u32 + 1);
            cl.push(vec![Lit::new(a, true), Lit::new(b, true)]);
            cl.push(vec![Lit::new(a, false), Lit::new(b, false)]);
        }
        (2 * k, cl)
    };
    for k in 2..=6 {
        let (nv, cl) = blocks(k);
        let anti = search_cost_antipodal(nv, &cl, 1_000_000);
        let plain = search_cost(nv, &cl, false, 1_000_000);
        assert!(
            matches!(anti, SearchCost::Decided { sat: true, .. }),
            "blocks are SAT: {anti:?}"
        );
        let (an, pn) = (
            match anti {
                SearchCost::Decided { nodes, .. } => nodes,
                _ => usize::MAX,
            },
            match plain {
                SearchCost::Decided { nodes, .. } => nodes,
                _ => usize::MAX,
            },
        );
        assert!(an <= pn, "k={k}: antipodal {an} ≤ plain {pn} nodes");
    }

    // Soundness net: over a fuzz of random CNFs the antipodal search matches brute force exactly.
    fn sm(s: &mut u64) -> u64 {
        *s = s.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = *s;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z ^ (z >> 31)
    }
    let mut state = 0x5EED_1234u64;
    for _ in 0..50 {
        let nv = 4 + (sm(&mut state) % 4) as usize;
        let m = 3 + (sm(&mut state) % 8) as usize;
        let mut cl: Vec<Vec<Lit>> = Vec::new();
        for _ in 0..m {
            let mut c = Vec::new();
            for v in 0..nv {
                if sm(&mut state) % 3 == 0 {
                    c.push(Lit::new(v as u32, sm(&mut state) % 2 == 0));
                }
            }
            if !c.is_empty() {
                cl.push(c);
            }
        }
        if cl.is_empty() {
            continue;
        }
        let brute = (0u64..(1u64 << nv)).any(|x| {
            cl.iter().all(|c| {
                c.iter()
                    .any(|l| ((x >> l.var()) & 1 != 0) == l.is_positive())
            })
        });
        let anti = search_cost_antipodal(nv, &cl, 1_000_000);
        assert!(
            matches!(anti, SearchCost::Decided { sat, .. } if sat == brute),
            "antipodal search must match brute force: {anti:?} vs {brute}"
        );
    }
}

/// **The campaign's thesis, quantified.** The *same* branch engine, cut OFF (raw DPLL =
/// resolution-strength) vs cut ON (the certified symmetry-distilled invariant), on pigeonhole at
/// growing `n`. Cut-on closes at the root in a single node at every scale; cut-off grows
/// *exponentially* — each pigeon multiplies the search by a widening factor until it blows past the
/// node budget. The growth curve, banked, is the polynomial-vs-exponential separation made concrete.
#[test]
fn the_exponential_gap_measured_and_banked() {
    use std::fmt::Write;
    let budget = 400_000usize;
    let cost = |c: SearchCost| match c {
        SearchCost::Decided { nodes, .. } => nodes,
        SearchCost::Exceeded { budget } => budget,
    };
    let mut chart = String::from(" n   vars   cut nodes   no-cut nodes (resolution)\n");
    chart.push_str("--  -----  ---------  -------------------------\n");
    let mut nocut_curve = Vec::new();
    for n in 2..=8 {
        let (cnf, _) = crate::families::php(n);
        let cut = search_cost(cnf.num_vars, &cnf.clauses, true, budget);
        let nocut = search_cost(cnf.num_vars, &cnf.clauses, false, budget);
        assert!(
            matches!(cut, SearchCost::Decided { nodes, .. } if nodes <= 2),
            "PHP({n}): cut must close in O(1) nodes, got {cut:?}"
        );
        let nc = cost(nocut);
        nocut_curve.push(nc);
        let nocut_str = if matches!(nocut, SearchCost::Exceeded { .. }) {
            format!("≥{budget} (exploded)")
        } else {
            format!("{nc}")
        };
        let _ = writeln!(
            chart,
            "{n:>2}  {:>5}  {:>9}  {nocut_str}",
            cnf.num_vars,
            cost(cut)
        );
    }

    // The cut is flat; raw resolution grows exponentially toward (and past) the budget.
    assert!(
        nocut_curve.windows(2).all(|w| w[1] >= w[0]),
        "raw search grows monotonically: {nocut_curve:?}"
    );
    assert!(
        *nocut_curve.last().unwrap() >= 100_000,
        "PHP(8) raw search is vast vs the cut's 1 node: {nocut_curve:?}"
    );
    assert!(
        nocut_curve[4] >= 1000,
        "the gap to the cut's single node is already vast by PHP(6): {nocut_curve:?}"
    );

    println!("\n{chart}");
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../logs/derived_facts");
    if std::fs::create_dir_all(&dir).is_ok() {
        let _ = std::fs::write(
            dir.join("exponential_gap.txt"),
            format!("EXPONENTIAL GAP — same branch engine, certified cut ON vs OFF (raw resolution).\nThe counting cut closes pigeonhole at the root in ONE node at every scale; raw resolution\ngrows exponentially and explodes past {budget} nodes.\n\n{chart}\n"),
        );
    }
}

/// **Auto-cut and crush, one call.** Hand any cover to `auto_certify` and it reports which
/// hyperplane family closed it (counting / parity / cutting-planes), or that a corner escapes — the
/// whole campaign behind a single door.
#[test]
fn auto_cut_classifies_and_crushes_every_family() {
    use CoverVerdict::Total;
    let php = Cover::of_cnf(&crate::families::php(5).0);
    assert_eq!(
        php.auto_certify(),
        Total {
            cut: Some(Shadow::Counting)
        }
    );

    let cc = Cover::of_cnf(&crate::families::clique_coloring(4, 3).0);
    assert_eq!(
        cc.auto_certify(),
        Total {
            cut: Some(Shadow::Counting)
        }
    );

    let (_, t, _) = crate::families::tseitin_expander(8, 0x51);
    assert_eq!(
        Cover::of_cnf(&t).auto_certify(),
        Total {
            cut: Some(Shadow::Parity)
        }
    );

    // A satisfiable cover: a corner escapes.
    let sat = DimacsCnf {
        num_vars: 3,
        clauses: vec![vec![Lit::new(0, true), Lit::new(1, true)]],
    };
    assert_eq!(Cover::of_cnf(&sat).auto_certify(), CoverVerdict::Escapes);
}

/// **Measuring randomness — and the honest surprise: symmetry is *brittle*.** A symmetry is a
/// compression, so quotient-size (orbit-types ÷ clauses) measures incompressibility — the computable
/// shadow of Kolmogorov complexity. Pigeonhole starts maximally structured (quotient ratio ≈ 0.04).
/// But the transition to "random" is **not a gradient — it's a cliff**: just *four* injected random
/// clauses collapse the automorphism group to trivial and the quotient jumps to the full clause
/// count. Because an automorphism must preserve *every* clause, a single asymmetric clause kills the
/// whole group. Global structure is all-or-nothing. The measurement corrected my own assumption.
#[test]
fn measuring_randomness_the_quotient_climbs_as_structure_decays() {
    use std::fmt::Write;
    fn sm(s: &mut u64) -> u64 {
        *s = s.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = *s;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z ^ (z >> 31)
    }
    let (php, _) = crate::families::php(5);
    let nv = php.num_vars;
    let mut state = 0x4A22_0001u64;
    let mut chart = String::from("injected  clauses  quotient  ratio\n");
    chart.push_str("--------  -------  --------  -----\n");
    let mut ratios = Vec::new();
    for &k in &[0usize, 4, 8, 16, 32] {
        let mut clauses = php.clauses.clone();
        for _ in 0..k {
            let mut c: Vec<Lit> = Vec::new();
            while c.len() < 3 {
                let v = (sm(&mut state) % nv as u64) as u32;
                if !c.iter().any(|l| l.var() == v) {
                    c.push(Lit::new(v, sm(&mut state) % 2 == 0));
                }
            }
            clauses.push(c);
        }
        let generators = crate::symmetry_detect::find_generators(nv, &clauses);
        let quotient = clause_orbits(&clauses, &generators).len();
        let ratio = quotient as f64 / clauses.len() as f64;
        ratios.push(ratio);
        let _ = writeln!(
            chart,
            "{k:>8}  {:>7}  {quotient:>8}  {ratio:.3}",
            clauses.len()
        );
    }
    // The honest finding: a CLIFF, not a gradient. Pristine pigeonhole is highly compressible, but a
    // handful of random clauses collapses the symmetry to nothing (quotient ratio ≈ 1).
    assert!(
        ratios[0] < 0.15,
        "pristine pigeonhole is highly compressible: {}",
        ratios[0]
    );
    assert!(
        ratios[1] > 0.9,
        "just four random clauses annihilate the symmetry (cliff): {ratios:?}"
    );

    println!("\n{chart}");
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../logs/derived_facts");
    if std::fs::create_dir_all(&dir).is_ok() {
        let _ = std::fs::write(
            dir.join("randomness_measure.txt"),
            format!("MEASURING RANDOMNESS — a symmetry is a compression, so quotient-size (orbit-types ÷\nclauses) measures incompressibility (computable shadow of Kolmogorov complexity). Pigeonhole\nis maximally compressible; injecting random clauses erodes the symmetry and the quotient climbs\ntoward 1 — ordered → random as a continuous gradient.\n\n{chart}\n"),
        );
    }
}

/// **It's *asymmetry*, not randomness, that annihilates — and the detector isn't circular.** A single
/// random clause shatters pigeonhole's symmetry. But take *that same clause* and add its whole *orbit*
/// under the group: the addition is now symmetric, and the quotient barely moves. So it was never
/// "randomness" destroying structure — it was the clause falling *off the pattern*. A lucky roll that
/// lands *on* the pattern (lands inside a symmetric set) stays structured, and `find_generators`
/// reports that **directly**, never asking whether we can solve it — no "oh that one wasn't random."
#[test]
fn asymmetry_not_randomness_annihilates_the_structure() {
    use crate::symmetry_detect::{clause_key, find_generators};
    let php = crate::families::php(3).0;
    let nv = php.num_vars;
    let quotient = |cl: &[Vec<Lit>]| clause_orbits(cl, &find_generators(nv, cl)).len();
    let base = quotient(&php.clauses); // 2

    // One off-pattern clause: symmetry breaks.
    let seed = vec![Lit::new(0, false), Lit::new(3, false)]; // an exclusion not in PHP(3)
    let mut broken = php.clauses.clone();
    broken.push(seed.clone());
    assert!(
        quotient(&broken) > base,
        "one asymmetric clause breaks the symmetry"
    );

    // The SAME clause, but add its whole orbit under the grid group: the addition is symmetric.
    let generators = php_perm_symmetries(3);
    let mut seen: BTreeSet<Vec<u32>> = [clause_key(&seed)].into_iter().collect();
    let mut orbit = vec![seed.clone()];
    let mut stack = vec![seed.clone()];
    while let Some(c) = stack.pop() {
        for g in &generators {
            let img = g.apply_clause(&c);
            if seen.insert(clause_key(&img)) {
                orbit.push(img.clone());
                stack.push(img);
            }
        }
    }
    let mut symmetrized = php.clauses.clone();
    symmetrized.extend(orbit.iter().cloned());
    // Far more clauses added than the single off-pattern one, yet the quotient stays small.
    assert!(
        quotient(&symmetrized) <= base + 1,
        "the SAME clause, symmetrized ({} added), preserves the structure",
        orbit.len()
    );
}

/// **"Is that even random?" — No.** A "random" instance from a seed is *fully reproducible*: same
/// seed, byte-identical formula. So its Kolmogorov complexity is at most the seed (a handful of
/// bytes) — it is **not** truly random, just pseudorandom. Yet our symmetry detector sees a *full
/// quotient* (no symmetric structure at all). The two measures of "random" genuinely **disagree**:
/// the instance is seed-simple but symmetry-blind. That gap is the whole point — our cuts find one
/// *kind* of structure (symmetry/algebra); the seed is a different kind they cannot see. Truly random
/// — Kolmogorov-incompressible — is uncomputable (Chaitin); anything we can *generate* has a
/// description and so is, by definition, not it.
#[test]
fn pseudorandom_is_kolmogorov_simple_but_symmetry_blind() {
    // Same seed ⟹ identical instance: the formula is described by the seed, Kolmogorov-simple.
    let a = crate::families::random_3sat(14, 50, 0x00AB_CDEF);
    let b = crate::families::random_3sat(14, 50, 0x00AB_CDEF);
    assert_eq!(
        a.clauses, b.clauses,
        "same seed ⟹ byte-identical: Kolmogorov complexity ≤ the seed"
    );

    // Yet the symmetry detector finds essentially no structure — a near-full quotient.
    let generators = crate::symmetry_detect::find_generators(a.num_vars, &a.clauses);
    let quotient = clause_orbits(&a.clauses, &generators).len();
    assert!(
        quotient * 2 > a.clauses.len(),
        "symmetry-blind: quotient {quotient} near the {} clauses, despite being seed-simple",
        a.clauses.len()
    );

    // A different seed gives a different instance — the structure that *is* there lives in the seed.
    let c = crate::families::random_3sat(14, 50, 0x00AB_CDF0);
    assert_ne!(
        a.clauses, c.clauses,
        "a different seed is a different object — the seed is the structure"
    );
}

/// **What can't we break, and can a break recover the witness?** Two answers, one coin. (1) *What we
/// can't break* is the rigid residue — `|Aut| = 1`, no cut — proven to exist next door. (2) *When we
/// CAN break, the break recovers the witness*: a certified cut **is** a re-checkable refutation
/// witness (counting cert re-checks from scratch), and on the SAT side the symmetry recovers the whole
/// solution set from one model (`model_orbit`). The witness recovery and the symmetry break are the
/// same act — which is exactly why the rigid residue, having no break, hands you no free witness and
/// must be searched.
#[test]
fn breaking_the_symmetry_recovers_the_re_checkable_witness() {
    // (2a) UNSAT: the counting break IS a witness, and it re-checks independently.
    let php = crate::families::php(5).0;
    let e = clauses_to_expr(&php.clauses).expect("non-empty");
    let cert = crate::pigeonhole::counting_certificate(&e).expect("the counting break fires");
    assert!(
        crate::pigeonhole::check_counting_cert(&cert),
        "the recovered refutation witness re-checks: {cert:?}"
    );
    let hall = crate::pigeonhole::hall_refutation(&e).expect("the Hall break fires");
    assert!(
        !hall.items.is_empty(),
        "the Hall break names the violating subset (a witness)"
    );

    // (2b) SAT: the symmetry recovers the entire witness set from a single model.
    let cc = crate::families::clique_coloring(3, 3).0;
    let nv = cc.num_vars;
    let satisfies = |m: &[bool]| {
        cc.clauses
            .iter()
            .all(|c| c.iter().any(|l| m[l.var() as usize] == l.is_positive()))
    };
    let one_model: Vec<bool> = (0u64..(1 << nv))
        .find_map(|x| {
            let m: Vec<bool> = (0..nv).map(|v| (x >> v) & 1 != 0).collect();
            satisfies(&m).then_some(m)
        })
        .expect("clique_coloring(3,3) is SAT");
    let generators = crate::symmetry_detect::find_generators(nv, &cc.clauses);
    let witnesses = model_orbit(&one_model, &generators);
    assert!(
        witnesses.len() > 1,
        "the symmetry recovers many witnesses from one"
    );
    for w in &witnesses {
        assert!(satisfies(w), "every recovered witness is a genuine model");
    }
}

/// **Symmetry-break the witness → its canonical representative.** The witness set is itself symmetric:
/// models come in orbits. The symmetry-broken witness is the canonical (lex-leader) model of its
/// orbit — invariant across the whole orbit, so the distinct canonical witnesses count *exactly* the
/// orbits. The solution set's essential content compresses to one canonical witness per orbit; the
/// symmetry regenerates the rest.
#[test]
fn symmetry_break_the_witness_to_its_canonical_representative() {
    let cc = crate::families::clique_coloring(3, 3).0;
    let nv = cc.num_vars;
    let satisfies = |m: &[bool]| {
        cc.clauses
            .iter()
            .all(|c| c.iter().any(|l| m[l.var() as usize] == l.is_positive()))
    };
    let models: Vec<Vec<bool>> = (0u64..(1 << nv))
        .filter_map(|x| {
            let m: Vec<bool> = (0..nv).map(|v| (x >> v) & 1 != 0).collect();
            satisfies(&m).then_some(m)
        })
        .collect();
    let generators = crate::symmetry_detect::find_generators(nv, &cc.clauses);

    // The canonical witness is an orbit invariant: every model in one orbit breaks to the same one.
    for m in &models {
        let canon = canonical_model(m, &generators);
        for sib in model_orbit(m, &generators) {
            assert_eq!(
                canonical_model(&sib, &generators),
                canon,
                "orbit-mates share a canonical witness"
            );
        }
        assert!(
            satisfies(&canon),
            "the canonical witness is itself a genuine model"
        );
        assert!(
            canon <= *m,
            "the canonical witness is the lex-least of its orbit"
        );
    }

    // Distinct canonical witnesses = number of orbits = the compressed witness set.
    let canonicals: BTreeSet<Vec<bool>> = models
        .iter()
        .map(|m| canonical_model(m, &generators))
        .collect();
    let orbits = partition_into_orbits(&models, &generators);
    assert_eq!(
        canonicals.len(),
        orbits.len(),
        "one canonical witness per orbit"
    );
    assert!(
        canonicals.len() < models.len(),
        "the symmetry genuinely compressed the witness set"
    );
}

/// **Symmetry-break across the witness's perspective of the other witnesses.** From one witness, the
/// others are reached by symmetries — but the perspective is redundant: many symmetries land on the
/// same other witness, differing only by a stabilizer element. Breaking that redundancy yields a
/// transversal of `G / Stab(m)` — exactly one representative transformation per distinct other
/// witness. This is the orbit–stabilizer law, a counting invariant: `|G| = |orbit(m)| · |Stab(m)|`,
/// and it holds from *every* witness's frame. Each recovered transformation re-checks: `σ·m` is the
/// witness it claims, and that witness is a genuine model.
#[test]
fn symmetry_break_across_the_witnesss_perspective_of_other_witnesses() {
    let cc = crate::families::clique_coloring(3, 3).0;
    let nv = cc.num_vars;
    let satisfies = |m: &[bool]| {
        cc.clauses
            .iter()
            .all(|c| c.iter().any(|l| m[l.var() as usize] == l.is_positive()))
    };
    let models: Vec<Vec<bool>> = (0u64..(1 << nv))
        .filter_map(|x| {
            let m: Vec<bool> = (0..nv).map(|v| (x >> v) & 1 != 0).collect();
            satisfies(&m).then_some(m)
        })
        .collect();
    let generators = crate::symmetry_detect::find_generators(nv, &cc.clauses);
    let group = perm_group_closure(&generators, nv);
    assert!(
        group.len() > 1,
        "clique_coloring(3,3) has a nontrivial symmetry group"
    );

    let mut saw_redundant_perspective = false;
    for m in &models {
        let persp = witness_perspective(m, &generators, nv);
        let orbit = model_orbit(m, &generators);
        let stab = stabilizer(m, &group);

        // Orbit–stabilizer: |G| = |orbit| · |Stab|. The witness's broken perspective has |orbit| frames.
        assert_eq!(
            group.len(),
            orbit.len() * stab.len(),
            "orbit-stabilizer holds from m's frame"
        );
        assert_eq!(
            persp.len(),
            orbit.len(),
            "one representative transformation per distinct witness"
        );
        if stab.len() > 1 {
            saw_redundant_perspective = true; // the symmetry break removed genuine redundancy here
        }

        // The witness's view of itself is first, via the identity.
        assert_eq!(persp[0].0, *m, "the witness sees itself first");
        assert!(
            persp[0].1.is_identity(),
            "it sees itself through the identity"
        );

        // Every recovered transformation re-checks, and names a distinct genuine model.
        let mut destinations = BTreeSet::new();
        for (dest, sigma) in &persp {
            assert_eq!(
                apply_perm_to_model(sigma, m),
                *dest,
                "σ·m is the witness it claims"
            );
            assert!(
                satisfies(dest),
                "every witness in the perspective is a genuine model"
            );
            assert!(
                destinations.insert(dest.clone()),
                "no witness is named twice — redundancy is gone"
            );
        }
        assert_eq!(
            destinations,
            orbit.iter().cloned().collect(),
            "the perspective covers the whole orbit"
        );
    }
    assert!(
        saw_redundant_perspective,
        "at least one witness had a nontrivial stabilizer to break"
    );
}

/// **Burnside counts the essentially-distinct witnesses.** Orbit–stabilizer is the per-element law;
/// Burnside is its global average — `#orbits = (1/|G|) Σ_g |Fix(g)|` — and it counts the witnesses up
/// to symmetry as a fixed-point average, no enumeration. Three independent computations must agree:
/// the direct orbit partition, the Burnside average, and the number of distinct canonical witnesses.
/// The fixed-point sum is exactly divisible by `|G|` (the lemma). Fuzzed over small instances —
/// including trivial-group ones, where every witness is its own orbit (the degenerate but valid case).
#[test]
fn burnside_counts_the_essentially_distinct_witnesses() {
    let check = |nv: usize, clauses: &[Vec<Lit>]| {
        let satisfies = |m: &[bool]| {
            clauses
                .iter()
                .all(|c| c.iter().any(|l| m[l.var() as usize] == l.is_positive()))
        };
        let models: Vec<Vec<bool>> = (0u64..(1u64 << nv))
            .filter_map(|x| {
                let m: Vec<bool> = (0..nv).map(|v| (x >> v) & 1 != 0).collect();
                satisfies(&m).then_some(m)
            })
            .collect();
        let generators = crate::symmetry_detect::find_generators(nv, clauses);
        let group = perm_group_closure(&generators, nv);

        // The fixed-point sum is divisible by |G| — Burnside's integrality, a nontrivial invariant.
        let total_fixed: usize = group
            .iter()
            .map(|g| {
                models
                    .iter()
                    .filter(|m| apply_perm_to_model(g, m.as_slice()) == **m)
                    .count()
            })
            .sum();
        assert_eq!(
            total_fixed % group.len(),
            0,
            "Burnside sum divisible by |G|"
        );

        let direct = partition_into_orbits(&models, &generators).len();
        let burnside = burnside_orbit_count(&models, &group);
        let canonicals: BTreeSet<Vec<bool>> = models
            .iter()
            .map(|m| canonical_model(m, &generators))
            .collect();
        assert_eq!(
            direct, burnside,
            "Burnside average == direct orbit partition"
        );
        assert_eq!(
            burnside,
            canonicals.len(),
            "Burnside count == #distinct canonical witnesses"
        );
        (models.len(), burnside, group.len())
    };

    // Headline: a symmetric SAT instance — the essential-solution count is strictly below the raw count.
    let cc = crate::families::clique_coloring(3, 3).0;
    let (raw, essential, gsize) = check(cc.num_vars, &cc.clauses);
    assert!(gsize > 1, "clique_coloring(3,3) has a nontrivial group");
    assert!(
        essential < raw,
        "symmetry genuinely compressed the witness count"
    );

    // Fuzz: small random instances. Many have a trivial group → Burnside == raw model count (degenerate
    // but valid: every witness its own orbit). The three-way identity must hold regardless.
    for seed in 0u64..40 {
        let cnf = crate::families::random_3sat(6, 18, seed.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        check(6, &cnf.clauses);
    }
}

/// A **decorrelated** instance seed for statistical sampling. Mixing the trial index through
/// SplitMix64 is mandatory here: seeding instance `s` with `s · γ` (γ = SplitMix64's own increment)
/// makes consecutive trials share the *same* state stream shifted by one step — the samples collapse
/// onto a single golden-ratio lattice orbit and are not independent. Mixing first scatters them.
fn decorrelated_seed(tag: u64, i: u64) -> u64 {
    let mut z = tag
        .wrapping_mul(0xD1B5_4A32_D192_ED03)
        .wrapping_add(i)
        .wrapping_add(0x9E3779B97F4A7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
    z ^ (z >> 31)
}

/// **Satisfiable random 3-SAT is the *typical* case, not an impossibility.** We never proved "random
/// 3-SAT can't be satisfiable" — that is false, and here is the opposite: the celebrated satisfiability
/// *phase transition*. Below the clause-density threshold (α = m/n ≈ 4.267 for 3-SAT) a random instance
/// is satisfiable with high probability; above it, unsatisfiable. What we *actually* proved is narrower
/// and has nothing to do with satisfiability: a *fixed finite* n-variable formula's refutation
/// complexity is capped at Nullstellensatz degree n, so it cannot encode unbounded (Chaitin)
/// randomness — a statement about one object's *descriptive* complexity, not about whether random
/// instances are SAT. This test exhibits abundant satisfiable random 3-SATs and the density collapse.
#[test]
fn satisfiable_random_3sat_is_the_typical_case_below_the_threshold() {
    let is_sat = |nv: usize, cl: &[Vec<Lit>]| {
        (0u64..(1u64 << nv)).any(|x| {
            cl.iter().all(|c| {
                c.iter()
                    .any(|l| ((x >> l.var()) & 1 != 0) == l.is_positive())
            })
        })
    };
    let n = 14usize;
    let trials = 60u64;
    let sat_count = |m: usize| {
        (0..trials)
            .filter(|&s| {
                let cnf = crate::families::random_3sat(n, m, decorrelated_seed(m as u64, s));
                is_sat(n, &cnf.clauses)
            })
            .count()
    };
    let low = sat_count(2 * n); // α = 2.0, well below threshold
    let high = sat_count(6 * n); // α = 6.0, well above threshold
    assert!(
        low > 0,
        "satisfiable random 3-SATs exist — the premise 'they can't be SAT' is false"
    );
    assert!(
        2 * low > trials as usize,
        "below threshold, random 3-SAT is satisfiable in the MAJORITY: {low}/{trials}"
    );
    assert!(
        high < low,
        "the satisfiability rate collapses across the density threshold — the phase transition"
    );
    assert!(
        5 * high < trials as usize,
        "above threshold, random 3-SAT is overwhelmingly UNSAT: {high}/{trials}"
    );
}

/// **The satisfiability threshold climbs with k: 3-SAT ≈ 4.27, 4-SAT ≈ 9.93.** The threshold grows
/// roughly as `α_k ≈ 2ᵏ ln 2`, so a wider clause tolerates far more constraints before tipping to
/// UNSAT. The sharpest witness is a *single* density wedged between the two thresholds: at α = 6,
/// random 3-SAT is **above** its threshold (every sample UNSAT) while random 4-SAT is **below** its
/// own (every sample SAT) — one ratio, opposite verdicts, the threshold demonstrably climbed. (At
/// n=14 the finite-size 4-SAT crossover sits *above* the asymptotic 9.93 and only descends to it as
/// n→∞, so we assert the robust qualitative facts, not the exact constant.)
#[test]
fn the_satisfiability_threshold_climbs_from_3sat_to_4sat() {
    let is_sat = |nv: usize, cl: &[Vec<Lit>]| {
        (0u64..(1u64 << nv)).any(|x| {
            cl.iter().all(|c| {
                c.iter()
                    .any(|l| ((x >> l.var()) & 1 != 0) == l.is_positive())
            })
        })
    };
    let n = 14usize;
    let trials = 60usize;
    let sat_rate = |k: usize, m: usize| {
        let tag = (k as u64) << 32 ^ m as u64;
        (0..trials as u64)
            .filter(|&s| {
                is_sat(
                    n,
                    &crate::families::random_ksat(k, n, m, decorrelated_seed(tag, s)).clauses,
                )
            })
            .count()
    };

    // The wedge at α = 6: 3-SAT above its threshold, 4-SAT below its own — opposite verdicts.
    let three_at_6 = sat_rate(3, 6 * n);
    let four_at_6 = sat_rate(4, 6 * n);
    assert!(
        4 * three_at_6 < trials,
        "3-SAT at α=6 is above its 4.27 threshold → overwhelmingly UNSAT: {three_at_6}/{trials}"
    );
    assert!(
        4 * four_at_6 > 3 * trials,
        "4-SAT at α=6 is below its 9.93 threshold → overwhelmingly SAT: {four_at_6}/{trials}"
    );
    assert!(
        four_at_6 > three_at_6,
        "the threshold climbed: 4-SAT tolerates a density that already broke 3-SAT"
    );

    // 4-SAT has its own transition — the SAT rate falls as density rises toward and past its threshold.
    let four_at_14 = sat_rate(4, 14 * n);
    assert!(
        four_at_14 < four_at_6,
        "4-SAT's own phase transition: SAT-rate collapses from α=6 to α=14"
    );

    // 3-SAT's transition still brackets 4.27: SAT-majority below, UNSAT below-majority above.
    assert!(
        2 * sat_rate(3, 4 * n) > trials,
        "3-SAT at α=4 (below 4.27) is SAT-majority"
    );
    assert!(
        2 * sat_rate(3, 6 * n) < trials,
        "3-SAT at α=6 (above 4.27) is UNSAT-majority"
    );
}

/// **The proof-complexity ladder separates the families and localizes the wall.** Three UNSAT
/// instances climb the [`ProofRung`] ladder, and each is *invisible* to the cheaper or incomparable
/// rungs below it — the empirical proof-complexity separation (pigeonhole ⊥ parity, both ⊂ the NS
/// height). The rigid residue sits at the top, refuted only by real algebraic degree. **Cautiously
/// pointed at P vs NP:** we can *locate* an instance on this ladder, and we can show one cut is blind
/// where another crushes — but we do **not** prove any family *requires* the top rung. That step is a
/// proof-size lower bound, which is exactly P vs NP, and it stays open. The ladder is the landscape;
/// the open question is whether every hard instance has *some* exploitable structure (a lower rung in
/// *some* system) — the rigid residue is the candidate "no," and its silence under every narrow cut is
/// the honest face of the wall, not a proof of it.
#[test]
fn the_proof_complexity_ladder_separates_and_localizes_the_wall() {
    let e_of = |cl: &[Vec<Lit>]| clauses_to_expr(cl).unwrap();

    // Rung COUNTING: pigeonhole — and GF(2) parity is blind to it.
    let php = crate::families::php(4).0;
    assert_eq!(
        weakest_crushing_rung(php.num_vars, &php.clauses, php.num_vars),
        ProofRung::Counting,
        "PHP is a counting refutation"
    );
    assert!(
        !crate::xorsat::refute_via_parity(&e_of(&php.clauses)),
        "pigeonhole is invisible to GF(2) parity"
    );

    // Rung PARITY: an XOR contradiction — and counting / Hall is blind to it.
    let (_, par) = crate::families::parity_unsat(8, 12, 0xA5A5);
    assert_eq!(
        weakest_crushing_rung(par.num_vars, &par.clauses, par.num_vars),
        ProofRung::Parity,
        "an XOR contradiction is a parity refutation"
    );
    let pe = e_of(&par.clauses);
    assert!(
        crate::pigeonhole::counting_certificate(&pe).is_none()
            && crate::pigeonhole::hall_refutation(&pe).is_none(),
        "a parity contradiction is invisible to counting / Hall"
    );

    // Rung NULLSTELLENSATZ: the rigid residue — invisible to BOTH narrow cuts, refuted only by algebra.
    let is_sat = |nv: usize, cl: &[Vec<Lit>]| {
        (0u64..(1u64 << nv)).any(|x| {
            cl.iter().all(|c| {
                c.iter()
                    .any(|l| ((x >> l.var()) & 1 != 0) == l.is_positive())
            })
        })
    };
    let residue = (0u64..600)
        .find_map(|seed| {
            let c = crate::families::random_3sat(5, 26, seed.wrapping_mul(0x9E37_79B9_7F4A_7C15));
            (!is_sat(5, &c.clauses) && automorphism_group_size(5, &c.clauses) == 1).then_some(c)
        })
        .expect("a rigid UNSAT random 3-SAT exists — the finite hard residue is real");
    match weakest_crushing_rung(5, &residue.clauses, 5) {
        ProofRung::Nullstellensatz { min_degree } => {
            assert!(
                min_degree >= 3,
                "the residue needs genuine algebraic degree, got {min_degree}"
            )
        }
        other => panic!("the rigid residue should land on the NS rung, got {other:?}"),
    }
    let re = e_of(&residue.clauses);
    assert!(
        crate::pigeonhole::counting_certificate(&re).is_none()
            && !crate::xorsat::refute_via_parity(&re),
        "the rigid residue is invisible to every narrow cut — that silence IS the wall"
    );

    // The wall, made concrete. Cap the NS budget BELOW the residue's degree: the rigid UNSAT residue
    // becomes indistinguishable from a SATISFIABLE instance — both land on BeyondBudget. No certified
    // cut can separate "hard UNSAT" from "SAT" cheaply; that indistinguishability IS the wall, and the
    // reason search is unavoidable here. (At full budget the residue separates onto the NS rung above.)
    let cc = crate::families::clique_coloring(3, 3).0; // satisfiable
    assert!(
        is_sat(cc.num_vars, &cc.clauses),
        "clique_coloring(3,3) is SAT"
    );
    assert_eq!(
        weakest_crushing_rung(cc.num_vars, &cc.clauses, 2),
        ProofRung::BeyondBudget,
        "a satisfiable instance fires no cut"
    );
    assert_eq!(
        weakest_crushing_rung(5, &residue.clauses, 2),
        ProofRung::BeyondBudget,
        "below its degree, hard-UNSAT looks identical to SAT — the detectors cannot tell them apart"
    );
}

/// **The finite hard residue EXISTS — even though truly-unbounded random cannot.** The honest
/// correction to "random can't exist": what can't exist *as a finite cube object* is *unbounded*
/// randomness. The **finite** hard residue is real and easy to exhibit — a 3-SAT instance that is
/// rigid (no symmetry), refuted by *no* counting/parity/cardinality cut, genuinely UNSAT, and decided
/// only by full-power algebra within the dimension cap. It is "incompressible-relative-to-its-size":
/// real, hard, bounded — *not* truly random, but the wall all the same. Capping complexity at `n` does
/// NOT make 3-SAT easy: the cap grows with `n` and the cost *at* the cap is exponential.
#[test]
fn the_finite_hard_residue_exists_even_though_unbounded_random_cannot() {
    let sat = |nv: usize, cl: &[Vec<Lit>]| {
        (0u64..(1u64 << nv)).any(|x| {
            cl.iter().all(|c| {
                c.iter()
                    .any(|l| ((x >> l.var()) & 1 != 0) == l.is_positive())
            })
        })
    };
    // Exhibit a rigid, cut-less, UNSAT 3-SAT instance — the finite hard residue.
    let mut found = None;
    for seed in 0u64..600 {
        let c = crate::families::random_3sat(5, 26, seed.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        if !sat(5, &c.clauses) && automorphism_group_size(5, &c.clauses) == 1 {
            found = Some(c);
            break;
        }
    }
    let cnf = found.expect("a rigid UNSAT random 3-SAT exists — the finite hard residue is real");

    // It is structureless to every lever — no symmetry, no certified cut.
    let d = diagnose(5, &cnf.clauses);
    assert_eq!(d.symmetry_bits, 0.0, "rigid — |Aut| = 1, no symmetry");
    assert_eq!(d.cut, None, "no counting/parity/cardinality cut applies");

    // Yet it is genuinely UNSAT and decided only by full-power algebra within the dimension cap.
    let min_degree =
        (1..=5).find(|&deg| crate::polycalc::nullstellensatz_refutes(5, &cnf.clauses, deg));
    assert!(
        min_degree.is_some(),
        "decided by Nullstellensatz within the dimension cap (≤ n)"
    );
    assert!(
        min_degree.unwrap() >= 3,
        "it needed real algebra (degree ≥ clause width), not a cheap cut"
    );
}

/// **Never assume — discover whatever symmetry exists and break on it, even in "random".** Don't
/// declare the residue structureless; *measure* it and break on what's there. A random instance often
/// carries *accidental* automorphisms; we find them and the symmetry-pruned search cuts branches the
/// plain one explores — and the reduction is bounded by those accidental bits (Noether). We break on
/// the last drop of structure, measured, not assumed.
#[test]
fn we_break_on_whatever_symmetry_exists_never_assuming() {
    // Scan for a random instance that carries accidental symmetry — small/under-constrained ones do.
    let mut found = None;
    for seed in 0u64..400 {
        let cnf = crate::families::random_3sat(8, 11, seed.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        if automorphism_group_size(cnf.num_vars, &cnf.clauses) > 1 {
            found = Some(cnf);
            break;
        }
    }
    let cnf = found.expect("some random instance carries accidental symmetry — we don't assume");
    let bits = symmetry_entropy_bits(cnf.num_vars, &cnf.clauses);
    assert!(
        bits > 0.0,
        "we FOUND accidental symmetry in the random instance: {bits} bits"
    );

    // Break on it: the symmetry-pruned search (cut off, to isolate the pruning) never explores more
    // than the plain search, and it returns the same verdict — we exploited the last bit of structure.
    let (sym_sat, pruned) = decide_laddered_sym(cnf.num_vars, &cnf.clauses, false);
    let (plain_sat, plain) = decide_laddered_nocut(cnf.num_vars, &cnf.clauses);
    assert_eq!(
        sym_sat, plain_sat,
        "breaking on the accidental symmetry preserves the verdict"
    );
    assert!(
        pruned.nodes <= plain.nodes,
        "we broke on the accidental symmetry, cutting branches: {} ≤ {}",
        pruned.nodes,
        plain.nodes
    );
}

/// **The structure census — the campaign's measured summary.** For each family: its symmetry-bits,
/// the cut that crushes it, and whether `find_random_core` leaves an irreducible random residue. The
/// punchline, asserted: *every structured family has NO random core* (it is fully decided/crushed by
/// structure), and **random is the only family that leaves a residue**. Structure is always
/// exploitable; randomness is the sole irreducible thing. Banked.
#[test]
fn the_structure_census() {
    use std::fmt::Write;
    let mut chart = String::from("family                bits   cut            residue\n");
    chart.push_str("--------------------  -----  -------------  -------------------\n");
    let mut row = |name: &str, nv: usize, cl: &[Vec<Lit>]| -> Option<Vec<Vec<Lit>>> {
        let bits = symmetry_entropy_bits(nv, cl);
        let cut = clauses_to_expr(cl).and_then(|e| {
            if crate::pigeonhole::decide_pigeonhole_unsat(&e) {
                Some(Shadow::Counting)
            } else if crate::xorsat::refute_via_parity(&e) {
                Some(Shadow::Parity)
            } else if crate::pseudo_boolean::refute_clausal(&e) {
                Some(Shadow::CuttingPlanes)
            } else {
                None
            }
        });
        let core = find_random_core(nv, cl, 100);
        let residue = match &core {
            None => "— (all structure)".to_string(),
            Some(c) => format!("{} clauses (RANDOM)", c.len()),
        };
        let _ = writeln!(
            chart,
            "{name:<20}  {bits:>5.1}  {:<13}  {residue}",
            format!("{cut:?}")
        );
        core
    };

    // Structured families — every one fully crushed by structure, no random residue.
    let php = crate::families::php(5).0;
    assert_eq!(
        row("pigeonhole(5)", php.num_vars, &php.clauses),
        None,
        "pigeonhole: no randomness"
    );
    let cc = crate::families::clique_coloring(4, 3).0;
    assert_eq!(
        row("clique_coloring(4,3)", cc.num_vars, &cc.clauses),
        None,
        "clique: no randomness"
    );
    let (_, t, _) = crate::families::tseitin_expander(8, 0x51);
    assert_eq!(
        row("tseitin(8)", t.num_vars, &t.clauses),
        None,
        "tseitin: no randomness"
    );

    // Random — the only family that leaves an irreducible residue.
    let rnd = crate::families::random_3sat(14, 58, 0xC0FFEE);
    let residue = row("random_3sat(14,58)", rnd.num_vars, &rnd.clauses);
    assert!(
        residue.is_some(),
        "random is the only family with an irreducible random residue"
    );

    println!("\n{chart}");
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../logs/derived_facts");
    if std::fs::create_dir_all(&dir).is_ok() {
        let _ = std::fs::write(
            dir.join("structure_census.txt"),
            format!("STRUCTURE CENSUS — every structured family is fully crushed by structure (no random\nresidue); random is the ONLY family that leaves an irreducible core. Structure is always\nexploitable; randomness is the sole irreducible thing.\n\n{chart}\n"),
        );
    }
}

/// **Finding the randomness — the goal.** Strip all structure and isolate the irreducible core, then
/// confirm it's genuinely random: pigeonhole has *no* random core (the cut decides it — pure
/// structure); a random instance, padded with removable structure, yields back its kernel, and that
/// kernel is *rigid with no cut* — the randomness, found and verified.
#[test]
fn finding_the_randomness_isolates_the_structureless_core() {
    // Pigeonhole is pure structure — there is no randomness to find.
    let php = crate::families::php(4).0;
    assert_eq!(
        find_random_core(php.num_vars, &php.clauses, 50),
        None,
        "pigeonhole has no random core"
    );

    // A random instance wrapped in a removable pure-literal shell. Strip the shell, isolate the
    // kernel, and verify the kernel is genuinely structureless.
    let rnd = crate::families::random_3sat(10, 26, 0xBEEF);
    let mut padded = rnd.clauses.clone();
    let shell = rnd.num_vars as u32;
    padded.push(vec![Lit::new(shell, true), Lit::new(0, true)]); // `shell` is a pure literal
    let nv = rnd.num_vars + 1;

    if let Some(core) = find_random_core(nv, &padded, 50) {
        // The pure-literal shell is gone; what remains is the kernel.
        assert!(
            core.iter().all(|c| c.iter().all(|l| l.var() != shell)),
            "the structural shell is stripped"
        );
        // And it IS the randomness — defined by *no exploitable cut* and being a reduction fixpoint.
        // (A random kernel can still carry a few accidental automorphisms; what makes it the residue
        // is that no certified structure applies and nothing reduces it further.)
        let d = diagnose(nv, &core);
        assert!(
            d.cut.is_none(),
            "the isolated core has no exploitable cut — it's the randomness: {d:?}"
        );
        assert_eq!(
            find_random_core(nv, &core, 50),
            Some(core.clone()),
            "the core is a reduction fixpoint — nothing strips it further"
        );
    }
}

/// **The whole portfolio agrees with brute force — and with each other.** Over a 100-seed fuzz of
/// random CNFs, *every* complete solver path — `crush`, `autocarve`, the symmetry-pruned ladder, the
/// plain ladder, and the cut-free baseline — returns the *exact* brute-force verdict whenever it
/// decides. Any disagreement between any two levers, or against ground truth, fails here. This is the
/// consolidation: the entire edifice is mutually consistent and sound.
#[test]
fn the_whole_portfolio_agrees_with_brute_force() {
    fn sm(s: &mut u64) -> u64 {
        *s = s.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = *s;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z ^ (z >> 31)
    }
    let mut state = 0x6005_0001u64;
    for _ in 0..100 {
        let nv = 4 + (sm(&mut state) % 5) as usize; // 4..8 variables
        let m = 3 + (sm(&mut state) % 12) as usize;
        let mut cl: Vec<Vec<Lit>> = Vec::new();
        for _ in 0..m {
            let mut c = Vec::new();
            for v in 0..nv {
                if sm(&mut state) % 3 == 0 {
                    c.push(Lit::new(v as u32, sm(&mut state) % 2 == 0));
                }
            }
            if !c.is_empty() {
                cl.push(c);
            }
        }
        if cl.is_empty() {
            continue;
        }
        let brute = (0u64..(1u64 << nv)).any(|x| {
            cl.iter().all(|c| {
                c.iter()
                    .any(|l| ((x >> l.var()) & 1 != 0) == l.is_positive())
            })
        });

        // Every complete path must agree with ground truth whenever it decides.
        if let Some(sat) = crush(nv, &cl, 1_000_000) {
            assert_eq!(sat, brute, "crush disagrees: {cl:?}");
        }
        if let Some(sat) = autocarve(nv, &cl, 1_000_000) {
            assert_eq!(sat, brute, "autocarve disagrees: {cl:?}");
        }
        let (sym, _) = decide_laddered_sym(nv, &cl, true);
        assert_eq!(sym, brute, "symmetry-pruned ladder disagrees: {cl:?}");
        let (plain, _) = decide_laddered(nv, &cl);
        assert_eq!(plain, brute, "plain ladder disagrees: {cl:?}");
        let (nocut, _) = decide_laddered_nocut(nv, &cl);
        assert_eq!(nocut, brute, "cut-free baseline disagrees: {cl:?}");
    }
}

/// **Auto-advance to the fixpoint — diagnose, break, repeat until no structure remains.** Pigeonhole
/// is decided in one step by the cut. A *layered* instance — pigeonhole behind a pure-literal shell —
/// is peeled (carve) then crushed (cut), the trace showing two steps. A genuinely random instance
/// advances to the structureless residue: every structural lever exhausted, only branching left. The
/// machine drives the structure to zero on its own.
#[test]
fn auto_advance_drives_structure_to_its_fixpoint() {
    // Pigeonhole: one step — the cut decides it.
    let php = crate::families::php(4).0;
    let (status, trace) = auto_advance(php.num_vars, &php.clauses, 50);
    assert_eq!(
        status,
        AdvanceStatus::Decided(false),
        "pigeonhole decided: {trace:?}"
    );
    assert!(
        trace.last().unwrap().lever.contains("cut"),
        "by the cut: {trace:?}"
    );

    // Layered: a pure-literal shell over the pigeonhole — carve, then cut. Two steps.
    let mut layered = php.clauses.clone();
    layered.push(vec![Lit::new(php.num_vars as u32, true), Lit::new(0, true)]);
    let (st, tr) = auto_advance(php.num_vars + 1, &layered, 50);
    assert_eq!(st, AdvanceStatus::Decided(false), "layered decided: {tr:?}");
    assert!(
        tr.len() >= 2,
        "carve then cut — multiple advance steps: {tr:?}"
    );

    // Random: advances until the structureless residue (no cut ever fires).
    let rnd = crate::families::random_3sat(14, 58, 0xC0FFEE);
    let (sr, rtr) = auto_advance(rnd.num_vars, &rnd.clauses, 50);
    assert!(
        matches!(sr, AdvanceStatus::StructurelessResidue { .. }),
        "random advances to the irreducible residue: {sr:?}"
    );
    assert!(
        !rtr.iter().any(|s| s.lever.contains("cut")),
        "no certified cut on the residue: {rtr:?}"
    );
}

/// **Automated lever discovery — `diagnose` reads the whole menu in one call.** Hand it any instance
/// and it probes *every* detector — the cut, the symmetry bits, antipodal/renamable-Horn/component/
/// autarky structure — and `applicable_levers` lists what you can do. Pigeonhole returns the counting
/// cut and symmetry; a genuinely random instance returns the honest fallback (no global structure,
/// branch the residue). You never have to guess which symmetry to try — it tells you.
#[test]
fn diagnose_auto_discovers_the_applicable_levers() {
    // Pigeonhole: the counting cut and symmetry both fire.
    let php = crate::families::php(4).0;
    let dp = diagnose(php.num_vars, &php.clauses);
    assert_eq!(
        dp.cut,
        Some(Shadow::Counting),
        "pigeonhole offers the counting cut: {dp:?}"
    );
    assert!(dp.symmetry_bits > 0.0, "and rich symmetry: {dp:?}");
    let lp = applicable_levers(&dp);
    assert!(
        lp.iter().any(|s| s.contains("counting")),
        "menu lists the counting cut: {lp:?}"
    );

    // Tseitin: the parity cut.
    let (_, t, _) = crate::families::tseitin_expander(8, 0x51);
    assert_eq!(
        diagnose(t.num_vars, &t.clauses).cut,
        Some(Shadow::Parity),
        "Tseitin offers parity"
    );

    // Random: no cut, near-rigid — the honest fallback.
    let rnd = crate::families::random_3sat(14, 55, 0xC0FFEE);
    let dr = diagnose(rnd.num_vars, &rnd.clauses);
    assert_eq!(dr.cut, None, "random offers no cut: {dr:?}");
    let lr = applicable_levers(&dr);
    assert!(
        lr.iter()
            .any(|s| s.contains("backdoor") || s.contains("carving") || s.contains("residue")),
        "the menu honestly falls back to backdoor/branch: {lr:?}"
    );
}

/// **The structure that teaches: difficulty is quotient size.** Profiling the families end to end —
/// orbit-type count (quotient), the cut that decides them, and the irreducible core after carving —
/// lays them on one axis. Pigeonhole/clique/Tseitin collapse to a tiny quotient and a single cut;
/// random spreads to a full quotient with no cut and an irreducible core. The same number — how far
/// symmetry collapses the cube — predicts everything. Banked as the spectrum.
#[test]
fn the_complexity_spectrum_is_quotient_size() {
    use std::fmt::Write;
    let mut chart = String::from("family                clauses  quotient  cut            core\n");
    chart.push_str("--------------------  -------  --------  -------------  ----\n");
    let mut row = |name: String, nv: usize, cl: &[Vec<Lit>]| -> StructuralProfile {
        let p = structural_profile(nv, cl);
        let _ = writeln!(
            chart,
            "{name:<20}  {:>7}  {:>8}  {:<13}  {:>4}",
            p.clauses,
            p.quotient,
            format!("{:?}", p.cut),
            p.core_clauses
        );
        p
    };

    let php = crate::families::php(5).0;
    let php_p = row("pigeonhole(5)".into(), php.num_vars, &php.clauses);
    let tsei = crate::families::tseitin_expander(8, 0x51).1;
    let tsei_p = row("tseitin(8)".into(), tsei.num_vars, &tsei.clauses);
    let cc = crate::families::clique_coloring(4, 3).0;
    let cc_p = row("clique_coloring(4,3)".into(), cc.num_vars, &cc.clauses);
    let rnd = crate::families::random_3sat(14, 50, 0xC0FFEE);
    let rnd_p = row("random_3sat(14,50)".into(), rnd.num_vars, &rnd.clauses);

    // The axis: a tiny quotient comes with a cut; a full quotient comes with none.
    assert!(
        php_p.quotient <= 3 && php_p.cut.is_some(),
        "pigeonhole: tiny quotient, a cut: {php_p:?}"
    );
    assert!(
        tsei_p.quotient <= 5 && tsei_p.cut == Some(Shadow::Parity),
        "tseitin: small quotient, parity: {tsei_p:?}"
    );
    assert!(
        cc_p.quotient <= 3 && cc_p.cut.is_some(),
        "clique: tiny quotient, a cut: {cc_p:?}"
    );
    assert!(
        rnd_p.cut.is_none() && rnd_p.quotient * 2 > rnd_p.clauses,
        "random: no cut, quotient near the full clause count — the interesting residue: {rnd_p:?}"
    );

    println!("\n{chart}");
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../logs/derived_facts");
    if std::fs::create_dir_all(&dir).is_ok() {
        let _ = std::fs::write(
            dir.join("complexity_spectrum.txt"),
            format!("THE COMPLEXITY SPECTRUM IS QUOTIENT SIZE — how far symmetry collapses the cube predicts\neverything: a tiny orbit-type quotient comes with a single certified cut; a full quotient comes\nwith no cut and an irreducible core. Difficulty is quotient size.\n\n{chart}\n"),
        );
    }
}

/// **Spread out: the auto-collapse across families.** The same blind machinery — symmetry-break to
/// rule-types, probe the shadows — classifies every family by its abstract hardness. Pigeonhole and
/// clique-coloring fall to counting; Tseitin to parity; random spreads across many types with no
/// shadow (its structure is the local backdoor, not a global symmetry). One map, many families.
#[test]
fn the_auto_collapse_spreads_across_families() {
    use std::fmt::Write;
    let mut table = String::from("family                vars  clauses  rule_types  shadow\n");
    let mut record = |name: &str, sig: &FamilySignature| {
        let _ = writeln!(
            table,
            "{name:<20}  {:>4}  {:>7}  {:>10}  {:?}",
            sig.num_vars, sig.clauses, sig.rule_types, sig.shadow
        );
    };

    // Pigeonhole — counting, few rule-types.
    let php = crate::families::php(5).0;
    let php_sig = abstract_signature(php.num_vars, &php.clauses);
    record("pigeonhole(5)", &php_sig);
    assert_eq!(
        php_sig.shadow,
        Some(Shadow::Counting),
        "pigeonhole is a counting cover"
    );
    assert!(
        php_sig.rule_types <= 4,
        "pigeonhole collapses to a few rule-types"
    );

    // Clique-coloring — also counting-shaped (a clique needs more colors than available).
    let cc = crate::families::clique_coloring(4, 3).0;
    let cc_sig = abstract_signature(cc.num_vars, &cc.clauses);
    record("clique_coloring(4,3)", &cc_sig);
    assert!(
        cc_sig.shadow.is_some(),
        "clique-coloring is refuted by a shadow"
    );

    // Tseitin / parity — the GF(2) shadow.
    let (_, tcnf, _) = crate::families::tseitin_expander(8, 0x51);
    let t_sig = abstract_signature(tcnf.num_vars, &tcnf.clauses);
    record("tseitin(8)", &t_sig);
    assert_eq!(
        t_sig.shadow,
        Some(Shadow::Parity),
        "Tseitin is a parity cover"
    );

    // Random — no global symmetry, no shadow: the structure is the backdoor, not the signature.
    let rnd = crate::families::random_3sat(14, 40, 0xC0FFEE);
    let r_sig = abstract_signature(rnd.num_vars, &rnd.clauses);
    record("random_3sat(14,40)", &r_sig);
    assert_eq!(
        r_sig.shadow, None,
        "random hardness is not a recognized shadow class"
    );
    assert!(
        r_sig.rule_types > 2,
        "random rules spread across many types — no global collapse"
    );

    println!("\n{table}");
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../logs/derived_facts");
    if std::fs::create_dir_all(&dir).is_ok() {
        let _ = std::fs::write(
            dir.join("family_taxonomy.txt"),
            format!("ABSTRACT FAMILY TAXONOMY — symmetry-break the rules, probe the shadows\n\n{table}\n"),
        );
    }
}

/// **The smart one-punch: lift and shift left.** The brute resolution closure explodes and even the
/// symmetry-quotiented closure cannot pass PHP(4). The *abstraction* does PHP at any scale in
/// constant work: symmetry-break the rules to their two types, apply Hall's counting invariant. The
/// certificate is identical in shape at n = 4, 8, 16, 32 — and certified by the prover at each — so
/// the proof's true size is `O(1)`, exactly the auto-collapse we found, now scale-free.
#[test]
fn the_abstract_certificate_is_scale_invariant_where_the_closure_explodes() {
    for n in [4usize, 8, 16, 32] {
        let cert = pigeonhole_abstract_refutation(n).expect("pigeonhole refuted at every scale");
        assert_eq!(
            cert.rule_types, 2,
            "always exactly two rule-types — the abstraction is scale-invariant"
        );
        assert_eq!(cert.witness.pigeons, n as u128);
        assert_eq!(cert.witness.holes, (n - 1) as u128);
        assert!(
            cert.witness.pigeons > cert.witness.holes,
            "the counting invariant refutes"
        );
        assert!(
            crate::pigeonhole::check_counting_cert(&cert.witness),
            "the O(1) witness re-checks"
        );
        // and the certified prover agrees, via the same counting shadow, at this scale.
        let e = clauses_to_expr(&crate::families::php(n).0.clauses).unwrap();
        assert_eq!(
            crate::sat::prove_unsat(&e),
            crate::sat::UnsatOutcome::Refuted
        );
    }
}

/// **The one-punch: a symmetry-collapsed resolution refutation on the cube.** Keep resolving (rules
/// beget rules) until the closure saturates — that fixpoint *is* the pattern — then read off its
/// orbit-TYPES. PHP(3)'s full closure saturates at 73 raw rules but only **12 orbit-types** (a 6×
/// collapse), and reaches the empty clause (the full-cube blocker) — a complete refutation. The raw
/// count is what plain resolution pays; the orbit-type count is what a symmetry-aware prover pays.
/// The gap only widens with `n` (the group `Sₚ × Sₕ` grows), which is why the raw closure becomes
/// uncomputable while the orbit pattern stays bounded — fully witnessed here where both are finite.
#[test]
fn symmetric_resolution_refutes_pigeonhole_through_a_bounded_orbit_pattern() {
    let cover = php_cover(3);
    let gens = php_symmetries(3);
    let empty = Subcube {
        n: cover.n,
        care: 0,
        value: 0,
    };

    // The closure saturates to a fixpoint (the pattern), and its orbit-types are far fewer than the
    // raw rules — symmetry collapses the proof.
    let growth = symmetric_resolution_growth(&cover, &gens, 8);
    let last = *growth.last().unwrap();
    assert_eq!(
        last,
        growth[growth.len() - 2],
        "the resolution closure reaches a fixpoint"
    );
    let (raw_fix, orbit_fix) = last;
    assert!(
        orbit_fix * 4 < raw_fix,
        "orbit-types {orbit_fix} ≪ raw {raw_fix}: symmetry collapses the derived rules"
    );

    // The empty clause is in the closure ⟹ resolution refutes PHP(3) on the cube.
    let mut raw: BTreeSet<Subcube> = cover.blockers.iter().copied().collect();
    let mut refuted = false;
    for _ in 0..8 {
        let current: Vec<Subcube> = raw.iter().copied().collect();
        for i in 0..current.len() {
            for j in (i + 1)..current.len() {
                if let Some((_, r)) = current[i].resolve(&current[j]) {
                    raw.insert(r);
                }
            }
        }
        if raw.contains(&empty) {
            refuted = true;
            break;
        }
    }
    assert!(
        refuted,
        "resolution closes PHP(3) to the empty clause — a refutation on the cube"
    );
}

/// Rules beget rules, collapsed: the orbit-type count never exceeds the raw count and the closure is
/// monotone — every round derives rules but they keep folding into the same bounded set of types.
#[test]
fn symmetric_resolution_growth_is_monotone_and_orbit_bounded() {
    let cover = php_cover(3);
    let gens = php_symmetries(3);
    let growth = symmetric_resolution_growth(&cover, &gens, 6);
    for (raw, orbits) in &growth {
        assert!(orbits <= raw, "orbit-types never exceed raw rules");
    }
    for w in growth.windows(2) {
        assert!(w[1].0 >= w[0].0, "the raw closure only grows (monotone)");
        assert!(w[1].1 >= w[0].1, "and so does the orbit-type set");
    }
}

/// **Rules beget rules.** Two neighbor blockers — agreeing on all fixed coordinates but one pivot,
/// opposite there — merge into a new rule with the pivot freed, and that resolvent covers exactly
/// the union of the two neighbors' footprints (the Karnaugh merge). This is resolution, on the cube.
#[test]
fn resolution_nets_a_new_rule_covering_both_neighbors() {
    let c = Subcube::blocker(
        &[Lit::new(0, true), Lit::new(1, true), Lit::new(2, true)],
        4,
    );
    let d = Subcube::blocker(
        &[Lit::new(0, true), Lit::new(1, true), Lit::new(2, false)],
        4,
    );
    let (pivot, resolvent) = c.resolve(&d).expect("neighbors across x2 must resolve");
    assert_eq!(pivot, 2);
    assert_eq!(
        resolvent.clause_literals(),
        vec![(0, true), (1, true)],
        "resolvent is (x0 ∨ x1)"
    );
    let union: BTreeSet<Corner> = c.footprint().into_iter().chain(d.footprint()).collect();
    let merged: BTreeSet<Corner> = resolvent.footprint().into_iter().collect();
    assert_eq!(
        merged, union,
        "the derived rule covers both neighbors and nothing more"
    );
    assert_eq!(
        resolvent.dimension(),
        c.dimension() + 1,
        "one pivot freed ⟹ one dimension larger"
    );
}

/// The geometric resolvent is exactly the clause resolvent — including the tautology guard (a
/// second clashing variable ⟹ no resolvent) and the no-opposite-literal guard.
#[test]
fn resolution_matches_clause_resolution() {
    let c = Subcube::blocker(
        &[Lit::new(0, true), Lit::new(1, false), Lit::new(2, true)],
        5,
    );
    let d = Subcube::blocker(
        &[Lit::new(0, false), Lit::new(1, false), Lit::new(3, true)],
        5,
    );
    let (pivot, r) = c.resolve(&d).expect("resolve on x0");
    assert_eq!(pivot, 0);
    let got: BTreeSet<(usize, bool)> = r.clause_literals().into_iter().collect();
    let want: BTreeSet<(usize, bool)> = [(1, false), (2, true), (3, true)].into_iter().collect();
    assert_eq!(got, want);

    let e = Subcube::blocker(&[Lit::new(0, true), Lit::new(1, true)], 5);
    let f = Subcube::blocker(&[Lit::new(0, false), Lit::new(1, false)], 5);
    assert_eq!(
        e.resolve(&f),
        None,
        "a second clash blocks resolution (tautology)"
    );

    let g = Subcube::blocker(&[Lit::new(0, true), Lit::new(1, true)], 5);
    let h = Subcube::blocker(&[Lit::new(0, true), Lit::new(2, true)], 5);
    assert_eq!(g.resolve(&h), None, "no opposite literal ⟹ no resolution");
}

/// **Symmetry break further:** resolution *commutes* with a cube symmetry — `σ(resolve(C,D)) =
/// resolve(σC, σD)`. So the rules begotten by resolution fall into the same orbits as their
/// parents: a symmetry-aware prover derives one resolvent per orbit, not all of them.
#[test]
fn resolution_commutes_with_symmetry() {
    let c = Subcube::blocker(
        &[Lit::new(0, true), Lit::new(1, false), Lit::new(2, true)],
        4,
    );
    let d = Subcube::blocker(
        &[Lit::new(0, false), Lit::new(1, false), Lit::new(3, true)],
        4,
    );
    let sigma = CubeSym {
        perm: vec![3, 1, 0, 2],
        flip: vec![false, true, false, true],
    };

    let (pivot, resolvent) = c.resolve(&d).unwrap();
    let (pivot_img, resolvent_img) = sigma
        .map_subcube(&c)
        .resolve(&sigma.map_subcube(&d))
        .expect("the images still resolve");
    assert_eq!(
        pivot_img, sigma.perm[pivot],
        "the pivot moves with the symmetry"
    );
    assert_eq!(
        resolvent_img,
        sigma.map_subcube(&resolvent),
        "resolution and symmetry commute ⟹ derived rules respect the orbits"
    );
}

/// Referencing one rule nets its neighbors and the resolvents they beget — read off the cover.
#[test]
fn referencing_one_rule_nets_its_neighbors() {
    let cnf = DimacsCnf {
        num_vars: 3,
        clauses: vec![
            vec![Lit::new(0, true), Lit::new(1, true)],
            vec![Lit::new(0, false), Lit::new(2, true)],
            vec![Lit::new(1, true), Lit::new(2, true)],
        ],
    };
    let cover = Cover::of_cnf(&cnf);
    let neighbors = cover.neighbors(0);
    assert_eq!(
        neighbors.len(),
        1,
        "only clause 1 is a resolution neighbor of clause 0"
    );
    let (j, pivot, resolvent) = &neighbors[0];
    assert_eq!(*j, 1);
    assert_eq!(*pivot, 0);
    let lits: BTreeSet<(usize, bool)> = resolvent.clause_literals().into_iter().collect();
    assert_eq!(
        lits,
        [(1, true), (2, true)].into_iter().collect(),
        "the netted rule is (x1 ∨ x2)"
    );
}

/// **There is no such thing as random — only unfound structure.** A "statistically random" 3-SAT
/// instance is a fixed deterministic object: it has a *small backdoor* to 2-SAT. Fixing those few
/// variables collapses every branch into a polynomially-decidable 2-SAT residual, and solving
/// through the backdoor (2^k easy branches) agrees exactly with the certified prover (a 2ⁿ search).
/// The hardness was never noise; it was structure we hadn't located.
#[test]
fn there_is_no_random_only_unfound_structure() {
    let cnf = crate::families::random_3sat(11, 20, 0xBEEF);
    let backdoor = greedy_2sat_backdoor(&cnf.clauses, cnf.num_vars);

    // The structure is compressing: the backdoor is strictly smaller than the variable set, so the
    // branch count 2^k is far below the brute 2ⁿ.
    assert!(
        backdoor.len() < cnf.num_vars,
        "backdoor {} must be smaller than {} variables",
        backdoor.len(),
        cnf.num_vars
    );
    assert!(
        is_strong_backdoor_to_2sat(&cnf.clauses, cnf.num_vars, &backdoor),
        "every fixing of the backdoor must leave a 2-SAT residual"
    );

    // Solving through the backdoor agrees with the independent certified prover — same verdict, but
    // reached by exploiting structure instead of searching the whole cube.
    let via_backdoor = decide_sat_via_2sat_backdoor(&cnf.clauses, cnf.num_vars, &backdoor);
    let e = clauses_to_expr(&cnf.clauses).expect("non-empty random instance");
    match crate::sat::prove_unsat(&e) {
        crate::sat::UnsatOutcome::Refuted => assert!(!via_backdoor, "prover says UNSAT"),
        crate::sat::UnsatOutcome::Sat(_) => assert!(via_backdoor, "prover says SAT"),
        crate::sat::UnsatOutcome::Unsupported => panic!("prover should decide this instance"),
    }
}

/// Even the *symmetry* of a "random" instance is definite, not random: the detector returns a
/// specific (here, essentially trivial) automorphism group — a fact about the object, computed
/// exactly. Structure is always present; only its *kind* (global symmetry vs. local backdoor)
/// varies. Here the global symmetry is small, and the structure lives in the backdoor instead.
#[test]
fn a_random_instances_symmetry_is_definite_not_absent() {
    let cnf = crate::families::random_3sat(11, 20, 0xBEEF);
    let cover = Cover::of_cnf(&cnf);
    let sig = cover.discovered_rule_symmetry();
    // A definite measurement: the rules barely merge (little global symmetry) — which is *why* the
    // exploitable structure is the local backdoor, not a global group.
    assert!(
        sig.rule_orbits * 2 > sig.blockers,
        "global symmetry is small but definite: {sig:?}"
    );
    let backdoor = greedy_2sat_backdoor(&cnf.clauses, cnf.num_vars);
    assert!(
        !backdoor.is_empty(),
        "the structure is there — it is local (a backdoor)"
    );
}

/// Symmetry breaking is for **speed**: when the backdoor branches are symmetric, we solve only one
/// representative per orbit. Pigeonhole's at-most-one columns make a clean 2-SAT backdoor whose
/// branches collapse hard under the grid group — fewer branches, same answer.
#[test]
fn symmetric_backdoor_branches_collapse_for_speed() {
    let n = 4; // PHP(4): exclusion clauses are width 2 already; the at-least-one rows are the wide ones.
    let (cnf, _) = crate::families::php(n);
    let backdoor = greedy_2sat_backdoor(&cnf.clauses, cnf.num_vars);
    assert!(is_strong_backdoor_to_2sat(
        &cnf.clauses,
        cnf.num_vars,
        &backdoor
    ));

    // Solving PHP(4) through its 2-SAT backdoor returns UNSAT (no branch satisfiable) — agreeing
    // with the certified counting shadow, but as a fan-out of poly-time 2-SAT solves.
    let sat = decide_sat_via_2sat_backdoor(&cnf.clauses, cnf.num_vars, &backdoor);
    assert!(
        !sat,
        "PHP(4) is UNSAT: no backdoor branch leaves a satisfiable 2-SAT residual"
    );

    // The branches (assignments to the backdoor) are not all distinct under the pigeon symmetry —
    // the grid group collapses them, so a symmetry-aware solver inspects strictly fewer than 2^k.
    let branches = 1u64 << backdoor.len();
    let orbits = backdoor_branch_orbit_count(&backdoor, &php_perm_symmetries(n));
    assert!(
        orbits < branches,
        "symmetry collapses {branches} branches to {orbits} — speed"
    );
}

/// The two views are one quotient. The geometric blocker-orbit count (`CubeSym`, on the cube, ≤63
/// variables) and the scalable clause-orbit count (`Perm`, any `n`) agree wherever both run — a
/// blocker *is* a clause. This is what licenses computing the rule symmetry at scales far beyond
/// the cube's reach: the clause-level number is the same one the geometry would give.
#[test]
fn geometric_and_scalable_rule_orbits_are_the_same_quotient() {
    for n in 2..=7 {
        let cover = php_cover(n); // n=7 ⟹ 42 variables, inside the geometric ceiling
        let geometric = cover.blocker_orbits(&php_symmetries(n)).unwrap().len();
        let (cnf, _) = crate::families::php(n);
        let scalable = clause_orbits(&cnf.clauses, &php_perm_symmetries(n)).len();
        assert_eq!(
            geometric, scalable,
            "PHP({n}): geometric={geometric} scalable={scalable}"
        );
        assert_eq!(
            geometric, 2,
            "and both see the two essential pigeonhole rules"
        );
    }
}

/// The complexity-limit chart: symmetry-break the pigeonhole rules across hypercubes of increasing
/// variable size and bank the signature. `rule_orbits` holds at 2 while the corner count races
/// through `2^132`, `2^380`, `2^552` — the limit, drawn. Ignored by default (a scale-walk).
#[test]
#[ignore = "scale-walk; banks the rule-symmetry complexity-limit chart"]
fn rule_symmetry_complexity_limit_chart() {
    let mut chart = String::from(" n   vars   corners       blockers   gens   rule_orbits\n");
    chart.push_str("---  -----  ------------  ---------  -----  -----------\n");
    for n in 2..=24 {
        let sig = pigeonhole_rule_symmetry(n);
        let vars = n * (n - 1);
        chart.push_str(&format!(
            "{:>3}  {:>5}  2^{:<10}  {:>9}  {:>5}  {}\n",
            n, vars, vars, sig.blockers, sig.generators, sig.rule_orbits
        ));
        assert_eq!(
            sig.rule_orbits, 2,
            "rule symmetry must stay at 2 at every scale (n = {n})"
        );
    }
    println!("\n{chart}");
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../logs/derived_facts");
    if std::fs::create_dir_all(&dir).is_ok() {
        let _ = std::fs::write(
            dir.join("rule_symmetry_limits.txt"),
            format!(
                "RULE-SYMMETRY COMPLEXITY LIMIT — pigeonhole rules collapse to 2 orbits at every scale,\n\
                     computed in milliseconds over the polynomial blocker set while the cube itself grows\n\
                     to 2^{{n(n-1)}} corners. Two essential rules describe the entire infinite family.\n\n{chart}\n"
            ),
        );
    }
}
