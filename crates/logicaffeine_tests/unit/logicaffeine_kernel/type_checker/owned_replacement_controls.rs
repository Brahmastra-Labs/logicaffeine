use super::*;
fn substitute_owned_closed_value(body: Term, var: &str, replacement: Term) -> Term {
    let occurrences =
        crate::term_store::bounded_occurrence_count_in_term(&body, var, 64, 4096).unwrap() as usize;
    super::substitute_owned_closed_value_counted(body, var, replacement, occurrences)
}

fn v(n: &str) -> Term {
    Term::Var(n.into())
}
fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn ap(f: Term, a: Term) -> Term {
    Term::App(Box::new(f), Box::new(a))
}
#[test]
fn single_occurrence_moves_the_original_replacement_storage() {
    let replacement = ap(g("callee"), g("argument"));
    let expected = replacement.clone();
    let Term::App(f, a) = &replacement else {
        unreachable!()
    };
    let pointers = (f.as_ref() as *const Term, a.as_ref() as *const Term);
    take_substitution_copy_work();
    let result = substitute_owned_closed_value(v("x"), "x", replacement);
    assert_eq!(result, expected);
    assert_eq!(take_substitution_copy_work(), (0, 0));
    let Term::App(f, a) = result else {
        unreachable!()
    };
    assert_eq!(
        (f.as_ref() as *const Term, a.as_ref() as *const Term),
        pointers
    );
}
#[test]
fn repeated_occurrences_clone_only_before_the_final_move() {
    let replacement = g("payload");
    let body = ap(v("x"), v("x"));
    let expected = substitute(&body, "x", &replacement);
    take_substitution_copy_work();
    let result = substitute_owned_closed_value(body, "x", replacement);
    assert_eq!(result, expected);
    assert_eq!(take_substitution_copy_work(), (1, 7));
}
#[test]
fn absent_occurrences_do_not_copy_or_insert_the_replacement() {
    let body = ap(g("f"), g("a"));
    let expected = body.clone();
    take_substitution_copy_work();
    assert_eq!(
        substitute_owned_closed_value(body, "x", g("unused")),
        expected
    );
    assert_eq!(take_substitution_copy_work(), (0, 0));
}
#[test]
fn annotation_let_fix_and_mutual_scopes_match_existing_substitution() {
    let body = Term::Let {
        name: "x".into(),
        ty: Box::new(v("x")),
        value: Box::new(v("x")),
        body: Box::new(v("x")),
    };
    let binder = Term::Lambda {
        param: "x".into(),
        param_type: Box::new(v("x")),
        body: Box::new(v("x")),
    };
    let fix = Term::Fix {
        name: "x".into(),
        body: Box::new(v("x")),
    };
    let mutual = Term::MutualFix {
        defs: vec![("x".into(), v("x")), ("y".into(), v("x"))],
        index: 0,
    };
    let open_mutual = Term::MutualFix {
        defs: vec![("f".into(), v("x")), ("g".into(), v("x"))],
        index: 1,
    };
    for body in [body, binder, fix, mutual, open_mutual] {
        let expected = substitute(&body, "x", &g("closed"));
        assert_eq!(
            substitute_owned_closed_value(body, "x", g("closed")),
            expected
        );
    }
}
#[test]
fn every_match_field_shares_one_occurrence_count() {
    let body = Term::Match {
        discriminant: Box::new(v("x")),
        motive: Box::new(v("x")),
        cases: vec![v("x"), v("x")],
    };
    let expected = substitute(&body, "x", &g("data"));
    take_substitution_copy_work();
    assert_eq!(
        substitute_owned_closed_value(body, "x", g("data")),
        expected
    );
    assert_eq!(take_substitution_copy_work(), (3, 12));
}
