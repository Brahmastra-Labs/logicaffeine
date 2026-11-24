//! Track L1 — `largo lean import --project` under every registered Lean profile.
//!
//! The project's `lean-toolchain` selects its profile; the toolchain must report that release and
//! its commit; the exporter must be that profile's, pinned by the SHA-256 of every source and of the
//! built executable, found by `--exporter`, else in the exporter registry (`$LOGOS_LEAN_EXPORTERS`,
//! else `~/.logos/lean/exporters/<version>/`, built by `scripts/lean/install-lean-exporters.py`); and
//! the export it writes must be admitted under exactly that profile. A pin outside the registry is a
//! typed `unsupported_toolchain` refusal; a Lean or an exporter that is not the profile's is an
//! `infrastructure` refusal naming what was expected. The published package names the profile.
//!
//! Every test needs the profile's toolchain and exporter. Where they are absent the same test asserts
//! the refusal path instead of passing silently; `LOGOS_REQUIRE_TOOLS=lean` makes their absence a
//! failure.

mod lean_common;

use lean_common::*;
use logicaffeine_kernel::interface::lean::profile::{self, ExporterForm, Profile};
use serde_json::Value;
use std::path::{Path, PathBuf};

/// The platform the pinned executables were built for.
const LINUX: &str = "x86_64-unknown-linux-gnu";

/// A copy of the project of `profile`, so a test can change its pin without touching the fixture.
fn project_copy(dir: &Path, version: &str) -> PathBuf {
    let from = project(&format!("profile-{version}"));
    let to = dir.join(format!("profile-{version}"));
    std::fs::create_dir_all(&to).unwrap();
    for entry in std::fs::read_dir(&from).unwrap().flatten() {
        std::fs::copy(entry.path(), to.join(entry.file_name())).unwrap();
    }
    to
}

/// `lean import --project <project> --target <target> --out <out> [extra…]`, the exporter found by
/// the registry (`LOGOS_LEAN_EXPORTERS`, set to the registry this box holds).
fn import_profile(project: &Path, target: &str, out: &Path, extra: &[&str]) -> Run {
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
    command.env_remove("LOGOS_LEAN4EXPORT");
    held(command.output().expect("largo spawns"))
}

fn assert_refused_as(run: &Run, outcome: &str, code: &str) {
    assert_eq!(run.outcome(), outcome, "{}", run.describe());
    assert_eq!(run.stage("lean_accepted"), "failed", "{}", run.describe());
    assert!(
        run.codes().iter().any(|c| c == code),
        "a {code} diagnostic\n{}",
        run.describe()
    );
    assert!(run.result["package"].is_null());
}

fn assert_infrastructure_refusal(run: &Run) {
    assert_refused_as(run, "infrastructure_failure", "infrastructure");
    assert_eq!(run.exit, 5);
}

/// The profile's toolchain and registered exporter, when this box has both.
fn tools(p: &Profile) -> bool {
    profile_toolchain(p).is_some() && profile_exporter(p).is_some()
}

// ============================================================================================
// EVERY PROFILE, END TO END
// ============================================================================================

