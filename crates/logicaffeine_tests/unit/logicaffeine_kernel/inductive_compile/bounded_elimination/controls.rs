use super::*;
use crate::Universe;

fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn v(n: &str) -> Term {
    Term::Var(n.into())
}
fn sort() -> Term {
    Term::Sort(Universe::Type(0))
}
fn app(f: Term, a: Term) -> Term {
    Term::App(Box::new(f), Box::new(a))
}
fn pi(n: &str, t: Term, b: Term) -> Term {
    Term::Pi {
        param: n.into(),
        param_type: Box::new(t),
        body_type: Box::new(b),
    }
}
fn budget() -> RecursorBudget {
    RecursorBudget {
        max_depth: crate::type_checker::MAX_TERM_DEPTH,
        max_tree_nodes: 2_000_000,
        max_store_nodes: 2_000_000,
        work: 1_000_000_000_000,
        allocation: 1_000_000_000,
        materialized_nodes: 20_000_000,
    }
}
fn context() -> Context {
    let mut ctx = Context::new();
    ctx.add_inductive("Wrap", pi("A", sort(), sort()));
    ctx.add_constructor(
        "wrap",
        "Wrap",
        pi("A", sort(), pi("value", v("A"), app(g("Wrap"), v("A")))),
    );
    assert!(ctx.is_inductive("Wrap"));
    assert_eq!(ctx.get_constructors("Wrap").len(), 1);
    ctx
}
fn nested(depth: usize) -> NestedDecl {
    let mut field = g("Tree");
    for _ in 0..depth {
        field = app(g("Wrap"), field);
    }
    NestedDecl {
        name: "Tree".into(),
        sort: sort(),
        constructors: vec![("node".into(), pi("children", field, g("Tree")))],
    }
}
fn compare(ctx: &Context, members: &[NestedDecl], np: usize, levels: &[String]) -> Elimination {
    let block = NestedBlock {
        members,
        num_params: np,
        levels,
    };
    let old = super::super::eliminate_nested(ctx, &block).unwrap();
    let new = eliminate_nested_bounded(ctx, &block, old.block.len(), &mut budget()).unwrap();
    assert_eq!(old.parameters, new.parameters);
    assert_eq!(old.restoration, new.restoration);
    assert_eq!(old.block.len(), new.block.len());
    for (old, new) in old.block.iter().zip(&new.block) {
        assert_eq!(old.name, new.name);
        assert_eq!(old.num_params, new.num_params);
        assert_eq!(old.sort, new.sort);
        assert_eq!(old.constructors, new.constructors);
    }
    new
}

#[test]
fn source_first_then_outer_to_inner_auxiliary_discovery_matches_original() {
    let ctx = context();
    let result = compare(&ctx, &[nested(2)], 0, &[]);
    assert_eq!(
        result
            .block
            .iter()
            .map(|m| m.name.as_str())
            .collect::<Vec<_>>(),
        ["Tree", "Tree$nested1", "Tree$nested2"]
    );
    assert_eq!(
        result.restoration[1].1,
        app(g("Wrap"), app(g("Wrap"), g("Tree")))
    );
    assert_eq!(result.restoration[3].1, app(g("Wrap"), g("Tree")));
}

#[test]
fn repeated_identical_instances_share_one_auxiliary_and_preserve_constructor_order() {
    let ctx = context();
    let mut decl = nested(1);
    decl.constructors
        .push(("nodeAgain".into(), decl.constructors[0].1.clone()));
    let result = compare(&ctx, &[decl], 0, &[]);
    assert_eq!(result.block.len(), 2);
    assert_eq!(result.block[0].constructors[1].0, "nodeAgain");
}

#[test]
fn uniform_parameters_are_canonicalized_before_auxiliary_rewriting() {
    let ctx = context();
    let tree = app(g("PTree"), v("A"));
    let decl = NestedDecl {
        name: "PTree".into(),
        sort: pi("A", sort(), sort()),
        constructors: vec![(
            "pnode".into(),
            pi(
                "A",
                sort(),
                pi("children", app(g("Wrap"), tree.clone()), tree),
            ),
        )],
    };
    let result = compare(&ctx, &[decl], 1, &[]);
    assert_eq!(result.parameters[0].0, "nested.parameter.0");
    assert_eq!(result.block[1].num_params, 1);
}

