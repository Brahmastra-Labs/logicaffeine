//! The **Plasticine** mesh descriptor — the open RDU (Prabhakar et al., ISCA 2017) and its SARA/Capstan-era
//! successor configuration — as a physically-described target that wraps the certified [`FabricTarget`] seam.
//!
//! [`FabricTarget::plasticine_shaped`] models ONE PCU datapath (the placement envelope the certified placer
//! fills); this descriptor adds what the papers publish about the MESH — how many PCUs/PMUs in what array, the
//! PCU's lanes / stages / pipeline registers, the PMU's banks / capacity / stages, the DRAM channels and
//! bandwidth, clock, area, power, fp32 roofline — every value cited at its constructor, and `Deref`s to the
//! wrapped `FabricTarget` so every existing `&FabricTarget` API (placement, certification, capability gates)
//! accepts it unchanged. Fields a paper does not publish are `None`, never guessed.
//!
//! P0 of the Plasticine lane: descriptor + backend. The metapipeline II model, PMU banking model, mesh
//! placement and the Spatial/PIR emitter follow (P1–P4); until the emitter lands the backend hands back the
//! certified neutral placement and says so.

use std::ops::Deref;

use super::backend::{serialize_grid, BackendArtifact, TargetBackend};
use super::fabric_grid::Grid;
use super::fabric_target::FabricTarget;
use super::fabric_target::Precision;
use crate::spatial::rdu_chip::{Fact, Provenance, RduChip, HBM_STACKS};
use crate::spatial::roofs::{
    ComputeRoof, Engine, MemLevel, MemRoof, Rate, Roofs, Scope, StreamMix, Transfer,
};

const ISCA17: &str = "Prabhakar et al., Plasticine, ISCA 2017 (DOI 10.1145/3079856.3080256)";
const SARA: &str = "Zhang, Scaling a Reconfigurable Dataflow Accelerator (SARA), Stanford 2020";
const TOOLCHAIN_LEDGER: &str = "benchmarks/silicon/plasticine_toolchain.json";

/// The lithographic reticle limit, 26 mm × 33 mm — what "a max-reticle die" measures (modeled area basis).
const RETICLE_LIMIT_MM2: f64 = 858.0;

/// Off-chip memory of a Plasticine configuration: channel count, kind, and published peak bandwidth.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct DramSpec {
    pub channels: usize,
    pub kind: &'static str,
    /// Published peak bandwidth in GB/s.
    pub gbps: f64,
}

/// A published Plasticine mesh configuration.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct PlasticineTarget {
    /// The certified per-PCU datapath descriptor (capabilities, precision, placement envelope).
    pub base: FabricTarget,
    /// Checkerboard array dimensions (PCUs and PMUs alternate 1:1).
    pub pcu_rows: usize,
    pub pcu_cols: usize,
    /// SIMD lanes per PCU.
    pub pcu_lanes: usize,
    /// FU pipeline stages per PCU.
    pub pcu_stages: usize,
    /// Pipeline registers per stage.
    pub pcu_regs_per_stage: usize,
    /// Scalar / vector / control ports per PCU (in, out).
    pub pcu_scalar_io: (usize, usize),
    pub pcu_vector_io: (usize, usize),
    /// PMU vector ports (in, out) — how many vector nets a scratchpad can sink / source at once.
    pub pmu_vector_io: (usize, usize),
    pub pcu_control_io: Option<(usize, usize)>,
    /// Counter-chain depth per PCU, when the paper tabulates it.
    pub pcu_counters: Option<usize>,
    /// Scratchpad banks per PMU, PMU capacity in KiB, PMU address-datapath stages.
    pub pmu_banks: usize,
    pub pmu_kib: usize,
    pub pmu_stages: usize,
    pub dram: DramSpec,
    pub clock_ghz: f64,
    pub area_mm2: f64,
    pub power_w: Option<f64>,
    /// Published peak fp32 throughput (the chip's native format; it has no bf16 datapath).
    pub peak_fp32_tflops: f64,
    /// Where every number above comes from.
    pub cite: &'static str,
    /// What vouches for the three rates above — the DRAM bandwidth, the clock and the fp32 peak — read by
    /// [`Self::roofs`]. The numbers live in their fields; this holds only their provenance.
    #[serde(skip)]
    rates: RateFacts,
}

