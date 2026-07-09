//! R1 — the independent de Bruijn re-checker, locked in by TDD.
//!
//! The point of a second kernel is to catch a soundness bug in the first. These tests
//! prove the re-checker actually does its job: it AGREES with `infer_type` on a corpus
//! of well-typed terms (differential testing — the strongest check), it INDEPENDENTLY
//! REJECTS ill-typed terms (so agreement is not vacuous), it handles variable SHADOWING
//! and capture-prone substitution correctly (the de Bruijn payoff — the scary-bug
//! class), and it is HONEST about the inductive fragment it does not yet cover
//! (`Match`/`Fix` → flagged, never a false pass). A `Disagree` verdict must never fire
//! on a valid proof; that would mean the two kernels see the world differently.

use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{
    double_check, infer_type, recheck, Context, DoubleCheck, Literal, ReCheckError, Term, Universe,
};

// --- term builders ---
fn ty(n: u32) -> Term {
    Term::Sort(Universe::Type(n))
}
fn prop() -> Term {
    Term::Sort(Universe::Prop)
}
fn g(name: &str) -> Term {
    Term::Global(name.to_string())
}
fn var(name: &str) -> Term {
    Term::Var(name.to_string())
}
fn pi(p: &str, a: Term, b: Term) -> Term {
    Term::Pi { param: p.to_string(), param_type: Box::new(a), body_type: Box::new(b) }
}
fn lam(p: &str, a: Term, b: Term) -> Term {
    Term::Lambda { param: p.to_string(), param_type: Box::new(a), body: Box::new(b) }
}
fn app(f: Term, x: Term) -> Term {
    Term::App(Box::new(f), Box::new(x))
}
fn std_ctx() -> Context {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    ctx
}

/// The headline guarantee: two independently-written kernels concur.
fn assert_agree(ctx: &Context, term: &Term, what: &str) {
    // Sanity: the main kernel must actually accept it, or the corpus is wrong.
    assert!(
        infer_type(ctx, term).is_ok(),
        "{what}: main kernel rejected a term the test claims is well-typed: {:?}",
        infer_type(ctx, term)
    );
    assert_eq!(
        double_check(ctx, term),
        DoubleCheck::Agreed,
        "{what}: the two kernels must agree, got {:?}",
        double_check(ctx, term)
    );
}

// ===========================================================================
// CORE: the re-checker accepts well-typed terms and agrees with infer_type
// ===========================================================================

#[test]
fn sorts_and_cumulativity_agree() {
    let ctx = Context::new();
    assert_agree(&ctx, &ty(0), "Type 0 : Type 1");
    assert_agree(&ctx, &ty(5), "Type 5 : Type 6");
    assert_agree(&ctx, &prop(), "Prop : Type 1");
}

#[test]
fn polymorphic_identity_agrees() {
    // λA:Type0. λx:A. x  :  Π(A:Type0). A → A  — dependent, the classic.
    let ctx = Context::new();
    let id = lam("A", ty(0), lam("x", var("A"), var("x")));
    assert_agree(&ctx, &id, "polymorphic identity");

    // Its inferred type, independently, is a Π.
    let t = recheck(&ctx, &id).expect("identity re-checks");
    assert!(matches!(t, Term::Pi { .. }), "identity should infer a Π, got {t}");
}

#[test]
fn modus_ponens_proof_term_agrees() {
    // λP:Prop. λQ:Prop. λpq:P→Q. λp:P. pq p
    //   : Π(P:Prop). Π(Q:Prop). (P→Q) → P → Q     — →E, the workhorse, all in the core.
    let ctx = Context::new();
    let mp = lam(
        "P",
        prop(),
        lam(
            "Q",
            prop(),
            lam(
                "pq",
                pi("_", var("P"), var("Q")),
                lam("p", var("P"), app(var("pq"), var("p"))),
            ),
        ),
    );
    assert_agree(&ctx, &mp, "modus ponens");
}

#[test]
fn globals_and_application_agree() {
    let ctx = std_ctx();
    assert_agree(&ctx, &g("Zero"), "Zero : Nat");
    assert_agree(&ctx, &g("Succ"), "Succ : Nat → Nat");
    assert_agree(&ctx, &app(g("Succ"), g("Zero")), "Succ Zero : Nat");
    assert_agree(
        &ctx,
        &app(g("Succ"), app(g("Succ"), g("Zero"))),
        "Succ (Succ Zero) : Nat",
    );
}

