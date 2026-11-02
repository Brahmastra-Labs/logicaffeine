//! fabric_target — the ADL spine: a `FabricTarget` descriptor + `place_with_target`.
//!
//! One certified lowering, retargeted across fabric shapes (the MLAR ask). A `FabricTarget` names the array
//! size, the ALU/functional-unit capability set, and the memory depth; `place_with_target` runs the
//! certified `route_loop_body` placer THROUGH that descriptor and pads onto the described array. Every
//! setting is either provably `≡VM` (certified by `fabric_target_lock`, reusing `verified_placer_lock`'s SAT)
//! or refused with a typed [`Unplaceable`] — a correct grid or a named refusal, never a silently-wrong chip.

use std::collections::BTreeSet;

use super::fabric_config::ConfigTable;
use super::fabric_grid::{pad_grid, route_loop_body, Grid};
use super::fabric_rtl::{decision_fabric_from_source, AluOp, FuncUnit, LoopCell, Src};
use super::simd::LaneTy;

/// Every combinational ALU op the fabric places (the `AluOp` universe).
pub const ALL_ALU_OPS: &[AluOp] = &[
    AluOp::Add,
    AluOp::Sub,
    AluOp::Mul,
    AluOp::And,
    AluOp::Or,
    AluOp::Xor,
    AluOp::Shl,
    AluOp::Shr,
    AluOp::UShr,
];

/// Every sequential functional unit the fabric places (the `FuncUnit` universe).
pub const ALL_FUNC_UNITS: &[FuncUnit] = &[
    FuncUnit::DivS,
    FuncUnit::ModS,
    FuncUnit::FloorDivS,
    FuncUnit::Pow,
    FuncUnit::MulHiU,
    FuncUnit::ApproxEqF,
    FuncUnit::FAddF,
    FuncUnit::FSubF,
    FuncUnit::FMulF,
    FuncUnit::FDivF,
    FuncUnit::FSqrtF,
    FuncUnit::FFloorF,
    FuncUnit::MulOvfS,
    FuncUnit::IntToF,
    FuncUnit::FLtF,
    FuncUnit::FGtF,
    FuncUnit::FLeF,
    FuncUnit::FGeF,
    FuncUnit::FEqF,
    FuncUnit::FNeF,
    FuncUnit::PopCount,
];

/// The GEMM dataflow a target realizes. `Reduction` is the universal baseline — the 1-D dot-product MAC lane
/// every fabric ships today; `WeightStationary` / `OutputStationary` are the 2-D systolic modes (Phase 1+).
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
pub enum Dataflow {
    /// 1-D unrolled dot-product → one accumulator (today's MAC lane). Every target has it.
    Reduction,
    /// 2-D systolic array: weights stationary per PE, activations streamed, partial sums accumulate spatially.
    WeightStationary,
    /// 2-D systolic array: outputs stationary per PE, weights + activations streamed.
    OutputStationary,
}

/// A numeric format a target's compute realizes (the precision dial, Phase 4).
///
/// `Int32` is the integer type a 32-bit datapath computes indices and addresses in — the SN40L's PCU
/// supports BF16, FP32, INT32 and INT8 and nothing wider, so a gathered row, an arg-max or a block start
/// lowered for an RDU rides `Int32`, never `Int64`.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
pub enum Precision {
    Int8,
    Int16,
    Int32,
    Int64,
    Fp8,
    Bf16,
    Fp16,
    Fp32,
}

/// A tier of the memory hierarchy a schedule demands (Phase 5).
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
pub enum MemTier {
    /// On-chip scratchpad SRAM (the PMU-equivalent).
    Sram,
    /// On-package HBM.
    Hbm,
    /// Off-package DDR.
    Ddr,
}

/// The on-/off-chip memory hierarchy a target provides, in the SN40L three-tier shape (Phase 5).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct MemHierarchy {
    pub sram_kib: usize,
    pub hbm_gib: usize,
    pub ddr_gib: usize,
}

impl MemHierarchy {
    /// No memory hierarchy at all — a pure combinational scalar array.
    pub const NONE: MemHierarchy = MemHierarchy {
        sram_kib: 0,
        hbm_gib: 0,
        ddr_gib: 0,
    };
}

