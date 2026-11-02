use logicaffeine_kernel::interface::lean as adapter;
use logicaffeine_kernel::{infer_type, is_subtype, recheck, Context, Term, Universe};
use serde_json::json;

fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn v(n: &str) -> Term {
    Term::Var(n.into())
}
fn app(f: Term, a: Term) -> Term {
    Term::App(Box::new(f), Box::new(a))
}
fn pi(n: &str, a: Term, b: Term) -> Term {
    Term::Pi {
        param: n.into(),
        param_type: Box::new(a),
        body_type: Box::new(b),
    }
}
fn let_(n: &str, a: Term, value: Term, body: Term) -> Term {
    Term::Let {
        name: n.into(),
        ty: Box::new(a),
        value: Box::new(value),
        body: Box::new(body),
    }
}
fn env() -> Context {
    let mut c = Context::new();
    c.add_declaration("A", Term::Sort(Universe::Type(0)));
    c.add_declaration("B", Term::Sort(Universe::Type(0)));
    c.add_declaration("a", g("A"));
    c.add_declaration("b", g("B"));
    c.add_declaration("P", pi("x", g("A"), Term::Sort(Universe::Type(0))));
    c.add_declaration("q", app(g("P"), g("a")));
    c
}
fn accepted(c: &Context, t: &Term, expected: &Term) {
    let main = infer_type(c, t).expect("primary checker");
    let independent = recheck(c, t).expect("independent checker");
    let imported = adapter::lean_type(c, t).expect("Lean exact checker");
    for ty in [main, independent, imported] {
        assert!(is_subtype(c, &ty, expected), "{ty:?} != {expected:?}");
    }
}
#[test]
fn dependent_let_falls_back_when_body_conversion_needs_its_value() {
    let c = env();
    let body = app(
        Term::Lambda {
            param: "proof".into(),
            param_type: Box::new(app(g("P"), v("x"))),
            body: Box::new(v("proof")),
        },
        g("q"),
    );
    assert!(
        infer_type(&c.extend("x", g("A")), &body).is_err(),
        "this control must exercise transparent-value conversion"
    );
    accepted(&c, &let_("x", g("A"), g("a"), body), &app(g("P"), g("a")));
}
#[test]
fn unused_ill_typed_let_values_remain_rejected_by_all_checkers() {
    let c = env();
    let t = let_("x", g("A"), g("b"), g("a"));
    assert!(infer_type(&c, &t).is_err());
    assert!(recheck(&c, &t).is_err());
    assert!(adapter::lean_type(&c, &t).is_err());
}
#[test]
fn non_type_let_annotations_remain_rejected_by_all_checkers() {
    let c = env();
    let t = let_("x", g("a"), g("a"), g("a"));
    assert!(infer_type(&c, &t).is_err());
    assert!(recheck(&c, &t).is_err());
    assert!(adapter::lean_type(&c, &t).is_err());
}
#[test]
fn shadowed_let_value_uses_outer_binding_and_body_uses_inner_binding() {
    let c = env();
    accepted(
        &c,
        &let_("x", g("A"), g("a"), let_("x", g("A"), v("x"), v("x"))),
        &g("A"),
    );
    accepted(
        &c,
        &let_("x", g("A"), g("a"), let_("x", g("B"), g("b"), v("x"))),
        &g("B"),
    );
}
#[test]
fn dependent_result_type_substitutes_the_let_value() {
    let c = env();
    let body = Term::Lambda {
        param: "proof".into(),
        param_type: Box::new(app(g("P"), v("x"))),
        body: Box::new(v("proof")),
    };
    accepted(
        &c,
        &let_("x", g("A"), g("a"), body),
        &pi("proof", app(g("P"), g("a")), app(g("P"), g("a"))),
    );
}
fn chain(depth: usize) -> String {
    let mut rows = vec![
        json!({"meta":{"format":{"version":"3.1.0"}}}),
        json!({"il":1,"succ":0}),
        json!({"ie":0,"sort":1}),
    ];
    for (id, name) in [(1, "A"), (2, "a"), (3, "f"), (4, "chain")] {
        rows.push(json!({"in":id,"str":{"pre":0,"str":name}}));
    }
    rows.extend([
        json!({"axiom":{"name":1,"type":0,"levelParams":[]}}),
        json!({"ie":1,"const":{"name":1,"us":[]}}),
        json!({"axiom":{"name":2,"type":1,"levelParams":[]}}),
        json!({"ie":2,"const":{"name":2,"us":[]}}),
        json!({"ie":3,"forallE":{"name":0,"type":1,"body":1,"binderInfo":"default"}}),
        json!({"axiom":{"name":3,"type":3,"levelParams":[]}}),
        json!({"ie":4,"const":{"name":3,"us":[]}}),
        json!({"ie":5,"bvar":0}),
        json!({"ie":6,"app":{"fn":4,"arg":5}}),
    ]);
    let mut body = 5;
    for k in (0..depth).rev() {
        let id = 7 + depth - 1 - k;
        rows.push(json!({"ie":id,"letE":{"name":0,"type":1,
            "value":if k==0 {2}else{6},"body":body,"nondep":false}}));
        body = id;
    }
    rows.push(
        json!({"def":{"name":4,"type":1,"value":body,"levelParams":[],
        "all":[4],"safety":"safe","hints":"opaque"}}),
    );
    rows.into_iter().map(|r| r.to_string() + "\n").collect()
}
fn assert_compact_chain(depth: usize) {
    let (ctx, _) = adapter::import_audited(&chain(depth)).expect("audited chain");
    let mut body = ctx
        .get_definition_body("/s:\"chain\"")
        .expect("retained body");
    let mut previous = None;
    for k in 0..depth {
        match body {
            Term::Let {
                name,
                value,
                body: next,
                ..
            } => {
                if k == 0 {
                    assert!(matches!(&**value, Term::Global(_)));
                } else {
                    match &**value {
                        Term::App(_, arg) => assert_eq!(
                            &**arg,
                            &Term::Var(previous.clone().unwrap()),
                            "let references must stay references, never expanded value trees"
                        ),
                        other => panic!("expected application: {other:?}"),
                    }
                }
                previous = Some(name.clone());
                body = next;
            }
            other => panic!("expected let {k}: {other:?}"),
        }
    }
    assert_eq!(body, &Term::Var(previous.unwrap()));
    // ImportedContext owns deep-term destruction on its measured stack.
    drop(ctx);
}
#[test]
fn imported_let_dag_retains_compact_references() {
    assert_compact_chain(24);
}
#[test]
fn deep_retained_lets_are_checked_and_dropped_on_a_small_caller_stack() {
    std::thread::Builder::new()
        .stack_size(128 * 1024)
        .spawn(|| assert_compact_chain(512))
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn nested_dependent_lets_fall_back_and_restore_probe_state_after_rejection() {
    let mut c = env();
    c.add_declaration("a2", g("A"));
    c.add_declaration("q2", app(g("P"), g("a2")));
    let nested = |proof: &str| {
        let mut body = app(
            Term::Lambda {
                param: "proof".into(),
                param_type: Box::new(app(g("P"), v("x11"))),
                body: Box::new(v("proof")),
            },
            g(proof),
        );
        for i in (0..12).rev() {
            body = let_(
                &format!("x{i}"),
                g("A"),
                if i == 0 {
                    g("a")
                } else {
                    v(&format!("x{}", i - 1))
                },
                body,
            );
        }
        body
    };
    accepted(&c, &nested("q"), &app(g("P"), g("a")));
    let bad = nested("q2");
    assert!(infer_type(&c, &bad).is_err());
    assert!(recheck(&c, &bad).is_err());
    assert!(adapter::lean_type(&c, &bad).is_err());
    accepted(&c, &nested("q"), &app(g("P"), g("a")));
}

#[test]
fn projection_through_a_local_type_alias_is_lowered_and_checked() {
    // Retain the existing checked Prod environment, then append
    // let T : Type := Prod Bool Bool; fun x : T => x.1.
    let mut input = include_str!("fixtures/lean_arena/098_projRed.ndjson").to_owned();
    for row in [
        json!({"in":40,"str":{"pre":0,"str":"hiddenStructure"}}),
        json!({"in":41,"str":{"pre":0,"str":"T"}}),
        json!({"in":42,"str":{"pre":0,"str":"x"}}),
        json!({"ie":200,"const":{"name":21,"us":[0,0]}}),
        json!({"ie":201,"app":{"fn":200,"arg":43}}),
        json!({"ie":202,"app":{"fn":201,"arg":43}}),
        json!({"ie":203,"forallE":{"name":42,"type":202,"body":43,"binderInfo":"default"}}),
        json!({"ie":204,"proj":{"typeName":21,"idx":0,"struct":1}}),
        json!({"ie":205,"lam":{"name":42,"type":1,"body":204,"binderInfo":"default"}}),
        json!({"ie":206,"letE":{"name":41,"type":42,"value":202,"body":205,"nondep":false}}),
        json!({"def":{"name":40,"type":203,"value":206,"levelParams":[],
            "all":[40],"safety":"safe","hints":"opaque"}}),
    ] {
        input.push_str(&(row.to_string() + "\n"));
    }
    let (ctx, report) =
        adapter::import_audited(&input).expect("checked hidden structure projection");
    let n = r#"/s:"hiddenStructure""#;
    let body = ctx.get_definition_body(n).unwrap();
    assert!(
        matches!(body, Term::Let { .. }),
        "preserve the checked source let"
    );
    let ty = ctx.get_definition_type(n).unwrap();
    accepted(&ctx, body, ty);
    let mut pair = Term::Const {
        name: r#"/s:"Prod"/s:"mk""#.into(),
        levels: vec![Universe::Prop, Universe::Prop],
    };
    for a in [
        g(r#"/s:"Bool""#),
        g(r#"/s:"Bool""#),
        g(r#"/s:"Bool"/s:"true""#),
        g(r#"/s:"Bool"/s:"false""#),
    ] {
        pair = app(pair, a);
    }
    let result = logicaffeine_kernel::normalize_checked(&ctx, &app(g(n), pair)).unwrap();
    assert!(logicaffeine_kernel::defeq_for_test(
        &ctx,
        &result,
        &g(r#"/s:"Bool"/s:"true""#)
    ));
    let audit = report.declarations.iter().find(|d| d.name == n).unwrap();
    assert!(audit.axioms.is_empty());
}

/// An export over `A : Type`, `a : A`, `f : A → A`, `g : A → A → A` whose one definition `name` has
/// the body `lets` builds from those constants (ids `A`=1, `a`=2, `f`=4, `g`=6, `bvar 0`=7).
fn over_a(
    name: &str,
    lets: impl FnOnce(&mut Vec<serde_json::Value>, &mut usize) -> usize,
) -> String {
    let mut rows = vec![
        json!({"meta":{"format":{"version":"3.1.0"}}}),
        json!({"il":1,"succ":0}),
        json!({"ie":0,"sort":1}),
    ];
    for (id, label) in [(1, "A"), (2, "a"), (3, "f"), (4, "g"), (5, name)] {
        rows.push(json!({"in":id,"str":{"pre":0,"str":label}}));
    }
    rows.extend([
        json!({"axiom":{"name":1,"type":0,"levelParams":[]}}),
        json!({"ie":1,"const":{"name":1,"us":[]}}),
        json!({"axiom":{"name":2,"type":1,"levelParams":[]}}),
        json!({"ie":2,"const":{"name":2,"us":[]}}),
        json!({"ie":3,"forallE":{"name":0,"type":1,"body":1,"binderInfo":"default"}}),
        json!({"axiom":{"name":3,"type":3,"levelParams":[]}}),
        json!({"ie":4,"const":{"name":3,"us":[]}}),
        json!({"ie":5,"forallE":{"name":0,"type":1,"body":3,"binderInfo":"default"}}),
        json!({"axiom":{"name":4,"type":5,"levelParams":[]}}),
        json!({"ie":6,"const":{"name":4,"us":[]}}),
        json!({"ie":7,"bvar":0}),
    ]);
    let mut next = 8;
    let body = lets(&mut rows, &mut next);
    rows.push(
        json!({"def":{"name":5,"type":1,"value":body,"levelParams":[],
        "all":[5],"safety":"safe","hints":"opaque"}}),
    );
    rows.into_iter().map(|r| r.to_string() + "\n").collect()
}
fn row(
    rows: &mut Vec<serde_json::Value>,
    next: &mut usize,
    kind: &str,
    value: serde_json::Value,
) -> usize {
    let id = *next;
    *next += 1;
    rows.push(json!({"ie":id,kind:value}));
    id
}
fn let_row(
    rows: &mut Vec<serde_json::Value>,
    next: &mut usize,
    value: usize,
    body: usize,
) -> usize {
    row(
        rows,
        next,
        "letE",
        json!({"name":0,"type":1,"value":value,"body":body,"nondep":false}),
    )
}
/// Walk a let spine: each let's value and the term under the last let.
fn let_spine(mut body: &Term) -> (Vec<(&str, &Term)>, &Term) {
    let mut spine = Vec::new();
    while let Term::Let {
        name,
        value,
        body: next,
        ..
    } = body
    {
        spine.push((name.as_str(), &**value));
        body = next;
    }
    (spine, body)
}

/// `x₀ := a` is used twice (`x₁ := g x₀ x₀`), every later `xₖ := f xₖ₋₁` once. The variable used
/// twice shares its checked closed value, so `x₁` reads `g a a`; every let used once stays a let
/// whose value refers to its predecessor, down to the body's reference to the last.
#[test]
fn a_let_used_twice_shares_its_value_and_every_let_used_once_stays_a_reference() {
    let depth = 512;
    let input = over_a("twice", |rows, next| {
        let f_x = row(rows, next, "app", json!({"fn":4,"arg":7}));
        let g_x = row(rows, next, "app", json!({"fn":6,"arg":7}));
        let g_x_x = row(rows, next, "app", json!({"fn":g_x,"arg":7}));
        let mut body = 7;
        for k in (0..depth).rev() {
            body = let_row(
                rows,
                next,
                match k {
                    0 => 2,
                    1 => g_x_x,
                    _ => f_x,
                },
                body,
            );
        }
        body
    });
    std::thread::Builder::new()
        .stack_size(128 * 1024)
        .spawn(move || {
            let (ctx, _) = adapter::import_audited(&input).expect("audited chain");
            let (spine, last) = let_spine(
                ctx.get_definition_body("/s:\"twice\"")
                    .expect("retained body"),
            );
            assert_eq!(
                spine.len(),
                depth - 1,
                "only the let used twice is shared through its value"
            );
            assert_eq!(
                spine[0].1,
                &app(app(g("/s:\"g\""), g("/s:\"a\"")), g("/s:\"a\"")),
                "x₁ reads the shared value of x₀"
            );
            for window in spine.windows(2) {
                assert_eq!(
                    window[1].1,
                    &app(g("/s:\"f\""), v(window[0].0)),
                    "a let used once stays a reference"
                );
            }
            assert_eq!(last, &v(spine[depth - 2].0));
            drop(ctx);
        })
        .unwrap()
        .join()
        .unwrap();
}

/// `n` closed lets `xᵢ := a`, each used exactly once, in the spine `g xₙ₋₁ (… (g x₀ a))` under all
/// of them. Within the kernel's depth door the lets stay lets and the spine refers to them; past
/// the door their checked values are substituted, so the spine closes and is cut into shallow
/// checked helpers — a let kept there would leave an open spine deeper than the kernel admits.
fn spine_under_single_use_lets(n: usize) -> String {
    over_a("spine", |rows, next| {
        let bvars: Vec<usize> = (0..n).map(|i| row(rows, next, "bvar", json!(i))).collect();
        let mut spine = 2;
        for bvar in bvars.iter().rev() {
            let partial = row(rows, next, "app", json!({"fn":6,"arg":*bvar}));
            spine = row(rows, next, "app", json!({"fn":partial,"arg":spine}));
        }
        let mut body = spine;
        for _ in 0..n {
            body = let_row(rows, next, 2, body);
        }
        body
    })
}
#[test]
fn single_use_lets_within_the_depth_door_stay_lets() {
    // 700 lets over a 700-deep spine: 1,400 source rungs, inside the kernel's door of 1,600.
    let n = 700;
    let (ctx, report) =
        adapter::import_audited(&spine_under_single_use_lets(n)).expect("retained spine");
    let (spine, mut body) = let_spine(
        ctx.get_definition_body("/s:\"spine\"")
            .expect("retained body"),
    );
    assert_eq!(spine.len(), n, "every single-use let stays");
    assert!(spine.iter().all(|(_, value)| *value == &g("/s:\"a\"")));
    for (name, _) in spine.iter().rev() {
        let Term::App(partial, rest) = body else {
            panic!("spine application")
        };
        assert_eq!(
            &**partial,
            &app(g("/s:\"g\""), v(name)),
            "the spine refers to the let"
        );
        body = rest;
    }
    assert_eq!(body, &g("/s:\"a\""));
    assert!(report.internal_helpers.is_empty(), "nothing needed cutting");
}
#[test]
fn single_use_lets_past_the_depth_door_are_lowered_so_the_spine_is_cut() {
    // 900 lets over a 900-deep spine: 1,800 source rungs, past the kernel's door of 1,600.
    let (ctx, report) =
        adapter::import_audited(&spine_under_single_use_lets(900)).unwrap_or_else(|e| {
            panic!("a spine past the depth door imports through checked helpers: {e:?}")
        });
    assert!(
        !report.internal_helpers.is_empty(),
        "the closed spine is cut into helpers"
    );
    let body = ctx
        .get_definition_body("/s:\"spine\"")
        .expect("retained body");
    assert!(
        logicaffeine_kernel::term_store::term_depth(body) < 900,
        "no let spine is left at the root"
    );
    drop(ctx);
}
