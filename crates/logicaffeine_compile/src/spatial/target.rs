//! `SpatialTarget` — the described MESH a spatial program is placed onto.
//!
//! One descriptor per chip class: how many sites, what each site holds (local memory, accumulator tiles,
//! movement engines, NOCs), the native compute tile, which interconnect and handshake primitives exist, and
//! which compute capabilities (tile matmul, elementwise, row-reduce, a scalar datapath) the site offers. The
//! passes read ONLY this descriptor; every number is cited at its constructor, and the fields that a public
//! datasheet does not give are listed in `modeled` so the descriptor never pretends to know them.
//!
//! The scalar datapath capability EMBEDS the existing [`FabricTarget`] descriptor — the fabric lane is a
//! consumer of the spatial IR, not a sibling of it.

use std::collections::BTreeSet;

use serde::Serialize;

use super::cost::CycleModel;
use super::ir::AccPolicy;
use super::part::{MemoryKind, PartResources};
use super::roofs::Roofs;
use crate::codegen_sva::fabric_target::{FabricTarget, MemHierarchy, Precision};

/// The mesh of sites: `x` columns × `y` rows.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, serde::Deserialize,
)]
pub struct MeshShape {
    pub x: usize,
    pub y: usize,
}

impl MeshShape {
    pub fn sites(&self) -> usize {
        self.x * self.y
    }
}

/// Geometry of one native compute tile — 32×32 on Tensix, a 16×16 fragment on a tensor core, 1×1 for a
/// scalar fabric cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, serde::Deserialize)]
pub struct TileShape {
    pub rows: usize,
    pub cols: usize,
}

impl TileShape {
    pub const TENSIX: TileShape = TileShape { rows: 32, cols: 32 };
    pub const WMMA16: TileShape = TileShape { rows: 16, cols: 16 };
    pub const SCALAR: TileShape = TileShape { rows: 1, cols: 1 };

    pub fn elems(&self) -> usize {
        self.rows * self.cols
    }
}

/// How sites exchange data.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, serde::Deserialize,
)]
pub enum Interconnect {
    /// One site writes a block into a rectangle of sites in one NOC transaction (Tensix multicast).
    MulticastRect,
    /// Wires between adjacent sites only (our fabric).
    NearestNeighbour,
    /// Any site to any site (the RDU's switch fabric).
    Crossbar,
    /// Sites share data only through the global memory (a GPU's L2/DRAM).
    GlobalMemory,
    /// Threads inside ONE site share a scratchpad (a thread block's shared memory).
    SharedMemoryBlock,
}

/// The synchronization primitive an edge may use.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, serde::Deserialize,
)]
pub enum HandshakeKind {
    Fifo,
    Semaphore,
    Barrier,
    ValidReady,
    Token,
}

/// Matmul math fidelity of a tile engine.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, serde::Deserialize,
)]
pub enum Fidelity {
    LoFi,
    HiFi2,
    HiFi3,
    HiFi4,
    Exact,
}

impl Fidelity {
    /// The identifier tt-metal's `MathFidelity::` enum uses — also the token on the `LOGOS_MM_PASS` line
    /// and the value of `LOGOS_TT_FIDELITY`.
    pub fn as_str(self) -> &'static str {
        match self {
            Fidelity::LoFi => "LoFi",
            Fidelity::HiFi2 => "HiFi2",
            Fidelity::HiFi3 => "HiFi3",
            Fidelity::HiFi4 => "HiFi4",
            Fidelity::Exact => "Exact",
        }
    }
    /// Parse that identifier.
    pub fn parse(s: &str) -> Option<Fidelity> {
        match s {
            "LoFi" => Some(Fidelity::LoFi),
            "HiFi2" => Some(Fidelity::HiFi2),
            "HiFi3" => Some(Fidelity::HiFi3),
            "HiFi4" => Some(Fidelity::HiFi4),
            "Exact" => Some(Fidelity::Exact),
            _ => None,
        }
    }
}

/// The three numerics knobs the measurement harness varies, and the PCC the program then requires of
/// itself. `scripts/tt-matmul-measure.sh` sets them as `LOGOS_TT_FIDELITY` / `_FP32` / `_L1ACC`, and every
/// ledger row records what the program PRINTED rather than what was asked for — so this parser is part of
/// the measurement contract, not a convenience.
///
/// ★ SALVAGED FROM THE RETIRED EMITTER, and narrowed on the way in. It used to carry its own math-fidelity
/// enum beside this crate's [`Fidelity`] — two spellings of one concept, each with its own `parse`/`as_str`
/// — and a register-budget helper that halved the tile count under fp32 accumulate while taking a SECOND
/// chip descriptor to do it. [`SpatialTarget::effective_acc_tiles`] already expresses that exactly, so the
/// dependency on a duplicate description of the part dissolved rather than moved.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NumericsKnobs {
    pub fidelity: Fidelity,
    /// Accumulate in fp32 in the register file: halves the tile budget, raises PCC.
    pub fp32_dest_acc: bool,
    /// Accumulate K-block partials in local memory through the packer instead of reloading registers.
    pub packer_l1_acc: bool,
    /// The PCC the emitted program requires of itself before it prints `LOGOS_MM_PASS`.
    pub pcc_floor: f64,
}

impl Default for NumericsKnobs {
    /// ★ ttnn's OWN bf16 default numerics — HiFi2, bf16 dest-accumulate — so a head-to-head against the
    /// vendor is matched by construction rather than by remembering to pass a flag. Defaulting to HiFi4
    /// would silently compare our four-pass arithmetic against the vendor's two-pass and call the
    /// difference a compiler result.
    fn default() -> Self {
        NumericsKnobs {
            fidelity: Fidelity::HiFi2,
            fp32_dest_acc: false,
            packer_l1_acc: false,
            pcc_floor: 0.99,
        }
    }
}

impl NumericsKnobs {
    /// Read the harness's environment contract.
    pub fn from_env() -> Result<Self, String> {
        let get = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
        Self::from_vars(
            get("LOGOS_TT_FIDELITY").as_deref(),
            get("LOGOS_TT_FP32").as_deref(),
            get("LOGOS_TT_L1ACC").as_deref(),
        )
    }

    /// The pure core of [`NumericsKnobs::from_env`] — every refusal is typed and names the variable, so a
    /// mistyped knob stops the measurement instead of silently recording a different program.
    pub fn from_vars(
        fidelity: Option<&str>,
        fp32: Option<&str>,
        l1acc: Option<&str>,
    ) -> Result<Self, String> {
        let mut k = NumericsKnobs::default();
        if let Some(f) = fidelity {
            k.fidelity = Fidelity::parse(f.trim()).ok_or_else(|| {
                format!("LOGOS_TT_FIDELITY must be LoFi|HiFi2|HiFi3|HiFi4, got {f:?}")
            })?;
        }
        let flag = |name: &str, v: Option<&str>| -> Result<bool, String> {
            match v.map(str::trim) {
                None | Some("0") => Ok(false),
                Some("1") => Ok(true),
                Some(other) => Err(format!("{name} must be 0|1, got {other:?}")),
            }
        };
        k.fp32_dest_acc = flag("LOGOS_TT_FP32", fp32)?;
        k.packer_l1_acc = flag("LOGOS_TT_L1ACC", l1acc)?;
        Ok(k)
    }

