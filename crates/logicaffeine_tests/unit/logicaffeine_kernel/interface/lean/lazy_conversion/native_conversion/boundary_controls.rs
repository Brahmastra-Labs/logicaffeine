//! Additional resource and capture boundaries for the native conversion bridge.
//! These tests do not grant exceptions to original typing or witness replay.
use super::*;

fn global(name: &str) -> Term {
    Term::Global(name.into())
}
fn variable(name: &str) -> Term {
    Term::Var(name.into())
}
fn sort0() -> Term {
    Term::Sort(Universe::Type(0))
}
fn apply(function: Term, argument: Term) -> Term {
    Term::App(Box::new(function), Box::new(argument))
}
fn lambda(name: &str, domain: Term, body: Term) -> Term {
    Term::Lambda {
        param: name.into(),
        param_type: Box::new(domain),
        body: Box::new(body),
    }
}
fn arrow(name: &str, domain: Term, body: Term) -> Term {
    Term::Pi {
        param: name.into(),
        param_type: Box::new(domain),
        body_type: Box::new(body),
    }
}
fn small_context() -> Context {
    let mut ctx = Context::new();
    ctx.add_declaration("A", sort0());
    ctx.add_declaration("B", sort0());
    ctx
}

#[test]
fn substituted_header_expansion_is_refused_even_with_a_small_exposed_body() {
    let mut ctx = Context::new();
    let u = Universe::Var("u".into());
    let mut header = Term::Sort(u.clone());
    for index in 0..40 {
        header = arrow(&format!("x{index}"), Term::Sort(u.clone()), header);
    }
    // An actual checked alias, not a manually supplied header/body pair.
    // Its original header has 81 terms and 41 universe leaves. The body has
    // only one Const. Instantiating u with nine level nodes expands the
    // HEADER to 369 level occurrences while the body stays small.
    ctx.add_universe_poly_declaration("wide_assumption", vec!["u".into()], header.clone())
        .unwrap();
    ctx.declare_universe_poly(
        "wide_alias",
        vec!["u".into()],
        header,
        Term::Const {
            name: "wide_assumption".into(),
            levels: vec![u],
        },
    )
    .unwrap();
    let mut level = Universe::Type(0);
    for _ in 0..8 {
        level = Universe::Succ(Box::new(level));
    }
    let constant = Term::Const {
        name: "wide_alias".into(),
        levels: vec![level],
    };
    let mut allowance = 100_000;
    let mut certificates = NativeCertificates::new(&ctx, &mut allowance).unwrap();
    assert!(matches!(
        certificates.expose(&constant, &mut allowance),
        Err(Verdict::Decline(_))
    ));
    assert!(allowance < 100_000);
}

#[test]
fn closed_endpoint_shadowing_cannot_capture_the_witness_fixed_names() {
    let ctx = small_context();
    let mut certificates = NativeCertificates::new(&ctx, &mut 100_000).unwrap();
    let names = ["native.P", "native.h", "native.k", "native.x"];
    let expected = lambda(
        "outer",
        sort0(),
        lambda("inner", sort0(), variable("outer")),
    );
    let expected_type = arrow("outer", sort0(), arrow("inner", sort0(), sort0()));
    for (index, name) in names.iter().enumerate() {
        let capture = names[(index + 1) % names.len()];
        // λp. (λh. λp. h) p = λouter. λinner. outer
        // Substitution must retain the outer value despite the repeated p.
        let original = lambda(
            name,
            sort0(),
            apply(
                lambda(capture, sort0(), lambda(name, sort0(), variable(capture))),
                variable(name),
            ),
        );
        certificates
            .conversion(&original, &expected, &expected_type, &mut 100_000)
            .unwrap_or_else(|e| panic!("closed capture with {name}/{capture}: {e:?}"));
    }
}

#[test]
fn a_function_valued_partial_application_is_not_claimed_as_a_ground_result() {
    let ctx = small_context();
    let mut certificates = NativeCertificates::new(&ctx, &mut 100_000).unwrap();
    let original = apply(
        lambda("x", sort0(), lambda("y", sort0(), variable("x"))),
        global("A"),
    );
    assert!(matches!(
        certificates.normalize_ground(&original, &mut 100_000),
        Err(Verdict::Decline(_))
    ));
}

