//! The DERIVED AUXILIARIES — `noConfusionType`/`noConfusion`, `inj`, `below`/`brecOn`,
//! `decEq…` — and the motive-sort rule they share with `derive_recursor`.
//!
//! Lean synthesises this family for EVERY inductive at declaration time. These are the locks
//! on our derivations of it. Each positive is paired with the neighbouring thing that must
//! still be REFUSED (a positive-only test is satisfiable by a weak implementation), each
//! computed type is pinned by ι-reduction rather than by "it registered", and every derived
//! term is put through `double_check` — two independent kernels, because the whole point of
//! deriving instead of axiomatising is that somebody CHECKS it.

use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::recursor::{
    derive_auxiliaries, derive_below, derive_brec_on, derive_ctor_inj, derive_dec_eq,
    derive_no_confusion, derive_no_confusion_type, derive_recursor,
};
use logicaffeine_kernel::{
    double_check, infer_type, normalize, resolve_instance, Context, DoubleCheck, MetaCtx,
    MutualInductive, Term, Universe,
};

// ---------------------------------------------------------------------------------------
// builders
// ---------------------------------------------------------------------------------------

fn g(s: &str) -> Term {
    Term::Global(s.to_string())
}
fn v(s: &str) -> Term {
    Term::Var(s.to_string())
}
fn app(f: Term, x: Term) -> Term {
    Term::App(Box::new(f), Box::new(x))
}
fn apps(f: Term, xs: &[Term]) -> Term {
    xs.iter().fold(f, |acc, x| app(acc, x.clone()))
}
fn lam(p: &str, t: Term, b: Term) -> Term {
    Term::Lambda {
        param: p.to_string(),
        param_type: Box::new(t),
        body: Box::new(b),
    }
}
fn pi(p: &str, t: Term, b: Term) -> Term {
    Term::Pi {
        param: p.to_string(),
        param_type: Box::new(t),
        body_type: Box::new(b),
    }
}
fn arrow(a: Term, b: Term) -> Term {
    pi("_", a, b)
}
fn ty(n: u32) -> Term {
    Term::Sort(Universe::Type(n))
}
fn prop() -> Term {
    Term::Sort(Universe::Prop)
}
fn nat() -> Term {
    g("Nat")
}
fn succ(n: Term) -> Term {
    app(g("Succ"), n)
}
fn peano(n: u32) -> Term {
    (0..n).fold(g("Zero"), |acc, _| succ(acc))
}
fn eq3(t: Term, a: Term, b: Term) -> Term {
    apps(g("Eq"), &[t, a, b])
}
fn prod(a: Term, b: Term) -> Term {
    apps(g("Prod"), &[a, b])
}

fn std_ctx() -> Context {
    let mut c = Context::new();
    StandardLibrary::register(&mut c);
    c
}

/// `Chain := cstop | clink (Nat) (Chain)` — one nullary and one two-field recursive
/// constructor, which is the smallest shape that exercises disjointness AND injectivity AND
/// a recursive below-entry at once.
fn register_chain(ctx: &mut Context) {
    ctx.add_inductive("Chain", ty(0));
    ctx.add_constructor_checked("cstop", "Chain", g("Chain"))
        .expect("cstop registers");
    ctx.add_constructor_checked(
        "clink",
        "Chain",
        arrow(nat(), arrow(g("Chain"), g("Chain"))),
    )
    .expect("clink is strictly positive");
}

/// A three-constructor enumeration, all nullary — the `deriving DecidableEq` bread-and-butter.
fn register_hue(ctx: &mut Context) {
    ctx.add_inductive("Hue", ty(0));
    for c in ["hred", "hgreen", "hblue"] {
        ctx.add_constructor_checked(c, "Hue", g("Hue"))
            .expect("a nullary constructor registers");
    }
}

/// Register a derived auxiliary and assert BOTH kernels certified it. `derive_*` already ran
/// `infer_type`; `double_check` is the independent re-derivation, and it is the assertion that
/// actually matters — a derivation both kernels miscount identically is the bug class this
/// campaign keeps finding.
fn register_checked(ctx: &mut Context, name: &str, derived: (Term, Term)) {
    let (t, body) = derived;
    match double_check(ctx, &body) {
        DoubleCheck::Agreed => {}
        other => panic!("both kernels must certify `{name}`, got {other:?}"),
    }
    ctx.add_definition(name.to_string(), t, body);
}

/// Every `Global` a term cites.
fn cited_globals(t: &Term, out: &mut Vec<String>) {
    match t {
        Term::Global(n) => out.push(n.clone()),
        Term::Var(_) | Term::Sort(_) | Term::Lit(_) | Term::Hole | Term::Const { .. } => {}
        Term::Annotated { term, .. } => cited_globals(term, out),
        Term::Pi {
            param_type,
            body_type,
            ..
        } => {
            cited_globals(param_type, out);
            cited_globals(body_type, out);
        }
        Term::Lambda {
            param_type, body, ..
        } => {
            cited_globals(param_type, out);
            cited_globals(body, out);
        }
        Term::App(f, a) => {
            cited_globals(f, out);
            cited_globals(a, out);
        }
        Term::Match {
            discriminant,
            motive,
            cases,
        } => {
            cited_globals(discriminant, out);
            cited_globals(motive, out);
            for c in cases {
                cited_globals(c, out);
            }
        }
        Term::Fix { body, .. } => cited_globals(body, out),
        Term::MutualFix { defs, .. } => {
            for (_, b) in defs {
                cited_globals(b, out);
            }
        }
        Term::Let {
            ty, value, body, ..
        } => {
            cited_globals(ty, out);
            cited_globals(value, out);
            cited_globals(body, out);
        }
    }
}

