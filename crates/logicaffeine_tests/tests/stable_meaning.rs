//! THE STABLE-MEANING INVARIANT — a registered global never changes meaning under a declaration
//! that depends on it.
//!
//! Lean's environment is append-only: a name is declared once. LOGOS lets a name be REDECLARED
//! (the REPL is a notebook, and fixtures re-register shapes), which is sound exactly while nothing
//! was checked against the old meaning. Every dependent was type-checked against what the name
//! meant then; change the meaning and the dependent is still certified for a statement that is no
//! longer what it says. Three routes led from that to a closed proof of `False` that both kernels
//! agreed on:
//!
//! * redeclaring a definition a theorem was proved about (`myAdd 1 1 = 2` by `refl`, then
//!   `myAdd := λm n. Zero`, and the theorem inhabits `0 = 2`);
//! * adding a constructor to an inductive a theorem already eliminated exhaustively;
//! * registering a constructor under a name that was still a definition, so `get_global` typed the
//!   name from one map while δ unfolded it from another.
//!
//! The rule, one sentence: a global that anything outside its own declaration unit (the name, its
//! constructors, its derived auxiliaries, its mutual block) depends on may not be redeclared with a
//! different meaning, displaced, or extended. An identical redeclaration changes nothing and stays
//! legal, and a name nothing uses may still be redeclared.

use logicaffeine_kernel::interface::Repl;
use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{
    defeq_for_test, infer_type, normalize, recheck, BigInt, Context, KernelError, Literal,
    MeaningChange, Term, Universe,
};

fn g(n: &str) -> Term {
    Term::Global(n.to_string())
}
fn v(n: &str) -> Term {
    Term::Var(n.to_string())
}
fn app(f: Term, a: Term) -> Term {
    Term::App(Box::new(f), Box::new(a))
}
fn apps(f: Term, args: &[Term]) -> Term {
    args.iter().cloned().fold(f, app)
}
fn lam(p: &str, ty: Term, body: Term) -> Term {
    Term::Lambda {
        param: p.to_string(),
        param_type: Box::new(ty),
        body: Box::new(body),
    }
}
fn pi(p: &str, ty: Term, body: Term) -> Term {
    Term::Pi {
        param: p.to_string(),
        param_type: Box::new(ty),
        body_type: Box::new(body),
    }
}
fn nat() -> Term {
    g("Nat")
}
fn lit(n: u64) -> Term {
    Term::Lit(Literal::Nat(BigInt::from_u64(n)))
}
fn eq(ty: Term, a: Term, b: Term) -> Term {
    apps(g("Eq"), &[ty, a, b])
}
fn refl(ty: Term, a: Term) -> Term {
    apps(g("refl"), &[ty, a])
}
fn ty0() -> Term {
    Term::Sort(Universe::Type(0))
}
fn std_ctx() -> Context {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    ctx
}
fn binary() -> Term {
    pi("m", nat(), pi("n", nat(), nat()))
}
/// `h` ascribed the type `ty` — the conversion a caller would need to use `h` at `ty`.
fn at(h: Term, ty: Term) -> Term {
    Term::Let {
        name: "h".to_string(),
        ty: Box::new(ty),
        value: Box::new(h),
        body: Box::new(v("h")),
    }
}
fn refused_with(result: Result<(), KernelError>, dependent: &str) {
    match result {
        Ok(()) => panic!("a change of meaning under `{dependent}` must be refused"),
        Err(e) => assert!(
            e.to_string().contains(dependent),
            "the refusal must name the declaration that depends on the old meaning \
             (`{dependent}`): {e}"
        ),
    }
}

