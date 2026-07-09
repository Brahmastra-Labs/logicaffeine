//! Well-founded recursion: the accessibility predicate `Acc`, its dependent
//! eliminator, and the guard that makes recursion over an `Acc` proof
//! terminate. Three interlocking extensions — nested strict positivity, a
//! functional induction hypothesis in the derived recursor, and the
//! applied-smaller termination rule — each SOUNDNESS-CRITICAL. The paradox
//! fences prove the guard did not become a hole (`fix f. f` and friends stay
//! rejected).

use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{
    derive_recursor, double_check, infer_type, is_subtype, recheck, Context, DoubleCheck, Term,
    Universe,
};

fn lam(p: &str, t: Term, b: Term) -> Term {
    Term::Lambda { param: p.to_string(), param_type: Box::new(t), body: Box::new(b) }
}
fn fix(name: &str, body: Term) -> Term {
    Term::Fix { name: name.to_string(), body: Box::new(body) }
}
fn nat() -> Term {
    g("Nat")
}
/// `match d return motive with cases` — small helper for the guard-boundary fixtures.
fn match_(d: Term, motive: Term, cases: Vec<Term>) -> Term {
    Term::Match { discriminant: Box::new(d), motive: Box::new(motive), cases }
}

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
fn pi(p: &str, t: Term, b: Term) -> Term {
    Term::Pi { param: p.to_string(), param_type: Box::new(t), body_type: Box::new(b) }
}
fn arrow(a: Term, b: Term) -> Term {
    pi("_", a, b)
}
fn prop() -> Term {
    Term::Sort(Universe::Prop)
}
fn ty0() -> Term {
    Term::Sort(Universe::Type(0))
}
fn std_ctx() -> Context {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    ctx
}

/// The standard library now registers `Acc (A : Type) (R : A → A → Prop) : A → Prop`
/// with `Acc_intro : Π(A)(R)(x). (Π(y:A). R y x → Acc A R y) → Acc A R x` (two
/// parameters, one index, a FUNCTIONAL recursive argument — the strictly-positive
/// occurrence under a `Π`), the derived `Acc_rec`, and `WellFounded`. These tests
/// exercise that registered definition directly.
fn acc_ctx() -> Context {
    std_ctx()
}

// --- Blocker 1: nested strict positivity ------------------------------------

#[test]
fn acc_intro_passes_positivity() {
    // Registering Acc_intro must SUCCEED — its recursive occurrence sits under a
    // `Π` (a strictly-positive functional argument), which the old positivity
    // check wrongly rejected.
    let ctx = acc_ctx();
    assert!(infer_type(&ctx, &g("Acc_intro")).is_ok(), "Acc_intro must type-check");
    assert!(infer_type(&ctx, &g("Acc")).is_ok(), "Acc must be registered");
}

#[test]
fn negative_occurrence_paradox_still_rejected() {
    // THE FENCE: the extension must NOT admit a negative occurrence. The
    // Russell-style `mk : (Bad → False) → Bad` and the mixed
    // `mk : (Bad → Nat) → Bad` both put `Bad` in a domain, and must STILL be
    // rejected — else the applied-smaller guard becomes a soundness hole.
    use logicaffeine_kernel::check_positivity_for_test as pos;
    assert!(
        pos("Bad", "mk", &arrow(arrow(g("Bad"), g("False")), g("Bad"))).is_err(),
        "(Bad → False) → Bad must be rejected"
    );
    assert!(
        pos("Bad", "mk", &arrow(arrow(g("Bad"), g("Nat")), g("Bad"))).is_err(),
        "(Bad → Nat) → Bad must be rejected"
    );
    // A doubly-nested negative occurrence: `((Bad → False) → False) → Bad` — the
    // inner `Bad` is in a negative-of-negative position; strict positivity
    // forbids any occurrence in a domain, so this is rejected too.
    assert!(
        pos(
            "Bad",
            "mk",
            &arrow(arrow(arrow(g("Bad"), g("False")), g("False")), g("Bad"))
        )
        .is_err(),
        "nested negative occurrence must be rejected"
    );
}

#[test]
fn positive_function_field_is_accepted() {
    // A strictly-POSITIVE functional field `(Nat → Tree) → Tree` (Tree only in
    // the codomain) must be accepted — the shape rose trees / Acc need.
    let mut ctx = std_ctx();
    ctx.add_inductive("Tree", ty0());
    assert!(
        logicaffeine_kernel::check_positivity_for_test(
            "Tree",
            "node",
            &arrow(arrow(g("Nat"), g("Tree")), g("Tree"))
        )
        .is_ok(),
        "(Nat → Tree) → Tree is strictly positive"
    );
}

// --- Blocker 2: the recursor has a functional induction hypothesis ----------