    /// The numerics policy these knobs express: fp32 accumulate is what keeps partial sums exact.
    pub fn policy(&self) -> crate::spatial::ir::NumericsPolicy {
        if self.fp32_dest_acc {
            crate::spatial::ir::NumericsPolicy::Exact
        } else {
            crate::spatial::ir::NumericsPolicy::AllowSpillRounding
        }
    }
}

/// Which operand stays resident in the compute engine while the other streams.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, serde::Deserialize,
)]
pub enum DataflowPolicy {
    OutputStationary,
    WeightStationary,
}

/// How a matrix engine's sub-block API runs one compute op over an `osh × osw` sub-block and its `ibw` inner
/// steps: the calls it makes and the operand tiles it loads.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
pub enum BlockApi {
    /// One call per inner step. The unpacker holds one operand tile along the sub-block's shorter side and
    /// re-loads the other operand's tiles for each: `min(osh, osw) + osh · osw` tiles a step. Tensix's
    /// `matmul_block` (`llk_unpack_AB_matmul`: `reuse_a = ct_dim >= rt_dim`, `t_dim` held tiles, each running
    /// the other operand's row of the block).
    Unpacker,
    /// One call over the whole inner block, its A and B panels loaded once: `osh · ibw + ibw · osw` tiles. The
    /// fabric's systolic array, which pays its fill once per call.
    Systolic,
    /// One fragment step per inner step. Each lane of the accumulator grid
    /// ([`SpatialTarget::accumulator_lane_grid`]) loads its own A rows and B columns: `wn · osh + wm · osw` tiles
    /// a step on a `wm × wn` grid, and two per output tile where no grid splits the sub-block evenly. The GPU's
    /// `wmma` loop.
    Fragments,
}

/// Whether a matrix engine's packer can ADD into a local-memory page instead of overwriting it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
pub enum PackerAccumulate {
    /// The packer only overwrites.
    No,
    /// The packer adds into a local-memory page, so a blocked reduction's partial sum stays in local memory
    /// between K blocks instead of round-tripping through the register file each block.
    InLocalMemory,
    /// …and the page it adds into can be the OUTPUT's: the intermediate and the output share one local region
    /// (two indices of one tt-metal `CircularBufferConfig`, Tensix `pack_reconfig_l1_acc`), so a reduction
    /// whose partial already is the result finishes in the page the writer stores, with no reload.
    IntoTheOutput,
}

/// A compute capability a site offers.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub enum ComputeCap {
    MatmulTile {
        precisions: BTreeSet<Precision>,
        fidelity: Fidelity,
        fp32_acc: bool,
        dataflows: BTreeSet<DataflowPolicy>,
        /// The engine accepts a whole sub-block per call (Tensix `matmul_block`), by this API; `None` takes one
        /// tile pair per call, two loads.
        block_api: Option<BlockApi>,
        /// Accumulator width of the integer path (64 on the exact fabrics, 32 on a tensor core).
        acc_int_bits: u32,
        /// The largest PE array the engine builds, `(contraction rows, output columns)`: a matmul's block is cut to it
        /// and its contraction runs across several passes. `None` where each matmul's block sets the array (a fabric
        /// sized per program) or the engine's tile does (Tensix, a tensor core).
        max_array: Option<(usize, usize)>,
        /// Whether the packer can ADD into a local-memory page instead of overwriting it, and into which.
        packer_accumulate: PackerAccumulate,
        /// A spilled partial in the accumulator's own width can be reloaded into the register file. On
        /// Wormhole and Blackhole under this tt-metal build the simulator refuses the Float32 unpack that
        /// reload needs (`tensix_unpacr: unpack_to_dst=0 in_data_format=0 out_data_format=0`) even with the
        /// buffer marked `UnpackToDestFp32`, so an EXACT blocked plan may not spill there — it keeps its
        /// partials in the register file (one K block) or streams.
        wide_spill_reload: bool,
        /// ★ THE ACCUMULATOR LIVES IN THE COMPUTE ENGINE'S OWN REGISTERS and survives the WHOLE
        /// reduction, so a blocked plan never spills a partial at all.
        ///
        /// This is the GPU: a `wmma` fragment is register-resident and the K loop runs inside the
        /// accumulate region, which is exactly what the CUDA lowering emits (and why it refuses an
        /// accumulator spill outright — "the fragments never leave the registers").
        ///
        /// Tensix is the other case: DST is a shared register file that must be DRAINED per
        /// sub-block, so a multi-block reduction spills its partial to local memory and reloads
        /// it. Conflating the two made every blocked GPU plan illegal under exact numerics — the
        /// spill it was refused for is one it would never perform — leaving only the streaming
        /// family, which computes ONE output tile per item and measured 214 GOPS against the knob
        /// lattice's 16,591 at 2048³.
        acc_in_registers: bool,
    },
    Elementwise {
        precisions: BTreeSet<Precision>,
    },
    RowReduce {
        precisions: BTreeSet<Precision>,
    },
    /// A per-site scalar datapath — the CGRA descriptor, embedded.
    Scalar(FabricTarget),
}

/// The latencies a bounded buffer's credit loop is sized against, in cycles.
///
/// These are ARCHITECTURAL facts about a part, not fitted ones, and the distinction is the whole point of the
/// type. A FIFO depth is a resource decision — too shallow stalls the producer every round, too deep spends
/// local memory another buffer needed — and deriving it from a regression against a cycle counter makes it
/// only as trustworthy as that counter. Ours turned out not to track work at all, which would have made every
/// depth a function of noise. A target that cannot cite these numbers declares `None` and gets the vendor's
/// double-buffering discipline instead of an invented rate.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct CreditLatency {
    /// Cycles to fetch one tile into local memory.
    pub fetch_tile: u32,
    /// Cycles for a consumer's credit to travel back to the producer.
    pub credit_return: u32,
    /// Cycles the consumer spends draining one page.
    pub service_page: u32,
}

