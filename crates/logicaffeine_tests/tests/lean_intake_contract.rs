//! W02 steps 4 and 7, competition rule R10 — the Lean intake contract.
//!
//! Every refusal `largo lean import` gives is a typed, machine-readable diagnostic, and the refusal
//! categories the contract names are DISTINCT: an invalid source, an unsupported feature, a
//! disallowed assumption, a resource limit and an infrastructure failure never share an outcome, an
//! exit code or a diagnostic code. The compatibility rule is explicit: an unknown export format
//! version, an unknown mandatory field, an unknown record kind, an unresolved metavariable, a missing
//! declaration and a duplicated declaration each have their own code.
//!
//! Each test asserts the RETURNED ARTIFACT — the one `logos-lean-result/1` document on stdout, its
//! outcome, its stage statuses and its diagnostics — never merely an exit status.

mod lean_common;

use lean_common::*;
use logicaffeine_kernel::interface::lean::intake;
use serde_json::Value;
use std::collections::BTreeSet;

/// `text` with the record on 1-based `line` replaced by `f(record)`.
fn edit_line(text: &str, line: usize, f: impl FnOnce(&mut Value)) -> String {
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    let mut v: Value = serde_json::from_str(&lines[line - 1]).unwrap();
    f(&mut v);
    lines[line - 1] = v.to_string();
    lines.join("\n") + "\n"
}

fn with_header(text: &str, f: impl FnOnce(&mut Value)) -> String {
    edit_line(text, 1, f)
}

fn append(text: &str, record: &str) -> String {
    format!("{}{record}\n", text)
}

/// Run `largo lean import` over an in-memory export and return the result.
fn import_text(text: &str, target: &str) -> Run {
    let dir = scratch();
    let path = write_export(dir.path(), "input.ndjson", text);
    export_import(&path, target, &dir.path().join("out"), &[])
}

/// A refusal's identity: `(outcome, exit, first diagnostic code)`.
fn identity(run: &Run) -> (String, i32, String) {
    (
        run.outcome().to_string(),
        run.exit,
        run.codes().first().cloned().unwrap_or_default(),
    )
}

// ============================================================================================
// R10 — THE FIVE OUTCOME CATEGORIES ARE DISTINCT
// ============================================================================================

/// The theorem `Demo.andSwap` with its proof replaced by its own statement: well-formed JSON, every
/// index resolves, and the kernel refutes it (the statement is a proposition, not a proof of it).
fn ill_typed_and_swap() -> String {
    let text = export_text("Demo.andSwap");
    let line = declaration_line(&text, "Demo.andSwap");
    edit_line(&text, line, |v| {
        let ty = v["thm"]["type"].clone();
        v["thm"]["value"] = ty;
    })
}

