//! Residency — which tensors the SRAM holds, and what leaves the chip.
//!
//! ★ THE SRAM IS THE PMUS' MEMORY, AND IT IS ALLOCATED. A spatial chip's SRAM tier is the sum of its sites' local
//! memories: every buffer a program lays out takes a share of it, and a tensor held there between sections — an
//! intermediate one section writes and a later one reads, a state one step leaves for the next — takes more. It is
//! only a tier a tensor can LIVE in where any site reaches any other's memory (a crossbar, the RDU); a target whose
//! sites share data through a global memory (Tensix, a GPU) hands intermediates through that memory.
//!
//! Two decisions, at the two scopes that can make them:
//!   • a PROGRAM places its intermediates ([`place_activations`]): each lives from the phase that writes it to the
//!     last phase that reads it, and the live set of every phase must fit the SRAM its buffers leave — past it, the
//!     intermediate whose last use is furthest away spills first (linear-scan allocation over live ranges);
//!   • a DEPLOYMENT places the carried state ([`plan_states`]): a step's state outlives the program, every layer
//!     holds its own, and only the whole model knows what else is resident. The state that moves the most bytes per
//!     byte held — a recurrent memory read and written every step before a cache read once — is kept first, within
//!     each chip's SRAM less the largest program's own footprint.
//! What leaves the SRAM goes to the next tier down ([`plan_off_chip`]): an RDU's HBM while it holds it, densest first,
//! then the DDR behind it (`Dram`); a chip with no HBM keeps it all in its DRAM.

use std::collections::{BTreeMap, BTreeSet};

use super::ir::*;
use super::target::{precision_bytes, Interconnect, SpatialTarget};

/// Bytes `t` takes in memory: its tiles, whole.
pub fn tensor_bytes(p: &SpatialProgram, t: &TensorDecl) -> u64 {
    (t.tiles.rows_t * t.tiles.cols_t * p.tile.elems() * precision_bytes(t.dtype)) as u64
}

/// The SRAM each chip's buffers take: over every site of the chip, the highest byte its local memory map reaches.
pub fn buffer_footprint(p: &SpatialProgram) -> BTreeMap<u16, u64> {
    let mut high: BTreeMap<Site, u64> = BTreeMap::new();
    if let Some(mm) = &p.memory {
        for at in mm.buffer_at.iter().filter(|a| a.space != MemSpace::Acc) {
            let top = high.entry(at.site).or_default();
            *top = (*top).max(at.offset_bytes as u64 + at.bytes as u64);
        }
    }
    let mut out = BTreeMap::new();
    for (site, top) in high {
        *out.entry(site.chip).or_default() += top;
    }
    out
}

/// The chips whose sites load or store each tensor, by tensor id.
fn tensor_chips(p: &SpatialProgram) -> Vec<BTreeSet<u16>> {
    let mut out = vec![BTreeSet::new(); p.tensors.len()];
    let pl = p.placement.as_ref();
    for s in &p.stages {
        let (StageKind::Load { tensor, .. } | StageKind::Store { tensor, .. }) = &s.kind else {
            continue;
        };
        let site = pl
            .and_then(|pl| pl.site_of.get(s.id.idx()).copied())
            .or(s.site);
        out[tensor.idx()].insert(site.map(|x| x.chip).unwrap_or(0));
    }
    out
}

/// Each intermediate's live range over the program's phases: from the first phase that stores it to the last that
/// loads it.
fn live_ranges(p: &SpatialProgram) -> BTreeMap<TensorId, (usize, usize)> {
    let phases = p.section_phases();
    let phase_of = |s: StageId| {
        p.section_of_stage(s)
            .and_then(|k| phases.get(k).copied())
            .unwrap_or(0)
    };
    let mut out: BTreeMap<TensorId, (usize, usize)> = BTreeMap::new();
    for s in &p.stages {
        let (tensor, stores) = match &s.kind {
            StageKind::Load { tensor, .. } => (*tensor, false),
            StageKind::Store { tensor, .. } => (*tensor, true),
            _ => continue,
        };
        if p.tensor(tensor).role != TensorRole::Activation {
            continue;
        }
        let ph = phase_of(s.id);
        let r = out.entry(tensor).or_insert((ph, ph));
        if stores {
            r.0 = r.0.min(ph);
        }
        r.1 = r.1.max(ph);
    }
    out
}

