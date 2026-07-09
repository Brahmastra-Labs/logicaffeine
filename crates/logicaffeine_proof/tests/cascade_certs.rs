//! Every new benchmark family emits a REAL, re-checkable proof certificate — never a bare verdict. The
//! parity-cardinality (fused) and modular-counting families are certified by the SR certifier
//! (`certified_unsat_auto`); its steps re-check from scratch (`check_pr_refutation`) and serialize to a
//! byte size (`emit_sr`), exactly as the pigeonhole SR proof does. (The ordering family carries its own
//! structural certificate — see `cascade_ordering`; clique-colouring a counting certificate.) This is
//! the "certified, not trusted" bar the benchmark page rests on.

use logicaffeine_proof::families;
use logicaffeine_proof::pr::check_pr_refutation;
use logicaffeine_proof::proof_emit::emit_sr;
use logicaffeine_proof::sym_certify::certified_unsat_auto;

#[test]
fn parity_cardinality_and_count_q_emit_recheckable_certified_proofs() {
    let cases: Vec<(&str, _)> = vec![
        ("parity_cardinality(20)", families::parity_exactly_one(20).0),
        ("parity_cardinality(40)", families::parity_exactly_one(40).0),
        ("parity_cardinality(60)", families::parity_exactly_one(60).0),
        ("count_q(4,3)", families::mod_counting(4, 3).0),
        ("count_q(7,3)", families::mod_counting(7, 3).0),
        ("count_q(8,3)", families::mod_counting(8, 3).0),
    ];
    for (name, cnf) in cases {
        let r = certified_unsat_auto(cnf.num_vars, &cnf.clauses);
        assert!(r.refuted, "{name}: the family must be certified UNSAT");
        assert!(
            check_pr_refutation(cnf.num_vars, &cnf.clauses, &r.steps),
            "{name}: the certificate must re-check from scratch against the original clauses"
        );
        let sr = emit_sr(cnf.num_vars, &cnf.clauses, &r.steps).expect("the certificate serializes to SR text");
        assert!(!sr.is_empty(), "{name}: the serialized certificate has a positive byte size");
    }
}
