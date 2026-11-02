//! Whether a persona build fits the card's partial-reconfiguration region and can close its clock, decided before
//! Quartus runs, from what the build ledger has measured.
//!
//! A design is read as the timing model reads it ([`Design::of`] a [`TimingGraph`]): its mapped area, its deepest
//! register-to-register path and its loop bound. The ALUTs it will take are the calibration's area fit of that area
//! ([`AreaFit`]); the ALMs, the larger of those ALUTs and its registers each at the ratio Quartus's own syntheses in the
//! ledger packed theirs ([`packing`]); the DSP blocks, the model's halves paired, an upper bound, since Quartus maps some small products
//! to logic. The capacity is the region the ledger's last persona was placed in ([`capacity`]). A design over the
//! region's ALMs, DSP blocks or registers is refused; one past the fullest persona that has fit, and still inside the
//! region, is tight. The deepest path must fit the period, and so must the loop bound, which no retiming can cut; a
//! loop through no register is refused outright. A design whose parts step at different rates (an engine core
//! stepping once in many clocks under a multicycle constraint beside a side stepping every clock) is held to its area
//! whole and timed part by part ([`preflight_domains`]), each part's paths given its cycles of the period. The fitter's time is predicted from the personas' own, a power law
//! fitted to their ALMs. A verdict that has no measurement to rest on (no area fit, no region) is a refusal too.

use std::fmt;

use logicaffeine_proof::cycle_ratio::{Bound, GraphError};

use super::quartus::{Ledger, RegionUsage, Row};
use super::timing_graph::{Area, AreaFit, Calibration, TimingError, TimingGraph};

/// The loop bound of a design: no loop, a loop through no register, or the most delay a register a loop holds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LoopBound {
    Acyclic,
    Combinational,
    Ratio(f64),
}

/// A design as the timing model reads it: its area, its deepest path and its loop bound, in the model's unit.
#[derive(Clone, Debug, PartialEq)]
pub struct Design {
    pub area: Area,
    pub depth: i64,
    pub loop_bound: LoopBound,
}

/// Why a timing graph is no design: one the model could not time, or not a graph.
#[derive(Debug)]
pub enum DesignError {
    Timing(TimingError),
    Graph(GraphError),
}

impl fmt::Display for DesignError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DesignError::Timing(e) => write!(f, "{e}"),
            DesignError::Graph(e) => write!(f, "{e:?}"),
        }
    }
}

impl Design {
    /// The design `graph` is. A loop through no register has no deepest path; its design's depth is 0, and the loop
    /// is its verdict.
    pub fn of(graph: &TimingGraph) -> Result<Design, DesignError> {
        let loop_bound = match graph.loop_bound().map_err(DesignError::Graph)? {
            Bound::Acyclic => LoopBound::Acyclic,
            Bound::CombinationalLoop(_) => LoopBound::Combinational,
            Bound::Ratio { delay, registers, .. } => LoopBound::Ratio(delay as f64 / registers as f64),
        };
        let depth = match graph.depth() {
            Ok(d) => d.depth,
            Err(TimingError::CombinationalLoop(_)) if loop_bound == LoopBound::Combinational => 0,
            Err(e) => return Err(DesignError::Timing(e)),
        };
        Ok(Design { area: graph.area_under(""), depth, loop_bound })
    }

    /// The design with `loop_bound` for its loop bound.
    pub fn with_loop(self, loop_bound: LoopBound) -> Design {
        Design { loop_bound, ..self }
    }
}

/// The clock a design must close: its period, and the nanoseconds one unit of the model's delay takes on the card.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Target {
    pub period_ns: f64,
    pub ns_per_unit: f64,
}

/// What a persona may use: the region the ledger's last persona was placed in, and that compile's name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Capacity {
    pub row: String,
    pub alms: u64,
    pub registers: u64,
    pub m20k: u64,
    pub dsp: u64,
}

/// The PR slot a persona is placed in.
const PR_SLOT: &str = "afu_top|port_gasket|pr_slot|afu_main";

/// The PR region a persona's fit was placed in: a compile of the persona revision (`iofs_pr_afu`) records the PR slot's
/// region. A flat image's fit (the FIM's own revision) is placed over the whole device, and the regions it records are
/// its floorplan's Logic Lock regions: no persona's.
fn persona_region(r: &Row) -> Option<&RegionUsage> {
    if r.build.flow.revision.as_deref() != Some("iofs_pr_afu") {
        return None;
    }
    r.build.regions.iter().find(|g| g.name == PR_SLOT)
}