/// The provenance of a configuration's DRAM bandwidth (with the access mix it holds for), clock and fp32
/// peak, and the sources their citations name.
#[derive(Clone, Copy, Debug, PartialEq)]
struct RateFacts {
    dram: (Provenance, StreamMix, &'static str),
    clock: (Provenance, &'static str),
    peak_fp32: (Provenance, &'static str),
    sources: &'static [&'static str],
}

impl Deref for PlasticineTarget {
    type Target = FabricTarget;
    fn deref(&self) -> &FabricTarget {
        &self.base
    }
}

impl PlasticineTarget {
    /// ISCA 2017 Plasticine (Table 3 + §3–4): 16×8 checkerboard (64 PCU + 64 PMU), PCU 16 lanes × 6 stages ×
    /// 6 registers, 6/5 scalar and 3/3 vector ports; PMU 16 banks × 16 KB = 256 KB, 4 stages, 3 vector in /
    /// 1 vector out; 4 × DDR3-1600 =
    /// 51.2 GB/s; 1 GHz; 112.8 mm² at 28 nm; 49 W; 12.3 fp32 TFLOPS.
    pub fn isca17() -> PlasticineTarget {
        PlasticineTarget {
            base: FabricTarget::plasticine_shaped(),
            pcu_rows: 16,
            pcu_cols: 8,
            pcu_lanes: 16,
            pcu_stages: 6,
            pcu_regs_per_stage: 6,
            pcu_scalar_io: (6, 5),
            pcu_vector_io: (3, 3),
            pmu_vector_io: (3, 1),
            pcu_control_io: None,
            pcu_counters: None,
            pmu_banks: 16,
            pmu_kib: 256,
            pmu_stages: 4,
            dram: DramSpec { channels: 4, kind: "DDR3-1600", gbps: 51.2 },
            clock_ghz: 1.0,
            area_mm2: 112.8,
            power_w: Some(49.0),
            peak_fp32_tflops: 12.3,
            cite: "Prabhakar et al., Plasticine, ISCA 2017 (DOI 10.1145/3079856.3080256), Tables 3/5, §3, §5",
            rates: RateFacts {
                dram: (Provenance::Published, StreamMix::Any, "Prabhakar et al., Plasticine, ISCA 2017 (DOI 10.1145/3079856.3080256), Tables 3/5, §3, §5: four DDR3-1600 channels, 51.2 GB/s"),
                clock: (Provenance::Published, "Prabhakar et al., Plasticine, ISCA 2017 (DOI 10.1145/3079856.3080256), Tables 3/5, §3, §5: 1 GHz"),
                peak_fp32: (Provenance::Published, "Prabhakar et al., Plasticine, ISCA 2017 (DOI 10.1145/3079856.3080256), Tables 3/5, §3, §5: 12.3 fp32 TFLOPS"),
                sources: &[ISCA17],
            },
        }
    }

    /// The SARA / Capstan-era configuration (Zhang, *Scaling a Reconfigurable Dataflow Accelerator*, Stanford
    /// 2020, Table 3.3 + §4; Rucker et al., Capstan, MICRO 2021, Table 7): 20×20 checkerboard, PCU 16 lanes ×
    /// 6 stages × 8 registers, 6/6 vector, 6/6 scalar, 16/8 control ports; PMU 16 banks × 256 KB, 10 stages;
    /// HBM2 at 1 TB/s; 352 mm² at 28 nm. Peak fp32 scales with the PCU count (200/64 × 12.3).
    pub fn sara_20x20() -> PlasticineTarget {
        PlasticineTarget {
            base: FabricTarget { name: "plasticine-sara-20x20", ..FabricTarget::plasticine_shaped() },
            pcu_rows: 20,
            pcu_cols: 20,
            pcu_lanes: 16,
            pcu_stages: 6,
            pcu_regs_per_stage: 8,
            pcu_scalar_io: (6, 6),
            pcu_vector_io: (6, 6),
            pmu_vector_io: (6, 6),
            pcu_control_io: Some((16, 8)),
            pcu_counters: None,
            pmu_banks: 16,
            pmu_kib: 256,
            pmu_stages: 10,
            dram: DramSpec { channels: 8, kind: "HBM2", gbps: 1000.0 },
            clock_ghz: 1.0,
            area_mm2: 352.0,
            power_w: None,
            peak_fp32_tflops: 12.3 * 200.0 / 64.0,
            cite: "Zhang, Scaling a Reconfigurable Dataflow Accelerator (SARA), Stanford 2020, Table 3.3, §4.2; Capstan, MICRO 2021, Table 7",
            rates: SARA_RATES,
        }
    }

    /// The mesh the public toolchain simulates by default: PIR's `--row=20 --col=20` checkerboard (the SARA
    /// unit shapes) in front of Tungsten's ramulator memory — `DDR4-2133`, 4 channels, `DDR4_8Gb_x16`
    /// (`DDR4-config.cfg` in every generated app). Its effective bandwidth is what Tungsten itself reports on
    /// a memory-bound run (`Average DRAM Read Bandwidth: 86.57 GB/s` on Spatial's `DotProduct_0`, recorded in
    /// `benchmarks/silicon/plasticine_toolchain.json`), used as the roofline of the rows calibrated against it.
    pub fn tungsten_20x20_ddr4() -> PlasticineTarget {
        PlasticineTarget {
            rates: RateFacts {
                dram: (
                    Provenance::Measured,
                    StreamMix::Read,
                    "benchmarks/silicon/plasticine_toolchain.json: Tungsten (ramulator, DDR4-2133, four channels) reports \
                     dram_read_gbps 86.5733 on Spatial's DotProduct_0, a read stream; 86.6 at the descriptor's precision",
                ),
                ..SARA_RATES
            },
            base: FabricTarget { name: "plasticine-tungsten-20x20-ddr4", ..FabricTarget::plasticine_shaped() },
            dram: DramSpec { channels: 4, kind: "DDR4-2133 x16, ramulator (Tungsten default config); effective 86.6 GB/s as Tungsten reports on DotProduct_0", gbps: 86.6 },
            cite: "PIR default mesh (--row=20 --col=20) + Tungsten DDR4-config.cfg (standard DDR4, channels 4, ranks 4, DDR4_2133, DDR4_8Gb_x16); bandwidth = Tungsten's own report on DotProduct_0",
            ..PlasticineTarget::sara_20x20()
        }
    }

    /// The SambaNova RDU — SN40L or SN50 — as a mesh descriptor, derived from
    /// [`RduChipFacts`](crate::spatial::rdu_chip::RduChipFacts): the per-PCU
    /// envelope is the facts' fabric target (`rdu_shaped()` / `sn50_shaped()`, every published number and
    /// its citation there); the checkerboard lays the facts' PCU count out on the unit model's column
    /// aspect (40 × 26 PCUs interleaved 1:1 with PMUs = 52 × 40 units); lanes, stages, banks, address
    /// stages and the counter chain read the unit model; the per-PMU capacity is the published SRAM spread
    /// over the PMU count; the DRAM is the published HBM at a MODELED rate; the clock, the area, the port
    /// counts and the register depth are modeled (cited on [`Self::rdu_provenance`]); power is not public
    /// (`None`); fp32 is not published, so the published bf16 peak stands as the roofline and says so. The
    /// mesh descriptor is therefore modeled through and through — the published facts live on the envelope.
    pub fn rdu(chip: RduChip) -> PlasticineTarget {
        let facts = chip.facts();
        let um = &facts.unit_model;
        let mesh = facts.mesh();
        assert_eq!(
            facts.pcu_count.value, facts.pmu_count.value,
            "the 1:1 checkerboard carries one PMU per PCU; a PMU count that differs is the descriptor refinement an SDK number brings"
        );
        let (kind, cite) = match chip {
            RduChip::Sn40l => (
                "HBM (generation not public)",
                "sambanova.ai / Hot Chips 2024 (hc2024.hotchips.org) / arXiv:2405.07518 for the envelope's published numbers; the mesh descriptor is modeled: the SN40L's 1040 PCUs and 1040 PMUs on the unit model's 40-column aspect, the PCU/PMU shapes inherited from Plasticine (Prabhakar et al., ISCA 2017), 520 MiB spread over 1040 PMUs, the co-packaged HBM at an HBM2E-class modeled rate (its generation and bandwidth are not public), a modeled 1 GHz clock, two reticle-limit dies as the area; fp32 peak not published — the 640 TFLOPS bf16 peak stands as the roofline",
            ),
            RduChip::Sn50 => (
                "HBM2E",
                "sambanova.ai/products/rdu-ai-chips / Hot Chips 2026 / ServeTheHome for the envelope's published numbers; the mesh descriptor is modeled: the PCU/PMU counts inherited from the SN40L (not public for the SN50) on the unit model's 40-column aspect, the PCU/PMU shapes inherited from Plasticine (Prabhakar et al., ISCA 2017), 432 MiB spread over 1040 PMUs, the published 64 GB HBM2E at a modeled JEDEC-rate bandwidth (not public), a modeled 1 GHz clock, two max-reticle dies as the area; fp32 peak not published — the 1600 TFLOPS bf16 peak stands as the roofline",
            ),
        };
        PlasticineTarget {
            base: facts.fabric_target(),
            pcu_rows: 2 * mesh.y,
            pcu_cols: mesh.x,
            pcu_lanes: um.pcu_lanes.value,
            pcu_stages: um.pcu_stages.value,
            pcu_regs_per_stage: 6,
            pcu_scalar_io: (6, 5),
            pcu_vector_io: (3, 3),
            pmu_vector_io: (3, 1),
            pcu_control_io: None,
            pcu_counters: Some(um.counter_nest.value),
            pmu_banks: um.pmu.value.banks,
            pmu_kib: facts.sram_mib.value * 1024 / facts.pmu_count.value.max(1),
            pmu_stages: um.pmu.value.stages,
            dram: DramSpec {
                channels: HBM_STACKS,
                kind,
                gbps: facts.hbm_gbps().value,
            },
            clock_ghz: facts.clock_ghz().value,
            area_mm2: facts.dies.value as f64 * RETICLE_LIMIT_MM2,
            power_w: None,
            peak_fp32_tflops: facts.peak_bf16_tflops.value as f64,
            cite,
            rates: RateFacts {
                dram: (
                    facts.hbm_gbps().provenance,
                    StreamMix::Any,
                    facts.hbm_gbps().cite,
                ),
                clock: (facts.clock_ghz().provenance, facts.clock_ghz().cite),
                peak_fp32: (Provenance::Modeled, rdu_roofline_cite(chip)),
                sources: facts.cites,
            },
        }
    }

    /// The SN40L mesh descriptor ([`Self::rdu`] on `RduChip::Sn40l`).
    pub fn sn40l_rdu() -> PlasticineTarget {
        PlasticineTarget::rdu(RduChip::Sn40l)
    }

    /// The SN50 mesh descriptor ([`Self::rdu`] on `RduChip::Sn50`).
    pub fn sn50_rdu() -> PlasticineTarget {
        PlasticineTarget::rdu(RduChip::Sn50)
    }

    /// Every numeric field of the RDU mesh descriptor with its provenance, by serialized path (the wrapped
    /// envelope's numbers are the facts' own and are ledgered there): the value the descriptor carries, and
    /// whether it is published or modeled, with the source or the gap it fills. Read by `rdu_loop_lane_lock`
    /// through reflection over the serialized descriptor, so a number without a row cannot land.
    pub fn rdu_provenance(chip: RduChip) -> Vec<(&'static str, Fact<f64>)> {
        let t = PlasticineTarget::rdu(chip);
        let facts = chip.facts();
        let counts: &'static str = match chip {
            RduChip::Sn40l => "not public as a mesh aspect; the published 1040 PCUs and 1040 PMUs (Hot Chips 2024 / arXiv:2405.07518) laid out on the unit model's modeled 40-column aspect, PMUs interleaved 1:1",
            RduChip::Sn50 => "not public; the SN50 describes a sea of PCUs and PMUs without a count — the SN40L's 1040 + 1040 inherited, laid out on the unit model's modeled 40-column aspect, PMUs interleaved 1:1",
        };
        let pmu_kib: &'static str = match chip {
            RduChip::Sn40l => "modeled: the published 520 MiB on-chip SRAM (arXiv:2405.07518) spread over the published 1040 PMUs — the per-PMU capacity is not tabulated",
            RduChip::Sn50 => "modeled: the published 432 MB on-chip SRAM (sambanova.ai/products/rdu-ai-chips) spread over the modeled 1040 PMUs — the per-PMU capacity is not public",
        };
        let hbm = facts.hbm_gbps().cite;
        let roofline = rdu_roofline_cite(chip);
        let area: &'static str = match chip {
            RduChip::Sn40l => "not public; modeled from the published two dies (Hot Chips 2024) at the 26 mm × 33 mm reticle limit (858 mm² each)",
            RduChip::Sn50 => "not public; modeled from the published two max-reticle logic dies (Hot Chips 2026) at the 26 mm × 33 mm reticle limit (858 mm² each)",
        };
        let um = &facts.unit_model;
        let m = |v: f64, cite: &'static str| Fact::modeled(v, cite);
        vec![
            ("pcu_rows", m(t.pcu_rows as f64, counts)),
            ("pcu_cols", m(t.pcu_cols as f64, counts)),
            ("pcu_lanes", m(t.pcu_lanes as f64, um.pcu_lanes.cite)),
            ("pcu_stages", m(t.pcu_stages as f64, um.pcu_stages.cite)),
            ("pcu_regs_per_stage", m(t.pcu_regs_per_stage as f64, "not public; inherited from Plasticine (Prabhakar et al., ISCA 2017) Table 3 — six pipeline registers per stage")),
            ("pcu_scalar_io.0", m(t.pcu_scalar_io.0 as f64, "not public; inherited from Plasticine (Prabhakar et al., ISCA 2017) Table 3 — 6 scalar inputs per PCU")),
            ("pcu_scalar_io.1", m(t.pcu_scalar_io.1 as f64, "not public; inherited from Plasticine (Prabhakar et al., ISCA 2017) Table 3 — 5 scalar outputs per PCU")),
            ("pcu_vector_io.0", m(t.pcu_vector_io.0 as f64, "not public; inherited from Plasticine (Prabhakar et al., ISCA 2017) Table 3 — 3 vector inputs per PCU")),
            ("pcu_vector_io.1", m(t.pcu_vector_io.1 as f64, "not public; inherited from Plasticine (Prabhakar et al., ISCA 2017) Table 3 — 3 vector outputs per PCU")),
            ("pmu_vector_io.0", m(t.pmu_vector_io.0 as f64, "not public; inherited from Plasticine (Prabhakar et al., ISCA 2017) Table 3 — 3 vector inputs per PMU")),
            ("pmu_vector_io.1", m(t.pmu_vector_io.1 as f64, "not public; inherited from Plasticine (Prabhakar et al., ISCA 2017) Table 3 — 1 vector output per PMU")),
            ("pcu_counters", m(t.pcu_counters.unwrap_or(0) as f64, um.counter_nest.cite)),
            ("pmu_banks", m(t.pmu_banks as f64, um.pmu.cite)),
            ("pmu_kib", m(t.pmu_kib as f64, pmu_kib)),
            ("pmu_stages", m(t.pmu_stages as f64, um.pmu.cite)),
            ("dram.channels", m(t.dram.channels as f64, hbm)),
            ("dram.gbps", m(t.dram.gbps, hbm)),
            ("clock_ghz", m(t.clock_ghz, facts.clock_ghz().cite)),
            ("area_mm2", m(t.area_mm2, area)),
            ("peak_fp32_tflops", m(t.peak_fp32_tflops, roofline)),
        ]
    }

    /// The configuration's roofs: its DRAM bandwidth, clock and fp32 peak, each with what vouches for it.
    pub fn roofs(&self) -> Roofs {
        let r = &self.rates;
        Roofs {
            clock_ghz: Fact {
                value: self.clock_ghz,
                provenance: r.clock.0,
                cite: r.clock.1,
            },
            mem: vec![MemRoof {
                level: MemLevel::Dram,
                mix: r.dram.1,
                scope: Scope::Chip,
                unit: Transfer::Bytes,
                rate: Fact {
                    value: Rate::PerSecond(self.dram.gbps * 1e9),
                    provenance: r.dram.0,
                    cite: r.dram.2,
                },
            }],
            compute: vec![ComputeRoof {
                engine: Engine::Vector,
                precision: Precision::Fp32,
                scope: Scope::Chip,
                ops: Fact {
                    value: Rate::PerSecond(self.peak_fp32_tflops * 1e12),
                    provenance: r.peak_fp32.0,
                    cite: r.peak_fp32.1,
                },
            }],
            sources: r.sources,
        }
    }

    /// PCUs in the mesh (a 1:1 checkerboard holds as many PMUs).
    pub fn pcus(&self) -> usize {
        self.pcu_rows * self.pcu_cols / 2
    }

    /// PMUs in the mesh.
    pub fn pmus(&self) -> usize {
        self.pcu_rows * self.pcu_cols - self.pcus()
    }

    /// Total scratchpad in KiB (PMU count × PMU capacity).
    pub fn scratchpad_kib(&self) -> usize {
        self.pmus() * self.pmu_kib
    }
}

