//! Structure-eta iota: Lean's kernel reduces the recursor of a non-recursive, never-`Prop`
//! structure on ANY major premise, by expanding it to `mk params (proj₀ e) … (projₙ e)`
//! (lean4lean `toCtorWhenStruct`). `Prod.instLawfulBEq` — 18 open roots rejected as
//! "conversion mismatch" — needs it: `instBEqProd` is a pattern match, so `a == b` for VARIABLE
//! pairs only unfolds through that rule.
//!
//! Original witnesses (`fixtures/lean_semantics/`), all accepted by Lean 4.14.0's kernel:
//! `LF_StructEtaIota` (a copied structure and Lean's own `Prod`, matched on variables) and
//! `LF_StructEtaControls` (a `Type` structure's recursor reduced on a variable). The negative
//! control is the kernel's own refusal, recorded in the controls source: the same reduction for a
//! `Prop` structure (`propPairRecVar`) is REJECTED by Lean's kernel, and must be here too.
use logicaffeine_kernel::interface::lean::{self as adapter, DiagnosticCode, Verdict};
use logicaffeine_kernel::{infer_type, recheck, KernelError, ReCheckError, Term, Universe};
use serde_json::{json, Value};

const IOTA: &str = include_str!("fixtures/lean_semantics/LF_StructEtaIota.ndjson");
const IOTA_PROVENANCE: &str =
    include_str!("fixtures/lean_semantics/LF_StructEtaIota.provenance.json");
const CONTROLS: &str = include_str!("fixtures/lean_semantics/LF_StructEtaControls.ndjson");
const CONTROLS_PROVENANCE: &str =
    include_str!("fixtures/lean_semantics/LF_StructEtaControls.provenance.json");
const CONTROLS_SOURCE: &str = include_str!("fixtures/lean_semantics/LF_StructEtaControls.lean");

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
fn lam(x: &str, a: Term, b: Term) -> Term {
    Term::Lambda {
        param: x.into(),
        param_type: Box::new(a),
        body: Box::new(b),
    }
}
fn nat() -> Term {
    Term::Global("Nat".into())
}
fn zero() -> Term {
    Term::Lit(logicaffeine_kernel::Literal::Nat(
        logicaffeine_kernel::BigInt::from_u64(0),
    ))
}

fn pinned(provenance: &str, source_sha: &str, export_sha: &str, input: &str) {
    let p: Value = serde_json::from_str(provenance).unwrap();
    assert_eq!(p["source_sha256"], source_sha);
    assert_eq!(p["export_sha256"], export_sha);
    assert_eq!(p["toolchain"], "leanprover/lean4:v4.14.0");
    assert_eq!(
        p["exporter_commit"],
        "20c4a64aa1b76f7c79a6db15d089084359f8c3dc"
    );
    assert_eq!(
        (p["compiler_exit"].as_i64(), p["exporter_exit"].as_i64()),
        (Some(0), Some(0))
    );
    assert_eq!(p["records"], input.lines().count());
}

#[test]
fn matches_on_variable_structures_unfold_as_in_lean() {
    pinned(
        IOTA_PROVENANCE,
        "d41b1e843fc781b260926f862c7f93c314c45223f4249075de5273cee6c64266",
        "ce7b94516d4a83fa80820e8d9471442937fd0e9f5b8abad8ecea2ef00978884e",
        IOTA,
    );
    let (ctx, report) = adapter::import_audited(IOTA).expect("Lean's kernel accepts both theorems");
    for theorem in ["LF_SE.unfoldsOnVariables", "LF_SE.prodUnfoldsOnVariables"] {
        let audit = report
            .declarations
            .iter()
            .find(|d| d.name == n(theorem))
            .unwrap();
        assert_eq!(audit.kind, "thm");
        assert!(audit.axioms.is_empty(), "{theorem}: {:?}", audit.axioms);
    }
    // `Prod.rec` itself unfolds on a variable pair to its minor at the pair's projections.
    assert!(ctx.get_universe_poly(&n("Prod.rec")).is_some());
}

