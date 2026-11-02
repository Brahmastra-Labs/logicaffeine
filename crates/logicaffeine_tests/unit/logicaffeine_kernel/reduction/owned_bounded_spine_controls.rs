use super::*;
fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn v(n: &str) -> Term {
    Term::Var(n.into())
}
fn app(f: Term, a: Term) -> Term {
    Term::App(Box::new(f), Box::new(a))
}
fn lam(n: &str, body: Term) -> Term {
    Term::Lambda {
        param: n.into(),
        param_type: Box::new(g("Nat")),
        body: Box::new(body),
    }
}
fn context() -> Context {
    let mut ctx = Context::new();
    ctx.add_inductive("Nat", Term::Sort(crate::Universe::Type(0)));
    ctx.add_constructor_checked("Zero", "Nat", g("Nat"))
        .unwrap();
    ctx.add_constructor_checked(
        "Succ",
        "Nat",
        Term::Pi {
            param: "n".into(),
            param_type: Box::new(g("Nat")),
            body_type: Box::new(g("Nat")),
        },
    )
    .unwrap();
    ctx
}
fn copied(f: impl FnOnce() -> Option<Term>) -> (Option<Term>, usize) {
    BOUNDED_HEAD_CLONE_NODES.with(|n| n.set(Some(0)));
    let value = f();
    let count = BOUNDED_HEAD_CLONE_NODES.with(|n| n.replace(None).unwrap());
    (value, count)
}
#[test]
fn partially_applied_two_lambda_helper_does_not_copy_its_function_prefix() {
    let ctx = context();
    let body = lam("x", lam("y", app(g("Succ"), v("x"))));
    let term = app(body, g("Zero"));
    crate::infer_type(&ctx, &term).unwrap();
    let input_nodes = bounded_conversion_size(&term).unwrap();
    let (value, copies) = copied(|| beta_iota_zeta_head(&ctx, &term, &mut 256));
    assert!(crate::type_checker::bounded_probe_alpha_eq_for_test(
        &value.unwrap(),
        &lam("y", app(g("Succ"), g("Zero")))
    ));
    assert!(
        copies <= input_nodes,
        "copied {copies} nodes from a {input_nodes}-node source"
    );
}
#[test]
fn owned_spine_matches_the_old_reducer_on_typed_beta_zeta_terms() {
    let ctx = context();
    for depth in 0..12 {
        let mut term = g("Zero");
        for i in 0..depth {
            term = if i % 2 == 0 {
                app(lam("x", app(g("Succ"), v("x"))), term)
            } else {
                Term::Let {
                    name: "x".into(),
                    ty: Box::new(g("Nat")),
                    value: Box::new(term),
                    body: Box::new(v("x")),
                }
            };
        }
        crate::infer_type(&ctx, &term).unwrap();
        let old = legacy_bounded_spine_head(&ctx, &term, &mut 256).unwrap();
        let new = beta_iota_zeta_head(&ctx, &term, &mut 256).unwrap();
        assert!(crate::type_checker::bounded_probe_alpha_eq_for_test(
            &new, &old
        ));
    }
}
#[test]
fn owned_spine_preserves_exact_small_fuel_exhaustion() {
    let ctx = context();
    let term = app(lam("x", v("x")), g("Zero"));
    for allowance in 0..=3 {
        let mut old_fuel = allowance;
        let mut new_fuel = allowance;
        let old = legacy_bounded_spine_head(&ctx, &term, &mut old_fuel);
        let new = beta_iota_zeta_head(&ctx, &term, &mut new_fuel);
        assert_eq!(new, old);
        assert_eq!(new_fuel, old_fuel);
    }
    let mut fuel = 1;
    assert_eq!(
        beta_iota_zeta_head(&ctx, &g("Zero"), &mut fuel),
        Some(g("Zero"))
    );
    assert_eq!(fuel, 0);
}
#[test]
fn owned_spine_preserves_capture_and_argument_order() {
    let ctx = context().extend("y", g("Nat"));
    let term = app(app(lam("x", lam("y", v("x"))), v("y")), g("Zero"));
    crate::infer_type(&ctx, &term).unwrap();
    assert_eq!(beta_iota_zeta_head(&ctx, &term, &mut 256), Some(v("y")));
    let term = app(app(lam("x", lam("y", v("y"))), v("y")), g("Zero"));
    assert_eq!(beta_iota_zeta_head(&ctx, &term, &mut 256), Some(g("Zero")));
}
#[test]
fn owned_spine_iota_uses_only_complete_constructor_inventories() {
    let ctx = context();
    let term = Term::Match {
        discriminant: Box::new(app(g("Succ"), g("Zero"))),
        motive: Box::new(lam("n", g("Nat"))),
        cases: vec![g("Zero"), lam("p", v("p"))],
    };
    crate::infer_type(&ctx, &term).unwrap();
    assert_eq!(beta_iota_zeta_head(&ctx, &term, &mut 256), Some(g("Zero")));
    let partial = Term::Match {
        discriminant: Box::new(g("Zero")),
        motive: Box::new(lam("n", g("Nat"))),
        cases: vec![g("Zero")],
    };
    assert_eq!(beta_iota_zeta_head(&ctx, &partial, &mut 256), Some(partial));
    let bad = Term::Fix {
        name: "f".into(),
        body: Box::new(lam("x", app(v("f"), v("x")))),
    };
    let bad = app(bad, g("Zero"));
    assert_eq!(beta_iota_zeta_head(&ctx, &bad, &mut 256), Some(bad));
}
#[test]
fn owned_spine_keeps_input_and_substitution_size_guards() {
    let ctx = context();
    let mut wide = g("Nat");
    for _ in 0..12 {
        wide = Term::Pi {
            param: "_".into(),
            param_type: Box::new(wide.clone()),
            body_type: Box::new(wide),
        };
    }
    let (result, copies) = copied(|| beta_iota_zeta_head(&ctx, &wide, &mut 256));
    assert!(result.is_none());
    assert_eq!(copies, 0);
    let mut body = v("x");
    for _ in 0..6 {
        body = app(app(g("pair"), body.clone()), body);
    }
    let mut argument = g("Zero");
    for _ in 0..32 {
        argument = app(g("Succ"), argument);
    }
    let term = app(lam("x", body), argument);
    assert!(bounded_conversion_size(&term).is_some());
    assert_eq!(beta_iota_zeta_head(&ctx, &term, &mut 256), None);
}

