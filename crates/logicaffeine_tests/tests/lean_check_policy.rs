//! Track L6 — `largo lean check`: the assumption policy, the JSON document and statement binding.
//!
//! The kernel core admits any axiom. `--assumptions lean` holds what the declarations depend on —
//! with Lean's `#print axioms` semantics, through types and values, transitively — to Lean's own
//! policy: `propext`, `Quot.sound` and `Classical.choice`, each in its canonical statement, and
//! nothing else (`sorryAx`, `Lean.ofReduceBool`, `Lean.trustCompiler`, `lcProof`, any custom axiom,
//! a standard name restated). A refusal names the axiom and one path to it, and exits 3. An axiom
//! nothing depends on is no violation. `--json` writes one `logos-lean-check/1` document.
//! `--target T --bind ORIGINAL` holds the checked export's `T` to the original's statement: their
//! statement fingerprints must agree, however each export numbers its records.

mod lean_common;

use lean_common::*;
use serde_json::{json, Value};

fn profile_path(version: &str, stem: &str) -> std::path::PathBuf {
    fixtures()
        .join("profiles")
        .join(version)
        .join(format!("{stem}.ndjson"))
}

/// `largo lean check --json [args…] <export>`: the exit code and the one document on stdout.
fn check_json(export: &std::path::Path, args: &[&str]) -> (i32, Value) {
    let output = largo()
        .args(["lean", "check", "--json"])
        .args(args)
        .arg(export)
        .output()
        .unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    let mut documents = serde_json::Deserializer::from_str(&stdout).into_iter::<Value>();
    let document = documents
        .next()
        .unwrap_or_else(|| panic!("one document on stdout: {stdout:?}"))
        .unwrap();
    assert!(documents.next().is_none(), "exactly one document: {stdout}");
    let exit = output.status.code().unwrap();
    assert_eq!(document["schema"], "logos-lean-check/1");
    assert_eq!(document["exit"], exit, "{document}");
    (exit, document)
}

/// A minimal Lean 4.14.0 export: the axiom `axiom` : ∀ a : Prop, a, and — when `used` — the theorem
/// `Demo.usesIt : ∀ a : Prop, a := axiom`, plus the theorem `Demo.clean : ∀ a : Prop, a → a`.
fn synthetic(axiom: &str, used: bool) -> String {
    let mut rows = vec![
        json!({"meta":{"lean":{"version":"4.14.0","githash":"410fab7284703f41660ca2454218dcca9b2ec896"},"format":{"version":"3.1.0"},"exporter":{"version":"3.1.0","name":"lean4export"}}}),
    ];
    let mut next_name = 1;
    let mut name = |rows: &mut Vec<Value>, pre: u64, s: &str| {
        let i = next_name;
        next_name += 1;
        rows.push(json!({"str":{"str":s,"pre":pre},"in":i}));
        i
    };
    let mut axiom_name = 0;
    for part in axiom.split('.') {
        axiom_name = name(&mut rows, axiom_name, part);
    }
    let demo = name(&mut rows, 0, "Demo");
    let uses = name(&mut rows, demo, "usesIt");
    let clean = name(&mut rows, demo, "clean");
    let a = name(&mut rows, 0, "a");
    rows.push(json!({"ie":0,"sort":0}));
    rows.push(json!({"ie":1,"bvar":0}));
    rows.push(json!({"ie":2,"forallE":{"name":a,"type":0,"body":1,"binderInfo":"default"}}));
    rows.push(json!({"ie":3,"bvar":1}));
    rows.push(json!({"ie":4,"forallE":{"name":a,"type":1,"body":3,"binderInfo":"default"}}));
    rows.push(json!({"ie":5,"forallE":{"name":a,"type":0,"body":4,"binderInfo":"default"}}));
    rows.push(json!({"ie":6,"lam":{"name":a,"type":1,"body":1,"binderInfo":"default"}}));
    rows.push(json!({"ie":7,"lam":{"name":a,"type":0,"body":6,"binderInfo":"default"}}));
    rows.push(json!({"axiom":{"name":axiom_name,"levelParams":[],"type":2,"isUnsafe":false}}));
    rows.push(json!({"thm":{"name":clean,"levelParams":[],"type":5,"value":7,"all":[clean]}}));
    if used {
        rows.push(json!({"ie":8,"const":{"name":axiom_name,"us":[]}}));
        rows.push(json!({"thm":{"name":uses,"levelParams":[],"type":2,"value":8,"all":[uses]}}));
    }
    rows.iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}

// ============================================================================================
// THE POLICY
// ============================================================================================