/// Assert a derived term rests on NO axiom: every global it cites is an inductive, a
/// constructor, or a definition with a body. This is rule 4 of the fix brief as a test —
/// axiomatising `noConfusion` would turn every `injection` proof into an assumption, and the
/// only way that stays true is if nothing in the derivation ever reaches for a declaration.
fn assert_axiom_free(ctx: &Context, what: &str, term: &Term) {
    let mut cited = Vec::new();
    cited_globals(term, &mut cited);
    cited.sort();
    cited.dedup();
    for n in cited {
        if n == "_" {
            continue;
        }
        assert!(
            ctx.get_global(&n).is_some(),
            "`{what}` cites `{n}`, which is not in scope at all"
        );
        assert!(
            ctx.is_definition(&n) || ctx.is_constructor(&n) || ctx.is_inductive(&n),
            "`{what}` rests on the AXIOM `{n}` — a derived auxiliary must be checked, not assumed"
        );
    }
}

// =======================================================================================
// The motive-sort rule (T04-006's boundary, above and below)
// =======================================================================================

/// The motive's target sort, read off a derived recursor's first binder that ends in a `Sort`.
fn motive_target_of(rec_ty: &Term, head: &str) -> Term {
    let mut cur = rec_ty;
    while let Term::Pi {
        param_type,
        body_type,
        ..
    } = cur
    {
        // The motive is the binder `Π indices. Π(major : head …). <target>`.
        let mut inner = param_type.as_ref();
        let mut last_dom: Option<&Term> = None;
        while let Term::Pi {
            param_type: d,
            body_type: b,
            ..
        } = inner
        {
            last_dom = Some(d);
            inner = b;
        }
        if let (Term::Sort(_), Some(dom)) = (inner, last_dom) {
            let mut h = dom;
            while let Term::App(f, _) = h {
                h = f;
            }
            if matches!(h, Term::Global(n) if n == head) {
                return inner.clone();
            }
        }
        cur = body_type;
    }
    panic!("no motive binder found in {rec_ty}");
}

#[test]
fn a_derived_recursor_targets_its_own_sort_in_both_directions() {
    // The rule is the inductive's OWN sort — pinned from BOTH sides, because either half alone
    // is satisfiable by a constant. A `Type 0` inductive gets `Type 0`…
    let mut c = std_ctx();
    let (rec_ty, body) = derive_recursor(&c, "Nat").expect("Nat's recursor derives");
    assert_eq!(motive_target_of(&rec_ty, "Nat"), ty(0));
    c.add_definition("NatRec".to_string(), rec_ty, body);
    let add = apps(
        g("NatRec"),
        &[
            lam("_", nat(), nat()),
            peano(5),
            lam("_", nat(), lam("ih", nat(), succ(v("ih")))),
            peano(2),
        ],
    );
    assert!(
        infer_type(&c, &add).is_ok(),
        "the `Type 0` motive must be accepted"
    );
    assert_eq!(normalize(&c, &add), peano(7), "…and ι-compute: 5 + 2 = 7");

    // …and a `Type 1` inductive gets `Type 1`, so it can recover the `Type 0` its constructor
    // stores. A recursor pinned to `Type 0` accepts the declaration and leaves the type
    // unusable, which is the shape T11-019 and T01-008 are about.
    let mut d = std_ctx();
    d.add_inductive("Boxed", ty(1));
    d.add_constructor_checked("boxed", "Boxed", arrow(ty(0), g("Boxed")))
        .expect("boxed registers");
    let (boxed_rec, boxed_tm) = derive_recursor(&d, "Boxed").expect("Boxed's recursor derives");
    assert_eq!(motive_target_of(&boxed_rec, "Boxed"), ty(1));
    d.add_definition("BoxedRec".to_string(), boxed_rec, boxed_tm);
    let unbox = apps(
        g("BoxedRec"),
        &[lam("_", g("Boxed"), ty(0)), lam("a", ty(0), v("a"))],
    );
    assert!(
        infer_type(&d, &unbox).is_ok(),
        "recovering the `Type 0` a `Type 1` inductive stores must type-check: {:?}",
        infer_type(&d, &unbox)
    );

    // THE CEILING, recorded rather than wished away: a monomorphic motive cannot ALSO reach
    // above the inductive's own sort, so a `Type 1`-valued family built by recursion over
    // `Nat` — Lean's `Nat.rec (motive := fun _ => Type 1)` — still has no image. The own sort
    // is the largest level that does not cap some other inductive below ITS own; closing this
    // needs universe-polymorphic recursors, not a bigger constant here. Pinned so that a later
    // change has to confront the trade rather than silently re-break `Box1`/`AtLevelOne`.
    let tower = apps(
        g("NatRec"),
        &[
            lam("_", nat(), ty(0)),
            nat(),
            lam("_", nat(), lam("ih", ty(0), v("ih"))),
            peano(2),
        ],
    );
    assert!(
        infer_type(&c, &tower).is_err(),
        "a `Type 1` motive over a `Type 0` inductive needs universe polymorphism; if this ever \
         starts passing, check that `Box0`/`Box1` and `AtLevelOne` still get their own level"
    );
}

#[test]
fn a_mutual_block_member_gets_the_same_motive_rule_as_a_standalone_one() {
    // A member must not get a WEAKER eliminator for being declared in company. The mutual path
    // used to compute the motive target with its own `match`, pinned to `Type 0`, so a `Type 1`
    // member's motive sat one level below its own fields — the exact defect T11-019 names,
    // surviving in the copy nobody looked at.
    let mut c = std_ctx();
    let block = vec![
        MutualInductive {
            name: "Hi".to_string(),
            sort: ty(1),
            num_params: 0,
            constructors: vec![
                ("hi_here".to_string(), arrow(ty(0), g("Hi"))),
                ("hi_there".to_string(), arrow(g("Lo"), g("Hi"))),
            ],
        },
        MutualInductive {
            name: "Lo".to_string(),
            sort: ty(1),
            num_params: 0,
            constructors: vec![("lo_stop".to_string(), g("Lo"))],
        },
    ];
    c.add_mutual_inductives(&block)
        .expect("the block registers");
    let (rec_ty, _) = derive_recursor(&c, "Hi").expect("the mutual recursor derives");
    assert_eq!(
        motive_target_of(&rec_ty, "Hi"),
        ty(1),
        "a `Type 1` mutual member's motive must reach its own level"
    );
    assert_eq!(
        motive_target_of(&rec_ty, "Lo"),
        ty(1),
        "…and so must its sibling's"
    );
}

