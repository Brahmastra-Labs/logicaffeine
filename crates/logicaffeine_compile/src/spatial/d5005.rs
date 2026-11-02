//! The Intel PAC D5005: a Stratix 10 SX 2800 on four channels of DDR4, as a decode card.
//!
//! The card's facts are its data sheet's, its shell's and its device's resource table's. What a design adds to them
//! is an [`EnginePlan`]: how many DSP blocks multiply 4-bit weight codes, two products to each of a block's two 18×19
//! multipliers; how many soft multiply-adds the logic carries beside them; how many block-scale lanes turn the
//! array's integer sums into f32, and the DSP blocks and logic they take; how many DSP blocks run in f32 for the
//! vector units; how much of each DDR4 beat carries data; how many lines of a recurrent state the stream engines
//! read before writing them back; and the clock the array closes at. Until the engine is fitted the clock, the soft
//! multiply-adds and the lanes' logic are modeled, and every price that rests on them says so. Until the card's
//! stream receipt is taken, the share of the DDR4 peak each way a pass moves its bytes sustains is the DDR4 ledger's
//! ([`DDR_STREAMS`]): the shell's own controller timing, issued by the JESD79-4 controller model.
//!
//! [`roofline`] prices Qwen3.8-27B on the card at every knob point of the decode engine, one trick at a time,
//! from the checkpoints' own headers: what each trick buys, which term binds, and how many sequences fit.

use std::path::Path;

use serde::Serialize;

use super::decode::{Chip, GgufHeader, Link, Tier, TierKind};
use super::decode_engine::ddr::{CACHE_POSITIONS, STATE_CHUNKS};
use super::decode_engine::numerics::ActScaling;
use super::part::{MemoryKind, PartResources};
use super::rdu_chip::{Fact, Provenance};
use super::serving::{
    best_serving, AttentionMath, Card, ContextClass, ContextMix, Draft, HostLink, Serving, ServingCost, StateFlush, Streams,
};
use crate::codegen_sva::fabric_target::Precision;

/// The card's data sheet.
pub const D5005_DATA_SHEET: &str = "cdrdv2-public.intel.com/691516/ds-pac-d5005-683568-691516.pdf";
/// The Open FPGA Stack guide for the card: its shell's memory and host interfaces.
pub const D5005_OFS_GUIDE: &str = "ofs.github.io/ofs-2024.2-1/hw/d5005/dev_guides/afu_dev/ug_dev_afu_d5005";
/// The device's overview and its product table: its resources.
pub use super::part::{S10_OVERVIEW, S10_PRODUCT_TABLE};
/// The host link's signalling.
pub const PCIE3_SPEC: &str = "PCI-SIG PCI Express Base Specification Revision 3.0";

pub const D5005_SOURCES: &[&str] =
    &[D5005_DATA_SHEET, D5005_OFS_GUIDE, D5005_FIM_EMIF, S10_OVERVIEW, S10_PRODUCT_TABLE, PCIE3_SPEC];

