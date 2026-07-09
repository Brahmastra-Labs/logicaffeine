//! Indexed inductive families — the layer past parametric (`TList`) recursors.
//!
//! An INDEXED family has arguments that VARY per constructor: `Eq A x : A → Prop`
//! with `refl : Eq A x x` (the index is forced to `x`), `Vector A : Nat → Type` with
//! `vnil : Vector A 0` / `vcons : … Vector A (Succ n)`, the `≤` predicate, etc. Their
//! dependent eliminator's motive must ABSTRACT over the indices — `Eq.rec`'s motive is
//! `P : Π(y:A). Eq A x y → Sort`, i.e. FULL Paulin-Mohring J, strictly stronger than a
//! `P : A → Prop` substitution axiom.
//!
//! These tests lock in: (1) the parameter/index split the kernel records per inductive,
//! (2) the auto-derived indexed recursor type-checks in BOTH kernels, (3) it has the
//! expected J shape, and (4) it actually ι-computes.

use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{
    derive_recursor, double_check, infer_type, is_subtype, normalize, Context, DoubleCheck, Term,
    Universe,
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

/// Register `Nat.le` as an indexed INDUCTIVE PREDICATE: `le (n:Nat) : Nat → Prop` with
/// `le_refl : le n n` and `le_step : Π(m:Nat). le n m → le n (Succ m)` — one parameter
/// (`n`), one index (the second `Nat`), and a RECURSIVE constructor whose index grows.
fn register_le(ctx: &mut Context) {
    // le : Π(n:Nat). Nat → Prop
    let le_ty = pi("n", g("Nat"), pi("_", g("Nat"), Term::Sort(Universe::Prop)));
    ctx.add_indexed_inductive("le", le_ty, 1);
    // le n m  helper
    let le = |n: Term, m: Term| app(app(g("le"), n), m);
    // le_refl : Π(n:Nat). le n n
    ctx.add_constructor("le_refl", "le", pi("n", g("Nat"), le(var("n"), var("n"))));
    // le_step : Π(n:Nat). Π(m:Nat). le n m → le n (Succ m)
    ctx.add_constructor(
        "le_step",
        "le",
        pi(
            "n",
            g("Nat"),
            pi(
                "m",
                g("Nat"),
                pi("_", le(var("n"), var("m")), le(var("n"), app(g("Succ"), var("m")))),
            ),
        ),
    );
}

/// `Vector A : Nat → Type` — length-indexed lists: 1 parameter (`A`), 1 index (the length),
/// a RECURSIVE constructor whose index grows.
fn register_vector(ctx: &mut Context) {
    let ty = Term::Sort(Universe::Type(0));
    let vty = pi("A", ty.clone(), pi("_", g("Nat"), ty.clone()));
    ctx.add_indexed_inductive("Vector", vty, 1);
    let vec = |a: Term, n: Term| app(app(g("Vector"), a), n);
    // vnil : Π(A:Type). Vector A Zero
    ctx.add_constructor("vnil", "Vector", pi("A", ty.clone(), vec(var("A"), g("Zero"))));
    // vcons : Π(A:Type). Π(n:Nat). A → Vector A n → Vector A (Succ n)
    ctx.add_constructor(
        "vcons",
        "Vector",
        pi(
            "A",
            ty,
            pi(
                "n",
                g("Nat"),
                pi(
                    "_",
                    var("A"),
                    pi("_", vec(var("A"), var("n")), vec(var("A"), app(g("Succ"), var("n")))),
                ),
            ),
        ),
    );
}

/// `Fin : Nat → Type` — the `n`-element finite type. ZERO parameters (the `Nat` is a pure
/// INDEX — the all-index edge case), a recursive constructor.
fn register_fin(ctx: &mut Context) {
    let ty = Term::Sort(Universe::Type(0));
    ctx.add_indexed_inductive("Fin", pi("_", g("Nat"), ty), 0);
    let fin = |n: Term| app(g("Fin"), n);
    // fzero : Π(n:Nat). Fin (Succ n)
    ctx.add_constructor("fzero", "Fin", pi("n", g("Nat"), fin(app(g("Succ"), var("n")))));
    // fsucc : Π(n:Nat). Fin n → Fin (Succ n)
    ctx.add_constructor(
        "fsucc",
        "Fin",
        pi("n", g("Nat"), pi("_", fin(var("n")), fin(app(g("Succ"), var("n"))))),
    );
}

// =============================================================================
// Next increment — Nat-INDEXED RECURSIVE families (inductive predicates like ≤).
// Gated on the termination guard picking the scrutinee (the matched proof) as the
// structural argument rather than the first inductive-typed binder (the Nat index).
// `#[ignore]`d until that guard fix lands so the shared suite stays green meanwhile.
// =============================================================================

#[test]
fn derives_le_recursor_two_kernel_verified() {
    // `le`'s dependent eliminator (induction on ≤) must derive and be verified by BOTH
    // kernels — the recursion is on the `le` proof, whose index (`m` → `Succ m`) is
    // Nat-typed, so the guard must not mistake the `Nat` index binder for the structural
    // argument.
    let mut ctx = std_ctx();
    register_le(&mut ctx);
    assert_recursor_two_kernel_verified(&ctx, "le");
}

#[test]
fn le_recursor_type_abstracts_the_index_and_proof() {
    let mut ctx = std_ctx();
    register_le(&mut ctx);
    let (ty, _) = assert_recursor_two_kernel_verified(&ctx, "le");
    // le.rec : Π(n:Nat). Π(P : Π(m:Nat). le n m → Prop). … — the motive (2nd binder)
    // abstracts over the index `m` AND the proof.
    let (motive_ty, _) = nth_pi_domain(&ty, 1);
    assert!(
        count_leading_pis(&motive_ty) >= 2,
        "le.rec's motive must abstract over the index and the proof: {motive_ty}"
    );
}

#[test]
fn le_recursor_computes_the_base_case() {
    // Induction on `≤` must ι-COMPUTE: `le.rec 0 P base step 0 (le_refl 0)` selects the
    // reflexivity branch and reduces to `base` — the recursive call in the `le_step` branch
    // (`rec m h`) is never reached here, but its presence exercises the guard's positional
    // structural check.
    let mut ctx = std_ctx();
    register_le(&mut ctx);
    let (_ty, rec) = derive_recursor(&ctx, "le").expect("derive le.rec");

    let zero = g("Zero");
    let le = |n: Term, m: Term| app(app(g("le"), n), m);
    // P := λm. λh. le 0 m  (a Prop family over the index)
    let motive =
        lam("m", g("Nat"), lam("h", le(zero.clone(), var("m")), le(zero.clone(), var("m"))));
    let refl0 = app(g("le_refl"), zero.clone());
    // step := λm. λh. λih. le_step 0 m ih  :  P m h → P (Succ m) (le_step 0 m h)
    let step = lam(
        "m",
        g("Nat"),
        lam(
            "h",
            le(zero.clone(), var("m")),
            lam(
                "ih",
                le(zero.clone(), var("m")),
                app(app(app(g("le_step"), zero.clone()), var("m")), var("ih")),
            ),
        ),
    );
    // le.rec 0 P (base = refl0) step 0 (le_refl 0)
    let applied = app(
        app(
            app(app(app(app(rec, zero.clone()), motive), refl0.clone()), step),
            zero.clone(),
        ),
        refl0.clone(),
    );
    assert_eq!(
        normalize(&ctx, &applied),
        refl0,
        "le.rec on le_refl must reduce to the base premise"
    );
}

#[test]
fn derives_vector_recursor_two_kernel_verified() {
    // `Vector`'s eliminator: a Type-valued, Nat-indexed, RECURSIVE family — the recursive
    // call `rec n tail` lands at the tail's index `n`, distinct from the result's `Succ n`.
    let mut ctx = std_ctx();
    register_vector(&mut ctx);
    assert_recursor_two_kernel_verified(&ctx, "Vector");
}

#[test]
fn vector_recursor_computes_length() {
    // `length` via `Vector.rec`: base = 0, step = λn h t ih. Succ ih. On a one-element
    // vector `vcons Nat 0 0 (vnil Nat) : Vector Nat 1` it must reduce to `Succ 0`.
    let mut ctx = std_ctx();
    register_vector(&mut ctx);
    let (_ty, rec) = derive_recursor(&ctx, "Vector").expect("derive Vector.rec");

    let nat = g("Nat");
    let zero = g("Zero");
    let one = app(g("Succ"), zero.clone());
    let vec = |a: Term, n: Term| app(app(g("Vector"), a), n);
    // vcons Nat 0 0 (vnil Nat)  :  Vector Nat 1
    let v1 = app(
        app(app(app(g("vcons"), nat.clone()), zero.clone()), zero.clone()),
        app(g("vnil"), nat.clone()),
    );
    let motive = lam("n", nat.clone(), lam("v", vec(nat.clone(), var("n")), nat.clone()));
    let base = zero.clone();
    let step = lam(
        "n",
        nat.clone(),
        lam(
            "h",
            nat.clone(),
            lam(
                "t",
                vec(nat.clone(), var("n")),
                lam("ih", nat.clone(), app(g("Succ"), var("ih"))),
            ),
        ),
    );
    // Vector.rec Nat P base step 1 v1
    let applied = app(
        app(app(app(app(app(rec, nat.clone()), motive), base), step), one.clone()),
        v1,
    );
    assert_eq!(normalize(&ctx, &applied), one, "length of a one-element vector must be 1");
}

#[test]
fn derives_fin_recursor_two_kernel_verified() {
    // `Fin` has ZERO parameters — its whole arity is the index — exercising the all-index
    // path of the recursor derivation and the guard.
    let mut ctx = std_ctx();
    register_fin(&mut ctx);
    assert_recursor_two_kernel_verified(&ctx, "Fin");
}

#[test]
fn fin_recursor_computes_to_nat() {
    // `toNat` via `Fin.rec`: fzero ↦ 0, fsucc _ ih ↦ Succ ih. On `fzero 0 : Fin 1` → 0.
    let mut ctx = std_ctx();
    register_fin(&mut ctx);
    let (_ty, rec) = derive_recursor(&ctx, "Fin").expect("derive Fin.rec");

    let nat = g("Nat");
    let zero = g("Zero");
    let fin = |n: Term| app(g("Fin"), n);
    let motive = lam("idx", nat.clone(), lam("f", fin(var("idx")), nat.clone()));
    let f_zero = lam("a0", nat.clone(), zero.clone());
    let f_succ = lam(
        "a0",
        nat.clone(),
        lam("a1", fin(var("a0")), lam("ih", nat.clone(), app(g("Succ"), var("ih")))),
    );
    // Fin.rec P f_zero f_succ 1 (fzero 0)  — no leading parameter (Fin has 0 params)
    let applied = app(
        app(
            app(app(app(rec, motive), f_zero), f_succ),
            app(g("Succ"), zero.clone()),
        ),
        app(g("fzero"), zero.clone()),
    );
    assert_eq!(normalize(&ctx, &applied), zero, "toNat (fzero 0) must be 0");
}

// =============================================================================
// Step A — the kernel records each inductive's parameter/index split.
// =============================================================================

#[test]
fn eq_is_indexed_two_params_one_index() {
    // `Eq : Π(A:Type). A → A → Prop` is Lean's `Eq {α} (a : α) : α → Prop`: the leading
    // `A` and `x` are PARAMETERS (uniform in `refl`'s result `Eq A x x`), the trailing
    // slot is the INDEX. So 2 params, 1 index.
    let ctx = std_ctx();
    assert_eq!(ctx.inductive_num_params("Eq"), 2, "Eq's parameters are A and x");
    assert_eq!(ctx.inductive_num_indices("Eq"), 1, "Eq's one index is the endpoint y");
}

#[test]
fn non_indexed_inductives_default_to_all_params() {
    // Every inductive that does NOT declare indices must report zero of them, so the
    // indexed machinery is a strict extension: `Nat` (arity 0) and the parametric
    // `TList : Type → Type` (arity 1, the parameter uniform) are unchanged.
    let ctx = std_ctx();
    assert_eq!(ctx.inductive_num_indices("Nat"), 0, "Nat has no indices");
    assert_eq!(ctx.inductive_num_params("TList"), 1, "TList's type parameter is uniform");
    assert_eq!(ctx.inductive_num_indices("TList"), 0, "TList is parametric, not indexed");
}

// =============================================================================
// Step C — the auto-derived indexed recursor is full dependent J, two-kernel verified.
// =============================================================================

/// Both kernels certify the synthesized recursor: the main kernel type-checks it, and the
/// independent de Bruijn re-checker agrees — so the eliminator is two-kernel-verified.
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
        "{ind}_rec must be independently re-checked, got {:?}",
        double_check(ctx, &term)
    );
    (ty, term)
}

