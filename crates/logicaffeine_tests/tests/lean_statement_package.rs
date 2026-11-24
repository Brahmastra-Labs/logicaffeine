//! Track L4 — statement packages, the `/2` contract.
//!
//! A statement import carries the proposition a Lean declaration states — a theorem's type, a `Prop`
//! definition's value — as the fresh definition `T.logos_statement`, derived deterministically from
//! the original export (`statement::derive`): only that expression's closure is selected, the
//! theorem's own proof never seeded, an `answer(sorry)` hole abstracted as a template parameter, an
//! unfinished proof the statement rests on kept and refused by the assumption policy with its path.
//! The package (`logos-lean-statement-package/1`) carries the original export beside the derived one,
//! and a replay re-derives the artifact from the original and must obtain the same bytes: a package
//! whose artifact states any other proposition — however well typed — fails its binding. The result
//! (`logos-lean-result/2`) and the replay (`logos-lean-replay/2`) say `proof_status: not_claimed`.

mod lean_common;

use lean_common::*;
use logicaffeine_kernel::interface::lean::package::{
    self, build_statement_observed, canonical_json, replay, sha256_hex as package_sha,
    BuildOptions, Manifest, PackageFiles, EXPORT, MANIFEST,
};
use logicaffeine_kernel::interface::lean::pipeline::{check_statement, LeanEvidence, Limits};
use logicaffeine_kernel::interface::lean::receipt::{StageStatus, REPLAY_STAGES};
use logicaffeine_kernel::interface::lean::statement::{
    self, derive, Origin, ANSWER_CATEGORY, INFORMATION_CATEGORY, ORIGINAL,
};
use logicaffeine_kernel::interface::lean::{intake, profile, DiagnosticCode};
use serde_json::{json, Value};
use std::collections::BTreeSet;

const VERSIONS: [&str; 5] = ["4.14.0", "4.28.0", "4.33.1", "4.34.0-rc1", "4.34.1"];

fn fixture(version: &str, stem: &str) -> String {
    std::fs::read_to_string(fixture_path(version, stem)).unwrap()
}

fn fixture_path(version: &str, stem: &str) -> std::path::PathBuf {
    fixtures()
        .join("profiles")
        .join(version)
        .join(format!("{stem}.ndjson"))
}

/// The four statement shapes whose proofs are placeholders.
fn statements(version: &str) -> String {
    fixture(version, "Profile.statements")
}

