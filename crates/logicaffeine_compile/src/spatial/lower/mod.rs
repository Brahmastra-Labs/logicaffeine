//! The lowering seam: every backend is a pure function of a VERIFIED program and its target, and every
//! construct it emits is the image of an IR object (the census bijection).

pub mod cuda;
pub mod fabric;
pub mod fabric_board;
pub mod rdu;
pub mod rdu_lanes;
pub mod rdu_sdk;
pub mod sambaflow;
pub mod spatial_lang;
pub mod tensix;

use serde::{Deserialize, Serialize};

use super::certificate::ChoreographyCertificate;
use super::eval::ChoreoFault;
use super::ir::*;
use super::target::SpatialTarget;

/// What the IR contains, counted — the number every artifact must reproduce.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Census {
    /// Distinct buffer templates (one circular buffer / ring / FIFO instance kind each).
    pub buffer_templates: usize,
    pub fifos: usize,
    pub semaphores: usize,
    pub barrier_semaphores: usize,
    pub mcast_groups: usize,
    pub kernels: usize,
    pub sites: usize,
}

impl Census {
    /// The census of an emitted Verilog artifact. A buffer TEMPLATE and an engine CLASS are module
    /// definitions (printed once); a FIFO, a multicast bus and a site are instances (one per IR object); a
    /// semaphore is one IR object however many sites hold a copy, so its instances are counted by id.
    pub fn of_verilog(rtl: &str) -> Census {
        let count = |prefix: &str| {
            rtl.lines()
                .filter(|l| l.trim_start().starts_with(prefix))
                .count()
        };
        // A semaphore instance is `sem_prim sem_<id>_…`, or `barrier_prim sem_<id>_…` for a section rendezvous.
        let sem_ids: std::collections::BTreeSet<&str> = rtl
            .lines()
            .filter_map(|l| {
                let l = l.trim_start();
                l.strip_prefix("sem_prim sem_")
                    .or_else(|| l.strip_prefix("barrier_prim sem_"))
            })
            .filter_map(|r| r.split('_').next())
            .collect();
        Census {
            buffer_templates: count("module fifo_tpl_"),
            fifos: count("fifo_tpl_"),
            semaphores: sem_ids.len(),
            barrier_semaphores: count("barrier_prim "),
            mcast_groups: count("mcast_bus_prim_"),
            kernels: count("module engine_"),
            sites: count("site_"),
        }
    }

    pub fn of(p: &SpatialProgram) -> Census {
        let templates: std::collections::BTreeSet<_> =
            p.buffers.iter().map(|b| b.origin.unwrap_or(b.id)).collect();
        Census {
            buffer_templates: templates.len(),
            // An intermediate spill is a bounded buffer with the same flow control as any other — every
            // backend materialises one as a FIFO, so counting only `Fifo` here undercounts the hardware and
            // makes the bijection fail against a lowering that is doing the right thing.
            fifos: p
                .buffers
                .iter()
                .filter(|b| matches!(b.kind, BufferKind::Fifo { .. } | BufferKind::Interm { .. }))
                .count(),
            semaphores: p.semaphores.len(),
            barrier_semaphores: p
                .semaphores
                .iter()
                .filter(|s| matches!(s.role, SemRole::Barrier { .. }))
                .count(),
            mcast_groups: p.placement.as_ref().map(|pl| pl.groups.len()).unwrap_or(0),
            kernels: p.summary().engine_program_classes,
            sites: p.sites().len(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum SpatialLowerError {
    NotVerified {
        phase: Phase,
    },
    Unsupported(&'static str),
    GroupSpansSites {
        group: GroupId,
    },
    NotNearestNeighbour {
        buffer: BufferId,
    },
    TooBig {
        what: &'static str,
        need: usize,
        have: usize,
    },
    Precision {
        needed: crate::codegen_sva::fabric_target::Precision,
    },
    /// A fabric whose declared memories or arrays need more of a part's resource than it has, refused before a
    /// line of RTL is printed. Only raised when the target names a part ([`crate::spatial::target::SpatialTarget::part`]).
    OverCapacity(CapacityRefusal),
    Uncertified(ChoreoFault),
}

/// A resource a programmable part carries a bounded number of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FabricResource {
    UltraRam,
    BlockRam18,
    LutRam,
    Flops,
    Dsp,
}

impl FabricResource {
    pub fn label(self) -> &'static str {
        match self {
            FabricResource::UltraRam => "UltraRAM blocks",
            FabricResource::BlockRam18 => "18Kb block RAMs",
            FabricResource::LutRam => "LUT-RAM LUTs",
            FabricResource::Flops => "flip-flops",
            FabricResource::Dsp => "DSP slices",
        }
    }
}

/// One resource a fabric needs more of than its part has.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Over {
    pub resource: FabricResource,
    /// The fewest of the resource the declared memories and arrays can take (a lower bound).
    pub need: u64,
    /// What the part has.
    pub have: u64,
    /// The memories that need the most of it, by name and bits, largest first.
    pub largest: Vec<(String, u64)>,
}

/// Why a fabric does not fit its part, naming every resource it overruns.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapacityRefusal {
    pub part: &'static str,
    pub over: Vec<Over>,
}

/// The neutral (SDK-gated) artifact of the RDU lane.
#[derive(Clone, Debug)]
pub struct NeutralArtifact {
    pub json: String,
    pub pretty: String,
    pub reason: &'static str,
    pub certificate: ChoreographyCertificate,
    /// The certificate was rechecked against the program (it is the one the program carried); `false` when the
    /// backend derived it.
    pub rechecked: bool,
    pub census: Census,
}

#[derive(Clone, Debug)]
pub enum SpatialArtifact {
    Tensix(tensix::TensixArtifact),
    Verilog(fabric::VerilogArtifact),
    Cuda(cuda::CudaArtifact),
    SdkGated(NeutralArtifact),
}

pub trait SpatialBackend {
    fn name(&self) -> &str;
    fn target(&self) -> &SpatialTarget;
    fn lower(&self, p: &SpatialProgram) -> Result<SpatialArtifact, SpatialLowerError>;
}

/// The precondition every lowering enforces.
pub fn require_verified(p: &SpatialProgram) -> Result<(), SpatialLowerError> {
    if p.phase() != Phase::Verified {
        return Err(SpatialLowerError::NotVerified { phase: p.phase() });
    }
    Ok(())
}

/// The precondition of a printer that emits ONE kernel shape from `work.plan`: every section of the program
/// runs under that plan. A composition's `work.plan` is its last section's, so any section lowered under
/// another plan would be printed as the wrong kernel.
pub fn require_one_plan(p: &SpatialProgram) -> Result<(), SpatialLowerError> {
    if p.sections.iter().any(|s| s.plan != p.work.plan) {
        return Err(SpatialLowerError::Unsupported("a composition whose sections run under different plans — this printer emits one kernel from one plan"));
    }
    Ok(())
}
