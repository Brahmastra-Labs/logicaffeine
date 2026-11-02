use super::*;
use crate::Universe;

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
fn pi(n: &str, ty: Term, body: Term) -> Term {
    Term::Pi {
        param: n.into(),
        param_type: Box::new(ty),
        body_type: Box::new(body),
    }
}
fn operation_type() -> Term {
    pi("m", g("Nat"), pi("n", g("Nat"), g("CacheBit")))
}
fn order_body() -> Term {
    let inner = Term::Match {
        discriminant: Box::new(v("n")),
        motive: Box::new(lam("_", g("Nat"), g("CacheBit"))),
        cases: vec![
            g("CacheBit.no"),
            lam("pn", g("Nat"), app(app(v("walk"), v("pm")), v("pn"))),
        ],
    };
    Term::Fix {
        name: "walk".into(),
        body: Box::new(lam(
            "m",
            g("Nat"),
            lam(
                "n",
                g("Nat"),
                Term::Match {
                    discriminant: Box::new(v("m")),
                    motive: Box::new(lam("_", g("Nat"), g("CacheBit"))),
                    cases: vec![g("CacheBit.yes"), lam("pm", g("Nat"), inner)],
                },
            ),
        )),
    }
}
fn fixture() -> Context {
    let mut c = Context::new();
    c.add_inductive("Nat", Term::Sort(Universe::Type(0)));
    c.add_constructor("Zero", "Nat", g("Nat"));
    c.add_constructor("Succ", "Nat", pi("n", g("Nat"), g("Nat")));
    c.add_inductive("CacheBit", Term::Sort(Universe::Type(0)));
    // Reversed registration order ensures ordinal/name guessing cannot pass.
    c.add_constructor("CacheBit.yes", "CacheBit", g("CacheBit"));
    c.add_constructor("CacheBit.no", "CacheBit", g("CacheBit"));
    c.declare("cache.order", operation_type(), order_body())
        .unwrap();
    c
}
fn attest(c: &Context) -> Option<[Term; 2]> {
    attest_with_noise(c, 0)
}
fn attest_with_noise(c: &Context, noise: usize) -> Option<[Term; 2]> {
    // A new store and reducer on every call are the causal distinction from the
    // existing certified_orders map, which already deduplicates within one reducer.
    let mut reducer = HeadReducerWithWork::<false>::new(c, 10_000);
    // Force different store-local IDs without adding any context declaration.
    for i in 0..noise {
        reducer.store.intern(&g(&format!("unrelated.interned.{i}")));
    }
    let head = reducer.store.intern(&g("cache.order"));
    let [no, yes] = reducer.certified_order(head)?;
    Some([reducer.store.extract(no), reducer.store.extract(yes)])
}
fn attempts() -> usize {
    ORDER_CERTIFICATIONS.with(|n| n.get())
}
fn correct_outcomes() -> Option<[Term; 2]> {
    Some([g("CacheBit.no"), g("CacheBit.yes")])
}

#[test]
fn successful_order_attestation_survives_fresh_reducer_in_same_immutable_context() {
    let c = fixture();
    let before = attempts();
    assert_eq!(attest(&c), correct_outcomes());
    let after_first = attempts();
    assert_eq!(
        after_first,
        before + 1,
        "first encounter must actually establish its recurrence"
    );
    assert_eq!(attest_with_noise(&c, 1), correct_outcomes());
    assert_eq!(attest_with_noise(&c, 3), correct_outcomes());
    assert_eq!(
        attempts(),
        after_first,
        "new reducers must reuse the checked global recurrence"
    );
}

#[test]
fn replacing_the_attested_body_invalidates_the_shared_result() {
    let mut c = fixture();
    assert_eq!(attest(&c), correct_outcomes());
    c.declare(
        "cache.order",
        operation_type(),
        lam("m", g("Nat"), lam("n", g("Nat"), g("CacheBit.no"))),
    )
    .unwrap();
    let before_changed = attempts();
    assert_eq!(
        attest(&c),
        None,
        "constant output does not satisfy the three order equations"
    );
    assert_eq!(
        attempts(),
        before_changed + 1,
        "body replacement must trigger a new attestation"
    );
}

#[test]
fn mutated_context_clone_cannot_reuse_its_parents_order_certificate() {
    let original = fixture();
    assert_eq!(attest(&original), correct_outcomes());
    let mut changed = original.clone();
    changed
        .declare(
            "cache.order",
            operation_type(),
            lam("m", g("Nat"), lam("n", g("Nat"), g("CacheBit.no"))),
        )
        .unwrap();
    let before_changed = attempts();
    assert_eq!(attest(&changed), None);
    assert_eq!(attempts(), before_changed + 1);
    // A bounded one-generation cache may evict the original entry: only the
    // original semantic answer is asserted here, not an eviction policy.
    assert_eq!(attest(&original), correct_outcomes());
}

#[test]
fn recurrence_check_context_cannot_consume_an_already_cached_certificate() {
    let c = fixture();
    assert_eq!(attest(&c), correct_outcomes());
    let before = attempts();
    let gated = c.order_recurrence_context();
    assert_eq!(
        attest(&gated),
        None,
        "attestation must never authorize its own recurrence check"
    );
    assert_eq!(attest(&gated.clone()), None);
    assert_eq!(
        attempts(),
        before,
        "the preexisting recurrence gate must precede cache lookup"
    );
    assert_eq!(attest(&c), correct_outcomes());
    assert_eq!(
        attempts(),
        before,
        "a gated refusal must not overwrite valid outer evidence"
    );
}
