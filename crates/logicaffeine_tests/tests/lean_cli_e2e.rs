//! The Lean import loop end to end — `largo lean import`, `largo lean replay`, and checking a LOGOS
//! kernel-vernacular document with `largo prove` / `largo check`.
//!
//! The plan's required tests, each asserting the returned artifact and its type: valid source project;
//! missing imported module; mismatched exporter version (`lean_intake_contract.rs`); invalid syntax;
//! unsupported semantic case; resource refusal; malformed JSON (`lean_intake_contract.rs`);
//! duplicated declaration (`lean_intake_contract.rs`); output determinism; compatibility mode; audit
//! emission. Competition rule R9 is locked here: a failed run never leaves a success artifact, and
//! never leaves an earlier success under the failed run's name.
//!
//! Project-mode tests need the pinned Lean 4.14.0 toolchain and the pinned lean4export checkout.
//! Where they are absent the same test asserts the refusal path — `infrastructure_failure`, exit 5 —
//! instead of passing silently; `LOGOS_REQUIRE_TOOLS=lean` makes their absence a failure.

mod lean_common;

use lean_common::*;
use serde_json::Value;
use std::path::Path;

fn assert_infrastructure_refusal(run: &Run) {
    assert_eq!(
        run.outcome(),
        "infrastructure_failure",
        "{}",
        run.describe()
    );
    assert_eq!(run.exit, 5);
    assert_eq!(run.stage("lean_accepted"), "failed");
    assert_eq!(run.codes()[0], "infrastructure");
}

/// A planted earlier success at `out`: a package directory as a previous verified run published it.
fn plant_earlier_success(out: &Path) -> Vec<(String, Vec<u8>)> {
    plant_earlier_package(out, "logos-lean-package/1")
}

