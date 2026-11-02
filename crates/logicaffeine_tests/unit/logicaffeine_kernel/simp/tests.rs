use super::*;

/// Helper to build SName "s"
fn make_sname(s: &str) -> Term {
    Term::App(
        Box::new(Term::Global("SName".to_string())),
        Box::new(Term::Lit(Literal::Text(s.to_string()))),
    )
}

/// Helper to build SVar i
fn make_svar(i: i64) -> Term {
    Term::App(
        Box::new(Term::Global("SVar".to_string())),
        Box::new(Term::Lit(Literal::Int(i))),
    )
}

/// Helper to build SLit n
fn make_slit(n: i64) -> Term {
    Term::App(
        Box::new(Term::Global("SLit".to_string())),
        Box::new(Term::Lit(Literal::Int(n))),
    )
}

/// Helper to build SApp f a
fn make_sapp(f: Term, a: Term) -> Term {
    Term::App(
        Box::new(Term::App(
            Box::new(Term::Global("SApp".to_string())),
            Box::new(f),
        )),
        Box::new(a),
    )
}

#[test]
fn test_term_to_sterm_lit() {
    let term = make_slit(42);
    let result = term_to_sterm(&term);
    assert_eq!(result, Some(STerm::Lit(42)));
}

#[test]
fn test_term_to_sterm_var() {
    let term = make_svar(0);
    let result = term_to_sterm(&term);
    assert_eq!(result, Some(STerm::Var(0)));
}

#[test]
fn test_term_to_sterm_name() {
    let term = make_sname("add");
    let result = term_to_sterm(&term);
    assert_eq!(result, Some(STerm::Name("add".to_string())));
}

#[test]
fn test_term_to_sterm_app() {
    // (add 2 3) = SApp (SApp (SName "add") (SLit 2)) (SLit 3)
    let add_2 = make_sapp(make_sname("add"), make_slit(2));
    let add_2_3 = make_sapp(add_2, make_slit(3));
    let result = term_to_sterm(&add_2_3);

    let expected = STerm::App(
        Box::new(STerm::App(
            Box::new(STerm::Name("add".to_string())),
            Box::new(STerm::Lit(2)),
        )),
        Box::new(STerm::Lit(3)),
    );
    assert_eq!(result, Some(expected));
}

#[test]
fn test_arithmetic_add() {
    // (add 2) applied to 3
    let func = STerm::App(
        Box::new(STerm::Name("add".to_string())),
        Box::new(STerm::Lit(2)),
    );
    let arg = STerm::Lit(3);
    let result = try_arithmetic(&func, &arg);
    assert_eq!(result, Some(STerm::Lit(5)));
}

#[test]
fn test_arithmetic_mul() {
    let func = STerm::App(
        Box::new(STerm::Name("mul".to_string())),
        Box::new(STerm::Lit(4)),
    );
    let arg = STerm::Lit(5);
    let result = try_arithmetic(&func, &arg);
    assert_eq!(result, Some(STerm::Lit(20)));
}

#[test]
fn test_arithmetic_sub() {
    let func = STerm::App(
        Box::new(STerm::Name("sub".to_string())),
        Box::new(STerm::Lit(10)),
    );
    let arg = STerm::Lit(3);
    let result = try_arithmetic(&func, &arg);
    assert_eq!(result, Some(STerm::Lit(7)));
}

#[test]
fn test_simplify_constant_addition() {
    // 2 + 3 should simplify to 5
    let term = STerm::App(
        Box::new(STerm::App(
            Box::new(STerm::Name("add".to_string())),
            Box::new(STerm::Lit(2)),
        )),
        Box::new(STerm::Lit(3)),
    );
    let result = simplify_sterm(&term, &HashMap::new(), 100);
    assert_eq!(result, STerm::Lit(5));
}

#[test]
fn test_simplify_nested_arithmetic() {
    // (1 + 1) * 3 = 6
    let one_plus_one = STerm::App(
        Box::new(STerm::App(
            Box::new(STerm::Name("add".to_string())),
            Box::new(STerm::Lit(1)),
        )),
        Box::new(STerm::Lit(1)),
    );
    let term = STerm::App(
        Box::new(STerm::App(
            Box::new(STerm::Name("mul".to_string())),
            Box::new(one_plus_one),
        )),
        Box::new(STerm::Lit(3)),
    );
    let result = simplify_sterm(&term, &HashMap::new(), 100);
    assert_eq!(result, STerm::Lit(6));
}

#[test]
fn test_simplify_with_substitution() {
    // x + 1 with x = 0 should give 1
    let x_plus_1 = STerm::App(
        Box::new(STerm::App(
            Box::new(STerm::Name("add".to_string())),
            Box::new(STerm::Var(0)),
        )),
        Box::new(STerm::Lit(1)),
    );
    let mut subst = HashMap::new();
    subst.insert(0, STerm::Lit(0));

    let result = simplify_sterm(&x_plus_1, &subst, 100);
    assert_eq!(result, STerm::Lit(1));
}

#[test]
fn test_check_goal_reflexive() {
    // (Eq x x) should be provable
    let x = make_svar(0);
    let goal = make_sapp(make_sapp(make_sname("Eq"), x.clone()), x);
    assert!(check_goal(&goal), "simp should prove x = x");
}

#[test]
fn test_check_goal_constant() {
    // (Eq (add 2 3) 5) should be provable
    let add_2_3 = make_sapp(make_sapp(make_sname("add"), make_slit(2)), make_slit(3));
    let goal = make_sapp(make_sapp(make_sname("Eq"), add_2_3), make_slit(5));
    assert!(check_goal(&goal), "simp should prove 2+3 = 5");
}

#[test]
fn test_check_goal_with_hypothesis() {
    // (implies (Eq x 0) (Eq (add x 1) 1)) should be provable
    let x = make_svar(0);
    let zero = make_slit(0);
    let one = make_slit(1);

    let x_plus_1 = make_sapp(make_sapp(make_sname("add"), x.clone()), one.clone());
    let hyp = make_sapp(make_sapp(make_sname("Eq"), x), zero);
    let concl = make_sapp(make_sapp(make_sname("Eq"), x_plus_1), one);
    let goal = make_sapp(make_sapp(make_sname("implies"), hyp), concl);

    assert!(check_goal(&goal), "simp should prove x=0 -> x+1=1");
}

#[test]
fn test_check_goal_false_equality() {
    // (Eq 2 3) should NOT be provable
    let goal = make_sapp(make_sapp(make_sname("Eq"), make_slit(2)), make_slit(3));
    assert!(!check_goal(&goal), "simp should NOT prove 2 = 3");
}
