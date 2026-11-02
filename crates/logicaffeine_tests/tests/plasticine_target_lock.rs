//! ═══════════════════════════════════════════════════════════════════════
//! PLASTICINE TARGET LOCK — P0 of the Plasticine lane: the open RDU (Prabhakar et al., ISCA 2017) as a
//! described `FabricTarget` + a `PlasticineTarget` mesh descriptor, every number cited, and the certified
//! placer retargeted onto it.
//!
//! A. `FabricTarget::plasticine_shaped()` carries the ISCA'17 envelope — 16×8 checkerboard of 64 PCUs + 64
//!    PMUs, 28 nm, PCU = 16 lanes × 6 stages, PMU = 16 banks × 16 KB = 256 KB, 16 MB scratchpad, 4 DDR3-1600
//!    channels at 51.2 GB/s — and NO bf16 datapath (peak bf16 is honestly 0; the 12.3 fp32 TFLOPS live on the
//!    mesh descriptor under their own name).
//! B. Its capability envelope is what the chip IS: the PCU's SIMD pipeline + reduction tree = `Reduction`;
//!    weight-/output-stationary systolic modes are REFUSED, typed; only WITNESSED precisions are advertised
//!    (fp32 — witnessed by `precision_gemm_lock`), never an int8 MAC the 32-bit FUs do not have.
//! C. The new spatial capabilities are gated: `Metapipeline(6)` (six PCU stages), `Banks(16)`, `CounterNest`.
//! D. `place_with_target` is SAT-certified semantics-preserving over the WHOLE bounded family on the
//!    descriptor (the theorem `fabric_target_lock` proves, retargeted).
//! E. The two mesh presets differ exactly where the papers differ (ISCA'17 16×8 vs SARA/Capstan 20×20), and
//!    the backend certifies a placement today while labeling its emission honestly (Spatial/PIR = P4).
//!
//! Honest boundary: MODELED from published parameters; nothing here is a Plasticine simulation or silicon.
//!
//! Run: `cargo nextest run -p logicaffeine-tests -E 'binary(plasticine_target_lock)'`
//! ═══════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

#[path = "common/bounded_family.rs"]
mod bounded_family;

use logicaffeine_compile::codegen_sva::backend::{BackendArtifact, TargetBackend};
use logicaffeine_compile::codegen_sva::fabric_rtl::Src;
use logicaffeine_compile::codegen_sva::fabric_target::{
    place_with_target, Capability, Dataflow, FabricTarget, MemTier, Precision, SiliconSpec,
    Unplaceable,
};
use logicaffeine_compile::codegen_sva::plasticine::{PlasticineBackend, PlasticineTarget};

#[test]
fn plasticine_shaped_carries_the_isca17_envelope() {
    let p = FabricTarget::plasticine_shaped();
    assert_eq!(p.name, "plasticine-isca17");
    // One PCU datapath envelope: 6 pipeline stages × 16 SIMD lanes of 32-bit FUs (ISCA'17 Table 3, §3.1).
    assert_eq!((p.rows, p.cols, p.width), (6, 16, 32));
    assert_eq!(
        p.silicon,
        Some(SiliconSpec { compute_tiles: 64, mem_tiles: 64, node_nm: 28, peak_bf16_pflops: 0.0 }),
        "16x8 array = 64 PCUs + 64 PMUs at 28 nm; Plasticine has NO bf16 datapath, so bf16 peak is 0"
    );
    assert_eq!(
        p.mem.sram_kib,
        16 * 1024,
        "64 PMUs x 256 KB = 16 MB scratchpad"
    );
    assert_eq!(p.mem.hbm_gib, 0, "no HBM");
    assert_eq!(
        p.mem_depth,
        256 * 1024 / 4,
        "one PMU = 256 KB = 65536 32-bit words"
    );
    assert!(p.agu, "the PMU carries an address-calculation datapath");
    assert_eq!(
        p.metapipeline_depth, 6,
        "six PCU stages per metapipeline stage"
    );
    assert_eq!(p.banks, 16, "16 scratchpad banks = the number of PCU lanes");
    assert_eq!(
        p.fusion_window, 6,
        "up to six chained FU stages fuse inside one PCU"
    );
    println!("plasticine_shaped: 64 PCU + 64 PMU, 28nm, 6x16 PCU, 16 banks x 16 KB, 16 MB, no bf16 (cited)");
}

