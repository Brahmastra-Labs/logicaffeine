use super::*;

/// The adjacent variable-transpositions that generate the full symmetric group `S_nv` on the bits.
fn full_symmetric_gens(nv: usize) -> Vec<Perm> {
    (0..nv.saturating_sub(1))
        .map(|i| {
            let images: Vec<Lit> = (0..nv)
                .map(|v| {
                    let w = if v == i {
                        i + 1
                    } else if v == i + 1 {
                        i
                    } else {
                        v
                    };
                    Lit::pos(w as u32)
                })
                .collect();
            Perm::from_images(images)
        })
        .collect()
}

#[test]
fn symmetry_breaking_is_pi_zero_of_the_action_groupoid() {
    // WHAT SYMMETRY BREAKING MEANS, made checkable. Under the full symmetric group acting on the
    // bits, two assignments are isomorphic iff they have the same Hamming weight — so the `2^n`
    // worlds collapse to exactly `n+1` orbits (weights `0..=n`). That orbit count IS the search
    // space symmetry breaking leaves; the exponential→linear collapse is `2^n → n+1`, computed
    // here as `π₀` of the action groupoid.
    for nv in 3..=8 {
        let g = ActionGroupoid::new(nv, full_symmetric_gens(nv));
        assert_eq!(
            g.num_orbits(),
            nv + 1,
            "S_n on bits ⇒ orbits = Hamming weights = n+1"
        );
        assert!(
            g.num_orbits() < (1usize << nv),
            "exponential collapse: {} orbits vs {} worlds",
            g.num_orbits(),
            1usize << nv
        );
    }
}

#[test]
fn pi_one_of_the_action_groupoid_is_the_stabilizer_and_orbit_stabilizer_holds() {
    // ONE MORE FINITE STEP up the tower: pin the homotopy type of X ⫽ G. π₀ = orbits (already);
    // π₁ at a component = the STABILIZER. The two are bound by the fiber sequence Stab → G → Orbit
    // — the orbit-stabilizer theorem: |G| = |orbit(a)| · |stab(a)| for every assignment a. We check
    // it exhaustively under the full symmetric group on the bits (a small, fully-enumerable group).
    for nv in 3..=5 {
        let g = ActionGroupoid::new(nv, full_symmetric_gens(nv));
        let group = g.group_elements(100_000);
        // |S_n| = n!
        let factorial: usize = (1..=nv).product();
        assert_eq!(
            group.len(),
            factorial,
            "the generated group is S_{nv} of order {nv}!"
        );
        for a in 0..(1u32 << nv) {
            let orbit = g.orbit_size(a);
            let stab = g.stabilizer_size(a, &group);
            assert_eq!(
                orbit * stab,
                group.len(),
                "orbit-stabilizer: |orbit|·|stab| = |G| at a={a}"
            );
        }
    }
}

#[test]
fn the_trivial_group_quotients_nothing_and_a_swap_halves() {
    // Sanity poles of the quotient. No symmetry ⇒ every world is its own orbit (`2^n`). A single
    // variable-swap (a `Z/2` action) pairs `α` with its swap, so the orbit count is the number of
    // swap-symmetric-or-paired worlds — strictly fewer than `2^n` whenever the swap moves anything.
    let nv = 5;
    let none = ActionGroupoid::new(nv, vec![Perm::identity(nv)]);
    assert_eq!(none.num_orbits(), 1 << nv, "trivial action ⇒ no collapse");

    let swap01: Perm = {
        let images: Vec<Lit> = (0..nv)
            .map(|v| {
                Lit::pos(if v == 0 {
                    1
                } else if v == 1 {
                    0
                } else {
                    v
                } as u32)
            })
            .collect();
        Perm::from_images(images)
    };
    let g = ActionGroupoid::new(nv, vec![swap01]);
    assert!(
        g.num_orbits() < (1 << nv),
        "a swap genuinely identifies worlds"
    );
    // Z/2 acting: orbits = fixed points (a₀=a₁) + paired points/2. #fixed = 2^(n-1), #moved = 2^(n-1),
    // so orbits = 2^(n-1) + 2^(n-2) = 3·2^(n-2).
    assert_eq!(
        g.num_orbits(),
        3 * (1 << (nv - 2)),
        "Z/2 swap ⇒ 3·2^(n-2) orbits"
    );
}