/// DSP blocks on the device: half its 18×19 multipliers, two to a block.
pub const DSP_BLOCKS: u32 = 5_760;
/// M20K blocks and the bits each holds.
pub const M20K_BLOCKS: u32 = 11_721;
pub const M20K_BITS: u32 = 20_480;
/// Adaptive logic modules.
pub const ALMS: u32 = 933_120;
/// Products of a 4-bit weight code and an 8-bit activation code one DSP block retires per cycle: each of its
/// two 18×19 multipliers takes an activation code on one port and two rows' weight codes on the other, so two
/// rows' products leave every multiplier in disjoint lanes (`decode_engine::rtl::array`).
pub const ARRAY_MACS_PER_DSP: u32 = 4;
/// DSP blocks one block-scale lane takes: its block scale times the activation scale times the block's integer sum
/// and its second scale's alike, two 27×27 products, and its f32 accumulate.
pub const SCALE_LANE_DSPS: u32 = 3;
/// Array cycles per lane cycle the standard plan's lanes keep up with: Qwen3.8-27B Q4_K_M's own mix, its K-quant
/// matrices a lane cycle per 256 elements, its Q8_0 matrices one per 32 over two passes, its drafting layer's Q4_0
/// one per 32, with room for the drafting layer.
pub const LANE_MIX: f64 = 216.0;
/// DDR4 channels, the transfers per second of each and the bytes per transfer (64 data bits; ECC beside them).
pub const DDR_CHANNELS: u32 = 4;
pub const DDR_TRANSFERS_PER_S: f64 = 2.4e9;
pub const DDR_BYTES_PER_TRANSFER: f64 = 8.0;
/// Bytes each channel's DIMM holds: 1 Gi words of 64 data bits.
pub const DDR_CHANNEL_BYTES: f64 = (1u64 << 33) as f64;
/// The card's shell: its four memory controllers built without ECC, each exposing all 72 bits of a beat.
pub const D5005_FIM_EMIF: &str = "OFS ofs-d5005 2024.1-1 ipss/mem/ip/emif_ddr4_no_ecc.ip";
/// Bytes per second one direction of the PCIe Gen3 x16 host link carries: 16 lanes of 8 GT/s, 128b/130b.
pub const PCIE3_X16_BYTES_PER_S: f64 = 16.0 * 8e9 * 128.0 / 130.0 / 8.0;

/// The card as a batch-one decode sees it: the M20K blocks and the DDR4 behind them.
pub fn chip() -> Chip {
    let plan = EnginePlan::standard();
    Chip {
        name: "d5005",
        tiers: vec![on_chip(&plan), memory(plan.beat)],
        matrix: vec![(
            Precision::Int8,
            Fact {
                value: plan.array_rate(),
                provenance: plan.clock_hz.provenance,
                cite: "not published: an int8 operand is two 4-bit digits, two passes of the 4-bit array, so int8 \
                       operations per second (two per multiply-add) equal the array's multiply-adds per second at the \
                       standard engine plan's modeled clock",
            },
        )],
        stage_overhead: None,
        link: Some(Link {
            bandwidth: Some(Fact::derived(
                2.0 * 100e9 / 8.0,
                "cdrdv2-public.intel.com/691516/ds-pac-d5005-683568-691516.pdf: \"Dual QSFP28 Ports: 2x100G\"; two \
                 ports of 100 Gb/s",
            )),
            hop_latency: None,
        }),
        power: Some(Fact::published(
            215.0,
            "cdrdv2-public.intel.com/691516/ds-pac-d5005-683568-691516.pdf: \"Power 215 W (Under sufficient cooling \
             capabilities)\"",
        )),
        sources: D5005_SOURCES,
    }
}

