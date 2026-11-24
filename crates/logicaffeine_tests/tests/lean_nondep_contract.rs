//! Track L3 — the elaborator-have translation contract for `letE` with `nondep: true`.
//!
//! Every release after 4.14.0 elaborates `have` as a nondependent let. Lean's kernel does not verify
//! the flag: the let is an ordinary let to it. LOGOS translates it exactly so — the checked terms of
//! an export are identical with the flag true and with it false, both kernels agree — and discloses
//! the flag at its expression as the `SOURCE_METADATA` loss `letE.nondep = true`, once per flagged
//! let and never otherwise. The disclosure reaches the result and the package, and a package's
//! replay recomputes it from the export it carries, so a package that drops or invents one fails.
//!
//! The real inputs are the per-profile fixtures (Lean's own `have`, `Profile.addZeroViaHave`, and the
//! nondependent lets of Lean core's `String` closure) and, where this box holds them, the five corpus
//! exports whose recorded package probes refused at their first nondependent let: OP175 at physical
//! line 4334, OP197 at 3797, OP198 at 3882, OP199 and OP200 at 3747 (`logs/alejandro200/REPORT.md`).

mod lean_common;

use lean_common::*;
use logicaffeine_kernel::interface::lean::metadata::NONDEP_KEY;
use logicaffeine_kernel::interface::lean::package::{
    self, build, canonical_json, replay, sha256_hex as package_sha, BuildOptions, Manifest,
    PackageFiles, EXPORT, MANIFEST, SOURCE_METADATA,
};
use logicaffeine_kernel::interface::lean::receipt::StageStatus;
use logicaffeine_kernel::interface::lean::{
    self as lean, DiagnosticCode, ImportRequest, RetainSource, Verdict,
};
use logicaffeine_kernel::Term;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

const LATER: [&str; 4] = ["4.28.0", "4.33.1", "4.34.0-rc1", "4.34.1"];

/// The export of `target` that `version`'s own toolchain and exporter produced.
fn profile_fixture(version: &str, target: &str) -> String {
    std::fs::read_to_string(profile_path(version, target)).unwrap()
}

fn profile_path(version: &str, target: &str) -> std::path::PathBuf {
    fixtures()
        .join("profiles")
        .join(version)
        .join(format!("{target}.ndjson"))
}

/// Lean's own tactic `have`: one nondependent let, in a small closure.
const HAVE: &str = "Profile.addZeroViaHave";
/// Lean core's `String` closure, whose library proofs carry nondependent lets of their own.
const STRING: &str = "Profile.greetingIsHi";

