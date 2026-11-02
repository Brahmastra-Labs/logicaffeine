use super::*;

#[test]
fn test_unify_same_constant() {
    let t1 = ProofTerm::Constant("a".into());
    let t2 = ProofTerm::Constant("a".into());
    let result = unify_terms(&t1, &t2);
    assert!(result.is_ok());
    assert!(result.unwrap().is_empty());
}

#[test]
fn test_unify_different_constants() {
    let t1 = ProofTerm::Constant("a".into());
    let t2 = ProofTerm::Constant("b".into());
    let result = unify_terms(&t1, &t2);
    assert!(result.is_err());
}

#[test]
fn test_unify_var_constant() {
    let t1 = ProofTerm::Variable("x".into());
    let t2 = ProofTerm::Constant("a".into());
    let result = unify_terms(&t1, &t2);
    assert!(result.is_ok());
    let subst = result.unwrap();
    assert_eq!(subst.get("x"), Some(&ProofTerm::Constant("a".into())));
}

#[test]
fn test_occurs_check() {
    let t1 = ProofTerm::Variable("x".into());
    let t2 = ProofTerm::Function("f".into(), vec![ProofTerm::Variable("x".into())]);
    let result = unify_terms(&t1, &t2);
    assert!(matches!(result, Err(ProofError::OccursCheck { .. })));
}

#[test]
fn test_compose_substitutions() {
    let mut s1 = Substitution::new();
    s1.insert("x".into(), ProofTerm::Variable("y".into()));

    let mut s2 = Substitution::new();
    s2.insert("y".into(), ProofTerm::Constant("a".into()));

    let composed = compose_substitutions(s1, s2);

    // x should map to a (via y)
    assert_eq!(composed.get("x"), Some(&ProofTerm::Constant("a".into())));
    // y should also map to a
    assert_eq!(composed.get("y"), Some(&ProofTerm::Constant("a".into())));
}

// =========================================================================
// ALPHA-EQUIVALENCE TESTS
// =========================================================================

#[test]
fn test_alpha_equivalence_exists() {
    // ∃e P(e) should unify with ∃x P(x)
    let e1 = ProofExpr::Exists {
        variable: "e".to_string(),
        body: Box::new(ProofExpr::Predicate {
            name: "run".to_string(),
            args: vec![ProofTerm::Variable("e".to_string())],
            world: None,
        }),
    };

    let e2 = ProofExpr::Exists {
        variable: "x".to_string(),
        body: Box::new(ProofExpr::Predicate {
            name: "run".to_string(),
            args: vec![ProofTerm::Variable("x".to_string())],
            world: None,
        }),
    };

    let result = unify_exprs(&e1, &e2);
    assert!(
        result.is_ok(),
        "Alpha-equivalent expressions should unify: {:?}",
        result
    );
}

#[test]
fn test_alpha_equivalence_forall() {
    // ∀x P(x) should unify with ∀y P(y)
    let e1 = ProofExpr::ForAll {
        variable: "x".to_string(),
        body: Box::new(ProofExpr::Predicate {
            name: "mortal".to_string(),
            args: vec![ProofTerm::Variable("x".to_string())],
            world: None,
        }),
    };

    let e2 = ProofExpr::ForAll {
        variable: "y".to_string(),
        body: Box::new(ProofExpr::Predicate {
            name: "mortal".to_string(),
            args: vec![ProofTerm::Variable("y".to_string())],
            world: None,
        }),
    };

    let result = unify_exprs(&e1, &e2);
    assert!(
        result.is_ok(),
        "Alpha-equivalent universals should unify: {:?}",
        result
    );
}

#[test]
fn test_alpha_equivalence_nested() {
    // ∃e (Run(e) ∧ Agent(e, John)) should unify with ∃x (Run(x) ∧ Agent(x, John))
    let e1 = ProofExpr::Exists {
        variable: "e".to_string(),
        body: Box::new(ProofExpr::And(
            Box::new(ProofExpr::Predicate {
                name: "run".to_string(),
                args: vec![ProofTerm::Variable("e".to_string())],
                world: None,
            }),
            Box::new(ProofExpr::Predicate {
                name: "agent".to_string(),
                args: vec![
                    ProofTerm::Variable("e".to_string()),
                    ProofTerm::Constant("John".to_string()),
                ],
                world: None,
            }),
        )),
    };

    let e2 = ProofExpr::Exists {
        variable: "x".to_string(),
        body: Box::new(ProofExpr::And(
            Box::new(ProofExpr::Predicate {
                name: "run".to_string(),
                args: vec![ProofTerm::Variable("x".to_string())],
                world: None,
            }),
            Box::new(ProofExpr::Predicate {
                name: "agent".to_string(),
                args: vec![
                    ProofTerm::Variable("x".to_string()),
                    ProofTerm::Constant("John".to_string()),
                ],
                world: None,
            }),
        )),
    };

    let result = unify_exprs(&e1, &e2);
    assert!(
        result.is_ok(),
        "Nested alpha-equivalent expressions should unify: {:?}",
        result
    );
}

