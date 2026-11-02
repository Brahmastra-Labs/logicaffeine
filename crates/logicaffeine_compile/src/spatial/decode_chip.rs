//! The decode chip — what an SRAM-resident LOGOS system must achieve to decode one user faster than GPUs can.
//!
//! [`super::decode`] prices a token on any machine. This module turns the question around: given a checkpoint, a
//! GPU deployment's FLOOR (its memory roofline with every overhead at zero — the most a GPU could ever do) and a
//! speedup, it asks what a system of LOGOS chips must achieve, and answers in the units a chip is designed in.
//!
//! ★ AT BATCH ONE ON SRAM, BANDWIDTH IS NOT WHAT BINDS. Weights held in on-die SRAM across enough chips stream in
//! microseconds; what remains is the CHAIN — a token walks every block's dependent stages (norm, projection,
//! mixer, projection, all-reduce, …), and each one costs its on-chip latency (the input vector out to every
//! tile, the partial sums back) plus, where it reduces across chips, its collective's hops. So the answer is a
//! pair of latency budgets: the most one dependent stage may take on chip, and the most one chip-to-chip hop
//! may take, for the system to beat the GPU floor by the speedup. [`demonstrated_stage_latency`] reads the same
//! quantity back out of the vendors' published per-user rates, so a budget can be set against what an SRAM
//! machine has actually shown.
//!
//! The chip is a design point ([`DecodeChipDesign`]): a die area, the share of it that is SRAM, the SRAM per
//! tile and the on-chip network's shape are CHOICES; the node's SRAM density, read width and clock are FACTS
//! ([`Node`]) with sources. Every number the design derives is therefore labeled modeled: a design is not a part.

use serde::Serialize;

use super::decode::{price, Chip, DecodeRefusal, DecodeStep, Link, Machine, Tier, TierKind};
use super::rdu_chip::{Fact, Provenance};
use crate::codegen_sva::fabric_target::Precision;

/// A process node, as an SRAM decode chip sees it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Node {
    pub name: &'static str,
    /// SRAM bits per mm² of die area given to SRAM.
    pub sram_bits_per_mm2: Fact<f64>,
    /// Bytes the SRAM reads per cycle per byte it holds: the macros' read width over their size.
    pub sram_read_per_cycle_per_byte: Fact<f64>,
    pub clock_ghz: Fact<f64>,
    pub sources: &'static [&'static str],
}

const SKY130_SRAM_LEF: &str =
    "open_pdks sky130A libs.ref/sky130_sram_macros/lef/sky130_sram_2kbyte_1rw1r_32x512_8.lef";
const SKY130_SYSTOLIC_SIGNOFF: &str = "benchmarks/silicon/systolic_pipelined_signoff.json";
const TSMC_N5_SRAM: &str = "tomshardware.com/news/no-sram-scaling-implies-on-more-expensive-cpus-and-gpus";
const GROQ_SPEC_SHEET: &str = "groq.com/product-spec-sheet-groqchip-processor";

impl Node {
    /// SkyWater 130 nm, as our own signoff records it.
    pub fn sky130() -> Node {
        let macro_mm2 = 683.1 * 416.54 / 1e6;
        Node {
            name: "sky130",
            sram_bits_per_mm2: Fact::derived(
                2048.0 * 8.0 / macro_mm2,
                "open_pdks sky130A libs.ref/sky130_sram_macros/lef/sky130_sram_2kbyte_1rw1r_32x512_8.lef: \"SIZE 683.1 BY \
                 416.54\" for 2 KiB (\"2kbyte\"), the largest sky130 SRAM macro; 16,384 bits over 0.28454 mm², periphery \
                 included",
            ),
            sram_read_per_cycle_per_byte: Fact::derived(
                4.0 / 2048.0,
                "open_pdks sky130A libs.ref/sky130_sram_macros/lef/sky130_sram_2kbyte_1rw1r_32x512_8.lef: \"1rw1r_32x512\", a \
                 32-bit read port on 512 words — four bytes per cycle from each 2 KiB",
            ),
            clock_ghz: Fact::measured(
                1.0 / 19.5,
                "benchmarks/silicon/systolic_pipelined_signoff.json: systolic_pipe16_realbist \"clock_period_ns\": 19.5, \
                 the largest array our OpenLane sky130 signoff has closed",
            ),
            sources: &[SKY130_SRAM_LEF, SKY130_SYSTOLIC_SIGNOFF],
        }
    }

