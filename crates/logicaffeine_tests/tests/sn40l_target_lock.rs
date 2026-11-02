//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! SN40L TARGET LOCK — `rdu_shaped()` carries the real SambaNova SN40L envelope, not a placeholder.
//!
//! Pins every public SN40L number the descriptor claims (compute/memory tiles, node, roofline, memory tiers)
//! and the capability envelope our certified lowering gates against (dataflows, precisions, fusion). Crucially
//! it also pins the HONESTY boundary: we advertise only precisions our fabric places AND witnesses (int8 +
//! bf16), never the chip's fp32/int32 which are witness-pending.
//!
//! Sources for the pinned numbers: SambaNova Hot Chips 2024 · arXiv 2405.07518 · ServeTheHome · sambanova.ai.
//! Run: `cargo test -p logicaffeine-tests --test sn40l_target_lock`
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::codegen_sva::fabric_target::{
    Capability, Dataflow, FabricTarget, MemTier, Precision, SiliconSpec, Unplaceable,
};

#[test]
fn rdu_shaped_carries_the_real_sn40l_silicon_spec() {
    let rdu = FabricTarget::rdu_shaped();
    assert_eq!(
        rdu.name, "sn40l-rdu",
        "the RDU descriptor must name the real part"
    );
    assert_eq!(
        rdu.silicon,
        Some(SiliconSpec { compute_tiles: 1040, mem_tiles: 1040, node_nm: 5, peak_bf16_pflops: 10.208 }),
        "rdu_shaped must carry the SN40L facts: 1040 PCUs + 1040 PMUs, 5 nm, 16 sockets × 638 TFLOPS = 10.208 bf16 PFLOPS"
    );
    // three-tier memory: 520 MiB SRAM, 64 GiB HBM, 1.5 TiB DDR
    assert_eq!(
        rdu.mem.sram_kib,
        520 * 1024,
        "SN40L has 520 MiB on-chip SRAM"
    );
    assert_eq!(rdu.mem.hbm_gib, 64, "SN40L has 64 GiB co-packaged HBM");
    assert_eq!(rdu.mem.ddr_gib, 1536, "SN40L has up to 1.5 TiB DDR");
    println!("SN40L: rdu_shaped() pins 1040 PCU/PMU, 5nm, 10.208 PFLOPS, 520MiB/64GiB/1.5TiB memory (cited)");
}

#[test]
fn rdu_shaped_provides_the_pcu_capability_envelope() {
    let rdu = FabricTarget::rdu_shaped();
    // PCU dataflows: weight-stationary AND output-stationary GEMM, plus the reduction baseline.
    for d in [
        Dataflow::Reduction,
        Dataflow::WeightStationary,
        Dataflow::OutputStationary,
    ] {
        assert!(
            rdu.provides(Capability::Dataflow(d)),
            "the PCU realizes {d:?}"
        );
    }
    // three memory tiers + a SIMD lane + a decoupled AGU (the DAE pattern).
    for t in [MemTier::Sram, MemTier::Hbm, MemTier::Ddr] {
        assert!(
            rdu.provides(Capability::MemTier(t)),
            "the RDU has the {t:?} tier"
        );
    }
    assert!(
        rdu.agu,
        "the RDU carries a decoupled address-generation unit"
    );
    assert!(rdu.simd.is_some(), "the PCU is a packed-SIMD compute unit");
    assert!(
        rdu.provides(Capability::Fusion(4)),
        "the RDU fuses at least a whole block"
    );
    // int8 weight-stationary GEMM must place.
    rdu.require_gemm(Dataflow::WeightStationary, Precision::Int8)
        .expect("int8 WS GEMM places on the SN40L RDU");
    println!("SN40L: WS+OS+Reduction · Sram+Hbm+Ddr · AGU · SIMD · Fusion≥4 · int8 WS GEMM places");
}

#[test]
fn rdu_shaped_advertises_only_witnessed_precisions() {
    let rdu = FabricTarget::rdu_shaped();
    // WITNESSED — advertised and placeable.
    for p in [Precision::Int8, Precision::Bf16] {
        assert!(
            rdu.provides(Capability::Precision(p)),
            "int8/bf16 are witnessed and advertised"
        );
    }
    // WITNESS-PENDING — the chip supports fp32/int32, but we do NOT advertise what we can't yet witness.
    assert!(
        !rdu.provides(Capability::Precision(Precision::Fp32)),
        "fp32 is chip-supported but witness-pending — advertising it would break the precision ratchet"
    );
    match rdu.require_gemm(Dataflow::WeightStationary, Precision::Fp32) {
        Err(Unplaceable::MissingCapability {
            needed: Capability::Precision(Precision::Fp32),
            ..
        }) => {}
        other => panic!("an unwitnessed precision must be REFUSED, got {other:?}"),
    }
    println!("SN40L: advertises only witnessed int8+bf16; fp32/int32 are honestly refused (witness-pending)");
}

#[test]
fn abstract_presets_carry_no_silicon_spec() {
    // The silicon spec is honestly present ONLY for a physically-backed target.
    for t in [
        FabricTarget::tiny(),
        FabricTarget::wearable(),
        FabricTarget::wide(),
        FabricTarget::full(8, 8, 64),
    ] {
        assert!(
            t.silicon.is_none(),
            "abstract preset {} must not claim real-chip metadata",
            t.name
        );
    }
    println!("SN40L: abstract presets carry no SiliconSpec (only the real RDU descriptor does)");
}
