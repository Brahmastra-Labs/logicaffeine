// Pending remote regression controls; not yet confirmed installed or executed.
use super::*;
use crate::term_store::{take_free_name_node_visits, TermStore};

fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn v(n: &str) -> Term {
    Term::Var(n.into())
}
fn app(f: Term, a: Term) -> Term {
    Term::App(Box::new(f), Box::new(a))
}
fn annotation() -> Term {
    (0..128).fold(v("external"), |body, _| app(g("Family"), body))
}

#[test]
fn repeated_distinct_name_queries_do_not_rewalk_immutable_annotations() {
    let mut context = Context::new();
    context.add("hypothesis", annotation());
    assert!(!context.local_name_in_use("warmup"));
    take_free_name_node_visits();
    for i in 0..128 {
        assert!(!context.local_name_in_use(&format!("fresh_{i}")));
    }
    let visits = take_free_name_node_visits();
    assert_eq!(
        visits, 0,
        "cached annotation analysis was repeated: {visits} node visits"
    );
    assert!(context.local_name_in_use("external"));
    assert!(context.local_name_in_use("hypothesis"));
}

#[test]
fn extending_a_context_reuses_unchanged_annotation_analysis() {
    let mut context = Context::new();
    context.add("hypothesis", annotation());
    assert!(!context.local_name_in_use("warmup"));
    take_free_name_node_visits();
    for i in 0..32 {
        let extended = context.extend(&format!("binder_{i}"), g("Nat"));
        assert!(!extended.local_name_in_use(&format!("fresh_{i}")));
        assert!(extended.local_name_in_use("external"));
    }
    assert_eq!(
        take_free_name_node_visits(),
        0,
        "extending the telescope rewalked shared, immutable types"
    );
}

#[test]
fn overwriting_a_binding_releases_only_its_own_old_names() {
    let mut context = Context::new();
    context.add("left", app(g("Family"), v("old")));
    context.add("right", app(g("Family"), v("shared")));
    assert!(context.local_name_in_use("old"));
    assert!(context.local_name_in_use("shared"));
    let mut changed = context.clone();
    changed.add("left", app(g("Family"), v("new")));
    assert!(!changed.local_name_in_use("old"));
    assert!(changed.local_name_in_use("new"));
    assert!(changed.local_name_in_use("shared"));
    assert!(context.local_name_in_use("old"));
    assert!(!context.local_name_in_use("new"));
}

#[test]
fn annotation_names_agree_with_term_store_lexical_scopes() {
    let terms = vec![
        Term::Lambda {
            param: "x".into(),
            param_type: Box::new(v("domain")),
            body: Box::new(app(v("x"), v("outside"))),
        },
        Term::Pi {
            param: "x".into(),
            param_type: Box::new(v("x")),
            body_type: Box::new(v("x")),
        },
        Term::Let {
            name: "x".into(),
            ty: Box::new(v("type")),
            value: Box::new(v("x")),
            body: Box::new(app(v("x"), v("outside"))),
        },
        Term::Fix {
            name: "loop".into(),
            body: Box::new(app(v("loop"), v("outside"))),
        },
        Term::MutualFix {
            defs: vec![
                ("a".into(), app(v("b"), v("outside"))),
                ("b".into(), v("a")),
            ],
            index: 0,
        },
        Term::Match {
            discriminant: Box::new(v("disc")),
            motive: Box::new(v("motive")),
            cases: vec![v("case"), g("global_name")],
        },
        Term::Sort(crate::Universe::Var("universe_name".into())),
        Term::Const {
            name: "constant_name".into(),
            levels: vec![],
        },
        Term::Lit(crate::Literal::Text("text_name".into())),
        Term::Hole,
    ];
    for ty in terms {
        let mut store = TermStore::new();
        let id = store.intern(&ty);
        let expected: std::collections::HashSet<_> =
            store.free_vars(id).into_iter().map(str::to_owned).collect();
        let mut context = Context::new();
        context.add("hypothesis", ty);
        for name in [
            "x",
            "domain",
            "outside",
            "type",
            "loop",
            "a",
            "b",
            "disc",
            "motive",
            "case",
            "global_name",
            "universe_name",
            "constant_name",
            "text_name",
            "absent",
        ] {
            assert_eq!(
                context.local_name_in_use(name),
                expected.contains(name),
                "{name}"
            );
        }
    }
}

