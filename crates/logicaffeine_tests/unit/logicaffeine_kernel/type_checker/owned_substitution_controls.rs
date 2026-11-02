use super::*;

fn v(name: &str) -> Term {
    Term::Var(name.into())
}
fn g(name: &str) -> Term {
    Term::Global(name.into())
}
fn app(f: Term, a: Term) -> Term {
    Term::App(Box::new(f), Box::new(a))
}
fn lam(name: &str, ty: Term, body: Term) -> Term {
    Term::Lambda {
        param: name.into(),
        param_type: Box::new(ty),
        body: Box::new(body),
    }
}
fn pi(name: &str, ty: Term, body: Term) -> Term {
    Term::Pi {
        param: name.into(),
        param_type: Box::new(ty),
        body_type: Box::new(body),
    }
}
fn reference(body: &Term, var: &str, value: &Term) -> Term {
    if !occurs_free(body, var) {
        return body.clone();
    }
    reference_substitute_avoiding(body, var, value, &free_vars(value))
}

#[test]
fn owned_application_retains_both_boxes_and_untouched_identifier_storage() {
    let mut name = String::with_capacity(128);
    name.push_str("stable_operator");
    let input = app(Term::Global(name), v("x"));
    let expected = reference(&input, "x", &g("Cold"));
    let Term::App(f, a) = &input else { panic!() };
    let fp = f.as_ref() as *const Term as usize;
    let ap = a.as_ref() as *const Term as usize;
    let Term::Global(name) = f.as_ref() else {
        panic!()
    };
    let np = name.as_ptr() as usize;
    let nc = name.capacity();
    let result = substitute_owned_closed(input, "x", &g("Cold"));
    assert_eq!(result, expected);
    let Term::App(f, a) = result else { panic!() };
    assert_eq!(f.as_ref() as *const Term as usize, fp);
    assert_eq!(a.as_ref() as *const Term as usize, ap);
    let Term::Global(name) = f.as_ref() else {
        panic!()
    };
    assert_eq!((name.as_ptr() as usize, name.capacity()), (np, nc));
}

#[test]
fn owned_match_retains_case_vector_and_preserves_motive_substitution() {
    let mut cases = Vec::with_capacity(32);
    cases.extend([v("x"), lam("x", v("x"), v("x"))]);
    let cp = cases.as_ptr() as usize;
    let cc = cases.capacity();
    let input = Term::Match {
        discriminant: Box::new(v("x")),
        motive: Box::new(v("x")),
        cases,
    };
    let expected = reference(&input, "x", &g("Cold"));
    let result = substitute_owned_closed(input, "x", &g("Cold"));
    assert_eq!(result, expected);
    let Term::Match { cases, .. } = result else {
        panic!()
    };
    assert_eq!((cases.as_ptr() as usize, cases.capacity()), (cp, cc));
}

#[test]
fn owned_mutual_block_retains_vector_and_simultaneous_shadowing() {
    let mut defs = Vec::with_capacity(16);
    defs.extend([
        ("f".into(), app(v("x"), v("g"))),
        ("g".into(), app(v("f"), v("x"))),
    ]);
    let pointer = defs.as_ptr() as usize;
    let capacity = defs.capacity();
    let input = Term::MutualFix { defs, index: 1 };
    let expected = reference(&input, "x", &g("Cold"));
    let result = substitute_owned_closed(input, "x", &g("Cold"));
    assert_eq!(result, expected);
    let Term::MutualFix { defs, index } = result else {
        panic!()
    };
    assert_eq!(index, 1);
    assert_eq!(
        (defs.as_ptr() as usize, defs.capacity()),
        (pointer, capacity)
    );
    let original = Term::MutualFix { defs, index };
    assert_eq!(
        substitute_owned_closed(original.clone(), "f", &g("Cold")),
        original
    );
}

#[test]
fn binder_annotations_are_outside_the_shadowing_scope() {
    for input in [lam("x", v("x"), v("x")), pi("x", v("x"), v("x"))] {
        let expected = reference(&input, "x", &g("Cold"));
        assert_eq!(substitute_owned_closed(input, "x", &g("Cold")), expected);
    }
    assert_eq!(
        substitute_owned_closed(lam("x", v("x"), v("x")), "x", &g("Cold")),
        lam("x", g("Cold"), v("x"))
    );
}

