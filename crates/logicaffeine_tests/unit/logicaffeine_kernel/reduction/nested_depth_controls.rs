//! A normalization nested inside a kernel call that was sized for a shallow term may be handed a
//! deep one — a reduct an earlier normalization of the same conversion built. It must grow its own
//! stack and answer, never abort: `lean_scale_nat_compare::a_comparison_whose_recursion_passes_
//! through_an_axiom_is_not_accelerated` aborted with a stack overflow inside
//! `def_eq_uncached → normalize → normalize_tree_with_spent → reduce_step_opt` before this.
use super::*;

fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn v(n: &str) -> Term {
    Term::Var(n.into())
}
fn app(f: Term, a: Term) -> Term {
    Term::App(Box::new(f), Box::new(a))
}
fn lam(n: &str, t: Term, b: Term) -> Term {
    Term::Lambda {
        param: n.into(),
        param_type: Box::new(t),
        body: Box::new(b),
    }
}

const DEPTH: usize = 1500;

fn wrapped(core: Term) -> Term {
    (0..DEPTH).fold(core, |t, _| app(g("wrap"), t))
}

#[test]
fn a_deep_reduct_normalized_inside_a_shallow_kernel_call_is_answered_not_aborted() {
    let mut c = Context::new();
    crate::prelude::StandardLibrary::register(&mut c);
    c.add_declaration(
        "wrap",
        Term::Pi {
            param: "n".into(),
            param_type: Box::new(g("Nat")),
            body_type: Box::new(g("Nat")),
        },
    );
    // A β-redex at the bottom of a 1500-deep spine: the first reduction step descends the whole way.
    let deep = wrapped(app(lam("y", g("Nat"), v("y")), g("Zero")));
    let answer = crate::term_store::deepen_stack(&g("Zero"), || normalize_checked(&c, &deep))
        .expect("the outermost kernel call");
    let normal = answer.unwrap_or_else(|(_, why)| panic!("the reduct normalizes: {why:?}"));
    let expected = wrapped(g("Zero"));
    std::thread::Builder::new()
        .stack_size(1 << 28)
        .spawn(move || {
            assert!(normal == expected, "the normal form is `wrap^{DEPTH} Zero`");
            drop((normal, expected));
        })
        .unwrap()
        .join()
        .unwrap();
}

/// A normalization nested inside a kernel call may be handed a reduct one level past the door —
/// the partial term an earlier normalization stopped at. The door admits every term once a kernel
/// call is under way, so the normalization refuses it itself: a `TooDeep` stop, never an abort.
#[test]
fn a_reduct_past_the_door_normalized_inside_a_kernel_call_is_refused_not_aborted() {
    let mut c = Context::new();
    crate::prelude::StandardLibrary::register(&mut c);
    c.add_declaration(
        "wrap",
        Term::Pi {
            param: "n".into(),
            param_type: Box::new(g("Nat")),
            body_type: Box::new(g("Nat")),
        },
    );
    let past = (0..=crate::type_checker::MAX_TERM_DEPTH)
        .fold(app(lam("y", g("Nat"), v("y")), g("Zero")), |t, _| {
            app(g("wrap"), t)
        });
    assert!(crate::term_store::term_depth(&past) > crate::type_checker::MAX_TERM_DEPTH);
    let answer = crate::term_store::deepen_stack(&g("Zero"), || {
        let _door = crate::type_checker::enter_kernel(&g("Zero"), "a shallow kernel call").unwrap();
        crate::reduction::observe_resource_stops(|| {
            normalize_checked(&c, &past)
                .map(drop)
                .map_err(|(partial, why)| {
                    std::thread::Builder::new()
                        .stack_size(1 << 28)
                        .spawn(move || drop(partial))
                        .unwrap()
                        .join()
                        .unwrap();
                    why
                })
        })
    })
    .expect("the outermost kernel call");
    assert_eq!(answer, (Err(Unreduced::TooDeep), Some(Unreduced::TooDeep)));
    std::thread::Builder::new()
        .stack_size(1 << 28)
        .spawn(move || drop(past))
        .unwrap()
        .join()
        .unwrap();
}
