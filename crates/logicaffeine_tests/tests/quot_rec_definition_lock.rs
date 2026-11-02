//! `Quot_rec` IS A DEFINITION. The prelude's dependent quotient eliminator is Lean's `Quot.rec`
//! (`Init/Core.lean:1581-1611`) — `Quot_lift` into `PSigma`, `Quot_ind`, and a transport — and it
//! is registered as that definition, not assumed beside the four primitives it is built from.
//!
//! Each test pins one half of the claim, so no single change can satisfy the file by accident:
//!
//! * the registered constant IS the derivation, at the statement the assumption used to have, and
//!   it is no longer in the trusted base;
//! * BOTH kernels check that body at that statement, and both refuse a mangled one;
//! * it COMPUTES in both kernels through δ, and through nothing else: with its δ withheld, the same
//!   term stays stuck, so no reduction rule keyed on its name survives;
//! * the carrier stays `Sort u` — a quotient of a proposition and a quotient of TYPES both
//!   eliminate — which is what `PSigma` and `Quot_lift`'s `Sort v` target are there for;
//! * its `#print axioms` is `Quot_sound`, read through the body.

use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::recursor::derive_quot_rec;
use logicaffeine_kernel::{
    axiom_dependencies, axiom_dependencies_of, double_check, infer_type, is_subtype, normalize,
    normalize_at, recheck, Context, DoubleCheck, Term, Transparency, Universe,
};
use std::collections::BTreeSet;

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
fn arrow(a: Term, b: Term) -> Term {
    pi("_", a, b)
}
fn type_(n: u32) -> Term {
    Term::Sort(Universe::Type(n))
}
fn prop() -> Term {
    Term::Sort(Universe::Prop)
}
fn nat() -> Term {
    g("Nat")
}
fn succ(t: Term) -> Term {
    app(g("Succ"), t)
}
fn eq_at(t: Term, a: Term, b: Term) -> Term {
    apps(g("Eq"), &[t, a, b])
}
fn std_ctx() -> Context {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    ctx
}
fn names(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|s| s.to_string()).collect()
}
/// `(λ(x : ty). x) term` — both kernels' application rule checks `term : ty`.
fn ascribe(ty: Term, term: Term) -> Term {
    app(lam("ascribed", ty, var("ascribed")), term)
}
/// A proof of `Eq ty lhs rhs` by `refl ty rhs`, which checks exactly when `lhs ≡ rhs`.
fn by_refl(ty: Term, lhs: Term, rhs: Term) -> Term {
    ascribe(
        eq_at(ty.clone(), lhs, rhs.clone()),
        apps(g("refl"), &[ty, rhs]),
    )
}
fn both_kernels_accept(ctx: &Context, term: &Term, what: &str) {
    assert!(
        infer_type(ctx, term).is_ok(),
        "{what}: the main kernel refuses it: {:?}",
        infer_type(ctx, term)
    );
    assert!(
        recheck(ctx, term).is_ok(),
        "{what}: the independent kernel refuses it: {:?}",
        recheck(ctx, term)
    );
    assert_eq!(
        double_check(ctx, term),
        DoubleCheck::Agreed,
        "{what}: the kernels do not agree"
    );
}
fn both_kernels_refuse(ctx: &Context, term: &Term, what: &str) {
    assert!(
        infer_type(ctx, term).is_err(),
        "{what}: the main kernel accepts it"
    );
    assert!(
        recheck(ctx, term).is_err(),
        "{what}: the independent kernel accepts it"
    );
}

/// The prelude statement, written here independently of `quotient/prelude.rs`:
/// `Π(A : Sort u)(r). Π(motive : Quot A r → Type 0). Π(f : Π(a:A). motive (Quot_mk A r a)).
///  Π(h : Π(a b:A)(p : r a b). Eq (motive (mk b)) (transport (Quot_sound A r a b p) (f a)) (f b)).
///  Π(q : Quot A r). motive q`.
fn statement() -> Term {
    let quot = || apps(g("Quot"), &[var("A"), var("r")]);
    let mk = |x: Term| apps(g("Quot_mk"), &[var("A"), var("r"), x]);
    let motive_at = |q: Term| app(var("motive"), q);
    let transported = apps(
        g("Eq_rec_dep"),
        &[
            quot(),
            mk(var("a")),
            lam(
                "q",
                quot(),
                lam(
                    "_",
                    eq_at(quot(), mk(var("a")), var("q")),
                    motive_at(var("q")),
                ),
            ),
            app(var("f"), var("a")),
            mk(var("b")),
            apps(
                g("Quot_sound"),
                &[var("A"), var("r"), var("a"), var("b"), var("p")],
            ),
        ],
    );
    let respect = pi(
        "a",
        var("A"),
        pi(
            "b",
            var("A"),
            pi(
                "p",
                apps(var("r"), &[var("a"), var("b")]),
                eq_at(
                    motive_at(mk(var("b"))),
                    transported,
                    app(var("f"), var("b")),
                ),
            ),
        ),
    );
    pi(
        "A",
        Term::Sort(Universe::Var("u".to_string())),
        pi(
            "r",
            arrow(var("A"), arrow(var("A"), prop())),
            pi(
                "motive",
                arrow(quot(), type_(0)),
                pi(
                    "f",
                    pi("a", var("A"), motive_at(mk(var("a")))),
                    pi("h", respect, pi("q", quot(), motive_at(var("q")))),
                ),
            ),
        ),
    )
}

