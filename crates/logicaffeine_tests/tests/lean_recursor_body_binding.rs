//! A well-typed recursor replacement must still be the source's canonical implementation.
mod lean_package_support;

use lean_package_support::Export;
use logicaffeine_kernel::certificate::{EnvironmentCertificate, EnvironmentEntry, TermNode};
use logicaffeine_kernel::interface::lean::package::{self, BuildOptions, Manifest, PackageFiles};
use logicaffeine_kernel::interface::lean::receipt::{Receipt, StageStatus};
use logicaffeine_kernel::Universe;
use serde_json::json;

const ROOT: &str = "RecursorBinding.root";
const REC: &str = "/s:\"RecursorBinding\"/s:\"Token\"/s:\"rec\"";
const HIDDEN: &str = "/s:\"RecursorBinding\"/s:\"hidden\"";

/// An axiom-free singleton proposition and a sealed inhabitant of its eliminator type.
/// The opaque implementation uses proof irrelevance, not the canonical recursor:
///   opaque hidden.{u} (m : Token → Sort u) (h : m Token.intro) (t : Token) : m t := h
/// Root retains BOTH eliminators as higher-order arguments, without relying on either one's
/// computation rules. Thus replacing the recursor does not break root's kernel typing.
fn source() -> String {
    let mut e = Export::new();
    e.lines[0] = include_str!("fixtures/lean_package/W05.clean.ndjson")
        .lines()
        .next()
        .unwrap()
        .to_string();
    let ns = e.name("RecursorBinding", 0);
    let token = e.name("Token", ns);
    let intro = e.name("intro", token);
    let rec = e.name("rec", token);
    let hidden = e.name("hidden", ns);
    let root = e.name("root", ns);
    let u_name = e.name("u", 0);
    let u = e.param(u_name);
    let m = e.name("motive", 0);
    let h = e.name("minor", 0);
    let t = e.name("major", 0);
    let f = e.name("f", 0);
    let first = e.name("first", 0);
    let second = e.name("second", 0);
    let prop = e.sort(0);
    let sort_u = e.sort(u);
    let token_c = e.constant(token, &[]);
    let intro_c = e.constant(intro, &[]);
    let zero = e.bvar(0);
    let one = e.bvar(1);
    let two = e.bvar(2);
    let motive_type = e.forall(t, token_c, sort_u);
    let minor_type = e.app(zero, intro_c); // under motive
    let result = e.app(two, zero); // under motive, minor, major
    let major_type = e.forall(t, token_c, result);
    let minors = e.forall(h, minor_type, major_type);
    let rec_type = e.forall(m, motive_type, minors);
    let rule_minor = e.lam(h, minor_type, zero);
    let rule = e.lam(m, motive_type, rule_minor);
    e.raw(
        json!({"inductive": {
            "types": [{"name": token, "type": prop, "levelParams": [],
                "numParams": 0, "numIndices": 0, "numNested": 0,
                "isUnsafe": false, "isReflexive": false, "isRec": false,
                "ctors": [intro], "all": [token]}],
            "ctors": [{"name": intro, "type": token_c, "levelParams": [],
                "numParams": 0, "numFields": 0, "isUnsafe": false,
                "induct": token, "cidx": 0}],
            "recs": [{"name": rec, "type": rec_type, "levelParams": [u_name],
                "numParams": 0, "numIndices": 0, "numMotives": 1, "numMinors": 1,
                "k": true, "isUnsafe": false, "all": [token],
                "rules": [{"rhs": rule, "nfields": 0, "ctor": intro}]}]
        }})
        .to_string(),
    );
    let hidden_major = e.lam(t, token_c, one);
    let hidden_minor = e.lam(h, minor_type, hidden_major);
    let hidden_body = e.lam(m, motive_type, hidden_minor);
    e.raw(
        json!({"opaque": {"name": hidden, "type": rec_type,
        "value": hidden_body, "levelParams": [u_name], "isUnsafe": false,
        "all": [hidden]}})
        .to_string(),
    );
    // root.{u} (f : RecType.{u} → RecType.{u} → Prop) : Prop := f Token.rec hidden
    let function_tail = e.forall(second, rec_type, prop);
    let function_type = e.forall(first, rec_type, function_tail);
    let root_type = e.forall(f, function_type, prop);
    let rec_c = e.constant(rec, &[u]);
    let hidden_c = e.constant(hidden, &[u]);
    let apply_rec = e.app(zero, rec_c);
    let apply_hidden = e.app(apply_rec, hidden_c);
    let root_body = e.lam(f, function_type, apply_hidden);
    e.raw(
        json!({"def": {"name": root, "type": root_type, "value": root_body,
        "levelParams": [u_name], "safety": "safe", "hints": "abbrev",
        "all": [root]}})
        .to_string(),
    );
    e.text()
}

fn require_stages(receipt: &Receipt, stages: &[&str]) {
    for stage in stages {
        assert_eq!(
            receipt.status(stage),
            StageStatus::Passed,
            "{stage}: {receipt:#?}"
        );
    }
}

fn entry_at(c: &EnvironmentCertificate, name: &str) -> usize {
    c.entries
        .iter()
        .position(|entry| entry.names().contains(&name))
        .expect(name)
}