/// What one site holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct SiteSpec {
    /// Local memory budget for buffers (L1 / shared memory / BRAM), in bytes.
    pub local_mem_bytes: usize,
    /// A larger budget available on explicit opt-in (a GPU's dynamic shared memory), if any.
    pub local_mem_opt_in_bytes: Option<usize>,
    /// Accumulator capacity in native tiles (DST register file / register fragments / systolic columns).
    pub acc_tiles: usize,
    /// Accumulating in fp32 halves the accumulator capacity (Tensix DST).
    pub fp32_acc_halves_acc_tiles: bool,
    /// Independent data-movement engines per site (Tensix: RISCV_0 + RISCV_1).
    pub movement_engines: u8,
    /// Network-on-chip channels a site can drive.
    pub nocs: u8,
    /// Declared credit-loop latencies, when the part's are known. `None` means undeclared, NOT zero.
    pub credit: Option<CreditLatency>,
    /// Cycles from a read address to its data in the site's local memory, where the lowering schedules each read
    /// itself: `Some(0)` for a flop array read combinationally, `Some(1)` for a synchronous SRAM or block RAM. `None`
    /// where the hardware sequences its memory reads and the lowering never sees their timing.
    pub local_mem_read_latency: Option<u32>,
    /// The banks the site's local memory interleaves its elements over, element `e` in bank `e mod banks`: a power of
    /// two, the consecutive elements one cycle reads or writes wherever the run starts. 1 where a memory serves an
    /// element a cycle, or the hardware sequences its own accesses.
    pub mem_banks: usize,
    /// The lanes of the site's vector unit: the widest run of consecutive coordinates its Generic unit takes a
    /// point, a power of two no wider than the banks, sized to the part as its PE array is — each lane is a copy of
    /// every body the unit computes. 1 where the site has no vector unit.
    #[serde(default = "one_lane")]
    pub vector_lanes: usize,
    /// The one integer width the site's elementwise datapath computes in, where an integer it stores is the integer it
    /// computes and nothing narrower widens on its way in: an RDU's PCU computes integers in its index datatype
    /// (`RduChipFacts::index_datatype`, INT32). `None` where a narrower stored integer widens as it is read — a GPU's
    /// registers, the fabric's sign extension — so a buffer may be stored at any integer the datapath lists.
    #[serde(default)]
    pub int_compute: Option<Precision>,
    /// The widest signed product one multiplier slice of the site takes, `(wide, narrow)` operand bits: a 7-series
    /// DSP48E1's 25 × 18. Synthesis splits a wider product over a cascade of slices. `None` where the site's multiplies
    /// are no fixed slices — standard cells, a GPU's units.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub multiplier: Option<(u32, u32)>,
}

fn one_lane() -> usize {
    1
}

/// The NOC-direction convention a mesh imposes on multicast roles — measured, not chosen: on Wormhole the
/// in0 senders must multicast on NOC 0 and the receivers listen on NOC 1 (all-on-one-NOC inverted the
/// multicast rectangle under ttsim); the writer takes the complement, except the first column.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct NocRule {
    pub sender_noc: u8,
    pub receiver_noc: u8,
    pub writer_noc_default: u8,
    pub writer_noc_first_column: u8,
}

/// Whether NOC writes retire in issue order (Wormhole) or need an explicit flush before a dependent
/// semaphore multicast (Blackhole).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, serde::Deserialize)]
pub enum WriteOrdering {
    Ordered,
    NeedsFlush,
}

/// The described mesh.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SpatialTarget {
    pub name: &'static str,
    pub sites: MeshShape,
    pub site: SiteSpec,
    pub tile: TileShape,
    pub interconnect: BTreeSet<Interconnect>,
    pub handshakes: BTreeSet<HandshakeKind>,
    pub compute: Vec<ComputeCap>,
    pub dram: MemHierarchy,
    pub precisions: BTreeSet<Precision>,
    pub noc_rule: Option<NocRule>,
    pub write_ordering: WriteOrdering,
    pub cycle: CycleModel,
    /// Fields whose values are MODELED (no public datasheet number) — pinned by the target lock so the
    /// descriptor never overstates what it knows.
    pub modeled: &'static [&'static str],
    /// The rates the chip sustains, each cited: what a program's work is held against.
    pub roofs: Roofs,
    /// The programmable part this fabric is printed onto, when it is one: what the lowering spends as a resource
    /// budget. `None` for an abstract target (a GPU roofline, an RDU, the sky fabric) that no budget bounds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<PartResources>,
}

/// Bytes per element of a precision.
pub fn precision_bytes(p: Precision) -> usize {
    match p {
        Precision::Int8 | Precision::Fp8 => 1,
        Precision::Int16 | Precision::Bf16 | Precision::Fp16 => 2,
        Precision::Int32 | Precision::Fp32 => 4,
        Precision::Int64 => 8,
    }
}

/// Whether a precision is an integer format.
pub fn precision_is_int(p: Precision) -> bool {
    matches!(
        p,
        Precision::Int8 | Precision::Int16 | Precision::Int32 | Precision::Int64
    )
}

/// Bit width of an integer precision (the operand truncation the systolic reference applies).
pub fn precision_int_bits(p: Precision) -> Option<u32> {
    match p {
        Precision::Int8 => Some(8),
        Precision::Int16 => Some(16),
        Precision::Int32 => Some(32),
        Precision::Int64 => Some(64),
        _ => None,
    }
}

/// The signed integer precision of exactly `bits` bits, if the dial has one.
pub fn int_precision_of_bits(bits: u32) -> Option<Precision> {
    match bits {
        8 => Some(Precision::Int8),
        16 => Some(Precision::Int16),
        32 => Some(Precision::Int32),
        64 => Some(Precision::Int64),
        _ => None,
    }
}

/// The largest value a signed integer precision holds.
pub fn int_precision_max(p: Precision) -> Option<i64> {
    precision_int_bits(p).map(|b| {
        if b >= 64 {
            i64::MAX
        } else {
            (1i64 << (b - 1)) - 1
        }
    })
}