/// The `myAdd` route to `False`, end to end: the redeclaration is refused, and the theorem keeps
/// saying what it was proved to say.
#[test]
fn a_definition_a_theorem_depends_on_cannot_be_redeclared_with_another_meaning() {
    let mut ctx = std_ctx();
    let real = lam(
        "m",
        nat(),
        lam("n", nat(), apps(g("Nat_add"), &[v("m"), v("n")])),
    );
    ctx.declare("myAdd", binary(), real)
        .expect("an ordinary definition");
    ctx.declare_theorem_checked(
        "thm",
        vec![],
        eq(nat(), apps(g("myAdd"), &[lit(1), lit(1)]), lit(2)),
        refl(nat(), lit(2)),
    )
    .expect("`myAdd 1 1 = 2` holds by computation");

    let constant_zero = lam("m", nat(), lam("n", nat(), g("Zero")));
    refused_with(ctx.declare("myAdd", binary(), constant_zero), "thm");

    assert!(
        defeq_for_test(&ctx, &apps(g("myAdd"), &[lit(1), lit(1)]), &lit(2)),
        "the refused redeclaration left the old meaning in place"
    );
    let zero_is_two = at(g("thm"), eq(nat(), g("Zero"), lit(2)));
    assert!(
        infer_type(&ctx, &zero_is_two).is_err(),
        "`thm` does not inhabit `0 = 2`"
    );
    let falsity = apps(g("nat_zs_ne"), &[lit(1), zero_is_two]);
    assert!(
        infer_type(&ctx, &falsity).is_err(),
        "no closed proof of `False`"
    );
    assert!(
        recheck(&ctx, &falsity).is_err(),
        "…in the second kernel either"
    );
}

/// The notebook convenience survives where it is sound: nothing uses the name, so nothing was
/// checked against its old meaning.
#[test]
fn a_definition_nothing_uses_may_still_be_redeclared() {
    let mut ctx = std_ctx();
    ctx.declare("draft", nat(), lit(2)).expect("a first draft");
    ctx.declare("draft", nat(), g("Zero"))
        .expect("an unused name may be redeclared");
    assert_eq!(
        normalize(&ctx, &g("draft")),
        g("Zero"),
        "the second draft is what `draft` means"
    );
}

/// Redeclaring a used name with EXACTLY its current meaning changes nothing any dependent relied
/// on, so it is accepted — a fixture that re-registers a shape the prelude already has is not a
/// change.
#[test]
fn an_identical_redeclaration_of_a_used_definition_is_accepted_and_changes_nothing() {
    let mut ctx = std_ctx();
    ctx.declare("two", nat(), lit(2)).expect("two");
    ctx.declare("usesTwo", nat(), g("two"))
        .expect("a dependent");
    ctx.declare("two", nat(), lit(2))
        .expect("the identical meaning is not a change");
    assert!(defeq_for_test(&ctx, &g("usesTwo"), &lit(2)));
    refused_with(ctx.declare("two", nat(), g("Zero")), "usesTwo");
    refused_with(ctx.declare("two", ty0(), nat()), "usesTwo");
    assert!(
        defeq_for_test(&ctx, &g("usesTwo"), &lit(2)),
        "…and a real change is still refused"
    );
}

/// The constructor-set route to `False`: a theorem that eliminated `B2` exhaustively when it had
/// one constructor must not meet a second one.
#[test]
fn an_inductive_a_theorem_eliminates_cannot_gain_a_constructor() {
    let mut ctx = std_ctx();
    ctx.add_inductive("B2", ty0());
    ctx.add_constructor_checked("b1", "B2", g("B2"))
        .expect("the first constructor");
    let only_b1 = pi("x", g("B2"), eq(g("B2"), v("x"), g("b1")));
    let by_cases = lam(
        "x",
        g("B2"),
        Term::Match {
            discriminant: Box::new(v("x")),
            motive: Box::new(lam("y", g("B2"), eq(g("B2"), v("y"), g("b1")))),
            cases: vec![refl(g("B2"), g("b1"))],
        },
    );
    ctx.declare_theorem_checked("all_b1", vec![], only_b1, by_cases)
        .expect("with one constructor every `B2` is `b1`");

    refused_with(ctx.add_constructor_checked("b2", "B2", g("B2")), "all_b1");
    assert_eq!(
        ctx.get_constructors("B2")
            .iter()
            .map(|(n, _)| *n)
            .collect::<Vec<_>>(),
        vec!["b1"],
        "the refused constructor registered nothing"
    );
    let b2_is_b1 = app(g("all_b1"), g("b2"));
    assert!(
        infer_type(&ctx, &b2_is_b1).is_err(),
        "`b2` does not exist, so `b2 = b1` has no proof"
    );
}

