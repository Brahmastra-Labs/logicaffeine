//! The adapter owns its outer typing stack, before cached telescope recursion.
use super::*;
use crate::term_store::{within_kernel_depth, BASE_STACK_BYTES};

fn universe(level: u32) -> Term {
    Term::Sort(Universe::Type(level))
}
fn lambda(name: String, domain: Term, body: Term) -> Term {
    Term::Lambda {
        param: name,
        param_type: Box::new(domain),
        body: Box::new(body),
    }
}
fn pi(name: String, domain: Term, body: Term) -> Term {
    Term::Pi {
        param: name,
        param_type: Box::new(domain),
        body_type: Box::new(body),
    }
}

// Fix the caller's stack to the ordinary kernel base allocation, even if the
// surrounding test runner was launched with a larger RUST_MIN_STACK setting.
fn on_ordinary_stack(check: impl FnOnce() + Send + 'static) {
    std::thread::Builder::new()
        .name("lean-type-stack-control".into())
        .stack_size(BASE_STACK_BYTES)
        .spawn(check)
        .expect("start isolated ordinary-stack control")
        .join()
        .unwrap_or_else(|panic| std::panic::resume_unwind(panic));
}

fn check_both(ctx: &Context, term: &Term, expected: &Term) {
    let actual = lean_type(ctx, term).expect("adapter must type the admitted telescope");
    assert_eq!(
        &actual, expected,
        "adapter must retain the complete function type"
    );
    let independent =
        crate::recheck(ctx, term).expect("independent checker must type the same telescope");
    assert!(
        definitionally_equal(ctx, &actual, &independent),
        "the sized adapter call must agree with independent typing"
    );
}

#[test]
fn valid_eighty_binder_telescope_types_on_an_ordinary_caller_stack() {
    on_ordinary_stack(|| {
        let ctx = Context::new();
        let mut term = Term::Var("stack.binder.0".into());
        let mut expected = universe(0);
        for index in (0..80).rev() {
            let name = format!("stack.binder.{index}");
            term = lambda(name.clone(), universe(0), term);
            expected = pi(name, universe(0), expected);
        }
        assert!(within_kernel_depth(&term));
        check_both(&ctx, &term, &expected);
    });
}

#[test]
fn deep_shadowed_binders_keep_the_innermost_domain_on_the_sized_stack() {
    on_ordinary_stack(|| {
        let ctx = Context::new();
        let mut term = Term::Var("shadow".into());
        // The innermost binder has Type 1 as its domain, while its immediate
        // predecessor has Type 0. Reusing a name cannot reuse that older type.
        let mut expected = universe(1);
        for index in (0..80).rev() {
            let domain = universe((index % 2) as u32);
            term = lambda("shadow".into(), domain.clone(), term);
            expected = pi("shadow".into(), domain, expected);
        }
        assert!(within_kernel_depth(&term));
        check_both(&ctx, &term, &expected);
    });
}

// The over-limit input must not abort later merely because the test drops its
// recursive Boxes on the ordinary caller stack. Its domains are shallow sorts.
fn dispose_lambda_chain(mut term: Term) {
    loop {
        match term {
            Term::Lambda {
                param_type, body, ..
            } => {
                drop(param_type);
                term = *body;
            }
            leaf => {
                drop(leaf);
                break;
            }
        }
    }
}

#[test]
fn first_over_limit_telescope_declines_then_a_shallow_call_still_succeeds() {
    on_ordinary_stack(|| {
        let ctx = Context::new();
        let mut term = universe(0);
        // Derive the boundary from the public admission predicate. Do not copy
        // the kernel's private depth ceiling into this regression.
        while within_kernel_depth(&term) {
            term = lambda("depth.boundary".into(), universe(0), term);
        }
        let Term::Lambda { body, .. } = &term else {
            panic!("constructed lambda chain")
        };
        assert!(
            within_kernel_depth(body),
            "the previous depth remains admitted"
        );
        let refused = lean_type(&ctx, &term);
        dispose_lambda_chain(term);
        assert!(
            matches!(refused, Err(Verdict::Decline(_))),
            "excess input depth is an honest refusal before recursive cached typing"
        );
        check_both(&ctx, &universe(0), &universe(1));
    });
}
