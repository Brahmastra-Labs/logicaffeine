use logicaffeine_kernel::quotient::{install, Kind};
use logicaffeine_kernel::{axiom_dependencies, Context, Term, Universe};
fn instance(kind: Kind) -> Term {
    Term::Const {
        name: kind.name().into(),
        levels: vec![Universe::Type(0)],
    }
}
#[test]
fn certified_quotient_computation_primitives_are_not_extra_logical_axioms() {
    let mut ctx = Context::new();
    install(&mut ctx, Kind::Type, vec!["u".into()], "Eq").unwrap();
    install(&mut ctx, Kind::Constructor, vec!["u".into()], "Eq").unwrap();
    install(&mut ctx, Kind::Induction, vec!["u".into()], "Eq").unwrap();
    for kind in [Kind::Type, Kind::Constructor, Kind::Induction] {
        let usage = axiom_dependencies(&ctx, &instance(kind));
        assert!(
            usage.axioms.is_empty() && usage.unresolved.is_empty(),
            "{kind:?}: {usage:?}"
        );
    }
}
#[test]
fn an_uncertified_same_spelled_quotient_declaration_remains_an_assumption() {
    let mut ctx = Context::new();
    ctx.add_declaration(Kind::Type.name(), Term::Sort(Universe::Type(0)));
    let usage = axiom_dependencies(&ctx, &Term::Global(Kind::Type.name().into()));
    assert_eq!(
        usage.axioms.into_iter().collect::<Vec<_>>(),
        vec![Kind::Type.name()]
    );
}
#[test]
fn a_quotient_dependency_cannot_be_replaced_under_its_certified_primitives() {
    // Replacing the quotient type under the certified constructor is a change of meaning the
    // stable-meaning gate refuses (recorded, since `add_declaration` cannot answer), so both
    // primitives keep their certified classification. The revocation of a certificate whose
    // dependency does change is locked in the kernel's own unit tests with the change permitted
    // (`quotient::tests::replacing_quotient_or_equality_dependencies_revokes_computation_certificates`).
    let mut ctx = Context::new();
    install(&mut ctx, Kind::Type, vec!["u".into()], "Eq").unwrap();
    install(&mut ctx, Kind::Constructor, vec!["u".into()], "Eq").unwrap();
    ctx.add_declaration(Kind::Type.name(), Term::Sort(Universe::Type(0)));
    assert!(
        ctx.refused_declarations()
            .iter()
            .any(|(n, why)| n == Kind::Type.name() && why.starts_with("Stable meaning violation")),
        "{:?}",
        ctx.refused_declarations()
    );
    for kind in [Kind::Type, Kind::Constructor] {
        let usage = axiom_dependencies(&ctx, &instance(kind));
        assert!(
            usage.axioms.is_empty() && usage.unresolved.is_empty(),
            "{kind:?}: {usage:?}"
        );
    }
}

#[test]
fn named_and_term_audits_agree_for_certified_quotient_primitives() {
    let mut ctx = Context::new();
    install(&mut ctx, Kind::Type, vec!["u".into()], "Eq").unwrap();
    install(&mut ctx, Kind::Constructor, vec!["u".into()], "Eq").unwrap();
    install(&mut ctx, Kind::Induction, vec!["u".into()], "Eq").unwrap();
    for kind in [Kind::Type, Kind::Constructor, Kind::Induction] {
        let named = logicaffeine_kernel::axiom_dependencies_of(&ctx, kind.name()).unwrap();
        let term = axiom_dependencies(&ctx, &instance(kind));
        assert!(
            named.axioms.is_empty() && named.unresolved.is_empty(),
            "{kind:?}: {named:?}"
        );
        assert_eq!(named.axioms, term.axioms);
        assert_eq!(named.unresolved, term.unresolved);
    }
}
