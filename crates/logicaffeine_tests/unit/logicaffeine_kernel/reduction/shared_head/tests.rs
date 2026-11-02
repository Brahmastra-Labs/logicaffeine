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
fn lam(n: &str, ty: Term, body: Term) -> Term {
    Term::Lambda {
        param: n.into(),
        param_type: Box::new(ty),
        body: Box::new(body),
    }
}
fn env() -> Context {
    let mut c = Context::new();
    crate::prelude::StandardLibrary::register(&mut c);
    c
}
fn recursive() -> Term {
    Term::Fix {
        name: "f".into(),
        body: Box::new(lam(
            "n",
            g("Nat"),
            Term::Match {
                discriminant: Box::new(v("n")),
                motive: Box::new(lam("n", g("Nat"), g("Bool"))),
                cases: vec![g("true"), lam("p", g("Nat"), app(v("f"), v("p")))],
            },
        )),
    }
}
#[test]
fn shared_head_agrees_with_checked_recursion_and_rejects_wrong_results() {
    let c = env();
    let t = app(
        recursive(),
        Term::Lit(Literal::Nat(crate::BigInt::from_i64(12))),
    );
    crate::infer_type(&c, &t).unwrap();
    assert_eq!(crate::normalize_checked(&c, &t).unwrap(), g("true"));
    assert!(constructor_equivalent(&c, &t, &g("true"), 100_000));
    assert!(constructor_equivalent(&c, &g("true"), &t, 100_000));
    assert!(!constructor_equivalent(&c, &t, &g("false"), 100_000));
    assert!(!constructor_equivalent(&c, &t, &g("true"), 1));
}
#[test]
fn shared_head_refuses_holes_open_terms_and_unguarded_fixpoints() {
    let c = env();
    let erased = app(lam("x", g("Bool"), g("true")), Term::Hole);
    assert!(!constructor_equivalent(&c, &erased, &g("true"), 1000));
    let open = app(lam("x", g("Bool"), g("true")), v("free"));
    assert!(!constructor_equivalent(&c, &open, &g("true"), 1000));
    let bad = Term::Fix {
        name: "f".into(),
        body: Box::new(lam("n", g("Nat"), app(v("f"), v("n")))),
    };
    assert!(!constructor_equivalent(
        &c,
        &app(bad, g("Zero")),
        &g("true"),
        1000
    ));
}
#[test]
fn closure_memo_keeps_distinct_lexical_environments() {
    let c = env();
    let choose = lam("x", g("Bool"), lam("y", g("Bool"), v("x")));
    let first = app(app(choose.clone(), g("true")), g("false"));
    let second = app(app(choose, g("false")), g("true"));
    let cases = c
        .get_constructors("Bool")
        .iter()
        .map(|(name, _)| {
            if *name == "true" {
                second.clone()
            } else {
                g("true")
            }
        })
        .collect();
    let t = Term::Match {
        discriminant: Box::new(first),
        motive: Box::new(lam("b", g("Bool"), g("Bool"))),
        cases,
    };
    crate::infer_type(&c, &t).unwrap();
    assert_eq!(crate::normalize_checked(&c, &t).unwrap(), g("false"));
    assert!(constructor_equivalent(&c, &t, &g("false"), 100_000));
    assert!(!constructor_equivalent(&c, &t, &g("true"), 100_000));
}

#[test]
fn lazy_fix_forces_the_guarded_column_and_leaves_other_arguments_opaque() {
    let mut c = env();
    c.add_declaration("opaque_nat", g("Nat"));
    let body = lam(
        "unused",
        g("Nat"),
        lam(
            "n",
            g("Nat"),
            Term::Match {
                discriminant: Box::new(v("n")),
                motive: Box::new(lam("n", g("Nat"), g("Bool"))),
                cases: vec![
                    g("true"),
                    lam("p", g("Nat"), app(app(v("f"), v("unused")), v("p"))),
                ],
            },
        ),
    );
    let t = app(
        app(
            Term::Fix {
                name: "f".into(),
                body: Box::new(body),
            },
            g("opaque_nat"),
        ),
        Term::Lit(Literal::Nat(crate::BigInt::from_i64(5))),
    );
    crate::infer_type(&c, &t).unwrap();
    assert_eq!(crate::normalize_checked(&c, &t).unwrap(), g("true"));
    assert!(constructor_equivalent(&c, &t, &g("true"), 100_000));
}

#[test]
fn lazy_delta_validates_universe_arity_and_drops_cache_between_contexts() {
    let mut c = env();
    c.declare_universe_poly("answer", vec!["u".into()], g("Bool"), g("true"))
        .unwrap();
    let valid = Term::Const {
        name: "answer".into(),
        levels: vec![crate::Universe::Type(0)],
    };
    let malformed = Term::Const {
        name: "answer".into(),
        levels: vec![],
    };
    assert!(constructor_equivalent(&c, &valid, &g("true"), 1000));
    assert!(!constructor_equivalent(&c, &malformed, &g("true"), 1000));
    let mut other = env();
    other
        .declare_universe_poly("answer", vec!["u".into()], g("Bool"), g("false"))
        .unwrap();
    assert!(!constructor_equivalent(&other, &valid, &g("true"), 1000));
    assert!(constructor_equivalent(&other, &valid, &g("false"), 1000));
}

#[test]
fn closed_boolean_programs_agree_with_the_tree_reducer() {
    let c = env();
    let ctors = c.get_constructors("Bool");
    let mut programs = vec![g("true"), g("false")];
    for round in 0..4 {
        let previous = programs.clone();
        for (i, a) in previous.iter().take(12).enumerate() {
            let b = previous[(i + round + 1) % previous.len()].clone();
            let chosen = app(lam("x", g("Bool"), lam("y", g("Bool"), v("x"))), a.clone());
            let selected = app(chosen, b.clone());
            let t = Term::Let {
                name: "x".into(),
                ty: Box::new(g("Bool")),
                value: Box::new(selected),
                body: Box::new(Term::Match {
                    discriminant: Box::new(v("x")),
                    motive: Box::new(lam("b", g("Bool"), g("Bool"))),
                    cases: ctors
                        .iter()
                        .map(|(name, _)| {
                            if *name == "true" {
                                b.clone()
                            } else {
                                a.clone()
                            }
                        })
                        .collect(),
                }),
            };
            crate::infer_type(&c, &t).unwrap();
            let normal = crate::normalize_checked(&c, &t).unwrap();
            for target in ["true", "false"] {
                assert_eq!(
                    constructor_equivalent(&c, &t, &g(target), 100_000),
                    normal == g(target)
                );
            }
            programs.push(t);
        }
    }
}