#[test]
fn plasticine_provides_reduction_and_refuses_systolic_modes_typed() {
    let p = FabricTarget::plasticine_shaped();
    assert!(
        p.provides(Capability::Dataflow(Dataflow::Reduction)),
        "the PCU is a SIMD pipeline with a cross-lane reduction tree"
    );
    for d in [Dataflow::WeightStationary, Dataflow::OutputStationary] {
        assert!(
            !p.provides(Capability::Dataflow(d)),
            "Plasticine is not a systolic array: {d:?} must be refused"
        );
        match p.require_gemm(d, Precision::Fp32) {
            Err(Unplaceable::MissingCapability {
                needed: Capability::Dataflow(x),
                target,
            }) => {
                assert_eq!(x, d);
                assert_eq!(target, "plasticine-isca17");
            }
            other => panic!("a systolic mode must be REFUSED typed on Plasticine, got {other:?}"),
        }
    }
    p.require_gemm(Dataflow::Reduction, Precision::Fp32)
        .expect("an fp32 reduction GEMM places on Plasticine");
    assert!(
        p.provides(Capability::MemTier(MemTier::Sram))
            && !p.provides(Capability::MemTier(MemTier::Hbm))
    );
    assert!(p.simd.is_some(), "the PCU is a 16-lane SIMD unit");
}

#[test]
fn plasticine_advertises_only_witnessed_precisions() {
    let p = FabricTarget::plasticine_shaped();
    assert!(
        p.provides(Capability::Precision(Precision::Fp32)),
        "fp32 is the PCU's native format and is witnessed"
    );
    assert!(
        FabricTarget::full(8, 8, 64).provides(Capability::Precision(Precision::Fp32)),
        "the superset target advertises the fp32 witness too"
    );
    for p8 in [
        Precision::Int8,
        Precision::Bf16,
        Precision::Fp16,
        Precision::Fp8,
    ] {
        assert!(
            !p.provides(Capability::Precision(p8)),
            "{p8:?}: the 32-bit FUs have no such MAC — not advertised"
        );
    }
    match p.require_gemm(Dataflow::Reduction, Precision::Int8) {
        Err(Unplaceable::MissingCapability {
            needed: Capability::Precision(Precision::Int8),
            ..
        }) => {}
        other => panic!("int8 must be refused typed on Plasticine, got {other:?}"),
    }
}

#[test]
fn plasticine_metapipeline_and_banking_capabilities_are_gated() {
    let p = FabricTarget::plasticine_shaped();
    assert!(p.provides(Capability::Metapipeline(6)) && !p.provides(Capability::Metapipeline(7)));
    assert!(p.provides(Capability::Banks(16)) && !p.provides(Capability::Banks(17)));
    assert!(
        p.provides(Capability::CounterNest(1)),
        "at least one counter is certain"
    );
    // Every abstract preset keeps the trivial values (no metapipeline / single bank) — the new dials do not leak.
    for t in [
        FabricTarget::tiny(),
        FabricTarget::wearable(),
        FabricTarget::wide(),
    ] {
        assert!(
            t.provides(Capability::Metapipeline(1)) && !t.provides(Capability::Metapipeline(2)),
            "{}",
            t.name
        );
        assert!(t.provides(Capability::Banks(1)), "{}", t.name);
    }
    // The RDU descriptor reads the six-stage / sixteen-bank unit model inherited from this chip — modeled and
    // cited on `RduChipFacts`, pinned by `rdu_unit_model_lock` — no longer the 1/1 placeholder.
    let rdu = FabricTarget::rdu_shaped();
    assert!(
        rdu.provides(Capability::Metapipeline(6)) && !rdu.provides(Capability::Metapipeline(7)),
        "{}",
        rdu.name
    );
    assert!(
        rdu.provides(Capability::Banks(16)) && !rdu.provides(Capability::Banks(17)),
        "{}",
        rdu.name
    );
}

