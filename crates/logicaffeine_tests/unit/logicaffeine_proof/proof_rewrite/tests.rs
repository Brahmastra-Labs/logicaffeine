use super::*;
use crate::pr::check_pr_refutation_fast;
use crate::proof::ProofStep;

#[test]
fn an_antichain_is_the_permutohedron_contractible_with_all_2cells() {
    // n PAIRWISE-INDEPENDENT steps: every ordering is valid, they form the permutohedron, and the
    // commutation 2-cells connect them all into one contractible cube complex (the full n-cube of
    // executions). χ = 1, the cube condition holds (CAT(0)), and there are n! linear extensions.
    for n in 1..=6 {
        let p = ProofPoset::new(n, &[]);
        assert_eq!(
            p.euler_characteristic(),
            1,
            "the n-cube of independent steps is contractible"
        );
        assert!(
            p.satisfies_cube_condition(),
            "all independent steps jointly form cubes (CAT(0))"
        );
        assert!(
            p.extensions_connected_by_commutation(),
            "all orderings connected by 2-cells"
        );
        let factorial: usize = (1..=n).product();
        assert_eq!(p.linear_extensions().len(), factorial, "n! orderings");
    }
}

#[test]
fn a_chain_is_an_interval_one_ordering_still_contractible() {
    // Totally dependent steps (each needs the last): a single ordering, the cube complex is an
    // interval — still contractible (χ = 1), trivially connected, no nontrivial 2-cells.
    let edges: Vec<(usize, usize)> = (0..6).map(|i| (i, i + 1)).collect();
    let p = ProofPoset::new(7, &edges);
    assert_eq!(
        p.linear_extensions().len(),
        1,
        "a chain has exactly one ordering"
    );
    assert_eq!(p.euler_characteristic(), 1, "an interval is contractible");
    assert!(p.satisfies_cube_condition());
    assert!(p.extensions_connected_by_commutation());
}

#[test]
fn the_fundamental_2cell_is_a_commutation_square() {
    // The atom of the theory: two independent steps a, b. Both orders [a,b] and [b,a] are valid,
    // and the single commutation between them is the 2-cell — the filled square (a 2-cube). χ = 1.
    let p = ProofPoset::new(2, &[]);
    let exts = p.linear_extensions();
    assert_eq!(exts.len(), 2, "[a,b] and [b,a]");
    assert!(p.independent(0, 1), "the two steps commute");
    assert_eq!(
        p.euler_characteristic(),
        1,
        "the commutation square is a filled (contractible) 2-cell"
    );
}

#[test]
fn a_mixed_poset_is_still_contractible() {
    // A diamond 0 < {1,2} < 3: steps 1 and 2 are independent (a genuine 2-cell) but both gated by
    // 0 and gating 3. Two orderings, connected by the 1–2 commutation; the complex stays contractible.
    let p = ProofPoset::new(4, &[(0, 1), (0, 2), (1, 3), (2, 3)]);
    assert!(p.independent(1, 2), "the diamond's middle is a 2-cell");
    assert!(!p.independent(0, 3), "0 must precede 3 (transitively)");
    assert_eq!(p.linear_extensions().len(), 2, "[0,1,2,3] and [0,2,1,3]");
    assert_eq!(
        p.euler_characteristic(),
        1,
        "the diamond execution is contractible"
    );
    assert!(p.satisfies_cube_condition());
    assert!(p.extensions_connected_by_commutation());
}

#[test]
fn a_real_refutations_independent_steps_all_commute_certified_by_the_checker() {
    // GROUNDING ON A REAL REFUTATION, with the PR checker as the oracle. F is two var-disjoint
    // contradictions; the four RUP lemmas (1),(¬1),(3),(¬3) are each derivable from the ORIGINALS
    // alone, hence pairwise independent. Therefore ALL 4! = 24 orderings (followed by the empty
    // clause) are valid refutations — and we make the trusted checker confirm every one. The
    // abstract antichain poset predicts exactly this: 24 linear extensions, contractible, all
    // connected by the commutation 2-cells.
    let (nv, f) = disjoint_double_contradiction();
    let lemmas = [
        vec![Lit::pos(1)],
        vec![Lit::neg(1)],
        vec![Lit::pos(3)],
        vec![Lit::neg(3)],
    ];

    let mut certified = 0;
    for perm in permutations(4) {
        let mut steps: Vec<ProofStep> = perm
            .iter()
            .map(|&i| ProofStep::Rup(lemmas[i].clone()))
            .collect();
        steps.push(ProofStep::Rup(vec![])); // the empty clause closes it
        assert!(
            check_pr_refutation_fast(nv, &f, &steps),
            "the independent steps reordered as {perm:?} must still refute F"
        );
        certified += 1;
    }
    assert_eq!(
        certified, 24,
        "all 4! reorderings of independent proof steps are checker-certified"
    );

    // the abstract theory predicts the same shape, exactly
    let poset = ProofPoset::new(4, &[]);
    assert_eq!(poset.linear_extensions().len(), 24);
    assert_eq!(
        poset.euler_characteristic(),
        1,
        "the proof-reordering space is contractible"
    );
    assert!(
        poset.extensions_connected_by_commutation(),
        "the 2-cells connect all 24 orderings"
    );
}

