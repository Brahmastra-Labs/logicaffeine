//! `RduChipFacts` — ONE source of facts for the SambaNova RDU family, from which BOTH chip descriptors
//! (the spatial mesh [`SpatialTarget`] and the scalar-fabric envelope [`FabricTarget`]) are derived.
//!
//! Every number is a [`Fact`] with a provenance: `Published` reads a vendor document named in the chip's
//! `cites`; `Modeled` fills a gap the public record leaves and says what it was inherited from. Nothing
//! here is a measurement of the chip and nothing is a time — the descriptors, the neutral artifact and
//! the fit report all label what they carry, and the locks reflect over the serialized facts so a number
//! without a citation cannot land.
//!
//! The **unit model** ([`RduUnitModel`]) is what the public record does not say about a PCU, a PMU and the
//! mesh — inherited from the open RDU, Plasticine (Prabhakar et al., ISCA 2017): a six-stage SIMD pipeline
//! of sixteen lanes, a 16-bank × 4096-word scratchpad with a four-stage address datapath, double-buffered
//! as the SN50 public description says. The credit-loop latencies are `None` — undeclared, NOT zero —
//! until the SDK names them; then [`RduChipFacts::with_credit`] drops the number in as data and every
//! derived figure ([`rdu_ii_report`], the descriptors, the envelope) follows without code.
//!
//! [`rdu_ii_report`] is the II view of a placed [`SpatialProgram`] on this unit model, built on the same
//! models the Plasticine lane uses — [`metapipeline`](crate::codegen_sva::metapipeline) for stage II and
//! N-buffer depths, [`pmu`](crate::codegen_sva::pmu) for bank conflicts, and
//! [`choreography::credit_depth`](crate::codegen_sva::choreography::credit_depth) for the bandwidth-delay
//! solve. MODELED: the model's cycles, never a simulation of the chip.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use super::cost::CycleModel;
use super::ir::{ComputeOp, IterKind, SiteRole, SpatialProgram, StageKind, TensorBody};
use super::roofs::{
    ComputeRoof, Engine, MemLevel, MemRoof, Rate, Roofs, Scope, StreamMix, Transfer,
};
use super::target::{
    ComputeCap, CreditLatency, DataflowPolicy, Fidelity, HandshakeKind, Interconnect, MeshShape,
    PackerAccumulate, SiteSpec, SpatialTarget, TileShape, WriteOrdering,
};
use crate::codegen_sva::choreography::credit_depth;
use crate::codegen_sva::fabric_target::{
    Dataflow, FabricTarget, MemHierarchy, Precision, SiliconSpec, ALL_ALU_OPS, ALL_FUNC_UNITS,
};
use crate::codegen_sva::metapipeline::{Metapipeline, Stage, UnitModel};
use crate::codegen_sva::pmu::{memory_ii, strided_banking_for, AffineAccess, BankingMode, PmuSpec};
use crate::codegen_sva::simd::LaneTy;

/// The RDU generations the facts describe.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RduChip {
    Sn40l,
    Sn50,
}

impl RduChip {
    pub const ALL: [RduChip; 2] = [RduChip::Sn40l, RduChip::Sn50];

    pub fn facts(self) -> RduChipFacts {
        match self {
            RduChip::Sn40l => RduChipFacts::sn40l(),
            RduChip::Sn50 => RduChipFacts::sn50(),
        }
    }

    /// The dial spelling (`--chip sn50`).
    pub fn as_str(self) -> &'static str {
        match self {
            RduChip::Sn40l => "sn40l",
            RduChip::Sn50 => "sn50",
        }
    }

    pub fn parse(s: &str) -> Option<RduChip> {
        match s.trim().to_ascii_lowercase().as_str() {
            "sn40l" => Some(RduChip::Sn40l),
            "sn50" => Some(RduChip::Sn50),
            _ => None,
        }
    }

    /// The vendor's part name.
    pub fn part(self) -> &'static str {
        match self {
            RduChip::Sn40l => "SN40L",
            RduChip::Sn50 => "SN50",
        }
    }

    /// The name both descriptors carry.
    pub fn target_name(self) -> &'static str {
        match self {
            RduChip::Sn40l => "sn40l-rdu",
            RduChip::Sn50 => "sn50-rdu",
        }
    }
}

/// Where a number comes from, strongest first: read from a public source, computed from read numbers,
/// measured by a run, or a model's. The order is the trust order, so a figure computed from several
/// facts is as strong as the weakest of them: their `max`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Provenance {
    Published,
    Derived,
    Measured,
    Modeled,
}

/// A number with its provenance and its source.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
pub struct Fact<T> {
    pub value: T,
    pub provenance: Provenance,
    /// For a published fact: the public source (one of the chip's `cites`) and the figure as it appears
    /// there. For a derived fact: the published inputs and the arithmetic. For a measured fact: the run's
    /// ledger and its method. For a modeled fact: what gap it fills and what it inherits.
    pub cite: &'static str,
}

impl<T> Fact<T> {
    pub const fn published(value: T, cite: &'static str) -> Fact<T> {
        Fact {
            value,
            provenance: Provenance::Published,
            cite,
        }
    }
    pub const fn derived(value: T, cite: &'static str) -> Fact<T> {
        Fact {
            value,
            provenance: Provenance::Derived,
            cite,
        }
    }
    pub const fn measured(value: T, cite: &'static str) -> Fact<T> {
        Fact {
            value,
            provenance: Provenance::Measured,
            cite,
        }
    }
    pub const fn modeled(value: T, cite: &'static str) -> Fact<T> {
        Fact {
            value,
            provenance: Provenance::Modeled,
            cite,
        }
    }
    pub fn is_modeled(&self) -> bool {
        self.provenance == Provenance::Modeled
    }
    pub fn is_published(&self) -> bool {
        self.provenance == Provenance::Published
    }
}

