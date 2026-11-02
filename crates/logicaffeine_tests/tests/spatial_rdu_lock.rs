//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! RDU LOCK — the SambaNova slot lowers the SAME spatial IR to a real, certified neutral placement.
//!
//! The RDU backend serializes the placed+scheduled program as `SpatialNeutral v1` — PCU programs (stages
//! with their choreography), PMU stage buffers with depths, semaphores, token edges, broadcast groups,
//! PCU loop bounds — plus the choreography certificate, and stays honestly SDK-gated for the RDA config
//! bytes. This lock proves:
//!   A. the serialization is a FUNCTION of the IR (a different tiling → different bytes; same → identical)
//!      and round-trips (`parse_neutral` gives back the program);
//!   B. it carries every buffer depth and every semaphore, and its census matches the IR;
//!   C. the artifact is SDK-gated with a reason naming the SDK, and its certificate rechecks;
//!   D. a tampered schedule earns no certificate — the backend refuses, and `certify_external_spatial`
//!      (the import-and-certify seam for a vendor-produced placement) returns `None`;
//!   E. the published SN40L numbers are preserved in the artifact's target block; the Grid-lane backend and
//!      its `grid rows=` payload are untouched;
//!   F. the backend ships a certificate only as proved: the one the program carries when it RECHECKS (the artifact then
//!      byte-for-byte the one a fresh derivation makes, and marked rechecked), else one derived — never a stale one.
//!
//! ★ MUTATION PROBE (2026-09-25, private copy): the backend shipping the carried certificate without rechecking it → F.
//!
//! Run: `cargo test -p logicaffeine-tests --test spatial_rdu_lock`
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

#[path = "common/spatial.rs"]
mod spatial_support;

use logicaffeine_compile::codegen_sva::backend::{BackendArtifact, RduBackend, TargetBackend};
use logicaffeine_compile::codegen_sva::fabric_grid::route_loop_body;
use logicaffeine_compile::codegen_sva::fabric_rtl::{AluOp, LoopCell, Src};
use logicaffeine_compile::codegen_sva::fabric_target::FabricTarget;
use logicaffeine_compile::spatial::certificate::certify_external_spatial;
use logicaffeine_compile::spatial::ir::{BufferKind, ChoreoOp, SpatialProgram};
use logicaffeine_compile::spatial::lower::rdu::{
    parse_neutral, RduSpatialBackend, NEUTRAL_SCHEMA, NEUTRAL_VERSION,
};
use logicaffeine_compile::spatial::lower::{
    Census, SpatialArtifact, SpatialBackend, SpatialLowerError,
};
use logicaffeine_compile::spatial::pipeline::compile_to_spatial;
use logicaffeine_compile::spatial::target::SpatialTarget;
use spatial_support::*;

fn rdu_program(m: usize, n: usize, k: usize) -> SpatialProgram {
    compile_to_spatial(&gemm_program(m, n, k), &SpatialTarget::sn40l_rdu()).unwrap()
}

fn lower(p: &SpatialProgram) -> logicaffeine_compile::spatial::lower::NeutralArtifact {
    match RduSpatialBackend::new().lower(p).unwrap() {
        SpatialArtifact::SdkGated(a) => a,
        other => panic!("the RDU artifact is SDK-gated, got {other:?}"),
    }
}

#[test]
fn neutral_serialization_is_a_function_of_the_ir_and_round_trips() {
    let a = rdu_program(256, 256, 128);
    let b = rdu_program(512, 256, 128);
    let na = lower(&a);
    let nb = lower(&b);
    assert_ne!(na.json, nb.json, "different tilings serialize differently");
    assert_eq!(
        na.json,
        lower(&a).json,
        "the same program serializes identically"
    );
    let back = parse_neutral(&na.json).expect("the neutral artifact parses");
    let mut expect = a.clone();
    expect.certificate = None;
    assert_eq!(
        back, expect,
        "round-trip: the parsed program is the program (the certificate travels beside it)"
    );
    let v: serde_json::Value = serde_json::from_str(&na.json).unwrap();
    assert_eq!(v["schema"], NEUTRAL_SCHEMA);
    assert_eq!(v["version"], NEUTRAL_VERSION);
}

#[test]
fn neutral_carries_every_buffer_depth_and_semaphore() {
    let p = rdu_program(512, 512, 256);
    let n = lower(&p);
    let v: serde_json::Value = serde_json::from_str(&n.json).unwrap();
    let buffers = v["program"]["buffers"].as_array().unwrap();
    assert_eq!(buffers.len(), p.buffers.len());
    for (b, jb) in p.buffers.iter().zip(buffers) {
        match b.kind {
            BufferKind::Fifo {
                depth_pages,
                page_tiles,
            }
            | BufferKind::Interm {
                depth_pages,
                page_tiles,
            } => {
                assert_eq!(
                    jb["kind"]
                        .as_object()
                        .and_then(|o| o.values().next())
                        .and_then(|k| k["depth_pages"].as_u64()),
                    Some(depth_pages as u64),
                    "{b:?} depth in the artifact"
                );
                assert_eq!(
                    jb["kind"]
                        .as_object()
                        .and_then(|o| o.values().next())
                        .and_then(|k| k["page_tiles"].as_u64()),
                    Some(page_tiles as u64)
                );
            }
            BufferKind::Accumulator { .. } => {}
        }
    }
    assert_eq!(
        v["program"]["semaphores"].as_array().unwrap().len(),
        p.semaphores.len()
    );
    assert_eq!(
        n.census,
        Census::of(&p),
        "the artifact's census is the IR's census"
    );
    assert!(
        n.pretty.lines().count() >= p.stages.len() + p.buffers.len(),
        "the pretty form lists every object"
    );
}

