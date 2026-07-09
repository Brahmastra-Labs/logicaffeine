//! Ramsey formulas Ramsey(s,t;n): 2-colour the edges of K_n avoiding a red K_s and a blue K_t — UNSAT
//! exactly when n >= R(s,t). A pure clique geometry carrying the full S_n vertex-relabeling symmetry.
//! These lock CORRECTNESS on both sides at the scales the cascade decides quickly: the refutation at
//! n = R, and — the critical soundness boundary — a re-checked valid 2-colouring at n = R-1, which the
//! solver must never falsely refute.
//!
//! NOTE: the WIN at the (3,5;14)+ frontier is an OPEN capability. Measurement shows the generic
//! symmetry arsenal is ~2600x SLOWER than plain CDCL here (its Schreier-Sims automorphism search is
//! the bottleneck), so a genuine Ramsey win needs fast *structural* symmetry breaking (S_n edge-symmetry
//! generators derived directly), not the existing engine — tracked separately, not claimed as a win.

use logicaffeine_proof::dimacs::DimacsCnf;
use logicaffeine_proof::families::{self, ExpectedVerdict};
use logicaffeine_proof::solve::{solve_structured, Answer};

/// A returned SAT model, re-checked against every clause from scratch.
fn model_satisfies(cnf: &DimacsCnf, model: &[bool]) -> bool {
    cnf.clauses.iter().all(|c| c.iter().any(|l| model[l.var() as usize] == l.is_positive()))
}

#[test]
fn ramsey_is_refuted_at_the_decidable_scales() {
    // n = R(s,t) → UNSAT, decided quickly by the cascade (via CDCL today). (3,5;14)+ is the open
    // frontier the module note describes — not asserted here, since no fast route wins it yet.
    for (s, t) in [(3usize, 3usize), (3, 4)] {
        let r = families::ramsey_number(s, t).expect("known Ramsey number");
        let (cnf, v) = families::ramsey(s, t, r);
        assert_eq!(v, ExpectedVerdict::Unsat, "Ramsey({s},{t};{r}) is UNSAT at n = R");
        let solved = solve_structured(cnf.num_vars, &cnf.clauses);
        assert!(matches!(solved.answer, Answer::Unsat), "Ramsey({s},{t};{r}) must be refuted (n = R)");
    }
}

#[test]
fn ramsey_returns_a_valid_colouring_below_the_number() {
    // n = R(s,t) - 1 → SAT: a 2-colouring with no red K_s / blue K_t must be returned and re-check. The
    // solver must NEVER falsely refute a colourable instance — the critical soundness boundary.
    for (s, t) in [(3usize, 3usize), (3, 4)] {
        let r = families::ramsey_number(s, t).unwrap();
        let (cnf, v) = families::ramsey(s, t, r - 1);
        assert_eq!(v, ExpectedVerdict::Sat, "Ramsey({s},{t};{}) is SAT below R", r - 1);
        match solve_structured(cnf.num_vars, &cnf.clauses).answer {
            Answer::Sat(m) => assert!(
                model_satisfies(&cnf, &m),
                "Ramsey({s},{t};{}) model must be a valid 2-colouring",
                r - 1
            ),
            Answer::Unsat => panic!("Ramsey({s},{t};{}) is SAT (n < R) — must not falsely refute", r - 1),
        }
    }
}
