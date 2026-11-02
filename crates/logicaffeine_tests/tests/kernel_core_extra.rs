//! # The kernel core's own specification locks
//!
//! The conformance corpus names one witness per defect. This file writes down the RULE each fix
//! implements and then pins it from BOTH sides — the thing that must now be refused, and the
//! neighbouring thing that must still be admitted — because a positive-only test is satisfied
//! by a weak implementation and a negative-only one is satisfied by a kernel that refuses
//! everything.
//!
//! Every rule below is stated once, in prose, and then checked against `type_checker.rs` and
//! `recheck.rs` SEPARATELY. Three soundness bugs this campaign were miscounted identically in
//! both kernels, so `double_check` returned `Agreed` throughout; the specification — not one
//! kernel's code — is the shared reference.

use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{
    defeq_for_test, derive_recursor, double_check, infer_type, is_subtype, normalize, recheck,
    Context, DoubleCheck, KernelError, Literal, Term, Universe,
};

// ==== HELPERS ===============================================================================

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
    xs.iter().cloned().fold(f, app)
}
fn lam(p: &str, ty: Term, b: Term) -> Term {
    Term::Lambda {
        param: p.to_string(),
        param_type: Box::new(ty),
        body: Box::new(b),
    }
}
fn pi(p: &str, ty: Term, b: Term) -> Term {
    Term::Pi {
        param: p.to_string(),
        param_type: Box::new(ty),
        body_type: Box::new(b),
    }
}
fn arrow(a: Term, b: Term) -> Term {
    pi("_", a, b)
}
fn prop() -> Term {
    Term::Sort(Universe::Prop)
}
fn type0() -> Term {
    Term::Sort(Universe::Type(0))
}
fn nat_lit(n: i64) -> Term {
    Term::Lit(Literal::Nat(logicaffeine_base::BigInt::from_i64(n)))
}
fn std_ctx() -> Context {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    ctx
}
/// The motive's target sort of `ind`'s derived recursor — `Prop` when large elimination was
/// refused, `Type 0` when granted.
fn large_elimination_target(ctx: &Context, ind: &str) -> Term {
    let (rec_ty, _) = derive_recursor(ctx, ind)
        .unwrap_or_else(|e| panic!("`{ind}`'s recursor must derive: {e:?}"));
    let mut binders: Vec<Term> = Vec::new();
    let mut cur = &rec_ty;
    while let Term::Pi {
        param_type,
        body_type,
        ..
    } = cur
    {
        binders.push((**param_type).clone());
        cur = body_type;
    }
    let motive_ty = binders
        .get(ctx.inductive_num_params(ind))
        .unwrap_or_else(|| panic!("{ind}_rec has no motive binder: {rec_ty}"))
        .clone();
    let mut target = &motive_ty;
    while let Term::Pi { body_type, .. } = target {
        target = body_type;
    }
    target.clone()
}

// =============================================================================================
// SPEC A — LARGE ELIMINATION OUT OF `Prop`
//
//   An inductive `I` living in `Prop` may be eliminated into a larger sort iff
//     (a) it has ZERO constructors, or
//     (b) it has EXACTLY ONE constructor, and every binder of that constructor's telescope
//         BEYOND `I`'s PARAMETERS is either
//           (i)  propositional (its type's sort is `Prop`), or
//           (ii) occurs, as itself, among the INDEX arguments of the constructor's result
//                type `I p̄ ī` (the arguments past the parameters).
//
//   Two telescopes meet here and they are NOT the same length. The inductive's ARITY is
//   params + indices; the constructor's telescope binds the params and then its own fields —
//   it does not bind the indices, it COMPUTES them. Counting the skip off the arity therefore
//   walks past real data fields, and counting it off the params without clause (ii) amputates
//   `Acc` and with it every definition by well-founded recursion.
// =============================================================================================

/// `Smuggle (A : Type 0) : Bool → Prop` with `mk : Π(A). Π(f : A → A). Smuggle A true`.
/// ONE parameter, ONE index. The field `f` is a FUNCTION at `Type 0` and the result index is
/// the constant `true`, so nothing about `f` is recoverable from the type — Lean refuses.
/// Counting the skip off the arity (2) walks past `f` entirely and grants it.
fn register_smuggle(ctx: &mut Context) {
    ctx.add_indexed_inductive("Smuggle", pi("A", type0(), arrow(g("Bool"), prop())), 1);
    ctx.add_constructor(
        "Smuggle_mk",
        "Smuggle",
        pi(
            "A",
            type0(),
            pi(
                "f",
                arrow(var("A"), var("A")),
                apps(g("Smuggle"), &[var("A"), g("true")]),
            ),
        ),
    );
}

/// `Carry (A : Type 0) : A → Prop` with `mk : Π(A). Π(h : True). Π(x : A). Carry A x`.
/// ONE parameter, ONE index. `h` is propositional and `x` IS the result's index, so both
/// fields are admissible and Lean grants large elimination. The propositional field is
/// deliberately FIRST, so a fixed-width skip lands on the wrong binder.
fn register_carry(ctx: &mut Context) {
    ctx.add_indexed_inductive("Carry", pi("A", type0(), arrow(var("A"), prop())), 1);
    ctx.add_constructor(
        "Carry_mk",
        "Carry",
        pi(
            "A",
            type0(),
            arrow(
                g("True"),
                pi("x", var("A"), apps(g("Carry"), &[var("A"), var("x")])),
            ),
        ),
    );
}

/// `Single : Prop` with `mk : True → Single` — no parameters, no indices, one propositional
/// field. Granted under every reading of the rule; the control that a tightening did not
/// simply refuse everything.
fn register_single(ctx: &mut Context) {
    ctx.add_inductive("Single", prop());
    ctx.add_constructor("Single_mk", "Single", arrow(g("True"), g("Single")));
}

/// `Hoard : Prop` with `mk : Nat → Hoard` — no parameters, no indices, one DATA field. Refused
/// under every reading; the control in the other direction.
fn register_hoard(ctx: &mut Context) {
    ctx.add_inductive("Hoard", prop());
    ctx.add_constructor("Hoard_mk", "Hoard", arrow(g("Nat"), g("Hoard")));
}

/// SPEC A, clause by clause, from ONE table — the parameterised cases the corpus's zero-param
/// `Tag`/`Weird` pair does not reach, plus every prelude proposition as a standing control.
#[test]
fn large_elimination_counts_its_skip_off_the_parameters_and_admits_the_indexed_fields() {
    type Reg = fn(&mut Context);
    // (inductive, registrar, grant?, why)
    let table: Vec<(&str, Reg, bool, &str)> = vec![
        (
            "Smuggle",
            register_smuggle as Reg,
            false,
            "the field `f : A → A` is data and the result index is the constant `true`, so a \
             proof of `Smuggle A true` must not be able to hand back the function it was \
             built from",
        ),
        (
            "Carry",
            register_carry as Reg,
            true,
            "`h : True` is propositional and `x : A` IS the result's index — the skip must be \
             one binder (the parameter `A`), not two",
        ),
        (
            "Single",
            register_single as Reg,
            true,
            "its only field is propositional",
        ),
        (
            "Hoard",
            register_hoard as Reg,
            false,
            "its only field is a `Nat`",
        ),
        (
            "False",
            (|_: &mut Context| {}) as Reg,
            true,
            "zero constructors — `ex falso` must keep reaching `Type`",
        ),
        (
            "And",
            (|_: &mut Context| {}) as Reg,
            true,
            "one constructor, both fields propositional",
        ),
        (
            "Or",
            (|_: &mut Context| {}) as Reg,
            false,
            "two constructors — the classic refusal, and the reason the rule exists",
        ),
        (
            "Eq",
            (|_: &mut Context| {}) as Reg,
            true,
            "`refl`'s telescope is exactly `Eq`'s two parameters, so nothing is left to check \
             — today it passes because the skip loop RUNS OUT, which is an accident; it must \
             pass because the rule says so",
        ),
        (
            "Acc",
            (|_: &mut Context| {}) as Reg,
            true,
            "`x : A` is data admitted ONLY by the index clause; if a fix loses that clause, \
             `Acc_rec` dies and with it every definition by well-founded recursion",
        ),
    ];

    let mut wrong: Vec<String> = Vec::new();
    for (ind, register, grant, why) in &table {
        let mut ctx = std_ctx();
        register(&mut ctx);
        let want = if *grant { type0() } else { prop() };
        let got = large_elimination_target(&ctx, ind);
        if got != want {
            wrong.push(format!(
                "{ind}: large elimination must be {} ({why}) — its recursor's motive targets \
                 {got} where it must target {want}",
                if *grant { "GRANTED" } else { "REFUSED" }
            ));
        }
    }
    assert!(wrong.is_empty(), "SPEC A violated: {wrong:#?}");
}

