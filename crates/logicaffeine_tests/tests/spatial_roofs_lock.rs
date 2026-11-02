//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! spatial_roofs_lock — every chip preset carries its roofs, and every roof says where it comes from.
//!
//! A roof is the rate a resource sustains: DRAM bytes per second, bytes per cycle over a network link,
//! matrix-engine operations per cycle. A program's work over a roof bounds its time from below, so a
//! roof that is too high hides a bottleneck and one that is too low invents one. Each roof is therefore a
//! `Fact`, and this lock holds the facts to their sources:
//!
//!   - a published roof quotes one of the preset's sources; a derived roof names its published inputs and
//!     the arithmetic; a modeled roof says what the record does not give; a converted rate is never
//!     stronger than the clock it went through;
//!   - the vendor numbers are pinned, and cross-checked against a second vendor source where one exists:
//!     the Tenstorrent DRAM roofs against the channel counts the SoC descriptors give
//!     (`tt_soc_facts.json`), the per-cycle matrix rate against the whole-card TFLOPS, the GA10x per-SM
//!     tensor rate against the RTX 3080 column of the same whitepaper;
//!   - a preset never inherits another chip's roofs (Blackhole is built `..wormhole()`), the RDU presets
//!     read their roofs from the RDU facts, and the Plasticine mesh reads the same HBM rate and clock;
//!   - our fabric carries no matrix roof: each program instantiates its own array, which the cycle twin
//!     times exactly, so a fixed figure would be a guess; its address lane carries one element per cycle
//!     whatever its width, so that roof is counted in elements;
//!   - each Plasticine mesh's roofs are its own bandwidth, clock and fp32 peak, under the provenance its
//!     record supports (Tungsten's bandwidth measured, on a read stream only).
//!
//!   cargo nextest run -p logicaffeine-tests --test spatial_roofs_lock
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::codegen_sva::fabric_target::Precision;
use logicaffeine_compile::codegen_sva::plasticine::PlasticineTarget;
use logicaffeine_compile::spatial::rdu_chip::{Fact, Provenance, RduChip, RduChipFacts};
use logicaffeine_compile::spatial::roofs::{
    Engine, MemLevel, Rate, Roofs, Scope, StreamMix, Transfer,
};
use logicaffeine_compile::spatial::target::SpatialTarget;

fn presets() -> Vec<SpatialTarget> {
    let mut all = SpatialTarget::presets();
    all.push(SpatialTarget::wormhole_ttsim());
    all
}

fn per_cycle(r: &Rate) -> f64 {
    match r {
        Rate::PerCycle(v) => *v,
        Rate::PerSecond(v) => panic!("expected a per-cycle rate, got {v} per second"),
    }
}

fn per_second(r: &Rate) -> f64 {
    match r {
        Rate::PerSecond(v) => *v,
        Rate::PerCycle(v) => panic!("expected a per-second rate, got {v} per cycle"),
    }
}

fn close(a: f64, b: f64, rel: f64) -> bool {
    (a - b).abs() <= rel * b.abs()
}

/// Every fact of `roofs` is cited by its provenance's rule, and every rate is a positive number.
fn assert_cited(name: &str, roofs: &Roofs) {
    let facts = roofs.facts();
    assert!(
        facts.len() >= 2,
        "{name}: a clock and at least one roof, got {facts:?}"
    );
    for (path, provenance, cite) in &facts {
        assert!(
            !cite.trim().is_empty(),
            "{name} `{path}`: an empty citation"
        );
        match provenance {
            Provenance::Published | Provenance::Derived => assert!(
                roofs.sources.iter().any(|s| cite.contains(s)),
                "{name} `{path}` is {provenance:?} but cites `{cite}`, which names none of its sources {:?}",
                roofs.sources
            ),
            Provenance::Measured => assert!(
                cite.contains("benchmarks/") && cite.contains(".json"),
                "{name} `{path}` is measured but cites `{cite}`, which names no run ledger"
            ),
            Provenance::Modeled => assert!(
                ["not public", "not published", "inherited", "not tabulated"].iter().any(|m| cite.contains(m)),
                "{name} `{path}` is modeled but its cite `{cite}` does not say what gap it fills"
            ),
        }
    }
    for r in &roofs.mem {
        let v = match r.rate.value {
            Rate::PerCycle(v) | Rate::PerSecond(v) => v,
        };
        assert!(
            v.is_finite() && v > 0.0,
            "{name} {:?}: a roof is a positive rate, got {v}",
            r.level
        );
    }
    for r in &roofs.compute {
        let v = match r.ops.value {
            Rate::PerCycle(v) | Rate::PerSecond(v) => v,
        };
        assert!(
            v.is_finite() && v > 0.0,
            "{name} {:?}: a roof is a positive rate, got {v}",
            r.precision
        );
    }
    assert!(roofs.clock_ghz.value > 0.0, "{name}: a positive clock");
}