    /// TSMC N5. Its SRAM density is published; its read width and clock for an SRAM dataflow die are inherited
    /// from the one such die whose figures are published, the 14 nm GroqChip — conservative at 5 nm.
    pub fn n5() -> Node {
        Node {
            name: "n5",
            sram_bits_per_mm2: Fact::published(
                31.8 * 1024.0 * 1024.0,
                "tomshardware.com/news/no-sram-scaling-implies-on-more-expensive-cpus-and-gpus (reporting WikiChip on TSMC's \
                 IEDM 2022 paper): N5's \"0.021 µm^² SRAM bitcell (which roughly translates to 31.8 Mib/mm^²)\"",
            ),
            sram_read_per_cycle_per_byte: Fact::modeled(
                80e12 / (230e6 * 0.9e9),
                "not published for an N5 SRAM dataflow die; inherited from groq.com/product-spec-sheet-groqchip-processor: \
                 \"230 MB SRAM per chip\", \"Up to 80 TB/s on-die memory bandwidth\", \"@900 MHz\" — 80e12 / (230e6 × 0.9e9) \
                 bytes read per cycle per byte held",
            ),
            clock_ghz: Fact::modeled(
                0.9,
                "not published for an N5 SRAM dataflow die; inherited from groq.com/product-spec-sheet-groqchip-processor: \
                 \"(INT8, FP16 @900 MHz)\" at 14nm",
            ),
            sources: &[TSMC_N5_SRAM, GROQ_SPEC_SHEET],
        }
    }

    pub fn facts(&self) -> Vec<(String, Provenance, &'static str)> {
        vec![
            ("sram_bits_per_mm2".into(), self.sram_bits_per_mm2.provenance, self.sram_bits_per_mm2.cite),
            (
                "sram_read_per_cycle_per_byte".into(),
                self.sram_read_per_cycle_per_byte.provenance,
                self.sram_read_per_cycle_per_byte.cite,
            ),
            ("clock_ghz".into(), self.clock_ghz.provenance, self.clock_ghz.cite),
        ]
    }
}

/// How a dependent stage's vector reaches every tile and its partial sums come back.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum OnChipNetwork {
    /// A 2-D mesh: out across its diameter and back.
    Mesh,
    /// A reduction / broadcast tree: out and back in `⌈log₂ tiles⌉` levels.
    Tree,
}

/// A decode chip design point: the choices, over a node's facts.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DecodeChipDesign {
    pub node: Node,
    pub die_mm2: f64,
    /// The share of the die that is SRAM array.
    pub sram_fraction: f64,
    /// SRAM per tile, bytes.
    pub tile_bytes: f64,
    pub network: OnChipNetwork,
    /// Cycles one on-chip network hop takes: one is a registered link.
    pub hop_cycles: f64,
    /// UCIe modules each chip's collectives stripe across.
    pub link_modules: u32,
}

/// UCIe's published die-to-die figures (UCIe Consortium, Hot Chips 2023 tutorial "UCIe overview and usage models").
const UCIE_TUTORIAL: &str = "hc2023.hotchips.org/assets/program/tutorials/ucie/UCIe overview and usage models.pdf";

/// The die-to-die link a decode system's chips connect with, at a hop latency the requirement solves for: `modules`
/// UCIe standard-package modules striped together.
pub fn ucie_link(hop_latency_s: f64, modules: u32) -> Link {
    Link {
        bandwidth: Some(Fact::derived(
            64e9 * modules as f64,
            "hc2023.hotchips.org/assets/program/tutorials/ucie/UCIe overview and usage models.pdf: \"B/W per module/ dir: \
             64 GB/s Std, 256 GB/s Adv\"; the standard-package module rate times the modules a chip stripes across",
        )),
        hop_latency: Some(Fact::modeled(
            hop_latency_s,
            "not published for a chip-to-chip collective hop; the latency under test (UCIe's own \"Latency (Tx + Rx) < 2ns\" \
             is a die-to-die link inside one package)",
        )),
    }
}

impl DecodeChipDesign {
    /// A dataflow die at `node`: half its area SRAM, 256 KiB tiles on a registered reduction tree, sixteen UCIe
    /// modules to its neighbours.
    pub fn at(node: Node, die_mm2: f64) -> DecodeChipDesign {
        DecodeChipDesign {
            node,
            die_mm2,
            sram_fraction: 0.5,
            tile_bytes: 256.0 * 1024.0,
            network: OnChipNetwork::Tree,
            hop_cycles: 1.0,
            link_modules: 16,
        }
    }

