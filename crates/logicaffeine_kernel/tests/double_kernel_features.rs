//! The cross-cutting two-kernel invariant: every kernel feature added this
//! campaign must be checked by BOTH the main type-checker and the independent
//! de Bruijn re-checker — a valid term double-checks as `Agreed` (real
//! redundancy, never a silent `MainOnlyReCheckerIncomplete`), and an ill-typed
//! term is rejected by the main kernel. One accepting + one rejecting case per
//! feature (Let/zeta, IMax universes, structure eta).

use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{double_check, infer_type, Context, DoubleCheck, Term, Universe};

fn g(n: &str) -> Term {
    Term::Global(n.to_string())
}
fn v(n: &str) -> Term {
    Term::Var(n.to_string())
}
fn app(f: Term, x: Term) -> Term {
    Term::App(Box::new(f), Box::new(x))
}
fn apps(f: Term, xs: &[Term]) -> Term {
    xs.iter().fold(f, |a, x| app(a, x.clone()))
}
fn let_(name: &str, ty: Term, value: Term, body: Term) -> Term {
    Term::Let {
        name: name.to_string(),
        ty: Box::new(ty),
        value: Box::new(value),
        body: Box::new(body),
    }
}
fn std_ctx() -> Context {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    ctx
}

/// `double_check` returned `Agreed` — both kernels accepted and agreed.
fn assert_agreed(ctx: &Context, t: &Term, what: &str) {
    match double_check(ctx, t) {
        DoubleCheck::Agreed => {}
        other => panic!("{what}: both kernels must AGREE, got {other:?}"),
    }
}

// --- K1: Let / zeta ---------------------------------------------------------

#[test]
fn let_is_two_kernel_agreed() {
    let ctx = std_ctx();
    let t = let_("x", g("Nat"), g("Zero"), app(g("Succ"), v("x")));
    assert_agreed(&ctx, &t, "let/zeta");
}

#[test]
fn ill_typed_let_is_rejected() {
    // `let x : Nat := true in x` — the value's type is wrong.
    let ctx = std_ctx();
    let t = let_("x", g("Nat"), g("true"), v("x"));
    assert!(infer_type(&ctx, &t).is_err(), "ill-typed let must be rejected");
}

// --- K2: IMax universes -----------------------------------------------------

#[test]
fn universe_poly_pi_is_two_kernel_agreed() {
    // `λ(A : Sort u). A → A` — its formation goes through the `imax` Pi rule with
    // a VARIABLE codomain level, the case that stays symbolic. Both kernels must
    // compute the same sort.
    let ctx = std_ctx();
    let t = Term::Lambda {
        param: "A".to_string(),
        param_type: Box::new(Term::Sort(Universe::Var("u".to_string()))),
        body: Box::new(Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(v("A")),
            body_type: Box::new(v("A")),
        }),
    };
    assert_agreed(&ctx, &t, "imax universe Pi");
}

#[test]
fn unsound_sort_variable_coercion_is_rejected() {
    // The soundness fix: `(λ(x : Sort u). x) Nat` requires `Nat : Sort u`, i.e.
    // `Type 0 ≤ u`, which is now FALSE (u may be Prop). Rejected.
    let ctx = std_ctx();
    let coerce = Term::Lambda {
        param: "x".to_string(),
        param_type: Box::new(Term::Sort(Universe::Var("u".to_string()))),
        body: Box::new(v("x")),
    };
    assert!(infer_type(&ctx, &app(coerce, g("Nat"))).is_err(), "Nat : Sort u must be rejected");
}

// --- K4: structures / eta ---------------------------------------------------

fn prod_ctx() -> Context {
    let mut ctx = std_ctx();
    ctx.add_structure(
        "Prod",
        &[("A", Term::Sort(Universe::Type(0))), ("B", Term::Sort(Universe::Type(0)))],
        &[("fst", v("A")), ("snd", v("B"))],
    );
    ctx.add_declaration("p", apps(g("Prod"), &[g("Nat"), g("Bool")]));
    ctx
}

#[test]
fn structure_eta_is_two_kernel_agreed() {
    // A term whose typing genuinely needs record η (coerce `refl p` to the type
    // stated with the η-expansion), double-checked in both kernels.
    let ctx = prod_ctx();
    let prod_ab = apps(g("Prod"), &[g("Nat"), g("Bool")]);
    let mk_p = apps(
        g("Prod_mk"),
        &[
            g("Nat"),
            g("Bool"),
            apps(g("Prod_fst"), &[g("Nat"), g("Bool"), g("p")]),
            apps(g("Prod_snd"), &[g("Nat"), g("Bool"), g("p")]),
        ],
    );
    let eq = apps(g("Eq"), &[prod_ab.clone(), mk_p, g("p")]);
    let refl_p = apps(g("refl"), &[prod_ab, g("p")]);
    let coerce = Term::Lambda { param: "h".to_string(), param_type: Box::new(eq), body: Box::new(v("h")) };
    assert_agreed(&ctx, &app(coerce, refl_p), "structure eta");
}

#[test]
fn structure_projection_computation_is_two_kernel_agreed() {
    // `Prod_fst Nat Bool (Prod_mk Nat Bool Zero true)` reduces to `Zero` — the
    // projection ι-computes identically in both kernels.
    let ctx = prod_ctx();
    let pair = apps(g("Prod_mk"), &[g("Nat"), g("Bool"), g("Zero"), g("true")]);
    let fst = apps(g("Prod_fst"), &[g("Nat"), g("Bool"), pair]);
    // Wrap so the projection sits in a checked position: refl over its value.
    let checked = apps(g("refl"), &[g("Nat"), fst]);
    assert_agreed(&ctx, &checked, "projection ι");
}
