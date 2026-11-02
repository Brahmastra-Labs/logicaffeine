use super::*;
fn g(n: &str) -> Term {
    Term::Global(n.into())
}

#[test]
fn long_proof_chain_shares_dependency_nodes_and_invalidates_linearly() {
    let mut c = Context::new();
    c.set_auto_auxiliaries(false);
    c.add_inductive("P", Term::Sort(Universe::Prop));
    c.add_constructor("p", "P", g("P"));
    let definitions = 64usize;
    let proofs = 128usize;
    let mut previous = "p".to_string();
    for i in 0..definitions {
        let name = format!("helper{i}");
        c.with_equations_suspended(|c| c.declare(&name, g("P"), g(&previous)))
            .unwrap();
        previous = name;
    }
    for i in 0..proofs {
        let name = format!("proof{i}");
        c.declare_opaque_checked(&name, vec![], g("P"), g(&previous))
            .unwrap();
        previous = name;
    }
    assert_eq!(c.checked_opaque.len(), proofs);
    assert_eq!(c.checked_opaque_graph.len(), definitions + proofs + 2);
    let edges: usize = c.checked_opaque_graph.values().map(|e| e.len()).sum();
    assert!(
        edges <= 2 * (definitions + proofs) + 4,
        "stored edges={edges}"
    );
    let reverse: usize = c.checked_opaque_dependents.values().map(|e| e.len()).sum();
    assert_eq!(reverse, edges);
    let untouched = c.clone();
    assert!(
        c.refused_meaning_change_for_test(|c| {
            c.add_declaration("helper0", g("P"));
            Ok(())
        })
        .is_some(),
        "the stable-meaning gate keeps `helper0` under `helper1`"
    );
    c.permit_meaning_changes_for_cache_tests();
    OPAQUE_INVALIDATION_VISITS.with(|count| count.set(0));
    c.add_declaration("helper0", g("P"));
    let visits = OPAQUE_INVALIDATION_VISITS.with(std::cell::Cell::get);
    assert!(
        visits <= 2 * (definitions + proofs) + 2,
        "actual invalidation pops={visits}"
    );
    assert!(c.checked_opaque.is_empty());
    assert_eq!(c.checked_opaque_graph.len(), 2);
    assert_eq!(untouched.checked_opaque.len(), proofs);
    assert!(crate::axiom_dependencies_of(&untouched, &previous)
        .unwrap()
        .axioms
        .is_empty());
    assert_eq!(
        crate::axiom_dependencies_of(&c, &previous).unwrap().axioms,
        std::collections::BTreeSet::from([previous])
    );
}
