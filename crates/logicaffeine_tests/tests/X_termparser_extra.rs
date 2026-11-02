//! ★ THE TERM SURFACE — the argument-passing conventions Lean users take for granted. ★
//!
//! Lane X of the Lean-parity campaign owns `interface/term_parser.rs`. The corpus cases it serves
//! (T05-005 named arguments, T05-006 defaults, T05-007 `autoParam`, T05-015 postponement,
//! T05-016 the ellipsis) each need an ELABORATOR half as well, and this file deliberately asserts
//! nothing about that half. What it locks is the half that is entirely the parser's:
//!
//! * every new production produces the SHAPE the elaborator is promised, and
//! * every neighbouring thing the parser must still REFUSE is still refused.
//!
//! ★ WHY THE NEGATIVES ARE HALF THE FILE. A grammar addition is the classic way to quietly repair
//! malformed input. `..` at application position is two dots, and `Zero..` is a complete term plus
//! a sentence terminator plus one dot too many — `term_parser_total_lock` pins that it is REFUSED,
//! and an ellipsis that did not require whitespace would have turned that refusal into an
//! acceptance with nobody noticing. Likewise `(x : T)` is name-shaped-then-colon, and so is
//! `(fun x : Nat => x)`: without a keyword guard the new binder production eats every parenthesised
//! lambda in the repository. Both of those are in here as pairs, positive and negative.
//!
//! ★ AND WHY NO MARKER MAY REACH THE KERNEL. Four reserved marker globals carry information
//! `Term` has no field for. `no_reserved_marker_is_inhabited_so_a_forgotten_one_fails_loudly` is
//! the backstop: each marker is an UNDECLARED global, so an elaborator that forgets to strip or
//! expand one gets `UnboundVariable` from the kernel rather than a term that typechecks by
//! accident. `a_guillemet_name_can_no_longer_spell_a_reserved_marker` closes the other direction:
//! the verbatim `«…»` name form COULD spell `⟨anon⟩`, which the marker's own documentation said
//! was impossible.

use logicaffeine_kernel::interface::{parse_command, Repl, TermParser};
use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{
    double_check, infer_type, normalize, surface_elaborate, Context, DoubleCheck, KernelError,
    Literal, ParamKind, Term, Universe, DOT_MARKER,
};

fn g(n: &str) -> Term {
    Term::Global(n.to_string())
}
fn var(n: &str) -> Term {
    Term::Var(n.to_string())
}
fn app(f: Term, x: Term) -> Term {
    Term::App(Box::new(f), Box::new(x))
}
fn apps(f: Term, xs: &[Term]) -> Term {
    xs.iter().fold(f, |a, x| app(a, x.clone()))
}
fn pi(p: &str, t: Term, b: Term) -> Term {
    Term::Pi {
        param: p.to_string(),
        param_type: Box::new(t),
        body_type: Box::new(b),
    }
}
fn arrow(a: Term, b: Term) -> Term {
    pi("_", a, b)
}
fn lam(p: &str, t: Term, b: Term) -> Term {
    Term::Lambda {
        param: p.to_string(),
        param_type: Box::new(t),
        body: Box::new(b),
    }
}
fn ty0() -> Term {
    Term::Sort(Universe::Type(0))
}
fn int(n: i64) -> Term {
    Term::Lit(Literal::Int(n))
}
fn parse(src: &str) -> Term {
    TermParser::parse(src).unwrap_or_else(|e| panic!("`{src}` must parse, got {e:?}"))
}
fn std_ctx() -> Context {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    ctx
}
fn marker(name: &str) -> Term {
    g(name)
}
fn named(name: &str, value: Term) -> Term {
    apps(marker(TermParser::NAMED_ARG_MARKER), &[g(name), value])
}
fn ellipsis() -> Term {
    marker(TermParser::ELLIPSIS_MARKER)
}

// ===========================================================================
// NAMED ARGUMENTS · `f (x := e)`
// ===========================================================================

#[test]
fn a_named_argument_parses_into_the_reserved_marker_carrying_its_name() {
    // Before this production, `sub3 (y := 2)` died in `parse_parens` with
    // Expected{expected: "')'", found: ":"} — the name never reached anything.
    assert_eq!(
        parse("sub3 (y := 2)"),
        app(g("sub3"), named("y", int(2))),
        "a named argument is one argument of the spine, carrying the parameter NAME"
    );
    assert_eq!(
        parse("sub3 (y := 2) (x := 1) (z := 3)"),
        apps(
            g("sub3"),
            &[named("y", int(2)), named("x", int(1)), named("z", int(3))]
        ),
        "three named arguments are three arguments, in SOURCE order — reordering them into the \
         declaration's order is the elaborator's job, and it needs all three names to do it"
    );
    assert_eq!(
        parse("idf (A := Int) 5"),
        apps(g("idf"), &[named("A", g("Int")), int(5)]),
        "a named argument mixes with positional ones"
    );
}

#[test]
fn a_named_arguments_value_may_be_any_term_and_is_recognised_whole() {
    let t = parse("f (x := Succ Zero)");
    let arg = match &t {
        Term::App(_, a) => a.as_ref(),
        other => panic!("expected an application, got {other}"),
    };
    let (name, value) = TermParser::as_named_arg(arg).expect("the argument is a named argument");
    assert_eq!(name, "x");
    assert_eq!(
        *value,
        app(g("Succ"), g("Zero")),
        "the value is the WHOLE application"
    );

    // A lambda, an arrow and a nested named argument are all values.
    for src in [
        "f (k := fun x : Nat => x)",
        "f (k := Nat -> Nat)",
        "f (k := g (j := 1))",
    ] {
        let t = parse(src);
        let arg = match &t {
            Term::App(_, a) => a.as_ref().clone(),
            other => panic!("expected an application, got {other}"),
        };
        assert!(
            TermParser::as_named_arg(&arg).is_some(),
            "`{src}` is a named argument"
        );
    }
}

#[test]
fn as_named_arg_refuses_everything_that_is_not_one() {
    // The recognizer is the elaborator's only way in, so a false positive is a term silently
    // reinterpreted. Nothing else may match it.
    for src in [
        "Zero",
        "Succ Zero",
        "fun x : Nat => x",
        "Nat -> Nat",
        "(Succ Zero)",
    ] {
        assert!(
            TermParser::as_named_arg(&parse(src)).is_none(),
            "`{src}` is not a named argument"
        );
    }
    // Not even the marker at the wrong arity: one argument short, or one too many.
    assert!(TermParser::as_named_arg(&marker(TermParser::NAMED_ARG_MARKER)).is_none());
    assert!(
        TermParser::as_named_arg(&app(marker(TermParser::NAMED_ARG_MARKER), g("x"))).is_none(),
        "the bare marker applied to a name only is not a complete named argument"
    );
    // And a marker whose NAME slot is not a name at all.
    assert!(
        TermParser::as_named_arg(&apps(
            marker(TermParser::NAMED_ARG_MARKER),
            &[int(1), int(2)]
        ))
        .is_none(),
        "a parameter name is a Global, and a literal in that slot is not a named argument"
    );
}

#[test]
fn a_malformed_named_argument_is_refused_rather_than_read_as_something_else() {
    for src in [
        "sub3 (y :=)",
        "sub3 (y := 2",
        "sub3 ( := 2)",
        "sub3 (y := 2 3",
    ] {
        assert!(
            TermParser::parse(src).is_err(),
            "`{src}` is malformed and must be refused"
        );
    }
}

#[test]
fn an_ordinary_parenthesised_term_still_takes_the_path_it_always_took() {
    // ★ THE REWIND. Every one of these begins `( name …` and must NOT be read as a named argument
    // or a binder. The lookahead restores the position; if it did not, the whole repository's
    // parenthesised terms would change meaning.
    assert_eq!(parse("(Succ Zero)"), app(g("Succ"), g("Zero")));
    assert_eq!(parse("(Nat -> Nat)"), arrow(g("Nat"), g("Nat")));
    assert_eq!(parse("(fun x : Nat => x)"), lam("x", g("Nat"), var("x")));
    assert_eq!(parse("(forall x : Nat, Nat)"), pi("x", g("Nat"), g("Nat")));
    assert_eq!(
        parse("(let y := 0 in y)"),
        int(0),
        "a `let` is still substituted, not a binder"
    );
    assert_eq!(parse("(Zero)"), g("Zero"));
    assert_eq!(
        parse("(match Zero with | Zero => Zero end)")
            .to_string()
            .is_empty(),
        false
    );
}

// ===========================================================================
// EXPLICIT BINDERS · `(x : T) -> B`, and the DEFAULT / `by` forms
// ===========================================================================

#[test]
fn a_parenthesised_binder_finally_spells_the_dependent_arrow() {
    // `(A : Type) -> A` had no spelling at all: only `forall A : Type, A` did.
    assert_eq!(
        parse("(A : Type) -> A"),
        pi("A", ty0(), var("A")),
        "the body sees the binder"
    );
    assert_eq!(
        parse("(A : Type) -> A -> A"),
        pi("A", ty0(), arrow(var("A"), var("A"))),
        "and the arrow after it is still right-associative"
    );
    assert_eq!(
        parse("(A : Type) A"),
        pi("A", ty0(), var("A")),
        "the arrow between a binder and its body is optional, exactly as it is for `{{x : T}}`"
    );
    assert_eq!(
        parse("(x y : Nat) -> x"),
        pi("x", g("Nat"), pi("y", g("Nat"), var("x"))),
        "a group of names is one binder each, all at the shared type, outermost first"
    );
    assert_eq!(
        parse("(A : Type) -> (a : A) -> A"),
        pi("A", ty0(), pi("a", var("A"), var("A"))),
        "a later binder's TYPE sees an earlier binder"
    );
    assert_eq!(
        parse("Nat -> (m : Nat) -> Nat"),
        arrow(g("Nat"), pi("m", g("Nat"), g("Nat"))),
        "a named binder composes with the anonymous arrow form"
    );
}

#[test]
fn a_binder_body_that_is_missing_is_refused_and_a_keyword_is_never_a_binder_name() {
    // A binder needs a body — `(A : Type)` alone is not a term.
    assert!(
        TermParser::parse("(A : Type)").is_err(),
        "a binder with no body is incomplete"
    );
    assert!(TermParser::parse("Check (A : Type)").is_err());

    // ★ THE KEYWORD GUARD. Each of these begins with two name-shaped tokens and then a `:`, which
    // is exactly the binder production's trigger. Reading them as binders is the regression this
    // guard exists to prevent, and it would have hit every parenthesised lambda in the suite.
    assert!(
        matches!(parse("(fun x : Nat => x)"), Term::Lambda { .. }),
        "still a lambda"
    );
    assert!(matches!(parse("(forall x : Nat, Nat)"), Term::Pi { param, .. } if param == "x"));
    assert_eq!(
        parse("(let x := Zero in Succ x)"),
        app(g("Succ"), g("Zero"))
    );
    assert!(matches!(parse("(fix f => f)"), Term::Fix { .. }));
}

#[test]
fn a_defaulted_binder_carries_its_default_inside_its_own_param_type() {
    let t = parse("(m : Int := 3) -> Int");
    let Term::Pi {
        param,
        param_type,
        body_type,
    } = &t
    else {
        panic!("a defaulted binder is still an ordinary Pi, got {t}")
    };
    assert_eq!(
        param, "m",
        "the binder keeps its NAME — that is what a caller omits by name"
    );
    assert_eq!(**body_type, g("Int"));
    let (declared, default) =
        TermParser::as_default_param(param_type).expect("the param type carries a default");
    assert_eq!(
        *declared,
        g("Int"),
        "the declared type is recoverable unchanged"
    );
    assert_eq!(*default, int(3), "and so is the default value");

    // `Term::Pi` did NOT grow a field: strip the decoration and the type is the ordinary one.
    assert_eq!(
        TermParser::strip_binder_markers(&t),
        pi("m", g("Int"), g("Int")),
        "the registered type is the kernel type, with the default read off it"
    );
    assert_eq!(
        TermParser::strip_binder_markers(&parse("Int -> (m : Int := 3) -> Int")),
        arrow(g("Int"), pi("m", g("Int"), g("Int"))),
        "and in the middle of a telescope too"
    );
}

#[test]
fn a_dependent_default_refers_to_an_earlier_binder_as_a_bound_variable() {
    // ★ THE CAPTURE GUARD, at the parser end of it. A default that mentions an earlier NAMED
    // binder must come out as `Var`, not `Global`: a `Global("n")` here is an unbound global that
    // either fails in the kernel or, worse, resolves to some unrelated declaration called `n`.
    let t = parse("(n : Int) -> (m : Int := mul n 2) -> Int");
    let Term::Pi {
        param_type: inner, ..
    } = &t
    else {
        panic!("outer Pi")
    };
    assert_eq!(**inner, g("Int"));
    let Term::Pi {
        body_type: rest, ..
    } = &t
    else {
        panic!("outer Pi")
    };
    let Term::Pi {
        param_type: m_ty, ..
    } = rest.as_ref()
    else {
        panic!("inner Pi, got {rest}")
    };
    let (declared, default) = TermParser::as_default_param(m_ty).expect("m has a default");
    assert_eq!(*declared, g("Int"));
    assert_eq!(
        *default,
        apps(g("mul"), &[var("n"), int(2)]),
        "the default's reference to `n` is the BOUND variable n, not a global of that spelling"
    );
}

#[test]
fn an_auto_param_binder_records_the_tactic_and_is_distinguishable_from_a_default() {
    let t = parse("(h : P := by decide) -> Nat");
    let Term::Pi {
        param, param_type, ..
    } = &t
    else {
        panic!("still a Pi")
    };
    assert_eq!(param, "h");
    let (obligation, tactic) =
        TermParser::as_auto_param(param_type).expect("the param type carries a tactic");
    assert_eq!(
        *obligation,
        g("P"),
        "the obligation is the declared proposition"
    );
    assert_eq!(
        tactic, "decide",
        "and the tactic that must discharge it at each call site"
    );

    // ★ THE TWO MARKERS ARE NOT INTERCHANGEABLE. A default is a TERM inlined once per omission;
    // an auto-param is a PROOF searched afresh per call site, about that call site's proposition.
    // A recognizer that confused them would reuse one proof for every call.
    assert!(
        TermParser::as_default_param(param_type).is_none(),
        "an auto-param is not a default value"
    );
    let defaulted = parse("(m : Int := 3) -> Int");
    let Term::Pi { param_type: d, .. } = &defaulted else {
        panic!("Pi")
    };
    assert!(
        TermParser::as_auto_param(d).is_none(),
        "a default value is not an auto-param"
    );

    assert_eq!(
        TermParser::strip_binder_markers(&t),
        pi("h", g("P"), g("Nat")),
        "stripping leaves the obligation as an ordinary explicit parameter"
    );
}

#[test]
fn a_defaulted_binder_may_be_the_argument_of_a_dependent_type_and_still_strips() {
    // The marker can sit anywhere a type can, so stripping is a WALK, not a peek at the spine.
    let t = parse("(f : (m : Nat := 3) -> Nat) -> Nat");
    assert_eq!(
        TermParser::strip_binder_markers(&t),
        pi("f", pi("m", g("Nat"), g("Nat")), g("Nat")),
        "a default nested inside another binder's type is stripped too"
    );
    let stripped = TermParser::strip_binder_markers(&t);
    assert_eq!(
        TermParser::strip_binder_markers(&stripped),
        stripped,
        "stripping is idempotent"
    );
    for src in [
        "Nat",
        "Nat -> Nat",
        "fun x : Nat => x",
        "forall x : Nat, Nat",
        "⟨Zero, Zero⟩",
    ] {
        let t = parse(src);
        assert_eq!(
            TermParser::strip_binder_markers(&t),
            t,
            "`{src}` has no binder markers and must come back untouched"
        );
    }
}

#[test]
fn a_malformed_binder_default_is_refused() {
    for src in [
        "(m : Int := ) -> Int",
        "(m : Int := 3 -> Int",
        "(m := 3 : Int) -> Int",
        "(m : Int := by) -> Int",
    ] {
        assert!(
            TermParser::parse(src).is_err(),
            "`{src}` is malformed and must be refused"
        );
    }
}

// ===========================================================================
// THE ELLIPSIS · `⟨a, ..⟩` and `f ..`
// ===========================================================================

