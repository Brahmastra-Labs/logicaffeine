//! Early independent proof irrelevance must precede context-aware delta probes.
//! These controls exercise existing evaluator entry points, not a proposed helper.
use super::*;

fn g(name: &str) -> Term {
    Term::Global(name.into())
}
fn dg(name: &str) -> Db {
    Db::Global(name.into())
}
fn dc(name: &str, levels: Vec<Universe>) -> Db {
    Db::Const {
        name: name.into(),
        levels,
    }
}
fn pi(name: &str, domain: Term, body: Term) -> Term {
    Term::Pi {
        param: name.into(),
        param_type: Box::new(domain),
        body_type: Box::new(body),
    }
}
fn app(f: Db, a: Db) -> Db {
    Db::App(DbBox::owned(Box::new(f)), DbBox::owned(Box::new(a)))
}
fn wrapped(seed: &str) -> Term {
    let mut body = g(seed);
    // Small checked beta steps; the aggregate exceeds the tiny alias probe.
    for _ in 0..32 {
        body = Term::App(
            Box::new(Term::Lambda {
                param: "proof".into(),
                param_type: Box::new(g("P")),
                body: Box::new(Term::Var("proof".into())),
            }),
            Box::new(body),
        );
    }
    body
}
fn context() -> Context {
    let mut c = Context::new();
    c.add_universe_poly_declaration("P", vec![], Term::Sort(Universe::Prop))
        .unwrap();
    c.add_universe_poly_declaration("Q", vec![], Term::Sort(Universe::Prop))
        .unwrap();
    for name in ["seed_left", "seed_right"] {
        c.add_universe_poly_declaration(name, vec![], g("P"))
            .unwrap();
    }
    c.add_universe_poly_declaration("seed_other", vec![], g("Q"))
        .unwrap();
    for (name, seed) in [("left", "seed_left"), ("right", "seed_right")] {
        c.declare_theorem_checked(name, vec!["unused".into()], g("P"), wrapped(seed))
            .unwrap();
    }
    c.declare_theorem_checked("other", vec![], g("Q"), g("seed_other"))
        .unwrap();
    c.declare_opaque_checked("sealed", vec![], g("P"), wrapped("seed_left"))
        .unwrap();
    c.add_universe_poly_declaration(
        "Family",
        vec![],
        pi("proof", g("P"), Term::Sort(Universe::Type(0))),
    )
    .unwrap();
    assert!(c.pending_obligations().is_empty());
    c
}

/// Isolate the existing meter and counter, including unwinding.
struct Meter {
    fuel: usize,
    depth: usize,
    context_depth: usize,
    exhausted: bool,
    lowerings: Option<usize>,
}
impl Meter {
    fn enter(fuel: usize) -> Self {
        let old = Self {
            fuel: WHNF_FUEL.with(|v| v.replace(fuel)),
            depth: WHNF_DEPTH.with(|v| v.replace(0)),
            context_depth: LOCAL_CONTEXT_BUDGET_DEPTH.with(|v| v.replace(1)),
            exhausted: WHNF_EXHAUSTED.with(|v| v.replace(false)),
            lowerings: DEFINITION_LOWERINGS.with(|v| v.replace(Some(0))),
        };
        old
    }
    fn count() -> usize {
        DEFINITION_LOWERINGS.with(|v| v.get().unwrap())
    }
}
impl Drop for Meter {
    fn drop(&mut self) {
        WHNF_FUEL.with(|v| v.set(self.fuel));
        WHNF_DEPTH.with(|v| v.set(self.depth));
        LOCAL_CONTEXT_BUDGET_DEPTH.with(|v| v.set(self.context_depth));
        WHNF_EXHAUSTED.with(|v| v.set(self.exhausted));
        DEFINITION_LOWERINGS.with(|v| v.set(self.lowerings));
    }
}
fn independent_pair(c: &Context, a: &Db, b: &Db, ty: &Db) {
    assert_eq!(infer(c, &[], a).unwrap(), *ty);
    assert_eq!(infer(c, &[], b).unwrap(), *ty);
}
fn no_proof_lowerings(c: &Context, a: &Db, b: &Db) {
    assert!(def_eq(c, &[], a, b));
    assert!(!WHNF_EXHAUSTED.with(|v| v.get()));
    assert_eq!(
        Meter::count(),
        0,
        "same-proposition references must settle before any definition-body lowering"
    );
}

#[test]
fn global_proof_references_compare_before_delta_body_lowering() {
    let c = context();
    let _meter = Meter::enter(MAX_RECHECK_STEPS);
    let _scope = PrefixScope::enter(&c);
    let (a, b) = (dg("left"), dg("right"));
    independent_pair(&c, &a, &b, &dg("P"));
    no_proof_lowerings(&c, &a, &b);
    no_proof_lowerings(&c, &b, &a);
}