/// Peel `n` leading `Π`s, returning the domain of the `n`-th and the residual body.
fn nth_pi_domain(ty: &Term, n: usize) -> (Term, Term) {
    let mut cur = ty.clone();
    let mut dom = Term::Hole;
    for _ in 0..=n {
        match cur {
            Term::Pi { param_type, body_type, .. } => {
                dom = *param_type;
                cur = *body_type;
            }
            _ => panic!("expected at least {} Π binders, got {ty}", n + 1),
        }
    }
    (dom, cur)
}

fn count_leading_pis(t: &Term) -> usize {
    let mut n = 0;
    let mut cur = t;
    while let Term::Pi { body_type, .. } = cur {
        n += 1;
        cur = body_type;
    }
    n
}

#[test]
fn derives_eq_recursor_two_kernel_verified() {
    // The identity type's eliminator, AUTO-DERIVED (not the `Eq_rec` axiom) and certified
    // by both kernels — the whole point of indexed families.
    let ctx = std_ctx();
    assert_recursor_two_kernel_verified(&ctx, "Eq");
}

#[test]
fn eq_recursor_type_is_full_dependent_j() {
    // `Eq.rec : Π(A). Π(x:A). Π(P : Π(y:A). Eq A x y → Type). P x (refl A x)
    //            → Π(y:A). Π(h:Eq A x y). P y h`.
    // The load-bearing check: the motive `P` (the 3rd binder) is DEPENDENT — it takes the
    // endpoint `y` AND the proof `h` (two Π binders before its codomain). A `P : A → Prop`
    // motive (the old axiom) would have only one. This is what makes it full J.
    let ctx = std_ctx();
    let (ty, _term) = assert_recursor_two_kernel_verified(&ctx, "Eq");
    let (motive_ty, _rest) = nth_pi_domain(&ty, 2); // A, x, then P
    assert!(
        count_leading_pis(&motive_ty) >= 2,
        "Eq.rec's motive must abstract over BOTH the endpoint and the equality proof \
         (full dependent J), but its type has {} Π binder(s): {motive_ty}",
        count_leading_pis(&motive_ty)
    );
}

