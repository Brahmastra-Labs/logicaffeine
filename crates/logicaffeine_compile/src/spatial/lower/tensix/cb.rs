//! Buffers → circular buffers. One `CircularBufferConfig` per buffer TEMPLATE (the pre-placement buffer
//! every site instantiates), with a deterministic index class: operand inputs `c_0..`, outputs `c_16..`,
//! spill buffers `c_24..`.

use std::collections::BTreeMap;

use super::{template_name, template_of, SpatialLowerError};
use crate::codegen_sva::fabric_target::Precision;
use crate::spatial::ir::*;
use crate::spatial::target::SpatialTarget;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CbSlot {
    pub template: BufferId,
    pub name: String,
    pub index: u32,
    pub dtype: Precision,
    /// The template's page: the LARGEST any site's instance holds. Every site of the template reserves,
    /// pushes, waits and pops in it, so each reservation divides the buffer; a boundary site whose own page
    /// is narrower fills only the front of each slot (and multicasts only that — `kernel::sent_tiles`).
    pub page_tiles: usize,
    pub depth_pages: usize,
    pub tile_bytes: usize,
    /// Every site that instantiates the template.
    pub sites: Vec<Site>,
    /// The template whose memory this one's pages live in (`Buffer::alias_of`): both are indices of one
    /// `CircularBufferConfig`, and this slot has none of its own.
    pub alias_of: Option<BufferId>,
}

impl CbSlot {
    pub fn total_bytes(&self) -> usize {
        self.page_tiles * self.depth_pages * self.tile_bytes
    }
    pub fn c_name(&self) -> String {
        format!("CB_{}", self.name.to_uppercase())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct CbPlan {
    pub slots: Vec<CbSlot>,
    by_template: BTreeMap<BufferId, usize>,
    by_instance: BTreeMap<BufferId, usize>,
}

/// The tt-metal data format of a precision the Tensix engines carry.
pub fn data_format(p: Precision) -> Result<&'static str, SpatialLowerError> {
    Ok(match p {
        Precision::Bf16 => "tt::DataFormat::Float16_b",
        Precision::Fp16 => "tt::DataFormat::Float16",
        Precision::Fp32 => "tt::DataFormat::Float32",
        Precision::Int8 => "tt::DataFormat::Int8",
        Precision::Int16 | Precision::Int32 | Precision::Int64 | Precision::Fp8 => {
            return Err(SpatialLowerError::Precision { needed: p })
        }
    })
}

impl CbPlan {
    pub fn of(p: &SpatialProgram, target: &SpatialTarget) -> Result<CbPlan, SpatialLowerError> {
        let mut inputs: BTreeMap<BufferId, ()> = Default::default();
        let mut outputs: BTreeMap<BufferId, ()> = Default::default();
        for s in &p.stages {
            match &s.kind {
                StageKind::Load { .. } | StageKind::Recv { .. } => {
                    for b in &s.outs {
                        inputs.insert(template_of(p.buffer(*b)), ());
                    }
                }
                StageKind::Store { .. } => {
                    for b in &s.ins {
                        outputs.insert(template_of(p.buffer(*b)), ());
                    }
                }
                _ => {}
            }
        }
        let mut templates: BTreeMap<BufferId, (&Buffer, Vec<Site>, usize)> = Default::default();
        for b in &p.buffers {
            let e = templates
                .entry(template_of(b))
                .or_insert((b, Vec::new(), 0));
            if let Some(site) = b.site {
                e.1.push(site);
            }
            e.2 = e.2.max(b.kind.page_tiles());
        }
        let (mut ni, mut no, mut nm) = (0u32, 0u32, 0u32);
        let mut slots = Vec::new();
        let mut by_template = BTreeMap::new();
        for (tid, (b, sites, page_tiles)) in &templates {
            data_format(b.dtype)?;
            let index = if matches!(b.kind, BufferKind::Interm { .. }) {
                nm += 1;
                24 + nm - 1
            } else if inputs.contains_key(tid) {
                ni += 1;
                ni - 1
            } else if outputs.contains_key(tid) {
                no += 1;
                16 + no - 1
            } else {
                return Err(SpatialLowerError::Unsupported(
                    "a buffer neither read by a mover nor written by one",
                ));
            };
            let max = if index < 16 { 8 } else { 8 };
            if (index < 16 && ni > max)
                || (16..24).contains(&index) && no > max
                || index >= 24 && nm > max
            {
                return Err(SpatialLowerError::TooBig {
                    what: "circular buffers",
                    need: (ni + no + nm) as usize,
                    have: 32,
                });
            }
            let depth_pages = match b.kind {
                BufferKind::Fifo { depth_pages, .. } | BufferKind::Interm { depth_pages, .. } => {
                    depth_pages
                }
                BufferKind::Accumulator { .. } => {
                    return Err(SpatialLowerError::Unsupported(
                        "an accumulator buffer in local memory",
                    ))
                }
            };
            by_template.insert(*tid, slots.len());
            let alias_of = b.alias_of.map(|a| template_of(p.buffer(a)));
            slots.push(CbSlot {
                template: *tid,
                name: template_name(p, b),
                index,
                dtype: b.dtype,
                page_tiles: *page_tiles,
                depth_pages,
                tile_bytes: target.tile_bytes(b.dtype),
                sites: sites.clone(),
                alias_of,
            });
        }
        let mut by_instance = BTreeMap::new();
        for b in &p.buffers {
            by_instance.insert(b.id, by_template[&template_of(b)]);
        }
        Ok(CbPlan {
            slots,
            by_template,
            by_instance,
        })
    }

    pub fn slot(&self, instance: BufferId) -> &CbSlot {
        &self.slots[self.by_instance[&instance]]
    }
}
