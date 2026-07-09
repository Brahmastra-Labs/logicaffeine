//! Universe consistency of inductive DECLARATIONS (audit fix). A `Type k` inductive may
//! only store fields whose sort is `≤ Type k`; storing a larger type (e.g. `Type 0` inside
//! a `Type 0` inductive) is the Girard/Hurkens inconsistency and must be REJECTED. `Prop`
//! is exempt (impredicative). This matches Coq/Lean, and closes a gap the audit found: the
//! kernel previously registered such a declaration through the "checked" API.

use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{Context, Term, Universe};

fn g(n: &str) -> Term {
    Term::Global(n.to_string())
}
fn arrow(a: Term, b: Term) -> Term {
    Term::Pi { param: "_".to_string(), param_type: Box::new(a), body_type: Box::new(b) }
}
fn ty(n: u32) -> Term {
    Term::Sort(Universe::Type(n))
}
fn std_ctx() -> Context {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    ctx
}

#[test]
fn type0_inductive_cannot_store_a_type0_typed_field() {
    // `Bad : Type 0` with `mk : Type 0 → Bad`. The field's type `Type 0` lives in `Type 1`,
    // so `Bad` would need to be in `Type 1`. Registering it in `Type 0` is universe-
    // inconsistent and MUST be rejected by the checked API.
    let mut ctx = std_ctx();
    ctx.add_inductive("Bad", ty(0));
    let mk = arrow(ty(0), g("Bad"));
    assert!(
        ctx.add_constructor_checked("mk", "Bad", mk).is_err(),
        "a Type-0 inductive storing a Type-0-typed field must be rejected (Girard inconsistency)"
    );
}

#[test]
fn small_fields_in_a_large_inductive_are_accepted() {
    // The other direction is fine: `Box : Type 1` storing a `Type 0` field (`Type 0 ≤ Type 1`).
    let mut ctx = std_ctx();
    ctx.add_inductive("Box", ty(1));
    let wrap = arrow(ty(0), g("Box"));
    assert!(
        ctx.add_constructor_checked("wrap", "Box", wrap).is_ok(),
        "storing a smaller-universe type in a larger inductive is universe-consistent"
    );
}

#[test]
fn ordinary_data_fields_are_accepted() {
    // Regression: don't over-reject. `Wrapper : Type 0` storing a `Nat` (itself `Type 0`)
    // is consistent — `sort(Nat) = Type 0 ≤ Type 0`.
    let mut ctx = std_ctx();
    ctx.add_inductive("Wrapper", ty(0));
    let wrap = arrow(g("Nat"), g("Wrapper"));
    assert!(
        ctx.add_constructor_checked("wrapNat", "Wrapper", wrap).is_ok(),
        "a Type-0 inductive storing a Nat field is fine"
    );
}

#[test]
fn prop_inductive_may_store_large_fields_impredicatively() {
    // `Prop` is impredicative: a proposition may quantify over / store arbitrarily large
    // types (as `Ex`/`And` do). `PBox : Prop` with `pmk : Type 0 → PBox` is accepted.
    let mut ctx = std_ctx();
    ctx.add_inductive("PBox", Term::Sort(Universe::Prop));
    let pmk = arrow(ty(0), g("PBox"));
    assert!(
        ctx.add_constructor_checked("pmk", "PBox", pmk).is_ok(),
        "impredicative Prop admits a large field"
    );
}