fn memory(beat: Beat) -> Tier {
    match beat {
        Beat::Data64 => Tier {
            kind: TierKind::Ddr,
            capacity: Fact::derived(
                DDR_CHANNELS as f64 * DDR_CHANNEL_BYTES,
                "cdrdv2-public.intel.com/691516/ds-pac-d5005-683568-691516.pdf: \"4x 8GB DDR4-2400 with ECC\"; \
                 ofs.github.io/ofs-2024.2-1/hw/d5005/dev_guides/afu_dev/ug_dev_afu_d5005: \"8GB RDIMM modules organized \
                 as 1 Gb × 72\", 1 Gi words of 64 data bits each, four channels",
            ),
            bandwidth: Fact::derived(
                DDR_CHANNELS as f64 * DDR_TRANSFERS_PER_S * DDR_BYTES_PER_TRANSFER,
                "cdrdv2-public.intel.com/691516/ds-pac-d5005-683568-691516.pdf: \"4x 8GB DDR4-2400 with ECC\"; four \
                 channels of 2400 MT/s, 8 data bytes a transfer",
            ),
        },
        Beat::Full72 => Tier {
            kind: TierKind::Ddr,
            capacity: Fact::derived(
                DDR_CHANNELS as f64 * DDR_CHANNEL_BYTES * 9.0 / 8.0,
                "ofs.github.io/ofs-2024.2-1/hw/d5005/dev_guides/afu_dev/ug_dev_afu_d5005: \"8GB RDIMM modules organized \
                 as 1 Gb × 72\"; OFS ofs-d5005 2024.1-1 ipss/mem/ip/emif_ddr4_no_ecc.ip: MEM_DDR4_DQ_WIDTH 72, \
                 CTRL_DDR4_ECC_EN false; 1 Gi words of all 72 bits each, four channels",
            ),
            bandwidth: Fact::derived(
                DDR_CHANNELS as f64 * DDR_TRANSFERS_PER_S * DDR_BYTES_PER_TRANSFER * 9.0 / 8.0,
                "OFS ofs-d5005 2024.1-1 ipss/mem/ip/emif_ddr4_no_ecc.ip: MEM_DDR4_DQ_WIDTH 72, CTRL_DDR4_ECC_EN false, \
                 MEM_DDR4_SPEEDBIN_ENUM DDR4_SPEEDBIN_2400; ipss/mem/includes/ofs_fim_emif_cfg_pkg.sv: AVMM_DATA_WIDTH 576; \
                 four channels of 2400 MT/s, 9 bytes a transfer",
            ),
        },
    }
}

fn on_chip(plan: &EnginePlan) -> Tier {
    let m20k = PartResources::sx2800().memory(MemoryKind::M20k).expect("the SX 2800 carries M20K blocks");
    Tier {
        kind: TierKind::Sram,
        capacity: Fact::derived(m20k.count.value as f64 * m20k.bits.value as f64 / 8.0, m20k.count.cite),
        bandwidth: Fact {
            value: m20k.count.value as f64 * m20k.read_bits_per_cycle.value as f64 / 8.0 * plan.clock_hz.value,
            provenance: plan.clock_hz.provenance.max(m20k.read_bits_per_cycle.provenance),
            cite: "not published: every M20K block's one widest read port at the standard engine plan's modeled clock",
        },
    }
}

/// How much of each DDR4 beat carries data.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum Beat {
    /// 64 of the 72 bits, the other 8 left to an error-correcting code.
    Data64,
    /// All 72: the shell builds its controllers without ECC, so the eight check bits of every beat are data. A
    /// flipped bit is found, not corrected: the weight image carries a check word per tile and is scrubbed, and
    /// every state and cache row is written with a check word and recomputed from its sequence when one fails.
    Full72,
}

/// How the decode engine spends the device: the multiplier budget, the block-scale lanes, the memory beat and the
/// clock it closes at.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct EnginePlan {
    /// The array's clock.
    pub clock_hz: Fact<f64>,
    /// DSP blocks in the 4-bit array, [`ARRAY_MACS_PER_DSP`] multiply-adds each per cycle.
    pub array_dsp_blocks: u32,
    /// Multiply-adds of a 4-bit weight code and an 8-bit activation code carried in logic beside the DSP blocks.
    pub array_soft_macs: Fact<f64>,
    /// Block-scale lanes, each one cycle a weight block at [`SCALE_LANE_DSPS`] DSP blocks and its logic.
    pub scale_lanes: u32,
    /// Logic one lane takes: its sub-block multipliers, the exact two-term sum, its rounding.
    pub scale_lane_alms: Fact<f64>,
    /// DSP blocks in f32 mode for the vector units, one multiply-add each per cycle.
    pub vector_dsp_blocks: u32,
    pub beat: Beat,
    /// Lines of a recurrent state a stream engine reads before it writes them back: the memory's bus turns twice a
    /// chunk, and the engine holds a chunk read ahead and a chunk written behind.
    pub state_chunk_lines: u64,
}

