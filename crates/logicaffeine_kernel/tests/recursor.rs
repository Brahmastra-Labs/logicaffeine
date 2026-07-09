//! R2 — auto-derived recursors, locked in by TDD.
//!
//! For each inductive we (1) synthesize its recursor, (2) confirm the MAIN kernel
//! certifies it (coverage + termination), (3) confirm the INDEPENDENT re-checker
//! agrees — so every generated eliminator is two-kernel-verified — and (4) confirm it
//! actually COMPUTES by ι/β-reduction. A recursor that type-checks but reduces wrongly
//! would be useless; a recursor that reduces but doesn't type-check would be unsound.

use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{
    derive_recursor, double_check, infer_type, normalize, Context, DoubleCheck, Term, Universe,
};

fn g(n: &str) -> Term {
    Term::Global(n.to_string())
}
fn app(f: Term, x: Term) -> Term {
    Term::App(Box::new(f), Box::new(x))
}
fn lam(p: &str, ty: Term, body: Term) -> Term {
    Term::Lambda { param: p.to_string(), param_type: Box::new(ty), body: Box::new(body) }
}
fn pi(p: &str, ty: Term, body: Term) -> Term {
    Term::Pi { param: p.to_string(), param_type: Box::new(ty), body_type: Box::new(body) }
}
fn var(n: &str) -> Term {
    Term::Var(n.to_string())
}
fn std_ctx() -> Context {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    ctx
}

/// The core guarantee: the synthesized recursor type-checks in the main kernel AND is
/// independently re-derived by the second kernel.
fn assert_recursor_two_kernel_verified(ctx: &Context, ind: &str) -> (Term, Term) {
    let (ty, term) = derive_recursor(ctx, ind).expect("recursor derives");
    assert!(
        infer_type(ctx, &term).is_ok(),
        "{ind}_rec must type-check in the main kernel: {:?}",
        infer_type(ctx, &term)
    );
    assert!(matches!(ty, Term::Pi { .. }), "{ind}_rec's type must be a Π, got {ty}");
    assert_eq!(
        double_check(ctx, &term),
        DoubleCheck::Agreed,
        "{ind}_rec must be independently re-checked (two-kernel-verified), got {:?}",
        double_check(ctx, &term)
    );
    (ty, term)
}

#[test]
fn derives_nat_recursor_two_kernel_verified() {
    let ctx = std_ctx();
    assert_recursor_two_kernel_verified(&ctx, "Nat");
}

#[test]
fn derives_bool_recursor_two_kernel_verified() {
    let ctx = std_ctx();
    assert_recursor_two_kernel_verified(&ctx, "Bool");
}

#[test]
fn derives_monomorphic_list_recursor_two_kernel_verified() {
    // EList = ENil | ECons Entity EList — a recursive constructor with a non-recursive
    // (head) and a recursive (tail) argument. The IH is on the tail only.
    let ctx = std_ctx();
    assert_recursor_two_kernel_verified(&ctx, "EList");
}

#[test]
fn derives_false_recursor_is_ex_falso() {
    // False has zero constructors → its recursor is `Π(P:False→Type). Π(x:False). P x`
    // (ex falso), a subsingleton large-elimination the kernel permits.
    let ctx = std_ctx();
    let (ty, _term) = assert_recursor_two_kernel_verified(&ctx, "False");
    // Π(P). Π(x:False). P x — two leading Πs, no minor premises.
    if let Term::Pi { body_type, .. } = &ty {
        assert!(
            matches!(body_type.as_ref(), Term::Pi { .. }),
            "False_rec = Π(P). Π(x:False). P x, got {ty}"
        );
    } else {
        panic!("expected a Π, got {ty}");
    }
}

#[test]
fn derives_three_constructor_enum_recursor() {
    // Light = Red | Yellow | Green — three nullary constructors, three minor premises.
    let mut ctx = std_ctx();
    ctx.add_inductive("Light", Term::Sort(Universe::Type(0)));
    for c in ["Red", "Yellow", "Green"] {
        ctx.add_constructor(c, "Light", g("Light"));
    }
    assert_recursor_two_kernel_verified(&ctx, "Light");
}

#[test]
fn derives_binary_tree_recursor_threads_both_hypotheses() {
    // Tree = Leaf | Node Tree Tree — a constructor with TWO recursive arguments, each
    // contributing its own induction hypothesis. Both `rec l` and `rec r` must appear,
    // and the whole thing must still pass the termination guard in BOTH kernels.
    let mut ctx = std_ctx();
    ctx.add_inductive("Tree", Term::Sort(Universe::Type(0)));
    ctx.add_constructor("Leaf", "Tree", g("Tree"));
    ctx.add_constructor("Node", "Tree", pi("l", g("Tree"), pi("r", g("Tree"), g("Tree"))));
    let (_ty, term) = assert_recursor_two_kernel_verified(&ctx, "Tree");
    // The term must contain two recursive calls (one per recursive Node argument).
    let rec_calls = format!("{term}").matches("(rec ").count();
    assert!(rec_calls >= 2, "Node's two IHs need two rec-calls, found {rec_calls}: {term}");
}