#[test]
fn mutual_members_precede_auxiliaries_even_when_first_member_discovers_sibling_nesting() {
    let ctx = context();
    let left = NestedDecl {
        name: "Left".into(),
        sort: sort(),
        constructors: vec![(
            "left".into(),
            pi("xs", app(g("Wrap"), g("Right")), g("Left")),
        )],
    };
    let right = NestedDecl {
        name: "Right".into(),
        sort: sort(),
        constructors: vec![(
            "right".into(),
            pi("xs", app(g("Wrap"), g("Left")), g("Right")),
        )],
    };
    let result = compare(&ctx, &[left, right], 0, &[]);
    assert_eq!(
        result
            .block
            .iter()
            .map(|m| m.name.as_str())
            .collect::<Vec<_>>(),
        ["Left", "Right", "Left$nested1", "Left$nested2"]
    );
}

fn eliminator<'a, 'b>(
    ctx: &'a Context,
    budget: &'b mut RecursorBudget,
    limit: usize,
) -> Eliminator<'a, 'b> {
    let mut d = Construction::new(budget).unwrap();
    let reference_model = d.intern(&g("Tree")).unwrap();
    Eliminator {
        ctx,
        d,
        params: vec![],
        reference_model,
        members: vec!["Tree".into()],
        base: "Tree".into(),
        auxiliaries: vec![],
        auxiliary_limit: limit,
    }
}

#[test]
fn exhausted_auxiliary_inventory_precedes_name_arguments_or_constructor_allocation() {
    let ctx = context();
    let mut limits = budget();
    let mut e = eliminator(&ctx, &mut limits, 1);
    let wrap = e.d.intern(&g("Wrap")).unwrap();
    let tree = e.d.intern(&g("Tree")).unwrap();
    let nested = e.d.app(wrap, tree).unwrap();
    assert_eq!(e.auxiliary(wrap, &[tree]).unwrap(), 0);
    // A deduplicated request still succeeds at capacity.
    assert_eq!(e.auxiliary(wrap, &[tree]).unwrap(), 0);
    let before = (
        e.d.store.len(),
        e.members.len(),
        e.auxiliaries.len(),
        e.d.budget.allocation,
        e.d.budget.materialized_nodes,
    );
    assert!(matches!(
        e.auxiliary(wrap, &[nested]),
        Err(RecursorGenerationError::ResourceLimit)
    ));
    assert_eq!(
        before,
        (
            e.d.store.len(),
            e.members.len(),
            e.auxiliaries.len(),
            e.d.budget.allocation,
            e.d.budget.materialized_nodes
        )
    );
}

#[test]
fn expected_inventory_is_an_exact_caller_supplied_bound() {
    let ctx = context();
    let members = [nested(2)];
    let block = NestedBlock {
        members: &members,
        num_params: 0,
        levels: &[],
    };
    assert!(matches!(
        eliminate_nested_bounded(&ctx, &block, 2, &mut budget()),
        Err(RecursorGenerationError::ResourceLimit)
    ));
    assert!(eliminate_nested_bounded(&ctx, &block, 3, &mut budget()).is_ok());
    assert!(matches!(
        eliminate_nested_bounded(&ctx, &block, 4, &mut budget()),
        Err(RecursorGenerationError::Kernel(_))
    ));
}

#[test]
fn generated_pi_is_refused_before_a_deeper_dag_node_or_tree_is_allocated() {
    let mut limits = budget();
    limits.max_depth = 4;
    let mut d = Construction::new(&mut limits).unwrap();
    let ty = d.intern(&sort()).unwrap();
    let name = d.symbol("x").unwrap();
    let params = vec![(name, ty); 3];
    let admitted = close(&mut d, &params, ty).unwrap();
    let before = (d.store.len(), d.budget.materialized_nodes);
    assert!(matches!(
        close(&mut d, &[(name, ty)], admitted),
        Err(RecursorGenerationError::ResourceLimit)
    ));
    assert_eq!(before, (d.store.len(), d.budget.materialized_nodes));
}

