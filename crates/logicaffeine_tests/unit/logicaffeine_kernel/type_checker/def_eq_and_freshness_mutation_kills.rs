//! The structural-equality core of the kernel (`def_eq_normalized`, `types_equal`) and
//! the capture-avoidance machinery (`fresh_name`, `freshen`, `all_var_names`) are the most
//! safety-critical code in the system — a `def_eq` bug means UNSOUND PROOFS. These tests
//! assert exhaustively that EQUAL terms of every shape are definitionally equal, and that a
//! difference in ANY subterm makes them unequal, killing the `delete match arm`, `&& -> ||`,
//! and `== -> !=` mutants cargo-mutants surfaced on `type_checker.rs` (Wave 4).
use super::*;
use crate::context::Context;
use crate::prelude::StandardLibrary;
use crate::term::{Literal, Term, Universe};
use std::collections::HashSet;

fn ctx() -> Context {
    let mut c = Context::new();
    StandardLibrary::register(&mut c);
    c
}
fn sort(n: u32) -> Term {
    Term::Sort(Universe::Type(n))
}
fn var(s: &str) -> Term {
    Term::Var(s.to_string())
}
fn g(s: &str) -> Term {
    Term::Global(s.to_string())
}
fn app(f: Term, x: Term) -> Term {
    Term::App(Box::new(f), Box::new(x))
}
fn pi(pt: Term, bt: Term) -> Term {
    Term::Pi {
        param: "x".to_string(),
        param_type: Box::new(pt),
        body_type: Box::new(bt),
    }
}
fn lam(pt: Term, body: Term) -> Term {
    Term::Lambda {
        param: "x".to_string(),
        param_type: Box::new(pt),
        body: Box::new(body),
    }
}
fn nat(n: u64) -> Term {
    Term::Lit(Literal::Nat(logicaffeine_base::BigInt::from_u64(n)))
}
fn set(items: &[&str]) -> HashSet<String> {
    items.iter().map(|s| s.to_string()).collect()
}

/// Equal terms of EVERY shape must be definitionally equal — kills every `delete match arm`
/// mutant in `def_eq_normalized` (deleting an arm makes equal terms fall through to `false`).
#[test]
fn def_eq_holds_for_equal_terms_of_every_shape() {
    let c = ctx();
    assert!(def_eq_normalized(&c, &sort(0), &sort(0)), "Sort ≡ Sort");
    assert!(
        def_eq_normalized(&c, &pi(sort(0), sort(1)), &pi(sort(0), sort(1))),
        "Pi ≡ Pi"
    );
    assert!(
        def_eq_normalized(&c, &lam(sort(0), var("x")), &lam(sort(0), var("x"))),
        "Lambda ≡ Lambda"
    );
    assert!(
        def_eq_normalized(&c, &app(g("f"), g("a")), &app(g("f"), g("a"))),
        "App ≡ App"
    );
    assert!(def_eq_normalized(&c, &nat(3), &nat(3)), "Nat(3) ≡ Nat(3)");
}

/// A difference in ANY subterm must break definitional equality — kills every `&& -> ||`
/// (which would make `def_eq` too permissive) and `== -> !=` mutant in `def_eq_normalized`.
#[test]
fn def_eq_fails_when_any_subterm_differs() {
    let c = ctx();
    assert!(
        !def_eq_normalized(&c, &sort(0), &sort(1)),
        "Type 0 ≢ Type 1"
    );
    assert!(
        !def_eq_normalized(&c, &pi(sort(0), sort(0)), &pi(sort(0), sort(1))),
        "Pi body differs"
    );
    assert!(
        !def_eq_normalized(&c, &lam(sort(0), sort(0)), &lam(sort(0), sort(1))),
        "Lambda body differs"
    );
    assert!(
        !def_eq_normalized(&c, &app(g("f"), g("a")), &app(g("f"), g("b"))),
        "App arg differs"
    );
    assert!(!def_eq_normalized(&c, &nat(3), &nat(4)), "Nat(3) ≢ Nat(4)");
}