/// The PR region of the last persona compile the ledger records with one.
pub fn capacity(ledger: &Ledger) -> Option<Capacity> {
    ledger.rows.iter().rev().find_map(|r| {
        let region = persona_region(r)?;
        Some(Capacity {
            row: r.name.clone(),
            alms: region.alms,
            registers: region.registers.map_or(0, |x| x.total),
            m20k: region.m20k.map_or(0, |x| x.total),
            dsp: region.dsp.map_or(0, |x| x.total),
        })
    })
}

/// How Quartus packs a synthesis into ALMs: the ALMs an ALUT takes, over the syntheses their ALUTs set, and the ALMs a
/// register takes, over the syntheses their registers set (none when the ledger records no such synthesis).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Packing {
    pub per_alut: f64,
    pub per_register: Option<f64>,
}

impl Packing {
    /// The ALMs a design of `aluts` ALUTs and `registers` registers takes: the larger of the two packings.
    pub fn alms(&self, aluts: f64, registers: f64) -> f64 {
        let by_aluts = aluts * self.per_alut;
        self.per_register.map_or(by_aluts, |r| by_aluts.max(registers * r))
    }
}

/// The packing every synthesis the ledger records from one Verilog file shows: each synthesis's estimated ALMs over its
/// top's ALUTs and registers, each synthesis set by the term the larger packing gives it (first by which it holds more
/// of), each packing the geometric mean over the syntheses it sets, settled when no synthesis changes side.
pub fn packing(ledger: &Ledger) -> Option<Packing> {
    let rows: Vec<(f64, f64, f64)> = ledger
        .rows
        .iter()
        .filter(|r| r.synthesis_sources().is_some())
        .filter_map(|r| {
            let alms = r.build.synthesis.first()?.estimate.alms?;
            let top = r.build.top.iter().find(|n| n.path == "|")?;
            (alms > 0 && top.aluts > 0).then(|| (alms as f64, top.aluts as f64, top.registers as f64))
        })
        .collect();
    let geomean = |xs: Vec<f64>| (!xs.is_empty()).then(|| (xs.iter().map(|x| x.ln()).sum::<f64>() / xs.len() as f64).exp());
    let fit = |by_registers: &[bool]| -> Option<Packing> {
        let lut = rows.iter().zip(by_registers).filter(|(_, r)| !**r).map(|(&(a, l, _), _)| a / l).collect();
        let reg = rows.iter().zip(by_registers).filter(|(_, r)| **r).map(|(&(a, _, g), _)| a / g).collect();
        Some(Packing { per_alut: geomean(lut)?, per_register: geomean(reg) })
    };
    let mut sides: Vec<bool> = rows.iter().map(|&(_, l, g)| g > l).collect();
    let mut p = fit(&sides).or_else(|| fit(&vec![false; rows.len()]))?;
    for _ in 0..=rows.len() {
        let next: Vec<bool> = rows.iter().map(|&(_, l, g)| p.per_register.is_some_and(|r| g * r > l * p.per_alut)).collect();
        if next == sides {
            break;
        }
        p = fit(&next).unwrap_or(p);
        sides = next;
    }
    Some(p)
}

/// The ALMs Quartus packs an ALUT into ([`packing`]'s, over the syntheses their ALUTs set).
pub fn alms_per_alut(ledger: &Ledger) -> Option<f64> {
    packing(ledger).map(|p| p.per_alut)
}

/// The fullest persona that fit: the passed compile whose region needed the most ALMs, and those ALMs.
pub fn fullest(ledger: &Ledger) -> Option<(String, f64)> {
    ledger
        .rows
        .iter()
        .filter(|r| r.verdict.passed)
        .filter_map(|r| persona_region(r).map(|g| (r.name.clone(), g.alms_needed)))
        .max_by(|a, b| a.1.total_cmp(&b.1))
}

/// The fitter's time for a persona of `alms` ALMs: `seconds = a × alms^b`, least squares in logs over every persona
/// fit the ledger records with its region and fitter time; none without two of different sizes.
pub fn fitter_seconds(ledger: &Ledger, alms: f64) -> Option<f64> {
    let points: Vec<(f64, f64)> = ledger
        .rows
        .iter()
        .filter_map(|r| {
            let needed = persona_region(r)?.alms_needed;
            let seconds = r.build.flow.phase("Fitter")?.seconds;
            (needed > 0.0 && seconds > 0).then(|| (needed.ln(), (seconds as f64).ln()))
        })
        .collect();
    let n = points.len() as f64;
    let (mx, my) = (points.iter().map(|p| p.0).sum::<f64>() / n, points.iter().map(|p| p.1).sum::<f64>() / n);
    let sxx: f64 = points.iter().map(|p| (p.0 - mx).powi(2)).sum();
    if points.len() < 2 || sxx < 1e-12 {
        return None;
    }
    let b = points.iter().map(|p| (p.0 - mx) * (p.1 - my)).sum::<f64>() / sxx;
    Some((my + b * (alms.ln() - mx)).exp())
}

