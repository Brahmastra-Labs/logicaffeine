//! DIMACS CNF parser — the standard SAT-competition input format. These tests pin the
//! parser as a fail-closed front door: a well-formed instance round-trips and solves to the
//! right verdict; a malformed instance is rejected, never silently mis-parsed.

use logicaffeine_proof::cdcl::{Lit, SolveResult};
use logicaffeine_proof::dimacs::{self, DimacsError};

/// DIMACS literal integer → packed `Lit` (`k` is positive var `k-1`, `-k` is its negation).
fn lit(i: i64) -> Lit {
    Lit::new((i.unsigned_abs() as u32) - 1, i > 0)
}

/// Does `model` (one bool per variable) satisfy `clause`?
fn clause_sat(model: &[bool], clause: &[Lit]) -> bool {
    clause.iter().any(|l| model[l.var() as usize] == l.is_positive())
}

#[test]
fn parses_and_solves_an_unsat_instance() {
    // All four 2-clauses over p,q — unsatisfiable, and not refutable by UP alone.
    let src = "p cnf 2 4\n1 2 0\n1 -2 0\n-1 2 0\n-1 -2 0\n";
    let cnf = dimacs::parse(src).expect("well-formed UNSAT instance parses");
    assert_eq!(cnf.num_vars, 2);
    assert_eq!(cnf.clauses.len(), 4);
    assert_eq!(cnf.into_solver().solve(), SolveResult::Unsat);
}

#[test]
fn parses_and_solves_a_sat_instance_with_a_real_model() {
    let src = "p cnf 3 3\n1 -3 0\n2 3 0\n-1 2 0\n";
    let cnf = dimacs::parse(src).expect("well-formed SAT instance parses");
    match cnf.into_solver().solve() {
        SolveResult::Sat(model) => {
            assert_eq!(model.len(), 3, "model covers every variable");
            for clause in &cnf.clauses {
                assert!(clause_sat(&model, clause), "model must satisfy {clause:?}");
            }
        }
        SolveResult::Unsat => panic!("instance is satisfiable"),
    }
}

#[test]
fn parses_exact_clause_literals() {
    let cnf = dimacs::parse("p cnf 3 2\n1 -2 3 0\n-3 0\n").unwrap();
    assert_eq!(cnf.clauses, vec![vec![lit(1), lit(-2), lit(3)], vec![lit(-3)]]);
}

#[test]
fn round_trips_through_print() {
    let src = "p cnf 4 3\n1 2 0\n-3 4 0\n-1 -2 3 -4 0\n";
    let cnf = dimacs::parse(src).unwrap();
    let printed = dimacs::print(&cnf);
    let reparsed = dimacs::parse(&printed).expect("printed text re-parses");
    assert_eq!(reparsed, cnf, "parse(print(x)) == x");
}

#[test]
fn ignores_comments_and_tolerates_multiline_and_packed_clauses() {
    // `c` comments, blank lines, a clause split across two lines, two clauses on one line.
    let src = "c the pigeonhole, in miniature\n\
               p cnf 3 3\n\
               1 2\n3 0\n\
               -1 -2 0 -2 -3 0\n";
    let cnf = dimacs::parse(src).unwrap();
    assert_eq!(
        cnf.clauses,
        vec![vec![lit(1), lit(2), lit(3)], vec![lit(-1), lit(-2)], vec![lit(-2), lit(-3)]]
    );
}

#[test]
fn an_empty_clause_makes_the_formula_unsat() {
    // A lone `0` is a clause with no literals — the empty clause — so the formula is UNSAT.
    let cnf = dimacs::parse("p cnf 1 1\n0\n").unwrap();
    assert_eq!(cnf.clauses, vec![Vec::<Lit>::new()]);
    assert_eq!(cnf.into_solver().solve(), SolveResult::Unsat);
}

#[test]
fn rejects_a_missing_header() {
    assert!(matches!(dimacs::parse("1 2 0\n-1 0\n"), Err(DimacsError::MissingHeader)));
}

#[test]
fn rejects_a_malformed_header() {
    assert!(matches!(dimacs::parse("p cnf two 4\n1 0\n"), Err(DimacsError::MalformedHeader(_))));
    assert!(matches!(dimacs::parse("p dnf 2 4\n1 0\n"), Err(DimacsError::MalformedHeader(_))));
}

#[test]
fn rejects_a_variable_out_of_range() {
    // var 5 exceeds the declared `p cnf 3 1`.
    assert!(matches!(
        dimacs::parse("p cnf 3 1\n1 5 0\n"),
        Err(DimacsError::VarOutOfRange { .. })
    ));
}

#[test]
fn rejects_an_unterminated_final_clause() {
    // Trailing literals with no closing `0` are a structural error, not a silent clause.
    assert!(matches!(dimacs::parse("p cnf 2 1\n1 2\n"), Err(DimacsError::UnterminatedClause)));
}

#[test]
fn rejects_a_non_integer_token() {
    assert!(matches!(dimacs::parse("p cnf 2 1\n1 x 0\n"), Err(DimacsError::InvalidToken(_))));
}