/// An inductive is built one constructor at a time, and its own recursor and auxiliaries are
/// regenerated after each — those are part of its declaration unit, not dependents. It closes
/// the moment something OUTSIDE the unit is checked against it.
#[test]
fn an_inductive_grows_until_something_outside_its_declaration_depends_on_it() {
    let mut ctx = std_ctx();
    ctx.add_inductive("Hue", ty0());
    for colour in ["red", "green", "blue"] {
        ctx.add_constructor_checked(colour, "Hue", g("Hue"))
            .unwrap_or_else(|e| panic!("`{colour}` extends an inductive nothing uses yet: {e}"));
    }
    assert!(
        ctx.get_global("Hue_rec").is_some(),
        "its own recursor was regenerated along the way"
    );

    ctx.declare("favourite", g("Hue"), g("green"))
        .expect("a dependent outside the unit");
    refused_with(
        ctx.add_constructor_checked("violet", "Hue", g("Hue")),
        "favourite",
    );
    assert_eq!(
        ctx.get_constructors("Hue").len(),
        3,
        "Hue kept exactly its three constructors"
    );
}

/// A structure's η rule (`s ≡ mk s.1 … s.n`) is sound only because it has one constructor, so a
/// structure is closed from the moment it exists.
#[test]
fn a_structure_never_gains_a_second_constructor() {
    let mut ctx = std_ctx();
    ctx.add_structure("Cellish", &[("A", ty0())], &[("val", v("A"))]);
    assert!(
        ctx.refused_declarations().is_empty(),
        "{:?}",
        ctx.refused_declarations()
    );
    let second = pi("A", ty0(), app(g("Cellish"), v("A")));
    let refusal = ctx
        .add_constructor_checked("Cellish_other", "Cellish", second)
        .expect_err("a structure has exactly one constructor");
    assert!(refusal.to_string().contains("structure"), "{refusal}");
    assert_eq!(
        ctx.get_constructors("Cellish").len(),
        1,
        "…and it still has exactly one"
    );
}

/// A constructor must never share its name with a live definition: `get_global` would type the
/// name from the constructor while δ unfolded the definition's body.
#[test]
fn a_constructor_never_shares_its_name_with_a_live_definition() {
    let mut ctx = std_ctx();
    ctx.add_inductive("Tok", ty0());

    // `Nat_add` is used by `Nat_mul` (and much else): refused, and it still computes.
    assert!(
        ctx.add_constructor_checked("Nat_add", "Tok", g("Tok"))
            .is_err(),
        "a constructor may not take the name of a definition other declarations use"
    );
    assert!(
        ctx.get_definition_body("Nat_add").is_some(),
        "`Nat_add` is still the definition"
    );
    assert!(defeq_for_test(
        &ctx,
        &apps(g("Nat_mul"), &[lit(2), lit(3)]),
        &lit(6)
    ));

    // A definition nothing uses is DISPLACED: the name then means the constructor, in typing
    // and in reduction alike.
    ctx.declare("lonely", nat(), g("Zero"))
        .expect("an unused definition");
    ctx.add_constructor_checked("lonely", "Tok", g("Tok"))
        .expect("the unused name is free to reuse");
    assert!(
        ctx.get_definition_body("lonely").is_none(),
        "no body survives under a constructor"
    );
    assert_eq!(infer_type(&ctx, &g("lonely")).unwrap(), g("Tok"));
    assert_eq!(
        normalize(&ctx, &g("lonely")),
        g("lonely"),
        "a constructor does not δ-unfold"
    );
}

/// The unchecked doors keep the invariant too: they cannot return an error, so a refused change
/// is recorded in the refusal ledger and the old meaning stays.
#[test]
fn the_unchecked_doors_record_a_refused_change_of_meaning() {
    let mut ctx = std_ctx();
    ctx.add_declaration("assumed", nat());
    ctx.declare("usesAssumed", nat(), g("assumed"))
        .expect("a dependent of the axiom");
    ctx.add_declaration("assumed", g("Bool"));
    assert_eq!(
        ctx.get_global("assumed"),
        Some(&nat()),
        "the axiom still says `Nat`"
    );
    assert!(
        ctx.refused_declarations()
            .iter()
            .any(|(n, why)| n == "assumed" && why.contains("usesAssumed")),
        "the refusal is recorded with the dependent it protects: {:?}",
        ctx.refused_declarations()
    );

    // Re-declaring the prelude's `Nat` header at its own sort changes nothing: it is kept, with
    // `Zero`/`Succ`, and nothing is refused.
    let before = ctx.refused_declarations().len();
    ctx.add_inductive("Nat", ty0());
    assert_eq!(
        ctx.get_constructors("Nat")
            .iter()
            .map(|(n, _)| *n)
            .collect::<Vec<_>>(),
        vec!["Zero", "Succ"],
        "the prelude's `Nat` keeps its constructors"
    );
    assert_eq!(
        ctx.refused_declarations().len(),
        before,
        "an identical header is not a change"
    );

    // At another sort it would displace `Zero`/`Succ` under everything built on them.
    ctx.add_inductive("Nat", Term::Sort(Universe::Type(1)));
    assert_eq!(
        ctx.get_global("Nat"),
        Some(&ty0()),
        "`Nat` still lives in `Type 0`"
    );
    assert_eq!(
        ctx.get_constructors("Nat").len(),
        2,
        "…with both its constructors"
    );
    assert!(
        ctx.refused_declarations()
            .iter()
            .any(|(n, why)| n == "Nat" && why.contains("redeclared")),
        "{:?}",
        ctx.refused_declarations()
    );
}