#[test]
fn every_roof_of_every_preset_names_its_source() {
    for t in presets() {
        assert_cited(t.name, &t.roofs);
    }
}

#[test]
fn every_roofs_preset_in_the_catalog_names_its_source() {
    let catalog = Roofs::catalog();
    for t in presets() {
        assert!(
            catalog.iter().any(|(_, r)| *r == t.roofs) || t.name.starts_with("sn") || t.name.contains("ttsim"),
            "{}: a spatial target's roofs are a catalog preset",
            t.name
        );
    }
    for (name, roofs) in &catalog {
        assert_cited(name, roofs);
        assert!(
            roofs.mem(MemLevel::Dram, StreamMix::Read).is_some(),
            "{name}: every catalog preset prices the memory every site shares"
        );
    }
    let names: Vec<&str> = catalog.iter().map(|(n, _)| *n).collect();
    let mut sorted = names.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted.len(), names.len(), "one entry per preset: {names:?}");
}

#[test]
fn every_plasticine_mesh_carries_its_own_numbers_as_roofs() {
    use Provenance::*;
    for (t, dram, clock, peak) in [
        (
            PlasticineTarget::isca17(),
            (Published, StreamMix::Any),
            Published,
            Published,
        ),
        (
            PlasticineTarget::sara_20x20(),
            (Published, StreamMix::Any),
            Modeled,
            Derived,
        ),
        (
            PlasticineTarget::tungsten_20x20_ddr4(),
            (Measured, StreamMix::Read),
            Modeled,
            Derived,
        ),
        (
            PlasticineTarget::rdu(RduChip::Sn40l),
            (Published, StreamMix::Any),
            Modeled,
            Modeled,
        ),
        (
            PlasticineTarget::rdu(RduChip::Sn50),
            (Modeled, StreamMix::Any),
            Modeled,
            Modeled,
        ),
    ] {
        let roofs = t.roofs();
        assert_cited(t.name, &roofs);
        assert_eq!(
            (roofs.clock_ghz.value, roofs.clock_ghz.provenance),
            (t.clock_ghz, clock),
            "{}: the clock",
            t.name
        );
        let [mem] = roofs.mem.as_slice() else {
            panic!("{}: one DRAM roof, got {:?}", t.name, roofs.mem)
        };
        assert_eq!(
            (
                mem.level,
                mem.mix,
                mem.rate.provenance,
                per_second(&mem.rate.value)
            ),
            (MemLevel::Dram, dram.1, dram.0, t.dram.gbps * 1e9),
            "{}: the DRAM roof is the descriptor's bandwidth",
            t.name
        );
        let fp32 = roofs
            .compute(Engine::Vector, Precision::Fp32)
            .expect("the fp32 roofline");
        assert_eq!(
            (fp32.ops.provenance, per_second(&fp32.ops.value)),
            (peak, t.peak_fp32_tflops * 1e12),
            "{}: the fp32 peak",
            t.name
        );
    }
    // Tungsten measured a read stream; no roof for another mix is claimed on its behalf.
    let tungsten = PlasticineTarget::tungsten_20x20_ddr4().roofs();
    assert!(
        tungsten.mem(MemLevel::Dram, StreamMix::Write).is_none(),
        "a measured read rate is not a write roof"
    );
}

#[test]
fn every_preset_prices_the_memory_every_site_shares() {
    for t in presets() {
        assert!(
            t.roofs.mem(MemLevel::Dram, StreamMix::Read).is_some(),
            "{}: no roof on the memory every site shares — nothing could tell a bandwidth-bound program from a compute-bound one",
            t.name
        );
    }
}

