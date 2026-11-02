use super::*;

/// The minimal resolution computes the Adams E₂ page, Ext_𝒜(Z/2, Z/2). For the sphere this collapses
/// (E₂ = E∞) through the 13-stem, so the E₂ ranks equal the 2-primary stable homotopy there. The first
/// nontrivial differential is d₂(h₄) = h₀h₃², landing in the 14-stem, so for n ≥ 14 the E₂ rank in a
/// stem strictly exceeds rank E∞: Σ_s dim Ext^{s,14+s} = 5, whereas π₁₄ˢ ⊗ Z₍₂₎ = (Z/2)² has order 4
/// (E∞ contribution 2). The minimal resolution recovers E₂ but not the differentials; those require the
/// algebraic Steenrod operations on Ext (chain-level diagonal), the May spectral sequence, or motivic
/// input, and remain open beyond the ~90-stem.
#[test]
fn e2_strictly_exceeds_einfinity_at_the_fourteen_stem() {
    let e = ext(8, 24);
    let st = |n: usize| -> usize { (0..=8).filter(|&s| n + s <= 24).map(|s| e[s][n + s]).sum() };
    assert_eq!(st(13), 0, "13-stem: E₂ = E∞, both trivial 2-primarily");
    assert_eq!(st(14), 5, "Σ_s dim Ext^{{s,14+s}} = 5 on the E₂ page");
    assert_ne!(
        st(14),
        2,
        "rank E₂ > rank E∞ at the 14-stem: d₂(h₄) = h₀h₃² acts here"
    );
}