/// SPEC A in BOTH kernels. `recheck::is_subsingleton` is a SEPARATE implementation of the same
/// rule, reached from the re-checker's own `Match` gate. A `Match` that large-eliminates must
/// be refused by both kernels or admitted by both — never one each, and never "agreed" because
/// they made the same arithmetic slip.
#[test]
fn both_kernels_decide_large_elimination_by_the_same_rule() {
    // REFUSED: `Hoard` hoards a `Nat`, so a match into `Nat` would hand it back.
    let mut refusing = std_ctx();
    register_hoard(&mut refusing);
    refusing.add_declaration("hh", g("Hoard"));
    let extract = Term::Match {
        discriminant: Box::new(g("hh")),
        motive: Box::new(lam("_", g("Hoard"), g("Nat"))),
        cases: vec![lam("n", g("Nat"), var("n"))],
    };
    assert!(
        infer_type(&refusing, &extract).is_err(),
        "the main kernel must refuse large elimination of `Hoard`: {:?}",
        infer_type(&refusing, &extract).map(|t| t.to_string())
    );
    assert!(
        recheck(&refusing, &extract).is_err(),
        "and so must the re-checker: {:?}",
        recheck(&refusing, &extract).map(|t| t.to_string())
    );
    assert!(
        !matches!(double_check(&refusing, &extract), DoubleCheck::Disagree(_)),
        "the two kernels must not disagree about `Hoard`: {:?}",
        double_check(&refusing, &extract)
    );

    // GRANTED: `Single`'s only field is a proof, so a match into `Nat` is legal — and must be
    // legal in BOTH kernels, or the tightening above was over-broad.
    let mut granting = std_ctx();
    register_single(&mut granting);
    granting.add_declaration("hs", g("Single"));
    let allowed = Term::Match {
        discriminant: Box::new(g("hs")),
        motive: Box::new(lam("_", g("Single"), g("Nat"))),
        cases: vec![lam("_t", g("True"), g("Zero"))],
    };
    assert_eq!(
        infer_type(&granting, &allowed).expect("`Single` is a subsingleton and may eliminate"),
        g("Nat"),
        "the main kernel must ADMIT large elimination of `Single`"
    );
    assert_eq!(
        double_check(&granting, &allowed),
        DoubleCheck::Agreed,
        "and the re-checker must corroborate it: {:?}",
        double_check(&granting, &allowed)
    );
}

// =============================================================================================
// SPEC B — `Term::Hole` IS NOT A TERM OF THE THEORY
//
//   A `Hole` is an UNSOLVED elaboration problem. `elaborate` turns it into a metavariable;
//   the kernel never sees one in a term it is asked to certify. Therefore: every kernel
//   judgement that reaches a `Hole` FAILS, and no comparison involving one succeeds — not
//   against a type, not against a sort, not against another `Hole`. The second kernel already
//   refuses one outright (`ReCheckError::Unsupported`); this is the first kernel agreeing.
//
//   The neighbouring thing that must STILL hold: every comparison between real terms.
// =============================================================================================

/// The refusing half: a `Hole` in any position `infer_type` can reach is an error, and no
/// comparison against one succeeds.
#[test]
fn a_hole_is_refused_wherever_the_kernel_reaches_it() {
    let ctx = std_ctx();
    let mut leaks: Vec<String> = Vec::new();

    // Comparison. A hole is not a supertype, not a subtype, not definitionally equal — and
    // two holes are not equal to EACH OTHER either, because neither is a term.
    let pairs: Vec<(&str, Term, Term)> = vec![
        ("Nat ≤ _", g("Nat"), Term::Hole),
        ("_ ≤ Nat", Term::Hole, g("Nat")),
        ("_ ≤ False", Term::Hole, g("False")),
        ("Type 0 ≤ _", type0(), Term::Hole),
        ("_ ≤ _", Term::Hole, Term::Hole),
    ];
    for (label, a, b) in &pairs {
        if is_subtype(&ctx, a, b) {
            leaks.push(format!("`is_subtype` accepted {label}"));
        }
        if defeq_for_test(&ctx, a, b) {
            leaks.push(format!("`def_eq` accepted {label}"));
        }
    }

    // Typing. Every position a `Hole` can occupy in a term the kernel is handed.
    let positions: Vec<(&str, Term)> = vec![
        ("a bare hole", Term::Hole),
        ("a Π domain", pi("_", Term::Hole, g("Nat"))),
        ("a Π codomain", pi("_", g("Nat"), Term::Hole)),
        ("a λ binder type", lam("x", Term::Hole, var("x"))),
        ("a λ body", lam("x", g("Nat"), Term::Hole)),
        ("an application's head", app(Term::Hole, g("Zero"))),
        ("an application's argument", app(g("Succ"), Term::Hole)),
        (
            "a `let`'s type",
            Term::Let {
                name: "x".to_string(),
                ty: Box::new(Term::Hole),
                value: Box::new(g("Zero")),
                body: Box::new(var("x")),
            },
        ),
        (
            "`Eq`'s carrier",
            apps(g("Eq"), &[Term::Hole, nat_lit(1), g("true")]),
        ),
        (
            "a partially-applied `Eq`",
            app(app(g("Eq"), Term::Hole), g("true")),
        ),
    ];
    for (label, t) in &positions {
        if let Ok(ty) = infer_type(&ctx, t) {
            leaks.push(format!("`infer_type` accepted {label}, giving `{ty}`"));
        }
    }

    assert!(
        leaks.is_empty(),
        "SPEC B: a `Term::Hole` is an unsolved elaboration problem, not a term — every \
         judgement that reaches one must fail. {} leak(s):\n  - {}",
        leaks.len(),
        leaks.join("\n  - ")
    );
}

