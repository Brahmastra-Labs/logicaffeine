//! Structure η through the Lean reader: structure-likeness is decided for the whole mutual block,
//! as Lean's kernel decides it.
//!
//! Lean's `isStructureLike` asks for one constructor, no indices and `isRec = false`, and `isRec`
//! is a property of the BLOCK: some constructor of some member takes an argument mentioning a
//! member. So a member of a mutual block is a structure exactly when the block is not recursive,
//! which is also the kernel door's rule (`StructureRefusal::Recursive`). The reader must ask the
//! door for every structure-shaped member of every block, not only for a lone inductive.
//!
//! Original witness (`fixtures/lean_semantics/LK_MutualStructEta`), Lean 4.14.0's kernel:
//! `NA`/`NB` form a block in which no constructor mentions a member, so `NA` is structure-like and
//! `naEta : ∀ a : NA, a = NA.mk a.get := rfl` is ACCEPTED. `MA`/`MB` form a block in which `MA.mk`
//! takes an `MB`: `MB` has one constructor and no indices but is NOT structure-like, and the same
//! `rfl` for `MB` (`mbEta`, built with `addDecl`) is REJECTED — recorded in the compile log.
use logicaffeine_kernel::interface::lean::{self as adapter, emit};
use logicaffeine_kernel::{
    double_check, infer_type, recheck, Context, DoubleCheck, Term, Universe,
};
use serde_json::Value;

const WITNESS: &str = include_str!("fixtures/lean_semantics/LK_MutualStructEta.ndjson");
const WITNESS_PROVENANCE: &str =
    include_str!("fixtures/lean_semantics/LK_MutualStructEta.provenance.json");
const WITNESS_SOURCE: &str = include_str!("fixtures/lean_semantics/LK_MutualStructEta.lean");

fn n(dotted: &str) -> String {
    dotted.split('.').map(|p| format!("/s:{p:?}")).collect()
}

fn g(dotted: &str) -> Term {
    Term::Global(n(dotted))
}

fn app(f: Term, xs: Vec<Term>) -> Term {
    xs.into_iter()
        .fold(f, |f, x| Term::App(Box::new(f), Box::new(x)))
}

/// Lean's `@Eq.{1} T x y` and `@Eq.refl.{1} T x` in the imported context.
fn eq1(ty: Term, x: Term, y: Term) -> Term {
    app(
        Term::Const {
            name: n("Eq"),
            levels: vec![Universe::Type(0)],
        },
        vec![ty, x, y],
    )
}
fn refl1(ty: Term, x: Term) -> Term {
    app(
        Term::Const {
            name: n("Eq.refl"),
            levels: vec![Universe::Type(0)],
        },
        vec![ty, x],
    )
}

/// `(λ h : ∀ v : ty, v = rebuild v. h) (λ v : ty. rfl)` — the proof `rfl` ascribed to the η
/// statement `∀ v, v = rebuild v`, exactly the declaration Lean's `addDecl` was asked to accept.
fn eta_by_rfl(ty: &str, rebuild: impl Fn(Term) -> Term) -> Term {
    let v = || Term::Var("v".into());
    let statement = Term::Pi {
        param: "v".into(),
        param_type: Box::new(g(ty)),
        body_type: Box::new(eq1(g(ty), v(), rebuild(v()))),
    };
    let proof = Term::Lambda {
        param: "v".into(),
        param_type: Box::new(g(ty)),
        body: Box::new(refl1(g(ty), v())),
    };
    Term::App(
        Box::new(Term::Lambda {
            param: "h".into(),
            param_type: Box::new(statement),
            body: Box::new(Term::Var("h".into())),
        }),
        Box::new(proof),
    )
}

fn imported() -> Context {
    let (ctx, _) = adapter::import_audited(WITNESS)
        .unwrap_or_else(|v| panic!("Lean's kernel accepts every exported declaration: {v:?}"));
    (*ctx).clone()
}