#[test]
fn closure_memo_reuses_equal_constructor_values_and_retains_every_field() {
    let c = env();
    let mut r = HeadReducer::new(&c, 1000);
    let x = r.store.intern(&v("x"));
    let Node::Var(name) = *r.store.node(x) else {
        unreachable!()
    };
    let expression = r.store.intern(&app(g("observe"), v("x")));
    let head = r.store.intern(&g("Pair.mk"));
    let mut delayed = Vec::new();
    for fields in [[1, 2], [1, 2], [1, 3], [2, 2]] {
        let args = fields
            .into_iter()
            .map(|n| {
                r.thunk(Thunk::Done(Value::Nat(crate::BigInt::from_i64(n))))
                    .unwrap()
            })
            .collect();
        let value = r
            .thunk(Thunk::Done(Value::Constructor { head, args }))
            .unwrap();
        delayed.push(r.delay(expression, &bind(None, name, value)).unwrap());
    }
    assert!(Rc::ptr_eq(&delayed[0], &delayed[1]));
    assert!(!Rc::ptr_eq(&delayed[0], &delayed[2]));
    assert!(!Rc::ptr_eq(&delayed[0], &delayed[3]));
}

#[test]
fn closure_memo_stays_useful_after_its_capacity_is_reached() {
    let c = env();
    let mut r = HeadReducer::new(&c, 1000);
    let id = r.store.intern(&g("true"));
    let value = r.thunk(Thunk::Done(Value::Neutral)).unwrap();
    for n in 0..100_000 {
        let key = (id, vec![n]);
        r.closures.insert(key, value.clone());
    }
    let first = r.delay(id, &None).unwrap();
    let second = r.delay(id, &None).unwrap();
    assert!(Rc::ptr_eq(&first, &second));
    assert!(r.closures.len() <= 100_000);
    // Eviction removes only a cache reference; active callers retain values.
    assert!(matches!(*value.state.borrow(), Thunk::Done(Value::Neutral)));
}

#[test]
fn closure_keys_ignore_annotations_but_keep_discriminants_and_constructor_fields() {
    let c = env();
    let mut r = HeadReducer::new(&c, 1000);
    let x = r.store.intern(&v("x"));
    let Node::Var(name) = *r.store.node(x) else {
        unreachable!()
    };
    let a = r
        .thunk(Thunk::Done(Value::Nat(crate::BigInt::from_i64(1))))
        .unwrap();
    let b = r
        .thunk(Thunk::Done(Value::Nat(crate::BigInt::from_i64(2))))
        .unwrap();
    let ea = bind(None, name, a);
    let eb = bind(None, name, b);
    let annotation_only = r.store.intern(&lam("y", v("x"), g("true")));
    let first = r.delay(annotation_only, &ea).unwrap();
    let second = r.delay(annotation_only, &eb).unwrap();
    assert!(Rc::ptr_eq(&first, &second));
    for t in [
        lam("y", g("Nat"), v("x")),
        app(g("Succ"), v("x")),
        Term::Match {
            discriminant: Box::new(v("x")),
            motive: Box::new(lam("n", g("Nat"), g("Bool"))),
            cases: vec![g("true"), lam("p", g("Nat"), g("false"))],
        },
    ] {
        let id = r.store.intern(&t);
        let first = r.delay(id, &ea).unwrap();
        let second = r.delay(id, &eb).unwrap();
        assert!(!Rc::ptr_eq(&first, &second));
    }
}

#[test]
fn closure_keys_share_equivalent_suspended_arguments_without_forcing_them() {
    let c = env();
    let mut r = HeadReducer::new(&c, 1000);
    let x = r.store.intern(&v("x"));
    let Node::Var(name) = *r.store.node(x) else {
        unreachable!()
    };
    let source = r.store.intern(&app(lam("n", g("Nat"), v("n")), g("Zero")));
    let other = r.store.intern(&app(g("Succ"), g("Zero")));
    let expression = r.store.intern(&app(g("Succ"), v("x")));
    let mut delayed = Vec::new();
    for id in [source, source, other] {
        let value = r.thunk(Thunk::Suspended(id, None)).unwrap();
        delayed.push(
            r.delay(expression, &bind(None, name, value.clone()))
                .unwrap(),
        );
        assert!(matches!(*value.state.borrow(), Thunk::Suspended(..)));
    }
    assert!(Rc::ptr_eq(&delayed[0], &delayed[1]));
    assert!(!Rc::ptr_eq(&delayed[0], &delayed[2]));
}

#[test]
fn value_key_eviction_never_recycles_live_identities() {
    let c = env();
    let mut r = HeadReducer::new(&c, 1000);
    let head = r.store.intern(&g("true"));
    let live = r
        .thunk(Thunk::Done(Value::Constructor { head, args: vec![] }))
        .unwrap();
    let original = r.binding_key(&live, 32, &mut 256);
    for n in 0..100_000 {
        r.intern_value_key(ValueKey::Constructor(head, vec![n]), live.id);
    }
    let refreshed = r.intern_value_key(ValueKey::Atom(head), live.id);
    assert_ne!(original, refreshed);
    assert_eq!(r.binding_key(&live, 32, &mut 256), original);
    let fresh = r.thunk(Thunk::Done(Value::Neutral)).unwrap();
    assert_ne!(fresh.id, original);
    assert_ne!(fresh.id, refreshed);
    assert!(r.values.len() <= 100_000);
}
#[test]
fn function_value_keys_respect_captured_values_and_parameter_shadowing() {
    let c = env();
    let mut r = HeadReducer::new(&c, 1000);
    let x = r.store.intern(&v("x"));
    let y = r.store.intern(&v("y"));
    let Node::Var(x_name) = *r.store.node(x) else {
        unreachable!()
    };
    let Node::Var(y_name) = *r.store.node(y) else {
        unreachable!()
    };
    let mut captured = Vec::new();
    let mut shadowed = Vec::new();
    for n in [1, 1, 2] {
        let value = r
            .thunk(Thunk::Done(Value::Nat(crate::BigInt::from_i64(n))))
            .unwrap();
        let env = bind(None, x_name, value);
        let f = r
            .thunk(Thunk::Done(Value::Lambda {
                param: y_name,
                param_type: x,
                body: x,
                env: env.clone(),
            }))
            .unwrap();
        captured.push(r.binding_key(&f, 32, &mut 256));
        let f = r
            .thunk(Thunk::Done(Value::Lambda {
                param: x_name,
                param_type: x,
                body: x,
                env,
            }))
            .unwrap();
        shadowed.push(r.binding_key(&f, 32, &mut 256));
    }
    assert_eq!(captured[0], captured[1]);
    assert_ne!(captured[0], captured[2]);
    assert_eq!(shadowed[0], shadowed[2]);
}
#[test]
fn generation_rotation_keeps_recent_hits_and_never_resurrects_old_values() {
    let mut cache = Generations::<u32, u32, 2>::new();
    cache.insert(0, 10);
    cache.insert(1, 11);
    cache.insert(2, 12);
    assert_eq!(cache.get(&0), Some(10));
    cache.insert(0, 20);
    assert_eq!(cache.get(&0), Some(20));
    for key in 3..50 {
        cache.insert(key, key + 10);
        assert_eq!(cache.get(&0), Some(20));
        assert!(cache.len() <= 4);
    }
    for key in 50..60 {
        cache.insert(key, key + 10);
    }
    assert_eq!(cache.get(&0), None);
    assert!(cache.len() <= 4);
}
#[test]
fn computed_peano_steps_share_the_existing_compact_nat_representation() {
    let c = env();
    let term = Term::Match {
        discriminant: Box::new(Term::Lit(Literal::Nat(crate::BigInt::from_i64(7)))),
        motive: Box::new(lam("n", g("Nat"), g("Nat"))),
        cases: vec![g("Zero"), lam("p", g("Nat"), app(g("Succ"), v("p")))],
    };
    crate::infer_type(&c, &term).unwrap();
    let literal = Term::Lit(Literal::Nat(crate::BigInt::from_i64(7)));
    assert!(crate::type_checker::def_eq(
        &c,
        &crate::normalize_checked(&c, &term).unwrap(),
        &literal
    ));
    let mut r = HeadReducer::new(&c, 10_000);
    let id = r.store.intern(&term);
    assert!(matches!(r.whnf(id),Some(Value::Nat(n)) if n==crate::BigInt::from_i64(7)));
}