/// The FULL Acc recursor type — the spec the derivation must produce. Its minor
/// premise `f` carries the FUNCTIONAL induction hypothesis `ih` for the field
/// `h`: `Π(y). Π(hr : R y x). P y (h y hr)`.
fn expected_acc_rec_type() -> Term {
    let acc = |x: Term| apps(g("Acc"), &[v("A"), v("R"), x]);
    let ryx = |y: Term, x: Term| apps(v("R"), &[y, x]);
    // h : Π(y). R y x → Acc A R y
    let h_ty = pi("y", v("A"), arrow(ryx(v("y"), v("x")), acc(v("y"))));
    // ih : Π(y). Π(hr : R y x). P y (h y hr)
    let ih_ty = pi(
        "y",
        v("A"),
        pi(
            "hr",
            ryx(v("y"), v("x")),
            apps(v("P"), &[v("y"), apps(v("h"), &[v("y"), v("hr")])]),
        ),
    );
    // P x (Acc_intro A R x h)
    let concl = apps(v("P"), &[v("x"), apps(g("Acc_intro"), &[v("A"), v("R"), v("x"), v("h")])]);
    // f : Π(x). Π(h). Π(ih). concl
    let minor = pi("x", v("A"), pi("h", h_ty, pi("ih", ih_ty, concl)));
    // P : Π(x). Acc A R x → Type 0
    let motive_ty = pi("x", v("A"), arrow(acc(v("x")), ty0()));
    pi(
        "A",
        ty0(),
        pi(
            "R",
            arrow(v("A"), arrow(v("A"), prop())),
            pi(
                "P",
                motive_ty,
                pi(
                    "f",
                    minor,
                    pi("x", v("A"), pi("acc", acc(v("x")), apps(v("P"), &[v("x"), v("acc")]))),
                ),
            ),
        ),
    )
}

#[test]
fn acc_recursor_derives_with_functional_ih() {
    // The derived recursor's TYPE must equal the full recursor (with the
    // functional IH), not the weak eliminator that drops it. This is the precise
    // spec for blocker 2 — a weak recursor (no `ih`) fails this equality.
    let ctx = acc_ctx();
    let (rec_ty, rec_body) = derive_recursor(&ctx, "Acc").expect("Acc recursor derives");
    let expected = expected_acc_rec_type();
    assert!(
        is_subtype(&ctx, &rec_ty, &expected) && is_subtype(&ctx, &expected, &rec_ty),
        "derived Acc_rec type must be the FULL recursor (with functional IH).\n\
         derived  = {rec_ty}\n\
         expected = {expected}"
    );
    // And the body actually has that type.
    let inferred = infer_type(&ctx, &rec_body).expect("Acc_rec body type-checks");
    assert!(is_subtype(&ctx, &inferred, &rec_ty), "Acc_rec body : Acc_rec type");
}

#[test]
fn acc_recursor_is_two_kernel() {
    // The derived `Acc_rec` — a `Fix` whose recursive call passes an APPLIED
    // smaller argument (`rec y (h y hr)`) — must pass the INDEPENDENT
    // re-checker's guard too.
    let ctx = acc_ctx();
    let (_rec_ty, rec_body) = derive_recursor(&ctx, "Acc").expect("Acc recursor derives");
    match double_check(&ctx, &rec_body) {
        DoubleCheck::Agreed => {}
        other => panic!("both kernels must accept Acc_rec, got {other:?}"),
    }
}

// --- Blocker 3: the applied-smaller rule is BOUNDED in BOTH kernels ----------

/// The applied-smaller rule earns `Acc`-recursion its termination, but it must
/// admit ONLY an application of a structurally-*smaller* variable. The dangerous
/// over-admission is a constructor-headed argument: `fix f. λn. match n with
/// Zero => Zero | Succ k => f (Succ k)` recurses on `Succ k`, which is LARGER than
/// the matched `k` — a non-terminating loop that would inhabit any type. BOTH
/// kernels must reject it: the main type-checker AND the independent de Bruijn
/// re-checker (whose guard has its own copy of the applied-smaller code). This is
/// the end-to-end proof that the `Acc` extension did not open a hole.
#[test]
fn constructor_applied_recursive_call_rejected_by_both_kernels() {
    let ctx = std_ctx();
    // fix f. λn:Nat. match n return (λ_:Nat. Nat) with Zero => Zero | Succ k => f (Succ k)
    let bad = fix(
        "f",
        lam(
            "n",
            nat(),
            match_(
                v("n"),
                lam("_", nat(), nat()),
                vec![
                    g("Zero"),
                    lam("k", nat(), app(v("f"), app(g("Succ"), v("k")))),
                ],
            ),
        ),
    );
    // Main kernel rejects (the guard fires during Fix inference).
    assert!(
        infer_type(&ctx, &bad).is_err(),
        "main kernel must reject a recursive call on `Succ k`"
    );
    // Independent re-checker rejects it too, in isolation.
    assert!(
        recheck(&ctx, &bad).is_err(),
        "de Bruijn re-checker must independently reject a recursive call on `Succ k`"
    );
}

