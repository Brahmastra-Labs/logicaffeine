use super::*;
use crate::Universe;

fn work() -> SourceWork {
    SourceWork::new(&EnvironmentLimits::default()).unwrap()
}
fn leaf() -> Term {
    Term::Sort(Universe::Prop)
}
fn tree(depth: usize) -> Term {
    if depth == 0 {
        leaf()
    } else {
        Term::App(Box::new(tree(depth - 1)), Box::new(tree(depth - 1)))
    }
}
#[test]
fn read_only_universe_scope_does_not_construct_a_quadratically_charged_graph() {
    let term = tree(10);
    let mut w = work();
    w.budget.work = 50_000;
    w.budget.allocation = 50_000;
    let before = w.budget.materialized_nodes;
    assert!(w
        .levels(&[], &[&term])
        .expect("scope traversal fits its unchanged finite budget"));
    assert_eq!(w.budget.materialized_nodes, before);
}

fn positions(t: Term) -> Vec<Term> {
    let b = || Box::new(t.clone());
    let p = || Box::new(leaf());
    vec![
        t.clone(),
        Term::App(b(), p()),
        Term::App(p(), b()),
        Term::Pi {
            param: "x".into(),
            param_type: b(),
            body_type: p(),
        },
        Term::Pi {
            param: "x".into(),
            param_type: p(),
            body_type: b(),
        },
        Term::Lambda {
            param: "x".into(),
            param_type: b(),
            body: p(),
        },
        Term::Lambda {
            param: "x".into(),
            param_type: p(),
            body: b(),
        },
        Term::Let {
            name: "x".into(),
            ty: b(),
            value: p(),
            body: p(),
        },
        Term::Let {
            name: "x".into(),
            ty: p(),
            value: b(),
            body: p(),
        },
        Term::Let {
            name: "x".into(),
            ty: p(),
            value: p(),
            body: b(),
        },
        Term::Match {
            discriminant: b(),
            motive: p(),
            cases: vec![leaf()],
        },
        Term::Match {
            discriminant: p(),
            motive: b(),
            cases: vec![leaf()],
        },
        Term::Match {
            discriminant: p(),
            motive: p(),
            cases: vec![leaf(), t.clone()],
        },
        Term::Fix {
            name: "f".into(),
            body: b(),
        },
        Term::MutualFix {
            defs: vec![("f".into(), leaf()), ("g".into(), t)],
            index: 0,
        },
    ]
}
#[test]
fn every_term_position_and_symbolic_level_agree_with_the_existing_collector() {
    let u = Universe::Var("u".into());
    let v = Universe::Var("v".into());
    let universes = vec![
        Universe::SProp,
        Universe::Prop,
        Universe::Type(u32::MAX),
        u.clone(),
        Universe::Succ(Box::new(v.clone())),
        Universe::Max(Box::new(u.clone()), Box::new(v.clone())),
        Universe::IMax(Box::new(u), Box::new(v)),
    ];
    for level in universes {
        for root in [
            Term::Sort(level.clone()),
            Term::Const {
                name: "K".into(),
                levels: vec![Universe::Prop, level],
            },
        ] {
            for term in positions(root) {
                let vars = crate::term::free_level_vars(&term);
                for allowed in [
                    vec![],
                    vec!["u".to_string()],
                    vec!["v".to_string()],
                    vec!["u".to_string(), "v".to_string()],
                ] {
                    assert_eq!(
                        work().levels(&allowed, &[&term]).unwrap(),
                        vars.iter().all(|v| allowed.contains(v))
                    );
                }
            }
        }
    }
}
#[test]
fn repeated_declared_universes_are_refused_and_non_universe_names_are_not_variables() {
    for term in [
        Term::Var("u".into()),
        Term::Global("u".into()),
        Term::Lit(crate::Literal::Text("u".into())),
        Term::Hole,
    ] {
        assert!(work().levels(&[], &[&term]).unwrap());
        assert!(!work().levels(&["u".into(), "u".into()], &[&term]).unwrap());
    }
}
#[test]
fn work_and_allocation_exhaustion_still_refuse() {
    for allocation in [false, true] {
        let mut w = work();
        if allocation {
            w.budget.allocation = 0;
        } else {
            w.budget.work = 0;
        }
        assert_eq!(
            w.levels(&[], &[&leaf()]).unwrap_err().reason().code(),
            DiagnosticCode::ResourceLimit
        );
    }
}
#[test]
fn term_and_universe_depth_and_total_nodes_remain_bounded() {
    let term = tree(5);
    let mut w = work();
    w.budget.max_depth = 4;
    assert_eq!(
        w.levels(&[], &[&term]).unwrap_err().reason().code(),
        DiagnosticCode::ResourceLimit
    );
    let level = Universe::Succ(Box::new(Universe::Succ(Box::new(Universe::Prop))));
    let mut w = work();
    w.budget.max_depth = 2;
    assert_eq!(
        w.levels(&[], &[&Term::Sort(level)])
            .unwrap_err()
            .reason()
            .code(),
        DiagnosticCode::ResourceLimit
    );
    let mut w = work();
    w.budget.max_tree_nodes = 1;
    assert_eq!(
        w.levels(&[], &[&leaf()]).unwrap_err().reason().code(),
        DiagnosticCode::ResourceLimit
    );
}
#[test]
fn sibling_terms_share_the_same_budget() {
    let mut w = work();
    let term = tree(4);
    let before = w.budget.work;
    assert!(w.levels(&[], &[&term]).unwrap());
    let cost = before - w.budget.work;
    let mut w = work();
    w.budget.work = cost;
    assert_eq!(
        w.levels(&[], &[&term, &term]).unwrap_err().reason().code(),
        DiagnosticCode::ResourceLimit
    );
}
