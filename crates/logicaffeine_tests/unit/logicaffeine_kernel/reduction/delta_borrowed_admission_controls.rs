use super::*;
use crate::Universe;
use std::cell::RefCell;

fn sort(n: u32) -> Term {
    Term::Sort(Universe::Type(n))
}
fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn context() -> Context {
    let mut ctx = Context::new();
    ctx.with_equations_suspended(|ctx| ctx.declare("small", sort(1), sort(0)))
        .unwrap();
    ctx
}

thread_local! {
    static ADMISSIONS: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) };
}
fn observed_admit(t: &Term) -> Option<usize> {
    ADMISSIONS.with(|xs| xs.borrow_mut().push(t as *const Term as usize));
    bounded_conversion_size(t)
}
fn observed(ctx: &Context, t: &Term, tr: Transparency<'_>) -> (Option<(usize, Term)>, Vec<usize>) {
    ADMISSIONS.with(|xs| xs.borrow_mut().clear());
    let result = bounded_delta_head_with(ctx, t, tr, observed_admit);
    let visits = ADMISSIONS.with(|xs| std::mem::take(&mut *xs.borrow_mut()));
    (result, visits)
}

#[test]
fn monomorphic_body_admission_is_borrowed_and_not_repeated_on_its_clone() {
    let ctx = context();
    let term = g("small");
    let body = ctx.get_definition_body("small").unwrap();
    let (result, visits) = observed(&ctx, &term, Transparency::All);
    assert_eq!(result, Some((1, sort(0))));
    assert_eq!(
        visits,
        vec![&term as *const Term as usize, body as *const Term as usize],
        "the immutable admitted body must not be rescanned after cloning"
    );
}

#[test]
fn open_body_is_rejected_from_the_borrowed_body_after_one_admission() {
    let mut ctx = Context::new().extend("outside", sort(1));
    ctx.with_equations_suspended(|ctx| ctx.declare("open", sort(1), Term::Var("outside".into())))
        .unwrap();
    let term = g("open");
    let body = ctx.get_definition_body("open").unwrap();
    let (result, visits) = observed(&ctx, &term, Transparency::All);
    assert_eq!(result, None);
    assert_eq!(
        visits,
        vec![&term as *const Term as usize, body as *const Term as usize]
    );
}

#[test]
fn transparency_and_whole_input_admission_precede_body_lookup() {
    let ctx = context();
    let term = g("small");
    let hide = |_: &str| true;
    let (result, visits) = observed(&ctx, &term, Transparency::Excluding(&hide));
    assert_eq!(result, None);
    assert_eq!(visits, vec![&term as *const Term as usize]);
    let oversized = Term::Match {
        discriminant: Box::new(g("small")),
        motive: Box::new(sort(0)),
        cases: vec![sort(0); 4094],
    };
    let (result, visits) = observed(&ctx, &oversized, Transparency::All);
    assert_eq!(result, None);
    assert_eq!(visits, vec![&oversized as *const Term as usize]);
}

#[test]
fn original_delta_results_match_frozen_oracle_at_size_and_universe_boundaries() {
    let mut ctx = context();
    ctx.declare_universe_poly(
        "poly",
        vec!["u".into()],
        Term::Sort(Universe::Succ(Box::new(Universe::Var("u".into())))),
        Term::Sort(Universe::Var("u".into())),
    )
    .unwrap();
    let terms = vec![
        g("small"),
        g("missing"),
        Term::App(Box::new(g("small")), Box::new(sort(0))),
        Term::Match {
            discriminant: Box::new(g("small")),
            motive: Box::new(sort(0)),
            cases: vec![sort(0); 4093],
        }, // exactly 4096 Term nodes
        Term::Match {
            discriminant: Box::new(g("small")),
            motive: Box::new(sort(0)),
            cases: vec![sort(0); 4094],
        }, // 4097 Term nodes: refuse
        Term::Const {
            name: "poly".into(),
            levels: vec![Universe::Type(0)],
        },
        Term::Const {
            name: "poly".into(),
            levels: vec![Universe::Type(1)],
        },
        Term::Const {
            name: "poly".into(),
            levels: vec![],
        },
        Term::Const {
            name: "poly".into(),
            levels: vec![Universe::Type(0); 2],
        },
    ];
    let hide = |n: &str| n == "small";
    for term in &terms {
        for tr in [Transparency::All, Transparency::Excluding(&hide)] {
            assert_eq!(
                bounded_delta_head_impl(&ctx, term, tr, bounded_conversion_size, false),
                previous_delta(&ctx, term, tr, bounded_conversion_size, false)
            );
            assert_eq!(
                bounded_delta_head_with(&ctx, term, tr, bounded_conversion_size),
                previous_delta(&ctx, term, tr, bounded_conversion_size, true)
            );
        }
    }
    assert!(bounded_delta_head(&ctx, &terms[3]).is_some());
    assert!(bounded_delta_head(&ctx, &terms[4]).is_none());
    assert_eq!(bounded_delta_head(&ctx, &terms[5]), Some((1, sort(0))));
    assert_eq!(bounded_delta_head(&ctx, &terms[6]), Some((1, sort(1))));
}

// The exact parent implementation is appended by prepare.py as a frozen oracle.

fn previous_delta(
    ctx: &Context,
    term: &Term,
    tr: Transparency<'_>,
    admit: fn(&Term) -> Option<usize>,
    bounded_telescope: bool,
) -> Option<(usize, Term)> {
    admit(term)?;
    match term {
        Term::Global(name) | Term::Const { name, .. } => {
            if !tr.unfolds(name) {
                return None;
            }
            let body = match term {
                Term::Global(_) => {
                    let body = ctx.get_definition_body(name)?;
                    admit(body)?;
                    body.clone()
                }
                Term::Const { levels, .. } => {
                    // const_step clones its header and body before resolving
                    // levels. Reject oversized candidates while still borrowed,
                    // using that resolver's exact lookup precedence.
                    match ctx.get_universe_poly(name) {
                        Some((params, ty, body)) => {
                            // const_step also copies explicit parameter names before
                            // resolving arity. Bound this borrowed metadata on the
                            // elaborator route, including unused parameters. The
                            // original kernel All wrapper retains its prior policy.
                            if bounded_telescope
                                && (params.len() > 16 || params.iter().any(|name| name.len() > 256))
                            {
                                return None;
                            }
                            admit(ty)?;
                            admit(body)?;
                        }
                        None => {
                            admit(ctx.get_global(name)?)?;
                            if let Some(body) = ctx.get_definition_body(name) {
                                admit(body)?;
                            }
                        }
                    }
                    match const_step(ctx, name, levels) {
                        ConstStep::Delta(subst, body) => {
                            crate::term::instantiate_universes(&body, &subst)
                        }
                        _ => return None,
                    }
                }
                _ => unreachable!(),
            };
            let size = admit(&body)?;
            if !crate::term_store::is_closed_without_holes(&body) {
                return None;
            }
            Some((size, body))
        }
        Term::App(f, a) => {
            let (size, f) = previous_delta(ctx, f, tr, admit, bounded_telescope)?;
            Some((size, Term::App(Box::new(f), a.clone())))
        }
        Term::Match {
            discriminant,
            motive,
            cases,
        } => {
            let (size, disc) = previous_delta(ctx, discriminant, tr, admit, bounded_telescope)?;
            Some((
                size,
                Term::Match {
                    discriminant: Box::new(disc),
                    motive: motive.clone(),
                    cases: cases.clone(),
                },
            ))
        }
        _ => None,
    }
}
