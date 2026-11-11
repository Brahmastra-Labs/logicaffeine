//! samba_exam_unit_use_lock — the row of the Samba book's exam E8 (section 10.8 of `benchmarks/PREPARE_FOR_SAMBA.md`,
//! the compiler's own quality) on unit use: every section of every program a bundle ships reports the PCUs, PMUs and
//! SRAM it holds.
//!
//! The row emits the SN50 bundle `largo emit rdu` writes for a tiny checkpoint of each shared architecture, and the one it
//! writes for a product wide enough to spread over several PCUs, and reads each program's `report.json` beside its
//! `neutral.json`. On the SN50's mesh a site is one PCU beside the PMU whose scratchpad is its local memory, so each
//! section's use is counted here again off the program on disk — its placement and its memory map, within the section's
//! own stages and buffers — and the report must say exactly that: the PCUs its compute stages run on, the PMUs whose
//! scratchpad holds one of its buffers, the SRAM those buffers take (an accumulator is the PCU's, an aliased buffer takes
//! none of its own) and the most of it one PMU holds, and the SRAM-resident tensors it moves. Every figure stays within
//! the chip the report names.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --cargo-profile test-opt --test samba_exam_unit_use_lock`

#![cfg(feature = "codegen")]

#[path = "cli_common/mod.rs"]
mod cli;
#[path = "common/tiny_models.rs"]
mod tiny_models;
#[path = "common/samba_handoff.rs"]
#[allow(dead_code)]
mod samba_handoff;

use logicaffeine_compile::spatial::ir::{MemSpace, Site, SiteRole, SpatialProgram, StageKind, TensorSpace};
use logicaffeine_compile::spatial::lower::rdu::parse_neutral;
use logicaffeine_compile::spatial::rdu_chip::RduChip;
use logicaffeine_compile::spatial::target::precision_bytes;
use samba_handoff::{Model, CTX};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use tiny_models::Tiny;

/// The architectures every row covers: attention alone, a hybrid with recurrent state, windowed attention, and an
/// expert mixture with sinks.
const SHARED: [Tiny; 4] = [Tiny::Qwen3, Tiny::Qwen35, Tiny::Gemma3, Tiny::GptOss];

/// Section `k`'s use of the chip, counted off the program: its compute stages' sites, its scratchpad buffers' sites and
/// bytes, and the SRAM-resident tensors its stages load, store or reduce.
fn counted(p: &SpatialProgram, k: usize) -> Value {
    let (stages, buffers) = match p.sections.get(k) {
        Some(s) => (s.stages.0..s.stages.1, s.buffers.0..s.buffers.1),
        None => (0..p.stages.len(), 0..p.buffers.len()),
    };
    let placement = p.placement.as_ref().expect("a shipped program is placed");
    let memory = p.memory.as_ref().expect("a shipped program is sized");
    let pcus: BTreeSet<Site> = stages.clone().filter(|&i| p.stages[i].role == SiteRole::Compute).map(|i| placement.site_of[i]).collect();
    let mut pmus: BTreeMap<Site, u64> = BTreeMap::new();
    for b in buffers {
        if p.buffers[b].space != MemSpace::Acc && p.buffers[b].alias_of.is_none() {
            *pmus.entry(memory.buffer_at[b].site).or_default() += u64::from(memory.buffer_at[b].bytes);
        }
    }
    let resident: BTreeSet<_> = stages
        .filter_map(|i| match &p.stages[i].kind {
            StageKind::Load { tensor, .. } | StageKind::Store { tensor, .. } | StageKind::AllReduce { tensor, .. } => Some(*tensor),
            _ => None,
        })
        .filter(|t| p.tensor(*t).space == TensorSpace::Sram)
        .collect();
    let resident_bytes: u64 = resident.iter().map(|t| p.tensor(*t)).map(|t| (t.elems.rows * t.elems.cols * precision_bytes(t.dtype)) as u64).sum();
    json!({
        "pcus": pcus.len(),
        "pmus": pmus.len(),
        "sram_bytes": pmus.values().sum::<u64>(),
        "sram_peak_pmu_bytes": pmus.values().copied().max().unwrap_or(0),
        "sram_resident_bytes": resident_bytes,
    })
}

