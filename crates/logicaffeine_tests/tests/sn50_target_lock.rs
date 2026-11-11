//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! SN50 TARGET LOCK — the SambaNova SN50 RDU is described from ONE source of chip facts, every public
//! number cited, every gap labeled `modeled`, and both descriptors (the spatial mesh and the scalar
//! fabric envelope) DERIVED from those facts rather than typed beside them.
//!
//! `RduChipFacts` (`spatial/rdu_chip.rs`) carries the SN40L and SN50 as data: each number is a `Fact`
//! with a provenance (`Published` — read from a vendor document named in `cites` — or `Modeled` — a gap
//! the public record leaves, with what it was inherited from). `SpatialTarget::sn50_rdu()` and
//! `FabricTarget::sn50_shaped()` are functions of those facts, and so are the SN40L twins the earlier
//! locks pin — whose numbers do not move by one byte.
//!
//! Public SN50 facts (verified 2026-09-18): TSMC 5 nm, two max-reticle dies, 1600 TFLOPS BF16 / 3200
//! TFLOPS FP8, 432 MB on-chip SRAM, 64 GB HBM2E, DDR5 up to 512 GB, 8 chips per node, 16 per rack,
//! 7×800G intra-node + 2×800G inter-node scale-up (2 TB/s per RDU), 400G RoCEv2 scale-out. The PCU/PMU
//! counts are NOT public ("a sea of PCUs and PMUs") — inherited from the SN40L and labeled modeled.
//! Sources: sambanova.ai/products/rdu-ai-chips · Hot Chips 2026 (sambanova.ai/blog/hot-chips-2026-
//! dataflow-at-scale) · ServeTheHome, SN50 RDU at Hot Chips 2026.
//!
//! ★ MUTATION PROBES (each run once against this tree; the named test went RED, the impl was restored
//! byte-identical — every line below is what the probe ACTUALLY did and what it ACTUALLY reported):
//!   - `rdu_chip.rs`: blank the `cite` of the SN50's `peak_fp8_tflops` (`3200, ""`) → `sn50_facts_pin_every_
//!     public_number_with_its_citation` fails on that path;
//!   - `rdu_chip.rs`: add `Precision::Fp8` to the SN50 `witnessed` list → BOTH `sn50_rdu_is_derived_from_the_
//!     facts…` and `sn50_shaped_carries_the_facts…` fail (`matmul_supports(Fp8)`, `require_gemm(WS, Fp8)` no
//!     longer refused);
//!   - `rdu_chip.rs`: drop `"silicon.mem_tiles"` from the SN50 `modeled` set → `the_modeled_set_is_load_bearing`;
//!   - `rdu_chip.rs`: change the SN40L's PUBLISHED `sram_mib` fact from 520 to 432 (the SN50's number) →
//!     `the_sn40l_published_numbers_are_untouched` — the derivation may not quietly re-point an older chip's
//!     published figure at the newer one;
//!   - `target.rs`: remove `SpatialTarget::sn50_rdu()` from `presets()` → `sn50_is_in_the_preset_matrix`
//!     (`left: 0` — the preset matrix must carry it exactly once).
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test sn50_target_lock`
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

#[path = "common/spatial.rs"]
mod spatial_support;

use std::collections::BTreeSet;

use logicaffeine_compile::codegen_sva::fabric_target::{
    Capability, Dataflow, FabricTarget, MemHierarchy, Precision, SiliconSpec, Unplaceable,
};
use logicaffeine_compile::spatial::ir::Phase;
use logicaffeine_compile::spatial::lower::rdu::RduSpatialBackend;
use logicaffeine_compile::spatial::lower::{SpatialArtifact, SpatialBackend};
use logicaffeine_compile::spatial::pipeline::compile_to_spatial;
use logicaffeine_compile::spatial::rdu_chip::{Provenance, RduChip, RduChipFacts};
use logicaffeine_compile::spatial::target::{HandshakeKind, SpatialTarget};
use serde_json::Value;
use spatial_support::gemm_program;

/// The bibliography keys (`benchmarks/samba_papers.tsv`) of every source the SN50's facts cite.
const SN50_SOURCES: &[&str] = &["DIAYN26", "HC24", "HC26", "ISCA17", "SN40L24", "SN50DS", "SN50WEB", "STH26"];

const SN50_MODELED: &[&str] = &[
    "sites",
    "acc_tiles",
    "tile",
    "local_mem_bytes",
    "silicon.compute_tiles",
    "silicon.mem_tiles",
];
const SN40L_MODELED: &[&str] = &["sites", "acc_tiles", "tile"];

/// Every numeric leaf of a JSON tree with the `Fact` object (`{value, provenance, cite}`) that vouches
/// for it — `None` when no ancestor is a fact, which is exactly an uncited number.
fn numeric_leaves(
    v: &Value,
    path: &mut Vec<String>,
    fact: Option<(&Value, String)>,
    out: &mut Vec<(String, Option<(Provenance, String, String)>)>,
) {
    match v {
        Value::Number(_) => {
            let vouched = fact.map(|(f, at)| {
                let prov = match f["provenance"].as_str() {
                    Some("Published") => Provenance::Published,
                    Some("Modeled") => Provenance::Modeled,
                    other => panic!(
                        "{}: a fact's provenance must be Published or Modeled, got {other:?}",
                        path.join(".")
                    ),
                };
                (prov, f["cite"].as_str().unwrap_or("").to_string(), at)
            });
            out.push((path.join("."), vouched));
        }
        Value::Object(m) => {
            let here_is_fact =
                m.contains_key("value") && m.contains_key("provenance") && m.contains_key("cite");
            for (k, child) in m {
                path.push(k.clone());
                let f = if here_is_fact {
                    Some((v, path[..path.len() - 1].join(".")))
                } else {
                    fact.clone()
                };
                numeric_leaves(child, path, f, out);
                path.pop();
            }
        }
        Value::Array(items) => {
            for (i, child) in items.iter().enumerate() {
                path.push(i.to_string());
                numeric_leaves(child, path, fact.clone(), out);
                path.pop();
            }
        }
        _ => {}
    }
}

#[test]
fn sn50_facts_pin_every_public_number_with_its_citation() {
    let facts = RduChipFacts::sn50();
    assert_eq!(facts.chip, RduChip::Sn50);
    assert_eq!(facts.part, "SN50");
    assert_eq!(
        facts.cites, SN50_SOURCES,
        "the SN50 facts name exactly the sources they cite, by bibliography key"
    );

    let json = serde_json::to_value(&facts).expect("the facts serialize");
    let mut leaves = Vec::new();
    numeric_leaves(&json, &mut Vec::new(), None, &mut leaves);
    assert!(
        leaves.len() >= 20,
        "the reflection saw only {} numbers — it is not walking the facts",
        leaves.len()
    );
    let mut published = 0usize;
    let mut modeled = 0usize;
    for (path, vouched) in &leaves {
        let (prov, cite, at) = vouched.as_ref().unwrap_or_else(|| panic!("`{path}` is a number no `Fact` vouches for — every number on the SN50 facts is Published or Modeled, with its citation"));
        assert!(
            !cite.trim().is_empty(),
            "`{path}` (fact `{at}`) has an empty citation"
        );
        match prov {
            Provenance::Published => {
                published += 1;
                assert!(SN50_SOURCES.iter().any(|s| cite.contains(s)), "`{path}` (fact `{at}`) cites `{cite}`, which is none of the SN50's sources");
            }
            Provenance::Modeled => {
                modeled += 1;
                assert!(
                    cite.contains("not public") || cite.contains("not tabulated") || cite.contains("inherited"),
                    "`{path}` (fact `{at}`) is modeled but its cite `{cite}` does not say what gap it fills or what it inherits"
                );
            }
            Provenance::Derived | Provenance::Measured => panic!("`{path}` (fact `{at}`) is {prov:?}: every number on the SN50 facts is Published or Modeled"),
        }
    }
    assert!(
        published >= 12,
        "only {published} published numbers — the public record has more than that"
    );
    assert!(
        modeled >= 6,
        "only {modeled} modeled numbers — the PCU/PMU counts and the unit model are all modeled"
    );

    // The public numbers themselves, pinned.
    assert_eq!(facts.peak_bf16_tflops.value, 1600);
    assert_eq!(facts.peak_fp8_tflops.map(|f| f.value), Some(3200));
    assert_eq!(facts.sram_mib.value, 432);
    assert_eq!(facts.hbm_gib.value, 64);
    assert_eq!(facts.ddr_gib_max.value, 512);
    assert_eq!(facts.node_nm.value, 5);
    assert_eq!(facts.dies.value, 2);
    assert_eq!(facts.chips_per_node.value, 8);
    let su = facts
        .scale_up
        .expect("the SN50 tabulates its scale-up fabric")
        .value;
    assert_eq!(
        (
            su.chips_per_rack,
            su.intra_node_links,
            su.inter_node_links,
            su.link_gbps
        ),
        (16, 7, 2, 800)
    );
    assert!(
        (su.tb_s_per_chip - 2.0).abs() < 1e-12,
        "2 TB/s of scale-up per RDU"
    );
    assert_eq!(su.scale_out, "400G RoCEv2");
    for f in [&facts.peak_bf16_tflops, &facts.peak_fp8_tflops.unwrap()] {
        assert_eq!(f.provenance, Provenance::Published);
    }
    // The tile counts are NOT public on the SN50: modeled, inherited, and they say so.
    assert_eq!(
        (facts.pcu_count.value, facts.pcu_count.provenance),
        (1040, Provenance::Modeled)
    );
    assert_eq!(
        (facts.pmu_count.value, facts.pmu_count.provenance),
        (1040, Provenance::Modeled)
    );
    assert!(
        facts.pcu_count.cite.contains("SN40L"),
        "the inherited count names where it came from: {}",
        facts.pcu_count.cite
    );
    assert_eq!(facts.witness_pending, &[Precision::Fp8, Precision::Fp32]);
    // The integer an index rides: the SN50 page names only BF16 and FP8, so INT32 is the SN40L's, inherited.
    assert_eq!(
        (facts.index_datatype.value, facts.index_datatype.provenance),
        (Precision::Int32, Provenance::Modeled)
    );
    assert!(
        facts.index_datatype.cite.contains("inherited")
            && facts.index_datatype.cite.contains("SN40L"),
        "{}",
        facts.index_datatype.cite
    );
    println!(
        "SN50: {} published + {modeled} modeled numbers, every one cited",
        published
    );
}

#[test]
fn sn50_rdu_is_derived_from_the_facts_and_declares_exactly_what_it_models() {
    let facts = RduChipFacts::sn50();
    let t = SpatialTarget::sn50_rdu();
    assert_eq!(
        t,
        facts.spatial_target(),
        "the preset IS the facts' derivation, not a copy typed beside it"
    );
    assert_eq!(t.name, "sn50-rdu");
    assert_eq!(
        t.modeled, SN50_MODELED,
        "the modeled set is exactly the fields the public record does not give"
    );
    assert_eq!(
        t.dram,
        MemHierarchy {
            sram_kib: 432 * 1024,
            hbm_gib: 64,
            ddr_gib: 512
        }
    );
    assert_eq!(
        t.sites.sites(),
        1040,
        "the mesh has the (modeled, inherited) PCU count"
    );
    assert_eq!(
        t.site.local_mem_bytes,
        432 * 1024 * 1024 / 1040,
        "the per-PCU local memory is the SRAM spread over the PCUs"
    );
    assert_eq!(
        t.precisions,
        [Precision::Int8, Precision::Bf16]
            .into_iter()
            .collect::<BTreeSet<_>>(),
        "only witnessed precisions"
    );
    assert!(t.matmul_supports(Precision::Int8) && t.matmul_supports(Precision::Bf16));
    assert!(
        !t.matmul_supports(Precision::Fp8),
        "FP8 is chip-supported and witness-pending — never advertised before its witness ships"
    );
    assert!(!t.matmul_supports(Precision::Fp32));
    for h in [
        HandshakeKind::Token,
        HandshakeKind::ValidReady,
        HandshakeKind::Barrier,
    ] {
        assert!(
            t.provides_handshake(h),
            "the RDU provides the {h:?} handshake"
        );
    }
    assert_eq!(
        *t.scalar_target().expect("the embedded scalar datapath"),
        FabricTarget::sn50_shaped()
    );
    assert_eq!(facts.sram_bytes(), 432 * 1024 * 1024);
    assert_eq!(facts.local_mem_bytes_per_pcu(), t.site.local_mem_bytes);
}

#[test]
fn sn50_shaped_carries_the_facts_and_only_witnessed_precisions() {
    let f = FabricTarget::sn50_shaped();
    assert_eq!(f, RduChipFacts::sn50().fabric_target());
    assert_eq!(f.name, "sn50-rdu");
    assert_eq!(
        f.silicon,
        Some(SiliconSpec {
            compute_tiles: 1040,
            mem_tiles: 1040,
            node_nm: 5,
            peak_bf16_pflops: 1.6
        })
    );
    assert_eq!(
        f.mem,
        MemHierarchy {
            sram_kib: 432 * 1024,
            hbm_gib: 64,
            ddr_gib: 512
        }
    );
    f.require_gemm(Dataflow::WeightStationary, Precision::Int8)
        .expect("int8 WS GEMM places");
    f.require_gemm(Dataflow::OutputStationary, Precision::Bf16)
        .expect("bf16 OS GEMM places");
    match f.require_gemm(Dataflow::WeightStationary, Precision::Fp8) {
        Err(Unplaceable::MissingCapability {
            needed: Capability::Precision(Precision::Fp8),
            target: "sn50-rdu",
        }) => {}
        other => panic!("FP8 is witness-pending and must be refused by name, got {other:?}"),
    }
    match f.require_gemm(Dataflow::WeightStationary, Precision::Fp32) {
        Err(Unplaceable::MissingCapability {
            needed: Capability::Precision(Precision::Fp32),
            ..
        }) => {}
        other => panic!("FP32 is witness-pending and must be refused by name, got {other:?}"),
    }
    assert_eq!(
        f.precisions,
        [Precision::Int8, Precision::Bf16]
            .into_iter()
            .collect::<BTreeSet<_>>()
    );
    assert!(f.agu && f.simd.is_some());
    assert!(f.provides(Capability::Fusion(4)));
}

#[test]
fn the_sn40l_published_numbers_are_untouched() {
    let facts = RduChipFacts::sn40l();
    assert_eq!((facts.chip, facts.part), (RduChip::Sn40l, "SN40L"));
    let f = FabricTarget::rdu_shaped();
    assert_eq!(
        f,
        facts.fabric_target(),
        "rdu_shaped() is now derived from the SN40L facts"
    );
    assert_eq!(f.name, "sn40l-rdu");
    assert_eq!(
        f.silicon,
        Some(SiliconSpec {
            compute_tiles: 1040,
            mem_tiles: 1040,
            node_nm: 5,
            peak_bf16_pflops: 10.208
        })
    );
    assert_eq!(
        f.mem,
        MemHierarchy {
            sram_kib: 520 * 1024,
            hbm_gib: 64,
            ddr_gib: 1536
        }
    );
    assert_eq!(
        (f.rows, f.cols, f.width, f.mem_depth, f.fusion_window),
        (64, 64, 64, 1 << 16, 64)
    );
    assert_eq!(
        f.precisions,
        [Precision::Int8, Precision::Bf16]
            .into_iter()
            .collect::<BTreeSet<_>>()
    );
    let t = SpatialTarget::sn40l_rdu();
    assert_eq!(t, facts.spatial_target());
    assert_eq!(t.name, "sn40l-rdu");
    assert_eq!((t.sites.x, t.sites.y), (40, 26));
    assert_eq!(t.site.local_mem_bytes, 520 * 1024 * 1024 / 1040);
    assert_eq!(t.site.acc_tiles, 8);
    assert_eq!(t.modeled, SN40L_MODELED, "the SN40L publishes its tile counts, so only the mesh aspect, the accumulator and the tile are modeled");
    assert!(
        t.provides_handshake(HandshakeKind::Barrier),
        "both RDU presets gain the barrier handshake"
    );
    assert!(
        t.provides_handshake(HandshakeKind::Token)
            && t.provides_handshake(HandshakeKind::ValidReady)
    );
    // The published SN40L numbers, on the facts.
    assert_eq!(
        (
            facts.peak_bf16_tflops.value,
            facts.peak_bf16_tflops.provenance
        ),
        (638, Provenance::Published)
    );
    assert_eq!(
        facts.peak_fp8_tflops, None,
        "the SN40L publishes no FP8 figure"
    );
    assert_eq!(
        (
            facts.sram_mib.value,
            facts.hbm_gib.value,
            facts.ddr_gib_max.value,
            facts.node_nm.value
        ),
        (520, 64, 1536, 5)
    );
    assert_eq!(
        (facts.pcu_count.value, facts.pcu_count.provenance),
        (1040, Provenance::Published)
    );
    assert_eq!(
        (facts.pmu_count.value, facts.pmu_count.provenance),
        (1040, Provenance::Published)
    );
    assert_eq!(facts.chips_per_node.value, 16);
    assert_eq!(facts.scale_up, None, "the SN40L's per-link scale-up fabric is not tabulated in the cited sources — nothing is typed for it");
    assert_eq!(facts.witness_pending, &[Precision::Fp32]);
    assert_eq!(
        (facts.index_datatype.value, facts.index_datatype.provenance),
        (Precision::Int32, Provenance::Published),
        "the SN40L publishes its PCU's INT32"
    );
}

#[test]
fn sn50_is_in_the_preset_matrix() {
    let presets = SpatialTarget::presets();
    let sn50: Vec<&SpatialTarget> = presets.iter().filter(|t| t.name == "sn50-rdu").collect();
    assert_eq!(sn50.len(), 1, "sn50 joins the preset matrix exactly once");
    assert_eq!(*sn50[0], SpatialTarget::sn50_rdu());
    assert!(
        presets.iter().any(|t| t.name == "sn40l-rdu"),
        "and the SN40L stays"
    );
    let p = compile_to_spatial(&gemm_program(256, 256, 128), &SpatialTarget::sn50_rdu())
        .expect("a GEMM compiles for the SN50");
    assert_eq!(p.phase(), Phase::Verified);
    assert!(p.certificate.as_ref().expect("a certificate").recheck(&p));
    match RduSpatialBackend::for_chip(RduChip::Sn50)
        .lower(&p)
        .expect("the SN50 lane lowers")
    {
        SpatialArtifact::SdkGated(a) => {
            assert!(a.certificate.recheck(&p));
            let v: Value = serde_json::from_str(&a.json).unwrap();
            assert_eq!(v["target"]["name"], "sn50-rdu");
            assert_eq!(v["target"]["chip"], "sn50");
        }
        other => panic!("the RDU lane is SDK-gated, got {other:?}"),
    }
}

/// Whether `dotted` names a real field path somewhere in `v`: the segments must be consecutive object
/// keys (the first may be at any depth). The `modeled` array itself is excluded, so a name cannot vouch
/// for itself by being listed.
fn names_a_field(v: &Value, dotted: &str) -> bool {
    fn at(v: &Value, segs: &[&str]) -> bool {
        match segs {
            [] => true,
            [k, rest @ ..] => v.get(k).is_some_and(|c| at(c, rest)),
        }
    }
    fn anywhere(v: &Value, segs: &[&str], skip_key: &str) -> bool {
        match v {
            Value::Object(m) => {
                if at(v, segs) {
                    return true;
                }
                m.iter()
                    .filter(|(k, _)| k.as_str() != skip_key)
                    .any(|(_, c)| anywhere(c, segs, skip_key))
            }
            Value::Array(items) => items.iter().any(|c| anywhere(c, segs, skip_key)),
            _ => false,
        }
    }
    let segs: Vec<&str> = dotted.split('.').collect();
    anywhere(v, &segs, "modeled")
}

#[test]
fn the_modeled_set_is_load_bearing() {
    // Reflection over every preset: each modeled name is a real (possibly dotted) field path of the serialized
    // descriptor — a name that vouches for itself only by appearing in the `modeled` array does not count.
    for t in SpatialTarget::presets() {
        let json = serde_json::to_value(&t).unwrap();
        for f in t.modeled {
            assert!(
                names_a_field(&json, f),
                "{}: modeled name `{f}` is not a field path of the descriptor",
                t.name
            );
        }
    }
    assert!(
        !names_a_field(
            &serde_json::to_value(SpatialTarget::sn50_rdu()).unwrap(),
            "silicon.bogus"
        ),
        "the path check is not vacuous"
    );

    // The set is a FUNCTION of which facts are modeled: a modeled PCU count makes the mesh, the per-site memory
    // and the silicon's compute-tile figure modeled; a modeled PMU count makes the mem-tile figure modeled.
    let sn50 = RduChipFacts::sn50();
    let sn40l = RduChipFacts::sn40l();
    assert!(sn50.pcu_count.is_modeled() && sn50.pmu_count.is_modeled());
    for derived in ["sites", "local_mem_bytes", "silicon.compute_tiles"] {
        assert!(
            sn50.target_modeled().contains(&derived),
            "sn50: a modeled PCU count makes `{derived}` modeled"
        );
        assert!(
            !sn40l.target_modeled().contains(&derived) || derived == "sites",
            "sn40l: a published PCU count does not make `{derived}` modeled"
        );
    }
    assert!(sn50.target_modeled().contains(&"silicon.mem_tiles"));
    assert!(!sn40l.target_modeled().contains(&"silicon.mem_tiles"));
    assert_eq!(sn50.target_modeled(), SN50_MODELED);
    assert_eq!(sn40l.target_modeled(), SN40L_MODELED);
    let unit_model_paths = [
        "unit_model.acc_tiles",
        "unit_model.counter_nest",
        "unit_model.datapath_bits",
        "unit_model.fusion_window",
        "unit_model.mesh_columns",
        "unit_model.pcu_envelope",
        "unit_model.pcu_lanes",
        "unit_model.pcu_stages",
        "unit_model.pmu",
    ];
    let mut sn50_paths = vec!["ddr_bandwidth_gbps", "hbm_bandwidth_gbps", "index_datatype", "pcu_count", "pmu_count", "simd"];
    sn50_paths.extend(unit_model_paths);
    assert_eq!(
        sn50.modeled_fields(),
        sn50_paths,
        "every modeled fact on the SN50, by path"
    );
    assert_eq!(
        sn40l.modeled_fields(),
        unit_model_paths,
        "the SN40L publishes its counts; only the unit model is modeled"
    );

    // And the neutral artifact carries the same set — a vendor engineer reads what is modeled from the envelope.
    let p = compile_to_spatial(&gemm_program(64, 64, 64), &SpatialTarget::sn50_rdu()).unwrap();
    let SpatialArtifact::SdkGated(a) = RduSpatialBackend::for_chip(RduChip::Sn50)
        .lower(&p)
        .unwrap()
    else {
        panic!("SDK-gated")
    };
    let v: Value = serde_json::from_str(&a.json).unwrap();
    let carried: Vec<&str> = v["target"]["modeled"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap())
        .collect();
    assert_eq!(carried, SN50_MODELED);
    let facts_modeled: Vec<&str> = v["target"]["facts_modeled"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap())
        .collect();
    assert_eq!(facts_modeled, sn50.modeled_fields());
}