#[test]
fn peano_compaction_does_not_force_an_unused_predecessor() {
    let mut c = env();
    c.add_declaration("opaque_nat", g("Nat"));
    let term = Term::Match {
        discriminant: Box::new(app(g("Succ"), g("opaque_nat"))),
        motive: Box::new(lam("n", g("Nat"), g("Bool"))),
        cases: vec![g("false"), lam("p", g("Nat"), g("true"))],
    };
    crate::infer_type(&c, &term).unwrap();
    assert_eq!(crate::normalize_checked(&c, &term).unwrap(), g("true"));
    assert!(constructor_equivalent(&c, &term, &g("true"), 10_000));
    assert!(!constructor_equivalent(&c, &term, &g("false"), 10_000));
}
#[test]
fn compact_nat_bridge_requires_canonical_signatures_and_preserves_zero() {
    let c = env();
    assert!(canonical_nat(&c));
    let zero = Term::Lit(Literal::Nat(crate::BigInt::from_i64(0)));
    let one = Term::Lit(Literal::Nat(crate::BigInt::from_i64(1)));
    assert!(constructor_equivalent(&c, &zero, &g("Zero"), 1000));
    assert!(constructor_equivalent(&c, &g("Zero"), &zero, 1000));
    assert!(!constructor_equivalent(&c, &one, &g("Zero"), 1000));
    for wrong_sort in [true, false] {
        let mut bad = Context::new();
        bad.add_inductive(
            "Nat",
            Term::Sort(if wrong_sort {
                crate::Universe::Prop
            } else {
                crate::Universe::Type(0)
            }),
        );
        bad.add_constructor("Zero", "Nat", g("Nat"));
        bad.add_constructor(
            "Succ",
            "Nat",
            Term::Pi {
                param: "n".into(),
                param_type: Box::new(g(if wrong_sort { "Nat" } else { "Bool" })),
                body_type: Box::new(g("Nat")),
            },
        );
        assert!(!canonical_nat(&bad));
        assert!(!constructor_equivalent(&bad, &zero, &g("Zero"), 1000));
    }
}
#[test]
fn application_spines_preserve_argument_order_and_captured_environments() {
    let c = env();
    for selected in 0..4 {
        let names = ["a", "b", "c", "d"];
        let mut function = v(names[selected]);
        for name in names.iter().rev() {
            function = lam(name, g("Bool"), function);
        }
        let mut term = function;
        for i in 0..4 {
            term = app(term, g(if i == selected { "true" } else { "false" }));
        }
        crate::infer_type(&c, &term).unwrap();
        assert_eq!(crate::normalize_checked(&c, &term).unwrap(), g("true"));
        assert!(constructor_equivalent(&c, &term, &g("true"), 10_000));
        assert!(!constructor_equivalent(&c, &term, &g("false"), 10_000));
    }
    let returned = app(
        lam(
            "x",
            g("Bool"),
            app(lam("y", g("Bool"), lam("z", g("Bool"), v("x"))), g("false")),
        ),
        g("true"),
    );
    let term = app(returned, g("false"));
    crate::infer_type(&c, &term).unwrap();
    assert_eq!(crate::normalize_checked(&c, &term).unwrap(), g("true"));
    assert!(constructor_equivalent(&c, &term, &g("true"), 10_000));
}
#[test]
fn unknown_proof_arguments_never_supply_a_constructor_or_proof_result() {
    let mut c = env();
    c.add_inductive("Witness", Term::Sort(crate::Universe::Prop));
    c.add_constructor("Witness_mk", "Witness", g("Witness"));
    let forced = app(
        lam(
            "p",
            g("Witness"),
            Term::Match {
                discriminant: Box::new(v("p")),
                motive: Box::new(lam("p", g("Witness"), g("Bool"))),
                cases: vec![g("true")],
            },
        ),
        g("Witness_mk"),
    );
    crate::infer_type(&c, &forced).unwrap();
    assert_eq!(crate::normalize_checked(&c, &forced).unwrap(), g("true"));
    // The fast evaluator must be inconclusive when its unknown is demanded.
    assert!(!constructor_equivalent(&c, &forced, &g("true"), 10_000));
    assert!(!constructor_equivalent(&c, &forced, &g("false"), 10_000));
    let forwarded = app(
        lam(
            "p",
            g("Witness"),
            app(lam("q", g("Witness"), g("true")), v("p")),
        ),
        g("Witness_mk"),
    );
    crate::infer_type(&c, &forwarded).unwrap();
    assert!(constructor_equivalent(&c, &forwarded, &g("true"), 10_000));
    assert!(!constructor_equivalent(&c, &forwarded, &g("false"), 10_000));
    // Non-proof inputs still compute normally; their values are indispensable.
    let data = app(lam("b", g("Bool"), v("b")), g("false"));
    assert!(constructor_equivalent(&c, &data, &g("false"), 10_000));
    assert!(!constructor_equivalent(&c, &data, &g("true"), 10_000));
}
#[test]
fn deep_shared_value_and_environment_graphs_drop_on_a_small_stack() {
    std::thread::Builder::new()
        .stack_size(128 * 1024)
        .spawn(|| {
            let mut store = TermStore::new();
            let head = store.intern(&g("C"));
            let var = store.intern(&v("x"));
            let name = match store.node(var) {
                Node::Var(n) => *n,
                _ => unreachable!(),
            };
            let cell = |id, state| {
                Rc::new(ThunkCell {
                    id,
                    key: Cell::new(None),
                    observation: Cell::new(None),
                    state: RefCell::new(state),
                })
            };
            let mut chain = cell(0, Thunk::Done(Value::Neutral));
            let leaf = Rc::downgrade(&chain);
            for id in 1..30_000 {
                chain = cell(
                    id,
                    Thunk::Done(Value::Constructor {
                        head,
                        args: vec![chain.clone(), chain],
                    }),
                );
            }
            let alias = chain.clone();
            drop(chain);
            assert!(leaf.upgrade().is_some());
            drop(alias);
            assert!(leaf.upgrade().is_none());
            let mut env = None;
            for id in 0..30_000 {
                let value = cell(id, Thunk::Suspended(head, env.clone()));
                env = bind(env, name, value);
            }
            let root = Rc::downgrade(env.as_ref().unwrap());
            drop(env);
            assert!(root.upgrade().is_none());
        })
        .unwrap()
        .join()
        .unwrap();
}
#[test]
fn closure_keys_converge_after_captured_computations_finish() {
    let c = env();
    let mut r = HeadReducer::new(&c, 100_000);
    let number = Term::Lit(Literal::Nat(crate::BigInt::from_i64(7)));
    let one = r
        .store
        .intern(&app(lam("a", g("Nat"), v("a")), number.clone()));
    let two = r.store.intern(&app(lam("b", g("Nat"), number), g("Zero")));
    let a = r.delay(one, &None).unwrap();
    let b = r.delay(two, &None).unwrap();
    let body = r.store.intern(&v("captured"));
    let name = match r.store.node(body) {
        Node::Var(n) => *n,
        _ => unreachable!(),
    };
    let ty = r.store.intern(&g("Nat"));
    // A different binder keeps the captured variable free in the body.
    let binder = r.store.intern(&v("unused"));
    let binder = match r.store.node(binder) {
        Node::Var(n) => *n,
        _ => unreachable!(),
    };
    let mk = |r: &mut HeadReducer, value| {
        r.thunk(Thunk::Done(Value::Lambda {
            param: binder,
            param_type: ty,
            body,
            env: bind(None, name, value),
        }))
        .unwrap()
    };
    let f = mk(&mut r, a);
    let g = mk(&mut r, b);
    assert_ne!(
        r.binding_key(&f, 32, &mut 256),
        r.binding_key(&g, 32, &mut 256)
    );
    assert!(matches!(r.whnf(one),Some(Value::Nat(n)) if n==crate::BigInt::from_i64(7)));
    assert!(matches!(r.whnf(two),Some(Value::Nat(n)) if n==crate::BigInt::from_i64(7)));
    assert_eq!(
        r.binding_key(&f, 32, &mut 256),
        r.binding_key(&g, 32, &mut 256)
    );
}
#[test]
fn constructor_proof_fields_share_speculation_but_data_fields_stay_distinct() {
    let mut c = env();
    c.add_inductive("Witness", Term::Sort(crate::Universe::Prop));
    c.add_constructor("Witness_mk", "Witness", g("Witness"));
    c.add_inductive("Packet", Term::Sort(crate::Universe::Type(0)));
    c.add_constructor(
        "Packet_mk",
        "Packet",
        Term::Pi {
            param: "n".into(),
            param_type: Box::new(g("Nat")),
            body_type: Box::new(Term::Pi {
                param: "p".into(),
                param_type: Box::new(g("Witness")),
                body_type: Box::new(g("Packet")),
            }),
        },
    );
    let packet = |n, p| {
        app(
            app(
                g("Packet_mk"),
                Term::Lit(Literal::Nat(crate::BigInt::from_i64(n))),
            ),
            p,
        )
    };
    let p = packet(0, g("Witness_mk"));
    let q = packet(0, app(lam("p", g("Witness"), v("p")), g("Witness_mk")));
    let other = packet(1, g("Witness_mk"));
    let mut r = HeadReducer::new(&c, 100_000);
    let mut keys = Vec::new();
    for term in [&p, &q, &other] {
        crate::infer_type(&c, term).unwrap();
        let id = r.store.intern(term);
        let value = r.whnf(id).unwrap();
        let thunk = r.thunk(Thunk::Done(value)).unwrap();
        keys.push(r.binding_key(&thunk, 32, &mut 256));
    }
    assert_eq!(keys[0], keys[1]);
    assert_ne!(keys[0], keys[2]);
    let observe = Term::Match {
        discriminant: Box::new(q),
        motive: Box::new(lam("q", g("Packet"), g("Bool"))),
        cases: vec![lam(
            "n",
            g("Nat"),
            lam(
                "p",
                g("Witness"),
                Term::Match {
                    discriminant: Box::new(v("p")),
                    motive: Box::new(lam("p", g("Witness"), g("Bool"))),
                    cases: vec![g("true")],
                },
            ),
        )],
    };
    crate::infer_type(&c, &observe).unwrap();
    assert_eq!(crate::normalize_checked(&c, &observe).unwrap(), g("true"));
    assert!(!constructor_equivalent(&c, &observe, &g("true"), 100_000));
    assert!(!constructor_equivalent(&c, &observe, &g("false"), 100_000));
}
fn data_context() -> Context {
    let mut c = Context::new();
    c.add_inductive("Data", Term::Sort(crate::Universe::Type(0)));
    c.add_constructor("Data.zero", "Data", g("Data"));
    c.add_constructor(
        "Data.next",
        "Data",
        Term::Pi {
            param: "value".into(),
            param_type: Box::new(g("Data")),
            body_type: Box::new(g("Data")),
        },
    );
    c.add_constructor(
        "Data.pair",
        "Data",
        Term::Pi {
            param: "left".into(),
            param_type: Box::new(g("Data")),
            body_type: Box::new(Term::Pi {
                param: "right".into(),
                param_type: Box::new(g("Data")),
                body_type: Box::new(g("Data")),
            }),
        },
    );
    c.declare(
        "data.id",
        Term::Pi {
            param: "x".into(),
            param_type: Box::new(g("Data")),
            body_type: Box::new(g("Data")),
        },
        lam("x", g("Data"), v("x")),
    )
    .unwrap();
    c
}
fn shared_data(depth: usize, leaf: Term, prefix: &str) -> Term {
    let mut result = v(&format!("{prefix}{depth}"));
    for i in (1..=depth).rev() {
        let previous = v(&format!("{prefix}{}", i - 1));
        result = Term::Let {
            name: format!("{prefix}{i}"),
            ty: Box::new(g("Data")),
            value: Box::new(app(app(g("Data.pair"), previous.clone()), previous)),
            body: Box::new(result),
        };
    }
    Term::Let {
        name: format!("{prefix}0"),
        ty: Box::new(g("Data")),
        value: Box::new(leaf),
        body: Box::new(result),
    }
}
#[test]
fn lazy_data_comparison_checks_every_constructor_field() {
    let c = data_context();
    let value = app(
        app(g("Data.pair"), g("Data.zero")),
        app(g("Data.next"), g("Data.zero")),
    );
    let delayed = app(g("data.id"), value.clone());
    crate::infer_type(&c, &delayed).unwrap();
    assert!(constructor_equivalent(&c, &delayed, &value, 10_000));
    let different = app(app(g("Data.pair"), g("Data.zero")), g("Data.zero"));
    assert!(!constructor_equivalent(&c, &delayed, &different, 10_000));
    assert!(!constructor_equivalent(&c, &different, &delayed, 10_000));
    assert!(!constructor_equivalent(&c, &delayed, &value, 1));
}
#[test]
fn lazy_data_comparison_reuses_only_completed_shared_subtrees() {
    let c = data_context();
    let left = app(g("data.id"), shared_data(128, g("Data.zero"), "left"));
    let right = app(g("data.id"), shared_data(128, g("Data.zero"), "right"));
    assert!(constructor_equivalent(&c, &left, &right, 20_000));
    let wrong = app(
        g("data.id"),
        shared_data(128, app(g("Data.next"), g("Data.zero")), "wrong"),
    );
    assert!(!constructor_equivalent(&c, &left, &wrong, 20_000));
}
#[test]
fn lazy_data_comparison_cannot_observe_unknown_proofs_or_function_fields() {
    let mut c = data_context();
    c.add_inductive("Witness", Term::Sort(crate::Universe::Prop));
    c.add_constructor("Witness.mk", "Witness", g("Witness"));
    c.add_constructor(
        "Data.proof",
        "Data",
        Term::Pi {
            param: "proof".into(),
            param_type: Box::new(g("Witness")),
            body_type: Box::new(g("Data")),
        },
    );
    let proof = app(g("Data.proof"), g("Witness.mk"));
    assert!(!constructor_equivalent(&c, &proof, &proof, 10_000));
    c.add_inductive("Other", Term::Sort(crate::Universe::Type(0)));
    c.add_constructor("Other.zero", "Other", g("Other"));
    c.add_constructor(
        "Data.function",
        "Data",
        Term::Pi {
            param: "f".into(),
            param_type: Box::new(Term::Pi {
                param: "x".into(),
                param_type: Box::new(g("Other")),
                body_type: Box::new(g("Other")),
            }),
            body_type: Box::new(g("Data")),
        },
    );
    let function = app(g("Data.function"), lam("x", g("Other"), v("x")));
    assert!(!constructor_equivalent(&c, &function, &function, 10_000));
    c.add_declaration("opaque.data", g("Data"));
    let opaque = app(g("data.id"), g("opaque.data"));
    assert!(!constructor_equivalent(&c, &opaque, &opaque, 10_000));
    c.add_inductive(
        "Box",
        Term::Pi {
            param: "A".into(),
            param_type: Box::new(Term::Sort(crate::Universe::Type(0))),
            body_type: Box::new(Term::Sort(crate::Universe::Type(0))),
        },
    );
    c.set_inductive_params("Box", 1);
    c.add_constructor(
        "Box.mk",
        "Box",
        Term::Pi {
            param: "A".into(),
            param_type: Box::new(Term::Sort(crate::Universe::Type(0))),
            body_type: Box::new(Term::Pi {
                param: "x".into(),
                param_type: Box::new(v("A")),
                body_type: Box::new(app(g("Box"), v("A"))),
            }),
        },
    );
    let parameter = app(app(g("Box.mk"), g("Other")), g("Other.zero"));
    crate::infer_type(&c, &parameter).unwrap();
    assert!(!constructor_equivalent(&c, &parameter, &parameter, 10_000));
}