/// Whether `target` can hold a tensor in its SRAM between sections: an SRAM tier any site reaches.
pub fn holds_tensors_on_chip(target: &SpatialTarget) -> bool {
    target.dram.sram_kib > 0 && target.provides_interconnect(Interconnect::Crossbar)
}

/// The chips each intermediate is held on: those whose sites load or store it, chip 0 for one no site touches.
fn held_on(chips: &[BTreeSet<u16>], t: TensorId) -> Vec<u16> {
    if chips[t.idx()].is_empty() {
        vec![0]
    } else {
        chips[t.idx()].iter().copied().collect()
    }
}

/// ★ PLACE THE INTERMEDIATES. Where `target` holds tensors on chip, each chip's intermediates are allocated over their
/// live ranges in the SRAM its buffers leave; where it does not, or past what fits, they stay in the tier they were
/// declared in. A live set past the SRAM spills, at the first phase it overflows, the intermediate live there whose
/// last use is furthest away — then the largest, then the lowest id — until every phase fits.
///
/// One sweep over the phases per chip (linear scan): a range joins the live set at its first phase and leaves after its
/// last, the live bytes are kept as a running sum, and an overflow spills from the live set ordered by last use. A
/// spill only lowers the phases after it, and every phase before already fit, so the sweep never looks back — the
/// same intermediates spill as re-searching from the first phase would, in `O(n log n)`.
pub fn place_activations(p: &mut SpatialProgram, target: &SpatialTarget) {
    let ranges = live_ranges(p);
    let chips = tensor_chips(p);
    let mut spilled: BTreeSet<TensorId> = BTreeSet::new();
    if holds_tensors_on_chip(target) {
        let sram = target.dram.sram_kib as u64 * 1024;
        let buffers = buffer_footprint(p);
        let mut by_chip: BTreeMap<u16, Vec<(TensorId, (usize, usize), u64)>> = BTreeMap::new();
        for (t, r) in &ranges {
            let b = tensor_bytes(p, p.tensor(*t));
            for c in held_on(&chips, *t) {
                by_chip.entry(c).or_default().push((*t, *r, b));
            }
        }
        for (chip, mut live) in by_chip {
            let room = sram.saturating_sub(buffers.get(&chip).copied().unwrap_or(0));
            live.sort_by_key(|(t, r, _)| (r.0, t.idx()));
            // The live set, ordered so its last element is the one to spill: furthest last use, then largest, then
            // lowest id. Ranges leave it in order of their last phase.
            let mut active: BTreeSet<(usize, u64, std::cmp::Reverse<usize>)> = BTreeSet::new();
            let mut ending: BTreeSet<(usize, usize, u64)> = BTreeSet::new();
            let mut sum = 0u64;
            let mut next = 0usize;
            let mut phases: Vec<usize> = live.iter().map(|(_, r, _)| r.0).collect();
            phases.dedup();
            for ph in phases {
                while let Some(&(last, id, b)) = ending.first() {
                    if last >= ph {
                        break;
                    }
                    ending.pop_first();
                    if active.remove(&(last, b, std::cmp::Reverse(id))) {
                        sum -= b;
                    }
                }
                while next < live.len() && live[next].1 .0 == ph {
                    let (t, r, b) = live[next];
                    next += 1;
                    if spilled.contains(&t) {
                        continue;
                    }
                    active.insert((r.1, b, std::cmp::Reverse(t.idx())));
                    ending.insert((r.1, t.idx(), b));
                    sum += b;
                }
                while sum > room {
                    let (last, b, std::cmp::Reverse(id)) = active
                        .pop_last()
                        .expect("an overflowing phase holds an intermediate");
                    ending.remove(&(last, id, b));
                    sum -= b;
                    spilled.insert(TensorId::new(id));
                }
            }
        }
    }
    let on_chip = holds_tensors_on_chip(target);
    for t in &mut p.tensors {
        if t.role == TensorRole::Activation {
            t.space = if on_chip && !spilled.contains(&t.id) {
                TensorSpace::Sram
            } else {
                TensorSpace::Dram
            };
        }
    }
}

