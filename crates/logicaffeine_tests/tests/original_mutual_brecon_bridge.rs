//! Lean's generated `below`/`brecOn` for an ordinary mutual block, checked by BOTH kernels.
//!
//! The witness (`fixtures/lean_semantics/LF_MutualBrecOn.lean`) is an original Lean 4.14.0 module:
//! a mutual `Tree`/`Forest` block, Lean's generated `Tree.brecOn`/`Forest.brecOn`, a structural
//! recursion through them and an `rfl` computation. Lean's kernel accepts all of it and
//! `#print axioms` reports none.
//!
//! `Tree.brecOn`'s body needs `Tree.below m₁ m₂ (node a) ≡ PProd (m₂ a) (Forest.below m₁ m₂ a)`.
//! Unfolding the mutual fixpoint behind `Tree.below` substitutes a projection of the block that
//! mentions the local motives; the independent re-checker once substituted it without shifting
//! it under the remaining mutual binders, so the motives pointed at the wrong variables and the
//! re-checker refused what Lean and the main kernel accept (the 54 `recheck_refused` roots).
use logicaffeine_kernel::interface::lean as adapter;
use logicaffeine_kernel::{
    axiom_dependencies_of, infer_type, normalize_checked, recheck, BigInt, KernelError, Literal,
    ReCheckError, Term, Universe,
};
use serde_json::Value;

const SOURCE: &str = include_str!("fixtures/lean_semantics/LF_MutualBrecOn.lean");
const INPUT: &str = include_str!("fixtures/lean_semantics/LF_MutualBrecOn.ndjson");
const PROVENANCE: &str = include_str!("fixtures/lean_semantics/LF_MutualBrecOn.provenance.json");

fn n(dotted: &str) -> String {
    dotted.split('.').map(|p| format!("/s:{p:?}")).collect()
}
fn g(dotted: &str) -> Term {
    Term::Global(n(dotted))
}
fn v(s: &str) -> Term {
    Term::Var(s.into())
}
fn app(f: Term, x: Term) -> Term {
    Term::App(Box::new(f), Box::new(x))
}
fn apps(f: Term, xs: Vec<Term>) -> Term {
    xs.into_iter().fold(f, app)
}
fn pi(x: &str, a: Term, b: Term) -> Term {
    Term::Pi {
        param: x.into(),
        param_type: Box::new(a),
        body_type: Box::new(b),
    }
}
fn lam(x: &str, a: Term, b: Term) -> Term {
    Term::Lambda {
        param: x.into(),
        param_type: Box::new(a),
        body: Box::new(b),
    }
}
fn u() -> Universe {
    Universe::Var(n("u"))
}

fn original() -> (adapter::ImportedContext, adapter::ImportReport) {
    let p: Value = serde_json::from_str(PROVENANCE).unwrap();
    assert_eq!(
        p["source_sha256"],
        "91c5993b2864752ee9dd7c45efbb68c1426597eba805a2d5f72216ea16c36ef4"
    );
    assert_eq!(
        p["export_sha256"],
        "482505fe2fd2bb4866160abc82b7cf1224e70faf2878ddccfaeb421341896c51"
    );
    assert_eq!(p["toolchain"], "leanprover/lean4:v4.14.0");
    assert_eq!(
        p["exporter_commit"],
        "20c4a64aa1b76f7c79a6db15d089084359f8c3dc"
    );
    assert_eq!(p["records"], INPUT.lines().count());
    assert_eq!(INPUT.len(), 60_510);
    assert!(SOURCE.contains("mutual\ninductive Tree"));
    adapter::import_audited(INPUT).expect("the audited import re-checks Lean's generated brecOn")
}