#[test]
fn lazy_data_congruence_compares_arguments_before_expensive_common_head() {
    let mut c = Context::new();
    c.add_inductive("Unary", Term::Sort(crate::Universe::Type(0)));
    c.add_constructor("Unary.zero", "Unary", g("Unary"));
    let arrow = |a, b| Term::Pi {
        param: "x".into(),
        param_type: Box::new(a),
        body_type: Box::new(b),
    };
    c.add_constructor("Unary.next", "Unary", arrow(g("Unary"), g("Unary")));
    let add = Term::Fix {
        name: "add".into(),
        body: Box::new(lam(
            "n",
            g("Unary"),
            lam(
                "m",
                g("Unary"),
                Term::Match {
                    discriminant: Box::new(v("n")),
                    motive: Box::new(lam("n", g("Unary"), g("Unary"))),
                    cases: vec![
                        v("m"),
                        lam(
                            "p",
                            g("Unary"),
                            app(g("Unary.next"), app(app(v("add"), v("p")), v("m"))),
                        ),
                    ],
                },
            ),
        )),
    };
    c.declare(
        "unary.add",
        arrow(g("Unary"), arrow(g("Unary"), g("Unary"))),
        add,
    )
    .unwrap();
    let count = Term::Fix {
        name: "count".into(),
        body: Box::new(lam(
            "n",
            g("Unary"),
            Term::Match {
                discriminant: Box::new(v("n")),
                motive: Box::new(lam("n", g("Unary"), g("Unary"))),
                cases: vec![
                    g("Unary.zero"),
                    lam(
                        "p",
                        g("Unary"),
                        app(
                            app(g("unary.add"), app(v("count"), v("p"))),
                            app(g("Unary.next"), g("Unary.zero")),
                        ),
                    ),
                ],
            },
        )),
    };
    c.declare("unary.count", arrow(g("Unary"), g("Unary")), count)
        .unwrap();
    let numeral = |n| (0..n).fold(g("Unary.zero"), |p, _| app(g("Unary.next"), p));
    let left = app(g("unary.count"), numeral(256));
    let right = app(
        g("unary.count"),
        app(app(g("unary.add"), numeral(255)), numeral(1)),
    );
    crate::infer_type(&c, &left).unwrap();
    crate::infer_type(&c, &right).unwrap();
    assert!(constructor_equivalent(&c, &left, &right, 20_000));
    let wrong = app(g("unary.count"), numeral(255));
    assert!(!constructor_equivalent(&c, &left, &wrong, 20_000));
}