fn rows(text: &str) -> Vec<Value> {
    text.lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

/// The declaration record of the dotted Lean name `lean` in `text`.
fn record(text: &str, lean: &str) -> Value {
    let index = name_index(text, lean);
    rows(text)
        .into_iter()
        .find(|r| {
            ["thm", "def", "axiom", "opaque"]
                .iter()
                .any(|k| r.get(*k).is_some_and(|d| d["name"] == index))
        })
        .unwrap_or_else(|| panic!("{lean} has a declaration record"))
}

fn declares(text: &str, lean: &str) -> bool {
    let kernel = intake::kernel_name(lean).unwrap();
    intake::read_admitted(text)
        .unwrap()
        .declaration(&kernel)
        .is_some()
}

fn refused(text: &str, target: &str) -> (DiagnosticCode, String) {
    match derive(text, target) {
        Ok(d) => panic!("{target} derived {}", d.relation.artifact),
        Err(v) => (v.reason().code(), v.reason().message().to_string()),
    }
}

// ============================================================================================
// THE DERIVATION
// ============================================================================================

/// ★ A theorem's statement is its type: the artifact's value is exactly the theorem's type record,
/// its type `Prop`, its universe parameters the theorem's; the theorem itself and its placeholder
/// proof are not in the derived export.
#[test]
fn a_theorems_statement_is_its_type_and_never_its_proof() {
    for version in VERSIONS {
        let original = statements(version);
        let derived = derive(&original, "Profile.unbounded").unwrap();
        let r = &derived.relation;
        assert_eq!(r.origin, Origin::TheoremType, "{version}");
        assert_eq!(r.original, "Profile.unbounded");
        assert_eq!(r.original_kind, "thm");
        assert_eq!(r.artifact, "Profile.unbounded.logos_statement");
        assert!(r.answer_holes.is_empty());
        assert!(r.level_params.is_empty());
        let theorem = record(&original, "Profile.unbounded");
        let artifact = record(&derived.export, "Profile.unbounded.logos_statement");
        assert_eq!(
            artifact["def"]["value"], theorem["thm"]["type"],
            "{version}: the value is the theorem's own type record"
        );
        let ty = rows(&derived.export)
            .into_iter()
            .find(|row| row["ie"] == artifact["def"]["type"])
            .unwrap();
        assert_eq!(ty["sort"], json!(0), "{version}: its type is Prop");
        assert_eq!(artifact["def"]["safety"], "safe");
        assert_eq!(artifact["def"]["hints"], "abbrev");
        assert!(!declares(&derived.export, "Profile.unbounded"), "{version}");
        assert!(
            !declares(&derived.export, "sorryAx"),
            "{version}: no admission"
        );
        assert_eq!(
            derived.export.lines().next(),
            original.lines().next(),
            "{version}: the original's header"
        );
        let read = intake::read_admitted(&derived.export).unwrap();
        assert_eq!(read.profile().lean_version, version);
    }
}

/// A `Prop` definition's statement is its value, with the definitions it uses.
#[test]
fn a_prop_definitions_statement_is_its_value() {
    for version in VERSIONS {
        let original = fixture(version, "Profile.EveryDoubleIsEven");
        let derived = derive(&original, "Profile.EveryDoubleIsEven").unwrap();
        assert_eq!(derived.relation.origin, Origin::DefinitionValue);
        assert_eq!(derived.relation.original_kind, "def");
        let definition = record(&original, "Profile.EveryDoubleIsEven");
        let artifact = record(&derived.export, "Profile.EveryDoubleIsEven.logos_statement");
        assert_eq!(
            artifact["def"]["value"], definition["def"]["value"],
            "{version}"
        );
        for used in ["Profile.IsEven", "Profile.double"] {
            assert!(declares(&derived.export, used), "{version} keeps {used}");
        }
        assert!(
            !declares(&derived.export, "Profile.EveryDoubleIsEven"),
            "{version}: the artifact replaces, never duplicates, the definition"
        );
    }
}

/// Every retained record is the original's own, byte for byte, and the derived export holds the
/// selected closure exactly: everything it declares the artifact reaches, but the quotient
/// primitives Lean declares as one.
#[test]
fn the_derived_export_is_exactly_the_selected_closure() {
    let quotient: BTreeSet<String> = ["Quot", "Quot.mk", "Quot.lift", "Quot.ind"]
        .iter()
        .map(|n| intake::kernel_name(n).unwrap())
        .collect();
    for version in VERSIONS {
        for (stem, target) in [
            ("Profile.statements", "Profile.unbounded"),
            ("Profile.statements", "Profile.RestsOnUnfinished"),
            ("Profile.EveryDoubleIsEven", "Profile.EveryDoubleIsEven"),
            ("Profile.greetingIsHi", "Profile.greetingIsHi"),
        ] {
            let original = fixture(version, stem);
            let derived = derive(&original, target).unwrap();
            let lines: BTreeSet<&str> = original.lines().collect();
            let appended = derived
                .export
                .lines()
                .filter(|l| !lines.contains(l))
                .count();
            assert!(
                appended >= 2,
                "{version} {target}: the artifact's own records"
            );
            let read = intake::read_admitted(&derived.export).unwrap();
            let artifact = intake::kernel_name(&derived.relation.artifact).unwrap();
            let mut closure: BTreeSet<String> =
                read.closure(&artifact).unwrap().into_iter().collect();
            // A literal names no constant, yet its kernel reading needs the release's interfaces.
            let packing = profile::release(version).unwrap().string_literals;
            let interfaces = [
                "Nat",
                "Char",
                "Char.ofNat",
                "String",
                "List",
                "List.nil",
                "List.cons",
                &intake::lean_name(packing.packing_constant()),
            ];
            for literal in ["\"natVal\"", "\"strVal\""] {
                if derived.export.contains(literal) {
                    for interface in interfaces {
                        let kernel = intake::kernel_name(interface).unwrap();
                        if read.declaration(&kernel).is_some() {
                            closure.extend(read.closure(&kernel).unwrap());
                        }
                    }
                }
            }
            for declaration in read.declarations() {
                assert!(
                    closure.contains(&declaration.kernel_name)
                        || quotient.contains(&declaration.kernel_name),
                    "{version} {target}: {} is not reached by the statement",
                    declaration.lean_name
                );
            }
            assert_eq!(
                derived.relation.retained_declarations,
                rows(&derived.export)
                    .iter()
                    .filter(|r| r.get("in").is_none()
                        && r.get("il").is_none()
                        && r.get("ie").is_none()
                        && r.get("meta").is_none())
                    .count()
                    - 1,
                "{version} {target}: every retained record, and the artifact's"
            );
        }
    }
}

/// The derivation is a function of the original export and the target alone.
#[test]
fn the_derivation_is_deterministic() {
    for version in VERSIONS {
        let original = statements(version);
        for target in [
            "Profile.unbounded",
            "Profile.answered",
            "Profile.RestsOnUnfinished",
        ] {
            assert_eq!(
                derive(&original, target).unwrap(),
                derive(&original, target).unwrap(),
                "{target}"
            );
        }
    }
}

/// An `answer(sorry)` hole is a parameter of the artifact — an open template over the answer —
/// and its `sorryAx` is not part of the artifact; no answer is chosen.
#[test]
fn an_answer_hole_becomes_a_template_parameter() {
    for version in VERSIONS {
        let original = statements(version);
        let derived = derive(&original, "Profile.answered").unwrap();
        let holes = &derived.relation.answer_holes;
        assert_eq!(holes.len(), 1, "{version}");
        assert_eq!(holes[0].parameter, "answer");
        let marked = rows(&original)
            .into_iter()
            .find(|r| r["ie"] == json!(holes[0].expression))
            .unwrap();
        assert_eq!(
            marked["mdata"]["data"]["answer"], "Lean.DataValue.ofBool true",
            "{version}: the hole is the marked answer"
        );
        let all = rows(&derived.export);
        let by_id = |id: &Value| all.iter().find(|r| r["ie"] == *id).unwrap().clone();
        let artifact = record(&derived.export, "Profile.answered.logos_statement");
        let ty = by_id(&artifact["def"]["type"]);
        assert_eq!(ty["forallE"]["type"], json!(holes[0].type_expression));
        assert_eq!(by_id(&ty["forallE"]["body"])["sort"], json!(0));
        let hole_type = by_id(&json!(holes[0].type_expression));
        assert_eq!(
            hole_type["sort"],
            json!(0),
            "{version}: the answer is a Prop"
        );
        let value = by_id(&artifact["def"]["value"]);
        assert_eq!(value["lam"]["type"], json!(holes[0].type_expression));
        assert!(
            !declares(&derived.export, "sorryAx"),
            "{version}: the template holds no admission"
        );
        assert!(
            !derived
                .export
                .lines()
                .any(|l| l.contains(&format!("\"ie\":{},", holes[0].expression))),
            "{version}: the hole itself is not part of the template"
        );
    }
}

/// A statement resting on an unfinished proof keeps that proof — and its `sorryAx` — so the policy
/// can refuse it: an unfinished dependency is never dropped to obtain a clean statement.
#[test]
fn an_unfinished_dependency_is_kept_never_dropped() {
    for version in VERSIONS {
        let derived = derive(&statements(version), "Profile.RestsOnUnfinished").unwrap();
        assert!(declares(&derived.export, "Profile.unfinished"), "{version}");
        assert!(declares(&derived.export, "sorryAx"), "{version}");
    }
}

/// Only a theorem or a safe `Prop` definition states a proposition: any other declaration is a
/// typed refusal, never a guessed statement.
#[test]
fn a_declaration_that_states_no_proposition_is_refused() {
    let even = fixture("4.33.1", "Profile.IsEven");
    let (code, message) = refused(&even, "Profile.IsEven");
    assert_eq!(code, DiagnosticCode::UnsupportedFeature, "{message}");
    assert!(message.contains("type is not Prop"), "{message}");
    let greeting = fixture("4.33.1", "Profile.greetingIsHi");
    let (code, _) = refused(&greeting, "Profile.greeting");
    assert_eq!(code, DiagnosticCode::UnsupportedFeature);
    for (target, kind) in [
        ("Nat", "inductive"),
        ("sorryAx", "axiom"),
        ("Nat.succ", "ctor"),
    ] {
        let (code, message) = refused(&statements("4.33.1"), target);
        assert_eq!(
            code,
            DiagnosticCode::UnsupportedFeature,
            "{target}: {message}"
        );
        assert!(message.contains(&format!("is a Lean {kind}")), "{message}");
    }
    let (code, _) = refused(&statements("4.33.1"), "Profile.nowhere");
    assert_eq!(code, DiagnosticCode::MissingDeclaration);
}

/// A derivation is held to the registry like any export: an unregistered or retagged header is
/// refused before anything is derived.
#[test]
fn a_derivation_admits_only_a_registered_profile() {
    let original = statements("4.33.1");
    let (first, rest) = original.split_once('\n').unwrap();
    let mut header: Value = serde_json::from_str(first).unwrap();
    header["meta"]["lean"]["version"] = "4.29.1".into();
    let (code, _) = refused(&format!("{header}\n{rest}"), "Profile.unbounded");
    assert_eq!(code, DiagnosticCode::UnsupportedToolchain);
    header["meta"]["lean"]["version"] = "4.34.1".into();
    let (code, message) = refused(&format!("{header}\n{rest}"), "Profile.unbounded");
    assert_eq!(code, DiagnosticCode::UnsupportedToolchain, "{message}");
}

/// The artifact's name must be fresh in the export.
#[test]
fn an_artifact_name_the_export_already_declares_is_refused() {
    let original = statements("4.33.1");
    let mut all = rows(&original);
    let unbounded = name_index(&original, "Profile.unbounded");
    let next = all.iter().filter_map(|r| r["in"].as_u64()).max().unwrap() + 1;
    let ty = record(&original, "Profile.unbounded")["thm"]["type"].clone();
    all.push(json!({"str": {"pre": unbounded, "str": "logos_statement"}, "in": next}));
    all.push(json!({"axiom": {"name": next, "levelParams": [], "type": ty, "isUnsafe": false}}));
    let text = all
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    let (code, message) = refused(&text, "Profile.unbounded");
    assert_eq!(code, DiagnosticCode::UnsupportedFeature, "{message}");
    assert!(message.contains("already declares"), "{message}");
}

// ============================================================================================
// THE PACKAGE AND ITS REPLAY
// ============================================================================================

fn statement_package(version: &str, stem: &str, target: &str) -> PackageFiles {
    let original = fixture(version, stem);
    let derived = derive(&original, target).unwrap();
    build_statement_observed(&original, &derived, &BuildOptions::default(), None)
        .unwrap_or_else(|v| panic!("{version} {target}: {:?}", v.reason()))
        .files
}

fn manifest(files: &PackageFiles) -> Manifest {
    serde_json::from_slice(&files[MANIFEST]).unwrap()
}

/// Recompute every digest and the result identity, as an attacker who can recompute a hash would.
fn redigest(files: &mut PackageFiles, edit: impl FnOnce(&mut Manifest)) {
    let mut m = manifest(files);
    edit(&mut m);
    m.files = files
        .iter()
        .filter(|(p, _)| *p != MANIFEST)
        .map(|(p, b)| (p.clone(), package_sha(b)))
        .collect();
    m.source.export_sha256 = m.files[EXPORT].clone();
    if let Some(s) = &mut m.statement {
        if let Some(digest) = m.files.get(ORIGINAL) {
            s.original_sha256 = digest.clone();
        }
    }
    m.result_id = package::result_id_in(&m.schema, &m.target.lean, &m.files);
    files.insert(MANIFEST.into(), canonical_json(&m));
}

fn assert_replays(files: &PackageFiles, what: &str) {
    let receipt = replay(files);
    for stage in REPLAY_STAGES {
        assert_eq!(
            receipt.status(stage),
            StageStatus::Passed,
            "{what} {stage}: {:?}",
            receipt.stage(stage)
        );
    }
}

fn assert_refused_at(files: &PackageFiles, stage: &str, code: &str, what: &str) {
    let receipt = replay(files);
    let at = receipt.stage(stage).unwrap();
    assert_eq!(at.status, StageStatus::Failed, "{what}: {at:?}");
    assert_eq!(at.code.as_deref(), Some(code), "{what}: {at:?}");
    assert!(!receipt.replayed(), "{what}");
}

/// ★ A statement package replays clean from its bytes alone, under the statement schema, carrying
/// the original export, stating its relation, its proof status `not_claimed`, and disclosing the
/// artifact's construction.
#[test]
fn a_statement_package_replays_clean() {
    for version in ["4.14.0", "4.33.1"] {
        for (stem, target) in [
            ("Profile.statements", "Profile.unbounded"),
            ("Profile.EveryDoubleIsEven", "Profile.EveryDoubleIsEven"),
        ] {
            let files = statement_package(version, stem, target);
            assert_replays(&files, &format!("{version} {target}"));
            let m = manifest(&files);
            assert_eq!(m.schema, statement::PACKAGE_SCHEMA);
            assert_eq!(m.target.lean, format!("{target}.logos_statement"));
            assert_eq!(m.target.kind, "def");
            assert_eq!(m.target.statement, "Prop");
            let record = m.statement.as_ref().unwrap();
            assert_eq!(record.proof_status, "not_claimed");
            assert_eq!(record.original, ORIGINAL);
            assert_eq!(
                files[ORIGINAL],
                fixture(version, stem).into_bytes(),
                "the original export, exactly"
            );
            assert_eq!(record.relation.original, target);
            assert!(m.assumptions.transitive.is_empty(), "{:?}", m.assumptions);
            assert_eq!(
                m.information_losses
                    .iter()
                    .filter(|l| l.category == INFORMATION_CATEGORY)
                    .count(),
                1
            );
            let document = package::replay_document(&files, &replay(&files));
            assert_eq!(document["schema"], statement::REPLAY_SCHEMA);
            assert_eq!(document["purpose"], "statement");
            assert_eq!(document["proof_status"], "not_claimed");
            assert_eq!(document["statement"]["original"], target);
        }
    }
}

/// An answer template packages and replays, its hole disclosed.
#[test]
fn an_answer_template_package_replays_with_its_hole_disclosed() {
    for version in ["4.14.0", "4.33.1"] {
        let files = statement_package(version, "Profile.statements", "Profile.answered");
        assert_replays(&files, version);
        let m = manifest(&files);
        assert_eq!(
            m.information_losses
                .iter()
                .filter(|l| l.category == ANSWER_CATEGORY)
                .count(),
            1,
            "{version}"
        );
        assert_ne!(
            m.target.statement, "Prop",
            "{version}: a template, not a proposition"
        );
        assert!(
            m.target.statement.contains("Prop"),
            "{}",
            m.target.statement
        );
    }
}

/// ★ A wrong but well-typed proposition fails binding: a package whose artifact is named for one
/// declaration but states another's (checked) proposition — its certificate, derived export and
/// manifest all consistent with each other — is refused, because the original derives otherwise.
#[test]
fn a_wrong_but_well_typed_proposition_fails_binding() {
    for version in ["4.14.0", "4.33.1"] {
        let original = statements(version);
        let honest = derive(&original, "Profile.unbounded").unwrap();
        let other = derive(&original, "Profile.unfinished").unwrap();
        // `Profile.unfinished`'s statement (`double 1 = 2`), renamed as `Profile.unbounded`'s.
        let unbounded = name_index(&original, "Profile.unbounded");
        let unfinished = name_index(&original, "Profile.unfinished");
        let forged_export: String = other
            .export
            .lines()
            .map(|line| {
                let mut row: Value = serde_json::from_str(line).unwrap();
                if row["str"]["str"] == "logos_statement" && row["str"]["pre"] == unfinished {
                    row["str"]["pre"] = unbounded.into();
                    row.to_string() + "\n"
                } else {
                    format!("{line}\n")
                }
            })
            .collect();
        let forged = statement::Derived {
            export: forged_export,
            relation: honest.relation.clone(),
        };
        let files = build_statement_observed(&original, &forged, &BuildOptions::default(), None)
            .unwrap()
            .files;
        assert_eq!(
            manifest(&files).target.lean,
            "Profile.unbounded.logos_statement"
        );
        assert_refused_at(
            &files,
            "translated",
            "statement_derivation_mismatch",
            &format!("{version}: a forged proposition"),
        );
    }
}

/// Every other tamper an attacker who can recompute digests could make is refused, by the stage and
/// code that owns it.
#[test]
fn a_tampered_statement_package_is_refused() {
    let files = statement_package("4.33.1", "Profile.statements", "Profile.unbounded");
    // Another original export carried in its place.
    let mut swapped = files.clone();
    swapped.insert(
        ORIGINAL.into(),
        fixture("4.33.1", "Profile.EveryDoubleIsEven").into_bytes(),
    );
    redigest(&mut swapped, |_| {});
    assert_refused_at(
        &swapped,
        "translated",
        "statement_derivation_mismatch",
        "swapped original",
    );
    // The relation misstated.
    for (what, edit) in [
        (
            "origin",
            Box::new(|m: &mut Manifest| {
                m.statement.as_mut().unwrap().relation.origin = Origin::DefinitionValue
            }) as Box<dyn Fn(&mut Manifest)>,
        ),
        (
            "kind",
            Box::new(|m: &mut Manifest| {
                m.statement.as_mut().unwrap().relation.original_kind = "def".into()
            }),
        ),
        (
            "retained",
            Box::new(|m: &mut Manifest| {
                m.statement.as_mut().unwrap().relation.retained_declarations += 1
            }),
        ),
    ] {
        let mut tampered = files.clone();
        redigest(&mut tampered, edit);
        assert_refused_at(
            &tampered,
            "translated",
            "statement_derivation_mismatch",
            what,
        );
    }
    // The construction disclosure dropped.
    let mut undisclosed = files.clone();
    redigest(&mut undisclosed, |m| {
        m.information_losses
            .retain(|l| l.category != INFORMATION_CATEGORY)
    });
    assert_refused_at(
        &undisclosed,
        "translated",
        "disclosure_mismatch",
        "undisclosed",
    );
    // A proof claimed, a statement record under a proof schema, the original removed: not
    // admitted at all.
    let mut claimed = files.clone();
    redigest(&mut claimed, |m| {
        m.statement.as_mut().unwrap().proof_status = "claimed".into()
    });
    let mut proof_schema = files.clone();
    redigest(&mut proof_schema, |m| {
        m.schema = package::PACKAGE_SCHEMA.into()
    });
    let mut without = files.clone();
    without.remove(ORIGINAL);
    redigest(&mut without, |_| {});
    for (what, tampered) in [
        ("claimed", claimed),
        ("proof schema", proof_schema),
        ("no original", without),
    ] {
        let receipt = replay(&tampered);
        assert_eq!(
            receipt.status("fresh_replay"),
            StageStatus::Failed,
            "{what}: {:?}",
            receipt.stage("fresh_replay")
        );
        assert_eq!(receipt.status("translated"), StageStatus::NotRun, "{what}");
    }
}

// ============================================================================================
// THE RESULT
// ============================================================================================

/// The in-process statement pipeline over an export: every stage but Lean's acceptance passes, the
/// result is a `/2` document, and an unfinished dependency is a disallowed assumption whose path
/// names it.
#[test]
fn the_statement_pipeline_reports_the_contract() {
    let original = statements("4.33.1");
    let clean = check_statement(
        &original,
        "Profile.unbounded",
        &LeanEvidence::NotObserved,
        &Limits::default(),
    );
    for stage in [
        "translated",
        "primary_check",
        "independent_recheck",
        "assumption_policy",
        "emission_reparsed",
    ] {
        assert_eq!(
            clean.result.status(stage),
            StageStatus::Passed,
            "{stage}: {:?}",
            clean.result.diagnostics
        );
    }
    let json = clean.result.to_json();
    assert_eq!(json["schema"], statement::RESULT_SCHEMA);
    assert_eq!(json["purpose"], "statement");
    assert_eq!(json["proof_status"], "not_claimed");
    assert_eq!(json["statement"]["origin"], "theorem_type");
    assert_eq!(
        json["statement"]["artifact"],
        "Profile.unbounded.logos_statement"
    );
    assert!(json["target"]["logos"].is_string());

    let unfinished = check_statement(
        &original,
        "Profile.RestsOnUnfinished",
        &LeanEvidence::NotObserved,
        &Limits::default(),
    );
    assert_eq!(
        unfinished.result.status("assumption_policy"),
        StageStatus::Failed
    );
    let messages: Vec<&str> = unfinished
        .result
        .diagnostics
        .iter()
        .filter(|d| d.code == DiagnosticCode::DisallowedAssumption)
        .map(|d| d.message.as_str())
        .collect();
    assert!(
        messages
            .iter()
            .any(|m| m.contains("Profile.unfinished") && m.contains("sorryAx")),
        "the unfinished dependency is named on its path: {messages:?}"
    );
    let refused = check_statement(
        &original,
        "Nat",
        &LeanEvidence::NotObserved,
        &Limits::default(),
    );
    assert_eq!(refused.result.status("translated"), StageStatus::Failed);
    assert_eq!(refused.result.to_json()["schema"], statement::RESULT_SCHEMA);
    assert!(refused.result.to_json()["statement"].is_null());
}

/// `largo lean import --statement` over an export: a `/2` result, never verified (Lean was not
/// observed), every in-process stage passed.
#[test]
fn the_cli_imports_a_statement_from_an_export() {
    let dir = scratch();
    let run = export_import(
        &fixture_path("4.33.1", "Profile.statements"),
        "Profile.unbounded",
        &dir.path().join("out"),
        &["--statement"],
    );
    assert_eq!(
        run.result["schema"],
        statement::RESULT_SCHEMA,
        "{}",
        run.describe()
    );
    assert_eq!(run.result["proof_status"], "not_claimed");
    assert_eq!(run.outcome(), "unsupported_feature");
    assert_eq!(
        run.codes(),
        vec!["lean_acceptance_not_observed"],
        "{}",
        run.describe()
    );
    for stage in [
        "translated",
        "primary_check",
        "independent_recheck",
        "assumption_policy",
        "emission_reparsed",
    ] {
        assert_eq!(run.stage(stage), "passed", "{stage}\n{}", run.describe());
    }
    assert!(!dir.path().join("out").exists());
}

/// `largo lean import --project … --statement` under the profile `p`: the clean statement, the
/// `Prop` definition and the answer template are verified, published and replayed; the statement
/// resting on an unfinished proof is a disallowed assumption naming it; a definition that states no
/// proposition is refused; and the proof import of the same theorem still refuses its placeholder.
fn imports_statements_from_its_project(p: &profile::Profile) {
    let v = p.lean_version;
    let project = project(&format!("profile-{v}"));
    let tools = profile_toolchain(p).is_some() && profile_exporter(p).is_some();
    let import = |target: &str, out: &std::path::Path, extra: &[&str]| {
        let mut args = vec![
            "lean".to_string(),
            "import".into(),
            "--project".into(),
            project.display().to_string(),
            "--target".into(),
            target.into(),
            "--out".into(),
            out.display().to_string(),
        ];
        args.extend(extra.iter().map(|s| s.to_string()));
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let mut command = largo();
        command.args(&args).current_dir(repo_root());
        if let Some(registry) = exporter_registry() {
            command.env("LOGOS_LEAN_EXPORTERS", registry);
        }
        held(command.output().unwrap())
    };
    let dir = scratch();
    if !tools {
        require_absent_tools_allowed(p);
        let run = import("Profile.unbounded", &dir.path().join("x"), &["--statement"]);
        assert_eq!(
            run.outcome(),
            "infrastructure_failure",
            "{}",
            run.describe()
        );
        return;
    }
    for target in [
        "Profile.unbounded",
        "Profile.EveryDoubleIsEven",
        "Profile.answered",
    ] {
        let out = dir.path().join(target);
        let run = import(target, &out, &["--statement"]);
        assert_eq!(
            run.outcome(),
            "verified",
            "{v} {target}\n{}",
            run.describe()
        );
        assert_eq!(run.result["schema"], statement::RESULT_SCHEMA);
        assert_eq!(run.result["proof_status"], "not_claimed");
        let replayed = run_in(dir.path(), &["lean", "replay", out.to_str().unwrap()]);
        assert_eq!(replayed.exit, 0, "{v} {target}\n{}", replayed.describe());
        assert_eq!(replayed.result["schema"], statement::REPLAY_SCHEMA);
        assert_eq!(replayed.result["proof_status"], "not_claimed");
    }
    let unfinished = import(
        "Profile.RestsOnUnfinished",
        &dir.path().join("unfinished"),
        &["--statement"],
    );
    assert_eq!(
        unfinished.outcome(),
        "disallowed_assumptions",
        "{v}\n{}",
        unfinished.describe()
    );
    assert!(
        unfinished.result["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["message"]
                .as_str()
                .unwrap()
                .contains("Profile.unfinished")),
        "{v}\n{}",
        unfinished.describe()
    );
    let no_proposition = import("Profile.IsEven", &dir.path().join("even"), &["--statement"]);
    assert_eq!(
        no_proposition.outcome(),
        "unsupported_feature",
        "{v}\n{}",
        no_proposition.describe()
    );
    let proof = import("Profile.unbounded", &dir.path().join("proof"), &[]);
    assert_eq!(
        proof.outcome(),
        "disallowed_assumptions",
        "{v}\n{}",
        proof.describe()
    );
    assert_eq!(proof.result["schema"], "logos-lean-result/1");
}

/// ★ Stage A and a later profile, every run of the suite.
#[test]
fn stage_a_and_a_later_profile_import_statements_from_their_projects() {
    for version in ["4.14.0", "4.33.1"] {
        imports_statements_from_its_project(profile::by_version(version).unwrap());
    }
}

/// Every registered profile — minutes per profile in a debug build: it runs with the ignored set.
#[test]
#[ignore = "heavy: every profile's project, minutes each; run with the ignored set"]
fn every_profile_imports_statements_from_its_project() {
    for p in profile::profiles() {
        imports_statements_from_its_project(p);
    }
}
