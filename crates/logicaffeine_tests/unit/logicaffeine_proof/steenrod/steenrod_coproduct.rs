use super::*;
use std::collections::HashSet;

fn set(pairs: &[(&[usize], &[usize])]) -> HashSet<(Vec<usize>, Vec<usize>)> {
    pairs
        .iter()
        .map(|(l, r)| (l.to_vec(), r.to_vec()))
        .collect()
}

/// The Cartan coproduct `ψ(Sqⁿ) = Σ_{a+b=n} Sqᵃ⊗Sqᵇ`, extended multiplicatively and reduced to the
/// admissible basis — the comultiplication that makes `C⊗C` a Steenrod-module and drives the chain-level
/// diagonal. Verified against hand computation (including the `Sq¹Sq¹ = 0` cancellations inside
/// `ψ(Sq²Sq¹)`), against the COCOMMUTATIVITY of the mod-2 Steenrod algebra (`ψ` is swap-invariant), and
/// against leg-wise degree preservation.
#[test]
fn the_cartan_coproduct_is_diagonal_cocommutative_and_adem_reduced() {
    assert_eq!(coproduct(&[1]), set(&[(&[], &[1]), (&[1], &[])]));
    assert_eq!(
        coproduct(&[2]),
        set(&[(&[2], &[]), (&[1], &[1]), (&[], &[2])])
    );
    assert_eq!(
        coproduct(&[3]),
        set(&[(&[3], &[]), (&[2], &[1]), (&[1], &[2]), (&[], &[3])])
    );

    // ψ(Sq²Sq¹): multiplicativity forces Sq¹Sq¹ = 0 cancellations; exactly these survive.
    assert_eq!(
        coproduct(&[2, 1]),
        set(&[(&[2, 1], &[]), (&[2], &[1]), (&[1], &[2]), (&[], &[2, 1])])
    );

    for m in [
        vec![1],
        vec![2],
        vec![3],
        vec![4],
        vec![2, 1],
        vec![4, 2, 1],
        vec![5],
        vec![6, 3],
    ] {
        let psi = coproduct(&m);
        let swapped: HashSet<_> = psi.iter().map(|(l, r)| (r.clone(), l.clone())).collect();
        assert_eq!(psi, swapped, "ψ is cocommutative on {m:?}");
        let deg = |x: &[usize]| -> usize { x.iter().sum() };
        for (l, r) in &psi {
            assert_eq!(
                deg(l) + deg(r),
                deg(&m),
                "ψ preserves internal degree leg-wise on {m:?}"
            );
        }
    }
}