/// Same exhaustive structural congruence for `types_equal` (the no-context syntactic equality).
#[test]
fn types_equal_is_exact_structural_congruence() {
    assert!(types_equal(&sort(0), &sort(0)), "Sort == Sort");
    assert!(
        types_equal(&pi(sort(0), sort(1)), &pi(sort(0), sort(1))),
        "Pi == Pi"
    );
    assert!(
        types_equal(&lam(sort(0), var("x")), &lam(sort(0), var("x"))),
        "Lambda == Lambda"
    );
    assert!(
        types_equal(&app(g("f"), g("a")), &app(g("f"), g("a"))),
        "App == App"
    );
    assert!(!types_equal(&sort(0), &sort(1)), "Type 0 != Type 1");
    assert!(
        !types_equal(&pi(sort(0), sort(0)), &pi(sort(0), sort(1))),
        "Pi body differs"
    );
    assert!(
        !types_equal(&lam(sort(0), sort(0)), &lam(sort(0), sort(1))),
        "Lambda body differs"
    );
    assert!(
        !types_equal(&app(g("f"), g("a")), &app(g("f"), g("b"))),
        "App arg differs"
    );
}

/// `fresh_name` must avoid collisions and INCREMENT correctly — kills the constant-return
/// (`String::new()`, `"xyzzy"`) and counter-arithmetic (`+= -> *=` gives `x'0`; `+= -> -=`
/// underflows) mutants.
#[test]
fn fresh_name_avoids_collisions_and_increments() {
    assert_eq!(fresh_name("x", &set(&[])), "x'", "no collision → base'");
    assert_eq!(fresh_name("x", &set(&["x'"])), "x'1", "one collision → +1");
    assert_eq!(
        fresh_name("x", &set(&["x'", "x'1"])),
        "x'2",
        "two collisions → +2"
    );
}

/// `freshen` must avoid every name occurring in the body — which only `all_var_names`
/// surfaces. Kills `all_var_names -> ()` (which would leave the avoid set incomplete, so
/// `freshen` returns the colliding `x'`).
#[test]
fn freshen_avoids_names_occurring_in_the_body() {
    let body = var("x'"); // occurs in body, but not in fvs or the substituted var
    let name = freshen("x", &body, &set(&[]), "z");
    assert_ne!(
        name, "x'",
        "freshen must avoid names occurring in the body (via all_var_names)"
    );
    assert_eq!(name, "x'1");
}

fn pi_named(p: &str, pt: Term, bt: Term) -> Term {
    Term::Pi {
        param: p.to_string(),
        param_type: Box::new(pt),
        body_type: Box::new(bt),
    }
}
fn lam_named(p: &str, pt: Term, body: Term) -> Term {
    Term::Lambda {
        param: p.to_string(),
        param_type: Box::new(pt),
        body: Box::new(body),
    }
}
fn fix_named(n: &str, body: Term) -> Term {
    Term::Fix {
        name: n.to_string(),
        body: Box::new(body),
    }
}
fn mtch(d: Term, m: Term, cases: Vec<Term>) -> Term {
    Term::Match {
        discriminant: Box::new(d),
        motive: Box::new(m),
        cases,
    }
}

/// ALPHA-EQUIVALENT binders (different names, same structure) are definitionally equal via
/// the CONGRUENCE arm — the syntactic fast-path does NOT match them, so this is what
/// actually exercises (and kills the `delete match arm` mutant for) the Pi/Lambda/Fix arms.
#[test]
fn def_eq_holds_for_alpha_equivalent_binders() {
    let c = ctx();
    assert!(
        def_eq_normalized(
            &c,
            &pi_named("x", sort(0), var("x")),
            &pi_named("y", sort(0), var("y"))
        ),
        "α-Pi"
    );
    assert!(
        def_eq_normalized(
            &c,
            &lam_named("x", sort(0), var("x")),
            &lam_named("y", sort(0), var("y"))
        ),
        "α-Lambda"
    );
    assert!(
        def_eq_normalized(&c, &fix_named("x", var("x")), &fix_named("y", var("y"))),
        "α-Fix"
    );
}

/// Match/Fix congruence: a differing subterm breaks equality — kills the `&& -> ||`,
/// `== -> !=`, and `delete match arm (Match/Fix)` mutants in both def_eq and types_equal.
#[test]
fn def_eq_and_types_equal_distinguish_match_and_fix() {
    let c = ctx();
    let m1 = mtch(g("h1"), g("M"), vec![]);
    let m2 = mtch(g("h2"), g("M"), vec![]);
    assert!(def_eq_normalized(&c, &m1, &m1.clone()), "Match ≡ itself");
    assert!(
        !def_eq_normalized(&c, &m1, &m2),
        "Match discriminant differs"
    );
    assert!(
        !types_equal(&m1, &m2),
        "types_equal: Match discriminant differs"
    );
    assert!(
        types_equal(&fix_named("f", g("a")), &fix_named("f", g("a"))),
        "Fix ≡ Fix"
    );
    assert!(
        !types_equal(&fix_named("f", g("a")), &fix_named("f", g("b"))),
        "Fix body differs"
    );
}