impl EnginePlan {
    /// The plan the engine is built to: 768 DSP blocks of f32 vector units, 24,576 soft multiply-adds, as many
    /// block-scale lanes as the array's multiply-adds fill at [`LANE_MIX`] a lane cycle, the rest of the DSP blocks in
    /// the array, 64 data bits of every beat, and a recurrent state rewritten 512 lines at a time — a value head's
    /// binary16 state of 128 by 128, at 64 bytes a line.
    pub fn standard() -> EnginePlan {
        const VECTOR: u32 = 768;
        const SOFT: f64 = 24_576.0;
        // The fewest lanes that keep up with what the DSP blocks left to the array multiply.
        let lanes = {
            let mut l = 0u32;
            while ((DSP_BLOCKS - VECTOR - SCALE_LANE_DSPS * l) as f64 * ARRAY_MACS_PER_DSP as f64 + SOFT) / LANE_MIX > l as f64 {
                l += 1;
            }
            l
        };
        EnginePlan {
            clock_hz: Fact::modeled(
                450e6,
                "not published: the clock a Hyperflex-retimed array is planned to close at; replaced by the fitted \
                 engine's timing",
            ),
            array_dsp_blocks: DSP_BLOCKS - VECTOR - SCALE_LANE_DSPS * lanes,
            array_soft_macs: Fact::modeled(
                SOFT,
                "not tabulated: soft multiply-adds of a 4-bit code and an 8-bit code in compressor trees, about 15 ALMs \
                 each over 40% of the device's ALMs; replaced by the engine's synthesized census",
            ),
            scale_lanes: lanes,
            scale_lane_alms: Fact::modeled(
                650.0,
                "not tabulated: sixteen radix-4 Booth products of a 6-to-8-bit multiplier and a 17-to-26-bit sum, the \
                 activation scale's 11×8 product, a two-term exact sum over 29 binary places and its rounding to f32; \
                 replaced by the engine's synthesized census",
            ),
            vector_dsp_blocks: VECTOR,
            beat: Beat::Data64,
            state_chunk_lines: 512,
        }
    }

    /// The standard plan reading and writing all 72 bits of every beat.
    pub fn full_beat() -> EnginePlan {
        EnginePlan { beat: Beat::Full72, ..EnginePlan::standard() }
    }

    /// Multiply-adds per second the array retires.
    pub fn array_rate(&self) -> f64 {
        (self.array_dsp_blocks as f64 * ARRAY_MACS_PER_DSP as f64 + self.array_soft_macs.value) * self.clock_hz.value
    }

    /// Lane cycles per second the block-scale lanes retire.
    pub fn scale_rate(&self) -> f64 {
        self.scale_lanes as f64 * self.clock_hz.value
    }

    /// DSP blocks the plan spends: the array's, the lanes', the vector units'.
    pub fn dsp_blocks(&self) -> u32 {
        self.array_dsp_blocks + SCALE_LANE_DSPS * self.scale_lanes + self.vector_dsp_blocks
    }

    /// ALMs the plan's soft multiply-adds and lanes take, at the modeled 15 a soft multiply-add.
    pub fn alms(&self) -> f64 {
        self.array_soft_macs.value * 15.0 + self.scale_lanes as f64 * self.scale_lane_alms.value
    }

    /// f32 multiply-adds per second the vector units retire.
    pub fn vector_rate(&self) -> f64 {
        self.vector_dsp_blocks as f64 * self.clock_hz.value
    }
}

