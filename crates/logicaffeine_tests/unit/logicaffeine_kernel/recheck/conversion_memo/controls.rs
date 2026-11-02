//! The independent kernel's positive conversion evidence: a proved conversion answers the same pair
//! again in the same context and environment, and nowhere else.
use super::*;

fn g(n: &str) -> Db {
    Db::Global(n.into())
}
fn standard() -> Context {
    let mut c = Context::new();
    crate::prelude::StandardLibrary::register(&mut c);
    c
}
fn hits() -> usize {
    HITS.with(|n| n.get())
}

/// `P : Prop, p : P, q : P` against `x y z : Nat`: `q ≡ p` holds by proof irrelevance in the first
/// context and not in the second, so evidence proved in one is never an answer in the other.
#[test]
fn evidence_never_crosses_contexts_that_differ_in_a_binder_type() {
    let c = standard();
    let proofs = vec![Db::Sort(Universe::Prop), Db::Var(0), Db::Var(1)];
    let naturals = vec![g("Nat"), g("Nat"), g("Nat")];
    let _scope = Scope::enter(&c);
    assert!(
        def_eq(&c, &proofs, &Db::Var(0), &Db::Var(1)),
        "two proofs of one proposition"
    );
    let before = hits();
    assert!(def_eq(&c, &proofs, &Db::Var(0), &Db::Var(1)));
    assert!(
        def_eq(&c, &proofs, &Db::Var(1), &Db::Var(0)),
        "evidence is symmetric"
    );
    assert_eq!(
        hits(),
        before + 2,
        "the same pair in the same context is answered from the evidence"
    );
    assert!(
        !def_eq(&c, &naturals, &Db::Var(0), &Db::Var(1)),
        "two naturals are never equal by irrelevance"
    );
    let mut extended = extend_local(&naturals);
    extended.push(g("Nat"));
    assert!(!def_eq(&c, &extended, &Db::Var(1), &Db::Var(2)));
    assert_eq!(hits(), before + 2, "no other context reads it");
}

/// A borrowed extension numbers its context once per session and again after a push; the same
/// binder types reached through a different chain of extensions get the same number.
#[test]
fn contexts_are_numbered_by_their_binder_types_however_they_are_built() {
    let c = standard();
    let _scope = Scope::enter(&c);
    let flat = vec![Db::Sort(Universe::Prop), Db::Var(0), Db::Var(1)];
    let base = vec![Db::Sort(Universe::Prop)];
    let mut chained = extend_local(&base);
    chained.push(Db::Var(0));
    assert!(def_eq(&c, &flat, &Db::Var(0), &Db::Var(1)));
    let before = hits();
    chained.push(Db::Var(1));
    assert!(
        def_eq(&c, &chained, &Db::Var(0), &Db::Var(1)),
        "the same types through another chain"
    );
    assert_eq!(hits(), before + 1);
}

/// `k := Zero` in the checked environment and `k := Succ Zero` in another: the evidence for
/// `k ≡ Zero` belongs to the first and is never consulted for the second.
#[test]
fn evidence_is_never_consulted_for_another_environment() {
    let mut c = standard();
    c.declare("k", Term::Global("Nat".into()), Term::Global("Zero".into()))
        .unwrap();
    let mut other = c.clone();
    other.permit_meaning_changes_for_cache_tests();
    other
        .declare(
            "k",
            Term::Global("Nat".into()),
            Term::App(
                Box::new(Term::Global("Succ".into())),
                Box::new(Term::Global("Zero".into())),
            ),
        )
        .unwrap();
    let _scope = Scope::enter(&c);
    assert!(def_eq(&c, &[], &g("k"), &g("Zero")));
    assert!(def_eq(&c, &[], &g("k"), &g("Zero")));
    let before = hits();
    assert!(
        !def_eq(&other, &[], &g("k"), &g("Zero")),
        "k is Succ Zero there"
    );
    assert!(def_eq(
        &c.order_recurrence_context(),
        &[],
        &g("k"),
        &g("Zero")
    ));
    assert_eq!(
        hits(),
        before,
        "neither another environment nor a recurrence probe's reads the evidence"
    );
}

/// Every check scope opens its own evidence, and nothing outlives the check that proved it: the
/// same context, changed between two checks, is never answered from the first.
#[test]
fn evidence_does_not_outlive_its_check() {
    let mut c = standard();
    c.declare("k", Term::Global("Nat".into()), Term::Global("Zero".into()))
        .unwrap();
    {
        let _scope = Scope::enter(&c);
        assert!(def_eq(&c, &[], &g("k"), &g("Zero")));
        assert!(def_eq(&c, &[], &g("k"), &g("Zero")));
    }
    c.permit_meaning_changes_for_cache_tests();
    c.declare(
        "k",
        Term::Global("Nat".into()),
        Term::App(
            Box::new(Term::Global("Succ".into())),
            Box::new(Term::Global("Zero".into())),
        ),
    )
    .unwrap();
    let _scope = Scope::enter(&c);
    let before = hits();
    assert!(
        !def_eq(&c, &[], &g("k"), &g("Zero")),
        "k is Succ Zero in the later check"
    );
    assert_eq!(hits(), before);
}

