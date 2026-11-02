//! T04-015 promotion evidence for `accCount`, `accIsAType` and `ltWf`, from the unchanged original
//! four-root export (`fixtures/lean_conformance/cases_lean414/T04-015.four.ndjson`).
//!
//! Complements `original_well_founded_bridge.rs`, which runs the computations through Lean's own
//! `WellFounded.apply` (a theorem, unfolded as Lean's kernel unfolds theorems). Every computation
//! below instead runs on accessibility proofs built from `Acc.intro`, so it holds independently of
//! theorem transparency, and no assumption is added.
use logicaffeine_kernel::interface::lean as adapter;
use logicaffeine_kernel::{
    axiom_dependencies, axiom_dependencies_of, defeq_for_test, infer_type, normalize_checked,
    recheck, BigInt, Context, KernelError, Literal, ReCheckError, Term, Universe,
};
use serde_json::Value;

const INPUT: &str = include_str!("fixtures/lean_conformance/cases_lean414/T04-015.four.ndjson");
const PROVENANCE: &str =
    include_str!("fixtures/lean_conformance/cases_lean414/T04-015.four.provenance.json");

fn root(short: &str) -> String {
    format!("/s:\"T04_015\"/s:{short:?}")
}
fn g(s: &str) -> Term {
    Term::Global(s.into())
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
fn pi(n: &str, a: Term, b: Term) -> Term {
    Term::Pi {
        param: n.into(),
        param_type: Box::new(a),
        body_type: Box::new(b),
    }
}
fn lam(n: &str, a: Term, b: Term) -> Term {
    Term::Lambda {
        param: n.into(),
        param_type: Box::new(a),
        body: Box::new(b),
    }
}
fn nat() -> Term {
    g("Nat")
}
fn lit(n: u64) -> Term {
    Term::Lit(Literal::Nat(BigInt::from_u64(n)))
}
fn at1(s: &str) -> Term {
    Term::Const {
        name: s.into(),
        levels: vec![Universe::Type(0)],
    }
}
/// Lean's `Nat.lt`, at the reader's reserved-namespace spelling.
fn lt() -> Term {
    g(r#"Nat/s:"lt""#)
}
fn acc(n: Term) -> Term {
    apps(at1(r#"/s:"Acc""#), vec![nat(), lt(), n])
}
fn well_founded() -> Term {
    apps(at1(r#"/s:"WellFounded""#), vec![nat(), lt()])
}
fn apply_wf(n: Term) -> Term {
    apps(
        at1(r#"/s:"WellFounded"/s:"apply""#),
        vec![nat(), lt(), g(&root("ltWf")), n],
    )
}
/// An accessibility proof of `y` that is `Acc.intro` for `depth` levels, then `ltWf`'s.
fn accessible(depth: usize, y: Term) -> Term {
    if depth == 0 {
        return apply_wf(y);
    }
    let (z, h) = (format!("z{depth}"), format!("h{depth}"));
    let step = lam(
        &z,
        nat(),
        lam(
            &h,
            apps(lt(), vec![v(&z), y.clone()]),
            accessible(depth - 1, v(&z)),
        ),
    );
    apps(at1(r#"/s:"Acc"/s:"intro""#), vec![nat(), lt(), y, step])
}
fn annotated(value: Term, expected: Term) -> Term {
    Term::Let {
        name: "original_goal".into(),
        ty: Box::new(expected),
        value: Box::new(value),
        body: Box::new(v("original_goal")),
    }
}
fn checks(ctx: &Context, value: Term, expected: Term) {
    let term = annotated(value, expected);
    infer_type(ctx, &term).expect("main kernel checks the actual goal");
    recheck(ctx, &term).expect("independent kernel checks the actual goal");
}
fn rejects(ctx: &Context, value: Term, expected: Term) {
    let term = annotated(value, expected);
    assert!(
        matches!(
            infer_type(ctx, &term),
            Err(KernelError::TypeMismatch { .. })
        ),
        "{:?}",
        infer_type(ctx, &term)
    );
    assert!(matches!(recheck(ctx, &term), Err(ReCheckError::Ill(_))));
}
fn axiom_free(ctx: &Context, term: &Term) {
    let usage = axiom_dependencies(ctx, term);
    assert!(
        usage.axioms.is_empty() && usage.unresolved.is_empty(),
        "{usage:?}"
    );
}

/// The unchanged original export with `short`'s checked body retained.
fn original(short: &str) -> (adapter::ImportedContext, Term, Term) {
    assert_eq!(INPUT.len(), 263_814);
    assert_eq!(INPUT.lines().count(), 4_907);
    let p: Value = serde_json::from_str(PROVENANCE).unwrap();
    assert_eq!(
        p["source_sha256"],
        "21c0b33bcd73fd5a901f4c76b5fd0c2d5ebd079b5f9dde6ea2aeaa117497d877"
    );
    assert_eq!(
        p["export_sha256"],
        "a8d5e679d0fb82e43a954f5f3edcaa96d58538fafb0344ef8c86a433c833787f"
    );
    assert_eq!(p["toolchain"], "leanprover/lean4:v4.14.0");
    assert_eq!(
        p["exporter_commit"],
        "20c4a64aa1b76f7c79a6db15d089084359f8c3dc"
    );
    assert_eq!(p["expression_equality"], "structural");
    let (ctx, report) = adapter::import_audited_retaining_source(INPUT, &root(short))
        .expect("the unchanged original four-root export imports");
    let audit = report
        .declarations
        .iter()
        .find(|d| d.name == root(short))
        .unwrap();
    assert_eq!(audit.kind, "def");
    assert!(
        audit.axioms.is_empty(),
        "Lean reports no axioms for T04_015.{short}: {:?}",
        audit.axioms
    );
    let usage = axiom_dependencies_of(&ctx, &root(short)).unwrap();
    assert!(usage.axioms.is_empty() && usage.unresolved.is_empty());
    assert!(
        ctx.get_global(r#"Nat/s:"lt""#).is_some(),
        "Lean's Nat.lt at the reserved Nat namespace"
    );
    assert!(
        ctx.get_global(r#"/s:"Nat"/s:"lt""#).is_none(),
        "the reserved namespace has one spelling"
    );
    let (retained, declared, body) = ctx.retained_source().expect("the actual original body");
    assert_eq!(retained, root(short));
    assert_eq!(ctx.get_definition_body(&root(short)), Some(body));
    let (declared, body) = (declared.clone(), body.clone());
    (ctx, declared, body)
}

#[test]
fn original_acc_count_has_its_exact_source_goal_and_rejects_a_wrong_one() {
    let (ctx, declared, body) = original("accCount");
    // Lean: `{x : Nat} → Acc Nat.lt x → Nat`.
    let goal = pi("x", nat(), pi("h", acc(v("x")), nat()));
    assert!(defeq_for_test(&ctx, &declared, &goal));
    checks(&ctx, body.clone(), goal);
    rejects(&ctx, body.clone(), nat());
    rejects(&ctx, body, pi("x", nat(), pi("h", nat(), nat())));
}

#[test]
fn original_acc_count_recurses_through_constructed_accessibility() {
    let (ctx, _, body) = original("accCount");
    for n in [0u64, 1, 3] {
        let proof = accessible(n as usize + 1, lit(n));
        checks(&ctx, proof.clone(), acc(lit(n)));
        axiom_free(&ctx, &proof);
        let value = apps(body.clone(), vec![lit(n), proof]);
        checks(&ctx, value.clone(), nat());
        let normal = normalize_checked(&ctx, &value).expect("the original recursion normalizes");
        assert!(
            defeq_for_test(&ctx, &normal, &lit(n)),
            "accCount {n} computes {normal}"
        );
        let eq = |k: u64| apps(at1(r#"/s:"Eq""#), vec![nat(), value.clone(), lit(k)]);
        let refl = apps(at1(r#"/s:"Eq"/s:"refl""#), vec![nat(), lit(n)]);
        checks(&ctx, refl.clone(), eq(n));
        rejects(&ctx, refl, eq(n + 1));
    }
    // An accessibility proof for a different index is not accepted for this one.
    rejects(
        &ctx,
        apps(body.clone(), vec![lit(1), accessible(2, lit(2))]),
        nat(),
    );
    rejects(&ctx, apps(body, vec![lit(0), lit(0)]), nat());
}

#[test]
fn original_acc_is_a_type_is_the_nat_accessibility_family() {
    let (ctx, declared, body) = original("accIsAType");
    let goal = pi("n", nat(), Term::Sort(Universe::Prop));
    assert!(defeq_for_test(&ctx, &declared, &goal));
    checks(&ctx, body.clone(), goal);
    rejects(&ctx, body.clone(), pi("n", nat(), nat()));
    // Both directions between the original's result and `Acc Nat Nat.lt n`.
    let result = app(body, v("n"));
    checks(
        &ctx,
        lam(
            "n",
            nat(),
            lam("a", acc(v("n")), annotated(v("a"), result.clone())),
        ),
        pi("n", nat(), pi("a", acc(v("n")), result.clone())),
    );
    checks(
        &ctx,
        lam(
            "n",
            nat(),
            lam("a", result.clone(), annotated(v("a"), acc(v("n")))),
        ),
        pi("n", nat(), pi("a", result, acc(v("n")))),
    );
}

#[test]
fn original_lt_wf_is_leans_well_founded_structure_for_nat_lt() {
    let (ctx, declared, body) = original("ltWf");
    assert!(defeq_for_test(&ctx, &declared, &well_founded()));
    checks(&ctx, body.clone(), well_founded());
    rejects(&ctx, body, pi("n", nat(), acc(v("n"))));
    // `WellFounded` is Lean's structure, not an abbreviation for its field.
    assert!(ctx.is_inductive(r#"/s:"WellFounded""#));
    // Its accessibility projection supplies `Acc Nat.lt n` at every `n`, without axioms.
    let supplied = lam("n", nat(), apply_wf(v("n")));
    axiom_free(&ctx, &supplied);
    checks(&ctx, supplied, pi("n", nat(), acc(v("n"))));
    rejects(&ctx, apply_wf(lit(2)), acc(lit(3)));
}