fn unary_count_context() -> Context {
    let mut c = Context::new();
    c.add_inductive("Unary", Term::Sort(crate::Universe::Type(0)));
    c.add_constructor("Unary.zero", "Unary", g("Unary"));
    let arrow = |a, b| Term::Pi {
        param: "x".into(),
        param_type: Box::new(a),
        body_type: Box::new(b),
    };
    c.add_constructor("Unary.next", "Unary", arrow(g("Unary"), g("Unary")));
    c.declare(
        "unary.add",
        arrow(g("Unary"), arrow(g("Unary"), g("Unary"))),
        Term::Fix {
            name: "add".into(),
            body: Box::new(lam(
                "n",
                g("Unary"),
                lam(
                    "m",
                    g("Unary"),
                    Term::Match {
                        discriminant: Box::new(v("n")),
                        motive: Box::new(lam("n", g("Unary"), g("Unary"))),
                        cases: vec![
                            v("m"),
                            lam(
                                "p",
                                g("Unary"),
                                app(g("Unary.next"), app(app(v("add"), v("p")), v("m"))),
                            ),
                        ],
                    },
                ),
            )),
        },
    )
    .unwrap();
    c.declare(
        "unary.count",
        arrow(g("Unary"), g("Unary")),
        Term::Fix {
            name: "count".into(),
            body: Box::new(lam(
                "n",
                g("Unary"),
                Term::Match {
                    discriminant: Box::new(v("n")),
                    motive: Box::new(lam("n", g("Unary"), g("Unary"))),
                    cases: vec![
                        g("Unary.zero"),
                        lam(
                            "p",
                            g("Unary"),
                            app(
                                app(g("unary.add"), app(v("count"), v("p"))),
                                app(g("Unary.next"), g("Unary.zero")),
                            ),
                        ),
                    ],
                },
            )),
        },
    )
    .unwrap();
    c
}

