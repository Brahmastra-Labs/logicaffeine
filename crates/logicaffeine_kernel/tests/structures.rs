//! Structures / records (Rung 0c substrate): one-constructor inductives with
//! auto-derived projections and DEFINITIONAL ETA (`p ≡ ⟨p.1, p.2⟩`), checked in
//! both kernels. This is what a typeclass/algebraic hierarchy is built on:
//! prove the field axioms once, carry the whole structure as one value.

use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{
    double_check, infer_type, is_subtype, normalize, Context, DoubleCheck, Term, Universe,
};

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
    xs.iter().fold(f, |acc, x| app(acc, x.clone()))
}
fn ty0() -> Term {
    Term::Sort(Universe::Type(0))
}

/// A context with the `Prod` structure: `Prod (A B : Type) := mk (fst : A) (snd : B)`.
fn prod_ctx() -> Context {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    ctx.add_structure(
        "Prod",
        &[("A", ty0()), ("B", ty0())],
        &[("fst", v("A")), ("snd", v("B"))],
    );
    ctx
}

#[test]
fn structure_registers_as_single_ctor_inductive() {
    let ctx = prod_ctx();
    // The type, constructor, and both projections are registered and typed.
    assert!(infer_type(&ctx, &g("Prod")).is_ok(), "Prod : Type→Type→Type");
    assert!(infer_type(&ctx, &g("Prod_mk")).is_ok(), "Prod_mk registered");
    assert!(infer_type(&ctx, &g("Prod_fst")).is_ok(), "Prod_fst registered");
    assert!(infer_type(&ctx, &g("Prod_snd")).is_ok(), "Prod_snd registered");
}

#[test]
fn projections_compute() {
    // Prod_fst Nat Bool (Prod_mk Nat Bool Zero true) ⤳ Zero.
    let ctx = prod_ctx();
    let pair = apps(g("Prod_mk"), &[g("Nat"), g("Bool"), g("Zero"), g("true")]);
    let fst = apps(g("Prod_fst"), &[g("Nat"), g("Bool"), pair.clone()]);
    assert_eq!(normalize(&ctx, &fst), g("Zero"), "fst computes to Zero");
    let snd = apps(g("Prod_snd"), &[g("Nat"), g("Bool"), pair]);
    assert_eq!(normalize(&ctx, &snd), g("true"), "snd computes to true");
}

#[test]
fn projection_types_are_correct() {
    // Prod_fst Nat Bool (mk …) : Nat, Prod_snd … : Bool.
    let ctx = prod_ctx();
    let pair = apps(g("Prod_mk"), &[g("Nat"), g("Bool"), g("Zero"), g("true")]);
    let fst = apps(g("Prod_fst"), &[g("Nat"), g("Bool"), pair.clone()]);
    let fst_ty = infer_type(&ctx, &fst).expect("fst type-checks");
    assert!(is_subtype(&ctx, &fst_ty, &g("Nat")), "fst : Nat, got {fst_ty}");
}

#[test]
fn structure_eta_defeq() {
    // THE HEADLINE: for `p : Prod Nat Bool`, `p ≡ Prod_mk _ _ (Prod_fst _ _ p) (Prod_snd _ _ p)`.
    // Definitional eta for structures — false today, the record-η rule.
    let ctx = {
        let mut c = prod_ctx();
        c.add_declaration("p", apps(g("Prod"), &[g("Nat"), g("Bool")]));
        c
    };
    let p = g("p");
    let expanded = apps(
        g("Prod_mk"),
        &[
            g("Nat"),
            g("Bool"),
            apps(g("Prod_fst"), &[g("Nat"), g("Bool"), p.clone()]),
            apps(g("Prod_snd"), &[g("Nat"), g("Bool"), p.clone()]),
        ],
    );
    // The two are definitionally equal: a term `p` and its η-expansion type-check
    // interchangeably. We witness this by checking a coercion accepts both.
    assert!(
        logicaffeine_kernel::defeq_for_test(&ctx, &p, &expanded),
        "structure eta: p ≡ ⟨p.fst, p.snd⟩"
    );
}

