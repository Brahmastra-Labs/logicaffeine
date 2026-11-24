//! Track L1 — the versioned Lean profile registry.
//!
//! A result is produced under exactly one registered profile: a Lean release, its toolchain, the
//! one lean4export revision (structurally patched after 4.14.0) that exports it, pinned by the
//! SHA-256 of every source and of the built executable, and the assumptions a result may rest on.
//! An export is admitted only under the profile its header names — and only when the header names
//! that release exactly, its commit included, and the export's own `String` is that release's. An
//! unknown release, a header retagged to another release, or a pin outside the registry is a typed
//! `unsupported_toolchain` refusal, never a read under the wrong rules.
//!
//! The registry is held to its build recipes (`scripts/lean/lean-exporters.json`, the structural
//! patch beside it) and to the per-profile fixtures, each produced by its own profile's toolchain
//! and exporter (`scripts/lean/export-profile-fixtures.py`, `fixtures/lean/profiles/PROVENANCE.json`).

mod lean_common;

use lean_common::*;
use logicaffeine_kernel::interface::lean::profile::{self, ExporterForm, StringLiterals};
use logicaffeine_kernel::interface::lean::{intake, DiagnosticCode, Verdict};
use serde_json::Value;
use std::collections::BTreeMap;

const REGISTERED: [&str; 5] = ["4.14.0", "4.28.0", "4.33.1", "4.34.0-rc1", "4.34.1"];
const AXIOMS: [&str; 3] = ["propext", "Quot.sound", "Classical.choice"];
const LINUX: &str = "x86_64-unknown-linux-gnu";

/// Every export of the per-profile fixture project and the targets it is the closure of, with
/// whether Lean's `#print axioms` reports an admission for each.
const EXPORTS: [(&str, &[(&str, bool)]); 7] = [
    ("Profile.andSwap", &[("Profile.andSwap", false)]),
    ("Profile.doubleTwo", &[("Profile.doubleTwo", false)]),
    ("Profile.greetingIsHi", &[("Profile.greetingIsHi", false)]),
    (
        "Profile.addZeroViaHave",
        &[("Profile.addZeroViaHave", false)],
    ),
    ("Profile.IsEven", &[("Profile.IsEven", false)]),
    (
        "Profile.EveryDoubleIsEven",
        &[("Profile.EveryDoubleIsEven", false)],
    ),
    (
        "Profile.statements",
        &[
            ("Profile.unbounded", true),
            ("Profile.answered", true),
            ("Profile.unfinished", true),
            ("Profile.RestsOnUnfinished", true),
        ],
    ),
];

/// The export of `target` that `version`'s own toolchain and exporter produced.
fn profile_fixture(version: &str, target: &str) -> String {
    std::fs::read_to_string(
        fixtures()
            .join("profiles")
            .join(version)
            .join(format!("{target}.ndjson")),
    )
    .unwrap_or_else(|e| panic!("the {version} {target} profile fixture: {e}"))
}

/// A small export of each profile: a proof over `And`.
fn small(version: &str) -> String {
    profile_fixture(version, "Profile.andSwap")
}

/// An export holding Lean core's `String`, which differs between releases.
fn with_string(version: &str) -> String {
    profile_fixture(version, "Profile.greetingIsHi")
}

fn json_file(path: &std::path::Path) -> Value {
    serde_json::from_str(
        &std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display())),
    )
    .unwrap_or_else(|e| panic!("{} is not JSON: {e}", path.display()))
}

fn recipes() -> Value {
    json_file(&repo_root().join("scripts/lean/lean-exporters.json"))
}

/// `text` with its header replaced by `f(header)`.
fn retag(text: &str, f: impl FnOnce(&mut Value)) -> String {
    let (first, rest) = text.split_once('\n').unwrap();
    let mut header: Value = serde_json::from_str(first).unwrap();
    f(&mut header);
    format!("{header}\n{rest}")
}

fn set_release(text: &str, version: &str, githash: &str) -> String {
    retag(text, |h| {
        h["meta"]["lean"]["version"] = version.into();
        h["meta"]["lean"]["githash"] = githash.into();
    })
}