#[test]
fn explicit_container_instances_are_part_of_auxiliary_identity() {
    let ctx = context();
    let mut limits = budget();
    let mut e = eliminator(&ctx, &mut limits, 2);
    let tree = e.d.intern(&g("Tree")).unwrap();
    let a =
        e.d.intern(&Term::Const {
            name: "PolyWrap".into(),
            levels: vec![Universe::Type(0)],
        })
        .unwrap();
    let b =
        e.d.intern(&Term::Const {
            name: "PolyWrap".into(),
            levels: vec![Universe::Type(1)],
        })
        .unwrap();
    assert_eq!(e.auxiliary(a, &[tree]).unwrap(), 0);
    assert_eq!(e.auxiliary(b, &[tree]).unwrap(), 1);
    let ctor = e.reference_like("polyWrap", b).unwrap();
    assert_eq!(
        e.d.extract(ctor).unwrap(),
        Term::Const {
            name: "polyWrap".into(),
            levels: vec![Universe::Type(1)]
        }
    );
}

#[test]
fn local_mentions_ignore_shadowing_and_block_mentions_cover_fix_and_match() {
    let ctx = context();
    let mut limits = budget();
    let mut e = eliminator(&ctx, &mut limits, 3);
    let local = e.d.symbol("local").unwrap();
    let term = e.d.intern(&pi("local", sort(), v("local"))).unwrap();
    assert!(e.mentions(term, Some(local)).unwrap());
    let term =
        e.d.intern(&Term::Fix {
            name: "self".into(),
            body: Box::new(g("Tree")),
        })
        .unwrap();
    assert!(e.mentions(term, None).unwrap());
    // Original replace does not descend into Fix bodies, despite mentions scanning them.
    assert_eq!(e.replace(term).unwrap(), term);
}

#[test]
fn work_and_allocation_are_cumulative_across_eliminations() {
    let ctx = context();
    let members = [nested(1)];
    let block = NestedBlock {
        members: &members,
        num_params: 0,
        levels: &[],
    };
    let mut limits = budget();
    let before = (limits.work, limits.allocation, limits.materialized_nodes);
    eliminate_nested_bounded(&ctx, &block, 2, &mut limits).unwrap();
    assert!(
        limits.work < before.0
            && limits.allocation < before.1
            && limits.materialized_nodes < before.2
    );
    limits.work = 0;
    assert!(matches!(
        eliminate_nested_bounded(&ctx, &block, 2, &mut limits),
        Err(RecursorGenerationError::ResourceLimit)
    ));
}

#[test]
fn indexed_container_keeps_index_binders_and_does_not_turn_indices_into_parameters() {
    let mut ctx = context();
    ctx.add_inductive("Index", sort());
    ctx.add_constructor("index", "Index", g("Index"));
    ctx.add_indexed_inductive(
        "IndexedWrap",
        pi("A", sort(), pi("i", g("Index"), sort())),
        1,
    );
    let family = app(app(g("IndexedWrap"), v("A")), v("i"));
    ctx.add_constructor(
        "indexedWrap",
        "IndexedWrap",
        pi(
            "A",
            sort(),
            pi("i", g("Index"), pi("value", v("A"), family)),
        ),
    );
    let field = app(app(g("IndexedWrap"), g("Tree")), v("i"));
    let decl = NestedDecl {
        name: "Tree".into(),
        sort: sort(),
        constructors: vec![(
            "node".into(),
            pi("i", g("Index"), pi("children", field, g("Tree"))),
        )],
    };
    let result = compare(&ctx, &[decl], 0, &[]);
    assert_eq!(result.block[1].num_params, 0);
    assert_eq!(result.block[1].sort, pi("i", g("Index"), sort()));
}

#[test]
fn polymorphic_block_and_auxiliary_constructors_preserve_the_exact_universe_instance() {
    let mut ctx = Context::new();
    let u = Universe::Var("u".into());
    let s = Term::Sort(u.clone());
    let reference = |name: &str| Term::Const {
        name: name.into(),
        levels: vec![u.clone()],
    };
    ctx.add_universe_poly_inductive("UWrap", vec!["u".into()], pi("A", s.clone(), s.clone()), 1);
    ctx.add_constructor(
        "uwrap",
        "UWrap",
        pi(
            "A",
            s.clone(),
            pi("value", v("A"), app(reference("UWrap"), v("A"))),
        ),
    );
    let tree = app(reference("UTree"), v("A"));
    let decl = NestedDecl {
        name: "UTree".into(),
        sort: pi("A", s.clone(), s.clone()),
        constructors: vec![(
            "unode".into(),
            pi(
                "A",
                s.clone(),
                pi("children", app(reference("UWrap"), tree.clone()), tree),
            ),
        )],
    };
    let result = compare(&ctx, &[decl], 1, &["u".into()]);
    assert_eq!(result.block.len(), 2);
    assert_eq!(result.restoration[0].0, "UTree$nested1$uwrap");
    assert!(
        matches!(&result.restoration[0].1, Term::App(head, _) if matches!(head.as_ref(), Term::Const {name,levels} if name == "uwrap" && levels == &[u]))
    );
}