/// A part of a design timed on its own: its name, its design, and the periods its paths have.
#[derive(Clone, Copy, Debug)]
pub struct Timed<'a> {
    pub name: &'a str,
    pub design: &'a Design,
    pub cycles: u32,
}

/// What a design is predicted to take and how fast it can run.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Predicted {
    pub aluts: f64,
    pub alms: f64,
    pub dsp: u64,
    pub registers: u64,
    /// Each timed part's deepest path and loop bound in nanoseconds (none for a part with no loop through a
    /// register), and the periods its paths have.
    pub timing: Vec<PartTiming>,
    pub fitter_seconds: Option<f64>,
}

/// One timed part's figures.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PartTiming {
    pub name: String,
    pub cycles: u32,
    pub depth_ns: f64,
    pub loop_ns: Option<f64>,
}

/// Why a design is not built.
#[derive(Clone, Debug, PartialEq)]
pub enum Refusal {
    /// More than the region holds.
    Over { what: &'static str, predicted: f64, capacity: u64 },
    /// More than the device holds.
    Device { what: &'static str, needs: u64, holds: u64 },
    /// A part's deepest path past the time its paths have.
    Depth { part: String, ns: f64, budget: f64 },
    /// A part's loop whose bound is past the time its paths have, or a loop through no register.
    Loop { part: String, ns: Option<f64>, budget: f64 },
    /// Nothing measured to predict from.
    Unmeasured { what: &'static str },
}

impl Refusal {
    /// What the refusal is of.
    pub fn what(&self) -> &'static str {
        match self {
            Refusal::Over { what, .. } | Refusal::Device { what, .. } | Refusal::Unmeasured { what } => what,
            Refusal::Depth { .. } => "the deepest path",
            Refusal::Loop { .. } => "a loop",
        }
    }
}

/// `"part: "`, or nothing for a design timed whole.
fn of(part: &str) -> String {
    if part.is_empty() {
        String::new()
    } else {
        format!("{part}: ")
    }
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Refusal::Over { what, predicted, capacity } => write!(f, "{predicted:.0} {what}, where the region holds {capacity}"),
            Refusal::Device { what, needs, holds } => write!(f, "{needs} {what}, where the device holds {holds}"),
            Refusal::Depth { part, ns, budget } => {
                write!(f, "{}the deepest path takes {ns:.2} ns, past the {budget} ns its paths have: pipeline it", of(part))
            }
            Refusal::Loop { part, ns: Some(ns), budget } => {
                write!(f, "{}a loop takes {ns:.2} ns a register, past the {budget} ns its paths have: no retiming reaches it", of(part))
            }
            Refusal::Loop { part, ns: None, .. } => write!(f, "{}a loop through no register", of(part)),
            Refusal::Unmeasured { what } => write!(f, "no {what} to predict from"),
        }
    }
}

/// A design's preflight: what it is predicted to take, the region it is held to, every refusal, and every way it is
/// tight without being refused.
#[derive(Clone, Debug, PartialEq)]
pub struct Preflight {
    pub predicted: Predicted,
    pub capacity: Option<Capacity>,
    pub refusals: Vec<Refusal>,
    pub tight: Vec<String>,
}

/// The preflight of `design` against what `ledger` and `calibration` measured, at `target`, timed whole a period a
/// path.
pub fn preflight(design: &Design, ledger: &Ledger, calibration: &Calibration, target: &Target) -> Preflight {
    preflight_domains(&design.area, &[Timed { name: "", design, cycles: 1 }], ledger, calibration, target)
}