fn checked_original() -> (PackageFiles, EnvironmentCertificate) {
    let built = package::build(&source(), ROOT, &BuildOptions::default()).unwrap();
    require_stages(
        &built.receipt,
        &[
            "translated",
            "primary_check",
            "independent_recheck",
            "assumption_policy",
            "emission_reparsed",
        ],
    );
    assert!(
        package::replay(&built.files).replayed(),
        "the unmodified source package must replay"
    );
    let manifest: Manifest = serde_json::from_slice(&built.files[package::MANIFEST]).unwrap();
    assert!(
        manifest.assumptions.transitive.is_empty(),
        "the fixture has no axioms"
    );
    let certificate: EnvironmentCertificate =
        serde_json::from_slice(&built.files[package::CERTIFICATE]).unwrap();
    assert!(!certificate
        .entries
        .iter()
        .any(|entry| matches!(entry, EnvironmentEntry::Axiom { .. })));
    assert!(matches!(
        certificate.entries[entry_at(&certificate, REC)],
        EnvironmentEntry::Definition { .. }
    ));
    assert!(matches!(
        certificate.entries[entry_at(&certificate, HIDDEN)],
        EnvironmentEntry::Opaque { .. }
    ));
    (built.files, certificate)
}

/// Hidden's type and implementation only depend on Token/intro. Move it immediately before
/// the recursor so the forged body can refer to it without a forward declaration. The positive
/// control checks this same reordering, independently of the subsequent body replacement.
fn put_hidden_in_prefix(certificate: &mut EnvironmentCertificate) {
    let hidden = certificate.entries.remove(entry_at(certificate, HIDDEN));
    let rec_at = entry_at(certificate, REC);
    certificate.entries.insert(rec_at, hidden);
}

fn replace_certificate(
    mut files: PackageFiles,
    certificate: &EnvironmentCertificate,
) -> PackageFiles {
    files.insert(
        package::CERTIFICATE.into(),
        package::canonical_certificate(certificate),
    );
    let mut manifest: Manifest = serde_json::from_slice(&files[package::MANIFEST]).unwrap();
    manifest.files = files
        .iter()
        .filter(|(name, _)| *name != package::MANIFEST)
        .map(|(name, bytes)| (name.clone(), package::sha256_hex(bytes)))
        .collect();
    manifest.source.export_sha256 = manifest.files[package::EXPORT].clone();
    manifest.result_id = package::result_id(&manifest.target.lean, &manifest.files);
    files.insert(package::MANIFEST.into(), package::canonical_json(&manifest));
    files
}

#[test]
fn canonical_recursor_and_independent_opaque_inhabitant_are_source_valid() {
    let (files, mut certificate) = checked_original();
    put_hidden_in_prefix(&mut certificate);
    let reordered = package::replay(&replace_certificate(files, &certificate));
    require_stages(
        &reordered,
        &[
            "translated",
            "primary_check",
            "independent_recheck",
            "assumption_policy",
        ],
    );
}

#[test]
fn replay_refuses_a_same_typed_opaque_replacement_for_a_canonical_recursor_body() {
    let (files, mut certificate) = checked_original();
    let original_source = files[package::EXPORT].clone();
    put_hidden_in_prefix(&mut certificate);
    let before = certificate.clone();
    let rec_at = entry_at(&certificate, REC);
    let (original_levels, original_type, original_body) = match &certificate.entries[rec_at] {
        EnvironmentEntry::Definition {
            level_params,
            ty,
            body,
            ..
        } => (level_params.clone(), *ty, *body),
        _ => unreachable!(),
    };
    assert!(
        !original_levels.is_empty(),
        "comparison must occur at the polymorphic, non-Prop recursor type"
    );
    let replacement = u32::try_from(certificate.nodes.len()).unwrap();
    certificate.nodes.push(TermNode::Const(
        HIDDEN.into(),
        original_levels.iter().cloned().map(Universe::Var).collect(),
    ));
    if let EnvironmentEntry::Definition { body, .. } = &mut certificate.entries[rec_at] {
        *body = replacement;
    }
    assert_ne!(replacement, original_body);
    for (index, (old, new)) in before.entries.iter().zip(&certificate.entries).enumerate() {
        if index == rec_at {
            match new {
                EnvironmentEntry::Definition {
                    level_params,
                    ty,
                    body,
                    ..
                } => {
                    assert_eq!(level_params, &original_levels);
                    assert_eq!(
                        *ty, original_type,
                        "the checked type row is byte-for-byte unchanged"
                    );
                    assert_eq!(*body, replacement);
                }
                _ => unreachable!(),
            }
        } else {
            assert_eq!(old, new, "only the canonical recursor body may change");
        }
    }
    assert_eq!(
        &certificate.nodes[..before.nodes.len()],
        before.nodes.as_slice()
    );
    let forged = replace_certificate(files, &certificate);
    assert_eq!(
        forged[package::EXPORT],
        original_source,
        "the source is untouched"
    );
    let manifest: Manifest = serde_json::from_slice(&forged[package::MANIFEST]).unwrap();
    assert!(manifest.assumptions.transitive.is_empty());
    let replay = package::replay(&forged);
    require_stages(
        &replay,
        &["primary_check", "independent_recheck", "assumption_policy"],
    );
    let translated = replay.stage("translated").unwrap();
    assert_eq!(translated.status, StageStatus::Failed, "{replay:#?}");
    assert_eq!(
        translated.code.as_deref(),
        Some("definition_mismatch"),
        "{replay:#?}"
    );
    assert!(
        translated.detail.contains(REC),
        "refusal must identify the sourced recursor: {replay:#?}"
    );
    assert!(!replay.replayed());
    assert_eq!(
        replay.status("fresh_replay"),
        StageStatus::Failed,
        "{replay:#?}"
    );
}