#[test]
fn nested_parameter_mentioning_constructor_local_is_still_an_invalid_declaration() {
    let ctx = context();
    let parameter = app(g("Tree"), v("local"));
    let decl = NestedDecl {
        name: "Tree".into(),
        sort: sort(),
        constructors: vec![(
            "node".into(),
            pi(
                "local",
                sort(),
                pi("children", app(g("Wrap"), parameter), g("Tree")),
            ),
        )],
    };
    let members = [decl];
    let block = NestedBlock {
        members: &members,
        num_params: 0,
        levels: &[],
    };
    assert!(matches!(
        super::super::eliminate_nested(&ctx, &block),
        Err(KernelError::InvalidDeclaration(_))
    ));
    assert!(matches!(
        eliminate_nested_bounded(&ctx, &block, 2, &mut budget()),
        Err(RecursorGenerationError::Kernel(
            KernelError::InvalidDeclaration(_)
        ))
    ));
}

fn collision_constructor(
    parameter: &str,
    name: &str,
    binder: &str,
    avoid_prime: bool,
) -> (String, Term) {
    let tree = app(g("CollisionTree"), v(parameter));
    let body = if avoid_prime {
        pi("nested.parameter.0'", v(parameter), tree.clone())
    } else {
        tree.clone()
    };
    let parameter_type = pi(binder, v(parameter), body);
    (
        name.into(),
        pi(
            parameter,
            sort(),
            pi("children", app(g("Wrap"), parameter_type), tree),
        ),
    )
}

#[test]
fn colliding_canonical_parameter_binders_have_exact_original_fresh_spelling() {
    let ctx = context();
    let decl = NestedDecl {
        name: "CollisionTree".into(),
        sort: pi("A", sort(), sort()),
        constructors: vec![collision_constructor(
            "A",
            "collisionNode",
            "nested.parameter.0",
            true,
        )],
    };
    let result = compare(&ctx, &[decl], 1, &[]);
    assert_eq!(result.block.len(), 2);
    let image = &result.restoration.last().unwrap().1;
    assert!(
        matches!(image, Term::App(_, argument) if matches!(argument.as_ref(), Term::Pi {param,..} if param == "nested.parameter.0'1"))
    );
}

#[test]
fn differently_named_source_parameters_still_deduplicate_original_equal_instances() {
    let ctx = context();
    let decl = NestedDecl {
        name: "CollisionTree".into(),
        sort: pi("A", sort(), sort()),
        constructors: vec![
            collision_constructor("A", "collisionA", "nested.parameter.0", false),
            collision_constructor("B", "collisionB", "nested.parameter.0", false),
            collision_constructor("C", "collisionC", "nested.parameter.0", false),
        ],
    };
    // Reference substitution chooses nested.parameter.0' independently in each
    // constructor. A store-global fresh counter would split this one instance.
    let result = compare(&ctx, &[decl], 1, &[]);
    assert_eq!(result.block.len(), 2);
    assert_eq!(result.block[0].constructors.len(), 3);
}

#[test]
fn alpha_equivalent_but_structurally_distinct_original_instances_remain_distinct() {
    let ctx = context();
    let decl = NestedDecl {
        name: "CollisionTree".into(),
        sort: pi("A", sort(), sort()),
        constructors: vec![
            collision_constructor("A", "collisionX", "x", false),
            collision_constructor("B", "collisionY", "y", false),
            collision_constructor("C", "collisionXAgain", "x", false),
        ],
    };
    let result = compare(&ctx, &[decl], 1, &[]);
    assert_eq!(
        result.block.len(),
        3,
        "alpha-canonicalizing auxiliary keys would incorrectly merge x and y"
    );
}

