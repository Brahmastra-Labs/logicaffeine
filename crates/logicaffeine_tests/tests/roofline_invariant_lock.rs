//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! roofline_invariant_lock — nothing the compiler predicts or the simulators measure beats a roof.
//!
//! A GEMM must read each operand and write its result at least once, so the bytes it moves are bounded
//! below by `(MK + KN + MN)` elements, and its time by those bytes over the bandwidth roof and its
//! arithmetic over the compute roof. A number below that floor is a wrong byte count or a wrong roof, never
//! a fast program (the roofline model's own consistency check: Williams, Waterman & Patterson, CACM 2009).
//! The floor is reachable — at 128³ the compiler's blocked Wormhole plan moves 98,312 bytes against a floor
//! of 98,304, the difference two four-byte semaphores — so it bites.
//!
//!   - every Tenstorrent ledger row the work meter measured moves at least its compulsory bytes;
//!   - every GEMM the pipeline plans moves at least its compulsory DRAM bytes, on every preset with a
//!     matrix engine;
//!   - every Plasticine report row stays under its target's compute roof, and its DRAM cycles are its
//!     bytes over the target's DRAM roof.
//!
//! ttsim measures no time (`tenstorrent_gemm.json`'s `clock_validity`), so its rows answer to the byte
//! floor only; time roofs apply where the clock is real or exact.
//!
//!   cargo nextest run -p logicaffeine-tests --test roofline_invariant_lock
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

use std::collections::BTreeMap;
use std::path::Path;

use logicaffeine_compile::codegen_sva::fabric_target::Precision;
use logicaffeine_compile::codegen_sva::plasticine::PlasticineTarget;
use logicaffeine_compile::spatial::ir::{ComputeOp, SpatialProgram, StageKind};
use logicaffeine_compile::spatial::pipeline::compile_to_spatial;
use logicaffeine_compile::spatial::programs::gemm_program;
use logicaffeine_compile::spatial::rdu_chip::RduChip;
use logicaffeine_compile::spatial::roofs::{Engine, MemLevel, StreamMix};
use logicaffeine_compile::spatial::target::{precision_bytes, SpatialTarget};
use logicaffeine_compile::spatial::work::work_of;

fn ledger(path: &str) -> serde_json::Value {
    let full = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(path);
    serde_json::from_str(&std::fs::read_to_string(&full).unwrap_or_else(|e| panic!("{path}: {e}")))
        .unwrap_or_else(|e| panic!("{path}: {e}"))
}

/// The elements an `m` x `n` x `k` GEMM must move at least once: both operands read, the result written.
fn compulsory_elements(m: u64, n: u64, k: u64) -> u64 {
    m * k + k * n + m * n
}

fn operand_of(p: &SpatialProgram) -> Precision {
    p.stages
        .iter()
        .find_map(|s| match &s.kind {
            StageKind::Compute(ComputeOp::MatmulTiles { operand, .. }) => Some(*operand),
            _ => None,
        })
        .expect("a GEMM program has a tile matmul")
}

#[test]
fn every_metered_gemm_moves_at_least_its_compulsory_bytes() {
    let d = ledger("benchmarks/silicon/tenstorrent_gemm.json");
    let mut checked = 0usize;
    let mut tightest = f64::INFINITY;
    for r in d["rows"].as_array().expect("rows") {
        let Some(moved) = r["work"]["noc_bytes"].as_u64() else {
            continue;
        };
        let key = r["key"].as_str().unwrap_or("?");
        let width = match r["dtype"].as_str() {
            Some("bf16") => 2,
            other => panic!("{key}: element width of dtype {other:?} is not stated here; add it before the row can be held to its floor"),
        };
        let (m, n, k) = (
            r["shape"]["m"].as_u64().unwrap(),
            r["shape"]["n"].as_u64().unwrap(),
            r["shape"]["k"].as_u64().unwrap(),
        );
        let floor = compulsory_elements(m, n, k) * width;
        assert!(moved >= floor, "{key}: the work meter counted {moved} NOC bytes, below the {floor} its operands and result must cross at least once — a wrong byte count");
        checked += 1;
        tightest = tightest.min(moved as f64 / floor as f64);
    }
    assert!(
        checked >= 500,
        "only {checked} metered rows — the ledger is not being read"
    );
    assert!(tightest < 1.001, "the floor is reached on the ledger (the 128³ blocked plan sits two semaphores above it), got {tightest}");
}

