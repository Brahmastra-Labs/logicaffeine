//! A proof the independent kernel could not finish checking is a RESOURCE LIMIT at the
//! `independent_recheck` stage — never an unsupported feature, never a refusal, and never published.
//!
//! Original witness `fixtures/lean_semantics/LF_RecheckBudget.lean` (Lean 4.14.0, pinned lean4export):
//! `parityDeep : parity 800 = true := @id (parity 800 = true) rfl`, where `parity (k+1)` eliminates
//! on `parity k`. Lean's kernel accepts it, and so does LOGOS's primary kernel. The independent kernel
//! types the application `@id (parity 800 = true) rfl` itself, and deciding `parity 800` nests 800
//! demands against its bound of 512: its check is cut short. `parityShallow` (`parity 100`) is the same
//! proof within the bound, verified by both kernels.
use logicaffeine_kernel::interface::lean::pipeline::{check_export, LeanEvidence, Limits};
use logicaffeine_kernel::interface::lean::receipt::StageStatus;
use logicaffeine_kernel::interface::lean::{self as adapter, DiagnosticCode, Outcome, Verdict};
use serde_json::Value;

const SOURCE: &str = include_str!("fixtures/lean_semantics/LF_RecheckBudget.lean");
const SOURCE_SHA: &str = "2e7e99bd5671a7ceb203dac740ac5e6adf0bc63247ddb14e28ebe2e6b0227d02";
const DEEP: &str = include_str!("fixtures/lean_semantics/LF_RecheckBudget.deep.ndjson");
const DEEP_PROVENANCE: &str =
    include_str!("fixtures/lean_semantics/LF_RecheckBudget.deep.provenance.json");
const SHALLOW: &str = include_str!("fixtures/lean_semantics/LF_RecheckBudget.shallow.ndjson");
const SHALLOW_PROVENANCE: &str =
    include_str!("fixtures/lean_semantics/LF_RecheckBudget.shallow.provenance.json");

fn pinned(provenance: &str, export_sha: &str, input: &str, root: &str) {
    let p: Value = serde_json::from_str(provenance).unwrap();
    assert_eq!(
        p["source"],
        "crates/logicaffeine_tests/tests/fixtures/lean_semantics/LF_RecheckBudget.lean"
    );
    assert_eq!(p["source_sha256"], SOURCE_SHA);
    assert_eq!(p["toolchain"], "leanprover/lean4:v4.14.0");
    assert_eq!(
        p["exporter_commit"],
        "20c4a64aa1b76f7c79a6db15d089084359f8c3dc"
    );
    assert_eq!(
        (p["compiler_exit"].as_i64(), p["exporter_exit"].as_i64()),
        (Some(0), Some(0))
    );
    assert_eq!(p["export_sha256"], export_sha);
    assert_eq!(p["records"], input.lines().count());
    assert_eq!(p["declarations"], serde_json::json!([root]));
    assert_eq!(
        p["lean_print_axioms"],
        serde_json::json!([format!("'{root}' does not depend on any axioms")])
    );
}

#[test]
fn a_proof_over_the_independent_budget_is_a_resource_limit_at_the_independent_recheck_stage() {
    pinned(
        DEEP_PROVENANCE,
        "fe1fdb1b315d8ac06f2857acafb4152a1b9551d5ea0aafe1b12747ebd6a8d4a2",
        DEEP,
        "LF_RB.parityDeep",
    );
    assert!(
        SOURCE.contains("theorem parityDeep : parity 800 = true := @id (parity 800 = true) rfl")
    );
    let checked = check_export(
        DEEP,
        "LF_RB.parityDeep",
        &LeanEvidence::NotObserved,
        &Limits::default(),
    );
    let result = &checked.result;
    assert_eq!(
        result.outcome(),
        Outcome::ResourceLimit,
        "{}",
        result.to_json()
    );
    assert_eq!(result.outcome().exit_code(), 4);
    assert_eq!(
        result.status("independent_recheck"),
        StageStatus::Failed,
        "{}",
        result.to_json()
    );
    for later in ["assumption_policy", "emission_reparsed", "fresh_replay"] {
        assert_eq!(
            result.status(later),
            StageStatus::NotRun,
            "{later}: nothing after the stop runs"
        );
    }
    let stop = result
        .diagnostics
        .iter()
        .find(|d| d.stage == "independent_recheck")
        .unwrap_or_else(|| {
            panic!(
                "the stop is reported at the independent recheck: {}",
                result.to_json()
            )
        });
    assert_eq!(
        stop.code,
        DiagnosticCode::ResourceLimit,
        "{}",
        result.to_json()
    );
    assert_eq!(stop.outcome(), Outcome::ResourceLimit);
    assert!(
        result
            .diagnostics
            .iter()
            .all(|d| d.code != DiagnosticCode::RecheckRefused
                && d.code != DiagnosticCode::UnsupportedFeature),
        "{}",
        result.to_json()
    );
    assert!(
        checked.package.is_none(),
        "a result that is not verified publishes nothing"
    );
    let json = result.to_json();
    assert_eq!(json["outcome"], "resource_limit");
    assert_eq!(json["stages"]["independent_recheck"], "failed");
    assert!(json["package"].is_null());
}