/// A capability a program cell requires from a target — the unit of the ADL's typed refusal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Capability {
    /// A combinational ALU op (`LoopCell::Arith`).
    Alu(AluOp),
    /// A sequential functional unit (`LoopCell::Func` — division / float / popcount / …).
    Func(FuncUnit),
    /// The predicated-select mux (`LoopCell::Select`) — every fabric carries it.
    Select,
    /// Any state-plane access (`LoopCell::{MemRead,HeapRead,HeapWrite,HeapAlloc}`) — needs a heap.
    Memory,
    /// A packed-SIMD tile (`LoopCell::VectorArith`) of a given lane type — needs a matching `target.simd`.
    Simd(LaneTy),
    /// The GEMM dataflow a lowering demands (`Dataflow::Reduction` is the universal baseline).
    Dataflow(Dataflow),
    /// A numeric MAC format a lowering demands.
    Precision(Precision),
    /// A memory tier a schedule demands (on-chip SRAM / HBM / DDR).
    MemTier(MemTier),
    /// A fusion window a lowering demands — the target must fuse at least this many ops into one stage.
    Fusion(usize),
    /// A metapipeline depth a lowering demands — the target sustains at least this many pipelined stages of
    /// compute between buffers (Plasticine: the PCU's pipeline stages; a scalar array: 1).
    Metapipeline(usize),
    /// A scratchpad banking factor a lowering demands — the target's memory unit serves at least this many
    /// concurrent lane accesses per cycle (Plasticine PMU: 16 banks; a flat heap: 1).
    Banks(usize),
    /// A nested-counter depth a lowering demands — the loop-nest depth the target's control can drive without
    /// a CPU (Plasticine: the PCU's counter chain; at least 1 everywhere).
    CounterNest(usize),
}

/// Why a program cannot be placed on a given target — a TYPED refusal, never a silently-wrong grid.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Unplaceable {
    /// The target lacks a capability the program needs; `needed` names exactly what is missing.
    MissingCapability {
        needed: Capability,
        target: &'static str,
    },
    /// The placed dataflow is larger than the described array.
    TooSmall {
        needed_rows: usize,
        needed_cols: usize,
        target_rows: usize,
        target_cols: usize,
        target: &'static str,
    },
}

/// Real-silicon facts about a *physical* target that do not map onto the placement fields (which model the
/// per-datapath array). Present only for a target backed by a real chip; `None` for the abstract presets. Every
/// value must be a published spec cited at the constructor — this is descriptor metadata, not a claim we place.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SiliconSpec {
    /// Compute-tile count (e.g. SambaNova PCUs).
    pub compute_tiles: usize,
    /// Memory-tile count (e.g. SambaNova PMUs).
    pub mem_tiles: usize,
    /// Process node in nanometres.
    pub node_nm: u32,
    /// Published peak bf16 throughput in PFLOPS.
    pub peak_bf16_pflops: f64,
}

/// A described fabric — the ADL. One certified lowering (`place_with_target`) retargets across shapes by
/// varying this descriptor; each setting is either provably `≡VM` or refused with an [`Unplaceable`].
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct FabricTarget {
    /// Human name (carried into refusal errors).
    pub name: &'static str,
    /// Described array dimensions.
    pub rows: usize,
    pub cols: usize,
    /// Datapath bit width (metadata for the reconfig/SIMD proofs; the SAT proof width is separate).
    pub width: u32,
    /// The combinational ALU ops this target provides.
    pub alu_ops: BTreeSet<AluOp>,
    /// The sequential functional units this target provides.
    pub func_units: BTreeSet<FuncUnit>,
    /// Heap depth in words; `0` means no state plane (memory cells are refused).
    pub mem_depth: usize,
    /// Whether the target carries a decoupled address-generation unit (Phase 1).
    pub agu: bool,
    /// The packed-SIMD lane configuration this target provides, if any (Phase 2). `None` ⇒ scalar-only.
    pub simd: Option<LaneTy>,
    /// The GEMM dataflow modes this target realizes — always includes `Dataflow::Reduction` (the baseline).
    pub dataflows: BTreeSet<Dataflow>,
    /// The numeric MAC formats this target realizes (the precision dial).
    pub precisions: BTreeSet<Precision>,
    /// The memory hierarchy this target provides (on-chip SRAM / HBM / DDR).
    pub mem: MemHierarchy,
    /// How many ops the target fuses into one spatial pipeline stage (`1` = no cross-op fusion).
    pub fusion_window: usize,
    /// Real-chip metadata (compute/memory tiles, node, roofline) for a physically-backed target; `None` for the
    /// abstract presets. Cited to public sources at the constructor.
    pub silicon: Option<SiliconSpec>,
    /// Pipelined compute stages between buffers the target sustains (`1` = a single combinational stage).
    pub metapipeline_depth: usize,
    /// Scratchpad banks a memory unit serves concurrently (`1` = a flat single-port heap).
    pub banks: usize,
    /// Loop-nest depth the target's own control (counter chain) drives without a CPU (`1` = one loop).
    pub counter_nest: usize,
}

