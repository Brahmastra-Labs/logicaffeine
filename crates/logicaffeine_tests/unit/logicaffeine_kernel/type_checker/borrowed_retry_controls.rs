//! Do not repeat borrowed congruence when neither head exposure changed its input.
//! Kept separate so the baseline and candidate execute identical assertions.
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
fn pi(name: &str, ty: Term, body: Term) -> Term {
    Term::Pi {
        param: name.into(),
        param_type: Box::new(ty),
        body_type: Box::new(body),
    }
}
fn lam(name: &str, ty: Term, body: Term) -> Term {
    Term::Lambda {
        param: name.into(),
        param_type: Box::new(ty),
        body: Box::new(body),
    }
}
fn context() -> Context {
    let mut ctx = Context::new();
    ctx.add_inductive("Nat", Term::Sort(Universe::Type(0)));
    ctx.add_constructor("Zero", "Nat", g("Nat"));
    ctx.add_constructor("Succ", "Nat", pi("n", g("Nat"), g("Nat")));
    ctx.add_declaration("retry_left", g("Nat"));
    ctx.add_declaration("retry_right", g("Nat"));
    ctx.add_declaration("retry_frame", pi("n", g("Nat"), g("Nat")));
    ctx.add_universe_poly_declaration("retry_poly", vec!["u".into()], pi("n", g("Nat"), g("Nat")))
        .unwrap();
    ctx
}
fn at(level: Universe) -> Term {
    Term::Const {
        name: "retry_poly".into(),
        levels: vec![level],
    }
}
fn chain(head: &Term, leaf: Term) -> Term {
    (0..4).fold(leaf, |tail, _| app(head.clone(), tail))
}
fn probe(ctx: &Context, a: &Term, b: &Term) -> bool {
    bounded_conversion(ctx, a, b, &mut 4096)
}
fn one_visit_per_obligation(head: Term) {
    let ctx = context();
    let a = chain(&head, g("retry_left"));
    let b = chain(&head, g("retry_right"));
    assert_eq!(infer_type(&ctx, &a).unwrap(), g("Nat"));
    assert_eq!(infer_type(&ctx, &b).unwrap(), g("Nat"));
    struct Restore(Option<usize>);
    impl Drop for Restore {
        fn drop(&mut self) {
            BORROWED_STRUCTURAL_VISITS.with(|count| count.set(self.0));
        }
    }
    let _restore = Restore(BORROWED_STRUCTURAL_VISITS.with(|count| count.replace(Some(0))));
    assert!(
        !probe(&ctx, &a, &b),
        "distinct opaque leaves supply no equality proof"
    );
    assert_eq!(
        BORROWED_STRUCTURAL_VISITS.with(|count| count.get()),
        Some(5),
        "four fixed application layers and their unequal leaf are five structural obligations; \
         unchanged borrowed exposure must not retry them"
    );
}

#[test]
fn global_fixed_chain_visits_each_structural_obligation_once() {
    one_visit_per_obligation(g("retry_frame"));
}

#[test]
fn explicit_const_fixed_chain_visits_each_structural_obligation_once() {
    one_visit_per_obligation(at(Universe::Type(0)));
}

#[test]
fn failed_borrowed_congruence_keeps_erasing_delta_fallback() {
    let mut ctx = context();
    ctx.with_equations_suspended(|ctx| {
        ctx.declare(
            "retry_erase",
            pi("n", g("Nat"), g("Nat")),
            lam("n", g("Nat"), g("Zero")),
        )
    })
    .unwrap();
    let a = app(g("retry_erase"), chain(&g("retry_frame"), g("retry_left")));
    let b = app(g("retry_erase"), chain(&g("retry_frame"), g("retry_right")));
    assert_eq!(infer_type(&ctx, &a).unwrap(), g("Nat"));
    assert_eq!(infer_type(&ctx, &b).unwrap(), g("Nat"));
    assert!(probe(&ctx, &a, &b));
    assert!(probe(&ctx, &b, &a));
    let mut changed = ctx.clone();
    changed
        .with_equations_suspended(|ctx| {
            ctx.declare(
                "retry_erase",
                pi("n", g("Nat"), g("Nat")),
                lam("n", g("Nat"), v("n")),
            )
        })
        .unwrap();
    assert!(
        !probe(&changed, &a, &b),
        "an identity must retain the unequal arguments"
    );
    assert!(
        probe(&ctx, &a, &b),
        "a separate context must not alter the original declaration"
    );
}