/// The admitting half — the negative of the negative. Tightening the `Hole` rule must not cost
/// a single comparison between REAL terms: reflexivity, cumulativity, Π variance, the Peano
/// literal bridge, δ-unfolding, η and proof irrelevance all still decide as before.
#[test]
fn tightening_the_hole_rule_costs_no_comparison_between_real_terms() {
    let mut ctx = std_ctx();
    ctx.add_declaration("Q", prop());
    ctx.add_declaration("q1", g("Q"));
    ctx.add_declaration("q2", g("Q"));

    let must_hold: Vec<(&str, Term, Term)> = vec![
        ("Nat ≤ Nat", g("Nat"), g("Nat")),
        ("Prop ≤ Type 0 (cumulativity)", prop(), type0()),
        ("Type 0 ≤ Type 1", type0(), Term::Sort(Universe::Type(1))),
        (
            "Π contravariance/covariance",
            pi("_", type0(), prop()),
            pi("_", prop(), type0()),
        ),
    ];
    let mut lost: Vec<String> = Vec::new();
    for (label, a, b) in &must_hold {
        if !is_subtype(&ctx, a, b) {
            lost.push(format!("`is_subtype` lost {label}"));
        }
    }

    let defeq_must_hold: Vec<(&str, Term, Term)> = vec![
        ("Nat ≡ Nat", g("Nat"), g("Nat")),
        (
            "the Peano bridge `4 ≡ Succ⁴ Zero`",
            nat_lit(4),
            app(
                g("Succ"),
                app(g("Succ"), app(g("Succ"), app(g("Succ"), g("Zero")))),
            ),
        ),
        (
            "β `(λx.x) Zero ≡ Zero`",
            app(lam("x", g("Nat"), var("x")), g("Zero")),
            g("Zero"),
        ),
        (
            "η `Succ ≡ λn. Succ n`",
            g("Succ"),
            lam("n", g("Nat"), app(g("Succ"), var("n"))),
        ),
        ("proof irrelevance `q1 ≡ q2`", g("q1"), g("q2")),
    ];
    for (label, a, b) in &defeq_must_hold {
        if !defeq_for_test(&ctx, a, b) {
            lost.push(format!("`def_eq` lost {label}"));
        }
        if !defeq_for_test(&ctx, b, a) {
            lost.push(format!("`def_eq` lost {label} in the reverse direction"));
        }
    }

    // And the refusals between real terms are still refusals — a tightening must not have
    // been implemented as "everything is equal now".
    let must_not_hold: Vec<(&str, Term, Term)> = vec![
        ("Nat ≡ Bool", g("Nat"), g("Bool")),
        ("Zero ≡ Succ Zero", g("Zero"), app(g("Succ"), g("Zero"))),
        ("Type 1 ≤ Type 0", Term::Sort(Universe::Type(1)), type0()),
    ];
    for (label, a, b) in &must_not_hold {
        if defeq_for_test(&ctx, a, b) || is_subtype(&ctx, a, b) {
            lost.push(format!(
                "the kernel became too permissive: it accepted {label}"
            ));
        }
    }

    assert!(lost.is_empty(), "SPEC B's other half: {lost:#?}");
}

/// A `Hole` still REDUCES to itself — `normalize` has no rule for one and must not invent one.
/// The corpus pins this too; it is repeated here because it is the invariant that makes the
/// refusals above complete (a hole can never be reduced away, so refusing it at comparison
/// time is refusing it for the term's whole life).
#[test]
fn a_hole_is_its_own_normal_form() {
    let ctx = std_ctx();
    assert_eq!(normalize(&ctx, &Term::Hole), Term::Hole);
    assert_eq!(
        normalize(&ctx, &app(g("Succ"), Term::Hole)),
        app(g("Succ"), Term::Hole),
        "and it survives inside a spine"
    );
}

// =============================================================================================
// SPEC B′ — `double_check` REPORTS AGREEMENT ONLY WHEN BOTH KERNELS REACHED A VERDICT
//
//   `Agreed` is the strongest claim the API makes: two independently-written kernels concurred.
//   A re-checker that DECLINED TO LOOK (`ReCheckError::Unsupported`) has not concurred with
//   anything. Today `(Err, Err)` collapses to `Agreed` without asking WHY the re-checker
//   failed, so `double_check(ctx, Term::Hole)` — a term the second kernel refuses to parse —
//   is reported as corroborated by both.
// =============================================================================================

#[test]
fn an_unsupported_recheck_is_never_reported_as_agreement() {
    let ctx = std_ctx();
    // `Hole` is outside the re-checker's fragment AND (once SPEC B lands) refused by the main
    // kernel. Neither kernel typed it; they did not therefore agree about it.
    for (label, t) in [
        ("a bare hole", Term::Hole),
        ("a hole under a binder", lam("x", g("Nat"), Term::Hole)),
        (
            "a hole in a spine",
            apps(g("Eq"), &[Term::Hole, nat_lit(1), g("true")]),
        ),
    ] {
        let verdict = double_check(&ctx, &t);
        assert!(
            matches!(verdict, DoubleCheck::MainOnlyReCheckerIncomplete(_)),
            "`double_check` on {label} must report the re-checker as INCOMPLETE, never as \
             agreement — it never examined the term: {verdict:?}"
        );
    }

    // The control: when both kernels genuinely REJECT a term they have both examined, that IS
    // agreement and must keep being reported as such.
    let ill = app(g("Succ"), type0());
    assert!(
        infer_type(&ctx, &ill).is_err(),
        "`Succ (Type 0)` is ill-typed"
    );
    assert!(recheck(&ctx, &ill).is_err(), "…in both kernels");
    assert_eq!(
        double_check(&ctx, &ill),
        DoubleCheck::Agreed,
        "two kernels that both examined and both rejected a term have agreed"
    );
}

// =============================================================================================
// SPEC C — DEFINITIONAL EQUALITY IS SYMMETRIC
//
//   `def_eq(a, b) = def_eq(b, a)` for every pair of terms, full stop. Conversion is an
//   equivalence relation; a checker whose answer depends on argument order accepts a proof
//   written one way and refuses the identical proof written the other, and the difference is
//   invisible at the call site. Proof irrelevance is the rule that breaks it: it infers the
//   LEFT side's type, demands that be a `Prop`, and only then looks at the right.
// =============================================================================================

#[test]
fn definitional_equality_is_symmetric_across_a_representative_table() {
    let mut ctx = std_ctx();
    ctx.add_declaration("Q", prop());
    ctx.add_declaration("q1", g("Q"));
    ctx.add_declaration("q2", g("Q"));
    ctx.add_declaration("R", prop());
    ctx.add_declaration("r1", g("R"));
    // A declaration whose TYPE will not infer — the shape a translator emits for a position it
    // could not resolve, and the shortest witness that the irrelevance rule is one-way.
    ctx.add_declaration("qbad", Term::Hole);
    // A declaration whose type is a `Type`, not a `Prop` — irrelevance must not fire for it in
    // EITHER direction.
    ctx.add_declaration("d1", g("Nat"));
    ctx.add_declaration("d2", g("Nat"));

    let terms: Vec<(&str, Term)> = vec![
        ("Nat", g("Nat")),
        ("Bool", g("Bool")),
        ("Prop", prop()),
        ("Type 0", type0()),
        ("Zero", g("Zero")),
        ("Succ Zero", app(g("Succ"), g("Zero"))),
        ("1", nat_lit(1)),
        ("q1", g("q1")),
        ("q2", g("q2")),
        ("r1", g("r1")),
        ("qbad", g("qbad")),
        ("d1", g("d1")),
        ("d2", g("d2")),
        ("_", Term::Hole),
        ("λx:Nat. x", lam("x", g("Nat"), var("x"))),
        ("Π_:Nat. Nat", pi("_", g("Nat"), g("Nat"))),
    ];

    let mut asymmetric: Vec<String> = Vec::new();
    for (na, a) in &terms {
        for (nb, b) in &terms {
            let forward = defeq_for_test(&ctx, a, b);
            let backward = defeq_for_test(&ctx, b, a);
            if forward != backward {
                asymmetric.push(format!(
                    "def_eq({na}, {nb}) = {forward} but def_eq({nb}, {na}) = {backward}"
                ));
            }
        }
    }
    assert!(
        asymmetric.is_empty(),
        "SPEC C: definitional equality must not depend on argument order. {} asymmetric \
         pair(s):\n  - {}",
        asymmetric.len(),
        asymmetric.join("\n  - ")
    );

    // Reflexivity, on the same table — the invariant a symmetric-but-broken rule would also
    // have to satisfy, so it is checked separately.
    for (n, t) in &terms {
        if matches!(t, Term::Hole) {
            continue; // SPEC B: a hole is not a term, so it is not equal to itself either.
        }
        assert!(
            defeq_for_test(&ctx, t, t),
            "def_eq must be reflexive, and it is not at {n}"
        );
    }

    // The negative that keeps symmetry honest: irrelevance must NOT equate two distinct
    // inhabitants of a `Type`, in either direction.
    assert!(
        !defeq_for_test(&ctx, &g("d1"), &g("d2")) && !defeq_for_test(&ctx, &g("d2"), &g("d1")),
        "two distinct `Nat` declarations are not definitionally equal — proof irrelevance \
         must fire only for propositions"
    );
    // …nor two proofs of DIFFERENT propositions.
    assert!(
        !defeq_for_test(&ctx, &g("q1"), &g("r1")) && !defeq_for_test(&ctx, &g("r1"), &g("q1")),
        "a proof of `Q` and a proof of `R` are not definitionally equal"
    );
}