/// [`plant_earlier_success`] under `schema`.
fn plant_earlier_package(out: &Path, schema: &str) -> Vec<(String, Vec<u8>)> {
    std::fs::create_dir_all(out.join("source")).unwrap();
    let files = vec![
        (
            "manifest.json".to_string(),
            format!(r#"{{"schema":"{schema}","result_id":"earlier"}}"#).into_bytes(),
        ),
        (
            "Earlier.lg".to_string(),
            b"-- an earlier, verified run\n".to_vec(),
        ),
        ("source/export.ndjson".to_string(), b"{}\n".to_vec()),
    ];
    for (name, bytes) in &files {
        std::fs::write(out.join(name), bytes).unwrap();
    }
    files
}

// ============================================================================================
// R9 — A FAILED RUN NEVER LEAVES A SUCCESS ARTIFACT
// ============================================================================================

#[test]
fn a_failed_run_never_leaves_a_success_artifact() {
    let dir = scratch();
    let out = dir.path().join("Gap.viaHole");
    let earlier = plant_earlier_success(&out);

    // A run that fails (a disallowed assumption) under the name of the earlier success.
    let run = export_import(&export("Gap.viaHole"), "Gap.viaHole", &out, &[]);
    assert_eq!(
        run.outcome(),
        "disallowed_assumptions",
        "{}",
        run.describe()
    );
    assert!(run.result["package"].is_null());
    assert!(
        !out.exists(),
        "the earlier success must not remain under the failed run's name"
    );

    // The earlier success is not destroyed: it moved aside, byte for byte, under its own name.
    let left = entries(dir.path());
    assert_eq!(
        left.len(),
        1,
        "exactly the superseded earlier package remains: {left:?}"
    );
    assert!(left[0].starts_with("Gap.viaHole.superseded-"), "{left:?}");
    let aside = dir.path().join(&left[0]);
    for (name, bytes) in &earlier {
        assert_eq!(
            &std::fs::read(aside.join(name)).unwrap(),
            bytes,
            "{name} is preserved"
        );
    }

    // Every failing category publishes nothing and leaves no staging directory behind.
    let clean = scratch();
    let failures = [
        export_import(
            &export("Runtime.identity"),
            "Runtime.identity",
            &clean.path().join("a"),
            &[],
        ),
        export_import(
            &export("Demo.addZero"),
            "Demo.addZero",
            &clean.path().join("b"),
            &["--max-export-bytes", "64"],
        ),
        export_import(
            &export("Demo.andSwap"),
            "Demo.nowhere",
            &clean.path().join("c"),
            &[],
        ),
        project_import(
            &project("valid"),
            "Demo.andSwap",
            &clean.path().join("d"),
            &["--lean", "/nonexistent/lean"],
        ),
        project_import(
            &project("missing_module"),
            "needs",
            &clean.path().join("e"),
            &[],
        ),
    ];
    for run in &failures {
        assert_ne!(run.outcome(), "verified", "{}", run.describe());
        assert!(run.result["package"].is_null());
    }
    assert!(
        entries(clean.path()).is_empty(),
        "failed runs left {:?}",
        entries(clean.path())
    );
}

/// Every package schema at `--out` is a package: an earlier native-rederived success is moved aside by
/// a failed run exactly as an ordinary one is, never refused as a directory that is not ours.
#[test]
fn an_earlier_package_of_every_schema_is_superseded_by_a_failed_run() {
    for schema in logicaffeine_kernel::interface::lean::package::PACKAGE_SCHEMAS {
        let dir = scratch();
        let out = dir.path().join("Gap.viaHole");
        let earlier = plant_earlier_package(&out, schema);
        let run = export_import(&export("Gap.viaHole"), "Gap.viaHole", &out, &[]);
        assert_eq!(
            run.outcome(),
            "disallowed_assumptions",
            "{schema}: {}",
            run.describe()
        );
        assert!(
            !out.exists(),
            "{schema}: the earlier success must not remain under the failed run's name"
        );
        let left = entries(dir.path());
        assert_eq!(
            left.len(),
            1,
            "{schema}: exactly the superseded earlier package remains: {left:?}"
        );
        assert!(
            left[0].starts_with("Gap.viaHole.superseded-"),
            "{schema}: {left:?}"
        );
        let aside = dir.path().join(&left[0]);
        for (name, bytes) in &earlier {
            assert_eq!(
                &std::fs::read(aside.join(name)).unwrap(),
                bytes,
                "{schema}: {name} is preserved"
            );
        }
    }
}

/// `--out` names a package. A directory there that is not a package is never touched: the run is
/// refused before any work and the directory stays byte-identical.
#[test]
fn a_non_package_directory_at_out_is_never_overwritten() {
    let dir = scratch();
    let out = dir.path().join("mine");
    std::fs::create_dir_all(&out).unwrap();
    std::fs::write(out.join("notes.txt"), "precious").unwrap();
    let run = export_import(&export("Demo.andSwap"), "Demo.andSwap", &out, &[]);
    assert_eq!(run.exit, 64, "{}", run.describe());
    assert!(run.stdout.is_empty());
    assert_eq!(
        std::fs::read_to_string(out.join("notes.txt")).unwrap(),
        "precious"
    );
    assert_eq!(entries(&out), vec!["notes.txt"]);
    assert_eq!(entries(dir.path()), vec!["mine"]);
}

// ============================================================================================
// THE VALID SOURCE PROJECT
// ============================================================================================

/// ★ The complete loop: a pinned Lean project goes in, a verified LOGOS package comes out, every
/// stage passed, published atomically at `--out` with the digest the result names.
#[test]
fn a_valid_source_project_is_verified_and_published() {
    let dir = scratch();
    let out = dir.path().join("Demo.andSwap");
    let run = project_import(&project("valid"), "Demo.andSwap", &out, &[]);
    let Some(_) = toolchain() else {
        return assert_infrastructure_refusal(&run);
    };
    assert_eq!(run.outcome(), "verified", "{}", run.describe());
    assert_eq!(run.exit, 0);
    for stage in STAGES {
        assert_eq!(run.stage(stage), "passed", "{stage}\n{}", run.describe());
    }
    assert_eq!(run.result["target"]["lean"], "Demo.andSwap");
    assert!(
        run.result["target"]["logos"].is_string(),
        "{}",
        run.describe()
    );
    assert_eq!(run.result["assumptions"], serde_json::json!([]));
    let package = &run.result["package"];
    assert_eq!(package["path"], out.display().to_string());
    let manifest =
        std::fs::read(out.join("manifest.json")).expect("the package is published at --out");
    assert_eq!(package["manifest_sha256"], sha256_hex(&manifest));
    let manifest: Value = serde_json::from_slice(&manifest).unwrap();
    assert_eq!(manifest["schema"], "logos-lean-package/1");
    assert!(out.join("certificate.json").is_file() && out.join("source/export.ndjson").is_file());
    assert!(
        entries(&out).iter().any(|f| f.ends_with(".lg")),
        "the readable LOGOS document is published: {:?}",
        entries(&out)
    );
    assert_eq!(
        entries(dir.path()),
        vec!["Demo.andSwap"],
        "nothing but the package is left"
    );

    // The published package replays in a fresh process.
    let replay = run_in(dir.path(), &["lean", "replay", out.to_str().unwrap()]);
    assert_eq!(replay.exit, 0, "{}", replay.describe());
    assert_eq!(replay.result["schema"], "logos-lean-replay/1");
    assert_eq!(replay.result["stages"]["fresh_replay"]["status"], "passed");
    assert_eq!(replay.result["replayed"], true);
}

/// The part of the loop the intake lane owns, independent of emission and packaging: Lean accepted
/// the project, the target and its closure were translated, checked, independently rechecked, and
/// its assumptions — from Lean's own `#print axioms` and from the LOGOS audit, which must agree —
/// are within the Stage-A policy.
#[test]
fn a_valid_projects_lean_and_kernel_stages_pass() {
    let dir = scratch();
    for (target, axioms) in [
        ("Demo.andSwap", Vec::<&str>::new()),
        ("Demo.doubleTwo", vec![]),
        ("Demo.addZero", vec![]),
    ] {
        let run = project_import(&project("valid"), target, &dir.path().join(target), &[]);
        let Some(_) = toolchain() else {
            return assert_infrastructure_refusal(&run);
        };
        for stage in [
            "lean_accepted",
            "translated",
            "primary_check",
            "independent_recheck",
            "assumption_policy",
        ] {
            assert_eq!(
                run.stage(stage),
                "passed",
                "{target} {stage}\n{}",
                run.describe()
            );
        }
        assert_eq!(run.result["target"]["lean"], target);
        assert_eq!(
            run.result["assumptions"],
            serde_json::json!(axioms),
            "{}",
            run.describe()
        );
        assert_eq!(run.result["audit"]["schema"], "logos-lean-import-report/1");
    }
}

/// Classical logic is inside the Stage-A policy; the assumptions are reported, sorted, by their Lean
/// names — and Lean's `#print axioms` agrees with the LOGOS audit.
#[test]
fn a_classical_project_is_admitted_with_its_assumptions_named() {
    let dir = scratch();
    let run = project_import(
        &project("classical"),
        "Excluded.middle",
        &dir.path().join("out"),
        &[],
    );
    let Some(_) = toolchain() else {
        return assert_infrastructure_refusal(&run);
    };
    assert_eq!(run.stage("lean_accepted"), "passed", "{}", run.describe());
    assert_eq!(
        run.stage("assumption_policy"),
        "passed",
        "{}",
        run.describe()
    );
    assert_eq!(
        run.result["assumptions"],
        serde_json::json!(["Classical.choice", "Quot.sound", "propext"]),
        "{}",
        run.describe()
    );
}

// ============================================================================================
// PROJECT-MODE REFUSALS
// ============================================================================================

/// Resolved before Lean runs: an import that is neither a project module nor Lean core.
#[test]
fn a_missing_imported_module_is_invalid_source() {
    let dir = scratch();
    let out = dir.path().join("out");
    let run = project_import(&project("missing_module"), "needs", &out, &[]);
    assert_eq!(run.outcome(), "invalid_source", "{}", run.describe());
    assert_eq!(run.exit, 1);
    assert_eq!(run.codes(), vec!["missing_module"], "{}", run.describe());
    let d = run.diagnostic("missing_module");
    assert!(
        d["message"].as_str().unwrap().contains("Missing.Thing"),
        "{d}"
    );
    assert_eq!(d["line"], 1);
    assert_eq!(run.stage("lean_accepted"), "failed");
    assert_eq!(run.stage("translated"), "not_run");
    assert!(!out.exists());
}

#[test]
fn invalid_syntax_is_invalid_source_at_its_lean_line() {
    let dir = scratch();
    let out = dir.path().join("out");
    let run = project_import(&project("invalid_syntax"), "Broken.fine", &out, &[]);
    let Some(_) = toolchain() else {
        return assert_infrastructure_refusal(&run);
    };
    assert_eq!(run.outcome(), "invalid_source", "{}", run.describe());
    assert_eq!(run.exit, 1);
    assert_eq!(run.stage("lean_accepted"), "failed");
    let d = run.diagnostic("lean_rejected");
    assert_eq!(
        d["line"],
        5,
        "Lean's own location of the syntax error\n{}",
        run.describe()
    );
    assert!(
        d["message"].as_str().unwrap().contains("Broken.lean"),
        "{d}"
    );
    assert!(!out.exists());
}

/// ★ Lean compiles a `sorry` with exit code 0 and a warning. Taint comes from `#print axioms`.
#[test]
fn sorry_is_refused_by_print_axioms_not_by_exit_code() {
    let dir = scratch();
    let out = dir.path().join("out");
    let run = project_import(&project("sorry"), "Gap.viaHole", &out, &[]);
    let Some(_) = toolchain() else {
        return assert_infrastructure_refusal(&run);
    };
    assert_eq!(
        run.stage("lean_accepted"),
        "passed",
        "Lean accepted the file\n{}",
        run.describe()
    );
    assert_eq!(
        run.outcome(),
        "disallowed_assumptions",
        "{}",
        run.describe()
    );
    assert_eq!(run.exit, 3);
    assert_eq!(run.stage("assumption_policy"), "failed");
    assert_eq!(run.result["assumptions"], serde_json::json!(["sorryAx"]));
    assert!(!out.exists());
}

/// An unknown target: the pinned exporter PANICS on it and still exits 0, so its exit code is never
/// trusted — the target is resolved by Lean first.
#[test]
fn an_unknown_target_is_a_missing_declaration_even_though_the_exporter_exits_zero() {
    let dir = scratch();
    let run = project_import(
        &project("valid"),
        "Demo.nowhere",
        &dir.path().join("out"),
        &[],
    );
    let Some(_) = toolchain() else {
        return assert_infrastructure_refusal(&run);
    };
    assert_eq!(run.outcome(), "invalid_source", "{}", run.describe());
    assert_eq!(
        run.codes(),
        vec!["missing_declaration"],
        "{}",
        run.describe()
    );
    assert_eq!(
        run.diagnostic("missing_declaration")["declaration"],
        "Demo.nowhere"
    );
}

#[test]
fn a_project_pinned_to_another_toolchain_is_unsupported() {
    let dir = scratch();
    let run = project_import(
        &project("wrong_toolchain"),
        "later",
        &dir.path().join("out"),
        &[],
    );
    assert_eq!(run.outcome(), "unsupported_feature", "{}", run.describe());
    assert_eq!(
        run.codes(),
        vec!["unsupported_toolchain"],
        "{}",
        run.describe()
    );
    assert!(run.diagnostic("unsupported_toolchain")["message"]
        .as_str()
        .unwrap()
        .contains("v4.29.1"));
}

#[test]
fn an_absent_toolchain_is_an_infrastructure_failure() {
    let dir = scratch();
    let run = project_import(
        &project("valid"),
        "Demo.andSwap",
        &dir.path().join("out"),
        &["--lean", "/nonexistent/lean"],
    );
    assert_infrastructure_refusal(&run);
    assert!(run.diagnostic("infrastructure")["message"]
        .as_str()
        .unwrap()
        .contains("/nonexistent/lean"));
}

/// The exporter is pinned by content: a checkout whose sources differ from revision 20c4a64a is not
/// the pinned exporter, whatever its directory is called.
#[test]
fn an_unpinned_exporter_is_an_infrastructure_failure() {
    let dir = scratch();
    let fake = dir.path().join("lean4export");
    std::fs::create_dir_all(fake.join(".lake/build/lib")).unwrap();
    std::fs::write(fake.join("Main.lean"), "-- not the pinned exporter\n").unwrap();
    std::fs::write(fake.join("Export.lean"), "-- not the pinned exporter\n").unwrap();
    std::fs::write(fake.join(".lake/build/lib/Export.olean"), b"").unwrap();
    let mut args: Vec<String> = ["lean", "import", "--project"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    args.push(project("valid").display().to_string());
    args.extend(
        ["--target", "Demo.andSwap", "--out"]
            .iter()
            .map(|s| s.to_string()),
    );
    args.push(dir.path().join("out").display().to_string());
    args.push("--exporter".into());
    args.push(fake.display().to_string());
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let run = run(&args);
    assert_infrastructure_refusal(&run);
    let named = run.result["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|d| {
            d["code"] == "infrastructure" && d["message"].as_str().unwrap().contains("20c4a64a")
        });
    assert!(
        named,
        "the refusal names the pinned revision\n{}",
        run.describe()
    );
}

// ============================================================================================
// UNSUPPORTED SEMANTICS, RESOURCES
// ============================================================================================

/// An `unsafe` definition has no faithful LOGOS reading: it is declined, never approximated, never
/// replaced by an axiom.
#[test]
fn an_unsupported_semantic_case_is_declined_not_approximated() {
    let dir = scratch();
    let run = export_import(
        &export("Runtime.identity"),
        "Runtime.identity",
        &dir.path().join("out"),
        &[],
    );
    assert_eq!(run.outcome(), "unsupported_feature", "{}", run.describe());
    assert_eq!(run.exit, 2);
    assert_eq!(run.stage("translated"), "failed");
    assert_eq!(run.stage("primary_check"), "not_run");
    let d = run.diagnostic("unsupported_feature");
    assert!(
        d["message"]
            .as_str()
            .unwrap()
            .contains("runtime declarations remain unchecked"),
        "{d}"
    );
    assert_eq!(
        run.result["assumptions"],
        serde_json::json!([]),
        "nothing was admitted in its place"
    );

    let project_run = project_import(
        &project("unsupported"),
        "Runtime.identity",
        &dir.path().join("p"),
        &[],
    );
    let Some(_) = toolchain() else {
        return assert_infrastructure_refusal(&project_run);
    };
    assert_eq!(
        project_run.outcome(),
        "unsupported_feature",
        "{}",
        project_run.describe()
    );
    assert_eq!(project_run.stage("lean_accepted"), "passed");
    assert_eq!(project_run.stage("translated"), "failed");
}

#[test]
fn a_resource_refusal_is_its_own_outcome() {
    let dir = scratch();
    let run = export_import(
        &export("Demo.addZero"),
        "Demo.addZero",
        &dir.path().join("a"),
        &["--max-export-bytes", "4096"],
    );
    assert_eq!(run.outcome(), "resource_limit", "{}", run.describe());
    assert_eq!(run.exit, 4);
    let d = run.diagnostic("resource_limit");
    assert!(
        d["message"].as_str().unwrap().contains("4096"),
        "the budget is named: {d}"
    );
    assert_eq!(run.stage("translated"), "failed");

    // A project-mode deadline is a declared budget too, decided before Lean runs.
    let project_run = project_import(
        &project("valid"),
        "Demo.andSwap",
        &dir.path().join("b"),
        &["--timeout-secs", "0"],
    );
    assert_eq!(
        project_run.outcome(),
        "resource_limit",
        "{}",
        project_run.describe()
    );
    assert_eq!(project_run.stage("lean_accepted"), "failed");
    assert!(entries(dir.path()).is_empty());
}

// ============================================================================================
// DETERMINISM, COMPATIBILITY, AUDIT
// ============================================================================================

#[test]
fn output_is_deterministic() {
    let dir = scratch();
    for (name, target) in [
        ("Demo.andSwap", "Demo.andSwap"),
        ("Gap.viaHole", "Gap.viaHole"),
        ("Excluded.middle", "Excluded.middle"),
    ] {
        let out = dir.path().join(name);
        let first = export_import(&export(name), target, &out, &[]);
        let first_tree = tree(&out);
        let _ = std::fs::remove_dir_all(&out);
        let second = export_import(&export(name), target, &out, &[]);
        assert_eq!(
            first.stdout, second.stdout,
            "{name}: the result is byte-identical across runs"
        );
        assert_eq!(
            first.stderr, second.stderr,
            "{name}: so are the diagnostics"
        );
        assert_eq!(first.exit, second.exit);
        assert_eq!(
            first_tree,
            tree(&out),
            "{name}: and so is anything published"
        );
    }
    if toolchain().is_some() {
        let first = project_import(
            &project("valid"),
            "Demo.andSwap",
            &dir.path().join("p1"),
            &[],
        );
        let second = project_import(
            &project("valid"),
            "Demo.andSwap",
            &dir.path().join("p1"),
            &[],
        );
        assert_eq!(
            first.stdout, second.stdout,
            "project mode is deterministic too"
        );
    }
}

/// Every file under `dir` with its bytes, sorted by path.
fn tree(dir: &Path) -> Vec<(String, Vec<u8>)> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, Vec<u8>)>) {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in rd.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(root, &path, out);
            } else {
                out.push((
                    path.strip_prefix(root).unwrap().display().to_string(),
                    std::fs::read(&path).unwrap(),
                ));
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out.sort();
    out
}