/// The SRAM each chip of `p` holds while it runs: its buffers and the peak of its on-chip intermediates — the running
/// sum of the live bytes over the phases, a range added at its first phase and taken away after its last.
pub fn sram_footprint(p: &SpatialProgram) -> BTreeMap<u16, u64> {
    let ranges = live_ranges(p);
    let chips = tensor_chips(p);
    let mut out = buffer_footprint(p);
    let mut deltas: BTreeMap<u16, BTreeMap<usize, i128>> = BTreeMap::new();
    for (t, r) in &ranges {
        let d = p.tensor(*t);
        if d.space != TensorSpace::Sram {
            continue;
        }
        let b = tensor_bytes(p, d) as i128;
        for c in held_on(&chips, *t) {
            let e = deltas.entry(c).or_default();
            *e.entry(r.0).or_default() += b;
            *e.entry(r.1 + 1).or_default() -= b;
        }
    }
    for (c, d) in deltas {
        let (mut sum, mut peak) = (0i128, 0i128);
        for (_, delta) in d {
            sum += delta;
            peak = peak.max(sum);
        }
        *out.entry(c).or_default() += peak as u64;
    }
    out
}

/// A carried state of a deployment: `tensor` of the program `program`, held on chip `chip` once for each of the
/// `instances` layers that run the program, each `bytes` in memory and moving `moved` bytes a step.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StateClass {
    pub program: String,
    pub tensor: String,
    pub chip: u16,
    pub instances: usize,
    pub bytes: u64,
    pub moved: u64,
}

impl StateClass {
    /// Every instance's bytes.
    pub fn total(&self) -> u64 {
        self.bytes * self.instances as u64
    }
}

/// Where a deployment keeps its carried state: each chip's SRAM, the reserve its largest program takes while it runs,
/// and every state class with the tier it was given.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct ResidencyPlan {
    pub sram_bytes: u64,
    /// Each chip's HBM; `0` where the chip has none.
    pub hbm_bytes: u64,
    pub reserve: BTreeMap<u16, u64>,
    /// Every carried state, with its tier.
    pub placed: Vec<(StateClass, TensorSpace)>,
    /// Every other tensor off the SRAM — weights, runtime inputs, results, spilled intermediates — with its tier.
    pub off_chip: Vec<(StateClass, TensorSpace)>,
}

impl ResidencyPlan {
    /// The state `chip` keeps in its SRAM.
    pub fn on_chip(&self, chip: u16) -> u64 {
        self.placed
            .iter()
            .filter(|(c, s)| c.chip == chip && *s == TensorSpace::Sram)
            .map(|(c, _)| c.total())
            .sum()
    }

    /// The tier the plan gives `tensor` of `program`.
    pub fn tier_of(&self, program: &str, tensor: &str) -> Option<TensorSpace> {
        self.placed
            .iter()
            .find(|(c, _)| c.program == program && c.tensor == tensor)
            .map(|(_, s)| *s)
    }
}

/// ★ KEEP WHAT MOVES MOST PER BYTE HELD. In order of bytes moved a step per byte held — then the smaller class, then
/// by name — each class goes on chip whole if its chip has room left beyond `reserve`, and stays in HBM/DDR
/// (`Dram`) otherwise; a class that does not fit leaves the room to the classes after it. Every byte a class keeps
/// on chip is a byte a step does not move off it, `moved / bytes` times over, so filling the SRAM densest-first is
/// the most traffic whole classes can keep.
pub fn plan_states(
    classes: Vec<StateClass>,
    sram_bytes: u64,
    reserve: &BTreeMap<u16, u64>,
) -> ResidencyPlan {
    let mut order: Vec<usize> = (0..classes.len()).collect();
    order.sort_by(|&a, &b| {
        let (x, y) = (&classes[a], &classes[b]);
        (y.moved as u128 * x.bytes as u128)
            .cmp(&(x.moved as u128 * y.bytes as u128))
            .then(x.total().cmp(&y.total()))
            .then((&x.program, &x.tensor).cmp(&(&y.program, &y.tensor)))
    });
    let mut room: BTreeMap<u16, u64> = BTreeMap::new();
    let mut tier = vec![TensorSpace::Dram; classes.len()];
    for i in order {
        let c = &classes[i];
        let left = room.entry(c.chip).or_insert_with(|| {
            sram_bytes.saturating_sub(reserve.get(&c.chip).copied().unwrap_or(0))
        });
        if c.total() <= *left {
            *left -= c.total();
            tier[i] = TensorSpace::Sram;
        }
    }
    ResidencyPlan {
        sram_bytes,
        hbm_bytes: 0,
        reserve: reserve.clone(),
        placed: classes.into_iter().zip(tier).collect(),
        off_chip: Vec::new(),
    }
}

