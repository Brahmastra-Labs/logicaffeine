//! The programmable-part resources a fabric program spends, counted from exactly what the printer declares, so a
//! program too big for its part is refused before a line of RTL is printed.
//!
//! Every count is a LOWER bound — the fewest of a resource the declared memories and arrays can take — so a
//! refusal is sound: the real design needs at least this much, and this much already does not fit. A block-RAM
//! bank cannot hold more bits than its RAMBs do, a distributed-RAM memory cannot pack its bits into fewer than
//! one LUT per 64 PER READ PORT (each port is its own LUT copy of the data — a pool read through a gather's lanes
//! replicates once a lane), a registers-style memory is one flip-flop a bit, and a matmul array is at least one DSP
//! a MAC. Each is the bits of the memories the printer declares, divided down — never more than a routed image used.

use std::collections::BTreeMap;

use super::*;
use crate::spatial::lower::{CapacityRefusal, FabricResource, Over};
use crate::spatial::part::MemoryKind;

/// Bits one RAMB18 holds, data and parity (a 36 Kb block RAM is two of these).
const RAMB18_BITS: u64 = 18_432;
/// Bits one distributed-RAM LUT holds (a 64 × 1 memory).
const LUTRAM_BITS: u64 = 64;

/// A memory the fabric prints: the arguments one `memory_decl` / `print_pool` call receives, and the read ports it
/// is read through (each a LUT copy when it is distributed RAM — a pool's `nread`, a tensor memory's one).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeclaredMemory {
    pub name: String,
    pub words: usize,
    pub width: u32,
    pub banks: usize,
    pub lat: u64,
    pub read_ports: usize,
}

impl DeclaredMemory {
    fn storage(&self) -> Storage {
        Storage::of(self.words, self.banks, self.lat)
    }

    /// The bits the declaration holds: every bank, every row, every lane — `banks × ⌈words/banks⌉ × width`, the
    /// shape `memory_decl` prints.
    pub fn bits(&self) -> u64 {
        let rows = self.words.max(1).div_ceil(self.banks) as u64;
        self.banks as u64 * rows * self.width as u64
    }
}

/// Every memory the fabric declares for `p` on `target`, in printer order: each site's tensor memories (as
/// `print_site` emits them, one per tensor in the site's banks) then its pools (as `print_pool` does).
pub fn fabric_memories(p: &SpatialProgram, target: &SpatialTarget) -> Result<Vec<DeclaredMemory>, SpatialLowerError> {
    let ctx = context(p, target)?;
    let plans = plans(&ctx)?;
    let banks = ctx.banks;
    let mut out = Vec::new();
    for site in p.sites() {
        for t in &p.tensors {
            out.push(DeclaredMemory {
                name: format!("mem_t{}", t.id.idx()),
                words: ctx.tensor_words(t),
                width: tensor_width(t, &ctx.ranges),
                banks,
                lat: ctx.lat,
                read_ports: 1,
            });
        }
        for (i, pool) in site_pools(&ctx, &plans, site).into_iter().enumerate() {
            out.push(DeclaredMemory {
                name: format!("pool_{}_{}_{i}", site.x, site.y),
                words: pool.words,
                width: pool.width,
                banks,
                lat: ctx.lat,
                read_ports: pool.nread,
            });
        }
    }
    Ok(out)
}

/// What a fabric spends of each part resource: a lower bound on the RAMB18s, LUT-RAM LUTs, flip-flops and DSP
/// slices the printed design takes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FabricUsage {
    pub ramb18: u64,
    pub lutram_luts: u64,
    pub flops: u64,
    pub dsp: u64,
    pub memories: Vec<DeclaredMemory>,
}

/// The resources `p` spends on `target`'s fabric.
pub fn fabric_usage(p: &SpatialProgram, target: &SpatialTarget) -> Result<FabricUsage, SpatialLowerError> {
    let memories = fabric_memories(p, target)?;
    let (mut block_bits, mut lutram_luts, mut ff_bits) = (0u64, 0u64, 0u64);
    for m in &memories {
        match m.storage() {
            Storage::Block => block_bits += m.bits(),
            // Each read port is its own LUT copy, and a copy of B bits is at least ⌈B/64⌉ LUTs.
            Storage::Distributed => lutram_luts += m.bits().div_ceil(LUTRAM_BITS) * m.read_ports as u64,
            Storage::Registers | Storage::Flops => ff_bits += m.bits(),
        }
    }
    let ctx = context(p, target)?;
    let plans = plans(&ctx)?;
    // A matmul array is at least one DSP per multiply-accumulate; wider operands than one multiplier only take more.
    let dsp = plans
        .iter()
        .filter_map(|pl| pe_array(&pl.micro))
        .map(|(k, n, _)| (k * n) as u64)
        .sum();
    Ok(FabricUsage {
        ramb18: block_bits.div_ceil(RAMB18_BITS),
        lutram_luts,
        flops: ff_bits,
        dsp,
        memories,
    })
}

/// The memories of a storage class, largest first, named for a refusal.
fn largest_of(mems: &[DeclaredMemory], want: Storage) -> Vec<(String, u64)> {
    let mut v: Vec<(String, u64)> = mems.iter().filter(|m| m.storage() == want).map(|m| (m.name.clone(), m.bits())).collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    v.truncate(3);
    v
}

/// Refuse `p` on `target` when it needs more of any of the part's resources than it has — before any RTL is
/// printed. A target with no part ([`SpatialTarget::part`] `None`) is never refused here.
pub fn check_capacity(p: &SpatialProgram, target: &SpatialTarget) -> Result<(), SpatialLowerError> {
    let Some(part) = target.part else {
        return Ok(());
    };
    let usage = fabric_usage(p, target)?;
    let have = |kind: MemoryKind| part.memory(kind).map_or(0, |m| m.count.value);
    let block_as_ramb18 = have(MemoryKind::Block36) * 2;
    let budgets: &[(FabricResource, u64, u64, Option<Storage>)] = &[
        (FabricResource::BlockRam18, usage.ramb18, block_as_ramb18, Some(Storage::Block)),
        (FabricResource::LutRam, usage.lutram_luts, have(MemoryKind::LutRam), Some(Storage::Distributed)),
        (FabricResource::Flops, usage.flops, part.flops.value, Some(Storage::Registers)),
        (FabricResource::Dsp, usage.dsp, part.dsp.value, None),
    ];
    let mut over = Vec::new();
    for &(resource, need, have, storage) in budgets {
        if need > have {
            let largest = storage.map(|s| largest_of(&usage.memories, s)).unwrap_or_default();
            over.push(Over { resource, need, have, largest });
        }
    }
    if over.is_empty() {
        Ok(())
    } else {
        Err(SpatialLowerError::OverCapacity(CapacityRefusal { part: part.part, over }))
    }
}

/// A map from each resource to its usage, for a test to read.
pub fn usage_by_resource(u: &FabricUsage) -> BTreeMap<&'static str, u64> {
    [
        (FabricResource::BlockRam18.label(), u.ramb18),
        (FabricResource::LutRam.label(), u.lutram_luts),
        (FabricResource::Flops.label(), u.flops),
        (FabricResource::Dsp.label(), u.dsp),
    ]
    .into_iter()
    .collect()
}
