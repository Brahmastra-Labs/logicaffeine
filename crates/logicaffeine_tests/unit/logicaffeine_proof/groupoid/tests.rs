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