/// `occurs_free` is binder-aware: a variable BOUND by an enclosing Pi/Lambda/Fix/Let does
/// NOT occur free. Kills `occurs_free -> true` and every `&& -> ||` / `!= -> ==` mutant that
/// would wrongly report a bound variable as free (a capture bug → unsound substitution).
#[test]
fn occurs_free_respects_binders() {
    assert!(occurs_free(&var("z"), "z"), "free z occurs");
    assert!(!occurs_free(&var("y"), "z"), "y is not z");
    assert!(!occurs_free(&sort(0), "z"), "closed term: no free z");
    assert!(
        !occurs_free(&pi_named("z", sort(0), var("z")), "z"),
        "Pi binds z"
    );
    assert!(
        !occurs_free(&lam_named("z", sort(0), var("z")), "z"),
        "Lambda binds z"
    );
    assert!(!occurs_free(&fix_named("z", var("z")), "z"), "Fix binds z");
    assert!(
        !occurs_free(
            &Term::Let {
                name: "z".into(),
                ty: Box::new(sort(0)),
                value: Box::new(g("v")),
                body: Box::new(var("z"))
            },
            "z"
        ),
        "Let binds z in its body"
    );
    assert!(
        occurs_free(&pi_named("x", sort(0), var("z")), "z"),
        "z free under a different binder"
    );
}

/// `beta_reduce` contracts exactly `(λ.body) arg` and leaves non-redexes alone — kills the
/// `delete match arm (App)` and `delete match arm (Lambda)` mutants.
#[test]
fn beta_reduce_contracts_redexes_only() {
    let redex = app(lam_named("x", sort(0), var("x")), g("a"));
    assert!(types_equal(&beta_reduce(&redex), &g("a")), "(λx.x) a → a");
    let stuck = app(g("f"), g("a"));
    assert!(
        types_equal(&beta_reduce(&stuck), &stuck),
        "non-redex unchanged"
    );
}

/// `Hole` is a type wildcard that matches anything (kills `|| -> &&` in the Hole guard),
/// and a Match with a different number of cases is not equal (kills `== -> !=` on arity).
#[test]
fn types_equal_hole_wildcard_and_match_arity() {
    assert!(types_equal(&Term::Hole, &sort(0)), "Hole matches Sort");
    assert!(types_equal(&sort(0), &Term::Hole), "Sort matches Hole");
    let m1 = mtch(g("h"), g("M"), vec![g("c")]);
    let m2 = mtch(g("h"), g("M"), vec![g("c"), g("d")]);
    assert!(!types_equal(&m1, &m2), "Match case-arity differs");
}

/// A `MutualFix` binds ALL of its def names, so a variable equal to a def name is NOT free
/// (kills the `&& -> ||` mutant that would report a bound mutual-recursion name as free).
#[test]
fn occurs_free_mutual_fix_binds_all_def_names() {
    let bound = Term::MutualFix {
        defs: vec![("z".to_string(), var("z"))],
        index: 0,
    };
    assert!(!occurs_free(&bound, "z"), "MutualFix binds its def name z");
    let free = Term::MutualFix {
        defs: vec![("f".to_string(), var("z"))],
        index: 0,
    };
    assert!(
        occurs_free(&free, "z"),
        "z is free in a MutualFix body under a different name"
    );
}

/// A `MutualFix` with an out-of-range index must be rejected by the validity guard with its
/// SPECIFIC "out-of-range" diagnostic — kills the `|| -> &&` mutant. Asserting the exact
/// message (not merely `is_err`) is what makes this robust: under `&&` the guard is skipped, so
/// the only way to reach this diagnostic is gone — the mutant instead falls through to a
/// different error or an out-of-bounds panic, both of which fail this assertion.
#[test]
fn infer_type_rejects_malformed_mutual_fix() {
    let c = ctx();
    let good_def = Term::Lambda {
        param: "n".into(),
        param_type: Box::new(sort(0)),
        body: Box::new(var("n")),
    };
    let oob = Term::MutualFix {
        defs: vec![("f".to_string(), good_def)],
        index: 5,
    };
    let got = format!("{:?}", infer_type(&c, &oob));
    assert!(
        got.contains("out-of-range"),
        "an out-of-range MutualFix index must be rejected by the guard's diagnostic, got: {got}",
    );
    // The empty-block arm of the same guard (does not distinguish `||`/`&&`, both reject it).
    let empty = Term::MutualFix {
        defs: vec![],
        index: 0,
    };
    assert!(
        infer_type(&c, &empty).is_err(),
        "empty MutualFix must be rejected"
    );
}