#[test]
fn a_multi_constructor_prop_still_refuses_a_type_motive() {
    // THE NEGATIVE. Following the own sort must not leak into the sorts where it would be
    // unsound: a proposition with two constructors keeps its `Prop`-restricted induction
    // principle, or a *proof* could pick a `Type`-level value.
    let mut c = std_ctx();
    c.add_inductive("Flip", prop());
    c.add_constructor("fa", "Flip", g("Flip"));
    c.add_constructor("fb", "Flip", g("Flip"));
    let (rec_ty, body) = derive_recursor(&c, "Flip").expect("Flip's recursor derives");
    assert_eq!(
        motive_target_of(&rec_ty, "Flip"),
        prop(),
        "a 2-constructor Prop must keep a Prop motive"
    );
    c.add_definition("FlipRec".to_string(), rec_ty, body);
    let large = Term::Match {
        discriminant: Box::new(v("h")),
        motive: Box::new(lam("_", g("Flip"), nat())),
        cases: vec![g("Zero"), succ(g("Zero"))],
    };
    let open = c.extend("h", g("Flip"));
    assert!(
        infer_type(&open, &large).is_err(),
        "large elimination of a 2-constructor Prop must stay refused"
    );

    // A SUBSINGLETON Prop is the other side of the same guard and must reach `Type 0` — that
    // is `False`'s ex falso and `Eq`'s J, and folding it into the own-sort arm would return
    // `Prop` and destroy both.
    let mut e = Context::new();
    e.add_inductive("Void", prop());
    let (void_rec, _) = derive_recursor(&e, "Void").expect("Void's recursor derives");
    assert_eq!(
        motive_target_of(&void_rec, "Void"),
        ty(0),
        "a 0-constructor Prop is a subsingleton ⇒ its motive targets `Type 0`, not `Prop`"
    );

    // And `SProp` eliminates only into `SProp`: definitional irrelevance makes anything else
    // unsound, so this arm must NOT have been folded in either.
    let mut s = Context::new();
    s.add_inductive("SVoid", Term::Sort(Universe::SProp));
    let (svoid_rec, _) = derive_recursor(&s, "SVoid").expect("SVoid's recursor derives");
    assert_eq!(
        motive_target_of(&svoid_rec, "SVoid"),
        Term::Sort(Universe::SProp),
        "an SProp eliminates only into SProp"
    );
}

// =======================================================================================
// noConfusionType
// =======================================================================================

#[test]
fn no_confusion_type_computes_injectivity_on_the_diagonal_and_anything_off_it() {
    let mut ctx = std_ctx();
    register_chain(&mut ctx);
    let derived = derive_no_confusion_type(&ctx, "Chain").expect("Chain's noConfusionType derives");
    assert_axiom_free(&ctx, "Chain_noConfusionType", &derived.1);
    register_checked(&mut ctx, "Chain_noConfusionType", derived);

    let nct = |p: Term, a: Term, b: Term| apps(g("Chain_noConfusionType"), &[p, a, b]);
    let link = |n: Term, t: Term| apps(g("clink"), &[n, t]);

    // DIFFERENT constructors, both ways round: the type IS the goal, so a false equation
    // yields anything. This is the half `cases h` uses.
    for (a, b) in [
        (g("cstop"), link(peano(1), g("cstop"))),
        (link(peano(1), g("cstop")), g("cstop")),
    ] {
        assert_eq!(
            normalize(&ctx, &nct(g("False"), a.clone(), b.clone())),
            g("False"),
            "noConfusionType P {a} {b} must be P outright"
        );
    }

    // The SAME constructor: `(x⃗ = y⃗ → P) → P`, one equation per field, in field order. This
    // is the half `injection` uses, and the equations' ORDER and CARRIERS are the content.
    let lhs = link(peano(1), g("cstop"));
    let rhs = link(peano(2), link(peano(0), g("cstop")));
    let want = arrow(
        arrow(
            eq3(nat(), peano(1), peano(2)),
            arrow(
                eq3(g("Chain"), g("cstop"), link(peano(0), g("cstop"))),
                g("False"),
            ),
        ),
        g("False"),
    );
    assert_eq!(
        normalize(&ctx, &nct(g("False"), lhs, rhs)),
        normalize(&ctx, &want),
        "the same constructor must reduce to (x⃗ = y⃗ → P) → P"
    );

    // A NULLARY constructor against itself carries no information: `P → P`, not `P`. Getting
    // this wrong (emitting `P`) would make `cstop = cstop` refute everything.
    assert_eq!(
        normalize(&ctx, &nct(g("False"), g("cstop"), g("cstop"))),
        arrow(g("False"), g("False")),
        "a nullary constructor's diagonal is `P → P`"
    );
}

#[test]
fn no_confusion_is_refused_where_it_would_be_false_or_unstateable() {
    // THE NEGATIVES. Each of these is a shape a weak implementation would happily emit.

    // A PROPOSITION: proof irrelevance identifies any two inhabitants, so disjointness is
    // FALSE there. This must be a refusal with a reason, not a derivation.
    let mut p = std_ctx();
    p.add_inductive("Tri", prop());
    for c in ["tria", "trib"] {
        p.add_constructor_checked(c, "Tri", g("Tri"))
            .expect("a Prop constructor registers");
    }
    let e = derive_no_confusion_type(&p, "Tri").expect_err("a Prop has no disjointness");
    assert!(
        format!("{e:?}").contains("PROPOSITION"),
        "the refusal must say WHY a Prop has no no-confusion, got {e:?}"
    );
    assert!(
        derive_no_confusion(&p, "Tri").is_err(),
        "and neither half may be emitted"
    );

    // A carrier ABOVE `Type 0`: the prelude's `Eq` cannot even be applied to it, so the
    // statement does not exist. Refuse rather than emit something that fails later.
    let mut h = std_ctx();
    h.add_inductive("Big", ty(1));
    h.add_constructor_checked("big", "Big", arrow(ty(0), g("Big")))
        .expect("big registers");
    assert!(
        derive_no_confusion_type(&h, "Big").is_err(),
        "no `Eq` at `Type 1`, no equation"
    );

    // An INDEXED family: the impossible constructor pairs would need an index unifier this
    // kernel does not have. Refusing beats inventing an inhabitant for an unreachable branch.
    let mut i = std_ctx();
    i.add_indexed_inductive("Vek", pi("A", ty(0), pi("_", nat(), ty(0))), 1);
    i.add_constructor_checked(
        "vnil",
        "Vek",
        pi("A", ty(0), apps(g("Vek"), &[v("A"), g("Zero")])),
    )
    .expect("vnil registers");
    let e = derive_no_confusion_type(&i, "Vek").expect_err("an indexed family is refused");
    assert!(
        format!("{e:?}").contains("INDEXED"),
        "the refusal must name the index unifier, got {e:?}"
    );

    // Not an inductive at all.
    assert!(derive_no_confusion_type(&std_ctx(), "decide").is_err());
    assert!(derive_no_confusion_type(&std_ctx(), "no_such_thing").is_err());
}

