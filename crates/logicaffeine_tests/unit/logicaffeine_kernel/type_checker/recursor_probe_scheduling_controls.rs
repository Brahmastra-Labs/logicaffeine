//! Reduced from genuine grind-ring-5 diagnostic wins: open explicit recursor iota.
//! The neutral minor premises prevent the closed-data evaluator from deciding these
//! pairs; an explicit recursor reference remains opaque to the beta/delta fragment.
use super::*;

fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn v(n: &str) -> Term {
    Term::Var(n.into())
}
fn app(f: Term, x: Term) -> Term {
    Term::App(Box::new(f), Box::new(x))
}
fn pi(n: &str, a: Term, b: Term) -> Term {
    Term::Pi {
        param: n.into(),
        param_type: Box::new(a),
        body_type: Box::new(b),
    }
}
fn succ(n: Term) -> Term {
    app(g("Succ"), n)
}
fn motive(n: Term) -> Term {
    app(v("motive"), n)
}
fn call(n: Term) -> Term {
    let head = Term::Const {
        name: "ProbeNatRec".into(),
        levels: vec![Universe::Type(0)],
    };
    app(app(app(app(head, v("motive")), v("base")), v("step")), n)
}

fn context() -> Context {
    let mut c = Context::new();
    c.add_inductive("Nat", Term::Sort(Universe::Type(0)));
    c.add_constructor("Zero", "Nat", g("Nat"));
    c.add_constructor("Succ", "Nat", pi("n", g("Nat"), g("Nat")));
    let (levels, ty, body) = crate::recursor::derive_recursor_universe_poly(&c, "Nat").unwrap();
    assert_eq!(
        levels.len(),
        1,
        "the motive universe is the only Nat recursor parameter"
    );
    crate::recheck(&c, &body).unwrap();
    c.with_equations_suspended(|c| c.declare_universe_poly("ProbeNatRec", levels, ty, body))
        .unwrap();
    c.extend("motive", pi("n", g("Nat"), Term::Sort(Universe::Type(0))))
        .extend("base", motive(g("Zero")))
        .extend(
            "step",
            pi(
                "n",
                g("Nat"),
                pi("ih", motive(v("n")), motive(succ(v("n")))),
            ),
        )
        .extend("n", g("Nat"))
}

fn checked_pair(c: &Context, a: &Term, b: &Term, expected: &Term) {
    for t in [a, b] {
        assert!(def_eq(c, &infer_type(c, t).unwrap(), expected));
        assert!(def_eq(c, &crate::recheck(c, t).unwrap(), expected));
    }
}

fn causal_probe(c: &Context, a: &Term, b: &Term) {
    let mut fuel = 256;
    assert!(bounded_conversion(c, a, b, &mut fuel));
    assert!(fuel > 0 && fuel < 256);
    struct Restore(Option<usize>);
    impl Drop for Restore {
        fn drop(&mut self) {
            CONVERSION_PROOF_TYPE_PROBES.with(|n| n.set(self.0));
        }
    }
    let _restore = Restore(CONVERSION_PROOF_TYPE_PROBES.with(|n| n.replace(Some(0))));
    assert!(def_eq_uncached(c, a, b));
    let probes = CONVERSION_PROOF_TYPE_PROBES.with(|n| n.get().unwrap());
    assert_eq!(
        probes, 0,
        "a bounded recursor-iota equality must be proved before speculative endpoint typing"
    );
}

#[test]
fn open_explicit_recursor_zero_iota_precedes_proof_typing() {
    let c = context();
    let a = v("base");
    let b = call(g("Zero"));
    checked_pair(&c, &a, &b, &motive(g("Zero")));
    causal_probe(&c, &a, &b);
    causal_probe(&c, &b, &a);
}

#[test]
fn open_explicit_recursor_successor_iota_precedes_proof_typing() {
    let c = context();
    let a = app(app(v("step"), v("n")), call(v("n")));
    let b = call(succ(v("n")));
    checked_pair(&c, &a, &b, &motive(succ(v("n"))));
    causal_probe(&c, &a, &b);
    causal_probe(&c, &b, &a);
}

#[test]
fn recursor_iota_does_not_identify_distinct_minor_premises() {
    let c = context().extend("other_base", motive(g("Zero")));
    let a = v("other_base");
    let b = call(g("Zero"));
    checked_pair(&c, &a, &b, &motive(g("Zero")));
    assert!(!bounded_conversion(&c, &a, &b, &mut 256));
    assert!(!def_eq(&c, &a, &b));
    assert!(!def_eq(&c, &b, &a));
    let bad = Term::Let {
        name: "wrong_minor".into(),
        ty: Box::new(app(v("Family"), a)),
        value: Box::new(v("witness")),
        body: Box::new(v("wrong_minor")),
    };
    let c = c
        .extend(
            "Family",
            pi("x", motive(g("Zero")), Term::Sort(Universe::Type(0))),
        )
        .extend("witness", app(v("Family"), b));
    assert!(infer_type(&c, &bad).is_err());
    assert!(crate::recheck(&c, &bad).is_err());
}

#[test]
fn recursor_iota_is_valid_in_an_invariant_dependent_family() {
    let c = context();
    let a = v("base");
    let b = call(g("Zero"));
    let c = c
        .extend(
            "Family",
            pi("x", motive(g("Zero")), Term::Sort(Universe::Type(0))),
        )
        .extend("witness", app(v("Family"), a));
    let good = Term::Let {
        name: "checked_minor".into(),
        ty: Box::new(app(v("Family"), b)),
        value: Box::new(v("witness")),
        body: Box::new(v("checked_minor")),
    };
    infer_type(&c, &good).unwrap();
    crate::recheck(&c, &good).unwrap();
}