#[test]
fn an_ellipsis_is_the_last_component_of_an_anonymous_constructor() {
    let anon = g("⟨anon⟩");
    assert_eq!(
        parse("⟨Zero, Zero, Zero, ..⟩"),
        apps(anon.clone(), &[g("Zero"), g("Zero"), g("Zero"), ellipsis()]),
        "the components are followed by the ellipsis request"
    );
    assert_eq!(
        parse("⟨Zero, ..⟩"),
        apps(anon.clone(), &[g("Zero"), ellipsis()]),
        "one component and the rest"
    );
    assert_eq!(
        parse("⟨..⟩"),
        app(anon.clone(), ellipsis()),
        "all of the fields, none written"
    );
    assert_eq!(
        parse("⟨Zero,..⟩"),
        apps(anon, &[g("Zero"), ellipsis()]),
        "the comma need not be followed by a space"
    );
}

#[test]
fn an_ellipsis_is_the_last_argument_of_an_application() {
    assert_eq!(
        parse("TNil .."),
        app(g("TNil"), ellipsis()),
        "`f ..` asks for the rest"
    );
    assert_eq!(
        parse("TCons Nat Zero .."),
        apps(g("TCons"), &[g("Nat"), g("Zero"), ellipsis()]),
        "after the arguments that were written"
    );
    assert!(TermParser::is_ellipsis(&ellipsis()));
    for src in ["Zero", "Nat -> Nat", "⟨Zero⟩"] {
        assert!(
            !TermParser::is_ellipsis(&parse(src)),
            "`{src}` is not an ellipsis"
        );
    }
}

#[test]
fn the_ellipsis_does_not_weaken_the_sentence_terminator_or_the_projection() {
    // ★ THE PAIR THAT MATTERS. `..` at application position needs WHITESPACE before it, because
    // `Zero..` is a term, a terminator, and one dot too many — and
    // `term_parser_total_lock::one_trailing_full_stop_is_a_terminator_and_not_input` pins that it
    // is refused. A `..` that did not require the space would have turned that refusal into an
    // ellipsis and nothing in the suite would have said so.
    assert!(
        TermParser::parse("Zero..").is_err(),
        "`Zero..` is still refused"
    );
    assert!(
        TermParser::parse("Zero...").is_err(),
        "and three dots are still trailing input"
    );
    assert!(
        TermParser::parse("Zero ...").is_err(),
        "three dots are not an ellipsis either"
    );
    assert!(
        TermParser::parse("Zero.").is_ok(),
        "one trailing full stop is still a terminator"
    );
    assert!(TermParser::parse("Nat -> Nat.").is_ok());

    // A single dot is still a tight field projection.
    let dotted = parse("p.fst");
    assert_eq!(
        dotted,
        apps(g("⟨proj⟩"), &[g("p"), g("fst")]),
        "`p.fst` is still a projection, not an ellipsis"
    );

    // Nothing may FOLLOW an ellipsis in a spine — "the rest" has no rest.
    assert!(
        TermParser::parse("TNil .. Zero").is_err(),
        "an argument after `..` is refused"
    );
    assert!(
        TermParser::parse("⟨Zero, .., Zero⟩").is_err(),
        "a component after `..` is refused"
    );
}

// ===========================================================================
// POSTPONED BINDER TYPES · `fun x => e`, `forall x, e`
// ===========================================================================

#[test]
fn a_lambda_may_postpone_its_parameter_type() {
    // `fun x => Succ x` died with Expected{expected: "':'", found: "="}: the annotation was
    // mandatory, so the one term Lean's elaborator POSTPONES had no spelling.
    assert_eq!(
        parse("fun x => Succ x"),
        lam("x", Term::Hole, app(g("Succ"), var("x"))),
        "the parameter type is a Hole for the elaborator to fill from the expected type"
    );
    assert_eq!(
        parse("fun x : Nat => Succ x"),
        lam("x", g("Nat"), app(g("Succ"), var("x"))),
        "and the annotated form is unchanged"
    );
    assert_eq!(
        parse("fun x => fun y => x"),
        lam("x", Term::Hole, lam("y", Term::Hole, var("x"))),
        "nested, and the inner body still sees the outer binder"
    );
}

#[test]
fn a_lambda_takes_a_telescope_of_binders_the_way_lean_does() {
    assert_eq!(
        parse("fun x y => x"),
        lam("x", Term::Hole, lam("y", Term::Hole, var("x"))),
        "two bare binders"
    );
    assert_eq!(
        parse("fun x y : Nat => x"),
        lam("x", g("Nat"), lam("y", g("Nat"), var("x"))),
        "a trailing annotation types EVERY bare binder, as Lean's optType does"
    );
    assert_eq!(
        parse("fun (x : Nat) (y : Bool) => x"),
        lam("x", g("Nat"), lam("y", g("Bool"), var("x"))),
        "parenthesised groups each carry their own type"
    );
    assert_eq!(
        parse("fun (x y : Nat) => y"),
        lam("x", g("Nat"), lam("y", g("Nat"), var("y"))),
        "a group of names shares one type"
    );
    assert_eq!(
        parse("fun (A : Type) (a : A) => a"),
        lam("A", ty0(), lam("a", var("A"), var("a"))),
        "a later group's TYPE sees an earlier binder"
    );
    assert_eq!(
        parse("fun (x : Nat) y : Bool => x"),
        lam("x", g("Nat"), lam("y", g("Bool"), var("x"))),
        "an explicit group and a bare binder mix, the trailing annotation taking the bare one"
    );
}

#[test]
fn a_forall_takes_the_same_telescope_and_may_postpone_too() {
    assert_eq!(
        parse("forall x : Nat, Nat"),
        pi("x", g("Nat"), g("Nat")),
        "unchanged"
    );
    assert_eq!(
        parse("forall x, x"),
        pi("x", Term::Hole, var("x")),
        "postponed"
    );
    assert_eq!(
        parse("forall x y : Nat, x"),
        pi("x", g("Nat"), pi("y", g("Nat"), var("x"))),
        "shared annotation"
    );
    assert_eq!(
        parse("forall (A : Type) (a : A), A"),
        pi("A", ty0(), pi("a", var("A"), var("A"))),
        "groups"
    );
    assert_eq!(
        parse("forall f : forall y : Nat, Nat, Nat"),
        pi("f", pi("y", g("Nat"), g("Nat")), g("Nat")),
        "a nested forall inside the annotation still ends at ITS own comma"
    );
}

#[test]
fn an_abstraction_with_no_binder_or_no_separator_is_refused() {
    for src in [
        "fun => Zero",
        "fun : Nat => Zero",
        "fun x Zero",
        "fun x : Nat Zero",
        "forall x Nat",
        "forall , Nat",
        "fun (x : Nat) => ",
        "fun (: Nat) => Zero",
        "fun ( ) => Zero",
    ] {
        assert!(TermParser::parse(src).is_err(), "`{src}` must be refused");
    }
}

#[test]
fn a_postponed_binder_type_is_never_mistaken_for_a_checkable_one() {
    // ★ THE SOUNDNESS-ADJACENT HALF. `types_equal` matches a `Term::Hole` against anything, so a
    // Hole that survived into a declaration would check against every type. It cannot: the kernel
    // refuses to infer one at all, which is what makes leaving the type to the elaborator safe.
    let ctx = std_ctx();
    let unannotated = parse("fun x => Succ x");
    assert!(
        matches!(
            infer_type(&ctx, &unannotated),
            Err(KernelError::CannotInferHole)
        ),
        "an un-elaborated postponed lambda must fail LOUDLY, got {:?}",
        infer_type(&ctx, &unannotated)
    );
    assert!(
        infer_type(&ctx, &parse("forall x, x")).is_err(),
        "and so must a postponed forall"
    );
    // The annotated neighbour still checks, so the refusal is about the hole and nothing else.
    assert!(infer_type(&ctx, &parse("fun x : Nat => Succ x")).is_ok());
}

// ===========================================================================
// IMPLICIT BINDER GROUPS · `{A B : Type}`
// ===========================================================================

#[test]
fn a_braced_group_of_names_counts_every_name_as_an_implicit_argument() {
    let (t, implicits) =
        TermParser::parse_with_implicits("{A B : Type} -> A -> B").expect("parses");
    assert_eq!(
        t,
        pi("A", ty0(), pi("B", ty0(), arrow(var("A"), var("B")))),
        "two binders, outermost first, both at the shared type"
    );
    assert_eq!(
        implicits, 2,
        "★ the count is per NAME, not per group — an application inserts one metavariable per \
         implicit parameter, and counting groups would insert too few"
    );
    let (_, one) = TermParser::parse_with_implicits("{A : Type} -> A -> A").expect("parses");
    assert_eq!(one, 1, "the single-name form is unchanged");
    let (_, chained) =
        TermParser::parse_with_implicits("{A : Type} -> {B : Type} -> A").expect("parses");
    assert_eq!(chained, 2, "and two separate groups still total two");
    assert!(
        TermParser::parse("{ : Type} -> Nat").is_err(),
        "a braced group needs a name"
    );
}

// ===========================================================================
// THE RESERVED MARKERS THEMSELVES
// ===========================================================================

#[test]
fn no_reserved_marker_is_inhabited_so_a_forgotten_one_fails_loudly() {
    // ★ THE BACKSTOP. Every marker is an UNDECLARED global in every context, including the full
    // standard library. An elaborator that forgets to strip or expand one therefore hands the
    // kernel an unbound variable and is refused — it cannot hand it a term that typechecks by
    // accident. This is the property that makes carrying surface information as a marker safe.
    let ctx = std_ctx();
    for m in [
        TermParser::NAMED_ARG_MARKER,
        TermParser::PARAM_DEFAULT_MARKER,
        TermParser::AUTO_PARAM_MARKER,
        TermParser::ELLIPSIS_MARKER,
        "⟨anon⟩",
        "⟨proj⟩",
    ] {
        assert!(
            ctx.get_global(m).is_none(),
            "`{m}` must not be a declared global"
        );
        assert!(
            matches!(
                infer_type(&ctx, &g(m)),
                Err(KernelError::UnboundVariable(_))
            ),
            "`{m}` reaching the kernel must be refused as unbound, got {:?}",
            infer_type(&ctx, &g(m))
        );
    }
}

#[test]
fn the_reserved_markers_are_distinct_and_unspellable() {
    let all = [
        TermParser::NAMED_ARG_MARKER,
        TermParser::PARAM_DEFAULT_MARKER,
        TermParser::AUTO_PARAM_MARKER,
        TermParser::ELLIPSIS_MARKER,
    ];
    for (i, a) in all.iter().enumerate() {
        for b in &all[i + 1..] {
            assert_ne!(a, b, "two markers sharing a spelling are one marker");
        }
        assert!(
            a.contains('⟨') && a.contains('⟩'),
            "`{a}` must carry the bracket characters no identifier may contain"
        );
        // ★ NO IDENTIFIER CAN DENOTE IT. Read as source, a marker's spelling is either refused
        // or read as some other production (`⟨named⟩` is an anonymous constructor of one
        // component) — what it can never be is the marker GLOBAL itself, in either name form.
        assert_ne!(
            TermParser::parse(a).ok(),
            Some(g(a)),
            "`{a}` must never parse into the marker global"
        );
        assert!(
            TermParser::parse(&format!("«{a}»")).is_err(),
            "`«{a}»` must not parse as a guillemet name either"
        );
    }
}

#[test]
fn a_guillemet_name_can_no_longer_spell_a_reserved_marker() {
    // ★ THE HOLE THIS CLOSES. The markers are documented as collision-proof because "the
    // identifier lexer cannot produce ⟨/⟩" — but the VERBATIM guillemet form could, so
    // `Definition «⟨anon⟩» : Nat := Zero.` declared a global whose every later mention was read
    // as an anonymous constructor instead: a write-only name again, by a different route.
    for m in [
        "⟨anon⟩",
        "⟨proj⟩",
        TermParser::NAMED_ARG_MARKER,
        TermParser::ELLIPSIS_MARKER,
    ] {
        assert!(
            parse_command(&format!("Definition «{m}» : Nat := Zero.")).is_err(),
            "the declaration door must refuse a name that spells the reserved marker `{m}`"
        );
        assert!(
            TermParser::parse(&format!("«{m}»")).is_err(),
            "and so must the term door: `«{m}»`"
        );
    }

    // ★ AND THE REFUSAL MUST NOT OVER-REACH. A guillemet name is how this language spells a name
    // with a space or punctuation in it, and that must still work at BOTH doors — the invariant
    // `scan_name` exists to enforce. (Both doors here are the PARSER's doors: the `Repl` currently
    // refuses every guillemet declaration for an unrelated reason of its own, reported separately.)
    assert_eq!(
        TermParser::parse("«a name»").expect("an ordinary guillemet name"),
        g("a name"),
        "a name with a space is still one name"
    );
    assert!(
        parse_command("Definition «a name» : Nat := Zero.").is_ok(),
        "and at the other door"
    );
    assert!(TermParser::parse("«h'»").is_ok(), "primes are still fine");
    assert!(
        TermParser::parse("«x + y»").is_ok(),
        "punctuation is still fine"
    );
    assert!(
        TermParser::parse("«a.b»").is_ok(),
        "and a dot inside one is still one component"
    );
}

// ===========================================================================
// SUBSTITUTING A DEFAULT'S PARAMETER REFERENCES
// ===========================================================================

#[test]
fn substituting_a_parameter_reference_reaches_both_representations() {
    // A default parsed under a NAMED earlier binder holds a `Var`; one parsed under an ANONYMOUS
    // earlier binder (`Int -> (m : Int := mul n 2) -> Int`, whose position 0 is `"_"`) holds a
    // `Global`. An inliner that handled only one of them would emit an OPEN term with a free `n`.
    let by_var = apps(g("mul"), &[var("n"), int(2)]);
    let by_global = apps(g("mul"), &[g("n"), int(2)]);
    let five = int(5);
    let want = apps(g("mul"), &[int(5), int(2)]);
    assert_eq!(
        TermParser::substitute_param_ref(&by_var, "n", &five),
        want,
        "Var form"
    );
    assert_eq!(
        TermParser::substitute_param_ref(&by_global, "n", &five),
        want,
        "Global form"
    );
}

#[test]
fn substituting_a_parameter_reference_does_not_capture() {
    // A binder of the same name SHADOWS the parameter, and the local variable under it is a
    // different variable that must survive untouched.
    let shadowed = lam("n", g("Int"), var("n"));
    assert_eq!(
        TermParser::substitute_param_ref(&shadowed, "n", &int(5)),
        shadowed,
        "λn. n is the identity on Int, whatever the parameter n happens to be"
    );
    // The free occurrence outside the shadowing binder is still replaced.
    let mixed = apps(g("add"), &[var("n"), lam("n", g("Int"), var("n"))]);
    assert_eq!(
        TermParser::substitute_param_ref(&mixed, "n", &int(5)),
        apps(g("add"), &[int(5), lam("n", g("Int"), var("n"))]),
        "outside the binder it is the parameter; inside it is the binder's own variable"
    );
    // A `forall`, a `let` and a `fix` shadow just as a `λ` does.
    for shadowing in [
        pi("n", g("Int"), var("n")),
        Term::Let {
            name: "n".to_string(),
            ty: Box::new(g("Int")),
            value: Box::new(int(1)),
            body: Box::new(var("n")),
        },
        Term::Fix {
            name: "n".to_string(),
            body: Box::new(var("n")),
        },
    ] {
        assert_eq!(
            TermParser::substitute_param_ref(&shadowing, "n", &int(5)),
            shadowing,
            "every binder shadows: {shadowing}"
        );
    }
    // A name that does not occur leaves the term alone.
    let untouched = apps(g("mul"), &[var("k"), int(2)]);
    assert_eq!(
        TermParser::substitute_param_ref(&untouched, "n", &int(5)),
        untouched
    );
}