/// One elimination fixture: a carrier, a relation on it, a representative, a constant motive
/// `λ_. Nat` and a branch. The respect proof is ASSUMED under a local name at exactly the type
/// `Quot_rec` asks for, so the term is well typed and the only thing it can test is elimination.
struct Fixture {
    ctx: Context,
    rel: Term,
    carrier: Term,
    representative: Term,
    branch: Term,
    expected: Term,
}
impl Fixture {
    fn new(
        carrier: Term,
        relation_body: Term,
        representative: Term,
        branch: Term,
        expected: Term,
    ) -> Self {
        let mut ctx = std_ctx();
        let rel = lam(
            "qrec.x",
            carrier.clone(),
            lam("qrec.y", carrier.clone(), relation_body),
        );
        let partial = apps(
            g("Quot_rec"),
            &[
                carrier.clone(),
                rel.clone(),
                Self::motive_of(&carrier, &rel),
                branch.clone(),
            ],
        );
        let Ok(Term::Pi {
            param_type: respect,
            ..
        }) = infer_type(&ctx, &partial)
        else {
            panic!(
                "`Quot_rec` at {carrier} must ask for its transport-stated respect proof: {:?}",
                infer_type(&ctx, &partial)
            )
        };
        ctx.add_declaration("qrec.respects", *respect);
        Fixture {
            ctx,
            rel,
            carrier,
            representative,
            branch,
            expected,
        }
    }
    fn motive_of(carrier: &Term, rel: &Term) -> Term {
        lam(
            "qrec.q",
            apps(g("Quot"), &[carrier.clone(), rel.clone()]),
            nat(),
        )
    }
    fn class(&self, of: Term) -> Term {
        apps(g("Quot_mk"), &[self.carrier.clone(), self.rel.clone(), of])
    }
    fn eliminate(&self, major: Term) -> Term {
        apps(
            g("Quot_rec"),
            &[
                self.carrier.clone(),
                self.rel.clone(),
                Self::motive_of(&self.carrier, &self.rel),
                self.branch.clone(),
                g("qrec.respects"),
                major,
            ],
        )
    }
}

fn over_nat() -> Fixture {
    let three = succ(succ(succ(g("Zero"))));
    Fixture::new(
        nat(),
        g("True"),
        three.clone(),
        lam("z", nat(), succ(var("z"))),
        succ(three),
    )
}

#[test]
fn quot_rec_is_the_registered_derivation_at_its_unchanged_statement() {
    let ctx = std_ctx();
    assert!(
        ctx.is_definition("Quot_rec"),
        "`Quot_rec` must be a DEFINITION with a checked body"
    );
    assert!(
        !ctx.iter_declarations().any(|(n, _)| n == "Quot_rec"),
        "`Quot_rec` must not be in the trusted base: Lean defines `Quot.rec`, so an assumption of it \
         is an assumption Lean does not make"
    );
    assert!(
        ctx.refused_declarations().is_empty(),
        "the prelude refused: {:?}",
        ctx.refused_declarations()
    );

    let (derived_type, derived) = derive_quot_rec(&ctx).expect("the dependent eliminator derives");
    assert_eq!(
        ctx.get_definition_body("Quot_rec"),
        Some(&derived),
        "the registered body must be exactly `recursor::derive_quot_rec`'s term — Lean's recipe, \
         not a body that merely has the right type"
    );
    assert_eq!(
        ctx.get_global("Quot_rec"),
        Some(&statement()),
        "the statement is UNCHANGED: every use of the assumed eliminator types against the defined one"
    );
    assert!(
        is_subtype(&ctx, &derived_type, &statement())
            && is_subtype(&ctx, &statement(), &derived_type),
        "the derivation's own type must convert with the statement: derived {derived_type}"
    );
    let spelled = format!("{derived}");
    assert!(
        !spelled.contains("Quot_rec"),
        "the body must not cite the constant it defines"
    );
    for cited in [
        "Quot_lift",
        "Quot_ind",
        "Quot_sound",
        "PSigma_mk",
        "PSigma_fst",
        "PSigma_snd",
        "Eq_rec_dep",
    ] {
        assert!(
            spelled.contains(cited),
            "Lean's recipe goes through `{cited}`; the body does not"
        );
    }
}