impl SpatialTarget {
    /// Tenstorrent Wormhole (n150/n300): 8×8 worker grid, 1464 KiB L1 per Tensix, an 8-tile DST that halves
    /// under fp32 accumulate, two data-movement RISC-Vs, two NOCs, rectangle multicast. The matmul FPU is
    /// float-primary — `Int8` is deliberately ABSENT from the tile precisions (int8 `matmul_tiles` measured
    /// 0/4096 correct under ttsim on both Wormhole and Blackhole); int8 GEMMs run as bf16 + SFPU requant.
    pub fn wormhole() -> SpatialTarget {
        SpatialTarget {
            name: "wormhole_b0",
            sites: MeshShape { x: 8, y: 8 },
            site: SiteSpec {
                local_mem_bytes: 1464 * 1024,
                local_mem_opt_in_bytes: None,
                acc_tiles: 8,
                fp32_acc_halves_acc_tiles: true,
                movement_engines: 2,
                nocs: 2,
                credit: None,
                local_mem_read_latency: None,
                mem_banks: 1,
                vector_lanes: 1,
                int_compute: None,
                multiplier: None,
            },
            tile: TileShape::TENSIX,
            interconnect: [Interconnect::MulticastRect, Interconnect::GlobalMemory]
                .into_iter()
                .collect(),
            handshakes: [HandshakeKind::Fifo, HandshakeKind::Semaphore]
                .into_iter()
                .collect(),
            compute: vec![
                ComputeCap::MatmulTile {
                    precisions: [Precision::Bf16, Precision::Fp16].into_iter().collect(),
                    fidelity: Fidelity::HiFi4,
                    fp32_acc: true,
                    dataflows: [DataflowPolicy::OutputStationary].into_iter().collect(),
                    block_api: Some(BlockApi::Unpacker),
                    // ★ IT CANNOT RELOAD A WIDE SPILL, and reconfiguring the unpacker does not change
                    //   that. RE-MEASURED 2026-09-10, and the measurement is the whole point.
                    //
                    // This was flipped to `true` on the reasoning that the garbage reload (PCC 0.165 at
                    // 640³) was a property of the LOWERING rather than the silicon, since the reload had
                    // learned to emit `copy_tile_to_dst_init_short_with_dt` and to put source A back with
                    // `reconfig_data_format_srca`. The lowering does emit both — and the device still
                    // refuses. Running the exact-numerics 640³ plan the flip unlocked:
                    //
                    //     LOGOS_MM_CFG strategy=blocked-split grid=5x5 per_core=4x4 in0_block_w=4
                    //     [4907] ERROR: UndefinedBehavior:
                    //            tensix_unpacr: unpack_to_dst=0 in_data_format=0 out_data_format=0
                    //
                    // — the same refusal `spatial_choreography_semantics_lock` recorded, unchanged. The
                    // consequence of the flip was not a faster plan, it was a program the device REJECTS:
                    // every exact-numerics matmul big enough to block emitted a kernel that aborts at
                    // runtime. Falling back to the streaming family is not a lost optimization, it is the
                    // only family that runs.
                    //
                    // A backend returning `Ok` proves a program was PRINTED. Only the simulator says
                    // whether it runs — and nothing in the type system could have caught this.
                    wide_spill_reload: false,
                    acc_in_registers: false,
                    packer_accumulate: PackerAccumulate::IntoTheOutput,
                    acc_int_bits: 32,
                    max_array: None,
                },
                ComputeCap::Elementwise {
                    precisions: [Precision::Bf16, Precision::Fp16, Precision::Fp32]
                        .into_iter()
                        .collect(),
                },
                ComputeCap::RowReduce {
                    precisions: [Precision::Bf16, Precision::Fp16, Precision::Fp32]
                        .into_iter()
                        .collect(),
                },
                ComputeCap::Scalar(FabricTarget::wearable()),
            ],
            dram: MemHierarchy {
                sram_kib: 64 * 1464,
                hbm_gib: 0,
                ddr_gib: 12,
            },
            precisions: [
                Precision::Bf16,
                Precision::Fp16,
                Precision::Fp32,
                Precision::Int8,
            ]
            .into_iter()
            .collect(),
            noc_rule: Some(NocRule {
                sender_noc: 0,
                receiver_noc: 1,
                writer_noc_default: 0,
                writer_noc_first_column: 1,
            }),
            write_ordering: WriteOrdering::Ordered,
            cycle: CycleModel::wormhole_ttsim(),
            modeled: &[],
            roofs: Roofs::wormhole(),
            part: None,
        }
    }

    /// Tenstorrent Blackhole: 13×10 compute-with-storage grid, 1536 KiB of L1 per Tensix, eight DRAM
    /// channels of 3.984 GiB, and NOC writes that need an explicit flush before a dependent semaphore
    /// multicast. The accumulator is the same eight-tile half-sync window as Wormhole's.
    ///
    /// ★ `..wh` says "the same part, these fields different", which is true of the NOC rule, the
    /// precisions and the compute capabilities — and was silently ALSO true of Wormhole's L1 size, its
    /// SRAM total and its DRAM capacity, none of which are Blackhole's. `size_buffers` shrinks a program
    /// to `local_mem_bytes` and `tile.rs` blocks against it, so inheriting 1464 KiB cost this chip 72 KiB
    /// of usable L1 on every program it ever compiled. The three lines below are read out of the vendor's
    /// `blackhole_140_arch.yaml` and held there by `spatial_chip_facts_lock`.
    pub fn blackhole() -> SpatialTarget {
        let wh = SpatialTarget::wormhole();
        let sites = MeshShape { x: 13, y: 10 };
        let l1_kib = 1536;
        SpatialTarget {
            name: "blackhole",
            sites,
            // ★ THE ACCUMULATOR IS THE HALF-SYNC WINDOW, AND IT IS THE SAME EIGHT ON BOTH PARTS.
            //
            // Blackhole's DST is not bigger than Wormhole's: the vendor's `tensix_types.h` gives both
            // parts a 16x16 dest face, so `DEST_NUM_TILES_FP16` is 16 on each and the half is 8. Our
            // compute kernels bracket their math with `tile_regs_acquire`/`tile_regs_commit`, which is
            // `DstSync::SyncHalf` — the packer drains one half while math fills the other — so a single
            // acquire reaches eight tiles, never sixteen.
            //
            // This said 16, and nothing caught it until a shape appeared whose plan could USE more than
            // eight: at 2048x1024x512 the planner chose a 4x4 sub-block, the kernel packed sixteen tiles
            // through an eight-tile window, and the program returned PCC 0.872 having retired EXACTLY the
            // right arithmetic. 640^3 passed on the same chip only because its sub-block was four tiles.
            // Held to the vendor header by `spatial_chip_facts_lock`.
            site: SiteSpec {
                local_mem_bytes: l1_kib * 1024,
                ..wh.site
            },
            dram: MemHierarchy {
                sram_kib: sites.sites() * l1_kib,
                hbm_gib: 0,
                ddr_gib: 31,
            },
            write_ordering: WriteOrdering::NeedsFlush,
            cycle: CycleModel::uncalibrated(),
            // The descriptor publishes 140 functional workers; 13×10 is the compute-with-storage grid the
            // part exposes. The DST depth is no longer modelled — it is read from the vendor's
            // `tensix_types.h` and locked.
            modeled: &["sites"],
            roofs: Roofs::blackhole(),
            ..wh
        }
    }