// =======================================================================================
// noConfusion
// =======================================================================================

/// A context with `Chain` and its no-confusion pair registered and double-checked.
fn chain_ctx() -> Context {
    let mut ctx = std_ctx();
    register_chain(&mut ctx);
    let d = derive_no_confusion_type(&ctx, "Chain").expect("noConfusionType derives");
    register_checked(&mut ctx, "Chain_noConfusionType", d);
    let d = derive_no_confusion(&ctx, "Chain").expect("noConfusion derives");
    assert_axiom_free(&ctx, "Chain_noConfusion", &d.1);
    register_checked(&mut ctx, "Chain_noConfusion", d);
    ctx
}

#[test]
fn no_confusion_refutes_a_cross_constructor_equation() {
    let ctx = chain_ctx();
    let link = |n: Term, t: Term| apps(g("clink"), &[n, t]);
    let bad = eq3(g("Chain"), g("cstop"), link(peano(0), g("cstop")));

    // `λh. noConfusion False cstop (clink 0 cstop) h` — a proof of `¬(cstop = clink 0 cstop)`,
    // which is what `cases h` on an impossible equation is.
    let refute = lam(
        "h",
        bad.clone(),
        apps(
            g("Chain_noConfusion"),
            &[g("False"), g("cstop"), link(peano(0), g("cstop")), v("h")],
        ),
    );
    let t = infer_type(&ctx, &refute).expect("the refutation must type-check");
    assert_eq!(
        normalize(&ctx, &t),
        normalize(&ctx, &pi("h", bad, g("False"))),
        "it must be exactly `cstop = clink 0 cstop → False`"
    );
    assert!(matches!(double_check(&ctx, &refute), DoubleCheck::Agreed));
}

#[test]
fn no_confusion_recovers_each_constructor_argument() {
    // `injection h`, as a CLOSED function per field. Closed on purpose: the re-checker cannot
    // see bindings introduced by `Context::extend` (it rejects `Succ a` for an `a` bound that
    // way as an unbound local), so a `double_check` stated over an open context would be
    // vacuously unavailable rather than a second opinion.
    let ctx = chain_ctx();
    let link = |n: Term, t: Term| apps(g("clink"), &[n, t]);
    let lhs = link(v("a"), v("t"));
    let rhs = link(v("a2"), v("t2"));
    let hyp = eq3(g("Chain"), lhs.clone(), rhs.clone());

    // Field 0 and field 1, by choosing which equation the continuation returns. This is the
    // point of the derivation: no defaulted projection, so no inhabitant of the OTHER
    // constructor's fields is needed — which is what stops `Chain`'s `clink`-only projection
    // from being writable by hand at all.
    for (k, want_body) in [
        (0usize, eq3(nat(), v("a"), v("a2"))),
        (1usize, eq3(g("Chain"), v("t"), v("t2"))),
    ] {
        let cont = lam(
            "e0",
            eq3(nat(), v("a"), v("a2")),
            lam(
                "e1",
                eq3(g("Chain"), v("t"), v("t2")),
                v(if k == 0 { "e0" } else { "e1" }),
            ),
        );
        let proj = app(
            apps(
                g("Chain_noConfusion"),
                &[want_body.clone(), lhs.clone(), rhs.clone(), v("h")],
            ),
            cont,
        );
        let closed = lam(
            "a",
            nat(),
            lam(
                "t",
                g("Chain"),
                lam(
                    "a2",
                    nat(),
                    lam("t2", g("Chain"), lam("h", hyp.clone(), proj)),
                ),
            ),
        );
        let want = pi(
            "a",
            nat(),
            pi(
                "t",
                g("Chain"),
                pi(
                    "a2",
                    nat(),
                    pi("t2", g("Chain"), pi("h", hyp.clone(), want_body)),
                ),
            ),
        );
        let got = infer_type(&ctx, &closed)
            .unwrap_or_else(|e| panic!("field-{k} injectivity must type-check: {e:?}"));
        assert_eq!(
            got, want,
            "field-{k} injectivity must state exactly that field's equation"
        );
        match double_check(&ctx, &closed) {
            DoubleCheck::Agreed => {}
            other => panic!("both kernels must certify field-{k} injectivity, got {other:?}"),
        }
    }
}

#[test]
fn no_confusion_computes_when_the_equation_is_refl() {
    // The COMPUTATION rule. A `noConfusion` that only type-checks is useless to a tactic that
    // has to reduce it: at `refl` it must ι-compute all the way to the `refl`s it packed.
    let ctx = chain_ctx();
    let link = |n: Term, t: Term| apps(g("clink"), &[n, t]);
    let val = link(peano(2), g("cstop"));
    let goal = eq3(nat(), peano(2), peano(2));
    let cont = lam(
        "e0",
        goal.clone(),
        lam("e1", eq3(g("Chain"), g("cstop"), g("cstop")), v("e0")),
    );
    let applied = app(
        apps(
            g("Chain_noConfusion"),
            &[
                goal,
                val.clone(),
                val.clone(),
                apps(g("refl"), &[g("Chain"), val]),
            ],
        ),
        cont,
    );
    assert_eq!(
        normalize(&ctx, &applied),
        apps(g("refl"), &[nat(), peano(2)]),
        "noConfusion at `refl` must reduce to the field's own reflexivity"
    );
}