#[test]
fn owned_beta_exposure_on_either_side_still_gets_new_congruence() {
    let ctx = context();
    // Head reduction exposes retry_frame but leaves its argument's beta redex.
    // Neither opaque frame can delta-reduce, so the new structural comparison
    // after Owned exposure is necessary to compare those arguments.
    let a = app(
        lam(
            "x",
            g("Nat"),
            app(g("retry_frame"), app(lam("z", g("Nat"), v("z")), v("x"))),
        ),
        g("retry_left"),
    );
    let b = app(g("retry_frame"), g("retry_left"));
    assert_eq!(infer_type(&ctx, &a).unwrap(), g("Nat"));
    assert_eq!(infer_type(&ctx, &b).unwrap(), g("Nat"));
    assert!(probe(&ctx, &a, &b));
    assert!(probe(&ctx, &b, &a));
}

#[test]
fn explicit_universe_identity_and_arity_are_not_erased() {
    let ctx = context();
    let a = app(at(Universe::Type(0)), g("retry_left"));
    let b = app(at(Universe::Type(1)), g("retry_left"));
    assert_eq!(infer_type(&ctx, &a).unwrap(), g("Nat"));
    assert_eq!(infer_type(&ctx, &b).unwrap(), g("Nat"));
    assert!(!probe(&ctx, &a, &b));
    assert!(!probe(&ctx, &b, &a));
    let malformed = app(
        Term::Const {
            name: "retry_poly".into(),
            levels: vec![],
        },
        g("retry_left"),
    );
    assert!(infer_type(&ctx, &malformed).is_err());
    assert!(!probe(&ctx, &a, &malformed));
}

#[test]
fn empty_and_small_allowances_never_reset_or_create_equality() {
    let ctx = context();
    let a = chain(&g("retry_frame"), g("retry_left"));
    let b = chain(&g("retry_frame"), g("retry_right"));
    for allowance in [0, 1, 2, 4, 8, 32, 256] {
        let mut remaining = allowance;
        assert!(!bounded_conversion(&ctx, &a, &b, &mut remaining));
        assert!(remaining <= allowance);
        let after_first = remaining;
        assert!(!bounded_conversion(&ctx, &a, &b, &mut remaining));
        assert!(
            remaining <= after_first,
            "a later attempt cannot refund consumed work"
        );
    }
    let mut empty = 0;
    assert!(!bounded_conversion(&ctx, &a, &a, &mut empty));
    assert_eq!(empty, 0);
    let mut one = 1;
    assert!(bounded_conversion(&ctx, &a, &a, &mut one));
    assert_eq!(one, 0);
}

#[test]
fn unchanged_fixed_heads_still_require_whole_input_admission() {
    let mut ctx = context();
    ctx.with_equations_suspended(|ctx| {
        ctx.declare(
            "retry_erase",
            pi("n", g("Nat"), g("Nat")),
            lam("n", g("Nat"), g("Zero")),
        )
    })
    .unwrap();
    fn tree(depth: usize) -> Term {
        if depth == 0 {
            return g("Zero");
        }
        app(app(g("retry_raw_tree"), tree(depth - 1)), tree(depth - 1))
    }
    // Deliberately raw syntax: this tests the existing 4096-node admission,
    // not the typing of an invented tree constructor.
    let oversized = app(g("retry_erase"), tree(11));
    assert!(crate::reduction::bounded_conversion_size(&oversized).is_none());
    assert!(!probe(&ctx, &oversized, &g("Zero")));
    assert!(!probe(&ctx, &g("Zero"), &oversized));
    assert!(!conversion_by_unfolded_head(
        &ctx,
        &app(g("retry_erase"), Term::Hole),
        &g("Zero"),
    ));
}