fn legacy_bounded_spine_head(ctx: &Context, term: &Term, budget: &mut usize) -> Option<Term> {
    let _clone_nodes = bounded_conversion_size(term)?;
    #[cfg(test)]
    BOUNDED_HEAD_CLONE_NODES.with(|count| {
        if let Some(previous) = count.get() {
            count.set(Some(previous + _clone_nodes));
        }
    });
    let mut current = term.clone();
    loop {
        if *budget == 0 {
            return None;
        }
        *budget -= 1;
        bounded_conversion_size(&current)?;
        match current {
            Term::Let {
                name, value, body, ..
            } => current = bounded_conversion_substitute(&body, &name, &value)?,
            Term::App(func, arg) => {
                let func = legacy_bounded_spine_head(ctx, &func, budget)?;
                if let Term::Lambda { param, body, .. } = func {
                    current = bounded_conversion_substitute(&body, &param, &arg)?;
                } else {
                    let applied = Term::App(Box::new(func), arg);
                    let (head, args) = app_spine(&applied);
                    if let Term::Fix { name, body } = head {
                        // Reuse the guard's structural column, exactly as the
                        // ordinary reducer does. No argument is normalized here.
                        // Both input and substituted output obey the small-term
                        // bound, so this probe cannot expand an unrestricted tree.
                        let column = (args.iter().any(|a| is_constructor_form(ctx, a))
                            && crate::termination::check_termination(ctx, name, body).is_ok())
                        .then(|| crate::termination::structural_columns(ctx, body))
                        .and_then(|columns| columns.first().copied());
                        if column
                            .and_then(|i| args.get(i))
                            .is_some_and(|arg| is_constructor_form(ctx, arg))
                        {
                            let unfolded = bounded_conversion_substitute(body, name, head)?;
                            current = args.iter().fold(unfolded, |f, a| {
                                Term::App(Box::new(f), Box::new((*a).clone()))
                            });
                            continue;
                        }
                    }
                    return Some(applied);
                }
            }
            Term::Match {
                discriminant,
                motive,
                cases,
            } => {
                let discriminant = legacy_bounded_spine_head(ctx, &discriminant, budget)?;
                // Pruned branch inventories need index conversion. Leave that
                // potentially expensive work to ordinary conversion.
                let full_inventory = constructor_inductive_of(ctx, &discriminant)
                    .is_some_and(|ind| ctx.get_constructors(&ind).len() == cases.len());
                if let Some(branch) = full_inventory
                    .then(|| iota_branch(ctx, &discriminant, &motive, &cases))
                    .flatten()
                {
                    current = branch;
                } else {
                    return Some(Term::Match {
                        discriminant: Box::new(discriminant),
                        motive,
                        cases,
                    });
                }
            }
            _ => return Some(current),
        }
    }
}