#[test]
fn eta_only_for_registered_structures() {
    // NEGATIVE control: eta must NOT fire for an ordinary (multi-constructor)
    // inductive. `n : Nat` is NOT def-eq to `Succ (pred n)` (Nat is not a
    // structure, and anyway that is not even always true).
    let ctx = {
        let mut c = prod_ctx();
        c.add_declaration("n", g("Nat"));
        c
    };
    let n = g("n");
    let succ_pred = app(g("Succ"), app(g("pred"), n.clone()));
    assert!(
        !logicaffeine_kernel::defeq_for_test(&ctx, &n, &succ_pred),
        "eta must not fire for the non-structure Nat"
    );
}

#[test]
fn structure_eta_is_two_kernel() {
    // The record-η must hold in the INDEPENDENT re-checker too — a term whose
    // checking requires eta must double-check as `Agreed`.
    let ctx = {
        let mut c = prod_ctx();
        c.add_declaration("p", apps(g("Prod"), &[g("Nat"), g("Bool")]));
        c
    };
    // `λ(x : Prod Nat Bool). Prod_mk Nat Bool (Prod_fst Nat Bool x) (Prod_snd Nat Bool x)`
    // has type `Prod Nat Bool → Prod Nat Bool`, provable only via eta when applied.
    let body = apps(
        g("Prod_mk"),
        &[
            g("Nat"),
            g("Bool"),
            apps(g("Prod_fst"), &[g("Nat"), g("Bool"), v("x")]),
            apps(g("Prod_snd"), &[g("Nat"), g("Bool"), v("x")]),
        ],
    );
    let repack = Term::Lambda {
        param: "x".to_string(),
        param_type: Box::new(apps(g("Prod"), &[g("Nat"), g("Bool")])),
        body: Box::new(body),
    };
    // Apply to `p`; the result must be def-eq to `p` (eta), and both kernels agree
    // on the whole term's type.
    let applied = app(repack, g("p"));
    assert!(
        logicaffeine_kernel::defeq_for_test(&ctx, &applied, &g("p")),
        "repack p ≡ p by eta"
    );
    match double_check(&ctx, &applied) {
        DoubleCheck::Agreed => {}
        other => panic!("both kernels must agree on the eta term, got {other:?}"),
    }
}

#[test]
fn dependent_field_structure_projects_and_computes() {
    // Σ-type: `Sig (A : Type)(B : A → Type) := mk (fst : A) (snd : B fst)`. The
    // SECOND field's type depends on the first — the projection must substitute
    // `fst` by `Sig_fst … s` in its own type (the dependent-record case).
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    let arrow_ty = Term::Pi {
        param: "_".to_string(),
        param_type: Box::new(v("A")),
        body_type: Box::new(ty0()),
    };
    ctx.add_structure(
        "Sig",
        &[("A", ty0()), ("B", arrow_ty)],
        &[("fst", v("A")), ("snd", app(v("B"), v("fst")))],
    );
    // A concrete family: B := λ_:Nat. Bool.
    let b_fam = Term::Lambda {
        param: "_".to_string(),
        param_type: Box::new(g("Nat")),
        body: Box::new(g("Bool")),
    };
    let pair = apps(g("Sig_mk"), &[g("Nat"), b_fam.clone(), g("Zero"), g("true")]);
    let fst = apps(g("Sig_fst"), &[g("Nat"), b_fam.clone(), pair.clone()]);
    assert_eq!(normalize(&ctx, &fst), g("Zero"), "Sig_fst computes");
    let snd = apps(g("Sig_snd"), &[g("Nat"), b_fam.clone(), pair.clone()]);
    assert_eq!(normalize(&ctx, &snd), g("true"), "Sig_snd computes");
    // The whole pair type-checks (dependent second field checked against B fst).
    assert!(infer_type(&ctx, &pair).is_ok(), "dependent pair type-checks");
    // And the second projection's TYPE is `B (Sig_fst … pair)` ≡ Bool here.
    let snd_ty = infer_type(&ctx, &snd).expect("Sig_snd type-checks");
    assert!(is_subtype(&ctx, &snd_ty, &g("Bool")), "Sig_snd : Bool, got {snd_ty}");
}

