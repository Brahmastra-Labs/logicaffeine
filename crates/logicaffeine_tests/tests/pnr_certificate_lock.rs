//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! PROOF-CARRYING PLACE-AND-ROUTE LOCK.
//!
//! A spatial-dataflow toolchain that validates a placement with performance counters asks you to TRUST that
//! the placed PE grid computes the intended datapath. We instead emit a CERTIFICATE and prove that an
//! INDEPENDENT checker re-verifies it — trusting only the tiny PR/RUP checker, never our placer or prover.
//! This is the concrete form of the SambaNova pitch's differentiator ("no formal verification of the mapping"
//! in their published work → a machine-checkable proof the vendor can re-run in ours).
//!
//! Legs:
//!   1. EMIT+RECHECK — the int8 PE placement `psum + a·w` yields a certificate that `recheck()` accepts.
//!   2. CHECKER IS LOAD-BEARING — dropping the last refutation step makes the SAME checker REJECT it
//!      (so a green recheck is not vacuous — it genuinely replays the proof).
//!   3. FAIL-CLOSED — a WRONG datapath (`psum − a·w`) earns NO certificate (`certify_mapping` returns None):
//!      we never ship a proof for a mapping that isn't actually equivalent.
//!
//! Run: `cargo test -p logicaffeine-tests --test pnr_certificate_lock`
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::codegen_sva::fabric_grid::{grid_output_to_bounded, route_loop_body};
use logicaffeine_compile::codegen_sva::fabric_rtl::{
    fabric_to_bounded, AluOp, FabricOp, LoopCell, Src,
};
use logicaffeine_compile::codegen_sva::pnr_certificate::certify_mapping;
use logicaffeine_compile::codegen_sva::sva_to_verify::BoundedExpr;

/// The processing element as placed on the fabric: In0 = activation, In1 = incoming partial sum, In2 = weight;
/// it computes `psum + a·w`. This is the exact cell the systolic locks SAT-certify — here we keep the witness.
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

/// The same shape but WRONG: `psum − a·w`. A placement claiming this is not equivalent to the real PE.
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

/// The placed grid's output as a bounded (bit-blastable) expression, and the datapath spec `claim` computes.
/// Width `w` bits per operand.
fn grid_and_datapath(claim: &[FabricOp], result: usize, w: u32) -> (BoundedExpr, BoundedExpr) {
    let g = route_loop_body(&to_cells(&pe_body()), 3, &[Src::Val(1)]);
    let grid = grid_output_to_bounded(&g, 0, w, &|k| format!("in{k}"));
    let datapath = fabric_to_bounded(claim, result, w);
    (grid, datapath)
}

// ── Leg 1 + 2: emit a certificate, re-check it independently, and prove the checker is load-bearing ──

#[test]
fn pe_placement_ships_an_independently_rechecked_certificate() {
    // A modest width keeps the certificate small; the argument (a shipped, re-checkable proof) is width-agnostic.
    let w = 8;
    let (grid, datapath) = grid_and_datapath(&pe_body(), 1, w);
    let cert = certify_mapping(grid, datapath)
        .expect("the correct int8 PE placement must earn a certificate that `psum + a·w` holds for all inputs");

    // (1) an INDEPENDENT re-check accepts it — trusting only the PR/RUP checker, not the prover.
    assert!(
        cert.recheck(),
        "the emitted place-and-route certificate must re-verify independently"
    );
    assert!(
        !cert.is_empty() && cert.len() >= 1,
        "a real refutation carries steps (got {})",
        cert.len()
    );

    // (2) the checker is LOAD-BEARING: drop the closing step and the same checker must REJECT — proving the
    //     green re-check above is not vacuous (it genuinely replays the refutation to the empty clause).
    let tampered = cert.with_last_step_removed();
    assert!(
        !tampered.recheck(),
        "a certificate missing its closing step must FAIL re-check — else `recheck` proves nothing"
    );

    println!(
        "PnR-cert: int8 PE placement `psum + a·w` ships a {}-step certificate that re-verifies independently; \
         tampering is caught",
        cert.len()
    );
}

// ── Leg 3: fail-closed — a wrong mapping earns no certificate ──

#[test]
fn a_wrong_placement_earns_no_certificate() {
    let w = 8;
    // The grid computes `psum + a·w`, but we ask to certify it against the WRONG spec `psum − a·w`.
    let (grid, wrong_datapath) = grid_and_datapath(&wrong_pe_body(), 1, w);
    assert!(
        certify_mapping(grid, wrong_datapath).is_none(),
        "a mapping that is NOT equivalent to its spec must earn NO certificate — we never ship a false proof"
    );
    println!("PnR-cert: a non-equivalent placement (`psum − a·w`) is refused a certificate (fail-closed)");
}
