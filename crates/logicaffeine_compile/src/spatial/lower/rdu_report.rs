//! The report an RDU program carries section by section: what each section of the composed program computes, every
//! fusion the shared optimizer fired or declined over those statements, and the PCUs, PMUs and SRAM the section holds.
//! Every figure is read off the program and its [`Lineage`] — the placement, the memory map, the optimizer's own report
//! — and none is entered by hand; like the unit model it is MODELED, never a measurement of the chip.
//!
//! On the RDU's mesh ([`RduChipFacts::spatial_target`]) a site is one PCU beside the PMU whose scratchpad is its local
//! memory. A section's PCUs are the sites its compute stages run on; its PMUs, the sites whose scratchpad holds one of
//! its buffers; its SRAM, the bytes those buffers take there — an accumulator is the PCU's own and a buffer aliased onto
//! another takes none of its own — and, apart, the bytes of every tensor the deployment keeps resident in SRAM that the
//! section loads or stores. A program never composed is one section.
//!
//! A section's fusions are the decisions whose lifted ops it holds: each fired, or declined — refused by the chip with
//! the chip's reason, or held by the compile. A fusion that fired and whose ops the lowering still ran in sections of
//! their own — a region whose every member's result is one the caller reads, each stored where it is made — is `fired,
//! run apart`, with the sections it runs in. A decision over ops in several sections is listed in each, and the
//! program's own list names the sections every decision reaches. A program lowered without the tensor graph (the
//! whole-program lane) had no fusion decided over it: its `fusion` is null and its sections list none.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;
use serde_json::{json, Map, Value};

use crate::spatial::ir::{MemSpace, Site, SiteRole, SpatialProgram, StageKind, TensorId, TensorSpace};
use crate::spatial::lineage::{outcome_word, Lineage};
use crate::spatial::rdu_chip::RduChipFacts;
use crate::spatial::target::precision_bytes;
use crate::tensor_graph::fuse::Outcome;

/// The schema of a program's `report.json` in a model bundle.
pub const SECTION_REPORT_SCHEMA: &str = "logos.rdu.section_report";

/// What one section holds of the chip.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct UnitUse {
    /// The sites its compute stages run on.
    pub pcus: usize,
    /// The sites whose scratchpad holds one of its buffers.
    pub pmus: usize,
    /// The bytes its buffers take in those scratchpads.
    pub sram_bytes: u64,
    /// The most bytes of them one PMU holds.
    pub sram_peak_pmu_bytes: u64,
    /// The bytes of the SRAM-resident tensors it loads or stores.
    pub sram_resident_bytes: u64,
}

/// Each section's name and its half-open stage and buffer ranges; a program never composed is one section of all of it.
fn spans(p: &SpatialProgram) -> Vec<(String, (usize, usize), (usize, usize))> {
    if p.sections.is_empty() {
        return vec![(p.name.clone(), (0, p.stages.len()), (0, p.buffers.len()))];
    }
    p.sections.iter().map(|s| (s.name.clone(), s.stages, s.buffers)).collect()
}

/// The unit use of every section of `p`, in order.
pub fn unit_use(p: &SpatialProgram) -> Vec<UnitUse> {
    let site_of = |i: usize| p.placement.as_ref().map(|pl| pl.site_of[i]).or(p.stages[i].site);
    let held_at = |b: usize| -> Option<(Site, u64)> {
        let buf = &p.buffers[b];
        if buf.space == MemSpace::Acc || buf.alias_of.is_some() {
            return None;
        }
        match p.memory.as_ref() {
            Some(m) => Some((m.buffer_at[b].site, u64::from(m.buffer_at[b].bytes))),
            None => buf.site.map(|s| (s, p.buffer_bytes(buf) as u64)),
        }
    };
    let touched = |s: &StageKind| -> Option<TensorId> {
        match s {
            StageKind::Load { tensor, .. } | StageKind::Store { tensor, .. } | StageKind::AllReduce { tensor, .. } => Some(*tensor),
            _ => None,
        }
    };
    spans(p)
        .into_iter()
        .map(|(_, stages, buffers)| {
            let pcus: BTreeSet<Site> = (stages.0..stages.1).filter(|&i| p.stages[i].role == SiteRole::Compute).filter_map(site_of).collect();
            let mut pmu: BTreeMap<Site, u64> = BTreeMap::new();
            for (site, bytes) in (buffers.0..buffers.1).filter_map(held_at) {
                *pmu.entry(site).or_default() += bytes;
            }
            let resident: BTreeSet<TensorId> = (stages.0..stages.1)
                .filter_map(|i| touched(&p.stages[i].kind))
                .filter(|t| p.tensor(*t).space == TensorSpace::Sram)
                .collect();
            UnitUse {
                pcus: pcus.len(),
                pmus: pmu.len(),
                sram_bytes: pmu.values().sum(),
                sram_peak_pmu_bytes: pmu.values().copied().max().unwrap_or(0),
                sram_resident_bytes: resident
                    .iter()
                    .map(|t| {
                        let t = p.tensor(*t);
                        (t.elems.rows * t.elems.cols * precision_bytes(t.dtype)) as u64
                    })
                    .sum(),
            }
        })
        .collect()
}

