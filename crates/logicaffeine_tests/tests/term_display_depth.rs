//! Diagnostics must remain printable after computation produces a deep term.
use logicaffeine_kernel::{Literal, Term, Universe};
use std::fmt;

fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn app(f: Term, x: Term) -> Term {
    Term::App(Box::new(f), Box::new(x))
}
fn pi(n: &str, a: Term, b: Term) -> Term {
    Term::Pi {
        param: n.into(),
        param_type: Box::new(a),
        body_type: Box::new(b),
    }
}
fn dispose_universe(u: Universe) {
    let mut pending = vec![u];
    while let Some(u) = pending.pop() {
        match u {
            Universe::Succ(a) => pending.push(*a),
            Universe::Max(a, b) | Universe::IMax(a, b) => {
                pending.push(*a);
                pending.push(*b);
            }
            _ => {}
        }
    }
}
// Fixture disposal is iterative so these controls isolate Display, not AST Drop.
fn dispose(t: Term) {
    let mut pending = vec![t];
    while let Some(t) = pending.pop() {
        match t {
            Term::Sort(u) => dispose_universe(u),
            Term::Const { levels, .. } => {
                for u in levels {
                    dispose_universe(u)
                }
            }
            Term::Pi {
                param_type,
                body_type,
                ..
            } => {
                pending.push(*param_type);
                pending.push(*body_type);
            }
            Term::Lambda {
                param_type, body, ..
            } => {
                pending.push(*param_type);
                pending.push(*body);
            }
            Term::App(f, a) => {
                pending.push(*f);
                pending.push(*a);
            }
            Term::Annotated { term, .. } => pending.push(*term),
            Term::Match {
                discriminant,
                motive,
                cases,
            } => {
                pending.push(*discriminant);
                pending.push(*motive);
                pending.extend(cases);
            }
            Term::Fix { body, .. } => pending.push(*body),
            Term::MutualFix { defs, .. } => pending.extend(defs.into_iter().map(|(_, b)| b)),
            Term::Let {
                ty, value, body, ..
            } => {
                pending.push(*ty);
                pending.push(*value);
                pending.push(*body);
            }
            Term::Var(_) | Term::Global(_) | Term::Lit(_) | Term::Hole => {}
        }
    }
}
fn app_chain(depth: usize) -> Term {
    (0..depth).fold(g("z"), |t, _| app(g("f"), t))
}

#[test]
fn every_term_form_keeps_its_existing_spelling_and_order() {
    let fixtures = vec![
        (Term::Sort(Universe::Prop), "Prop"),
        (Term::Var("x".into()), "x"),
        (g("Nat"), "Nat"),
        (
            Term::Const {
                name: "C".into(),
                levels: vec![],
            },
            "C.{}",
        ),
        (
            Term::Const {
                name: "C".into(),
                levels: vec![Universe::Prop, Universe::Type(2)],
            },
            "C.{Prop, Type2}",
        ),
        (pi("_", g("A"), g("B")), "A -> B"),
        (pi("x", g("A"), Term::Var("x".into())), "Π(x:A). x"),
        (
            Term::Lambda {
                param: "x".into(),
                param_type: Box::new(g("A")),
                body: Box::new(Term::Var("x".into())),
            },
            "λ(x:A). x",
        ),
        (app(g("f"), g("x")), "(f x)"),
        (app(g("f"), pi("_", g("A"), g("B"))), "(f (A -> B))"),
        (
            Term::Match {
                discriminant: Box::new(g("d")),
                motive: Box::new(g("m")),
                cases: vec![],
            },
            "match d return m with ",
        ),
        (
            Term::Match {
                discriminant: Box::new(g("d")),
                motive: Box::new(g("m")),
                cases: vec![g("a"), g("b"), g("c")],
            },
            "match d return m with a | b | c",
        ),
        (
            Term::Fix {
                name: "f".into(),
                body: Box::new(g("body")),
            },
            "fix f. body",
        ),
        (
            Term::MutualFix {
                defs: vec![],
                index: 0,
            },
            "mutualfix {  }.0",
        ),
        (
            Term::MutualFix {
                defs: vec![("f".into(), g("a")), ("g".into(), g("b"))],
                index: 1,
            },
            "mutualfix { f := a, g := b }.1",
        ),
        (
            Term::Let {
                name: "x".into(),
                ty: Box::new(g("A")),
                value: Box::new(g("a")),
                body: Box::new(Term::Var("x".into())),
            },
            "let x:A := a in x",
        ),
        (Term::Lit(Literal::Int(-7)), "-7"),
        (Term::Hole, "_"),
    ];
    for (term, expected) in fixtures {
        assert_eq!(term.to_string(), expected);
    }
    let universe = Universe::IMax(
        Box::new(Universe::Max(
            Box::new(Universe::SProp),
            Box::new(Universe::Type(3)),
        )),
        Box::new(Universe::Succ(Box::new(Universe::Var("u".into())))),
    );
    assert_eq!(universe.to_string(), "imax(max(SProp, Type3), (u+1))");
}

#[test]
fn deeply_nested_match_diagnostics_do_not_recurse_on_the_native_stack() {
    let depth = 8_000;
    let t = (0..depth).fold(g("z"), |body, _| Term::Match {
        discriminant: Box::new(g("d")),
        motive: Box::new(g("m")),
        cases: vec![body],
    });
    let displayed = t.to_string();
    dispose(t);
    assert_eq!(displayed, "match d return m with ".repeat(depth) + "z");
}

#[test]
fn deeply_nested_application_diagnostics_preserve_every_parenthesis() {
    let depth = 12_000;
    let t = app_chain(depth);
    let displayed = t.to_string();
    dispose(t);
    assert_eq!(displayed, "(f ".repeat(depth) + "z" + &")".repeat(depth));
}

#[test]
fn deeply_nested_universe_diagnostics_preserve_every_level() {
    let depth = 16_000;
    let u = (0..depth).fold(Universe::Var("u".into()), |u, _| {
        Universe::Succ(Box::new(u))
    });
    let displayed = u.to_string();
    dispose_universe(u);
    assert_eq!(displayed, "(".repeat(depth) + "u" + &"+1)".repeat(depth));
}

#[test]
fn constant_levels_use_the_same_nonrecursive_universe_formatting() {
    let depth = 8_000;
    let u = (0..depth).fold(Universe::Prop, |u, _| {
        Universe::Max(Box::new(Universe::Type(0)), Box::new(u))
    });
    let t = Term::Const {
        name: "Poly".into(),
        levels: vec![u, Universe::SProp],
    };
    let displayed = t.to_string();
    dispose(t);
    assert_eq!(
        displayed,
        "Poly.{".to_string()
            + &"max(Type0, ".repeat(depth)
            + "Prop"
            + &")".repeat(depth)
            + ", SProp}"
    );
}

#[test]
fn a_failing_output_writer_stops_without_visiting_the_deep_suffix() {
    struct Limited {
        bytes: usize,
        text: String,
    }
    impl fmt::Write for Limited {
        fn write_str(&mut self, s: &str) -> fmt::Result {
            if s.len() > self.bytes {
                return Err(fmt::Error);
            }
            self.bytes -= s.len();
            self.text.push_str(s);
            Ok(())
        }
    }
    let t = app_chain(12_000);
    let mut writer = Limited {
        bytes: 8,
        text: String::new(),
    };
    let result = fmt::write(&mut writer, format_args!("{}", t));
    dispose(t);
    assert!(result.is_err());
    assert!(writer.text.len() <= 8);
    assert!("(f (f (f ".starts_with(&writer.text));
}
