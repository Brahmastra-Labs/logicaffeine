//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! TARGET-BACKEND SEAM LOCK — "plug in a new hardware backend" is a named, tested extension point.
//!
//! Everything up to a placed grid is target-agnostic; only `Grid → bytes` is backend-specific. This lock pins
//! the `TargetBackend` seam:
//!   1. FAITHFUL — the default Sky130/Verilog backend emits EXACTLY the existing free-function RTL (no drift).
//!   2. HONEST RDU SLOT — the SN40L RDU backend cannot emit vendor config (no SDK), so it honestly returns
//!      `SdkGated` with the ready-to-bridge neutral placement — never a faked config — AND its proof-carrying
//!      `certify` works NOW (correct→cert+recheck, wrong→None).
//!   3. REAL EXTENSION POINT — the trait is object-safe: a `Vec<Box<dyn TargetBackend>>` dispatches both.
//!
//! Run: `cargo test -p logicaffeine-tests --test target_backend_lock`
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::codegen_sva::backend::{
    serialize_grid, BackendArtifact, RduBackend, SkyVerilogBackend, TargetBackend,
};
use logicaffeine_compile::codegen_sva::fabric_grid::{emit_grid_verilog, route_loop_body, Grid};
use logicaffeine_compile::codegen_sva::fabric_rtl::{AluOp, FabricOp, LoopCell, Src};

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
fn to_cells(b: &[FabricOp]) -> Vec<LoopCell> {
    b.iter()
        .map(|o| LoopCell::Arith {
            op: o.op,
            a: o.a,
            b: o.b,
        })
        .collect()
}
fn a_grid() -> Grid {
    route_loop_body(&to_cells(&pe_body()), 3, &[Src::Val(1)])
}

#[test]
fn sky_backend_emits_exactly_the_existing_rtl() {
    let g = a_grid();
    let sky = SkyVerilogBackend::new();
    assert_eq!(sky.name(), "sky130-verilog");
    match sky.emit_grid("t", &g) {
        BackendArtifact::Verilog(v) => {
            assert_eq!(
                v,
                emit_grid_verilog("t", &g),
                "the backend must be a FAITHFUL wrapper — no RTL drift"
            );
        }
        other => panic!("the Sky backend must emit Verilog, got {other:?}"),
    }
    println!(
        "target-backend: the Sky130 backend is byte-identical to the existing emit_grid_verilog"
    );
}

#[test]
fn rdu_backend_is_honestly_sdk_gated_but_certifies_now() {
    let g = a_grid();
    let rdu = RduBackend::new();
    assert_eq!(rdu.name(), "sn40l-rdu");
    assert_eq!(
        rdu.target().name,
        "sn40l-rdu",
        "the RDU backend targets the real SN40L descriptor"
    );

    // Emission is honestly gated on the SDK — a ready-to-bridge neutral placement, never a faked config.
    match rdu.emit_grid("t", &g) {
        BackendArtifact::SdkGated {
            neutral_placement,
            reason,
        } => {
            assert!(
                !neutral_placement.is_empty(),
                "the SDK-gated artifact must carry the placement to bridge"
            );
            assert!(
                neutral_placement.contains("grid rows="),
                "the neutral placement is the deterministic dump"
            );
            assert!(
                reason.to_lowercase().contains("sdk"),
                "the reason must name the SDK gating honestly"
            );
        }
        other => {
            panic!("the RDU backend must NOT fake a config — expected SdkGated, got {other:?}")
        }
    }

    // But the proof-carrying certificate works TODAY (no SDK): correct source → cert that re-checks.
    let cert = rdu
        .certify(&g, &pe_body(), 1, 8)
        .expect("the RDU backend certifies a correct placement now");
    assert!(
        cert.recheck(),
        "the RDU placement certificate must re-verify independently"
    );
    // fail-closed: a wrong source earns no certificate.
    assert!(
        rdu.certify(&g, &wrong_pe_body(), 1, 8).is_none(),
        "a wrong source must earn NO certificate"
    );

    println!("target-backend: the RDU slot is honestly SDK-gated for config, but ships a re-checked proof now");
}

#[test]
fn the_seam_is_an_object_safe_extension_point() {
    let g = a_grid();
    // The whole point: a heterogeneous list of backends dispatches through one trait — plugging in a new
    // backend (the RDU) required no change to callers, only a new impl.
    let backends: Vec<Box<dyn TargetBackend>> = vec![
        Box::new(SkyVerilogBackend::new()),
        Box::new(RduBackend::new()),
    ];
    let mut verilog = 0;
    let mut gated = 0;
    for b in &backends {
        match b.emit_grid("x", &g) {
            BackendArtifact::Verilog(_) => verilog += 1,
            BackendArtifact::SdkGated { .. } => gated += 1,
        }
        // every backend, whatever its emission, can prove the placement ≡ source (backend-agnostic).
        assert!(
            b.certify(&g, &pe_body(), 1, 8).is_some_and(|c| c.recheck()),
            "{} must certify",
            b.name()
        );
    }
    assert_eq!(
        (verilog, gated),
        (1, 1),
        "one Verilog backend + one SDK-gated backend dispatched through dyn"
    );
    // serialize_grid is deterministic (no hashmap order) so the neutral placement is stable.
    assert_eq!(
        serialize_grid(&g),
        serialize_grid(&g),
        "the neutral placement must be deterministic"
    );
    println!("target-backend: Vec<Box<dyn TargetBackend>> dispatches both — a real, tested extension point");
}
