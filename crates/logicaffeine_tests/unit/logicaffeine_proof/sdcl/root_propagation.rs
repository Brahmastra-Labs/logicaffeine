//! A solved root assignment needs no symmetry search; unresolved inputs retain
//! the certified solver. Work is counted, not inferred from noisy wall time.
use super::*;

fn run(vars: usize, clauses: &[Vec<Lit>]) -> (CertifiedOutcome, usize) {
    let mut entries = 0;
    let result = solve_with_discovery(vars, clauses, || {
        entries += 1;
        sdcl_discover(vars, clauses)
    });
    (result, entries)
}

fn model(vars: usize, clauses: &[Vec<Lit>], result: CertifiedOutcome) -> Vec<bool> {
    let CertifiedOutcome::Sat(model) = result else {
        panic!("the formula is satisfiable")
    };
    assert_eq!(
        model.len(),
        vars,
        "the witness covers all declared variables"
    );
    for clause in clauses {
        assert!(
            clause
                .iter()
                .any(|lit| model[lit.var() as usize] == lit.is_positive()),
            "the returned witness must satisfy every ORIGINAL clause"
        );
    }
    model
}

#[test]
fn root_forced_sat_is_checked_before_symmetry_discovery() {
    let clauses = vec![
        vec![Lit::pos(0)],
        vec![Lit::neg(1)],
        vec![Lit::pos(2)],
        vec![Lit::neg(3)],
        vec![Lit::pos(0), Lit::pos(1)],
        vec![Lit::neg(2), Lit::neg(3)],
    ];
    let (result, entries) = run(4, &clauses);
    assert_eq!(model(4, &clauses, result), [true, false, true, false]);
    assert_eq!(
        entries, 0,
        "a checked forced model needs no SDCL/symmetry discovery"
    );
}

#[test]
fn root_propagation_follows_a_reverse_ordered_implication_chain() {
    let clauses = vec![
        vec![Lit::neg(2), Lit::neg(3)],
        vec![Lit::neg(1), Lit::pos(2)],
        vec![Lit::neg(0), Lit::pos(1)],
        vec![Lit::pos(0)],
    ];
    let (result, entries) = run(4, &clauses);
    assert_eq!(model(4, &clauses, result), [true, true, true, false]);
    assert_eq!(
        entries, 0,
        "propagation, including another pass over earlier clauses, decides this model"
    );
}

#[test]
fn an_empty_formula_returns_a_complete_model_without_discovery() {
    let (result, entries) = run(3, &[]);
    model(3, &[], result);
    assert_eq!(entries, 0);
}

#[test]
fn an_unresolved_formula_keeps_the_certified_search_path() {
    let clauses = vec![vec![Lit::pos(0), Lit::pos(1)]];
    let (result, entries) = run(2, &clauses);
    model(2, &clauses, result);
    assert!(entries > 0, "no root assignment establishes a model yet");
}

#[test]
fn a_root_conflict_keeps_an_independently_checkable_refutation() {
    let clauses = vec![vec![Lit::pos(0)], vec![Lit::neg(0)]];
    let (result, entries) = run(2, &clauses);
    let CertifiedOutcome::Unsat { steps, .. } = result else {
        panic!("contradictory units are unsatisfiable")
    };
    assert!(
        check_pr_refutation(2, &clauses, &steps),
        "check against the original clauses"
    );
    assert!(
        entries > 0,
        "the SAT preflight must not fabricate an UNSAT certificate"
    );
}

#[test]
fn duplicate_unit_literals_still_propagate_before_discovery() {
    let clauses = vec![
        vec![Lit::pos(0), Lit::pos(0)],
        vec![Lit::neg(0), Lit::pos(1)],
    ];
    let (result, entries) = run(2, &clauses);
    assert_eq!(model(2, &clauses, result), [true, true]);
    assert_eq!(entries, 0);
}

#[test]
fn empty_clauses_and_out_of_range_literals_do_not_earn_a_model() {
    assert!(checked_root_model(0, &[vec![]]).is_none());
    assert!(checked_root_model(1, &[vec![Lit::pos(1)]]).is_none());
    assert!(checked_root_model(1, &[vec![Lit::neg(1)]]).is_none());
}

#[test]
fn tautologies_do_not_force_their_variable() {
    let clauses = vec![vec![Lit::pos(0), Lit::neg(0)], vec![Lit::neg(0)]];
    let candidate =
        checked_root_model(1, &clauses).expect("the negative unit decides this formula");
    assert_eq!(candidate, [false]);
}

#[test]
fn every_two_variable_formula_checks_the_original_clauses() {
    let literals = [Lit::pos(0), Lit::neg(0), Lit::pos(1), Lit::neg(1)];
    let clauses: Vec<Vec<Lit>> = (0usize..16)
        .map(|mask| {
            literals
                .iter()
                .enumerate()
                .filter_map(|(i, &lit)| (mask & (1 << i) != 0).then_some(lit))
                .collect()
        })
        .collect();
    for formula_mask in 0u32..(1 << 16) {
        let formula: Vec<Vec<Lit>> = clauses
            .iter()
            .enumerate()
            .filter_map(|(i, clause)| (formula_mask & (1 << i) != 0).then(|| clause.clone()))
            .collect();
        let sat = (0usize..4).any(|bits| {
            formula.iter().all(|clause| {
                clause
                    .iter()
                    .any(|lit| ((bits >> lit.var()) & 1 != 0) == lit.is_positive())
            })
        });
        if let Some(candidate) = checked_root_model(2, &formula) {
            assert!(sat, "a preflight model must not turn UNSAT into SAT");
            model(2, &formula, CertifiedOutcome::Sat(candidate));
        }
    }
}
