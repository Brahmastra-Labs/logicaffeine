//! How the reader reports a verdict the independent kernel did not give. Every such stop belongs to
//! the `independent_recheck` stage; a resource bound that stopped the check is a resource limit, and
//! a refutation — a genuinely ill-typed term — or a construct the independent kernel does not cover
//! is a refused recheck.
use super::*;

const SHALLOW: &str =
    include_str!("../../../../tests/fixtures/lean_semantics/LF_RecheckBudget.shallow.ndjson");

fn n(dotted: &str) -> String {
    dotted.split('.').map(|p| format!("/s:{p:?}")).collect()
}
fn relit(term: &Term, from: u64, to: u64) -> Term {
    let m = |t: &Term| Box::new(relit(t, from, to));
    match term {
        Term::Lit(crate::Literal::Nat(v)) if *v == crate::BigInt::from_u64(from) => {
            Term::Lit(crate::Literal::Nat(crate::BigInt::from_u64(to)))
        }
        Term::App(f, a) => Term::App(m(f), m(a)),
        Term::Lambda {
            param,
            param_type,
            body,
        } => Term::Lambda {
            param: param.clone(),
            param_type: m(param_type),
            body: m(body),
        },
        Term::Pi {
            param,
            param_type,
            body_type,
        } => Term::Pi {
            param: param.clone(),
            param_type: m(param_type),
            body_type: m(body_type),
        },
        other => other.clone(),
    }
}
fn rename(term: &Term, from: &str, to: &str) -> Term {
    let m = |t: &Term| Box::new(rename(t, from, to));
    match term {
        Term::Global(g) if g == from => Term::Global(to.into()),
        Term::Const { name, levels } if name == from => Term::Const {
            name: to.into(),
            levels: levels.clone(),
        },
        Term::App(f, a) => Term::App(m(f), m(a)),
        Term::Lambda {
            param,
            param_type,
            body,
        } => Term::Lambda {
            param: param.clone(),
            param_type: m(param_type),
            body: m(body),
        },
        Term::Pi {
            param,
            param_type,
            body_type,
        } => Term::Pi {
            param: param.clone(),
            param_type: m(param_type),
            body_type: m(body_type),
        },
        other => other.clone(),
    }
}
fn staged(verdict: &Verdict) -> (DiagnosticCode, &'static str, Outcome) {
    let stage = pipeline::import_stage(verdict);
    let diagnostic = pipeline::Diagnostic::of_refusal(stage, verdict);
    (diagnostic.code, diagnostic.stage, diagnostic.outcome())
}

#[test]
fn each_independent_verdict_is_reported_at_the_independent_recheck_stage_with_its_own_code() {
    let (ctx, _) = import_audited(SHALLOW).expect("parity 100 is checked by both kernels");
    let proof = ctx
        .get_definition_body(&n("LF_RB.parityShallow"))
        .unwrap()
        .clone();
    // A genuinely ill-typed term: `@id (parity 100 = false) rfl`, refuted within every bound.
    let ill = rename(&proof, &n("Bool.true"), &n("Bool.false"));
    let refuted = crate::recheck(&ctx, &ill).expect_err("ill-typed");
    assert!(
        matches!(refuted, crate::ReCheckError::Ill(_)),
        "{refuted:?}"
    );
    // The same proof beyond the independent kernel's bound: stopped, not refuted.
    let stopped = crate::recheck(&ctx, &relit(&proof, 100, 800)).expect_err("cut short");
    assert!(
        stopped.incomplete().is_some_and(|i| i.is_resource_limit()),
        "{stopped:?}"
    );
    // A construct outside the covered fragment.
    let uncovered = crate::recheck(&ctx, &Term::Hole).expect_err("a hole");
    assert!(
        uncovered
            .incomplete()
            .is_some_and(|i| !i.is_resource_limit()),
        "{uncovered:?}"
    );

    for (error, code, outcome) in [
        (
            &refuted,
            DiagnosticCode::RecheckRefused,
            Outcome::UnsupportedFeature,
        ),
        (
            &stopped,
            DiagnosticCode::ResourceLimit,
            Outcome::ResourceLimit,
        ),
        (
            &uncovered,
            DiagnosticCode::RecheckRefused,
            Outcome::UnsupportedFeature,
        ),
    ] {
        // As the reader's own declaration check reports it, and as it arrives through the
        // certified-declaration door of `Context`.
        let direct = independent_verdict(error, "independent declaration-body check for x");
        let through_context = kernel(crate::KernelError::IndependentRecheck {
            subject: "checked opaque declaration 'x' independent body".into(),
            error: error.clone(),
        });
        for verdict in [&direct, &through_context] {
            assert!(
                matches!(verdict, Verdict::Decline(_)),
                "never a rejection of valid Lean: {verdict:?}"
            );
            assert_eq!(
                staged(verdict),
                (code, "independent_recheck", outcome),
                "{error:?}"
            );
        }
        assert!(
            through_context.reason().message().starts_with(
                "Certification error: checked opaque declaration 'x' independent body: "
            ),
            "the protocol text is unchanged: {:?}",
            through_context.reason().message()
        );
    }
}

#[test]
fn an_undecided_comparison_of_the_two_kernels_types_is_a_resource_limit_not_a_disagreement() {
    let undecided = Err(declined(
        DiagnosticCode::ResourceLimit,
        "Lean conversion undecided: reduction stopped (BudgetSpent)",
    ));
    let verdict = independent_comparison(
        undecided,
        "independent declaration-body type disagreement for x",
    )
    .unwrap_err();
    assert_eq!(
        staged(&verdict),
        (
            DiagnosticCode::ResourceLimit,
            "independent_recheck",
            Outcome::ResourceLimit
        )
    );
    let refuted = Err(rejected(
        DiagnosticCode::InvalidProof,
        "Lean conversion mismatch",
    ));
    let verdict = independent_comparison(
        refuted,
        "independent declaration-body type disagreement for x",
    )
    .unwrap_err();
    assert_eq!(
        staged(&verdict),
        (
            DiagnosticCode::RecheckRefused,
            "independent_recheck",
            Outcome::UnsupportedFeature
        )
    );
}