#[test]
fn an_element_serial_lane_carries_one_element_per_cycle_whatever_its_width() {
    let fabric = Roofs::sky_fabric();
    let lane = fabric
        .mem(MemLevel::Dram, StreamMix::Read)
        .expect("the fabric's lane roof");
    assert_eq!(
        (lane.unit, per_cycle(&lane.rate.value), lane.rate.provenance),
        (Transfer::Elements, 1.0, Provenance::Derived),
        "one element per cycle, counted in elements"
    );
    let wh = Roofs::wormhole();
    assert_eq!(
        wh.mem(MemLevel::Dram, StreamMix::Read).map(|r| r.unit),
        Some(Transfer::Bytes),
        "a bus moves bytes whatever they hold"
    );
}

#[test]
fn a_matrix_engine_has_a_roof_at_a_precision_it_computes_except_the_fabric_that_builds_its_own() {
    for t in presets() {
        let roofed: Vec<Precision> = t
            .roofs
            .compute
            .iter()
            .filter(|r| r.engine == Engine::Matrix)
            .map(|r| r.precision)
            .collect();
        if t.name == "sky130-fabric" {
            assert!(roofed.is_empty(), "the fabric's array is sized per program and timed exactly by the twin; a fixed matrix roof would be a guess: {roofed:?}");
            continue;
        }
        assert!(
            roofed.iter().any(|p| t.matmul_supports(*p)),
            "{}: matrix roofs at {roofed:?}, none a precision the matrix engine computes",
            t.name
        );
    }
}

#[test]
fn the_tenstorrent_roofs_are_the_vendors_and_agree_with_its_soc_descriptors() {
    let soc: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../benchmarks/silicon/tt_soc_facts.json"),
        )
        .expect("benchmarks/silicon/tt_soc_facts.json"),
    )
    .expect("the SoC facts parse");
    // GDDR6 runs a 32-bit channel per device; the card pages give the data rate.
    for (t, chip, gts, gbps, ghz, noc) in [
        (
            SpatialTarget::wormhole(),
            "wormhole_b0",
            12.0,
            288e9,
            1.0,
            32.0,
        ),
        (
            SpatialTarget::blackhole(),
            "blackhole",
            16.0,
            512e9,
            1.35,
            64.0,
        ),
    ] {
        let dram = t
            .roofs
            .mem(MemLevel::Dram, StreamMix::Any)
            .expect("a DRAM roof");
        assert_eq!(
            (dram.rate.provenance, dram.scope),
            (Provenance::Published, Scope::Chip),
            "{}",
            t.name
        );
        assert_eq!(
            per_second(&dram.rate.value),
            gbps,
            "{}: the card's published bandwidth",
            t.name
        );
        let channels = soc["chips"][chip]["dram_channels"]
            .as_f64()
            .expect("dram_channels");
        assert_eq!(
            channels * 32.0 * gts * 1e9 / 8.0,
            gbps,
            "{}: {channels} channels x 32 bits x {gts} GT/s is not the published bandwidth",
            t.name
        );
        assert_eq!(
            (t.roofs.clock_ghz.value, t.roofs.clock_ghz.provenance),
            (ghz, Provenance::Published),
            "{}",
            t.name
        );
        let link = t
            .roofs
            .mem(MemLevel::Noc, StreamMix::Any)
            .expect("a NOC link roof");
        assert_eq!(
            (per_cycle(&link.rate.value), link.scope),
            (noc, Scope::Link),
            "{}: one flit per cycle",
            t.name
        );
        for p in [Precision::Bf16, Precision::Fp16] {
            let m = t.roofs.compute(Engine::Matrix, p).expect("a matrix roof");
            assert_eq!(
                (per_cycle(&m.ops.value), m.scope, m.ops.provenance),
                (4096.0, Scope::Site, Provenance::Published),
                "{} {p:?}",
                t.name
            );
        }
    }
    // One pass per cycle per engine reproduces the whole cards: n150s "TeraFLOPs (FP16): 74" on 72 cores at
    // 1 GHz is the four-pass HiFi4 rate, p150 "TeraFLOPS (BLOCKFP8): 664" on 120 cores at 1.35 GHz one pass.
    assert!(
        close(72.0 * 4096.0 / 4.0 * 1.0e9, 74e12, 0.005),
        "the Wormhole matrix rate no longer reproduces the n150's 74 TFLOPS"
    );
    assert!(
        close(120.0 * 4096.0 * 1.35e9, 664e12, 0.005),
        "the Blackhole matrix rate no longer reproduces the p150's 664 TFLOPS"
    );
}