/// The chip's units a section's use is out of.
fn capacity(facts: &RduChipFacts) -> Value {
    json!({
        "pcus": facts.pcu_count.value,
        "pmus": facts.pmu_count.value,
        "sram_bytes": facts.sram_bytes(),
        "pmu_bytes": facts.local_mem_bytes_per_pcu(),
    })
}

/// One decision as a section lists it: its place in the program's list, its rule and class, what became of it — and,
/// run apart, the sections it runs in; declined by the chip, why.
fn listed(i: usize, lineage: &Lineage) -> Value {
    let d = &lineage.fusion.decisions[i];
    let outcome = outcome_word(lineage, d);
    let mut v = json!({ "decision": i, "rule": d.rule, "class": d.class.id(), "outcome": outcome });
    if outcome == "fired, run apart" {
        v["sections"] = json!(lineage.sections_of(d));
    }
    if let Outcome::Refused(why) = &d.outcome {
        v["why"] = json!(why);
    }
    v
}

/// The program's fusion decisions in the order the optimizer made them: each with the statements it covers, what became
/// of it, and the sections holding them.
fn fusion(lineage: &Lineage) -> Value {
    let decisions: Vec<Value> = lineage
        .fusion
        .decisions
        .iter()
        .enumerate()
        .map(|(i, d)| {
            let mut v = listed(i, lineage);
            v["covers"] = json!(d
                .covers
                .iter()
                .map(|&o| json!({ "stmt": lineage.ops[o].stmt_index, "binder": lineage.ops[o].binder }))
                .collect::<Vec<_>>());
            v["sections"] = json!(lineage.sections_of(d));
            v
        })
        .collect();
    json!({ "digest": lineage.fusion.digest(), "decisions": decisions })
}

/// The per-section report of `p` on `facts`' chip: `capacity`, the chip's units; `fusion`, the program's decisions
/// (null without a lineage); and `sections`, a row per section — its place and name, the statements it computes, the
/// decisions it holds fired and declined, and its unit use.
pub fn section_report(p: &SpatialProgram, lineage: Option<&Lineage>, facts: &RduChipFacts) -> Map<String, Value> {
    let spans = spans(p);
    if let Some(l) = lineage {
        assert_eq!(l.sections.len(), spans.len(), "{}: a lineage names the ops of every section of its program", p.name);
    }
    let rows: Vec<Value> = spans
        .iter()
        .zip(unit_use(p))
        .enumerate()
        .map(|(k, ((name, _, _), units))| {
            let mut row = json!({ "section": k, "name": name });
            if let Some(l) = lineage {
                row["statements"] = json!(l.sections[k]
                    .iter()
                    .map(|&o| json!({ "stmt": l.ops[o].stmt_index, "binder": l.ops[o].binder, "op": l.ops[o].op }))
                    .collect::<Vec<_>>());
                let (fired, declined): (Vec<usize>, Vec<usize>) =
                    l.decisions_in(k).into_iter().partition(|&i| matches!(l.fusion.decisions[i].outcome, Outcome::Fired));
                row["fired"] = json!(fired.into_iter().map(|i| listed(i, l)).collect::<Vec<_>>());
                row["declined"] = json!(declined.into_iter().map(|i| listed(i, l)).collect::<Vec<_>>());
            }
            row["units"] = json!(units);
            row
        })
        .collect();
    let mut m = Map::new();
    m.insert("capacity".into(), capacity(facts));
    m.insert("fusion".into(), lineage.map_or(Value::Null, fusion));
    m.insert("sections".into(), json!(rows));
    m
}

/// A model bundle's `report.json` for one program: the schema, what it reports on — the chip, the program and the
/// schedule digest its certificate binds — and [`section_report`].
pub fn program_report(p: &SpatialProgram, schedule_digest: u64, lineage: Option<&Lineage>, facts: &RduChipFacts) -> String {
    let mut m = Map::new();
    m.insert("schema".into(), json!(SECTION_REPORT_SCHEMA));
    m.insert("version".into(), json!(1));
    m.insert("provenance".into(), json!(crate::spatial::fit::PROVENANCE_MODELED));
    m.insert("no_speed_claim".into(), json!(true));
    m.insert("chip".into(), json!(facts.chip.as_str()));
    m.insert("program".into(), json!(p.name));
    m.insert("schedule_digest".into(), json!(format!("{schedule_digest:016x}")));
    m.extend(section_report(p, lineage, facts));
    format!("{}\n", serde_json::to_string_pretty(&Value::Object(m)).expect("JSON"))
}