#[test]
fn ctor_inj_conjoins_every_field_and_a_nullary_constructor_has_none() {
    let mut ctx = chain_ctx();
    let d = derive_ctor_inj(&ctx, "Chain", "clink").expect("clink's injectivity derives");
    assert_axiom_free(&ctx, "clink_inj", &d.1);
    let inj_ty = d.0.clone();
    register_checked(&mut ctx, "clink_inj", d);

    // The type is exactly Lean's `C.inj`: the equation in, the conjunction of the field
    // equations out.
    let want = pi(
        "x0",
        nat(),
        pi(
            "x1",
            g("Chain"),
            pi(
                "y0",
                nat(),
                pi(
                    "y1",
                    g("Chain"),
                    pi(
                        "h",
                        eq3(
                            g("Chain"),
                            apps(g("clink"), &[v("x0"), v("x1")]),
                            apps(g("clink"), &[v("y0"), v("y1")]),
                        ),
                        apps(
                            g("And"),
                            &[
                                eq3(nat(), v("x0"), v("y0")),
                                eq3(g("Chain"), v("x1"), v("y1")),
                            ],
                        ),
                    ),
                ),
            ),
        ),
    );
    assert_eq!(inj_ty, want, "clink_inj must state exactly Lean's `C.inj`");

    // THE NEGATIVE: a nullary constructor has nothing to inject, and emitting a vacuous
    // lemma for it would be noise that reads as a fact.
    let e = derive_ctor_inj(&ctx, "Chain", "cstop").expect_err("cstop has no fields");
    assert!(format!("{e:?}").contains("no arguments"), "{e:?}");
    assert!(derive_ctor_inj(&ctx, "Chain", "not_a_ctor").is_err());
}

// =======================================================================================
// below / brecOn
// =======================================================================================

#[test]
fn below_packages_one_entry_per_recursive_field_and_unit_for_none() {
    let mut ctx = std_ctx();
    register_chain(&mut ctx);
    let d = derive_below(&ctx, "Chain").expect("Chain's below derives");
    assert_axiom_free(&ctx, "Chain_below", &d.1);
    register_checked(&mut ctx, "Chain_below", d);

    let m = lam("_", g("Chain"), nat());
    let below = |t: Term| apps(g("Chain_below"), &[m.clone(), t]);
    let link = |n: Term, t: Term| apps(g("clink"), &[n, t]);

    // No recursive field ⇒ the EMPTY package, which must be `Unit` and not something that
    // silently has a slot in it.
    assert_eq!(normalize(&ctx, &below(g("cstop"))), g("Unit"));

    // One recursive field ⇒ one entry, and the entry is `(motive xⱼ, below xⱼ)` — exactly the
    // type of the recursive result, which is what makes `brecOn`'s payload free.
    assert_eq!(
        normalize(&ctx, &below(link(peano(0), g("cstop")))),
        normalize(&ctx, &prod(prod(nat(), g("Unit")), g("Unit"))),
        "one recursive field ⇒ Prod (Prod (motive x) (below x)) Unit"
    );
    // Two levels down, the package genuinely NESTS — `below` of the sub-chain appears inside.
    assert_eq!(
        normalize(&ctx, &below(link(peano(0), link(peano(1), g("cstop"))))),
        normalize(
            &ctx,
            &prod(
                prod(nat(), prod(prod(nat(), g("Unit")), g("Unit"))),
                g("Unit")
            )
        ),
        "the package reaches every subterm, not just the immediate one"
    );

    // THE NEGATIVE: a mutual block's below would have to be keyed by member; refuse it.
    let e = derive_below(&std_ctx(), "Nat").ok();
    assert!(e.is_some(), "a plain inductive must still derive");
}

#[test]
fn brec_on_computes_a_course_of_values_definition() {
    // The payoff, and the case T04-016 is about: `halve (n+2) = halve n + 1` reaches TWO
    // constructors down, which the plain recursor cannot do and `brecOn` can.
    let mut ctx = std_ctx();
    let (bt, bb) = derive_below(&ctx, "Nat").expect("Nat's below derives");
    register_checked(&mut ctx, "Nat_below", (bt, bb));
    let d = derive_brec_on(&ctx, "Nat").expect("Nat's brecOn derives");
    assert_axiom_free(&ctx, "Nat_brecOn", &d.1);
    register_checked(&mut ctx, "Nat_brecOn", d);

    let m = lam("_", nat(), nat());
    let below = |t: Term| apps(g("Nat_below"), &[m.clone(), t]);
    let fst = |a: Term, b: Term, p: Term| apps(g("Prod_fst"), &[a, b, p]);
    let snd = |a: Term, b: Term, p: Term| apps(g("Prod_snd"), &[a, b, p]);

    // F : Π(t:Nat). Nat_below (λ_.Nat) t → Nat, matching twice and reading the SECOND
    // predecessor's answer out of the package.
    //   at `Succ (Succ k)` the package is
    //     Prod (Prod Nat (Prod (Prod Nat (below k)) Unit)) Unit
    //   so `halve k` is fst (fst (snd (fst b))).
    let inner = Term::Match {
        discriminant: Box::new(v("n")),
        motive: Box::new(lam("n✧", nat(), arrow(below(succ(v("n✧"))), nat()))),
        cases: vec![
            // n = Zero  ⇒  halve 1 = 0
            lam("_", below(succ(g("Zero"))), g("Zero")),
            // n = Succ k  ⇒  halve (k+2) = Succ (halve k)
            lam(
                "k",
                nat(),
                lam(
                    "b",
                    below(succ(succ(v("k")))),
                    succ(fst(
                        nat(),
                        below(v("k")),
                        fst(
                            prod(nat(), below(v("k"))),
                            g("Unit"),
                            snd(
                                nat(),
                                below(succ(v("k"))),
                                fst(prod(nat(), below(succ(v("k")))), g("Unit"), v("b")),
                            ),
                        ),
                    )),
                ),
            ),
        ],
    };
    let step = lam(
        "t",
        nat(),
        lam(
            "bt",
            below(v("t")),
            app(
                Term::Match {
                    discriminant: Box::new(v("t")),
                    motive: Box::new(lam("t✧", nat(), arrow(below(v("t✧")), nat()))),
                    cases: vec![
                        lam("_", below(g("Zero")), g("Zero")),
                        lam("n", nat(), inner.clone()),
                    ],
                },
                v("bt"),
            ),
        ),
    );
    let halve = lam(
        "x",
        nat(),
        apps(g("Nat_brecOn"), &[m.clone(), v("x"), step]),
    );
    let halve_ty = infer_type(&ctx, &halve)
        .unwrap_or_else(|e| panic!("a course-of-values `halve` must type-check: {e:?}"));
    assert_eq!(normalize(&ctx, &halve_ty), pi("x", nat(), nat()));
    assert!(matches!(double_check(&ctx, &halve), DoubleCheck::Agreed));
    ctx.add_definition("halve".to_string(), arrow(nat(), nat()), halve);

    for (n, want) in [(0u32, 0u32), (1, 0), (2, 1), (3, 1), (4, 2), (5, 2), (7, 3)] {
        assert_eq!(
            normalize(&ctx, &app(g("halve"), peano(n))),
            peano(want),
            "halve {n} = {want}"
        );
    }
}