#[test]
fn a_blackhole_never_inherits_wormholes_roofs() {
    let (wh, bh) = (SpatialTarget::wormhole(), SpatialTarget::blackhole());
    assert_ne!(
        wh.roofs, bh.roofs,
        "Blackhole is built `..wormhole()`; its roofs must be its own"
    );
    assert_eq!(
        SpatialTarget::wormhole_ttsim().roofs,
        wh.roofs,
        "the simulated grid is the same part"
    );
}

#[test]
fn the_rtx3060_tensor_roofs_are_derived_from_the_ga10x_sm_and_reproduce_the_whitepaper() {
    let t = SpatialTarget::rtx3060();
    // The GA10x SM the whitepaper tabulates, checked against its own RTX 3080 column: 68 SMs at 1710 MHz.
    let (sm_fp16_acc, sm_fp32_acc, sm_int8) = (1024.0, 512.0, 2048.0);
    assert!(
        close(68.0 * sm_fp16_acc * 1.71e9, 119e12, 0.005),
        "FP16 tensor with FP16 accumulate"
    );
    assert!(
        close(68.0 * sm_fp32_acc * 1.71e9, 59.5e12, 0.005),
        "FP16 tensor with FP32 accumulate"
    );
    assert!(close(68.0 * sm_int8 * 1.71e9, 238e12, 0.005), "INT8 tensor");
    let sms = 3584.0 / 128.0;
    for (p, per_sm) in [
        (Precision::Bf16, sm_fp32_acc),
        (Precision::Fp16, sm_fp32_acc),
        (Precision::Int8, sm_int8),
    ] {
        let m = t.roofs.compute(Engine::Matrix, p).expect("a tensor roof");
        assert_eq!(
            (per_cycle(&m.ops.value), m.scope, m.ops.provenance),
            (sms * per_sm, Scope::Chip, Provenance::Derived),
            "{p:?}"
        );
    }
    let bf16 = t.roofs.per_second(
        &t.roofs
            .compute(Engine::Matrix, Precision::Bf16)
            .unwrap()
            .ops,
    );
    assert!(
        close(bf16.value, 25.52e12, 0.001),
        "28 SMs x 512 x 1.78 GHz, got {}",
        bf16.value
    );
    let smem = t
        .roofs
        .mem(MemLevel::Local, StreamMix::Any)
        .expect("a shared-memory roof");
    assert_eq!(
        (per_cycle(&smem.rate.value), smem.rate.provenance),
        (sms * 128.0, Provenance::Derived)
    );
    let dram = t
        .roofs
        .mem(MemLevel::Dram, StreamMix::Any)
        .expect("a DRAM roof");
    assert_eq!(
        (per_second(&dram.rate.value), dram.rate.provenance),
        (360e9, Provenance::Modeled),
        "NVIDIA publishes no data rate"
    );
}