#[test]
fn a_default_parsed_under_a_named_binder_inlines_to_a_closed_term() {
    // End to end at the parser's end of it: read the default off the binder, substitute the
    // actual argument, and the result mentions nothing that is not closed.
    let t = parse("(n : Int) -> (m : Int := mul n 2) -> Int");
    let Term::Pi {
        body_type: rest, ..
    } = &t
    else {
        panic!("outer Pi")
    };
    let Term::Pi {
        param_type: m_ty, ..
    } = rest.as_ref()
    else {
        panic!("inner Pi")
    };
    let (_, default) = TermParser::as_default_param(m_ty).expect("m has a default");
    let inlined = TermParser::substitute_param_ref(default, "n", &int(5));
    assert_eq!(
        inlined,
        apps(g("mul"), &[int(5), int(2)]),
        "mul 5 2, not an open `mul n 2`"
    );
    let ctx = std_ctx();
    assert!(
        infer_type(&ctx, &inlined).is_ok(),
        "and the inlined default is a closed term the kernel accepts: {:?}",
        infer_type(&ctx, &inlined)
    );
}

// ===========================================================================
// THE WHOLE-INPUT GUARANTEE, UNDER EVERY NEW PRODUCTION
// ===========================================================================

#[test]
fn every_new_production_still_consumes_its_whole_input_or_refuses() {
    // The completeness guard is the reason this parser is safe for a proof kernel, and a new
    // production is exactly how one loses it: a form that stops early hands the kernel a PREFIX
    // of what was written.
    for src in [
        "sub3 (y := 2) garbage )",
        "(A : Type) -> A }{",
        "(m : Int := 3) -> Int ,",
        "fun x => Succ x )",
        "forall x, x |",
        "TNil .. ..",
        "⟨Zero, ..⟩ )",
    ] {
        assert!(
            TermParser::parse(src).is_err(),
            "`{src}` leaves input a term cannot absorb and must be refused, not truncated"
        );
    }
    // And the whole-input forms still parse.
    for src in [
        "sub3 (y := 2) (x := 1)",
        "(A : Type) -> A -> A",
        "(m : Int := 3) -> Int",
        "(h : P := by decide) -> Nat",
        "fun x => Succ x",
        "fun x y : Nat => x",
        "forall x, x",
        "TNil ..",
        "⟨Zero, Zero, ..⟩",
        "{A B : Type} -> A -> B",
    ] {
        assert!(
            TermParser::parse(src).is_ok(),
            "`{src}` is a whole term: {:?}",
            TermParser::parse(src)
        );
    }
}

// ===========================================================================
// SPLITTING A SENTENCE AT THE RIGHT `:=`
// ===========================================================================

#[test]
fn the_assignment_separator_is_found_at_top_level_and_not_inside_a_type() {
    // ★ THE DEFECT THIS EXISTS FOR. `Definition <name> : <type> := <body>.` is split on the FIRST
    // `:=` in the text. A defaulted binder puts one inside the TYPE and a guillemet name puts one
    // inside the NAME, and in both cases the sentence splits in the wrong place and is refused for
    // a reason that names neither. A top-level scan is the only correct split.
    let with_default = "withDefault : Int -> (m : Int := 3) -> Int := fun n : Int => n";
    assert_eq!(
        TermParser::find_top_level(with_default, ":="),
        with_default.rfind(":="),
        "the separator is the LAST `:=` here — the one inside the defaulted binder is not it"
    );
    assert_ne!(
        TermParser::find_top_level(with_default, ":="),
        with_default.find(":="),
        "and it is emphatically not the first one, which is inside the parentheses"
    );

    let quoted_name = "«a := b» : Nat := Zero";
    assert_eq!(
        TermParser::find_top_level(quoted_name, ":="),
        quoted_name.rfind(":="),
        "a `:=` inside a guillemet name is part of the NAME, not a separator"
    );
    assert_eq!(
        TermParser::find_top_level("«a:b» : Nat", ":"),
        Some("«a:b» ".len()),
        "and a `:` inside a guillemet name does not open the type ascription"
    );

    // The ordinary sentences are unchanged — a top-level scan must agree with `find` whenever
    // `find` was right, or every existing declaration changes meaning.
    for src in [
        "f : Nat := Zero",
        "f := Zero",
        "nil : {A : Type} -> TList A := fun A : Type => TNil A",
        "h' : Nat := (Succ Zero)",
    ] {
        assert_eq!(
            TermParser::find_top_level(src, ":="),
            src.find(":="),
            "`{src}` has its separator at top level already"
        );
    }
}

#[test]
fn a_top_level_scan_sees_through_every_bracket_a_string_and_an_unbalanced_closer() {
    assert_eq!(
        TermParser::find_top_level("(a := b) := c", ":="),
        Some(9),
        "parentheses"
    );
    assert_eq!(
        TermParser::find_top_level("{a := b} := c", ":="),
        Some(9),
        "braces"
    );
    assert_eq!(
        TermParser::find_top_level("⟨a := b⟩ := c", ":="),
        Some("⟨a := b⟩ ".len()),
        "⟨⟩"
    );
    assert_eq!(
        TermParser::find_top_level("\"a := b\" := c", ":="),
        Some(9),
        "a string literal is opaque"
    );
    assert_eq!(
        TermParser::find_top_level("\"a \\\" := b\" := c", ":="),
        Some(12),
        "and its escapes are honoured, so an escaped quote does not end it early"
    );
    assert_eq!(
        TermParser::find_top_level("((a := b)) := c", ":="),
        Some(11),
        "nesting is counted, not merely detected"
    );
    assert_eq!(
        TermParser::find_top_level(") := c", ":="),
        Some(2),
        "an unbalanced closer must not take the depth negative and hide the separator"
    );
    assert_eq!(
        TermParser::find_top_level("(a := b", ":="),
        None,
        "an unclosed bracket hides it"
    );
    assert_eq!(
        TermParser::find_top_level("f : Nat", ":="),
        None,
        "absent is None"
    );
    assert_eq!(TermParser::find_top_level("", ":="), None);
    assert_eq!(
        TermParser::find_top_level("f := g", ""),
        None,
        "an empty separator matches nothing"
    );
    // The index is a BYTE index into the original text, usable for slicing directly.
    let src = "α : Nat := Zero";
    let at = TermParser::find_top_level(src, ":=").expect("found");
    assert_eq!(
        &src[..at],
        "α : Nat ",
        "the index slices a multi-byte prefix correctly"
    );
    assert_eq!(&src[at + 2..], " Zero");
}

// ===========================================================================
// NUMERALS · arbitrary precision, and the prefix that used to vanish
// ===========================================================================

#[test]
fn a_numeral_the_kernel_can_hold_is_a_numeral_the_surface_can_spell() {
    // `Literal::BigInt` is the kernel's canonical arbitrary-precision integer: it holds exactly
    // the values `i64` cannot, `add` promotes into it, and `defeq` compares it exactly. The parser
    // read numerals with `str::parse::<i64>`, so a value the kernel computes with had no spelling.
    let wide = |d: &str| {
        Term::Lit(logicaffeine_kernel::int_lit(
            logicaffeine_kernel::BigInt::parse_decimal(d).expect("decimal"),
        ))
    };
    for d in [
        "9223372036854775808",
        "18446744073709551616",
        "123456789012345678901234567890",
    ] {
        assert_eq!(parse(d), wide(d), "`{d}` must parse at full precision");
    }
    for d in ["-9223372036854775809", "-123456789012345678901234567890"] {
        assert_eq!(parse(d), wide(d), "`{d}` must parse at full precision");
    }

    // ★ CANONICALITY. Everything that fits a machine word must still be `Literal::Int`, or one
    // value has two spellings and `==` — which is what `defeq` uses on literals — stops holding.
    for d in [
        "0",
        "1",
        "5",
        "9223372036854775807",
        "-1",
        "-9223372036854775808",
    ] {
        let t = parse(d);
        assert!(
            matches!(t, Term::Lit(Literal::Int(_))),
            "`{d}` fits a machine word and must stay in the machine form, got {t}"
        );
    }
    assert_eq!(
        parse("9223372036854775807"),
        Term::Lit(Literal::Int(i64::MAX))
    );
    assert_eq!(
        parse("-9223372036854775808"),
        Term::Lit(Literal::Int(i64::MIN)),
        "i64::MIN had NO spelling before: its digits do not fit i64 as a positive"
    );
    // One rung either side of the boundary, both signs.
    assert!(matches!(
        parse("9223372036854775806"),
        Term::Lit(Literal::Int(_))
    ));
    assert!(!matches!(
        parse("9223372036854775808"),
        Term::Lit(Literal::Int(_))
    ));
    assert!(matches!(
        parse("-9223372036854775807"),
        Term::Lit(Literal::Int(_))
    ));
    assert!(!matches!(
        parse("-9223372036854775809"),
        Term::Lit(Literal::Int(_))
    ));
}

#[test]
fn a_numeral_that_does_not_fit_can_never_vanish_and_leave_its_neighbour_behind() {
    // ★ THE SILENT PREFIX. `try_parse_negative_number` consumed the sign and the digits and THEN
    // returned `None` when the value overflowed, leaving the position past a literal nobody read.
    // `-9223372036854775809 Nat` therefore parsed as `Nat` — a whole term, so the completeness
    // guard was satisfied — with the numeral gone. That is the one failure a proof kernel's parser
    // must not have, arriving by the one path that discarded input instead of leaving it behind.
    let t = parse("-9223372036854775809 Nat");
    assert_ne!(
        t,
        g("Nat"),
        "the numeral must not vanish and leave `Nat` as the whole term"
    );
    assert!(
        matches!(t, Term::App(..)),
        "it is an application of the numeral to `Nat` — ill-typed, and that is the kernel's to \
         say — not a silently truncated prefix: {t}"
    );

    // A `-` that is NOT a numeral's sign still leaves the position alone for the other productions.
    assert_eq!(
        TermParser::parse("Zero -").map_err(|_| ()),
        Err(()),
        "a dangling `-` is still refused rather than read as something"
    );
}

#[test]
fn a_universe_level_that_does_not_fit_is_refused_and_never_read_as_level_zero() {
    // ★ THE SILENT LEVEL. `try_parse_number` consumed the digits and returned `None` on overflow,
    // and the caller reads `None` as "no level was written" — so `Type 99999999999999999999` was
    // `Type 0`, with the digits gone and the completeness guard satisfied. The kernel then
    // certified a claim at a universe nobody wrote.
    assert_eq!(parse("Type"), ty0(), "a bare `Type` is still Type 0");
    assert_eq!(parse("Type 0"), ty0());
    assert_eq!(parse("Type 3"), Term::Sort(Universe::Type(3)));
    assert_eq!(
        parse("Type4294967295"),
        Term::Sort(Universe::Type(u32::MAX)),
        "the top rung"
    );
    for src in [
        "Type 4294967296",
        "Type 99999999999999999999",
        "Type 18446744073709551616",
    ] {
        let got = TermParser::parse(src);
        assert!(
            got.is_err(),
            "`{src}` names a level `Universe::Type` cannot hold and must be REFUSED, not \
             silently read as Type 0 — got {got:?}"
        );
    }
    assert_ne!(
        TermParser::parse("Type 99999999999999999999").ok(),
        Some(ty0()),
        "and above all it must never come back as Type 0"
    );
}

// ===========================================================================
// THE BINDER-KIND TELESCOPE · a position, not a prefix count
// ===========================================================================

#[test]
fn a_nested_implicit_binder_is_not_a_leading_one() {
    // ★ A DEFECT THAT IS NOT IN THE CORPUS. `parse_with_implicits` reports how many LEADING
    // implicit parameters a declaration has, and the elaborator's fallback model turns that
    // number into exactly that many metavariables inserted at the FRONT of every application.
    // The count was a running total incremented wherever a `{…}` group was read — including one
    // read inside a PARAMETER'S OWN TYPE, which is not a parameter of this declaration at all.
    //
    // `{A : Type} -> ({B : Type} -> B) -> A` has ONE implicit parameter and one explicit one
    // whose type happens to be polymorphic. Counted the old way it reported TWO, so `f g` would
    // elaborate as `f ?0 ?1 g` — one metavariable too many, inserted into a position the
    // declaration does not have, and the `?1` that survives is an unbound variable the kernel
    // rejects far from the application that invented it.
    let (t, implicits) = TermParser::parse_with_implicits("{A : Type} -> ({B : Type} -> B) -> A")
        .expect("a polymorphic parameter type is expressible");
    assert_eq!(
        t,
        pi("A", ty0(), arrow(pi("B", ty0(), var("B")), var("A"))),
        "the term itself was always right — it is the COUNT that was wrong"
    );
    assert_eq!(
        implicits, 1,
        "★ one leading implicit parameter. A `{{…}}` inside a parameter's TYPE binds nothing of \
         this declaration and must not be counted as one of its arguments"
    );

    // The boundary on each side: nothing nested, and nested twice.
    let (_, flat) = TermParser::parse_with_implicits("{A : Type} -> A -> A").expect("parses");
    assert_eq!(flat, 1, "one leading implicit, nothing nested");
    let (_, twice) =
        TermParser::parse_with_implicits("{A : Type} -> ({B C : Type} -> B) -> A").expect("parses");
    assert_eq!(
        twice, 1,
        "a two-name nested group is still zero parameters of THIS declaration"
    );
}

#[test]
fn a_binder_group_may_stand_anywhere_in_the_telescope_and_not_only_at_its_head() {
    // ★ `parse_arrow` read its right-hand side with `parse_arrow`, not `parse_term`. Everything
    // `parse_term` dispatches on — an implicit binder group, a `forall`, a `fun`, a `let`, a
    // `match`, a `fix` — was therefore unreachable after the first `->`, so a telescope could
    // only ever open with one. Lean reads `Nat → {α : Type} → α → α` and `Nat → ∀ x, P x`; this
    // is the same grammar, and the only reason it was not is that the recursion skipped a level.
    assert_eq!(
        parse("Nat -> {A : Type} -> A -> A"),
        arrow(g("Nat"), pi("A", ty0(), arrow(var("A"), var("A")))),
        "an implicit binder after an arrow"
    );
    assert_eq!(
        parse("Nat -> forall x : Nat, Eq Nat x x"),
        arrow(
            g("Nat"),
            pi(
                "x",
                g("Nat"),
                apps(g("Eq"), &[g("Nat"), var("x"), var("x")])
            )
        ),
        "a `forall` after an arrow — the body extends maximally, exactly as it does at the head"
    );
    assert_eq!(
        parse("Nat -> let y := Nat in y"),
        arrow(g("Nat"), g("Nat")),
        "a `let` after an arrow is still substituted away"
    );
    assert_eq!(
        parse("Nat -> fun x : Type => x"),
        arrow(g("Nat"), lam("x", ty0(), var("x"))),
        "a type-level lambda after an arrow"
    );

    // The boundary: the arrow is still RIGHT-ASSOCIATIVE and the plain chain is untouched.
    assert_eq!(
        parse("Nat -> Nat -> Nat"),
        arrow(g("Nat"), arrow(g("Nat"), g("Nat"))),
        "an ordinary chain parses exactly as it always did"
    );
    // And the whole-input guard still fires on the new right-hand sides. (`A garbage` is
    // deliberately NOT in this list: juxtaposition is application, so that one is a complete
    // term and refusing it would be the bug.)
    for src in [
        "Nat -> {A : Type}",
        "Nat -> forall",
        "Nat -> {A : Type} -> A )",
        "Nat -> ->",
    ] {
        assert!(
            TermParser::parse(src).is_err(),
            "`{src}` is incomplete and must be refused"
        );
    }
    assert_eq!(
        parse("Nat -> {A : Type} -> A garbage"),
        arrow(g("Nat"), pi("A", ty0(), app(var("A"), g("garbage")))),
        "juxtaposition after a binder group is APPLICATION, not trailing input"
    );
}

// ===========================================================================
// INSTANCE BINDERS · `[C x] -> B` — ParamKind::Instance, finally spellable
// ===========================================================================

#[test]
fn an_instance_binder_is_spellable_and_binds_like_every_other_one() {
    // ★ `ParamKind::Instance` existed in the elaborator and NO SOURCE TEXT COULD PRODUCE ONE.
    // `Context::set_binder_kinds` is called by tests and by nothing else in the whole tree, so
    // an instance parameter could only ever be installed by a Rust caller reaching in — the
    // typeclass surface was unreachable from the language it belongs to.
    assert_eq!(
        parse("[Inhabited Nat] -> Nat"),
        pi("_", app(g("Inhabited"), g("Nat")), g("Nat")),
        "an anonymous instance binder is an ordinary Pi over the class type"
    );
    assert_eq!(
        parse("{A : Type} -> [Inhabited A] -> A"),
        pi("A", ty0(), pi("_", app(g("Inhabited"), var("A")), var("A"))),
        "the class type sees the binders before it"
    );
    // A NAMED instance binder — Lean's `[inst : Inhabited α]` — is in scope in the body.
    assert_eq!(
        parse("[i : Inhabited Nat] -> i"),
        pi("i", app(g("Inhabited"), g("Nat")), var("i")),
        "a named instance binder binds its name in the body"
    );
    // The arrow between the group and the body is optional, as it is for `{…}` and `(…)`.
    assert_eq!(
        parse("[Inhabited Nat] Nat"),
        parse("[Inhabited Nat] -> Nat")
    );
}