/// How the chips connect beyond the node count: the published rack topology and the scale-up fabric per
/// RDU. Present only where the vendor tabulates the links (the SN50); the SN40L's per-link fabric is not.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct ScaleUp {
    pub chips_per_rack: usize,
    /// Fully connected intra-node links per RDU.
    pub intra_node_links: usize,
    /// Inter-node links per RDU.
    pub inter_node_links: usize,
    /// Per-link rate in Gbit/s.
    pub link_gbps: u32,
    /// Aggregate scale-up bandwidth per RDU in TB/s.
    pub tb_s_per_chip: f64,
    /// The scale-out fabric, as the vendor names it.
    pub scale_out: &'static str,
}

/// A figure a source states for a fact other than the one the fact carries: superseded by a later source, or one of two
/// current figures the sources disagree on. Kept beside the facts, so no published figure is lost.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
pub struct OtherFigure {
    /// The fact it is a figure of, by its field name.
    pub fact: &'static str,
    /// The figure as the source states it.
    pub stated: &'static str,
    /// The source's bibliography key, where in it, and why the fact carries another figure.
    pub cite: &'static str,
}

/// What the public record does not say about a PCU, a PMU and the mesh — every field modeled and cited,
/// until an SDK number replaces it as `Published` data.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct RduUnitModel {
    /// FU pipeline stages per PCU — a result leaves the unit no earlier than this many cycles after entry.
    pub pcu_stages: Fact<usize>,
    /// SIMD lanes per PCU — the width of a vector access and of the cross-lane reduction tree.
    pub pcu_lanes: Fact<usize>,
    /// `(rows, cols)` placement envelope of ONE datapath: what `place_with_target` may place per PCU.
    pub pcu_envelope: Fact<(usize, usize)>,
    /// Datapath width in bits (the SIMD/reconfig metadata and the SAT proof width of the envelope).
    pub datapath_bits: Fact<u32>,
    /// Accumulator capacity per PCU in native tiles.
    pub acc_tiles: Fact<usize>,
    /// Columns of the modeled mesh: the aspect the PCU count is laid out into.
    pub mesh_columns: Fact<usize>,
    /// Ops fused into one spatial pipeline stage.
    pub fusion_window: Fact<usize>,
    /// Nested-counter depth the PCU's own control drives.
    pub counter_nest: Fact<usize>,
    /// The PMU: banks × words, N-buffering, address-datapath stages.
    pub pmu: Fact<PmuSpec>,
    /// Credit-loop latencies. `None` is undeclared, NOT zero: the descriptor then gets the vendor's
    /// double-buffering discipline instead of an invented rate.
    pub credit: Option<Fact<CreditLatency>>,
}

impl RduUnitModel {
    /// The unit model inherited from the open RDU.
    pub fn plasticine_inherited() -> RduUnitModel {
        RduUnitModel {
            pcu_stages: Fact::modeled(6, "not public; inherited from Plasticine [ISCA17] §3.1 — the six-stage PCU pipeline"),
            pcu_lanes: Fact::modeled(16, "not public; inherited from Plasticine [ISCA17] Table 3 — sixteen SIMD lanes"),
            pcu_envelope: Fact::modeled((64, 64), "not public; the per-PCU systolic dimension is not tabulated — a 64×64 placement envelope keeps every bounded-family body placeable"),
            datapath_bits: Fact::modeled(64, "not public; the descriptor's proof width — the PCU's own FU width is not tabulated"),
            acc_tiles: Fact::modeled(8, "not public; the per-PCU accumulator depth is not tabulated — eight native tiles, the Tensix-class DST budget the passes were calibrated on"),
            mesh_columns: Fact::modeled(40, "not public; the mesh aspect is not tabulated — 40 columns lay the 1040 PCUs out as 40×26"),
            fusion_window: Fact::modeled(64, "not public; the RDU fuses a long dataflow pipeline (operator fusion, [HC26]) — 64 ops, at least a whole fused block"),
            counter_nest: Fact::modeled(1, "not public; the counter chain's depth is not tabulated ([ISCA17]) — one counter is certain"),
            pmu: Fact::modeled(
                PmuSpec { banks: 16, words_per_bank: 4096, n_buffers: 2, stages: 4 },
                "not public; inherited from Plasticine [ISCA17] Table 3 — 16 banks × 4096 words, 4 address-datapath stages; double-buffered memory units per the SN50's description [HC26]",
            ),
            credit: None,
        }
    }

    /// The metapipeline unit this model drives: the ISCA'17 PCU as the public toolchain simulates it, at
    /// this depth (result latency = pipeline depth, one-cycle carried-register turnaround).
    pub fn metapipeline_unit(&self) -> UnitModel {
        UnitModel::plasticine_pcu(self.pcu_stages.value)
    }
}

