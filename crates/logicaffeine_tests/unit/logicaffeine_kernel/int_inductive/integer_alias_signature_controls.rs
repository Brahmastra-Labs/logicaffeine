use super::*;
#[test]
fn public_integer_aliases_allow_cosmetic_parameter_renaming() {
    for (name, target) in [("add", "Int_add"), ("mul", "Int_mul"), ("sub", "Int_sub")] {
        let mut ctx = Context::new();
        crate::prelude::StandardLibrary::register(&mut ctx);
        assert!(accelerates_integer_alias(&ctx, name));
        ctx.declare(
            name,
            pi("left", int(), pi("right", int(), int())),
            g(target),
        )
        .unwrap();
        assert!(accelerates_integer_alias(&ctx, name), "{name}");
        assert!(
            ctx.refused_declarations().is_empty(),
            "a binder rename is not a change of meaning"
        );
    }
}
#[test]
fn integer_binary_signature_accepts_named_anonymous_and_shadowing_spelling() {
    for (first, second) in [("_", "_"), ("a", "b"), ("value", "value"), ("Int", "Int")] {
        assert!(
            has_integer_binop_signature(&pi(first, int(), pi(second, int(), int()))),
            "{first}/{second}"
        );
    }
}
#[test]
fn integer_binary_signature_refuses_wrong_domains_results_dependencies_and_arity() {
    for ty in [
        pi("a", g("Bool"), pi("b", int(), int())),
        pi("a", int(), pi("b", g("Bool"), int())),
        pi("a", int(), pi("b", int(), g("Bool"))),
        pi("a", int(), pi("b", int(), v("a"))),
        pi("a", int(), pi("b", v("a"), int())),
        pi("a", int(), pi("b", int(), pi("c", int(), int()))),
        arrow(int(), int()),
        int(),
    ] {
        assert!(!has_integer_binop_signature(&ty), "{ty}");
    }
}