// =======================================================================================
// decidable equality
// =======================================================================================

#[test]
fn dec_eq_decides_an_enumeration_in_both_directions() {
    let mut ctx = std_ctx();
    register_hue(&mut ctx);
    let mut env = ctx.clone();
    let d = derive_no_confusion_type(&env, "Hue").unwrap();
    env.add_definition("Hue_noConfusionType".to_string(), d.0, d.1);
    let d = derive_no_confusion(&env, "Hue").unwrap();
    env.add_definition("Hue_noConfusion".to_string(), d.0, d.1);
    let d = derive_dec_eq(&env, "Hue").expect("Hue's decidable equality derives");
    assert_axiom_free(&env, "decEqHue", &d.1);
    register_checked(&mut env, "decEqHue", d);

    let decide_at = |a: &str, b: &str| {
        apps(
            g("decide"),
            &[
                eq3(g("Hue"), g(a), g(b)),
                apps(g("decEqHue"), &[g(a), g(b)]),
            ],
        )
    };
    for c in ["hred", "hgreen", "hblue"] {
        assert_eq!(normalize(&env, &decide_at(c, c)), g("true"), "{c} = {c}");
    }
    // Every OFF-diagonal pair, both ways: a derivation that only got the diagonal right, or
    // only the adjacent pair, is caught here.
    for (a, b) in [
        ("hred", "hgreen"),
        ("hgreen", "hred"),
        ("hred", "hblue"),
        ("hblue", "hred"),
        ("hgreen", "hblue"),
        ("hblue", "hgreen"),
    ] {
        assert_eq!(normalize(&env, &decide_at(a, b)), g("false"), "{a} ≠ {b}");
    }
}

#[test]
fn dec_eq_recurses_through_a_constructors_fields() {
    let mut ctx = std_ctx();
    register_chain(&mut ctx);
    let auxs = derive_auxiliaries(&ctx, "Chain");
    for a in auxs {
        ctx.add_definition(a.name, a.ty, a.term);
    }
    assert!(
        ctx.get_global("decEqChain").is_some(),
        "Chain's decidable equality must derive"
    );

    let link = |n: Term, t: Term| apps(g("clink"), &[n, t]);
    let decide_at = |a: Term, b: Term| {
        apps(
            g("decide"),
            &[
                eq3(g("Chain"), a.clone(), b.clone()),
                apps(g("decEqChain"), &[a, b]),
            ],
        )
    };
    // agreement, disagreement in the FIRST field, disagreement in the SECOND, and a
    // constructor mismatch — each is a different branch of the derivation.
    let cases: Vec<(Term, Term, &str)> = vec![
        (g("cstop"), g("cstop"), "true"),
        (g("cstop"), link(peano(0), g("cstop")), "false"),
        (link(peano(0), g("cstop")), g("cstop"), "false"),
        (
            link(peano(1), g("cstop")),
            link(peano(1), g("cstop")),
            "true",
        ),
        (
            link(peano(1), g("cstop")),
            link(peano(2), g("cstop")),
            "false",
        ),
        (
            link(peano(1), link(peano(3), g("cstop"))),
            link(peano(1), link(peano(3), g("cstop"))),
            "true",
        ),
        (
            link(peano(1), link(peano(3), g("cstop"))),
            link(peano(1), link(peano(4), g("cstop"))),
            "false",
        ),
    ];
    for (a, b, want) in cases {
        assert_eq!(
            normalize(&ctx, &decide_at(a.clone(), b.clone())),
            g(want),
            "{a} vs {b}"
        );
    }
}

#[test]
fn dec_eq_refuses_a_field_it_cannot_actually_decide() {
    // THE NEGATIVE, and the important one: `decEqNat` reused for another carrier would compare
    // one field and ignore the constructor tag, and `decide`'s `Bool` result would hide it. A
    // decision procedure we cannot exhibit must be refused, by name.
    let mut ctx = std_ctx();
    ctx.add_inductive("Tagged", ty(0));
    ctx.add_constructor_checked("tagged", "Tagged", arrow(g("Float"), g("Tagged")))
        .expect("tagged registers");
    let mut env = ctx.clone();
    let d = derive_no_confusion_type(&env, "Tagged").unwrap();
    env.add_definition("Tagged_noConfusionType".to_string(), d.0, d.1);
    let d = derive_no_confusion(&env, "Tagged").unwrap();
    env.add_definition("Tagged_noConfusion".to_string(), d.0, d.1);
    let e = derive_dec_eq(&env, "Tagged").expect_err("Float has no decidable equality here");
    let msg = format!("{e:?}");
    assert!(
        msg.contains("Float") && msg.contains("decEqFloat"),
        "the refusal must name it: {msg}"
    );

    // …and the umbrella must then simply OMIT it, never emit a broken one.
    let names: Vec<String> = derive_auxiliaries(&ctx, "Tagged")
        .into_iter()
        .map(|a| a.name)
        .collect();
    assert!(
        !names.iter().any(|n| n == "decEqTagged"),
        "an undecidable carrier must not get an instance anyway: {names:?}"
    );
    assert!(
        names.iter().any(|n| n == "Tagged_noConfusion"),
        "the derivable ones still come: {names:?}"
    );
}

