//! Fresh attestation boundaries: checked Bool aliases, legacy registry mutation,
//! and transitive delta transparency. A Bool alias is made before the orders are checked,
//! because rebinding `true` under them is a change of meaning the stable-meaning gate refuses.
use logicaffeine_kernel::{
    eval_bool, eval_bool_tree, native_compile_decide, normalize, normalize_at, BigInt, Context,
    Literal, Term, Transparency, Universe,
};

fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn v(n: &str) -> Term {
    Term::Var(n.into())
}
fn app(f: Term, x: Term) -> Term {
    Term::App(Box::new(f), Box::new(x))
}
fn bin(f: Term, x: Term, y: Term) -> Term {
    app(app(f, x), y)
}
fn nat(n: i64) -> Term {
    Term::Lit(Literal::Nat(BigInt::from_i64(n)))
}
fn lam(n: &str, ty: Term, body: Term) -> Term {
    Term::Lambda {
        param: n.into(),
        param_type: Box::new(ty),
        body: Box::new(body),
    }
}
fn arrow(a: Term, b: Term) -> Term {
    Term::Pi {
        param: "_".into(),
        param_type: Box::new(a),
        body_type: Box::new(b),
    }
}
fn mat(d: Term, cases: Vec<Term>) -> Term {
    Term::Match {
        discriminant: Box::new(d),
        motive: Box::new(lam("_", g("Nat"), g("Bool"))),
        cases,
    }
}
/// `Nat` and `Bool`, and nothing that depends on them yet.
fn carriers() -> Context {
    let mut ctx = Context::new();
    ctx.add_inductive("Nat", Term::Sort(Universe::Type(0)));
    ctx.add_constructor_checked("Zero", "Nat", g("Nat"))
        .unwrap();
    ctx.add_constructor_checked("Succ", "Nat", arrow(g("Nat"), g("Nat")))
        .unwrap();
    ctx.add_inductive("Bool", Term::Sort(Universe::Type(0)));
    ctx.add_constructor_checked("false", "Bool", g("Bool"))
        .unwrap();
    ctx.add_constructor_checked("true", "Bool", g("Bool"))
        .unwrap();
    ctx
}
fn context() -> Context {
    let mut ctx = carriers();
    declare_orders(&mut ctx);
    ctx
}
/// The canonical `natLe` and `natLt`, checked against whatever `true` and `false` mean here.
fn declare_orders(ctx: &mut Context) {
    let inner = mat(
        v("n"),
        vec![
            g("false"),
            lam("np", g("Nat"), bin(v("order"), v("mp"), v("np"))),
        ],
    );
    let outer = mat(v("m"), vec![g("true"), lam("mp", g("Nat"), inner)]);
    let le = Term::Fix {
        name: "order".into(),
        body: Box::new(lam("m", g("Nat"), lam("n", g("Nat"), outer))),
    };
    let ty = arrow(g("Nat"), arrow(g("Nat"), g("Bool")));
    ctx.declare("natLe", ty.clone(), le).unwrap();
    ctx.declare(
        "natLt",
        ty,
        lam(
            "m",
            g("Nat"),
            lam(
                "n",
                g("Nat"),
                bin(g("natLe"), app(g("Succ"), v("m")), v("n")),
            ),
        ),
    )
    .unwrap();
    assert!(ctx.pending_obligations().is_empty());
}
/// Rebinding `true` once the orders were checked against the constructor is a change of meaning
/// under them, which the stable-meaning gate refuses.
fn assert_true_is_kept(ctx: &mut Context) {
    assert!(
        matches!(
            ctx.declare("true", g("Bool"), g("false")),
            Err(logicaffeine_kernel::KernelError::StableMeaningViolation { .. })
        ),
        "`true` cannot be rebound under the orders checked against it"
    );
}
fn recheck_orders(ctx: &mut Context) {
    // The changed environment must still accept the actual definition bodies.
    for name in ["natLe", "natLt"] {
        let ty = ctx.get_definition_type(name).unwrap().clone();
        let body = ctx.get_definition_body(name).unwrap().clone();
        ctx.declare(name, ty, body).unwrap();
    }
}
fn assert_evaluators(ctx: &Context, t: &Term, expected: bool) {
    assert_eq!(eval_bool_tree(ctx, t), Some(expected), "tree evaluator");
    assert_eq!(
        native_compile_decide(ctx, t),
        Some(expected),
        "compiled evaluator"
    );
    assert_eq!(eval_bool(ctx, t), Some(expected), "public evaluator");
}