    /// Our own Sky130/FPGA fabric: ONE site (the whole array), nearest-neighbour wires, valid/ready and
    /// FIFO handshakes, an exact-arithmetic tile engine (the proven weight-/output-stationary systolic
    /// array), and the embedded CGRA scalar datapath `FabricTarget::full(rows, cols, 64)`.
    pub fn sky_fabric(rows: usize, cols: usize) -> SpatialTarget {
        let scalar = FabricTarget::full(rows, cols, 64);
        SpatialTarget {
            name: "sky130-fabric",
            sites: MeshShape { x: 1, y: 1 },
            site: SiteSpec {
                local_mem_bytes: scalar.mem_depth * 8,
                local_mem_opt_in_bytes: None,
                acc_tiles: cols,
                fp32_acc_halves_acc_tiles: false,
                movement_engines: 1,
                nocs: 0,
                credit: None,
                local_mem_read_latency: Some(0),
                mem_banks: 1,
                vector_lanes: 1,
                int_compute: None,
                multiplier: None,
            },
            tile: TileShape::SCALAR,
            interconnect: [Interconnect::NearestNeighbour].into_iter().collect(),
            handshakes: [HandshakeKind::ValidReady, HandshakeKind::Fifo]
                .into_iter()
                .collect(),
            compute: vec![
                ComputeCap::MatmulTile {
                    precisions: [
                        Precision::Int8,
                        Precision::Int16,
                        Precision::Int64,
                        Precision::Bf16,
                    ]
                    .into_iter()
                    .collect(),
                    fidelity: Fidelity::Exact,
                    fp32_acc: true,
                    dataflows: [
                        DataflowPolicy::WeightStationary,
                        DataflowPolicy::OutputStationary,
                    ]
                    .into_iter()
                    .collect(),
                    // The systolic array runs one sub-block over its whole inner block per call: the lowering issues
                    // one `Matmul` per compute op, whatever the granularity, and each call pays the array's fill.
                    block_api: Some(BlockApi::Systolic),
                    wide_spill_reload: true,
                    acc_in_registers: false,
                    packer_accumulate: PackerAccumulate::InLocalMemory,
                    acc_int_bits: 64,
                    max_array: None,
                },
                ComputeCap::Elementwise {
                    precisions: [Precision::Int8, Precision::Int16, Precision::Int64]
                        .into_iter()
                        .collect(),
                },
                ComputeCap::Scalar(scalar.clone()),
            ],
            dram: scalar.mem,
            precisions: [
                Precision::Int8,
                Precision::Int16,
                Precision::Int64,
                Precision::Bf16,
            ]
            .into_iter()
            .collect(),
            noc_rule: None,
            write_ordering: WriteOrdering::Ordered,
            cycle: CycleModel::uncalibrated(),
            modeled: &["acc_tiles"],
            roofs: Roofs::sky_fabric(),
            part: None,
        }
    }

    /// Our fabric on the Artix-7 XC7A100T of the Nexys A7-100T: [`sky_fabric`](Self::sky_fabric)'s machine with its
    /// local memory in the part's block RAM — 135 blocks of 36 Kb (Xilinx DS180, Table 3) — read one cycle after its
    /// address, as a block RAM's registered read port returns it, and an 8×8 PE array: 64 of the part's 240 DSP48
    /// slices (DS180, Table 3).
    pub fn xc7a100t_fabric(rows: usize, cols: usize) -> SpatialTarget {
        SpatialTarget::xc7_fabric("xc7a100t-fabric", PartResources::xc7a100t(), 8, 8, rows, cols)
    }

    /// Our fabric on the Artix-7 XC7A200T of the Nexys Video: 365 blocks of 36 Kb, read as
    /// [`xc7a100t_fabric`](Self::xc7a100t_fabric) reads its own, and a 16×16 PE array: 256 of the part's 740 DSP48
    /// slices (DS180, Table 3).
    pub fn xc7a200t_fabric(rows: usize, cols: usize) -> SpatialTarget {
        SpatialTarget::xc7_fabric("xc7a200t-fabric", PartResources::xc7a200t(), 16, 16, rows, cols)
    }

    /// This target on boards joined by a link: each board one chip of a tensor-parallel program, the link the hand-off
    /// a partial or an activation crosses from one to the other, a token with each word. A chip boundary needs that
    /// hand-off ([`HandshakeKind::Token`]); a hand-off on one board keeps its own.
    pub fn linked(mut self) -> SpatialTarget {
        self.handshakes.insert(HandshakeKind::Token);
        self
    }

    /// A 7-series part's fabric: each 36 Kb block holds 32 Kb of data beside its 4 Kb of parity, the PE array is
    /// `array × array`, a matmul's block cut to it, and each memory is as many banks as the array has columns, so a row
    /// of operands moves in a cycle.
    fn xc7_fabric(name: &'static str, part: PartResources, array: usize, lanes: usize, rows: usize, cols: usize) -> SpatialTarget {
        let sky = SpatialTarget::sky_fabric(rows, cols);
        let mut compute = sky.compute.clone();
        for c in compute.iter_mut() {
            if let ComputeCap::MatmulTile { max_array, .. } = c {
                *max_array = Some((array, array));
            }
        }
        let blocks = part.memory(MemoryKind::Block36).expect("a 7-series part has block RAM").count.value as usize;
        let (mw, mh) = part.multiplier.value;
        SpatialTarget {
            name,
            site: SiteSpec {
                local_mem_bytes: blocks * 4096,
                local_mem_read_latency: Some(1),
                mem_banks: array,
                vector_lanes: lanes,
                int_compute: None,
                multiplier: Some((mw, mh)),
                ..sky.site
            },
            compute,
            roofs: Roofs::xc7_fabric(),
            part: Some(part),
            ..sky
        }
    }

    /// The SambaNova SN40L RDU, derived from [`RduChipFacts::sn40l`](super::rdu_chip::RduChipFacts::sn40l):
    /// 1040 PCUs (the sites — the published `compute_tiles`), 520 MiB of on-chip SRAM spread over them, a
    /// crossbar switch fabric, token / valid-ready / barrier handshakes, and the embedded
    /// `FabricTarget::rdu_shaped()` scalar datapath. The mesh aspect, the per-PCU accumulator depth and the
    /// tile geometry are not public — listed in `modeled`, and cited on the facts' unit model.
    pub fn sn40l_rdu() -> SpatialTarget {
        super::rdu_chip::RduChipFacts::sn40l().spatial_target()
    }

    /// The SambaNova SN50 RDU, derived from [`RduChipFacts::sn50`](super::rdu_chip::RduChipFacts::sn50):
    /// 432 MB of on-chip SRAM, 64 GB HBM2E, DDR5 up to 512 GB, the same crossbar / handshake / compute
    /// envelope as the SN40L, and the embedded `FabricTarget::sn50_shaped()` scalar datapath. The PCU/PMU
    /// counts are not public, so the mesh, the per-site memory and the silicon tile figures join the
    /// `modeled` list beside the aspect, the accumulator and the tile.
    pub fn sn50_rdu() -> SpatialTarget {
        super::rdu_chip::RduChipFacts::sn50().spatial_target()
    }

