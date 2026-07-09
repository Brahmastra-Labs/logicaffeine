//! The expanded family corpus, run against the real cascade — the companion of `all_routes_certify.rs`,
//! but driven by the families a researcher would actually throw at the solver. Each new generator is
//! decided to its ground-truth verdict by `solve_structured`, and wherever a family has a structural
//! specialist the refutation is re-checked by an INDEPENDENT checker (O(1) counting, GF(2) Gaussian →
//! DRAT/RUP, Horn forward-chaining) — never trusting the dispatcher's own verdict. It also pins WHICH
//! route each family routes to, so a regression that quietly drops a specialist and limps home on brute
//! CDCL is caught here.

use logicaffeine_proof::dimacs::DimacsCnf;
use logicaffeine_proof::families::{self, ExpectedVerdict};
use logicaffeine_proof::hornsat::{self, HornClause};
use logicaffeine_proof::solve::{solve_structured, Answer, Route};
use logicaffeine_proof::xor_drat::emit_xor_drat;
use logicaffeine_proof::{pigeonhole, rup, xorsat};

/// A returned SAT model, re-checked against every clause from scratch.
fn model_satisfies(cnf: &DimacsCnf, model: &[bool]) -> bool {
    cnf.clauses.iter().all(|c| c.iter().any(|l| model[l.var() as usize] == l.is_positive()))
}

/// **Weak pigeonhole routes to the counting specialist — and the count re-checks.** Any `PHP^h_p` with
/// `p > h` (not just the tight `h = p−1`) is decided by the O(1) `pigeons > holes` inequality, never by
/// CDCL search; the satisfiable regime returns a model that checks.
#[test]
fn weak_php_routes_to_counting_and_the_certificate_rechecks() {
    for &(p, h) in &[(6usize, 5usize), (5, 3), (7, 4), (8, 2)] {
        let (cnf, v) = families::weak_php(p, h);
        assert_eq!(v, ExpectedVerdict::Unsat);
        let s = solve_structured(cnf.num_vars, &cnf.clauses);
        assert!(matches!(s.answer, Answer::Unsat), "PHP^{h}_{p} is UNSAT");
        assert_ne!(s.via, Route::Cdcl, "weak PHP^{h}_{p} must be certified structurally, not by CDCL search");
        let cert = pigeonhole::certify_pigeonhole_unsat(p as u128, h as u128).expect("p > h is a counting refutation");
        assert!(pigeonhole::check_counting_cert(&cert), "the weak-PHP counting certificate must re-check");
    }
    let (sat, v) = families::weak_php(4, 7);
    assert_eq!(v, ExpectedVerdict::Sat);
    match solve_structured(sat.num_vars, &sat.clauses).answer {
        Answer::Sat(m) => assert!(model_satisfies(&sat, &m), "the weak-PHP SAT model must satisfy every clause"),
        Answer::Unsat => panic!("PHP with more holes than pigeons is SAT"),
    }
}

/// **The strengthened pigeonhole variants are crushed by a specialist, not search.** Adding the
/// functional ("≤ 1 hole per pigeon") and onto ("every hole filled") clauses keeps PHP UNSAT and within
/// reach of the structural cascade (it routes to the covering-collapse specialist) — the extra clauses
/// must not knock it down to brute CDCL.
#[test]
fn functional_and_onto_php_stay_off_the_cdcl_fallback() {
    for n in 4..=6 {
        for (label, cnf) in [("FPHP", families::functional_php(n).0), ("onto-FPHP", families::onto_php(n).0)] {
            let s = solve_structured(cnf.num_vars, &cnf.clauses);
            assert!(matches!(s.answer, Answer::Unsat), "{label}({n}) is UNSAT");
            assert_ne!(s.via, Route::Cdcl, "{label}({n}) must route to a structural specialist, not brute search");
        }
    }
}

/// **k-XOR certifies algebraically AND as a clausal DRAT proof — at every arity.** Each random k-XOR
/// instance routes to a certified specialist (a 2-XOR is just binary clauses, so it falls to the 2-SAT
/// decider; `k ≥ 3` lands on the GF(2) parity engine — never brute CDCL). Its Gaussian `0 = 1`
/// dependency re-checks (`xorsat::is_refutation`) and compiles to a DRAT proof an independent RUP checker
/// accepts — extending the parity route's certificate ledger past the `k = 3` case in `all_routes_certify.rs`.
#[test]
fn random_kxor_certifies_via_gaussian_and_drat_at_every_arity() {
    for k in [2usize, 3, 4, 5] {
        let (eqs, cnf) = families::random_kxor(k, 16, 18, 0xA11CE ^ k as u64);
        let s = solve_structured(cnf.num_vars, &cnf.clauses);
        assert!(matches!(s.answer, Answer::Unsat), "{k}-XOR is UNSAT");
        let want = if k == 2 { Route::TwoSat } else { Route::Parity };
        assert_eq!(s.via, want, "{k}-XOR must route to its certified specialist ({want:?}), not CDCL");
        let refutation = match xorsat::solve(&eqs, cnf.num_vars) {
            xorsat::XorOutcome::Unsat(r) => r,
            xorsat::XorOutcome::Sat(_) => panic!("planted-then-flipped {k}-XOR must be UNSAT"),
        };
        assert!(xorsat::is_refutation(&eqs, cnf.num_vars, &refutation), "{k}-XOR Gaussian refutation must re-check");
        let drat = emit_xor_drat(&eqs, &refutation).expect("k-XOR compiles to a clausal DRAT proof");
        assert!(rup::check_refutation(cnf.num_vars, &cnf.clauses, &drat), "the {k}-XOR DRAT proof must RUP-refute the CNF");
    }
}

