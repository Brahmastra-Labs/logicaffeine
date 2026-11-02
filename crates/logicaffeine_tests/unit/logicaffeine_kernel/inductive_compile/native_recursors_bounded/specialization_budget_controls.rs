use super::super::{native_specialization, Elimination, MutualInductive};
use super::*;

fn input() -> (Context, Elimination) {
    let member = MutualInductive {
        name: "Source".into(),
        sort: Term::Sort(Universe::Type(0)),
        num_params: 0,
        constructors: vec![("sourceMk".into(), Term::Global("Source".into()))],
    };
    let mut ctx = Context::new();
    ctx.add_mutual_inductives(std::slice::from_ref(&member))
        .unwrap();
    (
        ctx,
        Elimination {
            block: vec![member],
            restoration: vec![],
            parameters: vec![],
        },
    )
}
fn budget() -> RecursorBudget {
    RecursorBudget {
        max_depth: 64,
        max_tree_nodes: 10_000,
        max_store_nodes: 10_000,
        work: 10_000_000,
        allocation: 10_000_000,
        materialized_nodes: 10_000,
    }
}

#[test]
fn bounded_and_ordinary_private_preparation_choose_identical_source_images_and_names() {
    let (ctx, input) = input();
    let ordinary =
        native_specialization::prepare(&ctx, input, &[], &mut native_specialization::Ordinary)
            .unwrap();
    let (ctx, input) = self::input();
    let mut budget = budget();
    let bounded =
        native_specialization::prepare(&ctx, input, &[], &mut SpecializationBudget(&mut budget))
            .unwrap();
    assert_eq!(ordinary.restoration, bounded.restoration);
    assert_eq!(ordinary.parameters, bounded.parameters);
    assert_eq!(ordinary.block.len(), bounded.block.len());
    for (old, new) in ordinary.block.iter().zip(&bounded.block) {
        assert_eq!(old.name, new.name);
        assert_eq!(old.sort, new.sort);
        assert_eq!(old.num_params, new.num_params);
        assert_eq!(old.constructors, new.constructors);
    }
    assert!(
        budget.materialized_nodes < 10_000,
        "new original-source images consume shared allowance"
    );
}

#[test]
fn private_preparation_refuses_exhausted_shared_allowances_without_changing_source() {
    for kind in 0..3 {
        let (ctx, input) = input();
        let source = ctx.get_global("Source").unwrap().clone();
        let mut budget = budget();
        match kind {
            0 => budget.work = 0,
            1 => budget.allocation = 0,
            _ => budget.materialized_nodes = 0,
        }
        let result = native_specialization::prepare(
            &ctx,
            input,
            &[],
            &mut SpecializationBudget(&mut budget),
        );
        assert!(matches!(
            result,
            Err(RecursorGenerationError::ResourceLimit)
        ));
        assert_eq!(ctx.get_global("Source"), Some(&source));
        assert!(ctx.get_global("arena.native.private.0.I0").is_none());
    }
}