#[test]
fn the_mutual_witness_is_the_pinned_lean_4_14_export_and_lean_refuses_the_recursive_member() {
    let p: Value = serde_json::from_str(WITNESS_PROVENANCE).unwrap();
    assert_eq!(p["toolchain"], "leanprover/lean4:v4.14.0");
    assert_eq!(
        p["exporter_commit"],
        "20c4a64aa1b76f7c79a6db15d089084359f8c3dc"
    );
    assert_eq!(
        (p["compiler_exit"].as_i64(), p["exporter_exit"].as_i64()),
        (Some(0), Some(0))
    );
    assert_eq!(p["records"], WITNESS.lines().count());
    let header: Value = serde_json::from_str(WITNESS.lines().next().unwrap()).unwrap();
    assert_eq!(header["meta"]["lean"]["version"], "4.14.0");
    assert_eq!(
        p["declarations"],
        serde_json::json!([
            "LK_MSE.NA",
            "LK_MSE.NA.get",
            "LK_MSE.naEta",
            "LK_MSE.MA",
            "LK_MSE.MB.get"
        ])
    );
    let log = p["compile_log"].as_str().unwrap();
    assert!(log.contains("mbEta: kernel REJECTED"), "{log}");
    assert!(!log.contains("kernel ACCEPTED"), "{log}");
    assert!(WITNESS_SOURCE.contains("theorem naEta (a : NA) : a = NA.mk a.get := rfl"));
    assert!(
        WITNESS_SOURCE.contains("| mk : MB → MA"),
        "the MA/MB block is recursive through MA.mk"
    );
}

#[test]
fn a_member_of_a_non_recursive_mutual_block_is_a_structure_and_its_eta_theorem_imports() {
    let (ctx, report) =
        adapter::import_audited(WITNESS).unwrap_or_else(|v| panic!("naEta is valid Lean: {v:?}"));
    let audit = report
        .declarations
        .iter()
        .find(|d| d.name == n("LK_MSE.naEta"))
        .expect("naEta audited");
    assert_eq!(audit.kind, "thm");
    assert!(audit.axioms.is_empty(), "{:?}", audit.axioms);
    for member in ["LK_MSE.NA", "LK_MSE.NB"] {
        let info = ctx
            .struct_info(&n(member))
            .unwrap_or_else(|| panic!("{member} is structure-like in Lean"));
        assert_eq!(
            (info.mk.as_str(), info.num_params, info.projections.len()),
            (n(&format!("{member}.mk")).as_str(), 0, 1)
        );
    }
}

#[test]
fn no_member_of_a_recursive_mutual_block_is_a_structure() {
    let ctx = imported();
    for member in ["LK_MSE.MA", "LK_MSE.MB"] {
        assert!(ctx.is_inductive(&n(member)), "{member} is imported");
        assert!(
            ctx.struct_info(&n(member)).is_none(),
            "{member}'s block is recursive, so Lean's isRec holds"
        );
    }
}

#[test]
fn eta_by_rfl_checks_exactly_where_leans_kernel_accepts_it() {
    let ctx = imported();
    // `naEta`, rebuilt by hand in the imported context: both kernels accept it.
    let na = eta_by_rfl("LK_MSE.NA", |v| {
        app(g("LK_MSE.NA.mk"), vec![app(g("LK_MSE.NA.get"), vec![v])])
    });
    infer_type(&ctx, &na).unwrap_or_else(|e| panic!("η for a structure-like mutual member: {e:?}"));
    recheck(&ctx, &na).unwrap_or_else(|e| panic!("the re-checker agrees: {e:?}"));
    assert_eq!(double_check(&ctx, &na), DoubleCheck::Agreed);
    // `mbEta`, the declaration Lean's kernel REJECTED: refused by both kernels.
    let mb = eta_by_rfl("LK_MSE.MB", |v| {
        app(g("LK_MSE.MB.mk"), vec![app(g("LK_MSE.MB.get"), vec![v])])
    });
    assert!(
        infer_type(&ctx, &mb).is_err(),
        "MB is not structure-like: no η"
    );
    assert!(
        recheck(&ctx, &mb).is_err(),
        "and the re-checker refuses it too"
    );
}