#[test]
fn derives_parametric_list_recursor_two_kernel_verified() {
    // The kernel's `TList : Type → Type` (TNil : Π(A). TList A, TCons : Π(A). A → TList A
    // → TList A) is PARAMETRIC. Its recursor must abstract the type parameter `A`,
    // thread it through the motive/cases, and still pass coverage + termination in BOTH
    // kernels: `Π(A). Π(P : TList A → Type). P (TNil A) → (Π h t. P t → P (TCons A h t))
    // → Π(x : TList A). P x`.
    let ctx = std_ctx();
    let (ty, term) = assert_recursor_two_kernel_verified(&ctx, "TList");
    // Leading Π binds the type parameter A.
    assert!(matches!(ty, Term::Pi { .. }), "TList_rec opens with Π(A:Type). …, got {ty}");
    // One recursive argument (the tail) → one rec-call.
    assert!(format!("{term}").contains("(rec "), "the Cons tail needs a recursive call: {term}");
}

#[test]
fn parametric_list_recursor_computes_length() {
    // TList_rec instantiated as `length` over `TList Nat`: f_nil = Zero, f_cons = λh t ih.
    // Succ ih. Applied to `TCons Nat Zero (TNil Nat)` it must reduce to `Succ Zero` (1).
    let ctx = std_ctx();
    let (_ty, rec_term) = derive_recursor(&ctx, "TList").expect("derive");

    let nat = g("Nat");
    let tlist_nat = app(g("TList"), nat.clone());
    // [Zero] : TList Nat  =  TCons Nat Zero (TNil Nat)
    let list = app(
        app(app(g("TCons"), nat.clone()), g("Zero")),
        app(g("TNil"), nat.clone()),
    );
    let motive = lam("_", tlist_nat.clone(), nat.clone());
    let f_nil = g("Zero");
    let f_cons = lam(
        "h",
        nat.clone(),
        lam("t", tlist_nat.clone(), lam("ih", nat.clone(), app(g("Succ"), var("ih")))),
    );

    // TList_rec Nat motive f_nil f_cons list
    let applied = app(
        app(app(app(app(rec_term, nat.clone()), motive), f_nil), f_cons),
        list,
    );
    let result = normalize(&ctx, &applied);
    let one = app(g("Succ"), g("Zero"));
    assert_eq!(result, one, "length [Zero] must reduce to 1, got {result}");
}

#[test]
fn nat_recursor_computes_by_reduction() {
    // The derived Nat recursor, instantiated as the IDENTITY (base = Zero, step = Succ∘IH),
    // must reduce `Nat_rec (λ_.Nat) Zero (λa.λih. Succ ih) (Succ (Succ Zero))` to
    // `Succ (Succ Zero)` — proving the generated `fix`/`match` actually ι-computes.
    let ctx = std_ctx();
    let (_ty, rec_term) = derive_recursor(&ctx, "Nat").expect("derive");

    let nat = g("Nat");
    let two = app(g("Succ"), app(g("Succ"), g("Zero")));
    let const_nat_motive = lam("_", nat.clone(), nat.clone());
    let base = g("Zero");
    let step = lam("a", nat.clone(), lam("ih", nat.clone(), app(g("Succ"), var("ih"))));

    let applied = app(app(app(app(rec_term, const_nat_motive), base), step), two.clone());
    let result = normalize(&ctx, &applied);
    assert_eq!(
        result, two,
        "the identity-shaped Nat recursor must reconstruct its argument, got {result}"
    );
}

#[test]
fn nat_recursor_computes_a_real_function_predecessor() {
    // A non-trivial computation: predecessor. base (pred Zero) = Zero; step a ih = a (the
    // Succ-argument itself). `pred (Succ (Succ Zero))` must reduce to `Succ Zero`.
    let ctx = std_ctx();
    let (_ty, rec_term) = derive_recursor(&ctx, "Nat").expect("derive");

    let nat = g("Nat");
    let three = app(g("Succ"), app(g("Succ"), app(g("Succ"), g("Zero"))));
    let two = app(g("Succ"), app(g("Succ"), g("Zero")));
    let const_nat_motive = lam("_", nat.clone(), nat.clone());
    let base = g("Zero");
    let step = lam("a", nat.clone(), lam("ih", nat.clone(), var("a"))); // predecessor: return `a`

    let applied = app(app(app(app(rec_term, const_nat_motive), base), step), three);
    let result = normalize(&ctx, &applied);
    assert_eq!(result, two, "pred(3) must reduce to 2, got {result}");
}