// =======================================================================================
// the umbrella
// =======================================================================================

#[test]
fn the_auxiliary_group_registers_in_dependency_order() {
    // The caller's contract: register the list in order and every one of them type-checks
    // against exactly what came before. If the order were wrong, `noConfusion` would cite an
    // unbound `noConfusionType`.
    let mut ctx = std_ctx();
    register_chain(&mut ctx);
    let auxs = derive_auxiliaries(&ctx, "Chain");
    let names: Vec<String> = auxs.iter().map(|a| a.name.clone()).collect();
    assert_eq!(
        names,
        vec![
            "Chain_rec",
            "Chain_recOn",
            "Chain_casesOn",
            "Chain_noConfusionType",
            "Chain_noConfusion",
            "clink_inj",
            "Chain_below",
            "Chain_brecOn",
            "decEqChain",
        ],
        "the whole family Lean generates, in dependency order"
    );
    for a in auxs {
        let got = infer_type(&ctx, &a.term)
            .unwrap_or_else(|e| panic!("`{}` must check against what precedes it: {e:?}", a.name));
        assert_eq!(
            got, a.ty,
            "`{}`'s reported type must be the one inferred",
            a.name
        );
        match double_check(&ctx, &a.term) {
            DoubleCheck::Agreed => {}
            other => panic!("both kernels must certify `{}`, got {other:?}", a.name),
        }
        assert_axiom_free(&ctx, &a.name, &a.term);
        ctx.add_definition(a.name, a.ty, a.term);
    }

    // And the umbrella never breaks on a shape it cannot reach — it returns what it could.
    let mut p = std_ctx();
    p.add_inductive("Duo", prop());
    p.add_constructor("da", "Duo", g("Duo"));
    p.add_constructor("db", "Duo", g("Duo"));
    let names: Vec<String> = derive_auxiliaries(&p, "Duo")
        .into_iter()
        .map(|a| a.name)
        .collect();
    assert_eq!(
        names,
        vec!["Duo_rec", "Duo_recOn", "Duo_casesOn"],
        "a multi-constructor Prop keeps its induction principle and its two Lean reshapings \
         (`Or.recOn`/`Or.casesOn` exist in Lean) and NOTHING else: disjointness is a category \
         error under proof irrelevance, and a course-of-values package built from the `Type 0` \
         `Prod`/`Unit` would be a large elimination it may not perform"
    );
}

#[test]
fn the_declaration_site_recipe_yields_every_constant_lean_names() {
    // What a DECLARATION SITE has to do, executed here so the recipe is checked rather than
    // described. `derive_*` never registers — that contract is `derive_recursor`'s and the whole
    // family keeps it — so the three lines below are what `Context`'s registration entry points
    // and `Repl::execute`'s `Inductive` arm owe, and this test is their specification.
    let mut ctx = std_ctx();
    register_chain(&mut ctx);
    register_hue(&mut ctx);

    for ind in ["Chain", "Hue"] {
        for aux in derive_auxiliaries(&ctx, ind) {
            // …and a decidable equality is also an INSTANCE, or nothing can resolve it.
            if aux.name == format!("decEq{ind}") {
                ctx.add_instance(aux.ty.clone(), Term::Global(aux.name.clone()));
            }
            ctx.add_definition(aux.name, aux.ty, aux.term);
        }
    }

    // T03-018's shape: `injection`/`cases h`/`simp [injEq]` all name a constant, and after the
    // recipe the constant is there — as a kernel-checked DEFINITION, not an axiom.
    for n in ["Chain_noConfusion", "Chain_noConfusionType", "clink_inj"] {
        assert!(
            ctx.get_global(n).is_some(),
            "`{n}` must come with the declaration"
        );
        assert!(
            ctx.is_definition(n),
            "`{n}` must be a definition, never an assumption"
        );
    }

    // T04-016's shape: course-of-values recursion has its package and its eliminator.
    for n in ["Chain_below", "Chain_brecOn"] {
        assert!(
            ctx.get_global(n).is_some(),
            "`{n}` must come with the declaration"
        );
        assert!(ctx.is_definition(n));
    }

    // T03-023's shape: a `Decidable (Eq Hue a b)` goal RESOLVES, and the instance it finds
    // actually decides — separating two different constructors, which is the half a wrong
    // instance (one that compares a field and ignores the tag) would get wrong.
    let mut mctx = MetaCtx::new();
    let required = app(g("Decidable"), eq3(g("Hue"), g("hred"), g("hgreen")));
    let inst = resolve_instance(&ctx, &mut mctx, &required)
        .expect("a declared inductive must yield a findable `Decidable (Eq I a b)` instance");
    assert_eq!(
        normalize(
            &ctx,
            &apps(g("decide"), &[eq3(g("Hue"), g("hred"), g("hgreen")), inst])
        ),
        g("false"),
        "the resolved instance must separate DIFFERENT constructors"
    );
    let mut mctx = MetaCtx::new();
    let same = resolve_instance(
        &ctx,
        &mut mctx,
        &app(g("Decidable"), eq3(g("Hue"), g("hred"), g("hred"))),
    )
    .expect("…and resolve on the diagonal too");
    assert_eq!(
        normalize(
            &ctx,
            &apps(g("decide"), &[eq3(g("Hue"), g("hred"), g("hred")), same])
        ),
        g("true"),
        "…and identify equal ones"
    );
}