/// ★ THE NEXT TIER DOWN. Each class the SRAM does not hold goes, in the order [`plan_states`] ranks by, to its chip's
/// HBM while `hbm_bytes` holds it whole, and to the DDR (`Dram`) past it; with no HBM, all of it to the DRAM.
pub fn plan_off_chip(classes: &[StateClass], hbm_bytes: u64) -> Vec<TensorSpace> {
    let mut order: Vec<usize> = (0..classes.len()).collect();
    order.sort_by(|&a, &b| {
        let (x, y) = (&classes[a], &classes[b]);
        (y.moved as u128 * x.bytes as u128)
            .cmp(&(x.moved as u128 * y.bytes as u128))
            .then(x.total().cmp(&y.total()))
            .then((&x.program, &x.tensor).cmp(&(&y.program, &y.tensor)))
    });
    let mut room: BTreeMap<u16, u64> = BTreeMap::new();
    let mut tier = vec![TensorSpace::Dram; classes.len()];
    for i in order {
        let c = &classes[i];
        let left = room.entry(c.chip).or_insert(hbm_bytes);
        if hbm_bytes > 0 && c.total() <= *left {
            *left -= c.total();
            tier[i] = TensorSpace::Hbm;
        }
    }
    tier
}

/// Bytes each tensor of `p` moves a step, every loop at its bound.
pub fn moved_bytes(p: &SpatialProgram) -> Vec<u64> {
    let mut out = vec![0u64; p.tensors.len()];
    let tile = p.tile.elems() as u64;
    if let Some(sch) = &p.schedule {
        for prog in &sch.programs {
            walk_tagged(&prog.ops, 1, &mut |t, m| {
                if let ChoreoOp::ReadTiles {
                    tensor,
                    block_h,
                    block_w,
                    ..
                }
                | ChoreoOp::WriteTiles {
                    tensor,
                    block_h,
                    block_w,
                    ..
                } = &t.op
                {
                    out[tensor.idx()] += m
                        * (*block_h * *block_w) as u64
                        * tile
                        * precision_bytes(p.tensor(*tensor).dtype) as u64;
                }
            });
        }
    }
    out
}

/// One program of a deployment: its name, the layers that run it, the program, and — when the lowering knows it —
/// which of its `State` tensors a step carries to the next. A state the step assembles whole each time is the step's
/// own table: it lives while the program runs, like an intermediate, and is not the deployment's to keep.
pub type DeployedProgram<'a> = (
    &'a str,
    usize,
    &'a mut SpatialProgram,
    Option<&'a BTreeSet<String>>,
);

