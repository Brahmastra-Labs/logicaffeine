//! A decode on one ZCU104: what the board holds, how fast its programmable logic reads it, and what a token of
//! each checkpoint we have costs there.
//!
//! The ZCU104's XCZU7EV keeps 4.98 MB in its block RAMs and UltraRAMs, and reaches the board's 2 GB of DDR4 only
//! through the processing system: the programmable logic reads it over 128-bit AXI HP ports, each moving 16 bytes
//! a PL clock cycle, all behind one DDR4-2400 controller on a 64-bit bus. So the board's decode rate is set by the
//! PL clock and how many ports the fabric drives, an operating point rather than one number, and
//! [`zcu104_points`] names the three that bound it: four ports at 100 MHz, which the FPT'19 study measured
//! saturating on the ZCU102 (the same processing system, the same DDR4 rate and width); three ports at 300 MHz, its
//! best measured read at 75% of the DDR's peak; and the DDR's peak itself.
//!
//! [`zcu104_sizing`] prices every local checkpoint at each point with [`price`]: the floor no implementation on
//! the board can beat, where the weights live, and the term that binds.

use std::path::Path;

use serde::Serialize;

use super::decode::{price, Bind, Chip, DecodeRefusal, DecodeStep, Deployment, GgufHeader, Machine, Tier, TierKind};
use super::part::{PartResources, DS891, UG573};
use super::rdu_chip::{Fact, Provenance};
use crate::codegen_sva::fabric_target::Precision;

pub const UG1267: &str = "UG1267 (v1.1) ZCU104 Evaluation Board User Guide";
pub const FPT19: &str = "pure.manchester.ac.uk/ws/files/137731499/FPT19_paper_14.pdf";
pub const WP486: &str = "WP486 (v1.0.1) Deep Learning with INT8 Optimization on Xilinx Devices";

const ZCU104_SOURCES: &[&str] = &[DS891, UG573, UG1267, FPT19, WP486];

/// Bytes one 128-bit AXI HP port carries in a PL clock cycle.
pub const HP_PORT_BYTES: f64 = 16.0;

/// The DDR4 behind the processing system, as the board guide states it.
pub fn ddr_capacity() -> Fact<f64> {
    Fact::derived(
        2_147_483_648.0,
        "UG1267 (v1.1) ZCU104 Evaluation Board User Guide: \"PS-side memory is a 2 GB, 64-bit wide DDR4 memory system comprised \
         of four 256 Mb x 16 SDRAMs\", each \"4 Gb (256 Mb x 16)\": 4 × 2^32 / 8 bytes",
    )
}

/// What the DDR4 itself moves at most: its transfer rate times its bus width.
pub fn ddr_peak() -> Fact<f64> {
    Fact::derived(
        19.2e9,
        "UG1267 (v1.1) ZCU104 Evaluation Board User Guide: \"Micron MT40A256M16GE-083E (DDR4-2400)\", \"64-bit wide\": 2400 MT/s \
         × 8 bytes",
    )
}

/// One way the programmable logic can run against the DDR: its clock, the HP ports it reads through, and what
/// those ports read.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct PlPoint {
    /// The chip's name at this point.
    pub name: &'static str,
    pub pl_hz: Fact<f64>,
    pub hp_ports: u32,
    /// Bytes per second the fabric reads from the DDR through those ports.
    pub ddr_read: Fact<f64>,
}

