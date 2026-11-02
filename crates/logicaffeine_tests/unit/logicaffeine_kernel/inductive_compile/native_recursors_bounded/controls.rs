use super::super::{derive_nested_block_recursors, NestedDecl};
use super::*;

fn limits() -> RecursorBudget {
    RecursorBudget {
        max_depth: crate::type_checker::MAX_TERM_DEPTH,
        max_tree_nodes: 2_000_000,
        max_store_nodes: 2_000_000,
        work: 1_000_000_000_000,
        allocation: 1_000_000_000,
        materialized_nodes: 40_000_000,
    }
}
fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn ty() -> Term {
    Term::Sort(Universe::Type(0))
}
fn pi(n: &str, a: Term, b: Term) -> Term {
    Term::Pi {
        param: n.into(),
        param_type: Box::new(a),
        body_type: Box::new(b),
    }
}
fn app(a: Term, b: Term) -> Term {
    Term::App(Box::new(a), Box::new(b))
}
fn simple() -> (Context, Vec<NestedDecl>) {
    let mut ctx = Context::new();
    ctx.add_mutual_inductives(&[crate::MutualInductive {
        name: "Wrap".into(),
        sort: pi("A", ty(), ty()),
        num_params: 1,
        constructors: vec![(
            "wrap".into(),
            pi(
                "A",
                ty(),
                pi(
                    "x",
                    Term::Var("A".into()),
                    app(g("Wrap"), Term::Var("A".into())),
                ),
            ),
        )],
    }])
    .unwrap();
    let decl = NestedDecl {
        name: "Tree".into(),
        sort: ty(),
        constructors: vec![
            ("leaf".into(), g("Tree")),
            (
                "node".into(),
                pi("children", app(g("Wrap"), g("Tree")), g("Tree")),
            ),
        ],
    };
    ctx.add_mutual_inductives(&[crate::MutualInductive {
        name: decl.name.clone(),
        sort: decl.sort.clone(),
        num_params: 0,
        constructors: decl.constructors.clone(),
    }])
    .unwrap();
    (ctx, vec![decl])
}
fn compare(ctx: &Context, block: &NestedBlock<'_>, expected: usize) {
    let old = derive_nested_block_recursors(ctx, block).expect("ordinary nested derivation");
    let mut budget = limits();
    let before = budget.materialized_nodes;
    let new = derive_nested_block_recursors_bounded(ctx, block, expected, &mut budget)
        .expect("bounded nested derivation");
    assert_eq!(old.len(), expected);
    assert_eq!(new.len(), expected);
    assert!(
        budget.materialized_nodes < before,
        "one shared materialization budget was consumed"
    );
    for (a, b) in old.iter().zip(&new) {
        assert_eq!(a.levels, b.levels, "universe telescope and order");
        assert_eq!(
            a.parameters, b.parameters,
            "canonical uniform parameter telescope"
        );
        assert_eq!(
            a.carrier, b.carrier,
            "source/auxiliary carrier discovery order"
        );
        assert!(crate::defeq_for_test(ctx, &a.ty, &b.ty));
        assert!(crate::defeq_for_test(ctx, &a.body, &b.body));
        let primary = crate::infer_type(ctx, &b.body).expect("primary kernel");
        let secondary = crate::recheck(ctx, &b.body).expect("independent kernel");
        assert!(crate::defeq_for_test(ctx, &primary, &b.ty));
        assert!(crate::defeq_for_test(ctx, &secondary, &b.ty));
    }
}

#[test]
fn checked_single_nested_block_matches_original_and_preserves_local_contexts() {
    crate::term_store::grow_stack_for_depth(crate::type_checker::MAX_TERM_DEPTH, || {
        let (ctx, members) = simple();
        let ctx = ctx.extend("unused.local", ty());
        compare(
            &ctx,
            &NestedBlock {
                members: &members,
                num_params: 0,
                levels: &[],
            },
            2,
        );
    });
}