/// The card as a serving sees it, under `plan`.
pub fn card(plan: &EnginePlan) -> Card {
    assert!(plan.dsp_blocks() <= DSP_BLOCKS, "the plan spends {} DSP blocks of {DSP_BLOCKS}", plan.dsp_blocks());
    assert!(plan.alms() <= ALMS as f64, "the plan spends {} ALMs of {ALMS}", plan.alms());
    let weakest = plan.clock_hz.provenance.max(plan.array_soft_macs.provenance);
    Card {
        name: "d5005",
        memory: memory(plan.beat),
        on_chip: on_chip(plan),
        streams: streams(plan),
        array: Fact {
            value: plan.array_rate(),
            provenance: weakest,
            cite: "not published: the plan's array DSP blocks at 4 multiply-adds a cycle plus its soft multiply-adds, \
                   at its modeled clock",
        },
        vector: Fact {
            value: plan.vector_rate(),
            provenance: plan.clock_hz.provenance,
            cite: "not published: the plan's f32 DSP blocks, one multiply-add a cycle each (S10-OVERVIEW Table 11: one \
                   single precision multiplier and adder a block), at its modeled clock",
        },
        scale: Fact {
            value: plan.scale_rate(),
            provenance: plan.clock_hz.provenance.max(plan.scale_lane_alms.provenance),
            cite: "not published: the plan's block-scale lanes, one lane cycle each a cycle, at its modeled clock",
        },
        host: Some(host_link()),
        sources: D5005_SOURCES,
    }
}

/// The clocks a channel carried data in of the clocks one stream took, as the DDR4 ledger counts them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct StreamClocks {
    pub data_ck: u64,
    pub end: u64,
}

impl StreamClocks {
    /// The share of the channel's peak the stream sustained.
    pub fn share(self) -> f64 {
        self.data_ck as f64 / self.end as f64
    }
}

/// One beat's streams in the DDR4 ledger (`benchmarks/silicon/d5005/ddr4_efficiency.json`, written by
/// [`super::decode_engine::ddr::ddr_ledger`] from the shell's EMIF timing through the JESD79-4 controller model), in the
/// ledger's order: a long read of weights; a Gated-DeltaNet memory read and written back
/// [`STATE_CHUNKS`] lines at a time; a cache read with its new position appended after it, at each of
/// [`CACHE_POSITIONS`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct BeatStreams {
    pub beat: Beat,
    pub weights: StreamClocks,
    pub state: [StreamClocks; STATE_CHUNKS.len()],
    pub cache: [StreamClocks; CACHE_POSITIONS.len()],
}

const fn clocks(data_ck: u64, end: u64) -> StreamClocks {
    StreamClocks { data_ck, end }
}

/// The DDR4 ledger's streams at both beats.
pub const DDR_STREAMS: [BeatStreams; 2] = [
    BeatStreams {
        beat: Beat::Data64,
        weights: clocks(262_144, 275_498),
        state: [clocks(817_152, 1_262_867), clocks(817_152, 962_962), clocks(817_152, 885_772), clocks(817_152, 866_952)],
        cache: [clocks(131_584, 138_155), clocks(1_049_088, 1_102_936), clocks(4_194_816, 4_411_164)],
    },
    BeatStreams {
        beat: Beat::Full72,
        weights: clocks(262_144, 275_498),
        state: [clocks(726_368, 1_122_940), clocks(726_368, 856_196), clocks(726_368, 787_655), clocks(726_368, 770_807)],
        cache: [clocks(116_992, 123_094), clocks(932_544, 980_425), clocks(3_728_752, 3_920_783)],
    },
];

/// The shares of the card's DDR4 the stream engines sustain under `plan`, from the ledger's streams at its beat: the
/// weights' long read; the state rewritten at the plan's chunk; the least of the caches', which holds at every context
/// the ledger prices.
pub fn streams(plan: &EnginePlan) -> Streams {
    let at = DDR_STREAMS.iter().find(|s| s.beat == plan.beat).expect("the ledger streams both beats");
    let chunk = STATE_CHUNKS.iter().position(|&c| c == plan.state_chunk_lines).unwrap_or_else(|| {
        panic!("the plan rewrites a state {} lines at a time; the DDR4 ledger prices {STATE_CHUNKS:?}", plan.state_chunk_lines)
    });
    Streams {
        read: Fact::modeled(
            at.weights.share(),
            "not published: the share of DDR4 peak the shell's controllers sustain on one long read, as the JESD79-4 \
             controller model issues it at the shell's EMIF timing (benchmarks/silicon/d5005/ddr4_efficiency.json, the \
             weights row of the plan's beat); replaced by the card's measured stream receipt",
        ),
        rewrite: Fact::modeled(
            at.state[chunk].share(),
            "not published: the share of DDR4 peak the shell's controllers sustain on a memory read and written back a \
             chunk at a time, as the JESD79-4 controller model issues it at the shell's EMIF timing \
             (benchmarks/silicon/d5005/ddr4_efficiency.json, the state row of the plan's chunk and beat); replaced by the \
             card's measured stream receipt",
        ),
        append: Fact::modeled(
            at.cache.iter().map(|c| c.share()).fold(f64::INFINITY, f64::min),
            "not published: the share of DDR4 peak the shell's controllers sustain on a long read with a row appended \
             after it, as the JESD79-4 controller model issues it at the shell's EMIF timing \
             (benchmarks/silicon/d5005/ddr4_efficiency.json, the least of the cache rows of the plan's beat); replaced by \
             the card's measured stream receipt",
        ),
    }
}