#[test]
fn a_malformed_instance_binder_is_refused_and_never_read_as_something_else() {
    for src in [
        "[] -> Nat",
        "[Inhabited Nat -> Nat",
        "[i :] -> Nat",
        "[: Nat] -> Nat",
        "[Inhabited Nat]",
    ] {
        assert!(TermParser::parse(src).is_err(), "`{src}` must be refused");
    }
}

// ===========================================================================
// STRICT-IMPLICIT BINDERS · `⦃x : T⦄` — Lean's third binder kind
// ===========================================================================

#[test]
fn a_strict_implicit_binder_is_spellable_and_shaped_like_the_eager_one() {
    assert_eq!(
        parse("⦃A : Type⦄ -> A -> A"),
        pi("A", ty0(), arrow(var("A"), var("A"))),
        "a strict implicit is an ordinary Pi — what differs is WHEN it is inserted"
    );
    assert_eq!(
        parse("⦃A B : Type⦄ -> A"),
        pi("A", ty0(), pi("B", ty0(), var("A"))),
        "a multi-name strict group is one binder per name, like `{{A B : Type}}`"
    );
    assert_eq!(
        parse("⦃A : Type⦄ A"),
        parse("⦃A : Type⦄ -> A"),
        "the arrow is optional"
    );
    for src in ["⦃⦄ -> Nat", "⦃A : Type⦄", "⦃A : Type -> Nat", "⦃A⦄ -> A"] {
        assert!(TermParser::parse(src).is_err(), "`{src}` must be refused");
    }
}

// ===========================================================================
// A BINDER KIND IS NEVER VISIBLE IN THE EMITTED TERM
// ===========================================================================

#[test]
fn no_binder_kind_survives_into_the_emitted_term() {
    // ★ The kind rides on a reserved marker during the parse and is read off the telescope
    // afterwards. What comes OUT is the pure kernel type — `{A : Type} -> A` and
    // `⦃A : Type⦄ -> A` and `(A : Type) -> A` are one and the same `Term`, because the kernel
    // has exactly one Pi and the difference is elaboration's, not typing's.
    let forms = [
        "{A : Type} -> A -> A",
        "⦃A : Type⦄ -> A -> A",
        "(A : Type) -> A -> A",
        "forall A : Type, A -> A",
    ];
    let expected = pi("A", ty0(), arrow(var("A"), var("A")));
    for src in forms {
        assert_eq!(parse(src), expected, "`{src}` must emit the pure kernel Pi");
    }
    // And the instance form, whose class type is not a sort.
    fn has_marked_global(t: &Term) -> bool {
        match t {
            Term::Global(n) => n.contains('⟨') || n.contains('⦃'),
            Term::App(f, a) => has_marked_global(f) || has_marked_global(a),
            Term::Pi {
                param_type,
                body_type,
                ..
            } => has_marked_global(param_type) || has_marked_global(body_type),
            Term::Lambda {
                param_type, body, ..
            } => has_marked_global(param_type) || has_marked_global(body),
            _ => false,
        }
    }
    for src in [
        "[Inhabited Nat] -> Nat",
        "{A : Type} -> [Inhabited A] -> ⦃B : Type⦄ -> A",
        "Nat -> {A : Type} -> A",
    ] {
        assert!(
            !has_marked_global(&parse(src)),
            "`{src}` leaked a binder-kind marker"
        );
    }
}

// ===========================================================================
// THE BINDER-KIND VECTOR · what `set_binder_kinds` has never been given
// ===========================================================================

#[test]
fn the_parser_reports_a_binder_kind_per_position_and_not_a_prefix_count() {
    // ★ `Context::set_binder_kinds` takes a Vec<ParamKind>, one per telescope position, and the
    // elaborator prefers it over the leading-count model whenever it is recorded. Nothing in the
    // tree outside the test suite has ever called it, because no PRODUCER of that vector existed:
    // the parser reported a number. This is the producer.
    let (t, kinds) = TermParser::parse_with_binder_kinds("Nat -> {A : Type} -> A -> A")
        .expect("an interleaved telescope parses");
    assert_eq!(
        t,
        arrow(g("Nat"), pi("A", ty0(), arrow(var("A"), var("A"))))
    );
    assert_eq!(
        kinds,
        vec![
            TermParser::EXPLICIT,
            TermParser::IMPLICIT,
            TermParser::EXPLICIT
        ],
        "explicit, then implicit, then explicit — a POSITION each, which no count can say"
    );
    // The count model is asked the same question and can only answer about the prefix.
    let (_, count) =
        TermParser::parse_with_implicits("Nat -> {A : Type} -> A -> A").expect("parses");
    assert_eq!(
        count, 0,
        "there is no LEADING implicit, and that is all a count can report"
    );

    // All four kinds, in one telescope, in order.
    let (_, every) = TermParser::parse_with_binder_kinds(
        "{A : Type} -> ⦃B : Type⦄ -> [Inhabited A] -> (x : A) -> A",
    )
    .expect("every binder kind in one telescope");
    assert_eq!(
        every,
        vec![
            TermParser::IMPLICIT,
            TermParser::STRICT_IMPLICIT,
            TermParser::INSTANCE,
            TermParser::EXPLICIT
        ],
        "one entry per binder, in source order"
    );

    // The boundary on each side of the telescope: none at all, and a type that is not a Pi.
    let (_, none) = TermParser::parse_with_binder_kinds("Nat").expect("parses");
    assert!(none.is_empty(), "a type with no binders has no kinds");
    let (_, plain) = TermParser::parse_with_binder_kinds("Nat -> Nat -> Nat").expect("parses");
    assert_eq!(
        plain,
        vec![TermParser::EXPLICIT, TermParser::EXPLICIT],
        "an arrow chain is explicit — and `A -> B -> C` is TWO binders, not three: the RESULT \
         type is not a `Pi` and has no binder of its own"
    );
    let (_, one) = TermParser::parse_with_binder_kinds("Nat -> Nat").expect("parses");
    assert_eq!(one, vec![TermParser::EXPLICIT], "one arrow, one binder");

    // And the nested-implicit trap: a `{…}` inside a PARAMETER'S type is not on this spine.
    let (_, nested) = TermParser::parse_with_binder_kinds("{A : Type} -> ({B : Type} -> B) -> A")
        .expect("parses");
    assert_eq!(
        nested,
        vec![TermParser::IMPLICIT, TermParser::EXPLICIT],
        "two parameters: one implicit, one explicit whose TYPE happens to be polymorphic"
    );
}

#[test]
fn a_strict_implicit_reaches_the_elaborator_as_itself_and_the_count_model_stops_at_it() {
    // `ParamKind` and `BinderKind` both have Lean's four `BinderInfo` variants, so `⦃α⦄` is handed
    // to the elaborator as itself: the hand-over is a bijection and loses nothing.
    let (_, kinds) = TermParser::parse_with_binder_kinds("⦃A : Type⦄ -> A -> A").expect("parses");
    assert_eq!(
        kinds[0],
        TermParser::STRICT_IMPLICIT,
        "the parser keeps the distinction"
    );
    assert_eq!(
        kinds[0].as_param_kind(),
        ParamKind::StrictImplicit,
        "and the hand-over keeps it too"
    );
    assert_eq!(TermParser::EXPLICIT.as_param_kind(), ParamKind::Explicit);
    assert_eq!(TermParser::IMPLICIT.as_param_kind(), ParamKind::Implicit);
    assert_eq!(TermParser::INSTANCE.as_param_kind(), ParamKind::Instance);
    let all = [
        TermParser::EXPLICIT,
        TermParser::IMPLICIT,
        TermParser::STRICT_IMPLICIT,
        TermParser::INSTANCE,
    ];
    for (i, a) in all.iter().enumerate() {
        for b in &all[i + 1..] {
            assert_ne!(
                a.as_param_kind(),
                b.as_param_kind(),
                "no two binder kinds may collapse onto one parameter kind"
            );
        }
    }
    assert_ne!(
        TermParser::STRICT_IMPLICIT.as_param_kind(),
        ParamKind::Implicit,
        "a strict implicit is never handed over as an eager implicit"
    );

    // ★ THE COUNT MODEL MUST NOT INSERT IT. `implicit_args` drives an unconditional insertion,
    // and a strict implicit is exactly the binder that must not be inserted with nothing
    // following it. Under-inserting leaves the polymorphic function; over-inserting emits a
    // metavariable nothing can solve.
    let (_, strict_first) =
        TermParser::parse_with_implicits("⦃A : Type⦄ -> A -> A").expect("parses");
    assert_eq!(strict_first, 0, "a strict implicit stops the leading run");
    let (_, after_eager) =
        TermParser::parse_with_implicits("{A : Type} -> ⦃B : Type⦄ -> A").expect("parses");
    assert_eq!(after_eager, 1, "and stops it wherever it stands");
}

// ===========================================================================
// THE DECLARATION DOOR · one call, everything the kernel type cannot hold
// ===========================================================================

#[test]
fn a_declared_type_splits_into_a_kernel_type_and_its_parameter_surface() {
    let d = TermParser::read_declaration(
        "{A : Type} -> [Inhabited A] -> (m : Int := 3) -> (h : Eq Int m m := by decide) -> A",
    )
    .expect("a declaration carrying every kind of surface information");

    // The kernel type is MARKER-FREE — this is what `Context` is keyed at, and a marker reaching
    // it would be an unbound global the kernel refuses.
    assert_eq!(
        d.kernel_type,
        pi(
            "A",
            ty0(),
            arrow(
                app(g("Inhabited"), var("A")),
                pi(
                    "m",
                    g("Int"),
                    pi(
                        "h",
                        apps(g("Eq"), &[g("Int"), var("m"), var("m")]),
                        var("A")
                    )
                )
            )
        ),
        "the declared type with every binder marker stripped"
    );

    assert_eq!(d.binders.len(), 4, "one entry per parameter");
    assert_eq!(d.binders[0].name, "A");
    assert_eq!(d.binders[0].kind, TermParser::IMPLICIT);
    assert_eq!(d.binders[0].declared, ty0());
    assert!(d.binders[0].default.is_none() && d.binders[0].auto.is_none());

    assert_eq!(
        d.binders[1].name, "_",
        "an anonymous instance binder is found, not named"
    );
    assert_eq!(d.binders[1].kind, TermParser::INSTANCE);
    assert_eq!(d.binders[1].declared, app(g("Inhabited"), var("A")));

    assert_eq!(d.binders[2].name, "m");
    assert_eq!(d.binders[2].kind, TermParser::EXPLICIT);
    assert_eq!(d.binders[2].declared, g("Int"));
    assert_eq!(
        d.binders[2].default,
        Some(int(3)),
        "the default value, ready to inline"
    );
    assert_eq!(d.binders[2].auto, None);

    assert_eq!(d.binders[3].name, "h");
    assert_eq!(
        d.binders[3].declared,
        apps(g("Eq"), &[g("Int"), var("m"), var("m")]),
        "the obligation mentions an earlier parameter, so it is a different proposition per call"
    );
    assert_eq!(
        d.binders[3].auto.as_deref(),
        Some("decide"),
        "the tactic that discharges it"
    );
    assert_eq!(
        d.binders[3].default, None,
        "an auto-param is NOT a default — they discharge \
         differently, and conflating them would inline one proof at every call site"
    );

    // The derived answers the declaration door actually asks.
    assert_eq!(
        d.param_kinds(),
        vec![
            ParamKind::Implicit,
            ParamKind::Instance,
            ParamKind::Explicit,
            ParamKind::Explicit
        ]
    );
    assert_eq!(d.leading_implicits(), 1);
    assert!(
        d.needs_recording(),
        "this declaration has surface information to record"
    );
}

#[test]
fn a_declaration_with_nothing_to_record_says_so_and_is_registered_as_it_always_was() {
    // ★ THE NEGATIVE THAT KEEPS THE DOOR CHEAP. The overwhelming majority of declarations carry
    // no binder kind, no default and no auto-param, and for those the new door must produce
    // exactly the old answer — otherwise every existing declaration changes shape the day it is
    // wired in.
    for src in [
        "Nat",
        "Nat -> Nat",
        "(x : Nat) -> Eq Nat x x",
        "forall x : Nat, Eq Nat x x",
    ] {
        let d = TermParser::read_declaration(src).unwrap_or_else(|e| panic!("`{src}`: {e:?}"));
        assert!(!d.needs_recording(), "`{src}` has nothing to record");
        assert_eq!(
            d.kernel_type,
            parse(src),
            "`{src}` is registered at exactly the old type"
        );
        assert_eq!(d.leading_implicits(), 0);
        assert!(
            d.binders.iter().all(|b| b.kind == TermParser::EXPLICIT),
            "`{src}` has only explicit binders"
        );
    }
    // A leading implicit is recordable but its kernel type is still the plain Pi.
    let d = TermParser::read_declaration("{A : Type} -> A -> A").expect("parses");
    assert_eq!(d.kernel_type, pi("A", ty0(), arrow(var("A"), var("A"))));
    assert!(
        d.needs_recording(),
        "an implicit parameter is information the kernel type loses"
    );
}

#[test]
fn the_declaration_surface_still_reads_a_default_off_a_type_the_legacy_door_produced() {
    // The legacy door strips binder KINDS and deliberately leaves defaults and auto-params in
    // place, so a caller that has only ever had `(Term, usize)` can still recover them without
    // re-parsing anything. What it cannot recover is the kinds — and it must not GUESS them.
    let (ty, implicits) =
        TermParser::parse_with_implicits("{A : Type} -> (m : Int := 3) -> A").expect("parses");
    assert_eq!(implicits, 1);
    let d = TermParser::declaration_surface(&ty);
    assert_eq!(
        d.binders[1].default,
        Some(int(3)),
        "the default survives the legacy door"
    );
    assert_eq!(
        d.binders[0].kind,
        TermParser::EXPLICIT,
        "and the kind does NOT — the legacy door stripped it, and reporting anything but the \
         default here would be an invention"
    );
    assert_eq!(
        d.kernel_type,
        pi("A", ty0(), pi("m", g("Int"), var("A"))),
        "marker-free either way"
    );
}

#[test]
fn an_anonymous_binder_takes_its_name_from_the_definition_body() {
    // ★ IN LOGOS THE BODY IS WHERE THE NAMES ARE. `Int -> Int -> Int` names nothing: every one
    // of those binders is `Pi(_ : Int)`. The parameter names `x`, `y`, `z` that a named argument
    // `sub3 (y := 2)` has to resolve against exist ONLY in the definition body's lambda
    // telescope, and reading them off the declared type's binders — the obvious place — gets
    // three underscores.
    let mut d = TermParser::read_declaration("Int -> Int -> Int -> Int").expect("parses");
    assert_eq!(
        d.binders.len(),
        3,
        "three arrows before the result type, so three parameters"
    );
    assert!(
        d.binders.iter().all(|b| b.name == "_"),
        "the declared type names nothing"
    );
    let body = parse("fun x : Int => fun y : Int => fun z : Int => x");
    TermParser::name_anonymous_binders(&mut d.binders, &body);
    assert_eq!(
        d.binders
            .iter()
            .map(|b| b.name.as_str())
            .collect::<Vec<_>>(),
        vec!["x", "y", "z"],
        "each position takes the name the body's lambda telescope binds there"
    );

    // A name the declared type already gives is never overwritten, and the two telescopes stop
    // together.
    let mut d = TermParser::read_declaration("(n : Int) -> Int -> Int").expect("parses");
    let body = parse("fun a : Int => fun b : Int => a");
    TermParser::name_anonymous_binders(&mut d.binders, &body);
    assert_eq!(
        d.binders
            .iter()
            .map(|b| b.name.as_str())
            .collect::<Vec<_>>(),
        vec!["n", "b"],
        "position 0 keeps its declared name `n`; position 1 takes `b` from the body"
    );

    // ★ THE NEGATIVE: a body name that would COLLIDE is refused. Two parameters answering to one
    // name is strictly worse than one parameter with no name, because `f (n := 1)` would then
    // silently pick one of them and typecheck.
    let mut d = TermParser::read_declaration("(n : Int) -> Int -> Int").expect("parses");
    let body = parse("fun a : Int => fun n : Int => a");
    TermParser::name_anonymous_binders(&mut d.binders, &body);
    assert_eq!(
        d.binders[1].name, "_",
        "`n` is already position 0's name, so position 1 stays anonymous rather than shadowing it"
    );

    // A shorter body, a body that is not a lambda at all, and an empty telescope: all no-ops.
    let mut d = TermParser::read_declaration("Int -> Int -> Int").expect("parses");
    TermParser::name_anonymous_binders(&mut d.binders, &parse("fun q : Int => q"));
    assert_eq!(d.binders[0].name, "q");
    assert_eq!(d.binders[1].name, "_", "the telescopes stop together");
    let mut d = TermParser::read_declaration("Int -> Int").expect("parses");
    TermParser::name_anonymous_binders(&mut d.binders, &g("Zero"));
    assert!(
        d.binders.iter().all(|b| b.name == "_"),
        "a non-lambda body renames nothing"
    );
    let mut d = TermParser::read_declaration("Nat").expect("parses");
    TermParser::name_anonymous_binders(&mut d.binders, &parse("fun q : Int => q"));
    assert!(d.binders.is_empty(), "no binders, nothing to name");
}

