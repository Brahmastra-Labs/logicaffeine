//! ★ THE PRINTER IS THE PARSER'S INVERSE — `print ∘ parse ≡ id`, up to α.
//!
//! `interface::printer::{print_term, print_command}` spell a kernel term or a vernacular command in
//! LOGOS's own surface. This file locks the one property that makes that worth having: reading the
//! printed text back — `TermParser` then the namespace resolver, which is how the `Repl` reads a
//! name — yields the SAME term, binder names aside. A term the surface cannot spell is a
//! `PrintError::NoSpelling`, never an approximation.
//!
//! Two corpora:
//!
//! * a GENERATED one — seeded, deterministic, every term shape the kernel has, with binder capture,
//!   shadowing, dependent binders, universe variables, prelude-name collisions and every `Literal`
//!   variant at its edges;
//! * every term of every declaration of every original Lean bridge fixture under
//!   `tests/fixtures/lean_conformance/cases_lean414/`, imported through `interface::lean`.
//!
//! The α-equality below is written HERE, independently of the kernel's own conversion, so the lock
//! cannot be satisfied by a lenient comparison.

use logicaffeine_kernel::interface::printer::{print_command, print_term, PrintError};
use logicaffeine_kernel::interface::surface::{resolve_names, Namespaces};
use logicaffeine_kernel::interface::{parse_command, Command, TermParser};
use logicaffeine_kernel::{BigInt, Literal, Term, Universe};
use std::collections::BTreeSet;

// =================================================================================================
// Builders
// =================================================================================================

fn g(n: &str) -> Term {
    Term::Global(n.to_string())
}
fn v(n: &str) -> Term {
    Term::Var(n.to_string())
}
fn app(f: Term, a: Term) -> Term {
    Term::App(Box::new(f), Box::new(a))
}
fn pi(p: &str, a: Term, b: Term) -> Term {
    Term::Pi {
        param: p.to_string(),
        param_type: Box::new(a),
        body_type: Box::new(b),
    }
}
fn lam(p: &str, a: Term, b: Term) -> Term {
    Term::Lambda {
        param: p.to_string(),
        param_type: Box::new(a),
        body: Box::new(b),
    }
}
fn sort(u: Universe) -> Term {
    Term::Sort(u)
}
fn uvar(n: &str) -> Universe {
    Universe::Var(n.to_string())
}
fn succ(u: Universe) -> Universe {
    Universe::Succ(Box::new(u))
}
fn umax(a: Universe, b: Universe) -> Universe {
    Universe::Max(Box::new(a), Box::new(b))
}
fn uimax(a: Universe, b: Universe) -> Universe {
    Universe::IMax(Box::new(a), Box::new(b))
}
fn konst(n: &str, levels: Vec<Universe>) -> Term {
    Term::Const {
        name: n.to_string(),
        levels,
    }
}
fn lit(l: Literal) -> Term {
    Term::Lit(l)
}
fn big(s: &str) -> BigInt {
    BigInt::parse_decimal(s).expect("a decimal")
}
fn let_(n: &str, ty: Term, value: Term, body: Term) -> Term {
    Term::Let {
        name: n.to_string(),
        ty: Box::new(ty),
        value: Box::new(value),
        body: Box::new(body),
    }
}
fn matchp(d: Term, m: Term, cases: Vec<Term>) -> Term {
    Term::Match {
        discriminant: Box::new(d),
        motive: Box::new(m),
        cases,
    }
}
fn fix(n: &str, b: Term) -> Term {
    Term::Fix {
        name: n.to_string(),
        body: Box::new(b),
    }
}
fn mfix(defs: Vec<(&str, Term)>, index: usize) -> Term {
    Term::MutualFix {
        defs: defs.into_iter().map(|(n, b)| (n.to_string(), b)).collect(),
        index,
    }
}

// =================================================================================================
// An α-equality written here, not borrowed from the kernel
// =================================================================================================

/// α-equality of NAMED terms: two terms are equal when they differ only in the names of their
/// binders. A variable is compared by WHICH binder binds it (the innermost binder of that name on
/// its own side), never by its spelling.
///
/// Each side keeps, per name, the stack of binders currently binding it — so a lookup is one hash
/// probe however deep the term, and a fixture term under thousands of binders compares in linear
/// time.
fn alpha_eq(a: &Term, b: &Term) -> bool {
    alpha(a, b, &mut Scopes::default())
}

#[derive(Default)]
struct Scopes {
    left: std::collections::HashMap<String, Vec<usize>>,
    right: std::collections::HashMap<String, Vec<usize>>,
    next: usize,
}

impl Scopes {
    fn push(&mut self, l: &str, r: &str) {
        let id = self.next;
        self.next += 1;
        self.left.entry(l.to_string()).or_default().push(id);
        self.right.entry(r.to_string()).or_default().push(id);
    }
    fn pop(&mut self, l: &str, r: &str) {
        self.left.get_mut(l).and_then(Vec::pop);
        self.right.get_mut(r).and_then(Vec::pop);
    }
}

fn alpha(a: &Term, b: &Term, env: &mut Scopes) -> bool {
    match (a, b) {
        (Term::Var(x), Term::Var(y)) => {
            let left = env.left.get(x).and_then(|s| s.last());
            let right = env.right.get(y).and_then(|s| s.last());
            match (left, right) {
                (Some(i), Some(j)) => i == j,
                (None, None) => x == y,
                _ => false,
            }
        }
        (Term::Global(x), Term::Global(y)) => x == y,
        (
            Term::Const {
                name: x,
                levels: lx,
            },
            Term::Const {
                name: y,
                levels: ly,
            },
        ) => x == y && lx == ly,
        (Term::Sort(u), Term::Sort(w)) => u == w,
        (Term::Lit(l), Term::Lit(m)) => l == m,
        (Term::Hole, Term::Hole) => true,
        (
            Term::Pi {
                param: p,
                param_type: t,
                body_type: body,
            },
            Term::Pi {
                param: q,
                param_type: s,
                body_type: other,
            },
        )
        | (
            Term::Lambda {
                param: p,
                param_type: t,
                body,
            },
            Term::Lambda {
                param: q,
                param_type: s,
                body: other,
            },
        ) => {
            if !alpha(t, s, env) {
                return false;
            }
            env.push(p, q);
            let r = alpha(body, other, env);
            env.pop(p, q);
            r
        }
        (Term::App(f, x), Term::App(h, y)) => alpha(f, h, env) && alpha(x, y, env),
        (
            Term::Match {
                discriminant: d,
                motive: m,
                cases: cs,
            },
            Term::Match {
                discriminant: e,
                motive: n,
                cases: ds,
            },
        ) => {
            alpha(d, e, env)
                && alpha(m, n, env)
                && cs.len() == ds.len()
                && cs.iter().zip(ds).all(|(c, d)| alpha(c, d, env))
        }
        (
            Term::Fix { name: n, body },
            Term::Fix {
                name: m,
                body: other,
            },
        ) => {
            env.push(n, m);
            let r = alpha(body, other, env);
            env.pop(n, m);
            r
        }
        (Term::MutualFix { defs: xs, index: i }, Term::MutualFix { defs: ys, index: j }) => {
            if i != j || xs.len() != ys.len() {
                return false;
            }
            for ((n, _), (m, _)) in xs.iter().zip(ys) {
                env.push(n, m);
            }
            let r = xs.iter().zip(ys).all(|((_, x), (_, y))| alpha(x, y, env));
            for ((n, _), (m, _)) in xs.iter().zip(ys).rev() {
                env.pop(n, m);
            }
            r
        }
        (
            Term::Let {
                name: n,
                ty: t,
                value: x,
                body,
            },
            Term::Let {
                name: m,
                ty: s,
                value: y,
                body: other,
            },
        ) => {
            if !alpha(t, s, env) || !alpha(x, y, env) {
                return false;
            }
            env.push(n, m);
            let r = alpha(body, other, env);
            env.pop(n, m);
            r
        }
        _ => false,
    }
}

#[test]
fn the_alpha_equality_this_file_trusts_is_itself_correct() {
    // Renaming a binder consistently is equal…
    assert!(alpha_eq(
        &lam("x", g("A"), v("x")),
        &lam("y", g("A"), v("y"))
    ));
    // …capturing a different binder is not…
    assert!(!alpha_eq(
        &lam("x", g("A"), lam("y", g("A"), v("x"))),
        &lam("x", g("A"), lam("y", g("A"), v("y")))
    ));
    // …a variable is not a global of the same spelling…
    assert!(!alpha_eq(
        &lam("x", g("A"), v("x")),
        &lam("x", g("A"), g("x"))
    ));
    // …shadowing binds the innermost…
    assert!(alpha_eq(
        &lam("x", g("A"), lam("x", g("B"), v("x"))),
        &lam("a", g("A"), lam("b", g("B"), v("b")))
    ));
    assert!(!alpha_eq(
        &lam("x", g("A"), lam("x", g("B"), v("x"))),
        &lam("a", g("A"), lam("b", g("B"), v("a")))
    ));
    // …and a universe level is compared STRUCTURALLY, which is the strict reading.
    assert!(!alpha_eq(
        &sort(Universe::Type(0)),
        &sort(succ(Universe::Prop))
    ));
    assert!(alpha_eq(
        &mfix(vec![("f", v("g")), ("g", v("f"))], 1),
        &mfix(vec![("a", v("b")), ("b", v("a"))], 1)
    ));
    assert!(!alpha_eq(
        &mfix(vec![("f", v("g")), ("g", v("f"))], 1),
        &mfix(vec![("a", v("a")), ("b", v("a"))], 1)
    ));
}