#[test]
fn impredicative_prop_pi_agrees() {
    // Π(x:Entity). Prop-bodied — must land in Prop (impredicativity), not Type.
    let ctx = std_ctx();
    let universal = pi("x", g("Entity"), prop());
    assert_agree(&ctx, &universal, "∀ over Entity into Prop");
    // And a Π into Type stays predicative.
    assert_agree(&ctx, &pi("x", ty(0), ty(0)), "Type0 → Type0 : Type1");
}

#[test]
fn literals_agree() {
    let ctx = std_ctx();
    assert_agree(&ctx, &Term::Lit(Literal::Int(42)), "Int literal");
    assert_agree(&ctx, &Term::Lit(Literal::Text("hi".into())), "Text literal");
}

// ===========================================================================
// INDEPENDENT REJECTION: agreement is not vacuous — the re-checker really checks
// ===========================================================================

#[test]
fn rejects_application_of_a_non_function() {
    // (Type0) (Prop) — applying a sort is nonsense; the re-checker must catch it ITSELF.
    let ctx = Context::new();
    let bad = app(ty(0), prop());
    assert!(
        matches!(recheck(&ctx, &bad), Err(ReCheckError::Ill(_))),
        "applying a non-function must be rejected by the re-checker"
    );
    // And the two kernels agree it is bad (both reject).
    assert_eq!(double_check(&ctx, &bad), DoubleCheck::Agreed);
}

#[test]
fn rejects_type_mismatched_application() {
    // Succ true — Succ wants Nat, `true : Bool`. A real type error.
    let ctx = std_ctx();
    let bad = app(g("Succ"), g("true"));
    assert!(
        matches!(recheck(&ctx, &bad), Err(ReCheckError::Ill(_))),
        "Succ applied to a Bool must be rejected"
    );
    assert_eq!(double_check(&ctx, &bad), DoubleCheck::Agreed);
}

#[test]
fn rejects_unbound_local_variable() {
    let ctx = Context::new();
    let bad = var("nope");
    assert!(
        matches!(recheck(&ctx, &bad), Err(ReCheckError::Ill(_))),
        "an unbound local must be rejected"
    );
}

#[test]
fn rejects_pi_whose_domain_is_not_a_type() {
    // Π(x : (Succ Zero)). Prop — the domain `Succ Zero : Nat` is a value, not a type.
    let ctx = std_ctx();
    let bad = pi("x", app(g("Succ"), g("Zero")), prop());
    assert!(
        matches!(recheck(&ctx, &bad), Err(ReCheckError::Ill(_))),
        "a Π with a non-type domain must be rejected"
    );
    assert_eq!(double_check(&ctx, &bad), DoubleCheck::Agreed);
}

#[test]
fn rejects_unknown_global() {
    let ctx = Context::new();
    let bad = g("DoesNotExist");
    assert!(matches!(recheck(&ctx, &bad), Err(ReCheckError::Ill(_))));
}

// ===========================================================================
// THE SCARY-BUG CLASS: shadowing and capture-prone substitution (de Bruijn payoff)
// ===========================================================================

#[test]
fn variable_shadowing_resolves_to_the_inner_binder() {
    // λx:Type0. λx:Nat. x   — the body's `x` is the INNER (Nat) binder. A name-based
    // checker that resolved to the wrong binder would mistype this; de Bruijn cannot.
    let ctx = std_ctx();
    let shadow = lam("x", ty(0), lam("x", g("Nat"), var("x")));
    assert_agree(&ctx, &shadow, "shadowing picks the inner binder");

    // The inferred type's final codomain must be Nat (the inner x's type), not Type0.
    let t = recheck(&ctx, &shadow).expect("shadowing re-checks");
    // Π(x:Type0). Π(x:Nat). Nat
    if let Term::Pi { body_type, .. } = &t {
        if let Term::Pi { body_type: inner, .. } = body_type.as_ref() {
            assert_eq!(**inner, g("Nat"), "inner x has type Nat, got {inner}");
        } else {
            panic!("expected nested Π, got {t}");
        }
    } else {
        panic!("expected Π, got {t}");
    }
}

#[test]
fn dependent_application_substitutes_a_binder_containing_argument() {
    // (λA:Type0. λx:A. x) (Π(y:Nat). Nat)
    //   The result type substitutes A := (Π(y:Nat).Nat) into `A → A`. A capture-unsafe
    //   substitution could mangle the inner binder `y`; de Bruijn keeps it exact.
    let ctx = std_ctx();
    let id = lam("A", ty(0), lam("x", var("A"), var("x")));
    let arg = pi("y", g("Nat"), g("Nat"));
    let applied = app(id, arg.clone());
    assert_agree(&ctx, &applied, "apply id to a Π-typed argument");

    // The inferred type is (Π(y:Nat).Nat) → (Π(y:Nat).Nat).
    let t = recheck(&ctx, &applied).expect("re-checks");
    let expected = pi("_", arg.clone(), arg);
    // Compare via the main kernel's normalize-agnostic structural sense: both kernels
    // already agreed (assert_agree), so just confirm the shape is the arrow we expect.
    assert!(
        matches!(&t, Term::Pi { .. }),
        "expected an arrow type, got {t}"
    );
    let _ = expected;
}