/// The SARA-era configuration's rates: the published HBM2 bandwidth, Plasticine's clock (the SARA tables
/// give none), and Plasticine's fp32 peak scaled to the PCU count.
const SARA_RATES: RateFacts = RateFacts {
    dram: (Provenance::Published, StreamMix::Any, "Zhang, Scaling a Reconfigurable Dataflow Accelerator (SARA), Stanford 2020, Table 3.3, §4.2: HBM2 at 1 TB/s"),
    clock: (Provenance::Modeled, "inherited from Plasticine (Prabhakar et al., ISCA 2017 §5): 1 GHz; the SARA configuration tabulates no clock"),
    peak_fp32: (
        Provenance::Derived,
        "Prabhakar et al., Plasticine, ISCA 2017 (DOI 10.1145/3079856.3080256), Tables 3/5, §3, §5: 12.3 fp32 TFLOPS on 64 PCUs, scaled to \
         the 200 PCUs of the 20 x 20 checkerboard: 12.3 x 200 / 64",
    ),
    sources: &[ISCA17, SARA, TOOLCHAIN_LEDGER],
};

/// What the RDU mesh descriptor's fp32 roofline stands on: no fp32 peak is published, so the bf16 one is.
fn rdu_roofline_cite(chip: RduChip) -> &'static str {
    match chip {
        RduChip::Sn40l => "fp32 peak not published; the published 640 TFLOPS bf16 peak (Hot Chips 2024) used as the roofline — modeled by that substitution",
        RduChip::Sn50 => "fp32 peak not published; the published 1600 TFLOPS bf16 peak (sambanova.ai/products/rdu-ai-chips) used as the roofline — modeled by that substitution",
    }
}

