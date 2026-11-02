//! Resource boundaries for the optional independent proof-reference rule.
use super::*;

fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn dg(n: &str) -> Db {
    Db::Global(n.into())
}
fn dc(n: &str, levels: Vec<Universe>) -> Db {
    Db::Const {
        name: n.into(),
        levels,
    }
}
fn pi(n: &str, a: Term, b: Term) -> Term {
    Term::Pi {
        param: n.into(),
        param_type: Box::new(a),
        body_type: Box::new(b),
    }
}
fn app(f: Db, a: Db) -> Db {
    Db::App(DbBox::owned(Box::new(f)), DbBox::owned(Box::new(a)))
}
fn base() -> Context {
    let mut c = Context::new();
    c.add_universe_poly_declaration("P", vec![], Term::Sort(Universe::Prop))
        .unwrap();
    c
}
fn checked_pair(c: &mut Context, header: Term, params: Vec<String>) {
    c.add_universe_poly_declaration("seed", vec![], header.clone())
        .unwrap();
    for name in ["left", "right"] {
        c.declare_theorem_checked(name, params.clone(), header.clone(), g("seed"))
            .unwrap();
    }
    assert!(c.pending_obligations().is_empty());
}
fn small() -> Context {
    let mut c = base();
    checked_pair(&mut c, g("P"), vec!["unused".into()]);
    c
}

struct Meter {
    fuel: usize,
    depth: usize,
    local_depth: usize,
    exhausted: bool,
    lowerings: Option<usize>,
}
impl Meter {
    fn enter(fuel: usize) -> Self {
        Self {
            fuel: WHNF_FUEL.with(|v| v.replace(fuel)),
            depth: WHNF_DEPTH.with(|v| v.replace(0)),
            local_depth: LOCAL_CONTEXT_BUDGET_DEPTH.with(|v| v.replace(1)),
            exhausted: WHNF_EXHAUSTED.with(|v| v.replace(false)),
            lowerings: DEFINITION_LOWERINGS.with(|v| v.replace(Some(0))),
        }
    }
    fn fuel() -> usize {
        WHNF_FUEL.with(|v| v.get())
    }
    fn lowerings() -> usize {
        DEFINITION_LOWERINGS.with(|v| v.get().unwrap())
    }
}
impl Drop for Meter {
    fn drop(&mut self) {
        WHNF_FUEL.with(|v| v.set(self.fuel));
        WHNF_DEPTH.with(|v| v.set(self.depth));
        LOCAL_CONTEXT_BUDGET_DEPTH.with(|v| v.set(self.local_depth));
        WHNF_EXHAUSTED.with(|v| v.set(self.exhausted));
        DEFINITION_LOWERINGS.with(|v| v.set(self.lowerings));
    }
}
fn declined(c: &Context, a: Db, b: Db) {
    let _meter = Meter::enter(16_384);
    let _prefix = PrefixScope::enter(c);
    let before = Meter::fuel();
    assert!(!folded::proof_references(c, &[], &a, &b));
    assert!(
        Meter::fuel() <= before,
        "a declined probe cannot refill outer fuel"
    );
    assert!(
        before - Meter::fuel() <= 4096,
        "one optional scope bounds charged reduction work"
    );
    assert!(
        !WHNF_EXHAUSTED.with(|v| v.get()),
        "local refusal must not poison outer checking"
    );
    assert_eq!(LOCAL_CONTEXT_BUDGET_DEPTH.with(|v| v.get()), 1);
    assert_eq!(WHNF_DEPTH.with(|v| v.get()), 0);
    assert_eq!(
        Meter::lowerings(),
        0,
        "header refusal must not lower proof bodies"
    );
}

#[test]
fn checked_small_proof_references_succeed_without_lowering_bodies() {
    let c = small();
    let _meter = Meter::enter(16_384);
    let _prefix = PrefixScope::enter(&c);
    let before = Meter::fuel();
    assert!(folded::proof_references(&c, &[], &dg("left"), &dg("right")));
    assert!(
        Meter::fuel() < before,
        "source copies and checking must consume the shared allowance"
    );
    assert_eq!(Meter::lowerings(), 0);
    assert!(!WHNF_EXHAUSTED.with(|v| v.get()));
}

#[test]
fn a_single_giant_reference_name_is_not_a_one_unit_copy() {
    let mut c = small();
    let giant = "g".repeat(4097);
    c.declare_theorem_checked(&giant, vec![], g("P"), g("seed"))
        .unwrap();
    declined(&c, dg(&giant), dg("right"));
}

