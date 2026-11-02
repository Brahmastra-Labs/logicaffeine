use logicaffeine_kernel::prelude::{axiom_dependencies, axiom_dependencies_of};
use logicaffeine_kernel::{Context, Term, Universe};
use std::collections::BTreeSet;
fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn c(n: &str, u: Universe) -> Term {
    Term::Const {
        name: n.into(),
        levels: vec![u],
    }
}
fn names(xs: &[&str]) -> BTreeSet<String> {
    xs.iter().map(|x| x.to_string()).collect()
}
#[test]
fn polymorphic_definition_audits_its_body_and_type_transitively() {
    let mut ctx = Context::new();
    ctx.add_declaration("P", Term::Sort(Universe::Prop));
    ctx.add_declaration("assumption", g("P"));
    ctx.declare_universe_poly("first", vec!["u".into()], g("P"), g("assumption"))
        .unwrap();
    ctx.declare_universe_poly(
        "second",
        vec!["v".into()],
        g("P"),
        c("first", Universe::Var("v".into())),
    )
    .unwrap();
    let usage = axiom_dependencies(&ctx, &c("second", Universe::Type(2)));
    assert_eq!(usage.axioms, names(&["P", "assumption"]));
    assert!(usage.unresolved.is_empty());
    assert_eq!(axiom_dependencies_of(&ctx, "second"), Some(usage));
}
/// Lean 4.14's `CollectAxioms` records an axiom without visiting its type: `#print axioms element`
/// for `axiom element : Carrier` is `[element]` (checked with Lean 4.14.0). A declaration whose own
/// statement mentions `Carrier` is charged for it through that statement.
#[test]
fn an_axiom_is_a_leaf_and_a_statement_over_its_type_is_charged_for_it() {
    let mut ctx = Context::new();
    ctx.add_declaration("Carrier", Term::Sort(Universe::Type(0)));
    ctx.add_declaration("element", g("Carrier"));
    let usage = axiom_dependencies(&ctx, &g("element"));
    assert_eq!(usage.axioms, names(&["element"]));
    assert!(usage.unresolved.is_empty());
    assert_eq!(axiom_dependencies_of(&ctx, "element"), Some(usage));
    ctx.declare("named", g("Carrier"), g("element")).unwrap();
    let named = axiom_dependencies_of(&ctx, "named").unwrap();
    assert_eq!(
        named.axioms,
        names(&["Carrier", "element"]),
        "a statement over `Carrier` rests on it"
    );
    assert!(named.unresolved.is_empty());
}
#[test]
fn pure_polymorphic_definition_is_neither_an_axiom_nor_unresolved() {
    let mut ctx = Context::new();
    let u = Universe::Var("u".into());
    let domain = Term::Sort(u.clone());
    let ty = Term::Pi {
        param: "A".into(),
        param_type: Box::new(domain.clone()),
        body_type: Box::new(domain.clone()),
    };
    let body = Term::Lambda {
        param: "A".into(),
        param_type: Box::new(domain),
        body: Box::new(Term::Var("A".into())),
    };
    ctx.declare_universe_poly("identity", vec!["u".into()], ty, body)
        .unwrap();
    let usage = axiom_dependencies(&ctx, &c("identity", Universe::Type(0)));
    assert!(usage.axioms.is_empty());
    assert!(usage.unresolved.is_empty());
    assert_eq!(axiom_dependencies_of(&ctx, "identity"), Some(usage));
    assert!(axiom_dependencies_of(&ctx, "absent").is_none());
}

#[test]
fn pending_definitions_remain_unresolved_until_their_obligations_are_checked() {
    let mut ctx = Context::new();
    let ty = Term::Sort(Universe::Type(0));
    ctx.declare("pending", ty.clone(), g("Missing")).unwrap();
    assert!(!ctx.pending_obligations().is_empty());
    let unresolved = axiom_dependencies(&ctx, &g("pending"));
    assert_eq!(unresolved.unresolved, names(&["pending"]));
    assert!(unresolved.axioms.is_empty());
    ctx.add_declaration("Missing", ty);
    assert!(ctx.pending_obligations().is_empty());
    let resolved = axiom_dependencies(&ctx, &g("pending"));
    assert_eq!(resolved.axioms, names(&["Missing"]));
    assert!(resolved.unresolved.is_empty());
}
