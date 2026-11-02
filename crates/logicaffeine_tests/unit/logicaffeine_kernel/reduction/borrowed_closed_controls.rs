use super::*;
use crate::term_store::{
    free_name_work, intern_visits, reset_free_name_work, reset_intern_visits, NodeShape, TermStore,
};
fn g(name: &str) -> Term {
    Term::Global(name.into())
}
fn v(name: &str) -> Term {
    Term::Var(name.into())
}
fn app(a: Term, b: Term) -> Term {
    Term::App(Box::new(a), Box::new(b))
}

#[test]
fn closed_admission_borrows_the_term_without_interning_or_collecting_names() {
    let body = (0..128).fold(v("x"), |t, _| app(g("step"), t));
    let term = Term::Lambda {
        param: "x".into(),
        param_type: Box::new(g("Nat")),
        body: Box::new(body),
    };
    let expected = bounded_conversion_size(&term);
    reset_intern_visits();
    reset_free_name_work();
    assert_eq!(bounded_closed_conversion_size(&term), expected);
    assert_eq!(
        intern_visits(),
        0,
        "admission must not build a temporary term graph"
    );
    assert_eq!(
        free_name_work(),
        (0, 0),
        "admission needs no materialized free-name vector"
    );
}

#[test]
fn borrowed_admission_matches_graph_oracle_across_all_lexical_positions() {
    let leaves = [
        g("Nat"),
        v("x"),
        v("y"),
        Term::Hole,
        Term::Sort(crate::Universe::Var("u".into())),
        Term::Const {
            name: "C".into(),
            levels: vec![crate::Universe::Var("u".into())],
        },
        Term::Lit(crate::Literal::Int(7)),
    ];
    for a in &leaves {
        for b in &leaves {
            let forms = [
                app(a.clone(), b.clone()),
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
                    defs: vec![("x".into(), a.clone()), ("y".into(), b.clone())],
                    index: 0,
                },
                Term::MutualFix {
                    defs: vec![("x".into(), a.clone()), ("x".into(), b.clone())],
                    index: 0,
                },
                Term::Match {
                    discriminant: Box::new(a.clone()),
                    motive: Box::new(b.clone()),
                    cases: vec![g("case")],
                },
                Term::Match {
                    discriminant: Box::new(g("Zero")),
                    motive: Box::new(a.clone()),
                    cases: vec![b.clone()],
                },
            ];
            for term in forms {
                let mut graph = TermStore::new();
                let id = graph.intern(&term);
                let admissible =
                    graph.free_names_are_empty(id) && !graph.shapes(id).contains(NodeShape::Hole);
                let expected = if admissible {
                    bounded_conversion_size(&term)
                } else {
                    None
                };
                assert_eq!(bounded_closed_conversion_size(&term), expected, "{term:?}");
            }
        }
    }
}

#[test]
fn borrowed_admission_preserves_the_existing_size_boundary() {
    for (links, expected) in [(2047, Some(4095)), (2048, None)] {
        let term = (0..links).fold(g("end"), |t, _| app(g("step"), t));
        assert_eq!(bounded_closed_conversion_size(&term), expected);
    }
    let wide = Term::Match {
        discriminant: Box::new(g("Zero")),
        motive: Box::new(g("Nat")),
        cases: vec![g("branch"); 4096],
    };
    assert_eq!(bounded_closed_conversion_size(&wide), None);
}