/// The host memory the card reaches over PCIe Gen3 x16, both directions at once.
pub fn host_link() -> HostLink {
    let direction = |cite: &'static str| Fact::derived(PCIE3_X16_BYTES_PER_S, cite);
    HostLink {
        capacity: Fact::modeled(
            (64u64 << 30) as f64,
            "not published: the host memory a serving pins for the card, 64 GiB; the card's host server is not yet \
             chosen",
        ),
        read: direction(
            "ofs.github.io/ofs-2024.2-1/hw/d5005/dev_guides/afu_dev/ug_dev_afu_d5005: \"AXI Streaming (AXI-S) interface \
             to the Host via PCIe Gen3x16\"; PCI-SIG PCI Express Base Specification Revision 3.0: 8.0 GT/s a lane, \
             128b/130b encoding; 16 lanes host to card",
        ),
        write: direction(
            "ofs.github.io/ofs-2024.2-1/hw/d5005/dev_guides/afu_dev/ug_dev_afu_d5005: \"AXI Streaming (AXI-S) interface \
             to the Host via PCIe Gen3x16\"; PCI-SIG PCI Express Base Specification Revision 3.0: 8.0 GT/s a lane, \
             128b/130b encoding; 16 lanes card to host",
        ),
        stream: Fact::modeled(
            0.8,
            "not published: the share of a PCIe Gen3 x16 direction the card's DMA sustains with 256-byte payloads; \
             replaced by the card's measured host-stream receipt",
        ),
    }
}

// ── The roofline ────────────────────────────────────────────────────────────────────────────────────

/// The ledger the measured MTP acceptance is read from.
pub const ACCEPTANCE_LEDGER: &str = "benchmarks/results/qwen38_27b.json";
/// The JSON pointer of the one-draft acceptance measured on Qwen3.8-27B.
pub const ACCEPTANCE_POINTER: &str = "/logos/speculative_decode_2026_09_21/b1/accept_rate";

/// The one-draft acceptance, as measured.
pub fn measured_acceptance(value: f64) -> Fact<f64> {
    Fact::measured(
        value,
        "benchmarks/results/qwen38_27b.json /logos/speculative_decode_2026_09_21/b1/accept_rate: the blk.64 draft \
         verified against the main model, H100 NVL, 512 prompt tokens and 128 generated",
    )
}

/// Vocabulary rows the drafting head reads when it keeps only the most frequent tokens.
pub const DRAFT_HEAD_ROWS: u64 = 32_768;
/// Positions per KV page.
pub const KV_PAGE: u64 = 256;

