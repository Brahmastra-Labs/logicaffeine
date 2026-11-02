//! A bare reference converts with an explicit instance exactly at the instance its typing used.
//!
//! `n a₁ … aₖ`, written with no levels, denotes `n.{ℓ̄} a₁ … aₖ` for the `ℓ̄` the application rule
//! solves from the arguments. Conversion must read that same `ℓ̄` — from the first argument, from
//! a later one, through a type written as a definition — and no other: an instance that is well
//! formed only by cumulativity is a different constant. Both kernels decide it, each through its
//! own application rule, and they agree.
use logicaffeine_kernel::interface::Repl;
use logicaffeine_kernel::{
    defeq_for_test, double_check, infer_type, recheck, Context, DoubleCheck, Term, Universe,
};

fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn at(n: &str, levels: Vec<Universe>) -> Term {
    Term::Const {
        name: n.into(),
        levels,
    }
}
fn app(f: Term, xs: Vec<Term>) -> Term {
    xs.into_iter()
        .fold(f, |f, x| Term::App(Box::new(f), Box::new(x)))
}
fn level(n: u32) -> Universe {
    if n == 0 {
        Universe::Prop
    } else {
        Universe::Type(n - 1)
    }
}

fn context(lines: &[&str]) -> Context {
    let mut r = Repl::new();
    for line in lines {
        r.execute(line).unwrap_or_else(|e| panic!("{line}: {e:?}"));
    }
    r.context().clone()
}

/// `λ (y : from). (λ (x : to). x) y` — well typed exactly when `from` converts to `to`.
fn coercion(from: &Term, to: &Term) -> Term {
    Term::Lambda {
        param: "y".into(),
        param_type: Box::new(from.clone()),
        body: Box::new(Term::App(
            Box::new(Term::Lambda {
                param: "x".into(),
                param_type: Box::new(to.clone()),
                body: Box::new(Term::Var("x".into())),
            }),
            Box::new(Term::Var("y".into())),
        )),
    }
}

/// `bare` converts with `explicit(ℓ)` for `ℓ = typed` and for no other level in `0..=3`, in both
/// kernels and in both directions; and the two typings agree at `typed`.
fn converts_exactly_at(
    ctx: &Context,
    bare: &Term,
    explicit: impl Fn(Universe) -> Term,
    typed: u32,
) {
    let bare_type = infer_type(ctx, bare).unwrap_or_else(|e| panic!("{bare}: {e:?}"));
    for l in 0..=3 {
        let e = explicit(level(l));
        let expected = l == typed;
        assert_eq!(defeq_for_test(ctx, bare, &e), expected, "{bare} ≡ {e}");
        assert_eq!(defeq_for_test(ctx, &e, bare), expected, "{e} ≡ {bare}");
        if expected {
            let explicit_type = infer_type(ctx, &e).unwrap_or_else(|err| panic!("{e}: {err:?}"));
            assert!(
                defeq_for_test(ctx, &bare_type, &explicit_type),
                "{bare} and {e} are typed alike"
            );
        }
        if matches!(bare_type, Term::Sort(_)) {
            for (from, to) in [(bare, &e), (&e, bare)] {
                let term = coercion(from, to);
                if expected {
                    recheck(ctx, &term).unwrap_or_else(|err| {
                        panic!("the re-checker converts {from} to {to}: {err:?}")
                    });
                    assert_eq!(
                        double_check(ctx, &term),
                        DoubleCheck::Agreed,
                        "{from} to {to}"
                    );
                } else {
                    assert!(
                        infer_type(ctx, &term).is_err(),
                        "{from} must not convert to {to}"
                    );
                    assert!(
                        recheck(ctx, &term).is_err(),
                        "the re-checker refuses {from} as {to}"
                    );
                }
            }
        }
    }
}

#[test]
fn the_level_the_first_argument_determines() {
    let ctx =
        context(&["Inductive Pt.{u} (A : Sort (u+1)) : Sort (u+1) := ptmk : A -> A -> Pt.{u} A."]);
    let eq = |carrier: Term, x: Term| {
        move |l| {
            app(
                at("Eq", vec![l]),
                vec![carrier.clone(), x.clone(), x.clone()],
            )
        }
    };
    let bare = |carrier: Term, x: Term| app(g("Eq"), vec![carrier, x.clone(), x]);
    converts_exactly_at(&ctx, &bare(g("Nat"), g("Zero")), eq(g("Nat"), g("Zero")), 1);
    converts_exactly_at(&ctx, &bare(g("True"), g("I")), eq(g("True"), g("I")), 0);
    let pt = app(at("Pt", vec![Universe::Prop]), vec![g("Nat")]);
    let point = app(
        at("ptmk", vec![Universe::Prop]),
        vec![g("Nat"), g("Zero"), g("Zero")],
    );
    converts_exactly_at(&ctx, &bare(pt.clone(), point.clone()), eq(pt, point), 1);
}

#[test]
fn the_level_a_later_argument_determines() {
    let ctx = context(&["Axiom Tag.{u} : Nat -> Sort u -> Prop."]);
    converts_exactly_at(
        &ctx,
        &app(g("Tag"), vec![g("Zero"), g("Nat")]),
        |l| app(at("Tag", vec![l]), vec![g("Zero"), g("Nat")]),
        1,
    );
    converts_exactly_at(
        &ctx,
        &app(g("Tag"), vec![g("Zero"), g("True")]),
        |l| app(at("Tag", vec![l]), vec![g("Zero"), g("True")]),
        0,
    );
    // Before the argument that determines it, nothing is concluded at any level.
    for l in 0..=3 {
        assert!(!defeq_for_test(
            &ctx,
            &app(g("Tag"), vec![g("Zero")]),
            &app(at("Tag", vec![level(l)]), vec![g("Zero")])
        ));
    }
}

#[test]
fn a_level_that_decides_the_result_sort() {
    let ctx = context(&["Inductive Bx.{u} (A : Sort u) : Sort (max 1 u) := bxmk : A -> Bx.{u} A."]);
    converts_exactly_at(
        &ctx,
        &app(g("Bx"), vec![g("Nat")]),
        |l| app(at("Bx", vec![l]), vec![g("Nat")]),
        1,
    );
    converts_exactly_at(
        &ctx,
        &app(g("Bx"), vec![g("True")]),
        |l| app(at("Bx", vec![l]), vec![g("True")]),
        0,
    );
}

#[test]
fn a_type_written_through_a_definition() {
    // `W`'s type is not syntactically a product: the application rule exposes it by unfolding
    // `Mk.{u}`, and conversion must read the level through the same exposure.
    let ctx = context(&[
        "Definition Mk.{u} : Sort (u+1) := Sort u -> Prop.",
        "Axiom W.{u} : Mk.{u}.",
    ]);
    converts_exactly_at(
        &ctx,
        &app(g("W"), vec![g("Nat")]),
        |l| app(at("W", vec![l]), vec![g("Nat")]),
        1,
    );
    converts_exactly_at(
        &ctx,
        &app(g("W"), vec![g("True")]),
        |l| app(at("W", vec![l]), vec![g("True")]),
        0,
    );
}
