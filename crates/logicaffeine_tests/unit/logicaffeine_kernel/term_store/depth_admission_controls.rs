use super::*;

fn spine(depth: usize) -> Term {
    (0..depth).fold(Term::Global("leaf".into()), |tail, _| {
        Term::App(Box::new(Term::Global("step".into())), Box::new(tail))
    })
}

#[test]
fn ordinary_syntax_is_depth_admitted_without_typing_or_scope_effects() {
    let before_stack = kernel_stack_bytes();
    let before_kernel = crate::type_checker::inside_kernel();
    // Admission measures syntax only: these need not be well typed or bound.
    for term in [Term::Hole, Term::Global("not_declared".into()), spine(3)] {
        assert!(within_kernel_depth(&term));
    }
    assert_eq!(kernel_stack_bytes(), before_stack);
    assert_eq!(crate::type_checker::inside_kernel(), before_kernel);
}

#[test]
fn the_existing_kernel_depth_limit_is_inclusive() {
    let term = spine(crate::type_checker::MAX_TERM_DEPTH);
    let depth = term_depth(&term);
    let admitted = within_kernel_depth(&term);
    // Match the existing deep-measurement controls: dispose before assertions,
    // so an assertion failure cannot recursively drop the deep test fixture.
    std::mem::forget(term);
    assert_eq!(depth, crate::type_checker::MAX_TERM_DEPTH);
    assert!(admitted, "the existing exact depth limit remains admitted");
}

#[test]
fn one_edge_above_the_existing_kernel_depth_limit_is_refused() {
    let term = spine(crate::type_checker::MAX_TERM_DEPTH + 1);
    let depth = term_depth(&term);
    let admitted = within_kernel_depth(&term);
    std::mem::forget(term);
    assert_eq!(depth, crate::type_checker::MAX_TERM_DEPTH + 1);
    assert!(
        !admitted,
        "the public predicate must preserve the existing door"
    );
}
