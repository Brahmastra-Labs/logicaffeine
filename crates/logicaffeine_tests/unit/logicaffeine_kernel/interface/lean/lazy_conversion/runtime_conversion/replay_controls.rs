//! Adversarial producer-state mutations: replay must recover original atoms and
//! independently validate native links, rather than trust their cached endpoints.
use super::*;
use serde_json::json;

fn g(name: &str) -> Term {
    Term::Global(name.into())
}
fn v(name: &str) -> Term {
    Term::Var(name.into())
}
fn universe() -> Term {
    Term::Sort(Universe::Type(0))
}
fn arrow(name: &str, domain: Term, body: Term) -> Term {
    Term::Pi {
        param: name.into(),
        param_type: Box::new(domain),
        body_type: Box::new(body),
    }
}
fn lambda(name: &str, domain: Term, body: Term) -> Term {
    Term::Lambda {
        param: name.into(),
        param_type: Box::new(domain),
        body: Box::new(body),
    }
}
fn apply(function: Term, argument: Term) -> Term {
    Term::App(Box::new(function), Box::new(argument))
}
fn fixture() -> Importer<'static> {
    let mut importer = Importer::new(false);
    importer.ctx.add_declaration("A", universe());
    importer.ctx.add_declaration("B", universe());
    let function = arrow("z", universe(), universe());
    importer
        .ctx
        .declare(
            "identity",
            function.clone(),
            lambda("z", universe(), v("z")),
        )
        .unwrap();
    importer
        .ctx
        .declare(
            "wrapper",
            arrow("f", function.clone(), arrow("x", universe(), universe())),
            lambda(
                "f",
                function,
                lambda("x", universe(), apply(v("f"), v("x"))),
            ),
        )
        .unwrap();
    for (id, name) in [(1, "A"), (2, "B"), (3, "identity"), (4, "wrapper")] {
        importer.names.insert(id, name.into());
        importer.declarations.insert(name.into(), 0);
    }
    importer.levels.insert(0, Universe::Type(0));
    importer.level_vars.insert(0, HashSet::new());
    for (id, row) in [
        (0, json!({"ie":0,"sort":0})),
        (1, json!({"ie":1,"const":{"name":1,"us":[]}})),
        (2, json!({"ie":2,"const":{"name":2,"us":[]}})),
        (3, json!({"ie":3,"const":{"name":3,"us":[]}})),
        (4, json!({"ie":4,"const":{"name":4,"us":[]}})),
        (5, json!({"ie":5,"bvar":0})),
        (
            6,
            json!({"ie":6,"lam":{"name":0,"type":0,"body":5,"binderInfo":"default"}}),
        ),
        (7, json!({"ie":7,"app":{"fn":3,"arg":1}})),
        (8, json!({"ie":8,"app":{"fn":4,"arg":6}})),
        (9, json!({"ie":9,"app":{"fn":8,"arg":1}})),
    ] {
        importer.exprs.insert(id, row);
    }
    importer
}

#[test]
fn replay_reconstructs_source_atoms_instead_of_trusting_the_producer_atom_cache() {
    let importer = fixture();
    let mut budget = 1_000_000;
    let typed = TypedSource::check(&importer, 1, &HashSet::new(), &mut budget).unwrap();
    let mut search = Search::new(&typed, &mut budget).unwrap();
    let root = Goal::Eval(search.pool.captured(Syntax::Source(1), 0));
    let proof = search.eval(root, &mut budget).unwrap();
    let correct = search.replay(root, proof, &mut budget).unwrap();

    // A and B have precisely the same type, but distinct opaque meanings.
    let other = search.pool.atom(2, &mut budget).unwrap();
    let other_value = search
        .value(RuntimeValue::Ground(other), &mut budget)
        .unwrap();
    assert_ne!(correct, RuntimeValue::Ground(other));
    search.pool.atoms.insert(1, other);
    search.proofs[proof].rule = Rule::Atom(other);
    search.proofs[proof].value = other_value;
    assert!(
        search.replay(root, proof, &mut budget).is_err(),
        "a forged source-to-native cache entry cannot change original A into B"
    );
    assert!(
        budget > 100_000,
        "this is a provenance failure, not budget exhaustion"
    );
}

#[test]
fn replay_rechecks_a_poisoned_native_ground_endpoint() {
    let importer = fixture();
    let mut budget = 1_000_000;
    let typed = TypedSource::check(&importer, 7, &HashSet::new(), &mut budget).unwrap();
    let mut search = Search::new(&typed, &mut budget).unwrap();
    let function = search.pool.atom(3, &mut budget).unwrap();
    let argument = search.pool.atom(1, &mut budget).unwrap();
    let input = search.pool.app(function, argument, &mut budget).unwrap();
    let link = search.pool.ground(input, true, &mut budget).unwrap();
    let other = search.pool.atom(2, &mut budget).unwrap();
    assert_ne!(search.pool.links[link].to, other);
    // Keep the exact original call and link kind. Only the claimed result lies.
    // Poison before evaluation so the producer emits internally consistent
    // dependent claims; replay must independently reject this whole forgery.
    search.pool.links[link].to = other;
    let root = Goal::Eval(search.pool.captured(Syntax::Source(7), 0));
    let proof = search.eval(root, &mut budget).unwrap();
    assert_eq!(
        search.values[search.proofs[proof].value],
        RuntimeValue::Ground(other)
    );
    assert!(
        search.replay(root, proof, &mut budget).is_err(),
        "a producer link cannot certify identity(A) = B merely by keeping the input"
    );
    assert!(
        budget > 100_000,
        "small forged conversion must not consume the test allowance"
    );
}

#[test]
fn replay_rechecks_a_poisoned_native_definition_body() {
    let importer = fixture();
    let mut budget = 1_000_000;
    let typed = TypedSource::check(&importer, 9, &HashSet::new(), &mut budget).unwrap();
    let mut search = Search::new(&typed, &mut budget).unwrap();
    let head = search.pool.atom(4, &mut budget).unwrap();
    let link = search.pool.expose(head, &mut budget).unwrap();
    let function = arrow("z", universe(), universe());
    let wrong_body = lambda("f", function, lambda("x", universe(), g("B")));
    let other = search.pool.intern(&wrong_body, &mut budget).unwrap();
    assert_ne!(search.pool.links[link].to, other);
    // Same declared function type, different checked-definition body.
    search.pool.links[link].to = other;
    let root = Goal::Eval(search.pool.captured(Syntax::Source(9), 0));
    let proof = search.eval(root, &mut budget).unwrap();
    assert!(
        search.replay(root, proof, &mut budget).is_err(),
        "replay must tie wrapper to its actual context body, not a producer replacement"
    );
    assert!(
        budget > 100_000,
        "this tiny trace must leave a generous allowance"
    );
}