#[test]
fn unary_constructor_runs_share_repeated_checked_computations() {
    let c = unary_count_context();
    let numeral = |n| (0..n).fold(g("Unary.zero"), |p, _| app(g("Unary.next"), p));
    let left = app(g("unary.count"), numeral(256));
    let right = numeral(256);
    crate::infer_type(&c, &left).unwrap();
    assert!(constructor_equivalent(&c, &left, &right, 20_000));
    assert!(!constructor_equivalent(&c, &left, &numeral(255), 20_000));
}

#[test]
fn unary_constructor_runs_retain_head_identity_and_every_tail_observation() {
    let mut c = data_context();
    let run = |n, tail| (0..n).fold(tail, |p, _| app(g("Data.next"), p));
    let a = app(g("data.id"), run(128, g("Data.zero")));
    let b = run(128, g("Data.zero"));
    assert!(constructor_equivalent(&c, &a, &b, 20_000));
    let wrong = run(
        127,
        app(app(g("Data.pair"), g("Data.zero")), g("Data.zero")),
    );
    assert!(!constructor_equivalent(&c, &a, &wrong, 20_000));
    c.add_inductive("Witness", Term::Sort(crate::Universe::Prop));
    c.add_constructor("Witness.mk", "Witness", g("Witness"));
    c.add_constructor(
        "Data.proof",
        "Data",
        Term::Pi {
            param: "p".into(),
            param_type: Box::new(g("Witness")),
            body_type: Box::new(g("Data")),
        },
    );
    let proof_tail = run(128, app(g("Data.proof"), g("Witness.mk")));
    assert!(!constructor_equivalent(
        &c,
        &proof_tail,
        &proof_tail,
        20_000
    ));
    c.add_constructor(
        "Data.function",
        "Data",
        Term::Pi {
            param: "f".into(),
            param_type: Box::new(Term::Pi {
                param: "x".into(),
                param_type: Box::new(g("Data")),
                body_type: Box::new(g("Data")),
            }),
            body_type: Box::new(g("Data")),
        },
    );
    let function_tail = run(128, app(g("Data.function"), lam("x", g("Data"), v("x"))));
    assert!(!constructor_equivalent(
        &c,
        &function_tail,
        &function_tail,
        20_000
    ));
}

#[test]
fn unary_constructor_runs_preserve_universe_instances() {
    let mut c = Context::new();
    let u = crate::Universe::Var("u".into());
    let tower = |level| Term::Const {
        name: "Tower".into(),
        levels: vec![level],
    };
    c.add_universe_poly_inductive("Tower", vec!["u".into()], Term::Sort(u.clone()), 0);
    c.add_constructor("Tower.zero", "Tower", tower(u.clone()));
    c.add_constructor(
        "Tower.next",
        "Tower",
        Term::Pi {
            param: "p".into(),
            param_type: Box::new(tower(u.clone())),
            body_type: Box::new(tower(u)),
        },
    );
    let run = |level: crate::Universe| {
        let head = Term::Const {
            name: "Tower.next".into(),
            levels: vec![level.clone()],
        };
        (0..32).fold(
            Term::Const {
                name: "Tower.zero".into(),
                levels: vec![level],
            },
            |p, _| app(head.clone(), p),
        )
    };
    let zero = crate::Universe::Type(0);
    let value = run(zero.clone());
    let delayed = app(lam("p", tower(zero), v("p")), value.clone());
    crate::infer_type(&c, &delayed).unwrap();
    assert!(constructor_equivalent(&c, &delayed, &value, 10_000));
    let other = run(crate::Universe::Type(1));
    crate::infer_type(&c, &other).unwrap();
    assert!(!constructor_equivalent(&c, &delayed, &other, 10_000));
}

#[test]
fn stable_unary_keys_do_not_restart_completed_work() {
    let c = unary_count_context();
    let mut r = HeadReducer::new(&c, 100_000);
    let zero = r.store.mk_global("Unary.zero");
    let next = r.store.mk_global("Unary.next");
    assert!(r.constructor(next).is_some());
    let tail = r
        .thunk(Thunk::Done(Value::Constructor {
            head: zero,
            args: vec![],
        }))
        .unwrap();
    let run = r
        .thunk(Thunk::Done(Value::UnaryRun {
            head: next,
            count: 4096,
            tail,
        }))
        .unwrap();
    let (expected, stable) = r.binding_key_state(&run, 32, &mut 256);
    assert!(stable, "the entire immutable run has already been observed");
    for _ in 0..64 {
        let mut remaining = 0;
        let (key, stable) = r.binding_key_state(&run, 0, &mut remaining);
        assert!(
            stable,
            "a completed value must not restart traversal to answer the same key"
        );
        assert_eq!(key, expected);
        assert_eq!(remaining, 0);
    }
}

#[test]
fn unresolved_unary_tail_keys_can_still_mature() {
    let c = unary_count_context();
    let mut r = HeadReducer::new(&c, 100_000);
    let next = r.store.mk_global("Unary.next");
    assert!(r.constructor(next).is_some());
    let expression = r
        .store
        .intern(&app(lam("x", g("Unary"), v("x")), g("Unary.zero")));
    let tail = r.delay(expression, &None).unwrap();
    let run = r
        .thunk(Thunk::Done(Value::UnaryRun {
            head: next,
            count: 7,
            tail: tail.clone(),
        }))
        .unwrap();
    assert!(
        !r.binding_key_state(&run, 32, &mut 256).1,
        "a suspended tail must not be frozen as a completed semantic key"
    );
    assert!(r.force(tail).is_some());
    assert!(
        r.binding_key_state(&run, 32, &mut 256).1,
        "the same run becomes stable only after its tail finishes"
    );
}
#[test]
fn lexical_capture_lookup_work_is_bounded_by_symbol_width() {
    let c = Context::new();
    let mut reducer = HeadReducer::new(&c, 10_000);
    let mut env = None;
    let mut bindings = Vec::new();
    for i in 0..512 {
        let name = reducer.store.symbol(&format!("local.{i}"));
        let value = reducer.thunk(Thunk::Done(Value::Neutral)).unwrap();
        env = bind(env, name, value.clone());
        bindings.push((name, value));
    }
    ENV_LOOKUP_STEPS.with(|steps| steps.set(0));
    for (name, expected) in &bindings {
        let actual = lookup(&env, *name).expect("bound lexical value");
        assert!(Rc::ptr_eq(&actual, expected));
    }
    let work = ENV_LOOKUP_STEPS.with(Cell::get);
    assert!(
        work <= 512 * 32,
        "512 captures traversed {work} environment entries"
    );
}