#[test]
fn the_auxiliary_group_never_displaces_a_constant_that_already_exists() {
    // Registration OVERWRITES. So the umbrella must return what a declaration should ADD, never
    // a replacement for a name that is taken: running it over `Nat` must not hand back a fresh
    // `decEqNat` to be swapped in for the prelude's hand-written, separately-tested one. Same
    // intent, different term, and nothing would have reported the swap.
    let ctx = std_ctx();
    assert!(
        ctx.is_definition("decEqNat"),
        "the prelude's hand-written instance is the premise"
    );
    let before = ctx
        .get_definition_body("decEqNat")
        .cloned()
        .expect("it has a body");

    let names: Vec<String> = derive_auxiliaries(&ctx, "Nat")
        .into_iter()
        .map(|a| a.name)
        .collect();
    assert!(
        !names.contains(&"decEqNat".to_string()),
        "`decEqNat` already exists and must be left alone: {names:?}"
    );
    // `Nat_rec` is the same kind of constant: the prelude PUBLISHES it (the integer laws are
    // proved by `Nat` induction), so it is left alone too.
    assert!(
        ctx.is_definition("Nat_rec"),
        "the prelude's published `Nat_rec` is the premise"
    );
    let rec_before = ctx
        .get_definition_body("Nat_rec")
        .cloned()
        .expect("it has a body");
    assert!(
        !names.contains(&"Nat_rec".to_string()),
        "`Nat_rec` already exists and must be left alone: {names:?}"
    );
    // …while everything genuinely MISSING for `Nat` is still offered, `Succ`'s injectivity
    // included — that is what `injection` on `Succ n = Succ m` needs.
    for want in [
        "Nat_recOn",
        "Nat_casesOn",
        "Nat_noConfusionType",
        "Nat_noConfusion",
        "Succ_inj",
        "Nat_below",
        "Nat_brecOn",
    ] {
        assert!(
            names.contains(&want.to_string()),
            "`{want}` is missing from {names:?}"
        );
    }
    // `Zero` has no fields, so it contributes no injectivity lemma.
    assert!(
        !names.contains(&"Zero_inj".to_string()),
        "a nullary constructor has none: {names:?}"
    );

    // And registering the whole group leaves the existing definition byte-identical.
    let mut c = ctx.clone();
    for a in derive_auxiliaries(&ctx, "Nat") {
        c.add_definition(a.name, a.ty, a.term);
    }
    assert_eq!(
        c.get_definition_body("decEqNat"),
        Some(&before),
        "the prelude's `decEqNat` must survive the group registration untouched"
    );
    assert_eq!(
        c.get_definition_body("Nat_rec"),
        Some(&rec_before),
        "…and so must the prelude's `Nat_rec`"
    );
}

// ---------------------------------------------------------------------------------------
// a universe-polymorphic family's auxiliaries
// ---------------------------------------------------------------------------------------

/// `Big.{u} : Sort (u+1) := b1 | b2` — never a proposition, so its course-of-values pair is
/// derived. Lean states the family's auxiliaries over the family's own telescope (`Big.below.{u}`),
/// so they serve every instance of it. Derived as if `Big` were monomorphic, an auxiliary names
/// `Big` bare at a level no use site can instantiate, and serves none of them.
#[test]
fn a_polymorphic_familys_auxiliaries_are_polymorphic_in_its_levels() {
    let mut ctx = std_ctx();
    ctx.set_auto_auxiliaries(true);
    let at = |name: &str, l: Universe| Term::Const {
        name: name.to_string(),
        levels: vec![l],
    };
    let u = || Universe::Var("u".to_string());
    ctx.add_universe_poly_inductives(
        &["u".to_string()],
        &[MutualInductive {
            name: "Big".to_string(),
            sort: Term::Sort(u().succ()),
            num_params: 0,
            constructors: vec![
                ("b1".to_string(), at("Big", u())),
                ("b2".to_string(), at("Big", u())),
            ],
        }],
    )
    .expect("`Big.{u}` registers");

    assert!(
        ctx.get_universe_poly("Big_below").is_some(),
        "`Big` is never a proposition: its pair is derived"
    );
    let family = ["Big", "b1", "b2", "Big_rec", "Big_below", "Big_brecOn"];
    for name in [
        "Big_rec",
        "Big_recOn",
        "Big_casesOn",
        "Big_noConfusionType",
        "Big_noConfusion",
        "b1_inj",
        "b2_inj",
        "Big_below",
        "Big_brecOn",
        "decEqBig",
    ] {
        if ctx.get_global(name).is_none() && ctx.get_universe_poly(name).is_none() {
            continue;
        }
        let (params, ty, body) = ctx
            .get_universe_poly(name)
            .unwrap_or_else(|| panic!("`{name}` is stated over `Big`'s telescope"));
        assert_eq!(
            params,
            &vec!["u".to_string()],
            "`{name}` is polymorphic in `Big`'s own level"
        );
        let mut cited = Vec::new();
        cited_globals(ty, &mut cited);
        cited_globals(body, &mut cited);
        let bare: Vec<&String> = cited
            .iter()
            .filter(|n| family.contains(&n.as_str()))
            .collect();
        assert!(
            bare.is_empty(),
            "`{name}` names the family with its levels written out, never bare: {bare:?}"
        );
    }

    // It serves every instance: `Big_below.{l} (λ_ : Big.{l}. Nat) b1.{l} : Type 0` at two levels.
    for l in [Universe::Prop, Universe::Type(0)] {
        let applied = apps(
            at("Big_below", l.clone()),
            &[lam("_", at("Big", l.clone()), nat()), at("b1", l.clone())],
        );
        let t = match infer_type(&ctx, &applied) {
            Ok(t) => t,
            Err(e) => panic!("`Big_below` at {l:?}: {e:?}"),
        };
        assert_eq!(
            normalize(&ctx, &t),
            ty(0),
            "`Big_below.{{{l:?}}}` is a family into `Type 0`"
        );
        match double_check(&ctx, &applied) {
            DoubleCheck::Agreed => {}
            other => panic!("both kernels type `Big_below` at {l:?}, got {other:?}"),
        }
    }

    // A monomorphic family's pair stays monomorphic.
    register_chain(&mut ctx);
    assert!(
        ctx.get_global("Chain_below").is_some(),
        "`Chain`'s pair is derived"
    );
    assert!(
        ctx.get_universe_poly("Chain_below").is_none(),
        "`Chain` has no telescope, so neither has its pair"
    );
}