// =============================================================================================
// SPEC D — NO KERNEL ENTRY POINT MAY KILL THE PROCESS
//
//   A term is UNTRUSTED INPUT: it arrives from a translator, a file, a network peer. Every
//   public entry point must answer with a value or a `KernelError`. Aborting the process on a
//   deep term is a denial of service with no error channel, and it is indistinguishable from a
//   crash in the code that called the kernel.
//
//   MEASURED, on this box, in the debug profile nextest builds (a 2 MiB test thread):
//     · `infer_type` on a `Succ` spine aborts between depth 120 (survives) and 150.
//     · `normalize`  aborts between 200 (survives) and 400.
//     · `recheck`    aborts between 150 (survives) and 400.
//     · `Term::clone`/`drop` — the DERIVED impls, no kernel code at all — abort between
//       2000 (survives) and 4000. A term that deep kills the process on the way IN.
//
//   So the kernel's depth door is set at 1600 — above the deepest term the suite builds (a
//   1500-deep β-spine, which normalises fine because its redex is at the head), below the depth
//   at which merely COPYING a `Term` a few times over aborts. It turns the pathological case into a
//   `KernelError`. The residue is recorded below and is NOT closed: how deep the kernel can
//   descend depends on the SHAPE of a term, not only its depth — an irreducible 400-deep spine
//   aborts inside `reduce_step` while a 1500-deep β-spine does not — so between the door and
//   each entry point's own cliff a term is still accepted and still aborts. Closing that needs
//   a larger kernel stack at the call site, or an iterative `Drop`/`Clone` for `Term` and an
//   explicit-stack reducer — `term.rs` and a reducer rewrite, not a constant.
// =============================================================================================

/// A spine past the door. Whatever the kernel decides, it must RETURN: the assertion is that
/// these lines are reached at all, and that the answer is an error rather than an abort.
#[test]
fn a_term_past_the_depth_door_is_refused_and_does_not_abort() {
    let ctx = std_ctx();
    let mut deep = g("Zero");
    for _ in 0..1800 {
        deep = app(g("Succ"), deep);
    }
    let inferred = infer_type(&ctx, &deep);
    assert!(
        matches!(inferred, Err(KernelError::CertificationError(ref why)) if why.contains("deeper")),
        "a term past the door must be REFUSED with an error, not walked: {inferred:?}"
    );
    assert!(
        recheck(&ctx, &deep).is_err(),
        "the re-checker must refuse it at the same door"
    );
    // Syntactic identity needs no descent, so a deep term is still α-equal to ITSELF — the
    // door governs the comparisons that would have to normalise.
    assert!(
        is_subtype(&ctx, &deep, &deep),
        "a term is still a subtype of itself"
    );
    let other = app(g("Succ"), deep.clone());
    assert!(
        !is_subtype(&ctx, &deep, &other) && !defeq_for_test(&ctx, &deep, &other),
        "two DIFFERENT terms past the door cannot be compared, so they are not equal — a          refused comparison must never answer `true`"
    );
    // `normalize` cannot report, so it must at least return — its contract is a bare `Term`.
    assert_eq!(
        normalize(&ctx, &deep),
        deep,
        "a refused reduction hands the term back unchanged"
    );
}

/// The same door reached through nested binders rather than an application spine.
#[test]
fn a_binder_tower_past_the_depth_door_is_refused_and_does_not_abort() {
    let ctx = std_ctx();
    let mut tower = g("Nat");
    for i in 0..1800 {
        tower = pi(&format!("x{i}"), g("Nat"), tower);
    }
    assert!(
        infer_type(&ctx, &tower).is_err(),
        "an 1800-binder telescope is past the door"
    );
    assert_eq!(
        normalize(&ctx, &tower),
        tower,
        "and reduction hands it back"
    );
}

/// The BOUNDARY, below the door: ordinary depths must be completely unaffected. A guard that
/// buys safety by refusing real terms is worse than the abort it replaced, because it fails
/// silently in the middle of a proof instead of loudly at the end.
#[test]
fn ordinary_depths_are_untouched_by_the_door() {
    let ctx = std_ctx();
    let mut spine = g("Zero");
    for _ in 0..100 {
        spine = app(g("Succ"), spine);
    }
    assert_eq!(
        infer_type(&ctx, &spine).expect("a 100-deep Peano numeral must type-check"),
        g("Nat")
    );
    assert!(
        defeq_for_test(&ctx, &spine, &nat_lit(100)),
        "…and equal the literal 100"
    );
    assert!(recheck(&ctx, &spine).is_ok(), "…in both kernels");

    let mut tower = g("Nat");
    for i in 0..100 {
        tower = pi(&format!("x{i}"), g("Nat"), tower);
    }
    assert!(
        matches!(infer_type(&ctx, &tower), Ok(Term::Sort(_))),
        "a 100-binder telescope is a type"
    );
}

/// THE RESIDUE, pinned so it cannot rot into a mystery. Given ENOUGH STACK the kernel is
/// perfectly correct at depths that abort on a default test thread — which is the actual
/// finding: the limit is the ambient stack, not the logic. This test runs the same work on a
/// 64 MiB thread and requires it to succeed; if the kernel's recursion ever stops being merely
/// stack-hungry and starts being wrong, this goes red rather than the depth tests going quiet.
#[test]
fn the_kernel_is_correct_at_depths_a_default_thread_cannot_afford() {
    let worker = std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(|| {
            let ctx = std_ctx();
            let mut spine = g("Zero");
            for _ in 0..500 {
                spine = app(g("Succ"), spine);
            }
            let ok_infer = infer_type(&ctx, &spine).ok() == Some(g("Nat"));
            let ok_defeq = defeq_for_test(&ctx, &spine, &nat_lit(500));
            let ok_recheck = recheck(&ctx, &spine).is_ok();
            (ok_infer, ok_defeq, ok_recheck)
        })
        .expect("a worker thread with a large stack");
    assert_eq!(
        worker
            .join()
            .expect("the kernel must not abort when it has stack to work in"),
        (true, true, true),
        "a 500-deep Peano numeral types, converts and re-checks correctly — the default test \
         thread's 2 MiB is what it cannot afford, not the arithmetic"
    );
}