#[test]
fn every_reserved_marker_including_the_binder_kinds_is_distinct_and_unreachable() {
    // The companion of `the_reserved_markers_are_distinct_and_unspellable`, extended over the
    // three BINDER-KIND markers added with the binder-kind telescope. A kind marker is the one
    // family that never leaves the parser at all, which makes it the easiest to forget — so it
    // gets the same backstop as the rest: an undeclared global the kernel refuses on sight.
    let ctx = std_ctx();
    let all = [
        TermParser::NAMED_ARG_MARKER,
        TermParser::PARAM_DEFAULT_MARKER,
        TermParser::AUTO_PARAM_MARKER,
        TermParser::ELLIPSIS_MARKER,
        TermParser::IMPLICIT_BINDER_MARKER,
        TermParser::STRICT_IMPLICIT_BINDER_MARKER,
        TermParser::INSTANCE_BINDER_MARKER,
    ];
    for (i, a) in all.iter().enumerate() {
        for b in &all[i + 1..] {
            assert_ne!(a, b, "two markers sharing a spelling are one marker");
        }
        assert!(
            ctx.get_global(a).is_none(),
            "`{a}` must not be a declared global"
        );
        assert!(
            matches!(
                infer_type(&ctx, &g(a)),
                Err(KernelError::UnboundVariable(_))
            ),
            "`{a}` reaching the kernel must be refused as unbound"
        );
        assert!(
            TermParser::parse(&format!("«{a}»")).is_err(),
            "`«{a}»` must not parse as a guillemet name"
        );
    }
}

// ===========================================================================
// DEFAULTS, END TO END ON THE PARSER'S SIDE · T05-006's parser half, proven done
// ===========================================================================

#[test]
fn the_declaration_door_has_everything_it_needs_for_a_defaulted_parameter() {
    // ★ THIS IS T05-006's PARSER HALF, AND IT IS COMPLETE. What is still missing is two lines in
    // `command_parser::parse_definition`, which splits the sentence with a raw `input.find(":=")`
    // and therefore splits `Definition withDefault : Int -> (m : Int := 3) -> Int := …` at the
    // default rather than at the body. `find_top_level` is the scan written for exactly that, and
    // this test pins that BOTH halves of the wiring already answer correctly.
    let sentence = "withDefault : Int -> (m : Int := 3) -> Int := \
                    fun n : Int => fun m : Int => add n m";

    // 1. The sentence splits at the BODY's `:=`, not at the default's.
    let at = TermParser::find_top_level(sentence, ":=").expect("a top-level `:=` exists");
    assert_eq!(
        sentence[..at].trim(),
        "withDefault : Int -> (m : Int := 3) -> Int",
        "the default's `:=` is inside a bracket and must not be mistaken for the separator"
    );
    let colon = TermParser::find_top_level(sentence[..at].trim(), ":").expect("an ascription");
    let type_str = &sentence[..at].trim()[colon + 1..];
    assert_eq!(type_str.trim(), "Int -> (m : Int := 3) -> Int");

    // 2. The type reads into a kernel type plus one recorded default.
    let d = TermParser::read_declaration(type_str).expect("the declared type parses");
    assert_eq!(
        d.kernel_type,
        arrow(g("Int"), pi("m", g("Int"), g("Int"))),
        "the type the context is keyed at carries no marker"
    );
    assert_eq!(d.binders.len(), 2);
    assert_eq!(d.binders[0].default, None, "position 0 is required");
    assert_eq!(d.binders[1].name, "m");
    assert_eq!(
        d.binders[1].default,
        Some(int(3)),
        "position 1's default, ready to inline"
    );

    // 3. Omitting it inlines the default, producing a FULL-ARITY spine.
    let filled = TermParser::inline_default(&d.binders, 1, &[int(1)]).expect("a default to inline");
    assert_eq!(filled, int(3), "a constant default needs no substitution");
    assert_eq!(
        apps(g("withDefault"), &[int(1), filled]),
        apps(g("withDefault"), &[int(1), int(3)]),
        "`withDefault 1` is `withDefault 1 3` — a shorter spine would be a function, not an Int"
    );
}

#[test]
fn a_dependent_default_is_closed_by_the_actual_argument_and_never_left_open() {
    // `Int -> (m : Int := mul n 2) -> Int` — the default MENTIONS position 0, which the declared
    // type leaves anonymous, so `n` parses as a `Global` and only the BODY's lambda telescope can
    // say that `n` is position 0.
    let mut d = TermParser::read_declaration("Int -> (m : Int := mul n 2) -> Int").expect("parses");
    assert_eq!(
        d.binders[1].default,
        Some(apps(g("mul"), &[g("n"), int(2)]))
    );

    // ★ THE NEGATIVE, AND IT IS THE WHOLE POINT. With the telescope still anonymous, substitution
    // by name has nothing to match and the inlined default is an OPEN TERM with a free `n` in it.
    let open = TermParser::inline_default(&d.binders, 1, &[int(5)]).expect("a default");
    assert_eq!(
        open,
        apps(g("mul"), &[g("n"), int(2)]),
        "with an anonymous telescope nothing is substituted — this is the term that must never \
         reach the kernel, and the reason `name_anonymous_binders` is not optional"
    );

    // Name the telescope from the body, and the same call closes the term.
    TermParser::name_anonymous_binders(
        &mut d.binders,
        &parse("fun n : Int => fun m : Int => add n m"),
    );
    assert_eq!(
        d.binders[0].name, "n",
        "position 0's name comes from the body"
    );
    let closed = TermParser::inline_default(&d.binders, 1, &[int(5)]).expect("a default");
    assert_eq!(
        closed,
        apps(g("mul"), &[int(5), int(2)]),
        "`n * 2` with n := 5"
    );

    // The boundary on each side: no default at all, and too few arguments to close it.
    assert_eq!(
        TermParser::inline_default(&d.binders, 0, &[]),
        None,
        "position 0 has no default"
    );
    assert_eq!(
        TermParser::inline_default(&d.binders, 1, &[]),
        None,
        "position 1's default needs position 0's argument, and refusing beats guessing"
    );
    assert_eq!(
        TermParser::inline_default(&d.binders, 9, &[int(5)]),
        None,
        "no such position"
    );
}

#[test]
fn a_default_that_shadows_the_parameter_it_mentions_is_not_captured() {
    // A default is parsed in the scope of the binders before it, so a reference to a NAMED
    // earlier binder is a `Term::Var` — and an inner binder of the same name must shadow it.
    let mut d = TermParser::read_declaration("(n : Int) -> (m : Int := (fun n : Int => n)) -> Int")
        .expect("parses");
    TermParser::name_anonymous_binders(&mut d.binders, &parse("fun a : Int => fun b : Int => a"));
    let inlined = TermParser::inline_default(&d.binders, 1, &[int(7)]).expect("a default");
    assert_eq!(
        inlined,
        lam("n", g("Int"), var("n")),
        "the lambda's own `n` is a DIFFERENT variable and substitution must stop at the binder"
    );
}

#[test]
fn an_unclosed_binder_group_says_which_group_it_was() {
    // ★ THE DIAGNOSTIC FOR THE REAL FAILURE. `Definition f : Int -> (m : Int := 3) -> Int := …`
    // is split at the wrong `:=` by the declaration door and this parser is handed
    // `Int -> (m : Int`. `Expected ')' found EOF` describes a parenthesis the reader never typed;
    // naming the binder points at what they did type.
    let err = TermParser::parse("Int -> (m : Int").expect_err("an unclosed binder group");
    let said = format!("{err}");
    assert!(
        said.contains('m'),
        "the refusal must name the binder group: {said}"
    );
    for src in ["{A : Type", "⦃A : Type", "(x y : Nat"] {
        let err = TermParser::parse(src).expect_err("unclosed");
        let said = format!("{err}");
        assert!(
            said.contains("binder group"),
            "`{src}` must be refused as an unclosed binder group, got {said}"
        );
    }
    // And a group that DOES close is untouched.
    assert!(TermParser::parse("Int -> (m : Int) -> Int").is_ok());
}

#[test]
fn an_auto_param_binder_reaches_the_declaration_door_with_its_tactic() {
    // T05-007's parser half. The obligation MENTIONS an earlier parameter, so it is a different
    // proposition at each call site — which is why the tactic name travels and the PROOF does not.
    let mut d = TermParser::read_declaration(
        "Nat -> (b : Nat) -> (h : Not (Eq Nat b Zero) := by decide) -> Nat",
    )
    .expect("parses");
    assert_eq!(
        d.kernel_type,
        arrow(
            g("Nat"),
            pi(
                "b",
                g("Nat"),
                pi(
                    "h",
                    app(g("Not"), apps(g("Eq"), &[g("Nat"), var("b"), g("Zero")])),
                    g("Nat")
                )
            )
        ),
        "the registered type is the obligation itself, with no marker and no proof in it"
    );
    assert_eq!(d.binders[2].auto.as_deref(), Some("decide"));
    assert_eq!(
        d.binders[2].default, None,
        "a tactic is not a default value"
    );
    assert_eq!(
        d.binders[2].declared,
        app(g("Not"), apps(g("Eq"), &[g("Nat"), var("b"), g("Zero")])),
        "the proposition the tactic must prove, still mentioning `b`"
    );
    // An auto-param is never INLINED — `inline_default` must decline it, or one call site's proof
    // would be reused at another about a different proposition.
    assert_eq!(
        TermParser::inline_default(&d.binders, 2, &[int(10), int(2)]),
        None,
        "an auto-param has no default to inline; the tactic runs per call site"
    );
    TermParser::name_anonymous_binders(&mut d.binders, &parse("fun a : Nat => fun b : Nat => a"));
    assert_eq!(
        d.binders[0].name, "a",
        "position 0's name still comes from the body"
    );
}

// ===========================================================================
// WAVE 3 · BINDER KINDS IN A `fun` / `forall` TELESCOPE
//
// ★ A TELESCOPE COULD ONLY BIND EXPLICITLY. The Pi-position grammar has read all four of Lean's
// brackets since wave 2 (`{A : Type} -> B`, `⦃A : Type⦄ -> B`, `[C x] -> B`), but `fun` and
// `forall` read `(x y : T)` and a bare name and NOTHING ELSE — so `theorem foo {α} [Monoid α]
// (a : α)`, the shape of very nearly every declaration in mathlib, had no spelling in an
// abstraction at all, and `forall {A : Type}, A -> A` was a parse error. A `forall` builds a `Pi`,
// so the kind marker carrier the Pi grammar already uses carries these end to end; a `fun` builds
// a `Lambda`, and `binder_kinds_of` now walks that spine too so the kinds reach the same door.
// ===========================================================================

#[test]
fn a_forall_telescope_binds_with_every_bracket_the_arrow_form_has() {
    // The implicit binder mathlib opens nearly every statement with.
    let (implicit, kinds) = TermParser::parse_with_binder_kinds("forall {A : Type}, A -> A")
        .expect("`forall` with an implicit binder must parse");
    assert_eq!(
        kinds,
        vec![TermParser::IMPLICIT, TermParser::EXPLICIT],
        "the braced binder is IMPLICIT and the arrow after it is explicit"
    );
    // ★ THE TWO SPELLINGS ARE ONE TYPE. Lean writes both and means the same thing; if the
    // telescope form produced a different term the surface would have two type languages.
    let (arrowed, arrow_kinds) = TermParser::parse_with_binder_kinds("{A : Type} -> A -> A")
        .expect("the arrow spelling still parses");
    assert_eq!(
        implicit, arrowed,
        "a forall telescope and the arrow form build the same Pi"
    );
    assert_eq!(kinds, arrow_kinds, "and report the same kinds");
    assert_eq!(implicit, pi("A", ty0(), arrow(var("A"), var("A"))));

    // The strict-implicit bracket.
    let (strict, kinds) = TermParser::parse_with_binder_kinds("forall ⦃A : Type⦄, A -> A")
        .expect("`forall` with a strict-implicit binder must parse");
    assert_eq!(
        kinds,
        vec![TermParser::STRICT_IMPLICIT, TermParser::EXPLICIT]
    );
    assert_eq!(
        strict, implicit,
        "the bracket is the only difference; the Pi is the same"
    );

    // The instance bracket, anonymous — an instance argument is FOUND, not named.
    let (inst, kinds) = TermParser::parse_with_binder_kinds("forall [Monoid A], Nat")
        .expect("`forall` with an anonymous instance binder must parse");
    assert_eq!(kinds, vec![TermParser::INSTANCE]);
    assert_eq!(
        inst,
        arrow(app(g("Monoid"), g("A")), g("Nat")),
        "the binder is `_`"
    );

    // …and named, Lean's `[inst : C x]`, whose name is in scope in the body.
    let (named_inst, kinds) =
        TermParser::parse_with_binder_kinds("forall [i : Monoid A], Monoid A")
            .expect("`forall [i : C]` must parse");
    assert_eq!(kinds, vec![TermParser::INSTANCE]);
    assert_eq!(
        named_inst,
        pi("i", app(g("Monoid"), g("A")), app(g("Monoid"), g("A")))
    );
}

#[test]
fn a_lambda_telescope_carries_the_kind_of_every_binder_it_binds() {
    // `fun {A} (a : A) => a` — the value half of the same shape. `Term::Lambda` has no kind field,
    // so the kind rides on the binder's own `param_type` exactly as it does on a `Pi`, and is
    // stripped by every public door.
    let (t, kinds) = TermParser::parse_with_binder_kinds("fun {A} (a : A) => a")
        .expect("`fun` with an implicit binder must parse");
    assert_eq!(kinds, vec![TermParser::IMPLICIT, TermParser::EXPLICIT]);
    assert_eq!(
        t,
        lam("A", Term::Hole, lam("a", var("A"), var("a"))),
        "the implicit binder's own type is POSTPONED — nothing wrote one"
    );
    // An annotated implicit lambda binder.
    let (annotated, kinds) =
        TermParser::parse_with_binder_kinds("fun {A : Type} (a : A) => a").expect("parses");
    assert_eq!(kinds, vec![TermParser::IMPLICIT, TermParser::EXPLICIT]);
    assert_eq!(annotated, lam("A", ty0(), lam("a", var("A"), var("a"))));

    // An ORDINARY lambda still reports every binder explicit, and is byte-identical to what it
    // always was — the kinds walk costs a term with no markers in it nothing.
    let (plain, kinds) =
        TermParser::parse_with_binder_kinds("fun (x : Nat) (y : Bool) => x").expect("parses");
    assert_eq!(kinds, vec![TermParser::EXPLICIT, TermParser::EXPLICIT]);
    assert_eq!(plain, lam("x", g("Nat"), lam("y", g("Bool"), var("x"))));
    assert_eq!(
        TermParser::parse_with_implicits("fun (x : Nat) => x")
            .unwrap()
            .1,
        0
    );
    assert_eq!(
        TermParser::parse_with_implicits("fun {A} (a : A) => a")
            .unwrap()
            .1,
        1
    );
}