/// `--arena-protocol` is the Lean Kernel Arena benchmark protocol, byte for byte: the same stdout,
/// stderr and exit code as `interface::lean::run_cli` (the `lean_arena` example) on accepted,
/// rejected and declined inputs, with and without its opt-in flags.
#[test]
fn compatibility_mode_is_the_arena_protocol_byte_for_byte() {
    let dir = scratch();
    let ill_typed = {
        let text = export_text("Demo.andSwap");
        let line = declaration_line(&text, "Demo.andSwap");
        let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
        let mut v: Value = serde_json::from_str(&lines[line - 1]).unwrap();
        v["thm"]["value"] = v["thm"]["type"].clone();
        lines[line - 1] = v.to_string();
        write_export(dir.path(), "ill_typed.ndjson", &(lines.join("\n") + "\n"))
    };
    let inputs = [
        export("Demo.andSwap"),
        export("Gap.viaHole"),
        export("Runtime.identity"),
        ill_typed,
    ];
    let mut exits = std::collections::BTreeSet::new();
    for input in &inputs {
        for flags in [
            &[][..],
            &["--audit-declarations"][..],
            &["--trace-stages"][..],
        ] {
            let mut expected_out = Vec::new();
            let mut expected_err = Vec::new();
            let args: Vec<std::ffi::OsString> = std::iter::once(input.clone().into_os_string())
                .chain(flags.iter().map(|f| f.into()))
                .collect();
            let expected_code = logicaffeine_kernel::interface::lean::run_cli(
                args,
                &mut expected_out,
                &mut expected_err,
            );
            let output = largo()
                .args([
                    "lean",
                    "import",
                    input.to_str().unwrap(),
                    "--arena-protocol",
                ])
                .args(flags)
                .output()
                .unwrap();
            assert_eq!(
                output.status.code(),
                Some(expected_code),
                "{} {flags:?}",
                input.display()
            );
            assert_eq!(
                output.stdout,
                expected_out,
                "{} {flags:?}: stdout",
                input.display()
            );
            assert_eq!(
                output.stderr,
                expected_err,
                "{} {flags:?}: stderr",
                input.display()
            );
            exits.insert(expected_code);
        }
    }
    assert_eq!(
        exits,
        [0, 1, 2].into_iter().collect(),
        "accept, reject and decline are all exercised"
    );
    let accept = largo()
        .args([
            "lean",
            "import",
            export("Demo.andSwap").to_str().unwrap(),
            "--arena-protocol",
        ])
        .output()
        .unwrap();
    assert_eq!(accept.stdout, b"ACCEPT\n");
    assert!(accept.stderr.is_empty());
}