// =================================================================================================
// Reading the printed text back — the Repl's own two stages
// =================================================================================================

/// Every global key and constant key the term mentions — the environment a printed term is read
/// back in. A printed reference is only obliged to mean its constant where that constant exists.
fn globals_of(t: &Term, out: &mut BTreeSet<String>) {
    let mut work = vec![t];
    while let Some(t) = work.pop() {
        match t {
            Term::Global(n) | Term::Const { name: n, .. } => {
                out.insert(n.clone());
            }
            Term::Pi {
                param_type,
                body_type,
                ..
            } => {
                work.push(param_type);
                work.push(body_type);
            }
            Term::Lambda {
                param_type, body, ..
            } => {
                work.push(param_type);
                work.push(body);
            }
            Term::App(f, a) => {
                work.push(f);
                work.push(a);
            }
            Term::Match {
                discriminant,
                motive,
                cases,
            } => {
                work.push(discriminant);
                work.push(motive);
                work.extend(cases.iter());
            }
            Term::Fix { body, .. } => work.push(body),
            Term::MutualFix { defs, .. } => work.extend(defs.iter().map(|(_, b)| b)),
            Term::Let {
                ty, value, body, ..
            } => {
                work.push(ty);
                work.push(value);
                work.push(body);
            }
            Term::Annotated { term, .. } => work.push(term),
            Term::Var(_) | Term::Sort(_) | Term::Lit(_) | Term::Hole => {}
        }
    }
}

/// Read `text` exactly as the `Repl` reads a term: the term door, then name resolution at the root
/// scope against `known`.
fn read_back(text: &str, known: &BTreeSet<String>) -> Result<Term, String> {
    let parsed = TermParser::parse(text).map_err(|e| format!("{e:?}"))?;
    let known_fn = |k: &str| known.contains(k);
    let resolved = resolve_names(&Namespaces::new(), &known_fn, &mut Vec::new(), &parsed)
        .map_err(|e| format!("{e:?}"))?;
    Ok(resolved.into_owned())
}

/// The whole lock for one term: it prints, the text reads back α-equal, the print is
/// deterministic, and printing what was read back reproduces the text exactly.
fn assert_round_trips(t: &Term) {
    let printed = print_term(t).unwrap_or_else(|e| panic!("no spelling for {t:?}: {e}"));
    let again = print_term(t).expect("a second print of the same term");
    assert_eq!(printed, again, "printing is deterministic");
    let mut known = BTreeSet::new();
    globals_of(t, &mut known);
    let read = read_back(&printed, &known).unwrap_or_else(|e| {
        panic!("printed text does not parse:\n  term: {t:?}\n  text: {printed}\n  error: {e}")
    });
    assert!(
        alpha_eq(t, &read),
        "print∘parse is not the identity:\n  term: {t:?}\n  text: {printed}\n  read: {read:?}"
    );
    let reprinted = print_term(&read).expect("the read-back term prints");
    assert_eq!(
        printed, reprinted,
        "printing is a fixpoint on its own output"
    );
}

fn assert_no_spelling(t: &Term, mentions: &str) {
    match print_term(t) {
        Err(PrintError::NoSpelling { what }) => assert!(
            what.contains(mentions),
            "the refusal must name what has no spelling ({mentions:?}): {what}"
        ),
        Ok(text) => panic!("{t:?} has no honest spelling, yet printed as {text:?}"),
    }
}

// =================================================================================================
// Hand-written controls — each is one hazard, stated
// =================================================================================================

#[test]
fn a_binder_that_would_capture_a_global_is_renamed() {
    // `λ(x : Nat). x` where the body's `x` is the GLOBAL `x`. Printing the binder as `x` would read
    // the body back as the bound variable — a well-formed term denoting a different function.
    let t = lam("x", g("Nat"), g("x"));
    assert_round_trips(&t);
    let text = print_term(&t).unwrap();
    let read = TermParser::parse(&text).unwrap();
    let Term::Lambda { param, body, .. } = &read else {
        panic!("a lambda: {read:?}")
    };
    assert_ne!(
        param, "x",
        "the capturing binder must have been renamed: {text}"
    );
    assert_eq!(
        **body,
        g("x"),
        "and the body still names the global: {text}"
    );
}

#[test]
fn a_binder_named_like_a_qualified_globals_first_component_is_renamed() {
    // `Nat.add` prints qualified; under a binder named `Nat` its head would read as the variable.
    let t = lam("Nat", sort(Universe::Type(0)), app(g("Nat.add"), v("Nat")));
    assert_round_trips(&t);
}

#[test]
fn shadowing_binds_the_innermost_binder() {
    assert_round_trips(&lam("x", g("A"), lam("x", g("B"), v("x"))));
    assert_round_trips(&lam(
        "x",
        g("A"),
        lam("y", g("B"), lam("x", v("y"), v("x"))),
    ));
    assert_round_trips(&pi(
        "x",
        g("A"),
        pi("x", app(g("P"), v("x")), app(g("Q"), v("x"))),
    ));
}

#[test]
fn a_dependent_binder_keeps_its_name_and_a_non_dependent_one_is_an_arrow() {
    let dep = pi(
        "x",
        g("A"),
        pi("y", app(g("P"), v("x")), app(app(g("Q"), v("x")), v("y"))),
    );
    assert_round_trips(&dep);
    let arrow = pi("_", g("A"), g("B"));
    assert_round_trips(&arrow);
    assert_eq!(print_term(&arrow).unwrap(), "A -> B");
    // A named but unused binder is still an arrow: the name is not part of the term.
    assert_round_trips(&pi("unused", g("A"), g("B")));
}

#[test]
fn prelude_names_as_binders_and_as_globals_both_survive() {
    // A binder may shadow a prelude name; a global of that name elsewhere must still reach it.
    assert_round_trips(&lam("Nat", sort(Universe::Type(0)), v("Nat")));
    assert_round_trips(&app(g("Eq"), lam("Eq", g("Nat"), v("Eq"))));
    assert_round_trips(&lam(
        "Zero",
        g("Nat"),
        app(app(g("Eq"), v("Zero")), g("Zero")),
    ));
}

#[test]
fn every_sort_and_every_level_shape_has_a_spelling() {
    for u in [
        Universe::Prop,
        Universe::SProp,
        Universe::Type(0),
        Universe::Type(1),
        Universe::Type(u32::MAX),
        uvar("u"),
        succ(uvar("u")),
        succ(succ(uvar("u"))),
        umax(uvar("u"), uvar("v")),
        uimax(uvar("u"), uvar("v")),
        umax(succ(uvar("u")), uimax(uvar("v"), Universe::Prop)),
        succ(umax(uvar("u"), Universe::Type(2))),
        succ(Universe::Prop),
        succ(Universe::Type(u32::MAX)),
        uvar("max"),
        uvar("imax"),
        uvar("Prop"),
        uvar("/s:\"u\""),
        uvar("u v"),
        uimax(umax(uvar("a"), uvar("b")), succ(succ(succ(uvar("c"))))),
    ] {
        assert_round_trips(&sort(u.clone()));
        if u != Universe::SProp {
            // `SProp` is a SORT, not a level: Lean's level grammar has no spelling for it and it
            // collapses out of every compound level, so a constant cannot be instantiated at it.
            assert_round_trips(&konst("f", vec![u.clone(), uvar("w")]));
        }
    }
    assert_eq!(print_term(&sort(Universe::Prop)).unwrap(), "Prop");
    assert_eq!(print_term(&sort(Universe::Type(0))).unwrap(), "Type");
    assert_eq!(print_term(&sort(uvar("u"))).unwrap(), "Sort u");
    assert_eq!(print_term(&sort(succ(uvar("u")))).unwrap(), "Sort (u+1)");
    assert_eq!(print_term(&konst("id", vec![uvar("u")])).unwrap(), "id.{u}");
    assert_eq!(print_term(&konst("id", vec![])).unwrap(), "id.{}");
}

#[test]
fn a_constant_with_levels_is_a_constant_even_when_qualified() {
    assert_round_trips(&konst("Nat.rec", vec![uvar("u")]));
    assert_round_trips(&konst("A.B.c", vec![Universe::Prop, Universe::Type(3)]));
    assert_round_trips(&konst(
        "/s:\"List\"/s:\"map\"",
        vec![uvar("/s:\"u\""), uvar("/s:\"v\"")],
    ));
    assert_round_trips(&app(konst("List.nil", vec![Universe::Type(0)]), g("Nat")));
}

#[test]
fn names_the_plain_grammar_cannot_spell_are_quoted() {
    for name in [
        "fun",
        "forall",
        "match",
        "with",
        "end",
        "in",
        "let",
        "fix",
        "return",
        "by",
        "Prop",
        "SProp",
        "Type",
        "Type3",
        "Sort",
        "a b",
        "/s:\"Nat\"/s:\"add\"",
        "h'",
        "α₁",
        "arena.structure.projection.P.0",
        "weird?!",
        "Outer.a b",
        "max",
    ] {
        assert_round_trips(&g(name));
        assert_round_trips(&app(g("f"), g(name)));
        assert_round_trips(&lam(name, g("A"), v(name)));
        assert_round_trips(&lam(name, g("A"), app(g(name), v(name))));
    }
    assert_eq!(
        print_term(&g("Nat.add")).unwrap(),
        "Nat.add",
        "a qualified name stays qualified"
    );
}