fn refusal(result: Result<intake::Intake, Verdict>) -> (DiagnosticCode, String) {
    match result {
        Ok(read) => panic!(
            "admitted under {:?}, expected a refusal",
            read.profile().lean_version
        ),
        Err(verdict) => (
            verdict.reason().code(),
            verdict.reason().message().to_string(),
        ),
    }
}

// ============================================================================================
// THE REGISTRY ITSELF
// ============================================================================================

#[test]
fn the_registry_holds_exactly_the_supported_releases() {
    let versions: Vec<&str> = profile::profiles().iter().map(|p| p.lean_version).collect();
    assert_eq!(versions, REGISTERED, "oldest release first");
    for version in REGISTERED {
        let p = profile::by_version(version).expect("registered");
        assert_eq!(p.lean_version, version);
    }
    for unknown in [
        "4.29.1", "4.30.0", "4.33.0", "4.34.0", "4.15.0", "5.0.0", "", "v4.33.1",
    ] {
        assert!(
            profile::by_version(unknown).is_none(),
            "{unknown:?} is no profile"
        );
    }
}

#[test]
fn every_profile_is_a_known_release_with_its_own_commit() {
    let mut commits = std::collections::BTreeSet::new();
    for p in profile::profiles() {
        let release = profile::release(p.lean_version).expect("every profile is a release");
        assert_eq!(release.githash, p.lean_githash, "{}", p.lean_version);
        assert_eq!(p.release(), release);
        assert_eq!(
            p.lean_githash.len(),
            40,
            "{} pins a full commit",
            p.lean_version
        );
        assert!(p
            .lean_githash
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)));
        assert!(commits.insert(p.lean_githash), "commits are distinct");
        assert_eq!(p.toolchain, format!("leanprover/lean4:v{}", p.lean_version));
        assert_eq!(p.format_version, "3.1.0");
        assert_eq!(
            (p.exporter.name, p.exporter.version),
            ("lean4export", "3.1.0")
        );
        assert_eq!(
            p.exporter.repository,
            "https://github.com/leanprover/lean4export"
        );
        assert_eq!(p.exporter.flags, ["--export-mdata", "--export-unsafe"]);
        assert_eq!(p.allowed_axioms, AXIOMS, "{}", p.lean_version);
        assert_eq!(p.exporter.revision.len(), 40);
        assert_eq!(p.exporter.archive_sha256.len(), 64);
    }
    let releases: Vec<&str> = profile::RELEASES.iter().map(|r| r.version).collect();
    let mut sorted = releases.clone();
    sorted.dedup();
    assert_eq!(sorted.len(), releases.len(), "a release is listed once");
}

/// Each release's string literal reading is its own `Init.Prelude`'s: `String.mk` over `List Char`
/// through 4.14.0, `String.ofList` over UTF-8 bytes from 4.28.0 on.
#[test]
fn every_release_reads_string_literals_as_its_own_prelude_defines_string() {
    for release in profile::RELEASES {
        let expected = if release.version == "4.14.0" {
            StringLiterals::ListOfChars
        } else {
            StringLiterals::Utf8Bytes
        };
        assert_eq!(release.string_literals, expected, "{}", release.version);
    }
    assert_eq!(
        StringLiterals::ListOfChars.packing_constant(),
        "/s:\"String\"/s:\"mk\""
    );
    assert_eq!(
        StringLiterals::ListOfChars.string_constructor(),
        "/s:\"String\"/s:\"mk\""
    );
    assert_eq!(
        StringLiterals::Utf8Bytes.packing_constant(),
        "/s:\"String\"/s:\"ofList\""
    );
    assert_eq!(
        StringLiterals::Utf8Bytes.string_constructor(),
        "/s:\"String\"/s:\"ofByteArray\""
    );
}

/// The kernel core reads string literals by the same release facts: its list-of-characters releases
/// are exactly the registry's.
#[test]
fn the_kernel_core_reads_string_literals_by_the_same_releases() {
    let registry: Vec<&str> = profile::RELEASES
        .iter()
        .filter(|r| r.string_literals == StringLiterals::ListOfChars)
        .map(|r| r.version)
        .collect();
    assert_eq!(
        logicaffeine_kernel::dag::export::LIST_CHAR_STRING_RELEASES,
        registry.as_slice()
    );
}