// =============================================================================================
// SPEC E — `normalize` MUST NOT PRESENT AN ABANDONED REDUCTION AS A NORMAL FORM
//
//   `normalize` spends a budget and returns a bare `Term`, so "this is the normal form" and
//   "this is as far as I got" are the SAME ANSWER. A δ-cycle proves no budget constant can fix
//   it: the reduction has no normal form at all.
//
//   Half of this is closed: the reducer now DETECTS a cycling reduction (Brent, O(1) memory)
//   instead of grinding through its whole budget, and a long TERMINATING reduction now
//   finishes instead of being abandoned at 10 000 steps. The other half — telling the caller —
//   needs `normalize`'s bare-`Term` signature to grow an error channel. `normalize_checked`
//   exists and carries `Unreduced::{Cyclic, BudgetSpent, TooDeep}`, but it is `pub(crate)`:
//   exporting it is one `pub use` line in `lib.rs`, which the kernel core does not own.
// =============================================================================================

#[test]
fn a_delta_cycle_terminates_at_once_instead_of_grinding_through_the_budget() {
    let mut ctx = std_ctx();
    ctx.add_definition("loop_a".to_string(), g("Nat"), g("loop_b"));
    ctx.add_definition("loop_b".to_string(), g("Nat"), g("loop_a"));

    let started = std::time::Instant::now();
    let cycled = normalize(&ctx, &g("loop_a"));
    let elapsed = started.elapsed();
    assert!(
        elapsed < std::time::Duration::from_millis(250),
        "a δ-cycle must be DETECTED, not ground through a step budget — took {elapsed:?}"
    );
    // Whatever it hands back must be stable: normalising it again cannot change the answer, or
    // the caller's fixpoint never settles either.
    assert_eq!(
        normalize(&ctx, &cycled),
        cycled,
        "the answer for a cycle must be stable"
    );
    // Typing a term containing it must still terminate rather than diverge.
    let _ = infer_type(&ctx, &app(lam("x", g("Nat"), var("x")), g("loop_a")));
}

/// The control: a long but TERMINATING δ-chain has a normal form and the kernel must find it.
/// A "fix" that reports every long reduction as a failure is not a fix.
#[test]
fn a_long_but_terminating_delta_chain_still_reaches_its_normal_form() {
    let mut ctx = std_ctx();
    const CHAIN: usize = 20_000;
    for i in 0..CHAIN {
        let next = if i + 1 == CHAIN {
            g("Zero")
        } else {
            g(&format!("chain{}", i + 1))
        };
        ctx.add_definition(format!("chain{i}"), g("Nat"), next);
    }
    assert_eq!(
        normalize(&ctx, &g("chain0")),
        g("Zero"),
        "a 20000-step δ-chain terminates; its normal form is `Zero`, and a 10000-step budget \
         used to hand back `chain10000` as though it were one"
    );
}

// =============================================================================================
// SPEC F — AN IMPOSSIBLE CONSTRUCTOR BRANCH NEEDS NO CASE
//
//   A `Match` on an INDEXED family must supply a case for exactly those constructors whose
//   declared result indices can UNIFY with the discriminant's. A constructor ruled out by the
//   indices contributes no branch; demanding one forces the caller to invent an inhabitant of
//   a type that has none.
//
//   The negative: when the indices genuinely CAN match, the case is still required.
// =============================================================================================

/// `Vect (A : Type 0) : Nat → Type 0` — the standard length-indexed vector.
fn register_vect(ctx: &mut Context) {
    ctx.add_indexed_inductive("Vect", pi("A", type0(), arrow(g("Nat"), type0())), 1);
    ctx.add_constructor(
        "VNil",
        "Vect",
        pi("A", type0(), apps(g("Vect"), &[var("A"), g("Zero")])),
    );
    ctx.add_constructor(
        "VCons",
        "Vect",
        pi(
            "A",
            type0(),
            pi(
                "n",
                g("Nat"),
                arrow(
                    var("A"),
                    arrow(
                        apps(g("Vect"), &[var("A"), var("n")]),
                        apps(g("Vect"), &[var("A"), app(g("Succ"), var("n"))]),
                    ),
                ),
            ),
        ),
    );
}

/// The generic non-dependent motive over `Vect Nat`: `λ(_n:Nat). λ(_v:Vect Nat _n). Nat`.
fn vect_motive() -> Term {
    lam(
        "_n",
        g("Nat"),
        lam("_v", apps(g("Vect"), &[g("Nat"), var("_n")]), g("Nat")),
    )
}

#[test]
fn a_match_whose_indices_rule_a_constructor_out_needs_no_case_for_it() {
    let mut ctx = std_ctx();
    register_vect(&mut ctx);
    ctx.add_declaration("v0", apps(g("Vect"), &[g("Nat"), g("Zero")]));

    // `Vect Nat Zero`: `VCons`'s result index is `Succ n`, which cannot be `Zero`. One case.
    // The motive abstracts the GENERIC family — `λ(_n:Nat). λ(_v:Vect Nat _n). Nat`, the shape of
    // Lean's `Vect.casesOn` motive `(n : Nat) → Vect α n → Sort u` — and the pruning is decided
    // by the discriminant's index, not by a motive specialised to it.
    let only_nil = Term::Match {
        discriminant: Box::new(g("v0")),
        motive: Box::new(vect_motive()),
        cases: vec![g("Zero")],
    };
    let typed = infer_type(&ctx, &only_nil);
    assert!(
        typed.is_ok(),
        "`Zero` cannot unify with `Succ n`, so a one-branch match on `Vect Nat Zero` is \
         exhaustive: {typed:?}"
    );

    // THE NEGATIVE. On a discriminant whose index is a VARIABLE, both constructors can match,
    // so both cases are still required — the unifier must not be "accept any case count".
    let open = ctx.extend("m", g("Nat"));
    let open = open.extend("vm", apps(g("Vect"), &[g("Nat"), var("m")]));
    let missing_a_case = Term::Match {
        discriminant: Box::new(var("vm")),
        motive: Box::new(vect_motive()),
        cases: vec![g("Zero")],
    };
    assert!(
        matches!(
            infer_type(&open, &missing_a_case),
            Err(KernelError::WrongNumberOfCases {
                expected: 2,
                found: 1
            })
        ),
        "`Vect Nat m` with `m` open admits BOTH constructors, so one case is not exhaustive: \
         {:?}",
        infer_type(&open, &missing_a_case).map(|t| t.to_string())
    );

    // A motive whose major premise is typed at the SPECIALISED family `Vect Nat Zero` instead of
    // `Vect Nat _n` is not an eliminator motive at all — Lean's `casesOn` would not accept it —
    // and the kernel refuses it before any branch is pruned or checked.
    let specialised = Term::Match {
        discriminant: Box::new(g("v0")),
        motive: Box::new(lam(
            "_n",
            g("Nat"),
            lam("_v", apps(g("Vect"), &[g("Nat"), g("Zero")]), g("Nat")),
        )),
        cases: vec![g("Zero")],
    };
    assert!(
        matches!(
            infer_type(&ctx, &specialised),
            Err(KernelError::InvalidMotive(_))
        ),
        "a motive over the specialised family must be refused: {:?}",
        infer_type(&ctx, &specialised)
    );
}

// =============================================================================================
// SPEC G — THE GIMÉNEZ GUARD ADMITS EXACTLY THE STRUCTURAL SUBTERMS
//
//   A recursive call is admissible iff its structural argument is a STRUCTURAL SUBTERM of the
//   current body's structural parameter. The subterm relation is generated by two rules and
//   closed under transitivity:
//
//     · matching a variable `v` on a constructor binds that constructor's FIELDS, each a
//       subterm of `v`;
//     · `let x := v` makes `x` the same value as `v`, so `x` is a subterm of exactly what `v`
//       is a subterm of.
//
//   Transitivity is the whole content: a field of a field of `n` is a subterm of `n`, which is
//   what makes `| n+2 => …` — every `halve`/`fib`/`pairSum` in every library — structurally
//   admissible. Refusing it does not make the kernel safer; it pushes a translator onto an
//   accumulator or a fuel parameter, and the guard accepts BOTH of those while they compute a
//   DIFFERENT function of the same type.
//
//   THE WIDENING IS WHERE THE DANGER IS. Every test below that admits something is paired with
//   the neighbouring thing that must still be refused, because an unguarded fixpoint inhabits
//   `False` and `False` inhabits everything.
// =============================================================================================