/// A deployment's carried state planned and placed: every CARRIED `State` tensor of every program becomes a
/// [`StateClass`], once per layer that runs the program; the reserve is each chip's largest footprint while a program
/// runs — [`sram_footprint`] and the program's own tables, kept on chip when the program has room for them — and
/// every carried state takes the tier [`plan_states`] gives it.
pub fn plan_deployment(programs: &mut [DeployedProgram], target: &SpatialTarget) -> ResidencyPlan {
    let sram = if holds_tensors_on_chip(target) {
        target.dram.sram_kib as u64 * 1024
    } else {
        0
    };
    let mut reserve: BTreeMap<u16, u64> = BTreeMap::new();
    let mut classes = Vec::new();
    for (name, instances, p, carried) in programs.iter_mut() {
        let chips = tensor_chips(p);
        let chip_of = |t: &TensorDecl| chips[t.id.idx()].iter().next().copied().unwrap_or(0);
        let own = |t: &TensorDecl| {
            t.role == TensorRole::State && carried.is_some_and(|c| !c.contains(&t.name))
        };
        let mut footprint = sram_footprint(p);
        let mut tables: BTreeMap<u16, u64> = BTreeMap::new();
        for t in p.tensors.iter().filter(|t| own(t)) {
            *tables.entry(chip_of(t)).or_default() += tensor_bytes(p, t);
        }
        let fits = tables
            .iter()
            .all(|(c, b)| footprint.get(c).copied().unwrap_or(0) + b <= sram);
        let table_ids: Vec<TensorId> = p.tensors.iter().filter(|t| own(t)).map(|t| t.id).collect();
        for id in table_ids {
            p.tensors[id.idx()].space = if fits {
                TensorSpace::Sram
            } else {
                TensorSpace::Dram
            };
        }
        if fits {
            for (c, b) in tables {
                *footprint.entry(c).or_default() += b;
            }
        }
        for (c, b) in footprint {
            let r = reserve.entry(c).or_default();
            *r = (*r).max(b);
        }
        let moved = moved_bytes(p);
        for t in p
            .tensors
            .iter()
            .filter(|t| t.role == TensorRole::State && !own(t))
        {
            classes.push(StateClass {
                program: name.to_string(),
                tensor: t.name.clone(),
                chip: chip_of(t),
                instances: *instances,
                bytes: tensor_bytes(p, t),
                moved: moved[t.id.idx()],
            });
        }
    }
    let mut plan = plan_states(classes, sram, &reserve);
    // Everything the SRAM does not hold — a carried state it could not keep, and every tensor that was never a
    // candidate — takes the next tier down, one class per tensor of each program, once per layer that runs it.
    let hbm = target.dram.hbm_gib as u64 * (1 << 30);
    let placed_at: std::collections::HashMap<(&str, &str), usize> = plan
        .placed
        .iter()
        .enumerate()
        .map(|(i, (c, _))| ((c.program.as_str(), c.tensor.as_str()), i))
        .collect();
    let mut rest: Vec<(StateClass, Option<usize>)> = Vec::new();
    for (name, instances, p, _) in programs.iter() {
        let chips = tensor_chips(p);
        let moved = moved_bytes(p);
        for t in &p.tensors {
            let carried = placed_at.get(&(*name, t.name.as_str())).copied();
            let on_chip = match carried {
                Some(i) => plan.placed[i].1 == TensorSpace::Sram,
                None => t.space == TensorSpace::Sram,
            };
            if !on_chip {
                let chip = chips[t.id.idx()].iter().next().copied().unwrap_or(0);
                rest.push((
                    StateClass {
                        program: name.to_string(),
                        tensor: t.name.clone(),
                        chip,
                        instances: *instances,
                        bytes: tensor_bytes(p, t),
                        moved: moved[t.id.idx()],
                    },
                    carried,
                ));
            }
        }
    }
    drop(placed_at);
    let classes: Vec<StateClass> = rest.iter().map(|(c, _)| c.clone()).collect();
    let tiers = plan_off_chip(&classes, hbm);
    plan.hbm_bytes = hbm;
    for ((c, carried), tier) in rest.into_iter().zip(tiers) {
        match carried {
            Some(i) => plan.placed[i].1 = tier,
            None => plan.off_chip.push((c, tier)),
        }
    }
    let tier_at: std::collections::HashMap<(&str, &str), TensorSpace> = plan
        .placed
        .iter()
        .chain(plan.off_chip.iter())
        .map(|(c, s)| ((c.program.as_str(), c.tensor.as_str()), *s))
        .collect();
    for (name, _, p, _) in programs.iter_mut() {
        for t in p.tensors.iter_mut() {
            if let Some(tier) = tier_at.get(&(*name, t.name.as_str())) {
                t.space = *tier;
            }
        }
    }
    drop(tier_at);
    plan
}