/// The preflight of a design of `area`, each of its `parts` timed on its own at its cycles of `target`'s period.
pub fn preflight_domains(area: &Area, parts: &[Timed], ledger: &Ledger, calibration: &Calibration, target: &Target) -> Preflight {
    let mut refusals = Vec::new();
    let mut tight = Vec::new();
    let mut predicted = Predicted { dsp: area.half_dsps.div_ceil(2), registers: area.flops, ..Predicted::default() };
    let fit: Option<&AreaFit> = calibration.area.as_ref();
    match (fit, packing(ledger)) {
        (Some(fit), Some(packing)) => {
            predicted.aluts = fit.aluts(area);
            predicted.alms = packing.alms(predicted.aluts, area.flops as f64);
        }
        _ => refusals.push(Refusal::Unmeasured { what: "the area fit" }),
    }
    let capacity = capacity(ledger);
    match &capacity {
        Some(c) => {
            if predicted.alms > c.alms as f64 {
                refusals.push(Refusal::Over { what: "ALMs", predicted: predicted.alms, capacity: c.alms });
            } else if let Some((row, alms)) = fullest(ledger).filter(|(_, a)| predicted.alms > *a) {
                tight.push(format!("{:.0} ALMs, past {alms:.0}, the fullest persona that fit ({row})", predicted.alms));
            }
            if predicted.dsp > c.dsp {
                refusals.push(Refusal::Over { what: "DSP blocks", predicted: predicted.dsp as f64, capacity: c.dsp });
            }
            if predicted.registers > c.registers {
                refusals.push(Refusal::Over { what: "registers", predicted: predicted.registers as f64, capacity: c.registers });
            }
            predicted.fitter_seconds = fitter_seconds(ledger, predicted.alms.max(1.0));
        }
        None => refusals.push(Refusal::Unmeasured { what: "the PR region" }),
    }
    for part in parts {
        let budget = target.period_ns * part.cycles as f64;
        let timing = PartTiming {
            name: part.name.to_string(),
            cycles: part.cycles,
            depth_ns: part.design.depth as f64 * target.ns_per_unit,
            loop_ns: match part.design.loop_bound {
                LoopBound::Ratio(r) => Some(r * target.ns_per_unit),
                _ => None,
            },
        };
        if timing.depth_ns > budget {
            refusals.push(Refusal::Depth { part: timing.name.clone(), ns: timing.depth_ns, budget });
        }
        match part.design.loop_bound {
            LoopBound::Combinational => refusals.push(Refusal::Loop { part: timing.name.clone(), ns: None, budget }),
            LoopBound::Ratio(_) if timing.loop_ns.is_some_and(|ns| ns > budget) => {
                refusals.push(Refusal::Loop { part: timing.name.clone(), ns: timing.loop_ns, budget })
            }
            _ => {}
        }
        predicted.timing.push(timing);
    }
    Preflight { predicted, capacity, refusals, tight }
}

/// The device primitives `verilog` instantiates and does not define, each declared as the box it is: the card's
/// floating-point DSP block (`fourteennm_fp_mac`, [`super::fp_oracle::fp_mac_blackbox`]), which only Quartus elaborates.
pub fn device_library(verilog: &str) -> String {
    const FP_MAC: &str = "fourteennm_fp_mac";
    let (mut defined, mut used) = (false, false);
    for line in verilog.lines() {
        let words: Vec<&str> = line.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '$')).filter(|w| !w.is_empty()).collect();
        for (i, w) in words.iter().enumerate() {
            if *w == FP_MAC {
                if i > 0 && words[i - 1] == "module" {
                    defined = true;
                } else {
                    used = true;
                }
            }
        }
    }
    if used && !defined {
        super::fp_oracle::fp_mac_blackbox()
    } else {
        String::new()
    }
}

/// A design's or a device's ALMs, DSP blocks and M20K blocks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Holds {
    pub alms: u64,
    pub dsp: u64,
    pub m20k: u64,
}

/// What the D5005's device holds: the 1SX280's ALMs, DSP blocks and M20Ks.
pub const DEVICE: Holds = Holds { alms: 933_120, dsp: 5_760, m20k: 11_721 };

/// What a synthesized flat image needs of the device, from its synthesis report: the ALMs and DSP blocks its partitions'
/// estimates sum to, and the M20K blocks the fitter gives its memories (`floorplan::Ram::m20k`, every partition's), not
/// its block memory bits over a block's 20,480: a stack of blocks a power of two deep, and a byte-enabled block's 32
/// bits, take more blocks than the bits do. None when the report gives no partition's ALMs or DSP blocks.
pub fn synthesized_holds(syn: &str) -> Option<Holds> {
    let partitions = super::quartus::parse_synthesis(syn);
    let sum = |of: fn(&super::quartus::Estimate) -> Option<u64>| -> Option<u64> {
        let each: Vec<u64> = partitions.iter().filter_map(|p| of(&p.estimate)).collect();
        (!each.is_empty()).then(|| each.iter().sum())
    };
    Some(Holds {
        alms: sum(|e| e.alms)?,
        dsp: sum(|e| e.dsp_blocks)?,
        m20k: super::floorplan::synthesized_rams(syn).iter().map(super::floorplan::Ram::m20k).sum(),
    })
}

/// Each resource `needs` takes past what `device` holds, named.
pub fn over(needs: Holds, device: Holds) -> Vec<Refusal> {
    [("ALMs", needs.alms, device.alms), ("DSP blocks", needs.dsp, device.dsp), ("M20K blocks", needs.m20k, device.m20k)]
        .into_iter()
        .filter(|&(_, n, d)| n > d)
        .map(|(what, needs, holds)| Refusal::Device { what, needs, holds })
        .collect()
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/codegen_sva/preflight/tests.rs"]
mod tests;