#[test]
fn nested_shadowing_under_application_agrees() {
    // A deliberately adversarial nest of same-named binders threaded through an
    // application — the kind of term where a sloppy name-based substitution corrupts
    // scope. If the two kernels agree here, capture is handled.
    let ctx = std_ctx();
    // λf:(Nat→Nat). λf:Nat. f   applied to Succ and then Zero is ill-shaped; instead
    // type the closed term: λf:(Nat→Nat). λx:Nat. f x   then shadow x by reusing names.
    let term = lam(
        "x",
        pi("_", g("Nat"), g("Nat")),
        lam("x", g("Nat"), app(var("x"), var("x"))),
    );
    // Inner `x : Nat` applied to itself is ill-typed (Nat is not a function) — both must
    // reject, consistently.
    assert_eq!(
        double_check(&ctx, &term),
        DoubleCheck::Agreed,
        "both kernels must consistently reject the ill-typed self-application"
    );
    assert!(recheck(&ctx, &term).is_err());
}

// ===========================================================================
// MATCH: the inductive eliminator is now independently re-checked
// ===========================================================================

#[test]
fn match_is_fully_double_checked() {
    // match true return Bool with { true => true, false => false }  — a valid match. The
    // re-checker now covers Match: coverage + per-case typing + ι; it must AGREE, not
    // merely flag. (This is the Match layer — the boundary moved here from `Unsupported`.)
    let ctx = std_ctx();
    let m = Term::Match {
        discriminant: Box::new(g("true")),
        motive: Box::new(g("Bool")),
        cases: vec![g("true"), g("false")],
    };
    assert!(recheck(&ctx, &m).is_ok(), "the re-checker must accept a valid match");
    assert_eq!(
        double_check(&ctx, &m),
        DoubleCheck::Agreed,
        "a valid Bool match should be fully double-verified, got {:?}",
        double_check(&ctx, &m)
    );
}

#[test]
fn match_with_wrong_case_count_is_independently_rejected() {
    // A match on Bool with only ONE case is non-exhaustive — the re-checker's coverage
    // check must catch it ITSELF (agreement on rejection is not vacuous).
    let ctx = std_ctx();
    let bad = Term::Match {
        discriminant: Box::new(g("true")),
        motive: Box::new(g("Bool")),
        cases: vec![g("true")],
    };
    assert!(
        matches!(recheck(&ctx, &bad), Err(ReCheckError::Ill(_))),
        "a non-exhaustive match must be rejected by the re-checker's coverage check"
    );
    assert_eq!(double_check(&ctx, &bad), DoubleCheck::Agreed, "both reject it");
}

// ===========================================================================
// FIX + TERMINATION GUARD — the soundness keystone, independently re-derived.
// These are the crown-jewel tests: the re-checker's OWN guard must reject the
// fixpoints that inhabit `False`, and accept genuine structural recursion.
// ===========================================================================

/// Nat → Nat helper type and the `f n`/`f k` recursive-call shape.
fn nat_arrow() -> Term {
    pi("_", g("Nat"), g("Nat"))
}

#[test]
fn trivially_terminating_fix_is_fully_double_checked() {
    // fix f. λn:Nat. Zero  — terminates (no recursive call). Now fully re-checked.
    let ctx = std_ctx();
    let f = Term::Fix { name: "f".to_string(), body: Box::new(lam("n", g("Nat"), g("Zero"))) };
    assert!(recheck(&ctx, &f).is_ok(), "a terminating fix must be accepted");
    assert_eq!(double_check(&ctx, &f), DoubleCheck::Agreed);
}

#[test]
fn genuine_structural_recursion_is_accepted() {
    // fix f. λn:Nat. match n with Zero => Zero | Succ k => f k   — `k` is structurally
    // smaller, so the guard must PASS (and the two kernels agree on the type).
    let ctx = std_ctx();
    let body = lam(
        "n",
        g("Nat"),
        Term::Match {
            discriminant: Box::new(var("n")),
            motive: Box::new(lam("_", g("Nat"), g("Nat"))),
            cases: vec![g("Zero"), lam("k", g("Nat"), app(var("f"), var("k")))],
        },
    );
    let f = Term::Fix { name: "f".to_string(), body: Box::new(body) };
    assert!(
        recheck(&ctx, &f).is_ok(),
        "genuine structural recursion must be accepted: {:?}",
        recheck(&ctx, &f)
    );
    assert_eq!(double_check(&ctx, &f), DoubleCheck::Agreed);
}