#[test]
fn the_rdu_roofs_are_the_rdu_facts() {
    for chip in RduChip::ALL {
        let facts = chip.facts();
        let t = facts.spatial_target();
        assert_eq!(
            t.roofs,
            facts.roofs(),
            "{chip:?}: the preset's roofs ARE the facts'"
        );
        let bf16 = t
            .roofs
            .compute(Engine::Matrix, Precision::Bf16)
            .expect("the published bf16 peak");
        assert_eq!(
            (per_second(&bf16.ops.value), bf16.ops.provenance),
            (
                facts.peak_bf16_tflops.value as f64 * 1e12,
                Provenance::Published
            ),
            "{chip:?}"
        );
        assert_eq!(
            t.roofs
                .compute(Engine::Matrix, Precision::Fp8)
                .map(|r| per_second(&r.ops.value)),
            facts.peak_fp8_tflops.map(|f| f.value as f64 * 1e12),
            "{chip:?}: an FP8 roof exactly where the vendor publishes one"
        );
        assert!(
            t.roofs.compute(Engine::Matrix, Precision::Int8).is_none(),
            "{chip:?}: no INT8 rate is published"
        );
        let hbm = t
            .roofs
            .mem(MemLevel::Dram, StreamMix::Any)
            .expect("the HBM roof");
        assert_eq!(
            (per_second(&hbm.rate.value), hbm.rate.provenance),
            (1.6e12, if chip == RduChip::Sn40l { Provenance::Published } else { Provenance::Modeled }),
            "{chip:?}"
        );
        assert_eq!(
            (t.roofs.clock_ghz.value, t.roofs.clock_ghz.provenance),
            (1.0, Provenance::Modeled),
            "{chip:?}"
        );
        // The Plasticine mesh descriptor reads the same HBM rate and clock.
        let mesh = PlasticineTarget::rdu(chip);
        assert_eq!(
            (mesh.dram.gbps, mesh.clock_ghz),
            (facts.hbm_gbps().value, facts.clock_ghz().value),
            "{chip:?}"
        );
    }
    assert_eq!(
        RduChipFacts::sn50().roofs(),
        SpatialTarget::sn50_rdu().roofs
    );
}

#[test]
fn a_converted_rate_is_never_stronger_than_the_clock_it_went_through() {
    let wh = Roofs::wormhole();
    let dram = wh.per_cycle(&wh.mem(MemLevel::Dram, StreamMix::Read).unwrap().rate);
    assert_eq!(
        (dram.value, dram.provenance),
        (288.0, Provenance::Published),
        "288 GB/s at a published 1 GHz"
    );
    let sn50 = RduChipFacts::sn50().roofs();
    let bf16 = sn50.per_cycle(&sn50.compute(Engine::Matrix, Precision::Bf16).unwrap().ops);
    assert_eq!(
        bf16.provenance,
        Provenance::Modeled,
        "a published peak over a modeled clock is modeled per cycle"
    );
    assert_eq!(bf16.value, 1600e12 / 1e9);
    let fabric = Roofs::sky_fabric();
    let lane = fabric.per_second(&fabric.mem(MemLevel::Dram, StreamMix::Copy).unwrap().rate);
    assert_eq!(
        lane.provenance,
        Provenance::Modeled,
        "a derived rate at a modeled clock is modeled per second"
    );
    assert!(
        Provenance::Published < Provenance::Derived
            && Provenance::Derived < Provenance::Measured
            && Provenance::Measured < Provenance::Modeled
    );
}

#[test]
fn a_mix_falls_back_to_the_peak_and_a_missing_level_has_no_roof() {
    let wh = Roofs::wormhole();
    for mix in [
        StreamMix::Read,
        StreamMix::Write,
        StreamMix::Copy,
        StreamMix::Triad,
    ] {
        assert_eq!(
            wh.mem(MemLevel::Dram, mix).map(|r| r.mix),
            Some(StreamMix::Any),
            "{mix:?} is bounded by the bus peak"
        );
    }
    assert!(
        wh.mem(MemLevel::Local, StreamMix::Any).is_none(),
        "no L1 rate is published, so none is claimed"
    );
    let mut measured = wh.clone();
    measured
        .mem
        .push(logicaffeine_compile::spatial::roofs::MemRoof {
            level: MemLevel::Dram,
            mix: StreamMix::Triad,
            scope: Scope::Chip,
            unit: Transfer::Bytes,
            rate: Fact::measured(
                Rate::PerSecond(250e9),
                "benchmarks/results/roofs.json: STREAM triad",
            ),
        });
    assert_eq!(
        measured
            .mem(MemLevel::Dram, StreamMix::Triad)
            .map(|r| r.mix),
        Some(StreamMix::Triad),
        "a mix measured for itself wins"
    );
    assert_eq!(
        measured.mem(MemLevel::Dram, StreamMix::Copy).map(|r| r.mix),
        Some(StreamMix::Any)
    );
}