fn fix(name: &str, body: Term) -> Term {
    Term::Fix {
        name: name.to_string(),
        body: Box::new(body),
    }
}
fn match_(disc: Term, motive: Term, cases: Vec<Term>) -> Term {
    Term::Match {
        discriminant: Box::new(disc),
        motive: Box::new(motive),
        cases,
    }
}
fn nat_to_nat() -> Term {
    lam("_", g("Nat"), g("Nat"))
}

/// The admitting half, and the COMPUTING half — a recursion that type-checks but does not
/// ι-reduce is only half a capability.
#[test]
fn recursion_on_a_subterm_of_a_subterm_is_admitted_and_computes() {
    let ctx = std_ctx();
    // halve = fix h. λn. match n with Zero => Zero | Succ m => match m with Zero => Zero
    //                                                          | Succ k => Succ (h k)
    let halve = fix(
        "h",
        lam(
            "n",
            g("Nat"),
            match_(
                var("n"),
                nat_to_nat(),
                vec![
                    g("Zero"),
                    lam(
                        "m",
                        g("Nat"),
                        match_(
                            var("m"),
                            nat_to_nat(),
                            vec![
                                g("Zero"),
                                lam("k", g("Nat"), app(g("Succ"), app(var("h"), var("k")))),
                            ],
                        ),
                    ),
                ],
            ),
        ),
    );
    assert!(
        infer_type(&ctx, &halve).is_ok(),
        "`k` is a field of `m` which is a field of `n`, so recursion on it decreases: {:?}",
        infer_type(&ctx, &halve)
    );
    // …and it must actually compute. `halve 4` is `2`.
    let mut four = g("Zero");
    for _ in 0..4 {
        four = app(g("Succ"), four);
    }
    assert!(
        defeq_for_test(&ctx, &app(halve.clone(), four), &nat_lit(2)),
        "a recursion the guard admits must also ι-reduce: `halve 4` is `2`"
    );

    // The `let`-alias form of the same function, which denotes exactly the same thing.
    let via_let = fix(
        "h",
        lam(
            "n",
            g("Nat"),
            match_(
                var("n"),
                nat_to_nat(),
                vec![
                    g("Zero"),
                    lam(
                        "m",
                        g("Nat"),
                        Term::Let {
                            name: "alias".to_string(),
                            ty: Box::new(g("Nat")),
                            value: Box::new(var("m")),
                            body: Box::new(app(g("Succ"), app(var("h"), var("alias")))),
                        },
                    ),
                ],
            ),
        ),
    );
    assert!(
        infer_type(&ctx, &via_let).is_ok(),
        "a `let`-alias of a smaller variable is the same value: {:?}",
        infer_type(&ctx, &via_let)
    );
}

/// THE REFUSING HALF. Each of these is one step outside the subterm relation and must stay
/// refused — the widening above must not have turned into "anything a `let` or a nested `match`
/// can reach".
#[test]
fn the_guard_still_refuses_everything_outside_the_subterm_relation() {
    let ctx = std_ctx();
    let mut refused: Vec<&str> = Vec::new();
    let mut admitted: Vec<&str> = Vec::new();

    // (label, the fixpoint)
    let cases: Vec<(&str, Term)> =
        vec![
        (
            "recursing on the structural parameter ITSELF",
            fix("f", lam("n", g("Nat"), app(var("f"), var("n")))),
        ),
        (
            "a `let` alias of the structural parameter — an alias inherits what its value IS, \
             and `n` is not a subterm of `n`",
            fix(
                "f",
                lam(
                    "n",
                    g("Nat"),
                    match_(
                        var("n"),
                        nat_to_nat(),
                        vec![
                            g("Zero"),
                            lam(
                                "_m",
                                g("Nat"),
                                Term::Let {
                                    name: "alias".to_string(),
                                    ty: Box::new(g("Nat")),
                                    value: Box::new(var("n")),
                                    body: Box::new(app(var("f"), var("alias"))),
                                },
                            ),
                        ],
                    ),
                ),
            ),
        ),
        (
            "a `let` alias of a term that is not a variable at all (a reconstructed `Succ m` is \
             BIGGER than `m`, not smaller)",
            fix(
                "f",
                lam(
                    "n",
                    g("Nat"),
                    match_(
                        var("n"),
                        nat_to_nat(),
                        vec![
                            g("Zero"),
                            lam(
                                "m",
                                g("Nat"),
                                Term::Let {
                                    name: "bigger".to_string(),
                                    ty: Box::new(g("Nat")),
                                    value: Box::new(app(g("Succ"), var("m"))),
                                    body: Box::new(app(var("f"), var("bigger"))),
                                },
                            ),
                        ],
                    ),
                ),
            ),
        ),
        (
            "a nested match on a variable that is NOT smaller — `k` is a field of `other`, so \
             it is a subterm of `other` and of nothing else; `f k other` loops forever",
            fix(
                "f",
                lam(
                    "n",
                    g("Nat"),
                    lam(
                        "other",
                        g("Nat"),
                        match_(
                            var("n"),
                            nat_to_nat(),
                            vec![
                                g("Zero"),
                                lam(
                                    "_m",
                                    g("Nat"),
                                    match_(
                                        var("other"),
                                        nat_to_nat(),
                                        vec![
                                            g("Zero"),
                                            lam(
                                                "k",
                                                g("Nat"),
                                                apps(var("f"), &[var("k"), var("other")]),
                                            ),
                                        ],
                                    ),
                                ),
                            ],
                        ),
                    ),
                ),
            ),
        ),
        (
            "the higher-order escape — the recursive name as a first-class value",
            fix("f", lam("n", g("Nat"), app(lam("gg", g("Nat"), var("gg")), var("f")))),
        ),
    ];

    for (label, term) in &cases {
        match infer_type(&ctx, term) {
            Err(_) => refused.push(label),
            Ok(_) => admitted.push(label),
        }
    }
    assert!(
        admitted.is_empty(),
        "the guard admitted {} recursion(s) outside the structural-subterm relation — an \
         unguarded fixpoint inhabits `False`, and `False` inhabits everything:\n  - {}",
        admitted.len(),
        admitted.join("\n  - ")
    );
    assert_eq!(
        refused.len(),
        cases.len(),
        "every case above must be refused"
    );
}

/// BOTH KERNELS. `recheck.rs`'s guard is a separate implementation over de Bruijn LEVELS; the
/// two must widen together or `double_check` reports a disagreement on a legal definition —
/// and, worse, could report `Agreed` on an illegal one because both made the same slip.
#[test]
fn both_kernels_apply_the_same_subterm_relation() {
    let ctx = std_ctx();
    let halve = fix(
        "h",
        lam(
            "n",
            g("Nat"),
            match_(
                var("n"),
                nat_to_nat(),
                vec![
                    g("Zero"),
                    lam(
                        "m",
                        g("Nat"),
                        match_(
                            var("m"),
                            nat_to_nat(),
                            vec![
                                g("Zero"),
                                lam("k", g("Nat"), app(g("Succ"), app(var("h"), var("k")))),
                            ],
                        ),
                    ),
                ],
            ),
        ),
    );
    assert_eq!(
        double_check(&ctx, &halve),
        DoubleCheck::Agreed,
        "a `| n+2 => …` recursion must be admitted by BOTH kernels: {:?}",
        double_check(&ctx, &halve)
    );

    // And the refusal must also be shared: neither kernel may accept `fix f. λn. f n`.
    let diverging = fix("f", lam("n", g("Nat"), app(var("f"), var("n"))));
    assert!(
        infer_type(&ctx, &diverging).is_err(),
        "the main kernel refuses `fix f. λn. f n`"
    );
    assert!(
        recheck(&ctx, &diverging).is_err(),
        "and so does the re-checker"
    );
    assert!(
        !matches!(double_check(&ctx, &diverging), DoubleCheck::Disagree(_)),
        "the two kernels must not disagree about a diverging fixpoint: {:?}",
        double_check(&ctx, &diverging)
    );
}