/// The facts of one RDU generation.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RduChipFacts {
    pub chip: RduChip,
    pub part: &'static str,
    /// Peak dense BF16 throughput per RDU, TFLOPS.
    pub peak_bf16_tflops: Fact<u32>,
    /// Peak dense FP8 throughput per RDU, TFLOPS — `None` where the vendor publishes no FP8 figure.
    pub peak_fp8_tflops: Option<Fact<u32>>,
    /// The bf16 roofline the [`SiliconSpec`] publishes, in PFLOPS: per RDU on the SN50; on the SN40L the
    /// 16-socket node figure the descriptor has carried since its lock (16 × 640 TFLOPS).
    pub silicon_peak_bf16_pflops: Fact<f64>,
    /// The capacities, each in the binary unit the vendor counts in ([`BINARY`]): SRAM in MiB, HBM and the largest
    /// DDR in GiB.
    pub sram_mib: Fact<usize>,
    pub hbm_gib: Fact<usize>,
    pub ddr_gib_max: Fact<usize>,
    /// One PMU's scratchpad, KiB — `None` where the vendor publishes no count of them.
    pub pmu_kib: Option<Fact<usize>>,
    /// Peak HBM bandwidth per RDU, GB/s.
    pub hbm_bandwidth_gbps: Fact<f64>,
    /// Peak DDR bandwidth per RDU, GB/s.
    pub ddr_bandwidth_gbps: Fact<f64>,
    /// Aggregate chip-to-chip bandwidth per RDU, TB/s.
    pub p2p_tb_s: Fact<f64>,
    pub node_nm: Fact<u32>,
    pub dies: Fact<u32>,
    /// RDUs per node — the tensor-parallel width a fit report shards across.
    pub chips_per_node: Fact<usize>,
    /// The scale-up fabric, where the vendor tabulates it.
    pub scale_up: Option<Fact<ScaleUp>>,
    pub pcu_count: Fact<usize>,
    pub pmu_count: Fact<usize>,
    /// The matrix path's precisions our fabric places AND witnesses — the only ones a product is advertised in.
    pub witnessed: &'static [Precision],
    /// Chip-supported matrix-path precisions no witness has shipped for — never advertised, named in the envelope.
    pub witness_pending: &'static [Precision],
    /// The precisions each SIMD stage of a PCU computes in: the elementwise and row-reduce datapath beside the matrix
    /// path's systolic array, where norms, softmaxes, routers, recurrent state and long reductions run while the
    /// products stay in [`Self::witnessed`].
    pub simd: Fact<&'static [Precision]>,
    /// The integer type the PCU computes indices and addresses in: the widest integer compute datatype the
    /// part has. Every index a lowering carries onto the chip — a gathered row, an arg-max, a block start —
    /// rides it, and the integer accumulator of the matrix path is this wide; nothing wider exists on the part.
    pub index_datatype: Fact<Precision>,
    pub unit_model: RduUnitModel,
    /// The bibliography keys (`benchmarks/samba_papers.tsv`) of the sources the facts cite: a published fact's citation
    /// opens with its source's key and the figure as that source states it.
    pub cites: &'static [&'static str],
    /// The figures sources state that the facts do not carry.
    pub others: &'static [OtherFigure],
}

/// How every capacity is read: as the vendor counts it, in powers of two. SambaNova states 1040 PMUs of 512 KB and 520 MB
/// of SRAM \[DIAYN26\], which agree only when a MB is 1024 KB; the SN40L paper writes the same figures as MiB, GiB and TiB
/// in its prose and as MB, GB and TB in its table \[SN40L24\]; an HBM2E stack holds 16 GiB.
pub const BINARY: &str = "read in powers of two, as the vendor counts (1040 PMUs × 512 KB = 520 MB [DIAYN26])";

const WITNESSED: &[Precision] = &[Precision::Int8, Precision::Bf16];

/// The SIMD stages' compute formats ([SN40L24]).
const SIMD: &[Precision] = &[Precision::Fp32, Precision::Bf16, Precision::Int32];

/// The HBM stacks an RDU's 64 GiB is laid out as for a model that counts channels: four 16 GiB HBM2E stacks.
pub const HBM_STACKS: usize = 4;

const MODELED_COUNTS_PUBLISHED: &[&str] = &["sites", "acc_tiles", "tile"];
const MODELED_PCU_COUNT: &[&str] = &[
    "sites",
    "acc_tiles",
    "tile",
    "local_mem_bytes",
    "silicon.compute_tiles",
];
const MODELED_PMU_COUNT: &[&str] = &["sites", "acc_tiles", "tile", "silicon.mem_tiles"];
const MODELED_BOTH_COUNTS: &[&str] = &[
    "sites",
    "acc_tiles",
    "tile",
    "local_mem_bytes",
    "silicon.compute_tiles",
    "silicon.mem_tiles",
];

impl RduChipFacts {
    pub fn of(chip: RduChip) -> RduChipFacts {
        chip.facts()
    }