fn exact_substitution(body: Term, variable: &str, replacement: Term) -> Term {
    let expected = crate::substitute(&body, variable, &replacement);
    let mut limits = budget();
    let mut d = Construction::new(&mut limits).unwrap();
    let root = d.intern(&body).unwrap();
    let value = d.intern(&replacement).unwrap();
    let symbol = d.symbol(variable).unwrap();
    // Earlier unrelated freshening must not affect the deterministic operation.
    let unrelated = d.intern(&pi("y", sort(), v("other"))).unwrap();
    let y = d.var("y").unwrap();
    let other = d.symbol("other").unwrap();
    d.instantiate(unrelated, &[(other, y)]).unwrap();
    let result = substitution_exact::substitute(&mut d, root, symbol, value).unwrap();
    let result = d.extract(result).unwrap();
    assert_eq!(result, expected);
    result
}

#[test]
fn exact_substitution_freshens_target_free_children_when_root_target_is_live() {
    let inert = pi("y", sort(), v("y"));
    let result = exact_substitution(app(inert, v("x")), "x", v("y"));
    assert!(
        matches!(result, Term::App(a, _) if matches!(a.as_ref(), Term::Pi {param,..} if param == "y'"))
    );
    // The public entry's sole occurrence check still preserves a wholly dead root.
    let inert = pi("y", sort(), v("y"));
    assert_eq!(exact_substitution(inert.clone(), "x", v("y")), inert);
}

#[test]
fn exact_substitution_matches_shadowing_and_fresh_avoidance_in_every_binding_variant() {
    let replacement = app(v("y"), v("y'"));
    for term in [
        pi("y", v("x"), app(v("x"), v("y'1"))),
        Term::Lambda {
            param: "y".into(),
            param_type: Box::new(v("x")),
            body: Box::new(app(v("x"), v("y"))),
        },
        Term::Fix {
            name: "y".into(),
            body: Box::new(app(v("x"), v("y"))),
        },
        Term::Let {
            name: "y".into(),
            ty: Box::new(v("x")),
            value: Box::new(v("x")),
            body: Box::new(app(v("x"), v("y"))),
        },
        Term::Let {
            name: "x".into(),
            ty: Box::new(v("x")),
            value: Box::new(v("x")),
            body: Box::new(app(v("x"), v("y"))),
        },
        Term::Match {
            discriminant: Box::new(v("x")),
            motive: Box::new(pi("y", sort(), v("y"))),
            cases: vec![pi("y", sort(), app(v("x"), v("y")))],
        },
        Term::MutualFix {
            defs: vec![
                ("y".into(), app(v("x"), v("z"))),
                ("z".into(), app(v("y"), v("y'1"))),
            ],
            index: 1,
        },
        Term::MutualFix {
            defs: vec![("x".into(), app(v("x"), v("y"))), ("z".into(), v("y"))],
            index: 0,
        },
    ] {
        exact_substitution(term, "x", replacement.clone());
    }
}

#[test]
fn exact_substitution_mutual_freshening_sees_every_current_name_and_body() {
    let term = Term::MutualFix {
        defs: vec![
            ("f".into(), app(v("x"), v("g"))),
            ("g".into(), app(v("f'"), app(v("g'"), v("f")))),
        ],
        index: 0,
    };
    let result = exact_substitution(term, "x", app(v("f"), v("g")));
    assert!(
        matches!(result, Term::MutualFix {defs,..} if defs[0].0 == "f'1" && defs[1].0 == "g'1")
    );
}

#[test]
fn collision_in_later_uniform_parameter_domain_preserves_exact_domain_equality() {
    let ctx = context();
    let domain = |parameter: &str| pi("nested.parameter.0", v(parameter), v(parameter));
    let tree = app(app(g("UniformCollision"), v("Q")), v("R"));
    let decl = NestedDecl {
        name: "UniformCollision".into(),
        sort: pi("A", sort(), pi("B", domain("A"), sort())),
        constructors: vec![(
            "uniformCollision".into(),
            pi(
                "Q",
                sort(),
                pi(
                    "R",
                    domain("Q"),
                    pi("children", app(g("Wrap"), tree.clone()), tree),
                ),
            ),
        )],
    };
    let result = compare(&ctx, &[decl], 2, &[]);
    assert_eq!(
        result.parameters[1].1,
        pi(
            "nested.parameter.0'",
            v("nested.parameter.0"),
            v("nested.parameter.0")
        )
    );
    assert_eq!(result.block.len(), 2);
}