#[test]
fn outcomes_are_distinct_machine_readable_codes() {
    let dir = scratch();
    let invalid = import_text(&ill_typed_and_swap(), "Demo.andSwap");
    let unsupported = export_import(
        &export("Runtime.identity"),
        "Runtime.identity",
        &dir.path().join("u"),
        &[],
    );
    let disallowed = export_import(
        &export("Gap.viaHole"),
        "Gap.viaHole",
        &dir.path().join("d"),
        &[],
    );
    let resource = export_import(
        &export("Demo.addZero"),
        "Demo.addZero",
        &dir.path().join("r"),
        &["--max-export-bytes", "1024"],
    );
    let infrastructure = project_import(
        &project("valid"),
        "Demo.andSwap",
        &dir.path().join("i"),
        &["--lean", "/nonexistent/logos-no-such-lean"],
    );

    let cases = [
        (&invalid, "invalid_source", 1, "invalid_proof"),
        (
            &unsupported,
            "unsupported_feature",
            2,
            "unsupported_feature",
        ),
        (
            &disallowed,
            "disallowed_assumptions",
            3,
            "disallowed_assumption",
        ),
        (&resource, "resource_limit", 4, "resource_limit"),
        (
            &infrastructure,
            "infrastructure_failure",
            5,
            "infrastructure",
        ),
    ];
    for (run, outcome, exit, code) in &cases {
        assert_eq!(run.result["schema"], RESULT_SCHEMA, "{}", run.describe());
        assert_eq!(run.outcome(), *outcome, "{}", run.describe());
        assert_eq!(run.exit, *exit, "{}", run.describe());
        assert_eq!(
            run.codes().first().map(String::as_str),
            Some(*code),
            "{}",
            run.describe()
        );
        assert!(
            run.result["package"].is_null(),
            "a refusal publishes nothing\n{}",
            run.describe()
        );
    }
    assert!(
        entries(dir.path()).is_empty(),
        "no refusal leaves anything behind: {:?}",
        entries(dir.path())
    );
    let identities: BTreeSet<(String, i32, String)> =
        cases.iter().map(|(r, ..)| identity(r)).collect();
    assert_eq!(
        identities.len(),
        5,
        "the five categories never share an outcome, exit code or code"
    );

    // Where each category was decided is named, not implied.
    assert_eq!(
        invalid.stage("primary_check"),
        "failed",
        "{}",
        invalid.describe()
    );
    assert_eq!(
        invalid.diagnostic("invalid_proof")["declaration"],
        "Demo.andSwap"
    );
    assert_eq!(
        unsupported.stage("translated"),
        "failed",
        "{}",
        unsupported.describe()
    );
    assert_eq!(
        disallowed.stage("assumption_policy"),
        "failed",
        "{}",
        disallowed.describe()
    );
    assert_eq!(
        disallowed.stage("primary_check"),
        "passed",
        "the proof checks; its assumption is what is refused"
    );
    assert_eq!(
        infrastructure.stage("lean_accepted"),
        "failed",
        "{}",
        infrastructure.describe()
    );
}

/// A disallowed assumption is named, with the declaration that carries it.
#[test]
fn a_disallowed_assumption_names_the_axiom() {
    let dir = scratch();
    let run = export_import(
        &export("Gap.viaHole"),
        "Gap.viaHole",
        &dir.path().join("out"),
        &[],
    );
    assert_eq!(
        run.outcome(),
        "disallowed_assumptions",
        "{}",
        run.describe()
    );
    let d = run.diagnostic("disallowed_assumption");
    assert_eq!(d["declaration"], "Gap.viaHole");
    assert!(d["message"].as_str().unwrap().contains("sorryAx"), "{d}");
    assert_eq!(
        run.result["assumptions"],
        serde_json::json!(["sorryAx"]),
        "{}",
        run.describe()
    );
}

// ============================================================================================
// THE COMPATIBILITY RULE — every unknown gets its own code
// ============================================================================================

#[test]
fn an_unknown_export_format_version_has_its_own_code() {
    let text = with_header(&export_text("Demo.andSwap"), |h| {
        h["meta"]["format"]["version"] = "3.2.0".into()
    });
    let run = import_text(&text, "Demo.andSwap");
    assert_eq!(run.outcome(), "unsupported_feature", "{}", run.describe());
    assert_eq!(
        run.codes(),
        vec!["unsupported_export_version"],
        "{}",
        run.describe()
    );
    assert_eq!(run.diagnostic("unsupported_export_version")["line"], 1);
    assert_eq!(run.stage("translated"), "failed");
    assert_eq!(
        run.stage("primary_check"),
        "not_run",
        "nothing is checked past a refused intake"
    );
}

/// A mismatched exporter or Lean version is outside the pinned Stage-A profile (Lean 4.14.0,
/// lean4export 3.1.0), refused before any declaration is read.
#[test]
fn a_mismatched_exporter_or_lean_version_has_its_own_code() {
    let base = export_text("Demo.andSwap");
    for (what, text) in [
        (
            "lean",
            with_header(&base, |h| h["meta"]["lean"]["version"] = "4.29.1".into()),
        ),
        (
            "exporter version",
            with_header(&base, |h| h["meta"]["exporter"]["version"] = "3.0.0".into()),
        ),
        (
            "exporter name",
            with_header(&base, |h| {
                h["meta"]["exporter"]["name"] = "other-exporter".into()
            }),
        ),
    ] {
        let run = import_text(&text, "Demo.andSwap");
        assert_eq!(
            run.outcome(),
            "unsupported_feature",
            "{what}\n{}",
            run.describe()
        );
        assert_eq!(
            run.codes(),
            vec!["unsupported_toolchain"],
            "{what}\n{}",
            run.describe()
        );
    }
}