#[test]
fn every_literal_variant_has_an_exact_spelling() {
    let lits = vec![
        Literal::Int(0),
        Literal::Int(42),
        Literal::Int(-42),
        Literal::Int(i64::MIN),
        Literal::Int(i64::MAX),
        Literal::BigInt(big("9223372036854775808")),
        Literal::BigInt(big("-9223372036854775809")),
        Literal::BigInt(big("123456789012345678901234567890123456789")),
        Literal::Nat(big("0")),
        Literal::Nat(big("7")),
        Literal::Nat(big("340282366920938463463374607431768211456")),
        Literal::Float(0.0),
        Literal::Float(-0.0),
        Literal::Float(1.5),
        Literal::Float(-2.25),
        Literal::Float(0.1),
        Literal::Float(1e300),
        Literal::Float(5e-324),
        Literal::Float(f64::MAX),
        Literal::Float(f64::MIN_POSITIVE),
        Literal::Float(f64::INFINITY),
        Literal::Float(f64::NEG_INFINITY),
        Literal::Float(f64::NAN),
        Literal::Float(f64::from_bits(0x7ff8_0000_0000_0001)),
        Literal::Float(f64::from_bits(0xfff0_0000_dead_beef)),
        Literal::Text(String::new()),
        Literal::Text("plain".into()),
        Literal::Text("quote \" and backslash \\".into()),
        Literal::Text("line\nbreak\ttab\rcarriage\u{0}nul".into()),
        Literal::Text("«guillemets» ⟨angles⟩ a. b := | with".into()),
        Literal::Text("unicode é 中 🦀 \u{2028}".into()),
        Literal::Duration(0),
        Literal::Duration(1),
        Literal::Duration(-1),
        Literal::Duration(5_000_000_000),
        Literal::Duration(-7_200_000_000_000),
        Literal::Duration(1_500),
        Literal::Duration(i64::MIN),
        Literal::Duration(i64::MAX),
        Literal::Date(0),
        Literal::Date(-1),
        Literal::Date(20_353),
        Literal::Date(i32::MIN),
        Literal::Date(i32::MAX),
        Literal::Date(-719_528),
        Literal::Moment(0),
        Literal::Moment(-1),
        Literal::Moment(1),
        Literal::Moment(1_700_000_000_123_456_789),
        Literal::Moment(i64::MIN),
        Literal::Moment(i64::MAX),
    ];
    for l in lits {
        assert_round_trips(&lit(l.clone()));
        assert_round_trips(&app(g("f"), lit(l.clone())));
        assert_round_trips(&app(app(g("f"), lit(l.clone())), lit(l)));
    }
}

#[test]
fn a_nat_literal_is_not_an_int_literal() {
    let nat = print_term(&lit(Literal::Nat(big("5")))).unwrap();
    let int = print_term(&lit(Literal::Int(5))).unwrap();
    assert_ne!(
        nat, int,
        "two distinct kernel literals need two distinct spellings"
    );
}

#[test]
fn match_fix_mutual_fix_and_let_have_exact_spellings() {
    let nat = g("Nat");
    let motive = lam("n", nat.clone(), nat.clone());
    let m = matchp(
        v("n"),
        motive.clone(),
        vec![g("Zero"), lam("k", nat.clone(), app(g("Succ"), v("k")))],
    );
    assert_round_trips(&lam("n", nat.clone(), m.clone()));
    // A match inside a case, inside an argument, inside a motive.
    let nested = matchp(
        v("n"),
        motive.clone(),
        vec![m.clone(), lam("k", nat.clone(), app(g("f"), m.clone()))],
    );
    assert_round_trips(&lam("n", nat.clone(), nested));
    assert_round_trips(&lam("n", nat.clone(), matchp(v("n"), Term::Hole, vec![])));
    assert_round_trips(&lam(
        "n",
        nat.clone(),
        app(matchp(v("n"), motive.clone(), vec![]), v("n")),
    ));

    let f = fix("f", lam("n", nat.clone(), app(v("f"), v("n"))));
    assert_round_trips(&f);
    assert_round_trips(&app(f.clone(), g("Zero")));

    let even_odd = |i| {
        mfix(
            vec![
                ("even", lam("n", nat.clone(), app(v("odd"), v("n")))),
                ("odd", lam("n", nat.clone(), app(v("even"), v("n")))),
            ],
            i,
        )
    };
    assert_round_trips(&even_odd(0));
    assert_round_trips(&even_odd(1));
    assert_round_trips(&app(even_odd(1), g("Zero")));
    assert_round_trips(&mfix(
        vec![("solo", lam("n", nat.clone(), app(v("solo"), v("n"))))],
        0,
    ));
    // A plain `fix` in the last body of a mutual block must not swallow the block's `with`.
    assert_round_trips(&mfix(vec![("a", f.clone()), ("b", f.clone())], 0));
    // A mutual fix whose body refers to a GLOBAL spelled like a later member.
    assert_round_trips(&mfix(vec![("p", g("q")), ("q", v("p"))], 0));

    // A kernel `let` whose body USES its binder, and one whose body does not.
    assert_round_trips(&let_("x", nat.clone(), g("Zero"), app(g("Succ"), v("x"))));
    assert_round_trips(&let_("x", nat.clone(), g("Zero"), g("Zero")));
    assert_round_trips(&let_(
        "x",
        nat.clone(),
        let_("y", nat.clone(), g("Zero"), v("y")),
        let_(
            "z",
            pi("_", nat.clone(), nat.clone()),
            lam("w", nat.clone(), v("x")),
            app(v("z"), v("x")),
        ),
    ));
    assert_round_trips(&Term::Hole);
    assert_round_trips(&lam("x", Term::Hole, v("x")));
}

#[test]
fn a_term_with_no_spelling_is_refused_by_name_never_approximated() {
    // A free local variable reads back as a GLOBAL; printing it would be an approximation.
    assert_no_spelling(&v("x"), "x");
    assert_no_spelling(&app(g("f"), v("loose")), "loose");
    // A parser marker standing where no production puts it.
    assert_no_spelling(&g("\u{27e8}proj\u{27e9}"), "\u{27e8}proj\u{27e9}");
    // The empty name and a name holding the guillemet itself.
    assert_no_spelling(&g(""), "name");
    assert_no_spelling(&g("a\u{bb}b"), "a\u{bb}b");
    // `_` as a global is the kernel's own placeholder protocol, not a name.
    assert_no_spelling(&g("_"), "_");
    // SProp collapses out of every compound level; a raw one inside `max` has no level spelling.
    assert_no_spelling(&sort(umax(Universe::SProp, uvar("u"))), "SProp");
    // A BigInt holding a machine-sized value breaks the one-representation invariant.
    assert_no_spelling(&lit(Literal::BigInt(big("5"))), "BigInt");
    // A mutual block naming two members alike cannot say which one it denotes.
    assert_no_spelling(&mfix(vec![("f", g("A")), ("f", g("B"))], 1), "f");
}

#[test]
fn a_root_escaped_reference_prints_verbatim_and_reads_back_as_the_absolute_name() {
    let text = print_term(&app(g("_root_.a.b"), g("c"))).unwrap();
    assert_eq!(text, "_root_.a.b c");
    let known: BTreeSet<String> = ["a.b".to_string(), "c".to_string()].into_iter().collect();
    assert_eq!(
        read_back(&text, &known).unwrap(),
        app(g("a.b"), g("c")),
        "the escape resolves to the root"
    );
}

#[test]
fn display_is_not_the_printer_and_stays_what_it_was() {
    let t = pi("x", g("A"), app(g("B"), v("x")));
    assert_eq!(
        format!("{t}"),
        "Π(x:A). (B x)",
        "Display is the diagnostic rendering, unchanged"
    );
    assert_eq!(print_term(&t).unwrap(), "(x : A) -> B x");
    assert!(
        TermParser::parse(&format!("{t}")).is_err(),
        "Display was never re-readable"
    );
}

// =================================================================================================
// Depth — the kernel admits terms 1600 deep, and the text of one must read back on an ordinary stack
// =================================================================================================

/// A term exactly `depth` binders deep, every binder referred to at the bottom — the shape the
/// importer's let ladders and long telescopes take.
fn deep_term(depth: usize, shape: usize) -> Term {
    let mut t = v("x0");
    for i in (0..depth).rev() {
        let name = format!("x{i}");
        t = match shape % 4 {
            0 => lam(&name, g("Nat"), t),
            1 => pi(&name, g("Nat"), app(t, v(&name))),
            2 => let_(&name, g("Nat"), g("Zero"), t),
            _ => app(g("Succ"), lam(&name, g("Nat"), t)),
        };
    }
    lam("x0", g("Nat"), t)
}

#[test]
fn a_term_as_deep_as_the_kernel_admits_reads_back_on_an_ordinary_stack() {
    // The printer and the reader both run on an ORDINARY thread — 2 MiB, the size a test harness
    // and a language server hand out — not on a stack sized for the occasion. A term the kernel
    // admits must never overflow either of them.
    let depth = 1590;
    for shape in 0..4 {
        let t = std::thread::Builder::new()
            .stack_size(256 << 20)
            .spawn(move || deep_term(depth, shape))
            .unwrap()
            .join()
            .unwrap();
        let (text, read) = std::thread::scope(|scope| {
            std::thread::Builder::new()
                .stack_size(2 << 20)
                .spawn_scoped(scope, || {
                    let text = print_term(&t).expect("a spelling");
                    let read = TermParser::parse(&text).expect("the text reads back");
                    (text, read)
                })
                .unwrap()
                .join()
                .unwrap()
        });
        let equal = std::thread::Builder::new()
            .stack_size(256 << 20)
            .spawn(move || {
                let same = alpha_eq(&t, &read);
                std::mem::forget(t);
                std::mem::forget(read);
                same
            })
            .unwrap()
            .join()
            .unwrap();
        assert!(
            equal,
            "shape {shape}: a deep term does not round-trip ({} bytes of text)",
            text.len()
        );
    }
}