#[test]
fn a_false_reconstruction_of_a_structure_like_member_is_refused() {
    let ctx = imported();
    // The field's type, read off `NA.mk`, and its two constructors (Lean's `Nat.zero`/`Nat.succ`,
    // under whatever key the reader registered `Nat` with).
    let Some(Term::Pi {
        param_type: nat, ..
    }) = ctx.get_global(&n("LK_MSE.NA.mk")).cloned()
    else {
        panic!("NA.mk takes one field");
    };
    let Term::Global(nat) = *nat else {
        panic!("the field type is a constant: {nat:?}")
    };
    let constructors = ctx.get_constructors(&nat);
    let pick = |fields: bool| {
        constructors
            .iter()
            .find(|(_, ty)| matches!(ty, Term::Pi { .. }) == fields)
            .map(|(c, _)| Term::Global(c.to_string()))
            .unwrap_or_else(|| {
                panic!(
                    "{nat} has a constructor with{} a field",
                    if fields { "" } else { "out" }
                )
            })
    };
    let (zero, succ) = (pick(false), pick(true));
    // Each control is well typed — only η's verdict can refuse it.
    let well_typed = |t: &Term| {
        infer_type(&ctx, t)
            .map(|_| ())
            .unwrap_or_else(|e| panic!("{t:?}: {e:?}"))
    };
    let v = Term::Var("v".into());
    let shifted_value = app(
        g("LK_MSE.NA.mk"),
        vec![app(
            succ.clone(),
            vec![app(g("LK_MSE.NA.get"), vec![v.clone()])],
        )],
    );
    let scoped = ctx.extend("v", g("LK_MSE.NA"));
    infer_type(&scoped, &shifted_value)
        .unwrap_or_else(|e| panic!("the shifted reconstruction is well typed: {e:?}"));
    well_typed(&app(g("LK_MSE.NA.mk"), vec![zero.clone()]));
    let shifted = eta_by_rfl("LK_MSE.NA", |v| {
        app(
            g("LK_MSE.NA.mk"),
            vec![app(succ.clone(), vec![app(g("LK_MSE.NA.get"), vec![v])])],
        )
    });
    assert!(
        infer_type(&ctx, &shifted).is_err(),
        "η does not equate a with NA.mk (succ a.get)"
    );
    assert!(recheck(&ctx, &shifted).is_err());
    let constant = eta_by_rfl("LK_MSE.NA", |_| app(g("LK_MSE.NA.mk"), vec![zero.clone()]));
    assert!(
        infer_type(&ctx, &constant).is_err(),
        "nor with a constant reconstruction"
    );
    assert!(recheck(&ctx, &constant).is_err());
}

#[test]
fn the_emitted_mutual_structure_theorem_replays_in_a_fresh_session() {
    let target = n("LK_MSE.naEta");
    let intake = adapter::intake::read(WITNESS, &adapter::intake::STAGE_A)
        .unwrap_or_else(|e| panic!("intake: {e:?}"));
    let closure = intake
        .closure(&target)
        .unwrap_or_else(|e| panic!("closure: {e:?}"));
    let (imported, _) = adapter::import_retaining_closure(WITNESS, &target, &closure)
        .unwrap_or_else(|e| panic!("import: {e:?}"));
    let doc = emit::emit_document(&imported, &target).unwrap_or_else(|e| panic!("emission: {e:?}"));
    let fresh = emit::verify(&doc)
        .unwrap_or_else(|e| panic!("the emitted document replays: {e}\n{}", doc.source));
    assert!(
        doc.source.contains("Structure "),
        "the document grants η through the vernacular door:\n{}",
        doc.source
    );
    let names: Vec<&str> = doc
        .correspondence
        .iter()
        .map(|c| c.logos.as_str())
        .collect();
    let na = doc
        .correspondence
        .iter()
        .find(|c| c.importer_key == n("LK_MSE.NA"))
        .expect("NA is written");
    assert!(
        fresh.context().struct_info(&na.logos).is_some(),
        "the replay grants NA η: {names:?}"
    );
}