/// One knob point of the engine and its price at the batch that emits the most tokens per second.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RooflineRow {
    pub label: String,
    /// The header ledger the checkpoint is read from.
    pub header: &'static str,
    pub serving: Serving,
    pub cost: ServingCost,
    pub tokens_per_s: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Roofline {
    pub card: Card,
    pub plan: EnginePlan,
    pub rows: Vec<RooflineRow>,
    /// Every context's last rung and the mixed contexts again on the plan that reads all 72 bits of every beat.
    pub full_beat: FullBeat,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FullBeat {
    pub card: Card,
    pub plan: EnginePlan,
    pub rows: Vec<RooflineRow>,
}

/// The engine's knob points, one trick added at a time, then every trick at every weight format.
pub fn ladder(acceptance: f64) -> Vec<(String, &'static str, Serving)> {
    let base = |limit: u64| Serving {
        batch: 1,
        contexts: ContextMix::uniform(limit as f64 / 2.0, limit),
        kv_page: None,
        attention: AttentionMath::Vector,
        kv_bytes: 2.0,
        state_bytes: 4.0,
        activation_words: 2,
        scaling: ActScaling::Sub32,
        kv_rotated: false,
        draft: None,
        flush: StateFlush::EveryPass,
        host_memory: false,
    };
    let per_block = |s: Serving| Serving { scaling: ActScaling::PerBlock, ..s };
    let f16_state = |s: Serving| Serving { state_bytes: 2.0, ..s };
    // An int8 KV element with an f16 scale per 256-element head row, each row rotated before its codes.
    let int8_kv = |s: Serving| Serving { kv_bytes: 1.0 + 2.0 / 256.0, kv_rotated: true, ..s };
    let lazy = |s: Serving| Serving { flush: StateFlush::Every { tokens: 16 }, ..s };
    let mtp = |s: Serving| Serving {
        draft: Some(Draft { tokens: 1, acceptance: vec![measured_acceptance(acceptance)], head_rows: None }),
        ..s
    };
    let fr_spec = |s: Serving| Serving { draft: s.draft.clone().map(|d| Draft { head_rows: Some(DRAFT_HEAD_ROWS), ..d }), ..s };
    let paged = |s: Serving| Serving { kv_page: Some(KV_PAGE), ..s };
    let host = |s: Serving| Serving { host_memory: true, ..s };
    let one_word = |s: Serving| Serving { activation_words: 1, ..s };
    let q8 = "qwen3.8-27b-q8_0";
    let q4 = "qwen3.8-27b-q4_k_m";
    let mut rows = Vec::new();
    for (ctx, tag) in [(2048u64, "2k"), (8192, "8k")] {
        let s0 = base(ctx);
        let sb = per_block(s0.clone());
        let s1 = f16_state(sb.clone());
        let s2 = int8_kv(s1.clone());
        let s3 = lazy(s2.clone());
        let s4 = mtp(s3.clone());
        let s5 = one_word(s4.clone());
        let s6 = fr_spec(s5.clone());
        let s7 = paged(s6.clone());
        let s8 = host(s7.clone());
        for (label, header, s) in [
            ("Q8_0, f16 KV, f32 state, two-word activations", q8, s0.clone()),
            ("Q4_K_M", q4, s0),
            ("+ activations scaled per weight block", q4, sb),
            ("+ f16 state", q4, s1),
            ("+ int8 KV, rotated", q4, s2),
            ("+ state flushed every 16 tokens", q4, s3),
            ("+ one MTP draft", q4, s4),
            ("+ one-word activations", q4, s5),
            ("+ 32k-row drafting head", q4, s6),
            ("+ paged KV", q4, s7),
            ("+ host memory as a fifth channel", q4, s8.clone()),
        ] {
            rows.push((format!("{tag}: {label}"), header, s));
        }
        if tag == "2k" {
            for (label, header) in [
                ("Q8_0", "qwen3.8-27b-q8_0"),
                ("Q6_K", "qwen3.8-27b-q6_k"),
                ("Q5_K_M", "qwen3.8-27b-q5_k_m"),
                ("IQ4_XS", "qwen3.8-27b-iq4_xs"),
                ("Q4_0", "qwen3.8-27b-q4_0"),
                ("Q3_K_M", "qwen3.8-27b-q3_k_m"),
            ] {
                rows.push((format!("{tag}, every trick: {label}"), header, s8.clone()));
            }
        }
    }
    // Every trick across the contexts a sequence may be allowed, attention on the vector units and on the array.
    let every = |contexts: ContextMix, attention: AttentionMath| Serving {
        contexts,
        attention,
        ..host(paged(fr_spec(one_word(mtp(lazy(int8_kv(f16_state(per_block(base(2048))))))))))
    };
    for limit in CONTEXT_SWEEP {
        for (math, name) in [(AttentionMath::Vector, "vector units"), (AttentionMath::Array, "array")] {
            let label = format!("context {} of {}, attention on the {name}", limit / 2, limit);
            rows.push((label, q4, every(ContextMix::uniform(limit as f64 / 2.0, limit), math)));
        }
    }
    for (math, name) in [(AttentionMath::Vector, "vector units"), (AttentionMath::Array, "array")] {
        rows.push((format!("mixed contexts, attention on the {name}"), q4, every(mixed(), math)));
    }
    rows
}

/// Context limits the sweep allows its sequences, each halfway through its limit: up to the checkpoint's own.
pub const CONTEXT_SWEEP: [u64; 7] = [1024, 4096, 16384, 32768, 65536, 131072, 262144];

/// A serving's mix of chats, documents and long contexts: 70% of sequences at 512 of 2,048 positions, 25% at
/// 4,096 of 8,192, 5% at 65,536 of 131,072.
pub fn mixed() -> ContextMix {
    ContextMix {
        classes: vec![
            ContextClass { share: 0.70, position: 512.0, limit: 2048 },
            ContextClass { share: 0.25, position: 4096.0, limit: 8192 },
            ContextClass { share: 0.05, position: 65536.0, limit: 131072 },
        ],
    }
}

/// Every knob point of [`ladder`] priced on the card under the standard plan, at its fastest batch.
pub fn roofline(decode_dir: &Path) -> Result<Roofline, String> {
    let ledger: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(decode_dir.join("../../results/qwen38_27b.json")).map_err(|e| format!("{ACCEPTANCE_LEDGER}: {e}"))?,
    )
    .map_err(|e| format!("{ACCEPTANCE_LEDGER}: {e}"))?;
    let acceptance = ledger
        .pointer(ACCEPTANCE_POINTER)
        .and_then(|v| v.as_f64())
        .ok_or_else(|| format!("{ACCEPTANCE_LEDGER}: no number at {ACCEPTANCE_POINTER}"))?;
    let price = |card: &Card, label: String, header: &'static str, s: Serving| -> Result<RooflineRow, String> {
        let h = GgufHeader::read_ledger(&decode_dir.join(format!("headers/{header}.json")))?;
        let cost = best_serving(&h, card, &s).map_err(|e| format!("{label}: {e:?}"))?;
        let serving = Serving { batch: cost.batch, ..s };
        Ok(RooflineRow { label, header, tokens_per_s: cost.tokens_per_s(), serving, cost })
    };
    let plan = EnginePlan::standard();
    let standard = card(&plan);
    let full_plan = EnginePlan::full_beat();
    let full = card(&full_plan);
    let rungs = ladder(acceptance);
    let mut rows = Vec::new();
    for (label, header, s) in rungs.iter().cloned() {
        rows.push(price(&standard, label, header, s)?);
    }
    let tops = rungs
        .iter()
        .filter(|(label, ..)| label.ends_with("host memory as a fifth channel") || label.starts_with("mixed contexts"))
        .cloned();
    let mut full_rows = Vec::new();
    for (label, header, s) in tops {
        full_rows.push(price(&full, format!("{label}, all 72 bits of every beat"), header, s)?);
    }
    Ok(Roofline { card: standard, plan, rows, full_beat: FullBeat { card: full, plan: full_plan, rows: full_rows } })
}

/// The weakest provenance of every fact a roofline row rests on.
pub fn weakest(rows: &[RooflineRow]) -> Provenance {
    rows.iter().map(|r| r.cost.provenance).max().unwrap_or(Provenance::Published)
}