#[test]
fn canonical_checked_orders_still_compute_in_all_engines() {
    let ctx = context();
    for (op, x, y, expected) in [
        ("natLe", 2, 3, true),
        ("natLe", 3, 2, false),
        ("natLt", 2, 2, false),
        ("natLt", 2, 3, true),
    ] {
        let t = bin(g(op), nat(x), nat(y));
        assert_eq!(
            normalize(&ctx, &t),
            g(if expected { "true" } else { "false" })
        );
        assert_evaluators(&ctx, &t, expected);
    }
}

#[test]
fn checked_boolean_alias_rebinding_does_not_fabricate_old_constructor_values() {
    let mut kept = context();
    assert_true_is_kept(&mut kept);
    assert_evaluators(&kept, &bin(g("natLe"), nat(0), nat(0)), true);
    // A legal definition, not raw registry corruption, made while nothing depends on `true`:
    // the orders are then checked against `true := false`. Neither order matches Bool, so both
    // complete programs are well typed.
    let mut ctx = carriers();
    ctx.declare("true", g("Bool"), g("false")).unwrap();
    declare_orders(&mut ctx);
    recheck_orders(&mut ctx);
    for t in [
        bin(g("natLe"), nat(0), nat(0)),
        bin(g("natLt"), nat(0), nat(1)),
    ] {
        assert_eq!(normalize(&ctx, &t), g("false"));
        assert_evaluators(&ctx, &t, false);
    }
}

#[test]
fn natlt_respects_opaque_natle_dependency_at_explicit_transparency() {
    let ctx = context();
    let opaque = |name: &str| name == "natLe";
    let t = bin(g("natLt"), nat(0), nat(1));
    let dependency = bin(g("natLe"), app(g("Succ"), nat(0)), nat(1));
    let expected = normalize_at(&ctx, &dependency, Transparency::Excluding(&opaque));
    assert_ne!(expected, g("true"));
    assert_ne!(expected, g("false"));
    assert_eq!(
        normalize_at(&ctx, &t, Transparency::Excluding(&opaque)),
        expected
    );
}

#[test]
fn opaque_outer_natlt_and_unrestricted_kernel_keep_existing_semantics() {
    let ctx = context();
    let t = bin(g("natLt"), nat(0), nat(1));
    let opaque = |name: &str| name == "natLt";
    assert_eq!(normalize_at(&ctx, &t, Transparency::Excluding(&opaque)), t);
    assert_eq!(normalize(&ctx, &t), g("true"));
}

#[test]
fn legacy_reordered_nat_inventory_never_gets_canonical_order_shortcut() {
    let mut ctx = context();
    let body = ctx.get_definition_body("natLe").unwrap().clone();
    // The legacy unchecked carrier API invalidates the previously checked
    // match inventory. This is an attestation-boundary test, not a claim that
    // this changed environment is a valid checked source program.
    ctx.add_inductive("Nat", Term::Sort(Universe::Type(0)));
    ctx.add_constructor("Succ", "Nat", arrow(g("Nat"), g("Nat")));
    ctx.add_constructor("Zero", "Nat", g("Nat"));
    let named = bin(g("natLe"), nat(0), nat(0));
    let expanded = bin(body, nat(0), nat(0));
    assert_eq!(
        normalize(&ctx, &named),
        normalize(&ctx, &expanded),
        "a legacy mutated inventory must take the ordinary body path"
    );
    assert_eq!(
        eval_bool_tree(&ctx, &named),
        eval_bool_tree(&ctx, &expanded)
    );
    assert_eq!(
        native_compile_decide(&ctx, &named),
        native_compile_decide(&ctx, &expanded)
    );
}

#[test]
fn checked_boolean_rebinding_is_isolated_to_its_context_clone() {
    let base = carriers();
    let t = bin(g("natLe"), nat(0), nat(0));
    let mut parent = base.clone();
    declare_orders(&mut parent);
    assert_evaluators(&parent, &t, true);
    let mut refused = parent.clone();
    assert_true_is_kept(&mut refused);
    assert_evaluators(&refused, &t, true);
    let mut child = base.clone();
    child.declare("true", g("Bool"), g("false")).unwrap();
    declare_orders(&mut child);
    recheck_orders(&mut child);
    assert_evaluators(&child, &t, false);
    assert_evaluators(&parent, &t, true);
}
