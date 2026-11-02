//! Public syntactic positivity keeps global references opaque, but exposes local beta/Let.
use logicaffeine_kernel::{positivity, KernelError, Term, Universe};
fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn v(n: &str) -> Term {
    Term::Var(n.into())
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
fn wrapped(beta: bool, negative: bool, argument: &str) -> Term {
    let field = if negative {
        pi("_", v("A"), g("Token"))
    } else {
        v("A")
    };
    let body = pi("field", field, g("Bad"));
    if beta {
        Term::App(
            Box::new(Term::Lambda {
                param: "A".into(),
                param_type: Box::new(sort()),
                body: Box::new(body),
            }),
            Box::new(g(argument)),
        )
    } else {
        Term::Let {
            name: "A".into(),
            ty: Box::new(sort()),
            value: Box::new(g(argument)),
            body: Box::new(body),
        }
    }
}
fn check(beta: bool, mutual: bool) {
    for negative in [false, true] {
        let term = wrapped(beta, negative, if mutual { "Other" } else { "Bad" });
        let answer = if mutual {
            positivity::check_positivity_mutual(&["Bad", "Other"], "ctor", &term)
        } else {
            positivity::check_positivity("Bad", "ctor", &term)
        };
        if negative {
            assert!(
                matches!(answer, Err(KernelError::PositivityViolation { .. })),
                "{answer:?}"
            );
        } else {
            assert!(answer.is_ok(), "{answer:?}");
        }
    }
}
#[test]
fn blind_single_beta_checks_positive_and_negative_substitution() {
    check(true, false)
}
#[test]
fn blind_mutual_beta_checks_positive_and_negative_substitution() {
    check(true, true)
}
#[test]
fn blind_single_let_checks_positive_and_negative_substitution() {
    check(false, false)
}
#[test]
fn blind_mutual_let_checks_positive_and_negative_substitution() {
    check(false, true)
}
