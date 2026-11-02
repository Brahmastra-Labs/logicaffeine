//! Same real search, with its existing audit as the deterministic attempt counter.
use super::*;
use crate::Universe;
fn g(s: &str) -> Term {
    Term::Global(s.into())
}
fn v(s: &str) -> Term {
    Term::Var(s.into())
}
fn sort() -> Term {
    Term::Sort(Universe::Type(0))
}
fn pi(n: &str, a: Term, b: Term) -> Term {
    Term::Pi {
        param: n.into(),
        param_type: Box::new(a),
        body_type: Box::new(b),
    }
}
fn app(f: Term, a: Term) -> Term {
    Term::App(Box::new(f), Box::new(a))
}
fn setup() -> Context {
    let mut c = Context::new();
    for name in ["A", "B"] {
        c.add_declaration(name, sort());
    }
    for (name, ty) in [("a", "A"), ("a2", "A"), ("b", "B")] {
        c.add_declaration(name, g(ty));
    }
    c
}
fn checked(c: &Context, goal: &Term, answer: &Term) {
    assert!(crate::is_subtype(
        c,
        &crate::infer_type(c, answer).unwrap(),
        goal
    ));
    assert!(crate::is_subtype(
        c,
        &crate::recheck(c, answer).unwrap(),
        goal
    ));
}
fn observed(c: &Context, goal: &Term) -> (Option<(Term, InstanceTier, i64)>, Vec<InstanceProbe>) {
    let mut attempts = Vec::new();
    let answer = search_instance(c, &mut MetaCtx::new(), goal, 0, Some(&mut attempts), None);
    (answer, attempts)
}
#[test]
fn unrelated_rigid_fallback_never_reaches_unification() {
    let mut c = setup();
    c.add_fallback_instance(g("A"), g("a"));
    c.add_fallback_instance(g("B"), g("b"));
    let result = resolve_instance_audited(&c, &mut MetaCtx::new(), &g("A")).unwrap();
    assert_eq!(result.value, g("a"));
    assert_eq!(result.tier, InstanceTier::Fallback { index: 0 });
    assert_eq!(
        result.considered.len(),
        1,
        "irrelevant rigid fallback must not be attempted"
    );
    assert_eq!(
        result.considered[0].tier,
        InstanceTier::Fallback { index: 0 }
    );
    assert!(result.considered[0].applied);
    checked(&c, &g("A"), &result.value);
}
#[test]
fn polymorphic_const_head_uses_the_same_fallback_discrimination() {
    let mut c = setup();
    c.add_universe_poly_declaration("Poly", vec!["u".into()], sort())
        .unwrap();
    let goal = Term::Const {
        name: "Poly".into(),
        levels: vec![Universe::Type(0)],
    };
    c.add_declaration("poly_value", goal.clone());
    c.add_fallback_instance(goal.clone(), g("poly_value"));
    c.add_fallback_instance(g("B"), g("b"));
    let result = resolve_instance_audited(&c, &mut MetaCtx::new(), &goal).unwrap();
    assert_eq!(result.value, g("poly_value"));
    assert_eq!(result.considered.len(), 1);
    checked(&c, &goal, &result.value);
}
#[test]
fn same_head_fallbacks_keep_later_first_order() {
    let mut c = setup();
    c.add_fallback_instance(g("A"), g("a"));
    c.add_fallback_instance(g("A"), g("a2"));
    let result = resolve_instance_audited(&c, &mut MetaCtx::new(), &g("A")).unwrap();
    assert_eq!(result.value, g("a2"));
    assert_eq!(result.tier, InstanceTier::Fallback { index: 1 });
    assert_eq!(result.considered.len(), 1);
}
#[test]
fn declared_instance_still_outranks_later_fallback() {
    let mut c = setup();
    c.add_instance_with_priority(g("A"), g("a"), -1000);
    c.add_fallback_instance(g("A"), g("a2"));
    let result = resolve_instance_audited(&c, &mut MetaCtx::new(), &g("A")).unwrap();
    assert_eq!(result.value, g("a"));
    assert_eq!(result.tier, InstanceTier::Declared { index: 0 });
    assert_eq!(result.considered.len(), 1);
}
#[test]
fn reducible_candidate_head_is_never_pruned() {
    let mut c = setup();
    c.declare("Alias", sort(), g("A")).unwrap();
    c.add_fallback_instance(g("Alias"), g("a"));
    let result = resolve_instance_audited(&c, &mut MetaCtx::new(), &g("A")).unwrap();
    assert_eq!(result.value, g("a"));
    assert_eq!(result.considered.len(), 1);
    checked(&c, &g("A"), &result.value);
}
#[test]
fn reducible_goal_keeps_the_full_candidate_tier() {
    let mut c = setup();
    c.declare("Alias", sort(), g("A")).unwrap();
    c.add_fallback_instance(g("A"), g("a"));
    c.add_fallback_instance(g("B"), g("b"));
    let result = resolve_instance_audited(&c, &mut MetaCtx::new(), &g("Alias")).unwrap();
    assert_eq!(result.value, g("a"));
    assert_eq!(result.considered.len(), 2);
    checked(&c, &g("Alias"), &result.value);
}
#[test]
fn metavariable_goal_keeps_later_candidate_and_commits_its_solution() {
    let mut c = setup();
    c.add_fallback_instance(g("A"), g("a"));
    c.add_fallback_instance(g("B"), g("b"));
    let mut m = MetaCtx::new();
    let goal = m.fresh();
    let result = resolve_instance_audited(&c, &mut m, &goal).unwrap();
    assert_eq!(result.value, g("b"));
    assert_eq!(instantiate(&goal, &m), g("B"));
    assert_eq!(result.tier, InstanceTier::Fallback { index: 1 });
}
#[test]
fn bound_conclusion_head_is_kept_and_freshened() {
    let mut c = setup();
    let ty = pi("T", sort(), v("T"));
    c.add_declaration("generic", ty.clone());
    c.add_fallback_instance(ty, g("generic"));
    let result = resolve_instance_audited(&c, &mut MetaCtx::new(), &g("A")).unwrap();
    assert_eq!(result.value, app(g("generic"), g("A")));
    assert_eq!(result.considered.len(), 1);
    checked(&c, &g("A"), &result.value);
}
#[test]
fn unknown_beta_head_is_kept_without_pre_normalizing_it() {
    let mut c = setup();
    let beta = app(
        Term::Lambda {
            param: "T".into(),
            param_type: Box::new(sort()),
            body: Box::new(v("T")),
        },
        g("A"),
    );
    c.add_fallback_instance(beta, g("a"));
    let result = resolve_instance_audited(&c, &mut MetaCtx::new(), &g("A")).unwrap();
    assert_eq!(result.value, g("a"));
    assert_eq!(result.considered.len(), 1);
    checked(&c, &g("A"), &result.value);
}
#[test]
fn fallback_classification_follows_current_context_after_definition_replacement() {
    let mut c = setup();
    c.add_declaration("Later", sort());
    c.add_fallback_instance(g("Later"), g("a"));
    let (answer, attempts) = observed(&c, &g("A"));
    assert!(answer.is_none());
    assert!(attempts.is_empty());
    c.declare("Later", sort(), g("A")).unwrap();
    assert!(c.get_definition_body("Later").is_some());
    let result = resolve_instance_audited(&c, &mut MetaCtx::new(), &g("A")).unwrap();
    assert_eq!(result.value, g("a"));
    assert_eq!(result.considered.len(), 1);
    c.add_declaration("Later", sort());
    assert!(c.get_definition_body("Later").is_none());
    let (answer, attempts) = observed(&c, &g("A"));
    assert!(answer.is_none());
    assert!(attempts.is_empty());
}
#[test]
fn scope_pop_restores_fallback_without_stale_candidate_order() {
    let mut c = setup();
    c.add_fallback_instance(g("A"), g("a"));
    c.add_fallback_instance(g("B"), g("b"));
    c.push_scope();
    c.add_local_instance(g("A"), g("a2"));
    assert_eq!(
        resolve_instance(&c, &mut MetaCtx::new(), &g("A")),
        Some(g("a2"))
    );
    c.pop_scope();
    let result = resolve_instance_audited(&c, &mut MetaCtx::new(), &g("A")).unwrap();
    assert_eq!(result.value, g("a"));
    assert_eq!(result.considered.len(), 1);
    assert_eq!(result.tier, InstanceTier::Fallback { index: 0 });
}
#[test]
fn pruned_failure_never_solves_or_allocates_goal_metavariables() {
    let mut c = setup();
    c.add_fallback_instance(g("B"), g("b"));
    let mut m = MetaCtx::new();
    let keep = m.fresh();
    let counter = m.counter;
    let solutions = m.solutions.clone();
    let mut attempts = Vec::new();
    assert!(search_instance(&c, &mut m, &g("A"), 0, Some(&mut attempts), None).is_none());
    assert!(attempts.is_empty());
    assert_eq!(m.counter, counter);
    assert_eq!(m.solutions, solutions);
    assert_eq!(instantiate(&keep, &m), keep);
}
#[test]
fn binder_shadowing_a_global_name_remains_an_open_candidate() {
    let mut c = setup();
    let ty = pi("A", sort(), g("A"));
    c.add_fallback_instance(ty, g("a"));
    let (answer, attempts) = observed(&c, &g("B"));
    assert!(answer.is_none());
    assert_eq!(
        attempts.len(),
        1,
        "shadowed global-looking binder head must remain open"
    );
}
#[test]
fn metavariable_candidate_head_is_not_pruned() {
    let mut c = setup();
    c.add_fallback_instance(v("?external"), g("a"));
    let result = resolve_instance_audited(&c, &mut MetaCtx::new(), &g("A")).unwrap();
    assert_eq!(result.value, g("a"));
    assert_eq!(result.considered.len(), 1);
}