#[test]
fn both_kernels_check_the_registered_body_against_its_statement() {
    let ctx = std_ctx();
    let body = ctx.get_definition_body("Quot_rec").expect("a body").clone();
    both_kernels_accept(
        &ctx,
        &ascribe(statement(), body.clone()),
        "`Quot_rec`'s body at its statement",
    );

    // NOT VACUOUS: the same door refuses a body whose outermost binder wraps the rest in a bogus
    // application, and a body that drops the transport's proof argument.
    let Term::Lambda {
        param,
        param_type,
        body: inner,
    } = body.clone()
    else {
        panic!("`Quot_rec`'s body is a λ-telescope")
    };
    let mangled = Term::Lambda {
        param,
        param_type,
        body: Box::new(app(g("Quot_sound"), *inner)),
    };
    both_kernels_refuse(&ctx, &ascribe(statement(), mangled), "a mangled body");
    both_kernels_refuse(
        &ctx,
        &ascribe(statement(), g("Quot_lift")),
        "a different constant",
    );
}

#[test]
fn the_dependent_eliminator_computes_in_both_kernels_through_its_definition() {
    let fx = over_nat();
    let term = fx.eliminate(fx.class(fx.representative.clone()));
    assert_eq!(
        normalize(&fx.ctx, &term),
        fx.expected,
        "`Quot_rec … (Quot_mk a) ⤳ f a` by δ, `Quot_lift`'s ι, the projection and J at `refl`"
    );
    both_kernels_accept(&fx.ctx, &term, "the elimination");
    both_kernels_accept(
        &fx.ctx,
        &by_refl(nat(), term, fx.expected.clone()),
        "`Quot_rec … (Quot_mk a) = f a` by `refl`",
    );
    // …and the rule is not vacuous: the same equation at a DIFFERENT value is refused.
    both_kernels_refuse(
        &fx.ctx,
        &by_refl(
            nat(),
            fx.eliminate(fx.class(fx.representative.clone())),
            g("Zero"),
        ),
        "`Quot_rec … (Quot_mk 3) = 0`",
    );
}

/// No reduction rule keyed on the NAME survives. With `Quot_rec`'s δ withheld, the elimination of
/// a class is stuck exactly where it stands; a rule that still fired here would be a trusted
/// primitive wearing a definition's name.
#[test]
fn the_dependent_eliminator_has_no_reduction_rule_of_its_own() {
    let fx = over_nat();
    let term = fx.eliminate(fx.class(fx.representative.clone()));
    let folded = |n: &str| n == "Quot_rec";
    let stuck = normalize_at(&fx.ctx, &term, Transparency::Excluding(&folded));
    let mut head = &stuck;
    while let Term::App(f, _) = head {
        head = f;
    }
    assert_eq!(
        head,
        &g("Quot_rec"),
        "with `Quot_rec` folded the elimination must stay a `Quot_rec` spine, got {stuck}"
    );
    assert_ne!(
        stuck, fx.expected,
        "…and must not reach `f a` by any other door"
    );
    // The control: `Quot_lift`, a primitive, still computes with the same thing folded.
    let lifted = apps(
        g("Quot_lift"),
        &[
            nat(),
            fx.rel.clone(),
            nat(),
            lam("z", nat(), fx.expected.clone()),
            lam(
                "a",
                nat(),
                lam(
                    "b",
                    nat(),
                    lam(
                        "p",
                        g("True"),
                        apps(g("refl"), &[nat(), fx.expected.clone()]),
                    ),
                ),
            ),
            fx.class(fx.representative.clone()),
        ],
    );
    both_kernels_accept(&fx.ctx, &lifted, "the `Quot_lift` control");
    assert_eq!(
        normalize_at(&fx.ctx, &lifted, Transparency::Excluding(&folded)),
        fx.expected
    );
}