/// `Tree` (`leaf_a`, `leaf_b`, `node : Tree → Tree`) and `grow x := node x`.
fn tree_context() -> Context {
    let mut c = standard();
    let tree = || Term::Global("Tree".into());
    c.add_inductive("Tree", Term::Sort(Universe::Type(0)));
    c.add_constructor("leaf_a", "Tree", tree());
    c.add_constructor("leaf_b", "Tree", tree());
    c.add_constructor(
        "node",
        "Tree",
        Term::Pi {
            param: "_".into(),
            param_type: Box::new(tree()),
            body_type: Box::new(tree()),
        },
    );
    c.declare(
        "grow",
        Term::Pi {
            param: "_".into(),
            param_type: Box::new(tree()),
            body_type: Box::new(tree()),
        },
        Term::Lambda {
            param: "x".into(),
            param_type: Box::new(tree()),
            body: Box::new(Term::App(
                Box::new(Term::Global("node".into())),
                Box::new(Term::Var("x".into())),
            )),
        },
    )
    .unwrap();
    c
}

/// `grow (grow … leaf)`, `depth` applications of `grow`.
fn tower(leaf: &str, depth: usize) -> Term {
    (0..depth).fold(Term::Global(leaf.into()), |t, _| {
        Term::App(Box::new(Term::Global("grow".into())), Box::new(t))
    })
}

/// Lean's `failed_before`: the arguments of two applications of one head, once refuted, are not
/// compared again in the same check — the refutation is read back from the evidence. Without it the
/// argument comparison and the unfolding after it would each re-descend the whole tower, twice per
/// level: the independent kernel refuses `grow³⁰ leaf_a = grow³⁰ leaf_b` within a bounded number of
/// comparisons, and asked a second time in one check, a tower's refutation costs one comparison per
/// level.
#[test]
fn refuted_arguments_are_never_compared_again_in_one_check() {
    let depth = 30;
    let c = tree_context();
    let (a, b) = (tower("leaf_a", depth), tower("leaf_b", depth));
    let eq = |x: Term, y: Term| {
        [Term::Global("Tree".into()), x, y]
            .into_iter()
            .fold(Term::Global("Eq".into()), |f, t| {
                Term::App(Box::new(f), Box::new(t))
            })
    };
    let refl = Term::App(
        Box::new(Term::App(
            Box::new(Term::Global("refl".into())),
            Box::new(Term::Global("Tree".into())),
        )),
        Box::new(a.clone()),
    );
    let claim = Term::Let {
        name: "claim".into(),
        ty: Box::new(eq(a, b)),
        value: Box::new(refl),
        body: Box::new(Term::Var("claim".into())),
    };
    let calls = || DEF_EQ_CALLS.with(|n| n.get());
    let before = calls();
    let verdict = crate::recheck(&c, &claim);
    let checked = calls() - before;
    assert!(
        matches!(verdict, Err(ReCheckError::Ill(_))),
        "the towers differ at their leaves: {verdict:?}"
    );
    assert!(
        checked < depth * depth * 8,
        "{checked} comparisons for towers {depth} deep"
    );

    // Within one check's evidence: the first decision records every level's refuted arguments, and
    // the same decision asked again reads each back — one comparison per level, never a descent.
    // Sixteen levels decide within every allowance; deeper, the first decision's top levels are cut
    // short by the probe's fuel and their refutations are rightly withheld.
    let depth = 16;
    let level = |k: usize| {
        let db = |t: &Term| to_db(t, &mut Vec::new()).expect("a closed tower lowers");
        (db(&tower("leaf_a", k)), db(&tower("leaf_b", k)))
    };
    let (a, b) = level(depth);
    let _scope = Scope::enter(&c);
    assert!(
        !arguments_refuted(&c, &[], &a, &b),
        "nothing is refuted before the first decision"
    );
    assert!(
        !def_eq(&c, &[], &a, &b),
        "the towers differ at their leaves"
    );
    for k in 1..=depth {
        let (x, y) = level(k);
        assert!(
            arguments_refuted(&c, &[], &x, &y),
            "level {k}: its refuted arguments are evidence"
        );
    }
    let before = calls();
    assert!(!def_eq(&c, &[], &a, &b), "the towers still differ");
    let again = calls() - before;
    assert!(again <= 2 * depth, "asked again, every level's refutation is read back: {again} comparisons for towers {depth} deep");
}