    /// The design's chip-to-chip link at `hop_latency_s`.
    pub fn link(&self, hop_latency_s: f64) -> Link {
        ucie_link(hop_latency_s, self.link_modules)
    }

    pub fn sram_bytes(&self) -> f64 {
        self.die_mm2 * self.sram_fraction * self.node.sram_bits_per_mm2.value / 8.0
    }

    pub fn tiles(&self) -> f64 {
        (self.sram_bytes() / self.tile_bytes).max(1.0)
    }

    /// Hops one dependent stage makes on chip: its vector out to every tile and its partial sums back.
    pub fn stage_hops(&self) -> f64 {
        let t = self.tiles();
        match self.network {
            OnChipNetwork::Mesh => 2.0 * 2.0 * (t.sqrt().ceil() - 1.0),
            OnChipNetwork::Tree => 2.0 * t.log2().ceil(),
        }
    }

    /// Seconds one dependent stage spends on the network.
    pub fn stage_latency_s(&self) -> f64 {
        self.stage_hops() * self.hop_cycles / (self.node.clock_ghz.value * 1e9)
    }

    /// The chip, with a chip-to-chip link and a per-stage overhead of `stage_s` (the design's own, or a budget
    /// under test).
    pub fn chip_with(&self, stage_s: f64, link: Link) -> Chip {
        let bytes = self.sram_bytes();
        let bandwidth = bytes * self.node.sram_read_per_cycle_per_byte.value * self.node.clock_ghz.value * 1e9;
        let modeled = |value: f64, cite: &'static str| Fact::modeled(value, cite);
        Chip {
            name: "logos-decode",
            tiers: vec![Tier {
                kind: TierKind::Sram,
                capacity: modeled(bytes, "not published: a LOGOS design point — die × SRAM share × the node's SRAM density"),
                bandwidth: modeled(bandwidth, "not published: a LOGOS design point — SRAM held × the node's read per cycle per byte × its clock"),
            }],
            matrix: [Precision::Int8, Precision::Fp8]
                .into_iter()
                .map(|p| {
                    // One multiply-add per weight byte read: the arithmetic a batch-1 decode asks of each byte, so
                    // compute is sized to the SRAM and never binds.
                    (p, modeled(2.0 * bandwidth, "not published: a LOGOS design point — one multiply-add per SRAM byte read per cycle, inherited from the decode's own arithmetic intensity"))
                })
                .collect(),
            stage_overhead: Some(modeled(stage_s, "not published: a LOGOS design point — the on-chip network's hops per dependent stage over the node's clock, or a budget under test")),
            link: Some(link),
            power: None,
            sources: &[UCIE_TUTORIAL],
        }
    }
}

/// What a LOGOS system must achieve to beat a GPU deployment's floor by a speedup on one checkpoint.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Requirement {
    pub node: &'static str,
    pub die_mm2: f64,
    pub network: OnChipNetwork,
    /// The fewest chips whose SRAM holds the deployment.
    pub chips: u32,
    pub sram_per_chip_bytes: f64,
    /// Tokens per second the system must reach: the GPU floor times the speedup.
    pub target_tok_s: f64,
    /// The system's own floor at those chips: its memory roofline.
    pub floor_tok_s: f64,
    /// Dependent stages per token, and those that reduce across chips.
    pub stages: usize,
    pub collectives: usize,
    /// The most one dependent stage may take on chip with chip-to-chip hops free, or `None` if even free
    /// stages miss the target.
    pub stage_budget_s: Option<f64>,
    /// The design's own on-chip stage latency.
    pub design_stage_s: f64,
    /// The most one chip-to-chip hop may take with the design's own stage latency, or `None` if the design's
    /// stage latency alone misses the target.
    pub hop_budget_s: Option<f64>,
    /// Predicted tokens per second at the design's stage latency, by chip-to-chip hop latency.
    pub by_hop_latency: Vec<(f64, f64)>,
}

/// The largest `x` in `[0, hi]` with `ok(x)`, for `ok` true at 0 and false past some point; `None` if false at 0.
fn largest(ok: impl Fn(f64) -> bool, hi: f64) -> Option<f64> {
    if !ok(0.0) {
        return None;
    }
    if ok(hi) {
        return Some(hi);
    }
    let (mut lo, mut up) = (0.0, hi);
    for _ in 0..200 {
        let mid = 0.5 * (lo + up);
        if ok(mid) {
            lo = mid;
        } else {
            up = mid;
        }
    }
    Some(lo)
}