#[test]
fn the_benchmark_protocol_declines_it_as_a_resource_limit_never_a_refusal() {
    let verdict = adapter::check_str(DEEP).expect_err("the independent check did not finish");
    assert!(
        matches!(verdict, Verdict::Decline(_)),
        "valid Lean is never rejected: {verdict:?}"
    );
    assert_eq!(
        verdict.reason().code(),
        DiagnosticCode::ResourceLimit,
        "{verdict:?}"
    );
    assert_eq!(verdict.outcome(), Outcome::ResourceLimit);
}

#[test]
fn the_same_proof_within_the_budget_is_checked_by_both_kernels() {
    pinned(
        SHALLOW_PROVENANCE,
        "b46889e3344ce2e491a0f7eebe8a2aa25f116a1ca3426147a1338eeeb61e575a",
        SHALLOW,
        "LF_RB.parityShallow",
    );
    let checked = check_export(
        SHALLOW,
        "LF_RB.parityShallow",
        &LeanEvidence::NotObserved,
        &Limits::default(),
    );
    let result = &checked.result;
    for stage in ["translated", "primary_check", "independent_recheck"] {
        assert_eq!(
            result.status(stage),
            StageStatus::Passed,
            "{stage}: {}",
            result.to_json()
        );
    }
    assert!(
        result
            .diagnostics
            .iter()
            .all(|d| d.code != DiagnosticCode::ResourceLimit),
        "{}",
        result.to_json()
    );
    adapter::check_str(SHALLOW).expect("both kernels check parity 100");
}

// ---- The independent kernel's own account, typed -------------------------------------------------

use logicaffeine_kernel::{
    double_check, infer_type, recheck, DoubleCheck, IncompleteKind, Literal, ReCheckError, Term,
};

fn n(dotted: &str) -> String {
    dotted.split('.').map(|p| format!("/s:{p:?}")).collect()
}

/// `term` with every natural literal `from` replaced by `to`.
fn map_literal(term: &Term, from: u64, to: u64) -> Term {
    let m = |t: &Term| Box::new(map_literal(t, from, to));
    match term {
        Term::Lit(Literal::Nat(v)) if *v == logicaffeine_kernel::BigInt::from_u64(from) => {
            Term::Lit(Literal::Nat(logicaffeine_kernel::BigInt::from_u64(to)))
        }
        Term::App(f, a) => Term::App(m(f), m(a)),
        Term::Pi {
            param,
            param_type,
            body_type,
        } => Term::Pi {
            param: param.clone(),
            param_type: m(param_type),
            body_type: m(body_type),
        },
        Term::Lambda {
            param,
            param_type,
            body,
        } => Term::Lambda {
            param: param.clone(),
            param_type: m(param_type),
            body: m(body),
        },
        Term::Let {
            name,
            ty,
            value,
            body,
        } => Term::Let {
            name: name.clone(),
            ty: m(ty),
            value: m(value),
            body: m(body),
        },
        other => other.clone(),
    }
}
/// `term` with the constant `from` replaced by `to` wherever it occurs.
fn map_constant(term: &Term, from: &str, to: &str) -> Term {
    let m = |t: &Term| Box::new(map_constant(t, from, to));
    match term {
        Term::Global(g) if g == from => Term::Global(to.into()),
        Term::Const { name, levels } if name == from => Term::Const {
            name: to.into(),
            levels: levels.clone(),
        },
        Term::App(f, a) => Term::App(m(f), m(a)),
        Term::Pi {
            param,
            param_type,
            body_type,
        } => Term::Pi {
            param: param.clone(),
            param_type: m(param_type),
            body_type: m(body_type),
        },
        Term::Lambda {
            param,
            param_type,
            body,
        } => Term::Lambda {
            param: param.clone(),
            param_type: m(param_type),
            body: m(body),
        },
        Term::Let {
            name,
            ty,
            value,
            body,
        } => Term::Let {
            name: name.clone(),
            ty: m(ty),
            value: m(value),
            body: m(body),
        },
        other => other.clone(),
    }
}
fn literal_occurrences(term: &Term, value: u64) -> usize {
    match term {
        Term::Lit(Literal::Nat(v)) => {
            usize::from(*v == logicaffeine_kernel::BigInt::from_u64(value))
        }
        Term::App(f, a) => literal_occurrences(f, value) + literal_occurrences(a, value),
        Term::Pi {
            param_type,
            body_type,
            ..
        } => literal_occurrences(param_type, value) + literal_occurrences(body_type, value),
        Term::Lambda {
            param_type, body, ..
        } => literal_occurrences(param_type, value) + literal_occurrences(body, value),
        Term::Let {
            ty, value: v, body, ..
        } => {
            literal_occurrences(ty, value)
                + literal_occurrences(v, value)
                + literal_occurrences(body, value)
        }
        _ => 0,
    }
}