/// W02 step 5: the audit the importer already computes is emitted with the result — every audited
/// declaration of the closure with its transitive axioms, the internal helpers, and the recorded
/// metadata losses.
#[test]
fn the_audit_is_emitted_with_the_result() {
    let dir = scratch();
    let run = export_import(
        &export("Gap.viaHole"),
        "Gap.viaHole",
        &dir.path().join("out"),
        &[],
    );
    let audit = &run.result["audit"];
    assert_eq!(
        audit["schema"],
        "logos-lean-import-report/1",
        "{}",
        run.describe()
    );
    assert_eq!(audit["declaration_audit"]["requested"], true);
    let declarations = audit["declaration_audit"]["declarations"]
        .as_array()
        .unwrap();
    let via_hole = declarations
        .iter()
        .find(|d| d["name"] == r#"/s:"Gap"/s:"viaHole""#)
        .unwrap_or_else(|| panic!("the target is audited: {audit}"));
    assert_eq!(via_hole["kind"], "thm");
    assert_eq!(via_hole["axioms"], serde_json::json!([r#"/s:"sorryAx""#]));
    let hole = declarations
        .iter()
        .find(|d| d["name"] == r#"/s:"Gap"/s:"hole""#)
        .unwrap();
    assert_eq!(
        hole["axioms"],
        serde_json::json!([r#"/s:"sorryAx""#]),
        "the taint is transitive"
    );

    let classical = export_import(
        &export("Excluded.middle"),
        "Excluded.middle",
        &dir.path().join("c"),
        &[],
    );
    assert_eq!(
        classical.result["audit"]["schema"],
        "logos-lean-import-report/1"
    );
    assert_eq!(
        classical.stage("assumption_policy"),
        "passed",
        "{}",
        classical.describe()
    );
    assert_eq!(
        classical.result["assumptions"],
        serde_json::json!(["Classical.choice", "Quot.sound", "propext"])
    );
    // The losses are reported as information losses of the result.
    for loss in classical.result["information_losses"].as_array().unwrap() {
        assert!(
            loss["category"].is_string() && loss["detail"].is_string(),
            "{loss}"
        );
    }
}

/// A pre-exported artifact's Lean validity was not observed by this run: the stage says `not_run`,
/// the result is not `verified`, and the reason is a typed diagnostic.
#[test]
fn export_mode_never_claims_lean_acceptance() {
    let dir = scratch();
    let run = export_import(
        &export("Demo.andSwap"),
        "Demo.andSwap",
        &dir.path().join("out"),
        &[],
    );
    assert_eq!(run.stage("lean_accepted"), "not_run", "{}", run.describe());
    assert_ne!(run.outcome(), "verified");
    assert!(
        run.codes()
            .contains(&"lean_acceptance_not_observed".to_string()),
        "{}",
        run.describe()
    );
    for stage in [
        "translated",
        "primary_check",
        "independent_recheck",
        "assumption_policy",
    ] {
        assert_eq!(run.stage(stage), "passed", "{stage}\n{}", run.describe());
    }
}

// ============================================================================================
// REPLAY
// ============================================================================================

#[test]
fn replay_prints_one_receipt_naming_every_stage() {
    let dir = scratch();
    let missing = run(&[
        "lean",
        "replay",
        dir.path().join("absent").to_str().unwrap(),
    ]);
    assert_eq!(missing.exit, 5, "{}", missing.describe());
    assert_eq!(missing.result["schema"], "logos-lean-replay/1");
    let stages = missing.result["stages"].as_object().unwrap();
    assert_eq!(stages.len(), STAGES.len());
    assert!(
        stages.values().all(|s| s["status"] != "passed"),
        "an unreadable package passes nothing"
    );
    assert_eq!(missing.result["replayed"], false);

    let junk = dir.path().join("junk");
    std::fs::create_dir_all(&junk).unwrap();
    std::fs::write(junk.join("manifest.json"), "not json").unwrap();
    let refused = run(&["lean", "replay", junk.to_str().unwrap()]);
    assert_ne!(refused.exit, 0, "{}", refused.describe());
    assert_eq!(refused.result["schema"], "logos-lean-replay/1");
    assert_ne!(refused.result["stages"]["fresh_replay"]["status"], "passed");
    assert_eq!(refused.result["replayed"], false);
}

// ============================================================================================
// A LOGOS KERNEL-VERNACULAR DOCUMENT IS CHECKED BY LOGOS'S OWN DOOR
// ============================================================================================

const TRUE_DOCUMENT: &str = "\
-- A small development, in LOGOS's own vernacular.
Definition two : Nat := Succ (Succ Zero).

## Theorem: TrueRing
    Statement: 48 equals (mul 6 8).
    Proof: ring.
";

const FALSE_DOCUMENT: &str = "\
## Theorem: TrueRing
    Statement: 48 equals (mul 6 8).
    Proof: ring.

## Theorem: FalseLia
    Statement: 5 < 2.
    Proof: lia.
";

#[test]
fn a_kernel_document_is_checked_by_largo_prove_and_check() {
    let dir = scratch();
    let good = dir.path().join("Good.lg");
    std::fs::write(&good, TRUE_DOCUMENT).unwrap();
    let bad = dir.path().join("Bad.lg");
    std::fs::write(&bad, FALSE_DOCUMENT).unwrap();

    let proved = largo()
        .args(["prove", "--kernel", good.to_str().unwrap(), "--json"])
        .output()
        .unwrap();
    assert_eq!(
        proved.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&proved.stderr)
    );
    let report: Value = serde_json::from_slice(&proved.stdout).unwrap();
    assert_eq!(report["surface"], "kernel_document");
    assert_eq!(report["checked"], true);
    // The door reads three blocks: the leading comment, the definition and the theorem.
    assert_eq!(report["blocks"].as_array().unwrap().len(), 3, "{report}");

    let checked = largo()
        .args(["check", "--kernel", good.to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(
        checked.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&checked.stderr)
    );

    for command in ["prove", "check"] {
        let refused = largo()
            .args([command, "--kernel", bad.to_str().unwrap()])
            .output()
            .unwrap();
        assert_eq!(
            refused.status.code(),
            Some(1),
            "{command} refuses a false block"
        );
        let text = String::from_utf8_lossy(&refused.stderr);
        assert!(
            text.contains("FalseLia") || text.contains("5 < 2") || text.contains("block 2"),
            "{command}: {text}"
        );
    }
    let refused = largo()
        .args(["prove", "--kernel", bad.to_str().unwrap(), "--json"])
        .output()
        .unwrap();
    let report: Value = serde_json::from_slice(&refused.stdout).unwrap();
    assert_eq!(report["checked"], false);
    assert_eq!(report["failed_block"], 2, "{report}");
}

// ============================================================================================
// THE CORPUS LOOP — a corpus case, submitted as Lean, out as verified LOGOS
// ============================================================================================

/// `T06-002_eq_rec_j.lean`, byte for byte the corpus case (pinned by its provenance's digest), goes
/// in as a Lean project; `T06_002.transport` comes out verified, published, and replayed by
/// `largo lean replay` in a fresh process.
#[test]
fn corpus_t06_002_transport_is_verified_end_to_end() {
    let source = std::fs::read(project("t06_002").join("T06_002.lean")).unwrap();
    let provenance: Value = serde_json::from_str(
        &std::fs::read_to_string(repo_root().join("crates/logicaffeine_tests/tests/fixtures/lean_conformance/cases_lean414/T06-002.provenance.json"))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        provenance["source_sha256"],
        sha256_hex(&source),
        "the fixture is the corpus case, unchanged"
    );

    let dir = scratch();
    let out = dir.path().join("T06_002.transport");
    let run = project_import(&project("t06_002"), "T06_002.transport", &out, &[]);
    let Some(_) = toolchain() else {
        return assert_infrastructure_refusal(&run);
    };
    assert_eq!(run.outcome(), "verified", "{}", run.describe());
    for stage in STAGES {
        assert_eq!(run.stage(stage), "passed", "{stage}\n{}", run.describe());
    }
    assert_eq!(run.result["target"]["lean"], "T06_002.transport");
    assert_eq!(run.result["package"]["path"], out.display().to_string());
    let manifest = std::fs::read(out.join("manifest.json")).unwrap();
    assert_eq!(
        run.result["package"]["manifest_sha256"],
        sha256_hex(&manifest)
    );
    assert!(
        entries(&out).iter().any(|f| f.ends_with(".lg")),
        "{:?}",
        entries(&out)
    );
    let document = std::fs::read_to_string(out.join("T06_002.transport.lg")).unwrap();
    let statement = document
        .lines()
        .find(|l| l.starts_with("Theorem T06_002.transport"))
        .unwrap_or_else(|| panic!("the target is stated:\n{document}"));
    // The corpus source: `theorem transport {α : Type} (P : α → Prop) {a b : α} (h : a = b) (pa : P a) : P b`.
    for binder in ["(P :", "(h :", "(pa :"] {
        assert!(
            statement.contains(binder),
            "the LOGOS reads with Lean's own binder {binder}: {statement}"
        );
    }

    let replay = run_in(dir.path(), &["lean", "replay", out.to_str().unwrap()]);
    assert_eq!(replay.exit, 0, "{}", replay.describe());
    assert_eq!(replay.result["replayed"], true);
    assert_eq!(replay.result["stages"]["fresh_replay"]["status"], "passed");
}

/// More of corpus case T06-002 through the same loop: symmetry via the recursor, and `Eq.mpr`.
#[test]
fn corpus_t06_002_symmetry_and_mpr_are_verified() {
    let dir = scratch();
    for target in ["T06_002.symmViaRec", "T06_002.viaMpr"] {
        let out = dir.path().join(target);
        let run = project_import(&project("t06_002"), target, &out, &[]);
        let Some(_) = toolchain() else {
            return assert_infrastructure_refusal(&run);
        };
        assert_eq!(run.outcome(), "verified", "{target}\n{}", run.describe());
        assert_eq!(run.result["assumptions"], serde_json::json!([]), "{target}");
        assert!(out.join("manifest.json").is_file(), "{target} is published");
        let replay = run_in(dir.path(), &["lean", "replay", out.to_str().unwrap()]);
        assert_eq!(replay.exit, 0, "{target}\n{}", replay.describe());
    }
}

/// A theorem proved with `propext` is admitted under the Stage-A policy: verified, and the
/// assumption is named — by Lean's `#print axioms` and by the LOGOS audit alike.
#[test]
fn a_propext_theorem_is_verified_with_its_assumption_named() {
    let dir = scratch();
    let out = dir.path().join("Ext.iffEq");
    let run = project_import(&project("propext"), "Ext.iffEq", &out, &[]);
    let Some(_) = toolchain() else {
        return assert_infrastructure_refusal(&run);
    };
    assert_eq!(run.outcome(), "verified", "{}", run.describe());
    assert_eq!(run.result["assumptions"], serde_json::json!(["propext"]));
    assert_eq!(run.stage("assumption_policy"), "passed");
    assert!(out.join("manifest.json").is_file());
}