/// Stage A is the 4.14.0 profile, unchanged: lean4export 20c4a64a run from its sources, no patch,
/// and the constants every existing consumer reads are that profile's.
#[test]
fn stage_a_is_the_unchanged_lean_4_14_profile() {
    let stage_a = profile::by_version("4.14.0").unwrap();
    assert_eq!(*stage_a, intake::STAGE_A);
    assert_eq!(
        stage_a.lean_githash,
        "410fab7284703f41660ca2454218dcca9b2ec896"
    );
    assert_eq!(stage_a.exporter.form, ExporterForm::Interpreted);
    assert_eq!(stage_a.exporter.structural_patch_sha256, None);
    assert!(stage_a.exporter.binaries.is_empty());
    assert_eq!(intake::TOOLCHAIN, "leanprover/lean4:v4.14.0");
    assert_eq!(intake::TOOLCHAIN, stage_a.toolchain);
    assert_eq!(
        intake::EXPORTER_REVISION,
        "20c4a64aa1b76f7c79a6db15d089084359f8c3dc"
    );
    assert_eq!(intake::EXPORTER_REVISION, stage_a.exporter.revision);
    assert_eq!(
        intake::EXPORTER_SOURCES.as_slice(),
        stage_a.exporter.sources
    );
    assert_eq!(
        intake::EXPORTER_SOURCES,
        [
            (
                "Main.lean",
                "bd8e350903f1382b25fe767f9086dd9882000f01085f7046ea06545e7848842a"
            ),
            (
                "Export.lean",
                "16fa1fec370500f4ca03f042fc3e65cac27839b640b32c5a921e085206c7031b"
            ),
        ]
    );
    assert_eq!(intake::EXPORTER_FLAGS.as_slice(), stage_a.exporter.flags);
}

/// Every release after 4.14.0 is exported by the structurally patched executable: binder names and
/// annotations survive, and the executable is pinned for this platform by its reproducible digest.
#[test]
fn every_later_profile_pins_a_structural_executable_exporter() {
    let patch = sha256_hex(
        &std::fs::read(repo_root().join("scripts/lean/lean4export-structural.json")).unwrap(),
    );
    for p in &profile::profiles()[1..] {
        assert_eq!(
            p.exporter.form,
            ExporterForm::Executable,
            "{}",
            p.lean_version
        );
        assert_eq!(
            p.exporter.structural_patch_sha256,
            Some(patch.as_str()),
            "{}",
            p.lean_version
        );
        let names: Vec<&str> = p.exporter.sources.iter().map(|(n, _)| *n).collect();
        for required in [
            "Export.lean",
            "Export/Parse.lean",
            "Main.lean",
            "lake-manifest.json",
            "lakefile.toml",
            "lean-toolchain",
        ] {
            assert!(
                names.contains(&required),
                "{} pins {required}",
                p.lean_version
            );
        }
        let toolchain = p
            .exporter
            .sources
            .iter()
            .find(|(n, _)| *n == "lean-toolchain")
            .unwrap()
            .1;
        assert_eq!(
            toolchain,
            sha256_hex(format!("{}\n", p.toolchain).as_bytes()),
            "{}'s exporter is built by its own toolchain",
            p.lean_version
        );
        let binary = p.exporter.binary_for(LINUX).expect("pinned on Linux");
        assert_eq!(binary.len(), 64);
        assert!(p.exporter.binary_for("aarch64-apple-darwin").is_none());
        let identity = p.exporter_identity(LINUX);
        assert!(identity.contains(p.exporter.revision), "{identity}");
        assert!(identity.contains(&patch), "{identity}");
        assert!(identity.contains(binary), "{identity}");
        assert!(!p
            .exporter_identity("aarch64-apple-darwin")
            .contains("sha256:"));
    }
    let binaries: std::collections::BTreeSet<&str> = profile::profiles()[1..]
        .iter()
        .map(|p| p.exporter.binary_for(LINUX).unwrap())
        .collect();
    assert_eq!(binaries.len(), 4, "every release builds its own executable");
}