#[test]
fn let_annotations_and_values_are_outside_its_binder_and_fix_shadows() {
    let input = Term::Let {
        name: "x".into(),
        ty: Box::new(v("x")),
        value: Box::new(v("x")),
        body: Box::new(v("x")),
    };
    assert_eq!(
        substitute_owned_closed(input, "x", &g("Cold")),
        Term::Let {
            name: "x".into(),
            ty: Box::new(g("Cold")),
            value: Box::new(g("Cold")),
            body: Box::new(v("x"))
        }
    );
    let fix = Term::Fix {
        name: "x".into(),
        body: Box::new(app(v("x"), g("Cold"))),
    };
    assert_eq!(substitute_owned_closed(fix.clone(), "x", &g("Warm")), fix);
}

#[test]
fn owned_closed_substitution_clones_only_replacement_nodes_and_names() {
    let input = lam(
        "long_preserved_binder",
        g("Pebble"),
        app(g("long_preserved_operator"), v("x")),
    );
    take_substitution_copy_work();
    let borrowed = substitute(&input, "x", &g("Cold"));
    let borrowed_work = take_substitution_copy_work();
    let owned = substitute_owned_closed(input, "x", &g("Cold"));
    let owned_work = take_substitution_copy_work();
    assert_eq!(owned, borrowed);
    assert_eq!(owned_work, (1, 4));
    assert!(borrowed_work.0 > owned_work.0 && borrowed_work.1 > owned_work.1);
}

#[test]
fn absent_variable_owned_substitution_allocates_no_term_or_identifier_copy() {
    let input = lam("x", g("Pebble"), app(g("Warm"), v("x")));
    let expected = input.clone();
    take_substitution_copy_work();
    assert_eq!(
        substitute_owned_closed(input, "absent", &g("Cold")),
        expected
    );
    assert_eq!(take_substitution_copy_work(), (0, 0));
}

#[test]
fn borrowed_capture_freshening_matches_the_original_algorithm() {
    let value = app(v("y"), v("y_0"));
    for input in [
        lam("y", g("Pebble"), app(v("x"), v("y"))),
        pi("y", v("x"), app(v("x"), v("y"))),
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
    ] {
        let expected = reference(&input, "x", &value);
        let result = substitute(&input, "x", &value);
        assert_eq!(result, expected);
        let fvs = free_vars(&result);
        assert!(fvs.contains("y") && fvs.contains("y_0"));
    }
}

#[test]
fn borrowed_mutual_freshening_is_consistent_across_all_bodies() {
    let input = Term::MutualFix {
        defs: vec![
            ("f".into(), app(v("g"), v("x"))),
            ("g".into(), app(v("f"), v("x"))),
        ],
        index: 1,
    };
    let value = app(v("f"), v("g"));
    assert_eq!(
        substitute(&input, "x", &value),
        reference(&input, "x", &value)
    );
    assert_eq!(substitute(&input, "f", &value), input);
}

#[test]
fn borrowed_and_owned_closed_paths_match_original_for_every_term_variant() {
    let terms = vec![
        Term::Sort(Universe::Prop),
        v("x"),
        g("x"),
        Term::Const {
            name: "x".into(),
            levels: vec![Universe::Type(1)],
        },
        lam("y", v("x"), v("x")),
        pi("x", v("x"), v("x")),
        app(v("x"), v("x")),
        Term::Match {
            discriminant: Box::new(v("x")),
            motive: Box::new(v("x")),
            cases: vec![v("x")],
        },
        Term::Fix {
            name: "f".into(),
            body: Box::new(v("x")),
        },
        Term::MutualFix {
            defs: vec![("f".into(), v("x"))],
            index: 0,
        },
        Term::Let {
            name: "y".into(),
            ty: Box::new(v("x")),
            value: Box::new(v("x")),
            body: Box::new(v("x")),
        },
        Term::Lit(crate::Literal::Nat(0i64.into())),
        Term::Hole,
    ];
    for input in terms {
        let expected = reference(&input, "x", &g("Cold"));
        assert_eq!(substitute(&input, "x", &g("Cold")), expected);
        assert_eq!(substitute_owned_closed(input, "x", &g("Cold")), expected);
    }
}

