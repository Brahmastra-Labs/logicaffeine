//! Competition rule R10 at its source: every refusal the Lean reader gives carries a TYPED,
//! machine-readable reason, never only a free string.
//!
//! A refusal is built by `declined(code, …)` / `rejected(code, …)`. The two unclassified
//! constructors, `decline(…)` and `reject(…)`, attach `DiagnosticCode::Unclassified`; every textual
//! occurrence of them under `interface/lean/*.rs` (the `*_controls.rs` test modules excluded) is an
//! unclassified refusal site. This lock drives their number to zero:
//!
//! * [`no_unclassified_refusal_site_remains`] is the goal — zero sites, the two helper definitions
//!   included, because a helper nobody may call is a helper that should not exist.
//! * [`unclassified_refusal_sites_only_fall`] is the ratchet while lanes classify the files they own:
//!   the total may never grow. It is a total, not a per-file table, so a lane that MOVES refusal
//!   code between files (say, the inductive functions out of `mod.rs`) is not mistaken for one that
//!   added unclassified refusals.
//! * [`intake_owned_refusal_sites_are_all_classified`] pins the intake lane's own share at zero now:
//!   everything in `mod.rs` outside the semantic-coverage functions, plus the new intake modules.
//!
//! The rest of the file pins the vocabulary itself (stable wire names, one outcome per code, the
//! contract's exit codes) and that the reader actually returns the classified code on the paths the
//! intake contract names.

use logicaffeine_kernel::interface::lean::{self as lean, DiagnosticCode, Outcome, Verdict};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

fn lean_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../logicaffeine_kernel/src/interface/lean")
}

/// The counted files: every `interface/lean/*.rs` except the `*_controls.rs` test modules.
fn counted_files() -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(lean_dir())
        .expect("interface/lean exists")
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "rs"))
        .filter(|path| {
            !path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .ends_with("_controls.rs")
        })
        .collect();
    files.sort();
    files
}

/// `(line, text)` of every unclassified refusal site in `source`: `decline(` or `reject(` not
/// preceded by an identifier character (so `declined(`, `rejected(` and `self.reject_x(` never
/// count, while `fn decline(`, `Err(reject(` and `|| decline(` do).
fn unclassified_sites(source: &str) -> Vec<(usize, String)> {
    let mut sites = Vec::new();
    for (number, line) in source.lines().enumerate() {
        let bytes = line.as_bytes();
        for needle in ["decline(", "reject("] {
            let mut from = 0;
            while let Some(at) = line[from..].find(needle) {
                let start = from + at;
                let preceded = start > 0 && {
                    let c = bytes[start - 1];
                    c.is_ascii_alphanumeric() || c == b'_'
                };
                if !preceded {
                    sites.push((number + 1, line.trim().to_string()));
                }
                from = start + needle.len();
            }
        }
    }
    sites
}

fn file_name(path: &Path) -> String {
    path.file_name().unwrap().to_string_lossy().into_owned()
}