#[test]
fn opaque_neutral_applications_are_not_equal_merely_because_their_types_match() {
    let mut ctx = small_context();
    ctx.add_declaration("a", global("A"));
    ctx.add_declaration("b", global("A"));
    ctx.add_declaration("unknown_function", arrow("x", global("A"), global("A")));
    let mut certificates = NativeCertificates::new(&ctx, &mut 100_000).unwrap();
    let left = apply(global("unknown_function"), global("a"));
    let right = apply(global("unknown_function"), global("b"));
    assert!(matches!(
        certificates.conversion(&left, &right, &global("A"), &mut 100_000),
        Err(Verdict::Reject(_))
    ));
}

#[test]
fn excessive_explicit_level_inventory_is_bounded_without_refilling_allowance() {
    let mut ctx = small_context();
    let u = Term::Sort(Universe::Var("u".into()));
    ctx.declare_universe_poly(
        "poly_identity",
        vec!["u".into()],
        arrow("x", u.clone(), u.clone()),
        lambda("x", u, variable("x")),
    )
    .unwrap();
    let excessive = Term::Const {
        name: "poly_identity".into(),
        levels: vec![Universe::Type(0); 257],
    };
    let mut certificates = NativeCertificates::new(&ctx, &mut 100_000).unwrap();
    let mut allowance = 10_000;
    assert!(certificates.expose(&excessive, &mut allowance).is_err());
    assert!(allowance < 10_000);
    let remaining = allowance;
    assert!(certificates.expose(&excessive, &mut allowance).is_err());
    assert!(allowance <= remaining);
}

#[test]
fn a_shallow_but_large_valid_definition_still_obeys_body_admission() {
    let mut ctx = small_context();
    let mut body = global("A");
    for _ in 0..90 {
        body = Term::Let {
            name: "unused".into(),
            ty: Box::new(sort0()),
            value: Box::new(global("A")),
            body: Box::new(body),
        };
    }
    // Depth is below128; expanded term count is271. The node bound must
    // still apply even though reduction could erase every binding.
    ctx.declare("large_shallow", sort0(), body).unwrap();
    let mut certificates = NativeCertificates::new(&ctx, &mut 100_000).unwrap();
    assert!(matches!(
        certificates.expose(&global("large_shallow"), &mut 100_000),
        Err(Verdict::Decline(_))
    ));
}

#[test]
fn a_large_literal_payload_obeys_a_tiny_materialization_allowance() {
    let mut ctx = Context::new();
    crate::prelude::StandardLibrary::register(&mut ctx);
    let mut value = crate::BigInt::from_u64(2);
    for _ in 0..14 {
        value = value.mul(&value);
    }
    // 2^16384 is valid Nat syntax. Its payload must be charged even though
    // there is only one literal node and its surrounding computation is trivial.
    let huge = Term::Lit(crate::Literal::Nat(value));
    let original = apply(lambda("n", global("Nat"), variable("n")), huge);
    let mut certificates = NativeCertificates::new(&ctx, &mut 100_000).unwrap();
    let mut allowance = 128;
    assert!(matches!(
        certificates.normalize_ground(&original, &mut allowance),
        Err(Verdict::Decline(_))
    ));
    assert!(allowance <= 128);
}

#[test]
fn two_to_the_128_is_normalized_and_certified_without_a_machine_integer_cap() {
    let mut ctx = Context::new();
    crate::prelude::StandardLibrary::register(&mut ctx);
    let half = crate::BigInt::from_u64(1u64 << 63);
    let value = half.mul(&half).mul(&crate::BigInt::from_u64(4));
    let huge = Term::Lit(crate::Literal::Nat(value));
    let original = apply(lambda("n", global("Nat"), variable("n")), huge.clone());
    let mut allowance = 100_000;
    let mut certificates = NativeCertificates::new(&ctx, &mut allowance).unwrap();
    let actual = certificates
        .normalize_ground(&original, &mut allowance)
        .unwrap_or_else(|e| panic!("valid 2^128 Nat identity must fit 100k: {e:?}"));
    assert_eq!(
        actual, huge,
        "every limb of the admitted Nat must survive readback"
    );
    certificates
        .conversion(&original, &actual, &global("Nat"), &mut allowance)
        .unwrap();
    assert!(allowance > 0 && allowance < 100_000);
}