/// ★ Each registered profile's own project goes in and a verified package comes out, naming the
/// release, its commit, the toolchain Lean reported and the profile's exporter by revision, patch
/// and executable digest; it replays in a fresh process.
#[test]
fn every_registered_profile_imports_its_own_project_verified() {
    for p in profile::profiles() {
        let dir = scratch();
        let out = dir.path().join("Profile.andSwap");
        let run = import_profile(
            &project(&format!("profile-{}", p.lean_version)),
            "Profile.andSwap",
            &out,
            &[],
        );
        if !tools(p) {
            require_absent_tools_allowed(p);
            assert_infrastructure_refusal(&run);
            continue;
        }
        let v = p.lean_version;
        assert_eq!(run.outcome(), "verified", "{v}\n{}", run.describe());
        for stage in STAGES {
            assert_eq!(
                run.stage(stage),
                "passed",
                "{v} {stage}\n{}",
                run.describe()
            );
        }
        let manifest: Value =
            serde_json::from_slice(&std::fs::read(out.join("manifest.json")).unwrap()).unwrap();
        assert_eq!(manifest["source"]["lean_version"], v);
        assert_eq!(manifest["source"]["lean_githash"], p.lean_githash);
        let lean = manifest["toolchain"]["lean"].as_str().unwrap();
        assert!(lean.starts_with(p.toolchain), "{v}: {lean}");
        assert!(lean.contains(&format!("version {v},")), "{v}: {lean}");
        let exporter = manifest["toolchain"]["exporter"].as_str().unwrap();
        assert!(exporter.contains(p.exporter.revision), "{v}: {exporter}");
        if let Some(patch) = p.exporter.structural_patch_sha256 {
            assert!(exporter.contains(patch), "{v}: {exporter}");
        }
        if p.exporter.form == ExporterForm::Executable {
            let binary = p.exporter.binary_for(LINUX).unwrap();
            assert!(exporter.contains(binary), "{v}: {exporter}");
        }
        let replay = run_in(dir.path(), &["lean", "replay", out.to_str().unwrap()]);
        assert_eq!(replay.exit, 0, "{v}\n{}", replay.describe());
        assert_eq!(replay.result["replayed"], true, "{v}");
    }
}

/// The export a profile's project writes is admitted under exactly that profile: the header carries
/// the release and the commit the toolchain itself reported.
#[test]
fn a_profiles_project_export_carries_its_own_release_and_commit() {
    for p in profile::profiles() {
        let dir = scratch();
        let out = dir.path().join("out");
        let run = import_profile(
            &project(&format!("profile-{}", p.lean_version)),
            "Profile.doubleTwo",
            &out,
            &[],
        );
        if !tools(p) {
            require_absent_tools_allowed(p);
            assert_infrastructure_refusal(&run);
            continue;
        }
        assert_eq!(
            run.outcome(),
            "verified",
            "{}\n{}",
            p.lean_version,
            run.describe()
        );
        let export = std::fs::read_to_string(out.join("source/export.ndjson")).unwrap();
        let header: Value = serde_json::from_str(export.lines().next().unwrap()).unwrap();
        assert_eq!(header["meta"]["lean"]["version"], p.lean_version);
        assert_eq!(header["meta"]["lean"]["githash"], p.lean_githash);
        let read = logicaffeine_kernel::interface::lean::intake::read_admitted(&export).unwrap();
        assert_eq!(read.profile().lean_version, p.lean_version);
    }
}

// ============================================================================================
// PINS OUTSIDE THE REGISTRY
// ============================================================================================

/// A pin naming a release with no registered profile — even one whose toolchain is installed — is
/// `unsupported_toolchain` before Lean runs, and the refusal lists the registered profiles.
#[test]
fn a_project_pinned_outside_the_registry_is_unsupported() {
    for pin in [
        "leanprover/lean4:v4.34.0",
        "leanprover/lean4:v4.29.1",
        "leanprover/lean4:nightly",
        "leanprover/lean4:v4.33.1\nleanprover/lean4:v4.34.1",
    ] {
        let dir = scratch();
        let project = project_copy(dir.path(), "4.33.1");
        std::fs::write(project.join("lean-toolchain"), format!("{pin}\n")).unwrap();
        let run = import_profile(&project, "Profile.andSwap", &dir.path().join("out"), &[]);
        assert_refused_as(&run, "unsupported_feature", "unsupported_toolchain");
        assert_eq!(
            run.codes(),
            vec!["unsupported_toolchain"],
            "{}",
            run.describe()
        );
        let message = run.diagnostic("unsupported_toolchain")["message"]
            .as_str()
            .unwrap()
            .to_string();
        for p in profile::profiles() {
            assert!(message.contains(p.lean_version), "{message}");
        }
        assert!(!dir.path().join("out").exists(), "nothing is published");
    }
}