/// The build recipes the install script reads are the registry, field for field: neither can
/// change without the other.
#[test]
fn the_build_recipes_are_the_registry() {
    let recipes = recipes();
    assert_eq!(recipes["schema"], "logos-lean-exporter-recipes/1");
    assert_eq!(
        recipes["repository"],
        "https://github.com/leanprover/lean4export"
    );
    let patch_file = repo_root()
        .join("scripts/lean")
        .join(recipes["structural_patch"].as_str().unwrap());
    let patch_digest = sha256_hex(&std::fs::read(&patch_file).unwrap());
    assert_eq!(recipes["structural_patch_sha256"], patch_digest.as_str());
    let exporters = recipes["exporters"].as_object().unwrap();
    let mut listed: Vec<&str> = exporters.keys().map(String::as_str).collect();
    listed.sort();
    let mut registered = REGISTERED.to_vec();
    registered.sort();
    assert_eq!(listed, registered, "one recipe per profile, and no other");
    for p in profile::profiles() {
        let r = &exporters[p.lean_version];
        let v = p.lean_version;
        assert_eq!(r["toolchain"], p.toolchain, "{v}");
        assert_eq!(r["lean_githash"], p.lean_githash, "{v}");
        assert_eq!(r["revision"], p.exporter.revision, "{v}");
        assert_eq!(r["archive_sha256"], p.exporter.archive_sha256, "{v}");
        assert_eq!(
            r["upstream_tag"],
            p.exporter.upstream_tag.map_or(Value::Null, Value::from),
            "{v}"
        );
        assert_eq!(
            r["structural"],
            p.exporter.structural_patch_sha256.is_some(),
            "{v}"
        );
        assert_eq!(r["form"], p.exporter.form.as_str(), "{v}");
        let sources: BTreeMap<&str, &str> = r["sources"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, s)| (k.as_str(), s.as_str().unwrap()))
            .collect();
        for (name, digest) in p.exporter.sources {
            assert_eq!(sources.get(name), Some(digest), "{v} {name}");
        }
        if p.exporter.form == ExporterForm::Executable {
            assert_eq!(
                sources.len(),
                p.exporter.sources.len(),
                "{v}: the recipe pins exactly the registry's sources"
            );
        }
        let binaries: BTreeMap<&str, &str> = r["binary"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, s)| (k.as_str(), s.as_str().unwrap()))
            .collect();
        let pinned: BTreeMap<&str, &str> = p.exporter.binaries.iter().copied().collect();
        assert_eq!(binaries, pinned, "{v}");
    }
}

/// The structural patch is five exact replacements in `Export.lean`, each keying an expression table
/// by `ExprStructEq` — nothing else.
#[test]
fn the_structural_patch_keys_every_expression_table_structurally() {
    let patch = json_file(&repo_root().join("scripts/lean/lean4export-structural.json"));
    assert_eq!(patch["schema"], "logos-lean4export-structural-patch/1");
    assert_eq!(patch["file"], "Export.lean");
    let replacements = patch["replacements"].as_array().unwrap();
    assert_eq!(replacements.len(), 5);
    for pair in replacements {
        let (before, after) = (pair[0].as_str().unwrap(), pair[1].as_str().unwrap());
        assert!(!before.contains("ExprStructEq"), "{before}");
        assert!(after.contains("ExprStructEq"), "{after}");
    }
}

// ============================================================================================
// ADMISSION
// ============================================================================================

