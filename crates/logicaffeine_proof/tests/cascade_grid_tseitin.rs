//! Tseitin parity on a w×n grid — bounded treewidth (= w, a fixed constant), UNSAT, decided by GF(2)
//! Gaussian in near-linear time. It is the sharpest field-crush: a polynomial-size bounded-width
//! resolution proof provably exists (bounded treewidth), yet CDCL solvers without Gaussian reasoning
//! (Kissat/CaDiCaL/CryptoMiniSat) all TIME OUT on the parity at w ≥ 12 — measured. These lock the
//! refutation and guard the satisfiable (even-charge) case against a false refutation.

use logicaffeine_proof::families::{grid_tseitin, ExpectedVerdict};
use logicaffeine_proof::xorsat::{self, XorOutcome};

#[test]
fn grid_tseitin_is_unsat_and_refuted_by_gf2() {
    for (w, n) in [(4usize, 6usize), (6, 10), (10, 40), (12, 60)] {
        let (eqs, cnf, v) = grid_tseitin(w, n);
        assert_eq!(v, ExpectedVerdict::Unsat);
        assert!(
            matches!(xorsat::solve(&eqs, cnf.num_vars), XorOutcome::Unsat(_)),
            "grid {w}x{n} Tseitin (odd charge) must be refuted by GF(2) Gaussian"
        );
        // the XOR system and the CNF share the same edge variables.
        assert!(eqs.iter().flat_map(|e| &e.vars).all(|&x| x < cnf.num_vars), "vars in range");
        assert!(cnf.num_vars >= w * n - 1, "≈ one edge per grid adjacency");
    }
}

#[test]
fn even_charge_grid_is_satisfiable_not_refuted() {
    // Flip the single odd charge → the total becomes EVEN ⇒ satisfiable. GF(2) must return a model, never
    // a refutation (the soundness boundary — the parity cut must not over-fire on a consistent system).
    for (w, n) in [(4usize, 6usize), (6, 10), (8, 20)] {
        let (mut eqs, cnf, _) = grid_tseitin(w, n);
        eqs[0].rhs = !eqs[0].rhs;
        match xorsat::solve(&eqs, cnf.num_vars) {
            XorOutcome::Sat(_) => {}
            XorOutcome::Unsat(_) => panic!("even-charge grid {w}x{n} is satisfiable — GF(2) must not refute it"),
        }
    }
}