// =================================================================================================
// The generated corpus
// =================================================================================================

/// xorshift64* — deterministic, seedable, no dependency.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn pick<'a, T>(&mut self, xs: &'a [T]) -> &'a T {
        &xs[self.below(xs.len())]
    }
}

const BINDER_NAMES: &[&str] = &[
    "x",
    "y",
    "x",
    "n",
    "Nat",
    "Eq",
    "Zero",
    "f",
    "arena.local.3",
    "fun",
    "Prop",
    "Type",
    "Sort",
    "_",
    "a b",
    "h'",
    "\u{3b1}",
    "max",
    "end",
    "x1",
    "",
    "with",
    "Type0",
    "/s:\"a\"",
];

const GLOBAL_NAMES: &[&str] = &[
    "Nat",
    "Zero",
    "Succ",
    "Eq",
    "f",
    "x",
    "y",
    "Nat.add",
    "A.B.c",
    "fun",
    "Prop",
    "Type0",
    "Sort",
    "a b",
    "/s:\"Nat\"/s:\"add\"",
    "arena.structure.projection.P.0",
    "with",
    "max",
    "Outer.a b",
    "\u{3b1}",
    "n",
];

const LEVEL_NAMES: &[&str] = &["u", "v", "w", "max", "/s:\"u\"", "Prop", "u v", "imax"];

fn gen_level(r: &mut Rng, depth: usize) -> Universe {
    let leaf = depth == 0 || r.below(3) == 0;
    if leaf {
        return match r.below(5) {
            0 => Universe::Prop,
            1 => Universe::Type(*r.pick(&[0u32, 1, 2, 7, u32::MAX])),
            _ => uvar(r.pick(LEVEL_NAMES)),
        };
    }
    match r.below(3) {
        0 => succ(gen_level(r, depth - 1)),
        1 => umax(gen_level(r, depth - 1), gen_level(r, depth - 1)),
        _ => uimax(gen_level(r, depth - 1), gen_level(r, depth - 1)),
    }
}

fn gen_literal(r: &mut Rng) -> Literal {
    match r.below(8) {
        0 => Literal::Int(*r.pick(&[0i64, 1, -1, 42, i64::MIN, i64::MAX])),
        1 => Literal::BigInt(big(
            r.pick(&["18446744073709551616", "-99999999999999999999"])
        )),
        2 => Literal::Nat(big(r.pick(&["0", "3", "18446744073709551616"]))),
        3 => Literal::Float(*r.pick(&[0.0f64, -0.0, 1.5, 1e-9, f64::NAN, f64::INFINITY])),
        4 => Literal::Text((*r.pick(&["", "a", "x\"y", "tab\there", "«q» ⟨a⟩ . b"])).to_string()),
        5 => Literal::Duration(*r.pick(&[0i64, 1, -1_000_000, 60_000_000_000, i64::MIN])),
        6 => Literal::Date(*r.pick(&[0i32, -1, 19_000, i32::MIN, i32::MAX])),
        _ => Literal::Moment(*r.pick(&[0i64, -1, 1_234_567_890_123_456_789, i64::MIN, i64::MAX])),
    }
}

/// A random term whose free variables are exactly within `scope` — closed when `scope` is empty.
fn gen_term(r: &mut Rng, depth: usize, scope: &mut Vec<String>) -> Term {
    let leaf = depth == 0 || r.below(4) == 0;
    if leaf {
        return match r.below(7) {
            0 | 1 if !scope.is_empty() => Term::Var(r.pick(scope).clone()),
            0 | 1 | 2 => g(r.pick(GLOBAL_NAMES)),
            3 => sort(gen_level(r, 2)),
            4 => konst(
                r.pick(GLOBAL_NAMES),
                (0..r.below(3)).map(|_| gen_level(r, 2)).collect(),
            ),
            5 => lit(gen_literal(r)),
            _ => Term::Hole,
        };
    }
    let d = depth - 1;
    let bind = |r: &mut Rng,
                scope: &mut Vec<String>,
                name: &str,
                f: &mut dyn FnMut(&mut Rng, &mut Vec<String>) -> Term| {
        scope.push(name.to_string());
        let body = f(r, scope);
        scope.pop();
        body
    };
    match r.below(9) {
        0 | 1 => {
            let head = gen_term(r, d, scope);
            let arg = gen_term(r, d, scope);
            app(head, arg)
        }
        2 => {
            let name = r.pick(BINDER_NAMES).to_string();
            let ty = gen_term(r, d, scope);
            let body = bind(r, scope, &name, &mut |r, s| gen_term(r, d, s));
            pi(&name, ty, body)
        }
        3 => {
            let name = r.pick(BINDER_NAMES).to_string();
            let ty = gen_term(r, d, scope);
            let body = bind(r, scope, &name, &mut |r, s| gen_term(r, d, s));
            lam(&name, ty, body)
        }
        4 => {
            let disc = gen_term(r, d, scope);
            let motive = gen_term(r, d, scope);
            let cases = (0..r.below(4)).map(|_| gen_term(r, d, scope)).collect();
            matchp(disc, motive, cases)
        }
        5 => {
            let name = r.pick(BINDER_NAMES).to_string();
            let body = bind(r, scope, &name, &mut |r, s| gen_term(r, d, s));
            fix(&name, body)
        }
        6 => {
            let count = 1 + r.below(3);
            let mut names: Vec<String> = Vec::new();
            while names.len() < count {
                let n = r.pick(BINDER_NAMES).to_string();
                if !names.contains(&n) {
                    names.push(n);
                }
            }
            for n in &names {
                scope.push(n.clone());
            }
            let defs: Vec<(String, Term)> = names
                .iter()
                .map(|n| (n.clone(), gen_term(r, d, scope)))
                .collect();
            for _ in &names {
                scope.pop();
            }
            let index = r.below(count);
            Term::MutualFix { defs, index }
        }
        7 => {
            let name = r.pick(BINDER_NAMES).to_string();
            let ty = gen_term(r, d, scope);
            let value = gen_term(r, d, scope);
            let body = bind(r, scope, &name, &mut |r, s| gen_term(r, d, s));
            let_(&name, ty, value, body)
        }
        _ => lit(gen_literal(r)),
    }
}

#[test]
fn the_generated_corpus_round_trips() {
    std::thread::Builder::new()
        .stack_size(256 << 20)
        .spawn(|| {
            let mut r = Rng(0x5eed_1a5e_c0de_f00d);
            let mut checked = 0usize;
            for depth in 1..=7 {
                for _ in 0..600 {
                    let t = gen_term(&mut r, depth, &mut Vec::new());
                    assert_round_trips(&t);
                    checked += 1;
                }
            }
            assert_eq!(checked, 4200, "the corpus is the size it claims");
        })
        .unwrap()
        .join()
        .unwrap();
}

// =================================================================================================
// Every term of every declaration of every original bridge fixture
// =================================================================================================

fn fixtures_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/lean_conformance/cases_lean414")
}

/// The kernel keys of every declaration an export names, with the export's own kind for each
/// (`def`, `thm`, `opaque`, `axiom`, `inductive`), spelled the way the importer spells them.
/// Written HERE from the export format rather than taken from the importer, so a declaration the
/// importer registered under an unexpected key is a failure, not a silent omission.
fn declared(export: &str) -> Vec<(String, String)> {
    use std::collections::HashMap;
    let mut names: HashMap<u64, String> = HashMap::from([(0, String::new())]);
    let mut out = Vec::new();
    let key = |raw: &str| -> Option<String> {
        match raw {
            "/s:\"Nat\"" => Some("Nat".into()),
            "/s:\"Nat\"/s:\"zero\"" => Some("Zero".into()),
            "/s:\"Nat\"/s:\"succ\"" => Some("Succ".into()),
            other => Some(match other.strip_prefix("/s:\"Nat\"/") {
                Some(suffix) => format!("Nat/{suffix}"),
                None => other.to_string(),
            }),
        }
    };
    for line in export.lines() {
        let row: serde_json::Value = serde_json::from_str(line).expect("an NDJSON row");
        if let Some(i) = row.get("in").and_then(|i| i.as_u64()) {
            let (kind, x) = match (row.get("str"), row.get("num")) {
                (Some(x), _) => ("s", x),
                (_, Some(x)) => ("n", x),
                _ => panic!("a name row"),
            };
            let pre = names[&x["pre"].as_u64().unwrap()].clone();
            let part = if kind == "s" {
                serde_json::to_string(x["str"].as_str().unwrap()).unwrap()
            } else {
                x["i"].as_u64().unwrap().to_string()
            };
            names.insert(i, format!("{pre}/{kind}:{part}"));
            continue;
        }
        let Some(obj) = row.as_object() else { continue };
        if obj.len() != 1 {
            continue;
        }
        let (kind, d) = obj.iter().next().unwrap();
        let name_of = |v: &serde_json::Value| key(&names[&v.as_u64().unwrap()]);
        match kind.as_str() {
            "def" | "thm" | "opaque" | "axiom" => {
                out.extend(name_of(&d["name"]).map(|n| (kind.clone(), n)));
            }
            "inductive" => {
                for field in ["types", "ctors", "recs"] {
                    for item in d[field].as_array().unwrap() {
                        out.extend(name_of(&item["name"]).map(|n| (kind.clone(), n)));
                    }
                }
            }
            _ => {}
        }
    }
    out
}

fn declared_names(export: &str) -> Vec<String> {
    declared(export).into_iter().map(|(_, n)| n).collect()
}