/// The fewest chips of `chip` that hold `step`.
pub fn fewest_chips(step: &DecodeStep, chip: &Chip, precision: Precision) -> Result<u32, DecodeRefusal> {
    let per_chip: f64 = chip.tiers.iter().map(|t| t.capacity.value).sum();
    let mut n = (step.held_bytes() / per_chip).ceil().max(1.0) as u32;
    loop {
        match price(step, &Machine::of(chip.clone(), n), precision) {
            Ok(_) => return Ok(n),
            Err(DecodeRefusal::NotResident { .. }) if n < 1 << 20 => n += 1,
            Err(e) => return Err(e),
        }
    }
}

/// What `design` must achieve to decode `step` at `target_tok_s`.
pub fn requirement(step: &DecodeStep, design: &DecodeChipDesign, target_tok_s: f64, precision: Precision) -> Result<Requirement, DecodeRefusal> {
    let own = design.stage_latency_s();
    let chips = fewest_chips(step, &design.chip_with(own, design.link(0.0)), precision)?;
    let time = |stage_s: f64, hop_s: f64| -> f64 {
        let cost = price(step, &Machine::of(design.chip_with(stage_s, design.link(hop_s)), chips), precision)
            .expect("the fewest chips hold it");
        cost.time_s.expect("every term of a design point is declared")
    };
    let target_s = 1.0 / target_tok_s;
    let base = price(step, &Machine::of(design.chip_with(own, design.link(0.0)), chips), precision)?;
    let stage_budget_s = largest(|s| time(s, 0.0) <= target_s, 1e-3);
    let hop_budget_s = largest(|h| time(own, h) <= target_s, 1e-3);
    let by_hop_latency = [2e-9, 50e-9, 100e-9, 250e-9, 500e-9, 1e-6]
        .into_iter()
        .map(|h| (h, 1.0 / time(own, h)))
        .collect();
    Ok(Requirement {
        node: design.node.name,
        die_mm2: design.die_mm2,
        network: design.network,
        chips,
        sram_per_chip_bytes: design.sram_bytes(),
        target_tok_s,
        floor_tok_s: base.floor_tokens_per_s(),
        stages: step.stages.len(),
        collectives: base.collectives,
        stage_budget_s,
        design_stage_s: own,
        hop_budget_s,
        by_hop_latency,
    })
}

/// The per-stage latency a published per-user rate implies: everything a token took beyond its memory floor,
/// spread over its dependent stages. A rate this machine reached is a latency it has shown.
pub fn demonstrated_stage_latency(step: &DecodeStep, machine: &Machine, published_tok_s: f64, precision: Precision) -> Result<f64, DecodeRefusal> {
    let cost = price(step, machine, precision)?;
    Ok(((1.0 / published_tok_s) - cost.floor_s).max(0.0) / step.stages.len() as f64)
}

// ── The report ──────────────────────────────────────────────────────────────────────────────────────

/// A GPU deployment's floor on one checkpoint.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct GpuFloor {
    pub machine: &'static str,
    pub chips: u32,
    pub floor_tok_s: f64,
    pub binds: super::decode::Bind,
}

/// A vendor's published per-user rate, and the per-stage latency it implies.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Demonstrated {
    pub label: String,
    pub machine: &'static str,
    pub chips: u32,
    pub published_tok_s: f64,
    pub stage_latency_s: f64,
}

/// One requirement: a GPU baseline, the speedup over its floor, the design asked to reach it, and the answer.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RequirementRow {
    pub baseline: String,
    pub baseline_floor_tok_s: f64,
    pub speedup: f64,
    pub requirement: Requirement,
}

/// Everything the report says about one checkpoint.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ModelReport {
    pub model: String,
    pub weight_bytes_per_token: f64,
    pub held_bytes: f64,
    pub stages: usize,
    pub gpu_floors: Vec<GpuFloor>,
    pub demonstrated: Vec<Demonstrated>,
    pub requirements: Vec<RequirementRow>,
}

/// The decode-chip report: what a LOGOS system must achieve, checkpoint by checkpoint.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DecodeChipReport {
    pub what: &'static str,
    pub deployment: super::decode::Deployment,
    pub speedup: f64,
    pub nodes: Vec<Node>,
    pub models: Vec<ModelReport>,
}

/// The checkpoints the report prices, by header ledger name.
pub const REPORT_MODELS: &[&str] = &[
    "llama-3.1-8b-instruct-q8_0",
    "llama-3.1-70b-instruct-q4_k_m",
    "qwen3.8-27b-q8_0",
];