fn rows(text: &str) -> Vec<Value> {
    text.lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

fn text(rows: &[Value]) -> String {
    rows.iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}

/// The expression ids of every `letE` whose flag is `true`.
fn nondep_ids(text: &str) -> BTreeSet<usize> {
    rows(text)
        .iter()
        .filter(|r| r["letE"]["nondep"] == json!(true))
        .map(|r| r["ie"].as_u64().unwrap() as usize)
        .collect()
}

/// `text` with every nondependent let's flag set to `false`, nothing else changed.
fn flags_cleared(text: &str) -> String {
    let mut rows = rows(text);
    for r in &mut rows {
        if r["letE"]["nondep"] == json!(true) {
            r["letE"]["nondep"] = json!(false);
        }
    }
    self::text(&rows)
}

fn import_everything(export: &str) -> (lean::ImportedContext, lean::ImportReport) {
    lean::import_with(
        export,
        ImportRequest {
            audit: true,
            retain: RetainSource::EveryProof,
            lean_binder_names: true,
        },
    )
    .unwrap_or_else(|v| panic!("import: {:?}", v.reason()))
}

fn retained(ctx: &lean::ImportedContext) -> BTreeMap<String, (Term, Term)> {
    ctx.retained_declarations()
        .map(|(n, ty, body)| (n.to_string(), (ty.clone(), body.clone())))
        .collect()
}

fn nondep_losses(report: &lean::ImportReport) -> BTreeSet<usize> {
    report
        .metadata_losses
        .iter()
        .filter(|l| l.key == NONDEP_KEY)
        .map(|l| {
            assert_eq!(
                l.value, "true",
                "a nondep disclosure states the flag exactly"
            );
            l.expression
        })
        .collect()
}

// ============================================================================================
// THE CONTRACT ON REAL LEAN `have`
// ============================================================================================

/// The fixtures do exercise the contract: Lean's own tactic `have` and Lean core's `String` closure
/// are nondependent lets in every release after 4.14.0, and in none of 4.14.0's.
#[test]
fn every_later_release_writes_have_as_a_nondependent_let() {
    for target in [HAVE, STRING] {
        assert!(nondep_ids(&profile_fixture("4.14.0", target)).is_empty());
    }
    for version in LATER {
        assert_eq!(
            nondep_ids(&profile_fixture(version, HAVE)).len(),
            1,
            "{version}"
        );
        let library = nondep_ids(&profile_fixture(version, STRING)).len();
        assert!(library >= 2, "{version}: {library} nondependent lets");
    }
}

/// ★ A nondependent let is translated and checked exactly as the same let with its flag false: the
/// import succeeds, and every checked declaration — type and body — is identical between the two.
#[test]
fn a_nondependent_let_is_checked_exactly_as_an_ordinary_let() {
    for version in LATER {
        let export = profile_fixture(version, HAVE);
        let (with_flag, flagged_report) = import_everything(&export);
        let (without_flag, cleared_report) = import_everything(&flags_cleared(&export));
        let flagged = retained(&with_flag);
        let cleared = retained(&without_flag);
        assert!(
            flagged.contains_key(&intake_name("Profile.addZeroViaHave")),
            "{version}"
        );
        assert_eq!(
            flagged.keys().collect::<Vec<_>>(),
            cleared.keys().collect::<Vec<_>>(),
            "{version}: the same declarations"
        );
        for (name, terms) in &flagged {
            assert_eq!(
                terms, &cleared[name],
                "{version}: {name} is checked identically"
            );
        }
        let axioms = |r: &lean::ImportReport| {
            r.declarations
                .iter()
                .map(|d| (d.name.clone(), d.axioms.clone()))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            axioms(&flagged_report),
            axioms(&cleared_report),
            "{version}"
        );
    }
}

fn dag_verdict(verdict: &logicaffeine_kernel::dag::Verdict) -> String {
    use logicaffeine_kernel::dag::Verdict as Dag;
    match verdict {
        Dag::Accept => "accepted".into(),
        Dag::Reject(why) => format!("rejected: {why:?}"),
        Dag::Decline(why) => format!("declined: {why:?}"),
    }
}

fn intake_name(lean: &str) -> String {
    lean::intake::kernel_name(lean).unwrap()
}

/// Every flagged let is disclosed once, at its own expression, and nothing else is disclosed as a
/// nondependent let; the export's other metadata losses are untouched by the flag.
#[test]
fn every_nondependent_let_is_disclosed_exactly_once() {
    for (version, target) in LATER.iter().map(|v| (*v, HAVE)).chain([("4.33.1", STRING)]) {
        let export = profile_fixture(version, target);
        let (_, report) = import_everything(&export);
        let disclosed = nondep_losses(&report);
        assert_eq!(disclosed, nondep_ids(&export), "{version}");
        let count = report
            .metadata_losses
            .iter()
            .filter(|l| l.key == NONDEP_KEY)
            .count();
        assert_eq!(count, disclosed.len(), "{version}: once each");
        let (_, cleared) = import_everything(&flags_cleared(&export));
        assert!(nondep_losses(&cleared).is_empty(), "{version}");
        let others = |r: &lean::ImportReport| {
            r.metadata_losses
                .iter()
                .filter(|l| l.key != NONDEP_KEY)
                .map(|l| (l.expression, l.key.clone(), l.value.clone()))
                .collect::<Vec<_>>()
        };
        assert_eq!(others(&report), others(&cleared), "{version}");
        let json = report.to_json();
        let reported = json["losses"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|l| l["key"] == NONDEP_KEY)
            .count();
        assert_eq!(
            reported,
            disclosed.len(),
            "{version}: the audit carries them"
        );
    }
}

/// The kernel core reads a nondependent let as the same ordinary let: both kernels accept every
/// later release's export, with the flag and without it.
#[test]
fn the_kernel_core_agrees_on_nondependent_lets() {
    for (version, target) in LATER.iter().flat_map(|v| [(*v, HAVE), (*v, STRING)]) {
        let export = profile_fixture(version, target);
        for (what, input) in [
            ("flagged", export.clone()),
            ("cleared", flags_cleared(&export)),
        ] {
            let verdict = std::thread::Builder::new()
                .stack_size(1 << 30)
                .spawn(move || {
                    logicaffeine_kernel::dag::check_export(std::io::BufReader::new(
                        input.as_bytes(),
                    ))
                    .0
                })
                .unwrap()
                .join()
                .unwrap();
            assert!(
                matches!(verdict, logicaffeine_kernel::dag::Verdict::Accept),
                "{version} {what}: {}",
                dag_verdict(&verdict)
            );
        }
    }
}

/// A flag that is not a boolean is malformed, never read as either value.
#[test]
fn a_flag_that_is_not_a_boolean_is_a_malformed_export() {
    let export = profile_fixture("4.33.1", HAVE);
    let mut base = rows(&export);
    let at = base
        .iter()
        .position(|r| r["letE"]["nondep"] == json!(true))
        .unwrap();
    for flag in [
        json!("true"),
        json!(1),
        json!(null),
        json!([true]),
        json!({}),
    ] {
        base[at]["letE"]["nondep"] = flag.clone();
        let verdict = lean::import(&text(&base)).map(|_| ()).unwrap_err();
        assert!(
            matches!(verdict, Verdict::Decline(_)),
            "{flag}: {verdict:?}"
        );
        assert_eq!(
            verdict.reason().code(),
            DiagnosticCode::MalformedExport,
            "{flag}: {verdict:?}"
        );
        assert_eq!(verdict.reason().line(), Some(at + 1), "{flag}");
    }
}

/// The `letFun` metadata marker is still declined: the contract covers the flag, not an opacity
/// annotation no contract defines.
#[test]
fn a_let_fun_annotation_is_still_declined() {
    let export = profile_fixture("4.33.1", HAVE);
    let mut base = rows(&export);
    let at = base
        .iter()
        .position(|r| r["letE"]["nondep"] == json!(true))
        .unwrap();
    let id = base[at]["ie"].as_u64().unwrap();
    let next = base.iter().filter_map(|r| r["ie"].as_u64()).max().unwrap() + 1;
    // Annotate the flagged let, and point every use of it at the annotation.
    base.insert(
        at + 1,
        json!({"ie": next, "mdata": {"expr": id, "data": {"letFun": "Lean.DataValue.ofBool true"}}}),
    );
    let verdict = lean::import(&text(&base)).map(|_| ()).unwrap_err();
    assert_eq!(verdict.reason().code(), DiagnosticCode::UnsupportedFeature);
    assert!(verdict.reason().message().contains("letFun"), "{verdict:?}");
}

// ============================================================================================
// THE DISCLOSURE IN THE RESULT AND THE PACKAGE
// ============================================================================================

/// The result of an export-mode import discloses every flagged let of the export, exactly.
#[test]
fn the_result_discloses_every_nondependent_let() {
    for version in LATER {
        let dir = scratch();
        let path = profile_path(version, HAVE);
        let run = export_import(&path, HAVE, &dir.path().join("out"), &[]);
        for stage in ["translated", "primary_check", "independent_recheck"] {
            assert_eq!(
                run.stage(stage),
                "passed",
                "{version} {stage}\n{}",
                run.describe()
            );
        }
        let expected: BTreeSet<String> = nondep_ids(&profile_fixture(version, HAVE))
            .into_iter()
            .map(|id| format!("expression {id}: {NONDEP_KEY} = true"))
            .collect();
        let disclosed: BTreeSet<String> = run.result["information_losses"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|l| {
                l["category"] == SOURCE_METADATA
                    && l["detail"].as_str().unwrap().contains(NONDEP_KEY)
            })
            .map(|l| l["detail"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(disclosed, expected, "{version}\n{}", run.describe());
    }
}

fn manifest(files: &PackageFiles) -> Manifest {
    serde_json::from_slice(&files[MANIFEST]).unwrap()
}

/// Rewrite the manifest and recompute every digest and the result identity, as an attacker who can
/// recompute a hash would.
fn with_manifest(files: &PackageFiles, edit: impl FnOnce(&mut Manifest)) -> PackageFiles {
    let mut files = files.clone();
    let mut m = manifest(&files);
    edit(&mut m);
    m.files = files
        .iter()
        .filter(|(p, _)| *p != MANIFEST)
        .map(|(p, b)| (p.clone(), package_sha(b)))
        .collect();
    m.source.export_sha256 = m.files[EXPORT].clone();
    m.result_id = package::result_id_in(&m.schema, &m.target.lean, &m.files);
    files.insert(MANIFEST.into(), canonical_json(&m));
    files
}

fn built(export: &str, target: &str) -> PackageFiles {
    build(export, target, &BuildOptions::default())
        .unwrap_or_else(|v| panic!("build {target}: {:?}", v.reason()))
        .files
}

/// ★ A package of a proof through a `have` replays clean, and its manifest discloses every flagged
/// let of its export.
#[test]
fn a_package_of_a_have_proof_replays_with_its_disclosures() {
    for version in LATER {
        let export = profile_fixture(version, HAVE);
        let files = built(&export, HAVE);
        let receipt = replay(&files);
        for stage in [
            "translated",
            "primary_check",
            "independent_recheck",
            "assumption_policy",
            "emission_reparsed",
        ] {
            assert_eq!(
                receipt.status(stage),
                StageStatus::Passed,
                "{version} {stage}: {:?}",
                receipt.stage(stage)
            );
        }
        let stated: BTreeSet<String> = manifest(&files)
            .information_losses
            .iter()
            .filter(|l| l.category == SOURCE_METADATA && l.detail.contains(NONDEP_KEY))
            .map(|l| l.detail.clone())
            .collect();
        assert_eq!(stated.len(), nondep_ids(&export).len(), "{version}");
    }
}

/// A package whose manifest drops a nondependent-let disclosure, invents one, or alters one is
/// refused at `translated` with `disclosure_mismatch`, re-digested so the refusal is the binding's.
#[test]
fn a_package_that_misstates_a_nondependent_let_is_refused() {
    let export = profile_fixture("4.33.1", HAVE);
    let files = built(&export, HAVE);
    let first = manifest(&files)
        .information_losses
        .iter()
        .position(|l| l.detail.contains(NONDEP_KEY))
        .unwrap();
    let dropped = with_manifest(&files, |m| {
        m.information_losses.remove(first);
    });
    let invented = with_manifest(&files, |m| {
        m.information_losses.push(package::InformationLoss {
            category: SOURCE_METADATA.into(),
            detail: format!("expression 0: {NONDEP_KEY} = true"),
        });
    });
    let altered = with_manifest(&files, |m| {
        m.information_losses[first].detail = m.information_losses[first]
            .detail
            .replace("= true", "= false");
    });
    let recategorized = with_manifest(&files, |m| {
        m.information_losses[first].category = "SOURCE_POSITIONS".into();
    });
    for (what, tampered) in [
        ("dropped", dropped),
        ("invented", invented),
        ("altered", altered),
        ("recategorized", recategorized),
    ] {
        let receipt = replay(&tampered);
        let stage = receipt.stage("translated").unwrap();
        assert_eq!(stage.status, StageStatus::Failed, "{what}: {stage:?}");
        assert_eq!(
            stage.code.as_deref(),
            Some("disclosure_mismatch"),
            "{what}: {stage:?}"
        );
        assert!(!receipt.replayed(), "{what}");
    }
}

// ============================================================================================
// THE FIVE RECORDED CORPUS REFUSALS
// ============================================================================================

/// `(export path under logs/alejandro200, SHA-256, physical line of the first nondependent let,
/// nondependent lets)` of the five exports whose package probes refused at the old policy.
const RECORDED: [(&str, &str, usize, usize); 5] = [
    (
        "op175-export-cgroup-r2/OP175.ndjson",
        "ff67de6e32dc7fbfb3f9330fb2f6a7abe63efda7c2cbd74752ade100488439ee",
        4334,
        9,
    ),
    (
        "op197-export-cgroup/OP197.ndjson",
        "d42d2afd0a64a98ac04c7b430024dc41186cf1f92bf468b721adec81beb1975a",
        3797,
        265,
    ),
    (
        "op198-export-cgroup/OP198.ndjson",
        "f28b4a7d373aee9fa95a8519b3d2be1f05c946f337fffee9647211065984d29f",
        3882,
        265,
    ),
    (
        "op199-export-cgroup/OP199.ndjson",
        "59836a1ecbc95d6fb44906a8e0611373a4ec8728723fd9ebaa0efd9191634357",
        3747,
        161,
    ),
    (
        "op200-export-cgroup/OP200.ndjson",
        "30cfec5a8d7bf3c5230e92319c1257548314b355a6d7caa2de144c56cca98911",
        3747,
        148,
    ),
];

/// The recorded corpus export, when this box holds it with its recorded digest. Absent, the test
/// says so; `LOGOS_REQUIRE_CORPUS=1` makes its absence a failure.
fn recorded(path: &str, sha256: &str) -> Option<String> {
    let full = std::env::var_os("LOGOS_ALEJANDRO200")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| repo_root().join("logs/alejandro200"))
        .join(path);
    let Ok(bytes) = std::fs::read(&full) else {
        assert!(
            std::env::var("LOGOS_REQUIRE_CORPUS").as_deref() != Ok("1"),
            "LOGOS_REQUIRE_CORPUS demands {}",
            full.display()
        );
        eprintln!("LOGOS_CORPUS_SKIP: {} is not on this box", full.display());
        return None;
    };
    assert_eq!(
        sha256_hex(&bytes),
        sha256,
        "{} is the recorded export",
        full.display()
    );
    Some(String::from_utf8(bytes).unwrap())
}

/// Each recorded refusal is where the report recorded it: the first nondependent let at its
/// physical line, with its recorded number of flagged lets.
#[test]
fn the_recorded_refusals_are_at_their_recorded_lines() {
    for (path, sha256, line, count) in RECORDED {
        let Some(export) = recorded(path, sha256) else {
            continue;
        };
        let first = export
            .lines()
            .position(|l| l.contains("\"nondep\":true"))
            .unwrap()
            + 1;
        assert_eq!(first, line, "{path}");
        assert_eq!(nondep_ids(&export).len(), count, "{path}");
    }
}

/// ★ The five exports pass the nondependent-let stage: the LOGOS reader no longer refuses at the
/// line the old policy refused, nor at any flagged let. An export it imports whole discloses every
/// flagged let exactly once; one it still refuses is refused later, for another reason, which the
/// statement ledger records. The kernel core accepts every one. Minutes per export in a debug build:
/// it runs with the ignored set.
#[test]
#[ignore = "heavy: minutes per Mathlib-scale export; run with the ignored set"]
fn the_recorded_refusals_pass_the_nondependent_let_stage() {
    for (path, sha256, first, count) in RECORDED {
        let Some(export) = recorded(path, sha256) else {
            continue;
        };
        let flagged: BTreeSet<usize> = export
            .lines()
            .enumerate()
            .filter(|(_, l)| l.contains("\"nondep\":true"))
            .map(|(at, _)| at + 1)
            .collect();
        match lean::import_with(&export, ImportRequest::default()) {
            Ok((_, report)) => {
                assert_eq!(nondep_losses(&report), nondep_ids(&export), "{path}");
                assert_eq!(nondep_losses(&report).len(), count, "{path}");
            }
            Err(verdict) => {
                let reason = verdict.reason();
                assert!(
                    !reason.message().contains("nondep"),
                    "{path}: still refused at a nondependent let: {reason:?}"
                );
                let line = reason.line().expect("a located refusal");
                assert!(
                    line > first,
                    "{path}: refused at line {line}, before its first flagged let"
                );
                assert!(
                    !flagged.contains(&line),
                    "{path}: refused at a flagged let ({line})"
                );
            }
        }
        let verdict = std::thread::Builder::new()
            .stack_size(1 << 30)
            .spawn(move || {
                logicaffeine_kernel::dag::check_export(std::io::BufReader::new(export.as_bytes())).0
            })
            .unwrap()
            .join()
            .unwrap();
        assert!(
            matches!(verdict, logicaffeine_kernel::dag::Verdict::Accept),
            "{path}: {}",
            dag_verdict(&verdict)
        );
    }
}