/// A deterministic SplitMix64 stream for the sweeps below.
fn splitmix(seed: u64) -> impl FnMut() -> u64 {
    let mut state = seed;
    move || {
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

/// For a true symmetry the model groupoid is the action groupoid's orbits, restricted to the models: the clause
/// `x₀ ∨ x₁ ∨ x₂ ∨ x₃` under the full symmetric group has its models fall into the four weights `1..=4`.
#[test]
fn the_model_groupoid_of_a_true_symmetry_is_its_orbits_on_the_models() {
    let nv = 4;
    let clauses = vec![(0..nv as u32).map(Lit::pos).collect::<Vec<_>>()];
    let gens = full_symmetric_gens(nv);
    let g = ModelGroupoid::new(nv, &clauses, &gens);
    assert_eq!(g.models().len(), 15, "every assignment but all-false");
    assert_eq!(g.num_components(), 4, "one component per Hamming weight 1..=4");
    let action = ActionGroupoid::new(nv, gens);
    let orbit = action.orbits();
    for &m in g.models() {
        for &k in g.models() {
            assert_eq!(
                g.component_of(m) == g.component_of(k),
                orbit[m as usize] == orbit[k as usize],
                "components and orbits agree on {m:04b}, {k:04b}"
            );
        }
    }
    assert_eq!(g.component_of(0), None, "the all-false assignment is no model");
}

/// The breaking theorem, checked by enumeration: the certified conditional breaks of random formulas keep a model in
/// every component of the model groupoid of the transpositions they came from (so satisfiability is kept too), and
/// across the sweep they remove models — the theorem is exercised, not vacuous.
#[test]
fn certified_partial_breaks_keep_a_model_in_every_component() {
    let (mut with_breaks, mut removed) = (0, 0usize);
    for seed in 0..300u64 {
        let mut next = splitmix(seed ^ 0xBEEF);
        let nv = 5 + (seed % 3) as usize;
        let f: Vec<Vec<Lit>> = (0..nv + 3)
            .map(|_| {
                let mut c: Vec<Lit> = Vec::new();
                while c.len() < 2 + (next() % 2) as usize {
                    let l = Lit::new((next() % nv as u64) as u32, next() & 1 == 0);
                    if !c.iter().any(|m| m.var() == l.var()) {
                        c.push(l);
                    }
                }
                c
            })
            .collect();
        let breaks = crate::sym_certify::conditional_symmetry_breaks(nv, &f, 2);
        if breaks.steps.is_empty() {
            continue;
        }
        with_breaks += 1;
        let maps: Vec<Perm> = breaks
            .pairs
            .iter()
            .map(|&(a, b)| {
                Perm::from_images(
                    (0..nv).map(|v| Lit::pos(if v == a { b } else if v == b { a } else { v } as u32)).collect(),
                )
            })
            .collect();
        let added: Vec<Vec<Lit>> = breaks.db[f.len()..].to_vec();
        let g = ModelGroupoid::new(nv, &f, &maps);
        assert!(g.kept_in_every_component(&added), "seed {seed}: a component lost every model");
        let survivors = g.models().iter().filter(|&&m| added.iter().all(|c| c.iter().any(|l| ((m >> l.var()) & 1 == 1) == l.is_positive()))).count();
        assert_eq!(survivors > 0, !g.models().is_empty(), "seed {seed}: satisfiability kept");
        removed += g.models().len() - survivors;
    }
    assert!(with_breaks >= 30, "only {with_breaks} formulas had a certified conditional break");
    assert!(removed > 0, "the breaks never removed a model — the theorem went unexercised");
}

/// Why the guard and the check matter: `F = a ∧ ¬b` with `σ = (a b)` — the one model `a ¬b` has no model image, so it
/// is a component of its own, and the unguarded lex-leader `¬a ∨ b` empties it. The partial-symmetry check refuses
/// that clause.
#[test]
fn an_unguarded_break_of_a_partial_symmetry_can_empty_a_component() {
    let f = vec![vec![Lit::pos(0)], vec![Lit::neg(1)]];
    let swap = Perm::from_images(vec![Lit::pos(1), Lit::pos(0)]);
    let g = ModelGroupoid::new(2, &f, &[swap.clone()]);
    assert_eq!(g.num_components(), 1);
    let unguarded = vec![Lit::neg(0), Lit::pos(1)];
    assert!(!g.kept_in_every_component(&[unguarded.clone()]), "the unguarded break deletes the only model");
    assert!(!crate::pr::is_pr(2, &f, &unguarded, &crate::proof::Witness::PartialSymmetry(swap)));
}

/// `pull_back` is `τ ∘ σ`: with `σ` the swap of variables 0 and 1 and the phase flip of variable 2, the assignment
/// `x₀ = 1, x₁ = 0, x₂ = 0` becomes `x₀ = 0, x₁ = 1, x₂ = 1`.
#[test]
fn pull_back_composes_the_assignment_with_the_mapping() {
    let sigma = Perm::from_images(vec![Lit::pos(1), Lit::pos(0), Lit::neg(2)]);
    assert_eq!(pull_back(3, &sigma, 0b001), 0b110);
    assert_eq!(pull_back(3, &Perm::identity(3), 0b101), 0b101);
}
