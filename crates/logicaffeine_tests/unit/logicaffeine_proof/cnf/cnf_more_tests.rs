//! Coverage for `from_premises` (bulk clausification) and `atom_value` (model decoding) — the
//! two cnf entry points the `cdcl_entails`-based tests above don't exercise.
use super::*;
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

#[test]
fn atom_key_is_injective_over_arity_and_argument_order() {
    // Distinct atoms MUST get distinct keys — the key is the identity of a SAT variable, so a
    // collision silently conflates two different propositions into one. The comma separator and
    // the argument order are load-bearing: P(a,b), P(b,a), P(a) and the single-arg P("ab") must
    // all differ. (The propositional differential can't see this — it uses only unary atoms.)
    let key = |e: &ProofExpr| atom_key(e).expect("a predicate has a canonical key");
    let atoms = [
        pred("P", vec![c("a"), c("b")]),
        pred("P", vec![c("b"), c("a")]),
        pred("P", vec![c("a")]),
        pred("P", vec![c("ab")]),
    ];
    let keys: Vec<String> = atoms.iter().map(key).collect();
    for i in 0..keys.len() {
        for j in (i + 1)..keys.len() {
            assert_ne!(
                keys[i], keys[j],
                "atoms {i} and {j} must not collide onto one variable"
            );
        }
    }
    // The `Atom(name)` variant is a recognised atom with its own key (not dropped to `None`).
    assert_eq!(
        atom_key(&ProofExpr::Atom("flag".to_string())),
        Some("atom:flag".to_string())
    );
}

#[test]
fn from_premises_equals_incremental_assertion() {
    let p = pred("P", vec![c("a")]);
    let q = pred("Q", vec![c("a")]);
    let prem = vec![or(p.clone(), q.clone()), not(p.clone())];
    let bulk = Cnf::from_premises(&prem).expect("premises are encodable");
    let mut manual = Cnf::new();
    for e in &prem {
        manual.assert(e).unwrap();
    }
    assert_eq!(
        bulk.clauses(),
        manual.clauses(),
        "bulk == incremental clausification"
    );
    assert_eq!(bulk.num_vars(), manual.num_vars());
}

#[test]
fn atom_value_reads_an_atom_from_a_sat_model_and_declines_unknowns() {
    let p = pred("P", vec![c("a")]);
    let mut cnf = Cnf::new();
    cnf.assert(&p).unwrap(); // unit clause forcing P true
    let probe = cnf.clone();
    let mut solver = cnf.into_solver();
    match solver.solve() {
        SolveResult::Sat(model) => {
            assert_eq!(
                probe.atom_value(&p, &model),
                Some(true),
                "asserted P is true"
            );
            let q = pred("Q", vec![c("a")]);
            assert_eq!(
                probe.atom_value(&q, &model),
                None,
                "an unencoded atom decodes to None"
            );
        }
        SolveResult::Unsat => panic!("P alone is satisfiable"),
    }
}
