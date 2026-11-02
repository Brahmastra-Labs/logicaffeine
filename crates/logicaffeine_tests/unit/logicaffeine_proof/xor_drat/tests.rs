use super::*;
use crate::xorsat::{self, XorOutcome};

#[test]
fn xor_linear_dependency_compiles_to_a_drat_refutation_checked_by_rup() {
    // x0⊕x1=0, x1⊕x2=0, x0⊕x2=1 — their GF(2) sum is 0=1, so UNSAT. The CNF is the union of the
    // three parity gadgets. We compile the linear-dependency certificate to DRAT and require our
    // INDEPENDENT RUP checker to accept it, ending in the empty clause.
    let eqs = vec![
        XorEquation::new(vec![0, 1], false),
        XorEquation::new(vec![1, 2], false),
        XorEquation::new(vec![0, 2], true),
    ];
    let num_vars = 3;
    let clauses: Vec<Vec<Lit>> = eqs.iter().flat_map(gadget_clauses).collect();

    let refutation = match xorsat::solve(&eqs, num_vars) {
        XorOutcome::Unsat(s) => s,
        XorOutcome::Sat(_) => panic!("the system is UNSAT"),
    };
    let drat = emit_xor_drat(&eqs, &refutation).expect("a linear dependency must compile to DRAT");
    assert!(
        drat.last().is_some_and(|c| c.is_empty()),
        "the proof must end in the empty clause"
    );
    assert!(
        crate::rup::check_refutation(num_vars, &clauses, &drat),
        "every DRAT line must be RUP and the proof must refute the CNF"
    );
}
