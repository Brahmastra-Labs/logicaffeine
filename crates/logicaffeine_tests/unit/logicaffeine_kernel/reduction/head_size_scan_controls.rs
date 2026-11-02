use super::*;

fn g(name: &str) -> Term {
    Term::Global(name.into())
}
fn v(name: &str) -> Term {
    Term::Var(name.into())
}
fn app(f: Term, a: Term) -> Term {
    Term::App(Box::new(f), Box::new(a))
}
fn lam(name: &str, body: Term) -> Term {
    Term::Lambda {
        param: name.into(),
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
fn counted(f: impl FnOnce() -> Option<Term>) -> (Option<Term>, usize) {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            BOUNDED_SIZE_CALLS.with(|n| n.set(None));
        }
    }
    BOUNDED_SIZE_CALLS.with(|n| n.set(Some(0)));
    let _reset = Reset;
    let result = f();
    (result, BOUNDED_SIZE_CALLS.with(|n| n.get().unwrap()))
}

#[test]
fn each_entered_application_head_is_size_checked_once() {
    let ctx = Context::new();
    for depth in [0, 1, 2, 8, 32, 64] {
        let mut term = g("stuck");
        for _ in 0..depth {
            term = app(term, g("argument"));
        }
        let mut fuel = depth + 1;
        let (result, scans) = counted(|| beta_iota_zeta_head(&ctx, &term, &mut fuel));
        assert_eq!(result, Some(term));
        assert_eq!(fuel, 0);
        assert_eq!(scans, depth + 2, "duplicate scan at spine depth {depth}");
    }
}

#[test]
fn entered_match_discriminant_is_size_checked_once() {
    let ctx = Context::new();
    let term = Term::Match {
        discriminant: Box::new(g("stuck")),
        motive: Box::new(g("motive")),
        cases: vec![],
    };
    let mut fuel = 2;
    let (result, scans) = counted(|| beta_iota_zeta_head(&ctx, &term, &mut fuel));
    assert_eq!(result, Some(term));
    assert_eq!(fuel, 0);
    assert_eq!(scans, 3);
}

#[test]
fn results_and_remaining_fuel_match_previous_reducer() {
    let ctx = context();
    let mut terms = vec![
        g("Zero"),
        app(g("Succ"), g("Zero")),
        app(lam("x", v("x")), g("Zero")),
        app(app(lam("x", lam("y", v("x"))), v("y")), g("Zero")),
        Term::Let {
            name: "x".into(),
            ty: Box::new(g("Nat")),
            value: Box::new(g("Zero")),
            body: Box::new(app(g("Succ"), v("x"))),
        },
        Term::Match {
            discriminant: Box::new(app(g("Succ"), g("Zero"))),
            motive: Box::new(lam("n", g("Nat"))),
            cases: vec![g("Zero"), lam("p", v("p"))],
        },
        Term::Match {
            discriminant: Box::new(g("Zero")),
            motive: Box::new(lam("n", g("Nat"))),
            cases: vec![g("Zero")],
        },
        app(
            Term::Fix {
                name: "f".into(),
                body: Box::new(lam("x", app(v("f"), v("x")))),
            },
            g("Zero"),
        ),
    ];
    let seed = terms.clone();
    for term in seed {
        terms.push(app(lam("x", v("x")), term.clone()));
        terms.push(Term::Let {
            name: "x".into(),
            ty: Box::new(g("Nat")),
            value: Box::new(term),
            body: Box::new(v("x")),
        });
    }
    for (index, term) in terms.iter().enumerate() {
        for allowance in 0..=24 {
            let (mut before, mut after) = (allowance, allowance);
            let expected = previous_head(&ctx, term, &mut before);
            let actual = beta_iota_zeta_head(&ctx, term, &mut after);
            assert_eq!(actual, expected, "term {index}, allowance {allowance}");
            assert_eq!(
                after, before,
                "fuel for term {index}, allowance {allowance}"
            );
        }
    }
}

