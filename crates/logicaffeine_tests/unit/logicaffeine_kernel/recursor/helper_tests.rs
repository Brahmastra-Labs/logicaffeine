//! Lib-unit coverage for the recursor-derivation helpers (`app_args`/`head_global`/`peel_pis`/
//! `subst_name`) and the `derive_recursor` entry — the eliminator machinery is soundness-
//! critical (a wrong recursion principle unsound), and these pure helpers had no unit coverage.
use super::*;
use crate::term::{Term, Universe};

fn g(s: &str) -> Term {
    Term::Global(s.to_string())
}
fn sort0() -> Term {
    Term::Sort(Universe::Type(0))
}

#[test]
fn app_args_collects_the_spine_in_order() {
    let spine = app(app(g("List"), g("A")), g("B"));
    assert_eq!(app_args(&spine), vec![g("A"), g("B")]);
    assert_eq!(
        app_args(&g("Nat")),
        Vec::<Term>::new(),
        "no args on a bare head"
    );
}

#[test]
fn head_global_finds_the_spine_head() {
    assert_eq!(
        head_global(&app(app(g("List"), g("A")), g("B"))),
        Some("List")
    );
    assert_eq!(head_global(&g("Nat")), Some("Nat"));
    assert_eq!(
        head_global(&var("x")),
        None,
        "a variable head is not a global"
    );
}

#[test]
fn peel_pis_splits_binders_from_the_residual() {
    let t = pi("x", sort0(), pi("y", sort0(), g("Nat")));
    let (params, res) = peel_pis(&t);
    assert_eq!(params.len(), 2);
    assert_eq!(params[0].0, "x");
    assert_eq!(params[1].0, "y");
    assert_eq!(*res, g("Nat"));
}

#[test]
fn subst_name_replaces_free_but_respects_shadowing() {
    assert_eq!(subst_name(&var("x"), "x", &g("R")), g("R"));
    // x shadowed by a Π binder of the same name → body untouched.
    let shadowed = pi("x", sort0(), var("x"));
    assert_eq!(subst_name(&shadowed, "x", &g("R")), shadowed);
    // x under a DIFFERENT binder → replaced in the body.
    assert_eq!(
        subst_name(&pi("y", sort0(), var("x")), "x", &g("R")),
        pi("y", sort0(), g("R")),
    );
}

#[test]
fn subst_name_respects_shadowing_in_lambda_fix_mutualfix_and_let() {
    // Every binder that rebinds `old` must leave its body untouched (shadowing), and every
    // binder that does NOT must substitute into its body. Pins the `binder == old` guard in
    // each of Lambda / Fix / MutualFix / Let (Pi + Var are covered above).
    let r = g("R");
    let fix = |name: &str, body: Term| Term::Fix {
        name: name.to_string(),
        body: Box::new(body),
    };
    let lett = |name: &str, body: Term| Term::Let {
        name: name.to_string(),
        ty: Box::new(sort0()),
        value: Box::new(g("V")),
        body: Box::new(body),
    };
    let mfix = |first: &str| Term::MutualFix {
        defs: vec![(first.to_string(), var("x")), ("h".to_string(), var("x"))],
        index: 0,
    };

    // Lambda
    assert_eq!(
        subst_name(&lam("x", sort0(), var("x")), "x", &r),
        lam("x", sort0(), var("x"))
    );
    assert_eq!(
        subst_name(&lam("y", sort0(), var("x")), "x", &r),
        lam("y", sort0(), g("R"))
    );
    // Fix
    assert_eq!(subst_name(&fix("x", var("x")), "x", &r), fix("x", var("x")));
    assert_eq!(subst_name(&fix("f", var("x")), "x", &r), fix("f", g("R")));
    // Let — the body rebinds, the value does not (a non-recursive let's value can't see its name)
    assert_eq!(
        subst_name(&lett("x", var("x")), "x", &r),
        lett("x", var("x"))
    );
    assert_eq!(subst_name(&lett("z", var("x")), "x", &r), lett("z", g("R")));
    // MutualFix — a def named `x` shadows the WHOLE block; otherwise every body substitutes.
    assert_eq!(subst_name(&mfix("x"), "x", &r), mfix("x"));
    assert_eq!(
        subst_name(&mfix("f"), "x", &r),
        Term::MutualFix {
            defs: vec![("f".to_string(), g("R")), ("h".to_string(), g("R"))],
            index: 0,
        },
    );
}

