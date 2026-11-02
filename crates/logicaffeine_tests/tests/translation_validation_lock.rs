//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! TRANSLATION VALIDATION LOCK — Phase 7b of the SambaNova-parity grind (the moat).
//!
//! The pitch to SambaNova is a PROOF-CARRYING place-and-route: given a source dataflow graph and a mapping of
//! it onto a tile array, machine-check that the mapping computes the source's function for ALL inputs — the
//! certificate SambaFlow's PnR does not carry. This lock proves the translation-validation ENGINE:
//!
//!   A. SOUND: over a battery of source datapaths, the mapping (a placed grid) is certified ≡ the source for
//!      all inputs, by our own SAT (`prove_unsat` refutes the negated equivalence).
//!   B. CATCHES ERRORS (non-vacuous): a mapping placed for one datapath does NOT certify as a DIFFERENT
//!      source — a wrong mapping is refused, so the certificate has teeth.
//!
//! Applying this to a vendor's actual Spatial-IR mapping is the integration (gated on toolchain
//! access); the ENGINE it rides is proven here.
//!
//! Run: `cargo test -p logicaffeine-tests --test translation_validation_lock`
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::codegen_sva::bitblast::lower_bool;
use logicaffeine_compile::codegen_sva::fabric_grid::{grid_output_to_bounded, route_loop_body};
use logicaffeine_compile::codegen_sva::fabric_rtl::{
    fabric_to_bounded, AluOp, FabricOp, LoopCell, Src,
};
use logicaffeine_compile::codegen_sva::sva_to_verify::BoundedExpr;
use logicaffeine_proof::sat::{prove_unsat, UnsatOutcome};
use logicaffeine_proof::ProofExpr;

const OPS: &[AluOp] = &[
    AluOp::Add,
    AluOp::Sub,
    AluOp::Mul,
    AluOp::And,
    AluOp::Or,
    AluOp::Xor,
];
const W: u32 = 4;

fn to_cells(body: &[FabricOp]) -> Vec<LoopCell> {
    body.iter()
        .map(|o| LoopCell::Arith {
            op: o.op,
            a: o.a,
            b: o.b,
        })
        .collect()
}

/// The translation-validation certificate: does the MAPPING (placed grid of `source`) compute `claim`?
fn mapping_certifies_as(source: &[FabricOp], claim: &[FabricOp], result: usize) -> bool {
    let g = route_loop_body(&to_cells(source), 2, &[Src::Val(result)]);
    let mapping = grid_output_to_bounded(&g, 0, W, &|k| format!("in{k}"));
    let claim_dp = fabric_to_bounded(claim, result, W);
    let eq = BoundedExpr::Eq(Box::new(mapping), Box::new(claim_dp));
    let p = lower_bool(&eq).expect("equivalence claim lowers");
    prove_unsat(&ProofExpr::Not(Box::new(p))) == UnsatOutcome::Refuted
}

#[test]
fn translation_validation_is_sound_over_a_battery() {
    // A. Every source's own mapping certifies as itself, over a 2-op family across the op set.
    let mut checked = 0usize;
    for &op0 in OPS {
        for &op1 in OPS {
            let source = vec![
                FabricOp {
                    op: op0,
                    a: Src::In(0),
                    b: Src::In(1),
                },
                FabricOp {
                    op: op1,
                    a: Src::Val(0),
                    b: Src::In(0),
                },
            ];
            assert!(
                mapping_certifies_as(&source, &source, 1),
                "the mapping of {source:?} must certify ≡ its own source"
            );
            checked += 1;
        }
    }
    assert!(
        checked >= 30,
        "the TV battery must be genuinely large (got {checked})"
    );
    println!("A: translation validation SOUND over {checked} source→mapping pairs (SAT-certified for all inputs)");
}

#[test]
fn translation_validation_catches_a_wrong_mapping() {
    // B. A mapping placed for `In0 + In1` must NOT certify as `In0 - In1` (they differ when In1 != 0).
    let source = vec![FabricOp {
        op: AluOp::Add,
        a: Src::In(0),
        b: Src::In(1),
    }];
    let wrong = vec![FabricOp {
        op: AluOp::Sub,
        a: Src::In(0),
        b: Src::In(1),
    }];
    assert!(
        mapping_certifies_as(&source, &source, 0),
        "sanity: the correct mapping certifies"
    );
    assert!(
        !mapping_certifies_as(&source, &wrong, 0),
        "a mapping for `In0+In1` must NOT certify as `In0-In1` — the certificate would be vacuous"
    );
    println!("B: translation validation CATCHES a wrong mapping (non-vacuous certificate)");
}