/// Every (label, term) the imported context holds for `names`, plus every other definition,
/// declaration, inductive and constructor it registered on the way (sharing helpers, derived
/// eliminators).
fn declaration_terms(ctx: &logicaffeine_kernel::Context, names: &[String]) -> Vec<(String, Term)> {
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    for name in names {
        let mut found = false;
        if let Some((_, ty, body)) = ctx.get_universe_poly(name) {
            out.push((format!("{name} : type"), ty.clone()));
            out.push((format!("{name} : body"), body.clone()));
            found = true;
        }
        if let Some((_, ty)) = ctx.poly_declaration(name) {
            out.push((format!("{name} : header"), ty.clone()));
            found = true;
        }
        if let Some(ty) = ctx.get_global(name) {
            out.push((format!("{name} : global type"), ty.clone()));
            found = true;
        }
        if let Some(body) = ctx.get_definition_body(name) {
            out.push((format!("{name} : definition body"), body.clone()));
        }
        assert!(
            found,
            "the export declares `{name}` and the imported context holds no such key"
        );
        seen.insert(name.clone());
    }
    // Everything ELSE the import registered: sharing helpers, structure projections, derived
    // eliminators. The declared names were covered above and are not re-checked.
    for (name, ty) in ctx.iter_declarations() {
        if !seen.contains(name) {
            out.push((format!("{name} : declaration"), ty.clone()));
        }
    }
    for (name, ty, body) in ctx.iter_definitions() {
        if !seen.contains(name) {
            out.push((format!("{name} : definition type"), ty.clone()));
            out.push((format!("{name} : definition body"), body.clone()));
        }
    }
    for (name, arity) in ctx.iter_inductives() {
        if !seen.contains(name) {
            out.push((format!("{name} : arity"), arity.clone()));
        }
        for (ctor, ty) in ctx.get_constructors(name) {
            if !seen.contains(ctor) {
                out.push((format!("{ctor} : constructor"), ty.clone()));
            }
        }
    }
    out
}

/// One original bridge fixture: every declaration's type and value, every constructor, every
/// registered helper and derived eliminator, and — re-imported with its source retained — every
/// theorem's PROOF, which the kernel keeps only as checked evidence.
fn fixture_round_trips(file: &str, retain_theorem_proofs: bool) {
    let started = std::time::Instant::now();
    let path = fixtures_dir().join(file);
    let export = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{file}: {e}"));
    let names = declared_names(&export);
    let (imported, _) = logicaffeine_kernel::interface::lean::import_audited(&export)
        .unwrap_or_else(|e| panic!("{file} must import to be covered: {e:?}"));
    let imported_at = started.elapsed();
    let mut terms = declaration_terms(&imported, &names);
    if retain_theorem_proofs {
        for theorem in theorem_names(&export) {
            let (retained, _) =
                logicaffeine_kernel::interface::lean::import_audited_retaining_source(
                    &export, &theorem,
                )
                .unwrap_or_else(|e| panic!("{file}: `{theorem}` must retain its proof: {e:?}"));
            let (_, ty, proof) = retained.retained_source().expect("a retained source");
            terms.push((format!("{theorem} : retained type"), ty.clone()));
            terms.push((format!("{theorem} : retained proof"), proof.clone()));
        }
    }
    let depth = terms
        .iter()
        .map(|(_, t)| logicaffeine_kernel::term_store::term_depth(t))
        .max()
        .unwrap_or(0);
    let stack = logicaffeine_kernel::term_store::stack_bytes_for_depth(depth.saturating_mul(32));
    let checked = std::thread::scope(|scope| {
        std::thread::Builder::new()
            .stack_size(stack.max(256 << 20))
            .spawn_scoped(scope, || {
                for (what, t) in &terms {
                    let printed = print_term(t)
                        .unwrap_or_else(|e| panic!("{file}: `{what}` has no spelling: {e}"));
                    let mut known = BTreeSet::new();
                    globals_of(t, &mut known);
                    let read = read_back(&printed, &known).unwrap_or_else(|e| {
                        panic!(
                            "{file}: `{what}` printed as text that does not parse: {e}\n{printed}"
                        )
                    });
                    assert!(
                        alpha_eq(t, &read),
                        "{file}: `{what}` does not round-trip\n  text: {printed}"
                    );
                }
                terms.len()
            })
            .unwrap()
            .join()
            .unwrap()
    });
    assert!(
        checked > names.len(),
        "{file}: every declared name contributes at least one term"
    );
    eprintln!(
        "{file}: {} declarations, {checked} terms round-trip (import {:?}, total {:?})",
        names.len(),
        imported_at,
        started.elapsed()
    );
}

/// The kernel keys of the export's `thm` declarations.
fn theorem_names(export: &str) -> Vec<String> {
    declared(export)
        .into_iter()
        .filter(|(k, _)| k == "thm")
        .map(|(_, n)| n)
        .collect()
}

const FIXTURES: &[&str] = &[
    "T00-001.ndjson",
    "T00-002.ndjson",
    "T00-003.structural.ndjson",
    "T00-004.ndjson",
    "T00-005.ndjson",
    "T00-005.structural.ndjson",
    "T00-014.ndjson",
    "T00-017.ndjson",
    "T00-019.sharing.structural.ndjson",
    "T02-014.forestSize.ndjson",
    "T03-008.binductionOn_1.ndjson",
    "T04-015.accDepth.ndjson",
    "T04-015.four.ndjson",
    "T06-002.ndjson",
    "T06-006.existsNested.ndjson",
];

#[test]
fn every_original_bridge_fixture_is_covered_by_a_round_trip_test() {
    let mut on_disk: Vec<String> = std::fs::read_dir(fixtures_dir())
        .expect("the bridge fixture directory")
        .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
        .filter(|n| n.ends_with(".ndjson"))
        .collect();
    on_disk.sort();
    assert_eq!(
        on_disk, FIXTURES,
        "a fixture added to the directory needs its round-trip test"
    );
}

macro_rules! fixture_tests {
    ($($test:ident => $file:literal, $proofs:literal;)*) => {
        $(
            #[test]
            fn $test() {
                fixture_round_trips($file, $proofs);
            }
        )*
    };
}

fixture_tests! {
    fixture_t00_001_round_trips => "T00-001.ndjson", true;
    fixture_t00_002_round_trips => "T00-002.ndjson", true;
    fixture_t00_003_structural_round_trips => "T00-003.structural.ndjson", true;
    fixture_t00_004_round_trips => "T00-004.ndjson", true;
    fixture_t00_005_round_trips => "T00-005.ndjson", true;
    fixture_t00_005_structural_round_trips => "T00-005.structural.ndjson", true;
    fixture_t00_014_round_trips => "T00-014.ndjson", true;
    fixture_t00_017_round_trips => "T00-017.ndjson", false;
    fixture_t00_019_sharing_structural_round_trips => "T00-019.sharing.structural.ndjson", true;
    fixture_t02_014_forest_size_round_trips => "T02-014.forestSize.ndjson", true;
    fixture_t03_008_binduction_on_round_trips => "T03-008.binductionOn_1.ndjson", true;
    fixture_t04_015_acc_depth_round_trips => "T04-015.accDepth.ndjson", true;
    fixture_t04_015_four_round_trips => "T04-015.four.ndjson", true;
    fixture_t06_002_round_trips => "T06-002.ndjson", true;
    fixture_t06_006_exists_nested_round_trips => "T06-006.existsNested.ndjson", true;
}

// =================================================================================================
// A whole fixture, replayed from its printed LOGOS into an EMPTY context
// =================================================================================================

/// One declaration record of an export, in export order.
enum Unit {
    /// A `quot` record: the primitive's kind.
    Quotient(logicaffeine_kernel::quotient::Kind),
    /// An `inductive` record: its types, constructors and recursors.
    Block {
        types: Vec<String>,
        recursors: Vec<String>,
    },
    /// `def`, `thm`, `opaque` or `axiom`.
    Declaration { kind: String, name: String },
}

fn export_units(export: &str) -> Vec<Unit> {
    use logicaffeine_kernel::quotient::Kind;
    let all = declared(export);
    let mut next = all.into_iter();
    let mut units = Vec::new();
    for line in export.lines() {
        let row: serde_json::Value = serde_json::from_str(line).unwrap();
        let Some(obj) = row.as_object() else { continue };
        if obj.len() != 1 {
            continue;
        }
        let (kind, d) = obj.iter().next().unwrap();
        match kind.as_str() {
            "quot" => units.push(Unit::Quotient(match d["kind"].as_str().unwrap() {
                "type" => Kind::Type,
                "ctor" => Kind::Constructor,
                "lift" => Kind::Lift,
                _ => Kind::Induction,
            })),
            "def" | "thm" | "opaque" | "axiom" => {
                let (_, name) = next.next().expect("a declared name");
                units.push(Unit::Declaration {
                    kind: kind.clone(),
                    name,
                });
            }
            "inductive" => {
                let count = |f: &str| d[f].as_array().unwrap().len();
                let types: Vec<String> = (0..count("types"))
                    .map(|_| next.next().unwrap().1)
                    .collect();
                for _ in 0..count("ctors") {
                    next.next();
                }
                let recursors = (0..count("recs")).map(|_| next.next().unwrap().1).collect();
                units.push(Unit::Block { types, recursors });
            }
            _ => {}
        }
    }
    units
}

