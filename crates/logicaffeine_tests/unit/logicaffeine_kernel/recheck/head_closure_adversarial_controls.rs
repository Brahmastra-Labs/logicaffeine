//! Direct positive-probe controls; the existing independent substitution
//! implementation supplies expected capture/rebasing results.
use super::*;
fn g(n: &str) -> Db {
    Db::Global(n.into())
}
fn t(n: &str) -> Term {
    Term::Global(n.into())
}
fn own(t: Db) -> DbBox {
    DbBox::owned(Box::new(t))
}
fn app(f: Db, a: Db) -> Db {
    Db::App(own(f), own(a))
}
fn lam(d: Db, b: Db) -> Db {
    Db::Lam(own(d), own(b))
}
fn pi(d: Db, b: Db) -> Db {
    Db::Pi(own(d), own(b))
}
fn let_(d: Db, v: Db, b: Db) -> Db {
    Db::Let(own(d), own(v), own(b))
}
fn join(a: Db, b: Db) -> Db {
    app(app(g("join"), a), b)
}
fn context() -> Context {
    let mut c = Context::new();
    c.add_declaration("N", Term::Sort(Universe::Type(0)));
    for n in ["zero", "one"] {
        c.add_declaration(n, t("N"));
    }
    c.add_declaration(
        "join",
        Term::Pi {
            param: "x".into(),
            param_type: Box::new(t("N")),
            body_type: Box::new(Term::Pi {
                param: "y".into(),
                param_type: Box::new(t("N")),
                body_type: Box::new(t("N")),
            }),
        },
    );
    c.add_declaration(
        "Family",
        Term::Pi {
            param: "x".into(),
            param_type: Box::new(t("N")),
            body_type: Box::new(Term::Sort(Universe::Type(0))),
        },
    );
    c
}
fn yes(c: &Context, a: &Db, b: &Db) {
    for (a, b) in [(a, b), (b, a)] {
        let mut fuel = 4096;
        assert!(
            head_closure::equivalent(c, a, b, &mut fuel),
            "capture-correct reference reduction must align"
        );
        assert!(
            fuel < 4096,
            "nontrivial probe work must spend its caller's allowance"
        );
    }
}
fn no(c: &Context, a: &Db, b: &Db) {
    for (a, b) in [(a, b), (b, a)] {
        let mut fuel = 4096;
        assert!(
            !head_closure::equivalent(c, a, b, &mut fuel),
            "positive probe must not invent a conversion"
        );
        assert!(fuel <= 4096);
    }
}

#[test]
fn head_closure_beta_matches_reference_under_lambda_and_external_depths() {
    let c = context();
    let body = lam(g("N"), join(Db::Var(1), Db::Var(0)));
    for index in [0, 1, 7] {
        let arg = Db::Var(index);
        let expected = beta_open(&body, &arg);
        assert_eq!(expected, lam(g("N"), join(shift(&arg, 1, 0), Db::Var(0))));
        yes(&c, &app(lam(g("N"), body.clone()), arg), &expected);
    }
}

#[test]
fn head_closure_beta_matches_reference_under_dependent_pi() {
    let c = context();
    let body = pi(g("N"), app(g("Family"), Db::Var(1)));
    for index in [0, 2, 7] {
        let arg = Db::Var(index);
        let expected = beta_open(&body, &arg);
        assert_eq!(expected, pi(g("N"), app(g("Family"), shift(&arg, 1, 0))));
        yes(&c, &app(lam(g("N"), body.clone()), arg), &expected);
    }
}

