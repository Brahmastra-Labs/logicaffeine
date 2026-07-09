//! The certified cascade must use the covering/cardinality/parity *collapse* engine we already have:
//! `auto_collapse` certifies the irregularly-encoded coverings and counting cores that the strict
//! structural recognizers (pigeonhole, cutting-planes) miss. The exhaustive census found 756 such
//! minimal-UNSAT families at n=4 alone — concrete coverage the cascade was leaving on the table.

use logicaffeine_proof::cdcl::Lit;
use logicaffeine_proof::hypercube::clauses_to_expr;
use logicaffeine_proof::lyapunov::{auto_collapse, AutoCollapse};
use logicaffeine_proof::sat::{prove_unsat, UnsatOutcome};
use logicaffeine_proof::{pigeonhole, pseudo_boolean, xorsat};

#[test]
fn cascade_certifies_irregular_counting_core_via_collapse() {
    // The counting core the census surfaced: x0 ∧ x1 ∧ x2 ∧ (¬x0∨¬x1∨¬x2) — "all three true"
    // contradicts "at least one false". A pure cardinality contradiction the narrow recognizers miss.
    let p = |v: u32| Lit::new(v, true);
    let q = |v: u32| Lit::new(v, false);
    let clauses = vec![vec![p(0)], vec![p(1)], vec![p(2)], vec![q(0), q(1), q(2)]];
    let e = clauses_to_expr(&clauses).unwrap();

    // The narrow structural cuts genuinely do NOT fire on it...
    assert!(!pigeonhole::decide_pigeonhole_unsat(&e), "pigeonhole recognizer does not fire");
    assert!(!pseudo_boolean::refute_clausal(&e), "cutting-planes recognizer does not fire");
    assert!(!xorsat::refute_via_parity(&e), "parity recognizer does not fire");
    // ...but the covering/cardinality collapse engine certifies it...
    assert!(
        !matches!(auto_collapse(3, &clauses), AutoCollapse::None),
        "auto_collapse must certify the counting core"
    );
    // ...and the cascade now refutes it through that cut.
    assert_eq!(prove_unsat(&e), UnsatOutcome::Refuted);
}

#[test]
fn collapse_cut_stays_sound_on_satisfiable() {
    // Covering-ish shape but SATISFIABLE: x0 ∧ x1 ∧ (¬x0∨¬x1∨x2) forces x0=x1=x2=true. The collapse
    // cut must NOT report a false `Refuted`.
    let p = |v: u32| Lit::new(v, true);
    let q = |v: u32| Lit::new(v, false);
    let clauses = vec![vec![p(0)], vec![p(1)], vec![q(0), q(1), p(2)]];
    let e = clauses_to_expr(&clauses).unwrap();
    assert!(
        matches!(prove_unsat(&e), UnsatOutcome::Sat(_)),
        "a satisfiable formula must never be falsely refuted by the collapse cut"
    );
}