/// The honest counterpart: genuine structural recursion `fix f. λn. match n with
/// Zero => Zero | Succ k => f k` — a BARE smaller argument — must still be accepted
/// by both kernels. The bound on the applied form must not reject honest calls.
#[test]
fn genuine_structural_recursion_accepted_by_both_kernels() {
    let ctx = std_ctx();
    let good = fix(
        "f",
        lam(
            "n",
            nat(),
            match_(
                v("n"),
                lam("_", nat(), nat()),
                vec![g("Zero"), lam("k", nat(), app(v("f"), v("k")))],
            ),
        ),
    );
    assert!(infer_type(&ctx, &good).is_ok(), "main kernel must accept honest structural recursion");
    match double_check(&ctx, &good) {
        DoubleCheck::Agreed => {}
        other => panic!("both kernels must agree on honest structural recursion, got {other:?}"),
    }
}

// --- The standard library exposes well-founded recursion --------------------

#[test]
fn prelude_registers_acc_wellfounded_and_recursor() {
    // `Acc`, its constructor `Acc_intro`, the derived eliminator `Acc_rec`, and the
    // `WellFounded` abbreviation must all be part of the standard library and
    // type-check.
    let mut ctx = std_ctx();
    for name in ["Acc", "Acc_intro", "Acc_rec", "WellFounded"] {
        assert!(infer_type(&ctx, &g(name)).is_ok(), "{name} must be registered and type-check");
    }
    // `WellFounded Nat lt : Prop` — the abbreviation applies and lands in Prop.
    ctx.add_declaration("lt", arrow(nat(), arrow(nat(), prop())));
    let wf = apps(g("WellFounded"), &[nat(), g("lt")]);
    let wf_sort = infer_type(&ctx, &wf).expect("WellFounded Nat lt type-checks");
    assert!(
        is_subtype(&ctx, &wf_sort, &prop()) && is_subtype(&ctx, &prop(), &wf_sort),
        "WellFounded Nat lt must be a Prop, got {wf_sort}"
    );
}

/// THE END-TO-END PAYOFF. `Acc_rec` instantiated at concrete `A := Nat`, a relation
/// `lt`, and a CONSTANT motive `λx.λ_. Nat` yields the recursion principle
/// `Π(x:Nat). Acc Nat lt x → Nat` — the exact skeleton a function defined by
/// well-founded recursion over `Nat` eliminates through. We build the recursor,
/// apply it up through its minor premise (the step, which receives the functional
/// induction hypothesis), and certify the resulting type in BOTH kernels. This
/// proves the whole `Acc` machinery — nested positivity, the functional-IH
/// eliminator, and the applied-smaller guard — composes into usable, kernel-checked
/// well-founded recursion, not just individually-passing pieces.
#[test]
fn acc_rec_builds_a_well_founded_recursion_over_nat() {
    let mut ctx = std_ctx();
    // An opaque well-founded-candidate relation `lt : Nat → Nat → Prop`.
    ctx.add_declaration("lt", arrow(nat(), arrow(nat(), prop())));
    let lt = g("lt");
    let acc = |x: Term| apps(g("Acc"), &[nat(), lt.clone(), x]);

    // P := λx:Nat. λ_:Acc Nat lt x. Nat   — a constant motive into Type 0.
    let motive = lam("x", nat(), lam("_", acc(v("x")), nat()));
    // h : Π(y:Nat). lt y x → Acc Nat lt y   (accessibility of all predecessors)
    let h_ty = |x: Term| pi("y", nat(), arrow(apps(lt.clone(), &[v("y"), x]), acc(v("y"))));
    // ih : Π(y:Nat). Π(hr : lt y x). Nat   (the functional IH, at the constant motive)
    let ih_ty = |x: Term| pi("y", nat(), pi("hr", apps(lt.clone(), &[v("y"), x]), nat()));
    // f := λx. λh. λih. Zero  — the recursion step, producing a Nat.
    let f = lam("x", nat(), lam("h", h_ty(v("x")), lam("ih", ih_ty(v("x")), g("Zero"))));

    let applied = apps(g("Acc_rec"), &[nat(), lt.clone(), motive, f]);
    let got = infer_type(&ctx, &applied).expect("Acc_rec application type-checks");
    // Expected tail: Π(x:Nat). Acc Nat lt x → Nat.
    let expected = pi("x", nat(), arrow(acc(v("x")), nat()));
    assert!(
        is_subtype(&ctx, &got, &expected) && is_subtype(&ctx, &expected, &got),
        "Acc_rec application must have the well-founded recursion type.\n\
         got      = {got}\n\
         expected = {expected}"
    );
    match double_check(&ctx, &applied) {
        DoubleCheck::Agreed => {}
        other => panic!("both kernels must certify the Acc_rec application, got {other:?}"),
    }
}
