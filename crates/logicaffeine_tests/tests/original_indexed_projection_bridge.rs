//! W03 indexed projection (F03): Lean's kernel accepts a projection out of an indexed family.
//!
//! Lean's elaborator only writes `Expr.proj` for structures, so the witness
//! (`fixtures/lean_semantics/LF_W03_IndexedProjection.lean`) hands raw projection terms to Lean
//! 4.14.0's kernel with `addDecl`, and the kernel ACCEPTS every one — a projection at a fixed
//! index, at a generic index variable, a dependent second field whose type mentions the first
//! field's projection, and a family with both a parameter and an index. Lean's `rfl` theorems fix
//! the reduction of a projection on a constructor. The pinned exporter records the kernel's own
//! terms. Before this lowering the reader declined all of it ("indexed projection lowering").
use logicaffeine_kernel::interface::lean::{self as adapter, DiagnosticCode, Verdict};
use logicaffeine_kernel::{
    axiom_dependencies_of, defeq_for_test, infer_type, normalize_checked, recheck, Context,
    KernelError, ReCheckError, Term, Universe,
};
use serde_json::{json, Value};

const SOURCE: &str = include_str!("fixtures/lean_semantics/LF_W03_IndexedProjection.lean");
const INPUT: &str = include_str!("fixtures/lean_semantics/LF_W03_IndexedProjection.ndjson");
const PROVENANCE: &str =
    include_str!("fixtures/lean_semantics/LF_W03_IndexedProjection.provenance.json");

