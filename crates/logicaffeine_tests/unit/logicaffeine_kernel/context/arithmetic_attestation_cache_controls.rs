use super::*;
use std::cell::Cell;

#[test]
fn arithmetic_cache_owns_only_weak_registry_snapshots_and_reuses_local_contexts() {
    let ctx = Context::new();
    let before = Arc::strong_count(&ctx.definitions);
    let validations = Cell::new(0);
    let verify = || {
        validations.set(validations.get() + 1);
        true
    };
    assert!(ctx.cached_integer_arithmetic("add", verify));
    assert_eq!(Arc::strong_count(&ctx.definitions), before);
    assert!(Arc::weak_count(&ctx.definitions) > 0);
    let local = ctx.extend("x", Term::Sort(Universe::Type(0)));
    assert!(local.cached_integer_arithmetic("add", verify));
    assert_eq!(
        validations.get(),
        1,
        "local binders do not change closed arithmetic definitions"
    );
}

#[test]
fn arithmetic_cache_revalidates_after_unique_owner_or_cloned_registry_mutation() {
    let mut ctx = Context::new();
    let validations = Cell::new(0);
    let verify = || {
        validations.set(validations.get() + 1);
        true
    };
    assert!(ctx.cached_integer_arithmetic("add", verify));
    let old = Arc::downgrade(&ctx.definitions);
    Arc::make_mut(&mut ctx.definitions).insert(
        "f".into(),
        Arc::new((
            vec![],
            Term::Sort(Universe::Type(0)),
            Term::Sort(Universe::Prop),
        )),
    );
    assert!(
        old.upgrade().is_none(),
        "weak observers must not retain the old map payload"
    );
    assert!(ctx.cached_integer_arithmetic("add", verify));
    assert_eq!(validations.get(), 2);
    let mut child = ctx.clone();
    Arc::make_mut(&mut child.definitions).clear();
    assert!(child.cached_integer_arithmetic("add", verify));
    assert_eq!(validations.get(), 3);
    assert!(ctx.cached_integer_arithmetic("add", verify));
    assert_eq!(
        validations.get(),
        4,
        "a shared one-entry cache must recognize the parent's generation"
    );
}

#[test]
fn arithmetic_cache_tracks_constructor_and_parameter_registries_separately() {
    let mut ctx = Context::new();
    let validations = Cell::new(0);
    let verify = || {
        validations.set(validations.get() + 1);
        true
    };
    assert!(ctx.cached_integer_arithmetic("mul", verify));
    Arc::make_mut(&mut ctx.constructors)
        .insert("Zero".into(), ("Nat".into(), Term::Global("Nat".into())));
    assert!(ctx.cached_integer_arithmetic("mul", verify));
    Arc::make_mut(&mut ctx.constructor_order).insert("Nat".into(), vec!["Zero".into()]);
    assert!(ctx.cached_integer_arithmetic("mul", verify));
    Arc::make_mut(&mut ctx.inductive_params).insert("Nat".into(), 1);
    assert!(ctx.cached_integer_arithmetic("mul", verify));
    Arc::make_mut(&mut ctx.inductive_inferred_params).insert("Nat".into(), 2);
    assert!(ctx.cached_integer_arithmetic("mul", verify));
    assert_eq!(validations.get(), 5);
}

#[test]
fn arithmetic_cache_tracks_pending_polymorphic_and_header_registry_changes() {
    let mut ctx = Context::new();
    let validations = Cell::new(0);
    let verify = || {
        validations.set(validations.get() + 1);
        true
    };
    let prop = Term::Sort(Universe::Prop);
    assert!(ctx.cached_integer_arithmetic("add", verify));
    Arc::make_mut(&mut ctx.pending).insert("add".into(), "missing".into());
    assert!(ctx.cached_integer_arithmetic("add", verify));
    Arc::make_mut(&mut ctx.pending).clear();
    assert!(ctx.cached_integer_arithmetic("add", verify));
    Arc::make_mut(&mut ctx.poly_types).insert("add".into(), (vec![], prop.clone()));
    assert!(ctx.cached_integer_arithmetic("add", verify));
    Arc::make_mut(&mut ctx.universe_polys)
        .insert("add".into(), Arc::new((vec![], prop.clone(), prop.clone())));
    assert!(ctx.cached_integer_arithmetic("add", verify));
    Arc::make_mut(&mut ctx.declarations).insert("add".into(), prop.clone());
    assert!(ctx.cached_integer_arithmetic("add", verify));
    Arc::make_mut(&mut ctx.inductives).insert("Nat".into(), prop);
    assert!(ctx.cached_integer_arithmetic("add", verify));
    assert_eq!(validations.get(), 7);
}

#[test]
fn arithmetic_cache_keeps_add_and_mul_attestations_distinct_and_caches_failures() {
    let ctx = Context::new();
    let validations = Cell::new(0);
    assert!(ctx.cached_integer_arithmetic("add", || {
        validations.set(validations.get() + 1);
        true
    }));
    assert!(!ctx.cached_integer_arithmetic("mul", || {
        validations.set(validations.get() + 1);
        false
    }));
    assert!(ctx.cached_integer_arithmetic("add", || panic!("add attestation should be cached")));
    assert!(
        !ctx.cached_integer_arithmetic("mul", || panic!("failed mul attestation should be cached"))
    );
    assert_eq!(validations.get(), 2);
}

/// The canonical-numerals answer lives exactly as long as the registries it read, a question
/// asked at every step: `Zero` then `Succ` registered read `0` then every literal, a third
/// constructor keeps the readings and loses the exact family, `Succ` assumed in its constructor's
/// place reads `0` alone, and a clone taken before keeps the answer for the registries it holds.
#[test]
fn the_canonical_numerals_follow_every_registration() {
    let nat = || Term::Global("Nat".into());
    let unary = || Term::Pi {
        param: "n".into(),
        param_type: Box::new(nat()),
        body_type: Box::new(nat()),
    };
    let numerals = |zero, succ, nat, int| CanonicalNumerals {
        zero,
        succ,
        nat,
        int,
    };
    let mut c = Context::new();
    assert_eq!(c.canonical_numerals(), numerals(false, false, false, false));
    c.add_inductive("Nat", Term::Sort(crate::term::Universe::Type(0)));
    c.add_constructor("Zero", "Nat", nat());
    assert_eq!(c.canonical_numerals(), numerals(true, false, false, false));
    c.add_constructor("Succ", "Nat", unary());
    assert_eq!(c.canonical_numerals(), numerals(true, true, true, false));
    let canonical = c.clone();
    c.add_constructor("Other", "Nat", unary());
    assert_eq!(
        c.canonical_numerals(),
        numerals(true, true, false, false),
        "a third constructor keeps both readings, not the exact family"
    );
    c.add_declaration("Succ", unary());
    assert_eq!(
        c.canonical_numerals(),
        numerals(true, false, false, false),
        "an assumed `Succ` is no constructor"
    );
    assert!(
        c.refused_declarations().is_empty(),
        "{:?}",
        c.refused_declarations()
    );
    assert_eq!(
        canonical.canonical_numerals(),
        numerals(true, true, true, false),
        "a clone keeps the answer for the registries it holds"
    );
    let mut other = Context::new();
    other.add_inductive("Nat", Term::Sort(crate::term::Universe::Type(0)));
    other.add_constructor("Z", "Nat", nat());
    assert_eq!(
        other.canonical_numerals(),
        numerals(false, false, false, false),
        "a `Nat` without `Zero` reads no literal"
    );
}