/// The operating points that bound a ZCU104 decode, slowest first. The first is the catalog's `zcu104`.
pub fn zcu104_points() -> Vec<PlPoint> {
    vec![
        PlPoint {
            name: "zcu104",
            pl_hz: Fact::modeled(
                100e6,
                "not published for a ZCU104 image of ours: the 100MHz PL clock the FPT'19 study ran its four-port read at, \
                 inherited until a routed image measures one",
            ),
            hp_ports: 4,
            ddr_read: Fact::derived(
                6.4e9,
                "pure.manchester.ac.uk/ws/files/137731499/FPT19_paper_14.pdf (FPT 2019, \"Unexpected Diversity: Quantitative \
                 Memory Analysis for Zynq UltraScale+ Systems\"): on the ZCU102, whose DDR4 is \"2400 MT/s × 64-bit\" as the \
                 ZCU104's is (UG1267 (v1.1)), at 100MHz the four-port configuration \"achieves 6.4 GB/s read throughput\" as \
                 \"all 4 AXI read ports achieve their theoretical maximum of 1.6 GB/s\"",
            ),
        },
        PlPoint {
            name: "zcu104-pl300",
            pl_hz: Fact::modeled(
                300e6,
                "not published for a ZCU104 image of ours: the 300MHz PL clock of the FPT'19 study's best read, inherited until \
                 a routed image measures one",
            ),
            hp_ports: 3,
            ddr_read: Fact::derived(
                14.4e9,
                "pure.manchester.ac.uk/ws/files/137731499/FPT19_paper_14.pdf (FPT 2019): the ZCU102's peak, HP0, HP1 and HP3 at \
                 300MHz with 128 Byte bursts, \"is 75% of the theoretical peak for the DDR memory\": 0.75 × 19.2 GB/s, the \
                 DDR4-2400 × 64-bit the ZCU104 shares (UG1267 (v1.1))",
            ),
        },
        PlPoint {
            name: "zcu104-ddr-peak",
            pl_hz: Fact::modeled(
                300e6,
                "not published for a ZCU104 image of ours: the slowest PL clock at which four 16-byte HP ports reach the DDR's \
                 19.2 GB/s, inherited from that bound until a routed image measures one",
            ),
            hp_ports: 4,
            ddr_read: ddr_peak(),
        },
    ]
}

/// The ZCU104 at its four-port, 100 MHz point: the one the catalog prices.
pub fn zcu104() -> Chip {
    zcu104_at(&zcu104_points()[0])
}

/// The ZCU104 at `p`. Its SRAM is the part's block RAMs and UltraRAMs, read on every port at the PL clock; its DDR
/// is the board's, read at what the point's ports read; its matrix rate is two int8 multiply-accumulates per DSP
/// slice per PL cycle. Facts computed from the clock are as strong as the clock.
pub fn zcu104_at(p: &PlPoint) -> Chip {
    let part = PartResources::xczu7ev();
    let at_clock = |value: f64, cite: &'static str| Fact { value, provenance: Provenance::Derived.max(p.pl_hz.provenance), cite };
    Chip {
        name: p.name,
        tiers: vec![
            Tier {
                kind: TierKind::Sram,
                capacity: Fact::derived(
                    part.block_bytes() as f64,
                    "DS891 (v1.11.1) Zynq UltraScale+ MPSoC Data Sheet: Overview, Table 5: ZU7EV \"Block RAM Blocks 312\", \
                     \"UltraRAM Blocks 96\"; UG573 (v1.9) UltraScale Architecture Memory Resources: 512 x 72 and 4K x 72 bits \
                     each: 312 × 4,608 + 96 × 36,864 bytes",
                ),
                bandwidth: at_clock(
                    part.block_read_bytes_per_cycle() as f64 * p.pl_hz.value,
                    "not published as a rate: DS891 (v1.11.1) Table 5's 312 block RAMs and 96 UltraRAMs read at UG573 (v1.9)'s \
                     port widths, 72 and 2 × 72 bits a cycle, times the point's PL clock, which no ZCU104 image of ours has \
                     measured yet",
                ),
            },
            Tier { kind: TierKind::Ddr, capacity: ddr_capacity(), bandwidth: p.ddr_read },
        ],
        matrix: vec![(
            Precision::Int8,
            at_clock(
                part.dsp.value as f64 * 2.0 * 2.0 * p.pl_hz.value,
                "not published as a rate: DS891 (v1.11.1) Table 5's \"DSP Slices 1,728\", each processing \"two concurrent INT8 \
                 multiply and accumulate (MACC) operations\" (WP486 (v1.0.1) Deep Learning with INT8 Optimization on Xilinx \
                 Devices), two operations each, at the point's PL clock, which no ZCU104 image of ours has measured yet",
            ),
        )],
        stage_overhead: None,
        link: None,
        power: None,
        sources: ZCU104_SOURCES,
    }
}

// ── The sizing ──────────────────────────────────────────────────────────────────────────────────────

/// The checkpoints the sizing prices: every local one, in `benchmarks/silicon/decode/headers/`.
pub const ZCU104_MODELS: &[&str] = &["qwen3.5-0.8b-q8_0", "gemma-3-1b-it-q8_0", "llama-3.2-1b-instruct-q8_0", "qwen3-8b-q8_0"];

/// The context the sizing's KV caches have reached.
pub const ZCU104_CTX: u64 = 512;

/// One operating point, as the sizing states it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PointSummary {
    pub name: &'static str,
    pub pl_mhz: f64,
    pub pl_provenance: Provenance,
    pub hp_ports: u32,
    pub ddr_read_gb_s: f64,
    pub ddr_read_provenance: Provenance,
    pub sram_read_gb_s: f64,
    pub int8_tops: f64,
}