#[test]
fn the_proof_complex_runs_through_the_same_engine_closing_the_circle() {
    // CLOSING THE CIRCLE. The proof-rewrite commutation complex is handed to the SAME general
    // cubical-homology engine that produced π₁, π₂, π₃ from concurrency. Its full Betti vector
    // confirms contractibility — the coherence theorem — now via honest GF(2) homology in every
    // dimension, and its Euler characteristic matches proof_rewrite's own count exactly. The ladder
    // that STARTED at π₀ = symmetry breaking is one engine, end to end.
    use crate::cubical::CubicalComplex;
    for n in 1..=5 {
        let poset = ProofPoset::new(n, &[]);
        let complex: CubicalComplex = poset.execution_complex();
        let beta = complex.betti();
        assert_eq!(beta[0], 1, "connected");
        assert!(
            beta[1..].iter().all(|&b| b == 0),
            "n independent steps → contractible n-cube (coherence)"
        );
        assert_eq!(complex.euler(), 1, "χ = 1");
        assert_eq!(
            complex.euler(),
            poset.euler_characteristic(),
            "the two engines agree on χ exactly"
        );
    }
    // a chain → an interval; a diamond → a filled square inside a contractible whole: both contractible.
    assert_eq!(
        ProofPoset::new(5, &[(0, 1), (1, 2), (2, 3), (3, 4)])
            .execution_complex()
            .betti(),
        vec![1, 0]
    );
    let diamond = ProofPoset::new(4, &[(0, 1), (0, 2), (1, 3), (2, 3)]).execution_complex();
    assert_eq!(diamond.betti()[0], 1);
    assert!(
        diamond.betti()[1..].iter().all(|&b| b == 0),
        "the diamond's middle 2-cell sits in a contractible whole"
    );
}

#[test]
fn recursive_symmetry_breaking_of_the_proof_complex_recovers_BG() {
    // RECURSIVE SYMMETRY BREAKING — and it CLOSES. The symmetry group of F acts on the contractible
    // proof-rewrite complex by cellular automorphisms (relabel steps, preserving commutation). The
    // block-swap σ of the double contradiction permutes the four lemmas (1),(¬1),(3),(¬3) as the
    // involution (0 2)(1 3) — swap the two contradiction blocks. It is a FREE involution of the
    // contractible 4-fold-independent complex. A free cellular action of ⟨σ⟩ = Z/2 on a contractible
    // complex has homotopy quotient B⟨σ⟩ = K(Z/2, 1). So symmetry-breaking the SPACE OF PROOFS
    // recovers a classifying space BG — the very shape the assignment tower produced. Self-similar:
    // the tower reappears one level up, and the ∞-direction is "BG all the way down".
    let p = ProofPoset::new(4, &[]); // the four independent lemmas
    let sigma = [2usize, 3, 0, 1]; // (0 2)(1 3): swap the two contradiction blocks

    assert!(
        p.is_complex_automorphism(&sigma),
        "σ preserves commutation — a cellular automorphism"
    );
    assert!(
        p.acts_freely_on_extensions(&sigma),
        "σ acts freely — no proof-ordering is σ-fixed"
    );

    // σ² = id ⇒ ⟨σ⟩ = Z/2; free Z/2 on a contractible complex ⇒ quotient ≃ K(Z/2,1) = BG.
    let sigma_sq: Vec<usize> = (0..4).map(|i| sigma[sigma[i]]).collect();
    assert_eq!(
        sigma_sq,
        vec![0, 1, 2, 3],
        "σ² = id ⇒ ⟨σ⟩ = Z/2, so the quotient is K(Z/2,1) = BG"
    );
    // a free Z/2 action halves the cells: 24 orderings ↦ 12 orbits, the cells of the K(Z/2,1).
    assert_eq!(
        p.linear_extensions().len() / 2,
        12,
        "free involution ⇒ 12 orbits = the quotient's cells"
    );
}