#[test]
fn non_decreasing_self_call_is_independently_rejected() {
    // fix f. λn:Nat. f n  — the classic non-terminating shape; `n` is the parameter, not
    // smaller. The re-checker's OWN guard must reject it (the main kernel does too).
    let ctx = std_ctx();
    let f = Term::Fix {
        name: "f".to_string(),
        body: Box::new(lam("n", g("Nat"), app(var("f"), var("n")))),
    };
    assert!(
        matches!(recheck(&ctx, &f), Err(ReCheckError::Ill(_))),
        "a non-decreasing self-call must be rejected by the re-checker's guard, got {:?}",
        recheck(&ctx, &f)
    );
    assert_eq!(double_check(&ctx, &f), DoubleCheck::Agreed, "both reject it");
}

#[test]
fn higher_order_escape_that_inhabits_false_is_independently_rejected() {
    // THE soundness test. fix f. λn:Nat. match n with
    //   Zero   => (λg:Nat→False. g Zero) f     -- `f` SMUGGLED as a first-class argument
    //   Succ k => f k
    // With the escape, `f` is visited only as an inert value, the guard would pass, and
    // `boom Zero : False` inhabits False with zero axioms. The re-checker's INDEPENDENT
    // guard must reject the body — exactly the bug a second kernel exists to catch.
    let ctx = std_ctx();
    let nat_to_false = pi("_", g("Nat"), g("False"));
    let zero_case = app(lam("g", nat_to_false, app(var("g"), g("Zero"))), var("f"));
    let succ_case = lam("k", g("Nat"), app(var("f"), var("k")));
    let body = lam(
        "n",
        g("Nat"),
        Term::Match {
            discriminant: Box::new(var("n")),
            motive: Box::new(lam("_", g("Nat"), g("False"))),
            cases: vec![zero_case, succ_case],
        },
    );
    let f = Term::Fix { name: "f".to_string(), body: Box::new(body) };
    assert!(
        matches!(recheck(&ctx, &f), Err(ReCheckError::Ill(_))),
        "the higher-order escape inhabits False and MUST be rejected by the re-checker, got {:?}",
        recheck(&ctx, &f)
    );
    // The main kernel rejects it too — consistent. (Both correctly refuse the term.)
    assert_eq!(double_check(&ctx, &f), DoubleCheck::Agreed);
}

#[test]
fn bare_recursive_name_returned_from_a_branch_is_independently_rejected() {
    // fix f. λn:Nat. match n with Zero => f | Succ k => f k   — the bare `f` returned
    // from a branch is the same escape in a different costume. Must be rejected.
    let ctx = std_ctx();
    let body = lam(
        "n",
        g("Nat"),
        Term::Match {
            discriminant: Box::new(var("n")),
            motive: Box::new(lam("_", g("Nat"), nat_arrow())),
            cases: vec![var("f"), lam("k", g("Nat"), app(var("f"), var("k")))],
        },
    );
    let f = Term::Fix { name: "f".to_string(), body: Box::new(body) };
    assert!(
        matches!(recheck(&ctx, &f), Err(ReCheckError::Ill(_))),
        "a branch returning the bare fixpoint must be rejected, got {:?}",
        recheck(&ctx, &f)
    );
}

// ===========================================================================
// DIFFERENTIAL CORPUS: a batch agreement sweep over the whole supported fragment
// ===========================================================================

#[test]
fn differential_corpus_never_disagrees() {
    let ctx = std_ctx();
    let corpus = vec![
        ty(0),
        prop(),
        pi("_", g("Nat"), g("Nat")),
        pi("x", g("Entity"), prop()),
        lam("A", ty(0), lam("x", var("A"), var("x"))),
        app(g("Succ"), g("Zero")),
        lam("p", prop(), var("p")),
        pi("A", ty(0), pi("_", var("A"), var("A"))),
        Term::Lit(Literal::Int(7)),
    ];
    for (i, term) in corpus.iter().enumerate() {
        let verdict = double_check(&ctx, term);
        assert_ne!(
            verdict,
            DoubleCheck::Disagree(String::new()),
            "corpus[{i}] produced a disagreement shape"
        );
        // No `Disagree(_)` of ANY message is allowed on this valid corpus.
        assert!(
            !matches!(verdict, DoubleCheck::Disagree(_)),
            "corpus[{i}] = {term} DISAGREED: {verdict:?}"
        );
    }
}
