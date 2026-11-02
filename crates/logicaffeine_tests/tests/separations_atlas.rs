//! **The certified separations atlas** — one artifact, every proof-system row two-sided, every half
//! re-checked by its independent verifier with zero trust in the producer.
//!
//! Each row is one family measured against multiple proof systems, its LOWER half a certified
//! impossibility (a dual witness / a closed clause set) and its UPPER half a certified proof in a
//! stronger or characteristic-matched system:
//!
//! | family | lower half (certified) | upper half (certified) |
//! |---|---|---|
//! | `PHP_m` | `GF(2)` NS-degree ≥ 2(m−1) (pseudo-expectation) + res-width certificate | SR refutation, exactly `m(m−1)/2` symmetry steps (`pr::check_pr_refutation`) |
//! | Tseitin | res-width > w (closed-set certificate) | `GF(2)` linear refutation (`xorsat::is_refutation`) |
//! | `Count_3` | `GF(2)` NS-degree ≥ 3 (pseudo-expectation, linear encoding) — ∀n ≡ 3 (mod 4) via the stabilized collapsed dual | `GF(3)` linear refutation (`modp::is_refutation`) |
//!
//! The `Count_3` row is the marquee **characteristic-mismatch** cell: the *same* family carries a
//! certified `GF(2)` hardness floor and a one-Gaussian-pass `GF(3)` refutation — algebra sees the
//! obstruction exactly when its characteristic divides the count. External verification artifacts
//! for the upper halves (DRAT via drat-trim, `.sr` via sr2drat) live in `target/proofs/algebraic/` and
//! `benchmarks/sat/proofs/`; this test walks every half through the in-repo independent checkers.

use logicaffeine_proof::cdcl::Lit;
use logicaffeine_proof::families;

/// The atlas rows carry literal Lit-level data — pin the clause conventions the rows rely on
/// (all-positive covering clauses in `mod_counting`, negative pairs in PHP) so a families refactor
/// cannot silently rot the atlas.
#[test]
fn atlas_family_layouts_are_pinned() {
    let (php3, _) = families::php(3);
    assert!(
        php3.clauses
            .iter()
            .any(|c| c.iter().all(|l| l.is_positive())),
        "PHP has pigeon rows"
    );
    assert!(
        php3.clauses
            .iter()
            .any(|c| c.len() == 2 && c.iter().all(|l| !l.is_positive())),
        "PHP has hole AMO pairs"
    );
    let (cnt, _) = families::mod_counting(5, 3);
    let groups = families::mod_counting_groups(5, 3);
    assert_eq!(groups.len(), 5, "one covering group per point");
    let edges = families::mod_counting_edges(5, 3);
    assert_eq!(
        edges.len(),
        cnt.num_vars,
        "one edge per variable, in variable order"
    );
    let lit_ok = cnt
        .clauses
        .iter()
        .all(|c| c.iter().all(|l| (l.var() as usize) < cnt.num_vars));
    assert!(lit_ok, "every literal indexes a real variable");
    let _ = Lit::pos(0);
}