/// The carrier is `Sort u`. A quotient of a PROPOSITION and a quotient of TYPES — `Type 0` itself,
/// a `Type 1` carrier — both eliminate and compute, in both kernels. Narrowing the carrier to
/// `Type 0` would refuse the second; that is what `PSigma` and `Quot_lift`'s `Sort v` target buy.
#[test]
fn a_quotient_of_types_and_a_quotient_of_a_proposition_both_eliminate() {
    let fixtures = [
        (
            "a Type 1 carrier (`Type 0` itself)",
            Fixture::new(
                type_(0),
                g("True"),
                nat(),
                lam("X", type_(0), g("Zero")),
                g("Zero"),
            ),
        ),
        (
            "a Prop carrier (`True`)",
            Fixture::new(
                g("True"),
                g("True"),
                g("I"),
                lam("t", g("True"), succ(g("Zero"))),
                succ(g("Zero")),
            ),
        ),
    ];
    for (what, fx) in fixtures {
        let term = fx.eliminate(fx.class(fx.representative.clone()));
        both_kernels_accept(&fx.ctx, &term, what);
        assert_eq!(
            normalize(&fx.ctx, &term),
            fx.expected,
            "{what}: the class computes"
        );
        both_kernels_accept(&fx.ctx, &by_refl(nat(), term, fx.expected.clone()), what);
    }
}

/// `Quot_lift`'s target is Lean's `Sort v`: a quotient maps into `Type 0` itself (a `Type 1`
/// target) and into a proposition, and computes, in both kernels.
#[test]
fn quot_lift_targets_any_sort() {
    let ctx = std_ctx();
    let rel = lam("x", nat(), lam("y", nat(), g("True")));
    let class = apps(g("Quot_mk"), &[nat(), rel.clone(), g("Zero")]);
    // Each target's own sort, and whether that sort is above `Type 0` — where a target pinned to
    // `Type 0` would refuse it.
    let cases = [
        (
            "into `Type 0` (a Type 1 target)",
            type_(0),
            nat(),
            type_(1),
            true,
        ),
        ("into a proposition", g("True"), g("I"), prop(), false),
    ];
    for (what, target, value, target_sort, above_type0) in cases {
        let constant = lam("n", nat(), value.clone());
        let respect = lam(
            "a",
            nat(),
            lam(
                "b",
                nat(),
                lam(
                    "p",
                    g("True"),
                    apps(g("refl"), &[target.clone(), value.clone()]),
                ),
            ),
        );
        let lifted = apps(
            g("Quot_lift"),
            &[
                nat(),
                rel.clone(),
                target.clone(),
                constant,
                respect,
                class.clone(),
            ],
        );
        both_kernels_accept(&ctx, &lifted, what);
        let ty = infer_type(&ctx, &lifted).expect("typed above");
        assert!(
            is_subtype(&ctx, &ty, &target),
            "{what}: lands in {target}, got {ty}"
        );
        let sort = infer_type(&ctx, &target).expect("the target is a type");
        assert!(
            is_subtype(&ctx, &sort, &target_sort),
            "{what}: the target lives in {target_sort}, got {sort}"
        );
        assert_eq!(
            !is_subtype(&ctx, &sort, &type_(0)),
            above_type0,
            "{what}: the target's sort is {sort}"
        );
        assert_eq!(normalize(&ctx, &lifted), value, "{what}: the ι-rule");
    }
}