#[test]
fn rdu_artifact_is_sdk_gated_and_certified() {
    let p = rdu_program(256, 256, 128);
    let n = lower(&p);
    assert!(n.reason.to_lowercase().contains("sdk"), "{}", n.reason);
    assert!(
        n.certificate.recheck(&p),
        "the shipped certificate re-verifies against the program"
    );
    let v: serde_json::Value = serde_json::from_str(&n.json).unwrap();
    assert!(
        v["certificate"]["schedule_digest"].is_number()
            || v["certificate"]["schedule_digest"].is_string()
    );
    assert_eq!(RduSpatialBackend::new().name(), "sn40l-rdu");
}

#[test]
fn a_tampered_schedule_earns_no_certificate() {
    let p = rdu_program(256, 256, 128);
    let wait = enumerate_ops(p.schedule.as_ref().unwrap())
        .into_iter()
        .find(|(_, op)| matches!(op, ChoreoOp::Wait { .. }))
        .unwrap();
    let broken = without_op(&p, &wait.0);
    match RduSpatialBackend::new().lower(&broken) {
        Err(SpatialLowerError::NotVerified { .. }) | Err(SpatialLowerError::Uncertified(_)) => {}
        other => panic!("a program whose choreography is broken must not lower: {other:?}"),
    }
    assert!(
        certify_external_spatial(&broken, &p).is_none(),
        "import-and-certify must refuse a vendor placement whose schedule is broken"
    );
    // A faithful external placement (the same program) certifies.
    let cert = certify_external_spatial(&p, &p).expect("a faithful external placement certifies");
    assert!(cert.recheck(&p));
    // An external program that changed the COMPUTATION (not just placement/schedule) is refused.
    let mut other = rdu_program(256, 256, 256);
    other.certificate = None;
    assert!(
        certify_external_spatial(&other, &p).is_none(),
        "a different computation is never certified against this source"
    );
}

#[test]
fn the_published_sn40l_numbers_are_preserved_and_the_grid_lane_is_untouched() {
    let p = rdu_program(256, 256, 128);
    let n = lower(&p);
    let v: serde_json::Value = serde_json::from_str(&n.json).unwrap();
    let t = &v["target"];
    assert_eq!(t["name"], "sn40l-rdu");
    let silicon = FabricTarget::rdu_shaped().silicon.unwrap();
    assert_eq!(t["silicon"]["compute_tiles"], silicon.compute_tiles as u64);
    assert_eq!(t["silicon"]["mem_tiles"], silicon.mem_tiles as u64);
    assert_eq!(t["silicon"]["node_nm"], silicon.node_nm as u64);
    assert_eq!(t["mem"]["sram_kib"], (520 * 1024) as u64);
    assert_eq!(t["mem"]["hbm_gib"], 64u64);
    assert_eq!(t["mem"]["ddr_gib"], 1536u64);
    // The Grid-lane backend still emits its deterministic dump — the two lanes coexist until migration completes.
    let cells = vec![
        LoopCell::Arith {
            op: AluOp::Mul,
            a: Src::In(0),
            b: Src::In(2),
        },
        LoopCell::Arith {
            op: AluOp::Add,
            a: Src::In(1),
            b: Src::Val(0),
        },
    ];
    let g = route_loop_body(&cells, 3, &[Src::Val(1)]);
    match RduBackend::new().emit_grid("t", &g) {
        BackendArtifact::SdkGated {
            neutral_placement, ..
        } => assert!(neutral_placement.contains("grid rows=")),
        other => panic!("{other:?}"),
    }
}

#[test]
fn the_backend_ships_the_carried_certificate_only_when_it_rechecks() {
    let p = rdu_program(256, 256, 128);
    let carried = lower(&p);
    assert!(
        carried.rechecked,
        "a certificate the program carries and that rechecks is shipped as rechecked"
    );
    assert_eq!(
        Some(&carried.certificate),
        p.certificate.as_ref(),
        "the carried certificate is the one shipped"
    );
    let mut stale = p.clone();
    stale.certificate = rdu_program(512, 256, 128).certificate;
    assert!(
        !stale
            .certificate
            .as_ref()
            .expect("certified")
            .recheck(&stale),
        "another program's certificate does not recheck"
    );
    let derived = lower(&stale);
    assert!(
        !derived.rechecked,
        "a certificate that does not recheck is not shipped as rechecked"
    );
    assert_eq!(derived.certificate, carried.certificate, "it is replaced by the derivation — the carried certificate a fresh one equals — never shipped");
    assert_eq!(
        derived.json, carried.json,
        "and the artifact is byte-for-byte the one the rechecked certificate makes"
    );
}