impl FabricTarget {
    /// The superset target: every ALU op + every functional unit + a heap. The "can place anything that fits"
    /// shape used to certify the placer without capability refusals getting in the way.
    pub fn full(rows: usize, cols: usize, width: u32) -> FabricTarget {
        FabricTarget {
            name: "full",
            rows,
            cols,
            width,
            alu_ops: ALL_ALU_OPS.iter().copied().collect(),
            func_units: ALL_FUNC_UNITS.iter().copied().collect(),
            mem_depth: 1 << 16,
            agu: true,
            simd: Some(LaneTy::I32x2),
            dataflows: [
                Dataflow::Reduction,
                Dataflow::WeightStationary,
                Dataflow::OutputStationary,
            ]
            .into_iter()
            .collect(),
            // Only WITNESSED precisions are advertised (int8/int16/int64 + bf16 + fp32 — each pinned by
            // `precision_gemm_lock`); Fp8/Fp16 stay enum variants but are unclaimed until their witness ships —
            // the precision ratchet stays honest.
            precisions: [
                Precision::Int8,
                Precision::Int16,
                Precision::Int64,
                Precision::Bf16,
                Precision::Fp32,
            ]
            .into_iter()
            .collect(),
            mem: MemHierarchy {
                sram_kib: 1024,
                hbm_gib: 64,
                ddr_gib: 1536,
            },
            fusion_window: 64,
            silicon: None,
            metapipeline_depth: 1,
            banks: 1,
            counter_nest: 1,
        }
    }

    /// An ALU-only target with exactly the given op set — no functional units, no heap. For refusal tests and
    /// minimal wearable-class arrays.
    pub fn with_alu_ops(
        name: &'static str,
        rows: usize,
        cols: usize,
        width: u32,
        ops: &[AluOp],
    ) -> FabricTarget {
        FabricTarget {
            name,
            rows,
            cols,
            width,
            alu_ops: ops.iter().copied().collect(),
            func_units: BTreeSet::new(),
            mem_depth: 0,
            agu: false,
            simd: None,
            dataflows: [Dataflow::Reduction].into_iter().collect(),
            precisions: [Precision::Int64].into_iter().collect(),
            mem: MemHierarchy::NONE,
            fusion_window: 1,
            silicon: None,
            metapipeline_depth: 1,
            banks: 1,
            counter_nest: 1,
        }
    }

    /// TINY — a pure combinational scalar array: full ALU, no functional units, no state plane. The
    /// smallest useful knob (branchless arithmetic kernels only).
    pub fn tiny() -> FabricTarget {
        FabricTarget {
            name: "tiny",
            rows: 8,
            cols: 8,
            width: 64,
            alu_ops: ALL_ALU_OPS.iter().copied().collect(),
            func_units: BTreeSet::new(),
            mem_depth: 0,
            agu: false,
            simd: None,
            dataflows: [Dataflow::Reduction].into_iter().collect(),
            precisions: [Precision::Int64].into_iter().collect(),
            mem: MemHierarchy::NONE,
            fusion_window: 1,
            silicon: None,
            metapipeline_depth: 1,
            banks: 1,
            counter_nest: 1,
        }
    }