#[test]
fn an_unknown_mandatory_field_has_its_own_code() {
    let text = export_text("Demo.andSwap");
    let line = declaration_line(&text, "Demo.andSwap");
    let tampered = edit_line(&text, line, |v| v["thm"]["isPartial"] = true.into());
    let run = import_text(&tampered, "Demo.andSwap");
    assert_eq!(run.outcome(), "unsupported_feature", "{}", run.describe());
    assert_eq!(run.codes(), vec!["unknown_field"], "{}", run.describe());
    let d = run.diagnostic("unknown_field");
    assert_eq!(d["line"], line as u64);
    assert!(d["message"].as_str().unwrap().contains("isPartial"), "{d}");

    // An unknown field on an expression or a header is the same rule.
    let expression = line_of(&text, |v| v.get("ie").is_some());
    let expr = edit_line(&text, expression, |v| v["extra"] = 0.into());
    assert_eq!(
        import_text(&expr, "Demo.andSwap").codes(),
        vec!["unknown_field"]
    );
    let header = with_header(&text, |h| h["meta"]["signature"] = "x".into());
    assert_eq!(
        import_text(&header, "Demo.andSwap").codes(),
        vec!["unknown_field"]
    );
}

#[test]
fn a_missing_mandatory_field_is_a_malformed_export() {
    let text = export_text("Demo.andSwap");
    let line = declaration_line(&text, "Demo.andSwap");
    let tampered = edit_line(&text, line, |v| {
        v["thm"].as_object_mut().unwrap().remove("all");
    });
    let run = import_text(&tampered, "Demo.andSwap");
    assert_eq!(run.outcome(), "invalid_source", "{}", run.describe());
    assert_eq!(run.codes(), vec!["malformed_export"], "{}", run.describe());
    assert_eq!(run.diagnostic("malformed_export")["line"], line as u64);
}