#[test]
fn the_independent_kernel_reports_a_typed_resource_limit_and_double_check_carries_it() {
    let (ctx, _) = adapter::import_audited(SHALLOW).expect("parity 100 is checked by both kernels");
    let proof = ctx
        .get_definition_body(&n("LF_RB.parityShallow"))
        .expect("theorems keep their proof")
        .clone();
    assert!(
        literal_occurrences(&proof, 100) > 0,
        "the proof names its bound: {proof}"
    );
    // The same proof at 800: the primary kernel checks it, the independent kernel is stopped.
    let deep = map_literal(&proof, 100, 800);
    infer_type(&ctx, &deep).expect("the primary kernel checks parity 800");
    match recheck(&ctx, &deep) {
        Err(ReCheckError::Unsupported(incomplete)) => {
            assert!(
                incomplete.is_resource_limit(),
                "a stopped check is a resource limit: {incomplete}"
            );
            assert_eq!(
                incomplete.kind(),
                IncompleteKind::ResourceLimit {
                    steps: 1 << 20,
                    depth: 512,
                    nodes: 524_288
                }
            );
            assert!(
                incomplete.contains("cut short by its budget"),
                "{incomplete}"
            );
        }
        other => panic!("the independent kernel cannot finish parity 800: {other:?}"),
    }
    match double_check(&ctx, &deep) {
        DoubleCheck::MainOnlyReCheckerIncomplete(incomplete) => {
            assert!(incomplete.is_resource_limit(), "{incomplete}")
        }
        other => panic!("single-checked, for want of a resource: {other:?}"),
    }
    // Within the bound, both kernels check it and agree.
    recheck(&ctx, &proof).expect("parity 100 is within the bound");
    assert_eq!(double_check(&ctx, &proof), DoubleCheck::Agreed);
}

#[test]
fn a_genuinely_ill_typed_term_is_refuted_not_stopped() {
    let (ctx, _) = adapter::import_audited(SHALLOW).expect("parity 100 is checked by both kernels");
    let proof = ctx
        .get_definition_body(&n("LF_RB.parityShallow"))
        .unwrap()
        .clone();
    // `@id (parity 100 = false) rfl`: decidably ill-typed, within every bound.
    let ill = map_constant(&proof, &n("Bool.true"), &n("Bool.false"));
    assert_ne!(ill, proof);
    assert!(
        infer_type(&ctx, &ill).is_err(),
        "the primary kernel refuses it"
    );
    assert!(
        matches!(recheck(&ctx, &ill), Err(ReCheckError::Ill(_))),
        "the independent kernel refutes it: {:?}",
        recheck(&ctx, &ill)
    );
    assert_eq!(
        double_check(&ctx, &ill),
        DoubleCheck::Agreed,
        "both kernels refuse it: that is agreement"
    );
    // A construct outside the covered fragment is incompleteness of the other kind.
    match recheck(&ctx, &Term::Hole) {
        Err(ReCheckError::Unsupported(incomplete)) => {
            assert_eq!(incomplete.kind(), IncompleteKind::Unsupported)
        }
        other => panic!("a hole is not covered: {other:?}"),
    }
}