/// `extract_inductive_name` must return `None` for a Global that is NOT a registered
/// inductive — kills the `ctx.is_inductive(name) -> true` guard mutant.
#[test]
fn extract_inductive_name_rejects_non_inductive_globals() {
    let c = ctx();
    assert_eq!(
        extract_inductive_name(&c, &g("definitely_not_an_inductive_xyzzy")),
        None
    );
}

/// `check_type`'s bidirectional (placeholder-`_`-param) path must α-rename the expected Pi's
/// binder to the lambda's param name before recursing. The outer lambda `λ(A:_). (λ(y:A). y)`
/// takes its param type from the expected `Π(B:Type0). Π(_:B). B`; the CONCRETE inner body
/// `λ(y:A).y` (non-placeholder → flows through `infer_type` + `is_subtype`) then has type
/// `Π(y:A).A`, which is a subtype of the expected body ONLY after `Π(_:B).B` is renamed to
/// `Π(_:A).A`. Kills `param != expected_param` -> `==` (which skips the rename → the inner body
/// is checked against `Π(_:B).B` → the contravariant domain `B ≰ A` fails).
#[test]
fn check_type_renames_lambda_binder_to_the_expected_pi_binder() {
    let c = ctx();
    // Concrete (non-placeholder) inner body so it goes through infer_type + is_subtype.
    let inner_lam = Term::Lambda {
        param: "y".into(),
        param_type: Box::new(var("A")),
        body: Box::new(var("y")),
    };
    // Outer param type is the placeholder `_`, which triggers the bidirectional branch (494).
    let outer_lam = Term::Lambda {
        param: "A".into(),
        param_type: Box::new(g("_")),
        body: Box::new(inner_lam),
    };
    let inner_pi = Term::Pi {
        param: "_".into(),
        param_type: Box::new(var("B")),
        body_type: Box::new(var("B")),
    };
    let expected = Term::Pi {
        param: "B".into(),
        param_type: Box::new(sort(0)),
        body_type: Box::new(inner_pi),
    };
    assert!(
        check_type(&c, &outer_lam, &expected).is_ok(),
        "the bidirectional check must α-rename the expected binder before checking the body",
    );
}

/// `substitute_avoiding` must respect let-shadowing: in `let x = _ in x`, the binder `x`
/// shadows the substitution target `x`, so the body occurrence is LEFT ALONE. Kills
/// `if name == var` -> `!=` (which would substitute into the shadowed body).
#[test]
fn substitute_avoiding_respects_let_shadowing() {
    let let_term = Term::Let {
        name: "x".into(),
        ty: Box::new(sort(0)),
        value: Box::new(sort(0)),
        body: Box::new(var("x")),
    };
    let result = substitute_avoiding(&let_term, "x", &var("y"), &set(&["y"]));
    match &result {
        Term::Let { body, .. } => assert_eq!(
            **body,
            var("x"),
            "the let-shadowed body occurrence of x must not be substituted away",
        ),
        other => panic!("expected a Let, got {other:?}"),
    }
}

/// Pi subtyping needs BOTH a contravariant domain and a covariant codomain — kills the
/// `&& -> ||` mutant (which would accept on the codomain alone). `Π(Type0).Type0` is NOT a
/// subtype of `Π(Type1).Type0` because the domain requires `Type1 ≤ Type0` (false, `1 ≰ 0`),
/// even though the codomains match; the positive control confirms subtyping fires when both
/// directions hold.
#[test]
fn is_subtype_pi_requires_both_domain_and_codomain() {
    let c = ctx();
    let pi_named = |pt: Term, bt: Term| Term::Pi {
        param: "_".to_string(),
        param_type: Box::new(pt),
        body_type: Box::new(bt),
    };
    assert!(
        !is_subtype_normalized(&c, &pi_named(sort(0), sort(0)), &pi_named(sort(1), sort(0))),
        "Π(Type0).Type0 must NOT subtype Π(Type1).Type0 (domain 1 ≰ 0)",
    );
    assert!(
        is_subtype_normalized(&c, &pi_named(sort(1), sort(0)), &pi_named(sort(0), sort(1))),
        "Π(Type1).Type0 must subtype Π(Type0).Type1 (0 ≤ 1 in both positions)",
    );
}