    /// The SN40L: 1040 PCUs and 1040 PMUs of 512 KB at TSMC 5 nm, 638 TFLOPS bf16 a socket, 16 sockets a node, a three-tier
    /// memory of 520 MiB on-chip SRAM, 64 GiB of HBM at 1.6 TB/s and up to 1.5 TiB of DDR at over 100 GB/s. The PCU's
    /// compute datatypes are BF16 / FP32 / INT32 / INT8: INT8 + BF16 are witnessed on the matrix path, where FP32 is
    /// chip-supported and witness-pending; each SIMD stage computes in FP32, BF16 and INT32; INT32 is the index datatype.
    pub fn sn40l() -> RduChipFacts {
        RduChipFacts {
            chip: RduChip::Sn40l,
            part: RduChip::Sn40l.part(),
            peak_bf16_tflops: Fact::published(638, "[HC24] 638 TFLOPS (bf16) a socket; [DIAYN26] one socket delivers 638 TFLOPs of peak BF16"),
            peak_fp8_tflops: None,
            silicon_peak_bf16_pflops: Fact::derived(10.208, "the 16-socket node: [SN50WEB] 16 chips a node × [HC24] 638 TFLOPS = 10,208 TFLOPS = 10.208 PFLOPS"),
            sram_mib: Fact::published(520, "[SN40L24] 520 MiB of on-chip PMU SRAM; read in powers of two, as the vendor counts (1040 PMUs × 512 KB = 520 MB [DIAYN26])"),
            hbm_gib: Fact::published(64, "[SN40L24] 64 GiB of co-packaged HBM; read in powers of two, as the vendor counts (1040 PMUs × 512 KB = 520 MB [DIAYN26])"),
            ddr_gib_max: Fact::published(1536, "[SN40L24] up to 1.5 TiB of DDR DRAM; read in powers of two, as the vendor counts (1040 PMUs × 512 KB = 520 MB [DIAYN26])"),
            pmu_kib: Some(Fact::published(512, "[DIAYN26] each PMU provides 512 KB; read in powers of two, as the vendor counts")),
            hbm_bandwidth_gbps: Fact::published(1600.0, "[DIAYN26] 1.6 TB/s of peak HBM bandwidth a socket; [HC24] 1600 GB/s"),
            ddr_bandwidth_gbps: Fact::published(100.0, "[DIAYN26] over 100 GB/s of peak DDR bandwidth a socket"),
            p2p_tb_s: Fact::derived(0.5, "the SN50 carries four times the network bandwidth of the previous generation [SN50PR], 2 TB/s an RDU [HC26]: 2 TB/s ÷ 4 = 0.5 TB/s"),
            node_nm: Fact::published(5, "[HC24] 5nm TSMC"),
            dies: Fact::published(2, "[DIAYN26] packaged as a two-die socket"),
            chips_per_node: Fact::published(16, "[SN50WEB] chips per node: 16 on the SN40"),
            scale_up: None,
            pcu_count: Fact::published(1040, "[HC24] 1040 PCUs; [DIAYN26] 1040 Pattern Compute Units a socket"),
            pmu_count: Fact::published(1040, "[HC24] 1040 PMUs; [DIAYN26] 1040 Pattern Memory Units a socket"),
            witnessed: WITNESSED,
            witness_pending: &[Precision::Fp32],
            simd: Fact::published(
                SIMD,
                "[SN40L24] each SIMD stage supports common arithmetic, logical, and bit-wise operations in FP32, BF16, and INT32 formats; its Table IV runs BF16 weights and mixed BF16/FP32 activations",
            ),
            index_datatype: Fact::published(
                Precision::Int32,
                "[SN40L24] the PCU supports BF16, FP32, INT32 and INT8 compute data types — INT32 is the widest integer (arXiv:2405.07518)",
            ),
            unit_model: RduUnitModel::plasticine_inherited(),
            cites: &["DIAYN26", "HC24", "HC26", "ISCA17", "SN40L24", "SN50PR", "SN50WEB"],
            others: &[
                OtherFigure { fact: "peak_bf16_tflops", stated: "640 TFLOPS", cite: "[SN50WEB] the SN40 column of the current specification table; the Hot Chips talk and the MLSys paper state 638" },
                OtherFigure { fact: "hbm_bandwidth_gbps", stated: "1.8 TB/s", cite: "[SN40L24] its table; superseded by 1.6 TB/s [DIAYN26]" },
                OtherFigure { fact: "hbm_bandwidth_gbps", stated: "about 2 TB/s", cite: "[SN40L24] its prose; superseded by 1.6 TB/s [DIAYN26]" },
                OtherFigure { fact: "ddr_bandwidth_gbps", stated: "over 200 GB/s", cite: "[SN40L24] its prose and table; superseded by over 100 GB/s [DIAYN26]" },
                OtherFigure { fact: "ddr_gib_max", stated: "Up to 512 GB", cite: "[SN50WEB] DDR5 in the SN40 column; both papers state 1.5 TB [SN40L24], [DIAYN26]" },
            ],
        }
    }

    /// The SN50: TSMC 5 nm, two max-reticle logic dies, 1600 TFLOPS BF16 / 3200 TFLOPS FP8, 432 MB on-chip SRAM, 64 GB
    /// HBM2E, DDR5 up to 512 GB (the datasheet: up to 2 TB), 8 chips per node and 16 per SambaRack, ten 800G ports a chip
    /// (seven make the node's full mesh, two reach the inter-node fabric) carrying 2 TB/s, 400G RoCEv2 scale-out. The
    /// PCU/PMU counts are NOT public ("a sea of PCUs and PMUs") — inherited from the SN40L and labeled modeled, and so
    /// are the HBM and DDR bandwidths. FP8 and FP32 are chip-supported and witness-pending on the matrix path; the SIMD
    /// stages' FP32, BF16 and INT32 are inherited from the SN40L and labeled modeled.
    pub fn sn50() -> RduChipFacts {
        RduChipFacts {
            chip: RduChip::Sn50,
            part: RduChip::Sn50.part(),
            peak_bf16_tflops: Fact::published(1600, "[SN50WEB] 1600 TFLOPS at BF16"),
            peak_fp8_tflops: Some(Fact::published(3200, "[SN50WEB] 3200 TFLOPS at FP8")),
            silicon_peak_bf16_pflops: Fact::published(1.6, "[SN50WEB] 1600 TFLOPS at BF16 per RDU = 1.6 PFLOPS"),
            sram_mib: Fact::published(432, "[SN50WEB] 432 MB of on-chip SRAM; read in powers of two, as the vendor counts (1040 PMUs × 512 KB = 520 MB [DIAYN26])"),
            hbm_gib: Fact::published(64, "[SN50WEB] 64 GB of HBM2E; read in powers of two, as the vendor counts (1040 PMUs × 512 KB = 520 MB [DIAYN26])"),
            ddr_gib_max: Fact::published(512, "[SN50WEB] up to 512 GB of DDR5; read in powers of two, as the vendor counts (1040 PMUs × 512 KB = 520 MB [DIAYN26])"),
            pmu_kib: None,
            hbm_bandwidth_gbps: Fact::modeled(1600.0, "not public; inherited from the SN40L, whose 64 GB of HBM2E moves 1.6 TB/s [DIAYN26] — the specification table (sambanova.ai/products/rdu-ai-chips) lists the same HBM2E on both [SN50WEB]"),
            ddr_bandwidth_gbps: Fact::modeled(100.0, "not public; inherited from the SN40L's over 100 GB/s [DIAYN26] — DDR5 on both [SN50WEB]"),
            p2p_tb_s: Fact::published(2.0, "[HC26] ten 800G Ethernet ports with an aggregate 2 TB/s per RDU"),
            node_nm: Fact::published(5, "[SN50WEB] 5 nm TSMC"),
            dies: Fact::published(2, "[STH26] two max reticle dies for the logic"),
            chips_per_node: Fact::published(8, "[SN50WEB] chips per node: 8"),
            scale_up: Some(Fact::published(
                ScaleUp { chips_per_rack: 16, intra_node_links: 7, inter_node_links: 2, link_gbps: 800, tb_s_per_chip: 2.0, scale_out: "400G RoCEv2" },
                "[HC26] seven integrated 800G Ethernet links make the eight-socket node's full mesh, two more 800G ports reach the inter-node fabric, ten 800G ports carry an aggregate 2 TB/s per RDU, a 400G RoCEv2 NIC scales out; [STH26] 16 RDUs in a rack, two nodes",
            )),
            pcu_count: Fact::modeled(1040, "not public; inherited from SN40L (1040 PCUs [HC24]) — the SN50 describes a sea of PCUs and PMUs without a count"),
            pmu_count: Fact::modeled(1040, "not public; inherited from SN40L (1040 PMUs [HC24]) — the SN50 describes a sea of PCUs and PMUs without a count"),
            witnessed: WITNESSED,
            witness_pending: &[Precision::Fp8, Precision::Fp32],
            simd: Fact::modeled(
                SIMD,
                "not public; inherited from the SN40L ([SN40L24]: each SIMD stage computes in FP32, BF16 and INT32) — the SN50's table names only BF16 and FP8",
            ),
            index_datatype: Fact::modeled(
                Precision::Int32,
                "not public; inherited from the SN40L ([SN40L24]: BF16, FP32, INT32 and INT8 compute data types) — the SN50's table names only BF16 and FP8",
            ),
            unit_model: RduUnitModel::plasticine_inherited(),
            cites: &["DIAYN26", "HC24", "HC26", "ISCA17", "SN40L24", "SN50DS", "SN50WEB", "STH26"],
            others: &[OtherFigure {
                fact: "ddr_gib_max",
                stated: "Up to 2 TB",
                cite: "[SN50DS] the datasheet's specification table; the product page states up to 512 GB [SN50WEB]",
            }],
        }
    }