#[test]
fn the_mathlib_binder_shape_interleaves_all_three_kinds_in_one_telescope() {
    // ★ THE SHAPE THE WHOLE LANE IS FOR. `theorem foo {α} [Monoid α] (a : α)` interleaves an
    // implicit, an instance and an explicit parameter — which is the one thing a LEADING-COUNT
    // model of implicits can never describe, and the reason `Context::set_binder_kinds` exists.
    let d = TermParser::read_declaration("forall {A : Type} [Monoid A] (a : A), Eq A a a")
        .expect("the interleaved telescope must parse");
    assert_eq!(
        d.param_kinds(),
        vec![
            ParamKind::Implicit,
            ParamKind::Instance,
            ParamKind::Explicit
        ],
        "one kind per POSITION, in telescope order"
    );
    assert_eq!(
        d.leading_implicits(),
        1,
        "the leading-count model sees only the first"
    );
    assert!(
        d.needs_recording(),
        "a declaration with an instance parameter must be recorded"
    );
    assert_eq!(
        d.kernel_type,
        pi(
            "A",
            ty0(),
            arrow(
                app(g("Monoid"), var("A")),
                pi(
                    "a",
                    var("A"),
                    apps(g("Eq"), &[var("A"), var("a"), var("a")])
                )
            )
        ),
        "the registered type is the pure kernel Pi telescope, with no marker anywhere in it"
    );
    assert_eq!(d.binders[0].name, "A");
    assert_eq!(
        d.binders[1].name, "_",
        "an anonymous instance binder is `_`"
    );
    assert_eq!(d.binders[2].name, "a");
    assert_eq!(
        d.binders[1].declared,
        app(g("Monoid"), var("A")),
        "the class type sees `A`"
    );
}

#[test]
fn no_binder_kind_marker_survives_an_abstraction_into_the_emitted_term() {
    // The backstop the Pi grammar already has, now for the abstraction grammar: a marker that
    // reached a kernel term would be an unbound global, and one that reached a DECLARED type
    // would key the context at a type mentioning it.
    for src in [
        "fun {A} (a : A) => a",
        "fun ⦃A : Type⦄ (a : A) => a",
        "fun [Monoid A] (a : A) => a",
        "forall {A : Type} [Monoid A] (a : A), Eq A a a",
        "forall ⦃A : Type⦄ {B : Type}, A",
    ] {
        let t = parse(src);
        let printed = format!("{t}");
        for m in [
            TermParser::IMPLICIT_BINDER_MARKER,
            TermParser::STRICT_IMPLICIT_BINDER_MARKER,
            TermParser::INSTANCE_BINDER_MARKER,
        ] {
            assert!(
                !printed.contains(m),
                "`{src}` must not carry the marker {m} into the emitted term: {printed}"
            );
        }
    }
}

#[test]
fn an_abstraction_binder_group_is_refused_when_it_is_not_one() {
    for src in [
        "fun {} => Zero",
        "fun { : Type} => Zero",
        "fun ⦃⦄ => Zero",
        "fun {A : Type => Zero",
        "fun [] => Zero",
        "forall {A : Type} Nat",
        "fun {fun x : Nat} => Zero",
    ] {
        assert!(TermParser::parse(src).is_err(), "`{src}` must be refused");
    }
    // …and the neighbours that must still be ACCEPTED, so the refusals are about the malformed
    // group and not about the bracket.
    for src in [
        "fun {A} => A",
        "forall {A}, A",
        "fun {A B : Type} (a : A) => a",
        "forall [C], C",
    ] {
        assert!(
            TermParser::parse(src).is_ok(),
            "`{src}` must parse, got {:?}",
            TermParser::parse(src)
        );
    }
}

// ===========================================================================
// WAVE 3 · `_` IS A HOLE, NOT A GLOBAL THE KERNEL RESERVES
// ===========================================================================

#[test]
fn an_underscore_is_a_hole_and_can_no_longer_spell_the_kernels_placeholder() {
    // ★ A DEFECT THAT IS NOT IN THE CORPUS. `_` read as `Term::Global("_")`, and that global is
    // the TRUSTED KERNEL'S OWN match-case placeholder protocol: `type_checker::check_type` matches
    // a λ whose `param_type` is exactly that global and takes the binder's type from the expected
    // `Pi` instead of checking the one that was written. So a user-written `fun x : _ => x` was
    // indistinguishable from a match case's synthesised binder, and `_` was a name a declaration
    // could be keyed at. `Term::Hole` is the honest carrier — the same collision class the
    // guillemet rule in `scan_name` closes, reached by a different route.
    assert_eq!(
        parse("_"),
        Term::Hole,
        "Lean's `_` is a placeholder, not a name"
    );
    assert_eq!(
        parse("Eq _ Zero Zero"),
        apps(g("Eq"), &[Term::Hole, g("Zero"), g("Zero")]),
        "`_` at an argument position is the hole the elaborator solves"
    );
    assert_eq!(parse("fun x : _ => x"), lam("x", Term::Hole, var("x")));

    // It fails LOUDLY rather than checking against everything.
    let ctx = std_ctx();
    assert!(
        matches!(
            infer_type(&ctx, &parse("_")),
            Err(KernelError::CannotInferHole)
        ),
        "a bare hole has no type: {:?}",
        infer_type(&ctx, &parse("_"))
    );

    // `_` as a BINDER NAME is untouched — `A -> B` names its binder `_` everywhere in this kernel.
    assert_eq!(parse("fun _ : Nat => Zero"), lam("_", g("Nat"), g("Zero")));
    // …and a reference to it in the body is a HOLE, as in Lean, where an `_`-named binder is
    // inaccessible: it is not a way to name the thing you declined to name.
    assert_eq!(parse("fun _ : Nat => _"), lam("_", g("Nat"), Term::Hole));

    // A name that merely BEGINS with `_` is an ordinary name and still is one.
    assert_eq!(parse("_foo"), g("_foo"));
    assert_eq!(parse("_1"), g("_1"));
}

#[test]
fn a_written_underscore_reaches_the_hole_solver_the_corpus_already_proves_works() {
    // ★ THE CAPABILITY THE SPELLING UNLOCKS, AND WHY THE DEFECT WAS INVISIBLE.
    // `lean_conformance_t05_elaboration::underscore_holes_are_solved_and_never_reach_the_kernel` (T05-013)
    // is GREEN: the elaborator solves `Term::Hole` from an argument, from the expected type, and
    // several at once. But that test CONSTRUCTS `Term::Hole` in Rust — and no source text could
    // write one, because `_` read as `Term::Global("_")`. A capability with no spelling is a
    // capability no user has. This is the same assertion, reached through the PARSER.
    let mut ctx = std_ctx();
    ctx.add_declaration("idf", pi("A", ty0(), pi("a", var("A"), var("A"))));
    let solved = surface_elaborate(&ctx, &parse("idf _ Zero"))
        .expect("`idf _ Zero` written as TEXT must elaborate");
    assert_eq!(
        solved,
        apps(g("idf"), &[g("Nat"), g("Zero")]),
        "the written `_` is solved to Nat, exactly as the hand-built Term::Hole is"
    );
    assert_eq!(infer_type(&ctx, &solved).unwrap(), g("Nat"));

    // The BOUNDARY, and the safety property: a written `_` is a request to INFER, never a licence
    // to check against anything. The RAW parse still carries the hole and the kernel refuses it
    // outright — only ELABORATION solves it, and only where a constraint actually reaches it.
    assert!(
        matches!(
            infer_type(&ctx, &parse("idf _ Zero")),
            Err(KernelError::CannotInferHole)
        ),
        "the unelaborated `idf _ Zero` must be refused by the kernel, got {:?}",
        infer_type(&ctx, &parse("idf _ Zero"))
    );
}

// ===========================================================================
// WAVE 3 · THE DOT COMPONENT, READ BY THE ONE NAME GRAMMAR
// ===========================================================================

#[test]
fn a_dot_component_is_read_by_the_same_name_grammar_the_declaration_door_uses() {
    let proj = |recv: Term, field: &str| apps(g(DOT_MARKER), &[recv, g(field)]);

    // The ordinary component, unchanged.
    assert_eq!(parse("p.fst"), proj(g("p"), "fst"));
    assert_eq!(parse("Outer.value"), proj(g("Outer"), "value"));

    // ★ THE HANDOFF FROM LANE V. `is_field` admitted a component only when the next character was
    // `is_alphabetic() || '_'` — `scan_name`'s FIRST rule rather than its whole rule — so
    // `Outer.«a b»` was a name no term could spell, and the declaration door had to refuse a name
    // Lean accepts (`def Foo.«a b»`) in order to keep "every name the declaration door accepts, a
    // term can spell" true.
    assert_eq!(parse("Outer.«a b»"), proj(g("Outer"), "a b"));
    assert_eq!(parse("«a b».«c d»"), proj(g("a b"), "c d"));

    // Lean's anonymous projection `h.1`.
    assert_eq!(parse("h.1"), proj(g("h"), "1"));
    assert_eq!(parse("h.1.2"), proj(proj(g("h"), "1"), "2"));

    // ★ AND THE COLLISION IT MUST NOT CREATE. `3.14` is a decimal numeral, not a projection out of
    // `3` — `command_parser::sentence_end` names it in its own doc comment as a thing that sits
    // inside a term without ending the sentence, so the collision is reachable rather than
    // theoretical. This grammar has no float literal, so `3.14` stays REFUSED; a grammar addition
    // that quietly turned refused input into a parse would be a bug introduced by a feature.
    assert!(
        TermParser::parse("3.14").is_err(),
        "`3.14` is not a projection out of `3`"
    );
    assert!(TermParser::parse("2.5").is_err(), "`2.5` likewise");
    // The sentence terminator and the bare trailing dot are untouched.
    assert_eq!(parse("Zero."), g("Zero"));
    assert!(
        TermParser::parse("Zero..").is_err(),
        "one terminator, and nothing after it"
    );
    assert_eq!(parse("3."), int(3), "a numeral then a full stop");
}

// ===========================================================================
// WAVE 3 · A `let` ASCRIPTION IS A CLAIM, AND A CLAIM IS CHECKED
// ===========================================================================

#[test]
fn a_let_ascription_reaches_the_kernel_instead_of_being_parsed_and_discarded() {
    let ctx = std_ctx();

    // The UN-ascribed form is unchanged: capture-avoiding substitution, no node at all.
    assert_eq!(parse("let x := Zero in Succ x"), app(g("Succ"), g("Zero")));

    // ★ A DEFECT THAT IS NOT IN THE CORPUS. `let x : T := e in b` parsed `T` and threw it away —
    // the code said so in a comment — so a FALSE ascription was a claim the language accepted and
    // no kernel ever saw. `Term::Let` has a typing rule in BOTH kernels, so the claim is now
    // checked twice by two independent implementations.
    let honest = parse("let x : Nat := Zero in Succ x");
    assert!(
        matches!(honest, Term::Let { .. }),
        "an ascribed let is a kernel Let: {honest}"
    );
    assert_eq!(infer_type(&ctx, &honest).unwrap(), g("Nat"));
    assert_eq!(
        double_check(&ctx, &honest),
        DoubleCheck::Agreed,
        "the ascription is checked by both kernels, not one"
    );
    assert_eq!(
        normalize(&ctx, &honest),
        normalize(&ctx, &app(g("Succ"), g("Zero"))),
        "zeta still reduces it to the value the substituting form produced"
    );

    let lie = parse("let x : Bool := Zero in x");
    assert!(
        infer_type(&ctx, &lie).is_err(),
        "`let x : Bool := Zero` is a false claim and must be refused: {:?}",
        infer_type(&ctx, &lie)
    );
    assert!(
        logicaffeine_kernel::recheck(&ctx, &lie).is_err(),
        "…and the independent re-checker must refuse it too"
    );

    // The ascription may be a real type expression, and the bound name is in scope in the body.
    let dependent = parse("let f : Nat -> Nat := Succ in f Zero");
    assert_eq!(infer_type(&ctx, &dependent).unwrap(), g("Nat"));
    assert_eq!(
        normalize(&ctx, &dependent),
        normalize(&ctx, &app(g("Succ"), g("Zero")))
    );
}

// ===========================================================================
// WAVE 3 · THE SORTS THE KERNEL HAS AND THE SURFACE COULD NOT NAME
// ===========================================================================

#[test]
fn every_sort_the_kernel_prints_is_a_sort_the_parser_reads_back() {
    // ★ A DEFECT THAT IS NOT IN THE CORPUS. `Term::Sort(SProp)` PRINTS as `SProp` — `Universe`'s
    // own `Display` says so — and this parser read that text straight back as an undeclared
    // global. So the kernel's definitionally-irrelevant universe, the one with its own elimination
    // rule and its own `0 = 1` corpus case, was a sort no source text could land in.
    assert_eq!(parse("SProp"), Term::Sort(Universe::SProp));
    assert_eq!(parse("Prop"), Term::Sort(Universe::Prop));
    for u in [
        Universe::SProp,
        Universe::Prop,
        Universe::Type(0),
        Universe::Type(7),
    ] {
        let printed = Term::Sort(u.clone()).to_string();
        assert_eq!(
            TermParser::parse(&printed).unwrap_or_else(|e| panic!("`{printed}` must parse: {e:?}")),
            Term::Sort(u.clone()),
            "the kernel prints `{printed}` for {u:?}, so the parser must read it back as that sort"
        );
    }
    // A name that merely begins with one of them is an ordinary name.
    assert_eq!(parse("SPropish"), g("SPropish"));
    assert_eq!(parse("Propositional"), g("Propositional"));
    assert_eq!(parse("Typeclass"), g("Typeclass"));
}

#[test]
fn the_concatenated_sort_spelling_refuses_exactly_what_the_spaced_one_refuses() {
    // ★ THE OTHER HALF OF `a_universe_level_that_does_not_fit_is_refused_and_never_read_as_level_zero`.
    // That test pinned the SPACED form (`Type 4294967296` is refused) and the top rung
    // (`Type4294967295` is a sort). The CONCATENATED form above the top rung fell through to
    // `Global("Type4294967296")` — an ordinary name that reads as a sort to every human who sees
    // it. Two spellings of one sort must refuse the same inputs.
    assert_eq!(
        parse("Type4294967295"),
        Term::Sort(Universe::Type(u32::MAX)),
        "the top rung"
    );
    for src in [
        "Type4294967296",
        "Type99999999999999999999",
        "Type18446744073709551616",
    ] {
        let got = TermParser::parse(src);
        assert!(
            got.is_err(),
            "`{src}` names a level `Universe::Type` cannot hold and must be REFUSED, exactly as \
             the spaced spelling is — got {got:?}"
        );
        let spaced = format!("Type {}", &src[4..]);
        assert!(
            TermParser::parse(&spaced).is_err(),
            "`{spaced}` is refused, and so must `{src}` be"
        );
    }
    // A suffix that is not all digits was never a level and is still an ordinary name.
    assert_eq!(parse("Type_x"), g("Type_x"));
    assert_eq!(parse("Type0x"), g("Type0x"));
    assert_eq!(
        parse("Type007"),
        Term::Sort(Universe::Type(7)),
        "leading zeros are still digits"
    );
}

// ===========================================================================
// WAVE 3 · THE HAND-OVER TO THE ELABORATOR IS A BIJECTION
// ===========================================================================

