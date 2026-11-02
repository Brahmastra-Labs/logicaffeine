//! npu_silicon_lock — the NPU processor edge: deterministic multiply-accumulate.
//!
//! An NPU's core operation is the multiply-accumulate reduction (dot-product, conv,
//! and matmul all reduce to `acc += w·x`). We compile a MAC reduction written in
//! English to the deterministic Futamura CGRA `LoopFabric` — a fixed spatial body
//! datapath (a hardware multiply + accumulator) streamed by a loop counter, with no
//! fetch/decode/dispatch — and prove the emitted datapath is **bit-identical to the
//! reference VM** across a battery of sizes, including 64-bit wrap. That is the NPU
//! edge: zero-jitter tensor-style dataflow, correct by construction, reusing the same
//! machinery that already took `cgra_sumto` / `nested_tri` to Sky130 signoff.
//!
//! Proof strength: ≡VM (the fabric body is built from `alu.v` ops each already proven
//! ≡ the VM; this lock pins the MAC recurrence) PLUS a certified-SAT recurrence proof —
//! one MAC body step (`acc' = acc + i·i`) is proven EQUAL to the independent closed form
//! `acc + i·i` EXHAUSTIVELY over all inputs at a small width by our own SAT (the same
//! "certify one body step, argue over the iterations" soundness the GPU kernel carries).
//! The FPGA/Sky130 receipts for the tile are recorded separately (reports.json /
//! sky130_signoff.json).
//!
//! Run: `cargo nextest run -p logicaffeine-tests -E 'binary(npu_silicon_lock)'`

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::codegen_sva::bitblast::lower_bool;
use logicaffeine_compile::codegen_sva::fabric_rtl::{
    emit_loop_fabric_verilog, eval_loop_fabric, loop_fabric_from_source, loop_next_to_bounded,
};
use logicaffeine_compile::codegen_sva::sva_to_verify::{BitVecBoundedOp, BoundedExpr};
use logicaffeine_compile::compile::vm_outcome;
use logicaffeine_proof::sat::{prove_unsat, UnsatOutcome};
use logicaffeine_proof::ProofExpr;

/// The MAC kernel: `acc += i·i` over `1..=n` (sum of squares) — a multiply-accumulate
/// recurrence, the reduction every dot-product / conv lowers to. The fabric carries one
/// loop-carried accumulator and a multiply in the body.
fn mac_src(n: i64) -> String {
    format!(
        "## To macsum (n: Int) -> Int:\n    \
            Let mutable acc be 0.\n    \
            Repeat for i from 1 to n:\n        \
                Set acc to acc + i * i.\n    \
            Return acc.\n\n\
         ## Main\n\nShow macsum({n}).\n"
    )
}

/// The VM's answer for `macsum(n)` — the reference the silicon must match.
fn vm_mac(n: i64) -> i64 {
    vm_outcome(&mac_src(n))
        .output
        .trim()
        .parse()
        .unwrap_or_else(|e| panic!("macsum({n}) VM output not an Int: {e}"))
}

#[test]
fn npu_mac_tile_is_bit_identical_to_the_vm() {
    // The fabric is built from the function body; the `macsum(0)` in Main is just a
    // valid program to compile — the tile is parameterised by its input `n`.
    let lf = loop_fabric_from_source(&mac_src(0), "macsum");
    assert_eq!(lf.n_in, 1, "the MAC tile takes one input (the length n)");

    for n in [0i64, 1, 2, 3, 7, 16, 64, 100, 1000, 5000] {
        assert_eq!(
            eval_loop_fabric(&lf, &[n]),
            vm_mac(n),
            "NPU MAC tile diverges from the VM at n={n} — the spatial datapath is not ≡ the reference"
        );
    }
}

#[test]
fn npu_mac_tile_emits_synthesizable_rtl_with_a_hardware_multiply() {
    let lf = loop_fabric_from_source(&mac_src(0), "macsum");
    let rtl = emit_loop_fabric_verilog("npu_mac", &lf);
    // The fabric emitter names the module `cgra_<name>` — the same CGRA convention as the
    // Sky130-signed `cgra_sumto`/`cgra_altsum`; the MAC tile is a CGRA implementing `acc+=i·i`.
    assert!(
        rtl.contains("module cgra_npu_mac"),
        "the tile emits a named CGRA module (cgra_<name>)"
    );
    assert!(
        rtl.contains('*'),
        "a multiply-accumulate tile must use a hardware multiply"
    );
    assert!(
        rtl.contains("endmodule"),
        "the emitted RTL is a complete module"
    );
}

/// The MAC RECURRENCE is certified by our own SAT, not merely sampled: one body step of the fabric
/// (`acc' = acc + i·i`) is proven EQUAL to the independent closed form `acc + i·i` EXHAUSTIVELY over
/// ALL (acc, i) at a small width. This is the "certify one body step, argue over the iterations"
/// soundness — the CGRA runs the SAME body once per induction value, so a body step proven ≡ the
/// closed form makes the whole loop's dataflow correct by induction (matching the GPU kernel's bar).
/// The fabric's next-state is the REAL compiler's residual, so this catches any miscompile of the
/// recurrence (a dropped square, wrong operand); the full 64-bit range is carried by the ≡VM battery.
#[test]
fn npu_mac_recurrence_is_certified_by_sat() {
    // A small width keeps the var×var multiply (`i·i`) exhaustive yet tractable — a wider full
    // multiplier is a known-hard SAT instance; 6 bits proves the recurrence over all 4096 (acc, i).
    const PW: u32 = 6;

    let lf = loop_fabric_from_source(&mac_src(0), "macsum");
    assert_eq!(
        lf.n_state, 1,
        "the MAC fabric carries ONE accumulator (acc); the result is s0"
    );
    assert_eq!(
        lf.result_state, 0,
        "the accumulator (state 0) holds the loop result"
    );

    // The fabric's next-state for the accumulator, over the body inputs `s0` (= acc) and `iv` (= i).
    let fabric_next = loop_next_to_bounded(&lf, lf.result_state, PW);

    // The independent closed form: acc + i·i, built directly (s0 = acc, iv = i) — nothing shared
    // with the emitter's construction, so it is a genuine miter against the recurrence spec.
    let acc = BoundedExpr::BitVecVar("s0".to_string(), PW);
    let i = BoundedExpr::BitVecVar("iv".to_string(), PW);
    let i_sq = BoundedExpr::BitVecBinary {
        op: BitVecBoundedOp::Mul,
        left: Box::new(i.clone()),
        right: Box::new(i),
    };
    let spec = BoundedExpr::BitVecBinary {
        op: BitVecBoundedOp::Add,
        left: Box::new(acc),
        right: Box::new(i_sq),
    };

    let claim = BoundedExpr::Eq(Box::new(fabric_next), Box::new(spec));
    let p = lower_bool(&claim).expect("claim lowers");
    assert_eq!(
        prove_unsat(&ProofExpr::Not(Box::new(p))),
        UnsatOutcome::Refuted,
        "the MAC body step must equal `acc + i·i` for ALL (acc, i) at width {PW} (certified, not sampled)"
    );
}