fn n(dotted: &str) -> String {
    dotted.split('.').map(|p| format!("/s:{p:?}")).collect()
}
fn p(short: &str) -> String {
    n(&format!("LF_W03P.{short}"))
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
fn pi(x: &str, a: Term, b: Term) -> Term {
    Term::Pi {
        param: x.into(),
        param_type: Box::new(a),
        body_type: Box::new(b),
    }
}
fn nat() -> Term {
    g("Nat")
}
fn lit(k: u64) -> Term {
    Term::Lit(logicaffeine_kernel::Literal::Nat(
        logicaffeine_kernel::BigInt::from_u64(k),
    ))
}
/// Lean's elaborated numeral `@OfNat.ofNat Nat k (instOfNatNat k)`.
fn numeral(k: u64) -> Term {
    apps(
        Term::Const {
            name: n("OfNat.ofNat"),
            levels: vec![Universe::Prop],
        },
        vec![nat(), lit(k), app(g(&n("instOfNatNat")), lit(k))],
    )
}
fn annotated(value: Term, ty: Term) -> Term {
    Term::Let {
        name: "claimed".into(),
        ty: Box::new(ty),
        value: Box::new(value),
        body: Box::new(v("claimed")),
    }
}
fn checks(ctx: &Context, term: &Term, expected: &Term) {
    let main = infer_type(ctx, term).expect("main kernel");
    assert!(
        defeq_for_test(ctx, &main, expected),
        "main: {main} vs {expected}"
    );
    let independent = recheck(ctx, term).expect("independent kernel");
    assert!(
        defeq_for_test(ctx, &independent, expected),
        "independent: {independent} vs {expected}"
    );
}
fn rejects(ctx: &Context, term: &Term) {
    assert!(
        matches!(infer_type(ctx, term), Err(KernelError::TypeMismatch { .. })),
        "{:?}",
        infer_type(ctx, term)
    );
    assert!(matches!(recheck(ctx, term), Err(ReCheckError::Ill(_))));
}

fn original() -> (adapter::ImportedContext, adapter::ImportReport) {
    let prov: Value = serde_json::from_str(PROVENANCE).unwrap();
    assert_eq!(
        prov["source_sha256"],
        "1614d4a6447d0cf49fd5a805634742e4fd52c3f072651730869369c0a73a9067"
    );
    assert_eq!(
        prov["export_sha256"],
        "89d2ae1e277336d9000418cfac0de118c0ba98b162d4a9383e3b13edda0d3107"
    );
    assert_eq!(prov["toolchain"], "leanprover/lean4:v4.14.0");
    assert_eq!(
        prov["exporter_commit"],
        "20c4a64aa1b76f7c79a6db15d089084359f8c3dc"
    );
    assert_eq!(prov["expression_equality"], "structural");
    assert_eq!(
        (
            prov["compiler_exit"].as_i64(),
            prov["exporter_exit"].as_i64()
        ),
        (Some(0), Some(0))
    );
    assert_eq!(prov["records"], INPUT.lines().count());
    assert_eq!(INPUT.len(), 653_907);
    assert_eq!(SOURCE.len(), 3_627);
    assert!(
        SOURCE.contains("addDecl"),
        "the witness is built through Lean's kernel door"
    );
    adapter::import_audited(INPUT).expect("Lean-kernel-accepted indexed projections import")
}

/// The `Fin 3` value Lean itself elaborated for `2` in `firstAtZero_mk`.
fn fin_two(ctx: &Context) -> Term {
    let mut ty = ctx.get_global(&p("firstAtZero_mk")).unwrap().clone();
    // Eq Nat (firstAtZero (IBox.mk 7 x)) 7  ↦  x
    let Term::App(eq_lhs, _) = ty.clone() else {
        panic!("{ty}")
    };
    let Term::App(_, lhs) = *eq_lhs else { panic!() };
    let Term::App(_, boxed) = *lhs else { panic!() };
    let Term::App(_, x) = *boxed else { panic!() };
    ty = *x;
    ty
}

#[test]
fn indexed_families_keep_their_indices() {
    let (ctx, _) = original();
    for (family, params, indices) in [("IBox", 0, 1), ("DBox", 0, 1), ("PBox", 1, 1)] {
        assert_eq!(ctx.inductive_num_params(&p(family)), params, "{family}");
        assert_eq!(ctx.inductive_num_indices(&p(family)), indices, "{family}");
        assert_eq!(
            ctx.get_constructors(&p(family)).len(),
            1,
            "{family} has one constructor"
        );
    }
}

#[test]
fn original_indexed_projections_have_leans_declared_types() {
    let (ctx, report) = original();
    let ibox = |i: Term| app(g(&p("IBox")), i);
    let expected = [
        ("firstAtZero", pi("b", ibox(numeral(0)), nat())),
        (
            "firstAtAnyIndex",
            pi("i", nat(), pi("b", ibox(v("i")), nat())),
        ),
        (
            "paramFirst",
            pi(
                "α",
                Term::Sort(Universe::Type(0)),
                pi(
                    "b",
                    apps(g(&p("PBox")), vec![v("α"), g(&n("Bool.true"))]),
                    v("α"),
                ),
            ),
        ),
    ];
    for (short, ty) in expected {
        let name = p(short);
        assert!(
            defeq_for_test(&ctx, ctx.get_global(&name).unwrap(), &ty),
            "{short}"
        );
        checks(&ctx, ctx.get_definition_body(&name).unwrap(), &ty);
        let audit = report.declarations.iter().find(|d| d.name == name).unwrap();
        assert!(audit.axioms.is_empty(), "{short}: {:?}", audit.axioms);
        let usage = axiom_dependencies_of(&ctx, &name).unwrap();
        assert!(usage.axioms.is_empty() && usage.unresolved.is_empty());
    }
    // Exactly Lean's `#print axioms`: the `Fin` literal `2 : Fin 3` brings `propext`.
    for (theorem, lean_axioms) in [
        ("firstAtZero_mk", vec![n("propext")]),
        ("paramFirst_mk", vec![]),
    ] {
        let audit = report
            .declarations
            .iter()
            .find(|d| d.name == p(theorem))
            .unwrap();
        assert_eq!(audit.kind, "thm");
        assert_eq!(audit.axioms, lean_axioms, "{theorem}");
    }
}

#[test]
fn the_lowered_projection_ranges_over_the_index_and_never_erases_it() {
    let (ctx, _) = original();
    // λ i. λ b. match b with motive λ(index). λ(value : IBox index). Nat
    let body = ctx.get_definition_body(&p("firstAtAnyIndex")).unwrap();
    let Term::Lambda { body, .. } = body else {
        panic!("{body}")
    };
    let Term::Lambda { body, .. } = body.as_ref() else {
        panic!("{body}")
    };
    let Term::Match { motive, cases, .. } = body.as_ref() else {
        panic!("a checked match: {body}")
    };
    assert_eq!(cases.len(), 1);
    let Term::Lambda {
        param: index,
        param_type: index_type,
        body: inner,
    } = motive.as_ref()
    else {
        panic!("{motive}")
    };
    assert_eq!(index_type.as_ref(), &nat());
    let Term::Lambda {
        param_type: value_type,
        body: output,
        ..
    } = inner.as_ref()
    else {
        panic!("{inner}")
    };
    assert_eq!(
        value_type.as_ref(),
        &app(g(&p("IBox")), v(index)),
        "the motive's value is at the motive's own index"
    );
    assert_eq!(output.as_ref(), &nat());
}

#[test]
fn projections_reduce_on_constructors_in_both_kernels() {
    let (ctx, _) = original();
    let x = fin_two(&ctx);
    let fin = |k: u64| app(g(&n("Fin")), numeral(k));
    checks(&ctx, &x, &fin(3));
    let mk = apps(g(&p("IBox.mk")), vec![numeral(7), x.clone()]);
    for call in [
        app(g(&p("firstAtZero")), mk.clone()),
        apps(g(&p("firstAtAnyIndex")), vec![numeral(0), mk.clone()]),
    ] {
        checks(&ctx, &call, &nat());
        assert!(
            defeq_for_test(&ctx, &normalize_checked(&ctx, &call).unwrap(), &lit(7)),
            "{call}"
        );
        rejects(&ctx, &annotated(call, fin(3)));
    }
    // At an index no constructor produces, the projection is still the family's field type.
    checks(
        &ctx,
        &app(g(&p("firstAtAnyIndex")), numeral(1)),
        &pi("b", app(g(&p("IBox")), numeral(1)), nat()),
    );
    let boxed = apps(
        g(&p("PBox.mk")),
        vec![g(&n("Bool")), g(&n("Bool.false")), numeral(3)],
    );
    let call = apps(g(&p("paramFirst")), vec![g(&n("Bool")), boxed]);
    checks(&ctx, &call, &g(&n("Bool")));
    assert_eq!(normalize_checked(&ctx, &call).unwrap(), g(&n("Bool.false")));
}

#[test]
fn a_dependent_projection_has_the_dependent_result_type() {
    let (ctx, _) = original();
    let x = fin_two(&ctx);
    // `DBox.mk 2 x : DBox 0` needs `x : Fin (2 + 1)`, which is `Fin 3` by conversion.
    let boxed = apps(g(&p("DBox.mk")), vec![numeral(2), x.clone()]);
    checks(&ctx, &boxed, &app(g(&p("DBox")), numeral(0)));
    let second = app(g(&p("dependentSecond")), boxed);
    let fin = |k: u64| app(g(&n("Fin")), numeral(k));
    checks(&ctx, &second, &fin(3));
    // The projection computes to the field Lean stored. Conversion decides it with the field's
    // `2 < 3` proof folded; fully normalizing that proof unfolds every theorem it cites.
    assert!(defeq_for_test(&ctx, &second, &x));
    // The field type follows the FIRST field's projection: Fin 3 here, never Fin 1 or Fin 2.
    rejects(&ctx, &annotated(second.clone(), fin(1)));
    rejects(&ctx, &annotated(second, fin(2)));
}

// ==== corruption controls on the original export ====

fn rows() -> Vec<Value> {
    INPUT
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}
fn text(rows: &[Value]) -> String {
    rows.iter().map(|r| format!("{r}\n")).collect()
}
fn name_id(rows: &[Value], dotted: &str) -> u64 {
    let mut id = 0;
    for part in dotted.split('.') {
        id = rows
            .iter()
            .find(|r| r["str"]["pre"] == id && r["str"]["str"] == part)
            .unwrap()["in"]
            .as_u64()
            .unwrap();
    }
    id
}
/// The `proj` expression of `firstAtAnyIndex`'s value, mutated.
fn mutate_projection(change: impl FnOnce(&mut Value, &[Value])) -> Verdict {
    let mut rows = rows();
    let def = name_id(&rows, "LF_W03P.firstAtAnyIndex");
    let value = rows.iter().find(|r| r["def"]["name"] == def).unwrap()["def"]["value"].clone();
    let outer = rows.iter().find(|r| r["ie"] == value).unwrap()["lam"]["body"].clone();
    let proj = rows.iter().find(|r| r["ie"] == outer).unwrap()["lam"]["body"].clone();
    let at = rows.iter().position(|r| r["ie"] == proj).unwrap();
    assert!(rows[at].get("proj").is_some());
    let snapshot = rows.clone();
    change(&mut rows[at]["proj"], &snapshot);
    adapter::check_str(&text(&rows)).unwrap_err()
}

#[test]
fn projection_controls_are_rejected() {
    let out_of_range = mutate_projection(|proj, _| proj["idx"] = json!(2));
    let wrong_family =
        mutate_projection(|proj, rows| proj["typeName"] = json!(name_id(rows, "LF_W03P.DBox")));
    let not_inductive = mutate_projection(|proj, rows| {
        proj["typeName"] = json!(name_id(rows, "LF_W03P.firstAtZero"))
    });
    for (what, verdict) in [
        ("index 2 of 2 fields", out_of_range),
        ("DBox projection of an IBox", wrong_family),
        ("a definition as the projection's family", not_inductive),
    ] {
        assert!(matches!(verdict, Verdict::Reject(_)), "{what}: {verdict:?}");
        assert_eq!(
            verdict.reason().code(),
            DiagnosticCode::InvalidProof,
            "{what}: {verdict:?}"
        );
    }
}
