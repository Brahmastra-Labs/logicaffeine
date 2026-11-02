//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! RDU ONE-LANE LOCK — the two RDU backends (the spatial-IR lane and the placed-grid lane) emit ONE
//! neutral envelope, name ONE schema, share ONE gate string, and both certificate seams are reachable
//! from the backend that will bridge to the SDK.
//!
//! Until now the grid lane (`codegen_sva::backend::RduBackend`) dumped a bare text placement and the
//! spatial lane (`spatial::lower::rdu::RduSpatialBackend`) a v1 JSON, each with its own SDK-gate
//! sentence. A vendor engineer opening either artifact must find the same `logos.spatial.neutral` v2
//! envelope — the same target block derived from the same `RduChipFacts` — with the lane named
//! (`"lane": "spatial"` / `"lane": "grid"`) and the lane's own body (`program` / `grid.pretty`).
//!
//! ★ MUTATION PROBES (each run once against this tree; the named test went RED, the impl was restored
//! byte-identical — every line below is what the probe ACTUALLY did and what it ACTUALLY reported):
//!   - `backend.rs`: give `RduBackend` back its own `GATED_REASON` literal → `the_gate_string_is_one_
//!     constant` (two definitions of the sentence, `left:` the re-typed copy);
//!   - `backend.rs`: emit the grid lane's `neutral_placement` as the bare `serialize_grid` dump →
//!     `both_lanes_emit_the_same_envelope` (`Error("expected value", line: 1, column: 1)` — not JSON);
//!   - `backend.rs`: build the grid lane's target block from a `modeled` list of its own instead of the
//!     facts' → `both_lanes_emit_the_same_envelope` (`the target block is the same derivation of the same
//!     facts`);
//!   - `rdu.rs`: make `NEUTRAL_VERSIONS_READ` accept version 3 → `the_spatial_lane_envelope_names_the_same_
//!     schema` (`an unknown version is refused` — a reader that accepts a version it cannot mean);
//!   - `rdu.rs`: build the spatial lane's target from a FIXED chip instead of the facts it was constructed
//!     with → `both_certificate_seams_are_reachable_from_the_backend` (`left: "sn40l-rdu"` — the closing
//!     claim of that test is that BOTH seams hang off the same `RduChipFacts`, and they must).
//!     (Tried first and did NOT fire: widening the `lower` certification refusal from `Uncertified` to a
//!     generic `Unsupported`. `require_verified` refuses a program whose op was dropped before
//!     certification is ever reached, so that arm of the match is already satisfied — worth knowing, since
//!     it means the typed-refusal assertion there is carried by the phase gate, not by the certifier.)
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test rdu_one_lane_lock`
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

#[path = "common/spatial.rs"]
mod spatial_support;

use std::collections::BTreeSet;

use logicaffeine_compile::codegen_sva::backend::{BackendArtifact, RduBackend, TargetBackend};
use logicaffeine_compile::codegen_sva::fabric_grid::route_loop_body;
use logicaffeine_compile::codegen_sva::fabric_rtl::{AluOp, FabricOp, LoopCell, Src};
use logicaffeine_compile::spatial::certificate::certify_external_spatial;
use logicaffeine_compile::spatial::ir::{ChoreoOp, SpatialProgram};
use logicaffeine_compile::spatial::lower::rdu::{
    parse_neutral, RduSpatialBackend, GATED_REASON, NEUTRAL_SCHEMA, NEUTRAL_VERSION,
};
use logicaffeine_compile::spatial::lower::{
    NeutralArtifact, SpatialArtifact, SpatialBackend, SpatialLowerError,
};
use logicaffeine_compile::spatial::pipeline::compile_to_spatial;
use logicaffeine_compile::spatial::rdu_chip::RduChip;
use logicaffeine_compile::spatial::target::SpatialTarget;
use serde_json::Value;
use spatial_support::*;

/// The processing element: In0=activation, In1=incoming psum, In2=weight → `psum + a·w`.
fn pe_body() -> Vec<FabricOp> {
    vec![
        FabricOp {
            op: AluOp::Mul,
            a: Src::In(0),
            b: Src::In(2),
        },
        FabricOp {
            op: AluOp::Add,
            a: Src::In(1),
            b: Src::Val(0),
        },
    ]
}

/// Same shape, wrong op: `psum − a·w`.
fn wrong_pe_body() -> Vec<FabricOp> {
    vec![
        FabricOp {
            op: AluOp::Mul,
            a: Src::In(0),
            b: Src::In(2),
        },
        FabricOp {
            op: AluOp::Sub,
            a: Src::In(1),
            b: Src::Val(0),
        },
    ]
}

fn to_cells(body: &[FabricOp]) -> Vec<LoopCell> {
    body.iter()
        .map(|o| LoopCell::Arith {
            op: o.op,
            a: o.a,
            b: o.b,
        })
        .collect()
}

fn sn50_program() -> SpatialProgram {
    compile_to_spatial(&gemm_program(128, 128, 64), &SpatialTarget::sn50_rdu()).unwrap()
}

fn spatial_lane(chip: RduChip, p: &SpatialProgram) -> NeutralArtifact {
    match RduSpatialBackend::for_chip(chip).lower(p).unwrap() {
        SpatialArtifact::SdkGated(a) => a,
        other => panic!("the RDU spatial lane is SDK-gated, got {other:?}"),
    }
}

fn grid_lane(chip: RduChip) -> (String, &'static str) {
    let g = route_loop_body(&to_cells(&pe_body()), 3, &[Src::Val(1)]);
    match RduBackend::for_chip(chip).emit_grid("pe", &g) {
        BackendArtifact::SdkGated {
            neutral_placement,
            reason,
        } => (neutral_placement, reason),
        other => panic!("the RDU grid lane is SDK-gated, got {other:?}"),
    }
}

fn keys(v: &Value) -> BTreeSet<String> {
    v.as_object().expect("an object").keys().cloned().collect()
}

#[test]
fn both_lanes_emit_the_same_envelope() {
    for chip in RduChip::ALL {
        let p = compile_to_spatial(&gemm_program(128, 128, 64), &chip.facts().spatial_target())
            .unwrap();
        let spatial = spatial_lane(chip, &p);
        let (grid_json, grid_reason) = grid_lane(chip);
        let s: Value = serde_json::from_str(&spatial.json).expect("the spatial lane emits JSON");
        let g: Value = serde_json::from_str(&grid_json)
            .expect("the grid lane emits the same JSON envelope, not a bare text dump");
        assert_eq!(
            keys(&s)
                .into_iter()
                .filter(|k| k != "program")
                .collect::<BTreeSet<_>>(),
            keys(&g)
                .into_iter()
                .filter(|k| k != "grid")
                .collect::<BTreeSet<_>>(),
            "{chip:?}: one envelope, two bodies"
        );
        assert_eq!(s["lane"], "spatial");
        assert_eq!(g["lane"], "grid");
        assert_eq!(
            s["target"], g["target"],
            "{chip:?}: the target block is the same derivation of the same facts"
        );
        assert_eq!(s["target"]["chip"], chip.as_str());
        assert_eq!(s["target"]["name"], chip.facts().spatial_target().name);
        assert!(s["target"]["unit_model"].is_object() && s["target"]["precisions"].is_object());
        match chip {
            RduChip::Sn50 => assert!(
                s["target"]["scale_up"].is_object(),
                "the SN50 tabulates its scale-up fabric"
            ),
            RduChip::Sn40l => assert!(
                s["target"]["scale_up"].is_null(),
                "the SN40L's is not tabulated — null, never invented"
            ),
        }
        assert_eq!(
            s["target"]["chips_per_node"],
            chip.facts().chips_per_node.value
        );
        assert!(s.get("program").is_some_and(Value::is_object) && s.get("grid").is_none());
        assert!(g.get("grid").is_some_and(Value::is_object) && g.get("program").is_none());
        let pretty = g["grid"]["pretty"]
            .as_str()
            .expect("the grid lane carries its deterministic dump");
        assert!(
            pretty.contains("grid rows="),
            "{chip:?}: the dump is the placed grid"
        );
        assert!(
            grid_json.contains("grid rows="),
            "{chip:?}: the raw artifact still carries the dump text for the bridge that reads it"
        );
        for lane in [&s, &g] {
            for k in ["sections", "state", "gathers"] {
                assert!(
                    lane[k].as_array().is_some_and(Vec::is_empty),
                    "{chip:?}: `{k}` is an (empty) array until the composed form lands"
                );
            }
            assert!(
                lane["ii_report"].is_object(),
                "{chip:?}: `ii_report` is an object"
            );
        }
        assert_eq!(spatial.reason, grid_reason, "{chip:?}: one gate sentence");
        assert!(
            s["certificate"]["schedule_digest"].is_number(),
            "{chip:?}: the spatial lane ships its certificate summary"
        );
        assert!(
            g["certificate"].is_null(),
            "{chip:?}: the grid lane's proof comes from `certify`, not from emission"
        );
    }
    // A reconfigurable array of grids: the same envelope, every grid named.
    let g1 = route_loop_body(&to_cells(&pe_body()), 3, &[Src::Val(1)]);
    let g2 = route_loop_body(&to_cells(&wrong_pe_body()), 3, &[Src::Val(1)]);
    match RduBackend::for_chip(RduChip::Sn50).emit_config_grid("pair", &[&g1, &g2], 1) {
        BackendArtifact::SdkGated {
            neutral_placement, ..
        } => {
            let v: Value = serde_json::from_str(&neutral_placement).unwrap();
            assert_eq!(v["lane"], "grid");
            assert_eq!(v["grid"]["grids"].as_array().unwrap().len(), 2);
            assert_eq!(v["grid"]["cfg_width"], 1);
            assert_eq!(
                neutral_placement.matches("grid rows=").count(),
                4,
                "two grids, each once in the array and once in the joined dump"
            );
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn the_spatial_lane_envelope_names_the_same_schema() {
    assert_eq!(NEUTRAL_VERSION, 2, "the envelope is v2");
    let p = sn50_program();
    let spatial = spatial_lane(RduChip::Sn50, &p);
    let (grid_json, _) = grid_lane(RduChip::Sn50);
    for (lane, json) in [("spatial", &spatial.json), ("grid", &grid_json)] {
        let v: Value = serde_json::from_str(json).unwrap();
        assert_eq!(v["schema"], NEUTRAL_SCHEMA, "{lane}");
        assert_eq!(v["version"], NEUTRAL_VERSION, "{lane}");
    }
    // The spatial lane round-trips; the grid lane carries no spatial program and says so.
    let back = parse_neutral(&spatial.json).expect("a v2 spatial artifact parses");
    let mut expect = p.clone();
    expect.certificate = None;
    assert_eq!(back, expect);
    let err = parse_neutral(&grid_json)
        .expect_err("a grid-lane artifact has no spatial program to parse");
    assert!(err.contains("grid"), "the refusal names the lane: {err}");
    // v1 is still read (the artifacts already shipped), v3 is not (nobody wrote one).
    let mut v1: Value = serde_json::from_str(&spatial.json).unwrap();
    v1["version"] = Value::from(1);
    v1.as_object_mut().unwrap().remove("lane");
    assert_eq!(parse_neutral(&v1.to_string()).expect("v1 parses"), expect);
    let mut v3: Value = serde_json::from_str(&spatial.json).unwrap();
    v3["version"] = Value::from(3);
    let err = parse_neutral(&v3.to_string()).expect_err("an unknown version is refused");
    assert!(err.contains('3'), "{err}");
    let mut wrong_schema: Value = serde_json::from_str(&spatial.json).unwrap();
    wrong_schema["schema"] = Value::from("logos.something.else");
    assert!(parse_neutral(&wrong_schema.to_string()).is_err());
}

#[test]
fn both_certificate_seams_are_reachable_from_the_backend() {
    // The grid seam: `TargetBackend::certify` proves a placed grid ≡ its PE body (SAT, re-checkable).
    let rdu = RduBackend::for_chip(RduChip::Sn50);
    assert_eq!(rdu.name(), "sn50-rdu");
    let g = route_loop_body(&to_cells(&pe_body()), 3, &[Src::Val(1)]);
    let cert = rdu
        .certify(&g, &pe_body(), 1, 8)
        .expect("a correct placement certifies");
    assert!(cert.recheck());
    assert!(
        rdu.certify(&g, &wrong_pe_body(), 1, 8).is_none(),
        "a wrong body earns no certificate"
    );
    // The spatial seam: `certify_external_spatial` proves an external placement+schedule ≡ the source program.
    let p = sn50_program();
    let backend = RduSpatialBackend::for_chip(RduChip::Sn50);
    assert_eq!(backend.name(), "sn50-rdu");
    let faithful = certify_external_spatial(&p, &p).expect("a faithful placement certifies");
    assert!(faithful.recheck(&p));
    let wait = enumerate_ops(p.schedule.as_ref().unwrap())
        .into_iter()
        .find(|(_, op)| matches!(op, ChoreoOp::Wait { .. }))
        .unwrap();
    let broken = without_op(&p, &wait.0);
    assert!(
        certify_external_spatial(&broken, &p).is_none(),
        "a dropped op is refused at the spatial seam"
    );
    match backend.lower(&broken) {
        Err(SpatialLowerError::NotVerified { .. }) | Err(SpatialLowerError::Uncertified(_)) => {}
        other => panic!("the backend refuses to lower a broken choreography: {other:?}"),
    }
    // Both seams hang off the same facts.
    assert_eq!(rdu.facts, backend.facts);
    assert_eq!(*rdu.target(), backend.facts.fabric_target());
    assert_eq!(*backend.target(), rdu.facts.spatial_target());
}

#[test]
fn the_gate_string_is_one_constant() {
    assert_eq!(RduBackend::GATED_REASON, GATED_REASON);
    assert_eq!(RduSpatialBackend::GATED_REASON, GATED_REASON);
    assert!(
        GATED_REASON.contains("SDK") && GATED_REASON.contains("certified"),
        "{GATED_REASON}"
    );
    let p = sn50_program();
    assert_eq!(spatial_lane(RduChip::Sn50, &p).reason, GATED_REASON);
    assert_eq!(grid_lane(RduChip::Sn40l).1, GATED_REASON);
    // ONE definition in the sources: the sentence appears as a literal exactly once across both lanes.
    let root = repo_root();
    let sources = [
        "crates/logicaffeine_compile/src/spatial/lower/rdu.rs",
        "crates/logicaffeine_compile/src/codegen_sva/backend.rs",
    ];
    let mut definitions = 0usize;
    for rel in sources {
        let text = std::fs::read_to_string(root.join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"));
        definitions += text
            .matches("config emission is gated on the vendor SDK")
            .count();
    }
    assert_eq!(
        definitions, 1,
        "the gate sentence is written once, in `spatial/lower/rdu.rs`, and re-exported"
    );
}