fn pred(name: &str, args: Vec<ProofTerm>) -> ProofExpr {
    ProofExpr::Predicate {
        name: name.into(),
        args,
        world: None,
    }
}

fn var(n: &str) -> ProofTerm {
    ProofTerm::Variable(n.into())
}

fn succ_of(t: ProofTerm) -> ProofTerm {
    ProofTerm::Function("succ".into(), vec![t])
}

#[test]
fn instantiating_a_binder_substitutes_once_even_when_the_term_mentions_the_variable() {
    assert_eq!(
        instantiate_expr(&pred("P", vec![var("a")]), "a", &succ_of(var("a"))),
        pred("P", vec![succ_of(var("a"))])
    );
}

#[test]
fn instantiating_a_variable_with_itself_is_the_identity() {
    let body = pred("P", vec![var("a"), succ_of(var("a"))]);
    assert_eq!(instantiate_expr(&body, "a", &var("a")), body);
}

#[test]
fn instantiation_renames_a_binder_that_would_capture_a_free_variable_of_the_term() {
    let body = ProofExpr::ForAll {
        variable: "b".into(),
        body: Box::new(pred("Q", vec![var("a"), var("b")])),
    };
    match instantiate_expr(&body, "a", &succ_of(var("b"))) {
        ProofExpr::ForAll { variable, body } => {
            assert_ne!(
                variable, "b",
                "the binder keeps the name `b`, so the term's free `b` is captured"
            );
            assert_eq!(*body, pred("Q", vec![succ_of(var("b")), var(&variable)]));
        }
        other => panic!("instantiation must keep the quantifier: {other:?}"),
    }
}

#[test]
fn a_binder_that_rebinds_the_variable_shadows_it() {
    let inner = ProofExpr::ForAll {
        variable: "a".into(),
        body: Box::new(pred("Q", vec![var("a")])),
    };
    let body = ProofExpr::And(Box::new(pred("P", vec![var("a")])), Box::new(inner.clone()));
    let c = ProofTerm::Constant("c".into());
    assert_eq!(
        instantiate_expr(&body, "a", &c),
        ProofExpr::And(Box::new(pred("P", vec![c.clone()])), Box::new(inner))
    );
}

#[test]
fn a_propositional_atom_is_not_a_term_position() {
    let body = ProofExpr::And(
        Box::new(ProofExpr::Atom("a".into())),
        Box::new(pred("P", vec![var("a")])),
    );
    let c = ProofTerm::Constant("c".into());
    assert_eq!(
        instantiate_expr(&body, "a", &c),
        ProofExpr::And(
            Box::new(ProofExpr::Atom("a".into())),
            Box::new(pred("P", vec![c]))
        )
    );
}

#[test]
fn a_triangular_substitution_still_resolves_its_chains() {
    let mut s = Substitution::new();
    s.insert("x".into(), succ_of(var("y")));
    s.insert("y".into(), ProofTerm::Constant("z".into()));
    assert_eq!(
        apply_subst_to_term(&var("x"), &s),
        succ_of(ProofTerm::Constant("z".into()))
    );
}

#[test]
#[should_panic(expected = "cyclic substitution a ↦ a")]
fn a_cyclic_substitution_is_refused_by_name_where_it_would_loop() {
    let mut s = Substitution::new();
    s.insert("a".into(), succ_of(var("a")));
    let _ = apply_subst_to_term(&var("a"), &s);
}

#[test]
#[should_panic(expected = "cyclic substitution x ↦ y ↦ x")]
fn a_cycle_through_two_variables_is_refused_naming_both() {
    let mut s = Substitution::new();
    s.insert("x".into(), succ_of(var("y")));
    s.insert("y".into(), succ_of(var("x")));
    let _ = apply_subst_to_term(&var("x"), &s);
}

#[test]
fn a_cyclic_map_applies_where_its_cycle_is_never_reached() {
    let mut s = Substitution::new();
    s.insert("a".into(), succ_of(var("a")));
    assert_eq!(apply_subst_to_term(&var("b"), &s), var("b"));
}
