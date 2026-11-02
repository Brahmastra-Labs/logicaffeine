//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! RDU SDK BRIDGE LOCK — every plausible SambaNova front door is a prepared, tested `RdaBridge`.
//!
//! We do not have SambaFlow. The RDU lane stays honestly `SdkGated` (a certified neutral placement) until
//! the vendor config bytes exist. What CAN be locked tonight is the SEAM the day fills in: one object-safe
//! trait, dispatched by name, whose two halves are
//!   * `ingest`      — the neutral artifact → whatever the SDK takes (the mock is the identity: the neutral
//!                     JSON bytes, stamped with the certificate digest), refused `Uncertified` when the
//!                     artifact's own certificate does not recheck, `Malformed` when it does not parse;
//!   * `certify_back` — whatever the SDK hands back (a placement of the SAME computation) → the
//!                     proof-carrying certificate on THEIR mapping, or `None`, fail-closed.
//! The SambaFlow bridge refuses every call `NotImplemented`, naming the SDK it needs, so `largo emit rdu
//! --bridge sambaflow` says exactly what is missing until the vendor API exists — and the file it lives in
//! is the one file the day rewrites.
//!
//! THE HOLE A PROBE FOUND. `ingest` originally proved the certificate STRUCT against the program parsed out
//! of the JSON — but nothing bound the JSON's own `certificate.schedule_digest` to the certificate travelling
//! beside it. A vendor reads the envelope; we forward `VendorConfig.certificate_digest` from the struct. Edit
//! that one number in the bytes and the two disagreed silently: the artifact still ingested, and the digest
//! the vendor could see was not the digest the proof was under. `envelope_digest` now reads it out of the
//! bytes and `ingest` refuses `Uncertified` when the two differ — `recheck` cannot see this (it never looks
//! at the JSON), which is exactly why it needed its own gate and its own probe.
//!
//! ★ MUTATION PROBES (each run once against this tree; the named test went RED, the impl was restored
//! byte-identical):
//!   - `rdu_sdk.rs`: make `MockBridge::certify_back` return `certify_choreography(&external).ok()` without
//!     comparing against `source` → the "different computation" case of `a_bridge_that_reorders_or_drops_a_
//!     stage_is_refused_by_certify_back` (a 64³ GEMM's placement certified against a 128³ source);
//!   - `rdu_sdk.rs`: drop the `recheck` in `MockBridge::ingest` → `a_bridge_never_emits_without_a_
//!     certificate` on the witness-tampered artifact;
//!   - `rdu_sdk.rs`: drop the `in_envelope != digest` guard from `MockBridge::ingest` → `a_bridge_never_
//!     emits_without_a_certificate` on the envelope whose digest was edited by one (the hole above).
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test rdu_sdk_bridge_lock`
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

#[path = "common/spatial.rs"]
mod spatial_support;

use logicaffeine_compile::spatial::ir::{ChoreoOp, SpatialProgram};
use logicaffeine_compile::spatial::lower::rdu::{
    parse_neutral, RduSpatialBackend, NEUTRAL_SCHEMA, NEUTRAL_VERSION,
};
use logicaffeine_compile::spatial::lower::rdu_sdk::{
    bridge_by_name, certificate_digest, envelope_digest, BridgeRefusal, MockBridge, RdaBridge,
    SambaFlowBridge, VendorPlacement, BRIDGE_NAMES, NEUTRAL_FORMAT,
};
use logicaffeine_compile::spatial::lower::{NeutralArtifact, SpatialArtifact, SpatialBackend};
use logicaffeine_compile::spatial::pipeline::compile_to_spatial;
use logicaffeine_compile::spatial::rdu_chip::RduChip;
use spatial_support::*;

/// Both RDU generations: the bridge is chip-agnostic, and the lock says so by running on each.
const CHIPS: [RduChip; 2] = [RduChip::Sn40l, RduChip::Sn50];

fn rdu_program_on(chip: RduChip, m: usize, n: usize, k: usize) -> SpatialProgram {
    let target = RduSpatialBackend::for_chip(chip).target;
    compile_to_spatial(&gemm_program(m, n, k), &target)
        .unwrap_or_else(|e| panic!("the {m}x{n}x{k} GEMM lowers on {}: {e:?}", target.name))
}

fn rdu_program(m: usize, n: usize, k: usize) -> SpatialProgram {
    rdu_program_on(RduChip::Sn40l, m, n, k)
}

fn neutral_on(chip: RduChip, p: &SpatialProgram) -> NeutralArtifact {
    match RduSpatialBackend::for_chip(chip)
        .lower(p)
        .expect("a verified program lowers")
    {
        SpatialArtifact::SdkGated(a) => a,
        other => panic!("the RDU artifact is SDK-gated, got {other:?}"),
    }
}

fn neutral(p: &SpatialProgram) -> NeutralArtifact {
    neutral_on(RduChip::Sn40l, p)
}

/// A vendor placement of `p`, as an SDK would hand it back: the program alone, in the neutral envelope.
fn placement_of(p: &SpatialProgram) -> VendorPlacement {
    let mut q = p.clone();
    q.certificate = None;
    VendorPlacement { program_json: serde_json::json!({ "schema": NEUTRAL_SCHEMA, "version": NEUTRAL_VERSION, "lane": "spatial", "program": q }).to_string() }
}

#[test]
fn the_mock_bridge_round_trips_the_neutral_artifact_and_certifies_back() {
    for chip in CHIPS {
        let p = rdu_program_on(chip, 64, 64, 64);
        let n = neutral_on(chip, &p);
        let cfg = MockBridge.ingest(&n).unwrap_or_else(|e| {
            panic!("{chip:?}: the mock bridge ingests a certified artifact: {e}")
        });
        assert_eq!(
            cfg.format, NEUTRAL_FORMAT,
            "the mock's format names the neutral schema"
        );
        assert_eq!(
            cfg.bytes,
            n.json.as_bytes(),
            "the mock is the identity: the config bytes ARE the neutral JSON"
        );
        let parsed = parse_neutral(std::str::from_utf8(&cfg.bytes).unwrap())
            .expect("the config bytes parse back to the program");
        let mut expect = p.clone();
        expect.certificate = None;
        assert_eq!(
            parsed, expect,
            "round trip: the program inside the config is the program"
        );
        let v: serde_json::Value = serde_json::from_str(&n.json).unwrap();
        assert_eq!(
            v["target"]["chip"],
            chip.as_str(),
            "the envelope names the chip the placement was made for (the `--chip` dial spelling)"
        );

        let back = VendorPlacement {
            program_json: String::from_utf8(cfg.bytes.clone()).unwrap(),
        };
        let cert = MockBridge
            .certify_back(&back, &p)
            .expect("the faithful placement certifies back");
        assert!(
            cert.recheck(&p),
            "the certificate handed back re-verifies against the source program"
        );
        assert_eq!(
            cert.schedule_digest, n.certificate.schedule_digest,
            "the same schedule earns the same digest"
        );
    }
}

#[test]
fn a_bridge_that_reorders_or_drops_a_stage_is_refused_by_certify_back() {
    let p = rdu_program(64, 64, 64);
    assert!(
        MockBridge.certify_back(&placement_of(&p), &p).is_some(),
        "control: the faithful placement certifies"
    );

    // Reordered: the compute engine's program run backwards (pops before waits, packs before computes).
    let mut reordered = p.clone();
    {
        let sched = reordered.schedule.as_mut().unwrap();
        let prog = sched
            .programs
            .iter_mut()
            .find(|pr| {
                pr.ops.iter().any(|t| {
                    matches!(t.op, ChoreoOp::Compute { .. })
                        || matches!(t.op, ChoreoOp::Loop { .. })
                })
            })
            .expect("a compute program");
        prog.ops.reverse();
        reordered.certificate = None;
    }
    assert!(
        MockBridge
            .certify_back(&placement_of(&reordered), &p)
            .is_none(),
        "a reordered choreography earns no certificate"
    );

    // Dropped from the schedule: every op of the first load stage removed (the stage is credited by the
    // coverage proof but never moves a tile).
    let load = p
        .stages
        .iter()
        .find(|s| s.name.starts_with("load_"))
        .expect("a load stage")
        .id;
    let mut dropped = p.clone();
    for prog in &mut dropped.schedule.as_mut().unwrap().programs {
        prog.ops.retain(|t| t.stage != load);
        for t in &mut prog.ops {
            if let ChoreoOp::Loop { body, .. } = &mut t.op {
                body.retain(|b| b.stage != load);
            }
        }
    }
    dropped.certificate = None;
    assert!(
        MockBridge
            .certify_back(&placement_of(&dropped), &p)
            .is_none(),
        "a placement that drops a stage's data movement earns no certificate"
    );

    // Dropped from the program: the last stage gone (its edges dangle).
    let mut truncated = p.clone();
    truncated.stages.pop();
    truncated.certificate = None;
    assert!(
        MockBridge
            .certify_back(&placement_of(&truncated), &p)
            .is_none(),
        "a placement missing a stage earns no certificate"
    );

    // A DIFFERENT computation, however well-formed, is never certified against this source.
    let other = rdu_program(128, 128, 128);
    assert!(
        MockBridge.certify_back(&placement_of(&other), &p).is_some() == false,
        "a placement of another computation is refused"
    );
    assert!(
        MockBridge
            .certify_back(&placement_of(&other), &other)
            .is_some(),
        "control: it certifies against its own source"
    );

    // Bytes that are not a neutral artifact at all: fail-closed, never a panic.
    assert!(MockBridge
        .certify_back(
            &VendorPlacement {
                program_json: "{\"program\": 42}".into()
            },
            &p
        )
        .is_none());
    assert!(MockBridge
        .certify_back(
            &VendorPlacement {
                program_json: "not json".into()
            },
            &p
        )
        .is_none());
}

#[test]
fn the_bridge_trait_is_object_safe_and_dispatches_by_name() {
    let mock: Box<dyn RdaBridge> = bridge_by_name("mock").expect("the mock bridge exists");
    assert_eq!(mock.name(), "mock");
    let sf: Box<dyn RdaBridge> = bridge_by_name("sambaflow").expect("the sambaflow bridge exists");
    assert_eq!(sf.name(), "sambaflow");
    assert!(
        bridge_by_name("nonesuch").is_none(),
        "an unknown bridge is None, never a default"
    );
    assert_eq!(
        BRIDGE_NAMES,
        &["mock", "sambaflow"],
        "the name table the CLI prints is exactly the dispatch table"
    );
    for name in BRIDGE_NAMES {
        assert_eq!(
            bridge_by_name(name).unwrap().name(),
            *name,
            "every listed name dispatches to a bridge of that name"
        );
    }

    // The day's front door: until the vendor SDK exists, sambaflow refuses BY NAME, naming the SDK.
    let p = rdu_program(64, 64, 64);
    let n = neutral(&p);
    match sf.ingest(&n) {
        Err(BridgeRefusal::NotImplemented { bridge, reason }) => {
            assert_eq!(bridge, "sambaflow");
            assert!(
                reason.to_lowercase().contains("sdk"),
                "the reason names the missing SDK: {reason}"
            );
        }
        other => panic!(
            "sambaflow must refuse NotImplemented until the vendor SDK exists, got {other:?}"
        ),
    }
    assert!(
        sf.certify_back(&placement_of(&p), &p).is_none(),
        "no SDK, no certificate — never a vacuous Some"
    );
    let refusal = SambaFlowBridge.ingest(&n).unwrap_err().to_string();
    assert!(
        refusal.starts_with("rdu bridge: sambaflow is not implemented"),
        "the typed message the CLI prints: {refusal}"
    );
    assert!(refusal.contains("vendor SDK"), "{refusal}");
}

#[test]
fn a_bridge_never_emits_without_a_certificate() {
    let p = rdu_program(64, 64, 64);
    let n = neutral(&p);
    let cfg = MockBridge.ingest(&n).unwrap();
    assert_eq!(
        cfg.certificate_digest,
        certificate_digest(&n.certificate),
        "the config carries the artifact's certificate digest"
    );
    assert_eq!(
        cfg.certificate_digest,
        format!("{:016x}", n.certificate.schedule_digest),
        "the digest is the schedule digest, 16 hex digits"
    );
    assert!(!cfg.certificate_digest.is_empty());

    // An artifact whose certificate does not recheck against its own program is refused — the bridge
    // forwards a PROOF, never a claim.
    let tampered = NeutralArtifact {
        certificate: n.certificate.with_witness_tampered(),
        ..n.clone()
    };
    assert!(
        !tampered.certificate.recheck(&p),
        "control: the tampered certificate does not recheck"
    );
    match MockBridge.ingest(&tampered) {
        Err(BridgeRefusal::Uncertified(why)) => assert!(why.contains(&certificate_digest(&tampered.certificate)), "the refusal names the digest it refused: {why}"),
        other => panic!("an artifact whose certificate does not recheck must be refused Uncertified, got {other:?}"),
    }

    // An artifact whose JSON is not a neutral artifact is refused Malformed.
    let malformed = NeutralArtifact {
        json: "{\"schema\": \"logos.spatial.neutral\"}".into(),
        ..n.clone()
    };
    assert!(
        matches!(
            MockBridge.ingest(&malformed),
            Err(BridgeRefusal::Malformed(_))
        ),
        "no program inside → Malformed"
    );
    let garbage = NeutralArtifact {
        json: "<not json>".into(),
        ..n.clone()
    };
    assert!(matches!(
        MockBridge.ingest(&garbage),
        Err(BridgeRefusal::Malformed(_))
    ));

    // A certificate that rechecks against a DIFFERENT program than the one in the JSON is also refused:
    // the digest binds the certificate to the bytes it travels with.
    let other = rdu_program(128, 128, 128);
    let mismatched = NeutralArtifact {
        json: neutral(&other).json,
        ..n.clone()
    };
    assert!(
        matches!(
            MockBridge.ingest(&mismatched),
            Err(BridgeRefusal::Uncertified(_))
        ),
        "a certificate of another program is not this program's proof"
    );

    // The envelope's own `certificate.schedule_digest` is read and bound: edit that one number and the
    // bytes no longer name the proof beside them.
    assert_eq!(
        envelope_digest(&n.json).unwrap(),
        cfg.certificate_digest,
        "the envelope carries the digest the config forwards"
    );
    let mut v: serde_json::Value = serde_json::from_str(&n.json).unwrap();
    let d = v["certificate"]["schedule_digest"]
        .as_u64()
        .expect("a v2 envelope carries the digest as a number");
    v["certificate"]["schedule_digest"] = serde_json::json!(d.wrapping_add(1));
    let edited = NeutralArtifact {
        json: v.to_string(),
        ..n.clone()
    };
    match MockBridge.ingest(&edited) {
        Err(BridgeRefusal::Uncertified(why)) => assert!(why.contains(&cfg.certificate_digest), "the refusal names the certificate's digest: {why}"),
        other => panic!("an envelope whose digest disagrees with its certificate must be refused Uncertified, got {other:?}"),
    }
    let mut v: serde_json::Value = serde_json::from_str(&n.json).unwrap();
    v["certificate"]
        .as_object_mut()
        .unwrap()
        .remove("schedule_digest");
    let stripped = NeutralArtifact {
        json: v.to_string(),
        ..n.clone()
    };
    assert!(
        matches!(
            MockBridge.ingest(&stripped),
            Err(BridgeRefusal::Malformed(_))
        ),
        "an envelope without a digest is malformed"
    );
}
