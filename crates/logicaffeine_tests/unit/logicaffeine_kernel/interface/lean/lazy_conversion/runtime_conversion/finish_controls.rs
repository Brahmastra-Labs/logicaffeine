//! Final readback must not reopen producer-cache authority after genuine replay.
use super::*;
use serde_json::json;

fn fixture() -> Importer<'static> {
    let mut importer = Importer::new(false);
    for (name_id, source_id, name) in [(1, 0, "A"), (2, 1, "B")] {
        importer
            .ctx
            .add_declaration(name, Term::Sort(Universe::Type(0)));
        importer.names.insert(name_id, name.into());
        importer.declarations.insert(name.into(), 0);
        importer.exprs.insert(
            source_id,
            json!({"ie":source_id,"const":{"name":name_id,"us":[]}}),
        );
    }
    importer
}

#[test]
fn an_unpoisoned_replayed_ground_endpoint_finishes_successfully() {
    let importer = fixture();
    let mut budget = 1_000_000;
    let typed = TypedSource::check(&importer, 0, &HashSet::new(), &mut budget).unwrap();
    let mut search = Search::new(&typed, &mut budget).unwrap();
    let root = Goal::Eval(search.pool.captured(Syntax::Source(0), 0));
    let proof = search.eval(root, &mut budget).unwrap();
    let RuntimeValue::Ground(id) = search.replay(root, proof, &mut budget).unwrap() else {
        panic!("a closed source constant must replay to a ground endpoint");
    };
    let result = search.finish(id, &mut budget).unwrap();
    assert!(definitionally_equal(
        &importer.ctx,
        &result,
        &Term::Global("A".into())
    ));
    assert!(!definitionally_equal(
        &importer.ctx,
        &result,
        &Term::Global("B".into())
    ));
    assert!(budget > 100_000);
}

#[test]
fn an_unused_poisoned_ground_cache_cannot_change_a_genuinely_replayed_endpoint() {
    let importer = fixture();
    let mut budget = 1_000_000;
    let typed = TypedSource::check(&importer, 0, &HashSet::new(), &mut budget).unwrap();
    let mut search = Search::new(&typed, &mut budget).unwrap();
    let root = Goal::Eval(search.pool.captured(Syntax::Source(0), 0));
    let proof = search.eval(root, &mut budget).unwrap();
    let RuntimeValue::Ground(id) = search.replay(root, proof, &mut budget).unwrap() else {
        panic!("the original A must genuinely replay before the final cache is poisoned");
    };
    assert!(
        !search.pool.grounds.contains_key(&id),
        "the attack must use a link that original-source replay never consulted"
    );
    let link = search.pool.ground(id, false, &mut budget).unwrap();
    let other = search.pool.atom(1, &mut budget).unwrap();
    assert_eq!(search.pool.links[link].from, id);
    assert_ne!(search.pool.links[link].to, other);
    search.pool.links[link].to = other;
    assert!(
        search.finish(id, &mut budget).is_err(),
        "finish must independently reject a producer cache claiming original A becomes B"
    );
    assert!(
        budget > 100_000,
        "failure must not be caused by resource exhaustion"
    );
}