const SHAPES: [(usize, usize, usize); 3] = [(64, 64, 64), (128, 96, 160), (256, 256, 128)];

#[test]
fn every_planned_gemm_moves_at_least_its_compulsory_dram_bytes() {
    for t in [
        SpatialTarget::wormhole_ttsim(),
        SpatialTarget::blackhole(),
        SpatialTarget::rtx3060(),
        SpatialTarget::sn40l_rdu(),
        SpatialTarget::sn50_rdu(),
    ] {
        let mut planned = 0usize;
        for (m, n, k) in SHAPES {
            let Ok(p) = compile_to_spatial(&gemm_program(m, n, k), &t) else {
                continue;
            };
            let w = work_of(&p, &t);
            let floor = compulsory_elements(m as u64, n as u64, k as u64)
                * precision_bytes(operand_of(&p)) as u64;
            assert!(
                w.dram_bytes >= floor,
                "{} {m}x{n}x{k}: the plan moves {} DRAM bytes, below the {floor} its operands and result must cross",
                t.name,
                w.dram_bytes
            );
            planned += 1;
        }
        assert!(
            planned > 0,
            "{}: no GEMM planned, so nothing was held to the floor",
            t.name
        );
    }
}

#[test]
fn every_plasticine_report_row_stays_under_its_targets_roofs() {
    let d = ledger("benchmarks/silicon/plasticine_report.json");
    let targets: BTreeMap<&str, PlasticineTarget> = [
        PlasticineTarget::isca17(),
        PlasticineTarget::sara_20x20(),
        PlasticineTarget::tungsten_20x20_ddr4(),
        PlasticineTarget::rdu(RduChip::Sn40l),
        PlasticineTarget::rdu(RduChip::Sn50),
    ]
    .into_iter()
    .map(|t| (t.name, t))
    .collect();
    let mut gemms = 0usize;
    for r in d["rows"].as_array().expect("rows") {
        let name = format!(
            "{} on {}",
            r["name"].as_str().unwrap_or("?"),
            r["target"].as_str().unwrap_or("?")
        );
        let t = targets
            .get(r["target"].as_str().unwrap_or(""))
            .unwrap_or_else(|| panic!("{name}: a target with no roofs"));
        let roofs = t.roofs();
        let dram = roofs
            .mem(MemLevel::Dram, StreamMix::Read)
            .expect("a DRAM roof");
        let bytes_per_cycle = roofs.per_cycle(&dram.rate).value;
        let (bytes, dram_cycles, modeled, compute) = (
            r["dram_bytes"].as_u64().unwrap(),
            r["dram_cycles"].as_u64().unwrap(),
            r["modeled_cycles"].as_u64().unwrap(),
            r["compute_cycles"].as_u64().unwrap(),
        );
        assert_eq!(
            dram_cycles,
            (bytes as f64 / bytes_per_cycle).ceil() as u64,
            "{name}: the DRAM term is the row's bytes over its target's DRAM roof"
        );
        assert!(
            modeled >= dram_cycles && modeled >= compute,
            "{name}: {modeled} modeled cycles beat a bound (DRAM {dram_cycles}, compute {compute})"
        );
        if r["gemm"].is_object() {
            let g = &r["gemm"];
            let (m, n, k) = (
                g["m"].as_u64().unwrap(),
                g["n"].as_u64().unwrap(),
                g["k"].as_u64().unwrap(),
            );
            let peak = roofs
                .compute(Engine::Vector, Precision::Fp32)
                .expect("the fp32 roofline");
            let ops_per_cycle = roofs.per_cycle(&peak.ops).value;
            let flops = 2.0 * (m * n * k) as f64;
            assert!(
                modeled as f64 >= flops / ops_per_cycle,
                "{name}: {flops} operations in {modeled} cycles outruns {ops_per_cycle} per cycle"
            );
            assert!(
                bytes >= compulsory_elements(m, n, k) * 4,
                "{name}: {bytes} DRAM bytes, below its fp32 operands and result"
            );
            let pct = r["pct_of_peak_fp32"]
                .as_f64()
                .expect("a GEMM row states its share of the peak");
            assert!(pct <= 100.0, "{name}: {pct}% of the fp32 peak");
            gemms += 1;
        }
    }
    assert!(
        gemms >= 4,
        "only {gemms} GEMM rows held to the compute roof"
    );
}