    /// WEARABLE — the realistic edge target: full ALU + every functional unit (division + the float lane) +
    /// a heap + a decoupled AGU. The HiVec-class knob these levers were built for.
    pub fn wearable() -> FabricTarget {
        FabricTarget {
            name: "wearable",
            rows: 16,
            cols: 16,
            width: 64,
            alu_ops: ALL_ALU_OPS.iter().copied().collect(),
            func_units: ALL_FUNC_UNITS.iter().copied().collect(),
            mem_depth: 1 << 16,
            agu: true,
            simd: Some(LaneTy::I32x2),
            dataflows: [Dataflow::Reduction].into_iter().collect(),
            precisions: [Precision::Int8, Precision::Int16, Precision::Int64]
                .into_iter()
                .collect(),
            mem: MemHierarchy {
                sram_kib: 64,
                hbm_gib: 0,
                ddr_gib: 0,
            },
            fusion_window: 1,
            silicon: None,
            metapipeline_depth: 1,
            banks: 1,
            counter_nest: 1,
        }
    }

    /// WIDE — the superset knob: everything the fabric can place, on a large array. Nothing is refused for
    /// lack of capability (only for lack of room).
    pub fn wide() -> FabricTarget {
        FabricTarget {
            name: "wide",
            ..FabricTarget::full(32, 32, 64)
        }
    }

    /// RDU-SHAPED — a descriptor modeling the SambaNova **SN40L** RDU, from published specs. The PCU is
    /// configurable as a weight-stationary / output-stationary systolic GEMM OR a SIMD-elementwise pipeline
    /// (both dataflows + reduction), fed by a three-tier SRAM/HBM/DDR hierarchy (PMU scratchpad + package
    /// memory) with deep operator fusion. Our certified lowering places onto THIS shape — the pitch target.
    ///
    /// Public SN40L specs (each pinned by `sn40l_target_lock`):
    /// - **1040 PCUs + 1040 PMUs**, **TSMC 5 nm**, **10.2 bf16 PFLOPS** roofline → [`SiliconSpec`].
    /// - Three-tier memory: **520 MiB** on-chip SRAM, **64 GiB** co-packaged HBM, **1.5 TiB** DDR.
    /// - Datatypes BF16 / FP32 / INT32 / INT8; we advertise only the precisions our fabric places AND witnesses
    ///   (Int8 + Bf16). FP32/INT32 are chip-supported but witness-pending — NOT gated as available, to keep the
    ///   precision ratchet honest.
    /// Sources: SambaNova Hot Chips 2024 (hc2024.hotchips.org) · arXiv 2405.07518 · ServeTheHome · sambanova.ai.
    ///
    /// `(rows, cols)` models the placement envelope of ONE datapath, not the 1040-PCU mesh (that is
    /// `silicon.compute_tiles`); the per-PCU systolic dimension is not public.
    ///
    /// Derived from [`RduChipFacts::sn40l`](crate::spatial::rdu_chip::RduChipFacts::sn40l): the published
    /// numbers above live there with their citations, and the per-PCU pipeline depth / PMU banking /
    /// counter chain — not public — come from the facts' unit model, labeled modeled and cited to the open
    /// RDU (Plasticine, ISCA 2017) they are inherited from.
    pub fn rdu_shaped() -> FabricTarget {
        crate::spatial::rdu_chip::RduChipFacts::sn40l().fabric_target()
    }

    /// SN50-SHAPED — the SambaNova **SN50** RDU, derived from
    /// [`RduChipFacts::sn50`](crate::spatial::rdu_chip::RduChipFacts::sn50): the same PCU capability
    /// envelope as the SN40L (systolic + SIMD dataflows, the three memory tiers, a decoupled AGU, deep
    /// fusion), with the SN50's published memory — **432 MB** on-chip SRAM, **64 GB** HBM2E, DDR5 up to
    /// **512 GB** — and a **1.6 bf16 PFLOPS** per-RDU roofline at **TSMC 5 nm**. The PCU/PMU counts are not
    /// public and are inherited from the SN40L (modeled, so labeled on the mesh descriptor). Only the
    /// witnessed precisions (int8 + bf16) are advertised; the chip's FP8 and FP32 are witness-pending and
    /// refused by name until their witness ships.
    pub fn sn50_shaped() -> FabricTarget {
        crate::spatial::rdu_chip::RduChipFacts::sn50().fabric_target()
    }