#[test]
fn head_closure_beta_and_zeta_preserve_let_value_and_body_scopes() {
    let c = context();
    // The let value refers to the outer beta binder; Var0 in its body is the
    // new let binder and Var1 is that original beta binder.
    let body = let_(g("N"), Db::Var(0), join(Db::Var(1), Db::Var(0)));
    for index in [0, 3] {
        let arg = Db::Var(index);
        let expected = beta_open(&body, &arg);
        let Db::Let(_, value, body) = &expected else {
            panic!("reference preserves the let");
        };
        let normalized = beta_open(body, value);
        assert_eq!(normalized, join(arg.clone(), arg.clone()));
        yes(&c, &app(lam(g("N"), body_for_let()), arg), &normalized);
    }
    fn body_for_let() -> Db {
        let_(g("N"), Db::Var(0), join(Db::Var(1), Db::Var(0)))
    }
}

#[test]
fn head_closure_nested_open_environments_match_two_reference_beta_steps() {
    let c = context();
    let body = lam(
        g("N"),
        lam(g("N"), join(Db::Var(2), join(Db::Var(1), Db::Var(0)))),
    );
    let first = beta_open(&body, &Db::Var(1));
    let Db::Lam(_, second_body) = first else {
        panic!("one remaining argument binder");
    };
    let expected = beta_open(&second_body, &Db::Var(0));
    assert_eq!(
        expected,
        lam(g("N"), join(Db::Var(2), join(Db::Var(1), Db::Var(0))))
    );
    let source = app(app(lam(g("N"), body), Db::Var(1)), Db::Var(0));
    yes(&c, &source, &expected);
    no(
        &c,
        &source,
        &lam(g("N"), join(Db::Var(1), join(Db::Var(2), Db::Var(0)))),
    );
}

#[test]
fn head_closure_same_syntax_under_distinct_captured_values_stays_distinct() {
    let c = context();
    let f = lam(g("N"), lam(g("N"), Db::Var(1)));
    let left = app(f.clone(), Db::Var(0));
    let right = app(f, Db::Var(1));
    no(&c, &left, &right);
    yes(&c, &left, &lam(g("N"), Db::Var(1)));
    yes(&c, &right, &lam(g("N"), Db::Var(2)));
}

#[test]
fn head_closure_argument_order_and_dependent_domains_are_not_erased() {
    let c = context();
    let f = lam(g("N"), lam(g("N"), join(Db::Var(1), Db::Var(0))));
    no(
        &c,
        &app(app(f.clone(), g("zero")), g("one")),
        &app(app(f, g("one")), g("zero")),
    );
    no(
        &c,
        &lam(app(g("Family"), Db::Var(0)), Db::Var(0)),
        &lam(app(g("Family"), Db::Var(1)), Db::Var(0)),
    );
    no(
        &c,
        &pi(app(g("Family"), Db::Var(0)), g("N")),
        &pi(app(g("Family"), Db::Var(1)), g("N")),
    );
}

#[test]
fn head_closure_explicit_universes_preserve_arity_order_and_call_isolation() {
    let mut c = context();
    let u = Universe::Var("u".into());
    c.declare_universe_poly(
        "poly",
        vec!["u".into(), "v".into()],
        Term::Sort(Universe::Succ(Box::new(u.clone()))),
        Term::Sort(u),
    )
    .unwrap();
    let poly = |levels| Db::Const {
        name: "poly".into(),
        levels,
    };
    let left = poly(vec![Universe::Type(0), Universe::Type(1)]);
    let right = poly(vec![Universe::Type(1), Universe::Type(0)]);
    yes(&c, &left, &Db::Sort(Universe::Type(0)));
    yes(&c, &right, &Db::Sort(Universe::Type(1)));
    no(&c, &left, &right);
    for levels in [vec![], vec![Universe::Type(0)], vec![Universe::Type(0); 3]] {
        no(&c, &poly(levels), &Db::Sort(Universe::Type(0)));
    }
    no(&c, &g("poly"), &Db::Sort(Universe::Type(0)));
    yes(&c, &left, &Db::Sort(Universe::Type(0)));
}

#[test]
fn head_closure_declared_opaque_heads_cannot_be_replaced_by_same_typed_values() {
    let mut c = context();
    c.add_declaration("opaque", t("N"));
    no(&c, &g("opaque"), &g("zero"));
    no(&c, &app(g("opaque"), Db::Var(0)), &Db::Var(0));
}