/// A Lean that is not the profile's release — here the next profile's own — is an infrastructure
/// refusal naming the release the project pins.
#[test]
fn a_lean_of_another_release_is_refused() {
    let p = profile::by_version("4.28.0").unwrap();
    let other = profile::by_version("4.33.1").unwrap();
    let dir = scratch();
    let Some(lean) = profile_toolchain(other) else {
        return;
    };
    let run = import_profile(
        &project("profile-4.28.0"),
        "Profile.andSwap",
        &dir.path().join("out"),
        &["--lean", lean.join("bin/lean").to_str().unwrap()],
    );
    assert_infrastructure_refusal(&run);
    let message = run.diagnostic("infrastructure")["message"]
        .as_str()
        .unwrap();
    assert!(
        message.contains(&format!("Lean {}", p.lean_version)),
        "{message}"
    );
}

/// Another profile's exporter is not this profile's, whatever its directory is called: the refusal
/// names the revision the profile pins.
#[test]
fn an_exporter_of_another_profile_is_refused() {
    let p = profile::by_version("4.28.0").unwrap();
    let other = profile::by_version("4.33.1").unwrap();
    let Some(exporter) = profile_exporter(other) else {
        return;
    };
    let dir = scratch();
    let run = import_profile(
        &project("profile-4.28.0"),
        "Profile.andSwap",
        &dir.path().join("out"),
        &["--exporter", exporter.to_str().unwrap()],
    );
    assert_infrastructure_refusal(&run);
    let named = run.result["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|d| {
            d["code"] == "infrastructure"
                && d["message"]
                    .as_str()
                    .unwrap()
                    .contains(&p.exporter.revision[..12])
        });
    assert!(
        named,
        "the refusal names the pinned revision\n{}",
        run.describe()
    );
}

/// An exporter whose sources are the profile's but whose executable is not the pinned build is
/// refused by the executable's digest.
#[test]
fn an_exporter_executable_that_is_not_the_pinned_build_is_refused() {
    let p = profile::by_version("4.33.1").unwrap();
    let Some(exporter) = profile_exporter(p) else {
        return;
    };
    let dir = scratch();
    let fake = dir.path().join("4.33.1");
    for (name, _) in p.exporter.sources {
        let to = fake.join(name);
        std::fs::create_dir_all(to.parent().unwrap()).unwrap();
        std::fs::copy(exporter.join(name), &to).unwrap();
    }
    std::fs::create_dir_all(fake.join(".lake/build/bin")).unwrap();
    std::fs::write(
        fake.join(".lake/build/bin/lean4export"),
        "#!/bin/sh\necho '{}'\n",
    )
    .unwrap();
    let run = import_profile(
        &project("profile-4.33.1"),
        "Profile.andSwap",
        &dir.path().join("out"),
        &["--exporter", fake.to_str().unwrap()],
    );
    assert_infrastructure_refusal(&run);
    let message = run.diagnostic("infrastructure")["message"]
        .as_str()
        .unwrap();
    assert!(message.contains("sha256"), "{message}");
    assert!(
        message.contains(p.exporter.binary_for(LINUX).unwrap()),
        "the refusal names the pinned digest: {message}"
    );
}

/// With no exporter for the profile anywhere, the refusal says where it looked and how to build it.
#[test]
fn a_missing_registry_entry_names_the_install_script() {
    let dir = scratch();
    let empty = dir.path().join("registry");
    std::fs::create_dir_all(&empty).unwrap();
    let mut command = largo();
    command
        .args([
            "lean",
            "import",
            "--project",
            project("profile-4.33.1").to_str().unwrap(),
            "--target",
            "Profile.andSwap",
            "--out",
            dir.path().join("out").to_str().unwrap(),
        ])
        .current_dir(repo_root())
        .env("LOGOS_LEAN_EXPORTERS", &empty);
    let run = held(command.output().unwrap());
    assert_infrastructure_refusal(&run);
    let message = run.result["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|d| d["code"] == "infrastructure")
        .map(|d| d["message"].as_str().unwrap().to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        message.contains(empty.join("4.33.1").to_str().unwrap()),
        "{message}"
    );
    assert!(message.contains("install-lean-exporters.py"), "{message}");
}