#[test]
fn derive_recursor_succeeds_for_a_registered_inductive_and_fails_otherwise() {
    use crate::prelude::StandardLibrary;
    let mut c = Context::new();
    StandardLibrary::register(&mut c);
    let (rec_ty, _impl) = derive_recursor(&c, "Nat").expect("Nat's recursor derives");
    assert!(
        matches!(rec_ty, Term::Pi { .. }),
        "a recursor's type is a Π-telescope"
    );
    assert!(derive_recursor(&c, "definitely_not_an_inductive").is_err());
}

#[test]
fn subsingleton_props_admit_large_elimination_but_multi_ctor_props_do_not() {
    // The large-elimination guard is SOUNDNESS-critical: a subsingleton `Prop` (0 constructors,
    // or 1 whose data is all propositional) may eliminate into `Type` (J for `Eq`, ex falso for
    // `False`), but a multi-constructor `Prop` must keep a `Prop`-restricted motive — else one
    // could extract `Type`-level data from a proof and break consistency. Pin the motive's
    // target sort for each shape (with 0 parameters the motive `C` is the first Π binder, and
    // its type is `Π indices. Π major. <target>`).
    fn motive_target(rec_ty: &Term) -> Term {
        let (binders, _res) = peel_pis(rec_ty);
        let motive_ty = &binders[0].1; // C, the motive (no parameters precede it here)
        let (_idx_and_major, target) = peel_pis(motive_ty);
        target.clone()
    }

    // Subsingleton: a 0-constructor `Prop` (the shape of `False`).
    let mut c = Context::new();
    c.add_inductive("Empty", Term::Sort(Universe::Prop));
    let (empty_rec, _) = derive_recursor(&c, "Empty").expect("Empty's recursor derives");
    assert_eq!(
        motive_target(&empty_rec),
        Term::Sort(Universe::Type(0)),
        "a 0-ctor Prop is a subsingleton ⇒ its motive eliminates into Type",
    );

    // Non-subsingleton: a 2-constructor `Prop` keeps its Prop induction principle.
    let mut c2 = Context::new();
    c2.add_inductive("Bit", Term::Sort(Universe::Prop));
    c2.add_constructor("B0", "Bit", g("Bit"));
    c2.add_constructor("B1", "Bit", g("Bit"));
    let (bit_rec, _) = derive_recursor(&c2, "Bit").expect("Bit's recursor derives");
    assert_eq!(
        motive_target(&bit_rec),
        Term::Sort(Universe::Prop),
        "a 2-ctor Prop is NOT a subsingleton ⇒ its motive stays in Prop",
    );
}

/// Each member's motive is the binder `Π indices. Π(major : <Ind> …). <target sort>` — the
/// only binders whose peeled residual is a `Sort`. Map each member's inductive head to its
/// motive's target.
fn mutual_motive_targets(
    rec_ty: &Term,
    members: &[&str],
) -> std::collections::HashMap<String, Term> {
    let (binders, _res) = peel_pis(rec_ty);
    let mut targets = std::collections::HashMap::new();
    for (_n, bty) in &binders {
        let (inner, residual) = peel_pis(bty);
        if matches!(residual, Term::Sort(_)) {
            if let Some((_, major_ty)) = inner.last() {
                if let Some(head) = head_global(major_ty) {
                    if members.contains(&head) {
                        targets.insert(head.to_string(), residual.clone());
                    }
                }
            }
        }
    }
    targets
}

