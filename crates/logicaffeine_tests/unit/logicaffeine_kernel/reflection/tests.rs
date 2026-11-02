//! The defined reflection operations: checked definitions of the standard library, attested only
//! as registered, and the native accelerator computing exactly what the definition computes.
use super::*;
use crate::prelude::{axiom_dependencies_of, StandardLibrary};
use crate::term::Literal;

fn g(name: &str) -> Term {
    Term::Global(name.to_string())
}

fn app(f: Term, args: impl IntoIterator<Item = Term>) -> Term {
    args.into_iter()
        .fold(f, |f, a| Term::App(Box::new(f), Box::new(a)))
}

fn int(n: i64) -> Term {
    Term::Lit(Literal::Int(n))
}

#[test]
fn every_defined_operation_is_a_checked_definition_resting_on_no_reflection_assumption() {
    let ctx = StandardLibrary::context();
    assert!(
        ctx.refused_declarations().is_empty(),
        "{:?}",
        ctx.refused_declarations()
    );
    for name in DEFINED {
        assert!(
            ctx.get_definition_body(name).is_some(),
            "`{name}` is a definition"
        );
        let usage = axiom_dependencies_of(ctx, name).expect("registered");
        let reflective: Vec<&String> = usage
            .axioms
            .iter()
            .filter(|a| a.starts_with("syn_") || a.starts_with("try_") || *a == "concludes")
            .collect();
        assert!(reflective.is_empty(), "`{name}` rests on {reflective:?}");
    }
}

#[test]
fn the_standard_library_attests_every_defined_operation() {
    let ctx = StandardLibrary::context();
    for name in DEFINED {
        assert!(attested(ctx, name), "`{name}`");
    }
    assert!(
        !attested(ctx, "syn_step"),
        "an assumption is not a defined operation"
    );
}

#[test]
fn a_changed_definition_or_embedding_is_not_attested() {
    let changed_body = {
        let mut ctx = StandardLibrary::context().clone();
        ctx.permit_meaning_changes_for_cache_tests();
        ctx.add_definition(
            "syn_size".to_string(),
            crate::interface::TermParser::parse("Syntax -> Int").unwrap(),
            crate::interface::TermParser::parse("fun (t : Syntax) => 0").unwrap(),
        );
        ctx
    };
    assert!(!attested(&changed_body, "syn_size"));
    assert!(
        attested(&changed_body, "syn_lift"),
        "another operation's definition is untouched"
    );
    assert_eq!(
        crate::normalize(
            &changed_body,
            &app(g("syn_size"), [app(g("SVar"), [int(0)])])
        ),
        int(0),
        "a changed definition computes by its body, never by the native rule"
    );
    let changed_arithmetic = {
        let mut ctx = StandardLibrary::context().clone();
        ctx.permit_meaning_changes_for_cache_tests();
        ctx.add_definition(
            "add".to_string(),
            crate::interface::TermParser::parse("Int -> Int -> Int").unwrap(),
            crate::interface::TermParser::parse("fun (a : Int) (b : Int) => a").unwrap(),
        );
        ctx
    };
    for name in DEFINED {
        assert!(
            !attested(&changed_arithmetic, name),
            "`{name}` over a changed `add`"
        );
    }
    let changed_embedding = {
        let mut ctx = StandardLibrary::context().clone();
        ctx.permit_meaning_changes_for_cache_tests();
        ctx.add_constructor("SHole", "Syntax", g("Syntax"));
        ctx
    };
    for name in DEFINED {
        assert!(
            !attested(&changed_embedding, name),
            "`{name}` over a `Syntax` with another constructor"
        );
    }
}

/// A deterministic corpus of closed `Syntax` terms: every constructor, small and negative indices,
/// nesting under binders.
fn corpus() -> Vec<Term> {
    let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut next = move |bound: u64| {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (state >> 33) % bound
    };
    fn build(depth: u32, next: &mut dyn FnMut(u64) -> u64) -> Term {
        let leaf = |next: &mut dyn FnMut(u64) -> u64| {
            let n = next(7) as i64 - 2;
            match next(5) {
                0 => app(g("SVar"), [int(n)]),
                1 => app(g("SGlobal"), [int(n)]),
                2 => app(
                    g("SSort"),
                    [if next(2) == 0 {
                        g("UProp")
                    } else {
                        app(g("UType"), [int(n)])
                    }],
                ),
                3 => app(g("SLit"), [int(n)]),
                _ => app(g("SName"), [Term::Lit(Literal::Text(format!("c{n}")))]),
            }
        };
        if depth == 0 || next(4) == 0 {
            return leaf(next);
        }
        let ctor = ["SApp", "SLam", "SPi"][next(3) as usize];
        let (a, b) = (build(depth - 1, next), build(depth - 1, next));
        app(g(ctor), [a, b])
    }
    (0..60).map(|_| build(4, &mut next)).collect()
}