/// `λ m₁ m₂ x (p : have). (let q : want := p in q)` — a conversion question for both kernels.
fn conversion(have: Term, want: Term) -> Term {
    let sort = Term::Sort(u());
    lam(
        "m1",
        pi("t", g("LF_MB.Tree"), sort.clone()),
        lam(
            "m2",
            pi("t", g("LF_MB.Forest"), sort),
            lam(
                "x",
                g("LF_MB.Forest"),
                lam(
                    "p",
                    have,
                    Term::Let {
                        name: "q".into(),
                        ty: Box::new(want),
                        value: Box::new(v("p")),
                        body: Box::new(v("q")),
                    },
                ),
            ),
        ),
    )
}
fn below(family: &str, x: Term) -> Term {
    apps(
        Term::Const {
            name: n(&format!("LF_MB.{family}.below")),
            levels: vec![u()],
        },
        vec![v("m1"), v("m2"), x],
    )
}
fn pprod(a: Term, b: Term) -> Term {
    let level = Universe::Max(Box::new(Universe::Type(0)), Box::new(u()));
    apps(
        Term::Const {
            name: n("PProd"),
            levels: vec![u(), level],
        },
        vec![a, b],
    )
}

#[test]
fn leans_mutual_brecon_is_accepted_by_both_kernels_with_no_axioms() {
    let (ctx, report) = original();
    for name in [
        "LF_MB.Tree.brecOn",
        "LF_MB.Forest.brecOn",
        "LF_MB.Tree.size",
        "LF_MB.size_sample",
    ] {
        let audit = report
            .declarations
            .iter()
            .find(|d| d.name == n(name))
            .unwrap_or_else(|| panic!("{name} audited"));
        assert!(
            audit.axioms.is_empty(),
            "Lean: {name} depends on no axioms; got {:?}",
            audit.axioms
        );
        let usage = axiom_dependencies_of(&ctx, &n(name)).unwrap();
        assert!(
            usage.axioms.is_empty() && usage.unresolved.is_empty(),
            "{name}: {usage:?}"
        );
    }
    let sample = app(
        g("LF_MB.Tree.size"),
        app(
            g("LF_MB.Tree.node"),
            apps(
                g("LF_MB.Forest.cons"),
                vec![
                    app(g("LF_MB.Tree.node"), g("LF_MB.Forest.nil")),
                    g("LF_MB.Forest.nil"),
                ],
            ),
        ),
    );
    let two = Term::Lit(Literal::Nat(BigInt::from_u64(2)));
    let normal = normalize_checked(&ctx, &sample).unwrap();
    assert!(
        logicaffeine_kernel::defeq_for_test(&ctx, &normal, &two),
        "Tree.size of the sample is 2, got {normal}"
    );
    let three = Term::Lit(Literal::Nat(BigInt::from_u64(3)));
    assert!(!logicaffeine_kernel::defeq_for_test(&ctx, &normal, &three));
}

#[test]
fn the_below_unfolding_obligation_holds_in_both_kernels_and_a_wrong_one_is_refused() {
    let (ctx, _) = original();
    let node = app(g("LF_MB.Tree.node"), v("x"));
    let right = pprod(app(v("m2"), v("x")), below("Forest", v("x")));
    for (have, want) in [
        (below("Tree", node.clone()), right.clone()),
        (right, below("Tree", node.clone())),
    ] {
        let term = conversion(have, want);
        infer_type(&ctx, &term).expect("main kernel");
        recheck(&ctx, &term).expect("independent kernel");
    }
    // A well-typed but different right component is refused by both kernels.
    let level = Universe::Max(Box::new(Universe::Type(0)), Box::new(u()));
    let unit = Term::Const {
        name: n("PUnit"),
        levels: vec![level],
    };
    let wrong = conversion(below("Tree", node), pprod(app(v("m2"), v("x")), unit));
    assert!(
        matches!(
            infer_type(&ctx, &wrong),
            Err(KernelError::TypeMismatch { .. })
        ),
        "{:?}",
        infer_type(&ctx, &wrong)
    );
    assert!(
        matches!(recheck(&ctx, &wrong), Err(ReCheckError::Ill(_))),
        "{:?}",
        recheck(&ctx, &wrong)
    );
}
