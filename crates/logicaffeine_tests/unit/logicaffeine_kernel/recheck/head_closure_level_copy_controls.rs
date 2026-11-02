//! The optional closure probe must refuse multiplied universe payloads before
//! entering const_body's instantiation/lowering closure, not after allocating it.
use super::*;
fn context() -> Context {
    let mut c = Context::new();
    let u = Universe::Var("u".into());
    let mut body = Term::Sort(u.clone());
    for _ in 0..20 {
        body = Term::Pi {
            param: "A".into(),
            param_type: Box::new(Term::Sort(u.clone())),
            body_type: Box::new(body),
        };
    }
    c.declare_universe_poly(
        "many_sorts",
        vec!["u".into()],
        Term::Sort(Universe::Succ(Box::new(u))),
        body,
    )
    .unwrap();
    c
}
fn instance(u: Universe) -> Db {
    Db::Const {
        name: "many_sorts".into(),
        levels: vec![u],
    }
}
fn expanded(u: Universe) -> Db {
    let mut body = Db::Sort(u.clone());
    for _ in 0..20 {
        body = Db::Pi(
            DbBox::owned(Box::new(Db::Sort(u.clone()))),
            DbBox::owned(Box::new(body)),
        );
    }
    body
}
struct LoweringCounter(Option<usize>);
impl LoweringCounter {
    fn enter() -> Self {
        Self(DEFINITION_LOWERINGS.with(|n| n.replace(Some(0))))
    }
    fn count(&self) -> usize {
        DEFINITION_LOWERINGS.with(|n| n.get().unwrap())
    }
}
impl Drop for LoweringCounter {
    fn drop(&mut self) {
        DEFINITION_LOWERINGS.with(|n| n.set(self.0));
    }
}
#[test]
fn head_closure_small_universe_substitution_still_uses_checked_delta() {
    let c = context();
    let level = Universe::Var("v".into());
    let source = instance(level.clone());
    let target = expanded(level);
    let observed = LoweringCounter::enter();
    let mut fuel = 4096;
    assert!(head_closure::equivalent(&c, &source, &target, &mut fuel));
    assert!(
        observed.count() > 0,
        "the positive fixture must exercise definition lowering"
    );
    assert!(fuel < 4096);
}
#[test]
fn head_closure_multiplied_universe_payload_is_refused_before_lowering() {
    let c = context();
    // Each individual name satisfies the 256-byte atom cap. Its 21 occurrences
    // would require 5376 payload bytes in the expanded body, beyond this call's
    // complete 4096 allowance even before nodes and semantic work.
    let source = instance(Universe::Var("L".repeat(256)));
    let observed = LoweringCounter::enter();
    let mut fuel = 4096;
    assert!(!head_closure::equivalent(
        &c,
        &source,
        &Db::Sort(Universe::Type(0)),
        &mut fuel
    ));
    assert_eq!(
        observed.count(),
        0,
        "post-copy admission is too late: universe expansion must be reserved first"
    );
    assert!(fuel < 4096);
}