#[test]
fn eq_recursor_computes_transport_on_refl() {
    // The J eliminator must ι-COMPUTE: `Eq.rec Nat 0 P base 0 (refl Nat 0)` selects the
    // `refl` branch and reduces to `base`. Here `P := λy.λh. Nat` (a constant family) and
    // `base := 0`, so the whole application must normalize to `0`.
    let ctx = std_ctx();
    let (_ty, rec_term) = derive_recursor(&ctx, "Eq").expect("derive Eq.rec");

    let nat = g("Nat");
    let zero = g("Zero");
    let eq_nat_zero_y = |y: Term| {
        app(app(app(g("Eq"), nat.clone()), zero.clone()), y)
    };
    // P : Π(y:Nat). Eq Nat Zero y → Type,  P := λy. λh. Nat
    let motive = lam("y", nat.clone(), lam("h", eq_nat_zero_y(var("y")), nat.clone()));
    let base = zero.clone();
    let refl_nat_zero = app(app(g("refl"), nat.clone()), zero.clone());

    // Eq.rec Nat Zero P base Zero (refl Nat Zero)
    let applied = app(
        app(
            app(app(app(app(rec_term, nat.clone()), zero.clone()), motive), base),
            zero.clone(),
        ),
        refl_nat_zero,
    );
    let result = normalize(&ctx, &applied);
    assert_eq!(result, zero, "J on refl must reduce to the base case, got {result}");
}