#[test]
fn indexed_environment_preserves_branching_shadowing_and_thunk_identity() {
    let c = Context::new();
    let mut reducer = HeadReducer::new(&c, 10_000);
    let name = reducer.store.symbol("shadowed");
    let other = reducer.store.symbol("other");
    let absent = reducer.store.symbol("absent");
    let first = reducer.thunk(Thunk::Done(Value::Neutral)).unwrap();
    let second = reducer.thunk(Thunk::Done(Value::Neutral)).unwrap();
    let third = reducer.thunk(Thunk::Done(Value::Neutral)).unwrap();
    let base = bind(None, name, first.clone());
    let shadow = bind(base.clone(), name, second.clone());
    let sibling = bind(base.clone(), other, third.clone());
    assert!(Rc::ptr_eq(&lookup(&base, name).unwrap(), &first));
    assert!(Rc::ptr_eq(&lookup(&shadow, name).unwrap(), &second));
    assert!(Rc::ptr_eq(&lookup(&sibling, name).unwrap(), &first));
    assert!(Rc::ptr_eq(&lookup(&sibling, other).unwrap(), &third));
    assert!(lookup(&shadow, other).is_none());
    assert!(lookup(&sibling, absent).is_none());
    drop(base);
    drop(shadow);
    assert!(Rc::ptr_eq(&lookup(&sibling, name).unwrap(), &first));
}

#[test]
fn persistent_environment_versions_match_an_independent_map() {
    let c = Context::new();
    let mut reducer = HeadReducer::new(&c, 10_000);
    let names = (0..65)
        .map(|i| reducer.store.symbol(&format!("key.{i}")))
        .collect::<Vec<_>>();
    let mut versions = vec![(None, HashMap::<Symbol, Lazy>::new())];
    let mut seed = 0xabcdu64;
    for _ in 0..512 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let parent = (seed as usize) % versions.len();
        let name = names[((seed >> 32) as usize) % 64];
        let value = reducer.thunk(Thunk::Done(Value::Neutral)).unwrap();
        let env = bind(versions[parent].0.clone(), name, value.clone());
        let mut expected = versions[parent].1.clone();
        expected.insert(name, value);
        for candidate in &names {
            match (lookup(&env, *candidate), expected.get(candidate)) {
                (Some(actual), Some(expected)) => assert!(Rc::ptr_eq(&actual, expected)),
                (None, None) => {}
                _ => panic!("persistent lexical environment disagreed with map"),
            }
        }
        versions.push((env, expected));
    }
    while let Some((env, expected)) = versions.pop() {
        for (name, value) in expected {
            assert!(Rc::ptr_eq(&lookup(&env, name).unwrap(), &value));
        }
    }
}

fn measured_identity_chain(shared: bool, count: usize) -> (Value, super::super::DagAtomWork, u64) {
    let mut ctx = env();
    let identity = lam("n", g("Nat"), v("n"));
    let head = if shared {
        let ty = crate::infer_type(&ctx, &identity).unwrap();
        ctx.declare("shared_identity", ty, identity.clone())
            .unwrap();
        g("shared_identity")
    } else {
        identity
    };
    let mut term = Term::Lit(Literal::Nat(crate::BigInt::from_i64(7)));
    for _ in 0..count {
        term = app(head.clone(), term);
    }
    crate::infer_type(&ctx, &term).unwrap();
    let mut reducer = HeadReducerWithWork::<true>::new(&ctx, 10000);
    reducer.work = Some(Default::default());
    let root = reducer.store.intern(&term);
    let value = reducer.whnf(root).expect("checked identity chain computes");
    (value, reducer.work.unwrap(), reducer.fuel)
}

#[test]
fn cached_lambda_application_avoids_ready_value_rewrites() {
    let (value, work, _) = measured_identity_chain(true, 64);
    assert!(matches!(value, Value::Nat(n) if n == crate::BigInt::from_i64(7)));
    assert!(
        work.ready_value_rewrites <= 2,
        "immutable cached lambdas were rewritten {} times",
        work.ready_value_rewrites
    );
}

#[test]
fn direct_lambda_application_avoids_transient_closure_values() {
    let (value, work, _) = measured_identity_chain(false, 64);
    assert!(matches!(value, Value::Nat(n) if n == crate::BigInt::from_i64(7)));
    assert_eq!(
        work.lambda_values, 0,
        "directly applied lambdas need no returned closure"
    );
}

#[test]
fn direct_application_work_reconciles_dispatch_and_semantic_counts() {
    let (value, work, remaining) = measured_identity_chain(true, 16);
    assert!(matches!(value, Value::Nat(n) if n == crate::BigInt::from_i64(7)));
    assert_eq!(
        work.eval + work.compute + work.force + work.returned,
        10000 - remaining
    );
    assert_eq!(
        work.force_ready + work.force_suspended + work.force_blocked,
        work.force
    );
    assert_eq!(
        work.return_update
            + work.return_apply
            + work.return_unroll
            + work.return_match
            + work.return_finish_unary
            + work.return_complete
            + work.return_unary_demand,
        work.returned
    );
    assert_eq!(work.beta, 16);
}

#[test]
fn optional_work_counters_preserve_result_shapes_and_fuel() {
    fn run<const TRACE: bool>(ctx: &Context, term: &Term, fuel: u64) -> (String, u64, usize) {
        let mut r = HeadReducerWithWork::<TRACE>::new(ctx, fuel);
        r.work = Some(Default::default());
        let root = r.store.intern(term);
        let shape = match r.whnf(root) {
            None => "inconclusive".into(),
            Some(Value::Nat(n)) => format!("Nat:{n:?}"),
            Some(Value::Neutral) => "neutral".into(),
            Some(Value::Constructor { head, args }) => {
                format!("constructor:{:?}/{}", r.store.extract(head), args.len())
            }
            Some(Value::Lambda {
                param,
                param_type,
                body,
                ..
            }) => format!(
                "lambda:{:?}:{:?}:{:?}",
                r.store.name(param),
                r.store.extract(param_type),
                r.store.extract(body)
            ),
            Some(Value::Fix { id, args, .. }) => {
                format!("fix:{:?}/{}", r.store.extract(id), args.len())
            }
            Some(Value::UnaryRun { head, count, .. }) => {
                format!("unary:{:?}/{count}", r.store.extract(head))
            }
        };
        if !TRACE {
            assert_eq!(r.work.unwrap(), Default::default());
        }
        (shape, r.fuel, r.store.len())
    }
    let c = env();
    let terms = [
        g("true"),
        g("false"),
        g("Zero"),
        Term::Lit(Literal::Nat(crate::BigInt::from_i64(19))),
        g("unknown"),
        Term::Hole,
        v("unbound"),
        lam("x", g("Nat"), v("x")),
        recursive(),
        app(g("Succ"), g("Zero")),
        app(lam("x", g("Nat"), v("x")), g("Zero")),
        app(recursive(), app(g("Succ"), g("Zero"))),
        app(
            app(lam("x", g("Bool"), lam("x", g("Bool"), v("x"))), g("true")),
            g("false"),
        ),
        Term::Const {
            name: "Nat".into(),
            levels: vec![crate::Universe::Type(0)],
        },
    ];
    for term in terms {
        for fuel in [0, 1, 8, 10000] {
            assert_eq!(
                run::<false>(&c, &term, fuel),
                run::<true>(&c, &term, fuel),
                "counter modes differ for {term:?} with fuel {fuel}"
            );
        }
    }
}