#[test]
fn a_false_unfolding_on_variables_is_still_rejected() {
    // Restate `unfoldsOnVariables` as `beqByMatch a b = true` for all variables `a b`: false, so
    // its `rfl` proof must be refused as invalid, never accepted through the new iota rule.
    let mut rows: Vec<Value> = IOTA
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let name_id = |rows: &[Value], dotted: &str| -> u64 {
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
    };
    let target = name_id(&rows, "LF_SE.unfoldsOnVariables");
    let at = rows
        .iter()
        .position(|r| r["thm"]["name"] == target)
        .unwrap();
    let bool_true = rows
        .iter()
        .find(|r| {
            r.get("const").is_some() && r["const"]["name"] == json!(name_id(&rows, "Bool.true"))
        })
        .map(|r| r["ie"].clone());
    let Some(bool_true) = bool_true else {
        panic!("Bool.true occurs in the export")
    };
    let ty = rows[at]["thm"]["type"].clone();
    let find = |rows: &[Value], id: &Value| rows.iter().position(|r| r["ie"] == *id).unwrap();
    // The statement is `Π … (a b), Eq Bool (beqByMatch a b) rhs`: walk to the Eq application.
    let mut cur = ty.clone();
    let mut path = Vec::new();
    while let Some(binder) = rows[find(&rows, &cur)].get("forallE").cloned() {
        path.push(binder.clone());
        cur = binder["body"].clone();
    }
    let eq_app = rows[find(&rows, &cur)]["app"].clone();
    let next = rows.iter().filter_map(|r| r["ie"].as_u64()).max().unwrap() + 1;
    let mut rebuilt = Vec::new();
    rebuilt.push(json!({"ie": next, "app": {"fn": eq_app["fn"], "arg": bool_true}}));
    let mut body = json!(next);
    for (i, binder) in path.iter().rev().enumerate() {
        let mut b = binder.clone();
        b["body"] = body;
        let id = next + 1 + i as u64;
        rebuilt.push(json!({"ie": id, "forallE": b}));
        body = json!(id);
    }
    for (k, row) in rebuilt.into_iter().enumerate() {
        rows.insert(at + k, row);
    }
    let at = at + path.len() + 1;
    rows[at]["thm"]["type"] = body;
    let text: String = rows.iter().map(|r| format!("{r}\n")).collect();
    let verdict = adapter::check_str(&text).expect_err("a false equation on variables");
    assert!(matches!(verdict, Verdict::Reject(_)), "{verdict:?}");
    assert_eq!(verdict.reason().code(), DiagnosticCode::InvalidProof);
}

#[test]
fn a_type_structure_eta_reduces_and_a_prop_structure_does_not_as_leans_kernel_decides() {
    pinned(
        CONTROLS_PROVENANCE,
        "b09d8d3230ba967571da64a68c0f8314127f6ac69313566ec96f58092e60d414",
        "9b97dc50bfe1c51b8ce0c12d5bdd235e4c61686c3c3c5cf16bf80f415baf50eb",
        CONTROLS,
    );
    assert!(
        CONTROLS_SOURCE.contains("propPairRecVar"),
        "the kernel refusal is part of the witness"
    );
    let (ctx, _) = adapter::import_audited(CONTROLS).expect("Lean's kernel accepts typePairRecVar");
    // The `Prop` structure: `λ h. Eq.refl 0 : Π h, PropPair.rec (fun _ _ => 0) h = 0` —
    // exactly the declaration Lean 4.14.0's kernel REJECTS. Both kernels here must reject it too.
    // `True` is not in this export; any exported proposition serves: `0 = 0` at `Nat`.
    let t = || {
        apps(
            Term::Const {
                name: n("Eq"),
                levels: vec![Universe::Type(0)],
            },
            vec![nat(), zero(), zero()],
        )
    };
    let pp = apps(g("LF_SEC.PropPair"), vec![t(), t()]);
    let motive = lam("x", pp.clone(), nat());
    let minor = lam("a", t(), lam("b", t(), zero()));
    let rec = apps(
        Term::Const {
            name: n("LF_SEC.PropPair.rec"),
            levels: vec![Universe::Type(0)],
        },
        vec![t(), t(), motive, minor, v("h")],
    );
    let eq = |a: Term, b: Term| {
        apps(
            Term::Const {
                name: n("Eq"),
                levels: vec![Universe::Type(0)],
            },
            vec![nat(), a, b],
        )
    };
    let refl = apps(
        Term::Const {
            name: n("Eq.refl"),
            levels: vec![Universe::Type(0)],
        },
        vec![nat(), zero()],
    );
    let claim = lam(
        "h",
        pp,
        Term::Let {
            name: "c".into(),
            ty: Box::new(eq(rec, zero())),
            value: Box::new(refl),
            body: Box::new(v("c")),
        },
    );
    assert!(
        matches!(
            infer_type(&ctx, &claim),
            Err(KernelError::TypeMismatch { .. })
        ),
        "{:?}",
        infer_type(&ctx, &claim)
    );
    assert!(
        matches!(recheck(&ctx, &claim), Err(ReCheckError::Ill(_))),
        "{:?}",
        recheck(&ctx, &claim)
    );
    // The `Type` structure's recursor on a variable is the minor at its projections, in both kernels.
    let theorem = n("LF_SEC.typePairRecVar");
    assert!(ctx.get_global(&theorem).is_some());
    let proof = ctx
        .get_definition_body(&theorem)
        .expect("theorems are published with their proof")
        .clone();
    infer_type(&ctx, &proof).expect("main kernel");
    recheck(&ctx, &proof).expect("independent kernel");
}