/// The command registering the definition `name` exactly as `ctx` holds it, under the same name.
fn definition_command(ctx: &logicaffeine_kernel::Context, name: &str) -> Command {
    match (ctx.get_universe_poly(name), ctx.get_definition_body(name)) {
        // A telescope written on a declaration is what put it in the polymorphic table; a
        // monomorphic definition is registered through `declare` and carries none.
        (Some((params, ty, body)), _) if !params.is_empty() => Command::Universes {
            params: params.clone(),
            inner: Box::new(Command::Definition {
                name: name.to_string(),
                ty: Some(ty.clone()),
                body: body.clone(),
                is_hint: false,
                implicit_count: 0,
                proof_goal: None,
            }),
        },
        (_, Some(body)) => Command::Definition {
            name: name.to_string(),
            ty: Some(
                ctx.get_definition_type(name)
                    .expect("a definition's type")
                    .clone(),
            ),
            body: body.clone(),
            is_hint: false,
            implicit_count: 0,
            proof_goal: None,
        },
        (Some((params, ty, body)), None) => Command::Universes {
            params: params.clone(),
            inner: Box::new(Command::Definition {
                name: name.to_string(),
                ty: Some(ty.clone()),
                body: body.clone(),
                is_hint: false,
                implicit_count: 0,
                proof_goal: None,
            }),
        },
        (None, None) => panic!("`{name}` is registered as no definition"),
    }
}

fn globals_of_command(cmd: &Command, out: &mut BTreeSet<String>) {
    match cmd {
        Command::Structure {
            inductive,
            projections,
        } => {
            out.insert(inductive.clone());
            out.extend(projections.iter().cloned());
        }
        Command::Universes { inner, .. } => globals_of_command(inner, out),
        Command::Definition { ty, body, .. } => {
            if let Some(ty) = ty {
                globals_of(ty, out);
            }
            globals_of(body, out);
        }
        Command::Theorem { ty, proof, .. } => {
            globals_of(ty, out);
            globals_of(proof, out);
        }
        Command::Axiom { ty, .. } => globals_of(ty, out),
        Command::InductiveBlock { members } => {
            for m in members {
                m.params.iter().for_each(|(_, t)| globals_of(t, out));
                globals_of(&m.sort, out);
                m.constructors.iter().for_each(|(_, t)| globals_of(t, out));
            }
        }
        Command::Quotient { equality, .. } => out.extend(equality.iter().cloned()),
        _ => {}
    }
}

/// ★ THE WHOLE FIXTURE, REPLAYED FROM PRINTED LOGOS. Every declaration the import registered —
/// quotient primitives, inductive blocks (through `printer::inductive_command`), their recursors,
/// definitions, theorems with their retained proofs, axioms, and the sharing helpers the importer
/// introduced — printed, and executed by a `Repl` whose context starts EMPTY, in dependency order.
/// Then every declared name must be registered with the same kernel terms the importer registered.
///
/// Nothing is shared with the import but the terms themselves: no binder kinds, no helper tables,
/// no quotient certificate. What the printed text says is all there is.
fn replay_from_printed_logos(file: &str) {
    use logicaffeine_kernel::interface::printer::inductive_command;
    use logicaffeine_kernel::interface::Repl;
    use logicaffeine_kernel::quotient::Kind;
    let export = std::fs::read_to_string(fixtures_dir().join(file)).unwrap();
    let (imported, report) = logicaffeine_kernel::interface::lean::import_audited(&export)
        .unwrap_or_else(|e| panic!("{file} must import to be covered: {e:?}"));
    let ctx: &logicaffeine_kernel::Context = &imported;

    // (keys it declares, command) per unit, helpers pooled separately.
    let mut units: Vec<(Vec<String>, Command)> = Vec::new();
    for unit in export_units(&export) {
        match unit {
            Unit::Quotient(kind) => {
                let (params, _) = ctx
                    .poly_declaration(kind.name())
                    .expect("an installed primitive");
                let equality = (kind == Kind::Lift).then(|| "/s:\"Eq\"".to_string());
                units.push((
                    vec![kind.name().to_string()],
                    Command::Universes {
                        params: params.to_vec(),
                        inner: Box::new(Command::Quotient { kind, equality }),
                    },
                ));
            }
            Unit::Block { types, recursors } => {
                let block = inductive_command(ctx, &types[0]).unwrap_or_else(|e| {
                    panic!("{file}: `{}` has no inductive command: {e}", types[0])
                });
                let mut keys = types.clone();
                for t in &types {
                    keys.extend(ctx.get_constructors(t).iter().map(|(c, _)| c.to_string()));
                }
                units.push((keys, block));
                // Structure η, exactly as the import granted it: the projections, then the
                // checked `Structure` command — before the recursor, which relies on η.
                for t in &types {
                    for command in
                        logicaffeine_kernel::interface::printer::structure_commands(ctx, t)
                            .unwrap_or_else(|e| {
                                panic!("{file}: `{t}`'s structure has no spelling: {e}")
                            })
                    {
                        let keys = match &command {
                            Command::Definition { name, .. } => vec![name.clone()],
                            Command::Universes { inner, .. } => match inner.as_ref() {
                                Command::Definition { name, .. } => vec![name.clone()],
                                _ => Vec::new(),
                            },
                            _ => Vec::new(),
                        };
                        units.push((keys, command));
                    }
                }
                for rec in recursors {
                    units.push((vec![rec.clone()], definition_command(ctx, &rec)));
                }
            }
            Unit::Declaration { kind, name } => {
                let command = match kind.as_str() {
                    "def" => definition_command(ctx, &name),
                    "axiom" => {
                        let (params, ty) = ctx.poly_declaration(&name).expect("an axiom's header");
                        let axiom = Command::Axiom {
                            name: name.clone(),
                            ty: ty.clone(),
                        };
                        if params.is_empty() {
                            axiom
                        } else {
                            Command::Universes {
                                params: params.to_vec(),
                                inner: Box::new(axiom),
                            }
                        }
                    }
                    _ => {
                        let (retained, _) =
                            logicaffeine_kernel::interface::lean::import_audited_retaining_source(
                                &export, &name,
                            )
                            .unwrap_or_else(|e| {
                                panic!("{file}: `{name}` must retain its proof: {e:?}")
                            });
                        let (_, ty, proof) = retained.retained_source().expect("a retained source");
                        let (params, _) = ctx.poly_declaration(&name).expect("a header");
                        let opaque = if kind == "thm" {
                            Command::Theorem {
                                name: name.clone(),
                                ty: ty.clone(),
                                proof: proof.clone(),
                            }
                        } else {
                            Command::Opaque {
                                name: name.clone(),
                                ty: ty.clone(),
                                value: proof.clone(),
                            }
                        };
                        if params.is_empty() {
                            opaque
                        } else {
                            Command::Universes {
                                params: params.to_vec(),
                                inner: Box::new(opaque),
                            }
                        }
                    }
                };
                units.push((vec![name], command));
            }
        }
    }
    units.extend(
        report
            .internal_helpers
            .iter()
            .map(|h| (vec![h.name.clone()], definition_command(ctx, &h.name))),
    );

    // Emit in dependency order — a unit once everything it mentions that some unit declares is
    // registered; among the ready ones, the earliest in export order.
    let declares: std::collections::HashMap<String, usize> = units
        .iter()
        .enumerate()
        .flat_map(|(i, (keys, _))| keys.iter().map(move |k| (k.clone(), i)))
        .collect();
    let needs: Vec<BTreeSet<usize>> = units
        .iter()
        .enumerate()
        .map(|(i, (_, command))| {
            let mut mentioned = BTreeSet::new();
            globals_of_command(command, &mut mentioned);
            mentioned
                .iter()
                .filter_map(|m| declares.get(m).copied())
                .filter(|j| *j != i)
                .collect()
        })
        .collect();
    let mut repl = Repl::new();
    *repl.context_mut() = logicaffeine_kernel::Context::new();
    let mut declared_so_far: BTreeSet<String> = BTreeSet::new();
    let mut emitted = vec![false; units.len()];
    for _ in 0..units.len() {
        let next = (0..units.len())
            .find(|&i| !emitted[i] && needs[i].iter().all(|j| emitted[*j]))
            .unwrap_or_else(|| {
                panic!("{file}: the remaining declarations depend on each other in a cycle")
            });
        let (keys, command) = &units[next];
        let text = print_command(command)
            .unwrap_or_else(|e| panic!("{file}: {keys:?} has no spelling: {e}"));
        repl.execute(&text).unwrap_or_else(|e| {
            panic!("{file}: {keys:?} printed as a command the Repl refuses: {e}\n{text}")
        });
        declared_so_far.extend(keys.iter().cloned());
        emitted[next] = true;
    }

    // Every declared key is registered with the importer's own terms.
    let got = repl.context();
    let mut compared = 0usize;
    for key in &declared_so_far {
        if let Some((params, ty)) = ctx.poly_declaration(key) {
            let (gp, gt) = got
                .poly_declaration(key)
                .unwrap_or_else(|| panic!("{file}: `{key}` lost its telescope"));
            assert_eq!(params, gp, "{file}: `{key}`'s universe telescope");
            assert!(
                alpha_eq(ty, gt),
                "{file}: `{key}`'s header changed\n  imported: {ty}\n  replayed: {gt}"
            );
        }
        if let Some(ty) = ctx.get_global(key) {
            let replayed = got
                .get_global(key)
                .unwrap_or_else(|| panic!("{file}: `{key}` missing"));
            assert!(
                alpha_eq(ty, replayed),
                "{file}: `{key}`'s type changed\n  imported: {ty}\n  replayed: {replayed}"
            );
        }
        if let Some(body) = ctx.get_definition_body(key) {
            assert!(
                alpha_eq(body, got.get_definition_body(key).expect("a body")),
                "{file}: `{key}`'s body changed"
            );
        }
        if let Some((_, _, body)) = ctx.get_universe_poly(key) {
            assert!(
                alpha_eq(
                    body,
                    &got.get_universe_poly(key).expect("a polymorphic body").2
                ),
                "{file}: `{key}`'s body changed"
            );
        }
        if ctx.is_inductive(key) {
            assert_eq!(
                ctx.struct_info(key)
                    .map(|i| (i.mk.clone(), i.num_params, i.projections.clone())),
                got.struct_info(key)
                    .map(|i| (i.mk.clone(), i.num_params, i.projections.clone())),
                "{file}: `{key}`'s structure η"
            );
            assert_eq!(
                ctx.inductive_num_params(key),
                got.inductive_num_params(key),
                "{file}: `{key}` params"
            );
            assert_eq!(
                ctx.inductive_num_indices(key),
                got.inductive_num_indices(key),
                "{file}: `{key}` indices"
            );
            let (want, have) = (ctx.get_constructors(key), got.get_constructors(key));
            assert_eq!(
                want.iter().map(|(n, _)| *n).collect::<Vec<_>>(),
                have.iter().map(|(n, _)| *n).collect::<Vec<_>>(),
                "{file}: `{key}` constructors"
            );
        }
        compared += 1;
    }
    assert!(compared > 0, "{file}: something was replayed");
    eprintln!("{file}: {compared} declarations replayed from printed LOGOS into an empty context");
}