#[test]
fn place_with_target_is_certified_over_the_whole_bounded_family_on_plasticine() {
    let p = FabricTarget::plasticine_shaped();
    let (mr, mc) = bounded_family::family_envelope();
    assert!(
        mr <= p.rows && mc <= p.cols,
        "the bounded family ({mr}x{mc}) must fit one PCU envelope ({}x{})",
        p.rows,
        p.cols
    );
    let total = bounded_family::certify_whole_family_on(&p);
    assert!(
        total >= 1000,
        "the certified family must be genuinely large (got {total})"
    );
    println!("Plasticine: place_with_target certified semantics-preserving over ALL {total} bodies on the 6x16 PCU envelope");
}

#[test]
fn the_two_mesh_presets_differ_where_the_papers_differ() {
    let a = PlasticineTarget::isca17();
    let b = PlasticineTarget::sara_20x20();
    assert_eq!(
        (a.pcu_rows, a.pcu_cols),
        (16, 8),
        "ISCA'17: 16x8 checkerboard"
    );
    assert_eq!((b.pcu_rows, b.pcu_cols), (20, 20), "SARA/Capstan: 20x20");
    assert_eq!(
        (a.pcu_lanes, a.pcu_stages, a.pcu_regs_per_stage),
        (16, 6, 6)
    );
    assert_eq!(
        (b.pcu_lanes, b.pcu_stages, b.pcu_regs_per_stage),
        (16, 6, 8)
    );
    assert_eq!((a.pmu_banks, a.pmu_kib, a.pmu_stages), (16, 256, 4));
    assert_eq!((b.pmu_banks, b.pmu_kib, b.pmu_stages), (16, 256, 10));
    assert_eq!(a.dram.channels, 4);
    assert!(
        (a.dram.gbps - 51.2).abs() < 1e-9,
        "4 x DDR3-1600 = 51.2 GB/s"
    );
    assert!((b.dram.gbps - 1000.0).abs() < 1e-9, "SARA: HBM2 at 1 TB/s");
    assert!((a.peak_fp32_tflops - 12.3).abs() < 1e-9 && a.clock_ghz == 1.0);
    assert!((a.area_mm2 - 112.8).abs() < 0.1 && (b.area_mm2 - 352.0).abs() < 0.1);
    // Both wrap the same certified FabricTarget seam (Deref) — one placer, two meshes.
    assert_eq!(a.name, "plasticine-isca17");
    assert!(
        a.provides(Capability::Dataflow(Dataflow::Reduction))
            && b.provides(Capability::Dataflow(Dataflow::Reduction))
    );
    assert_eq!(a.pcus(), 64);
    assert_eq!(b.pcus(), 200, "a 20x20 checkerboard is 200 PCUs + 200 PMUs");
    assert!(
        a.cite.contains("ISCA") && b.cite.contains("SARA"),
        "every preset names its source"
    );
}

#[test]
fn the_plasticine_backend_certifies_placements_and_labels_its_emission_honestly() {
    let be = PlasticineBackend::isca17();
    assert_eq!(be.name(), "plasticine-isca17");
    let (body, result) = bounded_family::enumerate_bodies(2)
        .into_iter()
        .next()
        .unwrap();
    let cells = bounded_family::to_cells(&body);
    let g = place_with_target(
        &cells,
        bounded_family::N_IN,
        &[Src::Val(result)],
        be.target(),
    )
    .unwrap();
    assert!(
        be.certify(&g, &body, result, bounded_family::W).is_some(),
        "the placement carries a SAT certificate today"
    );
    match be.emit_grid("t", &g) {
        BackendArtifact::SdkGated {
            neutral_placement,
            reason,
        } => {
            assert!(
                neutral_placement.starts_with("grid rows="),
                "the neutral placement is the ready-to-bridge dump"
            );
            assert!(
                reason.contains("Spatial") && reason.contains("P4"),
                "the reason names the open toolchain deliverable: {reason}"
            );
        }
        other => {
            panic!("P0 emits the neutral placement, not a fabricated Spatial program: {other:?}")
        }
    }
}
