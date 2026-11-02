use super::*;
fn context() -> Context {
    let mut c = Context::new();
    crate::prelude::StandardLibrary::register(&mut c);
    c
}
#[test]
fn checked_division_attests_each_exact_definition_dependency() {
    let base = context();
    for op in ["div", "mod"] {
        assert!(accelerates_integer_alias(&base, op));
        let target = if op == "div" { "Int_div" } else { "Int_mod" };
        let projection = if op == "div" { "Prod_fst" } else { "Prod_snd" };
        for name in [
            op,
            target,
            projection,
            "Int_divMod",
            "Nat_divMod",
            "Nat_divModSucc",
            "Nat_sub",
            "Nat_pred",
            "Int_negOfNat",
            "natLt",
            "natLe",
        ] {
            let mut c = base.clone();
            let ty = c.get_definition_type(name).unwrap().clone();
            let body = c.get_definition_body(name).unwrap().clone();
            let wrapped = app(lam("wrapped", ty.clone(), v("wrapped")), body);
            if name != op {
                super::checked_alias_attestation_controls::gate_keeps(&c, op, |c| {
                    c.declare(name, ty.clone(), wrapped.clone())
                });
            }
            c.permit_meaning_changes_for_cache_tests();
            c.declare(name, ty, wrapped).unwrap();
            assert!(!accelerates_integer_alias(&c, op), "{op}: {name}");
        }
    }
}
#[test]
fn checked_division_invalidates_for_product_and_carrier_inventory_changes() {
    let base = context();
    for op in ["div", "mod"] {
        assert!(accelerates_integer_alias(&base, op));
        for carrier in ["Nat", "Int", "Bool", "Prod"] {
            let mut c = base.clone();
            super::checked_alias_attestation_controls::gate_keeps(&c, op, |c| {
                c.set_inductive_params(carrier, 9);
                Ok(())
            });
            c.permit_meaning_changes_for_cache_tests();
            c.set_inductive_params(carrier, 9);
            assert!(!accelerates_integer_alias(&c, op), "{op}: {carrier}");
        }
        // `Prod` is a structure: its η rule depends on its one constructor, so a second one is
        // refused whatever depends on it — even where changes of meaning are permitted — and
        // the attested division keeps its meaning.
        let mut c = base.clone();
        let extra = pi(
            "A",
            Term::Sort(crate::Universe::Type(0)),
            pi(
                "B",
                Term::Sort(crate::Universe::Type(0)),
                apps(g("Prod"), &[v("A"), v("B")]),
            ),
        );
        c.permit_meaning_changes_for_cache_tests();
        assert!(matches!(
            c.add_constructor_checked("ExtraProd", "Prod", extra),
            Err(crate::error::KernelError::InvalidDeclaration(_))
        ));
        assert_eq!(c.get_constructors("Prod").len(), 1);
        assert!(accelerates_integer_alias(&c, op));
        for name in ["true", "false"] {
            let mut c = base.clone();
            let other = g(if name == "true" { "false" } else { "true" });
            super::checked_alias_attestation_controls::gate_keeps(&c, op, |c| {
                c.declare(name, g("Bool"), other.clone())
            });
            c.permit_meaning_changes_for_cache_tests();
            c.declare(name, g("Bool"), other).unwrap();
            assert!(
                !accelerates_integer_alias(&c, op),
                "{op}: {name} definition"
            );
        }
    }
}
#[test]
fn checked_division_rejects_polymorphic_and_visible_header_collisions() {
    let base = context();
    for op in ["div", "mod"] {
        assert!(accelerates_integer_alias(&base, op));
        for name in [
            op,
            "Int_divMod",
            "Nat_divMod",
            "natLt",
            "Prod_fst",
            "Prod_snd",
        ] {
            let mut c = base.clone();
            let ty = c.get_definition_type(name).unwrap().clone();
            let body = c.get_definition_body(name).unwrap().clone();
            if name != op {
                super::checked_alias_attestation_controls::gate_keeps(&c, op, |c| {
                    c.declare_universe_poly(name, vec![], ty.clone(), body.clone())
                });
            }
            c.permit_meaning_changes_for_cache_tests();
            c.declare_universe_poly(name, vec![], ty, body).unwrap();
            assert!(!accelerates_integer_alias(&c, op), "{op}: {name}");
        }
        let mut c = base.clone();
        c.permit_meaning_changes_for_cache_tests();
        c.add_constructor(op, "Nat", nat());
        assert!(!accelerates_integer_alias(&c, op));
    }
}
#[test]
fn checked_division_cache_distinguishes_operations_and_failed_replacements() {
    let mut c = context();
    assert!(accelerates_integer_alias(&c, "div"));
    assert!(accelerates_integer_alias(&c, "mod"));
    assert!(c.declare("Int_div", int_binop(), g("Zero")).is_err());
    assert!(accelerates_integer_alias(&c, "div"));
    c.declare("div", int_binop(), lam("a", int(), lam("b", int(), v("a"))))
        .unwrap();
    assert!(!accelerates_integer_alias(&c, "div"));
    assert!(accelerates_integer_alias(&c, "mod"));
    assert!(accelerates_integer_alias(&c.extend("local", int()), "mod"));
}
#[test]
fn checked_division_rejects_pending_dependencies_and_registered_rewrites() {
    let mut c = context();
    assert!(accelerates_integer_alias(&c, "div"));
    super::checked_alias_attestation_controls::gate_keeps(&c, "div", |c| {
        c.declare("Int_div", int_binop(), g("future_division"))
    });
    c.permit_meaning_changes_for_cache_tests();
    c.declare("Int_div", int_binop(), g("future_division"))
        .unwrap();
    assert!(!accelerates_integer_alias(&c, "div"));
    c.declare(
        "future_division",
        int_binop(),
        lam("a", int(), lam("b", int(), v("a"))),
    )
    .unwrap();
    assert!(!accelerates_integer_alias(&c, "div"));
    let mut c = context();
    assert!(accelerates_integer_alias(&c, "mod"));
    let eq = apps(
        g("Eq"),
        &[
            nat(),
            apps(g("Nat_add"), &[g("Zero"), g("Zero")]),
            g("Zero"),
        ],
    );
    c.declare(
        "division_zero_add_zero",
        eq,
        apps(g("refl"), &[nat(), g("Zero")]),
    )
    .unwrap();
    c.add_simp_lemma("division_zero_add_zero").unwrap();
    assert!(!accelerates_integer_alias(&c, "div"));
    assert!(!accelerates_integer_alias(&c, "mod"));
}