/// SPEC F in BOTH kernels, and its negative. A pruned indexed match must be accepted by the
/// re-checker too, and a genuinely non-exhaustive one refused by both.
#[test]
fn both_kernels_prune_an_indexed_match_by_the_same_rule() {
    let mut ctx = std_ctx();
    register_vect(&mut ctx);
    ctx.add_declaration("v0", apps(g("Vect"), &[g("Nat"), g("Zero")]));
    let motive = vect_motive;

    let pruned = Term::Match {
        discriminant: Box::new(g("v0")),
        motive: Box::new(motive()),
        cases: vec![g("Zero")],
    };
    assert!(
        infer_type(&ctx, &pruned).is_ok(),
        "the main kernel prunes the impossible `VCons`"
    );
    assert!(
        recheck(&ctx, &pruned).is_ok(),
        "and so must the re-checker, or `double_check` disagrees on a legal match: {:?}",
        recheck(&ctx, &pruned)
    );
    assert_eq!(
        double_check(&ctx, &pruned),
        DoubleCheck::Agreed,
        "{:?}",
        double_check(&ctx, &pruned)
    );

    // THE NEGATIVE, in both kernels: an index that is an open variable rules nothing out.
    let open = ctx
        .extend("m", g("Nat"))
        .extend("vm", apps(g("Vect"), &[g("Nat"), var("m")]));
    let under_covered = Term::Match {
        discriminant: Box::new(var("vm")),
        motive: Box::new(vect_motive()),
        cases: vec![g("Zero")],
    };
    assert!(
        matches!(
            infer_type(&open, &under_covered),
            Err(KernelError::WrongNumberOfCases {
                expected: 2,
                found: 1
            })
        ),
        "`Vect Nat m` with `m` open admits BOTH constructors: {:?}",
        infer_type(&open, &under_covered)
    );
    assert!(
        recheck(&open, &under_covered).is_err(),
        "and the re-checker must agree"
    );

    // And disjointness is a TYPE-level phenomenon: in `Prop`, proof irrelevance makes two
    // constructors definitionally equal, so a clash proves nothing and NO branch may be pruned.
    let mut prop_ctx = std_ctx();
    prop_ctx.add_indexed_inductive("PFam", arrow(g("Bool"), prop()), 0);
    prop_ctx.add_constructor("PTrue", "PFam", app(g("PFam"), g("true")));
    prop_ctx.add_constructor("PFalse", "PFam", app(g("PFam"), g("false")));
    prop_ctx.add_declaration("pt", app(g("PFam"), g("true")));
    let pruned_prop = Term::Match {
        discriminant: Box::new(g("pt")),
        motive: Box::new(lam(
            "b",
            g("Bool"),
            lam("_p", app(g("PFam"), var("b")), prop()),
        )),
        cases: vec![g("True")],
    };
    assert!(
        infer_type(&prop_ctx, &pruned_prop).is_err(),
        "a propositional family's constructors are definitionally equal, so `false ≠ true` \
         cannot discharge the `PFalse` branch: {:?}",
        infer_type(&prop_ctx, &pruned_prop).map(|t| t.to_string())
    );
}

// =============================================================================================
// SPEC H — `ex falso` REQUIRES A TYPE THE KERNEL CANNOT INHABIT
//
//   An inductive with ZERO constructors eliminates into any sort with no cases at all. That is
//   `False.elim`, and it is sound because nothing inhabits `False`, so the branch can never be
//   taken.
//
//   The rule is about INHABITATION, not about the constructor count — and the kernel has a
//   second way to inhabit a type: its own `Term::Lit` rule, which hands out inhabitants of
//   `Int`, `Float`, `Text`, `Duration`, `Date` and `Moment`. Every one of those is registered
//   as an inductive with zero constructors. So `match (5 : Int) return False with []` is a
//   CLOSED PROOF OF FALSE assembled from nothing but the standard library — and both kernels
//   count constructors the same way, so `double_check` corroborates it.
//
//   This is not in the conformance corpus. It was found by asking what `Int`'s missing constructors
//   (T10-004) cost beyond the case-split they block.
// =============================================================================================

#[test]
fn a_literal_carrier_with_no_constructors_must_not_eliminate_into_anything() {
    let ctx = std_ctx();

    // The CONTROL first, so a fix cannot be "refuse every empty match": `False` genuinely has
    // no inhabitants and must keep eliminating into any sort.
    let mut falso = std_ctx();
    falso.add_declaration("hf", g("False"));
    let ex_falso = Term::Match {
        discriminant: Box::new(g("hf")),
        motive: Box::new(lam("_", g("False"), g("Nat"))),
        cases: vec![],
    };
    assert_eq!(
        infer_type(&falso, &ex_falso).expect("`False.elim` must keep working"),
        g("Nat"),
        "ex falso out of a genuinely empty type is the rule, not the bug"
    );

    // Every carrier the kernel's own `Lit` rule inhabits, against a motive of `False`.
    let carriers: Vec<(&str, Term)> = vec![
        ("Int", Term::Lit(Literal::Int(5))),
        ("Float", Term::Lit(Literal::Float(1.5))),
        ("Text", Term::Lit(Literal::Text("hello".to_string()))),
        ("Duration", Term::Lit(Literal::Duration(1_000))),
        ("Date", Term::Lit(Literal::Date(20_000))),
        ("Moment", Term::Lit(Literal::Moment(1_700_000_000))),
    ];

    let mut proved_false: Vec<String> = Vec::new();
    for (carrier, literal) in &carriers {
        // The kernel manufactures this inhabitant itself…
        let inhabited = infer_type(&ctx, literal).ok().map(|t| t.to_string());
        assert_eq!(
            inhabited.as_deref(),
            Some(*carrier),
            "the `Lit` rule must still type this literal at `{carrier}`"
        );
        // …so an empty match on it must be REFUSED, whatever its constructor count says.
        let absurd = Term::Match {
            discriminant: Box::new(literal.clone()),
            motive: Box::new(lam("_", g(carrier), g("False"))),
            cases: vec![],
        };
        if let Ok(ty) = infer_type(&ctx, &absurd) {
            proved_false.push(format!(
                "`match ({literal} : {carrier}) return False with []` type-checks at `{ty}` — \
                 {} constructors registered, and the second kernel {}",
                ctx.get_constructors(carrier).len(),
                match double_check(&ctx, &absurd) {
                    DoubleCheck::Agreed => "AGREES".to_string(),
                    other => format!("{other:?}"),
                }
            ));
        }
        if recheck(&ctx, &absurd).is_ok() {
            proved_false.push(format!("the re-checker also accepted it for `{carrier}`"));
        }
    }

    assert!(
        proved_false.is_empty(),
        "an empty `match` is `ex falso` and needs an UNINHABITED type. These carriers have no \
         constructors AND are inhabited by the kernel's own literal rule, so each one is a \
         closed proof of `False` out of the standard library:\n  - {}",
        proved_false.join("\n  - ")
    );
}