/// `name`'s definition under a fresh name, every defined operation it names renamed with it: the
/// same body, never attested, so δ computes it.
fn reference(ctx: &mut Context) {
    fn rename(t: &Term) -> Term {
        match t {
            Term::Global(n) if DEFINED.contains(&n.as_str()) => {
                Term::Global(format!("{n}_reference"))
            }
            Term::App(f, a) => Term::App(Box::new(rename(f)), Box::new(rename(a))),
            Term::Lambda {
                param,
                param_type,
                body,
            } => Term::Lambda {
                param: param.clone(),
                param_type: Box::new(rename(param_type)),
                body: Box::new(rename(body)),
            },
            Term::Pi {
                param,
                param_type,
                body_type,
            } => Term::Pi {
                param: param.clone(),
                param_type: Box::new(rename(param_type)),
                body_type: Box::new(rename(body_type)),
            },
            Term::Match {
                discriminant,
                motive,
                cases,
            } => Term::Match {
                discriminant: Box::new(rename(discriminant)),
                motive: Box::new(rename(motive)),
                cases: cases.iter().map(rename).collect(),
            },
            Term::Fix { name, body } => Term::Fix {
                name: name.clone(),
                body: Box::new(rename(body)),
            },
            other => other.clone(),
        }
    }
    for (name, ty, body) in definitions() {
        ctx.add_definition(format!("{name}_reference"), ty.clone(), rename(body));
    }
    assert!(
        ctx.refused_declarations().is_empty(),
        "{:?}",
        ctx.refused_declarations()
    );
}

#[test]
fn the_accelerator_computes_what_the_definition_computes() {
    let mut ctx = StandardLibrary::context().clone();
    reference(&mut ctx);
    let terms = corpus();
    let mut checked = 0;
    for (i, t) in terms.iter().enumerate() {
        let u = &terms[(i + 1) % terms.len()];
        let n = int((i % 5) as i64 - 1);
        let m = int((i % 3) as i64);
        let applications: Vec<(&str, Vec<Term>)> = vec![
            ("syn_size", vec![t.clone()]),
            ("syn_max_var", vec![t.clone()]),
            ("syn_lift", vec![n.clone(), m.clone(), t.clone()]),
            ("syn_subst", vec![u.clone(), m.clone(), t.clone()]),
            ("syn_beta", vec![t.clone(), u.clone()]),
            ("syn_quote", vec![t.clone()]),
            ("syn_diag", vec![t.clone()]),
        ];
        for (name, args) in applications {
            let fast = crate::normalize(&ctx, &app(g(name), args.clone()));
            let defined =
                crate::normalize(&ctx, &app(g(&format!("{name}_reference")), args.clone()));
            assert_eq!(fast, defined, "`{name}` on {args:?}");
            checked += 1;
        }
    }
    assert_eq!(checked, 7 * terms.len());
}

#[test]
fn the_independent_checker_computes_by_the_definition() {
    let ctx = StandardLibrary::context();
    for t in corpus().into_iter().take(12) {
        let size = crate::normalize(ctx, &app(g("syn_size"), [t.clone()]));
        let claim = app(
            g("Eq"),
            [g("Int"), app(g("syn_size"), [t.clone()]), size.clone()],
        );
        let proof = app(g("refl"), [g("Int"), size]);
        let probe = Term::App(
            Box::new(Term::Lambda {
                param: "p".to_string(),
                param_type: Box::new(claim),
                body: Box::new(Term::Var("p".to_string())),
            }),
            Box::new(proof),
        );
        assert!(
            crate::infer_type(ctx, &probe).is_ok(),
            "the main kernel computes `syn_size` of {t:?}"
        );
        assert_eq!(
            crate::double_check(ctx, &probe),
            crate::DoubleCheck::Agreed,
            "{t:?}"
        );
    }
}