/// ★ A sorry-backed proof is refused by the Lean policy, with the path from the declaration to the
/// admission — over the whole export, and when the policy is held to the target alone.
#[test]
fn the_lean_policy_refuses_a_sorry_backed_proof_and_names_its_path() {
    let export = profile_path("4.33.1", "Profile.statements");
    for args in [
        vec!["--assumptions", "lean"],
        vec!["--assumptions", "lean", "--target", "Profile.unbounded"],
    ] {
        let (exit, document) = check_json(&export, &args);
        assert_eq!(exit, 3, "{document}");
        assert_eq!(document["verdict"], "disallowed_assumptions");
        let refused: Vec<&Value> = document["assumptions"]["dependencies"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|d| !d["refused"].is_null())
            .collect();
        assert_eq!(refused.len(), 1, "{document}");
        assert_eq!(refused[0]["axiom"], "sorryAx");
        let path: Vec<&str> = refused[0]["path"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p.as_str().unwrap())
            .collect();
        assert_eq!(path.last(), Some(&"sorryAx"));
        assert!(path.len() >= 2, "{path:?}");
    }
    let (exit, document) = check_json(
        &export,
        &["--assumptions", "lean", "--target", "Profile.unbounded"],
    );
    assert_eq!(exit, 3);
    assert_eq!(
        document["assumptions"]["dependencies"][0]["path"],
        json!(["Profile.unbounded", "sorryAx"])
    );
}

/// Without a policy the same export is accepted, and the dependencies are still reported.
#[test]
fn without_a_policy_the_dependencies_are_reported_not_refused() {
    let (exit, document) = check_json(&profile_path("4.33.1", "Profile.statements"), &[]);
    assert_eq!(exit, 0, "{document}");
    assert_eq!(document["verdict"], "accepted");
    assert_eq!(document["assumptions"]["policy"], "any");
    let dependencies = document["assumptions"]["dependencies"].as_array().unwrap();
    assert!(dependencies
        .iter()
        .any(|d| d["axiom"] == "sorryAx" && d["refused"].is_null()));
}

/// Lean's three standard axioms, in their canonical statements, are admitted and reported.
#[test]
fn the_three_standard_axioms_are_admitted_and_reported() {
    let (exit, document) = check_json(&export("Excluded.middle"), &["--assumptions", "lean"]);
    assert_eq!(exit, 0, "{document}");
    let used: Vec<&str> = document["assumptions"]["dependencies"]
        .as_array()
        .unwrap()
        .iter()
        .inspect(|d| assert!(d["refused"].is_null(), "{d}"))
        .map(|d| d["axiom"].as_str().unwrap())
        .collect();
    assert_eq!(used, vec!["Classical.choice", "Quot.sound", "propext"]);
    assert_eq!(
        document["assumptions"]["allowed"],
        json!(["propext", "Quot.sound", "Classical.choice"])
    );
}

/// Every escape the policy names — and any custom axiom — is refused once something depends on it;
/// a standard axiom's name declared with another statement is refused too.
#[test]
fn every_other_axiom_and_a_restated_standard_one_is_refused() {
    let dir = scratch();
    for (axiom, reason) in [
        ("sorryAx", "admission"),
        ("Lean.ofReduceBool", "native-code decision"),
        ("Lean.ofReduceNat", "native-code decision"),
        ("Lean.trustCompiler", "compiler-trust"),
        ("lcProof", "proof placeholder"),
        ("Demo.cheat", "outside Lean's standard three"),
        ("propext", "canonical"),
        ("Classical.choice", "canonical"),
    ] {
        let path = write_export(
            dir.path(),
            &format!("{axiom}.ndjson"),
            &synthetic(axiom, true),
        );
        let (exit, document) = check_json(&path, &["--assumptions", "lean"]);
        assert_eq!(exit, 3, "{axiom}: {document}");
        let dependency = &document["assumptions"]["dependencies"][0];
        assert_eq!(dependency["axiom"], axiom);
        assert!(
            dependency["refused"].as_str().unwrap().contains(reason),
            "{axiom}: {dependency}"
        );
        assert_eq!(dependency["path"], json!(["Demo.usesIt", axiom]));
    }
}

/// An axiom the export declares but nothing depends on is no violation; it is listed as unused.
#[test]
fn an_axiom_nothing_depends_on_is_no_violation() {
    let dir = scratch();
    let path = write_export(dir.path(), "unused.ndjson", &synthetic("sorryAx", false));
    let (exit, document) = check_json(&path, &["--assumptions", "lean"]);
    assert_eq!(exit, 0, "{document}");
    assert_eq!(document["assumptions"]["unused"], json!(["sorryAx"]));
    assert_eq!(document["assumptions"]["dependencies"], json!([]));
}

/// A target the export does not declare is a rejection, not an empty audit.
#[test]
fn a_target_the_export_does_not_declare_is_rejected() {
    let (exit, document) = check_json(
        &profile_path("4.33.1", "Profile.statements"),
        &["--assumptions", "lean", "--target", "Profile.nowhere"],
    );
    assert_eq!(exit, 1, "{document}");
    assert!(document["message"]
        .as_str()
        .unwrap()
        .contains("Profile.nowhere"));
}