/// SPEC F reads a constructor's declared indices at the scrutinee's own PARAMETERS. `refl`'s index
/// is its parameter `x`, so at `Eq Nat (Succ a) Zero` it is `Succ a`, which clashes with `Zero`: the
/// match owes no case at all, as Lean's `noConfusion` closes it. In both kernels, and the negative:
/// at `Eq Nat a Zero` with `a` open, `refl` can still produce the scrutinee.
#[test]
fn both_kernels_read_a_constructors_indices_at_the_scrutinees_parameters() {
    let eq_nat = |l: Term, r: Term| apps(g("Eq"), &[g("Nat"), l, r]);
    let empty = |lhs: Term| Term::Match {
        discriminant: Box::new(var("h")),
        motive: Box::new(lam(
            "z",
            g("Nat"),
            lam("_e", eq_nat(lhs, var("z")), g("False")),
        )),
        cases: vec![],
    };
    let closed = std_ctx()
        .extend("a", g("Nat"))
        .extend("h", eq_nat(app(g("Succ"), var("a")), g("Zero")));
    let none_owed = empty(app(g("Succ"), var("a")));
    assert!(
        infer_type(&closed, &none_owed).is_ok(),
        "`refl` cannot produce `Eq Nat (Succ a) Zero`: {:?}",
        infer_type(&closed, &none_owed)
    );
    assert!(
        recheck(&closed, &none_owed).is_ok(),
        "nor in the re-checker: {:?}",
        recheck(&closed, &none_owed)
    );
    assert_eq!(double_check(&closed, &none_owed), DoubleCheck::Agreed);

    let open = std_ctx()
        .extend("a", g("Nat"))
        .extend("h", eq_nat(var("a"), g("Zero")));
    let owed = empty(var("a"));
    assert!(
        matches!(
            infer_type(&open, &owed),
            Err(KernelError::WrongNumberOfCases {
                expected: 1,
                found: 0
            })
        ),
        "`refl` produces `Eq Nat a Zero` at `a := Zero`: {:?}",
        infer_type(&open, &owed)
    );
    assert!(
        recheck(&open, &owed).is_err(),
        "and the re-checker owes the case too"
    );
}

/// SPEC F reads a constructor written with its universe levels as the constructor it is: `pnil.{0}`
/// and `pcons.{0}` of a polymorphic list in `Sort (u+1)` (never a proposition) clash, so a match on a
/// `Len` at a `pcons` index owes no case, in both kernels.
#[test]
fn both_kernels_read_a_levelled_constructor_as_a_clash_head() {
    let mut repl = logicaffeine_kernel::interface::Repl::new();
    for command in [
        "Inductive PL.{u} (A : Sort (u+1)) : Sort (u+1) := pnil : PL.{u} A | pcons : A -> PL.{u} A -> PL.{u} A.",
        "Inductive Len.{u} (A : Sort (u+1)) : PL.{u} A -> Type := lnil : Len.{u} A (pnil.{u} A).",
    ] {
        repl.execute(command).unwrap_or_else(|e| panic!("`{command}`: {e}"));
    }
    let empty = logicaffeine_kernel::interface::TermParser::parse(
        "fun (x : Len.{0} Nat (pcons.{0} Nat Zero (pnil.{0} Nat))) => \
         match x return (fun (l : PL.{0} Nat) (q : Len.{0} Nat l) => False) with end",
    )
    .expect("parses");
    let ctx = repl.context();
    assert!(
        infer_type(ctx, &empty).is_ok(),
        "`lnil` cannot produce a `pcons` index: {:?}",
        infer_type(ctx, &empty)
    );
    assert!(
        recheck(ctx, &empty).is_ok(),
        "nor in the re-checker: {:?}",
        recheck(ctx, &empty)
    );
    assert_eq!(double_check(ctx, &empty), DoubleCheck::Agreed);
}

/// SPEC F's COMPUTATION half, and the trap in it. A pruned case list is SHORTER than the
/// constructor list, so the ι-rule cannot index it by the constructor's registration position —
/// the second of two constructors is the first of a one-case list. Getting that wrong does not
/// fail; it applies the WRONG BRANCH to the right arguments and hands back a plausible value.
///
/// The three-constructor shape is the one that exposes it: prune the FIRST constructor and the
/// remaining two shift by one, so a position-indexed reducer returns the third branch for the
/// second constructor.
#[test]
fn a_pruned_indexed_match_computes_the_branch_it_kept() {
    let mut ctx = std_ctx();
    // `Tri (A : Type 0) : Nat → Type 0` with THREE constructors at three different indices:
    //   T0 : Π(A). Tri A Zero
    //   T1 : Π(A). A → Tri A (Succ Zero)
    //   T2 : Π(A). A → A → Tri A (Succ (Succ Zero))
    let succ = |t: Term| app(g("Succ"), t);
    ctx.add_indexed_inductive("Tri", pi("A", type0(), arrow(g("Nat"), type0())), 1);
    ctx.add_constructor(
        "T0",
        "Tri",
        pi("A", type0(), apps(g("Tri"), &[var("A"), g("Zero")])),
    );
    ctx.add_constructor(
        "T1",
        "Tri",
        pi(
            "A",
            type0(),
            arrow(var("A"), apps(g("Tri"), &[var("A"), succ(g("Zero"))])),
        ),
    );
    ctx.add_constructor(
        "T2",
        "Tri",
        pi(
            "A",
            type0(),
            arrow(
                var("A"),
                arrow(var("A"), apps(g("Tri"), &[var("A"), succ(succ(g("Zero")))])),
            ),
        ),
    );

    // A match at index `Succ Zero`: only `T1` survives, and it is constructor NUMBER TWO.
    let at_one = |n: Term| apps(g("Tri"), &[g("Nat"), n]);
    let motive = lam("i", g("Nat"), lam("_t", at_one(var("i")), g("Nat")));
    let only_t1 = Term::Match {
        discriminant: Box::new(apps(g("T1"), &[g("Nat"), nat_lit(7)])),
        motive: Box::new(motive.clone()),
        cases: vec![lam("x", g("Nat"), var("x"))],
    };
    assert!(
        infer_type(&ctx, &only_t1).is_ok(),
        "`Succ Zero` rules out `T0` (index `Zero`) and `T2` (index `Succ (Succ Zero)`): {:?}",
        infer_type(&ctx, &only_t1)
    );
    // THE POINT. The kept branch is the identity on its field, so the answer is the field. A
    // reducer that indexed the one-case list by `T1`'s registration position (1) would find
    // nothing and get stuck; one that silently wrapped would return the wrong branch.
    assert_eq!(
        normalize(&ctx, &only_t1),
        nat_lit(7),
        "the surviving branch must be the one that RUNS — a pruned case list cannot be indexed \
         by the constructor's position in the full list"
    );

    // And the full-list form of the same match still computes identically, so pruning is an
    // option and never a requirement.
    let all_three = Term::Match {
        discriminant: Box::new(apps(g("T1"), &[g("Nat"), nat_lit(7)])),
        motive: Box::new(motive),
        cases: vec![
            g("Zero"),
            lam("x", g("Nat"), var("x")),
            lam("x", g("Nat"), lam("_y", g("Nat"), var("x"))),
        ],
    };
    assert!(
        infer_type(&ctx, &all_three).is_ok(),
        "supplying every branch stays legal"
    );
    assert_eq!(
        normalize(&ctx, &all_three),
        nat_lit(7),
        "and computes the same answer as the pruned form"
    );
}
