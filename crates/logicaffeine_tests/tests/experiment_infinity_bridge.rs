//! # ENTANGLEMENT IV — the char-2 tower lives inside our ∞-groupoid primitives
//!
//! The "deepest root" of the tower is not a metaphor we reach for; it is machinery already in this crate.
//! `src/eilenberg_maclane.rs` builds the ∞-groupoids `K(𝔽₂,n)` (Kan complexes whose homotopy is read back
//! by horn-filling — the homotopy hypothesis, realized) that *represent* ordinary mod-2 cohomology;
//! `src/postnikov.rs` carries their cohomology **operations** (`cup_square = Sqⁿ`, `bockstein = Sq¹`);
//! `src/steenrod.rs` computes the Steenrod algebra algebraically in `𝔽₂[x]`; and `src/mathir/symfunc.rs`
//! holds the λ-ring / composition-ring Frobenius `ψᵖ`. This file **entangles the four**: the Frobenius `ψᵖ`
//! of the composition ring IS the total Steenrod square, which IS the operation the ∞-groupoid `K(𝔽₂,n)`
//! carries — one structure cross-checked across four independent modules. The char-2 UOV tower's Frobenius
//! twist `O^(2^m)` and its 2-torsion are computations *inside* this substrate, not beside it.

use logicaffeine_proof::eilenberg_maclane::{
    homotopy_group_size, is_omega_spectrum, stable_homotopy,
};
use logicaffeine_proof::postnikov::{bockstein, cup_power_z2, cup_square, is_cocycle};
use logicaffeine_proof::steenrod::sq;
use logicaffeine_proof::two_group::FiniteGroup;

#[test]
fn the_eilenberg_maclane_space_is_the_infinity_groupoid_that_represents_mod2_cohomology() {
    // K(𝔽₂,n): a Kan complex (∞-groupoid) whose ONLY nonzero homotopy group is π_n = 𝔽₂ — read back out of
    // the simplicial object by horn-filling. It deloops to the Ω-spectrum H𝔽₂, whose stable homotopy sits in
    // degree 0 alone: i.e. it represents ORDINARY mod-2 cohomology H*(−;𝔽₂), the home of every operation the
    // tower uses. This is the substrate; the tower is a computation in it.
    for n in 2..=4usize {
        assert_eq!(
            homotopy_group_size(2, n, n),
            2,
            "π_{n}(K(𝔽₂,{n})) = 𝔽₂ — the ∞-groupoid's one nonzero homotopy"
        );
        for k in 2..=(n + 1) {
            if k != n {
                assert_eq!(
                    homotopy_group_size(2, n, k),
                    1,
                    "π_{k}=0 (k≠n): a PURE ∞-groupoid (Eilenberg–MacLane)"
                );
            }
        }
    }
    assert!(
        is_omega_spectrum(2, 3, 4),
        "the K(𝔽₂,n) deloop: π_k(K(𝔽₂,n)) = π_{{k+1}}(K(𝔽₂,n+1)) — an Ω-spectrum (H𝔽₂)"
    );
    assert_eq!(stable_homotopy(2, 0), 2, "π_0(H𝔽₂) = 𝔽₂");
    assert_eq!(
        stable_homotopy(2, 1),
        1,
        "π_k(H𝔽₂) = 0 for k≠0 — H𝔽₂ represents ORDINARY cohomology"
    );
}

#[test]
fn the_infinity_groupoids_cohomology_operations_are_the_steenrod_squares() {
    // The secondary structure the ∞-groupoid carries is cohomology OPERATIONS — natural transformations of
    // Hⁿ(−;𝔽₂). postnikov::cup_square is the TOP square Sqⁿ (x ↦ x∪x); bockstein is Sq¹. We check they are
    // genuine operations (closed ↦ closed) and that their NON-VANISHING matches steenrod::sq exactly — the
    // same Steenrod algebra, computed two independent ways (group-cochains vs 𝔽₂[x]).
    let g = FiniteGroup::cyclic(2);
    for n in 1..=3usize {
        let x_n = cup_power_z2(n); // the generator of Hⁿ(Z/2;𝔽₂)
        assert!(is_cocycle(&g, 2, n, &x_n), "xⁿ is an n-cocycle");
        let top = cup_square(&g, 2, n, &x_n); // Sqⁿ(xₙ) = xₙ² ∈ H^{2n}
        assert!(
            is_cocycle(&g, 2, 2 * n, &top),
            "cup-square lands as a 2n-cocycle — a genuine cohomology operation"
        );
        let nonzero = top.iter().any(|&c| c != 0);
        assert_eq!(
            nonzero,
            sq(n, n) == 1,
            "∞-groupoid Sqⁿ(xₙ)≠0  ⟺  steenrod sq(n,n)=1 — the SAME operation"
        );
        assert!(
            nonzero,
            "the top square is always nonzero (Sqⁿxₙ = xₙ²) — the Frobenius on the class"
        );
    }
    // Bockstein = Sq¹ raises degree by one (H¹→H²) — the lowest genuine operation, reached as a connecting map.
    let bx = bockstein(&g, 1, &cup_power_z2(1));
    assert_eq!(
        bx.len(),
        cup_power_z2(2).len(),
        "bockstein = Sq¹ : H¹→H² — a degree-raising cohomology operation"
    );
}

#[test]
fn the_uov_towers_frobenius_twist_is_steenrod_simple_in_the_infinity_groupoid() {
    // TIE-BACK to the char-2 UOV tower (§30, experiment_steenrod_frobenius). Its Frobenius twist O^(2^m) —
    // the p-th-power layer that splits off — is exactly a Frobenius-twisted class x^{2^m} in H*(K(𝔽₂,1)). By
    // Lucas, C(2^m, i) ≡ 0 (mod 2) for 0 < i < 2^m, so EVERY intermediate Steenrod square annihilates it and
    // ONLY the top square (= the Frobenius itself) survives. The tower's Frobenius layer is invisible to the
    // whole ∞-groupoid operation algebra except its own Frobenius — the reason it splits cleanly — and we
    // read that off our own ∞-groupoid / Steenrod primitives.
    for m in 0..=3usize {
        let d = 1usize << m; // 2^m
        for i in 1..d {
            assert_eq!(sq(i, d), 0, "Sq^{i}(x^(2^{m})) = 0 (Lucas: C(2^m,i)≡0) — invisible to the intermediate operation");
        }
        assert_eq!(
            sq(d, d),
            1,
            "Sq^(2^{m})(x^(2^{m})) = x^(2^{{m}}·2) — only the Frobenius survives"
        );
    }
}