#[test]
fn the_site_counter_counts_exactly_the_unclassified_constructors() {
    let sample = "\
fn decline(s: impl Into<String>) -> Verdict {
    Err(reject(\"x\"))
    .ok_or_else(|| decline(\"y\"))
    declined(DiagnosticCode::MalformedExport, \"z\")
    rejected(DiagnosticCode::InvalidProof, \"w\")
    self.try_reject(a); undecline(b)
";
    let lines: Vec<usize> = unclassified_sites(sample)
        .into_iter()
        .map(|(l, _)| l)
        .collect();
    assert_eq!(
        lines,
        vec![1, 2, 3],
        "only the bare constructors are unclassified sites"
    );
}

/// ★ THE GOAL. Zero unclassified refusal sites in the whole reader. Other lanes classify the files
/// they own; when the last site goes, the `decline`/`reject` helpers go with it.
#[test]
fn no_unclassified_refusal_site_remains() {
    let mut remaining = Vec::new();
    for path in counted_files() {
        let source = std::fs::read_to_string(&path).unwrap();
        for (line, text) in unclassified_sites(&source) {
            remaining.push(format!("{}:{line}: {text}", file_name(&path)));
        }
    }
    assert!(
        remaining.is_empty(),
        "{} refusal site(s) still carry DiagnosticCode::Unclassified — give each a typed code:\n  {}",
        remaining.len(),
        remaining.join("\n  ")
    );
}

/// The ratchet: the number of unclassified sites across the reader may only fall.
#[test]
fn unclassified_refusal_sites_only_fall() {
    let mut per_file: BTreeMap<String, usize> = BTreeMap::new();
    for path in counted_files() {
        let count = unclassified_sites(&std::fs::read_to_string(&path).unwrap()).len();
        if count > 0 {
            per_file.insert(file_name(&path), count);
        }
    }
    let total: usize = per_file.values().sum();
    assert!(
        total <= UNCLASSIFIED_CEILING,
        "unclassified refusal sites grew to {total} (ceiling {UNCLASSIFIED_CEILING}):\n  {:?}",
        per_file
    );
}

/// Unclassified refusal sites across the reader: 365 when the intake lane had classified its share
/// (2026-09-22), zero once every lane had and the two `decline`/`reject` helpers were deleted
/// (2026-09-23). It never rises.
const UNCLASSIFIED_CEILING: usize = 0;

/// Functions of `mod.rs` owned by the semantic-coverage lane (inductive, nested, metadata,
/// binder-metadata and projection lowering). Everything else in `mod.rs` is intake-owned.
const SEMANTIC_FUNCTIONS: &[&str] = &[
    "ctor_rule",
    "ctor_rule_applied",
    "projection",
    "source_binder_kind",
    "install_binder_kinds",
    "structure_view",
    "inductive",
    "ordinary_mutual_inductive",
    "native_nested_recursors",
];

/// The two metadata refusals inside `record_inner` belong to the semantic lane as well.
const SEMANTIC_MESSAGES: &[&str] = &[
    "metadata values must use the exporter's string representation",
    "letFun metadata requires the elaborator-have translation contract",
];

/// Line spans `[start, end]` (1-based, inclusive) of each named function in `source`, found by
/// brace matching from its signature.
fn function_spans(source: &str, names: &[&str]) -> Vec<(usize, usize)> {
    let lines: Vec<&str> = source.lines().collect();
    let mut spans = Vec::new();
    for (at, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        let Some(rest) = trimmed
            .strip_prefix("fn ")
            .or_else(|| trimmed.strip_prefix("pub fn "))
        else {
            continue;
        };
        let name: String = rest
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        if !names.contains(&name.as_str()) {
            continue;
        }
        let mut depth = 0i64;
        let mut opened = false;
        for (end, body) in lines.iter().enumerate().skip(at) {
            for c in body.chars() {
                match c {
                    '{' => {
                        depth += 1;
                        opened = true;
                    }
                    '}' => depth -= 1,
                    _ => {}
                }
            }
            if opened && depth == 0 {
                spans.push((at + 1, end + 1));
                break;
            }
        }
    }
    spans
}

#[test]
fn intake_owned_refusal_sites_are_all_classified() {
    let mod_rs = std::fs::read_to_string(lean_dir().join("mod.rs")).unwrap();
    // A semantic function that has moved to its own module is simply not in `mod.rs` any more.
    let spans = function_spans(&mod_rs, SEMANTIC_FUNCTIONS);
    let owned: Vec<String> = unclassified_sites(&mod_rs)
        .into_iter()
        .filter(|(line, _)| {
            !spans
                .iter()
                .any(|(start, end)| start <= line && line <= end)
        })
        .filter(|(_, text)| !SEMANTIC_MESSAGES.iter().any(|m| text.contains(m)))
        // The two helper definitions stay until the last caller in any lane is classified.
        .filter(|(_, text)| !text.starts_with("fn decline(") && !text.starts_with("fn reject("))
        .map(|(line, text)| format!("mod.rs:{line}: {text}"))
        .collect();
    let mut new_modules = Vec::new();
    for name in ["diagnostic.rs", "intake.rs", "pipeline.rs"] {
        let path = lean_dir().join(name);
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("{name} is an intake module and must exist: {e}"));
        for (line, text) in unclassified_sites(&source) {
            new_modules.push(format!("{name}:{line}: {text}"));
        }
    }
    let all: Vec<String> = owned.into_iter().chain(new_modules).collect();
    assert!(
        all.is_empty(),
        "intake-owned refusal sites are unclassified:\n  {}",
        all.join("\n  ")
    );
}

// ============================================================================================
// THE VOCABULARY
// ============================================================================================

/// Wire names are a contract with every consumer (the result document, MaCSad's worker, the
/// readiness report). They are pinned here; renaming one is a protocol break.
#[test]
fn every_code_has_a_stable_wire_name() {
    let wire: Vec<&str> = DiagnosticCode::ALL.iter().map(|c| c.as_str()).collect();
    assert_eq!(
        wire,
        vec![
            "invalid_proof",
            "malformed_export",
            "unsupported_export_version",
            "unsupported_toolchain",
            "unknown_record_kind",
            "unknown_field",
            "unresolved_metavariable",
            "missing_declaration",
            "duplicate_declaration",
            "missing_module",
            "import_cycle",
            "lean_rejected",
            "lean_acceptance_not_observed",
            "unsupported_feature",
            "recheck_refused",
            "audit_incomplete",
            "disallowed_assumption",
            "assumption_disagreement",
            "resource_limit",
            "infrastructure",
            "internal_inconsistency",
            "usage",
            "unclassified",
        ]
    );
    let distinct: BTreeSet<&str> = wire.iter().copied().collect();
    assert_eq!(distinct.len(), wire.len(), "wire names are distinct");
    for code in DiagnosticCode::ALL {
        assert_eq!(
            DiagnosticCode::from_wire(code.as_str()),
            Some(code),
            "{code} reads back"
        );
        assert_eq!(code.to_string(), code.as_str());
    }
    assert_eq!(DiagnosticCode::from_wire("verified"), None);
}

/// The result contract's outcomes and exit codes: 0 verified · 1 invalid source · 2 unsupported
/// feature · 3 disallowed assumptions · 4 resource limit · 5 infrastructure failure.
#[test]
fn outcomes_carry_the_contracts_exit_codes() {
    let table: Vec<(&str, i32)> = Outcome::ALL
        .iter()
        .map(|o| (o.as_str(), o.exit_code()))
        .collect();
    assert_eq!(
        table,
        vec![
            ("verified", 0),
            ("invalid_source", 1),
            ("unsupported_feature", 2),
            ("disallowed_assumptions", 3),
            ("resource_limit", 4),
            ("infrastructure_failure", 5),
        ]
    );
}

/// Every classified code belongs to exactly one outcome, and the named categories of rule R10 —
/// invalid source, unsupported feature, disallowed assumption, resource limit, infrastructure
/// failure — are reached by distinct codes. `usage` precedes any result and has no outcome;
/// `unclassified` has none either (its outcome follows the verdict's kind, see below).
#[test]
fn every_classified_code_has_exactly_one_outcome() {
    use DiagnosticCode as C;
    let expected: &[(DiagnosticCode, Option<Outcome>)] = &[
        (C::InvalidProof, Some(Outcome::InvalidSource)),
        (C::MalformedExport, Some(Outcome::InvalidSource)),
        (
            C::UnsupportedExportVersion,
            Some(Outcome::UnsupportedFeature),
        ),
        (C::UnsupportedToolchain, Some(Outcome::UnsupportedFeature)),
        (C::UnknownRecordKind, Some(Outcome::UnsupportedFeature)),
        (C::UnknownField, Some(Outcome::UnsupportedFeature)),
        (C::UnresolvedMetavariable, Some(Outcome::InvalidSource)),
        (C::MissingDeclaration, Some(Outcome::InvalidSource)),
        (C::DuplicateDeclaration, Some(Outcome::InvalidSource)),
        (C::MissingModule, Some(Outcome::InvalidSource)),
        (C::ImportCycle, Some(Outcome::InvalidSource)),
        (C::LeanRejected, Some(Outcome::InvalidSource)),
        (
            C::LeanAcceptanceNotObserved,
            Some(Outcome::UnsupportedFeature),
        ),
        (C::UnsupportedFeature, Some(Outcome::UnsupportedFeature)),
        (C::RecheckRefused, Some(Outcome::UnsupportedFeature)),
        (C::AuditIncomplete, Some(Outcome::UnsupportedFeature)),
        (
            C::DisallowedAssumption,
            Some(Outcome::DisallowedAssumptions),
        ),
        (
            C::AssumptionDisagreement,
            Some(Outcome::DisallowedAssumptions),
        ),
        (C::ResourceLimit, Some(Outcome::ResourceLimit)),
        (C::Infrastructure, Some(Outcome::InfrastructureFailure)),
        (
            C::InternalInconsistency,
            Some(Outcome::InfrastructureFailure),
        ),
        (C::Usage, None),
        (C::Unclassified, None),
    ];
    assert_eq!(
        expected.len(),
        DiagnosticCode::ALL.len(),
        "every code is listed"
    );
    for (code, outcome) in expected {
        assert_eq!(code.outcome(), *outcome, "{code}");
    }
}

/// An unclassified refusal keeps the benchmark protocol's meaning: a rejection is invalid source
/// and a decline is an unsupported feature. A classified one follows its code, whatever its kind.
#[test]
fn a_verdicts_outcome_follows_its_code_then_its_kind() {
    let reject = Verdict::Reject(lean::Reason::new(DiagnosticCode::Unclassified, "x"));
    let decline = Verdict::Decline(lean::Reason::new(DiagnosticCode::Unclassified, "x"));
    assert_eq!(reject.outcome(), Outcome::InvalidSource);
    assert_eq!(decline.outcome(), Outcome::UnsupportedFeature);
    let budget = Verdict::Decline(lean::Reason::new(DiagnosticCode::ResourceLimit, "x"));
    assert_eq!(budget.outcome(), Outcome::ResourceLimit);
    let malformed = Verdict::Decline(lean::Reason::new(DiagnosticCode::MalformedExport, "x"));
    assert_eq!(malformed.outcome(), Outcome::InvalidSource);
    let usage = Verdict::Decline(lean::Reason::new(DiagnosticCode::Usage, "x"));
    assert_eq!(
        usage.outcome(),
        Outcome::UnsupportedFeature,
        "usage falls back to the verdict kind"
    );
}

/// A reason's line and declaration are structured fields; the message the benchmark protocol prints
/// is unchanged by them.
#[test]
fn a_reason_locates_itself_without_changing_its_protocol_text() {
    let reason = lean::Reason::new(
        DiagnosticCode::DuplicateDeclaration,
        "duplicate declaration",
    )
    .at_line(7)
    .in_declaration("Demo.andSwap");
    assert_eq!(reason.line(), Some(7));
    assert_eq!(reason.declaration(), Some("Demo.andSwap"));
    assert_eq!(reason.message(), "duplicate declaration");
    assert_eq!(format!("{reason:?}"), "\"duplicate declaration\"");
}

// ============================================================================================
// THE READER RETURNS THE CLASSIFIED CODE
// ============================================================================================

const HEADER: &str = r#"{"meta":{"lean":{"version":"4.14.0","githash":"410fab7284703f41660ca2454218dcca9b2ec896"},"format":{"version":"3.1.0"},"exporter":{"version":"3.1.0","name":"lean4export"}}}"#;

/// `Prop : Type`, an axiom `ax : Prop`, then `rows`.
fn export(rows: &[&str]) -> String {
    let mut lines = vec![
        HEADER.to_string(),
        r#"{"in":1,"str":{"pre":0,"str":"ax"}}"#.to_string(),
        r#"{"ie":0,"sort":0}"#.to_string(),
        r#"{"axiom":{"name":1,"levelParams":[],"type":0,"isUnsafe":false}}"#.to_string(),
    ];
    lines.extend(rows.iter().map(|r| r.to_string()));
    lines.join("\n") + "\n"
}

fn refusal(input: &str) -> Verdict {
    lean::check_str(input).expect_err("the input must be refused")
}

#[test]
fn the_base_export_is_accepted() {
    lean::check_str(&export(&[])).expect("a lone axiom is a valid environment");
}

#[test]
fn malformed_json_is_a_malformed_export_at_its_line() {
    let verdict = refusal(&export(&[r#"{"in":2,"str":{"pre":0,"#]));
    assert_eq!(verdict.reason().code(), DiagnosticCode::MalformedExport);
    assert_eq!(verdict.reason().line(), Some(5));
}

#[test]
fn an_unknown_format_version_is_an_unsupported_export_version() {
    let verdict = refusal(&export(&[]).replacen("3.1.0\"},\"exporter", "9.9.9\"},\"exporter", 1));
    assert_eq!(
        verdict.reason().code(),
        DiagnosticCode::UnsupportedExportVersion
    );
}

#[test]
fn a_redeclared_constant_is_a_duplicate_declaration() {
    let verdict = refusal(&export(&[
        r#"{"axiom":{"name":1,"levelParams":[],"type":0,"isUnsafe":false}}"#,
    ]));
    assert!(
        matches!(verdict, Verdict::Reject(_)),
        "the protocol still rejects it: {verdict:?}"
    );
    assert_eq!(
        verdict.reason().code(),
        DiagnosticCode::DuplicateDeclaration
    );
    assert_eq!(verdict.reason().line(), Some(5));
    assert_eq!(verdict.reason().declaration(), Some("ax"));
}

#[test]
fn a_reference_to_an_undeclared_constant_is_a_missing_declaration() {
    let verdict = refusal(&export(&[
        r#"{"in":2,"str":{"pre":0,"str":"ghost"}}"#,
        r#"{"in":3,"str":{"pre":0,"str":"uses"}}"#,
        r#"{"ie":1,"const":{"name":2,"us":[]}}"#,
        r#"{"axiom":{"name":3,"levelParams":[],"type":1,"isUnsafe":false}}"#,
    ]));
    assert_eq!(verdict.reason().code(), DiagnosticCode::MissingDeclaration);
    assert_eq!(verdict.reason().line(), Some(8));
    assert_eq!(verdict.reason().declaration(), Some("uses"));
}

#[test]
fn an_unknown_declaration_kind_is_an_unknown_record_kind() {
    let verdict = refusal(&export(&[r#"{"quux":{"name":1}}"#]));
    assert_eq!(verdict.reason().code(), DiagnosticCode::UnknownRecordKind);
}

#[test]
fn a_metavariable_is_an_unresolved_metavariable() {
    let verdict = refusal(&export(&[r#"{"ie":1,"mvar":0}"#]));
    assert_eq!(
        verdict.reason().code(),
        DiagnosticCode::UnresolvedMetavariable
    );
}

#[test]
fn a_duplicated_expression_index_is_a_malformed_export() {
    let verdict = refusal(&export(&[r#"{"ie":0,"sort":0}"#]));
    assert_eq!(verdict.reason().code(), DiagnosticCode::MalformedExport);
}

#[test]
fn an_ill_typed_theorem_is_an_invalid_proof() {
    // thm bad : ax := ax   — the value `ax` has type Prop, not `ax`.
    let verdict = refusal(&export(&[
        r#"{"in":2,"str":{"pre":0,"str":"bad"}}"#,
        r#"{"ie":1,"const":{"name":1,"us":[]}}"#,
        r#"{"thm":{"name":2,"levelParams":[],"type":1,"value":1,"all":[2]}}"#,
    ]));
    assert!(matches!(verdict, Verdict::Reject(_)), "{verdict:?}");
    assert_eq!(verdict.reason().code(), DiagnosticCode::InvalidProof);
    assert_eq!(verdict.outcome(), Outcome::InvalidSource);
    assert_eq!(verdict.reason().declaration(), Some("bad"));
}

#[test]
fn an_exhausted_universe_budget_is_a_resource_limit() {
    let mut rows = Vec::new();
    for i in 1..=300 {
        rows.push(format!(r#"{{"il":{i},"succ":{}}}"#, i - 1));
    }
    let rows: Vec<&str> = rows.iter().map(String::as_str).collect();
    let verdict = refusal(&export(&rows));
    assert_eq!(verdict.reason().code(), DiagnosticCode::ResourceLimit);
    assert_eq!(verdict.outcome(), Outcome::ResourceLimit);
}

/// ★ Once every check of the user's input passed, a refusal of LOGOS's OWN artifact — the emitted
/// document, the assembled package, its replay — is ours: `internal_inconsistency`, an
/// infrastructure failure, never `invalid_source`. Unsupported, budget and environment refusals keep
/// their own meaning.
#[test]
fn our_own_artifact_failing_is_never_the_users_fault() {
    use lean::pipeline::self_verification_failure;
    // Every refusal whose outcome would be `invalid_source` — by its code, or (unclassified) by being a
    // rejection. An unclassified DECLINE is an unsupported feature and keeps that meaning.
    let invalid = [
        DiagnosticCode::InvalidProof,
        DiagnosticCode::MalformedExport,
        DiagnosticCode::MissingDeclaration,
    ];
    let refusals = invalid
        .iter()
        .flat_map(|&code| {
            [
                Verdict::Reject(lean::Reason::new(code, "the correspondence maps x")),
                Verdict::Decline(lean::Reason::new(code, "the correspondence maps x")),
            ]
        })
        .chain([Verdict::Reject(lean::Reason::new(
            DiagnosticCode::Unclassified,
            "the correspondence maps x",
        ))]);
    for verdict in refusals {
        let code = verdict.reason().code();
        {
            let d = self_verification_failure("fresh_replay", &verdict);
            assert_eq!(
                d.code,
                DiagnosticCode::InternalInconsistency,
                "{code} {verdict:?}"
            );
            assert_eq!(d.outcome(), Outcome::InfrastructureFailure);
            assert_eq!(d.stage, "fresh_replay");
            assert!(
                d.message.contains("the correspondence maps x")
                    && d.message.contains(code.as_str()),
                "{}",
                d.message
            );
        }
    }
    let unclassified_decline =
        Verdict::Decline(lean::Reason::new(DiagnosticCode::Unclassified, "m"));
    assert_eq!(
        self_verification_failure("fresh_replay", &unclassified_decline).code,
        DiagnosticCode::Unclassified
    );
    for code in [
        DiagnosticCode::UnsupportedFeature,
        DiagnosticCode::ResourceLimit,
        DiagnosticCode::Infrastructure,
    ] {
        let d = self_verification_failure(
            "emission_reparsed",
            &Verdict::Decline(lean::Reason::new(code, "m")),
        );
        assert_eq!(d.code, code, "{code} keeps its meaning");
    }
}