    // ── Numbers dropping in as data ──────────────────────────────────────────────────────────────────

    pub fn with_pcu_stages(mut self, stages: Fact<usize>) -> RduChipFacts {
        self.unit_model.pcu_stages = stages;
        self
    }

    pub fn with_pmu_banks(mut self, banks: Fact<usize>) -> RduChipFacts {
        let pmu = self.unit_model.pmu;
        self.unit_model.pmu = Fact {
            value: PmuSpec {
                banks: banks.value,
                ..pmu.value
            },
            provenance: banks.provenance,
            cite: banks.cite,
        };
        self
    }

    pub fn with_credit(mut self, credit: Fact<CreditLatency>) -> RduChipFacts {
        self.unit_model.credit = Some(credit);
        self
    }

    pub fn with_pcu_count(mut self, count: Fact<usize>) -> RduChipFacts {
        self.pcu_count = count;
        self
    }

    pub fn with_pmu_count(mut self, count: Fact<usize>) -> RduChipFacts {
        self.pmu_count = count;
        self
    }

    pub fn with_hbm_gib(mut self, hbm: Fact<usize>) -> RduChipFacts {
        self.hbm_gib = hbm;
        self
    }

    // ── Derived figures ──────────────────────────────────────────────────────────────────────────────

    /// The co-packaged HBM's bandwidth, GB/s.
    pub fn hbm_gbps(&self) -> Fact<f64> {
        self.hbm_bandwidth_gbps
    }

    /// The clock the unit model counts cycles in, GHz. Not published for either part: Plasticine's.
    pub fn clock_ghz(&self) -> Fact<f64> {
        Fact::modeled(1.0, "not public; the unit model's cycle basis — Plasticine's 1 GHz (Prabhakar et al., ISCA 2017 §5) — the RDU's clock is not published")
    }

    /// The roofs: the modeled HBM bandwidth over the clock, and the published matrix peaks per RDU. The
    /// vendor publishes no INT8 or on-chip rate, so those resources carry no roof.
    pub fn roofs(&self) -> Roofs {
        let tflops = |f: Fact<u32>| Fact {
            value: Rate::PerSecond(f.value as f64 * 1e12),
            provenance: f.provenance,
            cite: f.cite,
        };
        let hbm = self.hbm_gbps();
        let compute = std::iter::once((Precision::Bf16, tflops(self.peak_bf16_tflops)))
            .chain(self.peak_fp8_tflops.map(|f| (Precision::Fp8, tflops(f))))
            .map(|(precision, ops)| ComputeRoof {
                engine: Engine::Matrix,
                precision,
                scope: Scope::Chip,
                ops,
            })
            .collect();
        Roofs {
            clock_ghz: self.clock_ghz(),
            mem: vec![MemRoof {
                level: MemLevel::Dram,
                mix: StreamMix::Any,
                scope: Scope::Chip,
                unit: Transfer::Bytes,
                rate: Fact {
                    value: Rate::PerSecond(hbm.value * 1e9),
                    provenance: hbm.provenance,
                    cite: hbm.cite,
                },
            }],
            compute,
            sources: self.cites,
        }
    }

    pub fn sram_bytes(&self) -> usize {
        self.sram_mib.value * 1024 * 1024
    }

    /// The on-chip SRAM spread over the PCUs — one site's local-memory budget.
    pub fn local_mem_bytes_per_pcu(&self) -> usize {
        self.sram_bytes() / self.pcu_count.value.max(1)
    }

    /// The modeled mesh: the PCU count laid out into `mesh_columns` columns; a count the aspect cannot
    /// tile is laid out as one row.
    pub fn mesh(&self) -> MeshShape {
        let count = self.pcu_count.value.max(1);
        let cols = self.unit_model.mesh_columns.value.max(1);
        if count % cols == 0 {
            MeshShape {
                x: cols,
                y: count / cols,
            }
        } else {
            MeshShape { x: count, y: 1 }
        }
    }