    /// The local RTX 3060 (sm_86): 28 SMs, each site a persistent thread block with 48 KiB static shared
    /// memory (99 KiB on opt-in, from the CUDA chip descriptor), 16×16 tensor-core fragments, one producer
    /// engine (cp.async warps), intra-block shared memory + global memory only (no clusters / DSMEM on
    /// sm_86), pipeline barriers. The accumulator fragment budget per warp is modeled.
    pub fn rtx3060() -> SpatialTarget {
        let chip = crate::codegen_cuda::chip::CudaChip::rtx3060();
        SpatialTarget {
            name: "rtx3060",
            // ★ SITES ARE CUDA BLOCKS, NOT SMs — and there must be far more than SMs.
            //
            // One block per SM is 8 warps of the 48 an Ampere SM holds: 17% occupancy, with nothing
            // to run while a load is outstanding. A GPU hides DRAM latency by having OTHER WARPS,
            // and a block here needs only ~16 KiB of shared memory, so many fit. Sizing the mesh at
            // the SM count is a Tensix idea — there a site is a physical core — and on a GPU it
            // left most of the machine idle.
            //
            // MEASURED at 2048³ (the ratchet's own shape), everything else equal:
            //   28 → 2,397 GOPS · 112 → 5,389 · 168 → 5,370 · 224 → 5,366 · 448 → 5,848
            // The knee is at ~4 blocks per SM; past that the grid simply exceeds the item count and
            // the extra sites stand empty, which costs nothing. 448 is chosen so a GEMM's items are
            // never artificially serialised onto too few blocks.
            //
            // Adjacency carries no meaning here — no multicast — so `place` spreads items across
            // them linearly rather than by the 2-D map a physical mesh needs.
            sites: MeshShape { x: 448, y: 1 },
            site: SiteSpec {
                local_mem_bytes: 48 * 1024,
                local_mem_opt_in_bytes: Some(chip.smem_kib_per_block * 1024),
                acc_tiles: 64,
                fp32_acc_halves_acc_tiles: false,
                movement_engines: 1,
                nocs: 0,
                credit: None,
                local_mem_read_latency: None,
                mem_banks: 1,
                vector_lanes: 1,
                int_compute: None,
                multiplier: None,
            },
            tile: TileShape::WMMA16,
            interconnect: [Interconnect::SharedMemoryBlock, Interconnect::GlobalMemory]
                .into_iter()
                .collect(),
            handshakes: [
                HandshakeKind::Barrier,
                HandshakeKind::Fifo,
                HandshakeKind::Semaphore,
            ]
            .into_iter()
            .collect(),
            compute: vec![
                ComputeCap::MatmulTile {
                    precisions: [Precision::Bf16, Precision::Fp16, Precision::Int8]
                        .into_iter()
                        .collect(),
                    fidelity: Fidelity::Exact,
                    fp32_acc: true,
                    dataflows: [DataflowPolicy::OutputStationary].into_iter().collect(),
                    block_api: Some(BlockApi::Fragments),
                    wide_spill_reload: false,
                    acc_in_registers: true,
                    packer_accumulate: PackerAccumulate::No,
                    acc_int_bits: 32,
                    max_array: None,
                },
                ComputeCap::Elementwise {
                    precisions: chip.precisions.iter().copied().collect(),
                },
                ComputeCap::RowReduce {
                    precisions: chip.precisions.iter().copied().collect(),
                },
            ],
            dram: MemHierarchy {
                sram_kib: 28 * 128,
                hbm_gib: 0,
                ddr_gib: 12,
            },
            precisions: chip.precisions.iter().copied().collect(),
            noc_rule: None,
            write_ordering: WriteOrdering::Ordered,
            cycle: CycleModel::uncalibrated(),
            modeled: &["acc_tiles"],
            roofs: Roofs::rtx3060(),
            part: None,
        }
    }

    pub fn provides_handshake(&self, h: HandshakeKind) -> bool {
        self.handshakes.contains(&h)
    }

    pub fn provides_interconnect(&self, i: Interconnect) -> bool {
        self.interconnect.contains(&i)
    }

    /// The target can reload a spilled partial of the accumulator's own width.
    pub fn reloads_wide_spills(&self) -> bool {
        matches!(
            self.matmul_cap(),
            Some(ComputeCap::MatmulTile {
                wide_spill_reload: true,
                ..
            })
        )
    }

    /// The accumulator survives the whole reduction in the compute engine's own registers, so a
    /// blocked plan never spills a partial — see [`ComputeCap::MatmulTile::acc_in_registers`].
    pub fn accumulator_in_registers(&self) -> bool {
        matches!(
            self.matmul_cap(),
            Some(ComputeCap::MatmulTile {
                acc_in_registers: true,
                ..
            })
        )
    }

    /// Independent accumulator LANES a site runs a block's tiles across — CUDA warps on a GPU, one
    /// elsewhere (a Tensix site has a single DST register file).
    /// ★ HOW MANY SITES OF THIS FOOTPRINT A PHYSICAL UNIT HOLDS AT ONCE — the price of a deeper stage.
    ///
    /// On Tensix a site IS a core, so the answer is one and a deeper contraction stage costs nothing but
    /// L1. On a GPU a site is a thread block and an SM holds several; the number is set by whichever of
    /// registers and shared memory runs out first, and THAT is what makes depth a trade rather than a win.
    ///
    /// `tile::best_candidate_for` ranks register-accumulator plans by arithmetic intensity, and
    /// `intensity_key` cancels the contraction depth exactly — so depth fell to a tiebreak that maximized
    /// it, reasoning that "a deeper stage amortises the pipeline round-trip over more multiplies". True,
    /// and silent about the price. MEASURED at 2048³ bf16 on an RTX 3060, one 128×128 output tile:
    ///
    ///   ibw 1 (K=16): 24 KiB, 2 blocks/SM → 5,429 GOPS
    ///   ibw 2 (K=32): 40 KiB, 2 blocks/SM → 5,743 GOPS   ← the deepest stage that keeps its occupancy
    ///   ibw 4 (K=64): 72 KiB, 1 block/SM  → 4,102 GOPS
    ///
    /// The amortisation is real and the occupancy cliff is bigger. K=32 is also exactly the `BK` the
    /// hand-tuned knob lattice picks for this shape, which is the reassurance that the rule is the
    /// machine's and not ours.
    ///
    /// The constants are the CUDA lowering's own (64K registers and 100 KiB of shared memory per SM); this
    /// is where they live so the planner and the emitter cannot disagree about occupancy — a duplicate of
    /// this rule would drift the first time either moved.
    pub fn resident_sites_per_unit(&self, footprint_bytes: usize, osh: usize, osw: usize) -> usize {
        if !self.accumulator_in_registers() {
            return 1;
        }
        const REGS_PER_UNIT: usize = 65536;
        const SMEM_PER_UNIT: usize = 100 * 1024;
        let lanes = self.accumulator_lanes();
        let per_lane_tiles = (osh * osw).div_ceil(lanes).max(1);
        let regs_est = per_lane_tiles * 8 + 48;
        let threads = lanes * 32;
        let by_regs = REGS_PER_UNIT / (regs_est * threads).max(1);
        let by_smem = if footprint_bytes == 0 {
            16
        } else {
            SMEM_PER_UNIT / footprint_bytes.max(1)
        };
        by_regs.min(by_smem).clamp(1, 16)
    }

    pub fn accumulator_lanes(&self) -> usize {
        if self.accumulator_in_registers() {
            8
        } else {
            1
        }
    }

