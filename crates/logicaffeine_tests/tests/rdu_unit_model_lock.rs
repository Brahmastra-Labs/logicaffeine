//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! RDU UNIT-MODEL LOCK — the PCU/PMU unit model is data on the chip facts, labeled modeled and cited;
//! BOTH RDU descriptors read it (the 1/1/1 placeholders are retired); and the II report is a FUNCTION
//! of it in both directions, so an SDK number drops in as data and every derived figure follows.
//!
//! The unit model (`RduUnitModel`) is inherited from the open RDU — Plasticine (Prabhakar et al., ISCA
//! 2017): a six-stage PCU pipeline, sixteen SIMD lanes, a 16-bank × 4096-word PMU with a four-stage
//! address datapath, double-buffered as the SN50 public description says; the credit-loop latencies are
//! `None` (undeclared, NOT zero) until the SDK names them. `rdu_ii_report` builds on
//! `codegen_sva::metapipeline::{Stage, Metapipeline, UnitModel}` (stage II, N-buffer depths),
//! `codegen_sva::pmu::memory_ii` (bank conflicts) and `codegen_sva::choreography::credit_depth` (the
//! bandwidth-delay solve). MODELED throughout: the model's cycles, never a simulation of the chip.
//!
//! ★ MUTATION PROBES (each run once against this tree; the named test went RED, the impl was restored
//! byte-identical — every line below is what the probe ACTUALLY did and what it ACTUALLY reported):
//!   - `rdu_chip.rs`: hard-code `metapipeline_depth: 6` in `fabric_target()` instead of reading the unit
//!     model → `both_rdu_descriptors_read_the_unit_model` fails on the `with_pcu_stages(7)` derivation
//!     (`Sn40l: the descriptor follows the unit model, left: 6`);
//!   - `rdu_chip.rs`: blank the modeled `pcu_stages` cite → `the_unit_model_is_modeled_and_cited`
//!     (`Sn40l: the stage count is inherited from the open RDU`);
//!   - `rdu_chip.rs`: drop the reduction-tree term (`let tree = 0`) from a reduce stage's latency →
//!     `the_ii_report_moves_when_the_descriptor_moves` fails on `rmsnorm: a reduction pays its tree on top
//!     of the pipeline`. (Predicted to be masked by the +1 side and NOT masked: the reduce term is pinned in
//!     its own right, so the lock is stronger here than it was designed to be.)
//!   - `rdu_chip.rs`: replace `memory_ii(&accesses, banking, &pmu)` with a constant 1 →
//!     `the_ii_report_moves…` on `rmsnorm: on one bank every lane collides — memory II 1` (the banks 16→1
//!     side must rise ONLY through the PMU model);
//!   - `rdu_chip.rs`: ignore `credit` in the credit-depth solve (`backward_latency = hops`) →
//!     `the_ii_report_moves…` (a declared credit latency must differ from `None`);
//!   - `rdu_chip.rs`: return a static list from `modeled_fields()` instead of reflecting provenance →
//!     `an_sdk_number_drops_in_without_code` (the static list and the live one diverge by name).
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test rdu_unit_model_lock`
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

#[path = "common/spatial.rs"]
mod spatial_support;

use logicaffeine_compile::codegen_sva::fabric_target::{Capability, FabricTarget};
use logicaffeine_compile::codegen_sva::pmu::PmuSpec;
use logicaffeine_compile::spatial::ir::{ComputeOp, IterKind, SiteRole, StageKind};
use logicaffeine_compile::spatial::lower::rdu::RduSpatialBackend;
use logicaffeine_compile::spatial::lower::{SpatialArtifact, SpatialBackend};
use logicaffeine_compile::spatial::pipeline::compile_to_spatial;
use logicaffeine_compile::spatial::rdu_chip::{
    rdu_ii_report, Fact, Provenance, RduChip, RduChipFacts, RduIiReport,
};
use logicaffeine_compile::spatial::target::CreditLatency;
use serde_json::Value;
use spatial_support::gemm_program;

/// A row reduction (rmsnorm) — the workload whose stage folds an axis, so the reduction tree and the
/// PMU banking both have something to say.
const RMSNORM: &str = r#"## Main
Let X be a 2 by 3 tensor of Real.
Set item 1 of X to 1.0.
Set item 2 of X to 2.0.
Set item 3 of X to 3.0.
Set item 4 of X to 4.0.
Set item 5 of X to 5.0.
Set item 6 of X to 6.0.
Let W be a 1 by 3 tensor of Real.
Set item 1 of W to 0.5.
Set item 2 of W to 1.0.
Set item 3 of W to 2.0.
Let Y be rmsnorm of X with weight W.
Show round(item 1 of Y * 1000000).
"#;

fn every_fact_in(v: &Value, path: &mut Vec<String>, out: &mut Vec<(String, Provenance, String)>) {
    if let Value::Object(m) = v {
        if m.contains_key("value") && m.contains_key("provenance") && m.contains_key("cite") {
            let prov = match m["provenance"].as_str() {
                Some("Published") => Provenance::Published,
                Some("Modeled") => Provenance::Modeled,
                other => panic!("{}: provenance {other:?}", path.join(".")),
            };
            out.push((
                path.join("."),
                prov,
                m["cite"].as_str().unwrap_or("").to_string(),
            ));
            return;
        }
        for (k, c) in m {
            path.push(k.clone());
            every_fact_in(c, path, out);
            path.pop();
        }
    }
}

#[test]
fn the_unit_model_is_modeled_and_cited() {
    for chip in RduChip::ALL {
        let facts = chip.facts();
        let um = &facts.unit_model;
        assert_eq!(
            (um.pcu_stages.value, um.pcu_stages.provenance),
            (6, Provenance::Modeled),
            "{chip:?}: six PCU stages, modeled"
        );
        assert!(
            um.pcu_stages.cite.contains("ISCA"),
            "{chip:?}: the stage count is inherited from the open RDU: {}",
            um.pcu_stages.cite
        );
        assert_eq!(
            (um.pcu_lanes.value, um.pcu_lanes.provenance),
            (16, Provenance::Modeled)
        );
        assert_eq!(
            um.pmu.value,
            PmuSpec {
                banks: 16,
                words_per_bank: 4096,
                n_buffers: 2,
                stages: 4
            },
            "{chip:?}: the PMU shape"
        );
        assert_eq!(um.pmu.provenance, Provenance::Modeled);
        assert!(
            um.pmu.cite.contains("ISCA") && um.pmu.cite.contains("double-buffer"),
            "{chip:?}: {}",
            um.pmu.cite
        );
        assert_eq!(
            (um.counter_nest.value, um.counter_nest.provenance),
            (1, Provenance::Modeled)
        );
        assert_eq!(
            um.credit, None,
            "{chip:?}: the credit-loop latencies are undeclared (None), never invented"
        );
        let json = serde_json::to_value(um).unwrap();
        let mut facts_seen = Vec::new();
        every_fact_in(&json, &mut Vec::new(), &mut facts_seen);
        assert!(
            facts_seen.len() >= 6,
            "{chip:?}: the reflection saw only {} facts in the unit model",
            facts_seen.len()
        );
        for (path, prov, cite) in &facts_seen {
            assert_eq!(
                *prov,
                Provenance::Modeled,
                "{chip:?}: unit model `{path}` is modeled until the SDK says otherwise"
            );
            assert!(
                !cite.trim().is_empty(),
                "{chip:?}: unit model `{path}` has no citation"
            );
        }
        // The metapipeline unit the report drives is the calibrated Plasticine PCU at this depth.
        let unit = um.metapipeline_unit();
        assert_eq!(
            (
                unit.pipeline_depth,
                unit.register_turnaround,
                unit.tile_latency
            ),
            (6, 1, 1)
        );
    }
}

#[test]
fn both_rdu_descriptors_read_the_unit_model() {
    for (chip, f) in [
        (RduChip::Sn40l, FabricTarget::rdu_shaped()),
        (RduChip::Sn50, FabricTarget::sn50_shaped()),
    ] {
        let facts = chip.facts();
        assert_eq!(
            f.metapipeline_depth, facts.unit_model.pcu_stages.value,
            "{chip:?}: metapipeline depth is the unit model's"
        );
        assert_eq!(
            f.banks, facts.unit_model.pmu.value.banks,
            "{chip:?}: banks are the unit model's"
        );
        assert_eq!(f.counter_nest, facts.unit_model.counter_nest.value);
        assert_eq!(
            f.mem_depth,
            facts.unit_model.pmu.value.capacity_words(),
            "{chip:?}: the datapath's heap is one PMU's words"
        );
        assert_eq!(
            (f.metapipeline_depth, f.banks, f.counter_nest),
            (6, 16, 1),
            "{chip:?}: the 1/1/1 placeholders are retired"
        );
        assert!(
            f.provides(Capability::Metapipeline(6)) && !f.provides(Capability::Metapipeline(7)),
            "{chip:?}"
        );
        assert!(
            f.provides(Capability::Banks(16)) && !f.provides(Capability::Banks(17)),
            "{chip:?}"
        );
        assert!(
            f.provides(Capability::CounterNest(1)) && !f.provides(Capability::CounterNest(2)),
            "{chip:?}"
        );
        // The descriptor is a FUNCTION of the unit model, not a copy of today's numbers.
        let deeper = facts
            .clone()
            .with_pcu_stages(Fact::modeled(7, "probe: one more PCU stage"))
            .fabric_target();
        assert_eq!(
            deeper.metapipeline_depth, 7,
            "{chip:?}: the descriptor follows the unit model"
        );
        let narrower = facts
            .clone()
            .with_pmu_banks(Fact::modeled(8, "probe: half the banks"))
            .fabric_target();
        assert_eq!(narrower.banks, 8, "{chip:?}");
        assert_eq!(
            narrower.mem_depth,
            8 * 4096,
            "{chip:?}: the heap follows the banks"
        );
        // Everything published stays exactly where it was.
        assert_eq!(deeper.silicon, f.silicon);
        assert_eq!(deeper.mem, f.mem);
        assert_eq!(deeper.precisions, f.precisions);
        // And the spatial mesh embeds the same descriptor.
        assert_eq!(
            *facts.spatial_target().scalar_target().unwrap(),
            f,
            "{chip:?}: one descriptor, embedded"
        );
    }
}

fn reduce_stage_ids(p: &logicaffeine_compile::spatial::ir::SpatialProgram) -> Vec<usize> {
    p.stages
        .iter()
        .filter(|s| matches!(&s.kind, StageKind::Compute(ComputeOp::Generic { iters, .. }) if iters.iter().any(|i| matches!(i.kind, IterKind::Reduce { .. }))))
        .map(|s| s.id.idx())
        .collect()
}

fn rows_by_stage(
    r: &RduIiReport,
) -> std::collections::BTreeMap<usize, &logicaffeine_compile::spatial::rdu_chip::RduIiRow> {
    r.stages.iter().map(|row| (row.stage, row)).collect()
}

#[test]
fn the_ii_report_moves_when_the_descriptor_moves() {
    let facts = RduChipFacts::sn50();
    let target = facts.spatial_target();
    let gemm = compile_to_spatial(&gemm_program(256, 256, 128), &target).unwrap();
    let norm = compile_to_spatial(RMSNORM, &target).unwrap();
    let reduces = reduce_stage_ids(&norm);
    assert!(
        !reduces.is_empty(),
        "the rmsnorm program folds an axis somewhere"
    );

    for (label, p) in [("gemm", &gemm), ("rmsnorm", &norm)] {
        let base = rdu_ii_report(p, &facts);
        assert_eq!(base.chip, RduChip::Sn50);
        assert_eq!(base.provenance, "modeled");
        assert!(
            !base.stages.is_empty(),
            "{label}: one row per compute stage"
        );
        assert_eq!(
            base.stages.len(),
            p.stages
                .iter()
                .filter(|s| s.role == SiteRole::Compute)
                .count(),
            "{label}"
        );
        assert!(base.pipeline_ii >= 1);
        assert!(
            !base.credit_depths.is_empty(),
            "{label}: every IR edge has a credit-loop depth"
        );
        assert_eq!(
            base.credit_depths.len(),
            p.edges.len(),
            "{label}: one credit solve per edge"
        );
        for row in &base.stages {
            assert!(
                row.timing.latency >= facts.unit_model.pcu_stages.value,
                "{label}: a result leaves the PCU no earlier than its pipeline depth"
            );
            assert_eq!(
                row.timing.ii(),
                row.timing
                    .resource_ii
                    .max(row.timing.recurrence_ii)
                    .max(row.timing.memory_ii)
                    .max(1)
            );
        }
        // ── side one: +1 PCU stage → every latency +1, every carried recurrence +1, nothing else moves ──
        let plus = rdu_ii_report(p, &facts.clone().with_pcu_stages(Fact::modeled(7, "probe")));
        let b = rows_by_stage(&base);
        let d = rows_by_stage(&plus);
        assert_eq!(b.len(), d.len());
        for (id, row) in &b {
            let deeper = d[id];
            assert_eq!(
                deeper.timing.latency,
                row.timing.latency + 1,
                "{label}: stage {id} latency follows the PCU depth"
            );
            if row.carried {
                assert_eq!(
                    deeper.timing.recurrence_ii,
                    row.timing.recurrence_ii + 1,
                    "{label}: stage {id} carried recurrence follows the PCU depth"
                );
            } else {
                assert_eq!(
                    (row.timing.recurrence_ii, deeper.timing.recurrence_ii),
                    (0, 0),
                    "{label}: stage {id} carries nothing"
                );
            }
            assert_eq!(
                deeper.timing.memory_ii, row.timing.memory_ii,
                "{label}: the PCU depth does not touch the PMU"
            );
        }
        // ── side two: banks 16 → 1 → a reduce's memory II rises through the PMU model ──
        let one_bank = rdu_ii_report(p, &facts.clone().with_pmu_banks(Fact::modeled(1, "probe")));
        let o = rows_by_stage(&one_bank);
        if label == "rmsnorm" {
            for id in &reduces {
                assert_eq!(
                    b[id].timing.memory_ii, 1,
                    "{label}: a stride-1 vector read is conflict-free on 16 banks"
                );
                assert!(
                    o[id].timing.memory_ii > 1,
                    "{label}: on one bank every lane collides — memory II {}",
                    o[id].timing.memory_ii
                );
                assert_eq!(
                    o[id].timing.memory_ii, facts.unit_model.pcu_lanes.value,
                    "{label}: sixteen lanes on one bank cost sixteen cycles"
                );
                assert!(b[id].reduce, "{label}: stage {id} is a reduction");
                assert!(
                    b[id].timing.latency > facts.unit_model.pcu_stages.value,
                    "{label}: a reduction pays its tree on top of the pipeline"
                );
            }
            assert!(
                one_bank.pipeline_ii > base.pipeline_ii,
                "{label}: the pipeline II follows the slowest stage"
            );
        }
        for (id, row) in &o {
            assert_eq!(
                row.timing.latency, b[id].timing.latency,
                "{label}: the banks do not touch the PCU latency"
            );
        }
        // ── side three: declared credit latencies → the credit solve differs from the undeclared one ──
        let credit = CreditLatency {
            fetch_tile: 40,
            credit_return: 8,
            service_page: 4,
        };
        let declared = rdu_ii_report(
            p,
            &facts
                .clone()
                .with_credit(Fact::published(credit, "probe: an SDK reading")),
        );
        assert_eq!(declared.credit_depths.len(), base.credit_depths.len());
        assert!(declared
            .credit_depths
            .iter()
            .zip(&base.credit_depths)
            .all(|(x, y)| x.edge == y.edge));
        assert!(
            declared
                .credit_depths
                .iter()
                .zip(&base.credit_depths)
                .any(|(x, y)| x.credit_depth != y.credit_depth),
            "{label}: declared latencies change the depth"
        );
        assert!(
            declared
                .credit_depths
                .iter()
                .zip(&base.credit_depths)
                .all(|(x, y)| x.credit_depth >= y.credit_depth),
            "{label}: latency never makes a loop need fewer credits"
        );
        assert!(
            declared
                .credit_depths
                .iter()
                .zip(&base.credit_depths)
                .all(|(x, y)| x.backward_latency
                    >= y.backward_latency + credit.credit_return as usize)
        );
        // The report serializes with its provenance, for the fit report and the envelope.
        let json = serde_json::to_value(&base).unwrap();
        assert_eq!(json["provenance"], "modeled");
        assert!(json["stages"].as_array().unwrap().len() == base.stages.len());
        assert!(json["unit_model"]["pcu_stages"]["value"] == 6);
    }
}

#[test]
fn an_sdk_number_drops_in_without_code() {
    let sdk = RduChipFacts::sn50()
        .with_pcu_stages(Fact::published(8, "SN50 SDK: PCU pipeline depth"))
        .with_pmu_banks(Fact::published(32, "SN50 SDK: PMU banks"))
        .with_credit(Fact::published(
            CreditLatency {
                fetch_tile: 64,
                credit_return: 12,
                service_page: 6,
            },
            "SN50 SDK: credit loop",
        ))
        .with_pcu_count(Fact::published(2080, "SN50 SDK: PCU count"))
        .with_pmu_count(Fact::published(2080, "SN50 SDK: PMU count"));
    // The descriptors follow.
    let f = sdk.fabric_target();
    assert_eq!(
        (f.metapipeline_depth, f.banks, f.mem_depth),
        (8, 32, 32 * 4096)
    );
    assert_eq!(f.silicon.unwrap().compute_tiles, 2080);
    assert_eq!(f.silicon.unwrap().mem_tiles, 2080);
    let t = sdk.spatial_target();
    assert_eq!(t.sites.sites(), 2080);
    assert_eq!(t.site.local_mem_bytes, 432 * 1024 * 1024 / 2080);
    assert_eq!(
        t.site.credit,
        Some(CreditLatency {
            fetch_tile: 64,
            credit_return: 12,
            service_page: 6
        }),
        "the declared credit latencies reach the mesh descriptor"
    );
    // The modeled ledger shrinks to exactly what is still not public — by reflection, not by hand.
    assert_eq!(
        sdk.modeled_fields(),
        vec![
            "ddr_bandwidth_gbps",
            "hbm_bandwidth_gbps",
            "index_datatype",
            "simd",
            "unit_model.acc_tiles",
            "unit_model.counter_nest",
            "unit_model.datapath_bits",
            "unit_model.fusion_window",
            "unit_model.mesh_columns",
            "unit_model.pcu_envelope",
            "unit_model.pcu_lanes"
        ]
    );
    assert_eq!(
        t.modeled,
        &["sites", "acc_tiles", "tile"],
        "published counts leave only the mesh aspect, the accumulator and the tile modeled"
    );
    // The II report follows the SDK numbers.
    let p = compile_to_spatial(&gemm_program(64, 64, 64), &t).unwrap();
    let r = rdu_ii_report(&p, &sdk);
    assert!(r.stages.iter().all(|row| row.timing.latency >= 8));
    assert_eq!(r.unit_model.pcu_stages.provenance, Provenance::Published);
    // And the neutral envelope says which numbers are now published.
    let SpatialArtifact::SdkGated(a) = RduSpatialBackend::with_facts(sdk.clone())
        .lower(&p)
        .unwrap()
    else {
        panic!("SDK-gated")
    };
    let v: Value = serde_json::from_str(&a.json).unwrap();
    assert_eq!(
        v["target"]["unit_model"]["pcu_stages"]["provenance"],
        "Published"
    );
    assert_eq!(v["target"]["unit_model"]["pcu_stages"]["value"], 8);
    assert_eq!(
        v["target"]["unit_model"]["credit"]["value"]["fetch_tile"],
        64
    );
    assert_eq!(
        v["target"]["sites"]["x"].as_u64().unwrap() * v["target"]["sites"]["y"].as_u64().unwrap(),
        2080
    );
    let facts_modeled: Vec<&str> = v["target"]["facts_modeled"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap())
        .collect();
    assert_eq!(facts_modeled, sdk.modeled_fields());
}