/// Every profile fixture is its own profile's output: its provenance names the registry's exporter,
/// its bytes are the recorded ones, and its header is admitted under exactly that profile.
#[test]
fn every_profile_fixture_was_produced_by_its_own_profile() {
    let provenance = json_file(&fixtures().join("profiles/PROVENANCE.json"));
    let exports = provenance["exports"].as_object().unwrap();
    assert_eq!(exports.len(), REGISTERED.len() * EXPORTS.len());
    for p in profile::profiles() {
        let v = p.lean_version;
        for (stem, targets) in EXPORTS {
            let entry = &exports[&format!("{v}/{stem}.ndjson")];
            let text = profile_fixture(v, stem);
            assert_eq!(
                entry["export_sha256"],
                sha256_hex(text.as_bytes()),
                "{v} {stem}"
            );
            let recorded: Vec<&str> = entry["targets"]
                .as_array()
                .unwrap()
                .iter()
                .map(|t| t.as_str().unwrap())
                .collect();
            assert_eq!(
                recorded,
                targets.iter().map(|(t, _)| *t).collect::<Vec<_>>(),
                "{v} {stem}"
            );
            for ((target, admits), reported) in targets
                .iter()
                .zip(entry["print_axioms"].as_array().unwrap())
            {
                let reported = reported.as_str().unwrap();
                if *admits {
                    assert!(
                        reported.contains(&format!("'{target}' depends on axioms: [sorryAx]")),
                        "{v} {target}: {reported}"
                    );
                } else {
                    assert!(
                        reported.contains(&format!("'{target}' does not depend on any axioms")),
                        "{v} {target}: {reported}"
                    );
                }
            }
            assert_eq!(entry["lean_version"], v);
            assert_eq!(entry["toolchain"], p.toolchain);
            assert_eq!(entry["exporter"]["revision"], p.exporter.revision, "{v}");
            assert_eq!(entry["exporter"]["form"], p.exporter.form.as_str(), "{v}");
            assert_eq!(
                entry["exporter"]["binary_sha256"],
                p.exporter
                    .binary_for(LINUX)
                    .map_or(Value::Null, Value::from),
                "{v}"
            );
            assert!(
                entry["lean_reported"]
                    .as_str()
                    .unwrap()
                    .contains(&format!("version {v},")),
                "{v}"
            );
            let read = intake::read_admitted(&text)
                .unwrap_or_else(|e| panic!("{v} {stem} is not admitted: {:?}", e.reason()));
            assert_eq!(read.profile().lean_version, v);
            assert_eq!(read.header().lean_githash, p.lean_githash);
            for (target, _) in targets.iter() {
                let kernel = intake::kernel_name(target).unwrap();
                assert!(read.declaration(&kernel).is_some(), "{v} declares {target}");
                let closure = read.closure(&kernel).unwrap();
                assert_eq!(
                    closure.last(),
                    Some(&kernel),
                    "{v}: {target} closes its own closure, in dependency order"
                );
            }
        }
    }
}

/// A profile's export is admitted under that profile and refused under every other one.
#[test]
fn each_export_is_admitted_under_exactly_its_own_profile() {
    for p in profile::profiles() {
        let text = with_string(p.lean_version);
        intake::read(&text, p).unwrap_or_else(|e| {
            panic!("{} under its own profile: {:?}", p.lean_version, e.reason())
        });
        for other in profile::profiles() {
            if other.lean_version == p.lean_version {
                continue;
            }
            let (code, message) = refusal(intake::read(&text, other));
            assert_eq!(
                code,
                DiagnosticCode::UnsupportedToolchain,
                "{} under {}: {message}",
                p.lean_version,
                other.lean_version
            );
        }
    }
}

/// A release with no registered profile is refused before a record is read, with the typed
/// diagnostic and a message naming the release and every registered profile.
#[test]
fn an_unknown_release_is_refused_with_a_typed_diagnostic() {
    let base = small("4.33.1");
    for version in [
        "4.29.1", "4.30.0", "4.33.0", "4.34.0", "4.15.0", "5.0.0", "", "4.33.1 ",
    ] {
        let text = retag(&base, |h| h["meta"]["lean"]["version"] = version.into());
        let (code, message) = refusal(intake::read_admitted(&text));
        assert_eq!(
            code,
            DiagnosticCode::UnsupportedToolchain,
            "{version:?}: {message}"
        );
        assert!(message.contains(&format!("Lean {version}")), "{message}");
        for registered in REGISTERED {
            assert!(message.contains(registered), "{message} lists {registered}");
        }
    }
    for (field, value) in [("name", "other-exporter"), ("version", "3.0.0")] {
        let text = retag(&base, |h| h["meta"]["exporter"][field] = value.into());
        let (code, message) = refusal(intake::read_admitted(&text));
        assert_eq!(
            code,
            DiagnosticCode::UnsupportedToolchain,
            "{field}: {message}"
        );
    }
    let text = retag(&base, |h| h["meta"]["format"]["version"] = "3.2.0".into());
    let (code, _) = refusal(intake::read_admitted(&text));
    assert_eq!(code, DiagnosticCode::UnsupportedExportVersion);
}