#[test]
fn typeclass_as_structure_end_to_end() {
    // Rung 0c: a typeclass IS a structure. `Monoid (M : Type) := mk (unit : M)
    // (op : M → M → M)`; register an instance for Nat, and the instance resolver
    // finds it — the mechanism the algebraic hierarchy is built on.
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    let binop = Term::Pi {
        param: "_".to_string(),
        param_type: Box::new(v("M")),
        body_type: Box::new(Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(v("M")),
            body_type: Box::new(v("M")),
        }),
    };
    ctx.add_structure("Monoid", &[("M", ty0())], &[("unit", v("M")), ("op", binop)]);

    // A Nat monoid instance: unit = Zero, op = λ(x y : Nat). x (a Nat→Nat→Nat op).
    let nat_op = Term::Lambda {
        param: "x".to_string(),
        param_type: Box::new(g("Nat")),
        body: Box::new(Term::Lambda {
            param: "y".to_string(),
            param_type: Box::new(g("Nat")),
            body: Box::new(v("x")),
        }),
    };
    let nat_monoid = apps(g("Monoid_mk"), &[g("Nat"), g("Zero"), nat_op]);
    let monoid_nat = app(g("Monoid"), g("Nat"));
    assert!(
        is_subtype(&ctx, &infer_type(&ctx, &nat_monoid).unwrap(), &monoid_nat),
        "the Nat monoid instance has type Monoid Nat"
    );
    ctx.add_instance(monoid_nat.clone(), nat_monoid.clone());

    // The resolver finds the instance for the class `Monoid Nat`.
    let mut mctx = logicaffeine_kernel::MetaCtx::default();
    let resolved = logicaffeine_kernel::resolve_instance(&ctx, &mut mctx, &monoid_nat)
        .expect("instance resolution finds the Nat monoid");
    // And the resolved instance's `unit` projects to Zero (the whole structure is
    // one carried value).
    let unit = apps(g("Monoid_unit"), &[g("Nat"), resolved]);
    assert_eq!(normalize(&ctx, &unit), g("Zero"), "the monoid's unit is Zero");
}

#[test]
fn structure_eta_required_for_typing_is_two_kernel() {
    // A term whose TYPE-CHECKING genuinely needs record-η in BOTH kernels: coerce
    // `refl (Prod A B) p : Eq (Prod A B) p p` to the expected type
    // `Eq (Prod A B) ⟨p.fst, p.snd⟩ p` — accepted only if `p ≡ ⟨p.fst, p.snd⟩`.
    // If either kernel lacked eta it would REJECT (a false-alarm disagreement),
    // so `Agreed` is the real two-kernel proof of the rule.
    let ctx = {
        let mut c = prod_ctx();
        c.add_declaration("p", apps(g("Prod"), &[g("Nat"), g("Bool")]));
        c
    };
    let prod_ab = apps(g("Prod"), &[g("Nat"), g("Bool")]);
    let fst_p = apps(g("Prod_fst"), &[g("Nat"), g("Bool"), g("p")]);
    let snd_p = apps(g("Prod_snd"), &[g("Nat"), g("Bool"), g("p")]);
    let mk_p = apps(g("Prod_mk"), &[g("Nat"), g("Bool"), fst_p, snd_p]);
    let eq_expected = apps(g("Eq"), &[prod_ab.clone(), mk_p, g("p")]);
    let refl_p = apps(g("refl"), &[prod_ab.clone(), g("p")]);
    let coerce = Term::Lambda {
        param: "y".to_string(),
        param_type: Box::new(eq_expected),
        body: Box::new(v("y")),
    };
    let attack = app(coerce, refl_p);
    // Main kernel accepts (eta fired).
    assert!(infer_type(&ctx, &attack).is_ok(), "eta-requiring coercion must type-check");
    // And the independent re-checker AGREES — the rule is genuinely two-kernel.
    match double_check(&ctx, &attack) {
        DoubleCheck::Agreed => {}
        other => panic!("both kernels must do structure eta, got {other:?}"),
    }
}