// =============================================================================
// Step D — the equality eliminator/lemmas are DERIVED from J, not trusted axioms.
// Removing `Eq_rec`/`Eq_sym`/`Eq_trans` from the axiom base shrinks the TCB — Lean's
// `Eq.rec` is likewise derived from the inductive, not an axiom.
// =============================================================================

#[test]
fn eq_rec_is_derived_not_an_axiom() {
    let ctx = std_ctx();
    assert!(
        ctx.is_definition("Eq_rec"),
        "Eq_rec must be a kernel-CHECKED definition derived from J, not a trusted axiom"
    );
    let body = ctx.get_definition_body("Eq_rec").expect("Eq_rec has a body").clone();
    assert_eq!(
        double_check(&ctx, &body),
        DoubleCheck::Agreed,
        "Eq_rec's body must be two-kernel verified"
    );
    // Soundness: the body must actually inhabit the DECLARED type (a definition stores the
    // declared type, which downstream proofs trust — a mismatch would be unsound).
    let decl_ty = ctx.get_definition_type("Eq_rec").expect("Eq_rec has a type").clone();
    let body_ty = infer_type(&ctx, &body).expect("Eq_rec body is well-typed");
    assert!(
        is_subtype(&ctx, &body_ty, &decl_ty),
        "Eq_rec's body must inhabit its declared type\n  body: {body_ty}\n  decl: {decl_ty}"
    );
}