#[test]
fn directly_demanded_beta_bodies_avoid_suspended_construction() {
    let ctx = env();
    let mut term = Term::Lit(Literal::Nat(crate::BigInt::from_i64(7)));
    for _ in 0..64 {
        term = app(lam("unused", g("Nat"), term), g("Zero"));
    }
    crate::infer_type(&ctx, &term).unwrap();
    let mut reducer = HeadReducerWithWork::<true>::new(&ctx, 10000);
    reducer.work = Some(Default::default());
    let root = reducer.store.intern(&term);
    assert!(matches!(reducer.whnf(root),
        Some(Value::Nat(n)) if n == crate::BigInt::from_i64(7)));
    let work = reducer.work.unwrap();
    assert_eq!(work.beta, 64);
    assert_eq!(
        work.demanded_fresh_suspended, 0,
        "immediately demanded bodies must not construct temporary Suspended states"
    );
    assert_eq!(
        work.suspended_constructions, 0,
        "this closed unused-argument chain never needs a lazy argument"
    );
}

#[test]
fn demanding_an_existing_suspension_preserves_its_memo_identity() {
    let ctx = env();
    let term = app(
        lam("x", g("Nat"), v("x")),
        Term::Lit(Literal::Nat(crate::BigInt::from_i64(7))),
    );
    crate::infer_type(&ctx, &term).unwrap();
    let mut reducer = HeadReducerWithWork::<true>::new(&ctx, 10000);
    reducer.work = Some(Default::default());
    let root = reducer.store.intern(&term);
    let original = reducer.delay(root, &None).unwrap();
    assert!(matches!(&*original.state.borrow(), Thunk::Suspended(..)));
    assert!(matches!(reducer.whnf(root),
        Some(Value::Nat(n)) if n == crate::BigInt::from_i64(7)));
    let again = reducer.delay(root, &None).unwrap();
    assert!(Rc::ptr_eq(&original, &again));
    assert!(matches!(&*original.state.borrow(),
        Thunk::Done(Value::Nat(n)) if *n == crate::BigInt::from_i64(7)));
    assert_eq!(reducer.work.unwrap().demanded_cached_suspended, 1);
}

#[test]
fn demanded_running_memo_hits_remain_inconclusive() {
    let ctx = env();
    let mut reducer = HeadReducerWithWork::<true>::new(&ctx, 10000);
    reducer.work = Some(Default::default());
    let root = reducer.store.intern(&g("true"));
    let running = reducer.thunk(Thunk::Running).unwrap();
    reducer.closures.insert((root, Vec::new()), running.clone());
    assert!(reducer.whnf(root).is_none());
    let again = reducer.delay(root, &None).unwrap();
    assert!(Rc::ptr_eq(&running, &again));
    assert!(matches!(&*running.state.borrow(), Thunk::Running));
    assert_eq!(reducer.work.unwrap().force_blocked, 1);
}

#[test]
fn delayed_lambda_is_ready_without_observing_its_opaque_body() {
    let mut ctx = env();
    ctx.add_declaration("opaque_nat", g("Nat"));
    let lambda = lam("x", g("Nat"), g("opaque_nat"));
    crate::infer_type(&ctx, &lambda).unwrap();
    let mut reducer = HeadReducerWithWork::<true>::new(&ctx, 10000);
    reducer.work = Some(Default::default());
    let root = reducer.store.intern(&lambda);
    let delayed = reducer.delay(root, &None).unwrap();
    assert!(
        matches!(&*delayed.state.borrow(), Thunk::Done(Value::Lambda { .. })),
        "a syntactic lambda is a value; its body must remain unobserved"
    );
    assert_eq!(reducer.fuel, 10000);
    assert_eq!(reducer.work.unwrap().suspended_lambda_constructions, 0);
    assert_eq!(reducer.work.unwrap().compute, 0);
    let again = reducer.delay(root, &None).unwrap();
    assert!(Rc::ptr_eq(&delayed, &again));
}

#[test]
fn ready_lambda_arguments_keep_distinct_captured_environments() {
    let ctx = env();
    let lambda = lam("y", g("Nat"), v("outer"));
    crate::infer_type(&ctx.extend("outer", g("Nat")), &lambda).unwrap();
    let mut reducer = HeadReducerWithWork::<true>::new(&ctx, 10000);
    reducer.work = Some(Default::default());
    let root = reducer.store.intern(&lambda);
    let outer = reducer.store.intern(&v("outer"));
    let Node::Var(outer) = *reducer.store.node(outer) else {
        unreachable!()
    };
    let f = reducer.store.intern(&v("f"));
    let Node::Var(f) = *reducer.store.node(f) else {
        unreachable!()
    };
    let application = reducer.store.intern(&app(v("f"), g("Zero")));
    let seven = reducer.ready_nat(crate::BigInt::from_i64(7)).unwrap();
    let nine = reducer.ready_nat(crate::BigInt::from_i64(9)).unwrap();
    let first = reducer.delay(root, &bind(None, outer, seven)).unwrap();
    let second = reducer.delay(root, &bind(None, outer, nine)).unwrap();
    assert!(!Rc::ptr_eq(&first, &second));
    for (function, expected) in [(first, 7), (second, 9)] {
        assert!(
            matches!(reducer.run(Control::Eval(application, bind(None, f, function))),
            Some(Value::Nat(n)) if n == crate::BigInt::from_i64(expected))
        );
    }
}

#[test]
fn open_conversion_gate_does_not_materialize_cached_free_names() {
    let ctx = env();
    let mut term = g("opaque_function");
    for i in 0..64 {
        term = app(term, v(&format!("free{i}")));
    }
    crate::term_store::reset_free_name_work();
    assert!(!constructor_equivalent(&ctx, &term, &g("Zero"), 10_000));
    let work = crate::term_store::free_name_work();
    eprintln!("actual free-name collections/resolutions at open conversion gate: {work:?}");
    assert_eq!(
        work,
        (0, 0),
        "refusing an open conversion needs only cached exact emptiness"
    );
}