/// The report over the header and published ledgers under `decode_dir` (`benchmarks/silicon/decode`).
pub fn report(decode_dir: &std::path::Path) -> Result<DecodeChipReport, String> {
    use super::decode::{Deployment, GgufHeader, WeightBytes};
    let precision = Precision::Fp8;
    let speedup = 5.0;
    // Both sides at the same numerics: fp8 weights, an f16 KV cache at a 4096-token context.
    let deployment = Deployment { weights: WeightBytes::PerWeight(1.0), ..Deployment::as_stored(4096) };
    let published: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(decode_dir.join("published.json")).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let designs = [
        DecodeChipDesign::at(Node::n5(), 800.0),
        DecodeChipDesign { network: OnChipNetwork::Mesh, ..DecodeChipDesign::at(Node::n5(), 800.0) },
        DecodeChipDesign::at(Node::sky130(), 800.0),
    ];
    let mut models = Vec::new();
    for &name in REPORT_MODELS {
        let header = GgufHeader::read_ledger(&decode_dir.join(format!("headers/{name}.json")))?;
        let step = DecodeStep::of(&header, &deployment).map_err(|e| format!("{name}: {e:?}"))?;
        let mut gpu_floors = Vec::new();
        for chip in [Chip::h100_sxm(), Chip::b200()] {
            for chips in [1u32, 8] {
                if let Ok(cost) = price(&step, &Machine::of(chip.clone(), chips), precision) {
                    gpu_floors.push(GpuFloor { machine: chip.name, chips, floor_tok_s: cost.floor_tokens_per_s(), binds: cost.floor_binds });
                }
            }
        }
        let mut demonstrated = Vec::new();
        for row in published["rows"].as_array().ok_or("published rows")? {
            if row["header"].as_str() != Some(name) {
                continue;
            }
            let chip = Chip::by_name(row["machine"].as_str().ok_or("machine")?).ok_or("a catalog chip")?;
            let mut d = Deployment::as_stored(row["ctx"].as_u64().ok_or("ctx")?);
            if let Some(w) = row["bytes_per_weight"].as_f64() {
                d.weights = WeightBytes::PerWeight(w);
            }
            let theirs = DecodeStep::of(&header, &d).map_err(|e| format!("{e:?}"))?;
            let chips = fewest_chips(&theirs, &chip, precision).map_err(|e| format!("{e:?}"))?;
            let tok_s = row["tok_s"].as_f64().ok_or("tok_s")?;
            demonstrated.push(Demonstrated {
                label: row["label"].as_str().unwrap_or_default().to_string(),
                machine: chip.name,
                chips,
                published_tok_s: tok_s,
                stage_latency_s: demonstrated_stage_latency(&theirs, &Machine::of(chip, chips), tok_s, precision).map_err(|e| format!("{e:?}"))?,
            });
        }
        let mut requirements = Vec::new();
        for floor in &gpu_floors {
            for design in &designs {
                let r = requirement(&step, design, floor.floor_tok_s * speedup, precision).map_err(|e| format!("{name}: {e:?}"))?;
                requirements.push(RequirementRow {
                    baseline: format!("{} x {}", floor.chips, floor.machine),
                    baseline_floor_tok_s: floor.floor_tok_s,
                    speedup,
                    requirement: r,
                });
            }
        }
        models.push(ModelReport {
            model: name.to_string(),
            weight_bytes_per_token: step.weight_bytes(),
            held_bytes: step.held_bytes(),
            stages: step.stages.len(),
            gpu_floors,
            demonstrated,
            requirements,
        });
    }
    Ok(DecodeChipReport {
        what: "What a system of SRAM-resident LOGOS decode chips must achieve to beat a GPU deployment's FLOOR (its memory \
               roofline with every overhead at zero) by the speedup, per user at batch one. Each requirement is a pair of \
               latency budgets: the most one dependent stage may take on chip with chip-to-chip hops free, and the most one \
               chip-to-chip hop may take at the design's own on-chip stage latency. `demonstrated` is the per-stage latency a \
               vendor's published per-user rate implies, the yardstick for a budget. Regenerate with `cargo run -p \
               logicaffeine-compile --example decode_chip_report`; spatial_decode_chip_lock holds the file to the model.",
        deployment,
        speedup,
        nodes: vec![Node::n5(), Node::sky130()],
        models,
    })
}
