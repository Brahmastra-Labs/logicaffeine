use super::*;
use crate::cdcl::Lit;
use crate::{ProofExpr, ProofTerm};

fn c(s: &str) -> ProofTerm {
    ProofTerm::Constant(s.to_string())
}
fn pred(name: &str, args: Vec<ProofTerm>) -> ProofExpr {
    ProofExpr::Predicate {
        name: name.to_string(),
        args,
        world: None,
    }
}
fn not(e: ProofExpr) -> ProofExpr {
    ProofExpr::Not(Box::new(e))
}
fn or(a: ProofExpr, b: ProofExpr) -> ProofExpr {
    ProofExpr::Or(Box::new(a), Box::new(b))
}
fn imp(a: ProofExpr, b: ProofExpr) -> ProofExpr {
    ProofExpr::Implies(Box::new(a), Box::new(b))
}

#[test]
fn certified_disjunctive_syllogism() {
    let p = pred("P", vec![c("a")]);
    let q = pred("Q", vec![c("a")]);
    let prem = vec![or(p.clone(), q.clone()), not(p.clone())];
    assert_eq!(entails_certified(&prem, &q), Some(Verdict::Entailed));
    assert_eq!(
        entails_certified(&[or(p, q.clone())], &q),
        Some(Verdict::NotEntailed)
    );
}

#[test]
fn certified_two_value_grid() {
    let in_ = |t: &str, s: &str| pred("In", vec![c(t), c(s)]);
    let (fl, me) = ("Florida", "Maine");
    let prem = vec![
        or(in_("Alpha", fl), in_("Alpha", me)),
        or(in_("Beta", fl), in_("Beta", me)),
        imp(in_("Alpha", fl), not(in_("Beta", fl))),
        imp(in_("Alpha", me), not(in_("Beta", me))),
        or(in_("Alpha", fl), in_("Beta", fl)),
        or(in_("Alpha", me), in_("Beta", me)),
        in_("Alpha", fl),
    ];
    assert_eq!(
        entails_certified(&prem, &in_("Beta", me)),
        Some(Verdict::Entailed)
    );
    assert_eq!(
        entails_certified(&prem, &in_("Beta", fl)),
        Some(Verdict::NotEntailed)
    );
}

#[test]
fn rup_rejects_a_bogus_proof() {
    // Original `(p ∨ q)` is SATISFIABLE. A "proof" that bolts on the un-entailed units
    // `¬p`, `¬q` must NOT verify — the checker rejects clauses that are not RUP.
    let p = Lit::pos(0);
    let q = Lit::pos(1);
    let original = vec![vec![p, q]];
    let bogus_learned = vec![vec![p.negated()], vec![q.negated()]];
    assert!(
        !check_refutation(2, &original, &bogus_learned),
        "a non-RUP learned clause must be rejected"
    );
}

#[test]
fn rup_needs_the_proof_not_just_the_clauses() {
    // A genuinely UNSAT formula whose unsatisfiability is NOT visible to unit
    // propagation alone (all four 2-literal clauses over p,q) must fail to verify
    // WITHOUT the solver's learned clauses — proving the checker really replays the
    // proof rather than re-searching.
    let p = Lit::pos(0);
    let q = Lit::pos(1);
    let original = vec![
        vec![p, q],
        vec![p, q.negated()],
        vec![p.negated(), q],
        vec![p.negated(), q.negated()],
    ];
    assert!(
        !check_refutation(2, &original, &[]),
        "UP alone cannot refute this; needs the proof"
    );
    // With the resolvents the solver would learn (`p`, then `¬p` ⇒ empty), it checks.
    let learned = vec![vec![p], vec![p.negated()]];
    assert!(
        check_refutation(2, &original, &learned),
        "the learned resolvents make it RUP"
    );
}

#[test]
fn certified_prepared_agrees_with_the_from_scratch_check() {
    // entails_certified_prepared solves against a premise CNF clausified ONCE (the incremental
    // path used per puzzle-cell) — it must return the SAME certified verdict as
    // entails_certified. Kills `entails_certified_prepared -> None`.
    let p = pred("P", vec![c("a")]);
    let q = pred("Q", vec![c("a")]);
    let prepared =
        Cnf::from_premises(&[or(p.clone(), q.clone()), not(p.clone())]).expect("clausify");
    assert_eq!(
        entails_certified_prepared(&prepared, &q),
        Some(Verdict::Entailed)
    );
    // Dropping `¬p` leaves `(p ∨ q)`, which does NOT entail `q` → NotEntailed on the same path.
    let prepared_sat = Cnf::from_premises(&[or(p, q.clone())]).expect("clausify");
    assert_eq!(
        entails_certified_prepared(&prepared_sat, &q),
        Some(Verdict::NotEntailed)
    );
}

/// The clause rule as the checker first wrote it — every distinct unset literal collected, a tautology read as
/// satisfied — kept as the oracle `clause_state` answers for. Only its effect on propagation is compared: a
/// satisfied clause and one with two distinct unset literals both leave the assignment as it is.
fn collected_effect(clause: &[Lit], assign: &[Option<bool>]) -> ClauseState {
    let mut unset: Vec<Lit> = Vec::new();
    for &l in clause {
        match lit_val(assign, l) {
            Some(true) => return ClauseState::Quiet,
            Some(false) => {}
            None => {
                if unset.contains(&l.negated()) {
                    return ClauseState::Quiet;
                }
                if !unset.contains(&l) {
                    unset.push(l);
                }
            }
        }
    }
    match unset.as_slice() {
        [] => ClauseState::Conflict,
        [u] => ClauseState::Unit(*u),
        _ => ClauseState::Quiet,
    }
}