#[test]
fn a_single_giant_header_name_is_admitted_before_copying() {
    let mut c = base();
    let giant = "P".repeat(4097);
    c.add_universe_poly_declaration(&giant, vec![], Term::Sort(Universe::Prop))
        .unwrap();
    checked_pair(&mut c, g(&giant), vec![]);
    declined(&c, dg("left"), dg("right"));
}

#[test]
fn an_unused_explicit_level_still_pays_for_its_variable_payload() {
    let c = small();
    declined(
        &c,
        dc("left", vec![Universe::Var("u".repeat(4097))]),
        dc("right", vec![Universe::Type(0)]),
    );
}

#[test]
fn an_unused_declared_parameter_name_still_has_a_copy_bound() {
    let mut c = base();
    checked_pair(&mut c, g("P"), vec!["u".repeat(4097)]);
    declined(
        &c,
        dc("left", vec![Universe::Type(0)]),
        dc("right", vec![Universe::Type(0)]),
    );
}

#[test]
fn many_unused_explicit_parameters_cannot_evade_direct_node_admission() {
    let mut c = base();
    checked_pair(&mut c, g("P"), (0..257).map(|i| format!("u{i}")).collect());
    declined(
        &c,
        dc("left", vec![Universe::Type(0); 257]),
        dc("right", vec![Universe::Type(1); 257]),
    );
}

#[test]
fn a_valid_deep_proposition_header_is_left_to_the_ordinary_checker() {
    let mut c = base();
    let mut header = g("P");
    for i in 0..65 {
        header = pi(&format!("p{i}"), g("P"), header);
    }
    checked_pair(&mut c, header, vec![]);
    declined(&c, dg("left"), dg("right"));
}

#[test]
fn a_malformed_wide_header_is_refused_before_proof_body_work() {
    let mut c = base();
    c.add_universe_poly_declaration("seed", vec![], g("P"))
        .unwrap();
    // Raw definitions are intentional only here: the malformed header and
    // arbitrary seed body are untrusted admission input, not checked proofs.
    // A body is present so the reference probe reaches header admission.
    let header = Term::Match {
        discriminant: Box::new(g("P")),
        motive: Box::new(Term::Sort(Universe::Prop)),
        cases: vec![g("P"); 257],
    };
    c.add_definition("left".into(), header.clone(), g("seed"));
    c.add_definition("right".into(), header, g("seed"));
    declined(&c, dg("left"), dg("right"));
}

#[test]
fn no_quarter_allowance_means_no_evidence_and_no_refill() {
    let c = small();
    for allowance in [0, 1, 3] {
        let _meter = Meter::enter(allowance);
        let _prefix = PrefixScope::enter(&c);
        assert!(!folded::proof_references(
            &c,
            &[],
            &dg("left"),
            &dg("right")
        ));
        assert_eq!(Meter::fuel(), allowance);
        assert_eq!(LOCAL_CONTEXT_BUDGET_DEPTH.with(|v| v.get()), 1);
        assert_eq!(Meter::lowerings(), 0);
    }
}

#[test]
fn existing_exhaustion_cannot_be_cleared_by_optional_proof_comparison() {
    let c = small();
    let _meter = Meter::enter(128);
    let _prefix = PrefixScope::enter(&c);
    WHNF_EXHAUSTED.with(|v| v.set(true));
    assert!(!folded::proof_references(
        &c,
        &[],
        &dg("left"),
        &dg("right")
    ));
    assert!(WHNF_EXHAUSTED.with(|v| v.get()));
    assert_eq!(Meter::fuel(), 128);
}

#[test]
fn nested_common_family_uses_the_existing_scope_without_a_fuel_refill() {
    let mut c = small();
    c.add_universe_poly_declaration(
        "Family",
        vec![],
        pi("p", g("P"), Term::Sort(Universe::Type(0))),
    )
    .unwrap();
    let _meter = Meter::enter(16_384);
    let _prefix = PrefixScope::enter(&c);
    let before = Meter::fuel();
    assert!(folded::equivalent(
        &c,
        &[],
        &app(dg("Family"), dg("left")),
        &app(dg("Family"), dg("right"))
    ));
    let after = Meter::fuel();
    assert!(after < before);
    assert!(
        before - after <= 4096,
        "nested proof checking must use the outer optional scope"
    );
    assert_eq!(LOCAL_CONTEXT_BUDGET_DEPTH.with(|v| v.get()), 1);
    assert_eq!(WHNF_DEPTH.with(|v| v.get()), 0);
    assert!(!WHNF_EXHAUSTED.with(|v| v.get()));
    assert_eq!(Meter::lowerings(), 0);
}