    /// ★ HOW A BLOCK'S `osh × osw` OUTPUT TILES SPLIT ACROSS THE SITE'S ACCUMULATOR LANES.
    ///
    /// A lane that owns a RECTANGLE of `(osh/wm) × (osw/wn)` tiles loads that many A-fragments and
    /// B-fragments per contraction step and does their product — the reuse that makes a GEMM a
    /// GEMM. The split is chosen to make the rectangle as SQUARE as possible: for a fixed area the
    /// squarest rectangle has the smallest perimeter, and the perimeter is what is loaded.
    ///
    /// `None` when no split divides the block evenly. THE PLANNER MUST NOT OFFER SUCH A BLOCK on a
    /// register-accumulator target: its lanes cannot each own a rectangle, so the epilogue cannot
    /// write straight to global and the output has to stage through a ring the plan was not sized
    /// for. (It was: a 13×3 block sized against a 4-tile scratch wrote 39 tiles into it, which the
    /// device reported as an illegal access.)
    pub fn accumulator_lane_grid(&self, osh: usize, osw: usize) -> Option<(usize, usize)> {
        let lanes = self.accumulator_lanes();
        let mut best: Option<(usize, usize)> = None;
        for wm in 1..=lanes {
            if lanes % wm != 0 {
                continue;
            }
            let wn = lanes / wm;
            if osh % wm != 0 || osw % wn != 0 {
                continue;
            }
            if best.map_or(true, |(bm, bn)| osh / wm + osw / wn < osh / bm + osw / bn) {
                best = Some((wm, wn));
            }
        }
        best
    }

    /// The target's packer can accumulate into a local-memory page.
    pub fn packer_accumulates(&self) -> bool {
        matches!(
            self.matmul_cap(),
            Some(ComputeCap::MatmulTile {
                packer_accumulate: PackerAccumulate::InLocalMemory
                    | PackerAccumulate::IntoTheOutput,
                ..
            })
        )
    }

    /// The target's packer can accumulate into the output's own page, its intermediate sharing the output's memory.
    pub fn packer_accumulates_into_the_output(&self) -> bool {
        matches!(
            self.matmul_cap(),
            Some(ComputeCap::MatmulTile {
                packer_accumulate: PackerAccumulate::IntoTheOutput,
                ..
            })
        )
    }

    /// The tile-matmul capability, if the site has a matrix engine.

    pub fn matmul_cap(&self) -> Option<&ComputeCap> {
        self.compute
            .iter()
            .find(|c| matches!(c, ComputeCap::MatmulTile { .. }))
    }

    /// Whether the matrix engine accepts `p` as an operand precision.
    pub fn matmul_supports(&self, p: Precision) -> bool {
        match self.matmul_cap() {
            Some(ComputeCap::MatmulTile { precisions, .. }) => precisions.contains(&p),
            _ => false,
        }
    }

    /// The same machine with its matrix engine declared at a different fidelity.
    ///
    /// A Tensix core runs bf16 matmul at any of LoFi/HiFi2/HiFi3/HiFi4 — the fidelity is how many passes the
    /// engine makes, not a property of the part — but the descriptor pinned one value, so the pipeline could
    /// only ever emit HiFi4. That left us unable to emit at ttnn's OWN bf16 default (HiFi2), which is the
    /// configuration the vendor's tuned op actually runs, so the one knob group where a like-for-like
    /// head-to-head matters had nothing of ours in it at all.
    ///
    /// The refusal this unblocks is worth keeping in mind: the emitter still refuses to PRINT a fidelity it
    /// did not run (`cost.rs` gates on `matmul_fidelity()`), and that guard is untouched. This changes what
    /// the machine is declared to do, not what a program may claim about itself.
    pub fn at_fidelity(mut self, fidelity: Fidelity) -> SpatialTarget {
        for cap in &mut self.compute {
            if let ComputeCap::MatmulTile { fidelity: f, .. } = cap {
                *f = fidelity;
            }
        }
        self
    }

    /// The fidelity the matrix engine is declared at — the one the pipeline emits.
    pub fn matmul_fidelity(&self) -> Option<Fidelity> {
        match self.matmul_cap() {
            Some(ComputeCap::MatmulTile { fidelity, .. }) => Some(*fidelity),
            _ => None,
        }
    }

    /// Whether the matrix engine can accumulate in fp32.
    pub fn matmul_fp32_acc(&self) -> bool {
        matches!(
            self.matmul_cap(),
            Some(ComputeCap::MatmulTile { fp32_acc: true, .. })
        )
    }

    /// Accumulator width of the matrix engine's integer path.
    pub fn matmul_acc_int_bits(&self) -> u32 {
        match self.matmul_cap() {
            Some(ComputeCap::MatmulTile { acc_int_bits, .. }) => *acc_int_bits,
            _ => 64,
        }
    }

    /// The integer precisions a buffer is stored at, narrowest first: those the elementwise datapath lists, or — on a
    /// site that computes integers in one width and widens nothing ([`SiteSpec::int_compute`]) — that width alone.
    pub fn storable_ints(&self) -> Vec<Precision> {
        let ps = self.compute.iter().find_map(|c| match c {
            ComputeCap::Elementwise { precisions } => Some(precisions),
            _ => None,
        });
        [Precision::Int8, Precision::Int16, Precision::Int32, Precision::Int64]
            .into_iter()
            .filter(|p| ps.is_some_and(|ps| ps.contains(p)) && self.site.int_compute.is_none_or(|c| c == *p))
            .collect()
    }

    /// The largest PE array the matrix engine builds, `(contraction rows, output columns)`, if it caps one.
    pub fn matmul_max_array(&self) -> Option<(usize, usize)> {
        match self.matmul_cap() {
            Some(ComputeCap::MatmulTile { max_array, .. }) => *max_array,
            _ => None,
        }
    }

    /// Whether the matrix engine takes whole sub-blocks per call.
    pub fn matmul_block_api(&self) -> bool {
        matches!(
            self.matmul_cap(),
            Some(ComputeCap::MatmulTile {
                block_api: Some(_),
                ..
            })
        )
    }

    /// The sub-block API a compute op of `granularity` runs on: the engine's own, or none where the op is handed
    /// a tile pair at a time.
    fn block_api_for(&self, granularity: super::ir::Granularity) -> Option<BlockApi> {
        match (granularity, self.matmul_cap()) {
            (
                super::ir::Granularity::PerSubBlock,
                Some(ComputeCap::MatmulTile { block_api, .. }),
            ) => *block_api,
            _ => None,
        }
    }

    /// Calls the matrix engine makes for one compute op over `block`: by the engine's [`BlockApi`], or one per
    /// tile pair, `osh · osw · ibw`.
    pub fn matmul_calls(
        &self,
        block: super::ir::BlockShape,
        granularity: super::ir::Granularity,
    ) -> u64 {
        let (osh, osw, ibw) = (block.osh as u64, block.osw as u64, block.ibw as u64);
        match self.block_api_for(granularity) {
            Some(BlockApi::Unpacker) | Some(BlockApi::Fragments) => ibw,
            Some(BlockApi::Systolic) => 1,
            None => osh * osw * ibw,
        }
    }