#[test]
fn mutual_recursor_gives_large_elimination_to_the_subsingleton_member_only() {
    // The SAME subsingleton guard inside derive_mutual_recursor, pinned from BOTH sides — a
    // mutant that always restricts, never restricts, or drops the negation gets at least one
    // member's motive sort wrong.
    //
    // The criterion is Lean's, and it is stricter than "the datum's type is a Prop": a
    // constructor argument is admissible iff it is propositional OR it reappears VERBATIM
    // among the result's indices, so that eliminating can recover it. Lean's kernel searches
    // the result's argument LIST for the binder itself (`elim_only_at_universe_zero`), it
    // does not ask whether the binder merely occurs somewhere inside an index.
    use crate::prelude::StandardLibrary;
    use crate::MutualInductive;
    let prop = || Term::Sort(Universe::Prop);
    let nat = || g("Nat");
    let succ = |n: Term| app(g("Succ"), n);

    // BLOCK 1 — Even/Odd. `even_succ` gives Even two constructors, so Even is not a
    // subsingleton. `odd_succ : Π(n:Nat). Even n → Odd (Succ n)` has TWO fields and `n : Nat`
    // is DATA: the result's index is `Succ n`, not `n`, so `n` is not recoverable and Lean
    // refuses the large eliminator here too. Both members stay `Prop`.
    let block = vec![
        MutualInductive {
            name: "Even".to_string(),
            sort: pi("n", nat(), prop()),
            num_params: 0,
            constructors: vec![
                ("even_zero".to_string(), app(g("Even"), g("Zero"))),
                (
                    "even_succ".to_string(),
                    pi(
                        "n",
                        nat(),
                        arrow(app(g("Odd"), var("n")), app(g("Even"), succ(var("n")))),
                    ),
                ),
            ],
        },
        MutualInductive {
            name: "Odd".to_string(),
            sort: pi("n", nat(), prop()),
            num_params: 0,
            constructors: vec![(
                "odd_succ".to_string(),
                pi(
                    "n",
                    nat(),
                    arrow(app(g("Even"), var("n")), app(g("Odd"), succ(var("n")))),
                ),
            )],
        },
    ];
    let mut c = Context::new();
    StandardLibrary::register(&mut c);
    c.add_mutual_inductives(&block)
        .expect("Even/Odd mutual block registers");
    let (rec_ty, _) = derive_recursor(&c, "Even").expect("the mutual recursor derives");
    let targets = mutual_motive_targets(&rec_ty, &["Even", "Odd"]);
    assert_eq!(
        targets.get("Even"),
        Some(&Term::Sort(Universe::Prop)),
        "Even (2 constructors) is not a subsingleton ⇒ Prop-restricted motive",
    );
    assert_eq!(
        targets.get("Odd"),
        Some(&Term::Sort(Universe::Prop)),
        "Odd's `odd_succ` binds `n : Nat`, which is DATA, and its result index is `Succ n` — \
             `n` is not among the indices, so it cannot be recovered and Odd is NOT a \
             subsingleton. Lean refuses large elimination here; so must we.",
    );

    // BLOCK 2 — the other side of the guard, which Even/Odd no longer provides. `r1`'s only
    // datum `n : Nat` DOES reappear verbatim as the result index of `Rung1 n`, so it is
    // recoverable and the member is a genuine subsingleton ⇒ large elimination into `Type 0`.
    // One derivation, both verdicts: this is what pins the criterion rather than a constant.
    let rungs = vec![
        MutualInductive {
            name: "Rung0".to_string(),
            sort: pi("n", nat(), prop()),
            num_params: 0,
            constructors: vec![
                ("r0_base".to_string(), app(g("Rung0"), g("Zero"))),
                (
                    "r0_step".to_string(),
                    pi(
                        "n",
                        nat(),
                        arrow(app(g("Rung1"), var("n")), app(g("Rung0"), succ(var("n")))),
                    ),
                ),
            ],
        },
        MutualInductive {
            name: "Rung1".to_string(),
            sort: pi("n", nat(), prop()),
            num_params: 0,
            constructors: vec![(
                "r1".to_string(),
                pi(
                    "n",
                    nat(),
                    arrow(app(g("Rung0"), var("n")), app(g("Rung1"), var("n"))),
                ),
            )],
        },
    ];
    let mut d = Context::new();
    StandardLibrary::register(&mut d);
    d.add_mutual_inductives(&rungs)
        .expect("Rung0/Rung1 mutual block registers");
    let (rung_ty, _) = derive_recursor(&d, "Rung0").expect("the mutual recursor derives");
    let targets = mutual_motive_targets(&rung_ty, &["Rung0", "Rung1"]);
    assert_eq!(
        targets.get("Rung0"),
        Some(&Term::Sort(Universe::Prop)),
        "Rung0 (2 constructors) is not a subsingleton ⇒ Prop-restricted motive",
    );
    assert_eq!(
        targets.get("Rung1"),
        Some(&Term::Sort(Universe::Type(0))),
        "Rung1's sole constructor binds `n : Nat` and returns `Rung1 n`, so `n` IS among the \
             indices and is recoverable ⇒ subsingleton ⇒ large elimination into Type",
    );
}
