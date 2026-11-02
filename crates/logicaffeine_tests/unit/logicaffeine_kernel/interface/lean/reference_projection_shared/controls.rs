use super::*;
fn budget() -> RecursorBudget {
    RecursorBudget {
        max_depth: 32,
        max_tree_nodes: 2_000_000,
        max_store_nodes: 2_000_000,
        work: 1_000_000_000_000,
        allocation: 1_000_000_000,
        materialized_nodes: 20_000_000,
    }
}
fn g(name: &str) -> Term {
    Term::Global(name.into())
}
fn simple() -> (Context, Term) {
    let mut ctx = Context::new();
    ctx.add_inductive("ProjectionPebble", Term::Sort(Universe::Type(0)));
    ctx.add_constructor(
        "projectionPebble",
        "ProjectionPebble",
        g("ProjectionPebble"),
    );
    ctx.add_inductive("ProjectionBox", Term::Sort(Universe::Type(0)));
    ctx.add_constructor(
        "projectionBox",
        "ProjectionBox",
        Term::Pi {
            param: "value".into(),
            param_type: Box::new(g("ProjectionPebble")),
            body_type: Box::new(g("ProjectionBox")),
        },
    );
    (
        ctx,
        Term::App(
            Box::new(g("projectionBox")),
            Box::new(g("projectionPebble")),
        ),
    )
}

#[test]
fn unchanged_generated_tree_refusal_assertions_cover_shared_construction() {
    let mut limits = budget();
    let mut allowance = 2;
    let mut dag = Dag {
        d: Construction::new(&mut limits).unwrap(),
        allocation: &mut allowance,
        fresh: 0,
    };
    let a = dag.var("a").unwrap();
    let before = dag.d.store.len();
    assert!(dag.app(a, a).is_err());
    assert_eq!(dag.d.store.len(), before);
}

#[test]
fn unchanged_duplicated_substitution_refusal_assertions_cover_shared_construction() {
    let mut limits = budget();
    limits.max_store_nodes = 100;
    let mut allowance = 8;
    let mut dag = Dag {
        d: Construction::new(&mut limits).unwrap(),
        allocation: &mut allowance,
        fresh: 0,
    };
    let x = dag.var("x").unwrap();
    let pair = dag.app(x, x).unwrap();
    let v = dag.var("v").unwrap();
    let twice = dag.app(v, v).unwrap();
    let four = dag.app(twice, twice).unwrap();
    let before = *dag.allocation;
    assert!(dag.substitute(pair, "x", four).is_err());
    assert_eq!(*dag.allocation, before, "no tree was extracted");
    assert!(dag.d.store.len() <= dag.d.budget.max_store_nodes);
}

#[test]
fn allocation_exhaustion_precedes_name_interning_or_new_dag_node() {
    let mut limits = budget();
    let mut allowance = 100;
    let mut dag = Dag {
        d: Construction::new(&mut limits).unwrap(),
        allocation: &mut allowance,
        fresh: 0,
    };
    dag.d.budget.allocation = 0;
    let before = dag.d.store.len();
    assert!(dag.var("previously-unseen-name").is_err());
    assert_eq!(dag.d.store.len(), before);
    assert!(dag
        .d
        .store
        .lookup_symbol("previously-unseen-name")
        .is_none());
}

#[test]
fn shared_provider_preserves_ordinary_projection_semantics_and_both_tree_meters() {
    let (ctx, value) = simple();
    let mut old_allowance = 200_000;
    let old = super::super::reference_projection::ProjectionReader {
        context: &ctx,
        max_depth: 32,
    }
    .project("ProjectionBox", 0, value.clone(), &[], &mut old_allowance)
    .unwrap();
    let mut limits = budget();
    let before = limits.materialized_nodes;
    let mut allowance = 200_000;
    let new = SharedProjectionReader {
        context: &ctx,
        max_depth: 32,
        budget: &mut limits,
    }
    .project("ProjectionBox", 0, value, &[], &mut allowance)
    .unwrap();
    assert!(crate::defeq_for_test(&ctx, &old, &new));
    assert!(crate::recheck(&ctx, &new).is_ok());
    assert_eq!(
        before - limits.materialized_nodes,
        (200_000 - allowance) as u64
    );
    assert!(limits.work < 1_000_000_000_000 && limits.allocation < 1_000_000_000);
}

#[test]
fn repeated_project_calls_and_refusals_never_replenish_the_caller_budget() {
    let (ctx, value) = simple();
    let mut limits = budget();
    let mut allowance = 200_000;
    {
        let mut reader = SharedProjectionReader {
            context: &ctx,
            max_depth: 32,
            budget: &mut limits,
        };
        reader
            .project("ProjectionBox", 0, value.clone(), &[], &mut allowance)
            .unwrap();
        let first = (
            reader.budget.work,
            reader.budget.allocation,
            reader.budget.materialized_nodes,
        );
        reader
            .project("ProjectionBox", 0, value.clone(), &[], &mut allowance)
            .unwrap();
        assert!(
            reader.budget.work < first.0
                && reader.budget.allocation < first.1
                && reader.budget.materialized_nodes < first.2
        );
        reader.budget.work = 0;
        assert!(reader
            .project("ProjectionBox", 0, value.clone(), &[], &mut allowance)
            .is_err());
        assert_eq!(reader.budget.work, 0);
        assert!(reader
            .project("ProjectionBox", 0, value, &[], &mut allowance)
            .is_err());
        assert_eq!(reader.budget.work, 0);
    }
    assert_eq!(limits.work, 0);
}