    /// PLASTICINE — the open RDU: Prabhakar et al., *Plasticine: A Reconfigurable Architecture for Parallel
    /// Patterns*, ISCA 2017 (DOI 10.1145/3079856.3080256). A 16×8 checkerboard of **64 PCUs + 64 PMUs** at
    /// **28 nm** (112.8 mm², 49 W, 1 GHz, 12.3 fp32 TFLOPS, 16 MB scratchpad). A PCU is a **6-stage × 16-lane
    /// SIMD pipeline of 32-bit FUs** (integer + fp32) with a cross-lane reduction tree and a counter chain; a
    /// PMU is a **16-bank × 16 KB = 256 KB** scratchpad with an address-calculation datapath (strided / FIFO /
    /// line-buffer / duplication banking, N-buffering). Interconnect: statically configured scalar / vector /
    /// control networks. Off-chip: 4 DDR3-1600 channels, 51.2 GB/s (bandwidth on the mesh descriptor,
    /// [`super::plasticine::PlasticineTarget`]; DRAM capacity is unpublished, so no `ddr_gib` is claimed).
    ///
    /// Capability envelope, honestly: the PCU realizes `Reduction` (SIMD lanes + tree) — it is NOT a systolic
    /// array, so weight-/output-stationary modes are refused; the FUs are 32-bit, so the only advertised
    /// precision is the witnessed **fp32** (no int8/bf16 MAC exists). `(rows, cols)` = one PCU datapath
    /// envelope (stages × lanes); the mesh lives on `silicon` + the mesh descriptor. Peak bf16 is 0: the
    /// chip has no bf16 datapath.
    pub fn plasticine_shaped() -> FabricTarget {
        FabricTarget {
            name: "plasticine-isca17",
            rows: 6,
            cols: 16,
            width: 32,
            alu_ops: ALL_ALU_OPS.iter().copied().collect(),
            func_units: ALL_FUNC_UNITS.iter().copied().collect(),
            mem_depth: 256 * 1024 / 4, // one PMU: 256 KB of 32-bit words
            agu: true,                 // the PMU's address-calculation datapath
            simd: Some(LaneTy::I32x2), // 16-lane 32-bit SIMD (our packed lane type models a 32-bit lane)
            dataflows: [Dataflow::Reduction].into_iter().collect(),
            precisions: [Precision::Fp32].into_iter().collect(),
            mem: MemHierarchy {
                sram_kib: 16 * 1024,
                hbm_gib: 0,
                ddr_gib: 0,
            },
            fusion_window: 6, // up to six chained FU stages fuse inside one PCU
            silicon: Some(SiliconSpec {
                compute_tiles: 64,
                mem_tiles: 64,
                node_nm: 28,
                peak_bf16_pflops: 0.0,
            }),
            metapipeline_depth: 6,
            banks: 16,
            counter_nest: 1, // the counter chain's depth is not tabulated; one counter is certain
        }
    }

    /// Whether this target provides a required capability.
    pub fn provides(&self, cap: Capability) -> bool {
        match cap {
            Capability::Alu(op) => self.alu_ops.contains(&op),
            Capability::Func(u) => self.func_units.contains(&u),
            Capability::Select => true,
            Capability::Memory => self.mem_depth > 0,
            Capability::Simd(ty) => self.simd == Some(ty),
            Capability::Dataflow(d) => self.dataflows.contains(&d),
            Capability::Precision(p) => self.precisions.contains(&p),
            Capability::MemTier(t) => match t {
                MemTier::Sram => self.mem.sram_kib > 0,
                MemTier::Hbm => self.mem.hbm_gib > 0,
                MemTier::Ddr => self.mem.ddr_gib > 0,
            },
            Capability::Fusion(n) => self.fusion_window >= n,
            Capability::Metapipeline(n) => self.metapipeline_depth >= n,
            Capability::Banks(n) => self.banks >= n,
            Capability::CounterNest(n) => self.counter_nest >= n,
        }
    }