#[test]
fn explicit_proof_references_preserve_unused_universe_instances_without_lowering() {
    let c = context();
    let _meter = Meter::enter(MAX_RECHECK_STEPS);
    let _scope = PrefixScope::enter(&c);
    let a = dc("left", vec![Universe::Type(0)]);
    let b = dc("right", vec![Universe::Type(1)]);
    independent_pair(&c, &a, &b, &dg("P"));
    no_proof_lowerings(&c, &a, &b);
    no_proof_lowerings(&c, &b, &a);
}

#[test]
fn nested_common_family_comparison_preserves_proof_edges_and_outer_budget() {
    let c = context();
    let _meter = Meter::enter(MAX_RECHECK_STEPS);
    let _scope = PrefixScope::enter(&c);
    let a = app(dg("Family"), dg("left"));
    let b = app(dg("Family"), dg("right"));
    independent_pair(&c, &a, &b, &Db::Sort(Universe::Type(0)));
    let before = WHNF_FUEL.with(|v| v.get());
    no_proof_lowerings(&c, &a, &b);
    assert!(
        WHNF_FUEL.with(|v| v.get()) <= before,
        "nested speculation cannot refill fuel"
    );
    assert_eq!(LOCAL_CONTEXT_BUDGET_DEPTH.with(|v| v.get()), 1);
    assert_eq!(WHNF_DEPTH.with(|v| v.get()), 0);
}

#[test]
fn distinct_propositions_are_not_equal_and_do_not_poison_later_proof_comparison() {
    let c = context();
    let _meter = Meter::enter(MAX_RECHECK_STEPS);
    let _scope = PrefixScope::enter(&c);
    assert_eq!(infer(&c, &[], &dg("other")).unwrap(), dg("Q"));
    assert!(!def_eq(&c, &[], &dg("left"), &dg("other")));
    assert!(!def_eq(&c, &[], &dg("other"), &dg("left")));
    assert!(!WHNF_EXHAUSTED.with(|v| v.get()));
    // This assertion checks logical fallback after failure, not a cache hit count.
    assert!(def_eq(&c, &[], &dg("left"), &dg("right")));
    assert!(!WHNF_EXHAUSTED.with(|v| v.get()));
}

#[test]
fn malformed_explicit_proof_arity_cannot_borrow_another_instances_type() {
    let c = context();
    let _meter = Meter::enter(MAX_RECHECK_STEPS);
    let _scope = PrefixScope::enter(&c);
    let good = dc("right", vec![Universe::Type(0)]);
    for levels in [vec![], vec![Universe::Type(0), Universe::Type(1)]] {
        let bad = dc("left", levels.clone());
        assert!(infer(&c, &[], &bad).is_err());
        let source = Term::Const {
            name: "left".into(),
            levels,
        };
        assert!(crate::infer_type(&c, &source).is_err());
        assert!(crate::recheck(&c, &source).is_err());
        assert!(!def_eq(&c, &[], &bad, &good));
        assert!(!def_eq(&c, &[], &good, &bad));
    }
}

#[test]
fn exhausted_comparison_cannot_return_unqualified_evidence_or_refill_fuel() {
    let c = context();
    let _meter = Meter::enter(0);
    let _scope = PrefixScope::enter(&c);
    let equal = def_eq(&c, &[], &dg("left"), &dg("right"));
    assert!(
        !equal || WHNF_EXHAUSTED.with(|v| v.get()),
        "a true internal comparison after unfinished reduction must retain refusal evidence"
    );
    assert_eq!(WHNF_FUEL.with(|v| v.get()), 0);
    assert_eq!(LOCAL_CONTEXT_BUDGET_DEPTH.with(|v| v.get()), 1);
    // A fresh ordinary allowance must still compute the actual demanded proof.
    WHNF_EXHAUSTED.with(|v| v.set(false));
    WHNF_FUEL.with(|v| v.set(MAX_RECHECK_STEPS));
    assert_eq!(whnf(&c, &dg("left")), dg("seed_left"));
    assert!(!WHNF_EXHAUSTED.with(|v| v.get()));
}

#[test]
fn demanded_proof_whnf_still_unfolds_theorems_and_keeps_opaque_values_sealed() {
    let c = context();
    let _meter = Meter::enter(MAX_RECHECK_STEPS);
    let _scope = PrefixScope::enter(&c);
    assert_eq!(whnf(&c, &dg("left")), dg("seed_left"));
    assert_eq!(
        whnf(&c, &dc("right", vec![Universe::Type(0)])),
        dg("seed_right")
    );
    assert!(
        Meter::count() >= 2,
        "ordinary demanded computation still lowers both proof bodies"
    );
    let count = Meter::count();
    assert_eq!(whnf(&c, &dg("sealed")), dg("sealed"));
    assert_eq!(Meter::count(), count);
    assert!(!WHNF_EXHAUSTED.with(|v| v.get()));
}