    /// The `modeled` set of the derived [`SpatialTarget`] — a function of which counts are modeled: a
    /// modeled PCU count makes the mesh, the per-site memory and the silicon's compute-tile figure modeled;
    /// a modeled PMU count makes the mem-tile figure modeled. The mesh aspect, the accumulator depth and the
    /// tile geometry are modeled on every RDU.
    pub fn target_modeled(&self) -> &'static [&'static str] {
        match (self.pcu_count.is_modeled(), self.pmu_count.is_modeled()) {
            (false, false) => MODELED_COUNTS_PUBLISHED,
            (true, false) => MODELED_PCU_COUNT,
            (false, true) => MODELED_PMU_COUNT,
            (true, true) => MODELED_BOTH_COUNTS,
        }
    }

    /// Every modeled fact, by dotted path, sorted — read by reflection over the serialized facts so the
    /// ledger cannot drift from the data.
    pub fn modeled_fields(&self) -> Vec<String> {
        fn walk(v: &serde_json::Value, path: &mut Vec<String>, out: &mut Vec<String>) {
            if let serde_json::Value::Object(m) = v {
                if m.contains_key("value") && m.contains_key("provenance") && m.contains_key("cite")
                {
                    if m["provenance"] == "Modeled" {
                        out.push(path.join("."));
                    }
                    return;
                }
                for (k, c) in m {
                    path.push(k.clone());
                    walk(c, path, out);
                    path.pop();
                }
            }
        }
        let json = serde_json::to_value(self).expect("the chip facts serialize");
        let mut out = Vec::new();
        walk(&json, &mut Vec::new(), &mut out);
        out.sort();
        out
    }

    /// The scalar-fabric envelope of one PCU datapath: what `place_with_target` places onto, gated by the
    /// witnessed precisions and the three memory tiers; the silicon block carries the published tile counts
    /// (or the modeled ones, so labeled on the mesh descriptor).
    pub fn fabric_target(&self) -> FabricTarget {
        let um = &self.unit_model;
        let (rows, cols) = um.pcu_envelope.value;
        FabricTarget {
            name: self.chip.target_name(),
            rows,
            cols,
            width: um.datapath_bits.value,
            alu_ops: ALL_ALU_OPS.iter().copied().collect(),
            func_units: ALL_FUNC_UNITS.iter().copied().collect(),
            mem_depth: um.pmu.value.capacity_words(),
            agu: true,
            simd: Some(LaneTy::I32x2),
            dataflows: [
                Dataflow::Reduction,
                Dataflow::WeightStationary,
                Dataflow::OutputStationary,
            ]
            .into_iter()
            .collect(),
            precisions: self.witnessed.iter().copied().collect(),
            mem: MemHierarchy {
                sram_kib: self.sram_mib.value * 1024,
                hbm_gib: self.hbm_gib.value,
                ddr_gib: self.ddr_gib_max.value,
            },
            fusion_window: um.fusion_window.value,
            silicon: Some(SiliconSpec {
                compute_tiles: self.pcu_count.value,
                mem_tiles: self.pmu_count.value,
                node_nm: self.node_nm.value,
                peak_bf16_pflops: self.silicon_peak_bf16_pflops.value,
            }),
            metapipeline_depth: um.pcu_stages.value,
            banks: um.pmu.value.banks,
            counter_nest: um.counter_nest.value,
        }
    }

    /// The spatial mesh: one site per PCU, the SRAM spread over them, a crossbar switch fabric, token /
    /// valid-ready / barrier handshakes, and the embedded scalar datapath.
    pub fn spatial_target(&self) -> SpatialTarget {
        let scalar = self.fabric_target();
        let um = &self.unit_model;
        let index = self.index_datatype.value;
        let index_bits =
            super::target::precision_int_bits(index).expect("the index datatype is an integer");
        // ★ FP32 ISLANDS, BF16 PRODUCTS. The elementwise and row-reduce units are the PCU's SIMD stages, which compute in
        // their own formats beside the witnessed set, so a norm, a softmax, a router or a recurrent state is held at
        // fp32 there while every product takes the matrix path's bf16 — the mixed BF16/FP32 activations the vendor's
        // own rates run [SN40L24]. The tile datapath also computes in the index datatype, the integer an address is
        // computed in; the matrix dial stays exactly the witnessed set.
        let simd: BTreeSet<Precision> = scalar.precisions.iter().chain(self.simd.value).copied().collect();
        let elementwise: BTreeSet<Precision> = simd.iter().copied().chain([index]).collect();
        SpatialTarget {
            name: self.chip.target_name(),
            sites: self.mesh(),
            site: SiteSpec {
                local_mem_bytes: self.local_mem_bytes_per_pcu(),
                local_mem_opt_in_bytes: None,
                acc_tiles: um.acc_tiles.value,
                fp32_acc_halves_acc_tiles: false,
                movement_engines: 1,
                nocs: 1,
                credit: um.credit.map(|c| c.value),
                local_mem_read_latency: None,
                mem_banks: 1,
                vector_lanes: 1,
                int_compute: Some(index),
                multiplier: None,
            },
            tile: TileShape::TENSIX,
            interconnect: [Interconnect::Crossbar, Interconnect::GlobalMemory]
                .into_iter()
                .collect(),
            handshakes: [
                HandshakeKind::Token,
                HandshakeKind::ValidReady,
                HandshakeKind::Barrier,
            ]
            .into_iter()
            .collect(),
            compute: vec![
                ComputeCap::MatmulTile {
                    precisions: scalar.precisions.clone(),
                    fidelity: Fidelity::Exact,
                    fp32_acc: true,
                    dataflows: [
                        DataflowPolicy::WeightStationary,
                        DataflowPolicy::OutputStationary,
                    ]
                    .into_iter()
                    .collect(),
                    block_api: None,
                    wide_spill_reload: true,
                    acc_in_registers: false,
                    packer_accumulate: PackerAccumulate::No,
                    acc_int_bits: index_bits,
                    max_array: None,
                },
                ComputeCap::Elementwise {
                    precisions: elementwise,
                },
                ComputeCap::RowReduce { precisions: simd.clone() },
                ComputeCap::Scalar(scalar.clone()),
            ],
            dram: scalar.mem,
            precisions: simd,
            noc_rule: None,
            write_ordering: WriteOrdering::Ordered,
            cycle: CycleModel::uncalibrated(),
            modeled: self.target_modeled(),
            roofs: self.roofs(),
            part: None,
        }
    }
}