#[test]
fn oversized_input_is_rejected_before_spending_fuel() {
    let ctx = Context::new();
    let term = Term::Match {
        discriminant: Box::new(g("stuck")),
        motive: Box::new(g("motive")),
        cases: vec![g("case"); 4096],
    };
    for allowance in [0, 1, 64] {
        let mut fuel = allowance;
        let (result, scans) = counted(|| beta_iota_zeta_head(&ctx, &term, &mut fuel));
        assert_eq!(result, None);
        assert_eq!(fuel, allowance);
        assert_eq!(scans, 1);
    }
}

#[test]
fn substitution_expansion_keeps_size_guard_and_exact_fuel() {
    let ctx = context();
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
    for allowance in [0, 1, 2, 3, 64] {
        let (mut before, mut after) = (allowance, allowance);
        assert_eq!(previous_head(&ctx, &term, &mut before), None);
        assert_eq!(beta_iota_zeta_head(&ctx, &term, &mut after), None);
        assert_eq!(before, after);
    }
}

// Frozen reducer from the parent commit: exact results and fuel oracle.
fn previous_head(ctx: &Context, term: &Term, budget: &mut usize) -> Option<Term> {
    let _clone_nodes = bounded_conversion_size(term)?;
    #[cfg(test)]
    BOUNDED_HEAD_CLONE_NODES.with(|count| {
        if let Some(previous) = count.get() {
            count.set(Some(previous + _clone_nodes));
        }
    });
    // Continuations own the siblings which the recursive evaluator used to
    // leave in its callers. Moving a demanded child avoids copying its prefix;
    // every evaluation visit still spends the original shared work allowance.
    enum Frame {
        Apply(Term),
        Match { motive: Term, cases: Vec<Term> },
    }
    let mut frames = Vec::new();
    let mut current = term.clone();
    let mut entering = false; // the original input was preflighted above
    'evaluate: loop {
        #[cfg(test)]
        BOUNDED_HEAD_VISITS.with(|count| {
            if let Some(previous) = count.get() {
                count.set(Some(previous + 1));
            }
        });
        if entering {
            bounded_conversion_size(&current)?;
        }
        if *budget == 0 {
            return None;
        }
        *budget -= 1;
        bounded_conversion_size(&current)?;
        match current {
            Term::Let {
                name, value, body, ..
            } => {
                current = bounded_conversion_substitute_owned(*body, &name, *value)?;
                entering = false;
                continue;
            }
            Term::App(function, argument) => {
                frames.push(Frame::Apply(*argument));
                current = *function;
                entering = true;
                continue;
            }
            Term::Match {
                discriminant,
                motive,
                cases,
            } => {
                frames.push(Frame::Match {
                    motive: *motive,
                    cases,
                });
                current = *discriminant;
                entering = true;
                continue;
            }
            _ => {}
        }
        // Returning through a continuation is not a fresh evaluation visit.
        // This exactly mirrors the old call return, including partial spines.
        while let Some(frame) = frames.pop() {
            match frame {
                Frame::Apply(argument) => {
                    if let Term::Lambda { param, body, .. } = current {
                        current = bounded_conversion_substitute_owned(*body, &param, argument)?;
                        entering = false;
                        continue 'evaluate;
                    }
                    let applied = Term::App(Box::new(current), Box::new(argument));
                    let (head, args) = app_spine(&applied);
                    if let Term::Fix { name, body } = head {
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
                            entering = false;
                            continue 'evaluate;
                        }
                    }
                    current = applied;
                }
                Frame::Match { motive, cases } => {
                    let full_inventory = constructor_inductive_of(ctx, &current)
                        .is_some_and(|ind| ctx.get_constructors(&ind).len() == cases.len());
                    if let Some(branch) = full_inventory
                        .then(|| iota_branch(ctx, &current, &motive, &cases))
                        .flatten()
                    {
                        current = branch;
                        entering = false;
                        continue 'evaluate;
                    }
                    current = Term::Match {
                        discriminant: Box::new(current),
                        motive: Box::new(motive),
                        cases,
                    };
                }
            }
        }
        return Some(current);
    }
}