#[test]
fn every_binder_kind_reaches_the_elaborator_as_itself() {
    // ★ THE COLLAPSE THAT WAS WAITING FOR ONE VARIANT. `BinderKind` has four inhabitants and
    // `ParamKind` had three, so `⦃α⦄` could only be handed over as `Implicit` — and the elaborator
    // would then insert its metavariable unconditionally, which is the ONE thing a strict implicit
    // must not do. `ParamKind::StrictImplicit` now exists and `elaborate_app_against` implements
    // its rule, so the hand-over is a bijection: four kinds, four kinds, nothing lost.
    assert_eq!(TermParser::EXPLICIT.as_param_kind(), ParamKind::Explicit);
    assert_eq!(TermParser::IMPLICIT.as_param_kind(), ParamKind::Implicit);
    assert_eq!(
        TermParser::STRICT_IMPLICIT.as_param_kind(),
        ParamKind::StrictImplicit
    );
    assert_eq!(TermParser::INSTANCE.as_param_kind(), ParamKind::Instance);

    // It is INJECTIVE: no two surface brackets may arrive at the elaborator indistinguishable.
    let kinds = [
        TermParser::EXPLICIT,
        TermParser::IMPLICIT,
        TermParser::STRICT_IMPLICIT,
        TermParser::INSTANCE,
    ];
    for (i, a) in kinds.iter().enumerate() {
        for b in kinds.iter().skip(i + 1) {
            assert_ne!(
                a.as_param_kind(),
                b.as_param_kind(),
                "{a:?} and {b:?} are different brackets and must reach the elaborator differently"
            );
        }
    }

    // …and it travels through the DECLARATION door, in telescope order, interleaved.
    let d = TermParser::read_declaration("{A : Type} -> ⦃B : Type⦄ -> [Inhabited A] -> A -> A")
        .expect("parses");
    assert_eq!(
        d.param_kinds(),
        vec![
            ParamKind::Implicit,
            ParamKind::StrictImplicit,
            ParamKind::Instance,
            ParamKind::Explicit
        ]
    );
    // The legacy COUNT still stops at the strict implicit — a count describes an unconditional
    // prefix, and inserting there is the metavariable nothing can solve.
    assert_eq!(
        d.leading_implicits(),
        1,
        "the count model sees only the eager prefix"
    );
    assert_eq!(
        TermParser::parse_with_implicits("⦃A : Type⦄ -> A -> A")
            .unwrap()
            .1,
        0,
        "a strict implicit still stops the leading run"
    );
}

#[test]
fn a_body_name_is_never_given_to_two_positions_at_once() {
    // ★ WHERE THE PARAMETER NAMES ARE, ANSWERED ONCE. `A -> B` names nothing, so for a
    // non-dependent signature the writer's names exist ONLY in the body's λ-telescope — and a
    // named argument can be resolved from nowhere else. The rule is not local, which is why it
    // lives in one exported function instead of being re-derived at each door.
    let mut names = vec!["_".to_string(), "_".to_string(), "_".to_string()];
    TermParser::name_anonymous_positions(
        &mut names,
        &parse("fun x : Int => fun y : Int => fun z : Int => x"),
    );
    assert_eq!(
        names,
        vec!["x", "y", "z"],
        "each anonymous position takes the body's name"
    );

    // ★ THE COLLISION, AND IT IS ORDINARY TEXT. `(x : Int) -> Int -> Int` defined as
    // `fun y => fun x => …`: position 0 is DECLARED `x`, and position 1's body binder is ALSO
    // `x`. Handing both the name `x` makes `f (x := 1)` silently pick one and leaves the other
    // unreachable by name. The second position keeps `"_"` instead, so a named argument either
    // means the position that really is `x` or is refused by name.
    let mut collide = vec!["x".to_string(), "_".to_string()];
    TermParser::name_anonymous_positions(
        &mut collide,
        &parse("fun y : Int => fun x : Int => sub y x"),
    );
    assert_eq!(
        collide,
        vec!["x", "_"],
        "a body name already taken by another position is NOT given out twice"
    );

    // The telescopes run in step only while BOTH last: a body shorter than the signature leaves
    // the tail anonymous rather than sliding the names up a position.
    let mut short_body = vec!["_".to_string(), "_".to_string(), "_".to_string()];
    TermParser::name_anonymous_positions(&mut short_body, &parse("fun a : Int => Zero"));
    assert_eq!(
        short_body,
        vec!["a", "_", "_"],
        "names stop where the body's telescope stops"
    );

    // A body binder that is ITSELF anonymous names nothing, and does not consume the position.
    let mut anon_body = vec!["_".to_string(), "_".to_string()];
    TermParser::name_anonymous_positions(&mut anon_body, &parse("fun _ : Int => fun b : Int => b"));
    assert_eq!(
        anon_body,
        vec!["_", "b"],
        "an anonymous body binder gives no name away"
    );

    // A RECURSIVE definition's body is wrapped in a `Fix`; the names are inside it.
    let mut recursive = vec!["_".to_string(), "_".to_string()];
    TermParser::name_anonymous_positions(
        &mut recursive,
        &parse("fix go => fun n : Int => fun acc : Int => acc"),
    );
    assert_eq!(
        recursive,
        vec!["n", "acc"],
        "a `fix` wrapper is transparent to the name source"
    );

    // And the `BinderSurface` door delegates to exactly this, so the two cannot drift apart.
    let mut d = TermParser::read_declaration("(x : Int) -> Int -> Int").expect("parses");
    TermParser::name_anonymous_binders(&mut d.binders, &parse("fun y : Int => fun x : Int => y"));
    assert_eq!(
        d.binders
            .iter()
            .map(|b| b.name.as_str())
            .collect::<Vec<_>>(),
        vec!["x", "_"],
        "the declaration door answers the same, because it asks the same function"
    );
}

// ===========================================================================
// WAVE 3 · `@f` — EXPLICIT-ALL, THE ESCAPE HATCH THE LANGUAGE COULD NOT SPELL
// ===========================================================================

#[test]
fn an_explicit_all_head_is_spellable_and_carries_its_head_to_the_elaborator() {
    let explicit_all = |head: Term| app(marker(TermParser::EXPLICIT_ALL_MARKER), head);

    // ★ THE FOURTH ARGUMENT-PASSING CONVENTION, AND IT HAD NO SPELLING AT ALL. `@` turns implicit
    // insertion OFF for one call site, which is precisely what a writer reaches for when inference
    // has failed. Without it an implicit argument cannot be written down: once a declaration's
    // binder kinds are recorded, `idf Nat Zero` is "insert a metavariable, then two arguments for
    // one explicit parameter", not "here are both parameters". `@` was a lexer error.
    assert_eq!(
        parse("@idf"),
        explicit_all(g("idf")),
        "the bare form marks the head"
    );
    assert_eq!(
        parse("@idf Nat Zero"),
        apps(
            marker(TermParser::EXPLICIT_ALL_MARKER),
            &[g("idf"), g("Nat"), g("Zero")]
        ),
        "applied, the marker is the SPINE HEAD and the head it marks is the first argument"
    );
    assert_eq!(
        TermParser::as_explicit_all(&parse("@idf")),
        Some(&g("idf")),
        "the unapplied form reads back to the head it marks"
    );
    assert!(
        TermParser::as_explicit_all(&parse("idf")).is_none(),
        "an ordinary head is not one"
    );

    // The spine form, read the way an elaborator's application arm already reads a spine.
    let spine = parse("@idf Nat Zero");
    let mut head = &spine;
    let mut args: Vec<&Term> = Vec::new();
    while let Term::App(f, a) = head {
        args.push(a);
        head = f;
    }
    args.reverse();
    assert!(
        TermParser::is_explicit_all_head(head),
        "the innermost position holds the marker"
    );
    assert_eq!(
        args,
        vec![&g("idf"), &g("Nat"), &g("Zero")],
        "and `f` is args[0]"
    );

    // It composes with the other conventions: a dotted head, a quoted component, a named argument.
    assert_eq!(
        parse("@Outer.value"),
        explicit_all(apps(g(DOT_MARKER), &[g("Outer"), g("value")])),
        "`@` marks a hierarchical head too"
    );
    assert_eq!(
        parse("@f (x := 1)"),
        app(explicit_all(g("f")), named("x", int(1))),
        "`@` and a named argument are different conventions and do not interfere"
    );

    // ★ THE NEGATIVES. `@` binds TIGHTLY to a name, exactly as in Lean — a grammar addition that
    // quietly accepted the neighbouring malformed inputs would be a bug introduced by a feature.
    for src in ["@", "@ f", "@(f)", "@1", "@@f", "@ ", "@->"] {
        assert!(
            TermParser::parse(src).is_err(),
            "`{src}` is not an explicit-all head"
        );
    }
    // …and an `@` at ARGUMENT position is Lean's `f @g`, which is a different thing and legal.
    assert_eq!(parse("f @g"), app(g("f"), explicit_all(g("g"))));

    // The marker is a RESERVED global nothing can inhabit or spell, like every other one.
    let ctx = std_ctx();
    assert!(
        ctx.get_global(TermParser::EXPLICIT_ALL_MARKER).is_none(),
        "the explicit-all marker must never be a declared global"
    );
    assert!(
        infer_type(&ctx, &parse("@idf")).is_err(),
        "a marker that reaches the kernel unexpanded must fail LOUDLY, not typecheck"
    );
    for other in [
        TermParser::NAMED_ARG_MARKER,
        TermParser::PARAM_DEFAULT_MARKER,
        TermParser::AUTO_PARAM_MARKER,
        TermParser::ELLIPSIS_MARKER,
        TermParser::IMPLICIT_BINDER_MARKER,
        TermParser::STRICT_IMPLICIT_BINDER_MARKER,
        TermParser::INSTANCE_BINDER_MARKER,
        DOT_MARKER,
    ] {
        assert_ne!(
            TermParser::EXPLICIT_ALL_MARKER,
            other,
            "no two reserved markers may share a spelling"
        );
    }
    assert!(
        TermParser::parse(&format!("\u{ab}{}\u{bb}", TermParser::EXPLICIT_ALL_MARKER)).is_err(),
        "and a guillemet name may not spell it either"
    );
}

// ===========================================================================
// WAVE 4 · THE DECLARATION DOOR, END TO END THROUGH THE REPL
//
// ★ A DEFECT THAT IS NOT IN THE CORPUS, AND IT IS THE SHAPE OF MATHLIB. The elaborator has
// understood interleaved binder kinds since wave 2 — `full_param_kinds` and the kinds match in
// `elab_surface_uncoerced` both read `Context::binder_kinds`, one entry per POSITION — and the
// parser has read all four of Lean's brackets since wave 3. Between them the channel is a
// `usize`. `Command::Definition` carries `implicit_count`, `parse_with_implicits_in` strips the
// kind markers off the declared type before the REPL ever sees it, and `register_definition`
// then re-derives the surface from an ALREADY-STRIPPED type — where every binder necessarily
// reads `Explicit` — and paints the first `implicit_count` of them back to `Implicit`.
//
// So `Definition constOf : Nat -> {A : Type} -> A -> A` registers THREE EXPLICIT parameters:
// the count stops at the first non-leading binder, and the loop that repairs it only reaches
// the prefix the count described. `⦃A : Type⦄` and `[Inhabited A]` are lost the same way and
// worse — a strict implicit becomes an argument the caller must now WRITE, and an instance
// binder stops consulting the instance database at all. `theorem foo {α} [Monoid α] (a : α)`,
// which is very nearly every declaration in mathlib, has no working spelling.
//
// `Context::set_binder_kinds` is the API the elaborator reads and NOTHING outside the test
// suite has ever called it. These are the tests that say so.
// ===========================================================================

fn declaring(decls: &[&str]) -> Repl {
    let mut repl = Repl::new();
    for d in decls {
        repl.execute(d)
            .unwrap_or_else(|e| panic!("`{d}` must be accepted, got {e:?}"));
    }
    repl
}

#[test]
fn a_declared_binder_kind_survives_the_repl_and_is_not_flattened_to_explicit() {
    // ★ THE INTERLEAVED CASE. The implicit stands in SECOND position, so no count describes it.
    let repl = declaring(&["Definition constOf : Nat -> {A : Type} -> A -> A := \
         fun n : Nat => fun A : Type => fun a : A => a."]);
    assert_eq!(
        repl.context().binder_kinds("constOf"),
        Some(
            &[
                ParamKind::Explicit,
                ParamKind::Implicit,
                ParamKind::Explicit
            ][..]
        ),
        "a non-leading implicit must reach the elaborator as an implicit at ITS OWN position"
    );

    // …and the consequence at a call site, which is the whole reason the kind is recorded.
    let ctx = repl.context();
    assert_eq!(
        surface_elaborate(ctx, &parse("constOf Zero Zero")).expect("two explicit arguments"),
        apps(g("constOf"), &[g("Zero"), g("Nat"), g("Zero")]),
        "the implicit is INSERTED between them and solved from the second argument's type"
    );

    // ★ THE STRICT IMPLICIT. Lean inserts it only when a later explicit argument is supplied
    // (`Lean/Elab/App.lean:726 processStrictImplicitArg`), which is a rule a leading COUNT
    // cannot express at all — the count model stops dead at it.
    let strict = declaring(&[
        "Definition idOf : \u{2983}A : Type\u{2984} -> A -> A := fun A : Type => fun a : A => a.",
    ]);
    assert_eq!(
        strict.context().binder_kinds("idOf"),
        Some(&[ParamKind::StrictImplicit, ParamKind::Explicit][..]),
        "\u{2983}\u{2984} must not arrive as an argument the caller has to write"
    );
    assert_eq!(
        surface_elaborate(strict.context(), &parse("idOf Zero")).expect("one explicit argument"),
        apps(g("idOf"), &[g("Nat"), g("Zero")]),
        "supplied an explicit argument, the strict implicit is inserted and solved"
    );

    // ★ THE INSTANCE BINDER. Flattened to explicit, the instance database is never consulted.
    let inst = declaring(&["Definition pick : {A : Type} -> [Inhabited A] -> A := \
         fun A : Type => fun d : Inhabited A => Inhabited_default A d."]);
    assert_eq!(
        inst.context().binder_kinds("pick"),
        Some(&[ParamKind::Implicit, ParamKind::Instance][..]),
        "`[C x]` must reach the elaborator as an INSTANCE position, not an explicit one"
    );
}

#[test]
fn a_declaration_with_only_leading_implicits_records_the_same_thing_both_ways() {
    // THE BOUNDARY on the other side: the shape the legacy count model DOES describe must go on
    // meaning exactly what it meant, whichever channel carries it.
    let repl = declaring(&[
        "Definition idf : {A : Type} -> A -> A := fun A : Type => fun a : A => a.",
        "Definition plain : Nat -> Nat := fun n : Nat => n.",
    ]);
    assert_eq!(
        repl.context().implicit_args("idf"),
        1,
        "the count model still answers"
    );
    assert_eq!(
        repl.context().binder_kinds("idf"),
        Some(&[ParamKind::Implicit, ParamKind::Explicit][..]),
        "and the per-position model agrees with it"
    );
    assert_eq!(
        surface_elaborate(repl.context(), &parse("idf Zero")).expect("elaborates"),
        apps(g("idf"), &[g("Nat"), g("Zero")]),
        "which is the behaviour that was already there, unchanged"
    );
    // A declaration with nothing to say says nothing: recording an all-explicit vector for every
    // ordinary definition would put an entry in the map for every name in the prelude.
    assert_eq!(
        repl.context().binder_kinds("plain"),
        None,
        "an all-explicit telescope records no kinds — the elaborator's default already is that"
    );
}