#[test]
fn head_closure_native_heads_do_not_follow_untrusted_ordinary_bodies() {
    let mut c = context();
    // Deliberately raw spoofed definitions: delta must not override the native
    // dispatcher's semantics merely because a definition registry has a body.
    for n in [
        "reduceBool",
        "try_decide",
        "Quot_lift",
        "Quot_rec",
        "kernel.quot.lift",
    ] {
        c.add_definition(n.into(), t("N"), t("zero"));
        no(&c, &g(n), &g("zero"));
    }
}

#[test]
fn head_closure_open_global_body_cannot_capture_callers_environment() {
    let mut c = context();
    c.add_definition("untrusted.open".into(), t("N"), Term::Var("outer".into()));
    no(&c, &g("untrusted.open"), &Db::Var(0));
    no(
        &c,
        &app(lam(g("N"), g("untrusted.open")), g("zero")),
        &g("zero"),
    );
}

#[test]
fn head_closure_recursive_aliases_and_exhausted_allowances_are_inconclusive() {
    let mut c = context();
    c.add_definition("cycle.a".into(), t("N"), t("cycle.b"));
    c.add_definition("cycle.b".into(), t("N"), t("cycle.a"));
    for allowance in [0, 1, 8, 4096] {
        let mut fuel = allowance;
        assert!(!head_closure::equivalent(
            &c,
            &g("cycle.a"),
            &g("zero"),
            &mut fuel
        ));
        assert!(fuel <= allowance, "refusal must not replenish work");
    }
    let source = app(lam(g("N"), Db::Var(0)), g("zero"));
    let mut none = 0;
    assert!(!head_closure::equivalent(
        &c,
        &source,
        &g("zero"),
        &mut none
    ));
    assert_eq!(none, 0);
}

#[test]
fn head_closure_does_not_reduce_match_or_fix_as_a_hidden_native_extension() {
    let c = context();
    let matched = Db::Match {
        disc: own(g("zero")),
        motive: own(lam(g("N"), g("N"))),
        cases: vec![g("zero")],
    };
    no(&c, &matched, &g("zero"));
    let fixed = Db::Fix(FixBody::Owned(Box::new(g("zero"))));
    no(&c, &fixed, &g("zero"));
}

#[test]
fn head_closure_distinct_float_encodings_are_never_equated() {
    let c = context();
    // Literal's current kernel equality is exact-bit reflexive, including NaN.
    // This negative control does not prescribe whether the probe admits Float.
    for (a, b) in [
        (0.0, -0.0),
        (
            f64::from_bits(0x7ff8_0000_0000_0001),
            f64::from_bits(0x7ff8_0000_0000_0002),
        ),
    ] {
        no(&c, &Db::Lit(Literal::Float(a)), &Db::Lit(Literal::Float(b)));
    }
}

#[test]
fn head_closure_nan_is_conservatively_outside_the_first_fragment() {
    let c = context();
    let atom = Db::Lit(Literal::Float(f64::from_bits(0x7ff8_0000_0000_0001)));
    // The current fragment explicitly refuses Float. This is an admission
    // policy control, not a claim that the kernel's bitwise equality is nonreflexive.
    let mut fuel = 4096;
    assert!(!head_closure::equivalent(&c, &atom, &atom, &mut fuel));
    assert!(fuel <= 4096);
}

#[test]
fn head_closure_oversized_atoms_cannot_bypass_the_probe_allowance() {
    let c = context();
    for atom in [
        Db::Global("g".repeat(1 << 20)),
        Db::Lit(Literal::Text("t".repeat(1 << 20))),
        Db::Sort(Universe::Var("u".repeat(1 << 20))),
    ] {
        let mut fuel = 4096;
        assert!(
            !head_closure::equivalent(&c, &atom, &atom, &mut fuel),
            "even identity needs bounded atom admission"
        );
        assert!(fuel <= 4096);
    }
}