macro_rules! replay_tests {
    ($($test:ident => $file:literal;)*) => {
        $(
            #[test]
            fn $test() {
                std::thread::Builder::new()
                    .stack_size(512 << 20)
                    .spawn(|| replay_from_printed_logos($file))
                    .unwrap()
                    .join()
                    .unwrap();
            }
        )*
    };
}

replay_tests! {
    replay_t00_001 => "T00-001.ndjson";
    replay_t00_002 => "T00-002.ndjson";
    replay_t00_003_structural => "T00-003.structural.ndjson";
    replay_t00_004 => "T00-004.ndjson";
    replay_t00_005 => "T00-005.ndjson";
    replay_t00_005_structural => "T00-005.structural.ndjson";
    replay_t00_014 => "T00-014.ndjson";
    replay_t00_019_sharing_structural => "T00-019.sharing.structural.ndjson";
    replay_t02_014_forest_size => "T02-014.forestSize.ndjson";
    replay_t03_008_binduction_on => "T03-008.binductionOn_1.ndjson";
    replay_t04_015_acc_depth => "T04-015.accDepth.ndjson";
    replay_t04_015_four => "T04-015.four.ndjson";
    replay_t06_002 => "T06-002.ndjson";
    replay_t06_006_exists_nested => "T06-006.existsNested.ndjson";
}

// =================================================================================================
// Every term of the standard library
// =================================================================================================

/// Does `t` contain the kernel's own match-case placeholder, the global `_`?
fn has_placeholder(t: &Term) -> bool {
    let mut names = BTreeSet::new();
    globals_of(t, &mut names);
    names.contains("_")
}

/// ★ THE PRELUDE IS A CORPUS TOO. Thousands of kernel terms built in Rust rather than parsed —
/// eliminators, `fix`/mutual-`fix` bodies, positional matches, lets, literals, universe-polymorphic
/// constants. Every one of them either round-trips, or is refused because it holds the kernel's
/// match-case placeholder `Global("_")` — a term no source can write, since the surface's `_` is a
/// hole — and nothing else is refused.
#[test]
fn every_term_of_the_standard_library_round_trips_or_holds_the_placeholder() {
    std::thread::Builder::new()
        .stack_size(512 << 20)
        .spawn(|| {
            let ctx = logicaffeine_kernel::interface::standard_context();
            let mut terms: Vec<(String, Term)> = Vec::new();
            for (name, ty) in ctx.iter_declarations() {
                terms.push((format!("{name} : declaration"), ty.clone()));
            }
            for (name, ty, body) in ctx.iter_definitions() {
                terms.push((format!("{name} : type"), ty.clone()));
                terms.push((format!("{name} : body"), body.clone()));
            }
            for (name, arity) in ctx.iter_inductives() {
                terms.push((format!("{name} : arity"), arity.clone()));
                for (ctor, ty) in ctx.get_constructors(name) {
                    terms.push((format!("{ctor} : constructor"), ty.clone()));
                }
            }
            terms.sort_by(|a, b| a.0.cmp(&b.0));
            let (mut round_tripped, mut placeholders) = (0usize, 0usize);
            for (what, t) in &terms {
                if has_placeholder(t) {
                    match print_term(t) {
                        Err(PrintError::NoSpelling { what: why }) => {
                            assert!(
                                why.contains('_'),
                                "`{what}` refused for the wrong reason: {why}"
                            );
                            placeholders += 1;
                        }
                        Ok(text) => {
                            panic!("`{what}` holds the placeholder and yet printed: {text}")
                        }
                    }
                    continue;
                }
                assert_round_trips(t);
                round_tripped += 1;
            }
            assert!(
                round_tripped > 1000,
                "the prelude is the corpus it claims: {round_tripped}"
            );
            eprintln!(
                "prelude: {round_tripped} terms round-trip, {placeholders} hold the placeholder"
            );
        })
        .unwrap()
        .join()
        .unwrap();
}

// =================================================================================================
// Commands
// =================================================================================================

/// α-aware equality of two commands, field by field.
fn command_eq(a: &Command, b: &Command) -> bool {
    let terms_eq = |x: &[&Term], y: &[&Term]| {
        x.len() == y.len() && x.iter().zip(y).all(|(p, q)| alpha_eq(p, q))
    };
    let opt = |x: &Option<Term>, y: &Option<Term>| match (x, y) {
        (Some(x), Some(y)) => alpha_eq(x, y),
        (None, None) => true,
        _ => false,
    };
    let pairs_eq = |x: &Vec<(String, Term)>, y: &Vec<(String, Term)>| {
        x.len() == y.len()
            && x.iter()
                .zip(y)
                .all(|((n, t), (m, s))| n == m && alpha_eq(t, s))
    };
    match (a, b) {
        (
            Command::Definition {
                name,
                ty,
                body,
                is_hint,
                implicit_count,
                proof_goal,
            },
            Command::Definition {
                name: n2,
                ty: t2,
                body: b2,
                is_hint: h2,
                implicit_count: i2,
                proof_goal: p2,
            },
        ) => {
            name == n2
                && opt(ty, t2)
                && alpha_eq(body, b2)
                && is_hint == h2
                && implicit_count == i2
                && opt(proof_goal, p2)
        }
        (
            Command::Theorem { name, ty, proof },
            Command::Theorem {
                name: n2,
                ty: t2,
                proof: p2,
            },
        ) => name == n2 && terms_eq(&[ty, proof], &[t2, p2]),
        (
            Command::Opaque { name, ty, value },
            Command::Opaque {
                name: n2,
                ty: t2,
                value: v2,
            },
        ) => name == n2 && terms_eq(&[ty, value], &[t2, v2]),
        (Command::Axiom { name, ty }, Command::Axiom { name: n2, ty: t2 }) => {
            name == n2 && alpha_eq(ty, t2)
        }
        (
            Command::Inductive {
                name,
                params,
                sort,
                constructors,
            },
            Command::Inductive {
                name: n2,
                params: p2,
                sort: s2,
                constructors: c2,
            },
        ) => name == n2 && pairs_eq(params, p2) && alpha_eq(sort, s2) && pairs_eq(constructors, c2),
        (Command::InductiveBlock { members }, Command::InductiveBlock { members: m2 }) => {
            members.len() == m2.len()
                && members.iter().zip(m2).all(|(x, y)| {
                    x.name == y.name
                        && x.full_constructors == y.full_constructors
                        && pairs_eq(&x.params, &y.params)
                        && alpha_eq(&x.sort, &y.sort)
                        && pairs_eq(&x.constructors, &y.constructors)
                })
        }
        (
            Command::Universes { params, inner },
            Command::Universes {
                params: p2,
                inner: i2,
            },
        ) => params == p2 && command_eq(inner, i2),
        (
            Command::Structure {
                inductive,
                projections,
            },
            Command::Structure {
                inductive: i2,
                projections: p2,
            },
        ) => inductive == i2 && projections == p2,
        (
            Command::Quotient { kind, equality },
            Command::Quotient {
                kind: k2,
                equality: e2,
            },
        ) => kind == k2 && equality == e2,
        (Command::Check(x), Command::Check(y)) | (Command::Eval(x), Command::Eval(y)) => {
            alpha_eq(x, y)
        }
        (Command::Comment, Command::Comment) => true,
        (Command::PrintAxioms(x), Command::PrintAxioms(y)) => x == y,
        (Command::Protected(x), Command::Protected(y)) => command_eq(x, y),
        (
            Command::Export { namespace, names },
            Command::Export {
                namespace: n2,
                names: m2,
            },
        ) => namespace == n2 && names == m2,
        (Command::Open(x), Command::Open(y)) => format!("{x:?}") == format!("{y:?}"),
        (
            Command::OpenIn { opens, inner },
            Command::OpenIn {
                opens: o2,
                inner: i2,
            },
        ) => format!("{opens:?}") == format!("{o2:?}") && command_eq(inner, i2),
        (Command::Surface(x), Command::Surface(y)) => format!("{x:?}") == format!("{y:?}"),
        _ => false,
    }
}