    /// The capability half of [`place_with_target`]: every cell's required capability is provided, or
    /// the FIRST missing one is named. A program larger than one datapath envelope (a whole decision
    /// program spanning many PCUs) is gated here cell by cell; its geometry is the mesh's question.
    pub fn require_cells(&self, cells: &[LoopCell]) -> Result<(), Unplaceable> {
        for cell in cells {
            let cap = required_capability(cell);
            if !self.provides(cap) {
                return Err(Unplaceable::MissingCapability {
                    needed: cap,
                    target: self.name,
                });
            }
        }
        Ok(())
    }

    /// Gate a requested `(dataflow, precision)` GEMM against this target — the check Phase-1+ lowerings call
    /// before placing a systolic/precision datapath. Returns a typed [`Unplaceable::MissingCapability`] naming
    /// the FIRST dial the target lacks (dataflow before precision), or `Ok(())` if both are provided.
    pub fn require_gemm(
        &self,
        dataflow: Dataflow,
        precision: Precision,
    ) -> Result<(), Unplaceable> {
        for cap in [
            Capability::Dataflow(dataflow),
            Capability::Precision(precision),
        ] {
            if !self.provides(cap) {
                return Err(Unplaceable::MissingCapability {
                    needed: cap,
                    target: self.name,
                });
            }
        }
        Ok(())
    }
}

/// The capability a program cell demands of a target. Exhaustive over `LoopCell`, so a new cell kind forces a
/// deliberate capability decision here rather than silently placing on an incapable target.
fn required_capability(cell: &LoopCell) -> Capability {
    match cell {
        LoopCell::Arith { op, .. } => Capability::Alu(*op),
        LoopCell::Func { unit, .. } => Capability::Func(*unit),
        LoopCell::VectorArith { ty, .. } => Capability::Simd(*ty),
        LoopCell::Select { .. } => Capability::Select,
        LoopCell::MemRead { .. }
        | LoopCell::HeapRead { .. }
        | LoopCell::HeapWrite { .. }
        | LoopCell::HeapAlloc { .. } => Capability::Memory,
    }
}

/// Place a dataflow body onto a DESCRIBED target: gate every cell against the target's capability set, place
/// with the certified `route_loop_body`, gate the placed shape against the described array, then pad onto it.
/// Returns the placed+padded [`Grid`], or a typed [`Unplaceable`] — a correct grid or a named refusal.
pub fn place_with_target(
    cells: &[LoopCell],
    n_in: usize,
    next: &[Src],
    target: &FabricTarget,
) -> Result<Grid, Unplaceable> {
    target.require_cells(cells)?;
    let g = route_loop_body(cells, n_in, next);
    if g.rows > target.rows || g.cols > target.cols {
        return Err(Unplaceable::TooSmall {
            needed_rows: g.rows,
            needed_cols: g.cols,
            target_rows: target.rows,
            target_cols: target.cols,
            target: target.name,
        });
    }
    Ok(pad_grid(&g, target.rows, target.cols))
}

/// Compile a whole program corpus onto ONE described array: if-convert each `(source, function)` to a decision
/// island and place it THROUGH `target`, so every island shares the described shape. The resulting
/// [`ConfigTable`] is certifiable by [`super::fabric_config::certify_config_table`] — `config == k ⇒ the
/// described array ≡ island k`, for ALL inputs — the ADL's retarget-a-corpus-onto-a-chosen-shape payoff.
/// Returns a typed [`Unplaceable`] if any datapath needs a capability the target lacks or does not fit.
pub fn compile_to_config_table_on_target(
    sources: &[(&str, &str)],
    target: &FabricTarget,
    width: u32,
    cfg_width: u32,
) -> Result<ConfigTable, Unplaceable> {
    assert!(
        !sources.is_empty(),
        "a config table needs at least one datapath"
    );
    assert!(
        (1usize << cfg_width) >= sources.len(),
        "cfg_width {cfg_width} cannot index {} datapaths",
        sources.len()
    );
    let mut grids = Vec::with_capacity(sources.len());
    for (src, name) in sources {
        let df = decision_fabric_from_source(src, name);
        grids.push(place_with_target(&df.cells, df.n_in, &[df.result], target)?);
    }
    Ok(ConfigTable {
        grids,
        rows: target.rows,
        cols: target.cols,
        width,
        cfg_width,
    })
}