#[test]
fn eq_sym_and_eq_trans_are_derived_not_axioms() {
    let ctx = std_ctx();
    for name in ["Eq_sym", "Eq_trans"] {
        assert!(ctx.is_definition(name), "{name} must be derived from J, not a trusted axiom");
        let body = ctx.get_definition_body(name).unwrap_or_else(|| panic!("{name} has a body")).clone();
        assert_eq!(
            double_check(&ctx, &body),
            DoubleCheck::Agreed,
            "{name}'s body must be two-kernel verified"
        );
        let decl_ty = ctx.get_definition_type(name).unwrap_or_else(|| panic!("{name} has a type")).clone();
        let body_ty = infer_type(&ctx, &body).unwrap_or_else(|_| panic!("{name} body is well-typed"));
        assert!(
            is_subtype(&ctx, &body_ty, &decl_ty),
            "{name}'s body must inhabit its declared type\n  body: {body_ty}\n  decl: {decl_ty}"
        );
    }
}

#[test]
fn eq_rec_computes_the_base_case_on_refl() {
    // Weak `Eq_rec : Π(A). Π(x). Π(P:A→Prop). P x → Π(y). Eq A x y → P y`, now DERIVED
    // from J. `Eq_rec Nat 0 (λy. Eq Nat y y) (refl Nat 0) 0 (refl Nat 0)` selects the refl
    // branch and reduces to the base `refl Nat 0`.
    let ctx = std_ctx();
    let nat = g("Nat");
    let zero = g("Zero");
    let refl00 = app(app(g("refl"), nat.clone()), zero.clone());
    let motive = lam("y", nat.clone(), app(app(app(g("Eq"), nat.clone()), var("y")), var("y")));
    let applied = app(
        app(
            app(app(app(app(g("Eq_rec"), nat.clone()), zero.clone()), motive), refl00.clone()),
            zero.clone(),
        ),
        refl00.clone(),
    );
    assert_eq!(normalize(&ctx, &applied), refl00, "derived Eq_rec must compute the base case on refl");
}

#[test]
fn eq_sym_on_refl_computes_to_refl() {
    // `Eq_sym Nat 0 0 (refl Nat 0)` — symmetry applied to reflexivity — reduces to `refl`.
    let ctx = std_ctx();
    let nat = g("Nat");
    let zero = g("Zero");
    let refl00 = app(app(g("refl"), nat.clone()), zero.clone());
    let applied =
        app(app(app(app(g("Eq_sym"), nat.clone()), zero.clone()), zero.clone()), refl00.clone());
    assert_eq!(normalize(&ctx, &applied), refl00, "sym of refl must compute to refl");
}