const COMMANDS: &[&str] = &[
    "Definition one : Nat := Succ Zero.",
    "Definition two := Succ (Succ Zero).",
    "Definition idf : {A : Type} -> A -> A := fun {A} (a : A) => a.",
    "Definition strict : ⦃A : Type⦄ -> A -> A := fun ⦃A⦄ (a : A) => a.",
    "Definition inst : [Inhabited Nat] -> Nat := fun [Inhabited Nat] => Zero.",
    "Definition dflt : Int -> (m : Int := 3) -> Int := fun n m => add n m.",
    "Definition auto : (h : True := by trivial) -> Nat := fun h => Zero.",
    "Definition lazy := fun {A : Type} (a : A) => a.",
    "Definition named := f (x := Zero) (y := Succ Zero).",
    "Definition explicit := @f Nat Zero.",
    "Definition ellipsis := f ...",
    "Definition anon := ⟨Zero, Succ Zero⟩.",
    "Definition anon_rest := ⟨Zero, ..⟩.",
    "Definition proj := p.fst.",
    "Definition proj_num := p.1.",
    "Definition pat := fun n => match n with | Zero => Zero | Succ k => k end.",
    "Definition pat_return := fun n => match n return Nat with | Zero => Zero | Succ (Succ k) => k | _ => Zero.",
    "Definition id.{u} : (A : Sort u) -> A -> A := fun (A : Sort u) (a : A) => a.",
    "Definition lift.{u, v} : Sort (max u v) := PUnit.{max u v}.",
    "Definition «a b» : Nat := Zero.",
    "Definition Outer.Inner.value : Nat := Zero.",
    "Definition «/s:\"Nat\"/s:\"add\"» := Zero.",
    "Definition lit := f 5n 1.5f64 \"s\\n\" 5s 2026-05-20 2026-05-20T10:00:00.5Z (-3).",
    "Definition shared := fun (n : Nat) => let x : Nat := n; add x x.",
    "Definition subst := let x : Nat := Zero in Succ x.",
    "Definition evens := fix even => fun (n : Nat) => odd n with odd => fun (n : Nat) => even n return odd.",
    "Definition positional := fun (n : Nat) => match n return (fun (m : Nat) => Nat) with Zero | fun (k : Nat) => k end.",
    "Theorem refl_zero : Eq Nat Zero Zero := refl Nat Zero.",
    "Theorem poly.{u} : (A : Sort u) -> (a : A) -> Eq A a a := fun (A : Sort u) (a : A) => refl A a.",
    "Opaque secret : Nat := Succ Zero.",
    "Axiom choice.{u} : (A : Sort u) -> Nonempty A -> A.",
    "Axiom em : (p : Prop) -> Or p (Not p).",
    "Inductive Color := Red : Color | Green : Color.",
    "Inductive List (A : Type) := Nil : List A | Cons : A -> List A -> List A.",
    "Inductive Vec (A : Type) : Nat -> Type := vnil : Vec A Zero | vcons : (n : Nat) -> A -> Vec A n -> Vec A (Succ n).",
    "Inductive PBox.{u} (A : Sort u) : Sort (max 1 u) := pmk : A -> PBox.{u} A.",
    "Inductive Empty : Type.",
    "Inductive HAdd.{u, v, w} (a : Sort (u+1)) (b : Sort (v+1)) (c : Sort (w+1)) : Sort (max (u+1) (max (v+1) (w+1))) := HAdd.mk (a : Sort (u+1)) (b : Sort (v+1)) (c : outParam.{w+2} (Sort (w+1))) : (a -> b -> c) -> HAdd.{u, v, w} a b c.",
    "Inductive Even : Nat -> Prop := ev0 : Even Zero | evS : (n : Nat) -> Odd n -> Even (Succ n) with Odd : Nat -> Prop := odS : (n : Nat) -> Even n -> Odd (Succ n).",
    "Quotient Type.{u}.",
    "Structure Prod := Prod.fst Prod.snd.",
    "Structure «/s:\"PUnit\"» :=.",
    "Quotient Lift.{u, v} using \u{ab}/s:\"Eq\"\u{bb}.",
    "Check Succ Zero.",
    "Eval add (Succ Zero) (Succ Zero).",
    "/-- a doc comment -/",
    "Print Axioms refl_zero.",
    "Protected Definition Alpha.x : Nat := Zero.",
    "Protected Theorem Alpha.t : True := I.",
    "Export Alpha (x y).",
    "Open Alpha.",
    "Open Alpha (x) Beta hiding (y).",
    "Open Alpha renaming x -> z.",
    "Open Alpha in Definition w : Nat := x.",
    "Namespace Outer.",
    "End Outer.",
    "Section.",
    "Section S.",
    "End S.",
    "Notation \"a ⊕ b\" := (myOp a b).",
    "Infixl 65 \"⋄\" := f.",
    "Infixr 60 \"▷\" := g.",
    "Prefix \"∽\" := neg.",
    "Postfix \"‼\" := bang.",
    "Macro \"twice $e\" := (add $e $e).",
];

#[test]
fn every_command_the_vernacular_reads_prints_back_to_itself() {
    for text in COMMANDS {
        let parsed = parse_command(text).unwrap_or_else(|e| panic!("{text} must parse: {e:?}"));
        let printed = print_command(&parsed).unwrap_or_else(|e| panic!("{text}: no spelling: {e}"));
        let reparsed = parse_command(&printed).unwrap_or_else(|e| {
            panic!("{text} printed as {printed:?}, which does not parse: {e:?}")
        });
        assert!(
            command_eq(&parsed, &reparsed),
            "print∘parse is not the identity for a command:\n  source:  {text}\n  printed: {printed}\n  first:   {parsed:?}\n  second:  {reparsed:?}"
        );
        assert_eq!(
            print_command(&reparsed).unwrap(),
            printed,
            "printing a command is a fixpoint on its own output"
        );
    }
}

#[test]
fn a_command_slot_containing_a_delimiter_is_bracketed_not_misread() {
    // A kernel `let` in a declared TYPE puts a `:=` in it; a positional `match` in a constructor
    // type puts a `|` in it. Either would split the sentence at the wrong place if printed bare.
    let nat = g("Nat");
    let ty = let_(
        "x",
        sort(Universe::Type(0)),
        nat.clone(),
        pi("_", v("x"), v("x")),
    );
    let cmd = Command::Definition {
        name: "weird".into(),
        ty: Some(ty),
        body: lam("n", nat.clone(), v("n")),
        is_hint: false,
        implicit_count: 0,
        proof_goal: None,
    };
    let printed = print_command(&cmd).unwrap();
    let reparsed = parse_command(&printed).unwrap_or_else(|e| panic!("{printed}: {e:?}"));
    assert!(command_eq(&cmd, &reparsed), "{printed}");

    let ctor_ty = pi(
        "b",
        g("Bool"),
        app(
            g("Box"),
            matchp(
                v("b"),
                lam("_", g("Bool"), sort(Universe::Type(0))),
                vec![nat.clone(), g("Bool")],
            ),
        ),
    );
    let block = parse_command("Inductive Box : Type -> Type := mk : Box Nat.").unwrap();
    let Command::InductiveBlock { mut members } = block else {
        panic!("a block")
    };
    members[0].constructors = vec![("mk".into(), ctor_ty)];
    let cmd = Command::InductiveBlock { members };
    let printed = print_command(&cmd).unwrap();
    let reparsed = parse_command(&printed).unwrap_or_else(|e| panic!("{printed}: {e:?}"));
    assert!(command_eq(&cmd, &reparsed), "{printed}");
}

#[test]
fn a_command_with_no_vernacular_spelling_is_refused() {
    let hint = Command::Definition {
        name: "h".into(),
        ty: None,
        body: g("Zero"),
        is_hint: true,
        implicit_count: 0,
        proof_goal: None,
    };
    assert!(
        matches!(print_command(&hint), Err(PrintError::NoSpelling { .. })),
        "a hint flag"
    );
    let goal = Command::Definition {
        name: "t".into(),
        ty: None,
        body: g("Zero"),
        is_hint: false,
        implicit_count: 0,
        proof_goal: Some(g("Zero")),
    };
    assert!(
        matches!(print_command(&goal), Err(PrintError::NoSpelling { .. })),
        "a literate goal"
    );
    let miscount = Command::Definition {
        name: "m".into(),
        ty: Some(pi("A", sort(Universe::Type(0)), v("A"))),
        body: g("Zero"),
        is_hint: false,
        implicit_count: 1,
        proof_goal: None,
    };
    assert!(
        matches!(print_command(&miscount), Err(PrintError::NoSpelling { .. })),
        "an implicit count the type's binders do not state"
    );
    let unsortable = Command::Inductive {
        name: "P".into(),
        params: vec![],
        sort: sort(Universe::Prop),
        constructors: vec![("p".into(), g("P"))],
    };
    assert!(
        matches!(
            print_command(&unsortable),
            Err(PrintError::NoSpelling { .. })
        ),
        "the undeclared-arity form is always at `Type`"
    );
    // A universe telescope binds over a DECLARATION; on anything else it means nothing.
    let stray = Command::Universes {
        params: vec!["u".into()],
        inner: Box::new(Command::Check(g("Zero"))),
    };
    assert!(
        matches!(print_command(&stray), Err(PrintError::NoSpelling { .. })),
        "a telescope on a `Check`"
    );
    let nested = Command::Universes {
        params: vec!["u".into()],
        inner: Box::new(Command::Universes {
            params: vec!["v".into()],
            inner: Box::new(Command::Axiom {
                name: "a".into(),
                ty: sort(Universe::Prop),
            }),
        }),
    };
    assert!(
        matches!(print_command(&nested), Err(PrintError::NoSpelling { .. })),
        "two telescopes on one declaration"
    );
}