    /// Operand tiles the matrix engine loads for one compute op over `block`: by the engine's [`BlockApi`], or
    /// two for each tile-pair call.
    pub fn operand_loads(
        &self,
        block: super::ir::BlockShape,
        granularity: super::ir::Granularity,
    ) -> u64 {
        let (osh, osw, ibw) = (block.osh as u64, block.osw as u64, block.ibw as u64);
        match self.block_api_for(granularity) {
            Some(BlockApi::Unpacker) => (osh.min(osw) + osh * osw) * ibw,
            Some(BlockApi::Systolic) => (osh + osw) * ibw,
            Some(BlockApi::Fragments) => match self.accumulator_lane_grid(block.osh, block.osw) {
                Some((wm, wn)) => (wn as u64 * osh + wm as u64 * osw) * ibw,
                None => 2 * osh * osw * ibw,
            },
            None => 2 * osh * osw * ibw,
        }
    }

    /// The dataflow policies the matrix engine realizes, in descriptor order.
    pub fn matmul_dataflows(&self) -> Vec<DataflowPolicy> {
        match self.matmul_cap() {
            Some(ComputeCap::MatmulTile { dataflows, .. }) => dataflows.iter().copied().collect(),
            _ => Vec::new(),
        }
    }

    pub fn has_elementwise(&self) -> bool {
        self.compute
            .iter()
            .any(|c| matches!(c, ComputeCap::Elementwise { .. }))
    }

    pub fn has_row_reduce(&self) -> bool {
        self.compute
            .iter()
            .any(|c| matches!(c, ComputeCap::RowReduce { .. }))
    }

    /// The precisions the elementwise (tile) datapath computes in — what a `Generic` stage reads and writes.
    pub fn elementwise_precisions(&self) -> BTreeSet<Precision> {
        self.compute
            .iter()
            .find_map(|c| match c {
                ComputeCap::Elementwise { precisions } => Some(precisions.clone()),
                _ => None,
            })
            .unwrap_or_default()
    }

    /// ★ THE TYPE AN INDEX RIDES. An index is an ADDRESS — a token id, a cache position, a block start — so it
    /// is computed in an address-wide integer: the narrowest one of at least 32 bits the elementwise datapath
    /// has that HOLDS the index space `[0, max]`. On an RDU that is INT32, the widest integer the PCU has.
    /// `Err` names the precision the range would need — what the target is missing — so an index space past
    /// the chip's integers is refused, never wrapped.
    pub fn index_precision_for(&self, max: u64) -> Result<Precision, Precision> {
        let wide = [Precision::Int32, Precision::Int64];
        let holds = |p: Precision| int_precision_max(p).is_some_and(|m| max <= m as u64);
        let have = self.elementwise_precisions();
        match wide.into_iter().find(|p| have.contains(p) && holds(*p)) {
            Some(p) => Ok(p),
            None => Err(wide
                .into_iter()
                .find(|p| holds(*p))
                .unwrap_or(Precision::Int64)),
        }
    }

    /// Whether a `Generic` stage may hold a value of precision `p`: a precision the elementwise datapath
    /// computes in, or the matrix engine's fp32 accumulator width — the accumulation path every reduction
    /// scratch rides.
    pub fn generic_holds(&self, p: Precision) -> bool {
        self.elementwise_precisions().contains(&p)
            || (p == Precision::Fp32 && self.matmul_fp32_acc())
    }

    /// The embedded scalar (CGRA) datapath descriptor, if the site has one.
    pub fn scalar_target(&self) -> Option<&FabricTarget> {
        self.compute.iter().find_map(|c| match c {
            ComputeCap::Scalar(t) => Some(t),
            _ => None,
        })
    }

    /// Accumulator tiles available under an accumulation policy — the ONE place fp32 halving lives. The
    /// halving is paid only when the fp32 partial sums must survive packing (`exact`); a program that accepts
    /// bf16 spill rounding keeps the full accumulator file.
    pub fn effective_acc_tiles(&self, acc: AccPolicy, exact: bool) -> usize {
        if acc == AccPolicy::F32 && exact && self.site.fp32_acc_halves_acc_tiles {
            (self.site.acc_tiles / 2).max(1)
        } else {
            self.site.acc_tiles
        }
    }

    /// Bytes of one native tile at precision `p`.
    pub fn tile_bytes(&self, p: Precision) -> usize {
        self.tile.elems() * precision_bytes(p)
    }

    /// The largest local-memory budget a site can be configured with.
    pub fn max_local_mem_bytes(&self) -> usize {
        self.site
            .local_mem_opt_in_bytes
            .unwrap_or(self.site.local_mem_bytes)
            .max(self.site.local_mem_bytes)
    }

    /// Every preset, for matrix locks.
    /// The same chip on a different deployment grid: the compute-with-storage mesh a device exposes depends on
    /// harvesting and on the dispatch mode (fast dispatch reserves a row of Tensix cores for the dispatch kernels).
    pub fn with_sites(self, x: usize, y: usize) -> SpatialTarget {
        SpatialTarget {
            sites: MeshShape { x, y },
            ..self
        }
    }

    /// Wormhole as the cycle-accurate simulator presents it: `wormhole_b0_80_arch.yaml` (10 Tensix rows, none
    /// harvested) under slow dispatch exposes an 8×9 compute-with-storage grid — 72 cores, one row reserved. A
    /// production n150 (one harvested row, fast dispatch) exposes 8×8. Every vendor example measured on the
    /// simulator spread its work over these 72 cores; a program placed onto 8×8 leaves a ninth row idle.
    pub fn wormhole_ttsim() -> SpatialTarget {
        let grid = sim_grid("wormhole_b0").expect("the simulated wormhole grid");
        SpatialTarget::wormhole().with_sites(grid.x, grid.y)
    }

    // There is deliberately no `blackhole_ttsim()`: `blackhole()`'s 13×10 already IS the grid
    // `sim_grid("blackhole")` reports, so the twin would be the preset itself. Wormhole needs one only
    // because its preset is the production n150 (8×8, one row harvested) and the simulator exposes 8×9.

    pub fn presets() -> Vec<SpatialTarget> {
        vec![
            SpatialTarget::wormhole(),
            SpatialTarget::blackhole(),
            SpatialTarget::sky_fabric(64, 64),
            SpatialTarget::sn40l_rdu(),
            SpatialTarget::sn50_rdu(),
            SpatialTarget::rtx3060(),
        ]
    }
}

/// The compute-with-storage grid the cycle-accurate simulator exposes for a chip id (slow dispatch):
/// `wormhole_b0_80_arch.yaml` → 8×9, `blackhole_140_arch.yaml` → 13×10 (one column of the 14×10 reserved).
/// Read from the vendor examples' `compute_with_storage_grid_size()` on each descriptor.
pub fn sim_grid(chip: &str) -> Option<MeshShape> {
    match chip {
        "wormhole_b0" => Some(MeshShape { x: 8, y: 9 }),
        "blackhole" => Some(MeshShape { x: 13, y: 10 }),
        _ => None,
    }
}
