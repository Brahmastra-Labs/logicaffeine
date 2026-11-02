use super::*;

#[test]
fn finite_groups_are_well_formed() {
    for g in [
        FiniteGroup::trivial(),
        FiniteGroup::cyclic(5),
        FiniteGroup::symmetric(4),
    ] {
        let n = g.order();
        // identity acts as identity, and every element has an inverse
        for a in 0..n {
            assert_eq!(g.mul[g.id][a], a);
            assert_eq!(g.mul[a][g.id], a);
            let _ = g.inverse(a);
        }
    }
    assert_eq!(FiniteGroup::symmetric(4).order(), 24, "|S_4| = 24");
}

#[test]
fn discrete_symmetry_is_a_one_type_pi_two_vanishes() {
    // Our SAT symmetry is DISCRETE: the degenerate crossed module 1 → G. π₂ = ker(∂) = 1, π₁ = G.
    // So the ∞-groupoid is BG = K(G,1), a 1-TYPE — exactly the tower's named top, now via the
    // 2-group machinery. (G = S_4, the pigeon symmetry of PHP(5).)
    let g = FiniteGroup::symmetric(4);
    let cm = CrossedModule {
        g,
        h: FiniteGroup::trivial(),
        partial: vec![0],
    };
    assert!(cm.is_homomorphism(), "∂ : 1 → G is a homomorphism");
    assert_eq!(
        cm.pi_two().len(),
        1,
        "π₂ = ker(∂) = trivial ⇒ a 1-type (discrete symmetry)"
    );
    assert_eq!(cm.pi_one_order(), 24, "π₁ = G = S_4");
}

#[test]
fn a_genuine_2_group_populates_pi_two() {
    // A GENUINE 2-group (the frontier): the crossed module Z/m → 1 with ∂ = 0. Now π₂ = ker(∂) =
    // Z/m ≠ 0 — a K(Z/m, 2), a genuine 2-TYPE. This is what a symmetry-with-internal-symmetry
    // would contribute; our SAT structure does not provide it, but the machinery reads it off.
    for m in [2usize, 3, 5] {
        let cm = CrossedModule {
            g: FiniteGroup::trivial(),
            h: FiniteGroup::cyclic(m),
            partial: vec![0; m],
        };
        assert!(cm.is_homomorphism(), "∂ = 0 : Z/m → 1 is a homomorphism");
        assert_eq!(
            cm.pi_two().len(),
            m,
            "π₂ = ker(∂) = Z/{m} — genuine higher homotopy"
        );
        assert_eq!(cm.pi_one_order(), 1, "π₁ = coker = trivial");
    }
}

#[test]
fn an_isomorphism_crossed_module_is_contractible() {
    // ∂ = id : Z/m → Z/m kills both homotopy groups (π₁ = coker = 1, π₂ = ker = 1) — a contractible
    // 2-group. The machinery spans the range: from contractible, through K(G,1), to K(Z/m,2).
    let m = 6;
    let cm = CrossedModule {
        g: FiniteGroup::cyclic(m),
        h: FiniteGroup::cyclic(m),
        partial: (0..m).collect(), // ∂ = identity
    };
    assert!(cm.is_homomorphism());
    assert_eq!(cm.pi_two().len(), 1, "π₂ = ker(id) = trivial");
    assert_eq!(cm.pi_one_order(), 1, "π₁ = coker(id) = trivial");
}
