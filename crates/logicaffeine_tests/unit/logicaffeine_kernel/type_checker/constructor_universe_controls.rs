//! The constructor universe constraint at its two doors — the checked signature walk's
//! `check_constructor_field_universe` and the standalone `check_constructor_universes` the
//! compiled inductives pass through — exempting the same sorts: the impredicative ones.
use super::*;
use crate::context::Context;
use crate::term::{Term, Universe};

fn g(s: &str) -> Term {
    Term::Global(s.to_string())
}
fn arrow(a: Term, b: Term) -> Term {
    Term::Pi {
        param: "_".to_string(),
        param_type: Box::new(a),
        body_type: Box::new(b),
    }
}

/// A large field, `Type 0` (a sort in `Type 1`), against each target sort: stored by the
/// impredicative ones, refused by a predicative one below it.
const TARGETS: [(Universe, bool); 4] = [
    (Universe::SProp, true),
    (Universe::Prop, true),
    (Universe::Type(0), false),
    (Universe::Type(1), true),
];

#[test]
fn the_signature_walk_stores_a_large_field_exactly_in_the_impredicative_sorts_and_above_it() {
    let ctx = Context::new();
    for (target, stored) in TARGETS {
        let verdict = check_constructor_field_universe(
            &ctx,
            "I",
            "mk",
            &Term::Sort(Universe::Type(0)),
            &target,
        );
        assert_eq!(
            verdict.is_ok(),
            stored,
            "a `Type 0` field in a family in {target}: {verdict:?}"
        );
    }
}

#[test]
fn the_standalone_gate_stores_a_large_field_exactly_in_the_impredicative_sorts_and_above_it() {
    for (i, (target, stored)) in TARGETS.into_iter().enumerate() {
        let name = format!("Hold{i}");
        let mut ctx = Context::new();
        ctx.add_inductive(&name, Term::Sort(target.clone()));
        let verdict = check_constructor_universes(
            &ctx,
            &name,
            "mk",
            &arrow(Term::Sort(Universe::Type(0)), g(&name)),
        );
        assert_eq!(
            verdict.is_ok(),
            stored,
            "a `Type 0` field in a family in {target}: {verdict:?}"
        );
    }
}
