//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! IMPORT-AND-CERTIFY LOCK — certify a placement produced by an EXTERNAL tool against the English source.
//!
//! The SDK-facing half of proof-carrying place-and-route: when a vendor flow (e.g. SambaFlow) hands us a
//! placed grid, `certify_external_placement` proves it computes the intended `source` datapath for every input
//! and ships a re-checkable certificate — trusting only the PR/RUP checker, not the vendor's placer. Here the
//! "external" grid is produced by our own router as a stand-in (a real SDK would deserialize its placement into
//! the same `Grid`); the API accepts ANY grid regardless of origin.
//!
//!   1. ACCEPT — an external placement that computes `psum + a·w` earns a certificate that `recheck()` accepts.
//!   2. LOAD-BEARING — dropping the closing step makes the SAME checker reject (green re-check isn't vacuous).
//!   3. FAIL-CLOSED — certifying that placement against a WRONG source (`psum − a·w`) returns None.
//!
//! Run: `cargo test -p logicaffeine-tests --test certify_external_placement_lock`
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::codegen_sva::fabric_grid::route_loop_body;
use logicaffeine_compile::codegen_sva::fabric_rtl::{AluOp, FabricOp, LoopCell, Src};
use logicaffeine_compile::codegen_sva::pnr_certificate::certify_external_placement;

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

#[test]
fn an_external_placement_is_certified_against_the_source() {
    let w = 8;
    // Stand-in for a vendor-produced placement: a Grid that computes `psum + a·w`.
    let external = route_loop_body(&to_cells(&pe_body()), 3, &[Src::Val(1)]);

    // (1) certify it against the ENGLISH source it should compute.
    let cert = certify_external_placement(&external, &pe_body(), 1, w)
        .expect("an external placement equal to the source must earn a certificate");
    assert!(
        cert.recheck(),
        "the external-placement certificate must re-verify independently"
    );
    assert!(!cert.is_empty(), "a real refutation carries steps");

    // (2) the checker is load-bearing.
    assert!(
        !cert.with_last_step_removed().recheck(),
        "a certificate missing its closing step must FAIL re-check"
    );

    // (3) FAIL-CLOSED: the same placement certified against a WRONG source earns no certificate.
    assert!(
        certify_external_placement(&external, &wrong_pe_body(), 1, w).is_none(),
        "an external placement that does NOT match the source must earn NO certificate (fail-closed)"
    );
    println!(
        "import-and-certify: a vendor-shaped placement is proven ≡ its English source ({}-step cert, re-checked); \
         a mismatched source is refused",
        cert.len()
    );
}