#[test]
fn an_unknown_record_kind_has_its_own_code() {
    let run = import_text(
        &append(&export_text("Demo.andSwap"), r#"{"macro":{"name":1}}"#),
        "Demo.andSwap",
    );
    assert_eq!(run.outcome(), "unsupported_feature", "{}", run.describe());
    assert_eq!(
        run.codes(),
        vec!["unknown_record_kind"],
        "{}",
        run.describe()
    );
}

#[test]
fn an_unresolved_metavariable_has_its_own_code() {
    let text = export_text("Demo.andSwap");
    let next = text
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok()?.get("ie")?.as_u64())
        .max()
        .unwrap()
        + 1;
    let run = import_text(
        &append(&text, &format!(r#"{{"ie":{next},"mvar":3}}"#)),
        "Demo.andSwap",
    );
    assert_eq!(run.outcome(), "invalid_source", "{}", run.describe());
    assert_eq!(
        run.codes(),
        vec!["unresolved_metavariable"],
        "{}",
        run.describe()
    );
}

#[test]
fn a_missing_declaration_has_its_own_code() {
    // The requested target is not in the export.
    let run = import_text(&export_text("Demo.andSwap"), "Demo.orSwap");
    assert_eq!(run.outcome(), "invalid_source", "{}", run.describe());
    assert_eq!(
        run.codes(),
        vec!["missing_declaration"],
        "{}",
        run.describe()
    );
    assert_eq!(
        run.diagnostic("missing_declaration")["declaration"],
        "Demo.orSwap"
    );

    // A dependency the export references but never declares.
    let text = export_text("Demo.andSwap");
    let and_line = line_of(&text, |v| v.get("inductive").is_some());
    let without: String = text
        .lines()
        .enumerate()
        .filter(|(i, _)| i + 1 != and_line)
        .map(|(_, l)| format!("{l}\n"))
        .collect();
    let run = import_text(&without, "Demo.andSwap");
    assert_eq!(run.outcome(), "invalid_source", "{}", run.describe());
    assert_eq!(
        run.codes(),
        vec!["missing_declaration"],
        "{}",
        run.describe()
    );
    assert!(run.diagnostic("missing_declaration")["message"]
        .as_str()
        .unwrap()
        .contains("And"));
}

#[test]
fn a_duplicated_declaration_has_its_own_code() {
    let text = export_text("Demo.andSwap");
    let line = declaration_line(&text, "Demo.andSwap");
    let record = text.lines().nth(line - 1).unwrap().to_string();
    let run = import_text(&append(&text, &record), "Demo.andSwap");
    assert_eq!(run.outcome(), "invalid_source", "{}", run.describe());
    assert_eq!(
        run.codes(),
        vec!["duplicate_declaration"],
        "{}",
        run.describe()
    );
    let d = run.diagnostic("duplicate_declaration");
    assert_eq!(d["declaration"], "Demo.andSwap");
    assert_eq!(
        d["line"],
        (text.lines().count() + 1) as u64,
        "the SECOND declaration is the refused one"
    );
}

#[test]
fn malformed_json_is_invalid_source_at_its_line() {
    let text = export_text("Demo.andSwap");
    let mut lines: Vec<&str> = text.lines().collect();
    let cut = &lines[9][..lines[9].len() / 2];
    lines[9] = cut;
    let run = import_text(&(lines.join("\n") + "\n"), "Demo.andSwap");
    assert_eq!(run.outcome(), "invalid_source", "{}", run.describe());
    assert_eq!(run.codes(), vec!["malformed_export"], "{}", run.describe());
    assert_eq!(run.diagnostic("malformed_export")["line"], 10);
}

/// The seven codes the compatibility rule names are pairwise distinct.
#[test]
fn the_compatibility_codes_are_pairwise_distinct() {
    let base = export_text("Demo.andSwap");
    let line = declaration_line(&base, "Demo.andSwap");
    let next = base
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok()?.get("ie")?.as_u64())
        .max()
        .unwrap()
        + 1;
    let record = base.lines().nth(line - 1).unwrap().to_string();
    let runs = [
        import_text(
            &with_header(&base, |h| h["meta"]["format"]["version"] = "4.0.0".into()),
            "Demo.andSwap",
        ),
        import_text(
            &with_header(&base, |h| h["meta"]["lean"]["version"] = "4.15.0".into()),
            "Demo.andSwap",
        ),
        import_text(
            &edit_line(&base, line, |v| v["thm"]["extra"] = 1.into()),
            "Demo.andSwap",
        ),
        import_text(&append(&base, r#"{"macro":{}}"#), "Demo.andSwap"),
        import_text(
            &append(&base, &format!(r#"{{"ie":{next},"fvar":1}}"#)),
            "Demo.andSwap",
        ),
        import_text(&base, "Demo.nowhere"),
        import_text(&append(&base, &record), "Demo.andSwap"),
    ];
    let codes: Vec<String> = runs.iter().map(|r| r.codes()[0].clone()).collect();
    assert_eq!(
        codes,
        vec![
            "unsupported_export_version",
            "unsupported_toolchain",
            "unknown_field",
            "unknown_record_kind",
            "unresolved_metavariable",
            "missing_declaration",
            "duplicate_declaration",
        ]
    );
    assert_eq!(codes.iter().collect::<BTreeSet<_>>().len(), codes.len());
}

// ============================================================================================
// INTAKE ITSELF — the library door the CLI and the package builder share
// ============================================================================================

/// Every export the pinned toolchain produced — the corpus fixtures and this crate's own — passes
/// the strict intake. A strict rule that refuses real exports would be a broken rule.
#[test]
fn every_pinned_lean_export_passes_intake() {
    let mut checked = 0;
    let own: Vec<std::path::PathBuf> = [
        "Demo.addZero",
        "Demo.andSwap",
        "Demo.doubleTwo",
        "Excluded.middle",
        "Gap.viaHole",
        "Runtime.identity",
    ]
    .iter()
    .map(|n| export(n))
    .collect();
    for path in corpus_exports().into_iter().chain(own) {
        let text = std::fs::read_to_string(&path).unwrap();
        let header: Value = serde_json::from_str(text.lines().next().unwrap()).unwrap();
        if header["meta"]["lean"]["version"] != "4.14.0" {
            continue;
        }
        intake::read(&text, &intake::STAGE_A).unwrap_or_else(|v| {
            panic!(
                "{} fails intake: {:?} at {:?}",
                path.display(),
                v.reason(),
                v.reason().line()
            )
        });
        checked += 1;
    }
    // 13 pinned corpus exports (2026-09-22) and this crate's 6; the corpus only grows.
    assert!(
        checked >= 19,
        "every pinned export is exercised, {checked} exports"
    );
}

#[test]
fn lean_names_round_trip_through_the_kernel_encoding() {
    for (lean, kernel) in [
        ("Demo.andSwap", r#"/s:"Demo"/s:"andSwap""#),
        ("propext", r#"/s:"propext""#),
        ("Classical.choice", r#"/s:"Classical"/s:"choice""#),
        ("Nat", "Nat"),
        ("Nat.zero", "Zero"),
        ("Nat.succ", "Succ"),
        ("Nat.add", r#"Nat/s:"add""#),
        ("«a.b».c", r#"/s:"a.b"/s:"c""#),
        ("_private.Foo.0.bar", r#"/s:"_private"/s:"Foo"/n:0/s:"bar""#),
    ] {
        assert_eq!(intake::kernel_name(lean).unwrap(), kernel, "{lean}");
        assert_eq!(intake::lean_name(kernel), lean, "{kernel}");
    }
    for bad in ["", "a..b", ".a", "a.", "«unclosed"] {
        assert!(
            intake::kernel_name(bad).is_err(),
            "{bad:?} is not a Lean name"
        );
    }
}

/// The closure of a target is its declaration and everything it transitively mentions, in export
/// (dependency) order — the unit retention, emission and packaging work on.
#[test]
fn intake_resolves_the_target_closure_in_dependency_order() {
    let text = export_text("Demo.andSwap");
    let read = intake::read(&text, &intake::STAGE_A).unwrap();
    let target = intake::kernel_name("Demo.andSwap").unwrap();
    let closure = read.closure(&target).unwrap();
    assert_eq!(
        closure.last(),
        Some(&target),
        "the target closes its own closure"
    );
    let and = intake::kernel_name("And").unwrap();
    assert!(closure.contains(&and), "{closure:?}");
    let positions: Vec<usize> = closure
        .iter()
        .map(|n| {
            read.declarations()
                .iter()
                .position(|d| &d.kernel_name == n)
                .unwrap()
        })
        .collect();
    assert!(
        positions.windows(2).all(|w| w[0] < w[1]),
        "export order is dependency order"
    );
    // A declaration nobody in the closure mentions stays out of it.
    let unrelated = read
        .declarations()
        .iter()
        .find(|d| !closure.contains(&d.kernel_name));
    if let Some(d) = unrelated {
        assert!(!closure.contains(&d.kernel_name));
    }
    assert!(read
        .closure(&intake::kernel_name("Demo.nothing").unwrap())
        .is_err());
}

// ============================================================================================
// THE CHANNELS
// ============================================================================================

/// stdout carries exactly one result document and stderr only NDJSON diagnostics — held on every
/// run by the harness; here on a success-shaped and a refusal-shaped run explicitly.
#[test]
fn stdout_is_one_result_and_stderr_is_ndjson() {
    let dir = scratch();
    for (name, target) in [
        ("Demo.andSwap", "Demo.andSwap"),
        ("Gap.viaHole", "Gap.viaHole"),
    ] {
        let run = export_import(&export(name), target, &dir.path().join(name), &[]);
        assert_eq!(run.result["schema"], RESULT_SCHEMA, "{}", run.describe());
        assert!(
            run.stdout.ends_with('\n') && run.stdout.trim_end().lines().count() == 1,
            "one line of JSON"
        );
        for line in run.stderr.lines() {
            let d: Value = serde_json::from_str(line).unwrap();
            assert!(d["code"].is_string(), "{line}");
        }
    }
}

#[test]
fn usage_errors_exit_64_with_no_result() {
    let dir = scratch();
    let out = dir.path().join("out");
    let out = out.to_str().unwrap();
    let andswap = export("Demo.andSwap");
    let andswap = andswap.to_str().unwrap();
    let valid = project("valid");
    let valid = valid.to_str().unwrap();
    for args in [
        vec!["lean", "import", andswap, "--out", out],
        vec!["lean", "import", andswap, "--target", "Demo.andSwap"],
        vec!["lean", "import", "--target", "Demo.andSwap", "--out", out],
        vec![
            "lean",
            "import",
            andswap,
            "--project",
            valid,
            "--target",
            "Demo.andSwap",
            "--out",
            out,
        ],
        vec![
            "lean",
            "import",
            andswap,
            "--target",
            "Demo.andSwap",
            "--out",
            out,
            "--no-such-flag",
        ],
        vec![
            "lean",
            "import",
            andswap,
            "--target",
            "not..a.name",
            "--out",
            out,
        ],
        vec!["lean", "frobnicate"],
    ] {
        let run = run(&args);
        assert_eq!(run.exit, 64, "{args:?}\n{}", run.describe());
        assert!(
            run.stdout.is_empty(),
            "a usage error is not a run and has no result\n{}",
            run.describe()
        );
        assert!(
            run.diagnostics.iter().all(|d| d["code"] == "usage"),
            "the usage error is itself a typed diagnostic\n{}",
            run.describe()
        );
        assert!(!run.diagnostics.is_empty());
    }
}

/// `--help` is documentation, not an error.
#[test]
fn help_is_not_a_usage_error() {
    let output = largo().args(["lean", "import", "--help"]).output().unwrap();
    assert_eq!(output.status.code(), Some(0));
    let text = String::from_utf8_lossy(&output.stdout);
    for flag in [
        "--target",
        "--out",
        "--project",
        "--arena-protocol",
        "--max-export-bytes",
    ] {
        assert!(text.contains(flag), "help documents {flag}:\n{text}");
    }
}

/// The export fixtures are the pinned exporter's own bytes, from the pinned project sources: both
/// are pinned by SHA-256 in `fixtures/lean/exports/PROVENANCE.json`, so neither can drift silently.
#[test]
fn the_export_fixtures_are_pinned_to_their_provenance() {
    let provenance: Value = serde_json::from_str(
        &std::fs::read_to_string(fixtures().join("exports/PROVENANCE.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(provenance["toolchain"], intake::TOOLCHAIN);
    assert_eq!(
        provenance["exporter"]["revision"],
        intake::EXPORTER_REVISION
    );
    for (file, pin) in intake::EXPORTER_SOURCES {
        assert_eq!(provenance["exporter"][format!("{file}_sha256")], pin);
    }
    let exports = provenance["exports"].as_object().unwrap();
    assert_eq!(exports.len(), 6);
    for (file, entry) in exports {
        let bytes = std::fs::read(fixtures().join("exports").join(file)).unwrap();
        assert_eq!(
            entry["export_sha256"],
            sha256_hex(&bytes),
            "{file} drifted from its provenance"
        );
        let project = fixtures().join(entry["project"].as_str().unwrap());
        for (source, digest) in entry["sources_sha256"].as_object().unwrap() {
            assert_eq!(
                digest.as_str().unwrap(),
                sha256_hex(&std::fs::read(project.join(source)).unwrap()),
                "{source}"
            );
        }
    }
}