/// The REPL is the notebook the redeclaration convenience exists for, and the route a user
/// reaches it by. A redefinition under a dependent is not a change of meaning there: it is
/// registered under a fresh identity (ruling of 2026-09-24), and the dependent keeps the `two` it
/// was checked against.
#[test]
fn the_repl_refuses_to_redefine_a_name_a_later_definition_uses() {
    let mut repl = Repl::new();
    repl.execute("Definition two : Nat := Succ (Succ Zero).")
        .expect("two");
    repl.execute("Definition usesTwo : Nat := two.")
        .expect("a dependent");
    repl.execute("Definition two : Nat := Zero.")
        .expect("redefining `two` under `usesTwo` shadows it");
    assert!(
        repl.shadows().iter().any(|(written, _)| written == "two"),
        "{:?}",
        repl.shadows()
    );
    assert_eq!(
        repl.context().get_definition_body("two"),
        Some(&app(g("Succ"), app(g("Succ"), g("Zero")))),
        "the `two` `usesTwo` was checked against keeps its meaning at its key"
    );
    assert_eq!(
        repl.execute("Eval usesTwo.").unwrap().trim(),
        "(Succ (Succ Zero))"
    );

    repl.execute("Definition scratch : Nat := Zero.")
        .expect("scratch");
    repl.execute("Definition scratch : Nat := Succ Zero.")
        .expect("an unused name may be redefined");
    assert_eq!(repl.execute("Eval scratch.").unwrap().trim(), "(Succ Zero)");
}

/// The invariant costs the standard library nothing: it registers without a single refusal.
#[test]
fn the_prelude_registers_without_a_refused_change_of_meaning() {
    let ctx = std_ctx();
    assert!(
        ctx.refused_declarations().is_empty(),
        "the prelude must not trip its own invariant: {:?}",
        ctx.refused_declarations()
    );
}