/// Fixpoint propagation over the oracle's clause rule, as `propagate` runs it.
fn collected_propagate(db: &[Vec<Lit>], assign: &mut [Option<bool>]) -> bool {
    loop {
        let mut changed = false;
        for clause in db {
            match collected_effect(clause, assign) {
                ClauseState::Conflict => return true,
                ClauseState::Unit(l) => {
                    set_true(assign, l);
                    changed = true;
                }
                ClauseState::Quiet => {}
            }
        }
        if !changed {
            return false;
        }
    }
}

fn x() -> Lit {
    Lit::pos(0)
}
fn y() -> Lit {
    Lit::pos(1)
}
fn z() -> Lit {
    Lit::pos(2)
}

#[test]
fn a_clause_whose_every_literal_is_false_is_a_conflict() {
    let assign = [Some(false), Some(true), None];
    assert_eq!(clause_state(&[x(), y().negated()], &assign), ClauseState::Conflict);
    assert_eq!(clause_state(&[x(), x()], &assign), ClauseState::Conflict);
    assert_eq!(clause_state(&[], &assign), ClauseState::Conflict, "the empty clause");
}

#[test]
fn one_unset_literal_and_no_true_one_forces_it() {
    let assign = [Some(false), None, None];
    assert_eq!(clause_state(&[x(), y()], &assign), ClauseState::Unit(y()));
    assert_eq!(clause_state(&[y().negated(), x()], &assign), ClauseState::Unit(y().negated()));
}

#[test]
fn a_repeated_literal_counts_once() {
    let assign = [Some(false), None, None];
    assert_eq!(clause_state(&[y(), y()], &assign), ClauseState::Unit(y()));
    assert_eq!(clause_state(&[y(), x(), y(), x()], &assign), ClauseState::Unit(y()));
}

#[test]
fn a_tautology_never_propagates() {
    let assign = [Some(false), None, None];
    assert_eq!(clause_state(&[y(), y().negated()], &assign), ClauseState::Quiet);
    assert_eq!(clause_state(&[y(), x(), y().negated(), y()], &assign), ClauseState::Quiet);
}

#[test]
fn a_true_literal_quiets_the_clause_wherever_it_stands() {
    let assign = [Some(true), None, Some(false)];
    assert_eq!(clause_state(&[x(), y()], &assign), ClauseState::Quiet);
    assert_eq!(clause_state(&[y(), z(), x()], &assign), ClauseState::Quiet);
    assert_eq!(clause_state(&[z(), z().negated()], &assign), ClauseState::Quiet, "¬z is true");
}

#[test]
fn two_distinct_unset_literals_quiet_the_clause() {
    let assign = [None, None, Some(false)];
    assert_eq!(clause_state(&[x(), y()], &assign), ClauseState::Quiet);
    assert_eq!(clause_state(&[x(), z(), y(), z()], &assign), ClauseState::Quiet);
    assert_eq!(clause_state(&[x(), y().negated(), x()], &assign), ClauseState::Quiet);
}

/// Every clause of up to four literals over three variables, under every partial assignment of them: `clause_state`
/// has the collected rule's effect on each one.
#[test]
fn clause_state_answers_as_the_collected_rule_on_every_small_clause() {
    let lits: Vec<Lit> = (0..3).flat_map(|v| [Lit::pos(v), Lit::neg(v)]).collect();
    let values = [None, Some(false), Some(true)];
    let mut clauses: Vec<Vec<Lit>> = vec![Vec::new()];
    let mut frontier = clauses.clone();
    for _ in 0..4 {
        frontier = frontier
            .iter()
            .flat_map(|c| lits.iter().map(move |&l| [c.as_slice(), &[l]].concat()))
            .collect();
        clauses.extend(frontier.iter().cloned());
    }
    assert_eq!(clauses.len(), 1 + 6 + 36 + 216 + 1296);
    let mut cases = 0;
    for a in values {
        for b in values {
            for c in values {
                let assign = [a, b, c];
                for clause in &clauses {
                    assert_eq!(
                        clause_state(clause, &assign),
                        collected_effect(clause, &assign),
                        "clause {clause:?} under {assign:?}"
                    );
                    cases += 1;
                }
            }
        }
    }
    assert_eq!(cases, 27 * clauses.len());
}

/// Random databases over a handful of variables, propagated from random partial assignments: `propagate` reaches
/// the collected rule's verdict and leaves the same assignment.
#[test]
fn propagation_answers_as_the_collected_rule_on_random_databases() {
    let mut state: u64 = 0x5EED_0F_C1A05E;
    let mut next = move || {
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    };
    let (mut conflicts, mut quiet) = (0, 0);
    for _ in 0..4000 {
        let num_vars = 1 + (next() % 7) as usize;
        let db: Vec<Vec<Lit>> = (0..1 + next() % 14)
            .map(|_| {
                (0..next() % 5)
                    .map(|_| Lit::new((next() % num_vars as u64) as u32, next() % 2 == 0))
                    .collect()
            })
            .collect();
        let start: Vec<Option<bool>> = (0..num_vars)
            .map(|_| match next() % 4 {
                0 => Some(true),
                1 => Some(false),
                _ => None,
            })
            .collect();
        let (mut ours, mut oracle) = (start.clone(), start.clone());
        let verdict = propagate(&db, &mut ours);
        assert_eq!(verdict, collected_propagate(&db, &mut oracle), "verdict on {db:?} from {start:?}");
        assert_eq!(ours, oracle, "assignment after propagating {db:?} from {start:?}");
        if verdict {
            conflicts += 1;
        } else {
            quiet += 1;
        }
    }
    assert!(conflicts > 400 && quiet > 400, "both verdicts are exercised: {conflicts} conflicts, {quiet} fixpoints");
}
