use super::*;
fn budget() -> RecursorBudget {
    RecursorBudget {
        max_depth: crate::type_checker::MAX_TERM_DEPTH,
        max_tree_nodes: 1_000_000,
        max_store_nodes: 1_000_000,
        work: 1_000_000_000_000,
        allocation: 1_000_000_000,
        materialized_nodes: 10_000_000,
    }
}
#[test]
fn bounded_nat_rules_match_the_existing_checked_computation_rules() {
    let mut ctx = Context::new();
    crate::prelude::StandardLibrary::register(&mut ctx);
    let (levels, ty, body) = crate::derive_recursor_universe_poly(&ctx, "Nat").unwrap();
    let name = "bounded.test.Nat.rec";
    ctx.declare_universe_poly(name, levels.clone(), ty.clone(), body)
        .unwrap();
    for constructor in ["Zero", "Succ"] {
        let value = Term::Global(constructor.into());
        let ctor_ty = ctx.get_global(constructor).unwrap();
        let expected = super::super::ctor_rule_applied(
            &ctx,
            name,
            &levels,
            &ty,
            value.clone(),
            ctor_ty.clone(),
            0,
            3,
            0,
        )
        .unwrap();
        let actual = ctor_rule_applied_bounded(
            &ctx,
            name,
            &levels,
            &ty,
            &value,
            ctor_ty,
            0,
            3,
            0,
            &mut budget(),
        )
        .unwrap();
        assert!(crate::defeq_for_test(&ctx, &actual, &expected));
        assert!(crate::recheck(&ctx, &actual).is_ok());
    }
}
#[test]
fn authentic_structure_bodies_preserve_checked_projection_computation() {
    let source =
        include_str!("../../../../../tests/fixtures/lean_semantics/LF_StructEtaIota.ndjson");
    let (ctx, _) = super::super::import(source).unwrap();
    for family in ["/s:\"LF_SE\"/s:\"P2\"", "/s:\"Prod\""] {
        let (_, ty, _) = crate::derive_recursor_universe_poly(&ctx, family).unwrap();
        let structure = ctx.struct_info(family).expect("checked structure metadata");
        let parameters = ctx.inductive_num_params(family);
        let expected = super::super::structure_recursor_body(
            &ctx,
            &ty,
            parameters,
            parameters + 1,
            parameters + 3,
            &structure.projections,
        )
        .unwrap();
        let actual = structure_recursor_body_bounded(
            &ctx,
            &ty,
            parameters,
            parameters + 1,
            parameters + 3,
            &structure.projections,
            &mut budget(),
        )
        .unwrap();
        assert!(crate::defeq_for_test(&ctx, &actual, &expected));
        let actual_type = super::super::lean_type(&ctx, &actual).unwrap();
        super::super::exact(&ctx, &actual_type, &ty).unwrap();
        assert!(crate::recheck(&ctx, &actual).is_ok());
    }
}
#[test]
fn source_rule_construction_refuses_exhausted_allocation_before_returning_a_term() {
    let mut limits = budget();
    limits.allocation = 0;
    let error = ctor_rule_applied_bounded(
        &Context::new(),
        "r",
        &[],
        &Term::Hole,
        &Term::Hole,
        &Term::Hole,
        0,
        0,
        0,
        &mut limits,
    )
    .unwrap_err();
    assert_eq!(error.reason().code(), DiagnosticCode::ResourceLimit);
}
#[test]
fn overflowing_structure_telescope_metadata_is_rejected_without_arithmetic_panic() {
    let error = structure_recursor_body_bounded(
        &Context::new(),
        &Term::Hole,
        0,
        usize::MAX,
        usize::MAX,
        &[],
        &mut budget(),
    )
    .unwrap_err();
    assert_eq!(error.reason().code(), DiagnosticCode::InvalidProof);
}