/// The refusal is its own TYPED error, carrying the name, the kind of change and EVERY dependent,
/// sorted and named by the declaration it belongs to (`useA`'s derived `useA_eq_def` is reported as
/// `useA`) — so a caller tells a stable-meaning refusal from an ill-typed body without reading
/// prose, and the prose names what the change would have broken in the words its author wrote.
#[test]
fn the_refusal_is_a_typed_error_naming_the_change_and_every_dependent() {
    let mut ctx = std_ctx();
    ctx.declare("base", nat(), lit(1)).expect("base");
    for user in ["useD", "useA", "useC", "useB"] {
        ctx.declare(user, nat(), g("base"))
            .expect("a dependent of `base`");
    }
    match ctx.declare("base", nat(), lit(2)) {
        Err(KernelError::StableMeaningViolation {
            name,
            change,
            dependents,
        }) => {
            assert_eq!(name, "base");
            assert_eq!(change, MeaningChange::Redeclared);
            assert_eq!(
                dependents,
                ["useA", "useB", "useC", "useD"],
                "every dependent, by declaration, sorted"
            );
        }
        other => panic!("a redeclaration under dependents is a stable-meaning refusal: {other:?}"),
    }
    let message = ctx.declare("base", nat(), lit(2)).unwrap_err().to_string();
    assert!(
        message.contains("`base` cannot be redeclared with a different meaning"),
        "{message}"
    );
    assert!(
        message
            .contains("`useA`, `useB`, `useC` and 1 more were checked against its current meaning"),
        "{message}"
    );
    assert!(
        message.contains("declare a new name instead"),
        "the message says what to do: {message}"
    );

    // A constructor added to an inductive something observes is an EXTENSION.
    ctx.add_inductive("Two", ty0());
    ctx.add_constructor_checked("one", "Two", g("Two"))
        .expect("the first constructor");
    ctx.declare("pick", g("Two"), g("one"))
        .expect("a dependent of `Two`");
    match ctx.add_constructor_checked("other", "Two", g("Two")) {
        Err(KernelError::StableMeaningViolation {
            name,
            change,
            dependents,
        }) => {
            assert_eq!((name.as_str(), change), ("Two", MeaningChange::Extended));
            assert_eq!(dependents, ["pick"]);
        }
        other => panic!("extending an observed inductive is a stable-meaning refusal: {other:?}"),
    }
    let message = ctx
        .add_constructor_checked("other", "Two", g("Two"))
        .unwrap_err()
        .to_string();
    assert!(
        message.contains("`Two` cannot be extended with a constructor: `pick` was checked"),
        "{message}"
    );

    // A constructor taking a used definition's name REDECLARES that name.
    match ctx.add_constructor_checked("base", "Two", g("Two")) {
        Err(KernelError::StableMeaningViolation {
            name,
            change: MeaningChange::Redeclared,
            dependents,
        }) => {
            assert_eq!(name, "base");
            assert_eq!(dependents.len(), 4);
        }
        other => panic!("displacing a used definition is a stable-meaning refusal: {other:?}"),
    }

    // The doors that cannot answer record the same refusal, word for word.
    ctx.add_definition("base".to_string(), nat(), lit(3));
    ctx.add_inductive("Two", Term::Sort(Universe::Type(1)));
    let recorded: Vec<&str> = ctx
        .refused_declarations()
        .iter()
        .map(|(_, why)| why.as_str())
        .collect();
    assert!(
        recorded
            .iter()
            .any(|why| why.starts_with("Stable meaning violation: `base` cannot be redeclared")),
        "{recorded:?}"
    );
    assert!(
        recorded
            .iter()
            .any(|why| why.starts_with("Stable meaning violation: `Two` cannot be redeclared")),
        "{recorded:?}"
    );

    // …while a structure's second constructor is refused for a different reason, typed as such.
    ctx.add_structure("Boxy", &[("A", ty0())], &[("val", v("A"))]);
    let second = pi("A", ty0(), app(g("Boxy"), v("A")));
    assert!(
        matches!(
            ctx.add_constructor_checked("Boxy_other", "Boxy", second),
            Err(KernelError::InvalidDeclaration(_))
        ),
        "a structure has exactly one constructor whatever depends on it"
    );

    // The REPL registers a redefinition under a dependent at a FRESH IDENTITY instead of refusing
    // it (ruling of 2026-09-24): the gate's refusal is what tells it to, and the dependent's `two`
    // keeps its meaning.
    let mut repl = Repl::new();
    repl.execute("Definition two : Nat := Succ (Succ Zero).")
        .expect("two");
    repl.execute("Definition usesTwo : Nat := two.")
        .expect("a dependent");
    repl.execute("Definition two : Nat := Zero.")
        .expect("the redefinition shadows `two`");
    assert!(
        repl.shadows().iter().any(|(written, _)| written == "two"),
        "{:?}",
        repl.shadows()
    );
    assert_eq!(
        repl.execute("Eval usesTwo.").unwrap().trim(),
        "(Succ (Succ Zero))"
    );
    // …and so does its `Inductive` command. The identical declaration is kept; a declaration at
    // the same header with a constructor `Hue2` lacks would EXTEND it under `favHue`, so it is a
    // new `Hue2`.
    repl.execute("Inductive Hue2 := hr : Hue2 | hg : Hue2.")
        .expect("Hue2");
    repl.execute("Definition favHue : Hue2 := hr.")
        .expect("a dependent");
    repl.execute("Inductive Hue2 := hr : Hue2 | hg : Hue2.")
        .expect("the identical declaration is kept");
    repl.execute("Inductive Hue2 := hb : Hue2.")
        .expect("the different `Hue2` shadows the first");
    assert!(
        repl.shadows().iter().any(|(written, _)| written == "Hue2"),
        "{:?}",
        repl.shadows()
    );
    assert_eq!(
        repl.context()
            .get_constructors("Hue2")
            .iter()
            .map(|(c, _)| *c)
            .collect::<Vec<_>>(),
        ["hr", "hg"],
        "the first `Hue2` keeps its constructors under `favHue`"
    );
}
