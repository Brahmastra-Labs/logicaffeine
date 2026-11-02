use super::*;
fn var(n: &str) -> Term {
    Term::Var(n.into())
}
fn block(a: &str, b: &str, free: &str, index: usize) -> Term {
    Term::MutualFix {
        defs: vec![
            (a.into(), Term::App(Box::new(var(b)), Box::new(var(free)))),
            (
                b.into(),
                Term::Lambda {
                    param: "x".into(),
                    param_type: Box::new(var(free)),
                    body: Box::new(Term::App(Box::new(var(a)), Box::new(var("x")))),
                },
            ),
        ],
        index,
    }
}
#[test]
fn mutual_alpha_renames_all_recursive_binders_simultaneously() {
    assert!(alpha_eq(
        &block("f", "g", "outside", 0),
        &block("g", "f", "outside", 0)
    ));
}
#[test]
fn mutual_alpha_preserves_free_names_and_selected_component() {
    let a = block("f", "g", "outside", 0);
    assert!(!alpha_eq(&a, &block("f", "g", "outside", 1)));
    assert!(!alpha_eq(&a, &block("f", "g", "unknown", 0)));
    assert!(!alpha_eq(&a, &block("f", "g", "f", 0)));
    let mut changed = a.clone();
    if let Term::MutualFix { defs, .. } = &mut changed {
        defs[1].1 = var("unknown");
    }
    assert!(!alpha_eq(&a, &changed));
    let mut hole = a.clone();
    if let Term::MutualFix { defs, .. } = &mut hole {
        defs[1].1 = Term::Hole;
    }
    assert!(!alpha_eq(&hole, &hole));
}
