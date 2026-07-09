//! Polynomial Calculus — the dynamic strengthening of Nullstellensatz. These tests pin its correctness
//! (sound + complete at full degree, against a brute-force oracle), the domination `PC ⊇ NS`, and they
//! MEASURE whether degree-d PC actually certifies more than degree-d NS over GF(2) on the small-n census.

use logicaffeine_proof::cdcl::Lit;
use logicaffeine_proof::families::random_3sat;
use logicaffeine_proof::hypercube::minimal_cover_orbits;
use logicaffeine_proof::polycalc::{nullstellensatz_refutes, polynomial_calculus_refutes};

fn brute_sat(num_vars: usize, clauses: &[Vec<Lit>]) -> bool {
    (0u64..(1u64 << num_vars)).any(|code| {
        clauses
            .iter()
            .all(|c| c.iter().any(|l| ((code >> l.var()) & 1 == 1) == l.is_positive()))
    })
}

#[test]
fn pc_is_sound_and_complete_at_full_degree() {
    // Over random small formulas, full-degree PC must decide UNSAT exactly — refutes iff brute-UNSAT.
    for seed in 0..200u64 {
        let vars = 4 + (seed % 4) as usize; // 4..=7 vars
        let cnf = random_3sat(vars, vars * 4, seed); // around the hard ratio
        let refuted = polynomial_calculus_refutes(cnf.num_vars, &cnf.clauses, cnf.num_vars);
        assert_eq!(
            refuted,
            !brute_sat(cnf.num_vars, &cnf.clauses),
            "seed={seed}: full-degree PC must refute iff the formula is UNSAT"
        );
    }
}

#[test]
fn pc_dominates_ns_at_every_degree() {
    // Whenever degree-d Nullstellensatz refutes, degree-d Polynomial Calculus must too (PC ⊇ NS).
    for seed in 0..150u64 {
        let vars = 4 + (seed % 4) as usize;
        let cnf = random_3sat(vars, vars * 4, seed);
        for d in 1..=cnf.num_vars {
            if nullstellensatz_refutes(cnf.num_vars, &cnf.clauses, d) {
                assert!(
                    polynomial_calculus_refutes(cnf.num_vars, &cnf.clauses, d),
                    "seed={seed} d={d}: PC must refute everything NS refutes"
                );
            }
        }
    }
}

#[test]
fn pc_never_refutes_a_satisfiable_formula() {
    for seed in 0..200u64 {
        let vars = 4 + (seed % 4) as usize;
        let cnf = random_3sat(vars, vars * 3, seed); // sparser ⇒ usually SAT
        if brute_sat(cnf.num_vars, &cnf.clauses) {
            for d in 1..=cnf.num_vars {
                assert!(
                    !polynomial_calculus_refutes(cnf.num_vars, &cnf.clauses, d),
                    "seed={seed} d={d}: a satisfiable formula must never be PC-refuted"
                );
            }
        }
    }
}

/// Measurement: across the small-n census, count minimal-UNSAT families where degree-d PC certifies but
/// degree-d NS does not — the concrete strict-separation gain of the dynamic engine over GF(2).
#[test]
fn pc_vs_ns_separation_measured() {
    for n in 2..=3 {
        let mut pc_only = 0usize;
        let mut total = 0usize;
        for cover in minimal_cover_orbits(n) {
            let clauses = cover.clauses();
            total += 1;
            // The interesting degree is below full (at full degree both are complete).
            for d in 1..n {
                let ns = nullstellensatz_refutes(n, &clauses, d);
                let pc = polynomial_calculus_refutes(n, &clauses, d);
                assert!(!ns || pc, "PC must dominate NS");
                if pc && !ns {
                    pc_only += 1;
                    break;
                }
            }
        }
        eprintln!("n={n}: {total} families | {pc_only} certified by PC at a degree NS needs more for");
    }
}