#[test]
fn nested_shadowing_and_mutual_names_match_independent_occurrence_queries() {
    use crate::term_store::{free_names_in_term, occurs_free_in_term};
    let leaves = [v("x"), v("y"), v("free"), g("x")];
    for a in &leaves {
        for b in &leaves {
            let forms = [
                Term::Lambda {
                    param: "x".into(),
                    param_type: Box::new(a.clone()),
                    body: Box::new(b.clone()),
                },
                Term::Pi {
                    param: "x".into(),
                    param_type: Box::new(a.clone()),
                    body_type: Box::new(b.clone()),
                },
                Term::Let {
                    name: "x".into(),
                    ty: Box::new(a.clone()),
                    value: Box::new(b.clone()),
                    body: Box::new(v("x")),
                },
                Term::Fix {
                    name: "x".into(),
                    body: Box::new(app(a.clone(), b.clone())),
                },
                Term::MutualFix {
                    defs: vec![("x".into(), a.clone()), ("x".into(), b.clone())],
                    index: 0,
                },
                Term::MutualFix {
                    defs: vec![("x".into(), a.clone()), ("y".into(), b.clone())],
                    index: 0,
                },
            ];
            for inner in forms {
                for outer_name in ["x", "y"] {
                    let term = app(
                        Term::Lambda {
                            param: outer_name.into(),
                            param_type: Box::new(v("domain")),
                            body: Box::new(inner.clone()),
                        },
                        app(v("x"), v("after")),
                    );
                    let names = free_names_in_term(&term);
                    for name in ["x", "y", "free", "domain", "after", "absent"] {
                        assert_eq!(
                            names.contains(name),
                            occurs_free_in_term(&term, name),
                            "{term:?}: {name}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn lexical_collection_uses_no_native_recursion_for_deep_binders() {
    std::thread::Builder::new()
        .stack_size(256 * 1024)
        .spawn(|| {
            let mut term = v("outside");
            for _ in 0..20_000 {
                term = Term::Lambda {
                    param: "x".into(),
                    param_type: Box::new(v("domain")),
                    body: Box::new(term),
                };
            }
            let names = crate::term_store::free_names_in_term(&term);
            assert_eq!(
                names,
                ["outside".to_string(), "domain".to_string()]
                    .into_iter()
                    .collect()
            );
            // Dismantle the input iteratively too, so this tests collection's stack use.
            while let Term::Lambda { body, .. } = term {
                term = *body;
            }
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn concurrent_context_clones_initialize_shared_annotation_once() {
    let mut context = Context::new();
    context.add("hypothesis", annotation());
    let visits = std::thread::scope(|scope| {
        let mut handles = Vec::new();
        for _ in 0..8 {
            let cloned = context.clone();
            handles.push(scope.spawn(move || {
                take_free_name_node_visits();
                assert!(!cloned.local_name_in_use("absent"));
                assert!(cloned.local_name_in_use("external"));
                take_free_name_node_visits()
            }));
        }
        handles
            .into_iter()
            .map(|h| h.join().unwrap())
            .sum::<usize>()
    });
    assert_eq!(
        visits, 257,
        "one shared annotation must be analyzed only once"
    );
}

#[test]
fn bound_keys_do_not_force_annotation_analysis() {
    let mut context = Context::new();
    context.add("hypothesis", annotation());
    take_free_name_node_visits();
    assert!(context.local_name_in_use("hypothesis"));
    assert_eq!(take_free_name_node_visits(), 0);
}
