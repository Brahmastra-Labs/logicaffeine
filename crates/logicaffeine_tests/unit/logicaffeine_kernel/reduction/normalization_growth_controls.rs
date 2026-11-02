use super::*;

fn growing_term(depth: usize) -> Term {
    (0..depth).fold(Term::Global("argument".into()), |tail, _| {
        Term::App(Box::new(Term::Global("head".into())), Box::new(tail))
    })
}

#[test]
fn stack_growth_preserves_spent_reduction_budget() {
    let term = growing_term(crate::term_store::INLINE_DEPTH + 8);
    let result = normalize_progress(
        &Context::new(),
        Transparency::All,
        term,
        Term::Global("other".into()),
        0,
        1,
        MAX_REDUCTION_STEPS,
    );
    assert!(matches!(result, Err((_, Unreduced::BudgetSpent))));
}

#[test]
fn stack_growth_preserves_cycle_detector() {
    let term = growing_term(crate::term_store::INLINE_DEPTH + 8);
    let result = normalize_progress(
        &Context::new(),
        Transparency::All,
        term.clone(),
        term,
        0,
        1,
        1,
    );
    assert!(matches!(result, Err((_, Unreduced::Cyclic))));
}

#[test]
fn grown_terms_retain_the_existing_depth_ceiling() {
    let term = growing_term(crate::type_checker::MAX_TERM_DEPTH + 1);
    let result = normalize_progress(
        &Context::new(),
        Transparency::All,
        term,
        Term::Global("other".into()),
        0,
        1,
        1,
    );
    assert!(matches!(result, Err((_, Unreduced::TooDeep))));
}