/// A header naming one release with another's commit — or with no release's commit — is retagged,
/// and refused: an export is never read under rules its producer did not follow.
#[test]
fn a_header_retagged_to_another_release_is_refused() {
    for p in profile::profiles() {
        let text = small(p.lean_version);
        for other in profile::profiles() {
            if other.lean_version == p.lean_version {
                continue;
            }
            // The release renamed, its own commit kept.
            let renamed = retag(&text, |h| {
                h["meta"]["lean"]["version"] = other.lean_version.into()
            });
            let (code, message) = refusal(intake::read_admitted(&renamed));
            assert_eq!(code, DiagnosticCode::UnsupportedToolchain, "{message}");
            assert!(message.contains("retagged"), "{message}");
            assert!(
                message.contains(&format!("which is Lean {}'s", p.lean_version)),
                "the refusal names whose commit it is: {message}"
            );
            // The commit replaced by another release's, the release kept.
            let recommitted = retag(&text, |h| {
                h["meta"]["lean"]["githash"] = other.lean_githash.into()
            });
            let (code, message) = refusal(intake::read_admitted(&recommitted));
            assert_eq!(code, DiagnosticCode::UnsupportedToolchain, "{message}");
            assert!(message.contains("retagged"), "{message}");
        }
        for githash in [
            String::new(),
            p.lean_githash[..12].to_string(),
            p.lean_githash.to_uppercase(),
            "0".repeat(40),
            format!("{} ", p.lean_githash),
        ] {
            let text = retag(&text, |h| {
                h["meta"]["lean"]["githash"] = githash.clone().into()
            });
            let (code, message) = refusal(intake::read_admitted(&text));
            assert_eq!(
                code,
                DiagnosticCode::UnsupportedToolchain,
                "{githash:?}: {message}"
            );
        }
    }
}

/// A header retagged completely — release and commit both another's — is caught by the export's own
/// `String`, which is the producing release's: `String.mk` through 4.14.0, `String.ofByteArray`
/// after.
#[test]
fn a_fully_retagged_header_is_caught_by_the_exports_own_string() {
    for p in profile::profiles() {
        let text = with_string(p.lean_version);
        for other in profile::profiles() {
            let literals_differ = other.release().string_literals != p.release().string_literals;
            if other.lean_version == p.lean_version || !literals_differ {
                continue;
            }
            let forged = set_release(&text, other.lean_version, other.lean_githash);
            let (code, message) = refusal(intake::read_admitted(&forged));
            assert_eq!(
                code,
                DiagnosticCode::UnsupportedToolchain,
                "{} forged as {}: {message}",
                p.lean_version,
                other.lean_version
            );
            assert!(message.contains("String"), "{message}");
            assert!(
                message.contains(p.release().string_literals.string_constructor())
                    || message.contains(&intake::lean_name(
                        p.release().string_literals.string_constructor()
                    )),
                "the refusal names the export's own String constructor: {message}"
            );
        }
    }
}

// ============================================================================================
// PROJECT PINS
// ============================================================================================

#[test]
fn a_toolchain_pin_selects_its_profile() {
    for p in profile::profiles() {
        let v = p.lean_version;
        for pin in [
            format!("leanprover/lean4:v{v}"),
            format!("leanprover/lean4:v{v}\n"),
            format!("  leanprover/lean4:v{v}\r\n"),
            format!("leanprover/lean4:{v}"),
        ] {
            let selected = profile::for_toolchain(&pin)
                .unwrap_or_else(|e| panic!("{pin:?}: {:?}", e.reason()));
            assert_eq!(selected.lean_version, v, "{pin:?}");
        }
    }
}

#[test]
fn a_toolchain_pin_outside_the_registry_is_refused() {
    for pin in [
        "leanprover/lean4:v4.29.1",
        "leanprover/lean4:v4.34.0",
        "leanprover/lean4:nightly",
        "leanprover/lean4:nightly-2026-11-01",
        "leanprover/lean4-pr-releases:pr-release-1234",
        "lean4:v4.33.1",
        "leanprover/lean4:vv4.33.1",
        "leanprover/lean4:v4.33.1\nleanprover/lean4:v4.34.1",
        "",
        "   \n",
        "stable",
    ] {
        match profile::for_toolchain(pin) {
            Ok(p) => panic!("{pin:?} selected {}", p.lean_version),
            Err(verdict) => {
                assert_eq!(
                    verdict.reason().code(),
                    DiagnosticCode::UnsupportedToolchain,
                    "{pin:?}"
                );
                let message = verdict.reason().message();
                for registered in REGISTERED {
                    assert!(message.contains(registered), "{message}");
                }
            }
        }
    }
}