/// A checkpoint at one operating point.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SizingRow {
    pub point: &'static str,
    /// No decode on the board at this point is faster.
    pub floor_tok_s: f64,
    pub binds: Bind,
    /// Of the held bytes, the share the on-chip RAM keeps.
    pub on_chip_share: f64,
    pub provenance: Provenance,
}

/// Whether a checkpoint fits the board.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub enum Fit {
    Resident { rows: Vec<SizingRow> },
    NotResident { needed: f64, capacity: f64 },
}

/// One checkpoint on the board.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ModelSizing {
    pub model: String,
    pub file_bytes: u64,
    /// Whether its head reads the embedding table (no `output.weight`).
    pub tied_head: bool,
    pub held_bytes: f64,
    pub weight_bytes_per_token: f64,
    pub state_bytes_per_token: f64,
    pub fit: Fit,
}

/// Every local checkpoint, priced on the ZCU104 at each operating point.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Zcu104Sizing {
    pub board: &'static str,
    pub part: &'static str,
    pub ctx: u64,
    pub on_chip_bytes: f64,
    pub ddr_bytes: f64,
    pub points: Vec<PointSummary>,
    pub models: Vec<ModelSizing>,
}

/// The sizing, read from the header ledgers in `decode_dir`.
pub fn zcu104_sizing(decode_dir: &Path) -> Result<Zcu104Sizing, String> {
    let points = zcu104_points();
    let chips: Vec<Chip> = points.iter().map(zcu104_at).collect();
    let summaries = points
        .iter()
        .zip(&chips)
        .map(|(p, c)| PointSummary {
            name: p.name,
            pl_mhz: p.pl_hz.value / 1e6,
            pl_provenance: p.pl_hz.provenance,
            hp_ports: p.hp_ports,
            ddr_read_gb_s: p.ddr_read.value / 1e9,
            ddr_read_provenance: p.ddr_read.provenance,
            sram_read_gb_s: c.tiers[0].bandwidth.value / 1e9,
            int8_tops: c.matrix_rate(Precision::Int8).map_or(0.0, |f| f.value / 1e12),
        })
        .collect();
    let deployment = Deployment::as_stored(ZCU104_CTX);
    let mut models = Vec::new();
    for &name in ZCU104_MODELS {
        let path = decode_dir.join("headers").join(format!("{name}.json"));
        let header = GgufHeader::read_ledger(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let step = DecodeStep::of(&header, &deployment).map_err(|e| format!("{name}: {e:?}"))?;
        let mut rows = Vec::new();
        let mut refused = None;
        for c in &chips {
            match price(&step, &Machine::of(c.clone(), 1), Precision::Int8) {
                Ok(cost) => {
                    let on_chip = cost.residency.iter().find(|(k, _)| *k == TierKind::Sram).map_or(0.0, |(_, b)| *b);
                    rows.push(SizingRow {
                        point: c.name,
                        floor_tok_s: cost.floor_tokens_per_s(),
                        binds: cost.floor_binds,
                        on_chip_share: on_chip / step.held_bytes(),
                        provenance: cost.provenance,
                    });
                }
                Err(DecodeRefusal::NotResident { needed, capacity }) => refused = Some((needed, capacity)),
                Err(e) => return Err(format!("{name} on {}: {e:?}", c.name)),
            }
        }
        let fit = match refused {
            Some((needed, capacity)) if rows.is_empty() => Fit::NotResident { needed, capacity },
            None => Fit::Resident { rows },
            Some(_) => return Err(format!("{name}: resident at some points and not others, on one board's memory")),
        };
        models.push(ModelSizing {
            model: name.to_string(),
            file_bytes: header.file_bytes,
            tied_head: !header.tensors.iter().any(|t| t.name == "output.weight"),
            held_bytes: step.held_bytes(),
            weight_bytes_per_token: step.weight_bytes(),
            state_bytes_per_token: step.state_read_bytes() + step.state_write_bytes(),
            fit,
        });
    }
    let part = PartResources::xczu7ev();
    Ok(Zcu104Sizing {
        board: "ZCU104 (XCZU7EV-2FFVC1156)",
        part: part.part,
        ctx: ZCU104_CTX,
        on_chip_bytes: part.block_bytes() as f64,
        ddr_bytes: ddr_capacity().value,
        points: summaries,
        models,
    })
}