fn reference_substitute_avoiding(
    body: &Term,
    var: &str,
    replacement: &Term,
    replacement_fvs: &std::collections::HashSet<String>,
) -> Term {
    match body {
        Term::Sort(u) => Term::Sort(u.clone()),

        // A universe-polymorphic reference has no term variables to substitute.
        Term::Const { .. } => body.clone(),

        // Literals are never substituted
        Term::Lit(lit) => Term::Lit(lit.clone()),

        // Holes are never substituted (they're implicit type placeholders)
        Term::Hole => Term::Hole,

        Term::Annotated { annotation, term } => Term::annotated(
            annotation.clone(),
            reference_substitute_avoiding(term, var, replacement, replacement_fvs),
        ),

        Term::Var(name) if name == var => replacement.clone(),
        Term::Var(name) => Term::Var(name.clone()),

        // Globals are never substituted (they're not bound variables)
        Term::Global(name) => Term::Global(name.clone()),

        Term::Pi {
            param,
            param_type,
            body_type,
        } => {
            let new_param_type =
                reference_substitute_avoiding(param_type, var, replacement, replacement_fvs);
            if param == var {
                // The parameter shadows `var`: do not substitute in the body.
                Term::Pi {
                    param: param.clone(),
                    param_type: Box::new(new_param_type),
                    body_type: (*body_type).clone(),
                }
            } else if replacement_fvs.contains(param) {
                // Capture-avoidance: rename the binder away from the free vars
                // of `replacement` before substituting into the body.
                let fresh = freshen(param, body_type, replacement_fvs, var);
                let renamed = rename_var(body_type, param, &fresh);
                Term::Pi {
                    param: fresh,
                    param_type: Box::new(new_param_type),
                    body_type: Box::new(reference_substitute_avoiding(
                        &renamed,
                        var,
                        replacement,
                        replacement_fvs,
                    )),
                }
            } else {
                Term::Pi {
                    param: param.clone(),
                    param_type: Box::new(new_param_type),
                    body_type: Box::new(reference_substitute_avoiding(
                        body_type,
                        var,
                        replacement,
                        replacement_fvs,
                    )),
                }
            }
        }

        Term::Lambda {
            param,
            param_type,
            body,
        } => {
            let new_param_type =
                reference_substitute_avoiding(param_type, var, replacement, replacement_fvs);
            if param == var {
                // The parameter shadows `var`: do not substitute in the body.
                Term::Lambda {
                    param: param.clone(),
                    param_type: Box::new(new_param_type),
                    body: (*body).clone(),
                }
            } else if replacement_fvs.contains(param) {
                // Capture-avoidance: rename the binder away from the free vars
                // of `replacement` before substituting into the body.
                let fresh = freshen(param, body, replacement_fvs, var);
                let renamed = rename_var(body, param, &fresh);
                Term::Lambda {
                    param: fresh,
                    param_type: Box::new(new_param_type),
                    body: Box::new(reference_substitute_avoiding(
                        &renamed,
                        var,
                        replacement,
                        replacement_fvs,
                    )),
                }
            } else {
                Term::Lambda {
                    param: param.clone(),
                    param_type: Box::new(new_param_type),
                    body: Box::new(reference_substitute_avoiding(
                        body,
                        var,
                        replacement,
                        replacement_fvs,
                    )),
                }
            }
        }

        Term::App(func, arg) => Term::App(
            Box::new(reference_substitute_avoiding(
                func,
                var,
                replacement,
                replacement_fvs,
            )),
            Box::new(reference_substitute_avoiding(
                arg,
                var,
                replacement,
                replacement_fvs,
            )),
        ),

        Term::Match {
            discriminant,
            motive,
            cases,
        } => Term::Match {
            discriminant: Box::new(reference_substitute_avoiding(
                discriminant,
                var,
                replacement,
                replacement_fvs,
            )),
            motive: Box::new(reference_substitute_avoiding(
                motive,
                var,
                replacement,
                replacement_fvs,
            )),
            cases: cases
                .iter()
                .map(|c| reference_substitute_avoiding(c, var, replacement, replacement_fvs))
                .collect(),
        },

        Term::Fix { name, body } => {
            if name == var {
                // The fixpoint name shadows `var`: do not substitute in the body.
                Term::Fix {
                    name: name.clone(),
                    body: body.clone(),
                }
            } else if replacement_fvs.contains(name) {
                // Capture-avoidance: rename the fixpoint binder.
                let fresh = freshen(name, body, replacement_fvs, var);
                let renamed = rename_var(body, name, &fresh);
                Term::Fix {
                    name: fresh,
                    body: Box::new(reference_substitute_avoiding(
                        &renamed,
                        var,
                        replacement,
                        replacement_fvs,
                    )),
                }
            } else {
                Term::Fix {
                    name: name.clone(),
                    body: Box::new(reference_substitute_avoiding(
                        body,
                        var,
                        replacement,
                        replacement_fvs,
                    )),
                }
            }
        }

        Term::MutualFix { defs, index } => {
            // Every def name binds in EVERY body. If `var` is one of them it is shadowed
            // throughout — leave the whole block untouched.
            if defs.iter().any(|(n, _)| n == var) {
                return body.clone();
            }
            // Capture-avoidance: any def name that is a free var of `replacement` must be
            // α-renamed CONSISTENTLY across all bodies (it is one mutual binder shared by
            // all) before the substitution descends.
            let mut names: Vec<String> = defs.iter().map(|(n, _)| n.clone()).collect();
            let mut bodies: Vec<Term> = defs.iter().map(|(_, b)| b.clone()).collect();
            for i in 0..names.len() {
                if replacement_fvs.contains(&names[i]) {
                    let mut avoid = replacement_fvs.clone();
                    for b in &bodies {
                        all_var_names(b, &mut avoid);
                    }
                    for n in &names {
                        avoid.insert(n.clone());
                    }
                    avoid.insert(var.to_string());
                    let fresh = fresh_name(&names[i], &avoid);
                    for b in bodies.iter_mut() {
                        *b = rename_var(b, &names[i], &fresh);
                    }
                    names[i] = fresh;
                }
            }
            let new_defs = names
                .into_iter()
                .zip(bodies.iter())
                .map(|(n, b)| {
                    (
                        n,
                        reference_substitute_avoiding(b, var, replacement, replacement_fvs),
                    )
                })
                .collect();
            Term::MutualFix {
                defs: new_defs,
                index: *index,
            }
        }

        Term::Let {
            name,
            ty,
            value,
            body,
        } => {
            // `ty` and `value` are outside the `name` binder — always substitute.
            let new_ty = reference_substitute_avoiding(ty, var, replacement, replacement_fvs);
            let new_value = reference_substitute_avoiding(value, var, replacement, replacement_fvs);
            if name == var {
                // The let-binder shadows `var`: leave the body untouched.
                Term::Let {
                    name: name.clone(),
                    ty: Box::new(new_ty),
                    value: Box::new(new_value),
                    body: body.clone(),
                }
            } else if replacement_fvs.contains(name) {
                // Capture-avoidance: rename the let-binder away from the
                // replacement's free vars before substituting into the body.
                let fresh = freshen(name, body, replacement_fvs, var);
                let renamed = rename_var(body, name, &fresh);
                Term::Let {
                    name: fresh,
                    ty: Box::new(new_ty),
                    value: Box::new(new_value),
                    body: Box::new(reference_substitute_avoiding(
                        &renamed,
                        var,
                        replacement,
                        replacement_fvs,
                    )),
                }
            } else {
                Term::Let {
                    name: name.clone(),
                    ty: Box::new(new_ty),
                    value: Box::new(new_value),
                    body: Box::new(reference_substitute_avoiding(
                        body,
                        var,
                        replacement,
                        replacement_fvs,
                    )),
                }
            }
        }
    }
}