/// The Arena protocol is unchanged: no flag, no document — the verdict on stderr and its code.
#[test]
fn the_protocol_without_flags_is_unchanged() {
    let output = largo()
        .args(["lean", "check"])
        .arg(profile_path("4.33.1", "Profile.statements"))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).starts_with("accepted: "));
}

// ============================================================================================
// STATEMENT BINDING
// ============================================================================================

/// ★ A statement binds to its own original, and to the same statement exported by another release —
/// a fingerprint does not depend on how an export numbers its records.
#[test]
fn a_statement_binds_to_the_same_proposition() {
    let ours = profile_path("4.33.1", "Profile.statements");
    for original in [ours.clone(), profile_path("4.34.1", "Profile.statements")] {
        let (exit, document) = check_json(
            &ours,
            &[
                "--target",
                "Profile.unbounded",
                "--bind",
                original.to_str().unwrap(),
            ],
        );
        assert_eq!(exit, 0, "{document}");
        assert_eq!(document["binding"]["bound"], true, "{document}");
        assert_eq!(
            document["binding"]["fingerprint"],
            document["binding"]["original_fingerprint"]
        );
        assert!(document["binding"]["fingerprint"]
            .as_str()
            .unwrap()
            .starts_with("sha256:"));
    }
}

/// ★ A wrong but well-typed proposition does not bind: an original whose `Profile.unfinished`
/// states another (checked) proposition — `Profile.unbounded`'s, defined earlier in the export — is
/// not what the checked export's `Profile.unfinished` states, and the check exits 1.
#[test]
fn a_different_proposition_does_not_bind() {
    let dir = scratch();
    let text = std::fs::read_to_string(profile_path("4.33.1", "Profile.statements")).unwrap();
    let unbounded = name_index(&text, "Profile.unbounded");
    let unfinished = name_index(&text, "Profile.unfinished");
    let rows: Vec<Value> = text
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let other_type = rows
        .iter()
        .find(|r| r["thm"]["name"] == unbounded)
        .unwrap()["thm"]["type"]
        .clone();
    let forged: String = rows
        .into_iter()
        .map(|mut r| {
            if r["thm"]["name"] == unfinished {
                r["thm"]["type"] = other_type.clone();
            }
            r.to_string() + "\n"
        })
        .collect();
    let original = write_export(dir.path(), "forged.ndjson", &forged);
    let (exit, document) = check_json(
        &profile_path("4.33.1", "Profile.statements"),
        &[
            "--target",
            "Profile.unfinished",
            "--bind",
            original.to_str().unwrap(),
        ],
    );
    assert_eq!(exit, 1, "{document}");
    assert_eq!(document["verdict"], "unbound");
    assert_eq!(document["binding"]["bound"], false, "{document}");
    assert!(document["binding"]["fingerprint"].is_string(), "{document}");
    assert!(document["binding"]["original_fingerprint"].is_string());
    assert_ne!(
        document["binding"]["fingerprint"],
        document["binding"]["original_fingerprint"]
    );
}

/// A binding needs exactly one target, and an original that states nothing is a binding that
/// cannot be established.
#[test]
fn a_binding_without_one_target_or_a_statement_is_refused() {
    let export = profile_path("4.33.1", "Profile.statements");
    let output = largo()
        .args(["lean", "check", "--bind"])
        .arg(&export)
        .arg("--target")
        .arg("Profile.unbounded")
        .arg("--target")
        .arg("Profile.answered")
        .arg(&export)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(64));
    let (exit, document) = check_json(
        &export,
        &["--target", "Nat", "--bind", export.to_str().unwrap()],
    );
    assert_eq!(exit, 1, "{document}");
    assert_eq!(document["binding"]["bound"], false);
}

// ============================================================================================
// EVERY PROFILE
// ============================================================================================

/// The kernel core accepts every registered profile's own exports — a string literal included,
/// which each release's kernel reads through its own `String` (`String.mk` in Lean 4.14.0,
/// `String.ofList` from 4.28.0 on).
#[test]
fn the_kernel_core_accepts_every_profiles_exports() {
    for version in ["4.14.0", "4.28.0", "4.33.1", "4.34.0-rc1", "4.34.1"] {
        for stem in [
            "Profile.andSwap",
            "Profile.doubleTwo",
            "Profile.greetingIsHi",
            "Profile.addZeroViaHave",
            "Profile.IsEven",
            "Profile.EveryDoubleIsEven",
            "Profile.statements",
        ] {
            let (exit, document) = check_json(&profile_path(version, stem), &[]);
            assert_eq!(exit, 0, "{version} {stem}: {document}");
            assert_eq!(document["verdict"], "accepted", "{version} {stem}");
        }
    }
}