/// Reinterpret a CNF as Horn clauses (≤ 1 positive literal each): an all-negative clause is a goal, a
/// positive unit is a fact, and a clause with one positive literal is a rule (negatives → head). Panics
/// on any clause that is not Horn — so it doubles as a proof the family is genuinely Horn.
fn to_horn(cnf: &DimacsCnf) -> Vec<HornClause> {
    cnf.clauses
        .iter()
        .map(|c| {
            let pos: Vec<usize> = c.iter().filter(|l| l.is_positive()).map(|l| l.var() as usize).collect();
            let neg: Vec<usize> = c.iter().filter(|l| !l.is_positive()).map(|l| l.var() as usize).collect();
            match (pos.len(), neg.is_empty()) {
                (0, _) => HornClause::goal(neg),
                (1, true) => HornClause::fact(pos[0]),
                (1, false) => HornClause::rule(neg, pos[0]),
                _ => panic!("pebbling clause is not Horn: {c:?}"),
            }
        })
        .collect()
}

/// **Pebbling is Horn — linear forward-chaining decides and certifies it.** The pyramid pebbling
/// contradiction routes to the Horn specialist (its propagation clauses have exactly one positive
/// literal); the forward-chaining derivation that fires the goal re-checks independently
/// (`hornsat::is_refutation`), giving the Horn route a real structured family beyond the toy in the ledger.
#[test]
fn pebbling_is_horn_and_forward_chaining_refutes_it() {
    for h in 1..=6 {
        let (cnf, _) = families::pebbling_pyramid(h);
        let s = solve_structured(cnf.num_vars, &cnf.clauses);
        assert!(matches!(s.answer, Answer::Unsat), "pebbling({h}) is UNSAT");
        assert_eq!(s.via, Route::Horn, "pebbling is Horn — the linear forward-chaining decider must own it");
        let horn = to_horn(&cnf);
        match hornsat::solve(&horn, cnf.num_vars) {
            hornsat::HornOutcome::Unsat(d) => {
                assert!(hornsat::is_refutation(&horn, cnf.num_vars, &d), "the pebbling Horn derivation must re-check")
            }
            hornsat::HornOutcome::Sat(_) => panic!("pebbling must be UNSAT"),
        }
    }
}

/// **Modular counting and Ramsey, decided by the cascade.** `Count_q(n)` is UNSAT exactly across the
/// divisibility boundary (`q = 2` is perfect matching on `K_n`, UNSAT for odd `n`); the diagonal Ramsey
/// boundary `R(3,3) = 6` is decided on both sides. These are the general-engine controls — correctness,
/// not a structural crush.
#[test]
fn mod_counting_and_ramsey_decide_against_the_cascade() {
    for &(n, q) in &[(5usize, 2usize), (6, 2), (7, 3), (6, 3), (4, 3), (9, 3)] {
        let (cnf, v) = families::mod_counting(n, q);
        let unsat = matches!(solve_structured(cnf.num_vars, &cnf.clauses).answer, Answer::Unsat);
        assert_eq!(unsat, v == ExpectedVerdict::Unsat, "Count_{q}({n}) verdict (UNSAT iff q ∤ n)");
    }
    let (sat5, _) = families::ramsey(3, 3, 5);
    let (unsat6, _) = families::ramsey(3, 3, 6);
    match solve_structured(sat5.num_vars, &sat5.clauses).answer {
        Answer::Sat(m) => assert!(model_satisfies(&sat5, &m), "Ramsey(3,3;5) model must 2-colour K_5 with no mono triangle"),
        Answer::Unsat => panic!("Ramsey(3,3;5) is SAT (5 < R(3,3) = 6)"),
    }
    assert!(matches!(solve_structured(unsat6.num_vars, &unsat6.clauses).answer, Answer::Unsat), "Ramsey(3,3;6) is UNSAT");
}