/// The Plasticine backend: certifies a placement TODAY (backend-agnostic SAT proof). Counted loops reach the
/// open toolchain (Spatial → PIR → plastiroute → Tungsten, mirrored at gitlab.com/dataplane-ai) as Spatial
/// source through [`super::spatial_emit`] and `largo emit plasticine`; a placed straight-line [`Grid`] has no
/// Spatial/PIR form yet, so its artifact is the ready-to-bridge neutral placement with the reason stated.
pub struct PlasticineBackend {
    pub target: PlasticineTarget,
}

impl PlasticineBackend {
    pub fn isca17() -> Self {
        PlasticineBackend {
            target: PlasticineTarget::isca17(),
        }
    }
    pub fn sara_20x20() -> Self {
        PlasticineBackend {
            target: PlasticineTarget::sara_20x20(),
        }
    }
    const PENDING_REASON: &'static str =
        "Spatial source is emitted per counted loop by codegen_sva::spatial_emit (P4, `largo emit plasticine`); a placed straight-line Grid has no Spatial/PIR form yet — this is its certified neutral placement, not a Plasticine program";
}

impl TargetBackend for PlasticineBackend {
    fn name(&self) -> &str {
        self.target.name
    }
    fn target(&self) -> &FabricTarget {
        &self.target.base
    }
    fn emit_grid(&self, _name: &str, g: &Grid) -> BackendArtifact {
        BackendArtifact::SdkGated {
            neutral_placement: serialize_grid(g),
            reason: Self::PENDING_REASON,
        }
    }
    fn emit_config_grid(&self, _name: &str, grids: &[&Grid], _cfg_width: u32) -> BackendArtifact {
        let placement = grids
            .iter()
            .map(|g| serialize_grid(g))
            .collect::<Vec<_>>()
            .join("---\n");
        BackendArtifact::SdkGated {
            neutral_placement: placement,
            reason: Self::PENDING_REASON,
        }
    }
}