#[test]
fn the_marker_preserving_door_carries_every_binder_kind_to_the_context() {
    // ★ THE PARSER HALF OF THE FIX, PROVED WITHOUT THE DECLARATION DOOR. `read_declared_type`
    // keeps the kind markers ON, so the `declaration_surface` call the REPL's
    // `register_definition` ALREADY makes reads the real telescope instead of a flattened one.
    // Everything below uses only public API, so it is the two-line change in
    // `command_parser::parse_definition` and `Repl::register_definition` done by hand — and it
    // demonstrates that those two lines are sufficient, not merely necessary.
    let src = "Nat -> {A : Type} -> A -> A";

    // The legacy door: markers stripped, a count that cannot reach the second position.
    let (stripped, legacy_count) = TermParser::parse_with_implicits(src).expect("parses");
    assert_eq!(
        legacy_count, 0,
        "no LEADING implicit, which is all a count can report"
    );
    assert_eq!(
        TermParser::declaration_surface(&stripped).param_kinds(),
        vec![
            ParamKind::Explicit,
            ParamKind::Explicit,
            ParamKind::Explicit
        ],
        "read back off a stripped type every binder is explicit — correctly, the type no \
         longer says anything else. THIS is the flattening."
    );

    // The new door: the same count, and a type that still knows.
    let (marked, count) = TermParser::read_declared_type(src).expect("parses");
    assert_eq!(
        count, legacy_count,
        "the count beside it is unchanged, bit for bit"
    );
    let d = TermParser::declaration_surface(&marked);
    assert_eq!(
        d.param_kinds(),
        vec![
            ParamKind::Explicit,
            ParamKind::Implicit,
            ParamKind::Explicit
        ],
        "one kind per POSITION, and the implicit is at the position the source put it"
    );
    assert_eq!(
        d.kernel_type, stripped,
        "and the kernel type it splits off is EXACTLY what the legacy door returned — the \
         registered type cannot drift by carrying the markers one layer further"
    );

    // ★ THE NEGATIVE THAT MAKES A FORGOTTEN SPLIT LOUD. The marked type is not a kernel term.
    let mut ctx = std_ctx();
    assert!(
        infer_type(&ctx, &marked).is_err(),
        "a door that registered the marked type directly must fail LOUDLY, not typecheck"
    );

    // Now the hand-over, and the behaviour it buys: register the kernel type and the kinds.
    ctx.add_declaration("constOf", d.kernel_type.clone());
    ctx.set_binder_kinds("constOf", d.param_kinds());
    assert_eq!(
        surface_elaborate(&ctx, &parse("constOf Zero Zero")).expect("two explicit arguments"),
        apps(g("constOf"), &[g("Zero"), g("Nat"), g("Zero")]),
        "the implicit is inserted BETWEEN the two written arguments and solved from the \
         second one's type — which is the entire point of recording a position"
    );

    // The same, for the two kinds a count has no spelling for at all.
    for (src, kinds) in [
        (
            "\u{2983}A : Type\u{2984} -> A -> A",
            vec![ParamKind::StrictImplicit, ParamKind::Explicit],
        ),
        (
            "{A : Type} -> [Inhabited A] -> A",
            vec![ParamKind::Implicit, ParamKind::Instance],
        ),
        (
            "forall {A : Type} [Monoid A] (a : A), Eq A a a",
            vec![
                ParamKind::Implicit,
                ParamKind::Instance,
                ParamKind::Explicit,
            ],
        ),
    ] {
        let (marked, _) = TermParser::read_declared_type(src).expect("parses");
        assert_eq!(
            TermParser::declaration_surface(&marked).param_kinds(),
            kinds,
            "`{src}` must reach the context as itself"
        );
    }

    // ★ AND THE BOUNDARY ON THE OTHER SIDE: an ordinary declaration is bit-identical through
    // both doors, so the swap costs the overwhelming majority of declarations nothing.
    for src in [
        "Nat",
        "Nat -> Nat",
        "Nat -> Nat -> Bool",
        "forall (n : Nat), Eq Nat n n",
    ] {
        let (old, old_n) = TermParser::parse_with_implicits(src).expect("parses");
        let (new, new_n) = TermParser::read_declared_type(src).expect("parses");
        assert_eq!(
            new, old,
            "`{src}` has no marker to preserve, so the doors agree exactly"
        );
        assert_eq!(new_n, old_n, "`{src}` reports the same count either way");
        assert!(
            !TermParser::declaration_surface(&new).needs_recording(),
            "`{src}` has nothing to record, so the new branch is not even taken"
        );
    }
}

#[test]
fn a_strict_implicit_reaches_the_elaborator_as_itself_and_the_count_still_stops_at_it() {
    // ★ THE BEHAVIOUR LEAN HAS, pinned from the parser's side by
    // `a_strict_implicit_reaches_the_elaborator_as_itself_and_the_count_model_stops_at_it`.
    // `ParamKind` has four variants, so `⦃⦄` is handed over as itself and
    // `BinderKind::as_param_kind` is a bijection. Lean's `BinderInfo` has four constructors
    // (`Lean/Expr.lean:69-78`) and the fourth is not decoration: `Lean/Elab/App.lean:726`
    // `processStrictImplicitArg` inserts the metavariable only `if (← hasArgsToProcess)` and
    // otherwise `finalize`s, where `processImplicitArg` (App.lean:670) inserts unconditionally.
    // Collapsing here would make `nilOf` — unapplied — instantiate, which Lean does not do.
    let (_, kinds) =
        TermParser::parse_with_binder_kinds("\u{2983}A : Type\u{2984} -> A -> A").expect("parses");
    assert_eq!(
        kinds[0],
        TermParser::STRICT_IMPLICIT,
        "the parser keeps the distinction"
    );
    assert_eq!(
        kinds[0].as_param_kind(),
        ParamKind::StrictImplicit,
        "and hands it over as itself — there is no longer anything to collapse"
    );
    assert_eq!(TermParser::EXPLICIT.as_param_kind(), ParamKind::Explicit);
    assert_eq!(TermParser::IMPLICIT.as_param_kind(), ParamKind::Implicit);
    assert_eq!(TermParser::INSTANCE.as_param_kind(), ParamKind::Instance);

    // A BIJECTION, stated as one: four kinds in, four distinct kinds out.
    let every = [
        TermParser::EXPLICIT,
        TermParser::IMPLICIT,
        TermParser::STRICT_IMPLICIT,
        TermParser::INSTANCE,
    ];
    let mapped: Vec<ParamKind> = every.iter().map(|k| k.as_param_kind()).collect();
    for i in 0..mapped.len() {
        for j in (i + 1)..mapped.len() {
            assert_ne!(
                mapped[i], mapped[j],
                "{:?} and {:?} must not share a ParamKind — the hand-over is total and injective",
                every[i], every[j]
            );
        }
    }

    // ★ THE COUNT MODEL MUST NOT INSERT IT, and that half is unchanged. `implicit_args` drives
    // an UNCONDITIONAL insertion of a leading run, and a strict implicit is exactly the binder
    // whose insertion is conditional — so the honest thing a count can say about it is nothing.
    let (_, strict_first) =
        TermParser::parse_with_implicits("\u{2983}A : Type\u{2984} -> A -> A").expect("parses");
    assert_eq!(strict_first, 0, "a strict implicit stops the leading run");
    let (_, after_eager) =
        TermParser::parse_with_implicits("{A : Type} -> \u{2983}B : Type\u{2984} -> A")
            .expect("parses");
    assert_eq!(after_eager, 1, "and stops it wherever it stands");
    // …through the marker-preserving door too: the two doors report the same number, always.
    for src in [
        "\u{2983}A : Type\u{2984} -> A -> A",
        "{A : Type} -> \u{2983}B : Type\u{2984} -> A",
        "{A : Type} -> {B : Type} -> A",
        "[Inhabited Nat] -> Nat",
    ] {
        assert_eq!(
            TermParser::read_declared_type(src).expect("parses").1,
            TermParser::parse_with_implicits(src).expect("parses").1,
            "`{src}`: the marker-preserving door counts exactly what the stripping one counts"
        );
    }
}

// ===========================================================================
// WAVE 4 · `{{x : A}}` — LEAN'S ASCII STRICT-IMPLICIT BRACKETS
//
// ★ THE FOURTH BINDER KIND HAD NO SPELLING ON AN ORDINARY KEYBOARD. `Lean/Parser/Term.lean:346`
// defines `strictImplicitLeftBracket := atomic (group (symbol "{" >> "{")) <|> "⦃"` and the next
// line its `⦄` counterpart, and the binder's own doc (`:377`) calls `{{y z : A}}` "its ASCII
// alternative". We read only the guillemets, so `⦃⦄` — the one kind whose whole purpose is to
// be written in a signature — required a unicode input method to write at all, and `{{A : Type}}`
// was `Expected { expected: "a binder name", found: "{" }`.
//
// The two halves are separate TOKENS in Lean and Lean tokenises across whitespace, so `{ {x : A} }`
// is the same binder; and `bracketedBinder` tries `strictImplicitBinder` BEFORE `implicitBinder`
// (`:381`), as `funBinder` tries `funStrictImplicitBinder` first (`:493`) — without that ordering
// the first brace is eaten as an eager implicit whose binder name is then `{`.
// ===========================================================================

#[test]
fn the_ascii_strict_implicit_brackets_are_leans_and_mean_exactly_the_guillemets() {
    // The ASCII and the guillemet spelling are the SAME BINDER: same term, same kind, everywhere
    // a binder can stand. Anything less makes one of them a second-class spelling.
    for (ascii, guillemet) in [
        (
            "{{A : Type}} -> A -> A",
            "\u{2983}A : Type\u{2984} -> A -> A",
        ),
        ("{{A B : Type}} -> A", "\u{2983}A B : Type\u{2984} -> A"),
        (
            "Nat -> {{A : Type}} -> A -> A",
            "Nat -> \u{2983}A : Type\u{2984} -> A -> A",
        ),
        (
            "forall {{A : Type}}, A -> A",
            "forall \u{2983}A : Type\u{2984}, A -> A",
        ),
        ("fun {{A : Type}} => A", "fun \u{2983}A : Type\u{2984} => A"),
        (
            "fun {{A}} (a : A) => a",
            "fun \u{2983}A\u{2984} (a : A) => a",
        ),
        // The trap: a binder type that itself ENDS in a brace. The close must not be found early.
        (
            "{{A : {B : Type} -> Type}} -> A",
            "\u{2983}A : {B : Type} -> Type\u{2984} -> A",
        ),
    ] {
        let a = TermParser::parse_with_binder_kinds(ascii)
            .unwrap_or_else(|e| panic!("`{ascii}` must parse, got {e:?}"));
        let u = TermParser::parse_with_binder_kinds(guillemet)
            .unwrap_or_else(|e| panic!("`{guillemet}` must parse, got {e:?}"));
        assert_eq!(
            a.0, u.0,
            "`{ascii}` and `{guillemet}` must build the same term"
        );
        assert_eq!(
            a.1, u.1,
            "`{ascii}` and `{guillemet}` must report the same binder kinds"
        );
        assert!(
            a.1.contains(&TermParser::STRICT_IMPLICIT),
            "`{ascii}` must produce a STRICT implicit, not an eager one"
        );
    }

    // ★ THE ORDERING. `{{` must beat `{`, or its first brace is an eager implicit binder whose
    // name is the second brace. The eager spelling is untouched, which is the other half.
    let (_, strict) = TermParser::parse_with_binder_kinds("{{A : Type}} -> A").expect("parses");
    assert_eq!(
        strict,
        vec![TermParser::STRICT_IMPLICIT],
        "`{{` is strict, not eager"
    );
    let (_, eager) = TermParser::parse_with_binder_kinds("{A : Type} -> A").expect("parses");
    assert_eq!(
        eager,
        vec![TermParser::IMPLICIT],
        "and one brace is still eager"
    );
    let (_, mixed) =
        TermParser::parse_with_binder_kinds("{{A : Type}} -> {B : Type} -> A").expect("parses");
    assert_eq!(
        mixed,
        vec![TermParser::STRICT_IMPLICIT, TermParser::IMPLICIT],
        "the two spellings stand next to each other and stay distinct"
    );

    // Whitespace between the halves: two tokens, and Lean tokenises across whitespace.
    assert_eq!(
        parse("{ {A : Type} } -> A -> A"),
        parse("{{A : Type}} -> A -> A"),
        "the halves are separate tokens, so a space between them changes nothing"
    );

    // ★ THE NEGATIVES. A grammar addition is the classic way to quietly accept malformed input,
    // and every one of these was already refused before the ASCII bracket existed.
    for src in [
        "{{A : Type}",         // opened doubled, closed single
        "{A : Type}}",         // opened single, closed doubled
        "{{}} -> A",           // no binder name
        "{{A : Type}}",        // a binder group is not a term: there is no body
        "{{A}} -> A",          // `{α} → α` is CRYPTIC and Lean refuses it (Parser/Term.lean:384)
        "{{A : }} -> A",       // no type
        "f {{x : Nat}}",       // a binder group is not an argument
        "{{{A : Type}}} -> A", // three is not two
    ] {
        assert!(
            TermParser::parse(src).is_err(),
            "`{src}` must be REFUSED — the ASCII bracket may not repair a malformed binder"
        );
    }

    // …and the count model treats it exactly as it treats the guillemet form: a strict implicit
    // stops the leading run, because `implicit_args` inserts unconditionally and this binder's
    // insertion is conditional (`Lean/Elab/App.lean:726`).
    assert_eq!(
        TermParser::parse_with_implicits("{{A : Type}} -> A -> A")
            .expect("parses")
            .1,
        0,
        "the ASCII spelling stops the leading-implicit run, like the guillemet one"
    );
    assert_eq!(
        TermParser::parse_with_implicits("{A : Type} -> {{B : Type}} -> A")
            .expect("parses")
            .1,
        1,
        "and stops it wherever it stands"
    );

    // It reaches the DECLARATION DOOR as itself, which is the only thing a call site can use.
    let d = TermParser::read_declaration("{{A : Type}} -> [Inhabited A] -> A -> A")
        .expect("the ASCII telescope parses");
    assert_eq!(
        d.param_kinds(),
        vec![
            ParamKind::StrictImplicit,
            ParamKind::Instance,
            ParamKind::Explicit
        ]
    );
    assert!(
        !matches!(d.kernel_type, Term::Global(_)),
        "and its kernel type is an ordinary Pi telescope with no marker left in it"
    );
    let (marked, _) = TermParser::read_declared_type("{{A : Type}} -> A -> A").expect("parses");
    assert_eq!(
        TermParser::declaration_surface(&marked).param_kinds(),
        vec![ParamKind::StrictImplicit, ParamKind::Explicit],
        "through the marker-preserving door too"
    );
}

#[test]
fn only_an_explicit_binder_may_carry_a_default_and_the_refusal_says_why() {
    // ★ A DEFAULT THAT COULD NEVER FIRE WAS BEING ACCEPTED. Lean gives the `binderDefault` /
    // `binderTactic` clause to `explicitBinder` alone (`Lean/Parser/Term.lean:334`) —
    // `implicitBinder` (:344) and `strictImplicitBinder` (:361) have no such clause — and the
    // elaborator agrees: `getOptParamDefault?` / `getAutoParamTactic?` are consulted inside
    // `processExplicitArg` (`Lean/Elab/App.lean:614`) and NOWHERE else. We parsed `{n : Nat := 3}`
    // happily and recorded a default at a position `elaborate_app_against` fills with
    // `Fill::Infer`, so the writer's value was silently discarded. A refusal is strictly better
    // than a value that is read, stored, and never used.
    for src in [
        "{n : Nat := 3} -> Nat",
        "{n m : Nat := 3} -> Nat",
        "\u{2983}n : Nat := 3\u{2984} -> Nat",
        "{{n : Nat := 3}} -> Nat",
        "{h : P := by decide} -> Nat",
        "Nat -> {n : Nat := 3} -> Nat",
    ] {
        let e = TermParser::parse(src).expect_err(&format!("`{src}` must be refused"));
        let rendered = format!("{e:?}");
        assert!(
            rendered.contains("default"),
            "`{src}` must be refused FOR the default, not by accident — got {rendered}"
        );
        assert!(
            rendered.contains("unification"),
            "`{src}`'s refusal must say why a default there could never be used — got {rendered}"
        );
    }

    // ★ THE POSITIVE IT MUST NOT HAVE EATEN. The explicit binder's default is untouched, and so
    // is every implicit binder that never had one.
    let t = parse("(m : Int := 3) -> Int");
    let Term::Pi { param_type, .. } = &t else {
        panic!("still a Pi")
    };
    assert!(
        TermParser::as_default_param(param_type).is_some(),
        "`(m : Int := 3)` still defaults"
    );
    assert_eq!(
        TermParser::read_declaration("{A : Type} -> (m : Int := 3) -> A")
            .expect("parses")
            .binders
            .iter()
            .map(|b| (b.kind, b.default.clone()))
            .collect::<Vec<_>>(),
        vec![
            (TermParser::IMPLICIT, None),
            (TermParser::EXPLICIT, Some(int(3)))
        ],
        "an implicit beside a defaulted explicit is unaffected — only the default MOVED is refused"
    );
    for ok in [
        "{A : Type} -> A -> A",
        "\u{2983}A : Type\u{2984} -> A -> A",
        "{{A : Type}} -> A -> A",
        "{A B : Type} -> A",
        "(h : P := by decide) -> Nat",
    ] {
        assert!(TermParser::parse(ok).is_ok(), "`{ok}` must still parse");
    }

    // And the `:=` that is a DECLARATION's own separator is not a binder default: the top-level
    // scan must still find it, or `Definition f : {A : Type} -> A -> A := …` stops parsing.
    assert_eq!(
        TermParser::find_top_level("f : {A : Type} -> A -> A := body", ":="),
        Some(25),
        "the declaration's own `:=` is outside every binder group and still found"
    );
}
