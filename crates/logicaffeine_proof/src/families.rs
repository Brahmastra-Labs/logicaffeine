//! Parametric generators for symmetry-rich SAT families — the canonical hard cases where
//! symmetry breaking earns its keep. They are programmatic (reproducible, offline, parametric)
//! rather than vendored `.cnf` files, and each is pinned to its known verdict so the solver and
//! the certified pipeline can be tested against ground truth.

use crate::cdcl::Lit;
use crate::dimacs::DimacsCnf;

/// The known verdict of a generated instance, for test oracles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExpectedVerdict {
    Sat,
    Unsat,
}

/// The pigeonhole principle PHP(n): `n` pigeons into `n-1` holes — unsatisfiable, and the
/// textbook symmetry / resolution-hard family (any resolution refutation is exponential, while
/// breaking the `S_n × S_{n-1}` symmetry collapses it). The variable for "pigeon `p` sits in
/// hole `h`" lives at index `p*(n-1) + h`, so pigeons index rows and holes index columns.
pub fn php(n: usize) -> (DimacsCnf, ExpectedVerdict) {
    let holes = n.saturating_sub(1);
    let num_vars = n * holes;
    let var = |p: usize, h: usize| Lit::pos((p * holes + h) as u32);
    let mut clauses: Vec<Vec<Lit>> = Vec::new();
    // Each pigeon occupies at least one hole (an empty disjunction when holes == 0).
    for p in 0..n {
        clauses.push((0..holes).map(|h| var(p, h)).collect());
    }
    // No two pigeons share a hole.
    for h in 0..holes {
        for p in 0..n {
            for q in (p + 1)..n {
                clauses.push(vec![var(p, h).negated(), var(q, h).negated()]);
            }
        }
    }
    (DimacsCnf { num_vars, clauses }, ExpectedVerdict::Unsat)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cdcl::SolveResult;

    #[test]
    fn php_has_the_expected_shape() {
        let (cnf, verdict) = php(4);
        assert_eq!(verdict, ExpectedVerdict::Unsat);
        assert_eq!(cnf.num_vars, 4 * 3);
        // 4 "at least one hole" clauses + 3 holes × C(4,2)=6 conflict clauses.
        assert_eq!(cnf.clauses.len(), 4 + 3 * 6);
    }

    #[test]
    fn php_is_unsatisfiable_for_small_n() {
        for n in 1..=5 {
            let (cnf, _) = php(n);
            assert_eq!(
                cnf.into_solver().solve(),
                SolveResult::Unsat,
                "PHP({n}) must be unsatisfiable"
            );
        }
    }
}