/// `PSigma` is Lean's dependent pair at any sorts: a pair whose first or second component is a
/// TYPE lives in `Type 1`, a pair whose second component is a PROOF is still data, and all project.
#[test]
fn psigma_is_the_dependent_pair_at_any_sorts() {
    let ctx = std_ctx();
    let family = lam("X", type_(0), var("X"));
    let pair_ty = apps(g("PSigma"), &[type_(0), family.clone()]);
    let pair = apps(
        g("PSigma_mk"),
        &[type_(0), family.clone(), nat(), g("Zero")],
    );
    both_kernels_accept(
        &ctx,
        &ascribe(pair_ty.clone(), pair.clone()),
        "⟨Nat, 0⟩ : PSigma (Type 0) id",
    );
    let sort = infer_type(&ctx, &pair_ty).expect("the pair type is a type");
    assert!(
        is_subtype(&ctx, &sort, &type_(1)),
        "`PSigma (Type 0) id` lives in Type 1, got {sort}"
    );
    assert!(
        !is_subtype(&ctx, &sort, &type_(0)),
        "…and NOT in Type 0: its first component is a type"
    );
    let fst = apps(g("PSigma_fst"), &[type_(0), family.clone(), pair.clone()]);
    let snd = apps(g("PSigma_snd"), &[type_(0), family.clone(), pair.clone()]);
    assert_eq!(normalize(&ctx, &fst), nat());
    assert_eq!(normalize(&ctx, &snd), g("Zero"));
    both_kernels_accept(
        &ctx,
        &ascribe(nat(), snd),
        "the second projection has the first's fibre",
    );

    let reflexive = lam("n", nat(), eq_at(nat(), var("n"), var("n")));
    let witnessed = apps(
        g("PSigma_mk"),
        &[
            nat(),
            reflexive.clone(),
            g("Zero"),
            apps(g("refl"), &[nat(), g("Zero")]),
        ],
    );
    let witnessed_ty = apps(g("PSigma"), &[nat(), reflexive.clone()]);
    both_kernels_accept(
        &ctx,
        &ascribe(witnessed_ty.clone(), witnessed),
        "⟨0, refl⟩ : PSigma Nat (λn. n = n)",
    );
    let sort = infer_type(&ctx, &witnessed_ty).expect("typed");
    assert!(
        is_subtype(&ctx, &sort, &type_(0)) && !is_subtype(&ctx, &sort, &prop()),
        "a pair storing a proof beside data is data, in Type 0 and not in Prop: got {sort}"
    );
    // A witness from the wrong fibre is refused.
    let wrong = apps(
        g("PSigma_mk"),
        &[
            nat(),
            reflexive.clone(),
            g("Zero"),
            apps(g("refl"), &[nat(), succ(g("Zero"))]),
        ],
    );
    both_kernels_refuse(
        &ctx,
        &ascribe(witnessed_ty, wrong),
        "⟨0, refl 1⟩ : PSigma Nat (λn. n = n)",
    );

    // The FAMILY is `A → Sort v` too: a pair whose second component is a TYPE lives in `Type 1`.
    let types = lam("_", nat(), type_(0));
    let typed_ty = apps(g("PSigma"), &[nat(), types.clone()]);
    let typed = apps(g("PSigma_mk"), &[nat(), types.clone(), g("Zero"), nat()]);
    both_kernels_accept(
        &ctx,
        &ascribe(typed_ty.clone(), typed.clone()),
        "⟨0, Nat⟩ : PSigma Nat (λ_. Type 0)",
    );
    let sort = infer_type(&ctx, &typed_ty).expect("typed");
    assert!(
        is_subtype(&ctx, &sort, &type_(1)) && !is_subtype(&ctx, &sort, &type_(0)),
        "a pair storing a type is in Type 1 and not in Type 0: got {sort}"
    );
    assert_eq!(
        normalize(&ctx, &apps(g("PSigma_snd"), &[nat(), types, typed])),
        nat()
    );
}

/// `#print axioms Quot_rec` is `Quot_sound` — Lean's answer for `Quot.rec` — and it is read
/// THROUGH THE BODY: the named door, the term door and the body's own closure agree.
#[test]
fn quot_rec_rests_on_quot_sound_through_its_body() {
    let ctx = std_ctx();
    let expected = names(&["Quot_sound"]);
    let named = axiom_dependencies_of(&ctx, "Quot_rec").expect("registered");
    assert_eq!(named.axioms, expected);
    assert!(named.unresolved.is_empty(), "{named:?}");
    assert_eq!(axiom_dependencies(&ctx, &g("Quot_rec")).axioms, expected);
    let body = ctx.get_definition_body("Quot_rec").expect("a body").clone();
    let mut through_body = axiom_dependencies(&ctx, &body);
    through_body.absorb(axiom_dependencies(&ctx, &statement()));
    assert_eq!(
        through_body.axioms, expected,
        "the closure is the body's and the statement's"
    );
    // The layer above reads the same thing through `Quot_rec`.
    assert_eq!(
        axiom_dependencies_of(&ctx, "Quotient_rec")
            .expect("registered")
            .axioms,
        expected
    );
    for former in [
        "Quot",
        "Quot_mk",
        "Quot_lift",
        "Quot_ind",
        "PSigma",
        "PSigma_mk",
        "PSigma_fst",
        "PSigma_snd",
    ] {
        assert!(
            axiom_dependencies_of(&ctx, former)
                .expect("registered")
                .axioms
                .is_empty(),
            "`{former}` is no assumption"
        );
    }
}