// ── The II report ────────────────────────────────────────────────────────────────────────────────

/// One compute stage's timing on the unit model.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RduIiRow {
    /// The IR stage id.
    pub stage: usize,
    pub name: String,
    pub op: &'static str,
    /// The stage folds an axis (pays the cross-lane reduction tree).
    pub reduce: bool,
    /// The stage reads a buffer it also writes (a loop-carried dependency through the PMU).
    pub carried: bool,
    pub lanes: usize,
    pub timing: Stage,
}

/// One IR edge's credit loop on the unit model.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RduCreditRow {
    /// The IR edge id.
    pub edge: usize,
    /// Writer and (farthest) reader stage ids.
    pub from: usize,
    pub to: usize,
    /// Mesh hops between them, from the placement.
    pub hops: usize,
    /// Producing the item (the writer's latency when it is a compute stage; the declared fetch latency
    /// when it is a load and the latencies are declared) plus carrying it.
    pub forward_latency: usize,
    /// Carrying the freed credit back: the hops, plus the declared page service and credit return.
    pub backward_latency: usize,
    /// Spatial's writer-to-reader N-buffer rule for a compute→compute edge; the vendor's double-buffering
    /// discipline otherwise.
    pub n_buffer_depth: usize,
    /// The bandwidth-delay solve.
    pub credit_depth: usize,
    /// The deeper of the two.
    pub depth: usize,
}

/// The II view of a placed program on one RDU's unit model — the numbers an RDU engineer asks for first,
/// every one MODELED.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RduIiReport {
    pub chip: RduChip,
    pub program: String,
    pub unit_model: RduUnitModel,
    /// The PMU banking the vector accesses were costed under.
    pub banking: BankingMode,
    /// Whether the credit loops were solved with declared latencies or with the hops alone.
    pub credit_declared: bool,
    pub stages: Vec<RduIiRow>,
    /// The slowest stage's II.
    pub pipeline_ii: usize,
    /// `(writer, reader, depth)` over compute→compute edges, by row index.
    pub buffer_depths: Vec<(usize, usize, usize)>,
    pub credit_depths: Vec<RduCreditRow>,
    pub provenance: &'static str,
}

fn ceil_log2(n: usize) -> usize {
    (usize::BITS - n.max(1).saturating_sub(1).leading_zeros()) as usize
}

fn body_nodes(b: &TensorBody) -> usize {
    match b {
        TensorBody::Operand(_) | TensorBody::Const(_) => 1,
        TensorBody::Add(x, y)
        | TensorBody::Sub(x, y)
        | TensorBody::Mul(x, y)
        | TensorBody::Min(x, y)
        | TensorBody::Max(x, y)
        | TensorBody::FloorDiv(x, y) => 1 + body_nodes(x) + body_nodes(y),
        TensorBody::Unary { arg, .. } | TensorBody::Shr { arg, .. } => 1 + body_nodes(arg),
        TensorBody::Select {
            then, otherwise, ..
        } => 1 + body_nodes(then) + body_nodes(otherwise),
    }
}

/// The vector accesses one compute stage makes per iteration: lane `l` of a map that varies along the
/// innermost axis at stride `s` reads `base + s·l`; an operand constant along that axis is one scalar read.
/// A backward walk (a negative stride) lands in the same banks as the forward one, so the magnitude is what
/// the banking sees. Tile-streaming ops read their operands at unit stride.
fn vector_accesses(kind: &StageKind, n_ins: usize, lanes: usize) -> Vec<AffineAccess> {
    match kind {
        StageKind::Compute(ComputeOp::Generic {
            iters, ins, out, ..
        }) => {
            let inner = iters.last().map(|i| i.var);
            ins.iter()
                .chain(std::iter::once(out))
                .map(|m| {
                    let base = usize::try_from(m.base).unwrap_or(0);
                    let stride = inner
                        .and_then(|v| m.terms.iter().find(|(var, _)| *var == v))
                        .map(|(_, s)| {
                            usize::try_from(*s)
                                .unwrap_or_else(|_| usize::try_from(-*s).unwrap_or(0))
                        })
                        .unwrap_or(0);
                    if stride == 0 {
                        AffineAccess {
                            base,
                            stride: 0,
                            lanes: 1,
                        }
                    } else {
                        AffineAccess {
                            base,
                            stride,
                            lanes,
                        }
                    }
                })
                .collect()
        }
        _ => (0..n_ins.max(1))
            .map(|_| AffineAccess {
                base: 0,
                stride: 1,
                lanes,
            })
            .collect(),
    }
}