#[test]
fn incorrect_inventory_and_exhausted_budgets_never_return_partial_recursors() {
    let (ctx, members) = simple();
    let block = NestedBlock {
        members: &members,
        num_params: 0,
        levels: &[],
    };
    assert!(matches!(
        derive_nested_block_recursors_bounded(&ctx, &block, 1, &mut limits()),
        Err(RecursorGenerationError::ResourceLimit)
    ));
    assert!(matches!(
        derive_nested_block_recursors_bounded(&ctx, &block, 3, &mut limits()),
        Err(RecursorGenerationError::Kernel(_))
    ));
    for mode in 0..4 {
        let mut budget = limits();
        match mode {
            0 => budget.work = 0,
            1 => budget.allocation = 0,
            2 => budget.materialized_nodes = 0,
            _ => budget.max_tree_nodes = 1,
        }
        assert!(matches!(
            derive_nested_block_recursors_bounded(&ctx, &block, 2, &mut budget),
            Err(RecursorGenerationError::ResourceLimit)
        ));
    }
}

#[test]
fn registration_requires_exact_source_sort_parameter_count_and_constructor_order() {
    let (ctx, mut members) = simple();
    let block = NestedBlock {
        members: &members,
        num_params: 1,
        levels: &[],
    };
    assert!(matches!(
        derive_nested_block_recursors_bounded(&ctx, &block, 2, &mut limits()),
        Err(RecursorGenerationError::Kernel(_))
    ));
    members[0].constructors.swap(0, 1);
    assert!(matches!(
        derive_nested_block_recursors_bounded(
            &ctx,
            &NestedBlock {
                members: &members,
                num_params: 0,
                levels: &[]
            },
            2,
            &mut limits()
        ),
        Err(RecursorGenerationError::Kernel(_))
    ));
    members[0].constructors.swap(0, 1);
    members[0].sort = Term::Sort(Universe::Type(1));
    assert!(matches!(
        derive_nested_block_recursors_bounded(
            &ctx,
            &NestedBlock {
                members: &members,
                num_params: 0,
                levels: &[]
            },
            2,
            &mut limits()
        ),
        Err(RecursorGenerationError::Kernel(_))
    ));
}

#[test]
fn a_later_sibling_cannot_reset_the_shared_work_allowance() {
    let (ctx, members) = simple();
    let block = NestedBlock {
        members: &members,
        num_params: 0,
        levels: &[],
    };
    let mut first = limits();
    let before = first.work;
    derive_nested_block_recursors_bounded(&ctx, &block, 2, &mut first).unwrap();
    let spent = before - first.work;
    let mut shared = limits();
    shared.work = spent;
    derive_nested_block_recursors_bounded(&ctx, &block, 2, &mut shared).unwrap();
    assert!(matches!(
        derive_nested_block_recursors_bounded(&ctx, &block, 2, &mut shared),
        Err(RecursorGenerationError::ResourceLimit)
    ));
}