/// `report`'s rows held to program `p`: one a section, in order, each reporting the units the section holds as they are
/// counted off the program, within the chip's. Returns the rows' unit use.
fn holds_every_section(what: &str, p: &SpatialProgram, report: &Value) -> Vec<Value> {
    let facts = RduChip::Sn50.facts();
    let capacity = &report["capacity"];
    assert_eq!(
        *capacity,
        json!({ "pcus": facts.pcu_count.value, "pmus": facts.pmu_count.value, "sram_bytes": facts.sram_bytes(), "pmu_bytes": facts.local_mem_bytes_per_pcu() }),
        "{what}: the units a section's use is out of are the SN50's"
    );
    let rows = report["sections"].as_array().unwrap_or_else(|| panic!("{what}: the report has its sections"));
    let n = p.sections.len().max(1);
    assert_eq!(rows.len(), n, "{what}: a row for each of the program's {n} sections");
    let mut used = Vec::new();
    for (k, row) in rows.iter().enumerate() {
        assert_eq!(row["section"], k, "{what}: the rows in the program's order");
        let name = p.sections.get(k).map_or(p.name.as_str(), |s| s.name.as_str());
        assert_eq!(row["name"], name, "{what}: row {k} is section `{name}`");
        let units = &row["units"];
        assert_eq!(*units, counted(p, k), "{what}: section {k} `{name}` reports the units it holds");
        let at = |key: &str| units[key].as_u64().unwrap_or_else(|| panic!("{what}: section {k} reports `{key}`"));
        let computes = p.sections.get(k).map_or(0..p.stages.len(), |s| s.stages.0..s.stages.1).any(|i| p.stages[i].role == SiteRole::Compute);
        assert_eq!(at("pcus") > 0, computes, "{what}: section {k} `{name}` holds a PCU exactly when it computes");
        assert!(at("pcus") <= capacity["pcus"].as_u64().unwrap() && at("pmus") <= capacity["pmus"].as_u64().unwrap(), "{what}: section {k} `{name}` holds no more units than the chip has: {units}");
        assert!(at("sram_peak_pmu_bytes") <= capacity["pmu_bytes"].as_u64().unwrap(), "{what}: section {k} `{name}` holds no more in a PMU than its scratchpad: {units}");
        assert!(at("sram_peak_pmu_bytes") <= at("sram_bytes") && (at("sram_bytes") == 0) == (at("pmus") == 0), "{what}: section {k} `{name}`: {units}");
        used.push(units.clone());
    }
    used
}

/// A product of a 128 × 512 matrix by a 512 × 512 one, and the SiLU of it: sections the SN50 spreads over several PCUs.
const WIDE: &str = "## Main
Let A be a 128 by 512 tensor of Real.
Let B be a 512 by 512 tensor of Real.
Let C be A matmul B.
Let D be silu of C.
Show item 1 of D.
";

/// E8.5 — PCU, PMU and SRAM use reported for every section of every program of each shared architecture's SN50 bundle,
/// and of a program whose sections spread over several PCUs.
#[test]
fn every_section_reports_its_unit_use() {
    let dir = tempfile::tempdir().unwrap();
    for t in SHARED {
        let m = Model::write(t.spec(CTX), &format!("{t:?}"));
        let out = dir.path().join(format!("{t:?}"));
        let o = cli::largo_in(
            dir.path(),
            &["emit", "rdu", "--chip", "sn50", "--model", m.path.to_str().unwrap(), "--ctx", &CTX.to_string(), "-o", out.to_str().unwrap()],
        );
        assert_eq!(o.status.code(), Some(0), "{t:?}: largo emit rdu failed:\n{}", cli::stderr(&o));
        let manifest: Value = serde_json::from_str(&std::fs::read_to_string(out.join("manifest.json")).unwrap()).unwrap();
        let mut programs: Vec<(String, Value)> = vec![("embed".into(), manifest["embed"]["schedule_digest"].clone())];
        programs.extend(manifest["kinds"].as_array().expect("kinds").iter().map(|k| (k["dir"].as_str().unwrap().to_string(), k["schedule_digest"].clone())));
        programs.push(("head".into(), manifest["head"]["schedule_digest"].clone()));
        let mut sections = 0;
        for (sub, digest) in programs {
            let what = format!("{t:?} {sub}");
            let p = parse_neutral(&std::fs::read_to_string(out.join(&sub).join("neutral.json")).unwrap()).unwrap_or_else(|e| panic!("{what}: {e}"));
            let report: Value = serde_json::from_str(&std::fs::read_to_string(out.join(&sub).join("report.json")).unwrap_or_else(|e| panic!("{what}: report.json: {e}")))
                .unwrap_or_else(|e| panic!("{what}: report.json is JSON: {e}"));
            assert_eq!(report["schema"], "logos.rdu.section_report", "{what}: the report says what it is");
            assert_eq!(report["chip"], "sn50", "{what}: the report names its chip");
            assert_eq!(report["schedule_digest"], digest, "{what}: the report is on the program the manifest records");
            sections += holds_every_section(&what, &p, &report).len();
        }
        assert!(sections > 4, "{t:?}: the bundle's programs have their sections: {sections}");
    }

    let file = dir.path().join("wide.lg");
    std::fs::write(&file, WIDE).unwrap();
    let bundle = dir.path().join("wide");
    let o = cli::largo_in(dir.path(), &["emit", "rdu", "--chip", "sn50", file.to_str().unwrap(), "-o", bundle.to_str().unwrap()]);
    assert_eq!(o.status.code(), Some(0), "the wide program: largo emit rdu failed:\n{}", cli::stderr(&o));
    let p = parse_neutral(&std::fs::read_to_string(bundle.join("neutral.json")).unwrap()).expect("neutral.json parses");
    let report: Value = serde_json::from_str(&std::fs::read_to_string(bundle.join("report.json")).unwrap()).unwrap();
    let used = holds_every_section("the wide program", &p, &report);
    assert!(
        used.iter().any(|u| u["pcus"].as_u64() > Some(1) && u["pmus"].as_u64() > Some(1) && u["sram_peak_pmu_bytes"].as_u64() < u["sram_bytes"].as_u64()),
        "the wide program: a section spreads over several PCUs and PMUs: {used:?}"
    );
}