/// The II view of `p` on `facts`' unit model. One row per compute-role stage: latency = the PCU pipeline
/// depth (+ the reduction tree, `⌈log₂ lanes⌉`, when the stage folds an axis); resource II from the unit
/// model's occupancies; recurrence II = pipeline depth + register turnaround when the stage reads a buffer
/// it also writes (a spilled partial, an in-place update); memory II from the stage's vector accesses under
/// the PMU's strided banking. Then the pipeline's II, Spatial's N-buffer depths over compute→compute edges,
/// and the credit-loop depth of every IR edge.
pub fn rdu_ii_report(p: &SpatialProgram, facts: &RduChipFacts) -> RduIiReport {
    let um = &facts.unit_model;
    let unit = um.metapipeline_unit();
    let lanes = um.pcu_lanes.value.max(1);
    let pmu = um.pmu.value;
    let banking = strided_banking_for(1);
    let credit = um.credit.map(|c| c.value);

    let compute: Vec<&super::ir::Stage> = p
        .stages
        .iter()
        .filter(|s| s.role == SiteRole::Compute)
        .collect();
    let row_of = |id: super::ir::StageId| compute.iter().position(|s| s.id == id);
    let mut rows = Vec::with_capacity(compute.len());
    for s in &compute {
        let (op, reduce, carried_by_op, cells, resource_ii) = match &s.kind {
            StageKind::Compute(ComputeOp::MatmulTiles { interm, .. }) => (
                "matmul_tiles",
                false,
                interm.is_some(),
                2,
                unit.default_func.1,
            ),
            StageKind::Compute(ComputeOp::Requant { .. }) => {
                ("requant", false, false, 2, unit.default_func.1)
            }
            StageKind::Compute(ComputeOp::Bias { .. }) => {
                ("bias", false, false, 1, unit.default_func.1)
            }
            StageKind::Compute(ComputeOp::Relu) => ("relu", false, false, 1, unit.default_func.1),
            StageKind::Compute(ComputeOp::Generic { iters, body, .. }) => (
                "generic",
                iters
                    .iter()
                    .any(|i| matches!(i.kind, IterKind::Reduce { .. })),
                false,
                body_nodes(body),
                unit.default_func.1,
            ),
            StageKind::Compute(ComputeOp::Scalar(b)) | StageKind::Scalar(b) => (
                "scalar",
                false,
                false,
                b.cells.len(),
                b.cells.iter().map(|c| unit.occupancy(c)).max().unwrap_or(1),
            ),
            StageKind::AllReduce { combine, .. } => match combine.as_ref() {
                ComputeOp::Generic { iters, body, .. } => (
                    "allreduce",
                    iters
                        .iter()
                        .any(|i| matches!(i.kind, IterKind::Reduce { .. })),
                    false,
                    body_nodes(body),
                    unit.default_func.1,
                ),
                _ => ("allreduce", true, false, 1, unit.default_func.1),
            },
            StageKind::Load { .. } | StageKind::Store { .. } | StageKind::Recv { .. } => {
                unreachable!("a movement stage is not a compute row")
            }
        };
        let carried = carried_by_op || s.ins.iter().any(|b| s.outs.contains(b));
        let tree = if reduce {
            ceil_log2(lanes) * unit.tile_latency
        } else {
            0
        };
        let latency = unit.pipeline_depth.max(1) + tree;
        let recurrence_ii = if carried {
            unit.pipeline_depth + unit.register_turnaround
        } else {
            0
        };
        let accesses = vector_accesses(&s.kind, s.ins.len(), lanes);
        rows.push(RduIiRow {
            stage: s.id.idx(),
            name: s.name.clone(),
            op,
            reduce,
            carried,
            lanes,
            timing: Stage {
                name: s.name.clone(),
                resource_ii,
                recurrence_ii,
                latency,
                cells,
                memory_ii: memory_ii(&accesses, banking, &pmu),
            },
        });
    }

    let mut edges: Vec<(usize, usize)> = Vec::new();
    for e in &p.edges {
        if let Some(w) = row_of(e.from) {
            for t in &e.to {
                if let Some(r) = row_of(*t) {
                    if w < r {
                        edges.push((w, r));
                    }
                }
            }
        }
    }
    edges.sort_unstable();
    edges.dedup();
    let mp = Metapipeline {
        stages: rows.iter().map(|r| r.timing.clone()).collect(),
        edges,
    };
    let pipeline_ii = mp.ii();
    let buffer_depths = mp.buffer_depths();

    let site_of = |id: super::ir::StageId| p.placement.as_ref().map(|pl| pl.site_of[id.idx()]);
    let hops_between = |a: super::ir::StageId, b: super::ir::StageId| match (site_of(a), site_of(b))
    {
        (Some(x), Some(y)) => {
            (x.x as isize - y.x as isize).unsigned_abs()
                + (x.y as isize - y.y as isize).unsigned_abs()
        }
        _ => 0,
    };
    let mut credit_depths = Vec::with_capacity(p.edges.len());
    for e in &p.edges {
        let (to, hops) =
            e.to.iter()
                .map(|t| (*t, hops_between(e.from, *t)))
                .max_by_key(|(t, h)| (*h, t.idx()))
                .unwrap_or((e.from, 0));
        let writer_row = row_of(e.from);
        let produce = match writer_row {
            Some(w) => rows[w].timing.latency,
            None => credit.map(|c| c.fetch_tile as usize).unwrap_or(0),
        };
        let forward_latency = produce + hops;
        let backward_latency = hops
            + credit
                .map(|c| (c.service_page + c.credit_return) as usize)
                .unwrap_or(0);
        let n_buffer_depth = match (writer_row, row_of(to)) {
            (Some(w), Some(r)) if w < r => Metapipeline::buffer_depth(w, r),
            _ => pmu.n_buffers.max(1),
        };
        let credit_depth = credit_depth(forward_latency, backward_latency, pipeline_ii);
        credit_depths.push(RduCreditRow {
            edge: e.id.idx(),
            from: e.from.idx(),
            to: to.idx(),
            hops,
            forward_latency,
            backward_latency,
            n_buffer_depth,
            credit_depth,
            depth: credit_depth.max(n_buffer_depth),
        });
    }

    RduIiReport {
        chip: facts.chip,
        program: p.name.clone(),
        unit_model: *um,
        banking,
        credit_declared: credit.is_some(),
        stages: rows,
        pipeline_ii,
        buffer_depths,
        credit_depths,
        provenance: "modeled",
    }
}

/// The precisions the descriptors advertise, as a set.
pub fn witnessed_precisions() -> BTreeSet<Precision> {
    WITNESSED.iter().copied().collect()
}