#[cfg(feature = "lean")]
mod authentic {
    use super::*;
    use serde_json::Value;
    const ROSE: &str =
        include_str!("../../../../tests/fixtures/lean_semantics/LF_W03_NestedRose.ndjson");
    const ROSE_PROVENANCE: &str =
        include_str!("../../../../tests/fixtures/lean_semantics/LF_W03_NestedRose.provenance.json");
    const POLY: &str = include_str!(
        "../../../../tests/fixtures/lean_semantics/LF_W03_NestedFrontier.UTree.ndjson"
    );
    const POLY_PROVENANCE: &str = include_str!(
        "../../../../tests/fixtures/lean_semantics/LF_W03_NestedFrontier.UTree.provenance.json"
    );
    const MUTUAL: &str = include_str!(
        "../../../../tests/fixtures/lean_semantics/LF_W03_NestedFrontier.MTree.ndjson"
    );
    const MUTUAL_PROVENANCE: &str = include_str!(
        "../../../../tests/fixtures/lean_semantics/LF_W03_NestedFrontier.MTree.provenance.json"
    );
    fn spelling(n: &str) -> String {
        n.split('.').map(|part| format!("/s:{part:?}")).collect()
    }
    fn exercise(export: &'static str, provenance: &'static str, hash: &str, selected: &[&str]) {
        crate::term_store::grow_stack_for_depth(crate::type_checker::MAX_TERM_DEPTH, || {
            let provenance: Value = serde_json::from_str(provenance).unwrap();
            assert_eq!(provenance["export_sha256"], hash);
            assert_eq!(provenance["toolchain"], "leanprover/lean4:v4.14.0");
            assert_eq!(
                (
                    provenance["compiler_exit"].as_i64(),
                    provenance["exporter_exit"].as_i64()
                ),
                (Some(0), Some(0))
            );
            let (ctx, _) =
                crate::interface::lean::import_audited(export).expect("authentic fixture imports");
            // Count recursors from the authentic source block, not from either generator result.
            let mut source_names = std::collections::HashMap::from([(0u64, String::new())]);
            let mut expected = None;
            for line in export.lines() {
                let row: Value = serde_json::from_str(line).unwrap();
                if let Some(index) = row["in"].as_u64() {
                    let part = row.get("str").or_else(|| row.get("num")).unwrap();
                    let prefix = &source_names[&part["pre"].as_u64().unwrap()];
                    let component = part["str"]
                        .as_str()
                        .map(str::to_owned)
                        .unwrap_or_else(|| part["i"].to_string());
                    source_names.insert(
                        index,
                        if prefix.is_empty() {
                            component
                        } else {
                            format!("{prefix}.{component}")
                        },
                    );
                }
                if let Some(block) = row.get("inductive") {
                    let members: Vec<_> = block["types"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|m| source_names[&m["name"].as_u64().unwrap()].as_str())
                        .collect();
                    if members.as_slice() == selected {
                        assert!(expected.is_none());
                        expected = Some(block["recs"].as_array().unwrap().len());
                    }
                }
            }
            let members: Vec<_> = selected
                .iter()
                .map(|name| {
                    let name = spelling(name);
                    NestedDecl {
                        name: name.clone(),
                        sort: ctx.get_global(&name).unwrap().clone(),
                        constructors: ctx
                            .constructor_headers(&name)
                            .map(|(name, ty)| (name.to_owned(), ty.clone()))
                            .collect(),
                    }
                })
                .collect();
            let parameters = ctx.inductive_num_params(&members[0].name);
            let levels = ctx
                .poly_declaration(&members[0].name)
                .map(|(levels, _)| levels.to_vec())
                .unwrap_or_default();
            compare(
                &ctx,
                &NestedBlock {
                    members: &members,
                    num_params: parameters,
                    levels: &levels,
                },
                expected.expect("selected authentic block"),
            );
        });
    }
    #[test]
    fn authentic_single_nested_source_recursors_match_original() {
        exercise(
            ROSE,
            ROSE_PROVENANCE,
            "119c8c198f56a0d7ccbfa6fb46babb12b957da8efd9682e20d757b48a8133049",
            &["LF_W03R.Rose"],
        );
    }
    #[test]
    fn authentic_universe_polymorphic_nested_source_recursors_match_original() {
        exercise(
            POLY,
            POLY_PROVENANCE,
            "88dfd62f4508dd29d89745ff3ad6f2dc15ed9b1ad192e67ae016158c2efaa43e",
            &["LF_W03N.UTree"],
        );
    }
    #[test]
    fn authentic_mutual_nested_source_recursors_match_original() {
        exercise(
            MUTUAL,
            MUTUAL_PROVENANCE,
            "89ef5709a003f77fcb1106e6713853eaab0ffee65135f52955f17b727b31f787",
            &["LF_W03N.MTree", "LF_W03N.MForest"],
        );
    }
}