/// `is_subsingleton_prop` must return FALSE for an inductive whose single constructor stores
/// NON-propositional data beyond the inductive's arity — kills the `i += 1` -> `i *= 1` mutant
/// (which freezes the field counter at 0, so `i >= arity` never fires, every field is skipped,
/// and a data-carrying type is wrongly reported as a subsingleton).
#[test]
fn is_subsingleton_prop_rejects_a_constructor_storing_non_prop_data() {
    let mut c = ctx();
    // Box : Type0 -> Prop  (arity 1 — one index).
    c.add_inductive(
        "Box",
        Term::Pi {
            param: "_".into(),
            param_type: Box::new(sort(0)),
            body_type: Box::new(Term::Sort(Universe::Prop)),
        },
    );
    // mkBox : Π(A:Type0). Π(x:A). Box A  — the field `x:A` is data (A : Type0 is not Prop).
    let ctor = Term::Pi {
        param: "A".into(),
        param_type: Box::new(sort(0)),
        body_type: Box::new(Term::Pi {
            param: "x".into(),
            param_type: Box::new(var("A")),
            body_type: Box::new(app(g("Box"), var("A"))),
        }),
    };
    c.add_constructor("mkBox", "Box", ctor);
    assert!(
        !is_subsingleton_prop(&c, "Box").unwrap(),
        "Box stores non-Prop data `x:A` beyond its arity — it is not a subsingleton",
    );
}

/// `check_constructor_universes` must REJECT a constructor that stores a field in a universe
/// exceeding the inductive's own sort — kills the `i += 1` -> `i *= 1` mutant (which freezes the
/// parameter counter at 0, so `i >= num_params` never fires and the oversized field is never
/// checked against the target universe).
#[test]
fn check_constructor_universes_rejects_an_oversized_stored_field() {
    let mut c = ctx();
    // Wrap : Type0 -> Type0  with 1 uniform parameter; the inductive lives in Type0.
    c.add_indexed_inductive(
        "Wrap",
        Term::Pi {
            param: "_".into(),
            param_type: Box::new(sort(0)),
            body_type: Box::new(sort(0)),
        },
        1,
    );
    // mkWrap : Π(A:Type0). Π(big:Type0). Wrap A  — the field `big:Type0` has sort Type1,
    // which exceeds the inductive's Type0.
    let ctor = Term::Pi {
        param: "A".into(),
        param_type: Box::new(sort(0)),
        body_type: Box::new(Term::Pi {
            param: "big".into(),
            param_type: Box::new(sort(0)),
            body_type: Box::new(app(g("Wrap"), var("A"))),
        }),
    };
    assert!(
        check_constructor_universes(&c, "Wrap", "mkWrap", &ctor).is_err(),
        "a Type0-sorted (⇒ Type1) field stored in a Type0 inductive is universe-inconsistent",
    );
}

/// Structure eta (`S_mk P (S_f P y) ≡ y`) fires only when the constructor is FULLY applied —
/// `args.len() == num_params + nfields`. Kills `num_params + nfields` -> `num_params * nfields`
/// (which for a 1-param / 1-field record computes 1 instead of 2, rejects the fully-applied
/// term, and eta never fires so the two sides are wrongly reported unequal).
#[test]
fn structure_eta_uses_field_count_sum_not_product() {
    let mut c = ctx();
    // record S (A : Type0) { f : A }  — num_params = 1, nfields = 1  (sum 2 ≠ product 1).
    c.add_structure("S", &[("A", sort(0))], &[("f", var("A"))]);
    let p = var("P");
    let y = var("y");
    let eta = app(
        app(g("S_mk"), p.clone()),
        app(app(g("S_f"), p.clone()), y.clone()),
    );
    assert!(
        def_eq_normalized(&c, &eta, &y),
        "S_mk P (S_f P y) must be definitionally equal to y by structure eta",
    );
}
